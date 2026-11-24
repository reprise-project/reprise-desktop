//! The Start menu a config without `[[startmenu]]` gets, as the file would spell it, and that
//! menu with `[[startmenu_changes]]` made; the compositor and reprise-settings both convert it,
//! and `config.example.toml` shows it.
//!
//! A change names a default row by its id, so a row the user did not touch follows the defaults
//! of the Reprise that reads the file.

use super::file::{FileMenuChange, FileMenuEntry};
use serde::Deserialize;

pub const PRINTERS: &str = "reprise-printers";
pub const PRINTERS_TIP: &str =
    "Add, remove, and configure local and network printers, and see what is printing.";
pub const HELP: &str = "xdg-open https://github.com/reprise-project/reprise-desktop";
pub const CONTROL_PANEL_TIP: &str = "Provides options for you to customize the appearance \
     and functionality of your computer, add or remove programs, and set up network connections \
     and user accounts.";

/// The default `[[startmenu]]` entries, each with the id `[[startmenu_changes]]` names it by.
pub fn default_startmenu_entries() -> Vec<FileMenuEntry> {
    let row = |id: &str, label: &str, icon: &str| FileMenuEntry {
        label: Some(label.into()),
        icon: Some(icon.into()),
        id: Some(id.into()),
        ..FileMenuEntry::default()
    };
    let submenu = |id, label, icon, kind: &str| FileMenuEntry {
        submenu: Some(kind.into()),
        ..row(id, label, icon)
    };
    let command = |id, label, icon, cmd: &str| FileMenuEntry {
        command: Some(cmd.into()),
        ..row(id, label, icon)
    };
    let action = |id, label, icon, name: &str| FileMenuEntry {
        action: Some(name.into()),
        ..row(id, label, icon)
    };
    let folder = |id, label, icon, items| FileMenuEntry {
        items: Some(items),
        ..row(id, label, icon)
    };
    let tipped = |e: FileMenuEntry, tip: &str| FileMenuEntry {
        tip: Some(tip.into()),
        ..e
    };
    let separator = |id: &str| FileMenuEntry {
        separator: Some(true),
        id: Some(id.into()),
        ..FileMenuEntry::default()
    };
    vec![
        submenu("programs", "&Programs", "applications-other", "programs"),
        submenu("documents", "&Documents", "folder-documents", "documents"),
        folder(
            "settings",
            "&Settings",
            "preferences-system",
            vec![
                tipped(
                    submenu(
                        "control-panel",
                        "&Control Panel",
                        "preferences-other",
                        "settings",
                    ),
                    CONTROL_PANEL_TIP,
                ),
                separator("settings-separator"),
                tipped(
                    command(
                        "network-connections",
                        "&Network Connections",
                        "network-workgroup",
                        "reprise-network",
                    ),
                    "Displays existing network connections on this computer and helps you \
                     create new ones",
                ),
                tipped(
                    command("printers", "&Printers", "printer", PRINTERS),
                    PRINTERS_TIP,
                ),
                tipped(
                    command(
                        "taskbar",
                        "&Taskbar and Start Menu",
                        "preferences-desktop",
                        "reprise-settings --page taskbar",
                    ),
                    "Customize the Start Menu and the taskbar, such as the types of items to \
                     be displayed and how they should appear.",
                ),
            ],
        ),
        folder(
            "search",
            "Sear&ch",
            "system-search",
            vec![
                command(
                    "search-files",
                    "&For Files or Folders...",
                    "system-search",
                    "reprise-explorer --search \"$HOME\"",
                ),
                command(
                    "search-internet",
                    "On the &Internet...",
                    "web-browser",
                    "xdg-open https://duckduckgo.com/",
                ),
            ],
        ),
        command("help", "&Help and Support", "help-browser", HELP),
        action("run", "&Run...", "system-run", "run"),
        separator("separator"),
        action(
            "logoff",
            "&Log Off {user}...",
            "gnome-session-logout",
            "logoff",
        ),
        action(
            "turnoff",
            "T&urn Off Computer...",
            "system-shutdown",
            "turnoff",
        ),
    ]
}

