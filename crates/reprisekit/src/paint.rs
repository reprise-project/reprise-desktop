//! What a widget paints with: the [`Look`] a theme draws in (Classic's own
//! code, or a skinned [`Style`]), the [`Paint`] a widget gets (colours,
//! look, scale, what sits behind it) and the theme's accessors for them.
//! [`crate::look`] dispatches each surface on these and re-exports them.

use crate::raster::{Canvas, Rect};
use crate::style::{GlassSpec, Style};
use crate::taskbar::TaskbarShape;
use crate::text::{Align, CaretBar, TextRenderer};
use crate::textedit::{Caret, TextEdit, edit_first_visible};
use crate::theme::{ColorScheme, Theme};
use std::sync::Arc;

#[derive(Clone, Default)]
pub enum Look {
    #[default]
    Classic,
    Styled(Arc<Style>),
}

impl Look {
    pub fn style(&self) -> Option<&Style> {
        match self {
            Look::Classic => None,
            Look::Styled(s) => Some(s),
        }
    }

    pub fn at_scale(&self, n: i32) -> Look {
        match self {
            Look::Classic => Look::Classic,
            Look::Styled(s) => Look::Styled(s.at_scale(n)),
        }
    }

    /// The look's control sizes with every row at 100% text, whatever the text size: for art and
    /// fixed sizes. A row that holds a line of text takes [`Theme::controls`] instead.
    pub fn controls_at_100(&self) -> crate::metrics::ControlMetrics {
        self.style().map(|s| s.controls).unwrap_or_default()
    }

    /// Whether this look draws a control under the pointer or held apart
    /// from one at rest: a style with a Hot or Pressed frame for any of
    /// the common widgets, a list header or the dialog's ? box. A host redraws for a
    /// change of the part the pointer lights only then; Classic and a
    /// style with no control art redraw nothing for hover.
    pub fn lights_pointer(&self) -> bool {
        use crate::style::{Part, State};
        const LIT: [Part; 19] = [
            Part::PushButton,
            Part::CheckBox,
            Part::Radio,
            Part::ComboDrop,
            Part::SpinUp,
            Part::SpinDown,
            Part::TrackbarThumb,
            Part::TrackbarThumbV,
            Part::TabItem,
            Part::ScrollArrowUp,
            Part::ScrollArrowDown,
            Part::ScrollArrowLeft,
            Part::ScrollArrowRight,
            Part::ScrollThumbV,
            Part::ScrollThumbH,
            Part::ScrollTrackV,
            Part::ScrollTrackH,
            Part::HelpButton,
            Part::HeaderItem,
        ];
        self.style().is_some_and(|st| {
            LIT.iter().any(|&part| {
                st.part(part).is_some_and(|p| {
                    p.states
                        .iter()
                        .any(|s| matches!(s, State::Hot | State::Pressed))
                })
            })
        })
    }
}

impl PartialEq for Look {
    fn eq(&self, o: &Look) -> bool {
        match (self, o) {
            (Look::Classic, Look::Classic) => true,
            (Look::Styled(a), Look::Styled(b)) => (a.name, a.key) == (b.name, b.key),
            _ => false,
        }
    }
}

impl std::fmt::Debug for Look {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Look::Classic => f.write_str("Classic"),
            Look::Styled(s) => write!(f, "Styled({})", s.name),
        }
    }
}

/// What sits behind a control: the dialog face, or a property sheet's
/// page drawn into the given rect (the one [`crate::look::widgets::draw_page`] got).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Backdrop {
    #[default]
    Face,
    Page(Rect),
}

