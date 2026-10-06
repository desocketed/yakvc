//! The client side of `yakvc/rdv/1`: authenticate, keep the ticket fresh,
//! send pair tokens for the tab list, and turn matches into peer
//! connections. Reconnects with backoff for as long as the engine runs.

use std::collections::HashSet;
use std::convert::Infallible;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use iroh::endpoint::{Connection, SendStream};
use iroh::{EndpointAddr, TransportAddr, Watcher};
use rand::RngExt;
use tokio::sync::{mpsc, oneshot};
use tokio::time::Instant;
use yakvc_shared::auth::session_server_id;
use yakvc_shared::rdv::{ALPN, ClientMsg, CloseCode, Hello, MAX_PAIRS, ServerMsg};
use yakvc_shared::{PairToken, SignedTicket, Ticket, Uuid, is_offline_uuid, wire};

use super::{Identity, Inner};
use crate::config::RendezvousConfig;
use crate::event::{Event, RendezvousState};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// A ticket is renewed at a random point in this last stretch of its
/// lifetime, so renewals from many clients don't bunch up.
const RENEW_WINDOW: Duration = Duration::from_secs(2 * 60 * 60);
/// Tab-list changes are merged and sent at most once per interval. The game
/// can change the tab list every tick, and a batch is up to two messages
/// (`RemovePairs`, `AddPairs`), so this keeps us under the rendezvous's
/// [`PAIR_UPDATES_PER_SEC`](yakvc_shared::rdv::PAIR_UPDATES_PER_SEC) with
/// room left for `UpdateAddr`.
const PAIR_BATCH_INTERVAL: Duration = Duration::from_millis(250);

/// Why a session ended.
#[derive(Debug)]
enum Ended {
    /// The game changed the local identity; start over with the new one.
    IdentityChanged,
    /// Mojang is rate-limiting the rendezvous, or the rendezvous is
    /// rate-limiting us.
    RetryAfter(Duration),
    /// `joinServer` failed, the rendezvous refused our proof or sent a ticket
    /// we can't use, or our ticket expired without a renewal.
    AuthFailed(String),
    /// Network trouble or a protocol error.
    Failed(String),
}

impl From<wire::WireError> for Ended {
    fn from(err: wire::WireError) -> Self {
        Ended::Failed(err.to_string())
    }
}

pub(super) async fn run(inner: Arc<Inner>, config: RendezvousConfig, ticket_cache: PathBuf) {
    // Network trouble retries quickly; rate limits and auth failures back off
    // as DESIGN.md "Limits" asks: 10 s doubling to 10 min.
    let mut network_backoff = Backoff::new(Duration::from_secs(1), Duration::from_secs(60));
    let mut auth_backoff = Backoff::new(Duration::from_secs(10), Duration::from_secs(600));
    let mut identity = inner.identity.subscribe();
    loop {
        let current = match identity.wait_for(Option::is_some).await {
            Ok(current) => current.clone().expect("waited for an identity"),
            Err(_) => return,
        };
        identity.mark_unchanged();
        report(&inner, RendezvousState::Connecting);

        let mut session = Session {
            inner: &inner,
            config: &config,
            ticket_cache: &ticket_cache,
            identity: current,
            registered: false,
        };
        let ended = match session.run().await {
            Err(ended) => ended,
            Ok(never) => match never {},
        };
        if session.registered {
            network_backoff.reset();
            // A registered session closed for exceeding a limit keeps
            // doubling its backoff, or it would retry every 10 s forever.
            if !matches!(ended, Ended::RetryAfter(_)) {
                auth_backoff.reset();
            }
        }
        let delay = match ended {
            Ended::IdentityChanged => continue,
            Ended::RetryAfter(asked) => {
                let delay = auth_backoff.next().max(asked);
                report(&inner, RendezvousState::RetryingIn(delay));
                delay
            }
            Ended::AuthFailed(reason) => {
                inner
                    .events
                    .send(Event::Error(format!("voice chat sign-in failed: {reason}")));
                let delay = auth_backoff.next();
                report(&inner, RendezvousState::RetryingIn(delay));
                delay
            }
            Ended::Failed(reason) => {
                tracing::debug!(%reason, "rendezvous session ended");
                report(&inner, RendezvousState::Disconnected);
                network_backoff.next()
            }
        };
        tokio::time::sleep(delay).await;
    }
}

