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

    /// Whether the sink wants the player's own voice too, for
    /// [`own_voice`](Self::own_voice). No by default: a sink that played it
    /// as it plays other voices would make the player hear themselves.
    fn wants_own_voice(&self) -> bool {
        false
    }

    /// The player's own voice, `uuid` being the player: each 20 ms frame
    /// that goes out while they transmit, after input processing, as their
    /// listeners get it. Not while deafened. For a renderer that lets the
    /// room around the player answer them (its reverb, never the dry voice).
    /// Paced by the microphone, not by [`wants_frame`](Self::wants_frame).
    fn own_voice(&mut self, _uuid: Uuid, _pcm: &MonoFrame) {}
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
