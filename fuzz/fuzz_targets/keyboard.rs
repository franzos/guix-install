#![no_main]

use guix_install_core::keyboard::parse_layouts;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|s: &str| {
    let _ = parse_layouts(s);
});
