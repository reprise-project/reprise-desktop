use super::*;

#[test]
fn a_query_filters_the_list_and_enter_focuses_the_control_it_found() {
    let (mut a, mut ctx) = (app("search-enter"), test_ctx());
    find(&mut a, &mut ctx, "debug overlay");
    let adv = Page::Advanced.index();
    assert_eq!(a.search_hits[adv], Some(Hit::Control(pages::DEBUG_OVERLAY)));
    assert!(
        a.search_hits
            .iter()
            .enumerate()
            .all(|(i, h)| i == adv || h.is_none())
    );
    let list = a.sheet.list_mut().unwrap();
    assert!(list.shows(pagelist::Pick::Page(adv)));
    assert!(!list.shows(pagelist::Pick::Page(Page::Appearance.index())));
    plain_key(&mut a, &ctx, keysym::RETURN);
    assert_eq!(registry::open(&a.sheet), Page::Advanced);
    assert_eq!(focused_id(&a), Some(pages::DEBUG_OVERLAY));
    assert!(!a.sheet.tabs.focused);
}

#[test]
fn enter_on_a_found_control_takes_the_keyboard_from_the_permission_list() {
    let (mut a, mut ctx) = (app("search-from-list"), test_ctx());
    a.open_page(Page::Privacy, None);
    a.permissions_due = false;
    pages::fill_permissions(&mut a.permissions, &[("Maps".into(), "Location", true)]);
    press_side_row(&mut a, &mut ctx, 0, 1000);
    assert!(a.permissions_focus);
    find(&mut a, &mut ctx, "history");
    assert_eq!(
        a.search_hits[Page::Privacy.index()],
        Some(Hit::Keyword(Some("clipboard_history"))),
        "a keyword naming one control"
    );
    plain_key(&mut a, &ctx, keysym::RETURN);
    assert_eq!(registry::open(&a.sheet), Page::Privacy);
    assert_eq!(focused_id(&a), Some("clipboard_history"));
    assert_eq!(holders(&a), PAGE);
}

#[test]
fn escape_on_the_found_control_clears_the_query_and_keeps_the_sheet_open() {
    let (mut a, mut ctx) = (app("search-escape"), test_ctx());
    find(&mut a, &mut ctx, "debug overlay");
    plain_key(&mut a, &ctx, keysym::RETURN);
    let escape = keysym::ESCAPE;
    let cancelled = |a: &SettingsApp| matches!(a.display.pending, Some(displayhost::Press::Cancel));
    plain_key(&mut a, &ctx, escape);
    assert!(!cancelled(&a));
    assert!(a.search_hits.is_empty());
    assert_eq!(a.sheet.list_mut().unwrap().filter(), None);
    assert_eq!(focused_id(&a), Some(pages::DEBUG_OVERLAY));
    plain_key(&mut a, &ctx, escape);
    assert!(cancelled(&a), "with the field empty Escape cancels");
}

#[test]
fn enter_on_a_found_page_already_open_focuses_the_control_it_found() {
    let (mut a, mut ctx) = (app("search-enter-open"), test_ctx());
    a.open_page(Page::Keyboard, None);
    find(&mut a, &mut ctx, "repeat delay");
    let kbd = Page::Keyboard.index();
    assert_eq!(a.search_hits[kbd], Some(Hit::Control("repeat_delay")));
    plain_key(&mut a, &ctx, keysym::RETURN);
    assert_eq!(registry::open(&a.sheet), Page::Keyboard);
    assert_eq!(focused_id(&a), Some("repeat_delay"));
    assert!(!a.shortcuts_focus, "the shortcut list lets go");
}

/// A left press on `page`'s row of the page list, in a window that shows the whole sheet.
fn press_page_row(a: &mut SettingsApp, ctx: &mut Ctx, page: Page) {
    ctx.size = (1920, 1080);
    a.event(MAIN, Event::Resized, ctx);
    let px = a.sheet.open().units().px(ctx.theme.fonts.size_px);
    let inner = a
        .sheet
        .layout(&ctx.text, px)
        .list
        .expect("a page list")
        .inner;
    let x = inner.x + 20;
    let y = (inner.y..inner.bottom())
        .find(|&y| a.sheet.hit(x, y, &ctx.text, px) == Some(propsheet::Hit::Tab(page.index())))
        .expect("the page's row shows");
    let press = PointerInput::Press {
        x,
        y,
        button: BTN_LEFT,
        time: 0,
        mods: Mods::default(),
    };
    a.event(MAIN, Event::Pointer(press), ctx);
}

