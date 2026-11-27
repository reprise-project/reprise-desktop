use super::cued;
use crate::interfaces;
use reprisekit::controls::menu::MenuItem;
use reprisekit::interface::Interface;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::TextRenderer;
use reprisekit::textedit::EditKey;
use reprisekit::theme::{Color, FontConfig};

/// The Start panel over a flat blue desktop at `pct` text: Home, the program tree, Search, a jump
/// list, and Home under Classic.
pub fn render_start_panel(pct: u32) -> Canvas {
    start_panels(
        pct,
        reprisekit::glass::Mode::Light,
        1,
        reprisekit::glass::Style::Glass,
    )
}

/// [`render_start_panel`]'s Glass panels in dark Glass at scale `n`, Shut down keyed in the last.
pub fn render_start_panel_dark(n: i32) -> Canvas {
    start_panels(
        100,
        reprisekit::glass::Mode::Dark,
        n,
        reprisekit::glass::Style::Glass,
    )
}

/// The Onyx Start menu in `mode` at scale `n`: a lit program, a lit place showing its picture,
/// the program tree, Search beside the places, the power button hot, and the lock button keyed
/// with the arrow held.
pub fn render_onyx_start_panel(n: i32, mode: reprisekit::glass::Mode) -> Canvas {
    start_panels(100, mode, n, reprisekit::glass::Style::Onyx)
}

