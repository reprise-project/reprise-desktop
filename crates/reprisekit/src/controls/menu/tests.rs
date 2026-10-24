use super::draw::centred;
use super::layout::{
    ARROW_SPACE, BORDER, ICON_X, RIGHT_PAD, ROOT_RIGHT_PAD, ROOT_TEXT_X, SEPARATOR_H, TEXT_X,
};
use super::*;
use crate::access::{self, Tree};
use crate::controls::hosted::{Anchor, Spot};
use crate::text::Align;

fn th(m: &Metrics) -> Theme {
    Theme {
        metrics: m.clone(),
        ..Theme::default()
    }
}

fn wide_metrics() -> Metrics {
    Metrics {
        menu_width: 180,
        startmenu_width: 232,
        ..Metrics::default()
    }
}

fn wide_theme() -> Theme {
    Theme {
        metrics: wide_metrics(),
        ..Theme::default()
    }
}
use crate::anim::{Unroll, UnrollFrom};
use crate::interface::Interface;
use crate::raster::{Canvas, Rect};
use crate::text::TextRenderer;
use crate::theme::{FontConfig, Metrics, Theme};

impl MenuItem {
    fn label_for_test(&self) -> String {
        match self {
            MenuItem::Entry { label, .. } => label.clone(),
            MenuItem::Separator => String::new(),
        }
    }
}

fn items() -> Vec<MenuItem> {
    vec![
        MenuItem::entry("Terminal"),
        MenuItem::Separator,
        MenuItem::entry("Shut Down..."),
    ]
}

#[test]
fn expand_label_fills_in_the_user() {
    assert_eq!(expand_label("&Log Off {user}...", "jan"), "&Log Off jan...");
    assert_eq!(expand_label("&Run...", "jan"), "&Run...");
}

#[test]
fn context_style_uses_context_menu_metrics() {
    let m = wide_metrics();
    let plain = |label: &str| MenuItem::entry(label);
    let items = vec![plain("Open"), MenuItem::Separator, plain("Delete")];
    let style = MenuStyle::context_for(&wide_theme(), &items);
    let l = MenuLayout::compute_styled(&style, &items, usize::MAX);
    assert_eq!(l.rows[0].height, 17);
    assert_eq!(l.rows[1].height, 9);
    assert_eq!(l.rows[2].y, BORDER + 17 + 9);
    assert_eq!(l.text_x, 17);
    assert_eq!(l.height, 2 * BORDER + 17 + 9 + 17);
    let mut with_icon = items.clone();
    if let MenuItem::Entry { icon, .. } = &mut with_icon[0] {
        *icon = Some(Canvas::new(16, 16));
    }
    let style = MenuStyle::context_for(&wide_theme(), &with_icon);
    let l = MenuLayout::compute_styled(&style, &with_icon, usize::MAX);
    assert_eq!(l.rows[0].height, 19);
    assert_eq!(l.rows[1].height, 9);
    assert_eq!(l.text_x, 28);
    assert_eq!(l.icon_x, 6);
    assert_eq!(l.scaled(2).icon_x, 12);
    let mut with_glyph = items.clone();
    if let MenuItem::Entry { glyph, .. } = &mut with_glyph[2] {
        *glyph = Some(MenuGlyph::Close);
    }
    let l = MenuLayout::compute_styled(
        &MenuStyle::context_for(&wide_theme(), &with_glyph),
        &with_glyph,
        usize::MAX,
    );
    assert_eq!(l.rows[0].height, 19);
    assert_eq!(l.text_x, 22);
    assert_eq!(l.icon_x, 2);
    let sub = MenuLayout::compute_styled(&MenuStyle::submenu(&th(&m)), &items, usize::MAX);
    assert_eq!(sub.rows[0].height, m.menu_item_height);
    assert_eq!(sub.rows[1].height, SEPARATOR_H);
}

#[test]
fn a_menu_level_names_its_rows_and_focuses_the_highlighted_one() {
    let items = vec![
        MenuItem::entry("Programs").submenu(true),
        MenuItem::Separator,
        MenuItem::entry("Run..."),
    ];
    let lay = MenuLayout::compute(&wide_metrics(), &items);
    let rect = Rect::new(10, 20, lay.width, lay.height);
    let mut t = Tree::window("", (400, 300));
    let (menu, rows, hot) = access_menu(
        &mut t,
        access::ROOT,
        0,
        &items,
        &lay,
        rect,
        Some(2),
        Some(0),
    );
    assert_eq!(rows[1], menu, "a separator row maps to its menu");
    assert_eq!(hot, Some(rows[2]), "the highlighted row has the focus");
    let run = t.get(rows[2]).unwrap();
    assert_eq!(run.label(), Some("Run..."));
    assert_eq!(
        run.bounds().unwrap().y0,
        f64::from(lay.rows[2].y + 20),
        "rows sit at the menu's corner"
    );
    let programs = t.get(rows[0]).unwrap();
    assert_eq!(programs.is_expanded(), Some(true), "its submenu is open");
}

#[test]
fn access_key_is_the_mnemonic_or_first_letter() {
    let (label, mnemonic) = split_mnemonic("Sear&ch");
    let searched = MenuItem::entry(label).mnemonic(mnemonic);
    assert_eq!(searched.access_key(), Some('c'));
    assert_eq!(MenuItem::entry("Run...").access_key(), Some('r'));
    assert_eq!(MenuItem::entry("...").access_key(), None);
    assert_eq!(MenuItem::Separator.access_key(), None);
    let items = vec![
        MenuItem::entry("Programs"),
        MenuItem::Separator,
        MenuItem::entry("Printers"),
        searched,
    ];
    let rows = |key| rows_for_key(&items, key).collect::<Vec<_>>();
    assert_eq!(rows('P'), vec![0, 2]);
    assert_eq!(rows('c'), vec![3]);
    assert!(rows('z').is_empty());
}

#[test]
fn step_selection_wraps_and_skips_separators() {
    let mut items = items(); // Terminal, ---, Shut Down...
    assert_eq!(step_selection(&items, None, 1, KeyStops::Enabled), Some(0));
    assert_eq!(step_selection(&items, None, -1, KeyStops::Enabled), Some(2));
    assert_eq!(
        step_selection(&items, Some(0), 1, KeyStops::Enabled),
        Some(2)
    );
    assert_eq!(
        step_selection(&items, Some(2), 1, KeyStops::Enabled),
        Some(0)
    );
    assert_eq!(
        step_selection(&items, Some(0), -1, KeyStops::Enabled),
        Some(2)
    );
    if let MenuItem::Entry { enabled, .. } = &mut items[2] {
        *enabled = false;
    }
    assert_eq!(
        step_selection(&items, Some(0), 1, KeyStops::Enabled),
        Some(0)
    );
    assert_eq!(
        step_selection(&[MenuItem::Separator], None, 1, KeyStops::Enabled),
        None
    );
}

#[test]
fn relisted_rows_close_up_open_up_and_the_hover_keeps_its_entry() {
    let m = Metrics::default();
    let (a, b, c) = (
        MenuItem::entry("a.txt"),
        MenuItem::entry("b.txt"),
        MenuItem::entry("c.txt"),
    );
    let launch = MenuItem::entry("Launch");
    let mut items = vec![a.clone(), b.clone(), MenuItem::Separator, launch.clone()];
    let mut layout = MenuLayout::compute(&m, &items);
    let mut hovered = Some(1);
    let new = vec![c.clone(), a.clone(), b.clone(), MenuItem::Separator, launch];
    let column = layout.column_width;
    relist(
        &mut items,
        &mut layout,
        &mut hovered,
        &["a", "b", "-", "L"],
        &["c", "a", "b", "-", "L"],
        new.clone(),
        column,
    );
    assert_eq!(items, new);
    assert_eq!(layout, MenuLayout::compute(&m, &items));
    assert_eq!(hovered, Some(2), "b.txt kept the hover a row down");

    let column = layout.column_width;

    relist(
        &mut items,
        &mut layout,
        &mut hovered,
        &["c", "a", "b", "-", "L"],
        &["c", "a", "-", "L"],
        vec![c, a, MenuItem::Separator, MenuItem::entry("Launch")],
        column,
    );
    assert_eq!(layout, MenuLayout::compute(&m, &items));
    assert_eq!(hovered, Some(3), "b.txt went: Launch slid into its place");

    let column = layout.column_width;

    relist(
        &mut items,
        &mut layout,
        &mut hovered,
        &["c", "a", "-", "L"],
        &["E"],
        vec![MenuItem::entry("(Empty)").enabled(false)],
        column,
    );
    assert_eq!(layout, MenuLayout::compute(&m, &items));
    assert_eq!(hovered, None, "(Empty) takes no hover");
}

#[test]
fn a_relisted_row_keeps_its_item_and_the_width_stays() {
    let m = Metrics::default();
    let style = MenuStyle::compact(&m);
    let wide = MenuStyle {
        width: style.width + 40,
        ..style
    };
    let iconed = MenuItem::entry("a.txt").icon(Some(Canvas::new(1, 1)));
    let mut items = vec![iconed.clone()];
    let mut layout = MenuLayout::compute_styled(&wide, &items, usize::MAX);
    let mut hovered = None;
    let column = layout.column_width;
    relist(
        &mut items,
        &mut layout,
        &mut hovered,
        &["a"],
        &["b", "a"],
        vec![MenuItem::entry("b.txt"), MenuItem::entry("a.txt")],
        column,
    );
    assert_eq!(items, [MenuItem::entry("b.txt"), iconed]);
    assert_eq!(
        layout,
        MenuLayout::compute_styled(&wide, &items, usize::MAX)
    );
}

