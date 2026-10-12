//! `cargo xtask dev`: a dev-mode yakvc-server for `runClient` and local
//! tests, or with `--real-auth` one that checks accounts with Mojang, for the
//! manual real-account check in `docs/DEVELOPMENT.md`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, ensure};

use crate::{cargo, output, root};

/// A dev rendezvous ready to start: its server binary, its config and the
/// client config that points at it.
pub(crate) struct DevServer {
    pub(crate) server: PathBuf,
    pub(crate) config: PathBuf,
    pub(crate) client: String,
}

/// Where `--real-auth` serves its counters.
const METRICS: &str = "127.0.0.1:9187";

/// Builds yakvc-server, creates keys in `dir` (kept across runs, so the
/// printed client config stays valid), writes `dir/client.toml` and runs the
/// server in the foreground until Ctrl-C. With `real_auth` it also builds
/// the mod jar to try it with.
pub fn dev(
    port: u16,
    dir: Option<PathBuf>,
    write_client_config: bool,
    real_auth: bool,
) -> Result<()> {
    let root = root();
    let default = if real_auth {
        "target/real-auth"
    } else {
        "target/dev-server"
    };
    let dir = dir.unwrap_or_else(|| root.join(default));
    let DevServer {
        server,
        config,
        client,
    } = prepare(port, &dir, real_auth)?;
    if real_auth {
        let status = Command::new(root.join("mod/gradlew"))
            .args(["build", "--console=plain", "--quiet"])
            .current_dir(root.join("mod"))
            .status()?;
        ensure!(status.success(), "building the mod failed");
        println!(
            "Mod jar: {}",
            root.join(format!("mod/build/libs/yakvc-{}.jar", crate::VERSION))
                .display()
        );
        println!("Server counters: curl -s http://{METRICS}/metrics | grep -E 'auth|mojang'\n");
    }
    if write_client_config {
        let target = root.join("mod/run/config/yakvc/client.toml");
        fs::create_dir_all(target.parent().unwrap())?;
        fs::write(&target, &client)?;
        println!("Wrote {}", target.display());
    }
    println!("Client config (config/yakvc/client.toml):\n\n{client}");
    let kind = if real_auth { "Real-auth" } else { "Dev" };
    println!("{kind} yakvc-server on 127.0.0.1:{port}, Ctrl-C to stop.");

    run(Command::new(&server)
        .arg("run")
        .arg("--config")
        .arg(&config))
}

/// Builds yakvc-server and writes the keys, its config and `dir/client.toml`.
/// Without `real_auth` the server takes dev tickets and the client accepts
/// them; with it, clients prove their account with Mojang, and the client
/// config leaves out `dev_mode`, so it only accepts a real ticket.
pub(crate) fn prepare(port: u16, dir: &Path, real_auth: bool) -> Result<DevServer> {
    let root = root();
    fs::create_dir_all(dir)?;
    let dir = dir.canonicalize()?;

    let status = cargo(&["build", "--quiet", "-p", "yakvc-server"]).status()?;
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
    let auth = if real_auth {
        format!("metrics_bind = \"{METRICS}\"\n")
    } else {
        "insecure_dev_auth = true\n".to_owned()
    };
    fs::write(
        &config,
        format!(
            "endpoint_key = {}\nissuer_key = {}\nbind = \"127.0.0.1:{port}\"\n{auth}",
            toml_path(&endpoint_key),
            toml_path(&issuer_key),
        ),
    )?;

    let dev_mode = if real_auth { "" } else { "dev_mode = true\n" };
    let client = format!(
        "{dev_mode}trusted_issuers = [\"{issuer_id}\"]\n\n\
         [rendezvous]\nendpoint_id = \"{endpoint_id}\"\naddrs = [\"127.0.0.1:{port}\"]\n"
    );
    fs::write(dir.join("client.toml"), &client)?;
    Ok(DevServer {
        server,
        config,
        client,
    })
}

/// A TOML basic string for a path: backslashes (Windows) and quotes escaped.
fn toml_path(path: &Path) -> String {
    let escaped = path
        .display()
        .to_string()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    format!("\"{escaped}\"")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toml_path_escapes_backslashes_and_quotes() {
        assert_eq!(toml_path(Path::new("/a/b")), "\"/a/b\"");
        assert_eq!(
            toml_path(Path::new("C:\\x\\it's \"here\"")),
            "\"C:\\\\x\\\\it's \\\"here\\\"\""
        );
    }
}
