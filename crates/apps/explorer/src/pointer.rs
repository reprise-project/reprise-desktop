//! Where a pointer event goes: the chrome from the top layer down (popup, menu
//! bar, toolbar, address bar, Explorer bar, task pane, view). Every branch ends
//! in a [`Command`] and [`Explorer::run`]; nothing changes the model directly.

use crate::commands::{Command, Menu, next_mode};
use crate::menus;
use crate::window::Popup;
use crate::{BTN_LEFT, BTN_RIGHT, Explorer, Focus};
use repriseapp::{Ctx, PointerInput, Reply};
use reprisekit::classic::widgets::{Axis, ScrollHit};
use reprisekit::controls::addressbar;
use reprisekit::controls::explorerbar;
use reprisekit::controls::menu::MenuAction;
use reprisekit::controls::menubar::Action;
use reprisekit::controls::scrollbar::Press;
use reprisekit::controls::shellview;
use reprisekit::controls::statusbar;
use reprisekit::controls::taskpane;
use reprisekit::controls::toolbar;
use reprisekit::controls::treeview;
use reprisekit::look::widgets::item;
use reprisekit::pointer::Nav;
use reprisekit::raster::Rect;
use reprisekit::selection;

type BarPart = (Axis, ScrollHit);

/// One click of the tree's horizontal scrollbar arrows, in logical px: a little
/// under an indent level.
const HSCROLL_STEP: i32 = 16;

impl Explorer {
    /// Abandon a drag in progress, putting back what the press found (drag image
    /// and drop highlight, the selection under a marquee, or the column's width).
    /// `true` when there was one.
    pub(crate) fn cancel_drag(&mut self) -> bool {
        if self.cancel_item_drag() {
            return true;
        }
        if let Some((_, base, _)) = self.marquee_from.take() {
            self.sv.marquee = None;
            self.sv.selection.set = base;
            self.sv.selection.focus = self.sv.selection.set.iter().next().copied();
            self.sv.selection.anchor = self.sv.selection.focus;
            return true;
        }
        if let Some((col, _, width)) = self.resizing.take() {
            if let Some(c) = self.sv.columns.get_mut(col) {
                c.width = width;
            }
            return true;
        }
        false
    }

    pub fn pointer_inner(&mut self, p: PointerInput, ctx: &mut Ctx) -> Reply {
        // A button takes the keyboard away from a half-finished Alt: its release will
        // not mean the menu bar.
        if matches!(p, PointerInput::Press { .. }) {
            self.alt_arm.interrupt();
        }
        match p {
            PointerInput::Press {
                x,
                y,
                button,
                time,
                mods,
            } => match button {
                BTN_LEFT => {
                    // The scrollbar part under the press stays held until the release.
                    let (tree, view) = self.bars_at(x, y, ctx);
                    let column = self.header_at(x, y, ctx);
                    let lit = (self.tree.bars.press_bar(tree)
                        | self.sv.bars.press_bar(view)
                        | self.sv.header.press(column, false))
                        && ctx.styled();
                    let reply = self.press(x, y, time, mods, ctx);
                    Reply {
                        redraw: reply.redraw || lit,
                        ..reply
                    }
                }
                BTN_RIGHT => self.right_press(x, y, ctx),
                b => match reprisekit::pointer::nav_button(b) {
                    Some(nav) => self.nav_press(nav, ctx),
                    None => Reply::none(),
                },
            },
            // A thumb button acts on its press alone: its release must not end a drag the
            // left button holds.
            PointerInput::Release { button, .. }
                if reprisekit::pointer::nav_button(button).is_some() =>
            {
                Reply::none()
            }
            PointerInput::Release { x, y, .. } => {
                self.tree.bars.release_bar();
                self.sv.bars.release_bar();
                self.sv.header.release();
                // A drag owns the release: it drops, or acts on what the press meant.
                if self.drag.is_some() {
                    return self.drag_release(x, y, ctx);
                }
                if std::mem::take(&mut self.dragging) {
                    self.settings.bar_width = self.explorer_bar.width;
                    self.settings_dirty = true;
                }
                // The Recycle Bin's columns are not the ones the settings
                // keep.
                if self.resizing.take().is_some() && self.folder_columns() {
                    self.settings.columns = self.sv.columns.iter().map(|c| c.width).collect();
                    self.settings_dirty = true;
                }
                self.thumb.release();
                self.tree_thumb.release();
                self.marquee_from = None;
                self.sv.marquee = None;
                self.toolbar.pressed = None;
                Reply::redraw()
            }
            PointerInput::Motion { x, y } => self.motion(x, y, ctx),
            PointerInput::Scroll { x, y, rows } => self.scroll(x, y, rows, ctx),
            PointerInput::Leave => {
                // The pointer off the window disarms a drag that never started; a started one
                // is the compositor's.
                self.cancel_item_drag();
                self.close_drop_menu();
                self.toolbar.hot = None;
                self.bar.leave();
                self.explorer_bar.hot_close = false;
                self.taskpane.hot = None;
                self.taskpane.hot_header = None;
                self.tree.bars.leave();
                self.tree.unhover();
                self.sv.unhover();
                self.sv.bars.leave();
                self.sv.header.leave();
                Reply::redraw()
            }
        }
    }