#[test]
fn a_relisted_longer_name_widens_the_menu_as_an_open_would() {
    let m = Metrics {
        menu_width: 180,
        ..wide_metrics()
    };
    let text = TextRenderer::load(&FontConfig::default());
    let style = MenuStyle::submenu(&th(&m));
    let short = vec![MenuItem::entry("a.txt")];
    let mut items = short.clone();
    let mut layout =
        MenuLayout::compute_styled(&style.fit(&text, &items, 11, 640), &items, usize::MAX);
    let mut hovered = None;
    let long = vec![
        MenuItem::entry(format!("{}.txt", "x".repeat(40))),
        MenuItem::entry("a.txt"),
    ];
    let column = layout.fit_column(&text, &long, 11, 640);
    relist(
        &mut items,
        &mut layout,
        &mut hovered,
        &["a"],
        &["x", "a"],
        long.clone(),
        column,
    );
    let opened = MenuLayout::compute_styled(&style.fit(&text, &long, 11, 640), &long, usize::MAX);
    assert!(
        opened.width > m.menu_width,
        "the long name needs a wider menu"
    );
    assert_eq!(layout, opened);

    let column = layout.fit_column(&text, &short, 11, 640);
    relist(
        &mut items,
        &mut layout,
        &mut hovered,
        &["x", "a"],
        &["a"],
        short,
        column,
    );
    assert_eq!(
        layout.width, opened.width,
        "a relist never narrows the menu"
    );
}

#[test]
fn underline_sits_under_the_mnemonic_only_in_keyboard_mode() {
    let theme = wide_theme();
    let text = TextRenderer::load(&FontConfig::default());
    let (label, mnemonic) = split_mnemonic("Sear&ch");
    let items = vec![MenuItem::entry(label).mnemonic(mnemonic)];
    let l = MenuLayout::compute(&theme.metrics, &items);
    let size = theme.fonts.size_px;
    let r = l.rows[0];
    let text_rect = Rect::new(r.x + l.text_x, r.y, r.width - l.text_x, r.height);
    let baseline = text.baseline(text_rect, false, size).unwrap();
    let (dx, w) = text.span("Search", 4, false, size);
    assert!(w > 0);
    let fg = theme.colors.menu_text.argb();
    for keyboard in [false, true] {
        let mut c = Canvas::new(l.width as u32, l.height as u32);
        draw_menu(&mut c, &theme, &text, &l, &items, None, "", keyboard);
        let row: Vec<u32> = (0..w)
            .map(|i| c.get(text_rect.x + dx + i, baseline + 1))
            .collect();
        assert_eq!(
            row.iter().all(|&p| p == fg),
            keyboard,
            "keyboard={keyboard}"
        );
        assert_ne!(c.get(text_rect.x + dx - 1, baseline + 1), fg);
    }
}

#[test]
fn underline_is_dropped_when_the_ellipsis_cuts_the_mnemonic() {
    let theme = wide_theme();
    let text = TextRenderer::load(&FontConfig::default());
    let label = "Averylongapplicationnamethatcannotfit";
    let underlined = |m: usize| {
        let items = vec![MenuItem::entry(label).mnemonic(Some(m))];
        let mut l = MenuLayout::compute(&theme.metrics, &items);
        l.width = 90;
        l.rows[0].width = 90 - 2 * BORDER;
        let draw = |keyboard| {
            let mut c = Canvas::new(l.width as u32, l.height as u32);
            draw_menu(&mut c, &theme, &text, &l, &items, None, "", keyboard);
            c
        };
        let (plain, keyed) = (draw(false), draw(true));
        (0..l.height).any(|y| (0..l.width).any(|x| plain.get(x, y) != keyed.get(x, y)))
    };
    let text_x = MenuLayout::compute(&theme.metrics, &[MenuItem::entry(label)]).text_x;
    let width = 90 - 2 * BORDER - text_x;
    let shown = text.ellipsize(label, width - ARROW_SPACE, false, theme.fonts.size_px);
    let kept = shown.chars().count() - 3;
    assert!(kept > 0);
    for m in 0..label.chars().count() {
        assert_eq!(underlined(m), m < kept, "access key at char {m}");
    }
}

/// A frame of `items` unrolling from `from` halfway in at scale `s`: the menu and its
/// shadow moved by the frame's shift and cut to its clip, as the compositor's scene shows it.
fn unrolled(theme: &Theme, items: &[MenuItem], from: UnrollFrom, s: i32) -> Canvas {
    let text = TextRenderer::load(&FontConfig::default());
    let mut theme = theme.clone();
    theme.metrics.menu_width = 120;
    let theme = &theme;
    let layout = MenuLayout::compute_styled(&MenuStyle::submenu(theme), items, 99);
    let reach = shadow_reach(theme);
    let size = (layout.width + reach, layout.height + reach);
    let frame = Unroll::at(from, size, 0.5);
    let (theme, layout) = (theme.scaled(s), layout.scaled(s));
    let mut menu = Canvas::new(layout.width as u32, layout.height as u32);
    draw_menu(&mut menu, &theme, &text, &layout, items, Some(0), "", false);
    let shadow = draw_menu_shadow(&theme, layout.width, layout.height, i32::MAX);
    let (dx, dy) = (frame.shift.0 * s, frame.shift.1 * s);
    let c = frame.clip;
    let clip = Rect::new(c.x * s + dx, c.y * s + dy, c.width * s, c.height * s);
    let mut out = Canvas::new((size.0 * s) as u32, (size.1 * s) as u32);
    out.blit_over_clipped(dx, dy, &shadow, clip);
    out.blit_over_clipped(dx, dy, &menu, clip);
    out
}

#[test]
fn half_unrolled_menus_at_scale_1_and_2() {
    let items = [
        MenuItem::entry("").submenu(true),
        MenuItem::Separator,
        MenuItem::entry("").checked(true),
        MenuItem::entry(""),
    ];
    let glass = crate::interfaces::theme_with(
        crate::interface::Interface::Glass,
        Default::default(),
        &Default::default(),
    );
    let mut got = Vec::new();
    for (look, theme) in [("Classic", Theme::default()), ("Glass", glass)] {
        for from in [
            UnrollFrom::Top,
            UnrollFrom::Bottom,
            UnrollFrom::Left,
            UnrollFrom::Right,
        ] {
            for s in [1, 2] {
                let c = unrolled(&theme, &items, from, s);
                let clear = |x: i32, y: i32| c.get(x, y) >> 24 == 0;
                let (w, h) = (c.width as i32, c.height as i32);
                let (near, far) = match from {
                    UnrollFrom::Top => ((w / 4, 0), (w / 4, h - 1)),
                    UnrollFrom::Bottom => ((w / 4, h - 1), (w / 4, 0)),
                    UnrollFrom::Left => ((0, h / 4), (w - 1, h / 4)),
                    UnrollFrom::Right => ((w - 1, h / 4), (0, h / 4)),
                };
                assert!(
                    !clear(near.0, near.1),
                    "{look} {from:?} {s}: the near edge shows menu"
                );
                assert!(
                    clear(far.0, far.1),
                    "{look} {from:?} {s}: the far edge is still empty"
                );
                got.push((format!("{look} {from:?} {s}"), fnv(&c)));
            }
        }
    }
    crate::pins::check("menu_unroll", &got);
}

fn fnv(c: &Canvas) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for v in [c.width, c.height]
        .into_iter()
        .chain(c.pixels.iter().copied())
    {
        for b in v.to_le_bytes() {
            h = (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3);
        }
    }
    h
}

#[test]
fn shadow_fades_outward_and_spares_the_menu() {
    let c = draw_shadow(20, 10, 4);
    assert_eq!((c.width, c.height), (24, 14));
    assert_eq!(c.get(5, 5), 0);
    assert_eq!(c.get(21, 1), 0);
    assert_eq!(c.get(1, 11), 0);
    let alpha = |x: i32, y: i32| c.get(x, y) >> 24;
    assert_eq!(alpha(20, 5), 128);
    assert_eq!(alpha(23, 5), 32);
    assert!(alpha(20, 5) > alpha(21, 5) && alpha(21, 5) > alpha(23, 5));
    assert_eq!(alpha(5, 10), 128);
    assert!(alpha(21, 11) < alpha(21, 5));
    assert_eq!(c.get(20, 5) & 0x00FF_FFFF, 0);
    assert_eq!(draw_shadow(20, 10, 0).get(19, 9), 0);
}

#[test]
fn a_shadow_cut_at_a_floor_keeps_only_the_rows_above_it() {
    let full = draw_shadow(20, 10, 4);
    let c = draw_shadow_above(20, 10, 4, 10);
    assert_eq!((c.width, c.height), (24, 10), "cut at the menu's foot");
    for y in 0..10 {
        for x in 0..24 {
            assert_eq!(c.get(x, y), full.get(x, y), "({x}, {y}) as uncut");
        }
    }
    assert_eq!(
        draw_shadow_above(20, 10, 4, 99),
        full,
        "a floor past the shadow cuts nothing"
    );
}

#[test]
fn a_150_percent_context_menu_grows_its_rows_around_its_labels() {
    let theme = Theme::default().with_text_size(150);
    let items = vec![
        MenuItem::entry("Open"),
        MenuItem::Separator,
        MenuItem::entry("Paste"),
    ];
    let style = MenuStyle::context_for(&theme, &items);
    let l = MenuLayout::compute_styled(&style, &items, usize::MAX);
    assert_eq!(l.rows[0].height, 24);
    assert_eq!(l.rows[1].height, 9, "separators keep their size");
    assert_eq!(l.height, 2 * BORDER + 24 + 9 + 24);
    if !crate::text::test_tahoma() {
        return;
    }
    let text = TextRenderer::load(&theme.fonts);
    let l = MenuLayout::compute_styled(&style.fit(&text, &items, 17, 400), &items, usize::MAX);
    let mut c = Canvas::new(l.width as u32, l.height as u32);
    draw_menu(&mut c, &theme, &text, &l, &items, None, "", false);
    let ink = theme.colors.menu_text.argb();
    for (i, want) in [(0, (6, 22)), (2, (6, 18))] {
        let r = l.rows[i];
        let rows: Vec<i32> = (r.y - 3..r.bottom() + 3)
            .filter(|&y| (r.x..r.right()).any(|x| c.get(x, y) == ink))
            .collect();
        let (top, bottom) = (rows[0] - r.y, rows[rows.len() - 1] - r.y);
        assert_eq!((top, bottom), want, "row {i}'s ink rows");
    }
}

