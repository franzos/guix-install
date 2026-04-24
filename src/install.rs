use std::path::Path;

use anyhow::Result;
use zeroize::Zeroizing;

use crate::config::SystemConfig;
use crate::enterprise;
use crate::mode::InstallMode;
use crate::passwd;
use crate::resume::InstallState;
use crate::ui::UserInterface;
use crate::{disk, exec, scheme};

const NONGUIX_KEY: &str = include_str!("../keys/substitutes.nonguix.org.pub");
const PANTHERX_KEY: &str = include_str!("../keys/packages.pantherx.org.pub");

/// Checks network connectivity before starting installation.
///
/// Uses `guix describe` as a lightweight probe — it contacts substitute servers.
/// On failure, warns but does not block (offline installs from ISO are valid).
fn check_connectivity(ui: &dyn UserInterface) {
    ui.info("Checking network connectivity...");
    match exec::run_cmd(&["guix", "describe"]) {
        Ok(_) => {
            ui.info("  Network OK.");
        }
        Err(_) => {
            ui.warn("Network check failed. Installation may fail if substitutes are unavailable.");
            ui.warn("Run 'guix-install wifi' to connect to WiFi, or ensure a wired connection.");
        }
    }
}

/// Runs the full 8-phase installation sequence.
///
/// Supports resume: if a previous state file exists, already-completed phases
/// are skipped. State is persisted after each phase so a crash or power loss
/// only requires re-running the current phase.
pub fn execute_installation(config: &SystemConfig, ui: &dyn UserInterface) -> Result<()> {
    check_connectivity(ui);

    let mut state = match InstallState::load()? {
        Some(existing) => {
            let last = existing.completed_phases.last().copied().unwrap_or(0);

            if existing.config.disk.dev_path != config.disk.dev_path
                || existing.config.firmware != config.firmware
                || existing.config.mode != config.mode
            {
                ui.warn("Previous installation state found but config differs. Starting fresh.");
                InstallState::new(config)
            } else {
                ui.info(&format!(
                    "Found previous installation state (completed through phase {last}/8). Resuming."
                ));
                existing
            }
        }
        None => InstallState::new(config),
    };

    type PhaseFn = fn(&SystemConfig, &dyn UserInterface) -> Result<()>;
    let phases: &[(u8, PhaseFn)] = &[
        (1, phase_partition),
        (2, phase_format),
        (3, phase_mount),
        (4, phase_swap),
        (5, phase_config),
        (6, phase_authorize),
        (7, phase_pull),
        (8, phase_install),
    ];

    for (num, func) in phases {
        if state.completed_phases.contains(num) {
            ui.info(&format!("Phase {num}/8: Already completed, skipping."));
            continue;
        }

        func(config, ui).map_err(|e| e.context(format!("Phase {num}/8 failed")))?;
        state.mark_complete(*num);
        state.save()?;
    }

    InstallState::cleanup()?;
    Ok(())
}

fn phase_partition(config: &SystemConfig, ui: &dyn UserInterface) -> Result<()> {
    ui.info("Phase 1/8: Partitioning disk...");
    let cmds = disk::partition::partition_commands(&config.disk.dev_path, &config.firmware);
    for cmd in &cmds {
        let args: Vec<&str> = cmd.iter().map(|s| s.as_str()).collect();
        exec::run_cmd(&args)?;
    }
    Ok(())
}

fn phase_format(config: &SystemConfig, ui: &dyn UserInterface) -> Result<()> {
    ui.info("Phase 2/8: Formatting partitions...");
    let cmds = disk::format::format_commands(config);
    for cmd in &cmds {
        let args: Vec<&str> = cmd.iter().map(|s| s.as_str()).collect();
        if cmd[0] == "cryptsetup" {
            let exit = exec::run_cmd_interactive(&args)?;
            if exit != 0 {
                anyhow::bail!("cryptsetup failed with exit code {exit}");
            }
        } else {
            exec::run_cmd(&args)?;
        }
    }
    Ok(())
}

fn phase_mount(config: &SystemConfig, ui: &dyn UserInterface) -> Result<()> {
    ui.info("Phase 3/8: Mounting filesystems...");
    for action in disk::mount::mount_actions(config) {
        action.execute()?;
    }
    Ok(())
}

fn phase_swap(config: &SystemConfig, ui: &dyn UserInterface) -> Result<()> {
    ui.info("Phase 4/8: Creating swap...");
    for action in disk::mount::swap_actions(config) {
        action.execute()?;
    }
    Ok(())
}