    /// A thumb button: Back or Forward, as Alt+Left and Alt+Right. An open menu
    /// closes first and an in-place rename commits instead; a drag ignores it.
    fn nav_press(&mut self, nav: Nav, ctx: &mut Ctx) -> Reply {
        if self.drag.is_some() {
            return Reply::none();
        }
        self.popup = None;
        if self.sv.edit.is_some() {
            return self.commit_rename(ctx);
        }
        self.run(
            match nav {
                Nav::Back => Command::Back,
                Nav::Forward => Command::Forward,
            },
            ctx,
        )
    }

    fn press(&mut self, x: i32, y: i32, time: u32, mods: repriseapp::Mods, ctx: &mut Ctx) -> Reply {
        let client = self.layout(ctx).client;
        if let Some(p) = self.popup.as_mut() {
            return match p.click(x, y, ctx, client) {
                MenuAction::Chosen(path) => {
                    let cmd = p.command(&path);
                    self.popup = None;
                    self.run(cmd, ctx)
                }
                MenuAction::Close => {
                    self.popup = None;
                    Reply::redraw()
                }
                _ => Reply::redraw(),
            };
        }

        self.sync_menus();
        let lay = self.layout(ctx);
        // A click outside the view commits an in-place rename; inside
        // it, the view branch below decides.
        if self.sv.edit.is_some() && !lay.view.contains(x, y) {
            return self.commit_rename(ctx);
        }
        let ml = self.bar_layout(ctx);
        let (m, px) = (ctx.theme.clone(), ctx.theme.fonts.size_px);
        match self.bar.press(x, y, &ml, &ctx.text, &m, px) {
            Action::Chosen { title, path, .. } => {
                let cmd = menus::command_for(title, &path, &self.menu_state());
                return self.run(cmd, ctx);
            }
            Action::Redraw => return Reply::redraw(),
            Action::Ignored => {}
        }

        if let Some(r) = lay.status
            && statusbar::hit_grip(&self.status, r, x, y)
        {
            return Reply::resize();
        }

        if let Some(r) = lay.toolbar {
            let rects = toolbar::layout(&self.toolbar, &ctx.text, px, r, &ctx.theme.controls());
            if let Some((i, part)) =
                toolbar::hit_part(&self.toolbar, &rects, x, y, &ctx.theme.controls())
            {
                return self.toolbar_press(i, part, rects[i], lay.client, ctx);
            }
            if r.contains(x, y) {
                return Reply::none();
            }
        }

        if let Some(r) = lay.address {
            let al = addressbar::layout(&self.address, &ctx.text, px, r, &ctx.theme.controls());
            let hit = addressbar::hit(&self.address, &al, x, y);
            if hit != addressbar::Hit::Nothing {
                self.focus = Focus::Address;
                let ev = self.address.click(hit, x, &al, &ctx.text, px, 1);
                return self.address_event(ev, ctx);
            }
            if r.contains(x, y) {
                return Reply::none();
            }
        }

        if let Some(bl) = &lay.bar {
            match explorerbar::hit(bl, x, y) {
                explorerbar::Hit::Close => return self.run(Command::CloseBar, ctx),
                explorerbar::Hit::Splitter => {
                    self.dragging = true;
                    return Reply::redraw();
                }
                explorerbar::Hit::Body => {
                    let Some(frame) = lay.tree else {
                        return Reply::none();
                    };
                    self.focus = Focus::Tree;
                    let tl =
                        treeview::layout(&self.tree, &ctx.text, px, frame, &ctx.theme.controls());
                    match self.tree.hit(&tl, x, y) {
                        treeview::Hit::Expander(i) => {
                            if let Some(p) = self.tree.toggle(i, &tl) {
                                self.fill_node(p, ctx);
                            }
                        }
                        // A click opens the row as it picks it, and the
                        // view goes there.
                        treeview::Hit::Row(i) => {
                            if let Some(p) = self.tree.click_row(i, &tl) {
                                self.fill_node(p, ctx);
                            }
                            if let Some(p) =
                                self.tree.selected.clone().and_then(|p| self.place_of(&p))
                                && p != self.place
                            {
                                return self.go(p, ctx);
                            }
                        }
                        treeview::Hit::VBar(bar) => {
                            match self.tree_thumb.press(Axis::Vertical, bar) {
                                Press::Grab => return Reply::none(),
                                press => self.tree.scroll(press.by(1, tl.rows as i64), &tl),
                            };
                        }
                        // A deep chain overflows the bar; the tree grows
                        // its own horizontal bar for it.
                        treeview::Hit::HBar(bar) => {
                            let page = tl.body.width as i64;
                            match self.tree_thumb.press(Axis::Horizontal, bar) {
                                Press::Grab => return Reply::none(),
                                press => self
                                    .tree
                                    .hscroll(press.by(HSCROLL_STEP as i64, page) as i32, &tl),
                            };
                        }
                        _ => {}
                    }
                    return Reply::redraw();
                }
                explorerbar::Hit::Nothing => {}
            }
        }

        if let Some(r) = lay.tasks
            && r.contains(x, y)
        {
            let tl = taskpane::layout(&self.taskpane, &ctx.text, px, r, &ctx.theme.controls());
            let hit = taskpane::hit(&self.taskpane, &tl, x, y);
            let ev = self.taskpane.click(hit);
            return self.taskpane_event(ev, ctx);
        }

        if lay.view.contains(x, y) {
            self.focus = Focus::View;
            let sl = self.view_layout(lay.view, ctx);
            let hit = shellview::hit(&self.sv, &sl, x, y);
            // An in-place rename commits on a click anywhere but the box itself.
            if let Some((i, _)) = self.sv.edit.clone() {
                if shellview::rename_rect(&self.sv, &sl, i, &ctx.text, px)
                    .is_some_and(|r| r.contains(x, y))
                {
                    return Reply::none();
                }
                let reply = self.commit_rename(ctx);
                // The listing may have moved under the click; let the
                // next one land on the new layout.
                return reply;
            }
            match hit {
                // The double-click, the selection and the drag this
                // press arms are all `drag::press_entry`'s.
                shellview::Hit::Entry(i) => {
                    return self.press_entry(i, (x, y), time, mods, sl.cells.get(i).copied(), ctx);
                }
                shellview::Hit::Background => {
                    if !mods.ctrl {
                        self.sv.selection.clear();
                    }
                    // The marquee starts here and only becomes visible
                    // once the pointer has moved `DRAG_SLOP` px.
                    self.marquee_from = Some(((x, y), self.sv.selection.set.clone(), mods.ctrl));
                    self.clicks.reset();
                }
                shellview::Hit::Header(i) => return self.run(Command::Sort(i), ctx),
                shellview::Hit::Divider(i) => {
                    let width = self.sv.columns.get(i).map_or(0, |c| c.width);
                    self.resizing = Some((i, x, width));
                    return Reply::none();
                }
                // The view's own steps: a line or a page of whatever
                // the mode scrolls by.
                shellview::Hit::VBar(bar) => match self.thumb.press(Axis::Vertical, bar) {
                    Press::Lines(n) => _ = shellview::scroll(&mut self.sv, n as i32, &sl),
                    Press::Pages(n) => _ = shellview::page(&mut self.sv, n as i32, &sl),
                    Press::Grab => return Reply::none(),
                    Press::Nothing => {}
                },
                shellview::Hit::HBar(bar) => match self.thumb.press(Axis::Horizontal, bar) {
                    Press::Lines(n) => _ = shellview::hstep(&mut self.sv, n as i32, &sl),
                    Press::Pages(n) => _ = shellview::hpage(&mut self.sv, n as i32, &sl),
                    Press::Grab => return Reply::none(),
                    Press::Nothing => {}
                },
                _ => {}
            }
            self.refresh_chrome(ctx);
            return Reply::redraw();
        }
        Reply::none()
    }
    fn toolbar_press(
        &mut self,
        i: usize,
        part: toolbar::Part,
        rect: Rect,
        client: Rect,
        ctx: &mut Ctx,
    ) -> Reply {
        self.toolbar.pressed = Some(i);
        let id = match &self.toolbar.items[i] {
            toolbar::Item::Button { id, .. } | toolbar::Item::DropButton { id, .. } => *id,
            toolbar::Item::Separator => return Reply::none(),
        };
        if part == toolbar::Part::Drop {
            let at = (rect.x, rect.bottom());
            let menu = match id {
                toolbar::VIEWS => menus::views_menu(self.sv.mode),
                toolbar::BACK => {
                    let mut m = Menu::default();
                    for (n, p) in self.history.back_list().iter().enumerate() {
                        m.row(
                            reprisekit::controls::menu::MenuItem::entry(p.title()),
                            Command::Go(n + 1),
                        );
                    }
                    m
                }
                // The Forward drop is Back's mirror: the places ahead, nearest first.
                toolbar::FORWARD => {
                    let mut m = Menu::default();
                    for (n, p) in self.history.forward_list().iter().enumerate() {
                        m.row(
                            reprisekit::controls::menu::MenuItem::entry(p.title()),
                            Command::GoForward(n + 1),
                        );
                    }
                    m
                }
                _ => return Reply::redraw(),
            };
            if menu.items().is_empty() {
                return Reply::redraw();
            }
            self.popup = Some(Popup::new(at, menu, client, ctx, false));
            return Reply::redraw();
        }
        let cmd = match id {
            toolbar::BACK => Command::Back,
            toolbar::FORWARD => Command::Forward,
            toolbar::UP => Command::Up,
            toolbar::FOLDERS => Command::ToggleFolders,
            toolbar::VIEWS => Command::Mode(next_mode(self.sv.mode)),
            _ => Command::Nothing,
        };
        let mut reply = self.run(cmd, ctx);
        reply.redraw = true;
        reply
    }