#[test]
fn disabled_text_is_etched_and_long_labels_get_an_ellipsis() {
    let theme = wide_theme();
    let text = TextRenderer::load(&FontConfig::default());
    let mut off = MenuItem::entry("Off");
    if let MenuItem::Entry { enabled, .. } = &mut off {
        *enabled = false;
    }
    let items = vec![off];
    let l = MenuLayout::compute(&theme.metrics, &items);
    let mut c = Canvas::new(l.width as u32, l.height as u32);
    draw_menu(&mut c, &theme, &text, &l, &items, None, "", false);
    let gray = theme.colors.gray_text.argb();
    let white = theme.colors.button_hilight.argb();
    let r = l.rows[0];
    let text_rect = Rect::new(r.x + l.text_x, r.y, r.width - l.text_x, r.height);
    let mut etched = false;
    let mut whites = 0;
    for y in text_rect.y..text_rect.bottom() {
        for x in text_rect.x..text_rect.right() {
            if c.get(x, y) == white {
                whites += 1;
            }
            if c.get(x, y) == gray && c.get(x + 1, y + 1) == white {
                etched = true;
            }
        }
    }
    assert!(etched && whites > 0);

    let long = MenuItem::entry("A very long application name that cannot fit");
    let items = vec![long];
    let mut l = MenuLayout::compute(&theme.metrics, &items);
    let narrow = 90;
    l.width = narrow;
    for r in &mut l.rows {
        r.width = narrow - 2 * BORDER;
    }
    let mut c = Canvas::new(l.width as u32, l.height as u32);
    draw_menu(&mut c, &theme, &text, &l, &items, None, "", false);
    let mut ref_c = Canvas::new(l.width as u32, l.height as u32);
    let r = l.rows[0];
    let text_rect = Rect::new(
        r.x + l.text_x,
        r.y,
        (r.width - l.text_x - ARROW_SPACE).max(0),
        r.height,
    );
    ref_c.fill_rect(ref_c.bounds(), theme.colors.menu);
    let shown = text.ellipsize(
        "A very long application name that cannot fit",
        text_rect.width,
        false,
        theme.fonts.size_px,
    );
    assert!(shown.ends_with("..."));
    text.draw(
        &mut ref_c,
        text_rect,
        &shown,
        false,
        theme.colors.menu_text,
        Align::Left,
        theme.fonts.size_px,
    );
    for y in text_rect.y..text_rect.bottom() {
        for x in text_rect.x..text_rect.right() {
            assert_eq!(c.get(x, y), ref_c.get(x, y), "at {x},{y}");
        }
    }
}

#[test]
fn layout_rows() {
    let m = wide_metrics();
    let l = MenuLayout::compute(&m, &items());
    assert_eq!(l.width, 180);
    assert_eq!(l.rows[0], Rect::new(3, 3, 174, 20));
    assert_eq!(l.rows[1], Rect::new(3, 23, 174, 8));
    assert_eq!(l.rows[2], Rect::new(3, 31, 174, 20));
    assert_eq!(l.height, 3 + 20 + 8 + 20 + 3);
    assert_eq!(l.columns, 1);
}

#[test]
fn compute_columns_wraps_by_count() {
    let m = Metrics {
        menu_width: 180,
        ..wide_metrics()
    };
    let entries: Vec<MenuItem> = (0..5).map(|i| MenuItem::entry(i.to_string())).collect();
    let l = MenuLayout::compute_columns(&m, &entries, 2);
    assert_eq!(l.columns, 3);
    assert_eq!(l.width, 3 * m.menu_width);
    assert_eq!(l.rows[2].x, BORDER + m.menu_width);
    assert_eq!(l.rows[3].x, BORDER + m.menu_width);
    assert_eq!(l.height, 2 * m.menu_item_height + 2 * BORDER);
}

#[test]
fn compute_columns_counts_separators_as_rows() {
    // A separator still ends a column at max_rows, even though its own
    // height differs from an entry's.
    let m = Metrics {
        menu_width: 180,
        ..wide_metrics()
    };
    let items = vec![
        MenuItem::entry("a"),
        MenuItem::Separator,
        MenuItem::entry("b"),
    ];
    let l = MenuLayout::compute_columns(&m, &items, 2);
    assert_eq!(l.columns, 2);
    assert_eq!(l.rows[0].x, BORDER);
    assert_eq!(l.rows[1].x, BORDER);
    assert_eq!(l.rows[2].x, BORDER + m.menu_width);
}

#[test]
fn submenu_max_rows_from_usable_height() {
    let m = wide_metrics();
    assert_eq!(submenu_max_rows(&m, 2 * BORDER + 3 * m.menu_item_height), 3);
    assert_eq!(submenu_max_rows(&m, 0), 1);
}

#[test]
fn place_submenu_aligns_row_and_clamps() {
    let usable = Rect::new(0, 0, 800, 600);
    let parent = Rect::new(100, 50, 180, 200);
    let row = Rect::new(103, 90, 174, 20);
    let (x, y) = place_submenu(parent, row, (180, 100), usable);
    assert_eq!((x, y), (parent.right() - 2 * BORDER, row.y - 3));
    let (_, y) = place_submenu(parent, row, (180, 550), usable);
    assert_eq!(y, usable.bottom() - 550);
    let far_right = Rect::new(700, 50, 180, 200);
    let far_row = Rect::new(703, 90, 174, 20);
    let (x, _) = place_submenu(far_right, far_row, (180, 100), usable);
    assert_eq!(x, far_right.x - 180 + 2 * BORDER);
}

#[test]
fn popup_opens_at_the_pointer_and_flips_up_over_the_taskbar() {
    let usable = Rect::new(0, 0, 1920, 1050);
    assert_eq!(place_popup((100, 100), (150, 116), usable), (100, 100));
    assert_eq!(place_popup((120, 1066), (150, 116), usable), (120, 934));
    assert_eq!(place_popup((1900, 100), (150, 116), usable), (1750, 100));
    assert_eq!(place_popup((10, 10), (150, 2000), usable), (10, 0));
}

#[test]
fn a_child_menu_overlaps_its_parent_by_both_borders() {
    const PARENT_X: i32 = 68;
    const PARENT_W: i32 = 127;
    const CHILD_X: i32 = 189;
    const ROW_Y: i32 = 180;
    const CHILD_Y: i32 = 177;
    let usable = Rect::new(0, 0, 792, 573);
    let parent = Rect::new(PARENT_X, 22, PARENT_W, 238);
    let row = Rect::new(parent.x + BORDER, ROW_Y, PARENT_W - 2 * BORDER, 17);
    assert_eq!(
        place_submenu(parent, row, (118, 120), usable),
        (CHILD_X, CHILD_Y)
    );
    assert_eq!(CHILD_X + BORDER, parent.right() - BORDER);
}

#[test]
fn place_submenu_pins_to_usable_left_when_flip_still_overflows() {
    // A submenu wider than the usable area: after flipping left of the
    // parent it would still start left of `usable.x`, so it pins there
    // instead of running off-screen.
    let usable = Rect::new(0, 0, 400, 600);
    let parent = Rect::new(0, 50, 180, 200);
    let row = Rect::new(3, 90, 174, 20);
    let wide_3col = (3 * 180, 300); // three columns, wider than `usable`
    let (x, _) = place_submenu(parent, row, wide_3col, usable);
    assert_eq!(x, usable.x);
}

#[test]
fn open_submenu_wiring_aligns_first_entry_with_programs_row() {
    // Composes a root row layout, the row-to-absolute translation and
    // `place_submenu`; the row must be absolute, not menu-local.
    let m = wide_metrics();
    let root_items = vec![
        MenuItem::entry("Programs"),
        MenuItem::Separator,
        MenuItem::entry("Terminal"),
    ];
    let root_layout = MenuLayout::compute(&m, &root_items);
    let parent = Rect::new(0, 600, root_layout.width, root_layout.height);
    let local_row = root_layout.rows[0]; // the "Programs" row
    assert_eq!(local_row.y, BORDER); // the first row sits right under the border
    let row = Rect::new(
        parent.x + local_row.x,
        parent.y + local_row.y,
        local_row.width,
        local_row.height,
    );

    let apps: Vec<MenuItem> = (0..5).map(|i| MenuItem::entry(format!("app{i}"))).collect();
    let submenu_layout = MenuLayout::compute_columns(&m, &apps, submenu_max_rows(&m, 720));
    let usable = Rect::new(0, 0, 800, 720);
    let (_, y) = place_submenu(
        parent,
        row,
        (submenu_layout.width, submenu_layout.height),
        usable,
    );
    assert_eq!(y, 600 + BORDER - 3);
}

#[test]
fn hit_skips_separators() {
    let m = wide_metrics();
    let l = MenuLayout::compute(&m, &items());
    assert_eq!(l.hit(50, 10), Some(0));
    assert_eq!(l.hit(50, 26), None);
    assert_eq!(l.hit(50, 40), Some(2));
    assert_eq!(l.hit(-1, 10), None);
    assert_eq!(l.hit(50, 200), None);
}

#[test]
fn hit_uses_kind_not_height() {
    // menu_item_height equal to the separator height must not make
    // entries unclickable or the separator clickable.
    let m = Metrics {
        menu_item_height: 8,
        ..wide_metrics()
    };
    let l = MenuLayout::compute(&m, &items());
    assert_eq!(l.hit(50, l.rows[0].y + 2), Some(0));
    assert_eq!(l.hit(50, l.rows[1].y + 2), None);
}

