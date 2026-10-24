use crate::classic::widgets::{Arrow, Axis, ScrollHit};
use crate::controls::arrange;
use crate::controls::calendar;
use crate::controls::colorbutton;
use crate::controls::combo;
use crate::controls::hotkey;
use crate::controls::iconlist;
use crate::controls::listbox;
use crate::controls::multiedit;
use crate::controls::scrollbar::ThumbDrag;
use crate::controls::timefield;
use crate::controls::treeview;
use crate::controls::updown;
use crate::pointer::{Interaction, Lit, WHEEL_ROWS};
use crate::text::TextRenderer;
use crate::trackbar;

use super::field::{as_shown, unmasked_index, visible_from};
use super::layout::{
    check_box, check_label_rect, glyph_dy, icon_list_layout, radio_box, radio_label_rect,
};
use super::state::toggle_check;
use super::tip::LabelTip;
use super::{Control, Drag, Event, Form, Hit, Light, Memo, Part};

impl Form {
    pub(crate) fn styled(&self) -> bool {
        self.rendered.styled.get()
    }

    pub fn holding(&self) -> bool {
        self.pointer.holding()
    }

    pub fn held_item(&self) -> Option<usize> {
        self.pointer.held().map(|(i, _)| i)
    }

    pub fn lit(&self, i: usize, part: Light) -> Interaction {
        self.pointer.of((i, part))
    }

    pub(super) fn bar_lit(&self, i: usize) -> Lit<ScrollHit> {
        match self.pointer.lit() {
            Some(((j, Light::Bar(hit)), how)) if j == i => Some((hit, how)),
            _ => None,
        }
    }

    pub(super) fn spin_lit(&self, i: usize) -> Lit<Arrow> {
        match self.pointer.lit() {
            Some(((j, Light::Spin(a)), how)) if j == i => Some((a, how)),
            _ => None,
        }
    }

    /// The part `hit` lights, if it lights one: a live control's part the
    /// look draws apart under the pointer.
    fn light(&self, hit: Option<Hit>) -> Option<(usize, Light)> {
        let h = hit?;
        let l = match (&self.items.get(h.item)?.control, h.part) {
            (
                Control::Button { enabled: true, .. }
                | Control::Check { enabled: true, .. }
                | Control::Radio { enabled: true, .. },
                Part::Body,
            ) => Light::Body,
            (Control::Edit { enabled: true, .. }, Part::Edit(_)) => Light::Body,
            (Control::HotKey(h), Part::Body) if h.enabled => Light::Body,
            (Control::Memo(m), Part::Memo(_)) if m.enabled => Light::Body,
            (Control::IconList(_), Part::IconList(_)) => Light::Body,
            (Control::Arrange(a), Part::Tile { .. }) if a.enabled => Light::Body,
            (Control::Combo(c), Part::Combo(ch)) if c.enabled => match ch {
                combo::Hit::Field if c.edit.is_some() => Light::Field,
                combo::Hit::Drop | combo::Hit::Field => Light::Drop,
                combo::Hit::ScrollUp => Light::Bar(ScrollHit::Back),
                combo::Hit::ScrollDown => Light::Bar(ScrollHit::Forward),
                combo::Hit::PageUp => Light::Bar(ScrollHit::PageBack),
                combo::Hit::PageDown => Light::Bar(ScrollHit::PageForward),
                combo::Hit::Thumb(_) => Light::Bar(ScrollHit::Thumb),
                combo::Hit::Row(_) => return None,
            },
            (Control::ListBox { .. }, Part::List(lh)) => match lh {
                listbox::Hit::ScrollUp => Light::Bar(ScrollHit::Back),
                listbox::Hit::ScrollDown => Light::Bar(ScrollHit::Forward),
                listbox::Hit::PageUp => Light::Bar(ScrollHit::PageBack),
                listbox::Hit::PageDown => Light::Bar(ScrollHit::PageForward),
                listbox::Hit::Thumb(_) => Light::Bar(ScrollHit::Thumb),
                listbox::Hit::Row(_) | listbox::Hit::Check(_) => return None,
            },
            (Control::Tree(_), Part::Tree(treeview::Hit::VBar(b))) => Light::Bar(b.part()?),
            (Control::UpDown(u), Part::UpDown(uh)) if u.enabled => match uh {
                updown::Hit::Up => Light::Spin(Arrow::Up),
                updown::Hit::Down => Light::Spin(Arrow::Down),
                updown::Hit::Edit(_) => return None,
            },
            (Control::Time(tf), Part::Time(th)) if tf.enabled => match th {
                timefield::Hit::Up => Light::Spin(Arrow::Up),
                timefield::Hit::Down => Light::Spin(Arrow::Down),
                timefield::Hit::Part(_) => return None,
            },
            (Control::TrackBar(t), Part::TrackBar(trackbar::Hit::Thumb)) if t.enabled => {
                Light::Thumb
            }
            _ => return None,
        };
        Some((h.item, l))
    }