/// What a widget or control needs to paint: the colours, the look, the
/// scale, what sits behind it and whether its window is the active one.
/// [`Theme::paint`] builds one; a caller painting with another scheme or
/// scale overrides a field: `&Paint { s: 2 * p.s, ..*p }`.
#[derive(Clone, Copy)]
pub struct Paint<'a> {
    pub cs: &'a ColorScheme,
    pub look: &'a Look,
    pub s: i32,
    pub back: Backdrop,
    /// Whether the window painted into has the keyboard. While it is false
    /// a default push button draws as a plain one.
    pub active: bool,
    /// How a form laid out for 100% text grows ([`Theme::text`]).
    pub text: crate::theme::TextUnits,
    /// Whether labels underline their access keys ([`Theme::access_keys`]).
    pub access_keys: bool,
}

impl<'a> Paint<'a> {
    /// `label` as drawn: without its access key's marker while the underlines hide.
    pub fn label<'l>(&self, label: &'l str) -> std::borrow::Cow<'l, str> {
        crate::cues::Cues {
            focus: false,
            keys: self.access_keys,
        }
        .label(label)
    }

    pub fn on_page(&self, page: Rect) -> Paint<'a> {
        Paint {
            back: Backdrop::Page(page),
            ..*self
        }
    }

    /// The multiple a small control drawn with this paint grows by
    /// ([`crate::theme::TextUnits::small`]).
    pub fn small_k(&self) -> i32 {
        self.text.small()
    }

    /// The look a small control grown with the text draws in on `c`: a styled look built again at
    /// the grown device scale, so the grown control is a fresh drawing. Hand it to
    /// [`Paint::small`].
    pub fn small_look(&self, c: &Canvas) -> Look {
        self.look_grown(c, self.small_k())
    }

    /// [`Paint::small_look`] for a control laid out `k` times its 100% size.
    pub fn look_grown(&self, c: &Canvas, k: i32) -> Look {
        match k {
            ..=1 => self.look.clone(),
            k => self.look.at_scale(self.s.max(1) * c.scale() * k),
        }
    }

    /// This paint for a small control: `s` times [`Paint::small_k`], drawing in `look` from
    /// [`Paint::small_look`]. Its text is 100%, so a painter handed it grows nothing again.
    pub fn small<'b>(&self, look: &'b Look) -> Paint<'b>
    where
        'a: 'b,
    {
        Paint {
            s: self.s * self.small_k(),
            look,
            text: crate::theme::TextUnits::ONE,
            ..*self
        }
    }

    /// Fill `r` with what sits behind a control there: the face, and over
    /// it the page clipped to `r` when a styled page is under it. Outside
    /// the page, and under a look without its own page, this is the face.
    pub fn fill_back(&self, c: &mut Canvas, r: Rect) {
        c.fill_rect(r, self.cs.button_face);
        if let Backdrop::Page(page) = self.back
            && let Some(st) = self.look.style()
        {
            c.with_clip(r, |c| {
                st.draw_page(c, page, self.s);
            });
        }
    }
}

/// Draw `edit` left-aligned in `inner` from byte `first` (`edit_first_visible`): selection, caret
/// and, focused, the canvas' preedit with its `TextSpot`, which a secret field never shows. An
/// unfocused field hides its selection ([`TextEdit::unfocused`]). The caret goes where it covers
/// the least ink ([`CaretBar`]). `inner` sits at least 1 px inside its field's frame: the first
/// glyph's ink, and a caret before it, may draw there.
#[expect(clippy::too_many_arguments)]
pub fn draw_edit(
    c: &mut Canvas,
    text: &TextRenderer,
    edit: &TextEdit,
    inner: Rect,
    px: u32,
    p: &Paint,
    caret: Caret,
    first: usize,
) {
    edit_field(c, text, edit, inner, px, p, caret, first, None);
}

/// [`draw_edit`] while a drag over the field would drop its text at byte `at`: the caret's bar
/// stands there instead, whatever the blink.
#[expect(clippy::too_many_arguments)]
pub fn draw_edit_dropping(
    c: &mut Canvas,
    text: &TextRenderer,
    edit: &TextEdit,
    inner: Rect,
    px: u32,
    p: &Paint,
    caret: Caret,
    first: usize,
    at: usize,
) {
    edit_field(c, text, edit, inner, px, p, caret, first, Some(at));
}

