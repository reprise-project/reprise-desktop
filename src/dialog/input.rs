use super::*;
use crate::text::{access_key, split_mnemonic};
use crate::textedit::EditKey;
use reprisekit::controls::combo;
use reprisekit::controls::form::{FocusedEdit, Key};
use reprisekit::controls::propsheet::Event as SheetEvent;
use reprisekit::pointer::Lit;

impl Dialog {
    pub fn cycle_page(&mut self, back: bool) -> Outcome {
        match self.form_mut() {
            Some(f) => f.cycle_page(back),
            None => Outcome::Nothing,
        }
    }

    /// A key. A form or the chooser takes it first. Otherwise Enter presses the default button,
    /// Escape cancels, text edits the field; without a field Space presses the focused button and a
    /// letter the button with that access key. With Run's drop list open, Up/Down move the
    /// highlight, Enter takes it, Escape closes the list only. The file picker's keys go through
    /// `picker_mut`.
    pub fn key(&mut self, key: Key) -> Outcome {
        if let Some(f) = self.form_mut() {
            return f.key(key);
        }
        // Ctrl+Enter, Ctrl+Home and Ctrl+End are their own keys only in
        // a form's memo.
        let key = match key {
            Key::NewLine => Key::Enter,
            Key::DocEdge { end: false, select } => Key::Edit(EditKey::Home { select }),
            Key::DocEdge { end: true, select } => Key::Edit(EditKey::End { select }),
            k => k,
        };
        if let Some(c) = self.copy_mut() {
            let key = match key {
                Key::Edit(k) => match reprisekit::controls::form::flat_key(&k) {
                    Some(k) => k,
                    None => return Outcome::Nothing,
                },
                k => k,
            };
            let out = match key {
                Key::Access(ch) if ch.eq_ignore_ascii_case(&'d') => c.cp.toggle_details(),
                k => {
                    let ev = c.cp.form.key(k);
                    c.cp.event(ev)
                }
            };
            return Self::copy_outcome(out);
        }
        if let Some(t) = self.tray_mut() {
            // Nothing in it edits text: a space is Space.
            let key = match key {
                Key::Edit(k) => match reprisekit::controls::form::flat_key(&k) {
                    Some(k) => k,
                    None => return Outcome::Nothing,
                },
                k => k,
            };
            return Self::tray_outcome(t.key(key));
        }
        if let Some(c) = self.chooser_mut() {
            // The chooser has no edit field: a space is Space, a letter
            // the list's type-ahead.
            let key = match key {
                Key::Edit(k) => match reprisekit::controls::form::flat_key(&k) {
                    Some(k) => k,
                    None => return Outcome::Nothing,
                },
                k => k,
            };
            let ev = c.ow.key(key);
            return self.chooser_outcome(ev);
        }
        if self.buttons_only() {
            // Left and Right come in as editing keys; with no field to
            // take them they are the plain arrows.
            let arrow = match &key {
                Key::Edit(EditKey::Left { .. }) => Key::Left,
                Key::Edit(EditKey::Right { .. }) => Key::Right,
                _ => key.clone(),
            };
            match arrow {
                Key::Tab { back } => return self.move_default(!back, true),
                Key::Right | Key::Down => return self.move_default(true, false),
                Key::Left | Key::Up => return self.move_default(false, false),
                _ => {}
            }
        }
        if self.field.is_some()
            && let Some(out) = self.field_focus_key(&key)
        {
            return out;
        }
        if let Some(f) = self.field.as_mut().filter(|f| f.combo.open.is_some()) {
            match key {
                Key::Escape | Key::Enter | Key::Drop => {
                    f.combo.open = None;
                    return Outcome::Redraw;
                }
                Key::Up | Key::Down => {
                    return redraw_if(f.combo.step(key == Key::Down));
                }
                Key::Access(_) | Key::Backspace | Key::Edit(_) | Key::Char(_) | Key::Space => {
                    f.combo.open = None
                }
                _ => return Outcome::Nothing,
            }
        }
        match key {
            Key::Escape => Outcome::Close,
            Key::Enter => match self.buttons.iter().position(|b| b.default) {
                Some(i) => self.press(i),
                None => Outcome::Nothing,
            },
            Key::Up | Key::Down => match self.field.as_mut() {
                Some(f) => redraw_if(f.combo.step(key == Key::Down)),
                None => Outcome::Nothing,
            },
            Key::Drop => match self.field.as_mut() {
                Some(f) => redraw_if(f.combo.toggle()),
                None => Outcome::Nothing,
            },
            Key::Backspace => match self.field.as_mut().and_then(Field::edit_mut) {
                Some(e) => redraw_if(e.key(EditKey::Backspace { word: false })),
                None => Outcome::Nothing,
            },
            // Typed text goes into the field. Without one the default
            // button has the focus, so a space presses it and a letter
            // is a button's access key.
            Key::Edit(EditKey::Insert(s)) => match self.field.as_mut().and_then(Field::edit_mut) {
                Some(e) => {
                    e.key(EditKey::Insert(s));
                    Outcome::Redraw
                }
                None if s == " " => match self.buttons.iter().position(|b| b.default) {
                    Some(i) => self.press(i),
                    None => Outcome::Nothing,
                },
                None => self.press_access(&s),
            },
            Key::Edit(k) => match self.field.as_mut().and_then(Field::edit_mut) {
                Some(e) => redraw_if(e.key(k)),
                None => Outcome::Nothing,
            },
            Key::Char(c) => self.key(Key::Edit(EditKey::Insert(c.to_string()))),
            Key::Space => self.key(Key::Edit(EditKey::Insert(" ".into()))),
            Key::Access(c) => self.press_access(&c.to_string()),
            Key::Tab { .. } | Key::PageUp | Key::PageDown => Outcome::Nothing,
            // Left, Right, Home and End reach a field folded into an
            // `Edit` key; bare, they mean nothing here.
            Key::Left | Key::Right | Key::Home | Key::End => Outcome::Nothing,
            Key::NewLine | Key::DocEdge { .. } => unreachable!("read as Enter, Home or End above"),
        }
    }

