//! Iroh endpoint, rendezvous session, peer manager and protocol routing.
//!
//! `net` delivers verified peers: each [`PeerLink`] is one QUIC connection
//! whose remote EndpointId matched a trusted ticket for a UUID in the tab
//! list. Application protocols implement [`Protocol`] and get one link per
//! peer; they never see unverified traffic.
//!
//! This seam is internal to `yakvc-client` (one agent owns both sides), so it
//! is a sketch to refine during implementation, not a fixed contract.

mod peer;
mod rendezvous;
#[cfg(test)]
pub(crate) mod test_util;

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use bytes::Bytes;
use iroh::endpoint::{Connection, RecvStream, SendStream, VarInt};
use iroh::{Endpoint, EndpointAddr, EndpointId, Watcher};
use tokio::runtime::Handle;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::AbortHandle;
use yakvc_shared::peer::CloseCode;
use yakvc_shared::{ProtocolId, SignedTicket, Ticket, TicketVerifier, Uuid, wire};

use crate::config::RendezvousConfig;
use crate::event::{Event, EventSender, JoinId, PeerState};

/// An application protocol on peer connections.
pub trait Protocol: Send + Sync + 'static {
    fn id(&self) -> ProtocolId;

    /// Highest version this side speaks.
    fn version(&self) -> u16;

    /// Serves one peer until the link closes.
    fn serve(&self, link: PeerLink) -> Pin<Box<dyn Future<Output = ()> + Send>>;
}

/// One verified peer, scoped to one protocol. Its parts are separate so a
/// protocol can read datagrams and control messages from different tasks;
/// reading a control message is not cancel-safe, so it should not sit in a
/// `select!` next to datagrams.
#[derive(Debug)]
pub struct PeerLink {
    pub peer: Peer,
    /// Datagrams for this protocol, protocol byte removed. Ends when the
    /// connection closes.
    pub datagrams: mpsc::Receiver<Bytes>,
    pub control_send: ControlSend,
    pub control_recv: ControlRecv,
}

/// Cheap handle to a verified peer for sending datagrams, scoped to one
/// protocol. Cloneable, and usable from any thread.
#[derive(Debug, Clone)]
pub struct Peer {
    uuid: Uuid,
    version: u16,
    protocol: ProtocolId,
    conn: Connection,
    status: Arc<PeerStatus>,
}

/// Sending half of a protocol's control stream.
#[derive(Debug)]
pub struct ControlSend(SendStream);

/// Receiving half of a protocol's control stream.
#[derive(Debug)]
pub struct ControlRecv(RecvStream);

#[derive(Debug, thiserror::Error)]
pub enum LinkError {
    #[error("peer connection closed")]
    Closed,
    #[error("datagram too large")]
    TooLarge,
}

impl Peer {
    pub fn uuid(&self) -> Uuid {
        self.uuid
    }

    /// Version agreed in the peer hello.
    pub fn version(&self) -> u16 {
        self.version
    }

    /// Identifies the connection, so a protocol can tell a replaced link
    /// for the same player from the current one.
    pub fn link_id(&self) -> usize {
        self.conn.stable_id()
    }

    /// Sends a datagram; the protocol byte is added for you.
    pub fn send_datagram(&self, payload: &[u8]) -> Result<(), LinkError> {
        let mut datagram = Vec::with_capacity(1 + payload.len());
        datagram.push(self.protocol.0);
        datagram.extend_from_slice(payload);
        if self
            .conn
            .max_datagram_size()
            .is_some_and(|max| datagram.len() > max)
        {
            return Err(LinkError::TooLarge);
        }
        self.conn
            .send_datagram(Bytes::from(datagram))
            .map_err(|_| LinkError::Closed)
    }

    /// Whether traffic currently goes through the relay.
    pub fn is_relayed(&self) -> bool {
        self.status.is_relayed()
    }

    /// Marks the peer as outside (or back inside) the relay budget, which
    /// shows in its [`PeerState`].
    pub fn set_relay_full(&self, full: bool) {
        self.status.update(|flags| flags.relay_full = full);
    }
}

