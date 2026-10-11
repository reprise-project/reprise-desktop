//! Glass's item boxes and tree expanders.
//!
//! A list, tree or icon view item stands in a light box rounded over 3 px with
//! the text in the window's text colour: a 1 px outline, a 1 px line inside it
//! and a fill, the line and fill shading down the box. Five boxes: under the
//! pointer (B6D1FB round a near-white fill), selected (81A5DC round EAF3FE to
//! CDE1FE), selected and hot or focused (7A9BCD round DBE9FC to BFD7FC), selected
//! without the focus (D8D7D8 round F7F8F8 to E3E4E4), and the focus alone (the
//! 7A9BCD outline round a white line, nothing inside).
//!
//! A selected desktop icon stands in a see-through box (outline at 90%, line at
//! 83%, fill at 75%), so the wallpaper shows through.
//!
//! A tree's expander is a triangle: hollow grey pointing right when collapsed,
//! filled near-black pointing down-right when expanded, either cyan with a soft
//! glow under the pointer.

use super::glyph::{Mark, Piece};
use super::panel::{self, Fade, LightBox};
use crate::raster::{Canvas, Insets};
use crate::shape::{centre, draw, over, premul, scale, smooth};
use crate::style::{Layout, Part, PartImage, State};
use crate::theme::Color;

const fn ink(v: u32, a: u8) -> (Color, u8) {
    (Color::rgb((v >> 16) as u8, (v >> 8) as u8, v as u8), a)
}

const fn opaque(outline: u32, line: (u32, u32), fill: (u32, u32)) -> LightBox {
    LightBox {
        outline: ink(outline, 255),
        line: Some((ink(line.0, 255), ink(line.1, 255))),
        fill: (ink(fill.0, 255), ink(fill.1, 255)),
        radius: 3.0,
        fade: Fade::Ring,
    }
}

const HOT: LightBox = opaque(0xB6D1FB, (0xFCFDFE, 0xF1F6FE), (0xFAFBFD, 0xEAF2FD));
const SELECTED: LightBox = opaque(0x81A5DC, (0xF0F6FE, 0xDDEBFE), (0xEAF3FE, 0xCDE1FE));
const SELECTED_HOT: LightBox = opaque(0x7A9BCD, (0xEAF3FD, 0xDAE7FD), (0xDBE9FC, 0xBFD7FC));
const SELECTED_DIM: LightBox = opaque(0xD8D7D8, (0xFAF9FB, 0xEFEEEF), (0xF7F8F8, 0xE3E4E4));
const FOCUS: LightBox = LightBox {
    outline: ink(0x7A9BCD, 255),
    line: Some((ink(0xFFFFFF, 128), ink(0xFFFFFF, 128))),
    fill: (ink(0xFFFFFF, 0), ink(0xFFFFFF, 0)),
    radius: 3.0,
    fade: Fade::Ring,
};

/// A selected desktop icon: a 739CD8 outline at 90% (81A5DC over
/// white), an EEF6FF line at 83% shading to D8E8FF, and an E5F0FF fill at
/// 75% shading to BED8FF.
const DESKTOP: LightBox = LightBox {
    outline: ink(0x739CD8, 230),
    line: Some((ink(0xEEF6FF, 212), ink(0xD8E8FF, 212))),
    fill: (ink(0xE5F0FF, 191), ink(0xBED8FF, 191)),
    radius: 3.0,
    fade: Fade::Ring,
};

/// The boxes' art: 10 px wide, stretching past the rounded corners; 70 rows,
/// the tallest box (an icon cell), so a shorter one samples the shading down.
const BOX_W: i32 = 10;
const BOX_H: i32 = 70;
const BOX_GRID: Insets = Insets::new(4, 4, 4, 4);

fn boxes(boxes: &[(&LightBox, State)], d: i32) -> PartImage {
    let mut a = Canvas::new((BOX_W * d) as u32, (BOX_H * d * boxes.len() as i32) as u32);
    for (i, (b, _)) in boxes.iter().enumerate() {
        a.blit_over(
            0,
            i as i32 * BOX_H * d,
            &panel::light_box(b, BOX_W, BOX_H, d),
        );
    }
    let states: Vec<State> = boxes.iter().map(|b| b.1).collect();
    PartImage::new(a, &states, Layout::Vertical)
        .at_scale(d)
        .sizing(BOX_GRID)
}

