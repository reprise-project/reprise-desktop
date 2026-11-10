//! The Privacy page's Clipboard Access list: the installed Flatpak programs, ticked when
//! `[clipboard] trusted_apps` names them, and the ids it names that aren't installed.

use crate::SettingsApp;
use crate::registry::{self, Page};
use repriseapp::Reply;
use reprisekit::controls::form::{Control, Form};
use reprisekit::controls::listbox::ListBox;
use reprisekit::controls::openwith;
use reprisekit::raster::Canvas;
use std::path::PathBuf;

pub const LIST: &str = "clipboard_apps";
/// The list's rows at [`Area::MIN`](crate::pages::Area::MIN).
pub const ROWS: usize = 3;

/// An installed Flatpak app that may open Wayland connections, with the name and icon of its
/// desktop entry when it exports one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installed {
    pub id: String,
    pub entry: Option<(String, Option<String>)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub id: String,
    pub name: String,
    pub icon: Option<String>,
    pub installed: bool,
}

/// The apps in `installs` and the entries in `app_dirs` and the installations' exports that name
/// them.
pub fn read_installed(installs: &[PathBuf], app_dirs: &[PathBuf]) -> Vec<Installed> {
    let ids = reprisefs::flatpak::wayland_apps(installs);
    let entries = if ids.is_empty() {
        Vec::new()
    } else {
        let exports = installs
            .iter()
            .map(|i| i.join("exports/share/applications"));
        let dirs: Vec<PathBuf> = app_dirs.iter().cloned().chain(exports).collect();
        reprisefs::apps::scan(&dirs)
    };
    ids.into_iter()
        .map(|id| {
            let entry = reprisefs::apps::entry_for_app_id(&entries, &id, |_| true)
                .map(|e| (e.name.clone(), e.icon.clone()));
            Installed { id, entry }
        })
        .collect()
}

/// The rows: each installed app with a desktop entry, by name; then each id in `listed` that isn't
/// among them, by id.
pub fn rows(installed: &[Installed], listed: &[String]) -> Vec<Row> {
    let mut out: Vec<Row> = installed
        .iter()
        .filter_map(|a| {
            let (name, icon) = a.entry.clone()?;
            Some(Row {
                id: a.id.clone(),
                name,
                icon,
                installed: true,
            })
        })
        .collect();
    out.sort_by_key(|r| r.name.to_lowercase());
    for id in listed {
        if id.is_empty() || out.iter().any(|r| r.id == *id) {
            continue;
        }
        out.push(Row {
            id: id.clone(),
            name: id.clone(),
            icon: None,
            installed: installed.iter().any(|a| a.id == *id),
        });
    }
    out
}

fn list_mut(form: &mut Form) -> Option<(&mut ListBox, &mut Vec<Option<Canvas>>)> {
    match form.item_mut(LIST).map(|i| &mut i.control) {
        Some(Control::ListBox { list, icons }) => Some((list, icons)),
        _ => None,
    }
}

/// Show `rows`, ticked as `trusted` lists them, or a greyed Loading... while they are read (None).
/// True when the rows' names changed, which drops their icons.
pub fn fill(form: &mut Form, rows: Option<&[Row]>, trusted: &[String]) -> bool {
    let Some((list, icons)) = list_mut(form) else {
        return false;
    };
    let items: Vec<String> = match rows {
        Some(rows) => rows.iter().map(|r| r.name.clone()).collect(),
        None => vec!["Loading...".into()],
    };
    let renamed = list.items != items;
    if renamed {
        let picked = list.selected.filter(|&i| i < items.len());
        *list = ListBox::new(items, picked);
        *icons = Vec::new();
    }
    let rows = rows.unwrap_or_default();
    list.checks = rows.iter().map(|r| trusted.contains(&r.id)).collect();
    list.detail = rows
        .iter()
        .map(|r| match (r.installed, r.name == r.id) {
            (false, _) => "not installed".into(),
            (true, false) => r.id.clone(),
            (true, true) => String::new(),
        })
        .collect();
    list.greyed = if rows.is_empty() && !list.items.is_empty() {
        vec![true]
    } else {
        Vec::new()
    };
    renamed
}

/// The ticks into `trusted`: a ticked row's id is added, an unticked one's removed. True when
/// `trusted` changed.
pub fn read(form: &Form, rows: &[Row], trusted: &mut Vec<String>) -> bool {
    let Some(Control::ListBox { list, .. }) = form.item(LIST).map(|i| &i.control) else {
        return false;
    };
    let mut changed = false;
    for (row, &on) in rows.iter().zip(&list.checks) {
        let listed = trusted.contains(&row.id);
        if on && !listed {
            trusted.push(row.id.clone());
            changed = true;
        } else if !on && listed {
            trusted.retain(|t| *t != row.id);
            changed = true;
        }
    }
    changed
}

