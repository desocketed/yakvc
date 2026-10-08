//! Serves one `yakvc/rdv/1` connection.
//!
//! The client opens one bi-stream. Before registering it may only send
//! `Hello` and then `Joined` or `Decline` (for the `Challenge` it was sent),
//! and must be registered within [`AUTH_TIMEOUT`]. Once registered it sends
//! pair-token and address updates, and may `Renew` its ticket by running the
//! challenge again. A failed renewal never ends the session: the client keeps
//! its current ticket and is told to retry later. Every reply and push goes
//! through one writer task, so the reading side never has to give up halfway
//! through a message.

use std::collections::HashSet;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use iroh::endpoint::{Connection, Incoming, IncomingAddr, RecvStream, SendStream};
use tokio::sync::mpsc;
use yakvc_shared::auth::Nonce;
use yakvc_shared::rdv::{ClientMsg, CloseCode, Hello, ServerMsg, addr_fits};
use yakvc_shared::wire::{MAX_MESSAGE_LEN, WireError, read_msg_max, write_msg};
use yakvc_shared::{EndpointId, PairToken, Ticket, Uuid, is_offline_player};

use crate::auth::{JoinCheck, proves};
use crate::limits::{IpSlot, Slot, TokenBucket, ip_key};
use crate::server::Limits;
use crate::server::Shared;
use crate::sessions::{Outbox, close, push};

/// Time from accepting the connection until the client must be registered.
/// Covers the client's `joinServer` call to Mojang.
const AUTH_TIMEOUT: Duration = Duration::from_secs(30);

/// How long the writer may take to deliver its last messages on close.
const FLUSH_TIMEOUT: Duration = Duration::from_secs(1);

/// How long one message may wait for the client to make room for it.
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);

/// Largest message accepted before registering. A `Hello` with the largest
/// address `addr_fits` allows and a cached ticket is about 1 KiB.
const MAX_UNREGISTERED_MSG_LEN: usize = 4096;

/// Minecraft names are at most 16 characters.
const MAX_NAME_LEN: usize = 16;

/// How long a client whose renewal was refused waits before trying again.
/// It keeps its current ticket meanwhile.
const REFUSED_RENEWAL_RETRY: u32 = 60;

pub(crate) async fn serve(shared: Arc<Shared>, incoming: Incoming) {
    let ip = match incoming.remote_addr() {
        IncomingAddr::Ip(addr) => Some(addr.ip()),
        // Through the relay the client's IP is unknown.
        _ => None,
    };
    // Refused before the handshake, so a flood of connections that never
    // register costs as little as possible.
    let Some(unregistered) = shared.unregistered.take() else {
        incoming.refuse();
        return;
    };
    let unregistered_ip = match ip {
        Some(ip) => match shared.unregistered_per_ip.take(ip) {
            Some(slot) => Some(slot),
            None => {
                incoming.refuse();
                return;
            }
        },
        None => None,
    };
    let Ok(conn) = incoming.await else {
        return;
    };
    let deadline = tokio::time::Instant::now() + AUTH_TIMEOUT;
    let (send, recv) = match tokio::time::timeout_at(deadline, conn.accept_bi()).await {
        Ok(Ok(streams)) => streams,
        _ => {
            close(&conn, CloseCode::ProtocolError, "no stream");
            return;
        }
    };

    let (outbox, outbox_rx) = mpsc::channel(outbox_len(&shared.limits));
    let writer = tokio::spawn(write_all(conn.clone(), send, outbox_rx));
    let mut client = Client {
        shared: shared.clone(),
        conn: conn.clone(),
        id: conn.remote_id(),
        ip,
        recv,
        outbox,
        auth_deadline: deadline,
        // Replaced when the client registers.
        ticket_expires: deadline,
        hello: None,
        challenge: None,
        pair_updates: None,
        unregistered: Some((unregistered, unregistered_ip)),
        session_slot: None,
    };
    let code = match client.run().await {
        Ok(()) => CloseCode::Normal,
        Err(code) => code,
    };

    if shared.sessions.unregister(&conn)
        && let Some(gate) = &shared.relay_gate
    {
        gate.session_ended(client.id);
    }
    // The session held the other sender; with ours gone the writer finishes.
    drop(client);
    let _ = tokio::time::timeout(FLUSH_TIMEOUT, writer).await;
    close(&conn, code, "");
}

