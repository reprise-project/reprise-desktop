use super::*;

/// A resize lays every page out afresh and keeps what the controls hold; the size is kept for the next run unless maximized.
#[test]
fn the_window_resizes_and_its_size_is_kept() {
    let mut app = app("resize");
    assert!(!app.fixed());
    let min = app.min_size(MAIN).unwrap();
    assert_eq!(min, (544, 528));
    assert_eq!(app.sheet.size, min, "built at its smallest");
    let (w, h) = winstate::DEFAULT;
    assert!(
        w * 10 >= min.0 * 13 && h * 10 >= min.1 * 12,
        "opens noticeably larger"
    );
    let state = tmp("resize-state").join("settings.toml");
    app.state_path = Some(state.clone());
    let startup = Page::Startup.index();
    let add = |app: &SettingsApp| app.sheet.pages[startup].item(pages::ADD).unwrap().rect;
    let before = add(&app);
    let windows = Page::WindowBehavior.index();
    let tabs = !app.sheet.pages[windows].checked("window_tabs").unwrap();
    app.sheet.pages[windows].set_check("window_tabs", tabs);

    let mut ctx = test_ctx();
    ctx.size = (900, 700);
    app.event(MAIN, Event::Resized, &mut ctx);
    assert_eq!(app.sheet.size, (900, 700));
    let after = add(&app);
    assert_eq!(
        (after.x, after.y),
        (before.x + 356, before.y + 172),
        "Add... keeps to the longer, wider list's corner"
    );
    assert_eq!(app.sheet.pages[windows].checked("window_tabs"), Some(tabs));
    app.event(MAIN, Event::Focus(false), &mut ctx);
    assert_eq!(winstate::load(&state, min, 100), (900, 700), "kept");

    ctx.size = (1900, 1000);
    ctx.maximized = true;
    app.event(MAIN, Event::Resized, &mut ctx);
    assert_eq!(app.sheet.size, (1900, 1000));
    app.event(MAIN, Event::Focus(false), &mut ctx);
    assert_eq!(
        winstate::load(&state, min, 100),
        (900, 700),
        "not the screen's"
    );

    ctx.size = (300, 200);
    ctx.maximized = false;
    app.event(MAIN, Event::Resized, &mut ctx);
    assert_eq!(app.sheet.size, min, "no smaller than its smallest");
    assert_eq!(add(&app), before);
}

#[test]
fn a_window_shorter_than_the_sheet_keeps_the_open_page_in_the_list() {
    let mut app = app("short-list");
    app.sheet.tabs.open = Page::Advanced.index();
    app.sheet.reveal_open();
    let mut ctx = test_ctx();
    let min = app.min_size(MAIN).unwrap();
    ctx.size = (min.0, 300);
    app.event(MAIN, Event::Resized, &mut ctx);
    let lay = app
        .sheet
        .layout(&ctx.text, ctx.theme.fonts.size_px)
        .list
        .expect("list mode");
    assert!(lay.scrollbar.is_some(), "the list overflows");
    let propsheet::Nav::List(list) = &app.sheet.nav else {
        panic!("list mode");
    };
    let i = list
        .rows
        .iter()
        .position(|r| *r == reprisekit::controls::pagelist::Row::Page(Page::Advanced.index()))
        .unwrap();
    let (row, inner) = (lay.rows[i], lay.inner);
    assert!(
        row.y >= inner.y && row.bottom() <= inner.bottom(),
        "Advanced's row {row:?} shows inside {inner:?}"
    );
}

/// At 200% on a 1080-line screen the window opens at its pick held to the work area; that
/// size is not kept, so the size left at 100% stands. A size the user gives it is kept.
#[test]
fn a_held_opening_size_is_not_kept() {
    let mut app = app("held");
    let min = app.min_size(MAIN).unwrap();
    let state = tmp("held-state").join("settings.toml");
    app.state_path = Some(state.clone());
    winstate::save(&state, (900, 700), 100);
    let mut ctx = test_ctx();
    ctx.text_size = 200;
    ctx.size = (1088, 1026);
    ctx.at_pick = true;
    app.event(MAIN, Event::Resized, &mut ctx);
    app.event(MAIN, Event::Focus(false), &mut ctx);
    assert_eq!(winstate::load(&state, min, 100), (900, 700));

    ctx.size = (1200, 1000);
    ctx.at_pick = false;
    app.event(MAIN, Event::Resized, &mut ctx);
    app.event(MAIN, Event::Focus(false), &mut ctx);
    assert_eq!(
        winstate::kept(&state),
        winstate::Kept {
            size: (1200, 1000),
            text_size: Some(200)
        }
    );
}

