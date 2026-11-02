//! The keyboard shortcut sheet: groups of bindings in columns, a group never split across two.
//! Under a glass look it is a glass panel with white text; it takes no clicks.

use crate::access::{self, Tree, accesskit};
use crate::look::{Paint, panel};
use crate::raster::{Canvas, EdgeStyle, Rect};
use crate::text::{Align, TextRenderer};
use crate::theme::{Color, TextUnits, Theme};
use accesskit::{NodeId, Role};

pub const TITLE: &str = "Keyboard Shortcuts";
const PAD: i32 = 12;
const TITLE_H: i32 = 24;
const HEAD_H: i32 = 20;
const LINE_H: i32 = 16;
const GROUP_GAP: i32 = 8;
const COL_GAP: i32 = 24;
const KEY_GAP: i32 = 12;
const KEYS_MAX: i32 = 200;
const WHAT_MAX: i32 = 240;
/// Between a heading's label and its rule.
const RULE_GAP: i32 = 6;
/// The share of the room's height a sheet aims to stay within, in percent: past
/// it, groups go into another column while the width allows.
pub const TALL: i32 = 60;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    /// "Super+Shift+B", or several chords joined ("Alt+F4, Super+Q").
    pub keys: String,
    pub what: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub title: String,
    pub rows: Vec<Binding>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeySheet {
    pub groups: Vec<Group>,
}

/// Where the parts go, logical px in sheet coordinates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub width: i32,
    pub height: i32,
    pub title: Rect,
    /// Each group's heading row, the full column width.
    pub headings: Vec<Rect>,
    /// Each group's rows: the keys' rect and the description's.
    pub rows: Vec<Vec<(Rect, Rect)>>,
    pub columns: usize,
}

/// The fewest columns of `heights` (stacked `gap` apart, in order) no taller than `limit`.
fn columns_needed(heights: &[i32], gap: i32, limit: i32) -> usize {
    let (mut cols, mut used) = (1, 0);
    for &h in heights {
        if used > 0 && used + gap + h > limit {
            cols += 1;
            used = h;
        } else if used > 0 {
            used += gap + h;
        } else {
            used = h;
        }
    }
    cols
}

