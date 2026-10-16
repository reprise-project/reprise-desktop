//! A task button's progress in Glass: a soft green fill behind the icon and label, growing from
//! the left; amber while paused.

use crate::raster::Canvas;
use crate::shape::{self, mix, premul};
use crate::taskbar::TaskProgress;
use crate::theme::{Color, ColorScheme};

const INSET: f64 = 2.0;
const RADIUS: f64 = 3.0;
const GREEN: [Color; 2] = [Color::rgb(0x9C, 0xF0, 0x7C), Color::rgb(0x2F, 0xA8, 0x3C)];
const AMBER: [Color; 2] = [Color::rgb(0xFF, 0xE0, 0x7A), Color::rgb(0xE0, 0x96, 0x12)];
/// The fill's opacity at its top and bottom rows.
const ALPHA: [u8; 2] = [0x58, 0xA8];

pub(super) fn paint(p: &TaskProgress, _: &ColorScheme, w: i32, h: i32, d: i32) -> Canvas {
    let k = f64::from(d);
    let [a, b] = if p.paused { AMBER } else { GREEN };
    let (top, low) = (premul(a, ALPHA[0]), premul(b, ALPHA[1]));
    let (bw, bh) = (
        (f64::from(w) - 2.0 * INSET).max(0.0) * k,
        (f64::from(h) - 2.0 * INSET).max(0.0) * k,
    );
    let fw = bw * p.fraction.clamp(0.0, 1.0);
    shape::draw(w, h, d, |x, y| {
        let (lx, ly) = (x - INSET * k, y - INSET * k);
        let cov = shape::rounded(lx, ly, fw, bh, RADIUS * k);
        let t = ((ly + 0.5) / bh).clamp(0.0, 1.0);
        shape::scale(mix(top, low, t), cov)
    })
}