#[expect(clippy::too_many_arguments)]
fn edit_field(
    c: &mut Canvas,
    text: &TextRenderer,
    edit: &TextEdit,
    inner: Rect,
    px: u32,
    p: &Paint,
    caret: Caret,
    first: usize,
    drop: Option<usize>,
) {
    let Paint { cs, s, .. } = *p;
    let unfocused;
    let own = if caret.focused() {
        edit
    } else {
        unfocused = edit.unfocused();
        &*unfocused
    };
    // With a preedit the text is drawn with it spliced in, selection gone (text-input-v3 removes
    // it).
    let preedit = c
        .preedit()
        .filter(|_| caret.focused() && !own.is_secret())
        .cloned();
    let composed;
    let (edit, first, run, bar_on) = match &preedit {
        Some(p) => {
            let (e, run, bar_on) = p.compose(own);
            composed = e;
            let at = composed.selection().start;
            let from = edit_first_visible(text, &composed, inner.width, px, first.min(at));
            (&composed, from, Some(run), bar_on)
        }
        None => (own, first, None, true),
    };
    let at = |i: usize| {
        if i < first {
            inner.x
        } else {
            inner.x + text.measure(&edit.text[first..i], false, px)
        }
    };
    let room = Rect::new(inner.x - s, inner.y, inner.width + s, inner.height);
    let line = text
        .line(inner, false, px)
        .map_or(inner, |(y, h)| Rect::new(inner.x, y, inner.width, h))
        .intersect(&inner);
    let shown = &edit.text[first..];
    let sel = edit.selection();
    let sel = sel.start.max(first)..sel.end.max(first);
    let hl = (!sel.is_empty()).then(|| {
        let (x0, x1) = (at(sel.start), at(sel.end).min(inner.right()));
        let run = Rect::new(x0, line.y, x1 - x0, line.height);
        let sel = sel.start - first..sel.end - first;
        text.highlight(run, shown, sel, inner.x, room, false, px, c.scale())
            .clear_of_ink(room)
    });
    // The device pixels past the run are filled under the text, so a neighbour's ink there
    // stays the text colour.
    if let Some(hl) = &hl {
        hl.fill(c, room, cs.hilight);
    }
    let bar_at = |i: usize| Rect::new(at(i).min(inner.right() - s), line.y, s, line.height);
    let scale = c.scale();
    let x_at = |i: usize| {
        let x = text.caret_x(shown, i.max(first) - first, inner.x, false, px, scale);
        x.min(scale * (inner.right() - s))
    };
    let bar = bar_at(edit.caret());
    let drawn = match drop {
        Some(i) => Some(edit.text.floor_char_boundary(i)),
        None => (caret.shown() && bar_on).then_some(edit.caret()),
    };
    let shown_bar = drawn.map(|i| CaretBar::ground_at(c, bar_at(i), x_at(i), hl.as_ref(), room));
    if edit.text.is_empty() && !edit.cue().is_empty() {
        text.draw(c, inner, edit.cue(), false, cs.gray_text, Align::Left, px);
    } else {
        text.draw_in(c, inner, room, room, shown, false, cs.button_text, px);
    }
    if let Some(hl) = &hl {
        c.fill_rect(hl.rect, cs.hilight);
        text.draw_in(c, inner, room, hl.rect, shown, false, cs.hilight_text, px);
        text.draw_overhang(c, hl, room, cs.hilight_text);
    }
    if let Some(run) = run {
        let base = text
            .baseline(inner, false, px)
            .unwrap_or(inner.bottom() - 3 * s);
        let (x0, x1) = (at(run.start.max(first)), at(run.end.max(first)));
        let ul = Rect::new(x0, base + s, x1 - x0, s).intersect(&inner);
        c.fill_rect(ul, cs.button_text);
    }
    if let Some(bar) = shown_bar {
        bar.draw(c, cs.hilight_text);
    }
    if caret.focused() {
        c.report_spot(own.spot(inner, bar));
    }
}

