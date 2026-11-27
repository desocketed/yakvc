//! Who hears whom, and how loud: the rules from DESIGN.md "Voice transport
//! and audio pipeline", as plain functions over a snapshot of game state.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use yakvc_audio::{DeviceChoice, ECHO_TAPS, EchoTap, Spatial};
use yakvc_proto::{GroupId, Uuid};

use super::group::{Invite, OwnGroup, PendingGroup, SentInvite};
use crate::config::{AcousticsConfig, Activation, AudioConfig, InvitesFrom};
use crate::engine::PeerAudio;
use crate::world::{Input, Player, Pose, Room, Vec3, World, distance, interpolate};

/// Snapshots further apart than this are not interpolated: the game paused
/// or lagged, so gliding would only move voices late.
const MAX_SNAPSHOT_GAP: Duration = Duration::from_millis(250);

/// How long a change in occlusion takes to be heard in full, so a voice
/// fades behind a wall or back in rather than cutting.
const OCCLUSION_RAMP: Duration = Duration::from_millis(400);
/// Directly behind the listener a speaker is this much quieter.
const BEHIND_ATTENUATION: f64 = 0.3;
/// Seconds of decay below which a space sounds dry: houses, small rooms,
/// corridors.
const DRY_DECAY: f32 = 0.6;
/// Seconds of decay from which a space rings in full: big caves and halls.
const FULL_DECAY: f32 = 5.0;
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
    pub friends: Vec<Uuid>,
    pub friends_only: bool,
    pub invites_from: InvitesFrom,
    pub audio: AudioConfig,
    pub acoustics: AcousticsConfig,
    /// The game's selected sound device, which output follows unless
    /// `audio.output_device` is set.
    pub game_device: Option<String>,
    /// A call with no game (`yakvc-cli call`): every peer is heard at full
    /// volume and sent to, whatever the world says.
    pub ignore_world: bool,
    /// The group we are in. While in one, voice is group-only.
    pub group: Option<OwnGroup>,
    /// Peers whose announcement proves they are in our group.
    pub group_mates: HashSet<Uuid>,
    /// The group `group_mates` was worked out for, so the mates found right
    /// after we join aren't reported as joining.
    pub mates_of: Option<GroupId>,
    /// The group we invited players to while in none, which we join once
    /// one of them does.
    pub pending: Option<PendingGroup>,
    /// The latest invite we sent each player, for its peer task to send.
    pub invites_sent: HashMap<Uuid, SentInvite>,
    /// The latest invite from each player.
    pub invites: HashMap<Uuid, Invite>,
    /// When we asked each player to invite us, for its peer task to send.
    pub requests_sent: HashMap<Uuid, Instant>,
    /// When each player asked us to invite them.
    pub requests: HashMap<Uuid, Instant>,
}

/// Who a frame is heard from, before any panning or acoustics.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Heard {
    /// Not positional: group voice from a mate who isn't tracked, or a
    /// direct call.
    Centred,
    /// Group voice from a tracked mate: from the direction of this eye
    /// position, at full volume whatever the distance.
    Directional(Vec3),
    /// From this player's place in the world.
    Positioned(Player),
}

