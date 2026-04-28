use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use pwhash::sha512_crypt;
use zeroize::Zeroizing;

use crate::config::UserAccount;

/// Seed `<root>/etc/shadow` with one line per user before `guix system init`.
///
/// `guix system init` does not create `/etc/{passwd,shadow,group}` — those
/// are populated by activation at first boot. Activation's `user+group-databases`
/// (`gnu/build/accounts.scm`) reads any pre-existing shadow file and reuses
/// matching entries by name (`passwd->shadow`), preserving the password hash.
/// By seeding shadow now, the user can log in after first boot without a
/// post-install passwd dance.
///
/// The same SHA-512 crypt hash is applied to every user (single password for
/// the install). The hash never lands in `system.scm` or the store; the
/// plaintext is held in `Zeroizing` so it's wiped on drop.
pub fn seed_shadow(root: &Path, users: &[UserAccount], password: &Zeroizing<String>) -> Result<()> {
    // pwhash emits glibc-compatible $6$<16-char salt>$<digest> output.
    // sha-crypt 0.6 produced 22-char salts that crypt(3) silently truncates,
    // making the stored hash unverifiable on login.
    let hash = sha512_crypt::hash(password.as_str()).map_err(|e| anyhow!("sha512 hash: {e}"))?;

    // libc's `isexpired` treats a lastchange of 0 as "expired" — clamp to 1.
    let last_change = (SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system time before UNIX epoch")?
        .as_secs()
        / 86_400)
        .max(1);

    let mut body = String::new();
    for user in users {
        body.push_str(&format!(
            "{name}:{hash}:{last_change}:0:99999:7:::\n",
            name = user.name,
        ));
    }

    let etc = root.join("etc");
    fs::create_dir_all(&etc).with_context(|| format!("mkdir {}", etc.display()))?;

    let shadow_path = etc.join("shadow");
    atomic_write(&shadow_path, body.as_bytes(), 0o600)
}

/// Atomically write `contents` to `path` with `mode`.
///
/// Writes to a sibling `.new` file (mode 0600 during construction, then
/// adjusted to `mode` before rename), fsyncs the file, renames into place,
/// and fsyncs the parent directory so the rename survives a power loss.
fn atomic_write(path: &Path, contents: &[u8], mode: u32) -> Result<()> {
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
        f.write_all(contents)
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

    fn user(name: &str) -> UserAccount {
        UserAccount {
            name: name.into(),
            comment: format!("{name}'s account"),
            groups: vec!["wheel".into()],
        }
    }

    #[test]
    fn seed_shadow_writes_one_line_per_user() {
        let tmp = tempfile::tempdir().unwrap();
        let pw = Zeroizing::new("hunter2".to_string());
        seed_shadow(tmp.path(), &[user("alice"), user("bob")], &pw).unwrap();

        let content = fs::read_to_string(tmp.path().join("etc/shadow")).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("alice:$6$"));
        assert!(lines[1].starts_with("bob:$6$"));
    }

    #[test]
    fn seed_shadow_uses_sha512_crypt() {
        let tmp = tempfile::tempdir().unwrap();
        let pw = Zeroizing::new("hunter2".to_string());
        seed_shadow(tmp.path(), &[user("alice")], &pw).unwrap();

        let content = fs::read_to_string(tmp.path().join("etc/shadow")).unwrap();
        let line = content.lines().next().unwrap();
        let fields: Vec<&str> = line.split(':').collect();
        assert_eq!(fields[0], "alice");
        assert!(fields[1].starts_with("$6$"));
        assert!(fields[2].parse::<u64>().unwrap() > 0);
    }

    /// Regression: sha-crypt 0.6 emitted 22-char salts that crypt(3) silently
    /// truncates to 16, making the stored hash unverifiable on login. The salt
    /// portion of a SHA-512 crypt hash must be at most 16 characters.
    #[test]
    fn seed_shadow_salt_within_crypt3_limit() {
        let tmp = tempfile::tempdir().unwrap();
        let pw = Zeroizing::new("hunter2".to_string());
        seed_shadow(tmp.path(), &[user("alice")], &pw).unwrap();

        let content = fs::read_to_string(tmp.path().join("etc/shadow")).unwrap();
        let hash = content.lines().next().unwrap().split(':').nth(1).unwrap();
        // Strip the optional "rounds=N$" segment if present.
        let after_id = hash.strip_prefix("$6$").expect("must be SHA-512 crypt");
        let salt_part = after_id
            .strip_prefix("rounds=")
            .map(|r| r.split_once('$').unwrap().1)
            .unwrap_or(after_id);
        let salt = salt_part.split_once('$').unwrap().0;
        assert!(
            salt.len() <= 16,
            "salt is {} chars, crypt(3) only honors first 16: {salt:?}",
            salt.len()
        );
    }

    #[test]
    fn seed_shadow_round_trips_via_pwhash_verify() {
        let tmp = tempfile::tempdir().unwrap();
        let pw = Zeroizing::new("hunter2".to_string());
        seed_shadow(tmp.path(), &[user("alice")], &pw).unwrap();

        let content = fs::read_to_string(tmp.path().join("etc/shadow")).unwrap();
        let hash = content.lines().next().unwrap().split(':').nth(1).unwrap();
        assert!(pwhash::unix::verify("hunter2", hash));
        assert!(!pwhash::unix::verify("wrong", hash));
    }

    #[test]
    fn seed_shadow_file_mode_is_0600() {
        let tmp = tempfile::tempdir().unwrap();
        let pw = Zeroizing::new("hunter2".to_string());
        seed_shadow(tmp.path(), &[user("alice")], &pw).unwrap();

        let mode = fs::metadata(tmp.path().join("etc/shadow"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn seed_shadow_creates_etc_dir_if_missing() {
        let tmp = tempfile::tempdir().unwrap();
        // Note: no etc/ created beforehand.
        let pw = Zeroizing::new("hunter2".to_string());
        seed_shadow(tmp.path(), &[user("alice")], &pw).unwrap();

        assert!(tmp.path().join("etc/shadow").exists());
    }

    #[test]
    fn seed_shadow_overwrites_existing() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("etc")).unwrap();
        fs::write(tmp.path().join("etc/shadow"), "old:!:1:::::\n").unwrap();

        let pw = Zeroizing::new("hunter2".to_string());
        seed_shadow(tmp.path(), &[user("alice")], &pw).unwrap();

        let content = fs::read_to_string(tmp.path().join("etc/shadow")).unwrap();
        assert!(!content.contains("old:"));
        assert!(content.contains("alice:$6$"));
    }
}
