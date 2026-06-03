use anyhow::{Context, Result};
use serde::Deserialize;

use crate::config::BlockDevice;
use crate::exec;

/// Runs `lsblk --json --bytes` and returns detected disk devices.
pub fn detect_block_devices() -> Result<Vec<BlockDevice>> {
    let result = exec::run_cmd(&[
        "lsblk",
        "--json",
        "--bytes",
        "--output",
        "NAME,SIZE,TYPE,MODEL,PATH",
    ])
    .context("failed to run lsblk")?;
    parse_lsblk_json(&result.stdout)
}

/// Parses lsblk JSON output into `BlockDevice` structs.
///
/// Filters to entries with `type == "disk"` and excludes phantom devices that
/// can't host an install: floppy drives (`fd*`, even with no media the kernel
/// reports a 4096-byte placeholder) and anything smaller than 1 MiB (empty
/// card readers etc.).
pub fn parse_lsblk_json(json: &str) -> Result<Vec<BlockDevice>> {
    const MIN_SIZE: u64 = 1024 * 1024;

    let parsed: LsblkOutput = serde_json::from_str(json).context("failed to parse lsblk JSON")?;

    let devices = parsed
        .blockdevices
        .into_iter()
        .filter(|d| {
            d.device_type == "disk" && d.size.unwrap_or(0) >= MIN_SIZE && !d.name.starts_with("fd")
        })
        .map(|d| BlockDevice {
            name: d.name,
            dev_path: d.path.unwrap_or_default(),
            size_bytes: d.size.unwrap_or(0),
            model: d
                .model
                .map(|m| m.trim().to_string())
                .filter(|m| !m.is_empty()),
            boot_partition_uuid: None,
            root_partition_uuid: None,
        })
        .collect();

    Ok(devices)
}

/// Formats a `BlockDevice` for display (e.g. in `list-disks` output).
pub fn format_device(dev: &BlockDevice) -> String {
    let size = super::format_size(dev.size_bytes);
    let model = dev.model.as_deref().unwrap_or("Unknown");
    format!("{:<14}{:<9}{}", dev.dev_path, size, model)
}

#[derive(Deserialize)]
struct LsblkOutput {
    blockdevices: Vec<LsblkDevice>,
}

