//! Voice protocol: send/receive loop, recipient selection and spatial input.
//!
//! The network side runs one task per peer ([`serve_peer`]), which feeds
//! received frames to the audio thread and tells the peer whether we want
//! its audio. The audio thread ([`AudioLoop`]) captures, encodes and sends
//! frames to the recipients [`rules`] picks, and mixes what it receives.

mod rules;

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc as std_mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use bytes::Bytes;
use tokio::sync::watch;
use yakvc_audio::{
    DeviceChoice, Encoder, FRAME_SAMPLES, FrameSink, FrameSource, InputConfig, InputProcessor,
    JitterConfig, Microphone, Mixer, MonoFrame, Packet, Pulled, ReceiveStream, Speakers,
    StereoFrame, StreamStats,
};
use yakvc_shared::voice::{VERSION, VoiceHeader, VoiceMsg};
use yakvc_shared::{ProtocolId, Uuid};

use crate::config::{AudioConfig, Config};
use crate::engine::PeerAudio;
use crate::event::{Event, EventSender};
use crate::net::{ControlRecv, Peer, PeerLink, Protocol};
use crate::world::{Input, World};

pub(crate) use self::rules::VoiceState;
use self::rules::{Candidate, plan_send};

/// How often the audio thread wakes to move frames.
const AUDIO_TICK: Duration = Duration::from_millis(5);
/// Mic level is reported every this many frames, about 10 times a second.
const LEVEL_EVERY_FRAMES: u32 = 5;
/// A remote speaker counts as stopped after this many silent frames.
const TALKING_HANGOVER_FRAMES: u32 = 10;

/// The voice engine half: the [`Protocol`] for peers plus the audio thread.
#[derive(Debug)]
pub(crate) struct Voice {
    shared: Arc<Shared>,
    stop: Arc<AtomicBool>,
    audio_thread: Option<JoinHandle<()>>,
}

/// Where the audio thread gets its frames and plays them.
pub(crate) struct AudioIo {
    /// `None` opens the configured microphone.
    pub source: Option<Box<dyn FrameSource>>,
    /// `None` opens the configured speakers.
    pub sink: Option<Box<dyn FrameSink>>,
    #[cfg(feature = "sim")]
    pub impairment: Option<crate::sim::Impairment>,
}

struct Shared {
    state: Mutex<VoiceState>,
    /// Peers with an open voice link.
    peers: Mutex<HashMap<Uuid, VoicePeer>>,
    to_audio: Mutex<std_mpsc::Sender<ToAudio>>,
    /// Bumped whenever what we want to hear may have changed, so each peer
    /// task re-sends `ReceiveState` if needed.
    receive_policy: watch::Sender<()>,
    stats: Mutex<HashMap<Uuid, StreamStats>>,
    events: EventSender,
    #[cfg(feature = "sim")]
    impairment: Option<crate::sim::Impairment>,
}

struct VoicePeer {
    peer: Peer,
    /// The peer's latest `ReceiveState`.
    wants_audio: Arc<AtomicBool>,
}

/// Messages from the network tasks and the engine to the audio thread.
enum ToAudio {
    Frame {
        from: Uuid,
        header: VoiceHeader,
        payload: Bytes,
        arrived: Instant,
    },
    PeerGone(Uuid),
    /// The configured devices changed.
    ReopenDevices,
}

impl Voice {
    /// Starts the audio thread.
    pub(crate) fn start(config: &Config, io: AudioIo, events: EventSender) -> Voice {
        let (to_audio, from_net) = std_mpsc::channel();
        let shared = Arc::new(Shared {
            state: Mutex::new(VoiceState::new(
                config.audio.clone(),
                config.voice_range,
                config.friends_only.clone(),
            )),
            peers: Mutex::new(HashMap::new()),
            to_audio: Mutex::new(to_audio),
            receive_policy: watch::Sender::new(()),
            stats: Mutex::new(HashMap::new()),
            events,
            #[cfg(feature = "sim")]
            impairment: io.impairment.clone(),
        });
        let stop = Arc::new(AtomicBool::new(false));
        let audio_thread = std::thread::Builder::new()
            .name("yakvc-audio".into())
            .spawn({
                let shared = shared.clone();
                let stop = stop.clone();
                // Built on the audio thread, so a failing audio stack can't
                // stop the engine from starting.
                move || AudioLoop::new(shared, from_net, io, stop).run()
            })
            .expect("spawn audio thread");
        Voice {
            shared,
            stop,
            audio_thread: Some(audio_thread),
        }
    }