fn report(inner: &Inner, state: RendezvousState) {
    inner.events.send(Event::Rendezvous(state));
}

/// One connection to the rendezvous.
struct Session<'a> {
    inner: &'a Arc<Inner>,
    config: &'a RendezvousConfig,
    ticket_cache: &'a Path,
    identity: Identity,
    /// Got a ticket on this connection.
    registered: bool,
}

impl Session<'_> {
    async fn run(&mut self) -> Result<Infallible, Ended> {
        let conn = self.connect().await?;
        let result = self.serve(&conn).await;
        conn.close(rdv_close(CloseCode::Normal), b"");
        result
    }

    async fn connect(&self) -> Result<Connection, Ended> {
        let addrs = self.config.addrs.iter().copied().map(TransportAddr::Ip);
        let relay = self.config.relay.clone().map(TransportAddr::Relay);
        let addr = EndpointAddr::from_parts(self.config.endpoint_id, addrs.chain(relay));
        let connect = self.inner.endpoint.connect(addr, ALPN);
        match tokio::time::timeout(CONNECT_TIMEOUT, connect).await {
            Ok(Ok(conn)) => Ok(conn),
            Ok(Err(err)) => Err(Ended::Failed(err.to_string())),
            Err(_) => Err(Ended::Failed("timed out connecting".into())),
        }
    }

    async fn serve(&mut self, conn: &Connection) -> Result<Infallible, Ended> {
        let (mut send, mut recv) = conn
            .open_bi()
            .await
            .map_err(|err| Ended::Failed(err.to_string()))?;
        let hello = Hello {
            mod_version: env!("CARGO_PKG_VERSION").to_owned(),
            uuid: self.identity.uuid,
            name: self.identity.name.clone(),
            // Trimmed, since the rendezvous rejects oversize addresses.
            addr: yakvc_shared::rdv::fit_addr(&self.inner.endpoint.addr()),
            cached_ticket: self.cached_ticket(),
        };
        wire::write_msg(&mut send, &ClientMsg::Hello(hello)).await?;
        report(self.inner, RendezvousState::Authenticating);

        // Reading a message is not cancel-safe, so it gets its own task.
        let (msg_tx, mut from_server) = mpsc::channel(16);
        let reader = tokio::spawn(async move {
            loop {
                let msg = wire::read_msg::<ServerMsg>(&mut recv).await;
                let end = !matches!(msg, Ok(Some(_)));
                if msg_tx.send(msg).await.is_err() || end {
                    break;
                }
            }
        });
        let result = self.event_loop(conn, &mut send, &mut from_server).await;
        reader.abort();
        result
    }

    async fn event_loop(
        &mut self,
        conn: &Connection,
        send: &mut SendStream,
        from_server: &mut mpsc::Receiver<Result<Option<ServerMsg>, wire::WireError>>,
    ) -> Result<Infallible, Ended> {
        let mut identity = self.inner.identity.subscribe();
        identity.mark_unchanged();
        let mut tab_list = self.inner.tab_list.subscribe();
        let mut our_addr = self.inner.endpoint.watch_addr();
        // The pair tokens the rendezvous holds; `None` until registered.
        let mut sent_pairs: Option<HashSet<PairToken>> = None;
        // When to send the tab-list changes since the last batch, and the
        // earliest the next batch may go out.
        let mut pairs_due: Option<Instant> = None;
        let mut next_batch = Instant::now();
        let mut pending_join: Option<oneshot::Receiver<bool>> = None;
        // Our current ticket; `None` until registered.
        let mut ticket: Option<Ticket> = None;
        let mut renew_at: Option<Instant> = None;
        // A failed renewal keeps the current ticket and tries again later.
        let mut renew_backoff = Backoff::new(Duration::from_secs(10), Duration::from_secs(600));

        loop {
            tokio::select! {
                msg = from_server.recv() => {
                    let msg = match msg {
                        Some(Ok(Some(msg))) => msg,
                        Some(Err(err)) => return Err(err.into()),
                        Some(Ok(None)) | None => return Err(closed_reason(conn)),
                    };
                    match msg {
                        ServerMsg::Challenge(nonce) => {
                            let server_id = session_server_id(
                                &nonce,
                                self.inner.endpoint.id(),
                                self.config.endpoint_id,
                            );
                            let (id, answer) = self.inner.new_join();
                            self.inner.events.send(Event::JoinRequest { id, server_id });
                            pending_join = Some(answer);
                        }
                        ServerMsg::Registered(signed) => {
                            let accepted = self.accept_ticket(signed)?;
                            renew_at = Some(renewal_time(&accepted));
                            renew_backoff.reset();
                            ticket = Some(accepted);
                            if sent_pairs.is_none() {
                                let pairs = pair_tokens(self.identity.uuid, &tab_list.borrow_and_update());
                                let msg = ClientMsg::SetPairs(pairs.iter().copied().collect());
                                wire::write_msg(send, &msg).await?;
                                sent_pairs = Some(pairs);
                                next_batch = Instant::now() + PAIR_BATCH_INTERVAL;
                            }
                        }
                        ServerMsg::RetryAfter { secs } => {
                            let delay = Duration::from_secs(secs.into());
                            let Some(current) = &ticket else {
                                // The rendezvous closes the connection next.
                                return Err(Ended::RetryAfter(delay));
                            };
                            // A refused renewal: the session and the current
                            // ticket stay valid, so just renew again later.
                            let delay = renew_backoff.next().max(delay);
                            renew_at = Some(retry_renewal(current, delay));
                        }
                        ServerMsg::PeerAvailable { ticket, addr } => {
                            self.inner.peer_available(ticket, addr);
                        }
                        ServerMsg::PeerGone(remote) => self.inner.peer_gone(remote),
                    }
                }
                ok = wait_for_join(&mut pending_join), if pending_join.is_some() => {
                    pending_join = None;
                    let offline = is_offline_uuid(self.identity.uuid);
                    if ok {
                        wire::write_msg(send, &ClientMsg::Joined).await?;
                    } else if let Some(current) = &ticket {
                        // A failed renewal keeps the session and its ticket.
                        // An offline UUID settles for an unverified ticket
                        // once the current one would run out (or already is
                        // unverified), rather than losing voice.
                        let retry = renew_backoff.next();
                        let running_out = current.remaining(SystemTime::now()) <= retry;
                        if offline && (!current.verified || running_out) {
                            wire::write_msg(send, &ClientMsg::Decline).await?;
                        } else {
                            // The challenge stays unanswered; `Renew`
                            // replaces it.
                            renew_at = Some(retry_renewal(current, retry));
                        }
                    } else if offline {
                        // No account proof, but an offline-mode server's
                        // UUID can still get an unverified ticket.
                        wire::write_msg(send, &ClientMsg::Decline).await?;
                    } else {
                        return Err(Ended::AuthFailed("Minecraft session check failed".into()));
                    }
                }
                Ok(()) = tab_list.changed(), if sent_pairs.is_some() => {
                    // Later changes join this batch: it diffs against
                    // whatever the tab list is when it goes out.
                    pairs_due.get_or_insert(next_batch.max(Instant::now()));
                }
                () = sleep_until(pairs_due), if pairs_due.is_some() => {
                    pairs_due = None;
                    next_batch = Instant::now() + PAIR_BATCH_INTERVAL;
                    let pairs = pair_tokens(self.identity.uuid, &tab_list.borrow_and_update());
                    let sent = sent_pairs.as_mut().expect("batches start after registration");
                    let added: Vec<PairToken> = pairs.difference(sent).copied().collect();
                    let removed: Vec<PairToken> = sent.difference(&pairs).copied().collect();
                    if !removed.is_empty() {
                        wire::write_msg(send, &ClientMsg::RemovePairs(removed)).await?;
                    }
                    if !added.is_empty() {
                        wire::write_msg(send, &ClientMsg::AddPairs(added)).await?;
                    }
                    *sent = pairs;
                }
                () = sleep_until(renew_at), if renew_at.is_some() => {
                    renew_at = None;
                    if ticket.as_ref().is_some_and(|t| t.remaining(SystemTime::now()).is_zero()) {
                        return Err(Ended::AuthFailed("could not renew the voice chat ticket".into()));
                    }
                    wire::write_msg(send, &ClientMsg::Renew).await?;
                }
                // Before registration the rendezvous only accepts the
                // challenge reply; the Hello already carried our address.
                Ok(addr) = our_addr.updated(), if self.registered => {
                    let addr = yakvc_shared::rdv::fit_addr(&addr);
                    wire::write_msg(send, &ClientMsg::UpdateAddr(addr)).await?;
                }
                Ok(()) = identity.changed() => return Err(Ended::IdentityChanged),
            }
        }
    }

    /// Checks a ticket from the rendezvous, then caches and adopts it. A
    /// ticket we can't use (an untrusted issuer, a clock far off) is an auth
    /// failure: retrying soon would only fetch another one.
    fn accept_ticket(&mut self, signed: SignedTicket) -> Result<Ticket, Ended> {
        let ticket = self.verify_own(&signed).map_err(Ended::AuthFailed)?;
        // Losing the cache only costs a fresh Mojang check next start.
        let _ = std::fs::write(self.ticket_cache, signed.to_bytes());
        self.inner.own_ticket.send_replace(Some(signed));
        self.registered = true;
        let expires_at = SystemTime::UNIX_EPOCH + Duration::from_secs(ticket.expires_at);
        report(
            self.inner,
            RendezvousState::Registered {
                expires_at,
                verified: ticket.verified,
            },
        );
        Ok(ticket)
    }

    /// Our cached ticket, if it still names us and has enough time left for
    /// the rendezvous to accept it.
    fn cached_ticket(&self) -> Option<SignedTicket> {
        let bytes = std::fs::read(self.ticket_cache).ok()?;
        let signed = SignedTicket::from_bytes(&bytes).ok()?;
        let ticket = self.verify_own(&signed).ok()?;
        (ticket.remaining(SystemTime::now()) > RENEW_WINDOW).then_some(signed)
    }

    fn verify_own(&self, signed: &SignedTicket) -> Result<Ticket, String> {
        let verifier = self.inner.trust.lock().unwrap().verifier.clone();
        let ticket = verifier
            .verify(signed, SystemTime::now())
            .map_err(|err| format!("rendezvous sent a bad ticket: {err}"))?;
        if ticket.endpoint_id != self.inner.endpoint.id() || ticket.uuid != self.identity.uuid {
            return Err("rendezvous sent a ticket for someone else".into());
        }
        Ok(ticket)
    }
}

