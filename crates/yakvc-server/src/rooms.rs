//! Serves `yakvc/rooms/1`, the public room list: no sign-in, so every limit
//! is in [`RoomOptions`].
//!
//! A connection gets the list once it sends `Watch`. Its rooms live in the [`Board`] until
//! it withdraws them or goes away. Replies and list changes go through one
//! writer task per connection, as on the rendezvous.

use std::collections::{BTreeMap, HashMap};
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use iroh::endpoint::{Connection, SendStream};
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio::sync::mpsc::error::TrySendError;
use yakvc_proto::rooms::{ClientMsg, CloseCode, Refusal, Room, ServerMsg};
use yakvc_proto::wire::{WireError, read_msg_max, write_msg};

use crate::limits::{IpSlots, RateLimiter, Slots, TokenBucket, ip_key};

/// How long a new connection has to open its stream.
const STREAM_TIMEOUT: Duration = Duration::from_secs(10);

/// How long one message may wait for the client to make room for it.
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);

/// How long the writer may take to deliver its last messages on close.
const FLUSH_TIMEOUT: Duration = Duration::from_secs(1);

/// The room list's limits, the `[rooms]` section of the config. The list is
/// off unless the section is there, and `enabled = false` turns it off
/// again.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RoomOptions {
    pub enabled: bool,
    /// Rooms in the whole list.
    pub max_rooms: usize,
    /// Rooms one connection may list.
    pub max_rooms_per_connection: usize,
    /// Rooms listed from one source IP (IPv6: per /64).
    pub max_rooms_per_ip: usize,
    /// Rooms listed by every connection without a known source IP (those
    /// arriving through the relay), together.
    pub max_rooms_unknown_ip: usize,
    /// Connections open at once, listing or only watching.
    pub max_connections: usize,
    /// The same, per source IP (IPv6: per /64). Connections through the
    /// relay only count against `max_connections`.
    pub max_connections_per_ip: usize,
    /// Longest label, in bytes.
    pub max_label_len: usize,
    /// Longest code, in bytes.
    pub max_code_len: usize,
    /// Announces per minute from one source IP, in bursts of up to as many.
    pub announces_per_ip_per_min: u32,
    /// Announces per minute shared by every connection without a known
    /// source IP.
    pub announces_unknown_ip_per_min: u32,
}

impl Default for RoomOptions {
    fn default() -> Self {
        RoomOptions {
            enabled: true,
            max_rooms: 1024,
            max_rooms_per_connection: 4,
            max_rooms_per_ip: 16,
            max_rooms_unknown_ip: 256,
            max_connections: 4096,
            max_connections_per_ip: 32,
            max_label_len: 64,
            // An Iroh endpoint address with a few IPs and a relay URL, in
            // base32, is a few hundred bytes.
            max_code_len: 1024,
            announces_per_ip_per_min: 20,
            announces_unknown_ip_per_min: 120,
        }
    }
}

/// The listed rooms and who watches them.
#[derive(Debug)]
pub(crate) struct Board {
    options: RoomOptions,
    connections: Slots,
    connections_per_ip: IpSlots,
    announces_per_ip: Mutex<RateLimiter<IpAddr>>,
    announces_unknown_ip: Mutex<TokenBucket>,
    inner: Mutex<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    /// By code.
    rooms: BTreeMap<String, Listing>,
    watchers: HashMap<u64, Watcher>,
    next_id: u64,
}

#[derive(Debug)]
struct Listing {
    label: String,
    /// The watcher that listed it.
    owner: u64,
    /// The owner's source, under `ip_key`.
    source: Option<IpAddr>,
}

#[derive(Debug)]
struct Watcher {
    conn: Connection,
    outbox: mpsc::Sender<ServerMsg>,
    /// Sent `Watch`, so it gets list changes.
    watching: bool,
}

impl Board {
    pub fn new(options: RoomOptions) -> Self {
        let minute = Duration::from_secs(60);
        Board {
            connections: Slots::new(options.max_connections),
            connections_per_ip: IpSlots::new(options.max_connections_per_ip),
            announces_per_ip: Mutex::new(RateLimiter::new(
                options.announces_per_ip_per_min,
                minute,
            )),
            announces_unknown_ip: Mutex::new(TokenBucket::new(
                options.announces_unknown_ip_per_min,
                minute,
                Instant::now(),
            )),
            inner: Mutex::default(),
            options,
        }
    }

