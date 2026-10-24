use super::bind::binds;
use super::desktop::{DRAG_SNAP_CONTROLS, FPS_CONTROLS, REVEAL_CONTROLS};
use super::*;
use crate::model::{Settings, Shortcut, WATER_STYLES};
use crate::registry::Page;
use crate::{appearancepage, backgroundpage, registry};
use reprisefs::autostart::Entry;
use reprisefs::xkb::Layout;
use reprisekit::controls::propsheet;
use reprisekit::controls::{form, listview};
use reprisekit::raster::Canvas;
use reprisekit::text::TextRenderer;
use reprisekit::theme::Color;
use reprisekit::theme::{FontConfig, Theme};

mod appearance;
mod desktop;
mod input;
mod system;

fn read_all(sheet: &PropSheet, s: &mut Settings) {
    for b in binds() {
        read_one(sheet, b.id(), s);
    }
}

fn with(s: &Settings) -> PropSheet {
    registry::sheet(
        s,
        &registry::Sources {
            icon_themes: &themes(),
            gtk_themes: &gtk_themes(),
            qt_tools: true,
            cursor_themes: &cursor_themes(),
            layouts: &reg(),
            heads: &[],
        },
    )
}

/// Every control's access key is its page's alone, the sheet's own buttons
/// aside ("&Apply" shares Alt+A with "&Add..." by design), under every interface.
#[test]
fn no_two_controls_on_a_page_share_an_access_key() {
    for i in ["classic", "blue", "glass"] {
        let s = under(i);
        for p in Page::all() {
            let mut seen: Vec<(char, &str)> = Vec::new();
            for item in &s.pages[p.index()].items {
                if matches!(
                    item.id,
                    propsheet::OK | propsheet::CANCEL | propsheet::APPLY
                ) {
                    continue;
                }
                let label = match &item.control {
                    Control::Label { text, .. } => text,
                    Control::Check { label, .. }
                    | Control::Radio { label, .. }
                    | Control::Button { label, .. } => label,
                    _ => continue,
                };
                let Some(k) = reprisekit::text::access_key(label) else {
                    continue;
                };
                if let Some((_, other)) = seen.iter().find(|(c, _)| *c == k) {
                    panic!("{i}, {}: {other} and {} share Alt+{k}", p.label(), item.id);
                }
                seen.push((k, item.id));
            }
        }
    }
}

/// A bound control sits on one page at most under each interface, and on one
/// under some interface: none lost in a move or rebuild, none built twice.
#[test]
fn every_bind_names_a_control_on_exactly_one_page() {
    let sheets = ["classic", "blue", "glass"].map(under);
    for b in binds() {
        let mut shown = false;
        for s in &sheets {
            let on: Vec<&str> = Page::all()
                .filter(|p| s.pages[p.index()].item(b.id()).is_some())
                .map(Page::label)
                .collect();
            assert!(on.len() <= 1, "{} is on {on:?}", b.id());
            shown |= on.len() == 1;
        }
        assert!(shown, "{} is on no page under any interface", b.id());
    }
}

/// Every page's text fits its rects under each interface, in Tahoma and in DejaVu Sans, the
/// default family's second face, at each text size.
#[test]
fn every_page_fits_its_text_under_every_interface() {
    let over = reprisekit::interfaces::form_overflows(|i, _, _| {
        let sheet = under(i.as_str());
        Page::all()
            .map(|p| (p.label().to_string(), sheet.pages[p.index()].clone()))
            .collect()
    });
    assert!(over.is_empty(), "{}", over.join("\n"));
}

/// The dialogs the pages open fit their text too, with long layout, variant and program names.
#[test]
fn every_dialog_fits_its_text_under_every_interface() {
    let reg = reprisefs::xkb::parse(
        "<layoutList>\
         <layout><configItem><name>us</name><description>English (US)</description>\
         </configItem><variantList><variant><configItem><name>intl</name>\
         <description>English (US, intl., with dead keys)</description></configItem>\
         </variant></variantList></layout>\
         <layout><configItem><name>hu</name><description>Hungarian</description>\
         </configItem><variantList><variant><configItem><name>101_qwerty_comma_dead</name>\
         <description>Hungarian (QWERTY, 101-key, comma, dead keys)</description>\
         </configItem></variant></variantList></layout>\
         </layoutList>",
    );
    let app = |name: &str| {
        let entry = format!("[Desktop Entry]\nType=Application\nName={name}\nExec=true\n");
        reprisefs::apps::parse_desktop(&entry, "a.desktop").unwrap()
    };
    let long = Shortcut::new(
        "Super+Shift+Alt+Page_Down",
        "spawn",
        "flatpak run org.example.App",
    );
    let over = reprisekit::interfaces::form_overflows(|_, _, _| {
        let mut forms = vec![
            (
                "Add Item".into(),
                crate::additem::AddItemDialog::new(
                    vec![app("LibreOffice Impress"), app("Text Editor")],
                    "foot",
                )
                .form,
            ),
            (
                "Add Layout".into(),
                crate::layoutdlg::LayoutDialog::new(&reg).form,
            ),
            (
                "Keep display".into(),
                crate::monitordlg::MonitorDialog::new(15, None).form,
            ),
            (
                "Rename".into(),
                crate::renamedlg::RenameDialog::new("Documents").form,
            ),
            (
                "Shortcut".into(),
                crate::shortcutdlg::ShortcutDialog::new(Some(&long), vec![]).form,
            ),
            (
                "Program shortcut".into(),
                crate::shortcutdlg::ShortcutDialog::for_program("Ctrl+Alt+T", vec![]).form,
            ),
        ];
        for kind in reprisekit::saver::Kind::ALL {
            let d = crate::saverdlg::SaverDialog::new(kind, &Settings::default());
            forms.push((format!("{kind:?} settings"), d.form));
        }
        forms
    });
    assert!(over.is_empty(), "{}", over.join("\n"));
}

