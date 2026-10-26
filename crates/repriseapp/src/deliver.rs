//! Handing an event to the program and carrying out its `Reply`; a modal child gates input.

use crate::clipboard::{CLIPBOARD_FILES, CLIPBOARD_TEXT, text_mime};
use crate::sizing::{bounded, sane_size};
use crate::windows::send_tag;
use crate::{
    DropForm, Event, Host, MAIN, Reply, WinId, editmenu, fielddrop, filedlg, keys, spawn_in,
};
use reprisekit::controls::menu::MenuCtx;
use reprisekit::raster::Rect;
use smithay_client_toolkit::reexports::calloop::timer::{TimeoutAction, Timer};
use std::ffi::OsString;
use std::time::Duration;

pub const TICK_MS: u64 = 50;

impl Host {
    /// The modal child gating `win`: the newest over `win`'s own window.
    fn modal(&self, win: WinId) -> Option<WinId> {
        let slots = self.slots.iter().map(|s| (s.id, s.modal));
        modal_over(win, slots, |w| self.app.parent(w))
    }

    pub(crate) fn gated(&self, win: WinId, ev: &Event) -> bool {
        let flyout = self
            .slots
            .iter()
            .any(|s| s.id == win && s.window.toplevel().is_none());
        input_blocked(ev, self.modal(win), win, flyout)
    }

    pub(crate) fn send(&mut self, win: WinId, ev: Event) {
        if self.gated(win, &ev) {
            return;
        }
        match self.slots.iter().find(|s| s.id == win) {
            Some(s) => self.ctx.aim(s),
            // A hidden main window still takes events the host has no
            // surface for (a `Wake`, a blink); a child with no surface
            // does not exist at all.
            None if win == MAIN => self.ctx.origin = (0, 0),
            None => return,
        }
        if matches!(ev, Event::Focus(_)) {
            for s in &mut self.slots {
                s.access.state.dirty = true;
            }
        }
        let blink = ev == Event::Blink;
        if self.edit_menu_event(win, &ev) {
            return;
        }
        if self.file_dialog_clipboard(win, &ev) {
            return;
        }
        if let Some((reply, changed)) = self.field_drop(win, &ev) {
            self.reread(win, &reply);
            self.apply(win, reply);
            if changed != reprisekit::controls::form::Event::None {
                self.send(win, Event::FieldDropped(changed));
            }
            return;
        }
        filedlg::thumbs_on(&mut *self.app, win, &self.ctx.wake);
        // Read off the live offer: sctk calls `selection` for a new offer but not for a cleared one.
        let offer = self.selection_offer();
        CLIPBOARD_TEXT.set(
            offer
                .as_ref()
                .is_some_and(|o| o.with_mime_types(text_mime).is_some()),
        );
        CLIPBOARD_FILES
            .set(offer.is_some_and(|o| o.with_mime_types(reprisefs::fileclip::source).is_some()));
        let mut reply = self
            .app
            .event(win, ev, &mut self.ctx)
            .with_clipboard(keys::take_clipboard());
        reply.tick |= self.ctx.want_tick.take();
        if !blink {
            self.reread(win, &reply);
        }
        self.apply(win, reply);
    }

    /// Mark the reader trees of the windows `reply` to `win` draws again for a rebuild.
    fn reread(&mut self, win: WinId, reply: &Reply) {
        for s in &mut self.slots {
            s.access.state.dirty |= redraws(reply, win, s.id);
        }
    }

    /// The edit menu's share of `ev` ([`editmenu`]); `true` when the program does not get it. The
    /// menu shows on popups ([`menupopup`](crate::menupopup)), so only a pending right press draws
    /// the window again. A window keeping its canvas has none.
    fn edit_menu_event(&mut self, win: WinId, ev: &Event) -> bool {
        let Some(i) = self.slots.iter().position(|s| s.id == win) else {
            return false;
        };
        if self.app.keeps_canvas(win) {
            return false;
        }
        let within = self.menu_within(self.slots[i].size);
        let cx = MenuCtx {
            text: &self.ctx.text,
            theme: &self.ctx.theme,
            px: self.ctx.theme.fonts.size_px,
            within,
        };
        let paste = self.clipboard_text();
        let out = self
            .edit_menu
            .event(win, ev, self.slots[i].spot.as_ref(), paste, &cx);
        self.slots[i].dirty |= self.edit_menu.is_pending(win);
        match out {
            editmenu::Outcome::Pass => false,
            editmenu::Outcome::Taken => true,
            editmenu::Outcome::Redraw => {
                self.menu_changed(win);
                true
            }
            editmenu::Outcome::Run(k) => {
                self.send(win, Event::Key(k));
                true
            }
        }
    }

