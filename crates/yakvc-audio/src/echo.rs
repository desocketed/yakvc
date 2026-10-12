use std::f32::consts::{FRAC_PI_4, TAU};

use crate::{FRAME_SAMPLES, MonoFrame, SAMPLE_RATE, StereoFrame};

/// The longest delay a line holds: the far wall of a very large space.
const MAX_DELAY_SECONDS: f32 = 0.5;
/// Each repeat loses this much of its highs, as a real echo off stone does.
const DAMPING: f32 = 0.4;
/// The right channel's echo comes this much later, so it sounds wide.
const STEREO_OFFSET_SECONDS: f32 = 0.011;

/// A delay line: what was written some samples ago can be read back.
#[derive(Debug)]
pub(crate) struct DelayLine {
    buffer: Vec<f32>,
    pos: usize,
}

impl DelayLine {
    pub(crate) fn new() -> Self {
        Self::holding(MAX_DELAY_SECONDS)
    }

    fn holding(seconds: f32) -> Self {
        DelayLine {
            buffer: vec![0.0; (seconds * SAMPLE_RATE as f32) as usize + 1],
            pos: 0,
        }
    }

    /// The sample written `delay` writes ago, at most the line's length.
    pub(crate) fn read(&self, delay: usize) -> f32 {
        let len = self.buffer.len();
        self.buffer[(self.pos + len - delay.clamp(1, len)) % len]
    }

    pub(crate) fn write(&mut self, sample: f32) {
        self.buffer[self.pos] = sample;
        self.pos = (self.pos + 1) % self.buffer.len();
    }
}

/// The distinct echo of a big space: the voice comes back off the far walls
/// after a delay, then again, quieter and duller each time.
#[derive(Debug)]
pub struct Echo {
    line: DelayLine,
    /// The last repeat, low-passed: the damping filter's state.
    filtered: f32,
}

impl Echo {
    pub fn new() -> Self {
        Echo {
            line: DelayLine::new(),
            filtered: 0.0,
        }
    }

    /// Adds the echo of `input` to `out`: `level` of it comes back after
    /// `delay` seconds, and each repeat keeps `feedback` of the last.
    pub fn process(
        &mut self,
        input: &MonoFrame,
        level: f32,
        delay: f32,
        feedback: f32,
        out: &mut StereoFrame,
    ) {
        let delay = seconds_to_samples(delay);
        let offset = seconds_to_samples(STEREO_OFFSET_SECONDS);
        for i in 0..FRAME_SAMPLES {
            let left = self.line.read(delay);
            let right = self.line.read(delay + offset);
            self.filtered = left * (1.0 - DAMPING) + self.filtered * DAMPING;
            self.line.write(input[i] * level + self.filtered * feedback);
            out[i][0] += left;
            out[i][1] += right;
        }
    }
}

impl Default for Echo {
    fn default() -> Self {
        Self::new()
    }
}

/// How many far surfaces can echo at once.
pub const ECHO_TAPS: usize = 4;
/// The longest an echo takes to come back: a cliff about 250 m away.
const MAX_TAP_SECONDS: f32 = 1.5;

/// One distinct echo off a far surface, as the listener hears it.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct EchoTap {
    /// Seconds until it comes back.
    pub delay: f32,
    /// How loud it is, from 0.
    pub gain: f32,
    /// Where it comes from, `-1.0` fully left to `1.0` fully right.
    pub pan: f32,
    /// Its highs above this fade: the further sound travels through air, the
    /// darker it gets.
    pub cutoff_hz: f32,
}

/// The distinct echoes off far cliffs and walls: each comes back once, from
/// its own direction, after its own delay. Unlike [`Echo`], nothing repeats:
/// in the open, a sound that bounced once off a cliff is gone.
#[derive(Debug)]
pub struct Echoes {
    line: DelayLine,
    taps: [TapState; ECHO_TAPS],
}

/// What a tap played at the end of the last frame, so the next starts there.
#[derive(Debug, Clone, Copy, Default)]
struct TapState {
    delay: usize,
    gains: [f32; 2],
    /// The low-pass filter's state.
    filtered: f32,
}

impl Echoes {
    pub fn new() -> Self {
        Echoes {
            line: DelayLine::holding(MAX_TAP_SECONDS),
            taps: [TapState::default(); ECHO_TAPS],
        }
    }

