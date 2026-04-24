use anyhow::Result;

use crate::config::SystemConfig;
use crate::steps::StepResult;
use crate::ui::UserInterface;
use crate::ui_or_back;

pub fn step_timezone(ui: &mut dyn UserInterface, config: &mut SystemConfig) -> Result<StepResult> {
    loop {
        let tz = ui_or_back!(ui.input("Timezone", &config.timezone));

        if !tz.is_empty() && tz.contains('/') {
            config.timezone = tz;
            return Ok(StepResult::Next);
        }

        ui.error("Timezone must contain a '/' (e.g. Europe/Berlin)");
    }
}