fn start_panels(
    pct: u32,
    mode: reprisekit::glass::Mode,
    n: i32,
    style: reprisekit::glass::Style,
) -> Canvas {
    use reprisekit::glass::Style;
    use reprisekit::startpanel::{
        self as sp, Mode, PlaceRow, ProgramRow, ResultGroup, ResultRow, Spot, StartPanel, TreeRow,
    };
    let u = reprisekit::theme::Theme::default().with_text_size(pct).text;
    let grown = |px: u32| u.len(px as i32).max(px as i32) as u32;
    let square = |px: u32, c: Color| {
        let mut i = Canvas::new(px, px);
        let n = px as i32;
        i.fill_rect(Rect::new(n / 8, n / 8, n * 3 / 4, n * 3 / 4), c);
        i
    };
    let blue = Color::rgb(40, 120, 220);
    let prog = |name: &str, jump: bool| ProgramRow {
        detail: None,
        name: name.into(),
        icon: Some(square(grown(32), blue)),
        jump,
        new_pin: false,
    };
    let row = |name: &str| ResultRow {
        name: name.into(),
        icon: Some(square(grown(16), blue)),
        detail: String::new(),
    };
    let app = |name: &str, depth: u8| TreeRow::App {
        name: name.into(),
        icon: Some(square(grown(16), blue)),
        depth,
    };
    let onyx = style == Style::Onyx;
    let base = || {
        let mut p = StartPanel::new();
        if onyx {
            p.shape = sp::Shape::Onyx;
        }
        p.text = u;
        p.pinned = vec![prog("Text Editor", false)];
        if onyx {
            p.pinned[0].detail = Some("Plain text".into());
        }
        p.frequent = [
            ("Calculator", false),
            ("Getting Started", true),
            ("Media Player", false),
            ("Paint", false),
            ("Sticky Notes", false),
            ("Screenshot", false),
            ("Remote Desktop Connection", false),
            ("Magnifier", false),
            ("Solitaire", false),
        ]
        .iter()
        .map(|&(n, j)| prog(n, j && !onyx))
        .collect();
        p.places = [
            "user",
            "Documents",
            "Pictures",
            "Music",
            "Recent Items",
            "Computer",
            "Control Panel",
            "Devices and Printers",
            "Default Programs",
            "Help and Support",
        ]
        .iter()
        .enumerate()
        .map(|(k, l)| PlaceRow {
            label: l.to_string(),
            picture: (k == 1).then(|| square(48, Color::rgb(230, 200, 90))),
            rule_before: k == 4 || k == 6,
            submenu: k == 4,
            context: false,
        })
        .collect();
        p.user_picture = Some(square(48, Color::rgb(240, 120, 30)));
        p.tree = vec![
            app("Default Programs", 0),
            app("Web Browser", 0),
            TreeRow::Folder {
                name: "Accessories".into(),
                open: true,
                icon: Some(square(16, Color::rgb(230, 200, 90))),
            },
            app("Calculator", 1),
            app("Command Prompt", 1),
            app("Connect to a Projector", 1),
            app("Getting Started", 1),
            app("Math Input Panel", 1),
            app("Notepad", 1),
            app("Paint", 1),
            app("Remote Desktop Connection", 1),
            app("Run", 1),
            app("Screenshot", 1),
            app("Sound Recorder", 1),
            app("Sticky Notes", 1),
            app("Sync Center", 1),
            app("File Manager", 1),
            app("Word Processor", 1),
            app("Zip Archiver", 1),
            TreeRow::Folder {
                name: "Games".into(),
                open: false,
                icon: Some(square(16, Color::rgb(230, 200, 90))),
            },
            TreeRow::Folder {
                name: "Maintenance".into(),
                open: false,
                icon: Some(square(16, Color::rgb(230, 200, 90))),
            },
        ];
        p.jump = vec![
            ResultGroup {
                title: "Recent".into(),
                rows: vec![row("notes.txt"), row("todo.txt")],
            },
            ResultGroup {
                title: "Tasks".into(),
                rows: vec![row("Personalize"), row("New Window")],
            },
        ];
        p
    };
    let home_lit = {
        let mut p = base();
        p.pointer(Some(Spot::Program(1)));
        p
    };
    let place_lit = {
        let mut p = base();
        p.pointer(Some(Spot::Place(1)));
        p
    };
    let programs = {
        let mut p = base();
        p.set_mode(Mode::Programs);
        p.pointer(Some(Spot::Tree(4)));
        p
    };
    let search = {
        let mut p = base();
        p.key(sp::Key::Char(EditKey::Insert("no".into())));
        p.set_results(vec![
            ResultGroup {
                title: "Programs (3)".into(),
                rows: vec![row("Notepad"), row("Sticky Notes"), row("Web Browser")],
            },
            ResultGroup {
                title: "Settings (1)".into(),
                rows: vec![ResultRow {
                    detail: "Hold balloons during quiet hours".into(),
                    ..row("Notifications")
                }],
            },
            ResultGroup {
                title: "Files (3)".into(),
                rows: vec![
                    ResultRow {
                        detail: "~/Documents".into(),
                        ..row("notes.txt")
                    },
                    ResultRow {
                        detail: "~/Projects/Reprise".into(),
                        ..row("notes.txt")
                    },
                    row("north.png"),
                ],
            },
        ]);
        p
    };
    let jump = {
        let mut p = base();
        p.set_mode(Mode::Jump(2));
        p.pointer(Some(Spot::Jump(2)));
        p
    };
    let power_hot = {
        let mut p = base();
        p.pointer(Some(Spot::Shutdown));
        p
    };
    let lock_keyed = {
        let mut p = base();
        p.focus_on(Spot::Lock);
        p
    };
    let glass = reprisekit::glass::Settings {
        mode,
        style,
        ..reprisekit::glass::Settings::default()
    };
    let glass_theme = cued(interfaces::theme_with(
        Interface::Glass,
        reprisekit::blue::Variant::default(),
        false,
        &glass,
    ))
    .scaled(n);
    let classic = cued(interfaces::theme_for(Interface::Classic));
    let text = interfaces::start_panel_text(&glass_theme);
    let tall = if onyx { 560 } else { 540 };
    let (cell_w, cell_h) = (u.len(520) * n, u.len(tall) * n);
    let mut c = Canvas::new(3 * cell_w as u32, 2 * cell_h as u32);
    c.fill_rect(c.bounds(), Color::rgb(67, 149, 209));
    let search_keyed = {
        let mut p = search.clone();
        p.focus_on(Spot::Shutdown);
        p
    };
    let last = if onyx {
        (&lock_keyed, &glass_theme, Some(Spot::ShutdownArrow))
    } else if mode == reprisekit::glass::Mode::Dark {
        (&search_keyed, &glass_theme, None)
    } else {
        (&home_lit, &classic, None)
    };
    let fifth = if onyx { &power_hot } else { &jump };
    let panels = [
        (&home_lit, &glass_theme, None),
        (&place_lit, &glass_theme, None),
        (&programs, &glass_theme, None),
        (&search, &glass_theme, None),
        (fifth, &glass_theme, None),
        last,
    ];
    for (k, (p, theme, pressed)) in panels.into_iter().enumerate() {
        let l = sp::layout::layout(p).scaled(n);
        let mut panel = Canvas::new(l.canvas.width as u32, l.canvas.height as u32);
        interfaces::draw_start_panel(&mut panel, theme, &glass, &text, p, &l, pressed, true);
        let (x, y) = ((k % 3) as i32 * cell_w + 10, (k / 3) as i32 * cell_h + 10);
        c.blit_over(x, y + sp::layout::OVERHANG * n + l.canvas.y, &panel);
    }
    c
}

