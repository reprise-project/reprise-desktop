//! The Start panel's search mode, piece by piece, through a style's `Part::Start*` parts: the search
//! field, a result row and a group's header. The Glass Start panel and the launcher (under a look with
//! glass) draw with them. Every rect is in device pixels; the offsets in [`at`] are design pixels at
//! 100% text, grown and scaled through a [`Grow`].

use std::cell::RefCell;
use std::rc::Rc;

use super::SEARCH_CUE;
use crate::raster::{Canvas, Rect};
use crate::style::{Part, State, Style};
use crate::text::{CaretBar, TextRenderer};
use crate::textedit::{TextEdit, edit_first_visible};
use crate::theme::{Color, FontConfig, TextUnits};

pub const TEXT_PX: u32 = 12;
/// A search group's header, a size up: its capitals stand 11 rows tall.
pub const HEADER_PX: u32 = 15;

pub const FIELD: (i32, i32) = (222, 22);

/// 0xRRGGBB, over the light pane ([`PaneInks::LIGHT`]).
pub const INK_STRONG: u32 = 0x000000;
pub const INK_CUE: u32 = 0x6D6D6D;
pub const INK_GREYED: u32 = 0x6D6D6D;
pub const INK_GROUP: u32 = 0x1E3287;
pub const INK_PROGRAM: u32 = 0x272727;
pub const INK_JUMP: u32 = 0x434343;
/// A jump list's title.
pub const INK_HEADER: u32 = 0x002A95;

/// The text inks over the Start panel's left pane and the launcher's list, which a style's
/// `Part::StartPane` paints.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaneInks {
    /// All Programs, Back, the programs tree, typed text, results, and Shut down on the light button.
    pub strong: Color,
    pub program: Color,
    pub jump: Color,
    pub jump_title: Color,
    pub group: Color,
    pub cue: Color,
    pub greyed: Color,
}

impl PaneInks {
    pub const LIGHT: PaneInks = PaneInks {
        strong: ink(INK_STRONG),
        program: ink(INK_PROGRAM),
        jump: ink(INK_JUMP),
        jump_title: ink(INK_HEADER),
        group: ink(INK_GROUP),
        cue: ink(INK_CUE),
        greyed: ink(INK_GREYED),
    };

    pub fn map(self, f: impl Fn(Color) -> Color) -> PaneInks {
        PaneInks {
            strong: f(self.strong),
            program: f(self.program),
            jump: f(self.jump),
            jump_title: f(self.jump_title),
            group: f(self.group),
            cue: f(self.cue),
            greyed: f(self.greyed),
        }
    }
}

/// Design px between a result's name and its dimmed detail.
pub const DETAIL_GAP: i32 = 8;

/// How design pixels land on the canvas: grown by the text size `u`, then times the scale `s`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Grow {
    pub s: i32,
    pub u: TextUnits,
}

impl Grow {
    /// Scale `s` at 100% text.
    pub fn at(s: i32) -> Grow {
        Grow {
            s,
            u: TextUnits::default(),
        }
    }

    pub fn len(self, v: i32) -> i32 {
        self.u.len(v) * self.s
    }

    /// A font size of `px` at 100%.
    pub fn px(self, px: u32) -> u32 {
        self.u.px(px) * self.s as u32
    }

    /// Where a thing `size` design px long that keeps its size starts, for one placed `off` in at
    /// 100%: its centre moves with the text.
    pub fn kept(self, off: i32, size: i32) -> i32 {
        (self.u.len(off + size / 2) - size / 2) * self.s
    }

    /// Where an icon `actual` device px long starts, for one `size` design px long placed `off` in
    /// at 100% that grows with the text: centred where that one's centre moves to.
    pub fn centred(self, off: i32, size: i32, actual: i32) -> i32 {
        self.u.len(off + size / 2) * self.s - actual / 2
    }
}

