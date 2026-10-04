use clap::Parser;

/// Yak VC rendezvous server.
#[derive(Debug, Parser)]
#[command(version)]
struct Args {}

fn main() -> anyhow::Result<()> {
    let _args = Args::parse();
    anyhow::bail!("not implemented yet")
}