/// The Blue Start menu at scale `n` and `pct` text: Blue at rest, a lit pinned row, a lit place
/// and a hot Log Off, then Olive Green with All Programs lit and Silver with Turn Off Computer
/// pressed, then Blue's dark twin at rest, with a pinned row lit and with a place lit.
pub fn render_blue_start_panel(n: i32, pct: u32) -> Canvas {
    use reprisekit::blue::{Tone, Variant};
    use reprisekit::startpanel::{self as sp, PlaceRow, ProgramRow, Shape, Spot, StartPanel};
    let u = reprisekit::theme::Theme::default().with_text_size(pct).text;
    let grown = |px: i32| u.len(px).max(px);
    let square = |px: i32, c: Color| {
        let px = px * n;
        let mut i = Canvas::new(px as u32, px as u32);
        i.fill_rect(Rect::new(px / 8, px / 8, px * 3 / 4, px * 3 / 4), c);
        i
    };
    let blue = Color::rgb(40, 120, 220);
    let prog = |name: &str, detail: Option<&str>| ProgramRow {
        name: name.into(),
        icon: Some(square(grown(32), blue)),
        jump: false,
        new_pin: false,
        detail: detail.map(String::from),
    };
    let base = || {
        let mut p = StartPanel::new();
        p.shape = Shape::Blue;
        p.text = u;
        p.user = "user".into();
        p.user_picture = Some(square(48, Color::rgb(240, 120, 30)));
        p.pinned = vec![
            prog("Web Browser", Some("Firefox")),
            prog("E-mail", Some("Thunderbird")),
        ];
        p.frequent = [
            "Terminal",
            "Notepad",
            "Calculator",
            "Media Player",
            "Disk Usage",
            "Word Processor",
        ]
        .iter()
        .map(|&name| prog(name, None))
        .collect();
        p.places = [
            ("Documents", false),
            ("Recent Items", false),
            ("Pictures", false),
            ("Music", false),
            ("Computer", false),
            ("Control Panel", true),
            ("Printers", false),
            ("Help and Support", true),
            ("Run...", false),
        ]
        .iter()
        .map(|&(label, rule)| PlaceRow {
            label: label.into(),
            picture: Some(square(grown(24), Color::rgb(230, 200, 90))),
            rule_before: rule,
            submenu: label == "Recent Items",
            context: false,
        })
        .collect();
        p
    };
    let lit = |spot: Spot| {
        let mut p = base();
        p.pointer(Some(spot));
        p
    };
    let text = TextRenderer::load(&FontConfig::default());
    let dark = Tone::new(Variant::Blue, true);
    let panels = [
        (base(), Variant::Blue.into(), None),
        (lit(Spot::Program(0)), Variant::Blue.into(), None),
        (lit(Spot::Place(1)), Variant::Blue.into(), None),
        (lit(Spot::LogOff), Variant::Blue.into(), None),
        (lit(Spot::AllPrograms), Variant::Olive.into(), None),
        (
            lit(Spot::Shutdown),
            Variant::Silver.into(),
            Some(Spot::Shutdown),
        ),
        (base(), dark, None),
        (lit(Spot::Program(0)), dark, None),
        (lit(Spot::Place(1)), dark, None),
    ];
    let (cell_w, cell_h) = (u.len(400) * n, u.len(500) * n);
    let mut c = Canvas::new(3 * cell_w as u32, 3 * cell_h as u32);
    c.fill_rect(c.bounds(), Color::rgb(67, 149, 209));
    for (k, (p, tone, pressed)) in panels.iter().enumerate() {
        let mut p = p.clone();
        let g = sp::search::Grow { s: n, u };
        p.foot_labels = reprisekit::blue::startpanel::foot_labels(&text, g);
        let p = &p;
        let l = sp::layout::layout(p).scaled(n);
        let mut panel = Canvas::new(l.canvas.width as u32, l.canvas.height as u32);
        interfaces::draw_blue_start_panel(&mut panel, *tone, &text, p, &l, *pressed);
        let (x, y) = (
            (k % 3) as i32 * cell_w + 10 * n,
            (k / 3) as i32 * cell_h + 10 * n,
        );
        c.blit_over(x, y, &panel);
    }
    c
}

