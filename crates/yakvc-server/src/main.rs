use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use anyhow::Context;
use clap::{Parser, Subcommand};
use tokio::signal::unix::{SignalKind, signal};
use yakvc_proto::{IssuerKey, SecretKey};
use yakvc_server::Config;

/// Yak VC rendezvous server.
#[derive(Debug, Parser)]
#[command(version)]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the server. Creates the key files on first start.
    Run {
        #[arg(long)]
        config: PathBuf,
    },
    /// Generate an endpoint key, or an issuer key with --issuer. Optional:
    /// `run` creates missing keys itself.
    Keygen {
        #[arg(long)]
        issuer: bool,
        #[arg(long)]
        out: PathBuf,
    },
    /// Print the EndpointId of an endpoint key.
    EndpointId { key: PathBuf },
    /// Print the IssuerId of an issuer key.
    IssuerId { key: PathBuf },
}

fn main() -> anyhow::Result<()> {
    match Args::parse().command {
        Command::Run { config } => run(&config),
        Command::Keygen { issuer, out } => {
            let key = if issuer {
                IssuerKey::generate().to_bytes()
            } else {
                SecretKey::generate().to_bytes()
            };
            write_key(&out, &key)
        }
        Command::EndpointId { key } => {
            println!("{}", SecretKey::from_bytes(&read_key(&key)?).public());
            Ok(())
        }
        Command::IssuerId { key } => {
            println!("{}", IssuerKey::from_bytes(&read_key(&key)?).id());
            Ok(())
        }
    }
}

#[tokio::main]
async fn run(config: &Path) -> anyhow::Result<()> {
    let config = Config::load(config)?;
    create_missing_key(&config.endpoint_key, "endpoint", || {
        SecretKey::generate().to_bytes()
    })?;
    create_missing_key(&config.issuer_key, "issuer", || {
        IssuerKey::generate().to_bytes()
    })?;
    let builder = config.into_builder()?;
    let server = builder.spawn().await?;
    println!("endpoint id: {}", server.endpoint_id());
    println!("issuer id:   {}", server.issuer_id());
    if let Some(url) = server.relay_url() {
        println!("relay:       {url}");
    }
    notify_systemd_ready();
    // systemd stops services with SIGTERM; Ctrl-C sends SIGINT.
    let mut sigterm = signal(SignalKind::terminate())?;
    tokio::select! {
        result = tokio::signal::ctrl_c() => result?,
        _ = sigterm.recv() => {}
    }
    server.shutdown().await;
    Ok(())
}

/// Tells systemd that the server is up, for a `Type=notify` unit
/// (deploy/yakvc-server.service). Does nothing outside systemd, where
/// NOTIFY_SOCKET is unset.
fn notify_systemd_ready() {
    let Some(path) = std::env::var_os("NOTIFY_SOCKET").map(PathBuf::from) else {
        return;
    };
    let sent = std::os::unix::net::UnixDatagram::unbound()
        .and_then(|socket| socket.send_to(b"READY=1", &path));
    if let Err(e) = sent {
        eprintln!("could not notify systemd at {}: {e}", path.display());
    }
}

/// Creates a key file that doesn't exist yet, so a new server needs no setup.
/// Clients are configured with the ids these keys produce, so a key file lost
/// later would quietly become a new identity: say loudly that one was made.
fn create_missing_key(
    path: &Path,
    kind: &str,
    generate: impl FnOnce() -> [u8; 32],
) -> anyhow::Result<()> {
    let exists = path
        .try_exists()
        .with_context(|| format!("check {}", path.display()))?;
    if !exists {
        write_key(path, &generate())?;
        eprintln!(
            "created a new {kind} key at {}; back it up, because clients are configured with its id",
            path.display()
        );
    }
    Ok(())
}

/// Writes a new key file readable only by its owner. Never overwrites, so a
/// typo can't destroy a key in use.
fn write_key(path: &Path, key: &[u8; 32]) -> anyhow::Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("create {}", path.display()))?;
    file.write_all(key)
        .with_context(|| format!("write {}", path.display()))
}

fn read_key(path: &Path) -> anyhow::Result<[u8; 32]> {
    let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("{}: a key file holds 32 bytes", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_a_missing_key_once() {
        let dir = std::env::temp_dir().join(format!("yakvc-key-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("issuer.key");
        let _ = std::fs::remove_file(&path);

        create_missing_key(&path, "issuer", || [1; 32]).unwrap();
        create_missing_key(&path, "issuer", || [2; 32]).unwrap();

        assert_eq!(read_key(&path).unwrap(), [1; 32]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
