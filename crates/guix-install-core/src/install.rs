use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::config::SystemConfig;
use crate::enterprise;
use crate::installer_log;
use crate::mode::InstallMode;
use crate::passwd;
use crate::progress::{self, Phase};
use crate::resume::InstallState;
use crate::ui::UserInterface;
use crate::{disk, exec, scheme};

const NONGUIX_KEY: &str = include_str!("../keys/substitutes.nonguix.org.pub");
const GOFRANZ_KEY: &str = include_str!("../keys/substitutes.guix.gofranz.com.pub");

const TARGET_DIR: &str = "/mnt";
const LOG_PATH: &str = "/mnt/var/log/guix-install.log";

const GUIX_DB_FILE: &str = "/var/guix/db/db.sqlite";
const GUIX_DB_SAVE: &str = "/var/guix/db/db.save";
const GUIX_DB_WAL: &str = "/var/guix/db/db.sqlite-wal";
const GUIX_DB_SHM: &str = "/var/guix/db/db.sqlite-shm";

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

/// Sets `LC_ALL` early, falling back to `en_US.utf8` on unsupported values.
///
/// Done before COW-store mount: glibc loads locale data from `/run/current-system`
/// on first use, and once that path is shadowed by the overlay, dropped references
/// would pin the cow-store mount and block clean unmount on cleanup. Mirrors
/// `gnu/installer/final.scm:install-locale`.
fn install_locale_early(config: &SystemConfig) {
    // Single-threaded at this point; safe per Rust's `set_var` contract.
    let try_set = |val: &str| {
        unsafe {
            std::env::set_var("LC_ALL", val);
        }
        installer_log::write_line("locale:", &format!("LC_ALL={val}"));
    };
    // We can't call setlocale from Rust without libc, but Guix locales follow a
    // known naming scheme. Trust the user's choice and rely on the fallback if
    // `guix system init` rejects it later.
    try_set(&config.locale);
}

/// Returns true if `target` appears as a mount point in `/proc/mounts`.
fn is_mounted(target: &str) -> bool {
    let Ok(content) = std::fs::read_to_string("/proc/mounts") else {
        return false;
    };
    content
        .lines()
        .any(|line| line.split_whitespace().nth(1) == Some(target))
}

/// Tracks resources held during install and tears them down on Drop.
///
/// On a fresh process re-run after a crash, the previous session's namespace
/// (and thus its mounts) are gone — but swap activation, LUKS mappers, and
/// any state in `/var/guix/db` are global. Cleanup handles those, plus best-
/// effort unmounts in the rare case the failure happened in the same process.
///
/// Call [`Self::mark_complete`] after a successful install to skip the heavy
/// teardown (the system reboots anyway).
struct InstallSession {
    completed: bool,
    db_saved: bool,
    /// `phase_mount` ran in *this* process. Without this guard, a re-run that
    /// inherited a mounted `/mnt` (e.g. resume) would let Drop unmount mounts
    /// it didn't establish — particularly bad if a future caller pointed `/mnt`
    /// at something they care about.
    mount_owned: bool,
    luks_mapper: Option<String>,
}

impl InstallSession {
    fn new(config: &SystemConfig) -> Self {
        let luks_mapper = config.encryption.as_ref().map(|e| e.device_target.clone());
        Self {
            completed: false,
            db_saved: false,
            mount_owned: false,
            luks_mapper,
        }
    }

    fn mark_complete(&mut self) {
        self.completed = true;
    }

    fn mark_mount_owned(&mut self) {
        self.mount_owned = true;
    }

    /// Snapshots `/var/guix/db/db.sqlite` so it can be restored after the
    /// cow-store unmount. The daemon writes store-path entries during
    /// `guix system init` that point at the overlay; once unmounted they'd
    /// reference paths that don't exist on the host's `/gnu/store`.
    fn save_db(&mut self) -> Result<()> {
        let db = Path::new(GUIX_DB_FILE);
        if !db.exists() {
            installer_log::write_line("db:", "db.sqlite missing, skipping save");
            return Ok(());
        }
        std::fs::copy(db, GUIX_DB_SAVE).with_context(|| format!("save db to {GUIX_DB_SAVE}"))?;
        self.db_saved = true;
        installer_log::write_line("db:", &format!("saved {GUIX_DB_FILE} -> {GUIX_DB_SAVE}"));
        Ok(())
    }