#[test]
fn compute_with_no_items() {
    let m = wide_metrics();
    let l = MenuLayout::compute(&m, &[]);
    assert_eq!(l.height, 2 * BORDER);
    assert!(l.rows.is_empty());
    assert_eq!(l.hit(5, 5), None);
}

#[test]
fn hovered_row_uses_hilight() {
    let theme = wide_theme();
    let text = TextRenderer::load(&FontConfig::default());
    let l = MenuLayout::compute(&theme.metrics, &items());
    let mut c = Canvas::new(l.width as u32, l.height as u32);
    draw_menu(&mut c, &theme, &text, &l, &items(), Some(2), "", false);
    assert_eq!(c.get(5, l.rows[2].y + 2), theme.colors.hilight.argb());
    assert_eq!(c.get(5, l.rows[0].y + 2), theme.colors.menu.argb());
    assert_eq!(c.get(0, 10), theme.colors.button_face.argb());
    assert_eq!(c.get(1, 10), theme.colors.button_hilight.argb());
}

#[test]
fn scaled_layout_doubles_rows() {
    let m = wide_metrics();
    let l = MenuLayout::compute(&m, &items());
    let s = l.scaled(2);
    assert_eq!(s.width, 360);
    assert_eq!(s.height, l.height * 2);
    assert_eq!(s.rows[0], Rect::new(6, 6, 348, 40));
    assert_eq!(s.selectable, l.selectable);
    assert_eq!(s.columns, 1);
}

#[test]
fn hovered_row_at_scale_two() {
    let theme = wide_theme().scaled(2);
    let text = TextRenderer::load(&FontConfig::default());
    let l = MenuLayout::compute(&wide_theme().metrics, &items()).scaled(2);
    let mut c = Canvas::new(l.width as u32, l.height as u32);
    draw_menu(&mut c, &theme, &text, &l, &items(), Some(2), "", false);
    assert_eq!(c.get(10, l.rows[2].y + 4), theme.colors.hilight.argb());
    assert_eq!(c.get(10, l.rows[0].y + 4), theme.colors.menu.argb());
    assert_eq!(c.get(0, 20), theme.colors.button_face.argb());
    assert_eq!(c.get(1, 20), theme.colors.button_face.argb());
    assert_eq!(c.get(2, 20), theme.colors.button_hilight.argb());
    assert_eq!(c.get(3, 20), theme.colors.button_hilight.argb());
    let sep = l.rows[1];
    let y = sep.y + sep.height / 2 - 2;
    assert_eq!(c.get(40, y), theme.colors.button_shadow.argb());
    assert_eq!(c.get(40, y + 1), theme.colors.button_shadow.argb());
    assert_eq!(c.get(40, y + 2), theme.colors.button_hilight.argb());
}

#[test]
fn root_style_reserves_band_and_tall_rows() {
    let m = Metrics {
        startmenu_width: 232,
        ..wide_metrics()
    };
    let style = MenuStyle::root(&th(&m));
    assert_eq!(style.icon_px(), 24);
    assert_eq!(MenuStyle::submenu(&th(&m)).icon_px(), 16);
    let l = MenuLayout::compute_styled(&style, &items(), usize::MAX);
    assert_eq!(l.width, m.startmenu_width);
    assert_eq!(l.band, m.startmenu_band_width);
    assert_eq!(
        l.rows[0],
        Rect::new(
            m.startmenu_band_width + BORDER,
            BORDER,
            m.startmenu_width - m.startmenu_band_width - 2 * BORDER,
            m.startmenu_item_height
        )
    );
    assert_eq!(l.text_x, ROOT_TEXT_X);
    let s = l.scaled(2);
    assert_eq!(s.band, 2 * m.startmenu_band_width);
    assert_eq!(s.text_x, 2 * ROOT_TEXT_X);
    assert_eq!(s.column_width, 2 * l.column_width);
}

#[test]
fn fit_widens_for_long_labels_only() {
    let m = Metrics {
        menu_width: 180,
        ..wide_metrics()
    };
    let text = TextRenderer::load(&FontConfig::default());
    let style = MenuStyle::submenu(&th(&m));
    assert_eq!(style.fit(&text, &items(), 11, i32::MAX).width, m.menu_width);
    let long = vec![MenuItem::entry("x".repeat(80))];
    let wide = style.fit(&text, &long, 11, i32::MAX);
    let label_w = text.measure(&long[0].label_for_test(), false, 11);
    assert_eq!(wide.width, BORDER + TEXT_X + label_w + RIGHT_PAD + BORDER);
    assert!(wide.width > m.menu_width);
    let fitted = MenuStyle::submenu(&Theme::default()).fit(&text, &items(), 11, i32::MAX);
    let longest = items()
        .iter()
        .map(|it| match it {
            MenuItem::Entry { label, .. } => text.measure(label, false, 11),
            MenuItem::Separator => 0,
        })
        .max()
        .unwrap();
    assert_eq!(fitted.width, BORDER + TEXT_X + longest + RIGHT_PAD + BORDER);
    let root = MenuStyle::root(&Theme::default()).fit(&text, &items(), 11, i32::MAX);
    assert_eq!(
        root.width,
        Metrics::default().startmenu_band_width
            + BORDER
            + ROOT_TEXT_X
            + longest
            + ROOT_RIGHT_PAD
            + BORDER
    );
}

#[test]
fn fit_stops_at_the_max_width_but_keeps_the_minimum() {
    let m = Metrics {
        menu_width: 180,
        ..wide_metrics()
    };
    let text = TextRenderer::load(&FontConfig::default());
    let style = MenuStyle::submenu(&th(&m));
    let long = vec![MenuItem::entry("x".repeat(1000))];
    assert_eq!(style.fit(&text, &long, 11, 640).width, 640);
    assert_eq!(style.fit(&text, &long, 11, 100).width, 180);
}

#[test]
fn band_is_drawn_with_gradient_and_banner() {
    let theme = wide_theme();
    let text = TextRenderer::load(&FontConfig::default());
    let l = MenuLayout::compute_styled(&MenuStyle::root(&theme), &items(), usize::MAX);
    let mut c = Canvas::new(l.width as u32, l.height as u32);
    draw_menu(&mut c, &theme, &text, &l, &items(), None, "Reprise", false);
    let cs = &theme.colors;
    assert_eq!(
        c.get(BORDER + 1, l.height - BORDER - 1),
        cs.menu_band.argb()
    );
    assert_eq!(c.get(BORDER + 1, BORDER), cs.menu_band_top.argb());
    let third_up = l.height - BORDER - (l.height - 2 * BORDER) / 3;
    assert_eq!(c.get(BORDER + 1, third_up), cs.gradient_menu_band.argb());
    assert_eq!(c.get(l.rows[0].x, l.rows[0].y), cs.menu.argb());
    let white = cs.menu_band_text.argb();
    let has_text = (BORDER..BORDER + l.band)
        .any(|x| (BORDER..l.height - BORDER).any(|y| c.get(x, y) == white));
    assert!(has_text, "expected banner pixels in the band");
}

#[test]
fn each_blue_schemes_start_band_pins_at_scale_1_and_2() {
    use crate::blue::Variant;
    let text = TextRenderer::load(&FontConfig::default());
    let mut got = Vec::new();
    for v in Variant::ALL {
        for d in [1, 2] {
            let theme = crate::interfaces::blue_theme(v).scaled(d);
            let style = MenuStyle {
                width: 120 * d,
                ..MenuStyle::root(&theme)
            };
            let l = MenuLayout::compute_styled(&style, &items(), usize::MAX);
            let mut c = Canvas::new(l.width as u32, l.height as u32);
            draw_menu(&mut c, &theme, &text, &l, &items(), None, "", false);
            let (inset, h) = (l.inset, l.height);
            let band = c.copy_rect(Rect::new(
                inset.left,
                inset.top,
                l.band,
                h - inset.top - inset.bottom,
            ));
            let hash = band.pixels.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, &p| {
                (h ^ u64::from(p)).wrapping_mul(0x0000_0100_0000_01b3)
            });
            got.push((format!("{v:?} {d}"), hash));
        }
    }
    crate::pins::check("blue/start_band", &got);
}

#[test]
fn entry_constructor_defaults() {
    let e = MenuItem::entry("x");
    assert!(matches!(
        e,
        MenuItem::Entry { ref label, icon: None, enabled: true, checked: false, radio: false, submenu: false, mnemonic: None, shortcut: None, bold: false, glyph: None } if label == "x"
    ));
}

#[test]
fn entry_icon_is_drawn_in_the_gutter() {
    let theme = wide_theme();
    let text = TextRenderer::load(&FontConfig::default());
    let px = theme.icon_size() as u32;
    let mut red = Canvas::new(px, px);
    red.fill_rect(red.bounds(), crate::theme::Color::rgb(255, 0, 0));
    let items = vec![MenuItem::entry("App").icon(Some(red))];
    let l = MenuLayout::compute(&theme.metrics, &items);
    let mut c = Canvas::new(l.width as u32, l.height as u32);
    draw_menu(&mut c, &theme, &text, &l, &items, None, "", false);
    let r = l.rows[0];
    assert_eq!(c.get(r.x + ICON_X + 8, r.y + r.height / 2), 0xFFFF_0000);
    assert_eq!(
        c.get(r.x + ICON_X + 16 + 2, r.y + r.height / 2),
        theme.colors.menu.argb()
    );
}

