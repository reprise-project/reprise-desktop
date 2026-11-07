//! A font's sample: its alphabet, digits and punctuation, then a pangram at each of [`SIZES`]
//! beside its size, set in a face loaded from the font file ([`TextRenderer::from_file`]).

use crate::access::{self, Tree, accesskit};
use crate::look::{Paint, widgets};
use crate::raster::{Canvas, Rect};
use crate::text::{Align, TextRenderer};
use crate::theme::Color;
use accesskit::{NodeId, Role};

/// The pangram's sizes, px at 100%.
pub const SIZES: [u32; 6] = [12, 18, 24, 36, 48, 72];
pub const PANGRAM: &str = "The quick brown fox jumps over the lazy dog. 1234567890";
const CHARSET: [&str; 3] = [
    "abcdefghijklmnopqrstuvwxyz",
    "ABCDEFGHIJKLMNOPQRSTUVWXYZ",
    "1234567890.:,;(\"*!?')",
];
/// The charset rows' size, px.
const CHARSET_PX: u32 = 18;
/// The margin inside the sample and the gap round the rule under the charset.
const PAD: i32 = 8;
/// The column the size numbers sit in, left of each pangram.
const SIZE_COL: i32 = 28;
/// Space between pangram rows.
const ROW_GAP: i32 = 4;
/// The widest a sample asks to be, at 100%.
const MAX_W: i32 = 640;

/// A row's top and height.
type Row = (i32, i32);

/// The sample's rows from `top`, at `s` device px per unit.
struct Rows {
    charset: Vec<Row>,
    /// The rule's top, under the charset.
    rule: i32,
    /// One per [`SIZES`].
    sizes: Vec<Row>,
}

fn rows(sample: &TextRenderer, top: i32, s: i32) -> Rows {
    let height = |px: u32| {
        sample
            .line(Rect::new(0, 0, 0, 0), false, px)
            .map_or(px as i32 * 5 / 4, |(_, h)| h.max(1))
    };
    let mut y = top + PAD * s;
    let charset: Vec<Row> = CHARSET
        .iter()
        .map(|_| {
            let h = height(CHARSET_PX * s as u32);
            let row = (y, h);
            y += h;
            row
        })
        .collect();
    let rule = y + PAD * s;
    y = rule + s + PAD * s;
    let sizes = SIZES
        .iter()
        .map(|&px| {
            let h = height(px * s as u32);
            let row = (y, h);
            y += h + ROW_GAP * s;
            row
        })
        .collect();
    Rows {
        charset,
        rule,
        sizes,
    }
}

/// The size a sample of `sample` asks for at 100%, its pangrams cut at [`MAX_W`].
pub fn size(sample: &TextRenderer) -> (i32, i32) {
    let sizes = rows(sample, 0, 1).sizes;
    let bottom = sizes.last().map_or(0, |(y, h)| y + h) + PAD;
    let widest = SIZES
        .iter()
        .map(|&px| sample.measure(PANGRAM, false, px))
        .chain(
            CHARSET
                .iter()
                .map(|t| sample.measure(t, false, CHARSET_PX) - SIZE_COL),
        )
        .max()
        .unwrap_or(0);
    ((2 * PAD + SIZE_COL + widest).min(MAX_W), bottom)
}

/// Draw the sample's rows in `r`, clipped to it: `sample`'s ink in `ink`, the sizes in `ui` at
/// `ui_px`, at `s` device px per unit of `r`.
#[expect(clippy::too_many_arguments)]
pub fn draw_rows(
    c: &mut Canvas,
    ui: &TextRenderer,
    ui_px: u32,
    sample: &TextRenderer,
    ink: Color,
    rule: Color,
    r: Rect,
    s: i32,
) {
    let Rows {
        charset,
        rule: rule_y,
        sizes,
    } = rows(sample, r.y, s);
    let x = r.x + PAD * s;
    let wide = |y: i32, h: i32, x: i32| Rect::new(x, y, (r.right() - x).max(0), h);
    for (t, (y, h)) in CHARSET.iter().zip(charset) {
        sample.draw_clipped(
            c,
            wide(y, h, x),
            r,
            t,
            false,
            ink,
            Align::Left,
            CHARSET_PX * s as u32,
        );
    }
    c.fill_rect(
        Rect::new(x, rule_y, (r.width - 2 * PAD * s).max(0), s).intersect(&r),
        rule,
    );
    for (&px, (y, h)) in SIZES.iter().zip(sizes) {
        let row = wide(y, h, x + SIZE_COL * s);
        let px = px * s as u32;
        if let Some(base) = sample.baseline(row, false, px) {
            ui.draw_run(
                c,
                x,
                base,
                r,
                &(px / s as u32).to_string(),
                false,
                ink,
                ui_px,
            );
        }
        sample.draw_clipped(c, row, r, PANGRAM, false, ink, Align::Left, px);
    }
}