    /// The pointer let go over `over`. A form's or the chooser's held
    /// push button acts now (on the release, not the press), the chooser's
    /// only if `over` is still that button, and a held thumb lets go.
    pub fn release(&mut self, over: Option<DialogHit>) -> Outcome {
        if self.field.as_mut().is_some_and(|f| f.thumb.release()) {
            return Outcome::Redraw;
        }
        if self.field_lit.holding() {
            self.field_lit.motion(field_part(over));
            self.field_lit.release();
            return Outcome::Redraw;
        }
        if let Some(p) = self.picker_mut() {
            return p.release();
        }
        if let Some(c) = self.chooser_mut() {
            let h = match over {
                Some(DialogHit::Chooser(h)) => Some(h),
                _ => None,
            };
            let ev = c.ow.release(h);
            return self.chooser_outcome(ev);
        }
        if let Some(t) = self.tray_mut() {
            return Self::tray_outcome(t.release(tray_part(over)));
        }
        if let Some(c) = self.copy_mut() {
            let ev = c.cp.form.release();
            return Self::copy_outcome(c.cp.event(ev));
        }
        let Some(f) = self.form_mut() else {
            return Outcome::Nothing;
        };
        let bar = f.sheet_mut().is_some_and(|s| s.release_bars());
        let Some(form) = f.open_page_mut() else {
            return Outcome::Nothing;
        };
        let ev = form.release();
        match f.event(ev) {
            Outcome::Nothing if bar => Outcome::Redraw,
            out => out,
        }
    }

    /// Whether a held page arrow repeats, a sheet's or the tray page's: the host sends
    /// [`Dialog::tick`] until the release.
    pub fn ticking(&self) -> bool {
        self.form()
            .and_then(|f| f.sheet())
            .is_some_and(|s| s.ticking())
            || self.tray().is_some_and(|t| t.ticking())
    }

    /// A host tick at `now`: a held page arrow steps once it repeats. True when the page moved.
    pub fn tick(&mut self, now: std::time::Instant) -> bool {
        if let Some(s) = self.form_mut().and_then(|f| f.sheet_mut()) {
            return s.tick(now) != SheetEvent::None;
        }
        self.tray_mut()
            .is_some_and(|t| t.tick(now) != traycustomize::Event::None)
    }

    /// Whether the pointer holds a thumb or an arrow of a sheet's page scrollbar.
    pub fn holding_page_bar(&self) -> bool {
        self.form()
            .and_then(|f| f.sheet())
            .is_some_and(|s| s.holding_bar())
    }

    /// The pointer at (`x`, `y`) while a sheet's page scrollbar is held: a held thumb drags the
    /// page, a held arrow lights while the pointer is on it. True when anything moved or lit.
    pub fn drag_page_bar(&mut self, x: i32, y: i32, text: &TextRenderer, px: u32) -> bool {
        let Some(s) = self.form_mut().and_then(|f| f.sheet_mut()) else {
            return false;
        };
        let hit = s.hit(x, y, text, px);
        s.motion(x, y, hit) != SheetEvent::None
    }

