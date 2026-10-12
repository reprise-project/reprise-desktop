//! The time field: "11:41:24 PM" (or 24-hour, or without seconds) in a sunken
//! field with a spinner's two buttons: in Classic one pixel to its right and two
//! shorter than the field, in a look with `buddy_spin_inside` inside its frame.
//! One part (hour, minute, second, AM/PM) is current: Up / Down and the buttons
//! step it, wrapping; Left / Right pick the part; digits type it; A and P pick
//! the half of the day.

use crate::access::{self, Tree, accesskit};
use crate::classic::widgets::Arrow;
use crate::controls::metrics::ControlMetrics;
use crate::controls::updown;
use crate::look::Paint;
use crate::look::widgets::{draw_disabled_label, draw_spin_buttons, draw_spin_field};
use crate::pointer::Lit;
use crate::raster::{Canvas, Rect};
use crate::text::{Align, TextRenderer};
use accesskit::{NodeId, Role};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimeField {
    pub h: u32,
    pub m: u32,
    pub s: u32,
    pub twelve: bool,
    pub seconds: bool,
    /// The current part, counted along the shown ones.
    pub part: usize,
    pub enabled: bool,
    typed: String,
}

impl TimeField {
    pub fn new(h: u32, m: u32, s: u32, twelve: bool) -> TimeField {
        TimeField {
            h: h % 24,
            m: m % 60,
            s: s % 60,
            twelve,
            seconds: true,
            part: 0,
            enabled: true,
            typed: String::new(),
        }
    }

    pub fn set(&mut self, h: u32, m: u32, s: u32) {
        (self.h, self.m, self.s) = (h % 24, m % 60, s % 60);
    }

    pub fn hm(h: u32, m: u32, twelve: bool) -> TimeField {
        TimeField {
            seconds: false,
            ..TimeField::new(h, m, 0, twelve)
        }
    }

    fn parts(&self) -> usize {
        2 + self.seconds as usize + self.twelve as usize
    }

    fn kind(&self, k: usize) -> usize {
        if k >= 2 && !self.seconds { k + 1 } else { k }
    }

    pub fn select(&mut self, k: usize) -> bool {
        let k = k.min(self.parts() - 1);
        self.typed.clear();
        let moved = k != self.part;
        self.part = k;
        moved
    }

    pub fn move_part(&mut self, delta: i32) -> bool {
        let k = (self.part as i32 + delta).clamp(0, self.parts() as i32 - 1);
        self.select(k as usize)
    }

    /// Step the current part by `delta`, wrapping. In 12-hour time the
    /// hour wraps inside its half of the day.
    pub fn step(&mut self, delta: i32) -> bool {
        self.typed.clear();
        let before = (self.h, self.m, self.s);
        let wrap = |v: u32, n: u32| (v as i32 + delta).rem_euclid(n as i32) as u32;
        match self.kind(self.part) {
            0 if self.twelve => self.h = self.h / 12 * 12 + wrap(self.h % 12, 12),
            0 => self.h = wrap(self.h, 24),
            1 => self.m = wrap(self.m, 60),
            2 => self.s = wrap(self.s, 60),
            _ => {
                if delta % 2 != 0 {
                    self.h = (self.h + 12) % 24;
                }
            }
        }
        before != (self.h, self.m, self.s)
    }

    /// A typed character: a digit into a number part (two at most, a
    /// second one that makes no valid value starting over), A or P into
    /// the AM/PM part.
    pub fn char(&mut self, ch: char) -> bool {
        let before = (self.h, self.m, self.s);
        let kind = self.kind(self.part);
        if kind == 3 {
            match ch.to_ascii_lowercase() {
                'a' if self.h >= 12 => self.h -= 12,
                'p' if self.h < 12 => self.h += 12,
                _ => {}
            }
            return before != (self.h, self.m, self.s);
        }
        let Some(d) = ch.to_digit(10) else {
            return false;
        };
        let (lo, hi) = match kind {
            0 if self.twelve => (1, 12),
            0 => (0, 23),
            _ => (0, 59),
        };
        let two = self
            .typed
            .parse::<u32>()
            .ok()
            .filter(|_| self.typed.len() == 1)
            .map(|t| t * 10 + d)
            .filter(|v| (lo..=hi).contains(v));
        let v = match two {
            Some(v) => {
                self.typed.clear();
                v
            }
            None => {
                self.typed = d.to_string();
                d.clamp(lo, hi)
            }
        };
        match kind {
            0 if self.twelve => self.h = self.h / 12 * 12 + v % 12,
            0 => self.h = v,
            1 => self.m = v,
            _ => self.s = v,
        }
        before != (self.h, self.m, self.s)
    }

