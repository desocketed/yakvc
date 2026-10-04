//! Ticket issuing and the Mojang side of the auth challenge.

use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use yakvc_shared::auth::{Nonce, session_server_id};
use yakvc_shared::rdv::Hello;
use yakvc_shared::{EndpointId, IssuerKey, SignedTicket, TicketBody, TicketVerifier, Uuid};

use crate::mojang::{MojangError, Profile, SessionServer};

/// A cached ticket with less time left than this is replaced with a fresh one.
pub(crate) const RENEW_BEFORE: Duration = Duration::from_secs(2 * 3600);

/// How long to stop calling Mojang after a 429 without `Retry-After`, and how
/// long clients are told to wait when Mojang is down.
const DEFAULT_RETRY_AFTER: Duration = Duration::from_secs(10);

#[derive(Debug)]
pub(crate) struct Auth {
    issuer: IssuerKey,
    verifier: TicketVerifier,
    lifetime: Duration,
    /// `None` runs `--insecure-dev-auth`: claimed UUIDs are taken on trust.
    mojang: Option<SessionServer>,
    rendezvous: EndpointId,
    /// Set after Mojang answered 429; no calls until then.
    mojang_blocked_until: Mutex<Option<Instant>>,
}

/// Result of asking Mojang about a challenge.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum JoinCheck {
    Confirmed(Profile),
    Rejected,
    /// Mojang is rate-limiting us or down; the client should retry later.
    RetryAfter(u32),
}

impl Auth {
    pub fn new(
        issuer: IssuerKey,
        lifetime: Duration,
        mojang: Option<SessionServer>,
        rendezvous: EndpointId,
    ) -> Self {
        Auth {
            verifier: TicketVerifier::new([issuer.id()]).accept_dev(mojang.is_none()),
            issuer,
            lifetime,
            mojang,
            rendezvous,
            mojang_blocked_until: Mutex::new(None),
        }
    }

    pub fn is_dev(&self) -> bool {
        self.mojang.is_none()
    }

    /// Whether `cached` can register this client without a challenge: one of
    /// our tickets for this EndpointId, UUID and name, with more than
    /// [`RENEW_BEFORE`] left. While Mojang is rate-limiting us, any time left
    /// will do, since a fresh ticket can't be issued anyway.
    pub fn can_reuse(&self, cached: &SignedTicket, hello: &Hello, client: EndpointId) -> bool {
        let now = SystemTime::now();
        let Ok(ticket) = self.verifier.verify(cached, now) else {
            return false;
        };
        let fresh_enough = ticket.remaining(now) > RENEW_BEFORE || self.retry_after().is_some();
        ticket.endpoint_id == client
            && ticket.uuid == hello.uuid
            && ticket.name == hello.name
            && fresh_enough
    }

    pub fn issue(&self, uuid: Uuid, name: String, client: EndpointId) -> SignedTicket {
        let mut body = TicketBody::new(uuid, name, client, SystemTime::now(), self.lifetime);
        body.dev = self.is_dev();
        self.issuer.sign(&body)
    }

    /// Seconds until Mojang may be called again, if it is rate-limiting us.
    pub fn retry_after(&self) -> Option<u32> {
        let blocked_until = (*self.mojang_blocked_until.lock().unwrap())?;
        let left = blocked_until.checked_duration_since(Instant::now())?;
        Some(secs_rounded_up(left))
    }

    /// Asks Mojang whether `name` called `joinServer` for this challenge.
    pub async fn check_join(&self, name: &str, nonce: &Nonce, client: EndpointId) -> JoinCheck {
        let mojang = self.mojang.as_ref().expect("dev auth has no challenge");
        if let Some(secs) = self.retry_after() {
            return JoinCheck::RetryAfter(secs);
        }
        let server_id = session_server_id(nonce, client, self.rendezvous);
        match mojang.has_joined(name, &server_id).await {
            Ok(Some(profile)) => JoinCheck::Confirmed(profile),
            Ok(None) => JoinCheck::Rejected,
            Err(MojangError::RateLimited { retry_after }) => {
                let wait = retry_after.unwrap_or(DEFAULT_RETRY_AFTER);
                *self.mojang_blocked_until.lock().unwrap() = Some(Instant::now() + wait);
                JoinCheck::RetryAfter(secs_rounded_up(wait))
            }
            Err(MojangError::Unavailable(_)) => {
                JoinCheck::RetryAfter(secs_rounded_up(DEFAULT_RETRY_AFTER))
            }
        }
    }
}

