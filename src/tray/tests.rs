use super::customize::{past_icon, refill_past, remember};
use super::icons::icon_sizes;
use super::icons::pixmap_to_source;
use super::menu::{SubmenuRequest, menu_items, strip_mnemonic};
use super::place::{Change, Place, arrange, merge_item, shows_chevron};
use super::pointer::click_command;
use crate::icontheme::IconCache;
use crate::input::{BTN_LEFT, BTN_MIDDLE, BTN_RIGHT};
use crate::raster::Canvas;
use reprisekit::controls::shellview;
use reprisekit::controls::traycustomize::{self, Behaviour, Entry, TrayCustomize};
use reprisekit::icon::IconSource;
use std::collections::HashMap;

#[test]
fn a_remembered_icon_file_is_never_read_on_the_loop() {
    let file = "/srv/icons/tray.png";
    let mut icons = IconCache::new(crate::icontheme::IconTheme::load("none", &[]));
    assert!(past_icon(&mut icons, Some(file), 16).is_empty());
    assert_eq!(
        icons.take_wanted(),
        [(file.to_string(), 16), (file.to_string(), 32)]
    );
    icons.put_file(file, 16, Some(Canvas::new(16, 16)));
    icons.put_file(file, 32, None);
    let widths: Vec<u32> = past_icon(&mut icons, Some(file), 16)
        .iter()
        .map(|s| s.width)
        .collect();
    assert_eq!(widths, [16]);
}

#[test]
fn icon_sizes_are_the_cell_and_list_px_at_each_scale() {
    assert_eq!(icon_sizes(&[1], 16), [16]);
    assert_eq!(icon_sizes(&[1, 2], 24), [16, 24, 32, 48]);
    assert_eq!(icon_sizes(&[2, 2], 16), [32]);
}

