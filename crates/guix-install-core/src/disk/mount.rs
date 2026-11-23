use std::path::PathBuf;

use crate::config::{Filesystem, Firmware, SystemConfig};
use crate::disk::{Action, btrfs, partition_path, root_device};

/// Builds the mount action sequence for the target system.
///
/// Mounts root by device path (each btrfs subvolume in turn), then for EFI
/// also mounts the boot partition. Starts the cow-store and creates config
/// directories.
pub fn mount_actions(config: &SystemConfig) -> Vec<Action> {
    let mut actions = Vec::new();
    let root_device = root_device(config);

    match config.filesystem {
        Filesystem::Btrfs => {
            for subvol in btrfs::SUBVOLUMES {
                let target = subvol.target_path("/mnt");
                if subvol.mount_point != "/" {
                    actions.push(Action::mkdir(&target));
                }
                actions.push(Action::cmd(&[
                    "mount",
                    "-t",
                    "btrfs",
                    "-o",
                    &subvol.mount_options(),
                    &root_device,
                    &target,
                ]));
            }
        }
        Filesystem::Ext4 | Filesystem::Xfs => {
            let fs_type = config.filesystem.to_string();
            actions.push(Action::cmd(&[
                "mount",
                "-t",
                &fs_type,
                &root_device,
                "/mnt",
            ]));
        }
    }

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
/// On ext4 and xfs, creates a zero-filled file then runs `mkswap` + `swapon`.
/// On btrfs, swap files require NOCOW and contiguous extents, so we use
/// `btrfs filesystem mkswapfile -s <size>` which handles NOCOW + format
/// in one shot, then `swapon`. The btrfs swap file lives on the `@swap`
/// subvolume (see `Filesystem::swap_file_path`).
pub fn swap_actions(config: &SystemConfig) -> Vec<Action> {
    let path = format!("/mnt{}", config.filesystem.swap_file_path());

    match config.filesystem {
        Filesystem::Ext4 | Filesystem::Xfs => {
            let size_bytes = (config.swap_size_mb as u64) * 1024 * 1024;
            vec![
                Action::CreateSwapFile {
                    path: PathBuf::from(&path),
                    size_bytes,
                },
                Action::cmd(&["mkswap", &path]),
                Action::cmd(&["swapon", &path]),
            ]
        }
        Filesystem::Btrfs => {
            let size = format!("{}m", config.swap_size_mb);
            vec![
                Action::cmd(&["btrfs", "filesystem", "mkswapfile", "-s", &size, &path]),
                Action::cmd(&["swapon", &path]),
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
        assert_eq!(
            actions[0],
            Action::cmd(&["mount", "-t", "ext4", "/dev/sda2", "/mnt"])
        );
        assert_eq!(
            actions[1],
            Action::cmd(&["herd", "start", "cow-store", "/mnt"])
        );
        assert_eq!(actions[2], Action::mkdir("/mnt/etc/guix"));
    }

    #[test]
    fn mount_btrfs_subvolumes() {
        let mut config = test_config();
        config.firmware = Firmware::Bios;
        config.filesystem = Filesystem::Btrfs;

        let actions = mount_actions(&config);
        // root + 3 x (mkdir + mount) + cow-store + mkdir config = 9
        assert_eq!(actions.len(), 9);
        assert_eq!(
            actions[0],
            Action::cmd(&[
                "mount",
                "-t",
                "btrfs",
                "-o",
                "subvol=@,compress=zstd",
                "/dev/sda2",
                "/mnt"
            ])
        );
        assert_eq!(actions[1], Action::mkdir("/mnt/home"));
        assert_eq!(
            actions[2],
            Action::cmd(&[
                "mount",
                "-t",
                "btrfs",
                "-o",
                "subvol=@home,compress=zstd",
                "/dev/sda2",
                "/mnt/home"
            ])
        );
        assert_eq!(actions[3], Action::mkdir("/mnt/gnu"));
        assert_eq!(
            actions[4],
            Action::cmd(&[
                "mount",
                "-t",
                "btrfs",
                "-o",
                "subvol=@gnu,compress=zstd",
                "/dev/sda2",
                "/mnt/gnu"
            ])
        );
        assert_eq!(actions[5], Action::mkdir("/mnt/swap"));
        assert_eq!(
            actions[6],
            Action::cmd(&[
                "mount",
                "-t",
                "btrfs",
                "-o",
                "subvol=@swap,compress=zstd",
                "/dev/sda2",
                "/mnt/swap"
            ])
        );
        assert_eq!(
            actions[7],
            Action::cmd(&["herd", "start", "cow-store", "/mnt"])
        );
        assert_eq!(actions[8], Action::mkdir("/mnt/etc/guix"));
    }

    #[test]
    fn swap_btrfs_swapon_uses_subvolume_path() {
        let mut config = test_config();
        config.filesystem = Filesystem::Btrfs;

        let actions = swap_actions(&config);
        assert_eq!(actions[1], Action::cmd(&["swapon", "/mnt/swap/swapfile"]));
    }

    #[test]
    fn mount_efi() {
        let mut config = test_config();
        config.firmware = Firmware::Efi;

        let actions = mount_actions(&config);
        // mount root + mkdir boot + mount boot + cow-store + mkdir config = 5
        assert_eq!(actions.len(), 5);
        assert_eq!(
            actions[0],
            Action::cmd(&["mount", "-t", "ext4", "/dev/sda2", "/mnt"])
        );
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
                "/mnt/swap/swapfile",
            ])
        );
        assert_eq!(actions[1], Action::cmd(&["swapon", "/mnt/swap/swapfile"]));
    }

    #[test]
    fn swap_xfs_uses_plain_swapfile() {
        let mut config = test_config();
        config.filesystem = Filesystem::Xfs;
        config.swap_size_mb = 2048;

        let actions = swap_actions(&config);
        assert_eq!(actions.len(), 3);
        assert_eq!(
            actions[0],
            Action::CreateSwapFile {
                path: PathBuf::from("/mnt/swapfile"),
                size_bytes: 2048 * 1024 * 1024,
            }
        );
        assert_eq!(actions[1], Action::cmd(&["mkswap", "/mnt/swapfile"]));
        assert_eq!(actions[2], Action::cmd(&["swapon", "/mnt/swapfile"]));
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
