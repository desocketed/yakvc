use std::f32::consts::FRAC_PI_4;

use crate::echo::{DelayLine, seconds_to_samples};
use crate::{ECHO_TAPS, Echo, EchoTap, Echoes, FRAME_SAMPLES, MonoFrame, Reverb, StereoFrame};

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
    /// How muffled the source sounds, from 0 (clear) to 1 (behind thick
    /// walls). The mixer ignores it: each source's own [`Muffle`] applies it
    /// before mixing, because filtering needs state kept per source.
    ///
    /// [`Muffle`]: crate::Muffle
    pub muffle: f32,
    /// How much of the source goes to the room's reverb, from 0 (none) to 1.
    pub reverb: f32,
    /// How much of the source the far surfaces echo back, from 0 (none) to
    /// 1: a voice in the world, not one played centred.
    pub echo: f32,
}

/// How the listener's surroundings sound: the room the game estimated, and
/// the player's tuning of it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Acoustics {
    /// Seconds until the reverb fades by 60 dB.
    pub decay: f32,
    /// How loud the distinct echo off the far walls is, from 0.
    pub echo: f32,
    /// Seconds until the echo comes back. The reverb starts after half of
    /// it, so a big space has a moment of quiet before it rings.
    pub echo_delay: f32,
    /// Each echo repeat keeps this much of the last, from 0 to 0.9.
    pub echo_feedback: f32,
    /// Scales the reverb's loudness; 1 is as built.
    pub reverb_level: f32,
    /// How much faster the reverb's highs fade than its lows, from 0 to 0.95.
    pub reverb_damping: f32,
    /// The distinct echoes off far surfaces, each from its own direction.
    pub taps: [EchoTap; ECHO_TAPS],
}

impl Default for Acoustics {
    fn default() -> Self {
        Acoustics {
            decay: 1.0,
            echo: 0.0,
            echo_delay: 0.0,
            echo_feedback: 0.35,
            reverb_level: 1.0,
            reverb_damping: 0.3,
            taps: [EchoTap::default(); ECHO_TAPS],
        }
    }
}

/// Mixes mono sources into one stereo frame, with the room's reverb and
/// echo, master gain and a soft limiter.
#[derive(Debug)]
pub struct Mixer {
    master_gain: f32,
    acoustics: Acoustics,
    reverb: Reverb,
    /// Holds the reverb sends back for the pre-delay.
    predelay: DelayLine,
    echo: Echo,
    echoes: Echoes,
}

impl Mixer {
    pub fn new() -> Self {
        Mixer {
            master_gain: 1.0,
            acoustics: Acoustics::default(),
            reverb: Reverb::new(),
            predelay: DelayLine::new(),
            echo: Echo::new(),
            echoes: Echoes::new(),
        }
    }

    pub fn set_master_gain(&mut self, gain: f32) {
        self.master_gain = gain;
    }

    pub fn set_acoustics(&mut self, acoustics: Acoustics) {
        self.reverb.set_decay(acoustics.decay);
        self.reverb.set_damping(acoustics.reverb_damping);
        self.acoustics = acoustics;
    }

    /// Overwrites `out` with the mix of `sources`.
    pub fn mix<'a>(
        &mut self,
        sources: impl IntoIterator<Item = (&'a MonoFrame, Spatial)>,
        out: &mut StereoFrame,
    ) {
        *out = [[0.0; 2]; FRAME_SAMPLES];
        let mut sends: MonoFrame = [0.0; FRAME_SAMPLES];
        let mut far: MonoFrame = [0.0; FRAME_SAMPLES];
        for (frame, spatial) in sources {
            let [left, right] = pan_gains(spatial);
            let send = spatial.gain * finite_or_zero(spatial.reverb).clamp(0.0, 1.0);
            let echo = spatial.gain * finite_or_zero(spatial.echo).clamp(0.0, 1.0);
            let into = out.iter_mut().zip(&mut sends).zip(&mut far);
            for (sample, ((mixed, sent), echoed)) in frame.iter().zip(into) {
                mixed[0] += sample * left;
                mixed[1] += sample * right;
                *sent += sample * send;
                *echoed += sample * echo;
            }
        }
        // Every frame, even with nobody talking, so tails ring out.
        let a = self.acoustics;
        let predelay = seconds_to_samples(a.echo_delay / 2.0);
        let mut delayed: MonoFrame = [0.0; FRAME_SAMPLES];
        for (sent, late) in sends.iter().zip(&mut delayed) {
            self.predelay.write(*sent);
            *late = if predelay == 0 {
                *sent
            } else {
                self.predelay.read(predelay + 1)
            };
        }
        let mut wet: StereoFrame = [[0.0; 2]; FRAME_SAMPLES];
        self.reverb.process(&delayed, &mut wet);
        let level = finite_or_zero(a.reverb_level).max(0.0);
        for (mixed, wet) in out.iter_mut().zip(&wet) {
            mixed[0] += wet[0] * level;
            mixed[1] += wet[1] * level;
        }
        let feedback = finite_or_zero(a.echo_feedback).clamp(0.0, 0.9);
        let echo = finite_or_zero(a.echo).max(0.0);
        self.echo.process(&sends, echo, a.echo_delay, feedback, out);
        self.echoes.process(&far, &a.taps, out);
        for sample in out.iter_mut().flatten() {
            *sample = soft_limit(*sample * self.master_gain);
        }
        // Whatever went wrong upstream, never hand the sound server NaN or
        // infinity: it would pass them on to every filter and device after
        // us. A broken reverb starts over rather than ringing NaN forever.
        if !out.iter().flatten().all(|s| s.is_finite()) {
            *out = [[0.0; 2]; FRAME_SAMPLES];
            self.reverb = Reverb::new();
            self.set_acoustics(self.acoustics);
            self.predelay = DelayLine::new();
            self.echo = Echo::new();
            self.echoes = Echoes::new();
        }
    }
}

