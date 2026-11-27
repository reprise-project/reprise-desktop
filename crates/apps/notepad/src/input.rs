//! Keys and the pointer in the main window.

use super::*;
use reprisekit::pointer::Shape;

impl Notepad {
    fn key(&mut self, k: KeyInput, ctx: &Ctx) -> Reply {
        let lay = self.bar_layout(ctx);
        let m = &ctx.theme;
        let px = ctx.theme.fonts.size_px;
        if self.context.is_some() {
            return self.context_key(&k, ctx);
        }
        let is_alt = k.sym == keysym::ALT_L || k.sym == keysym::ALT_R;
        if is_alt {
            self.alt_arm.press_alt();
            return Reply::none();
        }
        self.alt_arm.interrupt();
        if editmenu::opens(k.sym, k.mods.shift) {
            self.bar.close();
            let at = self.caret_point(ctx);
            self.open_context(at, true, ctx);
            return Reply::redraw();
        }
        let bar_key = Key::from_input(k.sym, k.utf8.as_deref(), k.mods.shift, k.mods.alt);
        if let Some(bk) = bar_key {
            match self.bar.key(bk, &lay, &ctx.text, m, px) {
                Action::Chosen { title, path, .. } => return self.chosen(title, &path, ctx),
                Action::Redraw => return Reply::redraw(),
                Action::Ignored | Action::Passed => {}
            }
        }
        if let Some(r) = self.accel(&k, ctx) {
            return r;
        }
        let key = match k.sym {
            keysym::UP | keysym::KP_UP => Some(MultiKey::Up {
                select: k.mods.shift,
            }),
            keysym::DOWN | keysym::KP_DOWN => Some(MultiKey::Down {
                select: k.mods.shift,
            }),
            keysym::PAGE_UP | keysym::KP_PAGE_UP => Some(MultiKey::PageUp {
                select: k.mods.shift,
                rows: self.edit_layout(ctx).rows,
            }),
            keysym::PAGE_DOWN | keysym::KP_PAGE_DOWN => Some(MultiKey::PageDown {
                select: k.mods.shift,
                rows: self.edit_layout(ctx).rows,
            }),
            keysym::HOME | keysym::KP_HOME if k.mods.ctrl => Some(MultiKey::DocHome {
                select: k.mods.shift,
            }),
            keysym::END | keysym::KP_END if k.mods.ctrl => Some(MultiKey::DocEnd {
                select: k.mods.shift,
            }),
            keysym::RETURN | keysym::KP_ENTER => Some(MultiKey::Enter),
            keysym::TAB => Some(MultiKey::Tab),
            // An Alt+letter no menu title owns types nothing.
            _ if k.mods.alt && matches!(k.edit, Some(EditKey::Insert(_))) => None,
            _ => k.edit.clone().map(MultiKey::Edit),
        };
        let Some(key) = key else {
            return Reply::none();
        };
        let mut reply = Reply::redraw();
        match &key {
            MultiKey::Edit(EditKey::Copy) | MultiKey::Edit(EditKey::Cut) => {
                reply.copy = Some(self.edit.selected_text()).filter(|s| !s.is_empty());
            }
            MultiKey::Edit(EditKey::Paste) => reply.paste = true,
            _ => {}
        }
        self.edit_key(key, ctx);
        reply
    }

    /// Alt released with nothing typed since it went down: activate the menu bar and underline the
    /// access keys.
    fn key_up(&mut self, k: KeyInput, ctx: &Ctx) -> Reply {
        // A release for a key whose press never arrived is a key the
        // compositor grabbed (Alt+space opens its window menu): it took
        // the keyboard, so the Alt that follows means nothing here.
        if k.sym != keysym::ALT_L && k.sym != keysym::ALT_R {
            self.alt_arm.interrupt();
            return Reply::none();
        }
        if !self.alt_arm.release_alt() {
            return Reply::none();
        }
        let lay = self.bar_layout(ctx);
        self.bar.key(
            Key::Activate,
            &lay,
            &ctx.text,
            &ctx.theme,
            ctx.theme.fonts.size_px,
        );
        Reply::redraw()
    }

