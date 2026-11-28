use super::tests::{place, prow, row, section};
use super::*;
use crate::interface::Interface;
use crate::interfaces;
use crate::raster::Rect;
use crate::text::TextRenderer;
use crate::theme::{Color, FontConfig, Theme};

const ICON_INK: Color = Color::rgb(0xFF, 0x00, 0xFF);
const SIZES: [u32; 5] = [100, 125, 150, 175, 200];

fn square(px: i32) -> Canvas {
    let mut c = Canvas::new(px as u32, px as u32);
    c.fill_rect(c.bounds(), ICON_INK);
    c
}

fn with_icons(mut p: StartPanel, s: i32) -> StartPanel {
    let program = square(p.icon_px(PROGRAM_ICON, s));
    let small = square(p.icon_px(SMALL_ICON, s));
    let place = square(p.icon_px(crate::blue::startpanel::PLACE_ICON_PX, s));
    for r in p.pinned.iter_mut().chain(&mut p.frequent) {
        r.icon = Some(program.clone());
    }
    if p.shape == Shape::Blue {
        for r in &mut p.places {
            r.picture = Some(place.clone());
        }
    }
    for r in &mut p.tree {
        match r {
            TreeRow::Folder { icon, .. } | TreeRow::App { icon, .. } => *icon = Some(small.clone()),
        }
    }
    for r in (p.jump.iter_mut().chain(&mut p.results)).flat_map(|g| &mut g.rows) {
        r.icon = Some(small.clone());
    }
    p
}

fn programs(p: &mut StartPanel) {
    p.pinned = vec![prow("Web Browser", false), prow("E-mail", false)];
    p.frequent = ["Terminal", "Notepad", "Calculator", "Media Player", "Paint"]
        .iter()
        .enumerate()
        .map(|(k, name)| prow(name, k == 1))
        .collect();
}

fn blue_home() -> StartPanel {
    let mut p = StartPanel::new();
    p.shape = Shape::Blue;
    p.user = "user".into();
    programs(&mut p);
    p.pinned[0].detail = Some("Firefox".into());
    p.places = [
        ("Documents", false),
        ("Recent Items", false),
        ("Pictures", false),
        ("Computer", false),
        ("Control Panel", true),
        ("Run...", true),
    ]
    .iter()
    .map(|&(label, rule)| PlaceRow {
        rule_before: rule,
        submenu: label == "Recent Items",
        context: false,
        ..place(label)
    })
    .collect();
    p
}

/// Glass's Home, its program tree (short, and long enough for its scroll bar), a search and a
/// jump list: every row that wears an icon.
fn glass_modes() -> Vec<(&'static str, StartPanel)> {
    let mut base = StartPanel::new();
    programs(&mut base);
    base.places = ["user", "Documents", "Computer", "Control Panel"]
        .iter()
        .map(|l| place(l))
        .collect();
    base.tree = vec![
        TreeRow::App {
            name: "Calculator".into(),
            icon: None,
            depth: 0,
        },
        TreeRow::Folder {
            name: "Accessories".into(),
            open: true,
            icon: None,
        },
        TreeRow::App {
            name: "Notepad".into(),
            icon: None,
            depth: 1,
        },
    ];
    base.jump = vec![section("Recent", &["notes.txt", "todo.txt"])];
    let mut tree = base.clone();
    tree.set_mode(Mode::Programs);
    let mut search = base.clone();
    search.key(Key::Char(crate::textedit::EditKey::Insert("no".into())));
    search.set_results(vec![ResultGroup {
        title: "Programs (2)".into(),
        rows: vec![row("Notepad"), row("Notes")],
    }]);
    let mut jump = base.clone();
    jump.set_mode(Mode::Jump(3));
    let mut long = base.clone();
    long.tree = (0..30)
        .map(|k| TreeRow::App {
            name: format!("App {k}"),
            icon: None,
            depth: 0,
        })
        .collect();
    long.set_mode(Mode::Programs);
    vec![
        ("home", base),
        ("programs", tree),
        ("search", search),
        ("jump", jump),
        ("scrolled", long),
    ]
}

