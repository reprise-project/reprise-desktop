use super::*;
use crate::look::classic_paint;
use crate::theme::{FontConfig, Theme};

/// The header strip's top row, Name column's left edge, title ink start and sort arrow box.
const HEADER_Y: i32 = 83;
const COL0_X: i32 = 204;
const TITLE_X: i32 = 210;
const ARROW_BOX: (i32, i32, i32, i32) = (246, 89, 7, 4);
const HSCROLL: (i32, i32) = (558, 573);
/// The rename box, text origin inside it and the label it replaced.
const EDIT_BOX: (i32, i32, i32, i32) = (265, 217, 146, 17);
const EDIT_TEXT_X: i32 = 270;
const EDIT_LABEL_X: i32 = 268;
/// The group rule: left end at the body's edge, and 150 px along it.
const LINE_LEFT: Color = Color::rgb(166, 202, 240);
const LINE_MID: Color = Color::rgb(210, 228, 247);

fn view(n: usize) -> ListView {
    ListView {
        columns: vec![Column::new("Name", 120), Column::new("Size", 60)],
        rows: (0..n)
            .map(|i| Row {
                icon: None,
                cells: vec![format!("file{i}.txt"), format!("{i} KB")],
            })
            .collect(),
        ..Default::default()
    }
}

#[test]
fn a_second_press_on_a_row_in_time_and_place_activates_it() {
    let mut lv = view(3);
    let lay = layout(&lv, Rect::new(0, 0, 200, 100), &ControlMetrics::CLASSIC);
    let (x, y) = (lay.body.x + 10, lay.body.y + 3);
    let press = |lv: &mut ListView, x: i32, y: i32, time: u32| {
        let at = Click {
            x,
            y,
            time,
            double_ms: 500,
        };
        lv.activated(lv.hit(&lay, x, y), at)
    };
    assert_eq!(press(&mut lv, x, y, 1000), None, "a first press");
    assert_eq!(press(&mut lv, x + 4, y + 4, 1499), Some(0), "the same row");
    assert_eq!(press(&mut lv, x + 4, y + 4, 1550), None, "a third press");

    press(&mut lv, x, y, 3000);
    assert_eq!(press(&mut lv, x, y + ROW_H, 3100), None, "another row");
    press(&mut lv, x, y, 5000);
    assert_eq!(press(&mut lv, x, y, 5500), None, "too slow");
    press(&mut lv, x, y, 7000);
    assert_eq!(press(&mut lv, x + 5, y, 7100), None, "too far");
    press(&mut lv, x, y, 9000);
    press(&mut lv, x, lay.body.bottom() - 2, 9100);
    assert_eq!(press(&mut lv, x, y, 9200), None, "the background between");
}

/// The vertical bar answers page and thumb hits; `top_at` maps a dragged thumb back to a row.
#[test]
fn the_track_pages_and_the_thumb_drags() {
    let mut lv = view(40);
    let r = Rect::new(0, 0, 200, 100);
    let lay = layout(&lv, r, &ControlMetrics::CLASSIC);
    let sb = lay.scrollbar.expect("a vertical bar");
    assert_eq!(
        lv.hit(&lay, sb.x + 2, sb.bottom() - 3),
        Hit::VBar(BarHit::Forward)
    );
    assert_eq!(
        lv.hit(
            &lay,
            sb.x + 2,
            sb.bottom() - ControlMetrics::CLASSIC.scroll - 2
        ),
        Hit::VBar(BarHit::PageForward)
    );
    let p = scrollbar_parts(sb, 1, lv.top, lay.rows, lv.slots(), false, lay.thumb_min);
    let thumb = p.thumb.expect("a thumb");
    let Hit::VBar(BarHit::Thumb(grab)) = lv.hit(&lay, thumb.x + 2, thumb.y + 3) else {
        panic!("the thumb");
    };
    assert_eq!(grab, 3);
    assert_eq!(lv.top_at(&lay, thumb.y + 3, grab), 0, "not moved yet");
    let bottom = lv.top_at(&lay, sb.bottom(), grab);
    assert_eq!(bottom, lv.max_top(&lay), "dragged past the end clamps");
    lv.top = bottom;
    let lay = layout(&lv, r, &ControlMetrics::CLASSIC);
    assert_eq!(
        lv.hit(&lay, sb.x + 2, sb.y + ControlMetrics::CLASSIC.scroll + 2),
        Hit::VBar(BarHit::PageBack)
    );
}

/// With groups the bar counts slots, headers included; `top_at` stays within `max_top`.
#[test]
fn a_grouped_thumb_drags_to_rows_within_max_top() {
    let mut lv = view(20);
    lv.groups = vec![
        Group {
            title: "A".into(),
            first: 0,
        },
        Group {
            title: "B".into(),
            first: 10,
        },
    ];
    let r = Rect::new(0, 0, 200, 4 + HEADER_H + 6 * ROW_H);
    let lay = layout(&lv, r, &ControlMetrics::CLASSIC);
    let sb = lay.scrollbar.expect("a vertical bar");
    let bottom = lv.top_at(&lay, sb.bottom(), 0);
    assert_eq!(bottom, lv.max_top(&lay), "dragged past the end clamps");
    lv.top = 12;
    let lay = layout(&lv, r, &ControlMetrics::CLASSIC);
    let p = scrollbar_parts(
        sb,
        1,
        lv.top_slot(),
        lay.rows,
        lv.slots(),
        false,
        lay.thumb_min,
    );
    let thumb = p.thumb.expect("a thumb");
    assert_eq!(lv.top_at(&lay, thumb.y, 0), 12, "the thumb stays put");
}

/// `follow` scrolls a grouped list just far enough that the focused row and its header fit.
#[test]
fn follow_brings_a_grouped_row_just_into_view() {
    let mut lv = view(20);
    lv.groups = vec![
        Group {
            title: "A".into(),
            first: 0,
        },
        Group {
            title: "B".into(),
            first: 10,
        },
    ];
    let r = Rect::new(0, 0, 200, 4 + HEADER_H + 6 * ROW_H);
    let lay = layout(&lv, r, &ControlMetrics::CLASSIC);
    lv.select(Some(12), &lay);
    assert_eq!(lv.top, 9);
    lv.select(Some(19), &lay);
    assert_eq!(lv.top, 14, "the last six rows");
    lv.select(Some(11), &lay);
    assert_eq!(lv.top, 11, "up to the row itself");
    let short = layout(
        &lv,
        Rect::new(0, 0, 200, 4 + HEADER_H + ROW_H - 1),
        &ControlMetrics::CLASSIC,
    );
    lv.top = 0;
    lv.select(Some(10), &short);
    assert_eq!(lv.top, 10);
}

#[test]
fn a_reader_scrolls_a_row_out_of_view_in_without_picking_it() {
    use crate::access::{Published, ROOT, Tree};
    let mut lv = view(20);
    lv.selection.focus = Some(0);
    lv.top = 4;
    let r = Rect::new(0, 0, 200, 4 + HEADER_H + 6 * ROW_H);
    let lay = layout(&lv, r, &ControlMetrics::CLASSIC);
    let mut tree = Tree::window("", (200, 200));
    lv.access(&lay, &mut tree, ROOT, "list");
    let row = |i| {
        tree.find(Role::ListBoxOption, &format!("file{i}.txt"))
            .unwrap()
    };
    let (above, shown, below) = (row(3), row(4), row(15));
    let mut p = Published::default();
    p.update(tree);
    assert_eq!(p.reveal(shown), None, "the top row shows");
    assert_eq!(
        p.reveal(above).map(|r| r.row),
        Some(3),
        "the row under the header"
    );
    assert_eq!(
        p.target(above),
        None,
        "a click there would land on the header"
    );
    assert_eq!(p.reveal(below).map(|r| r.row), Some(15));
    assert!(lv.reveal(15, &lay));
    assert_eq!(lv.top, 10, "row 15 is the last row shown");
    assert!(lv.reveal(3, &lay));
    assert_eq!(lv.top, 3);
    assert!(!lv.reveal(5, &lay), "a row in view stays put");
    assert_eq!(lv.selection.focus, Some(0), "the pick stays");
}