    /// A click in the task pane: a chevron collapses its section (remembered by
    /// title), a live row runs its [`crate::pane`] command.
    fn taskpane_event(&mut self, ev: taskpane::Event, ctx: &mut Ctx) -> Reply {
        match ev {
            taskpane::Event::Toggle(i) => {
                if let Some(s) = self.taskpane.sections.get(i) {
                    self.settings.pane_open.insert(s.title.clone(), s.open);
                    self.settings_dirty = true;
                }
                Reply::redraw()
            }
            taskpane::Event::Row(i, j) => {
                let cmd = self
                    .pane_cmds
                    .get(i)
                    .and_then(|rows| rows.get(j))
                    .cloned()
                    .unwrap_or(Command::Nothing);
                self.run(cmd, ctx)
            }
            taskpane::Event::None => Reply::none(),
            taskpane::Event::Redraw => Reply::redraw(),
        }
    }

    fn right_press(&mut self, x: i32, y: i32, ctx: &mut Ctx) -> Reply {
        self.popup = None;
        let lay = self.layout(ctx);
        let px = ctx.theme.fonts.size_px;
        // A right-click in the rename box moves its caret unless it lands in the selection, and
        // leaves it to the edit menu; anywhere else it commits the rename, before the menu it
        // opens sees the new selection.
        if let Some((i, _)) = self.sv.edit {
            let sl = self.view_layout(lay.view, ctx);
            if lay.view.contains(x, y)
                && shellview::rename_rect(&self.sv, &sl, i, &ctx.text, px)
                    .is_some_and(|r| r.contains(x, y))
            {
                let at = shellview::edit_index_at(&self.sv, &sl, x, y, &ctx.text, px);
                if let (Some(at), Some((_, ed))) = (at, self.sv.edit.as_mut()) {
                    ed.press_for_menu(at);
                }
                return Reply::redraw();
            }
            return self.commit_rename(ctx);
        }
        if let Some(r) = lay.address {
            let al = addressbar::layout(&self.address, &ctx.text, px, r, &ctx.theme.controls());
            let hit = addressbar::hit(&self.address, &al, x, y);
            if self.address.right_press(hit, x, &al, &ctx.text, px, 1) != addressbar::Event::None {
                self.focus = Focus::Address;
                return Reply::redraw();
            }
        }
        if lay.view.contains(x, y) {
            let sl = self.view_layout(lay.view, ctx);
            if let shellview::Hit::Entry(i) = shellview::hit(&self.sv, &sl, x, y) {
                // An entry's menu waits for the release: the same press starts a right drag.
                // The selection follows the press, and an entry already in it keeps the rest.
                if !self.sv.selection.set.contains(&i) {
                    self.sv.selection.set_only(Some(i));
                    self.refresh_chrome(ctx);
                }
                if sl.cells.get(i).is_some() {
                    self.arm_drag(i, BTN_RIGHT, x, y);
                }
                // Without an armed drag nothing waits for the release, so the menu opens now.
                if self.drag.is_none() {
                    return self.entry_menu_at(i, (x, y), ctx, false);
                }
                return Reply::redraw();
            }
            return self.item_menu_at(x, y, ctx);
        }
        if let (Some(bl), Some(frame)) = (&lay.bar, lay.tree)
            && bl.body.contains(x, y)
        {
            let tl = treeview::layout(&self.tree, &ctx.text, px, frame, &ctx.theme.controls());
            if let treeview::Hit::Row(i) = self.tree.hit(&tl, x, y) {
                self.tree.select(i, &tl);
            }
            let node = self
                .tree
                .selected
                .clone()
                .and_then(|p| self.tree.node(&p))
                .map(|n| (n.expanded, n.has_children))
                .unwrap_or((false, false));
            let folder = self.node_folder().is_some();
            let menu = menus::tree_menu(node.0, node.1, folder);
            self.popup = Some(Popup::new((x, y), menu, lay.client, ctx, false));
            return Reply::redraw();
        }
        Reply::none()
    }

