use guix_install::config::{
    BlockDevice, DesktopEnvironment, EncryptionConfig, Filesystem, Firmware, SystemConfig,
    UserAccount, generate_hostname, validate_config_id, validate_hostname, validate_username,
};
use guix_install::mode::InstallMode;
use guix_install::scheme::channels::render_channels;
use guix_install::scheme::operating_system::render_operating_system;

fn test_disk() -> BlockDevice {
    BlockDevice {
        name: "sda".into(),
        dev_path: "/dev/sda".into(),
        size_bytes: 100_000_000_000,
        model: Some("Test Disk".into()),
        boot_partition_uuid: None,
        root_partition_uuid: None,
    }
}

fn base_config() -> SystemConfig {
    SystemConfig {
        mode: InstallMode::Panther,
        firmware: Firmware::Efi,
        hostname: "test-host".into(),
        timezone: "Europe/Berlin".into(),
        locale: "en_US.utf8".into(),
        keyboard_layout: None,
        disk: test_disk(),
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
    }
}

fn assert_balanced_parens(s: &str) {
    let open: usize = s.chars().filter(|&c| c == '(').count();
    let close: usize = s.chars().filter(|&c| c == ')').count();
    assert_eq!(
        open, close,
        "Unbalanced parentheses: {open} open vs {close} close\n\nGenerated:\n{s}"
    );
}

fn assert_scheme_structure(output: &str) {
    assert!(
        output.contains("(use-modules"),
        "Missing (use-modules\n\n{output}"
    );
    assert!(
        output.contains("(operating-system"),
        "Missing (operating-system\n\n{output}"
    );
    assert_balanced_parens(output);
}

// --- Panther + EFI + no encryption + ext4 + Gnome ---

#[test]
fn panther_efi_plain_ext4_gnome() {
    let mut config = base_config();
    config.mode = InstallMode::Panther;
    config.firmware = Firmware::Efi;
    config.filesystem = Filesystem::Ext4;
    config.desktop = Some(DesktopEnvironment::Gnome);

    let output = render_operating_system(&config);
    assert_scheme_structure(&output);
    assert!(output.contains("(inherit %panther-desktop-os)"));
    assert!(output.contains("grub-efi-bootloader"));
    assert!(output.contains("(px system panther)"));
    assert!(output.contains("gnome-desktop-service-type"));
    assert!(output.contains("\"ext4\""));
    assert!(output.contains("/boot/efi"));
    assert!(!output.contains("luks-device-mapping"));
    assert!(!output.contains("mapped-devices"));
}

// --- Panther + BIOS + LUKS + ext4 + no desktop ---

#[test]
fn panther_bios_luks_ext4_headless() {
    let mut config = base_config();
    config.mode = InstallMode::Panther;
    config.firmware = Firmware::Bios;
    config.filesystem = Filesystem::Ext4;
    config.encryption = Some(EncryptionConfig {
        device_target: "cryptroot".into(),
    });
    config.desktop = None;

    let output = render_operating_system(&config);
    assert_scheme_structure(&output);
    assert!(output.contains("(inherit %panther-os)"));
    assert!(output.contains("grub-bootloader"));
    assert!(output.contains("luks-device-mapping"));
    assert!(output.contains("/dev/mapper/cryptroot"));
    assert!(output.contains("dependencies mapped-devices"));
    assert!(!output.contains("grub-efi-bootloader"));
    assert!(!output.contains("/boot/efi"));
}

// --- Nonguix + EFI + no encryption + ext4 + Xfce ---

#[test]
fn nonguix_efi_plain_ext4_xfce() {
    let mut config = base_config();
    config.mode = InstallMode::Nonguix;
    config.firmware = Firmware::Efi;
    config.filesystem = Filesystem::Ext4;
    config.desktop = Some(DesktopEnvironment::Xfce);

    let output = render_operating_system(&config);
    assert_scheme_structure(&output);
    assert!(output.contains("(kernel linux)"));
    assert!(output.contains("(initrd microcode-initrd)"));
    assert!(output.contains("(firmware (list linux-firmware))"));
    assert!(output.contains("(nongnu packages linux)"));
    assert!(output.contains("(nongnu system linux-initrd)"));
    assert!(output.contains("grub-efi-bootloader"));
    assert!(output.contains("xfce-desktop-service-type"));
    assert!(output.contains("%base-services"));
    assert!(output.contains("%base-packages"));
    assert!(!output.contains("panther"));
}

