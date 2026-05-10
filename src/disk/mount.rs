use std::path::PathBuf;

use crate::config::{Filesystem, Firmware, SystemConfig};
use crate::disk::{Action, partition_path};

/// Builds the mount action sequence for the target system.
///
/// Always mounts root by label, then for EFI also mounts the boot partition.
/// Starts the cow-store and creates config directories.
pub fn mount_actions(config: &SystemConfig) -> Vec<Action> {
    let mut actions = Vec::new();

    // Mount root by label
    actions.push(Action::cmd(&["mount", "LABEL=my-root", "/mnt"]));

    // EFI: create and mount boot partition
    if config.firmware == Firmware::Efi {
        actions.push(Action::mkdir("/mnt/boot/efi"));
        let part1 = partition_path(&config.disk.dev_path, 1);
        actions.push(Action::Mount {
            source: PathBuf::from(&part1),
            target: PathBuf::from("/mnt/boot/efi"),
            fstype: "vfat".into(),
        });
    }

    // Start cow-store overlay
    actions.push(Action::cmd(&["herd", "start", "cow-store", "/mnt"]));

    // Create config directory
    actions.push(Action::mkdir("/mnt/etc/guix"));

    actions
}

/// Builds the swap file creation actions.
///
/// On ext4, creates a zero-filled file then runs `mkswap` + `swapon`.
/// On btrfs, swap files require NOCOW and contiguous extents, so we use
/// `btrfs filesystem mkswapfile -s <size>` which handles NOCOW + format
/// in one shot, then `swapon`.
pub fn swap_actions(config: &SystemConfig) -> Vec<Action> {
    let path = PathBuf::from("/mnt/swapfile");

    match config.filesystem {
        Filesystem::Ext4 => {
            let size_bytes = (config.swap_size_mb as u64) * 1024 * 1024;
            vec![
                Action::CreateSwapFile {
                    path: path.clone(),
                    size_bytes,
                },
                Action::cmd(&["mkswap", "/mnt/swapfile"]),
                Action::cmd(&["swapon", "/mnt/swapfile"]),
            ]
        }
        Filesystem::Btrfs => {
            let size = format!("{}m", config.swap_size_mb);
            vec![
                Action::cmd(&[
                    "btrfs",
                    "filesystem",
                    "mkswapfile",
                    "-s",
                    &size,
                    "/mnt/swapfile",
                ]),
                Action::cmd(&["swapon", "/mnt/swapfile"]),
            ]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::BlockDevice;

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

    // --- mount_actions ---

    #[test]
    fn mount_bios() {
        let mut config = test_config();
        config.firmware = Firmware::Bios;

        let actions = mount_actions(&config);
        // mount root + cow-store + mkdir = 3
        assert_eq!(actions.len(), 3);
        assert_eq!(actions[0], Action::cmd(&["mount", "LABEL=my-root", "/mnt"]));
        assert_eq!(
            actions[1],
            Action::cmd(&["herd", "start", "cow-store", "/mnt"])
        );
        assert_eq!(actions[2], Action::mkdir("/mnt/etc/guix"));
    }

    #[test]
    fn mount_efi() {
        let mut config = test_config();
        config.firmware = Firmware::Efi;

        let actions = mount_actions(&config);
        // mount root + mkdir boot + mount boot + cow-store + mkdir config = 5
        assert_eq!(actions.len(), 5);
        assert_eq!(actions[0], Action::cmd(&["mount", "LABEL=my-root", "/mnt"]));
        assert_eq!(actions[1], Action::mkdir("/mnt/boot/efi"));
        assert_eq!(
            actions[2],
            Action::Mount {
                source: PathBuf::from("/dev/sda1"),
                target: PathBuf::from("/mnt/boot/efi"),
                fstype: "vfat".into(),
            }
        );
        assert_eq!(
            actions[3],
            Action::cmd(&["herd", "start", "cow-store", "/mnt"])
        );
        assert_eq!(actions[4], Action::mkdir("/mnt/etc/guix"));
    }

    #[test]
    fn mount_efi_nvme() {
        let mut config = test_config();
        config.firmware = Firmware::Efi;
        config.disk = BlockDevice {
            name: "nvme0n1".into(),
            dev_path: "/dev/nvme0n1".into(),
            size_bytes: 500_000_000_000,
            model: None,
            boot_partition_uuid: None,
            root_partition_uuid: None,
        };

        let actions = mount_actions(&config);
        assert_eq!(
            actions[2],
            Action::Mount {
                source: PathBuf::from("/dev/nvme0n1p1"),
                target: PathBuf::from("/mnt/boot/efi"),
                fstype: "vfat".into(),
            }
        );
    }

    // --- swap_actions ---

    #[test]
    fn swap_default_size() {
        let config = test_config();
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
    fn swap_btrfs_uses_mkswapfile() {
        let mut config = test_config();
        config.filesystem = Filesystem::Btrfs;
        config.swap_size_mb = 4096;

        let actions = swap_actions(&config);
        assert_eq!(actions.len(), 2);
        assert_eq!(
            actions[0],
            Action::cmd(&[
                "btrfs",
                "filesystem",
                "mkswapfile",
                "-s",
                "4096m",
                "/mnt/swapfile",
            ])
        );
        assert_eq!(actions[1], Action::cmd(&["swapon", "/mnt/swapfile"]));
    }

    #[test]
    fn swap_custom_size() {
        let mut config = test_config();
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
}