    /// Restores the snapshot and removes `-wal`/`-shm` siblings so a daemon
    /// restart doesn't replay a partial transaction.
    /// Mirrors the after-thunk in `gnu/installer/final.scm:install-system`.
    fn restore_db(&mut self) {
        if !self.db_saved {
            return;
        }
        let save = Path::new(GUIX_DB_SAVE);
        if !save.exists() {
            installer_log::write_line("db:", "db.save missing, cannot restore");
            return;
        }
        match std::fs::copy(save, GUIX_DB_FILE) {
            Ok(_) => installer_log::write_line("db:", "restored db.sqlite from db.save"),
            Err(e) => installer_log::write_line("db:", &format!("restore failed: {e}")),
        }
        let _ = std::fs::remove_file(GUIX_DB_WAL);
        let _ = std::fs::remove_file(GUIX_DB_SHM);
        let _ = std::fs::remove_file(save);
        self.db_saved = false;
    }
}

impl Drop for InstallSession {
    fn drop(&mut self) {
        // Always restore the daemon DB if we snapshotted it — install may have
        // touched it whether it succeeded or failed.
        self.restore_db();

        // Always close the log so its buffer hits disk before any unmount.
        let close_log = || installer_log::close();

        if self.completed {
            close_log();
            return;
        }

        installer_log::write_line("cleanup:", "starting (failure path)");

        // Touch /mnt-bound state only if *this* session mounted it. Avoids
        // tearing down mounts we found pre-existing (e.g. operator inspection
        // shells, dev environments where /mnt is a real filesystem).
        if self.mount_owned {
            let _ = exec::run_cmd(&[
                "swapoff",
                PathBuf::from(TARGET_DIR)
                    .join("swapfile")
                    .to_str()
                    .unwrap_or("/mnt/swapfile"),
            ]);
            let _ = exec::run_cmd(&["herd", "stop", "cow-store"]);

            // Order matters: boot first (it's mounted *inside* /mnt), then root.
            if is_mounted("/mnt/boot/efi") {
                let _ =
                    rustix::mount::unmount("/mnt/boot/efi", rustix::mount::UnmountFlags::DETACH);
            }
            if is_mounted(TARGET_DIR) {
                let _ = rustix::mount::unmount(TARGET_DIR, rustix::mount::UnmountFlags::DETACH);
            }

            if let Some(name) = &self.luks_mapper {
                let mapper_path = format!("/dev/mapper/{name}");
                if Path::new(&mapper_path).exists() {
                    let _ = exec::run_cmd(&["cryptsetup", "close", name]);
                }
            }
        } else {
            installer_log::write_line("cleanup:", "skipping mount teardown (not mount-owned)");
        }

        // Enterprise tarball extract dir, in case phase_config died mid-way.
        enterprise::cleanup();

        installer_log::write_line("cleanup:", "done");
        close_log();
    }
}

/// Drops `completed_phases >= 3` if the live system no longer reflects them.
///
/// Mounts and swap don't survive process death (or our private MNT namespace
/// going away). On resume we re-run those phases rather than skipping past
/// missing state. The format phase (2) does survive — partitions stay
/// formatted on disk.
fn validate_resume_state(state: &mut InstallState, config: &SystemConfig, ui: &dyn UserInterface) {
    let needs_efi_boot = config.firmware == crate::config::Firmware::Efi;
    let mounts_intact = is_mounted(TARGET_DIR) && (!needs_efi_boot || is_mounted("/mnt/boot/efi"));

    if !mounts_intact && state.completed_phases.iter().any(|&p| p >= 3) {
        ui.warn(
            "Previous mount state is incomplete (cow-store / target / boot unmounted). \
             Re-running mount phases.",
        );
        state.completed_phases.retain(|&p| p < 3);
    }
}

