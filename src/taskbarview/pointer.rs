use std::{sync::Arc, time::Instant};

use reprisescene::{Input, TreeId};
use smithay::output::Output;

use super::{DRAG_PX, quick_icon};
use crate::{
    compositor::State,
    input::{BTN_LEFT, BTN_MIDDLE, BTN_RIGHT, Hit, Interaction},
    menuview::MenuRoot,
    output::upload_scaled,
    quicklaunch::QuickItem,
    taskbar::{TaskKey, TaskbarHit},
    taskslide,
    window::WindowId,
};

impl State {
    pub fn taskbar_hover(&mut self, tree: Option<TreeId>, hit: TaskbarHit) {
        let area = tree.filter(|&t| {
            matches!(hit, TaskbarHit::Tray(_) | TaskbarHit::Chevron)
                || self.bar(t).is_some_and(|b| {
                    let p = self.pointer_location();
                    let (x, y) = self.bar_local(b, p.x, p.y);
                    b.layout.notification.contains(x, y)
                })
        });
        crate::trayreveal::hover(self, area);
        let none = (false, None, None, None, false, None, false);
        let (fade_in, fade_out) = self.config.effects.hover_ms();
        let now = Instant::now();
        let hot_x = match (hit, tree.and_then(|t| self.bar(t))) {
            (TaskbarHit::Task(i), Some(bar)) => self.task_pointer_x(bar, i),
            _ => None,
        };
        let mut changed = Vec::new();
        let mut moved = Vec::new();
        let (mut strip_was, mut strip_now) = (false, false);
        for bar in &mut self.bars {
            bar.task_hot_x = if Some(bar.tree) == tree { hot_x } else { None };
            let want = if Some(bar.tree) != tree {
                none
            } else {
                match hit {
                    TaskbarHit::Start => (true, None, None, None, false, None, false),
                    TaskbarHit::Quick(i) => (false, Some(i), None, None, false, None, false),
                    TaskbarHit::Pager(i) => (false, None, Some(i), None, false, None, false),
                    TaskbarHit::Task(i) => {
                        let key =
                            crate::taskbar::task_key(&bar.buttons, &bar.pins, &bar.dialogs, i);
                        (false, None, None, key, false, None, false)
                    }
                    TaskbarHit::ShowDesktop => (false, None, None, None, true, None, false),
                    TaskbarHit::More | TaskbarHit::QuickMore => {
                        (false, None, None, None, false, Some(hit), false)
                    }
                    TaskbarHit::Chevron => (false, None, None, None, false, None, true),
                    _ => none,
                }
            };
            let was = (
                bar.start_hot,
                bar.quick_hot,
                bar.pager_hot,
                bar.task_hot,
                bar.show_desktop_hot,
                bar.more_hot,
                bar.chevron_hot,
            );
            strip_was |= was.4;
            strip_now |= want.4;
            if was != want {
                (
                    bar.start_hot,
                    bar.quick_hot,
                    bar.pager_hot,
                    bar.task_hot,
                    bar.show_desktop_hot,
                    bar.more_hot,
                    bar.chevron_hot,
                ) = want;
                bar.start_glow.set(want.0, now, fade_in, fade_out);
                changed.push(bar.tree);
            } else {
                moved.push(bar.tree);
            }
        }
        for t in changed {
            self.redraw_taskbar(t);
            self.watch_bar(t);
        }
        for t in moved {
            self.spot_moved(t);
        }
        if strip_was != strip_now {
            crate::peek::show_desktop_hover(self, strip_now);
        }
    }

    /// Hover the bars afresh where the pointer rests: their hot cells are
    /// the last layout's, and a bar moved or relaid under a still pointer
    /// would keep one lit until the next motion.
    pub(crate) fn rehover_bars(&mut self) {
        if self.pointer_free() {
            let p = self.pointer_location();
            self.repoint(p, smithay::backend::input::InputTime::now());
        } else {
            self.taskbar_hover(None, TaskbarHit::Nothing);
        }
    }

