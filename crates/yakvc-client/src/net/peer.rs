//! Peer connections, ALPN `yakvc/peer/1`: dialing and accepting, the hello
//! handshake, and routing each verified connection's streams and datagrams to
//! the application protocols.
//!
//! Stream layout: the dialer opens the peer control stream and both sides
//! send [`PeerMsg::Hello`] on it. Once both hellos verify, the dialer opens
//! one bi-stream per agreed protocol, starting with that protocol's ID byte.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use bytes::Bytes;
use iroh::endpoint::{Connection, ConnectionError, RecvStream, SendStream};
use iroh::{EndpointAddr, EndpointId};
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio::time::Instant;
use yakvc_shared::peer::{ALPN, CloseCode, PeerHello, PeerMsg};
use yakvc_shared::{SignedTicket, Ticket, Uuid, wire};

use super::{
    Bans, ControlRecv, ControlSend, Inner, Link, Peer, PeerEntry, PeerLink, PeerStatus, Protocol,
    RETRY_AFTER_FAILURE, close, is_relayed,
};

/// The higher EndpointId waits this long for the lower one to dial first.
const DIAL_GRACE: Duration = Duration::from_secs(3);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// How often the selected path is checked for the direct/relayed state.
const PATH_POLL: Duration = Duration::from_secs(1);
/// Datagrams queued per protocol before new ones are dropped.
const DATAGRAM_QUEUE: usize = 64;

/// Why a peer connection was refused or lost during setup.
#[derive(Debug, thiserror::Error)]
pub(super) enum SetupError {
    #[error("rejected: {0:?}")]
    Rejected(CloseCode),
    #[error("connection failed: {0}")]
    Connection(String),
    #[error("timed out")]
    TimedOut,
}

/// What a successful hello exchange yields.
struct Hello {
    uuid: Uuid,
    name: String,
    /// When the peer's ticket expires.
    expires: Instant,
    /// Protocols both sides speak, with the agreed version.
    agreed: Vec<(Arc<dyn Protocol>, u16)>,
    control_send: SendStream,
    control_recv: RecvStream,
}

pub(super) async fn accept_loop(inner: Arc<Inner>) {
    while let Some(incoming) = inner.endpoint.accept().await {
        let inner = inner.clone();
        tokio::spawn(async move {
            let Ok(conn) = incoming.await else { return };
            let remote = conn.remote_id();
            if conn.alpn() != ALPN || inner.is_banned(remote) {
                close(&conn, CloseCode::ProtocolError);
                return;
            }
            // At the cap, only a peer that outranks an open one gets in (and
            // the worst one is then evicted). Refusing others also stops a
            // peer we evicted from coming straight back.
            if connected_count(&inner) >= inner.max_peers && !inner.wants(remote) {
                close(&conn, CloseCode::TooManyPeers);
                return;
            }
            match setup(&inner, &conn, false).await {
                Ok(hello) => serve(inner, conn, false, hello).await,
                Err(SetupError::Rejected(_)) => mark_failed(&inner, conn.remote_id()),
                Err(_) => {}
            }
        });
    }
}

