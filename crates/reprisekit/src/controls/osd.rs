//! The on-screen panel of the media keys and the lock keys: either one icon, bold title and progress bar, or a title over one line per display; under glass the box is glass and the level is also written out.

use crate::art::icons::Icon;
pub use crate::art::lockkey::LockKey;
use crate::look::widgets::draw_progress;
use crate::look::{Paint, panel};
use crate::raster::{Canvas, EdgeStyle, Rect};
use crate::text::{Align, TextRenderer};
use crate::theme::{Color, TextUnits, Theme};

/// The panel's size at 100% text.
pub const W: i32 = 260;
pub const H: i32 = 72;
const PAD: i32 = 12;
const TEXT_X: i32 = PAD + ICON + PAD;
const TITLE_Y: i32 = 16;
const TITLE_H: i32 = 13;
const BAR_GAP: i32 = 7;
const BAR_BELOW: i32 = 18;
const ICON: i32 = 32;
const BAR_H: i32 = 18;
const PERCENT_GAP: i32 = 8;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Meter {
    /// The level, 0 to `top`.
    pub fraction: f64,
    pub enabled: bool,
    /// What a full bar stands for: 1, or more for a volume that runs past 100 %, whose
    /// percentage is then written out in every look.
    pub top: f64,
}

impl Meter {
    pub fn new(fraction: f64, enabled: bool) -> Meter {
        Meter {
            fraction,
            enabled,
            top: 1.0,
        }
    }
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

/// The panel's size with its title row and text column grown by `u`: [`W`] x [`H`] at 100%.
pub fn size(u: TextUnits) -> (i32, i32) {
    let bar = layout(u).bar;
    (bar.right() + PAD, bar.bottom() + BAR_BELOW)
}

pub fn layout(u: TextUnits) -> Layout {
    let text_w = u.len(W - TEXT_X - PAD);
    let title = Rect::new(TEXT_X, TITLE_Y, text_w, u.len(TITLE_H));
    let bar = Rect::new(TEXT_X, title.bottom() + BAR_GAP, text_w, BAR_H);
    let h = bar.bottom() + BAR_BELOW;
    Layout {
        icon: Rect::new(PAD, (h - ICON) / 2, ICON, ICON),
        title,
        bar,
    }
}

/// The panel a lock key shows when it changes: "Caps Lock On", "Num Lock Off".
pub fn lock_key(key: LockKey, on: bool) -> Osd {
    let name = match key {
        LockKey::Caps => "Caps Lock",
        LockKey::Num => "Num Lock",
    };
    Osd {
        title: format!("{name} {}", if on { "On" } else { "Off" }),
        meter: None,
    }
}

/// [`draw`] for [`lock_key`] with its [`lockkey`](crate::art::lockkey) icon.
pub fn draw_lock_key(theme: &Theme, text: &TextRenderer, key: LockKey, on: bool) -> Canvas {
    let icon = crate::art::lockkey::draw(key, on, theme.paint().s);
    draw(theme, text, &lock_key(key, on), Some(&icon))
}

/// The panel for `osd` with `icon` (32 px at the theme's scale). Under a look
/// with glass it is a piece of glass with its shadow (the canvas reaches
/// [`panel::margin`] past the panel), the title white and the meter's level
/// written as a percentage at the title line's right.
pub fn draw(theme: &Theme, text: &TextRenderer, osd: &Osd, icon: Option<&Canvas>) -> Canvas {
    let p = &theme.paint();
    let (cs, s) = (p.cs, p.s);
    let px = theme.fonts.size_px;
    let (w, h) = size(theme.text);
    if let Some((style, spec)) = panel::glass(theme) {
        return panel::draw_inked(style, spec, w, h, s, |c| {
            body(c, p, text, px, theme.text, osd, icon, true)
        });
    }
    let mut c = Canvas::new((w * s) as u32, (h * s) as u32);
    c.fill_rect(c.bounds(), cs.button_face);
    c.edge_t(c.bounds(), EdgeStyle::WindowRaised, cs, s);
    body(&mut c, p, text, px, theme.text, osd, icon, false);
    c
}

/// The icon, the title and the meter, in panel coordinates; `glass` sets
/// the title in white and adds the percentage.
#[expect(clippy::too_many_arguments)]
fn body(
    c: &mut Canvas,
    p: &Paint,
    text: &TextRenderer,
    px: u32,
    u: TextUnits,
    osd: &Osd,
    icon: Option<&Canvas>,
    glass: bool,
) {
    let (cs, s) = (p.cs, p.s);
    let lay = layout(u);
    if let Some(ic) = icon {
        c.blit_over(lay.icon.x * s, lay.icon.y * s, ic);
    }
    let title_r = if osd.meter.is_some() {
        lay.title
    } else {
        let h = size(u).1;
        Rect::new(
            lay.title.x,
            (h - lay.title.height) / 2,
            lay.title.width,
            lay.title.height,
        )
    }
    .scaled(s);
    let percent = osd
        .meter
        .filter(|m| glass || m.top > 1.0)
        .map(|m| format!("{}%", (m.fraction.clamp(0.0, m.top) * 100.0).round()));
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
        if let Some(t) = percent.as_deref() {
            text.draw(c, title_r, t, true, cs.button_text, Align::Right, px);
        }
    }
    if let Some(m) = osd.meter {
        draw_progress(c, p, lay.bar, m.fraction / m.top.max(1.0), m.enabled);
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

pub const NOT_ADJUSTABLE: &str = "Not adjustable";
const TITLE_GAP: i32 = 6;
/// A display's line at 100% text: its 13 px label row with 5 px above and 6 below.
const LINE_H: i32 = 24;
const LABEL_X: i32 = 36;
const LABEL_W: i32 = 120;
const BAR_W: i32 = 142;
const LEVEL_GAP: i32 = 8;
const PERCENT_W: i32 = 34;
const LINE_ICON: i32 = 16;

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

/// The panel's size at scale 1 for `n` displays, its rows and text columns grown by `u`.
pub fn lines_size(n: usize, u: TextUnits) -> (i32, i32) {
    let title_h = u.len(TITLE_H);
    let line_h = LINE_H - TITLE_H + title_h;
    (
        LABEL_X + u.len(LABEL_W) + LEVEL_GAP + BAR_W + LEVEL_GAP + u.len(PERCENT_W) + PAD,
        2 * PAD + title_h + TITLE_GAP + line_h * n as i32,
    )
}

pub fn lines_layout(n: usize, u: TextUnits) -> LinesLayout {
    let (w, _) = lines_size(n, u);
    let title_h = u.len(TITLE_H);
    let line_h = LINE_H - TITLE_H + title_h;
    let y_first = PAD + title_h + TITLE_GAP;
    let label_w = u.len(LABEL_W);
    let bar_x = LABEL_X + label_w + LEVEL_GAP;
    let mid = |h: i32| (line_h - h) / 2;
    LinesLayout {
        title: Rect::new(PAD, PAD, w - 2 * PAD, title_h),
        lines: (0..n as i32)
            .map(|i| {
                let y = y_first + line_h * i;
                let label = Rect::new(LABEL_X, y + mid(title_h), label_w, title_h);
                let bar = Rect::new(bar_x, y + mid(LINE_ICON), BAR_W, LINE_ICON);
                LineLayout {
                    icon: Rect::new(PAD, y + mid(LINE_ICON), LINE_ICON, LINE_ICON),
                    label,
                    bar,
                    percent: Rect::new(bar.right() + LEVEL_GAP, label.y, u.len(PERCENT_W), title_h),
                }
            })
            .collect(),
    }
}

/// The panel for `lines`; the canvas reaches [`panel::margin`] past the box under glass, as [`draw`].
pub fn draw_lines(theme: &Theme, text: &TextRenderer, lines: &Lines) -> Canvas {
    let p = &theme.paint();
    let (cs, s) = (p.cs, p.s);
    let (w, h) = lines_size(lines.lines.len(), theme.text);
    if let Some((style, spec)) = panel::glass(theme) {
        return panel::draw_inked(style, spec, w, h, s, |c| {
            lines_body(c, p, text, theme.fonts.size_px, theme.text, lines, true)
        });
    }
    let mut c = Canvas::new((w * s) as u32, (h * s) as u32);
    c.fill_rect(c.bounds(), cs.button_face);
    c.edge_t(c.bounds(), EdgeStyle::WindowRaised, cs, s);
    lines_body(
        &mut c,
        p,
        text,
        theme.fonts.size_px,
        theme.text,
        lines,
        false,
    );
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

fn lines_body(
    c: &mut Canvas,
    p: &Paint,
    text: &TextRenderer,
    px: u32,
    u: TextUnits,
    lines: &Lines,
    glass: bool,
) {
    let (cs, s) = (p.cs, p.s);
    let lay = lines_layout(lines.lines.len(), u);
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
                let r = Rect::new(
                    l.bar.x,
                    l.label.y,
                    l.percent.right() - l.bar.x,
                    l.label.height,
                );
                put(
                    c,
                    text,
                    r.scaled(s),
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
            meter: Some(Meter::new(1.0, true)),
        };
        let c = draw(&theme, &text, &osd, None);
        assert_eq!(c.get(0, 0), cs.button_face.argb());
        assert_eq!(c.get(1, 1), cs.button_hilight.argb());
        assert_eq!(c.get(W - 1, H - 1), cs.button_dk_shadow.argb());
        let bar = layout(TextUnits::ONE).bar;
        assert_eq!(c.get(bar.x, bar.y), cs.button_shadow.argb());
        assert_eq!(c.get(bar.x + 4, bar.y + 8), cs.hilight.argb());
        let muted = Osd {
            title: "Muted".into(),
            meter: Some(Meter::new(1.0, false)),
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
    fn a_boosted_volume_writes_its_percent_and_fills_by_150() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let text = TextRenderer::load(&FontConfig::default());
        let osd = |top| Osd {
            title: "Volume".into(),
            meter: Some(Meter {
                top,
                ..Meter::new(0.75, true)
            }),
        };
        let lay = layout(TextUnits::ONE);
        let inked = |c: &Canvas| {
            let t = lay.title;
            (t.right() - 30..t.right())
                .flat_map(|x| (t.y..t.bottom()).map(move |y| (x, y)))
                .any(|(x, y)| c.get(x, y) == cs.button_text.argb())
        };
        let (plain, boosted) = (
            draw(&theme, &text, &osd(1.0), None),
            draw(&theme, &text, &osd(1.5), None),
        );
        assert!(!inked(&plain), "a plain Classic meter writes no percent");
        assert!(inked(&boosted), "a boosted one writes 75%");
        let bar = lay.bar;
        let at = |c: &Canvas, f: f64| c.get(bar.x + (bar.width as f64 * f) as i32, bar.y + 8);
        assert_eq!(at(&plain, 0.7), cs.hilight.argb(), "0.75 of a 100 % bar");
        assert_ne!(at(&boosted, 0.6), cs.hilight.argb(), "half of a 150 % bar");
        assert_eq!(at(&boosted, 0.4), cs.hilight.argb());
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
        let lay = layout(TextUnits::ONE);
        let meter = |enabled| Some(Meter::new(0.5, enabled));
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
    fn a_lock_key_says_which_key_and_which_way() {
        assert_eq!(lock_key(LockKey::Caps, true).title, "Caps Lock On");
        assert_eq!(lock_key(LockKey::Caps, false).title, "Caps Lock Off");
        assert_eq!(lock_key(LockKey::Num, true).title, "Num Lock On");
        assert_eq!(lock_key(LockKey::Num, false).title, "Num Lock Off");
        assert_eq!(lock_key(LockKey::Num, true).meter, None);
    }

    fn fnv(c: &Canvas) -> u64 {
        let mut h: u64 = 0xcbf29ce484222325;
        for y in 0..c.height as i32 {
            for x in 0..c.width as i32 {
                for b in c.get(x, y).to_le_bytes() {
                    h = (h ^ u64::from(b)).wrapping_mul(0x100000001b3);
                }
            }
        }
        h
    }

    #[test]
    fn lock_key_panels_pin_in_every_interface_at_scale_1_and_2() {
        use crate::interface::Interface;
        crate::pins::fonts();
        let mut got = Vec::new();
        for look in [Interface::Classic, Interface::Blue, Interface::Glass] {
            for scale in [1, 2] {
                let theme = crate::interfaces::theme_for(look).scaled(scale);
                let text = TextRenderer::load(&theme.fonts);
                for (key, on) in [(LockKey::Caps, true), (LockKey::Num, false)] {
                    let c = draw_lock_key(&theme, &text, key, on);
                    got.push((format!("{look:?} {scale} {key:?} on={on}"), fnv(&c)));
                }
            }
        }
        crate::pins::check("controls/lock_key_panels", &got);
    }

    #[test]
    #[ignore]
    fn render_osd_preview() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let osd = Osd {
            title: "Volume".into(),
            meter: Some(Meter::new(0.45, true)),
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
        assert_eq!(lines_size(1, TextUnits::ONE), (360, 67));
        assert_eq!(lines_size(3, TextUnits::ONE), (360, 115));
        let l = lines_layout(2, TextUnits::ONE);
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
    fn at_200_percent_text_the_title_rows_and_text_columns_double_and_the_bars_keep_their_size() {
        let u = Theme::default().with_text_size(200).text;
        assert_eq!(size(TextUnits::ONE), (W, H));
        let lay = layout(u);
        assert_eq!(lay.title, Rect::new(56, 16, 384, 26));
        assert_eq!(lay.bar, Rect::new(56, 49, 384, BAR_H));
        assert_eq!(size(u), (452, 85));
        assert_eq!(lay.icon.y, (85 - ICON) / 2, "the icon stays centred");
        assert_eq!(
            lines_size(2, u),
            (36 + 240 + 8 + 142 + 8 + 68 + 12, 2 * 12 + 26 + 6 + 2 * 37)
        );
        let line = lines_layout(2, u).lines[1];
        assert_eq!(line.label, Rect::new(36, 12 + 26 + 6 + 37 + 5, 240, 26));
        assert_eq!(line.bar, Rect::new(284, 12 + 26 + 6 + 37 + 10, 142, 16));
        assert_eq!(line.percent, Rect::new(434, line.label.y, 68, 26));
    }

    #[test]
    fn panels_at_200_percent_text_pin_in_every_interface() {
        use crate::interface::Interface;
        crate::pins::fonts();
        let mut got = Vec::new();
        for look in [Interface::Classic, Interface::Blue, Interface::Glass] {
            for scale in [1, 2] {
                let theme = crate::interfaces::theme_for(look)
                    .with_text_size(200)
                    .scaled(scale);
                let text = TextRenderer::load(&theme.fonts);
                let volume = Osd {
                    title: "Volume".into(),
                    meter: Some(Meter::new(1.0, true)),
                };
                let key = format!("{look:?}@{scale}");
                got.push((
                    format!("{key} volume"),
                    fnv(&draw(&theme, &text, &volume, None)),
                ));
                let caps = draw_lock_key(&theme, &text, LockKey::Caps, true);
                got.push((format!("{key} caps"), fnv(&caps)));
                got.push((
                    format!("{key} lines"),
                    fnv(&draw_lines(&theme, &text, &three())),
                ));
            }
        }
        crate::pins::check("osd_text_200", &got);
    }

    #[test]
    fn classic_lines_draw_a_bar_per_adjustable_display_and_grey_the_rest() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let text = TextRenderer::load(&FontConfig::default());
        let c = draw_lines(&theme, &text, &three());
        let (w, h) = lines_size(3, TextUnits::ONE);
        assert_eq!(c.get(0, 0), cs.button_face.argb());
        assert_eq!(c.get(1, 1), cs.button_hilight.argb());
        assert_eq!(c.get(w - 1, h - 1), cs.button_dk_shadow.argb());
        let lay = lines_layout(3, TextUnits::ONE).lines;
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
        let (w, h) = lines_size(3, TextUnits::ONE);
        assert_eq!(
            (c.width as i32, c.height as i32),
            (w + m.left + m.right, h + m.top + m.bottom)
        );
        let moved = |r: Rect| Rect::new(r.x + m.left, r.y + m.top, r.width, r.height);
        let lay = lines_layout(3, TextUnits::ONE).lines;
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
        let l = lines_layout(1, TextUnits::ONE).lines[0];
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