    pub fn taskbar_press(
        &mut self,
        tree: TreeId,
        hit: TaskbarHit,
        button: u32,
        at: (i32, i32),
        shift: bool,
        closed: Option<MenuRoot>,
    ) {
        let pinned = self.config.taskbar.pinned;
        match hit {
            TaskbarHit::Task(i) => {
                if let Some(id) = self.task_button_dialog(tree, i) {
                    if button == BTN_LEFT {
                        self.task_dialog_click(id);
                    }
                    return;
                }
                if pinned && button == BTN_MIDDLE {
                    self.task_button_launch(tree, i);
                    return;
                }
                if button == BTN_RIGHT {
                    self.task_button_menu(tree, i, at, shift, closed);
                    return;
                }
                if button != BTN_LEFT {
                    return;
                }
                match crate::taskbar::press_timing(pinned) {
                    crate::taskbar::PressTiming::OnRelease => {
                        let p = self.pointer_location();
                        let held = self.bar(tree).and_then(|b| {
                            let key = crate::taskbar::task_key(&b.buttons, &b.pins, &b.dialogs, i)?;
                            let slot = b.layout.tasks.get(i).map_or(0, |r| b.layout.to_along(*r).x);
                            let grab = self.bar_along(b, f64::from(at.0), f64::from(at.1)) - slot;
                            Some((key, grab, b.layout.along_f(p.x, p.y)))
                        });
                        if let Some((key, grab, along0)) = held {
                            self.interaction = Interaction::TaskPress {
                                bar: tree,
                                key,
                                along0,
                                grab,
                                dragging: false,
                            };
                            self.redraw_taskbar(tree);
                        }
                    }
                    crate::taskbar::PressTiming::OnPress => {
                        let list_was_open = closed == Some(MenuRoot::WindowList(i));
                        self.task_button_click(tree, i, list_was_open);
                    }
                }
            }
            TaskbarHit::Start if button == BTN_LEFT => self.toggle_start(tree),
            TaskbarHit::Quick(index) if button == BTN_LEFT => {
                let icon_px = self.config.theme.icon_size();
                let grab = self.bar(tree).map_or((0, 0), |b| {
                    let Some(r) = b.layout.quick.get(index) else {
                        return (0, 0);
                    };
                    let (ix, iy) = crate::taskbar::quick_icon_origin(*r, icon_px, 1);
                    let (lx, ly) = self.bar_local(b, f64::from(at.0), f64::from(at.1));
                    (lx - ix, ly - iy)
                });
                let p = self.pointer_location();
                self.interaction = Interaction::QuickPress {
                    bar: tree,
                    index,
                    p0: (p.x, p.y),
                    grab,
                    dragging: false,
                };
                if let Some(b) = self.bar_mut(tree) {
                    b.quick_pressed = Some(index);
                }
                self.redraw_taskbar(tree);
            }
            TaskbarHit::Quick(index) if button == BTN_RIGHT => self.open_quick_menu(index, at),
            TaskbarHit::Nothing | TaskbarHit::Resize | TaskbarHit::Clock | TaskbarHit::Pager(_)
                if button == BTN_RIGHT =>
            {
                self.open_taskbar_menu(at)
            }
            TaskbarHit::ShowDesktop if button == BTN_LEFT => self.toggle_show_desktop(),
            TaskbarHit::More if button == BTN_LEFT => {
                if closed != Some(MenuRoot::TaskOverflow(tree)) {
                    self.open_task_overflow(tree);
                }
            }
            TaskbarHit::QuickMore if button == BTN_LEFT => {
                if closed != Some(MenuRoot::QuickOverflow(tree)) {
                    self.open_quick_overflow(tree);
                }
            }
            TaskbarHit::Pager(i) if button == BTN_LEFT => self.switch_desktop(i),
            TaskbarHit::Grip(i) if button == BTN_LEFT => {
                let Some(b) = self.bar(tree) else { return };
                let Some(g) = b.layout.grips.get(i) else {
                    return;
                };
                let dx =
                    self.bar_along(b, f64::from(at.0), f64::from(at.1)) - b.layout.to_along(*g).x;
                let band = b.layout.band_at_grip(i);
                let edge = b.layout.edge;
                self.interaction = Interaction::Band {
                    bar: tree,
                    band,
                    dx,
                };
                self.set_cursor(crate::taskbar::grip_cursor(edge));
            }
            TaskbarHit::Nothing if button == BTN_LEFT && !self.bar_shared.locked => {
                self.begin_bar_move(tree)
            }
            TaskbarHit::Resize if button == BTN_LEFT => self.begin_bar_width(tree),
            _ => {}
        }
    }

