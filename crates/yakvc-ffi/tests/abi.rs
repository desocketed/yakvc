//! Calls the C ABI the way Java does, without a JVM.

#![allow(unsafe_code)]

use std::ptr;
use std::time::{Duration, Instant};

use yakvc_ffi::*;

/// What Java does after a failed call.
fn last_error() -> String {
    // SAFETY: a null buffer with capacity 0 is never written.
    let len = unsafe { yakvc_last_error(ptr::null_mut(), 0) };
    let mut buf = vec![0u8; len];
    // SAFETY: `buf` is writable for its length.
    unsafe { yakvc_last_error(buf.as_mut_ptr(), buf.len()) };
    String::from_utf8(buf).unwrap()
}

/// Calls `yakvc_create` with `dir` and `toml`.
fn create(dir: &[u8], toml: &[u8], abi_version: u32) -> (i32, *mut YakVcEngine) {
    let mut engine = ptr::null_mut();
    // SAFETY: both slices are readable for their lengths and `engine` is
    // writable.
    let code = unsafe {
        yakvc_create(
            dir.as_ptr(),
            dir.len(),
            toml.as_ptr(),
            toml.len(),
            abi_version,
            &mut engine,
        )
    };
    (code, engine)
}

#[test]
fn abi_version_matches_constant() {
    assert_eq!(yakvc_abi_version(), YAKVC_ABI_VERSION);
}

#[test]
fn create_rejects_another_abi_version() {
    let (code, engine) = create(b"/tmp", b"", YAKVC_ABI_VERSION + 1);
    assert_eq!(code, YAKVC_ERR_ABI_MISMATCH);
    assert!(engine.is_null());
    assert!(last_error().contains("ABI"), "{}", last_error());
}

#[test]
fn create_rejects_bad_arguments() {
    let (code, engine) = create(&[0xff], b"", YAKVC_ABI_VERSION);
    assert_eq!(code, YAKVC_ERR_INVALID_ARGUMENT);
    assert!(engine.is_null());
    assert_eq!(last_error(), "config_dir is not UTF-8");

    // SAFETY: the null pointers are rejected before anything is read.
    let code = unsafe {
        yakvc_create(
            ptr::null(),
            0,
            ptr::null(),
            0,
            YAKVC_ABI_VERSION,
            ptr::null_mut(),
        )
    };
    assert_eq!(code, YAKVC_ERR_INVALID_ARGUMENT);
    assert_eq!(last_error(), "out is null");

    let mut engine = ptr::null_mut();
    // SAFETY: the null `config_dir` with a non-zero length is rejected before
    // anything is read.
    let code = unsafe {
        yakvc_create(
            ptr::null(),
            4,
            ptr::null(),
            0,
            YAKVC_ABI_VERSION,
            &mut engine,
        )
    };
    assert_eq!(code, YAKVC_ERR_INVALID_ARGUMENT);
    assert_eq!(last_error(), "config_dir is null");
}

#[test]
fn every_engine_call_rejects_a_null_engine() {
    let null = ptr::null_mut();
    let uuid = [0u8; 16];
    let listener = [0.0f64; 5];
    let mut buf = [0u8; 16];
    let mut len = 0usize;

    // SAFETY: every call is rejected on the null engine before any other
    // argument is used; the other arguments are valid anyway.
    let codes = unsafe {
        [
            yakvc_set_identity(null, uuid.as_ptr(), b"alice".as_ptr(), 5),
            yakvc_set_tab_list(null, uuid.as_ptr(), 1),
            yakvc_push_world(null, listener.as_ptr(), ptr::null(), ptr::null(), 0),
            yakvc_set_input(null, 0),
            yakvc_set_game_volume(null, 1.0),
            yakvc_set_game_device(null, ptr::null(), 0),
            yakvc_set_peer_volume(null, uuid.as_ptr(), 1.0, false),
            yakvc_complete_join(null, 1, true),
            yakvc_poll_events(null, buf.as_mut_ptr(), buf.len(), &mut len),
            yakvc_update_config(null, ptr::null(), 0),
            yakvc_list_devices(null, buf.as_mut_ptr(), buf.len(), &mut len),
            yakvc_stats(null, buf.as_mut_ptr(), buf.len(), &mut len),
            yakvc_join_group(null, b"g".as_ptr(), 1, ptr::null(), 0),
            yakvc_leave_group(null),
            yakvc_set_group_nearby(null, false),
            yakvc_list_groups(null, buf.as_mut_ptr(), buf.len(), &mut len),
        ]
    };
    for code in codes {
        assert_eq!(code, YAKVC_ERR_INVALID_ARGUMENT);
    }
    assert_eq!(last_error(), "engine is null");
}