    /// A left press other than on a button. A form or the chooser takes it first; on the drop arrow
    /// it toggles the list, on a row takes the entry, on the scroll arrows scrolls, elsewhere
    /// closes an open list. Not the file picker's (see `key`).
    pub fn click(&mut self, hit: Option<DialogHit>) -> Outcome {
        if let Some(f) = self.form_mut() {
            return f.click(hit);
        }
        if let Some(c) = self.chooser_mut() {
            let h = match hit {
                Some(DialogHit::Chooser(h)) => Some(h),
                _ => None,
            };
            let ev = c.ow.click(h);
            return self.chooser_outcome(ev);
        }
        if let Some(t) = self.tray_mut() {
            return Self::tray_outcome(t.click(tray_part(hit)));
        }
        if let Some(c) = self.copy_mut() {
            let out = match hit {
                Some(DialogHit::CopyDetails) => c.cp.toggle_details(),
                Some(DialogHit::Control(h)) => {
                    let ev = c.cp.form.click(Some(h));
                    c.cp.event(ev)
                }
                _ => {
                    let ev = c.cp.form.click(None);
                    c.cp.event(ev)
                }
            };
            return Self::copy_outcome(out);
        }
        if hit == Some(DialogHit::Drop) && self.field.is_some() {
            // The drop button draws held until the release, and the
            // field takes the focus.
            self.field_lit.press(Some(FieldPart::Drop), false);
            self.button_focus = None;
        }
        let Some(f) = self.field.as_mut() else {
            return Outcome::Nothing;
        };
        let c = &mut f.combo;
        let open = c.open.is_some();
        match hit {
            Some(DialogHit::Drop) => {
                c.toggle();
                Outcome::Redraw
            }
            Some(DialogHit::Row(i)) if open => {
                c.pick(i);
                Outcome::Redraw
            }
            // The bar is part of the list: a press on it keeps it open,
            // and one on the thumb grabs it until the release.
            Some(DialogHit::ListBar(bar)) if open => redraw_if(c.press_bar(bar, &mut f.thumb)),
            _ if open => {
                c.open = None;
                Outcome::Redraw
            }
            _ => Outcome::Nothing,
        }
    }

    /// The second press of a double click. On a row of the chooser's list
    /// it takes the row and presses OK, in a form's edit field it selects
    /// the word there; anywhere else it is a plain press.
    pub fn double_click(&mut self, hit: Option<DialogHit>) -> Outcome {
        if let Some(f) = self.form_mut() {
            let h = match hit {
                Some(DialogHit::Control(h)) => Some(h),
                _ => None,
            };
            let Some(form) = f.open_page_mut() else {
                return Outcome::Nothing;
            };
            let ev = form.double_click(h);
            return f.event(ev);
        }
        if let Some(c) = self.chooser_mut() {
            let h = match hit {
                Some(DialogHit::Chooser(h)) => Some(h),
                _ => None,
            };
            let ev = c.ow.double_click(h);
            return self.chooser_outcome(ev);
        }
        self.click(hit)
    }

    /// The wheel turned `rows` rows (down is positive) over `hit`. A form's page takes it as
    /// `Form::wheel` does, a page's scroll bar as `PropSheet::wheel` does, the chooser's list
    /// scrolls while the pointer is on it, Run's open drop list scrolls anywhere. Not the tray
    /// dialog's or the file picker's.
    pub fn wheel(&mut self, hit: Option<DialogHit>, rows: i32) -> Outcome {
        if let Some(f) = self.form_mut() {
            let h = match hit {
                Some(DialogHit::Control(h)) => Some(h),
                Some(DialogHit::PageBar(h)) => {
                    let Some(s) = f.sheet_mut() else {
                        return Outcome::Nothing;
                    };
                    return redraw_if(s.wheel(Some(h), rows) != SheetEvent::None);
                }
                _ => None,
            };
            let Some(form) = f.open_page_mut() else {
                return Outcome::Nothing;
            };
            let ev = form.wheel(h, rows);
            if ev == reprisekit::controls::form::Event::None
                && !matches!(hit, Some(DialogHit::Tab(_)))
                && let Some(s) = f.sheet_mut()
            {
                return redraw_if(s.wheel_page(rows) != SheetEvent::None);
            }
            return f.event(ev);
        }
        if let Some(c) = self.chooser_mut() {
            let h = match hit {
                Some(DialogHit::Chooser(h)) => Some(h),
                _ => None,
            };
            let ev = c.ow.wheel(h, rows);
            return self.chooser_outcome(ev);
        }
        match self.field.as_mut() {
            Some(f) if f.combo.open.is_some() => redraw_if(f.combo.scroll(rows)),
            _ => Outcome::Nothing,
        }
    }

    /// A press in the field `x` px from its text's origin (the list is
    /// closed): the caret goes to the nearest char boundary, extending the
    /// selection with Shift held (`extend`), else dropping it.
    pub fn press_field(&mut self, text: &TextRenderer, px: u32, x: i32, extend: bool) -> Outcome {
        let Some(f) = self.field.as_mut().filter(|f| f.combo.open.is_none()) else {
            return Outcome::Nothing;
        };
        let i = f.combo.edit_index(text, px, x);
        let placed = f.edit_mut().is_some_and(|e| e.place(i, extend));
        redraw_if(self.button_focus.take().is_some() || placed)
    }

