use super::cued;
use super::popups::monitor_icon;
// Classic helpers with no look::widgets twin.
use reprisekit::classic::widgets::{BUTTON_H, BUTTON_W};
use reprisekit::controls::form::{self, Item};
use reprisekit::controls::propsheet::{self, PropSheet};
use reprisekit::controls::{pagelist, tabstrip};
use reprisekit::interface::Interface;
use reprisekit::interfaces;
use reprisekit::look::Paint;
use reprisekit::look::widgets::{
    ICON_BUTTON_W, draw_button, draw_group_box, draw_icon_button, draw_page,
};
use reprisekit::pointer::Interaction;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::TextRenderer;
use reprisekit::textedit::EditKey;
use reprisekit::theme::{FontConfig, Theme};

/// Glass's push buttons, icon buttons, group box and tabs on its face, 950x160.
pub fn render_glass_sheet(theme: &Theme) -> Canvas {
    let text = TextRenderer::load(&FontConfig::default());
    let p = theme.paint();
    let px = theme.fonts.size_px;
    let mut c = Canvas::new(950, 160);
    c.fill_rect(c.bounds(), theme.colors.button_face);
    let (idle, hot, held) = (Interaction::Idle, Interaction::Hot, Interaction::Pressed);
    for (i, (label, default, ptr, enabled, focus)) in [
        ("Normal", false, idle, true, false),
        ("Hot", false, hot, true, false),
        ("Pressed", false, held, true, false),
        ("Disabled", false, idle, false, false),
        ("Default", true, idle, true, false),
        ("Focused", false, idle, true, true),
        ("Both", true, idle, true, true),
    ]
    .into_iter()
    .enumerate()
    {
        let r = Rect::new(10 + 85 * i as i32, 10, 75, 23);
        draw_button(
            &mut c, &text, &p, r, label, default, ptr, enabled, focus, px,
        );
    }
    let inactive = Paint { active: false, ..p };
    let r = Rect::new(10 + 85 * 7, 10, 75, 23);
    draw_button(
        &mut c, &text, &inactive, r, "Inactive", true, idle, true, false, px,
    );
    let icon = monitor_icon(1);
    for (i, (ptr, enabled)) in [(idle, true), (hot, true), (held, true), (idle, false)]
        .into_iter()
        .enumerate()
    {
        let r = Rect::new(10 + 85 * 8 + 30 * i as i32, 10, ICON_BUTTON_W, 23);
        draw_icon_button(&mut c, &text, &p, r, &icon, enabled, ptr, false, px);
    }
    let wide = Rect::new(10, 45, 160, 23);
    draw_button(
        &mut c, &text, &p, wide, "Wide", false, idle, true, false, px,
    );
    let tall = Rect::new(180, 45, 75, 50);
    draw_button(
        &mut c, &text, &p, tall, "Tall", false, idle, true, false, px,
    );
    let g = Rect::new(270, 50, 180, 90);
    let lr = Rect::new(g.x + 10, g.y - 5, 60, 13);
    draw_group_box(&mut c, &text, &p, g, lr, "&Group box", px);
    let mut strip = tabstrip::TabStrip::new(&["First", "Second", "Third"], 0);
    strip.hot = Some(2);
    strip.focused = true;
    let page = Rect::new(470, 65, 300, 85);
    let tabs = tabstrip::layout(
        &text,
        px,
        &strip,
        page.x,
        page.y - 20,
        page.width,
        reprisekit::theme::TextUnits::ONE,
    );
    draw_page(&mut c, &p, page);
    tabstrip::draw(&mut c, &text, &p, &strip, &tabs, page, true, px);
    let pg = Rect::new(page.x + 10, page.y + 20, 180, 55);
    let lr = Rect::new(pg.x + 10, pg.y - 5, 60, 13);
    draw_group_box(&mut c, &text, &p.on_page(page), pg, lr, "On the page", px);
    c
}