/// Tokens for every pair of us and a tab-list entry, at most [`MAX_PAIRS`].
fn pair_tokens(me: Uuid, tab_list: &HashSet<Uuid>) -> HashSet<PairToken> {
    let mut tokens: Vec<PairToken> = tab_list
        .iter()
        .filter(|&&other| other != me)
        .map(|&other| PairToken::new(me, other))
        .collect();
    if tokens.len() > MAX_PAIRS {
        tracing::warn!(
            pairs = tokens.len(),
            "tab list is over the rendezvous limit of {MAX_PAIRS}; ignoring the rest"
        );
        // Keeping the lowest tokens is stable as the list churns, and since
        // tokens are symmetric, two such clients tend to keep the same pairs.
        tokens.sort_unstable();
        tokens.truncate(MAX_PAIRS);
    }
    tokens.into_iter().collect()
}

/// A random point in the ticket's last [`RENEW_WINDOW`], leaving a little
/// slack before it actually expires.
fn renewal_time(ticket: &Ticket) -> Instant {
    let remaining = ticket.remaining(SystemTime::now());
    let window = RENEW_WINDOW.min(remaining).mul_f64(0.9);
    let window_start = remaining.saturating_sub(RENEW_WINDOW);
    let offset = window.mul_f64(rand::rng().random::<f64>());
    Instant::now() + window_start + offset
}