/// Keeps a listed peer connected until it is no longer listed. The task is
/// aborted when the rendezvous sends `PeerGone`.
pub(super) async fn keep_connected(inner: Arc<Inner>, remote: EndpointId) {
    loop {
        let (addr, open_conn, retry_after) = {
            let peers = inner.peers.lock().unwrap();
            let Some(entry) = peers.get(&remote) else {
                return;
            };
            (
                entry.addr.clone(),
                entry.open_conn().cloned(),
                entry.retry_after,
            )
        };
        let Some(addr) = addr else { return };
        if let Some(conn) = open_conn {
            conn.closed().await;
            continue;
        }
        if let Some(when) = retry_after
            && when > Instant::now()
        {
            tokio::time::sleep_until(when).await;
            continue;
        }
        // Only dial players we can see that rank among the best `max_peers`,
        // so nearer players get connections first. A peer evicted at the cap
        // is dialed again once it ranks high enough, e.g. when tracked again.
        if !inner.wants(remote) {
            tokio::time::sleep(Duration::from_secs(1)).await;
            continue;
        }
        // The lower EndpointId dials. The other side only dials if nothing
        // arrived in time, e.g. because only it can reach the other.
        if inner.endpoint.id() > remote && wait_for_link(&inner, remote, DIAL_GRACE).await {
            continue;
        }
        set_status(&inner, remote, |flags| flags.failed = false);
        if let Err(SetupError::Rejected(_) | SetupError::TimedOut) = dial(&inner, addr).await {
            mark_failed(&inner, remote);
        } else if open_conn_of(&inner, remote).is_none() {
            // Lost the connection during setup; try again shortly rather
            // than spinning.
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
}

/// Connects to `addr` and runs the handshake. Returns once the connection
/// is verified and being served, or failed.
pub(super) async fn dial(inner: &Arc<Inner>, addr: EndpointAddr) -> Result<(), SetupError> {
    let connect = inner.endpoint.connect(addr, ALPN);
    let conn = match tokio::time::timeout(CONNECT_TIMEOUT, connect).await {
        Ok(Ok(conn)) => conn,
        Ok(Err(err)) => return Err(SetupError::Connection(err.to_string())),
        Err(_) => return Err(SetupError::TimedOut),
    };
    let hello = setup(inner, &conn, true).await?;
    tokio::spawn(serve(inner.clone(), conn, true, hello));
    Ok(())
}

/// Runs the hello exchange, closing `conn` with the matching code on failure.
async fn setup(inner: &Arc<Inner>, conn: &Connection, dialed: bool) -> Result<Hello, SetupError> {
    let result = match tokio::time::timeout(HANDSHAKE_TIMEOUT, handshake(inner, conn, dialed)).await
    {
        // The peer closing the connection during the handshake means it
        // refused us (bad ticket, not visible, at its cap), which is retried
        // only after a while.
        Ok(Err(SetupError::Connection(_)))
            if matches!(
                conn.close_reason(),
                Some(ConnectionError::ApplicationClosed(_))
            ) =>
        {
            Err(SetupError::Rejected(CloseCode::Normal))
        }
        Ok(result) => result,
        Err(_) => Err(SetupError::TimedOut),
    };
    if let Err(err) = &result {
        tracing::debug!(remote = %conn.remote_id().fmt_short(), %err, "peer handshake failed");
        record_peer_close(inner, conn);
        let code = match err {
            SetupError::Rejected(code) => *code,
            SetupError::Connection(_) | SetupError::TimedOut => CloseCode::ProtocolError,
        };
        close(conn, code);
    }
    result
}

async fn handshake(
    inner: &Arc<Inner>,
    conn: &Connection,
    dialed: bool,
) -> Result<Hello, SetupError> {
    let remote = conn.remote_id();
    // Without our own ticket there is nothing to prove who we are.
    let own_ticket = inner.own_ticket.borrow().clone();
    let Some(own_ticket) = own_ticket else {
        return Err(SetupError::Rejected(CloseCode::Normal));
    };
    let (mut send, mut recv) = if dialed {
        conn.open_bi().await
    } else {
        conn.accept_bi().await
    }
    .map_err(|err| SetupError::Connection(err.to_string()))?;

    let ours = PeerHello {
        ticket: own_ticket,
        protocols: inner
            .protocols
            .iter()
            .map(|protocol| (protocol.id(), protocol.version()))
            .collect(),
    };
    wire::write_msg(&mut send, &PeerMsg::Hello(ours))
        .await
        .map_err(|err| SetupError::Connection(err.to_string()))?;
    let theirs = match wire::read_msg::<PeerMsg>(&mut recv).await {
        Ok(Some(PeerMsg::Hello(hello))) => hello,
        Ok(Some(PeerMsg::TicketUpdate(_)))
        | Err(wire::WireError::Decode(_) | wire::WireError::TooLarge(_)) => {
            return Err(SetupError::Rejected(CloseCode::ProtocolError));
        }
        Ok(None) => return Err(SetupError::Connection("stream ended".into())),
        Err(err) => return Err(SetupError::Connection(err.to_string())),
    };
    let ticket = inner
        .verify_peer(&theirs.ticket, remote)
        .map_err(SetupError::Rejected)?;

    let agreed = inner
        .protocols
        .iter()
        .filter_map(|protocol| {
            let (_, version) = theirs
                .protocols
                .iter()
                .find(|(id, _)| *id == protocol.id())?;
            Some((protocol.clone(), protocol.version().min(*version)))
        })
        .collect();
    Ok(Hello {
        uuid: ticket.uuid,
        name: ticket.name.clone(),
        expires: expiry_instant(&ticket),
        agreed,
        control_send: send,
        control_recv: recv,
    })
}

/// Serves a verified connection until it closes: protocol streams,
/// datagram routing, path state and ticket updates.
async fn serve(inner: Arc<Inner>, conn: Connection, dialed: bool, hello: Hello) {
    let remote = conn.remote_id();
    let Some(status) = register(&inner, &conn, dialed, hello.uuid, &hello.name) else {
        close(&conn, CloseCode::Duplicate);
        return;
    };
    let mut tasks = start_protocols(&conn, dialed, &hello.agreed, &status, &inner.banned).await;
    tasks.spawn(watch_paths(conn.clone(), status));

    peer_control(&inner, &conn, hello).await;
    record_peer_close(&inner, &conn);
    // Protocols end on their own once routing stops and their datagram
    // channel closes; aborting covers any that don't.
    tasks.shutdown().await;
    unregister(&inner, remote, &conn);
}

/// Opens each agreed protocol's control stream, hands the protocol its
/// [`PeerLink`], and routes datagrams to it. Everything runs in the returned
/// tasks.
async fn start_protocols(
    conn: &Connection,
    dialed: bool,
    agreed: &[(Arc<dyn Protocol>, u16)],
    status: &Arc<PeerStatus>,
    bans: &Bans,
) -> JoinSet<()> {
    let mut tasks = JoinSet::new();
    let mut routes = HashMap::new();
    for (protocol, version) in agreed {
        let Ok((send, recv)) = open_protocol_stream(conn, dialed, protocol.id().0).await else {
            close(conn, CloseCode::ProtocolError);
            break;
        };
        let (tx, rx) = mpsc::channel(DATAGRAM_QUEUE);
        routes.insert(protocol.id().0, tx);
        let peer = Peer {
            uuid: status.uuid,
            version: *version,
            protocol: protocol.id(),
            conn: conn.clone(),
            status: status.clone(),
        };
        tasks.spawn(protocol.serve(PeerLink {
            peer,
            datagrams: rx,
            control_send: ControlSend(send),
            control_recv: ControlRecv(recv),
        }));
    }
    tasks.spawn(route_datagrams(conn.clone(), routes, bans.clone()));
    tasks
}

/// Opens (dialer) or accepts (acceptor) one protocol's control stream. Its
/// first byte is the protocol ID.
async fn open_protocol_stream(
    conn: &Connection,
    dialed: bool,
    id: u8,
) -> Result<(SendStream, RecvStream), SetupError> {
    let failed = |err: &dyn std::fmt::Display| SetupError::Connection(err.to_string());
    if dialed {
        let (mut send, recv) = conn.open_bi().await.map_err(|e| failed(&e))?;
        send.write_all(&[id]).await.map_err(|e| failed(&e))?;
        Ok((send, recv))
    } else {
        let (send, mut recv) = conn.accept_bi().await.map_err(|e| failed(&e))?;
        let mut first = [0u8; 1];
        recv.read_exact(&mut first).await.map_err(|e| failed(&e))?;
        if first[0] != id {
            return Err(SetupError::Rejected(CloseCode::ProtocolError));
        }
        Ok((send, recv))
    }
}

/// Records a verified connection in the peer table. Returns `None` if an
/// existing connection to the same peer wins the duplicate tie break.
fn register(
    inner: &Arc<Inner>,
    conn: &Connection,
    dialed: bool,
    uuid: Uuid,
    name: &str,
) -> Option<Arc<PeerStatus>> {
    let remote = conn.remote_id();
    let dialed_by_lower = dialed == (inner.endpoint.id() < remote);
    let mut peers = inner.peers.lock().unwrap();
    let entry = peers
        .entry(remote)
        .or_insert_with(|| PeerEntry::new(uuid, name.to_owned(), &inner.events));
    if let Some(link) = &entry.link
        && entry.open_conn().is_some()
    {
        // Both sides apply the same rule, so they keep the same connection.
        if link.dialed_by_lower && !dialed_by_lower {
            return None;
        }
        close(&link.conn, CloseCode::Duplicate);
    }
    entry.uuid = uuid;
    entry.name = name.to_owned();
    entry.link = Some(Link {
        conn: conn.clone(),
        dialed_by_lower,
    });
    entry.retry_after = None;
    let relayed = is_relayed(conn);
    entry.status.update(|flags| {
        flags.connected = true;
        flags.failed = false;
        flags.relayed = relayed;
    });
    Some(entry.status.clone())
}

/// Forgets a closed connection. A peer the rendezvous still lists stays in
/// the table (its dialer reconnects); any other is gone.
fn unregister(inner: &Arc<Inner>, remote: EndpointId, conn: &Connection) {
    let mut peers = inner.peers.lock().unwrap();
    let Some(entry) = peers.get_mut(&remote) else {
        return;
    };
    let is_current = entry
        .link
        .as_ref()
        .is_some_and(|link| link.conn.stable_id() == conn.stable_id());
    if !is_current {
        return;
    }
    entry.link = None;
    if entry.addr.is_some() {
        entry.status.update(|flags| flags.connected = false);
    } else if let Some(entry) = peers.remove(&remote) {
        entry.remove(CloseCode::Normal);
    }
}

/// Handles the peer control stream after the hello: ticket updates both
/// ways, and closing the connection when the peer's ticket expires.
async fn peer_control(inner: &Arc<Inner>, conn: &Connection, hello: Hello) {
    let remote = conn.remote_id();
    let uuid = hello.uuid;
    let mut expires = hello.expires;
    let mut send = hello.control_send;
    let mut recv = hello.control_recv;

    // Reading a message is not cancel-safe, so it gets its own task.
    let (msg_tx, mut msgs) = mpsc::channel(4);
    let reader = tokio::spawn(async move {
        while let Ok(Some(msg)) = wire::read_msg::<PeerMsg>(&mut recv).await {
            if msg_tx.send(msg).await.is_err() {
                break;
            }
        }
    });
    let mut own_ticket = inner.own_ticket.subscribe();
    own_ticket.mark_unchanged();

    let close_code = loop {
        tokio::select! {
            msg = msgs.recv() => match msg {
                Some(PeerMsg::TicketUpdate(signed)) => match inner.verify_peer(&signed, remote) {
                    Ok(ticket) if ticket.uuid == uuid => expires = expiry_instant(&ticket),
                    Ok(_) => break CloseCode::BadTicket,
                    Err(code) => break code,
                },
                Some(PeerMsg::Hello(_)) => break CloseCode::ProtocolError,
                None => break CloseCode::Normal,
            },
            Ok(()) = own_ticket.changed() => {
                let ticket: Option<SignedTicket> = own_ticket.borrow_and_update().clone();
                if let Some(ticket) = ticket
                    && wire::write_msg(&mut send, &PeerMsg::TicketUpdate(ticket)).await.is_err()
                {
                    break CloseCode::Normal;
                }
            }
            () = tokio::time::sleep_until(expires) => break CloseCode::BadTicket,
            _ = conn.closed() => break CloseCode::Normal,
        }
    };
    reader.abort();
    close(conn, close_code);
}

fn expiry_instant(ticket: &Ticket) -> Instant {
    Instant::now() + ticket.remaining(SystemTime::now())
}

/// Reads datagrams and hands each to the protocol named by its first byte.
async fn route_datagrams(conn: Connection, routes: HashMap<u8, mpsc::Sender<Bytes>>, bans: Bans) {
    let mut guard = FloodGuard::new(Instant::now());
    while let Ok(datagram) = conn.read_datagram().await {
        match guard.check(datagram.len(), Instant::now()) {
            Verdict::Accept => {}
            Verdict::Drop => continue,
            Verdict::Abusive => {
                let until = Instant::now() + FloodGuard::BAN;
                bans.lock().unwrap().insert(conn.remote_id(), until);
                close(&conn, CloseCode::Flooding);
                return;
            }
        }
        let Some(route) = datagram.first().and_then(|id| routes.get(id)) else {
            continue;
        };
        // A full queue means the protocol is behind; dropping is what an
        // unreliable datagram would do anyway.
        let _ = route.try_send(datagram.slice(1..));
    }
}

/// Polls the selected path so the peer's state shows direct or relayed.
async fn watch_paths(conn: Connection, status: Arc<PeerStatus>) {
    let mut interval = tokio::time::interval(PATH_POLL);
    while conn.close_reason().is_none() {
        interval.tick().await;
        let relayed = is_relayed(&conn);
        status.update(|flags| flags.relayed = relayed);
    }
}

/// Remembers why the peer closed `conn`, so tests can check the close code
/// (e.g. that the cap refuses with `TooManyPeers`).
fn record_peer_close(inner: &Arc<Inner>, conn: &Connection) {
    if let Some(ConnectionError::ApplicationClosed(close)) = conn.close_reason() {
        let code = close.error_code.into_inner();
        set_status(inner, conn.remote_id(), |flags| {
            flags.closed_by_peer = Some(code);
        });
    }
}

fn mark_failed(inner: &Arc<Inner>, remote: EndpointId) {
    set_status(inner, remote, |flags| flags.failed = true);
    if let Some(entry) = inner.peers.lock().unwrap().get_mut(&remote) {
        entry.retry_after = Some(Instant::now() + RETRY_AFTER_FAILURE);
    }
}

fn set_status(
    inner: &Arc<Inner>,
    remote: EndpointId,
    change: impl FnOnce(&mut super::StatusFlags),
) {
    let status = inner
        .peers
        .lock()
        .unwrap()
        .get(&remote)
        .map(|entry| entry.status.clone());
    if let Some(status) = status {
        status.update(change);
    }
}

fn open_conn_of(inner: &Arc<Inner>, remote: EndpointId) -> Option<Connection> {
    let peers = inner.peers.lock().unwrap();
    peers
        .get(&remote)
        .and_then(|entry| entry.open_conn().cloned())
}

fn connected_count(inner: &Arc<Inner>) -> usize {
    let peers = inner.peers.lock().unwrap();
    peers
        .values()
        .filter(|entry| entry.open_conn().is_some())
        .count()
}

/// Waits up to `limit` for a verified connection from `remote`.
async fn wait_for_link(inner: &Arc<Inner>, remote: EndpointId, limit: Duration) -> bool {
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        if open_conn_of(inner, remote).is_some() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    open_conn_of(inner, remote).is_some()
}

/// Per-peer datagram limits from DESIGN.md "Security": at most 100
/// datagrams a second and 400 bytes of payload each. Excess is dropped, and
/// a peer that keeps exceeding the rate is closed and banned.
#[derive(Debug)]
pub(crate) struct FloodGuard {
    window_start: Instant,
    in_window: u32,
    /// Consecutive whole seconds over the rate.
    seconds_over: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Verdict {
    Accept,
    Drop,
    Abusive,
}

impl FloodGuard {
    const MAX_PER_SECOND: u32 = 100;
    /// Protocol byte plus 400 bytes.
    const MAX_LEN: usize = 401;
    /// Seconds over the rate in a row that count as abuse.
    const ABUSIVE_SECONDS: u32 = 5;
    const BAN: Duration = Duration::from_secs(10 * 60);

    pub(crate) fn new(now: Instant) -> Self {
        FloodGuard {
            window_start: now,
            in_window: 0,
            seconds_over: 0,
        }
    }

    pub(crate) fn check(&mut self, len: usize, now: Instant) -> Verdict {
        if now.duration_since(self.window_start) >= Duration::from_secs(1) {
            let was_over = self.in_window > Self::MAX_PER_SECOND;
            self.seconds_over = if was_over { self.seconds_over + 1 } else { 0 };
            self.window_start = now;
            self.in_window = 0;
        }
        if self.seconds_over >= Self::ABUSIVE_SECONDS {
            return Verdict::Abusive;
        }
        self.in_window += 1;
        if self.in_window > Self::MAX_PER_SECOND || len > Self::MAX_LEN {
            Verdict::Drop
        } else {
            Verdict::Accept
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use iroh::endpoint::{ConnectionError, VarInt};
    use yakvc_shared::{IssuerKey, TicketVerifier};

    use super::super::test_util::{
        connect, eventually, loopback_addr, loopback_endpoint, probe, ticket, uuid,
    };
    use super::*;
    use crate::event::{self, Event, Events, PeerState};
    use crate::net::{Identity, MAX_PEERS, Net, Trust};

    fn status(uuid: Uuid) -> (Arc<PeerStatus>, Events) {
        let (events, rx) = event::channel();
        let status = PeerStatus {
            uuid,
            events,
            flags: Default::default(),
        };
        (Arc::new(status), rx)
    }

    fn closed_with(reason: ConnectionError, code: CloseCode) -> bool {
        matches!(reason, ConnectionError::ApplicationClosed(close)
            if close.error_code == VarInt::from_u32(code as u32))
    }

    #[tokio::test]
    async fn datagrams_and_control_streams_reach_the_named_protocol() {
        let (a, b) = (loopback_endpoint().await, loopback_endpoint().await);
        let (conn_a, conn_b) = connect(&a, &b).await;
        let (voice_a, mut voice_links_a) = probe(1, 1);
        let (other_a, mut other_links_a) = probe(2, 1);
        let (voice_b, mut voice_links_b) = probe(1, 1);
        let (other_b, mut other_links_b) = probe(2, 1);
        let (status_a, _events_a) = status(uuid(2));
        let (status_b, _events_b) = status(uuid(1));
        let bans = Bans::default();
        let agreed_a = vec![(voice_a, 1), (other_a, 3)];
        let agreed_b = vec![(voice_b, 1), (other_b, 3)];
        let (_tasks_a, _tasks_b) = tokio::join!(
            start_protocols(&conn_a, true, &agreed_a, &status_a, &bans),
            start_protocols(&conn_b, false, &agreed_b, &status_b, &bans),
        );
        let mut voice_a = voice_links_a.recv().await.unwrap();
        let mut other_a = other_links_a.recv().await.unwrap();
        let mut voice_b = voice_links_b.recv().await.unwrap();
        let mut other_b = other_links_b.recv().await.unwrap();
        assert_eq!(voice_b.peer.uuid(), uuid(1));
        assert_eq!(other_b.peer.version(), 3);

        voice_a.peer.send_datagram(b"voice").unwrap();
        other_a.peer.send_datagram(b"other").unwrap();
        assert_eq!(&voice_b.datagrams.recv().await.unwrap()[..], b"voice");
        assert_eq!(&other_b.datagrams.recv().await.unwrap()[..], b"other");

        // Unknown protocols and oversized datagrams are dropped.
        conn_a
            .send_datagram(Bytes::from_static(&[9, 1, 2, 3]))
            .unwrap();
        conn_a.send_datagram(Bytes::from(vec![1; 402])).unwrap();
        voice_a.peer.send_datagram(b"after").unwrap();
        assert_eq!(&voice_b.datagrams.recv().await.unwrap()[..], b"after");

        // Control streams pair up by protocol, in both directions.
        let mut byte = [0u8; 1];
        voice_a.control_send.0.write_all(b"v").await.unwrap();
        voice_b.control_recv.0.read_exact(&mut byte).await.unwrap();
        assert_eq!(&byte, b"v");
        other_b.control_send.0.write_all(b"o").await.unwrap();
        other_a.control_recv.0.read_exact(&mut byte).await.unwrap();
        assert_eq!(&byte, b"o");

        // Closing the connection ends every protocol's datagram stream.
        conn_a.close(VarInt::from_u32(0), b"");
        assert_eq!(voice_b.datagrams.recv().await, None);
        assert_eq!(other_b.datagrams.recv().await, None);
    }

    /// A [`Net`] on a loopback endpoint, with no protocols.
    async fn net_without_handshake() -> (Net, Events) {
        let (events_tx, events) = event::channel();
        let trust = Trust {
            verifier: TicketVerifier::new([]),
            direct_calls: false,
        };
        let net = Net::new(
            loopback_endpoint().await,
            vec![],
            trust,
            events_tx,
            MAX_PEERS,
        );
        (net, events)
    }

    #[tokio::test]
    async fn duplicate_connections_keep_the_one_dialed_by_the_lower_id() {
        let (net, mut events) = net_without_handshake().await;
        let inner = net.inner();
        let a = &inner.endpoint;
        let b = loopback_endpoint().await;
        let a_is_lower = a.id() < b.id();
        let (first, first_b) = connect(a, &b).await;
        let (second, second_b) = connect(a, &b).await;

        // Pretend the first was dialed by the lower id and the second wasn't.
        assert!(register(inner, &first, a_is_lower, uuid(2), "bob").is_some());
        assert!(register(inner, &second, !a_is_lower, uuid(2), "bob").is_none());
        assert_eq!(
            events.try_next(),
            Some(Event::Peer {
                uuid: uuid(2),
                state: PeerState::Direct
            })
        );
        assert!(first.close_reason().is_none());
        drop((second, second_b));

        // A newer connection that was also dialed by the lower id replaces
        // the old one, which is closed as a duplicate.
        let (third, _third_b) = connect(a, &b).await;
        assert!(register(inner, &third, a_is_lower, uuid(2), "bob").is_some());
        assert!(closed_with(first_b.closed().await, CloseCode::Duplicate));

        // The stale connection ending doesn't remove the peer; the current
        // one ending does.
        unregister(inner, b.id(), &first);
        assert_eq!(net.peers().len(), 1);
        unregister(inner, b.id(), &third);
        assert!(net.peers().is_empty());
        assert_eq!(
            events.try_next(),
            Some(Event::Peer {
                uuid: uuid(2),
                state: PeerState::Gone
            })
        );
    }

    #[tokio::test]
    async fn leaving_the_tab_list_closes_the_connection() {
        let (net, _events) = net_without_handshake().await;
        let inner = net.inner();
        let b = loopback_endpoint().await;
        let (conn, conn_b) = connect(&inner.endpoint, &b).await;
        net.set_tab_list(HashSet::from([uuid(2)]));
        register(inner, &conn, true, uuid(2), "bob").unwrap();

        net.set_tab_list(HashSet::from([uuid(3)]));
        assert!(closed_with(conn_b.closed().await, CloseCode::NotVisible));
    }

    #[tokio::test]
    async fn peer_gone_closes_and_reports() {
        let (net, mut events) = net_without_handshake().await;
        let inner = net.inner();
        let b = loopback_endpoint().await;
        let (conn, conn_b) = connect(&inner.endpoint, &b).await;
        register(inner, &conn, true, uuid(2), "bob").unwrap();
        let _connected = events.try_next();

        inner.peer_gone(b.id());
        assert!(closed_with(conn_b.closed().await, CloseCode::Normal));
        assert_eq!(
            events.try_next(),
            Some(Event::Peer {
                uuid: uuid(2),
                state: PeerState::Gone
            })
        );
    }

    /// One side of a handshake test: a [`Net`] with a probe protocol, an
    /// identity and a ticket from `issuer`, trusting `trusted`.
    struct Side {
        net: Net,
        events: Events,
        _links: tokio::sync::mpsc::UnboundedReceiver<PeerLink>,
    }

    async fn side(n: u8, issuer: &IssuerKey, trusted: &IssuerKey) -> Side {
        let endpoint = loopback_endpoint().await;
        let (events_tx, events) = event::channel();
        let (probe, links) = probe(1, 1);
        let trust = Trust {
            verifier: TicketVerifier::new([trusted.id()]),
            direct_calls: false,
        };
        let net = Net::new(endpoint.clone(), vec![probe], trust, events_tx, MAX_PEERS);
        net.set_identity(Identity {
            uuid: uuid(n),
            name: format!("player{n}"),
        });
        let own = ticket(issuer, uuid(n), &endpoint);
        net.inner().own_ticket.send_replace(Some(own));
        Side {
            net,
            events,
            _links: links,
        }
    }

    impl Side {
        fn sees(&self, players: &[u8]) {
            self.net
                .set_tab_list(players.iter().map(|&n| uuid(n)).collect());
        }

        fn addr(&self) -> EndpointAddr {
            loopback_addr(&self.net.inner().endpoint)
        }

        async fn dial(&self, to: &Side) -> Result<(), SetupError> {
            dial(self.net.inner(), to.addr()).await
        }

        async fn next_peer_state(&mut self) -> (Uuid, PeerState) {
            let next = async {
                loop {
                    if let Some(Event::Peer { uuid, state }) = self.events.next().await {
                        return (uuid, state);
                    }
                }
            };
            tokio::time::timeout(Duration::from_secs(5), next)
                .await
                .expect("no peer event")
        }
    }

    #[tokio::test]
    async fn peers_with_trusted_tickets_connect() {
        let issuer = IssuerKey::generate();
        let mut alice = side(1, &issuer, &issuer).await;
        let mut bob = side(2, &issuer, &issuer).await;
        alice.sees(&[2]);
        bob.sees(&[1]);

        alice.dial(&bob).await.unwrap();
        assert_eq!(alice.next_peer_state().await, (uuid(2), PeerState::Direct));
        assert_eq!(bob.next_peer_state().await, (uuid(1), PeerState::Direct));
        eventually("both sides list the peer", || {
            alice.net.peers().len() == 1 && bob.net.peers().len() == 1
        })
        .await;
        assert_eq!(alice.net.peers()[0].name, "player2");
    }

    #[tokio::test]
    async fn untrusted_issuer_is_rejected() {
        let issuer = IssuerKey::generate();
        let rogue = IssuerKey::generate();
        let alice = side(1, &issuer, &issuer).await;
        let bob = side(2, &rogue, &issuer).await;
        alice.sees(&[2]);
        bob.sees(&[1]);

        let result = alice.dial(&bob).await;
        assert!(matches!(
            result,
            Err(SetupError::Rejected(CloseCode::BadTicket))
        ));
        assert!(alice.net.peers().is_empty());
    }

    #[tokio::test]
    async fn ticket_for_another_endpoint_is_rejected() {
        let issuer = IssuerKey::generate();
        let alice = side(1, &issuer, &issuer).await;
        let bob = side(2, &issuer, &issuer).await;
        // Bob presents a valid ticket that names someone else's endpoint.
        let stolen = ticket(&issuer, uuid(2), &loopback_endpoint().await);
        bob.net.inner().own_ticket.send_replace(Some(stolen));
        alice.sees(&[2]);
        bob.sees(&[1]);

        let result = alice.dial(&bob).await;
        assert!(matches!(
            result,
            Err(SetupError::Rejected(CloseCode::BadTicket))
        ));
    }

    #[tokio::test]
    async fn peer_outside_the_tab_list_is_rejected() {
        let issuer = IssuerKey::generate();
        let alice = side(1, &issuer, &issuer).await;
        let bob = side(2, &issuer, &issuer).await;
        alice.sees(&[3]);
        bob.sees(&[1]);

        let result = alice.dial(&bob).await;
        assert!(matches!(
            result,
            Err(SetupError::Rejected(CloseCode::NotVisible))
        ));
    }

    #[tokio::test]
    async fn the_accepting_side_checks_too() {
        let issuer = IssuerKey::generate();
        let alice = side(1, &issuer, &issuer).await;
        let mut bob = side(2, &issuer, &issuer).await;
        alice.sees(&[2]);
        // Bob can't see Alice, so he closes the connection Alice opened.
        bob.sees(&[3]);

        let _ = alice.dial(&bob).await;
        eventually("alice drops the connection", || {
            alice
                .net
                .peers()
                .iter()
                .all(|peer| peer.state != PeerState::Direct)
        })
        .await;
        let no_event = tokio::time::timeout(Duration::from_millis(300), bob.next_peer_state());
        assert!(no_event.await.is_err(), "bob never accepted alice");
    }

    #[test]
    fn flood_guard_drops_oversized_and_excess_datagrams() {
        let start = Instant::now();
        let mut guard = FloodGuard::new(start);
        assert_eq!(guard.check(401, start), Verdict::Accept);
        assert_eq!(guard.check(402, start), Verdict::Drop);
        for _ in 0..99 {
            guard.check(100, start);
        }
        assert_eq!(guard.check(100, start), Verdict::Drop);
        // A new second starts a new allowance.
        let later = start + Duration::from_secs(1);
        assert_eq!(guard.check(100, later), Verdict::Accept);
    }

    #[test]
    fn flood_guard_flags_sustained_flooding() {
        let start = Instant::now();
        let mut guard = FloodGuard::new(start);
        let mut verdict = Verdict::Accept;
        for second in 0..10 {
            let now = start + Duration::from_secs(second);
            for _ in 0..200 {
                verdict = guard.check(100, now);
            }
            if verdict == Verdict::Abusive {
                assert_eq!(second, 5);
                return;
            }
        }
        panic!("never flagged, last verdict {verdict:?}");
    }

    #[test]
    fn flood_guard_tolerates_a_steady_voice_stream() {
        let start = Instant::now();
        let mut guard = FloodGuard::new(start);
        for frame in 0..50 * 60 {
            let now = start + Duration::from_millis(20 * frame);
            assert_eq!(guard.check(80, now), Verdict::Accept);
        }
    }
}
