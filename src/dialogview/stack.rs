//! Opening, raising, focusing, minimising and closing dialogs on the stack.

use std::sync::atomic::Ordering;

use reprisekit::cues::Cues;
use reprisekit::pointer::DoubleClick;
use reprisescene::BufferId;
use smithay::reexports::calloop::RegistrationToken;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};

use crate::compositor::{SceneOwner, State};
use crate::dialog::{self, Dialog, DialogLayout, Placement, Style};
use crate::glass::Backdrop;
use crate::input::Interaction;
use crate::window::keep_inside;

use super::*;

impl State {
    /// Show `d`, placed per `d.placement` on the output under the pointer. A dialog with the same
    /// `identity` is raised instead of reopened, its cues out when a key asked ([`Cues::reopen`]).
    /// A Welcome panel takes the screen alone and closes the switchers; any other dialog opened
    /// under the 3D switcher joins its stack (`flip_join`) and stays hidden until its cards land
    /// (`flip_hide_dialog`).
    pub fn open_dialog(&mut self, d: Dialog) {
        if matches!(d.style, Style::Welcome) {
            self.switcher_cancel();
            self.close_all_dialogs();
            self.open_view(d, None);
            return;
        }
        let same = self
            .dialogs
            .views
            .iter()
            .position(|v| v.dialog.identity() == d.identity());
        match same {
            Some(i) => {
                let v = &mut self.dialogs.views[i];
                let id = v.id;
                let shown = v.cues.reopen(self.key_input.last_key_input);
                self.raise_dialog(i);
                if shown && let Some(j) = self.dialogs.views.iter().position(|v| v.id == id) {
                    self.redraw_dialog(j);
                }
            }
            None => self.open_view(d, None),
        }
    }

    /// Show `d` over the top dialog as its child, which takes the keyboard and the parent's pointer input.
    pub(super) fn open_dialog_over(&mut self, d: Dialog) {
        let parent = self.dialogs.views.last().map(|v| v.id);
        self.open_view(d, parent);
    }

    /// Show `d` as the child of `parent`, raised first; on its own when `parent` is closed or None.
    pub(super) fn open_dialog_over_id(&mut self, d: Dialog, parent: Option<u64>) {
        match self.dialogs.views.iter().position(|v| Some(v.id) == parent) {
            Some(i) => {
                self.raise_dialog(i);
                self.open_view(d, parent);
            }
            None => self.open_dialog(d),
        }
    }

    pub(super) fn open_view(&mut self, mut d: Dialog, parent: Option<u64>) {
        let pos = self.pointer_location();
        let Some(out) = self
            .output_at(pos.x, pos.y)
            .or_else(|| self.scene.outputs().first().cloned())
        else {
            return;
        };
        let area = self.usable_area(Some(&out));
        if area.width <= 0 {
            return;
        }
        let theme = &self.config.theme;
        d.fit_in(theme, &self.text, (area.width, area.height));
        let layout = DialogLayout::compute(theme, &self.text, &d);
        let (x, y) = match d.placement {
            Placement::ByStart => {
                dialog::by_start(area, self.bar_placement(&out).edge, layout.size)
            }
            Placement::Center => (
                area.x + (area.width - layout.size.0) / 2,
                area.y + (area.height - layout.size.1) / 2,
            ),
            Placement::CenterAs(h) => (
                area.x + (area.width - layout.size.0) / 2,
                area.y + (area.height - h) / 2,
            ),
            Placement::UpperThird => {
                let b = self.layout_box(Some(&out));
                (
                    b.x + (b.width - layout.size.0) / 2,
                    b.y + (b.height - layout.size.1) / 3,
                )
            }
            Placement::At(x, y) => keep_inside((x, y), layout.size, area),
        };
        let welcome = matches!(d.style, Style::Welcome);
        let tree = self.scene.tree_create(dialog_layer(&d, &self.layers));
        self.scene
            .set_data(tree, Some(SceneOwner::Dialog(self.dialogs.views.len())));
        // Only `content` shows in the backdrop.
        let content = self.scene.buffer_create(tree);
        let glass_tree = self.scene.tree_create(tree);
        let backdrop = Backdrop::new(&self.glass);
        let buffer = self.scene.buffer_create(tree);
        crate::glass::leave_out(&mut self.scene, buffer);
        self.scene.set_position(tree, x, y);
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let view = DialogView {
            dialog: d,
            id,
            seq: crate::taskbar::next_seq(),
            anchor: usize::MAX,
            parent,
            layout,
            tree,
            content,
            content_shown: None,
            glass_tree,
            backdrop,
            buffer,
            x,
            y,
            output: out,
            field_press: DoubleClick::default(),
            row_press: DoubleClick::default(),
            wheel: 0,
            icons: None,
            decodes: Default::default(),
            still_canvas: None,
            cues: Cues::opened(self.key_input.last_key_input),
            fade: None,
            minimized: false,
        };
        self.dialogs.views.push(view);
        self.activate_dialogs();
        self.blur_dialogs_without_keyboard();
        self.restart_caret();
        self.redraw_dialogs();
        self.sync_dialog_tasks(false);
        if welcome {
            self.grey_the_screen_under(id);
        }
        self.flip_hide_dialog(tree);
        self.flip_join();
        self.run_picker_requests();
        // A still pointer gets no motion, so what lay under it (a desktop icon's tip) keeps the hover.
        if self.pointer_free() {
            self.owe_repoint(
                smithay::backend::input::InputTime::now(),
                std::time::Duration::ZERO,
            );
        }
    }

