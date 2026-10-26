use super::details::details_listview;
use super::*;
use crate::access::accesskit::Role;
use crate::classic::widgets::FIELD_PAD;
use crate::controls::iconview::{self, IconMove};
use crate::look::{Paint, classic_paint};
use crate::pointer::WHEEL_ROWS;
use crate::selection::Mods;
use crate::text::{Align, TextRenderer};
use crate::textedit::TextEdit;
use crate::theme::{ColorScheme, FontConfig, Theme};

#[test]
fn a_thumbnail_fits_the_frame_at_the_windows_scale() {
    let big = Canvas::from_pixels(256, 192, vec![0xff00_00ff; 256 * 192]);
    let one = frame_thumb(big.clone(), (800, 600), 1);
    assert_eq!((one.width, one.height, one.scale()), (94, 71, 1));
    assert_eq!(one.get(10, 10), 0xff00_00ff);
    let two = frame_thumb(big, (800, 600), 2);
    assert_eq!((two.width, two.height, two.scale()), (94, 71, 2));
    assert_eq!(two.device_size(), (188, 142));
    let small = Canvas::from_pixels(94, 71, vec![0xff00_00ff; 94 * 71]);
    assert_eq!(frame_thumb(small, (800, 600), 2).device_size(), (188, 142));
    let tiny = Canvas::from_pixels(16, 8, vec![1; 128]);
    let t = frame_thumb(tiny.clone(), (16, 8), 1);
    assert_eq!(t, tiny);
    assert_eq!(frame_thumb(tiny, (16, 8), 3).device_size(), (48, 24));
}

pub(crate) fn rend() -> (TextRenderer, u32) {
    (
        TextRenderer::load(&FontConfig::default()),
        Theme::default().fonts.size_px,
    )
}

pub(crate) fn entry(name: &str) -> Entry {
    Entry {
        name: name.into(),
        kind: "File".into(),
        ..Default::default()
    }
}

pub(crate) fn view(n: usize) -> ShellView {
    ShellView {
        entries: (0..n).map(|i| entry(&format!("item{i}"))).collect(),
        ..Default::default()
    }
}

pub(crate) fn square(n: u32, color: Color) -> Canvas {
    let mut c = Canvas::new(n, n);
    c.fill_rect(Rect::new(0, 0, n as i32, n as i32), color);
    c
}

/// The cache hands back the same layout while nothing `layout` reads changes, and a fresh one equal to `layout`'s after any input does.
#[test]
fn layout_cache_rebuilds_only_when_an_input_changes() {
    let cache = LayoutCache::default();
    let mut sv = view(40);
    sv.mode = Mode::Icons;
    let r = Rect::new(0, 0, 300, 200);
    let first = cache.get(&sv, r, &ControlMetrics::CLASSIC);
    sv.selection.set_only(Some(3));
    sv.marquee = Some(Rect::new(1, 1, 5, 5));
    assert!(
        Rc::ptr_eq(&first, &cache.get(&sv, r, &ControlMetrics::CLASSIC)),
        "a selection or marquee change reuses the cached layout"
    );
    let changes: [fn(&mut ShellView, &mut Rect); 7] = [
        |_, r| r.width += 1,
        |sv, _| sv.set_mode(Mode::Details),
        |sv, _| sv.entries.truncate(10),
        |sv, _| sv.top = 1,
        |sv, _| sv.left = 5,
        |sv, _| sv.columns[0].width += 10,
        |sv, _| {
            sv.groups = vec![Group {
                title: "A".into(),
                first: 0,
            }]
        },
    ];
    let mut r = r;
    for (n, change) in changes.iter().enumerate() {
        let before = cache.get(&sv, r, &ControlMetrics::CLASSIC);
        change(&mut sv, &mut r);
        let after = cache.get(&sv, r, &ControlMetrics::CLASSIC);
        assert!(
            !Rc::ptr_eq(&before, &after),
            "change {n} rebuilds the layout"
        );
        assert_eq!(
            *after,
            layout(&sv, r, &ControlMetrics::CLASSIC),
            "change {n} caches layout's answer"
        );
    }
    let thick = ControlMetrics {
        scroll: 17,
        ..ControlMetrics::CLASSIC
    };
    let before = cache.get(&sv, r, &ControlMetrics::CLASSIC);
    let after = cache.get(&sv, r, &thick);
    assert!(!Rc::ptr_eq(&before, &after), "a look's sizes rebuild it");
    assert_eq!(*after, layout(&sv, r, &thick));
}

/// A List layout built before `entries` shrank draws the remaining entries and skips the stale cells.
#[test]
fn list_draw_skips_cells_past_the_entries() {
    let (text, px) = rend();
    let mut sv = view(6);
    let r = Rect::new(0, 0, 400, 200);
    let lay = layout(&sv, r, &ControlMetrics::CLASSIC);
    sv.entries.truncate(2);
    let mut c = Canvas::new(400, 200);
    draw(
        &mut c,
        &text,
        &classic_paint(&ColorScheme::default(), 1),
        &sv,
        &lay,
        None,
        px,
    );
}

/// List flows down a column then into the next, at the 17 px row / 196 px column pitch.
#[test]
fn list_mode_flows_column_major() {
    const EXPECTED_ROW_H: i32 = 17;
    const EXPECTED_COL_W: i32 = 196;
    let sv = view(40);
    // A body exactly 3 rows tall (17 * 3 plus the field's 3 px inset each side), so a fourth entry wraps to x + 196.
    let r = Rect::new(0, 0, 3000, 3 * EXPECTED_ROW_H + 2 * FIELD_PAD);
    let lay = layout(&sv, r, &ControlMetrics::CLASSIC);
    assert_eq!(lay.cells[0].y, lay.cells[1].y - EXPECTED_ROW_H);
    assert_eq!(
        lay.cells[3].y, lay.cells[0].y,
        "the fourth wraps to a new column"
    );
    assert_eq!(lay.cells[3].x, lay.cells[0].x + EXPECTED_COL_W);
}

/// More entries than fit gets a horizontal scrollbar; a view that holds them all gets none. List never scrolls vertically.
#[test]
fn list_mode_scrolls_horizontally_only_when_it_overflows() {
    let narrow = layout(
        &view(2),
        Rect::new(0, 0, 400, 200),
        &ControlMetrics::CLASSIC,
    );
    assert!(narrow.hscroll.is_none(), "two entries fit");
    assert!(narrow.vscroll.is_none());
    let full = layout(
        &view(400),
        Rect::new(0, 0, 400, 200),
        &ControlMetrics::CLASSIC,
    );
    assert!(full.hscroll.is_some());
    assert!(full.vscroll.is_none(), "List mode never scrolls vertically");
}

/// Right steps into the next column, Down within one, and neither walks off the end.
#[test]
fn arrows_walk_the_columns() {
    let mut sv = view(6);
    let r = Rect::new(0, 0, 400, 3 * ROW_H + 2 * FIELD_PAD);
    let lay = layout(&sv, r, &ControlMetrics::CLASSIC);
    sv.selection.set_only(Some(0));
    assert!(step(&mut sv, IconMove::Down, &lay, Mods::default()));
    assert_eq!(sv.selection.focus, Some(1));
    assert!(step(&mut sv, IconMove::Right, &lay, Mods::default()));
    assert_eq!(sv.selection.focus, Some(4));
    sv.selection.set_only(Some(5));
    assert!(!step(&mut sv, IconMove::Right, &lay, Mods::default()));
    assert_eq!(sv.selection.focus, Some(5), "the last column stays put");
}

