use anyhow::{Context, Result};

use crate::config::{Firmware, SystemConfig, validate_ssh_public_key};
use crate::disk::format_size;
use crate::exec;
use crate::mode::InstallMode;
use crate::scheme;
use crate::steps::StepResult;
use crate::ui::UserInterface;

const MAIN_OPTIONS: &[&str] = &[
    "Advanced configuration",
    "Proceed with installation",
    "Cancel installation",
];

const EDIT_PATH: &str = "/tmp/guix-install-edit.scm";

pub fn step_summary(ui: &mut dyn UserInterface, config: &mut SystemConfig) -> Result<StepResult> {
    loop {
        print_summary(ui, config);

        let choice = match ui.select("What next?", MAIN_OPTIONS, 1) {
            Ok(c) => c,
            Err(e) if crate::ui::is_cancelled(&e) => {
                if config.system_scm_override.is_some() {
                    ui.warn(
                        "Cannot go back — you have a custom system.scm. \
                         Discard it from Advanced configuration first.",
                    );
                    continue;
                }
                return Ok(StepResult::Back);
            }
            Err(e) => return Err(e),
        };
        match choice {
            0 => advanced_menu(ui, config)?,
            1 => return Ok(StepResult::Next),
            _ => return Ok(StepResult::Quit),
        }
    }
}

fn print_summary(ui: &dyn UserInterface, config: &SystemConfig) {
    let mode_label = match &config.mode {
        InstallMode::Guix => "Guix (libre only)".to_string(),
        InstallMode::Nonguix => "Nonguix (nonfree drivers)".to_string(),
        InstallMode::Panther => "Panther (PantherX OS)".to_string(),
        InstallMode::Enterprise { config_id, .. } => format!("Enterprise ({config_id})"),
    };

    let disk_size = format_size(config.disk.size_bytes);
    let disk_label = format!("{} ({disk_size})", config.disk.dev_path);

    let firmware_label = match config.firmware {
        Firmware::Efi => "UEFI",
        Firmware::Bios => "BIOS (legacy)",
    };

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

    if config.system_scm_override.is_some() {
        ui.warn(
            "Custom system.scm in use — fields below describe the current config \
             but the installer will write your edited version verbatim and may \
             not reflect them.",
        );
        ui.info("");
    }

    ui.info(&format!("Mode:           {mode_label}"));
    ui.info(&format!("Firmware:       {firmware_label} (auto-detected)"));
    ui.info(&format!("Disk:           {disk_label}"));
    ui.info(&format!("Filesystem:     {}", config.filesystem));
    ui.info(&format!("Encryption:     {encryption}"));

    if !matches!(config.mode, InstallMode::Enterprise { .. }) {
        ui.info(&format!("Hostname:       {}", config.hostname));
        ui.info(&format!("Timezone:       {}", config.timezone));
        ui.info(&format!("Locale:         {}", config.locale));
        let username = config.users.first().map(|u| u.name.as_str()).unwrap_or("-");
        ui.info(&format!("Username:       {username}"));
        ui.info(&format!("Desktop:        {desktop}"));
        let ssh = if config.ssh_key.is_some() {
            "Set"
        } else {
            "Not set"
        };
        ui.info(&format!("SSH key:        {ssh}"));
        let custom = if config.system_scm_override.is_some() {
            "Yes"
        } else {
            "No"
        };
        ui.info(&format!("Custom config:  {custom}"));
    }

    ui.info("");
    ui.info("Partition layout:");
    for line in partition_preview(config) {
        ui.info(&format!("  {line}"));
    }

    ui.info("");
    ui.warn(&format!(
        "{} will be formatted. ALL DATA WILL BE LOST.",
        config.disk.dev_path
    ));
    ui.info("");
}

fn partition_preview(config: &SystemConfig) -> Vec<String> {
    let dev = &config.disk.dev_path;
    let part = |n: u32| crate::disk::partition_path(dev, n);
    let root_label = if config.encryption.is_some() {
        format!("{} → LUKS → {} root", part(2), config.filesystem)
    } else {
        format!("{} {} root (rest of disk)", part(2), config.filesystem)
    };
    match config.firmware {
        Firmware::Efi => vec![
            format!("{} 200 MB EFI System Partition (FAT32)", part(1)),
            root_label,
        ],
        Firmware::Bios => vec![format!("{} 10 MB BIOS GRUB boot", part(1)), root_label],
    }
}

