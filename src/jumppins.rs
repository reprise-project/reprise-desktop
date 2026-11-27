//! The files pinned to programs' jump lists, by desktop file id, persisted in `state.toml`. A
//! jump list shows its pinned files above Recent, in the order they were pinned.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct JumpPins(BTreeMap<String, Vec<PathBuf>>);

impl JumpPins {
    pub fn from_saved(saved: &BTreeMap<String, Vec<String>>) -> JumpPins {
        let pins = (saved.iter())
            .filter(|(_, files)| !files.is_empty())
            .map(|(id, files)| (id.clone(), files.iter().map(PathBuf::from).collect()))
            .collect();
        JumpPins(pins)
    }

    /// What `state.toml` keeps; a file name that is not UTF-8 is never pinned.
    pub fn saved(&self) -> BTreeMap<String, Vec<String>> {
        (self.0.iter())
            .map(|(id, files)| {
                let files = files.iter().filter_map(|f| f.to_str().map(String::from));
                (id.clone(), files.collect())
            })
            .collect()
    }

    /// Program `id`'s pinned files.
    pub fn of(&self, id: &str) -> &[PathBuf] {
        self.0.get(id).map_or(&[], Vec::as_slice)
    }

    pub fn is_pinned(&self, id: &str, path: &Path) -> bool {
        self.of(id).iter().any(|p| p == path)
    }

    /// Pins `path` to the end of program `id`'s list; false when it was there or is not UTF-8.
    pub fn pin(&mut self, id: &str, path: &Path) -> bool {
        if path.to_str().is_none() || self.is_pinned(id, path) {
            return false;
        }
        self.0.entry(id.to_string()).or_default().push(path.into());
        true
    }

    /// False when `path` was not pinned to program `id`'s list.
    pub fn unpin(&mut self, id: &str, path: &Path) -> bool {
        let Some(files) = self.0.get_mut(id) else {
            return false;
        };
        let before = files.len();
        files.retain(|p| p != path);
        let unpinned = files.len() != before;
        if files.is_empty() {
            self.0.remove(id);
        }
        unpinned
    }
}

/// A program's jump list files: its `pinned` ones, then the first `limit` of `recent` (newest
/// first) that are not pinned.
pub fn jump_files(pinned: &[PathBuf], recent: Vec<PathBuf>, limit: usize) -> Vec<PathBuf> {
    let unpinned = recent.into_iter().filter(|p| !pinned.contains(p));
    pinned.iter().cloned().chain(unpinned.take(limit)).collect()
}

/// Program `e`'s jump list files ([`jump_files`]) from `pins` and the last recent read, its
/// recent ones only with `recent_on`, and how many of them lead as pinned.
pub fn files_for(
    pins: &JumpPins,
    recent: &crate::recentdocs::RecentDocs,
    recent_on: bool,
    e: &reprisefs::apps::AppEntry,
) -> (Vec<PathBuf>, usize) {
    use crate::taskbar::JUMP_RECENT;
    let pinned = pins.of(e.id.as_str());
    let recent = match recent_on {
        true => recent.for_app(JUMP_RECENT + pinned.len(), e),
        false => Vec::new(),
    };
    (jump_files(pinned, recent, JUMP_RECENT), pinned.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pins_keep_their_order_and_leave_the_saved_list_when_the_last_goes() {
        let mut p = JumpPins::default();
        assert!(p.pin("mail.desktop", Path::new("/a")));
        assert!(p.pin("mail.desktop", Path::new("/b")));
        assert!(!p.pin("mail.desktop", Path::new("/a")), "pinned once");
        assert_eq!(p.of("mail.desktop"), [PathBuf::from("/a"), "/b".into()]);
        assert!(p.of("paint.desktop").is_empty());
        let back = JumpPins::from_saved(&p.saved());
        assert_eq!(back, p);
        assert!(p.unpin("mail.desktop", Path::new("/a")));
        assert!(!p.unpin("mail.desktop", Path::new("/a")));
        assert!(p.unpin("mail.desktop", Path::new("/b")));
        assert!(p.saved().is_empty(), "no program with an empty list");
    }

    #[test]
    fn jump_files_put_the_pins_first_and_list_no_file_twice() {
        let path = |s: &str| PathBuf::from(s);
        let recent = vec![path("/r1"), path("/p"), path("/r2"), path("/r3")];
        assert_eq!(
            jump_files(&[path("/p")], recent, 2),
            [path("/p"), path("/r1"), path("/r2")]
        );
    }
}
