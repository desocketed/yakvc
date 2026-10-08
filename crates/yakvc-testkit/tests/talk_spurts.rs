//! How talk spurts end on the wire.

use std::path::PathBuf;
use std::time::Duration;

use yakvc_client::{Activation, Event, PeerState, Vec3};
use yakvc_testkit::{TestClient, TestNet, see_each_other};

/// A looped WAV of `talk` of tone, then `pause` of silence.
fn talk_and_pause(talk: Duration, pause: Duration) -> PathBuf {
    let path = std::env::temp_dir().join(format!("yakvc-spurts-{}.wav", std::process::id()));
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 48_000,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut wav = hound::WavWriter::create(&path, spec).unwrap();
    let talk = (talk.as_secs_f32() * 48_000.0) as usize;
    let pause = (pause.as_secs_f32() * 48_000.0) as usize;
    for i in 0..talk {
        let t = i as f32 / 48_000.0;
        wav.write_sample(0.3 * (std::f32::consts::TAU * 440.0 * t).sin())
            .unwrap();
    }
    for _ in 0..pause {
        wav.write_sample(0.0f32).unwrap();
    }
    wav.finalize().unwrap();
    path
}

/// Waits for `speaker` to start and then stop talking, as `listener` hears it.
async fn one_spurt(listener: &mut TestClient, speaker: &TestClient) {
    for want in [true, false] {
        let uuid = speaker.uuid();
        listener
            .wait_for("a talk spurt", |e| {
                matches!(e, Event::Talking { uuid: u, talking } if *u == uuid && *talking == want)
            })
            .await
            .unwrap();
    }
}

/// A speaker whose link closes mid-spurt stops showing as talking.
#[tokio::test(flavor = "multi_thread")]
async fn a_lost_speaker_stops_talking() {
    let net = TestNet::start().await;
    let alice = net.client().name("alice").tone(440.0).start().await;
    let mut bob = net.client().name("bob").start().await;
    see_each_other(&[&alice, &bob]);
    alice.move_to(Vec3::new(0.0, 64.0, 0.0));
    bob.move_to(Vec3::new(5.0, 64.0, 0.0));
    bob.wait_for_peer(alice.uuid(), PeerState::Direct)
        .await
        .unwrap();
    alice.talk(true);
    let uuid = alice.uuid();
    let talking = |want: bool| move |e: &Event| matches!(e, Event::Talking { uuid: u, talking } if *u == uuid && *talking == want);
    bob.wait_for("alice talking", talking(true)).await.unwrap();
    // Bob no longer sees alice, which closes the link while she talks.
    bob.sees(&[]);
    bob.wait_for("alice gone quiet", talking(false))
        .await
        .unwrap();
}

/// With voice activation, the 300 ms hangover outlasts the 200 ms before DTX
/// goes quiet, so the spurt's last frame is silenced. The end of talk still
/// goes out, header only (#71). Noise suppression is off because RNNoise
/// leaves enough in the silence to keep DTX from starting that soon.
#[tokio::test(flavor = "multi_thread")]
async fn end_of_talk_is_sent_after_dtx_silence() {
    let net = TestNet::start().await;
    let wav = talk_and_pause(Duration::from_millis(400), Duration::from_millis(1600));
    let mut alice = net
        .client()
        .name("alice")
        .wav(&wav)
        .config(|c| {
            c.audio.activation = Activation::Voice;
            c.audio.noise_suppression = false;
        })
        .start()
        .await;
    let mut bob = net.client().name("bob").start().await;
    see_each_other(&[&alice, &bob]);
    alice.move_to(Vec3::new(0.0, 64.0, 0.0));
    bob.move_to(Vec3::new(5.0, 64.0, 0.0));
    bob.wait_for_peer(alice.uuid(), PeerState::Direct)
        .await
        .unwrap();
    alice
        .wait_for_peer(bob.uuid(), PeerState::Direct)
        .await
        .unwrap();

    // Skip a spurt that may have started before bob was in range.
    one_spurt(&mut bob, &alice).await;
    let before = bob.stream_stats(alice.uuid()).unwrap().received;
    one_spurt(&mut bob, &alice).await;
    let after = bob.stream_stats(alice.uuid()).unwrap().received;
    let _ = std::fs::remove_file(&wav);
    // 20 frames of tone, 10 of silence before DTX stops sending, and the end
    // of talk at the end of the hangover.
    assert_eq!(after - before, 20 + 10 + 1);
}
