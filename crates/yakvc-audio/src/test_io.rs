//! Frame sources and sinks that need no audio hardware, for the CLI's
//! `--wav` / `--null-out` options, bots and integration tests.

use std::f32::consts::TAU;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use crate::resample::Resampler;
use crate::{
    AudioError, FRAME_DURATION, FrameSink, FrameSource, MonoFrame, SAMPLE_RATE, StereoFrame,
};

/// Loud enough to pass the default VAD threshold, quiet enough to leave room
/// when several tones are mixed.
const TONE_AMPLITUDE: f32 = 0.25;

/// `-60` dBFS.
const AUDIBLE_PEAK: f32 = 0.001;

/// A caller further behind than this skips ahead, like a sound card dropping
/// audio nobody handled in time, so a stalled caller gets no burst of frames.
const MAX_BACKLOG_FRAMES: u32 = 5;

/// Hands out one frame every 20 ms of wall-clock time, like a sound card.
#[derive(Debug, Default)]
struct Clock {
    /// Set by the first frame.
    start: Option<Instant>,
    frames: u32,
}

impl Clock {
    /// Whether the next frame is due. The first one always is.
    fn due(&self) -> bool {
        self.start
            .is_none_or(|start| start.elapsed() >= FRAME_DURATION * self.frames)
    }

    /// Counts one frame.
    fn tick(&mut self) {
        let start = *self.start.get_or_insert_with(Instant::now);
        let elapsed_frames = (start.elapsed().as_millis() / FRAME_DURATION.as_millis()) as u32;
        self.frames = (self.frames + 1).max(elapsed_frames.saturating_sub(MAX_BACKLOG_FRAMES));
    }
}

/// Plays a WAV file (any rate or channel count), paced in real time.
#[derive(Debug)]
pub struct WavSource {
    /// The whole file, mono at [`SAMPLE_RATE`].
    samples: Vec<f32>,
    position: usize,
    looping: bool,
    clock: Clock,
}

impl WavSource {
    pub fn open(path: &Path, looping: bool) -> Result<Self, AudioError> {
        let reader = hound::WavReader::open(path).map_err(wav_error)?;
        let spec = reader.spec();
        let interleaved: Vec<f32> = match spec.sample_format {
            hound::SampleFormat::Float => reader
                .into_samples::<f32>()
                .collect::<Result<_, _>>()
                .map_err(wav_error)?,
            hound::SampleFormat::Int => {
                let scale = 1.0 / (1u64 << (spec.bits_per_sample - 1)) as f32;
                reader
                    .into_samples::<i32>()
                    .map(|s| s.map(|s| s as f32 * scale))
                    .collect::<Result<_, _>>()
                    .map_err(wav_error)?
            }
        };
        let channels = usize::from(spec.channels);
        let mono: Vec<f32> = interleaved
            .chunks_exact(channels)
            .map(|frame| frame.iter().sum::<f32>() / channels as f32)
            .collect();

        let mut samples = Vec::with_capacity(mono.len());
        let mut resampler = Resampler::new(spec.sample_rate, SAMPLE_RATE, 1)?;
        resampler.process(&mono, &mut samples);
        if spec.sample_rate != SAMPLE_RATE {
            // Push the resampler's last partial chunk through.
            let flush = vec![0.0; spec.sample_rate as usize / 50];
            resampler.process(&flush, &mut samples);
        }

        Ok(WavSource {
            samples,
            position: 0,
            looping,
            clock: Clock::default(),
        })
    }
}

impl FrameSource for WavSource {
    fn read(&mut self, frame: &mut MonoFrame) -> bool {
        if self.position >= self.samples.len() || !self.clock.due() {
            return false;
        }
        self.clock.tick();
        for sample in frame.iter_mut() {
            // A file that does not loop ends with a partial frame padded
            // with silence.
            *sample = self.samples.get(self.position).copied().unwrap_or(0.0);
            self.position += 1;
            if self.looping && self.position == self.samples.len() {
                self.position = 0;
            }
        }
        true
    }
}

fn wav_error(err: hound::Error) -> AudioError {
    match err {
        hound::Error::IoError(err) => AudioError::Io(err),
        other => AudioError::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, other)),
    }
}

/// A continuous sine tone, paced in real time.
#[derive(Debug)]
pub struct ToneSource {
    /// Phase advance per sample, in radians.
    step: f32,
    phase: f32,
    clock: Clock,
}

impl ToneSource {
    pub fn new(frequency_hz: f32) -> Self {
        ToneSource {
            step: TAU * frequency_hz / SAMPLE_RATE as f32,
            phase: 0.0,
            clock: Clock::default(),
        }
    }
}

