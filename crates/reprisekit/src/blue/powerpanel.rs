//! The Log Off and Turn off computer panels in Blue: dark bands top and bottom in the scheme's
//! hue, a lit band between them, the title in large white text beside the logo, and each big
//! button a glossy rounded tile with a white mark. Cancel is the look's push button, drawn by
//! `interfaces::draw_power_panel`.

use std::rc::Rc;

use crate::art::logo;
use crate::blue::Variant;
use crate::cues::Cues;
use crate::pointer::{Interaction, Lit, lit_of};
use crate::powerpanel::{BORDER, Choice, ICON, Layout, Panel, Part, SEPARATOR};
use crate::raster::{Canvas, Rect};
use crate::shape::{
    self, CLEAR, Px, Stops, centre, ellipse, over, polygon, premul, radial, rounded, shares, stroke,
};
use crate::style::TextSpec;
use crate::text::{Align, TextRenderer};
use crate::theme::Color;

const WHITE: Color = Color::rgb(0xFF, 0xFF, 0xFF);
const BLACK: Color = Color::rgb(0x00, 0x00, 0x00);
const GREYED_LABEL: Color = Color::rgb(0xA0, 0xA0, 0xA0);

/// The bands' lightness (HSL) in the scheme's title hue, and the lit band's saturation against
/// the title's: the dark bands, the lit band's rim and its brightest spot.
const BAND_L: f64 = 0.30;
const LIT_EDGE: (f64, f64) = (0.67, 0.61);
const LIT_PEAK: (f64, f64) = (0.85, 0.79);
/// Where the lit band's brightest spot sits across and down it, and its reach as shares of the
/// band's width and height.
const LIT_CENTRE: (f64, f64) = (0.59, 0.38);
const LIT_REACH: (f64, f64) = (0.55, 1.5);
/// Where along the separator its light peaks.
const SEPARATOR_PEAK: f64 = 0.42;
const TITLE_INSET: i32 = 12;
const LOGO: i32 = 32;
const LOGO_RIGHT: i32 = 43;

struct Inks {
    band: Color,
    edge: Color,
    peak: Color,
}

fn inks(v: Variant) -> Inks {
    let (h, s, _) = hsl_of(v.scheme().system.active_title);
    Inks {
        band: hsl(h, s, BAND_L),
        edge: hsl(h, s * LIT_EDGE.0, LIT_EDGE.1),
        peak: hsl(h, s * LIT_PEAK.0, LIT_PEAK.1),
    }
}

fn hsl_of(c: Color) -> (f64, f64, f64) {
    let [r, g, b] = [c.r, c.g, c.b].map(|v| f64::from(v) / 255.0);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let l = (max + min) / 2.0;
    let d = max - min;
    if d == 0.0 {
        return (0.0, 0.0, l);
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs());
    let h = if max == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h * 60.0, s.min(1.0), l)
}

fn hsl(h: f64, s: f64, l: f64) -> Color {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0).rem_euclid(2.0) - 1.0).abs());
    let (r, g, b) = match (h / 60.0) as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    let ch = |v: f64| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    Color::rgb(ch(r), ch(g), ch(b))
}

fn mix(a: Color, b: Color, k: f64) -> Color {
    let m = |a: u8, b: u8| (f64::from(a) + (f64::from(b) - f64::from(a)) * k).round() as u8;
    Color::rgb(m(a.r, b.r), m(a.g, b.g), m(a.b, b.b))
}

fn solid(c: Color) -> Px {
    premul(c, 255)
}

