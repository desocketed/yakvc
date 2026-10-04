//! A rendezvous server and simulated clients in one process, for integration
//! tests.
//!
//! ```ignore
//! let net = TestNet::start().await;
//! let mut alice = net.client().name("alice").tone(440.0).start().await;
//! let mut bob = net.client().name("bob").start().await;
//! see_each_other(&[&alice, &bob]);
//! alice.move_to(Vec3::new(0.0, 64.0, 0.0));
//! bob.move_to(Vec3::new(10.0, 64.0, 0.0));
//! bob.wait_for_peer(alice.uuid(), PeerState::Direct).await?;
//! alice.talk(true);
//! ```
//!
//! [`TestNet`] plays the game server: it keeps every client's position and
//! pushes each client a world snapshot at 20 Hz containing the clients it
//! sees within tracking range.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use yakvc_audio::{
    FRAME_DURATION, FrameSource, MonoFrame, NullSink, Recording, ToneSource, WavSource,
};
use yakvc_client::sim::Impairment;
use yakvc_client::{
    Config, Engine, Event, Events, Input, PeerState, Pose, RendezvousConfig, RendezvousState,
    StreamStats, Uuid, Vec3, World,
};
use yakvc_server::{RelayOptions, Server};
use yakvc_shared::{IssuerKey, SecretKey};

/// Default time [`TestClient`] waits for an expected event.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// Player-entity tracking range the simulated game server uses (Paper's
/// default).
pub const TRACKING_RANGE: f64 = 48.0;

/// How often the simulated game server pushes world snapshots: every tick.
const TICK: Duration = Duration::from_millis(50);

/// A dev-auth rendezvous with a plain-HTTP relay on localhost, plus the
/// simulated game world.
#[derive(Debug)]
pub struct TestNet {
    server: Server,
    /// Every client's config starts from this one.
    config: Config,
    players: Players,
    /// Holds each client's key and ticket cache. Removed on drop.
    dir: PathBuf,
    clients_started: AtomicUsize,
    stop_ticking: Arc<AtomicBool>,
    ticker: Option<JoinHandle<()>>,
}

/// Configures a [`TestClient`].
#[derive(Debug)]
pub struct ClientBuilder<'a> {
    net: &'a TestNet,
    name: Option<String>,
    voice: Voice,
    impairment: Impairment,
    relay_only: bool,
}

/// A simulated player: an [`Engine`] with test audio I/O, driven by the
/// [`TestNet`] world.
#[derive(Debug)]
pub struct TestClient {
    uuid: Uuid,
    engine: Arc<Engine>,
    events: Events,
    recording: Recording,
    players: Players,
}

#[derive(Debug, thiserror::Error)]
#[error("timed out after {0:?} waiting for {1}")]
pub struct Timeout(pub Duration, pub String);

/// The simulated game world, shared by the [`TestNet`] ticker and every
/// [`TestClient`].
type Players = Arc<Mutex<Vec<Player>>>;

#[derive(Debug)]
struct Player {
    uuid: Uuid,
    pos: Vec3,
    /// Weak, so dropping the [`TestClient`] shuts its engine down.
    engine: Weak<Engine>,
}

/// What a client says while talking.
#[derive(Debug)]
enum Voice {
    Silence,
    Tone(f32),
    Wav(PathBuf),
}

impl TestNet {
    pub async fn start() -> TestNet {
        let localhost = "127.0.0.1:0".parse().expect("valid address");
        let server = Server::builder(SecretKey::generate(), IssuerKey::generate())
            .bind(localhost)
            .relay(RelayOptions {
                http_bind: localhost,
                tls: None,
                quic_bind: None,
            })
            .insecure_dev_auth()
            .spawn()
            .await
            .expect("test server starts");

        let addr = server.endpoint_addr();
        let config = Config {
            rendezvous: Some(RendezvousConfig {
                endpoint_id: addr.id,
                addrs: addr.ip_addrs().copied().collect(),
                relay: server.relay_url(),
            }),
            trusted_issuers: vec![server.issuer_id()],
            dev_mode: true,
            ..Config::default()
        };

        let players = Players::default();
        let stop_ticking = Arc::new(AtomicBool::new(false));
        let ticker = std::thread::spawn({
            let players = players.clone();
            let stop = stop_ticking.clone();
            move || {
                while !stop.load(Ordering::Relaxed) {
                    push_worlds(&players);
                    std::thread::sleep(TICK);
                }
            }
        });

        TestNet {
            server,
            config,
            players,
            dir: unique_temp_dir(),
            clients_started: AtomicUsize::new(0),
            stop_ticking,
            ticker: Some(ticker),
        }
    }

