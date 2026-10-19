//! Reads the recent files again when `recently-used.xbel` changes, and checks the recent files
//! and document folders again when a listed file or a document folder goes or `user-dirs.dirs`
//! changes, so open lists follow them.

use crate::compositor::State;
use crate::dirwatch::{DirWatch, Want};
use crate::menuview::MenuKind;
use crate::taskbar::JumpRow;
use inotify::WatchMask;
use smithay::reexports::calloop::{Interest, Mode, PostAction, generic::Generic};
use std::os::fd::{AsFd, BorrowedFd};
use std::path::{Path, PathBuf};

struct Watch(DirWatch);

impl AsFd for Watch {
    fn as_fd(&self) -> BorrowedFd<'_> {
        // SAFETY: the fd is the watch's inotify fd, open for as long as the
        // watch, which outlives the borrow.
        unsafe { BorrowedFd::borrow_raw(self.0.fd()) }
    }
}

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
        watch(state, "recent files", wants(&xbel), State::read_recent);
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
/// files the open lists show. Each folder is looked up on a file job of its own, since one may
/// sit on a dead mount; a folder newly watched calls for a check, as a file may have gone
/// before its watch came.
pub fn follow_recent(state: &mut State) {
    let mut want = document_wants(
        &reprisefs::places::user_dirs_file(),
        &state.recent.document_dir_paths(),
    );
    want.extend(file_wants(&listed_files(state)));
    let recent = &mut state.recent;
    let Some(w) = recent.folder_watch.as_mut() else {
        return;
    };
    let adds: Vec<_> = (w.plan(&want).each())
        .filter(|(dir, _)| recent.folder_adds.insert(dir.clone()))
        .collect();
    for (dir, add) in adds {
        state.spawn_fs_job(move || {
            let added = add.run();
            Box::new(move |s: &mut State| {
                s.recent.folder_adds.remove(&dir);
                let new = !added.is_empty();
                if let Some(w) = s.recent.folder_watch.as_mut() {
                    w.adopt(added);
                }
                if new {
                    s.check_recent();
                }
            })
        });
    }
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

/// Runs `hit` on the loop whenever an event `want` asks for arrives.
fn watch(state: &mut State, what: &str, want: Vec<Want>, hit: fn(&mut State)) {
    let mut watch = match DirWatch::new() {
        Ok(w) => w,
        Err(e) => {
            log::warn!("{what} watch: inotify unavailable: {e}");
            return;
        }
    };
    watch.sync(&want);
    let source = Generic::new(Watch(watch), Interest::READ, Mode::Level);
    let added = state.handle.insert_source(source, move |_, watch, state| {
        // SAFETY: the watch is not dropped here, only drained and
        // resynced; the source keeps its fd registered for the watch's
        // life.
        let watch = unsafe { &mut watch.get_mut().0 };
        let changed = watch.drain();
        watch.sync(&want);
        if changed {
            hit(state);
        }
        Ok(PostAction::Continue)
    });
    if let Err(e) = added {
        log::warn!("{what} watch: {}", e.error);
    }
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
}
