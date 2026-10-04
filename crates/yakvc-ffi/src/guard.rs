//! The panic barrier, error codes and argument checks shared by every export.

use std::cell::RefCell;
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::{YAKVC_ERR_INVALID_ARGUMENT, YAKVC_ERR_PANIC, YAKVC_ERR_POISONED, YAKVC_OK};

/// A failed call: the code returned to Java and the message kept for
/// `yakvc_last_error`.
#[derive(Debug)]
pub(crate) struct FfiError {
    pub code: i32,
    pub message: String,
}

impl FfiError {
    pub fn new(code: i32, message: impl Into<String>) -> Self {
        FfiError {
            code,
            message: message.into(),
        }
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        FfiError::new(YAKVC_ERR_INVALID_ARGUMENT, message)
    }
}

thread_local! {
    static LAST_ERROR: RefCell<String> = const { RefCell::new(String::new()) };
}

/// This thread's last error message, or an empty string.
pub(crate) fn last_error() -> String {
    LAST_ERROR.with(|last| last.borrow().clone())
}

fn fail(error: FfiError) -> i32 {
    LAST_ERROR.with(|last| *last.borrow_mut() = error.message);
    error.code
}

/// Runs `f` and returns its result as a code. A panic must never unwind into
/// Java (it would abort the game), so it is caught and reported as
/// [`YAKVC_ERR_PANIC`].
pub(crate) fn guard(f: impl FnOnce() -> Result<(), FfiError>) -> i32 {
    // Unwind safety: after a panic the engine is marked poisoned and only
    // dropped, so no half-updated state is observed.
    match panic::catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(())) => YAKVC_OK,
        Ok(Err(error)) => fail(error),
        Err(payload) => {
            let message = payload
                .downcast_ref::<&str>()
                .copied()
                .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
                .unwrap_or("unknown panic");
            fail(FfiError::new(
                YAKVC_ERR_PANIC,
                format!("engine panicked: {message}"),
            ))
        }
    }
}

/// Like [`guard`], but for an engine: refuses to run once `poisoned` is set,
/// and sets it if `f` panics.
pub(crate) fn guard_poisonable(
    poisoned: &AtomicBool,
    f: impl FnOnce() -> Result<(), FfiError>,
) -> i32 {
    if poisoned.load(Ordering::Acquire) {
        return fail(FfiError::new(
            YAKVC_ERR_POISONED,
            "an earlier call panicked; the engine can only be destroyed",
        ));
    }
    let code = guard(f);
    if code == YAKVC_ERR_PANIC {
        poisoned.store(true, Ordering::Release);
    }
    code
}

/// Borrows `len` elements from Java. A null pointer is fine when `len` is 0.
///
/// # Safety
/// If `ptr` is non-null it must be readable for `len` elements of `T` for the
/// rest of the call.
pub(crate) unsafe fn slice_arg<'a, T>(
    ptr: *const T,
    len: usize,
    what: &str,
) -> Result<&'a [T], FfiError> {
    if len == 0 {
        return Ok(&[]);
    }
    if ptr.is_null() {
        return Err(FfiError::invalid(format!("{what} is null")));
    }
    if !ptr.is_aligned() {
        return Err(FfiError::invalid(format!("{what} is misaligned")));
    }
    // SAFETY: non-null and aligned (checked above), readable for `len`
    // elements (caller's contract).
    Ok(unsafe { std::slice::from_raw_parts(ptr, len) })
}

/// Borrows a writable byte buffer from Java. A null pointer is fine when
/// `cap` is 0.
///
/// # Safety
/// If `ptr` is non-null it must be writable for `cap` bytes for the rest of
/// the call.
pub(crate) unsafe fn buf_arg<'a>(
    ptr: *mut u8,
    cap: usize,
    what: &str,
) -> Result<&'a mut [u8], FfiError> {
    if cap == 0 {
        return Ok(&mut []);
    }
    if ptr.is_null() {
        return Err(FfiError::invalid(format!("{what} is null")));
    }
    // SAFETY: non-null, `u8` is always aligned, writable for `cap` bytes
    // (caller's contract).
    Ok(unsafe { std::slice::from_raw_parts_mut(ptr, cap) })
}

/// Borrows a single out-parameter.
///
/// # Safety
/// If `ptr` is non-null it must be writable for one `T`.
pub(crate) unsafe fn out_arg<'a, T>(ptr: *mut T, what: &str) -> Result<&'a mut T, FfiError> {
    if !ptr.is_aligned() {
        return Err(FfiError::invalid(format!("{what} is misaligned")));
    }
    // SAFETY: aligned (checked above); null is handled by `as_mut`; writable
    // otherwise (caller's contract).
    unsafe { ptr.as_mut() }.ok_or_else(|| FfiError::invalid(format!("{what} is null")))
}

