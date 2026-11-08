use super::*;

/// Tabs on two rows grow the sheet by one row: the page keeps its size
/// 18 px lower, and the controls and buttons move down with it.
#[test]
fn a_second_row_of_tabs_grows_the_sheet_by_a_row() {
    let text = TextRenderer::load(&FontConfig::default());
    let px = Theme::default().fonts.size_px;
    let labels = [
        "General",
        "Sharing",
        "Customize",
        "Security",
        "Previous Versions",
        "Summary",
        "Compatibility",
    ];
    let mut s = sheet(&labels, (361, 424));
    let before = s.layout(&text, px);
    assert_eq!(s.fit_tabs(&text, px), 18);
    assert_eq!(s.size, (361, 442));
    let l = s.layout(&text, px);
    assert_eq!(l.page, Rect::new(6, 46, 349, 360));
    assert_eq!(l.ok, Rect::new(before.ok.x, before.ok.y + 18, 75, 23));
    assert_eq!(
        s.open().item("check").unwrap().rect,
        Rect::new(20, 68, 13, 13)
    );
    let mut one = sheet(&["Themes", "Desktop"], (398, 424));
    assert_eq!(one.fit_tabs(&text, px), 0);
    assert_eq!(one.size, (398, 424));
}

/// Tabs on one row across the sheet as built wrap onto two in a client cut narrower: the
/// page starts a row lower, the buttons keep to the client's bottom, and a client as wide
/// as the sheet takes the row back.
#[test]
fn a_narrow_client_wraps_the_tabs_to_its_width() {
    let text = TextRenderer::load(&FontConfig::default());
    let theme = Theme::default();
    let px = theme.fonts.size_px;
    let labels = [
        "General",
        "Sharing",
        "Customize",
        "Security",
        "Previous Versions",
        "Summary",
        "Compatibility",
    ];
    let mut s = sheet(&labels, (700, 424));
    s.drawn_with(&theme.paint());
    assert_eq!(s.fit_tabs(&text, px), 0);
    s.fit((361, 424));
    assert_eq!(s.fit_tabs(&text, px), 18);
    assert_eq!(s.fit_tabs(&text, px), 0, "idempotent");
    assert_eq!(s.client(), (361, 424), "the client keeps its size");
    let l = s.layout(&text, px);
    assert!(
        l.tabs.iter().all(|t| t.right() <= 361 - MARGIN),
        "every tab shows: {:?}",
        l.tabs
    );
    assert_eq!(l.page.y, 46, "the page starts under the second row");
    assert_eq!(l.ok.bottom(), 424 - BUTTON_BOTTOM);
    assert_eq!(s.open().rect("check").unwrap().y, 68);

    s.fit((700, 424));
    assert_eq!(s.fit_tabs(&text, px), -18);
    assert_eq!(s.size, (700, 424));
    assert_eq!(s.page_offset(), (0, 0));
    assert_eq!(s.layout(&text, px).ok.bottom(), 424 - BUTTON_BOTTOM);
}

/// A page cut short above where its last control ends shows whole: no scrollbar, and
/// the wheel moves nothing.
#[test]
fn a_page_cut_short_scrolls_no_further_than_its_last_control() {
    let text = TextRenderer::load(&FontConfig::default());
    let theme = Theme::default();
    let px = theme.fonts.size_px;
    let mut s = sheet(&["General"], (398, 424));
    s.drawn_with(&theme.paint());
    assert!(s.fit((398, 200)));
    assert_eq!(s.layout(&text, px).bar, None, "Check shows whole");
    assert_eq!(s.wheel(None, 3), Event::None);
    assert_eq!(s.page_offset(), (0, 0));
}

