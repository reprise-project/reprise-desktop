//! The recent files and document folders the Start menus and jump lists show. A recent file can
//! sit on a mount that never answers, so what each path is comes from a file job: the loop reads
//! the bookmarks and what the last check found, and shows a path not checked yet.

use reprisefs::apps::AppEntry;
use reprisefs::recent::{Bookmark, RecentFiles};
use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::compositor::State;
use crate::raster::Rect;

/// What a path was found to be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Found {
    File,
    Dir,
    Neither,
}

type Stat = fn(&Path) -> io::Result<std::fs::Metadata>;

#[derive(Default)]
pub struct RecentDocs {
    files: RecentFiles,
    /// Filled by the check job one path at a time, so a path that hangs keeps only itself and
    /// the paths after it unchecked.
    found: Arc<Mutex<HashMap<PathBuf, Found>>>,
    /// A check is running; another is not started until it answers.
    checking: bool,
}

impl RecentDocs {
    fn found(&self, path: &Path) -> Option<Found> {
        self.found.lock().ok()?.get(path).copied()
    }

    /// The newest `limit` bookmarks `keep` takes that were not found missing or a folder.
    fn pick(&mut self, limit: usize, keep: impl Fn(&Bookmark) -> bool) -> Vec<PathBuf> {
        let marks = self.files.bookmarks();
        let found = self.found.lock().unwrap_or_else(|e| e.into_inner());
        marks
            .iter()
            .filter(|b| keep(b) && matches!(found.get(&b.path), None | Some(Found::File)))
            .take(limit)
            .map(|b| b.path.clone())
            .collect()
    }

    pub fn current(&mut self, limit: usize) -> Vec<PathBuf> {
        self.pick(limit, |_| true)
    }

    /// `current`, keeping only the files the application `entry` used: its jump list.
    pub fn for_app(&mut self, limit: usize, entry: &AppEntry) -> Vec<PathBuf> {
        self.pick(limit, |b| reprisefs::recent::used_by_entry(b, entry))
    }

    /// The document folders that were not found missing.
    pub fn document_dirs(&self) -> Vec<(PathBuf, &'static str)> {
        reprisefs::places::document_dir_paths()
            .into_iter()
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
            paths.extend(self.files.bookmarks().iter().map(|b| b.path.clone()));
        }
        Some(paths)
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
    /// Checks the recent files and document folders off the loop, unless a check is running. The
    /// menus opened meanwhile show what the last check found.
    pub fn refresh_recent(&mut self) {
        let files = self.config.recent_files;
        let Some(paths) = self.recent.paths_to_check(files) else {
            return;
        };
        let found = self.recent.found.clone();
        self.spawn_fs_job(move || {
            check(paths, &found, |p| std::fs::metadata(p));
            Box::new(State::recent_checked)
        });
    }

    /// The check answered: the open Documents menus and jump lists drop the files it found gone.
    fn recent_checked(&mut self) {
        self.recent.checking = false;
        let found = self.recent.found.clone();
        let found = found.lock().unwrap_or_else(|e| e.into_inner());
        let gone = |p: &Path| found.get(p) == Some(&Found::Neither);
        let mut depth = 0;
        while let Some(m) = self.menus.get_mut(depth) {
            let (y, old_h) = (m.y, m.layout.height);
            depth += 1;
            if !m.drop_gone_files(gone) {
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
        self.drop_gone_jumps(gone);
    }
}

/// The top of a menu shrunk from `old_h` to `new_h` at `y`: a menu standing on the bottom of the
/// `usable` area (one opened upwards from the taskbar) keeps its bottom edge there.
fn keep_bottom(y: i32, old_h: i32, new_h: i32, usable: Rect) -> i32 {
    if y + old_h >= usable.bottom() {
        y + old_h - new_h
    } else {
        y
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let mut docs = RecentDocs {
            files: RecentFiles::at(Some(xbel_path)),
            ..Default::default()
        };
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

    /// A Documents menu opened before the check answered drops the file the check found
    /// deleted, keeps the hover on the row sliding into its place, and shows (Empty) once its
    /// last recent file goes.
    #[test]
    fn an_open_documents_menu_drops_the_files_the_check_found_gone() {
        use crate::menuview::{Menu, MenuFrom, MenuKind};
        use crate::startmenu::{MenuItem, MenuLayout, MenuStyle};
        let dir = std::env::temp_dir().join(format!("reprise-recentopen-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (a, b) = (dir.join("a.txt"), dir.join("b.txt"));
        std::fs::write(&a, "").unwrap();
        std::fs::write(&b, "").unwrap();
        let xbel_path = dir.join("recently-used.xbel");
        std::fs::write(&xbel_path, xbel(&[&a, &b])).unwrap();
        let mut docs = RecentDocs {
            files: RecentFiles::at(Some(xbel_path)),
            ..Default::default()
        };
        let listed = docs.current(10);
        assert_eq!(listed, [a.clone(), b.clone()]);

        let mut scene = crate::compositor::DeskScene::new();
        let kind = MenuKind::Documents {
            paths: vec![Some(dir.clone()), None, Some(a.clone()), Some(b.clone())],
            tips: vec![None; 4],
            files: 2,
        };
        let mut menu = Menu::for_test(&mut scene, test_output(), kind, MenuFrom::Bar);
        menu.items = vec![
            MenuItem::entry("Documents"),
            MenuItem::Separator,
            MenuItem::entry("a.txt"),
            MenuItem::entry("b.txt"),
        ];
        let style = MenuStyle::submenu(&crate::config::Config::default().theme);
        menu.layout = MenuLayout::compute_styled(&style, &menu.items, usize::MAX);
        menu.hovered = Some(2);

        let checked = |docs: &mut RecentDocs| {
            let paths = docs.paths_to_check(true).unwrap();
            check(paths, &docs.found, |p| std::fs::metadata(p));
            docs.checking = false;
            let found = docs.found.lock().unwrap().clone();
            move |p: &Path| found.get(p) == Some(&Found::Neither)
        };
        std::fs::remove_file(&a).unwrap();
        assert!(menu.drop_gone_files(checked(&mut docs)));
        assert_eq!(menu.items[2], MenuItem::entry("b.txt"));
        assert_eq!(menu.items.len(), 3);
        assert_eq!(menu.hovered, Some(2), "b.txt slid under the hover");
        assert_eq!(
            menu.layout,
            MenuLayout::compute_styled(&style, &menu.items, usize::MAX)
        );
        let MenuKind::Documents { paths, files, .. } = &menu.kind else {
            unreachable!()
        };
        assert_eq!((paths[2].as_deref(), *files), (Some(&*b), 2));
        assert!(
            !menu.drop_gone_files(checked(&mut docs)),
            "nothing more gone"
        );

        std::fs::remove_file(&b).unwrap();
        assert!(menu.drop_gone_files(checked(&mut docs)));
        assert_eq!(menu.items[2], MenuItem::entry("(Empty)").enabled(false));
        assert_eq!(menu.hovered, Some(0), "(Empty) takes no hover");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_shrunk_menu_on_the_taskbar_keeps_its_bottom_edge() {
        let usable = Rect::new(0, 0, 800, 570);
        assert_eq!(keep_bottom(370, 200, 150, usable), 420);
        assert_eq!(
            keep_bottom(100, 200, 150, usable),
            100,
            "a menu off the bottom keeps its top"
        );
    }
}