    /// Adds the echoes of `input` to `out`. Each tap glides over the frame
    /// from where it was to where `taps` puts it: its gains ramp, and a new
    /// delay crossfades from the old one, so a wall coming into view or the
    /// listener turning never clicks.
    pub fn process(
        &mut self,
        input: &MonoFrame,
        taps: &[EchoTap; ECHO_TAPS],
        out: &mut StereoFrame,
    ) {
        let targets = taps.map(|tap| {
            let gain = finite_or_zero(tap.gain).max(0.0);
            let angle = (finite_or_zero(tap.pan).clamp(-1.0, 1.0) + 1.0) * FRAC_PI_4;
            let delay = finite_or_zero(tap.delay).clamp(0.0, MAX_TAP_SECONDS);
            let cutoff = finite_or_zero(tap.cutoff_hz).clamp(20.0, SAMPLE_RATE as f32 / 2.0);
            Target {
                // The line reads back the sample just written as 1 ago.
                delay: (delay * SAMPLE_RATE as f32) as usize + 1,
                gains: [gain * angle.cos(), gain * angle.sin()],
                // A one-pole low-pass: how far it moves towards each sample.
                smoothing: 1.0 - (-TAU * cutoff / SAMPLE_RATE as f32).exp(),
            }
        });
        for (tap, target) in self.taps.iter_mut().zip(&targets) {
            // A silent tap has nothing to fade out of: it starts where it is going.
            if tap.gains == [0.0; 2] {
                tap.delay = target.delay;
            }
        }
        for (i, (sample, mixed)) in input.iter().zip(out.iter_mut()).enumerate() {
            self.line.write(*sample);
            let t = (i + 1) as f32 / FRAME_SAMPLES as f32;
            for (tap, target) in self.taps.iter_mut().zip(&targets) {
                let mut heard = self.line.read(target.delay);
                if target.delay != tap.delay {
                    heard = self.line.read(tap.delay) * (1.0 - t) + heard * t;
                }
                tap.filtered += (heard - tap.filtered) * target.smoothing;
                for (channel, (from, to)) in tap.gains.iter().zip(target.gains).enumerate() {
                    mixed[channel] += tap.filtered * (from + (to - from) * t);
                }
            }
        }
        for (tap, target) in self.taps.iter_mut().zip(&targets) {
            tap.delay = target.delay;
            tap.gains = target.gains;
        }
    }
}

impl Default for Echoes {
    fn default() -> Self {
        Self::new()
    }
}

/// Where a tap is headed this frame.
struct Target {
    delay: usize,
    gains: [f32; 2],
    smoothing: f32,
}

fn finite_or_zero(value: f32) -> f32 {
    if value.is_finite() { value } else { 0.0 }
}

