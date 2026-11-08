use super::{blurred_stripes, gallery_p3};
use reprisekit::art::viewsicon;
use reprisekit::classic::frame::{CaptionButton, Chrome, FrameGeometry, FrameState, Glow};
use reprisekit::controls::form::{self, Control, Item};
use reprisekit::controls::menu::{MenuCtx, MenuItem, MenuKey, MenuTree, PopupMenu};
use reprisekit::controls::statusbar::{self, Panel, StatusBar};
use reprisekit::controls::{combo, listview, rebar, toolbar};
use reprisekit::look;
use reprisekit::look::widgets::{
    draw_hscrollbar, draw_marquee, draw_progress, draw_scroll_corner, draw_scrollbar,
    draw_size_grip, draw_trackbar, draw_vertical_trackbar,
};
use reprisekit::pointer::Interaction;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::{Align, TextRenderer};
use reprisekit::textedit::Caret;
use reprisekit::theme::{Color, FontConfig, Theme};
use reprisekit::trackbar;

/// Glass's scrollbars on its face, 950x170: upright and lying bars in each state, then two bars meeting at their corner.
pub fn render_glass_scrollbars(theme: &Theme) -> Canvas {
    // Classic helpers with no look::widgets twin.
    use reprisekit::classic::widgets::ScrollHit;
    let p = theme.paint();
    let mut c = Canvas::new(950, 170);
    c.fill_rect(c.bounds(), theme.colors.button_face);
    let (hot, held) = (Interaction::Hot, Interaction::Pressed);
    let upright = [
        None,
        Some((ScrollHit::Back, hot)),
        Some((ScrollHit::Back, held)),
        Some((ScrollHit::Forward, hot)),
        Some((ScrollHit::Forward, held)),
        Some((ScrollHit::Thumb, hot)),
        Some((ScrollHit::Thumb, held)),
        Some((ScrollHit::PageForward, held)),
        Some((ScrollHit::PageForward, hot)),
    ];
    for (i, lit) in upright.into_iter().enumerate() {
        let r = Rect::new(10 + 30 * i as i32, 10, 17, 150);
        draw_scrollbar(&mut c, &p, r, 2, 6, 20, lit);
    }
    draw_scrollbar(&mut c, &p, Rect::new(280, 10, 17, 150), 0, 20, 20, None);
    let lying = [
        None,
        Some((ScrollHit::Back, hot)),
        Some((ScrollHit::Forward, held)),
        Some((ScrollHit::Thumb, hot)),
        Some((ScrollHit::PageBack, held)),
    ];
    for (i, lit) in lying.into_iter().enumerate() {
        let r = Rect::new(320, 10 + 25 * i as i32, 300, 17);
        draw_hscrollbar(&mut c, &p, r, 8, 6, 20, lit);
    }
    draw_hscrollbar(&mut c, &p, Rect::new(320, 135, 300, 17), 0, 20, 20, None);
    draw_scrollbar(&mut c, &p, Rect::new(850, 10, 17, 120), 0, 4, 20, None);
    draw_hscrollbar(&mut c, &p, Rect::new(660, 130, 190, 17), 0, 4, 20, None);
    draw_scroll_corner(&mut c, &p, Rect::new(850, 130, 17, 17), true);
    c
}

/// Glass's picture wells on the face, 950x210: full and thin wells, the colour spectrum, the appearance sample.
pub fn render_glass_wells(theme: &Theme) -> Canvas {
    let text = TextRenderer::load(&FontConfig::default());
    let mut c = Canvas::new(950, 210);
    c.fill_rect(c.bounds(), theme.colors.button_face);
    let picture = |w: u32, h: u32| {
        let mut pic = Canvas::new(w, h);
        pic.fill_rect(pic.bounds(), Color::rgb(0xFF, 0xFF, 0xFF));
        pic.fill_rect(Rect::new(8, 8, 16, 16), Color::rgb(40, 120, 220));
        Some(pic)
    };
    let form = form::Form::new(vec![
        Item::new(
            "full",
            Rect::new(10, 10, 160, 120),
            Control::Picture {
                canvas: picture(156, 116),
                frame: 2,
            },
        ),
        Item::new(
            "thin",
            Rect::new(190, 10, 160, 120),
            Control::Picture {
                canvas: picture(158, 118),
                frame: 1,
            },
        ),
        Item::new("spectrum", Rect::new(370, 10, 154, 15), Control::Spectrum),
        Item::new(
            "sample",
            Rect::new(540, 10, 350, 188),
            Control::Sample {
                themes: false,
                theme: theme.clone(),
                icon: None,
            },
        ),
    ]);
    form::draw(
        &mut c,
        &text,
        &theme.paint(),
        &form,
        false,
        theme.fonts.size_px,
    );
    c
}

