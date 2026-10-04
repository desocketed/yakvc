//! Developer diagnostics for the parts automated tests can't reach: real
//! audio hardware and real networks. Wiring only: every command is a thin
//! layer over `yakvc-client` and `yakvc-audio`.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "yakvc", version)]
struct Args {
    /// Machine-readable output.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    #[command(subcommand)]
    Audio(AudioCommand),
    #[command(subcommand)]
    Call(CallCommand),
    #[command(subcommand)]
    Net(NetCommand),
}

#[derive(Debug, Subcommand)]
enum AudioCommand {
    /// List input and output devices.
    Devices,
    /// Mic → Opus → jitter buffer → speakers, with stats.
    Loopback {
        /// Use a WAV file instead of the microphone.
        #[arg(long)]
        wav: Option<PathBuf>,
        /// Discard output instead of playing it.
        #[arg(long)]
        null_out: bool,
    },
}

#[derive(Debug, Subcommand)]
enum CallCommand {
    /// Wait for a direct call (no rendezvous or auth).
    Listen,
    /// Call an endpoint printed by `call listen`.
    Dial { addr: String },
}

#[derive(Debug, Subcommand)]
enum NetCommand {
    /// NAT type, IPv4/IPv6 reachability and relay latency.
    Report,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let _ = args.json;
    match args.command {
        Command::Audio(_) | Command::Call(_) | Command::Net(_) => {
            anyhow::bail!("not implemented yet")
        }
    }
}
