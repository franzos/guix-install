use anyhow::{Result, bail};

use guix_install_core::ui::UserInterface;

/// Non-prompting [`UserInterface`] for unattended installs.
///
/// The 8-phase install pipeline only ever calls the reporting methods
/// (`info`/`warn`/`error`/`progress`/`guix_output`); it reads the password and
/// LUKS passphrase straight off `SystemConfig`. The input methods exist only to
/// satisfy the trait — if one is reached it means a prompt slipped into the
/// install path, so they fail loudly rather than pick a silent default.
pub struct Headless;

impl UserInterface for Headless {
    fn select(&mut self, prompt: &str, _options: &[&str], _default: usize) -> Result<usize> {
        bail!("unattended install cannot answer prompt: {prompt}");
    }

    fn input(&mut self, prompt: &str, _default: &str) -> Result<String> {
        bail!("unattended install cannot answer prompt: {prompt}");
    }

    fn password(&mut self, prompt: &str) -> Result<zeroize::Zeroizing<String>> {
        bail!("unattended install cannot answer prompt: {prompt}");
    }

    fn confirm(&mut self, prompt: &str, _default: bool) -> Result<bool> {
        bail!("unattended install cannot answer prompt: {prompt}");
    }

    fn info(&self, msg: &str) {
        println!("{msg}");
    }

    fn warn(&self, msg: &str) {
        eprintln!("warning: {msg}");
    }

    fn error(&self, msg: &str) {
        eprintln!("error: {msg}");
    }

    fn progress(&self, msg: &str, pct: Option<f32>) {
        match pct {
            Some(p) => println!("[{:>3.0}%] {msg}", p * 100.0),
            None => println!("{msg}"),
        }
    }

    fn guix_output(&self, line: &str, _redraw: bool) {
        println!("{line}");
    }
}