/// The default menu with `changes` made. A row of the user's own whose `folder` is no default
/// folder goes at the menu's end; [`stray_rows`] names them.
pub fn startmenu_with(changes: &[FileMenuChange]) -> Vec<FileMenuEntry> {
    menu_with(default_startmenu_entries(), changes)
}

/// The rows of the user's own in `changes` that [`startmenu_with`] puts at the menu's end.
pub fn stray_rows(changes: &[FileMenuChange]) -> Vec<&FileMenuChange> {
    strays(&default_startmenu_entries(), changes).collect()
}

fn menu_with(defaults: Vec<FileMenuEntry>, changes: &[FileMenuChange]) -> Vec<FileMenuEntry> {
    let end: Vec<FileMenuEntry> = strays(&defaults, changes).map(|c| c.entry()).collect();
    let mut menu = folder_with("", defaults, changes);
    menu.extend(end);
    menu
}

fn strays<'a>(
    defaults: &[FileMenuEntry],
    changes: &'a [FileMenuChange],
) -> impl Iterator<Item = &'a FileMenuChange> {
    let folders = folder_ids(defaults);
    (changes.iter()).filter(move |c| {
        c.default.is_none()
            && (c.folder.as_deref())
                .is_some_and(|f| !f.is_empty() && !folders.iter().any(|d| d == f))
    })
}

/// The ids of every default folder, nested ones too.
fn folder_ids(defaults: &[FileMenuEntry]) -> Vec<String> {
    let mut folders = Vec::new();
    walk(defaults, &mut |e| {
        if e.items.is_some() {
            folders.extend(e.id.clone());
        }
        true
    });
    folders
}

/// Calls `f` on each default row, depth first, going into a folder when `f` returns true.
fn walk(rows: &[FileMenuEntry], f: &mut impl FnMut(&FileMenuEntry) -> bool) {
    for e in rows {
        if f(e) {
            walk(e.items.as_deref().unwrap_or_default(), f);
        }
    }
}

/// The `[[startmenu_changes]]` in the TOML `text` in file order, None when it has none. An entry
/// that does not read or that [`FileMenuChange::check`] refuses is None, as the compositor drops it.
pub fn startmenu_changes_in(text: &str) -> Option<Vec<Option<FileMenuChange>>> {
    let table: toml::Table = toml::from_str(text).ok()?;
    let list = table.get("startmenu_changes")?.as_array()?;
    Some(
        (list.iter())
            .map(|v| FileMenuChange::deserialize(v.clone()).ok())
            .map(|c| c.filter(|c| c.check().is_ok()))
            .collect(),
    )
}

/// Which of `changes` leave alone the menu [`startmenu_with`] builds from changes hiding the
/// default rows `hidden`: they name a default row this build lacks or one inside a hidden folder,
/// or put a row into such a folder.
pub fn idle_changes(changes: &[FileMenuChange], hidden: &[String]) -> Vec<bool> {
    idle_in(&default_startmenu_entries(), changes, hidden)
}

fn idle_in(defaults: &[FileMenuEntry], changes: &[FileMenuChange], hidden: &[String]) -> Vec<bool> {
    let folders = folder_ids(defaults);
    let (mut reached, mut open) = (Vec::new(), Vec::new());
    walk(defaults, &mut |e| {
        let id = e.id.clone().unwrap_or_default();
        reached.push(id.clone());
        let shows = e.items.is_some() && !hidden.contains(&id);
        if shows {
            open.push(id);
        }
        shows
    });
    (changes.iter())
        .map(|c| match (&c.default, c.folder.as_deref().unwrap_or("")) {
            (Some(id), _) => !reached.contains(id),
            (None, "") => false,
            (None, f) => folders.iter().any(|d| d == f) && !open.iter().any(|d| d == f),
        })
        .collect()
}

