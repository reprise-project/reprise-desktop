//! The desktop's file work that asks or answers through a dialog: Delete and Shift+Delete with their confirmations, the Recycle Bin's Empty and Properties, an icon's Properties and Open With. The work runs off the loop (`fsjobs`); the dialogs come back on it.

use std::path::{Path, PathBuf};

use reprisefs::{ops, places};

use crate::compositor::State;
use crate::desktop::{self, DesktopEntry};
use crate::dialog::Dialog;
use crate::fsjobs::OnLoop;

/// What Delete on the set `set` of `entries` covers: the `system_icons` names of its place icons and the paths of its files.
pub fn set_for_delete(entries: &[DesktopEntry], set: &[usize]) -> (Vec<String>, Vec<PathBuf>) {
    let (mut places, mut files) = (Vec::new(), Vec::new());
    for e in set.iter().filter_map(|&i| entries.get(i)) {
        match e.kind {
            desktop::Kind::Place(p) => places.push(p.config_name().to_string()),
            desktop::Kind::File => files.push(e.path.clone()),
            desktop::Kind::RecycleBin => {}
        }
    }
    (places, files)
}

fn lone_file(entries: &[DesktopEntry], files: &[PathBuf]) -> Option<usize> {
    let [path] = files else {
        return None;
    };
    entries
        .iter()
        .position(|e| e.kind == desktop::Kind::File && &e.path == path)
}

fn delete_each(
    items: impl Iterator<Item = (PathBuf, reprisefs::bincap::Plan)>,
) -> Option<(PathBuf, std::io::Error)> {
    let mut items = items;
    items.find_map(|(p, plan)| ops::delete(&p, plan).err().map(|e| (p, e)))
}

impl State {
    /// The Open With chooser for the file `path`: programs that take its MIME type (by its name,
    /// with no stat), its default under "Recommended Programs:", the rest under "Other
    /// Programs:". `set_default` is the sheet's Change..., which only writes the file type's
    /// default.
    pub fn choose_program(&mut self, path: &Path, set_default: bool) -> Dialog {
        let apps: &[reprisefs::apps::AppEntry] = &self.app_index;
        let mime = (path.file_name())
            .and_then(|n| reprisefs::apps::mime_globs().mime_of_name(&n.to_string_lossy()));
        let default = mime
            .as_deref()
            .and_then(|m| reprisefs::props::default_app(m, apps));
        let (recommended, others) = reprisefs::apps::chooser_groups(apps, mime.as_deref(), default);
        let program = |i: usize| reprisekit::controls::openwith::Program {
            id: apps[i].id.clone(),
            name: apps[i].name.clone(),
            icon: apps[i].icon.clone(),
        };
        Dialog::open_with(
            path,
            mime,
            recommended.into_iter().map(program).collect(),
            others.into_iter().map(program).collect(),
            default.map(|i| apps[i].id.as_str()),
            set_default,
        )
    }

    /// The file behind a desktop icon moved from `from` to `to` (a Properties OK): its dragged cell follows the new name, then the desktop rescans.
    pub fn desktop_renamed(&mut self, from: &Path, to: &Path) {
        if let Some(view) = self.desktop.as_mut() {
            let old = view
                .entries
                .iter()
                .find(|e| e.path == from)
                .map(|e| e.name.clone());
            let new = to
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if let Some(cell) = old.and_then(|o| view.positions.remove(&o)) {
                view.positions.insert(new, cell);
                crate::state::save_desktop(view.auto_arrange, &view.positions);
            }
        }
        self.rescan_desktop();
    }

    /// Take these `system_icons` names off the desktop and out of `config.toml`, which Reprise Settings shows so it can bring them back.
    pub fn hide_system_icons(&mut self, names: &[String]) {
        let icons = &self.config.system_icons;
        let shown: Vec<String> = icons
            .iter()
            .filter(|n| !names.contains(n))
            .cloned()
            .collect();
        if shown.len() == icons.len() {
            return;
        }
        self.config.system_icons = shown.clone();
        self.rescan_desktop();
        crate::configwrite::save_system_icons(&self.config_path, shown);
    }

    /// Delete icon `index`: a file asks Confirm File Delete (per [`reprisefs::bincap::delete_mode`]) before [`State::delete_paths`], a place asks to hide itself, the Recycle Bin is ignored.
    pub fn trash_icon(&mut self, index: usize) {
        let Some(entry) = self.desktop.as_ref().and_then(|v| v.entries.get(index)) else {
            return;
        };
        let dialog = match entry.kind {
            desktop::Kind::RecycleBin => return,
            desktop::Kind::Place(place) => Dialog::confirm_hide(place, &entry.label),
            desktop::Kind::File => {
                let (path, label) = (entry.path.clone(), entry.label.clone());
                return self.ask_delete(vec![path], Some(label), false);
            }
        };
        self.open_dialog(dialog);
    }