/// The expander cell's size: the triangle's middle sits on its pixel
/// (7, 8) ([`Part::TreeExpander`]).
const EXPANDER: i32 = 16;

/// The list items' boxes and the tree's expanders (remapped in dark
/// Glass with the rest of the control art), painted at `d`.
pub fn parts(d: i32) -> [(Part, PartImage); 2] {
    let items = boxes(
        &[
            (&HOT, State::Hot),
            (&SELECTED, State::Checked),
            (&SELECTED_HOT, State::CheckedHot),
            (&SELECTED_DIM, State::Inactive),
            (&FOCUS, State::Focused),
        ],
        d,
    );
    let mut a = Canvas::new((EXPANDER * d) as u32, (4 * EXPANDER * d) as u32);
    let frames = [
        (&COLLAPSED, COLLAPSED_INK, false),
        (&COLLAPSED, HOT_INK, true),
        (&EXPANDED, EXPANDED_INK, false),
        (&EXPANDED, HOT_INK, true),
    ];
    for (i, &(m, ink, hot)) in frames.iter().enumerate() {
        a.blit_over(0, i as i32 * EXPANDER * d, &expander(m, ink, hot, d));
    }
    let expanders = PartImage::new(
        a,
        &[State::Normal, State::Hot, State::Checked, State::CheckedHot],
        Layout::Vertical,
    )
    .at_scale(d);
    [(Part::ListItem, items), (Part::TreeExpander, expanders)]
}

/// The selected desktop icon's box; the same in light and dark Glass, as it
/// lies over the wallpaper.
pub fn desktop_part(d: i32) -> PartImage {
    boxes(&[(&DESKTOP, State::Checked)], d)
}

/// A collapsed node's triangle: a 1 px outline pointing right, its back
/// down column 5 from row 4 to row 12, its point at (9, 8).
const COLLAPSED: Mark = Mark {
    w: EXPANDER,
    h: EXPANDER,
    pieces: &[Piece::Line(
        &[(5.5, 4.5), (9.5, 8.5), (5.5, 12.5), (5.5, 4.5)],
        1.1,
    )],
};
/// An expanded node's triangle: solid, pointing down to the right, its
/// right angle at the bottom right of (10, 10), its sides 6 px.
const EXPANDED: Mark = Mark {
    w: EXPANDER,
    h: EXPANDER,
    pieces: &[Piece::Poly(&[(11.0, 4.6), (11.0, 11.0), (4.6, 11.0)])],
};
/// The collapsed triangle's grey, the expanded one's near-black, and
/// either's cyan under the pointer.
const COLLAPSED_INK: Color = Color::rgb(0x7C, 0x7D, 0x7D);
const EXPANDED_INK: Color = Color::rgb(0x24, 0x25, 0x25);
const HOT_INK: Color = Color::rgb(0x17, 0xB8, 0xF7);
/// The glow round a triangle under the pointer: its cyan at this alpha
/// beside the triangle, gone this many px out.
const GLOW_PEAK: f64 = 0.16;
const GLOW_REACH: f64 = 4.5;

fn expander(m: &Mark, ink: Color, hot: bool, d: i32) -> Canvas {
    let ink = premul(ink, 0xFF);
    draw(EXPANDER, EXPANDER, d, |x, y| {
        let (x, y) = (x as i32, y as i32);
        let top = scale(ink, m.cover(x, y, d, 0.0));
        if !hot {
            return top;
        }
        let out = m.dist(centre(x, d) + 0.5, centre(y, d) + 0.5).max(0.0);
        over(
            top,
            scale(ink, GLOW_PEAK * (1.0 - smooth(out / GLOW_REACH))),
        )
    })
}

#[cfg(test)]
mod tests {
    use crate::controls::iconview::{self, Selection as Picked};
    use crate::controls::listview::{self, Column, ListView, Row};
    use crate::controls::treeview::{self, Node, Style as TreeStyle, TreeView};
    use crate::interface::Interface;
    use crate::interfaces::theme_for;
    use crate::raster::{Canvas, Rect};
    use crate::text::TextRenderer;
    use crate::theme::{Color, FontConfig, Theme};

    fn rgb(v: u32) -> u32 {
        0xFF00_0000 | v
    }

    fn near(what: &str, got: u32, want: u32, tol: i32) {
        let d = [16, 8, 0]
            .iter()
            .map(|sh| ((got >> sh) & 0xFF) as i32 - ((want >> sh) & 0xFF) as i32)
            .map(i32::abs)
            .max()
            .unwrap_or(0);
        assert!(d <= tol, "{what}: {got:08X}, want {want:06X} within {tol}");
    }

