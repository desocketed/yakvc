//! Types shared by the Yak VC client and rendezvous server: wire messages,
//! tickets, pair tokens and the auth challenge digest.
//!
//! Everything that crosses the network is defined here, so client and server
//! cannot disagree about the format. See DESIGN.md, "Wire format".

pub mod auth;
pub mod group;
pub mod pair;
pub mod peer;
pub mod rdv;
pub mod ticket;
pub mod voice;
pub mod wire;

pub use iroh_base::{EndpointAddr, EndpointId, RelayUrl, SecretKey};
pub use uuid::Uuid;

pub use crate::auth::{is_offline_player, offline_uuid};
pub use crate::group::{GroupAnnounce, GroupId, GroupKey, GroupState, PublicGroup};
pub use crate::pair::PairToken;
pub use crate::ticket::{
    IssuerId, IssuerKey, SignedTicket, Ticket, TicketBody, TicketError, TicketVerifier,
};
pub use crate::wire::WireError;

/// Application protocol carried on a peer connection.
///
/// The ID is the first byte of every datagram on the connection and the first
/// byte of each protocol's control stream. Unknown IDs are representable so a
/// peer can ignore protocols it doesn't speak.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct ProtocolId(pub u8);

impl ProtocolId {
    /// Proximity voice, see [`voice`].
    pub const VOICE: ProtocolId = ProtocolId(1);
}
