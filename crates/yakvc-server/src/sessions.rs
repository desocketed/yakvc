//! Live rendezvous sessions: the [`Matcher`] plus a way to reach each client.

use std::collections::HashMap;
use std::sync::Mutex;

use iroh::endpoint::Connection;
use tokio::sync::mpsc;
use tokio::sync::mpsc::error::TrySendError;
use yakvc_shared::rdv::{CloseCode, ServerMsg};
use yakvc_shared::{EndpointAddr, EndpointId, PairToken, SignedTicket, Uuid};

use crate::matcher::{Matcher, Notice};
use crate::server::Stats;

#[derive(Debug, Default)]
pub(crate) struct Sessions(Mutex<Inner>);

#[derive(Debug, Default)]
struct Inner {
    matcher: Matcher,
    clients: HashMap<EndpointId, Client>,
    next_seq: u64,
}

/// Most sessions one UUID may hold at once; a new one closes the oldest.
const MAX_SESSIONS_PER_UUID: usize = 2;

#[derive(Debug)]
struct Client {
    conn: Connection,
    /// Messages for the connection's writer task.
    outbox: Outbox,
    uuid: Uuid,
    /// Whether the current ticket is verified.
    verified: bool,
    /// Registration order, to find the oldest session for a UUID.
    seq: u64,
    /// Closed by another session and out of the matcher, waiting for its
    /// connection's task to unregister it.
    ended: bool,
}

impl Sessions {
    /// Starts a session, unless the ticket is unverified and a verified
    /// session holds the same UUID (returns false). A verified session
    /// closes every unverified session for its UUID (see
    /// [`Inner::claim`]), and the oldest sessions for it are closed beyond
    /// [`MAX_SESSIONS_PER_UUID`]. An older session for the same EndpointId
    /// is closed, since its connection is most likely already dead.
    pub fn register(
        &self,
        conn: &Connection,
        uuid: Uuid,
        verified: bool,
        ticket: SignedTicket,
        addr: EndpointAddr,
        outbox: Outbox,
    ) -> bool {
        let id = conn.remote_id();
        let mut inner = self.0.lock().unwrap();
        if !inner.claim(id, uuid, verified) {
            return false;
        }
        inner.make_room(id, uuid);
        inner.next_seq += 1;
        let client = Client {
            conn: conn.clone(),
            outbox,
            uuid,
            verified,
            seq: inner.next_seq,
            ended: false,
        };
        if let Some(old) = inner.clients.insert(id, client) {
            close(&old.conn, CloseCode::Normal, "replaced by a newer session");
        }
        let notices = inner.matcher.register(id, uuid, ticket, addr);
        inner.deliver(notices);
        true
    }

    /// Ends the session `conn` started, unless a newer session replaced it.
    /// Returns whether a session ended.
    pub fn unregister(&self, conn: &Connection) -> bool {
        let id = conn.remote_id();
        let mut inner = self.0.lock().unwrap();
        let ours = inner
            .clients
            .get(&id)
            .is_some_and(|c| c.conn.stable_id() == conn.stable_id());
        if !ours {
            return false;
        }
        inner.clients.remove(&id);
        let notices = inner.matcher.unregister(id);
        inner.deliver(notices);
        true
    }

    pub fn contains(&self, id: EndpointId) -> bool {
        self.0.lock().unwrap().clients.contains_key(&id)
    }

    pub fn pair_count_after_adding(&self, id: EndpointId, tokens: &[PairToken]) -> usize {
        self.0
            .lock()
            .unwrap()
            .matcher
            .pair_count_after_adding(id, tokens)
    }

    pub fn set_pairs(&self, id: EndpointId, tokens: Vec<PairToken>) {
        let mut inner = self.0.lock().unwrap();
        let notices = inner.matcher.set_pairs(id, tokens);
        inner.deliver(notices);
    }

    pub fn add_pairs(&self, id: EndpointId, tokens: Vec<PairToken>) {
        let mut inner = self.0.lock().unwrap();
        let notices = inner.matcher.add_pairs(id, tokens);
        inner.deliver(notices);
    }

    pub fn remove_pairs(&self, id: EndpointId, tokens: Vec<PairToken>) {
        let mut inner = self.0.lock().unwrap();
        let notices = inner.matcher.remove_pairs(id, tokens);
        inner.deliver(notices);
    }

