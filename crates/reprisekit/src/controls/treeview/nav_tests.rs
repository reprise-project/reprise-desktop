use super::*;
use crate::interface::Interface;
use crate::interfaces::{theme_for, theme_with};
use crate::theme::FontConfig;

/// Favorites (open: Desktop, Downloads), Computer (open: Local Disk), Network.
fn nav() -> TreeView {
    let mut fav = Node::new("Favorites", true);
    fav.children = vec![Node::new("Desktop", false), Node::new("Downloads", false)];
    fav.expanded = true;
    let mut computer = Node::new("Computer", true);
    computer.children = vec![Node::new("Local Disk", true)];
    computer.expanded = true;
    TreeView::new(vec![fav, computer, Node::new("Network", false)], Style::Nav)
}

fn cm() -> ControlMetrics {
    theme_for(Interface::Glass).controls()
}

const PANE: Rect = Rect::new(0, 0, 200, 200);

fn labels(tv: &TreeView) -> Vec<String> {
    tv.visible()
        .iter()
        .map(|r| tv.node(&r.path).map_or("-".into(), |n| n.label.clone()))
        .collect()
}

#[test]
fn nav_rows_put_a_gap_between_roots_and_keys_step_over_it() {
    let mut tv = nav();
    assert_eq!(
        labels(&tv),
        [
            "Favorites",
            "Desktop",
            "Downloads",
            "-",
            "Computer",
            "Local Disk",
            "-",
            "Network"
        ]
    );
    let lay = layout_without_text(&tv, PANE, &cm());
    assert_eq!(lay.row_h, NAV_ROW_H);
    tv.selected = Some(vec![0, 1]);
    tv.step(1, &lay);
    assert_eq!(
        tv.selected,
        Some(vec![1]),
        "Down from Downloads lands on Computer"
    );
    tv.step(-1, &lay);
    assert_eq!(
        tv.selected,
        Some(vec![0, 1]),
        "Up from Computer lands on Downloads"
    );
    let gap_y = lay.body.y + 3 * lay.row_h + 5;
    assert_eq!(
        tv.hit(&lay, 60, gap_y),
        Hit::Nothing,
        "a gap row is never hit"
    );
    assert!(!tv.select(3, &lay), "nor picked");
    assert_eq!(tv.click_row(3, &lay), None);
    assert_eq!(tv.selected, Some(vec![0, 1]));
}

#[test]
fn a_nav_click_selects_without_opening_or_closing_branches() {
    let mut tv = nav();
    let lay = layout_without_text(&tv, PANE, &cm());
    tv.selected = Some(vec![1, 0]);
    assert_eq!(tv.click_row(1, &lay), None, "Desktop");
    assert_eq!(tv.selected, Some(vec![0, 0]));
    assert!(tv.roots[1].expanded, "Computer stays open");
    let network = tv.visible().len() - 1;
    tv.click_row(network, &lay);
    assert!(!tv.roots[2].expanded);
    tv.roots[1].children[0].expanded = false;
    tv.click_row(5, &lay);
    assert!(!tv.roots[1].children[0].expanded, "a click opens nothing");
}

/// Every root has its expander left of its icon; a level indents 10 px.
#[test]
fn nav_roots_and_children_hit_their_expanders() {
    let tv = nav();
    let lay = layout_without_text(&tv, PANE, &cm());
    let mid = |i: i32| lay.body.y + i * lay.row_h + lay.row_h / 2;
    let root_cx = NAV_ROOT_ICON_X - NAV_EXPANDER_BACK;
    assert_eq!(tv.hit(&lay, root_cx, mid(0)), Hit::Expander(0));
    assert_eq!(tv.hit(&lay, root_cx + NAV_INDENT, mid(5)), Hit::Expander(5));
    assert_eq!(tv.hit(&lay, NAV_ROOT_ICON_X + 2, mid(0)), Hit::Row(0));
    assert_eq!(
        tv.hit(&lay, root_cx + NAV_INDENT, mid(1)),
        Hit::Nothing,
        "Desktop holds nothing to open"
    );
}

#[test]
fn expanders_show_only_while_the_pointer_is_over_the_pane() {
    let mut tv = nav();
    let t0 = Instant::now();
    let ms = |n: u32| t0 + Duration::from_millis(u64::from(n));
    assert_eq!(tv.expanders.level(), 0.0, "hidden at first");
    let next = tv.show_expanders(true, t0);
    assert_eq!(next, Some(t0 + FADE_FRAME), "fading in: redraw a frame on");
    assert!(tv.tick_expanders(ms(EXPANDER_FADE_IN_MS / 2)).is_some());
    let half = tv.expanders.level();
    assert!(half > 0.0 && half < 1.0, "half way in: {half}");
    assert_eq!(tv.tick_expanders(ms(EXPANDER_FADE_IN_MS)), None, "settled");
    assert_eq!(tv.expanders.level(), 1.0);
    let out = ms(EXPANDER_FADE_IN_MS + 1000);
    assert_eq!(
        tv.show_expanders(true, out),
        None,
        "still shown: nothing to redraw"
    );
    assert!(tv.show_expanders(false, out).is_some(), "fading out");
    let gone = out + Duration::from_millis(u64::from(EXPANDER_FADE_OUT_MS));
    assert_eq!(tv.tick_expanders(gone), None);
    assert_eq!(tv.expanders.level(), 0.0, "hidden once the pointer left");
}