    fn motion(&mut self, x: i32, y: i32, ctx: &mut Ctx) -> Reply {
        if self.drag.is_some() {
            return self.drag_motion(x, y, ctx);
        }
        // While something holds the pointer (a thumb, a marquee, a
        // column or the bar's splitter), only a held bar part follows it:
        // no other surface lights under a grab.
        let grab = self.thumb.holding()
            || self.tree_thumb.holding()
            || self.marquee_from.is_some()
            || self.resizing.is_some()
            || self.dragging;
        let held = self.tree.bars.holding() || self.sv.bars.holding() || self.sv.header.holding();
        let (tree, view) = self.bars_at(x, y, ctx);
        let free = !grab && !held;
        let tree_lit = (free || self.tree.bars.holding()) && self.tree.bars.motion(tree);
        let view_lit = (free || self.sv.bars.holding()) && self.sv.bars.motion(view);
        let column = self.header_at(x, y, ctx);
        let head_lit = (free || self.sv.header.holding()) && self.sv.header.motion(column);
        let bars = (tree_lit || view_lit || head_lit) && ctx.styled();
        let hot = self.tree_hover(free, x, y, ctx);
        let view_hot = self.view_hover(free, x, y, ctx);
        let mut reply = self.hover_motion(x, y, ctx);
        reply.redraw |= bars || hot || view_hot;
        reply
    }

