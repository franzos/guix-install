//! Shared network-connect orchestration over the connman module.

use std::time::{Duration, Instant};

use anyhow::Result;

use crate::connman::{self, Service, Tech};
use crate::mode::InstallMode;
use crate::ui::{UserInterface, is_cancelled};

const SUBSTITUTE_PROBE_URL: &str = "https://bordeaux.guix.gnu.org/nix-cache-info";
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const REACHABLE_BUDGET: Duration = Duration::from_secs(20);
const REACHABLE_BACKOFF: Duration = Duration::from_millis(1500);

fn probe(url: &str) -> bool {
    ureq::get(url).timeout(PROBE_TIMEOUT).call().is_ok()
}

/// True if any of the mode's substitute servers is reachable, with a bordeaux fallback.
pub fn reachable(mode: &InstallMode) -> bool {
    mode.substitute_urls()
        .iter()
        .any(|base| probe(&format!("{base}/nix-cache-info")))
        || probe(SUBSTITUTE_PROBE_URL)
}

/// Patient variant of [`reachable`]: a freshly-connected link reports `ready`
/// before the route/ARP settle, so a single probe can miss. Retry with backoff
/// until reachable or the budget is spent.
fn wait_reachable(mode: &InstallMode, budget: Duration) -> bool {
    let deadline = Instant::now() + budget;
    loop {
        if reachable(mode) {
            return true;
        }
        if Instant::now() + REACHABLE_BACKOFF >= deadline {
            return false;
        }
        std::thread::sleep(REACHABLE_BACKOFF);
    }
}

/// Non-network action rows appended to the network list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkAction {
    Rescan,
    Ethernet,
    Skip,
}

/// What the user picked from the network menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    Network(usize), // index into the wifi slice passed to build_menu
    Action(NetworkAction),
}

/// Build the display labels for the select menu: wifi networks first (EAP rows
/// marked unsupported), then the action rows. Order matches resolve_selection.
pub fn build_menu(wifi: &[Service]) -> Vec<String> {
    let mut labels: Vec<String> = wifi.iter().map(network_label).collect();
    labels.push("⟳ Rescan".to_string());
    labels.push("Check wired connection".to_string());
    labels.push("Continue without network".to_string());
    labels
}

/// Display label for a single Wi-Fi service (lock icon, EAP note, connected mark).
fn network_label(s: &Service) -> String {
    let lock = if s.secured { "🔒 " } else { "   " };
    let mut label = if s.eap {
        format!("{lock}{} (enterprise — use Ethernet)", s.name)
    } else {
        format!("{lock}{}", s.name)
    };
    if s.connected {
        label.push_str(" (connected)");
    }
    label
}

/// Collapse services to one representative per SSID (`Service.name`), preserving
/// first-seen order. When several services share a name, a connected one wins;
/// otherwise the first seen. Two adapters on the same network thus yield one row,
/// marked connected if either is connected.
pub fn dedup_by_ssid(services: &[Service]) -> Vec<Service> {
    let mut reps: Vec<Service> = Vec::new();
    for s in services {
        match reps.iter_mut().find(|r| r.name == s.name) {
            Some(existing) => {
                if s.connected && !existing.connected {
                    *existing = s.clone();
                }
            }
            None => reps.push(s.clone()),
        }
    }
    reps
}

/// Map a selected menu index back to a Selection, given the wifi count.
pub fn resolve_selection(index: usize, wifi_count: usize) -> Selection {
    if index < wifi_count {
        Selection::Network(index)
    } else {
        match index - wifi_count {
            0 => Selection::Action(NetworkAction::Rescan),
            1 => Selection::Action(NetworkAction::Ethernet),
            _ => Selection::Action(NetworkAction::Skip),
        }
    }
}

