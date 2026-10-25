//! Reprise Glass's Start panel pieces, drawn from parameters; [`parts`] makes them the style's `Part::Start*` parts.
//! Pieces on the panel's glass are fixed layers the glass shows through; the picture frame is glass of its own ([`Glass::at`]);
//! pieces on the white pane are the same in every colour. Each piece is painted at `d` pixels per design pixel.

use crate::mutex::lock;
use std::sync::{Mutex, OnceLock};

use super::Settings;
use super::buttons::Hue;
use super::glyph;
use super::panel::{self, Fade, Glass, LightBox, Shadow};
use super::tint::{Recolour, Surface};
use crate::raster::{Canvas, Insets};
use crate::shape::{self, Px, centre, mix, over, pack, ringed, rounded, smooth};
use crate::style::{Layout, Part, PartImage, State};
use crate::theme::Color;

/// A colour at an alpha: the alpha in levels, then 0xRRGGBB, not
/// premultiplied.
type Ink = (f64, u32);

fn rgb(v: u32) -> [f64; 3] {
    [16, 8, 0].map(|sh| f64::from((v >> sh) & 0xFF))
}

fn px((a, c): Ink) -> Px {
    (a, rgb(c).map(|v| v * a / 255.0))
}

fn opaque(c: u32) -> Px {
    px((255.0, c))
}

/// How far row `y` (in design pixels) lies from stop `y0` to stop `y1`,
/// 0..1: straight between them, but between stops a row apart a step on
/// the design row, so a band's edge stays sharp at any scale.
fn between(y: f64, y0: i32, y1: i32) -> f64 {
    let t = if y1 - y0 == 1 {
        y.round() - f64::from(y0)
    } else {
        (y - f64::from(y0)) / f64::from(y1 - y0)
    };
    t.clamp(0.0, 1.0)
}

fn along(stops: &[(i32, Ink)], y: f64) -> Px {
    let (first, last) = (stops[0], stops[stops.len() - 1]);
    if y <= f64::from(first.0) {
        return px(first.1);
    }
    for w in stops.windows(2) {
        let ((y0, a), (y1, b)) = (w[0], w[1]);
        if y <= f64::from(y1) {
            let t = between(y, y0, y1);
            let (ca, cb) = (rgb(a.1), rgb(b.1));
            let alpha = a.0 + (b.0 - a.0) * t;
            let c: [f64; 3] = std::array::from_fn(|i| ca[i] + (cb[i] - ca[i]) * t);
            return (alpha, c.map(|v| v * alpha / 255.0));
        }
    }
    px(last.1)
}

fn level(stops: &[(i32, f64)], y: f64) -> f64 {
    let (first, last) = (stops[0], stops[stops.len() - 1]);
    if y <= f64::from(first.0) {
        return first.1;
    }
    for w in stops.windows(2) {
        let ((y0, a), (y1, b)) = (w[0], w[1]);
        if y <= f64::from(y1) {
            return a + (b - a) * between(y, y0, y1);
        }
    }
    last.1
}

fn share(y: f64, from: i32, to: i32) -> f64 {
    ((y - f64::from(from)) / f64::from((to - from).max(1))).clamp(0.0, 1.0)
}

fn cell(v: f64) -> i32 {
    v.round() as i32
}

fn canvas(w: i32, pixels: &[Px]) -> Canvas {
    let h = pixels.len() as i32 / w;
    let mut c = Canvas::new(w as u32, h as u32);
    for (i, &p) in pixels.iter().enumerate() {
        c.put_raw(i as i32 % w, i as i32 / w, pack(p));
    }
    c
}

fn one(c: Canvas, ins: Insets, d: i32) -> PartImage {
    PartImage::new(c, &[State::Normal], Layout::Vertical)
        .sizing(ins)
        .at_scale(d)
}

const PANE: (i32, i32) = (14, 55);
const PANE_R: f64 = 5.0;
const PANE_RINGS: [Ink; 2] = [(61.0, 0xFFFFFF), (143.0, 0x424444)];
const BAND_ROWS: i32 = 47;
const BAND_RULE: u32 = 0xCAD6E9;
const BAND: u32 = 0xF0F4FB;
const BAND_FADE: i32 = 5;
const PANE_GRID: Insets = Insets::new(6, 6, 6, BAND_ROWS);

fn pane(d: i32) -> Canvas {
    let (w, h) = PANE;
    let rule = h - BAND_ROWS;
    let fill = |y: f64| {
        let k = if cell(y) == rule {
            0.0
        } else {
            y - f64::from(rule)
        };
        if k < 0.0 {
            return opaque(0xFFFFFF);
        }
        let t = (1.0 - k / f64::from(BAND_FADE)).max(0.0);
        mix(opaque(BAND), opaque(BAND_RULE), t * t)
    };
    let pixels = ringed(w, h, PANE_R, 2, d, |_, y, i| match i {
        0 | 1 => px(PANE_RINGS[i]),
        _ => fill(y),
    });
    canvas(w * d, &pixels)
}

const JUMP: (i32, i32) = (9, 417);
const JUMP_GRID: Insets = Insets::new(4, 165, 3, 246);
const JUMP_DIVIDER: u32 = 0xD5E2F5;
const JUMP_EDGE: u32 = 0xECF2FA;
const JUMP_PEAK: u32 = 0xFCFDFE;
const JUMP_PEAK_ROW: i32 = 185;
const JUMP_POWER: f64 = 1.3;
const JUMP_SHADE: [[f64; 3]; 9] = [
    [0.0; 3],
    [-11.0, -7.0, -2.0],
    [-6.0, -4.0, -1.0],
    [0.0; 3],
    [0.0; 3],
    [0.0; 3],
    [3.0, 2.0, 1.0],
    [3.0, 2.0, 1.0],
    [3.0, 2.0, 1.0],
];

fn jump_pane(d: i32) -> Canvas {
    let (w, h) = JUMP;
    let (edge, peak) = (rgb(JUMP_EDGE), rgb(JUMP_PEAK));
    let k = f64::from(d);
    let mut pixels = Vec::with_capacity((w * h * d * d) as usize);
    for y in 0..h * d {
        let off = centre(y, d) - f64::from(JUMP_PEAK_ROW);
        let reach = if off < 0.0 {
            JUMP_PEAK_ROW
        } else {
            h - 1 - JUMP_PEAK_ROW
        };
        let pale = 1.0 - (off.abs() / f64::from(reach)).powf(JUMP_POWER);
        for x in 0..w * d {
            let col = x / d;
            let c: [f64; 3] = if col == 0 {
                rgb(JUMP_DIVIDER)
            } else {
                std::array::from_fn(|i| {
                    (edge[i] + (peak[i] - edge[i]) * pale + JUMP_SHADE[col as usize][i]).min(255.0)
                })
            };
            let far = 1000.0;
            let cov = rounded(
                f64::from(x) + far,
                f64::from(y),
                f64::from(w) * k + far,
                f64::from(h) * k + far,
                (PANE_R - 2.0) * k,
            );
            pixels.push((255.0 * cov, c.map(|v| v * cov)));
        }
    }
    canvas(w * d, &pixels)
}