    fn text() -> TextRenderer {
        TextRenderer::load(&FontConfig::default())
    }

    /// A two-column list of six rows, `set` applied, drawn at (10, 10)
    /// 260x150 on white; returns the canvas and the rows' top and the
    /// body's left edge.
    fn list(theme: &Theme, set: impl Fn(&mut ListView)) -> (Canvas, i32, i32) {
        let text = text();
        let mut lv = ListView {
            columns: vec![Column::new("Name", 120), Column::new("Size", 60)],
            rows: (0..6)
                .map(|i| Row {
                    icon: None,
                    cells: vec![format!("Item {i}"), format!("{i} KB")],
                })
                .collect(),
            ..ListView::default()
        };
        set(&mut lv);
        let mut c = Canvas::new(280, 170);
        c.fill_rect(c.bounds(), Color::rgb(0xFF, 0xFF, 0xFF));
        let lay = listview::layout(&lv, Rect::new(10, 10, 260, 150), &theme.look.controls());
        let px = theme.fonts.size_px;
        listview::draw(&mut c, &text, &theme.paint(), &lv, &lay, lv.bars.lit(), px);
        (c, lay.body.y, lay.body.x)
    }

    /// The Folders tree: Desktop open over Documents (holding a folder)
    /// and Music, Music selected; `set` applied, drawn at (10, 10) 180x120.
    fn tree(
        theme: &Theme,
        set: impl Fn(&mut TreeView, &treeview::Layout),
    ) -> (Canvas, treeview::Layout) {
        let text = text();
        let mut desk = Node::new("Desktop", true);
        desk.expanded = true;
        let mut docs = Node::new("Documents", true);
        docs.children.push(Node::new("Letters", false));
        desk.children.push(docs);
        desk.children.push(Node::new("Music", true));
        let mut tv = TreeView::new(vec![desk], TreeStyle::Explorer);
        tv.selected = Some(vec![0, 1]);
        let px = theme.fonts.size_px;
        let lay = treeview::layout(
            &tv,
            &text,
            px,
            Rect::new(10, 10, 180, 120),
            &theme.look.controls(),
        );
        set(&mut tv, &lay);
        let mut c = Canvas::new(200, 140);
        c.fill_rect(c.bounds(), Color::rgb(0xFF, 0xFF, 0xFF));
        treeview::draw(&mut c, &text, &theme.paint(), &tv, &lay, tv.bars.lit(), px);
        (c, lay)
    }

    /// Row `i`'s top in a [`list`] whose rows start at `top`.
    fn row_y(top: i32, i: usize) -> i32 {
        top + i as i32 * listview::ROW_H
    }

    /// Whether row `i`'s name has ink of `v` in it.
    fn inked(c: &Canvas, top: i32, i: usize, left: i32, v: u32) -> bool {
        let y = row_y(top, i);
        (y + 3..y + 14).any(|y| (left + 20..left + 60).any(|x| c.get(x, y) == rgb(v)))
    }

    /// Each state boxes the row from the first column to the body's
    /// right edge, a row past its foot, with its outline along the top,
    /// its line under that and its fill shading down; the names stay
    /// black.
    #[test]
    fn a_list_boxes_each_state_across_the_row() {
        let theme = theme_for(Interface::Glass);
        let (c, top, left) = list(&theme, |lv| {
            lv.selection.set.extend([0, 3]);
            lv.selection.focus = Some(3);
            lv.hot = Some(1);
        });
        let x = left + 150;
        // Row 0: selected, not the lead.
        assert_eq!(c.get(x, row_y(top, 0)), rgb(0x81A5DC), "selected outline");
        near("selected line", c.get(x, row_y(top, 0) + 1), 0xF0F6FE, 2);
        near("selected fill", c.get(x, row_y(top, 0) + 2), 0xEAF3FE, 2);
        near(
            "selected fill foot",
            c.get(x, row_y(top, 0) + 15),
            0xCDE1FE,
            2,
        );
        // Row 1: hot; its outline takes the row it shares with row 0.
        assert_eq!(c.get(x, row_y(top, 1)), rgb(0xB6D1FB), "hot outline");
        near("hot fill", c.get(x, row_y(top, 1) + 2), 0xFAFBFD, 2);
        // Row 3: the selected lead.
        assert_eq!(c.get(x, row_y(top, 3)), rgb(0x7A9BCD), "lead outline");
        near("lead fill", c.get(x, row_y(top, 3) + 2), 0xDBE9FC, 2);
        assert_eq!(
            c.get(x, row_y(top, 4)),
            rgb(0x7A9BCD),
            "a row past its foot"
        );
        // Row 5: nothing.
        assert_eq!(c.get(x, row_y(top, 5) + 2), rgb(0xFFFFFF), "a plain row");
        // The box starts on the first column, its corner rounded.
        assert_eq!(c.get(left, row_y(top, 3) + 8), rgb(0x7A9BCD), "left edge");
        assert_ne!(c.get(left, row_y(top, 3)), rgb(0x7A9BCD), "rounded corner");
        for i in [0, 1, 3] {
            assert!(inked(&c, top, i, left, 0x000000), "row {i}'s name is black");
            assert!(
                !inked(&c, top, i, left, 0xFFFFFF),
                "row {i}'s name is not white"
            );
        }
    }

