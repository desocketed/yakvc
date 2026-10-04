//! The embedded `iroh-relay`, open only to clients with a rendezvous session.
//!
//! A client's relay connection can arrive before its rendezvous session, and
//! a `relay_only` client can only reach the rendezvous through the relay, so
//! an EndpointId without a session is admitted for a grace period. If it has
//! not registered by then, or when its session ends, it is disconnected.
//! `RelayOptions::open` turns all of this off for self-hosted test relays.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::num::NonZeroU32;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use iroh_relay::server::{
    Access, AccessControl, AcmeConfig, CertConfig, ClientRateLimit, ClientRequest, QuicConfig,
    RelayConfig, RelayService, ServerConfig, TlsConfig,
};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use yakvc_shared::{EndpointId, RelayUrl};

use crate::limits::RateLimiter;
use crate::server::{RelayOptions, RelayTls, SpawnError};
use crate::sessions::Sessions;

/// Grace admissions allowed per EndpointId per [`GRACE_PERIOD`]. A client
/// that keeps reconnecting to the relay without ever registering is refused
/// after this many.
const GRACE_ADMISSIONS: u32 = 3;
const GRACE_PERIOD: Duration = Duration::from_secs(600);

/// Decides who may use the relay.
#[derive(Debug)]
pub(crate) struct RelayGate {
    sessions: Arc<Sessions>,
    /// The rendezvous' own endpoint, which uses the relay as its home relay.
    rendezvous: EndpointId,
    grace: Duration,
    grace_limiter: Mutex<RateLimiter<EndpointId>>,
    /// Set once the relay is running; used to disconnect clients.
    service: OnceLock<RelayService>,
}

impl RelayGate {
    pub fn new(sessions: Arc<Sessions>, rendezvous: EndpointId, grace: Duration) -> Self {
        RelayGate {
            sessions,
            rendezvous,
            grace,
            grace_limiter: Mutex::new(RateLimiter::new(GRACE_ADMISSIONS, GRACE_PERIOD)),
            service: OnceLock::new(),
        }
    }

    /// Disconnects `id` from the relay unless it has a session again.
    pub fn session_ended(&self, id: EndpointId) {
        if !self.sessions.contains(id) {
            self.disconnect(id);
        }
    }

    pub fn prune(&self, now: Instant) {
        self.grace_limiter.lock().unwrap().prune(now);
    }

    fn admit(&self, id: EndpointId, now: Instant) -> Admission {
        if id == self.rendezvous || self.sessions.contains(id) {
            Admission::Session
        } else if self.grace_limiter.lock().unwrap().allow(id, now) {
            Admission::Grace
        } else {
            Admission::Refused
        }
    }

