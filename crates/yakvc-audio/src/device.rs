use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{
    BufferSize, ErrorKind, FromSample, I24, Sample, SampleFormat, SizedSample, StreamConfig,
    SupportedBufferSize, SupportedStreamConfig,
};
use rtrb::{Consumer, Producer, RingBuffer};
use serde::Serialize;

use crate::resample::Resampler;
use crate::{
    AudioError, FRAME_SAMPLES, FrameSink, FrameSource, MonoFrame, SAMPLE_RATE, StereoFrame,
};

/// How much audio each ring between a device callback and the audio thread
/// can hold.
const RING_DURATION: Duration = Duration::from_millis(500);

/// Captured audio the audio thread may fall behind by before the oldest is
/// dropped, so a stall does not turn into lasting latency.
const MAX_CAPTURE_BACKLOG: usize = 5 * FRAME_SAMPLES;

/// Audio queued for the output device while it still asks for more. Enough
/// to ride out a late audio thread, little enough to keep latency down.
const PLAYBACK_LEAD: Duration = Duration::from_millis(40);

/// Audio devices on this machine. Serializes to the JSON returned by
/// `yakvc_list_devices`.
#[derive(Debug, Clone, Serialize)]
pub struct Devices {
    pub inputs: Vec<DeviceInfo>,
    pub outputs: Vec<DeviceInfo>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeviceInfo {
    /// What to store in the config and open with [`DeviceChoice::Id`].
    pub id: String,
    /// For people to read. Unique within its list.
    pub name: String,
    pub is_default: bool,
}

/// Which device to open.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum DeviceChoice {
    #[default]
    Default,
    /// The device with this [`DeviceInfo::id`].
    Id(String),
    /// The device whose name best matches, else the default. Used to follow
    /// Minecraft's selected sound device, whose name differs from cpal's.
    ClosestTo(String),
}

pub fn devices() -> Result<Devices, AudioError> {
    let host = cpal::default_host();
    Ok(Devices {
        inputs: list(
            Direction::Input.devices(&host)?,
            Direction::Input.default(&host),
        ),
        outputs: list(
            Direction::Output.devices(&host)?,
            Direction::Output.default(&host),
        ),
    })
}

fn list(devices: Vec<Listed>, default: Option<cpal::Device>) -> Vec<DeviceInfo> {
    devices
        .into_iter()
        .map(|listed| DeviceInfo {
            is_default: Some(&listed.device) == default.as_ref(),
            id: listed.id,
            name: listed.name,
        })
        .collect()
}

/// A device as [`Direction::devices`] lists it.
struct Listed {
    id: String,
    name: String,
    device: cpal::Device,
}

#[derive(Debug, Clone, Copy)]
enum Direction {
    Input,
    Output,
}

impl Direction {
    /// The devices, with unique names. Ones that fail to report an id or a
    /// name are left out.
    fn devices(self, host: &cpal::Host) -> Result<Vec<Listed>, AudioError> {
        let devices: Vec<cpal::Device> = match self {
            Direction::Input => host.input_devices().map_err(device_error)?.collect(),
            Direction::Output => host.output_devices().map_err(device_error)?.collect(),
        };
        let mut found = Vec::new();
        let mut names = Vec::new();
        for device in devices {
            let (Ok(id), Ok(description)) = (device.id(), device.description()) else {
                continue;
            };
            // PulseAudio lists each output's monitor as an input, which would
            // send the game's sound to everyone.
            if id.id().ends_with(".monitor") {
                continue;
            }
            names.push(Name {
                name: description.name().to_owned(),
                // ALSA puts the card on the first line and what the PCM is
                // for (raw hardware, with conversions...) on the next.
                detail: description.extended().nth(1).map(str::to_owned),
                id: id.id().to_owned(),
            });
            found.push((id.to_string(), device));
        }
        Ok(found
            .into_iter()
            .zip(unique_names(&names))
            .map(|((id, device), name)| Listed { id, name, device })
            .collect())
    }

    fn default(self, host: &cpal::Host) -> Option<cpal::Device> {
        match self {
            Direction::Input => host.default_input_device(),
            Direction::Output => host.default_output_device(),
        }
    }