    pub fn update_addr(&self, id: EndpointId, addr: EndpointAddr) {
        let mut inner = self.0.lock().unwrap();
        let notices = inner.matcher.update_addr(id, addr);
        inner.deliver(notices);
    }

    /// Records a renewed ticket, with the same priority rules as
    /// [`register`](Self::register). Returns false, keeping the old ticket,
    /// if the session is gone or the new ticket is refused.
    pub fn update_ticket(&self, id: EndpointId, ticket: SignedTicket, verified: bool) -> bool {
        let mut inner = self.0.lock().unwrap();
        let Some(uuid) = inner.clients.get(&id).map(|client| client.uuid) else {
            return false;
        };
        if !inner.claim(id, uuid, verified) {
            return false;
        }
        inner.clients.get_mut(&id).expect("checked above").verified = verified;
        inner.matcher.update_ticket(id, ticket);
        true
    }

    pub fn stats(&self) -> Stats {
        let inner = self.0.lock().unwrap();
        Stats {
            sessions: inner.matcher.sessions(),
            matches: inner.matcher.matches(),
        }
    }

    pub fn close_all(&self, code: CloseCode, reason: &str) {
        for client in self.0.lock().unwrap().clients.values() {
            close(&client.conn, code, reason);
        }
    }
}

impl Inner {
    /// Verified priority, for session `id` claiming `uuid`. An unverified
    /// claim is refused while a verified session holds the UUID. A verified
    /// claim ends every unverified session for it: their connections close
    /// with `Superseded` and their matches get `PeerGone` straight away.
    fn claim(&mut self, id: EndpointId, uuid: Uuid, verified: bool) -> bool {
        let mut others = self.others_for(id, uuid);
        if !verified {
            return !others.any(|(_, client)| client.verified);
        }
        let superseded: Vec<EndpointId> = others
            .filter(|(_, client)| !client.verified)
            .map(|(other, _)| other)
            .collect();
        for other in superseded {
            self.end(other, "a verified player holds this identity");
        }
        true
    }

    /// Ends the oldest other sessions for `uuid` until session `id` fits
    /// within [`MAX_SESSIONS_PER_UUID`]. Each session costs matching work,
    /// so one account must not hold many; two allow for a second game
    /// instance.
    fn make_room(&mut self, id: EndpointId, uuid: Uuid) {
        let mut others: Vec<(u64, EndpointId)> = self
            .others_for(id, uuid)
            .map(|(other, client)| (client.seq, other))
            .collect();
        others.sort();
        let excess = (others.len() + 1).saturating_sub(MAX_SESSIONS_PER_UUID);
        for &(_, oldest) in &others[..excess] {
            self.end(oldest, "a newer session took over this identity");
        }
    }

    /// The live sessions for `uuid` other than `id`.
    fn others_for(
        &self,
        id: EndpointId,
        uuid: Uuid,
    ) -> impl Iterator<Item = (EndpointId, &Client)> {
        self.clients
            .iter()
            .filter(move |&(&other, client)| other != id && client.uuid == uuid && !client.ended)
            .map(|(&other, client)| (other, client))
    }

    /// Closes session `id` with `Superseded`; its matches get `PeerGone`
    /// straight away. The entry stays until its connection's task
    /// unregisters it, so the relay gate hears about the session ending as
    /// usual.
    fn end(&mut self, id: EndpointId, reason: &str) {
        let client = self.clients.get_mut(&id).expect("ending a known session");
        client.ended = true;
        close(&client.conn, CloseCode::Superseded, reason);
        let notices = self.matcher.unregister(id);
        self.deliver(notices);
    }

    fn deliver(&self, notices: Vec<Notice>) {
        for notice in notices {
            if let Some(client) = self.clients.get(&notice.to) {
                push(&client.conn, &client.outbox, notice.msg);
            }
        }
    }
}

/// Messages for a connection's writer task.
pub(crate) type Outbox = mpsc::Sender<ServerMsg>;

/// Queues `msg` for `conn`'s writer. A full outbox means the client stopped
/// reading; it is closed rather than queued for without limit.
pub(crate) fn push(conn: &Connection, outbox: &Outbox, msg: ServerMsg) {
    // A closed outbox only happens while that connection is shutting down.
    if let Err(TrySendError::Full(_)) = outbox.try_send(msg) {
        close(conn, CloseCode::LimitExceeded, "not reading");
    }
}

pub(crate) fn close(conn: &Connection, code: CloseCode, reason: &str) {
    conn.close((code as u32).into(), reason.as_bytes());
}
