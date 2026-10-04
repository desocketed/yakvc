use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::server::ServerBuilder;

/// The server's `config.toml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub endpoint_key: PathBuf,
    pub issuer_key: PathBuf,
    pub bind: SocketAddr,
    pub relay: Option<RelayConfig>,
    pub ticket_lifetime_hours: Option<u64>,
    pub metrics_bind: Option<SocketAddr>,
    #[serde(default)]
    pub insecure_dev_auth: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelayConfig {
    pub http_bind: SocketAddr,
    pub https_bind: Option<SocketAddr>,
    pub quic_bind: Option<SocketAddr>,
    /// ACME via Let's Encrypt for this domain, or `cert` + `key` files.
    pub domain: Option<String>,
    pub acme_contact: Option<String>,
    pub acme_cache_dir: Option<PathBuf>,
    pub cert: Option<PathBuf>,
    pub key: Option<PathBuf>,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid config: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("invalid config: {0}")]
    Invalid(String),
}

impl Config {
    pub fn load(path: &Path) -> Result<Config, ConfigError> {
        let _ = path;
        todo!()
    }

    /// Reads the key files and returns a builder with every setting applied.
    pub fn into_builder(self) -> Result<ServerBuilder, ConfigError> {
        todo!()
    }
}
