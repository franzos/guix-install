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
        if pw.is_empty() {
            ui.error("Password must not be empty");
            continue;
        }
        if pw.len() < 8 {
            ui.warn("Password is shorter than 8 characters");
        }
        let confirm = ui_or_back!(ui.password("Confirm password"));
        if pw != confirm {
            ui.error("Passwords do not match");
            continue;
        }
        break pw;
    };
    config.password = Some(password);

    let ssh = ui_or_back!(ui.input("SSH public key (optional, press Enter to skip)", ""));
    config.ssh_key = if ssh.is_empty() { None } else { Some(ssh) };

    config.users = vec![UserAccount {
        name: username.clone(),
        comment: format!("{username}'s account"),
        groups: vec!["wheel".into(), "audio".into(), "video".into()],
    }];

    Ok(StepResult::Next)
}