    fn find(self, choice: &DeviceChoice) -> Result<cpal::Device, AudioError> {
        let host = cpal::default_host();
        let no_device = |name: &str| AudioError::NoDevice(name.to_owned());
        match choice {
            DeviceChoice::Default => self.default(&host).ok_or_else(|| no_device("default")),
            DeviceChoice::Id(wanted) => {
                let mut devices = self.devices(&host)?;
                let index = devices
                    .iter()
                    .position(|d| d.id == *wanted)
                    .ok_or_else(|| no_device(wanted))?;
                Ok(devices.swap_remove(index).device)
            }
            DeviceChoice::ClosestTo(wanted) => {
                let mut devices = self.devices(&host)?;
                match closest_name(wanted, devices.iter().map(|d| d.name.as_str())) {
                    Some(index) => Ok(devices.swap_remove(index).device),
                    None => self.default(&host).ok_or_else(|| no_device(wanted)),
                }
            }
        }
    }
}

/// What a device calls itself.
struct Name {
    name: String,
    /// More about the device, if the host says more.
    detail: Option<String>,
    /// The host's id for it, without the host prefix.
    id: String,
}

/// One name per device, telling apart devices that share one: ALSA gives
/// every PCM of a card (`hw:`, `plughw:`, `sysdefault:`, `front:`...) the
/// card's name. A shared name gets the detail added, and if that isn't
/// enough, the id.
fn unique_names(names: &[Name]) -> Vec<String> {
    fn shared(labels: &[String], label: &str) -> bool {
        labels.iter().filter(|l| *l == label).count() > 1
    }
    let plain: Vec<String> = names.iter().map(|n| n.name.clone()).collect();
    let detailed: Vec<String> = names
        .iter()
        .map(|n| match &n.detail {
            Some(detail) if shared(&plain, &n.name) => format!("{} ({detail})", n.name),
            _ => n.name.clone(),
        })
        .collect();
    names
        .iter()
        .zip(&detailed)
        .map(|(n, label)| match shared(&detailed, label) {
            true => format!("{label} [{}]", n.id),
            false => label.clone(),
        })
        .collect()
}

/// Index of the name sharing the most words with `wanted`, counting only
/// names that share more than half of their words. Minecraft reports OpenAL
/// names such as "OpenAL Soft on Speakers (Realtek(R) Audio)" where cpal says
/// "Speakers (Realtek(R) Audio)", so exact matching would rarely work.
fn closest_name<'a>(wanted: &str, names: impl Iterator<Item = &'a str>) -> Option<usize> {
    let wanted = words(wanted);
    let mut best: Option<(usize, usize, usize)> = None; // (index, shared, total)
    for (index, name) in names.enumerate() {
        let words = words(name);
        let shared = words.iter().filter(|w| wanted.contains(w)).count();
        if shared * 2 <= words.len() {
            continue;
        }
        // More shared words wins; on a tie, the name with fewer extra words.
        let better = best.is_none_or(|(_, best_shared, best_total)| {
            shared > best_shared || (shared == best_shared && words.len() < best_total)
        });
        if better {
            best = Some((index, shared, words.len()));
        }
    }
    best.map(|(index, _, _)| index)
}

fn words(name: &str) -> Vec<String> {
    name.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// Captures from an input device, resampled to mono 48 kHz.
pub struct Microphone {
    /// Keeps the device running; dropping it stops capture.
    _stream: cpal::Stream,
    /// Mono samples at the device rate, from the device callback.
    ring: Consumer<f32>,
    resampler: Resampler,
    /// Samples read from the ring, before resampling.
    captured: Vec<f32>,
    /// Resampled samples not yet handed out as a frame.
    pending: Vec<f32>,
    /// Counts overruns.
    health: Arc<Health>,
}

impl Microphone {
    /// Times captured audio was dropped because nobody read it in time,
    /// since opening.
    pub fn overruns(&self) -> u64 {
        self.health.glitches()
    }

    pub fn open(device: &DeviceChoice) -> Result<Self, AudioError> {
        let device = Direction::Input.find(device)?;
        let supported = device.default_input_config().map_err(device_error)?;
        let config = stream_config(&supported);
        let rate = config.sample_rate;
        let (producer, ring) = RingBuffer::new(ring_len(rate));
        let health = Arc::new(Health::default());

        let h = health.clone();
        let stream = match supported.sample_format() {
            SampleFormat::F32 => capture::<f32>(&device, config, producer, h),
            SampleFormat::I16 => capture::<i16>(&device, config, producer, h),
            SampleFormat::I24 => capture::<I24>(&device, config, producer, h),
            SampleFormat::I32 => capture::<i32>(&device, config, producer, h),
            SampleFormat::U8 => capture::<u8>(&device, config, producer, h),
            SampleFormat::U16 => capture::<u16>(&device, config, producer, h),
            other => return Err(unsupported_format(other)),
        }
        .map_err(device_error)?;
        stream.play().map_err(device_error)?;

        Ok(Microphone {
            _stream: stream,
            ring,
            resampler: Resampler::new(rate, SAMPLE_RATE, 1)?,
            captured: Vec::new(),
            pending: Vec::new(),
            health,
        })
    }
}

/// Opens a capture stream whose callback downmixes to mono and pushes into
/// `ring`. The callback never blocks or allocates; if the ring is full, the
/// newest samples are dropped and counted as one overrun.
fn capture<T>(
    device: &cpal::Device,
    config: StreamConfig,
    mut ring: Producer<f32>,
    health: Arc<Health>,
) -> Result<cpal::Stream, cpal::Error>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let channels = usize::from(config.channels);
    let errors = health.clone();
    device.build_input_stream(
        config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            let mut dropped = false;
            for frame in data.chunks_exact(channels) {
                let sum: f32 = frame.iter().map(|&s| f32::from_sample(s)).sum();
                dropped |= ring.push(sum / channels as f32).is_err();
            }
            if dropped {
                health.glitch();
            }
        },
        move |err| errors.stream_error(&err),
        None,
    )
}