#[cfg(test)]
pub(crate) fn classic_paint(cs: &ColorScheme, s: i32) -> Paint<'_> {
    static CLASSIC: Look = Look::Classic;
    Paint {
        cs,
        look: &CLASSIC,
        s,
        back: Backdrop::Face,
        active: true,
        text: crate::theme::TextUnits::ONE,
        access_keys: true,
    }
}

/// The bar a caret drew over `off` to give `on`, every pixel of it `off`'s inverted, and whether
/// it covers ink (a pixel other than `paper`).
#[cfg(test)]
#[track_caller]
pub(crate) fn inverted_bar(off: &Canvas, on: &Canvas, paper: u32) -> (Rect, bool) {
    let (w, h) = (off.width as i32, off.height as i32);
    let changed: Vec<(i32, i32)> = (0..h)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .filter(|&(x, y)| on.get(x, y) != off.get(x, y))
        .collect();
    assert!(!changed.is_empty(), "the caret changes pixels");
    for &(x, y) in &changed {
        let want = 0xFF00_0000 | (!off.get(x, y) & 0x00FF_FFFF);
        assert_eq!(on.get(x, y), want, "({x}, {y}) is inverted");
    }
    let x0 = changed.iter().map(|p| p.0).min().unwrap();
    let x1 = changed.iter().map(|p| p.0).max().unwrap() + 1;
    let y0 = changed.iter().map(|p| p.1).min().unwrap();
    let y1 = changed.iter().map(|p| p.1).max().unwrap() + 1;
    let bar = Rect::new(x0, y0, x1 - x0, y1 - y0);
    assert_eq!(
        changed.len() as i32,
        bar.width * bar.height,
        "{bar:?} is whole"
    );
    (bar, changed.iter().any(|&(x, y)| off.get(x, y) != paper))
}

impl Theme {
    /// The look's bar shape at this theme's text size.
    pub fn taskbar_shape(&self) -> TaskbarShape {
        let shape = self
            .look
            .style()
            .map_or(TaskbarShape::CLASSIC, |s| s.taskbar);
        TaskbarShape {
            line_growth: self.line_growth(),
            ..shape
        }
        .with_icon_grow(self.metrics.icon_grow / self.scale.max(1))
    }

    /// [`Theme::with_text_size`] for a bar at the look's own height: a bar that sets the clock on
    /// two lines at 100% text grows to hold both grown lines. A `[metrics] taskbar_height` takes
    /// plain [`Theme::with_text_size`].
    pub fn with_text_size_own_bar(&self, pct: u32) -> Theme {
        let mut t = self.with_text_size(pct);
        let two_lines = (self.taskbar_shape())
            .clock_baselines(self.metrics.taskbar_height)
            .is_some();
        let shape = t.taskbar_shape();
        if two_lines && let Some(b) = shape.two_line_clock {
            let need = shape.grown_lines(b).height;
            t.metrics.taskbar_height = t.metrics.taskbar_height.max(need);
        }
        t
    }

    /// This theme's colours and look, at its own scale, never below 1,
    /// for a window as active as [`Theme::active`] says.
    pub fn paint(&self) -> Paint<'_> {
        Paint {
            cs: &self.colors,
            look: &self.look,
            s: self.scale.max(1),
            back: Backdrop::Face,
            active: self.active,
            text: self.text,
            access_keys: self.access_keys,
        }
    }

    pub fn glass(&self) -> Option<GlassSpec> {
        self.look.style().and_then(|s| s.glass)
    }
}

