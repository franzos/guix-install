use std::path::Path;

use anyhow::{Context, Result, bail};
use flate2::read::GzDecoder;
use tar::Archive;

/// Downloaded and extracted enterprise configuration.
///
/// Contains the pre-built system.scm (required) and optional channels.scm
/// and config.json from the remote tarball.
#[derive(Debug)]
pub struct EnterpriseConfig {
    pub system_scm: String,
    pub channels_scm: Option<String>,
    pub config_json: Option<serde_json::Value>,
}

const EXTRACT_DIR: &str = "/tmp/guix-install-config";

/// Download and extract an enterprise config tarball.
///
/// Fetches `{config_url}/{config_id}.tar.gz` over HTTPS and streams the
/// response through gzip decompression and tar extraction into `EXTRACT_DIR`.
/// No intermediate tarball is written to disk. The archive must contain at
/// least a `system.scm` file (either at the top level or inside a single
/// subdirectory).
pub fn fetch_enterprise_config(config_id: &str, config_url: &str) -> Result<EnterpriseConfig> {
    let url = format!("{config_url}/{config_id}.tar.gz");

    std::fs::create_dir_all(EXTRACT_DIR)
        .with_context(|| format!("failed to create extract directory {EXTRACT_DIR}"))?;

    let response = ureq::get(&url)
        .call()
        .with_context(|| format!("failed to download enterprise config from {url}"))?;

    let gz = GzDecoder::new(response.into_reader());
    Archive::new(gz)
        .unpack(EXTRACT_DIR)
        .with_context(|| format!("failed to extract config tarball from {url}"))?;

    load_extracted_config(EXTRACT_DIR)
}

/// Load an enterprise config from an already-extracted directory.
///
/// Separated from `fetch_enterprise_config` so it can be tested independently.
pub fn load_extracted_config(dir: &str) -> Result<EnterpriseConfig> {
    let system_scm = read_config_file(dir, "system.scm")?;
    let channels_scm = read_config_file_opt(dir, "channels.scm");
    let config_json = read_config_file_opt(dir, "config.json")
        .map(|s| serde_json::from_str(&s))
        .transpose()
        .context("failed to parse config.json")?;

    Ok(EnterpriseConfig {
        system_scm,
        channels_scm,
        config_json,
    })
}

/// Read a required file from the extracted config directory.
///
/// Tries `dir/name` first, then `dir/*/name` (tarballs often contain a
/// top-level directory).
fn read_config_file(dir: &str, name: &str) -> Result<String> {
    // Try direct path first
    let direct = format!("{dir}/{name}");
    if Path::new(&direct).exists() {
        return std::fs::read_to_string(&direct)
            .with_context(|| format!("failed to read {direct}"));
    }

    // Look in subdirectories (tarball might have a top-level directory)
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            if entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
                let nested = entry.path().join(name);
                if nested.exists() {
                    return std::fs::read_to_string(&nested)
                        .with_context(|| format!("failed to read {}", nested.display()));
                }
            }
        }
    }

    bail!("required file '{name}' not found in config archive at {dir}")
}

/// Read an optional file from the extracted config directory.
///
/// Returns `None` if the file doesn't exist anywhere in the directory.
fn read_config_file_opt(dir: &str, name: &str) -> Option<String> {
    read_config_file(dir, name).ok()
}

