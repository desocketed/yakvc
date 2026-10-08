//! Who hears whom, and how loud: the rules from DESIGN.md "Voice transport
//! and audio pipeline", as plain functions over a snapshot of game state.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use yakvc_audio::{DeviceChoice, Spatial};
use yakvc_shared::{GroupAnnounce, GroupKey, Uuid};

use crate::config::{Activation, AudioConfig};
use crate::engine::PeerAudio;
use crate::world::{Input, Pose, Vec3, World, distance, interpolate};

/// Within this distance a speaker plays at full volume.
const FULL_VOLUME_DISTANCE: f64 = 4.0;
/// Snapshots further apart than this are not interpolated: the game paused
/// or lagged, so gliding would only move voices late.
const MAX_SNAPSHOT_GAP: Duration = Duration::from_millis(250);
/// Directly behind the listener a speaker is this much quieter.
const BEHIND_ATTENUATION: f64 = 0.3;
const MAX_RECIPIENTS: usize = 32;
/// Above this many recipients the encoder steps down to save upload.
const STEP_DOWN_RECIPIENTS: usize = 16;
pub(crate) const STEP_DOWN_BITRATE: u32 = 16_000;
/// Per-client budget for relayed voice, in bits per second.
const RELAY_BUDGET: u32 = 512_000;
/// Wire bytes per voice packet besides the Opus payload: the 10-byte voice
/// header, about 56 bytes of QUIC, UDP and IPv4, and about 10 of relay
/// framing.
const PACKET_OVERHEAD_BYTES: u32 = 76;
const PACKETS_PER_SECOND: u32 = 50;

/// What the voice layer knows about the game and the user's settings.
#[derive(Debug, Clone)]
pub(crate) struct VoiceState {
    pub own_uuid: Option<Uuid>,
    /// The latest snapshot, which decides who is sent to and heard.
    pub world: World,
    /// The snapshot before `world`, and when each arrived, so playback can
    /// glide between them.
    previous_world: World,
    previous_at: Instant,
    world_at: Instant,
    pub input: Input,
    pub game_volume: f32,
    pub peer_audio: HashMap<Uuid, PeerAudio>,
    pub voice_range: f32,
    pub friends_only: Option<Vec<Uuid>>,
    pub audio: AudioConfig,
    /// The game's selected sound device, which output follows unless
    /// `audio.output_device` is set.
    pub game_device: Option<String>,
    /// A direct call with no game (`yakvc-cli call`): every peer is heard at
    /// full volume and sent to, whatever the world says.
    pub direct: bool,
    /// The group we are in.
    pub group: Option<OwnGroup>,
    /// Peers whose announcement proves they are in our group.
    pub group_mates: HashSet<Uuid>,
}

/// The group we are in, and how.
#[derive(Debug, Clone)]
pub(crate) struct OwnGroup {
    pub key: GroupKey,
    pub locked: bool,
    /// Still hear, and be heard by, nearby players outside the group.
    pub nearby: bool,
}

/// A connected peer we might send this frame to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Candidate {
    pub uuid: Uuid,
    pub distance: f64,
    pub relayed: bool,
    /// A group mate, sent the frame with the `group` flag.
    pub group: bool,
}

/// Who gets this frame and at what bitrate.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SendPlan {
    pub recipients: Vec<Uuid>,
    /// Relayed peers in range that don't fit in the relay budget.
    pub relay_full: Vec<Uuid>,
    pub bitrate: u32,
}

impl VoiceState {
    pub(crate) fn new(
        audio: AudioConfig,
        voice_range: f32,
        friends_only: Option<Vec<Uuid>>,
    ) -> Self {
        let now = Instant::now();
        VoiceState {
            own_uuid: None,
            world: World::default(),
            previous_world: World::default(),
            previous_at: now,
            world_at: now,
            input: Input::default(),
            game_volume: 1.0,
            peer_audio: HashMap::new(),
            voice_range,
            friends_only,
            audio,
            game_device: None,
            direct: false,
            group: None,
            group_mates: HashSet::new(),
        }
    }

    pub(crate) fn set_world(&mut self, world: World, now: Instant) {
        self.previous_world = std::mem::replace(&mut self.world, world);
        self.previous_at = std::mem::replace(&mut self.world_at, now);
    }

