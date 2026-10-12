//! Voice protocol: send/receive loop, recipient selection and spatial input.
//!
//! The network side runs one task per peer ([`serve_peer`]), which feeds
//! received frames to the audio thread and tells the peer whether we want
//! its audio. The audio thread ([`AudioLoop`]) captures, encodes and sends
//! frames to the recipients [`rules`] picks, and mixes what it receives.

mod group;
mod rules;
mod sink;

use std::collections::{HashMap, HashSet};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc as std_mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use bytes::Bytes;
use tokio::sync::watch;
use yakvc_audio::{
    AudioError, DeviceChoice, Encoder, FRAME_DURATION, FRAME_SAMPLES, FrameSink, FrameSource, InputConfig,
    InputProcessor, JitterConfig, Microphone, Mixer, MonoFrame, Muffle, Packet, Pulled,
    ReceiveStream, Speakers, StereoFrame, StreamStats, level_db,
};
use yakvc_proto::peer::CloseCode;
use yakvc_proto::voice::{VERSION, VoiceHeader, VoiceMsg};
use yakvc_proto::{GroupAnnounce, GroupId, ProtocolId, Uuid};

use crate::config::{AudioConfig, Config};
use crate::engine::PeerAudio;
use crate::event::{Event, EventSender};
use crate::net::{ControlRecv, Peer, PeerLink, Protocol};
use crate::world::{Input, Vec3, World};

use self::group::Told;
pub use self::group::{GroupError, GroupInfo};
pub(crate) use self::rules::VoiceState;
use self::rules::{Heard, plan_send};
pub use self::sink::{VoiceFrame, VoiceSink};

/// How often the audio thread wakes to move frames.
const AUDIO_TICK: Duration = Duration::from_millis(5);
/// How long [`Voice::stop`] waits for the audio thread. It notices the stop
/// flag within a tick unless it is stuck opening a device, and shutdown
/// must not hang on that.
const STOP_TIMEOUT: Duration = Duration::from_millis(200);
/// At most this many frames are played per tick (200 ms of audio, plenty to
/// catch up after a stall), so an output whose `wants_frame` never says no,
/// a buggy voice sink say, can't pin the audio thread.
const MAX_PLAYS_PER_TICK: usize = 10;
/// Mic level is reported every this many frames, about 10 times a second.
const LEVEL_EVERY_FRAMES: u32 = 5;
/// A remote speaker counts as stopped after this many silent frames.
const TALKING_HANGOVER_FRAMES: u32 = 10;
/// A played frame only counts as talking above this level. With DTX on,
/// Opus keeps sending a comfort-noise packet every 400 ms or so while
/// push-to-talk is held over silence, and each would otherwise flash the
/// talking indicator (#113). Those packets can't be told apart by size:
/// measured, they are 8-9 bytes at 24 kbps but 14-18 at 16 kbps, 21 at
/// 32 kbps and 41-45 at 64 kbps, where quiet speech at 16 kbps can be 21.
/// Decoded, they (and the concealment after them) stay below -85 dBFS at
/// every bitrate, while speech is well above the default voice activation
/// threshold of -45 dBFS. They are still played, as Opus intends.
const TALKING_LEVEL_DB: f32 = -70.0;

/// The audio thread's own state, for the in-game debug overlay.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AudioStats {
    pub microphone_open: bool,
    pub speakers_open: bool,
    /// Voice goes to a [`VoiceSink`] instead of the speakers.
    pub external_output: bool,
    /// The Opus bitrate in use, which drops below the configured one when
    /// the relay budget is tight.
    pub bitrate: u32,
    pub transmitting: bool,
    /// Microphone overruns since it was last opened.
    pub overruns: u64,
    /// Speaker underruns since they were last opened.
    pub underruns: u64,
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
    /// Replaced on change rather than changed in place: the audio thread
    /// reads a snapshot per frame and never waits on a long edit.
    state: Mutex<Arc<VoiceState>>,
    /// Each player's open voice links, newest first. Two endpoints of equal
    /// standing can claim one player, and voice is per player: only the
    /// newest link is heard, sent to and believed about its group, so their
    /// frames never mix (#109). The older ones stay in reserve in case it
    /// closes (#114). [`current`] lists the links in use.
    peers: Mutex<HashMap<Uuid, Vec<VoicePeer>>>,
    to_audio: Mutex<std_mpsc::SyncSender<Frame>>,
    /// Bumped whenever what we want to hear may have changed, so each peer
    /// task re-sends `ReceiveState` if needed.
    receive_policy: watch::Sender<()>,
    stats: Mutex<HashMap<Uuid, StreamStats>>,
    audio_stats: Mutex<AudioStats>,
    /// A voice sink for the audio thread to switch to on its next tick;
    /// `Some(None)` goes back to the speakers.
    voice_sink: Mutex<Option<Option<Box<dyn VoiceSink>>>>,
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

/// The link in use for each player: the newest.
fn current(peers: &HashMap<Uuid, Vec<VoicePeer>>) -> impl Iterator<Item = &VoicePeer> {
    peers.values().filter_map(|links| links.first())
}

