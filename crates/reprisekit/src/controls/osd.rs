//! The media keys' on-screen panel: icon, bold title and a progress bar; under glass the box is glass and the level is also written out.

use crate::look::widgets::draw_progress;
use crate::look::{Paint, panel};
use crate::raster::{Canvas, EdgeStyle, Rect};
use crate::text::{Align, TextRenderer};
use crate::theme::Theme;

pub const W: i32 = 260;
pub const H: i32 = 72;
const PAD: i32 = 12;
const ICON: i32 = 32;
const BAR_H: i32 = 18;
const PERCENT_GAP: i32 = 8;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Meter {
    pub fraction: f64,
    pub enabled: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Osd {
    pub title: String,
    pub meter: Option<Meter>,
}

pub struct Layout {
    pub icon: Rect,
    pub title: Rect,
    pub bar: Rect,
}

pub fn layout() -> Layout {
    let text_x = PAD + ICON + PAD;
    Layout {
        icon: Rect::new(PAD, (H - ICON) / 2, ICON, ICON),
        title: Rect::new(text_x, 16, W - text_x - PAD, 13),
        bar: Rect::new(text_x, 36, W - text_x - PAD, BAR_H),
    }
}

/// The panel for `osd` with `icon` (32 px at the theme's scale). Under a look
/// with glass it is a piece of glass with its shadow (the canvas reaches
/// [`panel::margin`] past the panel), the title white and the meter's level
/// written as a percentage at the title line's right.
pub fn draw(theme: &Theme, text: &TextRenderer, osd: &Osd, icon: Option<&Canvas>) -> Canvas {
    let p = &theme.paint();
    let (cs, s) = (p.cs, p.s);
    let px = theme.fonts.size_px;
    if let Some((style, spec)) = panel::glass(theme) {
        return panel::draw(style, spec, W, H, s, |c| {
            body(c, p, text, px, osd, icon, true)
        });
    }
    let mut c = Canvas::new((W * s) as u32, (H * s) as u32);
    c.fill_rect(c.bounds(), cs.button_face);
    c.edge_t(c.bounds(), EdgeStyle::WindowRaised, cs, s);
    body(&mut c, p, text, px, osd, icon, false);
    c
}

/// The icon, the title and the meter, in panel coordinates; `glass` sets
/// the title in white and adds the percentage.
fn body(
    c: &mut Canvas,
    p: &Paint,
    text: &TextRenderer,
    px: u32,
    osd: &Osd,
    icon: Option<&Canvas>,
    glass: bool,
) {
    let (cs, s) = (p.cs, p.s);
    let lay = layout();
    if let Some(ic) = icon {
        c.blit_over(lay.icon.x * s, lay.icon.y * s, ic);
    }
    let title_r = if osd.meter.is_some() {
        lay.title
    } else {
        Rect::new(lay.title.x, (H - 13) / 2, lay.title.width, 13)
    }
    .scaled(s);
    let percent = osd
        .meter
        .filter(|_| glass)
        .map(|m| format!("{}%", (m.fraction.clamp(0.0, 1.0) * 100.0).round()));
    let percent_w = percent
        .as_deref()
        .map_or(0, |t| text.measure(t, true, px) + PERCENT_GAP * s);
    // A long title (a player's Identity, say) ends in an ellipsis rather
    // than being cut off mid-glyph at the padding.
    let title = text.ellipsize(&osd.title, title_r.width - percent_w, true, px);
    if glass {
        let baseline = text.baseline(title_r, true, px).unwrap_or(title_r.bottom());
        text.draw_run_over(
            c,
            title_r.x,
            baseline,
            title_r,
            &title,
            true,
            panel::INK,
            px,
        );
        if let Some(t) = percent.as_deref() {
            let x = title_r.right() - text.measure(t, true, px);
            text.draw_run_over(c, x, baseline, title_r, t, true, panel::INK, px);
        }
    } else {
        text.draw(c, title_r, &title, true, cs.button_text, Align::Left, px);
    }
    if let Some(m) = osd.meter {
        draw_progress(c, p, lay.bar, m.fraction, m.enabled);
        if glass && !m.enabled {
            // Glass's greyed bar fills as an enabled one does; a muted
            // sink's bar goes grey all the same.
            grey_out(c, lay.bar.scaled(s));
        }
    }
}

fn grey_out(c: &mut Canvas, r: Rect) {
    let r = r.intersect(&c.bounds());
    for y in r.y..r.bottom() {
        for x in r.x..r.right() {
            let px = c.get(x, y);
            let [red, g, b] = [16, 8, 0].map(|sh| (px >> sh) & 0xFF);
            let l = (red * 77 + g * 150 + b * 29) >> 8;
            c.put_raw(x, y, px & 0xFF00_0000 | l << 16 | l << 8 | l);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::FontConfig;

    #[test]
    fn the_panel_is_a_raised_face_box_with_the_bar_under_the_title() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let text = TextRenderer::load(&FontConfig::default());
        let osd = Osd {
            title: "Volume".into(),
            meter: Some(Meter {
                fraction: 1.0,
                enabled: true,
            }),
        };
        let c = draw(&theme, &text, &osd, None);
        assert_eq!(c.get(0, 0), cs.button_face.argb());
        assert_eq!(c.get(1, 1), cs.button_hilight.argb());
        assert_eq!(c.get(W - 1, H - 1), cs.button_dk_shadow.argb());
        let bar = layout().bar;
        assert_eq!(c.get(bar.x, bar.y), cs.button_shadow.argb());
        assert_eq!(c.get(bar.x + 4, bar.y + 8), cs.hilight.argb());
        let muted = Osd {
            title: "Muted".into(),
            meter: Some(Meter {
                fraction: 1.0,
                enabled: false,
            }),
        };
        let c = draw(&theme, &text, &muted, None);
        assert_eq!(c.get(bar.x + 4, bar.y + 8), cs.gray_text.argb());
        // A title far too wide is ellipsized: the panel keeps its size and the
        // raised edge at the right survives.
        let long = Osd {
            title: "A media player with an extremely long identity string".repeat(4),
            meter: None,
        };
        let c = draw(&theme, &text, &long, None);
        assert_eq!((c.width, c.height), (W as u32, H as u32));
        assert_eq!(c.get(W - 1, H - 1), cs.button_dk_shadow.argb());
    }

    #[test]
    fn under_glass_the_panel_is_glass_with_a_white_title_and_its_level() {
        let theme = crate::interfaces::theme_for(crate::interface::Interface::Glass);
        let text = TextRenderer::load(&FontConfig::default());
        let draw_one = |title: &str, meter: Option<Meter>| {
            let osd = Osd {
                title: title.into(),
                meter,
            };
            draw(&theme, &text, &osd, None)
        };
        let m = panel::margin(&theme);
        let lay = layout();
        let meter = |enabled| {
            Some(Meter {
                fraction: 0.5,
                enabled,
            })
        };
        let c = draw_one("Volume", meter(true));
        assert_eq!(
            (c.width as i32, c.height as i32),
            (W + m.left + m.right, H + m.top + m.bottom),
            "the panel and its shadow"
        );
        let at = |c: &Canvas, x: i32, y: i32| c.get(m.left + x, m.top + y);
        let a = at(&c, PAD, H - 6) >> 24;
        assert!(a > 0 && a < 0xFF, "glass under the icon: alpha {a:#x}");
        // White ink: the title's at the line's left, the percentage's at
        // its right, and with no meter nothing at the right.
        let white_in = |c: &Canvas, x0: i32, x1: i32| {
            let t = lay.title;
            (t.y..t.bottom()).any(|y| (x0..x1).any(|x| at(c, x, y) == 0xFFFF_FFFF))
        };
        let (t, half) = (lay.title, lay.title.width / 2);
        assert!(white_in(&c, t.x, t.x + half), "the title is white");
        assert!(white_in(&c, t.x + half, t.right()), "and so is 50%");
        assert!(
            !white_in(&draw_one("Volume", None), t.x + half, t.right()),
            "no meter, no percentage"
        );
        let (bx, by) = (
            lay.bar.x + lay.bar.width / 4,
            lay.bar.y + lay.bar.height / 2,
        );
        let rgb = |px: u32| [16, 8, 0].map(|s| (px >> s) & 0xFF);
        let [r, g, b] = rgb(at(&c, bx, by));
        assert!(g > r + 80 && g > b + 80, "green: {:?}", [r, g, b]);
        let [r, g, b] = rgb(at(&draw_one("Muted", meter(false)), bx, by));
        assert!(r == g && g == b, "grey: {:?}", [r, g, b]);
    }

    #[test]
    #[ignore]
    fn render_osd_preview() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let osd = Osd {
            title: "Volume".into(),
            meter: Some(Meter {
                fraction: 0.45,
                enabled: true,
            }),
        };
        crate::raster::write_preview(&draw(&theme, &text, &osd, None), "osd-preview.png");
    }
}