    /// The Folders pane's row under the pointer turns hot while nothing is held and
    /// no menu is open. True when that changed what draws.
    fn tree_hover(&mut self, free: bool, x: i32, y: i32, ctx: &Ctx) -> bool {
        let frame = self.layout(ctx).tree;
        match frame {
            Some(frame) if free && self.popup.is_none() => {
                let px = ctx.theme.fonts.size_px;
                let cm = ctx.theme.controls();
                let tl = treeview::layout(&self.tree, &ctx.text, px, frame, &cm);
                let hot = self.tree.hover(&tl, &ctx.text, px, x, y);
                // Only a look that boxes its items lights the row and the
                // expander under the pointer.
                let boxed = item::boxed(&ctx.theme.paint());
                hot | (boxed && self.tree.track(&tl, x, y))
            }
            _ => self.tree.unhover(),
        }
    }

    /// The view's entry under the pointer turns hot while nothing is held and no
    /// menu is open, under a look that boxes its items. True when that changed what
    /// draws.
    fn view_hover(&mut self, free: bool, x: i32, y: i32, ctx: &Ctx) -> bool {
        if !free || self.popup.is_some() || !item::boxed(&ctx.theme.paint()) {
            return self.sv.unhover();
        }
        let frame = self.layout(ctx).view;
        let sl = self.view_layout(frame, ctx);
        self.sv.hover(&sl, x, y)
    }