fn render(lv: &ListView, r: Rect) -> (Canvas, Layout, Theme) {
    let text = TextRenderer::load(&FontConfig::default());
    let theme = Theme::default();
    let lay = layout(lv, r, &ControlMetrics::CLASSIC);
    let mut c = Canvas::new(r.width as u32, r.height as u32);
    draw(
        &mut c,
        &text,
        &classic_paint(&theme.colors, 1),
        lv,
        &lay,
        None,
        theme.fonts.size_px,
    );
    (c, lay, theme)
}

/// The sorted column's band is the window colour 8/255 toward the window text.
#[test]
fn the_sort_band_tints_the_schemes_window() {
    let text = TextRenderer::load(&FontConfig::default());
    let mut lv = view(2);
    lv.sort = Some((1, true));
    lv.sort_tint = true;
    let r = Rect::new(0, 0, 300, 100);
    let lay = layout(&lv, r, &ControlMetrics::CLASSIC);
    let mut dark = Theme::default().colors;
    dark.window = Color::rgb(32, 32, 32);
    dark.window_text = Color::rgb(255, 255, 255);
    for (cs, band) in [
        (Theme::default().colors, Color::rgb(247, 247, 247)),
        (dark, Color::rgb(39, 39, 39)),
    ] {
        let mut c = Canvas::new(300, 100);
        draw(&mut c, &text, &classic_paint(&cs, 1), &lv, &lay, None, 11);
        let y = lay.body.bottom() - 2;
        assert_eq!(c.get(150, y), band.argb(), "the Size column's band");
        assert_eq!(c.get(60, y), cs.window.argb(), "the Name column");
    }
}

/// A group's rule fades into the window colour, so on a dark window its far end is dark.
#[test]
fn the_group_rule_fades_into_a_dark_window() {
    let text = TextRenderer::load(&FontConfig::default());
    let mut lv = view(2);
    lv.groups = vec![Group {
        title: "A".into(),
        first: 0,
    }];
    let r = Rect::new(0, 0, 400, 100);
    let lay = layout(&lv, r, &ControlMetrics::CLASSIC);
    let mut cs = Theme::default().colors;
    cs.window = Color::rgb(32, 32, 32);
    cs.window_text = Color::rgb(255, 255, 255);
    let mut c = Canvas::new(400, 100);
    draw(&mut c, &text, &classic_paint(&cs, 1), &lv, &lay, None, 11);
    let (x, y) = (lay.body.x + GROUP_LINE_W - 1, lay.body.y + GROUP_RULE_Y);
    assert_eq!(
        c.get(x, y),
        Color::rgb(33, 33, 33).argb(),
        "the rule's last px"
    );
}

#[test]
fn layout_hits_and_scrolls() {
    let lv = view(3);
    let r = Rect::new(0, 0, 300, 100);
    let lay = layout(&lv, r, &ControlMetrics::CLASSIC);
    assert_eq!(lay.header, Rect::new(2, 2, 296, HEADER_H));
    assert_eq!(lay.body.y, 2 + HEADER_H);
    assert!(lay.scrollbar.is_none());
    assert!(lay.hscroll.is_none());
    assert_eq!(lay.total_w, 180);
    assert_eq!(lv.hit(&lay, 10, 5), Hit::Header(0));
    assert_eq!(lv.hit(&lay, 130, 5), Hit::Header(1));
    assert_eq!(lv.hit(&lay, 10, lay.body.y + ROW_H + 3), Hit::Row(1));
    assert_eq!(
        lv.hit(&lay, 10, lay.body.y + 3 * ROW_H + 3),
        Hit::Background
    );
    assert_eq!(lv.hit(&lay, 1, 1), Hit::Nothing);
    let mut many = view(20);
    let lay = layout(&many, r, &ControlMetrics::CLASSIC);
    assert!(lay.scrollbar.is_some());
    assert_eq!(lay.rows, (96 - HEADER_H) as usize / ROW_H as usize);
    assert!(many.step(1, &lay, Mods::default()));
    assert_eq!(many.selection.focus, Some(0));
    many.step(100, &lay, Mods::default());
    assert_eq!(many.selection.focus, Some(19));
    assert_eq!(many.top, 20 - lay.rows);
    many.step(-100, &lay, Mods::default());
    assert_eq!((many.selection.focus, many.top), (Some(0), 0));
    assert!(many.scroll(2, &lay));
    assert_eq!(
        lv.hit(&lay, lay.scrollbar.unwrap().x + 3, lay.body.y + 3),
        Hit::VBar(BarHit::Back)
    );
}

/// Scrolled columns, highlight, rename box and check box stay inside the rows' box and header strip.
#[test]
fn scrolled_contents_stay_inside_the_body_and_header() {
    let text = TextRenderer::load(&FontConfig::default());
    let theme = Theme::default();
    let mut lv = view(20);
    lv.resize_column(0, 400);
    lv.checks = Some(vec![true; 20]);
    lv.select(
        Some(3),
        &layout(&lv, Rect::new(0, 0, 1, 1), &ControlMetrics::CLASSIC),
    );
    lv.edit = Some((4, TextEdit::new("a long name that runs on", true)));
    let r = Rect::new(20, 30, 200, 100);
    let lay = layout(&lv, r, &ControlMetrics::CLASSIC);
    lv.hscroll(40, &lay);
    let sentinel = 0xFF12_3456;
    let mut c = Canvas::new(260, 160);
    c.pixels.fill(sentinel);
    let mut bare = c.clone();
    draw(
        &mut c,
        &text,
        &classic_paint(&theme.colors, 1),
        &lv,
        &lay,
        None,
        theme.fonts.size_px,
    );
    let empty = ListView {
        columns: Vec::new(),
        ..view(0)
    };
    let bare_lay = Layout {
        total_w: lay.total_w,
        ..layout(&empty, r, &ControlMetrics::CLASSIC)
    };
    draw(
        &mut bare,
        &text,
        &classic_paint(&theme.colors, 1),
        &empty,
        &bare_lay,
        None,
        theme.fonts.size_px,
    );
    let inside = |x, y| lay.header.contains(x, y) || lay.body.contains(x, y);
    for y in 0..160 {
        for x in 0..260 {
            if !inside(x, y) && !r.contains(x, y) {
                assert_eq!(c.get(x, y), sentinel, "outside the rect at ({x}, {y})");
            }
        }
    }
    for y in r.y..r.y + 2 {
        for x in r.x..r.right() {
            assert_eq!(c.get(x, y), bare.get(x, y), "frame at ({x}, {y})");
        }
    }
    for y in r.y..r.bottom() {
        for x in [r.x, r.x + 1, r.right() - 2, r.right() - 1] {
            assert_eq!(c.get(x, y), bare.get(x, y), "frame at ({x}, {y})");
        }
    }
}