    /// Delete `paths`, asking first as `delete_mode` says (Delete, or Shift+Delete with `shift`). Each item's fate (`Bins::fate`) is read off the loop first: it stats the paths and their mounts, which a hung network mount would stall.
    pub(crate) fn ask_delete(&mut self, paths: Vec<PathBuf>, label: Option<String>, shift: bool) {
        self.spawn_fs_job(move || -> OnLoop {
            use reprisefs::bincap::{Fate, Policy, delete_mode};
            let bins = reprisefs::trash::Bins::system();
            let fates: Vec<Fate> = paths
                .iter()
                .map(|p| bins.as_ref().map_or(Fate::NoBin, |b| b.fate(p)))
                .collect();
            let confirm = bins
                .as_ref()
                .map_or(Policy::default().confirm, |b| b.policy.confirm);
            let mode = delete_mode(confirm, &fates, shift);
            // A symlink to a folder deletes as a file: only the link goes.
            let is_dir = matches!(paths.as_slice(), [one] if one.is_dir() && !one.is_symlink());
            Box::new(move |state| {
                let erase = mode.erase;
                let dialog = match paths.as_slice() {
                    [] => return,
                    _ if !mode.ask => return state.delete_paths(paths, erase),
                    [one] => {
                        let name = label.unwrap_or_else(|| {
                            one.file_name().unwrap_or_default().to_string_lossy().into()
                        });
                        Dialog::confirm_delete(&name, one, is_dir, erase, mode.no_bin)
                    }
                    _ => Dialog::confirm_delete_many(&paths, erase, mode.no_bin),
                };
                state.open_dialog(dialog);
            })
        });
    }

    /// Delete `paths` off the loop: to the trash, or for good with `erase`. Trashing first asks one at a time about items too big for their Recycle Bin ([`State::too_big_step`]). The first failure stops the rest and an error box says why.
    pub fn delete_paths(&mut self, paths: Vec<PathBuf>, erase: bool) {
        use reprisefs::bincap::{Plan, TooBig, TooBigStep};
        self.spawn_fs_job(move || -> OnLoop {
            let plan = if erase {
                paths.into_iter().map(|p| (p, Plan::Erase)).collect()
            } else {
                let bins = reprisefs::trash::Bins::system();
                let measured = paths
                    .iter()
                    .map(|p| bins.as_ref().and_then(|b| b.measure(p)))
                    .collect();
                let asker = TooBig::new(paths, measured);
                match asker.step() {
                    TooBigStep::Done(plan) => plan,
                    _ => return Box::new(move |state| state.too_big_step(asker)),
                }
            };
            let failed = delete_each(plan.into_iter());
            Box::new(move |state| state.delete_failed(failed))
        });
    }

    /// The next step of a Delete holding items too big for their Recycle Bin: the next question, the batch deleted as answered, or nothing after Cancel.
    pub fn too_big_step(&mut self, asker: reprisefs::bincap::TooBig) {
        use reprisefs::bincap::TooBigStep;
        match asker.step() {
            TooBigStep::Ask(path) => {
                let is_dir = path.is_dir() && !path.is_symlink();
                let d = Dialog::too_big(path, is_dir, asker.clone());
                self.open_dialog(d);
            }
            TooBigStep::Done(plan) => {
                self.spawn_fs_job(move || -> OnLoop {
                    let failed = delete_each(plan.into_iter());
                    Box::new(move |state| state.delete_failed(failed))
                });
            }
            TooBigStep::Cancelled => {}
        }
    }

    fn delete_failed(&mut self, failed: Option<(PathBuf, std::io::Error)>) {
        let Some((path, e)) = failed else {
            return;
        };
        log::warn!("desktop: could not delete {}: {e}", path.display());
        let name = path
            .file_name()
            .unwrap_or(path.as_os_str())
            .to_string_lossy();
        let text = format!("Cannot delete {name}: {e}");
        self.open_dialog(Dialog::error("Error Deleting File or Folder", text));
    }

    /// Empty the Recycle Bin, the home trash and each drive's, off the loop. The watch flips its icon; a failure says why in an error box.
    pub fn empty_bin(&mut self) {
        self.spawn_fs_job(move || -> OnLoop {
            let emptied = reprisefs::trash::Bins::system()
                .ok_or_else(|| std::io::Error::other("cannot find the Recycle Bin"))
                .and_then(|bins| bins.empty());
            Box::new(move |state| match emptied {
                Ok(n) => log::info!("desktop: emptied the Recycle Bin ({n} items)"),
                Err(e) => {
                    log::warn!("desktop: could not empty the Recycle Bin: {e}");
                    let text = format!("Cannot empty the Recycle Bin: {e}");
                    state.open_dialog(Dialog::error("Error Deleting File or Folder", text));
                }
            })
        });
    }

