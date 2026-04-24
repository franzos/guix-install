use anyhow::Result;
use console::Style;
use dialoguer::theme::ColorfulTheme;
use dialoguer::{Confirm, Input, Password, Select};

use crate::ui::{UserCancelled, UserInterface};

#[derive(Default)]
pub struct Repl {
    theme: ColorfulTheme,
}

impl Repl {
    pub fn new() -> Self {
        Self::default()
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
        let result = Select::with_theme(&self.theme)
            .with_prompt(prompt)
            .items(options)
            .default(default)
            .interact_opt()?;
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

    fn password(&mut self, prompt: &str) -> Result<String> {
        map_dialoguer(
            Password::with_theme(&self.theme)
                .with_prompt(prompt)
                .interact(),
        )
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
        println!("{msg}");
    }

    fn warn(&self, msg: &str) {
        let style = Style::new().yellow();
        eprintln!("{}", style.apply_to(format!("warning: {msg}")));
    }

    fn error(&self, msg: &str) {
        let style = Style::new().red().bold();
        eprintln!("{}", style.apply_to(format!("error: {msg}")));
    }

    fn progress(&self, msg: &str, pct: Option<f32>) {
        let style = Style::new().dim();
        match pct {
            Some(p) => println!("{}", style.apply_to(format!("[{:.0}%] {msg}", p * 100.0))),
            None => println!("{}", style.apply_to(format!("... {msg}"))),
        }
    }
}
