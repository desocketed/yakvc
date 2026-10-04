use yakvc_shared::Uuid;

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
    pub players: Vec<(Uuid, Vec3)>,
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

impl Vec3 {
    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Vec3 { x, y, z }
    }
}