/// The panel without its text, tiles and Cancel at `d` device px per logical px: a 1 px black
/// border round the title band, the separator, the lit band, a lighter line and the bottom band.
pub fn backdrop(v: Variant, lay: &Layout, d: i32) -> Canvas {
    let ink = inks(v);
    let (w, h) = lay.size;
    let (title, band) = (lay.title, lay.band);
    let sep_end = mix(ink.band, ink.edge, 0.45);
    let sep_peak = mix(ink.peak, WHITE, 0.6);
    let sep = [
        (0.0, solid(sep_end)),
        (SEPARATOR_PEAK, solid(sep_peak)),
        (1.0, solid(sep_end)),
    ];
    let lit = [(0.0, solid(ink.peak)), (1.0, solid(ink.edge))];
    let line = solid(mix(ink.band, ink.edge, 0.15));
    let k = f64::from(d);
    let (cx, cy) = (
        f64::from(band.x) + LIT_CENTRE.0 * f64::from(band.width),
        f64::from(band.y) + LIT_CENTRE.1 * f64::from(band.height),
    );
    let (rx, ry) = (
        LIT_REACH.0 * f64::from(band.width),
        LIT_REACH.1 * f64::from(band.height),
    );
    shape::draw(w, h, d, |x, y| {
        let (col, row) = (x as i32 / d, y as i32 / d);
        if col < BORDER || row < BORDER || col >= w - BORDER || row >= h - BORDER {
            return solid(BLACK);
        }
        if row >= title.bottom() && row < title.bottom() + SEPARATOR {
            return Stops(&sep).at((centre(x as i32, d) + 0.5) / f64::from(w));
        }
        if row >= band.y && row < band.bottom() {
            return radial(x, y, cx * k, cy * k, rx * k, ry * k, &Stops(&lit));
        }
        if row == band.bottom() {
            return line;
        }
        solid(ink.band)
    })
}

/// The tile's hue: each big button's own colour, the same in every scheme.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hue {
    Green,
    Orange,
    Red,
    Amber,
}

