//! Audio capture, playback, Opus coding, jitter buffering and spatial mixing.
//! Contains no networking.

/// Sample rate used throughout the pipeline.
pub const SAMPLE_RATE: u32 = 48_000;

/// Samples per 20 ms frame at [`SAMPLE_RATE`].
pub const FRAME_SAMPLES: usize = 960;
