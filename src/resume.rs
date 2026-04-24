use std::time::SystemTime;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::config::SystemConfig;

const STATE_FILE: &str = "/tmp/.guix-install-state";

/// Persisted installation state for resume-on-failure support.
///
/// After each phase completes, the state is written to disk so that a
/// crashed or interrupted installation can resume from the last completed phase.
#[derive(Debug, Serialize, Deserialize)]
pub struct InstallState {
    pub completed_phases: Vec<u8>,
    pub config: SystemConfig,
    pub started_at: String,
}

impl InstallState {
    pub fn new(config: &SystemConfig) -> Self {
        InstallState {
            completed_phases: Vec::new(),
            config: config.clone(),
            started_at: timestamp_now(),
        }
    }

    /// Marks a phase as completed. Idempotent — adding the same phase twice is a no-op.
    pub fn mark_complete(&mut self, phase: u8) {
        if !self.completed_phases.contains(&phase) {
            self.completed_phases.push(phase);
        }
    }

    /// Writes the current state to the state file on the target filesystem.
    pub fn save(&self) -> Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(STATE_FILE, json)?;
        Ok(())
    }

    /// Loads a previously saved state, if one exists.
    pub fn load() -> Result<Option<Self>> {
        if !std::path::Path::new(STATE_FILE).exists() {
            return Ok(None);
        }
        let data = std::fs::read_to_string(STATE_FILE)?;
        let state: InstallState = serde_json::from_str(&data)?;
        Ok(Some(state))
    }

    /// Removes the state file after a successful installation.
    pub fn cleanup() -> Result<()> {
        if std::path::Path::new(STATE_FILE).exists() {
            std::fs::remove_file(STATE_FILE)?;
        }
        Ok(())
    }
}

fn timestamp_now() -> String {
    let d = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}", d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SystemConfig;

    #[test]
    fn roundtrip_serialization() {
        let config = SystemConfig::default();
        let mut state = InstallState::new(&config);
        state.mark_complete(1);
        state.mark_complete(2);
        state.mark_complete(3);

        let json = serde_json::to_string_pretty(&state).unwrap();
        let loaded: InstallState = serde_json::from_str(&json).unwrap();

        assert_eq!(loaded.completed_phases, vec![1, 2, 3]);
        assert_eq!(loaded.config.hostname, config.hostname);
        assert_eq!(loaded.config.timezone, config.timezone);
        assert!(!loaded.started_at.is_empty());
    }

    #[test]
    fn mark_complete_idempotent() {
        let config = SystemConfig::default();
        let mut state = InstallState::new(&config);
        state.mark_complete(1);
        state.mark_complete(1);
        state.mark_complete(1);

        assert_eq!(state.completed_phases, vec![1]);
    }

    #[test]
    fn mark_complete_ordering() {
        let config = SystemConfig::default();
        let mut state = InstallState::new(&config);
        state.mark_complete(3);
        state.mark_complete(1);
        state.mark_complete(5);

        assert_eq!(state.completed_phases, vec![3, 1, 5]);
    }

    #[test]
    fn new_state_empty_phases() {
        let config = SystemConfig::default();
        let state = InstallState::new(&config);
        assert!(state.completed_phases.is_empty());
    }

    #[test]
    fn timestamp_is_numeric() {
        let ts = timestamp_now();
        assert!(
            ts.parse::<u64>().is_ok(),
            "timestamp should be numeric: {ts}"
        );
    }

    #[test]
    fn phase_skipping_logic() {
        let config = SystemConfig::default();
        let mut state = InstallState::new(&config);
        state.mark_complete(1);
        state.mark_complete(2);
        state.mark_complete(3);

        // Phases 1-3 should be skipped, 4+ should run
        for phase in 1..=8u8 {
            if state.completed_phases.contains(&phase) {
                assert!(phase <= 3, "phase {phase} should not be in completed list");
            } else {
                assert!(phase > 3, "phase {phase} should be in completed list");
            }
        }
    }
}
