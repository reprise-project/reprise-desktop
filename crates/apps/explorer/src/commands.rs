//! Every input (menu bar, context menus, toolbar, task pane, keys) resolves to a [`Command`]; [`Explorer::run`] is the only place that acts on one.

use crate::dialogs::ConfirmKind;
use crate::place::Place;
use crate::{Explorer, Focus};
use repriseapp::{Ctx, Reply};
use reprisekit::controls::menu::{MenuItem, MenuTree};
use reprisekit::controls::shellview::Mode;
use reprisekit::textedit::keysym::{self, F2, F5, MENU};

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Open,
    OpenWith(usize),
    ChooseProgram,
    Explore,
    Eject,
    Cut,
    Copy,
    Paste,
    PasteShortcut,
    /// Paste on the item menu's entry: into a folder, or a launcher opens the clipboard's files.
    PasteOnItem,
    Delete {
        erase: bool,
    },
    Rename,
    NewFolder,
    EmptyRecycleBin,
    Restore,
    NewTextDocument,
    CreateShortcut,
    SendTo(usize),
    MoveTo,
    CopyTo,
    Drop(reprisefs::ops::DropOp),
    DropCancel,
    Properties,
    Undo,
    SelectAll,
    InvertSelection,
    Close,
    Back,
    Forward,
    Up,
    Go(usize),
    GoForward(usize),
    Mode(Mode),
    /// Sort by a Details column, flipping the direction when it is
    /// already the sort column.
    Sort(usize),
    ToggleFolders,
    ToggleStatusBar,
    /// 0 Standard Buttons, 1 Address Bar, 2 Links.
    ToggleToolbar(usize),
    ToggleNode,
    /// Go to the selected node of the Folders tree: the tree menu's
    /// Explore and Open, which act on the node, not the view's selection.
    OpenNode,
    /// The tree menu's Cut, Copy, Delete and Properties: the node's own
    /// folder, never the view's selection.
    NodeCut,
    NodeCopy,
    NodeDelete,
    NodeProperties,
    Refresh,
    GoTo(Place),
    Navigate(String),
    AddressFocus,
    ContextMenu,
    CloseBar,
    About,
    SystemInformation,
    Nothing,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Menu {
    items: Vec<MenuItem>,
    cmds: Vec<Command>,
    subs: Vec<Option<Menu>>,
}

impl Menu {
    pub fn row(&mut self, item: MenuItem, cmd: Command) {
        self.items.push(item);
        self.cmds.push(cmd);
        self.subs.push(None);
    }

    pub fn sep(&mut self) {
        self.row(MenuItem::Separator, Command::Nothing);
    }

    pub fn sub(&mut self, item: MenuItem, child: Menu) {
        self.items.push(item);
        self.cmds.push(Command::Nothing);
        self.subs.push(Some(child));
    }

    pub fn tree(&self) -> MenuTree {
        let mut t = MenuTree::new();
        for (item, sub) in self.items.iter().zip(&self.subs) {
            match sub {
                Some(child) => t.push_sub(item.clone(), child.tree()),
                None => t.push(item.clone()),
            }
        }
        t
    }

    /// What the row at `path` does; `Nothing` for a separator, a submenu
    /// row or a path this menu does not have.
    pub fn command(&self, path: &[usize]) -> Command {
        match path.split_first() {
            Some((&row, [])) => self.cmds.get(row).cloned().unwrap_or(Command::Nothing),
            Some((&row, rest)) => match self.subs.get(row).and_then(|s| s.as_ref()) {
                Some(child) => child.command(rest),
                None => Command::Nothing,
            },
            None => Command::Nothing,
        }
    }

    pub fn items(&self) -> &[MenuItem] {
        &self.items
    }

    #[cfg(test)]
    pub fn at(&self, path: &[usize]) -> Option<&Menu> {
        match path.split_first() {
            None => Some(self),
            Some((&row, rest)) => self.subs.get(row)?.as_ref()?.at(rest),
        }
    }
}

/// The letter a Latin letter keysym types, folded to lowercase: those
/// keysyms are the letter's ASCII code.
fn letter(sym: u32) -> Option<char> {
    char::from_u32(sym | 0x20).filter(char::is_ascii_lowercase)
}