const fn row_box(outline: u32, line: [u32; 2], fill: [u32; 2]) -> LightBox {
    const fn ink(v: u32) -> (Color, u8) {
        (Color::rgb((v >> 16) as u8, (v >> 8) as u8, v as u8), 255)
    }
    LightBox {
        outline: ink(outline),
        line: Some((ink(line[0]), ink(line[1]))),
        fill: (ink(fill[0]), ink(fill[1])),
        radius: 3.0,
        fade: Fade::Ring,
    }
}

const BOX_W: i32 = 10;
const BOX_GRID: Insets = Insets::new(4, 3, 4, 3);
const HOT_LEFT: LightBox = row_box(0x7A9BCD, [0xEAF3FD, 0xDAE7FD], [0xDCEAFD, 0xC0D8FD]);
const HOT_ALL: LightBox = row_box(0x6C92C8, [0xE7EFFD, 0xD6E5FD], [0xD0E3FC, 0xB1D0FC]);
const HOT_JUMP: LightBox = row_box(0x799ACD, [0xE8F2FC, 0xD8E6FC], [0xD8E6FC, 0xBDD6FC]);
const LIT_RESULT: LightBox = row_box(0x81A5DC, [0xF1F7FF, 0xDEECFF], [0xEBF4FF, 0xCEE2FF]);
const NEW_PIN: LightBox = row_box(0xF1BD8D, [0xFEFBE2, 0xFFE8D0], [0xFDF4BE, 0xFECC99]);

fn row_part(b: &LightBox, h: i32, d: i32) -> PartImage {
    one(panel::light_box(b, BOX_W, h, d), BOX_GRID, d)
}

fn hot_split(d: i32) -> Canvas {
    let (w, h) = (3, 36);
    let b = &HOT_LEFT;
    let outline = (
        255.0,
        [b.outline.0.r, b.outline.0.g, b.outline.0.b].map(f64::from),
    );
    let mut pixels = Vec::new();
    for y in 0..h * d {
        for x in 0..w * d {
            let (col, row) = (x / d, y / d);
            pixels.push(match b.line_at(centre(y, d), h) {
                Some(line) if col != 1 && row != 0 && row != h - 1 => line,
                _ => outline,
            });
        }
    }
    canvas(w * d, &pixels)
}

const RULE_LEFT: u32 = JUMP_DIVIDER;
const RULE_JUMP: u32 = 0xA2B4DC;
const RULE_SEARCH: u32 = 0xE3E4E4;
const RULE_RIGHT: [Ink; 2] = [(39.0, 0x000000), (24.0, 0xFFFFFF)];

fn rule(rows: &[Ink]) -> Canvas {
    canvas(1, &rows.iter().map(|&r| px(r)).collect::<Vec<_>>())
}

const FIELD: (i32, i32) = (10, 24);
const FIELD_GRID: Insets = Insets::new(4, 11, 4, 11);
const FIELD_R: f64 = 3.0;
const FIELD_HALO: [f64; 4] = [128.0, 191.0, 128.0, 146.0];
const FIELD_TOP: u32 = 0x3D3E3E;
const FIELD_BOTTOM: u32 = 0xABABAD;
const FIELD_LEFT: [u32; 2] = [0x444444, 0x818182];
const FIELD_RIGHT: [u32; 2] = [0x686869, 0xA3A4A5];

/// Which side of ring `i` of a `w` x `h` box (`x`, `y`) lies on: 0 top,
/// 1 left, 2 right, 3 bottom; the top or bottom where a corner ties.
fn side(x: i32, y: i32, w: i32, h: i32, i: i32) -> usize {
    let d = [y - i, x - i, w - 1 - i - x, h - 1 - i - y];
    let near = *d.iter().min().unwrap();
    if d[0] == near {
        0
    } else if d[3] == near {
        3
    } else if d[1] == near {
        1
    } else {
        2
    }
}

fn field(d: i32) -> Canvas {
    let (w, h) = FIELD;
    let pixels = ringed(w, h, FIELD_R, 2, d, |x, y, i| match i {
        0 => px((FIELD_HALO[side(cell(x), cell(y), w, h, 0)], 0xFFFFFF)),
        1 => {
            let t = share(y, 3, h - 4);
            match side(cell(x), cell(y), w, h, 1) {
                0 => opaque(FIELD_TOP),
                3 => opaque(FIELD_BOTTOM),
                1 => mix(opaque(FIELD_LEFT[0]), opaque(FIELD_LEFT[1]), t),
                _ => mix(opaque(FIELD_RIGHT[0]), opaque(FIELD_RIGHT[1]), t),
            }
        }
        _ => opaque(0xFFFFFF),
    });
    canvas(w * d, &pixels)
}

const SHUT_W: i32 = 72;
const ARROW_W: i32 = 22;
const SHUT_H: i32 = 22;
const SHUT_R: f64 = 2.5;
const SHUT_GRID: Insets = Insets::new(35, 10, 35, 10);
const SHUT_MIDDLE: i32 = 11;
const SHUT_SPREAD: f64 = 5.0;

struct Face {
    outline: [Ink; 2],
    top: Ink,
    side: &'static [(i32, Ink)],
    bottom: Ink,
    face: &'static [(i32, Ink)],
}

const GLASS_NORMAL: Face = Face {
    outline: [(82.0, 0), (166.0, 0)],
    top: (181.0, 0xFFFFFF),
    side: &[
        (2, (151.0, 0xF4F6F6)),
        (10, (85.0, 0xFCFFFF)),
        (11, (59.0, 0xFCFFFF)),
        (18, (82.0, 0xFCFFFF)),
        (19, (96.0, 0xDBDEDE)),
    ],
    bottom: (104.0, 0xFAFCFF),
    face: &[
        (2, (132.0, 0xF3F5F5)),
        (4, (104.0, 0xE6EAEA)),
        (6, (90.0, 0xCFD2D3)),
        (8, (74.0, 0xB1B4B4)),
        (10, (59.0, 0x7A7E7E)),
        (11, (44.0, 0)),
        (18, (16.0, 0)),
        (19, (23.0, 0x6C8182)),
    ],
};

