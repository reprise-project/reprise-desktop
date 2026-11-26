//! [`MenuTree`] and the open chain of [`PopupMenu`] levels: pointer, keys and placement.

use super::draw::draw_menu_at;
use super::keys::{KeyAct, MenuKey, key_act_in};
use super::layout::{BORDER, INSET, MenuLayout, MenuStyle, ScrollArrow, place_submenu};
use super::shadow::menu_look;
use super::{KeyStops, MenuItem, step_selection};
use crate::controls::hosted::{Anchor, Chain, Spot};
use crate::raster::{Canvas, Rect};
use crate::text::TextRenderer;
use crate::theme::Theme;
use std::time::{Duration, Instant};

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

/// How often a scroll arrow the pointer rests on scrolls its menu a row.
pub const SCROLL_REPEAT: Duration = Duration::from_millis(60);

/// The open chain. Hovering a row opens or closes a submenu after `[metrics] menu_hover_ms`,
/// and resting on a scroll arrow scrolls every [`SCROLL_REPEAT`]: the host calls
/// [`PopupMenu::settle`] once [`PopupMenu::due`] passes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PopupMenu {
    pub(super) levels: Vec<Level>,
    /// What the hovered row does once the delay passes.
    pending: Option<Pending>,
    /// Opened from the keyboard: the chain underlines its access keys
    /// whatever its host's cues say.
    pub(crate) keys: bool,
    /// What the root opens off: the point it was asked at, before the clamp into `within`.
    pub(crate) anchor: Anchor,
    /// Shown on popups by a host ([`Hosted`]): [`PopupMenu::draw`] leaves it out.
    hosted: bool,
    /// The scroll arrow the pointer rests on: its level and when it scrolls next.
    scrolling: Option<(usize, ScrollArrow, Instant)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Pending {
    /// The level whose row the pointer rests on.
    level: usize,
    at: Instant,
    /// The submenu row it rests on; `None` for a plain row, a separator or the border.
    opens: Option<usize>,
    /// The submenu to open, laid out when the row was hovered; `None` closes what is open past
    /// `level`.
    open: Option<Level>,
}