#[test]
fn disabled_entry_uses_gray_text_even_when_hovered() {
    let theme = wide_theme();
    let text = TextRenderer::load(&FontConfig::default());
    let items = vec![MenuItem::entry("Off").enabled(false)];
    let l = MenuLayout::compute(&theme.metrics, &items);
    let mut c = Canvas::new(l.width as u32, l.height as u32);
    draw_menu(&mut c, &theme, &text, &l, &items, Some(0), "", false);
    let r = l.rows[0];
    assert_eq!(c.get(r.x + 2, r.y + 2), theme.colors.hilight.argb());
    let gray = theme.colors.gray_text.argb();
    let white = theme.colors.button_hilight.argb();
    let label = (r.x + TEXT_X..r.right()).flat_map(|x| (r.y..r.bottom()).map(move |y| (x, y)));
    let (mut has_gray, mut has_white) = (false, false);
    for (x, y) in label {
        has_gray |= c.get(x, y) == gray;
        has_white |= c.get(x, y) == white;
    }
    assert!(has_gray && !has_white, "gray {has_gray} white {has_white}");
    let mut c = Canvas::new(l.width as u32, l.height as u32);
    draw_menu(&mut c, &theme, &text, &l, &items, None, "", false);
    assert_eq!(c.get(r.x + 2, r.y + 2), theme.colors.menu.argb());
}

#[test]
fn context_menus_fit_their_labels() {
    if !crate::text::test_tahoma() {
        return;
    }
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let fit = |items: &[MenuItem]| {
        let style =
            MenuStyle::context_for(&theme, items).fit(&text, items, theme.fonts.size_px, i32::MAX);
        MenuLayout::compute_styled(&style, items, usize::MAX)
    };
    let e = MenuItem::entry;
    let view = [
        e("Toolbars").submenu(true),
        e("Status Bar").checked(true),
        e("Explorer Bar").submenu(true),
        MenuItem::Separator,
        e("Thumbnails"),
        e("Tiles").radio(true).checked(true),
        e("Icons"),
        e("List"),
        e("Details"),
        MenuItem::Separator,
        e("Arrange Icons by").submenu(true),
        MenuItem::Separator,
        e("Choose Details..."),
        e("Go To").submenu(true),
        e("Refresh"),
    ];
    assert_eq!(fit(&view).width, 127);
    let arrange = [
        e("Name"),
        e("Size"),
        e("Type"),
        e("Modified"),
        MenuItem::Separator,
        e("Show in Groups").enabled(false),
        e("Auto Arrange"),
        e("Align to Grid").checked(true),
        MenuItem::Separator,
        e("Show Desktop Icons").checked(true),
        e("Lock Web Items on Desktop"),
        e("Run Desktop Cleanup Wizard"),
    ];
    assert_eq!(fit(&arrange).width, 183);
    let icon = || Some(Canvas::new(16, 16));
    let new = [
        e("Folder").icon(icon()),
        e("Shortcut").icon(icon()),
        MenuItem::Separator,
        e("Text Document").icon(icon()),
        e("Compressed (zipped) Folder").icon(icon()),
    ];
    let l = fit(&new);
    assert_eq!((l.width, l.rows[0].height), (199, 19));
    let mut c = Canvas::new(l.width as u32, l.height as u32);
    draw_menu(&mut c, &theme, &text, &l, &new, None, "", false);
    let r = l.rows[0];
    let ink = theme.colors.menu_text.argb();
    let first =
        (l.icon_x + 16..r.width).find(|&x| (0..r.height).any(|y| c.get(r.x + x, r.y + y) == ink));
    assert_eq!(first, Some(28), "Folder's first inked column");
}

#[test]
fn a_context_rows_label_inks_17_px_in() {
    const EXPECTED_INK_X: i32 = 17;
    if !crate::text::test_tahoma() {
        return;
    }
    let theme = wide_theme();
    let text = TextRenderer::load(&FontConfig::default());
    for (label, top) in [("Properties", 4), ("New", 4), ("List", 4), ("Refresh", 3)] {
        let items = vec![MenuItem::entry(label)];
        let style = MenuStyle::context(&theme, ContextBitmaps::None);
        let l = MenuLayout::compute_styled(&style, &items, usize::MAX);
        let mut c = Canvas::new(l.width as u32, l.height as u32);
        draw_menu(&mut c, &theme, &text, &l, &items, None, "", false);
        let r = l.rows[0];
        let ink = theme.colors.menu_text.argb();
        let inked = |x: i32, y: i32| c.get(r.x + x, r.y + y) == ink;
        let first = (0..r.width).find(|&x| (0..r.height).any(|y| inked(x, y)));
        assert_eq!(first, Some(EXPECTED_INK_X), "{label}'s first inked column");
        let first_row = (0..r.height).find(|&y| (0..r.width).any(|x| inked(x, y)));
        assert_eq!(first_row, Some(top), "{label}'s first inked row");
    }
}

#[test]
fn a_checked_row_inks_its_mark_4_px_in_and_6_down() {
    const EXPECTED_ROW_H: i32 = 17;
    const EXPECTED_INK: (i32, i32) = (4, 6);
    const EXPECTED_MARK: [&str; 7] = [
        "......X", ".....XX", "X...XXX", "XX.XXX.", "XXXXX..", ".XXX...", "..X....",
    ];
    let theme = wide_theme();
    let text = TextRenderer::load(&FontConfig::default());
    let items = vec![MenuItem::entry("Lock the Taskbar").checked(true)];
    let style = MenuStyle::context(&theme, ContextBitmaps::None);
    let l = MenuLayout::compute_styled(&style, &items, usize::MAX);
    let mut c = Canvas::new(l.width as u32, l.height as u32);
    draw_menu(&mut c, &theme, &text, &l, &items, None, "", false);
    let r = l.rows[0];
    assert_eq!(r.height, EXPECTED_ROW_H);
    let ink = theme.colors.menu_text.argb();
    for y in 0..r.height {
        for x in 0..l.text_x - 2 {
            let (gx, gy) = (x - EXPECTED_INK.0, y - EXPECTED_INK.1);
            let want = (0..7).contains(&gx)
                && (0..7).contains(&gy)
                && EXPECTED_MARK[gy as usize].as_bytes()[gx as usize] == b'X';
            assert_eq!(c.get(r.x + x, r.y + y) == ink, want, "row pixel ({x}, {y})");
        }
    }
}

#[test]
fn a_radio_entry_draws_a_bullet_instead_of_a_check() {
    const ROW_X: i32 = 163;
    const ROW_Y: i32 = 213;
    const ROW_H: i32 = 17;
    const EXPECTED_BULLET_X: i32 = 168;
    const EXPECTED_BULLET_Y: i32 = 218;
    const EXPECTED_BULLET_D: i32 = 6;
    const CHECK_ROW_X: i32 = 284;
    const EXPECTED_CHECK_X: i32 = 288;
    assert_eq!(
        BULLET_X - CHECK_X,
        (EXPECTED_BULLET_X - ROW_X) - (EXPECTED_CHECK_X - CHECK_ROW_X)
    );
    let theme = wide_theme();
    let text = TextRenderer::load(&FontConfig::default());
    let items = vec![MenuItem::entry("Tiles").checked(true).radio(true)];
    let style = MenuStyle::context(&theme, ContextBitmaps::None);
    let l = MenuLayout::compute_styled(&style, &items, usize::MAX);
    let mut c = Canvas::new(l.width as u32, l.height as u32);
    draw_menu(&mut c, &theme, &text, &l, &items, None, "", false);
    let r = l.rows[0];
    assert_eq!(r.height, ROW_H, "a context menu's rows");
    let ink = theme.colors.menu_text.argb();
    let gutter = Rect::new(r.x, r.y, l.text_x, r.height);
    let cols: Vec<i32> = (gutter.x..gutter.right())
        .filter(|&x| (gutter.y..gutter.bottom()).any(|y| c.get(x, y) == ink))
        .collect();
    let rows: Vec<i32> = (gutter.y..gutter.bottom())
        .filter(|&y| (gutter.x..gutter.right()).any(|x| c.get(x, y) == ink))
        .collect();
    assert_eq!(cols.len(), EXPECTED_BULLET_D as usize);
    assert_eq!(rows.len(), EXPECTED_BULLET_D as usize);
    assert_eq!(cols[0] - r.x, BULLET_X);
    assert_eq!(rows[0] - r.y, EXPECTED_BULLET_Y - ROW_Y);
    assert_eq!(c.get(cols[0], rows[0]), theme.colors.menu.argb());
    assert_eq!(c.get(cols[1], rows[0]), ink);
    for x in &cols {
        assert_eq!(c.get(*x, rows[2]), ink);
    }
    let (gx, gy) = (r.x + CHECK_X, r.y + (r.height - 7) / 2);
    assert_eq!(c.get(gx + 6, gy), theme.colors.menu.argb());
}

#[test]
#[ignore]
fn render_menu_radio() {
    let theme = wide_theme();
    let text = TextRenderer::load(&FontConfig::default());
    let row = |label: &str, checked: bool, radio: bool| {
        MenuItem::entry(label).checked(checked).radio(radio)
    };
    let items = vec![
        row("Thumbnails", false, true),
        row("Tiles", true, true),
        row("Icons", false, true),
        MenuItem::Separator,
        row("Status Bar", true, false),
    ];
    let style = MenuStyle::context(&theme, ContextBitmaps::None);
    let l = MenuLayout::compute_styled(&style, &items, usize::MAX);
    let mut c = Canvas::new(l.width as u32, l.height as u32);
    draw_menu(&mut c, &theme, &text, &l, &items, Some(1), "", false);
    crate::raster::write_preview(&c, "menu-radio-preview.png");
}

fn nested_tree() -> MenuTree {
    let arrange = MenuTree::flat(vec![
        MenuItem::item("&Name"),
        MenuItem::item("Si&ze"),
        MenuItem::item("T&ype"),
    ]);
    let mut t = MenuTree::new();
    t.push(MenuItem::item("Status &Bar"));
    t.push_sub(MenuItem::item("Arrange &Icons by"), arrange);
    t.push(MenuItem::Separator);
    t.push(MenuItem::item("R&efresh"));
    t
}