impl ControlSend {
    /// Sends a message on this protocol's control stream.
    pub async fn send<M: serde::Serialize>(&mut self, msg: &M) -> Result<(), LinkError> {
        wire::write_msg(&mut self.0, msg)
            .await
            .map_err(|_| LinkError::Closed)
    }
}

impl ControlRecv {
    /// Next message on this protocol's control stream. `None` once closed or
    /// after a malformed message.
    pub async fn recv<M: serde::de::DeserializeOwned>(&mut self) -> Option<M> {
        wire::read_msg(&mut self.0).await.ok().flatten()
    }
}

/// The local player, as told by the game.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Identity {
    pub uuid: Uuid,
    pub name: String,
}

/// Which peer tickets are accepted.
#[derive(Debug, Clone)]
pub(crate) struct Trust {
    pub verifier: TicketVerifier,
    /// Direct-call mode (CLI): accept any self-signed ticket and ignore the
    /// tab list. Never set in the game.
    pub direct_calls: bool,
}

/// One row of [`Net::peers`].
#[derive(Debug, Clone)]
pub(crate) struct PeerSummary {
    pub uuid: Uuid,
    pub name: String,
    pub state: PeerState,
    pub rtt: Option<Duration>,
}

/// The network half of the engine: the Iroh endpoint, verified peer
/// connections and the rendezvous session.
#[derive(Debug, Clone)]
pub(crate) struct Net {
    inner: Arc<Inner>,
}

struct Inner {
    endpoint: Endpoint,
    runtime: Handle,
    protocols: Vec<Arc<dyn Protocol>>,
    events: EventSender,
    identity: watch::Sender<Option<Identity>>,
    tab_list: watch::Sender<HashSet<Uuid>>,
    /// Our ticket from the rendezvous, sent in every peer hello.
    own_ticket: watch::Sender<Option<SignedTicket>>,
    trust: Mutex<Trust>,
    peers: Mutex<HashMap<EndpointId, PeerEntry>>,
    banned: Bans,
    joins: Mutex<Joins>,
    /// The rendezvous session task, stopped by direct-call mode.
    rendezvous: Mutex<Option<AbortHandle>>,
}

/// Peers that flooded us, and until when they are refused.
type Bans = Arc<Mutex<HashMap<EndpointId, tokio::time::Instant>>>;

/// Everything known about one remote endpoint.
struct PeerEntry {
    uuid: Uuid,
    name: String,
    /// Where to dial. Set while the rendezvous lists the peer.
    addr: Option<EndpointAddr>,
    link: Option<Link>,
    status: Arc<PeerStatus>,
    /// After a failed handshake, no new dial before this time.
    retry_after: Option<tokio::time::Instant>,
    /// The task that keeps a listed peer connected.
    dialer: Option<AbortHandle>,
    /// How far away the player is, while it has a tracked entity.
    distance: Option<f64>,
    /// When the player was last tracked, or when we learned of it.
    untracked_since: tokio::time::Instant,
}

/// How much a peer deserves one of the [`MAX_PEERS`] connections: tracked
/// peers nearest first, then untracked ones, most recently tracked first.
/// Lower is better; the derived order compares the variant first.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
enum Rank {
    Tracked { distance: f64 },
    Untracked { for_secs: f64 },
}

/// A verified connection to a peer.
struct Link {
    conn: Connection,
    /// Whether the endpoint with the lower id dialed it, which decides
    /// which of two duplicate connections survives.
    dialed_by_lower: bool,
}

/// Outstanding `joinServer` requests from rendezvous challenges.
#[derive(Default)]
struct Joins {
    next_id: u32,
    pending: HashMap<JoinId, oneshot::Sender<bool>>,
}

/// Connection state of one peer, shared by the peer table, the path watcher
/// and protocol handles. Emits [`Event::Peer`] whenever the visible state
/// changes.
#[derive(Debug)]
struct PeerStatus {
    uuid: Uuid,
    events: EventSender,
    flags: Mutex<StatusFlags>,
}

