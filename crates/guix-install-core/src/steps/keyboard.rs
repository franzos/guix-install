use anyhow::Result;

use crate::config::SystemConfig;
use crate::keyboard;
use crate::steps::StepResult;
use crate::ui::UserInterface;
use crate::ui_or_back;

/// Keyboard is locked (read-only) once a secret is entered, so a relaunch can
/// never lose a password/passphrase (those are never persisted).
pub fn secret_entered(config: &SystemConfig) -> bool {
    config.password.is_some()
        || config
            .encryption
            .as_ref()
            .and_then(|e| e.passphrase.as_ref())
            .is_some()
}

pub fn step_keyboard(ui: &mut dyn UserInterface, config: &mut SystemConfig) -> Result<StepResult> {
    let live = keyboard::current_live_layout();

    if secret_entered(config) {
        let current = config.keyboard_layout.clone().unwrap_or_else(|| live.clone());
        ui.info(&format!(
            "Keyboard layout: {current} (locked — passwords already entered)"
        ));
        return Ok(StepResult::Next);
    }

    let layouts = keyboard::layouts();
    let labels: Vec<String> = layouts
        .iter()
        .map(|l| format!("{} — {}", l.code, l.description))
        .collect();
    let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();

    let default_code = config.keyboard_layout.clone().unwrap_or_else(|| live.clone());
    let default_idx = layouts.iter().position(|l| l.code == default_code).unwrap_or(0);

    let idx = ui_or_back!(ui.select("Keyboard layout", &label_refs, default_idx));
    let chosen = layouts[idx].code.clone();
    config.keyboard_layout = Some(chosen.clone());

    // Apply only when it differs from the live layout. In the GUI this exits the
    // process for a cage relaunch (never returns); in the CLI it's a no-op.
    if chosen != live {
        ui.apply_keyboard_layout(&chosen, config)?;
    }
    Ok(StepResult::Next)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locked_once_user_password_set() {
        let mut config = SystemConfig::default();
        assert!(!secret_entered(&config));
        config.password = Some(zeroize::Zeroizing::new("x".into()));
        assert!(secret_entered(&config));
    }

    #[test]
    fn locked_once_luks_passphrase_set() {
        let mut config = SystemConfig::default();
        config.encryption = Some(crate::config::EncryptionConfig {
            device_target: "cryptroot".into(),
            passphrase: Some(zeroize::Zeroizing::new("y".into())),
        });
        assert!(secret_entered(&config));
    }
}
