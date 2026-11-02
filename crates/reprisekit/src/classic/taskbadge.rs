//! A task button's unread badge in Classic: a square red box with a dark rim round white numerals.

use crate::raster::Canvas;
use crate::taskbar::BadgeArt;
use crate::theme::{Color, ColorScheme};

const RED: Color = Color::rgb(0xC0, 0x10, 0x10);
const RIM: Color = Color::rgb(0x60, 0x00, 0x00);
const WHITE: Color = Color::rgb(0xFF, 0xFF, 0xFF);

const ART: BadgeArt = BadgeArt {
    height: 9,
    pad_x: 2,
    radius: 0.0,
    rim: (RIM, 0xFF),
    fill: [RED, RED],
    sheen: (WHITE, 0),
    ink: WHITE,
};

pub fn paint(text: &str, _: &ColorScheme, d: i32) -> Canvas {
    ART.paint(text, d)
}

#[cfg(test)]
mod tests {
    use crate::blue::Variant;
    use crate::interface::Interface;
    use crate::interfaces::{blue_theme, theme_with};
    use crate::raster::Canvas;
    use crate::taskbar::BadgePainter;
    use crate::theme::Theme;

    fn fnv(c: &Canvas) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        for v in [c.width, c.height]
            .into_iter()
            .chain(c.pixels.iter().copied())
        {
            for b in v.to_le_bytes() {
                h = (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3);
            }
        }
        h
    }

    fn looks() -> Vec<(String, Theme, BadgePainter)> {
        let styled = |t: &Theme| t.look.style().and_then(|s| s.task_badge.clone()).unwrap();
        let classic: BadgePainter = std::sync::Arc::new(super::paint);
        let mut v = vec![("classic".to_string(), Theme::default(), classic)];
        for b in Variant::ALL {
            let t = blue_theme(b);
            let p = styled(&t);
            v.push((format!("blue {b:?}"), t, p));
        }
        for mode in [crate::glass::Mode::Light, crate::glass::Mode::Dark] {
            let settings = crate::glass::Settings {
                mode,
                ..crate::glass::Settings::default()
            };
            let t = theme_with(Interface::Glass, Variant::default(), &settings);
            let p = styled(&t);
            v.push((format!("glass {mode:?}"), t, p));
        }
        v
    }

    /// The badge in each look, Blue scheme and Glass mode, per count, at 1 and 2.
    #[test]
    fn the_task_badge_is_pinned_per_look_and_count() {
        let mut got = Vec::new();
        for (name, theme, paint) in looks() {
            let mut drawn = Vec::new();
            for text in ["1", "7", "42", "99+"] {
                let one = paint(text, &theme.colors, 1);
                let two = paint(text, &theme.colors, 2);
                assert_eq!(
                    two.device_size(),
                    (one.width * 2, one.height * 2),
                    "{name} {text}: the box doubles exactly at 2"
                );
                for (d, c) in [(1, &one), (2, &two)] {
                    drawn.push(fnv(c));
                    got.push((format!("{name} {text} @{d}"), fnv(c)));
                }
            }
            let distinct: std::collections::HashSet<_> = drawn.iter().collect();
            assert_eq!(
                distinct.len(),
                drawn.len(),
                "{name}: every count draws apart"
            );
        }
        crate::pins::check("task_badge", &got);
    }

    #[test]
    fn the_count_is_white_on_a_badge_in_every_look() {
        for (name, theme, paint) in looks() {
            let c = paint("8", &theme.colors, 1);
            let mid = c.get(c.width as i32 / 2, c.height as i32 / 2);
            assert_eq!(mid, 0xFFFF_FFFF, "{name}: the 8's middle bar is white");
        }
    }
}
