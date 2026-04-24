use anyhow::Result;

use crate::config::{Filesystem, SystemConfig};
use crate::disk::detect::{detect_block_devices, format_device};
use crate::steps::StepResult;
use crate::ui::UserInterface;
use crate::ui_or_back;

pub fn step_disk(ui: &mut dyn UserInterface, config: &mut SystemConfig) -> Result<StepResult> {
    let devices = detect_block_devices()?;

    if devices.is_empty() {
        ui.error("No disks found. Cannot continue.");
        return Ok(StepResult::Quit);
    }

    let labels: Vec<String> = devices.iter().map(format_device).collect();
    let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();

    let default_idx = devices
        .iter()
        .position(|d| d.dev_path == config.disk.dev_path)
        .unwrap_or(0);

    let choice = ui_or_back!(ui.select("Target disk", &label_refs, default_idx));
    config.disk = devices[choice].clone();

    let fs_options = &["ext4", "btrfs"];
    let fs_default = match config.filesystem {
        Filesystem::Btrfs => 1,
        _ => 0,
    };
    let fs_choice = ui_or_back!(ui.select("Filesystem", fs_options, fs_default));
    config.filesystem = match fs_choice {
        1 => Filesystem::Btrfs,
        _ => Filesystem::Ext4,
    };

    Ok(StepResult::Next)
}
