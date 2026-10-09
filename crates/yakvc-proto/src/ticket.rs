//! Tickets: issuer-signed statements binding an EndpointId to a Minecraft UUID.
//!
//! A ticket travels as a [`SignedTicket`]: the postcard-encoded [`TicketBody`]
//! bytes plus an ed25519 signature over `b"yakvc-ticket-v2" ‖ body`. Verifiers
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

use crate::auth::is_offline_player;
use crate::wire::WireError;

/// Domain tag prepended to the body bytes before signing. v2 added
/// [`TicketBody::verified`]; the new tag makes tickets cached by older builds
/// fail their signature check instead of being misread.
pub const SIGNING_CONTEXT: &[u8] = b"yakvc-ticket-v2";

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
    /// The holder proved the account to Mojang. Without that proof a ticket
    /// is only valid for the offline UUID of its name (see [`is_offline_player`]).
    pub verified: bool,
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
    verified_only: bool,
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
    #[error("unverified ticket for an account UUID or another name")]
    UnverifiedAccount,
    #[error("only verified players are accepted")]
    Unverified,
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
        let body = postcard::to_stdvec(body).expect("a TicketBody always encodes");
        let sig = self.0.sign(&signed_message(&body));
        SignedTicket {
            issuer: self.id(),
            body,
            sig,
        }
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
        verified: bool,
        now: SystemTime,
        lifetime: Duration,
    ) -> Self {
        let issued_at = unix_secs(now);
        TicketBody {
            uuid,
            name,
            endpoint_id,
            issued_at,
            expires_at: issued_at.saturating_add(lifetime.as_secs()),
            verified,
            dev: false,
        }
    }
}

impl SignedTicket {
    pub fn issuer(&self) -> IssuerId {
        self.issuer
    }

    /// Encodes for the on-disk ticket cache.
    pub fn to_bytes(&self) -> Vec<u8> {
        postcard::to_stdvec(self).expect("a SignedTicket always encodes")
    }

    /// Decodes from the on-disk ticket cache. Does not verify.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, WireError> {
        Ok(postcard::from_bytes(bytes)?)
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
        // Exact rather than in whole seconds: rounding `now` down would
        // promise up to a second more than is left, and a renewal scheduled
        // in that second would come too late.
        let expires = SystemTime::UNIX_EPOCH + Duration::from_secs(self.body.expires_at);
        expires.duration_since(now).unwrap_or_default()
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
            verified_only: false,
        }
    }

    pub fn accept_dev(mut self, accept: bool) -> Self {
        self.accept_dev = accept;
        self
    }

    /// Rejects every unverified ticket, for players who only want to talk
    /// with Mojang-proven accounts.
    pub fn verified_only(mut self, verified_only: bool) -> Self {
        self.verified_only = verified_only;
        self
    }

    /// Checks issuer, signature, expiry, the dev flag and the verified rules,
    /// in that order.
    ///
    /// An unverified ticket must name an offline UUID: offline-mode servers
    /// give every player one, so there it is no weaker than the server's own
    /// identity, while online-mode servers only show account UUIDs, which an
    /// unverified ticket can then never match.
    pub fn verify(&self, ticket: &SignedTicket, now: SystemTime) -> Result<Ticket, TicketError> {
        if !self.trusted.contains(&ticket.issuer) {
            return Err(TicketError::UntrustedIssuer);
        }
        ticket
            .issuer
            .0
            .verify(&signed_message(&ticket.body), &ticket.sig)
            .map_err(|_| TicketError::BadSignature)?;
        let body: TicketBody =
            postcard::from_bytes(&ticket.body).map_err(TicketError::Malformed)?;
        if body.expires_at <= unix_secs(now) {
            return Err(TicketError::Expired);
        }
        if body.dev && !self.accept_dev {
            return Err(TicketError::DevTicket);
        }
        if !body.verified && !is_offline_player(body.uuid, &body.name) {
            return Err(TicketError::UnverifiedAccount);
        }
        if !body.verified && self.verified_only {
            return Err(TicketError::Unverified);
        }
        Ok(Ticket {
            body,
            signed: ticket.clone(),
        })
    }
}

/// What the issuer signs: the domain tag, then the body bytes.
fn signed_message(body: &[u8]) -> Vec<u8> {
    [SIGNING_CONTEXT, body].concat()
}