    fn task_button_dialog(&self, tree: TreeId, i: usize) -> Option<u64> {
        self.bar(tree)?.dialogs.get(i).copied().flatten()
    }

    fn task_button_menu(
        &mut self,
        tree: TreeId,
        i: usize,
        at: (i32, i32),
        shift: bool,
        closed: Option<MenuRoot>,
    ) {
        if self.config.taskbar.pinned && !shift {
            if closed != Some(MenuRoot::JumpList(i)) {
                self.open_jump_list(tree, i, at);
            }
            return;
        }
        let Some(wins) = self.bar(tree).and_then(|b| b.buttons.get(i).cloned()) else {
            return;
        };
        match wins.as_slice() {
            [] => {}
            [w] => {
                if closed != Some(MenuRoot::Window(*w)) {
                    self.open_window_menu(*w, at, true);
                }
            }
            _ => {
                if closed != Some(MenuRoot::WindowList(i)) {
                    self.open_window_list(tree, i);
                }
            }
        }
    }

    pub fn taskbar_press_moved(&mut self) {
        let p = self.pointer_location();
        match self.interaction {
            Interaction::Band {
                bar: tree,
                band,
                dx,
            } => {
                let Some(b) = self.bar(tree) else { return };
                let at = self.bar_along(b, p.x, p.y);
                let next = b.layout.drag_band(self.bar_shared.bands, band, at - dx);
                if next != self.bar_shared.bands {
                    self.bar_shared.bands = next;
                    self.redraw_taskbars();
                }
            }
            Interaction::TaskPress {
                bar: tree,
                key,
                along0,
                grab,
                dragging,
            } => {
                if !dragging && let Some(b) = self.bar(tree) {
                    let held = self.held_task(b);
                    let x = held.and_then(|i| self.task_pointer_x(b, i));
                    let changed = held != b.task_held;
                    if let Some(b) = self.bar_mut(tree) {
                        b.task_hot_x = x;
                    }
                    if changed {
                        self.redraw_taskbar(tree);
                    } else {
                        self.spot_moved(tree);
                    }
                }
                let Some((button, along, at)) = self.bar(tree).and_then(|b| {
                    let i = crate::taskbar::find_task(&b.buttons, &b.pins, &b.dialogs, &key)?;
                    Some((i, b.layout.along_f(p.x, p.y), self.bar_along(b, p.x, p.y)))
                }) else {
                    return;
                };
                let moved = (along - along0).abs() >= DRAG_PX;
                if !crate::taskbar::press_drags(dragging, moved) {
                    return;
                }
                let button = self.row_drag(tree, button, at);
                let key = self
                    .bar(tree)
                    .and_then(|b| crate::taskbar::task_key(&b.buttons, &b.pins, &b.dialogs, button))
                    .unwrap_or(key);
                self.interaction = Interaction::TaskPress {
                    bar: tree,
                    key,
                    along0,
                    grab,
                    dragging: true,
                };
                self.slide_follow(tree, key, grab);
                if !dragging {
                    if let Some(b) = self.bar_mut(tree) {
                        b.task_hot_x = None;
                    }
                    self.redraw_taskbar(tree);
                }
            }
            Interaction::ArrowPress { bar } => crate::trayreveal::press_moved(self, bar),
            Interaction::QuickPress {
                bar: tree,
                index,
                p0,
                grab,
                dragging,
            } => {
                let Some(b) = self.bar(tree) else { return };
                let moved = b.layout.quick_drag(p.x - p0.0, p.y - p0.1);
                if !dragging && (self.bar_shared.locked || moved < DRAG_PX) {
                    let want =
                        (self.bar_hit(b, p.x, p.y) == TaskbarHit::Quick(index)).then_some(index);
                    if b.quick_pressed != want {
                        if let Some(b) = self.bar_mut(tree) {
                            b.quick_pressed = want;
                        }
                        self.redraw_taskbar(tree);
                    }
                    return;
                }
                let (lx, ly) = self.bar_local(b, p.x, p.y);
                let slot = b.layout.drop_slot(lx, ly);
                let output = b.output.clone();
                self.interaction = Interaction::QuickPress {
                    bar: tree,
                    index,
                    p0,
                    grab,
                    dragging: true,
                };
                if !dragging {
                    self.quick_ghost_start(&output, index);
                }
                if let Some(ghost) = self.bar_shared.quick_ghost {
                    self.scene
                        .set_position(ghost, p.x as i32 - grab.0, p.y as i32 - grab.1);
                    self.redraw_all();
                }
                let Some(b) = self.bar_mut(tree) else { return };
                if b.quick_pressed.is_some() || b.quick_drop != Some(slot) || b.quick_hot.is_some()
                {
                    b.quick_pressed = None;
                    b.quick_hot = None;
                    b.quick_drop = Some(slot);
                    self.redraw_taskbar(tree);
                }
            }
            _ => {}
        }
    }

