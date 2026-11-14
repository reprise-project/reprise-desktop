//! The main page, every control on one canvas, drawn section by section.

use super::page_files::{page_blue, page_files};
use super::page_shell::{page_desk, page_shell};
use reprisekit::art::glyph::Glyph;
use reprisekit::art::viewsicon;
use reprisekit::classic::indicator;
// Classic helpers with no look::widgets twin.
use reprisekit::classic::widgets::{
    BUTTON_GAP, BUTTON_H, BUTTON_W, CLOSE_H, CLOSE_W, CheckState, ROW_H, draw_caption, draw_edit,
    draw_label,
};
use reprisekit::controls::form::{self, Control, Item};
use reprisekit::controls::iconview::{self, Selection};
use reprisekit::controls::menu::{MenuCtx, MenuItem, MenuKey, MenuTree, PopupMenu};
use reprisekit::controls::propsheet::{self, PropSheet};
use reprisekit::controls::treeview::{self, Node, TreeView};
use reprisekit::controls::{
    aboutbox, addressbar, colorbutton, combo, dlgtemplate, explorerbar, filedlg, finddlg, infotip,
    listbox, listview, mixer, msgbox, multiedit, placesbar, rebar, shellview, tabstrip, taskpane,
    toolbar, tooltip, updown,
};
use reprisekit::look::widgets::{
    draw_button, draw_checkbox, draw_checkbox_state, draw_page, draw_scroll_corner, draw_scrollbar,
    draw_sunken_field,
};
use reprisekit::look::{Paint, sample};
use reprisekit::pointer::Interaction;
use reprisekit::raster::{Canvas, EdgeStyle, Rect};
use reprisekit::text::{Align, TextRenderer};
use reprisekit::textedit::{Caret, TextEdit};
use reprisekit::theme::{Color, FontConfig, Theme};
use reprisekit::{schemes, trackbar};

/// Every control on one canvas in `theme`'s colours; `with_blue` adds the Blue bands.
pub fn render(theme: &Theme, with_blue: bool) -> Canvas {
    let text = TextRenderer::load(&FontConfig::default());
    let mono = TextRenderer::load(&FontConfig {
        family: vec!["Liberation Mono".into(), "DejaVu Sans Mono".into()],
        size_px: 13,
        antialias: false,
        italic: false,
        full_hinting: false,
        shell_dlg: None,
    });
    let cs = &theme.colors;

    // Classic mode keeps the shorter canvas: skipping the Blue band must leave the rest where it was.
    let mut c = Canvas::new(950, if with_blue { 14229 } else { 13151 });
    c.fill_rect(Rect::new(0, 0, 950, 1660), cs.background);
    let c = page_dialogs(c, theme, &text, &mono);
    let c = page_explorer(c, theme, &text);
    let c = page_shell(c, theme, &text, with_blue);
    let c = page_desk(c, theme, &text, &mono, with_blue);
    let c = page_files(c, theme, &text);
    page_blue(c, theme, &text, with_blue)
}

/// A 16 px icon stand-in: a `color` box.
pub fn swatch16(color: Color) -> Canvas {
    let mut ic = Canvas::new(16, 16);
    ic.fill_rect(Rect::new(2, 3, 12, 10), color);
    ic
}

