use std::f32::consts::FRAC_PI_4;

use crate::{MonoFrame, StereoFrame};

/// Samples below this magnitude pass the limiter untouched.
const LIMITER_THRESHOLD: f32 = 0.9;

/// Where a source sits in the stereo image. Computed by the caller from game
/// positions.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spatial {
    /// Linear gain, including distance and per-player volume.
    pub gain: f32,
    /// `-1.0` fully left to `1.0` fully right; equal-power panning.
    pub pan: f32,
}

/// Mixes mono sources into one stereo frame, with master gain and a soft
/// limiter.
#[derive(Debug)]
pub struct Mixer {
    master_gain: f32,
}

impl Mixer {
    pub fn new() -> Self {
        Mixer { master_gain: 1.0 }
    }

    pub fn set_master_gain(&mut self, gain: f32) {
        self.master_gain = gain;
    }

    /// Overwrites `out` with the mix of `sources`.
    pub fn mix<'a>(
        &mut self,
        sources: impl IntoIterator<Item = (&'a MonoFrame, Spatial)>,
        out: &mut StereoFrame,
    ) {
        *out = [[0.0; 2]; crate::FRAME_SAMPLES];
        for (frame, spatial) in sources {
            let [left, right] = pan_gains(spatial);
            for (sample, mixed) in frame.iter().zip(out.iter_mut()) {
                mixed[0] += sample * left;
                mixed[1] += sample * right;
            }
        }
        for sample in out.iter_mut().flatten() {
            *sample = soft_limit(*sample * self.master_gain);
        }
    }
}

impl Default for Mixer {
    fn default() -> Self {
        Self::new()
    }
}

/// Left and right gains for a source. Equal-power: the total power is the
/// same wherever the source is panned.
fn pan_gains(spatial: Spatial) -> [f32; 2] {
    let angle = (spatial.pan.clamp(-1.0, 1.0) + 1.0) * FRAC_PI_4;
    [spatial.gain * angle.cos(), spatial.gain * angle.sin()]
}

/// Leaves quiet samples alone and bends loud ones smoothly towards ±1.0, so
/// many loud speakers at once saturate gently instead of clipping.
fn soft_limit(sample: f32) -> f32 {
    let magnitude = sample.abs();
    if magnitude <= LIMITER_THRESHOLD {
        return sample;
    }
    let headroom = 1.0 - LIMITER_THRESHOLD;
    let limited =
        LIMITER_THRESHOLD + headroom * ((magnitude - LIMITER_THRESHOLD) / headroom).tanh();
    limited.copysign(sample)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FRAME_SAMPLES;

    fn constant(value: f32) -> MonoFrame {
        [value; FRAME_SAMPLES]
    }

    fn mix(sources: &[(MonoFrame, Spatial)], master: f32) -> StereoFrame {
        let mut mixer = Mixer::new();
        mixer.set_master_gain(master);
        let mut out = [[1.0; 2]; FRAME_SAMPLES];
        mixer.mix(sources.iter().map(|(f, s)| (f, *s)), &mut out);
        out
    }

    #[test]
    fn no_sources_is_silence() {
        assert!(mix(&[], 1.0).iter().flatten().all(|&s| s == 0.0));
    }

    #[test]
    fn centre_pan_is_equal_power() {
        let out = mix(
            &[(
                constant(0.5),
                Spatial {
                    gain: 1.0,
                    pan: 0.0,
                },
            )],
            1.0,
        );
        let expected = 0.5 * std::f32::consts::FRAC_1_SQRT_2;
        assert!((out[0][0] - expected).abs() < 1e-6);
        assert!((out[0][1] - expected).abs() < 1e-6);
    }

    #[test]
    fn hard_pan_uses_one_channel() {
        let left = mix(
            &[(
                constant(0.5),
                Spatial {
                    gain: 1.0,
                    pan: -1.0,
                },
            )],
            1.0,
        );
        assert!((left[10][0] - 0.5).abs() < 1e-6);
        assert!(left[10][1].abs() < 1e-6);
        let right = mix(
            &[(
                constant(0.5),
                Spatial {
                    gain: 1.0,
                    pan: 1.0,
                },
            )],
            1.0,
        );
        assert!(right[10][0].abs() < 1e-6);
        assert!((right[10][1] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn gains_scale_and_sources_add() {
        let sources = [
            (
                constant(0.2),
                Spatial {
                    gain: 0.5,
                    pan: -1.0,
                },
            ),
            (
                constant(0.2),
                Spatial {
                    gain: 1.0,
                    pan: -1.0,
                },
            ),
        ];
        let out = mix(&sources, 2.0);
        // (0.2 * 0.5 + 0.2 * 1.0) * 2.0
        assert!((out[0][0] - 0.6).abs() < 1e-6);
        assert!(out[0][1].abs() < 1e-6);
    }

    #[test]
    fn limiter_keeps_output_in_range() {
        let sources = [(
            constant(1.0),
            Spatial {
                gain: 1.0,
                pan: -1.0,
            },
        ); 8];
        let out = mix(&sources, 1.0);
        assert!(out.iter().flatten().all(|s| s.abs() <= 1.0));
        assert!(out[0][0] > 0.95, "loud input stays loud: {}", out[0][0]);
        let negative = mix(
            &[(
                constant(-3.0),
                Spatial {
                    gain: 1.0,
                    pan: 1.0,
                },
            )],
            1.0,
        );
        assert!(negative[0][1] < -0.95 && negative[0][1] >= -1.0);
    }

    #[test]
    fn limiter_is_continuous_at_threshold() {
        let below = soft_limit(LIMITER_THRESHOLD - 1e-4);
        let above = soft_limit(LIMITER_THRESHOLD + 1e-4);
        assert!((above - below).abs() < 1e-3);
    }
}
