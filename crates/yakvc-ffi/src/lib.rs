//! C ABI over `yakvc-client`, called from Java through the FFM API.
//!
//! This is the only crate allowed to use `unsafe`. Run `cargo xtask header`
//! after changing any export to regenerate `include/yakvc.h`.
//!
//! Conventions: every `int32_t` return is [`YAKVC_OK`] or a negative
//! `YAKVC_ERR_*` code, with a message available from [`yakvc_last_error`].
//! Strings are UTF-8 `(ptr, len)`, not NUL-terminated. UUIDs are 16 bytes,
//! big-endian (most significant half first). Native code never keeps a
//! pointer after a call returns. Every function except [`yakvc_destroy`] may
//! be called from any thread.
//!
//! Events are little-endian TLV records, `u16 type | u16 len | payload`:
//!
//! | Type | Payload |
//! | --- | --- |
//! | [`YAKVC_EVENT_JOIN_REQUEST`] | `u32 id`, then the server ID as UTF-8 |
//! | [`YAKVC_EVENT_RENDEZVOUS_STATE`] | `u8 YAKVC_RDV_*`, `u64 value`: expiry (Unix s) when registered, delay (ms) when retrying, else 0; `u8` 1 when registered with a verified ticket, else 0 |
//! | [`YAKVC_EVENT_PEER_STATE`] | `uuid[16]`, `u8 YAKVC_PEER_*`, `u8` 1 if the peer's ticket is verified |
//! | [`YAKVC_EVENT_TALKING`] | `uuid[16]`, `u8` 1 talking / 0 stopped |
//! | [`YAKVC_EVENT_MIC_LEVEL`] | `f32` dBFS |
//! | [`YAKVC_EVENT_ERROR`] | message as UTF-8 |
//! | [`YAKVC_EVENT_INVITE`] | `uuid[16]` of the player who invited us |
//! | [`YAKVC_EVENT_GROUP_NOTICE`] | `u8 YAKVC_GROUP_*`, `uuid[16]` the member it is about, `uuid[16] by` who made the change; all-zero when not applicable |

#![allow(unsafe_code)]

mod events;
mod guard;

use std::ffi::c_void;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;

use yakvc_client::{
    Config, ConfigError, Engine, FarEcho, GroupError, GroupId, Input, JoinId, MonoFrame, PeerAudio,
    Player, Pose, Room, StartError, Uuid, Vec3, VoiceFrame, VoiceSink, World,
};

use crate::events::EventQueue;
use crate::guard::{
    FfiError, buf_arg, guard, guard_poisonable, out_arg, slice_arg, str_arg, uuids_arg,
};

/// Version of this C ABI. Java refuses to use a library whose version differs
/// from the one it was built against. Bump on any incompatible change.
/// 2: verified flags in the rendezvous and peer state events.
/// 3: `yakvc_stats`.
/// 4: group voice chat (`yakvc_join_group` and the rest).
/// 5: groups reworked: random keys instead of names and passwords, invites,
/// shared group state and kick votes; group-only voice in a group.
/// 6: kick votes removed (`yakvc_vote_kick`, the kick vote event and notice).
/// 7: `yakvc_join_group` takes the id length.
/// 8: `yakvc_push_world` takes each player's occlusion.
/// 9: `yakvc_push_world` takes the room around the listener.
/// 10: `yakvc_push_world` takes where each player is heard from.
/// 11: `yakvc_push_world` takes the room's echo and echo delay.
/// 12: `yakvc_set_voice_sink`.
/// 15: `yakvc_push_world` takes the far echoes after the room.
pub const YAKVC_ABI_VERSION: u32 = 15;

pub const YAKVC_OK: i32 = 0;
/// A null pointer, bad UUID, invalid UTF-8 or out-of-range value.
pub const YAKVC_ERR_INVALID_ARGUMENT: i32 = -1;
/// `yakvc_create` was passed a different ABI version.
pub const YAKVC_ERR_ABI_MISMATCH: i32 = -2;
/// The config TOML did not parse or validate.
pub const YAKVC_ERR_CONFIG: i32 = -3;
/// The engine failed to start (key file, network).
pub const YAKVC_ERR_START: i32 = -4;
/// `yakvc_list_devices` could not list audio devices. A device that is
/// missing or fails while the engine runs is a `YAKVC_EVENT_ERROR` instead.
pub const YAKVC_ERR_AUDIO: i32 = -5;
/// This call panicked. The engine is now poisoned.
pub const YAKVC_ERR_PANIC: i32 = -6;
/// An earlier call, or one of the engine's own threads, panicked; only
/// `yakvc_destroy` is still allowed.
pub const YAKVC_ERR_POISONED: i32 = -7;
/// A group action that can't be done now: no invite from that player in the
/// last two minutes, no public group with that id, or not in a group. The
/// message says which.
pub const YAKVC_ERR_UNAVAILABLE: i32 = -8;

pub const YAKVC_INPUT_PUSH_TO_TALK: u32 = 1 << 0;
pub const YAKVC_INPUT_MUTED: u32 = 1 << 1;
pub const YAKVC_INPUT_DEAFENED: u32 = 1 << 2;
pub const YAKVC_INPUT_SPECTATOR: u32 = 1 << 3;