/// Typing a letter jumps to the next entry starting with it, wrapping round.
#[test]
fn type_ahead_jumps_to_the_next_match() {
    let mut sv = ShellView {
        entries: ["alpha", "beta", "bravo", "delta", "École"]
            .iter()
            .map(|n| entry(n))
            .collect(),
        ..view(0)
    };
    sv.selection.set_only(Some(0));
    assert!(type_ahead(&mut sv, 'é'));
    assert_eq!(sv.selection.focus, Some(4));
    sv.selection.set_only(Some(0));
    assert!(type_ahead(&mut sv, 'b'));
    assert_eq!(sv.selection.focus, Some(1));
    assert!(type_ahead(&mut sv, 'b'));
    assert_eq!(
        sv.selection.focus,
        Some(2),
        "a second press goes to the next one"
    );
    assert!(!type_ahead(&mut sv, 'z'));
    assert_eq!(
        sv.selection.focus,
        Some(2),
        "no match leaves the selection alone"
    );
}

/// List and Details draw an entry's 16 px icon at 100% and its 32 px one at 200%, in a
/// column grown to 32 px with the name starting past it.
#[test]
fn list_and_details_icons_grow_to_32_px_at_200_percent() {
    let (text, _) = rend();
    let (red, blue) = (Color::rgb(255, 0, 0), Color::rgb(0, 0, 255));
    for mode in [Mode::List, Mode::Details] {
        for (pct, side) in [(100, 16), (200, 32)] {
            let theme = Theme::default().with_text_size(pct);
            let cs = &theme.colors;
            let mut sv = view(1);
            sv.mode = mode;
            sv.entries[0].name = "WWW".into();
            sv.entries[0].icon = Some(Rc::new(square(16, red)));
            sv.entries[0].icon32 = Some(Rc::new(square(32, blue)));
            let r = Rect::new(0, 0, 400, 120);
            let lay = layout(&sv, r, &theme.controls());
            let mut c = Canvas::new(400, 120);
            let p = Paint {
                text: theme.text,
                ..classic_paint(cs, 1)
            };
            draw(&mut c, &text, &p, &sv, &lay, None, theme.fonts.size_px);
            let ink = if side == 16 { red } else { blue };
            let at: Vec<(i32, i32)> = (0..120)
                .flat_map(|y| (0..400).map(move |x| (x, y)))
                .filter(|&(x, y)| c.get(x, y) == ink.argb())
                .collect();
            let xs = at.iter().map(|p| p.0);
            let ys = at.iter().map(|p| p.1);
            let (left, right) = (xs.clone().min().unwrap(), xs.max().unwrap());
            let (top, bottom) = (ys.clone().min().unwrap(), ys.max().unwrap());
            assert_eq!(
                (right - left + 1, bottom - top + 1),
                (side, side),
                "{mode:?} {pct}%: the icon's size"
            );
            let label = (right + 1..400)
                .find(|&x| (top..=bottom).any(|y| c.get(x, y) == cs.window_text.argb()));
            assert!(
                label.is_some_and(|x| x > right),
                "{mode:?} {pct}%: the name starts past the icon, got {label:?}"
            );
        }
    }
}

/// The selected entry's label is filled with the hilight colour and drawn in hilight text; nothing else is.
#[test]
fn the_selected_entry_is_filled_with_the_hilight() {
    const EXPECTED_ICON_PX: i32 = 16;
    let theme = Theme::default();
    let cs = &theme.colors;
    let (text, px) = rend();
    let mut sv = view(4);
    sv.selection.set_only(Some(1));
    let r = Rect::new(0, 0, 400, 100);
    let lay = layout(&sv, r, &ControlMetrics::CLASSIC);
    let mut c = Canvas::new(400, 100);
    draw(&mut c, &text, &classic_paint(cs, 1), &sv, &lay, None, px);
    let sel = lay.cells[1];
    assert_eq!(
        c.get(sel.x + EXPECTED_ICON_PX + 2, sel.y + 2),
        cs.hilight.argb()
    );
    let other = lay.cells[0];
    assert_eq!(
        c.get(other.x + EXPECTED_ICON_PX + 2, other.y + 2),
        cs.window.argb()
    );
}

#[test]
fn a_column_scrolled_part_way_off_draws_shifted_and_clipped_to_the_body() {
    let theme = Theme::default();
    let cs = &theme.colors;
    let (text, px) = rend();
    let mut sv = view(12);
    let red = Color::rgb(0xff, 0, 0);
    for e in &mut sv.entries {
        e.icon = Some(std::rc::Rc::new(square(16, red)));
    }
    let r = Rect::new(0, 0, 320, 80);
    let drawn = |sv: &ShellView| {
        let lay = layout(sv, r, &ControlMetrics::CLASSIC);
        let mut c = Canvas::new(320, 80);
        draw(&mut c, &text, &classic_paint(cs, 1), sv, &lay, None, px);
        (c, lay.body)
    };
    let (whole, body) = drawn(&sv);
    // 10 px puts the icon across the frame, 30 px cuts into the label.
    for left in [10, 30] {
        sv.left = left;
        let (scrolled, _) = drawn(&sv);
        for y in body.y..body.bottom() {
            for x in body.x..body.x + COL_W - left {
                assert_eq!(
                    scrolled.get(x, y),
                    whole.get(x + left, y),
                    "({x}, {y}): the first column moved {left} px left"
                );
            }
            for x in 0..body.x {
                assert_ne!(
                    scrolled.get(x, y),
                    red.argb(),
                    "({x}, {y}): no icon on the frame"
                );
            }
        }
    }
}

/// Stepping to a cell beyond the visible columns scrolls it into view.
/// `shellview::step` must follow in px, since List scrolls by column pitch, not row count.
#[test]
fn stepping_past_the_visible_columns_scrolls_to_follow() {
    let mut sv = view(6);
    // One column wide, three rows tall once the scrollbar takes its thickness: column 1 (rows 3..6) is off screen.
    let r = Rect::new(
        0,
        0,
        COL_W + 2 * FIELD_PAD,
        3 * ROW_H + ControlMetrics::CLASSIC.scroll + 2 * FIELD_PAD,
    );
    let lay = layout(&sv, r, &ControlMetrics::CLASSIC);
    sv.selection.set_only(Some(2)); // column 0's last row, on screen
    assert!(step(&mut sv, IconMove::Right, &lay, Mods::default()));
    assert_eq!(sv.selection.focus, Some(5), "column 1's last row");
    assert!(
        sv.left > 0,
        "must scroll to bring the new selection into view"
    );

    // Re-laying out at the new scroll position must put the selected cell
    // inside the body: draw skips a cell with an empty intersection.
    let lay2 = layout(&sv, r, &ControlMetrics::CLASSIC);
    let cell = lay2.cells[5];
    assert!(cell.x >= lay2.body.x, "not scrolled off the left");
    assert!(
        cell.right() <= lay2.body.right(),
        "not scrolled off the right"
    );
}