#[derive(Debug, Default)]
struct StatusFlags {
    connected: bool,
    failed: bool,
    relayed: bool,
    relay_full: bool,
    reported: Option<PeerState>,
}

/// Most open peer connections, see "Connection lifecycle" in DESIGN.md.
const MAX_PEERS: usize = 64;

/// How long to wait for the first net report. Iroh gives up on a report
/// after 5 s, and the first one starts once the endpoint is bound.
const NET_REPORT_WAIT: Duration = Duration::from_secs(10);

/// How long a peer that failed the handshake is left alone.
const RETRY_AFTER_FAILURE: Duration = Duration::from_secs(60);

impl Net {
    /// Starts serving peer connections on `endpoint`. Must be called inside
    /// the engine's Tokio runtime.
    pub(crate) fn new(
        endpoint: Endpoint,
        protocols: Vec<Arc<dyn Protocol>>,
        trust: Trust,
        events: EventSender,
    ) -> Net {
        let inner = Arc::new(Inner {
            endpoint,
            runtime: Handle::current(),
            protocols,
            events,
            identity: watch::Sender::new(None),
            tab_list: watch::Sender::new(HashSet::new()),
            own_ticket: watch::Sender::new(None),
            trust: Mutex::new(trust),
            peers: Mutex::new(HashMap::new()),
            banned: Bans::default(),
            joins: Mutex::new(Joins::default()),
            rendezvous: Mutex::new(None),
        });
        tokio::spawn(peer::accept_loop(inner.clone()));
        tokio::spawn(close_hidden_peers(inner.clone()));
        tokio::spawn(evict_over_cap(inner.clone()));
        Net { inner }
    }

    /// The distance to each player with a tracked entity, as a hint for
    /// which peers to keep connected when there are too many.
    pub(crate) fn set_tracked(&self, distances: &HashMap<Uuid, f64>) {
        let now = tokio::time::Instant::now();
        let mut peers = self.inner.peers.lock().unwrap();
        for entry in peers.values_mut() {
            entry.distance = distances.get(&entry.uuid).copied();
            if entry.distance.is_some() {
                entry.untracked_since = now;
            }
        }
    }

    pub(crate) fn endpoint_id(&self) -> EndpointId {
        self.inner.endpoint.id()
    }

    /// Our current address, for a direct call.
    pub(crate) fn addr(&self) -> EndpointAddr {
        self.inner.endpoint.addr()
    }

    pub(crate) fn set_identity(&self, identity: Identity) {
        self.inner.identity.send_if_modified(|current| {
            let changed = current.as_ref() != Some(&identity);
            *current = Some(identity);
            changed
        });
    }

    pub(crate) fn set_tab_list(&self, players: HashSet<Uuid>) {
        self.inner.tab_list.send_if_modified(|current| {
            let changed = *current != players;
            *current = players;
            changed
        });
    }

    pub(crate) fn set_trust(&self, verifier: TicketVerifier) {
        self.inner.trust.lock().unwrap().verifier = verifier;
    }

    /// Talks to the rendezvous until the engine shuts down: authenticates,
    /// keeps the ticket fresh, sends pair tokens and connects to matches.
    pub(crate) fn start_rendezvous(&self, config: RendezvousConfig, ticket_cache: PathBuf) {
        let inner = self.inner.clone();
        let task = self
            .inner
            .runtime
            .spawn(rendezvous::run(inner, config, ticket_cache));
        *self.inner.rendezvous.lock().unwrap() = Some(task.abort_handle());
    }

    /// Answers a [`Event::JoinRequest`].
    pub(crate) fn complete_join(&self, id: JoinId, ok: bool) {
        let sender = self.inner.joins.lock().unwrap().pending.remove(&id);
        if let Some(sender) = sender {
            let _ = sender.send(ok);
        }
    }