/// Borrows a UTF-8 string from Java.
///
/// # Safety
/// As [`slice_arg`].
pub(crate) unsafe fn str_arg<'a>(
    ptr: *const u8,
    len: usize,
    what: &str,
) -> Result<&'a str, FfiError> {
    // SAFETY: forwarded from the caller.
    let bytes = unsafe { slice_arg(ptr, len, what) }?;
    std::str::from_utf8(bytes).map_err(|_| FfiError::invalid(format!("{what} is not UTF-8")))
}

/// Reads `count` 16-byte big-endian UUIDs.
///
/// # Safety
/// As [`slice_arg`], for `16 * count` bytes.
pub(crate) unsafe fn uuids_arg(
    ptr: *const u8,
    count: usize,
    what: &str,
) -> Result<Vec<yakvc_client::Uuid>, FfiError> {
    let len = count
        .checked_mul(16)
        .ok_or_else(|| FfiError::invalid(format!("{what}: count is too large")))?;
    // SAFETY: forwarded from the caller.
    let bytes = unsafe { slice_arg(ptr, len, what) }?;
    Ok(bytes
        .as_chunks::<16>()
        .0
        .iter()
        .map(|bytes| yakvc_client::Uuid::from_bytes(*bytes))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::YAKVC_ERR_CONFIG;

    #[test]
    fn guard_returns_ok_and_error_codes() {
        assert_eq!(guard(|| Ok(())), YAKVC_OK);
        let code = guard(|| Err(FfiError::new(YAKVC_ERR_CONFIG, "bad toml")));
        assert_eq!(code, YAKVC_ERR_CONFIG);
        assert_eq!(last_error(), "bad toml");
    }

    #[test]
    fn guard_catches_panics() {
        assert_eq!(guard(|| panic!("boom")), YAKVC_ERR_PANIC);
        assert_eq!(last_error(), "engine panicked: boom");

        let code = guard(|| panic!("{} formatted", 42));
        assert_eq!(code, YAKVC_ERR_PANIC);
        assert_eq!(last_error(), "engine panicked: 42 formatted");
    }

    #[test]
    fn panic_poisons_and_later_calls_do_not_run() {
        let poisoned = AtomicBool::new(false);
        assert_eq!(guard_poisonable(&poisoned, || Ok(())), YAKVC_OK);

        // Errors don't poison.
        let code = guard_poisonable(&poisoned, || Err(FfiError::invalid("nope")));
        assert_eq!(code, YAKVC_ERR_INVALID_ARGUMENT);
        assert!(!poisoned.load(Ordering::Acquire));

        assert_eq!(
            guard_poisonable(&poisoned, || panic!("boom")),
            YAKVC_ERR_PANIC
        );
        assert!(poisoned.load(Ordering::Acquire));

        let mut ran = false;
        let code = guard_poisonable(&poisoned, || {
            ran = true;
            Ok(())
        });
        assert_eq!(code, YAKVC_ERR_POISONED);
        assert!(!ran);
    }

    #[test]
    fn slice_arg_accepts_null_only_when_empty() {
        // SAFETY: null with length 0 is never read.
        assert!(unsafe { slice_arg::<u8>(std::ptr::null(), 0, "x") }.is_ok());
        // SAFETY: null is rejected before reading.
        let err = unsafe { slice_arg::<u8>(std::ptr::null(), 1, "x") }.unwrap_err();
        assert_eq!(err.code, YAKVC_ERR_INVALID_ARGUMENT);
    }

    #[test]
    fn slice_arg_rejects_misaligned_pointers() {
        let doubles = [0.0f64; 2];
        let misaligned = doubles.as_ptr().cast::<u8>().wrapping_add(1).cast::<f64>();
        // SAFETY: the misaligned pointer is rejected before reading.
        let err = unsafe { slice_arg(misaligned, 1, "xyz") }.unwrap_err();
        assert_eq!(err.message, "xyz is misaligned");
    }

    #[test]
    fn str_arg_rejects_invalid_utf8() {
        let bytes = [0xff, 0xfe];
        // SAFETY: `bytes` is readable for its length.
        let err = unsafe { str_arg(bytes.as_ptr(), bytes.len(), "name") }.unwrap_err();
        assert_eq!(err.message, "name is not UTF-8");
    }

    #[test]
    fn uuids_arg_reads_big_endian_uuids() {
        let mut bytes = [0u8; 32];
        bytes[15] = 1;
        bytes[16] = 0xab;
        // SAFETY: `bytes` is readable for 2 UUIDs.
        let uuids = unsafe { uuids_arg(bytes.as_ptr(), 2, "uuids") }.unwrap();
        assert_eq!(uuids[0].as_u128(), 1);
        assert_eq!(uuids[1].as_u128(), 0xab << 120);
    }

    #[test]
    fn uuids_arg_rejects_overflowing_counts() {
        let byte = 0u8;
        // SAFETY: the count overflows and is rejected before reading.
        let err = unsafe { uuids_arg(&byte, usize::MAX, "uuids") }.unwrap_err();
        assert_eq!(err.code, YAKVC_ERR_INVALID_ARGUMENT);
    }
}
