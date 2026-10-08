//! Group voice chat (DESIGN.md "Group voice chat", #28): group mates hear
//! each other at any distance, unpositioned, and "group only" cuts nearby
//! players off both ways.

use std::time::Duration;

use tokio::time::{Instant, sleep};
use yakvc_client::{PeerState, Vec3};
use yakvc_testkit::{TestClient, TestNet, see_each_other};

/// Peak level above which a sample counts as audible: -60 dBFS.
const AUDIBLE: f32 = 0.001;
/// Long enough for announcements, a world tick and a few frames in flight.
const SETTLE: Duration = Duration::from_millis(500);

fn at(x: f64, z: f64) -> Vec3 {
    Vec3::new(x, 64.0, z)
}

/// What `client` plays during the next second.
async fn listen(client: &TestClient) -> Vec<[f32; 2]> {
    client.recording().take();
    sleep(Duration::from_secs(1)).await;
    client.recording().take()
}

fn is_silent(samples: &[[f32; 2]]) -> bool {
    !samples.iter().flatten().any(|s| s.abs() > AUDIBLE)
}

/// Root-mean-square level of the left and right channels.
fn levels(samples: &[[f32; 2]]) -> (f32, f32) {
    let n = samples.len().max(1) as f32;
    let left = samples.iter().map(|s| s[0] * s[0]).sum::<f32>() / n;
    let right = samples.iter().map(|s| s[1] * s[1]).sum::<f32>() / n;
    (left.sqrt(), right.sqrt())
}

/// Waits until `client` lists a group of `members`.
async fn wait_for_group(client: &TestClient, members: &[&TestClient]) {
    let mut wanted: Vec<_> = members.iter().map(|c| c.uuid()).collect();
    wanted.sort();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let found = client.engine().groups().into_iter().any(|group| {
            let mut have = group.members;
            have.sort();
            have == wanted
        });
        if found {
            return;
        }
        assert!(Instant::now() < deadline, "no group of {wanted:?}");
        sleep(Duration::from_millis(20)).await;
    }
}

/// Starts named clients that see each other, all connected directly.
async fn clients(net: &TestNet, names: &[&str]) -> Vec<TestClient> {
    let mut clients = Vec::new();
    for (i, name) in names.iter().enumerate() {
        let tone = 440.0 + 110.0 * i as f32;
        clients.push(net.client().name(name).tone(tone).start().await);
    }
    see_each_other(&clients.iter().collect::<Vec<_>>());
    for i in 0..clients.len() {
        for j in 0..clients.len() {
            if i != j {
                let other = clients[j].uuid();
                clients[i]
                    .wait_for_peer(other, PeerState::Direct)
                    .await
                    .unwrap();
            }
        }
    }
    clients
}

#[tokio::test(flavor = "multi_thread")]
async fn group_mates_hear_each_other_at_any_distance() {
    let net = TestNet::start().await;
    let [alice, bob, carol]: [TestClient; 3] = clients(&net, &["alice", "bob", "carol"])
        .await
        .try_into()
        .unwrap();
    // Far beyond voice and tracking range of each other.
    alice.move_to(at(0.0, 0.0));
    bob.move_to(at(1000.0, 0.0));
    carol.move_to(at(-1000.0, 0.0));
    alice.talk(true);
    sleep(SETTLE).await;
    assert!(is_silent(&listen(&bob).await), "heard without a group");

    alice.engine().join_group("Miners", "pw").unwrap();
    bob.engine().join_group("Miners", "pw").unwrap();
    // Same name, wrong password: a different group.
    carol.engine().join_group("Miners", "guess").unwrap();
    wait_for_group(&bob, &[&alice, &bob]).await;
    wait_for_group(&bob, &[&carol]).await;
    sleep(SETTLE).await;

    let heard = listen(&bob).await;
    assert!(!is_silent(&heard), "a group mate wasn't heard");
    let (left, right) = levels(&heard);
    assert!((left - right).abs() < left * 0.05, "group voice is panned");
    assert!(
        is_silent(&listen(&carol).await),
        "the wrong password let carol hear the group"
    );

    // Leaving the group stops it.
    bob.engine().leave_group();
    sleep(SETTLE).await;
    assert!(is_silent(&listen(&bob).await), "heard after leaving");
}

#[tokio::test(flavor = "multi_thread")]
async fn group_only_cuts_nearby_players_off_both_ways() {
    let net = TestNet::start().await;
    let [alice, bob, carol]: [TestClient; 3] = clients(&net, &["alice", "bob", "carol"])
        .await
        .try_into()
        .unwrap();
    // Carol stands next to alice but isn't in the group; bob is far away.
    alice.move_to(at(0.0, 0.0));
    carol.move_to(at(2.0, 0.0));
    bob.move_to(at(1000.0, 0.0));
    alice.engine().join_group("Miners", "").unwrap();
    bob.engine().join_group("Miners", "").unwrap();
    wait_for_group(&alice, &[&alice, &bob]).await;

    // Nearby on (the default): carol hears alice as usual.
    alice.talk(true);
    sleep(SETTLE).await;
    assert!(!is_silent(&listen(&carol).await));

    alice.engine().set_group_nearby(false);
    sleep(SETTLE).await;
    assert!(is_silent(&listen(&carol).await), "carol still hears alice");
    assert!(!is_silent(&listen(&bob).await), "bob lost the group");

    alice.talk(false);
    carol.talk(true);
    sleep(SETTLE).await;
    assert!(is_silent(&listen(&alice).await), "alice still hears carol");
}
