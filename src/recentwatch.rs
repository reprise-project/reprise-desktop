//! Reads the recent files again when `recently-used.xbel` changes, and checks the recent files
//! and document folders again when a listed file or a document folder goes or `user-dirs.dirs`
//! changes, so open lists follow them.

use crate::compositor::State;
use crate::dirwatch::{DirWatch, Landed, Want};
use crate::menuview::MenuKind;
use crate::taskbar::JumpRow;
use inotify::WatchMask;
use smithay::reexports::calloop::{Interest, Mode, PostAction, generic::Generic};
use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};

/// The file's directory, waking only for the file: GLib and KDE replace the xbel by a rename.
fn wants(file: &Path) -> Vec<Want> {
    let (Some(dir), Some(name)) = (file.parent(), file.file_name()) else {
        return Vec::new();
    };
    vec![Want {
        dir: dir.to_path_buf(),
        mask: WatchMask::CLOSE_WRITE
            | WatchMask::MOVED_TO
            | WatchMask::MOVED_FROM
            | WatchMask::DELETE,
        only: vec![name.to_os_string()],
    }]
}

/// Each document folder's parent, waking only for the folder's name.
fn folder_wants(dirs: &[PathBuf]) -> Vec<Want> {
    dirs.iter()
        .filter_map(|d| {
            Some(Want {
                dir: d.parent()?.to_path_buf(),
                mask: WatchMask::CREATE
                    | WatchMask::DELETE
                    | WatchMask::MOVED_TO
                    | WatchMask::MOVED_FROM,
                only: vec![d.file_name()?.to_os_string()],
            })
        })
        .collect()
}

/// `user-dirs.dirs`, and the document folders `dirs` read from it.
fn document_wants(user_dirs: &Path, dirs: &[PathBuf]) -> Vec<Want> {
    let mut want = wants(user_dirs);
    want.extend(folder_wants(dirs));
    want
}

pub fn setup(state: &mut State) {
    if let Some(xbel) = reprisefs::recent::xbel_path() {
        watch_xbel(state, xbel);
    }
    let watch = match DirWatch::new() {
        Ok(w) => w,
        Err(e) => return log::warn!("document folders watch: inotify unavailable: {e}"),
    };
    // SAFETY: the fd is the watch's inotify fd, open for this call.
    let fd = match unsafe { BorrowedFd::borrow_raw(watch.fd()) }.try_clone_to_owned() {
        Ok(fd) => fd,
        Err(e) => return log::warn!("document folders watch: no fd: {e}"),
    };
    let source = Generic::new(fd, Interest::READ, Mode::Level);
    let added = state.handle.insert_source(source, |_, _, state| {
        let changed = state
            .recent
            .folder_watch
            .as_mut()
            .is_some_and(|w| w.drain());
        follow_recent(state);
        if changed {
            state.check_recent();
        }
        Ok(PostAction::Continue)
    });
    if let Err(e) = added {
        return log::warn!("document folders watch: {}", e.error);
    }
    state.recent.folder_watch = Some(watch);
    follow_recent(state);
}

/// Watches `user-dirs.dirs`, the document folders the last recent check read from it and the
/// files the open lists show, each folder added on a file job of its own.
pub fn follow_recent(state: &mut State) {
    let mut want = document_wants(
        &reprisefs::places::user_dirs_file(),
        &state.recent.document_dir_paths(),
    );
    want.extend(file_wants(&listed_files(state)));
    let Some(w) = state.recent.folder_watch.as_mut() else {
        return;
    };
    let adds = w.plan(&want);
    let adds = w.send(adds.each());
    state.spawn_watch_adds(adds, |s| s.recent.folder_watch.as_mut(), folder_landed);
}

/// A folder's add landed: a want that widened while it was out is planned again, and the
/// recent files and folders are checked when [`checks`] says so.
fn folder_landed(state: &mut State, landed: Landed) {
    if landed.grew {
        follow_recent(state);
    }
    if checks(landed) {
        state.check_recent();
    }
}

/// Whether a landing calls for a check: a change between the check that listed a file and its
/// folder's watch starting reports no event, so a folder newly watched checks too.
fn checks(landed: Landed) -> bool {
    landed.grew || landed.hit
}

/// A menu or the Start panel closed: once the loop is idle, the watch drops the folders only
/// its files held.
pub fn lists_closed(state: &mut State) {
    state.handle.insert_idle(follow_recent);
}

/// The files the open Documents menus and jump lists show, the Start panel's included.
fn listed_files(state: &State) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = state
        .menus
        .iter()
        .flat_map(|m| listed_in(&m.kind))
        .collect();
    if let Some(v) = state.start_panel.as_ref() {
        files.extend(v.jump_files().cloned());
    }
    files
}