impl Hue {
    fn base(self) -> Color {
        match self {
            Hue::Green => Color::rgb(0x1F, 0x94, 0x1F),
            Hue::Orange => Color::rgb(0xE0, 0x93, 0x2A),
            Hue::Red => Color::rgb(0xEE, 0x5A, 0x3C),
            Hue::Amber => Color::rgb(0xE6, 0xAE, 0x1E),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    /// Two arrows, the top one pointing left and the bottom one right.
    Arrows,
    Key,
    /// A ring open at the top with a bar through the gap.
    Power,
    /// A whole ring with a bar inside it.
    Off,
    /// Eight dashes round the middle.
    Burst,
    Moon,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TileState {
    Normal,
    Focus,
    Hot,
    Pressed,
    Disabled,
}

pub const TILE_STATES: [TileState; 5] = [
    TileState::Normal,
    TileState::Focus,
    TileState::Hot,
    TileState::Pressed,
    TileState::Disabled,
];

pub fn look_of(c: Choice) -> (Hue, Mark) {
    match c {
        Choice::SwitchUser => (Hue::Green, Mark::Arrows),
        Choice::LogOff => (Hue::Orange, Mark::Key),
        Choice::StandBy => (Hue::Amber, Mark::Power),
        Choice::Hibernate => (Hue::Amber, Mark::Moon),
        Choice::TurnOff => (Hue::Red, Mark::Off),
        Choice::Restart => (Hue::Green, Mark::Burst),
    }
}

const TILE_RADIUS: f64 = 4.0;
pub(super) const RIM: (Color, u8) = (WHITE, 235);
/// The gloss's centre and radii, and its white at the centre.
const GLOSS: ((f64, f64), (f64, f64), u8) = ((9.0, 6.0), (15.0, 11.0), 110);
/// How far each state lifts the face towards white (negative: towards black).
const LIFT_FOCUS: f64 = 0.10;
const LIFT_HOT: f64 = 0.26;
const LIFT_PRESSED: f64 = -0.18;
/// How much of a greyed tile shows over the band: its face, and its rim and mark.
const GREYED_FACE: f64 = 0.22;
const GREYED_MARK: f64 = 0.45;
const MARK_W: f64 = 2.0;

/// A tile face's shading in `state`: its fill top to bottom, the gloss's white out from its centre,
/// and how much of the face, and of the rim and mark, a greyed tile keeps.
pub(super) struct Glaze {
    pub down: [(f64, Px); 3],
    pub gloss: [(f64, Px); 2],
    pub face_k: f64,
    pub mark_k: f64,
}

pub(super) fn glaze(hue: Hue, state: TileState) -> Glaze {
    let lift = match state {
        TileState::Focus => LIFT_FOCUS,
        TileState::Hot => LIFT_HOT,
        TileState::Pressed => LIFT_PRESSED,
        TileState::Normal | TileState::Disabled => 0.0,
    };
    let face = if lift >= 0.0 {
        mix(hue.base(), WHITE, lift)
    } else {
        mix(hue.base(), BLACK, -lift)
    };
    let gloss_a = if state == TileState::Pressed {
        GLOSS.2 / 2
    } else {
        GLOSS.2
    };
    let (face_k, mark_k) = match state {
        TileState::Disabled => (GREYED_FACE, GREYED_MARK),
        _ => (1.0, 1.0),
    };
    Glaze {
        down: [
            (0.0, solid(mix(face, WHITE, 0.30))),
            (0.55, solid(face)),
            (1.0, solid(mix(face, BLACK, 0.16))),
        ],
        gloss: [(0.0, premul(WHITE, gloss_a)), (1.0, CLEAR)],
        face_k,
        mark_k,
    }
}

/// A tile `size` design px square at `d` device px per design px: a big button's at [`ICON`],
/// the Blue Start menu's bottom buttons smaller. Marks scale with the tile; strokes stay whole px.
pub fn tile(hue: Hue, mark: Mark, state: TileState, size: i32, d: i32) -> Canvas {
    let Glaze {
        down,
        gloss,
        face_k,
        mark_k,
    } = glaze(hue, state);
    let k = f64::from(d);
    let n = f64::from(size);
    // Device px per design px of the 32 px tile the geometry is drawn in.
    let g = n / f64::from(ICON) * k;
    let pen = Pen {
        g,
        w: (MARK_W * g / k).round().max(1.0) * k,
    };
    let radius = TILE_RADIUS * g;
    shape::draw(size, size, d, |x, y| {
        let outer = rounded(x, y, n * k, n * k, radius);
        if outer == 0.0 {
            return CLEAR;
        }
        let inner = rounded(x - k, y - k, (n - 2.0) * k, (n - 2.0) * k, radius - k);
        let t = (centre(x as i32, d) + centre(y as i32, d)) / (2.0 * (n - 1.0));
        let shine = radial(
            x,
            y,
            GLOSS.0.0 * g,
            GLOSS.0.1 * g,
            GLOSS.1.0 * g,
            GLOSS.1.1 * g,
            &Stops(&gloss),
        );
        let fill = shape::scale(over(shine, Stops(&down).at(t)), face_k);
        let m = mark_cover(mark, x, y, pen);
        let fill = over(shape::scale(solid(WHITE), m * mark_k), fill);
        let rim = shape::scale(premul(RIM.0, RIM.1), mark_k);
        shares(&[(rim, outer - inner), (fill, inner)])
    })
}

/// `c`'s tile, `size` design px square.
pub fn icon(c: Choice, state: TileState, size: i32, d: i32) -> Canvas {
    let (hue, mark) = look_of(c);
    tile(hue, mark, state, size, d)
}

/// A mark's scale: `g` device px per design px of the 32 px tile, and `w` device px per stroke.
#[derive(Clone, Copy)]
struct Pen {
    g: f64,
    w: f64,
}

/// A ring `pen.w` wide inside radius `ro`.
fn ring(x: f64, y: f64, pen: Pen, (cx, cy): (f64, f64), ro: f64) -> f64 {
    let Pen { g, w } = pen;
    let o = ellipse(x, y, cx * g, cy * g, ro * g, ro * g);
    let i = ellipse(x, y, cx * g, cy * g, ro * g - w, ro * g - w);
    (o - i).max(0.0)
}

fn line(x: f64, y: f64, pen: Pen, pts: &[(f64, f64)]) -> f64 {
    let pts: Vec<_> = pts.iter().map(|&(a, b)| (a * pen.g, b * pen.g)).collect();
    stroke(x, y, &pts, pen.w)
}

/// How much of the device pixel at (`x`, `y`) the mark covers, its geometry in the 32 px tile's
/// design px.
fn mark_cover(mark: Mark, x: f64, y: f64, pen: Pen) -> f64 {
    let g = pen.g;
    let parts: Vec<f64> = match mark {
        Mark::Off => vec![
            ring(x, y, pen, (16.0, 16.0), 10.0),
            line(x, y, pen, &[(16.0, 10.0), (16.0, 22.0)]),
        ],
        Mark::Power => {
            let gap = [(16.0, 17.0), (9.0, 3.0), (23.0, 3.0)];
            let gap: Vec<_> = gap.iter().map(|&(a, b)| (a * g, b * g)).collect();
            vec![
                ring(x, y, pen, (16.0, 17.0), 9.0) * (1.0 - polygon(x, y, &gap)),
                line(x, y, pen, &[(16.0, 5.0), (16.0, 16.0)]),
            ]
        }
        Mark::Burst => (0..8)
            .map(|i| {
                let a = f64::from(i) * std::f64::consts::FRAC_PI_4;
                let (s, c) = a.sin_cos();
                let (r0, r1) = if i % 2 == 0 { (5.0, 11.0) } else { (5.0, 9.5) };
                line(
                    x,
                    y,
                    pen,
                    &[
                        (16.0 + c * r0, 16.0 + s * r0),
                        (16.0 + c * r1, 16.0 + s * r1),
                    ],
                )
            })
            .collect(),
        Mark::Arrows => vec![
            line(x, y, pen, &[(25.0, 11.0), (8.0, 11.0)]),
            line(x, y, pen, &[(13.0, 6.0), (8.0, 11.0), (13.0, 16.0)]),
            line(x, y, pen, &[(7.0, 21.0), (24.0, 21.0)]),
            line(x, y, pen, &[(19.0, 16.0), (24.0, 21.0), (19.0, 26.0)]),
        ],
        Mark::Key => vec![
            ring(x, y, pen, (20.0, 12.0), 6.5),
            ellipse(x, y, 21.5 * g, 10.5 * g, 1.5 * g, 1.5 * g),
            line(x, y, pen, &[(15.5, 16.5), (8.0, 24.0)]),
            line(x, y, pen, &[(11.0, 21.0), (13.5, 23.5)]),
            line(x, y, pen, &[(8.5, 23.5), (10.5, 25.5)]),
        ],
        Mark::Moon => vec![
            (ellipse(x, y, 15.0 * g, 17.0 * g, 10.0 * g, 10.0 * g)
                - ellipse(x, y, 20.0 * g, 12.5 * g, 8.5 * g, 8.5 * g))
            .max(0.0),
        ],
    };
    parts.into_iter().fold(0.0, f64::max).min(1.0)
}

/// The title's face: the first family at 20 px, else 18 px, each family's heaviest face.
pub fn title_text() -> TextSpec {
    TextSpec {
        family: vec![
            "Franklin Gothic Medium".into(),
            "Trebuchet MS".into(),
            "DejaVu Sans".into(),
        ],
        size_px: 18,
        family_sizes: vec![("Franklin Gothic Medium".into(), 20)],
        bold: true,
        italic: false,
        antialias: true,
        full_hinting: true,
        color: WHITE,
        inactive: WHITE,
        shadow: None,
        ink_gamma: 1.0,
    }
}

pub(super) fn title_renderer() -> Rc<TextRenderer> {
    thread_local! {
        static TITLE: Rc<TextRenderer> = Rc::new(TextRenderer::load(&title_text().font()));
    }
    TITLE.with(Rc::clone)
}

/// How big button `i` shows: greyed, held, under the pointer, focused or at rest.
pub fn tile_state(p: &Panel, i: usize, lit: Lit<Part>) -> TileState {
    if !p.live(Part::Big(i)) {
        return TileState::Disabled;
    }
    match lit_of(lit, Part::Big(i)) {
        Interaction::Pressed => TileState::Pressed,
        Interaction::Hot => TileState::Hot,
        Interaction::Idle if p.focus == Part::Big(i) => TileState::Focus,
        Interaction::Idle => TileState::Normal,
    }
}

/// Everything but Cancel onto `c`, a device-px canvas at scale 1 for `lay` at `s`; `text` is the
/// host's renderer for `s` and `px` its device size. Labels underline their access keys while
/// `cues` shows them.
#[expect(clippy::too_many_arguments)]
pub fn draw(
    c: &mut Canvas,
    text: &TextRenderer,
    v: Variant,
    p: &Panel,
    lay: &Layout,
    lit: Lit<Part>,
    cues: Cues,
    px: u32,
    s: i32,
) {
    c.blit_canvas(0, 0, &backdrop(v, lay, s));
    let u = lay.units;
    let title = lay.title;
    let spec = title_text();
    let t = title_renderer();
    t.for_output(s).draw(
        c,
        Rect::new(
            title.x + TITLE_INSET,
            title.y,
            title.width - TITLE_INSET - LOGO_RIGHT - 5,
            title.height,
        )
        .scaled(s),
        &p.title,
        spec.bold,
        spec.color,
        Align::Left,
        u.px(spec.size_in(&t)) * s as u32,
    );
    logo::draw_any(
        c,
        (title.right() - LOGO_RIGHT) * s,
        (title.y + (title.height - LOGO) / 2) * s,
        LOGO * s,
        WHITE,
    );
    for (i, cell) in lay.cells.iter().enumerate() {
        let Some((choice, on)) = p.shown(i) else {
            continue;
        };
        let (at, label) = lay.cell_parts(*cell);
        let (hue, mark) = look_of(choice);
        c.blit_over(
            at.x * s,
            at.y * s,
            &tile(hue, mark, tile_state(p, i, lit), ICON, s),
        );
        let ink = if on { WHITE } else { GREYED_LABEL };
        let label_text = if cues.keys {
            choice.label()
        } else {
            crate::text::split_mnemonic(&choice.label()).0
        };
        text.draw_marked(
            c,
            label.scaled(s),
            &label_text,
            true,
            ink,
            Align::Center,
            px,
            s,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::powerpanel::layout;
    use crate::theme::TextUnits;

    fn fnv(c: &Canvas) -> u64 {
        let mut x = 0xCBF2_9CE4_8422_2325_u64;
        let mut eat = |v: u32| {
            for b in v.to_le_bytes() {
                x = (x ^ u64::from(b)).wrapping_mul(0x0100_0000_01B3);
            }
        };
        eat(c.width);
        eat(c.height);
        for &p in &c.pixels {
            eat(p);
        }
        x
    }

    const MARKS: [(Hue, Mark); 6] = [
        (Hue::Green, Mark::Arrows),
        (Hue::Orange, Mark::Key),
        (Hue::Amber, Mark::Power),
        (Hue::Amber, Mark::Moon),
        (Hue::Red, Mark::Off),
        (Hue::Green, Mark::Burst),
    ];

    #[test]
    fn tiles_pin_at_scale_1_and_2() {
        let mut got = Vec::new();
        for (hue, mark) in MARKS {
            for d in [1, 2] {
                for st in TILE_STATES {
                    got.push((
                        format!("{hue:?} {mark:?} {d} {st:?}"),
                        fnv(&tile(hue, mark, st, ICON, d)),
                    ));
                }
            }
        }
        crate::pins::check("blue/power_tiles", &got);
    }

    #[test]
    fn backdrops_pin_per_scheme_at_scale_1_and_2() {
        let mut got = Vec::new();
        for v in [Variant::Blue, Variant::Silver, Variant::Olive] {
            for p in [Panel::log_off(false), Panel::turn_off(true, true)] {
                let lay = layout(&p, TextUnits::ONE);
                for d in [1, 2] {
                    got.push((
                        format!("{v:?} {} {d}", p.bigs.len()),
                        fnv(&backdrop(v, &lay, d)),
                    ));
                }
            }
        }
        crate::pins::check("blue/power_backdrop", &got);
    }

    fn alpha(p: u32) -> u32 {
        p >> 24
    }

    #[test]
    fn tiles_have_clear_corners_soft_curves_at_2_and_distinct_states() {
        for (hue, mark) in MARKS {
            for d in [1, 2] {
                let all: Vec<Canvas> = TILE_STATES
                    .iter()
                    .map(|&s| tile(hue, mark, s, ICON, d))
                    .collect();
                for (i, a) in all.iter().enumerate() {
                    let n = (ICON * d) as u32;
                    assert_eq!((a.width, a.height), (n, n), "{mark:?} is 32 design px");
                    assert!(
                        alpha(a.get(0, 0)) < 40,
                        "{mark:?} {:?} d={d}: the corner is clear",
                        TILE_STATES[i]
                    );
                    for b in &all[i + 1..] {
                        assert_ne!(a.pixels, b.pixels, "{mark:?} d={d}: states differ");
                    }
                }
                let normal = &all[0];
                let mid = 16 * d;
                assert_eq!(alpha(normal.get(mid, 3 * d)), 255, "{mark:?} is opaque");
                assert!(
                    alpha(all[4].get(mid, 3 * d)) < 128,
                    "{mark:?}: a greyed tile lets the band through"
                );
                if d == 2 {
                    let soft = (0..8)
                        .flat_map(|y| (0..8).map(move |x| (x, y)))
                        .filter(|&(x, y)| (1..255).contains(&alpha(normal.get(x, y))))
                        .count();
                    assert!(soft >= 4, "{mark:?}: the corner is smooth at 2 ({soft})");
                }
            }
        }
    }

    #[test]
    fn every_mark_inks_white_inside_its_tile() {
        for (hue, mark) in MARKS {
            let c = tile(hue, mark, TileState::Normal, ICON, 1);
            let white = c.pixels.iter().filter(|&&p| p == 0xFFFF_FFFF).count();
            assert!(white >= 12, "{mark:?} draws a white mark ({white} px)");
        }
    }

    #[test]
    fn the_bands_follow_the_scheme_and_double_at_2() {
        let lay = layout(&Panel::turn_off(true, false), TextUnits::ONE);
        let navy = backdrop(Variant::Blue, &lay, 1);
        let rgb = |p: u32| p & 0xFF_FFFF;
        assert_eq!(rgb(navy.get(0, 100)), 0, "the border is black");
        let band = rgb(navy.get(100, 20));
        assert_eq!(band, 0x00_31_99, "navy's dark band");
        assert_eq!(
            rgb(navy.get(100, 190)),
            band,
            "the bottom band is the title's"
        );
        let edge = rgb(navy.get(1, 100));
        let peak = rgb(navy.get(185, 88));
        let lum = |p: u32| ((p >> 16) & 0xFF) + ((p >> 8) & 0xFF) + (p & 0xFF);
        assert!(lum(peak) > lum(edge) + 100, "the band is lit off centre");
        let silver = backdrop(Variant::Silver, &lay, 1);
        assert_ne!(rgb(silver.get(100, 20)), band, "Silver has its own band");
        let two = backdrop(Variant::Blue, &lay, 2);
        assert_eq!((two.width, two.height), (628, 400));
        assert_eq!(rgb(two.get(1, 200)), 0, "the border is 2 device px at 2");
        assert_eq!(
            rgb(two.get(2, 200)),
            rgb(navy.get(1, 100)),
            "inside the border"
        );
    }
}
