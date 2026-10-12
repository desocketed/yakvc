//! `cargo xtask ci`: what `.github/workflows/ci.yml` checks on Linux, run
//! locally in order and stopping at the first failure. Also the checks CI
//! and this share: the server's dependencies, the jar's contents and the
//! server image's first start.

use std::fs;
use std::io::Read;
use std::path::Path;
use std::process::Command;
use std::thread::sleep;
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};

use crate::{root, run};

/// Crates the rendezvous server must never pull in: it has no audio.
const AUDIO_CRATES: &[&str] = &[
    "yakvc-audio",
    "yakvc-client",
    "cpal",
    "opus",
    "audiopus",
    "alsa",
];

/// What a release jar must hold: a native per platform, their checksums,
/// the xz decoder and the third-party licences.
const JAR_ENTRIES: &[&str] = &[
    "natives/natives.sha256",
    "natives/linux-x86_64/libyakvc_ffi.so.xz",
    "natives/linux-aarch64/libyakvc_ffi.so.xz",
    "natives/windows-x86_64/yakvc_ffi.dll.xz",
    "natives/macos-universal/libyakvc_ffi.dylib.xz",
    "META-INF/jars/xz-1.10.jar",
    "THIRD_PARTY_LICENSES",
];

/// Runs every Linux check; `fast` stops before the release builds, the mod
/// and the gametest. Windows and macOS natives build only on their own CI
/// runners, so the local jar has only the host's and isn't checked whole.
pub(crate) fn ci(fast: bool) -> Result<()> {
    let root = root();
    let cargo = |args: &[&str]| {
        let mut cmd = Command::new("cargo");
        cmd.args(args).current_dir(&root);
        run(&mut cmd)
    };

    step("rust: fmt");
    cargo(&["fmt", "--all", "--check"])?;
    step("rust: clippy");
    cargo(&[
        "clippy",
        "--workspace",
        "--all-targets",
        "--all-features",
        "--locked",
        "--",
        "-D",
        "warnings",
    ])?;
    step("rust: clippy on the client crates as they ship (no sim feature)");
    cargo(&[
        "clippy",
        "-p",
        "yakvc-client",
        "-p",
        "yakvc-ffi",
        "-p",
        "yakvc-cli",
        "--all-targets",
        "--locked",
        "--",
        "-D",
        "warnings",
    ])?;
    step("rust: header");
    crate::header(true)?;
    step("rust: test");
    cargo(&["test", "--workspace", "--all-features", "--locked"])?;
    step("rust: server has no audio dependencies");
    check_server_deps()?;
    step("deny");
    cargo(&["deny", "--all-features", "--locked", "check"])?;
    if fast {
        step("skipped the flake check, release builds, mod build and gametest (--fast)");
        return Ok(());
    }

    step("nix: flake check");
    run(Command::new("nix")
        .args(["flake", "check"])
        .current_dir(&root))?;
    step("natives: Linux release targets");
    crate::natives("all", &root.join("target/ci-natives"))?;
    step("cli: Linux release, for glibc 2.17");
    crate::cli("x86_64-unknown-linux-gnu", &root.join("target/ci-cli"))?;
    if has_docker() {
        step("server: image, and its first start");
        crate::server_image()?;
        server_smoke()?;
    } else {
        step("server: no docker here; skipped the image");
    }
    step("mod: build (cargo xtask dist, host natives)");
    crate::dist(None)?;
    step("mod: jar manifest");
    check_jar_manifest(&dist_jar()?)?;
    step("mod: client gametest, and the user guide's screenshots");
    crate::headless::gametest(&[])?;

    step("all checks passed");
    Ok(())
}

fn step(name: &str) {
    println!("\n==> {name}");
}

