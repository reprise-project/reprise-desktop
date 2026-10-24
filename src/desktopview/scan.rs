use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use inotify::WatchMask;
use reprisefs::{apps, places, sendto};
use reprisekit::selection::Selection;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};

use crate::{
    compositor::State,
    desktop::{self, DesktopEntry, ICON_PX, Target},
    dirwatch::{self, DirWatch, Want},
};

const SETTLE_MS: u64 = 100;

/// The icons the desktop lists: the system icons in `system_icons` order, then
/// the desktop directory's files when `show_files`. `system` makes one system
/// icon by config name, None to leave it out; `files` lists the directory and
/// runs only when the files show.
pub(super) fn listed(
    system_icons: &[String],
    show_files: bool,
    system: impl FnMut(&str) -> Option<DesktopEntry>,
    files: impl FnOnce() -> Vec<DesktopEntry>,
) -> Vec<DesktopEntry> {
    let mut entries: Vec<DesktopEntry> = system_icons
        .iter()
        .map(String::as_str)
        .filter_map(system)
        .collect();
    if show_files {
        entries.extend(files());
    }
    entries
}

/// The desktop's icons as [`listed`] gives them, read from the file system: the places'
/// folders, the home Recycle Bin's fullness (full anyway with `drive_bins_full`) and the
/// desktop folder's files.
pub(super) fn read_icons(
    dir: &Path,
    mime: &apps::MimeDb,
    names: &BTreeMap<String, String>,
    system_icons: &[String],
    show_files: bool,
    shown: reprisefs::names::Extensions,
    drive_bins_full: bool,
) -> Vec<DesktopEntry> {
    let mut entries = listed(
        system_icons,
        show_files,
        |name| {
            if name == "recycle-bin" {
                let files = places::trash_dir()?;
                let empty = !drive_bins_full && places::trash_empty(&files);
                Some(desktop::recycle_bin(files, empty))
            } else {
                let e = desktop::place_shown(name, names);
                if e.is_none() && desktop::Place::from_config(name).is_none() {
                    log::warn!("desktop: unknown system icon {name:?}");
                }
                e
            }
        },
        || desktop::scan(dir, mime, shown),
    );
    for e in &mut entries {
        if let (desktop::Kind::Place(_), Target::Open(p)) = (e.kind, &e.target) {
            e.is_dir = p.is_dir();
        }
    }
    entries
}

/// The icon files `entries` wear (a folder's or a launcher's `Icon=/...`), each decoded at `px`.
pub(super) fn icon_files(
    entries: &[DesktopEntry],
    px: i32,
) -> Vec<(String, Option<reprisekit::raster::Canvas>)> {
    let mut files: Vec<&str> = entries
        .iter()
        .map(|e| e.icon.as_str())
        .filter(|i| i.starts_with('/'))
        .collect();
    files.sort_unstable();
    files.dedup();
    files
        .into_iter()
        .map(|f| {
            (
                f.to_string(),
                reprisekit::icontheme::decode_file(Path::new(f), px, 1),
            )
        })
        .collect()
}

pub(super) fn migrate_launcher_keys(
    entries: &[DesktopEntry],
    positions: &mut BTreeMap<String, (i32, i32)>,
) -> bool {
    let mut moved = false;
    for e in entries {
        if !matches!(e.target, Target::Launch(_))
            || positions.contains_key(&e.name)
            || entries.iter().any(|o| o.name == e.label)
        {
            continue;
        }
        if let Some(cell) = positions.remove(&e.label) {
            positions.insert(e.name.clone(), cell);
            moved = true;
        }
    }
    moved
}

/// What the desktop watch covers besides the drives' bins. When `files` shows the folder's files:
/// the desktop folder (ATTRIB, because chmod +x turns a `.desktop` file into a launcher) and its
/// parent (for the folder being made). The home trash (`home_trash_wants`), so the Recycle Bin's
/// icon flips between empty and full; the drives' trashes as [`State::read_drive_bins`] last found them
/// join it on the loop.
pub(super) fn watch_wants(dir: &Path, files: bool, system_icons: &[String]) -> Vec<Want> {
    let mut want = Vec::new();
    if files {
        want.push(Want {
            dir: dir.to_path_buf(),
            mask: WatchMask::CREATE
                | WatchMask::DELETE
                | WatchMask::MOVED_FROM
                | WatchMask::MOVED_TO
                | WatchMask::CLOSE_WRITE
                | WatchMask::ATTRIB,
            only: Vec::new(),
        });
        if let (Some(parent), Some(name)) = (dir.parent(), dir.file_name()) {
            want.push(Want {
                dir: parent.to_path_buf(),
                mask: WatchMask::CREATE | WatchMask::MOVED_TO,
                only: vec![name.to_os_string()],
            });
        }
    }
    if system_icons.iter().any(|n| n == "recycle-bin")
        && let Some(root) = places::trash_root()
    {
        want.extend(home_trash_wants(&root));
    }
    want
}

