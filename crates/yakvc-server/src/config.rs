use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use yakvc_shared::{IssuerKey, SecretKey};

use crate::server::{RelayOptions, RelayTls, Server, ServerBuilder};

/// The server's `config.toml`.
///
/// Key files hold the 32 raw secret-key bytes, as written by
/// `yakvc-server run` on first start or by `yakvc-server keygen`.
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
    /// Serve clients without a rendezvous session too (see `RelayOptions::open`).
    #[serde(default)]
    pub open: bool,
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
        let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Io {
            path: path.to_owned(),
            source,
        })?;
        Ok(toml::from_str(&text)?)
    }

    /// Reads the key files and returns a builder with every setting applied.
    pub fn into_builder(self) -> Result<ServerBuilder, ConfigError> {
        let endpoint_key = SecretKey::from_bytes(&read_key(&self.endpoint_key)?);
        let issuer_key = IssuerKey::from_bytes(&read_key(&self.issuer_key)?);
        let mut builder = Server::builder(endpoint_key, issuer_key).bind(self.bind);
        if let Some(relay) = self.relay {
            builder = builder.relay(relay.into_options()?);
        }
        if let Some(hours) = self.ticket_lifetime_hours {
            if hours == 0 {
                return Err(invalid("ticket_lifetime_hours must be at least 1"));
            }
            builder = builder.ticket_lifetime(Duration::from_secs(hours * 3600));
        }
        if let Some(addr) = self.metrics_bind {
            builder = builder.metrics(addr);
        }
        if self.insecure_dev_auth {
            builder = builder.insecure_dev_auth();
        }
        Ok(builder)
    }
}

impl RelayConfig {
    fn into_options(self) -> Result<RelayOptions, ConfigError> {
        let tls = match (self.cert, self.key, self.domain) {
            (Some(cert), Some(key), domain) => Some(RelayTls::Files {
                https_bind: self
                    .https_bind
                    .ok_or_else(|| invalid("relay.cert needs relay.https_bind"))?,
                domain: domain.ok_or_else(|| invalid("relay.cert needs relay.domain"))?,
                cert,
                key,
            }),
            (None, None, Some(domain)) => Some(RelayTls::LetsEncrypt {
                https_bind: self
                    .https_bind
                    .ok_or_else(|| invalid("relay.domain needs relay.https_bind"))?,
                domain,
                contact: self
                    .acme_contact
                    .ok_or_else(|| invalid("relay.domain needs relay.acme_contact"))?,
                cache_dir: self
                    .acme_cache_dir
                    .ok_or_else(|| invalid("relay.domain needs relay.acme_cache_dir"))?,
            }),
            (None, None, None) if self.https_bind.is_none() => None,
            (None, None, None) => {
                return Err(invalid("relay.https_bind needs relay.domain or relay.cert"));
            }
            _ => return Err(invalid("relay.cert and relay.key go together")),
        };
        Ok(RelayOptions {
            http_bind: self.http_bind,
            tls,
            quic_bind: self.quic_bind,
            open: self.open,
        })
    }
}

fn read_key(path: &Path) -> Result<[u8; 32], ConfigError> {
    let bytes = std::fs::read(path).map_err(|source| ConfigError::Io {
        path: path.to_owned(),
        source,
    })?;
    bytes.try_into().map_err(|bytes: Vec<u8>| {
        invalid(&format!(
            "{}: a key file holds 32 bytes, found {}",
            path.display(),
            bytes.len()
        ))
    })
}