/// Where a selection of one char of a two-char text recolours wrongly at scale 2: a neighbour's
/// ink outside the scale 1 highlight that changes colour from the text drawn unselected, a pixel
/// changed in a device column outside both the scale 1 highlight and the selected glyph's ink
/// with two columns either side (and, selected from the text's start, the column left of its
/// pen), or a selected glyph `lit` whole at scale 1 but not across the highlight's rows at scale 2.
/// `draw(s, text, sel)` returns the pixels of a `w` px wide canvas at scale `s`, any caret shown
/// at `sel.start`; `geom(text)` gives its pen, the rect its text is laid in and the highlight's
/// rows.
#[cfg(test)]
pub(crate) fn selection_faults(
    text: &TextRenderer,
    px: u32,
    (w, h): (i32, i32),
    pairs: impl IntoIterator<Item = String>,
    geom: impl Fn(&str) -> (i32, Rect, std::ops::Range<i32>),
    draw: impl Fn(i32, &str, std::ops::Range<usize>) -> Vec<u32>,
    lit: crate::theme::Color,
) -> Vec<String> {
    let mut faults = Vec::new();
    for t in pairs {
        let (pen, line, rows) = geom(&t);
        let mask = |s: i32, at: usize| glyph_mask(text, px, (w, h), &t, at, pen, line, s);
        let rows2 = rows.start * 2..rows.end * 2;
        for (k, (a, b, run)) in one_char_runs(text, px, &t, pen, &rows)
            .into_iter()
            .enumerate()
        {
            let other = if k == 0 { b } else { 0 };
            let run = run.scaled(2);
            let plain = draw(2, &t, a..a);
            let (own, near, lit2) = (mask(2, a), mask(2, other), draw(2, &t, a..b));
            let at = |i: usize, s: i32| (i as i32 % (w * s), i as i32 / (w * s));
            let recoloured = (0..near.len())
                .filter(|&i| near[i] && !own[i] && lit2[i] != plain[i])
                .filter(|&i| !run.contains(at(i, 2).0, at(i, 2).1))
                .count();
            let (l, r) = (0..own.len())
                .filter(|&i| own[i])
                .map(|i| at(i, 2).0)
                .fold((run.x, run.right()), |(l, r), x| {
                    (l.min(x - 2), r.max(x + 3))
                });
            let l = if a == 0 { l.min(2 * (pen - 1)) } else { l };
            let filled = (0..lit2.len())
                .filter(|&i| lit2[i] != plain[i] && rows2.contains(&at(i, 2).1))
                .filter(|&i| !(l..r).contains(&at(i, 2).0))
                .count();
            let unlit = |s: i32, own: &[bool], pixels: &[u32], rows: std::ops::Range<i32>| {
                (0..own.len())
                    .filter(|&i| own[i] && rows.contains(&at(i, s).1) && pixels[i] != lit.argb())
                    .count()
            };
            let split = match unlit(1, &mask(1, a), &draw(1, &t, a..b), 0..h) {
                0 => unlit(2, &own, &lit2, rows2.clone()),
                _ => 0,
            };
            if recoloured + filled + split > 0 {
                let which = ["first", "second"][k];
                faults.push(format!(
                    "\"{t}\" {which} selected: {recoloured} neighbour px recoloured, \
                     {filled} px changed past the ink, {split} own px unlit"
                ));
            }
        }
    }
    faults
}