    /// A right press, ahead of the edit menu: the field or a page's edit field under it takes
    /// the keyboard, its caret moving to the press unless the press is inside its selection.
    /// `false` when no field is there.
    pub fn right_press(&mut self, text: &TextRenderer, px: u32, hit: Option<DialogHit>) -> bool {
        match hit {
            Some(DialogHit::Control(h)) => self
                .form_mut()
                .and_then(|f| f.open_page_mut())
                .is_some_and(|page| {
                    page.right_press(Some(h)) != reprisekit::controls::form::Event::None
                }),
            Some(DialogHit::Field(x)) => {
                let Some(f) = self.field.as_mut().filter(|f| f.combo.open.is_none()) else {
                    return false;
                };
                let i = f.combo.edit_index(text, px, x);
                let Some(e) = f.edit_mut() else {
                    return false;
                };
                e.press_for_menu(i);
                self.button_focus = None;
                true
            }
            _ => false,
        }
    }

    pub fn double_click_field(&mut self, text: &TextRenderer, px: u32, x: i32) -> Outcome {
        let Some(f) = self.field.as_mut().filter(|f| f.combo.open.is_none()) else {
            return Outcome::Nothing;
        };
        let i = f.combo.edit_index(text, px, x);
        let Some(e) = f.edit_mut() else {
            return Outcome::Nothing;
        };
        let before = e.clone();
        e.select_word_at(i);
        redraw_if(*e != before)
    }

    pub fn drag_field(&mut self, text: &TextRenderer, px: u32, x: i32) -> Outcome {
        self.press_field(text, px, x, true)
    }

    pub fn holding_thumb(&self) -> bool {
        self.field.as_ref().is_some_and(|f| f.thumb.holding())
    }

    pub fn drag_list(&mut self, lay: &DialogLayout, y: i32) -> Outcome {
        let (Some(f), Some(field)) = (self.field.as_mut(), lay.field) else {
            return Outcome::Nothing;
        };
        let cl = combo::layout(&f.combo, field, &lay.controls);
        redraw_if(f.combo.drag_bar(&cl, &f.thumb, y))
    }

    /// A message box or a Welcome panel: a dialog whose only controls are
    /// its buttons, so the keyboard focus is always on one of them.
    fn buttons_only(&self) -> bool {
        matches!(
            self.style,
            Style::Classic { .. } | Style::EndProgram | Style::Welcome
        ) && self.field.is_none()
    }

    /// Move focus to the next enabled button (`forward`) or the previous. Tab wraps (`wrap`);
    /// arrows stop at the ends. The focused button is the default.
    fn move_default(&mut self, forward: bool, wrap: bool) -> Outcome {
        let n = self.buttons.len();
        let Some(from) = self.buttons.iter().position(|b| b.default) else {
            return Outcome::Nothing;
        };
        let order: Vec<usize> = if forward {
            (from + 1..n)
                .chain(if wrap { 0..from } else { 0..0 })
                .collect()
        } else {
            (0..from)
                .rev()
                .chain(if wrap {
                    (from + 1..n).rev()
                } else {
                    (0..0).rev()
                })
                .collect()
        };
        let Some(to) = order.into_iter().find(|&i| self.buttons[i].enabled) else {
            return Outcome::Nothing;
        };
        self.buttons[from].default = false;
        self.buttons[to].default = true;
        Outcome::Redraw
    }

    /// A key in a dialog with a field when Tab or a focused button answers it: Tab/Shift+Tab walk
    /// the field and enabled buttons; on a button Left/Up and Right/Down move to the previous/next,
    /// Enter and Space press, Escape closes, a letter is an access key. `None` leaves the key to
    /// the field.
    fn field_focus_key(&mut self, key: &Key) -> Option<Outcome> {
        if let Key::Tab { back } = *key {
            if let Some(f) = self.field.as_mut() {
                f.combo.open = None;
            }
            let stops: Vec<Option<usize>> = std::iter::once(None)
                .chain(
                    (0..self.buttons.len())
                        .filter(|&i| self.buttons[i].enabled)
                        .map(Some),
                )
                .collect();
            let at = stops
                .iter()
                .position(|&s| s == self.button_focus)
                .unwrap_or(0);
            let n = stops.len();
            self.button_focus = stops[if back { (at + n - 1) % n } else { (at + 1) % n }];
            return Some(Outcome::Redraw);
        }
        let i = self.button_focus?;
        let enabled = |&j: &usize| self.buttons[j].enabled;
        let arrow = match key {
            Key::Left | Key::Up | Key::Edit(EditKey::Left { .. }) => {
                Some((0..i).rev().find(enabled))
            }
            Key::Right | Key::Down | Key::Edit(EditKey::Right { .. }) => {
                Some((i + 1..self.buttons.len()).find(enabled))
            }
            _ => None,
        };
        if let Some(to) = arrow {
            let Some(to) = to else {
                return Some(Outcome::Nothing);
            };
            self.button_focus = Some(to);
            return Some(Outcome::Redraw);
        }
        Some(match key {
            Key::Escape => Outcome::Close,
            Key::Enter | Key::Space => self.press(i),
            Key::Edit(EditKey::Insert(s)) if s == " " => self.press(i),
            Key::Edit(EditKey::Insert(s)) => self.press_access(s),
            Key::Char(c) | Key::Access(c) => self.press_access(&c.to_string()),
            _ => Outcome::Nothing,
        })
    }