    /// What the pointer lights that the last draw's look shows: every lit
    /// part under a style, only a held push button under Classic.
    fn shown(&self) -> Lit<(usize, Light)> {
        let ((i, part), how) = self.pointer.lit()?;
        let control = &self.items.get(i)?.control;
        if !self.rendered.styled.get() {
            let pushed = how == Interaction::Pressed && matches!(control, Control::Button { .. });
            return pushed.then_some(((i, part), how));
        }
        Some(((i, part), how))
    }

    /// The pointer left the form: the part it rested on goes cold. A held
    /// part stays held until the release.
    pub fn leave(&mut self) -> Event {
        let before = self.shown();
        let hid = self.rest_tip(None);
        self.point = None;
        self.pointer.leave();
        match self.redraw_if(before) {
            Event::None if hid => Event::Redraw,
            e => e,
        }
    }

    /// Let go of the pointer outright, held part and drag included: the
    /// page is going out of view (another tab opened) and will not see
    /// the release.
    pub fn cancel_pointer(&mut self) {
        self.pointer.clear();
        self.point = None;
        self.tip = LabelTip::default();
        // A held tile goes back where the press found it.
        if let Some(Drag::Arrange(i)) = self.drag.take()
            && let Some(Control::Arrange(a)) = self.items.get_mut(i).map(|it| &mut it.control)
        {
            a.cancel_held();
        }
    }

    /// Trackbar `i`'s thumb moved without the pointer (a key, the host):
    /// with the pointer resting on the trackbar, its thumb turns hot when
    /// it moved under it and cold when it moved away.
    pub(super) fn rehit_thumb(&mut self, i: usize) {
        let Some((x, y)) = self.point else {
            return;
        };
        let Some(it) = self.items.get(i) else {
            return;
        };
        let Control::TrackBar(t) = &it.control else {
            return;
        };
        let r = self.at(it);
        let lay = trackbar::layout(t, r.x, r.y, r.width);
        if !lay.bounds.contains(x, y) {
            return;
        }
        let on = t.enabled && lay.thumb.contains(x, y);
        self.pointer.motion(on.then_some((i, Light::Thumb)));
    }

    /// `Redraw` when what the look shows of the pointer differs from
    /// `before`.
    fn redraw_if(&self, before: Lit<(usize, Light)>) -> Event {
        if self.shown() == before {
            Event::None
        } else {
            Event::Redraw
        }
    }