    /// Freeze the screen behind Welcome panel `id` and fade it to grey. Taken once the panel has the
    /// keyboard, so windows and task buttons show inactive, with the panel's tree off.
    fn grey_the_screen_under(&mut self, id: u64) {
        let Some(v) = self.dialogs.views.iter().find(|v| v.id == id) else {
            return;
        };
        let (tree, out, x, y) = (v.tree, v.output.clone(), v.x, v.y);
        self.scene.set_enabled(tree, false);
        let still = self.without_debug_panels(false, |s| s.output_still(&out));
        self.scene.set_enabled(tree, true);
        let Some(mut c) = still else {
            return;
        };
        crate::snapshot::greyscale(&mut c);
        let b = self.layout_box(Some(&out));
        let node = self.scene.buffer_create(tree);
        self.scene.lower_to_bottom(node);
        crate::glass::leave_out(&mut self.scene, node);
        self.scene.set_position(node, b.x - x, b.y - y);
        self.scene
            .buffer_set_canvas(node, &c.pixels, c.width as i32, c.height as i32);
        self.scene
            .buffer_set_dest_size(node, Some((b.width, b.height).into()));
        self.scene.set_opacity(node, 0.0);
        let fade = self.fade_still(id, node);
        if let Some(v) = self.dialogs.views.iter_mut().find(|v| v.id == id) {
            v.fade = fade;
        }
    }

    /// Raise dialog `index` with its family (`raise_order`) and give the keyboard to the top one; a
    /// minimised one is restored first.
    pub(super) fn raise_dialog(&mut self, index: usize) {
        let Some(v) = self.dialogs.views.get_mut(index) else {
            return;
        };
        let restored = std::mem::take(&mut v.minimized);
        let (tree, windowed) = (v.tree, v.dialog.windowed());
        if restored {
            self.scene.set_enabled(tree, true);
        }
        // Over the windows even when it already tops the dialogs' stack.
        if windowed {
            self.scene.raise_to_top(tree);
        }
        let links: Vec<(u64, Option<u64>)> = self
            .dialogs
            .views
            .iter()
            .map(|v| (v.id, v.parent))
            .collect();
        let order = raise_order(&links, index);
        let first_moved = order.iter().enumerate().position(|(k, &i)| k != i);
        if let Some(first) = first_moved {
            let mut old: Vec<Option<DialogView>> = std::mem::take(&mut self.dialogs.views)
                .into_iter()
                .map(Some)
                .collect();
            self.dialogs.views = order.iter().filter_map(|&i| old[i].take()).collect();
            self.renumber_dialogs();
            // A windowed dialog that moved down keeps its place among the windows.
            for i in first..self.dialogs.views.len() {
                let v = &self.dialogs.views[i];
                if !v.dialog.windowed() {
                    self.scene.raise_to_top(v.tree);
                }
            }
        }
        let activated = self.activate_dialogs();
        let blurred = self.blur_dialogs_without_keyboard();
        if activated || blurred || first_moved.is_some() || restored {
            self.restart_caret();
            self.redraw_dialogs();
            self.sync_dialog_tasks(false);
        }
        if restored {
            self.redraw_all();
        }
    }