fn under(interface: &str) -> PropSheet {
    with(&Settings {
        interface: interface.into(),
        ..Settings::default()
    })
}

fn pick(s: &mut PropSheet, got: &mut Settings, page: Page, id: &str, row: usize) {
    if let Some(Control::Combo(c)) = s.pages[page.index()].item_mut(id).map(|i| &mut i.control) {
        c.selected = row;
    }
    read_one(s, id, got);
    follow(s, got);
}

fn tick(s: &mut PropSheet, got: &mut Settings, page: Page, id: &str, on: bool) {
    if let Some(Control::Check { checked, .. }) =
        s.pages[page.index()].item_mut(id).map(|i| &mut i.control)
    {
        *checked = on;
    }
    read_one(s, id, got);
    follow(s, got);
}

fn rows(s: &PropSheet, page: Page, id: &str) -> (Vec<String>, usize) {
    match &s.pages[page.index()]
        .item(id)
        .unwrap_or_else(|| panic!("{id} is on {}", page.label()))
        .control
    {
        Control::Combo(c) => (c.items.iter().map(|i| i.text.clone()).collect(), c.selected),
        _ => panic!("{id} is a combo"),
    }
}

/// The lists the host draws over the Startup and Keyboard pages move with
/// their pages: right of the sheet's list, inside the page, flush with the buttons under them.
#[test]
fn the_side_lists_move_with_their_pages() {
    let text = TextRenderer::load(&FontConfig::default());
    let s = built();
    let page = s.layout(&text, Theme::default().fonts.size_px).page;
    let list = s.on_page(list(Area::MIN));
    assert_eq!((list.x, list.y, list.width), (166, 44, 358));
    assert!(list.x >= page.x && list.right() <= page.right() && list.bottom() <= page.bottom());
    let remove = s.pages[Page::Startup.index()].item(REMOVE).unwrap().rect;
    assert_eq!(
        remove.right(),
        list.right(),
        "Remove is flush with the list"
    );
    let shortcuts = s.on_page(shortcuts(Area::MIN));
    let delete = s.pages[Page::Keyboard.index()]
        .item(SC_REMOVE)
        .unwrap()
        .rect;
    assert_eq!(delete.right(), shortcuts.right());
}

fn themes() -> Vec<String> {
    vec!["hicolor".into(), "Adwaita".into()]
}

fn reg() -> Vec<Layout> {
    reprisefs::xkb::parse(
        "<layoutList>\
         <layout><configItem><name>us</name><shortDescription>en</shortDescription>\
         <description>English (US)</description></configItem></layout>\
         <layout><configItem><name>hu</name><shortDescription>hu</shortDescription>\
         <description>Hungarian</description></configItem></layout>\
         </layoutList>",
    )
}

fn cursor_themes() -> Vec<String> {
    vec!["Adwaita".into(), "breeze_cursors".into()]
}

fn gtk_themes() -> Vec<String> {
    vec!["Adwaita".into(), "Arc-Dark".into()]
}

fn first_ink(c: &Canvas, r: Rect) -> Option<i32> {
    (r.x..r.right()).find(|&x| (r.y..r.bottom()).any(|y| c.get(x, y) & 0xFF < 0xA0))
}

