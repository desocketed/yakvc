//! Types shared by the Yak VC client and rendezvous server: wire protocol,
//! tickets, pair tokens and the Mojang session-server client.

pub use iroh_base::EndpointId;

/// ALPN for client ↔ rendezvous connections.
pub const ALPN_RDV: &[u8] = b"yakvc/rdv/1";

/// ALPN for peer ↔ peer connections.
pub const ALPN_PEER: &[u8] = b"yakvc/peer/1";

/// Application protocol IDs carried on a peer connection. The ID is the first
/// byte of every datagram and opens each protocol's control stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ProtocolId {
    Voice = 1,
}
