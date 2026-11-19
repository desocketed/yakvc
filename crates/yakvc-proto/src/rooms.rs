//! Public room list, ALPN [`ALPN`]: no sign-in, anyone may list and announce.
//!
//! One bi-stream per connection, opened by the client. After
//! [`ClientMsg::Watch`] the server sends one [`ServerMsg::Added`] per listed
//! room and then [`ServerMsg::Listed`], and after that `Added` and
//! [`ServerMsg::Gone`] as the list changes. (One room per message keeps a
//! full list within the frame limit.) The client may
//! [`Announce`](ClientMsg::Announce) rooms, each answered with `Announced` or
//! `Refused`, and [`Withdraw`](ClientMsg::Withdraw) them. A room is listed for
//! as long as the connection that announced it, so a host that goes away
//! takes its rooms with it.
//!
//! The server treats a room's code as opaque: what it means (an address to
//! dial, say) is up to the application.

use serde::{Deserialize, Serialize};

pub const ALPN: &[u8] = b"yakvc/rooms/1";

/// A listed room.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Room {
    /// What people see in the list.
    pub label: String,
    /// What they join it by; unique in the list.
    pub code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ClientMsg {
    /// Send the list and its changes from now on. Again is a no-op.
    Watch,
    /// List a room, or relabel one this connection already listed.
    Announce(Room),
    /// Unlist a room this connection listed, by code.
    Withdraw(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServerMsg {
    /// A room is listed, or relabelled.
    Added(Room),
    /// Every room listed when the connection opened has been sent.
    Listed,
    /// The room with this code is no longer listed.
    Gone(String),
    /// The announce of this code is listed.
    Announced(String),
    /// The announce of this code was refused; nothing changed.
    Refused { code: String, reason: Refusal },
}

/// Why an announce was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Refusal {
    /// The label is empty, too long or holds control characters, or the code
    /// is empty or too long.
    Invalid,
    /// Another connection lists a room with this code.
    Taken,
    /// This connection or its source lists as many rooms as it may.
    TooManyRooms,
    /// The list is full.
    ListFull,
    /// Too many announces; try again later.
    RateLimited,
}

/// QUIC application close codes for room-list connections.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum CloseCode {
    Normal = 0,
    ProtocolError = 1,
    /// Too many connections, or the client stopped reading.
    LimitExceeded = 4,
    ShuttingDown = 5,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Refusal::Invalid => "invalid label or code",
            Refusal::Taken => "code already listed",
            Refusal::TooManyRooms => "too many rooms",
            Refusal::ListFull => "room list full",
            Refusal::RateLimited => "too many announces",
        })
    }
}