    pub fn hit(&self, x: i32, y: i32, text: &TextRenderer, px: u32) -> Option<Hit> {
        let cm_all = self.rendered.controls.get();
        if let Some(i) = self.open_combo() {
            let it = &self.items[i];
            match &it.control {
                Control::Combo(c) => {
                    let lay = combo::layout(c, self.at(it), &self.rendered.controls.get());
                    if let Some(h) = combo::hit(c, &lay, x, y) {
                        return Some(Hit {
                            item: i,
                            part: Part::Combo(h),
                            x,
                        });
                    }
                }
                Control::Color(b) => match colorbutton::hit_popup(self.at(it), b, x, y) {
                    Some(colorbutton::PopupHit::Swatch(k)) => {
                        return Some(Hit {
                            item: i,
                            part: Part::Swatch(k),
                            x,
                        });
                    }
                    Some(colorbutton::PopupHit::Face) => {
                        return Some(Hit {
                            item: i,
                            part: Part::Popup,
                            x,
                        });
                    }
                    None => {}
                },
                _ => {}
            }
        }
        let text = text.dialog(self.font);
        for (i, it) in self.items.iter().enumerate().rev() {
            if self.clip_of(it).is_some_and(|v| !v.contains(x, y)) {
                continue;
            }
            let part = match &it.control {
                Control::Combo(c) => {
                    let lay = combo::layout(c, self.at(it), &self.rendered.controls.get());
                    match combo::hit(c, &lay, x, y) {
                        Some(h) => Part::Combo(h),
                        None => continue,
                    }
                }
                Control::ListBox { list, .. } => {
                    let lay = listbox::layout(list, self.at(it), &self.rendered.controls.get());
                    match listbox::hit(list, &lay, x, y) {
                        Some(h) => Part::List(h),
                        None if self.at(it).contains(x, y) => Part::Body,
                        None => continue,
                    }
                }
                Control::Tree(tv) => {
                    if !self.at(it).contains(x, y) {
                        continue;
                    }
                    let lay = treeview::layout_without_text(
                        tv,
                        self.at(it),
                        &self.rendered.controls.get(),
                    );
                    Part::Tree(tv.hit(&lay, x, y))
                }
                Control::Calendar(cal) => {
                    let lay = calendar::layout(self.at(it), &self.rendered.controls.get());
                    match calendar::hit(cal, &lay, x, y) {
                        Some(h) => Part::Calendar(h),
                        None => continue,
                    }
                }
                Control::Time(tf) => {
                    let cm = self.rendered.controls.get();
                    match timefield::hit(tf, &timefield::layout(self.at(it), &cm), x, y, text, px) {
                        Some(h) => Part::Time(h),
                        None => continue,
                    }
                }
                Control::Clock(_) => continue,
                Control::HotKey(_) => {
                    if !hotkey::hit(hotkey::layout(self.at(it)), x, y) {
                        continue;
                    }
                    Part::Body
                }
                Control::UpDown(u) => {
                    let lay = updown::layout_for(
                        u,
                        self.at(it),
                        &self.rendered.controls.get(),
                        self.units(),
                    );
                    match updown::hit(u, &lay, x, y, text, px) {
                        Some(h) => Part::UpDown(h),
                        None => continue,
                    }
                }
                Control::TrackBar(t) => {
                    let lay = trackbar::layout(t, self.at(it).x, self.at(it).y, self.at(it).width);
                    match trackbar::hit(&lay, x, y) {
                        Some(h) => Part::TrackBar(h),
                        None => continue,
                    }
                }
                Control::Check { .. } => {
                    let cm = self.rendered.controls.get();
                    let r = self.at(it);
                    let w = self.units().len(200);
                    if check_box(r, &cm, glyph_dy(it, r)).contains(x, y)
                        || check_label_rect(r, it.indent, &cm, w).contains(x, y)
                    {
                        Part::Body
                    } else {
                        continue;
                    }
                }
                Control::Radio { .. } => {
                    let cm = self.rendered.controls.get();
                    let r = self.at(it);
                    let w = self.units().len(200);
                    if radio_box(r, &cm, glyph_dy(it, r)).contains(x, y)
                        || radio_label_rect(r, &cm, w).contains(x, y)
                    {
                        Part::Body
                    } else {
                        continue;
                    }
                }
                Control::IconList(l) => {
                    let lay = icon_list_layout(it.grows, self.at(it), &cm_all);
                    match l.hit(&lay, &cm_all, x, y) {
                        Some(h) => Part::IconList(h),
                        None if lay.frame.contains(x, y) => Part::Body,
                        None => continue,
                    }
                }
                Control::Arrange(a) => {
                    let lay = arrange::layout(self.at(it), a);
                    match a.hit(&lay, x, y) {
                        Some(tile) => Part::Tile { tile, at: (x, y) },
                        None if lay.frame.contains(x, y) => Part::Body,
                        None => continue,
                    }
                }
                Control::Memo(memo) => {
                    let r = self.at(it);
                    if !r.contains(x, y) {
                        continue;
                    }
                    let text = text.dialog(self.font);
                    let m = Memo::metrics(px, &cm_all, self.units());
                    let lay = multiedit::layout(text, &m, &memo.edit, r, 1);
                    let view = self.memo_view_1x(memo);
                    let bar = multiedit::bar_hit(text, &m, &memo.edit, &lay, view, x, y).and_then(
                        |(bar, h)| {
                            let span = multiedit::bar_span(text, &m, &memo.edit, &lay, view, bar)?;
                            Some((bar, h, span))
                        },
                    );
                    match bar {
                        Some((bar, hit, span)) => {
                            let to = view.press_bar(text, &m, &memo.edit, &lay, bar, hit);
                            Part::MemoBar {
                                to: self.memo_view_device(memo, to),
                                hit,
                                span,
                            }
                        }
                        None => {
                            let at = multiedit::pos_at(text, &m, &memo.edit, &lay, view, x, y, 1);
                            Part::Memo(at)
                        }
                    }
                }
                Control::Edit {
                    edit,
                    first,
                    enabled,
                } => {
                    if !self.at(it).contains(x, y) {
                        continue;
                    }
                    if !*enabled {
                        // A greyed field is just face. `click` rejects it
                        // anyway, so there is nothing to measure here.
                        Part::Body
                    } else {
                        // The pen starts FIELD_PAD inside the sunken frame
                        // and at the field's scroll offset, so the index is
                        // counted from there, not from the value's start.
                        let shown = as_shown(edit);
                        let inner_x = self.at(it).x + crate::classic::widgets::FIELD_PAD;
                        let from = visible_from(&shown, first);
                        let dx = (x - inner_x).max(0);
                        let at = crate::textedit::index_at(text, &shown.text[from..], dx, px);
                        // Through the bullets and back, so `click` places
                        // the caret in the value, not in what hides it.
                        Part::Edit(if edit.is_secret() {
                            unmasked_index(edit, from + at)
                        } else {
                            from + at
                        })
                    }
                }
                Control::Label { .. }
                | Control::Lines(_)
                | Control::GreyedLines(_)
                | Control::Group(_)
                | Control::GroupTitle(_)
                | Control::Separator
                | Control::Rule
                | Control::Blank => continue,
                _ => {
                    if self.at(it).contains(x, y) {
                        Part::Body
                    } else {
                        continue;
                    }
                }
            };
            return Some(Hit { item: i, part, x });
        }
        None
    }

