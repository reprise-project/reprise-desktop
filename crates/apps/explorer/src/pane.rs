use crate::Explorer;
use crate::commands::Command;
use crate::place::Place;
use repriseapp::Ctx;
use reprisekit::controls::taskpane::{Row, Section};

#[derive(Clone, Debug, PartialEq)]
pub struct PaneRow {
    pub text: String,
    /// Theme names to try, best first; `chrome_icon` falls back to the
    /// placeholder when the theme has none.
    pub icons: Vec<&'static str>,
    /// [`Command::Nothing`] on a greyed row: `taskpane::click` refuses it.
    pub cmd: Command,
    pub enabled: bool,
    pub tip: Option<&'static str>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PaneSection {
    pub title: String,
    pub rows: Vec<PaneRow>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sel {
    None,
    File,
    Folder,
    Many,
}

const NEW_FOLDER: &[&str] = &["folder-new"];
const PUBLISH: &[&str] = &["web-browser"];
const SHARE: &[&str] = &["folder-remote"];
const RENAME: &[&str] = &["edit-rename", "document-properties"];
const MOVE: &[&str] = &["edit-cut", "go-jump"];
const COPY: &[&str] = &["edit-copy", "folder-copy"];
const MAIL: &[&str] = &["mail-send", "mail-message-new"];
const DELETE: &[&str] = &["edit-delete", "user-trash"];
const SYSINFO: &[&str] = &["computer"];
const PROGRAMS: &[&str] = &["system-software-install", "applications-other"];
const SETTINGS: &[&str] = &["preferences-system"];
const EMPTY_BIN: &[&str] = &["user-trash-full", "user-trash"];
const RESTORE: &[&str] = &["edit-undo"];

fn row(text: &str, icons: &[&'static str], cmd: Command, enabled: bool) -> PaneRow {
    PaneRow {
        text: text.to_string(),
        icons: icons.to_vec(),
        cmd,
        enabled,
        tip: None,
    }
}

fn dead(text: &str, icons: &[&'static str]) -> PaneRow {
    row(text, icons, Command::Nothing, false)
}

pub fn tasks(place: &Place, sel: Sel, count: usize) -> PaneSection {
    let (title, rows) = match place {
        Place::Computer => (
            "System Tasks",
            vec![
                row(
                    "View system information",
                    SYSINFO,
                    Command::SystemInformation,
                    true,
                ),
                dead("Add or remove programs", PROGRAMS),
                dead("Change a setting", SETTINGS),
            ],
        ),
        Place::RecycleBin if count == 0 => ("Recycle Bin Tasks", Vec::new()),
        Place::RecycleBin => (
            "Recycle Bin Tasks",
            vec![
                row(
                    "Empty the Recycle Bin",
                    EMPTY_BIN,
                    Command::EmptyRecycleBin,
                    true,
                ),
                match sel {
                    Sel::None => row("Restore all items", RESTORE, Command::Restore, true),
                    Sel::Many => row(
                        "Restore the selected items",
                        RESTORE,
                        Command::Restore,
                        true,
                    ),
                    _ => row("Restore this item", RESTORE, Command::Restore, true),
                },
            ],
        ),
        _ => ("File and Folder Tasks", folder_tasks(sel)),
    };
    PaneSection {
        title: title.to_string(),
        rows,
    }
}

fn folder_tasks(sel: Sel) -> Vec<PaneRow> {
    match sel {
        Sel::None => vec![
            row("Make a new folder", NEW_FOLDER, Command::NewFolder, true),
            dead("Publish this folder to the Web", PUBLISH),
            dead("Share this folder", SHARE),
        ],
        Sel::File => vec![
            row("Rename this file", RENAME, Command::Rename, true),
            row("Move this file", MOVE, Command::MoveTo, true),
            row("Copy this file", COPY, Command::CopyTo, true),
            dead("Publish this file to the Web", PUBLISH),
            dead("E-mail this file", MAIL),
            row(
                "Delete this file",
                DELETE,
                Command::Delete { erase: false },
                true,
            ),
        ],
        Sel::Folder => vec![
            row("Rename this folder", RENAME, Command::Rename, true),
            row("Move this folder", MOVE, Command::MoveTo, true),
            row("Copy this folder", COPY, Command::CopyTo, true),
            dead("Publish this folder to the Web", PUBLISH),
            dead("Share this folder", SHARE),
            row(
                "Delete this folder",
                DELETE,
                Command::Delete { erase: false },
                true,
            ),
        ],
        Sel::Many => vec![
            row("Move the selected items", MOVE, Command::MoveTo, true),
            row("Copy the selected items", COPY, Command::CopyTo, true),
            dead("Publish the selected items to the Web", PUBLISH),
            dead("E-mail the selected items", MAIL),
            row(
                "Delete the selected items",
                DELETE,
                Command::Delete { erase: false },
                true,
            ),
        ],
    }
}

pub fn other_places(place: &Place, parent: Option<Place>) -> PaneSection {
    let link = |p: &Place| PaneRow {
        tip: p.shell().map(|s| s.tip()),
        ..row(&p.title(), &[p.icon_name()], Command::GoTo(p.clone()), true)
    };
    let rows = match place {
        Place::Computer => vec![link(&Place::Documents), link(&Place::Home)],
        _ => {
            let mut rows = Vec::new();
            let parent = parent.filter(|p| p != place);
            if let Some(p) = &parent {
                rows.push(link(p));
            }
            let listed = |p: &Place| Some(p) == parent.as_ref() || p == place;
            for p in [
                Place::Desktop,
                Place::Documents,
                Place::Home,
                Place::Computer,
            ] {
                if !listed(&p) {
                    rows.push(link(&p));
                }
            }
            rows
        }
    };
    PaneSection {
        title: "Other Places".to_string(),
        rows,
    }
}

pub fn sections(place: &Place, parent: Option<Place>, sel: Sel, count: usize) -> Vec<PaneSection> {
    vec![tasks(place, sel, count), other_places(place, parent)]
}

/// Everything `sections` reads: while it holds, only Details can differ.
pub type PaneKey = (Place, Option<Place>, Sel, usize);

impl Explorer {
    pub fn build_taskpane(&mut self, ctx: &mut Ctx) {
        if let Some(i) = self.sv.selection.single() {
            self.read_dims(i, ctx);
        }
        let sel = self.pane_selection();
        let parent = self.place.parent_in(&self.drive_mounts);
        let key = (self.place.clone(), parent.clone(), sel, self.items.len());
        let mut details = Section::lines("Details", self.details_lines());
        details.open = self.pane_open(&details.title);
        if self.pane_key.as_ref() == Some(&key) {
            if let Some(last) = self.taskpane.sections.last_mut() {
                *last = details;
            }
            return;
        }
        let built = sections(&self.place, parent, sel, self.items.len());
        let mut out = Vec::new();
        let mut cmds = Vec::new();
        let mut tips = Vec::new();
        for s in built {
            let rows = s
                .rows
                .iter()
                .map(|r| Row {
                    icon: crate::chrome_icon(ctx, &r.icons, 16),
                    text: r.text.clone(),
                    enabled: r.enabled,
                })
                .collect();
            tips.push(s.rows.iter().map(|r| r.tip).collect());
            cmds.push(s.rows.into_iter().map(|r| r.cmd).collect());
            let mut section = Section::links(s.title, rows);
            section.open = self.pane_open(&section.title);
            out.push(section);
        }
        cmds.push(Vec::new());
        tips.push(Vec::new());
        out.push(details);
        self.taskpane.sections = out;
        self.pane_cmds = cmds;
        self.pane_tips = tips;
        self.pane_key = Some(key);
    }

    fn pane_open(&self, title: &str) -> bool {
        self.settings.pane_open.get(title).copied().unwrap_or(true)
    }

    fn pane_selection(&self) -> Sel {
        match self.sv.selection.len() {
            0 => Sel::None,
            1 => {
                let dir = self
                    .sv
                    .selection
                    .single()
                    .and_then(|i| self.items.get(i))
                    .is_some_and(|it| it.is_folder());
                if dir { Sel::Folder } else { Sel::File }
            }
            _ => Sel::Many,
        }
    }

    fn details_lines(&self) -> Vec<(String, bool)> {
        let chosen: Vec<&reprisefs::listing::Item> = self
            .sv
            .selection
            .set
            .iter()
            .filter_map(|&i| self.items.get(i))
            .collect();
        let dims = match chosen[..] {
            [it] => self.image_dims.get(&it.path).copied(),
            _ => None,
        };
        crate::window::details_text(
            &self.place.title(),
            self.place.is_system(),
            &chosen,
            dims,
            reprisefs::locale::is_12h(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn table(s: &PaneSection) -> (String, Vec<(String, bool)>) {
        (
            s.title.clone(),
            s.rows.iter().map(|r| (r.text.clone(), r.enabled)).collect(),
        )
    }

    fn texts(s: &PaneSection) -> Vec<String> {
        s.rows.iter().map(|r| r.text.clone()).collect()
    }

    fn folder() -> Place {
        Place::Folder(PathBuf::from("/tmp/work/Reports"))
    }

    #[test]
    fn nothing_selected_offers_three_folder_tasks() {
        let s = tasks(&folder(), Sel::None, 0);
        assert_eq!(
            table(&s),
            (
                "File and Folder Tasks".to_string(),
                vec![
                    ("Make a new folder".to_string(), true),
                    ("Publish this folder to the Web".to_string(), false),
                    ("Share this folder".to_string(), false),
                ]
            )
        );
        assert_eq!(s.rows[0].cmd, Command::NewFolder);
    }

    #[test]
    fn one_file_offers_six_file_tasks() {
        let s = tasks(&folder(), Sel::File, 3);
        assert_eq!(
            table(&s).1,
            vec![
                ("Rename this file".to_string(), true),
                ("Move this file".to_string(), true),
                ("Copy this file".to_string(), true),
                ("Publish this file to the Web".to_string(), false),
                ("E-mail this file".to_string(), false),
                ("Delete this file".to_string(), true),
            ]
        );
        assert_eq!(s.rows[0].cmd, Command::Rename);
        assert_eq!(s.rows[1].cmd, Command::MoveTo);
        assert_eq!(s.rows[2].cmd, Command::CopyTo);
        assert_eq!(s.rows[5].cmd, Command::Delete { erase: false });
        for r in &s.rows[3..5] {
            assert_eq!(r.cmd, Command::Nothing);
        }
    }

    #[test]
    fn one_folder_swaps_e_mail_for_share() {
        let s = tasks(&folder(), Sel::Folder, 3);
        assert_eq!(
            texts(&s),
            [
                "Rename this folder",
                "Move this folder",
                "Copy this folder",
                "Publish this folder to the Web",
                "Share this folder",
                "Delete this folder",
            ]
        );
    }

    #[test]
    fn several_selected_lose_rename_and_gain_the_plural() {
        let s = tasks(&folder(), Sel::Many, 3);
        assert_eq!(
            table(&s).1,
            vec![
                ("Move the selected items".to_string(), true),
                ("Copy the selected items".to_string(), true),
                ("Publish the selected items to the Web".to_string(), false),
                ("E-mail the selected items".to_string(), false),
                ("Delete the selected items".to_string(), true),
            ]
        );
    }

    #[test]
    fn computer_gets_system_tasks_and_two_other_places() {
        let s = sections(&Place::Computer, Some(Place::Desktop), Sel::None, 4);
        assert_eq!(s[0].rows[0].cmd, Command::SystemInformation);
        assert_eq!(
            table(&s[0]),
            (
                "System Tasks".to_string(),
                vec![
                    ("View system information".to_string(), true),
                    ("Add or remove programs".to_string(), false),
                    ("Change a setting".to_string(), false),
                ]
            )
        );
        assert_eq!(
            table(&s[1]),
            (
                "Other Places".to_string(),
                vec![("Documents".to_string(), true), (Place::Home.title(), true),]
            )
        );
        assert_eq!(s[1].rows[0].cmd, Command::GoTo(Place::Documents));
        assert_eq!(s[1].rows[1].cmd, Command::GoTo(Place::Home));
    }

    #[test]
    fn the_recycle_bin_gets_its_own_tasks() {
        let s = tasks(&Place::RecycleBin, Sel::None, 2);
        assert_eq!(
            table(&s),
            (
                "Recycle Bin Tasks".to_string(),
                vec![
                    ("Empty the Recycle Bin".to_string(), true),
                    ("Restore all items".to_string(), true),
                ]
            )
        );
        assert_eq!(s.rows[0].cmd, Command::EmptyRecycleBin);
        assert_eq!(s.rows[1].cmd, Command::Restore);
        assert!(tasks(&Place::RecycleBin, Sel::None, 0).rows.is_empty());
        let one = tasks(&Place::RecycleBin, Sel::File, 2);
        assert_eq!(texts(&one)[1], "Restore this item");
        assert!(one.rows[1].enabled);
        let many = tasks(&Place::RecycleBin, Sel::Many, 2);
        assert_eq!(texts(&many)[1], "Restore the selected items");
        assert_eq!(many.rows[1].cmd, Command::Restore);
        let home = Place::Home.title();
        assert_eq!(
            texts(&other_places(&Place::RecycleBin, Some(Place::Desktop))),
            ["Desktop", "Documents", home.as_str(), "Computer"]
        );
    }

    /// A folder inside another lists its parent first, by the parent's
    /// own name, and never lists the folder it is already showing.
    #[test]
    fn other_places_leads_with_the_parent_and_drops_the_current_place() {
        let home = Place::Home.title();
        let parent = Place::Folder(PathBuf::from("/tmp/work"));
        let s = other_places(&folder(), Some(parent));
        assert_eq!(
            texts(&s),
            ["work", "Desktop", "Documents", home.as_str(), "Computer"]
        );
        assert!(s.rows.iter().all(|r| r.enabled));

        let s = other_places(&Place::Documents, Some(Place::Desktop));
        assert_eq!(texts(&s), ["Desktop", home.as_str(), "Computer"]);
        let s = other_places(
            &Place::Folder(PathBuf::from("/home/u/Reports")),
            Some(Place::Home),
        );
        assert_eq!(
            texts(&s),
            [home.as_str(), "Desktop", "Documents", "Computer"]
        );
        assert_eq!(
            texts(&other_places(&Place::Desktop, None)),
            ["Documents", home.as_str(), "Computer"]
        );
    }

    #[test]
    fn the_built_pane_follows_the_selection_and_keeps_a_collapsed_section() {
        let dir = tempfile::tempdir().expect("a temp dir");
        std::fs::write(dir.path().join("notes.txt"), "x").expect("write");
        let mut ctx = crate::tests::test_ctx();
        let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);

        let titles = |ex: &Explorer| -> Vec<String> {
            ex.taskpane
                .sections
                .iter()
                .map(|s| s.title.clone())
                .collect()
        };
        assert_eq!(
            titles(&ex),
            ["File and Folder Tasks", "Other Places", "Details"]
        );
        assert_eq!(ex.pane_cmds[0][0], Command::NewFolder);
        assert_eq!(
            ex.taskpane.sections[2].body,
            reprisekit::controls::taskpane::Body::Lines(vec![
                (ex.place.title(), true),
                ("File Folder".to_string(), false),
            ])
        );

        ex.settings.pane_open.insert("Other Places".into(), false);
        ex.sv.selection.set_only(Some(0));
        ex.refresh_chrome(&mut ctx);
        assert!(!ex.taskpane.sections[1].open);
        assert!(ex.taskpane.sections[0].open);
        match &ex.taskpane.sections[0].body {
            reprisekit::controls::taskpane::Body::Links(rows) => {
                assert_eq!(rows[0].text, "Rename this file");
                assert_eq!(rows.len(), 6);
                assert!(rows[0].icon.is_some(), "every row wears an icon");
            }
            other => panic!("expected links, got {other:?}"),
        }
        assert_eq!(ex.pane_cmds[0][5], Command::Delete { erase: false });
        assert_eq!(ex.pane_cmds.len(), ex.taskpane.sections.len());
        assert!(ex.pane_cmds[2].is_empty());
    }

    #[test]
    fn a_selection_of_the_same_kind_only_rewrites_details() {
        let dir = tempfile::tempdir().expect("a temp dir");
        std::fs::write(dir.path().join("a.txt"), "x").expect("write");
        std::fs::write(dir.path().join("b.txt"), "xy").expect("write");
        let mut ctx = crate::tests::test_ctx();
        let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
        ex.sv.selection.set_only(Some(0));
        ex.refresh_chrome(&mut ctx);
        let details = |ex: &Explorer| match &ex.taskpane.sections[2].body {
            reprisekit::controls::taskpane::Body::Lines(lines) => lines.clone(),
            other => panic!("expected lines, got {other:?}"),
        };
        let first = details(&ex);
        ex.taskpane.sections[0].open = false;
        ex.sv.selection.set_only(Some(1));
        ex.refresh_chrome(&mut ctx);
        assert!(!ex.taskpane.sections[0].open, "the sections were kept");
        assert_ne!(details(&ex), first, "Details follows the file");
        ex.sv.selection.clear();
        ex.refresh_chrome(&mut ctx);
        assert!(ex.taskpane.sections[0].open, "a change of kind rebuilds");
    }
}
