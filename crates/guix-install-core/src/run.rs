use anyhow::Result;

use crate::config::SystemConfig;
use crate::install;
use crate::resume::InstallState;
use crate::scheme::operating_system::render_operating_system;
use crate::steps::desktop::step_desktop;
use crate::steps::disk::step_disk;
use crate::steps::encryption::step_encryption;
use crate::steps::hostname::step_hostname;
use crate::steps::keyboard::step_keyboard;
use crate::steps::locale::step_locale;
use crate::steps::mode::step_mode;
use crate::steps::network::step_network;
use crate::steps::summary::step_summary;
use crate::steps::timezone::step_timezone;
use crate::steps::users::step_users;
use crate::steps::{StepId, StepNavigator, StepResult};
use crate::ui::UserInterface;

/// On startup, if a saved install state exists, ask the user whether to resume
/// it or discard and start fresh. Returns `Some(config)` if resuming, `None` if
/// starting fresh.
pub fn handle_resume(
    ui: &mut dyn UserInterface,
    state: InstallState,
) -> Result<Option<SystemConfig>> {
    let last = state.completed_phases.last().copied().unwrap_or(0);
    let mode = state.config.mode.label();
    let disk = &state.config.disk.dev_path;

    ui.info(&format!(
        "Found incomplete installation: completed through phase {last}/8 \
         (mode={mode}, disk={disk})."
    ));

    let options = &["Resume previous installation", "Discard and start fresh"];
    let choice = ui.select("What next?", options, 0)?;
    if choice != 0 {
        InstallState::cleanup()?;
        ui.info("Discarded previous state.");
        return Ok(None);
    }

    let mut config = state.config;
    if !state.completed_phases.contains(&8) {
        let pw = ui.password("User password (re-enter to resume install)")?;
        config.password = Some(pw);
    }

    // The LUKS passphrase is `#[serde(skip)]`, so it's gone after a restart.
    // Re-prompt only if the format phase (2) hasn't run yet — once partition 2
    // is LUKS-formatted and the mapper is open, later phases don't need it.
    if let Some(enc) = config.encryption.as_mut()
        && !state.completed_phases.contains(&2)
    {
        let pass = ui.password("LUKS passphrase (re-enter to resume install)")?;
        enc.passphrase = Some(pass);
    }

    Ok(Some(config))
}

/// Shared interactive run loop driven by any [`UserInterface`] implementation
/// (CLI `Repl`, GUI `IcedUi`, …). Collects a [`SystemConfig`] via the step
/// navigator, prints the generated config, and runs the install unless
/// `dry_run` is set.
pub fn run_interactive(ui: &mut dyn UserInterface, dry_run: bool) -> Result<()> {
    ui.info("guix-install — Guix System Installer");
    ui.info("");

    // A GUI keyboard-relaunch leaves the in-progress config here; restore it first
    // so a keyboard relaunch is never mistaken for an interrupted-install resume.
    let restored = crate::resume::take_interview_state();
    if restored.is_none()
        && !dry_run
        && let Some(state) = InstallState::load()?
        && let Some(config) = handle_resume(ui, state)?
    {
        return install::execute_installation(&config, ui);
    }

    let mut config = restored.unwrap_or_default();
    let mut nav = StepNavigator::new(&config.mode);
    let mut came_from_back = false;

    loop {
        ui.set_steps(nav.steps(), nav.current_index());
        let result = match nav.current() {
            StepId::Keyboard => step_keyboard(ui, &mut config)?,
            StepId::Network => step_network(ui, came_from_back, &config.mode)?,
            StepId::Mode => {
                let old_mode = config.mode.clone();
                let r = step_mode(ui, &mut config)?;
                if config.mode != old_mode {
                    nav.reset_for_mode(&config.mode);
                    came_from_back = false;
                    continue;
                }
                r
            }
            StepId::Locale => step_locale(ui, &mut config)?,
            StepId::Timezone => step_timezone(ui, &mut config)?,
            StepId::Hostname => step_hostname(ui, &mut config)?,
            StepId::Disk => step_disk(ui, &mut config)?,
            StepId::Encryption => step_encryption(ui, &mut config)?,
            StepId::Users => step_users(ui, &mut config)?,
            StepId::Desktop => step_desktop(ui, &mut config)?,
            StepId::Summary => step_summary(ui, &mut config)?,
        };

        match result {
            StepResult::Next => {
                if nav.is_last() {
                    break;
                }
                came_from_back = false;
                nav.advance();
            }
            StepResult::Back => {
                came_from_back = true;
                nav.go_back();
            }
            StepResult::Quit => {
                ui.info("Installation cancelled.");
                return Ok(());
            }
        }
    }

    let system_scm = render_operating_system(&config);
    if !system_scm.is_empty() {
        println!("\n;;; Generated system.scm:");
        println!("{system_scm}");
    }

    if dry_run {
        ui.info("Dry run: skipping installation execution.");
        return Ok(());
    }

    install::execute_installation(&config, ui)?;
    Ok(())
}