    pub(crate) fn pointer(&mut self, p: PointerInput, ctx: &Ctx) -> Reply {
        // A button takes the keyboard away from a half-finished Alt:
        // its release must not activate the menu bar afterwards.
        if matches!(p, PointerInput::Press { .. }) {
            self.alt_arm.interrupt();
        }
        if self.context.is_some() {
            return self.context_pointer(p, ctx);
        }
        let lay = self.bar_layout(ctx);
        let m = &ctx.theme;
        let px = ctx.theme.fonts.size_px;
        match p {
            PointerInput::Press {
                x,
                y,
                button: BTN_LEFT,
                time,
                ..
            } => {
                match self.bar.press(x, y, &lay, &ctx.text, m, px) {
                    Action::Chosen { title, path, .. } => return self.chosen(title, &path, ctx),
                    Action::Redraw => return Reply::redraw(),
                    Action::Passed => {
                        return Reply {
                            redraw: true,
                            ..self.pointer(p, ctx)
                        };
                    }
                    Action::Ignored => {}
                }
                if let Some((bar, r)) = self.status(ctx)
                    && statusbar::hit_grip(&bar, r, x, y)
                {
                    return Reply::resize();
                }
                let el = self.edit_layout(ctx);
                let over = self.bar_at(&el, x, y, ctx);
                let lit = self.view.bars.press_bar(over) && ctx.styled();
                if el.inner.contains(x, y) {
                    let pos = multiedit::pos_at(
                        self.mono.get(),
                        &self.metrics,
                        &self.edit,
                        &el,
                        self.view,
                        x,
                        y,
                        1,
                    );
                    if self.text_clicks.press((), m.metrics.click(x, y, time)) {
                        self.edit.select_word_at(pos);
                    } else {
                        self.edit.place(pos, false);
                        self.dragging = Some((x, y));
                    }
                    return Reply::redraw();
                }
                if let Some((bar, hit)) = multiedit::bar_hit(
                    self.mono.get(),
                    &self.metrics,
                    &self.edit,
                    &el,
                    self.view,
                    x,
                    y,
                ) {
                    self.thumb.press(bar.axis(), hit);
                    self.view = self.view.press_bar(
                        self.mono.get(),
                        &self.metrics,
                        &self.edit,
                        &el,
                        bar,
                        hit,
                    );
                    return Reply::redraw();
                }
                if lit { Reply::redraw() } else { Reply::none() }
            }
            PointerInput::Press {
                x,
                y,
                button: BTN_RIGHT,
                ..
            } => {
                let el = self.edit_layout(ctx);
                if !el.inner.contains(x, y) {
                    return Reply::none();
                }
                self.bar.close();
                let pos = multiedit::pos_at(
                    self.mono.get(),
                    &self.metrics,
                    &self.edit,
                    &el,
                    self.view,
                    x,
                    y,
                    1,
                );
                let (start, end) = self.edit.selection();
                if !(self.edit.has_selection() && (start..=end).contains(&pos)) {
                    self.edit.place(pos, false);
                }
                // The menu takes the left button's release, so a drag in progress ends here.
                self.dragging = None;
                self.thumb.release();
                self.view.bars.release_bar();
                self.open_context((x, y), false, ctx);
                Reply::redraw()
            }
            PointerInput::Release { .. } => {
                self.dragging = None;
                self.thumb.release();
                if self.view.bars.release_bar() && ctx.styled() {
                    Reply::redraw()
                } else {
                    Reply::none()
                }
            }
            PointerInput::Leave => {
                let title = self.bar.leave();
                if (self.view.bars.leave() || title) && ctx.styled() {
                    Reply::redraw()
                } else {
                    Reply::none()
                }
            }
            PointerInput::Motion { x, y } => {
                if self.bar.open.is_some() {
                    self.view.bars.leave();
                    self.bar.motion(x, y, &lay, &ctx.text, m, px);
                    return Reply::redraw();
                }
                let title = self.bar.hover(x, y, &lay);
                let el = self.edit_layout(ctx);
                let over = self.bar_at(&el, x, y, ctx);
                let lit = (self.view.bars.motion(over) || title) && ctx.styled();
                let pointer = match self.dragging {
                    Some(_) => Shape::Text,
                    None if self.thumb.holding() => Shape::Default,
                    None => multiedit::shape(&el, x, y),
                };
                let r = self.drag_motion(x, y, ctx);
                Reply {
                    redraw: r.redraw || lit,
                    pointer,
                    ..r
                }
            }
            PointerInput::Scroll { rows, .. } => {
                if self.scroll(rows as i64, ctx) {
                    Reply::redraw()
                } else {
                    Reply::none()
                }
            }
            PointerInput::ScrollAcross { cols, .. } => {
                let el = self.edit_layout(ctx);
                let (mono, m, edit) = (self.mono.get(), &self.metrics, &self.edit);
                if self.view.scroll_across(cols, mono, m, edit, &el) {
                    Reply::redraw()
                } else {
                    Reply::none()
                }
            }
            _ => Reply::none(),
        }
    }

