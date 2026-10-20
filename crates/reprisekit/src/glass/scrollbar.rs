//! Glass's scrollbar parts, drawn at the output scale. The bar is 17 px thick.
//! Every part is drawn upright; the lying parts are the upright ones turned
//! over the diagonal (the left arrow is the up arrow, the right the down one).
//!
//! The track is a light grey groove: a light line along its leading edge, a
//! darker one along its trailing edge, and a face that starts dark beside the
//! leading line and lightens over three pixels. A held page is the same
//! groove, darker. It has no hot look, and greyed it draws at rest.
//!
//! An arrow at rest is its dark glyph alone on the track. Under the pointer
//! it is a pale blue button, held a deeper one; while the pointer is on or
//! holds another part of the bar it is a grey button like the thumb (BarHot);
//! greyed, only the glyph shows, in grey. A button is a 15 px wide box with
//! corners rounded over 2.5 px, a 1 px outline and a 1 px inner line, lit
//! along the top and left and shaded along the right and bottom; its face is
//! split down the middle into a light and a deeper half, both shading down.
//! The glyph is a solid triangle, 7 px across and 4 tall, dark at its top
//! left and lighter toward its bottom right, with a light 0.6 px halo. The
//! down arrow's last row is the bar's trailing line.
//!
//! The thumb is the grey button 15x9 with a clear pixel at each end and clear
//! columns beside it; hot and held it turns pale blue, held with the deeper
//! outline. While the pointer is on another part of the bar its trailing
//! outline darkens. Thumbs keep 5 px at each end, clear pixel included, and
//! the body between stretches, so a 10 px thumb is its two ends. The grip,
//! 9x10 (10x9 lying), sits centred once the thumb is 20 px long and lights
//! with it: three ridges, each a dark line over a lighter one, fading left
//! to right.

use crate::raster::{Canvas, Insets};
use crate::shape::{Px, Stops, centre, draw, mix, over, polygon, premul, ringed, scale, shares};
use crate::style::{Layout, Part, PartImage, State};
use crate::theme::Color;

const ARROW_STATES: [State; 5] = [
    State::Normal,
    State::Hot,
    State::Pressed,
    State::Disabled,
    State::BarHot,
];

const LIT_STATES: [State; 4] = [State::Normal, State::Hot, State::Pressed, State::BarHot];

const TRACK_STATES: [State; 2] = [State::Normal, State::Pressed];

const BAR: i32 = 17;
const THUMB: i32 = 11;
const GRIP: (i32, i32) = (9, 10);
const BUTTON: (i32, i32) = (15, 16);
const RADIUS: f64 = 2.5;
/// Where a button's face splits into its light and deep halves, in box pixel middles.
const SPLIT: f64 = 6.5;
const LIGHT_FALL: f64 = 0.03;
const DEEP_FALL: f64 = 0.10;

const fn grey(v: u8) -> Color {
    Color::rgb(v, v, v)
}

/// The track's groove: leading line (column 0), trailing line (column 16) and a gradient face at column middles.
struct Groove {
    lead: Color,
    trail: Color,
    face: [(f64, Color); 4],
}

const GROOVE: Groove = Groove {
    lead: grey(0xF9),
    trail: grey(0xE8),
    face: [
        (1.0, grey(0xE1)),
        (4.0, grey(0xEE)),
        (13.0, grey(0xEF)),
        (15.0, grey(0xF3)),
    ],
};

const GROOVE_HELD: Groove = Groove {
    lead: grey(0xF2),
    trail: grey(0xD5),
    face: [
        (1.0, grey(0xC7)),
        (4.0, grey(0xDE)),
        (13.0, grey(0xE1)),
        (15.0, grey(0xE6)),
    ],
};

/// A button's colours: outline (`lead` top/left, `trail` right/bottom), inner
/// line (`lit`/`shade` likewise), and the face's light and deep halves, each
/// at the top, body and bottom.
struct Face {
    lead: Color,
    trail: Color,
    lit: Color,
    shade: Color,
    light: (Color, Color, Color),
    deep: (Color, Color, Color),
}

const GREY: Face = Face {
    lead: grey(0x96),
    trail: grey(0x94),
    lit: grey(0xF7),
    shade: Color::rgb(0xD0, 0xCF, 0xD2),
    light: (
        grey(0xF6),
        Color::rgb(0xF1, 0xF0, 0xF2),
        Color::rgb(0xEB, 0xEA, 0xEC),
    ),
    deep: (
        Color::rgb(0xE2, 0xE1, 0xE4),
        Color::rgb(0xD8, 0xD9, 0xDB),
        Color::rgb(0xD1, 0xD0, 0xD4),
    ),
};

/// The grey button with its trailing outline darker, for the thumb while the pointer is elsewhere on the bar.
const GREY_NEAR: Face = Face {
    trail: grey(0x80),
    ..GREY
};

const HOT: Face = Face {
    lead: Color::rgb(0x3B, 0x77, 0xAD),
    trail: Color::rgb(0x37, 0x6F, 0xA4),
    lit: Color::rgb(0xFC, 0xFE, 0xFF),
    shade: Color::rgb(0xDA, 0xDD, 0xDF),
    light: (
        Color::rgb(0xE3, 0xF2, 0xFC),
        Color::rgb(0xD6, 0xEC, 0xFB),
        Color::rgb(0xCD, 0xE2, 0xF2),
    ),
    deep: (
        Color::rgb(0xC4, 0xE4, 0xF9),
        Color::rgb(0xA7, 0xD6, 0xF6),
        Color::rgb(0x9A, 0xC5, 0xE2),
    ),
};

