use std::time::SystemTime;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::config::SystemConfig;

/// Not under `/tmp`: the cow-store binds the target's `/tmp` over it mid-install.
const STATE_FILE: &str = "/run/guix-install-state";

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

    /// The saved config if `config` describes the same install, else `None`.
    ///
    /// A re-run picks fresh partition UUIDs, so the saved ones win. An edited
    /// system.scm embeds the fresh ones and can't be reconciled.
    pub fn resume_config(&self, config: &SystemConfig) -> Option<SystemConfig> {
        let mut candidate = config.clone();
        candidate.disk.boot_partition_uuid = self.config.disk.boot_partition_uuid.clone();
        candidate.disk.root_partition_uuid = self.config.disk.root_partition_uuid.clone();

        let same_uuids = config.disk.root_partition_uuid == candidate.disk.root_partition_uuid
            && config.disk.boot_partition_uuid == candidate.disk.boot_partition_uuid;
        if config.system_scm_override.is_some() && !same_uuids {
            return None;
        }
        let saved = serde_json::to_value(&self.config).ok()?;
        let wanted = serde_json::to_value(&candidate).ok()?;
        (saved == wanted).then_some(candidate)
    }

    /// Writes the current state to the state file.
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
        // A state file this version can't read is no state at all.
        Ok(serde_json::from_str(&data).ok())
    }

    /// Removes the state file after a successful installation.
    pub fn cleanup() -> Result<()> {
        if std::path::Path::new(STATE_FILE).exists() {
            std::fs::remove_file(STATE_FILE)?;
        }
        Ok(())
    }
}

/// In-progress interview config, persisted across a GUI keyboard-relaunch so the
/// restarted GUI resumes with the user's prior answers. Secrets are
/// `#[serde(skip)]` on SystemConfig, so they are naturally excluded.
const INTERVIEW_FILE: &str = "/run/guix-install-interview.json";

/// Serialize the in-progress config for a keyboard-relaunch resume.
pub fn save_interview_state(config: &SystemConfig) -> Result<()> {
    let json = serde_json::to_string(config)?;
    std::fs::write(INTERVIEW_FILE, json)?;
    Ok(())
}

/// Load and DELETE the interview state, if present. Returns the restored config.
pub fn take_interview_state() -> Option<SystemConfig> {
    let data = std::fs::read_to_string(INTERVIEW_FILE).ok()?;
    let _ = std::fs::remove_file(INTERVIEW_FILE);
    serde_json::from_str(&data).ok()
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
    fn interview_config_roundtrips_through_serde() {
        let config = SystemConfig {
            keyboard_layout: Some("de".into()),
            ..SystemConfig::default()
        };
        let json = serde_json::to_string(&config).unwrap();
        let loaded: SystemConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.keyboard_layout.as_deref(), Some("de"));
        assert!(loaded.password.is_none()); // serde(skip) — never serialized
    }

    #[test]
    fn resume_requires_the_same_install() {
        let mut saved = SystemConfig::default();
        saved.disk.assign_partition_uuids();
        let state = InstallState::new(&saved);

        // A re-run detects the disk again and draws fresh UUIDs; the saved ones win.
        let mut rerun = saved.clone();
        rerun.disk.root_partition_uuid = None;
        rerun.disk.boot_partition_uuid = None;
        rerun.disk.assign_partition_uuids();
        let resumed = state.resume_config(&rerun).expect("same install resumes");
        assert_eq!(
            resumed.disk.root_partition_uuid,
            saved.disk.root_partition_uuid
        );

        let mut other_fs = rerun.clone();
        other_fs.filesystem = crate::config::Filesystem::Btrfs;
        assert!(state.resume_config(&other_fs).is_none());

        let mut encrypted = rerun.clone();
        encrypted.encryption = Some(crate::config::EncryptionConfig::default());
        assert!(state.resume_config(&encrypted).is_none());

        let mut edited = rerun.clone();
        edited.system_scm_override = Some("(operating-system)".into());
        assert!(state.resume_config(&edited).is_none());
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