    /// A right press, ahead of the host's edit menu: an enabled single-line field or spinner under
    /// it takes the focus, its caret moving to the press unless the press is inside its selection.
    pub fn right_press(&mut self, hit: Option<Hit>) -> Event {
        let Some(Hit { item, part, .. }) = hit else {
            return Event::None;
        };
        let at = match (&self.items[item].control, part) {
            (Control::Edit { enabled: true, .. }, Part::Edit(at)) => at,
            (Control::UpDown(u), Part::UpDown(updown::Hit::Edit(at))) if u.enabled => at,
            _ => return Event::None,
        };
        self.close_popups();
        self.set_focus(Some(item));
        match &mut self.items[item].control {
            Control::Edit { edit, .. } => {
                edit.press_for_menu(at);
            }
            Control::UpDown(u) => u.press_for_menu(at),
            _ => {}
        }
        Event::Redraw
    }

    pub fn click(&mut self, hit: Option<Hit>) -> Event {
        let lit = self.light(hit);
        self.pointer
            .press(lit, lit.is_some_and(|(_, l)| l.sticky()));
        let open = self.open_combo();
        let Some(h) = hit else {
            return if self.close_popups() {
                Event::Redraw
            } else {
                Event::None
            };
        };
        if open.is_some_and(|o| o != h.item) {
            self.close_popups();
        }
        let i = h.item;
        let id = self.items[i].id;
        let rect = self.at(&self.items[i]);
        if self.items[i].focusable() {
            self.set_focus(Some(i));
        }
        let grows = self.items[i].grows;
        match (&mut self.items[i].control, h.part) {
            (
                Control::Button { enabled, .. }
                | Control::Check { enabled, .. }
                | Control::Radio { enabled, .. },
                Part::Body,
            ) => {
                if *enabled {
                    // These act on the release (`release`), not the
                    // press; until then the pointer holds them, and they
                    // draw pressed.
                    Event::Redraw
                } else {
                    Event::None
                }
            }
            (Control::IconList(l), Part::IconList(h)) => {
                let lay = icon_list_layout(grows, rect, &self.rendered.controls.get());
                let picked = matches!(h, iconlist::Hit::Cell(_));
                if l.press(h, &lay) && picked {
                    Event::Changed(self.items[i].id)
                } else {
                    Event::Redraw
                }
            }
            (Control::Arrange(a), Part::Tile { tile, at }) if a.enabled => {
                let lay = arrange::layout(rect, a);
                a.press(&lay, tile, at.0, at.1);
                self.drag = Some(Drag::Arrange(i));
                Event::Redraw
            }
            (Control::Memo(memo), Part::Memo(at)) => {
                if !memo.enabled {
                    return Event::None;
                }
                memo.edit.place(at, false);
                Event::Redraw
            }
            (Control::Memo(memo), Part::MemoBar { to, hit, span }) => {
                if !memo.enabled {
                    return Event::None;
                }
                let moved = memo.view.get() != to;
                memo.view.set(to);
                let mut thumb = ThumbDrag::default();
                thumb.press(span.axis(), hit);
                if thumb.holding() {
                    self.drag = Some(Drag::Memo {
                        item: i,
                        thumb,
                        span,
                    });
                }
                if moved { Event::Redraw } else { Event::None }
            }
            (Control::Edit { edit, enabled, .. }, Part::Edit(at)) => {
                if !*enabled {
                    return Event::None;
                }
                edit.place(at, false);
                self.drag = Some(Drag::Edit(i));
                Event::Redraw
            }
            (Control::Combo(c), Part::Combo(ch)) => match ch {
                combo::Hit::Drop | combo::Hit::Field => {
                    c.toggle();
                    Event::Redraw
                }
                combo::Hit::Row(r) => {
                    if c.pick(r) {
                        Event::Changed(id)
                    } else {
                        Event::Redraw
                    }
                }
                combo::Hit::ScrollUp
                | combo::Hit::ScrollDown
                | combo::Hit::PageUp
                | combo::Hit::PageDown
                | combo::Hit::Thumb(_) => {
                    let mut thumb = ThumbDrag::default();
                    if let Some(bar) = ch.bar() {
                        c.press_bar(bar, &mut thumb);
                    }
                    self.grab_bar(i, thumb);
                    Event::Redraw
                }
            },
            (Control::ListBox { list, .. }, Part::List(lh)) => {
                let visible = listbox::layout(list, rect, &self.rendered.controls.get()).visible;
                match lh {
                    listbox::Hit::Row(r) => {
                        if list.select(r, visible) {
                            Event::Changed(id)
                        } else {
                            Event::Redraw
                        }
                    }
                    listbox::Hit::Check(r) => {
                        list.select(r, visible);
                        list.toggle(r);
                        Event::Changed(id)
                    }
                    listbox::Hit::ScrollUp
                    | listbox::Hit::ScrollDown
                    | listbox::Hit::PageUp
                    | listbox::Hit::PageDown
                    | listbox::Hit::Thumb(_) => {
                        let mut thumb = ThumbDrag::default();
                        if let Some(bar) = lh.bar() {
                            let by = thumb.press(Axis::Vertical, bar).by(1, visible as i64);
                            list.scroll(by as i32, visible);
                        }
                        self.grab_bar(i, thumb);
                        Event::Redraw
                    }
                }
            }
            (Control::ListBox { .. }, _) => Event::Redraw,
            (Control::Tree(tv), Part::Tree(th)) => {
                let lay = treeview::layout_without_text(tv, rect, &self.rendered.controls.get());
                match th {
                    treeview::Hit::Row(r) => {
                        if tv.select(r, &lay) {
                            Event::Changed(id)
                        } else {
                            Event::Redraw
                        }
                    }
                    treeview::Hit::Expander(r) => {
                        tv.toggle(r, &lay);
                        Event::Redraw
                    }
                    treeview::Hit::VBar(bar) => {
                        let mut thumb = ThumbDrag::default();
                        let by = thumb.press(Axis::Vertical, bar).by(1, lay.rows as i64);
                        if by != 0 {
                            tv.scroll(by, &lay);
                        }
                        self.grab_bar(i, thumb);
                        Event::Redraw
                    }
                    // A form's tree never gets a horizontal bar (see
                    // `treeview::layout_without_text`).
                    treeview::Hit::HBar(_) | treeview::Hit::Nothing => Event::Redraw,
                }
            }
            (Control::Tree(_), _) => Event::Redraw,
            (Control::Calendar(cal), Part::Calendar(calendar::Hit::Day(d))) => {
                if !cal.enabled {
                    return Event::None;
                }
                let ch = cal.pick(d);
                self.changed(i, ch)
            }
            (Control::Calendar(_), Part::Calendar(_)) => Event::Redraw,
            (Control::Time(tf), Part::Time(th)) => {
                if !tf.enabled {
                    return Event::None;
                }
                match th {
                    timefield::Hit::Part(k) => {
                        tf.select(k);
                        Event::Redraw
                    }
                    timefield::Hit::Up => {
                        let ch = tf.step(1);
                        self.changed(i, ch)
                    }
                    timefield::Hit::Down => {
                        let ch = tf.step(-1);
                        self.changed(i, ch)
                    }
                }
            }
            (Control::UpDown(u), Part::UpDown(uh)) => {
                if !u.enabled {
                    return Event::None;
                }
                let changed = match uh {
                    updown::Hit::Up => u.step(1),
                    updown::Hit::Down => u.step(-1),
                    updown::Hit::Edit(at) => {
                        u.place(at);
                        false
                    }
                };
                if changed {
                    Event::Changed(id)
                } else {
                    Event::Redraw
                }
            }
            (Control::TrackBar(t), Part::TrackBar(th)) => {
                let changed = match th {
                    trackbar::Hit::Thumb => {
                        self.drag = Some(Drag::TrackBar(i));
                        false
                    }
                    trackbar::Hit::Left => t.step(-1),
                    trackbar::Hit::Right => t.step(1),
                };
                if changed {
                    Event::Changed(id)
                } else {
                    Event::Redraw
                }
            }
            (Control::Color(b), Part::Body) => {
                if b.toggle() {
                    Event::Redraw
                } else {
                    Event::None
                }
            }
            (Control::Color(b), Part::Swatch(k)) => {
                if b.pick(k) {
                    Event::Changed(id)
                } else {
                    Event::Redraw
                }
            }
            (Control::Color(_), Part::Popup) => Event::None,
            (Control::HotKey(h), Part::Body) if h.enabled => Event::Redraw,
            _ => Event::None,
        }
    }