/// Fails if `yakvc-server` depends, even indirectly, on an audio crate.
pub(crate) fn check_server_deps() -> Result<()> {
    let out = Command::new("cargo")
        .args([
            "tree",
            "--locked",
            "-p",
            "yakvc-server",
            "-e",
            "normal",
            "--prefix",
            "none",
        ])
        .current_dir(root())
        .output()
        .context("running cargo tree")?;
    ensure!(
        out.status.success(),
        "cargo tree failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let tree = String::from_utf8(out.stdout)?;
    let found: Vec<&str> = tree
        .lines()
        .filter(|line| AUDIO_CRATES.contains(&line.split(' ').next().unwrap_or("")))
        .collect();
    ensure!(
        found.is_empty(),
        "yakvc-server must not depend on audio crates: {found:?}"
    );
    Ok(())
}

/// Fails if a release jar lacks a platform's native, the checksums, the xz
/// decoder, the licences or native access in its manifest.
pub(crate) fn check_jar(jar: &Path) -> Result<()> {
    let names = jar_names(jar)?;
    let missing: Vec<&&str> = JAR_ENTRIES
        .iter()
        .filter(|entry| !names.iter().any(|n| n == **entry))
        .collect();
    ensure!(missing.is_empty(), "{} has no {missing:?}", jar.display());
    check_jar_manifest(jar)
}

fn check_jar_manifest(jar: &Path) -> Result<()> {
    let mut archive = zip::ZipArchive::new(fs::File::open(jar)?)?;
    let mut manifest = String::new();
    archive
        .by_name("META-INF/MANIFEST.MF")?
        .read_to_string(&mut manifest)?;
    ensure!(
        manifest
            .lines()
            .any(|line| line.trim() == "Enable-Native-Access: ALL-UNNAMED"),
        "{} doesn't enable native access in its manifest",
        jar.display()
    );
    Ok(())
}

fn jar_names(jar: &Path) -> Result<Vec<String>> {
    let archive =
        zip::ZipArchive::new(fs::File::open(jar).with_context(|| jar.display().to_string())?)?;
    let names = archive
        .file_names()
        .map(|name| name.map(|name| name.into_owned()));
    Ok(names.collect::<Result<_, _>>()?)
}

/// The jar `cargo xtask dist` just built.
fn dist_jar() -> Result<std::path::PathBuf> {
    let dir = root().join("dist");
    for entry in fs::read_dir(&dir)? {
        let path = entry?.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.starts_with("yakvc-") && name.ends_with(".jar") {
            return Ok(path);
        }
    }
    bail!("no yakvc jar in {}", dir.display())
}

fn has_docker() -> bool {
    Command::new("docker")
        .arg("version")
        .output()
        .is_ok_and(|out| out.status.success())
}

/// Starts the server image twice as `deploy/yakvc-server-docker.service`
/// does: the first start must create its keys in a fresh volume (#51), and
/// the second must find the same ones there.
pub(crate) fn server_smoke() -> Result<()> {
    let dir = root().join("target/server-smoke");
    fs::create_dir_all(&dir)?;
    fs::write(
        dir.join("config.toml"),
        "endpoint_key = \"/var/lib/yakvc/endpoint.key\"\nissuer_key = \"/var/lib/yakvc/issuer.key\"\nbind = \"127.0.0.1:7843\"\n",
    )?;
    // A fresh volume, as on a new host.
    let _ = Command::new("docker")
        .args(["volume", "rm", "-f", "yakvc-smoke-state"])
        .output();
    let first = start_once(&dir)?;
    let second = start_once(&dir)?;
    ensure!(
        first == second,
        "the keys weren't kept: {first}, then {second}"
    );
    Ok(())
}

/// One start of the image; its endpoint id.
fn start_once(dir: &Path) -> Result<String> {
    let config = format!("{}:/etc/yakvc:ro", dir.display());
    run(Command::new("docker").args([
        "run",
        "-d",
        "--name",
        "yakvc-smoke",
        "--network",
        "host",
        "--user",
        "0:0",
        "--cap-drop",
        "ALL",
        "--cap-add",
        "NET_BIND_SERVICE",
        "--security-opt",
        "no-new-privileges",
        "--read-only",
        "--volume",
        &config,
        "--volume",
        "yakvc-smoke-state:/var/lib/yakvc",
        "yakvc-server",
    ]))?;
    let mut log = String::new();
    for _ in 0..30 {
        let out = Command::new("docker")
            .args(["logs", "yakvc-smoke"])
            .output()?;
        log = String::from_utf8_lossy(&out.stdout).into_owned()
            + &String::from_utf8_lossy(&out.stderr);
        if log.lines().any(|line| line.starts_with("issuer id")) {
            break;
        }
        sleep(Duration::from_secs(1));
    }
    let _ = Command::new("docker")
        .args(["rm", "-f", "yakvc-smoke"])
        .output();
    eprintln!("{log}");
    log.lines()
        .find(|line| line.starts_with("endpoint id"))
        .map(str::to_owned)
        .context("the server didn't start")
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn jar_with(name: &str, entries: &[&str], manifest: &str) -> std::path::PathBuf {
        let path =
            std::env::temp_dir().join(format!("yakvc-jar-{}-{name}.jar", std::process::id()));
        let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        for entry in entries {
            zip.start_file(*entry, options).unwrap();
        }
        zip.start_file("META-INF/MANIFEST.MF", options).unwrap();
        zip.write_all(manifest.as_bytes()).unwrap();
        zip.finish().unwrap();
        path
    }

    #[test]
    fn a_complete_jar_passes() {
        let jar = jar_with(
            "complete",
            JAR_ENTRIES,
            "Manifest-Version: 1.0\nEnable-Native-Access: ALL-UNNAMED\n",
        );
        check_jar(&jar).unwrap();
    }

    #[test]
    fn a_jar_missing_a_native_fails() {
        let without_windows: Vec<&str> = JAR_ENTRIES
            .iter()
            .copied()
            .filter(|entry| !entry.ends_with(".dll.xz"))
            .collect();
        let jar = jar_with(
            "missing",
            &without_windows,
            "Enable-Native-Access: ALL-UNNAMED\n",
        );
        let error = check_jar(&jar).unwrap_err().to_string();
        assert!(error.contains("yakvc_ffi.dll.xz"), "{error}");
    }

    #[test]
    fn a_jar_without_native_access_fails() {
        let jar = jar_with("no-access", JAR_ENTRIES, "Manifest-Version: 1.0\n");
        assert!(check_jar(&jar).is_err());
    }
}
