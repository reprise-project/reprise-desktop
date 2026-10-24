//! Glass caption buttons on the Rounded frame, drawn from the glass settings: the minimize/maximize/close strip, a dialog's close box, its ? box and a windowed dialog's minimize box.

use super::frame::{BAND, Under};
use super::glyph::{self, Glyph};
use super::panel::Glass;
use super::tint::{Recolour, Surface};
use super::{SWATCHES, Settings};
use crate::raster::Canvas;
use crate::shape::{self, Px, Stops, centre, over, pack, rounded, smooth};
use crate::style::{Layout, Part, PartImage, State};
use crate::theme::Color;

struct Strip {
    width: i32,
    cells: &'static [(i32, i32, Glyph)],
    dividers: &'static [i32],
}

const GROUP: Strip = Strip {
    width: 105,
    cells: &[
        (2, 28, Glyph::Minimize),
        (30, 55, Glyph::Maximize),
        (57, 102, Glyph::Close),
    ],
    dividers: &[29, 56],
};

const MAX_GROUP: Strip = Strip {
    cells: &[
        (2, 28, Glyph::Minimize),
        (30, 55, Glyph::Restore),
        (57, 102, Glyph::Close),
    ],
    ..GROUP
};

const DIALOG_CLOSE: Strip = Strip {
    width: 49,
    cells: &[(2, 46, Glyph::Close)],
    dividers: &[],
};

const HELP: Strip = Strip {
    width: 78,
    cells: &[(2, 28, Glyph::Help), (30, 75, Glyph::Close)],
    dividers: &[29],
};

const MIN_CLOSE: Strip = Strip {
    cells: &[(2, 28, Glyph::Minimize), (30, 75, Glyph::Close)],
    ..HELP
};

/// Restored strip rows: it hangs from the caption's light line (row 0); bodies on rows 1 to 16, light line 17, outline 18, halo 19. A maximized strip omits row 0.
const ROWS: i32 = 20;

/// The body's rows, 1 to 16 ([`BODY_ROWS`]): what a grown strip stretches.
const BODY: i32 = 16;

const RIGHT: i32 = 6;

/// Rows the body's two bands are set at: upper 1 to 8, lower 9 to 16.
const BODY_ROWS: [i32; 5] = [1, 3, 8, 9, 16];

/// Rows a lit cell's fill is set at: the two bands', then the light line's, outline's and halo's.
const FILL_ROWS: [i32; 9] = [1, 3, 8, 9, 13, 16, 17, 18, 19];

/// Radius of the halo's bottom corners; each inner ring is a pixel tighter.
const RADIUS: f64 = 6.07;

/// Caption sheen carried by each body row at [`BODY_ROWS`]: focused window, then another window's (flat).
const SHEEN: [[f64; 5]; 2] = [
    [1.911, 1.91, 1.445, 0.692, 1.1],
    [1.037, 1.026, 1.022, 1.025, 1.009],
];

/// Alpha gained (levels) per body row over that, same order.
const DEEPEN: [[f64; 5]; 2] = [[1.874, 1.409, 15.6, 36.8, 21.9], [0.0; 5]];

/// Layers over the glass (alpha and premultiplied colour in levels, focused window then another): halo over the caption band, outline over the nearer cell's body, light line, dividers over the body right of them.
const HALO: [Px; 2] = [(172.9, [172.9, 172.9, 172.5]), (95.3, [95.3; 3])];
const OUTLINE: [Px; 2] = [(162.9, [12.0, 11.6, 11.8]), (58.9, [0.0; 3])];
const LIGHT: [Px; 2] = [(107.5, [107.5; 3]), (59.3, [59.3, 59.3, 59.0])];
const DIVIDER: [Px; 2] = [(102.8, [2.74, 0.77, 0.34]), (26.2, [0.0; 3])];

/// Close's red per body row; on another window close rests bare.
const CLOSE_RED: [Px; 5] = [
    (140.0, [135.0, 71.2, 60.5]),
    (141.0, [129.0, 62.7, 51.8]),
    (171.5, [145.7, 59.1, 44.6]),
    (208.7, [150.9, 24.7, 5.49]),
    (172.8, [147.8, 62.8, 39.2]),
];