    /// Press the enabled button whose access key is `s`'s first letter.
    /// The field's label's access key puts the focus back on the field.
    fn press_access(&mut self, s: &str) -> Outcome {
        let key = s.chars().next().and_then(|c| c.to_lowercase().next());
        if key.is_some()
            && self.button_focus.is_some()
            && self
                .field
                .as_ref()
                .is_some_and(|f| access_key(&f.label) == key)
        {
            self.button_focus = None;
            return Outcome::Redraw;
        }
        let hit = self
            .buttons
            .iter()
            .position(|b| b.enabled && access_key(&b.label) == key);
        match hit {
            Some(i) => self.press(i),
            None => Outcome::Nothing,
        }
    }

    pub fn set_field(&mut self, text: &str) -> Outcome {
        match self.field.as_mut() {
            Some(f) => {
                f.combo.open = None;
                if let Some(e) = f.edit_mut() {
                    e.set_selected(text);
                }
                Outcome::Redraw
            }
            None => Outcome::Nothing,
        }
    }

    pub fn paste(&mut self, text: &str) -> Outcome {
        if let Some(p) = self.picker_mut() {
            return redraw_if(p.dlg.paste(text));
        }
        if let Some(f) = self.form_mut() {
            let Some(form) = f.open_page_mut() else {
                return Outcome::Nothing;
            };
            let ev = form.paste(text.to_string());
            return f.event(ev);
        }
        match self.field.as_mut().and_then(Field::edit_mut) {
            Some(e) => redraw_if(e.paste(text, &[])),
            None => Outcome::Nothing,
        }
    }

    /// A reader moved the keyboard to what lies at dialog point (`x`, `y`), as Tab would: a form's
    /// control, or one of the dialog's buttons. Nothing is pressed.
    pub fn focus_at(
        &mut self,
        layout: &DialogLayout,
        x: i32,
        y: i32,
        text: &TextRenderer,
        px: u32,
    ) -> Outcome {
        match layout.hit(self, x, y, text, px) {
            Some(DialogHit::Button(i)) => self.focus_button(i),
            Some(DialogHit::Browse(_)) => match (self.picker_mut(), layout.page) {
                (Some(p), Some(client)) => p.focus_at(client, (x, y), text, px),
                _ => Outcome::Nothing,
            },
            Some(DialogHit::Chooser(h)) => match self.chooser_mut() {
                Some(c) => redraw_if(c.ow.focus_on(Some(h)) != openwith::Event::None),
                None => Outcome::Nothing,
            },
            Some(DialogHit::Tray(h)) => match self.tray_mut() {
                Some(t) => redraw_if(t.focus_on(Some(h)) != traycustomize::Event::None),
                None => Outcome::Nothing,
            },
            Some(DialogHit::Control(_)) => {
                if let Some(f) = self.form_mut() {
                    return redraw_if(match &mut f.body {
                        FormBody::Sheet(s) => {
                            s.focus_at(x, y, text, px)
                                != reprisekit::controls::propsheet::Event::None
                        }
                        FormBody::Plain(form) => {
                            form.focus_at(x, y, text, px) != reprisekit::controls::form::Event::None
                        }
                    });
                }
                match (self.copy_mut(), layout.page) {
                    (Some(c), Some(client)) => {
                        let ev = c.cp.form.focus_at(x - client.x, y - client.y, text, px);
                        redraw_if(ev != reprisekit::controls::form::Event::None)
                    }
                    _ => Outcome::Nothing,
                }
            }
            _ => Outcome::Nothing,
        }
    }

    /// Button `i` takes the keyboard: in a dialog with a field it wears the focus as Tab gives it,
    /// in one of buttons only it becomes the default as the arrows make it.
    fn focus_button(&mut self, i: usize) -> Outcome {
        if !self.buttons.get(i).is_some_and(|b| b.enabled) {
            return Outcome::Nothing;
        }
        if let Some(f) = self.field.as_mut() {
            if self.button_focus == Some(i) {
                return Outcome::Nothing;
            }
            f.combo.open = None;
            self.button_focus = Some(i);
            return Outcome::Redraw;
        }
        if !self.buttons_only() || self.buttons[i].default {
            return Outcome::Nothing;
        }
        for (k, b) in self.buttons.iter_mut().enumerate() {
            b.default = k == i;
        }
        Outcome::Redraw
    }