    /// A list without the focus, or in an inactive window, greys its
    /// selection; the focus alone is the lead's outline round a white
    /// line with the window showing inside.
    #[test]
    fn a_list_without_the_focus_greys_and_the_focus_alone_is_an_outline() {
        let mut theme = theme_for(Interface::Glass);
        let (c, top, left) = list(&theme, |lv| {
            lv.selection.set.insert(0);
            lv.selection.focus = Some(2);
            lv.focus_ring = true;
            lv.inactive = true;
        });
        let x = left + 150;
        assert_eq!(
            c.get(x, row_y(top, 0)),
            rgb(0xD8D7D8),
            "unfocused selection"
        );
        near("unfocused fill", c.get(x, row_y(top, 0) + 2), 0xF7F8F8, 2);
        assert_eq!(c.get(x, row_y(top, 2)), rgb(0x7A9BCD), "focus outline");
        // A face wider than the default UI face reaches the point with the row's size.
        if crate::text::test_tahoma() {
            assert_eq!(c.get(x, row_y(top, 2) + 8), rgb(0xFFFFFF), "nothing inside");
        }
        theme.active = false;
        let (c, top, _) = list(&theme, |lv| {
            lv.selection.set.insert(0);
            lv.selection.focus = Some(0);
        });
        assert_eq!(c.get(x, row_y(top, 0)), rgb(0xD8D7D8), "inactive window");
    }

    /// The tree boxes its rows across the body: the selection in the
    /// lighter blue, the darker one under the pointer, grey without the
    /// focus. Its expanders are hidden while the tree lacks both the
    /// focus and the pointer and show as grey triangles otherwise, the
    /// one under the pointer cyan; no Classic box is drawn.
    #[test]
    fn the_tree_boxes_rows_and_shows_triangles_while_the_pointer_is_in() {
        let theme = theme_for(Interface::Glass);
        let row = |lay: &treeview::Layout, i: i32| lay.body.y + i * 17;
        let tri = |c: &Canvas, lay: &treeview::Layout, i: i32| {
            (row(lay, i)..row(lay, i) + 17)
                .flat_map(|y| (lay.body.x..lay.body.x + 30).map(move |x| (x, y)))
                .map(|(x, y)| c.get(x, y))
                .collect::<Vec<u32>>()
        };
        let grey = |v: u32| {
            let (r, g, b) = ((v >> 16) & 0xFF, (v >> 8) & 0xFF, v & 0xFF);
            r == g && g == b && r < 0xC0
        };
        let (c, lay) = tree(&theme, |tv, _| tv.inactive = true);
        assert_eq!(
            c.get(lay.body.x + 120, row(&lay, 2)),
            rgb(0xD8D7D8),
            "unfocused"
        );
        assert!(!tri(&c, &lay, 1).iter().any(|&v| grey(v)), "no expander");
        let (c, lay) = tree(&theme, |_, _| {});
        assert_eq!(
            c.get(lay.body.x + 120, row(&lay, 2)),
            rgb(0x81A5DC),
            "selection"
        );
        assert!(
            tri(&c, &lay, 1).iter().any(|&v| grey(v)),
            "the focus shows them"
        );
        let (c, lay) = tree(&theme, |tv, lay| {
            tv.inactive = true;
            tv.track(lay, lay.body.x + 120, lay.body.y + 2 * 17 + 8);
        });
        assert_eq!(
            c.get(lay.body.x + 120, row(&lay, 2)),
            rgb(0x7A9BCD),
            "selected, hot"
        );
        assert!(
            tri(&c, &lay, 1).iter().any(|&v| grey(v)),
            "Documents' triangle"
        );
        let (c, lay) = tree(&theme, |tv, lay| {
            let mid = lay.body.y + 17 + 8;
            let x = (lay.body.x..lay.body.x + 40)
                .find(|&x| tv.hit(lay, x, mid) == treeview::Hit::Expander(1))
                .expect("Documents has an expander");
            tv.track(lay, x, mid);
        });
        assert_eq!(
            c.get(lay.body.x + 120, row(&lay, 1)),
            rgb(0xB6D1FB),
            "hot row"
        );
        let cyan = tri(&c, &lay, 1).iter().any(|&v| {
            let (r, g, b) = ((v >> 16) & 0xFF, (v >> 8) & 0xFF, v & 0xFF);
            r < 0x60 && g > 0xB0 && b > 0xE0
        });
        assert!(cyan, "the hot triangle is cyan");
    }

