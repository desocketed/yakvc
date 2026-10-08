//! Voice protocol: send/receive loop, recipient selection and spatial input.
//!
//! The network side runs one task per peer ([`serve_peer`]), which feeds
//! received frames to the audio thread and tells the peer whether we want
//! its audio. The audio thread ([`AudioLoop`]) captures, encodes and sends
//! frames to the recipients [`rules`] picks, and mixes what it receives.

mod rules;

use std::collections::{HashMap, HashSet};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc as std_mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use bytes::Bytes;
use tokio::sync::watch;
use yakvc_audio::{
    AudioError, DeviceChoice, Encoder, FRAME_SAMPLES, FrameSink, FrameSource, InputConfig,
    InputProcessor, JitterConfig, Microphone, Mixer, MonoFrame, Packet, Pulled, ReceiveStream,
    Speakers, StereoFrame, StreamStats,
};
use yakvc_shared::voice::{VERSION, VoiceHeader, VoiceMsg};
use yakvc_shared::{GroupAnnounce, GroupId, GroupKey, ProtocolId, Uuid};

use crate::config::{AudioConfig, Config};
use crate::engine::PeerAudio;
use crate::event::{Event, EventSender};
use crate::net::{ControlRecv, Peer, PeerLink, Protocol};
use crate::world::{Input, World};

pub(crate) use self::rules::VoiceState;
use self::rules::{OwnGroup, plan_send};

/// How often the audio thread wakes to move frames.
const AUDIO_TICK: Duration = Duration::from_millis(5);
/// Mic level is reported every this many frames, about 10 times a second.
const LEVEL_EVERY_FRAMES: u32 = 5;
/// A remote speaker counts as stopped after this many silent frames.
const TALKING_HANGOVER_FRAMES: u32 = 10;

/// The audio thread's own state, for the in-game debug overlay.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AudioStats {
    pub microphone_open: bool,
    pub speakers_open: bool,
    /// The Opus bitrate in use, which drops below the configured one when
    /// the relay budget is tight.
    pub bitrate: u32,
    pub transmitting: bool,
    /// Microphone overruns since it was last opened.
    pub overruns: u64,
    /// Speaker underruns since they were last opened.
    pub underruns: u64,
}

/// A group someone on the server is in, for the voice menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupInfo {
    pub id: GroupId,
    pub name: String,
    /// It has a password.
    pub locked: bool,
    /// Who announced it, us included. A peer's claim isn't checked unless
    /// we are in the group too.
    pub members: Vec<Uuid>,
    /// We are in it.
    pub joined: bool,
}

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
    to_audio: Mutex<std_mpsc::SyncSender<Frame>>,
    /// Bumped whenever what we want to hear may have changed, so each peer
    /// task re-sends `ReceiveState` if needed.
    receive_policy: watch::Sender<()>,
    stats: Mutex<HashMap<Uuid, StreamStats>>,
    audio_stats: Mutex<AudioStats>,
    events: EventSender,
    #[cfg(feature = "sim")]
    impairment: Option<crate::sim::Impairment>,
}

struct VoicePeer {
    peer: Peer,
    /// The peer's latest `ReceiveState`.
    wants_audio: Arc<AtomicBool>,
    /// The group the peer last announced.
    group: Option<GroupAnnounce>,
}

/// A received frame, on its way from a peer task to the audio thread.
struct Frame {
    from: Uuid,
    header: VoiceHeader,
    payload: Bytes,
    arrived: Instant,
}

/// Frames waiting for the audio thread: 32 talkers for the longest playout
/// delay (200 ms), with room to spare. Frames that waited longer would be
/// too late to play, so while the audio thread is stalled (say, opening a
/// slow device) the rest are dropped.
const AUDIO_QUEUE: usize = 512;

impl Voice {
    /// Starts the audio thread.
    pub(crate) fn start(config: &Config, io: AudioIo, events: EventSender) -> Voice {
        let (to_audio, from_net) = std_mpsc::sync_channel(AUDIO_QUEUE);
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
            audio_stats: Mutex::new(AudioStats::default()),
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
        let mut state = self.shared.state.lock().unwrap();
        state.set_world(world, Instant::now());
    }