/// A level taller than `within` wraps into columns, so every row stays on screen.
fn level_layout(items: &[MenuItem], cx: &MenuCtx) -> MenuLayout {
    let room = (cx.within.width, cx.within.height);
    MenuStyle::context_for(cx.theme, items).layout_within(cx.text, items, cx.px, room)
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
            pending: None,
            keys: by_keyboard,
            anchor: Anchor::Point(at.0, at.1),
            hosted: false,
            scrolling: None,
        }
    }

    pub fn levels(&self) -> &[Level] {
        &self.levels
    }

    /// The open levels' rows wear the icons `tree` gives them: the tree they opened from, its
    /// icons swapped. The layout stays.
    pub fn wear_icons(&mut self, tree: &MenuTree) {
        for level in &mut self.levels {
            let Some(from) = tree.at(&level.path) else {
                continue;
            };
            for (item, new) in level.items.iter_mut().zip(from.items()) {
                if let (MenuItem::Entry { icon, .. }, MenuItem::Entry { icon: worn, .. }) =
                    (item, new)
                {
                    *icon = worn.clone();
                }
            }
        }
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

    /// The submenu row `row` of `level` opens, laid out beside its row.
    fn child_level(
        &self,
        tree: &MenuTree,
        level: usize,
        row: usize,
        cx: &MenuCtx,
        by_keyboard: bool,
    ) -> Option<Level> {
        let parent = self.levels.get(level)?;
        let mut path = parent.path.clone();
        path.push(row);
        let child = tree.at(&path)?;
        let r = parent.layout.rows.get(row).copied()?;
        let items = child.items.clone();
        let layout = level_layout(&items, cx);
        let prect = parent.rect();
        let row_abs = Rect::new(prect.x + r.x, prect.y + r.y, r.width, r.height);
        let (x, y) = place_submenu(prect, row_abs, (layout.width, layout.height), cx.within);
        let hovered = by_keyboard
            .then(|| step_selection(&items, None, 1, KeyStops::of(cx.theme)))
            .flatten();
        Some(Level {
            path,
            items,
            origin: (x, y),
            layout,
            hovered,
        })
    }

    fn open_child(
        &mut self,
        tree: &MenuTree,
        level: usize,
        row: usize,
        cx: &MenuCtx,
        by_keyboard: bool,
    ) -> bool {
        let Some(child) = self.child_level(tree, level, row, cx, by_keyboard) else {
            return false;
        };
        self.pending = None;
        self.levels.truncate(level + 1);
        self.levels.push(child);
        true
    }

    fn child_open_for(&self, level: usize, row: Option<usize>) -> bool {
        match (row, self.levels.get(level + 1)) {
            (Some(r), Some(child)) => child.path.last() == Some(&r),
            _ => false,
        }
    }

    /// Hovering a submenu row opens it once the delay passes, and resting on another row of
    /// the same menu closes it again ([`PopupMenu::settle`]). `true` when the lit rows changed.
    pub fn motion(&mut self, tree: &MenuTree, x: i32, y: i32, cx: &MenuCtx) -> bool {
        self.motion_at(tree, x, y, cx, Instant::now())
    }

    /// [`PopupMenu::motion`] at time `now`.
    pub fn motion_at(
        &mut self,
        tree: &MenuTree,
        x: i32,
        y: i32,
        cx: &MenuCtx,
        now: Instant,
    ) -> bool {
        let Some((level, row)) = self.hit(x, y) else {
            self.scrolling = None;
            return false;
        };
        let (ox, oy) = self.levels[level].origin;
        let arrow = self.levels[level].layout.arrow_at(x - ox, y - oy);
        let mut dirty = self.rest_on_arrow(level, arrow, now);
        if self.levels[level].hovered != row {
            self.levels[level].hovered = row;
            dirty = true;
        }
        // A level the pointer reached keeps the rows that lead to it lit.
        for i in 0..level {
            let lead = self.levels[i + 1].path.last().copied();
            if self.levels[i].hovered != lead {
                self.levels[i].hovered = lead;
                dirty = true;
            }
        }
        if self.child_open_for(level, row) {
            self.pending = None;
            return dirty;
        }
        let opens = row.filter(|&r| self.levels[level].opens_submenu(r));
        let waiting = self
            .pending
            .as_ref()
            .is_some_and(|p| p.level == level && p.opens == opens);
        if !waiting {
            let open = opens.and_then(|r| self.child_level(tree, level, r, cx, false));
            let ms = cx.theme.metrics.menu_hover_ms.max(0) as u64;
            self.pending = (open.is_some() || self.levels.len() > level + 1).then(|| Pending {
                level,
                at: now + Duration::from_millis(ms),
                opens,
                open,
            });
        }
        dirty | self.settle(now)
    }

    /// The pointer rests on `arrow` (or none) of `level`: a new arrow scrolls a row at once and
    /// again every [`SCROLL_REPEAT`]. `true` when it scrolled.
    fn rest_on_arrow(&mut self, level: usize, arrow: Option<ScrollArrow>, now: Instant) -> bool {
        let Some(arrow) = arrow else {
            self.scrolling = None;
            return false;
        };
        if self
            .scrolling
            .is_some_and(|(l, a, _)| (l, a) == (level, arrow))
        {
            return false;
        }
        self.scrolling = Some((level, arrow, now + SCROLL_REPEAT));
        self.scroll_level(level, arrow_step(arrow))
    }

    /// Scrolls `level` `rows` rows, closing what it had open; `true` when it moved.
    fn scroll_level(&mut self, level: usize, rows: i32) -> bool {
        let moved = self.levels[level].layout.scroll_by(rows);
        if moved {
            self.levels.truncate(level + 1);
            self.pending = None;
        }
        moved
    }

    /// The wheel turned `rows` rows over (x, y): the menu under it scrolls, and the row under the
    /// pointer lights. `true` when it scrolled.
    pub fn wheel(&mut self, x: i32, y: i32, rows: i32) -> bool {
        let Some((level, _)) = self.hit(x, y) else {
            return false;
        };
        if !self.scroll_level(level, rows) {
            return false;
        }
        let (ox, oy) = self.levels[level].origin;
        self.levels[level].hovered = self.levels[level].layout.hit(x - ox, y - oy);
        true
    }

    /// When the hovered row's submenu opens or closes, or a scroll arrow scrolls; none while
    /// nothing waits.
    pub fn due(&self) -> Option<Instant> {
        let scroll = self.scrolling.map(|(_, _, at)| at);
        self.pending
            .as_ref()
            .map(|p| p.at)
            .into_iter()
            .chain(scroll)
            .min()
    }

    /// Opens or closes the submenu the pointer rests on once its delay has passed by `now`, and
    /// scrolls under a scroll arrow; `true` when the chain changed.
    pub fn settle(&mut self, now: Instant) -> bool {
        let mut scrolled = false;
        if let Some((level, arrow, at)) = self.scrolling
            && at <= now
        {
            scrolled = level < self.levels.len() && self.scroll_level(level, arrow_step(arrow));
            self.scrolling = scrolled.then_some((level, arrow, now + SCROLL_REPEAT));
        }
        scrolled | self.settle_pending(now)
    }

    fn settle_pending(&mut self, now: Instant) -> bool {
        if self.pending.as_ref().is_none_or(|p| p.at > now) {
            return false;
        }
        let Some(p) = self.pending.take() else {
            return false;
        };
        if p.level >= self.levels.len() {
            return false;
        }
        self.levels.truncate(p.level + 1);
        if let Some(child) = p.open {
            self.levels.push(child);
        }
        true
    }

    pub fn key(&mut self, tree: &MenuTree, key: MenuKey, cx: &MenuCtx) -> MenuAction {
        self.pending = None;
        let last = self.levels.len() - 1;
        let level = &self.levels[last];
        let stops = KeyStops::of(cx.theme);
        match key_act_in(&level.items, &level.layout, level.hovered, key, stops) {
            // Enter is the menu's even with no row to run.
            KeyAct::Nothing if key == MenuKey::Enter => MenuAction::Redraw,
            act => self.apply_key(tree, last, act, cx),
        }
    }

    fn apply_key(&mut self, tree: &MenuTree, last: usize, act: KeyAct, cx: &MenuCtx) -> MenuAction {
        match act {
            // Not the menu's: the host decides (Right off a plain row walks to the next title).
            KeyAct::Nothing => MenuAction::None,
            KeyAct::Select(row) => {
                self.select(last, row);
                MenuAction::Redraw
            }
            KeyAct::SelectAnd(row, then) => {
                self.select(last, Some(row));
                self.apply_key(tree, last, *then, cx)
            }
            KeyAct::CloseTopmost if last == 0 => MenuAction::Close,
            KeyAct::CloseSubmenu if last == 0 => MenuAction::None,
            KeyAct::CloseTopmost | KeyAct::CloseSubmenu => {
                self.levels.truncate(last);
                MenuAction::Redraw
            }
            KeyAct::OpenSubmenu(row) => {
                self.open_child(tree, last, row, cx, true);
                MenuAction::Redraw
            }
            KeyAct::Activate(row) => self.pick(tree, last, Some(row), cx),
        }
    }

    /// Lights `row` of `level`, scrolled into view.
    fn select(&mut self, level: usize, row: Option<usize>) {
        let l = &mut self.levels[level];
        l.hovered = row;
        if let Some(r) = row {
            l.layout.scroll_to_show(r);
        }
    }

    pub fn click(&mut self, tree: &MenuTree, x: i32, y: i32, cx: &MenuCtx) -> MenuAction {
        self.pending = None;
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
        submenu_anchor(&self.levels[i - 1].layout, *child.path.last()?)
    }
}

/// [`PopupMenu::submenu_anchor`] for the submenu of row `row` of a level laid out as `parent`.
pub fn submenu_anchor(parent: &MenuLayout, row: usize) -> Option<(Rect, i32)> {
    let row = parent.rows.get(row)?;
    let overlap = BORDER + INSET.left;
    let w = (parent.width - 2 * overlap).max(1);
    Some((Rect::new(overlap, row.y, w, row.height), -INSET.top))
}

fn arrow_step(arrow: ScrollArrow) -> i32 {
    match arrow {
        ScrollArrow::Up => -1,
        ScrollArrow::Down => 1,
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

#[cfg(test)]
mod tests;