    /// The world as playback should hear it at `now`: one snapshot behind,
    /// moving smoothly from the previous snapshot to the latest, so gain and
    /// pan don't step at every 20 Hz tick.
    pub(crate) fn world_at(&self, now: Instant) -> World {
        let interval = self.world_at.saturating_duration_since(self.previous_at);
        if interval.is_zero() || interval > MAX_SNAPSHOT_GAP {
            return self.world.clone();
        }
        let since = now.saturating_duration_since(self.world_at);
        let t = since.as_secs_f64() / interval.as_secs_f64();
        interpolate(&self.previous_world, &self.world, t.min(1.0))
    }

    pub(crate) fn input_device(&self) -> DeviceChoice {
        match &self.audio.input_device {
            Some(id) => DeviceChoice::Id(id.clone()),
            None => DeviceChoice::Default,
        }
    }

    /// The configured output device, else the one closest to the game's.
    pub(crate) fn output_device(&self) -> DeviceChoice {
        match (&self.audio.output_device, &self.game_device) {
            (Some(id), _) => DeviceChoice::Id(id.clone()),
            (None, Some(game)) => DeviceChoice::ClosestTo(game.clone()),
            (None, None) => DeviceChoice::Default,
        }
    }

    /// Whether the microphone goes out this frame. `voice_detected` is the
    /// VAD result, used in voice-activation mode.
    pub(crate) fn transmitting(&self, voice_detected: bool) -> bool {
        // A spectator can still talk to its group, which isn't positional.
        if self.input.muted || (self.input.spectator && self.group.is_none()) {
            return false;
        }
        match self.audio.activation {
            Activation::PushToTalk => self.input.push_to_talk,
            Activation::Voice => voice_detected,
        }
    }

    /// Whether we want `peer`'s audio at all. Sent to the peer as
    /// `ReceiveState` so it can stop sending when we don't.
    pub(crate) fn wants_audio_from(&self, peer: Uuid) -> bool {
        !self.input.deafened
            && !self.muted(peer)
            && self.is_friend(peer)
            && (self.hears_nearby() || self.is_group_mate(peer))
    }

    /// How to play a frame from `peer` without the `group` flag, or `None`
    /// to discard it.
    pub(crate) fn playback(&self, peer: Uuid) -> Option<Spatial> {
        if self.input.spectator || !self.hears_nearby() || !self.wants_audio_from(peer) {
            return None;
        }
        let volume = self.peer_audio.get(&peer).map_or(1.0, |audio| audio.volume);
        let spatial = if self.direct {
            Spatial {
                gain: 1.0,
                pan: 0.0,
            }
        } else {
            spatial(&self.world.listener, self.position(peer)?, self.voice_range)?
        };
        Some(Spatial {
            gain: spatial.gain * volume,
            ..spatial
        })
    }

    /// How to play a frame from `peer` with the `group` flag: centred, at the
    /// player's volume, and only from a group mate. Range and spectating
    /// don't matter, since group voice isn't positional.
    pub(crate) fn group_playback(&self, peer: Uuid) -> Option<Spatial> {
        if !self.is_group_mate(peer) || !self.wants_audio_from(peer) {
            return None;
        }
        let volume = self.peer_audio.get(&peer).map_or(1.0, |audio| audio.volume);
        Some(Spatial {
            gain: volume,
            pan: 0.0,
        })
    }

    /// Whether to send this frame to `peer`, and how. Whether the peer
    /// wants our audio is up to the peer and checked separately.
    pub(crate) fn send_to(&self, peer: Uuid, relayed: bool) -> Option<Candidate> {
        let group = self.is_group_mate(peer);
        let distance = if group {
            // Group mates come first for the recipient cap.
            0.0
        } else {
            self.send_distance(peer)?
        };
        Some(Candidate {
            uuid: peer,
            distance,
            relayed,
            group,
        })
    }

    /// The distance to `peer` if positional voice reaches it.
    fn send_distance(&self, peer: Uuid) -> Option<f64> {
        if !self.is_friend(peer) || self.input.spectator || !self.hears_nearby() {
            return None;
        }
        if self.direct {
            return Some(0.0);
        }
        let distance = distance(self.world.listener.pos, self.position(peer)?);
        (distance <= f64::from(self.voice_range)).then_some(distance)
    }