    pub fn end_taskbar_press(&mut self) {
        match self.interaction {
            Interaction::Band { .. } => {
                self.interaction = Interaction::Idle;
                self.save_taskbar_state();
            }
            Interaction::TaskPress {
                bar: tree,
                dragging: true,
                ..
            } => {
                self.interaction = Interaction::Idle;
                self.slide_release(tree);
                self.quick_store_order();
            }
            Interaction::TaskPress { bar: tree, .. } => {
                let held = self.bar(tree).and_then(|b| self.held_task(b));
                self.interaction = Interaction::Idle;
                self.redraw_taskbar(tree);
                if let Some(i) = held {
                    self.task_button_click(tree, i, false);
                }
            }
            Interaction::QuickPress {
                bar: tree,
                index,
                dragging,
                ..
            } => {
                self.interaction = Interaction::Idle;
                self.quick_ghost_end();
                let p = self.pointer_location();
                let on = self
                    .bar(tree)
                    .is_some_and(|b| self.bar_hit(b, p.x, p.y) == TaskbarHit::Quick(index));
                let slot = self.bar_mut(tree).and_then(|b| {
                    b.quick_pressed = None;
                    b.quick_drop.take()
                });
                self.redraw_taskbar(tree);
                if dragging {
                    if let Some(slot) = slot {
                        self.quick_move(index, slot);
                    }
                } else if on {
                    self.run_quick(index);
                }
            }
            Interaction::ArrowPress { bar } => {
                self.interaction = Interaction::Idle;
                crate::trayreveal::release(self, bar);
            }
            _ => {}
        }
    }

    pub fn cancel_taskbar_press(&mut self, was: Interaction) {
        match was {
            Interaction::TaskPress {
                bar: tree,
                dragging: true,
                ..
            } => {
                self.slide_release(tree);
                self.quick_store_order();
            }
            Interaction::TaskPress { bar: tree, .. } => self.redraw_taskbar(tree),
            Interaction::QuickPress { bar: tree, .. } => {
                self.quick_ghost_end();
                if let Some(b) = self.bar_mut(tree) {
                    b.quick_pressed = None;
                    b.quick_drop = None;
                }
                self.redraw_taskbar(tree);
            }
            Interaction::ArrowPress { bar } => crate::trayreveal::set_held(self, bar, false),
            Interaction::Band { .. } => self.save_taskbar_state(),
            _ => {}
        }
    }

    pub fn save_taskbar_state(&self) {
        let s = &self.bar_shared;
        crate::state::save_taskbar(s.locked, s.bands, s.quick_shown);
    }

    pub fn toggle_taskbar_lock(&mut self) {
        self.bar_shared.locked = !self.bar_shared.locked;
        self.save_taskbar_state();
        self.redraw_taskbars();
    }

    pub fn toggle_quick_shown(&mut self) {
        self.bar_shared.quick_shown = !self.bar_shared.quick_shown;
        self.save_taskbar_state();
        self.redraw_taskbars();
    }

