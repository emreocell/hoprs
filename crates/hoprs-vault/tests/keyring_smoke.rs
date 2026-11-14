//! Smoke test of the platform keyring the vault uses on Windows (Credential
//! Manager) and macOS (Keychain): store, read back, delete. Linux CI runners
//! have no Secret Service, so on Linux this file compiles to nothing.

#![cfg(any(windows, target_os = "macos"))]

use std::time::{SystemTime, UNIX_EPOCH};

const SERVICE: &str = "hoprs-ci-smoke";

/// An account name no other run (or a leftover from a crashed one) shares.
fn random_account() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after 1970")
        .as_nanos();
    format!("smoke-{}-{nanos}", std::process::id())
}

#[test]
fn store_read_delete_round_trip() {
    let account = random_account();
    let entry = keyring::Entry::new(SERVICE, &account).expect("build entry");

    entry.set_password("s3cret value").expect("store");
    assert_eq!(entry.get_password().expect("read back"), "s3cret value");

    entry.delete_credential().expect("delete");
    match entry.get_password() {
        Err(keyring::Error::NoEntry) => {}
        other => panic!("expected NoEntry after delete, got {other:?}"),
    }
}
