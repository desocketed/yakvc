//! What a remote voice is doing from moment to moment, for animating the
//! speaker: how loud it is and the pitch it is spoken at.

use crate::input::SILENCE_DB;
use crate::{FRAME_SAMPLES, MonoFrame, SAMPLE_RATE, level_db};

/// Frames per analysis: 40 ms, two periods of the lowest pitch, at 25 Hz.
const FRAMES_PER_WINDOW: usize = 2;
/// Pitch is estimated at a quarter of [`SAMPLE_RATE`], which is plenty for
/// a voice's fundamental and a sixteenth of the work.
const DECIMATION: usize = 4;
const RATE: f32 = (SAMPLE_RATE as usize / DECIMATION) as f32;
const WINDOW: usize = FRAMES_PER_WINDOW * FRAME_SAMPLES / DECIMATION;
/// Lowest and highest pitch looked for: a deep voice to a child's.
const MIN_PITCH_HZ: f32 = 70.0;
const MAX_PITCH_HZ: f32 = 400.0;
const MIN_LAG: usize = (RATE / MAX_PITCH_HZ) as usize;
const MAX_LAG: usize = (RATE / MIN_PITCH_HZ) as usize + 1;
/// YIN compares the window's first `WINDOW - MAX_LAG` samples with
/// themselves shifted by each lag.
const SPAN: usize = WINDOW - MAX_LAG;
/// YIN's dip threshold: a lag whose normalised difference is under this is
/// periodic enough to be the pitch.
const VOICED_THRESHOLD: f32 = 0.2;
/// Quieter than this is not worth a pitch estimate.
const PITCH_LEVEL_DB: f32 = -55.0;
/// One-pole low-pass before decimating, at about 1.5 kHz.
const ANTI_ALIAS: f32 = 0.18;

/// One analysis of a voice.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpeechFeatures {
    /// RMS level in dBFS.
    pub level_db: f32,
    /// The fundamental frequency, if the voice is voiced.
    pub pitch_hz: Option<f32>,
}

/// Turns one voice's frames into [`SpeechFeatures`], once every
/// [`FRAMES_PER_WINDOW`] frames. Allocation-free.
#[derive(Debug)]
pub struct SpeechAnalyser {
    window: [f32; WINDOW],
    filled: usize,
    lows: [f32; 2],
    level_sum: f32,
}

impl Default for SpeechAnalyser {
    fn default() -> Self {
        SpeechAnalyser {
            window: [0.0; WINDOW],
            filled: 0,
            lows: [0.0; 2],
            level_sum: 0.0,
        }
    }
}

impl SpeechAnalyser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Takes the next frame as decoded, and returns an analysis of the last
    /// window when this frame completes one.
    pub fn push(&mut self, frame: &MonoFrame) -> Option<SpeechFeatures> {
        // Averaging the frames' power, not their dB.
        self.level_sum += 10f32.powf(level_db(frame) / 10.0);
        for chunk in frame.as_chunks::<DECIMATION>().0 {
            for &sample in chunk {
                let mut value = sample;
                for low in &mut self.lows {
                    *low += ANTI_ALIAS * (value - *low);
                    value = *low;
                }
            }
            self.window[self.filled] = self.lows[1];
            self.filled += 1;
        }
        if !self.lows.iter().all(|l| l.is_finite()) {
            self.lows = [0.0; 2];
        }
        if self.filled < WINDOW {
            return None;
        }
        self.filled = 0;
        let mean = self.level_sum / FRAMES_PER_WINDOW as f32;
        self.level_sum = 0.0;
        let level_db = if mean > 0.0 {
            (10.0 * mean.log10()).max(SILENCE_DB)
        } else {
            SILENCE_DB
        };
        let pitch_hz = if level_db > PITCH_LEVEL_DB {
            yin(&self.window)
        } else {
            None
        };
        Some(SpeechFeatures { level_db, pitch_hz })
    }
}