    /// The pointer moved to (`x`, `y`); `over` is what it is over now.
    pub fn motion(&mut self, x: i32, y: i32, over: Option<Hit>) -> Event {
        let hid = self.rest_tip(self.label_at(x, y));
        match self.move_pointer(x, y, over) {
            Event::None if hid => Event::Redraw,
            e => e,
        }
    }

    fn move_pointer(&mut self, x: i32, y: i32, over: Option<Hit>) -> Event {
        self.point = Some((x, y));
        let before = self.shown();
        self.pointer.motion(self.light(over));
        let lit = self.redraw_if(before);
        if let Some(Hit {
            item,
            part: Part::Combo(combo::Hit::Row(r)),
            ..
        }) = over
            && self.drag.is_none()
            && let Some(Control::Combo(c)) = self.items.get_mut(item).map(|it| &mut it.control)
            && c.hover(r)
        {
            return Event::Redraw;
        }
        let Some(drag) = self.drag else {
            return lit;
        };
        let i = match drag {
            Drag::TrackBar(i)
            | Drag::Bar { item: i, .. }
            | Drag::Edit(i)
            | Drag::Memo { item: i, .. }
            | Drag::Arrange(i) => i,
        };
        if let (Control::Memo(memo), Drag::Memo { thumb, span, .. }) =
            (&self.items[i].control, drag)
        {
            let Some(to) = span.drag(self.memo_view_1x(memo), &thumb, (x, y)) else {
                return lit;
            };
            let to = self.memo_view_device(memo, to);
            if to == memo.view.get() {
                return lit;
            }
            memo.view.set(to);
            return Event::Redraw;
        }
        let rect = self.at(&self.items[i]);
        let id = self.items[i].id;
        match (&mut self.items[i].control, drag) {
            (Control::Edit { edit, .. }, Drag::Edit(_)) => {
                // The host hit-tested the pointer already, so the index
                // under it arrives in `over`; past either edge the sweep
                // runs to that end of the value.
                let to = match over {
                    Some(h) if h.item == i => match h.part {
                        Part::Edit(at) => at,
                        _ => return lit,
                    },
                    _ if x < rect.x => 0,
                    _ => edit.text.len(),
                };
                if edit.place(to, true) {
                    return Event::Redraw;
                }
            }
            (Control::TrackBar(t), Drag::TrackBar(_)) => {
                let lay = trackbar::layout(t, rect.x, rect.y, rect.width);
                if t.set(trackbar::position_at(&lay, x)) {
                    return Event::Changed(id);
                }
            }
            (Control::Arrange(a), Drag::Arrange(_)) => {
                if a.drag(x, y) {
                    return Event::Redraw;
                }
            }
            (Control::ListBox { list, .. }, Drag::Bar { thumb, .. }) => {
                let Some(grab) = thumb.grab(Axis::Vertical) else {
                    return lit;
                };
                let lay = listbox::layout(list, rect, &self.rendered.controls.get());
                let top = listbox::top_at(list, &lay, y, grab);
                if top != list.top {
                    list.top = top;
                    return Event::Redraw;
                }
            }
            (Control::Combo(c), Drag::Bar { thumb, .. }) => {
                let lay = combo::layout(c, rect, &self.rendered.controls.get());
                if c.drag_bar(&lay, &thumb, y) {
                    return Event::Redraw;
                }
            }
            (Control::Tree(tv), Drag::Bar { thumb, .. }) => {
                let Some(grab) = thumb.grab(Axis::Vertical) else {
                    return lit;
                };
                let lay = treeview::layout_without_text(tv, rect, &self.rendered.controls.get());
                let top = tv.top_at(&lay, y, grab);
                if top != tv.top {
                    tv.top = top;
                    return Event::Redraw;
                }
            }
            _ => {}
        }
        lit
    }

