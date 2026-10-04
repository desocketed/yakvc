use crate::{AudioError, MonoFrame};

/// Opus encoder: VOIP mode, in-band FEC and DTX on.
#[derive(Debug)]
pub struct Encoder {
    _p: (),
}

impl Encoder {
    /// `bitrate` in bits per second, clamped to 16–64 kbps.
    pub fn new(bitrate: u32) -> Result<Self, AudioError> {
        let _ = bitrate;
        todo!()
    }

    pub fn set_bitrate(&mut self, bitrate: u32) {
        let _ = bitrate;
        todo!()
    }

    /// Encodes one frame. Returns `None` when DTX decides the frame is
    /// silence and nothing needs sending.
    pub fn encode(&mut self, frame: &MonoFrame) -> Result<Option<&[u8]>, AudioError> {
        let _ = frame;
        todo!()
    }

    /// Resets state between talk spurts.
    pub fn reset(&mut self) {
        todo!()
    }
}