/// Where a caret at the end of one selected char of a two-char text at scale 2 changes a pixel a
/// glyph inks, or, with no ink in the two device columns right of the highlight's `hilight` fill,
/// changes other columns than those. `draw(text, sel, shown)` returns the scale 2 pixels of a `w`
/// px wide canvas with the caret at `sel.end`; `geom` as in [`selection_faults`].
#[cfg(test)]
pub(crate) fn caret_faults(
    text: &TextRenderer,
    px: u32,
    (w, h): (i32, i32),
    pairs: impl IntoIterator<Item = String>,
    geom: impl Fn(&str) -> (i32, Rect, std::ops::Range<i32>),
    draw: impl Fn(&str, std::ops::Range<usize>, bool) -> Vec<u32>,
    hilight: crate::theme::Color,
) -> Vec<String> {
    let w2 = 2 * w;
    let mut faults = Vec::new();
    for t in pairs {
        let (pen, line, rows) = geom(&t);
        let ink = ink_mask(text, px, (w, h), &t, pen, line, 2);
        for (k, (a, b, run)) in one_char_runs(text, px, &t, pen, &rows)
            .into_iter()
            .enumerate()
        {
            let (off, on) = (draw(&t, a..b, false), draw(&t, a..b, true));
            let filled = |x: i32| {
                (rows.start * 2..rows.end * 2).any(|y| off[(y * w2 + x) as usize] == hilight.argb())
            };
            let mut end = run.right() * 2;
            while end < w2 && filled(end) {
                end += 1;
            }
            let changed: Vec<usize> = (0..on.len()).filter(|&i| on[i] != off[i]).collect();
            let flipped = changed.iter().filter(|&&i| ink[i]).count();
            let mut cols: Vec<i32> = changed.iter().map(|&i| i as i32 % w2).collect();
            cols.sort_unstable();
            cols.dedup();
            let free =
                (0..ink.len()).all(|i| !ink[i] || !(end..end + 2).contains(&(i as i32 % w2)));
            if flipped > 0 || (free && cols != [end, end + 1]) {
                let which = ["first", "second"][k];
                faults.push(format!(
                    "\"{t}\" {which} selected: caret in columns {cols:?}, fill ends at {end}, \
                     {flipped} ink px flipped"
                ));
            }
        }
    }
    faults
}

/// Each char of `t` as its byte range and its scale 1 highlight over `rows`, drawn from `pen`.
#[cfg(test)]
fn one_char_runs(
    text: &TextRenderer,
    px: u32,
    t: &str,
    pen: i32,
    rows: &std::ops::Range<i32>,
) -> Vec<(usize, usize, Rect)> {
    let starts: Vec<usize> = t.char_indices().map(|(i, _)| i).collect();
    (0..starts.len())
        .map(|k| {
            let (a, b) = (starts[k], starts.get(k + 1).copied().unwrap_or(t.len()));
            let x = pen + text.measure(&t[..a], false, px);
            let width = text.measure(&t[a..b], false, px);
            (a, b, Rect::new(x, rows.start, width, rows.len() as i32))
        })
        .collect()
}

/// The device pixels the char of `t` at byte `at` inks alone at scale `s`, drawn in `line` as
/// the text's run puts it.
#[cfg(test)]
#[expect(clippy::too_many_arguments)]
fn glyph_mask(
    text: &TextRenderer,
    px: u32,
    (w, h): (i32, i32),
    t: &str,
    at: usize,
    pen: i32,
    line: Rect,
    s: i32,
) -> Vec<bool> {
    let ink = crate::theme::Color::rgb(0x12, 0x34, 0x56);
    let base = text.baseline(line, false, px).unwrap();
    let end = t[at..]
        .chars()
        .next()
        .map_or(t.len(), |c| at + c.len_utf8());
    let mut c = Canvas::with_scale(w as u32, h as u32, s);
    text.draw_part(&mut c, pen, base, line, line, t, at..end, false, ink, px);
    c.pixels.iter().map(|&p| p == ink.argb()).collect()
}