/// A sheet is set in its first page's font, every page and the tabs
/// with it, and a page put in later takes it too.
#[test]
fn a_sheet_takes_its_first_pages_font() {
    let text =
        TextRenderer::load(&FontConfig::default()).with_shell_dlg(TextRenderer::stub(11, |_| 9));
    let second = text.dialog(DialogFont::ShellDlg);
    let pages = vec![
        Form::new(vec![]).set_in(DialogFont::ShellDlg),
        Form::new(vec![]),
    ];
    let mut s = PropSheet::new(&["General", "Web"], pages, (371, 424));
    assert_eq!(s.font, DialogFont::ShellDlg);
    s.set_page(1, Form::new(vec![]));
    assert!(s.pages.iter().all(|p| p.font == DialogFont::ShellDlg));
    let l = s.layout(&text, 11);
    let want = tabstrip::layout(
        second,
        11,
        &s.tabs,
        l.strip.0,
        l.strip.1,
        l.page.width,
        TextUnits::ONE,
    );
    assert_eq!(l.tabs, want);
    let ui = sheet(&["General", "Web"], (371, 424));
    assert_eq!(ui.font, DialogFont::Ui);
    assert_ne!(ui.layout(&text, 11).tabs, want, "the UI font's tabs");
}

#[test]
fn every_page_gets_the_buttons_and_apply_greys_together() {
    let mut s = sheet(&["Volume", "Sounds"], (361, 292));
    let (ok, _, apply) = buttons(s.size);
    for page in &s.pages {
        assert_eq!(page.item(OK).map(|i| i.rect), Some(ok));
        assert_eq!(page.item(APPLY).map(|i| i.rect), Some(apply));
        assert_eq!(
            page.item(APPLY).map(|i| i.control.clone()),
            Some(Control::Button {
                label: "&Apply".into(),
                enabled: false,
                default: false,
            }),
            "a sheet opens with nothing to apply"
        );
    }
    s.set_apply_enabled(true);
    assert!(s.pages.iter().all(|p| matches!(
        p.item(APPLY).map(|i| &i.control),
        Some(Control::Button { enabled: true, .. })
    )));
}

#[test]
fn close_only_lays_out_one_button() {
    let pages = vec![Form::new(vec![]), Form::new(vec![])];
    let s = close_only(&["General", "Support"], pages, (361, 419));
    let text = TextRenderer::load(&FontConfig::default());
    let px = Theme::default().fonts.size_px;
    for page in &s.pages {
        let buttons: Vec<&Item> = page
            .items
            .iter()
            .filter(|i| matches!(i.control, Control::Button { .. }))
            .collect();
        assert_eq!(buttons.len(), 1, "one button per page");
        assert_eq!(buttons[0].id, OK);
        assert_eq!(
            buttons[0].control,
            Control::Button {
                label: "Close".into(),
                enabled: true,
                default: true,
            }
        );
    }
    // At a 361x419 client, the Apply slot flush with the page's right
    // edge.
    let l = s.layout(&text, px);
    assert_eq!(l.ok, Rect::new(280, 389, BUTTON_W, BUTTON_H));
    assert_eq!(l.ok.right(), 361 - MARGIN);
    assert_eq!(l.cancel.width, 0);
    assert_eq!(l.apply.width, 0);
}

/// A placed sheet reports its rects and holds its controls at the
/// origin; fitting and placing again move nothing.
#[test]
fn a_placed_sheet_moves_with_its_origin() {
    let text = TextRenderer::load(&FontConfig::default());
    let px = Theme::default().fonts.size_px;
    let mut s = sheet(&["General", "Advanced"], (338, 376));
    let before = s.layout(&text, px);
    s.place((3, 21));
    s.fit_tabs(&text, px);
    s.place((3, 21));
    assert_eq!(s.fit_tabs(&text, px), 0);
    let l = s.layout(&text, px);
    assert_eq!(l.strip, (9, 29));
    assert_eq!(l.page, Rect::new(9, 49, 326, 312));
    assert_eq!(l.ok, Rect::new(98, 367, BUTTON_W, BUTTON_H));
    assert_eq!(l.tabs[0].x, before.tabs[0].x + 3);
    assert_eq!(s.open().rect(OK), Some(l.ok));
    assert_eq!(s.open().rect("check"), Some(Rect::new(23, 71, 13, 13)));
    assert!(matches!(
        s.hit(24, 72, &text, px),
        Some(Hit::Control(form::Hit { item: 0, .. }))
    ));
}

