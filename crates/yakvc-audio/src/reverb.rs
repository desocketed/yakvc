use crate::{FRAME_SAMPLES, MonoFrame, SAMPLE_RATE, StereoFrame};

/// Comb delays from Freeverb, tuned for 44.1 kHz and scaled to ours. Four
/// of its eight are enough for voice.
const COMB_DELAYS: [usize; 4] = [1116, 1188, 1277, 1356];
const ALLPASS_DELAYS: [usize; 2] = [556, 441];
/// The right channel's delays are this much longer, so the tail is wide.
const STEREO_SPREAD: usize = 23;
/// How much each comb's feedback loses of its highs: walls absorb treble.
const DAMPING: f32 = 0.3;
const ALLPASS_FEEDBACK: f32 = 0.5;
/// The combs add up, so their sum is scaled back down.
const OUTPUT_GAIN: f32 = 0.25;
/// The shortest and longest tail, in seconds until it fades by 60 dB.
const MIN_DECAY: f32 = 0.1;
const MAX_DECAY: f32 = 10.0;

/// A small Schroeder–Moorer reverb (Freeverb's structure, trimmed): parallel
/// damped combs into series all-passes, one bank per channel. It turns the
/// sources' summed reverb sends into a stereo tail that sounds like the
/// listener's surroundings.
#[derive(Debug)]
pub struct Reverb {
    channels: [Bank; 2],
}

#[derive(Debug)]
struct Bank {
    combs: Vec<Comb>,
    allpasses: Vec<Delay>,
}

#[derive(Debug)]
struct Comb {
    delay: Delay,
    feedback: f32,
    /// The last output, low-passed: the damping filter's state.
    filtered: f32,
}

#[derive(Debug)]
struct Delay {
    buffer: Vec<f32>,
    pos: usize,
}

impl Reverb {
    pub fn new() -> Self {
        let bank = |spread: usize| Bank {
            combs: COMB_DELAYS
                .iter()
                .map(|&d| Comb {
                    delay: Delay::new(scaled(d + spread)),
                    feedback: 0.0,
                    filtered: 0.0,
                })
                .collect(),
            allpasses: ALLPASS_DELAYS
                .iter()
                .map(|&d| Delay::new(scaled(d + spread)))
                .collect(),
        };
        let mut reverb = Reverb {
            channels: [bank(0), bank(STEREO_SPREAD)],
        };
        reverb.set_decay(1.0);
        reverb
    }

    /// How long the tail lasts, in seconds until it has faded by 60 dB.
    pub fn set_decay(&mut self, seconds: f32) {
        let seconds = seconds.clamp(MIN_DECAY, MAX_DECAY);
        for bank in &mut self.channels {
            for comb in &mut bank.combs {
                let delay = comb.delay.buffer.len() as f32 / SAMPLE_RATE as f32;
                comb.feedback = 10f32.powf(-3.0 * delay / seconds);
            }
        }
    }

    /// Adds the reverberated `input` to `out`.
    pub fn process(&mut self, input: &MonoFrame, out: &mut StereoFrame) {
        for (channel, bank) in self.channels.iter_mut().enumerate() {
            for i in 0..FRAME_SAMPLES {
                out[i][channel] += bank.process(input[i]) * OUTPUT_GAIN;
            }
        }
    }
}

impl Default for Reverb {
    fn default() -> Self {
        Self::new()
    }
}

impl Bank {
    fn process(&mut self, input: f32) -> f32 {
        let mut sum = 0.0;
        for comb in &mut self.combs {
            let delayed = comb.delay.read();
            comb.filtered = delayed * (1.0 - DAMPING) + comb.filtered * DAMPING;
            comb.delay.write(input + comb.filtered * comb.feedback);
            sum += delayed;
        }
        for allpass in &mut self.allpasses {
            let delayed = allpass.read();
            allpass.write(sum + delayed * ALLPASS_FEEDBACK);
            sum = delayed - sum;
        }
        sum
    }
}

impl Delay {
    fn new(len: usize) -> Self {
        Delay {
            buffer: vec![0.0; len],
            pos: 0,
        }
    }

    /// The sample written `len` samples ago.
    fn read(&self) -> f32 {
        self.buffer[self.pos]
    }

    fn write(&mut self, sample: f32) {
        self.buffer[self.pos] = sample;
        self.pos = (self.pos + 1) % self.buffer.len();
    }
}

/// A delay tuned for 44.1 kHz, in samples at our rate.
fn scaled(samples_at_44k: usize) -> usize {
    samples_at_44k * SAMPLE_RATE as usize / 44_100
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tail's energy in the 20 ms frame `frames` after one click.
    fn tail_energy(decay: f32, frames: usize) -> f32 {
        let mut reverb = Reverb::new();
        reverb.set_decay(decay);
        let mut click = [0.0; FRAME_SAMPLES];
        click[0] = 1.0;
        let silence = [0.0; FRAME_SAMPLES];
        let mut energy = 0.0;
        for n in 0..=frames {
            let mut out = [[0.0; 2]; FRAME_SAMPLES];
            reverb.process(if n == 0 { &click } else { &silence }, &mut out);
            energy = out.iter().flatten().map(|s| s * s).sum();
        }
        energy
    }

    #[test]
    fn silence_stays_silent() {
        let mut reverb = Reverb::new();
        let mut out = [[0.0; 2]; FRAME_SAMPLES];
        reverb.process(&[0.0; FRAME_SAMPLES], &mut out);
        assert!(out.iter().flatten().all(|&s| s == 0.0));
    }

    #[test]
    fn a_click_rings_on_and_fades() {
        let early = tail_energy(1.0, 5);
        let late = tail_energy(1.0, 60);
        assert!(early > 0.0);
        assert!(late < early / 1000.0, "early {early}, late {late}");
    }

    #[test]
    fn longer_decay_rings_longer() {
        assert!(tail_energy(3.0, 25) > tail_energy(0.5, 25) * 10.0);
    }

    #[test]
    fn channels_differ() {
        let mut reverb = Reverb::new();
        let mut click = [0.0; FRAME_SAMPLES];
        click[0] = 1.0;
        let mut out = [[0.0; 2]; FRAME_SAMPLES];
        reverb.process(&click, &mut out);
        reverb.process(&[0.0; FRAME_SAMPLES], &mut out);
        assert!(out.iter().any(|[l, r]| (l - r).abs() > 1e-6));
    }
}
