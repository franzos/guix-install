//! Bridges the blocking [`UserInterface`] trait to iced's async event loop.
//!
//! The step loop runs on a worker thread and calls blocking trait methods.
//! Each blocking call pushes a [`UiEvent`] to the GUI over an async channel,
//! then waits on a sync channel for the user's [`PromptResponse`]. Non-blocking
//! methods (info/warn/error/progress/set_steps) are fire-and-forget.

use std::sync::mpsc::Receiver as SyncReceiver;

use guix_install_core::steps::StepId;
use guix_install_core::ui::{UserCancelled, UserInterface};
use iced::futures::channel::mpsc::UnboundedSender;

/// A blocking prompt the GUI must render and answer.
#[derive(Debug, Clone)]
pub enum PromptRequest {
    Select {
        prompt: String,
        options: Vec<String>,
        default: usize,
    },
    Input {
        prompt: String,
        default: String,
    },
    Password {
        prompt: String,
    },
    Confirm {
        prompt: String,
        default: bool,
    },
    /// A multi-line text edit (the in-app `system.scm` editor).
    Edit {
        title: String,
        initial: String,
    },
}

/// The user's answer to a [`PromptRequest`], sent back to the worker.
#[derive(Debug, Clone)]
pub enum PromptResponse {
    Index(usize),
    Text(String),
    Bool(bool),
    /// Result of an [`PromptRequest::Edit`]: `Some` when saved, `None` when the
    /// user closed the editor without keeping changes.
    Edited(Option<String>),
    Cancelled,
}

/// One rail entry: label + state relative to the current step.
#[derive(Debug, Clone)]
pub struct RailEntry {
    pub label: String,
    pub done: bool,
    pub current: bool,
}

/// Everything the worker pushes to the GUI thread.
#[derive(Debug, Clone)]
pub enum UiEvent {
    Prompt(PromptRequest),
    Rail {
        entries: Vec<RailEntry>,
        current: StepId,
    },
    Info(String),
    Warn(String),
    Error(String),
    Progress {
        msg: String,
        pct: Option<f32>,
    },
    /// An install phase started (or was skipped on resume). `num` is 1-based
    /// over the fixed 8-phase pipeline.
    Phase {
        num: u8,
        label: String,
    },
    /// Compact live detail line from an in-flight guix op (pull / system init).
    GuixDetail(String),
    /// A phase failed: the run loop returned `Err`. Carries a one-line summary
    /// plus the full error chain for the failure screen.
    Failed {
        summary: String,
        detail: String,
    },
    /// The worker's run loop returned; the install/interview is finished.
    Finished,
}

/// Worker-side [`UserInterface`]. Holds the async sender to the GUI and the
/// sync receiver it blocks on for each prompt answer.
pub struct IcedUi {
    to_gui: UnboundedSender<UiEvent>,
    from_gui: SyncReceiver<PromptResponse>,
}

impl IcedUi {
    pub fn new(to_gui: UnboundedSender<UiEvent>, from_gui: SyncReceiver<PromptResponse>) -> Self {
        Self { to_gui, from_gui }
    }

    fn send(&self, event: UiEvent) {
        let _ = self.to_gui.unbounded_send(event);
    }

    /// Push a prompt and block until the GUI answers (or the channel closes).
    fn ask(&self, prompt: PromptRequest) -> PromptResponse {
        self.send(UiEvent::Prompt(prompt));
        self.from_gui.recv().unwrap_or(PromptResponse::Cancelled)
    }
}

fn cancelled() -> anyhow::Error {
    anyhow::Error::new(UserCancelled)
}

