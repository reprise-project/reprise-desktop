use super::*;
use reprisekit::classic::widgets::scrollbar_parts;

#[test]
fn a_field_copies_its_selection_and_takes_a_paste() {
    let mut a = app("field-clipboard");
    let mut ctx = test_ctx();
    a.open_page(Page::WindowBehavior, Some("snap_layouts"));
    a.event(MAIN, Event::Pasted("1|1 2\nmore".into()), &mut ctx);
    let behavior = Page::WindowBehavior.index();
    let text = |a: &SettingsApp| match &a.sheet.pages[behavior]
        .item("snap_layouts")
        .unwrap()
        .control
    {
        form::Control::Edit { edit, .. } => edit.text.clone(),
        _ => unreachable!(),
    };
    assert_eq!(text(&a), "1|1 2", "the paste's first line goes in");
    a.event(MAIN, Event::Key(ctrl(b'a')), &mut ctx);
    let r = a.event(MAIN, Event::Key(ctrl(b'c')), &mut ctx);
    assert_eq!(r.copy.as_deref(), Some("1|1 2"));
    assert!(a.event(MAIN, Event::Key(ctrl(b'v')), &mut ctx).paste);
}

#[test]
fn the_permission_list_takes_its_tab_stop_after_its_check_on_the_privacy_page() {
    let mut a = app("privacy-tab");
    let ctx = test_ctx();
    a.open_page(Page::Privacy, None);
    assert!(
        !a.permissions_focus,
        "opening the page leaves the list unfocused"
    );

    let page = a.sheet.open_mut();
    let check = page.index_of(pages::PRIVACY_LOCATION).unwrap();
    page.set_focus(Some(check));
    tab(&mut a, &ctx, false);
    assert!(
        a.permissions_focus && a.permissions.focus_ring,
        "Tab from the check enters the list"
    );
    assert_eq!(a.sheet.open().focus(), None, "one dotted rectangle");

    tab(&mut a, &ctx, false);
    assert!(!a.permissions_focus);
    let next = a
        .sheet
        .open()
        .focus()
        .expect("a form control after the list");
    assert!(next > check, "Tab leaves the list onward, got item {next}");

    tab(&mut a, &ctx, true);
    assert!(a.permissions_focus, "Shift+Tab comes back into the list");
    tab(&mut a, &ctx, true);
    assert!(!a.permissions_focus);
    assert_eq!(
        a.sheet.open().focus(),
        Some(check),
        "Shift+Tab leaves it for its check"
    );
}

const FIELD: [bool; 4] = [true, false, false, false];

fn ctrl_key(a: &mut SettingsApp, ctx: &Ctx, sym: u32, shift: bool) {
    let mods = Mods {
        ctrl: true,
        shift,
        alt: false,
    };
    press_key(a, ctx, sym, mods);
}

fn press_search_field(a: &mut SettingsApp, ctx: &mut Ctx) {
    ctx.size = (1920, 1080);
    let px = a.sheet.open().units().px(ctx.theme.fonts.size_px);
    let field = a
        .sheet
        .layout(&ctx.text, px)
        .search
        .expect("a search field");
    let press = PointerInput::Press {
        x: field.x + 4,
        y: field.y + 4,
        button: BTN_LEFT,
        time: 0,
        mods: Mods::default(),
    };
    a.event(MAIN, Event::Pointer(press), ctx);
}

fn press_control(a: &mut SettingsApp, ctx: &mut Ctx, id: &str, button: u32) {
    ctx.size = (1920, 1080);
    let r = a.sheet.open().rect(id).expect("the control");
    let press = PointerInput::Press {
        x: r.x + 4,
        y: r.y + 4,
        button,
        time: 0,
        mods: Mods::default(),
    };
    a.event(MAIN, Event::Pointer(press), ctx);
}

