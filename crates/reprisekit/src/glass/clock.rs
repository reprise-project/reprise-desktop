//! The clock popup's analog clock: a round face in a bezel, minute and hour ticks, hour, minute
//! and second hands over a soft shadow, and a sheen across the face's top.

use std::f64::consts::TAU;

use super::Mode;
use crate::raster::Canvas;
use crate::shape::{
    Px, Stops, draw, ellipse, over, polygon, premul, radial, scale, smooth, stroke,
};
use crate::style::ClockPainter;
use crate::theme::Color;

/// The clock's side, design px: odd, so its middle is a pixel's and the 1 px second hand and
/// the 3 px quarter ticks fill whole pixels at scale 1.
pub const SIDE: i32 = 129;

struct Inks {
    rim: Color,
    /// The bezel's top and foot.
    bezel: (Color, Color),
    bezel_inner: Color,
    /// The face's middle and edge.
    face: (Color, Color),
    minute_tick: Color,
    hour_tick: Color,
    hand: Color,
    second: Color,
    /// The sheen's white at its top.
    sheen: u8,
    shadow: u8,
}

const LIGHT: Inks = Inks {
    rim: Color::rgb(0x7A, 0x87, 0x96),
    bezel: (Color::rgb(0xF6, 0xF8, 0xFB), Color::rgb(0xB2, 0xBE, 0xCB)),
    bezel_inner: Color::rgb(0x97, 0xA3, 0xB0),
    face: (Color::rgb(0xFF, 0xFF, 0xFF), Color::rgb(0xE2, 0xE9, 0xF0)),
    minute_tick: Color::rgb(0xA6, 0xAE, 0xB7),
    hour_tick: Color::rgb(0x34, 0x3B, 0x43),
    hand: Color::rgb(0x25, 0x2B, 0x31),
    second: Color::rgb(0xD6, 0x3E, 0x2A),
    sheen: 0x60,
    shadow: 0x50,
};

const DARK: Inks = Inks {
    rim: Color::rgb(0x0C, 0x0D, 0x0F),
    bezel: (Color::rgb(0x6A, 0x70, 0x78), Color::rgb(0x2C, 0x2F, 0x34)),
    bezel_inner: Color::rgb(0x14, 0x16, 0x19),
    face: (Color::rgb(0x3A, 0x3E, 0x44), Color::rgb(0x25, 0x28, 0x2C)),
    minute_tick: Color::rgb(0x68, 0x6E, 0x75),
    hour_tick: Color::rgb(0xD2, 0xD6, 0xDA),
    hand: Color::rgb(0xEC, 0xEE, 0xF0),
    second: Color::rgb(0xFF, 0x6B, 0x52),
    sheen: 0x28,
    shadow: 0x90,
};

/// Design px: the room left round the bezel for its shadow, the bezel's width, and how far
/// inside the face the ticks start.
const MARGIN: f64 = 3.0;
const BEZEL: f64 = 6.0;
const TICK_IN: f64 = 3.0;

/// A tick's length and width, design px.
const MINUTE_TICK: (f64, f64) = (3.0, 1.0);
const HOUR_TICK: (f64, f64) = (8.0, 2.0);
const QUARTER_TICK: (f64, f64) = (11.0, 3.0);

/// A hand: its reach past the middle and behind it (shares of the face's radius), and its
/// width at the middle and at the tip, design px.
struct Hand {
    reach: f64,
    tail: f64,
    base: f64,
    tip: f64,
}

const HOUR_HAND: Hand = Hand {
    reach: 0.52,
    tail: 0.12,
    base: 5.0,
    tip: 3.0,
};
const MINUTE_HAND: Hand = Hand {
    reach: 0.8,
    tail: 0.14,
    base: 4.0,
    tip: 2.0,
};
const SECOND_HAND: Hand = Hand {
    reach: 0.86,
    tail: 0.22,
    base: 1.0,
    tip: 1.0,
};

/// The hub under the second hand and the pin on it: radii, design px.
const HUB: f64 = 4.0;
const PIN: f64 = 1.5;

/// How far the hands' shadow falls right and down, design px.
const HAND_SHADOW: (f64, f64) = (0.8, 1.6);

/// The bezel's shadow: how far down it falls and over how many design px it fades out.
const DROP: f64 = 1.0;
const DROP_FADE: f64 = 2.5;