    /// [`Ctx::menu_within`](crate::Ctx::menu_within) for a window `size` big.
    pub(crate) fn menu_within(&self, size: (i32, i32)) -> Rect {
        let (w, h) = self.ctx.output_size.unwrap_or(size);
        Rect::new(0, 0, w, h)
    }

    /// `ev` for window `win`'s drop form, when it has one and `ev` is a drag over it or a tick
    /// while one rests on it.
    fn field_drop(
        &mut self,
        win: WinId,
        ev: &Event,
    ) -> Option<(Reply, reprisekit::controls::form::Event)> {
        let drag = matches!(
            ev,
            Event::DragEnter { .. }
                | Event::DragMotion { .. }
                | Event::DragFiles { .. }
                | Event::DragLeave
                | Event::DragDrop { .. }
        );
        let mut hover = self.field_drag.filter(|(w, _)| *w == win).map(|(_, h)| h);
        if !drag && !(hover.is_some() && *ev == Event::Tick) {
            return None;
        }
        let base = self.ctx.base_px();
        let out = if let Some(DropForm { form, px }) = self.app.drop_form(win) {
            let px = px.unwrap_or_else(|| form.units().px(base));
            fielddrop::event(form, &mut hover, ev, &self.ctx.text, px)
        } else {
            let px = self.ctx.form_units().px(base);
            let mut file = self.app.drop_file_dialog(win)?;
            fielddrop::event(&mut file, &mut hover, ev, &self.ctx.text, px)
        };
        self.field_drag = hover.map(|h| (win, h));
        out
    }

    fn arm_tick(&mut self, win: WinId) {
        if self.ticking.contains(&win) {
            return;
        }
        self.ticking.push(win);
        let _ = self.loop_handle.insert_source(
            Timer::from_duration(Duration::from_millis(TICK_MS)),
            move |_, _, host: &mut Host| {
                host.ticking.retain(|w| *w != win);
                if host.slots.iter().any(|s| s.id == win) {
                    host.send(win, Event::Tick);
                }
                TimeoutAction::Drop
            },
        );
    }

    pub(crate) fn apply(&mut self, win: WinId, reply: Reply) {
        if reply.redraw
            && let Some(s) = self.slot_mut(win)
        {
            s.dirty = true;
        }
        if reply.redraw_all {
            for s in self.slots.iter_mut() {
                s.dirty = true;
            }
        }
        for s in self.slots.iter_mut() {
            s.dirty |= reply.redraw_wins.contains(&s.id);
        }
        if reply.quit {
            self.exit = true;
        }
        // Diffed after every event, not just `win`'s: a dialog's event
        // (Open, Save As) often retitles the main window.
        let title = self.app.title();
        let caption = self.app.caption(MAIN);
        let tag = match self
            .slots
            .iter()
            .any(|s| s.id == MAIN && s.caption != caption)
        {
            true => self.tag_of(MAIN, caption),
            false => "",
        };
        let tags = self.tag_manager.clone();
        if let Some(s) = self.slot_mut(MAIN) {
            if s.title != title {
                s.access.state.dirty = true;
                s.title = title.clone();
                if let Some(w) = s.window.toplevel() {
                    w.set_title(title);
                }
            }
            if s.caption != caption
                && let Some(w) = s.window.toplevel()
            {
                s.caption = caption;
                send_tag(tags.as_ref(), w, tag);
            }
        }
        if let Some(text) = reply.copy {
            self.copy(text);
        }
        if let Some(clip) = reply.copy_files {
            self.copy_files(clip);
        }
        if reply.paste {
            self.paste(win);
        }
        if let Some(text) = reply.copy_primary {
            self.copy_primary(text);
        }
        if reply.paste_primary {
            self.paste_primary(win);
        }
        if reply.paste_files {
            self.paste_files(win);
        }
        for argv in reply.spawn {
            self.spawn(&reprisefs::scope::in_scope(
                &argv,
                reply.spawn_app.as_deref(),
            ));
        }
        if let Some(accept) = reply.drag_accept {
            self.drag_accept(win, accept);
        }
        if let Some(action) = reply.answer_drop {
            self.answer_drop(action);
        }
        if let Some(out) = reply.start_drag {
            self.start_drag(win, *out);
        }
        if reply.redraw_drag
            && let Some(scale) = self
                .drag_out
                .as_ref()
                .and_then(|o| o.icon.as_ref())
                .map(|i| i.scale)
        {
            self.draw_drag_icon(scale);
        }
        if reply.resize {
            self.begin_resize(win);
        }
        let fixed = self.app.fixed();
        let mut resized = false;
        if let Some(size) = reply.size
            && let Some(s) = self.slot_mut(MAIN)
        {
            s.picked = sane_size(size);
            let size = bounded(s.picked, s.bounds);
            if s.size != size {
                resized = true;
                s.size = size;
                s.buffers = [None, None];
                s.pool = None;
                s.dirty = true;
                if fixed && let Some(w) = s.window.toplevel() {
                    let (w_, h_) = (size.0 as u32, size.1 as u32);
                    w.set_min_size(Some((w_, h_)));
                    w.set_max_size(Some((w_, h_)));
                }
            }
            s.own = true;
        }
        if resized {
            self.send(MAIN, Event::Resized);
        }
        if reply.tick {
            self.arm_tick(win);
        }
        self.sync_main();
        self.sync_children();
        self.sync_flyout();
        self.sync_inhibitor();
        // An `Ask` drop cannot outlive the window answering it.
        if let Some(a) = &self.asked_drop
            && !self.slot_of_id(a.win)
            && let Some(a) = self.asked_drop.take()
        {
            a.offer.destroy();
        }
        if let Some((id, handoff)) = reply.activate
            && let Some(s) = self.slots.iter_mut().find(|s| s.id == id)
        {
            s.dirty |= s.cues.reopen(handoff.by_key);
            if let (Some(token), Some(activation)) = (handoff.token, &self.activation) {
                activation.activate::<Host>(s.window.wl_surface(), token);
            }
        }
        if let Some(id) = reply.raise {
            self.raise(id);
        }
        self.drain_head_requests();
    }

