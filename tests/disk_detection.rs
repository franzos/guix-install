use guix_install::config::{
    BlockDevice, EncryptionConfig, Filesystem, Firmware, SystemConfig, UserAccount,
};
use guix_install::disk::Action;
use guix_install::disk::detect::{format_device, parse_lsblk_json};
use guix_install::disk::format::{
    encryption_commands, format_commands, format_efi_commands, format_root_commands,
};
use guix_install::disk::mount::{mount_actions, swap_actions};
use guix_install::disk::partition::partition_commands;
use guix_install::disk::partition_path;
use guix_install::mode::InstallMode;
use std::path::PathBuf;

// --- lsblk parsing fixtures ---

const FIXTURE_MULTI: &str = r#"{
    "blockdevices": [
        {"name": "sda", "size": 120034123776, "type": "disk", "model": "Samsung SSD 860", "path": "/dev/sda"},
        {"name": "sda1", "size": 10485760, "type": "part", "model": null, "path": "/dev/sda1"},
        {"name": "sda2", "size": 120023638016, "type": "part", "model": null, "path": "/dev/sda2"},
        {"name": "nvme0n1", "size": 512110190592, "type": "disk", "model": "WD Blue SN570", "path": "/dev/nvme0n1"},
        {"name": "nvme0n1p1", "size": 209715200, "type": "part", "model": null, "path": "/dev/nvme0n1p1"},
        {"name": "loop0", "size": 734003200, "type": "loop", "model": null, "path": "/dev/loop0"},
        {"name": "sr0", "size": 1073741312, "type": "rom", "model": "DVD-ROM", "path": "/dev/sr0"}
    ]
}"#;

const FIXTURE_SINGLE: &str = r#"{
    "blockdevices": [
        {"name": "sda", "size": 256060514304, "type": "disk", "model": "VBOX HARDDISK", "path": "/dev/sda"}
    ]
}"#;

fn test_config(dev_path: &str, dev_name: &str) -> SystemConfig {
    SystemConfig {
        mode: InstallMode::Panther,
        firmware: Firmware::Efi,
        hostname: "test-host".into(),
        timezone: "Europe/Berlin".into(),
        locale: "en_US.utf8".into(),
        keyboard_layout: None,
        disk: BlockDevice {
            name: dev_name.into(),
            dev_path: dev_path.into(),
            size_bytes: 100_000_000_000,
            model: None,
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
    }
}

// === Disk Detection ===

#[test]
fn parse_multi_disk_fixture() {
    let devices = parse_lsblk_json(FIXTURE_MULTI).unwrap();
    assert_eq!(devices.len(), 2, "should find exactly 2 disks");

    assert_eq!(devices[0].name, "sda");
    assert_eq!(devices[0].dev_path, "/dev/sda");
    assert_eq!(devices[0].size_bytes, 120034123776);
    assert_eq!(devices[0].model.as_deref(), Some("Samsung SSD 860"));

    assert_eq!(devices[1].name, "nvme0n1");
    assert_eq!(devices[1].dev_path, "/dev/nvme0n1");
    assert_eq!(devices[1].size_bytes, 512110190592);
    assert_eq!(devices[1].model.as_deref(), Some("WD Blue SN570"));
}

#[test]
fn parse_single_disk_fixture() {
    let devices = parse_lsblk_json(FIXTURE_SINGLE).unwrap();
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].name, "sda");
    assert_eq!(devices[0].size_bytes, 256060514304);
}

#[test]
fn filters_non_disk_types() {
    let devices = parse_lsblk_json(FIXTURE_MULTI).unwrap();
    let names: Vec<&str> = devices.iter().map(|d| d.name.as_str()).collect();
    assert!(!names.contains(&"loop0"), "loop devices should be filtered");
    assert!(!names.contains(&"sr0"), "CD-ROM should be filtered");
    assert!(!names.contains(&"sda1"), "partitions should be filtered");
    assert!(
        !names.contains(&"nvme0n1p1"),
        "partitions should be filtered"
    );
}

#[test]
fn format_device_output() {
    let dev = BlockDevice {
        name: "sda".into(),
        dev_path: "/dev/sda".into(),
        size_bytes: 120_000_000_000,
        model: Some("Samsung SSD 860 EVO".into()),
        boot_partition_uuid: None,
        root_partition_uuid: None,
    };
    let output = format_device(&dev);
    assert!(output.contains("/dev/sda"));
    assert!(output.contains("120 GB"));
    assert!(output.contains("Samsung SSD 860 EVO"));
}

// === Partition Path ===

