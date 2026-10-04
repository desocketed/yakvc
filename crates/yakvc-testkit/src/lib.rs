//! A rendezvous server and simulated clients in one process, for integration
//! tests.
//!
//! ```ignore
//! let net = TestNet::start().await;
//! let mut alice = net.client().name("alice").tone(440.0).start().await;
//! let mut bob = net.client().name("bob").start().await;
//! see_each_other(&[&alice, &bob]);
//! alice.move_to(Vec3::new(0.0, 64.0, 0.0));
//! bob.move_to(Vec3::new(10.0, 64.0, 0.0));
//! bob.wait_for_peer(alice.uuid(), PeerState::Direct).await?;
//! alice.talk(true);
//! ```
//!
//! [`TestNet`] plays the game server: it keeps every client's position and
//! pushes each client a world snapshot at 20 Hz containing the clients it
//! sees within tracking range.

use std::path::Path;
use std::time::Duration;

use yakvc_audio::Recording;
use yakvc_client::sim::Impairment;
use yakvc_client::{Engine, Event, PeerState, StreamStats, Uuid, Vec3};
use yakvc_server::Server;

/// Default time [`TestClient`] waits for an expected event.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// Player-entity tracking range the simulated game server uses (Paper's
/// default).
pub const TRACKING_RANGE: f64 = 48.0;

/// A dev-auth rendezvous with a plain-HTTP relay on localhost, plus the
/// simulated game world.
#[derive(Debug)]
pub struct TestNet {
    _p: (),
}

/// Configures a [`TestClient`].
#[derive(Debug)]
pub struct ClientBuilder<'a> {
    _net: &'a TestNet,
}

/// A simulated player: an [`Engine`] with test audio I/O, driven by the
/// [`TestNet`] world.
#[derive(Debug)]
pub struct TestClient {
    _p: (),
}

#[derive(Debug, thiserror::Error)]
#[error("timed out after {0:?} waiting for {1}")]
pub struct Timeout(pub Duration, pub String);

impl TestNet {
    pub async fn start() -> TestNet {
        todo!()
    }

    pub fn server(&self) -> &Server {
        todo!()
    }

    pub fn client(&self) -> ClientBuilder<'_> {
        ClientBuilder { _net: self }
    }
}

impl<'a> ClientBuilder<'a> {
    /// Defaults to `player<N>`. The UUID is derived from the name.
    pub fn name(self, name: &str) -> Self {
        let _ = name;
        todo!()
    }

    /// Speaks a sine tone while talking. The default is silence.
    pub fn tone(self, frequency_hz: f32) -> Self {
        let _ = frequency_hz;
        todo!()
    }

    pub fn wav(self, path: &Path) -> Self {
        let _ = path;
        todo!()
    }

    pub fn impairment(self, impairment: Impairment) -> Self {
        let _ = impairment;
        todo!()
    }

    pub fn relay_only(self) -> Self {
        todo!()
    }

    /// Starts the engine and waits until it is registered with the rendezvous.
    pub async fn start(self) -> TestClient {
        todo!()
    }
}

impl TestClient {
    pub fn uuid(&self) -> Uuid {
        todo!()
    }

    pub fn engine(&self) -> &Engine {
        todo!()
    }

    /// Replaces this client's tab list with `others`.
    pub fn sees(&self, others: &[&TestClient]) {
        let _ = others;
        todo!()
    }

    pub fn move_to(&self, pos: Vec3) {
        let _ = pos;
        todo!()
    }

    /// Holds or releases push-to-talk.
    pub fn talk(&self, talking: bool) {
        let _ = talking;
        todo!()
    }

    /// Everything this client has played.
    pub fn recording(&self) -> &Recording {
        todo!()
    }

    /// Receive stats for audio from `from`, if it is a connected peer.
    pub fn stream_stats(&self, from: Uuid) -> Option<StreamStats> {
        let _ = from;
        todo!()
    }

    /// Waits for an event matching `pred`, discarding others.
    pub async fn wait_for(
        &mut self,
        what: &str,
        pred: impl FnMut(&Event) -> bool,
    ) -> Result<Event, Timeout> {
        let _ = (what, pred);
        todo!()
    }

    pub async fn wait_for_peer(&mut self, uuid: Uuid, state: PeerState) -> Result<(), Timeout> {
        let _ = (uuid, state);
        todo!()
    }
}

/// Makes every client in `clients` see every other one.
pub fn see_each_other(clients: &[&TestClient]) {
    let _ = clients;
    todo!()
}