    fn spawn(&mut self, argv: &[OsString]) {
        if let Err(e) = spawn_in(argv, None, None) {
            log::warn!("spawn {argv:?}: {e}");
        }
    }
}

/// The toplevel `win` belongs to: the window up its chain of [`App::parent`](crate::App::parent)s
/// that has none, or `MAIN`.
pub(crate) fn root_of(win: WinId, parent: impl Fn(WinId) -> Option<WinId>) -> WinId {
    let mut w = win;
    // A chain is a dialog over a dialog at most; the bound stops a loop a program made.
    for _ in 0..8 {
        if w == MAIN {
            break;
        }
        match parent(w) {
            Some(p) if p != w => w = p,
            _ => break,
        }
    }
    w
}

/// Whether `reply` to window `win` draws window `id` again.
pub(crate) fn redraws(reply: &Reply, win: WinId, id: WinId) -> bool {
    (reply.redraw && id == win) || reply.redraw_all || reply.redraw_wins.contains(&id)
}

/// The modal child among `slots` that gates `win`: the newest with the same toplevel.
pub(crate) fn modal_over(
    win: WinId,
    slots: impl Iterator<Item = (WinId, bool)>,
    parent: impl Fn(WinId) -> Option<WinId>,
) -> Option<WinId> {
    let root = root_of(win, &parent);
    top_modal(slots.filter(|&(id, _)| root_of(id, &parent) == root))
}

/// The newest, since each child maps over the ones before it.
pub(crate) fn top_modal(slots: impl Iterator<Item = (WinId, bool)>) -> Option<WinId> {
    slots.filter(|&(_, modal)| modal).map(|(id, _)| id).last()
}

/// Whether `ev` on `win` is dropped because modal child `modal` is up:
/// keys and pointer go to that child alone. A flyout is exempt: it sits
/// over the whole output, the modal child included, so gating it would
/// leave nothing on screen that takes input.
pub(crate) fn input_blocked(ev: &Event, modal: Option<WinId>, win: WinId, flyout: bool) -> bool {
    let input = matches!(
        ev,
        Event::Key(_)
            | Event::KeyUp(_)
            | Event::Pointer(_)
            | Event::DragEnter { .. }
            | Event::DragMotion { .. }
            | Event::DragFiles { .. }
            | Event::DragDrop { .. }
    );
    input && !flyout && modal.is_some_and(|m| m != win)
}

/// The keyboard's window, else the pointer's (a hover tip ticks on the blink), else none.
pub(crate) fn blink_target(focus: Option<WinId>, pointer_in: Option<WinId>) -> Option<WinId> {
    focus.or(pointer_in)
}
