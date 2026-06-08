use crate::config::SystemConfig;
use anyhow::Result;

/// Abstraction over user interaction.
///
/// The REPL implements this with dialoguer prompts. Future TUI/GUI
/// implementations can provide their own version without touching step logic.
///
/// Methods that collect input return `Err` with `UserCancelled` when the user
/// presses Escape, which steps should map to `StepResult::Back`.
pub trait UserInterface {
    fn select(&mut self, prompt: &str, options: &[&str], default: usize) -> Result<usize>;
    fn input(&mut self, prompt: &str, default: &str) -> Result<String>;
    fn password(&mut self, prompt: &str) -> Result<zeroize::Zeroizing<String>>;
    fn confirm(&mut self, prompt: &str, default: bool) -> Result<bool>;
    fn info(&self, msg: &str);
    fn warn(&self, msg: &str);
    fn error(&self, msg: &str);
    fn progress(&self, msg: &str, pct: Option<f32>);

    /// Renders the pre-install configuration summary.
    ///
    /// The default impl flattens `data` into `info`/`warn` lines — the CLI
    /// presentation. The GUI overrides this to send the structured data across
    /// the bridge and lay it out as grouped sections.
    fn summary(&self, data: &SummaryData) {
        self.info("");
        self.info("=== Installation Summary ===");
        if let Some(note) = &data.note {
            self.warn(note);
            self.info("");
        }
        for section in &data.sections {
            for row in &section.rows {
                self.info(&format!("{:<16}{}", format!("{}:", row.label), row.value));
            }
        }
        self.info("");
        self.info("Partition layout:");
        for line in &data.layout {
            self.info(&format!("  {line}"));
        }
        self.info("");
        self.warn(&data.warning);
        self.info("");
    }

    /// Edits a multi-line text blob, returning `Some(edited)` if saved or
    /// `None` if the user cancelled without changes.
    ///
    /// The default implementation writes `initial` to a temp file and opens it
    /// in `$EDITOR`/`$VISUAL`/nano/vi — the CLI behaviour. The GUI overrides
    /// this with an in-app editor since the install env has no TTY.
    fn edit_text(&mut self, _title: &str, initial: &str) -> Result<Option<String>> {
        const EDIT_PATH: &str = "/tmp/guix-install-edit.scm";
        use anyhow::Context;

        std::fs::write(EDIT_PATH, initial).with_context(|| format!("write {EDIT_PATH}"))?;

        if !launch_editor(self, EDIT_PATH) {
            let _ = std::fs::remove_file(EDIT_PATH);
            return Ok(None);
        }

        let edited =
            std::fs::read_to_string(EDIT_PATH).with_context(|| format!("read {EDIT_PATH}"))?;
        let _ = std::fs::remove_file(EDIT_PATH);
        Ok(Some(edited))
    }

    /// Reports the current per-mode step list and active index so a frontend
    /// can render a progress rail. No-op by default (the CLI ignores it).
    fn set_steps(&mut self, _steps: &[crate::steps::StepId], _current: usize) {}

    /// Marks the start (or resume-skip) of an install phase so a frontend can
    /// render the 8-phase checklist. No-op by default — the CLI relies on the
    /// `info`/`progress` lines instead.
    fn install_phase(&self, _num: u8, _total: u8, _label: &str) {}

    /// Structured snapshot of an in-flight guix op (pull / system init) for
    /// frontends that want per-download/per-build detail. No-op by default —
    /// the CLI relies on the flat `progress(msg, pct)` path; the GUI wires
    /// this up to its Install screen.
    fn guix_progress(&self, _summary: &libguix::progress::Summary) {}

    /// Apply the chosen keyboard layout to the live session.
    ///
    /// Called only when the layout actually changed. Default impl is the CLI
    /// behavior: no-op (the layout is already stored in `SystemConfig` for the
    /// target system). The GUI overrides this to persist interview state, write
    /// the keymap sentinel, and exit so cage relaunches in the new layout.
    fn apply_keyboard_layout(&mut self, _layout: &str, _config: &SystemConfig) -> Result<()> {
        Ok(())
    }
}

/// Launches an editor on `path`. Returns true on a successful spawn (regardless
/// of editor exit code), false if no editor could be spawned.
fn launch_editor<U: UserInterface + ?Sized>(ui: &U, path: &str) -> bool {
    let mut candidates: Vec<String> = Vec::new();
    candidates.extend(std::env::var("EDITOR").ok());
    candidates.extend(std::env::var("VISUAL").ok());
    candidates.push("nano".into());
    candidates.push("vi".into());

    for editor in &candidates {
        ui.info(&format!("Opening {path} in {editor}..."));
        if crate::exec::run_cmd_interactive(&[editor, path]).is_ok() {
            return true;
        }
    }
    ui.error(&format!(
        "No editor available (set $EDITOR or install nano/vi). Edit {path} manually."
    ));
    false
}

#[derive(Debug)]
pub struct UserCancelled;

impl std::fmt::Display for UserCancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "user cancelled")
    }
}

impl std::error::Error for UserCancelled {}

pub fn is_cancelled(e: &anyhow::Error) -> bool {
    e.downcast_ref::<UserCancelled>().is_some()
}

/// Structured pre-install summary: grouped sections plus the partition layout
/// and the destructive-action warning. Built once by the summary step, then
/// rendered per-UI (flat text on the CLI, grouped cards in the GUI).
#[derive(Debug, Clone)]
pub struct SummaryData {
    /// Caveat shown above the rows when a custom `system.scm` is in use.
    pub note: Option<String>,
    pub sections: Vec<SummarySection>,
    /// Partition-preview lines (already formatted, e.g. "sda1 200 MB EFI …").
    pub layout: Vec<String>,
    /// "/dev/sda will be formatted. ALL DATA WILL BE LOST."
    pub warning: String,
}

#[derive(Debug, Clone)]
pub struct SummarySection {
    pub title: String,
    pub rows: Vec<SummaryRow>,
}

#[derive(Debug, Clone)]
pub struct SummaryRow {
    pub label: String,
    pub value: String,
}

impl SummaryRow {
    pub fn new(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
        }
    }
}