#[test]
fn at_150_percent_the_window_holds_the_grown_sheet() {
    let mut app = app("resize-150");
    let mut ctx = test_ctx();
    ctx.text_size = 150;
    let u = TextUnits::new(17, 11);
    assert_eq!(ctx.forms().text, u);
    ctx.size = (1000, 900);
    app.event(MAIN, Event::Resized, &mut ctx);
    assert_eq!(app.min_size(MAIN), Some(u.size((544, 528))));
    let (w, h) = app.sheet.size;
    assert_eq!(app.sheet.client(), u.size((w, h)));
    assert!(
        u.len(w) <= 1000 && u.len(w + 1) > 1000,
        "{w} is the widest that fits"
    );
    assert!(
        u.len(h) <= 900 && u.len(h + 1) > 900,
        "{h} is the tallest that fits"
    );

    ctx.size = (300, 200);
    app.event(MAIN, Event::Resized, &mut ctx);
    assert_eq!(app.sheet.size, (544, 528), "no smaller than its smallest");

    app.rename = Some(renamedlg::RenameDialog::new("Name"));
    let rename = app.children().into_iter().find(|c| c.id == RENAME_WIN);
    assert_eq!(rename.map(|c| c.size), Some(u.size(renamedlg::CLIENT)));
}

/// The size Settings opens at follows the text size live: its smallest sheet at 200%, the
/// opening size again back at 100%, whatever size the window took in between.
#[test]
fn the_opening_size_follows_the_text_size() {
    let mut app = app("opening-size");
    app.kept = winstate::Kept {
        size: winstate::DEFAULT,
        text_size: None,
    };
    let mut ctx = test_ctx();
    ctx.text_size = 200;
    ctx.size = (1088, 1026);
    app.event(MAIN, Event::Resized, &mut ctx);
    assert_eq!(app.initial_size(), (1088, 1056));
    ctx.text_size = 100;
    app.event(MAIN, Event::Resized, &mut ctx);
    assert_eq!(app.initial_size(), winstate::DEFAULT);
}

/// The client a resizable window gets at 200% on a 1920x1080 output under `i`, its taskbar
/// along the bottom at the interface's height: the work area less the frame and caption.
fn client_at_200(i: Interface) -> (i32, i32) {
    let m = reprisekit::interfaces::theme_for(i)
        .with_text_size(200)
        .metrics;
    FrameGeometry::client_size_for_total(&m, 1920, 1080 - m.taskbar_height, false, Chrome::WINDOW)
}

/// At 200% on a 1080-line screen the work area is shorter than the smallest sheet under every
/// interface, and every page, the Remote Desktop page under the login manager too, still ends
/// above the window's bottom with its buttons inside it.
#[test]
fn at_200_percent_a_window_shorter_than_the_sheet_keeps_its_buttons_in_it() {
    let all = [Interface::Classic, Interface::Blue, Interface::Glass];
    assert_eq!(
        all.map(|i| client_at_200(i).1),
        [1000, 990, 976],
        "Classic, Blue, Glass"
    );
    for i in all {
        let dir = tmp(&format!("resize-200-{}", i.as_str()));
        let path = dir.join("config.toml");
        let config = format!("[appearance]\ninterface = \"{}\"\n", i.as_str());
        std::fs::write(&path, config).unwrap();
        let mut app = SettingsApp::open(path, "us".into());
        let mut ctx = test_ctx();
        ctx.theme = reprisekit::interfaces::theme_for(i);
        ctx.text_size = 200;
        let u = ctx.forms().text;
        let (w, h) = client_at_200(i);
        assert!(
            u.len(528) > h,
            "the smallest sheet is taller than the work area"
        );
        ctx.size = (w, h);
        app.event(MAIN, Event::Resized, &mut ctx);
        assert_eq!(app.min_size(MAIN), Some(u.size((544, 528))));
        assert_eq!(
            app.sheet.size.1, 528,
            "the pages keep their smallest height"
        );
        let client = app.sheet.client();
        assert!(client.1 <= h, "{i:?}: {client:?} fits the window");
        let mut c = Canvas::new(w as u32, h as u32);
        let mut fits = |app: &mut SettingsApp, pg: Page, managed: bool| {
            app.sheet.tabs.open = pg.index();
            app.draw(MAIN, &mut c, &ctx);
            let l = app.sheet.layout(&ctx.text, ctx.forms().fonts.size_px);
            assert_eq!(
                l.bar, None,
                "{i:?}: {pg:?} (managed {managed}) ends above the window's bottom"
            );
            for id in [propsheet::OK, propsheet::CANCEL, propsheet::APPLY] {
                let b = app.sheet.open().rect(id).unwrap();
                assert!(b.bottom() <= h, "{i:?}: {id} at {b:?} shows");
            }
        };
        for pg in Page::all() {
            fits(&mut app, pg, false);
        }
        let managed = remotehost::Remote {
            manager: true,
            ..remotehost::Remote::default()
        };
        remote_read(&mut app, managed, None);
        assert!(pages::managed(
            &app.sheet.pages[Page::RemoteDesktop.index()]
        ));
        fits(&mut app, Page::RemoteDesktop, true);
    }
}