    /// The shown text and each part's byte range in it.
    pub fn text(&self) -> (String, Vec<std::ops::Range<usize>>) {
        let (hour, am_pm) = if self.twelve {
            let h12 = if self.h.is_multiple_of(12) {
                12
            } else {
                self.h % 12
            };
            (h12.to_string(), Some(if self.h < 12 { "AM" } else { "PM" }))
        } else {
            (format!("{:02}", self.h), None)
        };
        let mut s = hour;
        let mut ranges = Vec::with_capacity(4);
        ranges.push(0..s.len());
        let shown = if self.seconds { 2 } else { 1 };
        for v in [self.m, self.s].into_iter().take(shown) {
            s.push(':');
            let start = s.len();
            s.push_str(&format!("{v:02}"));
            ranges.push(start..s.len());
        }
        if let Some(t) = am_pm {
            s.push(' ');
            let start = s.len();
            s.push_str(t);
            ranges.push(start..s.len());
        }
        (s, ranges)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub field: Rect,
    pub up: Rect,
    pub down: Rect,
}

/// The field and buttons filling `r`, the buttons' column `cm.buddy_spin` wide.
/// Beside the field it stands one pixel right, 2 px shorter; inside, the field is
/// all of `r` and the column stands inside its 2 px frame at the right.
pub fn layout(r: Rect, cm: &ControlMetrics) -> Layout {
    if cm.buddy_spin_inside {
        let up_down = updown::buddy_layout(r, cm);
        return Layout {
            field: r,
            up: up_down.up,
            down: up_down.down,
        };
    }
    let field = Rect::new(r.x, r.y, r.width - cm.buddy_spin - 1, r.height);
    let col = Rect::new(field.right() + 1, r.y, cm.buddy_spin, r.height - 2);
    let (up, down) = updown::split_buttons(col);
    Layout { field, up, down }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Part(usize),
    Up,
    Down,
}

fn text_x(lay: &Layout) -> i32 {
    lay.field.x + 3
}

struct Run<'a> {
    x: i32,
    text: &'a str,
    part: Option<usize>,
}

/// The shown text cut into its parts and separators, placed: a colon
/// keeps a pixel clear on each side, and the AM/PM part sits a pixel
/// closer than a space would put it. `s` is the scale `px` is at.
fn runs<'a>(
    shown: &'a str,
    ranges: &[std::ops::Range<usize>],
    text: &TextRenderer,
    px: u32,
    s: i32,
) -> Vec<Run<'a>> {
    let mut out = Vec::new();
    let mut x = 0;
    let mut end = 0;
    for (k, r) in ranges.iter().enumerate() {
        let sep = &shown[end..r.start];
        if sep == ":" {
            x += s;
            out.push(Run {
                x,
                text: sep,
                part: None,
            });
            x += text.measure(sep, false, px) + s;
        } else if !sep.is_empty() {
            x += text.measure(sep, false, px) - s;
        }
        out.push(Run {
            x,
            text: &shown[r.clone()],
            part: Some(k),
        });
        x += text.measure(&shown[r.clone()], false, px);
        end = r.end;
    }
    out
}

pub fn hit(
    tf: &TimeField,
    lay: &Layout,
    x: i32,
    y: i32,
    text: &TextRenderer,
    px: u32,
) -> Option<Hit> {
    if lay.up.contains(x, y) {
        return Some(Hit::Up);
    }
    if lay.down.contains(x, y) {
        return Some(Hit::Down);
    }
    if !lay.field.contains(x, y) {
        return None;
    }
    let (shown, ranges) = tf.text();
    let dx = x - text_x(lay);
    // The part whose right edge is past the pointer; past the last one,
    // the last one.
    let k = runs(&shown, &ranges, text, px, 1)
        .iter()
        .filter_map(|r| {
            let end = r.x + text.measure(r.text, false, px);
            r.part.filter(|_| end > dx)
        })
        .next()
        .unwrap_or(ranges.len() - 1);
    Some(Hit::Part(k))
}

