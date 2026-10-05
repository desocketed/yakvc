//! `cargo xtask dev`: a dev-mode yakvc-server for `runClient` and local tests.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, ensure};

use crate::root;

/// Builds yakvc-server, creates keys in `dir` (kept across runs, so the
/// printed client config stays valid), writes `dir/client.toml` and runs the
/// server in the foreground until Ctrl-C.
pub fn dev(port: u16, dir: Option<PathBuf>, write_client_config: bool) -> Result<()> {
    let root = root();
    let dir = dir.unwrap_or_else(|| root.join("target/dev-server"));
    fs::create_dir_all(&dir)?;
    let dir = dir.canonicalize()?;

    let status = Command::new("cargo")
        .args(["build", "--quiet", "-p", "yakvc-server"])
        .current_dir(&root)
        .status()?;
    ensure!(status.success(), "building yakvc-server failed");
    let server = root
        .join("target/debug/yakvc-server")
        .with_extension(std::env::consts::EXE_EXTENSION);

    let endpoint_key = dir.join("endpoint.key");
    let issuer_key = dir.join("issuer.key");
    for (key, issuer) in [(&endpoint_key, false), (&issuer_key, true)] {
        if !key.exists() {
            let mut keygen = Command::new(&server);
            keygen.arg("keygen").arg("--out").arg(key);
            if issuer {
                keygen.arg("--issuer");
            }
            ensure!(keygen.status()?.success(), "keygen failed");
        }
    }
    let endpoint_id = output(Command::new(&server).arg("endpoint-id").arg(&endpoint_key))?;
    let issuer_id = output(Command::new(&server).arg("issuer-id").arg(&issuer_key))?;

    let config = dir.join("config.toml");
    fs::write(
        &config,
        format!(
            "endpoint_key = {}\nissuer_key = {}\nbind = \"127.0.0.1:{port}\"\ninsecure_dev_auth = true\n",
            toml_path(&endpoint_key),
            toml_path(&issuer_key),
        ),
    )?;

    let client = format!(
        "dev_mode = true\ntrusted_issuers = [\"{issuer_id}\"]\n\n\
         [rendezvous]\nendpoint_id = \"{endpoint_id}\"\naddrs = [\"127.0.0.1:{port}\"]\n"
    );
    fs::write(dir.join("client.toml"), &client)?;
    if write_client_config {
        let target = root.join("mod/run/config/yakvc/client.toml");
        fs::create_dir_all(target.parent().unwrap())?;
        fs::write(&target, &client)?;
        println!("Wrote {}", target.display());
    }
    println!("Client config (config/yakvc/client.toml):\n\n{client}");
    println!("Dev yakvc-server on 127.0.0.1:{port}, Ctrl-C to stop.");

    run(Command::new(&server)
        .arg("run")
        .arg("--config")
        .arg(&config))
}

fn output(cmd: &mut Command) -> Result<String> {
    let out = cmd.output()?;
    ensure!(out.status.success(), "{cmd:?} failed");
    Ok(String::from_utf8(out.stdout)?.trim().to_owned())
}

/// A TOML string for a path; single quotes keep Windows backslashes literal.
fn toml_path(path: &Path) -> String {
    format!("'{}'", path.display())
}

/// Replaces this process with the server on Unix, so a script that starts
/// `xtask dev` in the background can stop the server by its pid.
#[cfg(unix)]
fn run(cmd: &mut Command) -> Result<()> {
    use std::os::unix::process::CommandExt;
    Err(cmd.exec()).context("starting yakvc-server")
}

#[cfg(not(unix))]
fn run(cmd: &mut Command) -> Result<()> {
    let status = cmd.status().context("starting yakvc-server")?;
    ensure!(status.success(), "yakvc-server exited with {status}");
    Ok(())
}
