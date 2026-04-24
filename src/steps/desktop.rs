use anyhow::Result;

use crate::config::{DesktopEnvironment, SystemConfig};
use crate::steps::StepResult;
use crate::ui::UserInterface;
use crate::ui_or_back;

const DESKTOP_OPTIONS: &[&str] = &[
    "None (headless)",
    "GNOME",
    "KDE Plasma",
    "Xfce",
    "MATE",
    "Sway",
    "i3",
    "LXQt",
];

pub fn step_desktop(ui: &mut dyn UserInterface, config: &mut SystemConfig) -> Result<StepResult> {
    let default = match &config.desktop {
        None => 0,
        Some(DesktopEnvironment::Gnome) => 1,
        Some(DesktopEnvironment::Kde) => 2,
        Some(DesktopEnvironment::Xfce) => 3,
        Some(DesktopEnvironment::Mate) => 4,
        Some(DesktopEnvironment::Sway) => 5,
        Some(DesktopEnvironment::I3) => 6,
        Some(DesktopEnvironment::Lxqt) => 7,
    };

    let choice = ui_or_back!(ui.select("Desktop environment", DESKTOP_OPTIONS, default));

    config.desktop = match choice {
        1 => Some(DesktopEnvironment::Gnome),
        2 => Some(DesktopEnvironment::Kde),
        3 => Some(DesktopEnvironment::Xfce),
        4 => Some(DesktopEnvironment::Mate),
        5 => Some(DesktopEnvironment::Sway),
        6 => Some(DesktopEnvironment::I3),
        7 => Some(DesktopEnvironment::Lxqt),
        _ => None,
    };

    Ok(StepResult::Next)
}
