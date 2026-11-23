mod headless;
mod repl;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use guix_install_core::config::{
    BlockDevice, DesktopEnvironment, EncryptionConfig, Filesystem, Firmware, SystemConfig,
    UserAccount, generate_hostname, validate_config_id, validate_hostname, validate_ssh_public_key,
    validate_username,
};
use guix_install_core::disk::detect::{detect_block_devices, format_device};
use guix_install_core::install;
use guix_install_core::mode::InstallMode;
use guix_install_core::network;
use guix_install_core::run;
use guix_install_core::scheme::channels::render_channels;
use guix_install_core::scheme::operating_system::render_operating_system;

use headless::Headless;
use repl::Repl;

/// Env var supplying the user account password in unattended mode.
const ENV_PASSWORD: &str = "GUIX_INSTALL_PASSWORD";
/// Env var supplying the LUKS passphrase in unattended mode (when `--encrypt`).
const ENV_LUKS_PASSPHRASE: &str = "GUIX_INSTALL_LUKS_PASSPHRASE";

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

    /// Run the install non-interactively from flags. Requires --disk and the
    /// GUIX_INSTALL_PASSWORD env var (plus GUIX_INSTALL_LUKS_PASSPHRASE when
    /// --encrypt is set). No prompts, no confirmation — proceeds straight to
    /// formatting the disk.
    #[arg(long)]
    unattended: bool,

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
        "ext4" | "btrfs" | "xfs" => Ok(s.to_string()),
        _ => Err(format!("invalid filesystem: {s} (expected ext4|btrfs|xfs)")),
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
        "xfs" => Filesystem::Xfs,
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
            passphrase: None,
        })
    } else {
        None
    };

    let mut config = SystemConfig {
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
    };
    config.disk.assign_partition_uuids();
    Ok(config)
}

fn run_interactive(dry_run: bool) -> Result<()> {
    let mut ui = Repl::new();
    run::run_interactive(&mut ui, dry_run)
}

/// Reads a required secret from `var`, erroring if it is unset or empty.
fn read_secret_env(var: &str) -> Result<zeroize::Zeroizing<String>> {
    let value = std::env::var(var)
        .ok()
        .filter(|s| !s.is_empty())
        .context(format!(
            "{var} is not set; unattended install needs it to set the password/passphrase"
        ))?;
    Ok(zeroize::Zeroizing::new(value))
}

/// Non-interactive install driven entirely by flags + secret env vars.
///
/// Bypasses the step navigator: builds the config from CLI flags, injects the
/// password (and LUKS passphrase, if encrypting) from the environment, resolves
/// the real target device, then runs the 8-phase pipeline with a non-prompting
/// UI. Intended for automated VM testing and scripted deployments.
fn run_unattended(cli: &Cli) -> Result<()> {
    if cli.disk.is_none() {
        anyhow::bail!("--unattended requires --disk <device> (refusing to guess a target disk)");
    }

    let mut config = build_config(cli)?;

    // Fill in the real device details so the summary/layout match what interactive
    // installs see. Not strictly needed for partition/format (they only use the
    // path + firmware), but keeps size/model accurate and fails loudly if the disk
    // is absent.
    match detect_block_devices() {
        Ok(devices) => {
            match devices
                .into_iter()
                .find(|d| d.dev_path == config.disk.dev_path)
            {
                Some(dev) => config.disk = dev,
                None => anyhow::bail!(
                    "target disk {} not found among detected devices",
                    config.disk.dev_path
                ),
            }
        }
        Err(e) => eprintln!("warning: disk detection failed ({e}); using --disk path as given"),
    }

    config.password = Some(read_secret_env(ENV_PASSWORD)?);

    if let Some(enc) = config.encryption.as_mut() {
        enc.passphrase = Some(read_secret_env(ENV_LUKS_PASSPHRASE)?);
    }

    let encrypt = if config.encryption.is_some() {
        "yes"
    } else {
        "no"
    };
    println!(
        "guix-install {} unattended install: mode={} disk={} filesystem={} encrypt={}",
        env!("CARGO_PKG_VERSION"),
        config.mode.label(),
        config.disk.dev_path,
        config.filesystem,
        encrypt
    );
    println!(
        "{} will be formatted. ALL DATA WILL BE LOST.",
        config.disk.dev_path
    );

    install::execute_installation(&config, &Headless)
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
            let mut ui = Repl::new();
            let mode = match cli.mode.as_str() {
                "guix" => InstallMode::Guix,
                "nonguix" => InstallMode::Nonguix,
                _ => InstallMode::Panther,
            };
            network::connect_flow(&mut ui, &mode)?;
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
            } else if cli.unattended {
                run_unattended(&cli)?;
            } else {
                run_interactive(cli.dry_run)?;
            }
        }
    }

    Ok(())
}
