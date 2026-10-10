use super::Mode;
use super::dark::remap_color;
use super::panel::{self, Fade, LightBox};
use crate::style::{BalloonLook, DropShadow, Layout, Part, PartImage, State};
use crate::theme::Color;

const CROSS: [u16; 8] = [
    0b1_1100_0111,
    0b0_1110_1110,
    0b0_0111_1100,
    0b0_0011_1000,
    0b0_0011_1000,
    0b0_0111_1100,
    0b0_1110_1110,
    0b1_1100_0111,
];

const LOOK: BalloonLook = BalloonLook {
    frame: Color::rgb(0, 0, 0),
    fill: (Color::rgb(0xFF, 0xFF, 0xFF), Color::rgb(0xE9, 0xEA, 0xF4)),
    corner: &[5, 3, 2, 1, 1, 0],
    tail: (16, 19, 19),
    title: Color::rgb(0x00, 0x2A, 0x95),
    title_px: 16,
    title_y: 11,
    text: Color::rgb(0, 0, 0),
    body_y: 32,
    line_h: 15,
    bottom: 8,
    icon_at: (13, 12),
    text_x: 33,
    right: 10,
    close: (27, 9, 17, 16),
    cross_at: (4, 4),
    cross: &CROSS,
    cross_ink: (Color::rgb(0x93, 0x94, 0x94), Color::rgb(0xB4, 0x41, 0x3B)),
    shadow: None,
};

pub fn look(mode: Mode, shadow: Option<&'static DropShadow>) -> BalloonLook {
    let light = BalloonLook { shadow, ..LOOK };
    match mode {
        Mode::Light => light,
        Mode::Dark => BalloonLook {
            frame: remap_color(light.frame),
            fill: (remap_color(light.fill.0), remap_color(light.fill.1)),
            title: remap_color(light.title),
            text: remap_color(light.text),
            cross_ink: (remap_color(light.cross_ink.0), light.cross_ink.1),
            ..light
        },
    }
}

const fn ink(v: u32) -> (Color, u8) {
    (Color::rgb((v >> 16) as u8, (v >> 8) as u8, v as u8), 255)
}

const CLOSE_HOT: LightBox = LightBox {
    outline: ink(0x939494),
    line: Some((ink(0xFFFFFF), ink(0xE9EAEC))),
    fill: (ink(0xF7F8F8), ink(0xDEDFE2)),
    radius: 2.0,
    fade: Fade::Ring,
};