fn draw(p: &StartPanel, s: i32) -> (Canvas, layout::Layout) {
    let blank = |l: &layout::Layout| Canvas::new(l.canvas.width as u32, l.canvas.height as u32);
    match p.shape {
        Shape::Blue => {
            let text = TextRenderer::load(&FontConfig::default());
            let text = text.for_output(s);
            let mut p = p.clone();
            p.foot_labels =
                crate::blue::startpanel::foot_labels(text, search::Grow { s, u: p.text });
            let l = layout::layout(&p).scaled(s);
            let mut c = blank(&l);
            interfaces::draw_blue_start_panel(
                &mut c,
                crate::blue::Variant::Blue.into(),
                text,
                &p,
                &l,
                None,
            );
            (c, l)
        }
        Shape::Glass | Shape::Onyx => {
            let theme = interfaces::theme_for(Interface::Glass).scaled(s);
            let text = interfaces::start_panel_text(&theme);
            let glass = crate::glass::Settings::default();
            let l = layout::layout(p).scaled(s);
            let mut c = blank(&l);
            interfaces::draw_start_panel(
                &mut c,
                &theme,
                &glass,
                text.for_output(s),
                p,
                &l,
                None,
                false,
            );
            (c, l)
        }
    }
}

/// A spot's row height and the icon it wears, design px at 100% text.
fn row_of(p: &StartPanel, spot: Spot) -> Option<(i32, i32)> {
    let place = crate::blue::startpanel::PLACE_ICON_PX;
    match (spot, p.shape) {
        (Spot::Program(_), Shape::Blue) => Some((40, PROGRAM_ICON)),
        (Spot::Place(_), Shape::Blue) => Some((30, place)),
        (Spot::Program(_), Shape::Glass) => Some((38, PROGRAM_ICON)),
        (Spot::Tree(_), Shape::Glass) => Some((19, SMALL_ICON)),
        (Spot::Jump(_), Shape::Glass) => Some((22, SMALL_ICON)),
        (Spot::Result(..), Shape::Glass) => Some((21, SMALL_ICON)),
        _ => None,
    }
}

/// Draws `p` at every text size at scale 1 and 2 and checks each row that wears an icon: the row
/// and its icon grow by the text size, the icon stands whole in the row, no label ink lands on it
/// and no line's ink is cut. Returns the canvas hashes at 200%.
fn check(name: &str, p: &StartPanel) -> Vec<(String, u64)> {
    let (mut hashes, mut bad, mut cut) = (Vec::new(), Vec::new(), Vec::new());
    let ink = 0xFFFF_00FF;
    for pct in SIZES {
        for s in [1, 2] {
            let mut p = p.clone();
            p.text = Theme::default().with_text_size(pct).text;
            let p = with_icons(p, s);
            let ((c, l), cuts) = crate::pins::clipped_ink(|| draw(&p, s));
            cut.extend(
                cuts.into_iter()
                    .filter(|c| c.rows > 0)
                    .map(|c| (name.to_string(), pct, s, c.text, c.rows)),
            );
            for &(spot, r) in &l.spots {
                let Some((row, design)) = row_of(&p, spot) else {
                    continue;
                };
                let at = format!("{name} {pct}% s{s} {spot:?}");
                let r = Rect::new(r.x - l.canvas.x, r.y - l.canvas.y, r.width, r.height);
                let side = p.icon_px(design, s);
                if side != p.text.len(design) * s || (r.height - p.text.len(row) * s).abs() > s {
                    bad.push(format!("{at}: a {side} px icon in a {} px row", r.height));
                }
                let inked = (r.y..r.bottom())
                    .flat_map(|y| (r.x..r.right()).map(move |x| (x, y)))
                    .filter(|&(x, y)| c.get(x, y) == ink)
                    .count() as i32;
                if inked != side * side {
                    bad.push(format!("{at}: {inked} icon px, not {side}x{side}"));
                }
            }
            if pct == 200 {
                hashes.push((format!("{name} {s}"), crate::pins::fnv(&c)));
            }
        }
    }
    assert_eq!(bad, Vec::<String>::new());
    assert_eq!(
        cut,
        Vec::new(),
        "(panel, text size, scale, line, ink rows cut)"
    );
    hashes
}

#[test]
fn the_blue_menus_icons_grow_with_their_rows_and_the_labels_clear_them() {
    crate::pins::fonts();
    let hashes = check("blue", &blue_home());
    crate::pins::check("blue/start_panel_200", &hashes);
}

#[test]
fn the_glass_menus_icons_grow_with_their_rows_and_the_labels_clear_them() {
    crate::pins::fonts();
    let hashes: Vec<_> = glass_modes()
        .into_iter()
        .flat_map(|(name, p)| check(name, &p))
        .collect();
    crate::pins::check("glass/start_panel_200", &hashes);
}
