#![no_main]

use guix_install_core::connman::{parse_service_detail, parse_services, parse_state};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|s: &str| {
    let _ = parse_state(s);
    let _ = parse_service_detail(s);
    let _ = parse_services(s);
});
