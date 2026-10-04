use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::{env, fs};

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
    /// Build yakvc-ffi in release mode and stage it for the mod.
    Natives {
        /// `host`, `all`, or a Rust target triple.
        #[arg(long, default_value = "host")]
        target: String,
        /// Staging directory; gets `<os>-<arch>/` subdirectories and a
        /// `natives.sha256` manifest.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Regenerate crates/yakvc-ffi/include/yakvc.h with cbindgen.
    Header {
        /// Fail instead of writing if the checked-in header is stale.
        #[arg(long)]
        check: bool,
    },
    /// Build the release jar into dist/.
    Dist,
    /// Build the yakvc-server Docker image.
    ServerImage,
    /// Start a dev-mode server and print client config.
    Dev,
}

fn main() -> Result<()> {
    match Args::parse().command {
        Cmd::Natives { target, out } => natives(
            &target,
            &out.unwrap_or_else(|| root().join("mod/build/natives")),
        ),
        Cmd::Header { check } => header(check),
        Cmd::Dist => bail!("`dist` is not implemented yet (M6)"),
        Cmd::ServerImage => bail!("`server-image` is not implemented yet (M6)"),
        Cmd::Dev => bail!("`dev` is not implemented yet (M3)"),
    }
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn natives(target: &str, out: &Path) -> Result<()> {
    let target = match target {
        "all" => bail!("`--target all` is not implemented yet (M6); use CI's per-platform builds"),
        "host" => None,
        triple => Some(triple),
    };

    let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let mut cmd = Command::new(cargo);
    cmd.current_dir(root())
        .args(["build", "--release", "--package", "yakvc-ffi"]);
    if let Some(triple) = target {
        cmd.args(["--target", triple]);
    }
    ensure!(cmd.status()?.success(), "cargo build failed");

    let (os, arch) = match target {
        None => (env::consts::OS, env::consts::ARCH),
        Some(triple) => parse_triple(triple)?,
    };
    let (prefix, ext) = match os {
        "linux" => ("lib", "so"),
        "macos" => ("lib", "dylib"),
        "windows" => ("", "dll"),
        other => bail!("unsupported OS {other}"),
    };
    let file = format!("{prefix}yakvc_ffi.{ext}");
    let mut built = root().join("target");
    if let Some(triple) = target {
        built.push(triple);
    }
    built.push("release");
    built.push(&file);

    let dir = out.join(format!("{os}-{arch}"));
    fs::create_dir_all(&dir)?;
    fs::copy(&built, dir.join(&file)).with_context(|| format!("copying {}", built.display()))?;
    write_manifest(out)?;
    println!("staged {}", dir.join(&file).display());
    Ok(())
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
