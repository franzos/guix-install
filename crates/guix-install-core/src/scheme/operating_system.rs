use crate::config::{DesktopEnvironment, Filesystem, Firmware, SystemConfig};
use crate::disk::partition_path;
use crate::mode::InstallMode;

pub fn render_operating_system(config: &SystemConfig) -> String {
    if matches!(config.mode, InstallMode::Enterprise { .. }) {
        return String::new();
    }

    let mut sections: Vec<String> = vec![render_use_modules(config), String::new()];

    // Nonguix bakes the nonguix channel + substitute key into the installed
    // system's guix service (see render_nonguix_services), so `guix describe`
    // lists nonguix out of the box. These top-level defines feed that.
    if matches!(config.mode, InstallMode::Nonguix) {
        sections.push(render_nonguix_defines());
        sections.push(String::new());
    }

    sections.push(render_os_form(config));

    sections.join("\n")
}

/// Nonguix-only top-level defines: the channel list baked into the guix service,
/// plus the substitute server's public key as a file-like object.
fn render_nonguix_defines() -> String {
    // 9-space continuation indent aligns the channel form under `(cons* (channel`.
    let channel = crate::scheme::channels::nonguix_channel_form("         ");
    let key = escape_scheme_string(include_str!("../../keys/substitutes.nonguix.org.pub"));

    format!(
        "(define %nonguix-channels\n\
         \x20 (cons* {channel}\n\
         \x20        %default-channels))\n\
         \n\
         (define %nonguix-substitute-key\n\
         \x20 (plain-file \"substitutes.nonguix.org.pub\"\n\
         \x20  \"{key}\"))"
    )
}