    pub fn server(&self) -> &Server {
        &self.server
    }

    pub fn client(&self) -> ClientBuilder<'_> {
        ClientBuilder {
            net: self,
            name: None,
            voice: Voice::Silence,
            impairment: Impairment::default(),
            relay_only: false,
        }
    }
}

impl Drop for TestNet {
    fn drop(&mut self) {
        self.stop_ticking.store(true, Ordering::Relaxed);
        if let Some(ticker) = self.ticker.take() {
            let _ = ticker.join();
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// One game tick: sends each client the players within tracking range of it.
fn push_worlds(players: &Mutex<Vec<Player>>) {
    let mut players = players.lock().expect("players lock");
    players.retain(|p| p.engine.strong_count() > 0);
    for me in players.iter() {
        let Some(engine) = me.engine.upgrade() else {
            continue;
        };
        let visible = players
            .iter()
            .filter(|other| other.uuid != me.uuid && distance(me.pos, other.pos) <= TRACKING_RANGE)
            .map(|other| (other.uuid, other.pos))
            .collect();
        engine.set_world(World {
            // Yaw 0: facing +Z, so +X is on the listener's left.
            listener: Pose {
                pos: me.pos,
                yaw: 0.0,
                pitch: 0.0,
            },
            players: visible,
        });
    }
}

fn distance(a: Vec3, b: Vec3) -> f64 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2) + (a.z - b.z).powi(2)).sqrt()
}

fn unique_temp_dir() -> PathBuf {
    static NETS: AtomicUsize = AtomicUsize::new(0);
    let n = NETS.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("yakvc-testkit-{}-{n}", std::process::id()))
}

/// Puts the name's bytes in the UUID, so UUIDs are stable and readable in
/// debug output. Minecraft names are at most 16 characters.
fn uuid_for(name: &str) -> Uuid {
    assert!(name.len() <= 16, "player name {name:?} is over 16 bytes");
    let mut bytes = [0u8; 16];
    bytes[..name.len()].copy_from_slice(name.as_bytes());
    Uuid::from_bytes(bytes)
}

impl<'a> ClientBuilder<'a> {
    /// Defaults to `player<N>`. The UUID is derived from the name.
    pub fn name(mut self, name: &str) -> Self {
        self.name = Some(name.to_owned());
        self
    }

    /// Speaks a sine tone while talking. The default is silence.
    pub fn tone(mut self, frequency_hz: f32) -> Self {
        self.voice = Voice::Tone(frequency_hz);
        self
    }

    pub fn wav(mut self, path: &Path) -> Self {
        self.voice = Voice::Wav(path.to_owned());
        self
    }

    pub fn impairment(mut self, impairment: Impairment) -> Self {
        self.impairment = impairment;
        self
    }

    pub fn relay_only(mut self) -> Self {
        self.relay_only = true;
        self
    }

    /// Starts the engine and waits until it is registered with the rendezvous.
    pub async fn start(self) -> TestClient {
        let n = self.net.clients_started.fetch_add(1, Ordering::Relaxed);
        let name = self.name.unwrap_or_else(|| format!("player{n}"));
        let uuid = uuid_for(&name);

        let config = Config {
            relay_only: self.relay_only,
            ..self.net.config.clone()
        };
        let (sink, recording) = NullSink::new();
        let builder = Engine::builder(config)
            .data_dir(self.net.dir.join(&name))
            .audio_sink(sink)
            .impairment(self.impairment);
        let builder = match self.voice {
            Voice::Silence => builder.audio_source(Silence::new()),
            Voice::Tone(hz) => builder.audio_source(ToneSource::new(hz)),
            Voice::Wav(path) => builder.audio_source(
                WavSource::open(&path, true)
                    .unwrap_or_else(|e| panic!("open {}: {e}", path.display())),
            ),
        };
        let (engine, events) = builder
            .start()
            .unwrap_or_else(|e| panic!("start {name}: {e}"));
        engine.set_identity(uuid, &name);

        let engine = Arc::new(engine);
        self.net.players.lock().expect("players lock").push(Player {
            uuid,
            pos: Vec3::default(),
            engine: Arc::downgrade(&engine),
        });

        let mut client = TestClient {
            uuid,
            engine,
            events,
            recording,
            players: self.net.players.clone(),
        };
        client
            .wait_for("registration", |e| {
                matches!(e, Event::Rendezvous(RendezvousState::Registered { .. }))
            })
            .await
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        client
    }
}

