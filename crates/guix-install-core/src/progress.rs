//! Weighted overall-progress across the 8 install phases.
//!
//! `guix pull` and `guix system init` dominate wall-clock time, so they carry
//! most of the weight; the cheap shell-out phases (partition/format/mount/swap/
//! config/authorize) report start/end only. The overall percent is
//! `(weight before this phase + intra-fraction × this phase's weight) / total`.

/// Install phases in execution order, with their relative weights baked into
/// [`Self::weight`]. Mirrors the 1..=8 numbering used by the pipeline.
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

const PHASES: [Phase; 8] = [
    Phase::Partition,
    Phase::Format,
    Phase::Mount,
    Phase::Swap,
    Phase::Config,
    Phase::Authorize,
    Phase::Pull,
    Phase::Install,
];

impl Phase {
    /// Relative weight. The two guix phases dwarf the rest; the cheap phases
    /// share a small constant slice so the bar still nudges forward through
    /// partition/format/mount.
    const fn weight(self) -> u32 {
        match self {
            Phase::Partition
            | Phase::Format
            | Phase::Mount
            | Phase::Swap
            | Phase::Config
            | Phase::Authorize => 1,
            Phase::Pull => 40,
            Phase::Install => 54,
        }
    }

    fn index(self) -> usize {
        PHASES.iter().position(|&p| p == self).unwrap_or(0)
    }
}

fn total_weight() -> u32 {
    PHASES.iter().map(|p| p.weight()).sum()
}

fn weight_before(phase: Phase) -> u32 {
    PHASES[..phase.index()].iter().map(|p| p.weight()).sum()
}

/// Overall fraction (0.0..=1.0) given the current phase and its intra-phase
/// fraction (also 0.0..=1.0). `intra` is clamped.
#[must_use]
pub fn overall_pct(phase: Phase, intra: f32) -> f32 {
    let intra = intra.clamp(0.0, 1.0);
    let done = weight_before(phase) as f32 + intra * phase.weight() as f32;
    (done / total_weight() as f32).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weights_sum_to_100() {
        assert_eq!(total_weight(), 100);
    }

    #[test]
    fn first_phase_start_is_zero() {
        assert_eq!(overall_pct(Phase::Partition, 0.0), 0.0);
    }

    #[test]
    fn install_complete_is_one() {
        assert_eq!(overall_pct(Phase::Install, 1.0), 1.0);
    }

    #[test]
    fn pull_start_reflects_cheap_phases_done() {
        // partition..authorize = 6 weight before pull.
        assert!((overall_pct(Phase::Pull, 0.0) - 0.06).abs() < 1e-6);
    }

    #[test]
    fn pull_half_is_weighted() {
        // 6 + 0.5*40 = 26 of 100.
        assert!((overall_pct(Phase::Pull, 0.5) - 0.26).abs() < 1e-6);
    }

    #[test]
    fn intra_is_clamped() {
        assert_eq!(overall_pct(Phase::Install, 5.0), 1.0);
        assert_eq!(overall_pct(Phase::Partition, -1.0), 0.0);
    }
}