const HELD: Face = Face {
    lead: Color::rgb(0x18, 0x52, 0x87),
    trail: Color::rgb(0x19, 0x4E, 0x7F),
    lit: Color::rgb(0xE6, 0xF6, 0xFC),
    shade: Color::rgb(0x9B, 0xC2, 0xD8),
    light: (
        Color::rgb(0xC9, 0xE9, 0xF9),
        Color::rgb(0xAD, 0xDC, 0xF7),
        Color::rgb(0xA7, 0xD4, 0xEB),
    ),
    deep: (
        Color::rgb(0x9C, 0xD6, 0xF5),
        Color::rgb(0x6C, 0xC2, 0xF0),
        Color::rgb(0x62, 0xB2, 0xDC),
    ),
};

const HELD_THUMB: Face = Face {
    lead: HELD.lead,
    trail: HELD.trail,
    ..HOT
};

/// An arrow glyph's ink, `dark` at its top left through `mid` to `light` at its bottom right, and its halo.
struct Ink {
    dark: Color,
    mid: Color,
    light: Color,
    halo: (Color, u8),
}

const WHITE: Color = grey(0xFF);

const INK: Ink = Ink {
    dark: grey(0x1B),
    mid: grey(0x73),
    light: grey(0xC0),
    halo: (WHITE, 0x66),
};
const INK_HOT: Ink = Ink {
    dark: Color::rgb(0x0E, 0x2E, 0x43),
    mid: Color::rgb(0x2D, 0x89, 0xC7),
    light: Color::rgb(0x97, 0xC6, 0xE7),
    halo: (WHITE, 0x66),
};
const INK_HELD: Ink = Ink {
    dark: Color::rgb(0x0E, 0x27, 0x36),
    mid: Color::rgb(0x32, 0x74, 0x9F),
    light: Color::rgb(0x99, 0xBB, 0xD2),
    halo: (WHITE, 0x66),
};
const INK_NEAR: Ink = Ink {
    dark: grey(0x2E),
    mid: grey(0x8B),
    light: grey(0xCC),
    halo: (WHITE, 0x4D),
};
const INK_GREYED: Ink = Ink {
    dark: grey(0x6E),
    mid: grey(0x9C),
    light: grey(0xC4),
    halo: (grey(0), 0x06),
};

/// The glyph: 4 rows apex to base, 4 px either side of the middle at the base, and its halo's width.
const GLYPH_TALL: f64 = 4.0;
const GLYPH_HALF: f64 = 4.0;
const HALO: f64 = 0.6;
/// Where along the glyph's diagonal (0 top left, 1 bottom right) its ink starts to lighten.
const INK_FROM: f64 = 0.3;
/// Where it reaches its middle tone.
const INK_MID: f64 = 0.7;

const RIDGE: (Color, f64) = (Color::rgb(0x29, 0x27, 0x27), 1.0);
const RIDGE_LIT: (Color, f64) = (Color::rgb(0x10, 0x43, 0x67), 0.7);
/// A ridge's dark line alpha from its left end (0) to its right (1); its light line has [`RIDGE_LIGHT`] of that.
const RIDGE_FADE: [(f64, f64); 4] = [(0.0, 0.9), (0.25, 0.65), (0.65, 0.6), (1.0, 0.25)];
const RIDGE_LIGHT: f64 = 0.55;

fn solid(c: Color) -> Px {
    premul(c, 255)
}

fn groove(g: &Groove, x: f64) -> Px {
    if x < 0.5 {
        solid(g.lead)
    } else if x > f64::from(BAR) - 1.5 {
        solid(g.trail)
    } else {
        let face = g.face.map(|(t, c)| (t, solid(c)));
        Stops(&face).at(x)
    }
}

/// A `w` x `h` button's face at box pixel middle (`x`, `y`): each half shades
/// from its top colour to its body colour over two rows, holds it, and shades
/// to its bottom colour over the last two; the halves darken toward the right.
fn face(f: &Face, x: f64, y: f64, w: i32, h: i32) -> Px {
    let (top, bottom): (f64, f64) = (2.0, f64::from(h) - 3.0);
    let mid = f64::from(h - 1) / 2.0;
    let (from, to) = ((top + 2.0).min(mid - 0.5), (bottom - 2.0).max(mid + 0.5));
    let down = |(a, b, c): (Color, Color, Color)| {
        let (a, b, c) = (solid(a), solid(b), solid(c));
        Stops(&[(top, a), (from, b), (to, b), (bottom, c)]).at(y)
    };
    let black = solid(grey(0));
    if x < SPLIT {
        let k = ((x - 2.0) / (SPLIT - 2.0)).clamp(0.0, 1.0);
        mix(down(f.light), black, LIGHT_FALL * k)
    } else {
        let right = f64::from(w) - 3.0;
        let k = ((x - SPLIT - 0.5) / (right - SPLIT - 0.5)).clamp(0.0, 1.0);
        mix(down(f.deep), black, DEEP_FALL * k)
    }
}

/// A `w` x `h` button at `d`, row by row in device pixels, corners rounded over [`RADIUS`].
fn button(f: &Face, w: i32, h: i32, d: i32) -> Vec<Px> {
    let (rw, rh) = (f64::from(w), f64::from(h));
    ringed(w, h, RADIUS, 2, d, |x, y, ring| match ring {
        0 if x > rw - 1.5 || y > rh - 1.5 => solid(f.trail),
        0 => solid(f.lead),
        1 if x > rw - 2.5 || y > rh - 2.5 => solid(f.shade),
        1 => solid(f.lit),
        _ => face(f, x, y, w, h),
    })
}

/// A button laid at design (`bx`, `by`) in a part: its pixel at device (`x`, `y`), if it covers it.
struct Laid {
    px: Vec<Px>,
    at: (i32, i32),
    size: (i32, i32),
}

