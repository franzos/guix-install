//! Flat btrfs subvolume layout used whenever the root filesystem is btrfs.
//!
//! Root snapshots then exclude user data (`@home`), the store (`@gnu`, so
//! `guix gc` keeps freeing space while snapshots are retained), and the swap
//! file (`@swap`, since btrfs refuses to snapshot a subvolume holding an active
//! swap file).

/// A btrfs subvolume and where it is mounted in the installed system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Subvolume {
    pub name: &'static str,
    pub mount_point: &'static str,
}

/// Subvolumes in mount order: root first, so the others nest under it.
pub const SUBVOLUMES: [Subvolume; 4] = [
    Subvolume {
        name: "@",
        mount_point: "/",
    },
    Subvolume {
        name: "@home",
        mount_point: "/home",
    },
    Subvolume {
        name: "@gnu",
        mount_point: "/gnu",
    },
    Subvolume {
        name: "@swap",
        mount_point: "/swap",
    },
];

/// Mount options shared by every subvolume. Compression is a per-filesystem
/// setting on btrfs, so listing it on each mount just keeps the config honest.
pub const MOUNT_OPTIONS: &str = "compress=zstd";

/// Mount point of the swap subvolume, relative to the installed root.
pub const SWAP_MOUNT_POINT: &str = "/swap";

impl Subvolume {
    /// Full `-o` string for mounting this subvolume.
    pub fn mount_options(&self) -> String {
        format!("subvol={},{MOUNT_OPTIONS}", self.name)
    }

    /// Mount point under the installer's target directory.
    pub fn target_path(&self, target_dir: &str) -> String {
        if self.mount_point == "/" {
            target_dir.to_string()
        } else {
            format!("{target_dir}{}", self.mount_point)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_subvolume_is_first() {
        assert_eq!(SUBVOLUMES[0].mount_point, "/");
        assert_eq!(SUBVOLUMES[0].name, "@");
    }

    #[test]
    fn mount_options_include_subvol_and_compression() {
        assert_eq!(SUBVOLUMES[1].mount_options(), "subvol=@home,compress=zstd");
    }

    #[test]
    fn target_path_joins_with_target_dir() {
        assert_eq!(SUBVOLUMES[0].target_path("/mnt"), "/mnt");
        assert_eq!(SUBVOLUMES[2].target_path("/mnt"), "/mnt/gnu");
    }

    #[test]
    fn swap_subvolume_matches_constant() {
        let swap = SUBVOLUMES.iter().find(|s| s.name == "@swap").unwrap();
        assert_eq!(swap.mount_point, SWAP_MOUNT_POINT);
    }
}