    /// A reader asked to see row `row` of the list at dialog point (`x`, `y`): Browse's file list,
    /// Open With's programs, the tray's items, an open drop list, a sheet's page list, or a form
    /// page's list box or tree, scrolls it in.
    pub fn reveal(
        &mut self,
        layout: &DialogLayout,
        (x, y): (i32, i32),
        row: usize,
        text: &TextRenderer,
        px: u32,
    ) -> Outcome {
        if let (Some(p), Some(client)) = (self.picker_mut(), layout.page) {
            return p.reveal(client, (x, y), row, text, px);
        }
        if let Some(c) = self.chooser_mut() {
            return redraw_if(c.ow.reveal(row) != openwith::Event::None);
        }
        if let (Some(t), Some(client)) = (self.tray_mut(), layout.page) {
            let lay = t.layout(text, px, (client.x, client.y));
            let hit = t.hit(&lay, x, y);
            return redraw_if(t.reveal(hit, row));
        }
        if let (Some(f), Some(fr)) = (self.field.as_mut(), layout.field)
            && f.combo.open.is_some()
            && combo::layout(&f.combo, fr, &layout.controls)
                .list
                .contains(x, y)
        {
            return redraw_if(f.combo.reveal(row));
        }
        let Some(f) = self.form_mut() else {
            return Outcome::Nothing;
        };
        if let FormBody::Sheet(s) = &mut f.body {
            let ev = s.reveal(x, y, row, text, px);
            return redraw_if(ev != reprisekit::controls::propsheet::Event::None);
        }
        let Some(form) = f.open_page_mut() else {
            return Outcome::Nothing;
        };
        let ev = form.reveal(x, y, row, text, px);
        f.event(ev)
    }

    /// A drag at dialog point (`x`, `y`): Run's field, or a form's edit field, shows a drop
    /// caret there and takes the drag (`true`); anywhere else the drop caret goes.
    pub fn drag_over(
        &mut self,
        layout: &DialogLayout,
        x: i32,
        y: i32,
        text: &TextRenderer,
        px: u32,
    ) -> bool {
        if let Some(f) = self.form_mut() {
            return f
                .open_page_mut()
                .is_some_and(|form| form.drag_over(x, y, text, px));
        }
        match (self.field.as_mut(), layout.field) {
            (Some(f), Some(r)) => {
                f.combo
                    .drag_over(&combo::layout(&f.combo, r, &layout.controls), x, y)
            }
            _ => false,
        }
    }

    /// The drag left or ended: `true` when a drop caret was showing.
    pub fn drag_leave(&mut self) -> bool {
        if let Some(f) = self.form_mut() {
            return f.open_page_mut().is_some_and(|form| form.drag_leave());
        }
        self.field.as_mut().is_some_and(|f| f.combo.drag_leave())
    }

    /// A drag resting at a field's edge scrolls it a step; `true` when it moved.
    pub fn drag_tick(&mut self, layout: &DialogLayout, text: &TextRenderer, px: u32) -> bool {
        if let Some(f) = self.form_mut() {
            return f
                .open_page_mut()
                .is_some_and(|form| form.drag_tick(text, px));
        }
        match (self.field.as_mut(), layout.field) {
            (Some(f), Some(r)) => {
                let lay = combo::layout(&f.combo, r, &layout.controls);
                f.combo.drag_tick(&lay, text, px)
            }
            _ => false,
        }
    }

    /// A drag dropped `line` on the field under it: it goes in at the drop caret, selected, and
    /// the field takes the keyboard.
    pub fn drop_text(
        &mut self,
        layout: &DialogLayout,
        line: &crate::textedit::DropLine,
        text: &TextRenderer,
        px: u32,
    ) -> Outcome {
        if let Some(f) = self.form_mut() {
            let Some(form) = f.open_page_mut() else {
                return Outcome::Nothing;
            };
            let ev = form.drop_text(line, text, px);
            return f.event(ev);
        }
        let (Some(f), Some(r)) = (self.field.as_mut(), layout.field) else {
            return Outcome::Nothing;
        };
        let lay = combo::layout(&f.combo, r, &layout.controls);
        if !f.combo.drop_text(&lay, line, text, px) {
            return Outcome::Nothing;
        }
        self.button_focus = None;
        Outcome::Redraw
    }

