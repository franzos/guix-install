use crate::config::Firmware;

/// Builds the partition command sequence for a given device and firmware type.
///
/// Returns a `Vec<Vec<String>>` where each inner vec is one command to execute.
pub fn partition_commands(device: &str, firmware: &Firmware) -> Vec<Vec<String>> {
    match firmware {
        Firmware::Bios => bios_partition_commands(device),
        Firmware::Efi => efi_partition_commands(device),
    }
}

fn bios_partition_commands(device: &str) -> Vec<Vec<String>> {
    vec![
        vec![
            "parted", "-s", device, "--", "mklabel", "gpt", "mkpart", "primary", "fat32", "0%",
            "10M", "mkpart", "primary", "10M", "100%",
        ]
        .into_iter()
        .map(String::from)
        .collect(),
        vec!["parted", "-s", device, "set", "1", "bios_grub", "on"]
            .into_iter()
            .map(String::from)
            .collect(),
    ]
}

fn efi_partition_commands(device: &str) -> Vec<Vec<String>> {
    vec![
        vec![
            "parted", "-s", device, "--", "mklabel", "gpt", "mkpart", "primary", "fat32", "0%",
            "200M", "mkpart", "primary", "200M", "100%",
        ]
        .into_iter()
        .map(String::from)
        .collect(),
        vec!["sgdisk", "-t", "1:ef00", device]
            .into_iter()
            .map(String::from)
            .collect(),
        vec!["sgdisk", "-t", "2:8300", device]
            .into_iter()
            .map(String::from)
            .collect(),
        vec!["parted", "-s", device, "set", "1", "esp", "on"]
            .into_iter()
            .map(String::from)
            .collect(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bios_commands_sata() {
        let cmds = partition_commands("/dev/sda", &Firmware::Bios);
        assert_eq!(cmds.len(), 2);

        // First command: parted mklabel + mkpart
        assert_eq!(cmds[0][0], "parted");
        assert!(cmds[0].contains(&"-s".into()));
        assert!(cmds[0].contains(&"/dev/sda".into()));
        assert!(cmds[0].contains(&"mklabel".into()));
        assert!(cmds[0].contains(&"gpt".into()));
        assert!(cmds[0].contains(&"10M".into()));

        // Second command: set bios_grub
        assert_eq!(cmds[1][0], "parted");
        assert!(cmds[1].contains(&"bios_grub".into()));
        assert!(cmds[1].contains(&"set".into()));
        assert!(cmds[1].contains(&"1".into()));
    }

    #[test]
    fn efi_commands_sata() {
        let cmds = partition_commands("/dev/sda", &Firmware::Efi);
        assert_eq!(cmds.len(), 4);

        // parted mklabel + mkpart
        assert_eq!(cmds[0][0], "parted");
        assert!(cmds[0].contains(&"200M".into()));

        // sgdisk ef00
        assert_eq!(cmds[1][0], "sgdisk");
        assert!(cmds[1].contains(&"1:ef00".into()));

        // sgdisk 8300
        assert_eq!(cmds[2][0], "sgdisk");
        assert!(cmds[2].contains(&"2:8300".into()));

        // parted set esp
        assert_eq!(cmds[3][0], "parted");
        assert!(cmds[3].contains(&"esp".into()));
    }

    #[test]
    fn efi_commands_nvme() {
        let cmds = partition_commands("/dev/nvme0n1", &Firmware::Efi);
        // All commands should reference the base device, not partitions
        for cmd in &cmds {
            assert!(
                cmd.contains(&"/dev/nvme0n1".into()),
                "command should reference /dev/nvme0n1: {:?}",
                cmd
            );
        }
    }

    #[test]
    fn bios_commands_nvme() {
        let cmds = partition_commands("/dev/nvme0n1", &Firmware::Bios);
        assert_eq!(cmds.len(), 2);
        assert!(cmds[0].contains(&"/dev/nvme0n1".into()));
        assert!(cmds[1].contains(&"/dev/nvme0n1".into()));
    }
}
