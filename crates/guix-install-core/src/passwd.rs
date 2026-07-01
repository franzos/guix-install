use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use rand::Rng;
use sha_crypt::{Params, sha512_crypt};
use zeroize::Zeroizing;

use crate::config::UserAccount;

/// crypt(3) base64 alphabet (note: not standard base64), least-significant 6
/// bits first.
const CRYPT64: &[u8; 64] = b"./0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

/// Generate a 16-character salt from the crypt(3) alphabet. 16 is the maximum
/// crypt(3) honors for `$6$`; going over is silently truncated on login, which
/// is the bug the old sha-crypt `sha512_simple` path hit (22-char salts).
fn gen_salt() -> String {
    let mut rng = rand::thread_rng();
    (0..16)
        .map(|_| CRYPT64[rng.gen_range(0..CRYPT64.len())] as char)
        .collect()
}

/// Emit `n` crypt(3) base64 characters from the 24-bit value `(b2<<16)|(b1<<8)|b0`.
fn b64_from_24bit(out: &mut String, b2: u8, b1: u8, b0: u8, n: usize) {
    let mut w = (u32::from(b2) << 16) | (u32::from(b1) << 8) | u32::from(b0);
    for _ in 0..n {
        out.push(CRYPT64[(w & 0x3f) as usize] as char);
        w >>= 6;
    }
}

/// Encode a 64-byte SHA-512 digest into the 86-char crypt(3) hash field.
///
/// The byte permutation is glibc's (`sha512-crypt.c`); it is not a plain
/// base64 of the digest.
fn sha512_crypt_b64(d: &[u8; 64]) -> String {
    // Each triple picks three digest bytes in glibc's fixed order.
    const TRIPLES: [(usize, usize, usize); 21] = [
        (0, 21, 42),
        (22, 43, 1),
        (44, 2, 23),
        (3, 24, 45),
        (25, 46, 4),
        (47, 5, 26),
        (6, 27, 48),
        (28, 49, 7),
        (50, 8, 29),
        (9, 30, 51),
        (31, 52, 10),
        (53, 11, 32),
        (12, 33, 54),
        (34, 55, 13),
        (56, 14, 35),
        (15, 36, 57),
        (37, 58, 16),
        (59, 17, 38),
        (18, 39, 60),
        (40, 61, 19),
        (62, 20, 41),
    ];
    let mut out = String::with_capacity(86);
    for (a, b, c) in TRIPLES {
        b64_from_24bit(&mut out, d[a], d[b], d[c], 4);
    }
    b64_from_24bit(&mut out, 0, 0, d[63], 2);
    out
}

/// Produce a glibc-compatible `$6$<salt>$<hash>` SHA-512 crypt string for the
/// given password and 16-char salt, using the crypt(3) default of 5000 rounds
/// (so no `rounds=` segment is emitted).
fn crypt_sha512(password: &str, salt: &str) -> String {
    let digest = sha512_crypt(password.as_bytes(), salt.as_bytes(), Params::default());
    format!("$6${salt}${}", sha512_crypt_b64(&digest))
}

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
    // sha-crypt's low-level digest + our own 16-char salt: crypt(3) only honors
    // the first 16 salt chars for $6$, so we cap it ourselves rather than let a
    // longer generated salt get silently truncated (unverifiable on login).
    let hash = crypt_sha512(password.as_str(), &gen_salt());

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

    /// Known-answer test: our `$6$` output must byte-match an independent
    /// implementation for the same password and salt. Ground truth generated
    /// with `openssl passwd -6 -salt abcdefghijklmnop hunter2` (crypt(3)
    /// default of 5000 rounds).
    #[test]
    fn crypt_sha512_matches_openssl_known_answer() {
        let expected = "$6$abcdefghijklmnop$EC.xeLW9zNWcX0r23FSpQaV7PG.Ibd4QnLe3w6UC47i3/vkPQouEDwvUpGtqFiad5mzQG96cD/LywQiXv9WfH/";
        assert_eq!(crypt_sha512("hunter2", "abcdefghijklmnop"), expected);
    }

    #[test]
    fn crypt_sha512_hash_field_is_86_chars() {
        let hash = crypt_sha512("hunter2", "abcdefghijklmnop");
        let field = hash.rsplit_once('$').unwrap().1;
        assert_eq!(field.len(), 86);
    }

    #[test]
    fn seed_shadow_generates_a_fresh_salt_each_time() {
        let tmp = tempfile::tempdir().unwrap();
        let pw = Zeroizing::new("hunter2".to_string());
        seed_shadow(tmp.path(), &[user("alice")], &pw).unwrap();
        let first = fs::read_to_string(tmp.path().join("etc/shadow")).unwrap();
        seed_shadow(tmp.path(), &[user("alice")], &pw).unwrap();
        let second = fs::read_to_string(tmp.path().join("etc/shadow")).unwrap();
        assert_ne!(first, second, "random salt should differ between runs");
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
