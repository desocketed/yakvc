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
        DelayLine {
            buffer: vec![0.0; (MAX_DELAY_SECONDS * SAMPLE_RATE as f32) as usize + 1],
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

    #[test]
    fn no_level_no_echo() {
        let mut echo = Echo::new();
        let mut out = [[0.0; 2]; FRAME_SAMPLES];
        echo.process(&[1.0; FRAME_SAMPLES], 0.0, 0.1, 0.5, &mut out);
        assert!(out.iter().flatten().all(|&s| s == 0.0));
    }
}