/// A label too long for its 196 px column is ellipsized, not given a wider
/// column. Checks pixel-for-pixel that `draw` renders what `text::ellipsize`
/// computes for the 180 px label width (196 column - 16 icon).
#[test]
fn an_overrunning_label_is_ellipsized_to_the_fixed_column() {
    const EXPECTED_LABEL_W: i32 = 180; // 196 px column - 16 px icon
    let theme = Theme::default();
    let cs = &theme.colors;
    let (text, px) = rend();
    let mut sv = view(1);
    let long = "Software Distribution and Setup Archive";
    sv.entries[0].name = long.into();
    assert!(
        text.measure(long, false, px) > EXPECTED_LABEL_W,
        "the name must not already fit, or this test proves nothing"
    );
    let shown = text.ellipsize(long, EXPECTED_LABEL_W, false, px);
    assert!(shown.len() < long.len(), "the shown text is trimmed");
    assert!(shown.ends_with("..."), "an end ellipsis, not a bare cut");

    let r = Rect::new(0, 0, 400, 100);
    let lay = layout(&sv, r, &ControlMetrics::CLASSIC);
    let cell = lay.cells[0];

    let mut got = Canvas::new(400, 100);
    draw(&mut got, &text, &classic_paint(cs, 1), &sv, &lay, None, px);

    let mut want = Canvas::new(400, 100);
    want.fill_rect(Rect::new(0, 0, 400, 100), cs.window);
    let label = Rect::new(cell.x + 16, cell.y, EXPECTED_LABEL_W, cell.height);
    text.draw(
        &mut want,
        label,
        &shown,
        false,
        cs.window_text,
        Align::Left,
        px,
    );

    for y in cell.y..cell.bottom() {
        for x in cell.x..cell.right() {
            assert_eq!(
                got.get(x, y),
                want.get(x, y),
                "pixel ({x},{y}) differs: draw must show exactly the ellipsized text"
            );
        }
    }
}

/// A file's size rounds up to whole KB; a folder shows none.
#[test]
fn size_text_rounds_up_to_kb() {
    assert_eq!(size_text(0), "0 KB");
    assert_eq!(size_text(1), "1 KB");
    assert_eq!(size_text(1024), "1 KB");
    assert_eq!(size_text(1025), "2 KB");
    assert_eq!(size_text(6_500), "7 KB");
    assert_eq!(size_text(150_000), "147 KB");
    assert_eq!(size_text(1_024_000), "1,000 KB");
    assert_eq!(size_text(1_234_567), "1,206 KB");
    assert_eq!(size_text(12_345_678), "12,057 KB");
    assert_eq!(size_text(1 << 40), "1,073,741,824 KB");
}

#[test]
fn a_tile_names_a_folder_alone_and_a_file_with_its_kind_and_size() {
    let e = |is_dir, link, kind: &str, size| Entry {
        is_dir,
        link,
        kind: kind.into(),
        size,
        ..Default::default()
    };
    assert!(tile_detail(&e(true, false, "File Folder", 4096)).is_empty());
    let tile = |size| tile_detail(&e(false, false, "Text Document", size));
    assert_eq!(tile(2_048), ["Text Document", "2 KB"]);
    assert_eq!(tile(6_500)[1], "7 KB");
    assert_eq!(
        tile_detail(&e(true, true, "Shortcut", 4096)),
        ["Shortcut", "4 KB"]
    );
}

#[test]
fn tiles_writes_a_pictures_dimensions_over_its_kind() {
    assert_eq!(
        image_detail((800, 600), "JPEG Image"),
        ["800 x 600", "JPEG Image"]
    );
}

/// Details lays out over listview's grid: a row is its height and the first starts below the header.
#[test]
fn details_rows_sit_under_the_header() {
    const EXPECTED_HEADER_H: i32 = 17;
    const EXPECTED_ROW_H: i32 = 17;
    let mut sv = view(4);
    sv.mode = Mode::Details;
    let lay = layout(&sv, Rect::new(0, 0, 400, 200), &ControlMetrics::CLASSIC);
    assert_eq!(lay.cells[0].height, EXPECTED_ROW_H);
    assert_eq!(lay.cells[0].y, lay.body.y + EXPECTED_HEADER_H);
    assert_eq!(lay.cells[1].y, lay.cells[0].y + EXPECTED_ROW_H);
}

/// A folder shows no size and its type is "File Folder"; a shortcut to a folder shows its own size.
#[test]
fn a_folder_has_no_size_column() {
    let mut sv = view(0);
    sv.mode = Mode::Details;
    sv.entries = vec![
        Entry {
            name: "Docs".into(),
            is_dir: true,
            kind: "File Folder".into(),
            modified: "1/1/2003 9:00 AM".into(),
            ..Default::default()
        },
        Entry {
            name: "Docs link".into(),
            is_dir: true,
            link: true,
            size: 4,
            kind: "Shortcut".into(),
            ..Default::default()
        },
    ];
    use listview::Rows;
    let rows = DetailsRows(&sv.entries, &sv.fields);
    assert_eq!(rows.cell(0, 1), "", "a folder shows no size");
    assert_eq!(rows.cell(0, 2), "File Folder");
    assert_eq!(rows.cell(1, 1), "1 KB", "a shortcut to a folder is sized");
}

/// The Recycle Bin's columns: Name 178, Original Location 180, Date Deleted
/// 120, Size 96 right-aligned, Type 120, Date Modified 120; location and date from the entry's `folder` and `deleted`.
#[test]
fn bin_columns_show_where_and_when_an_item_was_deleted() {
    let cols = bin_columns();
    let shape: Vec<(&str, i32, bool)> = cols
        .iter()
        .map(|c| (c.title.as_str(), c.width, c.right))
        .collect();
    assert_eq!(
        shape,
        [
            ("Name", 178, false),
            ("Original Location", 180, false),
            ("Date Deleted", 120, false),
            ("Size", 96, true),
            ("Type", 120, false),
            ("Date Modified", 120, false),
        ]
    );
    let entries = vec![Entry {
        name: "notes.txt".into(),
        folder: "/home/u/Desktop".into(),
        deleted: "9/14/2026 3:43 PM".into(),
        size: 4096,
        ..Default::default()
    }];
    use listview::Rows;
    let fields = bin_fields();
    assert_eq!(fields.len(), cols.len());
    let rows = DetailsRows(&entries, &fields);
    assert_eq!(rows.cell(0, 1), "/home/u/Desktop");
    assert_eq!(rows.cell(0, 2), "9/14/2026 3:43 PM");
    assert_eq!(rows.cell(0, 3), "4 KB");
}

/// Clicking a row reports its index and a column header reports that column,
#[test]
fn search_columns_show_the_folder_each_result_is_in() {
    let cols = search_columns();
    let shape: Vec<(&str, i32, bool)> = cols
        .iter()
        .map(|c| (c.title.as_str(), c.width, c.right))
        .collect();
    assert_eq!(
        shape,
        [
            ("Name", 178, false),
            ("In Folder", 180, false),
            ("Size", 94, true),
            ("Type", 118, false),
            ("Date Modified", 120, false),
        ]
    );
    let entries = vec![Entry {
        name: "notes.txt".into(),
        folder: "/home/u/Documents/2026".into(),
        size: 4096,
        ..Default::default()
    }];
    use listview::Rows;
    let fields = search_fields();
    assert_eq!(fields.len(), cols.len());
    let rows = DetailsRows(&entries, &fields);
    assert_eq!(rows.cell(0, 1), "/home/u/Documents/2026");
    assert_eq!(rows.cell(0, 2), "4 KB");
}

/// straight off the transient `ListView`; stepping Down moves the selection as `listview::ListView::step` would.
#[test]
fn details_hit_and_step_delegate_to_listview() {
    let mut sv = view(3);
    sv.mode = Mode::Details;
    let r = Rect::new(0, 0, 400, 200);
    let lay = layout(&sv, r, &ControlMetrics::CLASSIC);
    assert_eq!(
        hit(&sv, &lay, lay.cells[1].x + 5, lay.cells[1].y + 5),
        Hit::Entry(1)
    );
    assert_eq!(
        hit(&sv, &lay, 5, lay.body.y + 2),
        Hit::Header(0),
        "the header, not a row"
    );
    sv.selection.set_only(Some(0));
    assert!(step(&mut sv, IconMove::Down, &lay, Mods::default()));
    assert_eq!(sv.selection.focus, Some(1));
    assert!(
        !step(&mut sv, IconMove::Left, &lay, Mods::default()),
        "a single-column Details view has no row to Left's left"
    );
}