    /// The scrollbar parts at window point (`x`, `y`): the folder tree's
    /// and the view's, each `None` off its bars, under an open menu, or
    /// under a look that draws no part lit (which skips the layouts).
    fn bars_at(&self, x: i32, y: i32, ctx: &Ctx) -> (Option<BarPart>, Option<BarPart>) {
        if self.popup.is_some() || !ctx.styled() {
            return (None, None);
        }
        let on = |r: Option<Rect>| r.is_some_and(|r| r.contains(x, y));
        let lay = self.layout(ctx);
        let tree = lay.tree.filter(|r| r.contains(x, y)).and_then(|frame| {
            let px = ctx.theme.fonts.size_px;
            let cm = ctx.theme.controls();
            let tl = treeview::layout(&self.tree, &ctx.text, px, frame, &cm);
            (on(tl.scrollbar) || on(tl.hscroll))
                .then(|| self.tree.hit(&tl, x, y).bar())
                .flatten()
        });
        let view = lay.view.contains(x, y).then(|| {
            let sl = self.view_layout(lay.view, ctx);
            (on(sl.vscroll) || on(sl.hscroll))
                .then(|| shellview::hit(&self.sv, &sl, x, y).bar())
                .flatten()
        });
        (tree, view.flatten())
    }

    /// The Details header column under (`x`, `y`), which lights under the
    /// pointer in a look that draws it lit; `None` elsewhere, in Classic
    /// or with a menu open.
    fn header_at(&self, x: i32, y: i32, ctx: &Ctx) -> Option<usize> {
        if self.popup.is_some() || !ctx.styled() || self.sv.mode != shellview::Mode::Details {
            return None;
        }
        let lay = self.layout(ctx);
        if !lay.view.contains(x, y) {
            return None;
        }
        let sl = self.view_layout(lay.view, ctx);
        match shellview::hit(&self.sv, &sl, x, y) {
            shellview::Hit::Header(i) => Some(i),
            _ => None,
        }
    }

