//! The whole send and receive path through the public API, as the voice loop
//! drives it, with test I/O instead of hardware.

use std::f32::consts::TAU;
use std::time::{Duration, Instant};

use yakvc_audio::{
    Encoder, FRAME_DURATION, FRAME_SAMPLES, InputConfig, InputProcessor, JitterConfig, Mixer,
    MonoFrame, Packet, Pulled, ReceiveStream, SAMPLE_RATE, Spatial,
};

/// Runs on a simulated clock: 30 frames, one every 20 ms, with playout out of
/// phase with capture. The real-time pacing of the test sources and sinks
/// has its own tests; on the wall clock a stall of the test process itself
/// drops frames.
#[test]
fn tone_reaches_the_sink() {
    const FRAMES: u32 = 30;
    let mut input = InputProcessor::new(InputConfig {
        noise_suppression: false,
        ..InputConfig::default()
    });
    let mut encoder = Encoder::new(24_000).unwrap();
    let mut stream = ReceiveStream::new(JitterConfig::default());
    let mut mixer = Mixer::new();

    let mut decoded = [0.0; FRAME_SAMPLES];
    let mut mixed = [[0.0; 2]; FRAME_SAMPLES];
    let mut audible = 0;
    let start = Instant::now();
    for n in 0..FRAMES {
        let ts = n * FRAME_SAMPLES as u32;
        let mut captured: MonoFrame = std::array::from_fn(|i| {
            let t = (ts + i as u32) as f32 / SAMPLE_RATE as f32;
            0.25 * (TAU * 440.0 * t).sin()
        });
        let activity = input.process(&mut captured);
        assert!(activity.voice);
        if let Some(payload) = encoder.encode(&captured).unwrap() {
            let packet = Packet {
                seq: n,
                ts,
                end_of_talk: false,
                payload,
            };
            stream.push(packet, start + FRAME_DURATION * n);
        }

        let now = start + FRAME_DURATION * n + Duration::from_millis(7);
        let pulled = stream.pull(&mut decoded, now);
        let sources = (pulled == Pulled::Voice).then_some((
            &decoded,
            Spatial {
                gain: 1.0,
                pan: 0.0,
                muffle: 0.0,
                reverb: 0.0,
            },
        ));
        mixer.mix(sources, &mut mixed);
        audible += usize::from(mixed.iter().flatten().any(|s| s.abs() > 0.001));
    }

    // All but the first couple audible.
    assert!(audible >= FRAMES as usize - 5, "{audible}");
    assert_eq!(stream.stats().concealed, 0);
}
