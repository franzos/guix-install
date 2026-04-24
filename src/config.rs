use std::fmt;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::mode::InstallMode;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemConfig {
    pub mode: InstallMode,
    pub firmware: Firmware,
    pub hostname: String,
    pub timezone: String,
    pub locale: String,
    pub keyboard_layout: Option<String>,
    pub disk: BlockDevice,
    pub filesystem: Filesystem,
    pub encryption: Option<EncryptionConfig>,
    pub users: Vec<UserAccount>,
    pub desktop: Option<DesktopEnvironment>,
    pub ssh_key: Option<String>,
    pub swap_size_mb: u32,
    /// User password for chroot passwd after install. Not written to .scm.
    #[serde(skip)]
    pub password: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserAccount {
    pub name: String,
    pub comment: String,
    pub groups: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptionConfig {
    pub device_target: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Firmware {
    Efi,
    Bios,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Filesystem {
    Ext4,
    Btrfs,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum DesktopEnvironment {
    Gnome,
    Kde,
    Xfce,
    Mate,
    Sway,
    I3,
    Lxqt,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockDevice {
    pub name: String,
    pub dev_path: String,
    pub size_bytes: u64,
    pub model: Option<String>,
    pub boot_partition_uuid: Option<String>,
    pub root_partition_uuid: Option<String>,
}

impl fmt::Display for Firmware {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Firmware::Efi => write!(f, "efi"),
            Firmware::Bios => write!(f, "bios"),
        }
    }
}

impl fmt::Display for Filesystem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Filesystem::Ext4 => write!(f, "ext4"),
            Filesystem::Btrfs => write!(f, "btrfs"),
        }
    }
}

impl fmt::Display for DesktopEnvironment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DesktopEnvironment::Gnome => write!(f, "gnome"),
            DesktopEnvironment::Kde => write!(f, "kde"),
            DesktopEnvironment::Xfce => write!(f, "xfce"),
            DesktopEnvironment::Mate => write!(f, "mate"),
            DesktopEnvironment::Sway => write!(f, "sway"),
            DesktopEnvironment::I3 => write!(f, "i3"),
            DesktopEnvironment::Lxqt => write!(f, "lxqt"),
        }
    }
}

impl Firmware {
    pub fn detect() -> Self {
        if Path::new("/sys/firmware/efi").exists() {
            Firmware::Efi
        } else {
            Firmware::Bios
        }
    }
}

impl Default for SystemConfig {
    fn default() -> Self {
        SystemConfig {
            mode: InstallMode::default(),
            firmware: Firmware::detect(),
            hostname: generate_hostname(&InstallMode::default()),
            timezone: "Europe/Berlin".into(),
            locale: "en_US.utf8".into(),
            keyboard_layout: None,
            disk: BlockDevice {
                name: "sda".into(),
                dev_path: "/dev/sda".into(),
                size_bytes: 0,
                model: None,
                boot_partition_uuid: None,
                root_partition_uuid: None,
            },
            filesystem: Filesystem::Ext4,
            encryption: None,
            users: vec![UserAccount {
                name: "panther".into(),
                comment: "panther's account".into(),
                groups: vec!["wheel".into(), "audio".into(), "video".into()],
            }],
            desktop: None,
            ssh_key: None,
            swap_size_mb: 4096,
            password: None,
        }
    }
}

pub fn validate_hostname(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > 63 {
        return Err("hostname must be 1-63 characters".into());
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err("hostname must contain only lowercase letters, digits, and hyphens".into());
    }
    if name.starts_with('-') || name.ends_with('-') {
        return Err("hostname must not start or end with a hyphen".into());
    }
    Ok(())
}

pub fn validate_username(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > 32 {
        return Err("username must be 1-32 characters".into());
    }
    if !name.starts_with(|c: char| c.is_ascii_lowercase() || c == '_') {
        return Err("username must start with a lowercase letter or underscore".into());
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
    {
        return Err(
            "username must contain only lowercase letters, digits, underscores, and hyphens".into(),
        );
    }
    Ok(())
}

pub fn validate_config_id(id: &str) -> Result<(), String> {
    if id.is_empty() {
        return Err("config ID must not be empty".into());
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(
            "config ID must contain only alphanumeric characters, hyphens, and underscores".into(),
        );
    }
    Ok(())
}

pub fn generate_hostname(mode: &InstallMode) -> String {
    use rand::Rng;

    let prefix = mode.label();
    let mut rng = rand::thread_rng();
    let suffix: String = (0..6)
        .map(|_| {
            let idx = rng.gen_range(0..36);
            if idx < 10 {
                (b'0' + idx) as char
            } else {
                (b'a' + idx - 10) as char
            }
        })
        .collect();
    format!("{prefix}-{suffix}")
}