struct Shine {
    /// Fill at the cell's middle column, then at its edges; the edge's share grows with distance from the middle to the power `power`.
    fill: [Px; 9],
    edge: [Px; 9],
    power: f64,
    /// The spot at the foot's middle: peak alpha, reach across and down in px, row, and end sharpness (2 gaussian, more flatter).
    spot: (f64, f64, f64, f64, f64),
    spot_colour: [f64; 3],
    /// Glow alpha in the column next to the cell and the share kept each column further.
    bleed: (f64, f64),
    bleed_colour: [f64; 3],
}

/// The bleed eases out from 6 px past its cell to none at 12.
const BLEED_ENDS: (i32, i32) = (6, 12);

/// Glow for minimize, maximize and ?, in Sky: under the pointer, then held.
const LIT: [Shine; 2] = [
    Shine {
        fill: [
            (243.1, [176.1, 209.5, 232.5]),
            (242.5, [175.4, 210.7, 234.5]),
            (232.5, [129.1, 179.8, 210.0]),
            (229.6, [44.7, 110.2, 171.9]),
            (241.1, [21.6, 197.0, 213.4]),
            (235.1, [55.3, 148.8, 191.4]),
            (236.7, [108.8, 192.5, 217.9]),
            (248.4, [48.9, 111.3, 133.9]),
            (247.1, [92.5, 201.9, 246.2]),
        ],
        edge: [
            (248.0, [161.7, 200.3, 224.4]),
            (248.1, [162.3, 200.2, 223.3]),
            (242.0, [115.0, 162.4, 197.3]),
            (224.8, [4.288, 89.7, 141.9]),
            (226.1, [15.4, 75.3, 139.3]),
            (238.2, [54.5, 145.8, 190.1]),
            (237.1, [108.7, 192.2, 217.9]),
            (247.4, [50.2, 111.3, 134.9]),
            (195.4, [86.5, 155.9, 185.6]),
        ],
        power: 1.192,
        spot: (255.0, 8.204, 1.007, 16.8, 3.702),
        spot_colour: [16.0, 255.0, 250.9],
        bleed: (134.4, 0.806),
        bleed_colour: [59.5, 207.2, 255.0],
    },
    Shine {
        fill: [
            (249.5, [159.9, 185.5, 202.2]),
            (246.1, [157.6, 179.9, 197.3]),
            (251.6, [98.4, 151.5, 173.6]),
            (252.1, [24.5, 109.2, 132.5]),
            (235.8, [12.7, 147.2, 152.0]),
            (254.8, [45.0, 20.6, 72.7]),
            (213.9, [45.8, 138.9, 169.4]),
            (227.3, [58.5, 73.9, 111.0]),
            (232.4, [106.4, 178.2, 232.4]),
        ],
        edge: [
            (255.0, [141.8, 171.7, 191.0]),
            (255.0, [137.0, 171.2, 191.1]),
            (255.0, [72.8, 112.8, 146.2]),
            (255.0, [4.478, 22.2, 55.0]),
            (255.0, [4.673, 18.3, 60.3]),
            (255.0, [44.1, 20.7, 73.0]),
            (214.1, [46.6, 138.4, 169.0]),
            (226.9, [58.2, 75.1, 111.5]),
            (216.4, [101.6, 170.7, 200.6]),
        ],
        power: 1.296,
        spot: (214.0, 9.405, 2.588, 16.0, 2.518),
        spot_colour: [0.0, 255.0, 241.4],
        bleed: (124.5, 0.811),
        bleed_colour: [52.5, 207.4, 255.0],
    },
];