pub(crate) fn render(sv: &ShellView, r: Rect) -> (Canvas, Layout, Theme) {
    let (text, px) = rend();
    let theme = Theme::default();
    let lay = layout(sv, r, &ControlMetrics::CLASSIC);
    let mut c = Canvas::new(r.width as u32, r.height as u32);
    draw(
        &mut c,
        &text,
        &classic_paint(&theme.colors, 1),
        sv,
        &lay,
        None,
        px,
    );
    (c, lay, theme)
}

pub(crate) fn iconed(n: usize, mode: Mode) -> ShellView {
    let mut sv = view(n);
    sv.mode = mode;
    for e in &mut sv.entries {
        e.icon32 = Some(std::rc::Rc::new(square(32, Color::rgb(255, 0, 0))));
        e.icon48 = Some(std::rc::Rc::new(square(48, Color::rgb(255, 0, 0))));
    }
    sv
}

/// `hit` names the cell under the pointer, the background between them, a group header as background and the scrollbar's three parts.
#[test]
fn hit_maps_cells_headers_background_and_the_scrollbar() {
    let mut sv = iconed(60, Mode::Icons);
    sv.groups = vec![Group {
        title: "Folders".into(),
        first: 0,
    }];
    let r = Rect::new(0, 0, 400, 200);
    let lay = layout(&sv, r, &ControlMetrics::CLASSIC);
    let cell = lay.cells[1];
    assert_eq!(hit(&sv, &lay, cell.x + 2, cell.y + 2), Hit::Entry(1));
    let header = lay.headers[0];
    assert_eq!(
        hit(&sv, &lay, header.x + 2, header.y + 2),
        Hit::Background,
        "a group header is not an entry"
    );
    let vb = lay.vscroll.expect("60 icons overrun a 200 px body");
    assert_eq!(hit(&sv, &lay, vb.x + 2, vb.y + 2), Hit::VBar(BarHit::Back));
    assert_eq!(
        hit(&sv, &lay, vb.x + 2, vb.bottom() - 2),
        Hit::VBar(BarHit::Forward)
    );
    // At the top the thumb sits under the up arrow: the track's middle is a
    // page down, the thumb reports its grab, and a drag to the track's end scrolls to the last page.
    assert_eq!(
        hit(&sv, &lay, vb.x + 2, vb.y + vb.height / 2),
        Hit::VBar(BarHit::PageForward)
    );
    assert_eq!(
        hit(
            &sv,
            &lay,
            vb.x + 2,
            vb.y + ControlMetrics::CLASSIC.scroll + 2
        ),
        Hit::VBar(BarHit::Thumb(2))
    );
    assert!(drag_thumb(&mut sv, vb.bottom(), 2, &lay));
    assert_eq!(sv.top, (lay.content.1 - lay.body.height) as usize);
    assert_eq!(
        hit(
            &sv,
            &lay,
            vb.x + 2,
            vb.y + ControlMetrics::CLASSIC.scroll + 2
        ),
        Hit::VBar(BarHit::PageBack),
        "scrolled down, the track's top is a page up"
    );
    assert!(!drag_thumb(&mut sv, vb.bottom(), 2, &lay), "already there");
    assert_eq!(hit(&sv, &lay, 0, 0), Hit::Nothing, "the frame itself");
}

/// List's range is the columns its body holds once the bar has taken its
/// height off the rows: scrolled to the far end, the last entry's column ends
/// at the body's edge. A wider window shows its own far end, and an arrow steps from there.
#[test]
fn list_mode_scrolls_to_its_last_column_and_no_further() {
    let mut sv = view(60);
    sv.mode = Mode::List;
    let cm = ControlMetrics::CLASSIC;
    let r = Rect::new(0, 0, 300, 100);
    let lay = layout(&sv, r, &cm);
    let hb = lay.hscroll.expect("60 names overrun 300 px");
    drag_hthumb(&mut sv, hb.right(), 2, &lay);
    let lay = layout(&sv, r, &cm);
    assert_eq!(
        lay.cells[59].right(),
        lay.body.right(),
        "the last column ends there"
    );
    assert!(!hstep(&mut sv, 1, &lay), "and no further");

    let far = sv.left;
    let wide = layout(&sv, Rect::new(0, 0, 1000, 100), &cm);
    assert!(wide.hscroll.is_some());
    let end = wide.content.0 - wide.body.width;
    assert!(end < far);
    assert_eq!(wide.cells[59].right(), wide.body.right());
    assert!(!hstep(&mut sv, 1, &wide), "already at the far end");
    assert!(hstep(&mut sv, -1, &wide));
    assert_eq!(sv.left, end - COL_W);
}

/// In List the wheel scrolls sideways a column a notch, clamped.
#[test]
fn the_wheel_steps_list_mode_a_column_a_notch() {
    let mut sv = view(60);
    sv.mode = Mode::List;
    let lay = layout(&sv, Rect::new(0, 0, 300, 100), &ControlMetrics::CLASSIC);
    assert!(!scroll(&mut sv, -WHEEL_ROWS, &lay), "already at the left");
    assert!(scroll(&mut sv, WHEEL_ROWS, &lay));
    assert_eq!(sv.left, COL_W);
    assert!(scroll(&mut sv, 2 * WHEEL_ROWS, &lay));
    assert_eq!(sv.left, 3 * COL_W);
    assert!(scroll(&mut sv, -1, &lay), "a part notch still steps");
    assert_eq!(sv.left, 2 * COL_W);
    assert_eq!(sv.top, 0, "nothing scrolls down");
}

/// List's bar: the arrows step a column, the track pages, the thumb reports
/// its grab and a drag to the track's end lands on the last page; Details' bar does the same through its `listview`.
#[test]
fn the_horizontal_bar_steps_pages_and_drags() {
    let mut sv = view(60);
    sv.mode = Mode::List;
    let r = Rect::new(0, 0, 300, 100);
    let lay = layout(&sv, r, &ControlMetrics::CLASSIC);
    let hb = lay.hscroll.expect("60 names overrun 300 px");
    let max = lay.content.0 - lay.body.width;
    assert!(max > 0);
    assert_eq!(hit(&sv, &lay, hb.x + 2, hb.y + 2), Hit::HBar(BarHit::Back));
    assert_eq!(
        hit(&sv, &lay, hb.right() - 2, hb.y + 2),
        Hit::HBar(BarHit::Forward)
    );
    assert_eq!(
        hit(
            &sv,
            &lay,
            hb.x + ControlMetrics::CLASSIC.scroll + 2,
            hb.y + 2
        ),
        Hit::HBar(BarHit::Thumb(2))
    );
    assert_eq!(
        hit(
            &sv,
            &lay,
            hb.right() - ControlMetrics::CLASSIC.scroll - 2,
            hb.y + 2
        ),
        Hit::HBar(BarHit::PageForward)
    );
    assert!(hstep(&mut sv, 1, &lay));
    assert_eq!(sv.left, COL_W);
    assert!(hpage(&mut sv, 1, &lay));
    assert_eq!(sv.left, COL_W + lay.body.width);
    assert!(drag_hthumb(&mut sv, hb.right(), 2, &lay));
    assert_eq!(sv.left, max, "the last page");
    assert!(!hstep(&mut sv, 1, &lay), "clamped");
    assert_eq!(
        hit(
            &sv,
            &lay,
            hb.x + ControlMetrics::CLASSIC.scroll + 2,
            hb.y + 2
        ),
        Hit::HBar(BarHit::PageBack)
    );
    assert!(hpage(&mut sv, -99, &lay));
    assert_eq!(sv.left, 0);

    sv.mode = Mode::Details;
    sv.columns[0].width = 400;
    let lay = layout(&sv, r, &ControlMetrics::CLASSIC);
    let hb = lay.hscroll.expect("a 400 px column overruns 300 px");
    assert_eq!(
        hit(
            &sv,
            &lay,
            hb.x + ControlMetrics::CLASSIC.scroll + 2,
            hb.y + 2
        ),
        Hit::HBar(BarHit::Thumb(2))
    );
    assert!(drag_hthumb(&mut sv, hb.right(), 2, &lay));
    assert!(sv.left > 0, "Details scrolled through its listview");
    assert!(hpage(&mut sv, -99, &lay));
    assert_eq!(sv.left, 0);
}

