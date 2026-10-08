use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use iroh::endpoint::presets;
use iroh::{Endpoint, RelayMode};
use tokio::task::JoinSet;
use yakvc_shared::rdv::{self, CloseCode};
use yakvc_shared::{EndpointAddr, EndpointId, IssuerId, IssuerKey, RelayUrl, SecretKey};

use crate::auth::Auth;
use crate::limits::{IpSlots, RateLimiter, Slots, TokenBucket};
use crate::metrics::Metrics;
use crate::mojang::SessionServer;
use crate::relay::RelayGate;
use crate::sessions::Sessions;

/// How often rate limiters forget idle keys.
const PRUNE_INTERVAL: Duration = Duration::from_secs(60);

/// How long `spawn` waits for the rendezvous endpoint to connect to its relay.
const RELAY_ONLINE_TIMEOUT: Duration = Duration::from_secs(10);

/// A running rendezvous (and relay, if enabled).
#[derive(Debug)]
pub struct Server {
    endpoint: Endpoint,
    issuer_id: IssuerId,
    shared: Arc<Shared>,
    pub(crate) relay: Option<iroh_relay::server::Server>,
    relay_url: Option<RelayUrl>,
    /// The accept loop and housekeeping; dropping the set stops them.
    tasks: JoinSet<()>,
}

pub struct ServerBuilder {
    endpoint_key: SecretKey,
    issuer_key: IssuerKey,
    bind: SocketAddr,
    relay: Option<RelayOptions>,
    insecure_dev_auth: bool,
    session_server: Option<SessionServer>,
    ticket_lifetime: Duration,
    limits: Limits,
    metrics: Option<SocketAddr>,
    extra_relays: Vec<RelayUrl>,
}

/// Embedded relay settings.
#[derive(Debug, Clone)]
pub struct RelayOptions {
    /// Plain HTTP: captive-portal probe, ACME challenges, and the relay itself
    /// when `tls` is `None` (tests only).
    pub http_bind: SocketAddr,
    pub tls: Option<RelayTls>,
    /// QUIC address discovery.
    pub quic_bind: Option<SocketAddr>,
    /// Serve every client, not only those with a rendezvous session. For a
    /// self-hosted relay used for direct calls (`yakvc call`), which have no
    /// session; never for a public server.
    pub open: bool,
}

#[derive(Debug, Clone)]
pub enum RelayTls {
    LetsEncrypt {
        https_bind: SocketAddr,
        domain: String,
        contact: String,
        cache_dir: PathBuf,
    },
    /// Certificate files for `domain`, which the relay URL uses.
    Files {
        https_bind: SocketAddr,
        domain: String,
        cert: PathBuf,
        key: PathBuf,
    },
}

/// Abuse limits. Defaults are the DESIGN.md values.
#[derive(Debug, Clone, PartialEq)]
pub struct Limits {
    pub auth_per_endpoint_per_min: u32,
    pub auth_per_ip_per_min: u32,
    pub max_pairs: usize,
    pub pair_updates_per_sec: u32,
    /// Bytes per second each client may send into the relay.
    pub relay_client_bytes_per_sec: u32,
    /// How long a relay client may stay without a rendezvous session.
    pub relay_grace: Duration,
    /// Time from accepting a connection until the client must be
    /// registered. Covers the client's `joinServer` call to Mojang.
    pub auth_timeout: Duration,
    /// Challenges the whole server may issue per minute (with a burst of as
    /// many). Each one can lead to a `hasJoined` call, and fresh keys get
    /// around the per-EndpointId limit, so this keeps a flood from pushing
    /// the server into Mojang's rate limit, which would block every sign-in.
    pub mojang_checks_per_min: u32,
    /// Challenges per minute, shared by every connection without a known
    /// source IP (those arriving through the relay), which the per-IP limit
    /// can't cover.
    pub unknown_ip_challenges_per_min: u32,
    /// Connections that may be open at once without being registered yet.
    pub max_unregistered: usize,
    /// The same, per source IP (IPv6: per /64), so that one host can't take
    /// every slot. Connections through the relay only count against
    /// `max_unregistered`, since their IP is unknown.
    pub max_unregistered_per_ip: usize,
    /// Sessions (counted from `Hello`) per source IP (IPv6: per /64).
    /// Offline UUIDs cost only a `Decline`, so this is what bounds how many
    /// sessions one host can make the matcher work for. Room for a LAN or
    /// a shared NAT; sessions through the relay have no known IP and are
    /// bounded by `unknown_ip_challenges_per_min` instead.
    pub max_sessions_per_ip: usize,
}

