use guix_install_core::connman::{LinkState, Tech, parse_services, parse_state};

const SERVICES: &str = "\
*AO MyHome Wifi          wifi_001122334455_4d79486f6d65_managed_psk
*AR Wired                ethernet_aabbccddeeff_cable
    NeighbourNet         wifi_001122334455_4e6569676862_managed_none
    Office               wifi_001122334455_4f6666696365_managed_ieee8021x
    OldWep               wifi_001122334455_4f6c64576570_managed_wep
";

#[test]
fn parses_state_values() {
    assert_eq!(
        parse_state("  State = online\n  OfflineMode = False\n"),
        LinkState::Online
    );
    assert_eq!(parse_state("  State = ready\n"), LinkState::Ready);
    assert_eq!(parse_state("  State = idle\n"), LinkState::Idle);
    assert_eq!(parse_state("  State = offline\n"), LinkState::Disconnect);
    assert_eq!(parse_state("garbage"), LinkState::Unknown);
    assert_eq!(parse_state("  State = failure\n"), LinkState::Failure);
    assert_eq!(parse_state("  State = disconnect\n"), LinkState::Disconnect);
    assert_eq!(
        parse_state("  State = association\n"),
        LinkState::Association
    );
    assert_eq!(
        parse_state("  State = configuration\n"),
        LinkState::Association
    );
}

#[test]
fn state_connected_semantics() {
    assert!(LinkState::Online.is_connected());
    assert!(LinkState::Ready.is_connected());
    assert!(!LinkState::Idle.is_connected());
}

#[test]
fn parses_services_summary() {
    let svcs = parse_services(SERVICES);
    assert_eq!(svcs.len(), 5);

    let home = &svcs[0];
    assert_eq!(home.name, "MyHome Wifi");
    assert_eq!(home.path, "wifi_001122334455_4d79486f6d65_managed_psk");
    assert_eq!(home.tech, Tech::Wifi);
    assert!(home.connected);
    assert!(home.secured);
    assert!(!home.eap);

    let wired = &svcs[1];
    assert_eq!(wired.tech, Tech::Ethernet);
    assert!(wired.connected);

    let neighbour = &svcs[2];
    assert_eq!(neighbour.name, "NeighbourNet");
    assert!(!neighbour.secured);
    assert!(!neighbour.connected);

    let office = &svcs[3];
    assert!(office.eap);
    assert!(office.secured);

    assert!(svcs[4].secured);
}

#[test]
fn ignores_non_service_lines() {
    assert!(parse_services("\n   \n").is_empty());
}