    /// Delete aimed at icon `index`: alone it goes through `trash_icon`. In a multi-selection holding place icons, "Confirm Delete" asks first whether to take them off the desktop, then the files. The Recycle Bin in a set stays put.
    pub fn trash_selection(&mut self, index: usize) {
        let set = self.selection_around(index);
        let Some(view) = self.desktop.as_ref() else {
            return;
        };
        if set.len() == 1 {
            return self.trash_icon(index);
        }
        let (places, files) = set_for_delete(&view.entries, &set);
        if !places.is_empty() {
            return self.open_dialog(Dialog::confirm_delete_icons(places, files));
        }
        self.trash_files(files);
    }

    pub fn trash_files(&mut self, files: Vec<PathBuf>) {
        let one = self
            .desktop
            .as_ref()
            .and_then(|v| lone_file(&v.entries, &files));
        match one {
            Some(i) => self.trash_icon(i),
            None => self.ask_delete(files, None, false),
        }
    }

    pub fn trash_paths(&mut self, paths: Vec<PathBuf>) {
        self.ask_delete(paths, None, false);
    }

    /// Shift+Delete aimed at icon `index`: its selection's files go for good, once asked; place icons and the Recycle Bin in the set stay.
    pub fn erase_selection(&mut self, index: usize) {
        let set = self.selection_around(index);
        let Some(view) = self.desktop.as_ref() else {
            return;
        };
        let (_, files) = set_for_delete(&view.entries, &set);
        let label = lone_file(&view.entries, &files).map(|i| view.entries[i].label.clone());
        self.ask_delete(files, label, true);
    }

    /// Properties aimed at icon `index`: the combined sheet over its selection's files, `index`
    /// first, when two or more read; else the icon's own sheet. The files are read in a file job,
    /// as one may sit on a mount that never answers.
    pub fn open_icon_properties(&mut self, index: usize) {
        let set = self.selection_around(index);
        let Some(view) = self.desktop.as_ref() else {
            return;
        };
        let (Some(e), Some(cell)) = (view.entries.get(index), view.cells.get(index)) else {
            return;
        };
        let (own, icon, at) = (
            (e.path.clone(), e.label.clone()),
            e.icon.clone(),
            (cell.x, cell.y),
        );
        let files: Vec<(PathBuf, String)> = if set.len() > 1 {
            crate::desktopview::aimed_in(&view.entries, &set, index, true)
                .into_iter()
                .map(|i| &view.entries[i])
                .filter(|e| e.kind == desktop::Kind::File)
                .map(|e| (e.path.clone(), e.label.clone()))
                .collect()
        } else {
            Vec::new()
        };
        let apps = self.app_index.clone();
        self.spawn_fs_job(move || {
            let read = glance_aimed(&own, &files, &apps);
            Box::new(move |state: &mut State| match read {
                Some(Glanced::One(info)) => state.open_properties(&info, &icon, at),
                Some(Glanced::Set(items)) => {
                    let info = reprisefs::props::glance_set(&items);
                    state.open_set_properties(&info, &items[0].name, at);
                }
                None => {}
            })
        });
    }

    pub fn open_icon_chooser(&mut self, index: usize) {
        let Some(path) = self
            .desktop
            .as_ref()
            .and_then(|v| v.entries.get(index))
            .map(|e| e.path.clone())
        else {
            return;
        };
        let d = self.choose_program(&path, false);
        self.open_dialog(d);
    }

    pub fn open_bin_properties(&mut self, index: usize) {
        let Some((at, icon)) = self.desktop.as_ref().and_then(|v| {
            let cell = v.cells.get(index)?;
            Some(((cell.x, cell.y), v.entries.get(index)?.icon.clone()))
        }) else {
            return;
        };
        // Each drive's size is a statvfs, which a mount that never answers holds.
        self.spawn_fs_job(move || {
            let read = reprisefs::trash::Bins::system().map(|bins| {
                let drives = crate::binprops::drives(&bins, &places::mounted_drives());
                (bins.policy, drives)
            });
            Box::new(move |state: &mut State| {
                if let Some((policy, drives)) = read {
                    let sheet = crate::binprops::BinSheet::new(policy, drives);
                    state.open_dialog(crate::binprops::dialog(sheet, at, &icon));
                }
            })
        });
    }

