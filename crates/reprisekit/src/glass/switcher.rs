//! Reprise Glass's Alt+Tab panel: a glass panel over the blurred backdrop with
//! a soft shadow, the hovered (else selected) entry's title centred on top in
//! the caption face over its glow, and rows of pictures each with a shadow,
//! the selected cell in a light rounded box and the hovered one in a fainter
//! one. Layout and hit test are the shared ones in `crate::switcher`, given
//! Glass's [`GRID`]; this module draws the [`PANEL`] parts, the boxes and the panel.

use super::Settings;
use super::panel::{self, Corners, Ends, Fade, LightBox, Panel, Shadow};
use crate::raster::{Canvas, Insets, Rect};
use crate::style::{GlowMask, Layout, Part, PartImage, State, Style};
use crate::switcher::{Grid, ICON_PX, SwitcherLayout, ThumbCell, badge};
use crate::text::TextRenderer;
use crate::theme::Color;

/// Glass's panel: `controls::switcher::CLASSIC`'s cell, padding, title and
/// picture numbers with the title at 16 px. The shadow reaches 13 px left, 12
/// up, 18 right and 19 down; 5, 3, 2, 1 and 1 px of the outer rows lie
/// outside the rounded corners.
pub const GRID: Grid = Grid {
    cell: (144, 85),
    row_gap: 8,
    pad: Insets::new(8, 45, 8, 8),
    title_baseline: 30,
    title_px: 16,
    picture: Rect::new(11, 8, 122, 69),
    badge: (3, 3),
    max_cols: 7,
    shadow: Insets::new(13, 12, 18, 19),
    corner: &[5, 3, 2, 1, 1],
};

/// The panel: the focused window's plain glass with the caption's sheen at
/// both ends, brightest along the top, the frame's outline and light line, cut
/// corners per [`GRID`], over a soft shadow down and right. The backdrop's
/// streaks show through; the panel adds none.
pub const PANEL: Panel = Panel {
    corners: Corners::Cut {
        rows: GRID.corner,
        bottom: true,
    },
    lines: (1.0, 1.0),
    sheen_rows: 0,
    ends: Some(ENDS),
    deepen: 0.0,
    density: 0.0,
    shadow: Some(Shadow {
        margin: GRID.shadow,
        alpha: 200.0,
        sigma: 5.5,
        offset: (1.5, 2.25),
        grow: 3.0,
        radius: 0.0,
    }),
};

const ENDS: Ends = Ends {
    reach: 128.0,
    slope: 1.2,
    width: 90.0,
    fade: (90.0, STRETCHED_ROW as f64),
};

/// The icon style's panel (`[switcher] style = "icons"`): [`GRID`]'s padding,
/// title, shadow and corners round 56 px square cells with a 32 px icon each.
/// Up to 7 to a row.
pub const ICON_GRID: Grid = Grid {
    cell: (56, 56),
    picture: Rect::new(12, 12, 32, 32),
    ..GRID
};

const ICON_H: i32 = ICON_GRID.pad.top + ICON_GRID.cell.1 + ICON_GRID.pad.bottom;

/// The row a taller icon panel stretches: as far above its bottom as [`STRETCHED_ROW`] is in a thumbnail panel.
const ICON_STRETCHED_ROW: i32 =
    ICON_H - (GRID.pad.top + GRID.cell.1 + GRID.pad.bottom - STRETCHED_ROW);

/// [`PANEL`] for the icon panel, its sheen easing out above [`ICON_STRETCHED_ROW`].
const ICON_PANEL: Panel = Panel {
    ends: Some(Ends {
        fade: (60.0, ICON_STRETCHED_ROW as f64),
        ..ENDS
    }),
    ..PANEL
};

/// How far in from either end of the panel its sheen reaches; past it the glass is plain.
const SHEEN_REACH: i32 = 176;

/// The row of a one-row panel that a taller one stretches: the end sheens
/// have eased out by it and the shadow beside it matches every taller row's.
const STRETCHED_ROW: i32 = 122;

