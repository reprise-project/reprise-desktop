//! The menus the Blue Start menu opens beside it (All Programs, Recent Items and their folders):
//! white in a thin frame of the Start menu's edge colour, a strip of its band down the left, rows
//! lit in the scheme's selection colour.

use crate::blue::scheme::Scheme;
use crate::raster::Insets;
use crate::shape::{self, Px, Stops, mix, premul};
use crate::style::{DropShadow, Layout, MenuLook, Part, PartImage, State};
use crate::theme::Color;

const WHITE: Color = Color::rgb(0xFF, 0xFF, 0xFF);
/// The frame's line and the strip inside it down the left edge (its sheen line included), design px.
const LINE: i32 = 1;
const STRIP: i32 = 5;
const INSET: Insets = Insets::new(LINE + STRIP, LINE, LINE, LINE);
/// How far the strip's first line goes from the strip toward the face.
const SHEEN: f64 = 0.45;
/// Where along the Start menu's band gradient the strip takes its colour, at its outer and inner edge.
const STRIP_BAND: (f64, f64) = (0.5, 0.08);
/// A greyed row lit: the selection colour this far toward the face, so its grey label reads.
const GREYED_LIT: f64 = 0.7;

/// Soft and short: strongest just past the menu's right and bottom edges, gone 5 px out, fading in
/// over the first few px down the right and along the bottom.
const SHADOW: DropShadow = DropShadow {
    opacity: 0.6,
    blur: 1.2,
    spread: 1.5,
    inset: 5.0,
};

/// The menu look of the Start menu's menus in `s`; [`parts`] paints its frame and lit rows.
pub fn look(s: &Scheme) -> MenuLook {
    MenuLook {
        frame: s.start_menu.edge,
        ring: None,
        face: Some(WHITE),
        inset: INSET,
        row: 23,
        separator: 9,
        gutter: 0,
        gutter_fill: WHITE,
        divider: (WHITE, WHITE),
        rule: (s.start_menu.rule, WHITE),
        text_x: 28,
        icon_x: 5,
        right_pad: 28,
        shortcut_end: 16,
        arrow_inset: 9,
        arrow: s.system.menu_text,
        greyed: s.system.gray_text,
        check_at: (0, 0),
        radio_at: (0, 0),
        shadow: Some(&SHADOW),
    }
}

/// [`Part::MenuFrame`] and [`Part::MenuItem`] (Hot, and Disabled for a greyed row) at `d`.
pub(super) fn parts(s: &Scheme, d: i32) -> Vec<(Part, PartImage)> {
    vec![(Part::MenuFrame, frame(s, d)), (Part::MenuItem, lit(s, d))]
}

fn opaque(c: Color) -> Px {
    premul(c, 0xFF)
}

fn frame(s: &Scheme, d: i32) -> PartImage {
    let m = &s.start_menu;
    let band: Vec<(f64, Px)> = (m.band.iter())
        .map(|&(t, c)| (f64::from(t), opaque(c)))
        .collect();
    let band = Stops(&band);
    let (edge, face) = (opaque(m.edge), opaque(WHITE));
    let (w, h) = (INSET.left + 1 + INSET.right, INSET.top + 1 + INSET.bottom);
    let (k, line) = (f64::from(d), f64::from(LINE));
    let c = shape::draw(w, h, d, |x, y| {
        let (u, v) = ((x + 0.5) / k, (y + 0.5) / k);
        if u < line || v < line || u > f64::from(w) - line || v > f64::from(h) - line {
            return edge;
        }
        let across = (u - line) / f64::from(STRIP);
        if across >= 1.0 {
            return face;
        }
        let strip = band.at(STRIP_BAND.0 + (STRIP_BAND.1 - STRIP_BAND.0) * across);
        if u < line + 1.0 {
            mix(strip, face, SHEEN)
        } else {
            strip
        }
    });
    PartImage::new(c, &[State::Normal], Layout::Vertical)
        .at_scale(d)
        .sizing(INSET)
}

fn lit(s: &Scheme, d: i32) -> PartImage {
    let hot = opaque(s.system.menu_hilight);
    let greyed = mix(hot, opaque(WHITE), GREYED_LIT);
    let c = shape::draw(1, 2, d, |_, y| if y < f64::from(d) { hot } else { greyed });
    PartImage::new(c, &[State::Hot, State::Disabled], Layout::Vertical).at_scale(d)
}

#[cfg(test)]
mod tests;