pub(super) const TRASH_MASK: WatchMask = WatchMask::CREATE
    .union(WatchMask::DELETE)
    .union(WatchMask::MOVED_FROM)
    .union(WatchMask::MOVED_TO);

/// The home trash's watches (`places::trash_watches`), so the Recycle Bin
/// sees its folders made, moved back, deleted or moved away.
pub(super) fn home_trash_wants(root: &Path) -> Vec<Want> {
    places::trash_watches(root)
        .into_iter()
        .map(|(dir, only)| Want {
            dir,
            mask: TRASH_MASK,
            only: only.into_iter().collect(),
        })
        .collect()
}

/// Where each icon of `old` sits in `new`, matched by `DesktopEntry::id`; None
/// for one that is gone. Files are looked up by path in one map, so a rescan with
/// everything selected stays linear.
pub(super) fn remap(old: &[DesktopEntry], new: &[DesktopEntry]) -> Vec<Option<usize>> {
    // Reversed, so a path listed twice maps to its first index, as `find` does.
    let files: std::collections::HashMap<&Path, usize> = new
        .iter()
        .enumerate()
        .rev()
        .filter(|(_, e)| e.kind == desktop::Kind::File)
        .map(|(i, e)| (e.path.as_path(), i))
        .collect();
    old.iter()
        .map(|e| match e.kind {
            desktop::Kind::File => files.get(e.path.as_path()).copied(),
            _ => desktop::find(new, &e.id()),
        })
        .collect()
}

impl State {
    pub(super) fn desktop_watch_ready(&mut self) {
        let Some(view) = self.desktop.as_mut() else {
            return;
        };
        if view.watch.as_mut().is_some_and(|w| w.drain()) {
            self.settle_rescan();
        }
    }

    /// Rescan once changes stop arriving for `SETTLE_MS`.
    pub(super) fn settle_rescan(&mut self) {
        let Some(view) = self.desktop.as_mut() else {
            return;
        };
        if let Some(t) = view.settle.take() {
            self.handle.remove(t);
        }
        let timer = Timer::from_duration(Duration::from_millis(SETTLE_MS));
        let token = self.handle.insert_source(timer, |_, _, state| {
            if let Some(view) = state.desktop.as_mut() {
                view.settle = None;
            }
            state.rescan_desktop();
            TimeoutAction::Drop
        });
        match token {
            Ok(t) => {
                if let Some(view) = self.desktop.as_mut() {
                    view.settle = Some(t);
                }
            }
            // Without the timer every wakeup rescans at once.
            Err(_) => self.rescan_desktop(),
        }
    }

    /// Reads the desktop's icons off the loop, since a file on the desktop may sit on a mount that
    /// never answers; the icons on screen stay until the read lands. One read runs at a time.
    pub fn rescan_desktop(&mut self) {
        let first = self.first_output();
        let px = ICON_PX * self.draw_scale_at(first.as_ref());
        if self.config.system_icons.iter().any(|n| n == "recycle-bin") {
            self.read_drive_bins();
        }
        let State {
            desktop: Some(view),
            config,
            ..
        } = self
        else {
            return;
        };
        if view.scanning {
            view.rescan_again = true;
            return;
        }
        view.scanning = true;
        let (dir, mime, names) = (
            view.dir.clone(),
            view.mime.clone(),
            view.system_names.clone(),
        );
        let (system_icons, show_files, shown, drive_bins_full) = (
            config.system_icons.clone(),
            config.desktop_icons,
            config.desktop_extensions,
            view.bins.full,
        );
        let adder = view.watch.as_ref().map(DirWatch::adder);
        self.spawn_fs_job(move || {
            // Watch before reading, so a change between the two is not missed.
            let want = watch_wants(&dir, show_files, &system_icons);
            let added = adder.map(|a| a.run(&want));
            let entries = read_icons(
                &dir,
                &mime,
                &names,
                &system_icons,
                show_files,
                shown,
                drive_bins_full,
            );
            let art = icon_files(&entries, px);
            let send_dirs = sendto::Dirs::read(&dir);
            Box::new(move |state| {
                if let Some(view) = state.desktop.as_mut() {
                    view.send_dirs = send_dirs;
                }
                state.desktop_watched(want, added);
                state.desktop_scanned(entries, art, px)
            })
        });
    }

