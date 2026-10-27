//! The recent files and document folders the Start menus and jump lists show. A recent file can
//! sit on a mount that never answers, so the bookmarks and what each path is come from file jobs:
//! the loop shows what the last read and check found, a path not checked yet included.

use crate::mutex::lock;
use reprisefs::apps::AppEntry;
use reprisefs::recent::{Bookmark, RecentFiles};
use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::compositor::State;
use crate::fsjobs::Reads;
use crate::menuview::{MenuKind, RowIcons, jump_relisted};
use crate::raster::Rect;
use crate::startmenu::{MenuItem, MenuLayout};

/// What a path was found to be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Found {
    File,
    Dir,
    Neither,
}

type Stat = fn(&Path) -> io::Result<std::fs::Metadata>;

/// The paths a check asked about: what a landing check keeps in `found`.
type Asked = HashSet<PathBuf>;

pub struct RecentDocs {
    /// None while a read job has it.
    files: Option<RecentFiles>,
    /// The bookmarks the last read found, newest first.
    marks: Vec<Bookmark>,
    /// The xbel changed while a read was out: read again when it answers.
    reread: bool,
    /// A check waits for the read under way.
    check_after_read: bool,
    /// Filled by the check job one path at a time, so a path that hangs keeps only itself and
    /// the paths after it unchecked.
    found: Arc<Mutex<HashMap<PathBuf, Found>>>,
    /// The mount points a check is statting a path on. A later check skips their paths, so a
    /// dead mount holds one check thread, and the checks stuck on it hold no others back.
    statting: Arc<Mutex<HashSet<PathBuf>>>,
    /// The checks in flight: one at a time until it is stuck, then a newer one may start.
    checks: Reads,
    /// A check was asked for while one ran: check again when one answers or may start.
    recheck: bool,
    /// A timer waits to start the check `recheck` asks for.
    retry_armed: bool,
    /// The document folders with their icon names as the last check read them from
    /// `user-dirs.dirs`, whether or not they exist.
    dirs: Vec<(PathBuf, &'static str)>,
    /// Wakes for `user-dirs.dirs`, the document folders and the files open lists show
    /// (`recentwatch`).
    pub(crate) folder_watch: Option<crate::dirwatch::DirWatch>,
    /// The folders a file job is adding to `folder_watch`.
    pub(crate) folder_adds: HashSet<PathBuf>,
    /// Wakes for `recently-used.xbel` being replaced (`recentwatch`).
    pub(crate) xbel_watch: crate::dirwatch::JobWatch,
}

impl Default for RecentDocs {
    fn default() -> RecentDocs {
        RecentDocs::at(RecentFiles::new())
    }
}

impl RecentDocs {
    fn at(files: RecentFiles) -> RecentDocs {
        RecentDocs {
            files: Some(files),
            marks: Vec::new(),
            reread: false,
            check_after_read: false,
            found: Arc::default(),
            statting: Arc::default(),
            checks: Reads::default(),
            recheck: false,
            retry_armed: false,
            dirs: Vec::new(),
            folder_watch: None,
            folder_adds: HashSet::new(),
            xbel_watch: Default::default(),
        }
    }

    fn found(&self, path: &Path) -> Option<Found> {
        self.found.lock().ok()?.get(path).copied()
    }

    /// The newest `limit` bookmarks `keep` takes that were not found missing or a folder.
    fn pick(&self, limit: usize, keep: impl Fn(&Bookmark) -> bool) -> Vec<PathBuf> {
        let found = lock(&self.found);
        self.marks
            .iter()
            .filter(|b| keep(b) && matches!(found.get(&b.path), None | Some(Found::File)))
            .take(limit)
            .map(|b| b.path.clone())
            .collect()
    }

    pub fn current(&self, limit: usize) -> Vec<PathBuf> {
        self.pick(limit, |_| true)
    }