pub const YAKVC_EVENT_JOIN_REQUEST: u16 = 1;
pub const YAKVC_EVENT_RENDEZVOUS_STATE: u16 = 2;
pub const YAKVC_EVENT_PEER_STATE: u16 = 3;
pub const YAKVC_EVENT_TALKING: u16 = 4;
pub const YAKVC_EVENT_MIC_LEVEL: u16 = 5;
pub const YAKVC_EVENT_ERROR: u16 = 6;
pub const YAKVC_EVENT_INVITE: u16 = 7;
pub const YAKVC_EVENT_GROUP_NOTICE: u16 = 8;

/// `uuid` became a group mate.
pub const YAKVC_GROUP_JOINED: u8 = 0;
/// `uuid` is no longer a group mate (left or link closed).
pub const YAKVC_GROUP_LEFT: u8 = 1;
/// `by` made the group public.
pub const YAKVC_GROUP_MADE_PUBLIC: u8 = 2;
/// `by` made the group private.
pub const YAKVC_GROUP_MADE_PRIVATE: u8 = 3;
/// `by` changed the label.
pub const YAKVC_GROUP_LABEL_CHANGED: u8 = 4;

pub const YAKVC_RDV_CONNECTING: u8 = 0;
pub const YAKVC_RDV_AUTHENTICATING: u8 = 1;
pub const YAKVC_RDV_REGISTERED: u8 = 2;
pub const YAKVC_RDV_RETRYING: u8 = 3;
pub const YAKVC_RDV_DISCONNECTED: u8 = 4;

pub const YAKVC_PEER_CONNECTING: u8 = 0;
pub const YAKVC_PEER_DIRECT: u8 = 1;
pub const YAKVC_PEER_RELAYED: u8 = 2;
pub const YAKVC_PEER_RELAY_FULL: u8 = 3;
pub const YAKVC_PEER_FAILED: u8 = 4;
pub const YAKVC_PEER_GONE: u8 = 5;
/// Linked, but the peer runs an older Yak VC: no voice until it updates.
pub const YAKVC_PEER_OLDER_VERSION: u8 = 6;
/// Linked, but the peer runs a newer Yak VC: no voice until we update.
pub const YAKVC_PEER_NEWER_VERSION: u8 = 7;

/// The newest `YakVcVoiceSink` version this library takes. Version 1 sinks
/// get other players' voices; version 2 sinks also get the player's own, as
/// `YAKVC_VOICE_SELF` frames. The struct is the same.
pub const YAKVC_VOICE_SINK_VERSION: u32 = 2;
/// A `YakVcVoiceFrame` of group voice.
pub const YAKVC_VOICE_GROUP: u32 = 1 << 0;
/// A `YakVcVoiceFrame` that plays centred rather than from the speaker's
/// place in the world: group voice and direct calls.
pub const YAKVC_VOICE_UNPOSITIONED: u32 = 1 << 1;
/// A `YakVcVoiceFrame` of the player's own voice, as it goes out while they
/// transmit (after input processing; never while deafened). Only version 2
/// sinks get these, for letting the room around the player answer them: they
/// must not play it dry. Also `YAKVC_VOICE_UNPOSITIONED`, with volume 1 and
/// the player's UUID. Paced by the microphone, not `wants_frame`.
pub const YAKVC_VOICE_SELF: u32 = 1 << 2;

/// Opaque engine handle.
#[derive(Debug)]
pub struct YakVcEngine {
    engine: Engine,
    events: Mutex<EventQueue>,
    /// Set when a call panics; every later call except `yakvc_destroy` fails.
    poisoned: AtomicBool,
}

/// One speaker's 20 ms of voice for a [`YakVcVoiceSink`], valid only during
/// the `frame` call.
#[repr(C)]
#[derive(Debug)]
pub struct YakVcVoiceFrame {
    /// The speaker.
    pub uuid: [u8; 16],
    /// `samples` floats of 48 kHz mono, as decoded.
    pub pcm: *const f32,
    pub samples: usize,
    /// Counts play ticks: the frames played together share it. For
    /// `YAKVC_VOICE_SELF` frames, counts those instead.
    pub tick: u64,
    /// The player's volume setting, 0 to 2.
    pub volume: f32,
    /// The local voice range in blocks; 0 when unpositioned.
    pub range: f32,
    /// A combination of `YAKVC_VOICE_*`.
    pub flags: u32,
    /// Where the speakers would place the voice, `x, y, z`: the speaker's
    /// eye, or the opening it comes round through. Zero when unpositioned.
    pub heard_from: [f64; 3],
    /// How much of the way the speakers would block, from 0 (open air) to 1;
    /// 0 when unpositioned.
    pub occlusion: f32,
}

/// A mod that plays the voices instead of the speakers: see
/// [`yakvc_set_voice_sink`].
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct YakVcVoiceSink {
    /// 1, or 2 for a sink that also wants the player's own voice
    /// (`YAKVC_VOICE_SELF` frames).
    pub version: u32,
    /// Passed back to every callback.
    pub user: *mut c_void,
    /// Whether the sink wants the next 20 ms of voice now. It paces playback,
    /// as the speakers' clock does.
    pub wants_frame: Option<unsafe extern "C" fn(user: *mut c_void) -> bool>,
    /// One playable frame from one speaker.
    pub frame: Option<unsafe extern "C" fn(user: *mut c_void, frame: *const YakVcVoiceFrame)>,
    /// The speaker `uuid` (16 bytes) is gone: no more frames from it until
    /// it comes back. Also sent for every stream when the sink is replaced
    /// or cleared.
    pub stream_end: Option<unsafe extern "C" fn(user: *mut c_void, uuid: *const u8)>,
}

