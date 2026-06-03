use anyhow::Result;

use crate::config::{SystemConfig, validate_config_id};
use crate::hardware;
use crate::mode::InstallMode;
use crate::steps::StepResult;
use crate::ui::UserInterface;
use crate::ui_or_back;

const MODE_OPTIONS: &[&str] = &[
    "guix: Libre only",
    "nonguix: Nonfree kernels and applications (includes guix)",
    "panther: Recommended for most users (includes guix, nonguix)",
    "enterprise: From remote config",
];

pub fn step_mode(ui: &mut dyn UserInterface, config: &mut SystemConfig) -> Result<StepResult> {
    let default = match &config.mode {
        InstallMode::Guix => 0,
        InstallMode::Nonguix => 1,
        InstallMode::Panther => 2,
        InstallMode::Enterprise { .. } => 3,
    };

    loop {
        let choice = ui_or_back!(ui.select("Installation channels", MODE_OPTIONS, default));

        let new_mode = match choice {
            0 => {
                if !confirm_libre_compatibility(ui)? {
                    continue;
                }
                InstallMode::Guix
            }
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

        config.mode = new_mode;
        return Ok(StepResult::Next);
    }
}

/// Warn the user about devices that won't work without non-free firmware.
/// Returns `Ok(true)` if the user wants to proceed with Guix mode anyway.
fn confirm_libre_compatibility(ui: &mut dyn UserInterface) -> Result<bool> {
    let unsupported = hardware::detect_unsupported_devices();
    if !unsupported.is_empty() {
        let list = unsupported
            .iter()
            .map(|d| format!("  - {}", d.description()))
            .collect::<Vec<_>>()
            .join("\n");
        ui.warn(&format!(
            "These devices need non-free firmware and won't work with libre Guix:\n{list}\n\
             Consider 'nonguix' or 'panther' if you need them."
        ));
        let proceed = match ui.confirm("Continue with libre-only Guix?", false) {
            Ok(v) => v,
            Err(e) if crate::ui::is_cancelled(&e) => return Ok(false),
            Err(e) => return Err(e),
        };
        if !proceed {
            return Ok(false);
        }
    }

    if hardware::uvesafb_loaded() {
        ui.warn(
            "uvesafb is loaded — your GPU may need 'nomodeset' as a kernel argument post-install.",
        );
    }

    Ok(true)
}
