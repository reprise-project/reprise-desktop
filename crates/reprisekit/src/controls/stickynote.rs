//! A frameless sticky note: a strip along the top holding New (+) and Delete (x), over a flat
//! multi-line edit the host draws into [`Layout::text`] with [`scheme`]'s colours, all inside a
//! one-pixel edge.

use crate::access::{self, Tree};
use crate::frame::{self, Edges, Grip};
use crate::paint::Paint;
use crate::raster::{Canvas, Rect};
use crate::shape::{self, Glyph};
use crate::theme::{Color, ColorScheme};
use accesskit::NodeId;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Colour {
    #[default]
    Yellow,
    Green,
    Pink,
    Purple,
    Blue,
    White,
}

impl Colour {
    /// In the order the colour menu lists them.
    pub const ALL: [Colour; 6] = [
        Colour::Yellow,
        Colour::Green,
        Colour::Pink,
        Colour::Purple,
        Colour::Blue,
        Colour::White,
    ];

    /// The name a note's file keeps it under.
    pub fn slug(self) -> &'static str {
        match self {
            Colour::Yellow => "yellow",
            Colour::Green => "green",
            Colour::Pink => "pink",
            Colour::Purple => "purple",
            Colour::Blue => "blue",
            Colour::White => "white",
        }
    }

    pub fn from_slug(s: &str) -> Option<Colour> {
        Colour::ALL.into_iter().find(|c| c.slug() == s)
    }

    pub fn label(self) -> String {
        match self {
            Colour::Yellow => tr!("note-yellow"),
            Colour::Green => tr!("note-green"),
            Colour::Pink => tr!("note-pink"),
            Colour::Purple => tr!("note-purple"),
            Colour::Blue => tr!("note-blue"),
            Colour::White => tr!("note-white"),
        }
    }

    pub fn body(self) -> Color {
        match self {
            Colour::Yellow => Color::rgb(0xFE, 0xF6, 0xA8),
            Colour::Green => Color::rgb(0xD2, 0xF2, 0xC4),
            Colour::Pink => Color::rgb(0xFA, 0xD4, 0xE2),
            Colour::Purple => Color::rgb(0xE0, 0xD6, 0xF6),
            Colour::Blue => Color::rgb(0xCE, 0xE6, 0xFA),
            Colour::White => Color::rgb(0xFA, 0xFA, 0xFA),
        }
    }

    pub fn strip(self) -> Color {
        match self {
            Colour::Yellow => Color::rgb(0xF6, 0xE7, 0x7C),
            Colour::Green => Color::rgb(0xB4, 0xE2, 0xA0),
            Colour::Pink => Color::rgb(0xF2, 0xB4, 0xCB),
            Colour::Purple => Color::rgb(0xC9, 0xB8, 0xEC),
            Colour::Blue => Color::rgb(0xAF, 0xD3, 0xF2),
            Colour::White => Color::rgb(0xE4, 0xE4, 0xE4),
        }
    }

    /// The line around the note, so two overlapping notes of one colour stay apart.
    pub fn edge(self) -> Color {
        shade(self.strip(), EDGE_SHADE)
    }
}

/// The text and the strip's glyphs.
pub const INK: Color = Color::rgb(0x2A, 0x26, 0x1E);
/// How much a lit and a held strip button darken the strip, out of 1.
const HOT_SHADE: f64 = 0.10;
const HELD_SHADE: f64 = 0.20;
const EDGE_SHADE: f64 = 0.25;
/// The edge's width, design px.
const EDGE: i32 = 1;

pub const STRIP_H: i32 = 20;
const BUTTON_W: i32 = 22;
/// The + and x glyphs' box, design px.
const GLYPH: i32 = 8;
/// The room between the body's left and top edges and the text.
const TEXT_PAD: i32 = 4;
/// The resize band along the note's edges, and how far each corner reaches along them.
const BAND: i32 = 5;
const CORNER: i32 = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    New,
    Delete,
}

/// The strip buttons' pointer state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Strip {
    pub hot: Option<Part>,
    /// The button the left press went down on; it acts if the release lands on it too.
    pub held: Option<Part>,
}