/// Runs `f` on a live engine behind the panic barrier.
///
/// # Safety
/// `engine` must be null or live.
unsafe fn with_engine(
    engine: *mut YakVcEngine,
    f: impl FnOnce(&YakVcEngine) -> Result<(), FfiError>,
) -> i32 {
    // SAFETY: the caller guarantees `engine` is null or live, and live
    // engines are only freed by `yakvc_destroy`, which must be the last call.
    let Some(engine) = (unsafe { engine.as_ref() }) else {
        return guard(|| Err(FfiError::invalid("engine is null")));
    };
    // A panic on one of the engine's own threads poisons it too.
    if let Some(failure) = engine.engine.failure() {
        return guard(|| Err(FfiError::new(YAKVC_ERR_POISONED, failure)));
    }
    guard_poisonable(&engine.poisoned, || f(engine))
}

/// Returns [`YAKVC_ABI_VERSION`]. Safe to call before anything else.
#[unsafe(no_mangle)]
pub extern "C" fn yakvc_abi_version() -> u32 {
    YAKVC_ABI_VERSION
}

/// Starts an engine. `config_dir` holds the client key and ticket cache;
/// `config_toml` is the contents of `client.toml`.
///
/// # Safety
/// The `(ptr, len)` pairs must be readable; `out` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yakvc_create(
    config_dir: *const u8,
    dir_len: usize,
    config_toml: *const u8,
    toml_len: usize,
    abi_version: u32,
    out: *mut *mut YakVcEngine,
) -> i32 {
    guard(|| {
        // SAFETY: the caller guarantees `out` is writable.
        let out = unsafe { out_arg(out, "out") }?;
        *out = std::ptr::null_mut();
        if abi_version != YAKVC_ABI_VERSION {
            return Err(FfiError::new(
                YAKVC_ERR_ABI_MISMATCH,
                format!("native ABI is {YAKVC_ABI_VERSION}, Java expects {abi_version}"),
            ));
        }
        // SAFETY: the caller guarantees both pairs are readable.
        let dir = unsafe { str_arg(config_dir, dir_len, "config_dir") }?;
        // SAFETY: as above.
        let toml = unsafe { str_arg(config_toml, toml_len, "config_toml") }?;

        let config = Config::from_toml(toml).map_err(config_error)?;
        let (engine, events) = Engine::builder(config)
            .data_dir(dir)
            .start()
            .map_err(start_error)?;
        *out = Box::into_raw(Box::new(YakVcEngine {
            engine,
            events: Mutex::new(EventQueue::new(events)),
            poisoned: AtomicBool::new(false),
        }));
        Ok(())
    })
}

/// Shuts down gracefully (at most 500 ms) and frees the handle. Must be the
/// last call on `engine`. Null is ignored.
///
/// # Safety
/// `engine` must come from `yakvc_create` and not be used afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yakvc_destroy(engine: *mut YakVcEngine) {
    if engine.is_null() {
        return;
    }
    // SAFETY: `engine` came from `Box::into_raw` in `yakvc_create`, and the
    // caller never uses it again.
    let engine = unsafe { Box::from_raw(engine) };
    // Dropping shuts the engine down, which must not unwind into Java either,
    // even on a poisoned engine.
    guard(|| {
        drop(engine);
        Ok(())
    });
}

/// # Safety
/// `engine` must be live; `uuid` must point to 16 bytes; `name` must be readable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yakvc_set_identity(
    engine: *mut YakVcEngine,
    uuid: *const u8,
    name: *const u8,
    name_len: usize,
) -> i32 {
    // SAFETY: the caller guarantees `engine` is live.
    unsafe {
        with_engine(engine, |e| {
            // SAFETY: the caller guarantees `uuid` points to 16 bytes.
            let uuid = uuids_arg(uuid, 1, "uuid")?[0];
            // SAFETY: the caller guarantees `name` is readable.
            let name = str_arg(name, name_len, "name")?;
            e.engine.set_identity(uuid, name);
            Ok(())
        })
    }
}

/// Replaces the tab list with `count` UUIDs. Call only when it changes.
///
/// # Safety
/// `engine` must be live; `uuids` must point to `16 * count` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yakvc_set_tab_list(
    engine: *mut YakVcEngine,
    uuids: *const u8,
    count: usize,
) -> i32 {
    // SAFETY: the caller guarantees `engine` is live.
    unsafe {
        with_engine(engine, |e| {
            // SAFETY: the caller guarantees `uuids` points to `16 * count` bytes.
            let uuids = uuids_arg(uuids, count, "uuids")?;
            e.engine.set_tab_list(uuids);
            Ok(())
        })
    }
}