    /// The second press of a double click. On a list box row the row is
    /// taken and the dialog's default button acts; in an edit field the
    /// word under the pointer is selected. Anywhere else it is a plain
    /// press.
    pub fn double_click(&mut self, hit: Option<Hit>) -> Event {
        if let Some(Hit {
            item,
            part: Part::Edit(at),
            ..
        }) = hit
            && let Control::Edit {
                edit,
                enabled: true,
                ..
            } = &mut self.items[item].control
        {
            edit.select_word_at(at);
            self.set_focus(Some(item));
            self.drag = None;
            return Event::Redraw;
        }
        if let Some(Hit {
            item,
            part: Part::List(listbox::Hit::Row(_)),
            ..
        }) = hit
            && matches!(self.items[item].control, Control::ListBox { .. })
        {
            self.click(hit);
            return Event::Default;
        }
        self.click(hit)
    }

    /// The wheel turned `rows` rows (down positive) over `hit`. An open combo's list scrolls
    /// wherever the pointer is; an unfocused closed combo ignores the wheel.
    pub fn wheel(&mut self, hit: Option<Hit>, rows: i32) -> Event {
        if let Some(i) = self.open_combo()
            && let Control::Combo(c) = &mut self.items[i].control
        {
            return if c.scroll(rows) {
                Event::Redraw
            } else {
                Event::None
            };
        }
        let Some(h) = hit else {
            return Event::None;
        };
        let i = h.item;
        let cm = self.rendered.controls.get();
        let rect = self.at(&self.items[i]);
        let grows = self.items[i].grows;
        let focused = self.focus() == Some(i);
        let moved = match &mut self.items[i].control {
            Control::Combo(_) if !focused => false,
            Control::Combo(c) => {
                let notches = rows.signum() * (rows.abs() / WHEEL_ROWS).max(1);
                let mut changed = false;
                for _ in 0..notches.abs() {
                    changed |= c.step(notches > 0);
                }
                return if changed {
                    Event::Changed(self.items[i].id)
                } else {
                    Event::None
                };
            }
            Control::ListBox { list, .. } => {
                let visible = listbox::layout(list, rect, &cm).visible;
                list.scroll(rows, visible)
            }
            Control::Tree(tv) => {
                let lay = treeview::layout_without_text(tv, rect, &cm);
                tv.scroll(rows as i64, &lay)
            }
            Control::IconList(l) => {
                let notches = rows.signum() * (rows.abs() / WHEEL_ROWS).max(1);
                l.scroll(notches as i64, &icon_list_layout(grows, rect, &cm))
            }
            Control::Memo(memo) if memo.enabled => {
                let mut view = memo.view.get();
                let moved = view.scroll_lines(rows as i64, &memo.edit, memo.rows.get());
                memo.view.set(view);
                moved
            }
            _ => false,
        };
        if moved { Event::Redraw } else { Event::None }
    }