/// Interactive connect loop: enable Wi-Fi, scan, list, connect. Returns when the
/// user is connected or explicitly continues without a network.
pub fn connect_flow(ui: &mut dyn UserInterface, mode: &InstallMode) -> Result<()> {
    loop {
        // Assume present on probe error so a transient connmanctl hiccup doesn't
        // hide the Wi-Fi path entirely.
        let has_wifi = connman::has_technology(Tech::Wifi).unwrap_or(true);

        let wifi: Vec<Service> = if has_wifi {
            ui.info("Enabling Wi-Fi…");
            if let Err(e) = connman::enable(Tech::Wifi) {
                ui.warn(&format!("{e}"));
            }
            ui.info("Scanning for Wi-Fi networks…");
            if let Err(e) = connman::scan(Tech::Wifi) {
                ui.warn(&format!("Wi-Fi scan failed: {e}"));
            }
            let wifi: Vec<Service> = connman::services()
                .unwrap_or_default()
                .into_iter()
                .filter(|s| s.tech == Tech::Wifi)
                .collect();
            if wifi.is_empty() {
                ui.warn(
                    "No Wi-Fi networks found — check the router is on and in range, then ⟳ Rescan.",
                );
            }
            wifi
        } else {
            ui.warn(
                "No Wi-Fi adapter detected (or its driver isn't loaded). Use a wired connection.",
            );
            Vec::new()
        };

        let wifi = dedup_by_ssid(&wifi);
        let labels = build_menu(&wifi);
        let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();

        let index = match ui.select("Connect to a network", &label_refs, 0) {
            Ok(i) => i,
            Err(e) if is_cancelled(&e) => {
                if confirm_skip(ui)? {
                    return Ok(());
                }
                continue;
            }
            Err(e) => return Err(e),
        };

        let done = match resolve_selection(index, wifi.len()) {
            Selection::Action(action) => handle_action(ui, action, mode)?,
            Selection::Network(i) => connect_to(ui, &wifi[i], mode)?,
        };
        if done {
            return Ok(());
        }
    }
}

/// Handle a Rescan/Ethernet/Skip action row. `Ok(true)` ends the flow.
fn handle_action(
    ui: &mut dyn UserInterface,
    action: NetworkAction,
    mode: &InstallMode,
) -> Result<bool> {
    match action {
        NetworkAction::Rescan => Ok(false),
        NetworkAction::Ethernet => {
            ui.info("Checking wired connection…");
            if reachable(mode) {
                ui.info("Network connected ✓");
                return Ok(true);
            }
            ui.warn("No connection detected. Plug in the Ethernet cable, then choose ⟳ Rescan.");
            Ok(false)
        }
        NetworkAction::Skip => confirm_skip(ui),
    }
}

/// "Continue without a network?" confirm. `Ok(true)` to proceed, `Ok(false)`
/// to fall back into the menu loop.
fn confirm_skip(ui: &mut dyn UserInterface) -> Result<bool> {
    match ui.confirm(
        "Continue without a network? guix pull/init will likely fail.",
        false,
    ) {
        Ok(v) => Ok(v),
        Err(e) if is_cancelled(&e) => Ok(false),
        Err(e) => Err(e),
    }
}