/// Details' widths are 100% px: at 200% every column, its divider and the
/// scrolled width are twice as wide, and at 100% they draw as saved.
#[test]
fn details_columns_draw_grown_by_the_text_size() {
    let mut sv = view(3);
    sv.mode = Mode::Details;
    let r = Rect::new(0, 0, 400, 200);
    let saved: i32 = sv.columns.iter().map(|c| c.width).sum();
    for (pct, k) in [(100, 1), (200, 2)] {
        let cm = crate::theme::Theme::default()
            .with_text_size(pct)
            .controls();
        let lay = layout(&sv, r, &cm);
        assert_eq!(lay.content.0, k * saved, "{pct}%: the columns' width");
        let edge = FIELD_PAD + k * 178;
        assert_eq!(
            hit(&sv, &lay, edge, FIELD_PAD + 2),
            Hit::Divider(0),
            "{pct}%"
        );
    }
    assert_eq!(sv.columns[0].width, 178, "laying out keeps the saved width");
}

/// A divider dragged 40 px at 200% widens its column by 20 100% px; the same drag at 100% by 40.
#[test]
fn a_divider_drag_saves_its_width_at_100_percent() {
    let mut sv = view(3);
    let big = crate::theme::Theme::default()
        .with_text_size(200)
        .controls()
        .text;
    assert!(resize_column(&mut sv, 2, 118, 40, big));
    assert_eq!(sv.columns[2].width, 138);
    assert!(
        !resize_column(&mut sv, 2, 118, 41, big),
        "the edge stays at or left of the pointer"
    );
    assert!(resize_column(&mut sv, 2, 138, 40, TextUnits::ONE));
    assert_eq!(sv.columns[2].width, 178);
    assert!(resize_column(&mut sv, 2, 178, -400, big));
    assert_eq!(sv.columns[2].width, listview::MIN_COL_W);
    assert!(!resize_column(&mut sv, 9, 100, 40, big), "no such column");
}

/// Details re-lays its list out at the sizes its layout was built with: at
/// 17 px the up arrow's last row, 16 px down the bar, is still the arrow, not the thumb.
#[test]
fn details_hits_a_17_px_bar_at_17_px() {
    let mut sv = view(40);
    sv.mode = Mode::Details;
    let cm = ControlMetrics {
        scroll: 17,
        ..ControlMetrics::CLASSIC
    };
    let lay = layout(&sv, Rect::new(0, 0, 300, 100), &cm);
    let vb = lay.vscroll.expect("40 rows overrun the body");
    assert_eq!(vb.width, 17);
    assert_eq!(hit(&sv, &lay, vb.x + 2, vb.y + 16), Hit::VBar(BarHit::Back));
    assert_eq!(
        hit(&sv, &lay, vb.x + 2, vb.y + 17),
        Hit::VBar(BarHit::Thumb(0))
    );
}

/// The List rename box is one row tall at every scale: 34 px at scale 2, its bottom border on the box's last row.
#[test]
fn list_rename_box_is_a_row_tall_at_scale_2() {
    let (text, px) = rend();
    let theme = Theme::default();
    let cs = &theme.colors;
    let mut sv = view(3);
    sv.edit = Some((0, TextEdit::new("item0", true)));
    let r = Rect::new(0, 0, 300, 100);
    let lay = layout(&sv, r, &ControlMetrics::CLASSIC);
    let mut c = Canvas::new(600, 200);
    draw(&mut c, &text, &classic_paint(cs, 2), &sv, &lay, None, px);
    let label = label_rect(&sv, &lay, 0, &text, px).scaled(2);
    let (bx, by) = (label.x - listview::EDIT_BACK * 2, label.y - 2);
    assert_eq!(
        c.get(bx, by + 33),
        cs.window_frame.argb(),
        "border at row 33"
    );
    assert_ne!(c.get(bx, by + 34), cs.window_frame.argb(), "34 px tall");
}

#[test]
fn a_press_in_the_rename_box_finds_the_byte_under_it_in_every_mode() {
    let (text, px) = rend();
    for mode in [
        Mode::List,
        Mode::Details,
        Mode::Icons,
        Mode::Tiles,
        Mode::Thumbnails,
    ] {
        let mut sv = view(3);
        sv.mode = mode;
        sv.edit = Some((1, TextEdit::new("item1", true)));
        let lay = layout(&sv, Rect::new(0, 0, 400, 300), &ControlMetrics::CLASSIC);
        let (start, y) = match mode {
            Mode::Icons => {
                let cell = lay.cells[1];
                let el = iconview::edit_layout_at(
                    1,
                    px,
                    &text,
                    &sv.edit.as_ref().unwrap().1,
                    cell.width,
                );
                (cell.x + el.lines[0].x, cell.y + el.lines[0].y + 2)
            }
            Mode::Details => {
                let b = label_rect(&sv, &lay, 1, &text, px);
                (b.x + listview::EDIT_PAD_L, b.y + b.height / 2)
            }
            _ => {
                let l = label_rect(&sv, &lay, 1, &text, px);
                (
                    l.x - listview::EDIT_BACK + listview::EDIT_PAD_L,
                    l.y + l.height / 2,
                )
            }
        };
        let at = |x| edit_index_at(&sv, &lay, x, y, &text, px);
        assert_eq!(at(start), Some(0), "{mode:?}: the text's left edge");
        let it = text.measure("it", false, px);
        assert_eq!(at(start + it), Some(2), "{mode:?}: after \"it\"");
        assert_eq!(at(start + 200), Some(5), "{mode:?}: past the end");
        let off = |x, y| edit_index_at(&sv, &lay, x, y, &text, px);
        assert_eq!(
            off(start - 40, y - 60),
            Some(0),
            "{mode:?}: dragged above and left"
        );
        assert_eq!(
            off(start + 200, y + 60),
            Some(5),
            "{mode:?}: dragged below and right"
        );
    }
    assert_eq!(
        edit_index_at(
            &view(3),
            &layout(
                &view(3),
                Rect::new(0, 0, 400, 300),
                &ControlMetrics::CLASSIC
            ),
            0,
            0,
            &text,
            px
        ),
        None
    );
}

#[test]
fn the_rename_field_is_the_whole_box_in_every_mode() {
    let (text, px) = rend();
    let theme = Theme::default();
    for mode in [
        Mode::List,
        Mode::Details,
        Mode::Thumbnails,
        Mode::Tiles,
        Mode::Icons,
    ] {
        let mut sv = view(3);
        sv.mode = mode;
        sv.edit = Some((1, TextEdit::new("item1", true)));
        let lay = layout(&sv, Rect::new(0, 0, 400, 300), &ControlMetrics::CLASSIC);
        let mut c = Canvas::new(400, 300);
        draw(
            &mut c,
            &text,
            &classic_paint(&theme.colors, 1),
            &sv,
            &lay,
            None,
            px,
        );
        let spot = c.take_spot().expect("the rename box reports its field");
        let b = rename_rect(&sv, &lay, 1, &text, px).expect("entry 1 renames");
        assert_eq!(spot.field, b, "{mode:?}");
        assert_eq!(
            c.get(b.x, b.y + b.height / 2),
            theme.colors.window_frame.argb(),
            "{mode:?}: the box's left border"
        );
    }
}

