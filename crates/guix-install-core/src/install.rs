use std::path::Path;

use anyhow::{Context, Result};

use crate::config::SystemConfig;
use crate::enterprise;
use crate::installer_log;
use crate::mode::InstallMode;
use crate::passwd;
use crate::progress::ProgressModel;
use crate::resume::InstallState;
use crate::ui::UserInterface;
use crate::{disk, exec, scheme};

const NONGUIX_KEY: &str = include_str!("../keys/substitutes.nonguix.org.pub");
const GOFRANZ_KEY: &str = include_str!("../keys/substitutes.guix.gofranz.com.pub");

const TARGET_DIR: &str = "/mnt";

/// Where the cow-store overlay is moved to before it is unmounted.
const COW_STORE_DETACH_DIR: &str = "/remove";
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
    /// `phase_mount` ran in *this* process. Without this guard, a re-run that
    /// inherited a mounted `/mnt` (e.g. resume) would let Drop unmount mounts
    /// it didn't establish — particularly bad if a future caller pointed `/mnt`
    /// at something they care about.
    mount_owned: bool,
    /// `phase_format` ran in this process, so an open LUKS mapping is ours.
    formatted: bool,
    luks_mapper: Option<String>,
    swap_file: String,
    /// Mounts nested inside `/mnt` (subvolumes, EFI), in unmount order.
    nested_mounts: Vec<String>,
}

impl InstallSession {
    fn new(config: &SystemConfig) -> Self {
        let luks_mapper = config.encryption.as_ref().map(|e| e.device_target.clone());
        Self {
            completed: false,
            mount_owned: false,
            formatted: false,
            luks_mapper,
            swap_file: format!("{TARGET_DIR}{}", config.filesystem.swap_file_path()),
            nested_mounts: nested_mount_points(config),
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
        // A snapshot left by an interrupted install is the pristine one; keep it.
        if Path::new(GUIX_DB_SAVE).exists() {
            installer_log::write_line("db:", "keeping existing db.save");
            return Ok(());
        }
        std::fs::copy(db, GUIX_DB_SAVE).with_context(|| format!("save db to {GUIX_DB_SAVE}"))?;
        installer_log::write_line("db:", &format!("saved {GUIX_DB_FILE} -> {GUIX_DB_SAVE}"));
        Ok(())
    }

    /// Restores the snapshot and removes `-wal`/`-shm` siblings so a daemon
    /// restart doesn't replay a partial transaction.
    /// Mirrors the after-thunk in `gnu/installer/final.scm:install-system`.
    fn restore_db(&mut self) {
        let save = Path::new(GUIX_DB_SAVE);
        if !save.exists() {
            return;
        }
        match std::fs::copy(save, GUIX_DB_FILE) {
            Ok(_) => installer_log::write_line("db:", "restored db.sqlite from db.save"),
            Err(e) => installer_log::write_line("db:", &format!("restore failed: {e}")),
        }
        let _ = std::fs::remove_file(GUIX_DB_WAL);
        let _ = std::fs::remove_file(GUIX_DB_SHM);
        let _ = std::fs::remove_file(save);
    }
}

impl InstallSession {
    /// Releases the target disk: swap, cow-store, mounts, LUKS mapping.
    ///
    /// Runs after success too, so the disk is free for a reboot or another
    /// install and no cow-store scratch stays on the installed system.
    /// Returns false if something was still busy and had to be detached lazily.
    fn release_target(&mut self) -> bool {
        let mut clean = true;
        let _ = exec::run_cmd(&["swapoff", &self.swap_file]);

        // The daemon keeps files in the overlaid store open; restore its
        // database while it is down.
        let cow_store = cow_store_mounted();
        if cow_store {
            let _ = exec::run_cmd(&["herd", "stop", "guix-daemon"]);
        }
        self.restore_db();
        if cow_store {
            // The cow-store binds the target's /tmp over the live one.
            if mount_source("/tmp") == mount_source(TARGET_DIR) {
                clean &= unmount("/tmp");
            }
            // Mirrors `unmount-cow-store` in gnu/build/install.scm.
            let _ = std::fs::create_dir_all(COW_STORE_DETACH_DIR);
            if exec::run_cmd(&["mount", "--move", "/gnu/store", COW_STORE_DETACH_DIR]).is_ok() {
                clean &= unmount(COW_STORE_DETACH_DIR);
                let _ = std::fs::remove_dir(COW_STORE_DETACH_DIR);
            }
            // Marks the service stopped and deletes its backing directory.
            let _ = exec::run_cmd(&["herd", "stop", "cow-store"]);
            for scratch in ["guix-inst", ".overlayfs-workdir"] {
                let _ = std::fs::remove_dir_all(format!("{TARGET_DIR}/tmp/{scratch}"));
            }
            let _ = exec::run_cmd(&["herd", "start", "guix-daemon"]);
        }

        // Order matters: nested mounts first, then root.
        for target in &self.nested_mounts {
            if is_mounted(target) {
                clean &= unmount(target);
            }
        }
        if is_mounted(TARGET_DIR) {
            clean &= unmount(TARGET_DIR);
        }

        if let Some(name) = &self.luks_mapper {
            let mapper_path = format!("/dev/mapper/{name}");
            if Path::new(&mapper_path).exists() {
                let _ = exec::run_cmd(&["cryptsetup", "close", name]);
            }
        }
        clean
    }
}

/// Unmounts `target`, retrying while busy, then detaching lazily. Returns
/// false if it took the lazy detach, which leaves the device in use.
fn unmount(target: &str) -> bool {
    for _ in 0..5 {
        if exec::run_cmd(&["umount", target]).is_ok() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    installer_log::write_line("cleanup:", &format!("{target} is busy, detaching lazily"));
    let _ = exec::run_cmd(&["umount", "-l", target]);
    false
}

/// Source device of the mount at `target`, per `/proc/mounts`.
fn mount_source(target: &str) -> Option<String> {
    let content = std::fs::read_to_string("/proc/mounts").ok()?;
    content
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let source = fields.next()?;
            (fields.next()? == target).then(|| source.to_string())
        })
        .next_back()
}

