//! The taskbar's network meter: download over upload, each reading right-aligned in a block as
//! wide as the widest reading, so the cell holds still while the numbers change.

use crate::raster::{Canvas, Rect};
use crate::text::{Align, TextRenderer};
use crate::theme::Color;

/// The widest readings: three digits and the widest unit, under either arrow.
const WIDEST: [&str; 6] = [
    "\u{2193} 888 MB/s",
    "\u{2193} 888 KB/s",
    "\u{2193} 888 GB/s",
    "\u{2191} 888 MB/s",
    "\u{2191} 888 KB/s",
    "\u{2191} 888 GB/s",
];

/// The meter's font size for a bar font of `size_px` at scale `s`: two px smaller, so two lines
/// fit a 22 px button row.
pub fn font_px(size_px: u32, s: i32) -> u32 {
    size_px.saturating_sub(2 * s.max(1) as u32).max(1)
}

/// The text block, width by height, at `font_px`: the widest reading over two lines.
pub fn size(text: &TextRenderer, font_px: u32) -> (i32, i32) {
    let w = WIDEST
        .iter()
        .map(|t| text.measure(t, false, font_px))
        .max()
        .unwrap_or(0);
    (w, 2 * line_height(text, font_px))
}

fn line_height(text: &TextRenderer, font_px: u32) -> i32 {
    text.line(Rect::new(0, 0, 0, 0), false, font_px)
        .map_or(font_px as i32, |(_, h)| h)
}

/// Draw `lines` (download, upload) in `r`: the block centred in it, each line right-aligned.
pub fn draw(
    canvas: &mut Canvas,
    text: &TextRenderer,
    r: Rect,
    lines: &[String; 2],
    ink: Color,
    font_px: u32,
) {
    let (w, h) = size(text, font_px);
    let lh = h / 2;
    let x = r.x + (r.width - w) / 2;
    let top = r.y + (r.height - h) / 2;
    for (i, line) in lines.iter().enumerate() {
        let row = Rect::new(x, top + i as i32 * lh, w, lh);
        text.draw_clipped(canvas, row, r, line, false, ink, Align::Right, font_px);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::FontConfig;

    #[test]
    fn the_block_fits_every_reading_and_two_lines_fit_a_classic_row() {
        let text = TextRenderer::load(&FontConfig::default());
        let px = font_px(11, 1);
        assert_eq!(px, 9);
        let (w, h) = size(&text, px);
        assert!(h <= 22, "two lines at {px} px are {h} px");
        for r in ["\u{2193} 999 B/s", "\u{2191} 9.9 MB/s", "\u{2193} 120 KB/s"] {
            assert!(text.measure(r, false, px) <= w, "{r}");
        }
    }

    #[test]
    fn readings_are_right_aligned_so_the_unit_holds_still() {
        let text = TextRenderer::load(&FontConfig::default());
        let px = font_px(11, 1);
        let (w, h) = size(&text, px);
        let r = Rect::new(0, 0, w + 10, h + 4);
        let ink = Color::rgb(0, 0, 0);
        let right_edge = |lines: [&str; 2]| {
            let mut c = Canvas::new(r.width as u32, r.height as u32);
            draw(&mut c, &text, r, &lines.map(String::from), ink, px);
            (0..r.width)
                .rev()
                .find(|&x| (0..r.height).any(|y| c.get(x, y) >> 24 != 0))
        };
        let a = right_edge(["\u{2193} 2.4 MB/s", "\u{2191} 12 KB/s"]);
        let b = right_edge(["\u{2193} 999 B/s", "\u{2191} 0 B/s"]);
        assert!(a.is_some());
        assert_eq!(a, b);
    }

    fn bar(
        i: crate::interface::Interface,
        scale: i32,
        lines: [&str; 2],
    ) -> (Canvas, crate::taskbar::TaskbarLayout) {
        use crate::taskbar::{TaskbarCounts, TaskbarLayout, TaskbarState};
        let base = crate::interfaces::theme_for(i);
        let text = TextRenderer::load(&base.fonts);
        let meter = size(&text, font_px(base.fonts.size_px, 1));
        let counts = TaskbarCounts {
            tray: 2,
            meter: Some(meter),
            ..TaskbarCounts::default()
        };
        let clock = text.measure("12:00", false, base.fonts.size_px);
        let layout = TaskbarLayout::compute(
            &base.metrics,
            &base.taskbar_shape(),
            400,
            true,
            counts,
            clock,
        )
        .scaled(scale);
        let state = TaskbarState {
            clock: "12:00".into(),
            tray_icons: vec![None, None],
            meter: Some(lines.map(String::from)),
            ..TaskbarState::default()
        };
        let theme = base.scaled(scale);
        let text = text.for_output(scale);
        let mut c = Canvas::new(layout.width as u32, layout.height as u32);
        crate::look::draw_taskbar(&mut c, &theme, text, text, &layout, &state);
        (c, layout)
    }

    fn hash(c: &Canvas, r: Rect) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for y in r.y..r.bottom() {
            for x in r.x..r.right() {
                for b in c.get(x, y).to_le_bytes() {
                    h = (h ^ b as u64).wrapping_mul(0x0100_0000_01b3);
                }
            }
        }
        h
    }

    #[test]
    fn the_meter_sits_between_the_tray_and_the_clock() {
        use crate::interface::Interface;
        for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
            let (_, l) = bar(i, 1, ["", ""]);
            let m = l.meter.unwrap();
            assert_eq!(m.right(), l.clock.x - 6, "{i:?}");
            assert_eq!(l.tray.last().unwrap().right(), m.x - 4, "{i:?}");
            assert!(l.notification.x <= l.tray[0].x && l.notification.right() >= m.right());
            assert_eq!(l.hit(m.x + 1, m.y + 1), crate::taskbar::TaskbarHit::Meter);
        }
    }

    #[test]
    fn a_side_bar_stacks_the_meter_over_its_clock() {
        use crate::taskbar::{Bands, BarSpan, ClockText, Edge, TaskbarCounts, TaskbarLayout};
        let base = crate::interfaces::theme_for(crate::interface::Interface::Classic);
        let span = BarSpan {
            edge: Edge::Left,
            length: 700,
            thickness: 100,
        };
        let counts = TaskbarCounts {
            tray: 3,
            meter: Some((60, 22)),
            ..TaskbarCounts::default()
        };
        let clock = ClockText { time: 30, date: 0 };
        let l = TaskbarLayout::compute_on(
            &base.metrics,
            &base.taskbar_shape(),
            span,
            true,
            counts,
            clock,
            Bands::default(),
        );
        let m = l.meter.unwrap();
        assert_eq!((m.x, m.width), (l.clock.x, l.clock.width));
        assert_eq!((m.bottom(), m.height), (l.clock.y - 6, 22));
        assert!(l.tray.iter().all(|r| r.bottom() <= m.y - 4));
    }

    #[test]
    fn the_meter_pins_in_each_look_at_scale_1_and_2() {
        use crate::interface::Interface;
        if !crate::text::test_tahoma() {
            return;
        }
        let mut got = Vec::new();
        for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
            for scale in [1, 2] {
                let (c, l) = bar(i, scale, ["\u{2193} 2.4 MB/s", "\u{2191} 120 KB/s"]);
                let m = l.meter.unwrap();
                got.push((format!("{i:?} {scale}"), hash(&c, m)));
                let (empty, _) = bar(i, scale, ["", ""]);
                assert_ne!(hash(&c, m), hash(&empty, m), "{i:?}@{scale}: inked");
            }
        }
        crate::pins::check("netmeter", &got);
    }
}
