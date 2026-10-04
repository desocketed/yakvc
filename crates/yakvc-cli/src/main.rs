//! Headless Yak VC client and diagnostics. Wiring only: every command is a
//! thin layer over `yakvc-client` and `yakvc-audio`.

use std::path::PathBuf;

use clap::{Args as ClapArgs, Parser, Subcommand};

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
    /// Create a client key and print its EndpointId.
    Keygen {
        #[arg(long)]
        out: Option<PathBuf>,
    },
    #[command(subcommand)]
    Audio(AudioCommand),
    #[command(subcommand)]
    Call(CallCommand),
    #[command(subcommand)]
    Net(NetCommand),
    /// Full client against a dev-mode rendezvous. Reads commands from stdin:
    /// `pos X Y Z`, `ptt on|off`, `mute UUID`.
    Join {
        /// Rendezvous EndpointId.
        #[arg(long)]
        server: String,
        #[arg(long)]
        dev_uuid: String,
        /// Comma-separated UUIDs in the fake tab list.
        #[arg(long, value_delimiter = ',')]
        sees: Vec<String>,
        /// Starting position `x,y,z`.
        #[arg(long)]
        pos: Option<String>,
        #[command(flatten)]
        io: IoArgs,
        #[command(flatten)]
        impair: ImpairArgs,
    },
    /// N simulated players speaking a WAV file at random positions.
    Bot {
        #[arg(long)]
        count: usize,
        #[arg(long)]
        wav: PathBuf,
        /// Radius in blocks.
        #[arg(long)]
        spread: f64,
        #[command(flatten)]
        impair: ImpairArgs,
    },
    #[command(subcommand)]
    Ticket(TicketCommand),
}

#[derive(Debug, Subcommand)]
enum AudioCommand {
    /// List input and output devices.
    Devices,
    /// Mic → Opus → jitter buffer → speakers, with stats.
    Loopback {
        #[command(flatten)]
        io: IoArgs,
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

#[derive(Debug, Subcommand)]
enum TicketCommand {
    /// Decode and verify a cached ticket.
    Inspect { file: PathBuf },
}

#[derive(Debug, ClapArgs)]
struct IoArgs {
    /// Use a WAV file instead of the microphone.
    #[arg(long)]
    wav: Option<PathBuf>,
    /// Discard output instead of playing it.
    #[arg(long)]
    null_out: bool,
}

#[derive(Debug, ClapArgs)]
struct ImpairArgs {
    /// Loss probability, 0..1.
    #[arg(long, default_value_t = 0.0)]
    loss: f32,
    /// Added delay in ms.
    #[arg(long, default_value_t = 0)]
    delay: u64,
    /// Random extra delay in ms.
    #[arg(long, default_value_t = 0)]
    jitter: u64,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let _ = args.json;
    match args.command {
        Command::Keygen { .. }
        | Command::Audio(_)
        | Command::Call(_)
        | Command::Net(_)
        | Command::Join { .. }
        | Command::Bot { .. }
        | Command::Ticket(_) => anyhow::bail!("not implemented yet"),
    }
}