/// Prompt + dial a chosen network. If the representative is already connected,
/// skip the passphrase and dial and just verify reachability. `Ok(true)` when
/// connected, `Ok(false)` to return to the list.
fn connect_to(ui: &mut dyn UserInterface, svc: &Service, mode: &InstallMode) -> Result<bool> {
    if svc.eap {
        ui.warn("Enterprise (802.1x) Wi-Fi isn't supported here — use Ethernet or connect from a shell.");
        return Ok(false);
    }

    if svc.connected {
        ui.info("Verifying internet access…");
        if wait_reachable(mode, REACHABLE_BUDGET) {
            ui.info("Network connected ✓");
            return Ok(true);
        }
        ui.warn(
            "Connected to the network, but there's no internet yet (substitute server unreachable). Try again or pick a different network.",
        );
        return Ok(false);
    }

    let pw = if svc.secured {
        match ui.password("Wi-Fi passphrase") {
            Ok(p) => Some(p),
            Err(e) if is_cancelled(&e) => return Ok(false),
            Err(e) => return Err(e),
        }
    } else {
        None
    };
    ui.info(&format!("Connecting to {}…", svc.name));
    match connman::connect(&svc.path, &svc.name, pw.as_ref()) {
        Ok(()) => {
            ui.info("Verifying internet access…");
            if wait_reachable(mode, REACHABLE_BUDGET) {
                ui.info("Network connected ✓");
                return Ok(true);
            }
            ui.warn(
                "Connected to the network, but there's no internet yet (substitute server unreachable). Try again or pick a different network.",
            );
        }
        Err(e) => ui.warn(&format!("Couldn't connect: {e}")),
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn svc(name: &str, secured: bool, eap: bool) -> Service {
        svc_p(&format!("wifi_{name}"), name, secured, eap, false)
    }

    fn svc_p(path: &str, name: &str, secured: bool, eap: bool, connected: bool) -> Service {
        Service {
            path: path.into(),
            name: name.into(),
            tech: Tech::Wifi,
            connected,
            secured,
            eap,
        }
    }

    #[test]
    fn menu_lists_networks_then_actions() {
        let wifi = vec![svc("Home", true, false), svc("Cafe", false, false)];
        let labels = build_menu(&wifi);
        assert_eq!(labels.len(), 5);
        assert!(labels[0].contains("🔒"));
        assert!(labels[0].contains("Home"));
        assert!(!labels[1].contains("🔒"));
        assert!(labels[2].contains("Rescan"));
        assert!(labels[4].contains("without network"));
    }

    #[test]
    fn eap_label_marked_unsupported() {
        let wifi = vec![svc("Corp", true, true)];
        assert!(build_menu(&wifi)[0].contains("enterprise"));
    }

    #[test]
    fn connected_marker_only_on_connected() {
        let wifi = vec![
            svc_p("wifi_aa_x_managed_psk", "Home", true, false, true),
            svc("Cafe", false, false),
        ];
        let labels = build_menu(&wifi);
        assert!(labels[0].contains("(connected)"));
        assert!(labels[0].contains("Home"));
        assert!(!labels[1].contains("(connected)"));
    }

    #[test]
    fn dedup_collapses_same_ssid_prefers_connected() {
        let wifi = vec![
            svc_p("wifi_aaaa_x_managed_psk", "Home", true, false, false),
            svc_p("wifi_bbbb_x_managed_psk", "Home", true, false, true),
            svc_p("wifi_cccc_y_managed_none", "Cafe", false, false, false),
        ];
        let reps = dedup_by_ssid(&wifi);
        assert_eq!(reps.len(), 2);
        assert_eq!(reps[0].name, "Home");
        assert!(reps[0].connected);
        assert_eq!(reps[0].path, "wifi_bbbb_x_managed_psk");
        assert_eq!(reps[1].name, "Cafe");
        assert!(!reps[1].connected);

        let labels = build_menu(&reps);
        assert_eq!(labels.len(), 5);
        assert!(labels[0].contains("Home"));
        assert!(labels[0].contains("(connected)"));
        assert!(labels[1].contains("Cafe"));
        assert!(!labels[1].contains("(connected)"));
    }

    #[test]
    fn selection_resolution() {
        assert_eq!(resolve_selection(0, 2), Selection::Network(0));
        assert_eq!(resolve_selection(1, 2), Selection::Network(1));
        assert_eq!(
            resolve_selection(2, 2),
            Selection::Action(NetworkAction::Rescan)
        );
        assert_eq!(
            resolve_selection(3, 2),
            Selection::Action(NetworkAction::Ethernet)
        );
        assert_eq!(
            resolve_selection(4, 2),
            Selection::Action(NetworkAction::Skip)
        );
    }
}