/// Writes queued messages until every sender is gone, then finishes the
/// stream and waits for the client to receive it.
async fn write_all(conn: Connection, mut send: SendStream, mut outbox: mpsc::Receiver<ServerMsg>) {
    while let Some(msg) = outbox.recv().await {
        match tokio::time::timeout(WRITE_TIMEOUT, write_msg(&mut send, &msg)).await {
            Ok(Ok(())) => {}
            // A client that misses a message is out of step with its
            // session; closing makes it reconnect and start over.
            Ok(Err(_)) => {
                close(&conn, CloseCode::Normal, "failed to send");
                return;
            }
            // Flow control held the write: the client stopped reading.
            Err(_) => {
                close(&conn, CloseCode::LimitExceeded, "not reading");
                return;
            }
        }
    }
    let _ = send.finish();
    let _ = send.stopped().await;
}

struct Client {
    shared: Arc<Shared>,
    conn: Connection,
    id: EndpointId,
    ip: Option<IpAddr>,
    recv: RecvStream,
    outbox: Outbox,
    auth_deadline: tokio::time::Instant,
    /// When the session's ticket expires, once registered.
    ticket_expires: tokio::time::Instant,
    /// The client's `Hello`, once received.
    hello: Option<Hello>,
    /// The nonce of the challenge awaiting `Joined`.
    challenge: Option<Nonce>,
    /// Present once registered.
    pair_updates: Option<TokenBucket>,
    /// Held until registered, against `Limits::max_unregistered` and (with
    /// a known IP) `max_unregistered_per_ip`.
    unregistered: Option<(Slot, Option<IpSlot>)>,
    /// Held from `Hello` on, against `Limits::max_sessions_per_ip`, when
    /// the IP is known.
    session_slot: Option<IpSlot>,
}

impl Client {
    /// Handles messages until the client ends the stream (`Ok`) or the
    /// connection has to be closed with a code.
    async fn run(&mut self) -> Result<(), CloseCode> {
        loop {
            // Matches are handed the session's ticket, so a session must not
            // outlive it; a renewal moves the deadline.
            let (deadline, code) = if self.is_registered() {
                (self.ticket_expires, CloseCode::AuthFailed)
            } else {
                (self.auth_deadline, CloseCode::ProtocolError)
            };
            // Before registering only `Hello` is large, so a small limit keeps
            // unauthenticated connections from making us allocate much.
            let max_len = if self.is_registered() {
                MAX_MESSAGE_LEN
            } else {
                MAX_UNREGISTERED_MSG_LEN
            };
            let msg = tokio::time::timeout_at(deadline, read_msg_max(&mut self.recv, max_len))
                .await
                .map_err(|_| code)?;
            let msg = match msg {
                Ok(Some(msg)) => msg,
                Ok(None) => return Ok(()),
                // The connection is gone; there is no one to tell.
                Err(WireError::Io(_)) => return Ok(()),
                Err(WireError::TooLarge(_) | WireError::Decode(_)) => {
                    return Err(CloseCode::ProtocolError);
                }
            };
            self.handle(msg).await?;
        }
    }

    async fn handle(&mut self, msg: ClientMsg) -> Result<(), CloseCode> {
        match msg {
            ClientMsg::Hello(hello) => self.on_hello(hello),
            ClientMsg::Joined => self.on_joined().await,
            ClientMsg::Decline => self.on_decline(),
            ClientMsg::Renew => self.on_renew(),
            ClientMsg::UpdateAddr(addr) => {
                self.check_update()?;
                if addr.id != self.id || !addr_fits(&addr) {
                    return Err(CloseCode::ProtocolError);
                }
                self.shared.sessions.update_addr(self.id, addr);
                Ok(())
            }
            ClientMsg::SetPairs(tokens) => {
                self.check_update()?;
                let distinct: HashSet<&PairToken> = tokens.iter().collect();
                if distinct.len() > self.shared.limits.max_pairs {
                    return Err(CloseCode::LimitExceeded);
                }
                self.shared.sessions.set_pairs(self.id, tokens);
                Ok(())
            }
            ClientMsg::AddPairs(tokens) => {
                self.check_update()?;
                let total = self
                    .shared
                    .sessions
                    .pair_count_after_adding(self.id, &tokens);
                if total > self.shared.limits.max_pairs {
                    return Err(CloseCode::LimitExceeded);
                }
                self.shared.sessions.add_pairs(self.id, tokens);
                Ok(())
            }
            ClientMsg::RemovePairs(tokens) => {
                self.check_update()?;
                self.shared.sessions.remove_pairs(self.id, tokens);
                Ok(())
            }
        }
    }

