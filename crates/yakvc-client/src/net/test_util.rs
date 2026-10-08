//! Helpers for tests that run real Iroh endpoints on loopback.

use std::net::Ipv4Addr;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use iroh::endpoint::{Connection, presets};
use iroh::{Endpoint, EndpointAddr, RelayMode, SecretKey, TransportAddr};
use tokio::sync::mpsc;
use yakvc_shared::peer::ALPN;
use yakvc_shared::{IssuerKey, ProtocolId, SignedTicket, TicketBody, Uuid};

use super::{PeerLink, Protocol};

/// An endpoint on 127.0.0.1 with no relay, speaking the peer ALPN.
pub(crate) async fn loopback_endpoint() -> Endpoint {
    loopback_endpoint_with_key(SecretKey::generate()).await
}

pub(crate) async fn loopback_endpoint_with_key(key: SecretKey) -> Endpoint {
    Endpoint::builder(presets::Minimal)
        .secret_key(key)
        .alpns(vec![ALPN.to_vec()])
        .relay_mode(RelayMode::Disabled)
        .clear_ip_transports()
        .bind_addr("127.0.0.1:0")
        .unwrap()
        .bind()
        .await
        .unwrap()
}

/// `endpoint`'s IPv4 loopback address, also for endpoints bound to all
/// interfaces.
pub(crate) fn loopback_addr(endpoint: &Endpoint) -> EndpointAddr {
    let loopback = endpoint
        .bound_sockets()
        .into_iter()
        .filter(|socket| socket.is_ipv4())
        .map(|socket| TransportAddr::Ip((Ipv4Addr::LOCALHOST, socket.port()).into()));
    EndpointAddr::from_parts(endpoint.id(), loopback)
}

/// Connects `from` to `to`, returning both ends.
pub(crate) async fn connect(from: &Endpoint, to: &Endpoint) -> (Connection, Connection) {
    let accept = {
        let to = to.clone();
        tokio::spawn(async move { to.accept().await.unwrap().await.unwrap() })
    };
    let dialed = from.connect(loopback_addr(to), ALPN).await.unwrap();
    (dialed, accept.await.unwrap())
}

/// A protocol that hands each [`PeerLink`] to the test.
pub(crate) struct Probe {
    pub id: ProtocolId,
    pub version: u16,
    pub links: mpsc::UnboundedSender<PeerLink>,
}

/// A [`Probe`] for protocol `id`, and where its links arrive.
pub(crate) fn probe(
    id: u8,
    version: u16,
) -> (Arc<dyn Protocol>, mpsc::UnboundedReceiver<PeerLink>) {
    let (links, rx) = mpsc::unbounded_channel();
    let probe = Probe {
        id: ProtocolId(id),
        version,
        links,
    };
    (Arc::new(probe), rx)
}

impl Protocol for Probe {
    fn id(&self) -> ProtocolId {
        self.id
    }

    fn version(&self) -> u16 {
        self.version
    }

    fn serve(&self, link: PeerLink) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        let links = self.links.clone();
        Box::pin(async move {
            let _ = links.send(link);
        })
    }
}

pub(crate) fn uuid(n: u8) -> Uuid {
    Uuid::from_bytes([n; 16])
}

/// A verified ticket for `endpoint` as `uuid`, signed by `issuer`.
pub(crate) fn ticket(issuer: &IssuerKey, uuid: Uuid, endpoint: &Endpoint) -> SignedTicket {
    let name = format!("player{}", uuid.as_bytes()[0]);
    signed_ticket(issuer, uuid, &name, endpoint, true)
}

/// Like [`ticket`], but lasting `lifetime` (whole seconds, counted from the
/// start of the current second) instead of an hour.
pub(crate) fn ticket_lasting(
    issuer: &IssuerKey,
    uuid: Uuid,
    endpoint: &Endpoint,
    lifetime: Duration,
) -> SignedTicket {
    let name = format!("player{}", uuid.as_bytes()[0]);
    let body = TicketBody::new(uuid, name, endpoint.id(), true, SystemTime::now(), lifetime);
    issuer.sign(&body)
}

/// A ticket for `endpoint` as `uuid` and `name`, signed by `issuer`.
pub(crate) fn signed_ticket(
    issuer: &IssuerKey,
    uuid: Uuid,
    name: &str,
    endpoint: &Endpoint,
    verified: bool,
) -> SignedTicket {
    let body = TicketBody::new(
        uuid,
        name.into(),
        endpoint.id(),
        verified,
        SystemTime::now(),
        Duration::from_secs(3600),
    );
    issuer.sign(&body)
}

/// Waits up to five seconds for `condition`.
pub(crate) async fn eventually(what: &str, mut condition: impl FnMut() -> bool) {
    for _ in 0..500 {
        if condition() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out waiting for {what}");
}