/// Pushes one tick's world: `listener` is `x, y, z, yaw, pitch, reverb,
/// critical_distance, decay, echo, echo_delay`, the last five describing
/// the room around it (see `Room`), then four far echoes of `dx, dy, dz,
/// delay, gain, cutoff_hz` each: the unit vector to the surface, seconds
/// until it comes back, its gain (0 for an unused one) and the air's
/// low-pass (see `FarEcho`). Then `count`
/// tracked players as UUIDs and, per player, `x, y, z, occlusion, heard_x,
/// heard_y, heard_z`: the eye position, how much the sound reaching the
/// listener is blocked, from 0 (open air) to 1 (fully blocked), and where
/// the voice seems to come from (see `Player`).
///
/// # Safety
/// `engine` must be live; `listener` must point to 34 doubles, `uuids` to
/// `16 * count` bytes and `players` to `7 * count` doubles.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yakvc_push_world(
    engine: *mut YakVcEngine,
    listener: *const f64,
    uuids: *const u8,
    players: *const f64,
    count: usize,
) -> i32 {
    // SAFETY: the caller guarantees `engine` is live.
    unsafe {
        with_engine(engine, |e| {
            // SAFETY: the caller guarantees `listener` points to 34 doubles.
            let listener = slice_arg(listener, 34, "listener")?;
            // SAFETY: the caller guarantees `uuids` points to `16 * count` bytes.
            let uuids = uuids_arg(uuids, count, "uuids")?;
            let players_len = count
                .checked_mul(7)
                .ok_or_else(|| FfiError::invalid("count is too large"))?;
            // SAFETY: the caller guarantees `players` points to `7 * count` doubles.
            let players = slice_arg(players, players_len, "players")?;
            if !listener.iter().chain(players).all(|v| v.is_finite()) {
                return Err(FfiError::invalid("world contains NaN or infinity"));
            }

            let players = uuids
                .into_iter()
                .zip(players.as_chunks::<7>().0)
                .map(|(uuid, &[x, y, z, occlusion, hx, hy, hz])| Player {
                    uuid,
                    pos: Vec3::new(x, y, z),
                    occlusion: occlusion as f32,
                    heard_from: Vec3::new(hx, hy, hz),
                })
                .collect();
            let room = Room {
                reverb: listener[5] as f32,
                critical_distance: listener[6] as f32,
                decay: listener[7] as f32,
                echo: listener[8] as f32,
                echo_delay: listener[9] as f32,
                far_echoes: std::array::from_fn(|i| {
                    let [dx, dy, dz, delay, gain, cutoff_hz] = listener[10..].as_chunks::<6>().0[i];
                    FarEcho {
                        direction: Vec3::new(dx, dy, dz),
                        delay: delay as f32,
                        gain: gain as f32,
                        cutoff_hz: cutoff_hz as f32,
                    }
                }),
            };
            let listener = Pose {
                pos: Vec3::new(listener[0], listener[1], listener[2]),
                yaw: listener[3] as f32,
                pitch: listener[4] as f32,
            };
            e.engine.set_world(World {
                listener,
                players,
                room,
            });
            Ok(())
        })
    }
}

/// `flags` is a combination of `YAKVC_INPUT_*`.
///
/// # Safety
/// `engine` must be live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yakvc_set_input(engine: *mut YakVcEngine, flags: u32) -> i32 {
    const KNOWN: u32 =
        YAKVC_INPUT_PUSH_TO_TALK | YAKVC_INPUT_MUTED | YAKVC_INPUT_DEAFENED | YAKVC_INPUT_SPECTATOR;
    // SAFETY: the caller guarantees `engine` is live.
    unsafe {
        with_engine(engine, |e| {
            if flags & !KNOWN != 0 {
                return Err(FfiError::invalid(format!("unknown input flags {flags:#x}")));
            }
            e.engine.set_input(Input {
                push_to_talk: flags & YAKVC_INPUT_PUSH_TO_TALK != 0,
                muted: flags & YAKVC_INPUT_MUTED != 0,
                deafened: flags & YAKVC_INPUT_DEAFENED != 0,
                spectator: flags & YAKVC_INPUT_SPECTATOR != 0,
            });
            Ok(())
        })
    }
}

/// The game's Voice/Speech slider, `0..1`.
///
/// # Safety
/// `engine` must be live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yakvc_set_game_volume(engine: *mut YakVcEngine, volume: f32) -> i32 {
    // SAFETY: the caller guarantees `engine` is live.
    unsafe {
        with_engine(engine, |e| {
            if !(0.0..=1.0).contains(&volume) {
                return Err(FfiError::invalid(format!(
                    "game volume {volume} is outside 0..1"
                )));
            }
            e.engine.set_game_volume(volume);
            Ok(())
        })
    }
}

/// The name of the game's selected sound device; null or zero length means
/// the system default. Voice output follows the closest-named device unless
/// `audio.output_device` is set.
///
/// # Safety
/// `engine` must be live; `name` must be readable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yakvc_set_game_device(
    engine: *mut YakVcEngine,
    name: *const u8,
    len: usize,
) -> i32 {
    // SAFETY: the caller guarantees `engine` is live.
    unsafe {
        with_engine(engine, |e| {
            // SAFETY: the caller guarantees `name` is readable.
            let name = str_arg(name, len, "name")?;
            e.engine.set_game_device((!name.is_empty()).then_some(name));
            Ok(())
        })
    }
}

