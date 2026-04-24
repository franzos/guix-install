use std::fs;

use guix_install::enterprise;

#[test]
fn load_extracted_config_with_all_files() {
    let dir = tempfile::tempdir().unwrap();
    let dir_path = dir.path().to_str().unwrap();

    fs::write(
        dir.path().join("system.scm"),
        "(operating-system\n  (host-name \"test\"))",
    )
    .unwrap();
    fs::write(
        dir.path().join("channels.scm"),
        "(cons* (channel\n  (name 'test))\n  %default-channels)",
    )
    .unwrap();
    fs::write(
        dir.path().join("config.json"),
        r#"{"role": "DESKTOP", "timezone": "Europe/Berlin", "domain": "example.com"}"#,
    )
    .unwrap();

    let config = enterprise::load_extracted_config(dir_path).unwrap();

    assert!(config.system_scm.contains("host-name"));
    assert!(config.system_scm.contains("test"));
    assert!(config.channels_scm.is_some());
    assert!(config.channels_scm.as_ref().unwrap().contains("test"));

    let json = config.config_json.unwrap();
    assert_eq!(json["role"], "DESKTOP");
    assert_eq!(json["domain"], "example.com");
}

#[test]
fn load_extracted_config_system_scm_only() {
    let dir = tempfile::tempdir().unwrap();
    let dir_path = dir.path().to_str().unwrap();

    fs::write(dir.path().join("system.scm"), "(operating-system ...)").unwrap();

    let config = enterprise::load_extracted_config(dir_path).unwrap();

    assert_eq!(config.system_scm, "(operating-system ...)");
    assert!(config.channels_scm.is_none());
    assert!(config.config_json.is_none());
}

#[test]
fn load_extracted_config_fails_without_system_scm() {
    let dir = tempfile::tempdir().unwrap();
    let dir_path = dir.path().to_str().unwrap();

    // Only channels.scm, no system.scm
    fs::write(dir.path().join("channels.scm"), "(channels ...)").unwrap();

    let result = enterprise::load_extracted_config(dir_path);
    assert!(result.is_err());
}

#[test]
fn load_extracted_config_nested_tarball_directory() {
    let dir = tempfile::tempdir().unwrap();
    let dir_path = dir.path().to_str().unwrap();

    // Simulate: tar creates a top-level directory inside the extract dir
    let nested = dir.path().join("config-ABC123");
    fs::create_dir(&nested).unwrap();
    fs::write(nested.join("system.scm"), "(operating-system nested)").unwrap();
    fs::write(nested.join("channels.scm"), "(channels nested)").unwrap();
    fs::write(nested.join("config.json"), r#"{"role": "SERVER"}"#).unwrap();

    let config = enterprise::load_extracted_config(dir_path).unwrap();

    assert_eq!(config.system_scm, "(operating-system nested)");
    assert_eq!(config.channels_scm.as_deref(), Some("(channels nested)"));
    assert_eq!(config.config_json.unwrap()["role"], "SERVER");
}

#[test]
fn load_extracted_config_invalid_json_fails() {
    let dir = tempfile::tempdir().unwrap();
    let dir_path = dir.path().to_str().unwrap();

    fs::write(dir.path().join("system.scm"), "(operating-system ...)").unwrap();
    fs::write(dir.path().join("config.json"), "{invalid json}").unwrap();

    let result = enterprise::load_extracted_config(dir_path);
    assert!(result.is_err());
}

#[test]
fn cleanup_is_idempotent() {
    // Should not panic even when there's nothing to clean up
    enterprise::cleanup();
    enterprise::cleanup();
}