/// The box round the selected cell: a blue outline and a white line inside
/// it round a light blue fill, darkening from top to bottom.
pub const SELECT: LightBox = LightBox {
    outline: (Color::rgb(111, 152, 201), 230),
    line: Some((
        (Color::rgb(238, 246, 255), 219),
        (Color::rgb(219, 231, 244), 218),
    )),
    fill: (
        (Color::rgb(190, 220, 255), 152),
        (Color::rgb(112, 154, 201), 152),
    ),
    radius: 2.5,
    fade: Fade::Box,
};

/// The box round the hot cell: [`SELECT`]'s shape, fainter, its fill flat.
pub const HOT: LightBox = LightBox {
    outline: (Color::rgb(112, 152, 202), 153),
    line: Some((
        (Color::rgb(221, 230, 242), 172),
        (Color::rgb(221, 230, 242), 172),
    )),
    fill: (
        (Color::rgb(108, 151, 200), 83),
        (Color::rgb(108, 151, 200), 83),
    ),
    radius: 2.5,
    fade: Fade::Box,
};

/// The shadow round a picture, over its rect grown by 4: a soft black rect a
/// pixel right of and below the picture, showing about 2 px above and left, 4 px right and below.
pub const PICTURE_SHADOW: Shadow = Shadow {
    margin: Insets::new(4, 4, 4, 4),
    alpha: 142.0,
    sigma: 1.3,
    offset: (1.0, 1.0),
    grow: 0.8,
    radius: 0.0,
};

/// The panel over its whole canvas ([`PANEL`]), shadow included, for `s`,
/// painted at `d`: one row of cells tall and [`SHEEN_REACH`] past both
/// sheens wide, nine-gridded so the plain middle column and [`STRETCHED_ROW`]
/// stretch. A canvas narrower than both ends shares them, each keeping its own pixels.
pub fn frame_part(s: &Settings, d: i32) -> PartImage {
    let h = GRID.pad.top + GRID.cell.1 + GRID.pad.bottom;
    let fixed = Insets::new(
        SHEEN_REACH,
        STRETCHED_ROW,
        SHEEN_REACH,
        h - STRETCHED_ROW - 1,
    );
    panel::part(&PANEL, s, 2 * SHEEN_REACH + 1, h, fixed, d)
}

/// [`ICON_PANEL`] over its whole canvas, one row of icon cells tall, as [`frame_part`].
pub fn icon_frame_part(s: &Settings, d: i32) -> PartImage {
    let fixed = Insets::new(
        SHEEN_REACH,
        ICON_STRETCHED_ROW,
        SHEEN_REACH,
        ICON_H - ICON_STRETCHED_ROW - 1,
    );
    panel::part(&ICON_PANEL, s, 2 * SHEEN_REACH + 1, ICON_H, fixed, d)
}

/// `b` as a part for a cell, painted at `d`: a cell tall, nine-gridded with
/// 4 px borders, rows between stretched so the fill keeps its fade.
fn cell_box(b: &LightBox, d: i32) -> PartImage {
    PartImage::new(
        panel::light_box(b, 9, GRID.cell.1, d),
        &[State::Normal],
        Layout::Vertical,
    )
    .sizing(Insets::new(4, 4, 4, 4))
    .at_scale(d)
}

pub fn select_part(d: i32) -> PartImage {
    cell_box(&SELECT, d)
}

pub fn hot_part(d: i32) -> PartImage {
    cell_box(&HOT, d)
}

/// [`PICTURE_SHADOW`] as a part painted at `d`, 17x17 design px with 8 px borders.
pub fn shadow_part(d: i32) -> PartImage {
    let m = PICTURE_SHADOW.margin;
    PartImage::new(
        panel::shadow(&PICTURE_SHADOW.at(d), 9 * d, 9 * d),
        &[State::Normal],
        Layout::Vertical,
    )
    .sizing(Insets::new(8, 8, 8, 8))
    .outset(m)
    .at_scale(d)
}

/// The title band's top rows the glow leaves alone: the outline and light line.
const GLOW_TOP: i32 = 2;

