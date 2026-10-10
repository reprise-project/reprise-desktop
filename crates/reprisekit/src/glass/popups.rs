//! Glass's tooltip box, the calendar's weekday band and frame, the file dialog's places bar and selected tile, and the thumbnail popup's close button. The tooltip and close button are drawn at the style's scale, the rest at the design's size.

use crate::raster::{Canvas, Insets};
use crate::shape::{
    Px, Stops, centre, draw, over, premul as pre, ringed, rounded, scale, smooth, stroke,
};
use crate::style::{Layout, Part, PartImage, State};
use crate::theme::Color;

pub const TOOLTIP_INK: Color = Color::rgb(0x54, 0x55, 0x55);

const TIP_TEXT_TOP: i32 = 2;

pub const PLACES_FILL: Color = Color::rgb(0xF3, 0xF4, 0xF4);

pub const BAND_RULE: Color = Color::rgb(0xF4, 0xF5, 0xF5);

pub const CALENDAR_LINE: Color = Color::rgb(0x94, 0x95, 0x95);

pub const TILE_LINE: Color = Color::rgb(0xAA, 0xAB, 0xAB);
pub const TILE_SHADE: Color = Color::rgb(0x6F, 0x6F, 0x6F);
pub const TILE_SIDE_FOOT: Color = Color::rgb(0xBB, 0xBA, 0xBB);
pub const TILE_FACE_TOP: Color = Color::rgb(0xC1, 0xC2, 0xC2);
pub const TILE_FACE: Color = Color::rgb(0xE3, 0xE2, 0xE3);
pub const TILE_FOOT: Color = Color::rgb(0x78, 0x79, 0x79);
pub const TILE_FADE: i32 = 26;

pub const BAND_RULE_INSET: i32 = 5;

pub const TIP_LINE: Color = Color::rgb(0x72, 0x73, 0x73);
pub const TIP_TOP: Color = Color::rgb(0xFF, 0xFF, 0xFF);
pub const TIP_FOOT: Color = Color::rgb(0xE3, 0xE3, 0xEF);
/// The tooltip frame's size at one line, and how far its shadow reaches past the box's right and bottom edges.
const TIP_BOX: i32 = 20;
const TIP_SHADOW: i32 = 5;
const TIP_RADIUS: f64 = 3.0;
const TIP_HOLD: f64 = 0.12;
/// Share of the body's height the top colour holds before it shades, and the share from the bottom that holds the foot colour.
const TIP_SHADOW_ALPHA: f64 = 0.44;
const TIP_SHADOW_FADE: f64 = 1.5;
/// The shadow's darkest alpha, and the power its edges fade by (above 1 the fade starts gently).
const TIP_SHADOW_FROM: (i32, i32) = (5, 4);

/// Where the shadow starts: 5 px in from the box's left edge, 4 px down, so it is whole by the stretching rows and columns.
pub fn parts(d: i32) -> [(Part, PartImage); 5] {
    // The parts that are the same in every glass colour; the tooltip is painted at `d`.
    let tip = PartImage::new(tooltip(d), &[State::Normal], Layout::Vertical)
        .sizing(Insets::new(10, 9, 14, 11))
        .outset(Insets::new(0, 0, TIP_SHADOW, TIP_SHADOW))
        .content(Insets::new(5, TIP_TEXT_TOP, 5, 5))
        .at_scale(d);
    let places = {
        let mut a = Canvas::new(1, 1);
        a.fill_rect(a.bounds(), PLACES_FILL);
        PartImage::new(a, &[State::Normal], Layout::Vertical)
    };
    [
        (Part::Tooltip, tip),
        (Part::CalendarBand, band()),
        (Part::PlacesBar, places),
        (Part::CalendarFrame, calendar_frame()),
        (Part::PlacesTile, tile()),
    ]
}

