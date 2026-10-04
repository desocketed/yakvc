//! Developer diagnostics for the parts automated tests can't reach: real
//! audio hardware and real networks. Wiring only: every command is a thin
//! layer over `yakvc-client` and `yakvc-audio`.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use clap::{Parser, Subcommand};
use yakvc_audio::{
    DeviceChoice, DeviceInfo, Encoder, FRAME_SAMPLES, FrameSink, FrameSource, InputConfig,
    InputProcessor, JitterConfig, Microphone, Mixer, NullSink, Packet, ReceiveStream, Spatial,
    Speakers, StreamStats, WavSource,
};

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
    match args.command {
        Command::Audio(AudioCommand::Devices) => devices(args.json),
        Command::Audio(AudioCommand::Loopback { wav, null_out }) => {
            loopback(wav, null_out, args.json)
        }
        Command::Call(_) => {
            anyhow::bail!("`call` needs a direct-call API that yakvc-client does not have yet")
        }
        Command::Net(NetCommand::Report) => {
            anyhow::bail!("`net report` needs a net report API that yakvc-client does not have yet")
        }
    }
}

fn devices(json: bool) -> anyhow::Result<()> {
    let devices = yakvc_audio::devices()?;
    if json {
        println!("{}", serde_json::to_string_pretty(&devices)?);
        return Ok(());
    }
    let print = |title: &str, list: &[DeviceInfo]| {
        println!("{title}:");
        for device in list {
            let default = if device.is_default { " (default)" } else { "" };
            println!("  {}{default}", device.name);
        }
    };
    print("Inputs", &devices.inputs);
    print("Outputs", &devices.outputs);
    Ok(())
}

/// Runs the whole audio pipeline on one machine until interrupted, printing
/// stats every second.
fn loopback(wav: Option<PathBuf>, null_out: bool, json: bool) -> anyhow::Result<()> {
    let mut source: Box<dyn FrameSource> = match wav {
        Some(path) => Box::new(WavSource::open(&path, true)?),
        None => Box::new(Microphone::open(&DeviceChoice::Default)?),
    };
    let mut sink: Box<dyn FrameSink> = if null_out {
        Box::new(NullSink::new().0)
    } else {
        Box::new(Speakers::open(&DeviceChoice::Default)?)
    };
    let mut processor = InputProcessor::new(InputConfig::default());
    let mut encoder = Encoder::new(24_000)?;
    let mut stream = ReceiveStream::new(JitterConfig::default());
    let mut mixer = Mixer::new();

    let mut captured = [0.0; FRAME_SAMPLES];
    let mut decoded = [0.0; FRAME_SAMPLES];
    let mut mixed = [[0.0; 2]; FRAME_SAMPLES];
    // `seq` counts packets sent and `ts` counts samples captured, so frames
    // skipped by DTX advance `ts` but not `seq`, as on the network.
    let mut seq = 0u32;
    let mut ts = 0u32;
    let mut level_db = f32::NEG_INFINITY;
    let mut next_report = Instant::now() + Duration::from_secs(1);

    loop {
        if source.read(&mut captured) {
            level_db = processor.process(&mut captured).level_db;
            if let Some(payload) = encoder.encode(&captured)? {
                let packet = Packet {
                    seq,
                    ts,
                    end_of_talk: false,
                    payload,
                };
                stream.push(packet, Instant::now());
                seq = seq.wrapping_add(1);
            }
            ts = ts.wrapping_add(FRAME_SAMPLES as u32);
        }
        if sink.wants_frame() {
            stream.pull(&mut decoded, Instant::now());
            let centred = Spatial {
                gain: 1.0,
                pan: 0.0,
            };
            mixer.mix([(&decoded, centred)], &mut mixed);
            sink.write(&mixed);
        }
        if Instant::now() >= next_report {
            print_stats(level_db, &stream.stats(), json);
            next_report += Duration::from_secs(1);
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn print_stats(level_db: f32, stats: &StreamStats, json: bool) {
    let playout_ms = stats.playout_delay.as_millis();
    if json {
        let line = serde_json::json!({
            "level_db": level_db,
            "received": stats.received,
            "late": stats.late,
            "fec_recovered": stats.fec_recovered,
            "concealed": stats.concealed,
            "playout_delay_ms": playout_ms,
        });
        println!("{line}");
    } else {
        println!(
            "level {level_db:6.1} dBFS | received {} late {} fec {} concealed {} | playout {playout_ms} ms",
            stats.received, stats.late, stats.fec_recovered, stats.concealed,
        );
    }
}