/// The command a key press means once the menu bar has passed on it and
/// no editable field has focus. `Nothing` leaves the key to the focused
/// control (the tree's arrows, the view's type-ahead).
pub fn command_for_key(sym: u32, ctrl: bool, shift: bool, alt: bool) -> Command {
    match sym {
        keysym::LEFT if alt => Command::Back,
        keysym::RIGHT if alt => Command::Forward,
        keysym::RETURN | keysym::KP_ENTER if alt => Command::Properties,
        // Alt+D: "put the caret in the Address field". No menu
        // title answers to D, so the bar never wanted this one.
        s if alt && letter(s) == Some('d') => Command::AddressFocus,
        // The Menu key is Shift+F10; a bare F10 activates the menu
        // bar instead and never reaches here.
        MENU => Command::ContextMenu,
        keysym::F10 if shift => Command::ContextMenu,
        keysym::BACKSPACE => Command::Up,
        F5 => Command::Refresh,
        F2 => Command::Rename,
        keysym::DELETE | keysym::KP_DELETE => Command::Delete { erase: shift },
        _ if !ctrl => Command::Nothing,
        _ => match letter(sym) {
            Some('x') => Command::Cut,
            Some('c') => Command::Copy,
            Some('v') => Command::Paste,
            Some('z') => Command::Undo,
            Some('a') => Command::SelectAll,
            _ => Command::Nothing,
        },
    }
}

/// [`command_for_key`] with the focus taken into account: an editable
/// field owns the keys that edit text, so they must not also act on the
/// folder. The rename box owns *every* key: it swallows F2, Delete and
/// the rest until Enter or Escape closes it.
pub fn command_for_key_in(
    sym: u32,
    ctrl: bool,
    shift: bool,
    alt: bool,
    focus: Focus,
    renaming: bool,
) -> Command {
    if renaming {
        return Command::Nothing;
    }
    let cmd = command_for_key(sym, ctrl, shift, alt);
    if focus == Focus::Address && !address_allows(&cmd) {
        return Command::Nothing;
    }
    cmd
}

/// What the Address field lets past it. Everything else it either types
/// (a letter), edits with (Backspace, Delete, Ctrl+A/X/C/V/Z) or has no
/// business with while the caret is in it (F2's rename).
fn address_allows(cmd: &Command) -> bool {
    matches!(
        cmd,
        Command::Back
            | Command::Forward
            | Command::Refresh
            | Command::AddressFocus
            | Command::Properties
            | Command::Nothing
    )
}

