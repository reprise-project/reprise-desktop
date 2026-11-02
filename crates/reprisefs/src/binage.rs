//! `[recycle_bin] older_than_days`: items deleted longer ago than that are erased from every
//! Recycle Bin, on a thread of their own, at session start and then daily ([`start`]).

use crate::trash::{self, Bins};
use reprisebus::mutex::lock;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::Duration;

const DAY: Duration = Duration::from_secs(86_400);

/// Whether an item deleted at `deleted` (epoch seconds) is past `days` at `now`; never while
/// `days` is 0.
pub fn expired(deleted: i64, now: i64, days: u16) -> bool {
    days > 0 && now.saturating_sub(deleted) >= i64::from(days) * DAY.as_secs() as i64
}

/// Erase each item of the trash at `root` that [`expired`] at `now`, returning how many went.
/// An item whose `.trashinfo` has no readable `DeletionDate` stays.
pub fn purge_in(root: &Path, days: u16, now: i64) -> usize {
    if days == 0 {
        return 0;
    }
    let (files_dir, info_dir) = (root.join("files"), root.join("info"));
    let Ok(rd) = std::fs::read_dir(&info_dir) else {
        return 0;
    };
    let mut count = 0;
    for entry in rd.flatten() {
        let name = entry.file_name();
        let Some(stem) = name.as_bytes().strip_suffix(b".trashinfo") else {
            continue;
        };
        let body = std::fs::read_to_string(entry.path()).unwrap_or_default();
        if !trash::deletion_date(&body).is_some_and(|d| expired(d, now, days)) {
            continue;
        }
        let files = files_dir.join(std::ffi::OsStr::from_bytes(stem));
        if std::fs::symlink_metadata(&files).is_err() {
            let _ = std::fs::remove_file(entry.path());
            continue;
        }
        let listed = trash::trashed(&files).ok();
        match trash::erase_trashed(&files, listed.as_ref()) {
            Ok(()) => count += 1,
            Err(e) => log::warn!("recycle bin: kept {}: {e}", files.display()),
        }
    }
    count
}

/// The trashes of `bins`: the home trash, then each listed drive's, a drive that does not
/// answer within `within` (a dead mount) left out.
pub fn live_roots(bins: &Bins, within: Duration) -> Vec<PathBuf> {
    let b = bins.clone();
    live_roots_with(bins, within, move |top| b.drive_roots(top))
}

fn live_roots_with(
    bins: &Bins,
    within: Duration,
    probe: impl Fn(&Path) -> Vec<PathBuf> + Clone + Send + 'static,
) -> Vec<PathBuf> {
    let mut out = vec![bins.home.clone()];
    for top in bins.mounts.iter().filter(|m| !bins.unlisted.contains(*m)) {
        let (tx, rx) = mpsc::channel();
        let (probe, t) = (probe.clone(), top.clone());
        let spawned = std::thread::Builder::new()
            .name("bin-age-probe".into())
            .spawn(move || {
                let _ = tx.send(probe(&t));
            });
        if spawned.is_err() {
            continue;
        }
        match rx.recv_timeout(within) {
            Ok(roots) => {
                for r in roots {
                    if !out.contains(&r) {
                        out.push(r);
                    }
                }
            }
            Err(_) => log::info!("recycle bin: {} is not answering, skipped", top.display()),
        }
    }
    out
}

/// One pass over every trash `bins` keeps, by its policy's `older_than_days`.
pub fn purge(bins: &Bins, now: i64) -> usize {
    let days = bins.policy.older_than_days;
    if days == 0 {
        return 0;
    }
    (live_roots(bins, crate::deadline::ACTION_DEADLINE).iter())
        .map(|root| purge_in(root, days, now))
        .sum()
}

