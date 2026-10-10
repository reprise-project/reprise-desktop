//! Glass's list header and list frame parts, drawn at the output scale.
//! A header item is 24 rows: white over its top nine, a pale grey gradient ([`FACE_TOP`] to [`FACE_BOTTOM`]), a [`LINE`] row and a near-white foot. The divider is the last column. The sorted column rests pale blue (`Checked`), edged [`SORT_EDGE`]; under the pointer an item is light blue between two blue edges, held a deeper blue sunk into a darker rim. The strip past the last column is a resting item's face with no divider.
//! The sort arrow is a triangle on rows 1 to 4 of a 9 px frame (rows of 1, 3, 5 and 7 whole pixels plus a half pixel each side), pointing up rising and down falling, lit from the upper left: [`ARROW_DARK`] on the facing edges, [`ARROW_PALE`] two pixels in.
//! A list box, list or tree view is framed by one flat line, the ring inside it clear.

use crate::raster::{Canvas, Insets, Rect};
use crate::shape::{self, Px, Stops, centre, polygon, premul};
use crate::style::{Layout, Part, PartImage, State};
use crate::theme::Color;

pub const LIST_LINE: Color = Color::rgb(0x7F, 0x84, 0x8E);

/// The title's box inside an item: the rows the title is centred on, 5
/// under the top and 5 over the bottom, so its capitals start on row 8
/// and it stands on row 15.
const TITLE: Insets = Insets::new(0, 5, 0, 5);

/// An item's width: two left columns, one middle column that stretches,
/// three right columns. Every header part is the header's 24 rows.
const ITEM_W: i32 = 6;
const H: i32 = 24;
const GRAD_TOP: i32 = 9;
const GRAD_END: i32 = 21;
const LINE_ROW: i32 = 22;

const WHITE: Color = Color::rgb(0xFF, 0xFF, 0xFF);
pub const FACE_TOP: Color = Color::rgb(0xF6, 0xF8, 0xFA);
pub const FACE_BOTTOM: Color = Color::rgb(0xF0, 0xF1, 0xF4);
pub const LINE: Color = Color::rgb(0xD4, 0xD3, 0xD4);
pub const FOOT: Color = Color::rgb(0xFA, 0xFB, 0xFB);
pub const HIGHLIGHT: Color = Color::rgb(0xFB, 0xFC, 0xFD);
pub const DIVIDER_TOP: [Color; 2] = [Color::rgb(0xF2, 0xF1, 0xF2), Color::rgb(0xEE, 0xED, 0xEE)];
pub const DIVIDER_LOW: [Color; 2] = [Color::rgb(0xE6, 0xE7, 0xE9), Color::rgb(0xDD, 0xDE, 0xDF)];
pub const HOT_EDGE: [Color; 2] = [Color::rgb(0x85, 0xC4, 0xEA), Color::rgb(0x65, 0xB3, 0xE2)];
pub const HOT_TOP: Color = Color::rgb(0xE2, 0xF5, 0xFF);
pub const HOT_GRAD: [Color; 2] = [Color::rgb(0xBB, 0xE9, 0xFF), Color::rgb(0xB4, 0xE3, 0xFB)];
pub const HOT_LINE: Color = Color::rgb(0x90, 0xC3, 0xE2);
pub const HELD_TOP: Color = Color::rgb(0xBA, 0xE0, 0xF9);
pub const HELD_GRAD: [Color; 2] = [Color::rgb(0x8A, 0xCF, 0xF7), Color::rgb(0x86, 0xCA, 0xF5)];
const SINK_COLS: [f64; 2] = [0.32, 0.12];
const SINK_ROWS: [f64; 2] = [0.29, 0.10];
const SINK_CORNER: f64 = 0.2;
pub const SORT_EDGE: Color = Color::rgb(0x92, 0xD3, 0xF9);
pub const SORT_TOP: Color = Color::rgb(0xF1, 0xF8, 0xFC);
pub const SORT_GRAD: [Color; 2] = [Color::rgb(0xE0, 0xEF, 0xF9), Color::rgb(0xD7, 0xEA, 0xF6)];

