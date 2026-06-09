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

/// Parse a `connmanctl services <path>` detail block: indented `Key = Value`
/// lines. Returns the service's `State =` (same mapping as `parse_state`) plus
/// the optional `Error =` value (`invalid-key`, `connect-failed`, `dhcp-failed`).
pub fn parse_service_detail(stdout: &str) -> (LinkState, Option<String>) {
    let mut state = LinkState::Unknown;
    let mut saw_state = false;
    let mut error = None;
    for line in stdout.lines() {
        let l = line.trim();
        if !saw_state && let Some(v) = l.strip_prefix("State =") {
            state = match v.trim() {
                "online" => LinkState::Online,
                "ready" => LinkState::Ready,
                "idle" => LinkState::Idle,
                "association" | "configuration" => LinkState::Association,
                "failure" => LinkState::Failure,
                "disconnect" | "offline" => LinkState::Disconnect,
                _ => LinkState::Unknown,
            };
            saw_state = true;
            continue;
        }
        if error.is_none()
            && let Some(v) = l.strip_prefix("Error =")
        {
            let v = v.trim();
            if !v.is_empty() {
                error = Some(v.to_string());
            }
        }
    }
    (state, error)
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

/// State + optional error of one service (`connmanctl services <path>`).
pub fn service_state(path: &str) -> Result<(LinkState, Option<String>)> {
    Ok(parse_service_detail(&cc(&["services", path])?.stdout))
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
            bail!(
                "{} is hardware-disabled (check the physical switch / rfkill)",
                tech.as_str()
            );
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

/// True if `stdout` contains a technology block path line for `tech`.
pub(crate) fn lists_technology(stdout: &str, tech: Tech) -> bool {
    let marker = format!("/technology/{}", tech.as_str());
    stdout
        .lines()
        .any(|l| l.starts_with('/') && l.contains(&marker))
}

/// Whether connman lists a technology block for `tech` at all (i.e. the hardware
/// + driver is present). Distinguishes "no Wi-Fi adapter" from "Wi-Fi off".
pub fn has_technology(tech: Tech) -> Result<bool> {
    Ok(lists_technology(&cc(&["technologies"])?.stdout, tech))
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

fn strip_control(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).collect()
}

/// Render a connman provisioning file body binding an SSID + passphrase.
/// `name`/`path` come from scan results (attacker-influenced SSIDs), so control
/// characters are stripped to prevent INI line/section injection.
pub(crate) fn render_provisioning(path: &str, name: &str, passphrase: &str) -> String {
    let path = strip_control(path);
    let name = strip_control(name);
    format!("[service_{path}]\nType = wifi\nName = {name}\nPassphrase = {passphrase}\n")
}

/// Removes the provisioning file (and any leftover tmp). For explicit
/// end-of-session cleanup; reboot wipes the tmpfs ISO anyway.
#[allow(dead_code)] // retained for a future final-cleanup step
pub(crate) fn clear_provisioning() {
    let file = provision_file();
    let _ = std::fs::remove_file(&file);
    let _ = std::fs::remove_file(format!("{file}.tmp"));
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
    if let Some(pw) = passphrase {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;

        if pw.contains('\n') || pw.contains('\r') {
            bail!("Wi-Fi passphrase contains invalid characters");
        }

        // The .config must persist for the whole session: deleting it makes
        // connman disconnect the service and wipe its stored credentials.
        let file = provision_file();
        std::fs::create_dir_all(provision_dir()).ok();
        let tmp = format!("{file}.tmp");
        let body = render_provisioning(path, name, pw);
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(body.as_bytes())?;
        f.sync_all().ok();
        drop(f);
        std::fs::rename(&tmp, &file)?; // atomic publish so connman never sees a partial file
    }

    // connmanctl `connect` can exit non-zero yet still associate; its failure is
    // already in the installer log. Let the poll decide and surface its message.
    let _ = cc(&["connect", path]);
    // Start the poll budget AFTER connect returns — `connect` itself can block.
    let deadline = Instant::now() + timeout;
    poll_service_connected(path, deadline)
}

/// Poll the CHOSEN service (not global `state`) — a second already-connected
/// adapter must not make this read `ready`.
fn poll_service_connected(path: &str, deadline: Instant) -> Result<()> {
    loop {
        let (state, error) = service_state(path)?;
        if state.is_connected() {
            return Ok(());
        }
        if state == LinkState::Failure {
            bail!("{}", failure_message(error.as_deref()));
        }
        if Instant::now() >= deadline {
            bail!("connection timed out — check signal and passphrase");
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// Map a connman service `Error =` value to an actionable message.
fn failure_message(error: Option<&str>) -> String {
    match error {
        Some("invalid-key") => "incorrect Wi-Fi passphrase".into(),
        Some(e) if e == "connect-failed" || e.contains("association") => {
            "couldn't associate with the network (signal or AP issue)".into()
        }
        Some("dhcp-failed") => "connected to the AP but DHCP failed (no IP address)".into(),
        _ => "connection failed — check signal and passphrase".into(),
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
    fn has_technology_detects_presence() {
        let only_eth = "/net/connman/technology/ethernet\n  Powered = True\n";
        assert!(lists_technology(only_eth, Tech::Ethernet));
        assert!(!lists_technology(only_eth, Tech::Wifi));
    }

    #[test]
    fn service_detail_parses_state_and_error() {
        let block = "  Type = wifi\n  State = failure\n  Error = invalid-key\n";
        let (state, error) = parse_service_detail(block);
        assert_eq!(state, LinkState::Failure);
        assert_eq!(error.as_deref(), Some("invalid-key"));
    }

    #[test]
    fn service_detail_no_error() {
        let (state, error) = parse_service_detail("  State = online\n");
        assert_eq!(state, LinkState::Online);
        assert_eq!(error, None);
    }

    #[test]
    fn failure_message_maps_known_errors() {
        assert!(failure_message(Some("invalid-key")).contains("passphrase"));
        assert!(failure_message(Some("connect-failed")).contains("associate"));
        assert!(failure_message(Some("dhcp-failed")).contains("DHCP"));
        assert!(!failure_message(None).is_empty());
    }

    #[test]
    fn provisioning_body() {
        let body = render_provisioning("wifi_x_managed_psk", "My Net", "secret");
        assert!(body.contains("[service_wifi_x_managed_psk]"));
        assert!(body.contains("Type = wifi"));
        assert!(body.contains("Name = My Net"));
        assert!(body.contains("Passphrase = secret"));
    }

    #[test]
    fn clear_provisioning_removes_file() {
        let dir = std::env::temp_dir().join("guix-install-clear-prov");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("guix-install.config");
        std::fs::write(&file, "x").unwrap();
        // SAFETY: single-threaded test, no other connman call races this.
        unsafe { std::env::set_var("GUIX_INSTALL_CONNMAN_DIR", &dir) };
        clear_provisioning();
        unsafe { std::env::remove_var("GUIX_INSTALL_CONNMAN_DIR") };
        assert!(!file.exists());
    }

    #[test]
    fn provisioning_strips_control_chars() {
        let body = render_provisioning("wifi_x\r_evil", "Net\nPassphrase = injected", "pw");
        assert!(!body.contains('\r'));
        assert_eq!(
            body.lines().filter(|l| l.starts_with("Passphrase")).count(),
            1
        );
    }
}
