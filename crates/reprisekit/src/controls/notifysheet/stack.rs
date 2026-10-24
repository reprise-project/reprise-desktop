//! The stack by the tray, its arrival poses, the Droplet drip and the "+N more" chip.

use crate::anim::Ease;
use crate::backdrop::round_corner;
use crate::raster::{Canvas, Rect};
use crate::shape::smooth;
use crate::style::State;
use crate::taskbar::Edge;
use crate::text::{Align, TextRenderer};
use crate::theme::Theme;

use super::notify_look;

/// The chip's metrics, design px.
const CHIP_H: i32 = 28;
const CHIP_PX: u32 = 12;
const CHIP_PAD: i32 = 12;

pub fn chip_label(more: usize) -> String {
    format!("+{more} more")
}

/// The chip's size for `more` waiting, device px.
pub fn chip_layout(theme: &Theme, text: &TextRenderer, more: usize) -> (i32, i32) {
    let s = theme.scale.max(1);
    let label = text.measure(&chip_label(more), false, CHIP_PX * s as u32);
    (((label + s - 1) / s + 2 * CHIP_PAD) * s, CHIP_H * s)
}

pub fn draw_chip(theme: &Theme, text: &TextRenderer, more: usize, hot: bool) -> Canvas {
    let s = theme.scale.max(1);
    let (w, h) = chip_layout(theme, text, more);
    let Some(look) = notify_look(theme) else {
        return Canvas::new(w as u32, h as u32);
    };
    let state = if hot { State::Hot } else { State::Normal };
    let mut c = (look.chip)(&look, (w / s, h / s), state, s);
    let r = Rect::new(0, 0, w, h);
    let label = chip_label(more);
    let px = CHIP_PX * s as u32;
    text.draw(&mut c, r, &label, false, look.inks.text, Align::Center, px);
    c
}

/// A top-left, logical px.
pub type Point = (i32, i32);

const STACK_GAP: i32 = 8;
const STACK_INSET: i32 = 8;

/// Top-lefts for `footprints` (newest first, nearest the tray) and the `chip` past them, logical
/// px, from `usable`'s corner by the tray along `edge`, kept inside it.
pub fn place_stack(
    usable: Rect,
    footprints: &[(i32, i32)],
    chip: Option<(i32, i32)>,
    edge: Edge,
) -> (Vec<Point>, Option<Point>) {
    let (left, down) = match edge {
        Edge::Bottom | Edge::Right => (false, false),
        Edge::Left => (true, false),
        Edge::Top => (false, true),
    };
    let mut y = if down {
        usable.y + STACK_INSET
    } else {
        usable.bottom() - STACK_INSET
    };
    let mut place = |(w, h): (i32, i32)| {
        let x = if left {
            usable.x + STACK_INSET
        } else {
            usable.right() - STACK_INSET - w
        };
        let top = if down { y } else { y - h };
        y = if down {
            y + h + STACK_GAP
        } else {
            y - h - STACK_GAP
        };
        (
            x.min(usable.right() - w).max(usable.x),
            top.min(usable.bottom() - h).max(usable.y),
        )
    };
    let at = footprints.iter().map(|&f| place(f)).collect();
    (at, chip.map(place))
}

pub const RISE: i32 = 14;

/// A sheet `k` of the way in to `rest` and its opacity: scaled from `scale_from` about a centre
/// `rise` px toward the tray (`toward_tray` +1 down, -1 up).
pub fn pose(k: f64, rest: Rect, rise: i32, toward_tray: i32, scale_from: f64) -> (Rect, f32) {
    let k = k.clamp(0.0, 1.0);
    let f = scale_from + (1.0 - scale_from) * k;
    let (w, h) = (
        (f64::from(rest.width) * f).round(),
        (f64::from(rest.height) * f).round(),
    );
    let cx = f64::from(rest.x) + f64::from(rest.width) / 2.0;
    let cy = f64::from(rest.y)
        + f64::from(rest.height) / 2.0
        + f64::from(rise) * (1.0 - k) * f64::from(toward_tray);
    let r = Rect::new(
        (cx - w / 2.0).round() as i32,
        (cy - h / 2.0).round() as i32,
        w as i32,
        h as i32,
    );
    (r, k as f32)
}