pub fn painter(mode: Mode) -> ClockPainter {
    match mode {
        Mode::Light => |t, side, d| paint(&LIGHT, t, side, d),
        Mode::Dark => |t, side, d| paint(&DARK, t, side, d),
    }
}

/// Where the hands point at `(h, m, s)`: hour, minute and second, in turns clockwise from 12.
pub(crate) fn turns((h, m, s): (u32, u32, u32)) -> (f64, f64, f64) {
    let (h, m, s) = (f64::from(h % 12), f64::from(m), f64::from(s));
    (
        (h + m / 60.0 + s / 3600.0) / 12.0,
        (m + s / 60.0) / 60.0,
        s / 60.0,
    )
}

fn paint(ink: &Inks, time: (u32, u32, u32), side: i32, d: i32) -> Canvas {
    let k = f64::from(d);
    let mid = f64::from(side) * k / 2.0;
    let outer = mid - MARGIN * k;
    let face = outer - BEZEL * k;
    let (hour, minute, second) = turns(time);
    let way = |t: f64| ((t * TAU).sin(), -(t * TAU).cos());
    let hand = |h: &Hand, t: f64| {
        let (ux, uy) = way(t);
        let (nx, ny) = (-uy, ux);
        let (back, tip) = (face * h.tail, face * h.reach);
        let (b, w) = (h.base * k / 2.0, h.tip * k / 2.0);
        [
            (mid - ux * back + nx * b, mid - uy * back + ny * b),
            (mid + ux * tip + nx * w, mid + uy * tip + ny * w),
            (mid + ux * tip - nx * w, mid + uy * tip - ny * w),
            (mid - ux * back - nx * b, mid - uy * back - ny * b),
        ]
    };
    let hands = [hand(&HOUR_HAND, hour), hand(&MINUTE_HAND, minute)];
    let (sx, sy) = way(second);
    let sweep = [
        (
            mid - sx * face * SECOND_HAND.tail,
            mid - sy * face * SECOND_HAND.tail,
        ),
        (
            mid + sx * face * SECOND_HAND.reach,
            mid + sy * face * SECOND_HAND.reach,
        ),
    ];
    let c = |col: Color| premul(col, 255);
    let bezel = [(0.0, c(ink.bezel.0)), (1.0, c(ink.bezel.1))];
    let face_fill = [
        (0.0, c(ink.face.0)),
        (0.6, c(ink.face.0)),
        (1.0, c(ink.face.1)),
    ];
    let sheen_top = mid - face * 0.95;
    let sheen_h = face * 1.0;
    draw(side, side, d, |x, y| {
        let (cx, cy) = (x + 0.5 - mid, y + 0.5 - mid);
        let lift = (cx * cx + (cy - DROP * k).powi(2)).sqrt();
        let fade = smooth((outer + DROP_FADE * k - lift) / (DROP_FADE * k));
        let mut px = scale(premul(Color::rgb(0, 0, 0), ink.shadow), fade);
        let disc = |r: f64| ellipse(x, y, mid, mid, r, r);
        let on_bezel = disc(outer);
        if on_bezel == 0.0 && fade == 0.0 {
            return (0.0, [0.0; 3]);
        }
        px = over(scale(c(ink.rim), on_bezel), px);
        let down = (cy + outer) / (2.0 * outer);
        px = over(scale(Stops(&bezel).at(down), disc(outer - k)), px);
        px = over(scale(c(ink.bezel_inner), disc(face + k)), px);
        let on_face = disc(face);
        if on_face == 0.0 {
            return px;
        }
        let fill = radial(x, y, mid, mid, face, face, &Stops(&face_fill));
        px = over(scale(fill, on_face), px);
        px = over(tick(ink, x, y, mid, face - TICK_IN * k, k), px);
        let shade = |pts: &[(f64, f64)]| polygon(x - HAND_SHADOW.0 * k, y - HAND_SHADOW.1 * k, pts);
        let shadow = shade(&hands[0]).max(shade(&hands[1]));
        px = over(scale(premul(Color::rgb(0, 0, 0), 0x38), shadow), px);
        for pts in &hands {
            px = over(scale(c(ink.hand), polygon(x, y, pts)), px);
        }
        let red = stroke(x, y, &sweep, SECOND_HAND.base * k).max(disc(HUB * k));
        px = over(scale(c(ink.second), red), px);
        px = over(scale(c(ink.hand), disc(PIN * k)), px);
        let t = (y + 0.5 - sheen_top) / sheen_h;
        let lit = ellipse(
            x,
            y,
            mid,
            sheen_top + sheen_h / 2.0,
            face * 0.86,
            sheen_h / 2.0,
        );
        let white = premul(Color::rgb(0xFF, 0xFF, 0xFF), ink.sheen);
        over(
            scale(white, lit * on_face * (1.0 - t).clamp(0.0, 1.0).powi(2)),
            px,
        )
    })
}

