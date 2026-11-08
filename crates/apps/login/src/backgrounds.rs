//! The ids of the background sessions, kept in the runtime directory across restarts of the
//! daemon so a restart treats each session as the last run did.

use std::collections::BTreeSet;
use std::io;
use std::path::Path;

use crate::atomic;

pub const BACKGROUNDS: &str = "/run/reprise-login/background-sessions";

/// The ids `path` holds, one a line; None when there is no readable file.
pub fn load(path: &Path) -> Option<BTreeSet<String>> {
    match std::fs::read_to_string(path) {
        Ok(text) => Some(
            text.lines()
                .filter(|l| !l.is_empty())
                .map(Into::into)
                .collect(),
        ),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => {
            log::warn!("{}: {e}", path.display());
            None
        }
    }
}

pub fn save(path: &Path, ids: &BTreeSet<String>) -> io::Result<()> {
    let text: String = ids.iter().map(|id| format!("{id}\n")).collect();
    atomic::replace(path, text.as_bytes(), 0o755, 0o600)
}

/// The ids last written, and whether the last write failed.
#[derive(Default)]
pub struct Saved {
    ids: Option<BTreeSet<String>>,
    failing: bool,
}

impl Saved {
    /// Writes `ids` to `path` unless they were the last written. A failed write removes the
    /// file, so a restart falls back to the rule for no file, and the next call tries again.
    pub fn sync(&mut self, path: &Path, ids: BTreeSet<String>) {
        if self.ids.as_ref() == Some(&ids) {
            return;
        }
        match save(path, &ids) {
            Ok(()) => {
                self.ids = Some(ids);
                self.failing = false;
            }
            Err(e) => {
                self.ids = None;
                if let Err(rm) = std::fs::remove_file(path)
                    && rm.kind() != io::ErrorKind::NotFound
                {
                    log::warn!("{}: removing it: {rm}", path.display());
                }
                if !std::mem::replace(&mut self.failing, true) {
                    log::warn!("{}: {e}", path.display());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    #[test]
    fn the_background_ids_round_trip_in_mode_0600_and_a_missing_file_is_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("background-sessions");
        assert_eq!(load(&path), None);
        save(&path, &BTreeSet::new()).unwrap();
        assert_eq!(load(&path), Some(BTreeSet::new()));
        let ids = BTreeSet::from(["c2".to_string(), "17".to_string()]);
        save(&path, &ids).unwrap();
        assert_eq!(load(&path), Some(ids));
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn a_failed_write_removes_the_stale_file_and_the_next_sync_writes_again() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("background-sessions");
        save(&path, &BTreeSet::from(["c5".to_string()])).unwrap();
        let blocker = dir.path().join("background-sessions.tmp");
        std::fs::create_dir_all(blocker.join("in-the-way")).unwrap();
        let mut saved = Saved::default();
        let ids = BTreeSet::from(["c7".to_string()]);
        saved.sync(&path, ids.clone());
        assert_eq!(load(&path), None, "no stale ids are left for a restart");
        std::fs::remove_dir_all(&blocker).unwrap();
        saved.sync(&path, ids.clone());
        assert_eq!(load(&path), Some(ids));
    }
}
