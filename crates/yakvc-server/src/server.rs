use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use yakvc_shared::{EndpointAddr, EndpointId, IssuerId, IssuerKey, RelayUrl, SecretKey};

use crate::mojang::SessionServer;

/// A running rendezvous (and relay, if enabled).
#[derive(Debug)]
pub struct Server {
    _p: (),
}

pub struct ServerBuilder {
    _endpoint_key: SecretKey,
    _issuer_key: IssuerKey,
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
}

#[derive(Debug, Clone)]
pub enum RelayTls {
    LetsEncrypt {
        https_bind: SocketAddr,
        domain: String,
        contact: String,
        cache_dir: PathBuf,
    },
    Files {
        https_bind: SocketAddr,
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

impl Server {
    pub fn builder(endpoint_key: SecretKey, issuer_key: IssuerKey) -> ServerBuilder {
        ServerBuilder {
            _endpoint_key: endpoint_key,
            _issuer_key: issuer_key,
        }
    }

    pub fn endpoint_id(&self) -> EndpointId {
        todo!()
    }

    /// Bound addresses, including the relay URL if one is running.
    pub fn endpoint_addr(&self) -> EndpointAddr {
        todo!()
    }

    pub fn issuer_id(&self) -> IssuerId {
        todo!()
    }

    pub fn relay_url(&self) -> Option<RelayUrl> {
        todo!()
    }

    pub fn stats(&self) -> Stats {
        todo!()
    }

    /// Closes all sessions and stops the relay.
    pub async fn shutdown(self) {
        todo!()
    }
}

impl ServerBuilder {
    /// Rendezvous QUIC address. Defaults to `0.0.0.0:0`.
    pub fn bind(self, addr: SocketAddr) -> Self {
        let _ = addr;
        todo!()
    }

    /// Runs an embedded relay. Off by default.
    pub fn relay(self, relay: RelayOptions) -> Self {
        let _ = relay;
        todo!()
    }

    /// Skips Mojang and issues dev tickets for any claimed UUID.
    pub fn insecure_dev_auth(self) -> Self {
        todo!()
    }

    /// Mojang session server to verify against. Defaults to Mojang's.
    pub fn session_server(self, server: SessionServer) -> Self {
        let _ = server;
        todo!()
    }

    /// Defaults to 24 hours.
    pub fn ticket_lifetime(self, lifetime: Duration) -> Self {
        let _ = lifetime;
        todo!()
    }

    pub fn limits(self, limits: Limits) -> Self {
        let _ = limits;
        todo!()
    }

    /// Serves Prometheus metrics at this address.
    pub fn metrics(self, addr: SocketAddr) -> Self {
        let _ = addr;
        todo!()
    }

    pub async fn spawn(self) -> Result<Server, SpawnError> {
        todo!()
    }
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            auth_per_endpoint_per_min: 5,
            auth_per_ip_per_min: 30,
            max_pairs: 2048,
            pair_updates_per_sec: 10,
            relay_client_bytes_per_sec: 80_000,
            relay_grace: Duration::from_secs(30),
        }
    }
}

impl std::fmt::Debug for ServerBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerBuilder").finish_non_exhaustive()
    }
}
