use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail, ensure};
use sha_crypt::{PasswordHasher, ShaCrypt};
use zeroize::Zeroizing;

/// Set the shadow password for `user` in `<root>/etc/shadow`.
///
/// Generates a SHA-512 crypt (`$6$...`) hash with default parameters and a
/// fresh random salt (via OsRng). The password is held in `Zeroizing<String>`
/// so its bytes are wiped on drop (the plaintext never touches disk).
///
/// The shadow file is updated atomically: a sibling file is written with
/// mode 0600, fsync'd, and `rename(2)`'d over the original. The parent
/// directory is fsync'd so the rename survives a power loss.
///
/// The `lastchange` field (days since epoch) is updated to today.
pub fn set_shadow_password(root: &Path, user: &str, password: Zeroizing<String>) -> Result<()> {
    let hash = ShaCrypt::default()
        .hash_password(password.as_bytes())
        .map_err(|e| anyhow!("sha512 hash: {e:?}"))?;
    let hash: &str = hash.as_ref();

    let days_since_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system time before UNIX epoch")?
        .as_secs()
        / 86_400;

    let shadow_path = root.join("etc/shadow");
    let original = fs::read_to_string(&shadow_path)
        .with_context(|| format!("read {}", shadow_path.display()))?;
    let original_meta =
        fs::metadata(&shadow_path).with_context(|| format!("stat {}", shadow_path.display()))?;

    let updated = rewrite_shadow(&original, user, hash, days_since_epoch)?;
    atomic_replace(&shadow_path, &updated, original_meta.permissions().mode())
}

/// Replace the password and lastchange fields for `user` in a shadow file body.
///
/// The shadow line format is:
///   `name:passwd:lastchange:min:max:warn:inactive:expire:reserved`
///
/// Only fields 2 (passwd) and 3 (lastchange) are modified. Errors if the user
/// is not found or their line is malformed.
fn rewrite_shadow(body: &str, user: &str, hash: &str, days: u64) -> Result<String> {
    let mut out = String::with_capacity(body.len() + hash.len());
    let mut found = false;
    let days_str = days.to_string();

    for line in body.lines() {
        // splitn(9) keeps the trailing reserved field intact even if it contains colons
        let fields: Vec<&str> = line.splitn(9, ':').collect();
        if fields.first() == Some(&user) {
            ensure!(
                fields.len() == 9,
                "malformed shadow line for user {user}: expected 9 fields, got {}",
                fields.len()
            );
            let mut updated = fields.clone();
            updated[1] = hash;
            updated[2] = days_str.as_str();
            out.push_str(&updated.join(":"));
            found = true;
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }

    if !found {
        bail!("user {user} not found in shadow file");
    }
    Ok(out)
}

/// Atomically replace `path` with `contents`, preserving `mode`.
///
/// Writes to a sibling `.new` file (mode 0600 during construction, then
/// adjusted to `mode` before rename), fsyncs the file, renames into place,
/// and fsyncs the parent directory.
fn atomic_replace(path: &Path, contents: &str, mode: u32) -> Result<()> {
    let tmp_path: PathBuf = {
        let mut p = path.as_os_str().to_os_string();
        p.push(".new");
        p.into()
    };

    {
        let mut f = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp_path)
            .with_context(|| format!("create {}", tmp_path.display()))?;
        f.write_all(contents.as_bytes())
            .with_context(|| format!("write {}", tmp_path.display()))?;
        f.sync_all()
            .with_context(|| format!("fsync {}", tmp_path.display()))?;
    }

    fs::set_permissions(&tmp_path, fs::Permissions::from_mode(mode))
        .with_context(|| format!("chmod {:o} {}", mode, tmp_path.display()))?;
    fs::rename(&tmp_path, path)
        .with_context(|| format!("rename {} -> {}", tmp_path.display(), path.display()))?;

    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("shadow path has no parent: {}", path.display()))?;
    let dir = fs::File::open(parent).with_context(|| format!("open {}", parent.display()))?;
    dir.sync_all()
        .with_context(|| format!("fsync dir {}", parent.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "root:!:19000:0:99999:7:::
panther:!:19000:0:99999:7:::
daemon:*:19000:0:99999:7:::
";

    #[test]
    fn rewrite_updates_target_user() {
        let out = rewrite_shadow(SAMPLE, "panther", "$6$abc$def", 19_500).unwrap();
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], "root:!:19000:0:99999:7:::");
        assert_eq!(lines[1], "panther:$6$abc$def:19500:0:99999:7:::");
        assert_eq!(lines[2], "daemon:*:19000:0:99999:7:::");
    }

    #[test]
    fn rewrite_missing_user_errors() {
        let err = rewrite_shadow(SAMPLE, "ghost", "$6$a$b", 19_500).unwrap_err();
        assert!(err.to_string().contains("ghost"));
    }

    #[test]
    fn rewrite_malformed_line_errors() {
        let bad = "panther:!:19000\n";
        let err = rewrite_shadow(bad, "panther", "$6$a$b", 19_500).unwrap_err();
        assert!(err.to_string().contains("malformed"));
    }

    #[test]
    fn rewrite_preserves_other_fields() {
        // User has aging fields set (min=1, max=60, warn=14)
        let body = "alice:!:19000:1:60:14:7:19999:\n";
        let out = rewrite_shadow(body, "alice", "$6$x$y", 19_500).unwrap();
        assert_eq!(out, "alice:$6$x$y:19500:1:60:14:7:19999:\n");
    }

    #[test]
    fn atomic_replace_changes_file() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("shadow");
        fs::write(&f, "old\n").unwrap();
        atomic_replace(&f, "new\n", 0o640).unwrap();
        assert_eq!(fs::read_to_string(&f).unwrap(), "new\n");
        assert_eq!(
            fs::metadata(&f).unwrap().permissions().mode() & 0o777,
            0o640
        );
    }

    #[test]
    fn set_shadow_password_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("etc")).unwrap();
        let shadow = tmp.path().join("etc/shadow");
        fs::write(&shadow, SAMPLE).unwrap();
        fs::set_permissions(&shadow, fs::Permissions::from_mode(0o600)).unwrap();

        let password = Zeroizing::new("hunter2".to_string());
        set_shadow_password(tmp.path(), "panther", password).unwrap();

        let content = fs::read_to_string(&shadow).unwrap();
        let panther_line = content
            .lines()
            .find(|l| l.starts_with("panther:"))
            .expect("panther line present");
        let fields: Vec<&str> = panther_line.splitn(9, ':').collect();
        assert!(
            fields[1].starts_with("$6$"),
            "expected SHA-512 crypt hash, got: {}",
            fields[1]
        );
        // lastchange was updated from 19000
        assert_ne!(fields[2], "19000");
        assert_eq!(
            fs::metadata(&shadow).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