#[derive(Deserialize)]
struct LsblkDevice {
    name: String,
    size: Option<u64>,
    #[serde(rename = "type")]
    device_type: String,
    model: Option<String>,
    path: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    const MULTI_DISK_JSON: &str = r#"{
        "blockdevices": [
            {
                "name": "sda",
                "size": 120034123776,
                "type": "disk",
                "model": "Samsung SSD 860",
                "path": "/dev/sda"
            },
            {
                "name": "sda1",
                "size": 10485760,
                "type": "part",
                "model": null,
                "path": "/dev/sda1"
            },
            {
                "name": "sda2",
                "size": 120023638016,
                "type": "part",
                "model": null,
                "path": "/dev/sda2"
            },
            {
                "name": "nvme0n1",
                "size": 512110190592,
                "type": "disk",
                "model": "WD Blue SN570  ",
                "path": "/dev/nvme0n1"
            },
            {
                "name": "nvme0n1p1",
                "size": 209715200,
                "type": "part",
                "model": null,
                "path": "/dev/nvme0n1p1"
            },
            {
                "name": "loop0",
                "size": 734003200,
                "type": "loop",
                "model": null,
                "path": "/dev/loop0"
            },
            {
                "name": "sr0",
                "size": 1073741312,
                "type": "rom",
                "model": "DVD-ROM",
                "path": "/dev/sr0"
            }
        ]
    }"#;

    const SINGLE_DISK_JSON: &str = r#"{
        "blockdevices": [
            {
                "name": "sda",
                "size": 256060514304,
                "type": "disk",
                "model": "VBOX HARDDISK",
                "path": "/dev/sda"
            }
        ]
    }"#;

    #[test]
    fn parse_multi_disk() {
        let devices = parse_lsblk_json(MULTI_DISK_JSON).unwrap();
        assert_eq!(devices.len(), 2);

        assert_eq!(devices[0].name, "sda");
        assert_eq!(devices[0].dev_path, "/dev/sda");
        assert_eq!(devices[0].size_bytes, 120034123776);
        assert_eq!(devices[0].model.as_deref(), Some("Samsung SSD 860"));

        assert_eq!(devices[1].name, "nvme0n1");
        assert_eq!(devices[1].dev_path, "/dev/nvme0n1");
        assert_eq!(devices[1].size_bytes, 512110190592);
        // model should be trimmed
        assert_eq!(devices[1].model.as_deref(), Some("WD Blue SN570"));
    }

    #[test]
    fn parse_single_disk() {
        let devices = parse_lsblk_json(SINGLE_DISK_JSON).unwrap();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].name, "sda");
        assert_eq!(devices[0].dev_path, "/dev/sda");
        assert_eq!(devices[0].size_bytes, 256060514304);
        assert_eq!(devices[0].model.as_deref(), Some("VBOX HARDDISK"));
    }

    #[test]
    fn filters_non_disk_types() {
        let devices = parse_lsblk_json(MULTI_DISK_JSON).unwrap();
        for dev in &devices {
            assert!(
                dev.name != "loop0" && dev.name != "sr0" && !dev.name.contains("sda1"),
                "non-disk device leaked through: {}",
                dev.name
            );
        }
    }

    #[test]
    fn empty_blockdevices() {
        let json = r#"{"blockdevices": []}"#;
        let devices = parse_lsblk_json(json).unwrap();
        assert!(devices.is_empty());
    }

    #[test]
    fn format_device_display() {
        let dev = BlockDevice {
            name: "sda".into(),
            dev_path: "/dev/sda".into(),
            size_bytes: 120_000_000_000,
            model: Some("Samsung SSD 860 EVO".into()),
            boot_partition_uuid: None,
            root_partition_uuid: None,
        };
        let display = format_device(&dev);
        assert!(display.contains("/dev/sda"));
        assert!(display.contains("120 GB"));
        assert!(display.contains("Samsung SSD 860 EVO"));
    }

    #[test]
    fn format_device_no_model() {
        let dev = BlockDevice {
            name: "sda".into(),
            dev_path: "/dev/sda".into(),
            size_bytes: 120_000_000_000,
            model: None,
            boot_partition_uuid: None,
            root_partition_uuid: None,
        };
        let display = format_device(&dev);
        assert!(display.contains("Unknown"));
    }

    #[test]
    fn phantom_disks_filtered() {
        // fd0 with the kernel's 4096-byte "no media" placeholder, a 0-byte
        // card reader, and a real disk
        let json = r#"{
            "blockdevices": [
                {"name": "fd0", "size": 4096, "type": "disk", "model": null, "path": "/dev/fd0"},
                {"name": "fd1", "size": 0, "type": "disk", "model": null, "path": "/dev/fd1"},
                {"name": "mmcblk0", "size": 0, "type": "disk", "model": null, "path": "/dev/mmcblk0"},
                {"name": "sda", "size": 100000000000, "type": "disk", "model": "Real", "path": "/dev/sda"}
            ]
        }"#;
        let devices = parse_lsblk_json(json).unwrap();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].name, "sda");
    }

    #[test]
    fn null_model_filtered() {
        let json = r#"{
            "blockdevices": [
                {"name": "sda", "size": 100000000000, "type": "disk", "model": null, "path": "/dev/sda"},
                {"name": "sdb", "size": 200000000000, "type": "disk", "model": "  ", "path": "/dev/sdb"}
            ]
        }"#;
        let devices = parse_lsblk_json(json).unwrap();
        assert!(devices[0].model.is_none());
        // whitespace-only model is treated as None
        assert!(devices[1].model.is_none());
    }
}