#[test]
fn the_search_field_and_a_side_list_never_hold_the_keyboard_together() {
    let mut a = app("search-side-list");
    let mut ctx = test_ctx();
    a.open_page(Page::Keyboard, None);
    assert_eq!(
        holders(&a),
        SIDE_LIST,
        "the page opens on its shortcut list"
    );

    press_search_field(&mut a, &mut ctx);
    assert_eq!(holders(&a), FIELD, "a press in the field");

    press_side_row(&mut a, &mut ctx, 0, 1000);
    assert_eq!(holders(&a), SIDE_LIST, "a press on the shortcut list");

    press_control(&mut a, &mut ctx, pages::SC_ADD, BTN_LEFT);
    assert_eq!(holders(&a), PAGE, "a press on Add...");
    press_side_row(&mut a, &mut ctx, 0, 2000);

    ctrl_key(&mut a, &ctx, u32::from(b'f'), false);
    assert_eq!(holders(&a), FIELD, "Ctrl+F");

    tab(&mut a, &ctx, true);
    assert_eq!(holders(&a), PAGE, "Shift+Tab off the field");
    let last = a.sheet.open().focus().unwrap();
    assert_eq!(
        a.sheet.open().items[last].id,
        propsheet::CANCEL,
        "the page's last stop (Apply is greyed)"
    );

    let tab_sym = keysym::TAB;
    ctrl_key(&mut a, &ctx, tab_sym, true);
    ctrl_key(&mut a, &ctx, u32::from(b'f'), false);
    assert_eq!(holders(&a), FIELD, "Ctrl+F on the page before Keyboard");
    assert_ne!(
        a.sheet.tabs.open,
        Page::Keyboard.index(),
        "Ctrl+Shift+Tab leaves Keyboard"
    );
    ctrl_key(&mut a, &ctx, tab_sym, false);
    assert_eq!(a.sheet.tabs.open, Page::Keyboard.index());
    assert_eq!(holders(&a), SIDE_LIST, "Ctrl+Tab from the field");

    a.open_page(Page::DefaultPrograms, None);
    assert_eq!(holders(&a), SIDE_LIST, "Default Programs opens on its list");
    press_control(&mut a, &mut ctx, defaultspage::TERMINAL_ARGS, BTN_RIGHT);
    assert_eq!(holders(&a), PAGE, "a right press in the arguments field");
}

#[test]
fn a_side_list_never_fires_the_pages_access_keys() {
    let alt = Mods {
        alt: true,
        ..Mods::default()
    };
    for (page, letter) in [(Page::Startup, "r"), (Page::Keyboard, "d")] {
        let (mut a, mut ctx) = (app("side-list-letters"), test_ctx());
        a.open_page(page, None);
        assert_eq!(holders(&a), SIDE_LIST, "{page:?}");
        typed(&mut a, &mut ctx, letter);
        assert_eq!(
            holders(&a),
            SIDE_LIST,
            "{page:?}: {letter} stays on the list"
        );
        a.key(
            key_input(u32::from(b'a'), alt, Some("a")),
            &ctx.forms().controls(),
        );
        assert_eq!(holders(&a), PAGE, "{page:?}: Alt+A takes it to the page");
    }
}

/// Privacy's permission list sits mid-form, after its check: the field and the list hand the
/// keyboard over as on the other pages, but opening the page never gives it to the list.
#[test]
fn the_search_field_and_the_permission_list_never_hold_the_keyboard_together() {
    let mut a = app("search-location-list");
    let mut ctx = test_ctx();
    a.open_page(Page::Privacy, None);
    a.permissions_due = false;
    pages::fill_permissions(&mut a.permissions, &[("Maps".into(), "Location", true)]);

    press_search_field(&mut a, &mut ctx);
    assert_eq!(holders(&a), FIELD, "a press in the field");
    press_side_row(&mut a, &mut ctx, 0, 1000);
    assert_eq!(holders(&a), SIDE_LIST, "a press on the permission list");

    press_search_field(&mut a, &mut ctx);
    assert_eq!(
        holders(&a),
        FIELD,
        "a press in the field takes it from the list"
    );

    let tab_sym = keysym::TAB;
    ctrl_key(&mut a, &ctx, tab_sym, true);
    assert_ne!(registry::open(&a.sheet), Page::Privacy);
    ctrl_key(&mut a, &ctx, u32::from(b'f'), false);
    assert_eq!(holders(&a), FIELD, "Ctrl+F on the page before Privacy");
    ctrl_key(&mut a, &ctx, tab_sym, false);
    assert_eq!(registry::open(&a.sheet), Page::Privacy);
    assert!(
        !holders(&a)[1],
        "Ctrl+Tab from the field: the list is not the page's first stop"
    );

    ctrl_key(&mut a, &ctx, u32::from(b'f'), false);
    assert_eq!(holders(&a), FIELD, "Ctrl+F on Privacy");
    tab(&mut a, &ctx, true);
    assert_eq!(holders(&a), PAGE, "Shift+Tab off the field");
    assert_eq!(
        focused_id(&a),
        Some(propsheet::CANCEL),
        "Privacy's last stop (Apply is greyed)"
    );
}

