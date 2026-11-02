//! The list's context menu: an entry's (Select, Pin to Places, Delete, Rename) or the
//! background's (View, Refresh, New), and a pinned places tile's (Open, Unpin, Move Up/Down). Laid out with the dialog like the Views menu, so its keys need no text renderer; one
//! submenu level.

use super::{Event, FileDlg, Focus, Hit, views_items};
use crate::controls::form;
use crate::controls::menu::{
    ContextBitmaps, KeyStops, MenuItem, MenuLayout, MenuStyle, enter_row, place_submenu,
    rows_for_key, step_selection,
};
use crate::controls::shellview::{self, Mode};
use crate::raster::Rect;
use crate::text::TextRenderer;
use crate::theme::{TextUnits, Theme};

/// What a row does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Cmd {
    Select,
    Delete,
    Rename,
    Refresh,
    NewFolder,
    View(Mode),
    Pin(bool),
    OpenPlace,
    Unpin,
    Shift(bool),
}

/// A row: its item, and what it does or the rows of the submenu it opens.
enum Row {
    Cmd(MenuItem, Cmd),
    Sub(MenuItem, Vec<(MenuItem, Cmd)>),
    Separator,
}

impl Row {
    fn item(&self) -> MenuItem {
        match self {
            Row::Cmd(item, _) | Row::Sub(item, _) => item.clone(),
            Row::Separator => MenuItem::Separator,
        }
    }
}

/// The open menu.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Context {
    /// The selected entries' menu, else the background's.
    on_item: bool,
    /// A pinned places tile's menu, which wins over `on_item`.
    pub(super) place: Option<usize>,
    /// Where it was asked for: its top-left corner, slid inside the client.
    pub(super) at: (i32, i32),
    /// Where a host showing the levels on popups put each ([`super::Popped`]).
    pub(super) placed: [Option<(i32, i32)>; 2],
    /// The root row whose submenu is open.
    pub(super) sub: Option<usize>,
    /// The lit row of each level.
    pub(super) hot: [Option<usize>; 2],
    /// Opened from the keyboard: its access keys underline.
    pub(super) keys: bool,
}

/// One level as the dialog lays it out: its rect in the client and its rows.
pub(super) struct Level {
    pub(super) rect: Rect,
    pub(super) layout: MenuLayout,
    pub(super) items: Vec<MenuItem>,
    pub(super) hot: Option<usize>,
}

fn opens_sub(items: &[MenuItem], row: usize) -> bool {
    matches!(
        items.get(row),
        Some(MenuItem::Entry {
            submenu: true,
            enabled: true,
            ..
        })
    )
}

fn label(key: &str) -> String {
    match key {
        "select" => tr!("filedlg-menu-select"),
        "delete" => tr!("filedlg-menu-delete"),
        "rename" => tr!("filedlg-menu-rename"),
        "view" => tr!("filedlg-menu-view"),
        "refresh" => tr!("filedlg-menu-refresh"),
        "new" => tr!("filedlg-menu-new"),
        "open" => tr!("filedlg-menu-open"),
        "pin" => tr!("filedlg-menu-pin"),
        "unpin" => tr!("filedlg-menu-unpin"),
        "up" => tr!("filedlg-menu-move-up"),
        "down" => tr!("filedlg-menu-move-down"),
        _ => tr!("filedlg-menu-new-folder"),
    }
}

