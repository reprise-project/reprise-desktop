use std::path::Path;

use reprisefs::apps;
use reprisefs::program::{Program, ScriptOpen};

use crate::{
    compositor::State,
    desktop::{self, IconId, Target},
    startmenu::MenuItem,
    text::{escape_mnemonic, split_mnemonic},
};

use super::DesktopCommand;

pub(crate) fn entry(label: &str) -> MenuItem {
    let (label, mnemonic) = split_mnemonic(label);
    MenuItem::entry(label).mnemonic(mnemonic)
}

fn disabled_row(label: &str) -> MenuItem {
    entry(label).enabled(false)
}

/// A menu's rows, each with what it does, so a row and its command cannot drift.
type Rows = Vec<(MenuItem, Option<DesktopCommand>)>;

const SEP: (MenuItem, Option<DesktopCommand>) = (MenuItem::Separator, None);

fn split(rows: Rows) -> (Vec<MenuItem>, Vec<Option<DesktopCommand>>) {
    rows.into_iter().unzip()
}

/// The empty-desktop menu, ending on Reprise Settings and About Reprise. Paste
/// and Paste Shortcut are greyed unless `can_paste`. With `show_files` false, New
/// and the Paste pair are greyed (what they make would land in ~/Desktop, which
/// is not shown). While a `slideshow` runs, "Next desktop background" follows
/// Refresh.
pub fn desktop_menu(
    can_paste: bool,
    show_files: bool,
    slideshow: bool,
) -> (Vec<MenuItem>, Vec<Option<DesktopCommand>>) {
    use DesktopCommand::*;
    let new = if show_files {
        (entry("Ne&w").submenu(true), Some(New))
    } else {
        (disabled_row("Ne&w").submenu(true), None)
    };
    let paste = |label: &str, cmd: DesktopCommand| {
        if can_paste && show_files {
            (entry(label), Some(cmd))
        } else {
            (disabled_row(label), None)
        }
    };
    let mut rows = vec![
        (entry("Arrange &Icons By").submenu(true), Some(ArrangeIcons)),
        (entry("R&efresh"), Some(Refresh)),
        SEP,
        paste("&Paste", Paste),
        paste("Paste &Shortcut", PasteShortcut),
        SEP,
        new,
        SEP,
        (entry("&Reprise Settings"), Some(Settings)),
        (entry("&About Reprise"), Some(AboutReprise)),
    ];
    if slideshow {
        rows.insert(2, (entry("Next desktop &background"), Some(NextBackground)));
    }
    split(rows)
}

pub fn new_menu() -> (Vec<MenuItem>, Vec<Option<DesktopCommand>>) {
    use DesktopCommand::*;
    split(vec![
        (entry("&Folder"), Some(NewFolder)),
        SEP,
        (entry("&Text Document"), Some(NewTextDocument)),
    ])
}

pub const NEW_MENU_ICONS: [Option<&str>; 3] = [Some("folder"), None, Some("text-x-generic")];

pub fn arrange_menu(auto_arrange: bool) -> (Vec<MenuItem>, Vec<Option<DesktopCommand>>) {
    use DesktopCommand::*;
    split(vec![
        (entry("&Name"), Some(ArrangeByName)),
        SEP,
        (
            entry("&Auto Arrange").checked(auto_arrange),
            Some(AutoArrange),
        ),
    ])
}

