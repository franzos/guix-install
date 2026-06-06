use anyhow::Result;

use crate::connman;
use crate::network;
use crate::steps::StepResult;
use crate::ui::{UserInterface, is_cancelled};

/// Auto-skip only on forward entry while actually reachable. A deliberate Back
/// always renders the step so the user can change networks.
pub fn should_autoskip(came_from_back: bool, reachable: bool) -> bool {
    !came_from_back && reachable
}

pub fn step_network(ui: &mut dyn UserInterface, came_from_back: bool) -> Result<StepResult> {
    ui.info("Checking network connection…");
    let reachable = network::reachable();
    if should_autoskip(came_from_back, reachable) {
        return Ok(StepResult::Next);
    }

    // Online if substitutes are reachable OR connman reports a connected service.
    // The single reachable() probe can flake, so don't dead-end an online user.
    let online = reachable || connman::state().map(|s| s.is_connected()).unwrap_or(false);

    if online {
        ui.info("Network connected ✓");
        let choice = match ui.select("Network", &["Continue", "Change network…"], 0) {
            Ok(i) => i,
            Err(e) if is_cancelled(&e) => return Ok(StepResult::Back),
            Err(e) => return Err(e),
        };
        if choice == 0 {
            return Ok(StepResult::Next);
        }
    }

    network::connect_flow(ui)?;
    Ok(StepResult::Next)
}

#[cfg(test)]
mod tests {
    use super::should_autoskip;

    #[test]
    fn autoskips_forward_when_reachable() {
        assert!(should_autoskip(false, true));
    }

    #[test]
    fn renders_when_entered_via_back() {
        assert!(!should_autoskip(true, true));
    }

    #[test]
    fn renders_when_unreachable() {
        assert!(!should_autoskip(false, false));
    }
}
