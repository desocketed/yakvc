use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::{env, fs};

mod ci;
mod dev;
mod github;
mod headless;
mod pack;
mod pipewire;
mod relay;
mod session;
mod two_clients;

use anyhow::{Context, Result, bail, ensure};
use clap::{Parser, Subcommand};
use sha2::{Digest, Sha256};

#[derive(Debug, Parser)]
struct Args {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Debug, Subcommand)]
enum Cmd {
    /// Build yakvc-ffi with the `dist` profile and stage it for the mod.
    Natives {
        /// `host` (a quick dev build), `all` (every release target this
        /// host's OS builds), or a Rust target triple.
        #[arg(long, default_value = "host")]
        target: String,
        /// Staging directory; gets `<os>-<arch>/` subdirectories and a
        /// `natives.sha256` manifest.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Build yakvc-cli for release and copy it into a directory under its
    /// release name; Linux builds link against glibc 2.17.
    Cli {
        /// A Rust target triple.
        #[arg(long)]
        target: String,
        #[arg(long)]
        out: PathBuf,
    },
    /// Regenerate crates/yakvc-ffi/include/yakvc.h with cbindgen.
    Header {
        /// Fail instead of writing if the checked-in header is stale.
        #[arg(long)]
        check: bool,
    },
    /// Build the release jar into dist/.
    Dist {
        /// Natives staged by `natives --out` on each OS (CI artifacts,
        /// merged into one directory). Without it, builds them for this
        /// host only.
        #[arg(long)]
        from: Option<PathBuf>,
    },
    /// Package a release jar as launcher packs (a Modrinth .mrpack and a Prism
    /// instance zip), each with Fabric API inside.
    Pack {
        /// The release jar (default: dist/yakvc-<version>.jar).
        #[arg(long)]
        jar: Option<PathBuf>,
        /// Output directory (default: dist).
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Build the yakvc-server Docker image.
    ServerImage,
    /// Everything CI checks on Linux, locally and in order, stopping at the
    /// first failure. Also refreshes the user guide's screenshots.
    Ci {
        /// Stop before the flake check, release builds, mod and gametest.
        #[arg(long)]
        fast: bool,
    },
    /// The Fabric client gametest with no display or GPU (Xvfb and Mesa's
    /// software renderers), then the user guide's screenshots refreshed from
    /// it. Arguments go to Gradle.
    Gametest {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        gradle_args: Vec<String>,
    },
    /// Merge the macOS natives built per architecture (`macos-x86_64` and
    /// `macos-aarch64` in a natives directory) into one universal library
    /// in `macos-universal`, with `lipo`, or `llvm-lipo` off macOS.
    MergeMacos { natives: PathBuf },
    /// The end-to-end check: a dev rendezvous, a local Minecraft server and
    /// two headless clients that must hear each other, and not while one is
    /// a spectator. About a minute.
    TwoClients {
        /// Accept the Minecraft EULA (https://aka.ms/MinecraftEULA), which
        /// running the server needs; or set YAKVC_ACCEPT_MINECRAFT_EULA=1.
        #[arg(long)]
        accept_eula: bool,
        /// Give the clients real devices through a private PipeWire session
        /// each, instead of the dev test audio.
        #[arg(long)]
        pipewire: bool,
    },
    /// For CI's `changes` job: whether this run builds anything, written to
    /// `$GITHUB_OUTPUT` as `code=true` or `code=false`.
    Changes,
    /// Fail if yakvc-server depends on an audio crate.
    CheckServerDeps,
    /// Fail if a release jar lacks a platform's native, the checksums, the
    /// licences or native access in its manifest.
    CheckJar { jar: PathBuf },
    /// Start the yakvc-server image twice, as the service does: the first
    /// start must create its keys in a fresh volume, the second keep them.
    ServerSmoke,
    /// Start a dev-mode yakvc-server and print the client config for it.
    Dev {
        /// Port on 127.0.0.1.
        #[arg(long, default_value_t = 7843)]
        port: u16,
        /// Keys, server config and client.toml (default target/dev-server).
        #[arg(long)]
        dir: Option<PathBuf>,
        /// Also write the client config into runClient's game directory
        /// (mod/run/config/yakvc/client.toml).
        #[arg(long)]
        write_client_config: bool,
        /// Check accounts with Mojang instead of taking dev tickets, for the
        /// manual real-account check; also builds the mod jar, serves
        /// counters on 127.0.0.1:9187 and defaults to target/real-auth.
        #[arg(long)]
        real_auth: bool,
    },
    /// A dev session on your own machine: a local rendezvous, an offline dev
    /// Minecraft server and one client window per name, joining it by
    /// themselves. State lives in mod/run/dev/.
    Session {
        /// The players, one client each.
        #[arg(default_values = ["Alice", "Bob"])]
        names: Vec<String>,
        /// Only start these clients, joining a session that is already running.
        #[arg(long)]
        join: bool,
        /// Players who talk on their own, with no microphone: they send a
        /// looping clip, by voice activation. Repeat for more than one.
        #[arg(long, value_name = "NAME")]
        talk: Vec<String>,
        /// What they say: a WAV file. Without it, the engine's built-in
        /// test beeps, with gaps between so echoes and reverb can be heard.
        #[arg(long, value_name = "WAV")]
        talk_file: Option<PathBuf>,
    },
    /// Run a command against real audio devices: a private PipeWire session
    /// with a test microphone (playing a tone) and test speakers, reached
    /// through ALSA. Linux only; needs Nix.
    WithPipewire {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, required = true)]
        command: Vec<String>,
    },
    /// Switch the production relay to a commit of this repository: builds its
    /// NixOS system here and pushes it with `nixos-rebuild --target-host`.
    /// CI runs it after every release (the `deploy` job); the values are the
    /// `RELAY_*` secrets.
    DeployRelay {
        /// The host flake with its configuration name, such as
        /// `git+ssh://git@github.com/owner/relay#host`.
        #[arg(long)]
        flake: String,
        /// `user@host` of the box; the user needs passwordless sudo.
        #[arg(long)]
        target: String,
        /// SSH private key that reads the flake and logs in to the box.
        #[arg(long)]
        key: PathBuf,
        /// known_hosts file with the git host and the box.
        #[arg(long)]
        known_hosts: PathBuf,
        /// The yakvc commit to ship (default: HEAD).
        #[arg(long)]
        rev: Option<String>,
    },
}