/// The list-mode property sheet in each look: four headings over nine pages, the third open, the list focused.
pub fn render_list_sheets() -> Canvas {
    use pagelist::Item::Page;
    let groups: &[(&str, &[pagelist::Item])] = &[
        (
            "Personalize",
            &[Page("Appearance"), Page("Fonts & Icons"), Page("Effects")],
        ),
        ("Taskbar & Windows", &[Page("Taskbar"), Page("Start Menu")]),
        ("Devices", &[Page("Keyboard"), Page("Mouse & Touchpad")]),
        ("System", &[Page("Power"), Page("Advanced")]),
    ];
    render_list_sheets_of(groups, 2, |_| {})
}

/// The list sheets with seven headings over 26 pages, the last open and scrolled into view.
pub fn render_long_list_sheets() -> Canvas {
    use pagelist::Item::Page;
    let groups: &[(&str, &[pagelist::Item])] = &[
        (
            "Personalize",
            &[
                Page("Appearance"),
                Page("Background"),
                Page("Screen Saver & Lock"),
                Page("Fonts & Icons"),
                Page("Effects"),
            ],
        ),
        (
            "Taskbar & Windows",
            &[
                Page("Taskbar"),
                Page("Start Menu"),
                Page("Search"),
                Page("Window Switcher"),
                Page("Virtual Desktops"),
                Page("Window Behavior"),
            ],
        ),
        (
            "Display & Sound",
            &[Page("Display"), Page("Night Light"), Page("Sound")],
        ),
        (
            "Devices",
            &[
                Page("Keyboard"),
                Page("Mouse & Touchpad"),
                Page("Cursor"),
                Page("Tablet"),
            ],
        ),
        (
            "Accessibility",
            &[Page("Text & Keys"), Page("Magnifier & Filters")],
        ),
        ("Network", &[Page("Remote Desktop")]),
        (
            "System",
            &[
                Page("Power"),
                Page("Startup"),
                Page("Default Programs"),
                Page("Privacy"),
                Page("Advanced"),
                Page("Backup"),
            ],
        ),
    ];
    render_list_sheets_of(groups, 26, |_| {})
}

pub fn link(label: &str) -> pagelist::Item<'_> {
    link_named(label, label)
}

fn link_named<'a>(label: &'a str, name: &'a str) -> pagelist::Item<'a> {
    pagelist::Item::Link { label, name }
}

/// The list sheets with link rows among the pages, Printers greyed.
pub fn render_list_sheet_links() -> Canvas {
    use pagelist::Item::Page;
    let groups: &[(&str, &[pagelist::Item])] = &[
        ("Personalize", &[Page("Appearance"), Page("Effects")]),
        (
            "Devices",
            &[
                Page("Keyboard"),
                Page("Mouse & Touchpad"),
                link("Bluetooth"),
                link("Printers"),
            ],
        ),
        (
            "Network",
            &[
                link_named("Connections", "Network Connections"),
                Page("Remote Desktop"),
            ],
        ),
    ];
    render_list_sheets_of(groups, 1, |sheet| {
        if let propsheet::Nav::List(list) = &mut sheet.nav {
            list.set_link_enabled(1, false);
        }
    })
}

/// The list sheets with "power" typed in the focused search field and the list filtered to
/// Screen Saver & Lock, Power Options and Advanced; Printers greyed.
pub fn render_list_sheet_search(scale: i32) -> Canvas {
    use pagelist::Item::Page;
    let groups: &[(&str, &[pagelist::Item])] = &[
        (
            "Personalize",
            &[Page("Appearance"), Page("Screen Saver & Lock")],
        ),
        (
            "Devices",
            &[Page("Keyboard"), link("Bluetooth"), link("Printers")],
        ),
        ("System", &[link("Power Options"), Page("Advanced")]),
    ];
    render_list_sheets_at(groups, 1, scale, |sheet| {
        let list = sheet.list_mut().expect("a list sheet");
        list.set_link_enabled(1, false);
        list.set_filter(Some(&pagelist::Filter {
            pages: vec![false, true, false, true],
            links: vec![false, false, true],
        }));
        sheet.click(Some(propsheet::Hit::Search(0)));
        sheet.key(form::Key::Edit(EditKey::Insert("power".into())), None);
    })
}

/// `groups` as list sheets in each look, page `open` open and revealed, after `prep`.
fn render_list_sheets_of(
    groups: &[(&str, &[pagelist::Item])],
    open: usize,
    prep: impl Fn(&mut PropSheet),
) -> Canvas {
    render_list_sheets_at(groups, open, 1, prep)
}

