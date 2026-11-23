use crate::config::{Filesystem, Firmware, SystemConfig};
use crate::disk::{btrfs, partition_path, root_device};

/// Builds cryptsetup commands for LUKS encryption on partition 2.
///
/// Both read the passphrase from stdin (`--key-file -`), never from argv, so
/// the secret stays out of process listings and the installer log. The caller
/// (phase 2) feeds it via `run_cmd_with_stdin`.
///
/// Returns two commands:
/// 1. `cryptsetup luksFormat --batch-mode --key-file - [--uuid <uuid>] <partition>`
/// 2. `cryptsetup open --type luks --key-file - <partition> <target>`
pub fn encryption_commands(device: &str, target: &str, uuid: Option<&str>) -> Vec<Vec<String>> {
    let part2 = partition_path(device, 2);
    let mut luks_format = vec![
        "cryptsetup",
        "luksFormat",
        "--batch-mode",
        "--key-file",
        "-",
    ];
    if let Some(uuid) = uuid {
        luks_format.extend(["--uuid", uuid]);
    }
    luks_format.push(&part2);
    vec![
        luks_format.into_iter().map(String::from).collect(),
        vec![
            "cryptsetup",
            "open",
            "--type",
            "luks",
            "--key-file",
            "-",
            &part2,
            target,
        ]
        .into_iter()
        .map(String::from)
        .collect(),
    ]
}

/// Builds the root filesystem format commands.
///
/// If encrypted, formats `/dev/mapper/<target>`. Otherwise, formats partition 2.
/// For ext4, also runs `tune2fs -O ^metadata_csum_seed` (Guix compatibility workaround).
/// For btrfs, also creates the flat subvolume layout (see `disk::btrfs`) via a
/// temporary top-level mount.
pub fn format_root_commands(config: &SystemConfig) -> Vec<Vec<String>> {
    let root_device = root_device(config);
    // On LUKS the partition UUID names the container, not the filesystem in it.
    let fs_uuid = match config.encryption {
        Some(_) => None,
        None => config.disk.root_partition_uuid.as_deref(),
    };
    let xfs_uuid = fs_uuid.map(|uuid| format!("uuid={uuid}"));
    let uuid_args: Vec<&str> = match (&config.filesystem, fs_uuid, &xfs_uuid) {
        (Filesystem::Xfs, _, Some(meta)) => vec!["-m", meta],
        (Filesystem::Ext4 | Filesystem::Btrfs, Some(uuid), _) => vec!["-U", uuid],
        _ => vec![],
    };

    let mut cmds = Vec::new();

    match config.filesystem {
        Filesystem::Ext4 => {
            cmds.push(
                [
                    &["mkfs.ext4", "-q", "-L", "my-root"],
                    &uuid_args[..],
                    &[&root_device],
                ]
                .concat()
                .into_iter()
                .map(String::from)
                .collect(),
            );
            cmds.push(
                vec!["tune2fs", "-O", "^metadata_csum_seed", &root_device]
                    .into_iter()
                    .map(String::from)
                    .collect(),
            );
        }
        Filesystem::Btrfs => {
            cmds.push(
                [
                    &["mkfs.btrfs", "-f", "-L", "my-root"],
                    &uuid_args[..],
                    &[&root_device],
                ]
                .concat()
                .into_iter()
                .map(String::from)
                .collect(),
            );
            cmds.extend(btrfs_subvolume_commands(&root_device));
        }
        Filesystem::Xfs => {
            cmds.push(
                [
                    &["mkfs.xfs", "-f", "-L", "my-root"],
                    &uuid_args[..],
                    &[&root_device],
                ]
                .concat()
                .into_iter()
                .map(String::from)
                .collect(),
            );
        }
    }

    cmds
}

/// Mounts the btrfs top level, creates every subvolume, and unmounts again.
fn btrfs_subvolume_commands(root_device: &str) -> Vec<Vec<String>> {
    let mut cmds = vec![vec![
        "mount".to_string(),
        "-t".to_string(),
        "btrfs".to_string(),
        root_device.to_string(),
        "/mnt".to_string(),
    ]];
    for subvol in btrfs::SUBVOLUMES {
        cmds.push(vec![
            "btrfs".to_string(),
            "subvolume".to_string(),
            "create".to_string(),
            format!("/mnt/{}", subvol.name),
        ]);
    }
    cmds.push(vec!["umount".to_string(), "/mnt".to_string()]);
    cmds
}

/// Builds the EFI partition format command (FAT32 on partition 1).
///
/// Only needed for EFI firmware. `volume_id` is the `XXXX-XXXX` FAT serial.
pub fn format_efi_commands(device: &str, volume_id: Option<&str>) -> Vec<Vec<String>> {
    let part1 = partition_path(device, 1);
    let serial = volume_id.map(|id| id.replace('-', ""));
    let mut cmd = vec!["mkfs.fat", "-I", "-F32"];
    if let Some(serial) = &serial {
        cmd.extend(["-i", serial]);
    }
    cmd.push(&part1);
    vec![cmd.into_iter().map(String::from).collect()]
}