    pub(crate) fn protocol(&self) -> Arc<dyn Protocol> {
        Arc::new(VoiceProtocol(self.shared.clone()))
    }

    pub(crate) fn set_own_uuid(&self, uuid: Uuid) {
        self.shared.state.lock().unwrap().own_uuid = Some(uuid);
    }

    pub(crate) fn set_world(&self, world: World) {
        self.shared.state.lock().unwrap().world = world;
    }

    pub(crate) fn set_input(&self, input: Input) {
        self.shared.state.lock().unwrap().input = input;
        self.shared.receive_policy.send_replace(());
    }

    pub(crate) fn set_game_volume(&self, volume: f32) {
        self.shared.state.lock().unwrap().game_volume = volume.clamp(0.0, 1.0);
    }

    pub(crate) fn set_peer_audio(&self, uuid: Uuid, audio: PeerAudio) {
        self.shared
            .state
            .lock()
            .unwrap()
            .peer_audio
            .insert(uuid, audio);
        self.shared.receive_policy.send_replace(());
    }

    pub(crate) fn set_direct(&self, direct: bool) {
        self.shared.state.lock().unwrap().direct = direct;
    }

    pub(crate) fn update_config(&self, config: &Config) {
        let devices_changed = {
            let mut state = self.shared.state.lock().unwrap();
            let changed = state.audio.input_device != config.audio.input_device
                || state.audio.output_device != config.audio.output_device;
            state.audio = config.audio.clone();
            state.voice_range = config.voice_range;
            state.friends_only = config.friends_only.clone();
            changed
        };
        if devices_changed {
            self.shared.send_to_audio(ToAudio::ReopenDevices);
        }
        self.shared.receive_policy.send_replace(());
    }

    pub(crate) fn stats(&self, uuid: Uuid) -> Option<StreamStats> {
        self.shared.stats.lock().unwrap().get(&uuid).cloned()
    }

    /// Stops the audio thread and waits for it.
    pub(crate) fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.audio_thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Voice {
    fn drop(&mut self) {
        self.stop();
    }
}

impl Shared {
    fn send_to_audio(&self, msg: ToAudio) {
        // Fails only once the audio thread has stopped.
        let _ = self.to_audio.lock().unwrap().send(msg);
    }
}

struct VoiceProtocol(Arc<Shared>);

impl Protocol for VoiceProtocol {
    fn id(&self) -> ProtocolId {
        ProtocolId::VOICE
    }

    fn version(&self) -> u16 {
        VERSION
    }

    fn serve(&self, link: PeerLink) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(serve_peer(self.0.clone(), link))
    }
}

/// Serves one peer's voice link: hands received frames to the audio thread
/// and keeps the peer informed of whether we want its audio.
async fn serve_peer(shared: Arc<Shared>, link: PeerLink) {
    let PeerLink {
        peer,
        mut datagrams,
        mut control_send,
        control_recv,
    } = link;
    let uuid = peer.uuid();
    let link_id = peer.link_id();
    let wants_audio = Arc::new(AtomicBool::new(true));
    shared.peers.lock().unwrap().insert(
        uuid,
        VoicePeer {
            peer,
            wants_audio: wants_audio.clone(),
        },
    );
    let reader = tokio::spawn(read_receive_state(control_recv, wants_audio));
    let mut receive_policy = shared.receive_policy.subscribe();
    #[cfg(feature = "sim")]
    let mut impairer = shared.impairment.clone().map(crate::sim::Impairer::new);

    // Peers assume we want their audio until told otherwise.
    let mut told_wants_audio = true;
    loop {
        let wants = shared.state.lock().unwrap().wants_audio_from(uuid);
        if wants != told_wants_audio {
            let msg = VoiceMsg::ReceiveState { wants_audio: wants };
            if control_send.send(&msg).await.is_err() {
                break;
            }
            told_wants_audio = wants;
        }
        tokio::select! {
            datagram = datagrams.recv() => {
                let Some(datagram) = datagram else { break };
                #[cfg(feature = "sim")]
                if let Some(impairer) = &mut impairer {
                    deliver_impaired(&shared, uuid, datagram, impairer);
                    continue;
                }
                deliver(&shared, uuid, datagram);
            }
            changed = receive_policy.changed() => {
                if changed.is_err() {
                    break;
                }
            }
        }
    }

    reader.abort();
    let mut peers = shared.peers.lock().unwrap();
    // A newer link to the same player may have replaced this one.
    if peers
        .get(&uuid)
        .is_some_and(|p| p.peer.link_id() == link_id)
    {
        peers.remove(&uuid);
        shared.send_to_audio(ToAudio::PeerGone(uuid));
    }
}

