//! Pointer and key routing into dialogs: hover, tips, wheel, moving, presses, keys.

use reprisekit::controls::form::Key;
use reprisekit::frame::CaptionButton;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};

use crate::compositor::State;
use crate::dialog::{self, DialogHit, Outcome};
use crate::input::Interaction;
use crate::textedit::EditKey;

use super::*;

impl State {
    /// The pointer moved onto dialog `index`'s caption button `b` when `over` is
    /// `Some((index, Some(b)))`; a styled look redraws on hot changes, Classic does not.
    pub fn dialog_hover_caption(&mut self, over: Option<(usize, Option<CaptionButton>)>) {
        let styled = self.config.theme.look.style().is_some();
        for i in 0..self.dialogs.views.len() {
            let hot = (over.filter(|o| styled && o.0 == i)).and_then(|o| o.1);
            let v = &mut self.dialogs.views[i];
            if v.dialog.caption_hot != hot {
                v.dialog.caption_hot = hot;
                if styled {
                    self.redraw_dialog(i);
                }
            }
        }
    }

    /// The pointer moved with nothing held: `at` is `Some((index, over))` or off every dialog. A styled
    /// look lights the control under it; Classic redraws nothing.
    pub fn dialog_hover(&mut self, at: Option<(usize, Option<DialogHit>)>) {
        let styled = self.config.theme.look.lights_pointer();
        let pos = self.pointer_location();
        for i in 0..self.dialogs.views.len() {
            let v = &mut self.dialogs.views[i];
            let redraw = match at {
                Some((j, over)) if j == i => {
                    let p = to_local(pos.x, pos.y, (v.x, v.y));
                    v.dialog.hover(over, p, styled)
                }
                _ => v.dialog.leave(styled),
            };
            if redraw {
                self.redraw_dialog(i);
            }
        }
        self.arm_dialog_tip();
    }

    /// Starts counting blinks for the top dialog's label tip once the pointer rests on a label.
    fn arm_dialog_tip(&mut self) {
        let waiting = self
            .dialogs
            .views
            .last()
            .and_then(|v| v.dialog.form())
            .map(|f| f.open_page())
            .is_some_and(|p| p.tip_waiting());
        if !waiting || self.dialogs.tip_timer.is_some() {
            return;
        }
        let timer = Timer::from_duration(crate::caret::BLINK);
        match self
            .handle
            .insert_source(timer, |_, _, state| state.dialog_tip_tick())
        {
            Ok(t) => self.dialogs.tip_timer = Some(t),
            Err(e) => log::warn!("dialog tip timer: {}", e.error),
        }
    }

    /// One blink for the top dialog's label tip: it redraws when the tip came or went.
    fn dialog_tip_tick(&mut self) -> TimeoutAction {
        let top = self.dialogs.views.len().checked_sub(1);
        let page = top
            .and_then(|i| self.dialogs.views[i].dialog.form_mut())
            .map(|f| f.open_page_mut());
        let (changed, waiting) = match page {
            Some(p) => (p.tip_tick(), p.tip_waiting()),
            None => (false, false),
        };
        if let (true, Some(i)) = (changed, top) {
            self.redraw_dialog(i);
        }
        if waiting {
            TimeoutAction::ToDuration(crate::caret::BLINK)
        } else {
            self.dialogs.tip_timer = None;
            TimeoutAction::Drop
        }
    }

    /// A wheel frame over `over` on dialog `index` (`discrete` in 120ths of a notch, else
    /// `continuous` travel). The tray list scrolls a row a notch and ignores it while a combo is
    /// open; others take `WHEEL_ROWS` a notch. True when a dialog is there.
    pub fn dialog_wheel(
        &mut self,
        index: usize,
        over: Option<DialogHit>,
        discrete: i32,
        continuous: f64,
    ) -> bool {
        let Some(id) = self.dialogs.views.get(index).map(|v| v.id) else {
            return false;
        };
        if self.dialogs.views.iter().any(|v| v.parent == Some(id)) || self.under_browse(id) {
            return true;
        }
        let v = &mut self.dialogs.views[index];
        if let Some(t) = v.dialog.tray_mut() {
            let notches = v.wheel.notches(discrete, continuous);
            if notches != 0 && t.wheel(dialog::tray_part(over), notches) {
                self.redraw_dialog(index);
            }
            return true;
        }
        let rows = v.wheel.lines(discrete, continuous);
        if rows == 0 {
            return true;
        }
        let shown = v.layout.completion(&v.dialog).map(|(_, l)| l.rows);
        if shown.is_some_and(|n| v.dialog.wheel_completion(n, rows)) {
            self.redraw_dialog(index);
            return true;
        }
        match v.dialog.wheel(over, rows) {
            Outcome::Nothing => {}
            Outcome::Redraw => self.redraw_dialog(index),
            out if index + 1 == self.dialogs.views.len() => self.apply_dialog(out),
            _ => {}
        }
        true
    }

