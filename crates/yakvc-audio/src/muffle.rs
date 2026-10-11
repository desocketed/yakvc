use crate::{MonoFrame, SAMPLE_RATE};

/// Cutoff when fully muffled: a voice through thick walls keeps its low end.
const MUFFLED_CUTOFF_HZ: f32 = 600.0;

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
    /// (0 is clear, 1 fully muffled).
    pub fn apply(&mut self, frame: &mut MonoFrame, amount: f32) {
        let amount = amount.clamp(0.0, 1.0);
        let from = self.amount;
        self.amount = amount;
        if from == 0.0 && amount == 0.0 {
            // Clear: pass through, and start the next muffle from rest.
            self.stages = [0.0; 2];
            return;
        }
        let step = (amount - from) / frame.len() as f32;
        for (i, sample) in frame.iter_mut().enumerate() {
            let coefficient = coefficient(from + step * (i + 1) as f32);
            let mut value = *sample;
            for stage in &mut self.stages {
                *stage += coefficient * (value - *stage);
                value = *stage;
            }
            *sample = value;
        }
    }
}

/// The one-pole coefficient for a muffle amount. The cutoff moves
/// geometrically, so equal steps in amount sound like equal steps.
fn coefficient(amount: f32) -> f32 {
    let cutoff = CLEAR_CUTOFF_HZ * (MUFFLED_CUTOFF_HZ / CLEAR_CUTOFF_HZ).powf(amount);
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
            muffle.apply(&mut frame, amount);
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
        Muffle::new().apply(&mut frame, 0.0);
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
