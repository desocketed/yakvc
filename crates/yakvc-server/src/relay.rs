//! The embedded `iroh-relay`, open only to clients with a rendezvous session.
//!
//! A client's relay connection can arrive before its rendezvous session, and
//! a `relay_only` client can only reach the rendezvous through the relay, so
//! an EndpointId without a session is admitted for a grace period. If it has
//! not registered by then, or when its session ends, it is disconnected.
//! `RelayOptions::open` turns all of this off for self-hosted test relays.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::num::NonZeroU32;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};

use iroh_relay::server::{
    Access, AccessControl, AcmeConfig, CertConfig, ClientRateLimit, ClientRequest, QuicConfig,
    RelayConfig, RelayService, ServerConfig, TlsConfig,
};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use yakvc_proto::{EndpointId, RelayUrl};

use crate::limits::{RateLimiter, TokenBucket};
use crate::server::{RelayOptions, RelayTls, SpawnError};
use crate::sessions::Sessions;

/// Grace admissions allowed per EndpointId per [`GRACE_PERIOD`]. A client
/// that keeps reconnecting to the relay without ever registering is refused
/// after this many.
const GRACE_ADMISSIONS: u32 = 3;
const GRACE_PERIOD: Duration = Duration::from_secs(600);

/// Grace admissions for the whole relay: bursts of up to [`GRACE_BURST`]
/// (enough for everyone reconnecting after a restart), refilling at two a
/// second. Keys cost nothing, so the per-EndpointId limit alone would let a
/// non-Yak app relay for free by switching keys; this bounds it to about
/// 560 connections at once (the burst plus 30 s of refill), each held to
/// the per-client bandwidth limit.
const GRACE_BURST: u32 = 500;
const GRACE_REFILL: Duration = Duration::from_secs(250);