    pub fn dialog_hit(&self, index: usize, lx: f64, ly: f64) -> Option<DialogHit> {
        let v = self.dialogs.views.get(index)?;
        hit(v, lx, ly, &self.text, &self.config.theme)
    }

    /// Moves dialog `index` to `(x, y)`; the output under its title bar becomes its own, for its
    /// work area, scale and lifetime.
    pub fn move_dialog(&mut self, index: usize, x: i32, y: i32) {
        let Some(v) = self.dialogs.views.get(index) else {
            return;
        };
        if (v.x, v.y) == (x, y) {
            return;
        }
        let m = &self.config.theme.metrics;
        let bar = (
            x + v.layout.size.0 / 2,
            y + m.frame_width + m.caption_height / 2,
        );
        let onto = (self.output_at(f64::from(bar.0), f64::from(bar.1))).filter(|o| *o != v.output);
        let rescale = onto
            .as_ref()
            .is_some_and(|o| self.draw_scale_at(Some(o)) != self.draw_scale_at(Some(&v.output)));
        let v = &mut self.dialogs.views[index];
        v.x = x;
        v.y = y;
        if let Some(o) = onto {
            v.output = o;
        }
        let tree = v.tree;
        let outputs = self.scene.move_on_outputs(tree, x, y);
        if index + 1 == self.dialogs.views.len() {
            self.ime_field_moved(crate::caret::Editor::Dialog, (x, y));
        }
        if rescale {
            self.redraw_dialog(index);
        }
        self.redraw_nodes(outputs, &[]);
    }

    /// A left press on dialog `index` at `hit` (`None` for the bare body). A dialog under the top one is
    /// raised first; one with another dialog or a window opened over it raises that one instead.
    /// One waiting on a Browse child takes a press on Cancel alone.
    pub fn dialog_press(&mut self, index: usize, hit: Option<DialogHit>, time_msec: u32) {
        let Some(v) = self.dialogs.views.get(index) else {
            return;
        };
        let id = v.id;
        let cancel = match hit {
            Some(DialogHit::Button(i)) => (v.dialog.buttons.get(i))
                .is_some_and(|b| b.action == crate::dialog::DialogAction::Cancel),
            _ => false,
        };
        self.raise_dialog(index);
        if self.dialogs.views.last().is_some_and(|v| v.id != id)
            || (self.under_browse(id) && !cancel)
        {
            return;
        }
        let top = self.dialogs.views.len() - 1;
        if matches!(hit, Some(DialogHit::Caption)) {
            // The raise renumbered the stack; the pressed dialog is the top one now.
            let pos = self.pointer_location();
            let v = &self.dialogs.views[top];
            self.interaction = Interaction::DialogMove {
                index: top,
                dx: pos.x - f64::from(v.x),
                dy: pos.y - f64::from(v.y),
            };
            return;
        }
        let lights = self.config.theme.look.lights_pointer();
        let v = &mut self.dialogs.views[top];
        let outcome = match hit {
            Some(h @ DialogHit::Close) => {
                if let Some(b) = dialog::caption_button(Some(h)) {
                    v.dialog.press_caption(b);
                }
                self.interaction = Interaction::DialogPress;
                Outcome::Redraw
            }
            // The ? box does nothing but shows the press.
            Some(DialogHit::Help) => {
                v.dialog.press_help();
                self.interaction = Interaction::DialogPress;
                if lights {
                    Outcome::Redraw
                } else {
                    Outcome::Nothing
                }
            }
            Some(DialogHit::Button(i)) => {
                if v.dialog.press_button(i) {
                    self.interaction = Interaction::DialogPress;
                    Outcome::Redraw
                } else {
                    Outcome::Nothing
                }
            }
            Some(DialogHit::Field(x))
                if v.dialog
                    .field
                    .as_ref()
                    .is_some_and(|f| f.combo.open.is_none()) =>
            {
                self.press_dialog_field(x, time_msec)
            }
            other => self.press_dialog_body(other, time_msec),
        };
        self.apply_dialog(outcome);
        self.run_path_completions();
    }