/// Captions, edits, buttons, menus, tabs and the dialog boxes of the page.
fn page_dialogs(mut c: Canvas, theme: &Theme, text: &TextRenderer, mono: &TextRenderer) -> Canvas {
    let cs = &theme.colors;
    let px = theme.fonts.size_px;
    let s = 1;

    let dlg = Rect::new(10, 10, 344, 176);
    c.fill_rect(dlg, cs.button_face);
    c.edge_t(dlg, EdgeStyle::WindowRaised, cs, s);
    let cap = Rect::new(dlg.x + 3, dlg.y + 3, dlg.width - 6, 18);
    let close = Rect::new(cap.right() - 2 - CLOSE_W, cap.y + 2, CLOSE_W, CLOSE_H);
    let help = Rect::new(close.x - 2 - CLOSE_W, close.y, CLOSE_W, CLOSE_H);
    draw_caption(
        &mut c,
        text,
        cs,
        cap,
        "Run",
        true,
        &[(close, Glyph::Close), (help, Glyph::Help)],
        px,
        s,
    );
    let idle = Rect::new(dlg.x + 3, 340, dlg.width - 6, 18);
    let idle_close = Rect::new(idle.right() - 2 - CLOSE_W, idle.y + 2, CLOSE_W, CLOSE_H);
    draw_caption(
        &mut c,
        text,
        cs,
        idle,
        "Reprise Settings",
        false,
        &[
            (idle_close, Glyph::Close),
            (
                Rect::new(idle_close.x - 2 - CLOSE_W, idle_close.y, CLOSE_W, CLOSE_H),
                Glyph::Help,
            ),
        ],
        px,
        s,
    );
    let field = Rect::new(dlg.x + 54, dlg.y + 82, 275, ROW_H);
    let inner = draw_sunken_field(
        &mut c,
        &Paint { s, ..theme.paint() },
        field,
        theme.colors.window,
    );
    let mut edit = TextEdit::new("notepad readme.txt", false);
    edit.place(7, false);
    edit.place(0, true);
    draw_edit(
        &mut c,
        text,
        &edit,
        inner,
        px,
        &Paint { s, ..theme.paint() },
        Caret::Shown,
        0,
    );
    draw_label(
        &mut c,
        text,
        Rect::new(dlg.x + 11, field.y, 40, ROW_H),
        "&Open:",
        false,
        cs.button_text,
        Align::Left,
        px,
        s,
    );
    let by = dlg.bottom() - 17 - BUTTON_H;
    for (i, (label, default, enabled, pushed)) in [
        ("OK", true, true, false),
        ("Cancel", false, true, true),
        ("&Browse...", false, false, false),
    ]
    .iter()
    .enumerate()
    {
        let r = Rect::new(
            dlg.x + 86 + i as i32 * (BUTTON_W + BUTTON_GAP),
            by,
            BUTTON_W,
            BUTTON_H,
        );
        draw_button(
            &mut c,
            text,
            &Paint { s, ..theme.paint() },
            r,
            label,
            *default,
            Interaction::pressed_if(*pushed),
            *enabled,
            *default,
            px,
        );
    }
    let cb = Rect::new(dlg.x + 54, dlg.y + 115, 13, 13);
    draw_checkbox(
        &mut c,
        &Paint { s, ..theme.paint() },
        cb,
        true,
        Interaction::Idle,
    );
    let mixed = Rect::new(cb.x, cb.y + 18, 13, 13);
    draw_checkbox_state(
        &mut c,
        &Paint { s, ..theme.paint() },
        mixed,
        CheckState::Mixed,
        Interaction::Idle,
    );
    draw_label(
        &mut c,
        text,
        Rect::new(mixed.right() + 5, mixed.y, 200, 13),
        "&Read-only",
        false,
        cs.button_text,
        Align::Left,
        px,
        s,
    );
    draw_label(
        &mut c,
        text,
        Rect::new(cb.right() + 5, cb.y, 200, 13),
        "&Always ask",
        false,
        cs.button_text,
        Align::Left,
        px,
        s,
    );
    draw_scrollbar(
        &mut c,
        &Paint { s, ..theme.paint() },
        Rect::new(dlg.x + 300, dlg.y + 110, theme.look.controls().scroll, 40),
        1,
        3,
        9,
        None,
    );

    let tiles = MenuItem::item("Ti&les").checked(true).radio(true);
    let items = [
        MenuItem::item("&Open"),
        MenuItem::item("Open &With"),
        MenuItem::Separator,
        MenuItem::item("Cu&t"),
        MenuItem::item("&Copy"),
        MenuItem::Separator,
        tiles,
        MenuItem::item("P&roperties"),
    ];
    let mut tree = MenuTree::new();
    for (i, item) in items.iter().enumerate() {
        if i == 1 {
            tree.push_sub(
                item.clone(),
                MenuTree::flat(vec![
                    MenuItem::item("&Notepad"),
                    MenuItem::item("&Browser"),
                    MenuItem::Separator,
                    MenuItem::item("C&hoose Program..."),
                ]),
            );
        } else {
            tree.push(item.clone());
        }
    }
    let cx = MenuCtx {
        text,
        theme,
        px,
        within: c.bounds(),
    };
    let mut chain = PopupMenu::open(&tree, (370, 10), &cx, true);
    chain.key(&tree, MenuKey::Down, &cx);
    chain.key(&tree, MenuKey::Right, &cx);
    chain.draw(&mut c, theme, text, true);

    c.blit_canvas(
        370,
        160,
        &tooltip::draw_tip(theme, text, "Click here to begin"),
    );
    let balloon = tooltip::draw_balloon_tip(
        theme,
        text,
        "Turn Off",
        "Ends your session and shuts down the computer so that you can safely turn off power.",
    );
    c.blit_canvas(10, 200, &balloon);

    let cell = |name: &str, sel: Option<Selection>| {
        iconview::draw_cell(theme, text, name, None, true, sel, None, None)
    };
    c.blit_over(370, 260, &cell("Documents", None));
    c.blit_over(
        445,
        260,
        &cell(
            "New Text Document",
            Some(Selection {
                ring: true,
                origin: (445, 260),
            }),
        ),
    );

    c.fill_rect(Rect::new(540, 260, 44, 20), cs.button_face);
    c.blit_canvas(542, 262, &indicator::draw_record(16));
    c.blit_canvas(564, 262, &indicator::draw_microphone(16));
    {
        use reprisekit::art::powericon::{Source, draw};
        let b = |percent, charging, low, critical| Source::Battery {
            percent,
            charging,
            low,
            critical,
        };
        let icons = [
            Source::Ac,
            b(100, false, false, false),
            b(50, false, false, false),
            b(8, false, true, false),
            b(2, false, true, true),
            b(72, true, false, false),
        ];
        for (i, src) in icons.iter().enumerate() {
            c.blit_canvas(586 + 20 * i as i32, 262, &draw(*src));
        }
    }
    c.fill_rect(Rect::new(706, 260, 44, 20), cs.button_face);
    c.blit_canvas(708, 262, &indicator::draw_camera(16));
    c.blit_canvas(730, 262, &indicator::draw_location(16));

    let m = multiedit::Metrics {
        bar: theme.look.controls().scroll,
        thumb_min: theme.look.controls().thumb_min,
        ..Default::default()
    };
    let body = (0..12)
        .map(|i| format!("Line {i}: the quick brown fox jumps over the lazy dog"))
        .collect::<Vec<_>>()
        .join("\n");
    let edit = multiedit::MultiEdit::new(&body);
    let r = Rect::new(10, 400, 500, 90);
    let lay = multiedit::layout(mono, &m, &edit, r, s);
    let view = multiedit::View {
        top: 2,
        ..Default::default()
    };
    multiedit::draw(
        &mut c,
        mono,
        &Paint { s, ..theme.paint() },
        &m,
        &edit,
        view,
        &lay,
        Caret::Unfocused,
        None,
    );
    if let Some(corner) = lay.corner() {
        draw_scroll_corner(&mut c, &Paint { s, ..theme.paint() }, corner, true);
    }

    let page = Rect::new(530, 30, 360, 460);
    draw_page(&mut c, &Paint { s, ..theme.paint() }, page);
    let mut strip = tabstrip::TabStrip::new(&["Themes", "Desktop", "Screen Saver"], 1);
    strip.focused = true;
    let tabs = tabstrip::layout(
        text,
        px,
        &strip,
        page.x,
        page.y - 20,
        page.width,
        reprisekit::theme::TextUnits::ONE,
    );
    tabstrip::draw(
        &mut c,
        text,
        &Paint { s, ..theme.paint() },
        &strip,
        &tabs,
        page,
        true,
        px,
    );
    let mut open = combo::Combo::new(&["Fade effect", "Scroll effect"], 0, 8);
    open.toggle();
    let mut greyed = trackbar::TrackBar::new(13, 18);
    greyed.enabled = false;
    let items = vec![
        Item::new(
            "l",
            Rect::new(545, 45, 100, 13),
            Control::Label {
                text: "&Position:".into(),
                enabled: true,
                align: Align::Left,
            },
        ),
        Item::new(
            "combo",
            Rect::new(545, 60, 150, combo::COMBO_H),
            Control::Combo(combo::Combo::new(&["Center", "Tile", "Stretch"], 2, 8)),
        ),
        Item::new(
            "open",
            Rect::new(710, 60, 150, combo::COMBO_H),
            Control::Combo(open),
        ),
        Item::new(
            "list",
            Rect::new(545, 130, 200, 80),
            Control::ListBox {
                list: listbox::ListBox::new(
                    ["(None)", "Crystal", "Dunes", "Harbour", "Meadow", "Tide"]
                        .iter()
                        .map(|s| s.to_string())
                        .collect(),
                    Some(1),
                ),
                icons: Vec::new(),
            },
        ),
        Item::new(
            "checked_list",
            Rect::new(545, 310, 172, 55),
            Control::ListBox {
                list: listbox::ListBox {
                    checks: vec![true, false, true],
                    detail: vec![String::new(), String::new(), "not installed".into()],
                    ..listbox::ListBox::new(
                        ["Pass", "Notes", "org.example.Gone"]
                            .map(String::from)
                            .to_vec(),
                        None,
                    )
                },
                icons: vec![
                    Some(swatch16(Color::rgb(0, 128, 0))),
                    Some(swatch16(Color::rgb(0, 0, 160))),
                    None,
                ],
            },
        ),
        Item::new(
            "tree",
            Rect::new(545, 404, 172, 84),
            Control::Tree({
                let mut screens = treeview::Node::new("Screens:", true);
                screens.expanded = true;
                screens.children = vec![treeview::Node::new("HDMI-A-1 (1920x1080)", false)];
                let mut windows = treeview::Node::new("Windows:", true);
                windows.expanded = true;
                windows.children = vec![treeview::Node::new("Untitled - Notepad", false)];
                let mut tv =
                    treeview::TreeView::new(vec![screens, windows], treeview::Style::Plain);
                tv.selected = Some(vec![1, 0]);
                tv
            }),
        ),
        Item::new(
            "spin",
            Rect::new(760, 130, 50, 20),
            Control::UpDown(updown::UpDown::new(10, 1, 9999)),
        ),
        Item::new(
            "check",
            Rect::new(760, 160, 13, 13),
            Control::Check {
                label: "On &resume, password protect".into(),
                checked: true,
                enabled: true,
                mixed: None,
            },
        ),
        Item::new(
            "find",
            Rect::new(545, 100, 150, 20),
            Control::Edit {
                edit: TextEdit::new("Ln 1, Col 1", true),
                first: std::cell::Cell::new(0),
                enabled: true,
            },
        ),
        Item::new(
            "group",
            Rect::new(545, 230, 172, 64),
            Control::Group("Screen resolution".into()),
        ),
        Item::new(
            "track",
            Rect::new(590, 243, 71, trackbar::HEIGHT),
            Control::TrackBar(greyed),
        ),
        Item::new("rule", Rect::new(725, 243, 2, 16), Control::Rule),
        Item::new(
            "color",
            Rect::new(760, 230, 75, 21),
            Control::Color(colorbutton::ColorButton::new(
                cs.background,
                schemes::PALETTE.to_vec(),
            )),
        ),
        Item::radio("here", 370, 200, "&This folder only", "scope"),
        Item::radio("deep", 370, 223, "&Subfolders too", "scope").checked(true),
        Item::radio("none", 370, 246, "&Nowhere (disabled)", "scope").enabled(false),
        Item::new(
            "apply",
            Rect::new(800, 455, 75, 23),
            Control::Button {
                label: "&Apply".into(),
                enabled: false,
                default: false,
            },
        ),
    ];
    let mut form = form::Form::new(items);
    form.focus_id("combo");
    let drop_x = 545 + 3 + text.dialog(form.font).measure("Ln 1,", false, px);
    form.drag_over(drop_x, 110, text, px);
    form::draw(
        &mut c,
        text,
        &Paint { s, ..theme.paint() }.on_page(page),
        &form,
        true,
        px,
    );
    let sample = sample::draw_appearance(theme, text, None, sample::APPEARANCE_SIZE);
    let small = sample.resample_box(173, 92);
    c.blit_canvas(545, 310, &small);
    let monitor = form::Form::new(vec![
        Item::new("spectrum", Rect::new(745, 268, 154, 15), Control::Spectrum),
        Item::new(
            "mon",
            Rect::new(730, 290, 191, 168),
            Control::Monitor(form::Screen::Color(cs.background)),
        ),
    ]);
    form::draw(
        &mut c,
        text,
        &Paint { s, ..theme.paint() },
        &monitor,
        true,
        px,
    );

    let about = aboutbox::Fields {
        product: "Reprise".into(),
        version: "Version 0.1.0 (Build a1b2c3d : 6.9.0-generic)".into(),
        copyright: "".into(),
        facts: [
            "Example CPU @ 2.00GHz".into(),
            "Video:  amdgpu".into(),
            "Free space on /:  38.2 GB".into(),
        ],
        memory_label: "Physical memory available to Reprise:".into(),
        memory_value: "16,000,000 KB".into(),
        icon: None,
    };
    let about_form = aboutbox::form(&about, text, px);
    let (aw, ah) = aboutbox::CLIENT;
    let mut about_c = Canvas::new(aw as u32, ah as u32);
    about_c.fill_rect(about_c.bounds(), cs.button_face);
    form::draw(
        &mut about_c,
        text,
        &Paint { s, ..theme.paint() },
        &about_form,
        true,
        px,
    );
    c.blit_canvas(10, 510, &about_c);

    let places = placesbar::PlacesBar {
        places: [
            "My Recent Documents",
            "user's Home",
            "Desktop",
            "Documents",
            "Downloads",
        ]
        .iter()
        .map(|l| placesbar::Place {
            label: (*l).into(),
            icon: None,
            enabled: true,
            pinned: false,
        })
        .collect(),
        selected: Some(3),
        ..Default::default()
    };
    let places_r = Rect::new(700, 510, placesbar::WIDTH, 5 * placesbar::TILE_H);
    placesbar::draw(
        &mut c,
        text,
        &Paint { s, ..theme.paint() },
        &places,
        places_r,
        px,
    );
    // The same bar with two pinned folders, the second missing, scrolled to its end.
    let mut pinned = places.clone();
    for (label, enabled) in [("Projects", true), ("Stick", false)] {
        pinned.places.push(placesbar::Place {
            label: label.into(),
            enabled,
            pinned: true,
            ..Default::default()
        });
    }
    pinned.selected = Some(5);
    let pinned_r = Rect { x: 800, ..places_r };
    pinned.scroll(9, pinned_r, &theme.look.controls().grown(theme.text));
    placesbar::draw(
        &mut c,
        text,
        &Paint { s, ..theme.paint() },
        &pinned,
        pinned_r,
        px,
    );

    let find = finddlg::FindFields {
        query: "hello".into(),
        match_case: false,
        down: true,
    };
    let (fw, fh) = finddlg::FIND_CLIENT;
    let mut find_c = Canvas::new(fw as u32, fh as u32);
    find_c.fill_rect(find_c.bounds(), cs.button_face);
    let find_form = finddlg::find_form(&find);
    form::draw(
        &mut find_c,
        text,
        &Paint { s, ..theme.paint() },
        &find_form,
        true,
        px,
    );
    c.blit_canvas(10, 870, &find_c);

    let replace = finddlg::ReplaceFields {
        query: "hello".into(),
        with: "world".into(),
        match_case: false,
    };
    let (rw, rh) = finddlg::REPLACE_CLIENT;
    let mut replace_c = Canvas::new(rw as u32, rh as u32);
    replace_c.fill_rect(replace_c.bounds(), cs.button_face);
    let replace_form = finddlg::replace_form(&replace);
    form::draw(
        &mut replace_c,
        text,
        &Paint { s, ..theme.paint() },
        &replace_form,
        true,
        px,
    );
    c.blit_canvas(400, 870, &replace_c);

    let notice = msgbox::Fields {
        text: "Cannot find \"hello\"".into(),
        icon: None,
        buttons: vec!["OK".into()],
        default: 0,
        min_text_h: None,
        min_w: 0,
        max_w: 0,
    };
    let (mw, mh) = msgbox::client(&notice, text, px, theme.text);
    let mut msg_c = Canvas::new(mw as u32, mh as u32);
    msg_c.fill_rect(msg_c.bounds(), cs.button_face);
    let msg_form = msgbox::form(&notice, text, px, theme.text);
    form::draw(
        &mut msg_c,
        text,
        &Paint { s, ..theme.paint() },
        &msg_form,
        true,
        px,
    );
    c.blit_canvas(10, 990, &msg_c);

    {
        let buttons = ["&Yes".to_string(), "&No".to_string()];
        let lay = dlgtemplate::layout(
            dlgtemplate::Template::MessageBox,
            text,
            px,
            &dlgtemplate::Fields {
                text: "Are you sure you want to send 'New Rich Text Document' to the Recycle Bin?",
                icon: true,
                field: false,
                buttons: &buttons,
                text_h: None,
            },
            theme.text,
        )
        .client();
        let mut icon = Canvas::new(32, 32);
        icon.fill_rect(Rect::new(4, 2, 24, 28), Color::rgb(80, 193, 255));
        let f = dlgtemplate::form(&lay, Some(icon), &buttons, 0);
        let mut box_c = Canvas::new(lay.size.0 as u32, lay.size.1 as u32);
        box_c.fill_rect(box_c.bounds(), cs.button_face);
        form::draw(
            &mut box_c,
            text,
            &Paint { s, ..theme.paint() },
            &f,
            true,
            px,
        );
        c.blit_canvas(480, 3018, &box_c);
    }

    c
}