impl SettingsApp {
    /// Read the installed apps off the loop the first time the Privacy page shows, and keep the
    /// list in step with the pending settings. True when the list changed.
    pub(crate) fn ensure_clip_apps(&mut self) -> bool {
        if registry::open(&self.sheet) != Page::Privacy {
            return false;
        }
        if self.clip_installed.is_none() && self.clip_reading.is_none() {
            let installs = self.flatpak_installs.clone();
            let dirs = self.app_dirs.clone();
            self.clip_reading = Some(self.spawn(move || read_installed(&installs, &dirs)));
        }
        // An id unticked but not applied keeps its row, so it can be ticked again.
        let listed = [&self.loaded, &self.want]
            .map(|s| s.clipboard_trusted.clone())
            .concat();
        let rows = (self.clip_installed.as_deref())
            .map(|i| rows(i, &listed))
            .unwrap_or_default();
        let page = &mut self.sheet.pages[Page::Privacy.index()];
        let shown = self.clip_installed.is_some().then_some(rows.as_slice());
        let renamed = fill(page, shown, &self.want.clipboard_trusted);
        self.clip_icons_due |= renamed;
        let changed = renamed || rows != self.clip_rows;
        self.clip_rows = rows;
        changed
    }

    pub(crate) fn show_clip_apps(&mut self, found: Vec<Installed>) {
        self.clip_installed = Some(found);
    }

    pub(crate) fn clip_changed(&mut self) -> Reply {
        let page = &self.sheet.pages[Page::Privacy.index()];
        if read(page, &self.clip_rows, &mut self.want.clipboard_trusted) {
            self.sync_apply();
        }
        let page = &mut self.sheet.pages[Page::Privacy.index()];
        let rows = self
            .clip_installed
            .is_some()
            .then_some(self.clip_rows.as_slice());
        fill(page, rows, &self.want.clipboard_trusted);
        Reply::redraw()
    }

    /// Each row's icon: its entry's, else the generic program icon.
    pub(crate) fn set_clip_icons(&mut self, get: &mut dyn FnMut(&str) -> Option<Canvas>) {
        let loaded = self
            .clip_rows
            .iter()
            .map(|r| {
                r.icon
                    .as_deref()
                    .and_then(&mut *get)
                    .or_else(|| get(openwith::PROGRAM_ICON))
            })
            .collect();
        if let Some((_, icons)) = list_mut(&mut self.sheet.pages[Page::Privacy.index()]) {
            *icons = loaded;
        }
        self.clip_icons_due = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pages::{Area, privacy};

    fn installed() -> Vec<Installed> {
        let named = |id: &str, name: &str| Installed {
            id: id.into(),
            entry: Some((name.into(), Some(format!("{id}-icon")))),
        };
        vec![
            named("org.example.Pass", "Pass"),
            Installed {
                id: "org.example.Tool".into(),
                entry: None,
            },
            named("org.example.Notes", "Notes"),
        ]
    }

    fn list(f: &Form) -> &ListBox {
        match f.item(LIST).map(|i| &i.control) {
            Some(Control::ListBox { list, .. }) => list,
            _ => panic!("the list is on the page"),
        }
    }

    #[test]
    fn the_list_holds_the_named_installed_apps_then_the_trusted_ids_left() {
        let trusted =
            ["org.example.Gone", "org.example.Tool", "org.example.Pass"].map(String::from);
        let got: Vec<(String, String, bool)> = rows(&installed(), &trusted)
            .into_iter()
            .map(|r| (r.id, r.name, r.installed))
            .collect();
        let want = [
            ("org.example.Notes", "Notes", true),
            ("org.example.Pass", "Pass", true),
            ("org.example.Gone", "org.example.Gone", false),
            ("org.example.Tool", "org.example.Tool", true),
        ]
        .map(|(i, n, at)| (i.to_string(), n.to_string(), at));
        assert_eq!(got, want);
    }

    #[test]
    fn the_ticks_follow_trusted_apps_and_write_it_back() {
        let mut trusted = vec!["org.example.Gone".to_string(), "org.example.Pass".into()];
        let rows = rows(&installed(), &trusted);
        let mut f = privacy(Area::MIN);
        assert!(fill(&mut f, Some(&rows), &trusted));
        let lb = list(&f);
        assert_eq!(lb.items, ["Notes", "Pass", "org.example.Gone"]);
        assert_eq!(lb.checks, [false, true, true]);
        assert_eq!(
            lb.detail,
            ["org.example.Notes", "org.example.Pass", "not installed"],
            "a named row shows its app id"
        );

        if let Some((lb, _)) = list_mut(&mut f) {
            lb.toggle(0);
            lb.toggle(2);
        }
        assert!(read(&f, &rows, &mut trusted));
        assert_eq!(trusted, ["org.example.Pass", "org.example.Notes"]);
        assert!(!read(&f, &rows, &mut trusted), "nothing moved");
        assert!(!fill(&mut f, Some(&rows), &trusted), "the same names");
    }

    #[test]
    fn the_list_says_loading_until_the_apps_are_read() {
        let mut f = privacy(Area::MIN);
        fill(&mut f, None, &[]);
        let lb = list(&f);
        assert_eq!(lb.items, ["Loading..."]);
        assert!(lb.checks.is_empty(), "nothing to tick");
        assert_eq!(lb.greyed, [true]);
    }
}
