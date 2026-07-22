use std::cell::{Cell, RefCell};
use std::fmt::Display;
use std::io::Write;

use anyhow::Result;
use console::Style;
use dialoguer::theme::ColorfulTheme;
use dialoguer::{Confirm, FuzzySelect, Input, Password, Select};

use guix_install_core::installer_log;
use guix_install_core::ui::{UserCancelled, UserInterface};

/// Above this many options, switch to a type-to-filter select.
const FUZZY_THRESHOLD: usize = 12;

#[derive(Default)]
pub struct Repl {
    theme: ColorfulTheme,
    /// True when a sticky footer line is currently drawn at the bottom.
    live_line: Cell<bool>,
    /// Current footer text (guix status + elapsed clock). Redrawn in place on
    /// each update and re-pinned under every scrolling line so it stays at the
    /// bottom instead of being committed into the scrollback.
    footer: RefCell<String>,
}

impl Repl {
    pub fn new() -> Self {
        Self::default()
    }

    /// Print `line` into the scroll region above the sticky footer: erase the
    /// footer, emit the line, then redraw the footer beneath it. Used for every
    /// scrolling output (info/warn/error and guix's complete lines).
    fn scroll_line(&self, line: impl Display) {
        if self.live_line.get() {
            print!("\r\x1b[K");
        }
        println!("{line}");
        self.redraw_footer();
    }

    /// (Re)draw the footer in place on the current line, or clear the live flag
    /// when there's nothing to show.
    fn redraw_footer(&self) {
        let footer = self.footer.borrow();
        if footer.is_empty() {
            self.live_line.set(false);
            return;
        }
        print!("\r\x1b[K{}", Style::new().dim().apply_to(&*footer));
        let _ = std::io::stdout().flush();
        self.live_line.set(true);
    }
}

fn map_dialoguer<T>(result: std::result::Result<T, dialoguer::Error>) -> Result<T> {
    result.map_err(|e| match e {
        dialoguer::Error::IO(io_err) if io_err.kind() == std::io::ErrorKind::Other => {
            anyhow::Error::new(UserCancelled)
        }
        other => other.into(),
    })
}

impl UserInterface for Repl {
    fn select(&mut self, prompt: &str, options: &[&str], default: usize) -> Result<usize> {
        let result = if options.len() > FUZZY_THRESHOLD {
            FuzzySelect::with_theme(&self.theme)
                .with_prompt(prompt)
                .items(options)
                .default(default)
                .interact_opt()?
        } else {
            Select::with_theme(&self.theme)
                .with_prompt(prompt)
                .items(options)
                .default(default)
                .interact_opt()?
        };
        match result {
            Some(idx) => Ok(idx),
            None => Err(anyhow::Error::new(UserCancelled)),
        }
    }

    fn input(&mut self, prompt: &str, default: &str) -> Result<String> {
        map_dialoguer(
            Input::with_theme(&self.theme)
                .with_prompt(prompt)
                .default(default.to_string())
                .interact_text(),
        )
    }

    fn password(&mut self, prompt: &str) -> Result<zeroize::Zeroizing<String>> {
        map_dialoguer(
            Password::with_theme(&self.theme)
                .with_prompt(prompt)
                .interact(),
        )
        .map(zeroize::Zeroizing::new)
    }

    fn confirm(&mut self, prompt: &str, default: bool) -> Result<bool> {
        map_dialoguer(
            Confirm::with_theme(&self.theme)
                .with_prompt(prompt)
                .default(default)
                .interact(),
        )
    }

    fn info(&self, msg: &str) {
        self.scroll_line(msg);
        installer_log::write_line("info:", msg);
    }

    fn warn(&self, msg: &str) {
        self.scroll_line(Style::new().yellow().apply_to(format!("warning: {msg}")));
        installer_log::write_line("warn:", msg);
    }

    fn error(&self, msg: &str) {
        self.scroll_line(Style::new().red().bold().apply_to(format!("error: {msg}")));
        installer_log::write_line("error:", msg);
    }

    fn guix_output(&self, line: &str, redraw: bool) {
        // Complete guix lines scroll above the footer (real context on failure).
        // Guix's own `\r` redraw lines are its in-place progress bars; the sticky
        // footer already is our progress line (with elapsed + byte counts), so we
        // drop them rather than fight over the bottom line.
        if redraw {
            return;
        }
        self.scroll_line(Style::new().dim().apply_to(line));
    }

    fn progress(&self, msg: &str, _pct: Option<f32>) {
        // The bar/percentage are GUI-only chrome. On the CLI the status line is a
        // footer pinned to the bottom: we cache it and redraw in place, so the
        // elapsed clock ticks and scrolling guix output re-pins it beneath.
        *self.footer.borrow_mut() = msg.to_string();
        self.redraw_footer();
    }
}
