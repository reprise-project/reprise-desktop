//! Path completion's file system side: the folder a typed folder part names, and the names in it.
//! [`entries`] blocks on a mount that stopped answering, so hosts call it on a listing thread.

use std::path::{Path, PathBuf};

/// One name in a folder; `dir` follows a symlink to its target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub dir: bool,
}

/// Where `typed`, a folder part as typed (`~/`, `/usr/`, `sub/`, empty for `base` itself),
/// points: `~/` is `home`, a relative part sits under `base`. None for a relative part without
/// a base, or `~` without a home.
pub fn folder(typed: &str, base: Option<&Path>, home: Option<&Path>) -> Option<PathBuf> {
    let raw = if typed == "~" || typed.starts_with("~/") {
        home?.join(typed.trim_start_matches('~').trim_start_matches('/'))
    } else if typed.starts_with('/') {
        PathBuf::from(typed)
    } else {
        base?.join(typed)
    };
    Some(crate::ops::lexical(&raw))
}

/// The names in `dir` whose names are UTF-8; none when it cannot be read.
pub fn entries(dir: &Path) -> Vec<Entry> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    rd.flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let dir = match e.file_type() {
                Ok(t) if t.is_symlink() => e.path().is_dir(),
                Ok(t) => t.is_dir(),
                Err(_) => false,
            };
            Some(Entry { name, dir })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_part_resolves_from_home_the_root_or_the_base() {
        let (home, base) = (Path::new("/home/u"), Path::new("/home/u/Documents"));
        let f = |t| folder(t, Some(base), Some(home));
        assert_eq!(f("~/"), Some("/home/u".into()));
        assert_eq!(f("~/Music/"), Some("/home/u/Music".into()));
        assert_eq!(f("/usr/"), Some("/usr".into()));
        assert_eq!(f(""), Some("/home/u/Documents".into()));
        assert_eq!(f("../Music/"), Some("/home/u/Music".into()));
        assert_eq!(folder("sub/", None, Some(home)), None, "no base");
        assert_eq!(folder("~/", Some(base), None), None, "no home");
    }

    #[test]
    fn entries_mark_folders_and_follow_symlinks() {
        let t = tempfile::tempdir().unwrap();
        std::fs::create_dir(t.path().join("dir")).unwrap();
        std::fs::write(t.path().join("file"), "").unwrap();
        std::os::unix::fs::symlink(t.path().join("dir"), t.path().join("link")).unwrap();
        let mut got = entries(t.path());
        got.sort_by(|a, b| a.name.cmp(&b.name));
        let want = |name: &str, dir| Entry {
            name: name.into(),
            dir,
        };
        assert_eq!(
            got,
            [want("dir", true), want("file", false), want("link", true)]
        );
        assert!(entries(&t.path().join("missing")).is_empty());
    }
}