    /// Switches to direct-call mode: peers are trusted on a self-signed
    /// `ticket` and the tab list is ignored. For `yakvc-cli call`. The
    /// rendezvous session stops, because nobody can answer its Mojang
    /// challenge; the endpoint keeps the configured relay as a fallback path.
    pub(crate) fn enable_direct_calls(&self, ticket: SignedTicket) {
        if let Some(rendezvous) = self.inner.rendezvous.lock().unwrap().take() {
            rendezvous.abort();
        }
        self.inner.trust.lock().unwrap().direct_calls = true;
        self.inner.own_ticket.send_replace(Some(ticket));
    }

    /// Dials `addr` once, in direct-call mode.
    pub(crate) fn call(&self, addr: EndpointAddr) {
        let inner = self.inner.clone();
        self.inner.runtime.spawn(async move {
            if let Err(err) = peer::dial(&inner, addr).await {
                inner
                    .events
                    .send(Event::Error(format!("call failed: {err}")));
            }
        });
    }

    pub(crate) fn peers(&self) -> Vec<PeerSummary> {
        let peers = self.inner.peers.lock().unwrap();
        peers
            .values()
            .map(|entry| PeerSummary {
                uuid: entry.uuid,
                name: entry.name.clone(),
                state: entry.status.state(),
                rtt: entry
                    .link
                    .as_ref()
                    .and_then(|link| selected_rtt(&link.conn)),
            })
            .collect()
    }

    /// The endpoint's latest net report, waiting a while for the first one.
    /// `None` without a relay, because the probes go to relays.
    pub(crate) async fn net_report(&self) -> Option<iroh::unstable_net_report::NetReport> {
        let endpoint = self.inner.endpoint.clone();
        // On the engine's runtime, so any async caller can await it.
        let report = self.inner.runtime.spawn(async move {
            let mut reports = endpoint.net_report();
            tokio::time::timeout(NET_REPORT_WAIT, reports.initialized())
                .await
                .ok()
        });
        report.await.ok().flatten()
    }

    /// Closes every connection and the endpoint.
    pub(crate) async fn close(&self) {
        self.inner.endpoint.close().await;
    }

    #[cfg(test)]
    fn inner(&self) -> &Arc<Inner> {
        &self.inner
    }

    #[cfg(test)]
    pub(crate) fn loopback_addr(&self) -> EndpointAddr {
        test_util::loopback_addr(&self.inner.endpoint)
    }
}

impl Inner {
    fn own_uuid(&self) -> Option<Uuid> {
        self.identity
            .borrow()
            .as_ref()
            .map(|identity| identity.uuid)
    }

    /// Verifies a peer's ticket for a connection from `remote`: signature,
    /// issuer and expiry, then [`check_peer`].
    fn verify_peer(&self, signed: &SignedTicket, remote: EndpointId) -> Result<Ticket, CloseCode> {
        let trust = self.trust.lock().unwrap().clone();
        let verifier = if trust.direct_calls {
            // A direct call has no rendezvous, so each side signs its own
            // ticket. It still proves the EndpointId owns the claimed UUID's
            // key pair, which is all a direct call needs.
            TicketVerifier::new([signed.issuer()]).accept_dev(true)
        } else {
            trust.verifier
        };
        let ticket = verifier
            .verify(signed, SystemTime::now())
            .map_err(|_| CloseCode::BadTicket)?;
        check_peer(
            &ticket,
            remote,
            self.own_uuid(),
            &self.tab_list.borrow(),
            trust.direct_calls,
        )?;
        Ok(ticket)
    }

    /// A match from the rendezvous: remember where to reach it and keep it
    /// connected.
    fn peer_available(self: &Arc<Self>, signed: SignedTicket, addr: EndpointAddr) {
        let remote = addr.id;
        if remote == self.endpoint.id() {
            return;
        }
        let verifier = self.trust.lock().unwrap().verifier.clone();
        let ticket = match verifier.verify(&signed, SystemTime::now()) {
            Ok(ticket) if ticket.endpoint_id == remote => ticket,
            // The rendezvous should never send this; the peer handshake
            // would reject it anyway.
            _ => return,
        };
        let mut peers = self.peers.lock().unwrap();
        let entry = peers
            .entry(remote)
            .or_insert_with(|| PeerEntry::new(ticket.uuid, ticket.name.clone(), &self.events));
        entry.addr = Some(addr);
        entry.status.update(|_| {});
        if entry.dialer.as_ref().is_none_or(|task| task.is_finished()) {
            let task = self
                .runtime
                .spawn(peer::keep_connected(self.clone(), remote));
            entry.dialer = Some(task.abort_handle());
        }
    }