impl FileDlg {
    fn context_rows(&self, c: &Context) -> Vec<Row> {
        if let Some(i) = c.place {
            let pinned = |i: usize| self.places.places.get(i).is_some_and(|p| p.pinned);
            let open = self.places.places.get(i).is_some_and(|p| p.enabled);
            return vec![
                Row::Cmd(
                    MenuItem::item(&label("open")).bold(true).enabled(open),
                    Cmd::OpenPlace,
                ),
                Row::Separator,
                Row::Cmd(MenuItem::item(&label("unpin")), Cmd::Unpin),
                Row::Separator,
                Row::Cmd(
                    MenuItem::item(&label("up")).enabled(i > 0 && pinned(i - 1)),
                    Cmd::Shift(true),
                ),
                Row::Cmd(
                    MenuItem::item(&label("down")).enabled(pinned(i + 1)),
                    Cmd::Shift(false),
                ),
            ];
        }
        if c.on_item {
            let n = self.shell.selection.len();
            let mut rows = vec![
                Row::Cmd(MenuItem::item(&label("select")).bold(true), Cmd::Select),
                Row::Separator,
            ];
            if let Some(pinned) = self.pinnable() {
                let (key, cmd) = if pinned {
                    ("unpin", Cmd::Pin(false))
                } else {
                    ("pin", Cmd::Pin(true))
                };
                rows.extend([Row::Cmd(MenuItem::item(&label(key)), cmd), Row::Separator]);
            }
            rows.extend([
                Row::Cmd(MenuItem::item(&label("delete")).enabled(n > 0), Cmd::Delete),
                Row::Cmd(
                    MenuItem::item(&label("rename")).enabled(n == 1),
                    Cmd::Rename,
                ),
            ]);
            return rows;
        }
        let views = views_items()
            .into_iter()
            .map(|(name, mode)| {
                let item = MenuItem::entry(name)
                    .radio(true)
                    .checked(mode == self.shell.mode);
                (item, Cmd::View(mode))
            })
            .collect();
        vec![
            Row::Sub(MenuItem::item(&label("view")).submenu(true), views),
            Row::Separator,
            Row::Cmd(MenuItem::item(&label("refresh")), Cmd::Refresh),
            Row::Separator,
            Row::Sub(
                MenuItem::item(&label("new")).submenu(true),
                vec![(MenuItem::item(&label("folder")), Cmd::NewFolder)],
            ),
        ]
    }

    /// Each open level's rows, root first.
    pub(super) fn context_items(&self, c: &Context) -> Vec<(Vec<MenuItem>, Vec<Option<Cmd>>)> {
        let rows = self.context_rows(c);
        let mut levels = vec![(
            rows.iter().map(Row::item).collect(),
            rows.iter()
                .map(|r| match r {
                    Row::Cmd(_, cmd) => Some(*cmd),
                    _ => None,
                })
                .collect(),
        )];
        if let Some(Row::Sub(_, sub)) = c.sub.and_then(|i| rows.get(i)) {
            levels.push((
                sub.iter().map(|(item, _)| item.clone()).collect(),
                sub.iter().map(|(_, cmd)| Some(*cmd)).collect(),
            ));
        }
        levels
    }

    /// Whether the one selected entry is a folder the menu can pin, and if so whether it is
    /// pinned; None for a file, a shortcut or several entries.
    fn pinnable(&self) -> Option<bool> {
        let i = self.shell.selection.single()?;
        let e = self.shell.entries.get(i).filter(|e| e.is_dir && !e.link)?;
        Some(self.pinned_entries.contains(&e.name))
    }

    /// Opens pinned tile `i`'s menu with its corner at `at`.
    pub(super) fn open_place_context(&mut self, i: usize, at: (i32, i32), keys: bool) {
        self.open_context(false, at, keys);
        if let Some(c) = &mut self.context {
            c.place = Some(i);
            c.hot[0] = if keys { Some(0) } else { None };
        }
    }

    /// Opens the entries' menu (`on_item`) or the background's with its corner at `at`.
    pub(super) fn open_context(&mut self, on_item: bool, at: (i32, i32), keys: bool) {
        self.lookin.open = None;
        self.filetype.open = None;
        self.views_open = false;
        let mut c = Context {
            on_item,
            place: None,
            at,
            placed: [None; 2],
            sub: None,
            hot: [None; 2],
            keys,
        };
        if keys {
            let items = &self.context_items(&c)[0].0;
            c.hot[0] = step_selection(items, None, 1, KeyStops::Enabled);
        }
        self.context = Some(c);
    }