/// [`render_list_sheets_of`] drawn at `scale`.
fn render_list_sheets_at(
    groups: &[(&str, &[pagelist::Item])],
    open: usize,
    scale: i32,
    prep: impl Fn(&mut PropSheet),
) -> Canvas {
    let text = TextRenderer::load(&FontConfig::default());
    let count = groups
        .iter()
        .flat_map(|g| g.1.iter())
        .filter(|it| matches!(it, pagelist::Item::Page(_)))
        .count();
    let page = || {
        form::Form::new(vec![
            Item::group("group", Rect::new(20, 44, 358, 84), "Menus"),
            Item::check("check", 32, 66, "&Animate windows opening and closing"),
            Item::button(
                "button",
                Rect::new(291, 96, BUTTON_W, BUTTON_H),
                "Ad&vanced...",
                true,
                false,
            ),
        ])
    };
    let mut c = Canvas::new((564 * scale) as u32, (1312 * scale) as u32);
    c.fill_rect(c.bounds(), Theme::default().colors.background);
    let mut y = 10;
    for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
        let theme = cued(interfaces::theme_for(i)).scaled(scale);
        let pages = (0..count).map(|_| page()).collect();
        let mut sheet = PropSheet::with_list(groups, pages, (398, 424));
        sheet.tabs.open = open;
        sheet.reveal_open();
        sheet.tabs.focused = true;
        sheet.open_mut().set_focus(None);
        sheet.set_apply_enabled(true);
        prep(&mut sheet);
        let (w, h) = sheet.size;
        let mut sc = Canvas::new((w * scale) as u32, (h * scale) as u32);
        sc.fill_rect(sc.bounds(), theme.colors.button_face);
        propsheet::draw(
            &mut sc,
            &text,
            &theme.paint(),
            &sheet,
            true,
            theme.fonts.size_px,
        );
        c.blit_canvas(10 * scale, y * scale, &sc);
        y += h + 10;
    }
    c
}

/// The tray's Customize dialog client in Classic and Blue side by side, Glass's under them; Classic's combo list is open.
pub fn render_tray_customize() -> Canvas {
    use reprisekit::controls::traycustomize::{Behaviour, Entry, TrayCustomize, Wording};
    use reprisekit::icon::IconSource;
    let text = TextRenderer::load(&FontConfig::default());
    let swatch = |argb: u32| -> std::sync::Arc<[IconSource]> {
        std::sync::Arc::from([IconSource {
            width: 16,
            height: 16,
            pixels: vec![argb; 256],
        }])
    };
    let entry = |key: &str, name: &str, argb, passive, behaviour| Entry {
        key: key.into(),
        name: name.into(),
        detail: format!("{name}: all is well"),
        icon: swatch(argb),
        passive,
        behaviour,
    };
    let mut c = Canvas::new(1204, 1033);
    c.fill_rect(c.bounds(), Theme::default().colors.background);
    let (mut x, mut y) = (10, 10);
    for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
        let theme = cued(interfaces::theme_for(i));
        let w = Wording::of(&theme);
        let mut t = TrayCustomize::new(
            w,
            vec![
                entry("nm-applet", "Network", 0xFF20_60C0, false, Behaviour::Auto),
                entry("volume", "Volume", 0xFF40_A040, true, Behaviour::Hide),
            ],
            vec![entry(
                "sticky",
                "Sticky Keys",
                0xFF80_8080,
                false,
                Behaviour::Show,
            )],
            (w == Wording::Glass).then_some(false),
            theme.look.controls(),
        );
        t.select(1);
        if i == Interface::Classic {
            t.combo.toggle();
        }
        let px = theme.fonts.size_px;
        let lay = t.layout(&text, px, (0, 0));
        let (cw, ch) = t.client();
        let mut sc = Canvas::new(cw as u32, ch as u32);
        t.draw(
            &mut sc,
            &text,
            &theme.paint(),
            &lay,
            reprisekit::cues::Cues::NONE,
            px,
        );
        if i == Interface::Glass {
            (x, y) = (10, 411);
        }
        c.blit_canvas(x, y, &sc);
        x += cw + 10;
    }
    c
}
