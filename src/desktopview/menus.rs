use std::path::Path;

use reprisefs::apps;
use reprisefs::program::{Program, ScriptOpen};
use reprisekit::tr;

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

/// Pin to Places, or Unpin from Places when `pinned`; no row for a folder that cannot be
/// pinned from here (`None`).
fn pin_row(pinned: Option<bool>) -> Option<(MenuItem, Option<DesktopCommand>)> {
    pinned.map(|p| match p {
        false => (
            entry(&tr!("desk-menu-pin-to-places")),
            Some(DesktopCommand::Pin(true)),
        ),
        true => (
            entry(&tr!("desk-menu-unpin-from-places")),
            Some(DesktopCommand::Pin(false)),
        ),
    })
}

fn split(rows: Rows) -> (Vec<MenuItem>, Vec<Option<DesktopCommand>>) {
    rows.into_iter().unzip()
}

/// What the clipboard holds for the desktop's Paste rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pastable {
    Nothing,
    /// A picture alone: Paste saves it as a file, Paste Shortcut has nothing to link.
    Picture,
    Files,
}

/// The empty-desktop menu, the Widgets submenu under Arrange Icons By, ending on Reprise
/// Settings and About Reprise. Paste and Paste Shortcut are greyed unless `paste` offers them
/// ([`Pastable`]). With `show_files` false, New
/// and the Paste pair are greyed (what they make would land in ~/Desktop, which
/// is not shown). `undo` (the entry the history's head names, its "Undo New", and whether it can
/// run now) adds its row under the Paste pair, greyed while it cannot or the files are off. While a
/// `slideshow` runs, "Next desktop background" follows Refresh, ahead of Open in Terminal.
pub fn desktop_menu(
    paste: Pastable,
    show_files: bool,
    undo: Option<(reprisefs::undo::EntryId, &str, bool)>,
    slideshow: bool,
) -> (Vec<MenuItem>, Vec<Option<DesktopCommand>>) {
    use DesktopCommand::*;
    let new = if show_files {
        (entry(&tr!("desk-menu-new")).submenu(true), Some(New))
    } else {
        (disabled_row(&tr!("desk-menu-new")).submenu(true), None)
    };
    let paste = |label: &str, cmd: DesktopCommand| {
        let live = match cmd {
            PasteShortcut => paste == Pastable::Files,
            _ => paste != Pastable::Nothing,
        };
        if live && show_files {
            (entry(label), Some(cmd))
        } else {
            (disabled_row(label), None)
        }
    };
    let mut rows = vec![
        (
            entry(&tr!("desk-menu-arrange-icons-by")).submenu(true),
            Some(ArrangeIcons),
        ),
        (
            entry(&tr!("desk-menu-widgets")).submenu(true),
            Some(Widgets),
        ),
        (entry(&tr!("desk-menu-refresh")), Some(Refresh)),
        (
            entry(&tr!("desk-menu-open-in-terminal")),
            Some(OpenTerminal),
        ),
        SEP,
        paste(&tr!("desk-menu-paste"), Paste),
        paste(&tr!("desk-menu-paste-shortcut"), PasteShortcut),
        SEP,
        new,
        SEP,
        (entry(&tr!("desk-menu-reprise-settings")), Some(Settings)),
        (entry(&tr!("desk-menu-about-reprise")), Some(AboutReprise)),
    ];
    if let Some((id, label, live)) = undo {
        let live = live && show_files;
        let row = entry(&format!("&{label}"))
            .shortcut(Some("Ctrl+Z".into()))
            .enabled(live);
        rows.insert(7, (row, live.then_some(Undo(id))));
    }
    if slideshow {
        rows.insert(
            3,
            (
                entry(&tr!("desk-menu-next-desktop-background")),
                Some(NextBackground),
            ),
        );
    }
    split(rows)
}

/// New's rows: Folder and Shortcut..., then Text Document with the `templates` under it, then
/// Compressed (zipped) Folder.
pub fn new_menu(
    templates: &[reprisefs::templates::Template],
) -> (Vec<MenuItem>, Vec<Option<DesktopCommand>>) {
    use DesktopCommand::*;
    let mut rows = vec![
        (entry(&tr!("desk-menu-folder")), Some(NewFolder)),
        (entry(&tr!("desk-menu-shortcut")), Some(NewShortcut)),
        SEP,
        (
            entry(&tr!("desk-menu-text-document")),
            Some(NewTextDocument),
        ),
    ];
    for t in templates {
        let label = escape_mnemonic(&t.label);
        rows.push((entry(&label), Some(NewFrom(t.path.clone()))));
    }
    rows.push((
        entry(&tr!("desk-menu-compressed-folder")),
        Some(NewCompressedFolder),
    ));
    split(rows)
}