/// Toolbars, rebar, list view, file dialogs, address bar, task pane, tree and property sheets.
fn page_explorer(mut c: Canvas, theme: &Theme, text: &TextRenderer) -> Canvas {
    let cs = &theme.colors;
    let px = theme.fonts.size_px;
    let s = 1;

    let tb = toolbar::ToolBar {
        items: vec![
            toolbar::Item::DropButton {
                id: toolbar::BACK,
                icon: None,
                label: None,
                enabled: true,
            },
            toolbar::Item::Button {
                id: toolbar::UP,
                icon: None,
                label: None,
                enabled: true,
            },
            toolbar::Item::Button {
                id: toolbar::NEW_FOLDER,
                icon: None,
                label: None,
                enabled: false,
            },
            toolbar::Item::DropButton {
                id: toolbar::VIEWS,
                icon: Some(viewsicon::draw16()),
                label: None,
                enabled: true,
            },
        ],
        hot: Some(0),
        pressed: Some(1),
        latched: None,
        big: false,
    };
    let tb_r = Rect::new(10, 1100, 200, toolbar::HEIGHT);
    let cm = theme.look.controls();
    let tb_rects = toolbar::layout(&tb, text, px, tb_r, &cm);
    toolbar::draw(
        &mut c,
        text,
        &Paint { s, ..theme.paint() },
        &tb,
        tb_r,
        &tb_rects,
        px,
    );

    let rebar = rebar::Rebar {
        bands: vec![
            rebar::Band { height: 22 },
            rebar::Band {
                height: toolbar::height(true, &cm),
            },
            rebar::Band { height: 22 },
        ],
    };
    let rebar_r = Rect::new(240, 1095, 560, rebar::height(&rebar, &cm.rebar));
    rebar::draw(&mut c, &Paint { s, ..theme.paint() }, &rebar, rebar_r);
    let bands = rebar::layout(&rebar, rebar_r, &cm.rebar);
    let big = toolbar::ToolBar {
        items: vec![
            toolbar::Item::DropButton {
                id: toolbar::BACK,
                icon: None,
                label: Some("Back".into()),
                enabled: true,
            },
            toolbar::Item::DropButton {
                id: toolbar::FORWARD,
                icon: None,
                // Empty, not absent: Forward reserves a label's trailing pad, so it is 46 px, not 39.
                label: Some(String::new()),
                enabled: false,
            },
            toolbar::Item::Button {
                id: toolbar::UP,
                icon: None,
                label: None,
                enabled: true,
            },
            toolbar::Item::Separator,
            toolbar::Item::Button {
                id: toolbar::FOLDERS,
                icon: None,
                label: Some("Folders".into()),
                enabled: true,
            },
            toolbar::Item::Separator,
            toolbar::Item::DropButton {
                id: toolbar::VIEWS,
                icon: Some(viewsicon::draw24()),
                label: None,
                enabled: true,
            },
        ],
        hot: None,
        pressed: None,
        latched: Some(4),
        big: true,
    };
    let big_rects = toolbar::layout(&big, text, px, bands[1], &cm);
    toolbar::draw(
        &mut c,
        text,
        &Paint { s, ..theme.paint() },
        &big,
        bands[1],
        &big_rects,
        px,
    );

    for (i, (mode, grouped)) in [
        (shellview::Mode::List, false),
        (shellview::Mode::Details, false),
        (shellview::Mode::Icons, false),
        (shellview::Mode::Tiles, false),
        (shellview::Mode::Thumbnails, false),
        (shellview::Mode::Tiles, true),
    ]
    .iter()
    .enumerate()
    {
        let sv = shellview::sample(*mode, *grouped);
        let col = (i % 2) as i32;
        let row = (i / 2) as i32;
        let sv_r = Rect::new(10 + col * 440, 1650 + row * 180, 420, 170);
        let sv_lay = shellview::layout(&sv, sv_r, &theme.look.controls());
        shellview::draw(
            &mut c,
            text,
            &Paint { s, ..theme.paint() },
            &sv,
            &sv_lay,
            None,
            px,
        );
    }

    let lv = listview::sample();
    let lv_r = Rect::new(570, 1240, 320, 130);
    let lv_lay = listview::layout(&lv, lv_r, &theme.look.controls());
    listview::draw(
        &mut c,
        text,
        &Paint { s, ..theme.paint() },
        &lv,
        &lv_lay,
        None,
        px,
    );

    let mut filedlg = filedlg::FileDlg::new(
        filedlg::Fields {
            title: "Open".into(),
            accept_label: "Open".into(),
            mode: filedlg::DlgMode::Open,
            multiple: false,
            filters: vec!["Text Documents (*.txt)".into(), "All Files (*.*)".into()],
        },
        [
            "My Recent Documents",
            "user's Home",
            "Desktop",
            "Documents",
            "Downloads",
            "Computer",
        ]
        .iter()
        .map(|l| placesbar::Place {
            label: (*l).into(),
            icon: None,
            enabled: true,
            pinned: false,
        })
        .collect(),
    );
    filedlg.set_listing(filedlg::Folder {
        id: 1,
        parent: None,
        crumbs: vec![filedlg::Crumb {
            id: 1,
            label: "Documents".into(),
            depth: 0,
            icon: None,
        }],
        entries: vec![
            shellview::Entry {
                id: 2,
                name: "Pictures".into(),
                is_dir: true,
                kind: "File Folder".into(),
                ..Default::default()
            },
            shellview::Entry {
                id: 3,
                name: "Music".into(),
                is_dir: true,
                kind: "File Folder".into(),
                ..Default::default()
            },
            shellview::Entry {
                id: 4,
                name: "readme.txt".into(),
                size: 1_024,
                kind: "Text Document".into(),
                ..Default::default()
            },
        ],
    });
    filedlg.set_file_name("readme.txt");
    let (dw, dh) = filedlg::CLIENT;
    let mut filedlg_c = Canvas::new(dw as u32, dh as u32);
    filedlg_c.fill_rect(filedlg_c.bounds(), cs.button_face);
    filedlg.draw(
        &mut filedlg_c,
        text,
        &Paint { s, ..theme.paint() },
        Rect::new(0, 0, dw, dh),
        px,
    );
    c.blit_canvas(10, 1185, &filedlg_c);

    let dialog_in = |mode: filedlg::DlgMode, accept: &str, multiple: bool, folders_only: bool| {
        let mut d = filedlg::FileDlg::new(
            filedlg::Fields {
                title: "Open".into(),
                accept_label: accept.into(),
                filters: if folders_only {
                    vec![]
                } else {
                    vec!["All Files".into()]
                },
                mode,
                multiple,
            },
            vec![],
        );
        let mut entries = vec![shellview::Entry {
            id: 2,
            name: "Pictures".into(),
            is_dir: true,
            kind: "File Folder".into(),
            ..Default::default()
        }];
        if !folders_only {
            for (id, name) in [(3, "a.txt"), (4, "b.txt"), (5, "c.txt")] {
                entries.push(shellview::Entry {
                    id,
                    name: name.into(),
                    kind: "Text Document".into(),
                    ..Default::default()
                });
            }
        }
        d.set_listing(filedlg::Folder {
            id: 1,
            parent: None,
            crumbs: vec![filedlg::Crumb {
                id: 1,
                label: "Documents".into(),
                depth: 0,
                icon: None,
            }],
            entries,
        });
        d
    };
    let draw_dialog = |c: &mut Canvas, d: &filedlg::FileDlg, y: i32| {
        let (w, h) = d.client();
        let mut dc = Canvas::new(w as u32, h as u32);
        dc.fill_rect(dc.bounds(), cs.button_face);
        d.draw(
            &mut dc,
            text,
            &Paint { s, ..theme.paint() },
            Rect::new(0, 0, w, h),
            px,
        );
        c.blit_canvas(10, y, &dc);
    };
    let mut multi = dialog_in(filedlg::DlgMode::Open, "Open", true, false);
    let r = Rect::new(0, 0, filedlg::CLIENT.0, filedlg::CLIENT.1);
    let ctrl = reprisekit::selection::Mods {
        ctrl: true,
        shift: false,
    };
    for (i, mods) in [(1, Default::default()), (3, ctrl)] {
        multi.click(r, Some(filedlg::Hit::Shell(shellview::Hit::Entry(i))), mods);
    }
    // A Ctrl marquee from the list's background up over b.txt, mid-drag.
    let find = |d: &filedlg::FileDlg, want: shellview::Hit| {
        (0..r.height).rev().step_by(2).find_map(|y| {
            (0..r.width).rev().step_by(4).find_map(|x| {
                (d.hit(text, px, r, x, y) == Some(filedlg::Hit::Shell(want))).then_some((x, y))
            })
        })
    };
    if let (Some(from), Some(to)) = (
        find(&multi, shellview::Hit::Background),
        find(&multi, shellview::Hit::Entry(2)),
    ) {
        let background = Some(filedlg::Hit::Shell(shellview::Hit::Background));
        multi.click_at(r, background, ctrl, from);
        multi.motion(text, px, r, to.0, to.1);
    }
    draw_dialog(&mut c, &multi, 7140);
    let folder_dlg = dialog_in(filedlg::DlgMode::Folder, "Select Folder", false, true);
    draw_dialog(&mut c, &folder_dlg, 7600);
    let mut choices = dialog_in(filedlg::DlgMode::Open, "Open", false, false);
    let w = filedlg::field_w(filedlg::CLIENT.0);
    choices.set_extra(
        form::Form::new(vec![
            form::Item::check(
                "extra-check",
                filedlg::FIELD_X,
                filedlg::EXTRA_Y,
                "Open read-only",
            ),
            form::Item::combo(
                "extra-combo",
                filedlg::FIELD_X,
                filedlg::EXTRA_Y + 20,
                w,
                &["UTF-8", "Latin-1"],
                2,
            ),
        ]),
        70,
        &theme.look.controls(),
    );
    draw_dialog(&mut c, &choices, 8060);

    {
        let line = |key: &str, volume, balance, check, checked| mixer::Line {
            key: key.into(),
            title: key.into(),
            volume,
            balance,
            check,
            checked,
            live: true,
            boost: false,
            output: None,
        };
        // The playback view with volumes past 100 %: Firefox at 120 %, each program with its
        // output combo, mpv on the headphones.
        let routed = |picked| {
            Some(mixer::Route {
                outputs: vec!["Speakers".into(), "Headphones".into()],
                picked,
            })
        };
        let boosted = |l: mixer::Line| mixer::Line { boost: true, ..l };
        let playback = [
            boosted(line(
                "Volume Control",
                42,
                Some(10),
                mixer::Check::MuteAll,
                false,
            )),
            mixer::Line {
                output: routed(None),
                ..boosted(line("Firefox", 72, Some(6), mixer::Check::Mute, false))
            },
            mixer::Line {
                output: routed(Some(1)),
                ..boosted(line("mpv", 24, None, mixer::Check::Mute, true))
            },
        ];
        let mut speaker = Canvas::new(16, 16);
        speaker.fill_rect(Rect::new(4, 4, 6, 8), Color::rgb(64, 64, 64));
        speaker.fill_rect(Rect::new(10, 2, 4, 12), Color::rgb(128, 128, 128));
        let mut m = mixer::Mixer::new(&playback, true);
        m.focus = mixer::Focus::Balance(1);
        m.cues = true;
        m.speaker = Some(speaker.clone());
        let (w, h) = m.size(theme.text);
        let body = Rect::new(600, 7150, w, h);
        let lay = mixer::layout(&m, theme, text, body);
        mixer::draw(&mut c, theme, text, &m, &lay);
        let below = body.bottom() + 20;
        let mut recording = [
            line("Microphone", 36, Some(10), mixer::Check::Select, false),
            line("Line In", 18, Some(14), mixer::Check::Select, true),
            line("Monitor", 0, None, mixer::Check::Select, false),
        ];
        recording[2].live = false;
        let mut m = mixer::Mixer::new(&recording, false);
        m.speaker = Some(speaker);
        let (w, h) = m.size(theme.text);
        let body = Rect::new(600, below, w, h);
        let lay = mixer::layout(&m, theme, text, body);
        mixer::draw(&mut c, theme, text, &m, &lay);
    }

    let swatch = |w: u32, h: u32, colour: Color| {
        let mut i = Canvas::new(w, h);
        i.fill_rect(i.bounds(), colour);
        i
    };
    let folder = Color::rgb(255, 255, 153);
    let places: Vec<combo::ListItem> = [
        ("Desktop", 0),
        ("Documents", 1),
        ("Computer", 1),
        ("Local Disk (/)", 2),
        ("Control Panel", 2),
        ("user's Home", 1),
        ("Recycle Bin", 1),
    ]
    .into_iter()
    .map(|(t, d)| combo::ListItem::shell(t, Some(swatch(16, 16, folder)), d))
    .collect();
    let go = Some(swatch(20, 20, Color::rgb(0, 128, 0)));
    let ab = addressbar::AddressBar::new(places, 1, go);
    for (i, open) in [false, true].into_iter().enumerate() {
        let band = Rect::new(0, 2200 + i as i32 * 32, 900, 22);
        c.fill_rect(
            Rect::new(0, band.y - 2, 900, band.height + 2),
            cs.button_face,
        );
        c.etched_hline_t(0, band.right(), band.y - 2, cs, s);
        let mut ab = ab.clone();
        if open {
            ab.combo.toggle();
        }
        let lay = addressbar::layout(&ab, text, px, band, &theme.look.controls());
        addressbar::draw(
            &mut c,
            text,
            &Paint { s, ..theme.paint() },
            &ab,
            &lay,
            false,
            Caret::Unfocused,
            px,
        );
        addressbar::draw_list(&mut c, text, &Paint { s, ..theme.paint() }, &ab, &lay, px);
    }

    let strip = Rect::new(0, 2264, 900, 430);
    c.fill_rect(strip, cs.button_face);
    for (i, (title, hot, greyed)) in [
        ("Folders", false, false),
        ("Folders", true, false),
        ("Search", false, true),
    ]
    .into_iter()
    .enumerate()
    {
        let eb = explorerbar::ExplorerBar {
            title: title.into(),
            width: 200,
            hot_close: hot,
            greyed,
        };
        let r = Rect::new(10, strip.y + 16 + i as i32 * 40, 204, 30);
        c.fill_rect(Rect::new(r.x, r.y + 22, 200, 8), cs.window);
        let lay = explorerbar::layout(&eb, r, &theme.look.controls());
        explorerbar::draw(&mut c, text, &Paint { s, ..theme.paint() }, &eb, &lay, px);
    }
    let icon = |colour: Color| {
        let mut i = Canvas::new(16, 16);
        i.fill_rect(i.bounds(), colour);
        Some(i)
    };
    let tp = taskpane::TaskPane {
        sections: vec![
            taskpane::Section::links(
                "File and Folder Tasks",
                vec![
                    taskpane::Row::new("Make a new folder", icon(Color::rgb(255, 255, 153))),
                    taskpane::Row::new(
                        "Publish this folder to the Web",
                        icon(Color::rgb(0, 128, 0)),
                    ),
                    taskpane::Row::new("Share this folder", icon(Color::rgb(255, 204, 0))),
                ],
            ),
            taskpane::Section::lines(
                "Details",
                vec![("Documents".into(), true), ("System Folder".into(), false)],
            ),
            taskpane::Section {
                title: "Collapsed Tasks".into(),
                open: false,
                body: taskpane::Body::Links(vec![taskpane::Row::new("Nothing to see", None)]),
            },
        ],
        top: 0,
        hot: Some((0, 0)),
        hot_header: None,
    };
    let pane = Rect::new(240, strip.y + 10, 200, 300);
    let lay = taskpane::layout(&tp, text, px, pane, &theme.look.controls());
    taskpane::draw(&mut c, text, &Paint { s, ..theme.paint() }, &tp, &lay, px);
    let tip = infotip::draw(
        theme,
        text,
        &[
            "Dimensions: 800 x 600".into(),
            "Type: JPEG Image".into(),
            "Size: 27.8 KB".into(),
        ],
        1,
    );
    c.blit_canvas(470, strip.y + 40, &tip);

    let folder = swatch16(Color::rgb(255, 224, 123));
    let open_folder = swatch16(Color::rgb(174, 218, 255));
    let child = |label: &str| Node {
        icon: Some(folder.clone()),
        open_icon: Some(open_folder.clone()),
        ..Node::new(label, true)
    };
    let mut docs = child("Documents");
    docs.children = vec![child("Music"), child("Pictures")];
    docs.expanded = true;
    let mut desktop = child("Desktop");
    desktop.children = vec![
        docs,
        child("Computer with a name long enough to overflow"),
        child("user's Home"),
    ];
    desktop.expanded = true;
    {
        let mut tv = TreeView::new(vec![desktop.clone()], treeview::Style::Explorer);
        tv.selected = Some(vec![0, 0]);
        let eb = explorerbar::ExplorerBar::new("Folders");
        let bar_lay = explorerbar::layout(
            &eb,
            Rect::new(620, strip.y + 10, 204, 130),
            &theme.look.controls(),
        );
        explorerbar::draw(
            &mut c,
            text,
            &Paint { s, ..theme.paint() },
            &eb,
            &bar_lay,
            px,
        );
        let b = bar_lay.body;
        let tree_r = Rect::new(b.x - 1, b.y, b.width + 1, b.height);
        let tree_lay = treeview::layout(&tv, text, px, tree_r, &theme.look.controls());
        treeview::draw(
            &mut c,
            text,
            &Paint { s, ..theme.paint() },
            &tv,
            &tree_lay,
            None,
            px,
        );
    }
    let mut tv = TreeView::new(vec![desktop], treeview::Style::Classic);
    tv.selected = Some(vec![0, 0]);
    tv.focus_ring = true;
    tv.drop_hilight = Some(4);
    tv.left = 6;
    let tree_r = Rect::new(620, strip.y + 150, 240, 110);
    let tree_lay = treeview::layout(&tv, text, px, tree_r, &theme.look.controls());
    treeview::draw(
        &mut c,
        text,
        &Paint { s, ..theme.paint() },
        &tv,
        &tree_lay,
        None,
        px,
    );

    let sheet_page = || {
        form::Form::new(vec![
            Item::new(
                "type",
                Rect::new(17, 49, 120, 13),
                Control::Label {
                    text: "Type:".into(),
                    enabled: true,
                    align: Align::Left,
                },
            ),
            Item::new(
                "ro",
                Rect::new(17, 71, 13, 13),
                Control::Check {
                    label: "&Read-only".into(),
                    checked: true,
                    enabled: true,
                    mixed: None,
                },
            ),
            Item::new(
                "adv",
                Rect::new(247, 67, BUTTON_W, BUTTON_H),
                Control::Button {
                    label: "Ad&vanced...".into(),
                    enabled: true,
                    default: false,
                },
            ),
        ])
    };
    let mut sheet = PropSheet::new(
        &["General", "Sharing", "Customize"],
        vec![sheet_page(), sheet_page(), sheet_page()],
        (361, 424),
    );
    sheet.set_apply_enabled(true);
    sheet.set_restore(0, true);
    let (sw, sh) = sheet.size;
    let mut sheet_c = Canvas::new(sw as u32, sh as u32);
    sheet_c.fill_rect(sheet_c.bounds(), cs.button_face);
    propsheet::draw(
        &mut sheet_c,
        text,
        &Paint { s, ..theme.paint() },
        &sheet,
        true,
        px,
    );
    c.blit_canvas(20, 2570, &sheet_c);

    let seven = [
        "General",
        "Sharing",
        "Customize",
        "Security",
        "Previous Versions",
        "Summary",
        "Compatibility",
    ];
    let mut wide = PropSheet::new(
        &seven,
        seven.iter().map(|_| sheet_page()).collect(),
        (361, 424),
    );
    wide.tabs.open = 1;
    wide.fit_tabs(text, px);
    let (sw, sh) = wide.size;
    let mut wide_c = Canvas::new(sw as u32, sh as u32);
    wide_c.fill_rect(wide_c.bounds(), cs.button_face);
    propsheet::draw(
        &mut wide_c,
        text,
        &Paint { s, ..theme.paint() },
        &wide,
        true,
        px,
    );
    c.blit_canvas(420, 2570, &wide_c);

    c
}