/// The shortest column height that stacks `heights` into at most `cols` columns.
fn balanced_limit(heights: &[i32], gap: i32, cols: usize) -> i32 {
    let (mut lo, mut hi) = (
        heights.iter().copied().max().unwrap_or(0),
        heights.iter().sum::<i32>() + gap * heights.len() as i32,
    );
    while lo < hi {
        let mid = (lo + hi) / 2;
        if columns_needed(heights, gap, mid) <= cols {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    lo
}

/// `text`'s width in logical px, measured at `theme`'s scale and text size.
fn width_of(text: &TextRenderer, theme: &Theme, s: &str, bold: bool) -> i32 {
    let scale = theme.scale.max(1);
    let w = text.measure(s, bold, theme.fonts.size_px);
    (w + scale - 1) / scale
}

/// The sheet laid out to fit `room` (logical px) where it can: the fewest columns
/// whose tallest keeps the sheet within [`TALL`] of `room`'s height, at most as
/// many as fit its width.
pub fn layout(k: &KeySheet, theme: &Theme, text: &TextRenderer, room: (i32, i32)) -> Layout {
    let u: TextUnits = theme.text;
    let (pad, line, head, gap) = (u.len(PAD), u.len(LINE_H), u.len(HEAD_H), u.len(GROUP_GAP));
    let rows = k.groups.iter().flat_map(|g| &g.rows);
    let keys_w = (rows.clone())
        .map(|b| width_of(text, theme, &b.keys, true))
        .max()
        .unwrap_or(0)
        .min(u.len(KEYS_MAX));
    let heads = k
        .groups
        .iter()
        .map(|g| width_of(text, theme, &g.title, true));
    let what_w = (rows.map(|b| width_of(text, theme, &b.what, false)))
        .chain(heads)
        .max()
        .unwrap_or(0)
        .min(u.len(WHAT_MAX));
    let key_gap = u.len(KEY_GAP);
    let col_w = keys_w + key_gap + what_w;
    let col_gap = u.len(COL_GAP);
    let title_h = u.len(TITLE_H);
    let heights: Vec<i32> = (k.groups.iter())
        .map(|g| head + g.rows.len() as i32 * line)
        .collect();
    let max_cols = ((room.0 - 2 * pad + col_gap) / (col_w + col_gap)).max(1) as usize;
    let fit_h = room.1 * TALL / 100 - 2 * pad - title_h;
    let cols = columns_needed(&heights, gap, fit_h).min(max_cols);
    let limit = balanced_limit(&heights, gap, cols);
    let mut headings = Vec::with_capacity(k.groups.len());
    let mut placed = Vec::with_capacity(k.groups.len());
    let top = pad + title_h;
    let (mut col, mut y, mut bottom) = (0, top, top);
    for (g, &h) in k.groups.iter().zip(&heights) {
        if y > top && y - top + gap + h > limit {
            col += 1;
            y = top;
        } else if y > top {
            y += gap;
        }
        let x = pad + col * (col_w + col_gap);
        headings.push(Rect::new(x, y, col_w, head));
        let rows = (0..g.rows.len() as i32)
            .map(|i| {
                let ry = y + head + i * line;
                (
                    Rect::new(x, ry, keys_w, line),
                    Rect::new(x + keys_w + key_gap, ry, what_w, line),
                )
            })
            .collect();
        placed.push(rows);
        y += h;
        bottom = bottom.max(y);
    }
    let used_cols = if k.groups.is_empty() { 1 } else { col + 1 };
    let width = 2 * pad + used_cols * col_w + (used_cols - 1) * col_gap;
    let width = width.max(2 * pad + width_of(text, theme, TITLE, true));
    Layout {
        width,
        height: bottom + pad,
        title: Rect::new(pad, pad, width - 2 * pad, title_h),
        headings,
        rows: placed,
        columns: used_cols as usize,
    }
}

/// Draw the sheet laid out as `lay`. Under glass the canvas reaches
/// [`panel::margin`] past the sheet for its shadow.
pub fn draw(theme: &Theme, text: &TextRenderer, k: &KeySheet, lay: &Layout) -> Canvas {
    let paint = &theme.paint();
    let (cs, s) = (paint.cs, paint.s);
    if let Some((style, spec)) = panel::glass(theme) {
        return panel::draw_inked(style, spec, lay.width, lay.height, s, |c| {
            body(
                c,
                paint,
                text,
                theme.fonts.size_px,
                k,
                lay,
                Some(panel::INK),
            )
        });
    }
    let mut c = Canvas::new((lay.width * s) as u32, (lay.height * s) as u32);
    c.fill_rect(c.bounds(), cs.button_face);
    c.edge_t(c.bounds(), EdgeStyle::WindowRaised, cs, s);
    body(&mut c, paint, text, theme.fonts.size_px, k, lay, None);
    c
}

/// The text and rules; `ink` is the glass panel's, None for the face colours.
fn body(
    c: &mut Canvas,
    paint: &Paint,
    text: &TextRenderer,
    px: u32,
    k: &KeySheet,
    lay: &Layout,
    ink: Option<Color>,
) {
    let (cs, s) = (paint.cs, paint.s);
    let put = |c: &mut Canvas, r: Rect, t: &str, bold: bool| -> i32 {
        let r = r.scaled(s);
        let t = text.ellipsize(t, r.width, bold, px);
        match ink {
            Some(ink) => {
                let base = text.baseline(r, bold, px).unwrap_or(r.bottom());
                text.draw_run_over(c, r.x, base, r, &t, bold, ink, px);
            }
            None => text.draw(c, r, &t, bold, cs.button_text, Align::Left, px),
        }
        text.measure(&t, bold, px)
    };
    put(c, lay.title, TITLE, true);
    for ((g, head), rows) in k.groups.iter().zip(&lay.headings).zip(&lay.rows) {
        let label_w = put(c, *head, &g.title, true);
        let x0 = head.x * s + label_w + RULE_GAP * s;
        let (x1, y) = (head.right() * s, (head.y + head.height / 2) * s);
        if x0 < x1 {
            match ink {
                Some(_) => c.fill_rect(Rect::new(x0, y, x1 - x0, s), panel::DIM_INK),
                None => c.etched_hline_t(x0, x1, y, cs, s),
            }
        }
        for (b, (keys, what)) in g.rows.iter().zip(rows) {
            put(c, *keys, &b.keys, true);
            put(c, *what, &b.what, false);
        }
    }
}

/// The group and row under (`x`, `y`), sheet coordinates.
pub fn hit(lay: &Layout, x: i32, y: i32) -> Option<(usize, usize)> {
    lay.rows.iter().enumerate().find_map(|(g, rows)| {
        let i = rows.iter().position(|(k, w)| k.union(w).contains(x, y))?;
        Some((g, i))
    })
}

/// The sheet's `access` part: a group per heading holding a label per row,
/// "keys: what". `at` is the sheet's rect.
pub fn access(k: &KeySheet, lay: &Layout, at: Rect, tree: &mut Tree, parent: NodeId) -> NodeId {
    let shift = |r: Rect| Rect::new(at.x + r.x, at.y + r.y, r.width, r.height);
    let id = tree.push(parent, "keysheet", access::named(Role::Group, at, TITLE));
    for (gi, ((g, head), rows)) in k
        .groups
        .iter()
        .zip(&lay.headings)
        .zip(&lay.rows)
        .enumerate()
    {
        let span = rows
            .iter()
            .fold(*head, |r, (keys, what)| r.union(keys).union(what));
        let gid = tree.push(id, gi, access::named(Role::Group, shift(span), &g.title));
        for (i, (b, (keys, what))) in g.rows.iter().zip(rows).enumerate() {
            let line = format!("{}: {}", b.keys, b.what);
            tree.push(gid, i, access::label(shift(keys.union(what)), &line, true));
        }
    }
    id
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interface::Interface;
    use crate::interfaces::theme_for;
    use crate::theme::FontConfig;

    fn group(title: &str, n: usize) -> Group {
        Group {
            title: title.into(),
            rows: (0..n)
                .map(|i| Binding {
                    keys: format!("Super+{}", (b'A' + i as u8) as char),
                    what: format!("Action {i}"),
                })
                .collect(),
        }
    }

    pub(crate) fn sample() -> KeySheet {
        KeySheet {
            groups: vec![
                group("Windows", 6),
                group("Desktops", 4),
                group("Launch and search", 3),
                group("Screenshots", 4),
                Group {
                    title: "System".into(),
                    rows: vec![Binding {
                        keys: "Super+L".into(),
                        what: "Lock the screen".into(),
                    }],
                },
            ],
        }
    }

    fn fonts() -> TextRenderer {
        TextRenderer::load(&FontConfig::default())
    }

    #[test]
    fn groups_stack_in_one_column_when_the_room_is_tall_enough() {
        let theme = Theme::default();
        let lay = layout(&sample(), &theme, &fonts(), (1920, 1080));
        assert_eq!(lay.columns, 1);
        let h = 18 * LINE_H + 5 * HEAD_H + 4 * GROUP_GAP;
        assert_eq!(lay.height, PAD + TITLE_H + h + PAD);
        assert_eq!(
            lay.headings[0],
            Rect::new(PAD, PAD + TITLE_H, lay.headings[0].width, HEAD_H)
        );
        let (keys, what) = lay.rows[0][1];
        assert_eq!(keys.y, PAD + TITLE_H + HEAD_H + LINE_H);
        assert_eq!(what.x, keys.right() + KEY_GAP);
        assert_eq!(
            lay.headings[1].y,
            lay.rows[0][5].0.bottom() + GROUP_GAP,
            "the next group a gap under the last row"
        );
    }

    #[test]
    fn a_short_room_takes_more_columns_and_evens_their_heights() {
        let theme = Theme::default();
        let lay = layout(&sample(), &theme, &fonts(), (1920, 500));
        assert_eq!(lay.columns, 2, "the fewest columns within 300 px");
        let col_of = |r: &Rect| r.x;
        let xs: Vec<i32> = lay.headings.iter().map(col_of).collect();
        assert_eq!(xs[0], xs[1]);
        assert!(xs[2] > xs[0], "Launch and search starts the second column");
        assert!(lay.height <= 500 * TALL / 100);
        let first_col_bottom = lay.rows[1].last().unwrap().0.bottom();
        let second_col_bottom = lay.rows[4].last().unwrap().0.bottom();
        assert!((first_col_bottom - second_col_bottom).abs() <= 2 * LINE_H + HEAD_H);
        let wide = lay.headings[2].x - lay.headings[0].x;
        assert_eq!(wide, lay.headings[0].width + COL_GAP);
    }

    #[test]
    fn a_narrow_room_keeps_one_column_and_runs_past_the_height() {
        let theme = Theme::default();
        let lay = layout(&sample(), &theme, &fonts(), (200, 300));
        assert_eq!(lay.columns, 1);
        assert!(lay.height > 300);
    }

    #[test]
    fn a_group_never_breaks_across_columns() {
        let k = KeySheet {
            groups: vec![group("A", 10), group("B", 10)],
        };
        let lay = layout(&k, &Theme::default(), &fonts(), (1920, 150));
        for (head, rows) in lay.headings.iter().zip(&lay.rows) {
            assert!(rows.iter().all(|(k, _)| k.x == head.x));
        }
    }

    #[test]
    fn the_layout_grows_with_the_text_size() {
        let base = Theme::default();
        let big = base.with_text_size(200);
        let small = layout(&sample(), &base, &fonts(), (4000, 4000));
        let lay = layout(&sample(), &big, &fonts(), (4000, 4000));
        assert_eq!(lay.headings[0].y, 2 * small.headings[0].y);
        assert_eq!(lay.rows[0][0].0.height, 2 * LINE_H);
    }

    #[test]
    fn a_row_hits_as_its_group_and_index() {
        let lay = layout(&sample(), &Theme::default(), &fonts(), (1920, 1080));
        let (k, w) = lay.rows[1][2];
        assert_eq!(hit(&lay, k.x + 1, k.y + 1), Some((1, 2)));
        assert_eq!(hit(&lay, w.right() - 1, w.y + 1), Some((1, 2)));
        let h = lay.headings[1];
        assert_eq!(hit(&lay, h.x + 1, h.y + 1), None, "a heading");
    }

    #[test]
    fn the_reader_hears_each_group_and_its_rows() {
        let k = sample();
        let lay = layout(&k, &Theme::default(), &fonts(), (1920, 1080));
        let mut t = Tree::window("", (lay.width, lay.height));
        access(
            &k,
            &lay,
            Rect::new(0, 0, lay.width, lay.height),
            &mut t,
            crate::access::ROOT,
        );
        assert!(t.find(Role::Group, "Desktops").is_some());
        let lock = t
            .nodes()
            .find(|(_, n)| n.value() == Some("Super+L: Lock the screen"));
        assert_eq!(lock.map(|(_, n)| n.role()), Some(Role::Label));
    }

    /// FNV-1a over the pixels, for pins.
    fn hash(c: &Canvas) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for y in 0..c.height as i32 {
            for x in 0..c.width as i32 {
                for b in c.get(x, y).to_le_bytes() {
                    h = (h ^ b as u64).wrapping_mul(0x0100_0000_01b3);
                }
            }
        }
        h
    }

    fn drawn(i: Interface, scale: i32) -> (Canvas, Layout, Theme) {
        drawn_in(theme_for(i), scale)
    }

    fn drawn_in(theme: Theme, scale: i32) -> (Canvas, Layout, Theme) {
        let theme = theme.scaled(scale);
        let text = TextRenderer::load(&theme.fonts);
        let k = sample();
        let lay = layout(&k, &theme, &text, (1920, 500));
        (draw(&theme, &text, &k, &lay), lay, theme)
    }

    #[test]
    fn classic_and_blue_draw_a_raised_face_with_bold_keys_and_etched_rules() {
        for i in [Interface::Classic, Interface::Blue] {
            for scale in [1, 2] {
                let (c, lay, theme) = drawn(i, scale);
                let cs = &theme.colors;
                let (w, h) = (lay.width * scale, lay.height * scale);
                assert_eq!((c.width as i32, c.height as i32), (w, h), "{i:?}@{scale}");
                assert_eq!(c.get(0, 0), cs.button_face.argb());
                assert_eq!(c.get(w - 1, h - 1), cs.button_dk_shadow.argb());
                let head = lay.headings[1];
                let y = (head.y + head.height / 2) * scale;
                let x = (head.right() - 2) * scale;
                assert_eq!(c.get(x, y), cs.button_shadow.argb(), "{i:?}@{scale}: rule");
                assert_eq!(c.get(x, y + scale), cs.button_hilight.argb());
                let (keys, what) = lay.rows[0][0];
                let ink = cs.button_text.argb();
                let inked = |r: Rect| {
                    let r = r.scaled(scale);
                    (r.y..r.bottom()).any(|y| (r.x..r.right()).any(|x| c.get(x, y) == ink))
                };
                assert!(
                    inked(keys) && inked(what) && inked(lay.title),
                    "{i:?}@{scale}"
                );
            }
        }
    }

    #[test]
    fn under_glass_the_sheet_is_glass_with_white_text_and_dim_rules() {
        for scale in [1, 2] {
            let (c, lay, theme) = drawn(Interface::Glass, scale);
            let m = panel::margin(&theme);
            let at = |x: i32, y: i32| c.get((m.left + x) * scale, (m.top + y) * scale);
            assert_eq!(
                (c.width as i32, c.height as i32),
                (
                    (lay.width + m.left + m.right) * scale,
                    (lay.height + m.top + m.bottom) * scale
                )
            );
            let head = lay.headings[1];
            let rule = at(head.right() - 2, head.y + head.height / 2);
            assert_eq!(rule, panel::DIM_INK.argb(), "@{scale}");
            let (keys, _) = lay.rows[0][0];
            let white = (keys.y..keys.bottom())
                .any(|y| (keys.x..keys.right()).any(|x| at(x, y) == 0xFFFF_FFFF));
            assert!(white, "@{scale}: white keys");
        }
    }

    #[test]
    fn the_sheet_pins_in_each_look_at_scale_1_and_2() {
        if !crate::text::test_tahoma() {
            return;
        }
        let mut got = Vec::new();
        for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
            for scale in [1, 2] {
                got.push((format!("{i:?} {scale}"), hash(&drawn(i, scale).0)));
            }
        }
        let g = crate::glass::Settings {
            mode: crate::glass::Mode::Dark,
            ..Default::default()
        };
        let dark = crate::interfaces::theme_with(Interface::Glass, Default::default(), &g);
        for scale in [1, 2] {
            let c = drawn_in(dark.clone(), scale).0;
            got.push((format!("Glass Dark {scale}"), hash(&c)));
        }
        crate::pins::check("keysheet", &got);
    }
}