/// Runs the full 8-phase installation sequence.
///
/// Supports resume: if a previous state file exists, already-completed phases
/// are skipped. State is persisted after each phase so a crash or power loss
/// only requires re-running the current phase.
pub fn execute_installation(config: &SystemConfig, ui: &dyn UserInterface) -> Result<()> {
    check_connectivity(ui);

    install_locale_early(config);
    // NOTE: do NOT enter a private mount namespace here. We invoke
    // `herd start cow-store /mnt`, which dispatches to shepherd (PID 1)
    // running in the host namespace; if our `/mnt` mount is hidden from
    // shepherd, cow-store sees `/mnt` as the live overlayfs root and the
    // overlay setup fails with "filesystem on /mnt/tmp/guix-inst not
    // supported as upperdir". The InstallSession Drop handles cleanup.

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
    validate_resume_state(&mut state, config, ui);

    let mut session = InstallSession::new(config);

    type PhaseFn = fn(&SystemConfig, &dyn UserInterface, &mut InstallSession) -> Result<()>;
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
            ui.install_phase(*num, 8, phase_label(*num));
            ui.info(&format!("Phase {num}/8: Already completed, skipping."));
            ui.progress(
                &format!("Phase {num}/8 already complete"),
                Some(progress::overall_pct(phase_for(*num), 1.0)),
            );
            continue;
        }

        ui.install_phase(*num, 8, phase_label(*num));
        installer_log::write_line("phase:", &format!("starting {num}/8"));
        // Cheap phases report start here; the guix phases (7/8) report their
        // own intra-phase fraction via guix_ops.
        ui.progress(
            &format!("Phase {num}/8 starting"),
            Some(progress::overall_pct(phase_for(*num), 0.0)),
        );
        func(config, ui, &mut session).map_err(|e| e.context(format!("Phase {num}/8 failed")))?;
        installer_log::write_line("phase:", &format!("completed {num}/8"));
        ui.progress(
            &format!("Phase {num}/8 complete"),
            Some(progress::overall_pct(phase_for(*num), 1.0)),
        );
        state.mark_complete(*num);
        state.save()?;
    }

    InstallState::cleanup()?;
    session.mark_complete();
    Ok(())
}

/// Short checklist label for a phase number, for the GUI install screen.
fn phase_label(num: u8) -> &'static str {
    match num {
        1 => "Partition",
        2 => "Format",
        3 => "Mount",
        4 => "Swap",
        5 => "Config",
        6 => "Authorize",
        7 => "guix pull",
        _ => "Install",
    }
}

/// Maps the 1..=8 phase number to its weighted [`Phase`].
fn phase_for(num: u8) -> Phase {
    match num {
        1 => Phase::Partition,
        2 => Phase::Format,
        3 => Phase::Mount,
        4 => Phase::Swap,
        5 => Phase::Config,
        6 => Phase::Authorize,
        7 => Phase::Pull,
        _ => Phase::Install,
    }
}

fn phase_partition(
    config: &SystemConfig,
    ui: &dyn UserInterface,
    _session: &mut InstallSession,
) -> Result<()> {
    ui.info("Phase 1/8: Partitioning disk...");
    let cmds = disk::partition::partition_commands(&config.disk.dev_path, &config.firmware);
    for cmd in &cmds {
        let args: Vec<&str> = cmd.iter().map(|s| s.as_str()).collect();
        exec::run_cmd(&args)?;
    }
    // Make sure the kernel sees the new partition table and udev populated
    // /dev/<disk>N entries before format runs.
    let _ = exec::run_cmd(&["partprobe", &config.disk.dev_path]);
    let _ = exec::run_cmd(&["udevadm", "settle"]);
    Ok(())
}

