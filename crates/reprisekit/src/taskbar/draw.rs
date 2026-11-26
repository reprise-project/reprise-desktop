use super::ClockLines;
use crate::raster::{Canvas, Rect};
use crate::text::{Align, TextRenderer};
use crate::theme::Color;

/// The ^ glyph for the tray's chevron cell: a 7x4 triangle times `s`.
pub fn draw_chevron(canvas: &mut Canvas, r: Rect, ink: Color, s: i32) {
    let s = s.max(1);
    let cx = r.x + r.width / 2;
    let top = r.y + (r.height - 4 * s) / 2;
    for i in 0..4 {
        canvas.fill_rect(Rect::new(cx - i * s, top + i * s, (2 * i + 1) * s, s), ink);
    }
}

fn fill_rows(canvas: &mut Canvas, rows: &[&str], x: i32, y: i32, ink: Color, s: i32) {
    for (j, row) in rows.iter().enumerate() {
        for (i, ch) in row.chars().enumerate() {
            if ch == '#' {
                let at = Rect::new(x + i as i32 * s, y + j as i32 * s, s, s);
                canvas.fill_rect(at, ink);
            }
        }
    }
}

/// Two chevrons 8x5 times `s`: `«` while the run is shut, `»` while `open`.
pub fn draw_tray_arrow(canvas: &mut Canvas, r: Rect, ink: Color, s: i32, open: bool) {
    const SHUT: [&str; 5] = ["..##..##", ".##..##.", "##..##..", ".##..##.", "..##..##"];
    const OPEN: [&str; 5] = ["##..##..", ".##..##.", "..##..##", ".##..##.", "##..##.."];
    let s = s.max(1);
    let x = r.x + (r.width / s - 8 + 1) / 2 * s;
    let y = r.y + (r.height / s - 5 + 1) / 2 * s;
    fill_rows(canvas, if open { &OPEN } else { &SHUT }, x, y, ink, s);
}

/// Side bar version: two chevrons stacked (5x8 times `s`), pointing up while shut and down while open.
pub fn draw_tray_arrow_side(canvas: &mut Canvas, r: Rect, ink: Color, s: i32, open: bool) {
    const UP: [&str; 8] = [
        "..#..", ".###.", "##.##", "#...#", "..#..", ".###.", "##.##", "#...#",
    ];
    const DOWN: [&str; 8] = [
        "#...#", "##.##", ".###.", "..#..", "#...#", "##.##", ".###.", "..#..",
    ];
    let s = s.max(1);
    let x = r.x + (r.width / s - 5) / 2 * s;
    let (rows, y) = if open { (&DOWN, 6) } else { (&UP, 2) };
    fill_rows(canvas, rows, x, r.y + y * s, ink, s);
}

/// The » glyph on an overflow cell (the task row's or Quick Launch's): two chevrons 4x7 times `s`.
pub fn draw_more(canvas: &mut Canvas, r: Rect, ink: Color, s: i32) {
    let s = s.max(1);
    let (x0, y0) = (r.x + (r.width - 10 * s) / 2, r.y + (r.height - 7 * s) / 2);
    for dx in [0, 6 * s] {
        for i in 0..4 {
            let col = Rect::new(x0 + dx + i * s, y0 + i * s, s, (7 - 2 * i) * s);
            canvas.fill_rect(col, ink);
        }
    }
}

/// The clock on two lines: time over date, each centred across `clock` on its baseline, counted
/// from the top of the [`ClockLines`] box centred in `clock`.
#[expect(clippy::too_many_arguments)]
pub fn draw_clock_lines(
    canvas: &mut Canvas,
    text: &TextRenderer,
    clock: Rect,
    lines: [&str; 2],
    at: ClockLines,
    ink: Color,
    size: u32,
    s: i32,
) {
    let top = clock.y + (clock.height - at.height * s) / 2;
    for (line, baseline) in lines.into_iter().zip(at.baselines) {
        let pen = Align::Center.pen(clock, text.measure(line, false, size));
        text.draw_run(
            canvas,
            pen,
            top + baseline * s,
            clock,
            line,
            false,
            ink,
            size,
        );
    }
}

/// Icon rect in a Quick Launch cell: 3 px in, centred vertically; the drag image is grabbed from here.
pub fn quick_icon_origin(r: Rect, icon_px: i32, s: i32) -> (i32, i32) {
    (r.x + 3 * s, r.y + (r.height - icon_px) / 2)
}

/// Coverage below this alpha is left out of [`symbolic`]'s test: its premultiplied channels round
/// too coarsely to tell a grey from a hue.
const SYMBOLIC_MIN_ALPHA: u32 = 0x40;
/// How far apart (per channel, 0..255) the greys of a symbolic icon may be.
const SYMBOLIC_SPREAD: u32 = 24;

/// Tray icon `icon` at (x, y). A symbolic one (a single grey at varying coverage) is drawn in
/// `ink`, the bar's label ink, so it reads on a pale bar and a dark one; a colour icon as it is.
pub fn draw_tray_icon(canvas: &mut Canvas, x: i32, y: i32, icon: &Canvas, ink: Color) {
    if !symbolic(icon) {
        canvas.blit_canvas(x, y, icon);
        return;
    }
    let mut inked = icon.clone();
    for p in &mut inked.pixels {
        *p = crate::icon::premultiply((*p & 0xFF00_0000) | (ink.argb() & 0x00FF_FFFF));
    }
    canvas.blit_canvas(x, y, &inked);
}

/// Whether every pixel of `icon` with real coverage is the same grey, within [`SYMBOLIC_SPREAD`].
fn symbolic(icon: &Canvas) -> bool {
    let mut range = (u32::MAX, 0);
    for &p in &icon.pixels {
        let a = p >> 24;
        if a < SYMBOLIC_MIN_ALPHA {
            continue;
        }
        let [r, g, b] = [16, 8, 0].map(|sh| ((p >> sh) & 0xFF) * 255 / a);
        let (lo, hi) = (r.min(g).min(b), r.max(g).max(b));
        if hi - lo > SYMBOLIC_SPREAD {
            return false;
        }
        range = (range.0.min(lo), range.1.max(hi));
    }
    range.0 != u32::MAX && range.1 - range.0 <= SYMBOLIC_SPREAD
}