    /// A right press on dialog `index` at `at`: a field under it takes the keyboard (the caret
    /// moving to the press unless the press is inside its selection) and its edit menu opens.
    pub fn dialog_right_press(&mut self, index: usize, hit: Option<DialogHit>, at: (i32, i32)) {
        let Some(id) = self.dialogs.views.get(index).map(|v| v.id) else {
            return;
        };
        self.raise_dialog(index);
        if self.dialogs.views.last().is_none_or(|v| v.id != id) || self.under_browse(id) {
            return;
        }
        let px = self.top_dialog_px();
        let State { dialogs, text, .. } = self;
        let Some(v) = dialogs.views.last_mut() else {
            return;
        };
        if !v.dialog.right_press(text, px, hit) {
            return;
        }
        self.restart_caret();
        self.apply_dialog(Outcome::Redraw);
        self.open_edit_menu(Some(at));
    }

    /// A left press at the pointer for double-click pairing, in the theme's double-click time.
    fn dialog_click(&self, time_msec: u32) -> reprisekit::pointer::Click {
        let p = self.pointer_location();
        let (x, y) = (p.x.floor() as i32, p.y.floor() as i32);
        self.config.theme.metrics.click(x, y, time_msec)
    }

    /// The font size dialogs are set in.
    fn top_dialog_px(&self) -> u32 {
        self.config.theme.fonts.size_px
    }

    /// A left press in the top dialog's field at text offset `x`: caret under the pointer (Shift
    /// extends), maybe a drag-select; a second press in the double-click time selects the word.
    fn press_dialog_field(&mut self, x: i32, time_msec: u32) -> Outcome {
        let at = self.dialog_click(time_msec);
        let shift = self
            .seat
            .get_keyboard()
            .is_some_and(|k| k.modifier_state().shift);
        let px = self.top_dialog_px();
        self.restart_caret();
        let State { dialogs, text, .. } = self;
        let Some(v) = dialogs.views.last_mut() else {
            return Outcome::Nothing;
        };
        if v.field_press.press(0, at) {
            v.dialog.double_click_field(text, px, x)
        } else {
            let out = v.dialog.press_field(text, px, x, shift);
            self.interaction = Interaction::FieldDrag {
                editor: crate::caret::Editor::Dialog,
            };
            out
        }
    }

    /// A left press elsewhere on the top dialog: the dialog clicks `hit`, double-click on a repeat
    /// press on the same row. A held thumb or form button keeps the pointer until release.
    fn press_dialog_body(&mut self, hit: Option<DialogHit>, time_msec: u32) -> Outcome {
        let at = self.dialog_click(time_msec);
        let Some(v) = self.dialogs.views.last_mut() else {
            return Outcome::Nothing;
        };
        let double = match hit {
            Some(DialogHit::Control(h))
                if matches!(h.part, reprisekit::controls::form::Part::Edit(_)) =>
            {
                v.field_press.press(h.item, at)
            }
            _ => {
                v.field_press.reset();
                false
            }
        };
        let out = match double {
            true => v.dialog.double_click(hit),
            false => v.dialog.click(hit),
        };
        if v.dialog.ticking() {
            let id = v.id;
            self.tick_page(id);
        }
        let Some(v) = self.dialogs.views.last_mut() else {
            return out;
        };
        let page = v.dialog.form().map(|f| f.open_page());
        if page.is_some_and(|p| p.drag.is_some()) || v.dialog.holding_thumb() {
            self.interaction = Interaction::FieldDrag {
                editor: crate::caret::Editor::Dialog,
            };
        } else if page.is_some_and(|p| p.holding())
            || v.dialog.tray().is_some_and(|t| t.holding())
            || v.dialog.field_lit.holding()
        {
            // A held push button acts on release.
            self.interaction = Interaction::DialogPress;
        }
        out
    }

