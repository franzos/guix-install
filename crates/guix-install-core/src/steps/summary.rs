use anyhow::Result;

use crate::config::{Firmware, SystemConfig, validate_ssh_public_key};
use crate::disk::format_size;
use crate::mode::InstallMode;
use crate::scheme;
use crate::steps::StepResult;
use crate::ui::{SummaryData, SummaryRow, SummarySection, UserInterface};

const MAIN_OPTIONS: &[&str] = &[
    "Advanced configuration",
    "Proceed with installation",
    "Cancel installation",
];

pub fn step_summary(ui: &mut dyn UserInterface, config: &mut SystemConfig) -> Result<StepResult> {
    loop {
        ui.summary(&build_summary_data(config));

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

fn build_summary_data(config: &SystemConfig) -> SummaryData {
    let enterprise = matches!(config.mode, InstallMode::Enterprise { .. });

    let mode_label = match &config.mode {
        InstallMode::Guix => "Guix (libre only)".to_string(),
        InstallMode::Nonguix => "Nonguix (nonfree drivers)".to_string(),
        InstallMode::Panther => "Panther (PantherX OS)".to_string(),
        InstallMode::Enterprise { config_id, .. } => format!("Enterprise ({config_id})"),
    };

    let firmware_label = match config.firmware {
        Firmware::Efi => "UEFI",
        Firmware::Bios => "BIOS (legacy)",
    };

    let mut system = vec![
        SummaryRow::new("Mode", mode_label),
        SummaryRow::new("Firmware", format!("{firmware_label} (auto-detected)")),
    ];
    if !enterprise {
        let keyboard = config
            .keyboard_layout
            .clone()
            .unwrap_or_else(|| "default".into());
        system.push(SummaryRow::new("Hostname", config.hostname.clone()));
        system.push(SummaryRow::new("Timezone", config.timezone.clone()));
        system.push(SummaryRow::new("Locale", config.locale.clone()));
        system.push(SummaryRow::new("Keyboard", keyboard));
    }

    let swap = if config.swap_size_mb == 0 {
        "None".to_string()
    } else {
        format!("{} MB", config.swap_size_mb)
    };
    let encryption = if config.encryption.is_some() {
        "Yes (LUKS)"
    } else {
        "No"
    };
    let storage = vec![
        SummaryRow::new(
            "Disk",
            format!(
                "{} ({})",
                config.disk.dev_path,
                format_size(config.disk.size_bytes)
            ),
        ),
        SummaryRow::new("Filesystem", config.filesystem.to_string()),
        SummaryRow::new("Encryption", encryption),
        SummaryRow::new("Swap", swap),
    ];

    let mut sections = vec![
        SummarySection {
            title: "System".into(),
            rows: system,
        },
        SummarySection {
            title: "Storage".into(),
            rows: storage,
        },
    ];

    if !enterprise {
        let username = config
            .users
            .first()
            .map(|u| u.name.clone())
            .unwrap_or_else(|| "-".into());
        let desktop = config
            .desktop
            .as_ref()
            .map(|d| format!("{d}"))
            .unwrap_or_else(|| "None (headless)".into());
        let ssh = if config.ssh_key.is_some() {
            "Set"
        } else {
            "Not set"
        };
        let custom = if config.system_scm_override.is_some() {
            "Yes"
        } else {
            "No"
        };
        sections.push(SummarySection {
            title: "Account".into(),
            rows: vec![
                SummaryRow::new("User", username),
                SummaryRow::new("Desktop", desktop),
                SummaryRow::new("SSH key", ssh),
                SummaryRow::new("Custom config", custom),
            ],
        });
    }

    let note = config.system_scm_override.is_some().then(|| {
        "Custom system.scm in use — fields below describe the current config \
         but the installer will write your edited version verbatim and may \
         not reflect them."
            .to_string()
    });

    SummaryData {
        note,
        sections,
        layout: partition_preview(config),
        warning: format!(
            "{} will be formatted. ALL DATA WILL BE LOST.",
            config.disk.dev_path
        ),
    }
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

    let edited = match ui.edit_text("Edit system.scm", &initial)? {
        Some(edited) => edited,
        None => return Ok(()),
    };

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
