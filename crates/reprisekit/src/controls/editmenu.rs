//! The right-click menu of a text box: Undo, Cut, Copy, Paste, Delete and Select All, greyed
//! like an Edit menu. A host opens [`tree`] as a [`PopupMenu`] and maps the chosen path back
//! through [`command`], or holds a whole open [`Menu`] and runs a pick as its [`shortcut`].

use super::menu::{MenuAction, MenuCtx, MenuItem, MenuKey, MenuTree, PopupMenu};
use crate::access::Tree;
use crate::raster::{Canvas, Rect};
use crate::text::TextRenderer;
use crate::textedit::{TextEdit, TextSpot, keysym};
use crate::theme::Theme;
use accesskit::NodeId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Undo,
    Cut,
    Copy,
    Paste,
    Delete,
    SelectAll,
}

/// What the text box can do right now.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Live {
    pub undo: bool,
    pub selection: bool,
    /// The clipboard holds text.
    pub paste: bool,
    /// A password field: Cut and Copy stay greyed over a selection.
    pub secret: bool,
}

impl Live {
    /// What the focused field `spot` can do; `paste` says whether the clipboard holds text.
    pub fn of(spot: &TextSpot, paste: bool) -> Live {
        Live {
            undo: spot.can_undo,
            selection: spot.selected,
            paste,
            secret: spot.secret,
        }
    }

    /// What `edit` can do; `paste` says whether the clipboard holds text.
    pub fn of_edit(edit: &TextEdit, paste: bool) -> Live {
        Live {
            undo: edit.can_undo(),
            selection: edit.has_selection(),
            paste,
            secret: edit.is_secret(),
        }
    }
}

/// The rows top to bottom, `None` a separator.
const ROWS: [Option<(&str, Command)>; 8] = [
    Some(("&Undo", Command::Undo)),
    None,
    Some(("Cu&t", Command::Cut)),
    Some(("&Copy", Command::Copy)),
    Some(("&Paste", Command::Paste)),
    Some(("&Delete", Command::Delete)),
    None,
    Some(("Select &All", Command::SelectAll)),
];

pub fn tree(live: Live) -> MenuTree {
    MenuTree::flat(items(live))
}

/// [`tree`]'s rows, for a host that opens its own menus.
pub fn items(live: Live) -> Vec<MenuItem> {
    ROWS.iter()
        .map(|row| match row {
            None => MenuItem::Separator,
            Some((label, cmd)) => MenuItem::item(label).enabled(match cmd {
                Command::Undo => live.undo,
                Command::Cut | Command::Copy => live.selection && !live.secret,
                Command::Delete => live.selection,
                Command::Paste => live.paste,
                Command::SelectAll => true,
            }),
        })
        .collect()
}

/// The command a chosen path (`MenuAction::Chosen`) names.
pub fn command(path: &[usize]) -> Option<Command> {
    match path {
        [i] => ROWS.get(*i).copied().flatten().map(|(_, cmd)| cmd),
        _ => None,
    }
}

/// Whether a key press opens the menu of the focused field: the Menu key or Shift+F10.
pub fn opens(sym: u32, shift: bool) -> bool {
    sym == keysym::MENU || (sym == keysym::F10 && shift)
}

/// Where the keyboard opens the menu: under the caret, kept inside the field.
pub fn under_caret(spot: &TextSpot) -> (i32, i32) {
    let f = spot.field;
    (
        spot.caret.x.clamp(f.x, f.right()),
        spot.caret.bottom().clamp(f.y, f.bottom()),
    )
}

/// The key press a command stands for, as (keysym, Ctrl held): a host runs a pick through the
/// field's own handling of that shortcut.
pub fn shortcut(cmd: Command) -> (u32, bool) {
    match cmd {
        Command::Undo => (0x7a, true),
        Command::Cut => (0x78, true),
        Command::Copy => (0x63, true),
        Command::Paste => (0x76, true),
        Command::Delete => (keysym::DELETE, false),
        Command::SelectAll => (0x61, true),
    }
}

/// The menu while it is open over a field.
pub struct Menu {
    tree: MenuTree,
    chain: PopupMenu,
}

/// What a key, press or pick did to an open [`Menu`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    None,
    Redraw,
    /// It closed without a pick.
    Close,
    /// It closed on this pick.
    Run(Command),
}

impl Menu {
    /// Opens with its top-left at `at`; from the keyboard its first row starts picked.
    pub fn open(live: Live, at: (i32, i32), cx: &MenuCtx, by_keyboard: bool) -> Menu {
        let tree = tree(live);
        let chain = PopupMenu::open(&tree, at, cx, by_keyboard);
        Menu { tree, chain }
    }

    pub fn tree(&self) -> &MenuTree {
        &self.tree
    }

    pub fn rect(&self) -> Rect {
        self.chain.rect()
    }

    /// The chain, for a host that shows each level on a surface of its own.
    pub fn chain(&self) -> &PopupMenu {
        &self.chain
    }

    /// Moves level `i` to where the host's compositor put it ([`PopupMenu::place`]).
    pub fn place(&mut self, i: usize, origin: (i32, i32)) {
        self.chain.place(i, origin);
    }

