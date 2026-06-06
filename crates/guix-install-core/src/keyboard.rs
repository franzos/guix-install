//! Keyboard-layout data for the installer's Keyboard step.
//!
//! The layout list is parsed from the xkb `evdev.lst` rules file on the ISO;
//! a small built-in list is the fallback. The live layout the GUI is currently
//! running under is recorded in a sentinel file (set before cage by launch-gui).

/// Applied layout code; written by the GUI before a relaunch, read by
/// panther's launch-gui (to set XKB_DEFAULT_LAYOUT) and by step_keyboard.
pub const KEYMAP_SENTINEL: &str = "/run/guix-install-keymap";
/// Marker dropped by the GUI to ask launch-gui to relaunch cage.
pub const RELAUNCH_MARKER: &str = "/run/guix-install-relaunch";

const FALLBACK: &[(&str, &str)] = &[
    ("us", "English (US)"),
    ("gb", "English (UK)"),
    ("de", "German"),
    ("fr", "French"),
    ("es", "Spanish"),
    ("pt", "Portuguese"),
    ("it", "Italian"),
    ("nl", "Dutch"),
    ("se", "Swedish"),
    ("no", "Norwegian"),
    ("dk", "Danish"),
    ("fi", "Finnish"),
    ("pl", "Polish"),
    ("cz", "Czech"),
    ("ch", "Swiss"),
    ("br", "Portuguese (Brazil)"),
    ("ru", "Russian"),
];

const XKB_LST_CANDIDATES: &[&str] = &[
    "/run/current-system/profile/share/X11/xkb/rules/evdev.lst",
    "/usr/share/X11/xkb/rules/evdev.lst",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub code: String,
    pub description: String,
}

/// Parse the `! layout` section of an xkb `evdev.lst`.
///
/// Format: a `! layout` header line, then indented `code  description` rows,
/// terminated by the next `!` section header.
pub fn parse_layouts(lst: &str) -> Vec<Layout> {
    let mut out = Vec::new();
    let mut in_section = false;
    for line in lst.lines() {
        if line.starts_with('!') {
            in_section = line.trim_start_matches('!').trim() == "layout";
            continue;
        }
        if !in_section {
            continue;
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some((code, desc)) = line.split_once(char::is_whitespace) {
            out.push(Layout {
                code: code.to_string(),
                description: desc.trim().to_string(),
            });
        }
    }
    out
}

/// The selectable layout list: parsed from evdev.lst, or the built-in fallback.
pub fn layouts() -> Vec<Layout> {
    let path = std::env::var("GUIX_INSTALL_XKB_LST").ok().or_else(|| {
        XKB_LST_CANDIDATES
            .iter()
            .find(|p| std::path::Path::new(p).exists())
            .map(|p| p.to_string())
    });
    if let Some(p) = path
        && let Ok(data) = std::fs::read_to_string(&p)
    {
        let parsed = parse_layouts(&data);
        if !parsed.is_empty() {
            return parsed;
        }
    }
    FALLBACK
        .iter()
        .map(|(c, d)| Layout {
            code: c.to_string(),
            description: d.to_string(),
        })
        .collect()
}

/// The layout the GUI is currently running under (from the sentinel; default "us").
pub fn current_live_layout() -> String {
    std::fs::read_to_string(KEYMAP_SENTINEL)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "us".to_string())
}
