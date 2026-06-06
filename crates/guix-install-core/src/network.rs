//! Shared network-connect orchestration over the connman module.

use std::time::Duration;

use anyhow::Result;

use crate::connman::{self, Service, Tech};
use crate::ui::{UserInterface, is_cancelled};

const SUBSTITUTE_PROBE_URL: &str = "https://bordeaux.guix.gnu.org/nix-cache-info";

/// True if a Guix substitute server is actually reachable. This — not raw
/// `connmanctl state` — is the "are we online" test, so captive portals and
/// `ready`-without-internet do not pass. Mirrors connman.scm's
/// `check-substitute-availability`.
pub fn reachable() -> bool {
    ureq::get(SUBSTITUTE_PROBE_URL)
        .timeout(Duration::from_secs(5))
        .call()
        .is_ok()
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
    let mut labels: Vec<String> = wifi
        .iter()
        .map(|s| {
            let lock = if s.secured { "🔒 " } else { "   " };
            if s.eap {
                format!("{lock}{} (enterprise — use Ethernet)", s.name)
            } else {
                format!("{lock}{}", s.name)
            }
        })
        .collect();
    labels.push("⟳ Rescan".to_string());
    labels.push("Check wired connection".to_string());
    labels.push("Continue without network".to_string());
    labels
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
pub fn connect_flow(ui: &mut dyn UserInterface) -> Result<()> {
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

        let labels = build_menu(&wifi);
        let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();

        // Cancel on the top-level menu == "Continue without network" prompt.
        let index = match ui.select("Connect to a network", &label_refs, 0) {
            Ok(i) => i,
            Err(e) if is_cancelled(&e) => {
                match ui.confirm(
                    "Continue without a network? guix pull/init will likely fail.",
                    false,
                ) {
                    Ok(true) => return Ok(()),
                    Ok(false) => continue,
                    Err(e) if is_cancelled(&e) => continue,
                    Err(e) => return Err(e),
                }
            }
            Err(e) => return Err(e),
        };

        match resolve_selection(index, wifi.len()) {
            Selection::Action(NetworkAction::Rescan) => continue,
            Selection::Action(NetworkAction::Ethernet) => {
                ui.info("Checking wired connection…");
                if reachable() {
                    ui.info("Network connected ✓");
                    return Ok(());
                }
                ui.warn(
                    "No connection detected. Plug in the Ethernet cable, then choose ⟳ Rescan.",
                );
                continue;
            }
            Selection::Action(NetworkAction::Skip) => {
                match ui.confirm(
                    "Continue without a network? guix pull/init will likely fail.",
                    false,
                ) {
                    Ok(true) => return Ok(()),
                    Ok(false) => continue,
                    Err(e) if is_cancelled(&e) => continue,
                    Err(e) => return Err(e),
                }
            }
            Selection::Network(i) => {
                let svc = &wifi[i];
                if svc.eap {
                    ui.warn("Enterprise (802.1x) Wi-Fi isn't supported here — use Ethernet or connect from a shell.");
                    continue;
                }
                let pw = if svc.secured {
                    match ui.password("Wi-Fi passphrase") {
                        Ok(p) => Some(p),
                        Err(e) if is_cancelled(&e) => continue,
                        Err(e) => return Err(e),
                    }
                } else {
                    None
                };
                ui.info(&format!("Connecting to {}…", svc.name));
                match connman::connect(&svc.path, &svc.name, pw.as_ref()) {
                    Ok(()) => return Ok(()),
                    Err(e) => ui.warn(&format!("Couldn't connect: {e}")),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn svc(name: &str, secured: bool, eap: bool) -> Service {
        Service {
            path: format!("wifi_{name}"),
            name: name.into(),
            tech: Tech::Wifi,
            connected: false,
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
