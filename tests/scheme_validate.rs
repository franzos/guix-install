//! Tier-2 validation: pipe each rendered system.scm through Guix's evaluator.
//!
//! This catches drift in the modules and records we depend on (renamed fields,
//! removed services, shepherd dependency mismatches) by running
//! `guix time-machine --channels=…/channels.scm -- system -d …/system.scm`.
//!
//! `system -d` does the full module load + gexp lowering + `assert-valid-graph`
//! shepherd check, but stops short of building store items. Runtime safety
//! checks (mapped-devices, file-system availability) are gated to init/reconfigure
//! only, so `/dev/sda` not existing on the host is fine.
//!
//! Caveats:
//! - This validates that the rendered config builds; it does not validate that
//!   the resulting system boots, that activation succeeds, or that GRUB installs.
//! - For Guix mode (no channels.scm), uses the host's `guix` directly.
//! - Enterprise mode is skipped — its system.scm comes from a remote tarball.
//!
//! Run with: `guix shell guix -- cargo test --test scheme_validate -- --ignored`
//! First Panther/Nonguix run pulls channels via time-machine (slow); subsequent
//! runs are cached.
//!
//! The Guix invocation is `guix [time-machine ...] system build -d <file>`:
//! `build -d` (alias for `--derivation`) computes and prints the derivation
//! without realizing it. That triggers the full module load + gexp lowering +
//! `assert-valid-graph` shepherd check — which is what catches drift — but
//! skips the actual store build.

use guix_install::config::{
    BlockDevice, DesktopEnvironment, EncryptionConfig, Filesystem, Firmware, SystemConfig,
    UserAccount,
};
use guix_install::mode::InstallMode;
use guix_install::scheme::channels::render_channels;
use guix_install::scheme::operating_system::render_operating_system;

use std::process::Command;

fn base_config() -> SystemConfig {
    SystemConfig {
        mode: InstallMode::Panther,
        firmware: Firmware::Efi,
        hostname: "test-host".into(),
        timezone: "Europe/Berlin".into(),
        locale: "en_US.utf8".into(),
        keyboard_layout: None,
        disk: BlockDevice {
            name: "sda".into(),
            dev_path: "/dev/sda".into(),
            size_bytes: 100_000_000_000,
            model: Some("Test Disk".into()),
            boot_partition_uuid: None,
            root_partition_uuid: None,
        },
        filesystem: Filesystem::Ext4,
        encryption: None,
        users: vec![UserAccount {
            name: "testuser".into(),
            comment: "testuser's account".into(),
            groups: vec!["wheel".into(), "audio".into(), "video".into()],
        }],
        desktop: None,
        ssh_key: None,
        swap_size_mb: 4096,
        password: None,
        system_scm_override: None,
    }
}

fn guix_available() -> bool {
    Command::new("guix")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Render `config` and run `guix system -d` (via time-machine when a channels.scm
/// is needed). Panics with stderr on failure.
fn validate(config: &SystemConfig) {
    if !guix_available() {
        eprintln!("guix not on PATH — skipping (run via `guix shell guix -- cargo test`)");
        return;
    }

    let system_scm = render_operating_system(config);
    assert!(
        !system_scm.is_empty(),
        "render_operating_system returned empty for non-Enterprise mode"
    );

    let tmp = tempfile::tempdir().expect("tempdir");
    let system_path = tmp.path().join("system.scm");
    std::fs::write(&system_path, &system_scm).unwrap();

    let mut cmd = Command::new("guix");
    if let Some(channels) = render_channels(&config.mode) {
        let channels_path = tmp.path().join("channels.scm");
        std::fs::write(&channels_path, &channels).unwrap();
        // `--channels=PATH` (not space-separated) — Guile's argparse otherwise
        // treats the following `--` as end-of-options before the value lands.
        cmd.arg("time-machine")
            .arg(format!("--channels={}", channels_path.display()))
            .arg("--");
    }
    cmd.arg("system").arg("build").arg("-d").arg(&system_path);

    let output = cmd.output().expect("failed to spawn guix");
    if !output.status.success() {
        panic!(
            "guix rejected the rendered config\n\
             --- command: {cmd:?}\n\
             --- exit:    {}\n\
             --- stderr ---\n{}\n\
             --- system.scm ---\n{system_scm}",
            output.status,
            String::from_utf8_lossy(&output.stderr),
        );
    }
}

// One representative combo per axis, per mode. Each axis (firmware,
// filesystem, encryption, desktop) is exercised at least once.

#[test]
#[ignore = "requires guix; first run pulls channels via time-machine"]
fn guix_bios_ext4_headless() {
    let mut c = base_config();
    c.mode = InstallMode::Guix;
    c.firmware = Firmware::Bios;
    validate(&c);
}

#[test]
#[ignore = "requires guix"]
fn guix_efi_btrfs_luks_gnome() {
    let mut c = base_config();
    c.mode = InstallMode::Guix;
    c.firmware = Firmware::Efi;
    c.filesystem = Filesystem::Btrfs;
    c.encryption = Some(EncryptionConfig {
        device_target: "cryptroot".into(),
    });
    c.desktop = Some(DesktopEnvironment::Gnome);
    validate(&c);
}

#[test]
#[ignore = "requires guix; first run pulls nonguix channel"]
fn nonguix_bios_ext4_luks_headless() {
    let mut c = base_config();
    c.mode = InstallMode::Nonguix;
    c.firmware = Firmware::Bios;
    c.encryption = Some(EncryptionConfig {
        device_target: "cryptroot".into(),
    });
    validate(&c);
}

#[test]
#[ignore = "requires guix; first run pulls nonguix channel"]
fn nonguix_efi_btrfs_gnome() {
    let mut c = base_config();
    c.mode = InstallMode::Nonguix;
    c.firmware = Firmware::Efi;
    c.filesystem = Filesystem::Btrfs;
    c.desktop = Some(DesktopEnvironment::Gnome);
    validate(&c);
}

#[test]
#[ignore = "requires guix; first run pulls panther channel"]
fn panther_bios_ext4_headless() {
    let mut c = base_config();
    c.mode = InstallMode::Panther;
    c.firmware = Firmware::Bios;
    validate(&c);
}

#[test]
#[ignore = "requires guix; first run pulls panther channel"]
fn panther_efi_btrfs_luks_gnome() {
    let mut c = base_config();
    c.mode = InstallMode::Panther;
    c.firmware = Firmware::Efi;
    c.filesystem = Filesystem::Btrfs;
    c.encryption = Some(EncryptionConfig {
        device_target: "cryptroot".into(),
    });
    c.desktop = Some(DesktopEnvironment::Gnome);
    validate(&c);
}