#[test]
fn destroy_ignores_null() {
    // SAFETY: null is documented as ignored.
    unsafe { yakvc_destroy(ptr::null_mut()) };
}

#[test]
fn last_error_truncates_and_reports_the_full_length() {
    create(b"/tmp", b"", 0);
    let full = last_error();

    let mut buf = [0u8; 6];
    // SAFETY: `buf` is writable for its length.
    let len = unsafe { yakvc_last_error(buf.as_mut_ptr(), buf.len()) };
    assert_eq!(len, full.len());
    assert_eq!(&buf, &full.as_bytes()[..6]);
}

#[test]
fn last_error_is_per_thread() {
    create(b"/tmp", b"", 0);
    assert!(!last_error().is_empty());

    let other = std::thread::spawn(last_error).join().unwrap();
    assert_eq!(other, "");
}

/// A dev config whose rendezvous is unreachable, so the engine starts and
/// keeps trying without touching the real network.
fn offline_config() -> String {
    let config = yakvc_client::Config {
        dev_mode: true,
        rendezvous: Some(yakvc_client::RendezvousConfig {
            endpoint_id: yakvc_shared::SecretKey::generate().public(),
            addrs: vec!["127.0.0.1:9".parse().unwrap()],
            relay: None,
        }),
        trusted_issuers: vec![yakvc_shared::IssuerKey::generate().id()],
        ..Default::default()
    };
    toml::to_string(&config).unwrap()
}

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("yakvc-ffi-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn create_rejects_bad_toml() {
    let dir = temp_dir("bad-toml");
    let (code, engine) = create(dir.to_str().unwrap().as_bytes(), b"[[[", YAKVC_ABI_VERSION);
    assert_eq!(code, YAKVC_ERR_CONFIG);
    assert!(engine.is_null());
}

