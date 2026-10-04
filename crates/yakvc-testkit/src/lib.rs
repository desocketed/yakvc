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
use std::time::Duration;

use yakvc_audio::{FrameSource, NullSink, Recording, SilenceSource, ToneSource, WavSource};
use yakvc_client::sim::Impairment;
use yakvc_client::{
    Config, Engine, Event, Events, Input, PeerState, Pose, RendezvousState, StreamStats, Uuid,
    Vec3, World,
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
    config: Config,
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
    /// The last input sent to the engine, so [`TestClient::talk`] and
    /// [`TestClient::set_spectator`] each change only their own flag.
    input: Mutex<Input>,
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
    /// Left out of everyone else's snapshots, as the mod leaves out players
    /// whose tab-list game mode is spectator.
    spectator: bool,
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

        let config = Config {
            rendezvous: Some(server.endpoint_addr().into()),
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
            config: self.config.clone(),
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

/// One game tick: sends each client the non-spectators within tracking range
/// of it.
fn push_worlds(players: &Mutex<Vec<Player>>) {
    let mut players = players.lock().expect("players lock");
    players.retain(|p| p.engine.strong_count() > 0);
    for me in players.iter() {
        let Some(engine) = me.engine.upgrade() else {
            continue;
        };
        let visible = players
            .iter()
            .filter(|other| {
                other.uuid != me.uuid
                    && !other.spectator
                    && distance(me.pos, other.pos) <= TRACKING_RANGE
            })
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
        self.config.relay_only = true;
        self
    }

    /// Changes this client's config, which starts as the [`TestNet`]'s
    /// (rendezvous, trusted issuer and dev mode set).
    pub fn config(mut self, change: impl FnOnce(&mut Config)) -> Self {
        change(&mut self.config);
        self
    }

    /// Starts the engine and waits until it is registered with the rendezvous.
    pub async fn start(self) -> TestClient {
        let n = self.net.clients_started.fetch_add(1, Ordering::Relaxed);
        let name = self.name.unwrap_or_else(|| format!("player{n}"));
        let uuid = uuid_for(&name);

        let source: Box<dyn FrameSource> = match self.voice {
            Voice::Silence => Box::new(SilenceSource::new()),
            Voice::Tone(hz) => Box::new(ToneSource::new(hz)),
            Voice::Wav(path) => Box::new(
                WavSource::open(&path, true)
                    .unwrap_or_else(|e| panic!("open {}: {e}", path.display())),
            ),
        };
        let (sink, recording) = NullSink::new();
        let (engine, events) = Engine::builder(self.config)
            .data_dir(self.net.dir.join(&name))
            .audio_source(source)
            .audio_sink(sink)
            .impairment(self.impairment)
            .start()
            .unwrap_or_else(|e| panic!("start {name}: {e}"));
        engine.set_identity(uuid, &name);

        let engine = Arc::new(engine);
        self.net.players.lock().expect("players lock").push(Player {
            uuid,
            pos: Vec3::default(),
            spectator: false,
            engine: Arc::downgrade(&engine),
        });

        let mut client = TestClient {
            uuid,
            engine,
            events,
            recording,
            players: self.net.players.clone(),
            input: Mutex::new(Input::default()),
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
        self.update_player(|me| me.pos = pos);
    }

    /// Holds or releases push-to-talk.
    pub fn talk(&self, talking: bool) {
        self.update_input(|input| input.push_to_talk = talking);
    }

    /// Switches to or from spectator mode, as the game would: this client
    /// leaves everyone else's snapshots, and its engine is told it is a
    /// spectator.
    pub fn set_spectator(&self, spectator: bool) {
        self.update_player(|me| me.spectator = spectator);
        self.update_input(|input| input.spectator = spectator);
    }

    /// Sends `input` to the engine as is. Unlike [`TestClient::set_spectator`],
    /// `input.spectator` doesn't change what the others see, so a test can
    /// play a client that ignores the spectator rule.
    pub fn set_input(&self, input: Input) {
        self.update_input(|current| *current = input);
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

    fn update_player(&self, change: impl FnOnce(&mut Player)) {
        let mut players = self.players.lock().expect("players lock");
        if let Some(me) = players.iter_mut().find(|p| p.uuid == self.uuid) {
            change(me);
        }
    }

    fn update_input(&self, change: impl FnOnce(&mut Input)) {
        let mut input = self.input.lock().expect("input lock");
        change(&mut input);
        self.engine.set_input(*input);
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
    fn distance_is_euclidean() {
        let d = distance(Vec3::new(0.0, 64.0, 0.0), Vec3::new(3.0, 64.0, 4.0));
        assert_eq!(d, 5.0);
    }
}
