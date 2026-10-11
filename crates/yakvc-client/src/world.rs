use yakvc_proto::Uuid;

/// A position in block coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

/// Where the local listener is and which way they face, in Minecraft's
/// convention (yaw 0 faces +Z, degrees).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Pose {
    pub pos: Vec3,
    pub yaw: f32,
    pub pitch: f32,
}

/// One tick's view of the world, pushed at 20 Hz.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct World {
    pub listener: Pose,
    /// Players with a tracked entity, spectators already removed.
    pub players: Vec<Player>,
    /// The listener's surroundings, which set the reverb.
    pub room: Room,
}

/// How the space around the listener rings, as the game estimates it.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Room {
    /// How much of each positional voice goes to the reverb: 0 in open air,
    /// up to 1 shut in.
    pub reverb: f32,
    /// Seconds until the reverb fades by 60 dB: longer in bigger spaces.
    pub decay: f32,
}

/// Another player as the listener hears them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Player {
    pub uuid: Uuid,
    /// Eye position.
    pub pos: Vec3,
    /// How much of the direct path from them to the listener is blocked,
    /// from 0 (open air) to 1 (fully blocked), as the game estimates it.
    pub occlusion: f32,
}

/// Local input state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Input {
    pub push_to_talk: bool,
    pub muted: bool,
    pub deafened: bool,
    /// The local player is a spectator: send and play nothing.
    pub spectator: bool,
}

impl Player {
    /// A player in open air.
    pub const fn new(uuid: Uuid, pos: Vec3) -> Self {
        Player {
            uuid,
            pos,
            occlusion: 0.0,
        }
    }
}

impl Vec3 {
    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Vec3 { x, y, z }
    }
}

pub(crate) fn distance(a: Vec3, b: Vec3) -> f64 {
    let (dx, dy, dz) = (a.x - b.x, a.y - b.y, a.z - b.z);
    (dx * dx + dy * dy + dz * dz).sqrt()
}

/// The world `t` of the way from `from` to `to` (0 is `from`, 1 is `to`).
/// Players missing from `from` stay where `to` puts them.
pub(crate) fn interpolate(from: &World, to: &World, t: f64) -> World {
    let lerp = |a: f64, b: f64| a + (b - a) * t;
    let lerp_pos = |a: Vec3, b: Vec3| Vec3::new(lerp(a.x, b.x), lerp(a.y, b.y), lerp(a.z, b.z));
    let (from_yaw, to_yaw) = (f64::from(from.listener.yaw), f64::from(to.listener.yaw));
    // Turn the short way round: from 350° to 10° is +20°, not -340°.
    let turn = (to_yaw - from_yaw + 180.0).rem_euclid(360.0) - 180.0;
    let listener = Pose {
        pos: lerp_pos(from.listener.pos, to.listener.pos),
        yaw: (from_yaw + turn * t) as f32,
        pitch: lerp(f64::from(from.listener.pitch), f64::from(to.listener.pitch)) as f32,
    };
    let players = to
        .players
        .iter()
        .map(
            |&to| match from.players.iter().find(|p| p.uuid == to.uuid) {
                Some(before) => Player {
                    pos: lerp_pos(before.pos, to.pos),
                    occlusion: lerp(f64::from(before.occlusion), f64::from(to.occlusion)) as f32,
                    ..to
                },
                None => to,
            },
        )
        .collect();
    let room = Room {
        reverb: lerp(f64::from(from.room.reverb), f64::from(to.room.reverb)) as f32,
        decay: lerp(f64::from(from.room.decay), f64::from(to.room.decay)) as f32,
    };
    World {
        listener,
        players,
        room,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world(x: f64, yaw: f32, player_x: f64) -> World {
        World {
            listener: Pose {
                pos: Vec3::new(x, 64.0, 0.0),
                yaw,
                pitch: 0.0,
            },
            players: vec![Player::new(
                Uuid::from_u128(1),
                Vec3::new(player_x, 64.0, 0.0),
            )],
            ..Default::default()
        }
    }

    #[test]
    fn interpolation_moves_everyone_part_way() {
        let from = world(0.0, 0.0, 10.0);
        let to = world(4.0, 90.0, 20.0);
        let half = interpolate(&from, &to, 0.5);
        assert_eq!(half.listener.pos, Vec3::new(2.0, 64.0, 0.0));
        assert_eq!(half.listener.yaw, 45.0);
        assert_eq!(half.players[0].pos, Vec3::new(15.0, 64.0, 0.0));
        assert_eq!(interpolate(&from, &to, 1.0), to);
    }

    #[test]
    fn interpolation_turns_the_short_way() {
        let half = interpolate(&world(0.0, 350.0, 0.0), &world(0.0, 10.0, 0.0), 0.5);
        assert_eq!(half.listener.yaw, 360.0);
    }

    #[test]
    fn players_new_to_the_snapshot_appear_where_they_are() {
        let to = world(0.0, 0.0, 30.0);
        let half = interpolate(&World::default(), &to, 0.5);
        assert_eq!(half.players, to.players);
    }
}
