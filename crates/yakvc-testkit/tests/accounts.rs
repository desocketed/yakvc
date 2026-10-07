//! Players without a Mojang account (DESIGN.md "Identity, discovery and
//! authentication"): on an offline-mode server they get unverified tickets
//! and talk as usual; on an online-mode server they get nothing.

use std::time::Duration;

use tokio::time::sleep;
use yakvc_client::{Event, PeerState, RendezvousState, Vec3};
use yakvc_testkit::{DAY, TestNet, see_each_other};

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