/// The tick nearest pixel (`x`, `y`): every minute, longer each hour, longest each quarter,
/// starting `from` device px out from the middle at `mid`.
fn tick(ink: &Inks, x: f64, y: f64, mid: f64, from: f64, k: f64) -> Px {
    let (cx, cy) = (x + 0.5 - mid, y + 0.5 - mid);
    let n = (cx.atan2(-cy) / TAU * 60.0).round().rem_euclid(60.0) as u32;
    let ((len, width), col) = match n {
        n if n % 15 == 0 => (QUARTER_TICK, ink.hour_tick),
        n if n % 5 == 0 => (HOUR_TICK, ink.hour_tick),
        _ => (MINUTE_TICK, ink.minute_tick),
    };
    let t = f64::from(n) / 60.0;
    let (ux, uy) = ((t * TAU).sin(), -(t * TAU).cos());
    let (a, b) = (from, from - len * k);
    let pts = [(mid + ux * a, mid + uy * a), (mid + ux * b, mid + uy * b)];
    scale(premul(col, 255), stroke(x, y, &pts, width * k))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn light(t: (u32, u32, u32), d: i32) -> Canvas {
        painter(Mode::Light)(t, SIDE, d)
    }

    fn rgb(c: Color) -> u32 {
        c.argb()
    }

    /// Whether `got` is `want` give or take the sheen over the face's top half.
    fn near(got: u32, want: Color) -> bool {
        let w = want.argb();
        got >> 24 == 0xFF
            && (0..3).all(|i| ((got >> (8 * i)) & 0xFF).abs_diff((w >> (8 * i)) & 0xFF) <= 16)
    }

    /// The pixel `r` design px out from the middle towards `turn` (clockwise from 12), at `d`.
    fn out(c: &Canvas, turn: f64, r: f64, d: i32) -> u32 {
        let mid = f64::from(SIDE * d) / 2.0;
        let k = f64::from(d);
        let (x, y) = (
            mid + (turn * TAU).sin() * r * k,
            mid - (turn * TAU).cos() * r * k,
        );
        c.get(x.floor() as i32, y.floor() as i32)
    }

    fn fnv(c: &Canvas) -> u64 {
        [c.width, c.height]
            .into_iter()
            .chain(c.pixels.iter().copied())
            .flat_map(u32::to_le_bytes)
            .fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
                (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
            })
    }

    #[test]
    fn the_hands_turn_with_the_time() {
        assert_eq!(turns((3, 0, 0)), (0.25, 0.0, 0.0));
        assert_eq!(turns((15, 30, 0)), (3.5 / 12.0, 0.5, 0.0));
        assert_eq!(turns((0, 0, 45)), (45.0 / 3600.0 / 12.0, 0.75 / 60.0, 0.75));
    }

    /// At 9:30:15 the hour hand points between 9 and 10, the minute hand down and the second
    /// hand right; each leaves the face plain the other way.
    #[test]
    fn each_hand_points_where_the_time_says() {
        let face = f64::from(SIDE) / 2.0 - MARGIN - BEZEL;
        for d in [1, 2] {
            let c = light((9, 30, 15), d);
            let (hour, minute, second) = turns((9, 30, 15));
            assert!(
                near(out(&c, hour, face * 0.4, d), LIGHT.hand),
                "hour at {d}"
            );
            assert!(
                near(out(&c, minute, face * 0.65, d), LIGHT.hand),
                "minute at {d}"
            );
            assert!(
                near(out(&c, second, face * 0.7, d), LIGHT.second),
                "second at {d}"
            );
            for (what, turn) in [("hour", hour), ("minute", minute), ("second", second)] {
                let back = out(&c, turn + 0.5, face * 0.45, d);
                assert!(
                    !near(back, LIGHT.hand) && !near(back, LIGHT.second),
                    "nothing opposite the {what} hand at {d}: {back:08X}"
                );
            }
            assert_eq!(out(&c, 0.0, 0.0, d), rgb(LIGHT.hand), "the pin at {d}");
        }
    }

    /// The face sits in a bezel with a clear outside; every 5th tick is an hour's, darker and
    /// longer than a minute's; at 2 the bezel's edge is soft.
    #[test]
    fn the_face_has_ticks_in_a_bezel() {
        let face = f64::from(SIDE) / 2.0 - MARGIN - BEZEL;
        let c = light((10, 9, 30), 1);
        assert_eq!(c.get(0, 0), 0, "a clear corner");
        let quarter = out(&c, 0.25, face - TICK_IN - 5.0, 1);
        assert_eq!(quarter, rgb(LIGHT.hour_tick), "the 3 o'clock tick");
        let minute = out(&c, 16.0 / 60.0, face - TICK_IN - 1.0, 1);
        assert_ne!(minute, rgb(LIGHT.hour_tick), "a minute tick is lighter");
        assert_ne!(
            out(&c, 16.0 / 60.0, face - TICK_IN - 5.0, 1),
            minute,
            "and shorter"
        );
        let two = light((10, 9, 30), 2);
        assert_eq!((two.width, two.height), (258, 258));
        let row: Vec<u32> = (0..20).map(|x| two.get(x, 129) >> 24).collect();
        assert!(
            row.iter().any(|&a| a > 0x20 && a < 0xE0),
            "a soft edge at 2: {row:x?}"
        );
    }

    #[test]
    fn dark_glass_draws_light_hands_on_a_dark_face() {
        let c = painter(Mode::Dark)((9, 30, 15), SIDE, 1);
        let (hour, _, _) = turns((9, 30, 15));
        let face = f64::from(SIDE) / 2.0 - MARGIN - BEZEL;
        assert!(near(out(&c, hour, face * 0.4, 1), DARK.hand));
        let plain = out(&c, 0.6, face * 0.3, 1);
        assert!(plain & 0xFF < 0x60, "a dark face: {plain:08X}");
    }

    /// The clock keeps its pixels at three times, light and dark, at scale 1 and 2.
    #[test]
    fn the_clock_pins_at_scale_1_and_2() {
        let mut got = Vec::new();
        for mode in [Mode::Light, Mode::Dark] {
            for t in [(10, 9, 30), (3, 0, 45), (21, 37, 5)] {
                for d in [1, 2] {
                    got.push(fnv(&painter(mode)(t, SIDE, d)));
                }
            }
        }
        assert_eq!(
            got,
            [
                0x33bd_ebda_f9a1_6150,
                0x7dbf_1051_8b61_d2c9,
                0xfbe7_1acf_91fe_8492,
                0x17ec_ebc7_b7aa_d7db,
                0x0f54_85d5_5b5c_5ae3,
                0xe6ec_1d10_323f_1843,
                0xc36e_94ba_02d3_6b47,
                0x0015_e70f_b818_12b8,
                0x5888_d237_a795_f759,
                0x1a55_8425_7524_b516,
                0x16ad_f2ed_3882_8fed,
                0xb18a_168b_9d27_bebc,
            ],
            "{got:#x?}"
        );
    }

    /// Writes target/glass-clock-preview.png: light and dark at 10:09:30, at 1x and 2x.
    #[test]
    #[ignore]
    fn render_glass_clock() {
        let mut out = Canvas::new(3 * SIDE as u32 * 2, 2 * SIDE as u32);
        for (i, mode) in [Mode::Light, Mode::Dark].into_iter().enumerate() {
            let y = i as i32 * SIDE;
            out.fill_rect(
                crate::raster::Rect::new(0, y, out.width as i32, SIDE),
                if mode == Mode::Light {
                    Color::rgb(0xEF, 0xEE, 0xEF)
                } else {
                    Color::rgb(0x2B, 0x2B, 0x2B)
                },
            );
            out.blit_over(0, y, &painter(mode)((10, 9, 30), SIDE, 1));
            let two = painter(mode)((10, 9, 30), SIDE, 2);
            let mut half = Canvas::new(two.width, SIDE as u32);
            half.blit_over(0, 0, &two);
            out.blit_over(SIDE + 10, y, &half);
        }
        crate::raster::write_preview(&out, "glass-clock-preview.png");
    }
}
