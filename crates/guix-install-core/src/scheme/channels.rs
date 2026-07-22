use crate::mode::InstallMode;

// Single source of truth for the nonguix channel introduction. Reused both here
// (channels.scm) and in operating_system.rs, where the installed system's guix
// service bakes the same channel in so `guix describe` lists it.
pub(crate) const NONGUIX_NAME: &str = "nonguix";
pub(crate) const NONGUIX_URL: &str = "https://gitlab.com/nonguix/nonguix";
pub(crate) const NONGUIX_INTRO_COMMIT: &str = "897c1a470da759236cc11798f4e0a5f7d4d59fbc";
pub(crate) const NONGUIX_INTRO_FINGERPRINT: &str =
    "2A39 3FFF 68F4 EF7A 3D29  12AF 6F51 20A0 22FB B2D5";

pub fn render_channels(mode: &InstallMode) -> Option<String> {
    match mode {
        InstallMode::Guix => None,
        InstallMode::Nonguix => Some(nonguix_channels_scm()),
        InstallMode::Panther => Some(PANTHER_CHANNELS.into()),
        InstallMode::Enterprise { .. } => None,
    }
}

/// The `(channel …)` s-expression for the nonguix channel. `indent` is the
/// leading whitespace prefixed to every continuation line so the form aligns
/// wherever it is spliced (a `cons*` in channels.scm, or a `define` in
/// operating_system.rs). The form is paren-balanced on its own.
pub(crate) fn nonguix_channel_form(indent: &str) -> String {
    format!(
        "(channel\n\
         {i}(name '{name})\n\
         {i}(url \"{url}\")\n\
         {i}(introduction\n\
         {i} (make-channel-introduction\n\
         {i}  \"{commit}\"\n\
         {i}  (openpgp-fingerprint\n\
         {i}   \"{fpr}\"))))",
        i = indent,
        name = NONGUIX_NAME,
        url = NONGUIX_URL,
        commit = NONGUIX_INTRO_COMMIT,
        fpr = NONGUIX_INTRO_FINGERPRINT,
    )
}

fn nonguix_channels_scm() -> String {
    format!(
        "(cons* {}\n       %default-channels)\n",
        nonguix_channel_form("       ")
    )
}

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