#[test]
fn partition_path_sata() {
    assert_eq!(partition_path("/dev/sda", 1), "/dev/sda1");
    assert_eq!(partition_path("/dev/sda", 2), "/dev/sda2");
    assert_eq!(partition_path("/dev/sdb", 1), "/dev/sdb1");
}

#[test]
fn partition_path_nvme() {
    assert_eq!(partition_path("/dev/nvme0n1", 1), "/dev/nvme0n1p1");
    assert_eq!(partition_path("/dev/nvme0n1", 2), "/dev/nvme0n1p2");
}

#[test]
fn partition_path_mmc() {
    assert_eq!(partition_path("/dev/mmcblk0", 1), "/dev/mmcblk0p1");
    assert_eq!(partition_path("/dev/mmcblk0", 2), "/dev/mmcblk0p2");
}

// === Partition Commands ===

#[test]
fn bios_partition_commands() {
    let cmds = partition_commands("/dev/sda", &Firmware::Bios);
    assert_eq!(cmds.len(), 2);

    // parted mklabel + mkpart with 10M BIOS boot
    assert_eq!(cmds[0][0], "parted");
    assert!(cmds[0].contains(&"mklabel".into()));
    assert!(cmds[0].contains(&"gpt".into()));
    assert!(cmds[0].contains(&"10M".into()));

    // set bios_grub flag
    assert_eq!(cmds[1][0], "parted");
    assert!(cmds[1].contains(&"bios_grub".into()));
}

#[test]
fn efi_partition_commands() {
    let cmds = partition_commands("/dev/sda", &Firmware::Efi);
    assert_eq!(cmds.len(), 4);

    // parted mklabel + mkpart with 200M EFI
    assert!(cmds[0].contains(&"200M".into()));

    // sgdisk type codes
    assert_eq!(cmds[1][0], "sgdisk");
    assert!(cmds[1].contains(&"1:ef00".into()));

    assert_eq!(cmds[2][0], "sgdisk");
    assert!(cmds[2].contains(&"2:8300".into()));

    // set esp flag
    assert!(cmds[3].contains(&"esp".into()));
}

#[test]
fn partition_commands_nvme_device() {
    let cmds = partition_commands("/dev/nvme0n1", &Firmware::Efi);
    for cmd in &cmds {
        assert!(
            cmd.contains(&"/dev/nvme0n1".into()),
            "all partition commands should reference the base device: {:?}",
            cmd
        );
    }
}

// === Format Commands ===

#[test]
fn format_ext4_plain_sata() {
    let config = test_config("/dev/sda", "sda");
    let cmds = format_root_commands(&config);
    assert_eq!(cmds.len(), 2);
    assert_eq!(
        cmds[0],
        vec!["mkfs.ext4", "-q", "-L", "my-root", "/dev/sda2"]
    );
    assert_eq!(
        cmds[1],
        vec!["tune2fs", "-O", "^metadata_csum_seed", "/dev/sda2"]
    );
}

#[test]
fn format_btrfs_plain_sata() {
    let mut config = test_config("/dev/sda", "sda");
    config.filesystem = Filesystem::Btrfs;
    let cmds = format_root_commands(&config);
    assert_eq!(cmds.len(), 1);
    assert_eq!(
        cmds[0],
        vec!["mkfs.btrfs", "-f", "-L", "my-root", "/dev/sda2"]
    );
}

#[test]
fn format_ext4_encrypted() {
    let mut config = test_config("/dev/sda", "sda");
    config.encryption = Some(EncryptionConfig {
        device_target: "cryptroot".into(),
    });
    let cmds = format_root_commands(&config);
    assert_eq!(cmds[0][4], "/dev/mapper/cryptroot");
    assert_eq!(cmds[1][3], "/dev/mapper/cryptroot");
}

#[test]
fn format_btrfs_encrypted() {
    let mut config = test_config("/dev/sda", "sda");
    config.filesystem = Filesystem::Btrfs;
    config.encryption = Some(EncryptionConfig {
        device_target: "cryptroot".into(),
    });
    let cmds = format_root_commands(&config);
    assert_eq!(cmds.len(), 1);
    assert_eq!(cmds[0][4], "/dev/mapper/cryptroot");
}

#[test]
fn format_efi_partition() {
    let cmds = format_efi_commands("/dev/sda");
    assert_eq!(cmds[0], vec!["mkfs.fat", "-I", "-F32", "/dev/sda1"]);
}

#[test]
fn format_efi_nvme() {
    let cmds = format_efi_commands("/dev/nvme0n1");
    assert_eq!(cmds[0][3], "/dev/nvme0n1p1");
}

