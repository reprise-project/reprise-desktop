//! What an Open or Save As dialog does to the folder it shows besides listing it: New Folder,
//! a rename, Delete and Shift+Delete. Each blocks for as long as the file system does, so a host runs it off
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

/// How Delete (or Shift+Delete, `shift`) treats `paths` ([`bincap::delete_mode`]): it stats
/// each item and its mount, so a host runs it off its loop.
pub fn delete_mode(paths: &[PathBuf], shift: bool) -> bincap::DeleteMode {
    mode_in(trash::Bins::system().as_ref(), paths, shift)
}

fn mode_in(bins: Option<&trash::Bins>, paths: &[PathBuf], shift: bool) -> bincap::DeleteMode {
    let fates: Vec<bincap::Fate> = (paths.iter())
        .map(|p| bins.map_or(bincap::Fate::NoBin, |b| b.fate(p)))
        .collect();
    let confirm = bins.map_or(bincap::Policy::default().confirm, |b| b.policy.confirm);
    bincap::delete_mode(confirm, &fates, shift)
}

/// Delete to the Recycle Bin on `paths`: each measured against its bin's cap, which walks a
/// folder, so a host runs it off its loop. The asker holds the items too big for their bin.
pub fn measure(paths: Vec<PathBuf>) -> bincap::TooBig {
    let bins = trash::Bins::system();
    let measured = (paths.iter())
        .map(|p| bins.as_ref().and_then(|b| b.measure(p)))
        .collect();
    bincap::TooBig::new(paths, measured)
}

/// Deletes each item as `plan` says, to its Recycle Bin or for good, stopping at the first
/// failure, which it returns.
pub fn delete(plan: Vec<(PathBuf, bincap::Plan)>) -> Result<(), (PathBuf, io::Error)> {
    for (path, how) in plan {
        if let Err(e) = crate::ops::delete(&path, how) {
            return Err((path, e));
        }
    }
    Ok(())
}

/// The error box's text for `path`, which a delete could not take.
pub fn delete_failed_text(path: &Path, e: &io::Error) -> String {
    format!("Cannot delete {}: {e}", bincap::label(path))
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

    #[test]
    fn shift_delete_erases_after_asking_and_delete_erases_what_has_no_bin() {
        let t = tempfile::tempdir().unwrap();
        let home = t.path().join("home");
        std::fs::create_dir(&home).unwrap();
        let home = home.canonicalize().unwrap();
        std::fs::write(home.join("a.txt"), "").unwrap();
        let bins = trash::Bins::new(home.join(".trash"), vec![home.clone()]);
        let item = vec![home.join("a.txt")];
        let shift = mode_in(Some(&bins), &item, true);
        assert!(shift.erase && shift.ask, "{shift:?}");
        let plain = mode_in(Some(&bins), &item, false);
        assert!(!plain.erase, "{plain:?}");
        let none = mode_in(None, &item, false);
        assert!(none.erase && none.ask, "{none:?}");
        assert_eq!(none.no_bin, bincap::BinLack::All);
    }

    #[test]
    fn an_erase_removes_a_folder_with_its_contents_and_only_a_link_itself() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("Reports");
        std::fs::create_dir_all(dir.join("2026")).unwrap();
        std::fs::write(dir.join("2026/q1.txt"), "q1").unwrap();
        let kept = t.path().join("kept");
        std::fs::create_dir(&kept).unwrap();
        std::fs::write(kept.join("b.txt"), "b").unwrap();
        let link = t.path().join("link");
        std::os::unix::fs::symlink(&kept, &link).unwrap();
        let erase = |p: &Path| (p.to_path_buf(), bincap::Plan::Erase);
        assert!(delete(vec![erase(&dir), erase(&link)]).is_ok());
        assert!(!dir.exists());
        assert!(std::fs::symlink_metadata(&link).is_err());
        assert!(kept.join("b.txt").is_file(), "the link's folder stays");
    }

    #[test]
    fn a_failed_erase_stops_the_rest_and_says_which_item() {
        let t = tempfile::tempdir().unwrap();
        let gone = t.path().join("gone.txt");
        let after = t.path().join("after.txt");
        std::fs::write(&after, "").unwrap();
        let plan = vec![
            (gone.clone(), bincap::Plan::Erase),
            (after.clone(), bincap::Plan::Erase),
        ];
        let (path, e) = delete(plan).unwrap_err();
        assert_eq!(path, gone);
        assert!(after.is_file(), "the item after the failure stays");
        assert!(delete_failed_text(&path, &e).starts_with("Cannot delete gone.txt: "));
    }
}
