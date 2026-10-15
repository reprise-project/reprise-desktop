//! The media keys' on-screen panel: either one icon, bold title and progress bar, or a title over one line per display; under glass the box is glass and the level is also written out.

use crate::art::icons::Icon;
use crate::look::widgets::draw_progress;
use crate::look::{Paint, panel};
use crate::raster::{Canvas, EdgeStyle, Rect};
use crate::text::{Align, TextRenderer};
use crate::theme::{Color, Theme};

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

/// A display's level: not read yet, its percentage, or not adjustable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Unread,
    At(u8),
    Fixed,
}

pub struct Line {
    pub icon: &'static Icon,
    pub label: String,
    pub level: Level,
}

/// The panel with a title over one [`Line`] per display.
pub struct Lines {
    pub title: String,
    pub lines: Vec<Line>,
}

pub const LINES_W: i32 = 360;
pub const NOT_ADJUSTABLE: &str = "Not adjustable";
const TITLE_H: i32 = 13;
const TITLE_GAP: i32 = 6;
const LINE_H: i32 = 24;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineLayout {
    pub icon: Rect,
    pub label: Rect,
    pub bar: Rect,
    pub percent: Rect,
}

pub struct LinesLayout {
    pub title: Rect,
    pub lines: Vec<LineLayout>,
}

/// The panel's size at scale 1 for `n` displays.
pub fn lines_size(n: usize) -> (i32, i32) {
    (LINES_W, 2 * PAD + TITLE_H + TITLE_GAP + LINE_H * n as i32)
}

pub fn lines_layout(n: usize) -> LinesLayout {
    let y_first = PAD + TITLE_H + TITLE_GAP;
    LinesLayout {
        title: Rect::new(PAD, PAD, LINES_W - 2 * PAD, TITLE_H),
        lines: (0..n as i32)
            .map(|i| {
                let y = y_first + LINE_H * i;
                LineLayout {
                    icon: Rect::new(PAD, y + 4, 16, 16),
                    label: Rect::new(36, y + 5, 120, 13),
                    bar: Rect::new(164, y + 4, 142, 16),
                    percent: Rect::new(314, y + 5, 34, 13),
                }
            })
            .collect(),
    }
}

/// The panel for `lines`; the canvas reaches [`panel::margin`] past the box under glass, as [`draw`].
pub fn draw_lines(theme: &Theme, text: &TextRenderer, lines: &Lines) -> Canvas {
    let p = &theme.paint();
    let (cs, s) = (p.cs, p.s);
    let (w, h) = lines_size(lines.lines.len());
    if let Some((style, spec)) = panel::glass(theme) {
        return panel::draw(style, spec, w, h, s, |c| {
            lines_body(c, p, text, theme.fonts.size_px, lines, true)
        });
    }
    let mut c = Canvas::new((w * s) as u32, (h * s) as u32);
    c.fill_rect(c.bounds(), cs.button_face);
    c.edge_t(c.bounds(), EdgeStyle::WindowRaised, cs, s);
    lines_body(&mut c, p, text, theme.fonts.size_px, lines, false);
    c
}

#[expect(clippy::too_many_arguments)]
fn put(
    c: &mut Canvas,
    text: &TextRenderer,
    r: Rect,
    t: &str,
    bold: bool,
    ink: Color,
    align: Align,
    px: u32,
    glass: bool,
) {
    if !glass {
        return text.draw(c, r, t, bold, ink, align, px);
    }
    let baseline = text.baseline(r, bold, px).unwrap_or(r.bottom());
    let x = match align {
        Align::Right => r.right() - text.measure(t, bold, px),
        _ => r.x,
    };
    text.draw_run_over(c, x, baseline, r, t, bold, ink, px);
}

