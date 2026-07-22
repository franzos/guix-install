//! Overall install progress as a monotonic, milestone-driven percentage.
//!
//! Guix has no single "overall %" of its own — its native output is per-download
//! byte bars and per-build status lines. To drive one bar (identically on the CLI
//! and the GUI) we synthesise a percentage from the install pipeline's phase plus
//! the guix op's [`Stage`](libguix::progress::Stage):
//!
//! * The six cheap shell-out phases (partition..authorize) share a small prefix
//!   `[0, PREP_END]`.
//! * The guix work fills the rest. If `guix pull` runs it takes `[PREP_END,
//!   PULL_END]` and `guix system init` `[PULL_END, CEIL]`; if pull is a no-op
//!   (channels already on the ISO) init takes the whole `[PREP_END, CEIL]`.
//! * Inside a guix op, each [`Stage`] maps to a sub-band. Downloading and Building
//!   are refined by real metrics (bytes / build counts) when guix reports totals;
//!   otherwise the bar creeps *asymptotically* toward the sub-band ceiling, so it
//!   always moves but never arrives — duration-agnostic (a 20-minute box and a
//!   2-hour box both approach, never reach, the next milestone).
//!
//! The bar is strictly monotonic and never exceeds [`CEIL`]; the final 100% is a
//! completion-screen concern the frontends own, so a stalled or jittery guix
//! stream can never make it read "done" early.

use std::cell::Cell;

use libguix::progress::{BuildStatus, Stage, Summary};

/// Install phases in execution order. Retained for op labelling and to tell the
/// progress model which guix band a `drive()` call belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Partition,
    Format,
    Mount,
    Swap,
    Config,
    Authorize,
    Pull,
    Install,
}

/// Fraction the six cheap phases (partition..authorize) share before guix work.
const PREP_END: f32 = 0.08;
/// If `guix pull` runs it fills `[PREP_END, PULL_END]`; init fills the rest.
const PULL_END: f32 = 0.50;
/// The bar never displays above this until the install truly completes.
const CEIL: f32 = 0.99;
/// Per-update asymptotic creep toward the current sub-band ceiling, for stages
/// with no real metric. Small enough that even a long, chatty stage only nudges.
const CREEP_ALPHA: f32 = 0.03;

/// Stateful, monotonic overall-progress accumulator shared across the whole
/// install. Interior-mutable so it can be handed to phase fns and the guix
/// `drive()` loop as `&ProgressModel`.
pub struct ProgressModel {
    /// Highest overall fraction emitted so far, in `[0, CEIL]`. Never decreases.
    displayed: Cell<f32>,
    /// Whether `guix pull` actually ran this session (shifts init's band up).
    pull_ran: Cell<bool>,
    /// Order of the stage seen on the last update, to reset creep on advance.
    /// `u8::MAX` means "no guix stage seen yet".
    stage_order: Cell<u8>,
    /// Within-stage creep accumulator `[0, 1]` for metric-less stages.
    creep: Cell<f32>,
}

impl Default for ProgressModel {
    fn default() -> Self {
        Self::new()
    }
}

impl ProgressModel {
    #[must_use]
    pub fn new() -> Self {
        Self {
            displayed: Cell::new(0.0),
            pull_ran: Cell::new(false),
            stage_order: Cell::new(u8::MAX),
            creep: Cell::new(0.0),
        }
    }

    /// Clamp `target` to `[0, CEIL]` and emit the monotonic max.
    fn bump(&self, target: f32) -> f32 {
        let clamped = target.clamp(0.0, CEIL);
        let next = self.displayed.get().max(clamped);
        self.displayed.set(next);
        next
    }

    /// Report a cheap phase (`num` 1..=6) at its start (`done == false`) or
    /// completion (`done == true`). Fills `[0, PREP_END]` evenly across six.
    pub fn cheap_phase(&self, num: u8, done: bool) -> f32 {
        let step = (num.saturating_sub(1)) as f32 + if done { 1.0 } else { 0.0 };
        self.bump((step / 6.0) * PREP_END)
    }

