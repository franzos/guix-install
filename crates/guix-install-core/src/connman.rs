//! Wrapper over `connmanctl` — mirrors gnu/installer/connman.scm.
//! Every invocation forces `LANG=C LC_ALL=C` so parsing is locale-stable.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tech {
    Wifi,
    Ethernet,
}

impl Tech {
    #[allow(dead_code)]
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
