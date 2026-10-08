//! Audio capture, playback, Opus coding, jitter buffering and mixing.
//!
//! Contains no networking and knows nothing about the game world: callers pass
//! packets in and per-source gain/pan, and get PCM frames out. Everything runs
//! at [`SAMPLE_RATE`] in [`FRAME_SAMPLES`]-sample frames; device I/O resamples
//! at the edges.
//!
//! Send path: [`FrameSource`] → [`InputProcessor`] → [`Encoder`].
//! Receive path: packets → [`ReceiveStream`] (one per remote speaker) →
//! [`Mixer`] → [`FrameSink`].

mod codec;
mod device;
mod input;
mod mixer;
mod receive;
mod resample;
mod test_io;

use std::time::Duration;

pub use crate::codec::{Encoder, MAX_PACKET};
pub use crate::device::{DeviceChoice, DeviceInfo, Devices, Microphone, Speakers, devices};
pub use crate::input::{InputActivity, InputConfig, InputProcessor};
pub use crate::mixer::{Mixer, Spatial};
pub use crate::receive::{JitterConfig, Packet, Pulled, ReceiveStream, StreamStats};
pub use crate::test_io::{NullSink, Recording, SilenceSource, ToneSource, WavSource};

/// Sample rate used throughout the pipeline.
pub const SAMPLE_RATE: u32 = 48_000;

/// Samples per frame at [`SAMPLE_RATE`].
pub const FRAME_SAMPLES: usize = 960;

/// Duration of one frame.
pub const FRAME_DURATION: Duration = Duration::from_millis(20);

/// One frame of mono samples in `-1.0..=1.0`.
pub type MonoFrame = [f32; FRAME_SAMPLES];

/// One frame of stereo samples as `[left, right]` pairs.
pub type StereoFrame = [[f32; 2]; FRAME_SAMPLES];

/// Produces mono frames: a microphone, a WAV file, a test tone.
pub trait FrameSource: Send {
    /// Fills `frame` with the next frame if a whole one is available.
    /// Returns `false` without blocking otherwise.
    fn read(&mut self, frame: &mut MonoFrame) -> bool;

    /// Why the source stopped producing frames, once it has failed for good
    /// (for example, the device was unplugged). The engine reports it and
    /// reopens the device.
    fn failure(&self) -> Option<String> {
        None
    }

    /// Times captured audio was dropped because it wasn't read in time
    /// (overruns), for diagnostics. Sources without a device report 0.
    fn glitches(&self) -> u64 {
        0
    }
}

/// Consumes stereo frames: speakers, or a recorder for tests.
pub trait FrameSink: Send {
    /// Whether the sink has room for another frame now. The mixer runs only
    /// when this is true, so the sink's clock paces playback.
    fn wants_frame(&self) -> bool;

    fn write(&mut self, frame: &StereoFrame);

    /// Why the sink stopped accepting frames, once it has failed for good.
    fn failure(&self) -> Option<String> {
        None
    }

    /// Times the device ran out of audio to play (underruns), for
    /// diagnostics. Sinks without a device report 0.
    fn glitches(&self) -> u64 {
        0
    }
}

/// Lets callers choose a source at runtime.
impl FrameSource for Box<dyn FrameSource> {
    fn read(&mut self, frame: &mut MonoFrame) -> bool {
        (**self).read(frame)
    }

    fn failure(&self) -> Option<String> {
        (**self).failure()
    }

    fn glitches(&self) -> u64 {
        (**self).glitches()
    }
}

/// Lets callers choose a sink at runtime.
impl FrameSink for Box<dyn FrameSink> {
    fn wants_frame(&self) -> bool {
        (**self).wants_frame()
    }

    fn write(&mut self, frame: &StereoFrame) {
        (**self).write(frame);
    }

    fn failure(&self) -> Option<String> {
        (**self).failure()
    }

    fn glitches(&self) -> u64 {
        (**self).glitches()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("no audio device matches {0:?}")]
    NoDevice(String),
    #[error("audio device error: {0}")]
    Device(String),
    #[error("opus error: {0}")]
    Codec(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