fn main() -> Result<()> {
    match Args::parse().command {
        Cmd::Natives { target, out } => natives(
            &target,
            &out.unwrap_or_else(|| root().join("mod/build/natives")),
        ),
        Cmd::Cli { target, out } => cli(&target, &out),
        Cmd::Header { check } => header(check),
        Cmd::Dist { from } => dist(from.as_deref()),
        Cmd::ServerImage => server_image(),
        Cmd::Ci { fast } => ci::ci(fast),
        Cmd::MergeMacos { natives } => merge_macos(&natives),
        Cmd::Gametest { gradle_args } => headless::gametest(&gradle_args),
        Cmd::TwoClients {
            accept_eula,
            pipewire,
        } => two_clients::two_clients(
            accept_eula || std::env::var("YAKVC_ACCEPT_MINECRAFT_EULA").as_deref() == Ok("1"),
            pipewire,
        ),
        Cmd::Changes => github::changes(),
        Cmd::CheckServerDeps => ci::check_server_deps(),
        Cmd::CheckJar { jar } => ci::check_jar(&jar),
        Cmd::ServerSmoke => ci::server_smoke(),
        Cmd::Pack { jar, out } => pack::pack(
            &jar.unwrap_or_else(|| root().join(format!("dist/yakvc-{VERSION}.jar"))),
            &out.unwrap_or_else(|| root().join("dist")),
        ),
        Cmd::Dev {
            port,
            dir,
            write_client_config,
            real_auth,
        } => dev::dev(port, dir, write_client_config, real_auth),
        Cmd::Session {
            names,
            join,
            talk,
            talk_file,
        } => session::session(&names, join, &talk, talk_file.as_deref()),
        Cmd::WithPipewire { command } => std::process::exit(pipewire::with_pipewire(&command)?),
        Cmd::DeployRelay {
            flake,
            target,
            key,
            known_hosts,
            rev,
        } => relay::deploy(
            &flake,
            &target,
            &key,
            &known_hosts,
            &match rev {
                Some(rev) => rev,
                None => head()?,
            },
        ),
    }
}

