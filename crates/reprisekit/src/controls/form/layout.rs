use crate::classic::widgets::LINE_H;
use crate::controls::combo;
use crate::controls::fit;
use crate::controls::iconlist;
use crate::controls::metrics::ControlMetrics;
use crate::controls::treeview;
use crate::controls::updown;
use crate::raster::{Canvas, Rect};
use crate::text::{Align, TextRenderer};
use crate::theme::TextUnits;

use super::item::{Indent, Item};
use super::{CHECK, Control, Form, RADIO_ROW};

impl Form {
    /// The rect item `it` is clipped to on the surface: the [`View`](super::View)'s, unless it is kept.
    pub(super) fn clip_of(&self, it: &Item) -> Option<Rect> {
        let v = self.view.filter(|v| !v.keep.contains(&it.id))?;
        let r = self.units().rect(v.rect);
        Some(Rect::new(
            r.x + self.origin.0,
            r.y + self.origin.1,
            r.width,
            r.height,
        ))
    }

    /// Where `it` draws and takes the pointer at the last draw's text size: its
    /// [`Form::flowed`] rect [`place`]d, then moved and widened for the text ([`Form::fit_text`]).
    pub(super) fn at(&self, it: &Item) -> Rect {
        let r = place(
            it,
            self.flowed(it),
            &self.controls(),
            self.units(),
            self.origin,
        );
        let fit = self.rendered.fit.borrow();
        // A copy of an item (a host's one-item form) is not one of `self.items` and keeps its rect.
        match self
            .items
            .iter()
            .position(|x| std::ptr::eq(x, it))
            .and_then(|i| fit.get(i))
        {
            Some(&(dx, dw)) => Rect {
                x: r.x + dx,
                width: r.width + dw,
                ..r
            },
            _ => r,
        }
    }

    /// `it`'s 100% rect after the notes the last draw found too short for their lines
    /// ([`Form::lay_out_notes`]): such a note takes its lines' height, an item at or below its
    /// bottom moves down by the growth and an item around it grows by as much. Items the
    /// [`View`](super::View) keeps stay.
    pub fn flowed(&self, it: &Item) -> Rect {
        let flow = self.rendered.flow.borrow();
        let mut r = it.rect;
        let Some(view) = self.view.filter(|_| !flow.is_empty()) else {
            return r;
        };
        if view.keep.contains(&it.id) {
            return r;
        }
        for &(k, grow) in flow.iter() {
            let Some(note) = self.items.get(k) else {
                continue;
            };
            let n = note.rect;
            if std::ptr::eq(note, it) {
                r.height += grow;
            } else if it.rect.y >= n.bottom() {
                r.y += grow;
            } else if it.rect.y <= n.y
                && it.rect.x <= n.x
                && it.rect.right() >= n.right()
                && it.rect.bottom() >= n.bottom()
            {
                r.height += grow;
            }
        }
        r
    }

    /// The 100% rect, where the form has it now, of the part of the focused control the
    /// keyboard rests on: a tree's selected row while it shows in the tree, else the control's
    /// [`Form::flowed`] rect. None with no control focused.
    pub fn focus_part(&self) -> Option<Rect> {
        let it = self.items.get(self.focus?)?;
        let row = self.shown_row(it).map(|r| self.units().covering(r));
        Some(row.unwrap_or_else(|| self.flowed(it)))
    }

    /// [`Form::focus_part`] in px at the last draw's text size: a tree's row as it draws, else
    /// the flowed rect grown.
    pub fn focus_part_px(&self) -> Option<Rect> {
        let it = self.items.get(self.focus?)?;
        Some((self.shown_row(it)).unwrap_or_else(|| self.units().rect(self.flowed(it))))
    }

    /// Tree `it`'s selected row in px, the origin taken off, while it shows in the tree.
    fn shown_row(&self, it: &Item) -> Option<Rect> {
        let Control::Tree(tv) = &it.control else {
            return None;
        };
        let lay = treeview::layout_without_text(tv, self.at(it), &self.controls());
        let k = (tv.selected_row(&tv.visible()))
            .and_then(|i| i.checked_sub(tv.top))
            .filter(|&k| k < lay.rows)?;
        let (b, (ox, oy)) = (lay.body, self.origin);
        let y = b.y + k as i32 * lay.row_h - oy;
        Some(Rect::new(b.x - ox, y, b.width, lay.row_h))
    }

    /// Scrolls the focused tree's rows so its selected row is the first shown
    /// ([`TreeView::lift`](treeview::TreeView::lift)). True when they moved.
    pub fn lift_focus_part(&mut self) -> bool {
        let Some(i) = self.focus else {
            return false;
        };
        let Control::Tree(tv) = &mut self.items[i].control else {
            return false;
        };
        (tv.selected_row(&tv.visible())).is_some_and(|row| tv.lift(row))
    }

