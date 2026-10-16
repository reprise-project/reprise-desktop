//! A task button's progress in Blue: a rounded bar along the bottom shaded like the scheme's
//! progress blocks over a faint track; the same shading in grey while paused.

use super::Variant;
use super::scheme::Grad;
use crate::raster::Canvas;
use crate::shape::{self, Px, Stops, over, premul};
use crate::taskbar::{ProgressPainter, TaskProgress};
use crate::theme::ColorScheme;
use std::sync::Arc;

const INSET_X: f64 = 5.0;
/// From the button's bottom to the bar's bottom.
const BOTTOM: f64 = 3.0;
const BAR_H: f64 = 4.0;
const RADIUS: f64 = 2.0;
const TRACK_ALPHA: u8 = 0x50;

fn stops(g: Grad) -> Vec<(f64, Px)> {
    g.iter()
        .map(|&(t, c)| (f64::from(t), premul(c, 255)))
        .collect()
}

/// `p` at its own brightness with no colour.
fn grey((a, [r, g, b]): Px) -> Px {
    let y = 0.299 * r + 0.587 * g + 0.114 * b;
    (a, [y, y, y])
}

pub(super) fn painter(v: Variant) -> ProgressPainter {
    let running = stops(v.scheme().progress_chunk.normal.down);
    let paused = running
        .iter()
        .map(|&(t, px)| (t, grey(px)))
        .collect::<Vec<_>>();
    Arc::new(move |p, cs, w, h, d| paint(p, cs, w, h, d, &running, &paused))
}

fn paint(
    p: &TaskProgress,
    cs: &ColorScheme,
    w: i32,
    h: i32,
    d: i32,
    running: &[(f64, Px)],
    paused: &[(f64, Px)],
) -> Canvas {
    let k = f64::from(d);
    let down = Stops(if p.paused { paused } else { running });
    let (x0, y0) = (INSET_X * k, (f64::from(h) - BOTTOM - BAR_H) * k);
    let (bw, bh) = ((f64::from(w) - 2.0 * INSET_X).max(0.0) * k, BAR_H * k);
    let fw = bw * p.fraction.clamp(0.0, 1.0);
    let track = premul(cs.button_shadow, TRACK_ALPHA);
    shape::draw(w, h, d, |x, y| {
        let (lx, ly) = (x - x0, y - y0);
        let under = shape::scale(track, shape::rounded(lx, ly, bw, bh, RADIUS * k));
        let cov = shape::rounded(lx, ly, fw, bh, RADIUS * k);
        let t = ((ly + 0.5) / bh).clamp(0.0, 1.0);
        over(shape::scale(down.at(t), cov), under)
    })
}
