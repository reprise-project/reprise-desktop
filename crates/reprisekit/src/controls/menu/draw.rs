use super::MenuItem;
use super::layout::{
    ARROW_RIGHT_INSET, ARROW_SPACE, BANNER_OVER, BANNER_PAD, BULLET_X, CHECK_DY, CHECK_X,
    MenuLayout, ScrollArrow,
};
use crate::keycap;
use crate::raster::{Canvas, EdgeStyle, Rect};
use crate::text::{Align, TextRenderer};
use crate::theme::Theme;
use std::borrow::Cow;

/// The 7x7 check with its top-left at `x`, `y`.
fn draw_check(canvas: &mut Canvas, x: i32, y: i32, s: i32, color: crate::theme::Color) {
    let check_box = Rect::new(x - 3 * s, y - 3 * s, 13 * s, 13 * s);
    crate::classic::widgets::draw_check(canvas, check_box, s, color);
}

/// Design px between a heading's title and its rule.
const HEADING_GAP: i32 = 4;
/// A greyed row's icon in a gutter menu: in greys at this share of its alpha.
const GREYED_ICON: (u32, u32) = (1, 2);

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

/// A scrolling menu's arrow bands in `bg`, each with its 7x4 arrow in `ink`, or in `greyed` once
/// the menu can scroll no further that way.
#[allow(clippy::too_many_arguments)]
fn draw_scroll_arrows(
    canvas: &mut Canvas,
    (ox, oy): (i32, i32),
    layout: &MenuLayout,
    bg: crate::theme::Color,
    ink: crate::theme::Color,
    greyed: crate::theme::Color,
    k: i32,
    s: i32,
) {
    let g = k * s;
    for arrow in [ScrollArrow::Up, ScrollArrow::Down] {
        let Some(r) = layout.arrow_rect(arrow) else {
            return;
        };
        let r = Rect::new(r.x + ox, r.y + oy, r.width, r.height);
        canvas.fill_rect(r, bg);
        let color = if layout.can_scroll(arrow) {
            ink
        } else {
            greyed
        };
        let x = r.x + (r.width / s - 7 * k) / 2 * s;
        let y = centred(&r, 4 * k, s);
        for i in 0..4 {
            let (dx, w) = match arrow {
                ScrollArrow::Up => (3 - i, 2 * i + 1),
                ScrollArrow::Down => (i, 7 - 2 * i),
            };
            canvas.fill_rect(Rect::new(x + dx * g, y + i * g, w * g, g), color);
        }
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

/// The banner's font size at `theme`'s scale.
pub(super) fn banner_px(theme: &Theme) -> u32 {
    theme.fonts.size_px + (theme.text.len(BANNER_OVER) * theme.scale.max(1)) as u32
}

/// `banner` as the band draws it: up to its last word bold, the rest regular.
pub(super) fn banner_parts(banner: &str) -> impl Iterator<Item = (String, bool)> {
    let (bold, regular) = match banner.rsplit_once(' ') {
        Some((head, tail)) => (head.to_string(), format!(" {tail}")),
        None => (banner.to_string(), String::new()),
    };
    [(bold, true), (regular, false)]
        .into_iter()
        .filter(|(part, _)| !part.is_empty())
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
    let size = banner_px(theme);
    let mut pen = theme.text.len(BANNER_PAD) * s;
    for (part, is_bold) in banner_parts(banner) {
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
    // A look with a gutter draws only the menus laid out with one; a gutterless look draws them all.
    if let Some(st) = theme.look.style()
        && let Some(m) = &st.menu
        && (layout.gutter > 0 || m.gutter == 0)
    {
        return draw_gutter_menu(
            canvas, at, theme, st, m, text, layout, items, hovered, underline,
        );
    }
    let cs = &theme.colors;
    let s = theme.scale.max(1);
    let (k, g) = (theme.text.small(), s * theme.text.small());
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
    let greyed = cs.gray_text;
    draw_scroll_arrows(canvas, at, layout, cs.menu, cs.menu_text, greyed, k, s);
    for (i, (item, r)) in items.iter().zip(&layout.rows).enumerate() {
        if !layout.shown(i) {
            continue;
        }
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
            MenuItem::Heading(title) => {
                canvas.fill_rect(*r, cs.menu);
                let text_rect =
                    Rect::new(r.x + layout.text_x, r.y, r.width - layout.text_x, r.height);
                let shown = text.ellipsize(title, text_rect.width, true, size);
                text.draw(
                    canvas,
                    text_rect,
                    &shown,
                    true,
                    cs.menu_text,
                    Align::Left,
                    size,
                );
                let x0 = text_rect.x + text.measure(&shown, true, size) + HEADING_GAP * g;
                let x1 = r.right() - layout.separator_x;
                let y = r.y + (r.height / s / 2) * s;
                if x0 < x1 {
                    if flat {
                        canvas.fill_rect(Rect::new(x0, y, x1 - x0, s), cs.button_shadow);
                    } else {
                        canvas.etched_hline_t(x0, x1, y, cs, s);
                    }
                }
            }
            MenuItem::Entry {
                label,
                icon,
                enabled,
                checked,
                toggle: _,
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
                let text = if *enabled { text } else { text.inked() };
                canvas.fill_rect(*r, bg);
                if flat && hovered_here {
                    canvas.ring_t(*r, cs.hilight, cs.hilight, s);
                }
                // Classic greys a disabled label by etching it; a flat
                // menu draws it grey alone.
                let etched = !*enabled && !hovered_here && !flat;
                let glyph_y = centred(r, 7 * k, s);
                match icon {
                    Some(ic) => {
                        let ic = &*crate::icon::on_ground(ic, cs.menu, cs.menu_text, s);
                        let iy = r.y + (r.height - ic.height as i32) / 2;
                        canvas.blit_canvas(r.x + layout.icon_x, iy, ic);
                    }
                    None if *checked => {
                        let (x, y, mark): (_, _, fn(&mut Canvas, i32, i32, i32, _)) = if *radio {
                            (r.x + BULLET_X * g, centred(r, BULLET_D * k, s), draw_bullet)
                        } else {
                            (r.x + CHECK_X * g, glyph_y + CHECK_DY * g, draw_check)
                        };
                        // Etched as the label is.
                        if etched {
                            mark(canvas, x + s, y + s, g, cs.button_hilight);
                        }
                        mark(canvas, x, y, g, fg);
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
                    draw_arrow(canvas, r.right() - ARROW_RIGHT_INSET * g, glyph_y, g, ink);
                }
                let text_rect = Rect::new(
                    r.x + layout.text_x,
                    r.y,
                    (r.width - layout.text_x - ARROW_SPACE * g).max(0),
                    r.height,
                );
                let shown = text.ellipsize(label, text_rect.width, *bold, size);
                if let Some(sc) = shortcut {
                    let sc = &keycap::shown_chord(sc);
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
    // dots (or is only the dots that fit); an access key past that prefix is not drawn.
    let visible = if ellipsized {
        shown.chars().count().saturating_sub(3)
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
    let (k, g) = (theme.text.small(), s * theme.text.small());
    let marks = theme.paint().small_look(canvas);
    let marks = marks.style().unwrap_or(st);
    let size = theme.fonts.size_px;
    let all = Rect::new(ox, oy, layout.width, layout.height);
    look::draw_frame(st, m, canvas, all, cs.menu, s);
    let ins = layout.inset;
    let rows_h = layout.height - ins.top - ins.bottom;
    for c in 0..layout.columns as i32 {
        let x = ox + layout.band + c * layout.column_width + ins.left;
        if layout.gutter > 0 {
            canvas.fill_rect(
                Rect::new(x, oy + ins.top, layout.gutter, rows_h),
                m.gutter_fill,
            );
            let dx = x + layout.gutter;
            canvas.fill_rect(Rect::new(dx, oy + ins.top, s, rows_h), m.divider.0);
            canvas.fill_rect(Rect::new(dx + s, oy + ins.top, s, rows_h), m.divider.1);
        }
        if c > 0 {
            let x = x - ins.left;
            let line = |x: i32| Rect::new(x, oy + ins.top, s, rows_h);
            canvas.fill_rect(line(x - s), m.rule.0);
            canvas.fill_rect(line(x), m.rule.1);
        }
    }
    draw_scroll_arrows(canvas, (ox, oy), layout, cs.menu, m.arrow, m.greyed, k, s);
    let past_divider = layout.gutter + 2 * s;
    for (i, (item, r)) in items.iter().zip(&layout.rows).enumerate() {
        if !layout.shown(i) {
            continue;
        }
        let r = Rect::new(r.x + ox, r.y + oy, r.width, r.height);
        if let MenuItem::Heading(title) = item {
            let text_rect = Rect::new(r.x + layout.text_x, r.y, r.width - layout.text_x, r.height);
            let shown = text.ellipsize(title, text_rect.width, false, size);
            text.draw(
                canvas,
                text_rect,
                &shown,
                false,
                m.heading,
                Align::Left,
                size,
            );
            let x0 = text_rect.x + text.measure(&shown, false, size) + HEADING_GAP * g;
            let x1 = r.right() - m.shortcut_end * s;
            if x0 < x1 {
                let y = r.y + (r.height / s / 2) * s;
                canvas.fill_rect(Rect::new(x0, y, x1 - x0, s), m.rule.0);
                canvas.fill_rect(Rect::new(x0, y + s, x1 - x0, s), m.rule.1);
            }
            continue;
        }
        let MenuItem::Entry {
            label,
            icon,
            enabled,
            checked,
            toggle: _,
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
        let lit_ink = lit
            .then(|| st.text_color(crate::style::Part::MenuItem, crate::style::State::Hot))
            .flatten();
        let fg = match (*enabled, lit_ink) {
            (false, _) => m.greyed,
            (true, Some(ink)) => ink,
            (true, None) => cs.menu_text,
        };
        let square = Rect::new(r.x, r.y, r.height, r.height);
        match icon {
            Some(ic) => {
                // A checked row's icon sits in the check's box, which stands for the mark.
                if *checked {
                    look::draw_check_box(marks, canvas, square, lit, g);
                }
                let ground = if layout.gutter > 0 {
                    m.gutter_fill
                } else {
                    cs.menu
                };
                let ic = &*crate::icon::on_ground(ic, ground, cs.menu_text, s);
                let iy = r.y + (r.height - ic.height as i32) / 2;
                if layout.gutter > 0 && !*enabled {
                    let (num, den) = GREYED_ICON;
                    canvas.blit_over_greyed(r.x + layout.icon_x, iy, ic, num, den);
                } else {
                    canvas.blit_canvas(r.x + layout.icon_x, iy, ic);
                }
            }
            None if *checked => {
                look::draw_check(marks, m, canvas, square, *radio, lit, g);
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
            let ink = match (*enabled, lit_ink) {
                (false, _) => m.greyed,
                (true, Some(ink)) => ink,
                (true, None) => m.arrow,
            };
            draw_arrow(
                canvas,
                r.right() - m.arrow_inset * g,
                centred(&r, 7 * k, s),
                g,
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
            let sc = &keycap::shown_chord(sc);
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