/// When to try a failed renewal again: after `delay`, but no later than the
/// current ticket's expiry, when the session ends if nothing worked.
fn retry_renewal(current: &Ticket, delay: Duration) -> Instant {
    Instant::now() + delay.min(current.remaining(SystemTime::now()))
}

async fn wait_for_join(pending: &mut Option<oneshot::Receiver<bool>>) -> bool {
    match pending {
        // A dropped sender means the engine is shutting down.
        Some(answer) => answer.await.unwrap_or(false),
        None => std::future::pending().await,
    }
}

async fn sleep_until(when: Option<Instant>) {
    match when {
        Some(when) => tokio::time::sleep_until(when).await,
        None => std::future::pending().await,
    }
}

fn closed_reason(conn: &Connection) -> Ended {
    use iroh::endpoint::ConnectionError;
    match conn.close_reason() {
        Some(ConnectionError::ApplicationClosed(close)) => {
            let reason = close.to_string();
            closed_by_rendezvous(close.error_code).unwrap_or(Ended::Failed(reason))
        }
        Some(reason) => Ended::Failed(reason.to_string()),
        None => Ended::Failed("rendezvous closed the stream".into()),
    }
}

/// How a close code from the rendezvous ends the session, unless it is
/// plain network trouble.
fn closed_by_rendezvous(code: iroh::endpoint::VarInt) -> Option<Ended> {
    if code == rdv_close(CloseCode::AuthFailed) {
        Some(Ended::AuthFailed(
            "the rendezvous did not accept the Minecraft session".into(),
        ))
    } else if code == rdv_close(CloseCode::Superseded) {
        Some(Ended::AuthFailed(
            "another player, signed in to the Minecraft account, is using this identity".into(),
        ))
    } else if code == rdv_close(CloseCode::RateLimited)
        || code == rdv_close(CloseCode::LimitExceeded)
    {
        // Reconnecting quickly would only hit the same limit again, so
        // these take the slow auth backoff, not the network one.
        tracing::warn!(%code, "the rendezvous closed us for exceeding a limit");
        Some(Ended::RetryAfter(Duration::ZERO))
    } else {
        None
    }
}