/// Where a selection of `t`'s bytes `sel` draws off its highlight at scale `s`: pixels outside
/// the box of `hilight` pixels that differ from the text drawn unselected, pixels the selected
/// glyphs ink alone that are not `lit`, and `lit` ink beside a pixel no glyph inks that is `lit`
/// too, where the glyph's edge is lost: left or right of it in a column no other glyph inks in the
/// highlight's rows, or above or below it in `line`'s rows.
/// `plain` and `selected` are the pixels of a `w` px wide canvas; `pen` and `line` as in
/// [`selection_faults`]'s `geom`.
#[cfg(test)]
#[expect(clippy::too_many_arguments)]
pub(crate) fn off_highlight(
    text: &TextRenderer,
    px: u32,
    (w, h): (i32, i32),
    (t, sel): (&str, std::ops::Range<usize>),
    (pen, line): (i32, Rect),
    s: i32,
    (plain, selected): (&[u32], &[u32]),
    (hilight, lit): (crate::theme::Color, crate::theme::Color),
) -> (usize, usize, usize) {
    let ws = w * s;
    let at = |i: usize| (i as i32 % ws, i as i32 / ws);
    let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for (x, y) in (0..selected.len())
        .filter(|&i| selected[i] == hilight.argb())
        .map(at)
    {
        (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1));
    }
    let off = (0..selected.len())
        .filter(|&i| selected[i] != plain[i])
        .filter(|&i| {
            let (x, y) = at(i);
            !(x0..x1).contains(&x) || !(y0..y1).contains(&y)
        })
        .count();
    let mut own = vec![false; selected.len()];
    for (i, _) in t[sel.clone()].char_indices() {
        let mask = glyph_mask(text, px, (w, h), t, sel.start + i, pen, line, s);
        own.iter_mut().zip(mask).for_each(|(o, m)| *o |= m);
    }
    let unlit = (0..own.len())
        .filter(|&i| own[i] && selected[i] != lit.argb())
        .count();
    let mut neighbour = vec![false; ws as usize];
    for i in (0..own.len()).filter(|&i| !own[i] && plain[i] != lit.argb()) {
        let (x, y) = at(i);
        neighbour[x as usize] |= (y0..y1).contains(&y);
    }
    let paper = |i: usize| !own[i] && selected[i] == lit.argb() && plain[i] == lit.argb();
    let lost = |i: usize| paper(i) && !neighbour[at(i).0 as usize];
    let rows = s * line.y..s * line.bottom();
    let lost_row = |i: usize| paper(i) && rows.contains(&at(i).1);
    let wu = ws as usize;
    let merged = (0..own.len())
        .filter(|&i| own[i] && selected[i] == lit.argb())
        .filter(|&i| {
            let x = at(i).0;
            (x > 0 && lost(i - 1))
                || (x + 1 < ws && lost(i + 1))
                || (i >= wu && lost_row(i - wu))
                || (i + wu < own.len() && lost_row(i + wu))
        })
        .count();
    (off, unlit, merged)
}

/// A row of text for [`lost_selected_ink`]: its text, its selected bytes, its pen x and the rect
/// its text is laid in.
#[cfg(test)]
pub(crate) type SelectedRow<'a> = (&'a str, std::ops::Range<usize>, i32, Rect);

/// Where a selection over `rows` loses a selected glyph's edge at scale `s`: the device pixels
/// of its ink drawn `lit` beside (left, right, above or below) a pixel of `room` that is `lit`
/// both drawn `plain` and `selected` and that no glyph of `rows` inks. `plain` and `selected`
/// are the pixels of a `w` px wide canvas.
#[cfg(test)]
#[expect(clippy::too_many_arguments)]
pub(crate) fn lost_selected_ink(
    text: &TextRenderer,
    px: u32,
    (w, h): (i32, i32),
    rows: &[SelectedRow],
    room: Rect,
    s: i32,
    (plain, selected): (&[u32], &[u32]),
    lit: crate::theme::Color,
) -> Vec<(i32, i32)> {
    let n = (w * s * h * s) as usize;
    let (mut own, mut ink) = (vec![false; n], vec![false; n]);
    for (t, sel, pen, line) in rows {
        for (i, _) in t.char_indices() {
            let mask = glyph_mask(text, px, (w, h), t, i, *pen, *line, s);
            let mine = sel.contains(&i);
            for k in (0..n).filter(|&k| mask[k]) {
                ink[k] = true;
                own[k] |= mine;
            }
        }
    }
    let ws = (w * s) as usize;
    let at = |i: usize| ((i % ws) as i32, (i / ws) as i32);
    let room = room.scaled(s);
    let lit = lit.argb();
    let paper = |i: usize| {
        let (x, y) = at(i);
        !ink[i] && selected[i] == lit && plain[i] == lit && room.contains(x, y)
    };
    (0..n)
        .filter(|&i| own[i] && selected[i] == lit)
        .filter(|&i| {
            let x = i % ws;
            (x > 0 && paper(i - 1))
                || (x + 1 < ws && paper(i + 1))
                || (i >= ws && paper(i - ws))
                || (i + ws < n && paper(i + ws))
        })
        .map(at)
        .collect()
}