/// Live counters, for metrics and tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Stats {
    pub sessions: usize,
    /// Pairs currently matched.
    pub matches: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum SpawnError {
    #[error("bind {addr}: {source}")]
    Bind {
        addr: SocketAddr,
        #[source]
        source: std::io::Error,
    },
    #[error("relay: {0}")]
    Relay(String),
}

/// State every connection handler shares.
#[derive(Debug)]
pub(crate) struct Shared {
    pub auth: Auth,
    pub limits: Limits,
    pub sessions: Arc<Sessions>,
    pub relay_gate: Option<Arc<RelayGate>>,
    pub auth_per_endpoint: Mutex<RateLimiter<EndpointId>>,
    pub auth_per_ip: Mutex<RateLimiter<IpAddr>>,
    pub mojang_checks: Mutex<TokenBucket>,
    pub unknown_ip_challenges: Mutex<TokenBucket>,
    pub unregistered: Slots,
    pub unregistered_per_ip: IpSlots,
    pub sessions_per_ip: IpSlots,
    /// The relays clients may advertise: ours and `extra_relays`.
    pub relays: Vec<RelayUrl>,
    pub metrics: Metrics,
}

impl Server {
    pub fn builder(endpoint_key: SecretKey, issuer_key: IssuerKey) -> ServerBuilder {
        ServerBuilder {
            endpoint_key,
            issuer_key,
            bind: SocketAddr::from(([0, 0, 0, 0], 0)),
            relay: None,
            insecure_dev_auth: false,
            session_server: None,
            ticket_lifetime: Duration::from_secs(24 * 3600),
            limits: Limits::default(),
            metrics: None,
            extra_relays: Vec::new(),
        }
    }

    pub fn endpoint_id(&self) -> EndpointId {
        self.endpoint.id()
    }

    /// Bound addresses, including the relay URL if one is running.
    pub fn endpoint_addr(&self) -> EndpointAddr {
        let mut addr = EndpointAddr::new(self.endpoint.id());
        for socket in self.endpoint.bound_sockets() {
            addr = addr.with_ip_addr(socket);
        }
        if let Some(url) = &self.relay_url {
            addr = addr.with_relay_url(url.clone());
        }
        addr
    }

    pub fn issuer_id(&self) -> IssuerId {
        self.issuer_id
    }

    pub fn relay_url(&self) -> Option<RelayUrl> {
        self.relay_url.clone()
    }

    pub fn stats(&self) -> Stats {
        self.shared.sessions.stats()
    }

    /// Closes every session as a restart would, and keeps serving. For tests
    /// of clients reconnecting; a restart in the same process can't bind the
    /// same ports again.
    #[doc(hidden)]
    pub fn close_sessions(&self) {
        self.shared
            .sessions
            .close_all(CloseCode::ShuttingDown, "server restarting");
    }

    /// Closes all sessions and stops the relay.
    pub async fn shutdown(mut self) {
        self.tasks.shutdown().await;
        self.shared
            .sessions
            .close_all(CloseCode::ShuttingDown, "server shutting down");
        self.endpoint.close().await;
        if let Some(relay) = self.relay {
            let _ = relay.shutdown().await;
        }
    }
}

impl ServerBuilder {
    /// Rendezvous QUIC address. Defaults to `0.0.0.0:0`. `[::]` also serves
    /// IPv4.
    pub fn bind(mut self, addr: SocketAddr) -> Self {
        self.bind = addr;
        self
    }

