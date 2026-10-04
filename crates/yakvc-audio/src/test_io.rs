//! Frame sources and sinks that need no audio hardware, for the CLI's
//! `--wav` / `--null-out` options, bots and integration tests.

use std::path::Path;
use std::sync::Arc;

use crate::{AudioError, FrameSink, FrameSource, MonoFrame, StereoFrame};

/// Plays a WAV file (any rate or channel count), paced in real time.
#[derive(Debug)]
pub struct WavSource {
    _p: (),
}

impl WavSource {
    pub fn open(path: &Path, looping: bool) -> Result<Self, AudioError> {
        let _ = (path, looping);
        todo!()
    }
}

impl FrameSource for WavSource {
    fn read(&mut self, frame: &mut MonoFrame) -> bool {
        let _ = frame;
        todo!()
    }
}

/// A continuous sine tone, paced in real time.
#[derive(Debug)]
pub struct ToneSource {
    _p: (),
}

impl ToneSource {
    pub fn new(frequency_hz: f32) -> Self {
        let _ = frequency_hz;
        todo!()
    }
}

impl FrameSource for ToneSource {
    fn read(&mut self, frame: &mut MonoFrame) -> bool {
        let _ = frame;
        todo!()
    }
}

/// Accepts frames in real time and records them. Cloning the [`Recording`]
/// returned by [`NullSink::new`] lets a test inspect output while the engine
/// owns the sink.
#[derive(Debug)]
pub struct NullSink {
    _recording: Recording,
}

/// Shared view of everything a [`NullSink`] has received.
#[derive(Debug, Clone)]
pub struct Recording {
    _shared: Arc<()>,
}

impl NullSink {
    pub fn new() -> (NullSink, Recording) {
        todo!()
    }
}

impl FrameSink for NullSink {
    fn wants_frame(&self) -> bool {
        todo!()
    }

    fn write(&mut self, frame: &StereoFrame) {
        let _ = frame;
        todo!()
    }
}

impl Recording {
    /// Frames received so far.
    pub fn frames(&self) -> usize {
        todo!()
    }

    /// Frames whose peak level exceeded `-60` dBFS.
    pub fn audible_frames(&self) -> usize {
        todo!()
    }

    /// Takes the recorded samples, leaving the recording empty.
    pub fn take(&self) -> Vec<[f32; 2]> {
        todo!()
    }
}