#[test]
fn shift_arrow_extends_the_selection() {
    let mut lv = view(6);
    let lay = layout(&lv, Rect::new(0, 0, 300, 200), &ControlMetrics::CLASSIC);
    lv.select(Some(1), &lay);
    let shift = Mods {
        ctrl: false,
        shift: true,
    };
    lv.step(2, &lay, shift);
    assert_eq!(
        lv.selection.set.iter().copied().collect::<Vec<_>>(),
        [1, 2, 3]
    );
    assert_eq!(lv.selection.focus, Some(3));
}

#[test]
fn the_divider_wins_either_side_of_a_column_edge() {
    let lv = view(3);
    let lay = layout(&lv, Rect::new(0, 0, 300, 100), &ControlMetrics::CLASSIC);
    let edge = lay.header.x + lv.columns[0].width;
    for dx in -DIVIDER_GRAB..=DIVIDER_GRAB {
        assert_eq!(lv.hit(&lay, edge + dx, 5), Hit::Divider(0), "dx {dx}");
    }
    assert_eq!(lv.hit(&lay, edge - DIVIDER_GRAB - 1, 5), Hit::Header(0));
    assert_eq!(lv.hit(&lay, edge + DIVIDER_GRAB + 1, 5), Hit::Header(1));
}

#[test]
fn columns_lay_out_grown_by_the_text_size() {
    let lv = view(3);
    let r = Rect::new(0, 0, 600, 200);
    for (pct, k) in [(100, 1), (200, 2)] {
        let cm = Theme::default().with_text_size(pct).controls();
        let lay = layout(&lv, r, &cm);
        assert_eq!(lay.total_w, k * 180, "{pct}%: the columns' width");
        let edge = lay.header.x + k * 120;
        assert_eq!(lv.hit(&lay, edge, 5), Hit::Divider(0), "{pct}%");
        assert_eq!(lv.hit(&lay, edge + k * 60, 5), Hit::Divider(1), "{pct}%");
    }
    assert_eq!(lv.columns[0].width, 120, "laying out keeps the 100% width");
}

#[test]
fn a_filled_column_takes_what_the_grown_columns_leave() {
    let mut lv = view(3);
    lv.fill_column(1, 300, TextUnits::ONE);
    assert_eq!(lv.columns[1].width, 180);
    let cm = Theme::default().with_text_size(200).controls();
    lv.fill_column(1, 301, cm.text);
    assert_eq!(lv.columns[1].width, 30, "half of 301 - 240, rounded down");
    assert_eq!(lv.columns[0].width, 120, "the other columns keep theirs");
    let lay = layout(&lv, Rect::new(0, 0, 305, 200), &cm);
    assert_eq!((lay.body.width, lay.total_w), (301, 300));
    assert!(lay.hscroll.is_none());
    lv.fill_column(1, 100, cm.text);
    assert_eq!(lv.columns[1].width, MIN_COL_W);
}

#[test]
fn a_column_filled_beside_the_bar_leaves_a_scrolled_list_one_bar() {
    let r = Rect::new(0, 0, 304, 100);
    for (pct, scroll) in [(100, 16), (100, 17), (150, 16), (200, 17)] {
        let cm = ControlMetrics {
            scroll,
            ..Theme::default().with_text_size(pct).controls()
        };
        let mut lv = view(40);
        lv.fill_beside_bar(1, r, &cm);
        let lay = layout(&lv, r, &cm);
        assert!(lay.scrollbar.is_some(), "{pct}% {scroll}");
        assert!(
            lay.hscroll.is_none(),
            "no horizontal bar at {pct}% {scroll}"
        );
        assert!(
            lay.body.width - lay.total_w <= 2,
            "filled to the bar at {pct}% {scroll}"
        );
    }
    let mut lv = view(1);
    lv.fill_beside_bar(1, r, &ControlMetrics::CLASSIC);
    assert_eq!(lv.columns[1].width, 300 - 16 - 120);
}

/// A look with a 17 px scrollbar lays both bars 17 px thick; the body gives up 17 px to each.
#[test]
fn a_17_px_scrollbar_takes_17_px_off_the_body_each_way() {
    let mut lv = view(20);
    lv.resize_column(0, 300);
    let r = Rect::new(0, 0, 300, 100);
    let cm = ControlMetrics {
        scroll: 17,
        ..ControlMetrics::CLASSIC
    };
    let lay = layout(&lv, r, &cm);
    let vb = lay.scrollbar.expect("20 rows overflow the body");
    let hb = lay.hscroll.expect("the columns overflow the body");
    assert_eq!(vb, Rect::new(281, 2 + HEADER_H, 17, lay.body.height));
    assert_eq!(hb, Rect::new(2, lay.body.bottom(), 279, 17));
    assert_eq!(vb.right(), r.right() - 2);
    assert_eq!(hb.bottom(), r.bottom() - 2);
}

/// A Blue list lays its bar 17 px thick with a 10 px thumb at least, draws the thumb on the rows
/// `hit` answers Thumb on, and pages either side. Classic's thumb stays 8 px.
#[test]
fn a_blue_bar_is_drawn_where_it_is_hit() {
    let theme = crate::interfaces::theme_for(crate::interface::Interface::Blue);
    let cm = theme.look.controls();
    let text = TextRenderer::load(&FontConfig::default());
    let mut lv = view(500);
    lv.top = 250;
    let r = Rect::new(0, 0, 200, 150);
    let lay = layout(&lv, r, &cm);
    let sb = lay.scrollbar.expect("500 rows overflow the body");
    assert_eq!(sb.width, 17);
    let p = scrollbar_parts(sb, 1, lv.top_slot(), lay.rows, lv.slots(), false, 10);
    let thumb = p.thumb.expect("a thumb");
    assert_eq!(thumb.height, 10, "never shorter than 10 px");
    let x = sb.x + sb.width / 2;
    assert_eq!(lv.hit(&lay, x, thumb.y), Hit::VBar(BarHit::Thumb(0)));
    assert_eq!(
        lv.hit(&lay, x, thumb.bottom() - 1),
        Hit::VBar(BarHit::Thumb(9))
    );
    assert_eq!(lv.hit(&lay, x, thumb.y - 1), Hit::VBar(BarHit::PageBack));
    assert_eq!(
        lv.hit(&lay, x, thumb.bottom()),
        Hit::VBar(BarHit::PageForward)
    );
    let mut c = Canvas::new(r.width as u32, r.height as u32);
    draw(&mut c, &text, &theme.paint(), &lv, &lay, None, 11);
    let track = c.get(x, p.track.y);
    let drawn: Vec<i32> = (p.track.y..p.track.bottom())
        .filter(|&y| c.get(x, y) != track)
        .collect();
    assert_eq!(drawn.first(), Some(&thumb.y));
    assert_eq!(drawn.last(), Some(&(thumb.bottom() - 1)));

    let classic = layout(&lv, r, &ControlMetrics::CLASSIC);
    let sb = classic.scrollbar.expect("500 rows overflow the body");
    let p = scrollbar_parts(sb, 1, lv.top_slot(), classic.rows, lv.slots(), false, 8);
    assert_eq!(p.thumb.map(|t| t.height), Some(8));
    assert_eq!(classic.thumb_min, 8);
}