/// A page set anew lands where the pages sit, under the sheet's
/// buttons, and keeps Apply as it stood.
#[test]
fn a_page_set_anew_joins_the_sheet() {
    let mut s = sheet(&["General", "Advanced"], (338, 376));
    s.place((3, 21));
    s.set_apply_enabled(true);
    let page = Form::new(vec![Item::new(
        "new",
        Rect::new(20, 50, 13, 13),
        Control::Check {
            label: "New".into(),
            checked: false,
            enabled: true,
            mixed: None,
        },
    )]);
    s.set_page(1, page);
    let p = &s.pages[1];
    assert_eq!(p.rect("new"), Some(Rect::new(23, 71, 13, 13)));
    assert_eq!(p.rect(OK), s.pages[0].rect(OK));
    assert!(matches!(
        p.item(APPLY).unwrap().control,
        Control::Button { enabled: true, .. }
    ));
}

/// OK and Cancel alone take the row's two right slots.
#[test]
fn ok_cancel_fills_the_right_slots() {
    let text = TextRenderer::load(&FontConfig::default());
    let px = Theme::default().fonts.size_px;
    let pages = vec![Form::new(vec![]), Form::new(vec![])];
    let s = PropSheet::with_buttons(&["General", "Web"], pages, (371, 424), Buttons::OkCancel);
    let l = s.layout(&text, px);
    assert_eq!(l.ok, Rect::new(209, 394, BUTTON_W, BUTTON_H));
    assert_eq!(l.cancel, Rect::new(290, 394, BUTTON_W, BUTTON_H));
    assert_eq!(l.apply.width, 0);
    for page in &s.pages {
        assert_eq!(page.items.len(), 2);
        assert_eq!(page.item(OK).unwrap().rect, l.ok);
        assert_eq!(page.item(CANCEL).unwrap().rect, l.cancel);
        assert_eq!(page.focus(), page.index_of(OK));
    }
}

#[test]
#[should_panic(expected = "one page per tab")]
fn a_tab_without_a_page_is_refused() {
    PropSheet::new(&["General", "Sharing"], vec![Form::new(vec![])], (361, 424));
}

#[test]
#[should_panic(expected = "one page per tab")]
fn a_close_only_tab_without_a_page_is_refused() {
    close_only(&["General", "Support"], vec![Form::new(vec![])], (361, 419));
}

#[test]
fn ctrl_tab_cycles_the_pages() {
    let mut s = sheet(&["Volume", "Sounds", "Audio"], (361, 292));
    assert_eq!(s.tabs.open, 0);
    assert_eq!(
        s.key(form::Key::Tab { back: false }, Some(false)),
        Event::Redraw
    );
    assert_eq!(s.tabs.open, 1);
    assert!(
        matches!(&s.open().item("check").unwrap().control,
            Control::Check { label, .. } if label == "Sounds"),
        "the open page followed the tab"
    );
    assert_eq!(
        s.key(form::Key::Tab { back: true }, Some(true)),
        Event::Redraw
    );
    assert_eq!(s.tabs.open, 0);
    assert_eq!(
        s.key(form::Key::Tab { back: true }, Some(true)),
        Event::Redraw
    );
    assert_eq!(s.tabs.open, 2, "wraps");
    // Without the Ctrl flag Tab walks the page's controls instead.
    let before = s.tabs.open;
    assert_eq!(s.key(form::Key::Tab { back: false }, None), Event::Redraw);
    assert_eq!(s.tabs.open, before);
}

#[test]
fn the_sheets_buttons_become_sheet_events() {
    let text = TextRenderer::load(&FontConfig::default());
    let px = Theme::default().fonts.size_px;
    let mut s = sheet(&["Volume", "Sounds"], (361, 292));
    s.set_apply_enabled(true);
    let l = s.layout(&text, px);
    // Enter with the page's own control focused is the default button.
    assert_eq!(s.key(form::Key::Enter, None), Event::Ok);
    assert_eq!(s.key(form::Key::Escape, None), Event::Cancel);
    // Press and release Apply.
    let hit = s.hit(l.apply.x + 5, l.apply.y + 5, &text, px);
    assert_eq!(s.click(hit), Event::Redraw, "the button goes down");
    assert_eq!(s.release(), Event::Apply);
    // A control of the page's own stays the page's.
    let check = s.open().index_of("check").unwrap();
    let hit = s.hit(
        s.open().items[check].rect.x + 3,
        s.open().items[check].rect.y + 3,
        &text,
        px,
    );
    assert!(matches!(hit, Some(Hit::Control(_))));
    assert_eq!(s.click(hit), Event::Redraw, "the box goes down");
    assert_eq!(s.release(), Event::Page(0, form::Event::Changed("check")));
}

