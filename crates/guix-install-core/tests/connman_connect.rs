use std::sync::Mutex;
use std::time::Duration;

use guix_install_core::connman::connect_with_deadline_pub as connect_with_deadline;
use zeroize::Zeroizing;

// These tests mutate process-global env, so they must not run concurrently.
static ENV_LOCK: Mutex<()> = Mutex::new(());

fn fake() -> &'static str {
    concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/connmanctl-fake.sh"
    )
}

#[test]
fn connect_succeeds_when_service_goes_online() {
    let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::temp_dir().join("guix-install-test-connman");
    std::fs::create_dir_all(&dir).unwrap();
    // SAFETY: ENV_LOCK serializes these tests; set before any connman call.
    unsafe {
        std::env::set_var("GUIX_INSTALL_CONNMANCTL", fake());
        std::env::set_var("GUIX_INSTALL_CONNMAN_DIR", &dir);
        std::env::set_var("FAKE_SERVICE_STATE", "online");
        std::env::remove_var("FAKE_STATE");
        std::env::remove_var("FAKE_SERVICE_ERROR");
        std::env::remove_var("FAKE_CONNECT_EXIT");
    }
    let pw = Zeroizing::new("hunter2".to_string());
    let r = connect_with_deadline(
        "wifi_x_managed_psk",
        "Net",
        Some(&pw),
        Duration::from_secs(2),
    );
    assert!(r.is_ok(), "expected success, got {r:?}");
    // provisioning file must persist: deleting it makes connman disconnect and
    // wipe credentials, killing the link mid-install.
    assert!(dir.join("guix-install.config").exists());
}

#[test]
fn connect_does_not_trust_global_state() {
    let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // Global state is online (a second adapter is connected) but the chosen
    // service never leaves idle: must NOT report success.
    unsafe {
        std::env::set_var("GUIX_INSTALL_CONNMANCTL", fake());
        std::env::set_var("FAKE_STATE", "online");
        std::env::set_var("FAKE_SERVICE_STATE", "idle");
        std::env::remove_var("FAKE_SERVICE_ERROR");
        std::env::remove_var("FAKE_CONNECT_EXIT");
    }
    let r = connect_with_deadline(
        "wifi_x_managed_psk",
        "Net",
        None,
        Duration::from_millis(1200),
    );
    assert!(r.is_err(), "must not trust global state, got {r:?}");
}

#[test]
fn connect_bails_on_invalid_key() {
    let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        std::env::set_var("GUIX_INSTALL_CONNMANCTL", fake());
        std::env::set_var("FAKE_SERVICE_STATE", "failure");
        std::env::set_var("FAKE_SERVICE_ERROR", "invalid-key");
        std::env::remove_var("FAKE_STATE");
        std::env::remove_var("FAKE_CONNECT_EXIT");
    }
    let r = connect_with_deadline("wifi_x_managed_psk", "Net", None, Duration::from_secs(2));
    unsafe {
        std::env::remove_var("FAKE_SERVICE_STATE");
        std::env::remove_var("FAKE_SERVICE_ERROR");
    }
    let e = r.expect_err("invalid-key must bail");
    assert!(
        e.to_string().contains("passphrase"),
        "message should mention passphrase, got: {e}"
    );
}

#[test]
fn connect_times_out_when_never_connected() {
    let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        std::env::set_var("GUIX_INSTALL_CONNMANCTL", fake());
        std::env::set_var("FAKE_SERVICE_STATE", "idle");
        std::env::remove_var("FAKE_STATE");
        std::env::remove_var("FAKE_SERVICE_ERROR");
        std::env::remove_var("FAKE_CONNECT_EXIT");
    }
    let r = connect_with_deadline(
        "wifi_x_managed_psk",
        "Net",
        None,
        Duration::from_millis(1500),
    );
    assert!(r.is_err());
}

#[test]
fn connect_errors_when_connect_fails_and_never_ready() {
    let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        std::env::set_var("GUIX_INSTALL_CONNMANCTL", fake());
        std::env::set_var("FAKE_SERVICE_STATE", "idle");
        std::env::set_var("FAKE_CONNECT_EXIT", "1");
        std::env::remove_var("FAKE_STATE");
        std::env::remove_var("FAKE_SERVICE_ERROR");
    }
    let r = connect_with_deadline(
        "wifi_x_managed_psk",
        "Net",
        None,
        std::time::Duration::from_millis(800),
    );
    unsafe {
        std::env::remove_var("FAKE_CONNECT_EXIT");
    }
    assert!(r.is_err());
}

#[test]
fn connect_rejects_passphrase_with_newline() {
    let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