    /// The shell view boxes an icon cell from its top to under the label
    /// and a List entry's icon and name; the names stay black.
    #[test]
    fn the_shell_view_boxes_icon_cells_and_list_entries() {
        use crate::controls::shellview::{self, Mode};
        let theme = theme_for(Interface::Glass);
        let text = text();
        let px = theme.fonts.size_px;
        for mode in [Mode::Icons, Mode::List] {
            // Entry 2 is the selected lead; 3 goes hot.
            let mut sv = shellview::sample(mode, false);
            sv.hot = Some(3);
            let mut c = Canvas::new(460, 220);
            let lay = shellview::layout(&sv, Rect::new(5, 5, 450, 210), &theme.look.controls());
            shellview::draw(&mut c, &text, &theme.paint(), &sv, &lay, sv.bars.lit(), px);
            for (i, outline) in [(2, 0x7A9BCD), (3, 0xB6D1FB)] {
                let cell = lay.cells[i];
                // Past a List entry's icon, left of an icon cell's.
                let x = cell.x + 18 - if mode == Mode::Icons { 8 } else { 0 };
                assert_eq!(c.get(x, cell.y), rgb(outline), "{mode:?} {i}: the outline");
                let black = (cell.y..cell.bottom())
                    .any(|y| (cell.x..cell.right()).any(|x| c.get(x, y) == rgb(0)));
                assert!(black, "{mode:?} {i}: the name is black");
            }
        }
    }

    /// A selected desktop icon stands in a see-through box over the
    /// wallpaper, from the cell's top to under its label.
    #[test]
    fn a_selected_desktop_icon_stands_in_a_see_through_box() {
        let theme = theme_for(Interface::Glass);
        let text = text();
        let cell = |bg: Color, picked: bool| {
            let mut d = Canvas::new(80, 80);
            d.fill_rect(d.bounds(), bg);
            let pick = picked.then_some(Picked {
                ring: false,
                origin: (0, 0),
            });
            iconview::draw_cell_at(
                &mut d,
                (0, 0),
                &theme,
                &text,
                "Computer",
                None,
                false,
                pick,
                None,
            );
            d
        };
        let black = cell(Color::rgb(0, 0, 0), true);
        let white = cell(Color::rgb(0xFF, 0xFF, 0xFF), true);
        near("outline over black", black.get(37, 0), 0x678CC3, 2);
        near("outline over white", white.get(37, 0), 0x81A5DC, 2);
        near("fill over black", black.get(3, 3), 0xAAB2BD, 3);
        near("fill over white", white.get(3, 3), 0xEBF4FF, 3);
        assert_eq!(black.get(37, 79), rgb(0), "the box ends under the label");
    }

