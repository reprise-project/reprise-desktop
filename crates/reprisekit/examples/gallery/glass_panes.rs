//! Glass's shell window panes in light and dark: the navigation pane at rest, with a row under
//! the pointer and its expanders shown, with Computer selected and with the focus elsewhere; the
//! details pane for a folder, a file and several items; a Tiles view's group heading.

use reprisekit::controls::detailspane::{self, DetailsPane, Facts};
use reprisekit::controls::explorerbar::{self, ExplorerBar};
use reprisekit::controls::shellview::{self, Group, Mode};
use reprisekit::controls::treeview::{self, Node, Over, Style, TreeView};
use reprisekit::glass;
use reprisekit::interface::Interface;
use reprisekit::interfaces;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::TextRenderer;
use reprisekit::theme::{Color, Theme};
use std::time::{Duration, Instant};

const NAV: (i32, i32) = (204, 310);
const DETAILS: (i32, i32) = (660, detailspane::HEIGHT);
const HEADING: (i32, i32) = (660, 140);
const GAP: i32 = 12;

fn theme(mode: glass::Mode, style: glass::Style) -> Theme {
    let g = glass::Settings {
        mode,
        style,
        ..Default::default()
    };
    interfaces::theme_with(Interface::Glass, Default::default(), false, &g)
}

fn nav() -> TreeView {
    let mut fav = Node::new("Favorites", true);
    fav.children = vec![
        Node::new("Desktop", false),
        Node::new("Downloads", false),
        Node::new("Projects", false),
    ];
    fav.expanded = true;
    let mut computer = Node::new("Computer", true);
    computer.children = vec![Node::new("Local Disk", true), Node::new("Backup", true)];
    computer.expanded = true;
    let roots = vec![
        fav,
        Node::new("Home", true),
        computer,
        Node::new("Network", false),
        Node::new("Recycle Bin", false),
    ];
    TreeView::new(roots, Style::Nav)
}