#[test]
fn a_control_a_reader_focuses_takes_the_keyboard_from_the_side_list() {
    let (mut a, mut ctx) = (app("reader-focus"), test_ctx());
    ctx.size = (1920, 1080);
    a.open_page(Page::Keyboard, None);
    assert_eq!(holders(&a), SIDE_LIST);
    let r = a.sheet.open().rect(pages::SC_ADD).expect("Add...");
    let at = (r.x + r.width / 2, r.y + r.height / 2);
    assert!(a.reader(MAIN, repriseapp::Reader::Focus(at), &ctx));
    assert_eq!(holders(&a), PAGE);
    assert_eq!(focused_id(&a), Some(pages::SC_ADD));
}

fn send(a: &mut SettingsApp, ctx: &mut Ctx, ev: PointerInput) {
    a.event(MAIN, Event::Pointer(ev), ctx);
}

/// A left press at (`x`, `y`), then a motion and the release at `to`, or where it pressed.
fn left_drag(a: &mut SettingsApp, ctx: &mut Ctx, (x, y): (i32, i32), to: Option<(i32, i32)>) {
    let (button, mods) = (BTN_LEFT, Mods::default());
    let time = 0;
    send(
        a,
        ctx,
        PointerInput::Press {
            x,
            y,
            button,
            time,
            mods,
        },
    );
    let (x, y) = to.unwrap_or((x, y));
    send(a, ctx, PointerInput::Motion { x, y });
    send(a, ctx, PointerInput::Release { x, y, button, mods });
}

fn top(a: &mut SettingsApp) -> usize {
    a.side_list().expect("a side list").0.top
}

#[test]
fn the_shortcut_list_thumb_drags_to_the_end_and_the_track_pages_back() {
    let mut a = app("side-list-thumb");
    let mut ctx = test_ctx();
    ctx.size = (1000, 640);
    a.open_page(Page::Keyboard, None);
    send(&mut a, &mut ctx, PointerInput::Motion { x: 0, y: 0 });
    let cm = ctx.forms().controls();
    let (list, rect, _) = a.side_list().expect("the shortcut list");
    let lay = listview::layout(list, rect, &cm);
    let sb = lay.scrollbar.expect("the shortcuts overflow the list");
    let p = scrollbar_parts(sb, 1, 0, lay.rows, list.slots(), false, lay.thumb_min);
    let thumb = p.thumb.expect("a thumb");
    let (x, y) = (thumb.x + 2, thumb.y + 3);

    left_drag(&mut a, &mut ctx, (x, y), Some((x + 40, sb.bottom())));
    let end = top(&mut a);
    assert!(
        end > lay.rows,
        "the drag off the bar's side reaches the end"
    );
    send(&mut a, &mut ctx, PointerInput::Motion { x, y: sb.y });
    assert_eq!(top(&mut a), end, "the release lets go of the thumb");
    let (bx, by) = (lay.body.x + 8, lay.body.y + 8);
    send(
        &mut a,
        &mut ctx,
        PointerInput::Scroll {
            x: bx,
            y: by,
            rows: 3,
        },
    );
    assert_eq!(top(&mut a), end, "nothing lies past the end");

    left_drag(&mut a, &mut ctx, (x, sb.y + cm.scroll + 2), None);
    assert_eq!(top(&mut a), end - lay.rows, "the track pages back");
    send(
        &mut a,
        &mut ctx,
        PointerInput::Scroll {
            x: bx,
            y: by,
            rows: -1,
        },
    );
    assert_eq!(top(&mut a), end - lay.rows - 1, "the wheel scrolls a row");
}