    pub fn prune(&self, now: Instant) {
        self.announces_per_ip.lock().unwrap().prune(now);
    }

    pub fn close_all(&self) {
        for watcher in self.inner.lock().unwrap().watchers.values() {
            close(&watcher.conn, CloseCode::ShuttingDown);
        }
    }

    /// Adds a connection, not yet watching.
    fn join(&self, conn: &Connection, outbox: mpsc::Sender<ServerMsg>) -> u64 {
        let mut inner = self.inner.lock().unwrap();
        let id = inner.next_id;
        inner.next_id += 1;
        inner.watchers.insert(
            id,
            Watcher {
                conn: conn.clone(),
                outbox,
                watching: false,
            },
        );
        id
    }

    /// Queues the whole list for connection `id` and its changes from now on.
    fn watch(&self, id: u64) {
        let mut inner = self.inner.lock().unwrap();
        let Inner {
            rooms, watchers, ..
        } = &mut *inner;
        let Some(watcher) = watchers.get_mut(&id) else {
            return;
        };
        if watcher.watching {
            return;
        }
        watcher.watching = true;
        for (code, listing) in rooms.iter() {
            push(
                &watcher.conn,
                &watcher.outbox,
                ServerMsg::Added(listing.room(code)),
            );
        }
        push(&watcher.conn, &watcher.outbox, ServerMsg::Listed);
    }

    /// Lists `room` for watcher `id`, or says why not.
    fn announce(&self, id: u64, ip: Option<IpAddr>, room: &Room) -> Result<(), Refusal> {
        let options = &self.options;
        let label_ok = !room.label.is_empty()
            && room.label.len() <= options.max_label_len
            && !room.label.chars().any(char::is_control);
        if !label_ok || room.code.is_empty() || room.code.len() > options.max_code_len {
            return Err(Refusal::Invalid);
        }
        let source = ip.map(ip_key);
        let mut inner = self.inner.lock().unwrap();
        let relabel = match inner.rooms.get(&room.code) {
            Some(listing) if listing.owner == id => true,
            Some(_) => return Err(Refusal::Taken),
            None => false,
        };
        if !relabel {
            if inner.rooms.len() >= options.max_rooms {
                return Err(Refusal::ListFull);
            }
            let mine = inner.rooms.values().filter(|listing| listing.owner == id);
            if mine.count() >= options.max_rooms_per_connection {
                return Err(Refusal::TooManyRooms);
            }
            let max_from_source = match source {
                Some(_) => options.max_rooms_per_ip,
                None => options.max_rooms_unknown_ip,
            };
            let from_source = inner
                .rooms
                .values()
                .filter(|listing| listing.source == source);
            if from_source.count() >= max_from_source {
                return Err(Refusal::TooManyRooms);
            }
        }
        // Charged last, so a refused announce costs nothing.
        let now = Instant::now();
        let allowed = match source {
            Some(source) => self.announces_per_ip.lock().unwrap().allow(source, now),
            None => self.announces_unknown_ip.lock().unwrap().take(now),
        };
        if !allowed {
            return Err(Refusal::RateLimited);
        }
        inner.rooms.insert(
            room.code.clone(),
            Listing {
                label: room.label.clone(),
                owner: id,
                source,
            },
        );
        inner.broadcast(ServerMsg::Added(room.clone()));
        Ok(())
    }

    /// Unlists watcher `id`'s room `code`, if it has one.
    fn withdraw(&self, id: u64, code: &str) {
        let mut inner = self.inner.lock().unwrap();
        if inner
            .rooms
            .get(code)
            .is_some_and(|listing| listing.owner == id)
        {
            inner.rooms.remove(code);
            inner.broadcast(ServerMsg::Gone(code.to_owned()));
        }
    }

    /// Drops watcher `id` and unlists its rooms.
    fn leave(&self, id: u64) {
        let mut inner = self.inner.lock().unwrap();
        inner.watchers.remove(&id);
        let gone: Vec<String> = inner
            .rooms
            .iter()
            .filter(|(_, listing)| listing.owner == id)
            .map(|(code, _)| code.clone())
            .collect();
        for code in gone {
            inner.rooms.remove(&code);
            inner.broadcast(ServerMsg::Gone(code));
        }
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().rooms.len()
    }
}