    /// The edit control that has the keyboard: a form's focused field or
    /// memo, Browse's File name field, else the dialog's own field.
    pub fn focused_edit(&self) -> Option<FocusedEdit<'_>> {
        if let Some(p) = self.picker() {
            return p.dlg.focused_edit().map(FocusedEdit::Line);
        }
        match self.form() {
            Some(f) => f.open_page()?.focused_edit(),
            None => self
                .field
                .as_ref()
                .filter(|_| self.button_focus.is_none())
                .and_then(Field::edit)
                .map(FocusedEdit::Line),
        }
    }

    /// The balloon tip for button `i`: its label (access key stripped) over
    /// its description. None on a plain button or a greyed one, which
    /// shows nothing.
    pub fn tip(&self, i: usize) -> Option<(String, String)> {
        let b = self.buttons.get(i).filter(|b| b.enabled)?;
        let tip = b.tip.clone()?;
        Some((split_mnemonic(&b.label).0, tip))
    }

    fn live_button(&self, hit: Option<DialogHit>) -> Option<usize> {
        match hit {
            Some(DialogHit::Button(i)) if self.buttons.get(i).is_some_and(|b| b.enabled) => Some(i),
            _ => None,
        }
    }

    /// The pointer went down on classic button `i`: it draws pushed until
    /// the release. False (and nothing held) for a disabled button.
    pub fn press_button(&mut self, i: usize) -> bool {
        if !self.buttons.get(i).is_some_and(|b| b.enabled) {
            return false;
        }
        self.pointer.press(Some(i), false);
        true
    }

    pub fn holding_button(&self) -> bool {
        self.pointer.holding()
    }

    /// The pointer is taken away mid-press (the Start menu or another
    /// overlay opened over it): whatever it held lets go without acting.
    pub fn cancel_press(&mut self) -> Outcome {
        self.pointer.clear();
        self.caption_box.clear();
        self.help_box.clear();
        self.field_lit.clear();
        if let Some(f) = self.field.as_mut() {
            f.thumb.release();
        }
        if let Some(p) = self.picker_mut() {
            // A toolbar button, which acted on the press, or a column
            // title is held there: either lets go without acting.
            p.release();
        }
        if let Some(c) = self.chooser_mut() {
            c.ow.release(None);
        }
        if let Some(f) = self.form_mut() {
            if let Some(s) = f.sheet_mut() {
                s.release_bars();
            }
            if let Some(page) = f.open_page_mut() {
                page.cancel_pointer();
            }
        }
        Outcome::Redraw
    }

    /// The held box draws pushed, not hot, so dragged off it draws plain.
    pub fn press_caption(&mut self, b: CaptionButton) {
        self.caption_hot = None;
        self.caption_box.press(Some(b), false);
    }

    pub fn drag_caption(&mut self, over: Option<DialogHit>) -> bool {
        self.caption_box.holding() && self.caption_box.motion(caption_button(over))
    }

    /// The pointer let go of the held caption button: the button when it was over it, which acts.
    pub fn release_caption(&mut self) -> Option<CaptionButton> {
        self.caption_box.release()
    }

    /// The pointer went down on the caption's ? box: it draws pressed
    /// while the pointer stays on it, and its release does nothing.
    pub fn press_help(&mut self) {
        self.help_box.press(Some(()), false);
    }

    pub fn drag_help(&mut self, on: bool) -> bool {
        self.help_box.holding() && self.help_box.motion(on.then_some(()))
    }

    pub fn release_help(&mut self) -> bool {
        let before = self.help_box.lit();
        self.help_box.release();
        self.help_box.lit() != before
    }

    pub fn drag_drop(&mut self, over: Option<DialogHit>) -> bool {
        self.field_lit.holding() && self.field_lit.motion(field_part(over))
    }

    pub fn drag_picker(&mut self, over: Option<DialogHit>) -> bool {
        self.picker_mut()
            .is_some_and(|p| p.hover(browse_part(over)))
    }

    pub fn drag_button(&mut self, over: Option<usize>) -> bool {
        self.pointer.holding() && self.pointer.motion(over)
    }

    /// The pointer let go: the held button acts only if the release lands
    /// on it. `None` when no button was held.
    pub fn release_button(&mut self, over: Option<usize>) -> Option<Outcome> {
        if !self.pointer.holding() {
            return None;
        }
        self.pointer.motion(over);
        Some(match self.pointer.release() {
            Some(b) => self.press(b),
            None => Outcome::Redraw,
        })
    }

    pub fn lit(&self) -> Lit<usize> {
        self.pointer.lit()
    }

    /// The pointer rests on `over` at dialog point (`x`, `y`) with nothing
    /// held: the classic button, the closed tab or the page's control
    /// under it lights, and the one it left goes cold. True when a look
    /// that is `styled` has something to redraw; Classic draws no hot
    /// state, except a Welcome panel's hot icon.
    pub fn hover(&mut self, over: Option<DialogHit>, (x, y): (i32, i32), styled: bool) -> bool {
        let button = self.live_button(over);
        let mut redraw = self.pointer.motion(button) && (styled || self.lights_icons());
        let help = matches!(over, Some(DialogHit::Help)).then_some(());
        redraw |= self.help_box.motion(help) && styled;
        redraw |= self.field_lit.motion(field_part(over)) && styled;
        if let Some(p) = self.picker_mut() {
            redraw |= p.hover(browse_part(over));
        }
        if let Some(t) = self.tray_mut() {
            redraw |= t.motion(tray_part(over)) != traycustomize::Event::None && styled;
        }
        if let Some(c) = self.copy_mut() {
            let on = match over {
                Some(DialogHit::Control(h)) => Some(h),
                _ => None,
            };
            redraw |= c.cp.form.motion(x, y, on) != reprisekit::controls::form::Event::None;
        }
        if let Some(s) = self.form_mut().and_then(|f| f.sheet_mut()) {
            redraw |= s.motion(x, y, sheet_hit(over)) != SheetEvent::None;
        } else if let Some(f) = self.form_mut() {
            let control = match over {
                Some(DialogHit::Control(h)) => Some(h),
                _ => None,
            };
            if let Some(page) = f.open_page_mut() {
                redraw |= page.motion(x, y, control) != reprisekit::controls::form::Event::None;
            }
        }
        redraw
    }

    pub fn leave(&mut self, styled: bool) -> bool {
        let mut redraw = self.pointer.leave() && (styled || self.lights_icons());
        redraw |= (self.help_box.leave() | self.field_lit.leave()) && styled;
        if let Some(p) = self.picker_mut() {
            redraw |= p.leave();
        }
        if let Some(t) = self.tray_mut() {
            redraw |= t.leave() != traycustomize::Event::None && styled;
        }
        if let Some(c) = self.copy_mut() {
            redraw |= c.cp.form.leave() != reprisekit::controls::form::Event::None;
        }
        if let Some(s) = self.form_mut().and_then(|f| f.sheet_mut()) {
            redraw |= s.leave() != SheetEvent::None;
        } else if let Some(page) = self.form_mut().and_then(|f| f.open_page_mut()) {
            redraw |= page.leave() != reprisekit::controls::form::Event::None;
        }
        redraw
    }

    /// Whether the pointer lights buttons in every look: a Welcome
    /// panel's icon buttons draw hot under it.
    fn lights_icons(&self) -> bool {
        matches!(self.style, Style::Welcome)
    }

    /// Press button `i`; a disabled one does nothing. Run's Browse...
    /// hands the focus back to the field, where the picker's answer (a
    /// picked path, or nothing on Cancel) leaves Enter on OK.
    pub fn press(&mut self, i: usize) -> Outcome {
        let Some(b) = self.buttons.get(i).filter(|b| b.enabled) else {
            return Outcome::Nothing;
        };
        if matches!(b.action, DialogAction::BrowseRun) {
            self.button_focus = None;
        }
        let b = &self.buttons[i];
        match &b.action {
            DialogAction::Cancel => Outcome::Close,
            DialogAction::Spawn(cmd) => Outcome::Spawn(cmd.clone()),
            DialogAction::StandBy(cmd) => Outcome::StandBy(cmd.clone()),
            DialogAction::EndSession(goal, cmd) => Outcome::EndSession(*goal, cmd.clone()),
            DialogAction::Delete { paths, erase } => Outcome::Delete {
                paths: paths.clone(),
                erase: *erase,
            },
            DialogAction::EmptyBin => Outcome::EmptyBin,
            DialogAction::BrowseRun => Outcome::BrowseRun,
            DialogAction::Act(a) => Outcome::Act(a.clone()),
            DialogAction::HideSystemIcon(name) => Outcome::HideSystemIcon(name.clone()),
            DialogAction::DeleteIcons { hide, files } => Outcome::DeleteIcons {
                hide: hide.clone(),
                files: files.clone(),
            },
            DialogAction::EndProgram(pid) => Outcome::EndProgram(*pid),
            DialogAction::TurnOn(aid) => Outcome::TurnOn(*aid),
            DialogAction::TooBig { asker, answer } => Outcome::TooBig {
                asker: asker.clone(),
                answer: *answer,
            },
            DialogAction::RunField => {
                let cmd = self
                    .field
                    .as_ref()
                    .map(|f| f.combo.text().trim().to_string())
                    .unwrap_or_default();
                if cmd.is_empty() {
                    Outcome::Nothing
                } else {
                    Outcome::Run(cmd)
                }
            }
        }
    }
}

