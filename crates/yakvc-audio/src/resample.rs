//! Sample-rate conversion at the edges of the pipeline: device and WAV rates
//! to and from [`SAMPLE_RATE`](crate::SAMPLE_RATE).

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Resampler as _};

use crate::AudioError;

/// Roughly 10 ms at common rates: small enough for low latency, large enough
/// for the FFT to be cheap.
const CHUNK_FRAMES: usize = 480;

/// Converts interleaved audio between two rates, accepting input in pieces
/// of any size.
pub(crate) struct Resampler {
    /// `None` when the rates match and samples pass straight through.
    fft: Option<Fft<f32>>,
    channels: usize,
    /// Input waiting for a whole chunk.
    pending: Vec<f32>,
    scratch: Vec<f32>,
}

impl Resampler {
    pub(crate) fn new(from_rate: u32, to_rate: u32, channels: usize) -> Result<Self, AudioError> {
        let fft = if from_rate == to_rate {
            None
        } else {
            let fft = Fft::new(
                from_rate as usize,
                to_rate as usize,
                CHUNK_FRAMES,
                channels,
                FixedSync::Input,
            )
            .map_err(|err| AudioError::Device(format!("cannot resample: {err}")))?;
            Some(fft)
        };
        let scratch_len = fft.as_ref().map_or(0, |f| f.output_frames_max() * channels);
        Ok(Resampler {
            fft,
            channels,
            pending: Vec::new(),
            scratch: vec![0.0; scratch_len],
        })
    }

    /// Converts `input` and appends the result to `output`. Output lags input
    /// by up to one chunk.
    pub(crate) fn process(&mut self, input: &[f32], output: &mut Vec<f32>) {
        let Some(fft) = &mut self.fft else {
            output.extend_from_slice(input);
            return;
        };
        self.pending.extend_from_slice(input);
        loop {
            let frames_in = fft.input_frames_next();
            let samples_in = frames_in * self.channels;
            if self.pending.len() < samples_in {
                return;
            }
            let frames_out = self.scratch.len() / self.channels;
            let chunk =
                InterleavedSlice::new(&self.pending[..samples_in], self.channels, frames_in)
                    .expect("chunk holds exactly frames_in frames");
            let mut out = InterleavedSlice::new_mut(&mut self.scratch, self.channels, frames_out)
                .expect("scratch holds exactly frames_out frames");
            // Errors only report wrongly sized buffers, which the sizes above
            // rule out.
            if let Ok((_, produced)) = fft.process_into_buffer(&chunk, &mut out, None) {
                output.extend_from_slice(&self.scratch[..produced * self.channels]);
            }
            self.pending.drain(..samples_in);
        }
    }
}

impl std::fmt::Debug for Resampler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Resampler")
            .field("resampling", &self.fft.is_some())
            .field("channels", &self.channels)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(rate: u32, frequency: f32, seconds: f32) -> Vec<f32> {
        let len = (rate as f32 * seconds) as usize;
        (0..len)
            .map(|i| (2.0 * std::f32::consts::PI * frequency * i as f32 / rate as f32).sin() * 0.5)
            .collect()
    }

    fn zero_crossings(samples: &[f32]) -> usize {
        samples
            .windows(2)
            .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
            .count()
    }

    #[test]
    fn matching_rates_pass_through() {
        let mut resampler = Resampler::new(48_000, 48_000, 1).unwrap();
        let mut out = Vec::new();
        resampler.process(&[0.1, 0.2, 0.3], &mut out);
        assert_eq!(out, [0.1, 0.2, 0.3]);
    }

    #[test]
    fn upsamples_44100_keeping_pitch() {
        let input = sine(44_100, 1000.0, 1.0);
        let mut resampler = Resampler::new(44_100, 48_000, 1).unwrap();
        let mut out = Vec::new();
        // Feed in odd-sized pieces, like a device callback would.
        for piece in input.chunks(333) {
            resampler.process(piece, &mut out);
        }
        // All but the last partial chunk comes out.
        assert!(out.len() > 47_000 && out.len() <= 48_000, "{}", out.len());
        // Skip the start-up transient, then a 1 kHz tone crosses zero 2000
        // times a second.
        let steady = &out[4800..4800 + 38_400];
        let crossings = zero_crossings(steady) as f32 / 0.8;
        assert!((crossings - 2000.0).abs() < 10.0, "{crossings}");
        let peak = steady.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!((peak - 0.5).abs() < 0.02, "{peak}");
    }

    #[test]
    fn downsamples_stereo() {
        let mut resampler = Resampler::new(48_000, 44_100, 2).unwrap();
        let mut out = Vec::new();
        for _ in 0..50 {
            resampler.process(&[0.25; 960 * 2], &mut out);
        }
        assert_eq!(out.len() % 2, 0);
        let frames = out.len() / 2;
        assert!(frames > 43_000 && frames <= 44_100, "{frames}");
        // A constant signal stays constant once the filter has settled.
        assert!(out[20_000..].iter().all(|s| (s - 0.25).abs() < 0.01));
    }
}
