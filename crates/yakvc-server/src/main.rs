use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use anyhow::Context;
use clap::{Parser, Subcommand};
use tokio::signal::unix::{SignalKind, signal};
use yakvc_server::Config;
use yakvc_shared::{IssuerKey, SecretKey};

/// Yak VC rendezvous server.
#[derive(Debug, Parser)]
#[command(version)]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the server.
    Run {
        #[arg(long)]
        config: PathBuf,
    },
    /// Generate an endpoint key, or an issuer key with --issuer.
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
    let builder = Config::load(config)?.into_builder()?;
    let server = builder.spawn().await?;
    println!("endpoint id: {}", server.endpoint_id());
    println!("issuer id:   {}", server.issuer_id());
    if let Some(url) = server.relay_url() {
        println!("relay:       {url}");
    }
    // systemd stops services with SIGTERM; Ctrl-C sends SIGINT.
    let mut sigterm = signal(SignalKind::terminate())?;
    tokio::select! {
        result = tokio::signal::ctrl_c() => result?,
        _ = sigterm.recv() => {}
    }
    server.shutdown().await;
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