/// Clean up extracted config files.
pub fn cleanup() {
    let _ = std::fs::remove_dir_all(EXTRACT_DIR);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn make_temp_dir() -> tempfile::TempDir {
        tempfile::tempdir().expect("failed to create temp dir")
    }

    #[test]
    fn read_config_file_direct() {
        let dir = make_temp_dir();
        let dir_path = dir.path().to_str().unwrap();

        fs::write(dir.path().join("system.scm"), "(operating-system ...)").unwrap();

        let content = read_config_file(dir_path, "system.scm").unwrap();
        assert_eq!(content, "(operating-system ...)");
    }

    #[test]
    fn read_config_file_nested() {
        let dir = make_temp_dir();
        let dir_path = dir.path().to_str().unwrap();

        // Simulate tarball with a top-level directory
        let nested = dir.path().join("config-abc123");
        fs::create_dir(&nested).unwrap();
        fs::write(nested.join("system.scm"), "(operating-system nested)").unwrap();

        let content = read_config_file(dir_path, "system.scm").unwrap();
        assert_eq!(content, "(operating-system nested)");
    }

    #[test]
    fn read_config_file_not_found() {
        let dir = make_temp_dir();
        let dir_path = dir.path().to_str().unwrap();

        let result = read_config_file(dir_path, "system.scm");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("system.scm"),
            "error should mention the file name: {err}"
        );
    }

    #[test]
    fn read_config_file_opt_returns_none_when_missing() {
        let dir = make_temp_dir();
        let dir_path = dir.path().to_str().unwrap();

        assert!(read_config_file_opt(dir_path, "channels.scm").is_none());
    }

    #[test]
    fn read_config_file_opt_returns_some_when_present() {
        let dir = make_temp_dir();
        let dir_path = dir.path().to_str().unwrap();

        fs::write(dir.path().join("channels.scm"), "(channels ...)").unwrap();

        let result = read_config_file_opt(dir_path, "channels.scm");
        assert_eq!(result, Some("(channels ...)".into()));
    }

    #[test]
    fn load_extracted_config_all_files() {
        let dir = make_temp_dir();
        let dir_path = dir.path().to_str().unwrap();

        fs::write(dir.path().join("system.scm"), "(operating-system ...)").unwrap();
        fs::write(dir.path().join("channels.scm"), "(channels ...)").unwrap();
        fs::write(
            dir.path().join("config.json"),
            r#"{"role": "DESKTOP", "timezone": "Europe/Berlin"}"#,
        )
        .unwrap();

        let config = load_extracted_config(dir_path).unwrap();
        assert_eq!(config.system_scm, "(operating-system ...)");
        assert_eq!(config.channels_scm, Some("(channels ...)".into()));
        assert!(config.config_json.is_some());

        let json = config.config_json.unwrap();
        assert_eq!(json["role"], "DESKTOP");
        assert_eq!(json["timezone"], "Europe/Berlin");
    }

    #[test]
    fn load_extracted_config_minimal() {
        let dir = make_temp_dir();
        let dir_path = dir.path().to_str().unwrap();

        // Only system.scm is required
        fs::write(dir.path().join("system.scm"), "(operating-system ...)").unwrap();

        let config = load_extracted_config(dir_path).unwrap();
        assert_eq!(config.system_scm, "(operating-system ...)");
        assert!(config.channels_scm.is_none());
        assert!(config.config_json.is_none());
    }

    #[test]
    fn load_extracted_config_fails_without_system_scm() {
        let dir = make_temp_dir();
        let dir_path = dir.path().to_str().unwrap();

        // No system.scm at all
        fs::write(dir.path().join("channels.scm"), "(channels ...)").unwrap();

        let result = load_extracted_config(dir_path);
        assert!(result.is_err());
    }

    #[test]
    fn load_extracted_config_invalid_json() {
        let dir = make_temp_dir();
        let dir_path = dir.path().to_str().unwrap();

        fs::write(dir.path().join("system.scm"), "(operating-system ...)").unwrap();
        fs::write(dir.path().join("config.json"), "not valid json").unwrap();

        let result = load_extracted_config(dir_path);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("config.json"),
            "error should mention config.json: {err}"
        );
    }

    #[test]
    fn load_extracted_config_nested_directory() {
        let dir = make_temp_dir();
        let dir_path = dir.path().to_str().unwrap();

        // Simulate tarball with top-level directory
        let nested = dir.path().join("my-config");
        fs::create_dir(&nested).unwrap();
        fs::write(nested.join("system.scm"), "(operating-system nested)").unwrap();
        fs::write(nested.join("channels.scm"), "(channels nested)").unwrap();

        let config = load_extracted_config(dir_path).unwrap();
        assert_eq!(config.system_scm, "(operating-system nested)");
        assert_eq!(config.channels_scm, Some("(channels nested)".into()));
    }

    #[test]
    fn direct_file_takes_priority_over_nested() {
        let dir = make_temp_dir();
        let dir_path = dir.path().to_str().unwrap();

        // Both direct and nested versions exist — direct should win
        fs::write(dir.path().join("system.scm"), "direct").unwrap();

        let nested = dir.path().join("subdir");
        fs::create_dir(&nested).unwrap();
        fs::write(nested.join("system.scm"), "nested").unwrap();

        let content = read_config_file(dir_path, "system.scm").unwrap();
        assert_eq!(content, "direct");
    }
}