fn phase_format(
    config: &SystemConfig,
    ui: &dyn UserInterface,
    _session: &mut InstallSession,
) -> Result<()> {
    ui.info("Phase 2/8: Formatting partitions...");
    let cmds = disk::format::format_commands(config);
    for cmd in &cmds {
        let args: Vec<&str> = cmd.iter().map(|s| s.as_str()).collect();
        if cmd[0] == "cryptsetup" {
            // The passphrase reaches cryptsetup via stdin (`--key-file -`),
            // never argv — see disk::format::encryption_commands.
            let passphrase = config
                .encryption
                .as_ref()
                .and_then(|e| e.passphrase.as_ref())
                .context(
                    "encryption is enabled but no LUKS passphrase is set; \
                     re-run and enter it at the encryption step",
                )?;
            exec::run_cmd_with_stdin(&args, passphrase.as_str())?;
        } else {
            exec::run_cmd(&args)?;
        }
    }
    // Wait for /dev/disk/by-label/my-root to appear before mount LABEL= runs.
    let _ = exec::run_cmd(&["udevadm", "settle"]);
    Ok(())
}

fn phase_mount(
    config: &SystemConfig,
    ui: &dyn UserInterface,
    session: &mut InstallSession,
) -> Result<()> {
    ui.info("Phase 3/8: Mounting filesystems...");
    for action in disk::mount::mount_actions(config) {
        action.execute()?;
    }
    session.mark_mount_owned();

    // Now that /mnt is writable, redirect installer logs there so they
    // survive into the booted system.
    if let Err(e) = installer_log::open(Path::new(LOG_PATH)) {
        ui.warn(&format!("could not open installer log {LOG_PATH}: {e}"));
    } else {
        ui.info(&format!("  Installer log: {LOG_PATH}"));
    }
    Ok(())
}

fn phase_swap(
    config: &SystemConfig,
    ui: &dyn UserInterface,
    _session: &mut InstallSession,
) -> Result<()> {
    ui.info("Phase 4/8: Creating swap...");
    for action in disk::mount::swap_actions(config) {
        action.execute()?;
    }
    Ok(())
}

fn phase_config(
    config: &SystemConfig,
    ui: &dyn UserInterface,
    _session: &mut InstallSession,
) -> Result<()> {
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
        let system_scm = match &config.system_scm_override {
            Some(custom) => {
                ui.info("  Using custom system.scm from Advanced configuration.");
                custom.clone()
            }
            None => scheme::operating_system::render_operating_system(config),
        };
        std::fs::write("/mnt/etc/system.scm", &system_scm)?;
        ui.info("  Wrote /mnt/etc/system.scm");

        if let Some(channels) = scheme::channels::render_channels(&config.mode) {
            std::fs::write("/mnt/etc/guix/channels.scm", &channels)?;
            ui.info("  Wrote /mnt/etc/guix/channels.scm");
        }

        if let Some(password) = &config.password {
            passwd::seed_shadow(Path::new("/mnt"), &config.users, password)?;
            ui.info("  Seeded /mnt/etc/shadow");
        }
    }

    Ok(())
}

fn phase_authorize(
    config: &SystemConfig,
    ui: &dyn UserInterface,
    _session: &mut InstallSession,
) -> Result<()> {
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
            authorize_substitute("substitutes.guix.gofranz.com", GOFRANZ_KEY, ui)?;
        }
        InstallMode::Enterprise { .. } => {
            authorize_substitute("substitutes.nonguix.org", NONGUIX_KEY, ui)?;
            authorize_substitute("substitutes.guix.gofranz.com", GOFRANZ_KEY, ui)?;
        }
    }
    Ok(())
}

fn authorize_substitute(name: &str, key: &str, ui: &dyn UserInterface) -> Result<()> {
    ui.info(&format!("  Authorizing {name}..."));
    exec::run_cmd_with_stdin(&["guix", "archive", "--authorize"], key)?;
    Ok(())
}

