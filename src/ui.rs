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
    fn password(&mut self, prompt: &str) -> Result<String>;
    fn confirm(&mut self, prompt: &str, default: bool) -> Result<bool>;
    fn info(&self, msg: &str);
    fn warn(&self, msg: &str);
    fn error(&self, msg: &str);
    fn progress(&self, msg: &str, pct: Option<f32>);
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