fn listed_in(kind: &MenuKind) -> Vec<PathBuf> {
    match kind {
        MenuKind::Documents { paths, .. } => paths.iter().flatten().cloned().collect(),
        MenuKind::JumpList { rows, .. } => (rows.iter().flatten())
            .filter_map(|r| match r {
                JumpRow::Recent(p) => Some(p.clone()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Each file's folder, waking only for the file leaving it.
fn file_wants(files: &[PathBuf]) -> Vec<Want> {
    files
        .iter()
        .filter_map(|f| {
            Some(Want {
                dir: f.parent()?.to_path_buf(),
                mask: WatchMask::DELETE | WatchMask::MOVED_FROM,
                only: vec![f.file_name()?.to_os_string()],
            })
        })
        .collect()
}

/// Reads the recent files again whenever the xbel at `xbel` is replaced.
fn watch_xbel(state: &mut State, xbel: PathBuf) {
    let watch = match DirWatch::new() {
        Ok(w) => w,
        Err(e) => return log::warn!("recent files watch: inotify unavailable: {e}"),
    };
    // SAFETY: the fd is the watch's inotify fd, open for this call.
    let fd = match unsafe { BorrowedFd::borrow_raw(watch.fd()) }.try_clone_to_owned() {
        Ok(fd) => fd,
        Err(e) => return log::warn!("recent files watch: no fd: {e}"),
    };
    let source = Generic::new(fd, Interest::READ, Mode::Level);
    let file = xbel.clone();
    let added = state.handle.insert_source(source, move |_, _, state| {
        state.recent.xbel_watch.drain();
        sync_xbel(state, file.clone());
        Ok(PostAction::Continue)
    });
    if let Err(e) = added {
        return log::warn!("recent files watch: {}", e.error);
    }
    state.recent.xbel_watch.watch = Some(watch);
    sync_xbel(state, xbel);
}

/// Watches the xbel's folder again on a file job (it may sit on a mount that stopped
/// answering), then reads the recent files when an event came.
fn sync_xbel(state: &mut State, xbel: PathBuf) {
    let Some(job) = state.recent.xbel_watch.start() else {
        return;
    };
    let hit = job.hit;
    state.spawn_fs_job(move || {
        let want = wants(&xbel);
        let added = job.adder.run(&want);
        Box::new(move |state: &mut State| {
            if state.recent.xbel_watch.landed(&want, added) {
                sync_xbel(state, xbel);
            }
            if hit {
                state.read_recent();
            }
        })
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_replaced_xbel_wakes_the_watch_and_a_neighbour_does_not() {
        let dir = std::env::temp_dir().join(format!("reprise-recentwatch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let xbel = dir.join("recently-used.xbel");
        let mut w = DirWatch::new().unwrap();
        w.sync(&wants(&xbel));
        std::fs::write(dir.join("other.txt"), "").unwrap();
        assert!(!w.drain(), "another file in the folder");
        std::fs::write(dir.join("recently-used.xbel.TMP"), "<xbel/>").unwrap();
        std::fs::rename(dir.join("recently-used.xbel.TMP"), &xbel).unwrap();
        assert!(w.drain(), "the xbel replaced by a rename");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_edited_user_dirs_wakes_the_watch_and_the_folders_follow_the_last_read() {
        let home =
            std::env::temp_dir().join(format!("reprise-userdirswatch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join(".config")).unwrap();
        let user_dirs = home.join(".config/user-dirs.dirs");
        let mut w = DirWatch::new().unwrap();
        w.sync(&document_wants(&user_dirs, &[home.join("Documents")]));
        std::fs::write(home.join(".config/other.conf"), "").unwrap();
        assert!(!w.drain(), "another file in the config folder");
        std::fs::write(home.join(".config/user-dirs.dirs.tmp"), "").unwrap();
        std::fs::rename(home.join(".config/user-dirs.dirs.tmp"), &user_dirs).unwrap();
        assert!(w.drain(), "user-dirs.dirs replaced by a rename");

        w.sync(&document_wants(&user_dirs, &[home.join("Papers")]));
        std::fs::create_dir(home.join("Documents")).unwrap();
        assert!(!w.drain(), "Documents is no document folder any more");
        std::fs::create_dir(home.join("Papers")).unwrap();
        assert!(w.drain(), "Papers made");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_document_folder_coming_or_going_wakes_the_watch_and_a_file_beside_it_does_not() {
        let home = std::env::temp_dir().join(format!("reprise-folderwatch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join("Music")).unwrap();
        let mut w = DirWatch::new().unwrap();
        w.sync(&folder_wants(&[home.join("Music"), home.join("Videos")]));
        std::fs::write(home.join("notes.txt"), "").unwrap();
        assert!(!w.drain(), "a file beside the folders");
        std::fs::remove_dir(home.join("Music")).unwrap();
        assert!(w.drain(), "Music deleted");
        std::fs::create_dir(home.join("Videos")).unwrap();
        assert!(w.drain(), "Videos made");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_listed_file_deleted_or_moved_away_wakes_the_watch_and_a_neighbour_does_not() {
        let dir = std::env::temp_dir().join(format!("reprise-listedwatch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let [a, b, other] = ["a.txt", "b.txt", "other.txt"].map(|n| dir.join(n));
        for f in [&a, &b, &other] {
            std::fs::write(f, "").unwrap();
        }
        let docs = MenuKind::Documents {
            paths: vec![Some(a.clone()), None],
            tips: vec![None, None],
            dirs: false,
            icon_px: 16,
        };
        let jump = MenuKind::JumpList {
            rows: vec![
                Some(JumpRow::Recent(b.clone())),
                None,
                Some(JumpRow::Launch),
            ],
            target: Box::new(crate::menuview::JumpTarget {
                name: "Notepad".into(),
                launch: None,
                entry: None,
                pin_file: None,
                source: None,
                windows: Vec::new(),
            }),
            button: 0,
        };
        let listed: Vec<PathBuf> = [docs, jump].iter().flat_map(listed_in).collect();
        assert_eq!(listed, [a.clone(), b.clone()]);

        let mut w = DirWatch::new().unwrap();
        w.sync(&file_wants(&listed));
        std::fs::remove_file(&other).unwrap();
        std::fs::write(dir.join("c.txt"), "").unwrap();
        assert!(!w.drain(), "a file the lists do not show");
        std::fs::remove_file(&a).unwrap();
        assert!(w.drain(), "a.txt deleted");
        std::fs::rename(&b, dir.join("b2.txt")).unwrap();
        assert!(w.drain(), "b.txt moved away");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Plans `want` on `w` and runs each add sent out, as the file jobs do, for landing later.
    fn sent(w: &mut DirWatch, want: &[Want]) -> Vec<(PathBuf, crate::dirwatch::Added)> {
        let adds = w.plan(want);
        (w.send(adds.each()).into_iter())
            .map(|(dir, add)| (dir, add.run()))
            .collect()
    }

    #[test]
    fn a_folder_made_and_a_listed_file_renamed_or_deleted_while_its_add_is_out_count_when_it_lands()
    {
        let home = std::env::temp_dir().join(format!("reprise-recentgap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        for d in ["one", "two"] {
            std::fs::create_dir_all(home.join(d)).unwrap();
        }
        let (a, b) = (home.join("one/a.txt"), home.join("two/b.txt"));
        std::fs::write(&a, "").unwrap();
        std::fs::write(&b, "").unwrap();
        let mut want = folder_wants(&[home.join("Papers")]);
        want.extend(file_wants(&[a.clone(), b.clone()]));
        let mut w = DirWatch::new().unwrap();
        let added = sent(&mut w, &want);
        assert_eq!(added.len(), 3, "the home folder, one and two");

        std::fs::create_dir(home.join("Papers")).unwrap();
        std::fs::rename(&a, home.join("one/a2.txt")).unwrap();
        std::fs::remove_file(&b).unwrap();
        assert!(!w.drain(), "no add is taken yet");
        let again = w.plan(&want);
        assert!(w.send(again.each()).is_empty(), "each folder is out");
        for (dir, added) in added {
            let landed = w.land(&dir, added);
            assert!(landed.hit, "the change in {} counts", dir.display());
            assert!(checks(landed));
        }
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_listed_file_deleted_before_its_folder_is_watched_reports_no_event_and_checks() {
        let dir = std::env::temp_dir().join(format!("reprise-recentlate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.txt");
        std::fs::write(&a, "").unwrap();
        assert!(a.is_file(), "the check lists a.txt");
        std::fs::remove_file(&a).unwrap();

        let mut w = DirWatch::new().unwrap();
        let [(folder, added)] = sent(&mut w, &file_wants(&[a])).try_into().ok().unwrap();
        let landed = w.land(&folder, added);
        assert_eq!(
            landed,
            Landed {
                grew: true,
                hit: false
            },
            "no event reports a.txt gone, only its folder newly watched"
        );
        assert!(checks(landed), "a folder newly watched checks");
        assert!(!w.drain());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_listed_files_folder_deleted_while_its_add_is_out_is_watched_again_once_made() {
        let home = std::env::temp_dir().join(format!("reprise-recentgone-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join("proj")).unwrap();
        let a = home.join("proj/a.txt");
        std::fs::write(&a, "").unwrap();
        let want = file_wants(std::slice::from_ref(&a));
        let mut w = DirWatch::new().unwrap();
        let [(folder, added)] = sent(&mut w, &want).try_into().ok().unwrap();

        std::fs::remove_dir_all(home.join("proj")).unwrap();
        assert!(!w.drain(), "its add is not taken yet");
        assert!(w.land(&folder, added).hit, "the folder went");
        std::fs::create_dir(home.join("proj")).unwrap();
        std::fs::write(&a, "").unwrap();
        let [(folder, added)] = sent(&mut w, &want).try_into().ok().unwrap();
        assert!(w.land(&folder, added).grew, "the new folder is watched");
        std::fs::remove_file(&a).unwrap();
        assert!(w.drain(), "a.txt deleted from the new folder");
        let _ = std::fs::remove_dir_all(&home);
    }
}
