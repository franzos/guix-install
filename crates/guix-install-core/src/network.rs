//! Shared network-connect orchestration over the connman module.

use std::time::Duration;

use crate::connman::Service;

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
    labels.push("Ethernet (plug in cable)".to_string());
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connman::Tech;

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
        assert_eq!(resolve_selection(2, 2), Selection::Action(NetworkAction::Rescan));
        assert_eq!(resolve_selection(3, 2), Selection::Action(NetworkAction::Ethernet));
        assert_eq!(resolve_selection(4, 2), Selection::Action(NetworkAction::Skip));
    }
}