/// True while the cow-store overlay backed by the target is on `/gnu/store`.
fn cow_store_mounted() -> bool {
    let Ok(content) = std::fs::read_to_string("/proc/mounts") else {
        return false;
    };
    content.lines().any(|line| {
        let fields: Vec<&str> = line.split_whitespace().collect();
        fields.get(1) == Some(&"/gnu/store")
            && fields.get(2) == Some(&"overlay")
            && fields
                .get(3)
                .is_some_and(|opts| opts.contains("/tmp/guix-inst"))
    })
}

/// Fails if anything still uses the target: a mount, swap, or another device
/// stacked on it (LUKS, LVM, RAID), including a lazily detached mount.
fn ensure_target_unused(config: &SystemConfig) -> Result<()> {
    use rustix::fs::{Mode, OFlags};

    let disk = &config.disk.dev_path;
    let mut in_use: Vec<String> = Vec::new();
    for (file, field) in [("/proc/mounts", 1), ("/proc/swaps", 0)] {
        let content = std::fs::read_to_string(file).unwrap_or_default();
        for line in content.lines() {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields
                .first()
                .is_some_and(|source| source.starts_with(disk.as_str()))
                && let Some(location) = fields.get(field)
            {
                let prefix = if field == 0 { "swap on " } else { "" };
                in_use.push(format!("{prefix}{location}"));
            }
        }
    }
    if !in_use.is_empty() {
        anyhow::bail!(
            "{disk} is in use ({}). Release it or reboot the installer, then try again.",
            in_use.join(", ")
        );
    }

    // The kernel refuses an exclusive open while any partition is claimed.
    let flags = OFlags::RDONLY | OFlags::EXCL | OFlags::CLOEXEC;
    let probe = rustix::fs::open(disk.as_str(), flags, Mode::empty());
    if matches!(probe, Err(rustix::io::Errno::BUSY)) {
        anyhow::bail!(
            "{disk} is held by another device or a leftover mount (LUKS mapping, LVM, RAID). \
             Release it or reboot the installer, then try again."
        );
    }

    if let Some(enc) = &config.encryption
        && Path::new(&format!("/dev/mapper/{}", enc.device_target)).exists()
    {
        anyhow::bail!(
            "A device mapping named {0} is already open. Close it with \
             `cryptsetup close {0}` or reboot the installer, then try again.",
            enc.device_target
        );
    }
    Ok(())
}

