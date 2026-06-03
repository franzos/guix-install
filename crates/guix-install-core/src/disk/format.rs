use crate::config::{Filesystem, Firmware, SystemConfig};
use crate::disk::partition_path;

/// Builds cryptsetup commands for LUKS encryption on partition 2.
///
/// Both read the passphrase from stdin (`--key-file -`), never from argv, so
/// the secret stays out of process listings and the installer log. The caller
/// (phase 2) feeds it via `run_cmd_with_stdin`.
///
/// Returns two commands:
/// 1. `cryptsetup luksFormat --batch-mode --key-file - <partition>`
/// 2. `cryptsetup open --type luks --key-file - <partition> <target>`
pub fn encryption_commands(device: &str, target: &str) -> Vec<Vec<String>> {
    let part2 = partition_path(device, 2);
    vec![
        vec![
            "cryptsetup",
            "luksFormat",
            "--batch-mode",
            "--key-file",
            "-",
            &part2,
        ]
        .into_iter()
        .map(String::from)
        .collect(),
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
pub fn format_root_commands(config: &SystemConfig) -> Vec<Vec<String>> {
    let root_device = if let Some(enc) = &config.encryption {
        format!("/dev/mapper/{}", enc.device_target)
    } else {
        partition_path(&config.disk.dev_path, 2)
    };

    let mut cmds = Vec::new();

    match config.filesystem {
        Filesystem::Ext4 => {
            cmds.push(
                vec!["mkfs.ext4", "-q", "-L", "my-root", &root_device]
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
                vec!["mkfs.btrfs", "-f", "-L", "my-root", &root_device]
                    .into_iter()
                    .map(String::from)
                    .collect(),
            );
        }
    }

    cmds
}

/// Builds the EFI partition format command (FAT32 on partition 1).
///
/// Only needed for EFI firmware.
pub fn format_efi_commands(device: &str) -> Vec<Vec<String>> {
    let part1 = partition_path(device, 1);
    vec![
        vec!["mkfs.fat", "-I", "-F32", &part1]
            .into_iter()
            .map(String::from)
            .collect(),
    ]
}

/// Builds the complete format command sequence based on the system config.
pub fn format_commands(config: &SystemConfig) -> Vec<Vec<String>> {
    let mut cmds = Vec::new();

    if let Some(enc) = &config.encryption {
        cmds.extend(encryption_commands(
            &config.disk.dev_path,
            &enc.device_target,
        ));
    }

    if config.firmware == Firmware::Efi {
        cmds.extend(format_efi_commands(&config.disk.dev_path));
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
        let cmds = encryption_commands("/dev/sda", "cryptroot");
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
        let cmds = encryption_commands("/dev/nvme0n1", "cryptroot");
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
        assert_eq!(cmds.len(), 1);
        assert_eq!(
            cmds[0],
            vec!["mkfs.btrfs", "-f", "-L", "my-root", "/dev/sda2"]
        );
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
        assert_eq!(cmds.len(), 1);
        assert_eq!(
            cmds[0],
            vec!["mkfs.btrfs", "-f", "-L", "my-root", "/dev/mapper/cryptroot"]
        );
    }

    // --- format_efi_commands ---

    #[test]
    fn format_efi_sata() {
        let cmds = format_efi_commands("/dev/sda");
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0], vec!["mkfs.fat", "-I", "-F32", "/dev/sda1"]);
    }

    #[test]
    fn format_efi_nvme() {
        let cmds = format_efi_commands("/dev/nvme0n1");
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
        // no encryption, no efi, btrfs (1)
        assert_eq!(cmds.len(), 1);
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