/// # Safety
/// `engine` must be live; `uuid` must point to 16 bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yakvc_set_peer_volume(
    engine: *mut YakVcEngine,
    uuid: *const u8,
    volume: f32,
    muted: bool,
) -> i32 {
    // SAFETY: the caller guarantees `engine` is live.
    unsafe {
        with_engine(engine, |e| {
            // SAFETY: the caller guarantees `uuid` points to 16 bytes.
            let uuid = uuids_arg(uuid, 1, "uuid")?[0];
            if !(volume.is_finite() && volume >= 0.0) {
                return Err(FfiError::invalid(format!(
                    "peer volume {volume} is not a finite, non-negative number"
                )));
            }
            e.engine.set_peer_audio(uuid, PeerAudio { volume, muted });
            Ok(())
        })
    }
}

/// Sends playable voice to `sink` instead of the speakers, for a mod that
/// renders voices itself; null goes back to the speakers. Takes effect on the
/// audio thread's next tick.
///
/// The sink gets each frame the speakers would have played, as decoded:
/// before panning, distance gain, occlusion, reverb, the game volume and the
/// limiter, which the mod applies itself. Only the rules of who is heard
/// (mute, deafen, groups, spectators, range) and the player's volume setting
/// apply. While a sink is set the speakers are closed.
///
/// The struct is copied, but `user` and the callbacks must stay valid for the
/// rest of the process: the audio thread may still call an old sink for a
/// tick after a change, and a shutdown that abandons a stuck audio thread may
/// call it later still. The callbacks run on the engine's audio thread: they
/// must not block, must not unwind and must not call any `yakvc_*` function.
///
/// From Java, build the struct and any upcall stubs in `Arena.global()`: a
/// confined or automatic arena can free them while the audio thread still
/// calls them. A callback must never let an exception escape, which crashes
/// the JVM, and shouldn't allocate or block, since it runs on the audio
/// path; renderers usually write the callbacks natively and hand Java only
/// the struct's address.
///
/// # Safety
/// `engine` must be live; `sink` must be null or point to a `YakVcVoiceSink`
/// whose `user` and callbacks keep that contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yakvc_set_voice_sink(
    engine: *mut YakVcEngine,
    sink: *const YakVcVoiceSink,
) -> i32 {
    // SAFETY: the caller guarantees `engine` is live.
    unsafe {
        with_engine(engine, |e| {
            // SAFETY: the caller guarantees `sink` is null or readable.
            let sink = match sink.as_ref() {
                Some(sink) => Some(Box::new(ForeignSink::new(sink)?) as Box<dyn VoiceSink>),
                None => None,
            };
            e.engine.set_voice_sink(sink);
            Ok(())
        })
    }
}

/// A [`YakVcVoiceSink`] as the engine's [`VoiceSink`].
#[derive(Debug)]
struct ForeignSink {
    /// A version 2 sink, which wants the player's own voice.
    own_voice: bool,
    /// Counts own-voice frames, for their `tick`.
    own_frames: u64,
    user: *mut c_void,
    wants_frame: unsafe extern "C" fn(*mut c_void) -> bool,
    frame: unsafe extern "C" fn(*mut c_void, *const YakVcVoiceFrame),
    stream_end: unsafe extern "C" fn(*mut c_void, *const u8),
}

// SAFETY: `yakvc_set_voice_sink`'s contract: `user` and the callbacks may be
// used from the engine's audio thread for the rest of the process.
unsafe impl Send for ForeignSink {}

impl ForeignSink {
    fn new(sink: &YakVcVoiceSink) -> Result<ForeignSink, FfiError> {
        if !(1..=YAKVC_VOICE_SINK_VERSION).contains(&sink.version) {
            return Err(FfiError::invalid(format!(
                "voice sink version {} is not 1 to {YAKVC_VOICE_SINK_VERSION}",
                sink.version
            )));
        }
        let (Some(wants_frame), Some(frame), Some(stream_end)) =
            (sink.wants_frame, sink.frame, sink.stream_end)
        else {
            return Err(FfiError::invalid("a voice sink callback is null"));
        };
        Ok(ForeignSink {
            own_voice: sink.version >= 2,
            own_frames: 0,
            user: sink.user,
            wants_frame,
            frame,
            stream_end,
        })
    }
}

impl VoiceSink for ForeignSink {
    fn wants_frame(&mut self) -> bool {
        // SAFETY: `yakvc_set_voice_sink`'s contract.
        unsafe { (self.wants_frame)(self.user) }
    }

    fn frame(&mut self, frame: &VoiceFrame<'_>) {
        let mut flags = 0;
        if frame.group {
            flags |= YAKVC_VOICE_GROUP;
        }
        if !frame.positioned {
            flags |= YAKVC_VOICE_UNPOSITIONED;
        }
        let frame = YakVcVoiceFrame {
            uuid: *frame.uuid.as_bytes(),
            pcm: frame.pcm.as_ptr(),
            samples: frame.pcm.len(),
            tick: frame.tick,
            volume: frame.volume,
            range: frame.range,
            flags,
            heard_from: [frame.heard_from.x, frame.heard_from.y, frame.heard_from.z],
            occlusion: frame.occlusion,
        };
        // SAFETY: `yakvc_set_voice_sink`'s contract; `frame` and the samples
        // it points to outlive the call.
        unsafe { (self.frame)(self.user, &frame) }
    }

    fn stream_end(&mut self, uuid: Uuid) {
        // SAFETY: `yakvc_set_voice_sink`'s contract; the 16 bytes outlive the
        // call.
        unsafe { (self.stream_end)(self.user, uuid.as_bytes().as_ptr()) }
    }