const CLOSE_LIT: [Shine; 2] = [
    Shine {
        fill: [
            (252.0, [247.9, 154.5, 136.3]),
            (246.5, [243.2, 151.1, 133.1]),
            (243.8, [230.3, 102.0, 77.2]),
            (253.2, [213.1, 31.6, 3.21]),
            (218.3, [191.3, 49.3, 1.175]),
            (235.7, [208.8, 125.1, 40.1]),
            (218.4, [173.4, 67.3, 65.8]),
            (250.7, [133.0, 81.7, 41.2]),
            (202.4, [200.7, 105.2, 80.5]),
        ],
        edge: [
            (250.7, [224.8, 149.4, 138.8]),
            (243.9, [220.4, 145.4, 135.3]),
            (242.3, [196.4, 95.0, 80.1]),
            (251.5, [154.8, 27.8, 10.1]),
            (215.9, [136.2, 27.8, 10.4]),
            (242.6, [204.4, 115.9, 41.2]),
            (217.6, [173.9, 68.1, 66.2]),
            (252.2, [130.9, 78.5, 42.4]),
            (168.3, [155.8, 92.6, 77.9]),
        ],
        power: 1.796,
        spot: (251.7, 19.6, 0.514, 17.0, 6.632),
        spot_colour: [244.9, 223.0, 119.1],
        bleed: (67.7, 0.837),
        bleed_colour: [255.0, 21.4, 0.0],
    },
    Shine {
        fill: [
            (232.4, [191.7, 156.1, 134.5]),
            (196.9, [163.4, 135.9, 114.5]),
            (220.5, [163.3, 103.4, 77.8]),
            (255.0, [144.2, 20.2, 3.972]),
            (238.2, [127.9, 36.4, 0.0]),
            (237.6, [51.8, 28.4, 24.2]),
            (172.0, [77.4, 51.5, 7.751]),
            (151.7, [44.7, 15.1, 49.2]),
            (177.8, [177.3, 63.0, 92.4]),
        ],
        edge: [
            (253.0, [167.4, 136.9, 115.1]),
            (212.2, [131.5, 109.3, 88.7]),
            (251.5, [130.4, 78.9, 55.4]),
            (255.0, [68.3, 10.5, 4.03]),
            (255.0, [54.7, 0.0, 1.917]),
            (239.3, [50.8, 27.5, 22.4]),
            (172.5, [76.7, 50.9, 8.381]),
            (151.1, [44.5, 16.8, 50.4]),
            (179.6, [156.3, 97.3, 111.1]),
        ],
        power: 1.853,
        spot: (204.9, 15.7, 1.716, 16.9, 1.018),
        spot_colour: [255.0, 212.9, 2.687],
        bleed: (74.8, 0.849),
        bleed_colour: [255.0, 39.0, 0.0],
    },
];

/// Glyph outline: the glyph grown by 1 px, opaque where it covers a pixel whole.
const GLYPH_EDGE: [f64; 3] = [83.0, 86.0, 102.0];

/// Glyph white from the top row (0) to the bottom (1): white over the top third, greying below.
const GLYPH_FILL: [(f64, f64); 7] = [
    (0.0, 255.0),
    (0.3, 255.0),
    (0.43, 240.0),
    (0.57, 232.0),
    (0.71, 227.0),
    (0.86, 222.0),
    (1.0, 218.0),
];

fn level(v: f64) -> Px {
    (v, [0.0; 3])
}

/// Row whose middle lies at `t` takes its band and fill, with `t` kept inside the row's own band (1 to 8 or 9 to 16) so the step between bands holds at any scale.
fn body_row(r: i32, t: f64) -> f64 {
    match r {
        1..=8 => t.clamp(1.0, 8.0),
        9..=16 => t.clamp(9.0, 16.0),
        _ => f64::from(r),
    }
}

fn banded(vals: &[Px; 5], r: i32, t: f64) -> Px {
    let r = r.min(16);
    let t = body_row(r, t);
    let stops: [(f64, Px); 5] = std::array::from_fn(|i| (f64::from(BODY_ROWS[i]), vals[i]));
    if r < 9 {
        Stops(&stops[..3]).at(t)
    } else {
        Stops(&stops[3..]).at(t)
    }
}

fn lay(alpha: f64, rgb: [f64; 3]) -> Px {
    (alpha, rgb.map(|v| v * alpha / 255.0))
}

/// Takes the glow colours (and the Start picture frame's rim, `startparts`) from Sky's hue and saturation to the glass colour's: hue turned as far, saturation scaled as much.
#[derive(Clone, Copy)]
pub(super) struct Hue {
    turn: f64,
    scale: f64,
}

