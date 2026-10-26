const LINE_GRAY: Color = Color::rgb(128, 128, 128);

use super::*;
use crate::look::classic_paint;
use crate::theme::{FontConfig, Theme};

fn tree() -> TreeView {
    let mut c = Node::new("Local Disk", true);
    c.children = vec![Node::new("Programs", true), Node::new("Users", true)];
    c.expanded = true;
    let mut desktop = Node::new("Desktop", true);
    desktop.children = vec![
        Node::new("My Documents", true),
        c,
        Node::new("Recycle Bin", false),
    ];
    desktop.expanded = true;
    TreeView {
        roots: vec![desktop],
        ..Default::default()
    }
}

/// The rows are built once per tree shape and shared until a node opens or closes.
#[test]
fn visible_rows_are_shared_until_the_shape_changes() {
    let mut tv = tree();
    let a = tv.visible();
    let b = tv.visible();
    assert!(Rc::ptr_eq(&a, &b), "the same rows twice");
    tv.roots[0].children[1].expanded = false;
    let c = tv.visible();
    assert!(!Rc::ptr_eq(&a, &c), "a collapse through the field rebuilds");
    assert!(c.len() < a.len());
    tv.roots[0].children.push(Node {
        label: "New".into(),
        ..Node::default()
    });
    assert!(tv.visible().len() == c.len() + 1, "a new sibling rebuilds");
}

/// Rows cached for another shape of the same size are rebuilt.
#[test]
fn visible_rebuilds_rows_left_from_another_shape() {
    let tv = tree();
    let want = tv.visible().as_ref().clone();
    let mut other = tree();
    other.roots[0].children[1].expanded = false;
    other.roots[0].children[0].expanded = true;
    other.roots[0].children[0].children = vec![Node::new("a", false), Node::new("b", false)];
    let stale = other.visible();
    assert_eq!(stale.len(), want.len());
    *tv.rows.borrow_mut() = Some(stale);
    assert_eq!(*tv.visible(), want, "the rows of the tree as it stands");
}

#[test]
fn visible_rows_follow_expansion_and_keys_walk_them() {
    let mut tv = tree();
    let rows = tv.visible();
    let labels: Vec<&str> = rows
        .iter()
        .map(|r| tv.node(&r.path).unwrap().label.as_str())
        .collect();
    assert_eq!(
        labels,
        [
            "Desktop",
            "My Documents",
            "Local Disk",
            "Programs",
            "Users",
            "Recycle Bin"
        ]
    );
    assert_eq!(rows[2].depth, 1);
    assert!(rows[5].last && !rows[2].last);
    let text = TextRenderer::load(&FontConfig::default());
    let px = FontConfig::default().size_px;
    let lay = layout(
        &tv,
        &text,
        px,
        Rect::new(0, 0, 200, 300),
        &ControlMetrics::CLASSIC,
    );
    tv.step(1, &lay);
    assert_eq!(tv.selected, Some(vec![0]));
    tv.step(2, &lay);
    assert_eq!(tv.selected, Some(vec![0, 1]));
    assert!(tv.collapse_selected(&lay));
    assert_eq!(tv.visible().len(), 4);
    assert!(tv.collapse_selected(&lay));
    assert_eq!(tv.selected, Some(vec![0]));
    tv.step(2, &lay);
    assert_eq!(tv.expand_selected(&lay), None, "children already loaded");
    assert_eq!(tv.visible().len(), 6);
    tv.step(1, &lay);
    assert_eq!(tv.expand_selected(&lay), Some(vec![0, 1, 0]));
    assert!(tv.node(&[0, 1, 0]).unwrap().expanded);
}

/// The Open With style: 18 px rows, no expander, the icon 3 px into its level.
#[test]
fn plain_style_has_18_px_rows_and_no_expanders() {
    let mut tv = tree();
    tv.style = Style::Plain;
    let text = TextRenderer::load(&FontConfig::default());
    let lay = layout(
        &tv,
        &text,
        11,
        Rect::new(0, 0, 200, 210),
        &ControlMetrics::CLASSIC,
    );
    assert_eq!(lay.rows, 11, "206 px of body holds 11 full rows");
    let b = lay.body;
    let y = b.y + 2 * PLAIN_ROW_H + 8;
    assert_eq!(tv.hit(&lay, b.x + INDENT + BOX_X + 4, y), Hit::Row(2));
    assert_eq!(tv.hit(&lay, b.x + INDENT + 2, y), Hit::Nothing);
    assert_eq!(tv.hit(&lay, b.x + INDENT + PLAIN_ICON_X, y), Hit::Row(2));
}

#[test]
fn hits_expanders_and_rows() {
    let tv = tree();
    let text = TextRenderer::load(&FontConfig::default());
    let px = FontConfig::default().size_px;
    let lay = layout(
        &tv,
        &text,
        px,
        Rect::new(0, 0, 200, 300),
        &ControlMetrics::CLASSIC,
    );
    let b = lay.body;
    let y = b.y + 2 * ROW_H + 8;
    assert_eq!(tv.hit(&lay, b.x + INDENT + BOX_X + 4, y), Hit::Expander(2));
    assert_eq!(tv.hit(&lay, b.x + 2 * INDENT + 5, y), Hit::Row(2));
    assert_eq!(tv.hit(&lay, b.x + 2, y), Hit::Nothing);
    let y5 = b.y + 5 * ROW_H + 8;
    assert_eq!(tv.hit(&lay, b.x + INDENT + BOX_X + 4, y5), Hit::Nothing);
    assert_eq!(tv.hit(&lay, b.x + 5, b.y + 10 * ROW_H), Hit::Nothing);
}

#[test]
fn draws_boxes_lines_and_the_selection() {
    let text = TextRenderer::load(&FontConfig::default());
    let theme = Theme::default();
    let cs = &theme.colors;
    let px = theme.fonts.size_px;
    let mut tv = tree();
    let lay = layout(
        &tv,
        &text,
        px,
        Rect::new(0, 0, 200, 300),
        &ControlMetrics::CLASSIC,
    );
    tv.select(2, &lay);
    let mut c = Canvas::new(200, 300);
    draw(&mut c, &text, &classic_paint(cs, 1), &tv, &lay, None, px);
    let b = lay.body;
    let mid = b.y + ROW_H / 2;
    assert_eq!(c.get(b.x + BOX_X, mid - 4), LINE_GRAY.argb());
    assert_eq!(c.get(b.x + BOX_X + 4, mid), cs.button_text.argb());
    assert_eq!(
        c.get(b.x + BOX_X + 4, mid - 2),
        cs.window.argb(),
        "no + on an open node"
    );
    let mid4 = b.y + 4 * ROW_H + ROW_H / 2;
    assert_eq!(
        c.get(b.x + 2 * INDENT + BOX_X + 4, mid4 - 2),
        cs.button_text.argb()
    );
    let tx = b.x + 2 * INDENT + ICON_PX + LABEL_GAP;
    assert_eq!(c.get(tx - 1, b.y + 2 * ROW_H + 8), cs.hilight.argb());
}

