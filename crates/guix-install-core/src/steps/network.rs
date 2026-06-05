use anyhow::Result;

use crate::connman;
use crate::network;
use crate::steps::StepResult;
use crate::ui::UserInterface;
use crate::ui_or_back;

/// Auto-skip only on forward entry while actually reachable. A deliberate Back
/// always renders the step so the user can change networks.
pub fn should_autoskip(came_from_back: bool, reachable: bool) -> bool {
    !came_from_back && reachable
}

pub fn step_network(ui: &mut dyn UserInterface, came_from_back: bool) -> Result<StepResult> {
    let reachable = network::reachable();
    if should_autoskip(came_from_back, reachable) {
        return Ok(StepResult::Next);
    }

    let connected = match connman::state() {
        Ok(s) => s.is_connected(),
        Err(e) => {
            ui.warn(&format!("Could not query network state: {e}"));
            false
        }
    };
    if connected && reachable {
        ui.info("Network connected ✓");
        if ui_or_back!(ui.confirm("Connected. Continue? (No = change network)", true)) {
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