/// After scrolling to the far end, a widening list or narrowing column draws and hits as if
/// scrolled to its new far end (0 once the columns fit): no blank past the last column.
#[test]
fn a_wider_list_holds_its_sideways_scroll_to_the_columns() {
    let text = TextRenderer::load(&FontConfig::default());
    let cs = Theme::default().colors;
    let mut lv = view(3);
    lv.resize_column(0, 300);
    let lay = |r| layout(&lv, r, &ControlMetrics::CLASSIC);
    let narrow = lay(Rect::new(0, 0, 200, 100));
    let mut scrolled = lv.clone();
    scrolled.hscroll(1000, &narrow);
    let far = scrolled.left;
    assert_eq!(far, narrow.total_w - narrow.body.width);
    let shot = |lv: &ListView, l: &Layout| {
        let mut c = Canvas::new(500, 100);
        draw(&mut c, &text, &classic_paint(&cs, 1), lv, l, None, 11);
        c
    };

    let wider = lay(Rect::new(0, 0, 300, 100));
    let end = wider.total_w - wider.body.width;
    assert!((1..far).contains(&end), "still overflowing, by less");
    assert_eq!(scrolled.left_in(&wider), end);
    let held = ListView {
        left: end,
        ..scrolled.clone()
    };
    assert!(
        shot(&scrolled, &wider) == shot(&held, &wider),
        "drawn at the new far end"
    );
    let (x, y) = (wider.header.x + 5, wider.header.y + 5);
    assert_eq!(scrolled.hit(&wider, x, y), held.hit(&wider, x, y));
    assert!(!scrolled.hscroll(1, &wider), "already at the far end");

    scrolled.left = far;
    let roomy = lay(Rect::new(0, 0, 500, 100));
    assert!(roomy.hscroll.is_none());
    assert_eq!(scrolled.left_in(&roomy), 0);
    assert_eq!(
        scrolled.hit(&roomy, roomy.header.x + 5, roomy.header.y + 5),
        Hit::Header(0),
        "the Name column is back in reach"
    );
    assert!(shot(&scrolled, &roomy) == shot(&lv, &roomy));
}

#[test]
fn resizing_a_column_grows_the_total_and_brings_the_hscroll() {
    let mut lv = view(3);
    let r = Rect::new(0, 0, 300, 100);
    assert!(layout(&lv, r, &ControlMetrics::CLASSIC).hscroll.is_none());
    lv.resize_column(0, 300);
    assert_eq!(lv.columns[0].width, 300);
    let lay = layout(&lv, r, &ControlMetrics::CLASSIC);
    assert_eq!(lay.total_w, 360);
    let hb = lay.hscroll.expect("the columns overflow the body");
    assert_eq!(hb.height, HSCROLL.1 - HSCROLL.0 + 1);
    assert_eq!(
        hb,
        Rect::new(
            lay.body.x,
            lay.body.bottom(),
            lay.body.width,
            ControlMetrics::CLASSIC.scroll
        )
    );
    assert_eq!(hb.bottom(), lay.frame.bottom() - 2);
    assert_eq!(lv.hit(&lay, hb.x + 3, hb.y + 3), Hit::HBar(BarHit::Back));
    assert_eq!(
        lv.hit(&lay, hb.right() - 3, hb.y + 3),
        Hit::HBar(BarHit::Forward)
    );
    // The track pages; the thumb reports its grab offset; dragging it to the end scrolls to the last page.
    let p = scrollbar_parts(
        hb,
        1,
        0,
        lay.body.width as usize,
        lay.total_w as usize,
        true,
        lay.thumb_min,
    );
    let thumb = p.thumb.expect("a thumb");
    assert_eq!(
        lv.hit(&lay, thumb.x + 2, hb.y + 3),
        Hit::HBar(BarHit::Thumb(2))
    );
    assert_eq!(
        lv.hit(&lay, thumb.right() + 2, hb.y + 3),
        Hit::HBar(BarHit::PageForward)
    );
    assert_eq!(
        lv.left_at(&lay, p.track.right() + 2, 2),
        lay.total_w - lay.body.width
    );
    assert_eq!(lv.left_at(&lay, p.track.x, 2), 0);
    assert!(lv.hscroll(1000, &lay));
    assert_eq!(lv.left, lay.total_w - lay.body.width);
    assert!(!lv.hscroll(1, &lay));
    assert_eq!(
        lv.hit(&lay, hb.x + ControlMetrics::CLASSIC.scroll + 2, hb.y + 3),
        Hit::HBar(BarHit::PageBack)
    );
    lv.resize_column(0, 1);
    assert_eq!(lv.columns[0].width, MIN_COL_W);
}

#[test]
fn both_bars_leave_each_other_room() {
    let mut lv = view(40);
    lv.resize_column(0, 300);
    let r = Rect::new(0, 0, 300, 100);
    let lay = layout(&lv, r, &ControlMetrics::CLASSIC);
    let (sb, hb) = (lay.scrollbar.unwrap(), lay.hscroll.unwrap());
    assert_eq!(sb.x, lay.body.right());
    assert_eq!(sb.bottom(), hb.y);
    assert_eq!(hb.right(), sb.x);
    let (c, _, theme) = render(&lv, r);
    assert_eq!(c.get(sb.x + 3, hb.y + 3), theme.colors.button_face.argb());
}

/// Under Blue the header is 20 rows and the rows start right under it; the pointer's column draws
/// lit, the held one sunk.
#[test]
fn a_blue_header_lights_the_column_the_pointer_is_on() {
    let theme = crate::interfaces::blue_theme(crate::blue::Variant::Blue);
    let cm = theme.look.controls();
    let text = TextRenderer::load(&FontConfig::default());
    let mut lv = view(3);
    let r = Rect::new(0, 0, 200, 4 + 20 + 3 * ROW_H);
    let lay = layout(&lv, r, &cm);
    assert_eq!(lay.header, Rect::new(2, 2, 196, 20));
    assert_eq!(lay.body.y, 22, "the rows start under the header");
    let (x0, x1) = (lay.header.x + 10, lay.header.x + 130);
    assert_eq!(lv.hit(&lay, x1, 10).column(), Some(1));
    assert_eq!(lv.hit(&lay, x1, 30).column(), None, "a row is no column");
    let band = |lv: &ListView| {
        let mut c = Canvas::new(r.width as u32, r.height as u32);
        draw(&mut c, &text, &theme.paint(), lv, &lay, None, 11);
        (c.get(x0, 2 + 17), c.get(x1, 2 + 17), c.get(x1, 2))
    };
    let faces = &crate::blue::Variant::Blue.scheme().header_item;
    let (rest, _, _) = band(&lv);
    lv.header.motion(Some(1));
    let (left, lit, _) = band(&lv);
    assert_eq!(left, rest, "the other column stays at rest");
    assert_ne!(lit, rest, "the lit column's band");
    assert!(
        (lit >> 16 & 0xFF) > (lit & 0xFF) + 0x40,
        "the lit column's band is orange"
    );
    lv.header.press(Some(1), false);
    let (_, _, top) = band(&lv);
    assert_eq!(
        top,
        faces.pressed.down[0].1.argb(),
        "the held column's shadow along its top"
    );
}