#[test]
fn a_found_page_clicked_in_the_list_focuses_the_control_it_found() {
    for open_first in [false, true] {
        let (mut a, mut ctx) = (app("search-click"), test_ctx());
        if open_first {
            a.open_page(Page::Advanced, None);
        }
        find(&mut a, &mut ctx, "debug overlay");
        press_page_row(&mut a, &mut ctx, Page::Advanced);
        assert_eq!(registry::open(&a.sheet), Page::Advanced);
        assert_eq!(
            focused_id(&a),
            Some(pages::DEBUG_OVERLAY),
            "the page open before: {open_first}"
        );
    }
}

#[test]
fn a_page_handed_over_clears_a_query_that_hides_it() {
    let mut a = app("handoff-hidden");
    let query = |a: &mut SettingsApp, q: &str| {
        a.sheet.search.clear();
        a.sheet
            .search
            .key(reprisekit::textedit::EditKey::Insert(q.into()));
        a.sheet_event(propsheet::Event::Search);
    };
    query(&mut a, "wifi");
    assert!(
        !a.sheet
            .list_mut()
            .unwrap()
            .shows(pagelist::Pick::Page(Page::MouseTouchpad.index()))
    );
    assert!(a.request(&page_request("mouse-and-touchpad")));
    assert_eq!(registry::open(&a.sheet), Page::MouseTouchpad);
    assert_eq!(a.sheet.search.text(), "");
    assert!(a.search_hits.is_empty());
    assert_eq!(a.sheet.list_mut().unwrap().filter(), None);

    query(&mut a, "mouse");
    assert!(a.request(&page_request("text-and-keys")));
    assert_eq!(
        a.sheet.search.text(),
        "mouse",
        "a query showing the page stays"
    );
    assert!(a.sheet.list_mut().unwrap().filter().is_some());
}

/// The keys that step the list keep the keyboard on it, so the next row is a key away.
#[test]
fn down_and_tab_from_the_field_open_a_found_page_without_focusing_its_control() {
    let (mut a, mut ctx) = (app("search-down"), test_ctx());
    find(&mut a, &mut ctx, "debug overlay");
    plain_key(&mut a, &ctx, keysym::DOWN);
    assert_eq!(registry::open(&a.sheet), Page::Advanced);
    assert!(a.sheet.tabs.focused);
    assert_eq!(focused_id(&a), None);

    let (mut a, mut ctx) = (app("search-tab"), test_ctx());
    find(&mut a, &mut ctx, "debug overlay");
    tab(&mut a, &ctx, false);
    assert_eq!(registry::open(&a.sheet), Page::Advanced);
    assert!(a.sheet.tabs.focused);
    assert_eq!(focused_id(&a), None);
}

#[test]
fn an_emptied_query_drops_the_hits() {
    let mut a = app("search-clear");
    a.open_page(Page::Backup, None);
    a.sheet
        .search
        .key(reprisekit::textedit::EditKey::Insert("night".into()));
    a.sheet_event(propsheet::Event::Search);
    assert!(a.search_hits[Page::NightLight.index()].is_some());
    a.sheet.search.clear();
    a.sheet_event(propsheet::Event::Search);
    assert!(a.search_hits.is_empty());
    let list = a.sheet.list_mut().unwrap();
    assert_eq!(list.filter(), None, "every row shows again");
    assert!(list.top > 0, "the open page's row scrolls back into view");
}

#[test]
fn a_query_that_shows_the_same_rows_keeps_the_list_scrolled() {
    let mut a = app("search-scroll");
    a.search("s");
    a.sheet.list_mut().unwrap().top = 40;
    a.search("s ");
    assert_eq!(a.sheet.list_mut().unwrap().top, 40);
    a.search("sw");
    assert_eq!(
        a.sheet.list_mut().unwrap().top,
        0,
        "fewer rows start at the top"
    );
}

#[test]
fn enter_on_a_greyed_found_control_leaves_the_keyboard_on_the_list() {
    let (mut a, mut ctx) = (app("search-greyed"), test_ctx());
    find(&mut a, &mut ctx, "faster glass");
    let page = a
        .search_hits
        .iter()
        .position(|h| *h == Some(Hit::Control("glass_fast_drag")))
        .expect("a page holds the control");
    assert!(
        !a.sheet.pages[page].can_focus("glass_fast_drag"),
        "greyed under Classic"
    );
    plain_key(&mut a, &ctx, keysym::RETURN);
    assert_eq!(a.sheet.tabs.open, page);
    assert_eq!(holders(&a), [false, false, true, false]);
}