/// A received frame, on its way from a peer task to the audio thread.
struct Frame {
    from: Uuid,
    /// The link it came over, which changes when another endpoint for the
    /// player takes over.
    link_id: usize,
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
        let shared = Arc::new(Shared::new(
            config,
            to_audio,
            events,
            #[cfg(feature = "sim")]
            io.impairment.clone(),
        ));
        let stop = Arc::new(AtomicBool::new(false));
        let audio_thread = std::thread::Builder::new()
            .name("yakvc-audio".into())
            .spawn({
                let shared = shared.clone();
                let stop = stop.clone();
                // Built on the audio thread, so a failing audio stack can't
                // stop the engine from starting.
                move || {
                    crate::event::report_panics_on_this_thread(&shared.events);
                    AudioLoop::new(shared, from_net, io, stop).run();
                }
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
        self.shared.change(|state| state.own_uuid = Some(uuid));
    }

    pub(crate) fn set_world(&self, world: World) {
        self.shared
            .change(|state| state.set_world(world, Instant::now()));
    }

    /// The audio thread notices the change and reopens the speakers if
    /// they follow the game.
    pub(crate) fn set_game_device(&self, name: Option<&str>) {
        self.shared
            .change(|state| state.game_device = name.map(str::to_owned));
    }

    pub(crate) fn set_input(&self, input: Input) {
        self.shared.change(|state| state.input = input);
        self.shared.receive_policy.send_replace(());
    }

    pub(crate) fn set_game_volume(&self, volume: f32) {
        self.shared
            .change(|state| state.game_volume = volume.clamp(0.0, 1.0));
    }

    /// Sends playable voice to `sink` instead of the speakers, or back to the
    /// speakers with `None`, from the audio thread's next tick.
    pub(crate) fn set_voice_sink(&self, sink: Option<Box<dyn VoiceSink>>) {
        *self.shared.voice_sink.lock().unwrap() = Some(sink);
    }

    pub(crate) fn set_peer_audio(&self, uuid: Uuid, audio: PeerAudio) {
        self.shared.change(|state| {
            state.peer_audio.insert(uuid, audio);
        });
        self.shared.receive_policy.send_replace(());
    }

    /// Invites `target` to our group, or while we are in none, to a new
    /// group we join once it accepts. Its peer task delivers the invite.
    pub(crate) fn invite(&self, target: Uuid) {
        self.shared
            .update(|state, _| state.invite(target, Instant::now()));
    }

    /// Joins the group `from` invited us to, leaving any other.
    pub(crate) fn accept_invite(&self, from: Uuid) -> Result<(), GroupError> {
        self.shared
            .update(|state, _| state.accept_invite(from, Instant::now()))
    }

    /// Joins the public group `id` from the list, leaving any other.
    pub(crate) fn join_group(&self, id: GroupId) -> Result<(), GroupError> {
        let announced = self.shared.announced();
        self.shared
            .update(|state, _| state.join_public(id, &announced))
    }

    pub(crate) fn leave_group(&self) {
        self.shared.update(|state, _| state.set_group(None));
    }

    pub(crate) fn set_group_public(&self, public: bool) -> Result<(), GroupError> {
        self.shared
            .update(|state, _| state.change_group_state(|s| s.public = public))
    }

    /// Empty for none; at most 32 characters.
    pub(crate) fn set_group_label(&self, label: &str) -> Result<(), GroupError> {
        self.shared
            .update(|state, _| state.change_group_state(|s| s.label = label.to_owned()))
    }

    /// Our group and the public groups our peers announce.
    pub(crate) fn groups(&self) -> Vec<GroupInfo> {
        let announced = self.shared.announced();
        self.shared.state.lock().unwrap().groups(&announced)
    }

    pub(crate) fn group_mates(&self) -> HashSet<Uuid> {
        self.shared.state.lock().unwrap().group_mates.clone()
    }

    pub(crate) fn set_ignore_world(&self, ignore: bool) {
        self.shared.change(|state| state.ignore_world = ignore);
    }

    /// The audio thread reopens devices whose configured name changed.
    pub(crate) fn update_config(&self, config: &Config) {
        self.shared.change(|state| {
            state.audio = config.audio.clone();
            state.acoustics = config.acoustics.clone();
            state.voice_range = config.voice_range;
            state.friends_only = config.friends_only.clone();
            state.invites_from = config.invites;
        });
        self.shared.receive_policy.send_replace(());
    }

    pub(crate) fn stats(&self, uuid: Uuid) -> Option<StreamStats> {
        self.shared.stats.lock().unwrap().get(&uuid).cloned()
    }

    pub(crate) fn audio_stats(&self) -> AudioStats {
        self.shared.audio_stats.lock().unwrap().clone()
    }

    /// Stops the audio thread and waits up to [`STOP_TIMEOUT`] for it. A
    /// thread that takes longer (a device open that never returns) is left
    /// to finish on its own.
    pub(crate) fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let Some(thread) = self.audio_thread.take() else {
            return;
        };
        let deadline = Instant::now() + STOP_TIMEOUT;
        while !thread.is_finished() {
            if Instant::now() >= deadline {
                tracing::warn!("audio thread did not stop in time; leaving it behind");
                return;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        let _ = thread.join();
    }
}

impl Drop for Voice {
    fn drop(&mut self) {
        self.stop();
    }
}

impl Shared {
    fn new(
        config: &Config,
        to_audio: std_mpsc::SyncSender<Frame>,
        events: EventSender,
        #[cfg(feature = "sim")] impairment: Option<crate::sim::Impairment>,
    ) -> Shared {
        let mut state = VoiceState::new(
            config.audio.clone(),
            config.voice_range,
            config.friends_only.clone(),
        );
        state.invites_from = config.invites;
        state.acoustics = config.acoustics.clone();
        Shared {
            state: Mutex::new(Arc::new(state)),
            peers: Mutex::new(HashMap::new()),
            to_audio: Mutex::new(to_audio),
            receive_policy: watch::Sender::new(()),
            stats: Mutex::new(HashMap::new()),
            audio_stats: Mutex::new(AudioStats::default()),
            voice_sink: Mutex::new(None),
            events,
            #[cfg(feature = "sim")]
            impairment,
        }
    }

    fn send_to_audio(&self, frame: Frame) {
        // Fails when the queue is full (see `AUDIO_QUEUE`) or the audio
        // thread has stopped; a lost frame is concealed like any other.
        let _ = self.to_audio.lock().unwrap().try_send(frame);
    }

    /// Whether `link_id` is the link voice uses for `uuid`.
    fn is_current(&self, uuid: Uuid, link_id: usize) -> bool {
        self.peers
            .lock()
            .unwrap()
            .get(&uuid)
            .and_then(|links| links.first())
            .is_some_and(|p| p.peer.link_id() == link_id)
    }

    /// Stores what the peer announced over `link_id`. Kept for an older
    /// link too, for when it takes over, but only the current link's counts.
    /// Returns whether it differs from what the link announced before, so
    /// a repeated announcement costs no group refresh.
    fn set_announcement(&self, uuid: Uuid, link_id: usize, group: Option<GroupAnnounce>) -> bool {
        let mut peers = self.peers.lock().unwrap();
        let mut links = peers.get_mut(&uuid).into_iter().flatten();
        match links.find(|p| p.peer.link_id() == link_id) {
            Some(peer) if peer.group != group => {
                peer.group = group;
                true
            }
            _ => false,
        }
    }

    /// What each player's current link last announced.
    fn announced(&self) -> Vec<(Uuid, GroupAnnounce)> {
        current(&self.peers.lock().unwrap())
            .filter_map(|p| Some((p.peer.uuid(), p.group.clone()?)))
            .collect()
    }

    /// Works out again which peers are in our group, after an announcement,
    /// a peer leaving, or a change of our own group.
    fn refresh_group_mates(&self) {
        // Peers before state, never the other way round, so the two locks
        // can't deadlock.
        let announced = self.announced();
        let mut events = Vec::new();
        let changed = self.change(|state| {
            let had_group = state.group.is_some();
            state.join_pending(&announced);
            let joined = !had_group && state.group.is_some();
            let mates = state.mates_among(&announced);
            state.update_mates(mates, &mut events) || joined
        });
        for event in events {
            self.events.send(event);
        }
        if changed {
            self.receive_policy.send_replace(());
        }
    }

    /// The current state, to read without holding the lock.
    fn snapshot(&self) -> Arc<VoiceState> {
        self.state.lock().unwrap().clone()
    }

    /// Changes the state. A snapshot the audio thread still holds is left
    /// as it was and the change goes into a copy, so nothing here waits on
    /// the audio thread, and a frame sees one consistent state.
    fn change<T>(&self, change: impl FnOnce(&mut VoiceState) -> T) -> T {
        let mut state = self.state.lock().unwrap();
        change(Arc::make_mut(&mut state))
    }

    /// Changes the voice state, reports the events `change` adds, then
    /// works out the group mates again and has every peer task tell its
    /// peer what changed.
    fn update<T>(&self, change: impl FnOnce(&mut VoiceState, &mut Vec<Event>) -> T) -> T {
        let mut events = Vec::new();
        let result = self.change(|state| change(state, &mut events));
        for event in events {
            self.events.send(event);
        }
        self.refresh_group_mates();
        self.receive_policy.send_replace(());
        result
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
    let newest = VoicePeer {
        peer: peer.clone(),
        wants_audio: wants_audio.clone(),
        group: None,
    };
    shared
        .peers
        .lock()
        .unwrap()
        .entry(uuid)
        .or_default()
        .insert(0, newest);
    let reader = tokio::spawn(read_control(
        shared.clone(),
        peer,
        control_recv,
        wants_audio,
    ));
    // The net layer aborts this task when the connection closes, which can
    // come before the datagram channel ends, so the link is taken out of
    // the table on drop rather than after the loop: a link left behind
    // would stay the current one and keep the next link unheard.
    let _unlink = Unlink {
        shared: shared.clone(),
        uuid,
        link_id,
        reader: reader.abort_handle(),
    };
    let mut receive_policy = shared.receive_policy.subscribe();
    #[cfg(feature = "sim")]
    let mut impairer = shared.impairment.clone().map(crate::sim::Impairer::new);

    let mut told = Told::default();
    'serve: loop {
        let news = shared
            .state
            .lock()
            .unwrap()
            .news_for(uuid, &mut told, Instant::now());
        for msg in &news {
            if control_send.send(msg).await.is_err() {
                break 'serve;
            }
        }
        tokio::select! {
            datagram = datagrams.recv() => {
                let Some(datagram) = datagram else { break };
                if !shared.is_current(uuid, link_id) {
                    continue;
                }
                #[cfg(feature = "sim")]
                if let Some(impairer) = &mut impairer {
                    deliver_impaired(&shared, uuid, link_id, datagram, impairer);
                    continue;
                }
                deliver(&shared, uuid, link_id, datagram);
            }
            changed = receive_policy.changed() => {
                if changed.is_err() {
                    break;
                }
            }
        }
    }
}

/// Takes a link out of the voice peer table when its [`serve_peer`] ends,
/// however it ends.
struct Unlink {
    shared: Arc<Shared>,
    uuid: Uuid,
    link_id: usize,
    reader: tokio::task::AbortHandle,
}

impl Drop for Unlink {
    fn drop(&mut self) {
        self.reader.abort();
        let was_current = {
            let mut peers = self.shared.peers.lock().unwrap();
            let links = peers.entry(self.uuid).or_default();
            let was_current = links
                .first()
                .is_some_and(|p| p.peer.link_id() == self.link_id);
            links.retain(|p| p.peer.link_id() != self.link_id);
            if links.is_empty() {
                peers.remove(&self.uuid);
            }
            was_current
        };
        // The next newest link, if any, now speaks for the player. The audio
        // thread notices the change, or the player being gone, by itself.
        if was_current {
            self.shared.refresh_group_mates();
        }
    }
}

async fn read_control(
    shared: Arc<Shared>,
    peer: Peer,
    mut control: ControlRecv,
    wants_audio: Arc<AtomicBool>,
) {
    let uuid = peer.uuid();
    let link_id = peer.link_id();
    while let Some(msg) = control.recv::<VoiceMsg>().await {
        // An oversize label is as malformed as a message that won't decode,
        // and a peer sending one is off the protocol.
        if !group::labels_fit(&msg) {
            peer.close(CloseCode::ProtocolError);
            break;
        }
        match msg {
            VoiceMsg::ReceiveState { wants_audio: wants } => {
                wants_audio.store(wants, Ordering::Relaxed);
            }
            VoiceMsg::Group(group) => {
                if shared.set_announcement(uuid, link_id, group) {
                    shared.refresh_group_mates();
                }
            }
            // The rest speak for the player only over its current link.
            _ if !shared.is_current(uuid, link_id) => {}
            VoiceMsg::Invite { key, state } => shared.update(|voice, events| {
                voice.receive_invite(uuid, key, state, Instant::now(), events);
            }),
            VoiceMsg::GroupState(state) => shared.update(|voice, events| {
                voice.receive_group_state(uuid, state, events);
            }),
        }
    }
}

/// Passes a received datagram to the audio thread.
fn deliver(shared: &Shared, from: Uuid, link_id: usize, datagram: Bytes) {
    let Some((header, _)) = VoiceHeader::parse(&datagram) else {
        return;
    };
    shared.send_to_audio(Frame {
        from,
        link_id,
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
    link_id: usize,
    datagram: Bytes,
    impairer: &mut crate::sim::Impairer,
) {
    for delay in impairer.delays() {
        if delay.is_zero() {
            deliver(shared, from, link_id, datagram.clone());
        } else {
            let shared = shared.clone();
            let datagram = datagram.clone();
            tokio::spawn(async move {
                tokio::time::sleep(delay).await;
                // Checked again: a newer link may have taken over meanwhile.
                if shared.is_current(from, link_id) {
                    deliver(&shared, from, link_id, datagram);
                }
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
    /// Plays voice instead of `sink` while set.
    voice_sink: Option<Box<dyn VoiceSink>>,
    /// When `voice_sink` may next get a frame: it is held to real time,
    /// whatever its `wants_frame` says.
    voice_sink_due: Instant,
    input: InputProcessor,
    input_config: InputConfig,
    encoder: Option<Encoder>,
    mixer: Mixer,
    remotes: HashMap<Uuid, Remote>,
    /// Counts [`Self::play`] calls, for [`VoiceFrame::tick`].
    plays: u64,
    /// The previous frame was sent.
    transmitting: bool,
    seq: u32,
    /// Capture time of the next frame, in samples.
    ts: u32,
    bitrate: u32,
    frames_since_level: u32,
}

/// What [`AudioLoop::send`] needs to know about one connected peer, read
/// under the peers lock and used after it.
struct Sendable {
    peer: Peer,
    wants_audio: bool,
    relayed: bool,
}

/// One remote speaker's receive state.
struct Remote {
    /// The link `stream` is fed from.
    link_id: usize,
    stream: ReceiveStream,
    muffle: Muffle,
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
        let state = shared.snapshot();
        let audio = &state.audio;
        let input_config = input_config(audio);
        AudioLoop {
            source: Device::new("microphone", io.source, state.input_device()),
            sink: Device::new("speakers", io.sink, state.output_device()),
            voice_sink: None,
            voice_sink_due: Instant::now(),
            input: InputProcessor::new(input_config.clone()),
            input_config,
            encoder: None,
            mixer: Mixer::new(),
            remotes: HashMap::new(),
            plays: 0,
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
            self.switch_output();
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
            for _ in 0..MAX_PLAYS_PER_TICK {
                if !self.output_wants_frame() {
                    break;
                }
                self.play();
            }
            *self.shared.audio_stats.lock().unwrap() = self.audio_stats();
            std::thread::sleep(AUDIO_TICK);
        }
    }

    fn audio_stats(&self) -> AudioStats {
        AudioStats {
            microphone_open: self.source.io.is_some(),
            speakers_open: self.voice_sink.is_none() && self.sink.io.is_some(),
            external_output: self.voice_sink.is_some(),
            bitrate: self.bitrate,
            transmitting: self.transmitting,
            overruns: self.source.io.as_ref().map_or(0, |s| s.glitches()),
            underruns: self.sink.io.as_ref().map_or(0, |s| s.glitches()),
        }
    }

    /// Takes up a voice sink set since the last tick, or goes back to the
    /// speakers.
    fn switch_output(&mut self) {
        let Some(sink) = self.shared.voice_sink.lock().unwrap().take() else {
            return;
        };
        if sink.is_some() && self.sink.opened_as.is_some() {
            // Our own speakers close while a sink plays, and reopen once it
            // goes. Speakers the caller supplied stay, unused.
            self.sink.io = None;
            self.sink.retry_at = Instant::now();
        }
        // The old output hears its streams end, and the new one starts
        // fresh, from each voice's next frames rather than mid-stream.
        for (uuid, remote) in std::mem::take(&mut self.remotes) {
            if let Some(old) = &mut self.voice_sink {
                old.stream_end(uuid);
            }
            if remote.talking {
                self.talking(uuid, false);
            }
        }
        self.voice_sink = sink;
        self.voice_sink_due = Instant::now();
    }

    /// Whether the output in use wants another frame now. A voice sink's
    /// own pacing is trusted only up to real time: one that always says yes
    /// would otherwise drain the jitter buffers early. It may catch up on
    /// at most a tick's worth of frames it fell behind by.
    fn output_wants_frame(&mut self) -> bool {
        match &mut self.voice_sink {
            Some(sink) => {
                let now = Instant::now();
                if self.voice_sink_due > now || !sink.wants_frame() {
                    return false;
                }
                let catch_up = FRAME_DURATION * MAX_PLAYS_PER_TICK as u32;
                let earliest = now.checked_sub(catch_up).unwrap_or(now);
                self.voice_sink_due = self.voice_sink_due.max(earliest) + FRAME_DURATION;
                true
            }
            None => self.sink.io.as_ref().is_some_and(|sink| sink.wants_frame()),
        }
    }

    /// Opens, reopens and retries the microphone and speakers. A missing or
    /// failed device is reported but not fatal: the player can still hear
    /// without a microphone, and vice versa. The speakers are left alone
    /// while a voice sink plays instead.
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
        if self.voice_sink.is_some() {
            return;
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
        let state = self.shared.snapshot();
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
        if transmitting
            && !state.input.deafened
            && let Some(sink) = &mut self.voice_sink
            && sink.wants_own_voice()
        {
            sink.own_voice(state.own_uuid.unwrap_or_default(), frame);
        }
        self.set_transmitting(transmitting, &state);
        self.ts = self.ts.wrapping_add(FRAME_SAMPLES as u32);
    }

    /// Ends the talk spurt when the microphone fails during it, since no
    /// frame will come to end it.
    fn end_spurt_without_microphone(&mut self) {
        let state = self.shared.snapshot();
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
            for p in self.shared.peers.lock().unwrap().values().flatten() {
                p.peer.set_relay_full(false);
            }
        }
    }

    fn send(&mut self, frame: &MonoFrame, state: &VoiceState, end_of_talk: bool) {
        let Some(encoder) = &mut self.encoder else {
            return;
        };
        // Every received datagram takes the peers lock on a tokio worker, so
        // only this snapshot is taken under it; planning, encoding and
        // sending run on the handles.
        let peers: Vec<Sendable> = {
            let peers = self.shared.peers.lock().unwrap();
            current(&peers)
                .map(|p| Sendable {
                    peer: p.peer.clone(),
                    wants_audio: p.wants_audio.load(Ordering::Relaxed),
                    relayed: p.peer.is_relayed(),
                })
                .collect()
        };
        let candidates: Vec<_> = peers
            .iter()
            .filter(|p| p.wants_audio)
            .filter_map(|p| state.send_to(p.peer.uuid(), p.relayed))
            .collect();
        let groups: HashMap<Uuid, bool> = candidates.iter().map(|c| (c.uuid, c.group)).collect();
        let plan = plan_send(candidates, state.audio.bitrate);
        for p in peers.iter().filter(|p| p.relayed) {
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
        let mut datagram = Vec::with_capacity(VoiceHeader::LEN + payload.len());
        for uuid in &plan.recipients {
            let header = VoiceHeader {
                seq: self.seq,
                ts: self.ts,
                end_of_talk,
                group: groups[uuid],
            };
            datagram.clear();
            datagram.extend_from_slice(&header.to_bytes());
            datagram.extend_from_slice(payload);
            // A peer that just closed simply misses the frame.
            if let Some(p) = peers.iter().find(|p| p.peer.uuid() == *uuid) {
                let _ = p.peer.send_datagram(&datagram);
            }
        }
        self.seq = self.seq.wrapping_add(1);
        if end_of_talk {
            encoder.reset();
        }
    }

    fn receive(&mut self, frame: Frame) {
        // With nothing to play to, buffering would only grow.
        if self.sink.io.is_none() && self.voice_sink.is_none() {
            return;
        }
        let remote = self.remotes.entry(frame.from).or_insert_with(|| Remote {
            link_id: frame.link_id,
            stream: ReceiveStream::new(JitterConfig::default()),
            muffle: Muffle::new(),
            group: false,
            talking: false,
            silent_frames: TALKING_HANGOVER_FRAMES,
        });
        // Another endpoint for the player took over: its `seq` and `ts`
        // count from its own start, so the old stream's timing is no use.
        if remote.link_id != frame.link_id {
            remote.link_id = frame.link_id;
            remote.stream = ReceiveStream::new(JitterConfig::default());
        }
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
            if let Some(sink) = &mut self.voice_sink {
                sink.stream_end(uuid);
            }
            self.shared.stats.lock().unwrap().remove(&uuid);
        }
    }

    fn play(&mut self) {
        let now = Instant::now();
        let state = self.shared.snapshot();
        let world = state.world_at(now);
        self.plays += 1;
        let mut voices: Vec<(MonoFrame, yakvc_audio::Spatial)> = Vec::new();
        let mut talking_changes = Vec::new();
        for (&uuid, remote) in &mut self.remotes {
            let mut frame: MonoFrame = [0.0; FRAME_SAMPLES];
            let pulled = remote.stream.pull(&mut frame, now);
            let heard = match pulled {
                Pulled::Voice => state.heard(&world, uuid, remote.group),
                Pulled::Silence => None,
            };
            let talking_frame = heard.is_some() && level_db(&frame) > TALKING_LEVEL_DB;
            if let Some(playback) = heard {
                match &mut self.voice_sink {
                    // The sink does its own acoustics, so it gets the frame
                    // as decoded.
                    Some(sink) => {
                        let (positioned, heard_from, occlusion) = match playback.heard {
                            Heard::Positioned(player) => {
                                (true, player.heard_from, player.occlusion)
                            }
                            Heard::Centred => (false, Vec3::default(), 0.0),
                        };
                        sink.frame(&VoiceFrame {
                            uuid,
                            pcm: &frame,
                            tick: self.plays,
                            volume: playback.volume,
                            range: if positioned { state.voice_range } else { 0.0 },
                            group: remote.group,
                            positioned,
                            heard_from,
                            occlusion,
                        });
                    }
                    None => {
                        if let Some(spatial) = state.through_speakers(&world, playback) {
                            remote.muffle.apply(
                                &mut frame,
                                spatial.muffle,
                                state.acoustics.muffled_cutoff_hz,
                            );
                            voices.push((frame, spatial));
                        }
                    }
                }
            }
            if talking_frame {
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

        // A voice sink plays the voices itself: the mixer, with the room's
        // reverb, the game volume and the limiter, serves only the speakers.
        if self.voice_sink.is_none() {
            self.mixer.set_master_gain(state.game_volume);
            self.mixer.set_acoustics(yakvc_audio::Acoustics {
                decay: world.room.decay,
                echo: world.room.echo,
                echo_delay: world.room.echo_delay,
                echo_feedback: state.acoustics.echo_feedback,
                reverb_level: state.acoustics.reverb_level,
                reverb_damping: state.acoustics.reverb_damping,
            });
            let mut out: StereoFrame = [[0.0; 2]; FRAME_SAMPLES];
            self.mixer.mix(
                voices.iter().map(|(frame, spatial)| (frame, *spatial)),
                &mut out,
            );
            if let Some(sink) = &mut self.sink.io {
                sink.write(&out);
            }
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
    use iroh::endpoint::{ConnectionError, VarInt};
    use yakvc_proto::{GroupKey, PublicGroup};

    use super::*;
    use crate::net::test_util::{Served, eventually, probe, serve_protocols, uuid};

    /// The voice protocol as `uuid(1)`, without an audio thread, linked to
    /// a test probe standing in for `uuid(2)`'s voice protocol.
    async fn linked_to_probe() -> (Arc<Shared>, PeerLink, Served) {
        let (events, _rx) = crate::event::channel();
        let (to_audio, _from_net) = std_mpsc::sync_channel(AUDIO_QUEUE);
        let shared = Arc::new(Shared::new(
            &Config::default(),
            to_audio,
            events,
            #[cfg(feature = "sim")]
            None,
        ));
        shared.change(|state| state.own_uuid = Some(uuid(1)));
        let voice: Arc<dyn Protocol> = Arc::new(VoiceProtocol(shared.clone()));
        let (probe, mut links) = probe(ProtocolId::VOICE.0, VERSION);
        let served = serve_protocols(voice, probe).await;
        let link = links.recv().await.unwrap();
        (shared, link, served)
    }

    fn announcement(label: &str) -> GroupAnnounce {
        let key = GroupKey::random();
        GroupAnnounce {
            proof: key.proof(uuid(2)),
            public: Some(PublicGroup {
                key,
                label: label.into(),
            }),
        }
    }

    /// The net layer aborts a protocol's task when the connection closes,
    /// so the link must leave the table on abort as well as on a clean end.
    #[tokio::test]
    async fn an_aborted_link_task_leaves_the_peer_table() {
        let (_alice, link, _served) = linked_to_probe().await;
        let (events, _rx) = crate::event::channel();
        let (to_audio, _from_net) = std_mpsc::sync_channel(AUDIO_QUEUE);
        let bob = Arc::new(Shared::new(
            &Config::default(),
            to_audio,
            events,
            #[cfg(feature = "sim")]
            None,
        ));
        let serving = tokio::spawn(serve_peer(bob.clone(), link));
        eventually("bob lists alice's link", || {
            bob.peers.lock().unwrap().contains_key(&uuid(1))
        })
        .await;
        serving.abort();
        assert!(serving.await.unwrap_err().is_cancelled());
        assert!(bob.peers.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_oversize_label_closes_the_link_as_a_protocol_error() {
        let (shared, mut bob, served) = linked_to_probe().await;
        let oversize = announcement(&"x".repeat(33));
        bob.control_send
            .send(&VoiceMsg::Group(Some(oversize)))
            .await
            .unwrap();
        let reason = served.conn_b.closed().await;
        assert!(
            matches!(&reason, ConnectionError::ApplicationClosed(close)
                if close.error_code == VarInt::from_u32(CloseCode::ProtocolError as u32)),
            "{reason:?}"
        );
        assert!(shared.announced().is_empty());
    }

    #[tokio::test]
    async fn a_repeated_announcement_changes_nothing() {
        let (shared, mut bob, _served) = linked_to_probe().await;
        let group = announcement("Miners");
        bob.control_send
            .send(&VoiceMsg::Group(Some(group.clone())))
            .await
            .unwrap();
        eventually("alice stores the announcement", || {
            shared.announced() == [(uuid(2), group.clone())]
        })
        .await;
        // Each end numbers its own connection, so ask alice's side.
        let link_id = shared.peers.lock().unwrap()[&uuid(2)][0].peer.link_id();
        assert!(!shared.set_announcement(uuid(2), link_id, Some(group.clone())));
        assert!(shared.set_announcement(uuid(2), link_id, None));
        assert!(shared.announced().is_empty());
        // An unknown link stores nothing.
        assert!(!shared.set_announcement(uuid(2), link_id + 1, Some(group)));
    }

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

    /// Counts how often the audio thread asks it, and wants a frame every
    /// other time: once per tick.
    #[derive(Clone, Default)]
    struct CountingSink(Arc<Mutex<u32>>);

    impl VoiceSink for CountingSink {
        fn wants_frame(&mut self) -> bool {
            let mut asked = self.0.lock().unwrap();
            *asked += 1;
            *asked % 2 == 1
        }

        fn frame(&mut self, _frame: &VoiceFrame<'_>) {}

        fn stream_end(&mut self, _uuid: Uuid) {}
    }

    /// Waits up to 5 s for the audio thread to report `external` output.
    fn wait_for_output(voice: &Voice, external: bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while voice.audio_stats().external_output != external {
            assert!(Instant::now() < deadline, "the output never switched");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Counts the audible frames of the player's own voice it gets, asking
    /// for them or not.
    #[derive(Clone, Default)]
    struct OwnVoiceSink {
        wants: bool,
        frames: Arc<Mutex<u32>>,
    }

    impl VoiceSink for OwnVoiceSink {
        fn wants_frame(&mut self) -> bool {
            false
        }

        fn frame(&mut self, _frame: &VoiceFrame<'_>) {}

        fn stream_end(&mut self, _uuid: Uuid) {}

        fn wants_own_voice(&self) -> bool {
            self.wants
        }

        fn own_voice(&mut self, _uuid: Uuid, pcm: &MonoFrame) {
            if pcm.iter().any(|s| s.abs() > 0.001) {
                *self.frames.lock().unwrap() += 1;
            }
        }
    }

    /// A voice on a 440 Hz tone, with `sink` in place of the speakers.
    fn talking_into(sink: OwnVoiceSink) -> Voice {
        let (events, _rx) = crate::event::channel();
        let io = AudioIo {
            source: Some(Box::new(yakvc_audio::ToneSource::new(440.0))),
            sink: Some(Box::new(yakvc_audio::NullSink::new().0)),
            #[cfg(feature = "sim")]
            impairment: None,
        };
        let voice = Voice::start(&Config::default(), io, events);
        voice.set_voice_sink(Some(Box::new(sink)));
        wait_for_output(&voice, true);
        voice
    }

    /// The count once it has stopped changing for 200 ms.
    fn settled(frames: &Mutex<u32>) -> u32 {
        loop {
            let before = *frames.lock().unwrap();
            std::thread::sleep(Duration::from_millis(200));
            if *frames.lock().unwrap() == before {
                return before;
            }
        }
    }

    #[test]
    fn a_sink_that_asks_hears_the_own_voice_only_while_it_goes_out() {
        let sink = OwnVoiceSink {
            wants: true,
            ..OwnVoiceSink::default()
        };
        let mut voice = talking_into(sink.clone());
        // Push to talk, not pressed: nothing.
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(*sink.frames.lock().unwrap(), 0, "own voice while quiet");

        voice.set_input(Input {
            push_to_talk: true,
            ..Input::default()
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        while *sink.frames.lock().unwrap() < 5 {
            assert!(Instant::now() < deadline, "no own voice while transmitting");
            std::thread::sleep(Duration::from_millis(10));
        }

        // Deafened, or the key let go: it stops.
        for input in [
            Input {
                push_to_talk: true,
                deafened: true,
                ..Input::default()
            },
            Input::default(),
        ] {
            voice.set_input(input);
            let count = settled(&sink.frames);
            std::thread::sleep(Duration::from_millis(200));
            assert_eq!(
                *sink.frames.lock().unwrap(),
                count,
                "own voice with {input:?}"
            );
        }
        voice.stop();
    }

    #[test]
    fn a_sink_hears_no_own_voice_unless_it_asks() {
        let sink = OwnVoiceSink::default();
        let mut voice = talking_into(sink.clone());
        voice.set_input(Input {
            push_to_talk: true,
            ..Input::default()
        });
        std::thread::sleep(Duration::from_millis(300));
        assert!(voice.audio_stats().transmitting);
        assert_eq!(*sink.frames.lock().unwrap(), 0);
        voice.stop();
    }

    /// Always wants another frame, as a buggy sink might, and counts how
    /// often it is asked.
    #[derive(Clone, Default)]
    struct Greedy(Arc<Mutex<u32>>);

    impl VoiceSink for Greedy {
        fn wants_frame(&mut self) -> bool {
            *self.0.lock().unwrap() += 1;
            true
        }

        fn frame(&mut self, _frame: &VoiceFrame<'_>) {}

        fn stream_end(&mut self, _uuid: Uuid) {}
    }

    #[test]
    fn a_greedy_sink_cant_pin_the_audio_thread() {
        let (events, _rx) = crate::event::channel();
        let io = AudioIo {
            source: Some(Box::new(yakvc_audio::SilenceSource::new())),
            sink: Some(Box::new(yakvc_audio::NullSink::new().0)),
            #[cfg(feature = "sim")]
            impairment: None,
        };
        let mut voice = Voice::start(&Config::default(), io, events);
        voice.set_voice_sink(Some(Box::new(Greedy::default())));
        // The stats are written after the play loop, so they only change if
        // the loop ends.
        wait_for_output(&voice, true);
        let start = Instant::now();
        voice.stop();
        assert!(start.elapsed() < STOP_TIMEOUT, "the audio thread was stuck");
    }

    #[test]
    fn a_greedy_sink_plays_in_real_time() {
        let (events, _rx) = crate::event::channel();
        let io = AudioIo {
            source: Some(Box::new(yakvc_audio::SilenceSource::new())),
            sink: None,
            #[cfg(feature = "sim")]
            impairment: None,
        };
        let mut voice = Voice::start(&Config::default(), io, events);
        let sink = Greedy::default();
        voice.set_voice_sink(Some(Box::new(sink.clone())));
        wait_for_output(&voice, true);
        let start = *sink.0.lock().unwrap();
        std::thread::sleep(Duration::from_millis(500));
        let played = *sink.0.lock().unwrap() - start;
        voice.stop();
        // 25 frames in real time; unpaced it would be 1000. Leeway for a
        // slow test machine.
        assert!((15..=45).contains(&played), "played {played} frames in 500 ms");
    }

    /// Records the streams it is told ended.
    #[derive(Clone, Default)]
    struct EndsSink(Arc<Mutex<Vec<Uuid>>>);

    impl VoiceSink for EndsSink {
        fn wants_frame(&mut self) -> bool {
            false
        }

        fn frame(&mut self, _frame: &VoiceFrame<'_>) {}

        fn stream_end(&mut self, uuid: Uuid) {
            self.0.lock().unwrap().push(uuid);
        }
    }

    /// An audio loop with no devices, not running, and a remote speaker
    /// `uuid(2)` mid-stream.
    fn loop_with_a_remote() -> AudioLoop {
        let (events, _rx) = crate::event::channel();
        let (to_audio, from_net) = std_mpsc::sync_channel(AUDIO_QUEUE);
        let shared = Arc::new(Shared::new(
            &Config::default(),
            to_audio,
            events,
            #[cfg(feature = "sim")]
            None,
        ));
        let io = AudioIo {
            source: None,
            sink: None,
            #[cfg(feature = "sim")]
            impairment: None,
        };
        let mut audio = AudioLoop::new(shared, from_net, io, Arc::new(AtomicBool::new(false)));
        audio.remotes.insert(
            uuid(2),
            Remote {
                link_id: 0,
                stream: ReceiveStream::new(JitterConfig::default()),
                muffle: Muffle::new(),
                group: false,
                talking: true,
                silent_frames: 0,
            },
        );
        audio
    }

    #[test]
    fn a_replaced_sink_hears_its_streams_end_and_the_next_starts_fresh() {
        for next in [Some(Box::new(EndsSink::default()) as Box<dyn VoiceSink>), None] {
            let mut audio = loop_with_a_remote();
            let old = EndsSink::default();
            audio.voice_sink = Some(Box::new(old.clone()));
            *audio.shared.voice_sink.lock().unwrap() = Some(next);
            audio.switch_output();
            assert_eq!(*old.0.lock().unwrap(), [uuid(2)]);
            assert!(audio.remotes.is_empty(), "the stream carried over");
        }
    }

    #[test]
    fn a_voice_sink_replaces_the_speakers_until_cleared() {
        let (events, _rx) = crate::event::channel();
        let (speakers, recording) = yakvc_audio::NullSink::new();
        let io = AudioIo {
            source: Some(Box::new(yakvc_audio::SilenceSource::new())),
            sink: Some(Box::new(speakers)),
            #[cfg(feature = "sim")]
            impairment: None,
        };
        let mut voice = Voice::start(&Config::default(), io, events);
        let sink = CountingSink::default();
        voice.set_voice_sink(Some(Box::new(sink.clone())));
        wait_for_output(&voice, true);
        assert!(!voice.audio_stats().speakers_open);
        let played = recording.frames();
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(recording.frames(), played, "the speakers played");
        assert!(*sink.0.lock().unwrap() > 0, "the sink paces playback");

        voice.set_voice_sink(None);
        wait_for_output(&voice, false);
        std::thread::sleep(Duration::from_millis(200));
        assert!(recording.frames() > played, "the speakers didn't come back");
        assert!(voice.audio_stats().speakers_open);
        voice.stop();
    }
}