/// The device pixels any char of `t` inks alone at scale `s`, as [`glyph_mask`] draws each.
#[cfg(test)]
fn ink_mask(
    text: &TextRenderer,
    px: u32,
    (w, h): (i32, i32),
    t: &str,
    pen: i32,
    line: Rect,
    s: i32,
) -> Vec<bool> {
    let mut ink = vec![false; (w * s * h * s) as usize];
    for (i, _) in t.char_indices() {
        let mask = glyph_mask(text, px, (w, h), t, i, pen, line, s);
        ink.iter_mut().zip(mask).for_each(|(o, m)| *o |= m);
    }
    ink
}

/// What a caret drawn over `off` to give `on` does to the glyphs of `t` drawn from `pen` in
/// `line` at scale `s`: the pixels it changes that a glyph inks alone, and its pixels beside a
/// glyph's `lit` ink that are near `lit` too, where that ink's edge is lost. `off` and `on` are
/// the pixels of a `w` px wide canvas.
#[cfg(test)]
#[expect(clippy::too_many_arguments)]
pub(crate) fn caret_marks(
    text: &TextRenderer,
    px: u32,
    (w, h): (i32, i32),
    t: &str,
    (pen, line): (i32, Rect),
    s: i32,
    (off, on): (&[u32], &[u32]),
    lit: crate::theme::Color,
) -> (usize, usize) {
    let ink = ink_mask(text, px, (w, h), t, pen, line, s);
    let ws = (w * s) as usize;
    let near = |a: u32, b: u32| {
        let d = |k: u32| ((a >> k) & 0xFF).abs_diff((b >> k) & 0xFF);
        d(16) + d(8) + d(0) < 0x180
    };
    let lit_ink = |j: usize| ink[j] && on[j] == lit.argb();
    let changed: Vec<usize> = (0..on.len()).filter(|&i| on[i] != off[i]).collect();
    let flipped = changed.iter().filter(|&&i| ink[i]).count();
    let faint = changed
        .iter()
        .filter(|&&i| near(on[i], lit.argb()))
        .filter(|&&i| (i % ws > 0 && lit_ink(i - 1)) || (i % ws + 1 < ws && lit_ink(i + 1)))
        .count();
    (flipped, faint)
}

/// Texts for [`caret_marks`]: digits inked from their first column, descenders, a `|` and a `j`
/// whose tail runs under its neighbour.
#[cfg(test)]
pub(crate) const CARET_CASES: [&str; 4] = ["8605", "gjpqy", "o|g@", "1j9"];

/// Texts with descenders, a `|`, a `j` whose tail runs under its neighbour and digits inked from
/// their first column, each with every one-char selection and the whole text selected, for
/// [`off_highlight`].
#[cfg(test)]
pub(crate) fn descender_cases() -> Vec<(&'static str, std::ops::Range<usize>)> {
    ["gypqj", "ajo", "o|g@", "1j9", "8605"]
        .into_iter()
        .flat_map(|t| {
            (0..t.len())
                .map(move |i| (t, i..i + 1))
                .chain([(t, 0..t.len())])
        })
        .collect()
}

/// Every pair of printable ASCII chars, for [`selection_faults`].
#[cfg(test)]
pub(crate) fn ascii_pairs() -> impl Iterator<Item = String> {
    let chars = || ('!'..='~').filter(char::is_ascii_graphic);
    chars().flat_map(move |a| chars().map(move |b| format!("{a}{b}")))
}
