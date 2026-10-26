//! [`MenuTree`] and the open chain of [`PopupMenu`] levels: pointer, keys and placement.

use super::draw::draw_menu_at;
use super::layout::{BORDER, INSET, MenuLayout, MenuStyle, place_submenu};
use super::shadow::menu_look;
use super::{KeyStops, MenuItem, enter_row, rows_for_key, step_selection};
use crate::controls::hosted::{Anchor, Chain, Spot};
use crate::raster::{Canvas, Rect};
use crate::text::TextRenderer;
use crate::theme::Theme;

/// A menu whose rows can open menus of their own: `children[i]` is the submenu row `i` opens.
/// Build it through [`MenuTree::push`] and [`MenuTree::push_sub`] so the two vectors stay the same length.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MenuTree {
    items: Vec<MenuItem>,
    children: Vec<Option<MenuTree>>,
}

impl MenuTree {
    pub fn flat(items: Vec<MenuItem>) -> MenuTree {
        let children = items.iter().map(|_| None).collect();
        MenuTree { items, children }
    }

    pub fn new() -> MenuTree {
        MenuTree::default()
    }

    pub fn push(&mut self, item: MenuItem) {
        self.items.push(item);
        self.children.push(None);
    }

    /// A row that opens `child`; the row gets the arrow whether or not
    /// the caller asked for it.
    pub fn push_sub(&mut self, mut item: MenuItem, child: MenuTree) {
        if let MenuItem::Entry { submenu, .. } = &mut item {
            *submenu = true;
        }
        self.items.push(item);
        self.children.push(Some(child));
    }

    pub fn items(&self) -> &[MenuItem] {
        &self.items
    }

    pub fn child(&self, row: usize) -> Option<&MenuTree> {
        self.children.get(row)?.as_ref()
    }

    pub fn at(&self, path: &[usize]) -> Option<&MenuTree> {
        match path.split_first() {
            None => Some(self),
            Some((&row, rest)) => self.child(row)?.at(rest),
        }
    }
}

impl From<Vec<MenuItem>> for MenuTree {
    fn from(items: Vec<MenuItem>) -> MenuTree {
        MenuTree::flat(items)
    }
}

/// Reading and editing the rows is what a flat menu did; the slice cannot
/// resize, so the children stay paired with them.
impl std::ops::Deref for MenuTree {
    type Target = [MenuItem];

    fn deref(&self) -> &[MenuItem] {
        &self.items
    }
}

impl std::ops::DerefMut for MenuTree {
    fn deref_mut(&mut self) -> &mut [MenuItem] {
        &mut self.items
    }
}

