use guix_install_core::mode::InstallMode;
use guix_install_core::steps::{StepId, StepNavigator};

#[test]
fn panther_includes_all_steps() {
    let nav = StepNavigator::new(&InstallMode::Panther);
    let steps = nav.steps();
    assert_eq!(steps.len(), 11);
    assert_eq!(
        steps,
        &[
            StepId::Keyboard,
            StepId::Network,
            StepId::Mode,
            StepId::Locale,
            StepId::Timezone,
            StepId::Hostname,
            StepId::Disk,
            StepId::Encryption,
            StepId::Users,
            StepId::Desktop,
            StepId::Summary,
        ]
    );
}

#[test]
fn guix_includes_all_steps() {
    let nav = StepNavigator::new(&InstallMode::Guix);
    assert_eq!(nav.steps().len(), 11);
}

#[test]
fn nonguix_includes_all_steps() {
    let nav = StepNavigator::new(&InstallMode::Nonguix);
    assert_eq!(nav.steps().len(), 11);
}

#[test]
fn enterprise_skips_config_steps() {
    let mode = InstallMode::Enterprise {
        config_id: "test-id".into(),
        config_url: "https://example.com/install".into(),
    };
    let nav = StepNavigator::new(&mode);
    let steps = nav.steps();

    assert_eq!(steps.len(), 6);
    assert_eq!(
        steps,
        &[
            StepId::Keyboard,
            StepId::Network,
            StepId::Mode,
            StepId::Disk,
            StepId::Encryption,
            StepId::Summary,
        ]
    );

    assert!(!steps.contains(&StepId::Locale));
    assert!(!steps.contains(&StepId::Timezone));
    assert!(!steps.contains(&StepId::Hostname));
    assert!(!steps.contains(&StepId::Users));
    assert!(!steps.contains(&StepId::Desktop));
}

#[test]
fn advance_walks_forward() {
    let mut nav = StepNavigator::new(&InstallMode::Panther);
    assert_eq!(nav.current(), StepId::Keyboard);

    nav.advance();
    assert_eq!(nav.current(), StepId::Network);

    nav.advance();
    assert_eq!(nav.current(), StepId::Mode);

    nav.advance();
    assert_eq!(nav.current(), StepId::Locale);

    nav.advance();
    assert_eq!(nav.current(), StepId::Timezone);
}

#[test]
fn go_back_walks_backward() {
    let mut nav = StepNavigator::new(&InstallMode::Panther);
    nav.advance(); // Network
    nav.advance(); // Mode
    nav.advance(); // Locale

    nav.go_back();
    assert_eq!(nav.current(), StepId::Mode);

    nav.go_back();
    assert_eq!(nav.current(), StepId::Network);

    nav.go_back();
    assert_eq!(nav.current(), StepId::Keyboard);
}

#[test]
fn go_back_at_first_step_stays() {
    let mut nav = StepNavigator::new(&InstallMode::Panther);
    assert!(nav.is_first());
    assert_eq!(nav.current(), StepId::Keyboard);

    nav.go_back();
    assert!(nav.is_first());
    assert_eq!(nav.current(), StepId::Keyboard);
}

#[test]
fn advance_at_last_step_stays() {
    let mut nav = StepNavigator::new(&InstallMode::Panther);
    // Advance to the end
    for _ in 0..20 {
        nav.advance();
    }
    assert!(nav.is_last());
    assert_eq!(nav.current(), StepId::Summary);

    nav.advance();
    assert!(nav.is_last());
    assert_eq!(nav.current(), StepId::Summary);
}

#[test]
fn is_first_is_last_correct() {
    let mut nav = StepNavigator::new(&InstallMode::Panther);

    assert!(nav.is_first());
    assert!(!nav.is_last());

    // Walk to the last step
    for _ in 0..10 {
        nav.advance();
    }

    assert!(!nav.is_first());
    assert!(nav.is_last());
}

#[test]
fn reset_for_mode_rebuilds_to_enterprise() {
    let mut nav = StepNavigator::new(&InstallMode::Panther);
    assert_eq!(nav.steps().len(), 11);

    nav.advance();
    nav.advance();
    nav.advance();
    assert_eq!(nav.current(), StepId::Locale);

    let enterprise = InstallMode::Enterprise {
        config_id: "abc".into(),
        config_url: "https://example.com".into(),
    };
    nav.reset_for_mode(&enterprise);

    assert_eq!(nav.steps().len(), 6);
    // After reset, position is at index 3 (Disk)
    assert_eq!(nav.current(), StepId::Disk);
}

#[test]
fn reset_for_mode_rebuilds_back_to_full() {
    let enterprise = InstallMode::Enterprise {
        config_id: "x".into(),
        config_url: "https://example.com".into(),
    };
    let mut nav = StepNavigator::new(&enterprise);
    assert_eq!(nav.steps().len(), 6);

    nav.reset_for_mode(&InstallMode::Panther);
    assert_eq!(nav.steps().len(), 11);
    assert_eq!(nav.current(), StepId::Locale);
}

#[test]
fn enterprise_advance_through_all() {
    let mode = InstallMode::Enterprise {
        config_id: "test".into(),
        config_url: "https://example.com".into(),
    };
    let mut nav = StepNavigator::new(&mode);

    assert_eq!(nav.current(), StepId::Keyboard);
    nav.advance();
    assert_eq!(nav.current(), StepId::Network);
    nav.advance();
    assert_eq!(nav.current(), StepId::Mode);
    nav.advance();
    assert_eq!(nav.current(), StepId::Disk);
    nav.advance();
    assert_eq!(nav.current(), StepId::Encryption);
    nav.advance();
    assert_eq!(nav.current(), StepId::Summary);
    assert!(nav.is_last());
}