fn hsv(c: [f64; 3]) -> (f64, f64, f64) {
    let hi = c[0].max(c[1]).max(c[2]);
    let lo = c[0].min(c[1]).min(c[2]);
    let d = hi - lo;
    let h = if d == 0.0 {
        0.0
    } else if hi == c[0] {
        60.0 * ((c[1] - c[2]) / d).rem_euclid(6.0)
    } else if hi == c[1] {
        60.0 * ((c[2] - c[0]) / d + 2.0)
    } else {
        60.0 * ((c[0] - c[1]) / d + 4.0)
    };
    let s = if hi == 0.0 { 0.0 } else { d / hi };
    (h, s, hi / 255.0)
}

fn rgb((h, s, v): (f64, f64, f64)) -> [f64; 3] {
    let h = h.rem_euclid(360.0) / 60.0;
    let c = v * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    [r, g, b].map(|u| (u + m) * 255.0)
}

impl Hue {
    pub(super) fn to(c: Color) -> Hue {
        let of = |c: Color| hsv([c.r, c.g, c.b].map(f64::from));
        let ((h0, s0, _), (h1, s1, _)) = (of(SWATCHES[0].color), of(c));
        if c == SWATCHES[0].color {
            return Hue {
                turn: 0.0,
                scale: 1.0,
            };
        }
        Hue {
            turn: if s1 == 0.0 { 0.0 } else { h1 - h0 },
            scale: s1 / s0,
        }
    }

    pub(super) fn colour(&self, c: [f64; 3]) -> [f64; 3] {
        if self.turn == 0.0 && self.scale == 1.0 {
            return c;
        }
        let (h, s, v) = hsv(c);
        rgb((h + self.turn, (s * self.scale).min(1.0), v))
    }

    fn layer(&self, p: Px) -> Px {
        if p.0 <= 0.0 {
            return p;
        }
        let c = p.1.map(|v| (v * 255.0 / p.0).clamp(0.0, 255.0));
        lay(p.0, self.colour(c))
    }
}

struct Paint {
    k: usize,
    active: bool,
    under: Under,
    glass: Glass,
    finish: Recolour,
    hue: Hue,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Lit {
    Hot,
    Held,
}

impl Paint {
    fn new(s: &Settings, surface: Surface) -> Paint {
        Paint {
            k: usize::from(surface == Surface::Inactive),
            active: surface == Surface::Active,
            under: Under::new(s, surface),
            glass: Glass::new(s, surface, 0.0),
            finish: Recolour::new(s, surface),
            hue: Hue::to(s.color),
        }
    }

    fn body(&self, g: Glyph, r: i32, t: f64, lit: bool) -> Px {
        let sheen = banded(&SHEEN[self.k].map(level), r, t).0;
        let deepen = banded(&DEEPEN[self.k].map(level), r, t).0;
        let b = self.glass.at(sheen, deepen);
        if g == Glyph::Close && (self.active || lit) {
            over(banded(&CLOSE_RED, r, t), b)
        } else {
            b
        }
    }