/// The pitch of `x` by YIN (de Cheveigné and Kawahara, 2002): the first lag
/// whose cumulative-mean-normalised difference dips under the threshold,
/// refined by a parabola through its neighbours.
fn yin(x: &[f32; WINDOW]) -> Option<f32> {
    let mut d = [0.0f32; MAX_LAG + 1];
    let mut running = 0.0;
    for lag in 1..=MAX_LAG {
        let diff: f32 = (0..SPAN).map(|i| (x[i] - x[i + lag]).powi(2)).sum();
        running += diff;
        d[lag] = if running > 0.0 {
            diff * lag as f32 / running
        } else {
            1.0
        };
    }
    let mut lag = MIN_LAG;
    while lag < MAX_LAG {
        if d[lag] < VOICED_THRESHOLD {
            while lag + 1 < MAX_LAG && d[lag + 1] < d[lag] {
                lag += 1;
            }
            let (a, b, c) = (d[lag - 1], d[lag], d[lag + 1]);
            let bend = a - 2.0 * b + c;
            let shift = if bend > 0.0 {
                (0.5 * (a - c) / bend).clamp(-1.0, 1.0)
            } else {
                0.0
            };
            return Some(RATE / (lag as f32 + shift));
        }
        lag += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use std::f32::consts::TAU;

    use super::*;

    /// Feeds `seconds` of `signal(t)` and returns every analysis.
    fn analyse(seconds: f32, mut signal: impl FnMut(f32) -> f32) -> Vec<SpeechFeatures> {
        let mut analyser = SpeechAnalyser::new();
        let mut out = Vec::new();
        let frames = (seconds * SAMPLE_RATE as f32) as usize / FRAME_SAMPLES;
        for f in 0..frames {
            let mut frame: MonoFrame = [0.0; FRAME_SAMPLES];
            for (i, s) in frame.iter_mut().enumerate() {
                *s = signal((f * FRAME_SAMPLES + i) as f32 / SAMPLE_RATE as f32);
            }
            out.extend(analyser.push(&frame));
        }
        out
    }

    /// A buzz with harmonics, closer to a voice than a sine.
    fn voice(f0: f32, amplitude: f32) -> impl Fn(f32) -> f32 {
        move |t| {
            (1..=5)
                .map(|k| (TAU * f0 * k as f32 * t).sin() / k as f32)
                .sum::<f32>()
                * amplitude
        }
    }

    #[test]
    fn reports_at_25_hz() {
        assert_eq!(analyse(1.0, voice(150.0, 0.2)).len(), 25);
    }

    #[test]
    fn finds_the_pitch_of_voices() {
        for f0 in [85.0, 120.0, 180.0, 250.0, 350.0] {
            let features = analyse(0.4, voice(f0, 0.2));
            for f in &features[1..] {
                let pitch = f.pitch_hz.unwrap_or_else(|| panic!("{f0} Hz unvoiced"));
                assert!((pitch - f0).abs() < f0 * 0.03, "{f0} Hz heard as {pitch}");
            }
        }
    }

    #[test]
    fn noise_and_silence_have_no_pitch() {
        let mut seed = 1u32;
        let noise = analyse(0.4, move |_| {
            // A fixed xorshift, so the test is repeatable.
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            (seed as f32 / u32::MAX as f32 - 0.5) * 0.4
        });
        assert!(noise.iter().all(|f| f.pitch_hz.is_none()), "{noise:?}");
        let silence = analyse(0.4, |_| 0.0);
        assert!(
            silence
                .iter()
                .all(|f| f.pitch_hz.is_none() && f.level_db < -90.0)
        );
    }

    #[test]
    fn level_follows_loudness() {
        let loud = analyse(0.2, voice(150.0, 0.4))[1].level_db;
        let quiet = analyse(0.2, voice(150.0, 0.04))[1].level_db;
        assert!((loud - quiet - 20.0).abs() < 0.5, "{loud} vs {quiet}");
    }
}