/// At a grown text size the rows grow with the text while the expander box keeps its 9 px,
/// centred on the taller row; hit and the selection band follow the grown pitch.
#[test]
fn grown_text_grows_the_rows_and_keeps_the_box() {
    let text = TextRenderer::load(&FontConfig::default());
    let cs = &Theme::default().colors;
    for (pct, px) in [(100, 11), (150, 17), (200, 22)] {
        let u = crate::theme::TextUnits::new(px, 11);
        let cm = ControlMetrics::CLASSIC.grown(u);
        let mut tv = tree();
        let lay = layout(&tv, &text, px, Rect::new(0, 0, 300, 400), &cm);
        let row_h = u.len(ROW_H);
        assert_eq!(lay.row_h, row_h, "{pct}%");
        assert_eq!(lay.rows, ((400 - 4) / row_h) as usize, "{pct}%");
        tv.select(2, &lay);
        let p = Paint {
            text: u,
            ..classic_paint(cs, 1)
        };
        let mut c = Canvas::new(300, 400);
        draw(&mut c, &text, &p, &tv, &lay, None, px);
        let b = lay.body;
        assert_eq!(tv.hit(&lay, b.x + 60, b.y + 2 * row_h + 1), Hit::Row(2));
        assert_eq!(tv.hit(&lay, b.x + 60, b.y + 2 * row_h - 1), Hit::Row(1));
        // Row 0's box: its top ring row 4 px above the row's middle, the box 9 px tall.
        let mid = b.y + row_h / 2;
        assert_eq!(c.get(b.x + BOX_X, mid - 4), LINE_GRAY.argb(), "{pct}%");
        assert_eq!(c.get(b.x + BOX_X + 4, mid - 4), LINE_GRAY.argb(), "{pct}%");
        assert_eq!(c.get(b.x + BOX_X + 4, mid + 4), LINE_GRAY.argb(), "{pct}%");
        assert_ne!(c.get(b.x + BOX_X + 4, mid + 5), LINE_GRAY.argb(), "{pct}%");
        // The selected row's band fills its grown height.
        let tx = b.x + 2 * INDENT + ICON_PX + LABEL_GAP;
        assert_eq!(c.get(tx - 1, b.y + 2 * row_h), cs.hilight.argb(), "{pct}%");
        assert_eq!(
            c.get(tx - 1, b.y + 3 * row_h - 1),
            cs.hilight.argb(),
            "{pct}%"
        );
        assert_ne!(c.get(tx - 1, b.y + 3 * row_h), cs.hilight.argb(), "{pct}%");
    }
}

/// An expanded node blits `open_icon`, a collapsed one `icon`, as different pixels.
#[test]
fn an_expanded_node_blits_its_open_icon() {
    let text = TextRenderer::load(&FontConfig::default());
    let theme = Theme::default();
    let cs = &theme.colors;
    let px = theme.fonts.size_px;
    let solid = |color: Color| {
        let mut ic = Canvas::new(ICON_PX as u32, ICON_PX as u32);
        ic.fill_rect(Rect::new(0, 0, ICON_PX, ICON_PX), color);
        ic
    };
    let (shut, open) = (Color::rgb(255, 0, 0), Color::rgb(0, 255, 0));
    let mut tv = tree();
    for n in &mut tv.roots[0].children {
        n.icon = Some(solid(shut));
        n.open_icon = Some(solid(open));
    }
    let lay = layout(
        &tv,
        &text,
        px,
        Rect::new(0, 0, 200, 300),
        &ControlMetrics::CLASSIC,
    );
    let mut c = Canvas::new(200, 300);
    draw(&mut c, &text, &classic_paint(cs, 1), &tv, &lay, None, px);
    let b = lay.body;
    let icon_x = |row: &VisibleRow| b.x + (row.depth as i32 + 1) * INDENT + ICON_PX / 2;
    let rows = tv.visible();
    assert!(!tv.node(&rows[1].path).unwrap().expanded);
    assert!(tv.node(&rows[2].path).unwrap().expanded);
    assert_eq!(c.get(icon_x(&rows[1]), b.y + ROW_H + 8), shut.argb());
    assert_eq!(c.get(icon_x(&rows[2]), b.y + 2 * ROW_H + 8), open.argb());
}

#[test]
fn a_drop_hilight_draws_an_unselected_row_selected() {
    let text = TextRenderer::load(&FontConfig::default());
    let theme = Theme::default();
    let cs = &theme.colors;
    let px = theme.fonts.size_px;
    let mut tv = tree();
    tv.focus_ring = true;
    tv.drop_hilight = Some(2);
    assert_eq!(tv.selected, None, "nothing is selected");
    let lay = layout(
        &tv,
        &text,
        px,
        Rect::new(0, 0, 200, 300),
        &ControlMetrics::CLASSIC,
    );
    let mut c = Canvas::new(200, 300);
    draw(&mut c, &text, &classic_paint(cs, 1), &tv, &lay, None, px);
    let b = lay.body;
    let tx = b.x + 2 * INDENT + ICON_PX + LABEL_GAP;
    assert_eq!(c.get(tx - 1, b.y + 2 * ROW_H + 8), cs.hilight.argb());
    assert_eq!(c.get(tx - 1, b.y + ROW_H + 8), cs.window.argb());
    assert_ne!(c.get(tx - 2, b.y + 2 * ROW_H), cs.hilight_text.argb());
}