    fn glow(&self, g: Glyph, how: Lit) -> Shine {
        let i = usize::from(how == Lit::Held);
        let (src, hue) = if g == Glyph::Close {
            (&CLOSE_LIT[i], None)
        } else {
            (&LIT[i], Some(self.hue))
        };
        let turn = |p: Px| hue.map_or(p, |h| h.layer(p));
        let colour = |c: [f64; 3]| hue.map_or(c, |h| h.colour(c));
        Shine {
            fill: src.fill.map(turn),
            edge: src.edge.map(turn),
            power: src.power,
            spot: src.spot,
            spot_colour: colour(src.spot_colour),
            bleed: src.bleed,
            bleed_colour: colour(src.bleed_colour),
        }
    }
}

/// A strip laid out `grow` rows taller than its design, each cell as wide as the frame's
/// [`CaptionGroup::grown`](crate::theme::CaptionGroup::grown) cell: the body's rows and the cells
/// stretch, the rings round them keep their 1 px, and the glyphs grow by [`Laid::glyph`].
struct Laid {
    width: i32,
    rows: i32,
    grow: i32,
    cells: Vec<(i32, i32, Glyph)>,
    dividers: Vec<i32>,
    /// Each cell's width over its design width, for the lit spot's reach across.
    spread: Vec<f64>,
    /// How many times its design size each glyph is drawn: the body's growth ([`glyph::bold`]).
    glyph: f64,
}

impl Laid {
    fn of(layout: &Strip, grow: i32) -> Laid {
        let (base, wide) = (super::CAPTION_GROUP, super::CAPTION_GROUP.grown(grow));
        let more = |g: Glyph| {
            let i = match g {
                Glyph::Minimize | Glyph::Help => 0,
                Glyph::Maximize | Glyph::Restore => 1,
                Glyph::Close => 2,
            };
            wide.widths[i] - base.widths[i]
        };
        let (mut cells, mut spread, mut at) = (Vec::new(), Vec::new(), 0);
        for &(x0, x1, g) in layout.cells {
            let d = more(g);
            cells.push((x0 + at, x1 + at + d, g));
            spread.push(f64::from(x1 - x0 + d) / f64::from(x1 - x0));
            at += d;
        }
        let dividers = (layout.dividers.iter())
            .map(|&x| {
                let before = layout.cells.iter().filter(|c| c.1 < x);
                x + before.map(|c| more(c.2)).sum::<i32>()
            })
            .collect();
        Laid {
            width: layout.width + at,
            rows: ROWS + grow,
            grow,
            cells,
            dividers,
            spread,
            glyph: f64::from(BODY + grow) / f64::from(BODY),
        }
    }

    /// The design y (restored strip rows, [`centre`]d) `y` is shaded as: the body's rows stretch
    /// over the grown body, the light line, outline and halo below it keep their rows.
    fn shade_y(&self, y: f64) -> f64 {
        let (body, edge) = (f64::from(BODY + self.grow), y + 0.5);
        if self.grow == 0 || edge < 1.0 {
            y
        } else if edge < 1.0 + body {
            0.5 + (edge - 1.0) * f64::from(BODY) / body
        } else {
            y - f64::from(self.grow)
        }
    }

    /// The caption band's design row `y` (rows from the window's top) shows under a strip on a
    /// caption grown as much: the band is drawn [`BAND`] rows tall and stretched row by row.
    fn band_y(&self, y: f64) -> f64 {
        if self.grow == 0 {
            return y;
        }
        let row = (y + 0.5).floor() as i32;
        f64::from(row * BAND / (BAND + self.grow)) + y - f64::from(row)
    }

    /// The top row of a glyph `h` rows tall whose design middle is `mid`: its middle kept where
    /// the stretched body puts it.
    fn glyph_row(&self, mid: f64, h: i32) -> i32 {
        let grown = 1.0 + (mid - 1.0) * f64::from(BODY + self.grow) / f64::from(BODY);
        (grown - f64::from(h) / 2.0).round() as i32
    }
}

/// What every strip at `n` pixels per design pixel shares: per-pixel coverage by halo, outline, light line and body, and the glyph's cover, outline and white.
struct Shape {
    n: i32,
    laid: Laid,
    cov: Vec<[f64; 4]>,
    glyphs: Vec<[f64; 3]>,
}

impl Shape {
    /// Each ring is `n` px wide and the bottom corners follow [`RADIUS`] (grown with the strip) times `n`, antialiased at the strip's own pixels.
    fn of(layout: &Strip, grow: i32, n: i32) -> Shape {
        let laid = Laid::of(layout, grow);
        let k = f64::from(n);
        let (w, h) = (laid.width * n, laid.rows * n);
        let radius = RADIUS + RADIUS * f64::from(grow) / f64::from(ROWS);
        let mut cov = Vec::with_capacity((w * h) as usize);
        for y in 0..h {
            for x in 0..w {
                cov.push(std::array::from_fn(|i| {
                    let i = i as f64;
                    rounded(
                        f64::from(x) - i * k,
                        f64::from(y) + 50.0 * k,
                        f64::from(w) - 2.0 * i * k,
                        (f64::from(laid.rows) - i + 50.0) * k,
                        (radius - i) * k,
                    )
                }));
            }
        }
        let mut glyphs = vec![[0.0_f64; 3]; (w * h) as usize];
        let fill = GLYPH_FILL.map(|(t, v)| (t, level(v)));
        let column = glyph::bold_column(laid.glyph);
        for &(x0, x1, g) in &laid.cells {
            let (mid, mark) = glyph::bold(g, laid.glyph);
            let row = laid.glyph_row(mid, mark.h);
            let (gx, rows) = ((x0 + x1) / 2 + 1 - column, f64::from(mark.h.max(2) - 1));
            let (gw, gh) = (mark.w, mark.h);
            for y in ((row - 1) * n).max(0)..((row + gh + 1) * n).min(h) {
                for x in ((gx - 1) * n).max(0)..((gx + gw + 1) * n).min(w) {
                    let (dx, dy) = (x - gx * n, y - row * n);
                    let grown = mark.cover(dx, dy, n, 1.0);
                    if grown == 0.0 {
                        continue;
                    }
                    let t = ((centre(y, n) - f64::from(row)) / rows).clamp(0.0, 1.0);
                    let px = &mut glyphs[(y * w + x) as usize];
                    px[0] = px[0].max(mark.cover(dx, dy, n, 0.0));
                    px[1] = px[1].max(grown);
                    px[2] = Stops(&fill).at(t).0;
                }
            }
        }
        Shape {
            n,
            laid,
            cov,
            glyphs,
        }
    }

