//! Preflight check for devices that need non-free firmware.
//!
//! Ported from `gnu/installer/hardware.scm` in the official Guix installer.
//! Only relevant in `InstallMode::Guix` — Nonguix/Panther/Enterprise ship
//! the firmware these devices need.

use std::fs;
use std::path::{Path, PathBuf};

/// Linux modules that drive PCI devices needing non-free firmware.
///
/// Kept in sync with `gnu/installer/hardware.scm:%unsupported-linux-modules`.
const UNSUPPORTED_MODULES: &[&str] = &[
    // Wi-Fi.
    "brcmfmac",
    "ipw2100",
    "ipw2200",
    "iwlwifi",
    "mwl8k",
    "rtl8188ee",
    "rtl818x_pci",
    "rtl8192ce",
    "rtl8192de",
    "rtl8192ee",
    // Ethernet.
    "bnx2",
    "bnx2x",
    "liquidio",
    // Graphics.
    "amdgpu",
    "radeon",
    // Multimedia.
    "ivtv",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsupportedDevice {
    pub vendor_id: u16,
    pub device_id: u16,
    pub module: String,
}

impl UnsupportedDevice {
    pub fn description(&self) -> String {
        format!(
            "{:04x}:{:04x} ({})",
            self.vendor_id, self.device_id, self.module
        )
    }
}

pub fn detect_unsupported_devices() -> Vec<UnsupportedDevice> {
    let release = match fs::read_to_string("/proc/sys/kernel/osrelease") {
        Ok(s) => s.trim().to_string(),
        Err(_) => return Vec::new(),
    };
    let alias_file = PathBuf::from(format!("/lib/modules/{}/modules.alias", release));
    detect_unsupported_devices_at(Path::new("/sys/bus/pci/devices"), &alias_file)
}

pub fn uvesafb_loaded() -> bool {
    uvesafb_loaded_at(Path::new("/proc/modules"))
}

fn uvesafb_loaded_at(path: &Path) -> bool {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return false,
    };
    content
        .lines()
        .any(|line| line.split_whitespace().next() == Some("uvesafb"))
}

fn detect_unsupported_devices_at(pci_root: &Path, alias_file: &Path) -> Vec<UnsupportedDevice> {
    let aliases = match parse_modules_alias(alias_file) {
        Ok(a) => a,
        Err(_) => return Vec::new(),
    };

    let entries = match fs::read_dir(pci_root) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut found = Vec::new();
    for entry in entries.flatten() {
        let modalias = match fs::read_to_string(entry.path().join("modalias")) {
            Ok(s) => s.trim().to_string(),
            Err(_) => continue,
        };

        for (pattern, module) in &aliases {
            if glob_match(pattern, &modalias) {
                let (vendor_id, device_id) = parse_pci_modalias(&modalias).unwrap_or_default();
                found.push(UnsupportedDevice {
                    vendor_id,
                    device_id,
                    module: module.clone(),
                });
                break;
            }
        }
    }
    found
}

/// Parse `/lib/modules/<release>/modules.alias`, keeping only entries whose
/// target module is in `UNSUPPORTED_MODULES`. The full file is ~20k lines on
/// a typical system; pre-filtering keeps the match loop tight.
fn parse_modules_alias(path: &Path) -> std::io::Result<Vec<(String, String)>> {
    let content = fs::read_to_string(path)?;
    let mut out = Vec::new();
    for line in content.lines() {
        let rest = match line.trim().strip_prefix("alias ") {
            Some(r) => r,
            None => continue,
        };
        let (pattern, module) = match rest.rsplit_once(char::is_whitespace) {
            Some(parts) => parts,
            None => continue,
        };
        let module = module.trim();
        if UNSUPPORTED_MODULES.contains(&module) {
            out.push((pattern.trim().to_string(), module.to_string()));
        }
    }
    Ok(out)
}

