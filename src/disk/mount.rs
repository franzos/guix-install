use crate::config::{Firmware, SystemConfig};
use crate::disk::partition_path;

/// Builds the mount command sequence for the target system.
///
/// Always mounts root by label, then for EFI also mounts the boot partition.
/// Starts the cow-store and creates config directories.
pub fn mount_commands(config: &SystemConfig) -> Vec<Vec<String>> {
    let mut cmds = Vec::new();

    // Mount root by label
    cmds.push(strs(&["mount", "LABEL=my-root", "/mnt"]));

    // EFI: create and mount boot partition
    if config.firmware == Firmware::Efi {
        cmds.push(strs(&["mkdir", "-p", "/mnt/boot/efi"]));
        let part1 = partition_path(&config.disk.dev_path, 1);
        cmds.push(
            vec!["mount", &part1, "/mnt/boot/efi"]
                .into_iter()
                .map(String::from)
                .collect(),
        );
    }

    // Start cow-store overlay
    cmds.push(strs(&["herd", "start", "cow-store", "/mnt"]));

    // Create config directory
    cmds.push(strs(&["mkdir", "-p", "/mnt/etc/guix"]));

    cmds
}

/// Builds the swap file creation commands.
///
/// Creates a swap file at `/mnt/swapfile` of the configured size.
pub fn swap_commands(config: &SystemConfig) -> Vec<Vec<String>> {
    vec![
        vec![
            "dd".to_string(),
            "if=/dev/zero".to_string(),
            "of=/mnt/swapfile".to_string(),
            "bs=1MiB".to_string(),
            format!("count={}", config.swap_size_mb),
        ],
        strs(&["chmod", "600", "/mnt/swapfile"]),
        strs(&["mkswap", "/mnt/swapfile"]),
        strs(&["swapon", "/mnt/swapfile"]),
    ]
}

fn strs(args: &[&str]) -> Vec<String> {
    args.iter().map(|s| String::from(*s)).collect()
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

    // --- mount_commands ---

    #[test]
    fn mount_bios() {
        let mut config = test_config();
        config.firmware = Firmware::Bios;

        let cmds = mount_commands(&config);
        // mount root + cow-store + mkdir = 3
        assert_eq!(cmds.len(), 3);
        assert_eq!(cmds[0], vec!["mount", "LABEL=my-root", "/mnt"]);
        assert_eq!(cmds[1], vec!["herd", "start", "cow-store", "/mnt"]);
        assert_eq!(cmds[2], vec!["mkdir", "-p", "/mnt/etc/guix"]);
    }

    #[test]
    fn mount_efi() {
        let mut config = test_config();
        config.firmware = Firmware::Efi;

        let cmds = mount_commands(&config);
        // mount root + mkdir boot + mount boot + cow-store + mkdir config = 5
        assert_eq!(cmds.len(), 5);
        assert_eq!(cmds[0], vec!["mount", "LABEL=my-root", "/mnt"]);
        assert_eq!(cmds[1], vec!["mkdir", "-p", "/mnt/boot/efi"]);
        assert_eq!(cmds[2], vec!["mount", "/dev/sda1", "/mnt/boot/efi"]);
        assert_eq!(cmds[3], vec!["herd", "start", "cow-store", "/mnt"]);
        assert_eq!(cmds[4], vec!["mkdir", "-p", "/mnt/etc/guix"]);
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

        let cmds = mount_commands(&config);
        assert_eq!(cmds[2], vec!["mount", "/dev/nvme0n1p1", "/mnt/boot/efi"]);
    }

    // --- swap_commands ---

    #[test]
    fn swap_default_size() {
        let config = test_config();
        let cmds = swap_commands(&config);
        assert_eq!(cmds.len(), 4);

        assert_eq!(cmds[0][0], "dd");
        assert!(cmds[0].contains(&"if=/dev/zero".into()));
        assert!(cmds[0].contains(&"of=/mnt/swapfile".into()));
        assert!(cmds[0].contains(&"bs=1MiB".into()));
        assert!(cmds[0].contains(&"count=4096".into()));

        assert_eq!(cmds[1], vec!["chmod", "600", "/mnt/swapfile"]);
        assert_eq!(cmds[2], vec!["mkswap", "/mnt/swapfile"]);
        assert_eq!(cmds[3], vec!["swapon", "/mnt/swapfile"]);
    }

    #[test]
    fn swap_custom_size() {
        let mut config = test_config();
        config.swap_size_mb = 8192;

        let cmds = swap_commands(&config);
        assert!(cmds[0].contains(&"count=8192".into()));
    }
}