    fn glyph(&self, x: i32, y: i32) -> [f64; 3] {
        let w = self.laid.width * self.n;
        self.glyphs[(y * w + x) as usize]
    }
}

/// Per pixel parts and shares: band outside the halo, halo, outline, light line, body.
type Parts = [(Px, f64); 5];

impl Shine {
    /// Fill at row `r` (design px) and `u` from the cell's middle column (0) to its edge (1).
    fn fill_at(&self, r: f64, u: f64) -> Px {
        let row = |fill: &[Px; 9]| {
            let stops: [(f64, Px); 9] = std::array::from_fn(|i| (f64::from(FILL_ROWS[i]), fill[i]));
            Stops(&stops).at(r)
        };
        let (mid, edge) = (row(&self.fill), row(&self.edge));
        let t = u.abs().min(1.0).powf(self.power);
        shape::mix(mid, edge, t)
    }
}

/// Draw `layout`'s strip, restored or `maximized`, through `p` with cell `lit` lit, into `c` from row `top` at the shape's scale. The outline down the strip's end beside a lit end cell is the cell's own.
fn draw(
    c: &mut Canvas,
    top: i32,
    shape: &Shape,
    p: &Paint,
    lit: Option<(usize, Lit)>,
    maximized: bool,
) {
    let (layout, n) = (&shape.laid, shape.n);
    let w = layout.width;
    let skip = i32::from(maximized);
    let last = layout.cells.len() - 1;
    let cell_of = |x: i32| {
        layout
            .cells
            .iter()
            .position(|&(x0, x1, _)| (x0..=x1).contains(&x))
    };
    let near = |x: i32| cell_of(x).unwrap_or(if x < w / 2 { 0 } else { last });
    let lit_cell = lit.map(|(i, _)| i);
    let glow = lit.map(|(i, how)| (i, layout.cells[i], p.glow(layout.cells[i].2, how)));
    let k = p.k;
    for y in 0..(layout.rows - skip) * n {
        let yr = y + skip * n;
        let (at, cy) = (centre(yr, n), layout.shade_y(centre(yr, n)));
        let r = (cy + 0.5).floor() as i32;
        let body = |i: usize, r: i32| p.body(layout.cells[i].2, r, cy, lit_cell == Some(i));
        for xn in 0..w * n {
            let (x, cx) = (xn / n, centre(xn, n));
            let band = p.under.at(
                f64::from(RIGHT + w - 1) - cx,
                layout.band_y(at + 1.0 - f64::from(skip)),
                maximized,
            );
            let cov = shape.cov[(yr * w * n + xn) as usize];
            let divider = layout.dividers.contains(&x);
            let (ring, inside) = if r == 0 {
                // The light line's row: the halo, the outline and the
                // dividers reach up through it.
                let inside = if divider {
                    over(DIVIDER[k], band)
                } else {
                    band
                };
                (band, inside)
            } else if divider && r <= 17 {
                let d = over(DIVIDER[k], body(near(x + 1), r));
                (d, d)
            } else {
                let i = near(x);
                let (x0, x1, _) = layout.cells[i];
                let b = body(i, r);
                let ring = over(LIGHT[k], b);
                (ring, if x == x0 || x == x1 { ring } else { b })
            };
            let mut parts: Parts = [
                (band, 1.0 - cov[0]),
                (over(HALO[k], band), cov[0] - cov[1]),
                (
                    over(OUTLINE[k], body(near(x), r.clamp(1, 16))),
                    cov[1] - cov[2],
                ),
                (ring, cov[2] - cov[3]),
                (inside, cov[3]),
            ];
            let mut spot = None;
            if let Some((i, (x0, x1, _), g)) = &glow {
                let (i, x0, x1) = (*i, *x0, *x1);
                let mid = f64::from(x0 + x1) / 2.0;
                let u = (cx - mid) / (f64::from(x1 - x0) / 2.0);
                if (x0..=x1).contains(&x) {
                    if r >= 1 {
                        let halo = g.fill_at(19.0, u);
                        parts[0].0 = over(halo, parts[0].0);
                        parts[1].0 = over(halo, parts[1].0);
                        parts[2].0 = over(g.fill_at(18.0, u), parts[2].0);
                        let lit = over(g.fill_at(body_row(r, cy), u), body(i, r));
                        let ring = over(LIGHT[k], lit);
                        parts[3].0 = ring;
                        parts[4].0 = if x == x0 || x == x1 { ring } else { lit };
                        let (peak, across, down, row, power) = g.spot;
                        let dx = (cx - mid) / (across * layout.spread[i]);
                        let dy = (cy - row) / down;
                        let a = peak * (-(dx * dx + dy * dy).powf(power / 2.0)).exp();
                        spot = Some(lay(a, g.spot_colour));
                    }
                } else {
                    // Columns past the cell, from 1 for the pixel beside
                    // it: a finer pixel's middle, never nearer than that.
                    let d = if x < x0 {
                        f64::from(x0) - cx
                    } else {
                        cx - f64::from(x1)
                    }
                    .max(1.0);
                    let (edge, keep) = g.bleed;
                    let (from, to) = BLEED_ENDS;
                    let fade = 1.0 - smooth((d - f64::from(from)) / f64::from(to - from));
                    let bleed = lay(edge * keep.powf(d - 1.0) * fade, g.bleed_colour);
                    for n in [0, 1, 3, 4] {
                        parts[n].0 = over(bleed, parts[n].0);
                    }
                    if (x < x0 && i == 0) || (x > x1 && i == last) {
                        let r = r.clamp(1, 16);
                        let fill = g.fill_at(body_row(r, cy), 1.0);
                        parts[2].0 = over(OUTLINE[0], over(fill, body(i, r)));
                    } else {
                        parts[2].0 = over(bleed, parts[2].0);
                    }
                }
            }
            let mut px = shape::shares(&parts);
            if let Some(spot) = spot {
                px = over(spot, px);
            }
            let [ink, grown, v] = shape.glyph(xn, yr);
            if grown > 0.0 {
                px = over(shape::scale(lay(255.0, GLYPH_EDGE), grown), px);
                px = over(shape::scale((255.0, [v; 3]), ink), px);
            }
            c.put_raw(xn, top + y, p.finish.finish(pack(px)));
        }
    }
}

fn surface_of(state: State) -> (Surface, Lit) {
    match state {
        State::Inactive | State::InactiveHot => (Surface::Inactive, Lit::Hot),
        State::Pressed => (Surface::Active, Lit::Held),
        _ => (Surface::Active, Lit::Hot),
    }
}

fn frames(
    s: &Settings,
    layout: &Strip,
    list: &[(State, Option<usize>)],
    maximized: bool,
    (grow, n): (i32, i32),
) -> PartImage {
    frames_of_shape(s, list, maximized, &Shape::of(layout, grow, n))
}

fn frames_of_shape(
    s: &Settings,
    list: &[(State, Option<usize>)],
    maximized: bool,
    shape: &Shape,
) -> PartImage {
    let (n, layout) = (shape.n, &shape.laid);
    let h = (layout.rows - i32::from(maximized)) * n;
    let mut c = Canvas::new((layout.width * n) as u32, (h * list.len() as i32) as u32);
    let paints = [
        Paint::new(s, Surface::Active),
        Paint::new(s, Surface::Inactive),
    ];
    for (i, &(state, lit)) in list.iter().enumerate() {
        let (surface, how) = surface_of(state);
        let p = &paints[usize::from(surface == Surface::Inactive)];
        draw(
            &mut c,
            i as i32 * h,
            shape,
            p,
            lit.map(|i| (i, how)),
            maximized,
        );
    }
    let states: Vec<State> = list.iter().map(|&(st, _)| st).collect();
    PartImage::new(c, &states, Layout::Vertical).at_scale(n)
}

const GROUP_FRAMES: [(State, Option<usize>); 11] = [
    (State::Normal, None),
    (State::Inactive, None),
    (State::Hot, Some(0)),
    (State::Hot, Some(1)),
    (State::Hot, Some(2)),
    (State::Pressed, Some(0)),
    (State::Pressed, Some(1)),
    (State::Pressed, Some(2)),
    (State::InactiveHot, Some(0)),
    (State::InactiveHot, Some(1)),
    (State::InactiveHot, Some(2)),
];

/// Minimize, maximize and close in one 105x20 strip, in [`GROUP_FRAMES`] order, painted at `n` pixels per design pixel.
pub fn caption_group(s: &Settings, n: i32) -> PartImage {
    frames(s, &GROUP, &GROUP_FRAMES, false, (0, n))
}

/// The group on a maximized window, 105x19 (no row 0).
pub fn max_caption_group(s: &Settings, n: i32) -> PartImage {
    frames(s, &MAX_GROUP, &GROUP_FRAMES, true, (0, n))
}

pub fn dialog_close(s: &Settings, n: i32) -> PartImage {
    frames(s, &DIALOG_CLOSE, &CLOSE_FRAMES, false, (0, n))
}

const CLOSE_FRAMES: [(State, Option<usize>); 5] = [
    (State::Normal, None),
    (State::Inactive, None),
    (State::Hot, Some(0)),
    (State::Pressed, Some(0)),
    (State::InactiveHot, Some(0)),
];

/// A dialog's ? box joined to its close box, 78x20, one frame per box, the ? box's first.
pub fn help_strip(s: &Settings, n: i32) -> PartImage {
    frames(s, &HELP, &HELP_FRAMES, false, (0, n))
}

/// A windowed dialog's minimize box joined to its close box, 78x20, in [`HELP_FRAMES`] order.
pub fn min_close(s: &Settings, n: i32) -> PartImage {
    frames(s, &MIN_CLOSE, &HELP_FRAMES, false, (0, n))
}

const HELP_FRAMES: [(State, Option<usize>); 8] = [
    (State::Normal, None),
    (State::Inactive, None),
    (State::Hot, Some(0)),
    (State::Hot, Some(1)),
    (State::Pressed, Some(0)),
    (State::Pressed, Some(1)),
    (State::InactiveHot, Some(0)),
    (State::InactiveHot, Some(1)),
];

/// The strips and boxes for a caption whose restored strip is `side` rows tall, painted at `n`:
/// each laid out as the frame's [`CaptionGroup::grown`](crate::theme::CaptionGroup::grown).
pub fn sided(s: &Settings, n: i32, side: i32) -> Vec<(Part, PartImage)> {
    let at = (side - ROWS, n);
    vec![
        (
            Part::CaptionGroup,
            frames(s, &GROUP, &GROUP_FRAMES, false, at),
        ),
        (
            Part::MaxCaptionGroup,
            frames(s, &MAX_GROUP, &GROUP_FRAMES, true, at),
        ),
        (
            Part::CloseButton,
            frames(s, &DIALOG_CLOSE, &CLOSE_FRAMES, false, at),
        ),
        (Part::HelpButton, frames(s, &HELP, &HELP_FRAMES, false, at)),
        (
            Part::MinCloseGroup,
            frames(s, &MIN_CLOSE, &HELP_FRAMES, false, at),
        ),
    ]
}

#[cfg(test)]
mod tests;