#[test]
fn a_tab_hit_opens_its_page() {
    let text = TextRenderer::load(&FontConfig::default());
    let px = Theme::default().fonts.size_px;
    let mut s = sheet(&["Volume", "Sounds"], (361, 292));
    let l = s.layout(&text, px);
    let hit = s.hit(l.tabs[1].x + 4, l.tabs[1].y + 10, &text, px);
    assert_eq!(hit, Some(Hit::Tab(1)));
    assert_eq!(s.click(hit), Event::Redraw);
    assert_eq!(s.tabs.open, 1);
    assert!(!s.tabs.focused, "a closed tab opens its page");
    assert_eq!(s.open().focus(), Some(0), "on its first control");
    assert_eq!(s.click(hit), Event::Redraw, "the open tab takes the focus");
    assert!(s.tabs.focused);
    assert_eq!(s.open().focus(), None);
    assert_eq!(s.click(hit), Event::None, "already focused");
    // A control pressed on the page takes the focus back.
    let check = s.hit(25, 55, &text, px);
    s.click(check);
    assert!(!s.tabs.focused);
    assert_eq!(s.open().focus(), Some(0));
}

/// Around the strip and back: Shift+Tab off the page's first control
/// reaches the strip, Right and Left turn the tabs with the focus kept
/// there, Up and Down do nothing, Ctrl+Tab turns them too, and Tab
/// goes into the open page.
#[test]
fn the_strip_is_a_tab_stop_its_arrows_turn_the_tabs() {
    let mut s = sheet(&["Volume", "Sounds", "Audio"], (361, 292));
    let tab = |s: &mut PropSheet, back| s.key(form::Key::Tab { back }, None);
    assert_eq!(s.open().focus(), Some(0));
    assert_eq!(tab(&mut s, true), Event::Redraw);
    assert!(s.tabs.focused);
    assert_eq!(s.open().focus(), None);
    assert_eq!(s.key(form::Key::Left, None), Event::None, "first tab");
    // The arrow as a host folds it for a field: an editing key.
    let right = form::Key::Edit(EditKey::Right {
        select: false,
        word: false,
    });
    assert_eq!(s.key(right, None), Event::Redraw);
    assert_eq!((s.tabs.open, s.tabs.focused), (1, true));
    assert_eq!(s.open().focus(), None, "the new page holds no focus");
    assert_eq!(s.key(form::Key::Down, None), Event::None);
    assert_eq!(s.key(form::Key::Up, None), Event::None);
    assert_eq!(s.tabs.open, 1);
    assert_eq!(
        s.key(form::Key::Tab { back: false }, Some(false)),
        Event::Redraw
    );
    assert_eq!((s.tabs.open, s.tabs.focused), (2, true));
    assert_eq!(s.key(form::Key::Right, None), Event::None, "last tab");
    // Enter on the strip is the default button, Escape Cancel.
    assert_eq!(s.key(form::Key::Enter, None), Event::Ok);
    assert_eq!(s.key(form::Key::Escape, None), Event::Cancel);
    // Shift+Tab leaves for the page's last tab stop: Cancel, the
    // greyed Apply skipped.
    assert_eq!(tab(&mut s, true), Event::Redraw);
    assert!(!s.tabs.focused);
    assert_eq!(s.open().focus(), s.open().index_of(CANCEL));
    tab(&mut s, false);
    tab(&mut s, false);
    assert_eq!(s.open().focus(), Some(0), "Tab from the strip");
    // Ctrl+Tab from the page lands on the new page's first control,
    // whatever that page last had focused.
    s.pages[0].focus_id(OK);
    s.key(form::Key::Tab { back: false }, Some(false));
    assert_eq!((s.tabs.open, s.tabs.focused), (0, false));
    assert_eq!(s.open().focus(), Some(0));
}
