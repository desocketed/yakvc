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

pub(crate) fn codec_error(err: opus::Error) -> AudioError {
    AudioError::Codec(err.to_string())
}