    /// Lay the notes out at the last draw's text size, scale `s` and font size `px`: in a form
    /// with a [`View`](super::View), which scrolls through what it does not show, a note whose lines rewrap
    /// to more than its rect holds grows ([`Form::flowed`]). [`draw`](fn@super::draw) does this itself.
    pub fn lay_out_notes(&self, text: &TextRenderer, s: i32, px: u32) {
        self.rendered.flow.borrow_mut().clear();
        if self.view.is_none() {
            return;
        }
        let text = text.dialog(self.font);
        let flow = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(i, it)| {
                let (Control::Lines(lines) | Control::GreyedLines(lines)) = &it.control else {
                    return None;
                };
                let width = self.at(it).width * s;
                let grow =
                    note_lines(text, lines, width, px).len() as i32 * LINE_H - it.rect.height;
                (grow > 0).then_some((i, grow))
            })
            .collect();
        *self.rendered.flow.borrow_mut() = flow;
    }

    /// The rects every item takes as built, before [`Form::fit_text`] moves them and notes
    /// flow, with what each holds for the fit: text drawn at `px` on a paint at scale `s`, in
    /// 1x px.
    fn fit_input(&self, text: &TextRenderer, px: u32, s: i32) -> (Vec<Rect>, Vec<fit::Kind>) {
        let (cm, u) = (self.controls(), self.units());
        // A glyph run drawn at `s` times the 1x size can ink wider than `s` times its 1x width:
        // the 1x need holds both.
        let ink = |plain: &str| {
            let one = text.ink_width(plain, false, px / s as u32);
            match s {
                1 => one,
                _ => one.max((text.ink_width(plain, false, px) + s - 1) / s),
            }
        };
        let measure = |t: &str| ink(&crate::text::split_mnemonic(t).0);
        self.items
            .iter()
            .map(|it| {
                let r = place(it, it.rect, &cm, u, self.origin);
                let kind = match &it.control {
                    Control::Label {
                        text,
                        align: Align::Left,
                        ..
                    } => fit::Kind::Text {
                        need: measure(text),
                    },
                    Control::Link { label, .. } => fit::Kind::Text { need: ink(label) },
                    Control::Label {
                        text,
                        align: Align::Right,
                        ..
                    } => fit::Kind::TextRight {
                        need: measure(text),
                    },
                    Control::Combo(c) if c.elide => fit::Kind::Stretch { need: 0 },
                    Control::Combo(c) => {
                        let inset = r.width - combo::layout(c, r, &cm).text.width;
                        let widest = c.items.iter().map(|i| measure(&i.text)).max();
                        fit::Kind::Stretch {
                            need: widest.map_or(0, |w| w + inset),
                        }
                    }
                    Control::UpDown(ud) if ud.buddy => {
                        let edit = updown::layout_for(ud, r, &cm, u).edit;
                        let widest = [ud.min, ud.max].map(|v| measure(&v.to_string()));
                        fit::Kind::Stretch {
                            need: widest.into_iter().max().unwrap_or(0) + r.width - edit.width,
                        }
                    }
                    Control::Edit { .. }
                    | Control::ListBox { .. }
                    | Control::Tree(_)
                    | Control::TrackBar(_)
                    | Control::Memo(_) => fit::Kind::Stretch { need: 0 },
                    Control::Button { label, .. } => fit::Kind::Button {
                        need: measure(label) + 2 * fit::BUTTON_PAD,
                    },
                    Control::Group(_) => fit::Kind::Group,
                    Control::GroupTitle(_)
                    | Control::Blank
                    | Control::Separator
                    | Control::Rule => fit::Kind::Apart,
                    _ => fit::Kind::Fixed,
                };
                (r, kind)
            })
            .unzip()
    }

    /// Lays the form out for the text `text` sets at `px` on a paint at scale `s`: a label or a
    /// combo's value wider than its rect takes the room right of it, moving the column after it
    /// when there is too little ([`fit::fit`]). Run by `draw` after [`Form::drawn_with`] and
    /// before [`Form::lay_out_notes`], which rewraps the notes at the widths the fit leaves.
    pub(super) fn fit_text(&self, text: &TextRenderer, px: u32, s: i32) {
        let text = text.dialog(self.font);
        let (rects, kinds) = self.fit_input(text, px, s.max(1));
        let fits = rects.iter().zip(&kinds).all(|(r, k)| match *k {
            fit::Kind::Text { need }
            | fit::Kind::TextRight { need }
            | fit::Kind::Stretch { need }
            | fit::Kind::Button { need } => need <= r.width,
            _ => true,
        });
        let mut deltas = self.rendered.fit.borrow_mut();
        deltas.clear();
        if fits {
            return;
        }
        let u = self.units();
        let out = fit::fit(&rects, &kinds, u.len(fit::GAP), u.len(fit::PAD));
        deltas.extend(
            rects
                .iter()
                .zip(&out)
                .map(|(a, b)| (b.x - a.x, b.width - a.width)),
        );
    }

    /// Keeps the width each combo's open list takes at the text `text` sets at `px` on a paint at
    /// scale `s`, and `c`'s logical rect, for [`Form::combo_layout`]. Run by `draw`, closed combos
    /// too: a host asks for a hosted list's [`Spot`](crate::controls::hosted::Spot) as it opens,
    /// before the next draw. A combo whose rows, text and scale are unchanged keeps its width.
    pub(super) fn measure_lists(&self, c: &Canvas, text: &TextRenderer, px: u32, s: i32) {
        let text = text.dialog(self.font);
        let cm = self.controls();
        let mut need = self.rendered.list_need.borrow_mut();
        need.resize(self.items.len(), None);
        for (it, kept) in self.items.iter().zip(need.iter_mut()) {
            let Control::Combo(cb) = &it.control else {
                *kept = None;
                continue;
            };
            let row_h = cb.row_h(&cm);
            let key = list_key(cb, row_h, px, s);
            if kept.is_none_or(|(k, _)| k != key) {
                *kept = Some((key, combo::rows_need(cb, row_h, text, px, s)));
            }
        }
        let b = c.bounds();
        let s = s.max(1);
        self.rendered
            .bounds
            .set(Rect::new(0, 0, b.width / s, b.height / s));
    }

    /// Combo item `i`'s layout in `field`, its open list widened to its longest row as far as the
    /// last draw's canvas allows. Draw, hit, access and a hosted list all lay the list out here.
    pub(super) fn combo_layout(&self, i: usize, cb: &combo::Combo, field: Rect) -> combo::Layout {
        let mut lay = combo::layout(cb, field, &self.controls());
        let need = self
            .rendered
            .list_need
            .borrow()
            .get(i)
            .copied()
            .flatten()
            .map_or(0, |(_, need)| need);
        let mut bounds = self.rendered.bounds.get();
        if bounds.is_empty() {
            return lay;
        }
        // A popup's list stays where the compositor put it.
        if cb.list_at.is_some() {
            bounds.x = lay.list.x;
        }
        combo::widen_within_to(&mut lay, need, bounds);
        lay
    }

    /// The items whose text `text` at `px` does not fit once [`Form::fit_text`] and
    /// [`Form::lay_out_notes`] have laid them out at 1x, with how many px it runs over: a label
    /// or a combo's widest value wider than its rect, a check box's or radio's label running
    /// into the next control on its row or past its group box, and a note that wraps to more
    /// rows than it was built with, even where it flows and pushes the page down, or keeps a
    /// line wider than its rect.
    pub fn overflows(&self, text: &TextRenderer, px: u32) -> Vec<(&'static str, i32)> {
        self.fit_text(text, px, 1);
        self.lay_out_notes(text, 1, px);
        let text = text.dialog(self.font);
        let (cm, u) = (self.controls(), self.units());
        let measure = |s: &str| text.ink_width(&crate::text::split_mnemonic(s).0, false, px);
        let lead = |s: &str| text.ink_before(&crate::text::split_mnemonic(s).0, false, px);
        let room = self.choice_room(text, px);
        let (rects, kinds) = (&room.rects, &room.kinds);
        let mut over = Vec::new();
        for (i, it) in self.items.iter().enumerate() {
            let r = rects[i];
            let by = match (&it.control, kinds[i]) {
                (Control::Label { text, .. }, _) => measure(text) - r.width,
                (Control::Link { label, .. }, _) => text.ink_width(label, false, px) - r.width,
                (Control::Combo(_), fit::Kind::Stretch { need }) => need - r.width,
                (Control::Check { label, .. }, _) => {
                    let lr = check_label_rect(r, it.indent, &cm, measure(label));
                    lr.right() + lead(label) - room.edge(i, lr.x, self.clip_of(it))
                }
                (Control::Radio { label, .. }, _) => {
                    let lr = radio_label_rect(r, &cm, measure(label));
                    lr.right() + lead(label) - room.edge(i, lr.x, self.clip_of(it))
                }
                (Control::Lines(lines) | Control::GreyedLines(lines), _) => {
                    let shown = note_lines(text, lines, r.width, px);
                    let room = (it.rect.height / LINE_H) as usize;
                    let rows = u.len(shown.len().saturating_sub(room) as i32 * LINE_H);
                    let widest = shown.iter().map(|l| text.measure(l, false, px)).max();
                    rows.max(widest.unwrap_or(0) - r.width)
                }
                _ => 0,
            };
            if by > 0 {
                over.push((it.id, by));
            }
        }
        over
    }

    /// The laid-out rects a check box's or radio's label runs in, for text `text` at `px`
    /// ([`Form::fit_text`] has run).
    pub(super) fn choice_room(&self, text: &TextRenderer, px: u32) -> ChoiceRoom {
        let (_, kinds) = self.fit_input(text, px, 1);
        let wide = |it: &Item| match it.control {
            Control::Check { .. } => it.rect.width > CHECK,
            Control::Radio { .. } => it.rect.width > RADIO_ROW,
            _ => false,
        };
        ChoiceRoom {
            rects: self.items.iter().map(|it| self.at(it)).collect(),
            wide: self.items.iter().map(wide).collect(),
            kinds,
            pad: self.units().len(2),
        }
    }

    /// The rect item `id` was last drawn in, on the surface: its rect grown with the text and
    /// moved to the form's origin and by its indent. For a host that draws over an item.
    pub fn rect(&self, id: &str) -> Option<Rect> {
        self.item(id).map(|it| self.at(it))
    }

    /// The layout icon list `id` was last drawn with.
    pub(crate) fn icon_list_layout(&self, id: &str) -> Option<iconlist::Layout> {
        let it = self.item(id)?;
        Some(icon_list_layout(it.grows, self.at(it), &self.controls()))
    }
}