/// The tray dialog's part under the pointer, when it is on it.
pub(crate) fn tray_part(over: Option<DialogHit>) -> Option<traycustomize::Hit> {
    match over {
        Some(DialogHit::Tray(h)) => Some(h),
        _ => None,
    }
}

/// The file picker's part under the pointer, when it is on the picker.
fn browse_part(over: Option<DialogHit>) -> Option<reprisekit::controls::filedlg::Hit> {
    match over {
        Some(DialogHit::Browse(h)) => Some(h),
        _ => None,
    }
}

/// The sheet's part under the pointer, when it is on a form dialog's sheet.
fn sheet_hit(over: Option<DialogHit>) -> Option<reprisekit::controls::propsheet::Hit> {
    use reprisekit::controls::propsheet::Hit;
    match over? {
        DialogHit::Tab(i) => Some(Hit::Tab(i)),
        DialogHit::Control(h) => Some(Hit::Control(h)),
        DialogHit::PageBar(h) => Some(h),
        _ => None,
    }
}

fn field_part(hit: Option<DialogHit>) -> Option<FieldPart> {
    match hit {
        Some(DialogHit::Field(_)) => Some(FieldPart::Edit),
        Some(DialogHit::Drop) => Some(FieldPart::Drop),
        _ => None,
    }
}

pub(super) fn redraw_if(changed: bool) -> Outcome {
    if changed {
        Outcome::Redraw
    } else {
        Outcome::Nothing
    }
}
