//! Client ↔ rendezvous protocol, ALPN [`ALPN`].
//!
//! One bi-stream per connection. The client sends [`ClientMsg::Hello`]; the
//! server either registers it straight away from a valid cached ticket or runs
//! the challenge (`Challenge` → client calls `joinServer` → `Joined`), then
//! sends `Registered`. After that the client streams pair-token updates and
//! the server streams `PeerAvailable` / `PeerGone`.

use iroh_base::{EndpointAddr, EndpointId};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth::Nonce;
use crate::pair::PairToken;
use crate::ticket::SignedTicket;

pub const ALPN: &[u8] = b"yakvc/rdv/1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ClientMsg {
    Hello(Hello),
    /// `joinServer` succeeded for the last `Challenge`.
    Joined,
    /// The answer to a `Challenge` when `joinServer` can't be done (no
    /// account, an offline launcher, Mojang down): asks for an unverified
    /// ticket, which is only issued for an offline UUID.
    Decline,
    /// Start a challenge for a fresh ticket on the existing session. Also
    /// replaces a challenge left unanswered by a failed `joinServer`.
    Renew,
    /// The client's addresses changed.
    UpdateAddr(EndpointAddr),
    /// Replace the whole pair-token set.
    SetPairs(Vec<PairToken>),
    AddPairs(Vec<PairToken>),
    RemovePairs(Vec<PairToken>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hello {
    pub mod_version: String,
    pub uuid: Uuid,
    pub name: String,
    pub addr: EndpointAddr,
    pub cached_ticket: Option<SignedTicket>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServerMsg {
    Challenge(Nonce),
    /// Session registered (or ticket renewed).
    Registered(SignedTicket),
    /// Mojang is rate-limiting the rendezvous or is unavailable, or a
    /// renewal was refused; retry the challenge later. A registered session
    /// stays up with its current ticket.
    RetryAfter {
        secs: u32,
    },
    /// A mutual match. Sent again if the peer's address changes.
    PeerAvailable {
        ticket: SignedTicket,
        addr: EndpointAddr,
    },
    PeerGone(EndpointId),
}

/// QUIC application close codes for rendezvous connections.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum CloseCode {
    Normal = 0,
    ProtocolError = 1,
    /// `hasJoined` did not confirm the claimed UUID.
    AuthFailed = 2,
    RateLimited = 3,
    LimitExceeded = 4,
    ShuttingDown = 5,
    /// A verified session took over this session's unverified UUID, or holds
    /// the UUID an unverified ticket was asked for.
    Superseded = 6,
}
