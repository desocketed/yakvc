use std::time::Duration;

use crate::MonoFrame;

#[derive(Debug, Clone, PartialEq)]
pub struct InputConfig {
    /// Linear gain applied before everything else.
    pub gain: f32,
    pub noise_suppression: bool,
    /// Level above which a frame counts as voice.
    pub vad_threshold_db: f32,
    /// How long voice activity stays on after the level drops.
    pub vad_hangover: Duration,
}

impl Default for InputConfig {
    fn default() -> Self {
        InputConfig {
            gain: 1.0,
            noise_suppression: true,
            vad_threshold_db: -45.0,
            vad_hangover: Duration::from_millis(300),
        }
    }
}

/// Gain, noise suppression and voice activity detection for captured frames.
#[derive(Debug)]
pub struct InputProcessor {
    _p: (),
}

/// Result of processing one frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InputActivity {
    /// Level after processing, in dBFS.
    pub level_db: f32,
    /// Voice detected, including hangover.
    pub voice: bool,
}

impl InputProcessor {
    pub fn new(config: InputConfig) -> Self {
        let _ = config;
        todo!()
    }

    pub fn set_config(&mut self, config: InputConfig) {
        let _ = config;
        todo!()
    }

    /// Processes `frame` in place.
    pub fn process(&mut self, frame: &mut MonoFrame) -> InputActivity {
        let _ = frame;
        todo!()
    }
}