const ARROW_W: i32 = 9;
const ARROW_TOP: f64 = 1.0;
const ARROW_FOOT: f64 = 5.0;
pub const ARROW_DARK: Color = Color::rgb(0x3B, 0x59, 0x6E);
pub const ARROW_MID: Color = Color::rgb(0x66, 0xA2, 0xCB);
pub const ARROW_PALE: Color = Color::rgb(0xC2, 0xE0, 0xF4);
const ARROW_SWEEP: f64 = 0.3;
const ARROW_FADE: f64 = 0.5;

/// The header item, filler and sort arrow painted at `d` device px per
/// design px, and the list frame (round lists and dialog pictures alike),
/// the same in every glass colour.
pub fn parts(d: i32) -> [(Part, PartImage); 5] {
    [
        (Part::HeaderItem, item(d)),
        (Part::HeaderFiller, filler(d)),
        (Part::HeaderSortArrow, sort_arrow(d)),
        (Part::ListBorder, list_border()),
        (Part::WellFrame, list_border()),
    ]
}

fn item(d: i32) -> PartImage {
    let states = [State::Normal, State::Hot, State::Pressed, State::Checked];
    let frames = states.map(|s| shape::draw(ITEM_W, H, d, |x, y| item_px(s, x, y, d)));
    PartImage::new(stack(&frames), &states, Layout::Vertical)
        .sizing(Insets::new(2, 0, 3, 0))
        .content(TITLE)
        .at_scale(d)
}

fn filler(d: i32) -> PartImage {
    let frame = shape::draw(1, H, d, |_, y| rest_face(y as i32 / d, centre(y as i32, d)));
    PartImage::new(frame, &[State::Normal], Layout::Vertical).at_scale(d)
}

fn rgb(c: Color) -> Px {
    premul(c, 255)
}

fn between(pair: [Color; 2], t: f64) -> Px {
    Stops(&[(0.0, rgb(pair[0])), (1.0, rgb(pair[1]))]).at(t)
}

fn banded(row: i32, y: f64, top: Px, grad: [Color; 2]) -> Px {
    if row < GRAD_TOP {
        top
    } else {
        between(
            grad,
            (y - f64::from(GRAD_TOP)) / f64::from(GRAD_END - GRAD_TOP),
        )
    }
}

fn rest_face(row: i32, y: f64) -> Px {
    match row {
        LINE_ROW => rgb(LINE),
        r if r > LINE_ROW => rgb(FOOT),
        r => banded(r, y, rgb(WHITE), [FACE_TOP, FACE_BOTTOM]),
    }
}

fn item_px(state: State, x: f64, y: f64, d: i32) -> Px {
    let (col, row) = (x as i32 / d, y as i32 / d);
    let cy = centre(y as i32, d);
    let edge = col == 0 || col == ITEM_W - 1;
    match state {
        State::Hot | State::Pressed if row == LINE_ROW => rgb(HOT_LINE),
        State::Hot | State::Pressed if row > LINE_ROW => rgb(FOOT),
        State::Hot if edge => rgb(HOT_EDGE[usize::from(row >= GRAD_TOP)]),
        State::Hot => banded(row, cy, rgb(HOT_TOP), HOT_GRAD),
        State::Pressed => {
            let (a, [r, g, b]) = banded(row, cy, rgb(HELD_TOP), HELD_GRAD);
            let k = 1.0 - sink(col, row);
            (a, [r * k, g * k, b * k])
        }
        State::Checked if row > LINE_ROW => rgb(WHITE),
        State::Checked if edge || row == LINE_ROW => rgb(SORT_EDGE),
        State::Checked => banded(row, cy, rgb(SORT_TOP), SORT_GRAD),
        _ if row >= LINE_ROW => rest_face(row, cy),
        _ if col == ITEM_W - 1 && row < GRAD_TOP => {
            between(DIVIDER_TOP, cy / f64::from(GRAD_TOP - 1))
        }
        _ if col == ITEM_W - 1 => banded(row, cy, rgb(WHITE), DIVIDER_LOW),
        _ if row < GRAD_TOP => rgb(WHITE),
        _ if col == ITEM_W - 2 => rgb(HIGHLIGHT),
        _ if col == 0 => shape::mix(rest_face(row, cy), rgb(WHITE), 0.5),
        _ => rest_face(row, cy),
    }
}

/// How much light the held item's sink takes at design column `col` and
/// row `row`: the deeper of the side's and the top's, and
/// [`SINK_CORNER`] of the other where they meet.
fn sink(col: i32, row: i32) -> f64 {
    let from_side = col.min(ITEM_W - 1 - col) as usize;
    let side = SINK_COLS.get(from_side).copied().unwrap_or(0.0);
    let top = SINK_ROWS.get(row as usize).copied().unwrap_or(0.0);
    side.max(top) + SINK_CORNER * side.min(top)
}

