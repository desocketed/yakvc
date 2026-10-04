//! Developer diagnostics for the parts automated tests can't reach: real
//! audio hardware and real networks. Wiring only: every command is a thin
//! layer over `yakvc-client` and `yakvc-audio`.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use clap::{Parser, Subcommand};
use yakvc_audio::{
    DeviceChoice, DeviceInfo, Encoder, FRAME_SAMPLES, FrameSink, FrameSource, InputConfig,
    InputProcessor, JitterConfig, Microphone, Mixer, NullSink, Packet, ReceiveStream,
    SilenceSource, Spatial, Speakers, StreamStats, WavSource,
};
use yakvc_client::{Config, EndpointAddr, Engine, Event, Input, PeerInfo, Uuid};

#[derive(Debug, Parser)]
#[command(name = "yakvc", version)]
struct Args {
    /// Machine-readable output.
    #[arg(long, global = true)]
    json: bool,
    /// For `call` and `net`: a `client.toml` to start the engine with. Its
    /// `[rendezvous]` relay is the only relay used, so without one a call
    /// has no relay fallback and `net report` finds no public address.
    #[arg(long, global = true)]
    config: Option<PathBuf>,
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

/// A two-machine call with no rendezvous or Minecraft account. Both sides
/// transmit all the time and print each peer's path, RTT and loss every
/// second until Ctrl-C.
#[derive(Debug, Subcommand)]
enum CallCommand {
    /// Wait for a call, printing the address to dial.
    Listen,
    /// Call the address printed by `call listen`.
    Dial {
        /// The listener's address: Iroh's `EndpointAddr` as JSON, quoted
        /// for the shell. For example:
        ///
        /// '{"id":"<endpoint id>","addrs":[{"Relay":"https://relay.example.com/"},{"Ip":"192.0.2.1:4433"}]}'
        addr: String,
    },
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
        Command::Call(command) => {
            let dial = match command {
                CallCommand::Listen => None,
                CallCommand::Dial { addr } => Some(parse_addr(&addr)?),
            };
            let config = load_config(args.config.as_deref())?;
            runtime()?.block_on(call(config, dial, args.json))
        }
        Command::Net(NetCommand::Report) => {
            let config = load_config(args.config.as_deref())?;
            runtime()?.block_on(net_report(config, args.json))
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

/// The loopback's input, kept concrete so the stats can show overruns.
#[allow(clippy::large_enum_variant, reason = "one, for the whole run")]
enum LoopbackInput {
    Microphone(Microphone),
    Wav(WavSource),
}

/// The loopback's output, kept concrete so the stats can show underruns.
#[allow(clippy::large_enum_variant, reason = "one, for the whole run")]
enum LoopbackOutput {
    Speakers(Speakers),
    Null(NullSink),
}

impl LoopbackInput {
    fn source(&mut self) -> &mut dyn FrameSource {
        match self {
            LoopbackInput::Microphone(mic) => mic,
            LoopbackInput::Wav(wav) => wav,
        }
    }

    fn overruns(&self) -> Option<u64> {
        match self {
            LoopbackInput::Microphone(mic) => Some(mic.overruns()),
            LoopbackInput::Wav(_) => None,
        }
    }
}

impl LoopbackOutput {
    fn sink(&mut self) -> &mut dyn FrameSink {
        match self {
            LoopbackOutput::Speakers(speakers) => speakers,
            LoopbackOutput::Null(null) => null,
        }
    }

    fn underruns(&self) -> Option<u64> {
        match self {
            LoopbackOutput::Speakers(speakers) => Some(speakers.underruns()),
            LoopbackOutput::Null(_) => None,
        }
    }
}

/// Runs the whole audio pipeline on one machine until interrupted, printing
/// stats every second.
fn loopback(wav: Option<PathBuf>, null_out: bool, json: bool) -> anyhow::Result<()> {
    let mut input = match wav {
        Some(path) => LoopbackInput::Wav(WavSource::open(&path, true)?),
        None => LoopbackInput::Microphone(Microphone::open(&DeviceChoice::Default)?),
    };
    let mut output = if null_out {
        LoopbackOutput::Null(NullSink::new().0)
    } else {
        LoopbackOutput::Speakers(Speakers::open(&DeviceChoice::Default)?)
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
        if input.source().read(&mut captured) {
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
        let sink = output.sink();
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
            let devices = DeviceHealth {
                overruns: input.overruns(),
                underruns: output.underruns(),
            };
            print_stats(level_db, &stream.stats(), &devices, json);
            next_report += Duration::from_secs(1);
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Glitch counts since the devices opened. `None` when the file input or
/// null output stands in for the device.
struct DeviceHealth {
    overruns: Option<u64>,
    underruns: Option<u64>,
}

fn print_stats(level_db: f32, stats: &StreamStats, devices: &DeviceHealth, json: bool) {
    let playout_ms = stats.playout_delay.as_millis();
    if json {
        let line = serde_json::json!({
            "level_db": level_db,
            "received": stats.received,
            "late": stats.late,
            "fec_recovered": stats.fec_recovered,
            "concealed": stats.concealed,
            "playout_delay_ms": playout_ms,
            "overruns": devices.overruns,
            "underruns": devices.underruns,
        });
        println!("{line}");
    } else {
        let count = |n: Option<u64>| n.map_or("-".to_owned(), |n| n.to_string());
        println!(
            "level {level_db:6.1} dBFS | received {} late {} fec {} concealed {} | playout {playout_ms} ms | overruns {} underruns {}",
            stats.received,
            stats.late,
            stats.fec_recovered,
            stats.concealed,
            count(devices.overruns),
            count(devices.underruns),
        );
    }
}

/// Runs a direct call until Ctrl-C: dials `dial`, or listens if it is `None`.
async fn call(config: Config, dial: Option<EndpointAddr>, json: bool) -> anyhow::Result<()> {
    let data_dir = temp_data_dir();
    let (engine, mut events) = Engine::builder(config).data_dir(&data_dir).start()?;
    let name = if dial.is_some() { "caller" } else { "listener" };
    engine.enable_direct_calls(Uuid::new_v4(), name);
    // A terminal has no push-to-talk key, so the microphone stays open.
    engine.set_input(Input {
        push_to_talk: true,
        ..Input::default()
    });
    let listening = dial.is_none();
    if let Some(addr) = dial {
        engine.call(addr);
    }

    let mut shown_addr = None;
    let mut every_second = tokio::time::interval(Duration::from_secs(1));
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);
    loop {
        tokio::select! {
            _ = &mut ctrl_c => break,
            Some(event) = events.next() => print_event(&event),
            _ = every_second.tick() => {
                // The address gains public and relay entries as the
                // endpoint learns them, so show it again when it changes.
                let addr = engine.endpoint_addr();
                if listening && shown_addr.as_ref() != Some(&addr) {
                    print_addr(&addr, json)?;
                    shown_addr = Some(addr);
                }
                for peer in engine.peers() {
                    print_peer(&peer, json);
                }
            }
        }
    }

    drop(engine);
    let _ = std::fs::remove_dir_all(&data_dir);
    Ok(())
}

async fn net_report(config: Config, json: bool) -> anyhow::Result<()> {
    let data_dir = temp_data_dir();
    // Silent test audio I/O: a network report should not open the microphone.
    let (engine, _events) = Engine::builder(config)
        .data_dir(&data_dir)
        .audio_source(SilenceSource::new())
        .audio_sink(NullSink::new().0)
        .start()?;
    eprintln!("Measuring, this takes a few seconds…");
    let report = engine.net_report().await;
    drop(engine);
    let _ = std::fs::remove_dir_all(&data_dir);

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("{report}");
    }
    Ok(())
}

/// Parses the address format that `call listen` prints.
fn parse_addr(text: &str) -> anyhow::Result<EndpointAddr> {
    serde_json::from_str(text).map_err(|err| {
        anyhow::anyhow!(
            "not an address printed by `yakvc call listen` ({err}); quote it for the shell"
        )
    })
}

fn print_addr(addr: &EndpointAddr, json: bool) -> anyhow::Result<()> {
    let text = serde_json::to_string(addr)?;
    if json {
        println!("{}", serde_json::json!({ "addr": addr }));
    } else {
        println!("Listening. On the other machine, run:\n  yakvc call dial '{text}'");
    }
    Ok(())
}

/// Peer, connection state and log messages go to stderr, keeping stdout for
/// the per-second stats.
fn print_event(event: &Event) {
    match event {
        Event::Peer { .. } | Event::Rendezvous(_) | Event::Error(_) => eprintln!("{event:?}"),
        // Talking and mic level are too frequent to be useful here, and
        // the CLI has no Mojang session to answer join requests with.
        Event::Talking { .. } | Event::MicLevel(_) | Event::JoinRequest { .. } => {}
    }
}

fn print_peer(peer: &PeerInfo, json: bool) {
    let rtt_ms = peer.rtt.map(|rtt| rtt.as_secs_f64() * 1000.0);
    let stream = peer.stream.as_ref();
    // Frames that had to be rebuilt by FEC or concealed, out of all frames
    // due from this peer so far.
    let loss = stream.map(|s| {
        let missing = s.fec_recovered + s.concealed;
        missing as f64 / (s.received + missing).max(1) as f64
    });
    if json {
        let line = serde_json::json!({
            "name": peer.name,
            "uuid": peer.uuid,
            "state": format!("{:?}", peer.state),
            "rtt_ms": rtt_ms,
            "loss": loss,
            "received": stream.map(|s| s.received),
            "fec_recovered": stream.map(|s| s.fec_recovered),
            "concealed": stream.map(|s| s.concealed),
            "late": stream.map(|s| s.late),
            "playout_delay_ms": stream.map(|s| s.playout_delay.as_millis()),
        });
        println!("{line}");
    } else {
        let rtt = rtt_ms.map_or("-".to_owned(), |ms| format!("{ms:.0} ms"));
        let loss = loss.map_or("-".to_owned(), |loss| format!("{:.1}%", loss * 100.0));
        println!("{}: {:?} | rtt {rtt} | loss {loss}", peer.name, peer.state);
    }
}

fn load_config(path: Option<&Path>) -> anyhow::Result<Config> {
    let Some(path) = path else {
        return Ok(Config::default());
    };
    let toml = std::fs::read_to_string(path)
        .map_err(|err| anyhow::anyhow!("read {}: {err}", path.display()))?;
    Ok(Config::from_toml(&toml)?)
}

/// A fresh key per run: the CLI has no persistent identity to keep.
fn temp_data_dir() -> PathBuf {
    std::env::temp_dir().join(format!("yakvc-cli-{}", std::process::id()))
}

/// For waiting on events and Ctrl-C. The engine runs its own runtime.
fn runtime() -> anyhow::Result<tokio::runtime::Runtime> {
    Ok(tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn printed_addresses_parse_back() {
        let id = "ae58ff8833241ac82d6ff7611046ed67b5072d142c588d0063e942d9a75502b6"
            .parse()
            .unwrap();
        let addr = EndpointAddr::new(id)
            .with_ip_addr("192.0.2.1:4433".parse().unwrap())
            .with_relay_url("https://relay.example.com".parse().unwrap());
        let text = serde_json::to_string(&addr).unwrap();
        // The format `call dial --help` documents.
        assert_eq!(
            text,
            format!(
                r#"{{"id":"{id}","addrs":[{{"Relay":"https://relay.example.com/"}},{{"Ip":"192.0.2.1:4433"}}]}}"#
            )
        );
        assert_eq!(parse_addr(&text).unwrap(), addr);
        assert!(parse_addr("not json").is_err());
    }
}