    fn wants_own_voice(&self) -> bool {
        self.own_voice
    }

    fn own_voice(&mut self, uuid: Uuid, pcm: &MonoFrame) {
        self.own_frames += 1;
        let frame = YakVcVoiceFrame {
            uuid: *uuid.as_bytes(),
            pcm: pcm.as_ptr(),
            samples: pcm.len(),
            tick: self.own_frames,
            volume: 1.0,
            range: 0.0,
            flags: YAKVC_VOICE_SELF | YAKVC_VOICE_UNPOSITIONED,
            heard_from: [0.0; 3],
            occlusion: 0.0,
        };
        // SAFETY: `yakvc_set_voice_sink`'s contract; `frame` and the samples
        // it points to outlive the call.
        unsafe { (self.frame)(self.user, &frame) }
    }
}

/// Reports the result of `joinServer` for a `YAKVC_EVENT_JOIN_REQUEST`.
///
/// # Safety
/// `engine` must be live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yakvc_complete_join(
    engine: *mut YakVcEngine,
    request_id: u32,
    ok: bool,
) -> i32 {
    // SAFETY: the caller guarantees `engine` is live.
    unsafe {
        with_engine(engine, |e| {
            e.engine.complete_join(JoinId(request_id), ok);
            Ok(())
        })
    }
}

/// A refused group action: a bad label is the caller's mistake, the rest
/// depend on what has happened in the game since.
fn group_error(err: GroupError) -> FfiError {
    let code = match err {
        GroupError::LabelTooLong => YAKVC_ERR_INVALID_ARGUMENT,
        _ => YAKVC_ERR_UNAVAILABLE,
    };
    FfiError::new(code, err.to_string())
}

/// Invites `target` to our group. While we are in none, the invite is to a
/// new group we join only once `target` accepts; until then we stay in
/// proximity voice. Sent over the peer's link if there is one; the invite
/// lasts two minutes.
///
/// # Safety
/// `engine` must be live; `target` must point to 16 bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yakvc_invite(engine: *mut YakVcEngine, target: *const u8) -> i32 {
    // SAFETY: the caller guarantees `engine` is live.
    unsafe {
        with_engine(engine, |e| {
            // SAFETY: the caller guarantees `target` points to 16 bytes.
            let target = uuids_arg(target, 1, "target")?[0];
            e.engine.invite(target);
            Ok(())
        })
    }
}

/// Joins the group of the last invite (`YAKVC_EVENT_INVITE`) from `from`,
/// leaving any other. `YAKVC_ERR_UNAVAILABLE` if there was none in the last
/// two minutes.
///
/// # Safety
/// `engine` must be live; `from` must point to 16 bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yakvc_accept_invite(engine: *mut YakVcEngine, from: *const u8) -> i32 {
    // SAFETY: the caller guarantees `engine` is live.
    unsafe {
        with_engine(engine, |e| {
            // SAFETY: the caller guarantees `from` points to 16 bytes.
            let from = uuids_arg(from, 1, "from")?[0];
            e.engine.accept_invite(from).map_err(group_error)
        })
    }
}

/// Joins a public group from `yakvc_list_groups` by its `id`, 64 hex
/// characters, leaving any other.
/// `YAKVC_ERR_UNAVAILABLE` if no peer announces it any more.
///
/// # Safety
/// `engine` must be live; `id_hex` must be readable for `len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yakvc_join_group(
    engine: *mut YakVcEngine,
    id_hex: *const u8,
    len: usize,
) -> i32 {
    // SAFETY: the caller guarantees `engine` is live.
    unsafe {
        with_engine(engine, |e| {
            // SAFETY: the caller guarantees `id_hex` is readable.
            let hex = str_arg(id_hex, len, "id")?;
            let id = GroupId::from_hex(hex)
                .ok_or_else(|| FfiError::invalid("id is not 64 hex characters"))?;
            e.engine.join_group(id).map_err(group_error)
        })
    }
}

/// # Safety
/// `engine` must be live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yakvc_leave_group(engine: *mut YakVcEngine) -> i32 {
    // SAFETY: the caller guarantees `engine` is live.
    unsafe {
        with_engine(engine, |e| {
            e.engine.leave_group();
            Ok(())
        })
    }
}

/// Makes our group public (anyone may join) or private.
/// `YAKVC_ERR_UNAVAILABLE` outside a group.
///
/// # Safety
/// `engine` must be live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yakvc_set_group_public(engine: *mut YakVcEngine, public_: bool) -> i32 {
    // SAFETY: the caller guarantees `engine` is live.
    unsafe {
        with_engine(engine, |e| {
            e.engine.set_group_public(public_).map_err(group_error)
        })
    }
}

/// Sets our group's label, UTF-8: empty for none, at most 32 characters
/// (`YAKVC_ERR_INVALID_ARGUMENT` if longer). `YAKVC_ERR_UNAVAILABLE` outside
/// a group.
///
/// # Safety
/// `engine` must be live; `label` must be readable for `len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yakvc_set_group_label(
    engine: *mut YakVcEngine,
    label: *const u8,
    len: usize,
) -> i32 {
    // SAFETY: the caller guarantees `engine` is live.
    unsafe {
        with_engine(engine, |e| {
            // SAFETY: the caller guarantees `label` is readable.
            let label = str_arg(label, len, "label")?;
            e.engine.set_group_label(label).map_err(group_error)
        })
    }
}