/// The text under a check box or radio starts at that choice's label column in
/// every look: the shortcuts note, the Desktops page's indented rows and the
/// Night Light times row under "At fixed times". That row takes the check
/// box's column, so under Classic and Blue, whose radio label sits a pixel left
/// of a check box's, it starts a pixel right of the radio's.
#[test]
fn lines_under_a_choice_start_at_its_label_in_every_look() {
    if !reprisekit::text::test_tahoma() {
        return;
    }
    use reprisekit::interface::Interface;
    let text = TextRenderer::load(&FontConfig::default());
    for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
        let theme = reprisekit::interfaces::theme_for(i);
        let cm = theme.look.controls();
        for (form, choice, under, past) in [
            (
                accessibility(Area::MIN),
                "key_shortcuts",
                "shortcuts_note",
                0,
            ),
            (desktops(Area::MIN), "use_desktops", "desktops_label", 0),
            (
                night_light(Area::MIN),
                "nl_fixed",
                "nl_from_label",
                cm.check_label - cm.radio_label,
            ),
        ] {
            let mut c = Canvas::new(420, 420);
            c.fill_rect(c.bounds(), theme.colors.button_face);
            form::draw(&mut c, &text, &theme.paint(), &form, false, 11);
            let at = |id: &str| {
                let it = form.item(id).unwrap();
                let r = it.placed(&cm);
                Rect::new(r.x - 4, r.y, 60, 13)
            };
            let box_x = form.item(choice).unwrap().rect.x;
            let label = Rect {
                x: box_x + 14,
                ..at(choice)
            };
            assert_eq!(
                first_ink(&c, at(under)),
                first_ink(&c, label).map(|x| x + past),
                "{i:?}: {under} starts {past} px past {choice}'s label"
            );
        }
    }
}

fn built() -> PropSheet {
    with(&Settings::default())
}

fn rect(sheet: &PropSheet, page: Page, id: &str) -> Rect {
    let r = sheet.pages[page.index()]
        .item(id)
        .unwrap_or_else(|| panic!("{id} is on {}", page.label()))
        .placed(&reprisekit::controls::metrics::ControlMetrics::CLASSIC);
    let (dx, dy) = sheet.page_offset();
    Rect::new(r.x - dx, r.y - dy, r.width, r.height)
}

fn inside_page(sheet: &PropSheet, page: Page) {
    let text = TextRenderer::load(&FontConfig::default());
    let px = Theme::default().fonts.size_px;
    let p = sheet.layout(&text, px).page;
    for item in &sheet.pages[page.index()].items {
        let r = item.placed(&reprisekit::controls::metrics::ControlMetrics::CLASSIC);
        if matches!(
            item.id,
            propsheet::OK | propsheet::CANCEL | propsheet::APPLY
        ) {
            continue;
        }
        assert!(
            r.x >= p.x && r.y >= p.y && r.right() <= p.right() && r.bottom() <= p.bottom(),
            "{} at {r:?} escapes the page {p:?}",
            item.id
        );
    }
}

fn enabled_on(s: &PropSheet, page: Page, id: &str) -> bool {
    match &s.pages[page.index()].item(id).unwrap().control {
        Control::Check { enabled, .. }
        | Control::Label { enabled, .. }
        | Control::Edit { enabled, .. } => *enabled,
        Control::Combo(c) => c.enabled,
        Control::UpDown(u) => u.enabled,
        Control::TrackBar(t) => t.enabled,
        _ => panic!("{id} has no greyed state here"),
    }
}

#[test]
fn values_go_in_and_come_back_out() {
    let want = Settings {
        desktop_icons: false,
        desktop_extensions: "show".into(),
        wallpaper_position: "tile".into(),
        background: "#336699".into(),
        system_icons: vec!["computer".into(), "recycle-bin".into()],
        start_icon: false,
        adaptive_sync: true,
        preview_taskbar: false,
        previews_fps: 24,
        switcher_style: "thumbnails".into(),
        screenshot_save: false,
        clipboard_history: false,
        desktops: 9,
        desktop_animation: "zoom".into(),
        desktop_animation_ms: 400,
        remember_windows: false,
        session_restore: true,
        session_end_processes: false,
        hibernate: "zzz".into(),
        lock_on_sleep: false,
        taskbar_height: 32,
        icon_theme: "hicolor".into(),
        font_family: "Verdana".into(),
        color_scheme: "Brick".into(),
        ..Settings::default()
    };
    let sheet = with(&want);

    let mut got = Settings::default();
    read_all(&sheet, &mut got);
    assert_eq!(got, want);

    let glass = Settings {
        interface: "glass".into(),
        glass_color: "ruby".into(),
        glass_intensity: 70,
        glass_transparency: false,
        glass_blur: 4,
        glass_streaks: false,
        glass_fast_drag: false,
        glass_blur_frames: false,
        ..Settings::default()
    };
    let sheet = with(&glass);
    let mut got = Settings {
        interface: "glass".into(),
        ..Settings::default()
    };
    read_all(&sheet, &mut got);
    assert_eq!(got, glass);
}