async fn read_receive_state(mut control: ControlRecv, wants_audio: Arc<AtomicBool>) {
    while let Some(msg) = control.recv::<VoiceMsg>().await {
        match msg {
            VoiceMsg::ReceiveState { wants_audio: wants } => {
                wants_audio.store(wants, Ordering::Relaxed);
            }
        }
    }
}

/// Passes a received datagram to the audio thread.
fn deliver(shared: &Shared, from: Uuid, datagram: Bytes) {
    let Some((header, _)) = VoiceHeader::parse(&datagram) else {
        return;
    };
    shared.send_to_audio(ToAudio::Frame {
        from,
        header,
        payload: datagram.slice(VoiceHeader::LEN..),
        arrived: Instant::now(),
    });
}

/// [`deliver`], after the configured loss, delay and duplication.
#[cfg(feature = "sim")]
fn deliver_impaired(
    shared: &Arc<Shared>,
    from: Uuid,
    datagram: Bytes,
    impairer: &mut crate::sim::Impairer,
) {
    for delay in impairer.delays() {
        if delay.is_zero() {
            deliver(shared, from, datagram.clone());
        } else {
            let shared = shared.clone();
            let datagram = datagram.clone();
            tokio::spawn(async move {
                tokio::time::sleep(delay).await;
                deliver(&shared, from, datagram);
            });
        }
    }
}

/// The audio thread: capture → encode → send, and receive → decode → mix →
/// play. Sources and sinks are paced by their own clocks and never block,
/// so the loop just polls them.
struct AudioLoop {
    shared: Arc<Shared>,
    from_net: std_mpsc::Receiver<ToAudio>,
    stop: Arc<AtomicBool>,
    source: Option<Box<dyn FrameSource>>,
    sink: Option<Box<dyn FrameSink>>,
    /// The caller supplied the source/sink, so config changes don't reopen it.
    custom_source: bool,
    custom_sink: bool,
    input: InputProcessor,
    input_config: InputConfig,
    encoder: Option<Encoder>,
    mixer: Mixer,
    remotes: HashMap<Uuid, Remote>,
    /// The previous frame was sent.
    transmitting: bool,
    seq: u32,
    /// Capture time of the next frame, in samples.
    ts: u32,
    bitrate: u32,
    frames_since_level: u32,
}

/// One remote speaker's receive state.
struct Remote {
    stream: ReceiveStream,
    talking: bool,
    silent_frames: u32,
}

impl AudioLoop {
    fn new(
        shared: Arc<Shared>,
        from_net: std_mpsc::Receiver<ToAudio>,
        io: AudioIo,
        stop: Arc<AtomicBool>,
    ) -> AudioLoop {
        let audio = shared.state.lock().unwrap().audio.clone();
        let input_config = input_config(&audio);
        AudioLoop {
            custom_source: io.source.is_some(),
            custom_sink: io.sink.is_some(),
            source: io.source,
            sink: io.sink,
            input: InputProcessor::new(input_config.clone()),
            input_config,
            encoder: None,
            mixer: Mixer::new(),
            remotes: HashMap::new(),
            transmitting: false,
            seq: 0,
            ts: 0,
            bitrate: audio.bitrate,
            frames_since_level: 0,
            shared,
            from_net,
            stop,
        }
    }