/// A tree that lacks the focus shows its pick in the face colour with no ring.
#[test]
fn an_inactive_selection_draws_in_the_face_colour() {
    let (theme, text) = (Theme::default(), TextRenderer::load(&FontConfig::default()));
    let cs = &theme.colors;
    let mut tv = TreeView::new(vec![Node::new("Desktop", false)], Style::Classic);
    tv.selected = Some(vec![0]);
    tv.focus_ring = true;
    let r = Rect::new(0, 0, 200, 60);
    let lay = layout(&tv, &text, 11, r, &ControlMetrics::CLASSIC);
    // (fill, ring): fill is inside the highlight past "Desktop"'s glyphs; ring is the highlight's top-left corner, always a dot of `draw_focus_ring`'s pattern.
    let probe = |tv: &TreeView| {
        let mut c = Canvas::new(200, 60);
        draw(&mut c, &text, &classic_paint(cs, 1), tv, &lay, None, 11);
        (
            c.get(lay.body.x + 76, lay.body.y + 8),
            c.get(lay.body.x + 35, lay.body.y),
        )
    };
    tv.inactive = false;
    let (fill, ring) = probe(&tv);
    assert_eq!(
        fill,
        cs.hilight.argb(),
        "a focused tree's pick is the highlight"
    );
    assert_eq!(
        ring,
        cs.hilight_text.argb(),
        "a focused tree's pick keeps its focus ring"
    );
    tv.inactive = true;
    let (fill, ring) = probe(&tv);
    assert_eq!(
        fill,
        cs.button_face.argb(),
        "an inactive pick is the face colour"
    );
    assert_eq!(
        ring,
        cs.button_face.argb(),
        "an inactive pick draws no ring"
    );
}

/// Labels wider than the body bring the horizontal scrollbar, which hits
/// and scrolls like `listview`'s; `layout` measures labels once and reuses
/// the width until a label, the size or the rows change.
#[test]
fn layout_reuses_the_measured_width_until_a_label_changes() {
    let text = TextRenderer::load(&FontConfig::default());
    let px = Theme::default().fonts.size_px;
    let mut tv = tree();
    let r = Rect::new(0, 0, 200, 300);
    let w = layout(&tv, &text, px, r, &ControlMetrics::CLASSIC).content_w;
    assert!(w > 0);
    let (key, _) = tv.content_w.get().expect("the width is cached");
    tv.content_w.set(Some((key, 999)));
    assert_eq!(
        layout(&tv, &text, px, r, &ControlMetrics::CLASSIC).content_w,
        999,
        "the cached width"
    );
    assert_ne!(
        layout(&tv, &text, px + 2, r, &ControlMetrics::CLASSIC).content_w,
        999,
        "a new size"
    );
    tv.content_w.set(Some((key, 999)));
    tv.roots[0].children[0].label = "Docs".into();
    assert_ne!(
        layout(&tv, &text, px, r, &ControlMetrics::CLASSIC).content_w,
        999,
        "a rename"
    );
}

/// A look with a 17 px scrollbar lays both bars out 17 px thick, and the body gives up 17 px to each.
#[test]
fn a_17_px_scrollbar_takes_17_px_off_the_body_each_way() {
    let text = TextRenderer::load(&FontConfig::default());
    let px = Theme::default().fonts.size_px;
    let mut tv = tree();
    tv.roots[0].children[0].label = "a folder with a very, very long name".into();
    let r = Rect::new(0, 0, 150, 60);
    let cm = ControlMetrics {
        scroll: 17,
        ..ControlMetrics::CLASSIC
    };
    let lay = layout(&tv, &text, px, r, &cm);
    let vb = lay.scrollbar.expect("the rows overflow the body");
    let hb = lay.hscroll.expect("the label overflows the body");
    assert_eq!(lay.body, Rect::new(2, 2, 129, 39));
    assert_eq!(vb, Rect::new(131, 2, 17, 39));
    assert_eq!(hb, Rect::new(2, 41, 129, 17));
}

/// A tree whose widest row collapses away or whose pane widens, scrolled
/// to the far end, draws and hits as if scrolled to its new far end (0
/// once the rows fit), with no blank past it.
#[test]
fn a_narrower_tree_holds_its_sideways_scroll_to_the_widest_row() {
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let px = theme.fonts.size_px;
    let mut tv = tree();
    tv.style = Style::Explorer;
    tv.roots[0].children[1].children[0].label = "a folder with a very, very long name".into();
    tv.roots[0].children[2].label = "a folder with a long name".into();
    let narrow = Rect::new(0, 0, 160, 300);
    let lay = |tv: &TreeView, r| layout(tv, &text, px, r, &ControlMetrics::CLASSIC);
    let shot = |tv: &TreeView, l: &Layout| {
        let mut c = Canvas::new(400, 300);
        draw(
            &mut c,
            &text,
            &classic_paint(&theme.colors, 1),
            tv,
            l,
            None,
            px,
        );
        c
    };
    let wide = lay(&tv, narrow);
    tv.hscroll(10_000, &wide);
    let far = tv.left;
    assert_eq!(far, wide.content_w - wide.body.width);

    tv.roots[0].children[1].expanded = false;
    let l = lay(&tv, narrow);
    let end = l.content_w - l.body.width;
    assert!((1..far).contains(&end), "the tree still overflows, by less");
    assert_eq!(tv.left_in(&l), end);
    let held = TreeView {
        left: end,
        ..tv.clone()
    };
    assert!(shot(&tv, &l) == shot(&held, &l), "drawn at the new far end");
    let (x, y) = (l.body.right() - 20, l.body.y + 3 * ROW_H + 8);
    assert_eq!(tv.hit(&l, x, y), held.hit(&l, x, y));
    assert!(!tv.hscroll(1, &l), "already at the far end");
    assert!(tv.hscroll(-1, &l));
    assert_eq!(tv.left, end - 1, "the arrow steps from the far end");

    tv.left = far;
    let roomy = lay(&tv, Rect::new(0, 0, 400, 300));
    assert!(roomy.hscroll.is_none());
    assert_eq!(tv.left_in(&roomy), 0);
    let home = TreeView {
        left: 0,
        ..tv.clone()
    };
    assert!(shot(&tv, &roomy) == shot(&home, &roomy));
}

