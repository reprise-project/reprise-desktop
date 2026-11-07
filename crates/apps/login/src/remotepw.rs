//! The Remote Desktop passwords as NT hashes, in a root-only file of `user uid HEX` lines.

use std::io;
use std::path::Path;

use zeroize::Zeroizing;

use crate::atomic;
use crate::daemon::StoredPassword;

pub const STORE: &str = "/var/lib/reprise-login/remote-passwords";

/// The passwords in `path`; none when it is missing or unreadable. A malformed line is skipped
/// and logged by its number only, as it may hold a typed password.
pub fn load(path: &Path) -> Vec<StoredPassword> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => Zeroizing::new(text),
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Vec::new(),
        Err(e) => {
            log::warn!("{}: {e}", path.display());
            return Vec::new();
        }
    };
    let mut all = Vec::new();
    for (n, line) in text.lines().enumerate() {
        if line.is_empty() {
            continue;
        }
        match parse(line) {
            Some(p) => all.push(p),
            None => log::warn!("{}: line {} is malformed; skipped", path.display(), n + 1),
        }
    }
    all
}

fn parse(line: &str) -> Option<StoredPassword> {
    let mut fields = line.split(' ');
    let (user, uid, hash) = (fields.next()?, fields.next()?, fields.next()?);
    let hex = |b: u8| b.is_ascii_digit() || (b'A'..=b'F').contains(&b);
    if fields.next().is_some() || user.is_empty() || hash.len() != 32 || !hash.bytes().all(hex) {
        return None;
    }
    Some(StoredPassword {
        user: user.into(),
        uid: uid.parse().ok()?,
        nt_hash: Zeroizing::new(hash.into()),
    })
}

/// Replaces `path` with `all`, the file 0600 in a 0700 directory.
pub fn save(path: &Path, all: &[StoredPassword]) -> io::Result<()> {
    let mut text = Zeroizing::new(String::new());
    for p in all {
        text.push_str(&format!("{} {} ", p.user, p.uid));
        text.push_str(&p.nt_hash);
        text.push('\n');
    }
    atomic::replace(path, text.as_bytes(), 0o700, 0o600)
}

/// The entries of `all` whose user still has that uid, as `uid_of` reads it; an entry
/// `uid_of` fails for is kept.
pub fn current(
    all: Vec<StoredPassword>,
    uid_of: impl Fn(&str) -> io::Result<Option<u32>>,
) -> Vec<StoredPassword> {
    let keep = |p: &StoredPassword| match uid_of(&p.user) {
        Ok(uid) if uid == Some(p.uid) => true,
        Ok(_) => {
            log::info!(
                "dropping {}'s Remote Desktop password: uid {} is no longer theirs",
                p.user,
                p.uid
            );
            false
        }
        Err(e) => {
            log::warn!(
                "looking up {}: {e}; keeping their Remote Desktop password",
                p.user
            );
            true
        }
    };
    all.into_iter().filter(keep).collect()
}

#[cfg(test)]
mod tests {
    use std::fs::Permissions;
    use std::os::unix::fs::PermissionsExt;

    use repriselogin::account::nt_hash;

    use super::*;

    fn stored(user: &str, uid: u32, password: &str) -> StoredPassword {
        StoredPassword {
            user: user.into(),
            uid,
            nt_hash: nt_hash(password),
        }
    }

    fn mode(path: &Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o7777
    }

    #[test]
    fn the_store_round_trips_and_is_owner_only() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("reprise-login");
        let path = dir.join("remote-passwords");
        let all = vec![
            stored("anna", 1001, "anna-remote"),
            stored("bert", 1002, "b"),
        ];
        save(&path, &all).unwrap();
        assert_eq!(load(&path), all);
        assert_eq!(mode(&dir), 0o700, "the directory");
        assert_eq!(mode(&path), 0o600, "the file");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            format!(
                "anna 1001 {}\nbert 1002 {}\n",
                *nt_hash("anna-remote"),
                *nt_hash("b")
            )
        );
        save(&path, &all[1..]).unwrap();
        assert_eq!(load(&path), all[1..]);
        let names: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["remote-passwords"]);
    }

    #[test]
    fn a_malformed_line_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("remote-passwords");
        let hash = nt_hash("password");
        let lower = hash.to_lowercase();
        let short = &hash[..31];
        std::fs::write(
            &path,
            format!(
                "anna 1001 {}\n\
                 bert 1002\n\
                 carl x {}\n\
                 dora 1004 {lower}\n\
                 emil 1005 {short}\n\
                 fred 1006 {} extra\n\
                 \n\
                 gina 1007 {}\n",
                *hash, *hash, *hash, *hash
            ),
        )
        .unwrap();
        assert_eq!(
            load(&path),
            vec![
                stored("anna", 1001, "password"),
                stored("gina", 1007, "password")
            ]
        );
        assert_eq!(load(&dir.path().join("missing")), vec![]);
    }

    #[test]
    fn a_planted_tmp_symlink_is_replaced_and_its_target_kept() {
        let dir = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let victim = elsewhere.path().join("victim");
        std::fs::write(&victim, "victim").unwrap();
        std::fs::set_permissions(&victim, Permissions::from_mode(0o644)).unwrap();
        let path = dir.path().join("remote-passwords");
        let tmp = dir.path().join("remote-passwords.tmp");
        std::os::unix::fs::symlink(&victim, &tmp).unwrap();
        let all = vec![stored("anna", 1001, "anna-remote")];
        save(&path, &all).unwrap();
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "victim");
        assert_eq!(mode(&victim), 0o644);
        assert_eq!(load(&path), all);
        assert_eq!(mode(&path), 0o600);
        assert!(std::fs::symlink_metadata(&tmp).is_err(), "no tmp is left");
    }

    #[test]
    fn a_failed_save_leaves_no_tmp() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("remote-passwords");
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("keep"), "").unwrap();
        assert!(save(&path, &[stored("anna", 1001, "anna-remote")]).is_err());
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["remote-passwords"]);
    }

    #[test]
    fn entries_whose_uid_no_longer_names_the_user_are_dropped() {
        let all = vec![
            stored("anna", 1001, "a"),
            stored("bert", 1002, "b"),
            stored("carl", 1003, "c"),
            stored("dora", 1004, "d"),
        ];
        let kept = current(all.clone(), |user| match user {
            "anna" => Ok(Some(1001)),
            "bert" => Ok(Some(1010)),
            "carl" => Ok(None),
            _ => Err(io::Error::other("NSS is down")),
        });
        assert_eq!(kept, vec![all[0].clone(), all[3].clone()]);
    }
}