/// The left 10 columns, right 14 (the shadow's 5 among them), top 9 rows and bottom 11 stay as drawn; the middle stretches.
fn tooltip(d: i32) -> Canvas {
    let n = TIP_BOX + TIP_SHADOW;
    let fill = [
        (0.0, pre(TIP_TOP, 255)),
        (TIP_HOLD, pre(TIP_TOP, 255)),
        (1.0 - TIP_HOLD, pre(TIP_FOOT, 255)),
        (1.0, pre(TIP_FOOT, 255)),
    ];
    let body = f64::from(TIP_BOX - 3);
    let line = pre(TIP_LINE, 255);
    let tip = ringed(
        TIP_BOX,
        TIP_BOX,
        TIP_RADIUS,
        1,
        d,
        |_, y, ring| match ring {
            0 => line,
            _ => Stops(&fill).at((y - 1.0) / body),
        },
    );
    let wide = (TIP_BOX * d) as usize;
    draw(n, n, d, |x, y| {
        let (ix, iy) = (x as i32, y as i32);
        let shadow = tip_shadow(centre(ix, d), centre(iy, d));
        if ix < TIP_BOX * d && iy < TIP_BOX * d {
            over(tip[iy as usize * wide + ix as usize], shadow)
        } else {
            shadow
        }
    })
}

/// The tooltip frame at `d`: the box over its shadow, 25 px square.
fn tip_shadow(x: f64, y: f64) -> Px {
    let end = f64::from(TIP_BOX + TIP_SHADOW) - 0.5;
    let reach = f64::from(TIP_SHADOW);
    let fade = |from: i32, v: f64| {
        let inward = (v - (f64::from(from) - 0.5)).min(end - v);
        smooth(inward / reach).powf(TIP_SHADOW_FADE)
    };
    let k = fade(TIP_SHADOW_FROM.0, x) * fade(TIP_SHADOW_FROM.1, y);
    scale(pre(Color::rgb(0, 0, 0), 255), TIP_SHADOW_ALPHA * k)
}

/// The tooltip's shadow at design point (`x`, `y`): black over a rect from [`TIP_SHADOW_FROM`] to the frame's far corner, fading in over its outer 5 px.
pub const CLOSE_RIM: Color = Color::rgb(0x41, 0x13, 0x1F);
pub const CLOSE_LINE_TOP: Color = Color::rgb(0xF4, 0xD4, 0xCA);
pub const CLOSE_LINE_FOOT: Color = Color::rgb(0xEB, 0xC6, 0xBE);
pub const CLOSE_SIDE_GLOSS: (Color, Color) =
    (Color::rgb(0xEF, 0xCB, 0xC2), Color::rgb(0xEC, 0xC5, 0xBB));
pub const CLOSE_SIDE_FACE: (Color, Color) =
    (Color::rgb(0xD8, 0x9B, 0x8B), Color::rgb(0xE7, 0xBD, 0xB3));
pub const CLOSE_GLOSS: (Color, Color) =
    (Color::rgb(0xE3, 0xA0, 0x8F), Color::rgb(0xDD, 0x94, 0x81));
pub const CLOSE_FACE: (Color, Color) = (Color::rgb(0xB4, 0x48, 0x2B), Color::rgb(0xD4, 0x87, 0x72));
pub const CLOSE_X_LINE: Color = Color::rgb(0x51, 0x53, 0x63);
pub const CLOSE_X_TOP: Color = Color::rgb(0xFF, 0xFF, 0xFF);
pub const CLOSE_X_FOOT: Color = Color::rgb(0xDB, 0xDA, 0xDB);
const CLOSE: i32 = 14;
const CLOSE_RADIUS: f64 = 2.0;
const CLOSE_GLOSS_ROWS: (f64, f64) = (2.0, 4.0);
const CLOSE_FACE_ROWS: (f64, f64) = (5.0, 11.0);
/// The X fills the 6 px square from (4, 4); each line is 2 px across a row, the outline 1 px round it.
const CLOSE_X: (f64, f64) = (4.0, 10.0);
const CLOSE_X_WEIGHT: f64 = 2.0;
const CLOSE_X_SPLIT: f64 = 6.5;