/// Renaming in Details is the transient `ListView`'s own box, placed by `listview::ListView::edit_rect`.
#[test]
fn renaming_in_details_delegates_to_listview() {
    let mut sv = view(3);
    sv.mode = Mode::Details;
    sv.edit = Some((1, TextEdit::new("item1", true)));
    let (c, lay, theme) = render(&sv, Rect::new(0, 0, 400, 200));
    let cs = &theme.colors;
    let (text, px) = rend();
    let b = label_rect(&sv, &lay, 1, &text, px);
    assert_eq!(
        c.get(b.x, b.y + 2),
        cs.window_frame.argb(),
        "the rename box's left border"
    );
    assert_eq!(c.get(b.x + 1, b.y + 2), cs.window.argb(), "white inside");
}

/// An entry is named by its icon and label in List and Details, by its whole cell in the icon modes.
#[test]
fn a_name_hit_needs_the_icon_or_the_label_in_list_and_details() {
    let (text, px) = rend();
    let r = Rect::new(0, 0, 500, 200);
    for mode in [Mode::List, Mode::Details] {
        let mut sv = view(3);
        sv.mode = mode;
        let lay = layout(&sv, r, &ControlMetrics::CLASSIC);
        let label = label_rect(&sv, &lay, 1, &text, px);
        let y = label.y + label.height / 2;
        // Past the name but still on the entry: the end of a List cell, the Size column of a Details row.
        let past = match mode {
            Mode::List => lay.cells[1].right() - 1,
            _ => label.right() + 80,
        };
        assert!(past >= label.right());
        assert_eq!(hit(&sv, &lay, past, y), Hit::Entry(1), "{mode:?}");
        assert_eq!(name_hit(&sv, &lay, label.x + 2, y, &text, px), Some(1));
        assert_eq!(name_hit(&sv, &lay, label.x - 8, y, &text, px), Some(1));
        assert_eq!(
            name_hit(&sv, &lay, past, y, &text, px),
            None,
            "{mode:?}: past the name"
        );
    }
    // A long name cut short in a narrow Name column names nothing past the column.
    let mut sv = view(3);
    sv.mode = Mode::Details;
    sv.entries[1].name = "a name far longer than its column".into();
    sv.columns[0].width = 80;
    let lay = layout(&sv, r, &ControlMetrics::CLASSIC);
    let label = label_rect(&sv, &lay, 1, &text, px);
    let y = label.y + label.height / 2;
    let size = lay.body.x + 80 + 10;
    assert_eq!(hit(&sv, &lay, size, y), Hit::Entry(1));
    assert_eq!(
        name_hit(&sv, &lay, size, y, &text, px),
        None,
        "the Size cell"
    );
    assert_eq!(name_hit(&sv, &lay, label.x + 2, y, &text, px), Some(1));

    let mut sv = view(3);
    sv.mode = Mode::Icons;
    let lay = layout(&sv, r, &ControlMetrics::CLASSIC);
    let cell = lay.cells[1];
    assert_eq!(
        name_hit(&sv, &lay, cell.x + 1, cell.y + 1, &text, px),
        Some(1)
    );
}

/// `scroll` moves by a cell's height in the icon modes and whole rows in
/// Details; `ensure_visible` brings a cell below the body back into it.
#[test]
fn scrolling_moves_by_a_cell_and_ensure_visible_follows() {
    let mut sv = iconed(60, Mode::Icons);
    let r = Rect::new(0, 0, 400, 200);
    let lay = layout(&sv, r, &ControlMetrics::CLASSIC);
    assert!(scroll(&mut sv, 1, &lay));
    assert_eq!(sv.top, ICON_CELL.1 as usize, "one cell down");
    sv.top = 0;
    let lay = layout(&sv, r, &ControlMetrics::CLASSIC);
    let last = sv.entries.len() - 1;
    assert!(
        lay.cells[last].bottom() > lay.body.bottom(),
        "the last cell starts off screen"
    );
    ensure_visible(&mut sv, last, &lay);
    let lay = layout(&sv, r, &ControlMetrics::CLASSIC);
    assert!(lay.cells[last].bottom() <= lay.body.bottom());
    assert!(lay.cells[last].y >= lay.body.y);
}

#[test]
fn a_reader_scrolls_an_entry_in_without_picking_it_in_every_mode() {
    use crate::access::{Published, ROOT, Tree};
    let (text, px) = rend();
    let r = Rect::new(0, 0, 300, 120);
    for mode in [Mode::List, Mode::Details, Mode::Icons, Mode::Tiles] {
        let mut sv = iconed(200, mode);
        sv.selection.set_only(Some(0));
        let last = sv.entries.len() - 1;
        let lay = layout(&sv, r, &ControlMetrics::CLASSIC);
        let mut tree = Tree::window("", (300, 120));
        access(&sv, &lay, &text, px, &mut tree, ROOT, "view");
        let first = tree.find(Role::ListBoxOption, &sv.entries[0].name).unwrap();
        let end = tree
            .find(Role::ListBoxOption, &sv.entries[last].name)
            .unwrap();
        let mut p = Published::default();
        p.update(tree);
        assert_eq!(p.reveal(first), None, "{mode:?}: the first entry shows");
        assert_eq!(p.reveal(end).map(|r| r.row), Some(last), "{mode:?}");
        assert!(reveal(&mut sv, last, &lay), "{mode:?}");
        let lay = layout(&sv, r, &ControlMetrics::CLASSIC);
        let mut tree = Tree::window("", (300, 120));
        access(&sv, &lay, &text, px, &mut tree, ROOT, "view");
        let end = tree
            .find(Role::ListBoxOption, &sv.entries[last].name)
            .unwrap();
        let first = tree.find(Role::ListBoxOption, &sv.entries[0].name).unwrap();
        let mut p = Published::default();
        p.update(tree);
        assert_eq!(p.reveal(end), None, "{mode:?}: the last entry shows");
        assert!(p.reveal(first).is_some(), "{mode:?}: the first is out");
        assert_eq!(sv.selection.focus, Some(0), "{mode:?}: the pick stays");
    }
}

/// Writes target/shellview-preview.png: the five modes stacked, the last grouped, for a visual check.
#[test]
#[ignore]
fn render_shellview() {
    let (text, px) = rend();
    let theme = Theme::default();
    let (w, h) = (700, 240);
    let views = [
        sample(Mode::List, false),
        sample(Mode::Details, false),
        sample(Mode::Icons, false),
        sample(Mode::Tiles, false),
        sample(Mode::Thumbnails, false),
        sample(Mode::Tiles, true),
    ];
    let mut c = Canvas::new(w as u32, (h * views.len() as i32) as u32);
    c.fill_rect(
        Rect::new(0, 0, w, h * views.len() as i32),
        theme.colors.button_face,
    );
    for (i, sv) in views.iter().enumerate() {
        let r = Rect::new(8, 8 + i as i32 * h, w - 16, h - 16);
        let lay = layout(sv, r, &ControlMetrics::CLASSIC);
        draw(
            &mut c,
            &text,
            &classic_paint(&theme.colors, 1),
            sv,
            &lay,
            None,
            px,
        );
    }
    crate::raster::write_preview(&c, "shellview-preview.png");
}

