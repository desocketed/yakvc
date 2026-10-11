//! A voice sink (#174): another mod's renderer gets the voices instead of the
//! speakers, as decoded, under the same rules of who is heard.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::time::{Instant, sleep};
use yakvc_client::{Event, PeerState, Uuid, Vec3, VoiceFrame, VoiceSink};
use yakvc_testkit::{TestClient, TestNet, see_each_other};

/// Peak level above which a frame counts as audible: -60 dBFS.
const AUDIBLE: f32 = 0.001;

/// One frame as the sink got it.
#[derive(Debug, Clone, PartialEq)]
struct Got {
    uuid: Uuid,
    samples: usize,
    peak: f32,
    volume: f32,
    range: f32,
    group: bool,
    positioned: bool,
    heard_from: Vec3,
    occlusion: f32,
}

/// Records what it gets, paced in real time like speakers.
#[derive(Clone)]
struct Recorder {
    start: std::time::Instant,
    wanted: Arc<Mutex<u32>>,
    got: Arc<Mutex<Vec<Got>>>,
    ended: Arc<Mutex<Vec<Uuid>>>,
}

impl Recorder {
    fn new() -> Self {
        Recorder {
            start: std::time::Instant::now(),
            wanted: Arc::default(),
            got: Arc::default(),
            ended: Arc::default(),
        }
    }

    /// The latest audible frame from `uuid`.
    fn heard_from(&self, uuid: Uuid) -> Option<Got> {
        let got = self.got.lock().unwrap();
        got.iter()
            .rfind(|g| g.uuid == uuid && g.peak > AUDIBLE)
            .cloned()
    }
}

impl VoiceSink for Recorder {
    fn wants_frame(&mut self) -> bool {
        let mut wanted = self.wanted.lock().unwrap();
        let due = self.start.elapsed() >= Duration::from_millis(20) * *wanted;
        if due {
            *wanted += 1;
        }
        due
    }

    fn frame(&mut self, frame: &VoiceFrame<'_>) {
        let peak = frame.pcm.iter().fold(0.0f32, |peak, s| peak.max(s.abs()));
        self.got.lock().unwrap().push(Got {
            uuid: frame.uuid,
            samples: frame.pcm.len(),
            peak,
            volume: frame.volume,
            range: frame.range,
            group: frame.group,
            positioned: frame.positioned,
            heard_from: frame.heard_from,
            occlusion: frame.occlusion,
        });
    }

    fn stream_end(&mut self, uuid: Uuid) {
        self.ended.lock().unwrap().push(uuid);
    }
}

/// Puts a recorder in `client`'s place of its speakers, once the audio
/// thread has switched.
async fn record(client: &TestClient) -> Recorder {
    let recorder = Recorder::new();
    client
        .engine()
        .set_voice_sink(Some(Box::new(recorder.clone())));
    eventually("the switch to the sink", || {
        client.engine().audio_stats().external_output
    })
    .await;
    recorder
}

/// Waits up to 10 s for `recorder` to get audible voice from `uuid`.
async fn voice_from(recorder: &Recorder, uuid: Uuid) -> Got {
    eventually("voice in the sink", || recorder.heard_from(uuid).is_some()).await;
    recorder.heard_from(uuid).unwrap()
}

async fn eventually(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        sleep(Duration::from_millis(20)).await;
    }
}

/// Two talkers that see each other, connected directly.
async fn pair(net: &TestNet) -> (TestClient, TestClient) {
    let alice = net.client().name("alice").tone(440.0).start().await;
    let mut bob = net.client().name("bob").start().await;
    see_each_other(&[&alice, &bob]);
    bob.wait_for_peer(alice.uuid(), PeerState::Direct)
        .await
        .unwrap();
    (alice, bob)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_sink_gets_nearby_voice_instead_of_the_speakers() {
    let net = TestNet::start().await;
    let (alice, mut bob) = pair(&net).await;
    alice.move_to(Vec3::new(0.0, 64.0, 0.0));
    bob.move_to(Vec3::new(20.0, 64.0, 0.0));
    let recorder = record(&bob).await;
    let speakers = bob.recording().audible_frames();
    alice.talk(true);

    let got = voice_from(&recorder, alice.uuid()).await;
    assert_eq!(got.samples, 960);
    assert!(got.positioned && !got.group, "{got:?}");
    assert_eq!(got.range, 48.0);
    assert_eq!(got.volume, 1.0);
    // Nothing in the way: heard from where alice stands, as the speakers would,
    // once bob's world has her there (her first frames can beat her move).
    let stands = Vec3::new(0.0, 64.0, 0.0);
    eventually("alice heard from where she stands", || {
        recorder
            .heard_from(alice.uuid())
            .is_some_and(|got| got.heard_from == stands)
    })
    .await;
    assert_eq!(recorder.heard_from(alice.uuid()).unwrap().occlusion, 0.0);
    // The talking indicator still works, and the speakers stay quiet.
    let uuid = alice.uuid();
    bob.wait_for(
        "alice talking",
        |e| matches!(e, Event::Talking { uuid: u, talking: true } if *u == uuid),
    )
    .await
    .unwrap();
    assert_eq!(bob.recording().audible_frames(), speakers);

    // Out of range, the sink gets nothing more, as the speakers wouldn't.
    bob.move_to(Vec3::new(100.0, 64.0, 0.0));
    sleep(Duration::from_millis(500)).await;
    let count = recorder.got.lock().unwrap().len();
    sleep(Duration::from_millis(500)).await;
    assert_eq!(
        recorder.got.lock().unwrap().len(),
        count,
        "heard beyond range"
    );

    // When the link goes, the sink hears that the stream ended.
    bob.sees(&[]);
    eventually("the stream's end", || {
        recorder.ended.lock().unwrap().contains(&uuid)
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_sink_gets_group_voice_unpositioned() {
    let net = TestNet::start().await;
    let (alice, mut bob) = pair(&net).await;
    // Too far apart for proximity voice.
    alice.move_to(Vec3::new(0.0, 64.0, 0.0));
    bob.move_to(Vec3::new(1000.0, 64.0, 0.0));
    let from = alice.uuid();
    alice.engine().invite(bob.uuid());
    bob.wait_for("an invite", |e| *e == Event::Invite { from })
        .await
        .unwrap();
    bob.engine().accept_invite(from).unwrap();
    let recorder = record(&bob).await;
    alice.talk(true);

    let got = voice_from(&recorder, alice.uuid()).await;
    assert!(got.group && !got.positioned, "{got:?}");
    assert_eq!(got.range, 0.0);
    assert_eq!(got.heard_from, Vec3::default());
}
