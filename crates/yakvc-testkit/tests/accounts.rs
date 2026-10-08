//! Players without a Mojang account (DESIGN.md "Identity, discovery and
//! authentication"): on an offline-mode server they get unverified tickets
//! and talk as usual; on an online-mode server they get nothing.

use std::time::Duration;

use tokio::time::sleep;
use yakvc_client::{Event, PeerState, RendezvousState, Uuid, Vec3};
use yakvc_testkit::{DAY, TestClient, TestNet, see_each_other};

#[tokio::test(flavor = "multi_thread")]
async fn players_without_accounts_talk_on_offline_mode_servers() {
    let net = TestNet::start_without_accounts(DAY).await;
    let mut alice = net
        .client()
        .name("alice")
        .offline()
        .tone(440.0)
        .start()
        .await;
    let mut bob = net.client().name("bob").offline().start().await;
    see_each_other(&[&alice, &bob]);
    alice.move_to(Vec3::new(0.0, 64.0, 0.0));
    bob.move_to(Vec3::new(10.0, 64.0, 0.0));

    bob.wait_for_peer(alice.uuid(), PeerState::Direct)
        .await
        .unwrap();
    alice
        .wait_for_peer(bob.uuid(), PeerState::Direct)
        .await
        .unwrap();
    let peers = bob.engine().peers();
    assert!(!peers[0].verified, "{peers:?}");

    alice.talk(true);
    let before = bob.recording().audible_frames();
    sleep(Duration::from_secs(1)).await;
    assert!(bob.recording().audible_frames() > before + 10);
}

/// An account UUID (as on an online-mode server) can't do without the
/// Mojang proof, so it never registers and nobody connects to it.
#[tokio::test(flavor = "multi_thread")]
async fn players_without_accounts_get_nothing_on_online_mode_servers() {
    let net = TestNet::start_without_accounts(DAY).await;
    let mut carol = net.client().name("carol").spawn();
    let alice = net.client().name("alice").offline().start().await;
    see_each_other(&[&alice, &carol]);

    let failed = carol
        .wait_for("the sign-in to fail", |e| matches!(e, Event::Error(_)))
        .await
        .unwrap();
    assert!(
        matches!(&failed, Event::Error(msg) if msg.contains("sign-in failed")),
        "{failed:?}"
    );
    let registered = carol
        .wait_for("registration", |e| {
            matches!(e, Event::Rendezvous(RendezvousState::Registered { .. }))
        })
        .await;
    assert!(registered.is_err(), "carol registered");
    assert!(alice.engine().peers().is_empty());
}

/// Renewing needs no account either: the failed `joinServer` is declined
/// again, and the link stays up throughout.
#[tokio::test(flavor = "multi_thread")]
async fn players_without_accounts_renew_unverified_tickets() {
    // Clients renew with a tenth of the lifetime to spare, and a renewal
    // takes several round trips: 8 s leaves time for a loaded machine,
    // while the next renewal still comes within `DEFAULT_TIMEOUT`.
    let net = TestNet::start_without_accounts(Duration::from_secs(8)).await;
    let mut alice = net.client().name("alice").offline().start().await;
    let mut bob = net.client().name("bob").offline().start().await;
    see_each_other(&[&alice, &bob]);
    alice
        .wait_for_peer(bob.uuid(), PeerState::Direct)
        .await
        .unwrap();
    bob.wait_for_peer(alice.uuid(), PeerState::Direct)
        .await
        .unwrap();

    let renewed = alice
        .wait_for("a renewed ticket", |e| {
            matches!(
                e,
                Event::Rendezvous(RendezvousState::Registered {
                    verified: false,
                    ..
                })
            )
        })
        .await;
    assert!(renewed.is_ok(), "{renewed:?}");
    let peers = alice.engine().peers();
    assert_eq!(peers.len(), 1);
    assert_eq!(peers[0].state, PeerState::Direct);
}

/// Two unverified endpoints claiming one offline UUID both link up, since
/// neither outranks the other. Only the newer link is heard, so the two
/// never mix in one jitter buffer (#109).
#[tokio::test(flavor = "multi_thread")]
async fn only_the_newest_link_for_a_player_is_heard() {
    let net = TestNet::start_without_accounts(DAY).await;
    let alice = net.client().name("alice").offline().start().await;
    let first = net.client().name("bob").offline().tone(440.0).start().await;
    let bob = first.uuid();
    see_each_other(&[&alice, &first]);
    alice.move_to(Vec3::new(0.0, 64.0, 0.0));
    first.move_to(Vec3::new(5.0, 64.0, 0.0));
    links_to(&alice, bob, 1).await;

    // A second, silent "bob" links up after the first.
    let second = net.client().name("bob").offline().start().await;
    second.sees(&[&alice]);
    second.move_to(Vec3::new(5.0, 64.0, 0.0));
    links_to(&alice, bob, 2).await;
    sleep(Duration::from_millis(500)).await;

    first.talk(true);
    sleep(Duration::from_millis(500)).await;
    let before = alice.recording().audible_frames();
    sleep(Duration::from_secs(1)).await;
    assert_eq!(alice.recording().audible_frames(), before);
}

/// The older link closing must not take the newer one out of voice with
/// it: the newer link is still heard afterwards.
#[tokio::test(flavor = "multi_thread")]
async fn the_newest_link_is_still_heard_once_an_older_one_closes() {
    let net = TestNet::start_without_accounts(DAY).await;
    let alice = net.client().name("alice").offline().start().await;
    let first = net.client().name("bob").offline().start().await;
    let bob = first.uuid();
    see_each_other(&[&alice, &first]);
    alice.move_to(Vec3::new(0.0, 64.0, 0.0));
    first.move_to(Vec3::new(5.0, 64.0, 0.0));
    links_to(&alice, bob, 1).await;

    let second = net.client().name("bob").offline().tone(440.0).start().await;
    second.sees(&[&alice]);
    second.move_to(Vec3::new(5.0, 64.0, 0.0));
    links_to(&alice, bob, 2).await;
    second.talk(true);
    sleep(Duration::from_millis(500)).await;
    let before = alice.recording().audible_frames();
    sleep(Duration::from_secs(1)).await;
    assert!(alice.recording().audible_frames() > before + 10);

    drop(first);
    links_to(&alice, bob, 1).await;
    sleep(Duration::from_millis(500)).await;
    let before = alice.recording().audible_frames();
    sleep(Duration::from_secs(1)).await;
    assert!(alice.recording().audible_frames() > before + 10);
}

/// Waits until `listener` has `n` direct links to the player `to`.
async fn links_to(listener: &TestClient, to: Uuid, n: usize) {
    for _ in 0..500 {
        let up = listener
            .engine()
            .peers()
            .iter()
            .filter(|p| p.uuid == to && p.state == PeerState::Direct)
            .count();
        if up == n {
            return;
        }
        sleep(Duration::from_millis(20)).await;
    }
    panic!(
        "never had {n} links to {to}: {:?}",
        listener.engine().peers()
    );
}
