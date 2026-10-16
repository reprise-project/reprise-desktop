//! A task button's progress in Classic: a thin bar in the selection colour along the bottom,
//! inside the button's edge; grey while paused.

use crate::raster::{Canvas, Rect};
use crate::taskbar::TaskProgress;
use crate::theme::ColorScheme;

const INSET_X: i32 = 4;
/// From the button's bottom to the bar's top.
const BOTTOM: i32 = 4;
const BAR_H: i32 = 2;

pub fn paint(p: &TaskProgress, cs: &ColorScheme, w: i32, h: i32, d: i32) -> Canvas {
    let mut c = Canvas::new((w * d).max(0) as u32, (h * d).max(0) as u32);
    let ink = if p.paused { cs.gray_text } else { cs.hilight };
    let span = (w - 2 * INSET_X).max(0);
    let filled = (f64::from(span) * p.fraction.clamp(0.0, 1.0)).round() as i32;
    c.fill_rect(
        Rect::new(INSET_X * d, (h - BOTTOM) * d, filled * d, BAR_H * d),
        ink,
    );
    c
}

#[cfg(test)]
mod tests {
    use crate::blue::Variant;
    use crate::interface::Interface;
    use crate::interfaces::{blue_theme, theme_with};
    use crate::raster::Canvas;
    use crate::taskbar::{ProgressPainter, TaskProgress};
    use crate::theme::Theme;

    const W: i32 = 150;
    const H: i32 = 24;

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

    fn looks() -> Vec<(String, Theme, ProgressPainter)> {
        let styled = |t: &Theme| {
            t.look
                .style()
                .and_then(|s| s.task_progress.clone())
                .unwrap()
        };
        let classic: ProgressPainter = std::sync::Arc::new(super::paint);
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

    const STATES: [(&str, f64, bool); 4] = [
        ("empty", 0.0, false),
        ("copying", 0.4, false),
        ("paused", 0.4, true),
        ("done", 1.0, false),
    ];

    /// The art in each look, Blue scheme and Glass mode, per state, at 1 and 2.
    #[test]
    fn the_task_progress_is_pinned_per_look_and_state() {
        let mut got = Vec::new();
        for (name, theme, paint) in looks() {
            let mut drawn = Vec::new();
            for (state, fraction, paused) in STATES {
                for d in [1, 2] {
                    let p = TaskProgress { fraction, paused };
                    let c = paint(&p, &theme.colors, W, H, d);
                    assert_eq!(c.device_size(), ((W * d) as u32, (H * d) as u32));
                    drawn.push(fnv(&c));
                    got.push((format!("{name} {state} @{d}"), fnv(&c)));
                }
            }
            let distinct: std::collections::HashSet<_> = drawn.iter().collect();
            assert_eq!(
                distinct.len(),
                drawn.len(),
                "{name}: every state draws apart"
            );
        }
        crate::pins::check("task_progress", &got);
    }

    #[test]
    fn the_art_keeps_off_the_button_edge_and_rounds_its_ends_at_2() {
        let p = TaskProgress {
            fraction: 0.6,
            paused: false,
        };
        for (name, theme, paint) in looks() {
            let d = 2;
            let c = paint(&p, &theme.colors, W, H, d);
            let edge = 2 * d as u32;
            let (w, h) = c.device_size();
            for y in 0..h {
                for x in 0..w {
                    if x < edge || y < edge || x >= w - edge || y >= h - edge {
                        assert_eq!(
                            c.get(x as i32, y as i32) >> 24,
                            0,
                            "{name}: {x},{y} on the edge"
                        );
                    }
                }
            }
            let soft = c
                .pixels
                .iter()
                .filter(|&&px| (1..255).contains(&(px >> 24)))
                .count();
            if !name.starts_with("classic") {
                assert!(soft > 4, "{name}: {soft} partly covered pixels");
            }
        }
    }
}
