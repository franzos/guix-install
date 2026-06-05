//! Wrapper over `connmanctl` — mirrors gnu/installer/connman.scm.
//! Every invocation forces `LANG=C LC_ALL=C` so parsing is locale-stable.

use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use zeroize::Zeroizing;

use crate::exec::{self, CommandResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tech {
    Wifi,
    Ethernet,
}

impl Tech {
    fn as_str(self) -> &'static str {
        match self {
            Tech::Wifi => "wifi",
            Tech::Ethernet => "ethernet",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkState {
    Online,
    Ready,
    Idle,
    Association,
    Failure,
    Disconnect,
    Unknown,
}

impl LinkState {
    /// `online` (internet verified) or `ready` (IP, no portal check) both count
    /// as "connected enough", matching connman.scm's `connman-online?`.
    pub fn is_connected(self) -> bool {
        matches!(self, LinkState::Online | LinkState::Ready)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Service {
    pub path: String,
    pub name: String,
    pub tech: Tech,
    pub connected: bool,
    pub secured: bool,
    pub eap: bool,
}

/// Marker chars in the `connmanctl services` flags column (`*AO`, `*AR`, …).
const MARKER_CHARS: &[char] = &['*', 'A', 'O', 'R', 'a', 'c', 'd', 'p', 'f'];

/// Parse `connmanctl state` output (`  State = online`).
pub fn parse_state(stdout: &str) -> LinkState {
    for line in stdout.lines() {
        let line = line.trim();
        if let Some(val) = line.strip_prefix("State =") {
            return match val.trim() {
                "online" => LinkState::Online,
                "ready" => LinkState::Ready,
                "idle" => LinkState::Idle,
                "association" => LinkState::Association,
                "configuration" => LinkState::Association,
                "failure" => LinkState::Failure,
                "disconnect" | "offline" => LinkState::Disconnect,
                _ => LinkState::Unknown,
            };
        }
    }
    LinkState::Unknown
}

/// Parse the `connmanctl services` summary into services.
///
/// Each line is `[flags]  <name>  <service-path>`; the path is the last
/// whitespace token. The leading token is treated as flags only if every
/// char is a marker char (so a network literally named "AO" is mis-flagged —
/// acceptable for v1).
pub fn parse_services(stdout: &str) -> Vec<Service> {
    let mut out = Vec::new();
    for line in stdout.lines() {
        let trimmed = line.trim_end();
        if trimmed.trim().is_empty() {
            continue;
        }
        let Some((left_raw, path)) = trimmed.rsplit_once(char::is_whitespace) else {
            continue;
        };
        let path = path.to_string();
        let tech = if path.starts_with("wifi_") {
            Tech::Wifi
        } else if path.starts_with("ethernet_") {
            Tech::Ethernet
        } else {
            continue;
        };

        let left = left_raw.trim();
        let (flags, name) = match left.split_once(char::is_whitespace) {
            Some((first, rest)) if first.chars().all(|c| MARKER_CHARS.contains(&c)) => {
                (first, rest.trim())
            }
            _ => ("", left),
        };

        out.push(Service {
            secured: !path.ends_with("_managed_none"),
            eap: path.contains("_ieee8021x"),
            connected: flags.contains('O') || flags.contains('R'),
            path,
            name: name.to_string(),
            tech,
        });
    }
    out
}

fn connmanctl_bin() -> String {
    std::env::var("GUIX_INSTALL_CONNMANCTL").unwrap_or_else(|_| "connmanctl".into())
}

/// Run `connmanctl <args>` with a forced C locale (connman.scm:164).
fn cc(args: &[&str]) -> Result<CommandResult> {
    let bin = connmanctl_bin();
    let mut full: Vec<&str> = vec!["env", "LANG=C", "LC_ALL=C", bin.as_str()];
    full.extend_from_slice(args);
    exec::run_cmd(&full)
}

/// Overall connectivity (`connmanctl state`). Errors if the daemon is absent.
pub fn state() -> Result<LinkState> {
    Ok(parse_state(&cc(&["state"])?.stdout))
}

/// Available Wi-Fi/ethernet services (`connmanctl services`).
pub fn services() -> Result<Vec<Service>> {
    Ok(parse_services(&cc(&["services"])?.stdout))
}

/// Power on a technology and wait (bounded) for `Powered = True`.
///
/// Distinguishes a hardware/rfkill block (`Available = False`) from a transient
/// power-up delay so the caller can show an actionable message.
pub fn enable(tech: Tech) -> Result<()> {
    // Idempotent: "Already enabled" exits non-zero — ignore it.
    let _ = cc(&["enable", tech.as_str()]);

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if Instant::now() >= deadline {
            bail!("{} did not power on within 10s", tech.as_str());
        }
        let techs = cc(&["technologies"])?.stdout;
        let (available, powered) = technology_flags(&techs, tech);
        if !available {
            bail!("{} is hardware-disabled (check the physical switch / rfkill)", tech.as_str());
        }
        if powered {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// Trigger a scan round (`connmanctl scan wifi`); blocks until the round ends.
pub fn scan(tech: Tech) -> Result<()> {
    cc(&["scan", tech.as_str()]).map(|_| ())
}

/// Parse `connmanctl technologies` for one technology's Available/Powered flags.
/// The block for a technology starts at `/net/connman/technology/<tech>` and its
/// properties are indented `Key = Value` lines until the next block.
pub(crate) fn technology_flags(stdout: &str, tech: Tech) -> (bool, bool) {
    let marker = format!("/technology/{}", tech.as_str());
    let mut in_block = false;
    let mut available = false;
    let mut powered = false;
    let mut saw_available = false;
    for line in stdout.lines() {
        if line.starts_with('/') {
            in_block = line.contains(&marker);
            continue;
        }
        if in_block {
            let l = line.trim();
            if let Some(v) = l.strip_prefix("Powered =") {
                powered = v.trim() == "True";
            } else if let Some(v) = l.strip_prefix("Available =") {
                available = v.trim() == "True";
                saw_available = true;
            }
        }
    }
    // connman omits Available when the device is present; default true only if
    // the target block never stated it.
    if !saw_available {
        available = true;
    }
    (available, powered)
}

const DEFAULT_PROVISION_DIR: &str = "/var/lib/connman";

fn provision_dir() -> String {
    std::env::var("GUIX_INSTALL_CONNMAN_DIR").unwrap_or_else(|_| DEFAULT_PROVISION_DIR.into())
}

fn provision_file() -> String {
    format!("{}/guix-install.config", provision_dir())
}

/// Render a connman provisioning file body binding an SSID + passphrase.
pub(crate) fn render_provisioning(path: &str, name: &str, passphrase: &str) -> String {
    format!("[service_{path}]\nType = wifi\nName = {name}\nPassphrase = {passphrase}\n")
}

/// Removes the provisioning file on drop (also on error paths).
struct ProvisionGuard {
    file: String,
}
impl Drop for ProvisionGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.file);
    }
}

/// Connect to a service. For secured networks, writes a 0600 provisioning file
/// (passphrase never goes on the argv), triggers connect, polls until connected.
pub fn connect(path: &str, name: &str, passphrase: Option<&Zeroizing<String>>) -> Result<()> {
    connect_with_deadline(path, name, passphrase, Duration::from_secs(30))
}

pub(crate) fn connect_with_deadline(
    path: &str,
    name: &str,
    passphrase: Option<&Zeroizing<String>>,
    timeout: Duration,
) -> Result<()> {
    let _guard: Option<ProvisionGuard> = if let Some(pw) = passphrase {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;

        if pw.contains('\n') || pw.contains('\r') {
            bail!("Wi-Fi passphrase contains invalid characters");
        }

        let file = provision_file();
        std::fs::create_dir_all(provision_dir()).ok();
        let body = render_provisioning(path, name, pw);
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&file)?;
        f.write_all(body.as_bytes())?;
        f.sync_all().ok();
        Some(ProvisionGuard { file })
    } else {
        None
    };

    let deadline = Instant::now() + timeout;
    if let Err(e) = cc(&["connect", path]) {
        // connmanctl `connect` may exit non-zero yet still come up; let the poll
        // decide, but surface the original error if it never connects.
        return poll_connected(deadline)
            .map_err(|poll_err| e.context(format!("connect poll also failed: {poll_err}")));
    }
    poll_connected(deadline)
}

fn poll_connected(deadline: Instant) -> Result<()> {
    loop {
        if state()?.is_connected() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            bail!("connection timed out — check signal and passphrase");
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// Test-only public alias for the deadline-parameterised connect. Not stable API.
#[doc(hidden)]
pub fn connect_with_deadline_pub(
    path: &str,
    name: &str,
    passphrase: Option<&Zeroizing<String>>,
    timeout: Duration,
) -> Result<()> {
    connect_with_deadline(path, name, passphrase, timeout)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TECHS: &str = "\
/net/connman/technology/ethernet
  Name = Wired
  Type = ethernet
  Powered = True
  Connected = True

/net/connman/technology/wifi
  Name = WiFi
  Type = wifi
  Powered = True
  Connected = False
";

    #[test]
    fn wifi_powered_parsed() {
        let (available, powered) = technology_flags(TECHS, Tech::Wifi);
        assert!(available);
        assert!(powered);
    }

    #[test]
    fn rfkill_block_detected() {
        let blocked = "/net/connman/technology/wifi\n  Powered = False\n  Available = False\n";
        let (available, powered) = technology_flags(blocked, Tech::Wifi);
        assert!(!available);
        assert!(!powered);
    }

    #[test]
    fn available_fallback_ignores_other_tech() {
        let s = "/net/connman/technology/ethernet\n  Available = True\n\n\
                 /net/connman/technology/wifi\n  Powered = False\n";
        let (available, powered) = technology_flags(s, Tech::Wifi);
        assert!(available); // no Available line in wifi block → defaults true
        assert!(!powered);
    }

    #[test]
    fn provisioning_body() {
        let body = render_provisioning("wifi_x_managed_psk", "My Net", "secret");
        assert!(body.contains("[service_wifi_x_managed_psk]"));
        assert!(body.contains("Type = wifi"));
        assert!(body.contains("Name = My Net"));
        assert!(body.contains("Passphrase = secret"));
    }
}
