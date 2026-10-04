use std::path::PathBuf;

use clap::{Parser, Subcommand};

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
        Command::Run { .. }
        | Command::Keygen { .. }
        | Command::EndpointId { .. }
        | Command::IssuerId { .. } => {
            anyhow::bail!("not implemented yet")
        }
    }
}