#[test]
fn wide_labels_bring_the_horizontal_scrollbar() {
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let px = theme.fonts.size_px;
    let mut tv = tree();
    let r = Rect::new(0, 0, 200, 300);
    assert!(
        layout(&tv, &text, px, r, &ControlMetrics::CLASSIC)
            .hscroll
            .is_none()
    );
    tv.roots[0].children[0].label = "a folder with a very, very long name".into();
    let lay = layout(&tv, &text, px, r, &ControlMetrics::CLASSIC);
    let hb = lay.hscroll.expect("the label overflows the body");
    assert_eq!(
        hb,
        Rect::new(
            lay.body.x,
            lay.body.bottom(),
            lay.body.width,
            ControlMetrics::CLASSIC.scroll
        )
    );
    assert!(lay.content_w > lay.body.width);
    assert_eq!(tv.hit(&lay, hb.x + 3, hb.y + 3), Hit::HBar(BarHit::Back));
    assert_eq!(
        tv.hit(&lay, hb.right() - 3, hb.y + 3),
        Hit::HBar(BarHit::Forward)
    );
    // The thumb reports its grab, the track past it pages, a drag to the track's end lands on the last page.
    let p = scrollbar_parts(
        hb,
        1,
        0,
        lay.body.width as usize,
        lay.content_w as usize,
        true,
        lay.thumb_min,
    );
    let thumb = p.thumb.expect("a thumb");
    assert_eq!(
        tv.hit(&lay, thumb.x + 2, hb.y + 3),
        Hit::HBar(BarHit::Thumb(2))
    );
    assert_eq!(
        tv.hit(&lay, thumb.right() + 2, hb.y + 3),
        Hit::HBar(BarHit::PageForward)
    );
    assert_eq!(
        tv.left_at(&lay, p.track.right(), 2),
        lay.content_w - lay.body.width
    );
    assert!(tv.hscroll(1000, &lay));
    assert_eq!(
        tv.hit(&lay, hb.x + ControlMetrics::CLASSIC.scroll + 2, hb.y + 3),
        Hit::HBar(BarHit::PageBack)
    );
    assert_eq!(tv.left, lay.content_w - lay.body.width);
    assert!(!tv.hscroll(1, &lay), "clamped at the right edge");
    // Scrolled a little, the depth-1 expander box moves left with it and the hit test follows.
    tv.left = 4;
    let y = lay.body.y + 2 * ROW_H + 8;
    let bx = lay.body.x + INDENT + BOX_X + 4 - tv.left;
    assert_eq!(tv.hit(&lay, bx, y), Hit::Expander(2));
    assert_eq!(tv.hit(&lay, bx + INDENT, y), Hit::Row(2));
}

/// A folder icon and its open twin, as solid blocks so a test can find their columns.
fn icons() -> (Canvas, Canvas) {
    let mut shut = Canvas::new(ICON_PX as u32, ICON_PX as u32);
    shut.fill_rect(Rect::new(0, 0, ICON_PX, ICON_PX), Color::rgb(255, 224, 123));
    let mut open = Canvas::new(ICON_PX as u32, ICON_PX as u32);
    open.fill_rect(Rect::new(0, 0, ICON_PX, ICON_PX), Color::rgb(174, 218, 255));
    (shut, open)
}

/// The Folders tree is flat: no sunken edge, its whole rect white, rows from the rect's top. The connector-line tree keeps its 2 px edge.
#[test]
fn the_explorer_style_draws_no_edge() {
    let theme = Theme::default();
    let cs = &theme.colors;
    let text = TextRenderer::load(&FontConfig::default());
    let px = theme.fonts.size_px;
    let r = Rect::new(0, 0, 120, 80);
    for (style, edge) in [
        (Style::Explorer, cs.window),
        (Style::Classic, cs.button_shadow),
    ] {
        let tv = TreeView::new(vec![Node::new("Desktop", false)], style);
        let lay = layout(&tv, &text, px, r, &ControlMetrics::CLASSIC);
        let mut c = Canvas::new(120, 80);
        draw(&mut c, &text, &classic_paint(cs, 1), &tv, &lay, None, px);
        assert_eq!(c.get(0, 0), edge.argb(), "{style:?}'s corner");
        assert_eq!(c.get(0, 60), edge.argb(), "{style:?}'s left column");
    }
    let tv = TreeView::new(vec![Node::new("Desktop", false)], Style::Explorer);
    let lay = layout(&tv, &text, px, r, &ControlMetrics::CLASSIC);
    assert_eq!((lay.body.x, lay.body.y), (0, 0));
}

/// `Style::Explorer`'s pixels, client-local: the white body starts at x 1.
/// "Desktop" inks its icon at x 5..20 with nothing left of it and its label
/// from x 26. The selected "My Documents" at depth 1 has the 9 px box at
/// x 9..17, its icon at x 24..39 and its band from x 43. "Downloads"
/// (depth 2) inks its icon at x 43..58 and its label from x 64.
#[test]
fn explorer_style_lands_on_its_pixels() {
    const BODY_X: i32 = 1;
    const ROOT_ICON: i32 = 5;
    const ROOT_LABEL: i32 = 26;
    const BOX_L: i32 = 9;
    const BOX_R: i32 = 17;
    const D1_ICON: i32 = 24;
    const D1_HILIGHT: i32 = 43;
    const D2_ICON: i32 = 43;
    const D2_LABEL: i32 = 64;

    let theme = Theme::default();
    let cs = &theme.colors;
    let text = TextRenderer::load(&FontConfig::default());
    let px = theme.fonts.size_px;
    let (shut, open) = icons();

    let mut docs = Node::new("My Documents", true);
    docs.children = vec![Node::new("Downloads", false)];
    docs.expanded = true;
    let mut desktop = Node::new("Desktop", true);
    desktop.children = vec![docs];
    desktop.expanded = true;
    let mut tv = TreeView {
        roots: vec![desktop],
        selected: Some(vec![0, 0]),
        style: Style::Explorer,
        ..Default::default()
    };
    fn paint(n: &mut Node, shut: &Canvas, open: &Canvas) {
        n.icon = Some(shut.clone());
        n.open_icon = Some(open.clone());
        for ch in &mut n.children {
            paint(ch, shut, open);
        }
    }
    paint(&mut tv.roots[0], &shut, &open);

    let lay = layout(
        &tv,
        &text,
        px,
        Rect::new(0, 0, 200, 300),
        &ControlMetrics::CLASSIC,
    );
    let mut c = Canvas::new(200, 300);
    draw(&mut c, &text, &classic_paint(cs, 1), &tv, &lay, None, px);
    let b = lay.body;
    let band = |k: i32| (b.y + k * ROW_H, b.y + (k + 1) * ROW_H);
    let first = |argb: u32, k: i32, x0: i32, x1: i32| {
        let (y0, y1) = band(k);
        (x0..x1).find(|&x| (y0..y1).any(|y| c.get(x, y) == argb))
    };
    let any = |argb: u32, k: i32, x0: i32, x1: i32| first(argb, k, x0, x1).is_some();

    let icon_shut = Color::rgb(255, 224, 123).argb();
    let icon_open = Color::rgb(174, 218, 255).argb();
    assert_eq!(
        first(icon_open, 0, b.x, b.right()),
        Some(b.x + ROOT_ICON - BODY_X)
    );
    assert!(
        !any(LINE_GRAY.argb(), 0, b.x, b.right()),
        "a root has no expander box and Explorer draws no lines"
    );
    let mut scratch = Canvas::new(200, ROW_H as u32);
    scratch.fill_rect(scratch.bounds(), cs.window);
    let origin = Rect::new(0, 0, 200, ROW_H);
    text.draw(
        &mut scratch,
        origin,
        "Desktop",
        false,
        cs.button_text,
        Align::Left,
        px,
    );
    let bearing = (0..200)
        .find(|&x| (0..ROW_H).any(|y| scratch.get(x, y) == cs.button_text.argb()))
        .expect("the label inks something");
    assert_eq!(
        first(cs.button_text.argb(), 0, b.x, b.right()),
        Some(b.x + ROOT_LABEL - BODY_X + bearing)
    );

    let (y0, _) = band(1);
    let mid = y0 + ROW_H / 2;
    assert_eq!(
        first(LINE_GRAY.argb(), 1, b.x, b.right()),
        Some(b.x + BOX_L - BODY_X)
    );
    assert_eq!(c.get(b.x + BOX_R - BODY_X, mid), LINE_GRAY.argb());
    assert_eq!(
        first(icon_open, 1, b.x, b.right()),
        Some(b.x + D1_ICON - BODY_X)
    );
    assert_eq!(
        first(cs.hilight.argb(), 1, b.x, b.right()),
        Some(b.x + D1_HILIGHT - BODY_X)
    );

    assert_eq!(
        first(icon_shut, 2, b.x, b.right()),
        Some(b.x + D2_ICON - BODY_X)
    );
    assert!(
        !any(LINE_GRAY.argb(), 2, b.x, b.right()),
        "no lines run down to a leaf"
    );
    assert_eq!(
        first(cs.button_text.argb(), 2, b.x, b.right()),
        Some(b.x + D2_LABEL - BODY_X + bearing),
        "\"Downloads\" and \"Desktop\" both start on a D"
    );
}