/// An icon list's layout in `r`, its cells grown with the text when its item [`Item::grows`].
pub(super) fn icon_list_layout(grows: bool, r: Rect, cm: &ControlMetrics) -> iconlist::Layout {
    let u = if grows { cm.text } else { TextUnits::ONE };
    iconlist::layout_grown(r, cm, u)
}

/// Where `it` draws and takes the pointer, on the surface: its 100% `rect` grown by `u` and moved
/// to `origin`, then by its [`Indent`]. Pictures and controls with fixed art keep their size and
/// only move; a separator and a rule keep their thickness, and an icon list, whose
/// rows of fixed icons fill its height, keeps its height but for its grown scrollbar. Indents
/// keep their px, but one past a check box steps past the box grown with the text.
pub(super) fn place(
    it: &Item,
    rect: Rect,
    cm: &ControlMetrics,
    u: TextUnits,
    origin: (i32, i32),
) -> Rect {
    let r = match it.control {
        Control::Picture { frame, .. } if it.grows => {
            let f = 2 * frame;
            Rect {
                width: u.len(rect.width - f) + f,
                height: u.len(rect.height - f) + f,
                ..u.at(rect)
            }
        }
        Control::Picture { .. } | Control::Monitor(_) | Control::Spectrum => u.at(rect),
        Control::IconList(_) if it.grows => Rect {
            height: iconlist::grown_height(rect.height, u, cm.scroll_growth()),
            ..u.rect(rect)
        },
        Control::IconList(_) => Rect {
            height: rect.height + cm.scroll_growth(),
            ..u.rect(rect)
        },
        Control::Separator => Rect {
            height: rect.height,
            ..u.rect(rect)
        },
        Control::Rule if rect.width <= rect.height => Rect {
            width: rect.width,
            ..u.rect(rect)
        },
        Control::Rule => Rect {
            height: rect.height,
            ..u.rect(rect)
        },
        _ => u.rect(rect),
    };
    let placed = it.placed(cm);
    Rect {
        x: r.x + placed.x - it.rect.x + origin.0,
        y: r.y + placed.y - it.rect.y + origin.1,
        width: r.width + placed.width - it.rect.width,
        ..r
    }
}

