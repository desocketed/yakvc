//! M3 acceptance tests (DESIGN.md milestones): matching, spatial audio,
//! audio under loss and jitter, spectators, deafen, relay-only and range.

use std::time::Duration;

use tokio::time::sleep;
use yakvc_audio::FRAME_SAMPLES;
use yakvc_client::sim::{Impairment, Loss};
use yakvc_client::{Input, PeerState, Vec3};
use yakvc_testkit::{TestClient, TestNet, see_each_other};

/// Peak level above which a frame counts as audible: -60 dBFS.
const AUDIBLE: f32 = 0.001;

/// Long enough for a world tick, mixer interpolation and a few frames in
/// flight.
const SETTLE: Duration = Duration::from_millis(500);

fn at(x: f64, z: f64) -> Vec3 {
    Vec3::new(x, 64.0, z)
}

/// Discards what `client` has played so far, then returns what it plays
/// during `window`.
async fn listen(client: &TestClient, window: Duration) -> Vec<[f32; 2]> {
    client.recording().take();
    sleep(window).await;
    client.recording().take()
}

/// Root-mean-square level of the left and right channels.
fn levels(samples: &[[f32; 2]]) -> (f32, f32) {
    let n = samples.len().max(1) as f32;
    let left = samples.iter().map(|s| s[0] * s[0]).sum::<f32>() / n;
    let right = samples.iter().map(|s| s[1] * s[1]).sum::<f32>() / n;
    (left.sqrt(), right.sqrt())
}

/// For each 20 ms frame, whether it was audible.
fn audible_frames(samples: &[[f32; 2]]) -> Vec<bool> {
    samples
        .chunks(FRAME_SAMPLES)
        .map(|frame| frame.iter().flatten().any(|s| s.abs() > AUDIBLE))
        .collect()
}

fn is_silent(samples: &[[f32; 2]]) -> bool {
    !audible_frames(samples).contains(&true)
}