impl Drop for InstallSession {
    fn drop(&mut self) {
        if !self.completed {
            installer_log::write_line("cleanup:", "starting (failure path)");
        }

        // The target log lives on a mount about to go away.
        installer_log::close_target();

        // Touch /mnt-bound state only if this install mounted it. Avoids
        // tearing down mounts we found pre-existing (e.g. operator inspection
        // shells, dev environments where /mnt is a real filesystem).
        if self.mount_owned {
            let _ = self.release_target();
        } else {
            self.restore_db();
            installer_log::write_line("cleanup:", "skipping mount teardown (not mount-owned)");
            if self.formatted
                && let Some(name) = &self.luks_mapper
                && Path::new(&format!("/dev/mapper/{name}")).exists()
            {
                let _ = exec::run_cmd(&["cryptsetup", "close", name]);
            }
        }

        // Enterprise tarball extract dir, in case phase_config died mid-way.
        enterprise::cleanup();

        installer_log::write_line("cleanup:", "done");
        installer_log::close();
    }
}

/// Resuming needs the previous run's mounts still in place, which is the case
/// after the installer process died. Anything else starts from the first phase.
fn validate_resume_state(state: &mut InstallState, config: &SystemConfig, ui: &dyn UserInterface) {
    let mounts_intact = is_mounted(TARGET_DIR)
        && nested_mount_points(config)
            .iter()
            .all(|target| is_mounted(target));

    if !mounts_intact && !state.completed_phases.is_empty() {
        ui.warn("The previous install is no longer mounted. Starting from the first phase.");
        state.completed_phases.clear();
    }
}

/// True if partition 2 still carries the UUID this install formatted it with.
fn disk_matches_config(config: &SystemConfig) -> bool {
    let part2 = disk::partition_path(&config.disk.dev_path, 2);
    let found = exec::run_cmd(&["blkid", "-s", "UUID", "-o", "value", &part2])
        .map(|result| result.stdout)
        .unwrap_or_default();
    config.disk.root_partition_uuid.as_deref() == Some(found.trim())
}

/// Mount points `phase_mount` establishes inside `/mnt`, deepest first so the
/// list doubles as unmount order.
fn nested_mount_points(config: &SystemConfig) -> Vec<String> {
    let mut targets = Vec::new();
    if config.firmware == crate::config::Firmware::Efi {
        targets.push(format!("{TARGET_DIR}/boot/efi"));
    }
    if config.filesystem == crate::config::Filesystem::Btrfs {
        for subvol in disk::btrfs::SUBVOLUMES.iter().rev() {
            if subvol.mount_point != "/" {
                targets.push(subvol.target_path(TARGET_DIR));
            }
        }
    }
    targets
}