impl FrameSource for ToneSource {
    fn read(&mut self, frame: &mut MonoFrame) -> bool {
        if !self.clock.due() {
            return false;
        }
        self.clock.tick();
        for sample in frame.iter_mut() {
            *sample = TONE_AMPLITUDE * self.phase.sin();
            self.phase = (self.phase + self.step) % TAU;
        }
        true
    }
}

/// Silence, paced in real time: a player who never speaks.
#[derive(Debug, Default)]
pub struct SilenceSource {
    clock: Clock,
}

impl SilenceSource {
    pub fn new() -> Self {
        SilenceSource::default()
    }
}

impl FrameSource for SilenceSource {
    fn read(&mut self, frame: &mut MonoFrame) -> bool {
        if !self.clock.due() {
            return false;
        }
        self.clock.tick();
        frame.fill(0.0);
        true
    }
}

/// Accepts frames in real time and records them. Cloning the [`Recording`]
/// returned by [`NullSink::new`] lets a test inspect output while the engine
/// owns the sink.
#[derive(Debug)]
pub struct NullSink {
    recording: Recording,
    clock: Clock,
}

/// Shared view of everything a [`NullSink`] has received.
#[derive(Debug, Clone)]
pub struct Recording {
    shared: Arc<Mutex<Recorded>>,
}

#[derive(Debug, Default)]
struct Recorded {
    frames: usize,
    audible_frames: usize,
    samples: Vec<[f32; 2]>,
}

impl NullSink {
    pub fn new() -> (NullSink, Recording) {
        let recording = Recording {
            shared: Arc::default(),
        };
        let sink = NullSink {
            recording: recording.clone(),
            clock: Clock::default(),
        };
        (sink, recording)
    }
}

impl FrameSink for NullSink {
    fn wants_frame(&self) -> bool {
        self.clock.due()
    }

    fn write(&mut self, frame: &StereoFrame) {
        self.clock.tick();
        let audible = frame.iter().flatten().any(|s| s.abs() > AUDIBLE_PEAK);
        let mut recorded = lock(&self.recording.shared);
        recorded.frames += 1;
        recorded.audible_frames += usize::from(audible);
        recorded.samples.extend_from_slice(frame);
    }
}

impl Recording {
    /// Frames received so far.
    pub fn frames(&self) -> usize {
        lock(&self.shared).frames
    }

    /// Frames whose peak level exceeded `-60` dBFS.
    pub fn audible_frames(&self) -> usize {
        lock(&self.shared).audible_frames
    }

    /// Takes the samples recorded since the last call, leaving the buffer
    /// empty. [`frames`](Self::frames) and
    /// [`audible_frames`](Self::audible_frames) keep counting from the start.
    pub fn take(&self) -> Vec<[f32; 2]> {
        std::mem::take(&mut lock(&self.shared).samples)
    }
}

