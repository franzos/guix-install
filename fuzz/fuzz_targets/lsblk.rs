#![no_main]

use guix_install_core::disk::detect::parse_lsblk_json;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|s: &str| {
    let _ = parse_lsblk_json(s);
});