    fn run(mut self) {
        self.open_devices();
        self.encoder = match Encoder::new(self.bitrate) {
            Ok(encoder) => Some(encoder),
            Err(err) => {
                self.error(format!("cannot start the voice encoder: {err}"));
                None
            }
        };
        let mut frame: MonoFrame = [0.0; FRAME_SAMPLES];
        while !self.stop.load(Ordering::Relaxed) {
            while self
                .source
                .as_mut()
                .is_some_and(|source| source.read(&mut frame))
            {
                self.capture(&mut frame);
            }
            while let Ok(msg) = self.from_net.try_recv() {
                self.handle(msg);
            }
            while self.sink.as_ref().is_some_and(|sink| sink.wants_frame()) {
                self.play();
            }
            std::thread::sleep(AUDIO_TICK);
        }
    }

    /// Opens the configured microphone and speakers, unless the caller
    /// supplied its own. A missing device is reported but not fatal: the
    /// player can still hear without a microphone, and vice versa.
    fn open_devices(&mut self) {
        let audio = self.shared.state.lock().unwrap().audio.clone();
        if !self.custom_source {
            self.source = match Microphone::open(&device_choice(&audio.input_device)) {
                Ok(mic) => Some(Box::new(mic)),
                Err(err) => {
                    self.error(format!("cannot open the microphone: {err}"));
                    None
                }
            };
        }
        if !self.custom_sink {
            self.sink = match Speakers::open(&device_choice(&audio.output_device)) {
                Ok(speakers) => Some(Box::new(speakers)),
                Err(err) => {
                    self.error(format!("cannot open the speakers: {err}"));
                    None
                }
            };
        }
    }

    fn capture(&mut self, frame: &mut MonoFrame) {
        let state = self.shared.state.lock().unwrap().clone();
        let input_config = input_config(&state.audio);
        if input_config != self.input_config {
            self.input.set_config(input_config.clone());
            self.input_config = input_config;
        }
        let activity = self.input.process(frame);
        self.frames_since_level += 1;
        if self.frames_since_level >= LEVEL_EVERY_FRAMES {
            self.frames_since_level = 0;
            self.shared.events.send(Event::MicLevel(activity.level_db));
        }

        let transmitting = state.transmitting(activity.voice);
        if transmitting != self.transmitting {
            self.shared.events.send(Event::Talking {
                uuid: state.own_uuid.unwrap_or_default(),
                talking: transmitting,
            });
        }
        // The frame after the talk spurt ends goes out too, flagged
        // `end_of_talk`, so receivers can reset their decoder.
        if transmitting || self.transmitting {
            self.send(frame, &state, !transmitting);
        }
        self.transmitting = transmitting;
        self.ts = self.ts.wrapping_add(FRAME_SAMPLES as u32);
    }

    fn send(&mut self, frame: &MonoFrame, state: &VoiceState, end_of_talk: bool) {
        let Some(encoder) = &mut self.encoder else {
            return;
        };
        let peers = self.shared.peers.lock().unwrap();
        let candidates = peers
            .values()
            .filter(|p| p.wants_audio.load(Ordering::Relaxed))
            .filter_map(|p| {
                let uuid = p.peer.uuid();
                Some(Candidate {
                    uuid,
                    distance: state.send_distance(uuid)?,
                    relayed: p.peer.is_relayed(),
                })
            })
            .collect();
        let plan = plan_send(candidates, state.audio.bitrate);
        for p in peers.values().filter(|p| p.peer.is_relayed()) {
            p.peer
                .set_relay_full(plan.relay_full.contains(&p.peer.uuid()));
        }
        if plan.bitrate != self.bitrate {
            encoder.set_bitrate(plan.bitrate);
            self.bitrate = plan.bitrate;
        }

        let payload = match encoder.encode(frame) {
            Ok(Some(payload)) => payload,
            // DTX decided this frame is silence.
            Ok(None) => return,
            Err(err) => {
                let message = format!("voice encoder failed: {err}");
                self.shared.events.send(Event::Error(message));
                return;
            }
        };
        let header = VoiceHeader {
            seq: self.seq,
            ts: self.ts,
            end_of_talk,
        };
        let mut datagram = Vec::with_capacity(VoiceHeader::LEN + payload.len());
        datagram.extend_from_slice(&header.to_bytes());
        datagram.extend_from_slice(payload);
        for uuid in &plan.recipients {
            // A peer that just closed simply misses the frame.
            let _ = peers[uuid].peer.send_datagram(&datagram);
        }
        self.seq = self.seq.wrapping_add(1);
        if end_of_talk {
            encoder.reset();
        }
    }