    /// The rendezvous no longer matches us with `remote`.
    fn peer_gone(&self, remote: EndpointId) {
        let entry = self.peers.lock().unwrap().remove(&remote);
        if let Some(entry) = entry {
            entry.remove(CloseCode::Normal);
        }
    }

    fn new_join(&self) -> (JoinId, oneshot::Receiver<bool>) {
        let (tx, rx) = oneshot::channel();
        let mut joins = self.joins.lock().unwrap();
        let id = JoinId(joins.next_id);
        joins.next_id = joins.next_id.wrapping_add(1);
        joins.pending.insert(id, tx);
        (id, rx)
    }

    /// Peers we can see, best first (see [`Rank`]).
    fn ranked(&self) -> Vec<EndpointId> {
        let direct_calls = self.trust.lock().unwrap().direct_calls;
        let visible = self.tab_list.borrow();
        let now = tokio::time::Instant::now();
        let peers = self.peers.lock().unwrap();
        let ranks = peers
            .iter()
            .filter(|(_, entry)| direct_calls || visible.contains(&entry.uuid))
            .map(|(&id, entry)| (id, entry.rank(now)))
            .collect();
        by_rank(ranks)
    }

    /// Whether `remote` ranks among the [`MAX_PEERS`] best peers, so it
    /// deserves a connection even at the cap.
    fn wants(&self, remote: EndpointId) -> bool {
        self.ranked().iter().take(MAX_PEERS).any(|&id| id == remote)
    }

    fn is_banned(&self, remote: EndpointId) -> bool {
        let mut banned = self.banned.lock().unwrap();
        banned.retain(|_, until| *until > tokio::time::Instant::now());
        banned.contains_key(&remote)
    }
}

/// The checks on a verified ticket that depend on the connection and the
/// game, from DESIGN.md "Peer handshake" step 3.
pub(crate) fn check_peer(
    ticket: &yakvc_shared::TicketBody,
    remote: EndpointId,
    own_uuid: Option<Uuid>,
    tab_list: &HashSet<Uuid>,
    direct_calls: bool,
) -> Result<(), CloseCode> {
    if ticket.endpoint_id != remote {
        return Err(CloseCode::BadTicket);
    }
    if Some(ticket.uuid) == own_uuid {
        return Err(CloseCode::BadTicket);
    }
    if !direct_calls && !tab_list.contains(&ticket.uuid) {
        return Err(CloseCode::NotVisible);
    }
    Ok(())
}

/// Closes connections to players who left the tab list. The rendezvous will
/// also send `PeerGone`, but the tab list is the faster and local signal.
async fn close_hidden_peers(inner: Arc<Inner>) {
    let mut tab_list = inner.tab_list.subscribe();
    while tab_list.changed().await.is_ok() {
        if inner.trust.lock().unwrap().direct_calls {
            continue;
        }
        let visible = tab_list.borrow_and_update().clone();
        let peers = inner.peers.lock().unwrap();
        for entry in peers.values() {
            if let Some(link) = &entry.link
                && !visible.contains(&entry.uuid)
            {
                close(&link.conn, CloseCode::NotVisible);
            }
        }
    }
}

