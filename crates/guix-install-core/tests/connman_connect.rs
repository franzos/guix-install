use std::time::Duration;

use guix_install_core::connman::connect_with_deadline_pub as connect_with_deadline;
use zeroize::Zeroizing;

fn fake() -> &'static str {
    concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/connmanctl-fake.sh")
}

// These tests mutate process-global env, so the file is run with --test-threads=1.

#[test]
fn connect_succeeds_when_state_goes_online() {
    let dir = std::env::temp_dir().join("guix-install-test-connman");
    std::fs::create_dir_all(&dir).unwrap();
    // SAFETY: single-threaded test run (--test-threads=1); set before any connman call.
    unsafe {
        std::env::set_var("GUIX_INSTALL_CONNMANCTL", fake());
        std::env::set_var("GUIX_INSTALL_CONNMAN_DIR", &dir);
        std::env::set_var("FAKE_STATE", "online");
    }
    let pw = Zeroizing::new("hunter2".to_string());
    let r = connect_with_deadline("wifi_x_managed_psk", "Net", Some(&pw), Duration::from_secs(2));
    assert!(r.is_ok(), "expected success, got {r:?}");
    // provisioning file must be cleaned up
    assert!(!dir.join("guix-install.config").exists());
}

#[test]
fn connect_times_out_when_never_connected() {
    unsafe {
        std::env::set_var("GUIX_INSTALL_CONNMANCTL", fake());
        std::env::set_var("FAKE_STATE", "idle");
    }
    let r = connect_with_deadline("wifi_x_managed_psk", "Net", None, Duration::from_millis(1500));
    assert!(r.is_err());
}

#[test]
fn connect_rejects_passphrase_with_newline() {
    unsafe {
        std::env::set_var("GUIX_INSTALL_CONNMANCTL", fake());
        std::env::set_var(
            "GUIX_INSTALL_CONNMAN_DIR",
            std::env::temp_dir().join("guix-install-test-connman2"),
        );
    }
    let pw = zeroize::Zeroizing::new("bad\npass".to_string());
    let r = connect_with_deadline(
        "wifi_x_managed_psk",
        "Net",
        Some(&pw),
        std::time::Duration::from_secs(2),
    );
    assert!(r.is_err());
}