// --- Nonguix + BIOS + LUKS + btrfs + no desktop ---

#[test]
fn nonguix_bios_luks_btrfs_headless() {
    let mut config = base_config();
    config.mode = InstallMode::Nonguix;
    config.firmware = Firmware::Bios;
    config.filesystem = Filesystem::Btrfs;
    config.encryption = Some(EncryptionConfig {
        device_target: "cryptroot".into(),
    });
    config.desktop = None;

    let output = render_operating_system(&config);
    assert_scheme_structure(&output);
    assert!(output.contains("grub-bootloader"));
    assert!(output.contains("\"btrfs\""));
    assert!(output.contains("luks-device-mapping"));
    assert!(output.contains("/dev/mapper/cryptroot"));
    assert!(output.contains("(kernel linux)"));
    assert!(!output.contains("/boot/efi"));
}

// --- Guix + EFI + no encryption + ext4 + Gnome ---

#[test]
fn guix_efi_plain_ext4_gnome() {
    let mut config = base_config();
    config.mode = InstallMode::Guix;
    config.firmware = Firmware::Efi;
    config.filesystem = Filesystem::Ext4;
    config.desktop = Some(DesktopEnvironment::Gnome);

    let output = render_operating_system(&config);
    assert_scheme_structure(&output);
    assert!(output.contains("grub-efi-bootloader"));
    assert!(output.contains("gnome-desktop-service-type"));
    assert!(output.contains("%base-services"));
    assert!(output.contains("%base-packages"));
    assert!(!output.contains("kernel linux"));
    assert!(!output.contains("microcode-initrd"));
    assert!(!output.contains("panther"));
    assert!(!output.contains("nonguix"));
    assert!(!output.contains("nongnu"));
}

// --- Guix + BIOS + LUKS + ext4 + no desktop ---

#[test]
fn guix_bios_luks_ext4_headless() {
    let mut config = base_config();
    config.mode = InstallMode::Guix;
    config.firmware = Firmware::Bios;
    config.filesystem = Filesystem::Ext4;
    config.encryption = Some(EncryptionConfig {
        device_target: "cryptroot".into(),
    });
    config.desktop = None;

    let output = render_operating_system(&config);
    assert_scheme_structure(&output);
    assert!(output.contains("grub-bootloader"));
    assert!(output.contains("luks-device-mapping"));
    assert!(output.contains("/dev/mapper/cryptroot"));
    assert!(output.contains("%base-services"));
    assert!(!output.contains("/boot/efi"));
}

// --- Enterprise returns empty ---

#[test]
fn enterprise_returns_empty() {
    let mut config = base_config();
    config.mode = InstallMode::Enterprise {
        config_id: "ABC123".into(),
        config_url: "https://example.com".into(),
    };

    let output = render_operating_system(&config);
    assert!(output.is_empty());
}

// --- Channels ---

#[test]
fn channels_guix() {
    let result = render_channels(&InstallMode::Guix);
    assert!(result.is_none());
}

#[test]
fn channels_nonguix() {
    let result = render_channels(&InstallMode::Nonguix);
    assert!(result.is_some());
    let ch = result.unwrap();
    assert!(ch.contains("nonguix"));
    assert!(ch.contains("https://gitlab.com/nonguix/nonguix"));
    assert!(ch.contains("897c1a470da759236cc11798f4e0a5f7d4d59fbc"));
    assert!(ch.contains("2A39 3FFF 68F4 EF7A 3D29  12AF 6F51 20A0 22FB B2D5"));
    assert!(ch.contains("%default-channels"));
}

#[test]
fn channels_panther() {
    let result = render_channels(&InstallMode::Panther);
    assert!(result.is_some());
    let ch = result.unwrap();
    assert!(ch.contains("pantherx"));
    assert!(ch.contains("https://codeberg.org/gofranz/panther.git"));
    assert!(ch.contains("54b4056ac571611892c743b65f4c47dc298c49da"));
    assert!(ch.contains("A36A D41E ECC7 A871 1003  5D24 524F EB1A 9D33 C9CB"));
    assert!(ch.contains("%default-channels"));
}

#[test]
fn channels_enterprise() {
    let result = render_channels(&InstallMode::Enterprise {
        config_id: "X".into(),
        config_url: "https://example.com".into(),
    });
    assert!(result.is_none());
}