/// `--focus` names a control before the window has a size: once the window comes up shorter
/// than the sheet at 200%, the page scrolls to show the control.
#[test]
fn a_named_control_shows_once_a_short_window_cuts_its_page() {
    let mut app = app("focus-short");
    app.open_page(Page::Advanced, Some(pages::EDIT_FILE));
    let mut ctx = test_ctx();
    ctx.text_size = 200;
    ctx.size = (1914, 700);
    app.event(MAIN, Event::Resized, &mut ctx);
    let mut c = Canvas::new(1914, 700);
    app.draw(MAIN, &mut c, &ctx);
    let l = app.sheet.layout(&ctx.text, ctx.forms().fonts.size_px);
    assert!(l.bar.is_some(), "the page scrolls");
    assert_eq!(focused_id(&app), Some(pages::EDIT_FILE));
    let r = app.sheet.open().rect(pages::EDIT_FILE).unwrap();
    assert!(
        r.y >= l.page.y && r.bottom() <= l.page.bottom(),
        "{r:?} shows inside {:?}",
        l.page
    );
}

/// At 200% on a screen 1000 px wide the smallest sheet is wider than the window: the
/// buttons keep to the window's corner, and the page scrolls sideways to its right column.
#[test]
fn at_200_percent_a_window_narrower_than_the_sheet_scrolls_its_page_sideways() {
    let mut app = app("narrow-200");
    let mut ctx = test_ctx();
    ctx.text_size = 200;
    ctx.size = (1000, 1026);
    app.sheet.tabs.open = Page::Background.index();
    app.event(MAIN, Event::Resized, &mut ctx);
    let mut c = Canvas::new(1000, 1026);
    app.draw(MAIN, &mut c, &ctx);
    let client = app.sheet.client();
    assert!(client.0 <= 1000, "{client:?} fits the window");
    let l = app.sheet.layout(&ctx.text, ctx.forms().fonts.size_px);
    assert!(l.apply.right() <= 1000, "Apply at {:?} shows", l.apply);
    let hbar = l.hbar.expect("the page scrolls sideways");
    let browse = app.sheet.open().rect(backgroundpage::BROWSE).unwrap();
    assert!(
        browse.right() > hbar.right(),
        "Browse... starts past the page's edge"
    );
    app.sheet
        .wheel(Some(propsheet::Hit::PageHBar(BarHit::Track)), 100);
    let browse = app.sheet.open().rect(backgroundpage::BROWSE).unwrap();
    let bar = l.bar.map_or(hbar.right(), |b| b.x);
    assert!(browse.right() <= bar, "{browse:?} shows");
}

/// On a page cut narrow and short the Keyboard page's shortcut list runs on under the page's
/// scrollbar: a press on its arrow there scrolls the page, and the list keeps its selection.
#[test]
fn the_page_scrollbar_over_the_side_list_scrolls_the_page() {
    let mut app = app("bar-over-list");
    let mut ctx = test_ctx();
    ctx.text_size = 200;
    ctx.size = (1000, 700);
    app.sheet.tabs.open = Page::Keyboard.index();
    app.event(MAIN, Event::Resized, &mut ctx);
    let mut c = Canvas::new(1000, 700);
    app.draw(MAIN, &mut c, &ctx);
    let bar = app.sheet.layout(&ctx.text, ctx.forms().fonts.size_px).bar;
    let bar = bar.expect("the page scrolls");
    let list = app.on_page(pages::shortcuts(app.area()));
    let y = bar.bottom() - 4;
    assert!(
        list.contains(bar.x + 2, y),
        "{list:?} runs under the down arrow of {bar:?}"
    );
    let selected = app.shortcuts.selection.focus;
    let press = PointerInput::Press {
        x: bar.x + 2,
        y,
        button: BTN_LEFT,
        time: 0,
        mods: Mods::default(),
    };
    app.event(MAIN, Event::Pointer(press), &mut ctx);
    assert!(app.sheet.page_offset().1 < 0, "the page scrolled down");
    assert_eq!(app.shortcuts.selection.focus, selected);
}
