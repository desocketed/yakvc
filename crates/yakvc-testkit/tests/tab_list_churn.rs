//! A tab list that changes every few milliseconds must not trip the
//! rendezvous's pair-update rate limit, which would drop every voice link.

use std::time::Duration;

use tokio::time::{Instant, sleep};
use yakvc_client::{Event, PeerState, RendezvousState, Uuid};
use yakvc_testkit::{TestNet, see_each_other};

#[tokio::test(flavor = "multi_thread")]
async fn tab_list_churn_keeps_the_session() {
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

    // Players joining and leaving alice's tab list far faster than the
    // rate limit, every 5 ms for 2 s; bob stays listed throughout.
    let end = Instant::now() + Duration::from_secs(2);
    let mut n = 0u128;
    while Instant::now() < end {
        n += 1;
        let strangers = (n..n + 5).map(|i| Uuid::from_u128(i << 64));
        alice.engine().set_tab_list(strangers.chain([bob.uuid()]));
        sleep(Duration::from_millis(5)).await;
    }

    // Unlisting bob ends the link, after any session drop the churn caused.
    alice.engine().set_tab_list([]);
    let mut dropped = Vec::new();
    alice
        .wait_for("bob gone", |e| {
            if let Event::Rendezvous(
                state @ (RendezvousState::Disconnected | RendezvousState::RetryingIn(_)),
            ) = e
            {
                dropped.push(*state);
            }
            matches!(e, Event::Peer { uuid, state: PeerState::Gone, .. } if *uuid == bob.uuid())
        })
        .await
        .unwrap();
    assert!(
        dropped.is_empty(),
        "rendezvous session dropped: {dropped:?}"
    );
    assert_eq!(net.server().stats().sessions, 2);
}