impl Laid {
    fn new(f: &Face, (bx, by): (i32, i32), (w, h): (i32, i32), d: i32) -> Laid {
        Laid {
            px: button(f, w, h, d),
            at: (bx * d, by * d),
            size: (w * d, h * d),
        }
    }

    fn get(&self, x: i32, y: i32) -> Option<Px> {
        let (x, y) = (x - self.at.0, y - self.at.1);
        let inside = (0..self.size.0).contains(&x) && (0..self.size.1).contains(&y);
        inside.then(|| self.px[(y * self.size.0 + x) as usize])
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Way {
    Up,
    Down,
}

fn arrow(way: Way, state: State, d: i32) -> Canvas {
    let (face, ink) = match state {
        State::Hot => (Some(&HOT), &INK_HOT),
        State::Pressed => (Some(&HELD), &INK_HELD),
        State::BarHot => (Some(&GREY), &INK_NEAR),
        State::Disabled => (None, &INK_GREYED),
        _ => (None, &INK),
    };
    // The button touches the thumb's end of the cell; glyphs sit mirrored about the cell's middle.
    let (at, apex, base) = match way {
        Way::Up => ((1, 1), 7.0, 7.0 + GLYPH_TALL),
        Way::Down => ((1, 0), 10.0, 10.0 - GLYPH_TALL),
    };
    let laid = face.map(|f| Laid::new(f, at, BUTTON, d));
    let k = f64::from(d);
    let cx = f64::from(BAR) / 2.0;
    let tri = |g: f64| {
        let s = if apex < base { 1.0 } else { -1.0 };
        let half = GLYPH_HALF + g + g * std::f64::consts::SQRT_2;
        [
            (cx * k, (apex - s * g * std::f64::consts::SQRT_2) * k),
            ((cx + half) * k, (base + s * g) * k),
            ((cx - half) * k, (base + s * g) * k),
        ]
    };
    let (glyph, halo) = (tri(0.0), tri(HALO));
    let stops = [
        (INK_FROM, solid(ink.dark)),
        (INK_MID, solid(ink.mid)),
        (1.0, solid(ink.light)),
    ];
    let wash = premul(ink.halo.0, ink.halo.1);
    let top = apex.min(base);
    let end = way == Way::Down;
    draw(BAR, BAR, d, |x, y| {
        let (dx, dy) = (x as i32, y as i32);
        let (mx, my) = (centre(dx, d), centre(dy, d));
        let track = if end && my > f64::from(BAR) - 1.5 {
            solid(GROOVE.trail)
        } else {
            groove(&GROOVE, mx)
        };
        let under = match laid.as_ref().and_then(|l| l.get(dx, dy)) {
            Some(b) => over(b, track),
            None => track,
        };
        let g = polygon(x, y, &glyph);
        let h = polygon(x, y, &halo).max(g);
        if h == 0.0 {
            return under;
        }
        let u = (mx + 0.5 - (cx - GLYPH_HALF)) / (2.0 * GLYPH_HALF);
        let v = (my + 0.5 - top) / GLYPH_TALL;
        let ink = Stops(&stops).at((u + v) / 2.0);
        shares(&[(ink, g), (over(wash, under), h - g), (under, 1.0 - h)])
    })
}

/// The upright thumb's 17x11 frame at `d`: the button 15x9 at (1, 1), clear round it.
fn thumb(f: &Face, d: i32) -> Canvas {
    let laid = Laid::new(f, (1, 1), (BUTTON.0, THUMB - 2), d);
    draw(BAR, THUMB, d, |x, y| {
        laid.get(x as i32, y as i32).unwrap_or(crate::shape::CLEAR)
    })
}

/// The upright grip's 9x10 frame at `d`: the thumb's middle row under three ridges in whole design pixels.
fn grip(f: &Face, ridge: (Color, f64), d: i32) -> Canvas {
    let fade = RIDGE_FADE.map(|(t, a)| (t, (a, [0.0; 3])));
    let ink = solid(ridge.0);
    let (w, h) = GRIP;
    let inset = f64::from(BAR - w) / 2.0 - 1.0;
    let mid = f64::from(THUMB - 2 - 1) / 2.0;
    draw(w, h, d, |x, y| {
        let mx = centre(x as i32, d);
        let row = y as i32 / d;
        let col = x as i32 / d;
        let under = face(f, mx + inset, mid, BUTTON.0, THUMB - 2);
        let line = match row % 3 {
            1 => 1.0,
            2 => RIDGE_LIGHT,
            _ => return under,
        };
        if col < 1 || col > w - 2 {
            return under;
        }
        let t = (mx - 1.0) / f64::from(w - 3);
        let a = Stops(&fade).at(t).0 * line * ridge.1;
        over(scale(ink, a), under)
    })
}

fn track(g: &Groove, d: i32) -> Canvas {
    draw(BAR, 1, d, |x, _| groove(g, centre(x as i32, d)))
}

fn turned(c: &Canvas) -> Canvas {
    let mut t = Canvas::new(c.height, c.width);
    for y in 0..c.height as i32 {
        for x in 0..c.width as i32 {
            t.put_raw(y, x, c.get(x, y));
        }
    }
    t
}

fn stack(frames: &[Canvas]) -> Canvas {
    let (w, h) = (frames[0].width, frames[0].height);
    let mut a = Canvas::new(w, h * frames.len() as u32);
    for (i, f) in frames.iter().enumerate() {
        a.blit_over(0, (i as u32 * h) as i32, f);
    }
    a
}

fn frames(cells: &[Canvas], lying: bool) -> Canvas {
    if lying {
        stack(&cells.iter().map(turned).collect::<Vec<_>>())
    } else {
        stack(cells)
    }
}

/// The ten scrollbar parts at `d`, the same in every glass colour.
pub fn parts(d: i32) -> [(Part, PartImage); 10] {
    // Row and column 3 of an arrow are its one seam, so a 16 px bar drops a plain line and keeps the glyph.
    let arrows = |way| -> Vec<Canvas> { ARROW_STATES.iter().map(|&s| arrow(way, s, d)).collect() };
    let (up, down) = (arrows(Way::Up), arrows(Way::Down));
    let arrow_part = |cells: &[Canvas], lying| {
        PartImage::new(frames(cells, lying), &ARROW_STATES, Layout::Vertical)
            .sizing(Insets::new(3, 3, 13, 13))
            .at_scale(d)
    };
    let thumb_faces = [&GREY, &HOT, &HELD_THUMB, &GREY_NEAR];
    let grip_looks = [
        (&GREY, RIDGE),
        (&HOT, RIDGE_LIT),
        (&HOT, RIDGE_LIT),
        (&GREY, RIDGE),
    ];
    let thumbs: Vec<Canvas> = thumb_faces.iter().map(|f| thumb(f, d)).collect();
    let grips: Vec<Canvas> = grip_looks.iter().map(|&(f, r)| grip(f, r, d)).collect();
    let tracks = [track(&GROOVE, d), track(&GROOVE_HELD, d)];
    let lit_part = |cells: &[Canvas], states: &[State], lying| {
        PartImage::new(frames(cells, lying), states, Layout::Vertical).at_scale(d)
    };
    let thumb_part = |lying| lit_part(&thumbs, &LIT_STATES, lying);
    let grip_part = |lying| lit_part(&grips, &LIT_STATES, lying);
    let track_part = |lying| lit_part(&tracks, &TRACK_STATES, lying);
    [
        (Part::ScrollArrowUp, arrow_part(&up, false)),
        (Part::ScrollArrowDown, arrow_part(&down, false)),
        (Part::ScrollArrowLeft, arrow_part(&up, true)),
        (Part::ScrollArrowRight, arrow_part(&down, true)),
        // 5 fixed rows at each end (clear one included) round one body row; the middle line is the seam for a thinner bar.
        (
            Part::ScrollThumbV,
            thumb_part(false)
                .sizing(Insets::new(8, 5, 8, 5))
                .content(Insets::new(0, 5, 0, 5)),
        ),
        (
            Part::ScrollThumbH,
            thumb_part(true)
                .sizing(Insets::new(5, 8, 5, 8))
                .content(Insets::new(5, 0, 5, 0)),
        ),
        (Part::ScrollGripV, grip_part(false)),
        (Part::ScrollGripH, grip_part(true)),
        (
            Part::ScrollTrackV,
            track_part(false).sizing(Insets::new(8, 0, 8, 0)),
        ),
        (
            Part::ScrollTrackH,
            track_part(true).sizing(Insets::new(0, 8, 0, 8)),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::{ARROW_STATES, LIT_STATES, TRACK_STATES, parts};
    use crate::interface::Interface;
    use crate::interfaces::theme_for;
    use crate::look::widgets::{draw_hscrollbar, draw_scrollbar};
    use crate::pointer::ScrollHit;
    use crate::pointer::{Interaction, Lit};
    use crate::raster::{Canvas, Insets, Rect};
    use crate::style::{Part, PartImage, State};

    fn part(p: Part, d: i32) -> PartImage {
        parts(d)
            .into_iter()
            .find(|(q, _)| *q == p)
            .map(|(_, img)| img)
            .expect("a scroll part")
    }

    fn art(p: Part, state: State, x: i32, y: i32) -> u32 {
        let img = part(p, 1);
        let f = img.frame(state);
        img.atlas.get(f.x + x, f.y + y)
    }

    fn frame(img: &PartImage, state: State) -> Canvas {
        let f = img.frame(state);
        let mut c = Canvas::new(f.width as u32, f.height as u32);
        for y in 0..f.height {
            for x in 0..f.width {
                c.put_raw(x, y, img.atlas.get(f.x + x, f.y + y));
            }
        }
        c
    }

    fn fnv(c: &Canvas) -> u64 {
        let mut words = vec![c.width, c.height];
        for y in 0..c.height as i32 {
            for x in 0..c.width as i32 {
                words.push(c.get(x, y));
            }
        }
        words
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
                (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
            })
    }

    const ALL: [Part; 10] = [
        Part::ScrollArrowUp,
        Part::ScrollArrowDown,
        Part::ScrollArrowLeft,
        Part::ScrollArrowRight,
        Part::ScrollThumbV,
        Part::ScrollThumbH,
        Part::ScrollGripV,
        Part::ScrollGripH,
        Part::ScrollTrackV,
        Part::ScrollTrackH,
    ];

    fn states(p: Part) -> &'static [State] {
        match p {
            Part::ScrollArrowUp
            | Part::ScrollArrowDown
            | Part::ScrollArrowLeft
            | Part::ScrollArrowRight => &ARROW_STATES,
            Part::ScrollTrackV | Part::ScrollTrackH => &TRACK_STATES,
            _ => &LIT_STATES,
        }
    }

    /// A 17x421 vertical bar with `visible` of `total` rows in view from `top`: at 31 of 60 the thumb is 200 px.
    fn vbar(top: usize, visible: usize, total: usize, lit: Lit<ScrollHit>) -> Canvas {
        let theme = theme_for(Interface::Glass);
        let mut c = Canvas::new(17, 421);
        draw_scrollbar(
            &mut c,
            &theme.paint(),
            Rect::new(0, 0, 17, 421),
            top,
            visible,
            total,
            lit,
        );
        c
    }

    /// A 763x17 horizontal bar, 387 of 729 columns in view from the left: a 387 px thumb.
    fn hbar(lit: Lit<ScrollHit>) -> Canvas {
        let theme = theme_for(Interface::Glass);
        let mut c = Canvas::new(763, 17);
        draw_hscrollbar(
            &mut c,
            &theme.paint(),
            Rect::new(0, 0, 763, 17),
            0,
            387,
            729,
            lit,
        );
        c
    }

    #[track_caller]
    fn shows(c: &Canvas, (x0, y0): (i32, i32), p: Part, state: State, what: &str) {
        let img = part(p, 1);
        let f = img.frame(state);
        for y in 0..f.height {
            for x in 0..f.width {
                assert_eq!(
                    c.get(x0 + x, y0 + y),
                    img.atlas.get(f.x + x, f.y + y),
                    "{what}: ({x}, {y}) is the {p:?} {state:?} frame's"
                );
            }
        }
    }

    #[test]
    fn scroll_parts_pin_at_scale_1_and_2() {
        let mut got = Vec::new();
        for d in [1, 2] {
            for p in ALL {
                let img = part(p, d);
                for &s in states(p) {
                    got.push(format!("{p:?} {s:?} {d}: {:#018x}", fnv(&frame(&img, s))));
                }
            }
        }
        let want: Vec<&str> = PINS
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect();
        assert_eq!(got, want, "\n{}", got.join("\n"));
    }

    const PINS: &str = "
    ScrollArrowUp Normal 1: 0x31487496e2806f93
    ScrollArrowUp Hot 1: 0xcded665a2d967e44
    ScrollArrowUp Pressed 1: 0x4efb5d649380b91d
    ScrollArrowUp Disabled 1: 0xb02b79b5f1891a87
    ScrollArrowUp BarHot 1: 0x0f7c2d66263287b1
    ScrollArrowDown Normal 1: 0x48a733c4fc8424da
    ScrollArrowDown Hot 1: 0xd5d9f9cf779f3aeb
    ScrollArrowDown Pressed 1: 0x7bb4ea99eef966cc
    ScrollArrowDown Disabled 1: 0x181e5b333ecbd51d
    ScrollArrowDown BarHot 1: 0x6052b5cc7669001b
    ScrollArrowLeft Normal 1: 0x414f5fd2fe0c574b
    ScrollArrowLeft Hot 1: 0x4b780cba8150e800
    ScrollArrowLeft Pressed 1: 0x322b0ba1288bf7cd
    ScrollArrowLeft Disabled 1: 0x7b8d518b2cbf3da7
    ScrollArrowLeft BarHot 1: 0x2a8fa92a0e93e671
    ScrollArrowRight Normal 1: 0xa7a6ffde8a7fc302
    ScrollArrowRight Hot 1: 0x2c5f9f3adc05c353
    ScrollArrowRight Pressed 1: 0xc616eae4632459b0
    ScrollArrowRight Disabled 1: 0x15f6d51659619ce5
    ScrollArrowRight BarHot 1: 0x3e9e6bf7faa9303f
    ScrollThumbV Normal 1: 0x503d5a239c079b7a
    ScrollThumbV Hot 1: 0x738ab2cfa5435dae
    ScrollThumbV Pressed 1: 0xb74c9ea0ea48b89d
    ScrollThumbV BarHot 1: 0xacdde03a27a1b1c7
    ScrollThumbH Normal 1: 0x4fbdd8282590309e
    ScrollThumbH Hot 1: 0x19dc3d999f2c0892
    ScrollThumbH Pressed 1: 0x043cb2d6d148b081
    ScrollThumbH BarHot 1: 0x49e12f1de4b6ad2b
    ScrollGripV Normal 1: 0xdc675a90a841f00f
    ScrollGripV Hot 1: 0x62848463278fd926
    ScrollGripV Pressed 1: 0x62848463278fd926
    ScrollGripV BarHot 1: 0xdc675a90a841f00f
    ScrollGripH Normal 1: 0x5aa26d98c4ae44e7
    ScrollGripH Hot 1: 0x95ab976d7d8173c6
    ScrollGripH Pressed 1: 0x95ab976d7d8173c6
    ScrollGripH BarHot 1: 0x5aa26d98c4ae44e7
    ScrollTrackV Normal 1: 0x9ce7f7bbf63b6864
    ScrollTrackV Pressed 1: 0x42c3161a9b1424a4
    ScrollTrackH Normal 1: 0x20908199cea37f64
    ScrollTrackH Pressed 1: 0xc66b9ff8737c3ba4
    ScrollArrowUp Normal 2: 0xd6c1a54d6e86e234
    ScrollArrowUp Hot 2: 0x18a2cf8b79d9ce9c
    ScrollArrowUp Pressed 2: 0x852e6aa7fe67e810
    ScrollArrowUp Disabled 2: 0x3c60ff1de2b72308
    ScrollArrowUp BarHot 2: 0xf2b57d5fff18abd5
    ScrollArrowDown Normal 2: 0x33bb0eae409ff4bd
    ScrollArrowDown Hot 2: 0x9a809801c8241a29
    ScrollArrowDown Pressed 2: 0x335b5561feda5a56
    ScrollArrowDown Disabled 2: 0xf730194fd5d7823a
    ScrollArrowDown BarHot 2: 0xf7d3666ee7818248
    ScrollArrowLeft Normal 2: 0xe7490e06e50cd0c4
    ScrollArrowLeft Hot 2: 0xff465de5f46c0dbc
    ScrollArrowLeft Pressed 2: 0xa1ffdf03f81adbb0
    ScrollArrowLeft Disabled 2: 0x0fda6c43bc375bc8
    ScrollArrowLeft BarHot 2: 0xf559d45d00055235
    ScrollArrowRight Normal 2: 0xba446d90a9e843fd
    ScrollArrowRight Hot 2: 0x378bbf8bfe645a91
    ScrollArrowRight Pressed 2: 0xef5563da974620ce
    ScrollArrowRight Disabled 2: 0xc73a04ac49ba2c42
    ScrollArrowRight BarHot 2: 0x39fcc9fd90c58f40
    ScrollThumbV Normal 2: 0xae9bf746d47b23a6
    ScrollThumbV Hot 2: 0xd6a1d1d08f9e1866
    ScrollThumbV Pressed 2: 0x077bc79f6c796d59
    ScrollThumbV BarHot 2: 0xba237caa512d7ceb
    ScrollThumbH Normal 2: 0x0fde641db84ff3b6
    ScrollThumbH Hot 2: 0xf142a8e8ca0bc6e2
    ScrollThumbH Pressed 2: 0x6466130549b3bb75
    ScrollThumbH BarHot 2: 0xb0f9b2ec6122d0ff
    ScrollGripV Normal 2: 0x9bbbae1043751477
    ScrollGripV Hot 2: 0x18574ec673d977df
    ScrollGripV Pressed 2: 0x18574ec673d977df
    ScrollGripV BarHot 2: 0x9bbbae1043751477
    ScrollGripH Normal 2: 0x3eb1e56220b0ba13
    ScrollGripH Hot 2: 0xfe2ae8ee0438521b
    ScrollGripH Pressed 2: 0xfe2ae8ee0438521b
    ScrollGripH BarHot 2: 0x3eb1e56220b0ba13
    ScrollTrackV Normal 2: 0xbe3eae495b965d25
    ScrollTrackV Pressed 2: 0x9fbf92cfeb122be5
    ScrollTrackH Normal 2: 0xe0211d68391d3ac5
    ScrollTrackH Pressed 2: 0xa34af9c4b6fd6b75
";

    /// At 2 every frame is twice its scale-1 size; nine-grid insets and content margins stay in design pixels.
    #[test]
    fn a_scroll_part_at_2_is_twice_its_size_with_the_same_insets() {
        for p in ALL {
            let (one, two) = (part(p, 1), part(p, 2));
            let (a, b) = (one.frame(State::Normal), two.frame(State::Normal));
            assert_eq!((b.width, b.height), (2 * a.width, 2 * a.height), "{p:?}");
            assert_eq!((one.scale, two.scale), (1, 2), "{p:?}");
            assert_eq!(one.sizing, two.sizing, "{p:?} sizing");
            assert_eq!(one.content, two.content, "{p:?} content");
        }
        let thumb = part(Part::ScrollThumbV, 1);
        assert_eq!(thumb.sizing, Insets::new(8, 5, 8, 5));
        assert_eq!(
            part(Part::ScrollArrowUp, 1).sizing,
            Insets::new(3, 3, 13, 13)
        );
    }

    /// Every state draws its own frame except the grip: held is its hot frame, bar-lit is its rest frame.
    #[test]
    fn every_state_of_a_scroll_part_draws_its_own_frame() {
        for d in [1, 2] {
            for p in ALL {
                let img = part(p, d);
                let st = states(p);
                for (i, &a) in st.iter().enumerate() {
                    for &b in &st[i + 1..] {
                        let alias = matches!(p, Part::ScrollGripV | Part::ScrollGripH)
                            && matches!(
                                (a, b),
                                (State::Hot, State::Pressed) | (State::Normal, State::BarHot)
                            );
                        let same = frame(&img, a) == frame(&img, b);
                        assert_eq!(same, alias, "{p:?} at {d}: {a:?} against {b:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn the_lying_parts_are_the_upright_ones_turned() {
        for d in [1, 2] {
            for (up, lying) in [
                (Part::ScrollArrowUp, Part::ScrollArrowLeft),
                (Part::ScrollArrowDown, Part::ScrollArrowRight),
                (Part::ScrollThumbV, Part::ScrollThumbH),
                (Part::ScrollGripV, Part::ScrollGripH),
                (Part::ScrollTrackV, Part::ScrollTrackH),
            ] {
                let (u, l) = (part(up, d), part(lying, d));
                for &s in states(up) {
                    let (a, b) = (frame(&u, s), frame(&l, s));
                    assert_eq!((a.width, a.height), (b.height, b.width));
                    for y in 0..a.height as i32 {
                        for x in 0..a.width as i32 {
                            assert_eq!(a.get(x, y), b.get(y, x), "{lying:?} {s:?} at {d}");
                        }
                    }
                }
            }
        }
    }

    /// The thumb's ends and side columns are clear, its corners curve with partial alpha at 2, and a corner on the track is no darker than the outline.
    #[test]
    fn the_thumb_has_clear_ends_and_smooth_corners() {
        for d in [1, 2] {
            let img = part(Part::ScrollThumbV, d);
            for &s in &LIT_STATES {
                let f = frame(&img, s);
                for x in 0..17 * d {
                    for y in 0..d {
                        let b = 11 * d - 1 - y;
                        assert_eq!(f.get(x, y) >> 24, 0, "{s:?} at {d}: the top row is clear");
                        assert_eq!(f.get(x, b) >> 24, 0, "{s:?} at {d}: the bottom row");
                    }
                }
                for y in 0..11 * d {
                    assert_eq!(f.get(0, y) >> 24, 0, "{s:?}: the leading column is clear");
                    assert_eq!(f.get(17 * d - 1, y) >> 24, 0, "the trailing column");
                }
                let corner = f.get(d, d) >> 24;
                assert!(
                    corner < 0xFF,
                    "{s:?} at {d}: the corner is cut, {corner:#x}"
                );
                if d == 2 {
                    let partial = (0..3 * d)
                        .flat_map(|y| (0..3 * d).map(move |x| (x, y)))
                        .filter(|&(x, y)| (1..0xFF).contains(&(f.get(x, y) >> 24)))
                        .count();
                    assert!(
                        partial > 2,
                        "{s:?}: the corner curve has partial alpha at 2"
                    );
                }
            }
        }
        let rest = vbar(0, 31, 60, None);
        let outline = art(Part::ScrollThumbV, State::Normal, 1, 5) & 0xFF;
        for (x, y) in [(1, 18), (2, 18), (1, 19), (15, 18), (15, 17 + 198)] {
            assert!(
                rest.get(x, y) & 0xFF >= outline,
                "({x}, {y}) on the thumb's corner is no darker than its outline"
            );
        }
    }

    /// Each arrow glyph has solid dark ink at 1 and blended edges at 2, in any state.
    #[test]
    fn each_arrow_glyph_is_solid_at_1_and_smooth_at_2() {
        for p in [Part::ScrollArrowUp, Part::ScrollArrowDown] {
            for &s in &ARROW_STATES {
                let one = frame(&part(p, 1), s);
                let darkest = (0..17)
                    .flat_map(|y| (0..17).map(move |x| (x, y)))
                    .map(|(x, y)| one.get(x, y) & 0xFF)
                    .min()
                    .unwrap();
                assert!(darkest < 0x90, "{p:?} {s:?}: solid dark ink, {darkest:#x}");
                let two = frame(&part(p, 2), s);
                let tones: std::collections::BTreeSet<u32> =
                    (12..24).map(|y| two.get(11, y) & 0xFF).collect();
                assert!(tones.len() > 3, "{p:?} {s:?}: the glyph's edge blends at 2");
            }
        }
    }

    /// The up arrow at rest, hot, held, bar-lit by the thumb, and greyed: each its own frame.
    #[test]
    fn the_up_arrow_draws_each_state_from_its_own_frame() {
        let hot = |hit, how| Some((hit, how));
        for (c, state) in [
            (vbar(0, 31, 60, None), State::Normal),
            (
                vbar(0, 31, 60, hot(ScrollHit::Back, Interaction::Hot)),
                State::Hot,
            ),
            (
                vbar(0, 31, 60, hot(ScrollHit::Back, Interaction::Pressed)),
                State::Pressed,
            ),
            (
                vbar(0, 31, 60, hot(ScrollHit::Thumb, Interaction::Hot)),
                State::BarHot,
            ),
            (vbar(0, 31, 31, None), State::Disabled),
        ] {
            shows(&c, (0, 0), Part::ScrollArrowUp, state, "the up arrow");
        }
    }

    /// Wherever the pointer rests on a live bar or whatever it holds there, each arrow it is not on draws grey.
    #[test]
    fn a_lit_bar_draws_its_other_arrows_as_grey_buttons() {
        for (hit, how, up, down) in [
            (ScrollHit::Back, Interaction::Hot, false, true),
            (ScrollHit::Forward, Interaction::Pressed, true, false),
            (ScrollHit::Thumb, Interaction::Pressed, true, true),
            (ScrollHit::PageForward, Interaction::Hot, true, true),
            (ScrollHit::PageBack, Interaction::Pressed, true, true),
        ] {
            let c = vbar(0, 31, 60, Some((hit, how)));
            let what = format!("{hit:?} {how:?}");
            if up {
                shows(&c, (0, 0), Part::ScrollArrowUp, State::BarHot, &what);
            }
            if down {
                shows(&c, (0, 404), Part::ScrollArrowDown, State::BarHot, &what);
            }
        }
        let rest = vbar(0, 31, 60, None);
        shows(
            &rest,
            (0, 404),
            Part::ScrollArrowDown,
            State::Normal,
            "at rest",
        );
        let greyed = vbar(0, 31, 31, Some((ScrollHit::Back, Interaction::Hot)));
        assert!(
            greyed == vbar(0, 31, 31, None),
            "a greyed bar ignores the pointer"
        );
    }

    /// The upright thumb: track above and below it, its top end is its frame's, the grip centred on the 200 px thumb and lit with it.
    #[test]
    fn the_upright_thumb_has_clear_ends_and_a_centred_grip() {
        let track = art(Part::ScrollTrackV, State::Normal, 8, 0);
        for (lit, state) in [
            (None, State::Normal),
            (Some((ScrollHit::Thumb, Interaction::Hot)), State::Hot),
            (
                Some((ScrollHit::Thumb, Interaction::Pressed)),
                State::Pressed,
            ),
            (Some((ScrollHit::Back, Interaction::Hot)), State::BarHot),
        ] {
            let c = vbar(0, 31, 60, lit);
            let what = format!("{state:?}");
            assert_eq!(c.get(8, 17), track, "{what}: the track shows above");
            assert_eq!(c.get(8, 17 + 199), track, "{what}: the track shows below");
            for y in 1..5 {
                for x in 1..16 {
                    let want = art(Part::ScrollThumbV, state, x, y);
                    if want >> 24 == 0xFF {
                        assert_eq!(c.get(x, 17 + y), want, "{what}: the top end ({x}, {y})");
                    }
                }
            }
            shows(&c, (4, 17 + 5 + 90), Part::ScrollGripV, state, &what);
        }
    }

    /// The lying thumb at rest and hot, its trailing outline darkening while the pointer rests on an arrow.
    #[test]
    fn the_lying_thumb_darkens_its_trailing_line_while_the_bar_is_lit() {
        let rest = hbar(None);
        let near = hbar(Some((ScrollHit::Back, Interaction::Hot)));
        let hot = hbar(Some((ScrollHit::Thumb, Interaction::Hot)));
        let (x, y) = (17 + 193, 15);
        assert_eq!(rest.get(x, y), art(Part::ScrollThumbH, State::Normal, 5, y));
        assert_eq!(near.get(x, y), art(Part::ScrollThumbH, State::BarHot, 5, y));
        assert!(
            near.get(x, y) & 0xFF < rest.get(x, y) & 0xFF,
            "the trailing outline darkens"
        );
        assert_eq!(hot.get(x, y), art(Part::ScrollThumbH, State::Hot, 5, y));
        shows(
            &near,
            (0, 0),
            Part::ScrollArrowLeft,
            State::Hot,
            "the left arrow",
        );
        shows(
            &near,
            (746, 0),
            Part::ScrollArrowRight,
            State::BarHot,
            "the grey right arrow",
        );
        shows(
            &hot,
            (17 + 5 + 183, 4),
            Part::ScrollGripH,
            State::Hot,
            "the grip",
        );
    }

    /// A 10 px thumb is its two 5 px ends: every opaque pixel is the frame's own end pixel, so the clear line at each end shows the track.
    #[test]
    fn a_10_px_thumb_keeps_both_ends_and_their_clear_lines() {
        // Thumb line `i` of 10: the first end's 5, then the last end's 5, skipping the body.
        let src = |i: i32| if i < 5 { i } else { i + 1 };
        let up = vbar(0, 1, 1000, None);
        for y in 0..10 {
            for x in 0..17 {
                let want = art(Part::ScrollThumbV, State::Normal, x, src(y));
                if want >> 24 == 0xFF {
                    assert_eq!(up.get(x, 17 + y), want, "upright ({x}, {y})");
                }
            }
        }
        let track = art(Part::ScrollTrackV, State::Normal, 8, 0);
        assert_eq!(up.get(8, 17), track, "the clear row above");
        assert_eq!(up.get(8, 26), track, "the clear row below");
        let theme = theme_for(Interface::Glass);
        let mut c = Canvas::new(763, 17);
        draw_hscrollbar(
            &mut c,
            &theme.paint(),
            Rect::new(0, 0, 763, 17),
            0,
            1,
            1000,
            None,
        );
        for x in 0..10 {
            for y in 0..17 {
                let want = art(Part::ScrollThumbH, State::Normal, src(x), y);
                if want >> 24 == 0xFF {
                    assert_eq!(c.get(17 + x, y), want, "lying ({x}, {y})");
                }
            }
        }
        let track = art(Part::ScrollTrackH, State::Normal, 0, 8);
        assert_eq!(c.get(17, 8), track, "the clear column left");
        assert_eq!(c.get(17 + 9, 8), track, "the clear column right");
    }

    /// A held page draws the track pressed from the thumb to that end; the other page stays at rest.
    #[test]
    fn a_held_page_darkens_the_track_up_to_the_thumb() {
        let rest = vbar(50, 31, 361, None);
        let thumb = (17..404)
            .find(|&y| rest.get(8, y) != rest.get(8, 17))
            .expect("the thumb");
        let held = vbar(
            50,
            31,
            361,
            Some((ScrollHit::PageForward, Interaction::Pressed)),
        );
        for y in [thumb + 40, 300, 403] {
            for x in 0..17 {
                let want = art(Part::ScrollTrackV, State::Pressed, x, 0);
                assert_eq!(held.get(x, y), want, "the held page ({x}, {y})");
            }
        }
        let at_rest = art(Part::ScrollTrackV, State::Normal, 8, 0);
        assert!(at_rest & 0xFF > art(Part::ScrollTrackV, State::Pressed, 8, 0) & 0xFF);
        assert_eq!(held.get(8, 17), at_rest, "the page above stays at rest");
        assert_eq!(rest.get(8, 300), at_rest, "the track at rest");
        let hot = vbar(
            50,
            31,
            361,
            Some((ScrollHit::PageForward, Interaction::Hot)),
        );
        assert_eq!(hot.get(8, 300), rest.get(8, 300), "no hot track");
    }

    /// A bar at scale 2 draws the parts painted at 2: arrows, thumb end and grip are those frames pixel for pixel.
    #[test]
    fn a_glass_bar_at_scale_2_draws_the_parts_painted_at_2() {
        let theme = theme_for(Interface::Glass).scaled(2);
        let mut c = Canvas::new(34, 842);
        draw_scrollbar(
            &mut c,
            &theme.paint(),
            Rect::new(0, 0, 17, 421).scaled(2),
            0,
            31,
            60,
            Some((ScrollHit::Thumb, Interaction::Hot)),
        );
        let check = |p: Part, state: State, (x0, y0): (i32, i32), rows: i32| {
            let img = part(p, 2);
            let f = img.frame(state);
            for y in 0..rows.min(f.height) {
                for x in 0..f.width {
                    let want = img.atlas.get(f.x + x, f.y + y);
                    if want >> 24 == 0xFF {
                        assert_eq!(c.get(x0 + x, y0 + y), want, "{p:?} ({x}, {y})");
                    }
                }
            }
        };
        check(Part::ScrollArrowUp, State::BarHot, (0, 0), 34);
        check(Part::ScrollArrowDown, State::BarHot, (0, 808), 34);
        check(Part::ScrollThumbV, State::Hot, (0, 34), 10);
        check(Part::ScrollGripV, State::Hot, (8, 34 + 10 + 180), 20);
        let mut h = Canvas::new(1526, 34);
        draw_hscrollbar(
            &mut h,
            &theme.paint(),
            Rect::new(0, 0, 763, 17).scaled(2),
            0,
            387,
            729,
            None,
        );
        let img = part(Part::ScrollArrowLeft, 2);
        let f = img.frame(State::Normal);
        for y in 0..34 {
            for x in 0..34 {
                let want = img.atlas.get(f.x + x, f.y + y);
                assert_eq!(h.get(x, y), want, "the left arrow ({x}, {y})");
            }
        }
    }
}