#[test]
fn encryption_commands_sata() {
    let cmds = encryption_commands("/dev/sda", "cryptroot");
    assert_eq!(cmds.len(), 2);
    assert_eq!(cmds[0], vec!["cryptsetup", "luksFormat", "/dev/sda2"]);
    assert_eq!(
        cmds[1],
        vec![
            "cryptsetup",
            "open",
            "--type",
            "luks",
            "/dev/sda2",
            "cryptroot"
        ]
    );
}

#[test]
fn encryption_commands_nvme() {
    let cmds = encryption_commands("/dev/nvme0n1", "cryptroot");
    assert_eq!(cmds[0][2], "/dev/nvme0n1p2");
    assert_eq!(cmds[1][4], "/dev/nvme0n1p2");
}

#[test]
fn format_commands_complete_efi_encrypted() {
    let mut config = test_config("/dev/sda", "sda");
    config.firmware = Firmware::Efi;
    config.encryption = Some(EncryptionConfig {
        device_target: "cryptroot".into(),
    });

    let cmds = format_commands(&config);
    // cryptsetup (2) + mkfs.fat (1) + mkfs.ext4 (1) + tune2fs (1) = 5
    assert_eq!(cmds.len(), 5);
    assert_eq!(cmds[0][0], "cryptsetup");
    assert_eq!(cmds[1][0], "cryptsetup");
    assert_eq!(cmds[2][0], "mkfs.fat");
    assert_eq!(cmds[3][0], "mkfs.ext4");
    assert_eq!(cmds[4][0], "tune2fs");
}

#[test]
fn format_commands_bios_plain() {
    let mut config = test_config("/dev/sda", "sda");
    config.firmware = Firmware::Bios;

    let cmds = format_commands(&config);
    // no encryption, no efi: mkfs.ext4 (1) + tune2fs (1) = 2
    assert_eq!(cmds.len(), 2);
    assert_eq!(cmds[0][0], "mkfs.ext4");
    assert_eq!(cmds[1][0], "tune2fs");
}

// === Mount Actions ===

#[test]
fn mount_bios_sequence() {
    let mut config = test_config("/dev/sda", "sda");
    config.firmware = Firmware::Bios;

    let actions = mount_actions(&config);
    assert_eq!(actions.len(), 3);
    assert_eq!(actions[0], Action::cmd(&["mount", "LABEL=my-root", "/mnt"]));
    assert_eq!(
        actions[1],
        Action::cmd(&["herd", "start", "cow-store", "/mnt"])
    );
    assert_eq!(actions[2], Action::mkdir("/mnt/etc/guix"));
}

#[test]
fn mount_efi_sequence() {
    let config = test_config("/dev/sda", "sda");

    let actions = mount_actions(&config);
    assert_eq!(actions.len(), 5);
    assert_eq!(actions[0], Action::cmd(&["mount", "LABEL=my-root", "/mnt"]));
    assert_eq!(actions[1], Action::mkdir("/mnt/boot/efi"));
    assert_eq!(
        actions[2],
        Action::cmd(&["mount", "/dev/sda1", "/mnt/boot/efi"])
    );
    assert_eq!(
        actions[3],
        Action::cmd(&["herd", "start", "cow-store", "/mnt"])
    );
    assert_eq!(actions[4], Action::mkdir("/mnt/etc/guix"));
}

#[test]
fn mount_efi_nvme_partition_path() {
    let mut config = test_config("/dev/nvme0n1", "nvme0n1");
    config.firmware = Firmware::Efi;

    let actions = mount_actions(&config);
    assert_eq!(
        actions[2],
        Action::cmd(&["mount", "/dev/nvme0n1p1", "/mnt/boot/efi"])
    );
}

// === Swap Actions ===

#[test]
fn swap_default() {
    let config = test_config("/dev/sda", "sda");
    let actions = swap_actions(&config);
    assert_eq!(actions.len(), 3);

    assert_eq!(
        actions[0],
        Action::CreateSwapFile {
            path: PathBuf::from("/mnt/swapfile"),
            size_bytes: 4096 * 1024 * 1024,
        }
    );
    assert_eq!(actions[1], Action::cmd(&["mkswap", "/mnt/swapfile"]));
    assert_eq!(actions[2], Action::cmd(&["swapon", "/mnt/swapfile"]));
}

#[test]
fn swap_custom_size() {
    let mut config = test_config("/dev/sda", "sda");
    config.swap_size_mb = 8192;

    let actions = swap_actions(&config);
    assert_eq!(
        actions[0],
        Action::CreateSwapFile {
            path: PathBuf::from("/mnt/swapfile"),
            size_bytes: 8192 * 1024 * 1024,
        }
    );
}