    /// Empty Recycle Bin: asks to delete what the bins hold, counted in a file job.
    pub fn ask_empty_bin(&mut self) {
        self.spawn_fs_job(|| {
            let (count, first) = reprisefs::trash::Bins::system()
                .map(|bins| bins.files_dirs())
                .unwrap_or_default()
                .iter()
                .map(|files| places::trash_contents(files))
                .fold((0, None::<String>), |(n, first), (m, other)| {
                    let first = match (first, other) {
                        (Some(a), Some(b)) => Some(a.min(b)),
                        (a, b) => a.or(b),
                    };
                    (n + m, first)
                });
            Box::new(move |state: &mut State| {
                if count > 0 {
                    state.open_dialog(Dialog::confirm_empty_bin(count, first.as_deref()));
                }
            })
        });
    }
}

/// What Properties on a desktop icon reads: the set's files when two or more read, else the
/// icon's own.
#[derive(Debug)]
enum Glanced {
    One(Box<reprisefs::props::FileInfo>),
    Set(Vec<reprisefs::props::FileInfo>),
}

/// Reads the files of `set` (`(path, label)`, the aimed icon first), falling back to the aimed
/// icon `own` alone when fewer than two read.
fn glance_aimed(
    own: &(PathBuf, String),
    set: &[(PathBuf, String)],
    apps: &[reprisefs::apps::AppEntry],
) -> Option<Glanced> {
    let glance = |(path, label): &(PathBuf, String)| {
        reprisefs::props::glance(path, label, apps)
            .map_err(|err| log::warn!("properties: {}: {err}", path.display()))
            .ok()
    };
    let items: Vec<reprisefs::props::FileInfo> = set.iter().filter_map(glance).collect();
    if items.len() >= 2 {
        Some(Glanced::Set(items))
    } else {
        glance(own).map(|info| Glanced::One(Box::new(info)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::Target;

    fn file(name: &str) -> DesktopEntry {
        let path = PathBuf::from("/d").join(name);
        DesktopEntry {
            name: name.into(),
            label: name.into(),
            icon: "text-x-generic".into(),
            target: Target::Open(path.clone()),
            path,
            shortcut: false,
            kind: desktop::Kind::File,
            is_dir: false,
            allow_launching: false,
        }
    }

    #[test]
    fn delete_on_a_set_splits_its_places_from_its_files() {
        let docs = desktop::place("documents").unwrap();
        let pc = desktop::place("computer").unwrap();
        let bin = desktop::recycle_bin(PathBuf::from("/t/files"), true);
        let entries = vec![docs, file("a.txt"), bin, pc, file("b.txt")];
        assert_eq!(
            set_for_delete(&entries, &[0, 1, 2, 3, 4, 9]),
            (
                vec!["documents".to_string(), "computer".to_string()],
                vec![PathBuf::from("/d/a.txt"), PathBuf::from("/d/b.txt")]
            ),
            "the bin and an index past the entries are left out"
        );
        assert_eq!(
            set_for_delete(&entries, &[1, 4]),
            (
                Vec::new(),
                vec![PathBuf::from("/d/a.txt"), PathBuf::from("/d/b.txt")]
            )
        );
    }

    #[test]
    fn a_lone_file_after_the_places_is_its_icon() {
        let docs = desktop::place("documents").unwrap();
        let mut firefox = file("firefox.desktop");
        firefox.label = "Firefox".into();
        let entries = vec![docs, file("a.txt"), firefox];
        let path = |n: &str| PathBuf::from("/d").join(n);
        assert_eq!(lone_file(&entries, &[path("firefox.desktop")]), Some(2));
        assert_eq!(
            lone_file(&entries, &[path("a.txt"), path("firefox.desktop")]),
            None
        );
        assert_eq!(lone_file(&entries, &[path("gone.txt")]), None);
    }

    /// Properties on a selection reads its files off the loop: two that read give the combined
    /// sheet; a set left with one (a file gone meanwhile) gives the aimed icon's own sheet.
    #[test]
    fn properties_on_a_set_falls_back_to_the_aimed_icon() {
        let dir = std::env::temp_dir().join(format!("reprise-glance-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let named = |n: &str| (dir.join(n), n.to_string());
        std::fs::write(dir.join("a.txt"), "a").unwrap();
        std::fs::write(dir.join("b.txt"), "bb").unwrap();
        let (a, b, gone) = (named("a.txt"), named("b.txt"), named("gone.txt"));
        let names = |g: Option<Glanced>| match g {
            Some(Glanced::Set(items)) => items.into_iter().map(|i| i.name).collect(),
            Some(Glanced::One(info)) => vec![format!("one {}", info.name)],
            None => Vec::new(),
        };
        assert_eq!(
            names(glance_aimed(&a, &[a.clone(), gone.clone(), b.clone()], &[])),
            ["a.txt", "b.txt"]
        );
        assert_eq!(
            names(glance_aimed(&a, &[a.clone(), gone.clone()], &[])),
            ["one a.txt"]
        );
        assert!(glance_aimed(&gone, &[], &[]).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