    fn handle(&mut self, msg: ToAudio) {
        match msg {
            ToAudio::Frame {
                from,
                header,
                payload,
                arrived,
            } => {
                // With nothing to play to, buffering would only grow.
                if self.sink.is_none() {
                    return;
                }
                let remote = self.remotes.entry(from).or_insert_with(|| Remote {
                    stream: ReceiveStream::new(JitterConfig::default()),
                    talking: false,
                    silent_frames: TALKING_HANGOVER_FRAMES,
                });
                let packet = Packet {
                    seq: header.seq,
                    ts: header.ts,
                    end_of_talk: header.end_of_talk,
                    payload: &payload,
                };
                remote.stream.push(packet, arrived);
            }
            ToAudio::PeerGone(uuid) => {
                if self.remotes.remove(&uuid).is_some_and(|r| r.talking) {
                    self.talking(uuid, false);
                }
                self.shared.stats.lock().unwrap().remove(&uuid);
            }
            ToAudio::ReopenDevices => self.open_devices(),
        }
    }

    fn play(&mut self) {
        let now = Instant::now();
        let state = self.shared.state.lock().unwrap().clone();
        let mut voices: Vec<(MonoFrame, yakvc_audio::Spatial)> = Vec::new();
        let mut talking_changes = Vec::new();
        for (&uuid, remote) in &mut self.remotes {
            let mut frame: MonoFrame = [0.0; FRAME_SAMPLES];
            let pulled = remote.stream.pull(&mut frame, now);
            let spatial = (pulled == Pulled::Voice)
                .then(|| state.playback(uuid))
                .flatten();
            if let Some(spatial) = spatial {
                voices.push((frame, spatial));
                remote.silent_frames = 0;
            } else {
                remote.silent_frames = remote.silent_frames.saturating_add(1);
            }
            let talking = remote.silent_frames < TALKING_HANGOVER_FRAMES;
            if talking != remote.talking {
                remote.talking = talking;
                talking_changes.push((uuid, talking));
            }
        }
        for (uuid, talking) in talking_changes {
            self.talking(uuid, talking);
        }

        self.mixer.set_master_gain(state.game_volume);
        let mut out: StereoFrame = [[0.0; 2]; FRAME_SAMPLES];
        self.mixer.mix(
            voices.iter().map(|(frame, spatial)| (frame, *spatial)),
            &mut out,
        );
        if let Some(sink) = &mut self.sink {
            sink.write(&out);
        }

        let mut stats = self.shared.stats.lock().unwrap();
        for (&uuid, remote) in &self.remotes {
            stats.insert(uuid, remote.stream.stats());
        }
    }

    fn talking(&self, uuid: Uuid, talking: bool) {
        self.shared.events.send(Event::Talking { uuid, talking });
    }

    fn error(&self, message: String) {
        self.shared.events.send(Event::Error(message));
    }
}

fn input_config(audio: &AudioConfig) -> InputConfig {
    InputConfig {
        gain: audio.input_gain,
        noise_suppression: audio.noise_suppression,
        vad_threshold_db: audio.vad_threshold_db,
        ..InputConfig::default()
    }
}

fn device_choice(name: &Option<String>) -> DeviceChoice {
    match name {
        Some(name) => DeviceChoice::Named(name.clone()),
        None => DeviceChoice::Default,
    }
}

impl std::fmt::Debug for Shared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shared").finish_non_exhaustive()
    }
}

impl std::fmt::Debug for AudioIo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioIo")
            .field("source", &self.source.is_some())
            .field("sink", &self.sink.is_some())
            .finish_non_exhaustive()
    }
}