    /// Runs an embedded relay. Off by default.
    pub fn relay(mut self, relay: RelayOptions) -> Self {
        self.relay = Some(relay);
        self
    }

    /// Skips Mojang and issues dev tickets for any claimed UUID.
    pub fn insecure_dev_auth(mut self) -> Self {
        self.insecure_dev_auth = true;
        self
    }

    /// Mojang session server to verify against. Defaults to Mojang's, with
    /// the URL taken from Mojang's discovery document.
    pub fn session_server(mut self, server: SessionServer) -> Self {
        self.session_server = Some(server);
        self
    }

    /// Defaults to 24 hours.
    pub fn ticket_lifetime(mut self, lifetime: Duration) -> Self {
        self.ticket_lifetime = lifetime;
        self
    }

    pub fn limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    /// Relays besides our own that clients may advertise to each other, for
    /// a relay run apart from the rendezvous. Any other relay URL is dropped
    /// from clients' addresses: a peer dialling an arbitrary relay would
    /// reveal its IP to whoever runs it.
    pub fn extra_relays(mut self, relays: Vec<RelayUrl>) -> Self {
        self.extra_relays = relays;
        self
    }

    /// Serves Prometheus metrics at this address.
    pub fn metrics(mut self, addr: SocketAddr) -> Self {
        self.metrics = Some(addr);
        self
    }

    pub async fn spawn(self) -> Result<Server, SpawnError> {
        let endpoint_id = self.endpoint_key.public();
        let issuer_id = self.issuer_key.id();
        let sessions = Arc::new(Sessions::default());

        // The relay comes first: the rendezvous endpoint uses it as its home
        // relay, so `relay_only` clients can reach the rendezvous through it.
        let mut relay = None;
        let mut relay_url = None;
        let mut relay_gate = None;
        if let Some(options) = &self.relay {
            let gate = Arc::new(RelayGate::new(
                sessions.clone(),
                endpoint_id,
                self.limits.relay_grace,
            ));
            let (server, url) = crate::relay::spawn(
                options,
                gate.clone(),
                self.limits.relay_client_bytes_per_sec,
            )
            .await?;
            relay = Some(server);
            relay_url = Some(url);
            // An open relay never disconnects anyone, so there is no gate to tell.
            relay_gate = (!options.open).then_some(gate);
        }

        let endpoint = bind_endpoint(self.endpoint_key, self.bind, relay_url.clone()).await?;
        if relay_url.is_some() {
            // Until the endpoint is on its relay, a relay_only client's first
            // attempt stalls and can outlast its grace admission. Waiting is
            // only an optimisation, so give up after a while (an ACME
            // certificate, for example, can take longer).
            let _ = tokio::time::timeout(RELAY_ONLINE_TIMEOUT, endpoint.online()).await;
        }

        let mojang = if self.insecure_dev_auth {
            None
        } else {
            Some(self.session_server.unwrap_or_default())
        };
        let mut tasks = JoinSet::new();
        if let Some(mojang) = &mojang {
            tasks.spawn(mojang.clone().discovery_loop());
        }
        let minute = Duration::from_secs(60);
        let shared = Arc::new(Shared {
            auth: Auth::new(self.issuer_key, self.ticket_lifetime, mojang, endpoint_id),
            auth_per_endpoint: Mutex::new(RateLimiter::new(
                self.limits.auth_per_endpoint_per_min,
                minute,
            )),
            auth_per_ip: Mutex::new(RateLimiter::new(self.limits.auth_per_ip_per_min, minute)),
            mojang_checks: Mutex::new(TokenBucket::new(
                self.limits.mojang_checks_per_min,
                minute,
                Instant::now(),
            )),
            unknown_ip_challenges: Mutex::new(TokenBucket::new(
                self.limits.unknown_ip_challenges_per_min,
                minute,
                Instant::now(),
            )),
            unregistered: Slots::new(self.limits.max_unregistered),
            unregistered_per_ip: IpSlots::new(self.limits.max_unregistered_per_ip),
            sessions_per_ip: IpSlots::new(self.limits.max_sessions_per_ip),
            relays: relay_url.iter().cloned().chain(self.extra_relays).collect(),
            limits: self.limits,
            sessions,
            relay_gate,
            metrics: Metrics::default(),
        });

        tasks.spawn(accept_loop(endpoint.clone(), shared.clone()));
        tasks.spawn(prune_loop(shared.clone()));
        if let Some(addr) = self.metrics {
            let listener = tokio::net::TcpListener::bind(addr)
                .await
                .map_err(|source| SpawnError::Bind { addr, source })?;
            let relay_metrics = relay.as_ref().map(|r| r.metrics().server.clone());
            tasks.spawn(crate::metrics::serve(
                listener,
                shared.clone(),
                relay_metrics,
            ));
        }

        Ok(Server {
            endpoint,
            issuer_id,
            shared,
            relay,
            relay_url,
            tasks,
        })
    }
}