/// Glass's list headers, list frames and status bars on the face, 950x250.
pub fn render_glass_lists(theme: &Theme) -> Canvas {
    let text = TextRenderer::load(&FontConfig::default());
    let p = theme.paint();
    let cm = theme.look.controls();
    let mut c = Canvas::new(950, 250);
    c.fill_rect(c.bounds(), theme.colors.button_face);
    for (i, (sort, hot, held)) in [
        ((0, true), Some(1), None),
        ((0, false), Some(0), Some(2)),
        ((1, true), None, Some(0)),
    ]
    .into_iter()
    .enumerate()
    {
        let mut lv = listview::sample();
        lv.sort = Some(sort);
        lv.edit = None;
        lv.header.motion(hot);
        if held.is_some() {
            lv.header.press(held, false);
        }
        let r = Rect::new(10 + 310 * i as i32, 10, 300, 130);
        let lay = listview::layout(&lv, r, &cm);
        listview::draw(&mut c, &text, &p, &lv, &lay, None, theme.fonts.size_px);
    }
    let mut icon = Canvas::new(statusbar::ICON_PX as u32, statusbar::ICON_PX as u32);
    icon.fill_rect(Rect::new(2, 2, 12, 12), Color::rgb(80, 193, 255));
    let h = statusbar::height(&theme.controls());
    for (i, grip) in [true, false].into_iter().enumerate() {
        let sb = StatusBar {
            panels: vec![
                Panel {
                    text: "Processes: 37".into(),
                    width: 0,
                    pad: statusbar::EXPLORER_TEXT_PAD,
                    icon: None,
                },
                Panel {
                    text: "CPU Usage: 0%".into(),
                    width: 130,
                    pad: statusbar::EXPLORER_TEXT_PAD,
                    icon: None,
                },
                Panel {
                    text: "Computer".into(),
                    width: 160,
                    pad: statusbar::EXPLORER_TEXT_PAD,
                    icon: Some(icon.clone()),
                },
            ],
            grip,
        };
        let bar = Rect::new(10, 160 + i as i32 * (h + 10), 600, h);
        statusbar::draw(&mut c, &text, &p, &sb, bar, theme.fonts.size_px);
    }
    draw_size_grip(&mut c, &p, Rect::new(640, 160, 17, 17));
    c
}

/// Glass's selection box, 950x110: a marquee over black, white and grey patches, and over stripes as a snap outline.
pub fn render_glass_marquees(theme: &Theme) -> Canvas {
    let mut c = Canvas::new(950, 110);
    c.fill_rect(c.bounds(), Color::rgb(0xFF, 0xFF, 0xFF));
    let p = theme.paint();
    let backs = [
        Color::rgb(0, 0, 0),
        Color::rgb(0xFF, 0xFF, 0xFF),
        Color::rgb(0x80, 0x80, 0x80),
    ];
    for (i, back) in backs.into_iter().enumerate() {
        let x = 10 + 200 * i as i32;
        c.fill_rect(Rect::new(x, 10, 180, 90), back);
        draw_marquee(
            &mut c,
            &p,
            Rect::new(x + 30, 25, 170, 60),
            Color::rgb(0, 0, 0),
        );
    }
    for i in 0..10 {
        let shade = if i % 2 == 0 { 0x30 } else { 0xD0 };
        c.fill_rect(
            Rect::new(620 + 32 * i, 10, 32, 90),
            Color::rgb(shade, shade, shade),
        );
    }
    draw_marquee(&mut c, &p, Rect::new(640, 20, 280, 70), Color::rgb(0, 0, 0));
    c
}