/// Makes `a` and `b` see each other and waits until each has the other as a
/// peer in `state`.
async fn connect(a: &mut TestClient, b: &mut TestClient, state: PeerState) {
    see_each_other(&[a, b]);
    a.wait_for_peer(b.uuid(), state).await.unwrap();
    b.wait_for_peer(a.uuid(), state).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn only_mutual_tab_list_matches_connect() {
    let net = TestNet::start().await;
    let mut alice = net.client().name("alice").start().await;
    let mut bob = net.client().name("bob").start().await;
    let mut carol = net.client().name("carol").start().await;

    // alice ↔ bob is mutual. alice sees carol and carol sees bob, but neither
    // is seen back.
    alice.sees(&[&bob, &carol]);
    bob.sees(&[&alice]);
    carol.sees(&[&bob]);

    alice
        .wait_for_peer(bob.uuid(), PeerState::Direct)
        .await
        .unwrap();
    bob.wait_for_peer(alice.uuid(), PeerState::Direct)
        .await
        .unwrap();
    sleep(Duration::from_secs(2)).await;

    let peers_of =
        |c: &TestClient| -> Vec<_> { c.engine().peers().iter().map(|p| p.uuid).collect() };
    assert_eq!(peers_of(&alice), [bob.uuid()]);
    assert_eq!(peers_of(&bob), [alice.uuid()]);
    assert!(peers_of(&carol).is_empty());
    assert_eq!(net.server().stats().matches, 1);

    // Once carol also sees alice, they match too.
    carol.sees(&[&bob, &alice]);
    carol
        .wait_for_peer(alice.uuid(), PeerState::Direct)
        .await
        .unwrap();
    alice
        .wait_for_peer(carol.uuid(), PeerState::Direct)
        .await
        .unwrap();
    assert_eq!(net.server().stats().matches, 2);

    // Dropping someone from the tab list ends the match.
    alice.sees(&[&bob]);
    carol
        .wait_for_peer(alice.uuid(), PeerState::Gone)
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn gain_and_pan_follow_moves() {
    let net = TestNet::start().await;
    let mut alice = net.client().name("alice").tone(440.0).start().await;
    let mut bob = net.client().name("bob").start().await;
    bob.move_to(at(0.0, 0.0));
    alice.move_to(at(3.0, 0.0));
    connect(&mut alice, &mut bob, PeerState::Direct).await;
    alice.talk(true);
    sleep(SETTLE).await;

    // bob faces +Z (yaw 0), so +X is on his left.
    let (left, right) = levels(&listen(&bob, Duration::from_secs(1)).await);
    assert!(
        left > 2.0 * right,
        "alice at +X: left {left}, right {right}"
    );

    alice.move_to(at(-3.0, 0.0));
    sleep(SETTLE).await;
    let (left, right) = levels(&listen(&bob, Duration::from_secs(1)).await);
    assert!(
        right > 2.0 * left,
        "alice at -X: left {left}, right {right}"
    );

    // Straight ahead: centred. Full volume within 4 blocks.
    alice.move_to(at(0.0, 3.0));
    sleep(SETTLE).await;
    let (near_l, near_r) = levels(&listen(&bob, Duration::from_secs(1)).await);
    assert!(
        (near_l - near_r).abs() < 0.1 * near_l,
        "ahead: {near_l} vs {near_r}"
    );

    // At 30 blocks the gain is about 1 - 26/44 = 0.41.
    alice.move_to(at(0.0, 30.0));
    sleep(SETTLE).await;
    let (far_l, _) = levels(&listen(&bob, Duration::from_secs(1)).await);
    assert!(far_l > 0.0, "still audible at 30 blocks");
    assert!(
        (0.25..0.6).contains(&(far_l / near_l)),
        "gain at 30 blocks: {}",
        far_l / near_l
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn audio_stays_continuous_under_bursty_loss_and_jitter() {
    let net = TestNet::start().await;
    let mut alice = net.client().name("alice").tone(440.0).start().await;
    let mut bob = net
        .client()
        .name("bob")
        .impairment(Impairment {
            loss: Loss::Bursty {
                rate: 0.05,
                mean_burst: 2.0,
            },
            jitter: Duration::from_millis(30),
            seed: 3,
            ..Impairment::default()
        })
        .start()
        .await;
    alice.move_to(at(0.0, 0.0));
    bob.move_to(at(2.0, 0.0));
    connect(&mut alice, &mut bob, PeerState::Direct).await;

    bob.recording().take();
    alice.talk(true);
    sleep(Duration::from_secs(5)).await;
    alice.talk(false);
    sleep(SETTLE).await;
    let played = bob.recording().take();

    // One unbroken run of audible frames: every lost or late frame was
    // filled by FEC or PLC instead of dropping to silence.
    let audible = audible_frames(&played);
    let first = audible.iter().position(|&a| a).expect("bob heard alice");
    let last = audible.iter().rposition(|&a| a).unwrap();
    let gaps: Vec<usize> = (first..=last).filter(|&i| !audible[i]).collect();
    assert!(gaps.is_empty(), "silent frames at {gaps:?}");
    // 5 s is 250 frames; allow for push-to-talk and playout edges.
    let run = last - first + 1;
    assert!((235..=265).contains(&run), "talk spurt was {run} frames");

    let stats = bob.stream_stats(alice.uuid()).expect("alice is a peer");
    assert!(
        stats.fec_recovered + stats.concealed > 0,
        "impairment dropped nothing: {stats:?}"
    );
    let expected = stats.received + stats.fec_recovered + stats.concealed;
    assert!(
        expected as usize >= run,
        "frames unaccounted for: {stats:?}"
    );
    assert!(
        (Duration::from_millis(20)..=Duration::from_millis(200)).contains(&stats.playout_delay),
        "playout delay {:?}",
        stats.playout_delay
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn spectators_neither_send_nor_hear() {
    let net = TestNet::start().await;
    let mut alice = net.client().name("alice").tone(440.0).start().await;
    let mut bob = net.client().name("bob").tone(660.0).start().await;
    alice.move_to(at(0.0, 0.0));
    bob.move_to(at(2.0, 0.0));
    connect(&mut alice, &mut bob, PeerState::Direct).await;

    alice.set_spectator(true);
    alice.talk(true);
    sleep(SETTLE).await;
    assert!(
        is_silent(&listen(&bob, Duration::from_secs(1)).await),
        "a spectator's voice was played"
    );

    alice.talk(false);
    bob.talk(true);
    sleep(SETTLE).await;
    assert!(
        is_silent(&listen(&alice, Duration::from_secs(1)).await),
        "a spectator heard someone"
    );

    // Control: once alice stops spectating she hears bob.
    alice.set_spectator(false);
    sleep(SETTLE).await;
    assert!(!is_silent(&listen(&alice, Duration::from_secs(1)).await));
}

/// The receiver half of the spectator rule: a spectator has no entity in
/// anyone else's world, so even a client that ignores the rule and sends
/// anyway is not played.
#[tokio::test(flavor = "multi_thread")]
async fn a_spectator_is_left_out_of_others_worlds() {
    let net = TestNet::start().await;
    let mut alice = net.client().name("alice").tone(440.0).start().await;
    let mut bob = net.client().name("bob").start().await;
    alice.move_to(at(0.0, 0.0));
    bob.move_to(at(2.0, 0.0));
    connect(&mut alice, &mut bob, PeerState::Direct).await;

    alice.set_spectator(true);
    // A modified client: it talks while its engine is not told it spectates.
    alice.set_input(Input {
        push_to_talk: true,
        ..Input::default()
    });
    sleep(SETTLE).await;
    assert!(
        is_silent(&listen(&bob, Duration::from_secs(1)).await),
        "a spectator without an entity was played"
    );

    // Control: back in bob's world, alice is heard.
    alice.set_spectator(false);
    sleep(SETTLE).await;
    assert!(!is_silent(&listen(&bob, Duration::from_secs(1)).await));
}

/// Each side filters by its own range, so between two players the shorter
/// range applies, whoever is talking.
#[tokio::test(flavor = "multi_thread")]
async fn the_shorter_voice_range_wins() {
    let net = TestNet::start().await;
    let mut alice = net
        .client()
        .name("alice")
        .tone(440.0)
        .config(|c| c.voice_range = 20.0)
        .start()
        .await;
    // bob keeps the default 48-block range.
    let mut bob = net.client().name("bob").tone(660.0).start().await;
    alice.move_to(at(0.0, 0.0));
    bob.move_to(at(18.0, 0.0));
    connect(&mut alice, &mut bob, PeerState::Direct).await;
    alice.talk(true);
    bob.talk(true);

    // Inside both ranges: each hears the other.
    sleep(SETTLE).await;
    assert!(!is_silent(&listen(&bob, Duration::from_secs(1)).await));
    assert!(!is_silent(&listen(&alice, Duration::from_secs(1)).await));

    // Just outside alice's range but well inside bob's: alice sends nothing
    // to bob, and plays nothing from him.
    bob.move_to(at(22.0, 0.0));
    sleep(SETTLE).await;
    assert!(
        is_silent(&listen(&bob, Duration::from_secs(1)).await),
        "bob heard alice beyond her range"
    );
    assert!(
        is_silent(&listen(&alice, Duration::from_secs(1)).await),
        "alice heard bob beyond her range"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn deafened_listener_hears_nothing_and_speaker_stops_sending() {
    let net = TestNet::start().await;
    let mut alice = net.client().name("alice").tone(440.0).start().await;
    let mut bob = net.client().name("bob").start().await;
    alice.move_to(at(0.0, 0.0));
    bob.move_to(at(2.0, 0.0));
    connect(&mut alice, &mut bob, PeerState::Direct).await;

    bob.engine().set_input(Input {
        deafened: true,
        ..Input::default()
    });
    // Let `ReceiveState { wants_audio: false }` reach alice.
    sleep(SETTLE).await;
    let received_before = bob.stream_stats(alice.uuid()).map_or(0, |s| s.received);

    alice.talk(true);
    sleep(SETTLE).await;
    assert!(is_silent(&listen(&bob, Duration::from_secs(1)).await));
    let received_after = bob.stream_stats(alice.uuid()).map_or(0, |s| s.received);
    assert_eq!(
        received_after, received_before,
        "alice kept sending to a deafened peer"
    );

    // Control: undeafened, bob hears alice again.
    bob.engine().set_input(Input::default());
    sleep(SETTLE).await;
    assert!(!is_silent(&listen(&bob, Duration::from_secs(1)).await));
}

#[tokio::test(flavor = "multi_thread")]
async fn relay_only_client_talks_through_the_relay() {
    let net = TestNet::start().await;
    let mut alice = net.client().name("alice").tone(440.0).start().await;
    let mut bob = net.client().name("bob").relay_only().start().await;
    alice.move_to(at(0.0, 0.0));
    bob.move_to(at(2.0, 0.0));
    connect(&mut alice, &mut bob, PeerState::Relayed).await;

    alice.talk(true);
    sleep(SETTLE).await;
    assert!(!is_silent(&listen(&bob, Duration::from_secs(1)).await));

    let direct_peers = bob
        .engine()
        .peers()
        .iter()
        .filter(|p| p.state == PeerState::Direct)
        .count();
    assert_eq!(direct_peers, 0, "a relay-only client went direct");
}

#[tokio::test(flavor = "multi_thread")]
async fn voice_stops_at_range_and_without_a_tracked_entity() {
    let net = TestNet::start().await;
    let mut alice = net.client().name("alice").tone(440.0).start().await;
    let mut bob = net.client().name("bob").start().await;
    alice.move_to(at(0.0, 0.0));
    bob.move_to(at(40.0, 0.0));
    connect(&mut alice, &mut bob, PeerState::Direct).await;
    alice.talk(true);

    // Inside the default 48-block range, quiet but audible.
    sleep(SETTLE).await;
    assert!(!is_silent(&listen(&bob, Duration::from_secs(1)).await));

    // Out of tracking range: bob has no entity for alice, so nothing plays,
    // but they stay connected.
    bob.move_to(at(60.0, 0.0));
    sleep(SETTLE).await;
    assert!(is_silent(&listen(&bob, Duration::from_secs(1)).await));
    bob.wait_for_peer(alice.uuid(), PeerState::Direct)
        .await
        .unwrap();

    // Back in range: audible again.
    bob.move_to(at(10.0, 0.0));
    sleep(SETTLE).await;
    assert!(!is_silent(&listen(&bob, Duration::from_secs(1)).await));
}