/// Decides who may use the relay.
#[derive(Debug)]
pub(crate) struct RelayGate {
    sessions: Arc<Sessions>,
    /// The rendezvous' own endpoint, which uses the relay as its home relay.
    rendezvous: EndpointId,
    grace: Duration,
    grace_limiter: Mutex<RateLimiter<EndpointId>>,
    grace_admissions: Mutex<TokenBucket>,
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
            grace_admissions: Mutex::new(TokenBucket::new(
                GRACE_BURST,
                GRACE_REFILL,
                Instant::now(),
            )),
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
        } else {
            // The relay-wide budget is looked at first, so a flood of fresh
            // keys stops costing a bucket each once it is spent, and taken
            // last, so a refused connect charges neither limit.
            let mut limiter = self.grace_limiter.lock().unwrap();
            let mut admissions = self.grace_admissions.lock().unwrap();
            if admissions.has_token(now) && limiter.allow(id, now) && admissions.take(now) {
                Admission::Grace
            } else {
                Admission::Refused
            }
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

/// What the relay calls. The gate holds the relay (to disconnect clients),
/// so this holds the gate weakly: a strong reference would be a cycle that
/// keeps the gate, the sessions and their connections, and with them the
/// rendezvous socket, alive after the server shuts down.
#[derive(Debug)]
struct GateAccess(Weak<RelayGate>);

impl AccessControl for GateAccess {
    async fn on_connect(&self, request: &ClientRequest) -> Access {
        let refused = Access::Deny {
            reason: Some("no rendezvous session".into()),
        };
        // Gone once the server has shut down.
        let Some(gate) = self.0.upgrade() else {
            return refused;
        };
        let id = request.endpoint_id();
        match gate.admit(id, Instant::now()) {
            Admission::Session => Access::Allow,
            Admission::Grace => {
                // Weak for the same reason: the timer mustn't keep the gate.
                let grace = gate.grace;
                let gate = Arc::downgrade(&gate);
                tokio::spawn(async move {
                    tokio::time::sleep(grace).await;
                    if let Some(gate) = gate.upgrade() {
                        gate.session_ended(id);
                    }
                });
                Access::Allow
            }
            Admission::Refused => refused,
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
        relay.access = Arc::new(GateAccess(Arc::downgrade(&gate)));
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
    use std::sync::atomic::{AtomicU32, Ordering};

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;
    use yakvc_proto::SecretKey;

    use super::*;

    /// Longer than any deadline the relay gives a connection that hasn't
    /// become a relay client. The tests run on a paused clock, so waiting
    /// this long takes no time.
    const PATIENCE: Duration = Duration::from_secs(300);

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
    fn grace_admissions_are_rate_limited_for_the_whole_relay() {
        let gate = RelayGate::new(Arc::default(), id(0), Duration::from_secs(30));
        let now = Instant::now();
        // Fresh keys get around the per-EndpointId limit.
        for _ in 0..GRACE_BURST {
            let fresh = SecretKey::generate().public();
            assert_eq!(gate.admit(fresh, now), Admission::Grace);
        }
        assert_eq!(
            gate.admit(SecretKey::generate().public(), now),
            Admission::Refused
        );
        // Two admissions a second come back.
        let later = now + Duration::from_secs(1);
        for _ in 0..2 {
            let fresh = SecretKey::generate().public();
            assert_eq!(gate.admit(fresh, later), Admission::Grace);
        }
        assert_eq!(
            gate.admit(SecretKey::generate().public(), later),
            Admission::Refused
        );
    }

    #[test]
    fn a_refused_grace_admission_charges_neither_limit() {
        let gate = RelayGate::new(Arc::default(), id(0), Duration::from_secs(30));
        let now = Instant::now();
        for _ in 0..GRACE_BURST {
            assert_eq!(
                gate.admit(SecretKey::generate().public(), now),
                Admission::Grace
            );
        }
        // Refused keys get no bucket, so a flood can't grow the map.
        for _ in 0..GRACE_ADMISSIONS {
            assert_eq!(gate.admit(id(1), now), Admission::Refused);
        }
        assert_eq!(
            gate.grace_limiter.lock().unwrap().len(),
            GRACE_BURST as usize
        );
        // And the refusals cost the honest client none of its admissions.
        let later = now + Duration::from_secs(2);
        for _ in 0..GRACE_ADMISSIONS {
            assert_eq!(gate.admit(id(1), later), Admission::Grace);
        }
        assert_eq!(gate.admit(id(1), later), Admission::Refused);
    }

    /// A relay with HTTP, HTTPS and QUIC listeners on `bind`'s address.
    async fn tls_relay(bind: &str) -> iroh_relay::server::Server {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "yakvc-relay-tls-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let certified = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let (cert, key) = (dir.join("cert.pem"), dir.join("key.pem"));
        std::fs::write(&cert, certified.cert.pem()).unwrap();
        std::fs::write(&key, certified.signing_key.serialize_pem()).unwrap();
        let options = RelayOptions {
            http_bind: bind.parse().unwrap(),
            tls: Some(RelayTls::Files {
                https_bind: bind.parse().unwrap(),
                domain: "localhost".into(),
                cert,
                key,
            }),
            quic_bind: Some(bind.parse().unwrap()),
            open: false,
        };
        let gate = Arc::new(RelayGate::new(Arc::default(), id(0), GRACE_PERIOD));
        let (server, _) = spawn(&options, gate, 0).await.unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        server
    }

    /// A relay without TLS, as tests and dev sessions run it.
    async fn plain_relay() -> iroh_relay::server::Server {
        let options = RelayOptions {
            http_bind: "127.0.0.1:0".parse().unwrap(),
            tls: None,
            quic_bind: None,
            open: false,
        };
        let gate = Arc::new(RelayGate::new(Arc::default(), id(0), GRACE_PERIOD));
        spawn(&options, gate, 0).await.unwrap().0
    }

    /// Every relay listener bound to `[::]` serves IPv4 too. The rendezvous
    /// and plain HTTP are covered in `tests.rs`; HTTPS and QUIC address
    /// discovery need a certificate, which the rendezvous endpoint in those
    /// tests wouldn't trust.
    #[tokio::test]
    async fn wildcard_ipv6_binds_serve_both_families() {
        let server = tls_relay("[::]:0").await;

        let http = server.http_addr().unwrap().port();
        let https = server.https_addr().unwrap().port();
        let quic = server.quic_addr().unwrap().port();
        for ip in [IpAddr::from(Ipv4Addr::LOCALHOST), "::1".parse().unwrap()] {
            for port in [http, https] {
                tokio::net::TcpStream::connect((ip, port))
                    .await
                    .unwrap_or_else(|e| panic!("TCP {ip} port {port}: {e}"));
            }
            assert!(
                quic_answers(SocketAddr::new(ip, quic)).await,
                "QUIC {ip} port {quic}"
            );
        }
    }

    /// Whether a QUIC server answers at `addr`. A long-header packet with an
    /// unknown version gets a Version Negotiation reply (RFC 9000 §6), which
    /// needs neither a handshake nor a trusted certificate.
    async fn quic_answers(addr: SocketAddr) -> bool {
        let local: SocketAddr = if addr.is_ipv4() {
            "127.0.0.1:0".parse().unwrap()
        } else {
            "[::1]:0".parse().unwrap()
        };
        let socket = tokio::net::UdpSocket::bind(local).await.unwrap();
        // Long header, a reserved version, 8-byte destination and source
        // connection ids, padded to the 1200 bytes an Initial must have.
        let mut packet = vec![0xc0, 0x0a, 0x0a, 0x0a, 0x0a, 8];
        packet.extend([1; 8]);
        packet.push(8);
        packet.extend([2; 8]);
        packet.resize(1200, 0);
        socket.send_to(&packet, addr).await.unwrap();

        let mut reply = [0; 1500];
        match tokio::time::timeout(Duration::from_secs(5), socket.recv(&mut reply)).await {
            // Version Negotiation is a long header with version 0.
            Ok(Ok(len)) => len >= 5 && reply[0] & 0x80 != 0 && reply[1..5] == [0; 4],
            _ => false,
        }
    }

    /// Connects to `addr`, sends `sent` and nothing more, like a scanner or
    /// a client that died, and keeps its end open.
    async fn stalled(addr: SocketAddr, sent: &[u8]) -> TcpStream {
        let mut stream = TcpStream::connect(dialable(addr)).await.unwrap();
        stream.write_all(sent).await.unwrap();
        stream
    }

    /// Lets the listeners accept what was just connected. The paused clock
    /// stands still meanwhile: it only runs on when nothing else is going on.
    async fn settle() {
        tokio::task::spawn_blocking(|| std::thread::sleep(Duration::from_millis(100)))
            .await
            .unwrap();
    }

    /// Whether the server ends the connection (closes or resets it) within
    /// [`PATIENCE`].
    async fn closed_by_server(mut stream: TcpStream) -> bool {
        let mut rest = Vec::new();
        tokio::time::timeout(PATIENCE, stream.read_to_end(&mut rest))
            .await
            .is_ok()
    }

    /// The captive-portal probe iroh clients send over plain HTTP.
    async fn probe_answered(addr: SocketAddr) -> bool {
        let request = b"GET /generate_204 HTTP/1.1\r\nHost: localhost\r\n\r\n";
        let mut stream = stalled(addr, request).await;
        let mut response = [0; 12];
        tokio::time::timeout(PATIENCE, stream.read_exact(&mut response))
            .await
            .is_ok_and(|read| read.is_ok())
            && response == *b"HTTP/1.1 204"
    }

    /// What a peer may have sent before going silent, per kind of listener.
    const STALLED_HTTP: [&[u8]; 3] = [b"", b"GET /gener", b"GET / HTTP/1.1\r\nHost: x\r\n\r\n"];
    /// Nothing, or the start of a TLS record that never completes.
    const STALLED_TLS: [&[u8]; 2] = [b"", &[0x16, 0x03, 0x01, 0x02, 0x00, 0x01]];

    /// A peer that connects and goes silent must not hold its socket for
    /// good: enough of them use up the server's file descriptors, and then
    /// nobody can connect.
    #[tokio::test(start_paused = true)]
    async fn stalled_connections_are_closed_on_every_tcp_listener() {
        let tls = tls_relay("127.0.0.1:0").await;
        let plain = plain_relay().await;
        let mut stalls = Vec::new();
        for sent in STALLED_HTTP {
            let http = tls.http_addr().unwrap();
            stalls.push(("HTTP beside HTTPS", sent, stalled(http, sent).await));
            let relay = plain.http_addr().unwrap();
            stalls.push(("HTTP relay", sent, stalled(relay, sent).await));
        }
        for sent in STALLED_TLS {
            let https = tls.https_addr().unwrap();
            stalls.push(("HTTPS", sent, stalled(https, sent).await));
        }
        settle().await;

        let mut kept = Vec::new();
        for (listener, sent, stream) in stalls {
            if !closed_by_server(stream).await {
                kept.push((listener, String::from_utf8_lossy(sent)));
            }
        }
        assert!(kept.is_empty(), "silent connections kept open: {kept:?}");
    }

    /// The outage this guards against: silent connections piling up on the
    /// plain HTTP listener. However many there are, a real client is served
    /// while they are held, and none of them is held for good.
    #[tokio::test(start_paused = true)]
    async fn a_flood_of_idle_connections_passes() {
        let server = tls_relay("127.0.0.1:0").await;
        let http = server.http_addr().unwrap();
        let mut flood = Vec::new();
        for _ in 0..200 {
            flood.push(stalled(http, b"").await);
        }
        settle().await;
        assert!(probe_answered(http).await, "no answer during the flood");

        for stream in flood {
            assert!(closed_by_server(stream).await, "an idle connection stayed");
        }
        assert!(probe_answered(http).await, "no answer after the flood");
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