    /// Tick dialog `id` while an arrow of the tray's Customize page is held:
    /// the page steps as the arrow repeats.
    fn tick_page(&mut self, id: u64) {
        let every = reprisekit::controls::scrollbar::REPEAT_EVERY;
        let added = self
            .handle
            .insert_source(Timer::from_duration(every), move |now, _, state| {
                let Some(i) = state.dialog_index(id) else {
                    return TimeoutAction::Drop;
                };
                let d = &mut state.dialogs.views[i].dialog;
                let moved = d.tick(now);
                let ticking = d.ticking();
                if moved {
                    state.redraw_dialog(i);
                }
                if ticking {
                    TimeoutAction::ToDuration(every)
                } else {
                    TimeoutAction::Drop
                }
            });
        if let Err(e) = added {
            log::warn!("dialog: no scroll timer: {}", e.error);
        }
    }

    /// The pointer, held since a field or thumb press, is at (`lx`, `ly`): extend the selection or drive the thumb.
    pub fn dialog_drag_field(&mut self, lx: f64, ly: f64) {
        let px = self.top_dialog_px();
        let State { dialogs, text, .. } = self;
        let Some(v) = dialogs.views.last_mut() else {
            return;
        };
        let outcome = if let Some(fd) = v.dialog.form_mut() {
            let (x, y) = to_local(lx, ly, (v.x, v.y));
            let over = fd.form.hit(x, y, text, px);
            let ev = fd.form.motion(x, y, over);
            fd.event(ev)
        } else if v.dialog.holding_thumb() {
            let y = to_local(lx, ly, (v.x, v.y)).1;
            v.dialog.drag_list(&v.layout, y)
        } else {
            let Some(field) = v.layout.field else {
                return;
            };
            let x =
                to_local(lx, ly, (v.x, v.y)).0 - field.x - reprisekit::classic::widgets::FIELD_PAD;
            v.dialog.drag_field(text, px, x)
        };
        self.apply_dialog(outcome);
    }

    /// The pointer moved with a push button held: it draws pushed only while over it.
    pub fn dialog_drag_button(&mut self) {
        let pos = self.pointer_location();
        let Some(i) = self.dialogs.views.len().checked_sub(1) else {
            return;
        };
        let over = self.dialog_hit(i, pos.x, pos.y);
        let styled = self.config.theme.look.lights_pointer();
        let px = self.top_dialog_px();
        let State { dialogs, text, .. } = self;
        let v = &mut dialogs.views[i];
        if v.dialog.holding_button() {
            if v.dialog.drag_button(pressed_button(over)) {
                self.redraw_dialog(i);
            }
            return;
        }
        if v.dialog.caption_box.holding() {
            if v.dialog.drag_caption(over) {
                self.redraw_dialog(i);
            }
            return;
        }
        if v.dialog.help_box.holding() {
            if v.dialog.drag_help(matches!(over, Some(DialogHit::Help))) && styled {
                self.redraw_dialog(i);
            }
            return;
        }
        if v.dialog.field_lit.holding() {
            if v.dialog.drag_drop(over) && styled {
                self.redraw_dialog(i);
            }
            return;
        }
        if let (Some(t), Some(client)) = (v.dialog.tray_mut(), v.layout.page) {
            let lay = t.layout(text, px, (client.x, client.y));
            let y = to_local(pos.x, pos.y, (v.x, v.y)).1;
            let ev = match t.drag(&lay, y) {
                Some(ev) => ev,
                None => t.motion(dialog::tray_part(over)),
            };
            if ev != reprisekit::controls::traycustomize::Event::None {
                self.redraw_dialog(i);
            }
            return;
        }
        let (x, y) = to_local(pos.x, pos.y, (v.x, v.y));
        let Some(fd) = v.dialog.form_mut() else {
            return;
        };
        let on = fd.form.hit(x, y, text, px);
        let ev = fd.form.motion(x, y, on);
        let outcome = fd.event(ev);
        self.apply_dialog(outcome);
    }