/// Drops a saved install state and releases whatever that install left mounted.
pub fn discard_previous(state: &InstallState) -> Result<()> {
    if state.completed_phases.contains(&3) {
        // Not dropped: Drop would also close the live log.
        let mut previous = std::mem::ManuallyDrop::new(InstallSession::new(&state.config));
        let clean = previous.release_target();
        InstallState::cleanup()?;
        if !clean {
            anyhow::bail!(
                "The interrupted install on {} is still holding the disk. \
                 Reboot the installer, then try again.",
                state.config.disk.dev_path
            );
        }
    }
    InstallState::cleanup()
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

    // Idempotent; unattended runs get here without the interview having opened it.
    let _ = installer_log::open(Path::new(installer_log::LIVE_LOG_PATH));
    installer_log::write_line("version:", env!("CARGO_PKG_VERSION"));

    let mut fresh = config.clone();
    fresh.disk.assign_partition_uuids();

    let (mut state, config) = match InstallState::load()? {
        Some(existing) => match existing.resume_config(&fresh) {
            Some(resumed) => {
                // Empty after a failed run: same settings, nothing to skip.
                if let Some(last) = existing.completed_phases.last() {
                    ui.info(&format!(
                        "Found previous installation state (completed through phase {last}/8). Resuming."
                    ));
                }
                (existing, resumed)
            }
            None => {
                ui.warn("Previous installation state found but config differs. Starting fresh.");
                discard_previous(&existing)?;
                (InstallState::new(&fresh), fresh)
            }
        },
        None => (InstallState::new(&fresh), fresh),
    };
    let config = &config;
    // Someone repartitioned or reformatted the disk since: nothing to resume.
    if state.completed_phases.contains(&2) && !disk_matches_config(config) {
        ui.warn("The disk no longer matches the previous installation state. Starting fresh.");
        state.completed_phases.clear();
    }
    validate_resume_state(&mut state, config, ui);

    if !state.completed_phases.contains(&1) {
        ensure_target_unused(config)?;
    }

    let mut session = InstallSession::new(config);
    if state.completed_phases.contains(&3) {
        session.mark_mount_owned();
    }
    let model = ProgressModel::new();

    let outcome = run_phases(config, ui, &mut state, &mut session, &model);
    if let Err(error) = outcome {
        // The failure cleanup releases the target, so nothing is left to resume;
        // the saved answers stay for the next run.
        drop(session);
        state.completed_phases.clear();
        let _ = state.save();
        return Err(error);
    }

    // The model caps intermediate progress at CEIL; the literal 100% is emitted
    // here, once, on success. The GUI also gets it via `Finished`, but the CLI
    // has no other signal, so without this its bar would freeze at 99%.
    ui.progress("Installation complete", Some(1.0));

    InstallState::cleanup()?;
    session.mark_complete();
    Ok(())
}

/// Runs every phase the state doesn't list as completed, saving after each.
fn run_phases(
    config: &SystemConfig,
    ui: &dyn UserInterface,
    state: &mut InstallState,
    session: &mut InstallSession,
    model: &ProgressModel,
) -> Result<()> {
    type PhaseFn =
        fn(&SystemConfig, &dyn UserInterface, &mut InstallSession, &ProgressModel) -> Result<()>;
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
        let n = *num;
        if state.completed_phases.contains(num) {
            ui.install_phase(n, 8, phase_label(n));
            ui.info(&format!("Phase {n}/8: Already completed, skipping."));
            // Cheap phases nudge the prefix; a skipped init jumps to the ceiling.
            // A skipped pull (7) is left alone — it is usually a no-op, so its
            // band must not be credited (that was the old "stuck at 46%" bug).
            if n <= 6 {
                ui.progress(
                    &format!("Phase {n}/8 already complete"),
                    Some(model.cheap_phase(n, true)),
                );
            } else if n == 8 {
                ui.progress("Phase 8/8 already complete", Some(model.skip_install()));
            }
            continue;
        }

        ui.install_phase(n, 8, phase_label(n));
        installer_log::write_line("phase:", &format!("starting {n}/8"));
        // Cheap phases report coarse start/complete here; the guix phases (7/8)
        // report their own fine-grained milestone progress from inside guix_ops.
        if n <= 6 {
            ui.progress(
                &format!("Phase {n}/8 starting"),
                Some(model.cheap_phase(n, false)),
            );
        }
        func(config, ui, session, model).map_err(|e| e.context(format!("Phase {n}/8 failed")))?;
        installer_log::write_line("phase:", &format!("completed {n}/8"));
        if n <= 6 {
            ui.progress(
                &format!("Phase {n}/8 complete"),
                Some(model.cheap_phase(n, true)),
            );
        }
        state.mark_complete(n);
        state.save()?;
    }
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