    /// A completed guix phase was skipped on resume; nudge the bar to its band
    /// ceiling so the checklist and bar stay consistent. Pull skips are handled
    /// by *not* calling this (a skipped pull is usually a no-op, not real work).
    pub fn skip_install(&self) -> f32 {
        self.bump(CEIL)
    }

    /// Record that `guix pull` actually ran, so `guix system init` starts from
    /// `PULL_END` instead of `PREP_END`.
    pub fn mark_pull_ran(&self) {
        self.pull_ran.set(true);
    }

    /// Overall `[lo, hi]` band a guix phase occupies.
    fn band(&self, phase: Phase) -> (f32, f32) {
        match phase {
            Phase::Pull => (PREP_END, PULL_END),
            Phase::Install => {
                let lo = if self.pull_ran.get() {
                    PULL_END
                } else {
                    PREP_END
                };
                (lo, CEIL)
            }
            // Cheap phases don't drive the guix bands.
            _ => (0.0, PREP_END),
        }
    }

    /// Fold a fresh [`Summary`] into a monotonic overall fraction for `phase`.
    /// Call once per guix event batch.
    pub fn guix_update(&self, phase: Phase, summary: &Summary) -> f32 {
        let (lo, hi) = self.band(phase);
        let span = hi - lo;
        let (sub_lo, sub_hi, frac) = self.stage_subband(summary);
        let target = lo + span * (sub_lo + (sub_hi - sub_lo) * frac);
        self.bump(target)
    }

    /// The `(sub_lo, sub_hi, frac)` for the summary's current stage, where the
    /// stage occupies `[sub_lo, sub_hi]` of the phase band and `frac` is the
    /// position within it (a real metric where guix gives one, else creep).
    fn stage_subband(&self, s: &Summary) -> (f32, f32, f32) {
        let order = stage_order(s.stage);
        if order != self.stage_order.get() {
            self.stage_order.set(order);
            self.creep.set(0.0);
        }

        match s.stage {
            Stage::Starting => (0.00, 0.03, self.creep_step()),
            Stage::ChannelUpdate => (0.00, 0.12, self.creep_step()),
            Stage::ComputingDeriv => (0.03, 0.15, self.creep_step()),
            Stage::Downloading => {
                let frac = metric_frac(s.bytes_downloaded, s.would_download_bytes)
                    .unwrap_or_else(|| self.creep_step());
                (0.12, 0.55, frac)
            }
            Stage::Building => {
                let frac = count_frac(s.build_count_done, s.would_build_count)
                    .unwrap_or_else(|| self.creep_step());
                (0.55, 0.85, frac)
            }
            Stage::Profile => (0.85, 0.98, self.creep_step()),
            // Done maps to the band ceiling (`frac == 1`); the frontend shows the
            // literal 100% on completion.
            Stage::Done => (1.00, 1.00, 1.0),
            // Hold on failure: bump() keeps whatever we last displayed.
            Stage::Failed => (0.00, 0.00, 0.0),
        }
    }

    /// Advance the within-stage creep asymptotically toward 1.0 and return it.
    /// `c += (1 - c) * ALPHA` approaches but never reaches 1, so a metric-less
    /// stage keeps inching without ever completing its sub-band.
    fn creep_step(&self) -> f32 {
        let next = {
            let c = self.creep.get();
            c + (1.0 - c) * CREEP_ALPHA
        };
        self.creep.set(next);
        next
    }
}

/// `done / total` as a fraction, or `None` when no positive total is known.
fn metric_frac(done: u64, total: Option<u64>) -> Option<f32> {
    match total {
        Some(t) if t > 0 => Some((done as f32 / t as f32).clamp(0.0, 1.0)),
        _ => None,
    }
}

fn count_frac(done: usize, total: Option<usize>) -> Option<f32> {
    match total {
        Some(t) if t > 0 => Some((done as f32 / t as f32).clamp(0.0, 1.0)),
        _ => None,
    }
}

/// A total order over stages so the model can detect stage advances.
fn stage_order(stage: Stage) -> u8 {
    match stage {
        Stage::Starting => 0,
        Stage::ChannelUpdate => 1,
        Stage::ComputingDeriv => 2,
        Stage::Downloading => 3,
        Stage::Building => 4,
        Stage::Profile => 5,
        Stage::Done => 6,
        Stage::Failed => 7,
    }
}

