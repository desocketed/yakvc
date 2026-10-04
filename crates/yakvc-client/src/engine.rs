use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use iroh::endpoint::presets;
use iroh::{Endpoint, EndpointAddr, RelayMode, SecretKey};
use tokio::runtime::Runtime;
use yakvc_audio::{Devices, FrameSink, FrameSource, StreamStats};
use yakvc_shared::{IssuerKey, TicketBody, TicketVerifier, Uuid};

use crate::config::{Config, ConfigError};
use crate::event::{self, Event, Events, JoinId, PeerState, RendezvousState};
use crate::net::{Identity, Net, Trust};
use crate::voice::{AudioIo, Voice};
use crate::world::{Input, World};

/// A running voice engine. Dropping it shuts down gracefully, waiting at most
/// 500 ms.
#[derive(Debug)]
pub struct Engine {
    net: Net,
    voice: Voice,
    /// Taken on drop to shut it down.
    runtime: Option<Runtime>,
}

/// Configures an [`Engine`] before starting it.
pub struct EngineBuilder {
    config: Config,
    data_dir: Option<PathBuf>,
    io: AudioIo,
}

/// Per-player playback settings, persisted by the Java side.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PeerAudio {
    pub volume: f32,
    pub muted: bool,
}

/// Snapshot of one peer, for UIs and diagnostics.
#[derive(Debug, Clone, PartialEq)]
pub struct PeerInfo {
    pub uuid: Uuid,
    pub name: String,
    pub state: PeerState,
    pub rtt: Option<Duration>,
    pub stream: Option<StreamStats>,
}

#[derive(Debug, thiserror::Error)]
pub enum StartError {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error("could not read or create the client key: {0}")]
    Key(#[source] std::io::Error),
    #[error("audio: {0}")]
    Audio(#[from] yakvc_audio::AudioError),
    #[error("network: {0}")]
    Network(String),
}

/// The client's Iroh secret key, in the data directory.
const KEY_FILE: &str = "node.key";
/// The last ticket from the rendezvous, in the data directory.
const TICKET_FILE: &str = "ticket.bin";
/// The cap on a graceful shutdown.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_millis(500);

impl Engine {
    pub fn builder(config: Config) -> EngineBuilder {
        EngineBuilder {
            config,
            data_dir: None,
            io: AudioIo {
                source: None,
                sink: None,
                #[cfg(feature = "sim")]
                impairment: None,
            },
        }
    }

    /// Lists audio devices.
    pub fn devices(&self) -> Result<Devices, yakvc_audio::AudioError> {
        yakvc_audio::devices()
    }

    /// Sets the local Minecraft identity. Starts rendezvous authentication.
    pub fn set_identity(&self, uuid: Uuid, name: &str) {
        self.voice.set_own_uuid(uuid);
        self.net.set_identity(Identity {
            uuid,
            name: name.to_owned(),
        });
    }

    /// Replaces the tab list. Call only when it changes.
    pub fn set_tab_list(&self, players: impl IntoIterator<Item = Uuid>) {
        self.net.set_tab_list(players.into_iter().collect());
    }

    /// Pushes this tick's world snapshot.
    pub fn set_world(&self, world: World) {
        self.voice.set_world(world);
    }

    pub fn set_input(&self, input: Input) {
        self.voice.set_input(input);
    }

    /// The game's Voice/Speech volume slider, `0.0..=1.0`.
    pub fn set_game_volume(&self, volume: f32) {
        self.voice.set_game_volume(volume);
    }

    pub fn set_peer_audio(&self, uuid: Uuid, audio: PeerAudio) {
        self.voice.set_peer_audio(uuid, audio);
    }

    /// Reports the result of the `joinServer` call for an
    /// [`Event::JoinRequest`](crate::Event::JoinRequest).
    pub fn complete_join(&self, id: JoinId, ok: bool) {
        self.net.complete_join(id, ok);
    }

    pub fn update_config(&self, config: Config) {
        // The endpoint is built once, so `rendezvous` and `relay_only` only
        // take effect on the next start.
        self.voice.update_config(&config);
        self.net.set_trust(verifier(&config));
    }

    pub fn peers(&self) -> Vec<PeerInfo> {
        self.net
            .peers()
            .into_iter()
            .map(|peer| PeerInfo {
                stream: self.voice.stats(peer.uuid),
                uuid: peer.uuid,
                name: peer.name,
                state: peer.state,
                rtt: peer.rtt,
            })
            .collect()
    }

    /// Switches to direct-call mode for `yakvc-cli call` (M2): no
    /// rendezvous, a self-signed ticket, and every peer heard at full volume
    /// whatever the world says. Never used by the game.
    #[allow(dead_code, reason = "awaiting a public direct-call API")]
    pub(crate) fn enable_direct_calls(&self, uuid: Uuid, name: &str) {
        let body = TicketBody {
            dev: true,
            ..TicketBody::new(
                uuid,
                name.to_owned(),
                self.net.endpoint_id(),
                SystemTime::now(),
                DIRECT_CALL_TICKET_LIFETIME,
            )
        };
        self.set_identity(uuid, name);
        self.net
            .enable_direct_calls(IssuerKey::generate().sign(&body));
        self.voice.set_direct(true);
    }

    /// Our address, for the other side of a direct call.
    #[allow(dead_code, reason = "awaiting a public direct-call API")]
    pub(crate) fn endpoint_addr(&self) -> EndpointAddr {
        self.net.addr()
    }

    /// Dials a direct call. Needs [`Engine::enable_direct_calls`] on both sides.
    #[allow(dead_code, reason = "awaiting a public direct-call API")]
    pub(crate) fn call(&self, addr: EndpointAddr) {
        self.net.call(addr);
    }
}

/// Long enough for any diagnostic call.
const DIRECT_CALL_TICKET_LIFETIME: Duration = Duration::from_secs(24 * 60 * 60);

impl Drop for Engine {
    fn drop(&mut self) {
        self.voice.stop();
        let Some(runtime) = self.runtime.take() else {
            return;
        };
        let net = self.net.clone();
        // On a plain thread, because blocking on or dropping a runtime from
        // inside an async caller would panic.
        std::thread::scope(|scope| {
            scope.spawn(move || {
                runtime.block_on(async {
                    let _ = tokio::time::timeout(SHUTDOWN_TIMEOUT, net.close()).await;
                });
                runtime.shutdown_timeout(Duration::from_millis(100));
            });
        });
    }
}

impl EngineBuilder {
    /// Where the client key and ticket cache live. Required.
    pub fn data_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.data_dir = Some(dir.into());
        self
    }

    /// Replaces the microphone, e.g. with a WAV file.
    pub fn audio_source(mut self, source: impl FrameSource + 'static) -> Self {
        self.io.source = Some(Box::new(source));
        self
    }

    /// Replaces the speakers, e.g. with a recorder.
    pub fn audio_sink(mut self, sink: impl FrameSink + 'static) -> Self {
        self.io.sink = Some(Box::new(sink));
        self
    }

    /// Impairs every incoming voice datagram.
    #[cfg(feature = "sim")]
    pub fn impairment(mut self, impairment: crate::sim::Impairment) -> Self {
        self.io.impairment = Some(impairment);
        self
    }

    pub fn start(self) -> Result<(Engine, Events), StartError> {
        let EngineBuilder {
            config,
            data_dir,
            io,
        } = self;
        let data_dir = data_dir.ok_or_else(|| {
            StartError::Key(io::Error::new(
                io::ErrorKind::InvalidInput,
                "no data directory set",
            ))
        })?;
        let key = load_or_create_key(&data_dir).map_err(StartError::Key)?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("yakvc-net")
            .enable_all()
            .build()
            .map_err(|err| StartError::Network(err.to_string()))?;

        let (events_tx, events) = event::channel();
        let voice = Voice::start(&config, io, events_tx.clone());
        let protocols = vec![voice.protocol()];
        let trust = Trust {
            verifier: verifier(&config),
            direct_calls: false,
        };
        let net = block_on(&runtime, async {
            let endpoint = bind_endpoint(key, &config).await?;
            let net = Net::new(endpoint, protocols, trust, events_tx.clone());
            match config.rendezvous.clone() {
                Some(rendezvous) => net.start_rendezvous(rendezvous, data_dir.join(TICKET_FILE)),
                // There is no built-in default rendezvous yet.
                None => events_tx.send(Event::Rendezvous(RendezvousState::Disconnected)),
            }
            Ok::<_, StartError>(net)
        })?;
        let engine = Engine {
            net,
            voice,
            runtime: Some(runtime),
        };
        Ok((engine, events))
    }
}