static POKE: std::sync::Mutex<Option<Sender<()>>> = std::sync::Mutex::new(None);

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// Start the session's auto-empty: a pass now and every day after, and one on each [`poke`]
/// that finds `older_than_days` changed.
pub fn start() {
    let (tx, rx) = mpsc::channel();
    *lock(&POKE) = Some(tx);
    let run = move || {
        let mut last = None;
        let mut poked = false;
        loop {
            if let Some(bins) = Bins::system() {
                let days = bins.policy.older_than_days;
                if !poked || last != Some(days) {
                    let n = purge(&bins, now());
                    if n > 0 {
                        log::info!("recycle bin: erased {n} items older than {days} days");
                    }
                }
                last = Some(days);
            }
            match rx.recv_timeout(DAY) {
                Ok(()) => poked = true,
                Err(RecvTimeoutError::Timeout) => poked = false,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    };
    if let Err(e) = std::thread::Builder::new()
        .name("bin-age".into())
        .spawn(run)
    {
        log::warn!("recycle bin: no auto-empty thread: {e}");
    }
}

/// The Recycle Bin settings may have changed: the [`start`]ed thread rereads them.
pub fn poke() {
    if let Some(tx) = lock(&POKE).as_ref() {
        let _ = tx.send(());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const NOW: i64 = 1_800_000_000;

    fn item(root: &Path, name: &str, info: &str) -> PathBuf {
        fs::create_dir_all(root.join("files")).unwrap();
        fs::create_dir_all(root.join("info")).unwrap();
        let files = root.join("files").join(name);
        fs::write(&files, name).unwrap();
        fs::write(root.join("info").join(format!("{name}.trashinfo")), info).unwrap();
        files
    }

    fn aged(root: &Path, name: &str, days_ago: i64) -> PathBuf {
        let body = trash::trashinfo(Path::new("/home/u/x"), NOW - days_ago * 86_400);
        item(root, name, &body)
    }

    #[test]
    fn an_item_expires_once_its_days_have_passed_and_never_at_zero() {
        let day = 86_400;
        assert!(!expired(NOW - 30 * day + 1, NOW, 30));
        assert!(expired(NOW - 30 * day, NOW, 30));
        assert!(expired(NOW - 400 * day, NOW, 30));
        assert!(!expired(NOW - 400 * day, NOW, 0), "0 is off");
        assert!(!expired(NOW + day, NOW, 1), "a date ahead of the clock");
    }

    #[test]
    fn a_pass_erases_the_old_items_and_keeps_new_and_undated_ones() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("Trash");
        let old = aged(&root, "old.txt", 31);
        let new = aged(&root, "new.txt", 29);
        let missing = item(&root, "missing.txt", "[Trash Info]\nPath=/home/u/m\n");
        let garbled = item(
            &root,
            "garbled.txt",
            "[Trash Info]\nPath=/home/u/g\nDeletionDate=last tuesday\n",
        );
        assert_eq!(purge_in(&root, 30, NOW), 1);
        assert!(!old.exists());
        assert!(!root.join("info/old.txt.trashinfo").exists());
        for kept in [&new, &missing, &garbled] {
            assert!(kept.exists(), "{} stays", kept.display());
        }
        assert_eq!(purge_in(&root, 0, NOW), 0, "0 erases nothing");
        assert!(new.exists());
    }

    #[test]
    fn an_old_folder_goes_whole_and_an_old_record_without_its_item_goes_too() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("Trash");
        let dir = root.join("files/dir");
        fs::create_dir_all(dir.join("deep")).unwrap();
        fs::write(dir.join("deep/f"), "f").unwrap();
        let body = trash::trashinfo(Path::new("/home/u/dir"), NOW - 90 * 86_400);
        fs::create_dir_all(root.join("info")).unwrap();
        fs::write(root.join("info/dir.trashinfo"), &body).unwrap();
        fs::write(root.join("info/orphan.trashinfo"), &body).unwrap();
        assert_eq!(purge_in(&root, 30, NOW), 1);
        assert!(!dir.exists());
        assert!(!root.join("info/orphan.trashinfo").exists());
    }

    #[test]
    fn every_drives_bin_is_aged_and_an_unlisted_one_is_left() {
        let t = tempfile::tempdir().unwrap();
        let home = t.path().join("data/Trash");
        let (stick, proc_like) = (t.path().join("STICK"), t.path().join("PROC"));
        fs::create_dir_all(&stick).unwrap();
        fs::create_dir_all(&proc_like).unwrap();
        let mut bins = Bins::new(home.clone(), vec![stick.clone(), proc_like.clone()]);
        bins.unlisted.insert(proc_like.clone());
        bins.policy.older_than_days = 7;
        let uid = bins.uid;
        let a = aged(&home, "a", 8);
        let b = aged(&stick.join(format!(".Trash-{uid}")), "b", 8);
        let c = aged(&proc_like.join(format!(".Trash-{uid}")), "c", 8);
        let d = aged(&stick.join(format!(".Trash-{uid}")), "d", 6);
        assert_eq!(purge(&bins, NOW), 2);
        assert!(!a.exists() && !b.exists());
        assert!(c.exists(), "an unlisted file system's bin is left");
        assert!(d.exists());
        bins.policy.older_than_days = 0;
        assert_eq!(purge(&bins, NOW + 86_400 * 100), 0);
    }

    #[test]
    fn a_drive_that_does_not_answer_is_skipped() {
        let (dead, live) = (
            PathBuf::from("/media/u/DEAD"),
            PathBuf::from("/media/u/LIVE"),
        );
        let bins = Bins::new(
            "/home/u/.local/share/Trash".into(),
            vec![dead.clone(), live],
        );
        let (_hold, wait) = mpsc::channel::<()>();
        let wait = std::sync::Arc::new(std::sync::Mutex::new(wait));
        let roots = live_roots_with(&bins, Duration::from_millis(200), move |top: &Path| {
            if top.ends_with("DEAD") {
                let _ = lock(&wait).recv();
            }
            vec![top.join(".Trash-1000")]
        });
        assert_eq!(
            roots,
            [
                PathBuf::from("/home/u/.local/share/Trash"),
                "/media/u/LIVE/.Trash-1000".into()
            ]
        );
    }
}
