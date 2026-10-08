//! What happens hours into a session (DESIGN.md "Identity, discovery and
//! authentication"), sped up with short ticket lifetimes: renewals keep
//! links up, a ticket that runs out ends them, and links outlive the
//! rendezvous connection.

use std::time::{Duration, Instant};

use tokio::time::sleep;
use yakvc_client::{Event, PeerState, RendezvousState, Vec3};
use yakvc_testkit::{TestClient, TestNet, see_each_other};

/// Whether `client` has a direct link to `other`.
fn linked(client: &TestClient, other: &TestClient) -> bool {
    client
        .engine()
        .peers()
        .iter()
        .any(|p| p.uuid == other.uuid() && p.state == PeerState::Direct)
}

/// Asserts that `a` and `b` stay linked for `period`.
async fn stay_linked(a: &TestClient, b: &TestClient, period: Duration) {
    let end = Instant::now() + period;
    while Instant::now() < end {
        assert!(linked(a, b), "{:?}", a.engine().peers());
        assert!(linked(b, a), "{:?}", b.engine().peers());
        sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn links_outlive_the_first_tickets_and_end_when_one_runs_out() {
    // As in `players_without_accounts_renew_unverified_tickets`: long enough
    // for a renewal's round trips on a loaded machine.
    let lifetime = Duration::from_secs(8);
    let net = TestNet::start_without_accounts(lifetime).await;
    let mut alice = net.client().name("alice").offline().start().await;
    let mut bob = net.client().name("bob").offline().tone(440.0).start().await;
    see_each_other(&[&alice, &bob]);
    alice.move_to(Vec3::new(0.0, 64.0, 0.0));
    bob.move_to(Vec3::new(10.0, 64.0, 0.0));
    alice
        .wait_for_peer(bob.uuid(), PeerState::Direct)
        .await
        .unwrap();
    bob.wait_for_peer(alice.uuid(), PeerState::Direct)
        .await
        .unwrap();

    // Both first tickets have run out by the end of this, so each side
    // has accepted the other's renewed ticket.
    stay_linked(&alice, &bob, lifetime + Duration::from_secs(1)).await;
    bob.talk(true);
    sleep(Duration::from_millis(500)).await;
    let before = alice.recording().audible_frames();
    sleep(Duration::from_secs(1)).await;
    assert!(alice.recording().audible_frames() > before + 10);

    bob.stop_renewing();
    // A renewal already under way may still land, so allow two lifetimes.
    let deadline = Instant::now() + 2 * lifetime + Duration::from_secs(5);
    while linked(&alice, &bob) {
        assert!(
            Instant::now() < deadline,
            "bob's ticket ran out but alice still links him"
        );
        sleep(Duration::from_millis(100)).await;
    }
}

/// The rendezvous closing every session, as a restart does. Links between
/// peers don't go through it, so they stay up while clients reconnect, and
/// the new sessions match players as before.
#[tokio::test(flavor = "multi_thread")]
async fn links_survive_losing_the_rendezvous() {
    let net = TestNet::start().await;
    let mut alice = net.client().name("alice").start().await;
    let mut bob = net.client().name("bob").start().await;
    see_each_other(&[&alice, &bob]);
    alice
        .wait_for_peer(bob.uuid(), PeerState::Direct)
        .await
        .unwrap();
    bob.wait_for_peer(alice.uuid(), PeerState::Direct)
        .await
        .unwrap();

    net.server().close_sessions();
    for client in [&mut alice, &mut bob] {
        client
            .wait_for("the rendezvous to close", |e| {
                matches!(e, Event::Rendezvous(RendezvousState::Disconnected))
            })
            .await
            .unwrap();
        client
            .wait_for("registration again", |e| {
                matches!(e, Event::Rendezvous(RendezvousState::Registered { .. }))
            })
            .await
            .unwrap();
    }
    // Past the grace in which a new session must list a peer again
    // (`RELIST_GRACE`, 10 s) before it is dropped.
    stay_linked(&alice, &bob, Duration::from_secs(12)).await;

    let mut carol = net.client().name("carol").start().await;
    see_each_other(&[&alice, &bob, &carol]);
    carol
        .wait_for_peer(alice.uuid(), PeerState::Direct)
        .await
        .unwrap();
    assert!(linked(&alice, &bob));
    assert_eq!(net.server().stats().sessions, 3);
}