/// The menu a right-button drop opens, `default`'s row in bold (what a
/// left drop would have done, or Create Shortcuts Here for a shortcut
/// drag from another program). A drag carrying a `system` icon has no
/// Copy Here: its place is not a file to copy. Rows another program's
/// drag does not allow (`allowed`) are greyed.
pub fn drop_menu(
    system: bool,
    default: crate::dragview::DropOp,
    allowed: crate::dragview::Allowed,
) -> (Vec<MenuItem>, Vec<Option<DesktopCommand>>) {
    use crate::dragview::DropOp;
    use DesktopCommand::*;
    // A system icon is never copied: its default falls to Move.
    let default = match default {
        DropOp::Copy if system => DropOp::Move,
        op => op,
    };
    let mut rows = Rows::new();
    if !system {
        rows.push((
            entry("&Copy Here")
                .bold(default == DropOp::Copy)
                .enabled(allowed.copy),
            Some(DropCopy),
        ));
    }
    rows.extend([
        (
            entry("&Move Here")
                .bold(default == DropOp::Move)
                .enabled(allowed.moves),
            Some(DropMove),
        ),
        (
            entry("Create &Shortcuts Here")
                .bold(default == DropOp::Shortcut)
                .enabled(allowed.copy),
            Some(DropShortcut),
        ),
        SEP,
        (entry("Cancel"), Some(DropCancel)),
    ]);
    split(rows)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IconMenu {
    pub is_dir: bool,
    pub print: bool,
    pub edit: bool,
    pub allow_launching: bool,
    pub launcher: bool,
    pub in_set: bool,
    /// An executable script, which runs as this default when opened.
    pub script: Option<ScriptOpen>,
}

/// A file's or folder's context menu. A script has Run and Run in terminal in place of
/// Open. A file gets its verbs (`print`, `edit`) and Open With, and Allow Launching with `allow_launching`. A launcher gets a greyed
/// Run as... under Open (a program runs in the session of the logged-in user and
/// another user's program cannot reach its display), no Open With, and Paste under
/// Copy. A folder gets Explore, the greyed Sharing and Security... (no sheet behind
/// it), and Paste under Copy. On
/// an icon `in_set` of a multi-selection it is the same menu less Sharing and
/// Security..., its rows acting on the whole set but for Explore, Paste, Rename
/// and Properties.
pub fn icon_menu(m: IconMenu) -> (Vec<MenuItem>, Vec<Option<DesktopCommand>>) {
    use DesktopCommand::*;
    let script = m
        .script
        .and_then(|open| reprisefs::program::run_rows(Program::Script, open));
    let mut rows = match script {
        Some(runs) => runs
            .iter()
            .map(|&(o, default)| (entry(o.label()).bold(default), Some(Run(o))))
            .collect(),
        None => vec![(entry("&Open").bold(true), Some(Open))],
    };
    if m.is_dir {
        rows.push((entry("E&xplore"), Some(Explore)));
        if !m.in_set {
            rows.extend([SEP, (disabled_row("S&haring and Security..."), None)]);
        }
    } else {
        if m.launcher {
            rows.push((disabled_row("Run &as..."), None));
        }
        // The file verbs between Open and Open With: Print, then Edit.
        if m.print {
            rows.push((entry("&Print"), Some(Print)));
        }
        if m.edit {
            rows.push((entry("&Edit"), Some(Edit)));
        }
        if !m.launcher {
            rows.push((entry("Open Wit&h").submenu(true), Some(OpenWith)));
        }
        if m.allow_launching {
            rows.push((entry("Allow &Launching"), Some(AllowLaunching)));
        }
    }
    rows.extend([
        SEP,
        (entry("Se&nd To").submenu(true), Some(SendTo)),
        SEP,
        (entry("Cu&t"), Some(Cut)),
        (entry("&Copy"), Some(Copy)),
    ]);
    if m.is_dir {
        rows.push((entry("&Paste"), Some(PasteInto)));
    } else if m.launcher {
        rows.push((entry("&Paste"), Some(PasteOpen)));
    }
    rows.extend([
        SEP,
        (entry("Create &Shortcut"), Some(CreateShortcut)),
        (entry("&Delete"), Some(Trash)),
        (entry("Rena&me"), Some(Rename)),
        SEP,
        (entry("P&roperties"), Some(IconProperties)),
    ]);
    split(rows)
}

pub fn send_to_menu(
    rows: &[reprisefs::sendto::SendToRow],
) -> (Vec<MenuItem>, Vec<Option<DesktopCommand>>) {
    rows.iter()
        .enumerate()
        .map(|(i, r)| {
            let label = escape_mnemonic(&r.label);
            if r.enabled {
                (entry(&label), Some(DesktopCommand::SendToDest(i)))
            } else {
                (disabled_row(&label), None)
            }
        })
        .unzip()
}

/// The "Open With" submenu: one row per program in `apps` (desktop-file id and
/// name), then "Choose Program..." after a separator (alone when there are none).
pub fn open_with_menu(apps: &[(String, String)]) -> (Vec<MenuItem>, Vec<Option<DesktopCommand>>) {
    let mut rows: Rows = apps
        .iter()
        .map(|(id, name)| {
            (
                entry(&escape_mnemonic(name)),
                Some(DesktopCommand::OpenWithApp(id.clone())),
            )
        })
        .collect();
    if !apps.is_empty() {
        rows.push(SEP);
    }
    rows.push((
        entry("&Choose Program..."),
        Some(DesktopCommand::ChooseProgram),
    ));
    split(rows)
}

/// A place icon's menu. Documents and Home (real folders) open, explore and
/// have Copy, and Properties with `sheet` (a folder that exists); Computer opens
/// and ends on Reprise Settings and About Reprise, after a separator. All three
/// Create Shortcut, Delete (hide) and Rename.
pub fn place_menu(
    place: desktop::Place,
    sheet: bool,
) -> (Vec<MenuItem>, Vec<Option<DesktopCommand>>) {
    use DesktopCommand::*;
    let computer = place == desktop::Place::Computer;
    let mut rows = vec![(entry("&Open").bold(true), Some(Open))];
    if !computer {
        rows.extend([
            (entry("E&xplore"), Some(Explore)),
            SEP,
            (entry("&Copy"), Some(Copy)),
        ]);
    }
    rows.extend([
        SEP,
        (entry("Create &Shortcut"), Some(CreateShortcut)),
        (entry("&Delete"), Some(Trash)),
        (entry("Rena&me"), Some(Rename)),
    ]);
    if computer {
        rows.extend([
            SEP,
            (entry("&Reprise Settings"), Some(RepriseSettings)),
            (entry("&About Reprise"), Some(AboutReprise)),
        ]);
    }
    if sheet {
        rows.extend([SEP, (entry("P&roperties"), Some(IconProperties))]);
    }
    split(rows)
}

pub fn bin_menu(empty: bool) -> (Vec<MenuItem>, Vec<Option<DesktopCommand>>) {
    use DesktopCommand::*;
    let empty_bin = if empty {
        (disabled_row("Empt&y Recycle Bin"), None)
    } else {
        (entry("Empt&y Recycle Bin"), Some(EmptyBin))
    };
    split(vec![
        (entry("&Open").bold(true), Some(Open)),
        (entry("E&xplore"), Some(Explore)),
        empty_bin,
        SEP,
        (entry("Create &Shortcut"), Some(CreateShortcut)),
        SEP,
        (entry("P&roperties"), Some(BinProperties)),
    ])
}

impl State {
    pub(crate) fn icon_menu_rows(
        &self,
        index: usize,
    ) -> Option<(Vec<MenuItem>, Vec<Option<DesktopCommand>>)> {
        use desktop::Kind;
        let view = self.desktop.as_ref()?;
        let entry = view.entries.get(index)?;
        Some(match entry.kind {
            // The scan's fullness, which its icon shows.
            Kind::RecycleBin => bin_menu(entry.icon == "user-trash"),
            Kind::Place(place) => place_menu(place, matches!(entry.target, Target::Open(_))),
            Kind::File => {
                let is_dir = entry.is_dir;
                let (print, edit) = if is_dir {
                    (false, false)
                } else {
                    let mime = view.mime.mime_of_name(&entry.name);
                    reprisefs::props::verbs(mime.as_deref(), reprisefs::props::has_lp())
                };
                icon_menu(IconMenu {
                    is_dir,
                    print,
                    edit,
                    allow_launching: entry.allow_launching,
                    launcher: matches!(entry.target, Target::Launch(_)),
                    in_set: self.selection_around(index).len() > 1,
                    script: matches!(entry.target, Target::Run(_, Program::Script))
                        .then_some(self.config.script_open),
                })
            }
        })
    }

    pub(crate) fn open_with_apps(&self, target: &IconId) -> Vec<(String, String)> {
        let Some(view) = self.desktop.as_ref() else {
            return Vec::new();
        };
        match desktop::find(&view.entries, target) {
            Some(i) => self.open_with_apps_for(&view.entries[i].path),
            None => Vec::new(),
        }
    }

    /// The programs registered for the type of the file `path` (by its name, with no stat):
    /// desktop-file id and name.
    pub(crate) fn open_with_apps_for(&self, path: &Path) -> Vec<(String, String)> {
        let empty;
        let mime = match self.desktop.as_ref() {
            Some(v) => &v.mime,
            None => {
                empty = apps::MimeDb::empty();
                &empty
            }
        };
        let index = &self.app_index;
        (path.file_name())
            .and_then(|n| mime.mime_of_name(&n.to_string_lossy()))
            .map(|mime| apps::open_with(index, &mime))
            .unwrap_or_default()
            .into_iter()
            .map(|i| (index[i].id.clone(), index[i].name.clone()))
            .collect()
    }
}