#[test]
fn the_sort_arrow_sits_after_the_title() {
    if !crate::text::test_tahoma() {
        return;
    }
    let text = TextRenderer::load(&FontConfig::default());
    let mut lv = view(3);
    lv.sort = Some((0, true));
    let r = Rect::new(0, 0, 300, 100);
    let (c, lay, theme) = render(&lv, r);
    let cs = &theme.colors;
    let dx = lay.header.x - COL0_X;
    let dy = lay.header.y - HEADER_Y;
    assert_eq!(
        lay.header.x + HEADER_TEXT_X,
        TITLE_X + dx,
        "the title starts at the expected x"
    );
    let title_w = text.measure("Name", false, theme.fonts.size_px);
    let (ax, ay) = (
        lay.header.x + HEADER_TEXT_X + title_w + ARROW_GAP,
        lay.header.y + ARROW_BOX.1 - HEADER_Y,
    );
    assert_eq!((ax, ay), (ARROW_BOX.0 + dx, ARROW_BOX.1 + dy));
    let (arrow_w, arrow_h) = (ARROW_BOX.2, ARROW_BOX.3);
    for row in 0..arrow_h {
        let (w, inset) = (1 + 2 * row, arrow_h - 1 - row);
        assert_eq!(c.get(ax + inset - 1, ay + row), cs.button_face.argb());
        for k in 0..w {
            assert_eq!(
                c.get(ax + inset + k, ay + row),
                cs.button_shadow.argb(),
                "row {row} px {k}"
            );
        }
        assert_eq!(c.get(ax + inset + w, ay + row), cs.button_face.argb());
    }
    assert_eq!(arrow_w, 7, "the arrow's widest row");
    assert_eq!(c.get(ax + 3, ay - 1), cs.button_face.argb());
    assert_eq!(c.get(ax + 3, ay + arrow_h), cs.button_face.argb());
    lv.sort = Some((0, false));
    let (c, ..) = render(&lv, r);
    assert_eq!(c.get(ax + 3, ay), cs.button_shadow.argb());
    assert_eq!(c.get(ax, ay), cs.button_shadow.argb());
    assert_eq!(c.get(ax, ay + arrow_h - 1), cs.button_face.argb());
}

/// A right-aligned 96 px column, sorted: the arrow ends where the unsorted title's ink did, 7 px
/// short of the column's end, and the title ends its ink 10 px before the arrow.
#[test]
fn a_right_aligned_sorted_column_puts_its_arrow_after_the_title() {
    if !crate::text::test_tahoma() {
        return;
    }
    let mut lv = view(0);
    lv.columns = vec![
        Column::new("Name", 180),
        Column {
            right: true,
            ..Column::new("Size", 96)
        },
    ];
    let r = Rect::new(0, 0, 400, 100);
    let inks = |lv: &ListView| {
        let (c, lay, theme) = render(lv, r);
        let x0 = lay.header.x + 180;
        let y = lay.header.y;
        let cols = |ink: Color| -> Vec<i32> {
            (x0..x0 + 96)
                .filter(|&x| (y + 2..y + 15).any(|y| c.get(x, y) == ink.argb()))
                .map(|x| x - x0)
                .collect()
        };
        let cs = &theme.colors;
        (cols(cs.button_text), cols(cs.button_shadow))
    };
    let (title, _) = inks(&lv);
    assert_eq!(
        title.last(),
        Some(&88),
        "the unsorted title's last ink column"
    );
    lv.sort = Some((1, true));
    let (title, arrow) = inks(&lv);
    assert_eq!(
        title.last(),
        Some(&71),
        "the sorted title's last ink column"
    );
    let arrow: Vec<i32> = arrow.into_iter().filter(|&x| x < 90).collect();
    assert_eq!(arrow, (82..89).collect::<Vec<_>>(), "the arrow's columns");
}

/// Blue's Details header with Name 180, Size 96 (right-aligned) and Type 120, sorted by `sort`:
/// per column, the title and arrow ink columns from the left edge.
fn blue_header_inks(sort: Option<(usize, bool)>) -> Vec<(Vec<i32>, Vec<i32>)> {
    let theme = crate::interfaces::blue_theme(crate::blue::Variant::Blue);
    let text = TextRenderer::load(&FontConfig::default());
    let mut lv = view(0);
    lv.columns = vec![
        Column::new("Name", 180),
        Column {
            right: true,
            ..Column::new("Size", 96)
        },
        Column::new("Type", 120),
    ];
    lv.sort = sort;
    let r = Rect::new(0, 0, 500, 100);
    let lay = layout(&lv, r, &theme.look.controls());
    let mut c = Canvas::new(r.width as u32, r.height as u32);
    draw(
        &mut c,
        &text,
        &theme.paint(),
        &lv,
        &lay,
        None,
        theme.fonts.size_px,
    );
    let y = lay.header.y;
    let ink = |x0: i32, w: i32, ink: u32| -> Vec<i32> {
        (x0..x0 + w)
            .filter(|&x| (y + 3..y + 16).any(|y| c.get(x, y) == ink))
            .map(|x| x - x0)
            .collect()
    };
    let text_ink = theme.colors.button_text.argb();
    let arrow_ink = crate::blue::Variant::Blue.scheme().system.gray_text.argb();
    [(0, 180), (180, 96), (276, 120)]
        .into_iter()
        .map(|(x, w)| {
            let x0 = lay.header.x + x;
            (ink(x0, w, text_ink), ink(x0, w, arrow_ink))
        })
        .collect()
}

/// Under Blue a sorted left-aligned title sits 3 px further in (ink from 9 to 12 px) with the arrow
/// 10 px after its ink; a sorted right-aligned one stops its 9 px arrow against the column's 2 px
/// divider (85..93 of 96) and ends its ink 10 px before it, at 74 (unsorted: 88).
#[test]
fn blue_sorted_titles_move_in_and_a_right_aligned_arrow_meets_the_divider() {
    if !crate::text::test_tahoma() {
        return;
    }
    let rest = blue_header_inks(None);
    assert_eq!(rest[0].0.first(), Some(&9), "Name at rest");
    assert_eq!(rest[1].0.last(), Some(&88), "Size at rest");
    assert_eq!(rest[2].0.first(), Some(&9), "Type at rest");
    assert!(rest.iter().all(|(_, a)| a.is_empty()), "no arrow at rest");
    for (ci, name) in [(0, "Name"), (2, "Type")] {
        for asc in [true, false] {
            let inks = blue_header_inks(Some((ci, asc)));
            let (title, arrow) = &inks[ci];
            assert_eq!(title.first(), Some(&12), "sorted {name} moves in");
            let end = *title.last().unwrap();
            assert_eq!(
                *arrow,
                (end + 11..end + 20).collect::<Vec<_>>(),
                "{name}'s arrow 10 px after its ink"
            );
        }
    }
    for asc in [true, false] {
        let inks = blue_header_inks(Some((1, asc)));
        let (title, arrow) = &inks[1];
        assert_eq!(*arrow, (85..94).collect::<Vec<_>>(), "Size's arrow");
        assert_eq!(title.last(), Some(&74), "Size's title");
    }
}

#[test]
fn every_selected_row_gets_the_highlight() {
    let mut lv = view(4);
    let r = Rect::new(0, 0, 300, 200);
    let lay = layout(&lv, r, &ControlMetrics::CLASSIC);
    lv.select(Some(1), &lay);
    lv.selection.click(
        3,
        Mods {
            ctrl: true,
            shift: false,
        },
    );
    let (c, lay, theme) = render(&lv, r);
    let cs = &theme.colors;
    let at = |c: &Canvas, k: i32| c.get(lay.body.x + NAME_X - 1, lay.body.y + k * ROW_H + 8);
    assert_eq!(at(&c, 0), cs.window.argb());
    assert_eq!(at(&c, 1), cs.hilight.argb());
    assert_eq!(at(&c, 2), cs.window.argb());
    assert_eq!(at(&c, 3), cs.hilight.argb());
    let mut lv = view(4);
    lv.drop_row = Some(2);
    lv.focus_ring = true;
    let (c, ..) = render(&lv, r);
    assert_eq!(at(&c, 2), cs.hilight.argb());
}