/// `Style::Picker`'s pixels, body-relative: "Desktop" inks its icon from x
/// 3 with no box and its label from x 24; "My Documents" at depth 1 has
/// its 9 px box at x 7..15, icon from x 22, band from x 41, label from 43.
#[test]
fn picker_style_lands_on_its_pixels() {
    if !crate::text::test_tahoma() {
        return;
    }
    let theme = Theme::default();
    let cs = &theme.colors;
    let text = TextRenderer::load(&FontConfig::default());
    let px = theme.fonts.size_px;
    let (shut, open) = icons();
    let mut docs = Node::new("My Documents", true);
    docs.icon = Some(shut.clone());
    let mut stuff = Node::new("Stuff", false);
    stuff.icon = Some(shut.clone());
    let mut desktop = Node::new("Desktop", true);
    desktop.icon = Some(shut.clone());
    desktop.open_icon = Some(open.clone());
    desktop.children = vec![docs, stuff];
    desktop.expanded = true;
    let tv = TreeView {
        roots: vec![desktop],
        selected: Some(vec![0, 0]),
        style: Style::Picker,
        ..Default::default()
    };
    let lay = layout(
        &tv,
        &text,
        px,
        Rect::new(0, 0, 200, 100),
        &ControlMetrics::CLASSIC,
    );
    let mut c = Canvas::new(200, 100);
    draw(&mut c, &text, &classic_paint(cs, 1), &tv, &lay, None, px);
    let b = lay.body;
    let first = |argb: u32, k: i32| {
        (b.x..b.right())
            .find(|&x| (b.y + k * ROW_H..b.y + (k + 1) * ROW_H).any(|y| c.get(x, y) == argb))
            .map(|x| x - b.x)
    };
    let icon_open = Color::rgb(174, 218, 255).argb();
    let icon_shut = Color::rgb(255, 224, 123).argb();
    assert_eq!(first(icon_open, 0), Some(3), "the root's icon");
    assert_eq!(
        first(LINE_GRAY.argb(), 0),
        None,
        "no box or line on the root"
    );
    assert_eq!(
        first(cs.button_text.argb(), 0),
        Some(24),
        "the root's label"
    );
    assert_eq!(first(LINE_GRAY.argb(), 1), Some(7), "the box's left edge");
    assert_eq!(
        c.get(b.x + 15, b.y + ROW_H + 8),
        LINE_GRAY.argb(),
        "its right edge"
    );
    assert_eq!(first(icon_shut, 1), Some(22), "the depth 1 icon");
    assert_eq!(first(cs.hilight.argb(), 1), Some(41), "the band");
    let (y0, y1) = (b.y + ROW_H, b.y + 2 * ROW_H);
    let label = (b.x + 41..b.right())
        .find(|&x| (y0..y1).any(|y| c.get(x, y) == cs.hilight_text.argb()))
        .map(|x| x - b.x);
    assert_eq!(label, Some(43), "the label");
    assert_eq!(
        first(LINE_GRAY.argb(), 2),
        None,
        "no line runs down to a leaf"
    );
}

/// A picked "My Documents" at depth 1: the band covers the 17 px row from 2 px before the label to 5 px past its text, capitals 5 px below the top, in `Plain`'s 18 px row as in 17 px ones.
#[test]
fn the_band_runs_5_px_past_the_label_and_the_label_sits_5_px_down() {
    let theme = Theme::default();
    let cs = &theme.colors;
    let text = TextRenderer::load(&FontConfig::default());
    let px = theme.fonts.size_px;
    for style in [Style::Explorer, Style::Picker, Style::Classic, Style::Plain] {
        let mut desktop = Node::new("Desktop", true);
        desktop.children = vec![Node::new("My Documents", false)];
        desktop.expanded = true;
        let tv = TreeView {
            roots: vec![desktop],
            selected: Some(vec![0, 0]),
            style,
            ..Default::default()
        };
        let lay = layout(
            &tv,
            &text,
            px,
            Rect::new(0, 0, 200, 100),
            &ControlMetrics::CLASSIC,
        );
        let mut c = Canvas::new(200, 100);
        draw(&mut c, &text, &classic_paint(cs, 1), &tv, &lay, None, px);
        let b = lay.body;
        let row_h = lay.row_h;
        let (y0, y1) = (b.y + row_h, b.y + 2 * row_h);
        let lx = b.x + tv.label_x(1);
        let w = text.measure("My Documents", false, px);
        let band: Vec<i32> = (b.x..b.right())
            .filter(|&x| c.get(x, y0) == cs.hilight.argb())
            .collect();
        assert_eq!(
            (band[0], *band.last().unwrap()),
            (lx - 2, lx + w + 4),
            "{style:?}: the band's columns"
        );
        assert_eq!(
            c.get(lx, y1 - 1),
            cs.hilight.argb(),
            "{style:?}: the band's last row"
        );
        let ink_top = (y0..y1)
            .find(|&y| (lx..lx + w).any(|x| c.get(x, y) == cs.hilight_text.argb()))
            .expect("the label inks");
        assert_eq!(ink_top - y0, 5, "{style:?}: the capitals' top");
    }
}

