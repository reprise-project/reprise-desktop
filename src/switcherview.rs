//! The Alt+Tab switcher panel on the scene; `reprisekit::controls::switcher` orders, lays out and draws it.
//! Minimized windows and open dialogs are entries after the shown ones; a closing dialog leaves the open panel (`switcher_forget`).

use reprisekit::controls::switcher::{self, SwitcherLayout, ThumbCell};
use reprisekit::interfaces;
use reprisekit::taskpreview::fit;
use reprisescene::{BufferId, Input, TreeId};
use smithay::{backend::input::InputTime, utils::Rectangle};

use crate::{
    compositor::{SceneOwner, State},
    config::SwitcherStyle,
    glass::{self, Backdrop, Pane, PaneShape},
    input::BTN_LEFT,
    keys,
    livepreview::ThumbId,
    output::upload_scaled,
    peek::PeekTarget,
    raster::{Canvas, Rect},
    taskbar::TaskbarHit,
    window::WindowId,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Entry {
    Window(WindowId),
    Dialog(u64),
    Desktop,
}

/// What peeking at `e` keeps on screen; None for a dialog, which has no
/// window to show alone.
fn peek_target(e: &Entry) -> Option<PeekTarget> {
    match *e {
        Entry::Window(w) => Some(PeekTarget::Window(w)),
        Entry::Desktop => Some(PeekTarget::Desktop),
        Entry::Dialog(_) => None,
    }
}

#[derive(Default)]
pub struct SwitcherView {
    pub panel: Option<Switcher>,
    pub stack: Option<crate::flipview::Stack>,
    pub leaving: Option<crate::flipview::Stack>,
    /// Buttons whose press a switcher took: their releases reach no
    /// client either, even after it has closed.
    taken: Vec<u32>,
    /// The 3D switcher's cards are up or flying: every frame draws
    /// inactive and no window shows its shadow (`State::cards_changed`).
    pub cards: bool,
}

impl SwitcherView {
    pub fn is_open(&self) -> bool {
        self.panel.is_some() || self.stack.is_some()
    }
}

pub struct Switcher {
    pub entries: Vec<Entry>,
    pub selected: usize,
    pub mods: u32,
    pub area: Rect,
    pub layout: SwitcherLayout,
    pub tree: TreeId,
    glass_tree: TreeId,
    buffer: BufferId,
    badges: Option<BufferId>,
    pub preview: bool,
    pub thumbnails: bool,
    /// The entry under the pointer, which the title names over the
    /// selected one's; None while the pointer is off every cell, and
    /// cleared when the keys move the selection or the list or the shown
    /// page changes, until the pointer next moves.
    pub hot: Option<usize>,
    /// The blurred backdrop under a glass panel; it shows nothing for
    /// Classic's icon panel, a look without glass or one drawn opaque.
    backdrop: Backdrop,
    /// The desktop entry's wallpaper picture and the rect (device pixels,
    /// panel-relative) it was composed for, so it is composed once while
    /// the cell keeps its place.
    desktop_still: Option<(Rect, Canvas)>,
    thumb: Option<ThumbId>,
    thumbs: Vec<Option<ThumbId>>,
    peeking: bool,
}

impl Switcher {
    /// The layout for `selected` among the current entries; thumbnails go
    /// on `grid`, the look's sized by `[switcher] size`, and icons on `icons` when the look has an
    /// icon grid (no preview well then), else on the fixed icon panel.
    fn compute_layout(
        &self,
        grid: switcher::Grid,
        icons: Option<switcher::Grid>,
    ) -> SwitcherLayout {
        let n = self.entries.len();
        match (self.thumbnails, icons) {
            (true, _) => SwitcherLayout::thumbnails(n, self.selected, self.area, grid),
            (false, Some(icons)) => SwitcherLayout::thumbnails(n, self.selected, self.area, icons),
            (false, None) => SwitcherLayout::compute(n, self.selected, self.area, self.preview),
        }
    }
}

fn input_rect(r: Rect) -> Input {
    Input::Rect(Rectangle::new(
        (r.x, r.y).into(),
        (r.width, r.height).into(),
    ))
}

impl State {
    /// The windows Alt+Tab and `Action::Cycle` pick from, minimized ones
    /// included, in focus order (least recently focused first). Dialogs
    /// are represented by their owner.
    pub fn switchable_windows(&self) -> Vec<WindowId> {
        self.switchable_windows_on(self.desktops.current)
    }

    /// `switchable_windows` as desktop `n` would have them.
    pub fn switchable_windows_on(&self, n: usize) -> Vec<WindowId> {
        self.focus_order
            .iter()
            .copied()
            .filter(|&w| {
                self.win(w)
                    .is_some_and(|x| x.mapped && !x.override_redirect && !x.is_owned())
                    && self.desktop_of(w).is_none_or(|d| d == n)
            })
            .collect()
    }

    pub fn switch_entries(&self) -> Vec<Entry> {
        let mut windows = self.switchable_windows();
        windows.reverse();
        windows.sort_by_key(|&w| self.win(w).is_some_and(|x| x.minimized));
        let windows: Vec<Entry> = windows.into_iter().map(Entry::Window).collect();
        let dialogs: Vec<Entry> = self
            .dialogs
            .views
            .iter()
            .rev()
            .filter(|v| !matches!(v.dialog.style, crate::dialog::Style::Welcome))
            .map(|v| Entry::Dialog(v.id))
            .collect();
        let desktop = self.config.switcher.desktop.then_some(Entry::Desktop);
        switcher::order(&windows, &dialogs, self.dialogs.active, desktop)
    }

    /// Whether a window or a dialog has the keyboard, which makes it entry 0
    /// of `switch_entries`. False when the desktop has it.
    pub fn keyboard_held(&self) -> bool {
        self.focused.is_some() || self.dialog_has_keyboard()
    }

    /// The entry Alt+Tab's panel opens on, from `switch_entries`' length
    /// (`switcher::start`); None when there is nothing to switch to, which
    /// the desktop entry alone is.
    pub fn switcher_start(&self, len: usize) -> Option<usize> {
        switcher::start(len, self.keyboard_held(), self.config.switcher.desktop)
    }

    pub fn open_switcher(&mut self, mods: u32) {
        let entries = self.switch_entries();
        let Some(selected) = self.switcher_start(entries.len()) else {
            return;
        };
        // The panel takes over: a drag or held button under way ends
        // without its release.
        self.cancel_interaction();
        let p = self.pointer_location();
        let out = self.output_at(p.x, p.y);
        if let Some(o) = &out {
            crate::autohideview::keys(self, o, false);
        }
        let area = self.usable_area(out.as_ref());
        let preview = self.config.switcher.preview;
        let thumbnails = self.config.switcher.style == SwitcherStyle::Thumbnails;
        let tree = self.scene.tree_create(self.layers.menu);
        self.scene.set_data(tree, Some(SceneOwner::Switcher));
        crate::glass::leave_out(&mut self.scene, tree);
        let glass_tree = self.scene.tree_create(tree);
        let buffer = self.scene.buffer_create(tree);
        let badges = thumbnails.then(|| self.scene.buffer_create(tree));
        let mut sw = Switcher {
            entries,
            selected,
            mods,
            area,
            layout: SwitcherLayout::compute(0, 0, area, false),
            tree,
            glass_tree,
            buffer,
            badges,
            preview,
            thumbnails,
            hot: None,
            backdrop: Backdrop::new(&self.glass),
            desktop_still: None,
            thumb: None,
            thumbs: Vec::new(),
            peeking: false,
        };
        let theme = &self.config.theme;
        sw.layout = sw.compute_layout(
            interfaces::switcher_grid(theme).sized(self.config.switcher.size),
            interfaces::switcher_icon_grid(theme),
        );
        self.switcher.panel = Some(sw);
        self.redraw_switcher();
        // Peek is one state shared with the taskbar's preview panel and
        // Show desktop strip. Release their hold first, so their later
        // leave cannot end ours and the selection arms a fresh delay.
        crate::taskpreview::hide(self);
        crate::snappickerview::hide(self);
        self.taskbar_hover(None, TaskbarHit::Nothing);
        self.switcher_peek();
        // The keys stay with the switcher, so an open X11 menu closes here.
        crate::xwayland::menus::dismiss(self);
        // A resting pointer over the panel shows its hover box now, and
        // one elsewhere loses the window's pointer focus. Peek follows the
        // selection only, so this starts none.
        let p = self.pointer_location();
        self.repoint(p, InputTime::now());
    }

    pub fn switcher_step(&mut self, backwards: bool) {
        if self.switcher.stack.is_some() {
            self.flip_step(backwards);
            return;
        }
        let Some(sw) = self.switcher.panel.as_mut() else {
            return;
        };
        sw.selected = switcher::step(sw.selected, sw.entries.len(), backwards);
        sw.hot = None;
        self.redraw_switcher();
        self.switcher_peek();
    }

    pub fn switcher_hit(&self, lx: f64, ly: f64) -> Option<usize> {
        if self.switcher.stack.is_some() {
            return self.flip_hit(lx, ly);
        }
        let sw = self.switcher.panel.as_ref()?;
        let r = sw.layout.rect;
        switcher::hit(&sw.layout, lx as i32 - r.x, ly as i32 - r.y)
    }

    pub fn switcher_hover(&mut self, cell: Option<usize>) {
        let Some(sw) = self.switcher.panel.as_mut() else {
            return;
        };
        if sw.hot != cell {
            sw.hot = cell;
            self.redraw_switcher();
        }
    }

    pub fn switcher_click(&mut self, cell: Option<usize>, button: u32) {
        let Some(i) = cell.filter(|_| button == BTN_LEFT) else {
            return;
        };
        if self.switcher.stack.is_some() {
            self.flip_pick(i);
            return;
        }
        match self.switcher.panel.as_mut() {
            Some(p) if i < p.entries.len() => p.selected = i,
            _ => return,
        }
        self.switcher_commit();
    }

    /// A button pressed while a switcher is up, which owns the pointer: a
    /// press on it acts, a press anywhere else cancels it, and neither
    /// reaches a client. Closed on the same client with the bind's keys
    /// still down, their releases reach it no more than when releasing
    /// them commits, so it never reads its Alt as a lone tap.
    pub fn switcher_press(&mut self, button: u32) {
        if !self.switcher.taken.contains(&button) {
            self.switcher.taken.push(button);
        }
        let Some(keyboard) = self.seat.get_keyboard() else {
            return;
        };
        let focused = keyboard.current_focus();
        let pos = self.pointer_location();
        match self.scene_hit(pos) {
            crate::input::Hit::Switcher { cell } => self.switcher_click(cell, button),
            _ => {
                self.switcher_escape();
                self.repoint(pos, InputTime::now());
            }
        }
        if !self.switcher.is_open() && keyboard.current_focus() == focused {
            for k in keyboard.pressed_keys() {
                let k = k.raw();
                if !self.key_input.drag_keys.contains(&k) {
                    self.key_input.drag_keys.push(k);
                }
            }
        }
    }

    pub fn switcher_release(&mut self, button: u32) -> bool {
        let taken = &mut self.switcher.taken;
        let had = taken.contains(&button);
        taken.retain(|&b| b != button);
        had
    }

    pub fn switcher_modifiers(&mut self) {
        let sw = &self.switcher;
        let Some(mods) = (sw.panel.as_ref().map(|p| p.mods)).or(sw.stack.as_ref().map(|s| s.mods))
        else {
            return;
        };
        let held = self
            .seat
            .get_keyboard()
            .map_or(0, |k| keys::mask(&k.modifier_state()));
        if held & mods == 0 {
            self.switcher_commit();
        }
    }

    fn relayout_switcher(&mut self) {
        let grid = interfaces::switcher_grid(&self.config.theme).sized(self.config.switcher.size);
        let icons = interfaces::switcher_icon_grid(&self.config.theme);
        let Some(sw) = self.switcher.panel.as_mut() else {
            return;
        };
        let first = sw.layout.first;
        sw.layout = sw.compute_layout(grid, icons);
        if sw.layout.first != first {
            sw.hot = None;
        }
        let input = sw.layout.input_area();
        for b in std::iter::once(sw.buffer).chain(sw.badges) {
            self.scene.set_input(b, input_rect(input));
        }
        let r = sw.layout.rect;
        self.scene.set_position(sw.tree, r.x, r.y);
    }

    pub fn switcher_commit(&mut self) {
        if let Some(sw) = self.switcher.panel.take() {
            let entry = sw.entries.get(sw.selected).copied();
            self.destroy_switcher_tree(sw);
            self.switch_to(entry);
        } else if let Some(st) = self.switcher.stack.take() {
            // Released before it showed: nothing switches.
            if !st.revealed() {
                self.destroy_flip(st);
            } else {
                self.switch_to(st.entries.get(st.selected).copied());
                let picked = st.selected;
                self.flip_leave(st, Some(picked));
            }
        } else {
            return;
        }
        // The window under a resting pointer takes it back, as it lost
        // it when the switcher opened.
        let p = self.pointer_location();
        self.repoint(p, InputTime::now());
    }

    fn switch_to(&mut self, entry: Option<Entry>) {
        match entry {
            Some(Entry::Window(w)) if self.win(w).is_some_and(|x| x.minimized) => {
                self.set_minimized(w, false)
            }
            Some(Entry::Window(w)) => self.focus_window(w),
            Some(Entry::Dialog(id)) => self.focus_dialog(id),
            // Only ever minimizes: picking the desktop never restores.
            // The panel is gone already, so the minimizes find no
            // switcher to forget from.
            Some(Entry::Desktop) => self.show_desktop_now(),
            None => {}
        }
    }

    /// Close the switcher without changing focus, at once: the 3D one and
    /// one flying back too. No-op if neither is up. The callers then
    /// `repoint`, `switcher_forget` on the next idle pass.
    pub fn switcher_cancel(&mut self) {
        if let Some(sw) = self.switcher.panel.take() {
            self.destroy_switcher_tree(sw);
        }
        if let Some(st) = self.switcher.stack.take() {
            self.destroy_flip(st);
        }
        if let Some(st) = self.switcher.leaving.take() {
            self.destroy_flip(st);
        }
    }

    pub fn switcher_escape(&mut self) {
        if let Some(st) = self.switcher.stack.take() {
            if st.revealed() {
                self.flip_leave(st, None);
            } else {
                self.destroy_flip(st);
            }
            return;
        }
        self.switcher_cancel();
    }

    fn destroy_switcher_tree(&mut self, sw: Switcher) {
        if sw.peeking {
            crate::peek::leave(self);
        }
        for id in sw.thumb.into_iter().chain(sw.thumbs.into_iter().flatten()) {
            self.thumb_drop(id);
        }
        self.scene.destroy(sw.tree);
        self.redraw_all();
        crate::access::touch(self);
        crate::autohideview::poke(self);
    }

    /// Peek at the selected entry while `[switcher] peek` is on: its window
    /// alone, every window off for the desktop, and no peek for a dialog.
    /// Hover never calls this: peek follows the selection only.
    fn switcher_peek(&mut self) {
        if !self.config.switcher.peek {
            return;
        }
        let Some(sw) = self.switcher.panel.as_mut() else {
            return;
        };
        match sw.entries.get(sw.selected).and_then(peek_target) {
            Some(t) => {
                sw.peeking = true;
                crate::peek::arm(self, t);
            }
            None if sw.peeking => {
                sw.peeking = false;
                crate::peek::leave(self);
            }
            None => {}
        }
    }

    fn show_desktop_now(&mut self) {
        for w in self.task_windows() {
            if self.win(w).is_some_and(|x| !x.hidden()) {
                self.set_minimized(w, true);
            }
        }
    }

    /// Drop `entry` from the open switcher's list (called as a window
    /// unmaps or leaves the desktop, and as a dialog closes), cancelling the panel
    /// outright once fewer than two entries remain. A changed list drops
    /// the hovered entry, whose index may now name another. A cancel hands
    /// the pointer back on the next idle pass: the callers are
    /// mid-teardown, with the leaving window still in the scene.
    pub fn switcher_forget(&mut self, entry: Entry) {
        if self.flip_forget(entry) {
            let left = self.switcher.stack.as_ref().map_or(0, |s| s.entries.len());
            if left < 2 {
                self.switcher_cancel();
                self.handle.insert_idle(|state| {
                    let p = state.pointer_location();
                    state.repoint(p, InputTime::now());
                });
            } else {
                self.flip_moved();
            }
            return;
        }
        let Some(sw) = self.switcher.panel.as_mut() else {
            return;
        };
        let len = sw.entries.len();
        sw.entries.retain(|&e| e != entry);
        if sw.entries.len() != len {
            sw.hot = None;
        }
        if sw.entries.len() < 2 {
            self.switcher_cancel();
            self.handle.insert_idle(|state| {
                let p = state.pointer_location();
                state.repoint(p, InputTime::now());
            });
        } else {
            sw.selected = sw.selected.min(sw.entries.len() - 1);
            self.redraw_switcher();
            self.switcher_peek();
        }
    }

    pub fn redraw_switcher(&mut self) {
        match self.switcher.panel.as_ref() {
            // A look's icon grid draws as its thumbnail panel, without
            // pictures.
            Some(sw) if sw.thumbnails || sw.layout.grid.is_some() => {
                self.redraw_switcher_thumbnails()
            }
            Some(_) => self.redraw_switcher_icons(),
            None => {}
        }
        crate::access::touch(self);
    }

    fn redraw_switcher_icons(&mut self) {
        self.relayout_switcher();
        let Some(sw) = self.switcher.panel.as_ref() else {
            return;
        };
        let r = sw.layout.rect;
        let out = self.output_at(f64::from(r.x + r.width / 2), f64::from(r.y + r.height / 2));
        let scale = self.draw_scale_at(out.as_ref());
        let theme = self.config.theme.scaled(scale);
        let px = switcher::ICON_PX * scale;
        let first = sw.layout.first;
        let shown = sw.entries[first..first + sw.layout.cells.len()].to_vec();
        let selected = sw.entries.get(sw.selected).copied();
        let named = sw.entries.get(sw.hot.unwrap_or(sw.selected)).copied();
        let well = sw.layout.preview.is_some();
        let icons: Vec<Option<Canvas>> = shown.iter().map(|&e| self.switcher_icon(e, px)).collect();
        let title = named.map_or_else(String::new, |e| self.entry_title(e));
        let well_window = match selected {
            Some(Entry::Window(w)) if self.has_picture(w) => Some(w),
            _ => None,
        };
        self.place_switcher_thumb(well_window, scale);
        let has_thumb = self
            .switcher
            .panel
            .as_ref()
            .is_some_and(|sw| sw.thumb.is_some());
        let well_icon = match (well && !has_thumb, selected) {
            (true, Some(e)) => self.switcher_icon(e, px),
            _ => None,
        };
        let Some(sw) = self.switcher.panel.as_ref() else {
            return;
        };
        let buffer = sw.buffer;
        let layout = sw.layout.scaled(scale);
        let mut canvas = Canvas::new(
            layout.rect.width.max(1) as u32,
            layout.rect.height.max(1) as u32,
        );
        switcher::draw(
            &mut canvas,
            &theme,
            &self.text,
            &layout,
            &title,
            &icons,
            sw.selected,
            sw.hot,
            well_icon.as_ref(),
        );
        upload_scaled(&mut self.scene, buffer, &canvas, scale);
        self.redraw_all();
    }

    fn redraw_switcher_thumbnails(&mut self) {
        self.relayout_switcher();
        let wanted = glass::wanted(&self.config);
        let Some(sw) = self.switcher.panel.as_mut() else {
            return;
        };
        let r = sw.layout.rect;
        let pane = sw.layout.grid.filter(|_| wanted).map(|grid| Pane {
            shapes: vec![PaneShape {
                rect: Rect::new(0, 0, r.width, r.height),
                corner: grid.corner,
                square_bottom: false,
                square_top: false,
            }],
            holes: Vec::new(),
        });
        let glass_tree = sw.glass_tree;
        sw.backdrop
            .set(&mut self.scene, &self.glass, glass_tree, pane);
        let Some(sw) = self.switcher.panel.as_ref() else {
            return;
        };
        let out = self.output_at(f64::from(r.x + r.width / 2), f64::from(r.y + r.height / 2));
        let scale = self.draw_scale_at(out.as_ref());
        let theme = self.config.theme.scaled(scale);
        let px = switcher::ICON_PX * scale;
        let first = sw.layout.first;
        let shown = sw.entries[first..first + sw.layout.cells.len()].to_vec();
        let named = sw.entries.get(sw.hot.unwrap_or(sw.selected)).copied();
        let pictured = sw.thumbnails;
        let ob = self.layout_box(out.as_ref());
        let desktop = shown
            .iter()
            .position(|&e| pictured && e == Entry::Desktop)
            .and_then(|k| {
                let well = sw.layout.wells[k];
                let pic = fit(ob.width, ob.height, well, 1).scaled(scale);
                (!pic.is_empty()).then_some((k, pic))
            });
        let stale = desktop
            .is_some_and(|(_, pic)| sw.desktop_still.as_ref().is_none_or(|(at, _)| *at != pic));
        let icons: Vec<Option<Canvas>> = shown.iter().map(|&e| self.switcher_icon(e, px)).collect();
        let title = named.map_or_else(String::new, |e| self.entry_title(e));
        let windows: Vec<Option<WindowId>> = shown
            .iter()
            .map(|&e| match e {
                Entry::Window(w) if pictured && self.has_picture(w) => Some(w),
                _ => None,
            })
            .collect();
        self.place_switcher_thumbs(&windows, scale);
        if let (true, Some((_, pic))) = (stale, desktop) {
            let still = self.wallpaper_thumbnail(out.as_ref(), pic.width, pic.height);
            if let Some(sw) = self.switcher.panel.as_mut() {
                sw.desktop_still = Some((pic, still));
            }
        }
        let Some(sw) = self.switcher.panel.as_ref() else {
            return;
        };
        let layout = sw.layout.scaled(scale);
        // Pictures are fitted to the logical well and scaled, as
        // `livepreview` draws them, so the badges land on their corners.
        // A picture that could not be made gets the icon in its well.
        let cells: Vec<ThumbCell> = (0..layout.cells.len())
            .map(|k| {
                let window = windows[k].and_then(|w| self.win(w));
                let (picture, still) = match (sw.thumbs.get(k), window, desktop) {
                    (Some(Some(_)), Some(x), _) => {
                        let pic = fit(x.client_w, x.client_h, sw.layout.wells[k], 1);
                        (Some(pic.scaled(scale)), None)
                    }
                    (_, _, Some((d, pic))) if d == k => {
                        (Some(pic), sw.desktop_still.as_ref().map(|(_, c)| c))
                    }
                    _ => (None, None),
                };
                ThumbCell {
                    icon: icons[k].as_ref(),
                    picture,
                    still,
                }
            })
            .collect();
        // The canvas reaches past the panel by the look's shadow; the
        // buffers sit where it starts, relative to the panel.
        let area = layout.canvas();
        let at = sw.layout.canvas();
        let (w, h) = (area.width.max(1) as u32, area.height.max(1) as u32);
        let mut canvas = Canvas::new(w, h);
        interfaces::draw_switcher(
            &mut canvas,
            &theme,
            &self.text,
            &layout,
            &title,
            &cells,
            sw.selected,
            sw.hot,
        );
        let mut over = Canvas::new(w, h);
        interfaces::draw_switcher_badges(&mut over, &theme, &layout, &cells);
        let (buffer, badges) = (sw.buffer, sw.badges);
        self.scene.set_position(buffer, at.x, at.y);
        upload_scaled(&mut self.scene, buffer, &canvas, scale);
        if let Some(badges) = badges {
            self.scene.set_position(badges, at.x, at.y);
            upload_scaled(&mut self.scene, badges, &over, scale);
            self.scene.raise_to_top(badges);
        }
        self.redraw_all();
    }

    fn place_switcher_thumb(&mut self, window: Option<WindowId>, scale: i32) {
        let Some(sw) = self.switcher.panel.as_ref() else {
            return;
        };
        let (tree, well, old) = (sw.tree, sw.layout.preview, sw.thumb);
        let keep = match (old, window, well) {
            (Some(id), Some(w), Some(well)) => self.thumb_shows(id, w, well, scale),
            _ => false,
        };
        if keep {
            return;
        }
        if let Some(id) = old {
            self.thumb_drop(id);
        }
        let thumb = match (window, well) {
            (Some(w), Some(well)) => self.thumb_create(w, tree, well, scale),
            _ => None,
        };
        if let Some(sw) = self.switcher.panel.as_mut() {
            sw.thumb = thumb;
        }
    }

    fn place_switcher_thumbs(&mut self, windows: &[Option<WindowId>], scale: i32) {
        let Some(sw) = self.switcher.panel.as_mut() else {
            return;
        };
        let tree = sw.tree;
        let wells = sw.layout.wells.clone();
        let mut old = std::mem::take(&mut sw.thumbs);
        let mut thumbs = Vec::with_capacity(windows.len());
        for (&window, &well) in windows.iter().zip(&wells) {
            let Some(w) = window else {
                thumbs.push(None);
                continue;
            };
            let kept = old
                .iter()
                .position(|t| t.is_some_and(|id| self.thumb_shows(id, w, well, scale)))
                .and_then(|i| old[i].take());
            thumbs.push(kept.or_else(|| self.thumb_create(w, tree, well, scale)));
        }
        // The pictures no cell kept go before the list is put back.
        for id in old.into_iter().flatten() {
            self.thumb_drop(id);
        }
        if let Some(sw) = self.switcher.panel.as_mut() {
            sw.thumbs = thumbs;
        }
    }

    pub(crate) fn entry_title(&self, e: Entry) -> String {
        match e {
            Entry::Window(w) => self.win(w).map(|x| x.title()).unwrap_or_default(),
            Entry::Desktop => "Desktop".into(),
            Entry::Dialog(id) => self
                .dialogs
                .views
                .iter()
                .find(|v| v.id == id)
                .map(|v| v.dialog.title.clone())
                .unwrap_or_default(),
        }
    }

    pub fn switcher_access(&self) -> Option<reprisekit::access::Tree> {
        use reprisekit::access::{ROOT, accesskit::Role};
        if self.switcher.stack.is_some() {
            return self.flip_access();
        }
        let sw = self.switcher.panel.as_ref()?;
        let titles: Vec<String> = sw.entries.iter().map(|&e| self.entry_title(e)).collect();
        let mut tree = crate::access::root(Role::Window, "Task Switcher", sw.layout.rect);
        let focus = switcher::access(&titles, sw.selected, &sw.layout, &mut tree, ROOT);
        tree.set_focus(focus);
        Some(tree)
    }

    fn switcher_icon(&mut self, e: Entry, px: i32) -> Option<Canvas> {
        match e {
            Entry::Window(w) => self.window_icon(w, px),
            Entry::Dialog(id) => self
                .dialogs
                .views
                .iter()
                .find(|v| v.id == id)
                .and_then(|v| v.dialog.icon.clone())
                .and_then(|name| self.icons.get(&name, px)),
            Entry::Desktop => self.icons.get("user-desktop", px),
        }
    }
}