fn nested_ctx<'a>(theme: &'a Theme, text: &'a TextRenderer) -> MenuCtx<'a> {
    MenuCtx {
        text,
        theme,
        px: theme.fonts.size_px,
        within: Rect::new(0, 0, 800, 600),
    }
}

fn row_point(p: &PopupMenu, lvl: usize, i: usize) -> (i32, i32) {
    let level = &p.levels()[lvl];
    let r = level.layout.rows[i];
    (
        level.origin.0 + r.x + 5,
        level.origin.1 + r.y + r.height / 2,
    )
}

/// The nested chain with its submenu open, from the keyboard.
fn opened_nested(t: &MenuTree, cx: &MenuCtx) -> PopupMenu {
    let mut p = PopupMenu::open(t, (100, 50), cx, true);
    p.key(t, MenuKey::Down, cx);
    p.key(t, MenuKey::Right, cx);
    assert_eq!(p.depth(), 2);
    p
}

#[test]
fn a_hosted_chain_draws_nothing_in_its_window_and_opens_off_the_point_asked() {
    let text = TextRenderer::load(&FontConfig::default());
    let theme = Theme::default();
    let cx = MenuCtx {
        within: Rect::new(0, 0, 120, 80),
        ..nested_ctx(&theme, &text)
    };
    let t = nested_tree();
    let mut p = PopupMenu::open(&t, (100, 70), &cx, false);
    assert_ne!(
        (p.rect().x, p.rect().y),
        (100, 70),
        "pushed inside the window"
    );
    assert_eq!(
        p.spots(),
        [Spot {
            rect: p.rect(),
            anchor: Anchor::Point(100, 70)
        }]
    );
    let drawn = |p: &PopupMenu| {
        let mut c = Canvas::new(300, 200);
        p.draw(&mut c, &theme, &text, false);
        c.pixels.iter().any(|&px| px != 0)
    };
    assert!(drawn(&p));
    p.host();
    assert!(!drawn(&p));
    p.place(0, (100, 70));
    assert_eq!(p.rect().x, 100);
    assert_eq!(p.hit(101, 72), Some((0, None)), "hit where it was put");
}

#[test]
fn levels_drawn_alone_and_stacked_make_the_whole_chain() {
    let text = TextRenderer::load(&FontConfig::default());
    for interface in [Interface::Classic, Interface::Blue, Interface::Glass] {
        let base = crate::interfaces::theme_for(interface);
        let cx = nested_ctx(&base, &text);
        let t = nested_tree();
        let p = opened_nested(&t, &cx);
        let m = level_shadow(&base);
        for s in [1, 2] {
            let theme = base.scaled(s);
            let mut whole = Canvas::new(800 * s as u32, 600 * s as u32);
            p.draw(&mut whole, &theme, &text, false);
            let mut stacked = Canvas::new(800 * s as u32, 600 * s as u32);
            for (i, level) in p.levels().iter().enumerate() {
                let r = level.rect();
                let mut own = Canvas::new(((r.width + m) * s) as u32, ((r.height + m) * s) as u32);
                p.draw_level(i, (-r.x, -r.y), &mut own, &theme, &text, false);
                stacked.blit_over(r.x * s, r.y * s, &own);
            }
            let off = whole
                .pixels
                .iter()
                .zip(&stacked.pixels)
                .filter(|(a, b)| {
                    a.to_le_bytes()
                        .iter()
                        .zip(b.to_le_bytes())
                        .any(|(x, y)| x.abs_diff(y) > 1)
                })
                .count();
            assert_eq!(
                off, 0,
                "{interface:?} at {s}: no pixel off by more than a rounding step"
            );
        }
    }
}

#[test]
fn a_level_shadow_reaches_as_far_as_the_look_casts_it() {
    assert_eq!(
        level_shadow(&Theme::default()),
        0,
        "Classic chains cast none"
    );
    let glass = crate::interfaces::theme_for(Interface::Glass);
    assert_eq!(level_shadow(&glass), crate::style::DropShadow::N as i32);
}

#[test]
fn a_placed_level_is_hit_where_it_was_put() {
    let theme = wide_theme();
    let text = TextRenderer::load(&FontConfig::default());
    let cx = nested_ctx(&theme, &text);
    let t = nested_tree();
    let mut p = opened_nested(&t, &cx);
    let before = row_point(&p, 1, 0);
    p.place(1, (-300, 900));
    assert_eq!(p.levels()[1].origin, (-300, 900));
    assert_ne!(p.hit(before.0, before.1), Some((1, Some(0))));
    let (x, y) = row_point(&p, 1, 0);
    assert_eq!(
        p.hit(x, y),
        Some((1, Some(0))),
        "the moved row takes the point"
    );
    assert_eq!(
        p.click(&t, x, y, &cx),
        MenuAction::Chosen(vec![1, 0]),
        "a press on it picks it"
    );
}

#[test]
fn a_submenu_anchor_puts_the_child_where_place_submenu_does() {
    let theme = wide_theme();
    let text = TextRenderer::load(&FontConfig::default());
    let cx = nested_ctx(&theme, &text);
    let t = nested_tree();
    let mut p = opened_nested(&t, &cx);
    p.place(0, (400, 50));
    assert_eq!(p.submenu_anchor(0), None, "the root opens at its point");
    let (anchor, dy) = p.submenu_anchor(1).expect("the submenu's anchor");
    let (parent, child) = (p.levels()[0].rect(), p.levels()[1].rect());
    let row = p.levels()[0].layout.rows[1];
    let row_abs = Rect::new(parent.x + row.x, parent.y + row.y, row.width, row.height);
    let size = (child.width, child.height);
    // Anchored on the rect's right end, growing right and down; flipped, on its left end
    // growing left.
    let right = (parent.x + anchor.right(), parent.y + anchor.y + dy);
    let left = (parent.x + anchor.x - child.width, right.1);
    let open = Rect::new(0, 0, 800, 600);
    assert_eq!(right, place_submenu(parent, row_abs, size, open));
    let tight = Rect::new(0, 0, parent.right() + 10, 600);
    assert_eq!(left, place_submenu(parent, row_abs, size, tight));
}

#[test]
fn a_chain_opened_from_the_keyboard_underlines_without_the_hosts_cue() {
    let theme = wide_theme();
    let text = TextRenderer::load(&FontConfig::default());
    let cx = nested_ctx(&theme, &text);
    let t = nested_tree();
    let drawn = |p: &PopupMenu, cue: bool| {
        let mut c = Canvas::new(800, 600);
        p.draw(&mut c, &theme, &text, cue);
        c.pixels
    };
    let by_pointer = PopupMenu::open(&t, (100, 50), &cx, false);
    assert!(
        drawn(&by_pointer, false) != drawn(&by_pointer, true),
        "pointer: the cue decides"
    );
    let by_key = PopupMenu::open(&t, (100, 50), &cx, true);
    assert!(
        drawn(&by_key, false) == drawn(&by_key, true),
        "keyboard: underlined"
    );
}

#[test]
fn a_submenu_opens_on_right_and_closes_on_left() {
    let theme = wide_theme();
    let text = TextRenderer::load(&FontConfig::default());
    let cx = nested_ctx(&theme, &text);
    let t = nested_tree();
    let mut p = PopupMenu::open(&t, (100, 50), &cx, true);
    assert_eq!((p.depth(), p.hovered()), (1, Some(0)));
    assert_eq!(p.key(&t, MenuKey::Right, &cx), MenuAction::None);
    p.key(&t, MenuKey::Down, &cx);
    assert_eq!(p.hovered(), Some(1));
    assert_eq!(p.key(&t, MenuKey::Right, &cx), MenuAction::Redraw);
    assert_eq!(p.depth(), 2);
    assert_eq!(p.hovered(), Some(0), "the child's first row is picked");
    let parent = p.levels()[0].rect();
    let r = p.levels()[0].layout.rows[1];
    let row = Rect::new(parent.x + r.x, parent.y + r.y, r.width, r.height);
    let child = p.levels()[1].rect();
    assert_eq!(
        (child.x, child.y),
        place_submenu(parent, row, (child.width, child.height), cx.within)
    );
    assert_eq!(child.x + BORDER, parent.right() - BORDER);
    p.key(&t, MenuKey::Down, &cx);
    assert_eq!(
        p.key(&t, MenuKey::Enter, &cx),
        MenuAction::Chosen(vec![1, 1])
    );
    let mut p = PopupMenu::open(&t, (100, 50), &cx, true);
    p.key(&t, MenuKey::Down, &cx);
    p.key(&t, MenuKey::Right, &cx);
    assert_eq!(p.key(&t, MenuKey::Left, &cx), MenuAction::Redraw);
    assert_eq!(p.depth(), 1);
    assert_eq!(p.key(&t, MenuKey::Left, &cx), MenuAction::None);
    assert_eq!(p.key(&t, MenuKey::Escape, &cx), MenuAction::Close);
    let mut p = PopupMenu::open(&t, (100, 50), &cx, true);
    assert_eq!(p.key(&t, MenuKey::Char('i'), &cx), MenuAction::Redraw);
    assert_eq!(p.depth(), 2);
    assert_eq!(
        p.key(&t, MenuKey::Char('z'), &cx),
        MenuAction::Chosen(vec![1, 1])
    );
}

fn drop_rows(default_enabled: bool) -> Vec<MenuItem> {
    vec![
        MenuItem::item("&Copy Here"),
        MenuItem::item("&Move Here")
            .bold(true)
            .enabled(default_enabled),
        MenuItem::item("Create &Shortcuts Here"),
        MenuItem::Separator,
        MenuItem::item("Cancel"),
    ]
}

