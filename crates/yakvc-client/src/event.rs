use std::cell::RefCell;
use std::panic;
use std::sync::{Arc, Mutex, Once};
use std::time::{Duration, SystemTime};

use tokio::sync::mpsc;
use yakvc_proto::Uuid;

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
    /// `from` invited us to its group. Valid for two minutes; [`Engine::accept_invite`](crate::Engine::accept_invite)
    /// joins it.
    Invite {
        from: Uuid,
    },
    /// Something happened in our group. See [`GroupNoticeKind`] for what
    /// `uuid` and `by` mean; either is nil when it doesn't apply.
    GroupNotice {
        kind: GroupNoticeKind,
        uuid: Uuid,
        by: Uuid,
    },
    /// Local microphone level in dBFS, about 10 times a second.
    MicLevel(f32),
    /// A non-fatal problem worth showing the user.
    Error(String),
}

/// What a [`Event::GroupNotice`] reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupNoticeKind {
    /// `uuid` became a group mate.
    Joined,
    /// `uuid` is no longer a group mate: it left or its link closed.
    Left,
    /// `by` made the group public.
    MadePublic,
    /// `by` made the group private.
    MadePrivate,
    /// `by` changed the label.
    LabelChanged,
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
    /// Why the engine failed, shared by every clone. See [`EventSender::fail`].
    failure: Arc<Mutex<Option<String>>>,
}

impl EventSender {
    pub(crate) fn send(&self, event: Event) {
        let _ = self.tx.send(event);
    }

    /// Marks the engine failed for good, keeping the first reason, and
    /// reports it.
    pub(crate) fn fail(&self, message: String) {
        self.failure
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get_or_insert_with(|| message.clone());
        self.send(Event::Error(message));
    }

    pub(crate) fn failure(&self) -> Option<String> {
        self.failure
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

pub(crate) fn channel() -> (EventSender, Events) {
    let (tx, rx) = mpsc::unbounded_channel();
    let failure = Arc::default();
    (EventSender { tx, failure }, Events { rx })
}

thread_local! {
    /// The engine owning this thread, if it is one of the engine's own.
    static ENGINE_EVENTS: RefCell<Option<EventSender>> = const { RefCell::new(None) };
}

/// Marks the current thread as one of the engine's own (the audio thread,
/// the Tokio workers), so a panic on it fails the engine. Otherwise such a
/// panic would only end that thread or task, and voice could die quietly
/// while peers still show as connected. Panics on the caller's threads reach
/// the caller instead (the FFI catches them).
pub(crate) fn report_panics_on_this_thread(events: &EventSender) {
    static HOOK: Once = Once::new();
    HOOK.call_once(|| {
        let previous = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            let message = info.payload_as_str().unwrap_or("unknown panic");
            // `try_with`: the thread may be exiting.
            let _ = ENGINE_EVENTS.try_with(|events| {
                if let Some(events) = &*events.borrow() {
                    events.fail(format!("the voice engine crashed: {message}"));
                }
            });
            previous(info);
        }));
    });
    ENGINE_EVENTS.with(|current| *current.borrow_mut() = Some(events.clone()));
}