fn rdv_close(code: CloseCode) -> iroh::endpoint::VarInt {
    iroh::endpoint::VarInt::from_u32(code as u32)
}

/// Exponential backoff with jitter.
#[derive(Debug)]
struct Backoff {
    first: Duration,
    max: Duration,
    next: Duration,
}

impl Backoff {
    fn new(first: Duration, max: Duration) -> Self {
        Backoff {
            first,
            max,
            next: first,
        }
    }

    fn next(&mut self) -> Duration {
        let delay = self.next;
        self.next = (self.next * 2).min(self.max);
        // ±25 % so clients that failed together don't retry together.
        delay.mul_f64(rand::rng().random_range(0.75..1.25))
    }

    fn reset(&mut self) {
        self.next = self.first;
    }
}

#[cfg(test)]
mod tests {
    use yakvc_shared::rdv::PAIR_UPDATES_PER_SEC;

    use super::*;

    #[test]
    fn backoff_doubles_up_to_the_cap_with_jitter() {
        let mut backoff = Backoff::new(Duration::from_secs(10), Duration::from_secs(600));
        let delays: Vec<Duration> = (0..10).map(|_| backoff.next()).collect();
        let nominal = [10, 20, 40, 80, 160, 320, 600, 600, 600, 600];
        for (delay, nominal) in delays.iter().zip(nominal) {
            let nominal = Duration::from_secs(nominal);
            assert!(*delay >= nominal.mul_f64(0.75) && *delay <= nominal.mul_f64(1.25));
        }
        backoff.reset();
        assert!(backoff.next() <= Duration::from_secs(13));
    }

    #[test]
    fn limit_closes_take_the_auth_backoff() {
        for code in [CloseCode::RateLimited, CloseCode::LimitExceeded] {
            let ended = closed_by_rendezvous(rdv_close(code));
            assert!(matches!(ended, Some(Ended::RetryAfter(_))), "{code:?}");
        }
        let ended = closed_by_rendezvous(rdv_close(CloseCode::AuthFailed));
        assert!(matches!(ended, Some(Ended::AuthFailed(_))));
        for code in [
            CloseCode::Normal,
            CloseCode::ProtocolError,
            CloseCode::ShuttingDown,
        ] {
            assert!(closed_by_rendezvous(rdv_close(code)).is_none(), "{code:?}");
        }
    }

    #[test]
    fn pair_batches_stay_under_the_rate_limit() {
        // Two messages per batch, with a token a second to spare.
        let batches_per_sec = Duration::from_secs(1).div_duration_f64(PAIR_BATCH_INTERVAL);
        assert!(2.0 * batches_per_sec < f64::from(PAIR_UPDATES_PER_SEC) - 1.0);
    }

    #[test]
    fn pair_tokens_are_capped_deterministically() {
        let me = Uuid::from_u128(0);
        let players = 1..=MAX_PAIRS as u128 + 100;
        let tab_list: HashSet<Uuid> = players.clone().map(Uuid::from_u128).collect();
        let reversed: HashSet<Uuid> = players.rev().map(Uuid::from_u128).collect();
        let tokens = pair_tokens(me, &tab_list);
        assert_eq!(tokens.len(), MAX_PAIRS);
        assert_eq!(tokens, pair_tokens(me, &reversed));
    }
}