/// Escape a string for embedding as a Scheme string literal. Only backslash and
/// double-quote are special; newlines stay literal (valid inside Scheme strings,
/// and `read` ignores the resulting whitespace between s-expression tokens).
fn escape_scheme_string(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn render_use_modules(config: &SystemConfig) -> String {
    let mut lines: Vec<String> = Vec::new();

    match &config.mode {
        InstallMode::Guix => {
            lines.push("(use-modules (gnu))".into());
            lines.push("(use-service-modules networking ssh desktop)".into());
        }
        InstallMode::Nonguix => {
            lines.push("(use-modules (gnu))".into());
            // `base` -> guix-service-type, guix-configuration,
            // %default-authorized-guix-keys, %default-substitute-urls.
            lines.push("(use-service-modules networking ssh desktop base)".into());
            lines.push("(use-modules (nongnu packages linux))".into());
            lines.push("(use-modules (nongnu system linux-initrd))".into());
            // guix-for-channels lives here, not in (guix channels).
            lines.push("(use-modules (gnu packages package-management))".into());
            // channel, make-channel-introduction, openpgp-fingerprint,
            // %default-channels.
            lines.push("(use-modules (guix channels))".into());
            // plain-file for the substitute key.
            lines.push("(use-modules (guix gexp))".into());
        }
        InstallMode::Panther => {
            lines.push("(use-modules (gnu))".into());
            lines.push("(use-service-modules networking ssh desktop)".into());
            lines.push("(use-modules (px system os))".into());
        }
        InstallMode::Enterprise { .. } => unreachable!(),
    }

    lines.join("\n")
}

fn render_os_form(config: &SystemConfig) -> String {
    let mut parts: Vec<String> = Vec::new();

    let inherit = render_inherit(config);

    parts.push("(operating-system".into());

    if let Some(inh) = inherit {
        parts.push(format!("  {inh}"));
    }

    parts.push(format!("  (host-name \"{}\")", config.hostname));
    parts.push(format!("  (timezone \"{}\")", config.timezone));
    parts.push(format!("  (locale \"{}\")", config.locale));

    if let Some(layout) = &config.keyboard_layout {
        parts.push(render_keyboard_layout(layout));
    }

    parts.push(String::new());
    parts.push(render_bootloader(config));

    if matches!(config.mode, InstallMode::Nonguix) {
        parts.push(String::new());
        parts.push("  (kernel linux)".into());
        parts.push("  (initrd microcode-initrd)".into());
        parts.push("  (firmware (list linux-firmware))".into());
    }

    if config.encryption.is_some() {
        parts.push(String::new());
        parts.push(render_mapped_devices(config));
    }

    parts.push(String::new());
    parts.push(render_file_systems(config));

    parts.push(String::new());
    parts.push(render_swap());

    parts.push(String::new());
    parts.push(render_users(config));

    if let Some(packages) = render_packages(config) {
        parts.push(String::new());
        parts.push(packages);
    }

    parts.push(String::new());
    parts.push(render_services(config));

    parts.push(")".into());

    parts.join("\n")
}

fn render_inherit(config: &SystemConfig) -> Option<String> {
    match &config.mode {
        InstallMode::Panther => Some("(inherit %os-base)".into()),
        _ => None,
    }
}

fn render_keyboard_layout(layout: &str) -> String {
    format!("  (keyboard-layout (keyboard-layout \"{layout}\"))")
}

fn render_bootloader(config: &SystemConfig) -> String {
    let bootloader = match config.firmware {
        Firmware::Efi => "grub-efi-bootloader",
        Firmware::Bios => "grub-bootloader",
    };

    let target = match config.firmware {
        Firmware::Efi => "\"/boot/efi\"".to_string(),
        Firmware::Bios => format!("\"{}\"", config.disk.dev_path),
    };

    let kb = if config.keyboard_layout.is_some() {
        "\n                (keyboard-layout keyboard-layout)"
    } else {
        ""
    };

    format!(
        "  (bootloader (bootloader-configuration\n\
         \x20              (bootloader {bootloader})\n\
         \x20              (targets '({target})){kb}))"
    )
}

fn render_mapped_devices(config: &SystemConfig) -> String {
    let enc = config.encryption.as_ref().unwrap();
    let source = match &config.disk.root_partition_uuid {
        Some(uuid) => format!("(uuid \"{uuid}\")"),
        None => format!("\"{}\"", partition_path(&config.disk.dev_path, 2)),
    };

    format!(
        "  (mapped-devices\n\
         \x20  (list (mapped-device\n\
         \x20         (source {source})\n\
         \x20         (target \"{}\")\n\
         \x20         (type luks-device-mapping))))",
        enc.device_target
    )
}

fn render_file_systems(config: &SystemConfig) -> String {
    let fs_type = match config.filesystem {
        Filesystem::Ext4 => "ext4",
        Filesystem::Btrfs => "btrfs",
        Filesystem::Xfs => "xfs",
    };

    let encrypted = config.encryption.is_some();
    let efi = config.firmware == Firmware::Efi;

    match (efi, encrypted) {
        (false, false) => {
            format!(
                "  (file-systems (cons (file-system\n\
                 \x20                      (device (file-system-label \"my-root\"))\n\
                 \x20                      (mount-point \"/\")\n\
                 \x20                      (type \"{fs_type}\"))\n\
                 \x20                    %base-file-systems))"
            )
        }
        (true, false) => {
            let boot_device = render_boot_device(config);
            format!(
                "  (file-systems (cons* (file-system\n\
                 \x20                       (device (file-system-label \"my-root\"))\n\
                 \x20                       (mount-point \"/\")\n\
                 \x20                       (type \"{fs_type}\"))\n\
                 \x20                     (file-system\n\
                 \x20                       (mount-point \"/boot/efi\")\n\
                 \x20                       (device {boot_device})\n\
                 \x20                       (type \"vfat\"))\n\
                 \x20                     %base-file-systems))"
            )
        }
        (false, true) => {
            let target = &config.encryption.as_ref().unwrap().device_target;
            format!(
                "  (file-systems (cons (file-system\n\
                 \x20                      (mount-point \"/\")\n\
                 \x20                      (device \"/dev/mapper/{target}\")\n\
                 \x20                      (type \"{fs_type}\")\n\
                 \x20                      (dependencies mapped-devices))\n\
                 \x20                    %base-file-systems))"
            )
        }
        (true, true) => {
            let boot_device = render_boot_device(config);
            let target = &config.encryption.as_ref().unwrap().device_target;
            format!(
                "  (file-systems (cons* (file-system\n\
                 \x20                       (mount-point \"/boot/efi\")\n\
                 \x20                       (device {boot_device})\n\
                 \x20                       (type \"vfat\"))\n\
                 \x20                     (file-system\n\
                 \x20                       (mount-point \"/\")\n\
                 \x20                       (device \"/dev/mapper/{target}\")\n\
                 \x20                       (type \"{fs_type}\")\n\
                 \x20                       (dependencies mapped-devices))\n\
                 \x20                     %base-file-systems))"
            )
        }
    }
}

fn render_boot_device(config: &SystemConfig) -> String {
    match &config.disk.boot_partition_uuid {
        Some(uuid) => format!("(uuid \"{uuid}\" 'fat32)"),
        None => format!("\"{}\"", partition_path(&config.disk.dev_path, 1)),
    }
}

fn render_swap() -> String {
    "  (swap-devices (list (swap-space (target \"/swapfile\"))))".into()
}

fn render_users(config: &SystemConfig) -> String {
    if config.users.is_empty() {
        return "  (users %base-user-accounts)".into();
    }

    let mut parts: Vec<String> = Vec::new();

    if config.users.len() == 1 {
        let u = &config.users[0];
        parts.push("  (users (cons (user-account".into());
        parts.push(format!("                (name \"{}\")", u.name));
        parts.push(format!("                (comment \"{}\")", u.comment));
        parts.push("                (group \"users\")".into());
        parts.push(format!(
            "                (supplementary-groups '({})))",
            u.groups
                .iter()
                .map(|g| format!("\"{g}\""))
                .collect::<Vec<_>>()
                .join(" ")
        ));
        parts.push("               %base-user-accounts))".into());
    } else {
        parts.push("  (users (append (list".into());
        for u in &config.users {
            parts.push("                  (user-account".into());
            parts.push(format!("                    (name \"{}\")", u.name));
            parts.push(format!("                    (comment \"{}\")", u.comment));
            parts.push("                    (group \"users\")".into());
            parts.push(format!(
                "                    (supplementary-groups '({})))",
                u.groups
                    .iter()
                    .map(|g| format!("\"{g}\""))
                    .collect::<Vec<_>>()
                    .join(" ")
            ));
        }
        parts.push("                 )".into());
        parts.push("                %base-user-accounts))".into());
    }

    parts.join("\n")
}

fn render_packages(config: &SystemConfig) -> Option<String> {
    match &config.mode {
        // Desktop overrides the packages inherited from %os-base with
        // %os-desktop-packages, which adds the graphical Guix frontend (guix-gui).
        // Headless inherits %os-base-packages via (inherit %os-base), so no line.
        // Hardware-token / blueman extras the old %panther-desktop-packages
        // bundled can be re-added via Advanced > Edit system.scm.
        InstallMode::Panther if config.desktop.is_some() => {
            Some("  (packages %os-desktop-packages)".into())
        }
        InstallMode::Panther => None,
        _ => Some("  (packages %base-packages)".into()),
    }
}

fn render_services(config: &SystemConfig) -> String {
    match &config.mode {
        InstallMode::Panther => render_panther_services(config),
        InstallMode::Nonguix => render_nonguix_services(config),
        _ => render_standard_services(config),
    }
}

fn render_nonguix_services(config: &SystemConfig) -> String {
    // Same base service list as the standard path, but wrapped in
    // modify-services so the guix-service-type instance already present in
    // %base-services/%desktop-services is rewritten to build the system's guix
    // from the nonguix channel and authorize its substitute server + key. This
    // gives the installed system nonguix provenance (`guix describe`) and usable
    // channel modules without a manual `guix pull`, at the cost of building
    // guix-for-channels during `guix system init`.
    let mut service_list: Vec<String> = Vec::new();

    let base = if config.desktop.is_some() {
        "%desktop-services"
    } else {
        service_list.push("(service dhcpcd-service-type)".into());
        "%base-services"
    };

    if let Some(de) = &config.desktop {
        service_list.push(desktop_service(de));
    }

    service_list.push("(service openssh-service-type)".into());

    let svcs = service_list
        .iter()
        .map(|s| format!("                            {s}"))
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "  (services\n\
         \x20  (modify-services (append (list\n\
         {svcs})\n\
         \x20                           {base})\n\
         \x20    (guix-service-type config =>\n\
         \x20      (guix-configuration\n\
         \x20        (inherit config)\n\
         \x20        (guix (guix-for-channels %nonguix-channels))\n\
         \x20        (channels %nonguix-channels)\n\
         \x20        (substitute-urls (cons \"https://substitutes.nonguix.org\"\n\
         \x20                               %default-substitute-urls))\n\
         \x20        (authorized-keys (cons %nonguix-substitute-key\n\
         \x20                               %default-authorized-guix-keys))))))"
    )
}

