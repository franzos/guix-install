use anyhow::Result;
use clap::{Parser, Subcommand};

use guix_install::config::{
    BlockDevice, DesktopEnvironment, EncryptionConfig, Filesystem, Firmware, SystemConfig,
    UserAccount, generate_hostname, validate_config_id, validate_hostname, validate_ssh_public_key,
    validate_username,
};
use guix_install::disk::detect::{detect_block_devices, format_device};
use guix_install::install;
use guix_install::mode::InstallMode;
use guix_install::repl::Repl;
use guix_install::resume::InstallState;
use guix_install::scheme::channels::render_channels;
use guix_install::scheme::operating_system::render_operating_system;
use guix_install::steps::StepResult;
use guix_install::steps::desktop::step_desktop;
use guix_install::steps::disk::step_disk;
use guix_install::steps::encryption::step_encryption;
use guix_install::steps::hostname::step_hostname;
use guix_install::steps::locale::step_locale;
use guix_install::steps::mode::step_mode;
use guix_install::steps::summary::step_summary;
use guix_install::steps::timezone::step_timezone;
use guix_install::steps::users::step_users;
use guix_install::steps::{StepId, StepNavigator};
use guix_install::ui::UserInterface;
use guix_install::wifi;

#[derive(Parser)]
#[command(
    name = "guix-install",
    about = "Guix system installer — supports Guix, Nonguix, Panther, and Enterprise modes",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Installation mode
    #[arg(long, default_value = "panther", value_parser = parse_mode)]
    mode: String,

    /// System hostname
    #[arg(long)]
    hostname: Option<String>,

    /// Timezone
    #[arg(long, default_value = "Europe/Berlin")]
    timezone: String,

    /// Locale
    #[arg(long, default_value = "en_US.utf8")]
    locale: String,

    /// Target disk (e.g., /dev/sda)
    #[arg(long)]
    disk: Option<String>,

    /// Filesystem type
    #[arg(long, default_value = "ext4", value_parser = parse_filesystem)]
    filesystem: String,

    /// Enable LUKS disk encryption
    #[arg(long)]
    encrypt: bool,

    /// Username
    #[arg(long, default_value = "panther")]
    username: String,

    /// Desktop environment
    #[arg(long, value_parser = parse_desktop)]
    desktop: Option<String>,

    /// Keyboard layout (e.g., us, de)
    #[arg(long)]
    keyboard: Option<String>,

    /// SSH public key
    #[arg(long)]
    ssh_key: Option<String>,

    /// Swap size in MB
    #[arg(long, default_value = "4096")]
    swap: u32,

    /// Enterprise config ID (implies --mode enterprise)
    #[arg(long)]
    config: Option<String>,

    /// Enterprise config base URL
    #[arg(long, default_value = "https://temp.pantherx.org/install")]
    config_url: String,

    /// Print generated config without executing
    #[arg(long)]
    dry_run: bool,

    /// Skip confirmation prompts
    #[arg(long)]
    yes: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// List available disks
    ListDisks,
    /// Connect to WiFi
    Wifi,
}

fn parse_mode(s: &str) -> Result<String, String> {
    match s {
        "guix" | "nonguix" | "panther" | "enterprise" => Ok(s.to_string()),
        _ => Err(format!(
            "invalid mode: {s} (expected guix|nonguix|panther|enterprise)"
        )),
    }
}

fn parse_filesystem(s: &str) -> Result<String, String> {
    match s {
        "ext4" | "btrfs" => Ok(s.to_string()),
        _ => Err(format!("invalid filesystem: {s} (expected ext4|btrfs)")),
    }
}

fn parse_desktop(s: &str) -> Result<String, String> {
    match s {
        "gnome" | "kde" | "xfce" | "mate" | "sway" | "i3" | "lxqt" => Ok(s.to_string()),
        _ => Err(format!(
            "invalid desktop: {s} (expected gnome|kde|xfce|mate|sway|i3|lxqt)"
        )),
    }
}

