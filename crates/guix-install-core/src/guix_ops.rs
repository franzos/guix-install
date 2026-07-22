//! Drives the guix-heavy phases (`pull`, `system init`) through `libguix`,
//! folding its `ProgressEvent` stream into a [`libguix::progress::Summary`] and
//! reporting one unified `progress(status, pct)` update per batch — the same
//! monotonic milestone percentage (via [`ProgressModel`]) and status line the
//! CLI and GUI both render.
//!
//! Non-guix shell-outs stay in `exec.rs`; this is the only `libguix` entry
//! point. A current-thread tokio runtime is built per call and the async event
//! stream is run to completion via `block_on` — the phase fns are synchronous
//! and run on the install thread, so blocking here is correct.

use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use futures_util::StreamExt;
use libguix::progress::{BuildStatus, Failure, Summary};
use libguix::{BuildOptions, Guix, InitOptions, Privilege, SystemPullOptions};

use crate::progress::{self, Phase, ProgressModel};
use crate::ui::UserInterface;

/// Builds a current-thread runtime. One per guix phase keeps the install thread
/// synchronous; the runtime is dropped when the phase returns.
fn runtime() -> Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("failed to build tokio runtime for guix op")
}

/// Renders a `Failure` (or bare exit) into a one-line context string.
fn failure_message(summary: &Summary, op: &str) -> String {
    match &summary.failure {
        Some(Failure::Build { name, log_path }) => match log_path {
            Some(p) => format!("{op}: build failed: {name} (log: {p})"),
            None => format!("{op}: build failed: {name}"),
        },
        Some(Failure::Exit { code }) => format!("{op} exited with code {code}"),
        None => format!("{op} failed"),
    }
}

/// Runs `guix pull --channels=<channels>` as root, streaming progress.
///
/// `channels` is the generated `channels.scm`; `substitute_urls` are the
/// mode's authorized servers. Reports overall weighted progress for
/// [`Phase::Pull`].
pub fn run_pull(
    channels: &Path,
    substitute_urls: Vec<String>,
    ui: &dyn UserInterface,
    model: &ProgressModel,
) -> Result<()> {
    let rt = runtime()?;
    rt.block_on(async move {
        let guix = Guix::discover()
            .await
            .context("could not locate the guix binary")?;

        let opts = SystemPullOptions {
            channels: Some(channels.to_path_buf()),
            build: BuildOptions {
                substitute_urls,
                ..Default::default()
            },
            privilege: Privilege::AlreadyRoot,
            ..Default::default()
        };

        let op = guix
            .pull()
            .as_root(opts)
            .map_err(|e| anyhow!("failed to start guix pull: {e}"))?;

        drive(op, Phase::Pull, "guix pull", ui, model).await
    })
}

/// Runs `guix system init <config> <target>` as root, streaming progress.
///
/// `substitute_urls` are the mode's authorized servers. Reports overall
/// weighted progress for [`Phase::Install`].
pub fn run_system_init(
    config_scm: &Path,
    target: &Path,
    substitute_urls: Vec<String>,
    ui: &dyn UserInterface,
    model: &ProgressModel,
) -> Result<()> {
    let rt = runtime()?;
    rt.block_on(async move {
        let guix = Guix::discover()
            .await
            .context("could not locate the guix binary")?;

        let opts = InitOptions {
            build: BuildOptions {
                substitute_urls,
                fallback: true,
                ..Default::default()
            },
            privilege: Privilege::AlreadyRoot,
            ..Default::default()
        };

        let op = guix
            .system()
            .init(config_scm, target, opts)
            .map_err(|e| anyhow!("failed to start guix system init: {e}"))?;

        drive(op, Phase::Install, "guix system init", ui, model).await
    })
}

/// Emits one unified progress update: the monotonic milestone percentage and
/// the compact status line, rendered identically by the CLI and GUI.
fn emit_progress(model: &ProgressModel, phase: Phase, summary: &Summary, ui: &dyn UserInterface) {
    let overall = model.guix_update(phase, summary);
    let status = progress::summarize(summary);
    ui.progress(&status, Some(overall));
}