impl Inner {
    fn broadcast(&self, msg: ServerMsg) {
        for watcher in self.watchers.values().filter(|watcher| watcher.watching) {
            push(&watcher.conn, &watcher.outbox, msg.clone());
        }
    }
}

impl Listing {
    fn room(&self, code: &str) -> Room {
        Room {
            label: self.label.clone(),
            code: code.to_owned(),
        }
    }
}

/// Serves one connection until it closes.
pub(crate) async fn serve(board: &Board, conn: Connection, ip: Option<IpAddr>) {
    let Some(_slot) = board.connections.take() else {
        close(&conn, CloseCode::LimitExceeded);
        return;
    };
    let _ip_slot = match ip {
        Some(ip) => match board.connections_per_ip.take(ip) {
            Some(slot) => Some(slot),
            None => {
                close(&conn, CloseCode::LimitExceeded);
                return;
            }
        },
        None => None,
    };
    let Ok(Ok((send, mut recv))) = tokio::time::timeout(STREAM_TIMEOUT, conn.accept_bi()).await
    else {
        close(&conn, CloseCode::ProtocolError);
        return;
    };

    // The whole list goes out at once, so the outbox holds it.
    let (outbox, outbox_rx) = mpsc::channel(board.options.max_rooms + 64);
    let writer = tokio::spawn(write_all(conn.clone(), send, outbox_rx));
    let id = board.join(&conn, outbox.clone());
    let max_len = board.options.max_label_len + board.options.max_code_len + 16;
    let code = loop {
        let msg = match read_msg_max(&mut recv, max_len).await {
            Ok(Some(msg)) => msg,
            Ok(None) | Err(WireError::Io(_)) => break CloseCode::Normal,
            Err(WireError::TooLarge(_) | WireError::Decode(_)) => break CloseCode::ProtocolError,
        };
        match msg {
            ClientMsg::Watch => board.watch(id),
            ClientMsg::Announce(room) => {
                let reply = match board.announce(id, ip, &room) {
                    Ok(()) => ServerMsg::Announced(room.code),
                    Err(reason) => ServerMsg::Refused {
                        code: room.code,
                        reason,
                    },
                };
                push(&conn, &outbox, reply);
            }
            ClientMsg::Withdraw(code) => board.withdraw(id, &code),
        }
    };

    board.leave(id);
    // The board held the other sender; with ours gone the writer finishes.
    drop(outbox);
    let _ = tokio::time::timeout(FLUSH_TIMEOUT, writer).await;
    close(&conn, code);
}

/// Writes queued messages until every sender is gone, then finishes the
/// stream.
async fn write_all(conn: Connection, mut send: SendStream, mut outbox: mpsc::Receiver<ServerMsg>) {
    while let Some(msg) = outbox.recv().await {
        match tokio::time::timeout(WRITE_TIMEOUT, write_msg(&mut send, &msg)).await {
            Ok(Ok(())) => {}
            Ok(Err(_)) => {
                close(&conn, CloseCode::Normal);
                return;
            }
            // Flow control held the write: the client stopped reading.
            Err(_) => {
                close(&conn, CloseCode::LimitExceeded);
                return;
            }
        }
    }
    let _ = send.finish();
    let _ = send.stopped().await;
}

/// Queues `msg`; a full outbox means the client stopped reading, so it is
/// closed rather than queued for without limit.
fn push(conn: &Connection, outbox: &mpsc::Sender<ServerMsg>, msg: ServerMsg) {
    if let Err(TrySendError::Full(_)) = outbox.try_send(msg) {
        close(conn, CloseCode::LimitExceeded);
    }
}