#[test]
fn engine_lifecycle_through_the_c_abi() {
    let dir = temp_dir("lifecycle");
    let toml = offline_config();
    let (code, engine) = create(
        dir.to_str().unwrap().as_bytes(),
        toml.as_bytes(),
        YAKVC_ABI_VERSION,
    );
    assert_eq!(code, YAKVC_OK, "{}", last_error());
    assert!(!engine.is_null());

    let me = [1u8; 16];
    let other = [2u8; 16];
    let listener = [0.0, 64.0, 0.0, 90.0, 0.0];
    let xyz = [3.0, 64.0, 4.0];
    // SAFETY: `engine` is live and every buffer is valid for its length.
    unsafe {
        assert_eq!(
            yakvc_set_identity(engine, me.as_ptr(), b"alice".as_ptr(), 5),
            YAKVC_OK
        );
        assert_eq!(yakvc_set_tab_list(engine, other.as_ptr(), 1), YAKVC_OK);
        assert_eq!(
            yakvc_push_world(engine, listener.as_ptr(), other.as_ptr(), xyz.as_ptr(), 1),
            YAKVC_OK
        );
        assert_eq!(
            yakvc_set_input(engine, YAKVC_INPUT_PUSH_TO_TALK | YAKVC_INPUT_MUTED),
            YAKVC_OK
        );
        assert_eq!(yakvc_set_game_volume(engine, 0.5), YAKVC_OK);
        assert_eq!(
            yakvc_set_peer_volume(engine, other.as_ptr(), 1.5, false),
            YAKVC_OK
        );
        assert_eq!(
            yakvc_update_config(engine, toml.as_ptr(), toml.len()),
            YAKVC_OK
        );
    }

    // The engine reports its rendezvous state once it starts connecting.
    let mut buf = vec![0u8; 64 * 1024];
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut saw_rendezvous = false;
    while !saw_rendezvous && Instant::now() < deadline {
        let mut written = 0;
        // SAFETY: `engine` is live; `buf` and `written` are writable.
        let code = unsafe { yakvc_poll_events(engine, buf.as_mut_ptr(), buf.len(), &mut written) };
        assert_eq!(code, YAKVC_OK, "{}", last_error());
        let mut rest = &buf[..written];
        while !rest.is_empty() {
            let kind = u16::from_le_bytes([rest[0], rest[1]]);
            let len = usize::from(u16::from_le_bytes([rest[2], rest[3]]));
            saw_rendezvous |= kind == YAKVC_EVENT_RENDEZVOUS_STATE;
            rest = &rest[4 + len..];
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(saw_rendezvous);

    // Device list: ask for the size, then retry with a big enough buffer.
    let mut needed = 0;
    // SAFETY: `engine` is live; a null buffer with capacity 0 is not written.
    let code = unsafe { yakvc_list_devices(engine, ptr::null_mut(), 0, &mut needed) };
    if code == YAKVC_OK {
        let mut json = vec![0u8; needed];
        // SAFETY: `json` is writable for `needed` bytes.
        let code =
            unsafe { yakvc_list_devices(engine, json.as_mut_ptr(), json.len(), &mut needed) };
        assert_eq!(code, YAKVC_OK);
        assert_eq!(needed, json.len());
        let devices: serde_json::Value = serde_json::from_slice(&json).unwrap();
        assert!(devices["inputs"].is_array());
    } else {
        // Machines without sound hardware, like CI.
        assert_eq!(code, YAKVC_ERR_AUDIO);
    }

    // Stats snapshot, same buffer protocol.
    // SAFETY: `engine` is live; a null buffer with capacity 0 is not written.
    let code = unsafe { yakvc_stats(engine, ptr::null_mut(), 0, &mut needed) };
    assert_eq!(code, YAKVC_OK);
    let mut json = vec![0u8; needed];
    // SAFETY: `json` is writable for `needed` bytes.
    let code = unsafe { yakvc_stats(engine, json.as_mut_ptr(), json.len(), &mut needed) };
    assert_eq!(code, YAKVC_OK);
    let stats: serde_json::Value = serde_json::from_slice(&json).unwrap();
    assert_eq!(stats["endpoint_id"].as_str().map(str::len), Some(64));
    assert!(stats["audio"]["bitrate"].is_u64());
    assert!(stats["peers"].is_array());

    let start = Instant::now();
    // SAFETY: `engine` is live and not used afterwards.
    unsafe { yakvc_destroy(engine) };
    // 500 ms graceful close plus slack for slow CI.
    assert!(start.elapsed() < Duration::from_secs(2));
}

#[test]
fn engine_calls_reject_invalid_values() {
    let dir = temp_dir("invalid");
    let toml = offline_config();
    let (code, engine) = create(
        dir.to_str().unwrap().as_bytes(),
        toml.as_bytes(),
        YAKVC_ABI_VERSION,
    );
    assert_eq!(code, YAKVC_OK, "{}", last_error());

    let uuid = [1u8; 16];
    let nan_listener = [0.0, f64::NAN, 0.0, 0.0, 0.0];
    // SAFETY: `engine` is live and every buffer is valid for its length.
    unsafe {
        assert_eq!(yakvc_set_input(engine, 1 << 31), YAKVC_ERR_INVALID_ARGUMENT);
        assert_eq!(
            yakvc_set_game_volume(engine, 1.5),
            YAKVC_ERR_INVALID_ARGUMENT
        );
        assert_eq!(
            yakvc_set_peer_volume(engine, uuid.as_ptr(), f32::NAN, false),
            YAKVC_ERR_INVALID_ARGUMENT
        );
        assert_eq!(
            yakvc_push_world(engine, nan_listener.as_ptr(), ptr::null(), ptr::null(), 0),
            YAKVC_ERR_INVALID_ARGUMENT
        );
        assert_eq!(
            yakvc_set_tab_list(engine, ptr::null(), 3),
            YAKVC_ERR_INVALID_ARGUMENT
        );
        assert_eq!(
            yakvc_set_game_device(engine, [0xff].as_ptr(), 1),
            YAKVC_ERR_INVALID_ARGUMENT
        );
        assert_eq!(
            yakvc_update_config(engine, b"[[[".as_ptr(), 3),
            YAKVC_ERR_CONFIG
        );
        // Errors are not panics: the engine still works.
        assert_eq!(yakvc_set_input(engine, 0), YAKVC_OK);
        yakvc_destroy(engine);
    }
}

#[test]
fn game_device_takes_a_name_or_the_default() {
    let dir = temp_dir("game-device");
    let toml = offline_config();
    let (code, engine) = create(
        dir.to_str().unwrap().as_bytes(),
        toml.as_bytes(),
        YAKVC_ABI_VERSION,
    );
    assert_eq!(code, YAKVC_OK, "{}", last_error());

    let name = b"Built-in Audio Analog Stereo";
    // SAFETY: `engine` is live, `name` is readable for its length, and null
    // with length 0 is never read.
    unsafe {
        assert_eq!(
            yakvc_set_game_device(engine, name.as_ptr(), name.len()),
            YAKVC_OK,
            "{}",
            last_error()
        );
        // Both mean the system default.
        assert_eq!(yakvc_set_game_device(engine, ptr::null(), 0), YAKVC_OK);
        assert_eq!(yakvc_set_game_device(engine, name.as_ptr(), 0), YAKVC_OK);
        yakvc_destroy(engine);
    }
}

#[test]
fn groups_through_the_c_abi() {
    let dir = temp_dir("groups");
    let toml = offline_config();
    let (code, engine) = create(
        dir.to_str().unwrap().as_bytes(),
        toml.as_bytes(),
        YAKVC_ABI_VERSION,
    );
    assert_eq!(code, YAKVC_OK, "{}", last_error());
    let uuid = [7u8; 16];
    let (name, password) = (b"Miners", b"pw");
    let mut id = [0u8; 64];
    let mut buf = vec![0u8; 4096];
    let mut len = 0usize;
    // SAFETY: `engine` is live and every buffer is valid for its length.
    unsafe {
        assert_eq!(
            yakvc_set_identity(engine, uuid.as_ptr(), b"alice".as_ptr(), 5),
            YAKVC_OK
        );
        let code = yakvc_join_group(engine, name.as_ptr(), 6, password.as_ptr(), 2);
        assert_eq!(code, YAKVC_OK, "{}", last_error());
        assert_eq!(yakvc_set_group_nearby(engine, false), YAKVC_OK);
        let code = yakvc_group_id(name.as_ptr(), 6, password.as_ptr(), 2, id.as_mut_ptr());
        assert_eq!(code, YAKVC_OK, "{}", last_error());
        let code = yakvc_list_groups(engine, buf.as_mut_ptr(), buf.len(), &mut len);
        assert_eq!(code, YAKVC_OK, "{}", last_error());

        let groups: serde_json::Value = serde_json::from_slice(&buf[..len]).unwrap();
        let group = &groups[0];
        assert_eq!(group["id"], std::str::from_utf8(&id).unwrap());
        assert_eq!(group["name"], "Miners");
        assert_eq!(group["locked"], true);
        assert_eq!(group["joined"], true);
        assert_eq!(group["members"][0], "07070707-0707-0707-0707-070707070707");

        let blank = b"  ";
        let code = yakvc_join_group(engine, blank.as_ptr(), 2, ptr::null(), 0);
        assert_eq!(code, YAKVC_ERR_INVALID_ARGUMENT);
        assert_eq!(yakvc_leave_group(engine), YAKVC_OK);
        yakvc_list_groups(engine, buf.as_mut_ptr(), buf.len(), &mut len);
        assert_eq!(&buf[..len], b"[]");
        yakvc_destroy(engine);
    }
}
