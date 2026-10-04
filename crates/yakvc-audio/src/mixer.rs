use crate::{MonoFrame, StereoFrame};

/// Where a source sits in the stereo image. Computed by the caller from game
/// positions.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spatial {
    /// Linear gain, including distance and per-player volume.
    pub gain: f32,
    /// `-1.0` fully left to `1.0` fully right; equal-power panning.
    pub pan: f32,
}

/// Mixes mono sources into one stereo frame, with master gain and a soft
/// limiter.
#[derive(Debug)]
pub struct Mixer {
    _p: (),
}

impl Mixer {
    pub fn new() -> Self {
        todo!()
    }

    pub fn set_master_gain(&mut self, gain: f32) {
        let _ = gain;
        todo!()
    }

    /// Overwrites `out` with the mix of `sources`.
    pub fn mix<'a>(
        &mut self,
        sources: impl IntoIterator<Item = (&'a MonoFrame, Spatial)>,
        out: &mut StereoFrame,
    ) {
        let _ = (sources.into_iter(), out);
        todo!()
    }
}

impl Default for Mixer {
    fn default() -> Self {
        Self::new()
    }
}