    fn disconnect(&self, id: EndpointId) {
        if let Some(service) = self.service.get() {
            service.clients().disconnect(id, None);
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Admission {
    Session,
    Grace,
    Refused,
}

/// What the relay calls. Holds an `Arc` so a grace admission can start a
/// timer that outlives the call.
#[derive(Debug)]
struct GateAccess(Arc<RelayGate>);

impl AccessControl for GateAccess {
    async fn on_connect(&self, request: &ClientRequest) -> Access {
        let id = request.endpoint_id();
        match self.0.admit(id, Instant::now()) {
            Admission::Session => Access::Allow,
            Admission::Grace => {
                let gate = Arc::clone(&self.0);
                tokio::spawn(async move {
                    tokio::time::sleep(gate.grace).await;
                    gate.session_ended(id);
                });
                Access::Allow
            }
            Admission::Refused => Access::Deny {
                reason: Some("no rendezvous session".into()),
            },
        }
    }
}

/// Starts the relay and returns it with the URL clients should use.
pub(crate) async fn spawn(
    options: &RelayOptions,
    gate: Arc<RelayGate>,
    client_bytes_per_sec: u32,
) -> Result<(iroh_relay::server::Server, RelayUrl), SpawnError> {
    let mut relay = RelayConfig::new(options.http_bind);
    relay.tls = match &options.tls {
        Some(tls) => Some(tls_config(tls)?),
        None => None,
    };
    relay.limits.client_rx = NonZeroU32::new(client_bytes_per_sec).map(ClientRateLimit::new);
    if !options.open {
        relay.access = Arc::new(GateAccess(Arc::clone(&gate)));
    }

    let mut config = ServerConfig::default();
    config.relay = Some(relay);
    config.quic = options.quic_bind.map(QuicConfig::new);

    let server = iroh_relay::server::Server::spawn(config)
        .await
        .map_err(|e| SpawnError::Relay(e.to_string()))?;
    let service = server.relay_service().expect("relay is configured");
    let _ = gate.service.set(service.clone());

    let url = match &options.tls {
        None => {
            let addr = server.http_addr().expect("relay serves HTTP");
            format!("http://{}", dialable(addr))
        }
        Some(RelayTls::LetsEncrypt { domain, .. } | RelayTls::Files { domain, .. }) => {
            let port = server.https_addr().expect("relay serves HTTPS").port();
            format!("https://{domain}:{port}")
        }
    };
    let url = url
        .parse()
        .map_err(|e| SpawnError::Relay(format!("relay URL {url}: {e}")))?;
    Ok((server, url))
}

fn tls_config(tls: &RelayTls) -> Result<TlsConfig, SpawnError> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("ring supports the default TLS versions")
        .with_no_client_auth();
    match tls {
        RelayTls::LetsEncrypt {
            https_bind,
            domain,
            contact,
            cache_dir,
        } => {
            let contact = if contact.starts_with("mailto:") {
                contact.clone()
            } else {
                format!("mailto:{contact}")
            };
            let acme = AcmeConfig::letsencrypt(true)
                .domains(vec![domain.clone()])
                .contact(vec![contact])
                .cache_path(cache_dir.clone());
            let cert = CertConfig::LetsEncrypt {
                acme_config: acme,
                server_config_builder: builder,
            };
            Ok(TlsConfig::new(*https_bind, cert))
        }
        RelayTls::Files {
            https_bind,
            domain: _,
            cert,
            key,
        } => {
            let read_error = |path: &std::path::Path, e: rustls::pki_types::pem::Error| {
                SpawnError::Relay(format!("{}: {e}", path.display()))
            };
            let certs = CertificateDer::pem_file_iter(cert)
                .and_then(|certs| certs.collect::<Result<Vec<_>, _>>())
                .map_err(|e| read_error(cert, e))?;
            let key = PrivateKeyDer::from_pem_file(key).map_err(|e| read_error(key, e))?;
            let server_config = builder
                .with_single_cert(certs, key)
                .map_err(|e| SpawnError::Relay(format!("TLS certificate: {e}")))?;
            Ok(TlsConfig::new(
                *https_bind,
                CertConfig::Manual { server_config },
            ))
        }
    }
}

/// A bound address that clients can dial: a wildcard bind is reached over
/// loopback. Only used for plain-HTTP test relays and certificates for IPs.
fn dialable(addr: SocketAddr) -> SocketAddr {
    if addr.ip().is_unspecified() {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), addr.port())
    } else {
        addr
    }
}

#[cfg(test)]
mod tests {
    use yakvc_shared::SecretKey;

    use super::*;

    fn id(n: u8) -> EndpointId {
        SecretKey::from_bytes(&[n; 32]).public()
    }

    #[test]
    fn rendezvous_is_always_admitted() {
        let gate = RelayGate::new(Arc::default(), id(0), Duration::from_secs(30));
        let now = Instant::now();
        for _ in 0..10 {
            assert_eq!(gate.admit(id(0), now), Admission::Session);
        }
    }

    #[test]
    fn grace_admissions_are_rate_limited_per_endpoint() {
        let gate = RelayGate::new(Arc::default(), id(0), Duration::from_secs(30));
        let now = Instant::now();
        for _ in 0..GRACE_ADMISSIONS {
            assert_eq!(gate.admit(id(1), now), Admission::Grace);
        }
        assert_eq!(gate.admit(id(1), now), Admission::Refused);
        assert_eq!(gate.admit(id(2), now), Admission::Grace);
        // One admission comes back after a third of the period.
        assert_eq!(gate.admit(id(1), now + GRACE_PERIOD / 3), Admission::Grace);
    }

    #[test]
    fn wildcard_binds_are_dialled_over_loopback() {
        assert_eq!(
            dialable("0.0.0.0:3340".parse().unwrap()),
            "127.0.0.1:3340".parse().unwrap()
        );
        assert_eq!(
            dialable("10.0.0.1:3340".parse().unwrap()),
            "10.0.0.1:3340".parse().unwrap()
        );
    }
}
