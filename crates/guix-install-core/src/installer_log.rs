//! Append-only log fanned out to one or more file sinks.
//!
//! Two sinks are used over an install. A live sink at [`LIVE_LOG_PATH`] is
//! opened at the start of the interview, so wizard activity (network, channel,
//! config errors) is captured before any disk work. A target sink at
//! [`TARGET_LOG_PATH`] is opened once the install root is mounted; it survives
//! the install reboot and is a useful artifact for bug reports.
//!
//! UI lines (`info`/`warn`/`error`) and shell-out invocations from `exec` are
//! mirrored to every open sink. Calls made before any [`open`] are dropped.

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};

/// Live sink on the ISO overlay, opened at the start of the interview.
pub const LIVE_LOG_PATH: &str = "/var/log/guix-install.log";
/// Target sink under `/mnt`, opened once the install root is mounted.
pub const TARGET_LOG_PATH: &str = "/mnt/var/log/guix-install.log";

static LOG: Mutex<Vec<(PathBuf, BufWriter<File>)>> = Mutex::new(Vec::new());

/// Opens the log file (0600), creating its parent directory if needed.
///
/// The 0600 mode matters: this file ends up at `/mnt/var/log/guix-install.log`
/// and survives the install reboot. It contains argv lines and command stderr,
/// which on a multi-user system shouldn't be readable to unprivileged accounts.
pub fn open(path: &Path) -> Result<()> {
    if let Ok(guard) = LOG.lock()
        && guard.iter().any(|(p, _)| p == path)
    {
        return Ok(());
    }
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
        if guard.iter().any(|(p, _)| p == path) {
            return Ok(());
        }
        guard.push((path.to_path_buf(), BufWriter::new(file)));
    }
    write_line("session:", "log opened");
    Ok(())
}

/// Flushes and drops all log sinks. Idempotent. Tolerates a poisoned mutex by
/// silently dropping the handles rather than panicking.
pub fn close() {
    let Ok(mut guard) = LOG.lock() else { return };
    for w in guard.iter_mut() {
        let _ = writeln!(w.1, "[{}] session: log closed", unix_seconds());
        let _ = w.1.flush();
    }
    guard.clear();
}

/// Appends a single timestamped line to every open sink. No-op if none are open.
pub fn write_line(prefix: &str, msg: &str) {
    let Ok(mut guard) = LOG.lock() else { return };
    for w in guard.iter_mut() {
        let _ = writeln!(w.1, "[{}] {prefix} {msg}", unix_seconds());
        let _ = w.1.flush();
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

    #[test]
    fn two_sinks_both_receive_writes() {
        let _g = test_lock();

        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a.log");
        let b = tmp.path().join("b.log");
        open(&a).unwrap();
        open(&b).unwrap();
        write_line("phase:", "fanout");
        close();

        let ba = std::fs::read_to_string(&a).unwrap();
        let bb = std::fs::read_to_string(&b).unwrap();
        assert!(ba.contains("phase: fanout"), "sink a missing line: {ba}");
        assert!(bb.contains("phase: fanout"), "sink b missing line: {bb}");
    }

    #[test]
    fn open_same_path_twice_is_one_sink() {
        let _g = test_lock();

        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("dup.log");
        open(&path).unwrap();
        open(&path).unwrap();
        write_line("phase:", "once");
        close();

        let body = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            body.matches("session: log opened").count(),
            1,
            "log opened should appear once: {body}"
        );
        assert_eq!(
            body.matches("phase: once").count(),
            1,
            "single sink should write the line once: {body}"
        );
    }
}