impl FrameSource for Microphone {
    fn read(&mut self, frame: &mut MonoFrame) -> bool {
        self.captured.clear();
        while let Ok(sample) = self.ring.pop() {
            self.captured.push(sample);
        }
        self.resampler.process(&self.captured, &mut self.pending);
        if self.pending.len() > MAX_CAPTURE_BACKLOG {
            let excess = self.pending.len() - MAX_CAPTURE_BACKLOG;
            self.pending.drain(..excess);
            self.health.glitch();
        }
        if self.pending.len() < FRAME_SAMPLES {
            return false;
        }
        frame.copy_from_slice(&self.pending[..FRAME_SAMPLES]);
        self.pending.drain(..FRAME_SAMPLES);
        true
    }

    fn failure(&self) -> Option<String> {
        self.health.failure()
    }

    fn glitches(&self) -> u64 {
        self.health.glitches()
    }
}

/// Plays to an output device, resampled from stereo 48 kHz.
pub struct Speakers {
    /// Keeps the device running; dropping it stops playback.
    _stream: cpal::Stream,
    /// Stereo samples at the device rate, for the device callback.
    ring: Producer<[f32; 2]>,
    resampler: Resampler,
    /// Device-rate samples queued once the ring holds this many.
    lead_samples: usize,
    /// Resampled, interleaved samples on their way into the ring.
    resampled: Vec<f32>,
    /// Counts underruns.
    health: Arc<Health>,
}

impl Speakers {
    /// Times the device asked for audio and the queue was empty, since
    /// opening. Each is an audible glitch.
    pub fn underruns(&self) -> u64 {
        self.health.glitches()
    }

    pub fn open(device: &DeviceChoice) -> Result<Self, AudioError> {
        let device = Direction::Output.find(device)?;
        let supported = device.default_output_config().map_err(device_error)?;
        let config = stream_config(&supported);
        let rate = config.sample_rate;
        let (ring, consumer) = RingBuffer::new(ring_len(rate));
        let health = Arc::new(Health::default());

        let h = health.clone();
        let stream = match supported.sample_format() {
            SampleFormat::F32 => playback::<f32>(&device, config, consumer, h),
            SampleFormat::I16 => playback::<i16>(&device, config, consumer, h),
            SampleFormat::I24 => playback::<I24>(&device, config, consumer, h),
            SampleFormat::I32 => playback::<i32>(&device, config, consumer, h),
            SampleFormat::U8 => playback::<u8>(&device, config, consumer, h),
            SampleFormat::U16 => playback::<u16>(&device, config, consumer, h),
            other => return Err(unsupported_format(other)),
        }
        .map_err(device_error)?;
        stream.play().map_err(device_error)?;

        Ok(Speakers {
            _stream: stream,
            ring,
            resampler: Resampler::new(SAMPLE_RATE, rate, 2)?,
            lead_samples: (rate as f32 * PLAYBACK_LEAD.as_secs_f32()) as usize,
            resampled: Vec::new(),
            health,
        })
    }
}