/// Draw the panel into `canvas`, sized to `layout.canvas()` at `layout.scale`:
/// the frame and shadow (the icon panel's on [`ICON_GRID`]), then per shown
/// cell (`cells[0]` is entry `layout.first`) the box if it is `selected`, else
/// the fainter one if `hot`, its picture's shadow and still picture, or its
/// icon centred in the picture's box when it has none. Last the `title` in
/// `text` (caption face, `size_px`) centred over the caption's glow. Live
/// pictures and their badges are the host's.
#[expect(clippy::too_many_arguments)]
pub fn draw(
    canvas: &mut Canvas,
    style: &Style,
    text: &TextRenderer,
    size_px: u32,
    layout: &SwitcherLayout,
    title: &str,
    cells: &[ThumbCell],
    selected: usize,
    hot: Option<usize>,
) {
    let s = layout.scale.max(1);
    let area = layout.canvas();
    let at = |r: Rect| Rect::new(r.x - area.x, r.y - area.y, r.width, r.height);
    let whole = Rect::new(0, 0, area.width, area.height);
    let frame = if layout.grid == Some(ICON_GRID) {
        Part::SwitcherIconFrame
    } else {
        Part::SwitcherFrame
    };
    style.draw(canvas, whole, frame, State::Normal, s);
    for (k, (cell, well)) in layout.cells.iter().zip(&layout.wells).enumerate() {
        let i = layout.first + k;
        if i == selected {
            style.draw(canvas, at(*cell), Part::SwitcherSelect, State::Normal, s);
        } else if hot == Some(i) {
            style.draw(canvas, at(*cell), Part::SwitcherHot, State::Normal, s);
        }
        let Some(c) = cells.get(k) else { continue };
        match c.picture {
            Some(r) => {
                let r = at(r);
                style.draw(canvas, r, Part::SwitcherShadow, State::Normal, s);
                if let Some(still) = c.still {
                    canvas.blit_over(r.x, r.y, still);
                }
            }
            None => {
                if let Some(icon) = c.icon {
                    let w = at(*well);
                    let (iw, ih) = (icon.width as i32, icon.height as i32);
                    canvas.blit_over(w.x + (w.width - iw) / 2, w.y + (w.height - ih) / 2, icon);
                }
            }
        }
    }
    let band = at(layout.field);
    let glass = at(Rect::new(0, 0, layout.rect.width, layout.rect.height)).inset(2 * s);
    let baseline = layout.grid.unwrap_or(GRID).title_baseline * s - area.y;
    draw_title(
        canvas, style, text, size_px, band, baseline, glass, title, s,
    );
}

/// `title` centred on `band` with its baseline at `baseline`, in the caption's
/// ink over its glow: coverage rendered into a scratch mask over the band's
/// rows, reaching past the text as far as the glow does, the glow's alpha
/// worked out from it and laid down from the band's third row, clipped to
/// `glass`, then the ink through the same mask.
#[expect(clippy::too_many_arguments)]
pub(super) fn draw_title(
    canvas: &mut Canvas,
    style: &Style,
    text: &TextRenderer,
    size: u32,
    band: Rect,
    baseline: i32,
    glass: Rect,
    title: &str,
    s: i32,
) {
    let spec = &style.caption_text;
    let title = text.ellipsize(title, band.width, spec.bold, size);
    let w = text.measure(&title, spec.bold, size);
    if title.is_empty() || w <= 0 || band.height <= 0 {
        return;
    }
    let glow = style.caption_title.glow;
    let reach = glow.map_or(0, |g| (g.reach)(s));
    let x = band.x + (band.width - w) / 2;
    let (mw, mh) = ((w + 2 * reach) as usize, band.height as usize);
    let scratch = Canvas::new(mw as u32, mh as u32);
    let clip = scratch.bounds();
    let coverage = scratch.ink_coverage(|c, white| {
        text.draw_run(
            c,
            reach,
            baseline - band.y,
            clip,
            &title,
            spec.bold,
            white,
            size,
        );
    });
    let (mx, my) = (x - reach, band.y);
    if let Some(glow) = glow {
        let alpha = (glow.alpha)(&GlowMask {
            coverage: &coverage,
            width: mw,
            height: mh,
            rows: (((GLOW_TOP * s) as usize).min(mh), mh),
            active: true,
            scale: s,
        });
        canvas.blit_mask_over(mx, my, mw, mh, &alpha, glow.color, glass);
    }
    canvas.blit_mask_over(mx, my, mw, mh, &coverage, spec.color, glass);
}

