//! Glass's shell window panes: the navigation pane's splitter line, the details pane's backdrop
//! and the view's group headings ([`PaneLook`]).
//!
//! The details pane opens with a pale blue line, then shades over four rows into its pale blue
//! face, its values in a dark navy. A group heading is its title in blue with a grey rule after it
//! to the heading's right end.
//! Dark Glass takes the light faces through [`dark::remap_color`] and lighter blue inks.

use super::{Mode, dark};
use crate::raster::{Canvas, Rect};
use crate::shape::{centre, mix, pack, premul};
use crate::style::PaneLook;
use crate::theme::Color;

pub const NAV_EDGE: Color = Color::rgb(0xD6, 0xE5, 0xF5);
pub const DETAILS_LINE: Color = Color::rgb(0xCC, 0xD9, 0xEA);
pub const DETAILS_SHADE: Color = Color::rgb(0xD9, 0xE3, 0xF0);
pub const DETAILS_FACE: Color = Color::rgb(0xF1, 0xF5, 0xFB);
/// How many design rows under the line the shade takes to reach the face.
const DETAILS_SHADE_ROWS: f64 = 4.0;
pub const DETAILS_LABEL: Color = Color::rgb(0x5A, 0x67, 0x7C);
pub const DETAILS_VALUE: Color = Color::rgb(0x1E, 0x39, 0x5B);
pub const HEADING: Color = Color::rgb(0x1E, 0x32, 0x87);
pub const HEADING_RULE: Color = Color::rgb(0xE2, 0xE2, 0xE2);

pub const DARK_DETAILS_LABEL: Color = Color::rgb(0xA4, 0xAE, 0xBE);
pub const DARK_DETAILS_VALUE: Color = Color::rgb(0xDC, 0xE6, 0xF4);
pub const DARK_HEADING: Color = Color::rgb(0x8C, 0xB4, 0xF2);
pub const DARK_HEADING_RULE: Color = Color::rgb(0x4E, 0x4E, 0x4E);

pub fn look(mode: Mode) -> PaneLook {
    match mode {
        Mode::Light => PaneLook {
            nav_edge: NAV_EDGE,
            details: details_light,
            details_label: DETAILS_LABEL,
            details_value: DETAILS_VALUE,
            heading: HEADING,
            heading_rule: rule_light,
        },
        Mode::Dark => PaneLook {
            nav_edge: dark::remap_color(NAV_EDGE),
            details: details_dark,
            details_label: DARK_DETAILS_LABEL,
            details_value: DARK_DETAILS_VALUE,
            heading: DARK_HEADING,
            heading_rule: rule_dark,
        },
    }
}

fn details_light(c: &mut Canvas, r: Rect, d: i32) {
    details(c, r, d, [DETAILS_LINE, DETAILS_SHADE, DETAILS_FACE]);
}

fn details_dark(c: &mut Canvas, r: Rect, d: i32) {
    let [line, shade, face] = [DETAILS_LINE, DETAILS_SHADE, DETAILS_FACE].map(dark::remap_color);
    details(c, r, d, [line, shade, face]);
}

/// The backdrop down `r`: design row 0 the line, then the shade easing into the face.
fn details(c: &mut Canvas, r: Rect, d: i32, [line, shade, face]: [Color; 3]) {
    let dev = r.scaled(c.scale());
    let (line, shade, face) = (premul(line, 255), premul(shade, 255), premul(face, 255));
    c.compose_device(dev, |_, y| {
        let t = centre(y - dev.y, d);
        pack(match t < 0.5 {
            true => line,
            false => mix(shade, face, ease_out((t - 1.0) / DETAILS_SHADE_ROWS)),
        })
    });
}

fn ease_out(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t) * (1.0 - t)
}

fn rule_light(c: &mut Canvas, r: Rect, d: i32) {
    rule(c, r, d, HEADING_RULE);
}

fn rule_dark(c: &mut Canvas, r: Rect, d: i32) {
    rule(c, r, d, DARK_HEADING_RULE);
}

/// One design row across the middle of `r` in `ink`.
fn rule(c: &mut Canvas, r: Rect, d: i32, ink: Color) {
    let dev = r.scaled(c.scale());
    let row = Rect::new(dev.x, dev.y + (dev.height - d) / 2, dev.width, d);
    let ink = pack(premul(ink, 255));
    c.compose_device(row, |_, _| ink);
}
