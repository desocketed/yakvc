//! The peer connection cap end to end (DESIGN.md "Connection lifecycle"),
//! with the cap lowered from 64 to 2 so three peers reach it.

use std::time::Duration;

use tokio::time::{Instant, sleep};
use yakvc_client::{PeerState, Uuid, Vec3};
use yakvc_proto::peer::CloseCode;
use yakvc_testkit::{TRACKING_RANGE, TestClient, TestNet};

const TOO_MANY_PEERS: Option<u64> = Some(CloseCode::TooManyPeers as u64);

fn at(x: f64) -> Vec3 {
    Vec3::new(x, 64.0, 0.0)
}

/// The peers `client` has a direct connection to, sorted.
fn connected(client: &TestClient) -> Vec<Uuid> {
    let mut uuids: Vec<Uuid> = client
        .engine()
        .peers()
        .into_iter()
        .filter(|p| p.state == PeerState::Direct)
        .map(|p| p.uuid)
        .collect();
    uuids.sort();
    uuids
}

fn sorted(mut uuids: Vec<Uuid>) -> Vec<Uuid> {
    uuids.sort();
    uuids
}

/// Polls `check` until it holds, panicking with `what` after 30 s: well
/// past the few seconds a dial grace, handshake and eviction pass take.
async fn eventually(what: &str, mut check: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !check() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn nearer_peers_take_the_connections_at_the_cap() {
    let net = TestNet::start().await;
    let hub = net.client().name("hub").max_peers(2).start().await;
    let alice = net.client().name("alice").start().await;
    let bob = net.client().name("bob").start().await;
    let carol = net.client().name("carol").start().await;

    // alice and bob are tracked by the hub; carol is out of tracking range.
    hub.move_to(at(0.0));
    alice.move_to(at(10.0));
    bob.move_to(at(20.0));
    carol.move_to(at(TRACKING_RANGE * 2.0));
    hub.sees(&[&alice, &bob, &carol]);
    for spoke in [&alice, &bob, &carol] {
        spoke.sees(&[&hub]);
    }

    eventually("the hub to keep alice and bob and refuse carol", || {
        connected(&hub) == sorted(vec![alice.uuid(), bob.uuid()])
            && carol.engine().closed_by_peer(hub.uuid()) == TOO_MANY_PEERS
    })
    .await;
    assert!(connected(&carol).is_empty());

    // carol walks up and alice walks away: carol now outranks alice, so the
    // hub connects carol and evicts alice.
    carol.move_to(at(5.0));
    alice.move_to(at(TRACKING_RANGE * 2.0));
    eventually("the hub to evict alice for carol", || {
        connected(&hub) == sorted(vec![bob.uuid(), carol.uuid()])
            && alice.engine().closed_by_peer(hub.uuid()) == TOO_MANY_PEERS
    })
    .await;
    assert!(connected(&alice).is_empty());
}