impl TestClient {
    pub fn uuid(&self) -> Uuid {
        self.uuid
    }

    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// Replaces this client's tab list with `others`.
    pub fn sees(&self, others: &[&TestClient]) {
        self.engine.set_tab_list(others.iter().map(|c| c.uuid));
    }

    pub fn move_to(&self, pos: Vec3) {
        let mut players = self.players.lock().expect("players lock");
        if let Some(me) = players.iter_mut().find(|p| p.uuid == self.uuid) {
            me.pos = pos;
        }
    }

    /// Holds or releases push-to-talk.
    pub fn talk(&self, talking: bool) {
        self.engine.set_input(Input {
            push_to_talk: talking,
            ..Input::default()
        });
    }

    /// Everything this client has played.
    pub fn recording(&self) -> &Recording {
        &self.recording
    }

    /// Receive stats for audio from `from`, if it is a connected peer.
    pub fn stream_stats(&self, from: Uuid) -> Option<StreamStats> {
        self.engine
            .peers()
            .into_iter()
            .find(|p| p.uuid == from)
            .and_then(|p| p.stream)
    }

    /// Waits for an event matching `pred`, discarding others.
    pub async fn wait_for(
        &mut self,
        what: &str,
        mut pred: impl FnMut(&Event) -> bool,
    ) -> Result<Event, Timeout> {
        let found = tokio::time::timeout(DEFAULT_TIMEOUT, async {
            while let Some(event) = self.events.next().await {
                if pred(&event) {
                    return Some(event);
                }
            }
            None
        })
        .await;
        match found {
            Ok(Some(event)) => Ok(event),
            Ok(None) => Err(Timeout(
                DEFAULT_TIMEOUT,
                format!("{what} (engine shut down)"),
            )),
            Err(_) => Err(Timeout(DEFAULT_TIMEOUT, what.to_owned())),
        }
    }

    pub async fn wait_for_peer(&mut self, uuid: Uuid, state: PeerState) -> Result<(), Timeout> {
        // The event may have been discarded by an earlier wait.
        let already = self
            .engine
            .peers()
            .iter()
            .any(|p| p.uuid == uuid && p.state == state);
        if already {
            return Ok(());
        }
        self.wait_for(&format!("peer {uuid} to become {state:?}"), |e| {
            *e == Event::Peer { uuid, state }
        })
        .await
        .map(|_| ())
    }
}

/// Makes every client in `clients` see every other one.
pub fn see_each_other(clients: &[&TestClient]) {
    for me in clients {
        let others: Vec<&TestClient> = clients
            .iter()
            .copied()
            .filter(|c| c.uuid != me.uuid)
            .collect();
        me.sees(&others);
    }
}

/// A silent microphone: zeroed frames, paced in real time.
#[derive(Debug)]
struct Silence {
    start: Instant,
    frames: u32,
}

impl Silence {
    fn new() -> Self {
        Silence {
            start: Instant::now(),
            frames: 0,
        }
    }
}

impl FrameSource for Silence {
    fn read(&mut self, frame: &mut MonoFrame) -> bool {
        if self.start + FRAME_DURATION * (self.frames + 1) > Instant::now() {
            return false;
        }
        self.frames += 1;
        frame.fill(0.0);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuids_come_from_names() {
        assert_eq!(uuid_for("alice"), uuid_for("alice"));
        assert_ne!(uuid_for("alice"), uuid_for("bob"));
        assert_eq!(uuid_for("a").as_bytes()[0], b'a');
    }

    #[test]
    fn silence_is_paced_in_real_time() {
        let mut silence = Silence::new();
        let mut frame = [1.0; yakvc_audio::FRAME_SAMPLES];
        assert!(!silence.read(&mut frame), "no frame is due yet");

        std::thread::sleep(FRAME_DURATION * 3);
        let mut produced = 0;
        while silence.read(&mut frame) {
            produced += 1;
        }
        assert!((3..=5).contains(&produced), "{produced} frames in 60 ms");
        assert!(frame.iter().all(|&s| s == 0.0));
    }

    #[test]
    fn distance_is_euclidean() {
        let d = distance(Vec3::new(0.0, 64.0, 0.0), Vec3::new(3.0, 64.0, 4.0));
        assert_eq!(d, 5.0);
    }
}