    /// A motion: thumbs, the view's and the bars' hover.
    fn hover_motion(&mut self, x: i32, y: i32, ctx: &mut Ctx) -> Reply {
        if self.thumb.holding() {
            let frame = self.layout(ctx).view;
            let sl = self.view_layout(frame, ctx);
            let moved = match (
                self.thumb.grab(Axis::Vertical),
                self.thumb.grab(Axis::Horizontal),
            ) {
                (Some(grab), _) => shellview::drag_thumb(&mut self.sv, y, grab, &sl),
                (_, Some(grab)) => shellview::drag_hthumb(&mut self.sv, x, grab, &sl),
                _ => false,
            };
            return if moved {
                Reply::redraw()
            } else {
                Reply::none()
            };
        }
        if self.tree_thumb.holding() {
            let Some(frame) = self.layout(ctx).tree else {
                return Reply::none();
            };
            let px = ctx.theme.fonts.size_px;
            let tl = treeview::layout(&self.tree, &ctx.text, px, frame, &ctx.theme.controls());
            let (top, left) = (self.tree.top, self.tree.left);
            if let Some(grab) = self.tree_thumb.grab(Axis::Vertical) {
                self.tree.top = self.tree.top_at(&tl, y, grab);
            }
            if let Some(grab) = self.tree_thumb.grab(Axis::Horizontal) {
                self.tree.left = self.tree.left_at(&tl, x, grab);
            }
            return if (top, left) == (self.tree.top, self.tree.left) {
                Reply::none()
            } else {
                Reply::redraw()
            };
        }
        if let Some((col, from, width)) = self.resizing {
            let u = ctx.theme.controls().text;
            return if shellview::resize_column(&mut self.sv, col, width, x - from, u) {
                Reply::redraw()
            } else {
                Reply::none()
            };
        }
        if let Some((origin, base, ctrl)) = self.marquee_from.clone() {
            if (x - origin.0).abs() < crate::DRAG_SLOP && (y - origin.1).abs() < crate::DRAG_SLOP {
                return Reply::none();
            }
            let frame = self.layout(ctx).view;
            let sl = self.view_layout(frame, ctx);
            let band = selection::band(origin, (x, y));
            // `marquee` builds the set from `base` alone, so the old set is
            // free to take for the comparison. The chrome reads only the
            // set, and most moves leave it as it was.
            let before = std::mem::take(&mut self.sv.selection.set);
            self.sv.selection.marquee(&sl.cells, band, &base, ctrl);
            self.sv.marquee = Some(band);
            if self.sv.selection.set != before {
                self.refresh_chrome(ctx);
            }
            return Reply::redraw();
        }
        if self.dragging {
            // The view keeps at least 200 px, the width the task pane
            // would have taken.
            let u = ctx.theme.controls().text;
            let widest = u.within(ctx.size.0 - explorerbar::SPLITTER_W - 200);
            let width = u
                .within(x - explorerbar::SPLITTER_W / 2)
                .clamp(explorerbar::MIN_W, widest.max(explorerbar::MIN_W));
            if width != self.explorer_bar.width {
                self.explorer_bar.width = width;
                return Reply::redraw();
            }
            return Reply::none();
        }
        if self.popup.is_some() {
            let client = self.layout(ctx).client;
            let Some(p) = self.popup.as_mut() else {
                return Reply::none();
            };
            return if p.motion(x, y, ctx, client) {
                Reply::redraw()
            } else {
                Reply::none()
            };
        }
        let lay = self.layout(ctx);
        let (m, px) = (ctx.theme.clone(), ctx.theme.fonts.size_px);
        if self.bar.open.is_some() {
            let ml = self.bar_layout(ctx);
            self.bar.motion(x, y, &ml, &ctx.text, &m, px);
            return Reply::redraw();
        }
        let mut dirty = false;
        let ml = self.bar_layout(ctx);
        dirty |= self.bar.hover(x, y, &ml) && ctx.styled();
        dirty |= self.hover_view(x, y, &lay, ctx);
        if let Some(r) = lay.toolbar {
            let rects = toolbar::layout(&self.toolbar, &ctx.text, px, r, &ctx.theme.controls());
            let hot = toolbar::hit(&self.toolbar, &rects, x, y);
            if self.toolbar.hot != hot {
                self.toolbar.hot = hot;
                dirty = true;
            }
        }
        if let Some(r) = lay.address {
            let al = addressbar::layout(&self.address, &ctx.text, px, r, &ctx.theme.controls());
            let go = matches!(
                addressbar::hit(&self.address, &al, x, y),
                addressbar::Hit::Go
            );
            if self.address.go_hot != go {
                self.address.go_hot = go;
                dirty = true;
            }
        }
        dirty |= self.hover_tasks(x, y, &lay, ctx);
        if let Some(bl) = &lay.bar {
            let close = matches!(explorerbar::hit(bl, x, y), explorerbar::Hit::Close);
            if self.explorer_bar.hot_close != close {
                self.explorer_bar.hot_close = close;
                dirty = true;
            }
        }
        if dirty {
            Reply::redraw()
        } else {
            Reply::none()
        }
    }