/// Folds a guix [`Summary`] into one compact status line shown identically on
/// the CLI and the GUI: the active stage, the most relevant in-flight item (a
/// download with byte counts, or the current build), and running tallies.
#[must_use]
pub fn summarize(s: &Summary) -> String {
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

    // A ticking elapsed clock is the clearest "still alive" signal, especially
    // on the CLI (no bar) and during silent stretches with the keep-alive tick.
    let elapsed = s
        .elapsed()
        .map(|d| {
            let secs = d.as_secs();
            format!(" ({}:{:02})", secs / 60, secs % 60)
        })
        .unwrap_or_default();

    // The tally is only meaningful once guix has actually built or fetched
    // something; suppress the all-zero "0 built · 0 downloaded · 0 B total" that
    // otherwise clutters the channel-update / derivation / finalize stages.
    let tally = if s.build_count_done > 0 || s.download_count_done > 0 || s.bytes_downloaded > 0 {
        format!(
            " - {} built \u{b7} {} downloaded \u{b7} {} total",
            s.build_count_done,
            s.download_count_done,
            human_bytes(s.bytes_downloaded)
        )
    } else {
        String::new()
    };

    match active {
        Some(item) => format!("{stage}{elapsed}: {item}{tally}"),
        None => format!("{stage}{elapsed}{tally}"),
    }
}