fn sort_arrow(d: i32) -> PartImage {
    let frames = [true, false].map(|up| shape::draw(ARROW_W, H, d, |x, y| arrow_px(up, x, y, d)));
    PartImage::new(
        stack(&frames),
        &[State::Normal, State::Checked],
        Layout::Vertical,
    )
    .at_scale(d)
}

/// The arrow's pixel at device top-left (`x`, `y`), rising when `up`.
/// Its sides run at 45 degrees from a pixel-wide tip, through pixel
/// corners, so each row holds whole pixels and a half pixel each side.
fn arrow_px(up: bool, x: f64, y: f64, d: i32) -> Px {
    let k = f64::from(d);
    let w = f64::from(ARROW_W);
    let mid = w / 2.0;
    let (top, foot) = (ARROW_TOP * k, ARROW_FOOT * k);
    let (tip_l, tip_r) = ((mid - 0.5) * k, (mid + 0.5) * k);
    let cover = if up {
        polygon(
            x,
            y,
            &[(tip_l, top), (tip_r, top), (w * k, foot), (0.0, foot)],
        )
    } else {
        polygon(
            x,
            y,
            &[(0.0, top), (w * k, top), (tip_r, foot), (tip_l, foot)],
        )
    };
    if cover == 0.0 {
        return shape::CLEAR;
    }
    // The pixel's middle in design px, from the frame's top-left corner.
    let cx = centre(x as i32, d) + 0.5;
    let cy = centre(y as i32, d) + 0.5;
    // How far in from the lit edges: the rising arrow's left side (where
    // cx + cy = ARROW_FOOT), the falling one's top and left side (where
    // cx - cy = -ARROW_TOP), each edge pixel's middle at 0.5.
    let h = if up {
        cx + cy - ARROW_FOOT - 0.5
    } else {
        let down = cy - ARROW_TOP;
        let from_top = down + ARROW_SWEEP * (cx - 1.0);
        let from_left = cx - down - 0.5 + ARROW_FADE * (down - 0.5);
        from_top.min(from_left)
    };
    let shade = Stops(&[
        (0.5, rgb(ARROW_DARK)),
        (1.5, rgb(ARROW_MID)),
        (3.0, rgb(ARROW_PALE)),
    ])
    .at(h);
    shape::scale(shade, cover)
}

pub(super) fn stack(frames: &[Canvas]) -> Canvas {
    let (w, h) = (frames[0].width, frames[0].height);
    let mut a = Canvas::new(w, h * frames.len() as u32);
    for (i, f) in frames.iter().enumerate() {
        a.blit_over(0, i as i32 * h as i32, f);
    }
    a
}

