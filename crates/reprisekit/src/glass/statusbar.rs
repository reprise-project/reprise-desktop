//! Glass's status bar, pane and size grip parts.
//! The bar is 23 rows: a line along its top, a white row, a flat face and a white bottom row, with a white column down each end. A pane is no box: one divider column at its right end, the bar showing elsewhere. The size grip is six 2x2 dots in a triangle, each lit white at its upper left and shaded grey to its lower right, corners eased, in an 8x8 box one column and row short of the bar's white ends; drawn at the output scale.

use crate::raster::{Canvas, Insets, Rect};
use crate::shape::{self, Px, Stops, centre, premul, rounded};
use crate::style::{Layout, Part, PartImage, State};
use crate::theme::Color;

pub const BAR_LINE: Color = Color::rgb(0x8D, 0x8E, 0x8E);
pub const BAR_FACE: Color = Color::rgb(0xF0, 0xED, 0xEC);
const WHITE: Color = Color::rgb(0xFF, 0xFF, 0xFF);
pub const PANE_LINE: Color = Color::rgb(0xD6, 0xD5, 0xD6);

const BAR_W: i32 = 3;
const BAR_H: i32 = 23;
/// Where a pane's icon and text box starts under the bar's top, so the
/// text's capitals start on row 8.
const CONTENT_TOP: i32 = 3;
/// Where a pane's icon box ends over the bar's bottom: the box is 18 rows,
/// so a 16 px icon starts a row under the content top.
const CONTENT_BOTTOM: i32 = 2;

const GRIP: i32 = 9;
const DOT: f64 = 2.0;
const DOTS: [(i32, i32); 6] = [(6, 0), (3, 3), (6, 3), (0, 6), (3, 6), (6, 6)];
const DOT_RADIUS: f64 = 0.5;
pub const GRIP_LIGHT: Color = Color::rgb(0xFF, 0xFF, 0xFF);
pub const GRIP_SHADE: Color = Color::rgb(0xAC, 0xAD, 0xAD);

pub fn parts(d: i32) -> [(Part, PartImage); 3] {
    let mut a = Canvas::new(BAR_W as u32, BAR_H as u32);
    a.fill_rect(a.bounds(), WHITE);
    a.fill_rect(Rect::new(0, 0, BAR_W, 1), BAR_LINE);
    a.fill_rect(Rect::new(1, 2, 1, BAR_H - 3), BAR_FACE);
    let bar = PartImage::new(a, &[State::Normal], Layout::Vertical).sizing(Insets::new(1, 2, 1, 1));

    // One clear column, then the divider down the face rows: a nine-grid
    // whose right column keeps the divider and whose top two rows and
    // bottom row stay clear over the bar's own.
    let mut a = Canvas::new(2, BAR_H as u32);
    a.fill_rect(Rect::new(1, 2, 1, BAR_H - 3), PANE_LINE);
    let pane = PartImage::new(a, &[State::Normal], Layout::Vertical)
        .sizing(Insets::new(0, 2, 1, 1))
        .content(Insets::new(0, CONTENT_TOP, 0, CONTENT_BOTTOM));

    [
        (Part::StatusBar, bar),
        (Part::StatusPane, pane),
        (Part::SizeBox, size_grip(d)),
    ]
}

fn size_grip(d: i32) -> PartImage {
    let k = f64::from(d);
    let shade = [
        (0.0, premul(GRIP_LIGHT, 255)),
        (1.0, premul(GRIP_SHADE, 255)),
    ];
    let frame = shape::draw(GRIP, GRIP, d, |x, y| {
        let (cx, cy) = (centre(x as i32, d), centre(y as i32, d));
        DOTS.iter()
            .map(|&(ox, oy)| {
                let (ox, oy) = (f64::from(ox), f64::from(oy));
                let cover = rounded(x - ox * k, y - oy * k, DOT * k, DOT * k, DOT_RADIUS * k);
                let t = (cx - ox + cy - oy) / 2.0;
                shape::scale(Stops(&shade).at(t), cover)
            })
            .fold(shape::CLEAR, |a: Px, b| shape::over(b, a))
    });
    let mut grip = PartImage::new(frame, &[State::Normal], Layout::Vertical).at_scale(d);
    grip.true_size = true;
    grip
}