    fn position(&self, peer: Uuid) -> Option<Vec3> {
        let (_, pos) = self.world.players.iter().find(|(uuid, _)| *uuid == peer)?;
        Some(*pos)
    }

    /// What we tell peers about our group.
    pub(crate) fn announcement(&self) -> Option<GroupAnnounce> {
        let group = self.group.as_ref()?;
        Some(group.key.announce(self.own_uuid?, group.locked))
    }

    pub(crate) fn is_group_mate(&self, peer: Uuid) -> bool {
        self.group.is_some() && self.group_mates.contains(&peer)
    }

    /// Positional voice is on: we're in no group, or the group allows it.
    fn hears_nearby(&self) -> bool {
        self.group.as_ref().is_none_or(|group| group.nearby)
    }

    fn muted(&self, peer: Uuid) -> bool {
        self.peer_audio.get(&peer).is_some_and(|audio| audio.muted)
    }

    fn is_friend(&self, peer: Uuid) -> bool {
        self.friends_only
            .as_ref()
            .is_none_or(|friends| friends.contains(&peer))
    }
}

/// Gain and pan for a source at `source`, heard by `listener` with voice
/// range `range`. `None` beyond range.
///
/// Gain is 1 within 4 blocks and falls linearly to 0 at the range; sources
/// behind the listener are a little quieter. Pan is the sideways part of
/// the direction to the source, so equal-power panning in the mixer puts a
/// source at 90° fully to one side.
pub(crate) fn spatial(listener: &Pose, source: Vec3, range: f32) -> Option<Spatial> {
    let range = f64::from(range);
    let distance = distance(listener.pos, source);
    if distance > range {
        return None;
    }
    let mut gain = if distance <= FULL_VOLUME_DISTANCE {
        1.0
    } else {
        (range - distance) / (range - FULL_VOLUME_DISTANCE)
    };

    // Minecraft yaw: 0 faces +Z and grows clockwise seen from above, so
    // facing +Z the listener's right hand points to -X.
    let yaw = f64::from(listener.yaw).to_radians();
    let (forward_x, forward_z) = (-yaw.sin(), yaw.cos());
    let (right_x, right_z) = (-yaw.cos(), -yaw.sin());
    let (dx, dz) = (source.x - listener.pos.x, source.z - listener.pos.z);
    let horizontal = dx.hypot(dz);
    let (pan, front) = if horizontal < 1e-6 {
        (0.0, 1.0)
    } else {
        (
            (dx * right_x + dz * right_z) / horizontal,
            (dx * forward_x + dz * forward_z) / horizontal,
        )
    };
    if front < 0.0 {
        gain *= 1.0 + BEHIND_ATTENUATION * front;
    }
    Some(Spatial {
        gain: gain as f32,
        pan: pan as f32,
    })
}

/// Picks recipients nearest first, steps the bitrate down for big crowds,
/// and keeps relayed recipients within the relay budget (DESIGN.md
/// "Connection lifecycle").
pub(crate) fn plan_send(mut candidates: Vec<Candidate>, bitrate: u32) -> SendPlan {
    // Group mates first, then nearest first.
    candidates.sort_by(|a, b| {
        b.group
            .cmp(&a.group)
            .then(a.distance.total_cmp(&b.distance))
    });
    candidates.truncate(MAX_RECIPIENTS);

    let relayed = candidates.iter().filter(|c| c.relayed).count();
    let mut bitrate = bitrate;
    if candidates.len() > STEP_DOWN_RECIPIENTS || relayed > relay_capacity(bitrate) {
        bitrate = bitrate.min(STEP_DOWN_BITRATE);
    }

    let capacity = relay_capacity(bitrate);
    let mut plan = SendPlan {
        recipients: Vec::new(),
        relay_full: Vec::new(),
        bitrate,
    };
    let mut relayed_sent = 0;
    for candidate in candidates {
        if candidate.relayed {
            if relayed_sent == capacity {
                plan.relay_full.push(candidate.uuid);
                continue;
            }
            relayed_sent += 1;
        }
        plan.recipients.push(candidate.uuid);
    }
    plan
}