/// The list frame: a 5x5 nine-grid border, the 1 px line and a clear ring
/// inside it, so the caller's fill runs up to the line.
fn list_border() -> PartImage {
    let mut a = Canvas::new(5, 5);
    a.ring_t(Rect::new(0, 0, 5, 5), LIST_LINE, LIST_LINE, 1);
    PartImage {
        border_only: true,
        ..PartImage::new(a, &[State::Normal], Layout::Vertical).sizing(Insets::new(2, 2, 2, 2))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controls::listview::{self, Column, ListView};
    use crate::glass::dark::remap_part;
    use crate::interface::Interface;
    use crate::interfaces::theme_for;
    use crate::look::styled::fixture::doubles_at_paint_scale_2;
    use crate::look::widgets::header::{draw_item, draw_sort_arrow, draw_strip, sort_arrow_width};
    use crate::look::widgets::{ComboLook, draw_combo_frame, draw_field_frame, draw_well};
    use crate::paint::Paint;
    use crate::pointer::Interaction;
    use crate::raster::EdgeStyle;
    use crate::text::TextRenderer;
    use crate::theme::FontConfig;

    #[track_caller]
    fn pin(c: &Canvas, x: i32, y: i32, rgb: u32, what: &str) {
        assert_eq!(
            c.get(x, y),
            0xFF00_0000 | rgb,
            "{what}: ({x}, {y}) is {rgb:06X}"
        );
    }

    fn hex(c: Color) -> u32 {
        c.argb() & 0xFF_FFFF
    }

    fn light(px: u32) -> u32 {
        (px >> 16 & 0xFF) + (px >> 8 & 0xFF) + (px & 0xFF)
    }

    fn item(ptr: Interaction, sorted: bool) -> (Canvas, Rect) {
        let theme = theme_for(Interface::Glass);
        let mut c = Canvas::new(64, H as u32);
        let t = draw_item(&mut c, &theme.paint(), Rect::new(0, 0, 64, H), ptr, sorted);
        (c, t)
    }

    #[test]
    fn a_glass_header_item_at_rest_is_white_over_a_grey_gradient_with_its_divider_last() {
        let (c, _) = item(Interaction::Idle, false);
        for (x, y, rgb, what) in [
            (32, 0, 0xFFFFFF, "the white top"),
            (32, 8, 0xFFFFFF, "the white top's last row"),
            (32, 9, hex(FACE_TOP), "the gradient's first row"),
            (32, 21, hex(FACE_BOTTOM), "the gradient's last row"),
            (32, 22, hex(LINE), "the line under it"),
            (32, 23, hex(FOOT), "the last row"),
            (
                0,
                9,
                0xFBFCFD,
                "the first column on the gradient, halfway to white",
            ),
            (0, 0, 0xFFFFFF, "the first column on the white"),
            (62, 9, hex(HIGHLIGHT), "the column before the divider"),
            (63, 0, hex(DIVIDER_TOP[0]), "the divider on the white"),
            (63, 8, hex(DIVIDER_TOP[1]), "the divider's last white row"),
            (63, 9, hex(DIVIDER_LOW[0]), "the divider on the gradient"),
            (
                63,
                21,
                hex(DIVIDER_LOW[1]),
                "the divider's last gradient row",
            ),
        ] {
            pin(&c, x, y, rgb, what);
        }
    }

    #[test]
    fn a_glass_header_item_lights_blue_under_the_pointer_and_sinks_held() {
        let (c, _) = item(Interaction::Hot, false);
        for (x, y, rgb, what) in [
            (0, 0, hex(HOT_EDGE[0]), "hot: the left edge on top"),
            (0, 9, hex(HOT_EDGE[1]), "hot: the left edge below"),
            (63, 9, hex(HOT_EDGE[1]), "hot: the right edge below"),
            (32, 0, hex(HOT_TOP), "hot: the pale top"),
            (32, 9, hex(HOT_GRAD[0]), "hot: the blue below"),
            (32, 22, hex(HOT_LINE), "hot: the line under it"),
        ] {
            pin(&c, x, y, rgb, what);
        }
        let (c, _) = item(Interaction::Pressed, false);
        pin(&c, 32, 2, hex(HELD_TOP), "held: the face");
        pin(&c, 32, 9, hex(HELD_GRAD[0]), "held: the deeper face");
        pin(&c, 32, 22, hex(HOT_LINE), "held: the line under it");
        for y in [5, 15] {
            let (outer, inner, face) = (c.get(0, y), c.get(1, y), c.get(32, y));
            assert!(
                light(outer) < light(inner),
                "held row {y}: the outer edge is darkest"
            );
            assert!(light(inner) < light(face), "held row {y}: the inner edge");
            assert_eq!(
                c.get(63, y),
                outer,
                "held row {y}: the right edge as the left"
            );
            assert_eq!(c.get(62, y), inner, "held row {y}: the right inner edge");
        }
        assert!(
            light(c.get(32, 0)) < light(c.get(32, 1)),
            "held: the top row darkest"
        );
        assert!(
            light(c.get(32, 1)) < light(c.get(32, 2)),
            "held: the second row"
        );
        assert!(
            light(c.get(0, 0)) < light(c.get(0, 2)),
            "held: darkest in the corner"
        );
    }

    #[test]
    fn a_sorted_glass_column_rests_pale_blue_and_lights_as_any_column() {
        let (c, _) = item(Interaction::Idle, true);
        for (x, y, rgb, what) in [
            (0, 5, hex(SORT_EDGE), "the left edge"),
            (63, 5, hex(SORT_EDGE), "the right edge"),
            (32, 0, hex(SORT_TOP), "the pale top"),
            (32, 9, hex(SORT_GRAD[0]), "the gradient's first row"),
            (32, 21, hex(SORT_GRAD[1]), "the gradient's last row"),
            (32, 22, hex(SORT_EDGE), "the bottom edge"),
            (32, 23, 0xFFFFFF, "the last row"),
        ] {
            pin(&c, x, y, rgb, what);
        }
        for ptr in [Interaction::Hot, Interaction::Pressed] {
            assert!(
                item(ptr, true).0 == item(ptr, false).0,
                "{ptr:?}: a sorted column draws as an unsorted one"
            );
        }
    }

    #[test]
    fn a_glass_header_title_sits_on_the_item_s_middle_rows() {
        assert_eq!(item(Interaction::Idle, false).1, Rect::new(0, 5, 64, 14));
        assert_eq!(item(Interaction::Pressed, false).1, Rect::new(1, 6, 64, 14));
        assert_eq!(item(Interaction::Idle, true).1, Rect::new(0, 5, 64, 14));
    }

    #[test]
    fn the_glass_strip_past_the_last_column_has_no_divider() {
        let theme = theme_for(Interface::Glass);
        let mut c = Canvas::new(40, H as u32);
        draw_strip(&mut c, &theme.paint(), Rect::new(0, 0, 40, H));
        for x in [0, 20, 39] {
            pin(&c, x, 0, 0xFFFFFF, "the white top");
            pin(&c, x, 9, hex(FACE_TOP), "the gradient");
            pin(&c, x, 22, hex(LINE), "the line under it");
        }
    }

    #[test]
    fn the_glass_sort_arrow_rises_and_falls_on_the_header_s_top_rows() {
        let theme = theme_for(Interface::Glass);
        let draw = |asc| {
            let mut c = Canvas::new(9, H as u32);
            draw_sort_arrow(&mut c, &theme.paint(), 0, Rect::new(0, 0, 60, H), asc);
            c
        };
        let up = draw(true);
        pin(&up, 4, 1, hex(ARROW_DARK), "rising: the tip");
        pin(&up, 1, 4, hex(ARROW_DARK), "rising: the base's left end");
        pin(&up, 7, 4, hex(ARROW_PALE), "rising: the base's right end");
        let down = draw(false);
        pin(&down, 1, 1, hex(ARROW_DARK), "falling: the top's left end");
        assert!(
            light(down.get(7, 1)) > light(down.get(1, 1)),
            "falling: the top pales"
        );
        assert!(
            light(down.get(4, 4)) > light(down.get(2, 2)),
            "falling: the tip pales"
        );
        for (c, asc) in [(&up, true), (&down, false)] {
            for row in 1..=4 {
                let k = if asc { row - 1 } else { 4 - row };
                let whole = (4 - k..=4 + k).all(|x| c.get(x, row) >> 24 == 0xFF);
                assert!(
                    whole,
                    "asc {asc}: row {row} holds {} whole pixels",
                    2 * k + 1
                );
                for x in [3 - k, 5 + k] {
                    let a = c.get(x, row) >> 24;
                    assert!(
                        (0x70..=0x90).contains(&a),
                        "asc {asc}: ({x}, {row}) is half on: {a:#X}"
                    );
                }
            }
            for x in 0..9 {
                for y in [0, 5, 12, 23] {
                    assert_eq!(c.get(x, y), 0, "asc {asc}: ({x}, {y}) is clear");
                }
            }
        }
    }

    #[test]
    fn a_glass_list_centres_its_sort_arrow_over_the_column() {
        let theme = theme_for(Interface::Glass);
        let text = TextRenderer::load(&FontConfig::default());
        let cm = theme.look.controls();
        assert_eq!(cm.list_header, 24, "a 24 px header");
        let draw = |sort| {
            let lv: ListView = ListView {
                columns: vec![Column::new("Name", 90), Column::new("Type", 60)],
                sort,
                frame: false,
                ..ListView::default()
            };
            let r = Rect::new(0, 0, 160, 30);
            let lay = listview::layout(&lv, r, &cm);
            let mut c = Canvas::new(160, 30);
            listview::draw(
                &mut c,
                &text,
                &theme.paint(),
                &lv,
                &lay,
                None,
                theme.fonts.size_px,
            );
            c
        };
        let sorted = draw(Some((0, true)));
        pin(&sorted, 45, 1, hex(ARROW_DARK), "the arrow's tip");
        pin(&sorted, 42, 4, hex(ARROW_DARK), "the arrow's base");
        let plain = draw(Some((1, true)));
        let ink = |c: &Canvas| {
            (6..H)
                .flat_map(|y| (0..88).map(move |x| (x, y)))
                .filter(|&(x, y)| c.get(x, y) & 0xFF < 0x40)
                .collect::<Vec<_>>()
        };
        assert_eq!(ink(&sorted), ink(&plain), "the title keeps its place");
    }

    #[test]
    fn a_glass_list_is_framed_828790_and_a_combo_keeps_the_edit_frame() {
        let theme = theme_for(Interface::Glass);
        let p = theme.paint();
        let mut c = Canvas::new(60, 40);
        draw_field_frame(&mut c, &p, Rect::new(0, 0, 60, 40), p.cs.window);
        for (x, y) in [(0, 0), (30, 0), (59, 20), (30, 39), (0, 20), (59, 39)] {
            pin(&c, x, y, 0x7F848E, "the line");
        }
        for (x, y) in [(1, 1), (30, 1), (58, 38)] {
            pin(&c, x, y, 0xFFFFFF, "the fill inside the line");
        }
        let mut c = Canvas::new(80, 21);
        let look = ComboLook {
            enabled: true,
            focused: false,
            ptr: Interaction::Idle,
            editable: true,
        };
        let r = Rect::new(0, 0, 80, 21);
        draw_combo_frame(&mut c, &p, r, Rect::new(62, 2, 16, 17), p.cs.window, look);
        pin(&c, 30, 0, 0xA9AAB1, "the editable combo's top line");
    }

    #[test]
    fn a_glass_header_and_list_frame_double_at_paint_scale_2() {
        for ptr in [Interaction::Idle, Interaction::Hot, Interaction::Pressed] {
            assert!(
                doubles_at_paint_scale_2(Interface::Glass, 200, 60, |c, p| {
                    let s = p.s;
                    let header = Rect::new(2, 2, 190, H).scaled(s);
                    draw_strip(c, p, header);
                    for (i, sorted) in [true, false].into_iter().enumerate() {
                        let r = Rect::new(2 + 70 * i as i32, 2, 70, H).scaled(s);
                        draw_item(c, p, r, ptr, sorted);
                    }
                    draw_sort_arrow(c, p, 33 * s, header, true);
                    draw_field_frame(c, p, Rect::new(2, 30, 100, 28).scaled(s), p.cs.window);
                }),
                "{ptr:?}"
            );
        }
    }

    #[test]
    fn a_glass_picture_well_is_the_list_line_round_window() {
        for (edge, what) in [
            (EdgeStyle::ButtonSunken, "full"),
            (EdgeStyle::ThinSunken, "thin"),
        ] {
            let theme = theme_for(Interface::Glass);
            let p = theme.paint();
            let mut c = Canvas::new(40, 30);
            draw_well(&mut c, &p, Rect::new(0, 0, 40, 30), edge);
            let window = p.cs.window.argb() & 0xFF_FFFF;
            for (x, y) in [(0, 0), (39, 0), (0, 29), (39, 29), (20, 0), (0, 15)] {
                pin(&c, x, y, 0x7F848E, &format!("{what}: the line"));
            }
            for (x, y) in [(1, 1), (38, 28), (20, 15)] {
                pin(&c, x, y, window, &format!("{what}: window inside"));
            }
        }
        let theme = theme_for(Interface::Classic);
        let p = theme.paint();
        let mut c = Canvas::new(40, 30);
        draw_well(&mut c, &p, Rect::new(0, 0, 40, 30), EdgeStyle::ButtonSunken);
        let shadow = p.cs.button_shadow.argb() & 0xFF_FFFF;
        pin(&c, 0, 0, shadow, "Classic keeps its sunken edge");
    }

    #[test]
    fn a_glass_picture_well_doubles_at_paint_scale_2() {
        assert!(doubles_at_paint_scale_2(
            Interface::Glass,
            60,
            40,
            |c, p| {
                draw_well(
                    c,
                    p,
                    Rect::new(2, 2, 50, 30).scaled(p.s),
                    EdgeStyle::ThinSunken,
                );
            }
        ));
    }

    fn header_parts(d: i32) -> [PartImage; 3] {
        let [(_, i), (_, f), (_, a), ..] = parts(d);
        [i, f, a]
    }

    #[test]
    fn the_header_parts_are_their_design_size_at_scale_1_and_2() {
        let one = header_parts(1);
        for d in [1, 2] {
            for (i, (img, (w, h))) in header_parts(d)
                .iter()
                .zip([(6, 24), (1, 24), (9, 24)])
                .enumerate()
            {
                assert_eq!(img.scale, d, "part {i} at {d}");
                let f = img.frame(State::Normal);
                assert_eq!((f.width, f.height), (w * d, h * d), "part {i} at {d}");
                assert_eq!(
                    img.sizing, one[i].sizing,
                    "part {i}: the nine-grid stays design px"
                );
                assert_eq!(
                    img.content, one[i].content,
                    "part {i}: the title box stays design px"
                );
                let frames: Vec<Vec<u32>> = img
                    .states
                    .iter()
                    .map(|&s| {
                        let f = img.frame(s);
                        (f.y..f.bottom())
                            .flat_map(|y| (f.x..f.right()).map(move |x| (x, y)))
                            .map(|(x, y)| img.atlas.get(x, y))
                            .collect()
                    })
                    .collect();
                for a in 0..frames.len() {
                    for b in a + 1..frames.len() {
                        assert_ne!(frames[a], frames[b], "part {i} at {d}: states {a} and {b}");
                    }
                }
            }
        }
    }

    #[test]
    fn the_sort_arrow_is_smooth_at_scale_2_with_no_dark_fringe() {
        let [_, _, arrow] = header_parts(2);
        let a = &arrow.atlas;
        let soft: Vec<u32> = a
            .pixels
            .iter()
            .copied()
            .filter(|p| (1..0xFF).contains(&(p >> 24)))
            .collect();
        assert!(soft.len() >= 20, "the sides blend at 2 ({} px)", soft.len());
        let floor = light(ARROW_DARK.argb());
        for p in soft {
            let al = p >> 24;
            let unmul = |sh: u32| (p >> sh & 0xFF) * 255 / al;
            let l = unmul(16) + unmul(8) + unmul(0);
            assert!(
                l + 6 >= floor,
                "{p:08X} is darker than the arrow's dark shade"
            );
        }
    }

    fn fnv(c: &Canvas) -> u64 {
        let head = [c.width, c.height];
        head.iter()
            .chain(c.pixels.iter())
            .fold(0xcbf2_9ce4_8422_2325_u64, |h, &p| {
                p.to_le_bytes().iter().fold(h, |h, &b| {
                    (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
                })
            })
    }

    #[test]
    fn header_pins_at_scale_1_and_2() {
        let got = PINS.map(|(d, dark, _)| {
            let parts = header_parts(d).map(|p| if dark { remap_part(p) } else { p });
            let h = parts
                .iter()
                .fold(0, |h, p| h ^ fnv(&p.atlas).rotate_left(7));
            (d, dark, h)
        });
        assert!(
            got == PINS,
            "pins changed; now:\n{}",
            got.map(|(d, dark, h)| format!("({d}, {dark}, {h:#018x}),"))
                .join("\n")
        );
    }

    const PINS: [(i32, bool, u64); 4] = [
        (1, false, 0xc39bc66bd621e9ae),
        (2, false, 0x71930e02f0405507),
        (1, true, 0xf55b1e1587d3efe8),
        (2, true, 0xa8a03cf481a5aea2),
    ];

    #[test]
    fn the_scale_2_style_draws_the_arrow_at_its_design_size() {
        let theme = theme_for(Interface::Glass);
        let look = theme.look.at_scale(2);
        let p = Paint {
            look: &look,
            s: 2,
            ..theme.paint()
        };
        assert_eq!(sort_arrow_width(&p), 9, "design px");
        let mut c = Canvas::new(30, 2 * H as u32);
        draw_sort_arrow(&mut c, &p, 0, Rect::new(0, 0, 30, 2 * H), true);
        assert_eq!(c.get(8, 2) >> 24, 0xFF, "the tip's middle");
        for x in [0, 17] {
            let a = c.get(x, 9) >> 24;
            assert!(
                (1..0xFF).contains(&a),
                "the base's end at {x} is partly on: {a:#X}"
            );
        }
        assert!((18..30).all(|x| c.get(x, 9) == 0), "nothing past 18 px");
        assert!((0..30).all(|x| c.get(x, 10) == 0), "nothing under row 9");
    }
}