    fn on_hello(&mut self, hello: Hello) -> Result<(), CloseCode> {
        let valid_name = !hello.name.is_empty() && hello.name.len() <= MAX_NAME_LEN;
        // The address is forwarded to every match, so it must stay small.
        let valid_addr = hello.addr.id == self.id && addr_fits(&hello.addr);
        if self.hello.is_some() || !valid_addr || !valid_name {
            return Err(CloseCode::ProtocolError);
        }
        if let Some(ip) = self.ip {
            let slot = self.shared.sessions_per_ip.take(ip);
            self.session_slot = Some(slot.ok_or(CloseCode::LimitExceeded)?);
        }
        let cached = hello
            .cached_ticket
            .as_ref()
            .and_then(|ticket| self.shared.auth.reusable(ticket, &hello, self.id));
        self.hello = Some(hello);

        // An unverified cached ticket is refused while a verified session
        // holds its UUID; the challenge then decides, as without a ticket.
        if let Some(ticket) = cached
            && self.register(ticket)
        {
            self.shared.metrics.auth_cached.inc();
            Ok(())
        } else if self.shared.auth.is_dev() {
            let (uuid, name) = self.identity();
            let ticket = self.shared.auth.issue(uuid, name, self.id, true);
            self.shared.metrics.auth_ok.inc();
            self.register(ticket);
            Ok(())
        } else {
            self.start_challenge()
        }
    }

    /// Starts a renewal. A challenge left unanswered by a failed
    /// `joinServer` is simply replaced.
    fn on_renew(&mut self) -> Result<(), CloseCode> {
        if !self.is_registered() {
            return Err(CloseCode::ProtocolError);
        }
        self.challenge = None;
        if self.shared.auth.is_dev() {
            let (uuid, name) = self.identity();
            let ticket = self.shared.auth.issue(uuid, name, self.id, true);
            self.shared.metrics.auth_ok.inc();
            self.adopt(ticket);
            return Ok(());
        }
        self.start_challenge()
    }

    fn start_challenge(&mut self) -> Result<(), CloseCode> {
        // An offline UUID may decline the challenge, which needs no Mojang
        // call, so it is challenged even while Mojang is blocked.
        let (uuid, name) = self.identity();
        let offline = is_offline_player(uuid, &name);
        if !offline && let Some(secs) = self.shared.auth.retry_after() {
            return self.retry_later(secs);
        }
        // The challenge is what counts against the auth limits, so a
        // `Decline` (which must answer one) is counted too.
        if let Err(code) = self.check_auth_rate() {
            return self.refuse(code);
        }
        if let Some(secs) = self.check_challenge_budget(offline) {
            return self.retry_later(secs);
        }
        let nonce = Nonce::random();
        self.challenge = Some(nonce);
        self.send(ServerMsg::Challenge(nonce));
        Ok(())
    }

    async fn on_joined(&mut self) -> Result<(), CloseCode> {
        let nonce = self.challenge.take().ok_or(CloseCode::ProtocolError)?;
        let (uuid, name) = self.identity();
        if is_offline_player(uuid, &name)
            && let Some(secs) = self.check_mojang_budget()
        {
            return self.retry_later(secs);
        }

        let started = Instant::now();
        let check = self.shared.auth.check_join(&name, &nonce, self.id).await;
        self.shared.metrics.observe_mojang(started.elapsed());

        match check {
            JoinCheck::Confirmed(profile) if proves(&profile, uuid) => {
                self.shared.metrics.auth_ok.inc();
                // Mojang's spelling of the name; the UUID is the claimed one,
                // which may be the offline UUID of that name.
                let ticket = self.shared.auth.issue(uuid, profile.name, self.id, true);
                // A verified ticket is never refused.
                self.adopt(ticket);
                Ok(())
            }
            JoinCheck::Confirmed(_) | JoinCheck::Rejected => {
                self.shared.metrics.auth_failed.inc();
                self.refuse(CloseCode::AuthFailed)
            }
            JoinCheck::RetryAfter(secs) => self.retry_later(secs),
        }
    }

    /// The client can't prove its account: issue an unverified ticket, which
    /// is only valid for an offline UUID and only while no verified session
    /// holds that UUID. Mojang is not asked.
    fn on_decline(&mut self) -> Result<(), CloseCode> {
        self.challenge.take().ok_or(CloseCode::ProtocolError)?;
        let (uuid, name) = self.identity();
        if !is_offline_player(uuid, &name) {
            self.shared.metrics.auth_failed.inc();
            return self.refuse(CloseCode::AuthFailed);
        }
        let ticket = self.shared.auth.issue(uuid, name, self.id, false);
        if self.adopt(ticket) {
            self.shared.metrics.auth_unverified.inc();
            Ok(())
        } else {
            self.shared.metrics.auth_failed.inc();
            self.refuse(CloseCode::Superseded)
        }
    }

    /// Registers with `ticket`, or renews the session's ticket. Returns
    /// false if the sessions refused it (see `Sessions::register`).
    fn adopt(&mut self, ticket: Ticket) -> bool {
        if !self.is_registered() {
            return self.register(ticket);
        }
        let renewed =
            self.shared
                .sessions
                .update_ticket(self.id, ticket.signed().clone(), ticket.verified);
        if renewed {
            self.ticket_expires = expiry(&ticket);
            self.send(ServerMsg::Registered(ticket.signed().clone()));
        }
        renewed
    }