impl Explorer {
    /// Act on `cmd`. Everything that changes the model goes through here
    /// so the menus, the toolbar and the keys cannot drift apart.
    pub fn run(&mut self, cmd: Command, ctx: &mut Ctx) -> Reply {
        match cmd {
            Command::Nothing => Reply::none(),
            Command::Close => self.close(),
            Command::About => self.about_box(ctx),
            Command::SystemInformation => Reply {
                spawn: vec![vec!["reprise-about".into()]],
                ..Default::default()
            },
            Command::Eject => self.eject_selected(ctx),
            Command::AddressFocus => {
                self.focus = Focus::Address;
                if let Some(edit) = self.address.combo.edit.as_mut() {
                    edit.select_all();
                }
                Reply::redraw()
            }
            Command::ContextMenu => self.context_menu_key(ctx),

            Command::Back => match self.history.back().cloned() {
                Some(p) => self.show(p, ctx),
                None => Reply::none(),
            },
            Command::Forward => match self.history.forward().cloned() {
                Some(p) => self.show(p, ctx),
                None => Reply::none(),
            },
            Command::Go(n) => match self.history.go(n).cloned() {
                Some(p) => self.show(p, ctx),
                None => Reply::none(),
            },
            Command::GoForward(n) => match self.history.go_forward(n).cloned() {
                Some(p) => self.show(p, ctx),
                None => Reply::none(),
            },
            Command::Up => match self.place.parent_in(&self.drive_mounts) {
                Some(p) => self.go(p, ctx),
                None => Reply::none(),
            },
            Command::GoTo(p) => self.go(p, ctx),
            Command::Navigate(text) => self.navigate(&text, |d| d.is_dir(), ctx),
            Command::Refresh => self.refresh(ctx),
            Command::Open => self.open_selected(ctx),
            Command::Explore => {
                if let Some(v) = self.selected_unmounted() {
                    return self.mount_and_open(v, ctx);
                }
                let dir = self.selected_dir();
                match dir {
                    Some(p) => self.go(p, ctx),
                    None => Reply::none(),
                }
            }

            Command::Mode(m) => {
                self.sv.set_mode(m);
                self.settings.mode = mode_name(m).to_string();
                self.settings_dirty = true;
                // Thumbnails and Tiles are the modes that show one, so
                // switching into either is when the decode is asked for.
                self.request_thumbs(ctx.icons.scale());
                Reply::redraw()
            }
            Command::Sort(col) => {
                let asc = match self.sv.sort {
                    Some((c, asc)) if c == col => !asc,
                    _ => true,
                };
                self.sv.sort = Some((col, asc));
                // The Recycle Bin's columns are its own; the folder sort
                // the user saved stays as it was.
                if self.folder_columns() {
                    self.settings.sort = (col, asc);
                    self.settings_dirty = true;
                }
                self.resort();
                Reply::redraw()
            }
            Command::ToggleFolders => {
                self.settings.folders = !self.settings.folders;
                self.settings_dirty = true;
                self.sync_bar();
                self.refresh_chrome(ctx);
                Reply::redraw()
            }
            Command::CloseBar => {
                self.settings.folders = false;
                self.settings_dirty = true;
                self.sync_bar();
                self.refresh_chrome(ctx);
                Reply::redraw()
            }
            Command::ToggleStatusBar => {
                self.settings.status_bar = !self.settings.status_bar;
                self.settings_dirty = true;
                self.refresh_chrome(ctx);
                Reply::redraw()
            }
            Command::ToggleToolbar(i) => {
                match i {
                    0 => self.settings.toolbars = !self.settings.toolbars,
                    1 => self.settings.address_bar = !self.settings.address_bar,
                    // Links is drawn greyed: nothing is behind it.
                    _ => return Reply::none(),
                }
                self.settings_dirty = true;
                self.refresh_chrome(ctx);
                Reply::redraw()
            }
            Command::ToggleNode => {
                let Some(path) = self.tree.selected.clone() else {
                    return Reply::none();
                };
                let Some(node) = self.tree.node_mut(&path) else {
                    return Reply::none();
                };
                if !node.has_children {
                    return Reply::none();
                }
                node.expanded = !node.expanded;
                if node.expanded && node.children.is_empty() {
                    self.fill_node(path, ctx);
                }
                Reply::redraw()
            }
            Command::OpenNode => match self.tree.selected.clone().and_then(|p| self.place_of(&p)) {
                Some(place) => self.go(place, ctx),
                None => Reply::none(),
            },
            Command::NodeCut | Command::NodeCopy => match self.node_folder() {
                Some((path, _)) => self.clipboard_paths(cmd == Command::NodeCut, vec![path], ctx),
                None => Reply::none(),
            },
            Command::NodeDelete => match self.node_folder() {
                Some((path, name)) => {
                    let bins = reprisefs::trash::Bins::system();
                    let fate = bins
                        .as_ref()
                        .map_or(reprisefs::bincap::Fate::NoBin, |b| b.fate(&path));
                    let confirm = bins.as_ref().is_none_or(|b| b.policy.confirm);
                    let mode = reprisefs::bincap::delete_mode(confirm, &[fate], false);
                    if !mode.ask {
                        return self.delete_now(&[path], mode.erase, ctx);
                    }
                    let icon = crate::chrome_icon(ctx, &["user-trash-full"], 32);
                    self.confirm_box(
                        ConfirmKind::Folder,
                        &name,
                        vec![path],
                        mode.erase,
                        mode.no_bin,
                        icon,
                        false,
                        None,
                    )
                }
                None => Reply::none(),
            },
            Command::NodeProperties => match self.node_folder() {
                Some((path, name)) => self.open_properties(&path, &name, &name, ctx),
                None => Reply::none(),
            },
            Command::SelectAll => {
                let n = self.sv.entries.len();
                self.sv.selection.select_all(n);
                self.refresh_chrome(ctx);
                Reply::redraw()
            }
            Command::InvertSelection => {
                let n = self.sv.entries.len();
                self.sv.selection.invert(n);
                self.refresh_chrome(ctx);
                Reply::redraw()
            }
            Command::Rename => self.begin_rename(ctx),

            Command::Cut => self.clipboard(true, ctx),
            Command::Copy => self.clipboard(false, ctx),
            Command::Paste => self.begin_paste(false),
            Command::PasteShortcut => self.begin_paste(true),
            Command::PasteOnItem => self.begin_paste_on_item(),
            Command::Delete { erase } => self.confirm_delete(erase, ctx),
            Command::NewFolder => self.new_item(true, ctx),
            Command::EmptyRecycleBin => self.confirm_empty_bin(ctx),
            Command::Restore => self.restore(ctx),
            Command::NewTextDocument => self.new_item(false, ctx),
            Command::CreateShortcut => self.create_shortcut(ctx),
            Command::SendTo(i) => self.send_to(i, ctx),
            Command::MoveTo => self.open_browse(true, ctx),
            Command::CopyTo => self.open_browse(false, ctx),
            Command::Drop(op) => self.drop_pending(op, ctx),
            Command::DropCancel => self.cancel_pending_drop(),
            Command::OpenWith(i) => self.open_with(i),
            Command::ChooseProgram => self.choose_program(ctx),
            Command::Undo => self.undo_last(ctx),
            Command::Properties => self.properties(ctx),
        }
    }
}

