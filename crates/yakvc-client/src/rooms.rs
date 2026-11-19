//! The client side of `yakvc/rooms/1`, the server's public room list.
//!
//! [`RoomList`] is one connection: it keeps an up-to-date copy of the list
//! and announces rooms, which stay listed until withdrawn or until the
//! `RoomList` is dropped or closed. It needs no sign-in, only an Iroh
//! endpoint (any ALPNs) to dial from.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iroh::endpoint::{ConnectError, Connection, ConnectionError, RecvStream, SendStream};
use iroh::{Endpoint, EndpointAddr};
use tokio::sync::{oneshot, watch};
use tokio::task::JoinHandle;
use yakvc_proto::rooms::{ALPN, ClientMsg, CloseCode, ServerMsg};
pub use yakvc_proto::rooms::{Refusal, Room};
use yakvc_proto::wire::{self, WireError};

use crate::config::built_in_rendezvous;

/// How long `connect` waits for the connection and the whole list.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// The built-in server's address, the one the voice client uses when its
/// config names none. Reached through its relay.
pub fn default_server() -> EndpointAddr {
    let (rendezvous, _issuer) = built_in_rendezvous();
    let mut addr = EndpointAddr::new(rendezvous.endpoint_id);
    if let Some(relay) = rendezvous.relay {
        addr = addr.with_relay_url(relay);
    }
    addr
}

#[derive(Debug, thiserror::Error)]
pub enum RoomsError {
    #[error("connect: {0}")]
    Connect(#[from] ConnectError),
    #[error("connection: {0}")]
    Connection(#[from] ConnectionError),
    #[error(transparent)]
    Wire(#[from] WireError),
    #[error("timed out")]
    TimedOut,
    #[error("refused: {0}")]
    Refused(Refusal),
    /// The connection to the server is gone.
    #[error("room list connection closed")]
    Closed,
}

/// One connection to a server's room list.
#[derive(Debug)]
pub struct RoomList {
    conn: Connection,
    send: tokio::sync::Mutex<SendStream>,
    rooms: watch::Receiver<Vec<Room>>,
    pending: Arc<Pending>,
    reader: JoinHandle<()>,
}

/// Announces waiting for the server's answer, oldest first per code.
type Pending = Mutex<HashMap<String, VecDeque<oneshot::Sender<Result<(), Refusal>>>>>;

impl RoomList {
    /// Connects and waits for the whole list. `endpoint` must stay open for
    /// as long as the list is used.
    pub async fn connect(
        endpoint: &Endpoint,
        server: EndpointAddr,
    ) -> Result<RoomList, RoomsError> {
        tokio::time::timeout(CONNECT_TIMEOUT, RoomList::open(endpoint, server))
            .await
            .map_err(|_| RoomsError::TimedOut)?
    }

    async fn open(endpoint: &Endpoint, server: EndpointAddr) -> Result<RoomList, RoomsError> {
        let conn = endpoint.connect(server, ALPN).await?;
        let (mut send, mut recv) = conn.open_bi().await?;
        wire::write_msg(&mut send, &ClientMsg::Watch).await?;

        let mut listed = BTreeMap::new();
        loop {
            match wire::read_msg(&mut recv).await? {
                Some(ServerMsg::Added(room)) => {
                    listed.insert(room.code, room.label);
                }
                Some(ServerMsg::Listed) => break,
                // Nothing of ours is announced yet.
                Some(_) => {}
                None => return Err(RoomsError::Closed),
            }
        }
        let (tx_rooms, rooms) = watch::channel(to_vec(&listed));
        let pending = Arc::new(Pending::default());
        let reader = tokio::spawn(read_all(recv, listed, tx_rooms, pending.clone()));
        Ok(RoomList {
            conn,
            send: tokio::sync::Mutex::new(send),
            rooms,
            pending,
            reader,
        })
    }

    /// The list as it is now, by code.
    pub fn rooms(&self) -> Vec<Room> {
        self.rooms.borrow().clone()
    }

    /// The list, changing as rooms come and go. `changed` fails once the
    /// connection is gone.
    pub fn watch(&self) -> watch::Receiver<Vec<Room>> {
        self.rooms.clone()
    }

    /// Lists `room` for as long as this connection lasts, or relabels it if
    /// this connection listed its code already.
    pub async fn announce(&self, room: Room) -> Result<(), RoomsError> {
        let (tx_answer, rx_answer) = oneshot::channel();
        self.pending
            .lock()
            .unwrap()
            .entry(room.code.clone())
            .or_default()
            .push_back(tx_answer);
        self.send(ClientMsg::Announce(room)).await?;
        match rx_answer.await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(reason)) => Err(RoomsError::Refused(reason)),
            Err(_) => Err(RoomsError::Closed),
        }
    }

    /// Unlists the room with `code`, if this connection listed it.
    pub async fn withdraw(&self, code: &str) -> Result<(), RoomsError> {
        self.send(ClientMsg::Withdraw(code.to_owned())).await
    }

    /// Unlists every room of ours and closes the connection.
    pub async fn close(self) {
        let _ = self.send.lock().await.finish();
        self.conn.close((CloseCode::Normal as u32).into(), b"");
    }

    async fn send(&self, msg: ClientMsg) -> Result<(), RoomsError> {
        let mut send = self.send.lock().await;
        wire::write_msg(&mut *send, &msg).await?;
        Ok(())
    }
}

impl Drop for RoomList {
    fn drop(&mut self) {
        self.reader.abort();
        self.conn.close((CloseCode::Normal as u32).into(), b"");
    }
}

/// Applies list changes and answers announces until the stream ends.
async fn read_all(
    mut recv: RecvStream,
    mut listed: BTreeMap<String, String>,
    tx_rooms: watch::Sender<Vec<Room>>,
    pending: Arc<Pending>,
) {
    let answer = |code: &str, result: Result<(), Refusal>| {
        let mut pending = pending.lock().unwrap();
        let Some(waiting) = pending.get_mut(code) else {
            return;
        };
        if let Some(tx_answer) = waiting.pop_front() {
            let _ = tx_answer.send(result);
        }
        if waiting.is_empty() {
            pending.remove(code);
        }
    };
    while let Ok(Some(msg)) = wire::read_msg(&mut recv).await {
        match msg {
            ServerMsg::Added(room) => {
                listed.insert(room.code, room.label);
                tx_rooms.send_replace(to_vec(&listed));
            }
            ServerMsg::Gone(code) => {
                listed.remove(&code);
                tx_rooms.send_replace(to_vec(&listed));
            }
            ServerMsg::Announced(code) => answer(&code, Ok(())),
            ServerMsg::Refused { code, reason } => answer(&code, Err(reason)),
            ServerMsg::Listed => {}
        }
    }
    // Waiting announces see their sender dropped, and watchers the list's.
    pending.lock().unwrap().clear();
}

fn to_vec(listed: &BTreeMap<String, String>) -> Vec<Room> {
    listed
        .iter()
        .map(|(code, label)| Room {
            label: label.clone(),
            code: code.clone(),
        })
        .collect()
}
