use anyhow::Result;

use crate::config::SystemConfig;
use crate::steps::StepResult;
use crate::ui::UserInterface;
use crate::ui_or_back;

pub fn step_timezone(ui: &mut dyn UserInterface, config: &mut SystemConfig) -> Result<StepResult> {
    let mut options: Vec<&str> = crate::steps::data::timezones();
    let default = match options.iter().position(|t| *t == config.timezone.as_str()) {
        Some(i) => i,
        None => {
            options.insert(0, config.timezone.as_str());
            0
        }
    };
    let idx = ui_or_back!(ui.select("Timezone", &options, default));
    let chosen = options[idx].to_string();
    config.timezone = chosen;
    Ok(StepResult::Next)
}