#[cfg(test)]
mod tests {
    use crate::glass::dark::remap_part;
    use crate::interface::Interface;
    use crate::interfaces::theme_for;
    use crate::look::styled::fixture::doubles_at_paint_scale_2;
    use crate::look::widgets::draw_size_grip;
    use crate::look::widgets::statusbar::{draw_bar, draw_grip, draw_pane};
    use crate::raster::{Canvas, Rect};
    use crate::style::PartImage;

    const H: i32 = 23;

    #[track_caller]
    fn pin(c: &Canvas, x: i32, y: i32, rgb: u32, what: &str) {
        assert_eq!(
            c.get(x, y),
            0xFF00_0000 | rgb,
            "{what}: ({x}, {y}) is {rgb:06X}"
        );
    }

    fn bar(grip: bool) -> Canvas {
        let theme = theme_for(Interface::Glass);
        let p = theme.paint();
        let mut c = Canvas::new(300, H as u32);
        let r = Rect::new(0, 0, 300, H);
        draw_bar(&mut c, &p, r);
        draw_pane(
            &mut c,
            &p,
            r,
            Rect::new(0, 2, 100, 18),
            Some(102),
            grip,
            true,
        );
        if grip {
            draw_grip(&mut c, &p, r, Rect::new(283, 3, 16, 16));
        }
        c
    }

    #[test]
    fn the_glass_status_bar_is_a_grey_line_over_white_round_its_face() {
        assert_eq!(theme_for(Interface::Glass).look.controls().status_bar, H);
        let c = bar(false);
        for (x, y, rgb, what) in [
            (0, 0, 0x8D8E8E, "the top line's first pixel"),
            (150, 0, 0x8D8E8E, "the top line"),
            (150, 1, 0xFFFFFF, "the white row under it"),
            (150, 2, 0xF0EDEC, "the face's first row"),
            (150, 21, 0xF0EDEC, "the face's last row"),
            (150, 22, 0xFFFFFF, "the white bottom row"),
            (0, 10, 0xFFFFFF, "the white left end"),
            (299, 10, 0xFFFFFF, "the white right end"),
        ] {
            pin(&c, x, y, rgb, what);
        }
    }

    #[test]
    fn a_glass_status_pane_is_one_d7d7d7_column() {
        let c = bar(false);
        for y in 2..=21 {
            pin(&c, 98, y, 0xD6D5D6, "the divider");
            pin(&c, 97, y, 0xF0EDEC, "the face before it");
            pin(&c, 99, y, 0xF0EDEC, "the face after it");
        }
        pin(&c, 98, 1, 0xFFFFFF, "the bar's white row over it");
        pin(&c, 98, 22, 0xFFFFFF, "the bar's white row under it");
        pin(&c, 0, 10, 0xFFFFFF, "no edge round the pane");
    }

    fn light(px: u32) -> u32 {
        (px >> 16 & 0xFF) + (px >> 8 & 0xFF) + (px & 0xFF)
    }

    #[test]
    fn the_glass_size_grip_sits_inside_the_bar_s_white_ends() {
        let c = bar(true);
        for (ox, oy, what) in [
            (296, 19, "the corner dot"),
            (296, 13, "the top dot"),
            (290, 19, "the left dot"),
        ] {
            let (lit, side, shade) = (c.get(ox, oy), c.get(ox + 1, oy), c.get(ox + 1, oy + 1));
            assert!(light(lit) > light(side), "{what}: lit at its upper left");
            assert!(
                light(side) > light(shade),
                "{what}: shaded at its lower right"
            );
            assert!(
                light(lit) > 3 * 0xF8,
                "{what}: near white at its upper left"
            );
        }
        for (x, y, rgb, what) in [
            (290, 13, 0xF0EDEC, "clear above the left dot"),
            (298, 19, 0xF0EDEC, "the face right of the dots"),
            (297, 21, 0xF0EDEC, "the face under the dots"),
            (299, 19, 0xFFFFFF, "the bar's white end"),
        ] {
            pin(&c, x, y, rgb, what);
        }
    }