/// Standard wildcard match with `*` (any run) and `?` (single char).
/// Two-pointer backtracking — O(n*m) worst case, plenty for short modalias
/// strings.
fn glob_match(pattern: &str, text: &str) -> bool {
    let p = pattern.as_bytes();
    let t = text.as_bytes();
    let (mut pi, mut ti) = (0usize, 0usize);
    let mut star_pi: Option<usize> = None;
    let mut star_ti = 0usize;

    while ti < t.len() {
        if pi < p.len() && (p[pi] == b'?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == b'*' {
            star_pi = Some(pi);
            star_ti = ti;
            pi += 1;
        } else if let Some(spi) = star_pi {
            pi = spi + 1;
            star_ti += 1;
            ti = star_ti;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == b'*' {
        pi += 1;
    }
    pi == p.len()
}

/// Extract `(vendor_id, device_id)` from a PCI modalias such as
/// `pci:v00008086d000015A1sv00008086sd00002001bc02sc00i00`.
fn parse_pci_modalias(modalias: &str) -> Option<(u16, u16)> {
    let s = modalias.strip_prefix("pci:")?.strip_prefix('v')?;
    if s.len() < 8 {
        return None;
    }
    let vendor = u32::from_str_radix(&s[..8], 16).ok()? as u16;
    let s = s[8..].strip_prefix('d')?;
    if s.len() < 8 {
        return None;
    }
    let device = u32::from_str_radix(&s[..8], 16).ok()? as u16;
    Some((vendor, device))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn glob_matches_star() {
        assert!(glob_match("pci:v00008086d*", "pci:v00008086d000024F3"));
        assert!(glob_match("*", ""));
        assert!(glob_match("*", "anything"));
        assert!(glob_match("a*c", "abc"));
        assert!(glob_match("a*c", "ac"));
        assert!(glob_match("a*c", "axxxxxc"));
        assert!(!glob_match("a*c", "ab"));
        assert!(!glob_match("a*c", "abd"));
    }

    #[test]
    fn glob_matches_question() {
        assert!(glob_match("a?c", "abc"));
        assert!(glob_match("a?c", "axc"));
        assert!(!glob_match("a?c", "ac"));
        assert!(!glob_match("a?c", "abbc"));
    }

    #[test]
    fn glob_matches_pci_alias() {
        let pat = "pci:v00008086d*sv*sd*bc*sc*i*";
        assert!(glob_match(
            pat,
            "pci:v00008086d000024F3sv00008086sd00002001bc02sc80i00"
        ));
        assert!(!glob_match(
            pat,
            "pci:v000010DEd00001C03sv00001458sd0000374Cbc03sc00i00"
        ));
    }

    #[test]
    fn parses_pci_modalias() {
        let m = "pci:v00008086d000024F3sv00008086sd00002001bc02sc80i00";
        assert_eq!(parse_pci_modalias(m), Some((0x8086, 0x24F3)));
    }

    #[test]
    fn rejects_non_pci_modalias() {
        assert_eq!(parse_pci_modalias("usb:v1D6Bp0002d0510"), None);
        assert_eq!(parse_pci_modalias("pci:v0000"), None);
    }

    #[test]
    fn unsupported_device_description() {
        let d = UnsupportedDevice {
            vendor_id: 0x8086,
            device_id: 0x24F3,
            module: "iwlwifi".into(),
        };
        assert_eq!(d.description(), "8086:24f3 (iwlwifi)");
    }

    /// End-to-end: fixture sysfs + modules.alias → detect iwlwifi device.
    #[test]
    fn detects_unsupported_iwlwifi_device() {
        let dir = tempdir().unwrap();
        let pci_root = dir.path().join("sys/bus/pci/devices");
        fs::create_dir_all(&pci_root).unwrap();

        // Intel Wi-Fi 8260 (iwlwifi).
        let dev1 = pci_root.join("0000:03:00.0");
        fs::create_dir_all(&dev1).unwrap();
        fs::write(
            dev1.join("modalias"),
            "pci:v00008086d000024F3sv00008086sd00002001bc02sc80i00\n",
        )
        .unwrap();

        // PCI bridge (pcieport — supported).
        let dev2 = pci_root.join("0000:00:1c.0");
        fs::create_dir_all(&dev2).unwrap();
        fs::write(
            dev2.join("modalias"),
            "pci:v00008086d0000A290sv00000000sd00000000bc06sc04i00\n",
        )
        .unwrap();

        let alias_file = dir.path().join("modules.alias");
        fs::write(
            &alias_file,
            "alias pci:v00008086d000024F3sv*sd*bc*sc*i* iwlwifi\n\
             alias pci:v00008086d0000A290sv*sd*bc*sc*i* pcieport\n\
             alias pci:v000010DEd*sv*sd*bc03sc*i* nouveau\n",
        )
        .unwrap();

        let found = detect_unsupported_devices_at(&pci_root, &alias_file);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].module, "iwlwifi");
        assert_eq!(found[0].vendor_id, 0x8086);
        assert_eq!(found[0].device_id, 0x24F3);
    }

    #[test]
    fn detects_amdgpu_device() {
        let dir = tempdir().unwrap();
        let pci_root = dir.path().join("sys/bus/pci/devices");
        fs::create_dir_all(&pci_root).unwrap();

        let dev = pci_root.join("0000:01:00.0");
        fs::create_dir_all(&dev).unwrap();
        fs::write(
            dev.join("modalias"),
            "pci:v00001002d0000731Fsv00001458sd0000230Cbc03sc00i00\n",
        )
        .unwrap();

        let alias_file = dir.path().join("modules.alias");
        fs::write(
            &alias_file,
            "alias pci:v00001002d0000731Fsv*sd*bc*sc*i* amdgpu\n",
        )
        .unwrap();

        let found = detect_unsupported_devices_at(&pci_root, &alias_file);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].module, "amdgpu");
    }

    #[test]
    fn returns_empty_when_no_unsupported_devices() {
        let dir = tempdir().unwrap();
        let pci_root = dir.path().join("sys/bus/pci/devices");
        fs::create_dir_all(&pci_root).unwrap();

        let dev = pci_root.join("0000:00:1c.0");
        fs::create_dir_all(&dev).unwrap();
        fs::write(
            dev.join("modalias"),
            "pci:v00008086d0000A290sv00000000sd00000000bc06sc04i00\n",
        )
        .unwrap();

        let alias_file = dir.path().join("modules.alias");
        fs::write(
            &alias_file,
            "alias pci:v00008086d0000A290sv*sd*bc*sc*i* pcieport\n",
        )
        .unwrap();

        let found = detect_unsupported_devices_at(&pci_root, &alias_file);
        assert!(found.is_empty());
    }

    #[test]
    fn returns_empty_when_paths_missing() {
        let dir = tempdir().unwrap();
        let found = detect_unsupported_devices_at(
            &dir.path().join("nonexistent"),
            &dir.path().join("nonexistent.alias"),
        );
        assert!(found.is_empty());
    }

    #[test]
    fn detects_uvesafb() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("modules");
        fs::write(
            &path,
            "uvesafb 65536 0 - Live 0xffffffffc0123000\n\
             nls_iso8859_1 16384 1 - Live 0xffffffffc0456000\n",
        )
        .unwrap();
        assert!(uvesafb_loaded_at(&path));

        fs::write(
            &path,
            "nls_iso8859_1 16384 1 - Live 0xffffffffc0456000\n\
             usbcore 327680 5 - Live 0xffffffffc0789000\n",
        )
        .unwrap();
        assert!(!uvesafb_loaded_at(&path));
    }

    #[test]
    fn uvesafb_missing_path_returns_false() {
        assert!(!uvesafb_loaded_at(Path::new(
            "/this/does/not/exist/modules"
        )));
    }
}