/// Locks a mutex, ignoring poisoning: the data is plain counters and samples
/// that stay valid if a holder panicked.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::FRAME_SAMPLES;
    use crate::input::level_db;

    /// Makes the next frame due without waiting.
    fn skip_pacing(clock: &mut Clock) {
        *clock = Clock::default();
    }

    #[test]
    fn clock_paces_and_skips_backlog() {
        let mut clock = Clock::default();
        assert!(clock.due());
        clock.tick();
        assert!(!clock.due(), "second frame is not due yet");
        clock.start = Some(Instant::now() - Duration::from_secs(3600));
        assert!(clock.due());
        clock.tick();
        // An hour behind, the clock skipped to within the backlog limit.
        for _ in 0..=MAX_BACKLOG_FRAMES {
            assert!(clock.due());
            clock.tick();
        }
        assert!(!clock.due());
    }

    #[test]
    fn tone_is_paced_in_real_time() {
        let mut tone = ToneSource::new(440.0);
        let mut frame = [0.0; FRAME_SAMPLES];
        assert!(tone.read(&mut frame));
        assert!(!tone.read(&mut frame), "second frame is not due yet");
        std::thread::sleep(FRAME_DURATION + Duration::from_millis(5));
        assert!(tone.read(&mut frame));
        let expected = 20.0 * (TONE_AMPLITUDE / 2f32.sqrt()).log10();
        assert!((level_db(&frame) - expected).abs() < 0.1);
    }

    #[test]
    fn tone_is_continuous_across_frames() {
        let mut tone = ToneSource::new(1000.0);
        let mut a = [0.0; FRAME_SAMPLES];
        let mut b = [0.0; FRAME_SAMPLES];
        assert!(tone.read(&mut a));
        skip_pacing(&mut tone.clock);
        assert!(tone.read(&mut b));
        let step = TAU * 1000.0 / SAMPLE_RATE as f32;
        let jump = (b[0] - a[FRAME_SAMPLES - 1]).abs();
        assert!(jump <= TONE_AMPLITUDE * step * 1.01, "{jump}");
    }

    #[test]
    fn silence_is_paced_in_real_time() {
        let mut silence = SilenceSource::new();
        let mut frame = [1.0; FRAME_SAMPLES];
        assert!(silence.read(&mut frame));
        assert!(frame.iter().all(|&s| s == 0.0));
        assert!(!silence.read(&mut frame), "second frame is not due yet");
        std::thread::sleep(FRAME_DURATION + Duration::from_millis(5));
        assert!(silence.read(&mut frame));
    }

    #[test]
    fn boxed_source_and_sink_forward_calls() {
        let mut source: Box<dyn FrameSource> = Box::new(SilenceSource::new());
        let mut frame = [1.0; FRAME_SAMPLES];
        assert!(FrameSource::read(&mut source, &mut frame));
        assert!(FrameSource::failure(&source).is_none());

        let (sink, recording) = NullSink::new();
        let mut sink: Box<dyn FrameSink> = Box::new(sink);
        assert!(FrameSink::wants_frame(&sink));
        FrameSink::write(&mut sink, &[[0.5; 2]; FRAME_SAMPLES]);
        assert_eq!(recording.frames(), 1);
        assert!(FrameSink::failure(&sink).is_none());
    }

    #[test]
    fn null_sink_paces_and_records() {
        let (mut sink, recording) = NullSink::new();
        assert!(sink.wants_frame());
        let mut frame = [[0.0; 2]; FRAME_SAMPLES];
        sink.write(&frame);
        assert!(!sink.wants_frame(), "second frame is not due yet");
        frame[100] = [0.5, -0.5];
        sink.write(&frame);
        sink.write(&[[0.0001; 2]; FRAME_SAMPLES]);
        assert_eq!(recording.frames(), 3);
        assert_eq!(recording.audible_frames(), 1);
        let samples = recording.take();
        assert_eq!(samples.len(), 3 * FRAME_SAMPLES);
        assert_eq!(samples[FRAME_SAMPLES + 100], [0.5, -0.5]);
        assert!(recording.take().is_empty());
        assert_eq!(recording.frames(), 3);
    }

    fn write_wav(name: &str, spec: hound::WavSpec, seconds: f32) -> std::path::PathBuf {
        let file = format!("yakvc-audio-{}-{name}.wav", std::process::id());
        let path = std::env::temp_dir().join(file);
        let mut writer = hound::WavWriter::create(&path, spec).unwrap();
        let len = (spec.sample_rate as f32 * seconds) as usize;
        for i in 0..len {
            let value = (TAU * 500.0 * i as f32 / spec.sample_rate as f32).sin() * 0.5;
            for _ in 0..spec.channels {
                match spec.sample_format {
                    hound::SampleFormat::Float => writer.write_sample(value).unwrap(),
                    hound::SampleFormat::Int => {
                        writer.write_sample((value * 32767.0) as i16).unwrap()
                    }
                }
            }
        }
        writer.finalize().unwrap();
        path
    }

    fn read_all(source: &mut WavSource) -> Vec<MonoFrame> {
        let mut frames = Vec::new();
        let mut frame = [0.0; FRAME_SAMPLES];
        loop {
            skip_pacing(&mut source.clock);
            if !source.read(&mut frame) {
                return frames;
            }
            frames.push(frame);
        }
    }

    #[test]
    fn wav_source_resamples_and_downmixes() {
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 44_100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let path = write_wav("stereo", spec, 0.5);
        let mut source = WavSource::open(&path, false).unwrap();
        std::fs::remove_file(&path).unwrap();
        let frames = read_all(&mut source);
        // 0.5 s is 25 frames at 48 kHz, plus a partial one from the flush.
        assert!((25..=27).contains(&frames.len()), "{}", frames.len());
        let peak = frames[10].iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!((peak - 0.5).abs() < 0.02, "{peak}");
    }

    #[test]
    fn wav_source_plays_once_or_loops() {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let path = write_wav("loop", spec, 0.1);
        let mut once = WavSource::open(&path, false).unwrap();
        let mut looping = WavSource::open(&path, true).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(read_all(&mut once).len(), 5);
        let mut frame = [0.0; FRAME_SAMPLES];
        for _ in 0..100 {
            skip_pacing(&mut looping.clock);
            assert!(looping.read(&mut frame));
        }
    }

    #[test]
    fn wav_source_reports_missing_file() {
        let err = WavSource::open(Path::new("/nonexistent/yakvc.wav"), false).unwrap_err();
        assert!(matches!(err, AudioError::Io(_)), "{err:?}");
    }
}