    fn row_drag(&mut self, tree: TreeId, i: usize, at: i32) -> usize {
        let Some(bar) = self.bar(tree) else { return i };
        let Some(key) = crate::taskbar::task_key(&bar.buttons, &bar.pins, &bar.dialogs, i) else {
            return i;
        };
        let slot = crate::taskbar::row_drop_slot(
            &bar.layout.along_rects(&bar.layout.tasks),
            &bar.pins,
            &bar.runs,
            i,
            at,
        );
        let Some((order, anchors)) = crate::taskbar::reorder_row(&bar.pins, &bar.runs, i, slot)
        else {
            return i;
        };
        let before = taskslide::row(bar);
        // The buttons are from the last draw; a window or dialog may have closed.
        let moved: Vec<(Vec<WindowId>, Option<u64>, usize)> = anchors
            .iter()
            .map(|(u, anchor)| {
                let wins = u
                    .clone()
                    .filter_map(|b| bar.buttons.get(b))
                    .flatten()
                    .copied()
                    .collect();
                (wins, bar.dialogs.get(u.start).copied().flatten(), *anchor)
            })
            .collect();
        if order.iter().enumerate().any(|(n, &k)| n != k) {
            let mut old: Vec<Option<QuickItem>> = std::mem::take(&mut self.quicklaunch)
                .into_iter()
                .map(Some)
                .collect();
            self.quicklaunch = order.iter().filter_map(|&k| old[k].take()).collect();
            self.bar_shared.quick_moved = true;
            self.pins_reordered(&order);
        }
        let live = self.task_windows();
        for (wins, dialog, anchor) in moved {
            let seq = crate::taskbar::next_seq();
            // Each window its own seq, in row order, so a run keeps its order.
            for (n, w) in wins.into_iter().filter(|w| live.contains(w)).enumerate() {
                let seq = if n == 0 {
                    seq
                } else {
                    crate::taskbar::next_seq()
                };
                if let Some(win) = self.win_mut(w) {
                    win.anchor = anchor;
                    win.seq = seq;
                }
            }
            if let Some(v) =
                dialog.and_then(|id| self.dialogs.views.iter_mut().find(|v| v.id == id))
            {
                v.anchor = anchor;
                v.seq = seq;
            }
        }
        self.redraw_taskbars();
        // A pin's index is its place in the old order; name it by the new one.
        let Some(key) = crate::taskbar::pin_renamed(key, &order) else {
            return i;
        };
        self.slide_row_moved(tree, &before, &order, key);
        self.bar(tree)
            .and_then(|b| crate::taskbar::find_task(&b.buttons, &b.pins, &b.dialogs, &key))
            .unwrap_or(i)
    }

    fn pins_reordered(&mut self, order: &[usize]) {
        for b in &mut self.bars {
            crate::taskbar::pins_renamed(
                &mut b.task_hot,
                &mut b.task_float,
                &mut b.task_slides,
                order,
            );
        }
        if let Interaction::TaskPress { key, .. } = &mut self.interaction
            && let Some(k) = crate::taskbar::pin_renamed(*key, order)
        {
            *key = k;
        }
    }

    /// The callers redraw the bars afterwards.
    pub fn end_task_press_for(&mut self, w: WindowId) {
        let Interaction::TaskPress {
            key: TaskKey::Window(x),
            dragging,
            ..
        } = self.interaction
        else {
            return;
        };
        if x != w {
            return;
        }
        self.interaction = Interaction::Idle;
        if dragging {
            for b in &mut self.bars {
                taskslide::forget(b, TaskKey::Window(w));
            }
            self.quick_store_order();
        }
    }

    fn quick_move(&mut self, from: usize, slot: usize) {
        if crate::taskbar::move_button(&mut self.quicklaunch, from, slot) {
            self.bar_shared.quick_moved = true;
            self.quick_store_order();
            self.redraw_taskbars();
        }
    }

    /// The folder changed under us (`quickwatch`): take the buttons and
    /// the order file as they are now (`quicklaunch::reread`), keeping a
    /// move of our own not yet written.
    pub fn quick_reread(&mut self, dir: &std::path::Path) {
        let items = crate::quicklaunch::reread(
            dir,
            &self.config.terminal,
            &self.quicklaunch,
            self.bar_shared.quick_moved,
        );
        if items == self.quicklaunch {
            return;
        }
        // The held index may name another button now: let go, dropping nothing.
        if matches!(self.interaction, Interaction::QuickPress { .. }) {
            self.cancel_interaction();
        }
        self.quicklaunch = items;
        self.redraw_taskbars();
    }