#[derive(Clone, Copy)]
pub struct MenuCtx<'a> {
    pub text: &'a TextRenderer,
    pub theme: &'a Theme,
    pub px: u32,
    pub within: Rect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuKey {
    Up,
    Down,
    Left,
    Right,
    Enter,
    Escape,
    Char(char),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuAction {
    /// Not the menu's: the host decides (Left on the root menu of a menu
    /// bar walks to the previous title, say).
    None,
    Redraw,
    Chosen(Vec<usize>),
    Close,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Level {
    pub path: Vec<usize>,
    pub items: Vec<MenuItem>,
    pub origin: (i32, i32),
    pub layout: MenuLayout,
    pub hovered: Option<usize>,
}

impl Level {
    pub fn rect(&self) -> Rect {
        Rect::new(
            self.origin.0,
            self.origin.1,
            self.layout.width,
            self.layout.height,
        )
    }

    /// `Some(row)` over an entry, `Some(None)` over a separator or the
    /// border, `None` off the menu.
    fn hit(&self, x: i32, y: i32) -> Option<Option<usize>> {
        if !self.rect().contains(x, y) {
            return None;
        }
        Some(self.layout.hit(x - self.origin.0, y - self.origin.1))
    }

    fn opens_submenu(&self, row: usize) -> bool {
        matches!(
            self.items.get(row),
            Some(MenuItem::Entry {
                submenu: true,
                enabled: true,
                ..
            })
        )
    }
}

///
/// The host owns the submenu hover delay: [`PopupMenu::motion`] opens a
/// hovered row's submenu at once, so a host that wants the delay calls it
/// from its own timer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PopupMenu {
    pub(super) levels: Vec<Level>,
    /// Opened from the keyboard: the chain underlines its access keys
    /// whatever its host's cues say.
    pub(crate) keys: bool,
    /// What the root opens off: the point it was asked at, before the clamp into `within`.
    pub(crate) anchor: Anchor,
    /// Shown on popups by a host ([`Hosted`]): [`PopupMenu::draw`] leaves it out.
    hosted: bool,
}

/// A level taller than `within` wraps into columns, so every row stays on screen.
fn level_layout(items: &[MenuItem], cx: &MenuCtx) -> MenuLayout {
    let style = MenuStyle::context_for(cx.theme, items).fit(cx.text, items, cx.px, cx.within.width);
    let usable = cx.within.height - style.inset.top - style.inset.bottom;
    let tallest = style.row_height.max(style.separator_h).max(1);
    let max_rows = (usable / tallest).max(1) as usize;
    MenuLayout::compute_styled(&style, items, max_rows)
}

impl PopupMenu {
    pub fn open(tree: &MenuTree, at: (i32, i32), cx: &MenuCtx, by_keyboard: bool) -> PopupMenu {
        let items = tree.items.clone();
        let layout = level_layout(&items, cx);
        let x = at.0.min(cx.within.right() - layout.width).max(cx.within.x);
        let y =
            at.1.min(cx.within.bottom() - layout.height)
                .max(cx.within.y);
        let hovered = by_keyboard
            .then(|| step_selection(&items, None, 1, KeyStops::of(cx.theme)))
            .flatten();
        PopupMenu {
            levels: vec![Level {
                path: Vec::new(),
                items,
                origin: (x, y),
                layout,
                hovered,
            }],
            keys: by_keyboard,
            anchor: Anchor::Point(at.0, at.1),
            hosted: false,
        }
    }

    pub fn levels(&self) -> &[Level] {
        &self.levels
    }

    pub fn depth(&self) -> usize {
        self.levels.len()
    }

    pub fn rect(&self) -> Rect {
        self.levels[0].rect()
    }

    pub fn hovered(&self) -> Option<usize> {
        self.levels.last().and_then(|l| l.hovered)
    }

    /// The level and row under a point, deepest menu first (a child
    /// overlaps its parent). `Some((level, None))` is the border or a
    /// separator; `None` is off the chain entirely.
    pub fn hit(&self, x: i32, y: i32) -> Option<(usize, Option<usize>)> {
        self.levels
            .iter()
            .enumerate()
            .rev()
            .find_map(|(i, l)| l.hit(x, y).map(|row| (i, row)))
    }

    pub fn contains(&self, x: i32, y: i32) -> bool {
        self.hit(x, y).is_some()
    }

    fn open_child(
        &mut self,
        tree: &MenuTree,
        level: usize,
        row: usize,
        cx: &MenuCtx,
        by_keyboard: bool,
    ) -> bool {
        let Some(parent) = self.levels.get(level) else {
            return false;
        };
        let mut path = parent.path.clone();
        path.push(row);
        let (Some(child), Some(r)) = (tree.at(&path), parent.layout.rows.get(row).copied()) else {
            return false;
        };
        let items = child.items.clone();
        let layout = level_layout(&items, cx);
        let prect = parent.rect();
        let row_abs = Rect::new(prect.x + r.x, prect.y + r.y, r.width, r.height);
        let (x, y) = place_submenu(prect, row_abs, (layout.width, layout.height), cx.within);
        let hovered = by_keyboard
            .then(|| step_selection(&items, None, 1, KeyStops::of(cx.theme)))
            .flatten();
        self.levels.truncate(level + 1);
        self.levels.push(Level {
            path,
            items,
            origin: (x, y),
            layout,
            hovered,
        });
        true
    }

    fn child_open_for(&self, level: usize, row: Option<usize>) -> bool {
        match (row, self.levels.get(level + 1)) {
            (Some(r), Some(child)) => child.path.last() == Some(&r),
            _ => false,
        }
    }

    /// Hovering a submenu row opens it (see the type's doc on the delay),
    /// and leaving it for another row of the same menu closes it again.
    pub fn motion(&mut self, tree: &MenuTree, x: i32, y: i32, cx: &MenuCtx) -> bool {
        let Some((level, row)) = self.hit(x, y) else {
            return false;
        };
        let mut dirty = false;
        if self.levels[level].hovered != row {
            self.levels[level].hovered = row;
            dirty = true;
        }
        if self.child_open_for(level, row) {
            return dirty;
        }
        if self.levels.len() > level + 1 {
            self.levels.truncate(level + 1);
            dirty = true;
        }
        if let Some(r) = row
            && self.levels[level].opens_submenu(r)
        {
            dirty |= self.open_child(tree, level, r, cx, false);
        }
        dirty
    }

    pub fn key(&mut self, tree: &MenuTree, key: MenuKey, cx: &MenuCtx) -> MenuAction {
        let last = self.levels.len() - 1;
        match key {
            MenuKey::Up | MenuKey::Down => {
                let step = if key == MenuKey::Down { 1 } else { -1 };
                let level = &mut self.levels[last];
                level.hovered =
                    step_selection(&level.items, level.hovered, step, KeyStops::of(cx.theme));
                MenuAction::Redraw
            }
            MenuKey::Right => match self.levels[last].hovered {
                Some(r) if self.levels[last].opens_submenu(r) => {
                    self.open_child(tree, last, r, cx, true);
                    MenuAction::Redraw
                }
                // Not a submenu row: the host's (the next menu bar title).
                _ => MenuAction::None,
            },
            MenuKey::Left => {
                if last > 0 {
                    self.levels.truncate(last);
                    MenuAction::Redraw
                } else {
                    MenuAction::None
                }
            }
            MenuKey::Escape => {
                if last > 0 {
                    self.levels.truncate(last);
                    MenuAction::Redraw
                } else {
                    MenuAction::Close
                }
            }
            // With nothing highlighted, Enter takes the bold default row.
            MenuKey::Enter => {
                let row = enter_row(&self.levels[last].items, self.levels[last].hovered);
                self.levels[last].hovered = row;
                self.pick(tree, last, row, cx)
            }
            MenuKey::Char(c) => {
                let row = rows_for_key(&self.levels[last].items, c).next();
                match row {
                    Some(r) => {
                        self.levels[last].hovered = Some(r);
                        self.pick(tree, last, Some(r), cx)
                    }
                    None => MenuAction::None,
                }
            }
        }
    }

    pub fn click(&mut self, tree: &MenuTree, x: i32, y: i32, cx: &MenuCtx) -> MenuAction {
        match self.hit(x, y) {
            None => MenuAction::Close,
            Some((level, None)) => {
                self.levels.truncate(level + 1);
                MenuAction::Redraw
            }
            Some((level, Some(row))) => {
                self.levels[level].hovered = Some(row);
                self.pick(tree, level, Some(row), cx)
            }
        }
    }

    fn pick(
        &mut self,
        tree: &MenuTree,
        level: usize,
        row: Option<usize>,
        cx: &MenuCtx,
    ) -> MenuAction {
        let Some(row) = row else {
            return MenuAction::Redraw;
        };
        if !self.levels[level]
            .items
            .get(row)
            .is_some_and(MenuItem::selectable)
        {
            return MenuAction::None;
        }
        if self.levels[level].opens_submenu(row) {
            if !self.child_open_for(level, Some(row)) {
                self.open_child(tree, level, row, cx, true);
            }
            return MenuAction::Redraw;
        }
        let mut path = self.levels[level].path.clone();
        path.push(row);
        MenuAction::Chosen(path)
    }

    /// Under a look whose menus cast a shadow
    /// ([`MenuLook::shadow`](crate::style::MenuLook::shadow)) each level
    /// casts it over what lies under it first.
    /// `underline` is the host's cue; a chain opened from the keyboard underlines regardless.
    pub fn draw(&self, c: &mut Canvas, theme: &Theme, text: &TextRenderer, underline: bool) {
        if self.hosted {
            return;
        }
        for i in 0..self.levels.len() {
            self.draw_level(i, (0, 0), c, theme, text, underline);
        }
    }

    /// Level `i` alone with its shadow, moved `by` logical px: a host that shows each level on
    /// its own surface draws it there at its corner.
    pub fn draw_level(
        &self,
        i: usize,
        by: (i32, i32),
        c: &mut Canvas,
        theme: &Theme,
        text: &TextRenderer,
        underline: bool,
    ) {
        let Some(level) = self.levels.get(i) else {
            return;
        };
        let underline = underline || self.keys;
        let s = theme.scale.max(1);
        let (x, y) = (level.origin.0 + by.0, level.origin.1 + by.1);
        if let Some(m) = menu_look(theme) {
            let r = Rect::new(x, y, level.layout.width, level.layout.height).scaled(s);
            crate::look::styled::menu::draw_shadow(&m, c, r, s);
        }
        draw_menu_at(
            c,
            (x * s, y * s),
            theme,
            text,
            &level.layout.scaled(s),
            &level.items,
            level.hovered,
            "",
            underline,
        );
    }

    /// Each level's [`Spot`]: the root off its anchor, a submenu beside its row.
    pub fn spots(&self) -> Vec<Spot> {
        (0..self.levels.len())
            .map(|i| Spot {
                rect: self.levels[i].rect(),
                anchor: match i {
                    0 => self.anchor,
                    _ => {
                        let (rect, dy) = self.submenu_anchor(i).unwrap_or_default();
                        Anchor::Beside { rect, dy }
                    }
                },
            })
            .collect()
    }

    /// Opens the root off `anchor` when a host shows it ([`PopupMenu::spots`]); `open`'s point
    /// by default.
    pub fn set_anchor(&mut self, anchor: Anchor) {
        self.anchor = anchor;
    }

    /// The host shows the chain on popups: [`PopupMenu::draw`] leaves it out.
    pub fn host(&mut self) {
        self.hosted = true;
    }

    /// Moves level `i` to `origin`, where its host's compositor put it.
    pub fn place(&mut self, i: usize, origin: (i32, i32)) {
        if let Some(level) = self.levels.get_mut(i) {
            level.origin = origin;
        }
    }

    /// Where level `i` (a submenu) opens against its parent, for a host that lets the
    /// compositor place it: an anchor rect in the parent level's own coordinates and the
    /// child's offset down from it. The child's left edge sits on the rect's right end, or
    /// flipped, its right edge on the rect's left end, as [`place_submenu`] puts it.
    pub fn submenu_anchor(&self, i: usize) -> Option<(Rect, i32)> {
        let child = self.levels.get(i).filter(|_| i > 0)?;
        let parent = &self.levels[i - 1];
        let row = parent.layout.rows.get(*child.path.last()?)?;
        let overlap = BORDER + INSET.left;
        let w = (parent.layout.width - 2 * overlap).max(1);
        Some((Rect::new(overlap, row.y, w, row.height), -INSET.top))
    }
}

impl Chain for PopupMenu {
    fn chain(&self) -> &PopupMenu {
        self
    }
    fn chain_mut(&mut self) -> &mut PopupMenu {
        self
    }
}