/// What [`VoiceState::heard`] decided for one frame: the speakers and a
/// voice sink both play exactly the frames it lets through.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Playback {
    pub heard: Heard,
    /// The player's volume setting.
    pub volume: f32,
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
    pub(crate) fn new(audio: AudioConfig, voice_range: f32) -> Self {
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
            friends: Vec::new(),
            friends_only: false,
            invites_from: InvitesFrom::Everyone,
            audio,
            acoustics: AcousticsConfig::default(),
            game_device: None,
            ignore_world: false,
            group: None,
            group_mates: HashSet::new(),
            mates_of: None,
            pending: None,
            invites_sent: HashMap::new(),
            requests_sent: HashMap::new(),
            requests: HashMap::new(),
            invites: HashMap::new(),
        }
    }

    pub(crate) fn set_world(&mut self, world: World, now: Instant) {
        // A ramp in progress carries on from what is heard now, not from
        // where the last snapshot said it was going.
        let heard = self.world_at(now);
        self.previous_world = std::mem::replace(&mut self.world, world);
        for player in &mut self.previous_world.players {
            if let Some(h) = heard.players.iter().find(|p| p.uuid == player.uuid) {
                player.occlusion = h.occlusion;
            }
        }
        self.previous_at = std::mem::replace(&mut self.world_at, now);
    }

    /// The world as playback should hear it at `now`: one snapshot behind,
    /// moving smoothly from the previous snapshot to the latest, so gain and
    /// pan don't step at every 20 Hz tick.
    pub(crate) fn world_at(&self, now: Instant) -> World {
        let interval = self.world_at.saturating_duration_since(self.previous_at);
        let mut world = if interval.is_zero() || interval > MAX_SNAPSHOT_GAP {
            self.world.clone()
        } else {
            let since = now.saturating_duration_since(self.world_at);
            let t = since.as_secs_f64() / interval.as_secs_f64();
            interpolate(&self.previous_world, &self.world, t.min(1.0))
        };
        // Occlusion ramps on its own, slower clock.
        let since = now.saturating_duration_since(self.world_at);
        let t = (since.as_secs_f32() / OCCLUSION_RAMP.as_secs_f32()).min(1.0);
        for player in &mut world.players {
            let latest = self.world.players.iter().find(|p| p.uuid == player.uuid);
            let before = self
                .previous_world
                .players
                .iter()
                .find(|p| p.uuid == player.uuid);
            if let (Some(latest), Some(before)) = (latest, before) {
                player.occlusion = before.occlusion + (latest.occlusion - before.occlusion) * t;
            }
        }
        world
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
    /// `ReceiveState` so it can stop sending when we don't. In a group,
    /// only group mates are heard.
    pub(crate) fn wants_audio_from(&self, peer: Uuid) -> bool {
        if self.input.deafened || self.muted(peer) || !self.is_friend(peer) {
            return false;
        }
        self.group.is_none() || self.is_group_mate(peer)
    }

    /// How the speakers play a frame from `peer`, with or without the
    /// `group` flag, or `None` to discard it: [`Self::heard`], then
    /// [`Self::through_speakers`], as the audio thread does.
    /// `world` is the one playback should hear, usually [`Self::world_at`]
    /// for now, which lags the latest snapshot a little.
    #[cfg(test)]
    pub(crate) fn playback(&self, world: &World, peer: Uuid, group: bool) -> Option<Spatial> {
        self.through_speakers(world, self.heard(world, peer, group)?)
    }

    /// Whether a frame from `peer`, with or without the `group` flag, is
    /// heard at all, and from where: the rules the speakers and a voice sink
    /// both follow. `None` discards it.
    pub(crate) fn heard(&self, world: &World, peer: Uuid, group: bool) -> Option<Playback> {
        if !self.wants_audio_from(peer) {
            return None;
        }
        let heard = if group {
            // Group voice has direction but no falloff, so range and
            // spectating don't matter; it plays only from a group mate.
            if !self.is_group_mate(peer) {
                return None;
            }
            self.group_direction(world, peer)
        } else {
            self.nearby(world, peer)?
        };
        let volume = self.peer_audio.get(&peer).map_or(1.0, |audio| audio.volume);
        Some(Playback { heard, volume })
    }

    /// Which way group voice from `peer` comes: from the mate when they are
    /// tracked, else centred.
    fn group_direction(&self, world: &World, peer: Uuid) -> Heard {
        match world.players.iter().find(|p| p.uuid == peer) {
            Some(player) if !self.ignore_world => Heard::Directional(player.pos),
            _ => Heard::Centred,
        }
    }

    /// Where a positional frame from `peer` is heard from. In a group,
    /// nowhere.
    fn nearby(&self, world: &World, peer: Uuid) -> Option<Heard> {
        if self.input.spectator || self.group.is_some() {
            return None;
        }
        if self.ignore_world {
            return Some(Heard::Centred);
        }
        let player = world.players.iter().find(|p| p.uuid == peer)?;
        // Range counts the way the voice comes: one that has to go the long
        // way round can be out of range when its speaker isn't.
        let reach = distance(world.listener.pos, player.heard_from);
        (reach <= f64::from(self.voice_range)).then_some(Heard::Positioned(*player))
    }

    /// How the speakers play what [`Self::heard`] let through: panned,
    /// quieter with distance and occlusion, muffled, and sent to the room's
    /// reverb.
    pub(crate) fn through_speakers(&self, world: &World, playback: Playback) -> Option<Spatial> {
        let spatial = match playback.heard {
            Heard::Centred => CENTRED,
            Heard::Directional(source) => {
                let (dx, dz) = (
                    source.x - world.listener.pos.x,
                    source.z - world.listener.pos.z,
                );
                let (pan, _) = pan_and_front(&world.listener, dx, dz);
                Spatial {
                    pan: pan as f32,
                    ..CENTRED
                }
            }
            Heard::Positioned(player) => {
                let spatial = spatial(
                    &world.listener,
                    player.heard_from,
                    self.voice_range,
                    &self.acoustics,
                )?;
                let heard = distance(world.listener.pos, player.heard_from);
                Spatial {
                    reverb: reverb_send(world.room, heard),
                    ..occluded(spatial, player.occlusion, &self.acoustics)
                }
            }
        };
        Some(Spatial {
            gain: spatial.gain * playback.volume,
            ..spatial
        })
    }

    /// Whether to send this frame to `peer`, and how. Whether the peer
    /// wants our audio is up to the peer and checked separately.
    pub(crate) fn send_to(&self, peer: Uuid, relayed: bool) -> Option<Candidate> {
        let group = self.group.is_some();
        let distance = if group {
            // In a group we talk to the group alone, wherever it is.
            self.is_group_mate(peer).then_some(0.0)?
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
        if !self.is_friend(peer) || self.input.spectator {
            return None;
        }
        if self.ignore_world {
            return Some(0.0);
        }
        let distance = distance(self.world.listener.pos, self.position(peer)?);
        (distance <= f64::from(self.voice_range)).then_some(distance)
    }

    fn position(&self, peer: Uuid) -> Option<Vec3> {
        let player = self.world.players.iter().find(|p| p.uuid == peer)?;
        Some(player.pos)
    }

    pub(crate) fn muted(&self, peer: Uuid) -> bool {
        self.peer_audio.get(&peer).is_some_and(|audio| audio.muted)
    }

    /// Whether voice may pass between us and `peer`: everyone, unless only
    /// friends may.
    pub(crate) fn is_friend(&self, peer: Uuid) -> bool {
        !self.friends_only || self.friends.contains(&peer)
    }
}

const CENTRED: Spatial = Spatial {
    gain: 1.0,
    pan: 0.0,
    muffle: 0.0,
    reverb: 0.0,
    echo: 0.0,
};

/// Gain and pan for a source at `source`, heard by `listener` with voice
/// range `range`. `None` beyond range.
///
/// Gain is 1 within `full_volume_distance` and falls to 0 at the range,
/// curved by `distance_rolloff`; sources behind the listener are a little
/// quieter. Pan is the sideways part of the direction to the source, so
/// equal-power panning in the mixer puts a source at 90° fully to one side.
pub(crate) fn spatial(
    listener: &Pose,
    source: Vec3,
    range: f32,
    acoustics: &AcousticsConfig,
) -> Option<Spatial> {
    let range = f64::from(range);
    let full = f64::from(acoustics.full_volume_distance).min(range);
    let distance = distance(listener.pos, source);
    if distance > range {
        return None;
    }
    let mut gain = if distance <= full {
        1.0
    } else {
        ((range - distance) / (range - full)).powf(f64::from(acoustics.distance_rolloff))
    };

    let (dx, dz) = (source.x - listener.pos.x, source.z - listener.pos.z);
    let (pan, front) = pan_and_front(listener, dx, dz);
    if front < 0.0 {
        gain *= 1.0 + BEHIND_ATTENUATION * front;
    }
    Some(Spatial {
        gain: gain as f32,
        pan: pan as f32,
        muffle: 0.0,
        reverb: 0.0,
        echo: 1.0,
    })
}

/// The sideways (pan, -1 left to 1 right) and forward parts of the
/// horizontal direction `dx, dz` from `listener`.
fn pan_and_front(listener: &Pose, dx: f64, dz: f64) -> (f64, f64) {
    // Minecraft yaw: 0 faces +Z and grows clockwise seen from above, so
    // facing +Z the listener's right hand points to -X.
    let yaw = f64::from(listener.yaw).to_radians();
    let (forward_x, forward_z) = (-yaw.sin(), yaw.cos());
    let (right_x, right_z) = (-yaw.cos(), -yaw.sin());
    let horizontal = dx.hypot(dz);
    if horizontal < 1e-6 {
        return (0.0, 1.0);
    }
    (
        (dx * right_x + dz * right_z) / horizontal,
        (dx * forward_x + dz * forward_z) / horizontal,
    )
}

/// The room's far echoes as the mixer plays them: each panned to where its
/// wall is from the way the listener faces now, scaled by `level`.
pub(crate) fn echo_taps(world: &World, level: f32) -> [EchoTap; ECHO_TAPS] {
    world.room.far_echoes.map(|echo| {
        let (pan, _) = pan_and_front(&world.listener, echo.direction.x, echo.direction.z);
        EchoTap {
            delay: echo.delay,
            gain: echo.gain * level,
            pan: pan as f32,
            cutoff_hz: echo.cutoff_hz,
        }
    })
}

/// How much of a voice `distance` metres away goes to the reverb. A room's
/// reverberant sound is about as loud everywhere in it, while the direct
/// sound fades with distance; the two are equal at the critical distance.
/// So the reverb's share of what reaches the listener, in energy, is
/// `d² / (d² + r_c²)`: a nearby talker is mostly dry, a far one mostly room.
/// The send is its square root, an amplitude.
///
/// Only big, live spaces should be heard ringing, though: in a house or a
/// small stone room the ear takes the reverb as part of the voice. So the
/// send also grows with the decay, from nothing below [`DRY_DECAY`] to all
/// of it at [`FULL_DECAY`], smoothly between.
fn reverb_send(room: Room, distance: f64) -> f32 {
    let critical = f64::from(room.critical_distance).max(0.1);
    let share = distance / distance.hypot(critical);
    let size = ((room.decay - DRY_DECAY) / (FULL_DECAY - DRY_DECAY)).clamp(0.0, 1.0);
    let size = size * size * (3.0 - 2.0 * size);
    room.reverb * size * share as f32
}

/// `spatial` heard through obstacles that block `occlusion` of the direct
/// path (0 to 1): quieter and muffled.
fn occluded(spatial: Spatial, occlusion: f32, acoustics: &AcousticsConfig) -> Spatial {
    let occlusion = occlusion.clamp(0.0, 1.0);
    Spatial {
        gain: spatial.gain * (1.0 - acoustics.occlusion_attenuation * occlusion),
        muffle: occlusion,
        ..spatial
    }
}

/// Picks recipients nearest first, steps the bitrate down for big crowds,
/// and keeps relayed recipients within the relay budget (DESIGN.md
/// "Connection lifecycle").
pub(crate) fn plan_send(mut candidates: Vec<Candidate>, bitrate: u32) -> SendPlan {
    candidates.sort_by(|a, b| a.distance.total_cmp(&b.distance));
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
    use yakvc_proto::{GroupKey, GroupState};

    use super::*;
    use crate::world::FarEcho;

    fn uuid(n: u8) -> Uuid {
        Uuid::from_bytes([n; 16])
    }

    fn state() -> VoiceState {
        let mut state = VoiceState::new(AudioConfig::default(), 48.0);
        state.world.listener = listener(0.0);
        state
    }

    /// `x, z` from the listener, who stands away from the world's origin so
    /// that positions are really subtracted.
    fn at(x: f64, z: f64) -> Vec3 {
        Vec3::new(x + 120.0, 64.0, z - 75.0)
    }

    fn listener(yaw: f32) -> Pose {
        Pose {
            pos: at(0.0, 0.0),
            yaw,
            pitch: 0.0,
        }
    }

    fn nearby(state: &VoiceState, peer: u8) -> Option<Spatial> {
        state.playback(&state.world, uuid(peer), false)
    }

    #[test]
    fn gain_is_full_up_close_and_fades_to_zero_at_range() {
        let linear = AcousticsConfig {
            distance_rolloff: 1.0,
            ..AcousticsConfig::default()
        };
        let gain = |d: f64| spatial(&listener(0.0), at(0.0, d), 48.0, &linear).map(|s| s.gain);
        assert_eq!(gain(0.0), Some(1.0));
        assert_eq!(gain(4.0), Some(1.0));
        assert!((gain(26.0).unwrap() - 0.5).abs() < 1e-6);
        assert_eq!(gain(48.0), Some(0.0));
        assert_eq!(gain(48.1), None);

        // The default, steeper rolloff drops distant voices sooner.
        let steep = spatial(
            &listener(0.0),
            at(0.0, 40.0),
            48.0,
            &AcousticsConfig::default(),
        );
        assert!(steep.unwrap().gain < gain(40.0).unwrap() * 0.5);
    }

    #[test]
    fn pan_follows_yaw() {
        let pan = |yaw: f32, x: f64, z: f64| {
            spatial(&listener(yaw), at(x, z), 48.0, &AcousticsConfig::default())
                .unwrap()
                .pan
        };
        // Facing +Z (south): -X is to the right.
        assert!((pan(0.0, -10.0, 0.0) - 1.0).abs() < 1e-6);
        assert!((pan(0.0, 10.0, 0.0) + 1.0).abs() < 1e-6);
        assert!(pan(0.0, 0.0, 10.0).abs() < 1e-6);
        // Facing -X (west, yaw 90): -Z is to the right.
        assert!((pan(90.0, 0.0, -10.0) - 1.0).abs() < 1e-6);
        // Facing south-west (yaw 45), a source due west is ahead and right.
        let right_ahead = pan(45.0, -10.0, 0.0);
        assert!(right_ahead > 0.0 && right_ahead < 1.0, "{right_ahead}");
        assert!((pan(45.0, 0.0, 10.0) + right_ahead).abs() < 1e-6);
    }

    #[test]
    fn sources_behind_are_quieter() {
        let gain = |yaw: f32, x: f64, z: f64| {
            spatial(&listener(yaw), at(x, z), 48.0, &AcousticsConfig::default())
                .unwrap()
                .gain
        };
        // Facing south, ten blocks away in every direction.
        let front = gain(0.0, 0.0, 10.0);
        let side = gain(0.0, 10.0, 0.0);
        let diagonal_behind = gain(0.0, 7.071_067_8, -7.071_067_8);
        let behind = gain(0.0, 0.0, -10.0);
        assert_eq!(side, front);
        assert!(front > diagonal_behind && diagonal_behind > behind);
        // Turning turns what counts as behind.
        assert!((gain(90.0, 10.0, 0.0) - behind).abs() < 1e-6);
        assert!((gain(45.0, 7.071_067_8, -7.071_067_8) - behind).abs() < 1e-6);
        assert!((gain(45.0, -7.071_067_8, 7.071_067_8) - front).abs() < 1e-6);
    }

    #[test]
    fn playback_needs_a_tracked_entity_in_range() {
        let mut state = state();
        state.world.players = vec![
            Player::new(uuid(1), at(10.0, 0.0)),
            Player::new(uuid(2), at(60.0, 0.0)),
        ];
        assert!(nearby(&state, 1).is_some());
        assert_eq!(nearby(&state, 2), None, "beyond range");
        assert_eq!(nearby(&state, 3), None, "no tracked entity");
    }

    #[test]
    fn the_shorter_range_applies_between_two_players() {
        // Alice's range is 20, Bob's 48, and they stand 25 apart.
        let mut alice = state();
        alice.voice_range = 20.0;
        alice.world.players = vec![Player::new(uuid(2), at(25.0, 0.0))];
        let mut bob = state();
        bob.world.players = vec![Player::new(uuid(1), at(25.0, 0.0))];

        // Alice doesn't send to Bob, nor play what Bob sends her.
        assert_eq!(alice.send_to(uuid(2), false).map(|c| c.distance), None);
        assert_eq!(nearby(&alice, 2), None);
        // Bob would send and play, but Alice's side stops both directions.
        assert!(bob.send_to(uuid(1), false).map(|c| c.distance).is_some());
        assert!(nearby(&bob, 1).is_some());
    }

    #[test]
    fn deafen_mute_and_spectator_silence_playback() {
        let mut state = state();
        state.world.players = vec![
            Player::new(uuid(1), at(1.0, 0.0)),
            Player::new(uuid(2), at(1.0, 0.0)),
        ];
        state.peer_audio.insert(
            uuid(1),
            PeerAudio {
                volume: 1.0,
                muted: true,
            },
        );
        assert_eq!(nearby(&state, 1), None);
        assert!(!state.wants_audio_from(uuid(1)));
        assert!(state.wants_audio_from(uuid(2)));

        state.input.deafened = true;
        assert_eq!(nearby(&state, 2), None);
        assert!(!state.wants_audio_from(uuid(2)));

        state.input.deafened = false;
        state.input.spectator = true;
        assert_eq!(nearby(&state, 2), None);
    }

    #[test]
    fn peer_volume_scales_gain() {
        let mut state = state();
        state.world.players = vec![Player::new(uuid(1), at(1.0, 0.0))];
        state.peer_audio.insert(
            uuid(1),
            PeerAudio {
                volume: 0.25,
                muted: false,
            },
        );
        assert_eq!(nearby(&state, 1).unwrap().gain, 0.25);
    }

    #[test]
    fn friends_only_limits_both_directions() {
        let mut state = state();
        state.world.players = vec![
            Player::new(uuid(1), at(1.0, 0.0)),
            Player::new(uuid(2), at(1.0, 0.0)),
        ];
        state.friends = vec![uuid(1)];
        state.friends_only = true;
        assert!(nearby(&state, 1).is_some());
        assert_eq!(nearby(&state, 2), None);
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
        state.world.players = vec![
            Player::new(uuid(1), at(48.0, 0.0)),
            Player::new(uuid(2), at(48.5, 0.0)),
        ];
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
    fn far_echoes_follow_the_listener_turning() {
        let mut world = World::default();
        // A cliff to the west (-X).
        world.room.far_echoes[0] = FarEcho {
            direction: Vec3::new(-1.0, 0.0, 0.0),
            delay: 0.3,
            gain: 0.2,
            cutoff_hz: 4000.0,
        };
        // Facing +Z, -X is on the right; turned round, on the left.
        world.listener = listener(0.0);
        let taps = echo_taps(&world, 0.5);
        assert!((taps[0].pan - 1.0).abs() < 1e-6);
        assert!((taps[0].gain - 0.1).abs() < 1e-6);
        assert_eq!((taps[0].delay, taps[0].cutoff_hz), (0.3, 4000.0));
        assert_eq!(taps[1].gain, 0.0);
        world.listener = listener(180.0);
        assert!((echo_taps(&world, 0.5)[0].pan + 1.0).abs() < 1e-6);
    }

    #[test]
    fn far_voices_send_more_to_the_reverb() {
        let mut state = state();
        state.world.room = Room {
            reverb: 0.8,
            decay: 5.0,
            critical_distance: 2.0,
            ..Room::default()
        };
        state.world.players = vec![
            Player::new(uuid(1), at(1.0, 0.0)),
            Player::new(uuid(2), at(2.0, 0.0)),
            Player::new(uuid(3), at(20.0, 0.0)),
        ];
        let send = |state: &VoiceState, peer| nearby(state, peer).unwrap().reverb;
        assert!(send(&state, 1) < 0.4, "{}", send(&state, 1));
        // At the critical distance reverb and voice are equally loud.
        let equal = 0.8 * std::f32::consts::FRAC_1_SQRT_2;
        assert!((send(&state, 2) - equal).abs() < 1e-3);
        assert!(send(&state, 3) > 0.79, "{}", send(&state, 3));

        // A small room barely rings, however far the talker.
        state.world.room.decay = 0.9;
        assert!(send(&state, 3) < 0.05, "{}", send(&state, 3));

        state.world.room.reverb = 0.0;
        assert_eq!(send(&state, 3), 0.0);
    }

    #[test]
    fn occluded_speakers_are_quieter_and_muffled() {
        let mut state = state();
        // Off by default; tuning can turn it on.
        state.acoustics.occlusion_attenuation = 0.75;
        state.world.players = vec![
            Player::new(uuid(1), at(10.0, 0.0)),
            Player {
                occlusion: 1.0,
                ..Player::new(uuid(2), at(10.0, 0.0))
            },
        ];
        let (open, walled) = (nearby(&state, 1).unwrap(), nearby(&state, 2).unwrap());
        assert_eq!(open.muffle, 0.0);
        assert_eq!(walled.muffle, 1.0);
        assert!(walled.gain < open.gain);
        assert_eq!(walled.pan, open.pan);
    }

    #[test]
    fn voices_come_from_where_they_are_heard() {
        let mut state = state();
        // Bob is in front, but his voice comes round a wall from the right.
        state.world.players = vec![Player {
            heard_from: at(-10.0, 0.0),
            ..Player::new(uuid(1), at(0.0, 10.0))
        }];
        assert!(nearby(&state, 1).unwrap().pan > 0.9);
    }

    #[test]
    fn playback_glides_between_snapshots() {
        let mut state = state();
        let start = Instant::now();
        let tick = Duration::from_millis(50);
        let world = |x: f64| World {
            listener: listener(0.0),
            players: vec![Player::new(uuid(1), at(x, 0.0))],
            ..World::default()
        };
        state.set_world(world(10.0), start);
        state.set_world(world(20.0), start + tick);

        let x_at = |t: Duration| state.world_at(start + t).players[0].pos.x - at(0.0, 0.0).x;
        assert_eq!(x_at(tick), 10.0, "one snapshot behind");
        assert_eq!(x_at(tick + tick / 2), 15.0);
        assert_eq!(x_at(tick * 2), 20.0);
        assert_eq!(x_at(tick * 10), 20.0, "stops at the latest");

        // After a long gap there is nothing sensible to glide from.
        state.set_world(world(40.0), start + Duration::from_secs(5));
        assert_eq!(
            state.world_at(start + Duration::from_secs(5)).players[0].pos,
            at(40.0, 0.0)
        );
    }

    #[test]
    fn occlusion_ramps_over_time() {
        let mut state = state();
        let start = Instant::now();
        let world = |occlusion: f32| World {
            listener: listener(0.0),
            players: vec![Player {
                occlusion,
                ..Player::new(uuid(1), at(5.0, 0.0))
            }],
            ..World::default()
        };
        let heard =
            |state: &VoiceState, t: Duration| state.world_at(start + t).players[0].occlusion;
        state.set_world(world(0.0), start);
        state.set_world(world(1.0), start);
        let half = OCCLUSION_RAMP / 2;
        assert_eq!(heard(&state, Duration::ZERO), 0.0);
        assert!((heard(&state, half) - 0.5).abs() < 1e-6);
        assert_eq!(heard(&state, OCCLUSION_RAMP * 2), 1.0);

        // Open again halfway up: falls from where it was heard.
        state.set_world(world(0.0), start + half);
        assert!((heard(&state, half) - 0.5).abs() < 1e-6);
        assert!((heard(&state, half + half) - 0.25).abs() < 1e-6);
        assert_eq!(heard(&state, half + OCCLUSION_RAMP), 0.0);
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
    fn calls_ignore_the_world() {
        let mut state = state();
        state.ignore_world = true;
        assert_eq!(state.send_to(uuid(1), false).map(|c| c.distance), Some(0.0));
        assert_eq!(nearby(&state, 1), Some(CENTRED));
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
    fn in_group() -> VoiceState {
        let mut state = state();
        let group = OwnGroup::new(GroupKey::random(), GroupState::new(uuid(9)));
        state.set_group(Some(group));
        state.group_mates.insert(uuid(1));
        state.world.players = vec![Player::new(uuid(2), at(10.0, 0.0))];
        state
    }

    #[test]
    fn group_mates_are_heard_anywhere_and_unpositioned() {
        let mut state = in_group();
        // uuid(1) has no tracked entity, so only the group reaches it.
        assert_eq!(state.playback(&state.world, uuid(1), true), Some(CENTRED));
        assert_eq!(nearby(&state, 1), None);
        let mate = state.send_to(uuid(1), false).unwrap();
        assert!(mate.group);
        assert_eq!(mate.distance, 0.0);
        // Spectating doesn't stop group voice.
        state.input.spectator = true;
        state.input.push_to_talk = true;
        assert!(state.transmitting(false));
        assert_eq!(state.playback(&state.world, uuid(1), true), Some(CENTRED));
        // Mute still applies.
        state.peer_audio.insert(
            uuid(1),
            PeerAudio {
                volume: 1.0,
                muted: true,
            },
        );
        assert_eq!(state.playback(&state.world, uuid(1), true), None);
    }

    #[test]
    fn tracked_group_mates_have_direction_but_no_falloff() {
        let mut state = in_group();
        state.group_mates.insert(uuid(3));
        state.world.players = vec![
            // Near, to the left (facing +Z, left is +X).
            Player::new(uuid(1), at(2.0, 0.0)),
            // Far beyond the voice range, to the right, behind a wall.
            Player {
                occlusion: 1.0,
                ..Player::new(uuid(3), at(-500.0, 0.0))
            },
        ];
        let near = state.playback(&state.world, uuid(1), true).unwrap();
        let far = state.playback(&state.world, uuid(3), true).unwrap();
        assert_eq!(near.gain, 1.0);
        assert_eq!(far.gain, 1.0);
        assert!(near.pan < -0.99 && far.pan > 0.99, "{near:?} {far:?}");
        // Dry: no muffle, reverb or echo.
        assert_eq!(
            far,
            Spatial {
                pan: far.pan,
                ..CENTRED
            }
        );
        // Behind the listener: no gain cut either.
        state.world.players[0].pos = at(0.0, -20.0);
        let behind = state.playback(&state.world, uuid(1), true).unwrap();
        assert_eq!(behind.gain, 1.0);
        // The per-player volume still applies.
        state.peer_audio.insert(
            uuid(1),
            PeerAudio {
                volume: 0.5,
                muted: false,
            },
        );
        assert_eq!(
            state.playback(&state.world, uuid(1), true).unwrap().gain,
            0.5
        );
    }

    #[test]
    fn positional_voice_still_falls_off() {
        let mut state = state();
        state.world.players = vec![
            Player::new(uuid(1), at(2.0, 0.0)),
            Player::new(uuid(2), at(40.0, 0.0)),
        ];
        let near = nearby(&state, 1).unwrap();
        let far = nearby(&state, 2).unwrap();
        assert_eq!(near.gain, 1.0);
        assert!(far.gain < 0.5, "{far:?}");
    }

    #[test]
    fn a_group_cuts_nearby_players_off_both_ways() {
        let state = in_group();
        assert_eq!(nearby(&state, 2), None);
        assert_eq!(state.playback(&state.world, uuid(2), true), None);
        assert_eq!(state.send_to(uuid(2), false), None);
        assert!(!state.wants_audio_from(uuid(2)));
        assert!(state.wants_audio_from(uuid(1)));
        // Without the group, uuid(2) is an ordinary nearby player.
        let mut state = state;
        state.set_group(None);
        assert!(nearby(&state, 2).is_some());
        assert!(!state.send_to(uuid(2), false).unwrap().group);
    }

    #[test]
    fn the_speakers_and_a_sink_hear_the_same_frames() {
        let mut state = state();
        state.world.players = vec![
            Player::new(uuid(1), at(10.0, 0.0)),
            Player::new(uuid(2), at(60.0, 0.0)),
            Player {
                occlusion: 1.0,
                ..Player::new(uuid(3), at(5.0, 0.0))
            },
            // In range, but the way round to us is too long.
            Player {
                heard_from: at(0.0, 50.0),
                ..Player::new(uuid(4), at(5.0, 0.0))
            },
        ];
        state.peer_audio.insert(
            uuid(1),
            PeerAudio {
                volume: 0.5,
                muted: false,
            },
        );
        for peer in 1..=5 {
            let heard = state.heard(&state.world, uuid(peer), false);
            assert_eq!(
                heard.is_some(),
                nearby(&state, peer).is_some(),
                "player {peer}"
            );
        }
        // A sink gets the volume setting alone: distance and occlusion are
        // its own to apply.
        let heard = state.heard(&state.world, uuid(1), false).unwrap();
        assert_eq!(heard.volume, 0.5);
        assert_eq!(heard.heard, Heard::Positioned(state.world.players[0]));
        let walled = state.heard(&state.world, uuid(3), false).unwrap();
        assert_eq!(walled.volume, 1.0);
    }

    #[test]
    fn a_sink_hears_group_mates_and_calls_centred() {
        let state = in_group();
        let mate = state.heard(&state.world, uuid(1), true).unwrap();
        assert_eq!(mate.heard, Heard::Centred);
        let mut tracked = in_group();
        tracked.group_mates.insert(uuid(2));
        let eye = tracked.world.players[0].pos;
        let mate = tracked.heard(&tracked.world, uuid(2), true).unwrap();
        assert_eq!(mate.heard, Heard::Directional(eye));
        assert_eq!(state.heard(&state.world, uuid(2), true), None);

        let mut call = self::state();
        call.ignore_world = true;
        let peer = call.heard(&call.world, uuid(1), false).unwrap();
        assert_eq!(peer.heard, Heard::Centred);
    }

    /// The echo levels `Room` and `FarEchoes` give the test world's spaces.
    const HALL_ECHO: f32 = 0.045;
    const CAVERN_ECHO: f32 = 0.07;
    const CANYON_ECHO: f32 = 0.048;

    /// How loud a room's reverb and echoes are next to a voice `distance`
    /// metres away, in dB: a voice-like signal (a 150 Hz buzz spoken in
    /// syllables) mixed once dry and once in the room, the wet part being
    /// the difference, over 2 s of speech and the 4 s of tail after it.
    fn wet_db(room: Room, distance: f64) -> f32 {
        use yakvc_audio::{FRAME_SAMPLES, Mixer, SAMPLE_RATE};
        let config = AcousticsConfig::default();
        let world = World {
            room,
            ..World::default()
        };
        let mut wet_mixer = Mixer::new();
        wet_mixer.set_acoustics(yakvc_audio::Acoustics {
            decay: room.decay,
            echo: room.echo,
            echo_delay: room.echo_delay,
            echo_feedback: config.echo_feedback,
            reverb_level: config.reverb_level,
            reverb_damping: config.reverb_damping,
            taps: echo_taps(&world, config.echo_level),
        });
        let mut dry_mixer = Mixer::new();
        let wet = Spatial {
            gain: 1.0,
            pan: 0.0,
            muffle: 0.0,
            reverb: reverb_send(room, distance),
            echo: 1.0,
        };
        let dry = Spatial {
            reverb: 0.0,
            echo: 0.0,
            ..wet
        };
        let (mut dry_energy, mut wet_energy) = (0.0, 0.0);
        for frame in 0..300 {
            let mut voice = [0.0; FRAME_SAMPLES];
            for (i, sample) in voice.iter_mut().enumerate() {
                let t = (frame * FRAME_SAMPLES + i) as f32 / SAMPLE_RATE as f32;
                if t < 2.0 {
                    let syllable = (std::f32::consts::PI * 4.0 * t).sin().max(0.0);
                    let buzz: f32 = (1..=8)
                        .map(|h| (std::f32::consts::TAU * 150.0 * h as f32 * t).sin() / h as f32)
                        .sum();
                    *sample = 0.02 * syllable * buzz;
                }
            }
            let mut in_room = [[0.0; 2]; FRAME_SAMPLES];
            let mut plain = [[0.0; 2]; FRAME_SAMPLES];
            wet_mixer.mix([(&voice, wet)], &mut in_room);
            dry_mixer.mix([(&voice, dry)], &mut plain);
            for (w, d) in in_room.iter().flatten().zip(plain.iter().flatten()) {
                dry_energy += f64::from(d * d);
                wet_energy += f64::from((w - d) * (w - d));
            }
        }
        (10.0 * (wet_energy / dry_energy).log10()) as f32
    }

    /// The test world's spaces, as `Room` and `FarEchoes` measure them
    /// (DESIGN.md "Room reverb and echo").
    fn small_room() -> Room {
        Room {
            reverb: 1.0,
            critical_distance: 1.16,
            decay: 0.94,
            ..Room::default()
        }
    }

    fn hall() -> Room {
        Room {
            reverb: 1.0,
            critical_distance: 3.2,
            decay: 2.45,
            echo: HALL_ECHO,
            echo_delay: 0.065,
            ..Room::default()
        }
    }

    fn cavern() -> Room {
        Room {
            reverb: 1.0,
            critical_distance: 8.3,
            decay: 5.24,
            far_echoes: [0.0, 1.0, 2.0, 3.0].map(|n: f64| FarEcho {
                direction: Vec3::new(n.cos(), 0.0, n.sin()),
                delay: 0.14,
                gain: CAVERN_ECHO,
                cutoff_hz: 6000.0,
            }),
            ..Room::default()
        }
    }

    fn canyon() -> Room {
        Room {
            far_echoes: [1.0, -1.0, 0.0, 0.0].map(|x: f64| FarEcho {
                direction: Vec3::new(x, 0.0, 0.0),
                delay: 0.2,
                gain: CANYON_ECHO * x.abs() as f32,
                cutoff_hz: 4800.0,
            }),
            ..Room::default()
        }
    }

    #[test]
    fn effects_stay_under_the_voice() {
        let small = wet_db(small_room(), 3.0);
        let small_near = wet_db(small_room(), 2.0);
        let small_far = wet_db(small_room(), 5.0);
        let hall = wet_db(hall(), 5.0);
        let cavern = wet_db(cavern(), 8.0);
        let canyon = wet_db(canyon(), 5.0);
        // Small rooms nearly dry, a hall a light tail, a cavern clearly
        // ringing and a canyon a clear echo, all under the voice. Measured
        // when this was set: -41, -14, -6 and -23 dB.
        for db in [small_near, small, small_far] {
            assert!(db < -20.0, "small room {db:.1} dB");
        }
        assert!((-18.0..-10.0).contains(&hall), "hall {hall:.1} dB");
        assert!((-10.0..-3.0).contains(&cavern), "cavern {cavern:.1} dB");
        assert!((-28.0..-18.0).contains(&canyon), "canyon {canyon:.1} dB");
    }
}