/// Opens a playback stream whose callback pops from `ring`, playing silence
/// when it runs dry. Left and right go to the first two channels; a mono
/// device gets their average.
fn playback<T>(
    device: &cpal::Device,
    config: StreamConfig,
    mut ring: Consumer<[f32; 2]>,
    health: Arc<Health>,
) -> Result<cpal::Stream, cpal::Error>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = usize::from(config.channels);
    let errors = health.clone();
    // The ring is empty until the first write, which is not a glitch.
    let mut started = false;
    device.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            let mut ran_dry = false;
            for frame in data.chunks_exact_mut(channels) {
                let [left, right] = match ring.pop() {
                    Ok(pair) => {
                        started = true;
                        pair
                    }
                    Err(_) => {
                        ran_dry |= started;
                        [0.0; 2]
                    }
                };
                if let [only] = frame {
                    *only = T::from_sample((left + right) / 2.0);
                    continue;
                }
                for (channel, sample) in frame.iter_mut().enumerate() {
                    let value = match channel {
                        0 => left,
                        1 => right,
                        _ => 0.0,
                    };
                    *sample = T::from_sample(value);
                }
            }
            if ran_dry {
                health.glitch();
            }
        },
        move |err| errors.stream_error(&err),
        None,
    )
}

impl FrameSink for Speakers {
    fn wants_frame(&self) -> bool {
        let queued = self.ring.buffer().capacity() - self.ring.slots();
        queued < self.lead_samples
    }

    fn write(&mut self, frame: &StereoFrame) {
        self.resampled.clear();
        self.resampler
            .process(frame.as_flattened(), &mut self.resampled);
        for &pair in self.resampled.as_chunks::<2>().0 {
            // When full, the device has stalled; dropping is all we can do.
            let _ = self.ring.push(pair);
        }
    }

    fn failure(&self) -> Option<String> {
        self.health.failure()
    }