const GLASS_HOT: Face = Face {
    outline: [(128.0, 0), (221.0, 0)],
    top: (246.0, 0xFFFFFF),
    side: &[
        (2, (222.0, 0xF4F6F6)),
        (10, (186.0, 0xFDFFFF)),
        (11, (168.0, 0xFFFFFF)),
        (18, (178.0, 0xFFFFFF)),
        (19, (183.0, 0xDFE0DF)),
    ],
    bottom: (189.0, 0xFFFFFF),
    face: &[
        (2, (209.0, 0xFBFAFB)),
        (4, (158.0, 0xE7E9E9)),
        (6, (138.0, 0xCED0D1)),
        (8, (120.0, 0xA7A8A8)),
        (10, (103.0, 0x6F7171)),
        (11, (92.0, 0)),
        (18, (33.0, 0)),
        (19, (101.0, 0xCDCFCF)),
    ],
};

const GLASS_PRESSED: Face = Face {
    outline: [(128.0, 0), (221.0, 0)],
    top: (213.0, 0x9A9C9C),
    side: &[
        (2, (181.0, 0x7C7D7D)),
        (6, (149.0, 0x6F7171)),
        (10, (109.0, 0x3A3D3D)),
        (11, (83.0, 0x000303)),
        (15, (93.0, 0x1D1F1F)),
        (18, (101.0, 0x2D3030)),
        (19, (112.0, 0x252727)),
    ],
    bottom: (123.0, 0x505353),
    face: &[
        (2, (157.0, 0xF1F2F3)),
        (4, (135.0, 0xDFDFE1)),
        (6, (122.0, 0xC2C4C4)),
        (8, (109.0, 0x98999A)),
        (10, (96.0, 0x606162)),
        (11, (92.0, 0)),
        (18, (33.0, 0)),
        (19, (27.0, 0)),
    ],
};

const LIGHT: Face = Face {
    outline: [(255.0, 0xA2A3A8), (255.0, 0x525456)],
    top: (255.0, 0xFBFCFE),
    side: &[
        (2, (255.0, 0xF9FBFD)),
        (11, (255.0, 0xF4F6FC)),
        (19, (255.0, 0xF6F7FC)),
    ],
    bottom: (255.0, 0xF7F8FD),
    face: &[
        (2, (255.0, 0xF2F4F8)),
        (4, (255.0, 0xEDEFF4)),
        (6, (255.0, 0xE6E9EC)),
        (8, (255.0, 0xDFE2E6)),
        (10, (255.0, 0xD6D8DE)),
        (11, (255.0, 0xC7CBD1)),
        (12, (255.0, 0xD0D3DA)),
        (15, (255.0, 0xD9DCE2)),
        (19, (255.0, 0xE6EAEF)),
    ],
};

const ARROW_AT: [(i32, i32); 2] = [(8, 7), (9, 7)];
const ARROW_WHITE: [(i32, Ink); 4] = [
    (0, (255.0, 0xFFFFFF)),
    (2, (255.0, 0xFFFFFF)),
    (3, (255.0, 0xEBF0F8)),
    (4, (255.0, 0xD3D7DF)),
];
const ARROW_OUTLINE: f64 = 160.0;
const ARROW_DARK: u32 = 0x3E3E3E;

fn button(f: &Face, glass: bool, d: i32) -> Vec<Px> {
    let (w, h) = (SHUT_W + ARROW_W, SHUT_H);
    let divider = SHUT_W - 1;
    let deep = |y: f64| {
        let t = (1.0 - (y - f64::from(SHUT_MIDDLE)).abs() / SHUT_SPREAD).max(0.0);
        mix(px(f.outline[0]), px(f.outline[1]), t.powf(1.5))
    };
    let mut pixels = ringed(w, h, SHUT_R, 2, d, |fx, fy, i| {
        let (x, y) = (cell(fx), cell(fy));
        if i > 0 && x == divider {
            return deep(fy);
        }
        match i {
            0 if y == 0 || y == h - 1 => px(f.outline[0]),
            0 => deep(fy),
            1 if y == 1 => px(f.top),
            1 if y == h - 2 => px(f.bottom),
            1 => along(f.side, fy),
            _ if (x - divider).abs() == 1 => along(f.side, fy),
            _ => along(f.face, fy),
        }
    });
    let (gx, gy) = ARROW_AT[usize::from(!glass)];
    let (ox, oy) = (SHUT_W + gx, gy);
    let m = glyph::ARROW;
    let dw = w * d;
    for y in -d..(m.h + 1) * d {
        for x in -d..(m.w + 1) * d {
            let i = ((oy * d + y) * dw + ox * d + x) as usize;
            let ink = m.cover(x, y, d, 0.0);
            if glass {
                let edge = m.cover(x, y, d, 1.0);
                pixels[i] = over(shape::scale(px((ARROW_OUTLINE, 0)), edge), pixels[i]);
                let white = along(&ARROW_WHITE, centre(y, d));
                pixels[i] = over(shape::scale(white, ink), pixels[i]);
            } else {
                pixels[i] = over(shape::scale(opaque(ARROW_DARK), ink), pixels[i]);
            }
        }
    }
    pixels
}

fn shutdown(faces: &[&Face], glass: bool, states: &[State], d: i32) -> (PartImage, PartImage) {
    let (w, sw, sh) = ((SHUT_W + ARROW_W) * d, SHUT_W * d, SHUT_H * d);
    let n = faces.len() as i32;
    let mut main = Canvas::new(sw as u32, (sh * n) as u32);
    let mut arrow = Canvas::new((ARROW_W * d) as u32, (sh * n) as u32);
    for (k, f) in faces.iter().enumerate() {
        let pixels = button(f, glass, d);
        let top = k as i32 * sh;
        for y in 0..sh {
            for x in 0..w {
                let p = pack(pixels[(y * w + x) as usize]);
                if x < sw {
                    main.put_raw(x, top + y, p);
                } else {
                    arrow.put_raw(x - sw, top + y, p);
                }
            }
        }
    }
    (
        PartImage::new(main, states, Layout::Vertical)
            .sizing(SHUT_GRID)
            .at_scale(d),
        PartImage::new(arrow, states, Layout::Vertical).at_scale(d),
    )
}

