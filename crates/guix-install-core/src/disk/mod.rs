pub mod action;
pub mod detect;
pub mod format;
pub mod mount;
pub mod partition;

pub use action::Action;

/// Returns the device path for a given partition number.
///
/// NVMe and MMC devices use a `p` separator (e.g. `/dev/nvme0n1p1`),
/// while SATA/IDE devices append the number directly (e.g. `/dev/sda1`).
pub fn partition_path(device: &str, num: u32) -> String {
    if device.starts_with("/dev/nvme") || device.starts_with("/dev/mmcblk") {
        format!("{device}p{num}")
    } else {
        format!("{device}{num}")
    }
}

/// Formats a byte count into a human-readable string (e.g. "120 GB").
pub fn format_size(bytes: u64) -> String {
    const GB: u64 = 1_000_000_000;
    const TB: u64 = 1_000_000_000_000;

    if bytes >= TB {
        let tb = bytes as f64 / TB as f64;
        if tb >= 10.0 {
            format!("{:.0} TB", tb)
        } else {
            format!("{:.1} TB", tb)
        }
    } else {
        let gb = bytes as f64 / GB as f64;
        if gb >= 10.0 {
            format!("{:.0} GB", gb)
        } else {
            format!("{:.1} GB", gb)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partition_path_sata() {
        assert_eq!(partition_path("/dev/sda", 1), "/dev/sda1");
        assert_eq!(partition_path("/dev/sda", 2), "/dev/sda2");
        assert_eq!(partition_path("/dev/sdb", 3), "/dev/sdb3");
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

    #[test]
    fn format_size_gigabytes() {
        assert_eq!(format_size(120_000_000_000), "120 GB");
        assert_eq!(format_size(512_000_000_000), "512 GB");
        assert_eq!(format_size(8_000_000_000), "8.0 GB");
    }

    #[test]
    fn format_size_terabytes() {
        assert_eq!(format_size(1_000_000_000_000), "1.0 TB");
        assert_eq!(format_size(2_000_000_000_000), "2.0 TB");
        assert_eq!(format_size(10_000_000_000_000), "10 TB");
    }
}