fn render_panther_services(config: &SystemConfig) -> String {
    // %os-{base,desktop}-services from (px system os) — both wrap the stock
    // Guix bundles with Panther's daemon config (channels + substitute keys).
    // %os-desktop-services already pulls in NetworkManager via %desktop-services.
    // Headless still needs an explicit DHCP client to satisfy
    // openssh-service-type's `networking` requirement.
    let base = if config.desktop.is_some() {
        "%os-desktop-services"
    } else {
        "%os-base-services"
    };

    let mut service_list: Vec<String> = Vec::new();
    if config.desktop.is_none() {
        service_list.push("(service dhcpcd-service-type)".into());
    }
    service_list.push("(service openssh-service-type)".into());

    if let Some(de) = &config.desktop {
        service_list.push(desktop_service(de));
    }

    if service_list.is_empty() {
        format!("  (services {base})")
    } else {
        let svcs = service_list
            .iter()
            .map(|s| format!("                          {s}"))
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "  (services (append (list\n\
             {svcs})\n\
             \x20                   {base}))"
        )
    }
}

fn render_standard_services(config: &SystemConfig) -> String {
    let mut service_list: Vec<String> = Vec::new();

    // openssh-service-type requires the `networking` shepherd dependency.
    // `%desktop-services` bundles NetworkManager + wpa-supplicant + DBus etc.
    // for desktops; `%base-services` does not, so we add `dhcpcd-service-type`
    // for headless installs. Mirrors what the upstream Guix installer does.
    let base = if config.desktop.is_some() {
        "%desktop-services"
    } else {
        service_list.push("(service dhcpcd-service-type)".into());
        "%base-services"
    };

    if let Some(de) = &config.desktop {
        service_list.push(desktop_service(de));
    }

    service_list.push("(service openssh-service-type)".into());

    let svcs = service_list
        .iter()
        .map(|s| format!("                          {s}"))
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "  (services (append (list\n\
         {svcs})\n\
         \x20                   {base}))"
    )
}

fn desktop_service(de: &DesktopEnvironment) -> String {
    let svc = match de {
        DesktopEnvironment::Gnome => "gnome-desktop-service-type",
        DesktopEnvironment::Kde => "plasma-desktop-service-type",
        DesktopEnvironment::Xfce => "xfce-desktop-service-type",
        DesktopEnvironment::Mate => "mate-desktop-service-type",
        DesktopEnvironment::Sway => "sway-service-type",
        DesktopEnvironment::I3 => "i3-service-type",
        DesktopEnvironment::Lxqt => "lxqt-desktop-service-type",
    };
    format!("(service {svc})")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_partition_path_sata() {
        assert_eq!(partition_path("/dev/sda", 1), "/dev/sda1");
        assert_eq!(partition_path("/dev/sda", 2), "/dev/sda2");
    }

    #[test]
    fn test_partition_path_nvme() {
        assert_eq!(partition_path("/dev/nvme0n1", 1), "/dev/nvme0n1p1");
        assert_eq!(partition_path("/dev/nvme0n1", 2), "/dev/nvme0n1p2");
    }
}