const HOT_RIGHT: (i32, i32) = (138, 33);
const HOT_RIGHT_GRID: Insets = Insets::new(68, 15, 68, 16);
const HOT_RIGHT_R: f64 = 5.5;
const RIGHT_HALO: Ink = (50.0, 0xFFFFFF);
const RIGHT_OUTLINE: Ink = (255.0, 0x000000);
const RIGHT_TOP: [Ink; 2] = [(133.0, 0xFFFFFF), (158.0, 0xC6C5C6)];
const RIGHT_BOTTOM: [Ink; 2] = [(103.0, 0xFCFBFC), (134.0, 0xB1B2B2)];
const RIGHT_SIDE: [Ink; 2] = [(162.0, 0xB5B6B6), (138.0, 0x969797)];
const RIGHT_UPPER: i32 = 16;
const RIGHT_MID: [(i32, Ink); 14] = [
    (3, (82.0, 0xFFFFFF)),
    (5, (80.0, 0xFBFBF8)),
    (8, (79.0, 0xE0E0DD)),
    (10, (78.0, 0xBDBEBE)),
    (12, (78.0, 0xA1A4A4)),
    (14, (70.0, 0x999A9A)),
    (16, (68.0, 0x969797)),
    (17, (33.0, 0x161616)),
    (20, (31.0, 0x2F2F2F)),
    (22, (34.0, 0x5E5F5F)),
    (24, (36.0, 0x989999)),
    (26, (38.0, 0xC0B9B9)),
    (28, (39.0, 0xEFF0F0)),
    (29, (39.0, 0xFFFFFF)),
];
const RIGHT_END: [(i32, Ink); 14] = [
    (3, (127.0, 0x999A9A)),
    (5, (122.0, 0x8D8D8D)),
    (8, (121.0, 0x7E7F7F)),
    (10, (119.0, 0x727272)),
    (12, (117.0, 0x636464)),
    (14, (114.0, 0x5A5A5A)),
    (16, (112.0, 0x5A5A5A)),
    (17, (84.0, 0x030303)),
    (20, (86.0, 0x0D0D0D)),
    (22, (86.0, 0x191919)),
    (24, (86.0, 0x2C2D2D)),
    (26, (86.0, 0x393939)),
    (28, (87.0, 0x4A4D4D)),
    (29, (92.0, 0x5C5E5F)),
];
const RIGHT_REACH: f64 = 50.0;

/// A lit place's box: fixed layers, the glass under it its body. Drawn
/// once at scale 1: the panel's ink is picked against it for every
/// setting.
pub(super) fn hot_right() -> &'static Canvas {
    static HOT: OnceLock<Canvas> = OnceLock::new();
    HOT.get_or_init(|| draw_hot_right(1))
}

fn draw_hot_right(d: i32) -> Canvas {
    let (w, h) = HOT_RIGHT;
    let end = |x: f64| {
        let d = x.min(f64::from(w - 1) - x) - 2.0;
        1.0 - smooth(d / RIGHT_REACH)
    };
    let pixels = ringed(w, h, HOT_RIGHT_R, 3, d, |fx, fy, i| {
        let e = end(fx);
        let (x, y) = (cell(fx), cell(fy));
        match i {
            0 => px(RIGHT_HALO),
            1 => px(RIGHT_OUTLINE),
            2 => match side(x, y, w, h, 2) {
                0 => mix(px(RIGHT_TOP[0]), px(RIGHT_TOP[1]), e),
                3 => mix(px(RIGHT_BOTTOM[0]), px(RIGHT_BOTTOM[1]), e),
                _ => px(RIGHT_SIDE[usize::from(y > RIGHT_UPPER)]),
            },
            _ => mix(along(&RIGHT_MID, fy), along(&RIGHT_END, fy), e),
        }
    });
    canvas(w * d, &pixels)
}

const FRAME: (i32, i32) = (64, 62);
const FRAME_RECT: (i32, i32, i32) = (3, 1, 58);
const FRAME_R: f64 = 7.0;
const HOLE: (i32, i32, i32) = (8, 6, 48);
const FRAME_SHADOW: Shadow = Shadow {
    margin: Insets::new(FRAME_RECT.0, FRAME_RECT.1, 3, 3),
    alpha: 95.0,
    sigma: 0.8,
    offset: (1.0, 0.9),
    grow: 0.5,
    radius: FRAME_R,
};

const FRAME_OUTLINE: Ink = (90.0, 0x000000);
const FRAME_RIM: [Ink; 2] = [(240.0, 0xFFFFFF), (240.0, 0x14B4D7)];
const RIM_TURN: (f64, f64) = (1.0, 1.25);
const FRAME_SHEEN: [(i32, f64); 9] = [
    (3, 2.2),
    (6, 2.2),
    (10, 1.76),
    (20, 1.24),
    (27, 0.97),
    (28, 0.62),
    (40, 0.84),
    (53, 0.95),
    (56, 0.7),
];
const FRAME_DEEPEN: [(i32, f64); 2] = [(27, 0.0), (28, 25.0)];
const FRAME_INNER: [(i32, f64); 4] = [(5, 200.0), (27, 140.0), (28, 115.0), (54, 130.0)];

pub(super) fn picture_frame(s: &Settings, d: i32) -> Canvas {
    framed(s, d, &mut pack)
}