fn close(conn: &Connection, code: CloseCode) {
    conn.close((code as u32).into(), b"");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn room(code: &str) -> Room {
        Room {
            label: format!("room {code}"),
            code: code.to_owned(),
        }
    }

    fn ip(s: &str) -> Option<IpAddr> {
        Some(s.parse().unwrap())
    }

    #[test]
    fn labels_and_codes_are_bounded() {
        let board = Board::new(RoomOptions {
            max_label_len: 8,
            max_code_len: 4,
            ..RoomOptions::default()
        });
        let invalid = |label: &str, code: &str| {
            let room = Room {
                label: label.to_owned(),
                code: code.to_owned(),
            };
            board.announce(0, None, &room) == Err(Refusal::Invalid)
        };
        assert!(invalid("", "a"));
        assert!(invalid("123456789", "a"));
        assert!(invalid("new\nline", "a"));
        assert!(invalid("ok", ""));
        assert!(invalid("ok", "12345"));
        assert!(!invalid("12345678", "1234"));
    }

    #[test]
    fn a_code_belongs_to_whoever_listed_it() {
        let board = Board::new(RoomOptions::default());
        board.announce(0, None, &room("a")).unwrap();
        assert_eq!(board.announce(1, None, &room("a")), Err(Refusal::Taken));
        // The owner may relabel it, without it counting twice.
        let relabelled = Room {
            label: "renamed".into(),
            code: "a".into(),
        };
        board.announce(0, None, &relabelled).unwrap();
        assert_eq!(board.len(), 1);
        // Only the owner can withdraw it.
        board.withdraw(1, "a");
        assert_eq!(board.len(), 1);
        board.withdraw(0, "a");
        assert_eq!(board.len(), 0);
        board.announce(1, None, &room("a")).unwrap();
    }

    #[test]
    fn rooms_per_connection_are_capped() {
        let board = Board::new(RoomOptions {
            max_rooms_per_connection: 2,
            ..RoomOptions::default()
        });
        board.announce(0, ip("192.0.2.1"), &room("a")).unwrap();
        board.announce(0, ip("192.0.2.1"), &room("b")).unwrap();
        let refused = board.announce(0, ip("192.0.2.1"), &room("c"));
        assert_eq!(refused, Err(Refusal::TooManyRooms));
        board.announce(1, ip("192.0.2.1"), &room("c")).unwrap();
    }

    #[test]
    fn rooms_per_ip_are_capped() {
        let board = Board::new(RoomOptions {
            max_rooms_per_ip: 2,
            max_rooms_unknown_ip: 3,
            ..RoomOptions::default()
        });
        board.announce(0, ip("2001:db8::1"), &room("a")).unwrap();
        board.announce(1, ip("2001:db8::2"), &room("b")).unwrap();
        // The same /64 from a third connection.
        let refused = board.announce(2, ip("2001:db8::3"), &room("c"));
        assert_eq!(refused, Err(Refusal::TooManyRooms));
        board.announce(2, ip("192.0.2.1"), &room("c")).unwrap();
        // Unknown sources share their own cap.
        for (id, code) in [(3, "d"), (4, "e"), (5, "f")] {
            board.announce(id, None, &room(code)).unwrap();
        }
        assert_eq!(
            board.announce(6, None, &room("g")),
            Err(Refusal::TooManyRooms)
        );
    }

    #[test]
    fn the_list_is_capped() {
        let board = Board::new(RoomOptions {
            max_rooms: 2,
            ..RoomOptions::default()
        });
        board.announce(0, ip("192.0.2.1"), &room("a")).unwrap();
        board.announce(1, ip("192.0.2.2"), &room("b")).unwrap();
        let refused = board.announce(2, ip("192.0.2.3"), &room("c"));
        assert_eq!(refused, Err(Refusal::ListFull));
    }

    #[test]
    fn announces_are_rate_limited_per_ip() {
        let board = Board::new(RoomOptions {
            announces_per_ip_per_min: 2,
            announces_unknown_ip_per_min: 1,
            ..RoomOptions::default()
        });
        board.announce(0, ip("192.0.2.1"), &room("a")).unwrap();
        // A relabel counts as an announce too.
        board.announce(0, ip("192.0.2.1"), &room("a")).unwrap();
        let refused = board.announce(1, ip("192.0.2.1"), &room("b"));
        assert_eq!(refused, Err(Refusal::RateLimited));
        board.announce(1, ip("192.0.2.2"), &room("b")).unwrap();
        board.announce(2, None, &room("c")).unwrap();
        assert_eq!(
            board.announce(3, None, &room("d")),
            Err(Refusal::RateLimited)
        );
        // A refused announce costs nothing.
        assert_eq!(board.len(), 3);
    }
}