/// The sample in a field frame filling `r` (device px of `c` at `p.s` per unit).
pub fn draw(
    c: &mut Canvas,
    p: &Paint,
    ui: &TextRenderer,
    ui_px: u32,
    sample: &TextRenderer,
    r: Rect,
) {
    let s = p.s.max(1);
    widgets::draw_field_frame(c, p, r, p.cs.window);
    let inner = r.inset(2 * s);
    c.fill_rect(inner, p.cs.window);
    c.with_clip(inner, |c| {
        draw_rows(
            c,
            ui,
            ui_px,
            sample,
            p.cs.window_text,
            p.cs.button_shadow,
            inner,
            s,
        )
    });
}

/// The sample alone as a picture at most `max` device px, at `scale`: the file preview's.
pub fn picture(
    ui: &TextRenderer,
    ui_px: u32,
    sample: &TextRenderer,
    (ink, paper, rule): (Color, Color, Color),
    max: (u32, u32),
    scale: i32,
) -> Canvas {
    let s = scale.max(1);
    let (w, h) = size(sample);
    let w = w.min(max.0 as i32 / s).max(1);
    let h = h.min(max.1 as i32 / s).max(1);
    let mut c = Canvas::with_scale(w as u32, h as u32, s);
    let r = c.bounds();
    c.fill_rect(r, paper);
    draw_rows(&mut c, ui, ui_px, sample, ink, rule, r, 1);
    c
}

/// The sample's node under `parent`: a picture named for the face `title`.
pub fn access(tree: &mut Tree, parent: NodeId, title: &str, r: Rect) -> NodeId {
    tree.push(parent, "sample", access::titled(Role::Image, r, title))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Theme;

    fn tahoma() -> TextRenderer {
        TextRenderer::from_file(&crate::pins::fonts().join("tahoma.ttf"), 0).expect("Tahoma")
    }

    #[test]
    fn a_sample_grows_with_its_sizes_and_stops_at_the_widest() {
        let t = tahoma();
        let (w, h) = size(&t);
        assert_eq!(w, MAX_W, "the 72 px pangram is cut");
        let sum: u32 = SIZES.iter().sum();
        assert!(h > sum as i32, "every row fits: {h}");
    }

    #[test]
    fn a_file_face_draws_no_fallback_glyphs() {
        let t = tahoma();
        let ui = TextRenderer::load(&Theme::default().fonts);
        assert_eq!(
            t.advance('\u{4e2d}', false, 24),
            t.advance('\u{4e2e}', false, 24),
            "chars the face lacks draw its notdef"
        );
        assert_ne!(ui.measure(PANGRAM, false, 11), 0);
    }

    #[test]
    fn the_picture_fits_inside_its_bound_at_scale_2() {
        let t = tahoma();
        let ui = TextRenderer::load(&Theme::default().fonts);
        let cs = Theme::default().colors;
        let pic = picture(
            &ui,
            11,
            &t,
            (cs.window_text, cs.window, cs.button_shadow),
            (400, 300),
            2,
        );
        assert_eq!((pic.width, pic.height), (200, 150));
        assert_eq!(pic.device_size(), (400, 300));
        assert!(
            pic.pixels.iter().any(|&p| p != cs.window.argb()),
            "the sample inks"
        );
    }
}