async fn bind_endpoint(
    key: SecretKey,
    addr: SocketAddr,
    relay_url: Option<RelayUrl>,
) -> Result<Endpoint, SpawnError> {
    let bind_error = |e: &dyn std::fmt::Display| SpawnError::Bind {
        addr,
        source: std::io::Error::other(e.to_string()),
    };
    let relay_mode = match relay_url {
        Some(url) => RelayMode::custom([url]),
        None => RelayMode::Disabled,
    };
    // `Minimal` only picks the crypto provider: no n0 relays or address
    // lookup, and only the sockets we were asked to bind.
    let mut builder = Endpoint::builder(presets::Minimal)
        .secret_key(key)
        .alpns(vec![rdv::ALPN.to_vec()])
        .relay_mode(relay_mode)
        .clear_ip_transports()
        .bind_addr(addr)
        .map_err(|e| bind_error(&e))?;
    // iroh makes its IPv6 sockets IPv6-only, unlike the relay's listeners,
    // so `[::]` gets a second IPv4 socket on the same port to serve both
    // families like every other listener.
    if addr.ip() == Ipv6Addr::UNSPECIFIED {
        builder = builder
            .bind_addr(SocketAddr::from((Ipv4Addr::UNSPECIFIED, addr.port())))
            .map_err(|e| bind_error(&e))?;
    }
    builder.bind().await.map_err(|e| bind_error(&e))
}

async fn accept_loop(endpoint: Endpoint, shared: Arc<Shared>) {
    // Connection handlers live in this set, so they stop with the loop.
    let mut connections = JoinSet::new();
    while let Some(incoming) = endpoint.accept().await {
        connections.spawn(crate::rdv::serve(shared.clone(), incoming));
        // Reap finished handlers so the set doesn't grow forever.
        while connections.try_join_next().is_some() {}
    }
}

async fn prune_loop(shared: Arc<Shared>) {
    let mut interval = tokio::time::interval(PRUNE_INTERVAL);
    loop {
        interval.tick().await;
        let now = Instant::now();
        shared.auth_per_endpoint.lock().unwrap().prune(now);
        shared.auth_per_ip.lock().unwrap().prune(now);
        if let Some(gate) = &shared.relay_gate {
            gate.prune(now);
        }
    }
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            auth_per_endpoint_per_min: 5,
            auth_per_ip_per_min: 30,
            max_pairs: rdv::MAX_PAIRS,
            pair_updates_per_sec: rdv::PAIR_UPDATES_PER_SEC,
            relay_client_bytes_per_sec: 80_000,
            relay_grace: Duration::from_secs(30),
            auth_timeout: Duration::from_secs(30),
            // Mojang is said to allow about 600 `hasJoined` calls per 10
            // minutes per IP; a full bucket plus ten minutes of refill stays
            // under that.
            mojang_checks_per_min: 50,
            unknown_ip_challenges_per_min: 20,
            max_unregistered: 500,
            max_unregistered_per_ip: 10,
            max_sessions_per_ip: 64,
        }
    }
}

impl std::fmt::Debug for ServerBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerBuilder").finish_non_exhaustive()
    }
}