/// `Style::Classic` keeps its lines and root expander column; the `Style::Explorer` geometry is opt-in.
#[test]
fn the_classic_style_keeps_its_lines_and_root_box() {
    let theme = Theme::default();
    let cs = &theme.colors;
    let text = TextRenderer::load(&FontConfig::default());
    let px = theme.fonts.size_px;
    let tv = tree();
    assert_eq!(tv.style, Style::Classic, "the default");
    let lay = layout(
        &tv,
        &text,
        px,
        Rect::new(0, 0, 200, 300),
        &ControlMetrics::CLASSIC,
    );
    let mut c = Canvas::new(200, 300);
    draw(&mut c, &text, &classic_paint(cs, 1), &tv, &lay, None, px);
    let b = lay.body;
    let mid = b.y + ROW_H / 2;
    assert_eq!(c.get(b.x + BOX_X, mid - 4), LINE_GRAY.argb());
    let (y0, y1) = (b.y + 3 * ROW_H, b.y + 4 * ROW_H);
    assert!(
        (b.x + INDENT..b.x + 2 * INDENT).any(|x| (y0..y1).any(|y| c.get(x, y) == LINE_GRAY.argb()))
    );
}

/// A click in the shell's trees selects and opens a row, closing the old selection and its ancestors up to the first the new row sits under, and asks for a never-opened row's children. A connector-line tree only selects.
#[test]
fn a_click_opens_the_row_and_closes_the_old_branch() {
    let text = TextRenderer::load(&FontConfig::default());
    let mut tv = tree();
    tv.style = Style::Explorer;
    let lay = layout(
        &tv,
        &text,
        11,
        Rect::new(0, 0, 200, 300),
        &ControlMetrics::CLASSIC,
    );
    tv.roots[0].children[1].children[0].children = vec![Node::new("App", false)];
    tv.roots[0].children[1].children[0].expanded = true;
    tv.selected = Some(vec![0, 1, 0]);
    let row = |tv: &TreeView, path: &[usize]| {
        tv.visible()
            .iter()
            .position(|r| r.path == path)
            .expect("a visible row")
    };
    let i = row(&tv, &[0, 0]);
    assert_eq!(tv.click_row(i, &lay), Some(vec![0, 0]), "fill it");
    assert_eq!(tv.selected, Some(vec![0, 0]));
    let open = |tv: &TreeView, p: &[usize]| tv.node(p).unwrap().expanded;
    assert!(open(&tv, &[0, 0]), "My Documents opens");
    assert!(!open(&tv, &[0, 1, 0]), "Programs closes");
    assert!(!open(&tv, &[0, 1]), "and Local Disk above it");
    assert!(open(&tv, &[0]), "the Desktop holds both");
    tv.roots[0].children[0].children = vec![Node::new("Music", true)];
    let i = row(&tv, &[0, 0, 0]);
    assert_eq!(tv.click_row(i, &lay), Some(vec![0, 0, 0]));
    assert!(open(&tv, &[0, 0]), "the parent stays open");
    let i = row(&tv, &[0, 2]);
    assert_eq!(tv.click_row(i, &lay), None);
    assert_eq!(tv.selected, Some(vec![0, 2]));
    assert!(!open(&tv, &[0, 0]), "the old branch closed");

    let mut classic = tree();
    classic.selected = Some(vec![0, 1]);
    assert_eq!(classic.click_row(1, &lay), None);
    assert_eq!(classic.selected, Some(vec![0, 0]));
    assert!(!open(&classic, &[0, 0]), "a Classic row does not open");
    assert!(open(&classic, &[0, 1]), "nor the old one close");
}

/// A click that closes a long branch above the view scrolls back so the view ends at the last row.
#[test]
fn a_click_that_closes_a_long_branch_keeps_the_view_full() {
    let mut b = Node::new("B", true);
    b.expanded = true;
    b.children = (0..25)
        .map(|k| Node::new(&format!("b{k}"), false))
        .collect();
    let mut a = Node::new("A", true);
    a.expanded = true;
    a.children = vec![b, Node::new("C", false)];
    let mut tv = TreeView::new(vec![a], Style::Explorer);
    let lay = layout_without_text(&tv, Rect::new(0, 0, 200, 340), &ControlMetrics::CLASSIC);
    assert_eq!(lay.rows, 20);
    tv.top = 8;
    tv.selected = Some(vec![0, 0]);
    let c = tv.visible().iter().position(|r| r.path == [0, 1]).unwrap();
    assert_eq!(tv.click_row(c, &lay), None);
    assert!(!tv.node(&[0, 0]).unwrap().expanded, "B closes");
    assert_eq!(tv.visible().len(), 3);
    assert_eq!(tv.top, 0, "A, B and C all show");
}

/// The pointer on a row's icon or label makes it hot; past the label, off the rows or in a non-tracking tree, nothing is.
#[test]
fn the_icon_or_label_under_the_pointer_is_hot() {
    let text = TextRenderer::load(&FontConfig::default());
    let mut tv = tree();
    tv.style = Style::Picker;
    let lay = layout(
        &tv,
        &text,
        11,
        Rect::new(0, 0, 200, 300),
        &ControlMetrics::CLASSIC,
    );
    let b = lay.body;
    let y = b.y + ROW_H + 8;
    let end = b.x + 43 + text.measure("My Documents", false, 11);
    assert!(tv.hover(&lay, &text, 11, b.x + 23, y), "on the icon");
    assert_eq!(tv.hot, Some(1));
    assert!(!tv.hover(&lay, &text, 11, end - 1, y), "still row 1");
    assert!(tv.hover(&lay, &text, 11, end, y), "past the label");
    assert_eq!(tv.hot, None);
    tv.hover(&lay, &text, 11, b.x + 23, y);
    assert!(tv.unhover());
    tv.style = Style::Classic;
    tv.hover(&lay, &text, 11, b.x + 50, y);
    assert_eq!(tv.hot, None, "a connector-line tree does not track");
}

/// A scroll with the pointer at rest takes the hot row away: nothing is hot until it moves.
#[test]
fn a_scroll_leaves_no_row_hot() {
    let text = TextRenderer::load(&FontConfig::default());
    let mut b = Node::new("B", true);
    b.expanded = true;
    b.children = (0..30)
        .map(|k| Node::new(&format!("b{k}"), false))
        .collect();
    let mut tv = TreeView::new(vec![b], Style::Explorer);
    let lay = layout(
        &tv,
        &text,
        11,
        Rect::new(0, 0, 200, 120),
        &ControlMetrics::CLASSIC,
    );
    let y = lay.body.y + ROW_H + 8;
    assert!(tv.hover(&lay, &text, 11, lay.body.x + 30, y));
    assert!(tv.scroll(1, &lay));
    assert_eq!(tv.hot, None);
}