/// Clears old filesystem signatures from both partitions.
///
/// A new partition table puts partitions where the old ones were, and
/// mkfs.btrfs leaves an older ext4/xfs signature in place. Two signatures on
/// one partition make blkid and mount give up.
pub fn wipe_commands(device: &str) -> Vec<Vec<String>> {
    [1, 2]
        .iter()
        .map(|&n| {
            vec![
                "wipefs".to_string(),
                "-a".to_string(),
                partition_path(device, n),
            ]
        })
        .collect()
}

/// Builds the complete format command sequence based on the system config.
pub fn format_commands(config: &SystemConfig) -> Vec<Vec<String>> {
    let mut cmds = Vec::new();

    if let Some(enc) = &config.encryption {
        cmds.extend(encryption_commands(
            &config.disk.dev_path,
            &enc.device_target,
            config.disk.root_partition_uuid.as_deref(),
        ));
    }

    if config.firmware == Firmware::Efi {
        cmds.extend(format_efi_commands(
            &config.disk.dev_path,
            config.disk.boot_partition_uuid.as_deref(),
        ));
    }

    cmds.extend(format_root_commands(config));

    cmds
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{BlockDevice, EncryptionConfig};

    fn test_config() -> SystemConfig {
        SystemConfig {
            disk: BlockDevice {
                name: "sda".into(),
                dev_path: "/dev/sda".into(),
                size_bytes: 100_000_000_000,
                model: None,
                boot_partition_uuid: None,
                root_partition_uuid: None,
            },
            ..SystemConfig::default()
        }
    }

    // --- encryption_commands ---

    #[test]
    fn encryption_sata() {
        let cmds = encryption_commands("/dev/sda", "cryptroot", None);
        assert_eq!(cmds.len(), 2);
        assert_eq!(
            cmds[0],
            vec![
                "cryptsetup",
                "luksFormat",
                "--batch-mode",
                "--key-file",
                "-",
                "/dev/sda2"
            ]
        );
        assert_eq!(
            cmds[1],
            vec![
                "cryptsetup",
                "open",
                "--type",
                "luks",
                "--key-file",
                "-",
                "/dev/sda2",
                "cryptroot"
            ]
        );
        // Passphrase must never appear in argv — it is fed via stdin.
        for cmd in &cmds {
            assert!(!cmd.iter().any(|a| a == "cryptroot-passphrase"));
        }
    }

    #[test]
    fn encryption_nvme() {
        let cmds = encryption_commands("/dev/nvme0n1", "cryptroot", None);
        // luksFormat: cryptsetup luksFormat --batch-mode --key-file - <part>
        assert_eq!(cmds[0].last().unwrap(), "/dev/nvme0n1p2");
        // open: cryptsetup open --type luks --key-file - <part> <target>
        assert_eq!(cmds[1][cmds[1].len() - 2], "/dev/nvme0n1p2");
    }

    // --- format_root_commands ---

    #[test]
    fn format_root_ext4_plain() {
        let mut config = test_config();
        config.filesystem = Filesystem::Ext4;
        config.encryption = None;

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
    fn format_root_ext4_encrypted() {
        let mut config = test_config();
        config.filesystem = Filesystem::Ext4;
        config.encryption = Some(EncryptionConfig {
            device_target: "cryptroot".into(),
            passphrase: None,
        });

        let cmds = format_root_commands(&config);
        assert_eq!(cmds.len(), 2);
        assert_eq!(
            cmds[0],
            vec!["mkfs.ext4", "-q", "-L", "my-root", "/dev/mapper/cryptroot"]
        );
        assert_eq!(
            cmds[1],
            vec![
                "tune2fs",
                "-O",
                "^metadata_csum_seed",
                "/dev/mapper/cryptroot"
            ]
        );
    }

    #[test]
    fn format_root_btrfs_plain() {
        let mut config = test_config();
        config.filesystem = Filesystem::Btrfs;
        config.encryption = None;

        let cmds = format_root_commands(&config);
        // mkfs + top-level mount + 4 subvolumes + umount = 7
        assert_eq!(cmds.len(), 7);
        assert_eq!(
            cmds[0],
            vec!["mkfs.btrfs", "-f", "-L", "my-root", "/dev/sda2"]
        );
        assert_eq!(cmds[1], vec!["mount", "-t", "btrfs", "/dev/sda2", "/mnt"]);
        assert_eq!(cmds[2], vec!["btrfs", "subvolume", "create", "/mnt/@"]);
        assert_eq!(cmds[3], vec!["btrfs", "subvolume", "create", "/mnt/@home"]);
        assert_eq!(cmds[4], vec!["btrfs", "subvolume", "create", "/mnt/@gnu"]);
        assert_eq!(cmds[5], vec!["btrfs", "subvolume", "create", "/mnt/@swap"]);
        assert_eq!(cmds[6], vec!["umount", "/mnt"]);
    }

    #[test]
    fn format_root_btrfs_encrypted() {
        let mut config = test_config();
        config.filesystem = Filesystem::Btrfs;
        config.encryption = Some(EncryptionConfig {
            device_target: "cryptroot".into(),
            passphrase: None,
        });

        let cmds = format_root_commands(&config);
        assert_eq!(cmds.len(), 7);
        assert_eq!(
            cmds[0],
            vec!["mkfs.btrfs", "-f", "-L", "my-root", "/dev/mapper/cryptroot"]
        );
        assert_eq!(
            cmds[1],
            vec!["mount", "-t", "btrfs", "/dev/mapper/cryptroot", "/mnt"]
        );
        assert_eq!(cmds[6], vec!["umount", "/mnt"]);
    }

    #[test]
    fn format_root_xfs_plain() {
        let mut config = test_config();
        config.filesystem = Filesystem::Xfs;
        config.encryption = None;

        let cmds = format_root_commands(&config);
        assert_eq!(cmds.len(), 1);
        assert_eq!(
            cmds[0],
            vec!["mkfs.xfs", "-f", "-L", "my-root", "/dev/sda2"]
        );
    }

    #[test]
    fn format_root_xfs_encrypted() {
        let mut config = test_config();
        config.filesystem = Filesystem::Xfs;
        config.encryption = Some(EncryptionConfig {
            device_target: "cryptroot".into(),
            passphrase: None,
        });

        let cmds = format_root_commands(&config);
        assert_eq!(cmds.len(), 1);
        assert_eq!(
            cmds[0],
            vec!["mkfs.xfs", "-f", "-L", "my-root", "/dev/mapper/cryptroot"]
        );
    }

    // --- format_efi_commands ---

    #[test]
    fn format_efi_sata() {
        let cmds = format_efi_commands("/dev/sda", None);
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0], vec!["mkfs.fat", "-I", "-F32", "/dev/sda1"]);
    }

    #[test]
    fn format_efi_nvme() {
        let cmds = format_efi_commands("/dev/nvme0n1", None);
        assert_eq!(cmds[0][3], "/dev/nvme0n1p1");
    }

    // --- format_commands (combined) ---

    #[test]
    fn format_commands_efi_encrypted_ext4() {
        let mut config = test_config();
        config.firmware = Firmware::Efi;
        config.filesystem = Filesystem::Ext4;
        config.encryption = Some(EncryptionConfig {
            device_target: "cryptroot".into(),
            passphrase: None,
        });

        let cmds = format_commands(&config);
        // encryption (2) + efi (1) + root ext4 (2) = 5
        assert_eq!(cmds.len(), 5);

        assert_eq!(cmds[0][0], "cryptsetup"); // luksFormat
        assert_eq!(cmds[1][0], "cryptsetup"); // open
        assert_eq!(cmds[2][0], "mkfs.fat"); // EFI
        assert_eq!(cmds[3][0], "mkfs.ext4"); // root
        assert_eq!(cmds[4][0], "tune2fs"); // Guix workaround
    }

    #[test]
    fn format_commands_bios_plain_btrfs() {
        let mut config = test_config();
        config.firmware = Firmware::Bios;
        config.filesystem = Filesystem::Btrfs;
        config.encryption = None;

        let cmds = format_commands(&config);
        // no encryption, no efi, btrfs (1) + subvolume setup (6)
        assert_eq!(cmds.len(), 7);
        assert_eq!(cmds[0][0], "mkfs.btrfs");
    }

    #[test]
    fn format_commands_nvme_efi_plain_ext4() {
        let mut config = test_config();
        config.firmware = Firmware::Efi;
        config.filesystem = Filesystem::Ext4;
        config.encryption = None;
        config.disk = BlockDevice {
            name: "nvme0n1".into(),
            dev_path: "/dev/nvme0n1".into(),
            size_bytes: 500_000_000_000,
            model: None,
            boot_partition_uuid: None,
            root_partition_uuid: None,
        };

        let cmds = format_commands(&config);
        // efi (1) + ext4 (2) = 3
        assert_eq!(cmds.len(), 3);
        assert_eq!(cmds[0], vec!["mkfs.fat", "-I", "-F32", "/dev/nvme0n1p1"]);
        assert_eq!(
            cmds[1],
            vec!["mkfs.ext4", "-q", "-L", "my-root", "/dev/nvme0n1p2"]
        );
        assert_eq!(
            cmds[2],
            vec!["tune2fs", "-O", "^metadata_csum_seed", "/dev/nvme0n1p2"]
        );
    }
}