fn secs_rounded_up(d: Duration) -> u32 {
    let secs = d.as_secs() + u64::from(d.subsec_nanos() > 0);
    u32::try_from(secs).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use yakvc_shared::{EndpointAddr, SecretKey};

    use super::*;
    use crate::mojang::fake::{FakeMojang, response, uuid_for};

    const DAY: Duration = Duration::from_secs(24 * 3600);

    fn client() -> EndpointId {
        SecretKey::from_bytes(&[1; 32]).public()
    }

    fn rendezvous() -> EndpointId {
        SecretKey::from_bytes(&[2; 32]).public()
    }

    fn hello(name: &str, cached_ticket: Option<SignedTicket>) -> Hello {
        Hello {
            mod_version: "test".into(),
            uuid: uuid_for(name),
            name: name.into(),
            addr: EndpointAddr::new(client()),
            cached_ticket,
        }
    }

    fn auth(mojang: Option<&FakeMojang>, lifetime: Duration) -> Auth {
        let mojang = mojang.map(|m| SessionServer::new(&m.url));
        Auth::new(
            IssuerKey::from_bytes(&[3; 32]),
            lifetime,
            mojang,
            rendezvous(),
        )
    }

    #[tokio::test]
    async fn reuses_only_matching_fresh_tickets() {
        let mojang = FakeMojang::start().await;
        let auth = auth(Some(&mojang), DAY);
        let ticket = auth.issue(uuid_for("alice"), "alice".into(), client());

        assert!(auth.can_reuse(&ticket, &hello("alice", None), client()));
        // Different name (and so UUID), or different EndpointId.
        assert!(!auth.can_reuse(&ticket, &hello("bob", None), client()));
        assert!(!auth.can_reuse(&ticket, &hello("alice", None), rendezvous()));
        // Another issuer's ticket.
        let foreign = IssuerKey::generate().sign(&TicketBody::new(
            uuid_for("alice"),
            "alice".into(),
            client(),
            SystemTime::now(),
            DAY,
        ));
        assert!(!auth.can_reuse(&foreign, &hello("alice", None), client()));
    }

    #[tokio::test]
    async fn tickets_close_to_expiry_are_reused_only_while_mojang_is_blocked() {
        let mojang = FakeMojang::start().await;
        let auth = auth(Some(&mojang), Duration::from_secs(3600));
        let ticket = auth.issue(uuid_for("alice"), "alice".into(), client());
        assert!(!auth.can_reuse(&ticket, &hello("alice", None), client()));

        mojang.script([response("429 Too Many Requests", "Retry-After: 30\r\n", "")]);
        auth.check_join("alice", &Nonce([0; 32]), client()).await;
        assert!(auth.can_reuse(&ticket, &hello("alice", None), client()));
    }

    #[tokio::test]
    async fn production_rejects_dev_tickets_and_issues_real_ones() {
        let mojang = FakeMojang::start().await;
        let prod = auth(Some(&mojang), DAY);
        let dev = Auth::new(IssuerKey::from_bytes(&[3; 32]), DAY, None, rendezvous());
        let dev_ticket = dev.issue(uuid_for("alice"), "alice".into(), client());
        assert!(!prod.can_reuse(&dev_ticket, &hello("alice", None), client()));
        assert!(dev.can_reuse(&dev_ticket, &hello("alice", None), client()));

        let verifier = TicketVerifier::new([IssuerKey::from_bytes(&[3; 32]).id()]);
        let real = prod.issue(uuid_for("alice"), "alice".into(), client());
        assert!(!verifier.verify(&real, SystemTime::now()).unwrap().dev);
        assert!(verifier.verify(&dev_ticket, SystemTime::now()).is_err());
    }

    #[tokio::test]
    async fn check_join_asks_mojang_with_the_challenge_digest() {
        let mojang = FakeMojang::start().await;
        let auth = auth(Some(&mojang), DAY);
        let nonce = Nonce([5; 32]);
        let check = auth.check_join("alice", &nonce, client()).await;
        assert_eq!(
            check,
            JoinCheck::Confirmed(Profile {
                uuid: uuid_for("alice"),
                name: "alice".into()
            })
        );
        let server_id = session_server_id(&nonce, client(), rendezvous());
        assert_eq!(
            mojang.requests(),
            [format!(
                "/session/minecraft/hasJoined?username=alice&serverId={server_id}"
            )]
        );

        mojang.script([response("204 No Content", "", "")]);
        assert_eq!(
            auth.check_join("alice", &nonce, client()).await,
            JoinCheck::Rejected
        );
    }

    #[tokio::test]
    async fn rate_limit_stops_mojang_calls_until_retry_after() {
        let mojang = FakeMojang::start().await;
        let auth = auth(Some(&mojang), DAY);
        mojang.script([response("429 Too Many Requests", "Retry-After: 30\r\n", "")]);
        let nonce = Nonce([0; 32]);

        assert_eq!(
            auth.check_join("alice", &nonce, client()).await,
            JoinCheck::RetryAfter(30)
        );
        // Later auths are answered without calling Mojang.
        assert!(matches!(
            auth.check_join("bob", &nonce, client()).await,
            JoinCheck::RetryAfter(29 | 30)
        ));
        assert_eq!(mojang.requests().len(), 1);
        assert!(auth.retry_after().is_some());

        // Once the time has passed, Mojang is asked again.
        *auth.mojang_blocked_until.lock().unwrap() = Some(Instant::now());
        assert_eq!(auth.retry_after(), None);
        assert!(matches!(
            auth.check_join("bob", &nonce, client()).await,
            JoinCheck::Confirmed(_)
        ));
    }

    #[tokio::test]
    async fn rate_limit_without_retry_after_waits_ten_seconds() {
        let mojang = FakeMojang::start().await;
        let auth = auth(Some(&mojang), DAY);
        mojang.script([response("429 Too Many Requests", "", "")]);
        assert_eq!(
            auth.check_join("alice", &Nonce([0; 32]), client()).await,
            JoinCheck::RetryAfter(10)
        );
    }

    #[tokio::test]
    async fn mojang_outage_asks_the_client_to_retry_without_blocking() {
        let mojang = FakeMojang::start().await;
        let auth = auth(Some(&mojang), DAY);
        let down = response("503 Service Unavailable", "", "");
        mojang.script([down.clone(), down]);
        assert_eq!(
            auth.check_join("alice", &Nonce([0; 32]), client()).await,
            JoinCheck::RetryAfter(10)
        );
        assert_eq!(auth.retry_after(), None);
    }

    #[test]
    fn rounding() {
        assert_eq!(secs_rounded_up(Duration::from_millis(1)), 1);
        assert_eq!(secs_rounded_up(Duration::from_secs(10)), 10);
        assert_eq!(secs_rounded_up(Duration::from_millis(10_001)), 11);
    }
}
