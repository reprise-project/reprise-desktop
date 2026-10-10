//! Reprise Glass's Square frame (`[glass] frame = "square"`): square corners, flat caption buttons.

use super::glyph::{self, Glyph};
use super::shadow;
use crate::raster::{Canvas, Rect};
use crate::shape;
use crate::style::{Layout, Part, PartImage, State, Style};
use crate::theme::{CaptionGroup, Color, GroupPlace};

/// The caption buttons: minimize and maximize 26 px wide, close 45, no divider. Restored, the cells
/// are 21 rows: the frame's outline row over the 20 the buttons fill.
pub const CAPTION_GROUP: CaptionGroup = CaptionGroup {
    widths: [MIN_W, MIN_W, CLOSE_W],
    divider: 0,
    restored: GroupPlace {
        right: 6,
        top: 0,
        height: 21,
    },
    maximized: GroupPlace {
        right: 0,
        top: 0,
        height: 20,
    },
};

/// Minimize's and maximize's width, and a dialog's ? box's.
pub const MIN_W: i32 = 26;
/// Close's width.
pub const CLOSE_W: i32 = 45;
/// The rows a button fills.
pub const FILL_H: i32 = 20;

/// Close at rest on the focused window: a muted red.
pub const CLOSE_REST: Color = Color::rgb(0xC7, 0x50, 0x50);
/// Close under the pointer: a bright red.
pub const CLOSE_HOT: Color = Color::rgb(0xE0, 0x43, 0x43);
/// Close held: a dark red.
pub const CLOSE_PRESSED: Color = Color::rgb(0x99, 0x3D, 0x3D);
/// Close's glyph on its red.
pub const CLOSE_GLYPH: Color = Color::rgb(0xFF, 0xFF, 0xFF);
/// Minimize, maximize or ? under the pointer: white at this alpha.
pub const HOT_ALPHA: u8 = 0x55;
/// Minimize, maximize or ? held: black at this alpha.
pub const PRESSED_ALPHA: u8 = 0x40;
/// A glyph's alpha on a window that is not the focused one.
pub const DIM_ALPHA: u8 = 0x80;
/// The title's edge colour round a glyph: at most this alpha, half of it on a window that is not
/// the focused one, falling off from the glyph's strokes as [`EDGE_SIGMA`] and [`EDGE_GAIN`] set
/// ([`edge_alpha`]).
pub const EDGE_ALPHA: u8 = 0x60;
/// How far a glyph's edge spreads: the sigma, in px, of the Gaussian its strokes are blurred by.
pub const EDGE_SIGMA: f32 = 0.8;
/// How strongly a glyph's blurred strokes saturate toward [`EDGE_ALPHA`].
pub const EDGE_GAIN: f32 = 3.0;

/// `st`, a Glass style built for the Square frame (caption band and bottom border already square):
/// square corners on the backdrop, the flat caption buttons and their row's geometry, a lone ? box
/// beside close, and the title centred.
pub fn frame(mut st: Style, n: i32) -> Style {
    let ink = Ink::of(&st);
    if let Some(g) = st.glass.as_mut() {
        g.corner = &[];
    }
    st.metrics.caption_group = Some(CAPTION_GROUP);
    st.joined_help = None;
    st.buttons_over_caption = true;
    st.centred_title = true;
    st.with_part(Part::CaptionGroup, group(&ink, false, n))
        .with_part(Part::MaxCaptionGroup, group(&ink, true, n))
        .with_part(Part::CloseButton, lone(&ink, Glyph::Close, n))
        .with_part(Part::HelpButton, lone(&ink, Glyph::Help, n))
}

struct Ink {
    active: u32,
    inactive: u32,
    edge: Option<Color>,
}

impl Ink {
    fn of(st: &Style) -> Ink {
        Ink {
            active: premul(st.caption_text.color, 0xFF),
            inactive: premul(st.caption_text.inactive, DIM_ALPHA),
            edge: st.caption_title.glow.map(|g| g.color),
        }
    }
}

fn premul(c: Color, a: u8) -> u32 {
    let m = |v: u8| (u32::from(v) * u32::from(a) + 127) / 255;
    u32::from(a) << 24 | m(c.r) << 16 | m(c.g) << 8 | m(c.b)
}

