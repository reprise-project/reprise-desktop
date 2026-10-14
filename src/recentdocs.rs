//! The recent files and document folders the Start menus and jump lists show. A recent file can
//! sit on a mount that never answers, so the bookmarks and what each path is come from file jobs:
//! the loop shows what the last read and check found, a path not checked yet included.

use reprisefs::apps::AppEntry;
use reprisefs::recent::{Bookmark, RecentFiles};
use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::compositor::State;
use crate::menuview::{MenuKind, jump_relisted};
use crate::raster::Rect;

/// What a path was found to be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Found {
    File,
    Dir,
    Neither,
}

type Stat = fn(&Path) -> io::Result<std::fs::Metadata>;

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
    /// A check is running; another is not started until it answers.
    checking: bool,
    /// A check was asked for while one ran: check again when it answers.
    recheck: bool,
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
            checking: false,
            recheck: false,
        }
    }

    fn found(&self, path: &Path) -> Option<Found> {
        self.found.lock().ok()?.get(path).copied()
    }

    /// The newest `limit` bookmarks `keep` takes that were not found missing or a folder.
    fn pick(&self, limit: usize, keep: impl Fn(&Bookmark) -> bool) -> Vec<PathBuf> {
        let found = self.found.lock().unwrap_or_else(|e| e.into_inner());
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

    /// The document folders that were not found missing.
    pub fn document_dirs(&self) -> Vec<(PathBuf, &'static str)> {
        self.found_dirs(reprisefs::places::document_dir_paths())
    }

    fn found_dirs(&self, dirs: Vec<(PathBuf, &'static str)>) -> Vec<(PathBuf, &'static str)> {
        dirs.into_iter()
            .filter(|(dir, _)| matches!(self.found(dir), None | Some(Found::Dir)))
            .collect()
    }

    /// The paths a check asks about (the document folders, and the bookmarks with `files`), or
    /// None while a check is running.
    fn paths_to_check(&mut self, files: bool) -> Option<Vec<PathBuf>> {
        if self.checking {
            return None;
        }
        self.checking = true;
        let mut paths: Vec<PathBuf> = reprisefs::places::document_dir_paths()
            .into_iter()
            .map(|(dir, _)| dir)
            .collect();
        if files {
            paths.extend(self.marks.iter().map(|b| b.path.clone()));
        }
        Some(paths)
    }

    /// Takes what a read found; true when the bookmarks changed.
    fn take_read(&mut self, files: RecentFiles, marks: Vec<Bookmark>) -> bool {
        self.files = Some(files);
        let changed = self.marks != marks;
        self.marks = marks;
        changed
    }
}

/// Stats `paths` in order into `found`, then drops what `found` holds of other paths.
fn check(paths: Vec<PathBuf>, found: &Mutex<HashMap<PathBuf, Found>>, stat: Stat) {
    for p in &paths {
        let what = match stat(p) {
            Ok(m) if m.is_file() => Found::File,
            Ok(m) if m.is_dir() => Found::Dir,
            _ => Found::Neither,
        };
        if let Ok(mut f) = found.lock() {
            f.insert(p.clone(), what);
        }
    }
    let asked: HashSet<&PathBuf> = paths.iter().collect();
    if let Ok(mut f) = found.lock() {
        f.retain(|p, _| asked.contains(p));
    }
}

impl State {
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
        let files = self.config.recent_files;
        let Some(paths) = self.recent.paths_to_check(files) else {
            self.recent.recheck = true;
            return;
        };
        let found = self.recent.found.clone();
        self.spawn_fs_job(move || {
            check(paths, &found, |p| std::fs::metadata(p));
            Box::new(|s: &mut State| {
                s.recent.checking = false;
                if std::mem::take(&mut s.recent.recheck) {
                    s.check_recent();
                }
                s.relist_recent();
                s.relist_start_places();
            })
        });
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
                    ))
                }
                _ => None,
            };
            depth += 1;
            let Some((items, kind)) = fresh else {
                continue;
            };
            let m = &mut self.menus[depth - 1];
            if !m.relist_to(items, kind) {
                continue;
            }
            let (tree, x, new_h) = (m.tree, m.x, m.layout.height);
            let output = m.output.clone();
            let usable = self.usable_area(Some(&output));
            let y = keep_bottom(y, old_h, new_h, usable);
            self.menus[depth - 1].y = y;
            self.scene.set_position(tree, x, y);
            self.close_menus_above(depth - 1);
            crate::tooltip::hover_menu(self, depth - 1, None);
            self.redraw_menu(depth - 1);
        }
        self.relist_start_jumps();
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
    use crate::startmenu::{MenuItem, MenuLayout, MenuStyle};
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

    fn checked(docs: &mut RecentDocs) {
        let paths = docs.paths_to_check(true).unwrap();
        check(paths, &docs.found, |p| std::fs::metadata(p));
        docs.checking = false;
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

        let paths = docs.paths_to_check(true).unwrap();
        assert!(docs.paths_to_check(true).is_none(), "one check at a time");
        let found = docs.found.clone();
        let job = std::thread::spawn(move || check(paths, &found, open_then_stat));
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
            check(
                dirs().into_iter().map(|(d, _)| d).collect(),
                &docs.found,
                |p| std::fs::metadata(p),
            )
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
            menu.relist_to(items, kind)
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