    /// Minimise windowed dialog `id`: its tree goes off and it drops to the bottom of the stack,
    /// so the keyboard passes to the dialog now on top or, with none shown, to the top window.
    pub fn minimize_dialog(&mut self, id: u64) {
        let Some(i) = self.dialogs.views.iter().position(|v| v.id == id) else {
            return;
        };
        if self.dialogs.views[i].minimized || !self.dialogs.views[i].dialog.windowed() {
            return;
        }
        let mut v = self.dialogs.views.remove(i);
        v.minimized = true;
        self.scene.set_enabled(v.tree, false);
        self.dialogs.views.insert(0, v);
        self.settle_dialogs();
        self.blur_dialogs_without_keyboard();
        self.redraw_dialogs();
        self.sync_dialog_tasks(false);
        self.redraw_all();
    }

    /// A left click on dialog `id`'s task button: an active windowed dialog minimises, any other
    /// comes up with the keyboard.
    pub fn task_dialog_click(&mut self, id: u64) {
        let windowed = (self.dialogs.views.iter()).any(|v| v.id == id && v.dialog.windowed());
        if windowed && self.active_dialog_id() == Some(id) {
            self.minimize_dialog(id);
        } else {
            self.focus_dialog(id);
        }
    }

    /// Whether the top dialog has the keyboard; a Welcome panel always does, a minimised one never.
    pub fn dialog_has_keyboard(&self) -> bool {
        self.dialogs.views.last().is_some_and(|v| {
            !v.minimized && (self.dialogs.active || matches!(v.dialog.style, Style::Welcome))
        })
    }

    /// The active dialog: the top one while it has the keyboard. Its task button draws pushed in.
    pub fn active_dialog_id(&self) -> Option<u64> {
        self.dialogs
            .views
            .last()
            .filter(|_| self.dialog_has_keyboard())
            .map(|v| v.id)
    }

    /// Whether the top dialog is a Welcome panel, which keeps the keyboard even from Alt+Tab.
    pub fn dialog_is_modal(&self) -> bool {
        self.dialogs
            .views
            .last()
            .is_some_and(|v| matches!(v.dialog.style, Style::Welcome))
    }

    /// Redraw the taskbars after the stack or keyboard changed, when some dialog has a button or
    /// `closed_one` had one.
    fn sync_dialog_tasks(&mut self, closed_one: bool) {
        if closed_one || self.dialogs.views.iter().any(|v| v.dialog.on_taskbar()) {
            self.redraw_taskbars();
        }
    }

    /// Raise dialog `id` and give it the keyboard, as its task button click does; no-op when closed.
    pub fn focus_dialog(&mut self, id: u64) {
        if let Some(i) = self.dialogs.views.iter().position(|v| v.id == id) {
            self.raise_dialog(i);
        }
    }

    /// Give the keyboard to the top dialog; the focused window goes inactive (`focus_top` returns it
    /// when the last dialog closes). Returns whether the dialogs lacked it before.
    fn activate_dialogs(&mut self) -> bool {
        if self.dialogs.active {
            return false;
        }
        self.dialogs.active = true;
        self.end_bar_keys();
        self.drop_window_focus();
        true
    }

    /// A window took the focus: dialogs go inactive. No-op when they lacked the keyboard.
    pub fn deactivate_dialogs(&mut self) {
        if !self.dialogs.active {
            return;
        }
        self.dialogs.active = false;
        self.blur_dialogs_without_keyboard();
        self.redraw_dialogs();
        self.sync_dialog_tasks(false);
    }

    /// Keep the underlines on the dialog with the keyboard only; whether any others hid.
    fn blur_dialogs_without_keyboard(&mut self) -> bool {
        let keyboard = self
            .dialog_has_keyboard()
            .then(|| self.dialogs.views.len() - 1);
        blur_cues_without_keyboard(self.dialogs.views.iter_mut().map(|v| &mut v.cues), keyboard)
    }

    pub(super) fn close_dialog(&mut self) {
        if let Some(i) = self.dialogs.views.len().checked_sub(1) {
            self.close_dialog_at(i);
        }
    }