    fn glitches(&self) -> u64 {
        self.health.glitches()
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

/// What a device's callbacks report to its owner.
#[derive(Debug, Default)]
struct Health {
    /// Overruns for a microphone, underruns for speakers.
    glitches: AtomicU64,
    /// Why the stream stopped for good. The first fatal error wins.
    failure: OnceLock<String>,
}

impl Health {
    /// Counts one glitch. Lock-free, for the device callback.
    fn glitch(&self) {
        self.glitches.fetch_add(1, Ordering::Relaxed);
    }

    fn glitches(&self) -> u64 {
        self.glitches.load(Ordering::Relaxed)
    }

    /// Handles an error from the stream's error callback. Errors are rare,
    /// so allocating the message here does not hurt the audio thread.
    fn stream_error(&self, err: &cpal::Error) {
        match err.kind() {
            // The device dropped audio that our rings never saw.
            ErrorKind::Xrun => self.glitch(),
            // The stream will never deliver audio again; only reopening the
            // device helps.
            ErrorKind::DeviceNotAvailable
            | ErrorKind::HostUnavailable
            | ErrorKind::StreamInvalidated => {
                let _ = self.failure.set(err.to_string());
            }
            // Backends report everything else and keep the stream running.
            _ => {}
        }
    }

    fn failure(&self) -> Option<String> {
        self.failure.get().cloned()
    }
}

/// The device's own config, asking for 10 ms buffers where the device takes a
/// range. With PulseAudio's default buffers, capture lost most of its audio.
fn stream_config(supported: &SupportedStreamConfig) -> StreamConfig {
    let mut config = supported.config();
    if let SupportedBufferSize::Range { min, max } = *supported.buffer_size() {
        config.buffer_size = BufferSize::Fixed((config.sample_rate / 100).clamp(min, max));
    }
    config
}

fn ring_len(rate: u32) -> usize {
    (rate as f32 * RING_DURATION.as_secs_f32()) as usize
}

fn device_error(err: cpal::Error) -> AudioError {
    AudioError::Device(err.to_string())
}

fn unsupported_format(format: SampleFormat) -> AudioError {
    AudioError::Device(format!("unsupported sample format {format}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn closest(wanted: &str, names: &[&str]) -> Option<usize> {
        closest_name(wanted, names.iter().copied())
    }

    #[test]
    fn closest_name_follows_openal_names() {
        let names = [
            "HDMI Audio",
            "Speakers (Realtek(R) Audio)",
            "Headphones (USB Audio)",
        ];
        assert_eq!(
            closest("OpenAL Soft on Speakers (Realtek(R) Audio)", &names),
            Some(1)
        );
        assert_eq!(
            closest("OpenAL Soft on Headphones (USB Audio)", &names),
            Some(2)
        );
        assert_eq!(closest("speakers (realtek(r) audio)", &names), Some(1));
    }

    #[test]
    fn closest_name_rejects_weak_matches() {
        let names = ["HDMI Audio", "default"];
        assert_eq!(closest("OpenAL Soft on Speakers (USB Audio)", &names), None);
        assert_eq!(closest("", &names), None);
        assert_eq!(closest("Speakers", &[]), None);
    }

    #[test]
    fn closest_name_prefers_fewer_extra_words() {
        let names = ["Speakers Digital Output", "Speakers"];
        assert_eq!(closest("Speakers", &names), Some(1));
    }

    fn name(name: &str, detail: Option<&str>, id: &str) -> Name {
        Name {
            name: name.into(),
            detail: detail.map(Into::into),
            id: id.into(),
        }
    }

    #[test]
    fn unique_names_tell_apart_one_cards_pcms() {
        let card = "Yeti Stereo Microphone, USB Audio";
        let raw = "Direct hardware device without any conversions";
        let plug = "Hardware device with all software conversions";
        let names = unique_names(&[
            name(card, Some(raw), "hw:CARD=Microphone,DEV=0"),
            name(card, Some(plug), "plughw:CARD=Microphone,DEV=0"),
            name(card, Some(plug), "plughw:CARD=1,DEV=0"),
            name("PipeWire Sound Server", None, "pipewire"),
        ]);
        assert_eq!(
            names,
            [
                format!("{card} ({raw})"),
                format!("{card} ({plug}) [plughw:CARD=Microphone,DEV=0]"),
                format!("{card} ({plug}) [plughw:CARD=1,DEV=0]"),
                "PipeWire Sound Server".to_owned(),
            ]
        );
    }

    #[test]
    fn unique_names_fall_back_to_the_id() {
        let names = unique_names(&[name("USB Audio", None, "a"), name("USB Audio", None, "b")]);
        assert_eq!(names, ["USB Audio [a]", "USB Audio [b]"]);
    }

    #[test]
    fn health_counts_xruns_as_glitches() {
        let health = Health::default();
        health.glitch();
        health.stream_error(&ErrorKind::Xrun.into());
        assert_eq!(health.glitches(), 2);
        assert_eq!(health.failure(), None);
    }

    #[test]
    fn health_keeps_the_first_fatal_error() {
        let health = Health::default();
        let unplugged = cpal::Error::with_message(ErrorKind::DeviceNotAvailable, "unplugged");
        health.stream_error(&unplugged);
        health.stream_error(&ErrorKind::StreamInvalidated.into());
        assert_eq!(health.failure().as_deref(), Some("unplugged"));
        assert_eq!(health.glitches(), 0);
    }

    #[test]
    fn health_ignores_transient_errors() {
        let health = Health::default();
        health.stream_error(&ErrorKind::DeviceBusy.into());
        health.stream_error(&ErrorKind::DeviceChanged.into());
        health.stream_error(&ErrorKind::RealtimeDenied.into());
        assert_eq!(health.failure(), None);
        assert_eq!(health.glitches(), 0);
    }

    #[test]
    #[ignore = "needs audio hardware"]
    fn lists_devices() {
        let devices = devices().unwrap();
        assert!(!devices.inputs.is_empty() || !devices.outputs.is_empty());
    }

    #[test]
    #[ignore = "needs audio hardware"]
    fn microphone_delivers_frames() {
        let mut mic = Microphone::open(&DeviceChoice::Default).unwrap();
        let mut frame = [0.0; FRAME_SAMPLES];
        let start = std::time::Instant::now();
        let mut frames = 0;
        while start.elapsed() < Duration::from_secs(1) {
            if mic.read(&mut frame) {
                frames += 1;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!((40..=60).contains(&frames), "{frames} frames in 1 s");
    }

    #[test]
    #[ignore = "needs audio hardware"]
    fn speakers_take_frames_in_real_time() {
        let mut speakers = Speakers::open(&DeviceChoice::Default).unwrap();
        let frame = [[0.0; 2]; FRAME_SAMPLES];
        let start = std::time::Instant::now();
        let mut frames = 0;
        while start.elapsed() < Duration::from_secs(1) {
            if speakers.wants_frame() {
                speakers.write(&frame);
                frames += 1;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!((40..=60).contains(&frames), "{frames} frames in 1 s");
    }
}