/// Writes the groups we know of as JSON, with the buffer protocol of
/// [`yakvc_list_devices`]: an array of `{ "id": 64 hex characters, "label",
/// "public", "mine", "members": [hyphenated UUIDs] }`. Ours first (members
/// include us), then the public groups our peers announce.
///
/// # Safety
/// `engine` must be live; `buf` must be writable for `cap` bytes; `needed`
/// must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yakvc_list_groups(
    engine: *mut YakVcEngine,
    buf: *mut u8,
    cap: usize,
    needed: *mut usize,
) -> i32 {
    // SAFETY: the caller guarantees `engine` is live.
    unsafe {
        with_engine(engine, |e| {
            // SAFETY: the caller guarantees `needed` is writable.
            let needed = out_arg(needed, "needed")?;
            // SAFETY: the caller guarantees `buf` is writable for `cap` bytes.
            let buf = buf_arg(buf, cap, "buf")?;
            let groups: Vec<_> = e
                .engine
                .groups()
                .into_iter()
                .map(|group| {
                    serde_json::json!({
                        "id": group.id.to_string(),
                        "label": group.label,
                        "public": group.public,
                        "mine": group.mine,
                        "members": group.members.iter().map(Uuid::to_string).collect::<Vec<_>>(),
                    })
                })
                .collect();
            let json = serde_json::to_vec(&groups).expect("group list serializes");
            *needed = write_if_fits(&json, buf);
            Ok(())
        })
    }
}

/// Writes whole queued event records into `buf`; `*written` is the bytes
/// used. Records that don't fit stay queued for the next call.
///
/// # Safety
/// `engine` must be live; `buf` must be writable for `cap` bytes; `written`
/// must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yakvc_poll_events(
    engine: *mut YakVcEngine,
    buf: *mut u8,
    cap: usize,
    written: *mut usize,
) -> i32 {
    // SAFETY: the caller guarantees `engine` is live.
    unsafe {
        with_engine(engine, |e| {
            // SAFETY: the caller guarantees `written` is writable.
            let written = out_arg(written, "written")?;
            *written = 0;
            // SAFETY: the caller guarantees `buf` is writable for `cap` bytes.
            let buf = buf_arg(buf, cap, "buf")?;
            let mut events = e.events.lock().expect("event queue lock");
            *written = events.drain_into(buf)?;
            Ok(())
        })
    }
}

/// Applies a new `client.toml`.
///
/// # Safety
/// `engine` must be live; `toml` must be readable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yakvc_update_config(
    engine: *mut YakVcEngine,
    toml: *const u8,
    len: usize,
) -> i32 {
    // SAFETY: the caller guarantees `engine` is live.
    unsafe {
        with_engine(engine, |e| {
            // SAFETY: the caller guarantees `toml` is readable.
            let toml = str_arg(toml, len, "toml")?;
            let config = Config::from_toml(toml).map_err(config_error)?;
            e.engine.update_config(config);
            Ok(())
        })
    }
}

/// Writes the audio device list as JSON. `*needed` is the full length; if it
/// exceeds `cap`, nothing is written and the caller retries with a larger
/// buffer.
///
/// # Safety
/// `engine` must be live; `buf` must be writable for `cap` bytes; `needed`
/// must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yakvc_list_devices(
    engine: *mut YakVcEngine,
    buf: *mut u8,
    cap: usize,
    needed: *mut usize,
) -> i32 {
    // SAFETY: the caller guarantees `engine` is live.
    unsafe {
        with_engine(engine, |e| {
            // SAFETY: the caller guarantees `needed` is writable.
            let needed = out_arg(needed, "needed")?;
            // SAFETY: the caller guarantees `buf` is writable for `cap` bytes.
            let buf = buf_arg(buf, cap, "buf")?;
            let devices = e
                .engine
                .devices()
                .map_err(|err| FfiError::new(YAKVC_ERR_AUDIO, err.to_string()))?;
            let json = serde_json::to_vec(&devices).expect("device list serializes");
            *needed = write_if_fits(&json, buf);
            Ok(())
        })
    }
}

/// Writes a diagnostics snapshot as JSON, for the in-game debug overlay. Same
/// buffer protocol as [`yakvc_list_devices`]. Cheap enough for a few calls a
/// second, not for every frame. Fields:
///
/// - `endpoint_id`: our Iroh endpoint ID, hex
/// - `voice_version`: the voice protocol version we speak
/// - `audio`: `microphone_open`, `speakers_open`, `external_output` (voice
///   goes to a voice sink), `transmitting` (bools);
///   `bitrate` (bit/s in use); `overruns`, `underruns` (counts since the
///   device opened)
/// - `peers`: per connected peer, `uuid` (hyphenated), `rtt_ms` (number or
///   null), `voice_version` (the one it offered; null until linked), and
///   once audio has arrived `received`, `late`, `fec_recovered`,
///   `concealed` (frame counts) and `playout_delay_ms`
///
/// # Safety
/// `engine` must be live; `buf` must be writable for `cap` bytes; `needed`
/// must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yakvc_stats(
    engine: *mut YakVcEngine,
    buf: *mut u8,
    cap: usize,
    needed: *mut usize,
) -> i32 {
    // SAFETY: the caller guarantees `engine` is live.
    unsafe {
        with_engine(engine, |e| {
            // SAFETY: the caller guarantees `needed` is writable.
            let needed = out_arg(needed, "needed")?;
            // SAFETY: the caller guarantees `buf` is writable for `cap` bytes.
            let buf = buf_arg(buf, cap, "buf")?;
            let json = serde_json::to_vec(&stats_json(&e.engine)).expect("stats serialize");
            *needed = write_if_fits(&json, buf);
            Ok(())
        })
    }
}

