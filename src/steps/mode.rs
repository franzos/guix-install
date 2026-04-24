use anyhow::Result;

use crate::config::{SystemConfig, validate_config_id};
use crate::mode::InstallMode;
use crate::steps::StepResult;
use crate::ui::UserInterface;
use crate::ui_or_back;

const MODE_OPTIONS: &[&str] = &[
    "Plain Guix (libre only)",
    "Nonguix (nonfree drivers)",
    "Panther (PantherX OS)",
    "Enterprise (remote config)",
];

pub fn step_mode(ui: &mut dyn UserInterface, config: &mut SystemConfig) -> Result<StepResult> {
    let default = match &config.mode {
        InstallMode::Guix => 0,
        InstallMode::Nonguix => 1,
        InstallMode::Panther => 2,
        InstallMode::Enterprise { .. } => 3,
    };

    let choice = ui_or_back!(ui.select("Installation mode", MODE_OPTIONS, default));

    config.mode = match choice {
        0 => InstallMode::Guix,
        1 => InstallMode::Nonguix,
        3 => {
            let config_id = loop {
                let id = ui_or_back!(ui.input("Config ID", ""));
                match validate_config_id(&id) {
                    Ok(()) => break id,
                    Err(e) => ui.error(&e),
                }
            };

            let config_url =
                ui_or_back!(ui.input("Config URL", "https://temp.pantherx.org/install",));

            if !config_url.starts_with("https://") {
                ui.warn("Config URL should use HTTPS");
            }

            InstallMode::Enterprise {
                config_id,
                config_url,
            }
        }
        _ => InstallMode::Panther,
    };

    Ok(StepResult::Next)
}
