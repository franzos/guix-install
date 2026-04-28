use std::fs::{File, OpenOptions, Permissions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::exec;

/// A unit of work in a disk-related installation phase.
///
/// Each phase builds a list of `Action` values, and `install.rs` dispatches
/// each one via `execute()`. Cmd variants run external tools (parted, mkfs,
/// cryptsetup, mount, mkswap, etc.); native variants (Mkdir, CreateSwapFile,
/// SetPermissions) use `std::fs` + Unix APIs directly, avoiding subprocess
/// spawn overhead and shell-escaping surface for trivial operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Cmd(Vec<String>),
    Mkdir(PathBuf),
    CreateSwapFile {
        path: PathBuf,
        size_bytes: u64,
    },
    SetPermissions {
        path: PathBuf,
        mode: u32,
    },
    Mount {
        source: PathBuf,
        target: PathBuf,
        fstype: String,
    },
}

impl Action {
    pub fn cmd<S: AsRef<str>>(args: &[S]) -> Self {
        Action::Cmd(args.iter().map(|s| s.as_ref().to_string()).collect())
    }

    pub fn mkdir(path: impl Into<PathBuf>) -> Self {
        Action::Mkdir(path.into())
    }

    pub fn execute(&self) -> Result<()> {
        match self {
            Action::Cmd(args) => {
                let strs: Vec<&str> = args.iter().map(String::as_str).collect();
                exec::run_cmd(&strs)?;
                Ok(())
            }
            Action::Mkdir(path) => std::fs::create_dir_all(path)
                .with_context(|| format!("mkdir -p {}", path.display())),
            Action::CreateSwapFile { path, size_bytes } => write_swap_file(path, *size_bytes),
            Action::SetPermissions { path, mode } => {
                std::fs::set_permissions(path, Permissions::from_mode(*mode))
                    .with_context(|| format!("chmod {:o} {}", mode, path.display()))
            }
            Action::Mount {
                source,
                target,
                fstype,
            } => rustix::mount::mount(
                source.as_path(),
                target.as_path(),
                fstype.as_str(),
                rustix::mount::MountFlags::empty(),
                "",
            )
            .with_context(|| {
                format!(
                    "mount {} on {} ({})",
                    source.display(),
                    target.display(),
                    fstype
                )
            }),
        }
    }
}

/// Creates a fully-allocated file of `size_bytes`, suitable for `mkswap`.
///
/// Uses `open(O_CREAT | O_TRUNC, 0600)` so the file is never readable by
/// other users, and writes zeroed blocks in 1 MiB chunks. Sparse files
/// (via `set_len`) are not used: `mkswap` rejects swap files with holes.
fn write_swap_file(path: &PathBuf, size_bytes: u64) -> Result<()> {
    let mut file: File = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("create {}", path.display()))?;

    const CHUNK: usize = 1024 * 1024;
    let zeros = [0u8; CHUNK];
    let mut remaining = size_bytes;
    while remaining > 0 {
        let n = remaining.min(CHUNK as u64) as usize;
        file.write_all(&zeros[..n])
            .with_context(|| format!("write {}", path.display()))?;
        remaining -= n as u64;
    }
    file.sync_all()
        .with_context(|| format!("fsync {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cmd_constructor_from_slice() {
        let a = Action::cmd(&["mount", "LABEL=my-root", "/mnt"]);
        assert_eq!(
            a,
            Action::Cmd(vec!["mount".into(), "LABEL=my-root".into(), "/mnt".into()])
        );
    }

    #[test]
    fn mkdir_constructor() {
        let a = Action::mkdir("/mnt/etc/guix");
        assert_eq!(a, Action::Mkdir(PathBuf::from("/mnt/etc/guix")));
    }

    #[test]
    fn mkdir_executes() {
        let tmp = tempfile::tempdir().unwrap();
        let nested = tmp.path().join("a/b/c");
        Action::Mkdir(nested.clone()).execute().unwrap();
        assert!(nested.is_dir());
    }

    #[test]
    fn create_swap_file_executes() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("swapfile");
        Action::CreateSwapFile {
            path: path.clone(),
            size_bytes: 4 * 1024 * 1024,
        }
        .execute()
        .unwrap();

        let meta = std::fs::metadata(&path).unwrap();
        assert_eq!(meta.len(), 4 * 1024 * 1024);
        // Must be 0600 so mkswap is happy
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn mount_constructor_equality() {
        let a = Action::Mount {
            source: PathBuf::from("/dev/sda1"),
            target: PathBuf::from("/mnt/boot/efi"),
            fstype: "vfat".into(),
        };
        let b = Action::Mount {
            source: PathBuf::from("/dev/sda1"),
            target: PathBuf::from("/mnt/boot/efi"),
            fstype: "vfat".into(),
        };
        assert_eq!(a, b);
    }

    #[test]
    fn set_permissions_executes() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("f");
        std::fs::write(&path, b"x").unwrap();
        Action::SetPermissions {
            path: path.clone(),
            mode: 0o600,
        }
        .execute()
        .unwrap();
        let meta = std::fs::metadata(&path).unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
    }
}