    /// Takes the desktop's own watches a rescan added, with `want` the folders it wanted, then
    /// adds the drives' bin folders the last drive bin read found, each on a file job of its own:
    /// a drive that stopped answering since holds only its own.
    fn desktop_watched(&mut self, mut want: Vec<Want>, added: Option<dirwatch::Added>) {
        let State {
            desktop: Some(view),
            config,
            ..
        } = self
        else {
            return;
        };
        let tried = want.len();
        if config.system_icons.iter().any(|n| n == "recycle-bin") {
            want.extend(view.bins.wants.iter().cloned());
        }
        let Some(w) = view.watch.as_mut() else {
            return;
        };
        w.plan(&want);
        if let Some(added) = added {
            w.adopt(added);
        }
        // A folder of the rescan's own that is missing was just looked for.
        let tried: Vec<&Path> = want[..tried].iter().map(|w| w.dir.as_path()).collect();
        let adds: Vec<_> = (w.plan(&want).each())
            .filter(|(dir, _)| {
                !tried.contains(&dir.as_path()) && view.watch_adds.insert(dir.clone())
            })
            .collect();
        for (dir, add) in adds {
            self.spawn_fs_job(move || {
                let added = add.run();
                Box::new(move |state: &mut State| {
                    if let Some(view) = state.desktop.as_mut() {
                        view.watch_adds.remove(&dir);
                        if let Some(w) = view.watch.as_mut() {
                            w.adopt(added);
                        }
                    }
                })
            });
        }
    }

    /// A desktop read is done: its icon files go into the icon cache, its plain files wear the
    /// theme's icons, and the icons on screen are replaced.
    fn desktop_scanned(
        &mut self,
        mut entries: Vec<DesktopEntry>,
        art: Vec<(String, Option<reprisekit::raster::Canvas>)>,
        px: i32,
    ) {
        let State {
            desktop: Some(view),
            icons,
            ..
        } = self
        else {
            return;
        };
        view.scanning = false;
        view.landed.clear();
        icons.put_files(art.into_iter().map(|(f, c)| (f, px, c)).collect());
        view.icons_read = (px, icons.file_drops());
        desktop::wear_file_icons(&mut entries, &view.mime, &|n| icons.has(n));
        let old = std::mem::replace(&mut view.entries, entries);
        let moved = remap(&old, &view.entries);
        let remap = |i: &usize| moved.get(*i).copied().flatten();
        view.selection = Selection {
            set: view.selection.set.iter().filter_map(remap).collect(),
            focus: view.selection.focus.as_ref().and_then(remap),
            anchor: None,
        };
        // The rename follows its file; it ends only when the file is gone.
        view.rename = view
            .rename
            .take()
            .and_then(|(i, edit)| Some((remap(&i)?, edit)));
        view.last_click.reset();
        if migrate_launcher_keys(&view.entries, &mut view.positions) {
            crate::state::save_desktop(view.auto_arrange, &view.positions);
        }
        let again = std::mem::take(&mut view.rescan_again);
        self.relayout_desktop();
        if again {
            self.rescan_desktop();
        }
    }

    /// The theme icons the desktop's cells wear at draw scale `scale`, with their size in
    /// device pixels: a scale warm loads them off the loop for the relayout at that scale.
    pub fn desktop_theme_icons(&self, scale: i32) -> Vec<(String, i32)> {
        let mut icons: Vec<(String, i32)> = (self.desktop.iter())
            .flat_map(|v| &v.entries)
            .filter(|e| !e.icon.starts_with('/'))
            .map(|e| (e.icon.clone(), ICON_PX * scale))
            .collect();
        icons.sort_unstable();
        icons.dedup();
        icons
    }
}
