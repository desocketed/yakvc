#[test]
fn abi_version_matches_constant() {
    assert_eq!(yakvc_ffi::yakvc_abi_version(), yakvc_ffi::YAKVC_ABI_VERSION);
}