pub fn preview_close(d: i32) -> PartImage {
    let c = |c: Color| pre(c, 255);
    let between = |(a, b): (Color, Color), (top, foot): (f64, f64), y: f64| {
        Stops(&[(0.0, c(a)), (1.0, c(b))]).at((y - top) / (foot - top))
    };
    let split = CLOSE_GLOSS_ROWS.1 + 0.5;
    let last = f64::from(CLOSE - 2);
    let px = ringed(CLOSE, CLOSE, CLOSE_RADIUS, 2, d, |_, y, ring| match ring {
        0 => c(CLOSE_RIM),
        1 if y < 1.5 => c(CLOSE_LINE_TOP),
        1 if y > last - 0.5 => c(CLOSE_LINE_FOOT),
        1 if y < split => between(CLOSE_SIDE_GLOSS, CLOSE_GLOSS_ROWS, y),
        1 => between(CLOSE_SIDE_FACE, CLOSE_FACE_ROWS, y),
        _ if y < split => between(CLOSE_GLOSS, CLOSE_GLOSS_ROWS, y),
        _ => between(CLOSE_FACE, CLOSE_FACE_ROWS, y),
    });
    let k = f64::from(d);
    let (a, b) = CLOSE_X;
    // Falling line through the middles of the top and bottom rows, and how far it runs across per row down.
    let (top, foot) = ((a + 1.0, a + 0.5), (b - 1.0, b - 0.5));
    let run = (foot.0 - top.0) / (foot.1 - top.1);
    let across = |w: f64| w * k / run.hypot(1.0);
    // Each line runs past the square and is cut level and upright, so its ends are whole pixels.
    let x_cover = |x: f64, y: f64, grow: f64| {
        let (lo, hi) = ((a - grow) * k, (b + grow) * k);
        let at = |t: f64| ((top.0 + run * (t - top.1)) * k, t * k);
        let ((x0, y0), (x1, y1)) = (at(a - 3.0), at(b + 3.0));
        let w = across(CLOSE_X_WEIGHT + 2.0 * grow);
        let m = (a + b) * k;
        let down = stroke(x, y, &[(x0, y0), (x1, y1)], w);
        let up = stroke(x, y, &[(m - x0, y0), (m - x1, y1)], w);
        down.max(up) * rounded(x - lo, y - lo, hi - lo, hi - lo, 0.0)
    };
    let wide = (CLOSE * d) as usize;
    let canvas = draw(CLOSE, CLOSE, d, |x, y| {
        let face = px[y as usize * wide + x as usize];
        let line = scale(c(CLOSE_X_LINE), x_cover(x, y, 1.0));
        let ink = if centre(y as i32, d) < CLOSE_X_SPLIT {
            CLOSE_X_TOP
        } else {
            CLOSE_X_FOOT
        };
        over(scale(c(ink), x_cover(x, y, 0.0)), over(line, face))
    });
    PartImage::new(canvas, &[State::Normal], Layout::Vertical).at_scale(d)
}

/// The calendar's frame: a 3x3 ring of [`CALENDAR_LINE`], so the box's own fill shows inside it.
fn calendar_frame() -> PartImage {
    let mut a = Canvas::new(3, 3);
    a.fill_rect(a.bounds(), CALENDAR_LINE);
    a.put_raw(1, 1, 0);
    let mut img =
        PartImage::new(a, &[State::Normal], Layout::Vertical).sizing(Insets::new(1, 1, 1, 1));
    img.border_only = true;
    img
}

fn premul(c: Color, a: u8) -> u32 {
    let m = |v: u8| (u32::from(v) * u32::from(a) + 127) / 255;
    u32::from(a) << 24 | m(c.r) << 16 | m(c.g) << 8 | m(c.b)
}