/// The icons of [`new_menu`]'s rows above its templates.
pub const NEW_MENU_ICONS: [Option<&str>; 4] = [Some("folder"), None, None, Some("text-x-generic")];

pub fn arrange_menu(auto_arrange: bool) -> (Vec<MenuItem>, Vec<Option<DesktopCommand>>) {
    use DesktopCommand::*;
    split(vec![
        (entry(&tr!("desk-menu-name")), Some(ArrangeByName)),
        SEP,
        (
            entry(&tr!("desk-menu-auto-arrange")).checked(auto_arrange),
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
            entry(&tr!("desk-menu-copy-here"))
                .bold(default == DropOp::Copy)
                .enabled(allowed.copy),
            Some(DropCopy),
        ));
    }
    rows.extend([
        (
            entry(&tr!("desk-menu-move-here"))
                .bold(default == DropOp::Move)
                .enabled(allowed.moves),
            Some(DropMove),
        ),
        (
            entry(&tr!("desk-menu-create-shortcuts-here"))
                .bold(default == DropOp::Shortcut)
                .enabled(allowed.copy),
            Some(DropShortcut),
        ),
        SEP,
        (entry(&tr!("desk-menu-cancel")), Some(DropCancel)),
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
    /// For a lone folder, whether it is pinned to the places bar: Pin to Places or Unpin.
    pub pinned: Option<bool>,
}

/// A file's or folder's context menu. A script has Run and Run in terminal in place of
/// Open. A file gets its verbs (`print`, `edit`) and Open With, and Allow Launching with `allow_launching`. A launcher gets a greyed
/// Run as... under Open (a program runs in the session of the logged-in user and
/// another user's program cannot reach its display), no Open With, and Paste under
/// Copy. A folder gets Explore, Open in Terminal, Pin to Places (or Unpin) when alone, the greyed Sharing and Security... (no sheet
/// behind it), and Paste under Copy. On
/// an icon `in_set` of a multi-selection it is the same menu less Sharing and
/// Security..., its rows acting on the whole set but for Explore, Open in Terminal, Paste,
/// Rename and Properties.
pub fn icon_menu(m: IconMenu) -> (Vec<MenuItem>, Vec<Option<DesktopCommand>>) {
    use DesktopCommand::*;
    let script = m
        .script
        .and_then(|open| reprisefs::program::run_rows(Program::Script, open));
    let mut rows = match script {
        Some(runs) => runs
            .iter()
            .map(|&(o, default)| (entry(&o.label()).bold(default), Some(Run(o))))
            .collect(),
        None => vec![(entry(&tr!("desk-menu-open")).bold(true), Some(Open))],
    };
    if m.is_dir {
        rows.push((entry(&tr!("desk-menu-explore")), Some(Explore)));
        rows.push((
            entry(&tr!("desk-menu-open-in-terminal")),
            Some(OpenTerminal),
        ));
        rows.extend(pin_row(m.pinned));
        if !m.in_set {
            rows.extend([
                SEP,
                (disabled_row(&tr!("desk-menu-sharing-and-security")), None),
            ]);
        }
    } else {
        if m.launcher {
            rows.push((disabled_row(&tr!("desk-menu-run-as")), None));
        }
        // The file verbs between Open and Open With: Print, then Edit.
        if m.print {
            rows.push((entry(&tr!("desk-menu-print")), Some(Print)));
        }
        if m.edit {
            rows.push((entry(&tr!("desk-menu-edit")), Some(Edit)));
        }
        if !m.launcher {
            rows.push((
                entry(&tr!("desk-menu-open-with")).submenu(true),
                Some(OpenWith),
            ));
        }
        if m.allow_launching {
            rows.push((
                entry(&tr!("desk-menu-allow-launching")),
                Some(AllowLaunching),
            ));
        }
    }
    rows.extend([
        SEP,
        (entry(&tr!("desk-menu-send-to")).submenu(true), Some(SendTo)),
        SEP,
        (entry(&tr!("desk-menu-cut")), Some(Cut)),
        (entry(&tr!("desk-menu-copy")), Some(Copy)),
    ]);
    if m.is_dir {
        rows.push((entry(&tr!("desk-menu-paste")), Some(PasteInto)));
    } else if m.launcher {
        rows.push((entry(&tr!("desk-menu-paste")), Some(PasteOpen)));
    }
    rows.extend([
        SEP,
        (
            entry(&tr!("desk-menu-create-shortcut")),
            Some(CreateShortcut),
        ),
        (entry(&tr!("desk-menu-delete")), Some(Trash)),
        (entry(&tr!("desk-menu-rename")), Some(Rename)),
        SEP,
        (entry(&tr!("desk-menu-properties")), Some(IconProperties)),
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
pub fn open_with_menu(
    apps: &[(apps::DesktopId, String)],
) -> (Vec<MenuItem>, Vec<Option<DesktopCommand>>) {
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
        entry(&tr!("desk-menu-choose-program")),
        Some(DesktopCommand::ChooseProgram),
    ));
    split(rows)
}

/// A place icon's menu. Documents and Home (real folders) open, explore, open in the
/// terminal, pin to the places bar as `pinned` says (as [`icon_menu`] does) and have Copy, and
/// Properties with `sheet` (a folder that exists); Computer opens
/// and ends on Reprise Settings and About Reprise, after a separator. All three
/// Create Shortcut, Delete (hide) and Rename.
pub fn place_menu(
    place: desktop::Place,
    sheet: bool,
    pinned: Option<bool>,
) -> (Vec<MenuItem>, Vec<Option<DesktopCommand>>) {
    use DesktopCommand::*;
    let computer = place == desktop::Place::Computer;
    let mut rows = vec![(entry(&tr!("desk-menu-open")).bold(true), Some(Open))];
    if !computer {
        rows.extend([
            (entry(&tr!("desk-menu-explore")), Some(Explore)),
            (
                entry(&tr!("desk-menu-open-in-terminal")),
                Some(OpenTerminal),
            ),
        ]);
        rows.extend(pin_row(pinned));
        rows.extend([SEP, (entry(&tr!("desk-menu-copy")), Some(Copy))]);
    }
    rows.extend([
        SEP,
        (
            entry(&tr!("desk-menu-create-shortcut")),
            Some(CreateShortcut),
        ),
        (entry(&tr!("desk-menu-delete")), Some(Trash)),
        (entry(&tr!("desk-menu-rename")), Some(Rename)),
    ]);
    if computer {
        rows.extend([
            SEP,
            (
                entry(&tr!("desk-menu-reprise-settings")),
                Some(RepriseSettings),
            ),
            (entry(&tr!("desk-menu-about-reprise")), Some(AboutReprise)),
        ]);
    }
    if sheet {
        rows.extend([
            SEP,
            (entry(&tr!("desk-menu-properties")), Some(IconProperties)),
        ]);
    }
    split(rows)
}

pub fn bin_menu(empty: bool) -> (Vec<MenuItem>, Vec<Option<DesktopCommand>>) {
    use DesktopCommand::*;
    let empty_bin = if empty {
        (disabled_row(&tr!("desk-menu-empty-recycle-bin")), None)
    } else {
        (entry(&tr!("desk-menu-empty-recycle-bin")), Some(EmptyBin))
    };
    split(vec![
        (entry(&tr!("desk-menu-open")).bold(true), Some(Open)),
        (entry(&tr!("desk-menu-explore")), Some(Explore)),
        empty_bin,
        SEP,
        (
            entry(&tr!("desk-menu-create-shortcut")),
            Some(CreateShortcut),
        ),
        SEP,
        (entry(&tr!("desk-menu-properties")), Some(BinProperties)),
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
        // A place's `is_dir` says it opens a folder; Computer's does not.
        let pinned = (entry.is_dir && self.selection_around(index).len() <= 1)
            .then_some(self.dialogs.pins.as_ref())
            .flatten()
            .map(|p| reprisefs::pins::is_pinned(&p.current(), &entry.path));
        Some(match entry.kind {
            // The scan's fullness, which its icon shows.
            Kind::RecycleBin => bin_menu(entry.icon == "user-trash"),
            Kind::Place(place) => {
                place_menu(place, matches!(entry.target, Target::Open(_)), pinned)
            }
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
                    pinned,
                })
            }
        })
    }

    pub(crate) fn open_with_apps(&self, target: &IconId) -> Vec<(apps::DesktopId, String)> {
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
    pub(crate) fn open_with_apps_for(&self, path: &Path) -> Vec<(apps::DesktopId, String)> {
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