/// Closes the worst-ranked connections while there are more than
/// [`MAX_PEERS`]. That happens when a peer that now ranks higher, say one
/// that just came into tracking range, connects at the cap.
async fn evict_over_cap(inner: Arc<Inner>) {
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    loop {
        interval.tick().await;
        let ranked = inner.ranked();
        let peers = inner.peers.lock().unwrap();
        let open: HashSet<EndpointId> = ranked
            .iter()
            .filter(|&id| peers.get(id).is_some_and(|e| e.open_conn().is_some()))
            .copied()
            .collect();
        for id in evictions(&ranked, &open, MAX_PEERS) {
            if let Some(conn) = peers.get(&id).and_then(PeerEntry::open_conn) {
                tracing::debug!(remote = %id.fmt_short(), "too many peers, evicting");
                close(conn, CloseCode::Normal);
            }
        }
    }
}

/// Sorts peers best first.
fn by_rank(mut peers: Vec<(EndpointId, Rank)>) -> Vec<EndpointId> {
    peers.sort_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    peers.into_iter().map(|(id, _)| id).collect()
}

/// The `open` connections to close, worst first, to get down to `max`.
/// `ranked` is best first.
fn evictions(ranked: &[EndpointId], open: &HashSet<EndpointId>, max: usize) -> Vec<EndpointId> {
    let excess = open.len().saturating_sub(max);
    ranked
        .iter()
        .rev()
        .filter(|&id| open.contains(id))
        .take(excess)
        .copied()
        .collect()
}

impl PeerEntry {
    fn new(uuid: Uuid, name: String, events: &EventSender) -> PeerEntry {
        PeerEntry {
            uuid,
            name,
            addr: None,
            link: None,
            status: Arc::new(PeerStatus {
                uuid,
                events: events.clone(),
                flags: Mutex::new(StatusFlags::default()),
            }),
            retry_after: None,
            dialer: None,
            distance: None,
            untracked_since: tokio::time::Instant::now(),
        }
    }

    fn rank(&self, now: tokio::time::Instant) -> Rank {
        match self.distance {
            Some(distance) => Rank::Tracked { distance },
            None => Rank::Untracked {
                for_secs: (now - self.untracked_since).as_secs_f64(),
            },
        }
    }

    /// Drops the peer for good: closes its connection and reports it gone.
    fn remove(self, code: CloseCode) {
        if let Some(dialer) = self.dialer {
            dialer.abort();
        }
        if let Some(link) = self.link {
            close(&link.conn, code);
        }
        self.status.report_gone();
    }

    /// The verified connection, if it is still open.
    fn open_conn(&self) -> Option<&Connection> {
        self.link
            .as_ref()
            .map(|link| &link.conn)
            .filter(|conn| conn.close_reason().is_none())
    }
}

impl PeerStatus {
    fn update(&self, change: impl FnOnce(&mut StatusFlags)) {
        let mut flags = self.flags.lock().unwrap();
        change(&mut flags);
        let state = flags.state();
        if flags.reported != Some(state) {
            flags.reported = Some(state);
            self.events.send(Event::Peer {
                uuid: self.uuid,
                state,
            });
        }
    }

    fn report_gone(&self) {
        let mut flags = self.flags.lock().unwrap();
        if flags.reported.is_some() && flags.reported != Some(PeerState::Gone) {
            flags.reported = Some(PeerState::Gone);
            self.events.send(Event::Peer {
                uuid: self.uuid,
                state: PeerState::Gone,
            });
        }
    }

    fn state(&self) -> PeerState {
        self.flags.lock().unwrap().state()
    }

    fn is_relayed(&self) -> bool {
        self.flags.lock().unwrap().relayed
    }
}

impl StatusFlags {
    fn state(&self) -> PeerState {
        match (self.connected, self.relayed, self.relay_full) {
            (false, _, _) if self.failed => PeerState::Failed,
            (false, _, _) => PeerState::Connecting,
            (true, false, _) => PeerState::Direct,
            (true, true, false) => PeerState::Relayed,
            (true, true, true) => PeerState::RelayFull,
        }
    }
}

fn close(conn: &Connection, code: CloseCode) {
    conn.close(
        VarInt::from_u32(code as u32),
        format!("{code:?}").as_bytes(),
    );
}