impl FileMenuChange {
    /// Whether the change fits its kind: a change to a default row sets only its label, icon,
    /// tip, whether it is hidden and where it goes; a row of the user's own is not `hidden` and
    /// passes [`FileMenuEntry::check`].
    pub fn check(&self) -> Result<(), String> {
        let Some(id) = &self.default else {
            return match self.hidden {
                Some(_) => Err(format!(
                    "startmenu_changes: {:?} sets hidden, which needs default = \"<id>\"; a row \
                     of your own is left out by removing its change",
                    self.label.as_deref().unwrap_or("-")
                )),
                None => self
                    .entry()
                    .check()
                    .map_err(|e| format!("startmenu_changes: {e}")),
            };
        };
        let target = self.command.is_some()
            || self.action.is_some()
            || self.separator.is_some()
            || self.submenu.is_some()
            || self.items.is_some()
            || self.folder.is_some();
        match target {
            true => Err(format!(
                "startmenu_changes: the change to default row {id:?} sets only label, icon, tip, \
                 hidden and after"
            )),
            false => Ok(()),
        }
    }

    /// The row of the user's own this change adds.
    fn entry(&self) -> FileMenuEntry {
        FileMenuEntry {
            label: self.label.clone(),
            icon: self.icon.clone(),
            command: self.command.clone(),
            action: self.action.clone(),
            separator: self.separator,
            submenu: self.submenu.clone(),
            items: self.items.clone(),
            tip: self.tip.clone(),
            id: None,
        }
    }
}

impl FileMenuEntry {
    /// Whether the compositor can build the row: a separator, or a label with at most one of
    /// `command` or `action`, `submenu` and `items`, naming a known action or submenu kind, and
    /// rows in `items` that pass too.
    pub fn check(&self) -> Result<(), String> {
        if self.separator == Some(true) {
            return Ok(());
        }
        let label =
            (self.label.as_deref()).ok_or("startmenu entry needs a label or separator = true")?;
        match (&self.action, &self.command, &self.submenu, &self.items) {
            (Some(a), None, None, None) if a == "spawn" => Err("spawn needs a command".into()),
            (Some(a), _, None, None) if a != "spawn" && crate::keybind::describe(a).is_none() => {
                Err(format!("startmenu entry {label:?}: unknown action {a:?}"))
            }
            (Some(_), _, None, None) | (None, _, None, None) => Ok(()),
            (None, None, Some(kind), None) => match kind.as_str() {
                "programs" | "settings" | "documents" => Ok(()),
                other => Err(format!(
                    "startmenu entry {label:?}: unknown submenu {other:?}, expected programs|settings|documents"
                )),
            },
            (None, None, None, Some(items)) => items.iter().try_for_each(FileMenuEntry::check),
            _ => Err(format!(
                "startmenu entry {label:?}: command/action, submenu and items are mutually exclusive"
            )),
        }
    }
}