/// Details hands the lit bar part to the list it draws through: a Blue Details view lights its hot arrow, and `Hit::bar` names it.
#[test]
fn a_blue_details_view_lights_its_bar_part() {
    use crate::classic::widgets::{Axis, ScrollHit};
    let theme = crate::interfaces::theme_for(crate::interface::Interface::Blue);
    let cm = theme.look.controls();
    let (text, px) = rend();
    let mut sv = view(80);
    sv.mode = Mode::Details;
    let r = Rect::new(0, 0, 300, 160);
    let lay = layout(&sv, r, &cm);
    let lv = details_listview(&sv);
    let sb = listview::layout(&lv, r, &cm)
        .scrollbar
        .expect("80 rows overflow");
    let up = hit(&sv, &lay, sb.x + 8, sb.y + 2).bar();
    assert_eq!(up, Some((Axis::Vertical, ScrollHit::Back)));
    sv.bars.motion(up);
    let mut c = Canvas::new(300, 160);
    draw(&mut c, &text, &theme.paint(), &sv, &lay, sv.bars.lit(), px);
    assert_eq!(
        c.get(sb.x + 8, sb.y + 2),
        crate::look::styled::fixture::blue_pixel(
            crate::style::Part::ScrollArrowUp,
            crate::style::State::Hot,
            8,
            2
        ),
        "the hot up arrow"
    );
}

/// Silver's icon and List modes take its list line, as its Details view does, not its edit line.
#[test]
fn silvers_view_takes_its_list_line_in_every_mode() {
    let theme = crate::interfaces::blue_theme(crate::blue::Variant::Silver);
    let cm = theme.look.controls();
    let silver = crate::blue::Variant::Silver.scheme();
    let list = silver.list_border.as_ref().unwrap().normal.rim.argb();
    let (text, px) = rend();
    let r = Rect::new(0, 0, 300, 160);
    for mode in [Mode::Icons, Mode::Tiles, Mode::List] {
        let mut sv = view(3);
        sv.mode = mode;
        let lay = layout(&sv, r, &cm);
        let mut c = Canvas::new(300, 160);
        draw(&mut c, &text, &theme.paint(), &sv, &lay, None, px);
        let f = lay.frame;
        for (x, y) in [(f.x + 40, f.y), (f.x, f.y + 40), (f.right() - 1, f.y + 40)] {
            assert_eq!(c.get(x, y), list, "{mode:?} ({x}, {y})");
        }
    }
}

/// Type sorts case-insensitively and Date Modified by time, not text;
/// ascending keeps folders first, descending reverses it all. A view without a sort keeps the host's order.
#[test]
fn sort_orders_by_type_and_by_date() {
    let e = |name: &str, is_dir: bool, kind: &str, mtime: i64| Entry {
        name: name.into(),
        is_dir,
        kind: kind.into(),
        modified: format!("{mtime}"),
        mtime: Some(mtime),
        ..Default::default()
    };
    let mut sv = ShellView {
        entries: vec![
            e("notes", false, "text Document", 30),
            e("pics", true, "Folder", 10),
            e("a.jpg", false, "JPEG Image", 200),
            e("docs", true, "File Folder", 900),
        ],
        ..Default::default()
    };
    let names =
        |sv: &ShellView| -> Vec<String> { sv.entries.iter().map(|e| e.name.clone()).collect() };
    sort(&mut sv);
    assert_eq!(names(&sv), ["notes", "pics", "a.jpg", "docs"], "no sort");
    sv.sort = Some((2, true));
    sort(&mut sv);
    assert_eq!(names(&sv), ["docs", "pics", "a.jpg", "notes"], "Type");
    sv.sort = Some((3, true));
    sort(&mut sv);
    assert_eq!(names(&sv), ["pics", "docs", "notes", "a.jpg"], "Date");
    sv.sort = Some((3, false));
    sort(&mut sv);
    assert_eq!(
        names(&sv),
        ["a.jpg", "notes", "docs", "pics"],
        "Date, reversed"
    );
}

/// A shortcut to a folder sorts among the files, by name and type alike.
#[test]
fn a_shortcut_to_a_folder_sorts_among_the_files() {
    let e = |name: &str, is_dir: bool, link: bool, kind: &str| Entry {
        name: name.into(),
        is_dir,
        link,
        kind: kind.into(),
        ..Default::default()
    };
    let mut sv = ShellView {
        entries: vec![
            e("c", false, false, "Text Document"),
            e("b link", true, true, "Shortcut"),
            e("a", false, false, "Text Document"),
            e("mfolder", true, false, "File Folder"),
        ],
        sort: Some((0, true)),
        ..Default::default()
    };
    let names =
        |sv: &ShellView| -> Vec<String> { sv.entries.iter().map(|e| e.name.clone()).collect() };
    sort(&mut sv);
    assert_eq!(names(&sv), ["mfolder", "a", "b link", "c"], "Name");
    sv.sort = Some((2, true));
    sort(&mut sv);
    assert_eq!(names(&sv), ["mfolder", "b link", "a", "c"], "Type");
}

const DRAG_HALF: i32 = iconview::DRAG_IMAGE_MAX / 2;

fn canvas_fnv(c: &Canvas) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let words = [c.width, c.height]
        .into_iter()
        .chain(c.pixels.iter().copied());
    for w in words {
        for b in w.to_le_bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    h
}

/// Each picked entry's icon, its colour redrawn at `s` times the size its mode draws.
fn drag_icons(sv: &ShellView, picked: &[usize], s: i32) -> Vec<(usize, Canvas)> {
    let px = match sv.mode {
        Mode::Tiles => 48,
        Mode::Thumbnails => 80,
        _ => ICON_PX,
    };
    let n = (px * s) as u32;
    picked
        .iter()
        .map(|&i| {
            let argb = sv.entries[i].icon.as_ref().expect("an icon").pixels[0];
            (i, Canvas::from_pixels(n, n, vec![argb; (n * n) as usize]))
        })
        .collect()
}

fn drag_of(
    sv: &ShellView,
    lay: &Layout,
    picked: &[usize],
    pointer: (i32, i32),
    s: i32,
) -> Option<(Canvas, (i32, i32))> {
    let icons = drag_icons(sv, picked, s);
    let rows: Vec<DragRow> = icons
        .iter()
        .map(|(index, icon)| DragRow {
            index: *index,
            icon: Some(icon),
        })
        .collect();
    let (text, _) = rend();
    drag_image(&Theme::default().scaled(s), &text, sv, lay, &rows, pointer)
}

/// A List or Details drag image is each dragged row's icon and name where the row sits,
/// faded, held where the pointer is; Details rows start at the Name column's icon.
#[test]
fn a_rows_drag_image_holds_each_dragged_row_where_it_sits() {
    for mode in [Mode::List, Mode::Details] {
        let sv = sample(mode, false);
        let lay = layout(&sv, Rect::new(0, 0, 500, 200), &ControlMetrics::CLASSIC);
        let picked = [0, 2, 3];
        let (x0, w) = match mode {
            Mode::List => (lay.cells[0].x, lay.cells[0].width),
            _ => (
                lay.cells[0].x + listview::ICON_X,
                sv.columns[0].width - listview::ICON_X,
            ),
        };
        let row_h = lay.cells[0].height;
        let pointer = (x0 + 30, lay.cells[2].y + 5);
        let (img, grab) = drag_of(&sv, &lay, &picked, pointer, 1).expect("an image");
        assert_eq!(
            (img.width as i32, img.height as i32),
            (w, 4 * row_h),
            "{mode:?}"
        );
        assert_eq!(grab, (30, 2 * row_h + 5), "{mode:?}");
        let alpha = |x: i32, y: i32| img.get(x, y) >> 24;
        for (row, drawn) in [(0, true), (1, false), (2, true), (3, true)] {
            let y = row * row_h;
            let icon = alpha(8, y + 8);
            assert_eq!(icon > 0, drawn, "{mode:?}: row {row}'s icon");
            assert!(icon <= 128, "{mode:?}: row {row} faded");
            let ink = (16..w).any(|x| (0..row_h).any(|dy| alpha(x, y + dy) > 0));
            assert_eq!(ink, drawn, "{mode:?}: row {row}'s name");
        }
    }
}