/// Times before 1970 count as 0; they only occur with a broken clock.
fn unix_secs(time: SystemTime) -> u64 {
    time.duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::offline_uuid;

    const HOUR: Duration = Duration::from_secs(3600);

    fn now() -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000)
    }

    fn body() -> TicketBody {
        let endpoint_id = SecretKey::from_bytes(&[3; 32]).public();
        TicketBody::new(
            Uuid::from_u128(42),
            "alice".into(),
            endpoint_id,
            true,
            now(),
            24 * HOUR,
        )
    }

    /// An unverified ticket for alice's offline UUID.
    fn offline_body() -> TicketBody {
        TicketBody {
            uuid: offline_uuid("alice"),
            verified: false,
            ..body()
        }
    }

    fn issuer() -> IssuerKey {
        IssuerKey::from_bytes(&[9; 32])
    }

    #[test]
    fn body_times() {
        let body = body();
        assert_eq!(body.issued_at, 1_800_000_000);
        assert_eq!(body.expires_at, 1_800_000_000 + 24 * 3600);
        assert!(!body.dev);
    }

    #[test]
    fn signed_ticket_verifies() {
        let key = issuer();
        let signed = key.sign(&body());
        assert_eq!(signed.issuer(), key.id());
        let ticket = TicketVerifier::new([key.id()])
            .verify(&signed, now())
            .unwrap();
        assert_eq!(*ticket, body());
        assert_eq!(ticket.name, "alice");
        assert_eq!(ticket.signed().to_bytes(), signed.to_bytes());
        assert_eq!(ticket.remaining(now()), 24 * HOUR);
        let half_second = Duration::from_millis(500);
        assert_eq!(
            ticket.remaining(now() + half_second),
            24 * HOUR - half_second
        );
        assert_eq!(ticket.remaining(now() + 30 * HOUR), Duration::ZERO);
    }

    #[test]
    fn untrusted_issuer() {
        let signed = IssuerKey::generate().sign(&body());
        let err = TicketVerifier::new([issuer().id()]).verify(&signed, now());
        assert!(matches!(err, Err(TicketError::UntrustedIssuer)));
    }

    #[test]
    fn tampered_body_or_signature() {
        let key = issuer();
        let verifier = TicketVerifier::new([key.id()]);

        let mut signed = key.sign(&body());
        let last = signed.body.len() - 1;
        signed.body[last] ^= 1;
        assert!(matches!(
            verifier.verify(&signed, now()),
            Err(TicketError::BadSignature)
        ));

        let mut signed = key.sign(&body());
        let mut sig = signed.sig.to_bytes();
        sig[0] ^= 1;
        signed.sig = Signature::from_bytes(&sig);
        assert!(matches!(
            verifier.verify(&signed, now()),
            Err(TicketError::BadSignature)
        ));
    }

    #[test]
    fn signature_covers_the_domain_tag() {
        let key = issuer();
        let body = postcard::to_stdvec(&body()).unwrap();
        let signed = SignedTicket {
            issuer: key.id(),
            sig: key.0.sign(&body),
            body,
        };
        let err = TicketVerifier::new([key.id()]).verify(&signed, now());
        assert!(matches!(err, Err(TicketError::BadSignature)));
    }

    #[test]
    fn malformed_body() {
        let key = issuer();
        let body = vec![0xff; 4];
        let signed = SignedTicket {
            issuer: key.id(),
            sig: key.0.sign(&signed_message(&body)),
            body,
        };
        let err = TicketVerifier::new([key.id()]).verify(&signed, now());
        assert!(matches!(err, Err(TicketError::Malformed(_))));
    }

    #[test]
    fn expired() {
        let key = issuer();
        let signed = key.sign(&body());
        let verifier = TicketVerifier::new([key.id()]);
        let just_before = now() + 24 * HOUR - Duration::from_secs(1);
        assert!(verifier.verify(&signed, just_before).is_ok());
        assert!(matches!(
            verifier.verify(&signed, now() + 24 * HOUR),
            Err(TicketError::Expired)
        ));
    }

    #[test]
    fn dev_tickets_need_opt_in() {
        let key = issuer();
        let signed = key.sign(&TicketBody {
            dev: true,
            ..body()
        });
        let verifier = TicketVerifier::new([key.id()]);
        assert!(matches!(
            verifier.verify(&signed, now()),
            Err(TicketError::DevTicket)
        ));
        assert!(verifier.accept_dev(true).verify(&signed, now()).is_ok());
    }

    #[test]
    fn unverified_tickets_are_only_valid_for_offline_uuids() {
        let key = issuer();
        let verifier = TicketVerifier::new([key.id()]);
        assert!(verifier.verify(&key.sign(&offline_body()), now()).is_ok());

        // An account UUID (version 4) needs a Mojang proof.
        let account = TicketBody {
            uuid: Uuid::from_u128(0x0123_4567_89ab_4def_8123_4567_89ab_cdef),
            ..offline_body()
        };
        assert!(matches!(
            verifier.verify(&key.sign(&account), now()),
            Err(TicketError::UnverifiedAccount)
        ));
        // Alice's offline UUID under another name.
        let renamed = TicketBody {
            name: "mallory".into(),
            ..offline_body()
        };
        assert!(matches!(
            verifier.verify(&key.sign(&renamed), now()),
            Err(TicketError::UnverifiedAccount)
        ));
        // A verified ticket may name any UUID, an offline one included.
        let verified = TicketBody {
            verified: true,
            ..offline_body()
        };
        assert!(verifier.verify(&key.sign(&verified), now()).is_ok());
    }

    #[test]
    fn verified_only_rejects_every_unverified_ticket() {
        let key = issuer();
        let verifier = TicketVerifier::new([key.id()]).verified_only(true);
        assert!(matches!(
            verifier.verify(&key.sign(&offline_body()), now()),
            Err(TicketError::Unverified)
        ));
        assert!(verifier.verify(&key.sign(&body()), now()).is_ok());
    }

    #[test]
    fn cache_bytes_round_trip() {
        let key = issuer();
        let signed = key.sign(&body());
        let decoded = SignedTicket::from_bytes(&signed.to_bytes()).unwrap();
        assert_eq!(decoded.to_bytes(), signed.to_bytes());
        assert!(
            TicketVerifier::new([key.id()])
                .verify(&decoded, now())
                .is_ok()
        );
        assert!(SignedTicket::from_bytes(&[1, 2, 3]).is_err());
    }

    #[test]
    fn issuer_id_text_round_trips() {
        let id = issuer().id();
        assert_eq!(id.to_string().parse::<IssuerId>().unwrap(), id);
        assert_eq!(IssuerKey::from_bytes(&issuer().to_bytes()).id(), id);
    }
}