impl std::fmt::Debug for EngineBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineBuilder").finish_non_exhaustive()
    }
}

fn verifier(config: &Config) -> TicketVerifier {
    TicketVerifier::new(config.trusted_issuers.iter().copied()).accept_dev(config.dev_mode)
}

/// Binds the Iroh endpoint. It uses only our own relay and no address
/// lookup: peer addresses come from the rendezvous, and publishing them
/// anywhere else would leak IPs.
async fn bind_endpoint(key: SecretKey, config: &Config) -> Result<Endpoint, StartError> {
    let relay = config.rendezvous.as_ref().and_then(|r| r.relay.clone());
    let relay_mode = match relay {
        Some(url) => RelayMode::custom([url]),
        None => RelayMode::Disabled,
    };
    let mut builder = Endpoint::builder(presets::Minimal)
        .secret_key(key)
        .alpns(vec![yakvc_shared::peer::ALPN.to_vec()])
        .relay_mode(relay_mode);
    if config.relay_only {
        builder = builder.clear_ip_transports();
    }
    builder
        .bind()
        .await
        .map_err(|err| StartError::Network(err.to_string()))
}

/// Runs `future` on the engine's runtime and waits for it.
/// `Runtime::block_on` panics on a thread that is already inside a Tokio
/// runtime, which is where async callers like the CLI and testkit call us
/// from, so it runs on a short-lived thread instead.
fn block_on<F>(runtime: &Runtime, future: F) -> F::Output
where
    F: Future + Send,
    F::Output: Send,
{
    std::thread::scope(|scope| scope.spawn(|| runtime.block_on(future)).join())
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
}

/// Reads the client key, creating it on first start.
fn load_or_create_key(dir: &Path) -> io::Result<SecretKey> {
    let path = dir.join(KEY_FILE);
    match std::fs::read(&path) {
        Ok(bytes) => {
            let bytes: [u8; 32] = bytes.try_into().map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "node.key is not 32 bytes")
            })?;
            Ok(SecretKey::from_bytes(&bytes))
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            std::fs::create_dir_all(dir)?;
            let key = SecretKey::generate();
            write_private(&path, &key.to_bytes())?;
            Ok(key)
        }
        Err(err) => Err(err),
    }
}

/// Writes a file only the current user can read.
fn write_private(path: &Path, contents: &[u8]) -> io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)?.write_all(contents)
}