fn framed(s: &Settings, d: i32, pack: &mut dyn FnMut(Px) -> u32) -> Canvas {
    let (fx, fy, side) = FRAME_RECT;
    let (hx, hy, hole) = HOLE;
    let glass = Glass::new(s, Surface::Active, 0.0);
    let re = Recolour::new(s, Surface::Active);
    let hue = Hue::to(s.color);
    let plain = glass.plain;
    let mut c = panel::shadow(&FRAME_SHADOW.at(d), side * d, side * d);
    let mut finish = |p: Px| re.finish(pack(p));
    let outline = finish(over(px(FRAME_OUTLINE), plain));
    let (a, sky) = FRAME_RIM[1];
    let cyan: Px = (a, hue.colour(rgb(sky)).map(|v| v * a / 255.0));
    let k = f64::from(d);
    for y in 0..FRAME.1 * d {
        for x in 0..FRAME.0 * d {
            let cov = |i: i32| {
                let i = f64::from(i);
                rounded(
                    f64::from(x - fx * d) - i * k,
                    f64::from(y - fy * d) - i * k,
                    f64::from(side * d) - 2.0 * i * k,
                    f64::from(side * d) - 2.0 * i * k,
                    (FRAME_R - i) * k,
                )
            };
            let [c0, c1, c2] = [cov(0), cov(1), cov(2)];
            if c0 == 0.0 {
                continue;
            }
            let (mx, my) = (centre(x, d), centre(y, d));
            let (dx, dy) = (cell(mx), cell(my));
            let along_frame = (mx - f64::from(fx)) / f64::from(side - 1)
                + (my - f64::from(fy)) / f64::from(side - 1);
            let t = smooth((along_frame - RIM_TURN.0) / (RIM_TURN.1 - RIM_TURN.0));
            let rim = finish(over(mix(px(FRAME_RIM[0]), cyan, t), plain));
            let in_hole = (hx..hx + hole).contains(&dx) && (hy..hy + hole).contains(&dy);
            let by_hole = (hx - 1..=hx + hole).contains(&dx) && (hy - 1..=hy + hole).contains(&dy);
            let inside = if in_hole {
                0
            } else if by_hole {
                finish(over(px((level(&FRAME_INNER, my), 0xFFFFFF)), plain))
            } else {
                let sheen = level(&FRAME_SHEEN, my);
                finish(glass.at(sheen, level(&FRAME_DEEPEN, my)))
            };
            let layers = [
                (c.get(x, y), 1.0 - c0),
                (outline, c0 - c1),
                (rim, c1 - c2),
                (inside, c2),
            ];
            let ch = |sh: u32| {
                let v: f64 = layers
                    .iter()
                    .map(|&(p, k)| f64::from((p >> sh) & 0xFF) * k)
                    .sum();
                (v.round() as u32).min(255) << sh
            };
            c.put_raw(x, y, ch(24) | ch(16) | ch(8) | ch(0));
        }
    }
    c
}

fn glyph_canvas(
    m: &glyph::Mark,
    stops: &[(i32, Ink)],
    level: impl Fn(f64) -> f64,
    d: i32,
) -> Canvas {
    m.canvas(d, |x, y| shape::scale(along(stops, y), level(x)))
}

const MAGNIFIER_LEVEL: f64 = 0.85;
const CLEAR_LEVEL: f64 = 0.7;
const TRIANGLE_LIGHT: f64 = 0.4;
const TRIANGLE_DEEP: f64 = 0.93;
const TRIANGLE_RIGHT_DEEP_AT: f64 = 2.0;
const TRIANGLE_LEFT_DEEP_AT: f64 = 4.0;

fn triangle_level(x: f64, deep_at: f64) -> f64 {
    let t = (x / deep_at).clamp(0.0, 1.0);
    TRIANGLE_LIGHT + (TRIANGLE_DEEP - TRIANGLE_LIGHT) * t
}

const MAGNIFIER_INK: [(i32, Ink); 3] = [
    (1, (255.0, 0x0082C3)),
    (7, (255.0, 0x00399B)),
    (11, (255.0, 0x00174E)),
];
const CLEAR_INK: [(i32, Ink); 3] = [
    (0, (255.0, 0x005DAA)),
    (3, (255.0, 0x004083)),
    (6, (255.0, 0x002155)),
];
const BLACK: [(i32, Ink); 1] = [(0, (255.0, 0x000000))];

pub(super) fn parts(s: &Settings, d: i32) -> Vec<(Part, PartImage)> {
    type Fixed = Vec<(i32, Vec<(Part, PartImage)>)>;
    static FIXED: Mutex<Fixed> = Mutex::new(Vec::new());
    let mut fixed = lock(&FIXED);
    let mut parts = match fixed.iter().find(|(k, _)| *k == d) {
        Some((_, p)) => p.clone(),
        None => {
            let p = fixed_parts(d);
            fixed.push((d, p.clone()));
            p
        }
    };
    parts.push((
        Part::StartPictureFrame,
        one(picture_frame(s, d), Insets::ZERO, d),
    ));
    parts
}