/// Glass's toolbars and rebar, 950x150: three Explorer strips, the file dialog's small strip and a three-band rebar.
pub fn render_glass_toolbars(theme: &Theme) -> Canvas {
    use toolbar::{Item, ToolBar};
    let text = TextRenderer::load(&FontConfig::default());
    let px = theme.fonts.size_px;
    let p = theme.paint();
    let cm = theme.look.controls();
    let mut c = Canvas::new(950, 150);
    c.fill_rect(c.bounds(), theme.colors.button_face);
    let big = |hot: usize, pressed: Option<usize>, latched: Option<usize>, history: bool| {
        let button = |id, label: Option<&str>| Item::Button {
            id,
            icon: None,
            label: label.map(str::to_string),
            enabled: true,
        };
        ToolBar {
            items: vec![
                Item::DropButton {
                    id: toolbar::BACK,
                    icon: None,
                    label: Some("Back".into()),
                    enabled: history,
                },
                Item::DropButton {
                    id: toolbar::FORWARD,
                    icon: None,
                    label: Some(String::new()),
                    enabled: history,
                },
                button(toolbar::UP, None),
                Item::Separator,
                button(toolbar::FOLDERS, Some("Folders")),
                Item::Separator,
                Item::DropButton {
                    id: toolbar::VIEWS,
                    icon: Some(viewsicon::draw24()),
                    label: None,
                    enabled: true,
                },
            ],
            hot: Some(hot),
            pressed,
            latched,
            big: true,
        }
    };
    let one_band = rebar::Rebar {
        bands: vec![rebar::Band {
            height: toolbar::height(true, &cm),
        }],
    };
    let band_h = rebar::height(&one_band, &cm.rebar);
    let strips = [
        big(0, None, Some(4), true),
        big(4, Some(1), Some(4), true),
        big(6, Some(2), None, false),
    ];
    for (i, tb) in strips.iter().enumerate() {
        let rr = Rect::new(10, 10 + i as i32 * (band_h + 12), 420, band_h);
        rebar::draw(&mut c, &p, &one_band, rr);
        let band = rebar::layout(&one_band, rr, &cm.rebar)[0];
        let rects = toolbar::layout(tb, &text, px, band, &cm);
        toolbar::draw(&mut c, &text, &p, tb, band, &rects, px);
    }
    let small = ToolBar {
        items: vec![
            Item::DropButton {
                id: toolbar::BACK,
                icon: None,
                label: None,
                enabled: true,
            },
            Item::Button {
                id: toolbar::UP,
                icon: None,
                label: None,
                enabled: true,
            },
            Item::Separator,
            Item::Button {
                id: toolbar::NEW_FOLDER,
                icon: None,
                label: None,
                enabled: false,
            },
            Item::DropButton {
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
    let strip = Rect::new(460, 14, 200, toolbar::HEIGHT);
    let rects = toolbar::layout(&small, &text, px, strip, &cm);
    toolbar::draw(&mut c, &text, &p, &small, strip, &rects, px);
    let stack = rebar::Rebar {
        bands: [22, toolbar::height(true, &cm), 22]
            .map(|height| rebar::Band { height })
            .into(),
    };
    let sr = Rect::new(460, 60, 480, rebar::height(&stack, &cm.rebar));
    rebar::draw(&mut c, &p, &stack, sr);
    if let Some(st) = theme.look.style() {
        for band in rebar::layout(&stack, sr, &cm.rebar) {
            let grip = Rect::new(band.x + 3, band.y, 3, band.height);
            st.draw(
                &mut c,
                grip,
                reprisekit::style::Part::RebarGripper,
                reprisekit::style::State::Normal,
                1,
            );
        }
    }
    c
}

/// Glass's progress bars, trackbars and a dialog's ? box, 950x430, in each state.
pub fn render_glass_ranges(theme: &Theme) -> Canvas {
    let text = TextRenderer::load(&FontConfig::default());
    let p = theme.paint();
    let mut c = Canvas::new(950, 430);
    c.fill_rect(Rect::new(0, 0, 950, 110), theme.colors.button_face);
    c.fill_rect(Rect::new(0, 200, 950, 230), theme.colors.button_face);
    let bars = [
        (0.0, true, 18),
        (0.05, true, 18),
        (0.25, true, 18),
        (0.5, true, 18),
    ];
    let more = [(1.0, true, 18), (0.5, false, 18), (0.5, true, 16)];
    for (i, (f, enabled, h)) in bars.into_iter().chain(more).enumerate() {
        let r = Rect::new(10 + 133 * i as i32, 10, 120, h);
        draw_progress(&mut c, &p, r, f, enabled);
    }
    let (idle, hot, held) = (Interaction::Idle, Interaction::Hot, Interaction::Pressed);
    for (i, (enabled, focused, ptr)) in [
        (true, false, idle),
        (true, false, hot),
        (true, false, held),
        (true, true, idle),
        (false, false, idle),
    ]
    .into_iter()
    .enumerate()
    {
        let lay = trackbar::layout(
            &trackbar::TrackBar::new(4, 11),
            10 + 180 * i as i32,
            50,
            160,
        );
        draw_trackbar(&mut c, &p, &lay, enabled, focused, ptr);
    }
    let stripes = blurred_stripes(950, 90);
    c.blit_canvas(0, 110, &stripes);
    use CaptionButton::Close;
    for (i, (active, help, hot, pressed)) in [
        (true, idle, None, None),
        (true, hot, None, None),
        (true, held, None, None),
        (true, idle, Some(Close), None),
        (true, idle, None, Some(Close)),
        (false, idle, None, None),
        (false, hot, None, None),
        (false, idle, Some(Close), None),
    ]
    .into_iter()
    .enumerate()
    {
        let g = FrameGeometry::layout(&theme.metrics, 100, 2, false, Chrome::DIALOG);
        let mut f = Canvas::new(g.total.0 as u32, g.total.1 as u32);
        let st = FrameState {
            title: "Run",
            active,
            maximized: false,
            pressed,
            glow: Glow::lit(hot),
            icon: None,
            tabbed: false,
        };
        look::draw_dialog_frame(&mut f, theme, &text, &g, &st, Some(help));
        let (x, y) = (10 + 117 * i as i32, 125);
        c.blit_canvas(x, y, &f);
    }
    for (i, (enabled, focused, ptr)) in [
        (true, false, idle),
        (true, false, hot),
        (true, false, held),
        (true, true, idle),
        (false, false, idle),
    ]
    .into_iter()
    .enumerate()
    {
        let lay = trackbar::layout_vertical(
            &trackbar::TrackBar::new(14, 21).paged(4, 2),
            10 + 60 * i as i32,
            215,
            81,
        );
        draw_vertical_trackbar(&mut c, &p, &lay, enabled, focused, ptr);
    }
    c
}

/// Glass's menus on white: the menu bars and context menu of `gallery_p3::menu`, a desktop menu with its View submenu open, and the Start panel's Shut down options with Restart lit.
pub fn render_glass_menus(theme: &Theme) -> Canvas {
    let text = TextRenderer::load(&FontConfig::default());
    let mut c = Canvas::new(950, 290);
    c.fill_rect(c.bounds(), Color::rgb(0xFF, 0xFF, 0xFF));
    gallery_p3::menu::draw(&mut c, theme, &text, Rect::new(0, 0, 520, 250));
    let mut icon = Canvas::new(16, 16);
    icon.fill_rect(Rect::new(2, 2, 12, 12), Color::rgb(40, 120, 220));
    let mut view = MenuTree::new();
    view.push(MenuItem::item("&Large icons").radio(true));
    view.push(MenuItem::item("&Medium icons").radio(true).checked(true));
    view.push(MenuItem::item("&Small icons").radio(true));
    view.push(MenuItem::Separator);
    view.push(MenuItem::item("&Auto arrange icons"));
    view.push(MenuItem::item("A&lign icons to grid").checked(true));
    view.push(MenuItem::Separator);
    view.push(MenuItem::item("&Show desktop icons").checked(true));
    let mut desk = MenuTree::new();
    desk.push_sub(MenuItem::item("&View").submenu(true), view);
    desk.push_sub(MenuItem::item("S&ort by").submenu(true), MenuTree::new());
    desk.push(MenuItem::item("R&efresh"));
    desk.push(MenuItem::Separator);
    desk.push(MenuItem::item("&Paste").enabled(false));
    desk.push(MenuItem::item("Paste &shortcut").enabled(false));
    desk.push(MenuItem::item("&Undo Delete").shortcut(Some("Ctrl+Z".into())));
    desk.push(MenuItem::Separator);
    desk.push(MenuItem::item("Screen &resolution").icon(Some(icon.clone())));
    desk.push(MenuItem::item("Personali&ze").icon(Some(icon)));
    let cx = MenuCtx {
        text: &text,
        theme,
        px: theme.fonts.size_px,
        within: c.bounds(),
    };
    let mut chain = PopupMenu::open(&desk, (540, 10), &cx, true);
    chain.key(&desk, MenuKey::Right, &cx);
    for _ in 0..5 {
        chain.key(&desk, MenuKey::Down, &cx);
    }
    chain.draw(&mut c, theme, &text, true);

    let options = theme.for_options_menu();
    let mut shut = MenuTree::new();
    for item in [
        MenuItem::item("&Log off"),
        MenuItem::item("L&ock"),
        MenuItem::Separator,
        MenuItem::item("&Restart"),
        MenuItem::Separator,
        MenuItem::item("&Sleep"),
        MenuItem::item("&Hibernate"),
    ] {
        shut.push(item);
    }
    let cx = MenuCtx {
        theme: &options,
        ..cx
    };
    let mut chain = PopupMenu::open(&shut, (410, 10), &cx, false);
    for _ in 0..3 {
        chain.key(&shut, MenuKey::Down, &cx);
    }
    chain.draw(&mut c, &options, &text, true);
    c
}

/// Glass's balloons, 950x250, over white, grey and the stripes: at rest, with action buttons under a Top bar, a balloon tip and a reply row.
pub fn render_glass_balloons(theme: &Theme) -> Canvas {
    use reprisekit::controls::tooltip::{self, NotifyHit};
    use reprisekit::taskbar::Edge;
    let text = TextRenderer::load(&theme.fonts);
    let mut c = Canvas::new(950, 250);
    c.fill_rect(Rect::new(0, 0, 320, 250), Color::rgb(0xFF, 0xFF, 0xFF));
    c.fill_rect(Rect::new(320, 0, 320, 250), Color::rgb(0x80, 0x80, 0x80));
    c.blit_canvas(640, 0, &blurred_stripes(310, 250));
    let icon = reprisekit::icon::fallback(theme, 16);
    let lay = tooltip::notify_layout(
        theme,
        &text,
        "Updates are ready",
        "Click here to install them. Your computer may restart once they are in.",
        &[],
        &[],
        None,
    );
    let rest = tooltip::draw_notify_balloon(theme, &text, &lay, Some(&icon), &[], None, None, None);
    c.blit_over(8, 8, &rest);
    let lay = tooltip::notify_layout(
        theme,
        &text,
        "New message",
        "From Ann: Lunch at noon?",
        &["Reply", "Mark as read"],
        &[],
        None,
    )
    .on_edge(Edge::Top);
    for (x, hot) in [(328, NotifyHit::Close), (644, NotifyHit::Button(1))] {
        let lit = tooltip::draw_notify_balloon(
            theme,
            &text,
            &lay,
            Some(&icon),
            &[],
            None,
            Some(hot),
            None,
        );
        c.blit_over(x, 8, &lit);
    }
    let reply = tooltip::notify_layout(
        theme,
        &text,
        "New message",
        "From Ann: Lunch at noon?",
        &["Reply", "Mark as read"],
        &[],
        Some(tooltip::NotifyReply {
            placeholder: "Write a reply",
            send: "Send",
        }),
    );
    let typed = reprisekit::textedit::TextEdit::new("Sounds good", false);
    let caret = reprisekit::textedit::Caret::Shown;
    let open = tooltip::draw_notify_balloon(
        theme,
        &text,
        &reply,
        Some(&icon),
        &[],
        None,
        None,
        Some((&typed, caret, None)),
    );
    c.blit_over(328, 130, &open);
    let tip = tooltip::draw_balloon_tip(
        theme,
        &text,
        "Turn Off",
        "Ends your session and shuts down the computer so that you can safely turn off power.",
    );
    c.blit_over(8, 140, &tip);
    c
}

/// Glass's fields on its face, 950x150: edits, drop-down lists, editable combos and spinners in each state, then every frame of the field, list and spin-button parts.
pub fn render_glass_fields(theme: &Theme) -> Canvas {
    use reprisekit::controls::updown;
    use reprisekit::look::widgets::draw_edit_field;
    use reprisekit::style::{Part, State};
    let text = TextRenderer::load(&FontConfig::default());
    let p = theme.paint();
    let cm = theme.look.controls();
    let px = theme.fonts.size_px;
    let mut c = Canvas::new(950, 150);
    c.fill_rect(c.bounds(), theme.colors.button_face);
    let edits = [
        (false, Interaction::Idle),
        (false, Interaction::Hot),
        (true, Interaction::Idle),
    ];
    for (i, (focused, ptr)) in edits.into_iter().enumerate() {
        let r = Rect::new(10 + i as i32 * 80, 10, 75, 21);
        let inner = draw_edit_field(&mut c, &p, r, theme.colors.window, focused, ptr);
        text.draw(
            &mut c,
            inner,
            "Edit",
            false,
            theme.colors.button_text,
            Align::Left,
            px,
        );
    }
    let greyed = form::Form::new(vec![Item::edit(
        "greyed",
        Rect::new(250, 10, 75, 21),
        "Greyed",
        false,
    )]);
    form::draw(&mut c, &text, &p, &greyed, false, px);
    let idle = combo::ComboLit::IDLE;
    let lit = |frame, drop| combo::ComboLit { frame, drop };
    let combos = [
        (340, 10, false, false, true, idle),
        (490, 10, false, true, true, idle),
        (640, 10, false, false, false, idle),
        (
            790,
            10,
            false,
            false,
            true,
            lit(Interaction::Hot, Interaction::Hot),
        ),
        (10, 45, true, false, true, idle),
        (160, 45, true, true, true, idle),
        (
            490,
            45,
            true,
            false,
            true,
            lit(Interaction::Hot, Interaction::Idle),
        ),
        (
            640,
            45,
            true,
            false,
            true,
            lit(Interaction::Hot, Interaction::Hot),
        ),
        (
            790,
            45,
            false,
            false,
            true,
            lit(Interaction::Pressed, Interaction::Pressed),
        ),
    ];
    for (x, y, editable, focus, enabled, lit) in combos {
        let mut cb = combo::Combo::new(&["(None)", "Blank"], 0, 8);
        cb.enabled = enabled;
        if editable {
            cb.edit = Some(reprisekit::textedit::TextEdit::new("notepad", false));
        }
        let r = Rect::new(x, y, 140, combo::COMBO_H);
        let lay = combo::layout(&cb, r, &cm);
        combo::draw(
            &mut c,
            &text,
            &p,
            &cb,
            &lay,
            focus,
            focus,
            Caret::Unfocused,
            lit,
            px,
        );
    }
    for (i, enabled) in [true, false].into_iter().enumerate() {
        let mut ud = updown::UpDown::new(10, 1, 9999);
        ud.enabled = enabled;
        let lay = updown::layout(310 + i as i32 * 60, 45);
        updown::draw(&mut c, &text, &p, &ud, &lay, false, None, px);
    }
    let st = theme.look.style().unwrap();
    let states = [State::Normal, State::Hot, State::Pressed, State::Disabled];
    let mut x = 10;
    for state in [State::Normal, State::Hot, State::Focused, State::Disabled] {
        let r = Rect::new(x, 100, 40, 21);
        c.fill_rect(r, theme.colors.window);
        st.draw(&mut c, r, Part::EditBorder, state, 1);
        x += 46;
    }
    for state in states {
        st.draw(&mut c, Rect::new(x, 100, 60, 21), Part::ComboList, state, 1);
        x += 66;
    }
    for state in states {
        let field = Rect::new(x, 100, 40, 21);
        c.fill_rect(field, theme.colors.window);
        st.draw(&mut c, field, Part::EditBorder, State::Normal, 1);
        let drop = Rect::new(
            field.right() - 2 - cm.combo_drop,
            field.y + 2,
            cm.combo_drop,
            17,
        );
        st.draw(&mut c, drop, Part::ComboDrop, state, 1);
        x += 46;
    }
    for part in [Part::SpinUp, Part::SpinDown] {
        for state in states {
            st.draw(&mut c, Rect::new(x, 100, 15, 8), part, state, 1);
            x += 20;
        }
    }
    c
}

/// Glass's open combo lists, 950x110: a drop-down list on white, and a scrolling one on the face.
pub fn render_glass_open_lists(theme: &Theme) -> Canvas {
    let text = TextRenderer::load(&FontConfig::default());
    let p = theme.paint();
    let cm = theme.look.controls();
    let px = theme.fonts.size_px;
    let mut c = Canvas::new(950, 110);
    c.fill_rect(c.bounds(), theme.colors.button_face);
    c.fill_rect(Rect::new(0, 0, 320, 110), theme.colors.window);
    let lists: [(Rect, &[&str], usize, usize, usize); 2] = [
        (
            Rect::new(10, 10, 279, 23),
            &[
                "Show icon and notifications",
                "Hide icon and notifications",
                "Only show notifications",
            ],
            3,
            0,
            1,
        ),
        (
            Rect::new(340, 10, 200, 21),
            &["(None)", "Blank", "Bubbles", "Mystify", "Photos", "Ribbons"],
            4,
            1,
            3,
        ),
    ];
    for (r, items, rows, top, hot) in lists {
        let mut cb = combo::Combo::new(items, 0, rows);
        cb.toggle();
        cb.top = top;
        cb.open = Some(hot);
        let lay = combo::layout(&cb, r, &cm);
        let lit = combo::ComboLit {
            frame: Interaction::Pressed,
            drop: Interaction::Pressed,
        };
        combo::draw(
            &mut c,
            &text,
            &p,
            &cb,
            &lay,
            true,
            false,
            Caret::Unfocused,
            lit,
            px,
        );
        combo::draw_list(
            &mut c,
            &text,
            &p,
            &cb.items,
            cb.open,
            cb.top,
            cb.rows,
            lay.list,
            lay.scrollbar,
            None,
            px,
        );
    }
    c
}

/// Glass's dialog controls on its face, 950x120: check box, radio, static label and push button at rest and greyed, then every frame of the check box and radio parts.
pub fn render_glass_choices(theme: &Theme) -> Canvas {
    use reprisekit::style::{Check, Part, State};
    let text = TextRenderer::load(&FontConfig::default());
    let mut c = Canvas::new(950, 120);
    c.fill_rect(c.bounds(), theme.colors.button_face);
    let choices = form::Form::new(vec![
        Item::check("check", 10, 10, "&Check box").checked(true),
        Item::check("greyed_check", 10, 34, "&Greyed check box")
            .checked(true)
            .enabled(false),
        Item::radio("radio", 200, 10, "&Radio", "radio").checked(true),
        Item::radio("greyed_radio", 200, 34, "Gr&eyed radio", "greyed")
            .checked(true)
            .enabled(false),
        Item::label("greyed_label", Rect::new(380, 12, 120, 13), "Greyed label:").enabled(false),
        Item::button("ok", Rect::new(520, 8, 75, 23), "OK", true, false),
        Item::button("apply", Rect::new(605, 8, 75, 23), "&Apply", false, false),
    ]);
    form::draw(
        &mut c,
        &text,
        &theme.paint(),
        &choices,
        false,
        theme.fonts.size_px,
    );
    let st = theme.look.style().unwrap();
    let states = [State::Normal, State::Hot, State::Pressed, State::Disabled];
    let mut x = 10;
    for (part, checks) in [
        (
            Part::CheckBox,
            &[Check::Unchecked, Check::Checked, Check::Mixed][..],
        ),
        (Part::Radio, &[Check::Unchecked, Check::Checked][..]),
    ] {
        for &check in checks {
            for state in states {
                st.draw_check(&mut c, Rect::new(x, 80, 13, 13), part, check, state, 1);
                x += 20;
            }
            x += 10;
        }
    }
    c
}