#[expect(clippy::too_many_arguments)]
pub fn draw(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    tf: &TimeField,
    lay: &Layout,
    focused: bool,
    spin: Lit<Arrow>,
    px: u32,
) {
    let Paint { cs, s, .. } = *p;
    let field = lay.field.scaled(s);
    draw_spin_field(
        c,
        p,
        field,
        lay.up.scaled(s),
        lay.down.scaled(s),
        tf.enabled,
    );
    let inner = field.inset(2 * s);
    // Up to the buttons when they stand inside the frame.
    let right = inner.right().min(lay.up.x * s);
    let tr = Rect::new(
        text_x(lay) * s,
        inner.y,
        right - text_x(lay) * s,
        inner.height,
    );
    let (shown, ranges) = tf.text();
    let lh = p.text.len(crate::classic::widgets::LINE_H) * s;
    for run in runs(&shown, &ranges, text, px, s) {
        let w = text.measure(run.text, false, px);
        let r = Rect::new(tr.x + run.x, tr.y, w, tr.height);
        if !tf.enabled {
            draw_disabled_label(
                c,
                text,
                p,
                r,
                run.text,
                Align::Left,
                px,
                Some(crate::style::Part::EditBorder),
            );
        } else if focused && run.part == Some(tf.part) {
            let hl = Rect::new(r.x, tr.y + (tr.height - lh) / 2, w, lh);
            c.fill_rect(hl, cs.hilight);
            text.draw(c, hl, run.text, false, cs.hilight_text, Align::Left, px);
        } else {
            text.draw(c, r, run.text, false, cs.window_text, Align::Left, px);
        }
    }
    draw_spin_buttons(c, p, lay.up, lay.down, tf.enabled, spin);
}

/// Where the access tree places the time's text: to the field's right edge, or
/// to the buttons when they stand inside it, as `draw` clips it.
fn access_text_rect(lay: &Layout) -> Rect {
    let x = text_x(lay);
    let right = lay.field.right().min(lay.up.x);
    Rect::new(x, lay.field.y, right - x, lay.field.height)
}