#[test]
fn only_the_control_that_changed_is_read_back() {
    // Past the idle up-down's maximum: the control has to clamp it to show it at
    // all, but that clamp must not reach `Settings` and be written over a key the user never touched.
    let want = Settings {
        lock_after: 99_999_999,
        ..Settings::default()
    };
    let sheet = with(&want);

    let mut got = want.clone();
    read_one(&sheet, "taskbar_height", &mut got);
    assert_eq!(
        got.lock_after, 99_999_999,
        "an untouched field keeps what the file said"
    );

    // Naming it is the user having worked it, and then the clamp is what they see and what gets written.
    read_one(&sheet, "lock_after", &mut got);
    assert_eq!(got.lock_after, 99_999);
}

/// Writes target/reprise-settings-<page>.png for each page.
#[test]
#[ignore]
fn render_the_pages() {
    let theme = Theme {
        access_keys: true,
        ..Theme::default()
    };
    let text = TextRenderer::load(&FontConfig::default());
    let px = theme.fonts.size_px;
    let two = Settings {
        kb_layout: "us,hu".into(),
        ..Settings::default()
    };
    let mut sheet = with(&two);
    fill_layouts(&mut sheet, &two, &reg(), Some(1));
    let pic = crate::cursors::preview("Adwaita", 32, preview_size(Area::MIN), theme.colors.window);
    sheet.pages[Page::Pointers.index()].set_picture(CURSOR_PREVIEW, pic);
    backgroundpage::show(
        &mut sheet.pages[Page::Background.index()],
        &two,
        "",
        None,
        false,
    );
    let mut lv = new_list();
    let entries: Vec<Entry> = [
        (
            "reprise-sound",
            "Sounds and Audio Devices",
            "reprise-sound --tray",
            false,
        ),
        ("nm-applet", "Network", "nm-applet", false),
        ("mine", "My Script", "/home/user/bin/mine", true),
    ]
    .iter()
    .map(|(id, name, exec, user)| Entry {
        id: (*id).into(),
        name: (*name).into(),
        exec: (*exec).into(),
        exec_line: (*exec).into(),
        hidden: false,
        only_show_in: Vec::new(),
        not_show_in: Vec::new(),
        try_exec: None,
        path: None,
        terminal: false,
        under_systemd: false,
        user: *user,
    })
    .collect();
    fill_list(&mut lv, &entries, &[true, false, true]);
    lv.selection.set_only(Some(0));
    let keyboard = crate::model::load(
        "[[global_shortcuts]]\napp = \"org.example.Talk\"\nid = \"talk\"\n\
         description = \"Hold to talk\"\nkeys = \"Pause\"\n\
         [portal]\nshortcuts_denied = [\"org.example.Notes\"]\n",
    );
    let mut sc = new_shortcut_list();
    fill_shortcuts(&mut sc, &keyboard, &|a| {
        a.strip_prefix("org.example.").unwrap_or(a).into()
    });
    let n = keyboard.shortcuts.len();
    sc.top = n - 2;
    sc.selection.set_only(Some(n));

    let mut shown = crate::displaypage::test_head("DP-1");
    shown.adaptive_sync = repriseapp::heads::AdaptiveSync::Off;
    let display = crate::displaypage::Display::with_heads(vec![
        shown,
        crate::displaypage::test_head("HDMI-A-1"),
    ]);
    crate::displaypage::refresh(&mut sheet, &display);
    show_clearing(
        &mut sheet.pages[Page::Advanced.index()],
        Clearing::Done(Cleared {
            files: 214,
            bytes: 1_384_448,
        }),
    );
    for p in Page::all() {
        sheet.tabs.open = p.index();
        sheet.reveal_open();
        let mut c = Canvas::new(sheet.size.0 as u32, sheet.size.1 as u32);
        c.fill_rect(c.bounds(), theme.colors.button_face);
        propsheet::draw(&mut c, &text, &theme.paint(), &sheet, true, px);
        if p == Page::Startup {
            draw_list(
                &mut c,
                &text,
                &theme,
                &lv,
                sheet.on_page(list(Area::MIN)),
                px,
                1,
            );
        }
        if p == Page::Keyboard {
            draw_shortcuts(
                &mut c,
                &text,
                &theme,
                &sc,
                sheet.on_page(shortcuts(Area::MIN)),
                px,
                1,
            );
        }
        reprisekit::raster::write_preview(&c, &format!("reprise-settings-{}.png", p.slug()));
    }
    let mut glass = with(&Settings {
        interface: "glass".into(),
        ..two.clone()
    });
    glass.tabs.open = Page::Appearance.index();
    let mut c = Canvas::new(glass.size.0 as u32, glass.size.1 as u32);
    c.fill_rect(c.bounds(), theme.colors.button_face);
    propsheet::draw(&mut c, &text, &theme.paint(), &glass, true, px);
    reprisekit::raster::write_preview(&c, "reprise-settings-appearance-glass.png");
}