const BEAD: i32 = 12;
/// The drip holds its bead until `BEAD_UNTIL`, then swells; the bead is gone `BEAD_FADE` into
/// the swell.
pub(super) const BEAD_UNTIL: f64 = 0.25;
pub(super) const BEAD_FADE: f64 = 0.3;

/// The drip's duration for a sheet whose plain arrival takes `open_ms`.
pub fn drip_ms(open_ms: u32) -> u32 {
    open_ms * 12 / 5
}

/// A droplet `k` of the way in: a bead at the tray-side cap that swells with an overshoot into
/// `rest`. `tray_right` is true for every edge but Left.
pub fn drip_pose(k: f64, rest: Rect, tray_right: bool) -> (Rect, f32) {
    let k = k.clamp(0.0, 1.0);
    let opacity = (k / 0.1).min(1.0) as f32;
    if k >= 1.0 {
        return (rest, 1.0);
    }
    let cy = rest.y + rest.height / 2;
    let bead_x = if tray_right {
        rest.right() - rest.height / 2 - BEAD / 2
    } else {
        rest.x + rest.height / 2 - BEAD / 2
    };
    if k < BEAD_UNTIL {
        return (Rect::new(bead_x, cy - BEAD / 2, BEAD, BEAD), opacity);
    }
    let s = ((k - BEAD_UNTIL) / (1.0 - BEAD_UNTIL)) as f32;
    let e = f64::from(Ease::OutBack.apply(s));
    let e_h = f64::from(Ease::OutBack.apply((s - 0.08).max(0.0) / 0.92));
    let lerp = |a: i32, b: i32, t: f64| f64::from(a) + f64::from(b - a) * t;
    let w = lerp(BEAD, rest.width, e).round() as i32;
    let h = lerp(BEAD, rest.height, e_h).round() as i32;
    let x = if tray_right {
        lerp(bead_x + BEAD, rest.right(), e).round() as i32 - w
    } else {
        lerp(bead_x, rest.x, e).round() as i32
    };
    (Rect::new(x, cy - h / 2, w, h), opacity)
}

/// How much of a drip `k` of the way in shows as the bead, the sheet showing the rest.
pub fn drip_bead_share(k: f64) -> f32 {
    let swell = (k - BEAD_UNTIL) / (1.0 - BEAD_UNTIL);
    (1.0 - smooth(swell / BEAD_FADE)) as f32
}

/// The bead's body for a drip posed at `p` ([`drip_pose`]): round, as tall as `p`, at its
/// tray-side end.
pub fn drip_bead_at(p: Rect, tray_right: bool) -> Rect {
    let side = p.height.min(p.width);
    let x = if tray_right { p.right() - side } else { p.x };
    Rect::new(x, p.y + (p.height - side) / 2, side, side)
}

/// The glass bead a Droplet drips from and its body's rect in the canvas, device px.
pub fn drip_bead(theme: &Theme) -> Option<(Canvas, Rect)> {
    let look = notify_look(theme)?;
    let s = theme.scale.max(1);
    let c = (look.bead)(&look, BEAD, s);
    let side = BEAD * s;
    let at = |n: u32| (n as i32 - side) / 2;
    let body = Rect::new(at(c.width), at(c.height), side, side);
    Some((c, body))
}

/// The largest round corner table whose radius fits half of `height`; radius 6 below that.
pub fn drip_corner(height: i32) -> &'static [i32] {
    const R6: [i32; 5] = round_corner();
    const R12: [i32; 11] = round_corner();
    const R20: [i32; 19] = round_corner();
    const R34: [i32; 33] = round_corner();
    match height / 2 {
        34.. => &R34,
        20.. => &R20,
        12.. => &R12,
        _ => &R6,
    }
}