/// The folder `folder` (`""` for the menu itself) holding `defaults`, with `changes` made. Slot 0
/// is the folder's top, slot `i + 1` follows default row `i` (shown or not), the last its end.
fn folder_with(
    folder: &str,
    defaults: Vec<FileMenuEntry>,
    changes: &[FileMenuChange],
) -> Vec<FileMenuEntry> {
    let n = defaults.len();
    let slot = |after: &str| match after {
        "" => Some(0),
        id => (defaults.iter())
            .position(|d| d.id.as_deref() == Some(id))
            .map(|i| i + 1),
    };
    let mut slots: Vec<Vec<(usize, FileMenuEntry)>> = vec![Vec::new(); n + 2];
    let mut stay: Vec<Option<FileMenuEntry>> = vec![None; n];
    for (i, mut row) in defaults.clone().into_iter().enumerate() {
        let id = row.id.clone().unwrap_or_default();
        let mine: Vec<(usize, &FileMenuChange)> = (changes.iter().enumerate())
            .filter(|(_, c)| c.default.as_deref() == Some(id.as_str()))
            .collect();
        if mine.iter().any(|(_, c)| c.hidden == Some(true)) {
            continue;
        }
        for (_, c) in &mine {
            row.label = c.label.clone().or(row.label);
            row.icon = c.icon.clone().or(row.icon);
            row.tip = c.tip.clone().or(row.tip);
        }
        if let Some(items) = row.items.take() {
            row.items = Some(folder_with(&id, items, changes));
        }
        // An `after` naming no row of this folder leaves the row where it is.
        let moved = (mine.iter().rev()).find_map(|(at, c)| Some((*at, slot(c.after.as_deref()?)?)));
        match moved {
            Some((at, to)) => slots[to].push((at, row)),
            None => stay[i] = Some(row),
        }
    }
    for (at, c) in changes.iter().enumerate() {
        if c.default.is_none() && c.folder.as_deref().unwrap_or("") == folder {
            let to = c.after.as_deref().and_then(slot).unwrap_or(n + 1);
            slots[to].push((at, c.entry()));
        }
    }
    for s in &mut slots {
        s.sort_by_key(|(at, _)| *at);
    }
    let mut slots = slots.into_iter().map(|s| s.into_iter().map(|(_, e)| e));
    let mut out: Vec<FileMenuEntry> = slots.next().into_iter().flatten().collect();
    for (row, after) in stay.into_iter().zip(slots.by_ref()) {
        out.extend(row);
        out.extend(after);
    }
    out.extend(slots.flatten());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn changes(text: &str) -> Vec<FileMenuChange> {
        #[derive(serde::Deserialize)]
        struct File {
            startmenu_changes: Vec<FileMenuChange>,
        }
        toml::from_str::<File>(text).unwrap().startmenu_changes
    }

    fn labels(rows: &[FileMenuEntry]) -> Vec<&str> {
        (rows.iter())
            .map(|e| e.label.as_deref().unwrap_or("-"))
            .collect()
    }

    #[test]
    fn no_changes_is_the_default_menu() {
        assert_eq!(startmenu_with(&[]), default_startmenu_entries());
    }

    #[test]
    fn rows_hide_move_rename_and_join_the_menu() {
        let menu = startmenu_with(&changes(
            r#"
[[startmenu_changes]]
default = "help"
hidden = true

[[startmenu_changes]]
default = "run"
label = "&Go..."
after = ""

[[startmenu_changes]]
label = "Terminal"
command = "foot"
after = ""

[[startmenu_changes]]
label = "Last"
command = "last"

[[startmenu_changes]]
label = "Mine"
command = "mine"
folder = "search"
after = "search-files"
"#,
        ));
        assert_eq!(
            labels(&menu),
            [
                "&Go...",
                "Terminal",
                "&Programs",
                "&Documents",
                "&Settings",
                "Sear&ch",
                "-",
                "&Log Off {user}...",
                "T&urn Off Computer...",
                "Last",
            ]
        );
        assert_eq!(menu[0].action.as_deref(), Some("run"));
        assert_eq!(
            labels(menu[5].items.as_deref().unwrap()),
            ["&For Files or Folders...", "Mine", "On the &Internet..."]
        );
    }

    #[test]
    fn a_later_default_reaches_rows_the_changes_leave_alone() {
        let saved = changes(
            r#"
[[startmenu_changes]]
default = "b"
label = "Bee"

[[startmenu_changes]]
label = "Mine"
command = "mine"
after = "a"
"#,
        );
        let row = |id: &str, cmd: &str| FileMenuEntry {
            label: Some(id.to_uppercase()),
            command: Some(cmd.into()),
            id: Some(id.into()),
            ..FileMenuEntry::default()
        };
        let later = vec![row("a", "a2"), row("new", "new"), row("b", "b2")];
        let menu = folder_with("", later, &saved);
        assert_eq!(labels(&menu), ["A", "Mine", "NEW", "Bee"]);
        let commands: Vec<_> = menu.iter().map(|e| e.command.as_deref()).collect();
        assert_eq!(
            commands,
            [Some("a2"), Some("mine"), Some("new"), Some("b2")],
            "the renamed row runs the later default's command"
        );
    }

    #[test]
    fn a_change_naming_a_row_the_defaults_dropped_is_ignored() {
        let menu = startmenu_with(&changes(
            r#"
[[startmenu_changes]]
default = "gone"
hidden = true

[[startmenu_changes]]
label = "After gone"
command = "x"
after = "gone"
"#,
        ));
        let defaults = default_startmenu_entries();
        let mut want = labels(&defaults);
        want.push("After gone");
        assert_eq!(labels(&menu), want, "the row goes at the menu's end");
    }

    #[test]
    fn a_change_to_a_default_row_sets_only_its_look_and_place() {
        let bad = changes("[[startmenu_changes]]\ndefault = \"help\"\ncommand = \"x\"\n");
        assert!(bad[0].check().unwrap_err().contains("\"help\""));
        let ok = changes("[[startmenu_changes]]\ndefault = \"help\"\nlabel = \"x\"\n");
        assert_eq!(ok[0].check(), Ok(()));
    }

    #[test]
    fn a_row_of_the_users_own_cannot_be_hidden() {
        let bad = changes("[[startmenu_changes]]\nlabel = \"A\"\ncommand = \"a\"\nhidden = true\n");
        assert!(bad[0].check().unwrap_err().contains("needs default"));
    }

    #[test]
    fn default_ids_never_change_and_name_one_row_each() {
        let mut ids = Vec::new();
        walk(&default_startmenu_entries(), &mut |e| {
            ids.extend(e.id.clone());
            true
        });
        assert_eq!(
            ids,
            [
                "programs",
                "documents",
                "settings",
                "control-panel",
                "settings-separator",
                "network-connections",
                "printers",
                "taskbar",
                "search",
                "search-files",
                "search-internet",
                "help",
                "run",
                "separator",
                "logoff",
                "turnoff",
            ],
            "users' config files name default rows by these ids: add new ones, never rename or \
             reuse one"
        );
        let mut unique = ids.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), ids.len(), "each id names one row");
    }

    #[test]
    fn a_row_whose_folder_is_no_default_folder_goes_at_the_menus_end() {
        let saved = changes(
            r#"
[[startmenu_changes]]
label = "In search"
command = "a"
folder = "search"
after = "search-files"

[[startmenu_changes]]
label = "In help"
command = "b"
folder = "help"

[[startmenu_changes]]
default = "search"
label = "Find"
"#,
        );
        let later: Vec<FileMenuEntry> = (default_startmenu_entries().into_iter())
            .filter(|e| e.id.as_deref() != Some("search"))
            .collect();
        let mut want = labels(&later);
        want.extend(["In search", "In help"]);
        assert_eq!(
            labels(&menu_with(later.clone(), &saved)),
            want,
            "a later menu without Search keeps its rows"
        );
        let now = startmenu_with(&saved);
        assert_eq!(labels(&now).last(), Some(&"In help"));
        assert_eq!(stray_rows(&saved), [&saved[1]]);
        assert_eq!(
            labels(now[3].items.as_deref().unwrap()),
            [
                "&For Files or Folders...",
                "In search",
                "On the &Internet..."
            ]
        );
    }

    #[test]
    fn an_after_naming_no_row_of_the_folder_leaves_a_default_row_in_place() {
        let menu = startmenu_with(&changes(
            "[[startmenu_changes]]\ndefault = \"run\"\nafter = \"gone\"\n",
        ));
        assert_eq!(menu, default_startmenu_entries());
    }

    #[test]
    fn changes_the_menu_does_not_show_are_idle() {
        let saved = changes(
            r#"
[[startmenu_changes]]
default = "gone"
hidden = true

[[startmenu_changes]]
default = "search"
hidden = true

[[startmenu_changes]]
default = "search-files"
label = "Files"

[[startmenu_changes]]
label = "Mine"
command = "mine"
folder = "search"

[[startmenu_changes]]
label = "Lost"
command = "lost"
folder = "typo"

[[startmenu_changes]]
default = "printers"
label = "Print"
"#,
        );
        assert_eq!(
            idle_changes(&saved, &["gone".into(), "search".into()]),
            [true, false, true, true, false, false],
            "a row inside hidden Search or one this build lacks"
        );
        assert_eq!(
            idle_changes(&saved, &[]),
            [true, false, false, false, false, false],
            "Search shown by the changes being written"
        );
    }

    #[test]
    fn a_row_in_a_default_folder_inside_a_hidden_one_is_idle() {
        let folder = |id: &str, items| FileMenuEntry {
            label: Some(id.into()),
            items: Some(items),
            id: Some(id.into()),
            ..FileMenuEntry::default()
        };
        let defaults = vec![folder("outer", vec![folder("inner", Vec::new())])];
        let saved = changes(
            r#"
[[startmenu_changes]]
default = "outer"
hidden = true

[[startmenu_changes]]
label = "Mine"
command = "mine"
folder = "inner"
"#,
        );
        assert_eq!(idle_in(&defaults, &saved, &["outer".into()]), [false, true]);
        assert_eq!(
            menu_with(defaults.clone(), &saved),
            [],
            "Start shows it nowhere"
        );
        assert_eq!(idle_in(&defaults, &saved, &[]), [false, false]);
    }

    #[test]
    fn a_row_of_the_users_own_is_refused_where_start_cannot_build_it() {
        let refused = [
            (
                "submenu = \"programs\"\nlabel = \"P\"\n[[startmenu_changes.items]]\nlabel = \"A\"\ncommand = \"a\"\n",
                "mutually exclusive",
            ),
            (
                "command = \"c\"\nlabel = \"C\"\n[[startmenu_changes.items]]\nlabel = \"A\"\ncommand = \"a\"\n",
                "mutually exclusive",
            ),
            ("command = \"c\"\n", "needs a label"),
            ("label = \"B\"\naction = \"bogus\"\n", "unknown action"),
            (
                "label = \"B\"\naction = \"spawn\"\n",
                "spawn needs a command",
            ),
            ("label = \"B\"\nsubmenu = \"bogus\"\n", "unknown submenu"),
            (
                "label = \"F\"\n[[startmenu_changes.items]]\nicon = \"x\"\n",
                "needs a label",
            ),
        ];
        for (body, why) in refused {
            let c = changes(&format!("[[startmenu_changes]]\n{body}"));
            let err = c[0].check().unwrap_err();
            assert!(err.contains(why), "{body}: {err}");
        }
        let built = [
            "separator = true\n",
            "label = \"C\"\ncommand = \"c\"\n",
            "label = \"R\"\naction = \"run\"\n",
            "label = \"S\"\naction = \"spawn\"\ncommand = \"s\"\n",
            "label = \"D\"\nsubmenu = \"documents\"\n",
            "label = \"Empty\"\n",
            "label = \"F\"\n[[startmenu_changes.items]]\nlabel = \"A\"\ncommand = \"a\"\n",
        ];
        for body in built {
            let c = changes(&format!("[[startmenu_changes]]\n{body}"));
            assert_eq!(c[0].check(), Ok(()), "{body}");
        }
        for e in default_startmenu_entries() {
            assert_eq!(e.check(), Ok(()), "{:?}", e.id);
        }
    }
}