/// `1536` → `1.5 KiB`. Exact-byte counts below 1 KiB show as `N B`.
#[must_use]
pub fn human_bytes(n: u64) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;
    use libguix::{ProgressEvent, ProgressStream};

    fn line(text: &str) -> ProgressEvent {
        ProgressEvent::Line {
            stream: ProgressStream::Stderr,
            text: text.to_string(),
            redraw: false,
        }
    }

    #[test]
    fn cheap_phases_fill_the_prefix_monotonically() {
        let m = ProgressModel::new();
        assert!((m.cheap_phase(1, false) - 0.0).abs() < 1e-6);
        let a = m.cheap_phase(3, false);
        let b = m.cheap_phase(3, true);
        assert!(b >= a);
        // Phase 6 complete lands exactly on the prefix ceiling.
        assert!((m.cheap_phase(6, true) - PREP_END).abs() < 1e-6);
    }

    #[test]
    fn never_decreases() {
        let m = ProgressModel::new();
        m.cheap_phase(6, true); // 0.08
        // A fresh guix Starting summary would compute low; must not regress.
        let s = Summary::new();
        let p = m.guix_update(Phase::Install, &s);
        assert!(p >= PREP_END - 1e-6);
    }

    #[test]
    fn install_without_pull_spans_from_prep_end() {
        let m = ProgressModel::new();
        // No pull ran: install band starts at PREP_END, not PULL_END.
        let mut s = Summary::new();
        s.ingest(&line("Computing Guix derivation for 'x86_64-linux'..."));
        let p = m.guix_update(Phase::Install, &s);
        // ComputingDeriv sub-band is [0.03, 0.15] of [0.08, 0.99] → ~0.11..0.20.
        assert!(p > PREP_END && p < 0.30, "unexpected {p}");
    }

    #[test]
    fn pull_then_install_hands_off_at_pull_end() {
        let m = ProgressModel::new();
        m.mark_pull_ran();

        // Pull completes → its band ceiling (PULL_END).
        let mut pull = Summary::new();
        pull.ingest(&ProgressEvent::ExitSummary {
            code: 0,
            duration_secs: 1.0,
        });
        let after_pull = m.guix_update(Phase::Pull, &pull);
        assert!((after_pull - PULL_END).abs() < 1e-6);

        // Install starts fresh; must not regress below PULL_END.
        let install = Summary::new();
        let p = m.guix_update(Phase::Install, &install);
        assert!(p >= PULL_END - 1e-6);
    }

    #[test]
    fn downloading_tracks_real_bytes() {
        let m = ProgressModel::new();
        let mut s = Summary::new();
        s.ingest(&ProgressEvent::WouldDownload {
            bytes: 1000,
            items: vec!["/gnu/store/x-foo".into()],
        });
        s.ingest(&ProgressEvent::SubstituteDownload {
            item: "/gnu/store/x-foo".into(),
            bytes_done: 0,
            bytes_total: Some(1000),
        });
        let low = m.guix_update(Phase::Install, &s);
        s.ingest(&ProgressEvent::SubstituteDownload {
            item: "/gnu/store/x-foo".into(),
            bytes_done: 900,
            bytes_total: Some(1000),
        });
        let high = m.guix_update(Phase::Install, &s);
        assert!(high > low, "bytes progress should advance the bar");
    }

    #[test]
    fn creep_approaches_but_never_reaches_subband_ceiling() {
        let m = ProgressModel::new();
        // ComputingDeriv has no metric → pure creep, ceiling 0.15 of the band.
        let mut s = Summary::new();
        s.ingest(&line("Computing Guix derivation for 'x86_64-linux'..."));
        let mut last = 0.0;
        for _ in 0..500 {
            last = m.guix_update(Phase::Install, &s);
        }
        // Band [0.08, 0.99], sub ceiling 0.15 → 0.08 + 0.91*0.15 ≈ 0.2265.
        let ceiling = PREP_END + (CEIL - PREP_END) * 0.15;
        assert!(last < ceiling, "creep must stay below the sub-band ceiling");
        assert!(last > ceiling - 0.02, "creep should get close over time");
    }

    #[test]
    fn never_reaches_ceiling_before_done() {
        let m = ProgressModel::new();
        let mut s = Summary::new();
        // Push through downloading + building fully complete by count/bytes.
        s.ingest(&ProgressEvent::WouldDownload {
            bytes: 100,
            items: vec!["/gnu/store/x-foo".into()],
        });
        s.ingest(&ProgressEvent::SubstituteDownloadDone {
            item: "/gnu/store/x-foo".into(),
            bytes_total: Some(100),
        });
        s.ingest(&ProgressEvent::WouldBuild {
            bytes: 0,
            items: vec!["/gnu/store/a-foo.drv".into()],
        });
        s.ingest(&ProgressEvent::BuildStart {
            drv: "/gnu/store/a-foo.drv".into(),
        });
        s.ingest(&ProgressEvent::BuildDone {
            drv: "/gnu/store/a-foo.drv".into(),
        });
        let p = m.guix_update(Phase::Install, &s);
        assert!(p < CEIL, "must not hit CEIL while still Building");

        // Only the terminal Done pins it to CEIL.
        s.ingest(&ProgressEvent::ExitSummary {
            code: 0,
            duration_secs: 1.0,
        });
        let done = m.guix_update(Phase::Install, &s);
        assert!((done - CEIL).abs() < 1e-6);
    }

    #[test]
    fn summarize_downloading_shows_bytes() {
        let mut s = Summary::new();
        s.ingest(&ProgressEvent::SubstituteDownload {
            item: "/gnu/store/abc-hello-2.12".into(),
            bytes_done: 512,
            bytes_total: Some(1024),
        });
        let out = summarize(&s);
        assert!(out.contains("downloading"), "{out}");
        assert!(out.contains("hello-2.12"), "{out}");
        assert!(out.contains("512 B/1.0 KiB"), "{out}");
        // A fresh summary shows a sub-minute elapsed clock.
        assert!(out.contains("(0:"), "{out}");
    }

    #[test]
    fn summarize_suppresses_zero_tally() {
        let mut s = Summary::new();
        s.ingest(&line("Updating channel 'guix' from Git repository at ..."));
        let out = summarize(&s);
        assert!(out.contains("updating channels"), "{out}");
        assert!(out.contains("(0:"), "{out}");
        // Nothing built or downloaded yet → no "0 built · …" clutter.
        assert!(!out.contains("built"), "{out}");
        assert!(!out.contains("downloaded"), "{out}");
    }

    #[test]
    fn human_bytes_units() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(1536), "1.5 KiB");
    }
}