fn stats_json(engine: &Engine) -> serde_json::Value {
    let audio = engine.audio_stats();
    let peers: Vec<_> = engine
        .peers()
        .into_iter()
        .map(|peer| {
            let mut json = serde_json::json!({
                "uuid": peer.uuid.to_string(),
                "rtt_ms": peer.rtt.map(|rtt| rtt.as_secs_f64() * 1000.0),
                "voice_version": peer.voice_version,
            });
            if let Some(stream) = peer.stream {
                json["received"] = stream.received.into();
                json["late"] = stream.late.into();
                json["fec_recovered"] = stream.fec_recovered.into();
                json["concealed"] = stream.concealed.into();
                json["playout_delay_ms"] = (stream.playout_delay.as_secs_f64() * 1000.0).into();
            }
            json
        })
        .collect();
    serde_json::json!({
        "endpoint_id": engine.endpoint_id(),
        "voice_version": yakvc_proto::voice::VERSION,
        "audio": {
            "microphone_open": audio.microphone_open,
            "speakers_open": audio.speakers_open,
            "external_output": audio.external_output,
            "transmitting": audio.transmitting,
            "bitrate": audio.bitrate,
            "overruns": audio.overruns,
            "underruns": audio.underruns,
        },
        "peers": peers,
    })
}

/// Copies this thread's last error message into `buf` (truncated to `cap`,
/// at a character boundary, so what is copied is still UTF-8) and returns
/// its full length, or 0 if there is none.
///
/// # Safety
/// `buf` must be writable for `cap` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yakvc_last_error(buf: *mut u8, cap: usize) -> usize {
    let message = guard::last_error();
    // SAFETY: the caller guarantees `buf` is writable for `cap` bytes.
    if let Ok(buf) = unsafe { buf_arg(buf, cap, "buf") } {
        let n = message.floor_char_boundary(buf.len());
        buf[..n].copy_from_slice(&message.as_bytes()[..n]);
    }
    message.len()
}

/// Copies all of `bytes` into `buf` if they fit, else nothing. Returns the
/// length needed either way.
fn write_if_fits(bytes: &[u8], buf: &mut [u8]) -> usize {
    if let Some(dest) = buf.get_mut(..bytes.len()) {
        dest.copy_from_slice(bytes);
    }
    bytes.len()
}

fn config_error(err: ConfigError) -> FfiError {
    FfiError::new(YAKVC_ERR_CONFIG, err.to_string())
}

fn start_error(err: StartError) -> FfiError {
    let code = match err {
        StartError::Config(_) => YAKVC_ERR_CONFIG,
        StartError::Key(_) | StartError::Network(_) => YAKVC_ERR_START,
    };
    FfiError::new(code, err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_if_fits_is_all_or_nothing() {
        let mut small = [0u8; 3];
        assert_eq!(write_if_fits(b"abcd", &mut small), 4);
        assert_eq!(small, [0, 0, 0]);

        let mut big = [0u8; 6];
        assert_eq!(write_if_fits(b"abcd", &mut big), 4);
        assert_eq!(&big[..4], b"abcd");
    }

    #[test]
    fn last_error_is_cut_between_characters() {
        guard(|| Err(FfiError::invalid("ää")));
        let mut buf = [0u8; 3];
        // SAFETY: `buf` is writable for its length.
        let len = unsafe { yakvc_last_error(buf.as_mut_ptr(), buf.len()) };
        assert_eq!(len, 4);
        assert_eq!(&buf, "ä\0".as_bytes());
    }

    #[test]
    fn engine_handle_is_shareable_across_threads() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<YakVcEngine>();
    }

    /// `cargo xtask header --check` keeps the header current, but only when
    /// it runs: this catches a function added without regenerating it.
    #[test]
    fn header_declares_every_exported_function() {
        let crate_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut exported = std::collections::BTreeSet::new();
        for entry in std::fs::read_dir(crate_dir.join("src")).unwrap() {
            let source = std::fs::read_to_string(entry.unwrap().path()).unwrap();
            for after in source.split("extern \"C\" fn ").skip(1) {
                exported.insert(identifier(after));
            }
        }
        let header = std::fs::read_to_string(crate_dir.join("include/yakvc.h")).unwrap();
        let declared: std::collections::BTreeSet<String> = header
            .match_indices("yakvc_")
            .map(|(at, _)| identifier(&header[at..]))
            .filter(|name| header.contains(&format!("{name}(")))
            .collect();
        assert!(!exported.is_empty());
        assert_eq!(exported, declared, "run `cargo xtask header`");
    }

    /// The identifier `text` starts with.
    fn identifier(text: &str) -> String {
        text.chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect()
    }
}
