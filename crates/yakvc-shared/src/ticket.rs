//! Tickets: issuer-signed statements binding an EndpointId to a Minecraft UUID.
//!
//! A ticket travels as a [`SignedTicket`]: the postcard-encoded [`TicketBody`]
//! bytes plus an ed25519 signature over `b"yakvc-ticket-v1" ‖ body`. Verifiers
//! check the signature over the bytes as received and only then decode them,
//! so a ticket is never re-encoded. The only way to obtain a [`Ticket`] is
//! through [`TicketVerifier::verify`], so holding one means it was checked.

use std::fmt;
use std::ops::Deref;
use std::str::FromStr;
use std::time::{Duration, SystemTime};

use iroh_base::{EndpointId, PublicKey, SecretKey, Signature};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::wire::WireError;

/// Domain tag prepended to the body bytes before signing.
pub const SIGNING_CONTEXT: &[u8] = b"yakvc-ticket-v1";

/// The ed25519 key a rendezvous uses to sign tickets. Distinct from its
/// endpoint key so several rendezvous instances can share one issuer.
#[derive(Clone)]
pub struct IssuerKey(SecretKey);

/// Public half of an [`IssuerKey`]. Clients trust a list of these.
///
/// Displays and parses in the same z-base-32 form as an `EndpointId`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct IssuerId(PublicKey);

/// What a ticket asserts. Times are Unix seconds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TicketBody {
    pub uuid: Uuid,
    pub name: String,
    pub endpoint_id: EndpointId,
    pub issued_at: u64,
    pub expires_at: u64,
    /// Issued by a rendezvous running `--insecure-dev-auth`; rejected unless
    /// the verifier accepts dev tickets.
    pub dev: bool,
}

/// A ticket as sent on the wire and cached on disk. Unverified.
#[derive(Clone, Serialize, Deserialize)]
pub struct SignedTicket {
    issuer: IssuerId,
    body: Vec<u8>,
    sig: Signature,
}

/// A ticket whose signature, issuer, expiry and dev flag have been checked.
/// Dereferences to its [`TicketBody`].
#[derive(Debug, Clone)]
pub struct Ticket {
    body: TicketBody,
    signed: SignedTicket,
}

/// Checks [`SignedTicket`]s against a set of trusted issuers.
#[derive(Debug, Clone)]
pub struct TicketVerifier {
    trusted: Vec<IssuerId>,
    accept_dev: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum TicketError {
    #[error("ticket issuer is not trusted")]
    UntrustedIssuer,
    #[error("ticket signature is invalid")]
    BadSignature,
    #[error("ticket body is malformed")]
    Malformed(#[source] postcard::Error),
    #[error("ticket has expired")]
    Expired,
    #[error("dev tickets are not accepted")]
    DevTicket,
}

impl IssuerKey {
    pub fn generate() -> Self {
        IssuerKey(SecretKey::generate())
    }

    pub fn from_bytes(bytes: &[u8; 32]) -> Self {
        IssuerKey(SecretKey::from_bytes(bytes))
    }

    pub fn to_bytes(&self) -> [u8; 32] {
        self.0.to_bytes()
    }

    pub fn id(&self) -> IssuerId {
        IssuerId(self.0.public())
    }

    /// Encodes and signs `body`.
    pub fn sign(&self, body: &TicketBody) -> SignedTicket {
        let _ = body;
        todo!()
    }
}

impl fmt::Debug for IssuerKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("IssuerKey").field(&self.id()).finish()
    }
}

impl fmt::Display for IssuerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for IssuerId {
    type Err = iroh_base::KeyParsingError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        PublicKey::from_str(s).map(IssuerId)
    }
}

impl TicketBody {
    /// A body issued at `now` that expires after `lifetime`.
    pub fn new(
        uuid: Uuid,
        name: String,
        endpoint_id: EndpointId,
        now: SystemTime,
        lifetime: Duration,
    ) -> Self {
        let _ = (uuid, name, endpoint_id, now, lifetime);
        todo!()
    }
}

impl SignedTicket {
    pub fn issuer(&self) -> IssuerId {
        self.issuer
    }

    /// Encodes for the on-disk ticket cache.
    pub fn to_bytes(&self) -> Vec<u8> {
        todo!()
    }

    /// Decodes from the on-disk ticket cache. Does not verify.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, WireError> {
        let _ = bytes;
        todo!()
    }
}

impl fmt::Debug for SignedTicket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SignedTicket")
            .field("issuer", &self.issuer)
            .field("body_len", &self.body.len())
            .field("sig", &self.sig)
            .finish()
    }
}

impl Ticket {
    /// The ticket exactly as received, for forwarding to peers or caching.
    pub fn signed(&self) -> &SignedTicket {
        &self.signed
    }

    /// Time left before expiry, or zero if already expired.
    pub fn remaining(&self, now: SystemTime) -> Duration {
        let _ = now;
        todo!()
    }
}

impl Deref for Ticket {
    type Target = TicketBody;

    fn deref(&self) -> &TicketBody {
        &self.body
    }
}

impl TicketVerifier {
    /// Trusts tickets signed by any of `issuers`. Dev tickets are rejected
    /// unless [`accept_dev`](Self::accept_dev) is set.
    pub fn new(issuers: impl IntoIterator<Item = IssuerId>) -> Self {
        TicketVerifier {
            trusted: issuers.into_iter().collect(),
            accept_dev: false,
        }
    }

    pub fn accept_dev(mut self, accept: bool) -> Self {
        self.accept_dev = accept;
        self
    }

    /// Checks issuer, signature, expiry and the dev flag, in that order.
    pub fn verify(&self, ticket: &SignedTicket, now: SystemTime) -> Result<Ticket, TicketError> {
        let _ = (ticket, now, &self.trusted, self.accept_dev);
        todo!()
    }
}
