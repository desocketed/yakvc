//! Peer ↔ peer connection protocol, ALPN [`ALPN`].
//!
//! Both sides open a control bi-stream and send [`PeerMsg::Hello`]. After both
//! hellos verify, each agreed application protocol opens its own control
//! stream (first byte: its [`ProtocolId`]) and prefixes its datagrams with the
//! same byte. The peer layer strips that byte before handing data to the
//! protocol.

use serde::{Deserialize, Serialize};

use crate::ProtocolId;
use crate::ticket::SignedTicket;

pub const ALPN: &[u8] = b"yakvc/peer/1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PeerMsg {
    Hello(PeerHello),
    /// Sent to every open peer after renewing a ticket.
    TicketUpdate(SignedTicket),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerHello {
    pub ticket: SignedTicket,
    /// Each protocol this side speaks, with the highest version it supports.
    pub protocols: Vec<(ProtocolId, u16)>,
}

/// QUIC application close codes for peer connections.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum CloseCode {
    Normal = 0,
    ProtocolError = 1,
    BadTicket = 2,
    /// The remote UUID is not in our tab list.
    NotVisible = 3,
    /// Lost the duplicate-connection tie break.
    Duplicate = 4,
    /// Exceeded datagram rate or size limits.
    Flooding = 5,
    ShuttingDown = 6,
    /// The peer is at its connection cap and we rank too low to displace
    /// anyone, or we were evicted for a better-ranked peer.
    TooManyPeers = 7,
}