// --- Hostname generation ---

#[test]
fn hostname_format_panther() {
    let h = generate_hostname(&InstallMode::Panther);
    assert!(h.starts_with("panther-"), "got: {h}");
    assert_eq!(h.len(), "panther-".len() + 6);
    let suffix = &h["panther-".len()..];
    assert!(suffix.chars().all(|c| c.is_ascii_alphanumeric()));
}

#[test]
fn hostname_format_guix() {
    let h = generate_hostname(&InstallMode::Guix);
    assert!(h.starts_with("guix-"), "got: {h}");
    assert_eq!(h.len(), "guix-".len() + 6);
}

#[test]
fn hostname_format_nonguix() {
    let h = generate_hostname(&InstallMode::Nonguix);
    assert!(h.starts_with("nonguix-"), "got: {h}");
    assert_eq!(h.len(), "nonguix-".len() + 6);
}

#[test]
fn hostname_format_enterprise() {
    let h = generate_hostname(&InstallMode::Enterprise {
        config_id: "X".into(),
        config_url: "u".into(),
    });
    assert!(h.starts_with("enterprise-"), "got: {h}");
    assert_eq!(h.len(), "enterprise-".len() + 6);
}

// --- NVMe partition paths ---

#[test]
fn nvme_disk_efi_file_systems() {
    let mut config = base_config();
    config.mode = InstallMode::Guix;
    config.firmware = Firmware::Efi;
    config.disk = BlockDevice {
        name: "nvme0n1".into(),
        dev_path: "/dev/nvme0n1".into(),
        size_bytes: 500_000_000_000,
        model: None,
        boot_partition_uuid: None,
        root_partition_uuid: None,
    };

    let output = render_operating_system(&config);
    assert!(
        output.contains("/dev/nvme0n1p1"),
        "EFI partition should use p1 separator\n\n{output}"
    );
}

#[test]
fn nvme_disk_luks_source() {
    let mut config = base_config();
    config.mode = InstallMode::Guix;
    config.firmware = Firmware::Bios;
    config.disk = BlockDevice {
        name: "nvme0n1".into(),
        dev_path: "/dev/nvme0n1".into(),
        size_bytes: 500_000_000_000,
        model: None,
        boot_partition_uuid: None,
        root_partition_uuid: None,
    };
    config.encryption = Some(EncryptionConfig {
        device_target: "cryptroot".into(),
    });

    let output = render_operating_system(&config);
    assert!(
        output.contains("/dev/nvme0n1p2"),
        "LUKS source should use p2 separator\n\n{output}"
    );
}

// --- Swap ---

#[test]
fn swap_present() {
    let config = base_config();
    let output = render_operating_system(&config);
    assert!(output.contains("swap-devices"));
    assert!(output.contains("/swapfile"));
}

// --- Keyboard layout ---

#[test]
fn keyboard_layout_included() {
    let mut config = base_config();
    config.mode = InstallMode::Guix;
    config.keyboard_layout = Some("de".into());

    let output = render_operating_system(&config);
    assert!(output.contains("keyboard-layout"));
    assert!(output.contains("\"de\""));
}

#[test]
fn keyboard_layout_omitted() {
    let mut config = base_config();
    config.mode = InstallMode::Guix;
    config.keyboard_layout = None;

    let output = render_operating_system(&config);
    assert!(!output.contains("keyboard-layout"));
}

// --- Btrfs ---

#[test]
fn guix_efi_plain_btrfs_gnome() {
    let mut config = base_config();
    config.mode = InstallMode::Guix;
    config.firmware = Firmware::Efi;
    config.filesystem = Filesystem::Btrfs;
    config.desktop = Some(DesktopEnvironment::Gnome);

    let output = render_operating_system(&config);
    assert_scheme_structure(&output);
    assert!(output.contains("\"btrfs\""));
    assert!(!output.contains("\"ext4\""));
    assert!(output.contains("grub-efi-bootloader"));
}

#[test]
fn panther_efi_luks_btrfs_gnome() {
    let mut config = base_config();
    config.mode = InstallMode::Panther;
    config.firmware = Firmware::Efi;
    config.filesystem = Filesystem::Btrfs;
    config.encryption = Some(EncryptionConfig {
        device_target: "cryptroot".into(),
    });
    config.desktop = Some(DesktopEnvironment::Gnome);

    let output = render_operating_system(&config);
    assert_scheme_structure(&output);
    assert!(output.contains("\"btrfs\""));
    assert!(output.contains("luks-device-mapping"));
    assert!(output.contains("/boot/efi"));
    assert!(output.contains("(inherit %panther-desktop-os)"));
}

