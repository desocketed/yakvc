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
    _p: (),
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
    let _ = (config_dir, dir_len, config_toml, toml_len, abi_version, out);
    YAKVC_ERR_UNIMPLEMENTED
}

/// Shuts down gracefully (at most 500 ms) and frees the handle. Must be the
/// last call on `engine`. Null is ignored.
///
/// # Safety
/// `engine` must come from `yakvc_create` and not be used afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yakvc_destroy(engine: *mut YakVcEngine) {
    let _ = engine;
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
    let _ = (engine, uuid, name, name_len);
    YAKVC_ERR_UNIMPLEMENTED
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
    let _ = (engine, uuids, count);
    YAKVC_ERR_UNIMPLEMENTED
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
    let _ = (engine, listener, uuids, xyz, count);
    YAKVC_ERR_UNIMPLEMENTED
}

/// `flags` is a combination of `YAKVC_INPUT_*`.
///
/// # Safety
/// `engine` must be live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yakvc_set_input(engine: *mut YakVcEngine, flags: u32) -> i32 {
    let _ = (engine, flags);
    YAKVC_ERR_UNIMPLEMENTED
}

/// The game's Voice/Speech slider, `0..1`.
///
/// # Safety
/// `engine` must be live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yakvc_set_game_volume(engine: *mut YakVcEngine, volume: f32) -> i32 {
    let _ = (engine, volume);
    YAKVC_ERR_UNIMPLEMENTED
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
    let _ = (engine, uuid, volume, muted);
    YAKVC_ERR_UNIMPLEMENTED
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
    let _ = (engine, request_id, ok);
    YAKVC_ERR_UNIMPLEMENTED
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
    let _ = (engine, buf, cap, written);
    YAKVC_ERR_UNIMPLEMENTED
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
    let _ = (engine, toml, len);
    YAKVC_ERR_UNIMPLEMENTED
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
    let _ = (engine, buf, cap, needed);
    YAKVC_ERR_UNIMPLEMENTED
}

/// Copies this thread's last error message into `buf` (truncated to `cap`)
/// and returns its full length, or 0 if there is none.
///
/// # Safety
/// `buf` must be writable for `cap` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn yakvc_last_error(buf: *mut u8, cap: usize) -> usize {
    let _ = (buf, cap);
    0
}