    /// Every key while the menu is open: the arrows, Enter, Escape and access letters work it,
    /// the rest do nothing.
    pub fn key(&mut self, sym: u32, utf8: Option<&str>, cx: &MenuCtx) -> Step {
        let key = match sym {
            keysym::ESCAPE => MenuKey::Escape,
            keysym::UP | keysym::KP_UP => MenuKey::Up,
            keysym::DOWN | keysym::KP_DOWN => MenuKey::Down,
            keysym::LEFT | keysym::KP_LEFT => MenuKey::Left,
            keysym::RIGHT | keysym::KP_RIGHT => MenuKey::Right,
            keysym::RETURN | keysym::KP_ENTER => MenuKey::Enter,
            _ => match utf8.and_then(|s| s.chars().next()) {
                Some(c) if c.is_alphanumeric() => MenuKey::Char(c),
                _ => return Step::None,
            },
        };
        let act = self.chain.key(&self.tree, key, cx);
        step(act)
    }

    /// A press anywhere: on a live row it picks it, off the menu it closes it.
    pub fn press(&mut self, x: i32, y: i32, cx: &MenuCtx) -> Step {
        let act = self.chain.click(&self.tree, x, y, cx);
        step(act)
    }

    /// `true` when the lit row changed.
    pub fn motion(&mut self, x: i32, y: i32, cx: &MenuCtx) -> bool {
        self.chain.motion(&self.tree, x, y, cx)
    }

    pub fn draw(&self, c: &mut Canvas, theme: &Theme, text: &TextRenderer, underline: bool) {
        self.chain.draw(c, theme, text, underline);
    }

    /// The menu's nodes under `parent`; the returned node takes the focus.
    pub fn access(&self, tree: &mut Tree, parent: NodeId) -> NodeId {
        self.chain.access(tree, parent, "editmenu")
    }
}

fn step(act: MenuAction) -> Step {
    match act {
        MenuAction::Chosen(path) => command(&path).map_or(Step::Close, Step::Run),
        MenuAction::Close => Step::Close,
        MenuAction::Redraw => Step::Redraw,
        MenuAction::None => Step::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enabled(t: &MenuTree) -> Vec<Option<bool>> {
        t.iter()
            .map(|item| match item {
                MenuItem::Entry { enabled, .. } => Some(*enabled),
                MenuItem::Separator => None,
            })
            .collect()
    }

    #[test]
    fn rows_grey_with_nothing_to_act_on_but_select_all() {
        let none = tree(Live::default());
        assert_eq!(
            enabled(&none),
            [
                Some(false),
                None,
                Some(false),
                Some(false),
                Some(false),
                Some(false),
                None,
                Some(true)
            ]
        );
        let all = tree(Live {
            undo: true,
            selection: true,
            paste: true,
            secret: false,
        });
        assert!(enabled(&all).iter().flatten().all(|on| *on));
        let paste = tree(Live {
            paste: true,
            ..Live::default()
        });
        assert_eq!(
            enabled(&paste)[4],
            Some(true),
            "Paste follows the clipboard"
        );
        assert_eq!(enabled(&paste)[3], Some(false), "Copy needs a selection");
    }

    #[test]
    fn a_chosen_row_names_its_command_and_a_separator_none() {
        let t = tree(Live::default());
        let labels: Vec<_> = t
            .iter()
            .filter_map(|item| match item {
                MenuItem::Entry { label, .. } => Some(label.as_str()),
                MenuItem::Separator => None,
            })
            .collect();
        assert_eq!(
            labels,
            ["Undo", "Cut", "Copy", "Paste", "Delete", "Select All"]
        );
        assert_eq!(command(&[0]), Some(Command::Undo));
        assert_eq!(command(&[1]), None);
        assert_eq!(command(&[4]), Some(Command::Paste));
        assert_eq!(command(&[7]), Some(Command::SelectAll));
        assert_eq!(command(&[8]), None);
        assert_eq!(command(&[4, 0]), None);
    }

    #[test]
    fn a_password_greys_cut_and_copy_but_not_delete_or_paste() {
        let mut pw = TextEdit::secret();
        pw.paste("pw", &[]);
        pw.select_all();
        let r = Rect::new(10, 10, 100, 20);
        let live = Live::of(&pw.spot(r, r), true);
        assert_eq!(
            live,
            Live::of_edit(&pw, true),
            "the spot says what the edit does"
        );
        assert_eq!(
            enabled(&tree(live)),
            [
                Some(false),
                None,
                Some(false),
                Some(false),
                Some(true),
                Some(true),
                None,
                Some(true)
            ]
        );
    }

    #[test]
    fn the_menu_key_and_shift_f10_open_it_under_the_caret() {
        assert!(opens(keysym::MENU, false));
        assert!(opens(keysym::F10, true));
        assert!(!opens(keysym::F10, false), "F10 alone is the menu bar's");
        let mut spot =
            TextEdit::new("x", false).spot(Rect::new(10, 10, 100, 20), Rect::new(30, 12, 1, 16));
        assert_eq!(under_caret(&spot), (30, 28));
        spot.caret.x = 200;
        assert_eq!(under_caret(&spot), (110, 28), "kept inside the field");
    }
}
