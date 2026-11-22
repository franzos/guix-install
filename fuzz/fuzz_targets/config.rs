#![no_main]

use guix_install_core::config::{
    validate_config_id, validate_hostname, validate_ssh_public_key, validate_username,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|s: &str| {
    let _ = validate_hostname(s);
    let _ = validate_username(s);
    let _ = validate_ssh_public_key(s);
    let _ = validate_config_id(s);
});
