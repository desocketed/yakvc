use crate::{AudioError, MonoFrame, SAMPLE_RATE};

const MIN_BITRATE: u32 = 16_000;
const MAX_BITRATE: u32 = 64_000;

/// The largest packet Opus produces for one frame.
const MAX_PACKET: usize = 1275;

/// In-band FEC only carries redundancy when the encoder expects some loss, so
/// tell it to plan for a modest amount.
const EXPECTED_LOSS_PERCENT: i32 = 10;

/// Opus encoder: VOIP mode, in-band FEC and DTX on.
#[derive(Debug)]
pub struct Encoder {
    opus: opus::Encoder,
    packet: Vec<u8>,
}

impl Encoder {
    /// `bitrate` in bits per second, clamped to 16–64 kbps.
    pub fn new(bitrate: u32) -> Result<Self, AudioError> {
        let mut opus =
            opus::Encoder::new(SAMPLE_RATE, opus::Channels::Mono, opus::Application::Voip)
                .map_err(codec_error)?;
        opus.set_inband_fec(true).map_err(codec_error)?;
        opus.set_packet_loss_perc(EXPECTED_LOSS_PERCENT)
            .map_err(codec_error)?;
        opus.set_dtx(true).map_err(codec_error)?;
        let mut encoder = Encoder {
            opus,
            packet: vec![0; MAX_PACKET],
        };
        encoder.set_bitrate(bitrate);
        Ok(encoder)
    }

    pub fn set_bitrate(&mut self, bitrate: u32) {
        let bits = bitrate.clamp(MIN_BITRATE, MAX_BITRATE) as i32;
        // Only fails for out-of-range values, which the clamp rules out.
        let _ = self.opus.set_bitrate(opus::Bitrate::Bits(bits));
    }

    /// Encodes one frame. Returns `None` when DTX decides the frame is
    /// silence and nothing needs sending.
    pub fn encode(&mut self, frame: &MonoFrame) -> Result<Option<&[u8]>, AudioError> {
        let len = self
            .opus
            .encode_float(frame, &mut self.packet)
            .map_err(codec_error)?;
        // In DTX, Opus emits packets of at most two bytes that carry no audio
        // and need not be sent.
        if len <= 2 {
            return Ok(None);
        }
        Ok(Some(&self.packet[..len]))
    }

    /// Resets state between talk spurts.
    pub fn reset(&mut self) {
        // Only fails on an invalid encoder, which `new` rules out.
        let _ = self.opus.reset_state();
    }
}

fn codec_error(err: opus::Error) -> AudioError {
    AudioError::Codec(err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FRAME_SAMPLES;

    fn tone(start: usize) -> MonoFrame {
        std::array::from_fn(|i| {
            let t = (start + i) as f32 / SAMPLE_RATE as f32;
            0.3 * (std::f32::consts::TAU * 300.0 * t).sin()
        })
    }

    /// Average packet size over a second of tone.
    fn average_packet(encoder: &mut Encoder) -> usize {
        let total: usize = (0..50)
            .map(|i| {
                encoder
                    .encode(&tone(i * FRAME_SAMPLES))
                    .unwrap()
                    .unwrap()
                    .len()
            })
            .sum();
        total / 50
    }

    #[test]
    fn packet_size_follows_bitrate() {
        let mut encoder = Encoder::new(24_000).unwrap();
        // 24 kbps is 60 bytes per 20 ms frame.
        let size = average_packet(&mut encoder);
        assert!((40..=80).contains(&size), "{size}");
        encoder.set_bitrate(64_000);
        // VBR spends less on a pure tone than on speech, but still more.
        assert!(average_packet(&mut encoder) > size);
        // Out-of-range bitrates are clamped rather than rejected.
        encoder.set_bitrate(1_000_000);
        assert!(average_packet(&mut encoder) <= 200);
        assert!(Encoder::new(1).is_ok());
    }

    #[test]
    fn dtx_skips_silence() {
        let mut encoder = Encoder::new(24_000).unwrap();
        for i in 0..10 {
            assert!(encoder.encode(&tone(i * FRAME_SAMPLES)).unwrap().is_some());
        }
        let silent = (0..50)
            .filter(|_| encoder.encode(&[0.0; FRAME_SAMPLES]).unwrap().is_none())
            .count();
        // DTX starts after a short run of silence and then skips most frames.
        assert!(silent > 30, "{silent}");
        encoder.reset();
        assert!(encoder.encode(&tone(0)).unwrap().is_some());
    }
}