/// How many relayed streams fit in the relay budget at `bitrate`.
fn relay_capacity(bitrate: u32) -> usize {
    let per_stream = bitrate + PACKET_OVERHEAD_BYTES * 8 * PACKETS_PER_SECOND;
    (RELAY_BUDGET / per_stream) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uuid(n: u8) -> Uuid {
        Uuid::from_bytes([n; 16])
    }

    fn state() -> VoiceState {
        let mut state = VoiceState::new(AudioConfig::default(), 48.0, None);
        state.world.listener = listener(0.0);
        state
    }

    fn at(x: f64, z: f64) -> Vec3 {
        Vec3::new(x, 64.0, z)
    }

    fn listener(yaw: f32) -> Pose {
        Pose {
            pos: at(0.0, 0.0),
            yaw,
            pitch: 0.0,
        }
    }

    #[test]
    fn gain_is_full_up_close_and_fades_to_zero_at_range() {
        let gain = |d: f64| spatial(&listener(0.0), at(0.0, d), 48.0).map(|s| s.gain);
        assert_eq!(gain(0.0), Some(1.0));
        assert_eq!(gain(4.0), Some(1.0));
        assert!((gain(26.0).unwrap() - 0.5).abs() < 1e-6);
        assert_eq!(gain(48.0), Some(0.0));
        assert_eq!(gain(48.1), None);
    }

    #[test]
    fn pan_follows_yaw() {
        let pan = |yaw: f32, x: f64, z: f64| spatial(&listener(yaw), at(x, z), 48.0).unwrap().pan;
        // Facing +Z (south): -X is to the right.
        assert!((pan(0.0, -10.0, 0.0) - 1.0).abs() < 1e-6);
        assert!((pan(0.0, 10.0, 0.0) + 1.0).abs() < 1e-6);
        assert!(pan(0.0, 0.0, 10.0).abs() < 1e-6);
        // Facing -X (west, yaw 90): -Z is to the right.
        assert!((pan(90.0, 0.0, -10.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn sources_behind_are_quieter() {
        let front = spatial(&listener(0.0), at(0.0, 10.0), 48.0).unwrap().gain;
        let behind = spatial(&listener(0.0), at(0.0, -10.0), 48.0).unwrap().gain;
        assert!((behind - front * 0.7).abs() < 1e-6);
    }

    #[test]
    fn playback_needs_a_tracked_entity_in_range() {
        let mut state = state();
        state.world.players = vec![(uuid(1), at(10.0, 0.0)), (uuid(2), at(60.0, 0.0))];
        assert!(state.playback(uuid(1)).is_some());
        assert_eq!(state.playback(uuid(2)), None, "beyond range");
        assert_eq!(state.playback(uuid(3)), None, "no tracked entity");
    }

    #[test]
    fn the_shorter_range_applies_between_two_players() {
        // Alice's range is 20, Bob's 48, and they stand 25 apart.
        let mut alice = state();
        alice.voice_range = 20.0;
        alice.world.players = vec![(uuid(2), at(25.0, 0.0))];
        let mut bob = state();
        bob.world.players = vec![(uuid(1), at(25.0, 0.0))];

        // Alice doesn't send to Bob, nor play what Bob sends her.
        assert_eq!(alice.send_to(uuid(2), false).map(|c| c.distance), None);
        assert_eq!(alice.playback(uuid(2)), None);
        // Bob would send and play, but Alice's side stops both directions.
        assert!(bob.send_to(uuid(1), false).map(|c| c.distance).is_some());
        assert!(bob.playback(uuid(1)).is_some());
    }

    #[test]
    fn deafen_mute_and_spectator_silence_playback() {
        let mut state = state();
        state.world.players = vec![(uuid(1), at(1.0, 0.0)), (uuid(2), at(1.0, 0.0))];
        state.peer_audio.insert(
            uuid(1),
            PeerAudio {
                volume: 1.0,
                muted: true,
            },
        );
        assert_eq!(state.playback(uuid(1)), None);
        assert!(!state.wants_audio_from(uuid(1)));
        assert!(state.wants_audio_from(uuid(2)));

        state.input.deafened = true;
        assert_eq!(state.playback(uuid(2)), None);
        assert!(!state.wants_audio_from(uuid(2)));

        state.input.deafened = false;
        state.input.spectator = true;
        assert_eq!(state.playback(uuid(2)), None);
    }

    #[test]
    fn peer_volume_scales_gain() {
        let mut state = state();
        state.world.players = vec![(uuid(1), at(1.0, 0.0))];
        state.peer_audio.insert(
            uuid(1),
            PeerAudio {
                volume: 0.25,
                muted: false,
            },
        );
        assert_eq!(state.playback(uuid(1)).unwrap().gain, 0.25);
    }

    #[test]
    fn friends_only_limits_both_directions() {
        let mut state = state();
        state.world.players = vec![(uuid(1), at(1.0, 0.0)), (uuid(2), at(1.0, 0.0))];
        state.friends_only = Some(vec![uuid(1)]);
        assert!(state.playback(uuid(1)).is_some());
        assert_eq!(state.playback(uuid(2)), None);
        assert!(state.send_to(uuid(1), false).map(|c| c.distance).is_some());
        assert_eq!(state.send_to(uuid(2), false).map(|c| c.distance), None);
    }

    #[test]
    fn transmitting_follows_activation_mute_and_spectator() {
        let mut state = state();
        assert!(!state.transmitting(true), "push-to-talk not held");
        state.input.push_to_talk = true;
        assert!(state.transmitting(false));
        state.input.muted = true;
        assert!(!state.transmitting(false));
        state.input.muted = false;
        state.input.spectator = true;
        assert!(!state.transmitting(false), "spectators never send");

        state.input.spectator = false;
        state.input.push_to_talk = false;
        state.audio.activation = Activation::Voice;
        assert!(state.transmitting(true));
        assert!(!state.transmitting(false));
    }

    #[test]
    fn send_range_is_exactly_the_voice_range() {
        let mut state = state();
        state.world.players = vec![(uuid(1), at(48.0, 0.0)), (uuid(2), at(48.5, 0.0))];
        assert_eq!(
            state.send_to(uuid(1), false).map(|c| c.distance),
            Some(48.0)
        );
        assert_eq!(state.send_to(uuid(2), false).map(|c| c.distance), None);
        assert_eq!(
            state.send_to(uuid(3), false).map(|c| c.distance),
            None,
            "untracked"
        );
    }

    #[test]
    fn playback_glides_between_snapshots() {
        let mut state = state();
        let start = Instant::now();
        let tick = Duration::from_millis(50);
        let world = |x: f64| World {
            listener: listener(0.0),
            players: vec![(uuid(1), at(x, 0.0))],
        };
        state.set_world(world(10.0), start);
        state.set_world(world(20.0), start + tick);

        let x_at = |t: Duration| state.world_at(start + t).players[0].1.x;
        assert_eq!(x_at(tick), 10.0, "one snapshot behind");
        assert_eq!(x_at(tick + tick / 2), 15.0);
        assert_eq!(x_at(tick * 2), 20.0);
        assert_eq!(x_at(tick * 10), 20.0, "stops at the latest");

        // After a long gap there is nothing sensible to glide from.
        state.set_world(world(40.0), start + Duration::from_secs(5));
        assert_eq!(
            state.world_at(start + Duration::from_secs(5)).players[0]
                .1
                .x,
            40.0
        );
    }

    #[test]
    fn output_follows_the_game_device_unless_configured() {
        let mut state = state();
        assert_eq!(state.output_device(), DeviceChoice::Default);
        state.game_device = Some("OpenAL Soft on Headphones".into());
        assert_eq!(
            state.output_device(),
            DeviceChoice::ClosestTo("OpenAL Soft on Headphones".into())
        );
        state.audio.output_device = Some("Speakers".into());
        assert_eq!(state.output_device(), DeviceChoice::Id("Speakers".into()));
    }

    #[test]
    fn direct_calls_ignore_the_world() {
        let mut state = state();
        state.direct = true;
        assert_eq!(state.send_to(uuid(1), false).map(|c| c.distance), Some(0.0));
        assert_eq!(
            state.playback(uuid(1)),
            Some(Spatial {
                gain: 1.0,
                pan: 0.0
            })
        );
    }

    fn candidates(direct: usize, relayed: usize) -> Vec<Candidate> {
        (0..direct + relayed)
            .map(|i| Candidate {
                uuid: uuid(i as u8),
                distance: i as f64,
                relayed: i >= direct,
                group: false,
            })
            .collect()
    }

    #[test]
    fn nearest_32_are_sent_to() {
        let mut far_first = candidates(40, 0);
        far_first.reverse();
        let plan = plan_send(far_first, 24_000);
        assert_eq!(plan.recipients.len(), 32);
        assert_eq!(plan.recipients[0], uuid(0));
        assert!(!plan.recipients.contains(&uuid(32)));
    }

    #[test]
    fn crowds_step_the_bitrate_down() {
        assert_eq!(plan_send(candidates(16, 0), 24_000).bitrate, 24_000);
        assert_eq!(plan_send(candidates(17, 0), 24_000).bitrate, 16_000);
        // Never steps up.
        assert_eq!(plan_send(candidates(17, 0), 12_000).bitrate, 12_000);
    }

    #[test]
    fn relay_budget_fits_about_nine_then_eleven() {
        assert_eq!(relay_capacity(24_000), 9);
        assert_eq!(relay_capacity(16_000), 11);

        let plan = plan_send(candidates(2, 9), 24_000);
        assert_eq!(plan.bitrate, 24_000);
        assert!(plan.relay_full.is_empty());

        // Ten relayed don't fit at 24 kbps, so the encoder steps down.
        let plan = plan_send(candidates(2, 10), 24_000);
        assert_eq!(plan.bitrate, 16_000);
        assert_eq!(plan.recipients.len(), 12);

        // Beyond eleven, the farthest relayed peers get nothing.
        let plan = plan_send(candidates(2, 14), 24_000);
        assert_eq!(plan.recipients.len(), 13);
        assert_eq!(plan.relay_full, vec![uuid(13), uuid(14), uuid(15)]);
    }

    /// In a group with uuid(1); uuid(2) is a nearby player outside it.
    fn in_group(nearby: bool) -> VoiceState {
        let mut state = state();
        state.group = Some(OwnGroup {
            key: GroupKey::new("Miners", "").unwrap(),
            locked: false,
            nearby,
        });
        state.group_mates.insert(uuid(1));
        state.world.players = vec![(uuid(2), at(10.0, 0.0))];
        state
    }

    #[test]
    fn group_mates_are_heard_anywhere_and_unpositioned() {
        let mut state = in_group(true);
        let centred = Some(Spatial {
            gain: 1.0,
            pan: 0.0,
        });
        // uuid(1) has no tracked entity, so only the group reaches it.
        assert_eq!(state.group_playback(uuid(1)), centred);
        assert_eq!(state.playback(uuid(1)), None);
        let mate = state.send_to(uuid(1), false).unwrap();
        assert!(mate.group);
        // Nearby players outside the group are heard as usual, but their
        // group frames are not.
        assert!(state.playback(uuid(2)).is_some());
        assert_eq!(state.group_playback(uuid(2)), None);
        assert!(!state.send_to(uuid(2), false).unwrap().group);
        // Spectating doesn't stop group voice.
        state.input.spectator = true;
        state.input.push_to_talk = true;
        assert!(state.transmitting(false));
        assert_eq!(state.group_playback(uuid(1)), centred);
        assert_eq!(state.send_to(uuid(2), false), None);
        // Mute still applies.
        state.peer_audio.insert(
            uuid(1),
            PeerAudio {
                volume: 1.0,
                muted: true,
            },
        );
        assert_eq!(state.group_playback(uuid(1)), None);
    }

    #[test]
    fn group_only_cuts_nearby_players_off_both_ways() {
        let state = in_group(false);
        assert!(state.group_playback(uuid(1)).is_some());
        assert_eq!(state.playback(uuid(2)), None);
        assert_eq!(state.send_to(uuid(2), false), None);
        assert!(!state.wants_audio_from(uuid(2)));
        assert!(state.wants_audio_from(uuid(1)));
    }

    #[test]
    fn group_mates_are_sent_to_first() {
        let mut all = candidates(40, 0);
        all[39].group = true;
        let plan = plan_send(all, 24_000);
        assert_eq!(plan.recipients[0], uuid(39));
        assert_eq!(plan.recipients.len(), 32);
    }
}