pub fn access(
    tf: &TimeField,
    lay: &Layout,
    text: &TextRenderer,
    px: u32,
    tree: &mut Tree,
    parent: NodeId,
    key: impl std::hash::Hash,
) -> NodeId {
    let mut n = access::node(Role::TimeInput, lay.field);
    access::focusable(&mut n, tf.enabled);
    let id = tree.push(parent, key, n);
    let (value, parts) = tf.text();
    let part = parts.get(tf.part).cloned().unwrap_or(0..0);
    access::value_run(tree, id, access_text_rect(lay), &value, part, text, px);
    id
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::look::classic_paint;
    use crate::theme::{FontConfig, Theme};

    #[test]
    fn text_in_both_clocks() {
        let tf = TimeField::new(23, 41, 24, true);
        assert_eq!(tf.text().0, "11:41:24 PM");
        assert_eq!(tf.text().1, vec![0..2, 3..5, 6..8, 9..11]);
        assert_eq!(TimeField::new(0, 5, 7, true).text().0, "12:05:07 AM");
        assert_eq!(TimeField::new(9, 5, 7, false).text().0, "09:05:07");
    }

    #[test]
    fn stepping_wraps_each_part() {
        let mut tf = TimeField::new(11, 59, 0, true);
        assert!(tf.step(1));
        assert_eq!(tf.h, 0, "11 AM steps to 12 AM, the same half");
        tf.select(1);
        tf.step(1);
        assert_eq!(tf.m, 0);
        tf.select(3);
        tf.step(-1);
        assert_eq!(tf.h, 12, "AM/PM flips the half");
        let mut t24 = TimeField::new(23, 0, 0, false);
        t24.step(1);
        assert_eq!(t24.h, 0);
        t24.select(3);
        assert_eq!(t24.part, 2, "no AM/PM part in 24-hour time");
    }

    #[test]
    fn without_seconds_the_field_is_hours_and_minutes() {
        let mut tf = TimeField::hm(21, 0, false);
        assert_eq!(tf.text(), ("21:00".to_string(), vec![0..2, 3..5]));
        tf.select(5);
        assert_eq!(tf.part, 1, "the minutes are the last part");
        tf.step(-1);
        assert_eq!((tf.h, tf.m, tf.s), (21, 59, 0));
        let mut t12 = TimeField::hm(21, 5, true);
        assert_eq!(t12.text(), ("9:05 PM".to_string(), vec![0..1, 2..4, 5..7]));
        t12.select(2);
        t12.char('a');
        assert_eq!(t12.h, 9, "the third part is AM/PM");
        t12.step(1);
        assert_eq!((t12.h, t12.m), (21, 5));
        let l = layout(Rect::new(0, 0, 80, 20), &ControlMetrics::CLASSIC);
        let text = TextRenderer::load(&crate::theme::FontConfig::default());
        let past = text.measure("21:00", false, 11) + 20;
        assert_eq!(
            hit(&TimeField::hm(21, 0, false), &l, past, 10, &text, 11),
            Some(Hit::Part(1)),
            "past the text is the minutes"
        );
    }

    #[test]
    fn typing_digits_and_halves() {
        let mut tf = TimeField::new(14, 0, 0, false);
        tf.select(1);
        tf.char('4');
        tf.char('5');
        assert_eq!(tf.m, 45);
        tf.char('7');
        assert_eq!(tf.m, 7, "a third digit starts over");
        tf.char('9');
        assert_eq!(tf.m, 9, "79 is no minute, so the 9 starts over");
        let mut t12 = TimeField::new(14, 0, 0, true);
        t12.char('9');
        assert_eq!(t12.h, 21, "9 PM stays PM");
        t12.select(3);
        t12.char('a');
        assert_eq!(t12.h, 9);
    }

    /// Blue's time field at the Date and Time sheet's 103x20: the field
    /// is the whole rect, and the 15 px buttons stand inside its line, one
    /// row apart: the field is 20 tall, one row short of the year spin's
    /// 21, so the up button gives up its spare row to the gap, 7 px over
    /// 8.
    #[test]
    fn a_blue_time_field_puts_its_buttons_inside_the_field() {
        let theme = crate::interfaces::theme_for(crate::interface::Interface::Blue);
        let text = TextRenderer::load(&FontConfig::default());
        let cs = &theme.colors;
        let r = Rect::new(249, 201, 103, 20);
        let l = layout(r, &theme.look.controls());
        assert_eq!(l.field, r);
        assert_eq!(l.up, Rect::new(335, 203, 15, 7));
        assert_eq!(l.down, Rect::new(335, 211, 15, 8));
        let tf = TimeField::new(23, 41, 24, true);
        assert_eq!(hit(&tf, &l, 340, 205, &text, 11), Some(Hit::Up));
        assert_eq!(hit(&tf, &l, 340, 215, &text, 11), Some(Hit::Down));
        assert_eq!(hit(&tf, &l, 256, 208, &text, 11), Some(Hit::Part(0)));
        let mut c = Canvas::new(360, 230);
        c.fill_rect(c.bounds(), cs.button_face);
        let p = theme.paint();
        draw(&mut c, &text, &p, &tf, &l, false, None, 11);
        let line = crate::blue::Variant::Blue
            .scheme()
            .edit_border
            .normal
            .rim
            .argb();
        let (white, face) = (cs.window.argb(), cs.button_face.argb());
        for y in 201..221 {
            assert_eq!(c.get(351, y), line, "the right line at y {y}");
        }
        for x in 334..351 {
            assert_eq!(c.get(x, 202), white, "the well's top at x {x}");
            assert_eq!(c.get(x, 219), white, "the well's bottom at x {x}");
        }
        for x in 335..350 {
            assert_eq!(c.get(x, 210), face, "the gap row at x {x}");
        }
        let mut b = Canvas::new(360, 230);
        b.fill_rect(b.bounds(), cs.window);
        crate::look::widgets::draw_spin_buttons(&mut b, &p, l.up, l.down, true, None);
        for y in (203..210).chain(211..219) {
            assert_eq!(c.get(334, y), white, "the well's left side at y {y}");
            for x in 335..350 {
                assert_eq!(c.get(x, y), b.get(x, y), "button art at ({x}, {y})");
            }
        }
    }

    #[test]
    fn the_access_text_stops_at_the_buttons() {
        let r = Rect::new(249, 201, 103, 20);
        let blue = crate::interfaces::theme_for(crate::interface::Interface::Blue);
        let l = layout(r, &blue.look.controls());
        assert_eq!(access_text_rect(&l), Rect::new(252, 201, 83, 20));
        let l = layout(r, &ControlMetrics::CLASSIC);
        assert_eq!(
            access_text_rect(&l),
            Rect::new(252, 201, 83, 20),
            "Classic's runs to its field's right edge, left of the buttons"
        );
    }

    #[test]
    fn a_greyed_blue_time_field_inks_its_text_flat() {
        let theme = crate::interfaces::theme_for(crate::interface::Interface::Blue);
        let text = TextRenderer::load(&FontConfig::default());
        let cs = &theme.colors;
        let l = layout(Rect::new(249, 201, 103, 20), &theme.look.controls());
        let mut tf = TimeField::new(23, 41, 24, true);
        tf.enabled = false;
        let mut c = Canvas::new(360, 230);
        c.fill_rect(c.bounds(), cs.button_face);
        draw(&mut c, &text, &theme.paint(), &tf, &l, false, None, 11);
        let ink: Vec<u32> = (203..219)
            .flat_map(|y| (251..334).map(move |x| (x, y)))
            .map(|(x, y)| c.get(x, y))
            .collect();
        assert!(ink.contains(&cs.gray_text.argb()), "the time inks #ACA899");
        assert!(
            !ink.contains(&cs.window.argb()),
            "with no white copy under it"
        );
    }

    #[test]
    fn layout_hit_and_pixels() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let cs = &theme.colors;
        let l = layout(Rect::new(249, 198, 103, 20), &ControlMetrics::CLASSIC);
        assert_eq!(l.field, Rect::new(249, 198, 86, 20));
        assert_eq!(l.up, Rect::new(336, 198, 16, 9));
        assert_eq!(l.down, Rect::new(336, 207, 16, 9));
        let mut tf = TimeField::new(23, 41, 24, true);
        assert_eq!(hit(&tf, &l, 340, 200, &text, 11), Some(Hit::Up));
        assert_eq!(hit(&tf, &l, 340, 212, &text, 11), Some(Hit::Down));
        assert_eq!(hit(&tf, &l, 256, 205, &text, 11), Some(Hit::Part(0)));
        assert_eq!(hit(&tf, &l, 330, 205, &text, 11), Some(Hit::Part(3)));
        tf.select(1);
        let mut c = Canvas::new(360, 230);
        c.fill_rect(c.bounds(), cs.button_face);
        draw(
            &mut c,
            &text,
            &classic_paint(cs, 1),
            &tf,
            &l,
            true,
            None,
            11,
        );
        assert_eq!(c.get(249, 198), cs.button_shadow.argb());
        assert_eq!(c.get(334, 217), cs.button_hilight.argb());
        assert_eq!(c.get(335, 205), cs.button_face.argb(), "the gap");
        // Up arrow tip (343, 202); down arrow base on row 210, tip (343, 211).
        assert_eq!(c.get(343, 202), cs.button_text.argb());
        assert_eq!(c.get(342, 203), cs.button_text.argb());
        assert_eq!(c.get(342, 210), cs.button_text.argb());
        assert_eq!(c.get(343, 211), cs.button_text.argb());
        // The minutes are on the highlight.
        let x0 = 249 + 3 + text.measure("11", false, 11) + 2 + text.measure(":", false, 11);
        assert_eq!(c.get(x0, 202), cs.hilight.argb());
    }
}