#[test]
fn scrolling_left_shifts_the_header_and_the_cells() {
    let mut lv = view(3);
    let r = Rect::new(0, 0, 300, 100);
    lv.resize_column(0, 300);
    let lay = layout(&lv, r, &ControlMetrics::CLASSIC);
    let (before, ..) = render(&lv, r);
    assert!(lv.hscroll(4, &lay));
    let (after, ..) = render(&lv, r);
    let theme = Theme::default();
    let black = theme.colors.button_text.argb();
    let ink = |c: &Canvas, y: i32| {
        (lay.header.x..lay.header.right())
            .find(|&x| c.get(x, y) == black)
            .expect("the title's ink")
    };
    let hy = lay.header.y + 8;
    assert_eq!(ink(&after, hy), ink(&before, hy) - 4);
    let ry = lay.body.y + 8;
    assert_eq!(ink(&after, ry), ink(&before, ry) - 4);
    for y in lay.header.y..lay.body.bottom() {
        assert_eq!(
            after.get(0, y),
            theme.colors.button_shadow.argb(),
            "frame at y {y}"
        );
    }
}

#[test]
fn the_rename_box_is_a_black_border_on_white_over_the_name() {
    if !crate::text::test_tahoma() {
        return;
    }
    let text = TextRenderer::load(&FontConfig::default());
    let theme = Theme::default();
    let px = theme.fonts.size_px;
    let cs = &theme.colors;
    let mut lv = view(3);
    lv.edit = Some((1, TextEdit::new("supermium_144_32_setup", true)));
    let r = Rect::new(0, 0, 300, 100);
    let lay = layout(&lv, r, &ControlMetrics::CLASSIC);
    let b = lv.edit_rect(&lay, 1, &text, px);
    let name_x = lay.body.x + NAME_X;
    assert_eq!(name_x - b.x, EDIT_LABEL_X - EDIT_BOX.0);
    assert_eq!(b.x + EDIT_PAD_L - name_x, EDIT_TEXT_X - EDIT_LABEL_X);
    assert_eq!((b.width, b.height), (EDIT_BOX.2, EDIT_BOX.3));
    assert_eq!(b.y, lay.body.y + ROW_H);
    let (c, ..) = render(&lv, r);
    let mid = b.y + b.height / 2;
    assert_eq!(c.get(b.x, mid), cs.window_frame.argb(), "left border");
    assert_eq!(c.get(b.right() - 1, mid), cs.window_frame.argb());
    assert_eq!(c.get(b.x + 2, b.y), cs.window_frame.argb(), "top border");
    assert_eq!(c.get(b.x + 2, b.bottom() - 1), cs.window_frame.argb());
    assert_eq!(c.get(b.x + 1, mid), cs.window.argb(), "white inside");
    // The whole name is selected: text sits on the highlight from EDIT_PAD_L, EDIT_PAD_R of white after.
    let sel_y = b.y + EDIT_INSET;
    assert_eq!(c.get(b.x + EDIT_PAD_L, sel_y), cs.hilight.argb());
    assert_eq!(c.get(b.x + EDIT_PAD_L - 1, sel_y), cs.window.argb());
    assert_eq!(c.get(b.right() - 3, mid), cs.window.argb());
    assert_eq!(c.get(b.x - 1, mid), cs.window.argb());
}

#[test]
fn check_rect_sits_8_px_in_and_3_px_down() {
    let mut lv = view(3);
    lv.checks = Some(vec![false; 3]);
    let r = Rect::new(0, 0, 300, 100);
    let lay = layout(&lv, r, &ControlMetrics::CLASSIC);
    assert_eq!(lv.check_rect(&lay, 0), Rect::new(8, lay.body.y + 3, 13, 13));
    assert_eq!(
        lv.check_rect(&lay, 1),
        Rect::new(8, lay.body.y + ROW_H + 3, 13, 13)
    );
}

/// At 150% and 200% the header, rows and group headers grow with the text; the check box
/// keeps its 13 px, centred down its taller row, and a selected name's highlight fills the row.
#[test]
fn grown_text_grows_rows_and_group_headers() {
    let text = TextRenderer::load(&FontConfig::default());
    let cs = &Theme::default().colors;
    for (pct, px) in [(100, 11), (150, 17), (200, 22)] {
        let u = TextUnits::new(px, 11);
        let cm = ControlMetrics::CLASSIC.grown(u);
        let (row_h, group_h) = (u.len(ROW_H), u.len(GROUP_H));
        let lv = grouped();
        let lay = layout(&lv, Rect::new(0, 0, 400, 400), &cm);
        assert_eq!(lay.header.height, u.len(HEADER_H), "{pct}%");
        let at = |y: i32| lv.hit(&lay, 30, lay.body.y + y);
        assert_eq!(at(group_h - 1), Hit::Background, "{pct}%: the header");
        assert_eq!(at(group_h), Hit::Row(0), "{pct}%");
        assert_eq!(at(group_h + row_h), Hit::Row(1), "{pct}%");
        assert_eq!(at(2 * group_h + 2 * row_h), Hit::Row(2), "{pct}%");

        let mut lv = view(3);
        lv.checks = Some(vec![true; 3]);
        lv.selection.set_only(Some(1));
        let lay = layout(&lv, Rect::new(0, 0, 400, 200), &cm);
        let b = lv.check_rect(&lay, 1);
        assert_eq!(b.width, 13, "{pct}%");
        assert_eq!(b.y, lay.body.y + row_h + 3 + (row_h - ROW_H) / 2, "{pct}%");
        let mut c = Canvas::new(400, 200);
        let p = Paint {
            text: u,
            ..classic_paint(cs, 1)
        };
        draw(&mut c, &text, &p, &lv, &lay, None, px);
        assert_eq!(
            c.get(b.x, b.y),
            cs.button_shadow.argb(),
            "{pct}%: the box's edge"
        );
        let hx = b.right() + CHECK_TEXT_GAP;
        let top = lay.body.y + row_h;
        assert_eq!(c.get(hx, top), cs.hilight.argb(), "{pct}%");
        assert_eq!(c.get(hx, top + row_h - 1), cs.hilight.argb(), "{pct}%");
        assert_ne!(c.get(hx, top + row_h), cs.hilight.argb(), "{pct}%");
    }
}

#[test]
fn a_check_box_takes_the_looks_side() {
    let mut lv = view(3);
    lv.checks = Some(vec![false; 3]);
    let cm = ControlMetrics {
        check: 11,
        ..ControlMetrics::CLASSIC
    };
    let lay = layout(&lv, Rect::new(0, 0, 300, 100), &cm);
    assert_eq!(lv.check_rect(&lay, 0), Rect::new(8, lay.body.y + 3, 11, 11));
    let y = lay.body.y + 3 + 10;
    assert_eq!(
        lv.hit(&lay, 8 + 10, y),
        Hit::Check(0),
        "the box's last column"
    );
    assert_eq!(lv.hit(&lay, 8 + 11, y), Hit::Row(0), "the row past it");
}

#[test]
fn hit_on_the_box_gives_check() {
    let mut lv = view(3);
    lv.checks = Some(vec![false; 3]);
    let r = Rect::new(0, 0, 300, 100);
    let lay = layout(&lv, r, &ControlMetrics::CLASSIC);
    let cr = lv.check_rect(&lay, 1);
    assert_eq!(lv.hit(&lay, cr.x + 1, cr.y + 1), Hit::Check(1));
}