fn phase_config(config: &SystemConfig, ui: &dyn UserInterface) -> Result<()> {
    ui.info("Phase 5/8: Writing system configuration...");
    std::fs::create_dir_all("/mnt/etc/guix")?;

    if let InstallMode::Enterprise {
        ref config_id,
        ref config_url,
    } = config.mode
    {
        ui.info("  Fetching enterprise configuration...");
        let ent = enterprise::fetch_enterprise_config(config_id, config_url)?;

        std::fs::write("/mnt/etc/system.scm", &ent.system_scm)?;
        ui.info("  Wrote /mnt/etc/system.scm (from remote config)");

        if let Some(channels) = &ent.channels_scm {
            std::fs::write("/mnt/etc/guix/channels.scm", channels)?;
            ui.info("  Wrote /mnt/etc/guix/channels.scm (from remote config)");
        }

        enterprise::cleanup();
    } else {
        let system_scm = scheme::operating_system::render_operating_system(config);
        std::fs::write("/mnt/etc/system.scm", &system_scm)?;
        ui.info("  Wrote /mnt/etc/system.scm");

        if let Some(channels) = scheme::channels::render_channels(&config.mode) {
            std::fs::write("/mnt/etc/guix/channels.scm", &channels)?;
            ui.info("  Wrote /mnt/etc/guix/channels.scm");
        }
    }

    Ok(())
}

fn phase_authorize(config: &SystemConfig, ui: &dyn UserInterface) -> Result<()> {
    ui.info("Phase 6/8: Authorizing substitute servers...");

    match &config.mode {
        InstallMode::Guix => {
            ui.info("  No additional substitute servers needed.");
        }
        InstallMode::Nonguix => {
            authorize_substitute("substitutes.nonguix.org", NONGUIX_KEY, ui)?;
        }
        InstallMode::Panther => {
            authorize_substitute("substitutes.nonguix.org", NONGUIX_KEY, ui)?;
            authorize_substitute("packages.pantherx.org", PANTHERX_KEY, ui)?;
        }
        InstallMode::Enterprise { .. } => {
            authorize_substitute("substitutes.nonguix.org", NONGUIX_KEY, ui)?;
            authorize_substitute("packages.pantherx.org", PANTHERX_KEY, ui)?;
        }
    }
    Ok(())
}

fn authorize_substitute(name: &str, key: &str, ui: &dyn UserInterface) -> Result<()> {
    ui.info(&format!("  Authorizing {name}..."));
    exec::run_cmd_with_stdin(&["guix", "archive", "--authorize"], key)?;
    Ok(())
}

fn phase_pull(config: &SystemConfig, ui: &dyn UserInterface) -> Result<()> {
    ui.info("Phase 7/8: Checking channel availability...");

    if matches!(config.mode, InstallMode::Guix) {
        ui.info("  Using default channels, no pull needed.");
        return Ok(());
    }

    // Enterprise always pulls (channels from remote config may not be on ISO)
    let must_pull = matches!(config.mode, InstallMode::Enterprise { .. });

    if !must_pull && channels_available(config) {
        ui.info("  Required channels already available, skipping pull.");
        return Ok(());
    }

    ui.info("  Running guix pull (this may take a while)...");
    let exit =
        exec::run_cmd_interactive(&["guix", "pull", "--channels=/mnt/etc/guix/channels.scm"])?;
    if exit != 0 {
        anyhow::bail!("guix pull failed with exit code {exit}");
    }

    Ok(())
}

/// Tests whether the required Guile modules for the selected mode are loadable.
fn channels_available(config: &SystemConfig) -> bool {
    let test_module = match &config.mode {
        InstallMode::Guix => return true,
        InstallMode::Nonguix => "(use-modules (nongnu packages linux))",
        InstallMode::Panther | InstallMode::Enterprise { .. } => {
            "(use-modules (px system panther))"
        }
    };

    exec::run_cmd(&["guile", "-c", test_module]).is_ok()
}

fn phase_install(config: &SystemConfig, ui: &dyn UserInterface) -> Result<()> {
    ui.info("Phase 8/8: Installing system (this will take a while)...");

    let exit =
        exec::run_cmd_interactive(&["guix", "system", "init", "/mnt/etc/system.scm", "/mnt"])?;
    if exit != 0 {
        anyhow::bail!("guix system init failed with exit code {exit}");
    }

    // Set user password by writing directly to /mnt/etc/shadow.
    // Plaintext never leaves this process (no chroot/chpasswd subprocess);
    // Zeroizing wipes the clone on drop.
    if let Some(password) = &config.password {
        ui.info("Setting user password...");
        let root = Path::new("/mnt");
        for user in &config.users {
            let copy = Zeroizing::new(password.clone());
            passwd::set_shadow_password(root, &user.name, copy)?;
        }
    }

    ui.info("Installation complete! You can now reboot.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonguix_key_is_valid() {
        assert!(NONGUIX_KEY.contains("public-key"));
        assert!(NONGUIX_KEY.contains("Ed25519"));
    }

    #[test]
    fn pantherx_key_is_valid() {
        assert!(PANTHERX_KEY.contains("public-key"));
        assert!(PANTHERX_KEY.contains("Ed25519"));
    }

    #[test]
    fn channels_available_guix_always_true() {
        let mut config = SystemConfig::default();
        config.mode = InstallMode::Guix;
        assert!(channels_available(&config));
    }
}