/// Consumes the op's event stream, folding batches into a `Summary` and
/// reporting progress. Bails with the captured failure on a non-zero exit.
async fn drive(
    mut op: libguix::Operation,
    phase: Phase,
    op_name: &str,
    ui: &dyn UserInterface,
    model: &ProgressModel,
) -> Result<()> {
    let mut summary = Summary::new();
    let mut exit_code: Option<i32> = None;
    // Ring buffer of the last complete guix lines, so a failure screen can show
    // the real error context (e.g. a git/SSL fetch failure) even when there's no
    // specific build log to fall back on.
    let mut recent: std::collections::VecDeque<String> = std::collections::VecDeque::new();
    const RECENT_CAP: usize = 25;

    // Wake on guix output *or* a ~1s keep-alive tick. The tick keeps the bar
    // creeping and the elapsed clock ticking even while guix is silent (e.g. a
    // long channel git-fetch that emits nothing for minutes), so the display
    // never looks frozen.
    let mut keepalive = tokio::time::interval(std::time::Duration::from_secs(1));
    keepalive.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            maybe_batch = op.events_mut().next() => {
                let Some(batch) = maybe_batch else { break };
                for evt in &batch {
                    if let libguix::ProgressEvent::ExitSummary { code, .. } = evt {
                        exit_code = Some(*code);
                    }
                    // Surface guix's native output for context: redraw (`\r`)
                    // lines render in place, complete lines scroll. Only the
                    // complete lines go to the log file (redraws are transient).
                    if let libguix::ProgressEvent::Line { text, redraw, .. } = evt {
                        let trimmed = text.trim_end_matches(['\r', '\n']);
                        if !trimmed.is_empty() {
                            ui.guix_output(trimmed, *redraw);
                            if !redraw {
                                crate::installer_log::write_line("guix:", trimmed);
                                recent.push_back(trimmed.to_string());
                                if recent.len() > RECENT_CAP {
                                    recent.pop_front();
                                }
                            }
                        }
                    }
                    summary.ingest(evt);
                }
                emit_progress(model, phase, &summary, ui);
            }
            _ = keepalive.tick() => {
                emit_progress(model, phase, &summary, ui);
            }
        }
    }

    if exit_code == Some(0) {
        return Ok(());
    }

    let msg = failure_message(&summary, op_name);
    crate::installer_log::write_line("guix-fail:", &msg);
    if let Some(line) = &summary.last_status_line {
        crate::installer_log::write_line("guix-fail:", &format!("last: {line}"));
    }

    let failed: Vec<String> = summary
        .builds
        .values()
        .filter(|b| b.status == BuildStatus::Failed)
        .map(|b| b.drv.clone())
        .collect();

    let mut tail = String::new();
    for drv in &failed {
        if let Ok(out) = crate::exec::run_cmd(&["guix", "log", drv.as_str()])
            && !out.stdout.is_empty()
        {
            crate::installer_log::write_line("build-log:", &format!("{drv}\n{}", out.stdout));
            tail = out.stdout;
        }
    }

    // Build one detailed message shown identically to the user by both
    // frontends: the GUI renders it on the failure screen, the CLI prints it at
    // top level. It leads with the real error, then recent guix output for
    // context, then a specific build log if one failed, then the log path.
    let mut screen = msg.clone();
    if let Some(line) = &summary.last_status_line {
        screen.push_str(&format!("\n{line}"));
    }
    if !recent.is_empty() {
        screen.push_str("\n\n--- recent guix output ---\n");
        screen.push_str(&recent.iter().cloned().collect::<Vec<_>>().join("\n"));
    }
    if !tail.is_empty() {
        let lines: Vec<&str> = tail.lines().collect();
        let start = lines.len().saturating_sub(30);
        screen.push_str("\n\n--- build log (tail) ---\n");
        screen.push_str(&lines[start..].join("\n"));
    }
    screen.push_str(&format!(
        "\n\nFull log: {}",
        crate::installer_log::LIVE_LOG_PATH
    ));

    bail!("{}", screen)
}
