use crate::{MonoFrame, SAMPLE_RATE};

/// Cutoff when barely muffled, near the top of what Opus keeps.
const CLEAR_CUTOFF_HZ: f32 = 20_000.0;

/// A low-pass filter for one source, whose cutoff follows how muffled the
/// source is: two one-pole stages, so it rolls off at 12 dB per octave.
/// The amount glides across each frame, so changes don't click.
#[derive(Debug, Default)]
pub struct Muffle {
    amount: f32,
    stages: [f32; 2],
}

impl Muffle {
    pub fn new() -> Self {
        Self::default()
    }

    /// Filters `frame` in place, gliding from the last amount to `amount`
    /// (0 is clear, 1 fully muffled, which cuts above `muffled_cutoff_hz`).
    pub fn apply(&mut self, frame: &mut MonoFrame, amount: f32, muffled_cutoff_hz: f32) {
        let muffled = if muffled_cutoff_hz.is_finite() {
            muffled_cutoff_hz.clamp(100.0, CLEAR_CUTOFF_HZ)
        } else {
            CLEAR_CUTOFF_HZ
        };
        // `clamp` keeps NaN, which would stick in the filter's state.
        let amount = if amount.is_nan() {
            0.0
        } else {
            amount.clamp(0.0, 1.0)
        };
        let from = self.amount;
        self.amount = amount;
        if from == 0.0 && amount == 0.0 {
            // Clear: pass through, and start the next muffle from rest.
            self.stages = [0.0; 2];
            return;
        }
        let step = (amount - from) / frame.len() as f32;
        for (i, sample) in frame.iter_mut().enumerate() {
            let coefficient = coefficient(from + step * (i + 1) as f32, muffled);
            let mut value = *sample;
            for stage in &mut self.stages {
                *stage += coefficient * (value - *stage);
                value = *stage;
            }
            *sample = value;
        }
        if !self.stages.iter().all(|s| s.is_finite()) {
            self.stages = [0.0; 2];
        }
    }
}

/// The one-pole coefficient for a muffle amount. The cutoff moves
/// geometrically, and fastest at first, so even a little muffle is heard:
/// with a 600 Hz muffle, half muffled cuts above about 1.7 kHz.
fn coefficient(amount: f32, muffled_cutoff_hz: f32) -> f32 {
    let cutoff = CLEAR_CUTOFF_HZ * (muffled_cutoff_hz / CLEAR_CUTOFF_HZ).powf(amount.sqrt());
    1.0 - (-std::f32::consts::TAU * cutoff / SAMPLE_RATE as f32).exp()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FRAME_SAMPLES;

    /// RMS of a tone at `hz` after a few frames through `muffle` at `amount`.
    fn rms_after(hz: f32, amount: f32) -> f32 {
        let mut muffle = Muffle::new();
        let mut sum = 0.0;
        for n in 0..10 {
            let mut frame: MonoFrame = std::array::from_fn(|i| {
                let t = (n * FRAME_SAMPLES + i) as f32 / SAMPLE_RATE as f32;
                (std::f32::consts::TAU * hz * t).sin()
            });
            muffle.apply(&mut frame, amount, 600.0);
            if n == 9 {
                sum = frame.iter().map(|s| s * s).sum::<f32>();
            }
        }
        (sum / FRAME_SAMPLES as f32).sqrt()
    }

    #[test]
    fn clear_passes_through() {
        let mut frame: MonoFrame = std::array::from_fn(|i| (i as f32 * 0.37).sin());
        let original = frame;
        Muffle::new().apply(&mut frame, 0.0, 600.0);
        assert_eq!(frame, original);
    }

    #[test]
    fn muffled_keeps_lows_and_cuts_highs() {
        let low = rms_after(150.0, 1.0);
        let high = rms_after(4_000.0, 1.0);
        assert!(low > 0.6, "low {low}");
        assert!(high < 0.05, "high {high}");
    }

    #[test]
    fn more_muffle_cuts_more() {
        assert!(rms_after(3_000.0, 0.5) > rms_after(3_000.0, 1.0));
        assert!(rms_after(3_000.0, 0.2) > rms_after(3_000.0, 0.5));
    }
}