fn lines_body(c: &mut Canvas, p: &Paint, text: &TextRenderer, px: u32, lines: &Lines, glass: bool) {
    let (cs, s) = (p.cs, p.s);
    let lay = lines_layout(lines.lines.len());
    let (ink, dim) = if glass {
        (panel::INK, panel::DIM_INK)
    } else {
        (cs.button_text, cs.gray_text)
    };
    let title_r = lay.title.scaled(s);
    let title = text.ellipsize(&lines.title, title_r.width, true, px);
    put(c, text, title_r, &title, true, ink, Align::Left, px, glass);
    for (line, l) in lines.lines.iter().zip(&lay.lines) {
        line.icon.paint(c, l.icon.x * s, l.icon.y * s, s);
        let label_r = l.label.scaled(s);
        let label = text.ellipsize(&line.label, label_r.width, false, px);
        let label_ink = if line.level == Level::Fixed { dim } else { ink };
        put(
            c,
            text,
            label_r,
            &label,
            false,
            label_ink,
            Align::Left,
            px,
            glass,
        );
        match line.level {
            Level::Unread => {}
            Level::At(v) => {
                draw_progress(c, p, l.bar, f64::from(v) / 100.0, true);
                let t = format!("{v}%");
                put(
                    c,
                    text,
                    l.percent.scaled(s),
                    &t,
                    false,
                    ink,
                    Align::Right,
                    px,
                    glass,
                );
            }
            Level::Fixed => {
                put(
                    c,
                    text,
                    l.bar.scaled(s),
                    NOT_ADJUSTABLE,
                    false,
                    dim,
                    Align::Left,
                    px,
                    glass,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::art::icons;
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

    fn three() -> Lines {
        Lines {
            title: "Brightness".into(),
            lines: vec![
                Line {
                    icon: &icons::LAPTOP,
                    label: "Built-in display".into(),
                    level: Level::At(100),
                },
                Line {
                    icon: &icons::MONITOR,
                    label: "DELL U2720Q".into(),
                    level: Level::Fixed,
                },
                Line {
                    icon: &icons::MONITOR,
                    label: "LG 27GL850".into(),
                    level: Level::Unread,
                },
            ],
        }
    }

    fn any_px(c: &Canvas, r: Rect, f: impl Fn(u32) -> bool) -> bool {
        (r.y..r.bottom()).any(|y| (r.x..r.right()).any(|x| f(c.get(x, y))))
    }

    #[test]
    fn lines_grow_by_24_px_a_display() {
        assert_eq!(lines_size(1), (360, 67));
        assert_eq!(lines_size(3), (360, 115));
        let l = lines_layout(2);
        assert_eq!(l.title, Rect::new(12, 12, 336, 13));
        assert_eq!(
            l.lines[1],
            LineLayout {
                icon: Rect::new(12, 59, 16, 16),
                label: Rect::new(36, 60, 120, 13),
                bar: Rect::new(164, 59, 142, 16),
                percent: Rect::new(314, 60, 34, 13),
            }
        );
    }

    #[test]
    fn classic_lines_draw_a_bar_per_adjustable_display_and_grey_the_rest() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let text = TextRenderer::load(&FontConfig::default());
        let c = draw_lines(&theme, &text, &three());
        let (w, h) = lines_size(3);
        assert_eq!(c.get(0, 0), cs.button_face.argb());
        assert_eq!(c.get(1, 1), cs.button_hilight.argb());
        assert_eq!(c.get(w - 1, h - 1), cs.button_dk_shadow.argb());
        let lay = lines_layout(3).lines;
        assert_eq!(c.get(164, 35), cs.button_shadow.argb());
        assert_eq!(c.get(168, 43), cs.hilight.argb());
        assert!(any_px(&c, lay[0].percent, |p| p == cs.button_text.argb()));
        let hilight = cs.hilight.argb();
        assert!(!any_px(&c, lay[1].bar, |p| p == hilight), "no bar");
        assert!(
            any_px(&c, lay[1].bar, |p| p == cs.gray_text.argb()),
            "Not adjustable"
        );
        assert!(any_px(&c, lay[1].label, |p| p == cs.gray_text.argb()));
        let face = cs.button_face.argb();
        assert!(!any_px(&c, lay[2].bar, |p| p != face));
        assert!(!any_px(&c, lay[2].percent, |p| p != face));
    }

    #[test]
    fn glass_lines_write_each_level_in_white() {
        let theme = crate::interfaces::theme_for(crate::interface::Interface::Glass);
        let text = TextRenderer::load(&FontConfig::default());
        let c = draw_lines(&theme, &text, &three());
        let m = panel::margin(&theme);
        let (w, h) = lines_size(3);
        assert_eq!(
            (c.width as i32, c.height as i32),
            (w + m.left + m.right, h + m.top + m.bottom)
        );
        let moved = |r: Rect| Rect::new(r.x + m.left, r.y + m.top, r.width, r.height);
        let lay = lines_layout(3).lines;
        let white = |p: u32| p == 0xFFFF_FFFF;
        assert!(any_px(&c, moved(lay[0].percent), white));
        assert!(any_px(&c, moved(lay[1].label), |p| p == panel::DIM_INK.argb()));
        assert!(!any_px(&c, moved(lay[1].bar), white));
        assert!(!any_px(&c, moved(lay[2].percent), white));
    }

    #[test]
    fn a_long_label_ends_before_the_bar() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let text = TextRenderer::load(&FontConfig::default());
        let label = "A monitor with an extremely long model name".repeat(3);
        let lines = Lines {
            title: "Brightness".into(),
            lines: vec![Line {
                icon: &icons::MONITOR,
                label: label.clone(),
                level: Level::At(50),
            }],
        };
        let c = draw_lines(&theme, &text, &lines);
        let l = lines_layout(1).lines[0];
        let ink = cs.button_text.argb();
        let px = theme.fonts.size_px;
        assert!(
            text.ellipsize(&label, l.label.width, false, px)
                .ends_with("...")
        );
        let rows_with_ink = |r: Rect| {
            (r.y..r.bottom())
                .filter(|&y| (r.x..r.right()).any(|x| c.get(x, y) == ink))
                .count()
        };
        let last = (l.label.x..l.label.right())
            .rev()
            .find(|&x| (l.label.y..l.label.bottom()).any(|y| c.get(x, y) == ink))
            .expect("label ink");
        let dots = Rect::new(last - 8, l.label.y, 9, l.label.height);
        let rows = rows_with_ink(dots);
        assert!(
            (1..=2).contains(&rows),
            "the label ends in dots, not a cut glyph: {rows} rows"
        );
        let gap = Rect::new(
            l.label.right(),
            l.label.y,
            l.bar.x - l.label.right(),
            l.label.height,
        );
        assert!(!any_px(&c, gap, |p| p == ink));
        assert_eq!(c.get(l.bar.x, l.bar.y), cs.button_shadow.argb());
    }
}
