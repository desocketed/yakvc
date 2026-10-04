//! Live rendezvous sessions: the [`Matcher`] plus a way to reach each client.

use std::collections::HashMap;
use std::sync::Mutex;

use iroh::endpoint::Connection;
use tokio::sync::mpsc;
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
}

#[derive(Debug)]
struct Client {
    conn: Connection,
    /// Messages for the connection's writer task.
    outbox: mpsc::UnboundedSender<ServerMsg>,
}

impl Sessions {
    /// Starts a session. An older session for the same EndpointId is closed,
    /// since its connection is most likely already dead.
    pub fn register(
        &self,
        conn: &Connection,
        uuid: Uuid,
        ticket: SignedTicket,
        addr: EndpointAddr,
        outbox: mpsc::UnboundedSender<ServerMsg>,
    ) {
        let id = conn.remote_id();
        let mut inner = self.0.lock().unwrap();
        let client = Client {
            conn: conn.clone(),
            outbox,
        };
        if let Some(old) = inner.clients.insert(id, client) {
            close(&old.conn, CloseCode::Normal, "replaced by a newer session");
        }
        let notices = inner.matcher.register(id, uuid, ticket, addr);
        inner.deliver(notices);
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

    pub fn pair_count(&self, id: EndpointId) -> usize {
        self.0.lock().unwrap().matcher.pair_count(id)
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

    pub fn update_ticket(&self, id: EndpointId, ticket: SignedTicket) {
        self.0.lock().unwrap().matcher.update_ticket(id, ticket);
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
    fn deliver(&self, notices: Vec<Notice>) {
        for notice in notices {
            if let Some(client) = self.clients.get(&notice.to) {
                // A send only fails while that connection is shutting down.
                let _ = client.outbox.send(notice.msg);
            }
        }
    }
}

pub(crate) fn close(conn: &Connection, code: CloseCode, reason: &str) {
    conn.close((code as u32).into(), reason.as_bytes());
}
