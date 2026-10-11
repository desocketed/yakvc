//! `cargo xtask deploy-relay`: switch the production relay to a commit.
//!
//! The host's NixOS flake lives outside this repository and imports the
//! `yakvc` flake. The box is too small to compile the server, so the system
//! is built here (or on the CI runner) and pushed with `nixos-rebuild
//! switch --target-host`. CI runs this after every release with the
//! `RELAY_*` secrets; by hand it takes the same values as flags.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, ensure};

use crate::run;

pub(crate) fn deploy(
    flake: &str,
    target: &str,
    key: &Path,
    known_hosts: &Path,
    rev: &str,
) -> Result<()> {
    let key = key.canonicalize().context("ssh key")?;
    let known_hosts = known_hosts.canonicalize().context("known_hosts")?;
    let ssh_opts = format!(
        "-i {} -o IdentitiesOnly=yes -o UserKnownHostsFile={}",
        key.display(),
        known_hosts.display()
    );

    // Nix fetches the flake with git and copies the system with ssh; both
    // need the key, and nothing else from the runner's ssh setup.
    run(Command::new("nix")
        .args([
            "run",
            "nixpkgs#nixos-rebuild",
            "--",
            "switch",
            "--flake",
            flake,
        ])
        .args([
            "--override-input",
            "yakvc",
            &format!("github:desocketed/yakvc/{rev}"),
        ])
        .args(["--target-host", target, "--use-remote-sudo"])
        .env("GIT_SSH_COMMAND", format!("ssh {ssh_opts}"))
        .env("NIX_SSHOPTS", &ssh_opts))?;

    let status = Command::new("ssh")
        .args(ssh_opts.split(' '))
        .args([target, "systemctl", "is-active", "yakvc-server"])
        .status()
        .context("running ssh")?;
    ensure!(status.success(), "yakvc-server is not active on {target}");
    Ok(())
}
