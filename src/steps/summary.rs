use anyhow::Result;

use crate::config::SystemConfig;
use crate::disk::format_size;
use crate::mode::InstallMode;
use crate::steps::StepResult;
use crate::ui::UserInterface;

pub fn step_summary(ui: &mut dyn UserInterface, config: &SystemConfig) -> Result<StepResult> {
    let mode_label = match &config.mode {
        InstallMode::Guix => "Guix (libre only)".to_string(),
        InstallMode::Nonguix => "Nonguix (nonfree drivers)".to_string(),
        InstallMode::Panther => "Panther (PantherX OS)".to_string(),
        InstallMode::Enterprise { config_id, .. } => format!("Enterprise ({config_id})"),
    };

    let disk_size = format_size(config.disk.size_bytes);
    let disk_label = format!("{} ({disk_size})", config.disk.dev_path);

    let encryption = if config.encryption.is_some() {
        "Yes (LUKS)"
    } else {
        "No"
    };

    let desktop = config
        .desktop
        .as_ref()
        .map(|d| format!("{d}"))
        .unwrap_or_else(|| "None (headless)".into());

    ui.info("");
    ui.info("=== Installation Summary ===");
    ui.info(&format!("Mode:        {mode_label}"));
    ui.info(&format!("Disk:        {disk_label}"));
    ui.info(&format!("Filesystem:  {}", config.filesystem));
    ui.info(&format!("Encryption:  {encryption}"));

    if !matches!(config.mode, InstallMode::Enterprise { .. }) {
        ui.info(&format!("Hostname:    {}", config.hostname));
        ui.info(&format!("Timezone:    {}", config.timezone));
        ui.info(&format!("Locale:      {}", config.locale));
        let username = config.users.first().map(|u| u.name.as_str()).unwrap_or("-");
        ui.info(&format!("Username:    {username}"));
        ui.info(&format!("Desktop:     {desktop}"));
    }

    ui.info("");
    ui.warn(&format!(
        "{} will be formatted. ALL DATA WILL BE LOST.",
        config.disk.dev_path
    ));
    ui.info("");

    let proceed = ui.confirm("Proceed with installation?", false)?;

    if proceed {
        Ok(StepResult::Next)
    } else {
        Ok(StepResult::Back)
    }
}