/// Folds a guix [`Summary`] into a single compact detail line for the Install
/// screen: the active stage plus the most relevant in-flight item (a download
/// with byte counts, or the current build), and running tallies.
fn summarize(s: &libguix::progress::Summary) -> String {
    use libguix::progress::{BuildStatus, Stage};

    let stage = match s.stage {
        Stage::Starting => "starting",
        Stage::ChannelUpdate => "updating channels",
        Stage::ComputingDeriv => "computing derivation",
        Stage::Downloading => "downloading",
        Stage::Building => "building",
        Stage::Profile => "finalizing profile",
        Stage::Done => "done",
        Stage::Failed => "failed",
    };

    // Prefer an in-flight download (with bytes), else the current running build.
    let active = s
        .downloads
        .values()
        .rev()
        .find(|d| !d.done)
        .map(|d| match d.bytes_total {
            Some(total) if total > 0 => format!(
                "{} ({}/{})",
                d.pretty_name,
                human_bytes(d.bytes_done),
                human_bytes(total)
            ),
            _ => format!("{} ({})", d.pretty_name, human_bytes(d.bytes_done)),
        })
        .or_else(|| {
            s.builds
                .values()
                .rev()
                .find(|b| b.status == BuildStatus::Running)
                .map(|b| format!("building {}", b.pretty_name))
        });

    let tally = format!(
        "{} built · {} downloaded · {} total",
        s.build_count_done,
        s.download_count_done,
        human_bytes(s.bytes_downloaded)
    );

    match active {
        Some(item) => format!("{stage}: {item} — {tally}"),
        None => format!("{stage} — {tally}"),
    }
}

fn human_bytes(n: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

impl UserInterface for IcedUi {
    fn select(&mut self, prompt: &str, options: &[&str], default: usize) -> anyhow::Result<usize> {
        let req = PromptRequest::Select {
            prompt: prompt.to_string(),
            options: options.iter().map(|s| s.to_string()).collect(),
            default,
        };
        match self.ask(req) {
            PromptResponse::Index(i) => Ok(i),
            _ => Err(cancelled()),
        }
    }

    fn input(&mut self, prompt: &str, default: &str) -> anyhow::Result<String> {
        let req = PromptRequest::Input {
            prompt: prompt.to_string(),
            default: default.to_string(),
        };
        match self.ask(req) {
            PromptResponse::Text(t) => Ok(t),
            _ => Err(cancelled()),
        }
    }

    fn password(&mut self, prompt: &str) -> anyhow::Result<zeroize::Zeroizing<String>> {
        let req = PromptRequest::Password {
            prompt: prompt.to_string(),
        };
        match self.ask(req) {
            PromptResponse::Text(t) => Ok(zeroize::Zeroizing::new(t)),
            _ => Err(cancelled()),
        }
    }

    fn confirm(&mut self, prompt: &str, default: bool) -> anyhow::Result<bool> {
        let req = PromptRequest::Confirm {
            prompt: prompt.to_string(),
            default,
        };
        match self.ask(req) {
            PromptResponse::Bool(b) => Ok(b),
            _ => Err(cancelled()),
        }
    }

    fn edit_text(&mut self, title: &str, initial: &str) -> anyhow::Result<Option<String>> {
        let req = PromptRequest::Edit {
            title: title.to_string(),
            initial: initial.to_string(),
        };
        match self.ask(req) {
            PromptResponse::Edited(result) => Ok(result),
            _ => Err(cancelled()),
        }
    }

    fn info(&self, msg: &str) {
        self.send(UiEvent::Info(msg.to_string()));
    }

    fn warn(&self, msg: &str) {
        self.send(UiEvent::Warn(msg.to_string()));
    }

    fn error(&self, msg: &str) {
        self.send(UiEvent::Error(msg.to_string()));
    }

    fn progress(&self, msg: &str, pct: Option<f32>) {
        self.send(UiEvent::Progress {
            msg: msg.to_string(),
            pct,
        });
    }

    fn install_phase(&self, num: u8, _total: u8, label: &str) {
        self.send(UiEvent::Phase {
            num,
            label: label.to_string(),
        });
    }

    fn guix_progress(&self, summary: &libguix::progress::Summary) {
        self.send(UiEvent::GuixDetail(summarize(summary)));
    }

    fn set_steps(&mut self, steps: &[StepId], current: usize) {
        let entries = steps
            .iter()
            .enumerate()
            .map(|(i, s)| RailEntry {
                label: s.label().to_string(),
                done: i < current,
                current: i == current,
            })
            .collect();
        let current_step = steps.get(current).copied().unwrap_or(StepId::Mode);
        self.send(UiEvent::Rail {
            entries,
            current: current_step,
        });
    }
}