    /// Write the Quick Launch order to the folder's order file after a
    /// move of ours, on the state writer's thread since it fsyncs; the
    /// order file is otherwise left to whoever wrote it last.
    pub fn quick_store_order(&mut self) {
        if !std::mem::take(&mut self.bar_shared.quick_moved) {
            return;
        }
        let Some(dir) = reprisefs::quicklaunch::dir() else {
            return;
        };
        let order: Vec<String> = self.quicklaunch.iter().map(|q| q.file.clone()).collect();
        crate::state::run_in_order(move || {
            if let Err(e) = reprisefs::quicklaunch::write_order(&dir, &order) {
                log::warn!("quicklaunch: could not write the order: {e}");
            }
        });
    }

    pub fn quick_order_changed(&mut self) {
        self.bar_shared.quick_moved = true;
        self.quick_store_order();
    }

    fn quick_ghost_start(&mut self, output: &Output, index: usize) {
        self.quick_ghost_end();
        let scale = self.draw_scale_at(Some(output)).max(1);
        let theme = self.config.theme.scaled(scale);
        let icon_px = theme.icon_size();
        let Some(item) = self.quicklaunch.get(index) else {
            return;
        };
        let mut canvas = quick_icon(&mut self.icons, item, icon_px).map_or_else(
            || reprisekit::icon::chrome_fallback(&theme, icon_px),
            Arc::unwrap_or_clone,
        );
        canvas.fade(1, 2);
        let buffer = self.scene.buffer_create(self.layers.overlay);
        self.scene.set_input(buffer, Input::Passthrough);
        upload_scaled(&mut self.scene, buffer, &canvas, scale);
        self.bar_shared.quick_ghost = Some(buffer);
    }

    fn quick_ghost_end(&mut self) {
        if let Some(b) = self.bar_shared.quick_ghost.take() {
            self.scene.destroy(b);
            self.redraw_all();
        }
    }

    pub fn task_button_click(&mut self, tree: TreeId, i: usize, list_was_open: bool) {
        if let Some(id) = self.task_button_dialog(tree, i) {
            self.task_dialog_click(id);
            return;
        }
        let Some(wins) = self.bar(tree).and_then(|b| b.buttons.get(i).cloned()) else {
            return;
        };
        match wins.as_slice() {
            [] => self.task_button_launch(tree, i),
            [w] => {
                let w = *w;
                let Some(win) = self.win(w) else { return };
                if win.minimized {
                    self.set_minimized(w, false);
                } else if self.focused == Some(w) {
                    self.set_minimized(w, true);
                } else {
                    self.focus_window(w);
                }
            }
            _ if !self.config.taskbar.pinned => {
                if !list_was_open {
                    self.open_window_list(tree, i);
                }
            }
            wins => {
                let live = self.task_windows();
                let wins: Vec<WindowId> =
                    wins.iter().copied().filter(|w| live.contains(w)).collect();
                if let Some(w) =
                    crate::taskbar::group_click_pick(&wins, &self.focus_order, self.focused)
                {
                    self.bring_up(w);
                }
            }
        }
    }

    pub(crate) fn bring_up(&mut self, w: WindowId) {
        if self.win(w).is_some_and(|x| x.minimized) {
            self.set_minimized(w, false);
        } else {
            self.focus_window(w);
        }
    }

    fn task_button_launch(&mut self, tree: TreeId, i: usize) {
        if let Some(k) = self
            .bar(tree)
            .and_then(|b| b.pins.get(i).copied().flatten())
        {
            self.run_quick(k);
        }
    }

    pub fn run_quick(&mut self, k: usize) {
        let Some(action) = self.quicklaunch.get(k).map(|q| q.entry.action.clone()) else {
            return;
        };
        self.run_action(&action, 0);
    }

    pub fn forget_task_window(&mut self, w: WindowId) {
        for bar in &mut self.bars {
            if bar.task_hot == Some(TaskKey::Window(w)) {
                bar.task_hot = None;
            }
            taskslide::forget(bar, TaskKey::Window(w));
        }
    }

    pub fn taskbar_scene_hit(&self, tree: TreeId, x: f64, y: f64) -> Hit {
        let hit = self
            .bar(tree)
            .map_or(TaskbarHit::Nothing, |b| self.bar_hit(b, x, y));
        Hit::Taskbar { bar: tree, hit }
    }
}