fn over(dst: u32, src: u32) -> u32 {
    let k = 255 - (src >> 24);
    let ch = |sh: u32| {
        let (s, d) = ((src >> sh) & 0xFF, (dst >> sh) & 0xFF);
        (s + (d * k + 127) / 255).min(255) << sh
    };
    ch(24) | ch(16) | ch(8) | ch(0)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Lit {
    Rest,
    Hot,
    Pressed,
}

fn button(c: &mut Canvas, r: Rect, g: Glyph, lit: Lit, active: bool, ink: &Ink, n: i32) {
    let close = g == Glyph::Close;
    let fill = match (close, lit) {
        (true, Lit::Rest) if active => premul(CLOSE_REST, 0xFF),
        (true, Lit::Rest) => 0,
        (true, Lit::Hot) => premul(CLOSE_HOT, 0xFF),
        (true, Lit::Pressed) => premul(CLOSE_PRESSED, 0xFF),
        (false, Lit::Rest) => 0,
        (false, Lit::Hot) => premul(Color::rgb(0xFF, 0xFF, 0xFF), HOT_ALPHA),
        (false, Lit::Pressed) => premul(Color::rgb(0, 0, 0), PRESSED_ALPHA),
    };
    let glyph = if close && fill != 0 {
        premul(CLOSE_GLYPH, 0xFF)
    } else if active || lit != Lit::Rest {
        ink.active
    } else {
        ink.inactive
    };
    for y in r.y..r.bottom() {
        for x in r.x..r.right() {
            c.put_raw(x, y, fill);
        }
    }
    let (gx, gy) = (
        r.x + n * ((r.width / n - 10) / 2),
        r.y + n * ((r.height / n - 10) / 2),
    );
    let mark = glyph::lines(g);
    // Pushed x-major, so `dots` is sorted for `edge_alpha`'s binary search.
    let mut dots = Vec::new();
    for dx in 0..mark.w * n {
        for dy in 0..mark.h * n {
            let k = mark.cover(dx, dy, n, 0.0);
            if k > 0.0 {
                dots.push((gx + dx, gy + dy, k));
            }
        }
    }
    if let Some(e) = ink.edge.filter(|_| fill >> 24 != 0xFF) {
        let peak = if active || lit != Lit::Rest {
            EDGE_ALPHA
        } else {
            EDGE_ALPHA / 2
        };
        for (x, y, a) in edge_alpha(&dots, r, peak, n) {
            c.put_raw(x, y, over(c.get(x, y), premul(e, a)));
        }
    }
    for (x, y, k) in dots {
        let ink = shape::pack(shape::scale(shape::unpack(glyph), k));
        c.put_raw(x, y, over(c.get(x, y), ink));
    }
}

/// Beside a straight stroke the edge comes to 0x42 of 0x60, further out or off a stroke's end much
/// less, so it shades the glyph without outlining it.
fn edge_alpha(dots: &[(i32, i32, f64)], r: Rect, peak: u8, n: i32) -> Vec<(i32, i32, u8)> {
    let reach = 2 * n;
    let sigma = EDGE_SIGMA * n as f32;
    let k = |d: i32| (-((d * d) as f32) / (2.0 * sigma * sigma)).exp();
    let norm: f32 = (-reach..=reach).map(k).sum();
    let at = |x: i32, y: i32| {
        dots.binary_search_by_key(&(x, y), |&(x, y, _)| (x, y))
            .map_or(0.0, |i| dots[i].2 as f32)
    };
    let mut out = Vec::new();
    for y in r.y..r.bottom() {
        for x in r.x..r.right() {
            let own = at(x, y);
            if own >= 1.0 {
                continue;
            }
            let mut blur = 0.0;
            for dy in -reach..=reach {
                for dx in -reach..=reach {
                    blur += k(dx) * k(dy) * at(x + dx, y + dy);
                }
            }
            let a = (f32::from(peak) * (EDGE_GAIN * blur / (norm * norm)).min(1.0) * (1.0 - own))
                .round() as u8;
            if a > 0 {
                out.push((x, y, a));
            }
        }
    }
    out
}

/// Atlas order, as `Part::CaptionGroup` lists it: the state and, for a one-button state, which
/// button is lit (0 minimize, 1 maximize, 2 close).
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

fn lit_of(state: State) -> (Lit, bool) {
    match state {
        State::Hot => (Lit::Hot, true),
        State::Pressed => (Lit::Pressed, true),
        State::Inactive => (Lit::Rest, false),
        State::InactiveHot => (Lit::Hot, false),
        _ => (Lit::Rest, true),
    }
}

/// The row carries no caption band: the frame lays it over the band
/// ([`Style::buttons_over_caption`]).
fn group(ink: &Ink, maximized: bool, n: i32) -> PartImage {
    let place = if maximized {
        CAPTION_GROUP.maximized
    } else {
        CAPTION_GROUP.restored
    };
    let (w, h) = (
        CAPTION_GROUP.widths.iter().sum::<i32>() * n,
        place.height * n,
    );
    let top = h - FILL_H * n;
    let glyphs = [
        Glyph::Minimize,
        if maximized {
            Glyph::Restore
        } else {
            Glyph::Maximize
        },
        Glyph::Close,
    ];
    let mut a = Canvas::new(w as u32, (h * GROUP_FRAMES.len() as i32) as u32);
    for (i, &(state, lit_one)) in GROUP_FRAMES.iter().enumerate() {
        let (lit, active) = lit_of(state);
        let mut x = 0;
        for (b, (&g, &bw)) in glyphs.iter().zip(&CAPTION_GROUP.widths).enumerate() {
            let l = if lit_one == Some(b) { lit } else { Lit::Rest };
            let r = Rect::new(x, i as i32 * h + top, bw * n, FILL_H * n);
            button(&mut a, r, g, l, active, ink, n);
            x += bw * n;
        }
    }
    let states: Vec<State> = GROUP_FRAMES.iter().map(|&(s, _)| s).collect();
    PartImage::new(a, &states, Layout::Vertical).at_scale(n)
}

fn lone(ink: &Ink, g: Glyph, n: i32) -> PartImage {
    const STATES: [State; 5] = [
        State::Normal,
        State::Inactive,
        State::Hot,
        State::Pressed,
        State::InactiveHot,
    ];
    let w = if g == Glyph::Close { CLOSE_W } else { MIN_W } * n;
    let (h, fill) = (CAPTION_GROUP.restored.height * n, FILL_H * n);
    let mut a = Canvas::new(w as u32, (h * STATES.len() as i32) as u32);
    for (i, &state) in STATES.iter().enumerate() {
        let (lit, active) = lit_of(state);
        let r = Rect::new(0, i as i32 * h + h - fill, w, fill);
        button(&mut a, r, g, lit, active, ink, n);
    }
    PartImage::new(a, &STATES, Layout::Vertical).at_scale(n)
}

/// Piece `i` of the active or inactive shadow around a square frame, as [`shadow::piece`] gives the
/// rounded frame's: edges are the same; each corner is the shades of its two sides multiplied, the
/// side running on past the corner into the frame as its outside falls away, and clear over the
/// frame. Each device pixel takes the shade at its middle, so a finer scale draws the corner anew.
pub fn shadow_piece(active: bool, i: usize, scale: i32) -> Canvas {
    if i >= shadow::TOP {
        return shadow::piece(active, i, scale);
    }
    let d = shadow::design(active);
    let m = d.margins;
    let piece = &shadow::shadow(active).pieces[i];
    let (w, h) = (piece.w, piece.h);
    let left = i == shadow::TOP_LEFT || i == shadow::BOTTOM_LEFT;
    let top = i == shadow::TOP_LEFT || i == shadow::TOP_RIGHT;
    let (ex, mx) = if left {
        (d.edge[0], m.left)
    } else {
        (d.edge[2], m.right)
    };
    let (ey, my) = if top {
        (d.edge[1], m.top)
    } else {
        (d.edge[3], m.bottom)
    };
    let s = scale.max(1);
    let k = f64::from(s);
    let mut c = Canvas::new((w * s) as u32, (h * s) as u32);
    for y in 0..h * s {
        let py = (f64::from(y) + 0.5) / k;
        let ty = if top {
            py - f64::from(my)
        } else {
            f64::from(h - my) - py
        };
        for x in 0..w * s {
            let px = (f64::from(x) + 0.5) / k;
            let tx = if left {
                px - f64::from(mx)
            } else {
                f64::from(w - mx) - px
            };
            let a = if tx > 0.0 && ty > 0.0 {
                0.0
            } else {
                factor(ex, f64::from(mx), tx, f64::from(w - mx))
                    * factor(ey, f64::from(my), ty, f64::from(h - my))
            };
            let a = (a * 255.0).round().clamp(0.0, 255.0) as u32;
            c.put_raw(x, y, a << 24);
        }
    }
    c
}

/// Inside the frame the shade is mirrored and eased out to full at the piece's end, so the edge
/// piece beyond meets it without a seam.
fn factor(edge: f64, reach: f64, t: f64, inside: f64) -> f64 {
    if t < 0.0 {
        return shadow::shade(edge, reach, -t);
    }
    let left = ((inside - 0.5 - t) / (inside - 1.0).max(1.0)).clamp(0.0, 1.0);
    1.0 - shadow::shade(edge, reach, t) * left
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{CaptionButton, Chrome, FrameGeometry, FrameHit, FrameState, Glow};
    use crate::glass::{Frame, Mode, Settings, style};
    use crate::style::frame::draw_frame;
    use crate::text::TextRenderer;
    use crate::theme::Theme;

    fn settings(mode: Mode) -> Settings {
        Settings {
            frame: Frame::Square,
            mode,
            ..Settings::default()
        }
    }

    fn theme(mode: Mode) -> Theme {
        crate::interfaces::theme_with(
            crate::interface::Interface::Glass,
            crate::blue::Variant::default(),
            &settings(mode),
        )
    }

    fn draw(
        t: &Theme,
        cw: i32,
        ch: i32,
        chrome: Chrome,
        st: &FrameState,
    ) -> (FrameGeometry, Canvas) {
        let s = t.look.style().unwrap();
        let text = TextRenderer::load(&s.caption_text.font());
        let g = FrameGeometry::layout(&t.metrics, cw, ch, st.maximized, chrome);
        let mut c = Canvas::new(g.total.0 as u32, g.total.1 as u32);
        draw_frame(&mut c, s, t, &text, &g, st);
        (g, c)
    }

    fn near(a: u32, b: u32) -> bool {
        (0..4).all(|i| ((a >> (8 * i)) & 0xFF).abs_diff((b >> (8 * i)) & 0xFF) <= 2)
    }

    fn state(title: &'static str, active: bool, maximized: bool) -> FrameState<'static> {
        FrameState {
            title,
            active,
            maximized,
            pressed: None,
            glow: Glow::lit(None),
            icon: None,
            tabbed: false,
        }
    }

    #[test]
    fn the_buttons_are_26_26_and_45_wide_hanging_from_the_top_edge() {
        let t = theme(Mode::Light);
        let g = FrameGeometry::layout(&t.metrics, 400, 200, false, Chrome::WINDOW);
        let w = g.total.0;
        assert_eq!(
            g.buttons,
            [
                Rect::new(w - 6 - 97, 0, 26, 21),
                Rect::new(w - 6 - 71, 0, 26, 21),
                Rect::new(w - 6 - 45, 0, 45, 21),
            ],
            "restored: 26, 26 and 45 px, 6 px in, from the frame's top"
        );
        let m = FrameGeometry::layout(&t.metrics, 400, 200, true, Chrome::WINDOW);
        let w = m.total.0;
        assert_eq!(
            m.buttons,
            [
                Rect::new(w - 97, 0, 26, 20),
                Rect::new(w - 71, 0, 26, 20),
                Rect::new(w - 45, 0, 45, 20),
            ],
            "maximized: in the screen's corner"
        );
        for (b, r) in CaptionButton::ALL.into_iter().zip(g.buttons) {
            assert_eq!(g.hit(r.x, r.y, &t.metrics), FrameHit::Button(b));
            assert_eq!(
                g.hit(r.right() - 1, r.bottom() - 1, &t.metrics),
                FrameHit::Button(b)
            );
        }
        let d = FrameGeometry::layout(&t.metrics, 300, 100, false, Chrome::HELP_DIALOG);
        assert_eq!(
            d.help_box(),
            Rect::new(d.buttons[2].x - 26, 0, 26, 21),
            "the ? box is 26 px, against close"
        );
    }

    #[test]
    fn close_is_muted_red_at_rest_bright_under_the_pointer_and_dark_held() {
        let t = theme(Mode::Light);
        let close = |st: &FrameState| {
            let (g, c) = draw(&t, 300, 60, Chrome::WINDOW, st);
            let r = g.buttons[2];
            c.get(r.x + 1, r.y + 2)
        };
        assert_eq!(close(&state("Notes", true, false)), CLOSE_REST.argb());
        let hot = FrameState {
            glow: Glow::lit(Some(CaptionButton::Close)),
            ..state("Notes", true, false)
        };
        assert_eq!(close(&hot), CLOSE_HOT.argb());
        let held = FrameState {
            pressed: Some(CaptionButton::Close),
            ..state("Notes", true, false)
        };
        assert_eq!(close(&held), CLOSE_PRESSED.argb());
        let idle = close(&state("Notes", false, false));
        assert_ne!(
            idle & 0xFF_FFFF,
            CLOSE_REST.argb() & 0xFF_FFFF,
            "unfocused, close rests bare over the glass"
        );
    }

    #[test]
    fn minimize_rests_bare_lights_and_darkens() {
        let t = theme(Mode::Light);
        let (g, rest) = draw(&t, 300, 60, Chrome::WINDOW, &state("Notes", true, false));
        let r = g.buttons[0];
        let (x, y) = (r.x + 1, r.y + 2);
        let hot = FrameState {
            glow: Glow::lit(Some(CaptionButton::Minimize)),
            ..state("Notes", true, false)
        };
        let (_, lit) = draw(&t, 300, 60, Chrome::WINDOW, &hot);
        let want = over(
            rest.get(x, y),
            premul(Color::rgb(0xFF, 0xFF, 0xFF), HOT_ALPHA),
        );
        assert!(
            near(lit.get(x, y), want),
            "lit: white at {HOT_ALPHA:#x} over the glass, {want:08x}, got {:08x}",
            lit.get(x, y)
        );
        let held = FrameState {
            pressed: Some(CaptionButton::Minimize),
            ..state("Notes", true, false)
        };
        let (_, held) = draw(&t, 300, 60, Chrome::WINDOW, &held);
        let want = over(rest.get(x, y), premul(Color::rgb(0, 0, 0), PRESSED_ALPHA));
        assert!(
            near(held.get(x, y), want),
            "held: black at {PRESSED_ALPHA:#x} over the glass, {want:08x}, got {:08x}",
            held.get(x, y)
        );
        let s = t.look.style().unwrap();
        let mut band = Canvas::new(g.total.0 as u32, g.client.y as u32);
        s.draw(
            &mut band,
            Rect::new(0, 0, g.total.0, g.client.y),
            Part::Caption,
            State::Normal,
            1,
        );
        assert_eq!(rest.get(x, y), band.get(x, y), "at rest, the bare band");
        let ink = premul(s.caption_text.color, 0xFF);
        let dash = r.y + 1 + 5 + 5;
        for dx in 8..18 {
            assert_eq!(
                rest.get(r.x + dx, dash),
                over(band.get(r.x + dx, dash), ink)
            );
        }
    }

    #[test]
    fn the_corners_are_square_and_opaque() {
        let t = theme(Mode::Light);
        let (g, c) = draw(&t, 300, 100, Chrome::WINDOW, &state("Notes", true, false));
        let (w, h) = g.total;
        let s = t.look.style().unwrap();
        for ((x, y), along) in [
            ((0, 0), (7, 0)),
            ((w - 1, 0), (w - 8, 0)),
            ((0, h - 1), (6, h - 1)),
            ((w - 1, h - 1), (w - 7, h - 1)),
        ] {
            assert!(c.get(x, y) >> 24 > 0xC0, "({x}, {y}) is drawn");
            assert_eq!(
                c.get(x, y),
                c.get(along.0, along.1),
                "({x}, {y}) is the outline"
            );
            assert!(
                crate::style::frame::frame_opaque_at(s, &g, true, false, x, y),
                "({x}, {y}) catches the pointer"
            );
        }
        assert_eq!(
            c.get(1, 1),
            c.get(7, 1),
            "the light line runs into the corner"
        );
        assert_eq!(
            s.glass.unwrap().corner,
            &[] as &[i32],
            "the backdrop is square"
        );
        let rounded = style(&Settings::default());
        assert_eq!(
            rounded.glass.unwrap().corner,
            super::super::CORNER,
            "Rounded keeps its own"
        );
    }

    /// The columns from `x0` to `x1` holding a dark title's ink: opaque, near black pixels, which
    /// the translucent band never has.
    fn inked(c: &Canvas, x0: i32, x1: i32) -> Vec<i32> {
        let ink = |px: u32| px >> 24 >= 0xF0 && (px >> 16) & 0xFF < 0x40;
        (x0..x1)
            .filter(|&x| (8..26).any(|y| ink(c.get(x, y))))
            .collect()
    }

    #[test]
    fn the_title_is_centred_over_the_frame() {
        let t = theme(Mode::Light);
        let (g, c) = draw(&t, 500, 60, Chrome::WINDOW, &state("Notes", true, false));
        let s = t.look.style().unwrap();
        let text = TextRenderer::load(&s.caption_text.font());
        let tw = text.measure("Notes", false, s.caption_text.size_px);
        let cols = inked(&c, 30, g.buttons[0].x);
        let (l, r) = (cols[0], *cols.last().unwrap());
        let mid = g.total.0 / 2;
        assert!(
            (mid - tw / 2 - l).abs() <= 2 && (r + 1 - (mid + tw - tw / 2)).abs() <= 2,
            "the {tw} px title spans {l}..={r}, centred on {mid}"
        );
    }

    #[test]
    fn a_title_too_long_to_centre_starts_after_the_icon_and_stops_at_the_buttons() {
        let t = theme(Mode::Light);
        let long = "A title far too long to sit centred in so narrow a window";
        let (g, c) = draw(&t, 260, 40, Chrome::WINDOW, &state(long, true, false));
        let first = g.buttons[0].x;
        let cols = inked(&c, 24, first);
        assert!(
            (30..33).contains(&cols[0]),
            "it starts at x 30, got {}",
            cols[0]
        );
        let last = *cols.last().unwrap();
        assert!(
            (first - 8..first).contains(&last),
            "it runs up to the first button at {first}, ending at {last}"
        );
    }

    #[test]
    fn unfocused_glyphs_are_dimmer() {
        let t = theme(Mode::Dark);
        let s = t.look.style().unwrap();
        let ink = Ink::of(s);
        assert_eq!(
            ink.active,
            s.caption_text.color.argb(),
            "focused: the title's ink"
        );
        assert_eq!(
            ink.inactive >> 24,
            u32::from(DIM_ALPHA),
            "unfocused: dimmed"
        );
        assert_eq!(
            ink.edge,
            s.caption_title.glow.map(|g| g.color),
            "shaded in the title's edge colour"
        );
    }

    #[test]
    fn a_glyph_is_shaded_in_the_titles_edge_colour() {
        let t = theme(Mode::Light);
        let s = t.look.style().unwrap();
        let edge = s.caption_title.glow.unwrap().color;
        let (g, c) = draw(&t, 300, 60, Chrome::WINDOW, &state("Notes", true, false));
        let mut band = Canvas::new(g.total.0 as u32, g.client.y as u32);
        let all = Rect::new(0, 0, g.total.0, g.client.y);
        s.draw(&mut band, all, Part::Caption, State::Normal, 1);
        let x = g.buttons[0].x + 13;
        for y in [10, 12] {
            let want = over(band.get(x, y), premul(edge, 0x42));
            assert!(
                near(c.get(x, y), want),
                "({x}, {y}): the edge at 0x42 over the band, {want:08x}, got {:08x}",
                c.get(x, y)
            );
        }
    }

    #[test]
    fn a_glyphs_edge_falls_off_softly() {
        let dots: Vec<(i32, i32, f64)> = (5..15).map(|x| (x, 5, 1.0)).collect();
        let a = edge_alpha(&dots, Rect::new(0, 0, 20, 11), EDGE_ALPHA, 1);
        let at = |x: i32, y: i32| a.iter().find(|p| (p.0, p.1) == (x, y)).map_or(0, |p| p.2);
        assert_eq!(at(10, 4), 0x42, "beside the stroke, 0x42");
        assert_eq!(at(10, 6), 0x42, "on both sides");
        assert_eq!(at(10, 3), 6, "2 px out, 6");
        assert_eq!(at(10, 2), 0, "nothing 3 px out");
        assert_eq!(at(15, 5), 36, "past its end, 36");
        assert_eq!(at(15, 4), 16, "diagonally off its end, 16: no box corner");
        assert!(
            a.iter()
                .all(|&(x, y, _)| !dots.iter().any(|d| (d.0, d.1) == (x, y))),
            "the edge stays off the glyph"
        );
    }

    #[test]
    fn a_glyphs_edge_at_scale_2_spreads_twice_as_far() {
        let dots: Vec<(i32, i32, f64)> = (10..30)
            .flat_map(|x| [(x, 10, 1.0), (x, 11, 1.0)])
            .collect();
        let a = edge_alpha(&dots, Rect::new(0, 0, 40, 22), EDGE_ALPHA, 2);
        let at = |x: i32, y: i32| a.iter().find(|p| (p.0, p.1) == (x, y)).map_or(0, |p| p.2);
        assert_eq!(
            [6, 7, 8, 9].map(|y| at(20, y)),
            [3, 16, 45, 92],
            "out from the stroke"
        );
        assert_eq!(at(20, 5), 0, "nothing 5 px out");
        for y in 6..10 {
            assert_eq!(at(20, y), at(20, 21 - y), "row {y}: both sides alike");
        }
        assert_eq!(at(30, 10), 49, "past its end");
        assert_eq!(at(31, 8), 8, "diagonally off its end");
        let side: u32 = (5..10).map(|y| u32::from(at(20, y))).sum();
        assert!(
            side.abs_diff(2 * (0x42 + 6)) <= 16,
            "the side sums to {side}, about twice scale 1's"
        );
    }

    #[test]
    fn the_buttons_at_scale_2_are_their_scale_1_design_twice_over() {
        let st = style(&settings(Mode::Light));
        let ink = Ink {
            edge: None,
            ..Ink::of(&st)
        };
        for (name, of) in [
            (
                "group",
                &(|n| group(&ink, false, n)) as &dyn Fn(i32) -> PartImage,
            ),
            ("maximized group", &|n| group(&ink, true, n)),
            ("close", &|n| lone(&ink, Glyph::Close, n)),
            ("?", &|n| lone(&ink, Glyph::Help, n)),
        ] {
            let (one, two) = (of(1), of(2));
            assert_eq!(two.scale, 2);
            let f = one.frame(State::Normal);
            let (w, h) = (f.width, f.height);
            for i in 0..one.states.len() as i32 {
                for y in 0..2 * h {
                    for x in 0..2 * w {
                        let what = format!("{name} frame {i} ({x}, {y})");
                        let (bx, by) = (x / 2 * 2, 2 * i * h + y / 2 * 2);
                        let block = [(0, 0), (1, 0), (0, 1), (1, 1)]
                            .map(|(dx, dy)| two.atlas.get(bx + dx, by + dy));
                        let (cx, cy) = (x / 2, i * h + y / 2);
                        let near = (-1..=1)
                            .flat_map(|dy| (-1..=1).map(move |dx| (dx, dy)))
                            .any(|(dx, dy)| {
                                one.atlas.get(cx + dx, cy + dy) != one.atlas.get(cx, cy)
                            });
                        if near && block.iter().any(|&v| v != block[0]) {
                            continue;
                        }
                        let pair = ((&one.atlas, i * h), (&two.atlas, 2 * i * h));
                        let check = (true, super::super::twice::ONE_KIND);
                        super::super::twice::twice_over(
                            pair.0,
                            pair.1,
                            (w, h),
                            (x, y),
                            check,
                            &what,
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_dialog_draws_its_lone_close_box_and_a_bare_question_mark() {
        let t = theme(Mode::Light);
        let (g, c) = draw(&t, 240, 60, Chrome::DIALOG, &state("Open", true, false));
        let r = g.buttons[2];
        assert_eq!(c.get(r.x + 1, r.y + 2), CLOSE_REST.argb(), "the close box");
        let m = g.buttons[0];
        let s = t.look.style().unwrap();
        let mut band = Canvas::new(g.total.0 as u32, g.client.y as u32);
        s.draw(
            &mut band,
            Rect::new(0, 0, g.total.0, g.client.y),
            Part::Caption,
            State::Normal,
            1,
        );
        for x in m.x..r.x {
            assert_eq!(
                c.get(x, 12),
                band.get(x, 12),
                "no minimize or maximize at x {x}"
            );
        }
        let help = s.part(Part::HelpButton).unwrap().frame(State::Normal);
        assert_eq!((help.width, help.height), (26, 21), "the ? box");
        assert_eq!(s.joined_help, None, "it stands alone");
    }

    #[test]
    fn the_square_shadow_is_clear_over_the_frame_and_meets_its_edges() {
        for active in [true, false] {
            let sh = shadow::shadow(active);
            let m = sh.margins;
            let tl = shadow_piece(active, shadow::TOP_LEFT, 1);
            assert_eq!(tl.get(m.left, m.top), 0, "clear over the frame's corner");
            assert!(tl.get(m.left - 1, m.top - 1) >> 24 > 0, "shaded outside it");
            let top = &sh.pieces[shadow::TOP];
            for y in 0..m.top {
                let got = tl.get(tl.width as i32 - 1, y) >> 24;
                let want = u32::from(top.get(0, y));
                assert!(
                    got.abs_diff(want) <= 1,
                    "active {active} row {y}: {got} vs {want}"
                );
            }
            let br = shadow_piece(active, shadow::BOTTOM_RIGHT, 1);
            let (bw, bh) = (br.width as i32, br.height as i32);
            assert_eq!(br.get(bw - m.right - 1, bh - m.bottom - 1), 0);
            let two = shadow_piece(active, shadow::TOP_RIGHT, 2);
            let one = shadow_piece(active, shadow::TOP_RIGHT, 1);
            assert_eq!(two.width, one.width * 2, "scale 2 is twice the size");
            let doubled = (0..two.height as i32)
                .flat_map(|y| (0..two.width as i32).map(move |x| (x, y)))
                .all(|(x, y)| two.get(x, y) == one.get(x / 2, y / 2));
            assert!(!doubled, "scale 2 draws its own pixels");
        }
        assert_eq!(
            shadow_piece(true, shadow::LEFT, 1).pixels,
            shadow::piece(true, shadow::LEFT, 1).pixels,
            "the edges are the rounded frame's"
        );
    }

    #[test]
    fn the_buttons_keep_their_pixels() {
        let fnv = |img: &PartImage| {
            img.atlas
                .pixels
                .iter()
                .fold(0xcbf2_9ce4_8422_2325_u64, |h, &p| {
                    p.to_le_bytes().iter().fold(h, |h, &b| {
                        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
                    })
                })
        };
        for (mode, wants) in [
            (
                Mode::Light,
                [
                    0x8979_797A_FF89_1B8D_u64,
                    0x03C3_EBD4_7434_6041,
                    0x5130_6DCE_6374_5C15,
                    0x9CBA_5FAD_5F30_7241,
                ],
            ),
            (
                Mode::Dark,
                [
                    0x426D_62AB_EFCA_08F5,
                    0x7A48_567F_7173_9AFE,
                    0x6337_A21E_5304_0775,
                    0x4273_373A_5E31_1896,
                ],
            ),
        ] {
            let s = style(&settings(mode));
            let parts = [
                Part::CaptionGroup,
                Part::MaxCaptionGroup,
                Part::CloseButton,
                Part::HelpButton,
            ];
            let got = parts.map(|part| fnv(s.part(part).unwrap()));
            assert_eq!(got, wants, "{mode:?}: {got:#018X?}");
        }
    }

    #[test]
    fn rounded_keeps_its_strip() {
        let r = style(&Settings::default());
        assert_eq!(r.metrics.caption_group, Some(super::super::CAPTION_GROUP));
        assert!(!r.buttons_over_caption && !r.centred_title);
        let s = style(&settings(Mode::Light));
        assert_eq!(s.metrics.caption_group, Some(CAPTION_GROUP));
        assert!(s.buttons_over_caption && s.centred_title);
    }
}
