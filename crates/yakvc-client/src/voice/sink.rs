//! External voice output: another mod renders the voices instead of the
//! speakers (DESIGN.md "Voice transport and audio pipeline", #174).

use yakvc_audio::MonoFrame;
use yakvc_proto::Uuid;

use crate::world::Vec3;

/// Plays decoded voice in place of the speakers, for a mod that renders
/// voices itself, with the game's own sounds. It gets each frame the speakers
/// would have played, as decoded: before panning, distance gain, occlusion,
/// reverb, the game volume and the limiter. Only the rules of who is heard
/// apply, and the player's volume setting.
///
/// Called on the audio thread: every method must return quickly and never
/// block.
pub trait VoiceSink: Send {
    /// Whether the sink wants the next 20 ms of voice now. Paces playback,
    /// like [`FrameSink::wants_frame`](yakvc_audio::FrameSink::wants_frame).
    fn wants_frame(&mut self) -> bool;

    /// One playable 20 ms frame from one speaker.
    fn frame(&mut self, frame: &VoiceFrame<'_>);

    /// `uuid`'s voice link is gone: no more frames from it until it comes
    /// back.
    fn stream_end(&mut self, uuid: Uuid);
}

/// One speaker's frame for a [`VoiceSink`].
#[derive(Debug, Clone, Copy)]
pub struct VoiceFrame<'a> {
    pub uuid: Uuid,
    /// 48 kHz mono, as decoded.
    pub pcm: &'a MonoFrame,
    /// Counts play ticks: the frames played together share it.
    pub tick: u64,
    /// The player's volume setting, 0 to 2.
    pub volume: f32,
    /// The local voice range in blocks, beyond which nothing is played; 0
    /// when the frame isn't positioned.
    pub range: f32,
    /// Group voice.
    pub group: bool,
    /// Heard from the speaker's place in the world. Group voice and direct
    /// calls aren't: they play centred.
    pub positioned: bool,
    /// Where the speakers would place the voice: the speaker's eye, or the
    /// opening the voice comes round through, as far away as its way is long.
    /// Zero when not positioned. A renderer with acoustics of its own can
    /// ignore it, and the occlusion.
    pub heard_from: Vec3,
    /// How much of the way the speakers would block, from 0 (open air) to 1.
    /// Zero when not positioned.
    pub occlusion: f32,
}