    /// A reader moved the focus to the control at (`x`, `y`), as Tab would: a button is not
    /// pressed, a check box not flipped, a radio not picked.
    pub fn focus_at(&mut self, x: i32, y: i32, text: &TextRenderer, px: u32) -> Event {
        let Some(i) = self.hit(x, y, text, px).map(|h| h.item) else {
            return Event::None;
        };
        if !self.items[i].focusable() || self.focus == Some(i) {
            return Event::None;
        }
        self.close_popups();
        self.set_focus(Some(i));
        Event::Redraw
    }

    /// A reader asked to see row `row` of the list box, tree or open drop list at (`x`, `y`): it
    /// scrolls in, nothing is picked.
    pub fn reveal(&mut self, x: i32, y: i32, row: usize, text: &TextRenderer, px: u32) -> Event {
        let Some(i) = self.hit(x, y, text, px).map(|h| h.item) else {
            return Event::None;
        };
        let cm = self.rendered.controls.get();
        let rect = self.at(&self.items[i]);
        let moved = match &mut self.items[i].control {
            Control::ListBox { list, .. } => {
                let visible = listbox::layout(list, rect, &cm).visible;
                list.reveal(row, visible)
            }
            Control::Tree(tv) => {
                let lay = treeview::layout_without_text(tv, rect, &cm);
                tv.reveal(row, &lay)
            }
            Control::Combo(c) => c.reveal(row),
            _ => false,
        };
        if moved { Event::Redraw } else { Event::None }
    }

