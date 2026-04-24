pub mod desktop;
pub mod disk;
pub mod encryption;
pub mod hostname;
pub mod locale;
pub mod mode;
pub mod summary;
pub mod timezone;
pub mod users;

use crate::mode::InstallMode;
use crate::ui::is_cancelled;

/// Run a UI operation, mapping Escape/cancel to `StepResult::Back`.
/// Returns `Ok(value)` on success, `Ok(StepResult::Back)` wrapper on cancel.
pub fn or_back<T>(result: anyhow::Result<T>) -> Result<Result<T, StepResult>, anyhow::Error> {
    match result {
        Ok(v) => Ok(Ok(v)),
        Err(e) if is_cancelled(&e) => Ok(Err(StepResult::Back)),
        Err(e) => Err(e),
    }
}

/// Macro-free helper: run a UI call, return Back on cancel.
#[macro_export]
macro_rules! ui_or_back {
    ($expr:expr) => {
        match $crate::steps::or_back($expr)? {
            Ok(v) => v,
            Err(back) => return Ok(back),
        }
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepId {
    Mode,
    Locale,
    Timezone,
    Hostname,
    Disk,
    Encryption,
    Users,
    Desktop,
    Summary,
}

pub enum StepResult {
    Next,
    Back,
    Quit,
}

pub struct StepNavigator {
    steps: Vec<StepId>,
    current: usize,
}

impl StepNavigator {
    /// Build the step list filtered by mode.
    ///
    /// Enterprise skips Locale, Timezone, Hostname, Users, and Desktop
    /// because those come from the remote config tarball.
    pub fn new(mode: &InstallMode) -> Self {
        Self {
            steps: steps_for_mode(mode),
            current: 0,
        }
    }

    pub fn current(&self) -> StepId {
        self.steps[self.current]
    }

    pub fn advance(&mut self) {
        if self.current < self.steps.len() - 1 {
            self.current += 1;
        }
    }

    pub fn go_back(&mut self) {
        if self.current > 0 {
            self.current -= 1;
        }
    }

    pub fn is_first(&self) -> bool {
        self.current == 0
    }

    pub fn is_last(&self) -> bool {
        self.current == self.steps.len() - 1
    }

    /// Rebuild the step list when the mode changes.
    ///
    /// Resets position to step 1 (the step right after Mode) so the user
    /// continues forward from the mode selection.
    pub fn reset_for_mode(&mut self, mode: &InstallMode) {
        self.steps = steps_for_mode(mode);
        // Stay on step index 1 (past Mode) since the user just completed Mode
        self.current = 1.min(self.steps.len() - 1);
    }

    pub fn steps(&self) -> &[StepId] {
        &self.steps
    }
}

fn steps_for_mode(mode: &InstallMode) -> Vec<StepId> {
    match mode {
        InstallMode::Enterprise { .. } => vec![
            StepId::Mode,
            StepId::Disk,
            StepId::Encryption,
            StepId::Summary,
        ],
        _ => vec![
            StepId::Mode,
            StepId::Locale,
            StepId::Timezone,
            StepId::Hostname,
            StepId::Disk,
            StepId::Encryption,
            StepId::Users,
            StepId::Desktop,
            StepId::Summary,
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panther_includes_all_steps() {
        let nav = StepNavigator::new(&InstallMode::Panther);
        assert_eq!(nav.steps().len(), 9);
        assert_eq!(nav.steps()[0], StepId::Mode);
        assert_eq!(nav.steps()[1], StepId::Locale);
        assert_eq!(nav.steps()[2], StepId::Timezone);
        assert_eq!(nav.steps()[3], StepId::Hostname);
        assert_eq!(nav.steps()[4], StepId::Disk);
        assert_eq!(nav.steps()[5], StepId::Encryption);
        assert_eq!(nav.steps()[6], StepId::Users);
        assert_eq!(nav.steps()[7], StepId::Desktop);
        assert_eq!(nav.steps()[8], StepId::Summary);
    }

    #[test]
    fn guix_includes_all_steps() {
        let nav = StepNavigator::new(&InstallMode::Guix);
        assert_eq!(nav.steps().len(), 9);
    }

    #[test]
    fn nonguix_includes_all_steps() {
        let nav = StepNavigator::new(&InstallMode::Nonguix);
        assert_eq!(nav.steps().len(), 9);
    }

    #[test]
    fn enterprise_skips_config_steps() {
        let mode = InstallMode::Enterprise {
            config_id: "test".into(),
            config_url: "https://example.com".into(),
        };
        let nav = StepNavigator::new(&mode);
        assert_eq!(nav.steps().len(), 4);
        assert_eq!(nav.steps()[0], StepId::Mode);
        assert_eq!(nav.steps()[1], StepId::Disk);
        assert_eq!(nav.steps()[2], StepId::Encryption);
        assert_eq!(nav.steps()[3], StepId::Summary);

        assert!(!nav.steps().contains(&StepId::Locale));
        assert!(!nav.steps().contains(&StepId::Timezone));
        assert!(!nav.steps().contains(&StepId::Hostname));
        assert!(!nav.steps().contains(&StepId::Users));
        assert!(!nav.steps().contains(&StepId::Desktop));
    }

    #[test]
    fn advance_and_go_back() {
        let mut nav = StepNavigator::new(&InstallMode::Panther);
        assert_eq!(nav.current(), StepId::Mode);

        nav.advance();
        assert_eq!(nav.current(), StepId::Locale);

        nav.advance();
        assert_eq!(nav.current(), StepId::Timezone);

        nav.go_back();
        assert_eq!(nav.current(), StepId::Locale);

        nav.go_back();
        assert_eq!(nav.current(), StepId::Mode);
    }

    #[test]
    fn go_back_at_first_stays() {
        let mut nav = StepNavigator::new(&InstallMode::Panther);
        assert!(nav.is_first());
        nav.go_back();
        assert!(nav.is_first());
        assert_eq!(nav.current(), StepId::Mode);
    }

    #[test]
    fn advance_at_last_stays() {
        let mut nav = StepNavigator::new(&InstallMode::Panther);
        for _ in 0..20 {
            nav.advance();
        }
        assert!(nav.is_last());
        assert_eq!(nav.current(), StepId::Summary);
        nav.advance();
        assert!(nav.is_last());
        assert_eq!(nav.current(), StepId::Summary);
    }

    #[test]
    fn is_first_and_is_last() {
        let mut nav = StepNavigator::new(&InstallMode::Panther);
        assert!(nav.is_first());
        assert!(!nav.is_last());

        for _ in 0..8 {
            nav.advance();
        }
        assert!(!nav.is_first());
        assert!(nav.is_last());
    }

    #[test]
    fn reset_for_mode_rebuilds() {
        let mut nav = StepNavigator::new(&InstallMode::Panther);
        assert_eq!(nav.steps().len(), 9);

        nav.advance();
        nav.advance();
        assert_eq!(nav.current(), StepId::Timezone);

        let enterprise = InstallMode::Enterprise {
            config_id: "abc".into(),
            config_url: "https://example.com".into(),
        };
        nav.reset_for_mode(&enterprise);
        assert_eq!(nav.steps().len(), 4);
        assert_eq!(nav.current(), StepId::Disk);
    }

    #[test]
    fn reset_for_mode_back_to_full() {
        let enterprise = InstallMode::Enterprise {
            config_id: "abc".into(),
            config_url: "https://example.com".into(),
        };
        let mut nav = StepNavigator::new(&enterprise);
        assert_eq!(nav.steps().len(), 4);

        nav.reset_for_mode(&InstallMode::Panther);
        assert_eq!(nav.steps().len(), 9);
        assert_eq!(nav.current(), StepId::Locale);
    }
}