/// The lines a note draws in `width` device px at `px`: its own, or its text rewrapped to the
/// width when a line is wider.
pub(super) fn note_lines<'a>(
    text: &TextRenderer,
    lines: &'a [String],
    width: i32,
    px: u32,
) -> std::borrow::Cow<'a, [String]> {
    if lines.iter().all(|l| text.measure(l, false, px) <= width) {
        lines.into()
    } else {
        crate::text::wrap(text, &lines.join(" "), width, px).into()
    }
}

/// How far down its placed rect a check box's or radio's glyph sits beyond where it does at
/// 100%: half the row's growth, so the glyph stays centred on its label's line. A glyph grown
/// with the text ([`grown_dy`]) rises by half its own growth.
pub(super) fn glyph_dy(it: &Item, placed: Rect) -> i32 {
    (placed.height - it.rect.height) / 2
}

/// Half of what a glyph `side` px at `cm`'s text grew by past its 100% size.
fn grown_dy(side: i32, cm: &ControlMetrics) -> i32 {
    (side - side / cm.text.small()) / 2
}

pub(super) fn check_box(r: Rect, cm: &ControlMetrics, dy: i32) -> Rect {
    let y = r.y + dy - grown_dy(cm.check, cm);
    Rect::new(r.x, y, cm.check, cm.check)
}

