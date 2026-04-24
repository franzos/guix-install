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
        Some(EncryptionConfig {
            device_target: "cryptroot".into(),
        })
    } else {
        None
    };

    Ok(StepResult::Next)
}