#[test]
fn a_layout_taken_at_another_top_still_draws_every_check_box() {
    let mut lv = view(10);
    lv.checks = Some(vec![false; 10]);
    let r = Rect::new(0, 0, 300, 4 + HEADER_H + 8 * ROW_H);
    lv.top = 5;
    let lay = layout(&lv, r, &ControlMetrics::CLASSIC);
    assert_eq!(lay.rows, 8);
    lv.top = 0;
    let text = TextRenderer::load(&FontConfig::default());
    let theme = Theme::default();
    let mut c = Canvas::new(r.width as u32, r.height as u32);
    draw(
        &mut c,
        &text,
        &classic_paint(&theme.colors, 1),
        &lv,
        &lay,
        None,
        theme.fonts.size_px,
    );
    let cr = lv.check_rect(&lay, 7);
    assert_eq!(cr, Rect::new(8, lay.body.y + 7 * ROW_H + 3, 13, 13));
    assert_eq!(c.get(cr.x + 6, cr.y + 6), theme.colors.window.argb());
    assert_ne!(c.get(cr.x, cr.y), theme.colors.window.argb());
}

#[test]
fn toggle_flips_a_row_and_reports_whether_it_could() {
    let mut lv = view(3);
    assert!(!lv.toggle(0), "no checks column yet");
    lv.checks = Some(vec![false, false, false]);
    assert!(lv.toggle(0));
    assert!(lv.checks.as_ref().unwrap()[0]);
    assert!(lv.toggle(0));
    assert!(!lv.checks.as_ref().unwrap()[0]);
    assert!(!lv.toggle(3), "row out of range");
}

/// A row's check box acts on release: the press holds it, a release over it toggles, a release
/// after dragging off changes nothing, and dragging back on re-arms it.
#[test]
fn a_row_check_box_toggles_on_the_release_over_it() {
    use crate::pointer::Interaction;
    let mut lv = view(3);
    lv.checks = Some(vec![false; 3]);
    let on = |lv: &ListView, i: usize| lv.checks.as_ref().unwrap()[i];

    assert!(lv.press_check(Hit::Check(1)), "the press draws it held");
    assert_eq!(lv.held_check.of(1), Interaction::Pressed);
    assert!(!on(&lv, 1), "the press waits for the release");
    assert_eq!(lv.release_check(Hit::Check(1)), Some(1));
    assert!(on(&lv, 1), "the release over it toggles it");
    assert_eq!(lv.held_check.of(1), Interaction::Idle, "at rest after");

    for off in [Hit::Row(1), Hit::Check(2), Hit::Background] {
        lv.press_check(Hit::Check(1));
        assert!(lv.motion_check(off), "{off:?}: dragged off");
        assert_eq!(lv.held_check.of(1), Interaction::Idle, "{off:?}");
        assert_eq!(lv.release_check(off), None, "{off:?}");
        assert_eq!(lv.checks, Some(vec![false, true, false]), "{off:?}");
    }

    lv.press_check(Hit::Check(1));
    lv.motion_check(Hit::Row(1));
    assert!(lv.motion_check(Hit::Check(1)), "back on");
    assert_eq!(lv.held_check.of(1), Interaction::Pressed);
    assert_eq!(lv.release_check(Hit::Check(1)), Some(1));
    assert!(!on(&lv, 1), "back on, the release toggles it");

    assert!(!lv.press_check(Hit::Row(0)), "the row's text holds no box");
    assert!(!lv.motion_check(Hit::Check(0)), "nothing held lights");
    assert_eq!(lv.release_check(Hit::Check(0)), None);
    assert_eq!(lv.checks, Some(vec![false; 3]));
}

/// A held row box draws the Pressed frame with its old state; the release draws the new one at rest.
#[test]
fn a_held_blue_row_box_draws_pressed_and_ticks_on_the_release() {
    let theme = crate::interfaces::theme_for(crate::interface::Interface::Blue);
    let cm = theme.look.controls();
    let text = TextRenderer::load(&FontConfig::default());
    let mut lv = view(3);
    lv.checks = Some(vec![false; 3]);
    let r = Rect::new(0, 0, 300, 100);
    let lay = layout(&lv, r, &cm);
    let cr = lv.check_rect(&lay, 0);
    let px = |lv: &ListView, dx: i32, dy: i32| {
        let mut c = Canvas::new(r.width as u32, r.height as u32);
        draw(&mut c, &text, &theme.paint(), lv, &lay, None, 11);
        c.get(cr.x + dx, cr.y + dy)
    };
    let mut ticked = lv.clone();
    ticked.checks = Some(vec![true; 3]);
    let faces = &crate::blue::Variant::Blue.scheme().check_box;
    let rest = px(&lv, 6, 6);
    lv.press_check(Hit::Check(0));
    assert_ne!(px(&lv, 1, 1), px(&ticked, 1, 1), "the pressed well");
    assert_ne!(px(&lv, 6, 6), rest, "the pressed fill");
    assert_ne!(
        px(&lv, 6, 7),
        0xFF00_0000 | faces.pressed_checked.mark.unwrap().argb(),
        "no tick while held"
    );
    lv.release_check(Hit::Check(0));
    assert_eq!(px(&lv, 1, 1), px(&ticked, 1, 1), "a ticked box at rest");
    assert_eq!(
        px(&lv, 6, 7),
        0xFF00_0000 | faces.normal_checked.mark.unwrap().argb(),
        "the tick"
    );
}

#[test]
fn without_checks_the_box_spot_hits_the_row() {
    let lv = view(3);
    let r = Rect::new(0, 0, 300, 100);
    let lay = layout(&lv, r, &ControlMetrics::CLASSIC);
    assert_eq!(lv.hit(&lay, 10, lay.body.y + 3), Hit::Row(0));
}

#[test]
fn draws_the_selected_name_on_the_highlight() {
    let mut lv = view(3);
    let r = Rect::new(0, 0, 300, 100);
    let lay = layout(&lv, r, &ControlMetrics::CLASSIC);
    lv.select(Some(1), &lay);
    let (c, lay, theme) = render(&lv, r);
    let cs = &theme.colors;
    let y = lay.body.y + ROW_H + 8;
    assert_eq!(c.get(lay.body.x + NAME_X - 1, y), cs.hilight.argb());
    assert_eq!(c.get(lay.body.x + NAME_X - 3, y), cs.window.argb());
    assert_eq!(c.get(lay.body.x + 1, lay.body.y + 8), cs.window.argb());
    assert_eq!(c.get(lay.header.x, lay.header.y), cs.button_hilight.argb());
}

/// A grouped list: two groups over four rows.
fn grouped() -> ListView {
    let mut lv = view(4);
    lv.groups = vec![
        Group {
            title: "LAN or High-Speed Internet".into(),
            first: 0,
        },
        Group {
            title: "Wizard".into(),
            first: 2,
        },
    ];
    lv
}

