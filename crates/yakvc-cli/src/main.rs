use clap::Parser;

/// Headless Yak VC client and diagnostics.
#[derive(Debug, Parser)]
#[command(version)]
struct Args {}

fn main() -> anyhow::Result<()> {
    let _args = Args::parse();
    anyhow::bail!("not implemented yet")
}
