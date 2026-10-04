//! The whole send and receive path through the public API, as the voice loop
//! drives it, with test I/O instead of hardware.

use std::time::{Duration, Instant};

use yakvc_audio::{
    Encoder, FRAME_SAMPLES, FrameSink, FrameSource, InputConfig, InputProcessor, JitterConfig,
    Mixer, NullSink, Packet, Pulled, ReceiveStream, Spatial, ToneSource,
};

#[test]
fn tone_reaches_the_sink() {
    let mut source = ToneSource::new(440.0);
    let mut input = InputProcessor::new(InputConfig {
        noise_suppression: false,
        ..InputConfig::default()
    });
    let mut encoder = Encoder::new(24_000).unwrap();
    let mut stream = ReceiveStream::new(JitterConfig::default());
    let mut mixer = Mixer::new();
    let (mut sink, recording) = NullSink::new();

    let mut captured = [0.0; FRAME_SAMPLES];
    let mut decoded = [0.0; FRAME_SAMPLES];
    let mut mixed = [[0.0; 2]; FRAME_SAMPLES];
    let (mut seq, mut ts) = (0u32, 0u32);
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(600) {
        if source.read(&mut captured) {
            let activity = input.process(&mut captured);
            assert!(activity.voice);
            if let Some(payload) = encoder.encode(&captured).unwrap() {
                let packet = Packet {
                    seq,
                    ts,
                    end_of_talk: false,
                    payload,
                };
                stream.push(packet, Instant::now());
                seq += 1;
            }
            ts += FRAME_SAMPLES as u32;
        }
        if sink.wants_frame() {
            let pulled = stream.pull(&mut decoded, Instant::now());
            let sources = (pulled == Pulled::Voice).then_some((
                &decoded,
                Spatial {
                    gain: 1.0,
                    pan: 0.0,
                },
            ));
            mixer.mix(sources, &mut mixed);
            sink.write(&mixed);
        }
        std::thread::sleep(Duration::from_millis(2));
    }

    // About 30 frames in 600 ms, all but the first couple audible.
    let frames = recording.frames();
    assert!((25..=35).contains(&frames), "{frames}");
    assert!(recording.audible_frames() >= frames - 5);
    assert_eq!(stream.stats().concealed, 0);
}