/// Draw the icons of the pictured cells into `canvas` (`layout.canvas()` size,
/// otherwise clear, since it goes over the live pictures), each with its
/// bottom-right 3 px past its picture's. A cell without an icon gets none.
pub fn draw_badges(canvas: &mut Canvas, layout: &SwitcherLayout, cells: &[ThumbCell]) {
    let s = layout.scale.max(1);
    let area = layout.canvas();
    let grid = layout.grid.unwrap_or(GRID);
    for c in cells {
        let (Some(r), Some(icon)) = (c.picture, c.icon) else {
            continue;
        };
        let (x, y) = badge(&grid, r, ICON_PX * s, s);
        canvas.blit_over(x - area.x, y - area.y, icon);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glass::streak::{self, STREAK_SLOPE};
    use crate::interface::Interface;
    use crate::raster::Rect;
    use crate::theme::Color;

    const PANEL_AT: (i32, i32) = (736, 451);

    /// The 3-entry panel, cell 1 selected and `hot` under the pointer, over grey with the backdrop's streaks inside it.
    fn panel(hot: Option<usize>) -> (Canvas, SwitcherLayout) {
        let theme = crate::interfaces::theme_for(Interface::Glass);
        let style = theme.look.style().unwrap();
        let text = crate::look::caption_text(style);
        let l = SwitcherLayout::thumbnails(3, 1, Rect::new(0, 0, 1920, 1040), GRID);
        assert_eq!((l.rect.x, l.rect.y), PANEL_AT);
        let a = l.canvas();
        let mut c = Canvas::new(a.width as u32, a.height as u32);
        c.fill_rect(c.bounds(), Color::rgb(128, 128, 128));
        // The backdrop's streaks blended over grey and rounded, inside the panel only.
        for y in 0..l.rect.height {
            for x in 0..l.rect.width {
                let (sx, sy) = (x + PANEL_AT.0, y + PANEL_AT.1);
                let px = streak::at(f64::from(sx) - STREAK_SLOPE * f64::from(sy));
                let keep = f64::from(255 - (px >> 24)) / 255.0;
                let ch =
                    |shift: u32| (f64::from((px >> shift) & 0xFF) + 128.0 * keep).round() as u8;
                c.put_raw(x - a.x, y - a.y, Color::rgb(ch(16), ch(8), ch(0)).argb());
            }
        }
        let pic = |k: usize, dx: i32, w: i32| {
            let r = l.cells[k];
            Some(Rect::new(r.x + dx, r.y + 8, w, 69))
        };
        let shown = [
            ThumbCell {
                icon: None,
                picture: pic(0, 14, 116),
                still: None,
            },
            ThumbCell {
                icon: None,
                picture: pic(1, 14, 116),
                still: None,
            },
            ThumbCell {
                icon: None,
                picture: pic(2, 11, 122),
                still: None,
            },
        ];
        draw(
            &mut c,
            style,
            &text,
            16,
            &l,
            "Untitled - Notepad",
            &shown,
            1,
            hot,
        );
        (c, l)
    }

    #[test]
    fn the_hot_cell_lightens_and_the_selection_stays() {
        let (plain, l) = panel(None);
        let (hot, _) = panel(Some(2));
        let a = l.canvas();
        let lum = |p: u32| ((p >> 16) & 255) + ((p >> 8) & 255) + (p & 255);
        let c2 = l.cells[2];
        let (x, y) = (c2.x + 3 - a.x, c2.y + c2.height / 2 - a.y);
        assert!(
            lum(hot.get(x, y)) > lum(plain.get(x, y)) + 15,
            "the hover box lightens the cell's glass"
        );
        let c1 = l.cells[1];
        let (x, y) = (c1.x + 3 - a.x, c1.y + 40 - a.y);
        assert_eq!(hot.get(x, y), plain.get(x, y), "the selection box stays");
        let (on_selected, _) = panel(Some(1));
        assert_eq!(
            on_selected.get(c1.x - a.x, c1.y + 40 - a.y),
            plain.get(c1.x - a.x, c1.y + 40 - a.y),
            "the pointer on the selection draws the selection box alone"
        );
    }

    fn frame_only(n: usize) -> (Canvas, SwitcherLayout) {
        frame_on(n, GRID)
    }

    fn frame_on(n: usize, grid: Grid) -> (Canvas, SwitcherLayout) {
        let theme = crate::interfaces::theme_for(Interface::Glass);
        let style = theme.look.style().unwrap();
        let text = crate::look::caption_text(style);
        let l = SwitcherLayout::thumbnails(n, 0, Rect::new(0, 0, 1920, 1040), grid);
        let a = l.canvas();
        let mut c = Canvas::new(a.width as u32, a.height as u32);
        draw(&mut c, style, &text, 16, &l, "", &[], usize::MAX, None);
        (c, l)
    }

    #[test]
    fn a_large_panel_stretches_plain_glass_between_its_ends() {
        use crate::switcher::Size;
        let (small, ls) = frame_on(7, GRID);
        let (large, ll) = frame_on(7, GRID.sized(Size::Large));
        let grow = ll.rect.height - ls.rect.height;
        assert_eq!(grow, 154 - 85, "one row of Large cells");
        let (sx, lx) = (-ls.canvas().x, -ll.canvas().x);
        let top = -ls.canvas().y;
        for x in 0..SHEEN_REACH + sx {
            for y in 0..top + STRETCHED_ROW {
                assert_eq!(large.get(x, y), small.get(x, y), "the top left, ({x}, {y})");
            }
            for y in top + STRETCHED_ROW + 1..small.height as i32 {
                assert_eq!(
                    large.get(x, y + grow),
                    small.get(x, y),
                    "the bottom left, ({x}, {y})"
                );
            }
        }
        // The right end too, by its distance from the panel's right edge.
        let (sr, lr) = (sx + ls.rect.width, lx + ll.rect.width);
        for d in -SHEEN_REACH..small.width as i32 - sr {
            for y in 0..top + STRETCHED_ROW {
                assert_eq!(
                    large.get(lr + d, y),
                    small.get(sr + d, y),
                    "the top right, ({d}, {y})"
                );
            }
        }
    }

    #[test]
    fn one_and_two_entry_panels_keep_their_ends_as_drawn() {
        let (three, l3) = frame_only(3);
        let (w3, top) = (three.width as i32, -l3.canvas().y);
        for n in [1, 2] {
            let (c, l) = frame_only(n);
            let w = c.width as i32;
            let panel_right = -l.canvas().x + l.rect.width - 1;
            let panel_right3 = -l3.canvas().x + l3.rect.width - 1;
            let left = -l.canvas().x;
            for y in 0..c.height as i32 {
                assert_eq!(
                    c.get(0, y),
                    three.get(0, y),
                    "{n}: the shadow's column 0, row {y}"
                );
                assert_eq!(
                    c.get(left, y),
                    three.get(left, y),
                    "{n}: the left outline, row {y}"
                );
                assert_eq!(
                    c.get(panel_right, y),
                    three.get(panel_right3, y),
                    "{n}: the right outline, row {y}"
                );
                assert_eq!(
                    c.get(w - 1, y),
                    three.get(w3 - 1, y),
                    "{n}: the shadow's last column, row {y}"
                );
            }
            assert!(
                c.get(left, top + 60) >> 24 > 0xB0,
                "{n}: the outline is there"
            );
        }
    }

    #[test]
    fn the_frame_part_draws_the_panel_as_drawn_at_its_size() {
        // One to three rows of 3 cells and up: the fixed ends reach past both sheens and the stretched row lies clear of the shadow's corners.
        let mut rows = std::collections::BTreeSet::new();
        for n in 3..=21 {
            let (c, l) = frame_only(n);
            let drawn = panel::draw(&PANEL, &Settings::default(), l.rect.width, l.rect.height, 1);
            assert_eq!(
                (c.width, c.height),
                (drawn.width, drawn.height),
                "{n}: its size"
            );
            for y in 0..c.height as i32 {
                for x in 0..c.width as i32 {
                    assert_eq!(
                        c.get(x, y),
                        drawn.get(x, y),
                        "{n} cells: ({x}, {y}) as drawn"
                    );
                }
            }
            rows.insert(l.rect.height);
        }
        assert_eq!(rows.len(), 3, "one, two and three rows: {rows:?}");
    }

    #[test]
    fn a_badge_sits_past_its_picture() {
        let l = SwitcherLayout::thumbnails(1, 0, Rect::new(0, 0, 1920, 1040), GRID);
        let a = l.canvas();
        let r = Rect::new(l.cells[0].x + 14, l.cells[0].y + 8, 116, 69);
        let red = Color::rgb(255, 0, 0);
        let mut icon = Canvas::new(32, 32);
        icon.fill_rect(icon.bounds(), red);
        let cells = [ThumbCell {
            icon: Some(&icon),
            picture: Some(r),
            still: None,
        }];
        let mut c = Canvas::new(a.width as u32, a.height as u32);
        draw_badges(&mut c, &l, &cells);
        let (x, y) = badge(&GRID, r, 32, 1);
        let (x, y) = (x - a.x, y - a.y);
        assert_eq!(c.get(x, y), red.argb(), "its top-left");
        assert_eq!(c.get(x + 31, y + 31), red.argb(), "its bottom-right");
        assert_eq!(c.get(x - 1, y), 0, "nothing left of it");
        assert_eq!(c.get(x + 32, y + 31), 0, "nothing right of it");
    }

    #[test]
    fn a_panel_at_scale_2_is_its_scale_1_design_twice_over() {
        // Panel, shadow, selection and hot boxes and a picture's shadow through the scale-2 style: the scale-1 panel at twice the size (`panel::assert_twice_over`), but for the round corners.
        let style = crate::glass::style(&Settings::default());
        let style2 = style.at_scale(2);
        let text = crate::look::caption_text(&style);
        let one = SwitcherLayout::thumbnails(2, 1, Rect::new(0, 0, 1920, 1040), GRID);
        let two = one.scaled(2);
        let pic = one.cells[0];
        let pic = Rect::new(pic.x + 14, pic.y + 8, 116, 69);
        let cells = [
            ThumbCell {
                icon: None,
                picture: Some(pic),
                still: None,
            },
            ThumbCell {
                icon: None,
                picture: None,
                still: None,
            },
        ];
        let cells2 = [
            ThumbCell {
                picture: Some(pic.scaled(2)),
                ..cells[0]
            },
            cells[1],
        ];
        let draw_at = |st: &Style, l: &SwitcherLayout, cells: &[ThumbCell], size: u32| {
            let a = l.canvas();
            let mut c = Canvas::new(a.width as u32, a.height as u32);
            c.fill_rect(c.bounds(), Color::rgb(128, 128, 128));
            draw(&mut c, st, &text, size, l, "", cells, 1, Some(0));
            c
        };
        let a = two.canvas();
        let at = |r: Rect| Rect::new(r.x - a.x, r.y - a.y, r.width, r.height);
        let panel = at(Rect::new(0, 0, two.rect.width, two.rect.height));
        let boxes = [at(two.cells[0]), at(two.cells[1])];
        panel::assert_twice_over(
            "Alt+Tab",
            &draw_at(&style, &one, &cells, 16),
            &draw_at(&style2, &two, &cells2, 32),
            |x, y| {
                panel::in_corner(panel, 7.0, x, y)
                    || panel::beside(panel, x, y)
                    || boxes.iter().any(|&b| panel::in_corner(b, 2.5, x, y))
            },
        );
    }

    /// The icon panel for `n` entries, the second selected, each a solid red 32 px icon, over clear.
    fn icon_panel(n: usize) -> (Canvas, SwitcherLayout) {
        let theme = crate::interfaces::theme_for(Interface::Glass);
        let style = theme.look.style().unwrap();
        let text = crate::look::caption_text(style);
        let l = SwitcherLayout::thumbnails(n, 1, Rect::new(0, 0, 1920, 1040), ICON_GRID);
        let mut red = Canvas::new(32, 32);
        red.fill_rect(red.bounds(), Color::rgb(255, 0, 0));
        let cells: Vec<ThumbCell> = (0..n)
            .map(|_| ThumbCell {
                icon: Some(&red),
                picture: None,
                still: None,
            })
            .collect();
        let a = l.canvas();
        let mut c = Canvas::new(a.width as u32, a.height as u32);
        draw(&mut c, style, &text, 16, &l, "", &cells, 1, None);
        (c, l)
    }

    #[test]
    fn the_icon_panel_is_the_glass_panel_round_56_px_cells() {
        let (c, l) = icon_panel(3);
        assert_eq!((l.rect.width, l.rect.height), (16 + 3 * 56, 45 + 56 + 8));
        let (c9, l9) = icon_panel(9);
        assert_eq!(
            (l9.rect.width, l9.rect.height),
            (16 + 7 * 56, 45 + 2 * 56 + 8 + 8),
            "seven to a row, 8 px between rows"
        );
        let a = l.canvas();
        let alpha = |c: &Canvas, x: i32, y: i32| c.get(x - a.x, y - a.y) >> 24;
        // The outline is opaque, the glass between cells translucent.
        assert!(alpha(&c, 0, 30) > 0xC0, "the outline down the left side");
        let glass = alpha(&c, 4, 70);
        assert!(
            glass > 0 && glass < 0xF0,
            "the glass is translucent: {glass:X}"
        );
        for (k, cell) in l.cells.iter().enumerate() {
            for (dx, dy) in [(12, 12), (43, 43)] {
                assert_eq!(
                    c.get(cell.x + dx - a.x, cell.y + dy - a.y),
                    0xFFFF_0000,
                    "cell {k}: the icon at ({dx}, {dy})"
                );
            }
            assert_ne!(c.get(cell.x + 11 - a.x, cell.y + 12 - a.y), 0xFFFF_0000);
        }
        let lum = |p: u32| ((p >> 16) & 255) + ((p >> 8) & 255) + (p & 255);
        let (s0, s1) = (l.cells[0], l.cells[1]);
        let at = |cell: Rect| c.get(cell.x + 5 - a.x, cell.y + 28 - a.y);
        assert!(lum(at(s1)) > lum(at(s0)) + 30, "the selection box");
        let taller = l9.canvas();
        let bottom = taller.height - 1 + taller.y;
        assert_eq!(
            c9.get(l9.rect.width / 2 - taller.x, bottom - taller.y),
            c.get(l.rect.width / 2 - a.x, a.height - 1),
            "a taller panel keeps the shadow's last row"
        );
    }

    #[test]
    fn the_icon_frame_part_draws_the_panel_as_drawn_at_its_size() {
        // A full row of seven and up to three rows: as wide as both sheens.
        let theme = crate::interfaces::theme_for(Interface::Glass);
        let style = theme.look.style().unwrap();
        let text = crate::look::caption_text(style);
        let mut rows = std::collections::BTreeSet::new();
        for n in 7..=21 {
            let l = SwitcherLayout::thumbnails(n, 0, Rect::new(0, 0, 1920, 1040), ICON_GRID);
            let a = l.canvas();
            let mut c = Canvas::new(a.width as u32, a.height as u32);
            draw(&mut c, style, &text, 16, &l, "", &[], usize::MAX, None);
            let drawn = panel::draw(
                &ICON_PANEL,
                &Settings::default(),
                l.rect.width,
                l.rect.height,
                1,
            );
            assert_eq!(
                (c.width, c.height),
                (drawn.width, drawn.height),
                "{n}: its size"
            );
            for y in 0..c.height as i32 {
                for x in 0..c.width as i32 {
                    assert_eq!(
                        c.get(x, y),
                        drawn.get(x, y),
                        "{n} cells: ({x}, {y}) as drawn"
                    );
                }
            }
            rows.insert(l.rect.height);
        }
        assert_eq!(rows.len(), 3, "one, two and three rows: {rows:?}");
    }
}