/// The menus the Blue Start menu opens at scale `n`, a scheme a column (Blue, Olive Green,
/// Silver, Blue's dark twin): All Programs with a folder lit and open, All Programs with a program lit, and an empty
/// Recent Items with its greyed row lit.
pub fn render_blue_start_menus(n: i32) -> Canvas {
    use reprisekit::blue::{Tone, Variant};
    use reprisekit::controls::menu::{
        MenuLayout, MenuStyle, draw_menu_at, draw_menu_shadow, place_submenu_beside,
    };
    let text = TextRenderer::load(&FontConfig::default());
    let square = |c: Color| {
        let px = 16 * n;
        let mut i = Canvas::new(px as u32, px as u32);
        i.fill_rect(Rect::new(n, 2 * n, px - 2 * n, px - 4 * n), c);
        i
    };
    let folder = |label: &str| {
        MenuItem::entry(label)
            .icon(Some(square(Color::rgb(230, 200, 90))))
            .submenu(true)
    };
    let program = |label: &str| MenuItem::entry(label).icon(Some(square(Color::rgb(40, 120, 220))));
    let programs = [
        folder("Accessories"),
        folder("Games"),
        folder("System Tools"),
        program("Web Browser"),
        program("Terminal"),
        program("Media Player"),
    ];
    let accessories = [
        program("Calculator"),
        program("Notepad"),
        program("Paint"),
        program("Word Processor"),
    ];
    let empty = [MenuItem::entry("(Empty)").enabled(false)];
    let none: &[MenuItem] = &[];
    let menus = [
        (&programs[..], 0, &accessories[..]),
        (&programs[..], 4, none),
        (&empty[..], 0, none),
    ];
    let (cell_w, cell_h) = (440, 200);
    let mut c = Canvas::new((4 * cell_w * n) as u32, (3 * cell_h * n) as u32);
    c.fill_rect(c.bounds(), Color::rgb(67, 149, 209));
    let base = interfaces::theme_for(Interface::Classic);
    let usable = Rect::new(0, 0, 4 * cell_w, 3 * cell_h);
    for (col, v) in [
        Variant::Blue.into(),
        Variant::Olive.into(),
        Variant::Silver.into(),
        Tone::new(Variant::Blue, true),
    ]
    .into_iter()
    .enumerate()
    {
        let one = interfaces::blue_start_menus(&base, v);
        let theme = interfaces::blue_start_menus(&base.scaled(n), v);
        let laid_out = |items: &[MenuItem]| {
            let style =
                MenuStyle::context_for(&one, items).fit(&text, items, one.fonts.size_px, 400);
            MenuLayout::compute_styled(&style, items, usize::MAX)
        };
        let level = |c: &mut Canvas, at: (i32, i32), l: &MenuLayout, items: &[MenuItem], lit| {
            let l = l.scaled(n);
            let reach = reprisekit::controls::menu::shadow_reach(&theme);
            c.blit_over(
                at.0 * n,
                at.1 * n,
                &draw_menu_shadow(&theme, l.width, l.height, l.height + reach),
            );
            draw_menu_at(
                c,
                (at.0 * n, at.1 * n),
                &theme,
                &text,
                &l,
                items,
                lit,
                "",
                false,
            );
        };
        for (row, &(items, lit, child)) in menus.iter().enumerate() {
            let at = (col as i32 * cell_w + 10, row as i32 * cell_h + 10);
            let l = laid_out(items);
            level(&mut c, at, &l, items, Some(lit));
            if !child.is_empty() {
                let cl = laid_out(child);
                let r = l.rows[lit];
                let spot = place_submenu_beside(
                    Rect::new(at.0, at.1, l.width, l.height),
                    Rect::new(at.0 + r.x, at.1 + r.y, r.width, r.height),
                    (cl.width, cl.height),
                    usable,
                    cl.inset,
                );
                level(&mut c, spot, &cl, child, None);
            }
        }
    }
    c
}