    /// Close the top dialog and raise the one it was opened over; returns whether that one is still open.
    pub(super) fn close_to_parent(&mut self) -> bool {
        let parent = self.dialogs.views.last().and_then(|v| v.parent);
        self.close_dialog();
        match parent.and_then(|p| self.dialogs.views.iter().position(|v| v.id == p)) {
            Some(i) => {
                self.raise_dialog(i);
                true
            }
            None => false,
        }
    }

    pub fn close_dialog_at(&mut self, index: usize) {
        let v = self.dialogs.views.remove(index);
        let on_taskbar = v.dialog.on_taskbar();
        self.destroy_dialog(v);
        self.settle_dialogs();
        self.sync_dialog_tasks(on_taskbar);
    }

    /// After dialogs left the stack or went minimised: redraw the new top, or return the keyboard
    /// to the windows when no dialog is shown.
    fn settle_dialogs(&mut self) {
        self.renumber_dialogs();
        if self.dialogs.views.last().is_none_or(|v| v.minimized) {
            self.stop_caret();
            if self.dialogs.active {
                self.dialogs.active = false;
                self.focus_top();
            }
        } else {
            self.restart_caret();
            self.redraw_top_dialog();
        }
    }

    pub fn close_all_dialogs(&mut self) {
        while !self.dialogs.views.is_empty() {
            self.close_dialog();
        }
    }

    fn fade_still(&mut self, id: u64, node: BufferId) -> Option<RegistrationToken> {
        let mut step = 0;
        let added =
            self.handle
                .insert_source(Timer::from_duration(FADE_TICK), move |_, _, state| {
                    step += 1;
                    state
                        .scene
                        .set_opacity(node, (step as f32 / FADE_STEPS as f32).min(1.0));
                    state.redraw_all();
                    if step < FADE_STEPS {
                        return TimeoutAction::ToDuration(FADE_TICK);
                    }
                    if let Some(v) = state.dialogs.views.iter_mut().find(|v| v.id == id) {
                        v.fade = None;
                    }
                    TimeoutAction::Drop
                });
        match added {
            Ok(t) => Some(t),
            Err(e) => {
                log::warn!("dialog: no fade timer: {}", e.error);
                self.scene.set_opacity(node, 1.0);
                None
            }
        }
    }

    fn destroy_dialog(&mut self, mut v: DialogView) {
        if v.dialog.picker().is_some() {
            self.pickthumbs.forget(v.id);
        }
        self.switcher_forget(crate::switcherview::Entry::Dialog(v.id));
        if let Some(t) = v.fade.take() {
            self.handle.remove(t);
        }
        if matches!(
            self.interaction,
            Interaction::DialogMove { .. }
                | Interaction::DialogResize { .. }
                | Interaction::DialogPress
                | Interaction::FieldDrag { .. }
        ) {
            self.interaction = Interaction::Idle;
        }
        v.backdrop
            .set(&mut self.scene, &self.glass, v.glass_tree, None);
        self.scene.destroy(v.tree);
        self.redraw_all();
        crate::access::touch(self);
    }

    /// Pull every dialog back inside its output's usable area after a layout change
    /// (`update_layout`); the tray's Customize page lays itself out in the new area first.
    pub fn keep_dialogs_inside(&mut self) {
        for i in 0..self.dialogs.views.len() {
            if self.dialogs.views[i].dialog.tray().is_some() {
                self.refit_dialog(i);
                self.redraw_dialog(i);
            }
            let v = &self.dialogs.views[i];
            let area = self.usable_area(Some(&v.output));
            if let Some((x, y)) = refit_origin(&v.dialog, (v.x, v.y), v.layout.size, area) {
                self.move_dialog(i, x, y);
            }
        }
    }

    pub fn close_dialogs_off_scene(&mut self) {
        let (mut i, mut removed, mut on_taskbar) = (0, false, false);
        while i < self.dialogs.views.len() {
            if !self.scene.outputs().contains(&self.dialogs.views[i].output) {
                let v = self.dialogs.views.remove(i);
                on_taskbar |= v.dialog.on_taskbar();
                removed = true;
                self.destroy_dialog(v);
            } else {
                i += 1;
            }
        }
        if removed {
            self.settle_dialogs();
            self.sync_dialog_tasks(on_taskbar);
        }
    }

    fn renumber_dialogs(&mut self) {
        for (i, v) in self.dialogs.views.iter().enumerate() {
            self.scene.set_data(v.tree, Some(SceneOwner::Dialog(i)));
        }
    }
}
