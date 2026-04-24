use anyhow::Result;

use crate::config::{SystemConfig, generate_hostname, validate_hostname};
use crate::steps::StepResult;
use crate::ui::UserInterface;
use crate::ui_or_back;

pub fn step_hostname(ui: &mut dyn UserInterface, config: &mut SystemConfig) -> Result<StepResult> {
    let default = if config.hostname.is_empty() {
        generate_hostname(&config.mode)
    } else {
        config.hostname.clone()
    };

    loop {
        let name = ui_or_back!(ui.input("Hostname", &default));

        match validate_hostname(&name) {
            Ok(()) => {
                config.hostname = name;
                return Ok(StepResult::Next);
            }
            Err(e) => ui.error(&e),
        }
    }
}