    /// The audio thread notices the change and reopens the speakers if
    /// they follow the game.
    pub(crate) fn set_game_device(&self, name: Option<&str>) {
        self.shared.state.lock().unwrap().game_device = name.map(str::to_owned);
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

    /// Joins `key`'s group, leaving any other, with nearby voice on.
    pub(crate) fn join_group(&self, key: GroupKey, locked: bool) {
        self.shared.state.lock().unwrap().group = Some(OwnGroup {
            key,
            locked,
            nearby: true,
        });
        self.shared.refresh_group_mates();
        // Peer tasks announce the new group.
        self.shared.receive_policy.send_replace(());
    }

    pub(crate) fn leave_group(&self) {
        self.shared.state.lock().unwrap().group = None;
        self.shared.refresh_group_mates();
        self.shared.receive_policy.send_replace(());
    }

    /// Whether nearby players outside the group are heard and sent to.
    /// Ignored outside a group.
    pub(crate) fn set_group_nearby(&self, nearby: bool) {
        if let Some(group) = &mut self.shared.state.lock().unwrap().group {
            group.nearby = nearby;
        }
        self.shared.receive_policy.send_replace(());
    }

    /// Every group we know of: ours, and those our peers announce.
    pub(crate) fn groups(&self) -> Vec<GroupInfo> {
        let state = self.shared.state.lock().unwrap().clone();
        let mut groups: Vec<GroupInfo> = Vec::new();
        let mut add = |id: GroupId, name: &str, locked: bool, member: Uuid| match groups
            .iter_mut()
            .find(|g| g.id == id)
        {
            Some(group) => group.members.push(member),
            None => groups.push(GroupInfo {
                id,
                name: name.to_owned(),
                locked,
                members: vec![member],
                joined: false,
            }),
        };
        if let (Some(own), Some(me)) = (&state.group, state.own_uuid) {
            add(own.key.id(), own.key.name(), own.locked, me);
        }
        for (&uuid, peer) in self.shared.peers.lock().unwrap().iter() {
            if let Some(announce) = &peer.group {
                add(announce.id, &announce.name, announce.locked, uuid);
            }
        }
        for group in &mut groups {
            group.joined = state
                .group
                .as_ref()
                .is_some_and(|own| own.key.id() == group.id);
            group.members.sort();
        }
        // A stable order, so a menu showing the list only changes when it does.
        groups.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
        groups
    }

    pub(crate) fn group_mates(&self) -> HashSet<Uuid> {
        self.shared.state.lock().unwrap().group_mates.clone()
    }

    pub(crate) fn set_direct(&self, direct: bool) {
        self.shared.state.lock().unwrap().direct = direct;
    }

    /// The audio thread reopens devices whose configured name changed.
    pub(crate) fn update_config(&self, config: &Config) {
        {
            let mut state = self.shared.state.lock().unwrap();
            state.audio = config.audio.clone();
            state.voice_range = config.voice_range;
            state.friends_only = config.friends_only.clone();
        }
        self.shared.receive_policy.send_replace(());
    }

    pub(crate) fn stats(&self, uuid: Uuid) -> Option<StreamStats> {
        self.shared.stats.lock().unwrap().get(&uuid).cloned()
    }

    pub(crate) fn audio_stats(&self) -> AudioStats {
        self.shared.audio_stats.lock().unwrap().clone()
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
    fn send_to_audio(&self, frame: Frame) {
        // Fails when the queue is full (see `AUDIO_QUEUE`) or the audio
        // thread has stopped; a lost frame is concealed like any other.
        let _ = self.to_audio.lock().unwrap().try_send(frame);
    }

    /// Works out again which peers are in our group, after an announcement,
    /// a peer leaving, or a change of our own group.
    fn refresh_group_mates(&self) {
        let announced: Vec<(Uuid, GroupAnnounce)> = self
            .peers
            .lock()
            .unwrap()
            .iter()
            .filter_map(|(&uuid, peer)| Some((uuid, peer.group.clone()?)))
            .collect();
        let mut state = self.state.lock().unwrap();
        let mates: HashSet<Uuid> = match &state.group {
            Some(own) => announced
                .iter()
                .filter(|(uuid, announce)| own.key.has_member(*uuid, announce))
                .map(|(uuid, _)| *uuid)
                .collect(),
            None => HashSet::new(),
        };
        if mates != state.group_mates {
            state.group_mates = mates;
            drop(state);
            self.receive_policy.send_replace(());
        }
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
/// and keeps the peer informed of whether we want its audio and which group
/// we are in.
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
            group: None,
        },
    );
    let reader = tokio::spawn(read_control(
        shared.clone(),
        uuid,
        control_recv,
        wants_audio,
    ));
    let mut receive_policy = shared.receive_policy.subscribe();
    #[cfg(feature = "sim")]
    let mut impairer = shared.impairment.clone().map(crate::sim::Impairer::new);

    // Peers assume we want their audio, and that we are in no group, until
    // told otherwise.
    let mut told_wants_audio = true;
    let mut told_group = None;
    loop {
        let (wants, group) = {
            let state = shared.state.lock().unwrap();
            (state.wants_audio_from(uuid), state.announcement())
        };
        if wants != told_wants_audio {
            let msg = VoiceMsg::ReceiveState { wants_audio: wants };
            if control_send.send(&msg).await.is_err() {
                break;
            }
            told_wants_audio = wants;
        }
        if group != told_group {
            if control_send
                .send(&VoiceMsg::Group(group.clone()))
                .await
                .is_err()
            {
                break;
            }
            told_group = group;
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
    let removed = {
        let mut peers = shared.peers.lock().unwrap();
        // A newer link to the same player may have replaced this one.
        let current = peers
            .get(&uuid)
            .is_some_and(|p| p.peer.link_id() == link_id);
        current && peers.remove(&uuid).is_some()
    };
    // The audio thread notices the peer is gone by itself.
    if removed {
        shared.refresh_group_mates();
    }
}

async fn read_control(
    shared: Arc<Shared>,
    uuid: Uuid,
    mut control: ControlRecv,
    wants_audio: Arc<AtomicBool>,
) {
    while let Some(msg) = control.recv::<VoiceMsg>().await {
        match msg {
            VoiceMsg::ReceiveState { wants_audio: wants } => {
                wants_audio.store(wants, Ordering::Relaxed);
            }
            VoiceMsg::Group(group) => {
                if let Some(peer) = shared.peers.lock().unwrap().get_mut(&uuid) {
                    peer.group = group;
                }
                shared.refresh_group_mates();
            }
        }
    }
}

/// Passes a received datagram to the audio thread.
fn deliver(shared: &Shared, from: Uuid, datagram: Bytes) {
    let Some((header, _)) = VoiceHeader::parse(&datagram) else {
        return;
    };
    shared.send_to_audio(Frame {
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
    from_net: std_mpsc::Receiver<Frame>,
    stop: Arc<AtomicBool>,
    source: Device<Box<dyn FrameSource>>,
    sink: Device<Box<dyn FrameSink>>,
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
    /// Its latest frame had the `group` flag: it is speaking to our group.
    group: bool,
    talking: bool,
    silent_frames: u32,
}

impl AudioLoop {
    fn new(
        shared: Arc<Shared>,
        from_net: std_mpsc::Receiver<Frame>,
        io: AudioIo,
        stop: Arc<AtomicBool>,
    ) -> AudioLoop {
        let state = shared.state.lock().unwrap().clone();
        let audio = &state.audio;
        let input_config = input_config(audio);
        AudioLoop {
            source: Device::new("microphone", io.source, state.input_device()),
            sink: Device::new("speakers", io.sink, state.output_device()),
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
        self.encoder = match Encoder::new(self.bitrate) {
            Ok(encoder) => Some(encoder),
            Err(err) => {
                self.error(format!("cannot start the voice encoder: {err}"));
                None
            }
        };
        let mut frame: MonoFrame = [0.0; FRAME_SAMPLES];
        while !self.stop.load(Ordering::Relaxed) {
            self.maintain_devices();
            if self.source.io.is_none() && self.transmitting {
                self.end_spurt_without_microphone();
            }
            while self
                .source
                .io
                .as_mut()
                .is_some_and(|source| source.read(&mut frame))
            {
                self.capture(&mut frame);
            }
            while let Ok(frame) = self.from_net.try_recv() {
                self.receive(frame);
            }
            self.drop_gone_remotes();
            while self.sink.io.as_ref().is_some_and(|sink| sink.wants_frame()) {
                self.play();
            }
            *self.shared.audio_stats.lock().unwrap() = self.audio_stats();
            std::thread::sleep(AUDIO_TICK);
        }
    }

    fn audio_stats(&self) -> AudioStats {
        AudioStats {
            microphone_open: self.source.io.is_some(),
            speakers_open: self.sink.io.is_some(),
            bitrate: self.bitrate,
            transmitting: self.transmitting,
            overruns: self.source.io.as_ref().map_or(0, |s| s.glitches()),
            underruns: self.sink.io.as_ref().map_or(0, |s| s.glitches()),
        }
    }

    /// Opens, reopens and retries the microphone and speakers. A missing or
    /// failed device is reported but not fatal: the player can still hear
    /// without a microphone, and vice versa.
    fn maintain_devices(&mut self) {
        let now = Instant::now();
        let (input, output) = {
            let state = self.shared.state.lock().unwrap();
            (state.input_device(), state.output_device())
        };
        let failure = self.source.io.as_ref().and_then(|source| source.failure());
        let problem = self.source.maintain(failure, &input, now, |choice| {
            Microphone::open(choice).map(|mic| Box::new(mic) as Box<dyn FrameSource>)
        });
        if let Some(problem) = problem {
            self.error(problem);
        }
        let failure = self.sink.io.as_ref().and_then(|sink| sink.failure());
        let problem = self.sink.maintain(failure, &output, now, |choice| {
            Speakers::open(choice).map(|speakers| Box::new(speakers) as Box<dyn FrameSink>)
        });
        if let Some(problem) = problem {
            self.error(problem);
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
        // The frame after the talk spurt ends goes out too, flagged
        // `end_of_talk`, so receivers can reset their decoder.
        if transmitting || self.transmitting {
            self.send(frame, &state, !transmitting);
        }
        self.set_transmitting(transmitting, &state);
        self.ts = self.ts.wrapping_add(FRAME_SAMPLES as u32);
    }

    /// Ends the talk spurt when the microphone fails during it, since no
    /// frame will come to end it.
    fn end_spurt_without_microphone(&mut self) {
        let state = self.shared.state.lock().unwrap().clone();
        self.send(&[0.0; FRAME_SAMPLES], &state, true);
        self.set_transmitting(false, &state);
        self.ts = self.ts.wrapping_add(FRAME_SAMPLES as u32);
    }

    fn set_transmitting(&mut self, transmitting: bool, state: &VoiceState) {
        if transmitting == self.transmitting {
            return;
        }
        self.transmitting = transmitting;
        self.shared.events.send(Event::Talking {
            uuid: state.own_uuid.unwrap_or_default(),
            talking: transmitting,
        });
        if !transmitting {
            // Only `send` updates the relay budget, so nothing else would
            // clear it while we are quiet.
            for p in self.shared.peers.lock().unwrap().values() {
                p.peer.set_relay_full(false);
            }
        }
    }

    fn send(&mut self, frame: &MonoFrame, state: &VoiceState, end_of_talk: bool) {
        let Some(encoder) = &mut self.encoder else {
            return;
        };
        let peers = self.shared.peers.lock().unwrap();
        let candidates = peers
            .values()
            .filter(|p| p.wants_audio.load(Ordering::Relaxed))
            .filter_map(|p| state.send_to(p.peer.uuid(), p.peer.is_relayed()))
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
            // DTX decided this frame is silence. The end of talk still goes
            // out, header only, or receivers would never see the spurt end.
            Ok(None) if end_of_talk => &[],
            Ok(None) => return,
            Err(err) => {
                let message = format!("voice encoder failed: {err}");
                self.shared.events.send(Event::Error(message));
                if end_of_talk {
                    encoder.reset();
                }
                return;
            }
        };
        // The same frame, with and without the `group` flag.
        let datagram = |group: bool| {
            let header = VoiceHeader {
                seq: self.seq,
                ts: self.ts,
                end_of_talk,
                group,
            };
            let mut datagram = Vec::with_capacity(VoiceHeader::LEN + payload.len());
            datagram.extend_from_slice(&header.to_bytes());
            datagram.extend_from_slice(payload);
            datagram
        };
        let (nearby, group) = (datagram(false), datagram(true));
        for uuid in &plan.recipients {
            let datagram = if state.is_group_mate(*uuid) {
                &group
            } else {
                &nearby
            };
            // A peer that just closed simply misses the frame.
            let _ = peers[uuid].peer.send_datagram(datagram);
        }
        self.seq = self.seq.wrapping_add(1);
        if end_of_talk {
            encoder.reset();
        }
    }

    fn receive(&mut self, frame: Frame) {
        // With nothing to play to, buffering would only grow.
        if self.sink.io.is_none() {
            return;
        }
        let remote = self.remotes.entry(frame.from).or_insert_with(|| Remote {
            stream: ReceiveStream::new(JitterConfig::default()),
            group: false,
            talking: false,
            silent_frames: TALKING_HANGOVER_FRAMES,
        });
        let packet = Packet {
            seq: frame.header.seq,
            ts: frame.header.ts,
            end_of_talk: frame.header.end_of_talk,
            payload: &frame.payload,
        };
        remote.group = frame.header.group;
        remote.stream.push(packet, frame.arrived);
    }

    /// Forgets remote speakers whose voice link has closed. Checked here
    /// rather than sent as a message, so a full frame queue can't lose it.
    fn drop_gone_remotes(&mut self) {
        let gone: Vec<Uuid> = {
            let peers = self.shared.peers.lock().unwrap();
            self.remotes
                .keys()
                .filter(|uuid| !peers.contains_key(uuid))
                .copied()
                .collect()
        };
        for uuid in gone {
            if self.remotes.remove(&uuid).is_some_and(|r| r.talking) {
                self.talking(uuid, false);
            }
            self.shared.stats.lock().unwrap().remove(&uuid);
        }
    }

    fn play(&mut self) {
        let now = Instant::now();
        let mut state = self.shared.state.lock().unwrap().clone();
        state.world = state.world_at(now);
        let mut voices: Vec<(MonoFrame, yakvc_audio::Spatial)> = Vec::new();
        let mut talking_changes = Vec::new();
        for (&uuid, remote) in &mut self.remotes {
            let mut frame: MonoFrame = [0.0; FRAME_SAMPLES];
            let pulled = remote.stream.pull(&mut frame, now);
            let spatial = match (pulled, remote.group) {
                (Pulled::Voice, true) => state.group_playback(uuid),
                (Pulled::Voice, false) => state.playback(uuid),
                (Pulled::Silence, _) => None,
            };
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
        if let Some(sink) = &mut self.sink.io {
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

/// The microphone or the speakers. A device the audio loop opened itself
/// is reopened when the wanted device changes, and retried every
/// [`REOPEN_INTERVAL`] while it is missing or has failed. One the caller
/// supplied is never reopened.
struct Device<T> {
    /// "microphone" or "speakers", for messages.
    name: &'static str,
    io: Option<T>,
    /// What `io` was opened (or last tried) as. `None` when the caller
    /// supplied `io`.
    opened_as: Option<DeviceChoice>,
    retry_at: Instant,
    /// The current problem has been reported. Cleared once the device works,
    /// so each outage is reported once rather than at every retry.
    reported: bool,
}

/// How often a missing or failed device is tried again.
const REOPEN_INTERVAL: Duration = Duration::from_secs(2);

impl<T> Device<T> {
    /// The caller's `io`, or else our own device `wanted`, opened on the
    /// first [`maintain`](Self::maintain).
    fn new(name: &'static str, io: Option<T>, wanted: DeviceChoice) -> Self {
        Device {
            name,
            opened_as: io.is_none().then_some(wanted),
            io,
            retry_at: Instant::now(),
            reported: false,
        }
    }

    /// Drops `io` once it reports a `failure`, and opens our own device if
    /// it is due for a retry or `wanted` changed. Returns a problem to
    /// report, if it is a new one.
    fn maintain(
        &mut self,
        failure: Option<String>,
        wanted: &DeviceChoice,
        now: Instant,
        open: impl FnOnce(&DeviceChoice) -> Result<T, AudioError>,
    ) -> Option<String> {
        let mut problem = None;
        if let Some(reason) = failure {
            self.io = None;
            self.retry_at = now + REOPEN_INTERVAL;
            problem = Some(format!("the {} stopped working: {reason}", self.name));
        }
        if let Some(opened_as) = &self.opened_as {
            let changed = opened_as != wanted;
            if changed || (self.io.is_none() && now >= self.retry_at) {
                self.opened_as = Some(wanted.clone());
                self.io = None;
                match open(wanted) {
                    Ok(io) => {
                        self.io = Some(io);
                        self.reported = false;
                    }
                    Err(err) => {
                        self.retry_at = now + REOPEN_INTERVAL;
                        // A different device is a new problem.
                        self.reported &= !changed;
                        problem = Some(format!("cannot open the {}: {err}", self.name));
                    }
                }
            }
        }
        if problem.is_some() && !self.reported {
            self.reported = true;
            return problem;
        }
        None
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

#[cfg(test)]
mod tests {
    use super::*;

    fn missing(choice: &DeviceChoice) -> Result<&'static str, AudioError> {
        Err(AudioError::NoDevice(format!("{choice:?}")))
    }

    #[test]
    fn a_missing_device_is_reported_once_and_retried() {
        let mut mic = Device::new("microphone", None, DeviceChoice::Default);
        let start = Instant::now();
        let wanted = DeviceChoice::Default;
        assert!(mic.maintain(None, &wanted, start, missing).is_some());
        // Not retried before the interval, and not reported again after.
        assert_eq!(mic.maintain(None, &wanted, start, |_| panic!()), None);
        let later = start + REOPEN_INTERVAL;
        assert_eq!(mic.maintain(None, &wanted, later, missing), None);
        let later = later + REOPEN_INTERVAL;
        assert_eq!(mic.maintain(None, &wanted, later, |_| Ok("mic")), None);
        assert_eq!(mic.io, Some("mic"));
    }

    #[test]
    fn a_failed_device_is_reported_and_reopened() {
        let wanted = DeviceChoice::Default;
        let mut mic = Device::new("microphone", None, wanted.clone());
        let start = Instant::now();
        mic.maintain(None, &wanted, start, |_| Ok("first"));
        assert_eq!(mic.io, Some("first"));

        let problem = mic.maintain(Some("unplugged".into()), &wanted, start, |_| panic!());
        assert_eq!(
            problem.as_deref(),
            Some("the microphone stopped working: unplugged")
        );
        assert_eq!(mic.io, None);
        let later = start + REOPEN_INTERVAL;
        assert_eq!(mic.maintain(None, &wanted, later, |_| Ok("second")), None);
        assert_eq!(mic.io, Some("second"));
        // A later failure is a new outage, so it is reported too.
        assert!(
            mic.maintain(Some("again".into()), &wanted, later, |_| panic!())
                .is_some()
        );
    }

    #[test]
    fn a_new_wanted_device_is_opened_at_once() {
        let mut speakers = Device::new("speakers", None, DeviceChoice::Default);
        let start = Instant::now();
        speakers.maintain(None, &DeviceChoice::Default, start, |_| Ok("default"));
        assert_eq!(speakers.io, Some("default"));
        let game = DeviceChoice::ClosestTo("Headphones".into());
        speakers.maintain(None, &game, start, |choice| {
            assert_eq!(*choice, game);
            Ok("headphones")
        });
        assert_eq!(speakers.io, Some("headphones"));
    }

    /// A microphone that captures `frames` frames and then breaks.
    struct Breaks {
        frames: u32,
    }

    impl FrameSource for Breaks {
        fn read(&mut self, frame: &mut MonoFrame) -> bool {
            if self.frames == 0 {
                return false;
            }
            self.frames -= 1;
            frame.fill(0.1);
            true
        }

        fn failure(&self) -> Option<String> {
            (self.frames == 0).then(|| "unplugged".into())
        }
    }

    #[test]
    fn a_microphone_failing_mid_spurt_ends_it() {
        let (events, mut rx) = crate::event::channel();
        let io = AudioIo {
            source: Some(Box::new(Breaks { frames: 5 })),
            sink: Some(Box::new(yakvc_audio::NullSink::new().0)),
            #[cfg(feature = "sim")]
            impairment: None,
        };
        let mut voice = Voice::start(&Config::default(), io, events);
        voice.set_input(Input {
            push_to_talk: true,
            ..Input::default()
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut talking = Vec::new();
        while Instant::now() < deadline && talking != [true, false] {
            match rx.try_next() {
                Some(Event::Talking { talking: t, .. }) => talking.push(t),
                Some(_) => {}
                None => std::thread::sleep(Duration::from_millis(10)),
            }
        }
        assert_eq!(talking, [true, false]);
        assert!(!voice.audio_stats().transmitting);
        voice.stop();
    }

    #[test]
    fn a_supplied_device_is_never_reopened() {
        let start = Instant::now();
        let later = start + REOPEN_INTERVAL;
        let mut sink = Device::new("speakers", Some("recorder"), DeviceChoice::Default);
        let other = DeviceChoice::Id("other".into());
        assert_eq!(sink.maintain(None, &other, start, |_| panic!()), None);
        assert!(
            sink.maintain(Some("broke".into()), &other, start, |_| panic!())
                .is_some()
        );
        assert_eq!(sink.maintain(None, &other, later, |_| panic!()), None);
        assert_eq!(sink.io, None);
    }
}