    fn bar_at(
        &self,
        el: &multiedit::Layout,
        x: i32,
        y: i32,
        ctx: &Ctx,
    ) -> Option<(Axis, ScrollHit)> {
        if !ctx.styled() {
            return None;
        }
        multiedit::bar_hit(
            self.mono.get(),
            &self.metrics,
            &self.edit,
            el,
            self.view,
            x,
            y,
        )
        .and_then(|(bar, hit)| Some((bar.axis(), hit.part()?)))
    }

    fn drag_motion(&mut self, x: i32, y: i32, ctx: &Ctx) -> Reply {
        if self.thumb.holding() {
            let el = self.edit_layout(ctx);
            let to = multiedit::drag_bar(
                self.mono.get(),
                &self.metrics,
                &self.edit,
                &el,
                self.view,
                &self.thumb,
                (x, y),
            );
            if let Some(v) = to {
                self.view = v;
            }
            return Reply::redraw();
        }
        if self.dragging.is_some() {
            self.dragging = Some((x, y));
            return self.select_drag(false, ctx);
        }
        Reply::none()
    }

    fn select_drag(&mut self, tick: bool, ctx: &Ctx) -> Reply {
        let Some((x, y)) = self.dragging else {
            return Reply::none();
        };
        let el = self.edit_layout(ctx);
        let step = multiedit::select_drag(
            self.mono.get(),
            &self.metrics,
            &mut self.edit,
            &el,
            &mut self.view,
            x,
            y,
            1,
            tick,
        );
        Reply {
            redraw: step.changed,
            tick: step.scrolling,
            ..Reply::none()
        }
    }

    pub(crate) fn main_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        match ev {
            Event::Key(k) => self.key(k, ctx),
            Event::KeyUp(k) => self.key_up(k, ctx),
            Event::Pointer(p) => self.pointer(p, ctx),
            Event::Pasted(text) => {
                if self.edit.paste(&text) {
                    self.scroll_to_caret(ctx);
                }
                Reply::redraw()
            }
            Event::CloseRequest => self.confirm_then(PendingAction::Quit),
            Event::Focus(false) => {
                self.focused = false;
                self.alt_arm.interrupt();
                self.bar.close();
                self.context = None;
                Reply::redraw()
            }
            Event::Focus(true) => {
                self.focused = true;
                Reply::redraw()
            }
            Event::Blink if !self.focused => Reply::none(),
            Event::Resized | Event::Blink => Reply::redraw(),
            Event::DragEnter { .. }
            | Event::DragMotion { .. }
            | Event::DragFiles { .. }
            | Event::DragLeave
            | Event::DragDrop { .. } => self.drop_event(ev, ctx),
            Event::Tick if self.dragging.is_some() => self.select_drag(true, ctx),
            Event::Tick => self.drop_event(ev, ctx),
            Event::PastedFiles(_)
            | Event::PastedImage(_)
            | Event::PastedPrimary(_)
            | Event::FieldDropped(_)
            | Event::Wake
            | Event::DragEnd { .. }
            | Event::DropCancelled
            | Event::Output(_)
            | Event::ShortcutsInactive
            | Event::OnTop(_)
            | Event::Suspended(_)
            | Event::Heads(_)
            | Event::HeadsApplied { .. }
            | Event::Handoff(_)
            | Event::TrialEnded(_) => Reply::none(),
        }
    }
}
