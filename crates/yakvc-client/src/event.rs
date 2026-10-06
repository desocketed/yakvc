use std::time::{Duration, SystemTime};

use tokio::sync::mpsc;
use yakvc_shared::Uuid;

/// Notifications from the engine, in order.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// Call Mojang `joinServer` with `server_id`, then
    /// [`Engine::complete_join`](crate::Engine::complete_join) with `id`.
    JoinRequest {
        id: JoinId,
        server_id: String,
    },
    Rendezvous(RendezvousState),
    /// A peer's connection state, or whether its ticket is verified,
    /// changed.
    Peer {
        uuid: Uuid,
        state: PeerState,
        /// The peer proved its account to Mojang.
        verified: bool,
    },
    /// A peer (or the local player) started or stopped talking.
    Talking {
        uuid: Uuid,
        talking: bool,
    },
    /// Local microphone level in dBFS, about 10 times a second.
    MicLevel(f32),
    /// A non-fatal problem worth showing the user.
    Error(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct JoinId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RendezvousState {
    Connecting,
    Authenticating,
    Registered {
        expires_at: SystemTime,
        /// Our ticket is verified: the account was proven to Mojang.
        verified: bool,
    },
    /// Mojang or the rendezvous asked us to back off.
    RetryingIn(Duration),
    Disconnected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerState {
    Connecting,
    Direct,
    Relayed,
    /// Relayed, but outside the relay budget, so not receiving our audio.
    RelayFull,
    Failed,
    /// Connection closed; the peer is no longer listed.
    Gone,
}

/// Receiving end of the engine's event queue.
#[derive(Debug)]
pub struct Events {
    rx: mpsc::UnboundedReceiver<Event>,
}

impl Events {
    /// Next event if one is queued. For the game loop.
    pub fn try_next(&mut self) -> Option<Event> {
        self.rx.try_recv().ok()
    }

    /// Waits for the next event. `None` once the engine has shut down.
    pub async fn next(&mut self) -> Option<Event> {
        self.rx.recv().await
    }
}

/// Sending end of the event queue, cloned into every task that reports
/// something. Sending never blocks and never fails: once [`Events`] is
/// dropped nobody is listening, so events are discarded.
#[derive(Debug, Clone)]
pub(crate) struct EventSender {
    tx: mpsc::UnboundedSender<Event>,
}

impl EventSender {
    pub(crate) fn send(&self, event: Event) {
        let _ = self.tx.send(event);
    }
}

pub(crate) fn channel() -> (EventSender, Events) {
    let (tx, rx) = mpsc::unbounded_channel();
    (EventSender { tx }, Events { rx })
}
