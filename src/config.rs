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
    /// User-edited replacement for the rendered system.scm. When set, phase 5
    /// writes this verbatim instead of rendering from the other config fields.
    #[serde(default)]
    pub system_scm_override: Option<String>,
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
            system_scm_override: None,
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

/// Validate an OpenSSH `authorized_keys`-style public key.
///
/// Decodes the base64 body and verifies its embedded length-prefixed algorithm
/// name matches the line's prefix. That single check rejects empty input,
/// pasted private keys, mangled paste, and prefix/body mismatches without
/// hardcoding the set of valid algorithms.
pub fn validate_ssh_public_key(key: &str) -> Result<(), String> {
    use base64::Engine;

    let line = key.trim();
    let mut parts = line.split_whitespace();
    let prefix = parts.next().ok_or("ssh key is empty")?;
    let body_b64 = parts.next().ok_or("ssh key is missing the key body")?;

    let blob = base64::engine::general_purpose::STANDARD
        .decode(body_b64)
        .map_err(|_| "ssh key body is not valid base64".to_string())?;

    let len = blob
        .get(0..4)
        .and_then(|b| Some(u32::from_be_bytes(b.try_into().ok()?) as usize))
        .ok_or("ssh key body is too short")?;
    let inner = blob
        .get(4..4 + len)
        .ok_or("ssh key length prefix is invalid")?;

    if inner != prefix.as_bytes() {
        return Err(format!(
            "ssh key prefix \"{prefix}\" does not match embedded algorithm name"
        ));
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

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    fn make_key(prefix: &str, body: &[u8]) -> String {
        let mut blob = Vec::new();
        blob.extend(&(prefix.len() as u32).to_be_bytes());
        blob.extend(prefix.as_bytes());
        blob.extend(body);
        let b64 = base64::engine::general_purpose::STANDARD.encode(&blob);
        format!("{prefix} {b64}")
    }

    #[test]
    fn ssh_key_valid_ed25519() {
        let key = make_key("ssh-ed25519", &[0u8; 32]);
        assert!(validate_ssh_public_key(&key).is_ok());
    }

    #[test]
    fn ssh_key_valid_with_comment() {
        let key = format!("{} user@host", make_key("ssh-rsa", &[0u8; 256]));
        assert!(validate_ssh_public_key(&key).is_ok());
    }

    #[test]
    fn ssh_key_empty_rejected() {
        assert!(validate_ssh_public_key("").is_err());
        assert!(validate_ssh_public_key("   ").is_err());
    }

    #[test]
    fn ssh_key_missing_body_rejected() {
        assert!(validate_ssh_public_key("ssh-ed25519").is_err());
    }

    #[test]
    fn ssh_key_bad_base64_rejected() {
        assert!(validate_ssh_public_key("ssh-ed25519 not_valid_base64!!!").is_err());
    }

    #[test]
    fn ssh_key_prefix_mismatch_rejected() {
        let key = make_key("ssh-ed25519", &[0u8; 32]);
        let tampered = key.replace("ssh-ed25519 ", "ssh-rsa ");
        let err = validate_ssh_public_key(&tampered).unwrap_err();
        assert!(err.contains("does not match"));
    }

    #[test]
    fn ssh_key_truncated_blob_rejected() {
        // Length prefix says 100 bytes but blob has only 4
        let blob = (100u32).to_be_bytes();
        let b64 = base64::engine::general_purpose::STANDARD.encode(blob);
        let key = format!("ssh-ed25519 {b64}");
        assert!(validate_ssh_public_key(&key).is_err());
    }
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