fn fixed_parts(d: i32) -> Vec<(Part, PartImage)> {
    let none = Insets::ZERO;
    let (glass, glass_arrow) = shutdown(
        &[&GLASS_NORMAL, &GLASS_HOT, &GLASS_PRESSED],
        true,
        &[State::Normal, State::Hot, State::Pressed],
        d,
    );
    let (light, light_arrow) = shutdown(&[&LIGHT], false, &[State::Normal], d);
    let hot_right = if d == 1 {
        hot_right().clone()
    } else {
        draw_hot_right(d)
    };
    vec![
        (Part::StartPane, one(pane(d), PANE_GRID, d)),
        (Part::StartJumpPane, one(jump_pane(d), JUMP_GRID, d)),
        (Part::StartHotLeft, row_part(&HOT_LEFT, 36, d)),
        (Part::StartNewPin, row_part(&NEW_PIN, 36, d)),
        (Part::StartHotAll, row_part(&HOT_ALL, 23, d)),
        (Part::StartHotSplit, one(hot_split(d), none, d)),
        (Part::StartHotJump, row_part(&HOT_JUMP, 20, d)),
        (Part::StartLitResult, row_part(&LIT_RESULT, 22, d)),
        (Part::StartHotRight, one(hot_right, HOT_RIGHT_GRID, d)),
        (
            Part::StartRuleLeft,
            one(rule(&[(255.0, RULE_LEFT)]), none, 1),
        ),
        (Part::StartRuleRight, one(rule(&RULE_RIGHT), none, 1)),
        (
            Part::StartJumpRule,
            one(rule(&[(255.0, RULE_JUMP)]), none, 1),
        ),
        (
            Part::StartSearchRule,
            one(rule(&[(255.0, RULE_SEARCH)]), none, 1),
        ),
        (Part::StartShutdown, glass),
        (Part::StartShutdownArrow, glass_arrow),
        (Part::StartShutdownLight, light),
        (Part::StartShutdownArrowLight, light_arrow),
        (Part::StartSearchField, one(field(d), FIELD_GRID, d)),
        (
            Part::StartMagnifier,
            one(
                glyph_canvas(&glyph::MAGNIFIER, &MAGNIFIER_INK, |_| MAGNIFIER_LEVEL, d),
                none,
                d,
            ),
        ),
        (
            Part::StartClear,
            one(
                glyph_canvas(&glyph::CLEAR, &CLEAR_INK, |_| CLEAR_LEVEL, d),
                none,
                d,
            ),
        ),
        (
            Part::StartTriangleRight,
            one(
                glyph_canvas(
                    &glyph::TRIANGLE_RIGHT,
                    &BLACK,
                    |x| triangle_level(x, TRIANGLE_RIGHT_DEEP_AT),
                    d,
                ),
                none,
                d,
            ),
        ),
        (
            Part::StartTriangleLeft,
            one(
                glyph_canvas(
                    &glyph::TRIANGLE_LEFT,
                    &BLACK,
                    |x| triangle_level(x, TRIANGLE_LEFT_DEEP_AT),
                    d,
                ),
                none,
                d,
            ),
        ),
        (
            Part::StartRowArrow,
            one(glyph_canvas(&glyph::ARROW, &BLACK, |_| 1.0, d), none, d),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glass::panel::DENSE_CEILING;
    use crate::glass::{Material, SWATCHES, tint};
    use crate::raster::Rect;

    fn fnv(c: &Canvas) -> u64 {
        let words = [c.width, c.height]
            .into_iter()
            .chain(c.pixels.iter().copied());
        words.fold(0xcbf2_9ce4_8422_2325, |h, w| {
            w.to_le_bytes().iter().fold(h, |h, &b| {
                (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
            })
        })
    }

    fn find(parts: &[(Part, PartImage)], part: Part) -> &PartImage {
        &parts.iter().find(|(p, _)| *p == part).unwrap().1
    }

    fn settings() -> Vec<Settings> {
        let ruby = SWATCHES.iter().find(|w| w.name == "ruby").unwrap().color;
        [SWATCHES[0].color, ruby]
            .into_iter()
            .flat_map(|color| {
                Material::ALL.into_iter().flat_map(move |material| {
                    [42, 75, 100].map(|intensity| Settings {
                        color,
                        material,
                        intensity,
                        ..Settings::default()
                    })
                })
            })
            .collect()
    }

    #[test]
    fn sky_keeps_its_pieces() {
        let all = parts(&Settings::default(), 1);
        let got: Vec<(Part, u64)> = all.iter().map(|(p, img)| (*p, fnv(&img.atlas))).collect();
        let want = [
            (Part::StartPane, 0x544A6E607CED9508),
            (Part::StartJumpPane, 0xD3A4E96BFCB874C2),
            (Part::StartHotLeft, 0xAA034D21DD7E0B6F),
            (Part::StartNewPin, 0xD4B21638B523539B),
            (Part::StartHotAll, 0xB3A1B5EDAACA72BC),
            (Part::StartHotSplit, 0x4D387B1E527EED8A),
            (Part::StartHotJump, 0x7056990E3B349293),
            (Part::StartLitResult, 0x889492F524D9CF09),
            (Part::StartHotRight, 0x6F09C6DBF227EB5E),
            (Part::StartRuleLeft, 0xAD208DCB7E0BAFC2),
            (Part::StartRuleRight, 0xB0471F39865DD4EF),
            (Part::StartJumpRule, 0x3EFD371CBD34EB26),
            (Part::StartSearchRule, 0x8995D65A7C608E97),
            (Part::StartShutdown, 0x4143E6CE7819454E),
            (Part::StartShutdownArrow, 0x06C7A85E70B87A32),
            (Part::StartShutdownLight, 0xBD0E28B65C05E584),
            (Part::StartShutdownArrowLight, 0xDF3EAA40EB2C25B4),
            (Part::StartSearchField, 0xEAF423D1C5EDE2EB),
            (Part::StartMagnifier, 0xBA3472476B3D9692),
            (Part::StartClear, 0xCB3E3A5441D037E5),
            (Part::StartTriangleRight, 0x2AAE1B4A1AA170FE),
            (Part::StartTriangleLeft, 0x54F4A166AE99E375),
            (Part::StartRowArrow, 0xF426C0D0415635C6),
            (Part::StartPictureFrame, 0xE79C46D27F497A74),
        ];
        assert_eq!(got, want, "the parts' atlases, hashed");
        for (p, x, y, want, what) in [
            (Part::StartPane, 7, 5, 0xFFFF_FFFFu32, "the pane's white"),
            (Part::StartPane, 7, 8, 0xFFCA_D6E9, "the band's rule"),
            (Part::StartPane, 7, 30, 0xFFF0_F4FB, "the band"),
            (Part::StartPane, 0, 20, 0x3D3D_3D3D, "the pane's light line"),
            (Part::StartHotLeft, 1, 1, 0xFF9F_B8DD, "a lit row's corner"),
            (Part::StartHotLeft, 0, 18, 0xFF7A_9BCD, "its outline"),
            (
                Part::StartHotLeft,
                5,
                2,
                0xFFDC_EAFD,
                "its fill's first row",
            ),
            (
                Part::StartHotRight,
                69,
                1,
                0xFF00_0000,
                "a lit place's outline",
            ),
            (Part::StartHotRight, 69, 3, 0x5252_5252, "its face's top"),
            (
                Part::StartShutdown,
                36,
                1,
                0xB5B5_B5B5,
                "Shut down's light line",
            ),
            (Part::StartShutdown, 36, 2, 0x847E_7F7F, "its upper band"),
            (Part::StartShutdown, 36, 11, 0x2C00_0000, "its lower band"),
            (
                Part::StartPictureFrame,
                20,
                4,
                0xDCCA_D3D7,
                "the frame's upper body",
            ),
            (
                Part::StartPictureFrame,
                6,
                40,
                0x934D_5B69,
                "its lower body",
            ),
            (Part::StartPictureFrame, 59, 30, 0xF413_AACD, "its cyan rim"),
            (Part::StartPictureFrame, 30, 30, 0, "the picture's hole"),
            (
                Part::StartMagnifier,
                4,
                4,
                0xD900_4F95,
                "the magnifier's stroke",
            ),
        ] {
            let img = find(&all, p);
            let f = img.frame(State::Normal);
            let got = img.atlas.get(f.x + x, f.y + y);
            assert_eq!(got, want, "{p:?} ({x}, {y}): {what}");
        }
    }

    #[test]
    fn the_frames_body_moves_as_the_plain_glass_its_sheen_kept_inside_the_alpha() {
        let sky = Settings::default();
        let from = picture_frame(&sky, 1);
        let (a_sky, p_sky) = tint::plain(&sky, Surface::Active);
        let body = [(6, 10), (6, 15), (6, 40), (30, 56)];
        let (mut capped, mut scaled) = (0, 0);
        for s in settings() {
            let what = format!("{:?} {:?} at {}", s.color, s.material, s.intensity);
            let frame = picture_frame(&s, 1);
            let (a_s, p_s) = tint::plain(&s, Surface::Active);
            for (x, y) in body {
                let (a0, c0) = tint::split(from.get(x, y));
                let (a1, c1) = tint::split(frame.get(x, y));
                let added = a0 - a_sky;
                assert!(
                    a_sky + added < DENSE_CEILING,
                    "Sky is not capped at ({x}, {y})"
                );
                let alpha = if a_s + added > DENSE_CEILING {
                    capped += 1;
                    a_s.max(DENSE_CEILING)
                } else {
                    a_s + added
                };
                assert!(
                    (a1 - alpha).abs() <= 1.0,
                    "{what} ({x}, {y}): alpha {a1}, want {alpha}"
                );
                let sky_adds: [f64; 3] = std::array::from_fn(|i| c0[i] - p_sky[i]);
                assert!(
                    sky_adds.iter().any(|&v| v > 20.0),
                    "Sky's sheen whitens ({x}, {y})"
                );
                let adds = std::array::from_fn(|i| c1[i] - p_s[i]);
                if panel::sheen_share(&format!("{what} ({x}, {y})"), sky_adds, adds) < 0.98 {
                    scaled += 1;
                }
            }
        }
        assert!(
            capped > 0,
            "intensity 100 takes the frame's sheen to the ceiling"
        );
        assert!(scaled > 0, "some setting keeps the sheen's colour down");
    }

    #[test]
    fn no_setting_clips_the_frames_colour() {
        let mut clipped = Vec::new();
        for s in panel::every_setting() {
            let mut worst: f64 = 0.0;
            framed(&s, 1, &mut |p| {
                worst = worst.max(panel::clip(p));
                pack(p)
            });
            if worst > 1e-9 {
                clipped.push(format!(
                    "{:?} {:?} {}: {worst:.1}",
                    s.color, s.material, s.intensity
                ));
            }
        }
        assert!(clipped.is_empty(), "{clipped:#?}");
    }

    #[test]
    fn the_frames_rim_takes_the_glass_colours_hue() {
        let ruby = SWATCHES.iter().find(|w| w.name == "ruby").unwrap();
        let rim = |s: &Settings| {
            let (a, p) = tint::split(picture_frame(s, 1).get(59, 30));
            p.map(|v| v * 255.0 / a)
        };
        let [r, g, b] = rim(&Settings::default());
        assert!(
            g > r + 60.0 && b > r + 60.0,
            "Sky's rim is cyan: ({r}, {g}, {b})"
        );
        let red = Settings {
            color: ruby.color,
            intensity: ruby.intensity,
            ..Settings::default()
        };
        let [r, g, b] = rim(&red);
        assert!(
            r > g + 60.0 && r > b + 60.0,
            "Ruby's is red: ({r}, {g}, {b})"
        );
    }

    #[test]
    fn with_transparency_off_the_frame_is_opaque_and_its_shadow_is_not() {
        let (fx, fy, side) = FRAME_RECT;
        for s in settings() {
            let opaque_s = Settings {
                transparency: false,
                ..s
            };
            let (c, glass) = (picture_frame(&opaque_s, 1), picture_frame(&s, 1));
            for y in 0..FRAME.1 {
                for x in 0..FRAME.0 {
                    let on = rounded(
                        f64::from(x - fx),
                        f64::from(y - fy),
                        f64::from(side),
                        f64::from(side),
                        FRAME_R,
                    );
                    let (hx, hy, hole) = HOLE;
                    let in_hole = (hx..hx + hole).contains(&x) && (hy..hy + hole).contains(&y);
                    let what = format!(
                        "{:?} {:?} at {} ({x}, {y})",
                        s.color, s.material, s.intensity
                    );
                    if in_hole {
                        assert_eq!(c.get(x, y), 0, "{what}: the hole is clear");
                    } else if on == 1.0 {
                        assert_eq!(c.get(x, y) >> 24, 0xFF, "{what}: opaque");
                    } else if on == 0.0 {
                        assert_eq!(c.get(x, y), glass.get(x, y), "{what}: the shadow");
                        assert!(c.get(x, y) >> 24 < 0xFF, "{what}: the shadow");
                    }
                }
            }
        }
    }

    fn round_corners(part: Part, w: i32, h: i32) -> Option<(Rect, f64)> {
        let all = Rect::new(0, 0, w, h);
        Some(match part {
            Part::StartPanel => (all, 7.0),
            Part::StartPane => (all, PANE_R),
            Part::StartJumpPane => (Rect::new(-w, 0, 2 * w, 2 * h), PANE_R - 2.0),
            Part::StartHotLeft
            | Part::StartNewPin
            | Part::StartHotAll
            | Part::StartHotJump
            | Part::StartLitResult => (all, 3.0),
            Part::StartHotRight => (all, HOT_RIGHT_R),
            Part::StartShutdown | Part::StartShutdownLight => {
                (Rect::new(0, 0, SHUT_W + ARROW_W, h), SHUT_R)
            }
            Part::StartShutdownArrow | Part::StartShutdownArrowLight => {
                (Rect::new(-SHUT_W, 0, SHUT_W + ARROW_W, h), SHUT_R)
            }
            Part::StartSearchField => (all, FIELD_R),
            Part::StartPictureFrame => {
                let (x, y, side) = FRAME_RECT;
                (Rect::new(x, y, side, side), FRAME_R)
            }
            _ => return None,
        })
    }

    #[test]
    fn every_piece_at_scale_2_is_its_scale_1_design_twice_over() {
        let s = Settings::default();
        let style = crate::glass::style(&s);
        let two = style.at_scale(2);
        for (p, img) in crate::glass::startpanel::parts(&s, 1) {
            if matches!(
                p,
                Part::StartMagnifier
                    | Part::StartClear
                    | Part::StartTriangleRight
                    | Part::StartTriangleLeft
                    | Part::StartRowArrow
            ) {
                continue;
            }
            for &st in &img.states {
                let f = img.frame(st);
                let (w, h) = (f.width, f.height);
                let mut one = Canvas::new(w as u32, h as u32);
                style.draw(&mut one, Rect::new(0, 0, w, h), p, st, 1);
                let mut big = Canvas::new(2 * w as u32, 2 * h as u32);
                two.draw(&mut big, Rect::new(0, 0, 2 * w, 2 * h), p, st, 2);
                let corners = round_corners(p, w, h).map(|(r, rad)| (r.scaled(2), rad));
                let glyph = match p {
                    Part::StartShutdownArrow => Some(ARROW_AT[0]),
                    Part::StartShutdownArrowLight => Some(ARROW_AT[1]),
                    _ => None,
                };
                let near_glyph = glyph.map(|(gx, gy)| Rect::new(gx - 2, gy - 2, 8, 11).scaled(2));
                let what = format!("{p:?} {st:?}");
                panel::assert_twice_over(&what, &one, &big, |x, y| {
                    corners.is_some_and(|(r, rad)| panel::in_corner(r, rad, x, y))
                        || near_glyph.is_some_and(|g| g.contains(x, y))
                });
                if let Some((gx, gy)) = glyph {
                    let split = (gy..gy + 7)
                        .flat_map(|y| (gx..gx + 4).map(move |x| (x, y)))
                        .filter(|&(x, y)| {
                            let b = [(0, 0), (1, 0), (0, 1), (1, 1)]
                                .map(|(dx, dy)| big.get(2 * x + dx, 2 * y + dy));
                            b.iter().any(|&v| v != b[0])
                        })
                        .count();
                    assert!(split > 0, "{what}: the arrow is drawn at 2");
                }
            }
        }
    }

    #[test]
    fn the_glyphs_are_drawn_at_each_scale() {
        let s = Settings::default();
        let glyphs = [
            Part::StartMagnifier,
            Part::StartClear,
            Part::StartTriangleRight,
            Part::StartTriangleLeft,
            Part::StartRowArrow,
        ];
        let one = crate::glass::startpanel::parts(&s, 1);
        let two = crate::glass::startpanel::parts(&s, 2);
        let find = |parts: &[(Part, PartImage)], p: Part| {
            parts
                .iter()
                .find(|q| q.0 == p)
                .map(|q| q.1.clone())
                .unwrap()
        };
        for p in glyphs {
            let (a, b) = (find(&one, p), find(&two, p));
            let (fa, fb) = (a.frame(State::Normal), b.frame(State::Normal));
            assert_eq!(
                (b.scale, fb.width, fb.height),
                (2, 2 * fa.width, 2 * fa.height),
                "{p:?}"
            );
            let alphas: Vec<u32> = (0..fb.height)
                .flat_map(|y| (0..fb.width).map(move |x| (x, y)))
                .map(|(x, y)| b.atlas.get(fb.x + x, fb.y + y) >> 24)
                .collect();
            assert!(
                alphas.iter().any(|&a| a > 0 && a < 0xD0),
                "{p:?}: soft edges at 2"
            );
            assert!(
                alphas.iter().any(|&a| a >= 0xB0),
                "{p:?}: a solid stroke at 2"
            );
        }
    }

    #[test]
    fn a_lit_row_and_the_picture_frame_at_scale_2_round_their_corners() {
        let c = panel::light_box(&HOT_LEFT, BOX_W, 36, 2);
        panel::assert_round_corner("a lit row", &c, (0, 0), 6.0, 0xFF, 4);
        let c = draw_hot_right(2);
        panel::assert_round_corner("a lit place", &c, (0, 0), 11.0, 0x20, 6);
        let mut frame = picture_frame(&Settings::default(), 2);
        let shadow = panel::shadow(&FRAME_SHADOW.at(2), 2 * FRAME_RECT.2, 2 * FRAME_RECT.2);
        for (px, sh) in frame.pixels.iter_mut().zip(&shadow.pixels) {
            if px == sh {
                *px = 0;
            }
        }
        let (x, y, _) = FRAME_RECT;
        panel::assert_round_corner("the frame", &frame, (2 * x, 2 * y), 14.0, 0, 8);
    }

    #[test]
    fn sky_keeps_its_pieces_at_scale_2() {
        let got: Vec<(Part, u64)> = crate::glass::startpanel::parts(&Settings::default(), 2)
            .iter()
            .filter(|(_, img)| img.scale == 2)
            .map(|(p, img)| (*p, fnv(&img.atlas)))
            .collect();
        let want = [
            (Part::StartPanel, 0xC3F3B37D5AD0B999),
            (Part::StartPane, 0x73BC18D20127D203),
            (Part::StartJumpPane, 0xDED370D4D39A66E6),
            (Part::StartHotLeft, 0x1D1A4F02473A9CCD),
            (Part::StartNewPin, 0xC389537DB479EC69),
            (Part::StartHotAll, 0xC6DDB7FCEEC19FD3),
            (Part::StartHotSplit, 0x2051EC004C21C3E3),
            (Part::StartHotJump, 0x5B3481A242D30755),
            (Part::StartLitResult, 0x26CD6856171441F9),
            (Part::StartHotRight, 0x25C03E5A8666E764),
            (Part::StartShutdown, 0x5075CA8DCB1E8672),
            (Part::StartShutdownArrow, 0xB2118744A9D8028F),
            (Part::StartShutdownLight, 0xA0905F93840A71BA),
            (Part::StartShutdownArrowLight, 0xC9944BFDC683030E),
            (Part::StartSearchField, 0xB73B34BF1531AA5B),
            (Part::StartMagnifier, 0xAD6FFDFCFEABBA3B),
            (Part::StartClear, 0x703DE9DE6F81F83D),
            (Part::StartTriangleRight, 0xE0AEB6AB5B97A9BF),
            (Part::StartTriangleLeft, 0x77ABAFD65849821F),
            (Part::StartRowArrow, 0x5E5ADFF370831EE3),
            (Part::StartPictureFrame, 0x036214A0CF0451DB),
        ];
        assert_eq!(got, want, "the pieces' atlases at 2, hashed");
    }

    #[test]
    fn a_lit_row_box_is_opaque_inside_its_rounded_corners() {
        for (b, h) in [
            (HOT_LEFT, 36),
            (HOT_ALL, 23),
            (HOT_JUMP, 20),
            (LIT_RESULT, 22),
            (NEW_PIN, 36),
        ] {
            let c = panel::light_box(&b, BOX_W, h, 1);
            for y in 0..h {
                for x in 0..10 {
                    let corner = !(3..=6).contains(&x) && !(3..h - 3).contains(&y);
                    if !corner {
                        assert_eq!(c.get(x, y) >> 24, 0xFF, "({x}, {y}) of a box {h} tall");
                    }
                    assert_eq!(c.get(x, y), c.get(9 - x, y), "mirrored at ({x}, {y})");
                }
            }
            assert!(c.get(0, 0) >> 24 < 0x80, "its corner is mostly clear");
            let (o, _) = b.outline;
            let outline = 0xFF00_0000 | u32::from(o.r) << 16 | u32::from(o.g) << 8 | u32::from(o.b);
            assert_eq!(c.get(0, h / 2), outline, "its outline");
        }
    }
}