    /// The open levels laid out in client `r` with text grown by `u`, root first.
    pub(super) fn context_levels(
        &self,
        text: &TextRenderer,
        px: u32,
        r: Rect,
        u: TextUnits,
    ) -> Vec<Level> {
        let Some(c) = &self.context else {
            return Vec::new();
        };
        let mut theme = Theme::default();
        theme.fonts.size_px = u.px(theme.fonts.size_px);
        let mut out: Vec<Level> = Vec::new();
        for (depth, (items, _)) in self.context_items(c).into_iter().enumerate() {
            let style =
                MenuStyle::context(&theme, ContextBitmaps::None).fit(text, &items, px, r.width);
            let layout = MenuLayout::compute_styled(&style, &items, usize::MAX);
            let size = (layout.width, layout.height);
            let corner = match (c.placed[depth], out.last()) {
                (Some(at), _) => at,
                (None, None) => (
                    c.at.0.min(r.right() - size.0).max(r.x),
                    c.at.1.min(r.bottom() - size.1).max(r.y),
                ),
                (None, Some(parent)) => {
                    let row = parent.layout.rows[c.sub.unwrap_or(0)];
                    let p = parent.rect;
                    let row = Rect::new(p.x + row.x, p.y + row.y, row.width, row.height);
                    place_submenu(p, row, size, r)
                }
            };
            out.push(Level {
                rect: Rect::new(corner.0, corner.1, size.0, size.1),
                layout,
                items,
                hot: c.hot[depth],
            });
        }
        out
    }

    /// The menu's part at (`x`, `y`), deepest level first: `Some(None)` on a level's border or a
    /// separator.
    pub(super) fn context_hit(&self, levels: &[Level], x: i32, y: i32) -> Option<Hit> {
        levels.iter().enumerate().rev().find_map(|(level, l)| {
            l.rect.contains(x, y).then(|| Hit::Menu {
                level,
                row: l.layout.hit(x - l.rect.x, y - l.rect.y),
            })
        })
    }

    /// A press while the menu is open: a row picks, its border does nothing, anywhere else closes
    /// the menu and goes no further.
    pub(super) fn context_click(&mut self, hit: Option<Hit>, shift: bool) -> Event {
        match hit {
            Some(Hit::Menu {
                level,
                row: Some(row),
            }) => self.context_pick(level, row, shift),
            Some(Hit::Menu { row: None, .. }) => Event::None,
            _ => {
                self.context = None;
                Event::Redraw
            }
        }
    }

    /// The pointer over the menu lights the row under it; resting on a submenu's row opens it,
    /// and on another row of the root closes it again.
    pub(super) fn context_hover(&mut self, hit: Option<Hit>) -> Event {
        let (Some(Hit::Menu { level, row }), Some(c)) = (hit, &self.context) else {
            return Event::None;
        };
        let root = self.context_items(c).swap_remove(0).0;
        let mut next = c.clone();
        next.hot[level] = row;
        if level == 0 {
            let opens = row.filter(|&r| opens_sub(&root, r));
            if next.sub != opens {
                next.sub = opens;
                next.hot[1] = None;
                next.placed[1] = None;
            }
        }
        if self.context.as_ref() == Some(&next) {
            return Event::None;
        }
        self.context = Some(next);
        Event::Redraw
    }

    /// A key while the menu is open: the arrows walk it, Enter or a row's access key picks,
    /// Escape closes the submenu, then the menu. It takes every key.
    pub(super) fn context_key(&mut self, key: form::Key) -> Event {
        let Some(c) = self.context.clone() else {
            return Event::None;
        };
        let levels = self.context_items(&c);
        let depth = levels.len() - 1;
        let items = &levels[depth].0;
        let key = match key {
            form::Key::Edit(ek) => form::flat_key(&ek).unwrap_or(form::Key::Edit(ek)),
            k => k,
        };
        let set = |dlg: &mut FileDlg, f: &dyn Fn(&mut Context)| {
            if let Some(c) = dlg.context.as_mut() {
                f(c);
            }
            Event::Redraw
        };
        match key {
            form::Key::Up | form::Key::Down => {
                let step = if matches!(key, form::Key::Down) {
                    1
                } else {
                    -1
                };
                let hot = step_selection(items, c.hot[depth], step, KeyStops::Enabled);
                set(self, &|c| c.hot[depth] = hot)
            }
            form::Key::Right if depth == 0 => match c.hot[0] {
                Some(r) if opens_sub(items, r) => self.context_pick(0, r, false),
                _ => Event::None,
            },
            form::Key::Left | form::Key::Escape if depth == 1 => set(self, &|c| {
                c.sub = None;
                c.hot[1] = None;
                c.placed[1] = None;
            }),
            form::Key::Escape => {
                self.context = None;
                Event::Redraw
            }
            form::Key::Enter | form::Key::Space => match enter_row(items, c.hot[depth]) {
                Some(r) => self.context_pick(depth, r, false),
                None => Event::None,
            },
            form::Key::Char(ch) | form::Key::Access(ch) => match rows_for_key(items, ch).next() {
                Some(r) => self.context_pick(depth, r, false),
                None => Event::None,
            },
            _ => Event::None,
        }
    }