    /// FNV-1a of a canvas's size and pixels.
    fn fnv(c: &Canvas) -> u64 {
        let words = [c.width, c.height]
            .into_iter()
            .chain((0..c.height as i32).flat_map(|y| {
                (0..c.width as i32)
                    .map(move |x| (x, y))
                    .map(|(x, y)| c.get(x, y))
            }));
        words.fold(0xcbf2_9ce4_8422_2325, |h, w| {
            w.to_le_bytes()
                .iter()
                .fold(h, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
        })
    }

    /// The expanders are drawn at each scale: 16 px cells, 32 at 2; the
    /// four states distinct; the collapsed outline grey with whole pixels
    /// down its back, the expanded triangle near-black, the hot ones cyan
    /// in a faint glow; soft along their slants at 2.
    #[test]
    fn the_expanders_are_drawn_at_each_scale() {
        use crate::style::{Part, State};
        let states = [State::Normal, State::Hot, State::Checked, State::CheckedHot];
        for d in [1, 2] {
            let [_, (p, img)] = super::parts(d);
            assert_eq!(p, Part::TreeExpander);
            assert_eq!(img.scale, d);
            let frames: Vec<Canvas> = states
                .iter()
                .map(|&s| img.atlas.copy_rect(img.frame(s)))
                .collect();
            for f in &frames {
                assert_eq!((f.width, f.height), (16 * d as u32, 16 * d as u32));
            }
            for i in 0..4 {
                for j in i + 1..4 {
                    assert_ne!(
                        fnv(&frames[i]),
                        fnv(&frames[j]),
                        "{:?} vs {:?}",
                        states[i],
                        states[j]
                    );
                }
            }
            let (x, y) = (5 * d, 8 * d);
            assert_eq!(frames[0].get(x, y), rgb(0x7C7D7D), "the collapsed back");
            assert_eq!(frames[0].get(7 * d, 8 * d), 0, "hollow");
            assert_eq!(
                frames[2].get(10 * d, 8 * d),
                rgb(0x242525),
                "the expanded side"
            );
            assert_eq!(frames[1].get(x, y), rgb(0x17B8F7), "hot cyan");
            let glow = frames[1].get(2 * d, 8 * d) >> 24;
            assert!(glow > 0 && glow < 0x40, "the glow beside it: {glow:#x}");
            assert_eq!(frames[1].get(0, 0), 0, "the glow fades out");
            if d == 2 {
                for f in [&frames[0], &frames[2]] {
                    let soft = (0..32)
                        .flat_map(|y| (0..32).map(move |x| (x, y)))
                        .any(|(x, y)| (1..0xFF).contains(&(f.get(x, y) >> 24)));
                    assert!(soft, "soft slants at 2");
                }
            }
        }
    }

    /// The expanders keep their pixels at scale 1 and 2.
    #[test]
    fn expanders_pin_at_scale_1_and_2() {
        let got: Vec<u64> = [1, 2].map(|d| fnv(&super::parts(d)[1].1.atlas)).to_vec();
        assert_eq!(
            got,
            [0xa440_b690_e410_5295, 0x8808_50c4_7a5f_329d],
            "{got:#x?}"
        );
    }

    /// Writes target/glass-items-preview.png: lists in each state, the
    /// tree with the pointer in it and over Documents' expander, and a
    /// selected desktop icon over black and over white.
    #[test]
    #[ignore]
    fn render_glass_items_preview() {
        let theme = theme_for(Interface::Glass);
        let mut out = Canvas::new(600, 340);
        out.fill_rect(out.bounds(), Color::rgb(0xEF, 0xEE, 0xEF));
        let (a, _, _) = list(&theme, |lv| {
            lv.selection.set.extend([0, 2, 3]);
            lv.selection.focus = Some(3);
            lv.hot = Some(1);
        });
        out.blit_over(0, 0, &a);
        let (b, _, _) = list(&theme, |lv| {
            lv.selection.set.insert(0);
            lv.selection.focus = Some(2);
            lv.focus_ring = true;
            lv.inactive = true;
            lv.hot = Some(4);
        });
        out.blit_over(290, 0, &b);
        let text = text();
        let (t, _) = tree(&theme, |tv, lay| {
            tv.track(lay, 20, 10 + 17 + 8);
        });
        out.blit_over(0, 175, &t);
        for (i, bg) in [0x000000u32, 0xFFFFFF].iter().enumerate() {
            let mut d = Canvas::new(90, 90);
            d.fill_rect(
                d.bounds(),
                Color::rgb((bg >> 16) as u8, (bg >> 8) as u8, *bg as u8),
            );
            let pick = Picked {
                ring: false,
                origin: (0, 0),
            };
            iconview::draw_cell_at(
                &mut d,
                (5, 5),
                &theme,
                &text,
                "Computer",
                None,
                false,
                Some(pick),
                None,
            );
            out.blit_over(220 + i as i32 * 100, 200, &d);
        }
        crate::raster::write_preview(&out, "glass-items-preview.png");
    }
}