fn advanced_menu(ui: &mut dyn UserInterface, config: &mut SystemConfig) -> Result<()> {
    loop {
        let mut options: Vec<&str> = vec!["SSH public key", "Edit system.scm"];
        if config.system_scm_override.is_some() {
            options.push("Discard custom system.scm");
        }
        options.push("Back to summary");

        let default = options.len() - 1;
        let choice = match ui.select("Advanced configuration", &options, default) {
            Ok(c) => c,
            Err(e) if crate::ui::is_cancelled(&e) => return Ok(()),
            Err(e) => return Err(e),
        };
        match options[choice] {
            "SSH public key" => prompt_ssh_key(ui, config)?,
            "Edit system.scm" => edit_system_scm(ui, config)?,
            "Discard custom system.scm" => discard_override(ui, config),
            _ => return Ok(()),
        }
    }
}

fn prompt_ssh_key(ui: &mut dyn UserInterface, config: &mut SystemConfig) -> Result<()> {
    let current = config.ssh_key.as_deref().unwrap_or("");
    loop {
        let ssh = match ui.input("SSH public key (empty to clear)", current) {
            Ok(v) => v,
            Err(e) if crate::ui::is_cancelled(&e) => return Ok(()),
            Err(e) => return Err(e),
        };
        if ssh.trim().is_empty() {
            config.ssh_key = None;
            return Ok(());
        }
        match validate_ssh_public_key(&ssh) {
            Ok(()) => {
                config.ssh_key = Some(ssh.trim().to_string());
                return Ok(());
            }
            Err(e) => ui.error(&e),
        }
    }
}

fn edit_system_scm(ui: &mut dyn UserInterface, config: &mut SystemConfig) -> Result<()> {
    if matches!(config.mode, InstallMode::Enterprise { .. }) {
        ui.error(
            "Enterprise mode pulls system.scm from the remote config tarball — \
             nothing to edit locally.",
        );
        return Ok(());
    }

    let initial = config
        .system_scm_override
        .clone()
        .unwrap_or_else(|| scheme::operating_system::render_operating_system(config));

    std::fs::write(EDIT_PATH, &initial).with_context(|| format!("write {EDIT_PATH}"))?;

    if !launch_editor(ui, EDIT_PATH) {
        let _ = std::fs::remove_file(EDIT_PATH);
        return Ok(());
    }

    let edited = std::fs::read_to_string(EDIT_PATH).with_context(|| format!("read {EDIT_PATH}"))?;
    let _ = std::fs::remove_file(EDIT_PATH);

    if edited == initial && config.system_scm_override.is_none() {
        ui.info("No changes — discarding.");
        return Ok(());
    }
    config.system_scm_override = Some(edited);
    ui.info("Custom system.scm saved. It will be written verbatim during install.");
    Ok(())
}

fn discard_override(ui: &dyn UserInterface, config: &mut SystemConfig) {
    config.system_scm_override = None;
    ui.info("Custom system.scm discarded — installer will render fresh from config.");
}

/// Launches an editor on `path`. Returns true on a successful spawn (regardless
/// of editor exit code), false if no editor could be spawned.
fn launch_editor(ui: &dyn UserInterface, path: &str) -> bool {
    let mut candidates: Vec<String> = Vec::new();
    candidates.extend(std::env::var("EDITOR").ok());
    candidates.extend(std::env::var("VISUAL").ok());
    candidates.push("nano".into());
    candidates.push("vi".into());

    for editor in &candidates {
        ui.info(&format!("Opening {path} in {editor}..."));
        if exec::run_cmd_interactive(&[editor, path]).is_ok() {
            return true;
        }
    }
    ui.error(&format!(
        "No editor available (set $EDITOR or install nano/vi). Edit {path} manually."
    ));
    false
}
