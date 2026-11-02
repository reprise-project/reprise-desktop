//! The analog clock of a look without its own: a round dial sunk into the face, twelve hour ticks
//! and the hands in the scheme's window colours, the second hand in its highlight.

use std::f64::consts::TAU;

use crate::clock::turns;
use crate::raster::Canvas;
use crate::shape::{CLEAR, Px, draw, ellipse, over, premul, scale, stroke};
use crate::theme::{Color, ColorScheme};

/// Design px: the sunken rim's two rings, and how far inside the dial the ticks start.
const RIM: f64 = 1.0;
const TICK_IN: f64 = 4.0;

/// A tick's length and width, design px: whole pixels wide, so they stay crisp at scale 1.
const HOUR_TICK: (f64, f64) = (5.0, 1.0);
const QUARTER_TICK: (f64, f64) = (8.0, 2.0);

/// A hand's reach past the middle (a share of the dial's radius) and width, design px.
const HOUR_HAND: (f64, f64) = (0.5, 3.0);
const MINUTE_HAND: (f64, f64) = (0.78, 2.0);
const SECOND_HAND: (f64, f64) = (0.84, 1.0);
/// The hub's radius, design px.
const HUB: f64 = 3.0;

/// The clock at `time` on a `side` design px square at scale `d`, the second hand only with
/// `seconds`. Outside the dial the canvas is clear.
pub fn paint(cs: &ColorScheme, time: (u32, u32, u32), seconds: bool, side: i32, d: i32) -> Canvas {
    let k = f64::from(d);
    let mid = f64::from(side) * k / 2.0;
    let outer = mid;
    let dial = outer - 2.0 * RIM * k;
    let (hour, minute, second) = turns(time);
    let way = |t: f64| ((t * TAU).sin(), -(t * TAU).cos());
    let hand = |(reach, _): (f64, f64), t: f64| {
        let (ux, uy) = way(t);
        [
            (mid, mid),
            (mid + ux * dial * reach, mid + uy * dial * reach),
        ]
    };
    let hands = [
        (hand(HOUR_HAND, hour), HOUR_HAND.1),
        (hand(MINUTE_HAND, minute), MINUTE_HAND.1),
    ];
    let sweep = hand(SECOND_HAND, second);
    let solid = |c: Color| premul(c, 255);
    draw(side, side, d, |x, y| {
        let disc = |r: f64| ellipse(x, y, mid, mid, r, r);
        let on = disc(outer);
        if on == 0.0 {
            return CLEAR;
        }
        // Lit from the top left, so the sunken rim is dark there and light at the bottom right.
        let lit = (x + 0.5 - mid) + (y + 0.5 - mid) > 0.0;
        let (rim, inner) = if lit {
            (cs.button_hilight, cs.button_light)
        } else {
            (cs.button_shadow, cs.button_dk_shadow)
        };
        let mut px = scale(solid(rim), on);
        px = over(scale(solid(inner), disc(outer - RIM * k)), px);
        let on_dial = disc(dial);
        if on_dial == 0.0 {
            return px;
        }
        px = over(scale(solid(cs.window), on_dial), px);
        px = over(tick(cs.window_text, x, y, mid, dial - TICK_IN * k, k), px);
        for (pts, width) in &hands {
            px = over(
                scale(solid(cs.window_text), stroke(x, y, pts, width * k)),
                px,
            );
        }
        if seconds {
            px = over(
                scale(solid(cs.hilight), stroke(x, y, &sweep, SECOND_HAND.1 * k)),
                px,
            );
        }
        over(scale(solid(cs.window_text), disc(HUB * k)), px)
    })
}

/// The hour tick nearest pixel (`x`, `y`), longer and wider each quarter, starting `from` device
/// px out from the middle at `mid`.
fn tick(ink: Color, x: f64, y: f64, mid: f64, from: f64, k: f64) -> Px {
    let (cx, cy) = (x + 0.5 - mid, y + 0.5 - mid);
    let n = (cx.atan2(-cy) / TAU * 12.0).round().rem_euclid(12.0) as u32;
    let (len, width) = if n.is_multiple_of(3) {
        QUARTER_TICK
    } else {
        HOUR_TICK
    };
    let t = f64::from(n) / 12.0;
    let (ux, uy) = ((t * TAU).sin(), -(t * TAU).cos());
    let (a, b) = (from, from - len * k);
    let pts = [(mid + ux * a, mid + uy * a), (mid + ux * b, mid + uy * b)];
    scale(premul(ink, 255), stroke(x, y, &pts, width * k))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIDE: i32 = 101;

    fn cs() -> ColorScheme {
        ColorScheme::default()
    }

    /// The pixel `r` design px out from the middle towards `turn`, at `d`.
    fn out(c: &Canvas, turn: f64, r: f64, d: i32) -> u32 {
        let mid = f64::from(SIDE * d) / 2.0;
        let k = f64::from(d);
        let (x, y) = (
            mid + (turn * TAU).sin() * r * k,
            mid - (turn * TAU).cos() * r * k,
        );
        c.get(x.floor() as i32, y.floor() as i32)
    }

    #[test]
    fn the_hands_point_where_the_time_says_and_the_second_hand_shows_only_with_seconds() {
        let dial = f64::from(SIDE) / 2.0 - 2.0 * RIM;
        let cs = cs();
        for d in [1, 2] {
            let c = paint(&cs, (3, 30, 45), true, SIDE, d);
            let (hour, minute, second) = turns((3, 30, 45));
            assert_eq!(
                out(&c, hour, dial * 0.35, d),
                cs.window_text.argb(),
                "hour at {d}"
            );
            assert_eq!(
                out(&c, minute, dial * 0.6, d),
                cs.window_text.argb(),
                "minute at {d}"
            );
            assert_eq!(
                out(&c, second, dial * 0.6, d),
                cs.hilight.argb(),
                "second at {d}"
            );
            let plain = paint(&cs, (3, 30, 45), false, SIDE, d);
            assert_eq!(
                out(&plain, second, dial * 0.6, d),
                cs.window.argb(),
                "no second hand at {d}"
            );
        }
    }

    #[test]
    fn the_dial_sits_in_a_sunken_rim_with_clear_corners_and_soft_edges_at_2() {
        let cs = cs();
        let c = paint(&cs, (10, 10, 0), false, SIDE, 1);
        assert_eq!((c.width, c.height), (101, 101));
        assert_eq!(c.get(0, 0), 0, "a clear corner");
        assert_eq!(c.get(50, 0), cs.button_shadow.argb(), "dark rim at the top");
        assert_eq!(
            c.get(50, 100),
            cs.button_hilight.argb(),
            "light rim at the foot"
        );
        assert_eq!(
            out(&c, 0.25, 50.0 - 2.0 - TICK_IN - 3.0, 1),
            cs.window_text.argb(),
            "3 o'clock tick"
        );
        let two = paint(&cs, (10, 10, 0), false, SIDE, 2);
        assert_eq!((two.width, two.height), (202, 202));
        let row: Vec<u32> = (0..60).map(|x| two.get(x, 30) >> 24).collect();
        assert!(
            row.iter().any(|&a| a > 0x20 && a < 0xE0),
            "a soft edge at 2: {row:x?}"
        );
    }
}
