//! Group voice chat (DESIGN.md "Group voice chat", #28): an accepted invite
//! makes a group whose members hear each other at any distance, unpositioned,
//! and only each other; public groups can be joined from the list.

use std::time::Duration;

use tokio::time::{Instant, sleep};
use yakvc_client::{Event, InvitesFrom, PeerState, Vec3};
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

async fn hears(client: &TestClient) -> bool {
    !is_silent(&listen(client).await)
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

/// Waits until `client`'s own group has exactly `members`.
async fn wait_for_group(client: &TestClient, members: &[&TestClient]) {
    let mut wanted: Vec<_> = members.iter().map(|c| c.uuid()).collect();
    wanted.sort();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let mine = client.engine().groups().into_iter().find(|g| g.mine);
        if mine.as_ref().is_some_and(|g| g.members == wanted) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "no group of {wanted:?}: {mine:?}"
        );
        sleep(Duration::from_millis(20)).await;
    }
}

/// `inviter` invites `invitee`, which accepts once the invite arrives.
async fn invite(inviter: &TestClient, invitee: &mut TestClient) {
    let from = inviter.uuid();
    inviter.engine().invite(invitee.uuid());
    invitee
        .wait_for("an invite", |e| *e == Event::Invite { from })
        .await
        .unwrap();
    invitee.engine().accept_invite(from).unwrap();
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
async fn an_accepted_invite_makes_a_group_heard_at_any_distance() {
    let net = TestNet::start().await;
    let [alice, mut bob, carol]: [TestClient; 3] = clients(&net, &["alice", "bob", "carol"])
        .await
        .try_into()
        .unwrap();
    // Bob is far away; Carol stands next to Alice.
    alice.move_to(at(0.0, 0.0));
    carol.move_to(at(2.0, 0.0));
    bob.move_to(at(1000.0, 0.0));
    alice.talk(true);
    sleep(SETTLE).await;
    assert!(!hears(&bob).await, "heard without a group");

    // Inviting alone changes nothing: Alice stays in proximity voice.
    alice.engine().invite(bob.uuid());
    let from = alice.uuid();
    bob.wait_for("an invite", |e| *e == Event::Invite { from })
        .await
        .unwrap();
    sleep(SETTLE).await;
    assert!(alice.engine().groups().is_empty());
    assert!(hears(&carol).await, "the inviter left proximity voice");
    assert!(!hears(&bob).await);

    // Once Bob accepts, both are in the group.
    bob.engine().accept_invite(from).unwrap();
    wait_for_group(&alice, &[&alice, &bob]).await;
    wait_for_group(&bob, &[&alice, &bob]).await;
    sleep(SETTLE).await;
    let heard = listen(&bob).await;
    assert!(!is_silent(&heard), "a group mate wasn't heard");
    let (left, right) = levels(&heard);
    assert!((left - right).abs() < left * 0.05, "group voice is panned");

    // In a group, voice is group-only: Carol no longer hears Alice, and
    // Alice no longer hears Carol.
    assert!(!hears(&carol).await, "carol still hears alice");
    alice.talk(false);
    carol.talk(true);
    sleep(SETTLE).await;
    assert!(!hears(&alice).await, "alice still hears carol");

    // Bob talks back across the distance; leaving ends it.
    carol.talk(false);
    bob.talk(true);
    sleep(SETTLE).await;
    assert!(hears(&alice).await, "alice doesn't hear bob");
    alice.engine().leave_group();
    sleep(SETTLE).await;
    assert!(!hears(&alice).await, "heard after leaving");
}

#[tokio::test(flavor = "multi_thread")]
async fn only_public_groups_are_listed_and_joinable() {
    let net = TestNet::start().await;
    let [alice, mut bob, carol]: [TestClient; 3] = clients(&net, &["alice", "bob", "carol"])
        .await
        .try_into()
        .unwrap();
    invite(&alice, &mut bob).await;
    wait_for_group(&alice, &[&alice, &bob]).await;
    sleep(SETTLE).await;
    // Private: Carol sees only proofs she can't place.
    assert!(carol.engine().groups().is_empty());

    alice.engine().set_group_public(true).unwrap();
    alice.engine().set_group_label("Miners").unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let listed = loop {
        let groups = carol.engine().groups();
        if let Some(group) = groups.first()
            && group.members.len() == 2
            && group.label == "Miners"
        {
            break group.clone();
        }
        assert!(Instant::now() < deadline, "not listed: {groups:?}");
        sleep(Duration::from_millis(20)).await;
    };
    assert!(listed.public && !listed.mine);
    // Bob got the change from Alice.
    assert!(bob.engine().groups()[0].public);

    carol.engine().join_group(listed.id).unwrap();
    wait_for_group(&alice, &[&alice, &bob, &carol]).await;
    wait_for_group(&carol, &[&alice, &bob, &carol]).await;
    carol.move_to(at(-1000.0, 0.0));
    carol.talk(true);
    sleep(SETTLE).await;
    assert!(hears(&alice).await);
    assert!(hears(&bob).await);
}

#[tokio::test(flavor = "multi_thread")]
async fn invites_from_nobody_are_dropped() {
    let net = TestNet::start().await;
    let alice = net.client().name("alice").start().await;
    let mut bob = net
        .client()
        .name("bob")
        .config(|config| config.invites = InvitesFrom::Nobody)
        .start()
        .await;
    see_each_other(&[&alice, &bob]);
    bob.wait_for_peer(alice.uuid(), PeerState::Direct)
        .await
        .unwrap();

    alice.engine().invite(bob.uuid());
    let from = alice.uuid();
    let invited = tokio::time::timeout(
        Duration::from_secs(2),
        bob.wait_for("an invite", |e| *e == Event::Invite { from }),
    )
    .await;
    assert!(!matches!(invited, Ok(Ok(_))), "the invite got through");
    assert!(bob.engine().accept_invite(from).is_err());
}