fn lerp(a: Color, b: Color, k: f32) -> Color {
    a.toward(b, (k.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// The selected tile's frame (`Checked`), 59 rows by five columns: outline, side, face, side, outline. The face column and the rows from its last lightening one to the foot stretch. It stands 1 px in from either side of the bar.
fn tile() -> PartImage {
    const H: i32 = 59;
    const W: i32 = 5;
    let mut a = Canvas::new(W as u32, H as u32);
    let sides = H - 5;
    for y in 0..H {
        let (edge, side, face) = match y {
            0 => (premul(TILE_LINE, 0xC0), TILE_LINE, TILE_LINE),
            1 => (
                TILE_LINE.argb(),
                TILE_SHADE.toward(TILE_LINE, 51),
                TILE_SHADE,
            ),
            y if y == H - 2 => (
                premul(TILE_LINE, 0x40),
                TILE_SIDE_FOOT.toward(TILE_FOOT, 128),
                TILE_FACE,
            ),
            y if y == H - 1 => (0, TILE_LINE, TILE_FOOT),
            y => (
                TILE_LINE.argb(),
                lerp(TILE_SHADE, TILE_SIDE_FOOT, (y - 2) as f32 / sides as f32),
                lerp(
                    TILE_FACE_TOP,
                    TILE_FACE,
                    (y - 2) as f32 / (TILE_FADE - 1) as f32,
                ),
            ),
        };
        for (x, px) in [edge, side.argb(), face.argb(), side.argb(), edge]
            .into_iter()
            .enumerate()
        {
            a.put_raw(x as i32, y, px);
        }
    }
    PartImage::new(a, &[State::Checked], Layout::Vertical)
        .sizing(Insets::new(2, 2 + TILE_FADE, 2, 2))
        .outset(Insets::new(-1, 0, -1, 0))
}

/// The weekday band: Normal white, Disabled clear, each over the [`BAND_RULE`] row along its foot. A frame is 11x3; its middle column stretches.
fn band() -> PartImage {
    let (w, h) = (2 * BAND_RULE_INSET + 1, 3);
    let white = Color::rgb(0xFF, 0xFF, 0xFF);
    let mut a = Canvas::new(w as u32, 2 * h as u32);
    for (i, fill) in [Some(white), None].into_iter().enumerate() {
        let top = i as i32 * h;
        for y in top..top + h {
            for x in 0..w {
                let px = if y == top + h - 1 && x == BAND_RULE_INSET {
                    Some(BAND_RULE)
                } else {
                    fill
                };
                a.put_raw(x, y, px.map_or(0, |c| c.argb()));
            }
        }
    }
    PartImage::new(a, &[State::Normal, State::Disabled], Layout::Vertical).sizing(Insets::new(
        BAND_RULE_INSET,
        0,
        BAND_RULE_INSET,
        1,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controls::tooltip::{draw_tip, tip_size};
    use crate::interface::Interface;
    use crate::interfaces::{theme_for, theme_with};
    use crate::look::styled::fixture::doubles_at_scale_1;
    use crate::look::widgets::placesbar::{draw_panel, draw_selected};
    use crate::look::widgets::tooltip;
    use crate::look::widgets::{draw_calendar_band, draw_calendar_frame};
    use crate::raster::Rect;
    use crate::text::TextRenderer;
    use crate::theme::{FontConfig, Theme};

    fn dark() -> Theme {
        theme_with(
            Interface::Glass,
            crate::blue::Variant::default(),
            &crate::glass::Settings {
                mode: crate::glass::Mode::Dark,
                ..crate::glass::Settings::default()
            },
        )
    }

    #[track_caller]
    fn pin(c: &Canvas, x: i32, y: i32, rgb: u32, what: &str) {
        assert_eq!(
            c.get(x, y),
            0xFF00_0000 | rgb,
            "{what}: ({x}, {y}) is {rgb:06X}"
        );
    }

    fn tip_over(theme: &Theme, label: &str, back: u32) -> (Canvas, (i32, i32)) {
        let text = TextRenderer::load(&FontConfig::default());
        let tip = draw_tip(theme, &text, label);
        let mut c = Canvas::new(tip.width, tip.height);
        let [_, r, g, b] = back.to_be_bytes();
        c.fill_rect(c.bounds(), Color::rgb(r, g, b));
        c.blit_over(0, 0, &tip);
        (c, tip_size(&text, theme, label))
    }

    #[test]
    fn a_glass_tip_is_a_shaded_box_with_round_corners_and_a_shadow() {
        let theme = theme_for(Interface::Glass);
        let (c, (w, h)) = tip_over(&theme, "Contains the files you deleted", 0xFFFFFF);
        assert_eq!(h, 20, "one line makes a 20 px box");
        assert_eq!((c.width as i32, c.height as i32), (w + 5, h + 5));
        for x in [0, w - 1] {
            assert!(c.get(x, 0) & 0xFF >= 0xF8, "the top corner at {x} is round");
        }
        pin(
            &c,
            w - 1,
            h - 1,
            0x8F8F8F,
            "the shadow shows through the bottom-right corner",
        );
        pin(&c, 5, 0, 0x727373, "the top edge");
        pin(&c, 0, 10, 0x727373, "the left edge");
        pin(&c, w - 1, 10, 0x727373, "the right edge");
        pin(&c, 5, h - 1, 0x727373, "the bottom edge");
        pin(&c, 5, 1, 0xFFFFFF, "the body's top row");
        pin(&c, 5, h - 2, 0xE3E3EF, "the body's bottom row");
        pin(&c, w, 12, 0x939393, "the shadow beside the box");
        pin(&c, w / 2, h, 0x939393, "the shadow under the box");
        pin(&c, w + 4, 2, 0xFFFFFF, "the shadow starts 4 rows down");
        pin(&c, 2, h + 1, 0xFFFFFF, "and 5 columns in");
        let (c, _) = tip_over(&theme, "Contains the files you deleted", 0);
        pin(&c, w, 12, 0, "the shadow leaves black black");
    }

    #[test]
    fn a_glass_tip_inks_575757_text_inside_its_box() {
        let theme = theme_for(Interface::Glass);
        let (c, (w, h)) = tip_over(&theme, "HHH", 0xFFFFFF);
        let ink = |x: i32, y: i32| c.get(x, y) == 0xFF54_5555;
        let inked: Vec<(i32, i32)> = (1..h - 1)
            .flat_map(|y| (1..w - 1).map(move |x| (x, y)))
            .filter(|&(x, y)| ink(x, y))
            .collect();
        assert!(!inked.is_empty(), "the text inks 575757");
        let left = inked.iter().map(|p| p.0).min().unwrap();
        let top = inked.iter().map(|p| p.1).min().unwrap();
        assert!((5..=7).contains(&left), "the ink starts 5 px in, at {left}");
        assert_eq!(top, 5, "the capitals' tops are 5 px down");
        let (_, (_, two)) = tip_over(&theme, "Volume\nMuted", 0xFFFFFF);
        assert_eq!(two, 33, "a second line adds 13 px");
    }

    #[test]
    fn a_dark_glass_tip_is_dark_with_light_text() {
        let theme = dark();
        let mut c = Canvas::new(40, 30);
        let ink = tooltip::draw_box(&mut c, &theme.paint(), Rect::new(0, 0, 30, 20));
        assert_eq!(ink, crate::glass::dark::remap_color(TOOLTIP_INK));
        assert_eq!(ink.argb(), 0xFF9A_9B9B, "the ink is 999999");
        let body = c.get(5, 10) & 0xFF;
        assert!(body < 0x60, "the body is dark: {body:02X}");
        assert_eq!(c.get(31, 12) & 0x00FF_FFFF, 0, "the shadow is black");
        assert!(c.get(31, 12) >> 24 > 0x40, "and still there");
    }

    #[test]
    fn the_glass_weekday_band_is_white_over_one_f5f5f5_row() {
        let theme = theme_for(Interface::Glass);
        let r = Rect::new(2, 2, 158, 18);
        for (enabled, fill, letters) in [
            (true, 0xFFFFFF, theme.colors.window_text),
            (false, 0xEFEEEF, theme.colors.gray_text),
        ] {
            let mut c = Canvas::new(162, 22);
            c.fill_rect(c.bounds(), theme.colors.button_face);
            let ink = draw_calendar_band(&mut c, &theme.paint(), r, enabled);
            assert_eq!(ink, letters, "the letters, {enabled}");
            pin(&c, r.x, r.y, fill, "the band's top-left");
            pin(&c, 80, r.y + 16, fill, "the band over its foot");
            let foot = r.bottom() - 1;
            pin(&c, r.x + 4, foot, fill, "the row starts 5 px in");
            pin(&c, r.x + 5, foot, 0xF4F5F5, "the row's first pixel");
            pin(&c, r.right() - 6, foot, 0xF4F5F5, "the row's last pixel");
            pin(&c, r.right() - 5, foot, fill, "the row stops 5 px short");
        }
        assert_eq!(
            theme.colors.window_text.argb(),
            0xFF00_0000,
            "the letters are black"
        );
    }

    #[test]
    fn the_glass_places_bar_is_a_flat_f4f4f4_panel() {
        let theme = theme_for(Interface::Glass);
        let mut c = Canvas::new(91, 140);
        c.fill_rect(c.bounds(), theme.colors.button_face);
        let r = Rect::new(2, 2, 87, 136);
        draw_panel(&mut c, &theme.paint(), r);
        for (x, y) in [(2, 2), (88, 137), (45, 70), (2, 137), (88, 2)] {
            pin(&c, x, y, 0xF3F4F4, "the panel");
        }
        pin(&c, 1, 70, 0xEFEEEF, "the face beside it");
    }

    #[test]
    fn the_glass_calendar_frame_is_one_979797_line() {
        let theme = theme_for(Interface::Glass);
        for enabled in [true, false] {
            let mut c = Canvas::new(166, 134);
            c.fill_rect(c.bounds(), theme.colors.button_face);
            let r = Rect::new(2, 2, 162, 130);
            c.fill_rect(r, theme.colors.window);
            draw_calendar_frame(&mut c, &theme.paint(), r, enabled);
            for (x, y) in [
                (2, 2),
                (80, 2),
                (163, 2),
                (2, 60),
                (163, 60),
                (2, 131),
                (80, 131),
                (163, 131),
            ] {
                pin(&c, x, y, 0x949595, "the line");
            }
            for (x, y) in [(3, 3), (80, 3), (162, 60), (80, 130)] {
                pin(&c, x, y, 0xFFFFFF, "the white inside it");
            }
            pin(&c, 1, 60, 0xEFEEEF, "the face outside it");
        }
    }

    #[test]
    fn the_glass_selected_places_tile_is_sunk_into_a_grey_box() {
        let theme = theme_for(Interface::Glass);
        let mut c = Canvas::new(91, 69);
        c.fill_rect(c.bounds(), theme.colors.button_face);
        let bar = Rect::new(2, 2, 87, 65);
        draw_panel(&mut c, &theme.paint(), bar);
        draw_selected(&mut c, &theme.paint(), bar);
        let (l, r, top, foot) = (3, 87, 2, 66);
        pin(&c, 2, 30, 0xF3F4F4, "the bar left of the box");
        pin(&c, 88, 30, 0xF3F4F4, "the bar right of it");
        pin(&c, l, top, 0xBCBDBD, "the top-left corner blends");
        pin(&c, r, top, 0xBCBDBD, "the top-right corner blends");
        pin(&c, 40, top, 0xAAABAB, "the outline's top");
        pin(&c, l, 30, 0xAAABAB, "its left side");
        pin(&c, r, 30, 0xAAABAB, "its right side");
        pin(&c, 40, top + 1, 0x6F6F6F, "the line along the top");
        pin(&c, l + 1, top + 1, 0x7B7B7B, "its ends");
        pin(&c, l + 1, top + 2, 0x6F6F6F, "the left side starts dark");
        pin(&c, r - 1, top + 2, 0x6F6F6F, "and so does the right");
        pin(&c, l + 1, foot - 2, 0xBBBABB, "the side ends light");
        pin(&c, 40, top + 2, 0xC1C2C2, "the face's first row");
        pin(&c, 40, top + 27, 0xE3E2E3, "the face after 26 rows");
        pin(&c, 40, foot - 1, 0xE3E2E3, "the face over the foot");
        pin(&c, 40, foot, 0x787979, "the line closing the foot");
        pin(&c, l, foot - 1, 0xE1E1E1, "the outline fading");
        pin(&c, l, foot, 0xF3F4F4, "and gone beside the foot");
    }

    #[test]
    fn the_glass_popup_parts_double_at_scale_2() {
        for theme in [theme_for(Interface::Glass), dark()] {
            let draw = |s: i32| {
                let p = crate::look::Paint { s, ..theme.paint() };
                let mut c = Canvas::new((120 * s) as u32, (110 * s) as u32);
                c.fill_rect(c.bounds(), theme.colors.button_face);
                draw_calendar_band(&mut c, &p, Rect::new(70, 2, 40, 18).scaled(s), true);
                draw_calendar_band(&mut c, &p, Rect::new(70, 20, 40, 18).scaled(s), false);
                draw_panel(&mut c, &p, Rect::new(112, 2, 6, 36).scaled(s));
                draw_calendar_frame(&mut c, &p, Rect::new(40, 40, 30, 20).scaled(s), true);
                draw_selected(&mut c, &p, Rect::new(2, 40, 30, 65).scaled(s));
                c
            };
            let name = theme.look.style().unwrap().name;
            assert!(doubles_at_scale_1(&draw(1), &draw(2)), "{name}");
        }
    }

    fn drawn(d: i32) -> Vec<(&'static str, PartImage)> {
        let tip = |dark: bool| {
            let img = parts(d).into_iter().next().unwrap().1;
            if dark {
                crate::glass::dark::remap_part(img)
            } else {
                img
            }
        };
        vec![
            ("tooltip", tip(false)),
            ("dark tooltip", tip(true)),
            ("close", preview_close(d)),
        ]
    }

    fn fnv(c: &Canvas) -> u64 {
        [c.width, c.height]
            .into_iter()
            .chain(
                (0..c.height as i32)
                    .flat_map(|y| (0..c.width as i32).map(move |x| (x, y)))
                    .map(|(x, y)| c.get(x, y)),
            )
            .flat_map(u32::to_le_bytes)
            .fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
                (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
            })
    }

    #[test]
    fn the_tooltip_and_close_button_are_drawn_again_at_scale_2() {
        let (one, two) = (drawn(1), drawn(2));
        for ((name, a), (_, b)) in one.iter().zip(&two) {
            assert_eq!(
                (b.atlas.width, b.atlas.height),
                (2 * a.atlas.width, 2 * a.atlas.height),
                "{name}: twice the size"
            );
            assert_eq!((a.scale, b.scale), (1, 2), "{name}");
            assert_eq!(
                (a.sizing, a.content, a.outset),
                (b.sizing, b.content, b.outset),
                "{name}: the same design-px insets"
            );
            assert!(
                a.atlas.get(0, 0) >> 24 < 0x60,
                "{name}: a mostly clear corner"
            );
            let soft = (0..6).any(|y| {
                (0..6).any(|x| {
                    let al = b.atlas.get(x, y) >> 24;
                    al > 0 && al < 0xFF
                })
            });
            assert!(soft, "{name}: the corner is soft at 2");
        }
        for (name, img) in one.iter().chain(&two).filter(|(n, _)| !n.contains("dark")) {
            let line = if *name == "close" {
                CLOSE_RIM
            } else {
                TIP_LINE
            };
            let n = img.atlas.width as i32 / if *name == "close" { 3 } else { 4 };
            for y in 0..n {
                for x in 0..n {
                    let p = crate::shape::unpack(img.atlas.get(x, y));
                    let on_white = 255.0 - p.0 + p.1[1];
                    assert!(
                        on_white >= f64::from(line.g) - 1.0,
                        "{name} ({x}, {y}) is darker than its line over white"
                    );
                }
            }
        }
        let close = &one[2].1.atlas;
        let lightest = |x: i32, y: i32| (close.get(x, y) & 0xFF) as u8;
        for (x, y) in [(4, 4), (5, 4), (8, 4), (9, 4), (6, 6), (7, 6)] {
            assert!(lightest(x, y) >= 0xE8, "the X is near white at ({x}, {y})");
        }
        for (x, y) in [(4, 9), (5, 9), (8, 9), (9, 9)] {
            assert!(lightest(x, y) >= 0xC8, "its foot is light at ({x}, {y})");
        }
        for (x, y) in [(6, 4), (7, 4), (6, 9), (7, 9)] {
            assert!(lightest(x, y) < 0x90, "the outline between at ({x}, {y})");
        }
    }

    #[test]
    fn the_tooltip_and_close_button_pin_at_scale_1_and_2() {
        let want = [
            (1, "tooltip", 7100469417102715413),
            (1, "dark tooltip", 1418182114977198849),
            (1, "close", 4960589212196123621),
            (2, "tooltip", 16607512807499100920),
            (2, "dark tooltip", 14632703910114294106),
            (2, "close", 11477109601501468505),
        ];
        let got: Vec<_> = [1, 2]
            .into_iter()
            .flat_map(|d| {
                drawn(d)
                    .into_iter()
                    .map(move |(n, img)| (d, n, fnv(&img.atlas)))
            })
            .collect();
        assert_eq!(got, want);
    }
}