    #[test]
    fn a_glass_status_bar_and_grip_double_at_paint_scale_2() {
        assert!(doubles_at_paint_scale_2(
            Interface::Glass,
            240,
            50,
            |c, p| {
                let s = p.s;
                for (i, grip) in [true, false].into_iter().enumerate() {
                    let r = Rect::new(2, 2 + 24 * i as i32, 200, H).scaled(s);
                    draw_bar(c, p, r);
                    let pane = Rect::new(2, 4 + 24 * i as i32, 100, 18).scaled(s);
                    draw_pane(c, p, r, pane, Some(104 * s), grip, true);
                    let last = Rect::new(104, 4 + 24 * i as i32, 98, 18).scaled(s);
                    draw_pane(c, p, r, last, None, grip, false);
                    if grip {
                        draw_grip(c, p, r, Rect::new(185, 5, 16, 16).scaled(s));
                    }
                }
                draw_size_grip(c, p, Rect::new(210, 2, 17, 17).scaled(s));
            }
        ));
    }

    fn grip(d: i32) -> PartImage {
        let [_, _, (_, g)] = super::parts(d);
        g
    }

    #[test]
    fn the_size_grip_is_its_design_size_and_smooth_at_scale_2() {
        for d in [1, 2] {
            let g = grip(d);
            assert!(g.true_size);
            let n = 9 * d;
            assert_eq!(
                (g.scale, g.atlas.width, g.atlas.height),
                (d, n as u32, n as u32)
            );
            for (x, y) in [(0, 0), (n - 1, 0), (n - 1, n - 1)] {
                assert_eq!(g.atlas.get(x, y), 0, "at {d}: ({x}, {y}) is clear");
            }
        }
        let soft = |d| {
            let g = grip(d);
            g.atlas
                .pixels
                .iter()
                .filter(|&&p| (1..0xFF).contains(&(p >> 24)))
                .count()
        };
        assert_eq!(soft(1), 24, "four corners on each of six dots at 1");
        assert!(soft(2) >= 24, "the corners blend at 2 ({} px)", soft(2));
    }

    #[test]
    fn the_scale_2_style_draws_the_grip_at_its_design_size() {
        let theme = theme_for(Interface::Glass);
        let look = theme.look.at_scale(2);
        let p = crate::paint::Paint {
            look: &look,
            s: 2,
            ..theme.paint()
        };
        let mut c = Canvas::new(60, 2 * H as u32);
        let r = Rect::new(0, 0, 60, 2 * H);
        draw_bar(&mut c, &p, r);
        let under = c.clone();
        draw_grip(&mut c, &p, r, Rect::new(26, 6, 32, 32));
        let changed: Vec<(i32, i32)> = (0..2 * H)
            .flat_map(|y| (0..60).map(move |x| (x, y)))
            .filter(|&(x, y)| c.get(x, y) != under.get(x, y))
            .collect();
        let (x0, y0) = changed
            .iter()
            .fold((60, 99), |(a, b), &(x, y)| (a.min(x), b.min(y)));
        let (x1, y1) = changed
            .iter()
            .fold((0, 0), |(a, b), &(x, y)| (a.max(x), b.max(y)));
        assert_eq!(
            (x1, y1),
            (60 - 2 - 2 - 1, 2 * H - 2 - 2 - 1),
            "the last dot's corner"
        );
        assert_eq!((x1 - x0 + 1, y1 - y0 + 1), (16, 16), "8 design px of dots");
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
    fn size_grip_pins_at_scale_1_and_2() {
        let got = GRIP_PINS.map(|(d, dark, _)| {
            let g = if dark { remap_part(grip(d)) } else { grip(d) };
            (d, dark, fnv(&g.atlas))
        });
        assert!(
            got == GRIP_PINS,
            "pins changed; now:\n{}",
            got.map(|(d, dark, h)| format!("({d}, {dark}, {h:#018x}),"))
                .join("\n")
        );
    }

    const GRIP_PINS: [(i32, bool, u64); 4] = [
        (1, false, 0x8fe87ce460e87211),
        (2, false, 0x505dc486efdc0989),
        (1, true, 0x7cffaf4b0a75da35),
        (2, true, 0x228b6a0bdc47ac6d),
    ];
}