    /// Row `row` of level `level` picked: a submenu's row opens it, lighting its first row, a
    /// greyed row or a separator does nothing, and any other closes the menu and does its work.
    /// Delete clicked with Shift held deletes for good.
    fn context_pick(&mut self, level: usize, row: usize, shift: bool) -> Event {
        let Some(c) = self.context.clone() else {
            return Event::None;
        };
        let levels = self.context_items(&c);
        let Some((items, cmds)) = levels.get(level) else {
            return Event::None;
        };
        if !items.get(row).is_some_and(MenuItem::selectable) {
            return Event::None;
        }
        if let Some(cmd) = cmds.get(row).copied().flatten() {
            self.context = None;
            if let Some(tile) = c.place {
                return self.run_place_cmd(tile, cmd);
            }
            return self.run_cmd(cmd, shift);
        }
        if level != 0 || !opens_sub(items, row) {
            return Event::None;
        }
        let mut next = c;
        next.hot[0] = Some(row);
        if next.sub != Some(row) {
            next.sub = Some(row);
            next.placed[1] = None;
        }
        let sub = self.context_items(&next).swap_remove(1).0;
        next.hot[1] = step_selection(&sub, None, 1, KeyStops::Enabled);
        self.context = Some(next);
        Event::Redraw
    }

    fn run_place_cmd(&mut self, tile: usize, cmd: Cmd) -> Event {
        match cmd {
            Cmd::OpenPlace => {
                self.focus = Focus::Places;
                self.places.selected = Some(tile);
                Event::NeedPlace(tile)
            }
            Cmd::Unpin => Event::UnpinPlace(tile),
            Cmd::Shift(up) => Event::ShiftPlace { tile, up },
            _ => Event::None,
        }
    }

    fn run_cmd(&mut self, cmd: Cmd, shift: bool) -> Event {
        match cmd {
            Cmd::Select => {
                self.focus = Focus::Shell;
                self.activate(false)
            }
            Cmd::Delete => self.delete_selected(shift),
            Cmd::Rename => self.begin_rename(),
            Cmd::Refresh => Event::Refresh,
            Cmd::NewFolder => Event::CreateFolder(tr!("filedlg-new-folder")),
            Cmd::View(mode) => {
                self.shell.set_mode(mode);
                Event::Redraw
            }
            Cmd::Pin(pin) => match self
                .shell
                .selection
                .single()
                .and_then(|i| self.shell.entries.get(i))
            {
                Some(e) => Event::PinEntry {
                    name: e.name.clone(),
                    pin,
                },
                None => Event::None,
            },
            Cmd::OpenPlace | Cmd::Unpin | Cmd::Shift(_) => Event::None,
        }
    }

    /// Shift+F10 or the Menu key in the list: the entries' menu under the focused entry when
    /// it is selected, else the background's at the list's corner.
    pub(super) fn context_from_keys(&mut self, r: Rect) -> Event {
        let cm = self.drawn.0.get();
        let lay = self.shell_layout(r, &cm);
        let focused = self
            .shell
            .selection
            .focus
            .filter(|&i| self.shell.selection.is(i));
        let (on_item, at) = match focused {
            Some(i) => {
                shellview::ensure_visible(&mut self.shell, i, &lay);
                let lay = self.shell_layout(r, &cm);
                let cell = lay.cells.get(i).copied().unwrap_or(lay.body);
                (true, (cell.x + 8, cell.bottom()))
            }
            None => (false, (lay.body.x + 8, lay.body.y + 8)),
        };
        self.open_context(on_item, at, true);
        Event::Redraw
    }
}