#[test]
fn a_text_size_or_scale_change_sends_the_new_icon_sizes() {
    use smithay::output::Scale;
    let mut h = crate::harness::Harness::new();
    let output = h.output("TEST-1", 1024, 768);
    let (_tx, _wake, heard) = headless(&mut h.state);
    let sizes = || {
        std::iter::from_fn(|| heard.try_recv().ok())
            .filter_map(|c| match c {
                TrayCommand::IconSizes(s) => Some(s),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    crate::configwatch::redraw_for_scale(&mut h.state);
    assert!(sizes().is_empty(), "nothing sent while the sizes stand");

    let large = crate::config::Config::parse("[accessibility]\ntext_size = 150\n").unwrap();
    crate::configwatch::reload(&mut h.state, large);
    let list = shellview::list_icon_px(h.state.config.theme.controls().text);
    assert!(list > 16, "the list icon grows at 150%, got {list}");
    assert_eq!(sizes(), [vec![16, list]]);

    output.change_current_state(None, None, Some(Scale::Fractional(2.0)), None);
    h.state.update_layout();
    assert_eq!(sizes(), [vec![32, 2 * list]]);
}

#[test]
fn a_remembered_icon_is_fetched_at_the_lists_grown_icon_size() {
    let mut icons = IconCache::new(crate::icontheme::IconTheme::load("none", &[]));
    past_icon(&mut icons, Some("/srv/icons/tray.png"), 24);
    let sizes: Vec<i32> = icons.take_wanted().into_iter().map(|(_, px)| px).collect();
    assert_eq!(sizes, [16, 24, 32, 48]);
}

#[test]
fn a_remembered_row_takes_its_icon_file_when_it_lands() {
    let file = "/srv/icons/tray.png";
    let mut icons = IconCache::new(crate::icontheme::IconTheme::load("none", &[]));
    let entry = |key: &str, icon: Vec<IconSource>| Entry {
        key: key.into(),
        name: key.into(),
        detail: String::new(),
        icon: icon.into(),
        passive: false,
        behaviour: Behaviour::Hide,
    };
    let past = past_icon(&mut icons, Some(file), 16);
    assert!(past.is_empty());
    let mut t = TrayCustomize::new(
        traycustomize::Wording::Classic,
        vec![entry("live", Vec::new())],
        vec![entry("gone", past), entry("other", Vec::new())],
        None,
        reprisekit::metrics::ControlMetrics::CLASSIC,
    );
    let kept = |icon: &str| Remembered {
        behaviour: Behaviour::Hide,
        name: String::new(),
        icon: Some(icon.into()),
    };
    let choices = [
        ("live".to_string(), kept(file)),
        ("gone".to_string(), kept(file)),
        ("other".to_string(), kept("/srv/icons/other.png")),
    ]
    .into_iter()
    .collect();
    icons.put_file(file, 16, Some(Canvas::new(16, 16)));
    assert!(!refill_past(
        &mut t,
        &choices,
        &mut icons,
        "/srv/icons/else.png"
    ));
    assert!(refill_past(&mut t, &choices, &mut icons, file));
    let widths: Vec<Vec<u32>> = (t.entries.iter())
        .map(|e| e.icon.iter().map(|s| s.width).collect())
        .collect();
    assert_eq!(widths, [vec![], vec![16], vec![]]);
}

#[test]
fn the_balloon_icon_is_the_picture_else_the_app_icon() {
    let src = |w: u32| IconSource {
        width: w,
        height: w,
        pixels: Vec::new(),
    };
    let mut n = Notification::new(1, "s".into(), "b".into());
    n.app_icon = vec![src(16)];
    assert_eq!(n.balloon_icon(), &n.app_icon[..]);
    n.image = vec![src(32)];
    assert_eq!(n.balloon_icon(), &n.image[..]);
}

use super::*;
use crate::startmenu::MenuItem;

#[test]
fn a_tray_cell_is_placed_from_the_bars_own_strip() {
    let right = crate::barplace::place(
        Rect::new(0, 0, 1920, 1080),
        reprisekit::taskbar::Edge::Right,
        60,
    );
    assert_eq!(
        bar_cell(right.shown, Rect::new(4, 1000, 16, 16)),
        Rect::new(1864, 1000, 16, 16),
        "a right bar's cell is right of the output's left edge"
    );
    let bottom = crate::barplace::place(
        Rect::new(1920, 0, 1280, 720),
        reprisekit::taskbar::Edge::Bottom,
        30,
    );
    assert_eq!(
        bar_cell(bottom.shown, Rect::new(1200, 7, 16, 16)),
        Rect::new(3120, 697, 16, 16)
    );
}

#[test]
fn a_click_reaches_the_item_at_the_cells_centre_on_the_bars_screen_side_edge() {
    use reprisekit::taskbar::Edge;
    let screen = Rect::new(0, 0, 1920, 1080);
    let bottom = crate::barplace::place(screen, Edge::Bottom, 30);
    let cell = Rect::new(1841, 1057, 16, 16);
    assert_eq!(
        bottom.click_point(cell),
        (1849, 1050),
        "a bottom bar's top edge"
    );
    let top = crate::barplace::place(screen, Edge::Top, 30);
    let cell = Rect::new(1841, 7, 16, 16);
    assert_eq!(top.click_point(cell), (1849, 30), "a top bar's foot");
    let left = crate::barplace::place(screen, Edge::Left, 60);
    let cell = Rect::new(22, 1000, 16, 16);
    assert_eq!(
        left.click_point(cell),
        (60, 1008),
        "a left bar's right edge"
    );
}

#[test]
fn an_item_hears_a_click_at_the_point_it_is_given() {
    for (button, has_menu, want) in [
        (BTN_LEFT, true, ClickKind::Activate),
        (BTN_MIDDLE, true, ClickKind::SecondaryActivate),
        (BTN_RIGHT, false, ClickKind::ContextMenu),
    ] {
        match click_command("a".into(), (1849, 1050), button, has_menu, || "t".into()) {
            Some(TrayCommand::Click { id, x, y, kind, .. }) => assert_eq!(
                (id.as_str(), x, y, kind),
                ("a", 1849, 1050, want),
                "{want:?} is heard at (1849, 1050)"
            ),
            other => panic!("{want:?}: a click at (1849, 1050), not {other:?}"),
        }
    }
    assert!(
        matches!(
            click_command("a".into(), (1849, 1050), BTN_RIGHT, true, || "t".into()),
            Some(TrayCommand::GetLayout { parent: 0, .. })
        ),
        "a right click on an item with a menu asks for the menu's layout"
    );
    assert!(
        click_command("a".into(), (1849, 1050), 0x113, false, || "t".into()).is_none(),
        "a side button asks nothing"
    );
}

#[test]
fn an_activation_carries_a_token_and_a_context_menu_none() {
    for (button, want) in [
        (BTN_LEFT, Some("t")),
        (BTN_MIDDLE, Some("t")),
        (BTN_RIGHT, None),
    ] {
        match click_command("a".into(), (0, 0), button, false, || "t".into()) {
            Some(TrayCommand::Click { token, .. }) => {
                assert_eq!(
                    token.as_deref(),
                    want,
                    "button {button:#x} carries {want:?}"
                )
            }
            other => panic!("button {button:#x}: a click, not {other:?}"),
        }
    }
}

#[test]
fn clip_cuts_on_a_char_boundary() {
    assert_eq!(clip("héllo".into(), 2), "hé");
    assert_eq!(clip("hi".into(), 5), "hi");
    assert_eq!(
        clip("x".repeat(1000), BODY_CHARS).chars().count(),
        BODY_CHARS
    );
}

fn request(id: &str, node: i32, depth: usize, row: usize) -> SubmenuRequest {
    SubmenuRequest {
        id: id.into(),
        node,
        depth,
        row,
    }
}

#[test]
fn submenu_reply_is_accepted_only_for_the_node_still_wanted() {
    let mut s = Submenus::default();
    s.request(request("a", 5, 0, 2));
    // A reply for the row the pointer has already left: dropped, and the
    // request in flight survives it.
    assert_eq!(s.accept("a", 9), None);
    assert_eq!(s.accept("b", 5), None);
    assert_eq!(s.accept("a", 5), Some((0, 2)));
    assert_eq!(s.accept("a", 5), None);
}

#[test]
fn hovering_does_not_re_request_an_open_or_in_flight_submenu() {
    let mut s = Submenus::default();
    assert!(s.needs_request("a", 0, 2));
    s.request(request("a", 5, 0, 2));
    assert!(!s.needs_request("a", 0, 2));
    assert!(s.needs_request("a", 0, 3));
    s.accept("a", 5);
    s.opened(0, 2);
    assert!(!s.needs_request("a", 0, 2));
    assert!(s.needs_request("a", 1, 0));
}

#[test]
fn closing_menus_drops_the_requests_they_belong_to() {
    let mut s = Submenus::default();
    s.request(request("a", 5, 0, 2));
    s.accept("a", 5);
    s.opened(0, 2);
    s.request(request("a", 7, 1, 0));
    s.closed_above(0);
    assert_eq!(s.pending, None);
    assert!(s.needs_request("a", 0, 2));

    s.request(request("a", 5, 0, 2));
    s.clear();
    assert_eq!(s, Submenus::default());

    s.request(request("a", 5, 0, 2));
    s.forget_item("b");
    assert!(s.pending.is_some());
    s.forget_item("a");
    assert_eq!(s.pending, None);
}

#[test]
fn strip_mnemonic_drops_single_underscores_and_keeps_doubles() {
    assert_eq!(strip_mnemonic("_File"), "File");
    assert_eq!(strip_mnemonic("Save _As..."), "Save As...");
    assert_eq!(strip_mnemonic("100__percent"), "100_percent");
    assert_eq!(strip_mnemonic("plain"), "plain");
    assert_eq!(strip_mnemonic(""), "");
}

fn merge(bar: &mut Vec<TrayItem>, hidden: &mut Vec<TrayItem>, it: TrayItem) -> bool {
    merge_left(bar, hidden, &mut HashMap::new(), it) != Change::None
}

fn merge_left(
    bar: &mut Vec<TrayItem>,
    hidden: &mut Vec<TrayItem>,
    left: &mut HashMap<String, usize>,
    it: TrayItem,
) -> Change {
    let place = placement(&it, Behaviour::Auto, true);
    merge_item(bar, hidden, left, it, place)
}

fn item(id: &str, passive: bool) -> TrayItem {
    TrayItem {
        id: id.into(),
        key: id.into(),
        service: ":1.1".into(),
        path: "/StatusNotifierItem".into(),
        title: String::new(),
        tooltip: String::new(),
        icon_name: None,
        icon_theme_path: None,
        pixmaps: Arc::from([]),
        named: Arc::from([]),
        menu_path: None,
        passive,
        pid: None,
        ..Default::default()
    }
}

#[test]
fn cells_past_the_shown_items_are_builtin() {
    assert_eq!(Cell::at(0, 3, 0, 2), Cell::Item(2));
    assert_eq!(Cell::at(0, 3, 0, 3), Cell::Builtin(0));
    assert_eq!(
        Cell::at(0, MAX_ITEMS + 4, 0, MAX_ITEMS - 1),
        Cell::Item(MAX_ITEMS - 1)
    );
    assert_eq!(Cell::at(0, MAX_ITEMS + 4, 0, MAX_ITEMS), Cell::Builtin(0));
    assert_eq!(
        Cell::at(0, MAX_ITEMS + 4, 0, MAX_ITEMS + 1),
        Cell::Builtin(1)
    );
}

#[test]
fn revealed_cells_lead_and_map_to_the_hidden_run_nearest_the_icons() {
    assert_eq!(Cell::at(2, 3, 1, 0), Cell::Hidden(0));
    assert_eq!(Cell::at(2, 3, 1, 1), Cell::Hidden(1));
    assert_eq!(Cell::at(2, 3, 1, 2), Cell::Builtin(0));
    assert_eq!(Cell::at(2, 3, 1, 3), Cell::Item(0));
    assert_eq!(Cell::at(0, 3, 1, 0), Cell::Builtin(0), "shut: as before");
}

#[test]
fn a_revealed_cell_maps_to_the_hidden_items_nearest_the_icons() {
    assert_eq!(hidden_index(4, 2, 0), Some(2));
    assert_eq!(hidden_index(4, 2, 1), Some(3));
    assert_eq!(hidden_index(1, 2, 0), None, "the run shrank under the draw");
    assert_eq!(
        hidden_index(1, 2, 1),
        Some(0),
        "the cell nearest the shown icons keeps the last item"
    );
}

#[test]
fn a_cut_run_draws_the_items_its_cells_click() {
    let hidden = ["a", "b", "c", "d", "e"];
    let mut icons: Vec<&str> = hidden[2..].to_vec();
    icons.push("shown");
    keep_run(&mut icons, 3, 2);
    assert_eq!(icons, ["d", "e", "shown"]);
    for (i, icon) in icons.iter().take(2).enumerate() {
        assert_eq!(Some(icon), hidden_index(5, 2, i).map(|k| &hidden[k]));
    }
}

#[test]
fn a_whole_run_keeps_every_picture() {
    let mut icons = vec![1, 2, 3];
    keep_run(&mut icons, 2, 2);
    assert_eq!(icons, [1, 2, 3]);
}

#[test]
fn leading_cells_come_before_the_items() {
    assert_eq!(Cell::at(0, 2, 1, 0), Cell::Builtin(0));
    assert_eq!(Cell::at(0, 2, 1, 1), Cell::Item(0));
    assert_eq!(Cell::at(0, 2, 1, 2), Cell::Item(1));
    assert_eq!(Cell::at(0, 2, 1, 3), Cell::Builtin(1));
    assert_eq!(Cell::at(0, 0, 1, 1), Cell::Builtin(1));
}

#[test]
fn passive_items_are_hidden_and_the_rest_stay_on_the_bar() {
    assert_eq!(
        placement(&item("a", true), Behaviour::Auto, true),
        Place::Hidden
    );
    assert_eq!(
        placement(&item("a", false), Behaviour::Auto, true),
        Place::Bar
    );
}

#[test]
fn a_choice_overrides_the_status_only_with_overflow_on() {
    let (busy, idle) = (item("a", false), item("b", true));
    assert_eq!(placement(&busy, Behaviour::Hide, true), Place::Hidden);
    assert_eq!(placement(&busy, Behaviour::Mute, true), Place::Hidden);
    assert_eq!(placement(&idle, Behaviour::Show, true), Place::Bar);
    assert_eq!(placement(&busy, Behaviour::Hide, false), Place::Bar);
    assert_eq!(placement(&idle, Behaviour::Auto, false), Place::Bar);
}

#[test]
fn an_item_needing_attention_leaves_the_overflow_unless_muted() {
    let mut calling = item("a", false);
    calling.needs_attention = true;
    assert_eq!(placement(&calling, Behaviour::Hide, true), Place::Bar);
    assert_eq!(placement(&calling, Behaviour::Auto, true), Place::Bar);
    assert_eq!(placement(&calling, Behaviour::Mute, true), Place::Hidden);
    let (mut bar, mut hidden) = (Vec::new(), vec![item("a", false)]);
    assert!(
        merge(&mut bar, &mut hidden, calling),
        "the status change redraws"
    );
    assert_eq!((bar.len(), hidden.len()), (1, 0));
}

#[test]
fn new_choices_move_items_and_keep_the_order_in_each_list() {
    fn ids(l: &[TrayItem]) -> Vec<&str> {
        l.iter().map(|i| i.id.as_str()).collect()
    }
    let mut bar = vec![item("a", false), item("b", false), item("c", false)];
    let mut hidden = vec![item("p", true)];
    let mut choices = BTreeMap::new();
    let keep = |b| Remembered {
        behaviour: b,
        name: String::new(),
        icon: None,
    };
    choices.insert("b".to_string(), keep(Behaviour::Hide));
    choices.insert("p".to_string(), keep(Behaviour::Show));
    arrange(&mut bar, &mut hidden, &choices, true);
    assert_eq!((ids(&bar), ids(&hidden)), (vec!["a", "c", "p"], vec!["b"]));
    arrange(&mut bar, &mut hidden, &choices, false);
    assert_eq!(
        (ids(&bar), ids(&hidden)),
        (vec!["a", "c", "p", "b"], vec![])
    );
}

#[test]
fn ok_keeps_only_choices_away_from_auto_and_names_them() {
    let mut choices = BTreeMap::new();
    choices.insert(
        "gone".to_string(),
        Remembered {
            behaviour: Behaviour::Hide,
            name: "Old Applet".into(),
            icon: Some("old".into()),
        },
    );
    let mut net = item(":1.4/StatusNotifierItem", false);
    net.key = "nm-applet".into();
    net.tooltip = "Network\nConnected".into();
    net.icon_name = Some("network-wired".into());
    let picked = vec![
        ("nm-applet".to_string(), Behaviour::Show),
        ("gone".to_string(), Behaviour::Mute),
        ("other".to_string(), Behaviour::Auto),
    ];
    remember(&mut choices, &picked, &[&net]);
    assert_eq!(
        choices["nm-applet"],
        Remembered {
            behaviour: Behaviour::Show,
            name: "Network".into(),
            icon: Some("network-wired".into()),
        }
    );
    assert_eq!(choices["gone"].behaviour, Behaviour::Mute);
    assert_eq!(
        choices["gone"].name, "Old Applet",
        "a past item keeps its name"
    );
    remember(&mut choices, &[("gone".into(), Behaviour::Auto)], &[]);
    assert!(!choices.contains_key("gone"), "back to Auto: forgotten");
    assert!(!choices.contains_key("other"));
}

#[test]
fn the_chevron_shows_only_with_overflow_on_and_an_item_hidden() {
    let parse = |text: &str| crate::config::Config::parse(text).unwrap();
    let off = parse("[taskbar]\ntray_overflow = false\n");
    let glass = parse("[appearance]\ninterface = \"glass\"\n");
    assert!(!shows_chevron(&off, 2), "overflow off");
    assert!(!shows_chevron(&glass, 0), "nothing hidden");
    assert!(shows_chevron(&glass, 1));
}

#[test]
fn tray_overflow_follows_its_taskbar_key_in_every_look() {
    let parse = |text: &str| crate::config::Config::parse(text).unwrap();
    let glass_off =
        parse("[appearance]\ninterface = \"glass\"\n[taskbar]\ntray_overflow = false\n");
    assert!(
        !shows_chevron(&glass_off, 2),
        "off under Glass: no ^, the tray as it is without overflow"
    );
    let classic_on = parse("[taskbar]\ntray_overflow = true\n");
    assert!(
        shows_chevron(&classic_on, 1),
        "on under Classic: the ^ shows"
    );
}

#[test]
fn an_item_changing_status_moves_between_the_bar_and_the_hidden_list() {
    let ids = |l: &[TrayItem]| l.iter().map(|i| i.id.clone()).collect::<Vec<_>>();
    let (mut bar, mut hidden) = (Vec::new(), Vec::new());
    merge(&mut bar, &mut hidden, item("a", true));
    merge(&mut bar, &mut hidden, item("b", false));
    merge(&mut bar, &mut hidden, item("c", false));
    assert_eq!(
        (ids(&bar), ids(&hidden)),
        (vec!["b".into(), "c".into()], vec!["a".into()])
    );
    merge(&mut bar, &mut hidden, item("b", true));
    assert_eq!(
        (ids(&bar), ids(&hidden)),
        (vec!["c".into()], vec!["a".into(), "b".into()])
    );
    merge(&mut bar, &mut hidden, item("a", false));
    assert_eq!(
        (ids(&bar), ids(&hidden)),
        (vec!["c".into(), "a".into()], vec!["b".into()])
    );
    let mut c = item("c", false);
    c.title = "new".into();
    merge(&mut bar, &mut hidden, c);
    assert_eq!(ids(&bar), vec!["c".to_string(), "a".into()]);
    assert_eq!(bar[0].title, "new");
}

#[test]
fn a_status_flip_keeps_the_hidden_order_and_the_icons() {
    let ids = |l: &[TrayItem]| l.iter().map(|i| i.id.clone()).collect::<Vec<_>>();
    let (mut bar, mut left) = (Vec::new(), HashMap::new());
    let mut hidden = vec![item("a", true), item("b", true), item("c", true)];
    for _ in 0..2 {
        let mut calling = item("a", true);
        calling.needs_attention = true;
        let flip = merge_left(&mut bar, &mut hidden, &mut left, calling);
        assert_eq!(flip, Change::Status, "the icons drawn stay cached");
        assert_eq!(ids(&bar), ["a"]);
        let back = merge_left(&mut bar, &mut hidden, &mut left, item("a", true));
        assert_eq!(back, Change::Status);
        assert_eq!(ids(&hidden), ["a", "b", "c"], "a goes back first");
    }
}

#[test]
fn only_a_change_to_what_the_cell_draws_redraws_the_tray() {
    let (mut bar, mut hidden) = (Vec::new(), Vec::new());
    assert!(merge(&mut bar, &mut hidden, item("a", false)), "a new item");
    let mut a = item("a", false);
    a.tooltip = "Battery 80%".into();
    a.title = "Power".into();
    a.menu_path = Some("/Menu".into());
    a.pid = Some(42);
    assert!(
        !merge(&mut bar, &mut hidden, a.clone()),
        "tooltip, title, menu and pid are not drawn"
    );
    assert_eq!(bar[0].tooltip, "Battery 80%", "the model still takes them");
    let red = IconSource {
        width: 1,
        height: 1,
        pixels: vec![0xFFFF_0000],
    };
    a.pixmaps = Arc::from([red.clone()]);
    assert!(merge(&mut bar, &mut hidden, a.clone()), "a new pixmap");
    a.pixmaps = Arc::from([red]);
    assert!(
        !merge(&mut bar, &mut hidden, a.clone()),
        "the same pixels in a new Arc"
    );
    a.icon_name = Some("battery-low".into());
    assert!(merge(&mut bar, &mut hidden, a.clone()), "an icon name");
    a.icon_theme_path = Some("/opt/icons".into());
    assert!(merge(&mut bar, &mut hidden, a.clone()), "a theme path");
    a.named = Arc::from([IconSource {
        width: 1,
        height: 1,
        pixels: vec![0xFF00_FF00],
    }]);
    assert!(merge(&mut bar, &mut hidden, a.clone()), "a decoded name");
    a.passive = true;
    assert!(merge(&mut bar, &mut hidden, a), "Passive: off the bar");
}

#[test]
fn pixmap_to_source_reads_big_endian_argb_and_premultiplies() {
    let bytes = [
        0xFF, 0xFF, 0x00, 0x00, // A R G B
        0x80, 0xFF, 0xFF, 0xFF,
    ];
    let src = pixmap_to_source(2, 1, &bytes).unwrap();
    assert_eq!((src.width, src.height), (2, 1));
    assert_eq!(src.pixels[0], 0xFFFF_0000);
    assert_eq!(src.pixels[1], reprisekit::icon::premultiply(0x80FF_FFFF));
}

#[test]
fn pixmap_to_source_rejects_short_or_empty_buffers() {
    assert!(pixmap_to_source(2, 1, &[0; 4]).is_none());
    assert!(pixmap_to_source(0, 0, &[]).is_none());
    assert!(pixmap_to_source(-1, 1, &[0; 4]).is_none());
}

#[test]
fn pixmap_to_source_rejects_oversized_claims_regardless_of_buffer_len() {
    // A hostile peer claiming 60000x60000 must be rejected before the
    // `w * h * 4` byte length is even computed, whether the buffer it
    // sent is short (a few bytes) or actually that large.
    assert!(pixmap_to_source(60_000, 60_000, &[0; 16]).is_none());
    let huge = vec![0u8; 60_000 * 4];
    assert!(pixmap_to_source(60_000, 1, &huge).is_none());
}

#[test]
fn item_pixmaps_keeps_an_entry_per_drawn_size_of_at_most_256_px() {
    let pixels = vec![0u8; 1024 * 1024 * 4];
    let entry = |side: i32| (side, side, &pixels[..(side * side * 4) as usize]);
    let mut raw: Vec<_> = (0..30).map(|_| entry(1024)).collect();
    raw.extend([16, 20, 22, 32, 48, 64, 256, 300].map(entry));
    let kept = item_pixmaps(&raw, &[16, 25, 32, 50]);
    let widths: Vec<u32> = kept.iter().map(|p| p.width).collect();
    assert_eq!(
        widths,
        [16, 32, 64],
        "the smallest entry at least each drawn size"
    );
    let kept = item_pixmaps(&raw, &[16, 300]);
    let widths: Vec<u32> = kept.iter().map(|p| p.width).collect();
    assert_eq!(widths, [16, 256], "the widest kept entry past them all");

    let big: Vec<_> = (0..30).map(|_| entry(256)).collect();
    assert_eq!(
        item_pixmaps(&big, &[16, 32]).len(),
        1,
        "one 256 px entry out of 30"
    );
    assert!(
        item_pixmaps(&[entry(1024)], &[16]).is_empty(),
        "1024 px is dropped"
    );
}

fn node(id: i32, label: &str) -> MenuNode {
    MenuNode {
        id,
        label: label.into(),
        separator: false,
        enabled: true,
        visible: true,
        checked: None,
        radio: false,
        has_children: false,
        default: false,
    }
}

#[test]
fn menu_items_maps_nodes_and_skips_invisible() {
    let nodes = vec![
        node(1, "_Connect"),
        MenuNode {
            separator: true,
            ..node(2, "")
        },
        MenuNode {
            visible: false,
            ..node(3, "hidden")
        },
        MenuNode {
            enabled: false,
            checked: Some(true),
            has_children: true,
            ..node(4, "More")
        },
        MenuNode {
            default: true,
            ..node(5, "Open")
        },
        MenuNode {
            checked: Some(true),
            radio: true,
            ..node(6, "Balanced")
        },
    ];
    let (items, ids) = menu_items(&nodes);
    assert_eq!(items.len(), 5);
    assert_eq!(ids, vec![Some(1), None, Some(4), Some(5), Some(6)]);
    assert_eq!(items[0], MenuItem::entry("Connect"));
    assert_eq!(items[1], MenuItem::Separator);
    assert_eq!(
        items[2],
        MenuItem::entry("More")
            .enabled(false)
            .checked(true)
            .submenu(true)
    );
    assert_eq!(
        items[3],
        MenuItem::entry("Open").bold(true),
        "the default row is bold"
    );
    assert_eq!(
        items[4],
        MenuItem::entry("Balanced").checked(true).radio(true),
        "a radio row is bulleted"
    );
}
