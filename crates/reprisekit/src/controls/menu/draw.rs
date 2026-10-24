use super::MenuItem;
use super::layout::{
    ARROW_RIGHT_INSET, ARROW_SPACE, BANNER_PAD, BULLET_X, CHECK_DY, CHECK_X, MenuLayout,
};
use crate::raster::{Canvas, EdgeStyle, Rect};
use crate::text::{Align, TextRenderer};
use crate::theme::Theme;
use std::borrow::Cow;

const CHECK_GLYPH: [&str; 7] = [
    "......X", ".....XX", "X...XXX", "XX.XXX.", "XXXXX..", ".XXX...", "..X....",
];

fn draw_check(canvas: &mut Canvas, x: i32, y: i32, s: i32, color: crate::theme::Color) {
    canvas.paint_map(x, y, s, &CHECK_GLYPH, |ch| (ch == 'X').then_some(color));
}

pub const BULLET_D: i32 = 6;
const BULLET_RUNS: [(i32, i32); BULLET_D as usize] =
    [(1, 4), (0, 6), (0, 6), (0, 6), (0, 6), (1, 4)];

fn draw_bullet(canvas: &mut Canvas, x: i32, y: i32, s: i32, color: crate::theme::Color) {
    for (row, (off, len)) in BULLET_RUNS.iter().enumerate() {
        canvas.fill_rect(
            Rect::new(x + off * s, y + row as i32 * s, len * s, s),
            color,
        );
    }
}

fn draw_arrow(canvas: &mut Canvas, x: i32, y: i32, s: i32, color: crate::theme::Color) {
    for col in 0..4 {
        let h = 7 - 2 * col;
        canvas.fill_rect(Rect::new(x + col * s, y + col * s, s, h * s), color);
    }
}

fn band_stops(cs: &crate::theme::ColorScheme) -> [(f32, crate::theme::Color); 7] {
    [
        (0.00, cs.menu_band),
        (0.04, cs.menu_band),
        (0.22, cs.gradient_menu_band),
        (0.45, cs.gradient_menu_band),
        (0.75, cs.menu_band),
        (0.82, cs.menu_band_top),
        (1.00, cs.menu_band_top),
    ]
}

fn band_color(stops: &[(f32, crate::theme::Color)], t: f32) -> crate::theme::Color {
    let mut prev = stops[0];
    for &next in &stops[1..] {
        if t <= next.0 {
            let span = (next.0 - prev.0).max(f32::EPSILON);
            let k = ((t - prev.0) / span).clamp(0.0, 1.0);
            let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * k).round() as u8;
            return crate::theme::Color::rgb(
                mix(prev.1.r, next.1.r),
                mix(prev.1.g, next.1.g),
                mix(prev.1.b, next.1.b),
            );
        }
        prev = next;
    }
    prev.1
}

fn draw_band(canvas: &mut Canvas, theme: &Theme, text: &TextRenderer, band: Rect, banner: &str) {
    let cs = &theme.colors;
    let s = theme.scale.max(1);
    // Drawn sideways (text left to right, gradient bottom colour at the
    // left) and then turned so the left edge becomes the bottom.
    let mut strip = Canvas::new(band.height as u32, band.width as u32);
    let stops = band_stops(cs);
    let denom = (band.height - 1).max(1) as f32;
    for x in 0..band.height {
        strip.vline(x, 0, band.width, band_color(&stops, x as f32 / denom));
    }
    let size = theme.fonts.size_px + 3 * (s as u32);
    let (bold, regular) = match banner.rsplit_once(' ') {
        Some((head, tail)) => (head.to_string(), format!(" {tail}")),
        None => (banner.to_string(), String::new()),
    };
    let mut pen = BANNER_PAD * s;
    for (part, is_bold) in [(bold, true), (regular, false)] {
        if part.is_empty() {
            continue;
        }
        text.draw(
            &mut strip,
            Rect::new(pen, 0, (band.height - pen).max(0), band.width),
            &part,
            is_bold,
            cs.menu_band_text,
            Align::Left,
            size,
        );
        pen += text.measure(&part, is_bold, size);
    }
    canvas.blit_canvas(band.x, band.y, &strip.rotate_ccw());
}

