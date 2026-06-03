use anyhow::Result;

use crate::config::{SystemConfig, UserAccount, validate_username};
use crate::steps::StepResult;
use crate::ui::UserInterface;
use crate::ui_or_back;

pub fn step_users(ui: &mut dyn UserInterface, config: &mut SystemConfig) -> Result<StepResult> {
    let current_name = config
        .users
        .first()
        .map(|u| u.name.as_str())
        .unwrap_or("panther");

    let username = loop {
        let name = ui_or_back!(ui.input("Username", current_name));
        match validate_username(&name) {
            Ok(()) => break name,
            Err(e) => ui.error(&e),
        }
    };

    let password = loop {
        let pw = ui_or_back!(ui.password("Password"));
        let confirm = ui_or_back!(ui.password("Confirm password"));
        if pw != confirm {
            ui.error("Passwords do not match");
            continue;
        }
        break pw;
    };
    config.password = Some(password);

    config.users = vec![UserAccount {
        name: username.clone(),
        comment: format!("{username}'s account"),
        groups: vec!["wheel".into(), "audio".into(), "video".into()],
    }];

    Ok(StepResult::Next)
}
