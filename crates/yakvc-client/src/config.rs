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
        let config: Config = toml::from_str(toml)?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), ConfigError> {
        let invalid = |msg: &str| Err(ConfigError::Invalid(msg.to_owned()));
        if !(1.0..=MAX_VOICE_RANGE).contains(&self.voice_range) {
            return invalid("voice_range must be between 1 and 256 blocks");
        }
        if !(MIN_BITRATE..=MAX_BITRATE).contains(&self.audio.bitrate) {
            return invalid("audio.bitrate must be between 16000 and 64000");
        }
        if !(0.0..=MAX_INPUT_GAIN).contains(&self.audio.input_gain) {
            return invalid("audio.input_gain must be between 0 and 10");
        }
        if !(-100.0..=0.0).contains(&self.audio.vad_threshold_db) {
            return invalid("audio.vad_threshold_db must be between -100 and 0");
        }
        Ok(())
    }
}

/// Opus bitrates the design allows.
pub(crate) const MIN_BITRATE: u32 = 16_000;
pub(crate) const MAX_BITRATE: u32 = 64_000;
const MAX_VOICE_RANGE: f32 = 256.0;
const MAX_INPUT_GAIN: f32 = 10.0;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_toml_gives_defaults() {
        assert_eq!(Config::from_toml("").unwrap(), Config::default());
    }

    #[test]
    fn engine_keys_parse_and_unknown_keys_are_ignored() {
        let config = Config::from_toml(
            r#"
            voice_range = 32.0
            relay_only = true
            dev_mode = true
            friends_only = ["00000000-0000-0000-0000-000000000001"]
            # Java's own settings live in the same file.
            show_talking_indicator = true

            [audio]
            activation = "voice"
            bitrate = 32000

            [ui]
            hud_corner = "top_left"
            "#,
        )
        .unwrap();
        assert_eq!(config.voice_range, 32.0);
        assert!(config.relay_only && config.dev_mode);
        assert_eq!(config.friends_only, Some(vec![Uuid::from_u128(1)]));
        assert_eq!(config.audio.activation, Activation::Voice);
        assert_eq!(config.audio.bitrate, 32_000);
        assert_eq!(config.audio.input_gain, 1.0, "unset keys keep defaults");
    }

    #[test]
    fn rendezvous_section_parses() {
        let id = iroh::SecretKey::generate().public();
        let config = Config::from_toml(&format!(
            r#"
            [rendezvous]
            endpoint_id = "{id}"
            addrs = ["127.0.0.1:4433"]
            relay = "https://relay.example.com/"
            "#
        ))
        .unwrap();
        let rendezvous = config.rendezvous.unwrap();
        assert_eq!(rendezvous.endpoint_id, id);
        assert_eq!(rendezvous.addrs, vec!["127.0.0.1:4433".parse().unwrap()]);
        assert!(rendezvous.relay.is_some());
    }

    #[test]
    fn out_of_range_values_are_rejected() {
        for toml in [
            "voice_range = 0.0",
            "voice_range = 1000.0",
            "[audio]\nbitrate = 8000",
            "[audio]\ninput_gain = -1.0",
            "[audio]\nvad_threshold_db = 3.0",
        ] {
            assert!(
                matches!(Config::from_toml(toml), Err(ConfigError::Invalid(_))),
                "{toml}"
            );
        }
        assert!(matches!(
            Config::from_toml("voice_range = \"far\""),
            Err(ConfigError::Parse(_))
        ));
    }
}