pub fn parts(d: i32) -> [(Part, PartImage); 1] {
    let (_, _, w, h) = LOOK.close;
    let a = panel::light_box(&CLOSE_HOT, w, h, d);
    let close = PartImage::new(a, &[State::Hot], Layout::Vertical).at_scale(d);
    [(Part::BalloonClose, close)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controls::tooltip::{
        NotifyHit, balloon_size, draw_balloon_tip, draw_notify_balloon, notify_hit, notify_layout,
    };
    use crate::interface::Interface;
    use crate::interfaces::{theme_for, theme_with};
    use crate::raster::{Canvas, Rect};
    use crate::taskbar::Edge;
    use crate::text::TextRenderer;
    use crate::theme::Theme;

    fn rgb(v: u32) -> u32 {
        0xFF00_0000 | v
    }

    fn text(theme: &Theme) -> TextRenderer {
        TextRenderer::load(&theme.fonts)
    }

    fn frame_row(c: &Canvas, y: i32, xs: std::ops::Range<i32>) -> String {
        xs.map(|x| match c.get(x, y) {
            0 => ' ',
            v if v == rgb(0) => '#',
            _ => '.',
        })
        .collect()
    }

    #[test]
    fn the_notification_balloon_is_a_shaded_bubble_with_stepped_corners() {
        let theme = theme_for(Interface::Glass);
        let t = text(&theme);
        let lay = notify_layout(&theme, &t, "Update", "Ready to install.", &[], None);
        assert_eq!((lay.bubble_w, lay.bubble_h), (300, 55), "one body line");
        assert_eq!((lay.width, lay.height), (305, 55 + 19));
        let c = draw_notify_balloon(&theme, &t, &lay, None, None, None, None);
        let corner: Vec<String> = (0..7).map(|y| frame_row(&c, y, 0..7)).collect();
        assert_eq!(
            corner,
            [
                "     ##", "   ###.", "  ##...", " ##....", " #.....", "##.....", "#......"
            ],
            "the top-left corner"
        );
        let foot: Vec<String> = (48..55).rev().map(|y| frame_row(&c, y, 293..300)).collect();
        assert_eq!(
            foot,
            [
                "##     ", ".###   ", "...##  ", "....## ", ".....# ", ".....##", "......#"
            ],
            "the bottom-right corner, turned"
        );
        assert_eq!(c.get(150, 1), rgb(0xFFFFFF), "the fill's first row");
        assert_eq!(c.get(150, 53), rgb(0xE9EAF4), "the fill's last row");
        let out = [0x8D8D8D, 0xABABAB, 0xD4D4D4, 0xF2F2F2, 0xFDFDFD];
        for (i, &v) in out.iter().enumerate() {
            let k = i as i32;
            let a = 255 - (v & 0xFF);
            assert_eq!(c.get(300 + k, 30) >> 24, a, "right, {i} out");
            assert_eq!(c.get(100, 55 + k) >> 24, a, "below, {i} out");
        }
        let v = 299 - 16;
        assert_eq!(frame_row(&c, 54, v - 19..v + 2), "##.................##");
        assert_eq!(frame_row(&c, 55, v - 18..v + 1), "##................#");
        assert_eq!(frame_row(&c, 70, v - 4..v + 1), " ##.#");
        assert_eq!(frame_row(&c, 71, v - 2..v + 1), "###");
        assert_eq!(frame_row(&c, 72, v - 2..v + 1), " ##");
        assert_eq!(
            frame_row(&c, 73, v - 2..v + 1),
            "  #",
            "the tail's last row"
        );
        assert_eq!(c.get(v - 3, 56), rgb(0xE9EAF4), "the tail's fill");
    }

    #[test]
    fn icon_text_and_close_cross_sit_where_glass_puts_them() {
        let theme = theme_for(Interface::Glass);
        let t = text(&theme);
        let lay = notify_layout(&theme, &t, "Update", "Ready to install.", &[], None);
        let mut icon = Canvas::new(16, 16);
        icon.fill_rect(icon.bounds(), Color::rgb(0xFA, 12, 0));
        let c = draw_notify_balloon(&theme, &t, &lay, Some(&icon), None, None, None);
        assert_eq!(c.get(13, 12), rgb(0xFA0C00), "the icon's top-left");
        assert_eq!(c.get(28, 27), rgb(0xFA0C00), "the icon's bottom-right");
        assert_ne!(c.get(12, 12), rgb(0xFA0C00));
        let ink = |v: u32, ys: std::ops::Range<i32>| {
            let pts: Vec<(i32, i32)> = ys
                .flat_map(|y| (29..270).map(move |x| (x, y)))
                .filter(|&(x, y)| c.get(x, y) == rgb(v))
                .collect();
            let x = pts.iter().map(|p| p.0).min();
            let y = pts.iter().map(|p| p.1).min();
            (x, y)
        };
        let (tx, ty) = ink(0x002A95, 0..30);
        let (bx, by) = ink(0x000000, 30..50);
        assert!(
            tx.is_some_and(|x| (33..=34).contains(&x)),
            "the title from 33 px in: {tx:?}"
        );
        assert!(
            bx.is_some_and(|x| (33..=34).contains(&x)),
            "the body: {bx:?}"
        );
        if t.has_face("Segoe UI") {
            assert_eq!(
                by.zip(ty).map(|(b, t)| b - t),
                Some(19),
                "the body's capitals 19 px under the title's"
            );
        }
        let cross = |c: &Canvas, ink: u32| -> Vec<String> {
            (13..21)
                .map(|y| {
                    (277..286)
                        .map(|x| if c.get(x, y) == rgb(ink) { '#' } else { '.' })
                        .collect()
                })
                .collect()
        };
        let want = [
            "###...###",
            ".###.###.",
            "..#####..",
            "...###...",
            "...###...",
            "..#####..",
            ".###.###.",
            "###...###",
        ];
        assert_eq!(cross(&c, 0x939494), want, "the cross at rest");
        assert_eq!(lay.close, Rect::new(273, 9, 17, 16), "the close box");

        let hot = draw_notify_balloon(
            &theme,
            &t,
            &lay,
            Some(&icon),
            None,
            Some(NotifyHit::Close),
            None,
        );
        assert_eq!(cross(&hot, 0xB4413B), want, "the cross under the pointer");
        let b = lay.close;
        assert_eq!(hot.get(b.x, b.y + 8), rgb(0x939494), "the box's left side");
        assert_eq!(hot.get(b.x + 8, b.y), rgb(0x939494), "its top");
        assert_eq!(hot.get(b.x + 8, b.bottom() - 1), rgb(0x939494), "its foot");
        assert_eq!(hot.get(b.x + 8, b.y + 1), rgb(0xFFFFFF), "the line inside");
        assert_eq!(hot.get(b.x + 8, b.y + 2), rgb(0xF7F8F8), "the fill's top");
        assert_ne!(hot.get(b.x, b.y), rgb(0x939494), "a rounded corner");
        let rest = draw_notify_balloon(
            &theme,
            &t,
            &lay,
            Some(&icon),
            None,
            Some(NotifyHit::Body),
            None,
        );
        assert_eq!(rest.pixels, c.pixels, "the body lights nothing");
    }

    #[test]
    fn hits_follow_the_glass_layout_on_either_edge() {
        let theme = theme_for(Interface::Glass);
        let t = text(&theme);
        let lay = notify_layout(&theme, &t, "Mail", "New message", &["Open", "Reply"], None);
        let close = lay.close;
        assert_eq!(notify_hit(&lay, close.x, close.y), NotifyHit::Close);
        assert_eq!(
            notify_hit(&lay, close.right() - 1, close.bottom() - 1),
            NotifyHit::Close
        );
        assert_eq!(notify_hit(&lay, close.x - 1, close.y + 4), NotifyHit::Body);
        let [open, reply] = lay.buttons[..] else {
            panic!("two buttons: {:?}", lay.buttons);
        };
        assert_eq!(reply.right(), 290, "10 px in from the right");
        assert_eq!(open.y, 32 + 15 + 8, "8 px under the body's line");
        assert_eq!(reply.bottom(), lay.bubble_h - 10);
        let top = lay.clone().on_edge(Edge::Top);
        assert_eq!(top.bubble_y, 19);
        assert_eq!(top.height, 19 + lay.bubble_h + 5, "the shadow under it");
        assert_eq!(top.close.y, close.y + 19);
        let b = top.buttons[1];
        assert_eq!(notify_hit(&top, b.x + 3, b.y + 3), NotifyHit::Button(1));
        let rest = draw_notify_balloon(&theme, &t, &top, None, None, None, None);
        let lit = draw_notify_balloon(
            &theme,
            &t,
            &top,
            None,
            None,
            Some(NotifyHit::Button(1)),
            None,
        );
        assert_ne!(rest.get(b.x + 5, b.y + 3), lit.get(b.x + 5, b.y + 3), "lit");
        assert_eq!(frame_row(&rest, 0, 280..285), "   # ");
        assert_eq!(frame_row(&rest, 19, 260..267), "######.", "the frame opens");
    }

    #[test]
    fn the_balloon_tip_is_the_same_bubble_under_its_stem() {
        let theme = theme_for(Interface::Glass);
        let t = text(&theme);
        let body = "Ends your session and shuts down the computer.";
        let (w, h) = balloon_size(&t, &theme, "Turn Off", body);
        let c = draw_balloon_tip(&theme, &t, "Turn Off", body);
        assert_eq!((c.width as i32, c.height as i32), (w, h));
        assert_eq!(h, 19 + 32 + 15 + 8 + 5, "stem, one line, shadow");
        assert_eq!(frame_row(&c, 0, 15..18), " # ", "the stem's tip");
        assert_eq!(frame_row(&c, 19, 14..18), "###.", "the frame opens");
        assert_eq!(frame_row(&c, 19, 0..6), "     #", "the stepped corner");
        assert_ne!(c.get(w - 3, h / 2), 0, "the shadow");
        let has = |v: u32| (0..h).any(|y| (13..w).any(|x| c.get(x, y) == rgb(v)));
        assert!(has(0x002A95), "the title");
        let left = (19 + 30..19 + 48)
            .flat_map(|y| (2..w - 8).map(move |x| (x, y)))
            .filter(|&(x, y)| c.get(x, y) == rgb(0))
            .map(|(x, _)| x)
            .min();
        assert!(
            left.is_some_and(|x| (13..=14).contains(&x)),
            "the body: {left:?}"
        );
    }

    #[test]
    fn dark_and_scale_2_balloons() {
        let dark = theme_with(
            Interface::Glass,
            crate::blue::Variant::default(),
            &super::super::Settings {
                mode: Mode::Dark,
                ..super::super::Settings::default()
            },
        );
        let t = text(&dark);
        let lay = notify_layout(&dark, &t, "Update", "Ready.", &[], None);
        let c = draw_notify_balloon(&dark, &t, &lay, None, None, None, None);
        assert_eq!(c.get(150, 53), remap_color(LOOK.fill.1).argb(), "dark fill");
        assert_eq!(c.get(150, 0), remap_color(LOOK.frame).argb(), "dark frame");

        let theme = theme_for(Interface::Glass);
        let t = text(&theme);
        let two = theme.scaled(2);
        let lay = notify_layout(&two, &t, "Update", "Ready.", &[], None);
        assert_eq!((lay.bubble_w, lay.bubble_h), (600, 110));
        assert_eq!(lay.close, Rect::new(546, 18, 34, 32));
        let c = draw_notify_balloon(&two, &t, &lay, None, None, None, None);
        assert_eq!(frame_row(&c, 0, 8..12), "  ##", "the corner, doubled");
        assert_eq!(frame_row(&c, 2, 4..12), "  ######");
        let v = 598 - 32;
        assert_eq!(frame_row(&c, 109 + 38, v..v + 2), "##", "the tail's tip");
    }
}
