use anyhow::Result;

use crate::config::SystemConfig;
use crate::steps::StepResult;
use crate::ui::UserInterface;
use crate::ui_or_back;

pub fn step_locale(ui: &mut dyn UserInterface, config: &mut SystemConfig) -> Result<StepResult> {
    let locales = crate::steps::data::locales();
    let mut labels: Vec<&str> = locales.iter().map(|l| l.label).collect();
    let mut codes: Vec<&str> = locales.iter().map(|l| l.code).collect();
    let default = match codes.iter().position(|c| *c == config.locale.as_str()) {
        Some(i) => i,
        None => {
            labels.insert(0, config.locale.as_str());
            codes.insert(0, config.locale.as_str());
            0
        }
    };
    let idx = ui_or_back!(ui.select("Locale", &labels, default));
    let chosen = codes[idx].to_string();
    config.locale = chosen;
    Ok(StepResult::Next)
}
