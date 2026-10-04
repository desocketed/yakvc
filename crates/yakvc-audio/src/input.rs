use std::time::Duration;

use nnnoiseless::DenoiseState;

use crate::{FRAME_DURATION, MonoFrame};

/// Level reported for digital silence, in dBFS.
const SILENCE_DB: f32 = -100.0;

#[derive(Debug, Clone, PartialEq)]
pub struct InputConfig {
    /// Linear gain applied before everything else.
    pub gain: f32,
    pub noise_suppression: bool,
    /// Level above which a frame counts as voice.
    pub vad_threshold_db: f32,
    /// How long voice activity stays on after the level drops.
    pub vad_hangover: Duration,
}

impl Default for InputConfig {
    fn default() -> Self {
        InputConfig {
            gain: 1.0,
            noise_suppression: true,
            vad_threshold_db: -45.0,
            vad_hangover: Duration::from_millis(300),
        }
    }
}

/// Gain, noise suppression and voice activity detection for captured frames.
pub struct InputProcessor {
    config: InputConfig,
    /// Present while noise suppression is on.
    denoiser: Option<Box<DenoiseState<'static>>>,
    /// Frames of hangover left before voice activity turns off.
    hangover_left: u32,
}

/// Result of processing one frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InputActivity {
    /// Level after processing, in dBFS.
    pub level_db: f32,
    /// Voice detected, including hangover.
    pub voice: bool,
}

impl InputProcessor {
    pub fn new(config: InputConfig) -> Self {
        let mut processor = InputProcessor {
            config: InputConfig::default(),
            denoiser: None,
            hangover_left: 0,
        };
        processor.set_config(config);
        processor
    }

    pub fn set_config(&mut self, config: InputConfig) {
        if config.noise_suppression && self.denoiser.is_none() {
            self.denoiser = Some(DenoiseState::new());
        }
        if !config.noise_suppression {
            self.denoiser = None;
        }
        self.config = config;
    }

    /// Processes `frame` in place.
    pub fn process(&mut self, frame: &mut MonoFrame) -> InputActivity {
        for sample in frame.iter_mut() {
            *sample *= self.config.gain;
        }
        if let Some(denoiser) = &mut self.denoiser {
            denoise(denoiser, frame);
        }

        let level_db = level_db(frame);
        if level_db > self.config.vad_threshold_db {
            self.hangover_left = hangover_frames(self.config.vad_hangover);
            InputActivity {
                level_db,
                voice: true,
            }
        } else {
            let voice = self.hangover_left > 0;
            self.hangover_left = self.hangover_left.saturating_sub(1);
            InputActivity { level_db, voice }
        }
    }
}

impl std::fmt::Debug for InputProcessor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InputProcessor")
            .field("config", &self.config)
            .field("hangover_left", &self.hangover_left)
            .finish_non_exhaustive()
    }
}

/// Runs RNNoise over the frame in its 10 ms blocks.
fn denoise(denoiser: &mut DenoiseState<'static>, frame: &mut MonoFrame) {
    // RNNoise expects samples on the 16-bit integer scale.
    const SCALE: f32 = 32_768.0;
    let mut input = [0.0; DenoiseState::FRAME_SIZE];
    let mut output = [0.0; DenoiseState::FRAME_SIZE];
    for block in frame.as_chunks_mut::<{ nnnoiseless::FRAME_SIZE }>().0 {
        for (scaled, sample) in input.iter_mut().zip(block.iter()) {
            *scaled = sample * SCALE;
        }
        denoiser.process_frame(&mut output, &input);
        for (sample, denoised) in block.iter_mut().zip(output.iter()) {
            *sample = denoised / SCALE;
        }
    }
}

/// RMS level in dBFS.
pub(crate) fn level_db(frame: &[f32]) -> f32 {
    let mean_square = frame.iter().map(|s| s * s).sum::<f32>() / frame.len() as f32;
    if mean_square <= 0.0 {
        return SILENCE_DB;
    }
    (10.0 * mean_square.log10()).max(SILENCE_DB)
}

fn hangover_frames(hangover: Duration) -> u32 {
    (hangover.as_millis() / FRAME_DURATION.as_millis()) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FRAME_SAMPLES, SAMPLE_RATE};

    fn tone(amplitude: f32, start: usize) -> MonoFrame {
        std::array::from_fn(|i| {
            let t = (start + i) as f32 / SAMPLE_RATE as f32;
            amplitude * (2.0 * std::f32::consts::PI * 440.0 * t).sin()
        })
    }

    fn plain() -> InputConfig {
        InputConfig {
            noise_suppression: false,
            ..InputConfig::default()
        }
    }

    #[test]
    fn level_of_full_scale_sine_is_minus_three_db() {
        assert!((level_db(&tone(1.0, 0)) + 3.01).abs() < 0.05);
        assert_eq!(level_db(&[0.0; FRAME_SAMPLES]), SILENCE_DB);
    }

    #[test]
    fn gain_is_applied() {
        let mut processor = InputProcessor::new(InputConfig {
            gain: 0.5,
            ..plain()
        });
        let mut frame = [0.4; FRAME_SAMPLES];
        let activity = processor.process(&mut frame);
        assert!((frame[0] - 0.2).abs() < 1e-6);
        assert!((activity.level_db - 20.0 * 0.2f32.log10()).abs() < 0.01);
    }

    #[test]
    fn vad_detects_tone_not_silence() {
        let mut processor = InputProcessor::new(plain());
        assert!(!processor.process(&mut [0.0; FRAME_SAMPLES]).voice);
        assert!(processor.process(&mut tone(0.1, 0)).voice);
    }

    #[test]
    fn vad_holds_for_hangover() {
        let mut processor = InputProcessor::new(InputConfig {
            vad_hangover: Duration::from_millis(60),
            ..plain()
        });
        assert!(processor.process(&mut tone(0.1, 0)).voice);
        // 60 ms of hangover is three more frames of voice.
        for _ in 0..3 {
            assert!(processor.process(&mut [0.0; FRAME_SAMPLES]).voice);
        }
        assert!(!processor.process(&mut [0.0; FRAME_SAMPLES]).voice);
    }

    #[test]
    fn noise_suppression_quietens_noise() {
        let mut processor = InputProcessor::new(InputConfig::default());
        // Deterministic low-passed noise around -40 dBFS, like a fan or hum.
        let mut state = 12345u32;
        let mut low_passed = 0.0f32;
        let mut reduction = 0.0;
        for _ in 0..50 {
            let mut frame: MonoFrame = std::array::from_fn(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let white = (state >> 8) as f32 / (1 << 24) as f32 * 2.0 - 1.0;
                low_passed = low_passed * 0.95 + white * 0.005;
                low_passed
            });
            let before = level_db(&frame);
            reduction = before - processor.process(&mut frame).level_db;
        }
        assert!(reduction > 20.0, "noise reduced by only {reduction} dB");
    }

    #[test]
    fn noise_suppression_can_be_toggled() {
        let mut processor = InputProcessor::new(plain());
        assert!(processor.denoiser.is_none());
        processor.set_config(InputConfig::default());
        assert!(processor.denoiser.is_some());
        processor.set_config(plain());
        assert!(processor.denoiser.is_none());
    }
}
