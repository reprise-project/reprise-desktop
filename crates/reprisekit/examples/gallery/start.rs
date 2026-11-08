use super::cued;
use reprisekit::controls::menu::MenuItem;
use reprisekit::interface::Interface;
use reprisekit::interfaces;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::TextRenderer;
use reprisekit::textedit::EditKey;
use reprisekit::theme::{Color, FontConfig};

/// The Start panel over a flat blue desktop: Home, the program tree, Search, a jump list, and Home under Classic.
pub fn render_start_panel() -> Canvas {
    use reprisekit::startpanel::{
        self as sp, Mode, PlaceRow, ProgramRow, ResultGroup, ResultRow, Spot, StartPanel, TreeRow,
    };
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
        icon: Some(square(32, blue)),
        jump,
        new_pin: false,
    };
    let row = |name: &str| ResultRow {
        name: name.into(),
        icon: Some(square(16, blue)),
    };
    let app = |name: &str, depth: u8| TreeRow::App {
        name: name.into(),
        icon: Some(square(16, blue)),
        depth,
    };
    let base = || {
        let mut p = StartPanel::new();
        p.pinned = vec![prog("Text Editor", false)];
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
        .map(|&(n, j)| prog(n, j))
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
        p.jump = vec![row("Personalize"), row("notes.txt"), row("todo.txt")];
        p.jump_title = "Recent".into();
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
                title: "Files (2)".into(),
                rows: vec![row("notes.txt"), row("north.png")],
            },
        ]);
        p
    };
    let jump = {
        let mut p = base();
        p.set_mode(Mode::Jump(2));
        p.pointer(Some(Spot::Jump(1)));
        p
    };
    let glass = reprisekit::glass::Settings::default();
    let glass_theme = cued(interfaces::theme_for(Interface::Glass));
    let classic = cued(interfaces::theme_for(Interface::Classic));
    let text = interfaces::start_panel_text(&glass_theme);
    let (cell_w, cell_h) = (520, 540);
    let mut c = Canvas::new(3 * cell_w as u32, 2 * cell_h as u32);
    c.fill_rect(c.bounds(), Color::rgb(67, 149, 209));
    let panels = [
        (&home_lit, &glass_theme),
        (&place_lit, &glass_theme),
        (&programs, &glass_theme),
        (&search, &glass_theme),
        (&jump, &glass_theme),
        (&home_lit, &classic),
    ];
    for (k, (p, theme)) in panels.into_iter().enumerate() {
        let l = sp::layout::layout(p);
        let mut panel = Canvas::new(l.canvas.width as u32, l.canvas.height as u32);
        interfaces::draw_start_panel(&mut panel, theme, &glass, &text, p, &l, None, true);
        let (x, y) = ((k % 3) as i32 * cell_w + 10, (k / 3) as i32 * cell_h + 10);
        c.blit_over(x, y + sp::layout::OVERHANG + l.canvas.y, &panel);
    }
    c
}

/// The Blue Start menu at scale `n`: Blue at rest, a lit pinned row, a lit place and a hot Log Off,
/// then Olive Green with All Programs lit and Silver with Turn Off Computer pressed.
pub fn render_blue_start_panel(n: i32) -> Canvas {
    use reprisekit::blue::Variant;
    use reprisekit::startpanel::{self as sp, PlaceRow, ProgramRow, Shape, Spot, StartPanel};
    let square = |px: i32, c: Color| {
        let px = px * n;
        let mut i = Canvas::new(px as u32, px as u32);
        i.fill_rect(Rect::new(px / 8, px / 8, px * 3 / 4, px * 3 / 4), c);
        i
    };
    let blue = Color::rgb(40, 120, 220);
    let prog = |name: &str, detail: Option<&str>| ProgramRow {
        name: name.into(),
        icon: Some(square(32, blue)),
        jump: false,
        new_pin: false,
        detail: detail.map(String::from),
    };
    let base = || {
        let mut p = StartPanel::new();
        p.shape = Shape::Blue;
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
            picture: Some(square(24, Color::rgb(230, 200, 90))),
            rule_before: rule,
            submenu: label == "Recent Items",
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
    let panels = [
        (base(), Variant::Blue, None),
        (lit(Spot::Program(0)), Variant::Blue, None),
        (lit(Spot::Place(1)), Variant::Blue, None),
        (lit(Spot::LogOff), Variant::Blue, None),
        (lit(Spot::AllPrograms), Variant::Olive, None),
        (lit(Spot::Shutdown), Variant::Silver, Some(Spot::Shutdown)),
    ];
    let (cell_w, cell_h) = (400 * n, 500 * n);
    let mut c = Canvas::new(3 * cell_w as u32, 2 * cell_h as u32);
    c.fill_rect(c.bounds(), Color::rgb(67, 149, 209));
    for (k, (p, variant, pressed)) in panels.iter().enumerate() {
        let mut p = p.clone();
        p.foot_labels = reprisekit::blue::startpanel::foot_labels(&text, sp::search::Grow::at(n));
        let p = &p;
        let l = sp::layout::layout(p).scaled(n);
        let mut panel = Canvas::new(l.canvas.width as u32, l.canvas.height as u32);
        interfaces::draw_blue_start_panel(&mut panel, *variant, &text, p, &l, *pressed);
        let (x, y) = (
            (k % 3) as i32 * cell_w + 10 * n,
            (k / 3) as i32 * cell_h + 10 * n,
        );
        c.blit_over(x, y, &panel);
    }
    c
}

/// The menus the Blue Start menu opens at scale `n`, a scheme a column (Blue, Olive Green,
/// Silver): All Programs with a folder lit and open, All Programs with a program lit, and an empty
/// Recent Items with its greyed row lit.
pub fn render_blue_start_menus(n: i32) -> Canvas {
    use reprisekit::blue::Variant;
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
    let mut c = Canvas::new((3 * cell_w * n) as u32, (3 * cell_h * n) as u32);
    c.fill_rect(c.bounds(), Color::rgb(67, 149, 209));
    let base = interfaces::theme_for(Interface::Classic);
    let usable = Rect::new(0, 0, 3 * cell_w, 3 * cell_h);
    for (col, v) in [Variant::Blue, Variant::Olive, Variant::Silver]
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