#[test]
fn enter_takes_the_highlighted_row_else_the_bold_one() {
    assert_eq!(default_row(&drop_rows(true)), Some(1));
    assert_eq!(enter_row(&drop_rows(true), None), Some(1));
    assert_eq!(enter_row(&drop_rows(true), Some(4)), Some(4));
    assert_eq!(default_row(&drop_rows(false)), None, "a greyed default");
    assert_eq!(enter_row(&drop_rows(false), None), None);
    assert_eq!(enter_row(&items(), None), None, "no bold row");

    let theme = wide_theme();
    let text = TextRenderer::load(&FontConfig::default());
    let cx = nested_ctx(&theme, &text);
    let t = MenuTree::flat(drop_rows(true));
    let mut p = PopupMenu::open(&t, (100, 50), &cx, false);
    assert_eq!(p.key(&t, MenuKey::Enter, &cx), MenuAction::Chosen(vec![1]));
    assert_eq!(p.hovered(), Some(1), "the default row lights up");
    let mut p = PopupMenu::open(&t, (100, 50), &cx, false);
    p.key(&t, MenuKey::Up, &cx);
    assert_eq!(p.key(&t, MenuKey::Enter, &cx), MenuAction::Chosen(vec![4]));
    let t = MenuTree::flat(drop_rows(false));
    let mut p = PopupMenu::open(&t, (100, 50), &cx, false);
    assert_eq!(p.key(&t, MenuKey::Enter, &cx), MenuAction::Redraw);
    assert_eq!(p.hovered(), None);
}

#[test]
fn glass_arrow_keys_light_greyed_rows_and_enter_on_one_does_nothing() {
    use crate::interface::Interface;
    let edit = || {
        vec![
            MenuItem::item("&Undo").enabled(false),
            MenuItem::Separator,
            MenuItem::item("Cu&t").enabled(false),
            MenuItem::item("&Paste"),
        ]
    };
    let text = TextRenderer::load(&FontConfig::default());
    let t = MenuTree::flat(edit());
    let glass = crate::interfaces::theme_for(Interface::Glass);
    assert_eq!(KeyStops::of(&glass), KeyStops::Entries);
    let cx = nested_ctx(&glass, &text);
    let mut p = PopupMenu::open(&t, (100, 50), &cx, true);
    assert_eq!(
        p.hovered(),
        Some(0),
        "opened from the keys on a greyed Undo"
    );
    assert_eq!(p.key(&t, MenuKey::Enter, &cx), MenuAction::None);
    assert_eq!(p.depth(), 1, "the menu stays open");
    p.key(&t, MenuKey::Down, &cx);
    assert_eq!(
        p.hovered(),
        Some(2),
        "Down passes the separator onto a greyed Cut"
    );
    p.key(&t, MenuKey::Down, &cx);
    assert_eq!(p.key(&t, MenuKey::Enter, &cx), MenuAction::Chosen(vec![3]));
    let (x, y) = row_point(&p, 0, 2);
    assert_eq!(
        p.click(&t, x, y, &cx),
        MenuAction::None,
        "a click on a greyed row"
    );

    for interface in [Interface::Classic, Interface::Blue] {
        let theme = crate::interfaces::theme_for(interface);
        assert_eq!(KeyStops::of(&theme), KeyStops::Enabled, "{interface:?}");
        let cx = nested_ctx(&theme, &text);
        let mut p = PopupMenu::open(&t, (100, 50), &cx, true);
        assert_eq!(p.hovered(), Some(3), "{interface:?} opens on Paste");
        p.key(&t, MenuKey::Up, &cx);
        assert_eq!(p.hovered(), Some(3), "{interface:?} skips the greyed rows");
    }
}

#[test]
fn enter_opens_a_bold_submenu_row() {
    let theme = wide_theme();
    let text = TextRenderer::load(&FontConfig::default());
    let cx = nested_ctx(&theme, &text);
    let mut t = nested_tree();
    t[1] = t[1].clone().bold(true);
    let mut p = PopupMenu::open(&t, (100, 50), &cx, false);
    assert_eq!(p.key(&t, MenuKey::Enter, &cx), MenuAction::Redraw);
    assert_eq!(p.depth(), 2);
    assert_eq!(p.levels()[0].hovered, Some(1));
    assert_eq!(p.hovered(), Some(0));
}

#[test]
fn hovering_a_submenu_row_opens_it_and_leaving_it_closes_it() {
    let theme = wide_theme();
    let text = TextRenderer::load(&FontConfig::default());
    let cx = nested_ctx(&theme, &text);
    let t = nested_tree();
    let mut p = PopupMenu::open(&t, (100, 50), &cx, false);
    let (x, y) = row_point(&p, 0, 1);
    assert!(p.motion(&t, x, y, &cx));
    assert_eq!(p.depth(), 2, "hovering the row opens its child");
    let (cxp, cyp) = row_point(&p, 1, 2);
    assert!(p.motion(&t, cxp, cyp, &cx));
    assert_eq!(p.levels()[0].hovered, Some(1));
    assert_eq!(p.levels()[1].hovered, Some(2));
    assert!(!p.motion(&t, cxp, cyp, &cx));
    assert_eq!(p.depth(), 2);
    assert_eq!(p.click(&t, cxp, cyp, &cx), MenuAction::Chosen(vec![1, 2]));
    let (px0, py0) = row_point(&p, 0, 0);
    assert!(p.motion(&t, px0, py0, &cx));
    assert_eq!(p.depth(), 1);
    assert_eq!(p.click(&t, 700, 560, &cx), MenuAction::Close);
}

#[test]
#[ignore]
fn render_nested_menu() {
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let arrange = MenuTree::flat(vec![
        MenuItem::item("&Name"),
        MenuItem::item("Si&ze"),
        MenuItem::item("T&ype"),
        MenuItem::item("Modified"),
    ]);
    let mut view = MenuTree::new();
    view.push(MenuItem::item("&Toolbars"));
    view.push(MenuItem::item("Status &Bar"));
    view.push(MenuItem::Separator);
    view.push_sub(MenuItem::item("Arrange &Icons by"), arrange);
    view.push(MenuItem::Separator);
    view.push(MenuItem::item("R&efresh"));
    let cx = MenuCtx {
        text: &text,
        theme: &theme,
        px: theme.fonts.size_px,
        within: Rect::new(0, 0, 400, 300),
    };
    let mut p = PopupMenu::open(&view, (10, 10), &cx, false);
    p.key(&view, MenuKey::Down, &cx);
    p.key(&view, MenuKey::Down, &cx);
    p.key(&view, MenuKey::Down, &cx);
    p.key(&view, MenuKey::Right, &cx);
    let mut c = Canvas::new(400, 300);
    c.fill_rect(c.bounds(), theme.colors.button_face);
    p.draw(&mut c, &theme, &text, true);
    crate::raster::write_preview(&c, "menu-nested-preview.png");
}

#[test]
fn popup_levels_are_scaled_to_the_theme() {
    let theme = wide_theme().scaled(2);
    let text = TextRenderer::load(&FontConfig::default());
    let mut view = MenuTree::new();
    view.push(MenuItem::item("&Open"));
    view.push(MenuItem::item("E&xit"));
    let cx = MenuCtx {
        text: &text,
        theme: &wide_theme(),
        px: wide_theme().fonts.size_px,
        within: Rect::new(0, 0, 400, 300),
    };
    let p = PopupMenu::open(&view, (10, 10), &cx, false);
    let level = p.levels[0].rect();
    let fill = crate::theme::Color::rgb(1, 2, 3);
    let mut c = Canvas::new(800, 600);
    c.fill_rect(c.bounds(), fill);
    p.draw(&mut c, &theme, &text, false);
    let (far_x, far_y) = (level.right() * 2 - 1, level.bottom() * 2 - 1);
    assert_ne!(
        c.get(far_x, far_y),
        fill.argb(),
        "frame reaches the scaled edge"
    );
    assert_eq!(c.get(far_x + 1, far_y), fill.argb(), "and stops there");
    assert_eq!(
        c.get(20, 20),
        theme.colors.menu.argb(),
        "origin is scaled too"
    );
    assert_eq!(c.get(12, 12), fill.argb());
}

#[test]
fn draw_menu_at_matches_a_blit_of_draw_menu() {
    let theme = wide_theme();
    let text = TextRenderer::load(&FontConfig::default());
    let items = vec![
        MenuItem::item("&Open"),
        MenuItem::Separator,
        MenuItem::item("Pr&operties"),
    ];
    let l = MenuLayout::compute(&theme.metrics, &items);
    let mut local = Canvas::new(l.width as u32, l.height as u32);
    draw_menu(&mut local, &theme, &text, &l, &items, Some(2), "", true);
    let mut copied = Canvas::new(200, 120);
    copied.blit_canvas(13, 7, &local);
    let mut direct = Canvas::new(200, 120);
    draw_menu_at(
        &mut direct,
        (13, 7),
        &theme,
        &text,
        &l,
        &items,
        Some(2),
        "",
        true,
    );
    assert_eq!(direct.pixels, copied.pixels);
}

#[test]
fn submenu_entry_draws_right_arrow() {
    let theme = wide_theme();
    let text = TextRenderer::load(&FontConfig::default());
    let items = vec![MenuItem::entry("More").submenu(true)];
    let l = MenuLayout::compute(&theme.metrics, &items);
    let mut c = Canvas::new(l.width as u32, l.height as u32);
    draw_menu(&mut c, &theme, &text, &l, &items, None, "", false);
    let r = l.rows[0];
    let ax = r.right() - ARROW_RIGHT_INSET;
    let ay = r.y + (r.height - 7) / 2;
    assert_eq!(c.get(ax, ay), theme.colors.menu_text.argb());
    assert_eq!(c.get(ax, ay + 6), theme.colors.menu_text.argb());
    assert_eq!(c.get(ax + 3, ay + 3), theme.colors.menu_text.argb());
    assert_eq!(c.get(ax + 3, ay + 2), theme.colors.menu.argb());
}