/// The rows' access items skip the gaps and nest under their groups.
#[test]
fn access_lists_the_groups_and_their_rows_without_the_gaps() {
    let tv = nav();
    let text = TextRenderer::load(&FontConfig::default());
    let lay = layout(&tv, &text, 12, PANE, &cm());
    let mut t = access::Tree::window("Explorer", (400, 400));
    access(&tv, &lay, &text, 12, &mut t, access::ROOT, "nav");
    let items: Vec<String> = (t.nodes())
        .filter(|(_, n)| n.role() == Role::TreeItem)
        .filter_map(|(_, n)| n.label().map(str::to_string))
        .collect();
    assert_eq!(items.len(), 6, "{items:?}");
    let computer = t.find(Role::TreeItem, "Computer").expect("Computer");
    let disk = t.find(Role::TreeItem, "Local Disk").expect("Local Disk");
    let kids = t.get(computer).map(|n| n.children().to_vec());
    assert_eq!(kids, Some(vec![disk]));
}

/// The pane in Glass light or dark at scale `s`: the rows laid out at 100%, drawn at the
/// theme's scale, `set` applied first.
fn render(mode: crate::glass::Mode, s: i32, set: impl Fn(&mut TreeView)) -> Canvas {
    let g = crate::glass::Settings {
        mode,
        ..Default::default()
    };
    let theme = theme_with(Interface::Glass, Default::default(), false, &g);
    let mut tv = nav();
    set(&mut tv);
    let one = TextRenderer::load(&theme.fonts);
    let lay = layout(&tv, &one, theme.fonts.size_px, PANE, &theme.controls());
    let theme = theme.scaled(s);
    let text = TextRenderer::load(&theme.fonts);
    let mut c = Canvas::new((PANE.width * s) as u32, (PANE.height * s) as u32);
    let lit = Default::default();
    draw(
        &mut c,
        &text,
        &theme.paint(),
        &tv,
        &lay,
        lit,
        theme.fonts.size_px,
    );
    c
}

type Scene = (&'static str, fn(&mut TreeView));

fn scenes() -> [Scene; 5] {
    [
        ("rest", |_| {}),
        ("hot", |tv| {
            tv.over = Over {
                inside: true,
                row: Some(4),
                expander: false,
            }
        }),
        ("selected", |tv| tv.selected = Some(vec![1])),
        ("selected, focus elsewhere", |tv| {
            tv.selected = Some(vec![1]);
            tv.inactive = true;
        }),
        ("expanders shown", |tv| {
            tv.show_expanders(true, Instant::now() - Duration::from_secs(1));
            tv.tick_expanders(Instant::now());
        }),
    ]
}

#[test]
fn nav_pane_pins_per_state_at_scale_1_and_2() {
    crate::pins::fonts();
    let mut got = Vec::new();
    for mode in crate::glass::Mode::ALL {
        for (state, set) in scenes() {
            for s in [1, 2] {
                let h = crate::pins::fnv(&render(mode, s, set));
                got.push((format!("{mode:?} {state} {s}"), h));
            }
        }
    }
    crate::pins::check("controls/treeview_nav", &got);
}

#[test]
fn every_nav_state_draws_differently() {
    crate::pins::fonts();
    for mode in crate::glass::Mode::ALL {
        let drawn: Vec<(&str, Canvas)> = (scenes().into_iter())
            .map(|(n, set)| (n, render(mode, 1, set)))
            .collect();
        for (i, (a, ca)) in drawn.iter().enumerate() {
            for (b, cb) in &drawn[i + 1..] {
                assert!(ca != cb, "{mode:?}: {a} and {b} draw alike");
            }
        }
    }
}

/// Half faded in, an expander's triangle is lighter than fully shown and darker than none.
#[test]
fn a_half_faded_expander_draws_between_hidden_and_shown() {
    let at = |level: f32| {
        render(crate::glass::Mode::Light, 1, move |tv| {
            tv.expanders.level = level;
        })
    };
    let (none, half, full) = (at(0.0), at(0.5), at(1.0));
    let cx = NAV_ROOT_ICON_X - NAV_EXPANDER_BACK;
    let ink = |c: &Canvas| {
        (0..NAV_ROW_H)
            .map(|y| (c.get(cx + 2, y) & 0xFF) as i32)
            .min()
            .unwrap_or(255)
    };
    assert!(ink(&none) > ink(&half) && ink(&half) > ink(&full));
}
