use std::path::PathBuf;
use std::time::Duration;

use yakvc_audio::{Devices, FrameSink, FrameSource, StreamStats};
use yakvc_shared::Uuid;

use crate::config::{Config, ConfigError};
use crate::event::{Events, JoinId, PeerState};
use crate::world::{Input, World};

/// A running voice engine. Dropping it shuts down gracefully, waiting at most
/// 500 ms.
#[derive(Debug)]
pub struct Engine {
    _p: (),
}

/// Configures an [`Engine`] before starting it.
pub struct EngineBuilder {
    _config: Config,
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

impl Engine {
    pub fn builder(config: Config) -> EngineBuilder {
        EngineBuilder { _config: config }
    }

    /// Lists audio devices.
    pub fn devices(&self) -> Result<Devices, yakvc_audio::AudioError> {
        todo!()
    }

    /// Sets the local Minecraft identity. Starts rendezvous authentication.
    pub fn set_identity(&self, uuid: Uuid, name: &str) {
        let _ = (uuid, name);
        todo!()
    }

    /// Replaces the tab list. Call only when it changes.
    pub fn set_tab_list(&self, players: impl IntoIterator<Item = Uuid>) {
        let _ = players.into_iter();
        todo!()
    }

    /// Pushes this tick's world snapshot.
    pub fn set_world(&self, world: World) {
        let _ = world;
        todo!()
    }

    pub fn set_input(&self, input: Input) {
        let _ = input;
        todo!()
    }

    /// The game's Voice/Speech volume slider, `0.0..=1.0`.
    pub fn set_game_volume(&self, volume: f32) {
        let _ = volume;
        todo!()
    }

    pub fn set_peer_audio(&self, uuid: Uuid, audio: PeerAudio) {
        let _ = (uuid, audio);
        todo!()
    }

    /// Reports the result of the `joinServer` call for an
    /// [`Event::JoinRequest`](crate::Event::JoinRequest).
    pub fn complete_join(&self, id: JoinId, ok: bool) {
        let _ = (id, ok);
        todo!()
    }

    pub fn update_config(&self, config: Config) {
        let _ = config;
        todo!()
    }

    pub fn peers(&self) -> Vec<PeerInfo> {
        todo!()
    }
}

impl EngineBuilder {
    /// Where the client key and ticket cache live. Required.
    pub fn data_dir(self, dir: impl Into<PathBuf>) -> Self {
        let _ = dir.into();
        todo!()
    }

    /// Replaces the microphone, e.g. with a WAV file.
    pub fn audio_source(self, source: impl FrameSource + 'static) -> Self {
        let _ = source;
        todo!()
    }

    /// Replaces the speakers, e.g. with a recorder.
    pub fn audio_sink(self, sink: impl FrameSink + 'static) -> Self {
        let _ = sink;
        todo!()
    }

    /// Impairs every incoming voice datagram.
    #[cfg(feature = "sim")]
    pub fn impairment(self, impairment: crate::sim::Impairment) -> Self {
        let _ = impairment;
        todo!()
    }

    pub fn start(self) -> Result<(Engine, Events), StartError> {
        todo!()
    }
}

impl std::fmt::Debug for EngineBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineBuilder").finish_non_exhaustive()
    }
}