    /// The pointer let go after a push button press: acts only on the pressed button.
    pub fn dialog_release_button(&mut self) {
        let pos = self.pointer_location();
        let Some(i) = self.dialogs.views.len().checked_sub(1) else {
            return;
        };
        let over = self.dialog_hit(i, pos.x, pos.y);
        let styled = self.config.theme.look.lights_pointer();
        let v = &mut self.dialogs.views[i];
        if v.dialog.caption_box.holding() {
            v.dialog.drag_caption(over);
            let outcome = match v.dialog.release_caption() {
                Some(CaptionButton::Close) => v.dialog.dismiss(),
                _ => Outcome::Redraw,
            };
            self.apply_dialog(outcome);
            return;
        }
        if v.dialog.help_box.holding() {
            v.dialog.drag_help(matches!(over, Some(DialogHit::Help)));
            if v.dialog.release_help() && styled {
                self.redraw_dialog(i);
            }
            return;
        }
        let outcome = match v.dialog.release_button(pressed_button(over)) {
            Some(out) => out,
            None => v.dialog.release(over),
        };
        self.apply_dialog(outcome);
    }

    /// The pointer was released: a held thumb lets go.
    pub fn dialog_release(&mut self) {
        let outcome = match self.dialogs.views.last_mut() {
            Some(v) => v.dialog.release(None),
            None => return,
        };
        self.apply_dialog(outcome);
    }

    /// The pointer is taken away mid-press: the top dialog lets go without acting.
    pub fn dialog_cancel_press(&mut self) {
        let outcome = match self.dialogs.views.last_mut() {
            Some(v) => v.dialog.cancel_press(),
            None => return,
        };
        self.apply_dialog(outcome);
    }

    pub fn dialog_paste(&mut self, text: &str) {
        if self
            .dialogs
            .views
            .last()
            .is_some_and(|v| self.under_browse(v.id))
        {
            return;
        }
        let outcome = match self.dialogs.views.last_mut() {
            Some(v) => v.dialog.paste(text),
            None => return,
        };
        self.apply_dialog(outcome);
        self.run_path_completions();
    }

    /// A key press for the top dialog; always consumed. One waiting on a Browse child takes
    /// Escape alone.
    pub fn dialog_key(&mut self, k: &DialogKey) {
        use crate::keys::{KEY_Alt_L, KEY_Alt_R, KEY_Escape, MOD_ALT, MOD_CTRL, MOD_SHIFT};
        let alt = k.mods & MOD_ALT != 0;
        // Any key brings out focus rectangles, Alt the underlines ([`Cues::key`]); bare Alt redraws here.
        let alt_key = k.syms.iter().any(|&s| s == KEY_Alt_L || s == KEY_Alt_R);
        if let Some(v) = self.dialogs.views.last_mut()
            && v.cues.key(alt || alt_key)
        {
            self.redraw_top_dialog();
        }
        if !k.syms.contains(&KEY_Escape)
            && self
                .dialogs
                .views
                .last()
                .is_some_and(|v| self.under_browse(v.id))
        {
            return;
        }
        let ctrl = k.mods & MOD_CTRL != 0;
        let shift = k.mods & MOD_SHIFT != 0;
        // The table every repriseapp program reads, so Ctrl+Tab moves the focus as Tab does.
        let special = k
            .syms
            .iter()
            .find_map(|&sym| reprisekit::controls::form::special_key(sym, shift, alt));
        // Everything else is an editing key. Alt+letter is an access key even in a field; bare Backspace
        // stays its own key so a form can route it to the focused item.
        let key = match special.or_else(|| {
            k.edit.clone().and_then(|e| match e {
                EditKey::Insert(s) if alt => {
                    let mut chars = s.chars();
                    match (chars.next(), chars.next()) {
                        (Some(c), None) if !c.is_whitespace() => Some(Key::Access(c)),
                        _ => None,
                    }
                }
                EditKey::Backspace { word: false } => Some(Key::Backspace),
                e => Some(Key::Edit(e)),
            })
        }) {
            Some(k) => k,
            None => return,
        };
        if let Key::Edit(e) = &key {
            self.clip_edit_key(e);
        }
        let outcome = match self.dialogs.views.last_mut() {
            // Ctrl+Enter breaks a memo line; Ctrl+Home/End move its caret to the ends.
            Some(v) if ctrl => v.dialog.key(reprisekit::controls::form::ctrl_key(key)),
            Some(v) => v.dialog.key(key),
            None => return,
        };
        self.restart_caret();
        self.apply_dialog(outcome);
        self.run_path_completions();
    }
}