pub fn next_mode(m: Mode) -> Mode {
    match m {
        Mode::Thumbnails => Mode::Tiles,
        Mode::Tiles => Mode::Icons,
        Mode::Icons => Mode::List,
        Mode::List => Mode::Details,
        Mode::Details => Mode::Thumbnails,
    }
}

pub fn mode_name(m: Mode) -> &'static str {
    match m {
        Mode::Thumbnails => "thumbnails",
        Mode::Tiles => "tiles",
        Mode::Icons => "icons",
        Mode::List => "list",
        Mode::Details => "details",
    }
}

pub fn mode_from_name(s: &str) -> Mode {
    match s {
        "thumbnails" => Mode::Thumbnails,
        "icons" => Mode::Icons,
        "list" => Mode::List,
        "details" => Mode::Details,
        _ => Mode::Tiles,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_views_button_cycles_the_five_modes() {
        let mut m = Mode::Thumbnails;
        let mut seen = vec![m];
        for _ in 0..4 {
            m = next_mode(m);
            seen.push(m);
        }
        assert_eq!(
            seen,
            [
                Mode::Thumbnails,
                Mode::Tiles,
                Mode::Icons,
                Mode::List,
                Mode::Details
            ]
        );
        assert_eq!(next_mode(Mode::Details), Mode::Thumbnails);
    }

    #[test]
    fn mode_names_round_trip() {
        for m in [
            Mode::Thumbnails,
            Mode::Tiles,
            Mode::Icons,
            Mode::List,
            Mode::Details,
        ] {
            assert_eq!(mode_from_name(mode_name(m)), m);
        }
        assert_eq!(mode_from_name("nonsense"), Mode::Tiles);
    }

    #[test]
    fn the_whole_accelerator_table() {
        let k = |sym, ctrl, shift, alt| command_for_key(sym, ctrl, shift, alt);
        assert_eq!(k(keysym::LEFT, false, false, true), Command::Back);
        assert_eq!(k(keysym::RIGHT, false, false, true), Command::Forward);
        assert_eq!(k(keysym::BACKSPACE, false, false, false), Command::Up);
        assert_eq!(k(F5, false, false, false), Command::Refresh);
        assert_eq!(k(F2, false, false, false), Command::Rename);
        assert_eq!(
            k(keysym::DELETE, false, false, false),
            Command::Delete { erase: false }
        );
        assert_eq!(
            k(keysym::DELETE, false, true, false),
            Command::Delete { erase: true }
        );
        assert_eq!(
            k(keysym::RETURN, false, false, true),
            Command::Properties,
            "Alt+Enter is Properties, a bare Enter opens"
        );
        assert_eq!(k(keysym::RETURN, false, false, false), Command::Nothing);
        assert_eq!(k('x' as u32, true, false, false), Command::Cut);
        assert_eq!(k('c' as u32, true, false, false), Command::Copy);
        assert_eq!(k('v' as u32, true, false, false), Command::Paste);
        assert_eq!(k('z' as u32, true, false, false), Command::Undo);
        assert_eq!(k('a' as u32, true, false, false), Command::SelectAll);
        for c in ['x', 'c', 'v', 'z', 'a'] {
            assert_eq!(k(c as u32, false, false, false), Command::Nothing);
        }
        assert_eq!(k('X' as u32, true, true, false), Command::Cut);
        assert_eq!(k('[' as u32, true, false, false), Command::Nothing);
        assert_eq!(k('e' as u32, true, false, false), Command::Nothing);
        assert_eq!(k('d' as u32, false, false, true), Command::AddressFocus);
        assert_eq!(k('D' as u32, false, false, true), Command::AddressFocus);
        assert_eq!(k('d' as u32, false, false, false), Command::Nothing);
        // The context menu: the Menu key, and Shift+F10. A bare F10
        // belongs to the menu bar and never reaches the table.
        assert_eq!(k(MENU, false, false, false), Command::ContextMenu);
        assert_eq!(k(keysym::F10, false, true, false), Command::ContextMenu);
        assert_eq!(k(keysym::F10, false, false, false), Command::Nothing);
        for sym in [keysym::LEFT, keysym::UP, keysym::TAB, keysym::ESCAPE] {
            assert_eq!(k(sym, false, false, false), Command::Nothing);
        }
    }

    #[test]
    fn an_editable_field_keeps_the_keys_that_are_its_own() {
        let in_view =
            |sym, ctrl, shift, alt| command_for_key_in(sym, ctrl, shift, alt, Focus::View, false);
        let in_tree =
            |sym, ctrl, shift, alt| command_for_key_in(sym, ctrl, shift, alt, Focus::Tree, false);
        let in_address = |sym, ctrl, shift, alt| {
            command_for_key_in(sym, ctrl, shift, alt, Focus::Address, false)
        };
        let renaming =
            |sym, ctrl, shift, alt| command_for_key_in(sym, ctrl, shift, alt, Focus::View, true);

        for f in [in_view, in_tree] {
            assert_eq!(f(F2, false, false, false), Command::Rename);
            assert_eq!(f(keysym::BACKSPACE, false, false, false), Command::Up);
            assert_eq!(
                f(keysym::DELETE, false, false, false),
                Command::Delete { erase: false }
            );
            assert_eq!(f('a' as u32, true, false, false), Command::SelectAll);
        }

        for (sym, ctrl) in [
            (keysym::BACKSPACE, false),
            (keysym::DELETE, false),
            (F2, false),
            ('a' as u32, true),
            ('x' as u32, true),
            ('c' as u32, true),
            ('v' as u32, true),
            ('z' as u32, true),
        ] {
            assert_eq!(in_address(sym, ctrl, false, false), Command::Nothing);
        }
        assert_eq!(in_address(keysym::LEFT, false, false, true), Command::Back);
        assert_eq!(
            in_address(keysym::RIGHT, false, false, true),
            Command::Forward
        );
        assert_eq!(in_address(F5, false, false, false), Command::Refresh);
        assert_eq!(
            in_address('d' as u32, false, false, true),
            Command::AddressFocus
        );

        for (sym, ctrl, alt) in [
            (F2, false, false),
            (F5, false, false),
            (keysym::DELETE, false, false),
            (keysym::BACKSPACE, false, false),
            ('a' as u32, true, false),
            ('d' as u32, false, true),
            (MENU, false, false),
        ] {
            assert_eq!(renaming(sym, ctrl, false, alt), Command::Nothing);
        }
    }
}
