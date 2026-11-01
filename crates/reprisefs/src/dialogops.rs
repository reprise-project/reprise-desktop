//! What an Open or Save As dialog does to the folder it shows besides listing it: New Folder,
//! a rename and Delete. Each blocks for as long as the file system does, so a host runs it off
//! its loop.

use crate::{bincap, hidden, names, trash};
use std::io;
use std::path::{Path, PathBuf};

/// Makes a folder in `dir` named `base`, numbered past the names in `taken` and past any on disk
/// that `taken` missed (a file the dialog's filter hides). Returns its name.
pub fn new_folder(dir: &Path, base: &str, taken: &[String]) -> io::Result<String> {
    let mut taken = taken.to_vec();
    loop {
        let name = names::unique_name(base, &taken);
        match std::fs::create_dir(dir.join(&name)) {
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists && taken.len() < 10_000 => {
                taken.push(name);
            }
            made => return made.map(|()| name),
        }
    }
}

/// Renames the entry `old` at `from` to `typed`, trimmed. `taken` is the folder's other names as
/// listed; one on disk the listing missed is refused at the rename itself, never overwritten.
/// Returns the new name, or why it was refused, worded for the user.
pub fn rename(from: &Path, old: &str, typed: &str, taken: &[String]) -> Result<String, String> {
    match names::verdict(old, typed, taken) {
        names::Verdict::Cancel => Ok(old.to_string()),
        names::Verdict::Refuse(why) => Err(why),
        names::Verdict::Rename(name) => {
            let to = from.with_file_name(&name);
            hidden::rename(from, &to)
                .map(|()| name)
                .map_err(|e| names::failed_text(old, &e))
        }
    }
}

/// Moves each of `paths` to its Recycle Bin. One without a bin, too big for its bin or on a bin
/// set to remove at once is left where it is: the dialog has no box to ask before erasing it.
/// Returns those left, with why.
pub fn trash(paths: &[PathBuf]) -> Vec<(PathBuf, String)> {
    let Some(bins) = trash::Bins::system() else {
        let why = "there is no Recycle Bin".to_string();
        return paths.iter().map(|p| (p.clone(), why.clone())).collect();
    };
    let mut left = Vec::new();
    for path in paths {
        if bins.fate(path) != bincap::Fate::Recycle {
            left.push((path.clone(), "it would be deleted for good".to_string()));
            continue;
        }
        let measured = bins.measure(path);
        if measured.as_ref().is_some_and(bincap::Measured::too_big) {
            left.push((
                path.clone(),
                "it is too big for the Recycle Bin".to_string(),
            ));
            continue;
        }
        if let Err(e) = bins.trash_measured(path, measured) {
            left.push((path.clone(), e.to_string()));
        }
    }
    left
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_folder_takes_the_first_name_free_on_disk() {
        let t = tempfile::tempdir().unwrap();
        std::fs::write(t.path().join("New Folder"), "").unwrap();
        let name = new_folder(t.path(), "New Folder", &[]).unwrap();
        assert_eq!(name, "New Folder (2)");
        assert!(t.path().join("New Folder (2)").is_dir());
    }

    #[test]
    fn a_rename_onto_a_name_on_disk_is_refused_and_overwrites_nothing() {
        let t = tempfile::tempdir().unwrap();
        std::fs::write(t.path().join("a.txt"), "a").unwrap();
        std::fs::write(t.path().join("b.txt"), "b").unwrap();
        let from = t.path().join("a.txt");
        assert!(rename(&from, "a.txt", "b.txt", &[]).is_err());
        assert_eq!(std::fs::read(t.path().join("b.txt")).unwrap(), b"b");
        assert!(rename(&from, "a.txt", "/x", &[]).is_err());
        assert_eq!(rename(&from, "a.txt", " c.txt ", &[]), Ok("c.txt".into()));
        assert!(t.path().join("c.txt").is_file());
    }
}
