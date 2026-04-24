use crate::config::{DesktopEnvironment, Filesystem, Firmware, SystemConfig};
use crate::disk::partition_path;
use crate::mode::InstallMode;

pub fn render_operating_system(config: &SystemConfig) -> String {
    if matches!(config.mode, InstallMode::Enterprise { .. }) {
        return String::new();
    }

    let sections: Vec<String> = vec![
        render_use_modules(config),
        String::new(),
        render_os_form(config),
    ];

    sections.join("\n")
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
            lines.push("(use-service-modules networking ssh desktop)".into());
            lines.push("(use-modules (nongnu packages linux))".into());
            lines.push("(use-modules (nongnu system linux-initrd))".into());
        }
        InstallMode::Panther => {
            lines.push("(use-modules (gnu))".into());
            lines.push("(use-service-modules networking ssh desktop)".into());
            lines.push("(use-modules (px system panther))".into());
        }
        InstallMode::Enterprise { .. } => unreachable!(),
    }

    lines.join("\n")
}

fn render_os_form(config: &SystemConfig) -> String {
    let mut parts: Vec<String> = Vec::new();

    let inherit = render_inherit(config);
    let has_inherit = inherit.is_some();

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

    if !has_inherit {
        parts.push(String::new());
        parts.push(render_packages(config));
    }

    parts.push(String::new());
    parts.push(render_services(config));

    parts.push(")".into());

    parts.join("\n")
}

fn render_inherit(config: &SystemConfig) -> Option<String> {
    match &config.mode {
        InstallMode::Panther => {
            if config.desktop.is_some() {
                Some("(inherit %panther-desktop-os)".into())
            } else {
                Some("(inherit %panther-os)".into())
            }
        }
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

fn render_packages(config: &SystemConfig) -> String {
    match &config.mode {
        InstallMode::Panther => {
            if config.desktop.is_some() {
                "  (packages %panther-desktop-packages)".into()
            } else {
                "  (packages %panther-base-packages)".into()
            }
        }
        _ => "  (packages %base-packages)".into(),
    }
}

fn render_services(config: &SystemConfig) -> String {
    match &config.mode {
        InstallMode::Panther => render_panther_services(config),
        _ => render_standard_services(config),
    }
}

fn render_panther_services(config: &SystemConfig) -> String {
    let base = if config.desktop.is_some() {
        "%panther-desktop-services"
    } else {
        "%panther-base-services"
    };

    let mut service_list: Vec<String> = Vec::new();
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
         \x20                   %base-services))"
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
