use serde::Serialize;

use crate::{AudioError, FrameSink, FrameSource, MonoFrame, StereoFrame};

/// Audio devices on this machine. Serializes to the JSON returned by
/// `yakvc_list_devices`.
#[derive(Debug, Clone, Serialize)]
pub struct Devices {
    pub inputs: Vec<DeviceInfo>,
    pub outputs: Vec<DeviceInfo>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeviceInfo {
    pub name: String,
    pub is_default: bool,
}

/// Which device to open.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum DeviceChoice {
    #[default]
    Default,
    /// Exactly this device name.
    Named(String),
    /// The device whose name best matches, else the default. Used to follow
    /// Minecraft's selected sound device, whose name differs from cpal's.
    ClosestTo(String),
}

pub fn devices() -> Result<Devices, AudioError> {
    todo!()
}

/// Captures from an input device, resampled to mono 48 kHz.
pub struct Microphone {
    _p: (),
}

impl Microphone {
    pub fn open(device: &DeviceChoice) -> Result<Self, AudioError> {
        let _ = device;
        todo!()
    }
}

impl FrameSource for Microphone {
    fn read(&mut self, frame: &mut MonoFrame) -> bool {
        let _ = frame;
        todo!()
    }
}

/// Plays to an output device, resampled from stereo 48 kHz.
pub struct Speakers {
    _p: (),
}

impl Speakers {
    pub fn open(device: &DeviceChoice) -> Result<Self, AudioError> {
        let _ = device;
        todo!()
    }
}

impl FrameSink for Speakers {
    fn wants_frame(&self) -> bool {
        todo!()
    }

    fn write(&mut self, frame: &StereoFrame) {
        let _ = frame;
        todo!()
    }
}

impl std::fmt::Debug for Microphone {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Microphone").finish_non_exhaustive()
    }
}

impl std::fmt::Debug for Speakers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Speakers").finish_non_exhaustive()
    }
}
