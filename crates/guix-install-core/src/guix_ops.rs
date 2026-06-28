//! Drives the guix-heavy phases (`pull`, `system init`) through `libguix`,
//! folding its `ProgressEvent` stream into a [`libguix::progress::Summary`] and
//! reporting both flat (`progress(msg, pct)`) and structured
//! (`guix_progress(summary)`) updates to the [`UserInterface`].
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

use crate::progress::{self, Phase};
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

        drive(op, Phase::Pull, "guix pull", ui).await
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

        drive(op, Phase::Install, "guix system init", ui).await
    })
}

/// Consumes the op's event stream, folding batches into a `Summary` and
/// reporting progress. Bails with the captured failure on a non-zero exit.
async fn drive(
    mut op: libguix::Operation,
    phase: Phase,
    op_name: &str,
    ui: &dyn UserInterface,
) -> Result<()> {
    let mut summary = Summary::new();
    let mut exit_code: Option<i32> = None;

    while let Some(batch) = op.events_mut().next().await {
        for evt in &batch {
            if let libguix::ProgressEvent::ExitSummary { code, .. } = evt {
                exit_code = Some(*code);
            }
            summary.ingest(evt);
        }

        let intra = summary.percent_complete().unwrap_or(0.0);
        let overall = progress::overall_pct(phase, intra);
        let msg = summary
            .last_status_line
            .as_deref()
            .unwrap_or(op_name)
            .to_string();
        ui.progress(&msg, Some(overall));
        ui.guix_progress(&summary);
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

    let mut screen = msg.clone();
    if let Some(line) = &summary.last_status_line {
        screen.push_str(&format!("\n{line}"));
    }
    if !tail.is_empty() {
        let lines: Vec<&str> = tail.lines().collect();
        let start = lines.len().saturating_sub(30);
        screen.push_str("\n--- build log (tail) ---\n");
        screen.push_str(&lines[start..].join("\n"));
    }
    ui.error(&screen);

    bail!("{}", msg)
}