/// The top of an `h` px (logical) glyph centred down row `r` at scale
/// `s`, rounded in logical pixels so a scale-2 row is its scale-1 row
/// doubled.
pub(super) fn centred(r: &Rect, h: i32, s: i32) -> i32 {
    r.y + (r.height / s - h) / 2 * s
}

#[allow(clippy::too_many_arguments)]
pub fn draw_menu(
    canvas: &mut Canvas,
    theme: &Theme,
    text: &TextRenderer,
    layout: &MenuLayout,
    items: &[MenuItem],
    hovered: Option<usize>,
    banner: &str,
    underline: bool,
) {
    draw_menu_at(
        canvas,
        (0, 0),
        theme,
        text,
        layout,
        items,
        hovered,
        banner,
        underline,
    );
}

#[allow(clippy::too_many_arguments)]
pub fn draw_menu_at(
    canvas: &mut Canvas,
    at: (i32, i32),
    theme: &Theme,
    text: &TextRenderer,
    layout: &MenuLayout,
    items: &[MenuItem],
    hovered: Option<usize>,
    banner: &str,
    underline: bool,
) {
    debug_assert_eq!(
        items.len(),
        layout.rows.len(),
        "menu layout does not match items"
    );
    if layout.gutter > 0
        && let Some(st) = theme.look.style()
        && let Some(m) = &st.menu
    {
        return draw_gutter_menu(
            canvas, at, theme, st, m, text, layout, items, hovered, underline,
        );
    }
    let cs = &theme.colors;
    let s = theme.scale.max(1);
    let size = theme.fonts.size_px;
    let (ox, oy) = at;
    let flat = theme.look.style().is_some_and(|st| st.flat_menus);
    let all = Rect::new(ox, oy, layout.width, layout.height);
    canvas.fill_rect(all, cs.menu);
    if flat {
        canvas.ring_t(all, cs.button_shadow, cs.button_shadow, s);
    } else {
        canvas.edge_t(all, EdgeStyle::WindowRaised, cs, s);
    }
    if layout.band > 0 {
        // A flat menu's band reaches out to its 1 px border; Classic's
        // starts where the rows' inset does.
        let (x0, y0, y1) = if flat {
            (s, s, s)
        } else {
            (layout.inset.left, layout.inset.top, layout.inset.bottom)
        };
        let band = Rect::new(
            ox + x0,
            oy + y0,
            layout.band + layout.inset.left - x0,
            layout.height - y0 - y1,
        );
        draw_band(canvas, theme, text, band, banner);
    }
    for c in 1..layout.columns {
        let x = ox + layout.band + c as i32 * layout.column_width - s;
        if flat {
            let line = Rect::new(x, oy + s, s, layout.height - 2 * s);
            canvas.fill_rect(line, cs.button_shadow);
        } else {
            canvas.etched_vline_t(x, oy + s, oy + layout.height - s, cs, s);
        }
    }
    for (i, (item, r)) in items.iter().zip(&layout.rows).enumerate() {
        let r = &Rect::new(r.x + ox, r.y + oy, r.width, r.height);
        match item {
            MenuItem::Separator => {
                let y = r.y + (r.height / s / 2 - 1) * s;
                let dx = layout.separator_x;
                if flat {
                    let line = Rect::new(r.x + dx, y, r.width - 2 * dx, s);
                    canvas.fill_rect(line, cs.button_shadow);
                } else {
                    canvas.etched_hline_t(r.x + dx, r.right() - dx, y, cs, s);
                }
            }
            MenuItem::Entry {
                label,
                icon,
                enabled,
                checked,
                radio,
                submenu,
                mnemonic,
                shortcut,
                bold,
                glyph,
            } => {
                // A disabled row highlights too, its text staying grey.
                // A flat menu's lit row is `menu_hilight` in a 1 px
                // `hilight` frame.
                let hovered_here = hovered == Some(i);
                let lit = if flat { cs.menu_hilight } else { cs.hilight };
                let (bg, fg) = if hovered_here && *enabled {
                    (lit, cs.hilight_text)
                } else if hovered_here {
                    (lit, cs.gray_text)
                } else if *enabled {
                    (cs.menu, cs.menu_text)
                } else {
                    (cs.menu, cs.gray_text)
                };
                canvas.fill_rect(*r, bg);
                if flat && hovered_here {
                    canvas.ring_t(*r, cs.hilight, cs.hilight, s);
                }
                // Classic greys a disabled label by etching it; a flat
                // menu draws it grey alone.
                let etched = !*enabled && !hovered_here && !flat;
                let glyph_y = centred(r, 7, s);
                match icon {
                    Some(ic) => {
                        let iy = r.y + (r.height - ic.height as i32) / 2;
                        canvas.blit_canvas(r.x + layout.icon_x, iy, ic);
                    }
                    None if *checked && *radio => {
                        draw_bullet(canvas, r.x + BULLET_X * s, centred(r, BULLET_D, s), s, fg)
                    }
                    None if *checked => {
                        draw_check(canvas, r.x + CHECK_X * s, glyph_y + CHECK_DY * s, s, fg)
                    }
                    None => {
                        if let Some(g) = glyph {
                            let gy = centred(r, 14, s);
                            crate::art::glyph::draw_glyph_strokes(
                                canvas,
                                r.x + layout.icon_x,
                                gy,
                                s,
                                *g,
                                fg,
                                bg,
                            );
                        }
                    }
                }
                if *submenu {
                    let ink = if hovered_here && *enabled && layout.lit_arrow_menu_text {
                        cs.menu_text
                    } else {
                        fg
                    };
                    draw_arrow(canvas, r.right() - ARROW_RIGHT_INSET * s, glyph_y, s, ink);
                }
                let text_rect = Rect::new(
                    r.x + layout.text_x,
                    r.y,
                    (r.width - layout.text_x - ARROW_SPACE * s).max(0),
                    r.height,
                );
                let shown = text.ellipsize(label, text_rect.width, *bold, size);
                if let Some(sc) = shortcut {
                    let sc_rect = Rect::new(text_rect.x, r.y, text_rect.width, r.height);
                    if etched {
                        let lit =
                            Rect::new(sc_rect.x + s, sc_rect.y + s, sc_rect.width, sc_rect.height);
                        text.draw(
                            canvas,
                            lit,
                            sc,
                            false,
                            cs.button_hilight,
                            Align::Right,
                            size,
                        );
                    }
                    text.draw(canvas, sc_rect, sc, false, fg, Align::Right, size);
                }
                if etched {
                    let lit = Rect::new(
                        text_rect.x + s,
                        text_rect.y + s,
                        text_rect.width,
                        text_rect.height,
                    );
                    text.draw(
                        canvas,
                        lit,
                        &shown,
                        *bold,
                        cs.button_hilight,
                        Align::Left,
                        size,
                    );
                }
                let mnemonic = mnemonic.filter(|_| underline);
                draw_label(
                    canvas,
                    text,
                    text_rect,
                    &shown,
                    matches!(shown, Cow::Owned(_)),
                    *bold,
                    mnemonic,
                    fg,
                    size,
                    s,
                );
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_label(
    canvas: &mut Canvas,
    text: &TextRenderer,
    r: Rect,
    shown: &str,
    ellipsized: bool,
    bold: bool,
    mnemonic: Option<usize>,
    fg: crate::theme::Color,
    size: u32,
    s: i32,
) {
    text.draw(canvas, r, shown, bold, fg, Align::Left, size);
    // An ellipsized label keeps a prefix of `label` before its three
    // dots; an access key past that prefix is not drawn.
    let visible = if ellipsized {
        shown.chars().count() - 3
    } else {
        usize::MAX
    };
    if let Some(m) = mnemonic.filter(|&m| m < visible) {
        // The access-key underline sits one pixel under the baseline, `s` thick.
        if let Some(baseline) = text.baseline(r, bold, size) {
            let (dx, w) = text.span(shown, m, bold, size);
            let ul = Rect::new(r.x + dx, baseline + s, w, s);
            canvas.fill_rect(ul.intersect(&r), fg);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_gutter_menu(
    canvas: &mut Canvas,
    (ox, oy): (i32, i32),
    theme: &Theme,
    st: &crate::style::Style,
    m: &crate::style::MenuLook,
    text: &TextRenderer,
    layout: &MenuLayout,
    items: &[MenuItem],
    hovered: Option<usize>,
    underline: bool,
) {
    use crate::look::styled::menu as look;
    let cs = &theme.colors;
    let s = theme.scale.max(1);
    let size = theme.fonts.size_px;
    let all = Rect::new(ox, oy, layout.width, layout.height);
    look::draw_frame(m, canvas, all, cs.menu, s);
    let ins = layout.inset;
    let rows_h = layout.height - ins.top - ins.bottom;
    for c in 0..layout.columns as i32 {
        let x = ox + layout.band + c * layout.column_width + ins.left;
        canvas.fill_rect(
            Rect::new(x, oy + ins.top, layout.gutter, rows_h),
            m.gutter_fill,
        );
        let dx = x + layout.gutter;
        canvas.fill_rect(Rect::new(dx, oy + ins.top, s, rows_h), m.divider.0);
        canvas.fill_rect(Rect::new(dx + s, oy + ins.top, s, rows_h), m.divider.1);
        if c > 0 {
            let x = x - ins.left;
            let line = |x: i32| Rect::new(x, oy + ins.top, s, rows_h);
            canvas.fill_rect(line(x - s), m.rule.0);
            canvas.fill_rect(line(x), m.rule.1);
        }
    }
    let past_divider = layout.gutter + 2 * s;
    for (i, (item, r)) in items.iter().zip(&layout.rows).enumerate() {
        let r = Rect::new(r.x + ox, r.y + oy, r.width, r.height);
        let MenuItem::Entry {
            label,
            icon,
            enabled,
            checked,
            radio,
            submenu,
            mnemonic,
            shortcut,
            bold,
            glyph,
        } = item
        else {
            let line =
                |dy: i32| Rect::new(r.x + past_divider, r.y + dy * s, r.width - past_divider, s);
            let mid = m.separator / 2;
            canvas.fill_rect(line(mid), m.rule.0);
            canvas.fill_rect(line(mid + 1), m.rule.1);
            continue;
        };
        let lit = hovered == Some(i);
        if lit {
            look::draw_lit_row(st, canvas, r, !*enabled, s);
        }
        let fg = if *enabled { cs.menu_text } else { m.greyed };
        let square = Rect::new(r.x, r.y, r.height, r.height);
        match icon {
            Some(ic) => {
                let iy = r.y + (r.height - ic.height as i32) / 2;
                canvas.blit_canvas(r.x + layout.icon_x, iy, ic);
            }
            None if *checked => {
                look::draw_check(st, m, canvas, square, *radio, lit, s);
            }
            None => {
                if let Some(g) = glyph {
                    let gy = centred(&r, 14, s);
                    let bg = canvas.get(r.x + layout.icon_x, gy);
                    crate::art::glyph::draw_glyph_strokes(
                        canvas,
                        r.x + layout.icon_x,
                        gy,
                        s,
                        *g,
                        fg,
                        crate::theme::Color::rgb((bg >> 16) as u8, (bg >> 8) as u8, bg as u8),
                    );
                }
            }
        }
        if *submenu {
            let ink = if *enabled { m.arrow } else { m.greyed };
            draw_arrow(
                canvas,
                r.right() - m.arrow_inset * s,
                centred(&r, 7, s),
                s,
                ink,
            );
        }
        let text_rect = Rect::new(
            r.x + layout.text_x,
            r.y,
            (r.width - layout.text_x - m.shortcut_end * s).max(0),
            r.height,
        );
        let shown = text.ellipsize(label, text_rect.width, *bold, size);
        if let Some(sc) = shortcut {
            text.draw(canvas, text_rect, sc, false, fg, Align::Right, size);
        }
        let mnemonic = mnemonic.filter(|_| underline);
        draw_label(
            canvas,
            text,
            text_rect,
            &shown,
            matches!(shown, Cow::Owned(_)),
            *bold,
            mnemonic,
            fg,
            size,
            s,
        );
    }
}