/// A Tiles or Thumbnails drag image paints each dragged cell unselected where it sits: a
/// tile's icon left of its name and detail lines, a thumbnail's picture in its frame over its
/// name; with the pointer off every cell, a cell partly past the pointer's square is cut at it.
#[test]
fn a_grid_drag_image_paints_each_tile_or_thumbnail_in_its_cell() {
    for mode in [Mode::Tiles, Mode::Thumbnails] {
        let sv = sample(mode, false);
        let lay = layout(&sv, Rect::new(0, 0, 500, 400), &ControlMetrics::CLASSIC);
        let c = lay.cells[2];
        let mid = (c.x + c.width / 2, c.y + 20);
        let (img, grab) = drag_of(&sv, &lay, &[2], mid, 1).expect("an image");
        assert_eq!(
            (img.width as i32, img.height as i32),
            (c.width, c.height),
            "{mode:?}"
        );
        assert_eq!(grab, (c.width / 2, 20), "{mode:?}");
        let alpha = |x: i32, y: i32| img.get(x, y) >> 24;
        let ink = |r: Rect| (r.x..r.right()).any(|x| (r.y..r.bottom()).any(|y| alpha(x, y) > 0));
        let (picture, label) = match mode {
            Mode::Tiles => ((5 + 24, 4 + 24), Rect::new(58, 0, c.width - 58, c.height)),
            _ => (
                (17 + 48, 3 + 48),
                Rect::new(0, 3 + 96 + 4, c.width, c.height - 103),
            ),
        };
        let a = alpha(picture.0, picture.1);
        assert!(a > 0 && a <= 128, "{mode:?}: the picture, faded ({a})");
        assert!(ink(label), "{mode:?}: the name");
        if mode == Mode::Thumbnails {
            assert!(alpha(17 + 1, 3 + 1) > 0, "the frame round the picture");
            assert_eq!(alpha(2, 2), 0, "nothing outside the frame");
        }

        let far = (c.x + c.width + 100, c.y + 20);
        let (cut, _) = drag_of(&sv, &lay, &[2], far, 1).expect("a cut image");
        assert_eq!(
            cut.width as i32,
            c.right() - (far.0 - DRAG_HALF),
            "{mode:?}: off every cell, cut at the square's left edge"
        );
    }
}

/// A tile or thumbnail pressed near its edge shows whole; the square round the pressed cell's
/// centre keeps the other cells it holds whole (the tile below; no thumbnail) and drops the
/// ones it would cut.
#[test]
fn a_grid_drag_keeps_the_pressed_cell_whole_wherever_it_is_pressed() {
    for mode in [Mode::Tiles, Mode::Thumbnails] {
        let sv = sample(mode, false);
        let lay = layout(&sv, Rect::new(0, 0, 500, 400), &ControlMetrics::CLASSIC);
        let cells = &lay.cells;
        for (dx, dy) in [(2, 2), (cells[0].width - 3, cells[0].height - 3)] {
            let at = (cells[0].x + dx, cells[0].y + dy);
            let (img, grab) = drag_of(&sv, &lay, &[0], at, 1).expect("an image");
            assert_eq!(
                (img.width as i32, img.height as i32, grab),
                (cells[0].width, cells[0].height, (dx, dy)),
                "{mode:?} pressed at {dx},{dy}"
            );
        }
        let all: Vec<usize> = (0..cells.len()).collect();
        let at = (cells[0].x + 2, cells[0].y + 2);
        let (img, grab) = drag_of(&sv, &lay, &all, at, 1).expect("an image");
        let held = match mode {
            Mode::Tiles => cells[0].union(&cells[2]),
            _ => cells[0],
        };
        assert_eq!(
            (img.width as i32, img.height as i32, grab),
            (held.width, held.height, (2, 2)),
            "{mode:?}: the cells the square round cell 0 holds whole"
        );
    }
}

/// The Tiles and Thumbnails images' pixels in Tahoma at scale 1 and 2; scale 2 is twice the size.
#[test]
fn a_grid_drag_image_draws_its_pixels() {
    if !crate::text::test_tahoma() {
        return;
    }
    let mut got = Vec::new();
    for mode in [Mode::Tiles, Mode::Thumbnails] {
        let sv = sample(mode, false);
        let lay = layout(&sv, Rect::new(0, 0, 500, 400), &ControlMetrics::CLASSIC);
        let pointer = (lay.cells[2].x + 30, lay.cells[2].y + 5);
        let (one, grab) = drag_of(&sv, &lay, &[2, 3, 5], pointer, 1).expect("an image");
        let (two, grab2) = drag_of(&sv, &lay, &[2, 3, 5], pointer, 2).expect("at scale 2");
        assert_eq!(grab2, grab, "{mode:?}");
        assert_eq!(
            (two.width, two.height),
            (one.width * 2, one.height * 2),
            "{mode:?}"
        );
        got.push(canvas_fnv(&one));
        got.push(canvas_fnv(&two));
    }
    assert_eq!(
        got,
        [
            13645227026126532502,
            1281062025351903798,
            7530801195331324834,
            6682858532581181662
        ]
    );
}

/// Rows scrolled out of the body stay out of the image, and with none shown there is none.
#[test]
fn a_rows_drag_image_keeps_only_the_rows_on_screen() {
    let mut sv = sample(Mode::Details, false);
    sv.top = 2;
    let lay = layout(&sv, Rect::new(0, 0, 500, 200), &ControlMetrics::CLASSIC);
    let row_h = lay.cells[2].height;
    let at = (lay.cells[2].x + 30, lay.cells[2].y + 4);
    let (img, grab) = drag_of(&sv, &lay, &[0, 1, 2, 3], at, 1).expect("an image");
    assert_eq!(img.height as i32, 2 * row_h, "rows 2 and 3");
    assert_eq!(grab.1, 4);
    sv.top = 4;
    let lay = layout(&sv, Rect::new(0, 0, 500, 200), &ControlMetrics::CLASSIC);
    assert!(
        drag_of(&sv, &lay, &[0, 1], at, 1).is_none(),
        "both rows scrolled off"
    );
    let sv = sample(Mode::Icons, false);
    let lay = layout(&sv, Rect::new(0, 0, 500, 200), &ControlMetrics::CLASSIC);
    assert!(
        drag_of(&sv, &lay, &[0], at, 1).is_none(),
        "no rows image in Icons"
    );
}

/// The image's pixels in Tahoma, per mode at scale 1 and 2; scale 2 is twice the size.
#[test]
fn a_rows_drag_image_draws_its_pixels() {
    if !crate::text::test_tahoma() {
        return;
    }
    let mut got = Vec::new();
    for mode in [Mode::List, Mode::Details] {
        let sv = sample(mode, false);
        let lay = layout(&sv, Rect::new(0, 0, 500, 200), &ControlMetrics::CLASSIC);
        let pointer = (lay.cells[2].x + 30, lay.cells[2].y + 5);
        let (one, grab) = drag_of(&sv, &lay, &[2, 3, 5], pointer, 1).expect("an image");
        let (two, grab2) = drag_of(&sv, &lay, &[2, 3, 5], pointer, 2).expect("at scale 2");
        assert_eq!(grab2, grab, "{mode:?}");
        assert_eq!(
            (two.width, two.height),
            (one.width * 2, one.height * 2),
            "{mode:?}"
        );
        got.push(canvas_fnv(&one));
        got.push(canvas_fnv(&two));
    }
    assert_eq!(
        got,
        [
            5882507750434519416,
            12420945453230247706,
            7509144477264736716,
            8858607519483314338
        ]
    );
}
