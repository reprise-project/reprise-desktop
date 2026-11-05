//! The page list's link rows and the page buttons that start a program: each starts it from its
//! desktop entry, in its app scope as Start does; a program without a shown entry greys its row
//! or button.

use crate::SettingsApp;
use crate::pages::PROGRAM_BUTTONS;
use repriseapp::Reply;
use repriseconf::pages::links;
use reprisefs::apps::{AppEntry, launch_argv, parse_desktop, read_desktop_file};
use std::path::PathBuf;

/// The entry `id` names: the first of `dirs` holding the file decides; `None` when that one is
/// not shown (`NoDisplay`, `Hidden`, `OnlyShowIn`, `NotShowIn`) or does not parse.
pub(crate) fn find(id: &str, dirs: &[PathBuf]) -> Option<AppEntry> {
    let path = dirs.iter().map(|d| d.join(id)).find(|p| p.is_file())?;
    parse_desktop(&read_desktop_file(&path).ok()?, id)
}

/// Start `entry` in its app scope.
pub(crate) fn launch(entry: &AppEntry, terminal: &str) -> Reply {
    Reply {
        spawn: vec![launch_argv(entry, terminal)],
        spawn_app: Some(entry.id.clone()),
        ..Reply::redraw()
    }
}

/// Where Settings looks for desktop entries; none under test.
pub(crate) fn dirs() -> Vec<PathBuf> {
    if cfg!(test) {
        Vec::new()
    } else {
        reprisefs::apps::default_dirs()
    }
}

impl SettingsApp {
    /// Each link row's entry, `None` for a program without a shown one.
    pub(crate) fn find_links(&mut self) {
        self.links = links().map(|p| find(p.id, &self.app_dirs)).collect();
    }

    /// Grey each link row and page button whose program has no shown entry.
    pub(crate) fn show_links(&mut self) {
        for (button, program) in PROGRAM_BUTTONS {
            let shown = find(program, &self.app_dirs).is_some();
            for page in &mut self.sheet.pages {
                page.set_enabled(button, shown);
            }
        }
        let Some(list) = self.sheet.list_mut() else {
            return;
        };
        for (i, e) in self.links.iter().enumerate() {
            list.set_link_enabled(i, e.is_some());
        }
    }

    /// Start link row `i`'s program; nothing for a greyed row.
    pub(crate) fn open_link(&mut self, i: usize) -> Reply {
        match self.links.get(i) {
            Some(Some(e)) => launch(e, &self.loaded.terminal),
            _ => Reply::none(),
        }
    }

    /// Start the program whose desktop entry id is `desktop_id`; nothing when it has no shown
    /// entry.
    pub(crate) fn open_program(&mut self, desktop_id: &str) -> Reply {
        match find(desktop_id, &self.app_dirs) {
            Some(e) => launch(&e, &self.loaded.terminal),
            None => Reply::none(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    const SHOWN: &str = "[Desktop Entry]\nType=Application\nName=N\nExec=";

    fn dir(tag: &str, files: &[(&str, String)]) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "reprise-settings-links-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        for (name, text) in files {
            std::fs::write(d.join(name), text).unwrap();
        }
        d
    }

    #[test]
    fn a_link_finds_its_shown_entry_only() {
        let d = dir(
            "first",
            &[
                (
                    "reprise-network.desktop",
                    format!("{SHOWN}reprise-network\n"),
                ),
                (
                    "reprise-printers.desktop",
                    format!("{SHOWN}reprise-printers\nNoDisplay=true\n"),
                ),
            ],
        );
        let d2 = dir(
            "second",
            &[(
                "reprise-printers.desktop",
                format!("{SHOWN}reprise-printers\n"),
            )],
        );
        assert_eq!(
            find("reprise-network.desktop", std::slice::from_ref(&d)).map(|e| e.exec_line),
            Some("reprise-network".into())
        );
        assert_eq!(
            find("reprise-printers.desktop", &[d.clone(), d2.clone()]),
            None,
            "the first dir's hidden copy wins"
        );
        assert!(find("reprise-printers.desktop", &[d2]).is_some());
        assert_eq!(find("reprise-updates.desktop", &[d]), None);
    }

    #[test]
    fn every_link_rows_program_ships_a_shown_entry() {
        let apps = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../data/apps");
        for p in links() {
            assert!(
                find(p.id, std::slice::from_ref(&apps)).is_some(),
                "{} is missing or hidden",
                p.id
            );
        }
    }
}
