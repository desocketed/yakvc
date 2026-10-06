//! Client ↔ rendezvous protocol, ALPN [`ALPN`].
//!
//! One bi-stream per connection. The client sends [`ClientMsg::Hello`]; the
//! server either registers it straight away from a valid cached ticket or runs
//! the challenge (`Challenge` → client calls `joinServer` → `Joined`), then
//! sends `Registered`. After that the client streams pair-token updates and
//! the server streams `PeerAvailable` / `PeerGone`.

use iroh_base::{EndpointAddr, EndpointId, TransportAddr};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth::Nonce;
use crate::pair::PairToken;
use crate::ticket::SignedTicket;

pub const ALPN: &[u8] = b"yakvc/rdv/1";

/// Most pair tokens a session may hold; more closes it with
/// [`CloseCode::LimitExceeded`].
pub const MAX_PAIRS: usize = 2048;

/// Session updates (`SetPairs`, `AddPairs`, `RemovePairs` and `UpdateAddr`)
/// a client may send per second, in bursts of up to as many; more closes it
/// with [`CloseCode::RateLimited`].
pub const PAIR_UPDATES_PER_SEC: u32 = 10;

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

/// Most IP addresses an [`EndpointAddr`] may carry on the rendezvous. Real
/// clients have a handful (one per interface and family, plus what the relay
/// sees); the bound keeps `PeerAvailable` far below the message size limit.
pub const MAX_IP_ADDRS: usize = 16;

/// Longest relay URL an [`EndpointAddr`] may carry on the rendezvous.
pub const MAX_RELAY_URL_LEN: usize = 256;

/// `addr` reduced to what the rendezvous accepts: the first
/// [`MAX_IP_ADDRS`] IP addresses and one relay URL of at most
/// [`MAX_RELAY_URL_LEN`] bytes. Other transports are dropped.
pub fn fit_addr(addr: &EndpointAddr) -> EndpointAddr {
    let ips = addr
        .ip_addrs()
        .take(MAX_IP_ADDRS)
        .copied()
        .map(TransportAddr::Ip);
    let relay = addr
        .relay_urls()
        .find(|url| url.as_str().len() <= MAX_RELAY_URL_LEN)
        .cloned()
        .map(TransportAddr::Relay);
    EndpointAddr::from_parts(addr.id, ips.chain(relay))
}

/// Whether the rendezvous accepts `addr` as it is.
pub fn addr_fits(addr: &EndpointAddr) -> bool {
    fit_addr(addr) == *addr
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use iroh_base::{RelayUrl, SecretKey};

    use super::*;

    fn ips(count: usize) -> impl Iterator<Item = TransportAddr> {
        (0..count).map(|n| TransportAddr::Ip(SocketAddr::from(([127, 0, 0, 1], n as u16))))
    }

    #[test]
    fn small_addresses_fit() {
        let relay: RelayUrl = "https://relay.example.com".parse().unwrap();
        let addr = EndpointAddr::new(SecretKey::generate().public())
            .with_relay_url(relay)
            .with_addrs(ips(MAX_IP_ADDRS));
        assert!(addr_fits(&addr));
        assert_eq!(fit_addr(&addr), addr);
    }

    #[test]
    fn oversize_addresses_are_trimmed() {
        let id = SecretKey::generate().public();
        let many_ips = EndpointAddr::new(id).with_addrs(ips(MAX_IP_ADDRS + 1));
        assert!(!addr_fits(&many_ips));
        assert_eq!(fit_addr(&many_ips).ip_addrs().count(), MAX_IP_ADDRS);

        let long: RelayUrl = format!("https://{}.example.com", "a".repeat(MAX_RELAY_URL_LEN))
            .parse()
            .unwrap();
        let short: RelayUrl = "https://relay.example.com".parse().unwrap();
        assert!(!addr_fits(
            &EndpointAddr::new(id).with_relay_url(long.clone())
        ));
        let two_relays = EndpointAddr::new(id)
            .with_relay_url(long)
            .with_relay_url(short.clone());
        assert!(!addr_fits(&two_relays));
        assert_eq!(
            fit_addr(&two_relays),
            EndpointAddr::new(id).with_relay_url(short)
        );
    }
}