/// Round-trip time on the path currently carrying traffic.
fn selected_rtt(conn: &Connection) -> Option<Duration> {
    let paths = conn.paths();
    let selected = paths.iter().find(|path| path.is_selected());
    selected.map(|path| path.rtt())
}

/// Whether the path currently carrying traffic goes through a relay.
fn is_relayed(conn: &Connection) -> bool {
    let paths = conn.paths();
    let selected = paths.iter().find(|path| path.is_selected());
    selected.is_some_and(|path| path.is_relay())
}

impl std::fmt::Debug for Inner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Net")
            .field("id", &self.endpoint.id())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use iroh::SecretKey;
    use yakvc_shared::TicketBody;

    use super::test_util::uuid;
    use super::*;

    fn body(uuid: Uuid, endpoint: EndpointId) -> TicketBody {
        TicketBody {
            uuid,
            name: "bob".into(),
            endpoint_id: endpoint,
            issued_at: 0,
            expires_at: u64::MAX,
            dev: false,
        }
    }

    #[test]
    fn peer_checks_follow_the_handshake_rules() {
        let remote = SecretKey::generate().public();
        let other = SecretKey::generate().public();
        let me = Some(uuid(1));
        let tab_list = HashSet::from([uuid(1), uuid(2)]);
        let check = |body: &TicketBody, direct| check_peer(body, remote, me, &tab_list, direct);

        assert_eq!(check(&body(uuid(2), remote), false), Ok(()));
        assert_eq!(
            check(&body(uuid(2), other), false),
            Err(CloseCode::BadTicket),
            "ticket names another endpoint"
        );
        assert_eq!(
            check(&body(uuid(1), remote), false),
            Err(CloseCode::BadTicket),
            "claims to be us"
        );
        assert_eq!(
            check(&body(uuid(3), remote), false),
            Err(CloseCode::NotVisible),
            "not in our tab list"
        );
        assert_eq!(check(&body(uuid(3), remote), true), Ok(()), "direct call");
        assert_eq!(
            check(&body(uuid(3), other), true),
            Err(CloseCode::BadTicket),
            "direct calls still bind the endpoint"
        );
    }

    #[test]
    fn tracked_peers_rank_nearest_first_then_untracked_most_recent_first() {
        let ids: Vec<EndpointId> = (0..5).map(|_| SecretKey::generate().public()).collect();
        let ranked = by_rank(vec![
            (ids[4], Rank::Untracked { for_secs: 600.0 }),
            (ids[2], Rank::Tracked { distance: 90.0 }),
            (ids[3], Rank::Untracked { for_secs: 5.0 }),
            (ids[0], Rank::Tracked { distance: 3.0 }),
            (ids[1], Rank::Tracked { distance: 40.0 }),
        ]);
        assert_eq!(ranked, ids);
    }

    #[test]
    fn eviction_closes_the_worst_open_connections_down_to_the_cap() {
        let ids: Vec<EndpointId> = (0..5).map(|_| SecretKey::generate().public()).collect();
        // 1 isn't open; the other four are, one over a cap of three.
        let open = HashSet::from([ids[0], ids[2], ids[3], ids[4]]);
        assert_eq!(evictions(&ids, &open, 3), vec![ids[4]]);
        assert_eq!(evictions(&ids, &open, 2), vec![ids[4], ids[3]]);
        assert!(evictions(&ids, &open, 4).is_empty());
    }

    #[test]
    fn peer_state_combines_connection_and_path() {
        let state = |connected, failed, relayed, relay_full| {
            StatusFlags {
                connected,
                failed,
                relayed,
                relay_full,
                reported: None,
            }
            .state()
        };
        assert_eq!(state(false, false, false, false), PeerState::Connecting);
        assert_eq!(state(false, true, false, false), PeerState::Failed);
        assert_eq!(state(true, false, false, true), PeerState::Direct);
        assert_eq!(state(true, false, true, false), PeerState::Relayed);
        assert_eq!(state(true, false, true, true), PeerState::RelayFull);
    }
}
