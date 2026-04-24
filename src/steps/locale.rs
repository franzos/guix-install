use anyhow::Result;

use crate::config::SystemConfig;
use crate::steps::StepResult;
use crate::ui::UserInterface;
use crate::ui_or_back;

pub fn step_locale(ui: &mut dyn UserInterface, config: &mut SystemConfig) -> Result<StepResult> {
    loop {
        let locale = ui_or_back!(ui.input("Locale", &config.locale));

        if locale.contains('.') || locale.contains('_') {
            config.locale = locale;
            return Ok(StepResult::Next);
        }

        ui.error("Locale must contain a dot or underscore (e.g. en_US.utf8)");
    }
}
