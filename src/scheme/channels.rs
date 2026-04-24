use crate::mode::InstallMode;

pub fn render_channels(mode: &InstallMode) -> Option<String> {
    match mode {
        InstallMode::Guix => None,
        InstallMode::Nonguix => Some(NONGUIX_CHANNELS.into()),
        InstallMode::Panther => Some(PANTHER_CHANNELS.into()),
        InstallMode::Enterprise { .. } => None,
    }
}

const NONGUIX_CHANNELS: &str = "\
(cons* (channel
        (name 'nonguix)
        (url \"https://gitlab.com/nonguix/nonguix\")
        (introduction
         (make-channel-introduction
          \"897c1a470da759236cc11798f4e0a5f7d4d59fbc\"
          (openpgp-fingerprint
           \"2A39 3FFF 68F4 EF7A 3D29  12AF 6F51 20A0 22FB B2D5\"))))
       %default-channels)
";

const PANTHER_CHANNELS: &str = "\
(cons* (channel
        (name 'pantherx)
        (branch \"master\")
        (url \"https://codeberg.org/gofranz/panther.git\")
        (introduction
         (make-channel-introduction
          \"54b4056ac571611892c743b65f4c47dc298c49da\"
          (openpgp-fingerprint
           \"A36A D41E ECC7 A871 1003  5D24 524F EB1A 9D33 C9CB\"))))
       %default-channels)
";