/// The checked-out commit.
fn head() -> Result<String> {
    let out = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(root())
        .output()
        .context("running git")?;
    ensure!(out.status.success(), "git rev-parse HEAD failed");
    Ok(String::from_utf8(out.stdout)?.trim().to_string())
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

/// The workspace version, which the mod shares.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The oldest glibc the Linux natives run on.
const GLIBC: &str = "2.17";

fn natives(target: &str, out: &Path) -> Result<()> {
    match target {
        "host" => {
            let lib = build_ffi(None)?;
            let platform = format!("{}-{}", env::consts::OS, env::consts::ARCH);
            stage(&lib, &out.join(platform))?;
        }
        // Each OS builds its own release targets: CI runs this once per OS
        // and merges the outputs.
        "all" => match env::consts::OS {
            "linux" => {
                for triple in ["x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"] {
                    natives(triple, out)?;
                }
            }
            "macos" => {
                let x86 = build_ffi(Some("x86_64-apple-darwin"))?;
                let arm = build_ffi(Some("aarch64-apple-darwin"))?;
                let dir = out.join("macos-universal");
                fresh_dir(&dir)?;
                let universal = dir.join(x86.file_name().unwrap());
                run(Command::new("lipo")
                    .arg("-create")
                    .arg(&x86)
                    .arg(&arm)
                    .arg("-output")
                    .arg(&universal))?;
                println!("staged {}", universal.display());
            }
            "windows" => natives("x86_64-pc-windows-msvc", out)?,
            other => bail!("no release targets for {other}"),
        },
        triple => {
            let (os, arch) = parse_triple(triple)?;
            let lib = build_ffi(Some(triple))?;
            stage(&lib, &out.join(format!("{os}-{arch}")))?;
        }
    }
    write_manifest(out)
}

/// Builds yakvc-cli for release on `triple` and copies it into `out` under
/// its release name, `yakvc-cli-<os>-<arch>`.
fn cli(triple: &str, out: &Path) -> Result<()> {
    let (os, arch) = parse_triple(triple)?;
    let mut cmd = if os == "linux" {
        // The runner's libasound.so needs a newer glibc than 2.17. Skip
        // checking it: at run time the user's libasound matches their glibc.
        let mut cmd = cargo(&["zigbuild", "--release", "--package", "yakvc-cli"]);
        cmd.arg(format!("--target={triple}.{GLIBC}"))
            .env("RUSTFLAGS", "-C link-arg=-Wl,--allow-shlib-undefined");
        cmd
    } else {
        let mut cmd = cargo(&["build", "--release", "--package", "yakvc-cli"]);
        cmd.arg(format!("--target={triple}"));
        cmd
    };
    run(&mut cmd)?;
    let exe = if os == "windows" { ".exe" } else { "" };
    let built = root().join(format!("target/{triple}/release/yakvc-cli{exe}"));
    let staged = out.join(format!("yakvc-cli-{os}-{arch}{exe}"));
    fs::create_dir_all(out)?;
    fs::copy(&built, &staged).with_context(|| format!("copying {}", built.display()))?;
    println!("staged {}", staged.display());
    Ok(())
}

/// Builds yakvc-ffi with the size-focused `dist` profile for `target` (or
/// the host) and returns the library's path. Linux targets are built with
/// cargo-zigbuild, so the library runs on any glibc since 2.17.
fn build_ffi(target: Option<&str>) -> Result<PathBuf> {
    let mut cmd = cargo(&["build", "--profile", "dist", "--package", "yakvc-ffi"]);
    let mut built = root().join("target");
    let os = match target {
        None => env::consts::OS,
        Some(triple) => {
            let (os, _) = parse_triple(triple)?;
            if os == "linux" {
                // alsa-sys finds the target's libasound with pkg-config,
                // which refuses cross builds unless allowed. Its search path
                // comes from PKG_CONFIG_PATH_<triple> (see flake.nix and the
                // CI workflow).
                cmd = cargo(&["zigbuild", "--profile", "dist", "--package", "yakvc-ffi"]);
                cmd.arg(format!("--target={triple}.{GLIBC}"))
                    .env("PKG_CONFIG_ALLOW_CROSS", "1");
            } else {
                cmd.arg(format!("--target={triple}"));
            }
            built.push(triple);
            os
        }
    };
    run(&mut cmd)?;
    built.push("dist");
    built.push(match os {
        "linux" => "libyakvc_ffi.so",
        "macos" => "libyakvc_ffi.dylib",
        "windows" => "yakvc_ffi.dll",
        other => bail!("unsupported OS {other}"),
    });
    Ok(built)
}

fn stage(lib: &Path, dir: &Path) -> Result<()> {
    fresh_dir(dir)?;
    let to = dir.join(lib.file_name().unwrap());
    fs::copy(lib, &to).with_context(|| format!("copying {}", lib.display()))?;
    println!("staged {}", to.display());
    Ok(())
}

/// An empty `dir`: a library from an earlier run with another name would
/// otherwise be shipped and listed in the manifest beside the new one.
fn fresh_dir(dir: &Path) -> Result<()> {
    if dir.exists() {
        fs::remove_dir_all(dir)?;
    }
    fs::create_dir_all(dir)?;
    Ok(())
}

/// Replaces `macos-x86_64` and `macos-aarch64` in `natives` with their
/// universal merge in `macos-universal`, as `natives --target all` makes on
/// a Mac. CI builds the two architectures in parallel and merges them here.
fn merge_macos(natives: &Path) -> Result<()> {
    let lib = "libyakvc_ffi.dylib";
    let (x86, arm) = (natives.join("macos-x86_64"), natives.join("macos-aarch64"));
    let dir = natives.join("macos-universal");
    fresh_dir(&dir)?;
    // Apple's lipo on a Mac; LLVM's, which reads and writes the same files, elsewhere.
    let lipo = if Command::new("lipo").arg("-info").output().is_ok() {
        "lipo"
    } else {
        "llvm-lipo"
    };
    run(Command::new(lipo)
        .arg("-create")
        .arg(x86.join(lib))
        .arg(arm.join(lib))
        .arg("-output")
        .arg(dir.join(lib)))?;
    fs::remove_dir_all(&x86)?;
    fs::remove_dir_all(&arm)?;
    println!("staged {}", dir.join(lib).display());
    write_manifest(natives)
}

fn parse_triple(triple: &str) -> Result<(&'static str, &'static str)> {
    let arch = match triple.split('-').next() {
        Some("x86_64") => "x86_64",
        Some("aarch64") => "aarch64",
        _ => bail!("unsupported architecture in {triple}"),
    };
    let os = if triple.contains("-linux-") {
        "linux"
    } else if triple.contains("-apple-darwin") {
        "macos"
    } else if triple.contains("-windows-") {
        "windows"
    } else {
        bail!("unsupported OS in {triple}")
    };
    Ok((os, arch))
}

/// `cargo <args> --locked` on the workspace. Locked, so a Cargo.toml that no
/// longer matches Cargo.lock fails instead of quietly re-resolving and
/// shipping a dependency graph the lock never recorded.
fn cargo(args: &[&str]) -> Command {
    let mut cmd = Command::new(env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    cmd.current_dir(root()).args(args).arg("--locked");
    cmd
}

fn run(cmd: &mut Command) -> Result<()> {
    let status = cmd
        .status()
        .with_context(|| format!("running {:?}", cmd.get_program()))?;
    ensure!(status.success(), "{cmd:?} failed");
    Ok(())
}

/// Runs `cmd` and returns its trimmed standard output.
fn output(cmd: &mut Command) -> Result<String> {
    let out = cmd
        .output()
        .with_context(|| format!("running {:?}", cmd.get_program()))?;
    ensure!(out.status.success(), "{cmd:?} failed");
    Ok(String::from_utf8(out.stdout)?.trim().to_owned())
}

/// Writes `natives.sha256` covering every staged library, in `sha256sum`
/// format with `/`-separated paths relative to `out`.
fn write_manifest(out: &Path) -> Result<()> {
    let mut entries = Vec::new();
    for platform in fs::read_dir(out)? {
        let platform = platform?;
        if !platform.file_type()?.is_dir() {
            continue;
        }
        for lib in fs::read_dir(platform.path())? {
            let lib = lib?;
            let hash: String = Sha256::digest(fs::read(lib.path())?)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            let rel = format!(
                "{}/{}",
                platform.file_name().to_string_lossy(),
                lib.file_name().to_string_lossy()
            );
            entries.push((rel, hash));
        }
    }
    entries.sort();
    let mut manifest = String::new();
    for (rel, hash) in entries {
        writeln!(manifest, "{hash}  {rel}")?;
    }
    fs::write(out.join("natives.sha256"), manifest)?;
    Ok(())
}

/// Packages the release jar from the staged natives and THIRD_PARTY_LICENSES
/// with the Gradle build, which also runs the mod's unit tests.
fn dist(from: Option<&Path>) -> Result<()> {
    let work = root().join("target/dist");
    let natives_dir = work.join("natives");
    if natives_dir.exists() {
        fs::remove_dir_all(&natives_dir)?;
    }
    match from {
        // Every CI artifact brings its own manifest, so copy only the
        // platform directories and write the manifest anew.
        Some(from) => {
            let platforms =
                fs::read_dir(from).with_context(|| format!("reading {}", from.display()))?;
            for platform in platforms {
                let platform = platform?;
                if !platform.file_type()?.is_dir() {
                    continue;
                }
                for lib in fs::read_dir(platform.path())? {
                    stage(&lib?.path(), &natives_dir.join(platform.file_name()))?;
                }
            }
            write_manifest(&natives_dir)?;
        }
        None => natives("host", &natives_dir)?,
    }

    let licenses = work.join("THIRD_PARTY_LICENSES");
    third_party_licenses("yakvc-ffi", &licenses)?;

    let gradlew = if cfg!(windows) {
        "gradlew.bat"
    } else {
        "gradlew"
    };
    run(Command::new(root().join("mod").join(gradlew))
        .current_dir(root().join("mod"))
        .arg("build")
        .arg(format!("-Pyakvc.prebuiltNatives={}", natives_dir.display()))
        .arg(format!("-Pyakvc.thirdPartyLicenses={}", licenses.display())))?;

    let jar = format!("yakvc-{VERSION}.jar");
    let dist = root().join("dist");
    fs::create_dir_all(&dist)?;
    fs::copy(root().join("mod/build/libs").join(&jar), dist.join(&jar))?;
    println!("wrote {}", dist.join(&jar).display());
    Ok(())
}

/// Builds a static yakvc-server and the Docker image around it, tagged
/// `yakvc-server:<version>` and `yakvc-server:latest`.
fn server_image() -> Result<()> {
    let triple = "x86_64-unknown-linux-musl";
    run(
        cargo(&["zigbuild", "--release", "--package", "yakvc-server"])
            .arg(format!("--target={triple}")),
    )?;

    let context = root().join("target/server-image");
    if context.exists() {
        fs::remove_dir_all(&context)?;
    }
    fs::create_dir_all(context.join("state"))?;
    let binary = root()
        .join("target")
        .join(triple)
        .join("release/yakvc-server");
    fs::copy(&binary, context.join("yakvc-server"))?;
    fs::copy(root().join("deploy/Dockerfile"), context.join("Dockerfile"))?;
    third_party_licenses("yakvc-server", &context.join("THIRD_PARTY_LICENSES"))?;

    let tag = format!("yakvc-server:{VERSION}");
    run(Command::new("docker")
        // `--pull` checks the pinned base against the registry, so a stale
        // local copy under the same tag can't slip into a release.
        .args([
            "build",
            "--pull",
            "--tag",
            &tag,
            "--tag",
            "yakvc-server:latest",
        ])
        .arg(&context))?;
    println!("built image {tag} (also yakvc-server:latest)");
    Ok(())
}

/// Writes the licence notices of everything `package` links, which the MIT,
/// Apache and BSD licences (libopus's among them) require to ship with it.
fn third_party_licenses(package: &str, out: &Path) -> Result<()> {
    fs::create_dir_all(out.parent().unwrap())?;
    run(cargo(&["about", "generate"])
        .args(["--fail", "--config", "about.toml"])
        .arg("--manifest-path")
        .arg(format!("crates/{package}/Cargo.toml"))
        .arg("--output-file")
        .arg(out)
        .arg("xtask/third-party-licenses.hbs"))
}

fn header(check: bool) -> Result<()> {
    let crate_dir = root().join("crates/yakvc-ffi");
    let config = cbindgen::Config::from_file(crate_dir.join("cbindgen.toml"))
        .map_err(|e| anyhow::anyhow!(e))?;
    let mut generated = Vec::new();
    cbindgen::Builder::new()
        .with_crate(&crate_dir)
        .with_config(config)
        .generate()?
        .write(&mut generated);

    let path = crate_dir.join("include/yakvc.h");
    let current = fs::read(&path).unwrap_or_default();
    if current == generated {
        return Ok(());
    }
    if check {
        bail!("{} is stale; run `cargo xtask header`", path.display());
    }
    fs::write(&path, generated)?;
    println!("wrote {}", path.display());
    Ok(())
}