#[test]
fn a_revealed_row_scrolls_in_as_the_last_row_and_stays_unpicked() {
    let text = TextRenderer::load(&FontConfig::default());
    let mut b = Node::new("B", true);
    b.expanded = true;
    b.children = (0..30)
        .map(|k| Node::new(&format!("b{k}"), false))
        .collect();
    let mut tv = TreeView::new(vec![b], Style::Explorer);
    let lay = layout(
        &tv,
        &text,
        11,
        Rect::new(0, 0, 200, 120),
        &ControlMetrics::CLASSIC,
    );
    assert!(!tv.reveal(1, &lay), "a row in view");
    assert!(tv.reveal(20, &lay));
    assert_eq!(tv.top + lay.rows, 21);
    assert_eq!(tv.selected, None);
    assert!(tv.reveal(0, &lay));
    assert_eq!(tv.top, 0);
    assert!(!tv.reveal(31, &lay), "past the last row");
}

#[test]
fn a_long_tree_publishes_the_rows_in_view_under_their_ancestors() {
    use crate::access::{Published, ROOT, Tree};
    let text = TextRenderer::load(&FontConfig::default());
    let mut b = Node::new("B", true);
    b.expanded = true;
    b.children = (0..10_000)
        .map(|k| Node::new(&format!("b{k}"), false))
        .collect();
    let mut tv = TreeView::new(vec![b], Style::Explorer);
    tv.selected = Some(vec![0, 9_000]);
    let lay = layout(
        &tv,
        &text,
        11,
        Rect::new(0, 0, 200, 120),
        &ControlMetrics::CLASSIC,
    );
    tv.top = 101;
    let mut t = Tree::window("", (200, 120));
    access(&tv, &lay, &text, 11, &mut t, ROOT, "tree");
    let list = t.get(ROOT).unwrap().children()[0];
    let node = t.get(list).unwrap();
    assert_eq!(node.scroll_y(), Some(101.0 * f64::from(lay.row_h)));
    let top = node.children();
    assert_eq!(top.len(), 1, "the root row B");
    let b = t.get(top[0]).unwrap();
    assert_eq!(b.label(), Some("B"));
    assert_eq!(b.position_in_set(), Some(1));
    let items = b.children().to_vec();
    assert_eq!(
        items.len(),
        lay.rows + 2,
        "the rows in view, the cut one and the selection"
    );
    let first = t.get(items[0]).unwrap();
    assert_eq!(first.label(), Some("b100"));
    assert_eq!(
        (first.position_in_set(), first.size_of_set()),
        (Some(101), Some(10_000))
    );
    assert_eq!(first.row_index(), Some(101));
    let picked = *items.last().unwrap();
    assert_eq!(t.get(picked).unwrap().label(), Some("b9000"));
    let mut p = Published::default();
    p.update(t);
    assert_eq!(p.reveal(picked).map(|r| r.row), Some(9_001));
}

#[test]
fn a_tree_item_counts_among_its_siblings_and_reveals_by_its_visible_row() {
    use crate::access::{Published, ROOT, Tree};
    let text = TextRenderer::load(&FontConfig::default());
    let mut b = Node::new("B", true);
    b.expanded = true;
    b.children = (0..50).map(|k| Node::new(&format!("c{k}"), true)).collect();
    b.children[10].expanded = true;
    b.children[10].children = (0..30)
        .map(|k| Node::new(&format!("g{k}"), false))
        .collect();
    let mut tv = TreeView::new(vec![Node::new("A", false), b], Style::Explorer);
    tv.selected = Some(vec![1, 10, 20]);
    let lay = layout(
        &tv,
        &text,
        11,
        Rect::new(0, 0, 200, 120),
        &ControlMetrics::CLASSIC,
    );
    let mut t = Tree::window("", (200, 120));
    access(&tv, &lay, &text, 11, &mut t, ROOT, "tree");
    let list = t.get(ROOT).unwrap().children()[0];
    let roots = t.get(list).unwrap().children().to_vec();
    let b = t.get(roots[1]).unwrap();
    assert_eq!((b.position_in_set(), b.size_of_set()), (Some(2), Some(2)));
    let c10 = *b.children().last().unwrap();
    let c = t.get(c10).unwrap();
    assert_eq!(c.label(), Some("c10"));
    assert_eq!((c.position_in_set(), c.size_of_set()), (Some(11), Some(50)));
    let g20 = c.children()[0];
    let g = t.get(g20).unwrap();
    assert_eq!(g.label(), Some("g20"));
    assert_eq!(g.level(), Some(3));
    assert_eq!(
        (g.position_in_set(), g.size_of_set()),
        (Some(21), Some(30)),
        "the 21st of c10's 30 children"
    );
    let mut p = Published::default();
    p.update(t);
    assert_eq!(
        p.reveal(g20).map(|r| r.row),
        Some(33),
        "A, B, c0..c10, then g0..g20 in the visible order"
    );
}

/// A hot label is drawn in `hot_light` and underlined 2 px below its ink's lowest row, as wide as its text; on the selection's band the underline takes the band's ink.
#[test]
fn a_hot_label_is_underlined_in_the_hot_colour() {
    let theme = Theme::default();
    let cs = &theme.colors;
    let text = TextRenderer::load(&FontConfig::default());
    let px = theme.fonts.size_px;
    let mut tv = tree();
    tv.style = Style::Picker;
    let lay = layout(
        &tv,
        &text,
        px,
        Rect::new(0, 0, 200, 300),
        &ControlMetrics::CLASSIC,
    );
    let shot = |tv: &TreeView| {
        let mut c = Canvas::new(200, 300);
        draw(&mut c, &text, &classic_paint(cs, 1), tv, &lay, None, px);
        c
    };
    let b = lay.body;
    let (label, y) = (b.x + 43, b.y + 2 * ROW_H);
    let w = text.measure("Local Disk", false, px);
    let plain = shot(&tv);
    let l_w = text.advance('L', false, px);
    let l_bottom = (y..y + ROW_H)
        .filter(|&yy| (label..label + l_w).any(|x| plain.get(x, yy) == cs.button_text.argb()))
        .max()
        .expect("the L inks");
    tv.hot = Some(2);
    let c = shot(&tv);
    let ul = l_bottom + 2;
    let hot = cs.hot_light.argb();
    assert!(
        (label..label + w).all(|x| c.get(x, ul) == hot),
        "the underline at y {ul}"
    );
    assert_eq!(c.get(label + w, ul), cs.window.argb(), "no further");
    assert_eq!(c.get(label - 1, ul), cs.window.argb());
    assert!(
        (y..ul).any(|yy| (label..label + l_w).any(|x| c.get(x, yy) == hot)),
        "the label inks in the hot colour"
    );
    assert!(
        !(y..y + ROW_H).any(|yy| (label..label + w).any(|x| c.get(x, yy) == cs.button_text.argb())),
        "and not in the plain one"
    );
    tv.selected = Some(vec![0, 1]);
    let c = shot(&tv);
    assert_eq!(c.get(label, ul), cs.hilight_text.argb(), "on the band");
}

