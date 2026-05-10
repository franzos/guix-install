//! Append-only log mirrored to a file on the target filesystem.
//!
//! Once `phase_mount` has populated `/mnt`, [`open`] is called with
//! `/mnt/var/log/guix-install.log` so subsequent UI lines (`info`/`warn`/`error`)
//! and shell-out invocations from `exec` are recorded. The file survives the
//! install reboot and is a useful artifact for bug reports.
//!
//! Calls before [`open`] are silently dropped — the live ISO has no obvious
//! place to put them, and stderr is already shown to the user.

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::sync::Mutex;

use anyhow::{Context, Result};

static LOG: Mutex<Option<BufWriter<File>>> = Mutex::new(None);

/// Opens the log file (0600), creating its parent directory if needed.
///
/// The 0600 mode matters: this file ends up at `/mnt/var/log/guix-install.log`
/// and survives the install reboot. It contains argv lines and command stderr,
/// which on a multi-user system shouldn't be readable to unprivileged accounts.
pub fn open(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create log dir {}", parent.display()))?;
    }
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("open log {}", path.display()))?;
    if let Ok(mut guard) = LOG.lock() {
        *guard = Some(BufWriter::new(file));
    }
    write_line("session:", "log opened");
    Ok(())
}

/// Flushes and drops the log file handle. Idempotent. Tolerates a poisoned
/// mutex by silently dropping the handle rather than panicking.
pub fn close() {
    let Ok(mut guard) = LOG.lock() else { return };
    if let Some(mut w) = guard.take() {
        let _ = writeln!(w, "[{}] session: log closed", unix_seconds());
        let _ = w.flush();
    }
}

/// Appends a single timestamped line. No-op if the file isn't open.
pub fn write_line(prefix: &str, msg: &str) {
    let Ok(mut guard) = LOG.lock() else { return };
    if let Some(w) = guard.as_mut() {
        let _ = writeln!(w, "[{}] {prefix} {msg}", unix_seconds());
        let _ = w.flush();
    }
}

fn unix_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_before_open_is_noop() {
        // Sanity: must not panic if open was never called.
        write_line("test:", "no-op");
    }

    /// All open/write/close tests share the static `LOG` and must serialize.
    /// Cargo runs tests in parallel within a single binary; without this guard
    /// two open() calls race and corrupt each other's BufWriter handle.
    fn test_lock() -> std::sync::MutexGuard<'static, ()> {
        static GUARD: Mutex<()> = Mutex::new(());
        GUARD.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn open_write_close_and_mode() {
        use std::os::unix::fs::PermissionsExt;
        let _g = test_lock();

        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("install.log");
        open(&path).unwrap();
        write_line("phase:", "hello");
        close();

        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.contains("phase: hello"), "missing phase line: {body}");
        assert!(body.contains("session: log opened"));
        assert!(body.contains("session: log closed"));

        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "log must be 0600 (got {mode:o})");
    }
}