fn invalid(msg: &str) -> ConfigError {
    ConfigError::Invalid(msg.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &str = r#"
        endpoint_key = "/etc/yakvc/endpoint.key"
        issuer_key = "/etc/yakvc/issuer.key"
        bind = "0.0.0.0:7843"
        ticket_lifetime_hours = 12
        metrics_bind = "127.0.0.1:9100"

        [relay]
        http_bind = "0.0.0.0:80"
        https_bind = "0.0.0.0:443"
        quic_bind = "0.0.0.0:7842"
        domain = "relay.example.com"
        acme_contact = "ops@example.com"
        acme_cache_dir = "/var/lib/yakvc/acme"
    "#;

    fn relay(toml: &str) -> Result<Option<RelayOptions>, ConfigError> {
        let config: Config = toml::from_str(toml).unwrap();
        config.relay.map(RelayConfig::into_options).transpose()
    }

    #[test]
    fn parses_a_full_config() {
        let config: Config = toml::from_str(FULL).unwrap();
        assert_eq!(config.bind, "0.0.0.0:7843".parse().unwrap());
        assert_eq!(config.ticket_lifetime_hours, Some(12));
        assert!(!config.insecure_dev_auth);
        let options = relay(FULL).unwrap().unwrap();
        assert!(matches!(
            options.tls,
            Some(RelayTls::LetsEncrypt { ref domain, .. }) if domain == "relay.example.com"
        ));
        assert_eq!(options.quic_bind, Some("0.0.0.0:7842".parse().unwrap()));
    }

    #[test]
    fn deploy_example_is_valid() {
        let config: Config = toml::from_str(include_str!("../../../deploy/config.toml")).unwrap();
        assert!(!config.insecure_dev_auth);
        let options = config.relay.unwrap().into_options().unwrap();
        assert!(!options.open);
        assert!(matches!(options.tls, Some(RelayTls::LetsEncrypt { .. })));
    }

    #[test]
    fn rejects_unknown_keys() {
        let toml = "endpoint_key = \"a\"\nissuer_key = \"b\"\nbind = \"0.0.0.0:1\"\nfoo = 1";
        assert!(toml::from_str::<Config>(toml).is_err());
    }

    #[test]
    fn relay_tls_modes() {
        let base = "endpoint_key = \"a\"\nissuer_key = \"b\"\nbind = \"0.0.0.0:1\"\n[relay]\nhttp_bind = \"0.0.0.0:80\"\n";
        let plain = relay(base).unwrap().unwrap();
        assert!(plain.tls.is_none());
        assert!(!plain.open);
        let open = relay(&format!("{base}open = true\n")).unwrap().unwrap();
        assert!(open.open);

        let files = format!(
            "{base}https_bind = \"0.0.0.0:443\"\ndomain = \"relay.example.com\"\ncert = \"c.pem\"\nkey = \"k.pem\"\n"
        );
        assert!(matches!(
            relay(&files).unwrap().unwrap().tls,
            Some(RelayTls::Files { ref domain, .. }) if domain == "relay.example.com"
        ));

        for bad in [
            "cert = \"c.pem\"\nkey = \"k.pem\"\n",
            "https_bind = \"0.0.0.0:443\"\ncert = \"c.pem\"\n",
            "https_bind = \"0.0.0.0:443\"\ncert = \"c.pem\"\nkey = \"k.pem\"\n",
            "https_bind = \"0.0.0.0:443\"\n",
            "https_bind = \"0.0.0.0:443\"\ndomain = \"x\"\n",
        ] {
            assert!(
                matches!(relay(&format!("{base}{bad}")), Err(ConfigError::Invalid(_))),
                "{bad}"
            );
        }
    }

    #[test]
    fn into_builder_reads_keys() {
        let dir = std::env::temp_dir().join(format!("yakvc-config-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let endpoint_key = dir.join("endpoint.key");
        let issuer_key = dir.join("issuer.key");
        std::fs::write(&endpoint_key, [1; 32]).unwrap();
        std::fs::write(&issuer_key, [2; 32]).unwrap();

        let config = Config {
            endpoint_key: endpoint_key.clone(),
            issuer_key: issuer_key.clone(),
            bind: "127.0.0.1:0".parse().unwrap(),
            relay: None,
            ticket_lifetime_hours: Some(1),
            metrics_bind: None,
            insecure_dev_auth: true,
        };
        assert!(config.clone().into_builder().is_ok());

        std::fs::write(&issuer_key, [2; 31]).unwrap();
        assert!(matches!(
            config.clone().into_builder(),
            Err(ConfigError::Invalid(_))
        ));

        std::fs::remove_file(&issuer_key).unwrap();
        assert!(matches!(
            config.clone().into_builder(),
            Err(ConfigError::Io { .. })
        ));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn load_reports_the_path() {
        let err = Config::load(Path::new("/nonexistent/yakvc.toml")).unwrap_err();
        assert!(err.to_string().starts_with("/nonexistent/yakvc.toml"));
    }
}