    fn grab_bar(&mut self, i: usize, thumb: ThumbDrag) {
        if thumb.holding() {
            self.drag = Some(Drag::Bar { item: i, thumb });
        }
    }

    /// The pointer let go. A held push button, check box or radio acts only when released over
    /// it; everything else acted on the press.
    pub fn release(&mut self) -> Event {
        if let Some(Drag::Arrange(i)) = self.drag.take()
            && let Some(it) = self.items.get_mut(i)
            && let Control::Arrange(a) = &mut it.control
            && a.drop_held()
        {
            self.pointer.release();
            return Event::Changed(it.id);
        }
        let before = self.shown();
        let acts = |f: &Form, i: usize| {
            matches!(
                f.items.get(i).map(|it| &it.control),
                Some(Control::Button { .. } | Control::Check { .. } | Control::Radio { .. })
            )
        };
        let held = self.pointer.held().is_some_and(|(i, _)| acts(self, i));
        match self.pointer.release() {
            Some((i, Light::Body)) if acts(self, i) => self.act(i),
            _ if held => Event::Redraw,
            _ => self.redraw_if(before),
        }
    }

    /// A push button, check box or radio released over: a greyed one does nothing.
    fn act(&mut self, i: usize) -> Event {
        let id = self.items[i].id;
        match &mut self.items[i].control {
            Control::Button { enabled: true, .. } => Event::Pressed(id),
            Control::Check {
                checked,
                enabled: true,
                mixed,
                ..
            } => {
                toggle_check(checked, mixed);
                Event::Changed(id)
            }
            Control::Radio { enabled: true, .. } => {
                // Same convention as the keys: re-picking the radio that
                // is already checked mutates nothing, so it is a Redraw.
                let ch = self.pick_radio(i);
                self.changed(i, ch)
            }
            _ => Event::Redraw,
        }
    }
}