fn flat_theme(v: crate::blue::Variant) -> Theme {
    let mut t = crate::interfaces::blue_theme(v);
    t.colors.menu_hilight = crate::theme::Color::rgb(0xBB, 0xB7, 0xC7);
    t.colors.hilight = crate::theme::Color::rgb(0xB2, 0xB4, 0xBF);
    t.colors.button_hilight = crate::theme::Color::rgb(0xF0, 0x10, 0x10);
    t
}

fn chrome_rows() -> Vec<MenuItem> {
    vec![
        MenuItem::entry(""),
        MenuItem::entry("").checked(true),
        MenuItem::entry("").checked(true).radio(true),
        MenuItem::Separator,
        MenuItem::entry("").enabled(false),
        MenuItem::entry("").submenu(true),
    ]
}

#[test]
fn a_flat_menu_has_a_one_px_border_a_one_line_separator_and_a_framed_lit_row() {
    let text = TextRenderer::load(&FontConfig::default());
    for v in crate::blue::Variant::ALL {
        let theme = flat_theme(v);
        let cs = &theme.colors;
        let items = vec![
            MenuItem::item("&Toolbars"),
            MenuItem::item("Status &Bar"),
            MenuItem::Separator,
            MenuItem::item("&Details"),
        ];
        let style = MenuStyle::context_for(&theme, &items).fit(&text, &items, 11, 400);
        let l = MenuLayout::compute_styled(&style, &items, usize::MAX);
        let mut c = Canvas::new(l.width as u32, l.height as u32);
        draw_menu(&mut c, &theme, &text, &l, &items, Some(0), "", false);
        let (w, h) = (l.width, l.height);
        for (x, y) in [
            (0, 0),
            (w - 1, 0),
            (0, h - 1),
            (w - 1, h - 1),
            (0, h / 2),
            (w / 2, 0),
        ] {
            assert_eq!(
                c.get(x, y),
                cs.button_shadow.argb(),
                "{v:?}: border at {x},{y}"
            );
        }
        for (x, y) in [(1, 1), (2, 2), (w - 2, h - 2), (w - 3, h / 2)] {
            assert_eq!(
                c.get(x, y),
                cs.menu.argb(),
                "{v:?}: menu inside the border at {x},{y}"
            );
        }
        let lit = l.rows[0];
        assert_eq!((lit.x, lit.y), (3, 3), "{v:?}: the rows start 3 px in");
        for (x, y) in [(lit.x, lit.y), (lit.right() - 1, lit.bottom() - 1)] {
            assert_eq!(
                c.get(x, y),
                cs.hilight.argb(),
                "{v:?}: the lit row's frame at {x},{y}"
            );
        }
        assert_eq!(
            c.get(lit.x + 1, lit.y + 1),
            cs.menu_hilight.argb(),
            "{v:?}: its fill"
        );
        let sep = l.rows[2];
        assert_eq!(sep.height, 9);
        let y = sep.y + 3;
        assert_eq!(
            c.get(sep.x, y),
            cs.menu.argb(),
            "{v:?}: the line starts a pixel in"
        );
        assert_eq!(
            c.get(sep.x + 1, y),
            cs.button_shadow.argb(),
            "{v:?}: the separator line"
        );
        assert_eq!(
            c.get(sep.right() - 2, y),
            cs.button_shadow.argb(),
            "{v:?}: its last pixel"
        );
        assert_eq!(
            c.get(sep.right() - 1, y),
            cs.menu.argb(),
            "{v:?}: a pixel short"
        );
        for dy in [-1, 1] {
            assert_eq!(
                c.get(sep.x + 5, y + dy),
                cs.menu.argb(),
                "{v:?}: one line, no etch"
            );
        }
    }
}

#[test]
fn a_flat_menus_greyed_label_has_no_etched_copy() {
    if !crate::text::test_tahoma() {
        return;
    }
    let theme = flat_theme(crate::blue::Variant::Blue);
    let text = TextRenderer::load(&FontConfig::default());
    let items = vec![
        MenuItem::item("&Delete")
            .enabled(false)
            .shortcut(Some("Del".into())),
    ];
    let style = MenuStyle::context_for(&theme, &items).fit(&text, &items, 11, 400);
    let l = MenuLayout::compute_styled(&style, &items, usize::MAX);
    let mut c = Canvas::new(l.width as u32, l.height as u32);
    draw_menu(&mut c, &theme, &text, &l, &items, None, "", false);
    let px = |col: crate::theme::Color| {
        (0..l.width)
            .flat_map(|x| (0..l.height).map(move |y| (x, y)))
            .filter(|&(x, y)| c.get(x, y) == col.argb())
            .count()
    };
    assert!(px(theme.colors.gray_text) > 20, "the label inks gray_text");
    assert_eq!(px(theme.colors.button_hilight), 0, "no etched copy");
}

#[test]
fn blues_start_menu_rows_sit_inside_a_1_px_border() {
    let theme = crate::interfaces::blue_theme(crate::blue::Variant::Blue);
    let text = TextRenderer::load(&FontConfig::default());
    let items = vec![
        MenuItem::entry("").submenu(true),
        MenuItem::Separator,
        MenuItem::entry(""),
    ];
    let style = MenuStyle {
        width: 194,
        ..MenuStyle::root(&theme)
    };
    let l = MenuLayout::compute_styled(&style, &items, usize::MAX);
    assert_eq!(l.rows[0], Rect::new(22, 2, 171, 32));
    assert_eq!(l.rows[1], Rect::new(22, 34, 171, 8));
    assert_eq!(l.height, 2 + 32 + 8 + 32 + 1);
    let mut c = Canvas::new(l.width as u32, l.height as u32);
    draw_menu(&mut c, &theme, &text, &l, &items, None, "", false);
    let cs = &theme.colors;
    assert_eq!(c.get(0, 5), cs.button_shadow.argb(), "the border");
    assert_eq!(c.get(1, 1), cs.menu_band_top.argb(), "the band's top-left");
    assert_eq!(
        c.get(1, l.height - 2),
        cs.menu_band.argb(),
        "the band's bottom-left"
    );
    assert_eq!(c.get(100, 1), cs.menu.argb(), "the row of menu on top");
    let line = |x| c.get(x, 37) == cs.button_shadow.argb();
    assert!(
        !line(23) && line(24) && line(190) && !line(191),
        "separator"
    );
    let ink = |x| (2..34).any(|y| c.get(x, y) == cs.menu_text.argb());
    assert!(!ink(183) && ink(184) && ink(187) && !ink(188), "the arrow");

    let sub = MenuStyle {
        width: 161,
        ..MenuStyle::submenu(&theme)
    };
    let sl = MenuLayout::compute_styled(&sub, &items[..1], usize::MAX);
    assert_eq!(sl.rows[0], Rect::new(1, 1, 159, 20));
    assert_eq!(sl.height, 22);
    let parent = Rect::new(0, 783, l.width, l.height);
    let row = Rect::new(22, 785, 171, 32);
    assert_eq!(
        place_submenu_inset(
            parent,
            row,
            (sl.width, sl.height),
            Rect::new(0, 0, 1920, 1050),
            l.inset.right,
            sl.inset,
        ),
        (192, 784)
    );
}

fn lit_arrow_ink(theme: &Theme, style: MenuStyle, s: i32) -> u32 {
    let text = TextRenderer::load(&FontConfig::default());
    let items = vec![MenuItem::entry("").submenu(true)];
    let theme = Theme {
        scale: s,
        ..theme.clone()
    };
    let style = MenuStyle {
        width: 160,
        ..style
    };
    let l = MenuLayout::compute_styled(&style, &items, usize::MAX).scaled(s);
    let mut c = Canvas::new(l.width as u32, l.height as u32);
    draw_menu(&mut c, &theme, &text, &l, &items, Some(0), "", false);
    let r = l.rows[0];
    c.get(r.right() - ARROW_RIGHT_INSET * s, centred(&r, 7, s))
}

#[test]
fn a_lit_start_menu_row_keeps_its_arrow_black_under_blue() {
    for v in crate::blue::Variant::ALL {
        let theme = crate::interfaces::blue_theme(v);
        let cs = &theme.colors;
        for s in [1, 2] {
            for style in [MenuStyle::root(&theme), MenuStyle::submenu(&theme)] {
                assert_eq!(
                    lit_arrow_ink(&theme, style, s),
                    cs.menu_text.argb(),
                    "{v:?} s={s}: a Start menu's lit arrow"
                );
            }
            let context = MenuStyle::context(&theme, ContextBitmaps::None);
            assert_eq!(
                lit_arrow_ink(&theme, context, s),
                cs.hilight_text.argb(),
                "{v:?} s={s}: a context menu's lit arrow"
            );
        }
    }
    let classic = wide_theme();
    for style in [MenuStyle::root(&classic), MenuStyle::submenu(&classic)] {
        assert_eq!(
            lit_arrow_ink(&classic, style, 1),
            classic.colors.hilight_text.argb(),
            "Classic's Start menu"
        );
    }
}

#[test]
fn a_flat_menu_doubles_at_scale_2() {
    let text = TextRenderer::load(&FontConfig::default());
    let items = chrome_rows();
    let draw = |c: &mut Canvas, p: &crate::look::Paint| {
        let theme = Theme::for_draw(p, 11 * p.s as u32);
        let style = MenuStyle::context(&Theme::default(), ContextBitmaps::None);
        let style = MenuStyle { width: 80, ..style };
        let l = MenuLayout::compute_styled(&style, &items, usize::MAX).scaled(p.s);
        draw_menu_at(
            c,
            (2 * p.s, 2 * p.s),
            &theme,
            &text,
            &l,
            &items,
            Some(0),
            "",
            false,
        );
    };
    assert!(crate::look::styled::fixture::blue_doubles_at_paint_scale_2(
        90, 110, draw
    ));
}