fn build_config(cli: &Cli) -> Result<SystemConfig> {
    let mode = if let Some(config_id) = &cli.config {
        validate_config_id(config_id).map_err(|e| anyhow::anyhow!(e))?;
        InstallMode::Enterprise {
            config_id: config_id.clone(),
            config_url: cli.config_url.clone(),
        }
    } else {
        match cli.mode.as_str() {
            "guix" => InstallMode::Guix,
            "nonguix" => InstallMode::Nonguix,
            "enterprise" => {
                anyhow::bail!("--mode enterprise requires --config <ID>");
            }
            _ => InstallMode::Panther,
        }
    };

    let disk_path = cli.disk.clone().unwrap_or_else(|| "/dev/sda".into());
    let disk_name = disk_path
        .strip_prefix("/dev/")
        .unwrap_or(&disk_path)
        .to_string();

    let hostname = match &cli.hostname {
        Some(h) => {
            validate_hostname(h).map_err(|e| anyhow::anyhow!(e))?;
            h.clone()
        }
        None => generate_hostname(&mode),
    };

    validate_username(&cli.username).map_err(|e| anyhow::anyhow!(e))?;

    if let Some(key) = &cli.ssh_key {
        validate_ssh_public_key(key).map_err(|e| anyhow::anyhow!(e))?;
    }

    let filesystem = match cli.filesystem.as_str() {
        "btrfs" => Filesystem::Btrfs,
        _ => Filesystem::Ext4,
    };

    let desktop = cli.desktop.as_deref().map(|d| match d {
        "gnome" => DesktopEnvironment::Gnome,
        "kde" => DesktopEnvironment::Kde,
        "xfce" => DesktopEnvironment::Xfce,
        "mate" => DesktopEnvironment::Mate,
        "sway" => DesktopEnvironment::Sway,
        "i3" => DesktopEnvironment::I3,
        "lxqt" => DesktopEnvironment::Lxqt,
        _ => unreachable!(),
    });

    let encryption = if cli.encrypt {
        Some(EncryptionConfig {
            device_target: "cryptroot".into(),
        })
    } else {
        None
    };

    Ok(SystemConfig {
        mode,
        firmware: Firmware::detect(),
        hostname,
        timezone: cli.timezone.clone(),
        locale: cli.locale.clone(),
        keyboard_layout: cli.keyboard.clone(),
        disk: BlockDevice {
            name: disk_name,
            dev_path: disk_path,
            size_bytes: 0,
            model: None,
            boot_partition_uuid: None,
            root_partition_uuid: None,
        },
        filesystem,
        encryption,
        users: vec![UserAccount {
            name: cli.username.clone(),
            comment: format!("{}'s account", cli.username),
            groups: vec!["wheel".into(), "audio".into(), "video".into()],
        }],
        desktop,
        ssh_key: cli.ssh_key.clone(),
        swap_size_mb: cli.swap,
        password: None,
        system_scm_override: None,
    })
}

/// On startup, if a saved install state exists, ask the user whether to resume
/// it or discard and start fresh. Returns `Some(config)` if resuming, `None` if
/// starting fresh.
fn handle_resume(ui: &mut Repl, state: InstallState) -> Result<Option<SystemConfig>> {
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
    Ok(Some(config))
}

fn run_interactive(dry_run: bool) -> Result<()> {
    let mut ui = Repl::new();

    ui.info("guix-install — Guix System Installer");
    ui.info("");

    if !dry_run
        && let Some(state) = InstallState::load()?
        && let Some(config) = handle_resume(&mut ui, state)?
    {
        return install::execute_installation(&config, &ui);
    }

    let mut config = SystemConfig::default();
    let mut nav = StepNavigator::new(&config.mode);

    loop {
        let result = match nav.current() {
            StepId::Mode => {
                let old_mode = config.mode.clone();
                let r = step_mode(&mut ui, &mut config)?;
                if config.mode != old_mode {
                    nav.reset_for_mode(&config.mode);
                    continue;
                }
                r
            }
            StepId::Locale => step_locale(&mut ui, &mut config)?,
            StepId::Timezone => step_timezone(&mut ui, &mut config)?,
            StepId::Hostname => step_hostname(&mut ui, &mut config)?,
            StepId::Disk => step_disk(&mut ui, &mut config)?,
            StepId::Encryption => step_encryption(&mut ui, &mut config)?,
            StepId::Users => step_users(&mut ui, &mut config)?,
            StepId::Desktop => step_desktop(&mut ui, &mut config)?,
            StepId::Summary => step_summary(&mut ui, &mut config)?,
        };

        match result {
            StepResult::Next => {
                if nav.is_last() {
                    break;
                }
                nav.advance();
            }
            StepResult::Back => nav.go_back(),
            StepResult::Quit => {
                ui.info("Installation cancelled.");
                return Ok(());
            }
        }
    }

    // After REPL completes, show generated config
    let system_scm = render_operating_system(&config);
    if !system_scm.is_empty() {
        println!("\n;;; Generated system.scm:");
        println!("{system_scm}");
    }

    if dry_run {
        ui.info("Dry run: skipping installation execution.");
        return Ok(());
    }

    install::execute_installation(&config, &ui)?;
    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match &cli.command {
        Some(Commands::ListDisks) => {
            let devices = detect_block_devices()?;
            if devices.is_empty() {
                eprintln!("No disks found.");
            } else {
                for dev in &devices {
                    println!("{}", format_device(dev));
                }
            }
        }
        Some(Commands::Wifi) => {
            wifi::wifi_connect()?;
        }
        None => {
            if cli.dry_run {
                let config = build_config(&cli)?;

                let system_scm = render_operating_system(&config);
                if !system_scm.is_empty() {
                    println!(";;; system.scm");
                    println!("{system_scm}");
                } else {
                    println!(";;; Enterprise mode: system.scm comes from remote config");
                }

                let channels_scm = render_channels(&config.mode);
                if let Some(ch) = channels_scm {
                    println!();
                    println!(";;; channels.scm");
                    println!("{ch}");
                }
            } else {
                run_interactive(cli.dry_run)?;
            }
        }
    }

    Ok(())
}