    /// `current`, keeping only the files the application `entry` used: its jump list.
    pub fn for_app(&self, limit: usize, entry: &AppEntry) -> Vec<PathBuf> {
        self.pick(limit, |b| reprisefs::recent::used_by_entry(b, entry))
    }

    /// The document folders that were not found missing; none before the first check.
    pub fn document_dirs(&self) -> Vec<(PathBuf, &'static str)> {
        self.found_dirs(self.dirs.clone())
    }

    /// The document folders the last check read, found or not.
    pub fn document_dir_paths(&self) -> Vec<PathBuf> {
        self.dirs.iter().map(|(d, _)| d.clone()).collect()
    }

    /// Whether `path` is one of the document folders, found or not.
    pub fn is_document_dir(&self, path: &Path) -> bool {
        self.dirs.iter().any(|(d, _)| d == path)
    }

    fn found_dirs(&self, dirs: Vec<(PathBuf, &'static str)>) -> Vec<(PathBuf, &'static str)> {
        dirs.into_iter()
            .filter(|(dir, _)| matches!(self.found(dir), None | Some(Found::Dir)))
            .collect()
    }

    /// A new check's id and the bookmarks it asks about besides the document folders (none
    /// without `files`), or None while [`Reads::start`] refuses one.
    fn start_check(&mut self, now: Instant, files: bool) -> Option<(u64, Vec<PathBuf>)> {
        let id = self.checks.start(now, ())?;
        let marks = self.marks.iter().filter(|_| files);
        Some((id, marks.map(|b| b.path.clone()).collect()))
    }

    /// Ends check `id`; true when it is the newest to land, and then its folders and what it
    /// asked about are kept, `found` dropping other paths.
    fn land_check(&mut self, id: u64, dirs: Vec<(PathBuf, &'static str)>, asked: Asked) -> bool {
        if !self.checks.land(id) {
            return false;
        }
        self.dirs = dirs;
        if let Ok(mut f) = self.found.lock() {
            f.retain(|p, _| asked.contains(p));
        }
        true
    }

    /// Takes what a read found; true when the bookmarks changed.
    fn take_read(&mut self, files: RecentFiles, marks: Vec<Bookmark>) -> bool {
        self.files = Some(files);
        let changed = self.marks != marks;
        self.marks = marks;
        changed
    }
}

/// What a check job works with: the shared `found` and `statting`, the mount points, and how
/// it stats.
struct Probe<'a> {
    found: &'a Mutex<HashMap<PathBuf, Found>>,
    statting: &'a Mutex<HashSet<PathBuf>>,
    mounts: &'a [PathBuf],
    stat: Stat,
}

/// Reads the document folders, then [`check`]s them and `paths`; returns the folders read and
/// every path asked about. Runs off the loop: `user-dirs.dirs` lives in the home folder, which
/// may be a network mount.
fn check_with_dirs(
    paths: Vec<PathBuf>,
    probe: &Probe,
    read_dirs: impl FnOnce() -> Vec<(PathBuf, &'static str)>,
) -> (Vec<(PathBuf, &'static str)>, Asked) {
    let dirs = read_dirs();
    let asked: Vec<PathBuf> = dirs.iter().map(|(d, _)| d.clone()).chain(paths).collect();
    check(&asked, probe);
    (dirs, asked.into_iter().collect())
}

/// Stats `paths` in order into `found`, skipping a path on a mount another check is statting
/// on: it keeps what was found before.
fn check(paths: &[PathBuf], probe: &Probe) {
    for p in paths {
        let mount = mount_of(p, probe.mounts).to_path_buf();
        let free = (probe.statting.lock()).map_or(true, |mut s| s.insert(mount.clone()));
        if !free {
            continue;
        }
        let what = match (probe.stat)(p) {
            Ok(m) if m.is_file() => Found::File,
            Ok(m) if m.is_dir() => Found::Dir,
            _ => Found::Neither,
        };
        if let Ok(mut s) = probe.statting.lock() {
            s.remove(&mount);
        }
        if let Ok(mut f) = probe.found.lock() {
            f.insert(p.clone(), what);
        }
    }
}

/// The mount point of `mounts` holding `path` by its spelling (a symlink is not followed, as
/// that could wait on the mount), or `path` itself when none does.
fn mount_of<'a>(path: &'a Path, mounts: &'a [PathBuf]) -> &'a Path {
    (mounts.iter())
        .filter(|m| path.starts_with(m))
        .max_by_key(|m| m.as_os_str().len())
        .map_or(path, PathBuf::as_path)
}

impl State {
    /// Records `path`, opened by `app` (else its type's default program), in `recently-used.xbel`
    /// on a file job, unless `[privacy] recent_files` is off.
    pub fn record_recent(&mut self, path: PathBuf, app: Option<AppEntry>) {
        if !self.config.recent_files {
            return;
        }
        let mime = (path.file_name())
            .and_then(|n| self.with_mime(|db| db.mime_of_name(&n.to_string_lossy())));
        let app = app.or_else(|| {
            let m = mime.as_deref()?;
            reprisefs::props::default_app_entry(m, &self.app_index).cloned()
        });
        self.spawn_fs_job(move || {
            if let Err(e) = reprisefs::recent::record(&path, app.as_ref(), mime.as_deref()) {
                log::warn!("recent files: could not record {}: {e}", path.display());
            }
            Box::new(|_: &mut State| {})
        });
    }

    /// Reads the recent files again, then checks them and the document folders, all off the
    /// loop; open lists are relisted as each answers.
    pub fn refresh_recent(&mut self) {
        self.recent.check_after_read = true;
        self.read_recent();
    }

    /// Reads `recently-used.xbel` off the loop, or once more after the read under way.
    pub fn read_recent(&mut self) {
        let Some(mut files) = self.recent.files.take() else {
            self.recent.reread = true;
            return;
        };
        self.spawn_fs_job(move || {
            let marks = files.bookmarks().to_vec();
            Box::new(move |s: &mut State| s.recent_read(files, marks))
        });
    }

    fn recent_read(&mut self, files: RecentFiles, marks: Vec<Bookmark>) {
        let changed = self.recent.take_read(files, marks);
        if std::mem::take(&mut self.recent.reread) {
            self.read_recent();
        } else if std::mem::take(&mut self.recent.check_after_read) {
            self.check_recent();
        }
        if changed {
            self.relist_recent();
        }
    }

    /// Checks the bookmarks and document folders off the loop, or once more after the check
    /// under way.
    pub fn check_recent(&mut self) {
        let (files, now) = (self.config.recent_files, Instant::now());
        let Some((id, paths)) = self.recent.start_check(now, files) else {
            self.recent.recheck = true;
            if let Some(wait) = self.recent.checks.retry_in(now) {
                self.recheck_recent_in(wait);
            }
            return;
        };
        let (found, statting) = (self.recent.found.clone(), self.recent.statting.clone());
        self.spawn_fs_job(move || {
            let table = std::fs::read("/proc/self/mounts").unwrap_or_default();
            let mounts: Vec<PathBuf> = (reprisefs::places::mount_table(&table).into_iter())
                .map(|(m, _)| m)
                .collect();
            let probe = Probe {
                found: &found,
                statting: &statting,
                mounts: &mounts,
                stat: |p| std::fs::metadata(p),
            };
            let (dirs, asked) =
                check_with_dirs(paths, &probe, reprisefs::places::document_dir_paths);
            Box::new(move |s: &mut State| {
                s.recent.land_check(id, dirs, asked);
                if std::mem::take(&mut s.recent.recheck) {
                    s.check_recent();
                }
                s.relist_recent();
                s.relist_start_places();
            })
        });
    }

    /// Starts the check `recheck` asks for after `wait`, when the running ones may be stuck.
    fn recheck_recent_in(&mut self, wait: std::time::Duration) {
        use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
        if std::mem::replace(&mut self.recent.retry_armed, true) {
            return;
        }
        let added = self
            .handle
            .insert_source(Timer::from_duration(wait), |_, _, state| {
                state.recent.retry_armed = false;
                if std::mem::take(&mut state.recent.recheck) {
                    state.check_recent();
                }
                TimeoutAction::Drop
            });
        if let Err(e) = added {
            self.recent.retry_armed = false;
            log::warn!("recent check retry timer: {e}");
        }
    }

    /// Relists the open Documents menus and jump lists, the Start panel's included, from the last
    /// read and check.
    fn relist_recent(&mut self) {
        let mut depth = 0;
        while depth < self.menus.len() {
            let m = &self.menus[depth];
            let (y, old_h) = (m.y, m.layout.height);
            let fresh = match &m.kind {
                MenuKind::Documents { dirs, icon_px, .. } => {
                    Some(self.documents_rows(*icon_px, *dirs))
                }
                MenuKind::JumpList {
                    rows,
                    target,
                    button,
                } => {
                    let (rows, target, button) = (rows.clone(), target.clone(), *button);
                    let recent = self.jump_recent(&target);
                    let (items, rows) = jump_relisted(&self.menus[depth].items, &rows, &recent);
                    Some((
                        items,
                        MenuKind::JumpList {
                            rows,
                            target,
                            button,
                        },
                        RowIcons::default(),
                    ))
                }
                _ => None,
            };
            depth += 1;
            let Some((items, kind, icons)) = fresh else {
                continue;
            };
            let usable = self.usable_area(Some(&self.menus[depth - 1].output));
            let (text, size_px) = (&self.text, self.config.theme.fonts.size_px);
            let m = &mut self.menus[depth - 1];
            let fit = |l: &MenuLayout, items: &[MenuItem]| {
                l.fit_column(text, items, size_px, usable.width)
            };
            if !m.relist_to(items, kind, icons, fit) {
                continue;
            }
            let (tree, new_w, new_h) = (m.tree, m.layout.width, m.layout.height);
            let x = m.x.min(usable.right() - new_w).max(usable.x);
            let y = keep_bottom(y, old_h, new_h, usable);
            (m.x, m.y) = (x, y);
            self.scene.set_position(tree, x, y);
            self.close_menus_above(depth - 1);
            crate::tooltip::hover_menu(self, depth - 1, None);
            self.redraw_menu(depth - 1);
        }
        self.relist_start_jumps();
        crate::recentwatch::follow_recent(self);
    }
}

/// The top of a menu at `y` resized from `old_h` to `new_h`: one standing on the `usable` area's
/// bottom (opened up from the taskbar) keeps its bottom edge, and none grows past it.
fn keep_bottom(y: i32, old_h: i32, new_h: i32, usable: Rect) -> i32 {
    if y + old_h >= usable.bottom() {
        y + old_h - new_h
    } else {
        y.min(usable.bottom() - new_h)
    }
    .max(usable.y)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::startmenu::MenuStyle;
    use std::time::{Duration, Instant};

    fn xbel(paths: &[&Path]) -> String {
        let marks: String = paths
            .iter()
            .enumerate()
            .map(|(i, p)| {
                format!(
                    "<bookmark href=\"file://{}\" modified=\"2026-01-0{}T00:00:00Z\"></bookmark>\n",
                    p.display(),
                    9 - i
                )
            })
            .collect();
        format!("<xbel>{marks}</xbel>")
    }

    /// What `State::read_recent` does off the loop and back on it.
    fn read(docs: &mut RecentDocs) -> bool {
        let mut files = docs.files.take().unwrap();
        let marks = files.bookmarks().to_vec();
        docs.take_read(files, marks)
    }

    fn probe<'a>(docs: &'a RecentDocs, mounts: &'a [PathBuf], stat: Stat) -> Probe<'a> {
        Probe {
            found: &docs.found,
            statting: &docs.statting,
            mounts,
            stat,
        }
    }

    fn checked(docs: &mut RecentDocs) {
        let (id, paths) = docs.start_check(Instant::now(), true).unwrap();
        let (dirs, asked) =
            check_with_dirs(paths, &probe(docs, &[], |p| std::fs::metadata(p)), Vec::new);
        assert!(docs.land_check(id, dirs, asked));
    }

    fn open_then_stat(p: &Path) -> io::Result<std::fs::Metadata> {
        std::fs::File::open(p)?.metadata()
    }

    /// The menus read only the bookmarks and what the check found: with the check blocked on a
    /// path that does not answer, they list at once, a path not checked yet included, and drop
    /// a missing file and a folder as soon as the check has passed them.
    #[test]
    fn recent_files_list_while_the_check_hangs() {
        use std::os::unix::fs::OpenOptionsExt;
        let dir = std::env::temp_dir().join(format!("reprise-recentdocs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("folder")).unwrap();
        let (kept, gone, folder, hang, later) = (
            dir.join("kept.txt"),
            dir.join("gone.txt"),
            dir.join("folder"),
            dir.join("hang"),
            dir.join("later.txt"),
        );
        std::fs::write(&kept, "").unwrap();
        let c = std::ffi::CString::new(hang.as_os_str().as_encoded_bytes()).unwrap();
        // SAFETY: `c` is a valid NUL-terminated path for the call's duration.
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        let xbel_path = dir.join("recently-used.xbel");
        let order = [&*kept, &*gone, &*folder, &*hang, &*later];
        std::fs::write(&xbel_path, xbel(&order)).unwrap();
        let mut docs = RecentDocs::at(RecentFiles::at(Some(xbel_path)));
        read(&mut docs);
        assert_eq!(docs.current(10).len(), 5, "nothing checked: all listed");

        let (_, paths) = docs.start_check(Instant::now(), true).unwrap();
        assert!(
            docs.start_check(Instant::now(), true).is_none(),
            "one check at a time"
        );
        let (found, statting) = (docs.found.clone(), docs.statting.clone());
        let job = std::thread::spawn(move || {
            let probe = Probe {
                found: &found,
                statting: &statting,
                mounts: &[],
                stat: open_then_stat,
            };
            check(&paths, &probe)
        });
        let end = Instant::now() + Duration::from_secs(5);
        while docs.found(&folder).is_none() {
            assert!(Instant::now() < end, "timed out waiting for the check");
            std::thread::sleep(Duration::from_millis(5));
        }
        let listed = Instant::now();
        assert_eq!(
            docs.current(10),
            [kept.clone(), hang.clone(), later.clone()]
        );
        assert!(listed.elapsed() < Duration::from_secs(1));
        assert!(!job.is_finished(), "the check still hangs");

        // A writer opening the pipe answers the blocked open.
        let _w = std::fs::OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&hang)
            .unwrap();
        job.join().unwrap();
        assert_eq!(docs.current(10), [kept], "the pipe is not a file");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A check stuck on a dead mount lets a newer one start once it is stuck; the newer one
    /// skips that mount's paths, drops a file deleted elsewhere, and the stuck one landing
    /// later is stale.
    #[test]
    fn a_check_stuck_on_a_dead_mount_lets_a_newer_check_drop_a_deleted_file() {
        use std::os::unix::fs::OpenOptionsExt;
        let dir = std::env::temp_dir().join(format!("reprise-recentdead-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mount = dir.join("mnt");
        std::fs::create_dir_all(&mount).unwrap();
        let (hang, remote, local) = (mount.join("hang"), mount.join("b.txt"), dir.join("a.txt"));
        std::fs::write(&remote, "").unwrap();
        std::fs::write(&local, "").unwrap();
        let c = std::ffi::CString::new(hang.as_os_str().as_encoded_bytes()).unwrap();
        // SAFETY: `c` is a valid NUL-terminated path for the call's duration.
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        let xbel_path = dir.join("recently-used.xbel");
        std::fs::write(&xbel_path, xbel(&[&*hang, &*remote, &*local])).unwrap();
        let mut docs = RecentDocs::at(RecentFiles::at(Some(xbel_path)));
        read(&mut docs);
        let mounts = vec![PathBuf::from("/"), mount.clone()];

        let t0 = Instant::now();
        let (first, paths) = docs.start_check(t0, true).unwrap();
        let (found, statting, table) = (docs.found.clone(), docs.statting.clone(), mounts.clone());
        let job = std::thread::spawn(move || {
            let probe = Probe {
                found: &found,
                statting: &statting,
                mounts: &table,
                stat: open_then_stat,
            };
            check_with_dirs(paths, &probe, Vec::new)
        });
        let end = Instant::now() + Duration::from_secs(5);
        while !docs.statting.lock().unwrap().contains(&mount) {
            assert!(Instant::now() < end, "timed out waiting for the check");
            std::thread::sleep(Duration::from_millis(5));
        }

        std::fs::remove_file(&local).unwrap();
        let fresh = t0 + crate::fsjobs::STUCK / 2;
        assert!(
            docs.start_check(fresh, true).is_none(),
            "the first is fresh"
        );
        let (second, paths) = docs.start_check(t0 + crate::fsjobs::STUCK, true).unwrap();
        let (dirs, asked) =
            check_with_dirs(paths, &probe(&docs, &mounts, open_then_stat), Vec::new);
        assert!(docs.land_check(second, dirs, asked));
        assert_eq!(
            docs.current(10),
            [hang.clone(), remote.clone()],
            "a.txt dropped; the dead mount's paths stay unchecked"
        );
        assert!(!job.is_finished(), "the first check still hangs");

        // A writer opening the pipe answers the blocked open.
        let _w = std::fs::OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&hang)
            .unwrap();
        let (dirs, asked) = job.join().unwrap();
        assert!(!docs.land_check(first, dirs, asked), "the first is stale");
        assert_eq!(docs.current(10), [remote], "the pipe is not a file");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_document_folder_found_gone_drops_its_start_place_and_one_made_comes_back() {
        let home = std::env::temp_dir().join(format!("reprise-recentdirs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let (documents, music) = (home.join("Documents"), home.join("Music"));
        std::fs::create_dir_all(&documents).unwrap();
        std::fs::create_dir_all(&music).unwrap();
        let docs = RecentDocs::at(RecentFiles::at(None));
        let dirs = || {
            vec![
                (documents.clone(), "folder-documents"),
                (music.clone(), "folder-music"),
            ]
        };
        let places = |docs: &RecentDocs| -> Vec<String> {
            crate::startmenu::places(false, &docs.found_dirs(dirs()))
                .into_iter()
                .filter_map(|e| match e {
                    crate::config::MenuEntry::Item { label, .. } => Some(label),
                    crate::config::MenuEntry::Separator => None,
                })
                .take(3)
                .collect()
        };
        let checked = |docs: &RecentDocs| {
            let paths: Vec<PathBuf> = dirs().into_iter().map(|(d, _)| d).collect();
            check(&paths, &probe(docs, &[], |p| std::fs::metadata(p)))
        };
        checked(&docs);
        assert_eq!(places(&docs), ["{user}", "Documents", "Music"]);
        std::fs::remove_dir(&music).unwrap();
        checked(&docs);
        assert_eq!(places(&docs), ["{user}", "Documents", "Computer"]);
        std::fs::create_dir(&music).unwrap();
        checked(&docs);
        assert_eq!(places(&docs), ["{user}", "Documents", "Music"]);
        let _ = std::fs::remove_dir_all(&home);
    }

    /// The document folders come from the check's own read of `user-dirs.dirs`: none before
    /// the first check answers, then the ones it read that exist, and the list follows a folder
    /// moved in `user-dirs.dirs` at the next check.
    #[test]
    fn the_document_folders_are_read_by_the_check_not_on_the_loop() {
        let home = std::env::temp_dir().join(format!("reprise-userdirs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join("Documents")).unwrap();
        std::fs::create_dir_all(home.join("Papers")).unwrap();
        let mut docs = RecentDocs::at(RecentFiles::at(None));
        assert_eq!(docs.document_dirs(), [], "nothing read yet");

        let first = || {
            vec![
                (home.join("Documents"), "folder-documents"),
                (home.join("Music"), "folder-music"),
            ]
        };
        let moved = || vec![(home.join("Papers"), "folder-documents")];
        let (id, paths) = docs.start_check(Instant::now(), false).unwrap();
        assert_eq!(paths, Vec::<PathBuf>::new());
        let (dirs, asked) =
            check_with_dirs(paths, &probe(&docs, &[], |p| std::fs::metadata(p)), first);
        assert!(docs.land_check(id, dirs, asked));
        assert_eq!(
            docs.document_dirs(),
            [(home.join("Documents"), "folder-documents")],
            "Music does not exist"
        );
        assert!(docs.is_document_dir(&home.join("Music")));

        let (id, paths) = docs.start_check(Instant::now(), false).unwrap();
        let (dirs, asked) =
            check_with_dirs(paths, &probe(&docs, &[], |p| std::fs::metadata(p)), moved);
        assert!(docs.land_check(id, dirs, asked));
        assert_eq!(
            docs.document_dirs(),
            [(home.join("Papers"), "folder-documents")]
        );
        assert!(!docs.is_document_dir(&home.join("Documents")));
        let _ = std::fs::remove_dir_all(&home);
    }

    fn test_output() -> smithay::output::Output {
        smithay::output::Output::new(
            "T-1".into(),
            smithay::output::PhysicalProperties {
                size: (0, 0).into(),
                subpixel: smithay::output::Subpixel::Unknown,
                make: "Reprise".into(),
                model: "Test".into(),
                serial_number: "0".into(),
            },
        )
    }

    /// The rows `State::documents_rows` builds under one document folder.
    fn documents(dir: &Path, files: &[PathBuf]) -> (Vec<MenuItem>, MenuKind) {
        let name = |p: &Path| p.file_name().unwrap().to_string_lossy().into_owned();
        let mut items = vec![MenuItem::entry(name(dir)), MenuItem::Separator];
        let mut paths = vec![Some(dir.to_path_buf()), None];
        for f in files {
            items.push(MenuItem::entry(name(f)));
            paths.push(Some(f.clone()));
        }
        if files.is_empty() {
            items.push(MenuItem::entry("(Empty)").enabled(false));
            paths.push(None);
        }
        let tips = vec![None; paths.len()];
        let kind = MenuKind::Documents {
            paths,
            tips,
            dirs: true,
            icon_px: 16,
        };
        (items, kind)
    }

    #[test]
    fn an_open_documents_menu_takes_new_files_and_drops_gone_ones_in_place() {
        use crate::menuview::{Menu, MenuFrom};
        let dir = std::env::temp_dir().join(format!("reprise-recentopen-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let [a, b, c, d] = ["a.txt", "b.txt", "c.txt", "d.txt"].map(|n| dir.join(n));
        for f in [&a, &b, &c, &d] {
            std::fs::write(f, "").unwrap();
        }
        let xbel_path = dir.join("recently-used.xbel");
        std::fs::write(&xbel_path, xbel(&[&a, &b])).unwrap();
        let mut docs = RecentDocs::at(RecentFiles::at(Some(xbel_path.clone())));
        read(&mut docs);

        let mut scene = crate::compositor::DeskScene::new();
        let (items, kind) = documents(&dir, &docs.current(3));
        let mut menu = Menu::for_test(&mut scene, test_output(), kind, MenuFrom::Bar);
        let style = MenuStyle::submenu(&crate::config::Config::default().theme);
        menu.layout = MenuLayout::compute_styled(&style, &items, usize::MAX);
        menu.items = items;
        menu.hovered = Some(3);
        let relist = |menu: &mut Menu, docs: &RecentDocs| {
            let (items, kind) = documents(&dir, &docs.current(3));
            menu.relist_to(items, kind, RowIcons::default(), |l, _| l.column_width)
        };
        let listed = |menu: &Menu| -> Vec<Option<PathBuf>> {
            let MenuKind::Documents { paths, .. } = &menu.kind else {
                unreachable!()
            };
            paths[2..].to_vec()
        };

        std::fs::write(&xbel_path, xbel(&[&c, &a, &b])).unwrap();
        assert!(read(&mut docs));
        assert!(relist(&mut menu, &docs));
        assert_eq!(
            listed(&menu),
            [Some(c.clone()), Some(a.clone()), Some(b.clone())]
        );
        assert_eq!(menu.items[2], MenuItem::entry("c.txt"));
        assert_eq!(menu.hovered, Some(4), "b.txt kept the hover a row down");
        assert_eq!(
            menu.layout,
            MenuLayout::compute_styled(&style, &menu.items, usize::MAX)
        );
        assert!(!read(&mut docs), "the xbel did not change");
        assert!(!relist(&mut menu, &docs));

        std::fs::write(&xbel_path, xbel(&[&d, &c, &a, &b])).unwrap();
        read(&mut docs);
        assert!(relist(&mut menu, &docs));
        assert_eq!(
            listed(&menu),
            [Some(d.clone()), Some(c.clone()), Some(a.clone())],
            "b.txt pushed past the limit"
        );
        assert_eq!(menu.hovered, Some(4), "the hover took a.txt above");

        std::fs::remove_file(&c).unwrap();
        checked(&mut docs);
        assert!(relist(&mut menu, &docs));
        assert_eq!(
            listed(&menu),
            [Some(d.clone()), Some(a.clone()), Some(b.clone())]
        );
        assert_eq!(menu.hovered, Some(3), "a.txt slid up under the hover");

        for f in [&a, &b, &d] {
            std::fs::remove_file(f).unwrap();
        }
        checked(&mut docs);
        assert!(relist(&mut menu, &docs));
        assert_eq!(menu.items[2], MenuItem::entry("(Empty)").enabled(false));
        assert_eq!(menu.items.len(), 3);
        assert_eq!(menu.hovered, Some(0), "(Empty) takes no hover");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_relisted_jump_list_swaps_only_its_recent_files() {
        use crate::taskbar::JumpRow;
        let (a, c) = (PathBuf::from("/srv/a.txt"), PathBuf::from("/srv/c.txt"));
        let tail = || {
            (
                vec![MenuItem::entry("Notepad"), MenuItem::entry("Pin")],
                vec![Some(JumpRow::Launch), Some(JumpRow::Pin)],
            )
        };
        let (items, rows) = tail();
        let (items, rows) = jump_relisted(&items, &rows, std::slice::from_ref(&a));
        assert_eq!(
            rows,
            [
                Some(JumpRow::Recent(a.clone())),
                None,
                Some(JumpRow::Launch),
                Some(JumpRow::Pin)
            ]
        );
        let (items, rows) = jump_relisted(&items, &rows, &[c.clone(), a.clone()]);
        assert_eq!(
            rows[..3],
            [Some(JumpRow::Recent(c)), Some(JumpRow::Recent(a)), None]
        );
        assert_eq!(items[3..], tail().0);
        assert_eq!(jump_relisted(&items, &rows, &[]), tail());
    }

    #[test]
    fn a_relisted_menu_on_the_taskbar_keeps_its_bottom_edge() {
        let usable = Rect::new(0, 0, 800, 570);
        assert_eq!(keep_bottom(370, 200, 150, usable), 420);
        assert_eq!(keep_bottom(370, 200, 250, usable), 320, "grown upwards");
        assert_eq!(
            keep_bottom(300, 200, 300, usable),
            270,
            "a menu grown past the bottom moves up"
        );
        assert_eq!(
            keep_bottom(100, 200, 150, usable),
            100,
            "a menu off the bottom keeps its top"
        );
    }
}