/// A state's name and what sets it up.
type Scene = (&'static str, fn(&mut TreeView));

fn nav_scenes() -> [Scene; 4] {
    [
        ("rest", |_| {}),
        ("hot, expanders shown", |tv| {
            tv.over = Over {
                inside: true,
                row: Some(7),
                expander: false,
            };
            tv.show_expanders(true, Instant::now() - Duration::from_secs(1));
            tv.tick_expanders(Instant::now());
        }),
        ("selected", |tv| tv.selected = Some(vec![2])),
        ("selected, focus elsewhere", |tv| {
            tv.selected = Some(vec![2, 0]);
            tv.inactive = true;
        }),
    ]
}

/// One navigation pane in `t` at scale `s`, laid out at 100%, into `c` at device (x, y).
fn draw_nav(c: &mut Canvas, t: &Theme, s: i32, (x, y): (i32, i32), set: fn(&mut TreeView)) {
    let mut tv = nav();
    set(&mut tv);
    let one = TextRenderer::load(&t.fonts);
    let frame = Rect::new(0, 0, NAV.0, NAV.1);
    let eb = ExplorerBar {
        width: NAV.0 - explorerbar::SPLITTER_W,
        ..ExplorerBar::new("Navigation pane")
    };
    let bl = explorerbar::nav_layout(&eb, frame);
    let tl = treeview::layout(&tv, &one, t.fonts.size_px, bl.body, &t.controls());
    let ts = t.scaled(s);
    let text = TextRenderer::load(&ts.fonts);
    let mut pane = Canvas::new((NAV.0 * s) as u32, (NAV.1 * s) as u32);
    let p = ts.paint();
    explorerbar::draw(&mut pane, &text, &p, &eb, &bl, ts.fonts.size_px);
    treeview::draw(
        &mut pane,
        &text,
        &p,
        &tv,
        &tl,
        Default::default(),
        ts.fonts.size_px,
    );
    c.blit_canvas(x, y, &pane);
}

fn icon(s: i32, rgb: (u8, u8, u8)) -> Canvas {
    let e = detailspane::ICON * s;
    let mut c = Canvas::new(e as u32, e as u32);
    c.fill_rect(
        Rect::new(6 * s, 10 * s, 36 * s, 30 * s),
        Color::rgb(rgb.0, rgb.1, rgb.2),
    );
    c
}

fn details_scenes(s: i32) -> [Facts; 3] {
    let row = |n: &str, v: &str| (n.to_string(), v.to_string());
    [
        Facts {
            icon: Some(icon(s, (0xE8, 0xC0, 0x50))),
            title: "Documents".into(),
            subtitle: "11 items".into(),
            rows: vec![row("Space free:", "120 GB")],
        },
        Facts {
            icon: Some(icon(s, (0x50, 0x80, 0xC8))),
            title: "Quarterly report.odt".into(),
            subtitle: "Text Document".into(),
            rows: vec![
                row("Date modified:", "11/28/2026 9:41 AM"),
                row("Size:", "48.2 KB"),
                row("Dimensions:", "1920 x 1080"),
                row("Original location:", "/home/user/Documents"),
            ],
        },
        Facts {
            icon: Some(icon(s, (0x50, 0x80, 0xC8))),
            title: "3 items selected".into(),
            subtitle: String::new(),
            rows: vec![row("Size:", "12.6 KB")],
        },
    ]
}

fn draw_details(c: &mut Canvas, t: &Theme, s: i32, (x, y): (i32, i32), facts: Facts) {
    let one = TextRenderer::load(&t.fonts);
    let r = Rect::new(0, 0, DETAILS.0, DETAILS.1);
    let probe = DetailsPane {
        facts: Facts {
            icon: None,
            ..facts.clone()
        },
    };
    let lay = detailspane::layout(&probe, &one, t.fonts.size_px, r, &t.controls());
    let ts = t.scaled(s);
    let text = TextRenderer::load(&ts.fonts);
    let mut pane = Canvas::new((DETAILS.0 * s) as u32, (DETAILS.1 * s) as u32);
    let dp = DetailsPane { facts };
    detailspane::draw(&mut pane, &text, &ts.paint(), &dp, &lay, ts.fonts.size_px);
    c.blit_canvas(x, y, &pane);
}

fn draw_heading(c: &mut Canvas, t: &Theme, s: i32, (x, y): (i32, i32)) {
    let mut sv = shellview::sample(Mode::Tiles, false);
    sv.groups = vec![
        Group {
            title: "Hard Disk Drives (2)".into(),
            first: 0,
        },
        Group {
            title: "Devices with Removable Storage (1)".into(),
            first: 2,
        },
    ];
    let r = Rect::new(0, 0, HEADING.0, HEADING.1);
    let lay = shellview::layout(&sv, r, &t.controls());
    let ts = t.scaled(s);
    let text = TextRenderer::load(&ts.fonts);
    let mut view = Canvas::new((HEADING.0 * s) as u32, (HEADING.1 * s) as u32);
    shellview::draw(
        &mut view,
        &text,
        &ts.paint(),
        &sv,
        &lay,
        None,
        ts.fonts.size_px,
    );
    c.blit_canvas(x, y, &view);
}

/// The page at scale `s`: a column per mode, light then dark.
pub fn render_glass_panes(s: i32) -> Canvas {
    render_panes(glass::Style::Glass, s)
}

/// [`render_glass_panes`] in `style`.
pub fn render_panes(style: glass::Style, s: i32) -> Canvas {
    let col = DETAILS.0.max(4 * (NAV.0 + GAP)) + GAP;
    let h = GAP + NAV.1 + GAP + 3 * (DETAILS.1 + GAP) + HEADING.1 + GAP;
    let mut c = Canvas::new((2 * col * s) as u32, (h * s) as u32);
    c.fill_rect(c.bounds(), Color::rgb(0x80, 0x80, 0x80));
    for (m, mode) in glass::Mode::ALL.into_iter().enumerate() {
        let t = theme(mode, style);
        let x0 = m as i32 * col + GAP;
        for (i, (_, set)) in nav_scenes().into_iter().enumerate() {
            draw_nav(
                &mut c,
                &t,
                s,
                ((x0 + i as i32 * (NAV.0 + GAP)) * s, GAP * s),
                set,
            );
        }
        let mut y = GAP + NAV.1 + GAP;
        for facts in details_scenes(s) {
            draw_details(&mut c, &t, s, (x0 * s, y * s), facts);
            y += DETAILS.1 + GAP;
        }
        draw_heading(&mut c, &t, s, (x0 * s, y * s));
    }
    c
}