    /// Ends an initial authentication with `code`. A registered client keeps
    /// its session and current ticket instead, and is told to retry later.
    fn refuse(&mut self, code: CloseCode) -> Result<(), CloseCode> {
        if self.is_registered() {
            self.send(ServerMsg::RetryAfter {
                secs: REFUSED_RENEWAL_RETRY,
            });
            Ok(())
        } else {
            Err(code)
        }
    }

    /// Tells the client to try the challenge again later. A registered client
    /// keeps its session; one still authenticating is disconnected.
    fn retry_later(&mut self, secs: u32) -> Result<(), CloseCode> {
        self.shared.metrics.auth_retry_after.inc();
        self.send(ServerMsg::RetryAfter { secs });
        if self.is_registered() {
            Ok(())
        } else {
            Err(CloseCode::RateLimited)
        }
    }

    /// Returns false if the sessions refused the ticket (see
    /// `Sessions::register`).
    fn register(&mut self, ticket: Ticket) -> bool {
        let hello = self.hello.as_ref().expect("registered after hello");
        let registered = self.shared.sessions.register(
            &self.conn,
            hello.uuid,
            ticket.verified,
            ticket.signed().clone(),
            hello.addr.clone(),
            self.outbox.clone(),
        );
        if !registered {
            return false;
        }
        self.ticket_expires = expiry(&ticket);
        let limits = &self.shared.limits;
        self.pair_updates = Some(TokenBucket::new(
            limits.pair_updates_per_sec,
            Duration::from_secs(1),
            Instant::now(),
        ));
        self.unregistered = None;
        self.send(ServerMsg::Registered(ticket.signed().clone()));
        true
    }

    fn is_registered(&self) -> bool {
        self.pair_updates.is_some()
    }

    /// The UUID and name from the client's `Hello`.
    fn identity(&self) -> (Uuid, String) {
        let hello = self.hello.as_ref().expect("challenges follow a hello");
        (hello.uuid, hello.name.clone())
    }

    /// Session updates need a session and are rate limited.
    fn check_update(&mut self) -> Result<(), CloseCode> {
        let bucket = self.pair_updates.as_mut().ok_or(CloseCode::ProtocolError)?;
        if bucket.take(Instant::now()) {
            Ok(())
        } else {
            Err(CloseCode::RateLimited)
        }
    }

    /// Each challenge counts against the per-EndpointId and per-IP limits.
    fn check_auth_rate(&self) -> Result<(), CloseCode> {
        let now = Instant::now();
        let shared = &self.shared;
        let endpoint_ok = shared.auth_per_endpoint.lock().unwrap().allow(self.id, now);
        let ip_ok = match self.ip {
            Some(ip) => shared.auth_per_ip.lock().unwrap().allow(ip_key(ip), now),
            None => true,
        };
        if endpoint_ok && ip_ok {
            Ok(())
        } else {
            Err(CloseCode::RateLimited)
        }
    }

    /// Takes a challenge from the server-wide budgets, or returns the seconds
    /// to wait. Taken when the challenge is issued rather than at `Joined`,
    /// so a refused player hasn't called `joinServer` for nothing. An offline
    /// player may only decline, so their Mojang check is taken at `Joined`
    /// instead: declines must not drain the budget signed-in players need.
    fn check_challenge_budget(&self, offline: bool) -> Option<u32> {
        let now = Instant::now();
        if self.ip.is_none() {
            let mut bucket = self.shared.unknown_ip_challenges.lock().unwrap();
            if !bucket.take(now) {
                return Some(bucket.secs_to_next_token(now));
            }
        }
        if offline {
            return None;
        }
        self.check_mojang_budget()
    }

    /// Takes a Mojang check from the server-wide budget, or returns the
    /// seconds to wait.
    fn check_mojang_budget(&self) -> Option<u32> {
        let now = Instant::now();
        let mut bucket = self.shared.mojang_checks.lock().unwrap();
        if bucket.take(now) {
            None
        } else {
            Some(bucket.secs_to_next_token(now))
        }
    }

    fn send(&self, msg: ServerMsg) {
        push(&self.conn, &self.outbox, msg);
    }
}

/// Room for a `PeerAvailable` and a `PeerGone` for every pair, plus replies:
/// more than an honest client that keeps reading ever has queued.
fn outbox_len(limits: &Limits) -> usize {
    2 * limits.max_pairs + 16
}

/// When `ticket` expires, on the clock the read loop's timeouts use.
fn expiry(ticket: &Ticket) -> tokio::time::Instant {
    tokio::time::Instant::now() + ticket.remaining(SystemTime::now())
}