/// The rename box over "Programs" (picker style, depth 2, label at
/// body.x + 62): a 1 px black ring the row's 17 px tall from 5 px left of the
/// label, the name on the highlight's 13 rows from the box's third, the
/// caret after it and 15 px of white to the right edge.
#[test]
fn the_rename_box_rings_the_row_and_keeps_the_labels_place() {
    let theme = Theme::default();
    let cs = &theme.colors;
    let text = TextRenderer::load(&FontConfig::default());
    let px = theme.fonts.size_px;
    let mut tv = tree();
    tv.style = Style::Picker;
    let lay = layout(
        &tv,
        &text,
        px,
        Rect::new(0, 0, 200, 300),
        &ControlMetrics::CLASSIC,
    );
    let shot = |tv: &TreeView| {
        let mut c = Canvas::new(200, 300);
        draw(&mut c, &text, &classic_paint(cs, 1), tv, &lay, None, px);
        c
    };
    let plain = shot(&tv);
    tv.begin_edit(vec![0, 1, 0]);
    let c = shot(&tv);
    let b = lay.body;
    let (label, y) = (b.x + 62, b.y + 3 * ROW_H);
    let w = text.measure("Programs", false, px);
    let r = tv.edit_rect(&lay, &text, px).expect("the box is in sight");
    assert_eq!(r, Rect::new(label - 5, y, 5 + w + 15, ROW_H), "the box");
    let black = cs.window_frame.argb();
    assert_eq!(c.get(r.x, y + 8), black, "left ring");
    assert_eq!(c.get(r.right() - 1, y + 8), black, "right ring");
    assert_eq!(c.get(r.x + 3, y), black, "top ring on the row's top");
    assert_eq!(c.get(r.x + 3, y + ROW_H - 1), black, "bottom ring");
    assert_eq!(c.get(label - 1, y + 2), cs.window.argb(), "white before");
    assert_eq!(c.get(label, y + 2), cs.hilight.argb(), "highlight top");
    assert_eq!(c.get(label, y + 14), cs.hilight.argb(), "highlight bottom");
    assert_eq!(c.get(label, y + 15), cs.window.argb(), "white under it");
    assert_eq!(c.get(label + w, y + 8), black, "the caret after the name");
    assert_eq!(c.get(r.right() - 3, y + 8), cs.window.argb());
    let rows = |c: &Canvas, ink: u32| {
        (y + 2..y + 15)
            .filter(|&yy| (label..label + w).any(|x| c.get(x, yy) == ink))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        rows(&c, cs.hilight_text.argb()),
        rows(&plain, cs.button_text.argb()),
        "the name inks the label's rows"
    );
}

/// Scrolled to "Programs", the first row still carries "Local Disk"'s line down to "Recycle Bin", 27 px into the body.
#[test]
fn a_scrolled_classic_tree_keeps_its_ancestors_lines() {
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let px = theme.fonts.size_px;
    let mut tv = tree();
    tv.top = 3;
    let lay = layout(
        &tv,
        &text,
        px,
        Rect::new(0, 0, 200, 300),
        &ControlMetrics::CLASSIC,
    );
    let mut c = Canvas::new(200, 300);
    draw(
        &mut c,
        &text,
        &classic_paint(&theme.colors, 1),
        &tv,
        &lay,
        None,
        px,
    );
    let b = lay.body;
    let x = b.x + INDENT + BOX_X + BOX / 2;
    assert!(
        (b.y..b.y + ROW_H).any(|y| c.get(x, y) == LINE_GRAY.argb()),
        "a dotted line at x {x} through the first row"
    );
}

/// Writes target/treeview-preview.png: `Style::Explorer` left, `Style::Classic` right.
#[test]
#[ignore]
fn render_treeview() {
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let px = theme.fonts.size_px;
    let (icon, open) = icons();
    let mut c = Canvas::new(410, 120);
    c.fill_rect(c.bounds(), theme.colors.button_face);
    for (col, style) in [Style::Explorer, Style::Classic].into_iter().enumerate() {
        let mut tv = tree();
        tv.style = style;
        for n in &mut tv.roots[0].children {
            n.icon = Some(icon.clone());
            n.open_icon = Some(open.clone());
        }
        tv.roots[0].children[1].label = "Local Disk: a name long enough to overflow".into();
        tv.selected = Some(vec![0, 0]);
        tv.focus_ring = true;
        tv.drop_hilight = Some(3);
        let r = Rect::new(5 + col as i32 * 205, 5, 195, 110);
        let lay = layout(&tv, &text, px, r, &ControlMetrics::CLASSIC);
        draw(
            &mut c,
            &text,
            &classic_paint(&theme.colors, 1),
            &tv,
            &lay,
            None,
            px,
        );
    }
    crate::raster::write_preview(&c, "treeview-preview.png");
}

#[test]
fn a_blue_trees_lit_bar_part_draws_hot() {
    use crate::classic::widgets::{Axis, ScrollHit};
    let theme = crate::interfaces::theme_for(crate::interface::Interface::Blue);
    let cm = theme.look.controls();
    let text = TextRenderer::load(&FontConfig::default());
    let mut tv = TreeView {
        roots: (0..40)
            .map(|i| Node::new(&format!("Folder {i}"), false))
            .collect(),
        ..Default::default()
    };
    let r = Rect::new(0, 0, 150, 120);
    let lay = layout(&tv, &text, 11, r, &cm);
    let sb = lay.scrollbar.expect("40 rows overflow the tree");
    let down = tv.hit(&lay, sb.x + 8, sb.bottom() - 3).bar();
    assert_eq!(down, Some((Axis::Vertical, ScrollHit::Forward)));
    tv.bars.motion(down);
    let mut c = Canvas::new(150, 120);
    draw(&mut c, &text, &theme.paint(), &tv, &lay, tv.bars.lit(), 11);
    assert_eq!(
        c.get(sb.x + 8, sb.bottom() - 17 + 2),
        crate::look::styled::fixture::blue_pixel(
            crate::style::Part::ScrollArrowDown,
            crate::style::State::Hot,
            8,
            2
        ),
        "the hot down arrow"
    );
}