/// A check box's label: `cm.check_label` right of its item rect's left
/// edge, `w` wide, as tall as the item rect; `cm.check_wrap_dy` lower for
/// a label that runs onto a second line (`indent` [`Indent::Wraps`]).
pub(super) fn check_label_rect(r: Rect, indent: Indent, cm: &ControlMetrics, w: i32) -> Rect {
    let dy = if indent == Indent::Wraps {
        cm.check_wrap_dy
    } else {
        0
    };
    Rect::new(r.x + cm.check_label, r.y + dy, w, r.height)
}

/// A radio button's circle: `cm.radio` px square at its item rect's top,
/// `cm.radio_x` right of its left edge.
pub(super) fn radio_box(r: Rect, cm: &ControlMetrics, dy: i32) -> Rect {
    let y = r.y + dy - grown_dy(cm.radio, cm);
    Rect::new(r.x + cm.radio_x, y, cm.radio, cm.radio)
}

/// A radio button's label: `cm.radio_label` right of its item rect's left
/// edge, `w` wide, as tall as the item rect or the circle, whichever is
/// taller, so a 13 px circle's label sits on the same row as a check box's.
/// Grown text gets at least a grown line.
pub(super) fn radio_label_rect(r: Rect, cm: &ControlMetrics, w: i32) -> Rect {
    let line = if cm.text.is_one() {
        0
    } else {
        cm.text.len(LINE_H)
    };
    Rect::new(
        r.x + cm.radio_label,
        r.y,
        w,
        r.height.max(cm.radio).max(line),
    )
}

/// The rects and fit kinds a check box's or radio's label is bounded by, as laid out.
pub(super) struct ChoiceRoom {
    rects: Vec<Rect>,
    /// Built wider than its box at 100%: a grown box-sized rect is no edge for its label.
    wide: Vec<bool>,
    kinds: Vec<fit::Kind>,
    /// Kept inside a group box's frame.
    pad: i32,
}

impl ChoiceRoom {
    /// Where item `i`'s label starting at `from` may run to for [`Form::overflows`]: where
    /// drawing cuts it ([`ChoiceRoom::cut`], the view `clip`), else the form's right edge.
    fn edge(&self, i: usize, from: i32, clip: Option<Rect>) -> i32 {
        let form = fit::limit(&self.rects, &self.kinds, i, from, self.pad);
        let drawn = self.cut(i, from).into_iter().chain(clip.map(|v| v.right()));
        drawn.fold(form, i32::min)
    }

    /// Where item `i`'s label starting at `from` is cut: the next control on its row, its group
    /// box's frame, or the item's own right edge when it was built wider than the box.
    fn cut(&self, i: usize, from: i32) -> Option<i32> {
        let own = Some(self.rects[i].right()).filter(|&r| self.wide[i] && r > from);
        let stop = fit::stop(&self.rects, &self.kinds, i, from, self.pad);
        stop.into_iter().chain(own).min()
    }
}

/// The device px item `i`'s check box or radio label starting at `from` (logical px) has before
/// [`ChoiceRoom::cut`] or the edge of the window or clip `clip`.
pub(super) fn choice_label_room(
    room: Option<&ChoiceRoom>,
    i: usize,
    from: i32,
    clip: Option<Rect>,
    c: &Canvas,
    s: i32,
) -> i32 {
    let shown = clip.map_or(c.bounds(), |v| v.scaled(s).intersect(&c.bounds()));
    let cut = room
        .and_then(|r| r.cut(i, from))
        .map_or(i32::MAX, |x| x * s);
    cut.min(shown.right()) - from * s
}

/// What [`combo::rows_need`] measures `cb`'s rows from.
fn list_key(cb: &combo::Combo, row_h: i32, px: u32, s: i32) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = rustc_hash::FxHasher::default();
    (row_h, px, s).hash(&mut h);
    for it in &cb.items {
        (&it.text, it.indent, it.icon.is_some()).hash(&mut h);
    }
    h.finish()
}