#[test]
fn group_headers_take_a_taller_row() {
    let lv = grouped();
    let lay = layout(&lv, Rect::new(0, 0, 300, 200), &ControlMetrics::CLASSIC);
    assert_eq!(lv.slots(), 6, "four rows and a header over each group");
    let at = |y: i32| lv.hit(&lay, 10, lay.body.y + y + 3);
    // Slots down the body: a 31 px header, two rows, the second header, two more rows.
    assert_eq!(at(0), Hit::Background, "the first group's header");
    assert_eq!(at(GROUP_H), Hit::Row(0));
    assert_eq!(at(GROUP_H + ROW_H), Hit::Row(1));
    assert_eq!(
        at(GROUP_H + 2 * ROW_H),
        Hit::Background,
        "the second header"
    );
    assert_eq!(at(2 * GROUP_H + 2 * ROW_H), Hit::Row(2));
    assert_eq!(at(2 * GROUP_H + 3 * ROW_H), Hit::Row(3));
    assert_eq!(
        at(2 * GROUP_H + 4 * ROW_H),
        Hit::Background,
        "past the last"
    );
    assert_eq!(
        lv.hit(&lay, 10, lay.body.y + GROUP_H + 2 * ROW_H - 1),
        Hit::Row(1)
    );
    assert_eq!(view(4).slots(), 4);
    let plain = layout(
        &view(4),
        Rect::new(0, 0, 300, 200),
        &ControlMetrics::CLASSIC,
    );
    assert_eq!(
        plain.rows,
        layout(
            &view(99),
            Rect::new(0, 0, 300, 200),
            &ControlMetrics::CLASSIC
        )
        .rows
    );
}

#[test]
fn a_borderless_list_lays_out_on_the_rect() {
    // A borderless list puts the header and body on the rect's own top and left.
    let mut lv = view(4);
    lv.frame = false;
    let r = Rect::new(20, 30, 300, 200);
    let lay = layout(&lv, r, &ControlMetrics::CLASSIC);
    assert_eq!(lay.header, Rect::new(20, 30, 300, HEADER_H));
    assert_eq!(lay.body.x, 20);
    assert_eq!(lay.body.y, 30 + HEADER_H);
    assert_eq!(lay.body.width, 300);
    lv.frame = true;
    let framed = layout(&lv, r, &ControlMetrics::CLASSIC);
    assert_eq!(framed.header, Rect::new(22, 32, 296, HEADER_H));
    assert_eq!(framed.body.x, 22);
    if !crate::text::test_tahoma() {
        return;
    }
    lv.frame = false;
    let (c, _, theme) = render(&lv, Rect::new(0, 0, 300, 200));
    assert_ne!(
        c.get(0, 0),
        theme.colors.button_shadow.argb(),
        "no sunken edge"
    );
}

#[test]
fn a_group_header_is_bold_over_the_gradient_rule() {
    if !crate::text::test_tahoma() {
        return;
    }
    let lv = grouped();
    let r = Rect::new(0, 0, 400, 200);
    let (c, lay, theme) = render(&lv, r);
    let cs = &theme.colors;
    let mid = lay.body.y + ROW_H / 2;
    let ink = (lay.body.x..lay.body.right())
        .find(|&x| c.get(x, mid) == cs.window_text.argb())
        .expect("the title's ink");
    assert_eq!(ink, lay.body.x + GROUP_TEXT_X, "inks 14 px in");
    // The rule from that edge: three pixels at 0, 150 and 300 px in.
    let ly = lay.body.y + GROUP_RULE_Y;
    assert_eq!(c.get(lay.body.x, ly), LINE_LEFT.argb(), "the left end");
    assert_eq!(c.get(lay.body.x + 150, ly), LINE_MID.argb(), "half way");
    assert_eq!(
        c.get(lay.body.x + GROUP_LINE_W, ly),
        cs.window.argb(),
        "white once the fade is spent"
    );
    assert_eq!(c.get(lay.body.x, ly - 1), cs.window.argb());
    assert_eq!(c.get(lay.body.x, ly + 1), cs.window.argb());
    assert_eq!(
        c.get(lay.body.x + 1, lay.body.y + ROW_H + 8),
        cs.window.argb()
    );
}

#[test]
fn a_header_at_the_top_pushes_its_own_rows_down() {
    let mut lv = view(10);
    lv.groups = vec![Group {
        title: "LAN or High-Speed Internet".into(),
        first: 0,
    }];
    let r = Rect::new(0, 0, 300, 4 + HEADER_H + 5 * ROW_H);
    let lay = layout(&lv, r, &ControlMetrics::CLASSIC);
    // The 31 px header slot against 17 px rows: four slots fill what held five rows.
    assert_eq!(lay.rows, 4);
    assert_eq!(
        lv.hit(&lay, 10, lay.body.y + GROUP_H + 2 * ROW_H + 3),
        Hit::Row(2)
    );
    let text = TextRenderer::load(&FontConfig::default());
    let px = Theme::default().fonts.size_px;
    assert_eq!(lv.edit_rect(&lay, 0, &text, px).y, lay.body.y + GROUP_H);
    assert_eq!(
        lv.edit_rect(&lay, 1, &text, px).y,
        lay.body.y + GROUP_H + ROW_H
    );
    // Focus scrolls one row sooner than without the header.
    lv.select(Some(4), &lay);
    assert_eq!(lv.top, 1, "row 4 sits below the header at top 0");
    // Scrolled past the group's start the header is gone: row 4 is three slots down, row 0 one above.
    assert_eq!(lv.edit_rect(&lay, 4, &text, px).y, lay.body.y + 3 * ROW_H);
    assert_eq!(lv.edit_rect(&lay, 0, &text, px).y, lay.body.y - ROW_H);
}

#[test]
#[ignore]
fn render_listview() {
    let (c, ..) = render(&sample(), Rect::new(0, 0, 320, 130));
    crate::raster::write_preview(&c, "listview-preview.png");
}

/// The bar part the host lights draws its Blue Hot frame, a held one Pressed; `Hit::bar` names it.
#[test]
fn a_blue_lists_lit_bar_part_draws_hot_or_pressed() {
    use crate::classic::widgets::{Axis, ScrollHit};
    let theme = crate::interfaces::theme_for(crate::interface::Interface::Blue);
    let cm = theme.look.controls();
    let text = TextRenderer::load(&FontConfig::default());
    let mut lv = view(500);
    let r = Rect::new(0, 0, 200, 150);
    let lay = layout(&lv, r, &cm);
    let sb = lay.scrollbar.expect("500 rows overflow the body");
    let up = Some((Axis::Vertical, ScrollHit::Back));
    assert_eq!(lv.hit(&lay, sb.x + 8, sb.y + 2).bar(), up);
    let drawn = |lv: &ListView| {
        let mut c = Canvas::new(r.width as u32, r.height as u32);
        draw(&mut c, &text, &theme.paint(), lv, &lay, lv.bars.lit(), 11);
        c.get(sb.x + 8, sb.y + 2)
    };
    assert!(lv.bars.motion(up));
    assert_eq!(
        drawn(&lv),
        crate::look::styled::fixture::blue_pixel(
            crate::style::Part::ScrollArrowUp,
            crate::style::State::Hot,
            8,
            2
        ),
        "the hot up arrow"
    );
    assert!(lv.bars.press_bar(up));
    assert_eq!(
        drawn(&lv),
        crate::look::styled::fixture::blue_pixel(
            crate::style::Part::ScrollArrowUp,
            crate::style::State::Pressed,
            8,
            2
        ),
        "the held up arrow"
    );
    assert!(lv.bars.release_bar(), "back to hot");
    assert!(lv.bars.leave());
    assert_eq!(
        drawn(&lv),
        crate::look::styled::fixture::blue_pixel(
            crate::style::Part::ScrollArrowUp,
            crate::style::State::Normal,
            8,
            2
        ),
        "the up arrow at rest"
    );
}
