use anyhow::Result;

use crate::config::{EncryptionConfig, SystemConfig};
use crate::steps::StepResult;
use crate::ui::UserInterface;
use crate::ui_or_back;

pub fn step_encryption(
    ui: &mut dyn UserInterface,
    config: &mut SystemConfig,
) -> Result<StepResult> {
    let default = config.encryption.is_some();

    let encrypt = ui_or_back!(ui.confirm("Enable disk encryption (LUKS)?", default));

    config.encryption = if encrypt {
        let passphrase = loop {
            let pw = ui_or_back!(ui.password("LUKS passphrase"));
            let confirm = ui_or_back!(ui.password("Confirm LUKS passphrase"));
            if pw != confirm {
                ui.error("Passphrases do not match");
                continue;
            }
            if pw.is_empty() {
                ui.error("Passphrase must not be empty");
                continue;
            }
            break pw;
        };
        Some(EncryptionConfig {
            device_target: "cryptroot".into(),
            passphrase: Some(passphrase),
        })
    } else {
        None
    };

    Ok(StepResult::Next)
}
