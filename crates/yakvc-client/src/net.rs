//! Iroh endpoint, rendezvous session, peer manager and protocol routing.
//!
//! `net` delivers verified peers: each [`PeerLink`] is one QUIC connection
//! whose remote EndpointId matched a trusted ticket for a UUID in the tab
//! list. Application protocols implement [`Protocol`] and get one link per
//! peer; they never see unverified traffic.
//!
//! This seam is internal to `yakvc-client` (one agent owns both sides), so it
//! is a sketch to refine during implementation, not a fixed contract.

use std::future::Future;
use std::pin::Pin;

use bytes::Bytes;
use yakvc_shared::{ProtocolId, Uuid};

/// An application protocol on peer connections.
pub trait Protocol: Send + Sync + 'static {
    fn id(&self) -> ProtocolId;

    /// Highest version this side speaks.
    fn version(&self) -> u16;

    /// Serves one peer until the link closes.
    fn serve(&self, link: PeerLink) -> Pin<Box<dyn Future<Output = ()> + Send>>;
}

/// One verified peer, scoped to one protocol.
#[derive(Debug)]
pub struct PeerLink {
    _p: (),
}

#[derive(Debug, thiserror::Error)]
pub enum LinkError {
    #[error("peer connection closed")]
    Closed,
    #[error("datagram too large")]
    TooLarge,
}

impl PeerLink {
    pub fn uuid(&self) -> Uuid {
        todo!()
    }

    /// Version agreed in the peer hello.
    pub fn version(&self) -> u16 {
        todo!()
    }

    /// Sends a datagram; the protocol byte is added for you.
    pub fn send_datagram(&self, payload: &[u8]) -> Result<(), LinkError> {
        let _ = payload;
        todo!()
    }

    /// Next datagram, protocol byte removed. `None` once closed.
    pub async fn recv_datagram(&mut self) -> Option<Bytes> {
        todo!()
    }

    /// Sends a message on this protocol's control stream.
    pub async fn send_control<M: serde::Serialize>(&mut self, msg: &M) -> Result<(), LinkError> {
        let _ = msg;
        todo!()
    }

    /// Next message on this protocol's control stream. `None` once closed.
    pub async fn recv_control<M: serde::de::DeserializeOwned>(&mut self) -> Option<M> {
        todo!()
    }
}