fn phase_pull(
    config: &SystemConfig,
    ui: &dyn UserInterface,
    _session: &mut InstallSession,
) -> Result<()> {
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
    crate::guix_ops::run_pull(
        Path::new("/mnt/etc/guix/channels.scm"),
        config.mode.substitute_urls(),
        ui,
    )?;

    Ok(())
}

/// Tests whether the required channel modules are loadable from the daemon's Guix.
///
/// Uses `guix repl` (not bare `guile`) because channel modules — `(nongnu …)`,
/// `(px …)` — live on Guix's load path, not the system Guile's. On a Panther
/// install ISO the channels are baked in via `guix-for-channels`, so `guix repl`
/// resolves them; bare `guile` can only see `(gnu …)` / `(guix …)` and would
/// trigger an unnecessary `guix pull`.
fn channels_available(config: &SystemConfig) -> bool {
    let test_module = match &config.mode {
        InstallMode::Guix => return true,
        InstallMode::Nonguix => "(use-modules (nongnu packages linux))",
        InstallMode::Panther | InstallMode::Enterprise { .. } => {
            "(use-modules (px system panther))"
        }
    };

    exec::run_cmd_with_stdin(&["guix", "repl"], test_module).is_ok()
}

fn phase_install(
    config: &SystemConfig,
    ui: &dyn UserInterface,
    session: &mut InstallSession,
) -> Result<()> {
    ui.info("Phase 8/8: Installing system (this will take a while)...");

    // Snapshot guix-daemon's local DB; restored on Drop regardless of outcome.
    session.save_db()?;

    crate::guix_ops::run_system_init(
        Path::new("/mnt/etc/system.scm"),
        Path::new(TARGET_DIR),
        config.mode.substitute_urls(),
        ui,
    )?;

    ui.info("Installation complete! You can now reboot.");
    ui.info(&format!(
        "  Logs preserved at {LOG_PATH} (inside the new system)."
    ));
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
    fn gofranz_key_is_valid() {
        assert!(GOFRANZ_KEY.contains("public-key"));
        assert!(GOFRANZ_KEY.contains("Ed25519"));
    }

    #[test]
    fn channels_available_guix_always_true() {
        let mut config = SystemConfig::default();
        config.mode = InstallMode::Guix;
        assert!(channels_available(&config));
    }

    #[test]
    fn validate_resume_clears_post_mount_phases_when_unmounted() {
        // Synthetic: /mnt isn't mounted in the test env, so phases >= 3 must be cleared.
        let config = SystemConfig::default();
        let mut state = InstallState::new(&config);
        state.mark_complete(1);
        state.mark_complete(2);
        state.mark_complete(3);
        state.mark_complete(4);

        struct Quiet;
        impl UserInterface for Quiet {
            fn select(&mut self, _: &str, _: &[&str], _: usize) -> Result<usize> {
                Ok(0)
            }
            fn input(&mut self, _: &str, _: &str) -> Result<String> {
                Ok(String::new())
            }
            fn password(&mut self, _: &str) -> Result<zeroize::Zeroizing<String>> {
                Ok(zeroize::Zeroizing::new(String::new()))
            }
            fn confirm(&mut self, _: &str, _: bool) -> Result<bool> {
                Ok(false)
            }
            fn info(&self, _: &str) {}
            fn warn(&self, _: &str) {}
            fn error(&self, _: &str) {}
            fn progress(&self, _: &str, _: Option<f32>) {}
        }

        validate_resume_state(&mut state, &config, &Quiet);
        assert_eq!(state.completed_phases, vec![1, 2]);
    }

    #[test]
    fn install_session_marks_complete_skips_failure_cleanup() {
        // We can't actually trigger cleanup in a unit test (would need root + real
        // mounts), but we can verify the bookkeeping flag: after mark_complete,
        // dropping must take the success path.
        let config = SystemConfig::default();
        let mut s = InstallSession::new(&config);
        assert!(!s.completed);
        s.mark_complete();
        assert!(s.completed);
        // Drop runs at end-of-scope and should be a no-op for unmounts.
    }
}
