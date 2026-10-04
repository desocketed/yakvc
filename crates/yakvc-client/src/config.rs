use std::net::SocketAddr;

use serde::{Deserialize, Serialize};
use yakvc_shared::{EndpointId, IssuerId, RelayUrl, Uuid};

/// Engine configuration, parsed from the engine's keys in
/// `client.toml`. Unknown keys are ignored, because the Java side keeps its
/// own settings in the same file. Every field has a default.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// `None` uses the built-in default rendezvous.
    pub rendezvous: Option<RendezvousConfig>,
    /// Issuers whose tickets are accepted from peers. Empty means the
    /// built-in default issuer.
    pub trusted_issuers: Vec<IssuerId>,
    /// Accept dev tickets. Only for local testing against `--insecure-dev-auth`.
    pub dev_mode: bool,
    /// Never use direct IP paths; all traffic goes through the relay.
    pub relay_only: bool,
    /// Voice range in blocks.
    pub voice_range: f32,
    /// When set, only these players are heard and sent to.
    pub friends_only: Option<Vec<Uuid>>,
    pub audio: AudioConfig,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RendezvousConfig {
    pub endpoint_id: EndpointId,
    /// Direct addresses of the rendezvous.
    pub addrs: Vec<SocketAddr>,
    /// Relay run alongside the rendezvous.
    pub relay: Option<RelayUrl>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioConfig {
    /// `None` follows `game_device`, then the system default.
    pub input_device: Option<String>,
    pub output_device: Option<String>,
    pub activation: Activation,
    /// Opus bitrate in bits per second.
    pub bitrate: u32,
    pub noise_suppression: bool,
    pub input_gain: f32,
    pub vad_threshold_db: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Activation {
    PushToTalk,
    Voice,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("invalid config: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("invalid config: {0}")]
    Invalid(String),
}

impl Config {
    pub fn from_toml(toml: &str) -> Result<Config, ConfigError> {
        let _ = toml;
        todo!()
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            rendezvous: None,
            trusted_issuers: Vec::new(),
            dev_mode: false,
            relay_only: false,
            voice_range: 48.0,
            friends_only: None,
            audio: AudioConfig::default(),
        }
    }
}

impl Default for AudioConfig {
    fn default() -> Self {
        AudioConfig {
            input_device: None,
            output_device: None,
            activation: Activation::PushToTalk,
            bitrate: 24_000,
            noise_suppression: true,
            input_gain: 1.0,
            vad_threshold_db: -45.0,
        }
    }
}