// --- EFI + LUKS ---

#[test]
fn nonguix_efi_luks_ext4_gnome() {
    let mut config = base_config();
    config.mode = InstallMode::Nonguix;
    config.firmware = Firmware::Efi;
    config.filesystem = Filesystem::Ext4;
    config.encryption = Some(EncryptionConfig {
        device_target: "cryptroot".into(),
    });
    config.desktop = Some(DesktopEnvironment::Gnome);

    let output = render_operating_system(&config);
    assert_scheme_structure(&output);
    assert!(output.contains("grub-efi-bootloader"));
    assert!(output.contains("luks-device-mapping"));
    assert!(output.contains("/dev/mapper/cryptroot"));
    assert!(output.contains("/boot/efi"));
    assert!(output.contains("(kernel linux)"));
    assert!(output.contains("gnome-desktop-service-type"));
}

// --- Multi-user ---

#[test]
fn multi_user_rendering() {
    let mut config = base_config();
    config.mode = InstallMode::Guix;
    config.firmware = Firmware::Bios;
    config.users = vec![
        UserAccount {
            name: "alice".into(),
            comment: "Alice".into(),
            groups: vec!["wheel".into(), "audio".into()],
        },
        UserAccount {
            name: "bob".into(),
            comment: "Bob".into(),
            groups: vec!["audio".into(), "video".into()],
        },
    ];

    let output = render_operating_system(&config);
    assert_scheme_structure(&output);
    assert!(output.contains("\"alice\""));
    assert!(output.contains("\"bob\""));
    assert!(output.contains("append"));
    assert!(output.contains("%base-user-accounts"));
}

// --- UUID rendering ---

#[test]
fn efi_boot_uuid_rendered() {
    let mut config = base_config();
    config.mode = InstallMode::Guix;
    config.firmware = Firmware::Efi;
    config.disk.boot_partition_uuid = Some("ABCD-1234".into());

    let output = render_operating_system(&config);
    assert_scheme_structure(&output);
    assert!(output.contains("(uuid \"ABCD-1234\" 'fat32)"));
    assert!(!output.contains("/dev/sda1"));
}

#[test]
fn luks_uuid_rendered() {
    let mut config = base_config();
    config.mode = InstallMode::Guix;
    config.firmware = Firmware::Bios;
    config.encryption = Some(EncryptionConfig {
        device_target: "cryptroot".into(),
    });
    config.disk.root_partition_uuid = Some("12345678-abcd-ef01-2345-6789abcdef01".into());

    let output = render_operating_system(&config);
    assert_scheme_structure(&output);
    assert!(output.contains("(uuid \"12345678-abcd-ef01-2345-6789abcdef01\")"));
    assert!(!output.contains("/dev/sda2"));
}

// --- Validation ---

#[test]
fn hostname_validation() {
    assert!(validate_hostname("my-host").is_ok());
    assert!(validate_hostname("host123").is_ok());
    assert!(validate_hostname("a").is_ok());
    assert!(validate_hostname("").is_err());
    assert!(validate_hostname("-bad").is_err());
    assert!(validate_hostname("bad-").is_err());
    assert!(validate_hostname("BAD").is_err());
    assert!(validate_hostname("bad host").is_err());
    assert!(validate_hostname(&"a".repeat(64)).is_err());
}

#[test]
fn username_validation() {
    assert!(validate_username("alice").is_ok());
    assert!(validate_username("_admin").is_ok());
    assert!(validate_username("user-1").is_ok());
    assert!(validate_username("user_name").is_ok());
    assert!(validate_username("").is_err());
    assert!(validate_username("1bad").is_err());
    assert!(validate_username("-bad").is_err());
    assert!(validate_username("BAD").is_err());
    assert!(validate_username("user name").is_err());
}

#[test]
fn config_id_validation() {
    assert!(validate_config_id("ABC123").is_ok());
    assert!(validate_config_id("my-config_1").is_ok());
    assert!(validate_config_id("").is_err());
    assert!(validate_config_id("../../etc/passwd").is_err());
    assert!(validate_config_id("id with spaces").is_err());
}