    /// Light the task pane's title band or enabled link under `(x, y)`; whether
    /// anything changed.
    fn hover_tasks(&mut self, x: i32, y: i32, lay: &crate::window::Layout, ctx: &Ctx) -> bool {
        let hit = lay.tasks.filter(|r| r.contains(x, y)).map(|r| {
            let px = ctx.theme.fonts.size_px;
            let tl = taskpane::layout(&self.taskpane, &ctx.text, px, r, &ctx.theme.controls());
            taskpane::hit(&self.taskpane, &tl, x, y)
        });
        let header = match hit {
            Some(taskpane::Hit::Header(i)) => Some(i),
            _ => None,
        };
        let link = match hit {
            Some(taskpane::Hit::Row(i, j)) => Some((i, j)),
            _ => None,
        };
        let changed = self.taskpane.hot_header != header || self.taskpane.hot != link;
        self.taskpane.hot_header = header;
        self.taskpane.hot = link;
        changed
    }

    /// The wheel: the address bar's open list takes it wherever the
    /// pointer is, else the tree, task pane or view under the pointer
    /// scrolls.
    fn scroll(&mut self, x: i32, y: i32, rows: i32, ctx: &mut Ctx) -> Reply {
        // The open list lies over the view. It shows every place, so it
        // has nothing to scroll, and nothing beneath it moves either.
        if self.address.combo.open.is_some() {
            return if self.address.combo.scroll(rows) {
                Reply::redraw()
            } else {
                Reply::none()
            };
        }
        let lay = self.layout(ctx);
        let px = ctx.theme.fonts.size_px;
        if let Some(frame) = lay.tree
            && lay.bar.as_ref().is_some_and(|b| b.body.contains(x, y))
        {
            let tl = treeview::layout(&self.tree, &ctx.text, px, frame, &ctx.theme.controls());
            self.tree.scroll(rows as i64, &tl);
            return Reply::redraw();
        }
        if let Some(r) = lay.tasks
            && r.contains(x, y)
        {
            self.taskpane.top = (self.taskpane.top + rows * 20).max(0);
            return Reply::redraw();
        }
        if lay.view.contains(x, y) {
            let sl = self.view_layout(lay.view, ctx);
            shellview::scroll(&mut self.sv, rows, &sl);
            return Reply::redraw();
        }
        Reply::none()
    }
}
