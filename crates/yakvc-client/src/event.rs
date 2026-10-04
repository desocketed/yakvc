use std::time::{Duration, SystemTime};

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
    Peer {
        uuid: Uuid,
        state: PeerState,
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
    _p: (),
}

impl Events {
    /// Next event if one is queued. For the game loop.
    pub fn try_next(&mut self) -> Option<Event> {
        todo!()
    }

    /// Waits for the next event. `None` once the engine has shut down.
    pub async fn next(&mut self) -> Option<Event> {
        todo!()
    }
}
