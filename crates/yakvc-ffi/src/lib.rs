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
//! | [`YAKVC_EVENT_RENDEZVOUS_STATE`] | `u8 YAKVC_RDV_*`, `u64 value`: expiry (Unix s) when registered, delay (ms) when retrying, else 0 |
//! | [`YAKVC_EVENT_PEER_STATE`] | `uuid[16]`, `u8 YAKVC_PEER_*` |
//! | [`YAKVC_EVENT_TALKING`] | `uuid[16]`, `u8` 1 talking / 0 stopped |
//! | [`YAKVC_EVENT_MIC_LEVEL`] | `f32` dBFS |
//! | [`YAKVC_EVENT_ERROR`] | message as UTF-8 |

#![allow(unsafe_code)]

mod events;
mod guard;

use std::sync::Mutex;
use std::sync::atomic::AtomicBool;

use yakvc_client::{
    Config, ConfigError, Engine, Input, JoinId, PeerAudio, Pose, StartError, Vec3, World,
};

use crate::events::EventQueue;
use crate::guard::{
    FfiError, buf_arg, guard, guard_poisonable, out_arg, slice_arg, str_arg, uuids_arg,
};

/// Version of this C ABI. Java refuses to use a library whose version differs
/// from the one it was built against. Bump on any incompatible change.
pub const YAKVC_ABI_VERSION: u32 = 1;

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
/// An earlier call panicked; only `yakvc_destroy` is still allowed.
pub const YAKVC_ERR_POISONED: i32 = -7;
/// Not implemented yet. Removed before release.
pub const YAKVC_ERR_UNIMPLEMENTED: i32 = -99;

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

/// Opaque engine handle.
#[derive(Debug)]
pub struct YakVcEngine {
    engine: Engine,
    events: Mutex<EventQueue>,
    /// Set when a call panics; every later call except `yakvc_destroy` fails.
    poisoned: AtomicBool,
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

/// Pushes one tick's world: `listener` is `x, y, z, yaw, pitch`; then `count`
/// tracked players as UUIDs and `x, y, z` triples.
///
/// # Safety
/// `engine` must be live; `listener` must point to 5 doubles, `uuids` to
/// `16 * count` bytes and `xyz` to `3 * count` doubles.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yakvc_push_world(
    engine: *mut YakVcEngine,
    listener: *const f64,
    uuids: *const u8,
    xyz: *const f64,
    count: usize,
) -> i32 {
    // SAFETY: the caller guarantees `engine` is live.
    unsafe {
        with_engine(engine, |e| {
            // SAFETY: the caller guarantees `listener` points to 5 doubles.
            let listener = slice_arg(listener, 5, "listener")?;
            // SAFETY: the caller guarantees `uuids` points to `16 * count` bytes.
            let uuids = uuids_arg(uuids, count, "uuids")?;
            let xyz_len = count
                .checked_mul(3)
                .ok_or_else(|| FfiError::invalid("count is too large"))?;
            // SAFETY: the caller guarantees `xyz` points to `3 * count` doubles.
            let xyz = slice_arg(xyz, xyz_len, "xyz")?;
            if !listener.iter().chain(xyz).all(|v| v.is_finite()) {
                return Err(FfiError::invalid("world contains NaN or infinity"));
            }

            let players = uuids
                .into_iter()
                .zip(xyz.as_chunks::<3>().0)
                .map(|(uuid, &[x, y, z])| (uuid, Vec3::new(x, y, z)))
                .collect();
            let listener = Pose {
                pos: Vec3::new(listener[0], listener[1], listener[2]),
                yaw: listener[3] as f32,
                pitch: listener[4] as f32,
            };
            e.engine.set_world(World { listener, players });
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

/// Copies this thread's last error message into `buf` (truncated to `cap`)
/// and returns its full length, or 0 if there is none.
///
/// # Safety
/// `buf` must be writable for `cap` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yakvc_last_error(buf: *mut u8, cap: usize) -> usize {
    let message = guard::last_error();
    // SAFETY: the caller guarantees `buf` is writable for `cap` bytes.
    if let Ok(buf) = unsafe { buf_arg(buf, cap, "buf") } {
        let n = message.len().min(buf.len());
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
    fn engine_handle_is_shareable_across_threads() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<YakVcEngine>();
    }
}