/// `seconds` as a delay in samples, within what a line holds.
pub(crate) fn seconds_to_samples(seconds: f32) -> usize {
    let seconds = if seconds.is_finite() { seconds } else { 0.0 };
    (seconds.clamp(0.0, MAX_DELAY_SECONDS) * SAMPLE_RATE as f32) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_click_comes_back_after_the_delay_and_fades() {
        let mut echo = Echo::new();
        let mut click = [0.0; FRAME_SAMPLES];
        click[0] = 1.0;
        let silence = [0.0; FRAME_SAMPLES];
        // 0.1 s is five frames: the first repeat lands in frame 5.
        let mut peaks = Vec::new();
        for n in 0..16 {
            let mut out = [[0.0; 2]; FRAME_SAMPLES];
            let input = if n == 0 { &click } else { &silence };
            echo.process(input, 1.0, 0.1, 0.5, &mut out);
            peaks.push(out.iter().map(|[l, _]| l.abs()).fold(0.0, f32::max));
        }
        assert!(peaks[..5].iter().all(|&p| p == 0.0), "{peaks:?}");
        assert!(peaks[5] > 0.5);
        assert!(peaks[10] > 0.0 && peaks[10] < peaks[5], "{peaks:?}");
    }

    /// A click through one tap, then silence: each frame's peak per channel.
    fn tap_peaks(tap: EchoTap, frames: usize) -> Vec<[f32; 2]> {
        let mut echoes = Echoes::new();
        let mut click = [0.0; FRAME_SAMPLES];
        click[0] = 1.0;
        let taps = [
            tap,
            EchoTap::default(),
            EchoTap::default(),
            EchoTap::default(),
        ];
        (0..frames)
            .map(|n| {
                let mut out = [[0.0; 2]; FRAME_SAMPLES];
                let input = if n == 0 { click } else { [0.0; FRAME_SAMPLES] };
                echoes.process(&input, &taps, &mut out);
                let peak = |c: usize| out.iter().map(|s| s[c].abs()).fold(0.0, f32::max);
                [peak(0), peak(1)]
            })
            .collect()
    }

    #[test]
    fn a_tap_comes_back_once_after_its_delay_from_its_side() {
        // 0.4 s is twenty frames; panned fully right, so nothing on the left.
        let tap = EchoTap {
            delay: 0.4,
            gain: 0.5,
            pan: 1.0,
            cutoff_hz: 20_000.0,
        };
        let peaks = tap_peaks(tap, 40);
        assert!(peaks[..20].iter().all(|&[l, r]| l == 0.0 && r == 0.0));
        assert!(peaks[20][1] > 0.3 && peaks[20][1] <= 0.5, "{:?}", peaks[20]);
        assert!(peaks.iter().all(|&[l, _]| l < 1e-6));
        // Once: no repeats once the filter has rung out.
        assert!(peaks[22..].iter().all(|&[_, r]| r < 1e-6), "{peaks:?}");
    }

    #[test]
    fn a_far_tap_is_darker() {
        let tap = |cutoff_hz| EchoTap {
            delay: 0.1,
            gain: 1.0,
            pan: 0.0,
            cutoff_hz,
        };
        // A click is all highs: a lower cutoff flattens its spike.
        let near = tap_peaks(tap(12_000.0), 6)[5][0];
        let far = tap_peaks(tap(1_000.0), 6)[5][0];
        assert!(far < near / 3.0, "near {near}, far {far}");
    }

    #[test]
    fn moving_a_tap_glides_instead_of_jumping() {
        // A steady tone, then the taps jump to another delay and side: no
        // sample may step further than the tone itself ever moves.
        let mut echoes = Echoes::new();
        let tone: MonoFrame =
            std::array::from_fn(|i| (TAU * 200.0 * i as f32 / SAMPLE_RATE as f32).sin() * 0.5);
        let mut tap = EchoTap {
            delay: 0.2,
            gain: 0.25,
            pan: -1.0,
            cutoff_hz: 20_000.0,
        };
        let mut previous = [0.0f32; 2];
        let mut largest_step = 0.0f32;
        for n in 0..60 {
            if n == 30 {
                tap.delay = 0.9;
                tap.pan = 1.0;
            }
            let mut out = [[0.0; 2]; FRAME_SAMPLES];
            echoes.process(&tone, &[tap; ECHO_TAPS], &mut out);
            for sample in out {
                for c in 0..2 {
                    assert!(sample[c].is_finite());
                    largest_step = largest_step.max((sample[c] - previous[c]).abs());
                }
                previous = sample;
            }
        }
        // Four taps of a 200 Hz sine at 0.5 × 0.25 move at most
        // 4 × 2π·200/48000 × 0.125 ≈ 0.013 a sample.
        assert!(largest_step < 0.02, "{largest_step}");
    }

    #[test]
    fn broken_taps_are_silent_not_nan() {
        let mut echoes = Echoes::new();
        let bad = EchoTap {
            delay: f32::NAN,
            gain: f32::INFINITY,
            pan: f32::NAN,
            cutoff_hz: f32::NEG_INFINITY,
        };
        for _ in 0..10 {
            let mut out = [[0.0; 2]; FRAME_SAMPLES];
            echoes.process(&[0.5; FRAME_SAMPLES], &[bad; ECHO_TAPS], &mut out);
            assert!(out.iter().flatten().all(|&s| s == 0.0));
        }
    }

    #[test]
    fn no_level_no_echo() {
        let mut echo = Echo::new();
        let mut out = [[0.0; 2]; FRAME_SAMPLES];
        echo.process(&[1.0; FRAME_SAMPLES], 0.0, 0.1, 0.5, &mut out);
        assert!(out.iter().flatten().all(|&s| s == 0.0));
    }
}