impl Strip {
    /// The pointer moved to `over`; true when that changed what draws.
    pub fn motion(&mut self, over: Option<Part>) -> bool {
        std::mem::replace(&mut self.hot, over) != over
    }

    /// A left press on `over`.
    pub fn press(&mut self, over: Option<Part>) -> bool {
        self.hot = over;
        self.held = over;
        over.is_some()
    }

    /// The left release on `over`: the button it clicked.
    pub fn release(&mut self, over: Option<Part>) -> Option<Part> {
        self.held.take().filter(|&p| Some(p) == over)
    }

    pub fn leave(&mut self) -> bool {
        self.hot.take().is_some()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub body: Rect,
    pub strip: Rect,
    pub new: Rect,
    pub delete: Rect,
    /// Where the host lays the multi-line edit out (flat, see `multiedit::Metrics::flat`).
    pub text: Rect,
}

pub fn layout(r: Rect) -> Layout {
    let strip = Rect::new(r.x, r.y, r.width, STRIP_H.min(r.height));
    let below = r.y + strip.height;
    Layout {
        body: r,
        strip,
        new: Rect::new(r.x, r.y, BUTTON_W, strip.height),
        delete: Rect::new(r.right() - BUTTON_W, r.y, BUTTON_W, strip.height),
        text: Rect::new(
            r.x + TEXT_PAD,
            below + TEXT_PAD,
            (r.width - TEXT_PAD - EDGE).max(0),
            (r.bottom() - below - TEXT_PAD - EDGE).max(0),
        ),
    }
}

/// The strip button at `(x, y)`; none in the resize band.
pub fn hit(lay: &Layout, x: i32, y: i32) -> Option<Part> {
    match grip(lay, x, y) {
        Some(_) => None,
        None => button(lay, x, y),
    }
}

/// What a left press at `(x, y)` grabs: the band along the edges resizes the note, the strip
/// outside its buttons moves it.
pub fn grip(lay: &Layout, x: i32, y: i32) -> Option<Grip> {
    let b = lay.body;
    let e = frame::band_edges((b.width, b.height), x - b.x, y - b.y, BAND, CORNER);
    if e != Edges::NONE {
        return Some(Grip::Resize(e));
    }
    (lay.strip.contains(x, y) && button(lay, x, y).is_none()).then_some(Grip::Move)
}

fn button(lay: &Layout, x: i32, y: i32) -> Option<Part> {
    if lay.new.contains(x, y) {
        Some(Part::New)
    } else if lay.delete.contains(x, y) {
        Some(Part::Delete)
    } else {
        None
    }
}

/// `cs` with the note's colours: the edit's box in the body colour, its text in [`INK`].
pub fn scheme(cs: &ColorScheme, colour: Colour) -> ColorScheme {
    ColorScheme {
        window: colour.body(),
        window_text: INK,
        button_text: INK,
        ..cs.clone()
    }
}

fn shade(c: Color, k: f64) -> Color {
    let f = |v: u8| (f64::from(v) * (1.0 - k)).round() as u8;
    Color::rgb(f(c.r), f(c.g), f(c.b))
}

/// The body in `r`, the strip with its buttons and the edge over both; the edit is the host's
/// to draw over `lay.text`, which stays clear of the edge.
pub fn draw(c: &mut Canvas, p: &Paint, r: Rect, colour: Colour, strip: Strip) {
    let lay = layout(r);
    c.fill_rect(r, colour.body());
    c.fill_rect(lay.strip, colour.strip());
    for (part, b, g) in [
        (Part::New, lay.new, Glyph::Plus),
        (Part::Delete, lay.delete, Glyph::Cross),
    ] {
        let k = match (strip.held, strip.hot) {
            (Some(h), Some(o)) if h == part && o == part => HELD_SHADE,
            (None, Some(o)) if o == part => HOT_SHADE,
            _ => 0.0,
        };
        if k > 0.0 {
            c.fill_rect(b, shade(colour.strip(), k));
        }
        let cs = c.scale();
        let img = shape::glyph(g, GLYPH, p.s * cs, 1.0, shape::premul(INK, 255)).at_scale(cs);
        let (x, y) = (
            b.x + (b.width - GLYPH * p.s) / 2,
            b.y + (b.height - GLYPH * p.s) / 2,
        );
        c.blit_over(x, y, &img);
    }
    let e = colour.edge();
    c.fill_rect(Rect::new(r.x, r.y, r.width, EDGE), e);
    c.fill_rect(Rect::new(r.x, r.bottom() - EDGE, r.width, EDGE), e);
    c.fill_rect(Rect::new(r.x, r.y, EDGE, r.height), e);
    c.fill_rect(Rect::new(r.right() - EDGE, r.y, EDGE, r.height), e);
}

/// The strip's two buttons under `parent`; the host adds the edit's node.
pub fn access(lay: &Layout, tree: &mut Tree, parent: NodeId) {
    tree.push(
        parent,
        "new",
        access::button(lay.new, &tr!("note-new"), true, false),
    );
    tree.push(
        parent,
        "delete",
        access::button(lay.delete, &tr!("note-delete"), true, false),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Theme;

    fn drawn(colour: Colour, strip: Strip, scale: i32) -> Canvas {
        let theme = Theme::default();
        let mut c = Canvas::with_scale(120, 60, scale);
        draw(
            &mut c,
            &Paint {
                s: 1,
                ..theme.paint()
            },
            Rect::new(0, 0, 120, 60),
            colour,
            strip,
        );
        c
    }

    fn hash(c: &Canvas) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for v in [c.width, c.height]
            .into_iter()
            .chain(c.pixels.iter().copied())
        {
            for b in v.to_le_bytes() {
                h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
            }
        }
        h
    }

    fn device(c: &Canvas, x: i32, y: i32) -> u32 {
        c.pixels[(y * c.width as i32 * c.scale() + x) as usize]
    }

    #[test]
    fn the_strip_holds_new_at_the_left_and_delete_at_the_right() {
        let lay = layout(Rect::new(0, 0, 200, 150));
        assert_eq!(hit(&lay, 8, 10), Some(Part::New));
        assert_eq!(hit(&lay, 190, 10), Some(Part::Delete));
        assert_eq!(hit(&lay, 100, 10), None, "the strip between them");
        assert_eq!(hit(&lay, 8, 40), None, "the text");
        assert_eq!(lay.text, Rect::new(4, 24, 195, 125));
    }

    #[test]
    fn the_strip_moves_the_note_and_the_band_along_its_edges_resizes_it() {
        let lay = layout(Rect::new(0, 0, 200, 150));
        let at = |x, y| grip(&lay, x, y);
        assert_eq!(
            at(100, 10),
            Some(Grip::Move),
            "the strip between the buttons"
        );
        assert_eq!(at(8, 10), None, "New");
        assert_eq!(at(190, 10), None, "Delete");
        assert_eq!(at(100, 80), None, "the text");
        assert_eq!(at(100, 2), Some(Grip::Resize(Edges::TOP)), "over the strip");
        assert_eq!(at(100, 148), Some(Grip::Resize(Edges::BOTTOM)));
        assert_eq!(at(1, 80), Some(Grip::Resize(Edges::LEFT)));
        assert_eq!(
            at(198, 80),
            Some(Grip::Resize(Edges::RIGHT)),
            "over the text"
        );
        for (x, y, e) in [
            (2, 2, Edges::TOP | Edges::LEFT),
            (197, 2, Edges::TOP | Edges::RIGHT),
            (2, 147, Edges::BOTTOM | Edges::LEFT),
            (197, 147, Edges::BOTTOM | Edges::RIGHT),
        ] {
            assert_eq!(at(x, y), Some(Grip::Resize(e)), "({x}, {y})");
            assert_eq!(hit(&lay, x, y), None, "({x}, {y}) is no button");
        }
        assert_eq!(
            at(2, 10),
            Some(Grip::Resize(Edges::TOP | Edges::LEFT)),
            "over New"
        );
    }

    #[test]
    fn a_click_is_a_press_and_release_on_the_same_button() {
        let mut s = Strip::default();
        s.press(Some(Part::Delete));
        assert_eq!(s.release(Some(Part::New)), None, "released elsewhere");
        s.press(Some(Part::New));
        assert_eq!(s.release(Some(Part::New)), Some(Part::New));
        assert_eq!(s.held, None);
    }

    #[test]
    fn each_colour_paints_its_body_and_strip_and_inks_both_glyphs() {
        for colour in Colour::ALL {
            let c = drawn(colour, Strip::default(), 1);
            assert_eq!(device(&c, 60, 40), colour.body().argb(), "{colour:?} body");
            assert_eq!(
                device(&c, 60, 10),
                colour.strip().argb(),
                "{colour:?} strip"
            );
            for b in [Rect::new(0, 0, 22, 20), Rect::new(98, 0, 22, 20)] {
                let inked = (b.y..b.bottom())
                    .flat_map(|y| (b.x..b.right()).map(move |x| (x, y)))
                    .any(|(x, y)| device(&c, x, y) == INK.argb());
                assert!(inked, "{colour:?}: a glyph in {b:?}");
            }
        }
    }

    #[test]
    fn an_edge_darker_than_the_strip_rings_the_note_at_scale_1_and_2() {
        for colour in Colour::ALL {
            assert_ne!(colour.edge(), colour.strip(), "{colour:?}");
            for s in [1, 2] {
                let c = drawn(colour, Strip::default(), s);
                let (w, h) = (120 * s, 60 * s);
                let edge = colour.edge().argb();
                // Device pixel `d` in from each side, half way along it.
                let sides = |d: i32| {
                    [
                        ("top", w / 2, d),
                        ("bottom", w / 2, h - 1 - d),
                        ("left", d, h / 2),
                        ("right", w - 1 - d, h / 2),
                    ]
                };
                for d in 0..s {
                    for (side, x, y) in sides(d) {
                        assert_eq!(device(&c, x, y), edge, "{colour:?}@{s} {side}, {d} in");
                    }
                }
                for (side, x, y) in sides(s) {
                    let want = if side == "top" {
                        colour.strip()
                    } else {
                        colour.body()
                    };
                    assert_ne!(want.argb(), edge, "{colour:?} {side}");
                    assert_eq!(
                        device(&c, x, y),
                        want.argb(),
                        "{colour:?}@{s} inside the {side}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_lit_button_darkens_and_a_held_one_darkens_more() {
        let at = |s: Strip| device(&drawn(Colour::Yellow, s, 1), 2, 2);
        let plain = at(Strip::default());
        let hot = at(Strip {
            hot: Some(Part::New),
            held: None,
        });
        let held = at(Strip {
            hot: Some(Part::New),
            held: Some(Part::New),
        });
        assert_eq!(plain, Colour::Yellow.strip().argb());
        assert!(plain != hot && hot != held && held != plain, "three states");
        assert_eq!(hot, shade(Colour::Yellow.strip(), HOT_SHADE).argb());
        assert_eq!(held, shade(Colour::Yellow.strip(), HELD_SHADE).argb());
    }

    #[test]
    fn the_cross_has_soft_edges_at_scale_2() {
        let c = drawn(Colour::White, Strip::default(), 2);
        let (body, ink) = (Colour::White.strip().argb(), INK.argb());
        let soft = (0..40).any(|y| {
            (196..240).any(|x| {
                let v = device(&c, x, y);
                v != body && v != ink
            })
        });
        assert!(soft, "partial pixels along the diagonals");
    }

    #[test]
    fn the_strip_pins_at_scale_1_and_2() {
        let hot = Strip {
            hot: Some(Part::Delete),
            held: None,
        };
        let got: Vec<u64> = [1, 2]
            .into_iter()
            .flat_map(|s| {
                [
                    hash(&drawn(Colour::Yellow, Strip::default(), s)),
                    hash(&drawn(Colour::Blue, hot, s)),
                ]
            })
            .collect();
        assert_eq!(
            got,
            [
                0x0747_cc9c_aef9_bbb0,
                0xa267_d7e3_f2b1_ffbd,
                0x4161_6c26_3034_8831,
                0xfbb6_889b_cfe7_6511
            ]
        );
    }
}
