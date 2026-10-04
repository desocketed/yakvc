//! C ABI over `yakvc-client`, called from Java through the FFM API.
//!
//! This is the only crate allowed to use `unsafe`. Run `cargo xtask header`
//! after changing any export to regenerate `include/yakvc.h`.

#![allow(unsafe_code)]

/// Version of this C ABI. Java refuses to use a library whose version differs
/// from the one it was built against. Bump on any incompatible change.
pub const YAKVC_ABI_VERSION: u32 = 1;

/// Returns [`YAKVC_ABI_VERSION`]. Safe to call before anything else.
#[unsafe(no_mangle)]
pub extern "C" fn yakvc_abi_version() -> u32 {
    YAKVC_ABI_VERSION
}
