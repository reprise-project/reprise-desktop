//! Opening, raising, focusing, minimising and closing dialogs on the stack.

use std::sync::atomic::Ordering;

use reprisekit::cues::Cues;
use reprisekit::pointer::DoubleClick;

use crate::compositor::{SceneOwner, State};
use crate::dialog::{self, Dialog, DialogLayout, Kind as FormKind, Placement, Style};
use crate::glass::Backdrop;
use crate::input::Interaction;

use super::*;

/// Why a box left the stack.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gone {
    /// Its own answer, or its owner, closed it.
    Closed,
    /// The stack was cleared under it: a Welcome panel, a session end, the last output unplugged.
    TornDown,
}

impl State {
    pub(crate) fn dialog_index(&self, id: u64) -> Option<usize> {
        self.dialogs.views.iter().position(|v| v.id == id)
    }

    /// Show `d`, placed per `d.placement` on the output under the pointer. A dialog with the same
    /// `identity` is raised instead of reopened, its cues out when a key asked ([`Cues::reopen`]).
    /// A Welcome panel takes the screen alone and closes the switchers; any other dialog opened
    /// under the 3D switcher joins its stack (`flip_join`) and stays hidden until its cards land
    /// (`flip_hide_dialog`).
    pub fn open_dialog(&mut self, d: Dialog) {
        if matches!(d.style, Style::Welcome(_)) {
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
                if shown && let Some(j) = self.dialog_index(id) {
                    self.redraw_dialog(j);
                }
            }
            None => {
                self.open_view(d, None);
            }
        }
    }

    /// Show `d` over the top dialog as its child, which takes the keyboard and the parent's pointer input.
    pub(super) fn open_dialog_over(&mut self, d: Dialog) {
        let parent = self.dialogs.views.last().map(|v| v.id);
        self.open_view(d, parent);
    }

    /// Show `d` as the child of `parent`, raised first; on its own when `parent` is closed or None.
    pub(crate) fn open_dialog_over_id(&mut self, d: Dialog, parent: Option<u64>) {
        match parent.and_then(|id| self.dialog_index(id)) {
            Some(i) => {
                self.raise_dialog(i);
                self.open_view(d, parent);
            }
            None => self.open_dialog(d),
        }
    }

    /// Show `d` on top, as `parent`'s child when given; returns its view, None with no output to
    /// show it on.
    pub(crate) fn open_view(&mut self, mut d: Dialog, parent: Option<u64>) -> Option<u64> {
        let pos = self.pointer_location();
        let out = self
            .output_at(pos.x, pos.y)
            .or_else(|| self.scene.outputs().first().cloned())?;
        let area = self.usable_area(Some(&out));
        if area.width <= 0 {
            return None;
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
        };
        let welcome = matches!(d.style, Style::Welcome(_));
        let tree = self.scene.tree_create(self.layers.menu);
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
            wheel: Default::default(),
            icons: None,
            decodes: Default::default(),
            still_canvas: None,
            cues: Cues::opened(self.key_input.last_key_input),
            fade: None,
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
        // A still pointer gets no motion, so what lay under it (a desktop icon's tip) keeps the hover.
        if self.pointer_free() {
            self.owe_repoint(
                smithay::backend::input::InputTime::now(),
                std::time::Duration::ZERO,
            );
        }
        Some(id)
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
        self.fade_in_still(id, node);
    }

    /// Fade dialog `id`'s still `node` in over [`FADE`], or show it at once under reduce motion.
    pub(super) fn fade_in_still(&mut self, id: u64, node: reprisescene::BufferId) {
        if self.config.effects.reduce_motion {
            return;
        }
        if let Some(v) = self.dialogs.views.iter_mut().find(|v| v.id == id) {
            self.scene.set_opacity(node, 0.0);
            v.fade = Some((node, std::time::Instant::now()));
            self.animate(crate::animclock::Anim::DialogFade);
            self.redraw_nodes(Vec::new(), &[node.into()]);
        }
    }

    /// Raise dialog `index` with its family (`raise_order`) and give the keyboard to the top one,
    /// or to the window standing over it (`window_over_dialog`).
    pub(crate) fn raise_dialog(&mut self, index: usize) {
        if index >= self.dialogs.views.len() {
            return;
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
            for i in first..self.dialogs.views.len() {
                self.scene.raise_to_top(self.dialogs.views[i].tree);
            }
        }
        let activated = self.activate_dialogs();
        let blurred = self.blur_dialogs_without_keyboard();
        if activated || blurred || first_moved.is_some() {
            self.restart_caret();
            self.redraw_dialogs();
            self.sync_dialog_tasks(false);
        }
        self.raise_windows_over_dialogs();
        if let Some(id) = self.dialogs.views.last().map(|v| v.id)
            && let Some(w) = self.window_over_dialog(id)
        {
            self.focus_window(w);
        }
    }

    /// Keeps the windows standing over dialogs (`Window::over_dialog`) above every dialog, the
    /// last focused on top.
    fn raise_windows_over_dialogs(&mut self) {
        let over: Vec<_> = (self.focus_order.iter())
            .filter_map(|&w| self.win(w).filter(|x| x.over_dialog.is_some()))
            .map(|x| x.tree)
            .collect();
        for tree in over {
            self.scene.raise_to_top(tree);
        }
    }

    /// Whether the top dialog has the keyboard; a Welcome panel always does.
    pub fn dialog_has_keyboard(&self) -> bool {
        (self.dialogs.views.last())
            .is_some_and(|v| self.dialogs.active || matches!(v.dialog.style, Style::Welcome(_)))
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
            .is_some_and(|v| matches!(v.dialog.style, Style::Welcome(_)))
    }

    /// After the stack or keyboard changed: the consent gate follows the keyboard, and the taskbars
    /// redraw when some dialog has a button or `closed_one` had one.
    fn sync_dialog_tasks(&mut self, closed_one: bool) {
        self.follow_consent_keyboard();
        if closed_one || self.dialogs.views.iter().any(|v| v.dialog.on_taskbar()) {
            self.redraw_taskbars();
        }
    }

    /// Raise dialog `id` and give it the keyboard, as its task button click does; no-op when closed.
    pub fn focus_dialog(&mut self, id: u64) {
        if let Some(i) = self.dialog_index(id) {
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

    /// The close bind on the top dialog: it answers as its close box does ([`Dialog::dismiss`]).
    pub fn dismiss_dialog(&mut self) {
        if let Some(out) = self.dialogs.views.last().map(|v| v.dialog.dismiss()) {
            self.apply_dialog(out);
        }
    }

    pub(super) fn close_dialog(&mut self) {
        if let Some(i) = self.dialogs.views.len().checked_sub(1) {
            self.close_dialog_at(i);
        }
    }

    /// Close dialog `index` for its own answer or for its owner ([`Gone::Closed`]).
    pub fn close_dialog_at(&mut self, index: usize) {
        self.remove_dialog(index, Gone::Closed);
    }

    fn remove_dialog(&mut self, index: usize, gone: Gone) {
        let v = self.dialogs.views.remove(index);
        let on_taskbar = v.dialog.on_taskbar();
        self.destroy_dialog(v, gone);
        self.settle_dialogs();
        self.sync_dialog_tasks(on_taskbar);
    }

    /// After dialogs left the stack: redraw the new top, or return the keyboard to the windows
    /// when no dialog is left.
    fn settle_dialogs(&mut self) {
        self.renumber_dialogs();
        if self.dialogs.views.is_empty() {
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

    /// Clear the stack for a Welcome panel or a session end ([`Gone::TornDown`]).
    pub fn close_all_dialogs(&mut self) {
        while let Some(i) = self.dialogs.views.len().checked_sub(1) {
            self.remove_dialog(i, Gone::TornDown);
        }
    }

    /// One frame of every still fading in; whether one still fades.
    pub(crate) fn dialog_fade_step(&mut self, now: std::time::Instant) -> bool {
        let mut fading = false;
        let (mut shown, mut on) = (Vec::new(), Vec::new());
        for v in &mut self.dialogs.views {
            let Some((node, at)) = v.fade else {
                continue;
            };
            let t = (now.saturating_duration_since(at).as_secs_f32() / FADE.as_secs_f32()).min(1.0);
            self.scene.set_opacity(node, t);
            shown.push(node.into());
            // A still faded to nothing yet shows on no output.
            on.push(v.output.clone());
            if t < 1.0 {
                fading = true;
            } else {
                v.fade = None;
            }
        }
        self.redraw_nodes(on, &shown);
        fading
    }

    /// The one way off the stack: the scene goes, then [`State::dialog_gone`] tells the asker.
    fn destroy_dialog(&mut self, mut v: DialogView, gone: Gone) {
        self.switcher_forget(crate::switcherview::Entry::Dialog(v.id));
        if matches!(
            self.interaction,
            Interaction::DialogMove { .. }
                | Interaction::DialogPress
                | Interaction::FieldDrag { .. }
        ) {
            self.interaction = Interaction::Idle;
        }
        v.backdrop
            .set(&mut self.scene, &self.glass, v.glass_tree, None);
        let was = self.scene.outputs_of(v.tree);
        self.scene.destroy(v.tree);
        self.redraw_nodes(was, &[]);
        crate::access::touch(self);
        self.dialog_gone(v.id, &v.dialog, gone);
    }

    /// What a box leaving the stack means to whoever waits on it. Every kind says, so a new kind
    /// does not build until it does.
    fn dialog_gone(&mut self, id: u64, d: &Dialog, gone: Gone) {
        self.browse_over_gone(id);
        match &d.style {
            Style::Form(f) => match &f.kind {
                // While the session end still waits, `sessionend::tick` raises the prompt again.
                FormKind::EndSession => {}
                FormKind::Location(desktop_id) => crate::location::dismissed(self, desktop_id),
            },
            Style::EndProgram => {
                if let (Gone::TornDown, Some(pid)) = (gone, d.end_program_pid()) {
                    self.end_program_due(pid);
                }
            }
            Style::Classic { .. } | Style::Welcome(_) | Style::TrayCustomize(_) => {}
        }
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

    /// Move each dialog whose output left the layout onto the pointer's output, or the first, so
    /// a question or a job's progress stays up. A Welcome panel, frozen over the output it left,
    /// closes, as does every dialog once no output is left.
    pub fn move_dialogs_off_scene(&mut self) {
        let pos = self.pointer_location();
        let onto = (self.output_at(pos.x, pos.y)).or_else(|| self.scene.outputs().first().cloned());
        let (mut i, mut removed, mut on_taskbar) = (0, false, false);
        while i < self.dialogs.views.len() {
            let v = &self.dialogs.views[i];
            if self.scene.outputs().contains(&v.output) {
                i += 1;
                continue;
            }
            let welcome = matches!(v.dialog.style, Style::Welcome(_));
            match onto.clone().filter(|_| !welcome) {
                Some(o) => {
                    self.dialogs.views[i].output = o;
                    self.refit_dialog(i);
                    let v = &self.dialogs.views[i];
                    let area = self.usable_area(Some(&v.output));
                    if let Some((x, y)) = refit_origin(&v.dialog, (v.x, v.y), v.layout.size, area) {
                        self.move_dialog(i, x, y);
                    }
                    self.redraw_dialog(i);
                    i += 1;
                }
                None => {
                    let v = self.dialogs.views.remove(i);
                    on_taskbar |= v.dialog.on_taskbar();
                    removed = true;
                    self.destroy_dialog(v, Gone::TornDown);
                }
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
