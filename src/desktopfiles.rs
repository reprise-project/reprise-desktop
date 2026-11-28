//! The desktop's Delete, Shift+Delete and Empty Recycle Bin: a place icon asks here whether to leave the desktop, and files go to Explorer, which asks and deletes them (`filejobs`). An icon's Properties and Open With and Recycle Bin Properties are Explorer's too, in a process of their own ([`State::show_properties`]).

use std::path::{Path, PathBuf};

use crate::compositor::State;
use crate::desktop::{self, DesktopEntry};
use crate::dialog::Dialog;

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

impl State {
    /// Explorer's Properties sheet for `paths` alone, combined for two or more.
    pub fn show_properties(&mut self, paths: &[PathBuf]) {
        self.explorer_box("--properties", paths);
    }

    /// Explorer's Open With chooser for the file `path` alone.
    pub fn choose_program(&mut self, path: &Path) {
        self.explorer_box("--open-with", &[path.to_path_buf()]);
    }

    /// An Explorer window on the archive `path` as a compressed folder.
    pub fn browse_archive(&mut self, path: &Path) {
        self.explorer_box("--archive", &[path.to_path_buf()]);
    }

    fn explorer_box(&mut self, flag: &str, paths: &[PathBuf]) {
        let argv: Vec<&std::ffi::OsStr> = [EXPLORER.as_ref(), flag.as_ref()]
            .into_iter()
            .chain(paths.iter().map(|p| p.as_os_str()))
            .collect();
        self.spawn_app_activated(&argv, &EXPLORER_APP, None);
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

    /// Delete icon `index`: a file goes to Explorer's Delete, a place asks to hide itself, the Recycle Bin is ignored.
    pub fn trash_icon(&mut self, index: usize) {
        let Some(entry) = self.desktop.as_ref().and_then(|v| v.entries.get(index)) else {
            return;
        };
        let dialog = match entry.kind {
            desktop::Kind::RecycleBin => return,
            desktop::Kind::Place(place) => Dialog::confirm_hide(place, &entry.label),
            desktop::Kind::File => {
                let path = entry.path.clone();
                return self.delete_files(vec![path], false);
            }
        };
        self.open_dialog(dialog);
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
        self.delete_files(files, false);
    }

    /// Shift+Delete aimed at icon `index`: its selection's files go for good, once asked; place icons and the Recycle Bin in the set stay.
    pub fn erase_selection(&mut self, index: usize) {
        let set = self.selection_around(index);
        let Some(view) = self.desktop.as_ref() else {
            return;
        };
        let (_, files) = set_for_delete(&view.entries, &set);
        self.delete_files(files, true);
    }

    /// Properties aimed at icon `index`: the combined sheet over its selection's files, `index`
    /// first, when it holds two or more; else the icon's own sheet.
    pub fn open_icon_properties(&mut self, index: usize) {
        let set = self.selection_around(index);
        let Some(view) = self.desktop.as_ref() else {
            return;
        };
        let paths = properties_paths(&view.entries, &set, index);
        if !paths.is_empty() {
            self.show_properties(&paths);
        }
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
        self.choose_program(&path);
    }

    /// Explorer's Recycle Bin Properties.
    pub fn open_bin_properties(&mut self) {
        self.explorer_box("--bin-properties", &[]);
    }
}

/// The program whose process shows an icon's Properties and Open With.
const EXPLORER: &str = "reprise-explorer";
const EXPLORER_APP: reprisefs::apps::AppId = reprisefs::apps::AppId::from_static(EXPLORER);

/// The paths Properties on icon `index` of `entries` shows, `set` being its selection: the set's
/// files, `index` first, when there are two or more; else the icon's own.
fn properties_paths(entries: &[DesktopEntry], set: &[usize], index: usize) -> Vec<PathBuf> {
    let files: Vec<PathBuf> = crate::desktopview::aimed_in(entries, set, index, true)
        .into_iter()
        .map(|i| &entries[i])
        .filter(|e| e.kind == desktop::Kind::File)
        .map(|e| e.path.clone())
        .collect();
    match files.len() {
        0 | 1 => entries
            .get(index)
            .map(|e| e.path.clone())
            .into_iter()
            .collect(),
        _ => files,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::Target;
    use crate::harness::Harness;

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
            picture: None,
            zip: None,
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
    fn properties_takes_the_selections_files_aimed_first_else_the_icon_alone() {
        let docs = desktop::place("documents").unwrap();
        let entries = vec![docs, file("a.txt"), file("b.txt")];
        let path = |n: &str| PathBuf::from("/d").join(n);
        assert_eq!(
            properties_paths(&entries, &[0, 1, 2], 2),
            [path("b.txt"), path("a.txt")]
        );
        assert_eq!(
            properties_paths(&entries, &[0, 1], 1),
            [path("a.txt")],
            "a place is no file to share the sheet"
        );
        assert_eq!(
            properties_paths(&entries, &[0, 1, 2], 0),
            [path("a.txt"), path("b.txt")]
        );
        assert_eq!(
            properties_paths(&entries, &[], 0),
            [entries[0].path.clone()]
        );
    }

    #[test]
    fn properties_on_an_icon_starts_explorers_sheet_and_opens_no_dialog() {
        crate::spawn::record();
        let mut h = Harness::new();
        h.output("TEST-1", 1024, 768);
        h.state.show_properties(&[PathBuf::from("/d/a.txt")]);
        h.state.choose_program(Path::new("/d/a.txt"));
        assert!(h.state.dialogs.views.is_empty());
        assert_eq!(crate::spawn::recorded(), [EXPLORER, EXPLORER]);
    }

    /// The command lines of the file jobs started, after the program's name.
    fn jobs(h: &Harness) -> Vec<Vec<String>> {
        (h.state.filejobs.jobs.iter())
            .map(|j| {
                (j.args.iter())
                    .map(|a| a.to_string_lossy().into_owned())
                    .collect()
            })
            .collect()
    }

    #[test]
    fn delete_and_shift_delete_on_icons_hand_their_files_to_explorer() {
        crate::spawn::record();
        let mut h = Harness::new();
        h.output("TEST-1", 1024, 768);
        h.state
            .trash_files(vec![PathBuf::from("/d/a.txt"), PathBuf::from("/d/b.txt")]);
        h.state.delete_files(vec![PathBuf::from("/d/c.txt")], true);
        h.state.delete_files(Vec::new(), false);
        assert_eq!(
            jobs(&h),
            [
                vec!["--delete", "/d/a.txt", "/d/b.txt"],
                vec!["--delete", "--erase", "/d/c.txt"],
            ]
        );
        assert_eq!(crate::spawn::recorded(), [EXPLORER, EXPLORER]);
        assert!(
            h.state.dialogs.views.is_empty(),
            "Explorer asks, not the desktop"
        );
    }

    #[test]
    fn empty_recycle_bin_starts_explorers_and_a_second_starts_none_while_it_runs() {
        crate::spawn::record();
        let mut h = Harness::new();
        h.output("TEST-1", 1024, 768);
        h.state.empty_bin();
        assert_eq!(jobs(&h), [vec!["--empty-bin"]]);
        let mut sleeper = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("sleep");
        crate::spawn::adopt(sleeper.id() as libc::pid_t, "sleep");
        h.state.filejobs.jobs[0].pid = sleeper.id();
        h.state.empty_bin();
        assert_eq!(jobs(&h).len(), 1, "the running one stays alone");
        let _ = sleeper.kill();
        let _ = sleeper.wait();
        crate::spawn::reap_children();
        h.state.empty_bin();
        assert_eq!(jobs(&h).len(), 2, "once it ends, Empty starts again");
    }
}