impl Default for Mixer {
    fn default() -> Self {
        Self::new()
    }
}

fn finite_or_zero(value: f32) -> f32 {
    if value.is_finite() { value } else { 0.0 }
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
                    muffle: 0.0,
                    reverb: 0.0,
                    echo: 0.0,
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
                    muffle: 0.0,
                    reverb: 0.0,
                    echo: 0.0,
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
                    muffle: 0.0,
                    reverb: 0.0,
                    echo: 0.0,
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
                    muffle: 0.0,
                    reverb: 0.0,
                    echo: 0.0,
                },
            ),
            (
                constant(0.2),
                Spatial {
                    gain: 1.0,
                    pan: -1.0,
                    muffle: 0.0,
                    reverb: 0.0,
                    echo: 0.0,
                },
            ),
        ];
        let out = mix(&sources, 2.0);
        // (0.2 * 0.5 + 0.2 * 1.0) * 2.0
        assert!((out[0][0] - 0.6).abs() < 1e-6);
        assert!(out[0][1].abs() < 1e-6);
    }

    #[test]
    fn non_finite_input_plays_silence() {
        let mut mixer = Mixer::new();
        let mut out = [[0.0; 2]; FRAME_SAMPLES];
        let spatial = Spatial {
            gain: 1.0,
            pan: 0.0,
            muffle: 0.0,
            reverb: 1.0,
            echo: 0.0,
        };
        mixer.mix([(&constant(f32::NAN), spatial)], &mut out);
        assert!(out.iter().flatten().all(|&s| s == 0.0));
        // The reverb started over, so a clean frame plays cleanly.
        mixer.mix([(&constant(0.1), spatial)], &mut out);
        assert!(out.iter().flatten().all(|s| s.is_finite()));
        assert!(out.iter().flatten().any(|&s| s != 0.0));
    }

    #[test]
    fn only_voices_in_the_world_echo_off_far_cliffs() {
        let echoed = |echo| {
            let mut mixer = Mixer::new();
            let mut taps = [EchoTap::default(); ECHO_TAPS];
            taps[0] = EchoTap {
                delay: 0.1,
                gain: 0.5,
                pan: 1.0,
                cutoff_hz: 20_000.0,
            };
            mixer.set_acoustics(Acoustics {
                taps,
                ..Acoustics::default()
            });
            let spatial = Spatial {
                gain: 1.0,
                pan: -1.0,
                muffle: 0.0,
                reverb: 0.0,
                echo,
            };
            let mut out = [[0.0; 2]; FRAME_SAMPLES];
            mixer.mix([(&constant(0.2), spatial)], &mut out);
            // The voice is fully left: anything on the right is the echo.
            let silence = constant(0.0);
            (0..6)
                .map(|_| {
                    mixer.mix([(&silence, spatial)], &mut out);
                    out.iter().map(|[_, r]| r.abs()).fold(0.0, f32::max)
                })
                .fold(0.0, f32::max)
        };
        assert!(echoed(1.0) > 0.05);
        assert_eq!(echoed(0.0), 0.0);
    }

    #[test]
    fn limiter_keeps_output_in_range() {
        let sources = [(
            constant(1.0),
            Spatial {
                gain: 1.0,
                pan: -1.0,
                muffle: 0.0,
                reverb: 0.0,
                echo: 0.0,
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
                    muffle: 0.0,
                    reverb: 0.0,
                    echo: 0.0,
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