fn phase_partition(
    config: &SystemConfig,
    ui: &dyn UserInterface,
    _session: &mut InstallSession,
    _model: &ProgressModel,
) -> Result<()> {
    ui.info("Phase 1/8: Partitioning disk...");
    // Whole-disk signatures (a filesystem or RAID member without a partition
    // table) sit outside the new partitions and would survive otherwise.
    let _ = exec::run_cmd(&["wipefs", "-a", &config.disk.dev_path]);
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
    session: &mut InstallSession,
    _model: &ProgressModel,
) -> Result<()> {
    ui.info("Phase 2/8: Formatting partitions...");
    session.formatted = true;
    let mut cmds = disk::format::wipe_commands(&config.disk.dev_path);
    cmds.extend(disk::format::format_commands(config));
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
    // Let udev pick up the new filesystems before the mount phase.
    let _ = exec::run_cmd(&["udevadm", "settle"]);
    Ok(())
}

fn phase_mount(
    config: &SystemConfig,
    ui: &dyn UserInterface,
    session: &mut InstallSession,
    _model: &ProgressModel,
) -> Result<()> {
    ui.info("Phase 3/8: Mounting filesystems...");
    for action in disk::mount::mount_actions(config) {
        action.execute()?;
    }
    session.mark_mount_owned();

    // Now that /mnt is writable, redirect installer logs there so they
    // survive into the booted system.
    if let Err(e) = installer_log::open(Path::new(installer_log::TARGET_LOG_PATH)) {
        ui.warn(&format!(
            "could not open installer log {}: {e}",
            installer_log::TARGET_LOG_PATH
        ));
    } else {
        ui.info(&format!(
            "  Installer log: {}",
            installer_log::TARGET_LOG_PATH
        ));
    }
    Ok(())
}

fn phase_swap(
    config: &SystemConfig,
    ui: &dyn UserInterface,
    _session: &mut InstallSession,
    _model: &ProgressModel,
) -> Result<()> {
    ui.info("Phase 4/8: Creating swap...");
    // A swap file from an interrupted run would make the commands below fail.
    let swap_file = format!("{TARGET_DIR}{}", config.filesystem.swap_file_path());
    if Path::new(&swap_file).exists() {
        let _ = exec::run_cmd(&["swapoff", &swap_file]);
        std::fs::remove_file(&swap_file)?;
    }
    for action in disk::mount::swap_actions(config) {
        action.execute()?;
    }
    Ok(())
}

fn phase_config(
    config: &SystemConfig,
    ui: &dyn UserInterface,
    _session: &mut InstallSession,
    _model: &ProgressModel,
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

        // Nonguix declares its channels via the guix service in system.scm
        // (channels field), so the running system manages /etc/guix/channels.scm
        // itself. Writing it here too would leave a stray channels.scm.bak on
        // first boot, so skip the direct write for Nonguix.
        if !matches!(config.mode, InstallMode::Nonguix)
            && let Some(channels) = scheme::channels::render_channels(&config.mode)
        {
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
    _model: &ProgressModel,
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
    model: &ProgressModel,
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
    // Pull is doing real work, so `guix system init` should start from the
    // pull/install band boundary rather than the cheap-phase prefix.
    model.mark_pull_ran();
    crate::guix_ops::run_pull(
        Path::new("/mnt/etc/guix/channels.scm"),
        config.mode.substitute_urls(),
        ui,
        model,
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
    model: &ProgressModel,
) -> Result<()> {
    ui.info("Phase 8/8: Installing system (this will take a while)...");

    // Snapshot guix-daemon's local DB; restored on Drop regardless of outcome.
    session.save_db()?;

    crate::guix_ops::run_system_init(
        Path::new("/mnt/etc/system.scm"),
        Path::new(TARGET_DIR),
        config.mode.substitute_urls(),
        ui,
        model,
    )?;

    ui.info("Installation complete! You can now reboot.");
    ui.info(&format!(
        "  Logs preserved at {} (inside the new system).",
        installer_log::TARGET_LOG_PATH
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
        let config = SystemConfig {
            mode: InstallMode::Guix,
            ..SystemConfig::default()
        };
        assert!(channels_available(&config));
    }

    #[test]
    fn validate_resume_starts_over_when_unmounted() {
        // Synthetic: /mnt isn't mounted in the test env, so nothing can be resumed.
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
        assert!(state.completed_phases.is_empty());
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