pub const fn ink(rgb: u32) -> Color {
    Color::rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

/// Where things sit inside the field, a header and a result, in design pixels from the rect's
/// top-left: (x, y) of an icon or glyph, the pen x of a line and its baseline (one row under its capitals).
pub mod at {
    /// The search field's box: the cue, typed text and caret, magnifier and clear glyph. The magnifier,
    /// clear glyph and the text's right edge are given in the 222 px box and keep their distance from its right edge in a wider one.
    pub const CUE_PEN: i32 = 9;
    pub const CUE_BASE: i32 = 15;
    pub const TYPED_PEN: i32 = 6;
    pub const CARET: (i32, i32, i32) = (6, 3, 15);
    pub const MAGNIFIER: (i32, i32) = (203, 5);
    pub const CLEAR: (i32, i32) = (206, 8);
    pub const TEXT_RIGHT: i32 = 200;
    /// A search group's header (from its capitals' top) and rule; a result's 22 px box, icon and name.
    pub const HEAD_BASE: i32 = 11;
    pub const HEAD_RULE_Y: i32 = 6;
    pub const HEAD_RULE_GAP: i32 = 8;
    pub const RESULT_BOX_H: i32 = 22;
    pub const RESULT_ICON: (i32, i32) = (4, 2);
    pub const RESULT_PEN: i32 = 24;
    pub const RESULT_BASE: i32 = 15;
}

/// `font`'s italic, for the search field's cue; loaded once per thread and kept while `font` stays the same.
pub fn italic(font: &FontConfig) -> Rc<TextRenderer> {
    thread_local! {
        static LOADED: RefCell<Option<(FontConfig, Rc<TextRenderer>)>> =
            const { RefCell::new(None) };
    }
    let font = FontConfig {
        italic: true,
        ..font.clone()
    };
    LOADED.with_borrow_mut(|slot| match slot {
        Some((f, t)) if *f == font => t.clone(),
        _ => {
            let t = Rc::new(TextRenderer::load(&font));
            *slot = Some((font, t.clone()));
            t
        }
    })
}

/// `line` in `face` from pen `x` on baseline `y`, ellipsized to what is left of `clip` and laid over what is there; its width.
#[expect(clippy::too_many_arguments)]
pub fn line(
    canvas: &mut Canvas,
    face: &TextRenderer,
    x: i32,
    y: i32,
    clip: Rect,
    line: &str,
    color: Color,
    size: u32,
) -> i32 {
    let room = clip.right() - x;
    let shown = face.ellipsize(line, room, false, size);
    face.draw_run_over(canvas, x, y, clip, &shown, false, color, size);
    face.measure(&shown, false, size)
}

/// A glyph part's size in design pixels; (0, 0) when the style lacks it.
pub fn glyph_size(style: &Style, part: Part) -> (i32, i32) {
    style.part(part).map_or((0, 0), |img| {
        let f = img.frame(State::Normal);
        (f.width / img.scale, f.height / img.scale)
    })
}

pub fn glyph(canvas: &mut Canvas, style: &Style, (x, y): (i32, i32), part: Part, s: i32) {
    let Some(img) = style.part(part) else {
        return;
    };
    let f = img.frame(State::Normal);
    let (w, h) = (f.width / img.scale, f.height / img.scale);
    style.draw(
        canvas,
        Rect::new(x, y, w * s, h * s),
        part,
        State::Normal,
        s,
    );
}

/// The search field in its box `r` holding `edit`, its lines in `text` and the cue in `cue`
/// ([`italic`]): the box and halo, then the cue in italic grey while empty, else the text, scrolled by
/// whole characters to keep the caret in the field, its selection on the style's selection colour, a
/// pixel clear of the selected ink, while `focused`; the caret while `focused` and `caret` (the blink)
/// is on, a [`CaretBar`] on the opaque face or beside the selection; the magnifier, or the clear glyph once there is text, at the box's right.
///
/// While `focused` it also shows the canvas' preedit at the caret, underlined under the baseline with
/// the caret inside it, and reports its [`crate::textedit::TextSpot`] on the canvas, as `draw_edit` does.
#[expect(clippy::too_many_arguments)]
pub fn field(
    canvas: &mut Canvas,
    style: &Style,
    text: &TextRenderer,
    cue: &TextRenderer,
    r: Rect,
    edit: &TextEdit,
    focused: bool,
    caret: bool,
    g: Grow,
) {
    let s = g.s;
    let size = g.px(TEXT_PX);
    let from_right = |x: i32| r.width - g.len(FIELD.0 - x);
    style.draw(
        canvas,
        r.inset(-s),
        Part::StartSearchField,
        State::Normal,
        s,
    );
    let clip = Rect::new(
        r.x + s,
        r.y + s,
        from_right(at::TEXT_RIGHT),
        r.height - 2 * s,
    );
    let own = edit;
    let preedit = canvas.preedit().filter(|_| focused).cloned();
    let composed;
    let (edit, composing, bar_on) = match &preedit {
        Some(p) => {
            let (e, run, bar_on) = p.compose(own);
            composed = e;
            (&composed, Some(run), bar_on)
        }
        None => (own, None, true),
    };
    let caret = caret && bar_on;
    let query = edit.text.as_str();
    let (cx, cy, ch) = at::CARET;
    let row = Rect::new(r.x, r.y + g.len(cy), r.width, g.len(ch));
    let mut bar = Rect::new(r.x + g.len(cx), row.y, s, row.height);
    let lit = style.colors.hilight_text;
    if query.is_empty() {
        let placed = (caret && focused).then(|| CaretBar::ground(canvas, bar, None, clip));
        let (x, y) = (r.x + g.len(at::CUE_PEN), r.y + g.len(at::CUE_BASE));
        line(
            canvas,
            cue,
            x,
            y,
            clip,
            SEARCH_CUE,
            style.pane_inks.cue,
            size,
        );
        if let Some(placed) = placed {
            placed.draw(canvas, lit);
        }
    } else {
        let (pen, y) = (r.x + g.len(at::TYPED_PEN), r.y + g.len(at::CUE_BASE));
        let room = clip.right() - pen - s;
        let first = edit_first_visible(text, edit, room, size, 0);
        let x_of = |i: usize| pen + text.measure(&query[first..i.max(first)], false, size);
        let shown = &query[first..];
        let sel = edit.selection();
        let sel = sel.start.max(first)..sel.end.max(first);
        let hl = (focused && !sel.is_empty()).then(|| {
            let (x0, x1) = (x_of(sel.start), x_of(sel.end).min(clip.right()));
            let run = Rect::new(x0, row.y, x1 - x0, row.height).intersect(&clip);
            let sel = sel.start - first..sel.end - first;
            let scale = canvas.scale();
            text.highlight_on(run, shown, sel, pen, Some(y), clip, false, size, scale)
                .clear_of_ink(clip)
        });
        let cs = &style.colors;
        // Filled under the text too, so the caret's bar reads it as ground, not ink.
        if let Some(hl) = &hl {
            hl.fill(canvas, clip, cs.hilight);
        }
        let ci = edit.caret().min(query.len()).max(first);
        let shift = g.len(cx) - g.len(at::TYPED_PEN);
        let x = x_of(ci) + shift;
        bar.x = x.min(clip.right() - s);
        let scale = canvas.scale();
        let dx = text.caret_x(shown, ci - first, pen, false, size, scale) + scale * shift;
        let placed = (caret && focused && x < clip.right())
            .then(|| CaretBar::ground_at(canvas, Rect { x, ..bar }, dx, hl.as_ref(), clip));
        let strong = style.pane_inks.strong;
        text.draw_run_over(canvas, pen, y, clip, shown, false, strong, size);
        if let Some(hl) = &hl {
            canvas.fill_rect(hl.rect, cs.hilight);
            canvas.with_clip(hl.rect, |canvas| {
                text.draw_run_over(canvas, pen, y, clip, shown, false, cs.hilight_text, size);
            });
            text.draw_overhang(canvas, hl, clip, cs.hilight_text);
        }
        if let Some(composing) = composing {
            let (x0, x1) = (x_of(composing.start), x_of(composing.end).min(clip.right()));
            let ul = Rect::new(x0, y + s, x1 - x0, s).intersect(&clip);
            canvas.fill_rect(ul, strong);
        }
        if let Some(placed) = placed {
            placed.draw(canvas, lit);
        }
    }
    if focused {
        canvas.report_spot(own.spot(clip, bar));
    }
    let (part, (gx, gy)) = if query.is_empty() {
        (Part::StartMagnifier, at::MAGNIFIER)
    } else {
        (Part::StartClear, at::CLEAR)
    };
    let (w, h) = glyph_size(style, part);
    let x = r.right() - g.kept(FIELD.0 - gx - w, w) - w * s;
    glyph(canvas, style, (x, r.y + g.kept(gy, h)), part, s);
}

/// A result row `r`: [`Part::StartLitResult`]'s 22 px box across it when `lit`, the `icon` and
/// `name` in `text`, then `detail` in the pane's greyed ink after it, both cut short of `r`'s right by
/// 2 px.
#[expect(clippy::too_many_arguments)]
pub fn result(
    canvas: &mut Canvas,
    style: &Style,
    text: &TextRenderer,
    r: Rect,
    icon: Option<&Canvas>,
    name: &str,
    detail: &str,
    lit: bool,
    g: Grow,
) {
    let s = g.s;
    if lit {
        let b = Rect::new(r.x, r.y, r.width, g.len(at::RESULT_BOX_H));
        style.draw(canvas, b, Part::StartLitResult, State::Normal, s);
    }
    if let Some(icon) = icon {
        let (x, y) = at::RESULT_ICON;
        let (w, h) = (icon.width as i32, icon.height as i32);
        let size = crate::startpanel::SMALL_ICON;
        canvas.blit_over(
            r.x + g.centred(x, size, w),
            r.y + g.centred(y, size, h),
            icon,
        );
    }
    let clip = Rect::new(r.x, r.y, r.width - 2 * s, r.height);
    let (x, y) = (r.x + g.len(at::RESULT_PEN), r.y + g.len(at::RESULT_BASE));
    let size = g.px(TEXT_PX);
    let end = x + line(canvas, text, x, y, clip, name, style.pane_inks.strong, size);
    if !detail.is_empty() {
        let x = end + g.len(DETAIL_GAP);
        line(
            canvas,
            text,
            x,
            y,
            clip,
            detail,
            style.pane_inks.greyed,
            size,
        );
    }
}

/// A group's header `r` (its top on the capitals' top): `title` in the pane's group ink a size up, cut to
/// `clip`, then [`Part::StartSearchRule`] from past it to a pixel beyond `r`'s right.
pub fn header(
    canvas: &mut Canvas,
    style: &Style,
    text: &TextRenderer,
    r: Rect,
    clip: Rect,
    title: &str,
    g: Grow,
) {
    let s = g.s;
    let (x, y) = (r.x, r.y + g.len(at::HEAD_BASE));
    let px = g.px(HEADER_PX);
    let w = line(canvas, text, x, y, clip, title, style.pane_inks.group, px);
    let x0 = x + w + g.len(at::HEAD_RULE_GAP);
    let x1 = r.right() + s;
    let ry = r.y + g.len(at::HEAD_RULE_Y);
    style.draw(
        canvas,
        Rect::new(x0, ry, x1 - x0, s),
        Part::StartSearchRule,
        State::Normal,
        s,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::textedit::Preedit;

    /// The Glass field composing "ni" after "ab": the preedit after the text, underlined a row under
    /// the baseline, the caret after it, and the spot reporting the field's own text.
    #[test]
    fn a_focused_field_shows_the_preedit_underlined_and_reports_its_spot() {
        let st = crate::glass::style(&crate::glass::Settings::default());
        let text = TextRenderer::load(&FontConfig::default());
        let r = Rect::new(4, 4, FIELD.0, FIELD.1);
        let edit = TextEdit::new("ab", false);
        let draw = |preedit: Option<Preedit>, focused: bool| {
            let mut c = Canvas::new(240, 32);
            c.set_preedit(preedit);
            field(
                &mut c,
                &st,
                &text,
                &text,
                r,
                &edit,
                focused,
                true,
                Grow::at(1),
            );
            c
        };
        let mut plain = draw(None, true);
        let mut composing = draw(Preedit::new("ni".into(), 2, 2), true);
        let spot = composing.take_spot().expect("a focused field reports");
        let before = plain.take_spot().expect("a focused field reports");
        assert_eq!(
            spot.surrounding, before.surrounding,
            "the preedit is no text yet"
        );
        let (ab, ni) = (
            text.measure("ab", false, TEXT_PX),
            text.measure("ni", false, TEXT_PX),
        );
        assert_eq!(
            spot.caret.x,
            before.caret.x + ni,
            "the caret after the preedit"
        );
        let pen = r.x + at::TYPED_PEN + ab;
        let y = r.y + at::CUE_BASE + 1;
        for x in pen..pen + ni {
            assert_eq!(composing.get(x, y), 0xFF00_0000, "underline at ({x}, {y})");
        }
        assert_ne!(
            plain.get(pen + 2, y),
            0xFF00_0000,
            "no underline without a preedit"
        );
        let mut away = draw(Preedit::new("ni".into(), 2, 2), false);
        assert_eq!(away.take_spot(), None, "an unfocused field reports nothing");
        assert_eq!(away.pixels, draw(None, false).pixels, "nor shows a preedit");
    }

    #[test]
    fn the_caret_at_0_stands_off_the_first_stem_on_the_opaque_face_in_light_and_dark() {
        use crate::glass::{Mode, Settings};
        crate::pins::fonts();
        let text = TextRenderer::load(&FontConfig::default());
        let r = Rect::new(4, 4, FIELD.0, FIELD.1);
        let mut edit = TextEdit::new("AVA", false);
        edit.set_selection(0, 0);
        let (pen, top) = (r.x + at::TYPED_PEN, r.y + at::CARET.1);
        for mode in Mode::ALL {
            let st = crate::glass::style(&Settings {
                mode,
                ..Settings::default()
            });
            let draw = |caret| {
                let mut c = Canvas::new(240, 32);
                field(
                    &mut c,
                    &st,
                    &text,
                    &text,
                    r,
                    &edit,
                    true,
                    caret,
                    Grow::at(1),
                );
                c
            };
            let (off, on) = (draw(false), draw(true));
            assert_eq!(
                crate::paint::inverted_bar(&off, &on, st.colors.window.argb()),
                (Rect::new(pen - 1, top, 1, at::CARET.2), false),
                "{mode:?}: the caret is the column left of the pen, off the A's foot"
            );
            for y in top..top + at::CARET.2 {
                assert_eq!(
                    off.get(pen - 1, y) >> 24,
                    0xFF,
                    "{mode:?}: opaque face at row {y}"
                );
            }
        }
    }

    #[test]
    fn a_preedit_range_draws_as_a_caret_at_its_end() {
        let st = crate::glass::style(&crate::glass::Settings::default());
        let text = TextRenderer::load(&FontConfig::default());
        let r = Rect::new(4, 4, FIELD.0, FIELD.1);
        let edit = TextEdit::new("ab", false);
        let draw = |p| {
            let mut c = Canvas::new(240, 32);
            c.set_preedit(p);
            field(&mut c, &st, &text, &text, r, &edit, true, true, Grow::at(1));
            c
        };
        let mut range = draw(Preedit::new("ni".into(), 0, 2));
        let mut at_end = draw(Preedit::new("ni".into(), 2, 2));
        let hl = st.colors.hilight.argb();
        assert!(range.pixels.iter().all(|&p| p != hl), "nothing on hilight");
        assert_eq!(range.pixels, at_end.pixels);
        assert_eq!(range.take_spot(), at_end.take_spot());
    }

    /// The field at grow `s` holding `t`, `sel` selected with the caret at its end, focused.
    fn drawn(
        st: &Style,
        text: &TextRenderer,
        t: &str,
        sel: (usize, usize),
        s: i32,
        caret: bool,
    ) -> Canvas {
        let r = Rect::new(4, 4, FIELD.0 * s, FIELD.1 * s);
        let mut edit = TextEdit::new(t, false);
        edit.set_selection(sel.0, sel.1);
        let mut c = Canvas::new(240 * s as u32, 32 * s as u32);
        field(&mut c, st, text, text, r, &edit, true, caret, Grow::at(s));
        c
    }

    /// The ink of a selected glyph that inks within its advance, at grow 1 and 2, meets only its
    /// own ink or the highlight: the highlight reaches a pixel past it on every side.
    #[test]
    fn a_selected_glyph_keeps_a_pixel_of_highlight_around_its_ink() {
        crate::pins::fonts();
        let st = crate::glass::style(&crate::glass::Settings::default());
        let text = TextRenderer::load(&FontConfig::default());
        let hl = st.colors.hilight.argb();
        for s in [1, 2] {
            for t in ["8", "g", "q", "y"] {
                let c = drawn(&st, &text, t, (0, t.len()), s, false);
                let ink = Canvas::new(c.width, c.height).ink_coverage(|m, white| {
                    let (pen, y) = (4 + s * at::TYPED_PEN, 4 + s * at::CUE_BASE);
                    let clip = Rect::new(0, 0, c.width as i32, c.height as i32);
                    text.draw_run_over(m, pen, y, clip, t, false, white, TEXT_PX * s as u32);
                });
                let inked =
                    |x: i32, y: i32| x >= 0 && y >= 0 && ink[(y * c.width as i32 + x) as usize] > 0;
                let mut bare = Vec::new();
                for y in 0..c.height as i32 {
                    for x in 0..c.width as i32 {
                        if !inked(x, y) {
                            continue;
                        }
                        for (dx, dy) in [
                            (-1, -1),
                            (0, -1),
                            (1, -1),
                            (-1, 0),
                            (1, 0),
                            (-1, 1),
                            (0, 1),
                            (1, 1),
                        ] {
                            let (nx, ny) = (x + dx, y + dy);
                            if !inked(nx, ny) && c.get(nx, ny) != hl {
                                bare.push((nx, ny));
                            }
                        }
                    }
                }
                assert!(
                    bare.is_empty(),
                    "\"{t}\" at grow {s}: off the highlight beside its ink at {bare:?}"
                );
            }
        }
    }

    /// With a digit of "8 6" selected, the caret at either end of it, where a space or the
    /// field's start leaves room, stands beside the highlight at grow 1 and 2.
    #[test]
    fn the_caret_at_a_selection_end_stands_beside_the_highlight() {
        crate::pins::fonts();
        let st = crate::glass::style(&crate::glass::Settings::default());
        let text = TextRenderer::load(&FontConfig::default());
        let hl = st.colors.hilight.argb();
        for s in [1, 2] {
            for sel in [(0, 1), (1, 0), (2, 3), (3, 2)] {
                let off = drawn(&st, &text, "8 6", sel, s, false);
                let on = drawn(&st, &text, "8 6", sel, s, true);
                let changed: Vec<usize> = (0..on.pixels.len())
                    .filter(|&i| on.pixels[i] != off.pixels[i])
                    .collect();
                assert!(!changed.is_empty(), "{sel:?} at grow {s}: the caret shows");
                let lit: Vec<usize> = changed
                    .into_iter()
                    .filter(|&i| off.pixels[i] == hl)
                    .collect();
                assert!(
                    lit.is_empty(),
                    "{sel:?} at grow {s}: the caret is on {} highlight px",
                    lit.len()
                );
            }
        }
    }
}
