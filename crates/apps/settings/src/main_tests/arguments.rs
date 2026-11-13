use super::*;

/// The server's state, read: Remote Desktop on, which lights the page's controls.
fn remote_on(a: &mut SettingsApp) {
    let (tx, rx) = std::sync::mpsc::channel();
    let remote = remotehost::Remote {
        enabled: true,
        ..remotehost::Remote::default()
    };
    tx.send(remotehost::Reading {
        remote,
        failed: None,
    })
    .unwrap();
    a.remote_job = Some(rx);
    assert!(a.wake().redraw);
}

/// Whether control `id` shows whole inside the open page, which scrolls in `ctx`'s window.
fn shows_at_200(a: &mut SettingsApp, ctx: &Ctx, id: &str) -> bool {
    let mut c = Canvas::new(ctx.size.0 as u32, ctx.size.1 as u32);
    a.draw(MAIN, &mut c, ctx);
    let l = a.sheet.layout(&ctx.text, ctx.forms().fonts.size_px);
    let r = a.sheet.open().rect(id).unwrap();
    l.bar.is_some() && r.y >= l.page.y && r.bottom() <= l.page.bottom()
}

/// The Remote Desktop page's controls are greyed until the server is read, which may land before
/// or after the window has its size.
#[test]
fn a_named_control_greyed_until_its_page_is_read_takes_the_keyboard_once_lit() {
    let border = "remote_border";
    let mut ctx = test_ctx();
    ctx.text_size = 200;
    ctx.size = (1914, 700);

    let mut a = app("focus-remote");
    a.open_page(Page::RemoteDesktop, Some(border));
    assert_ne!(focused_id(&a), Some(border), "greyed, it waits");
    a.event(MAIN, Event::Resized, &mut ctx);
    assert_ne!(focused_id(&a), Some(border), "still greyed");
    remote_on(&mut a);
    assert!(shows_at_200(&mut a, &ctx, border));
    assert_eq!(focused_id(&a), Some(border));

    let mut b = app("focus-remote-early");
    b.open_page(Page::RemoteDesktop, Some(border));
    remote_on(&mut b);
    b.event(MAIN, Event::Resized, &mut ctx);
    assert_eq!(
        focused_id(&b),
        Some(border),
        "read before the window had its size"
    );
    assert!(shows_at_200(&mut b, &ctx, border));
}

#[test]
fn a_key_before_the_read_drops_the_named_control() {
    let mut a = app("focus-remote-key");
    let mut ctx = test_ctx();
    a.open_page(Page::RemoteDesktop, Some("remote_border"));
    a.event(MAIN, Event::Resized, &mut ctx);
    let tab = key_input(keysym::TAB, Mods::default(), None);
    a.event(MAIN, Event::Key(tab), &mut ctx);
    let after_key = focused_id(&a);
    assert_ne!(after_key, Some("remote_border"));
    remote_on(&mut a);
    let mut c = Canvas::new(10, 10);
    a.draw(MAIN, &mut c, &ctx);
    assert_eq!(focused_id(&a), after_key);
}

#[test]
fn the_page_argument_names_a_page_or_a_program() {
    let args = |v: &[&str]| page_arg(v.iter().map(|s| s.to_string())).map(|a| a.0);
    assert_eq!(
        args(&["--page", "night-light"]),
        Some(Entry::Page(Page::NightLight))
    );
    assert_eq!(
        args(&["--page=fonts-and-icons"]),
        Some(Entry::Page(Page::FontsAndIcons))
    );
    assert_eq!(
        args(&["--page", "power"]),
        Some(Entry::Program("reprise-power.desktop"))
    );
    assert_eq!(args(&["--page", "nowhere"]), None);
    assert_eq!(args(&[]), None);
}

#[test]
fn focus_on_a_control_that_left_power_opens_its_page() {
    let args = |v: &[&str]| page_arg(v.iter().map(|s| s.to_string())).map(|a| a.0);
    assert_eq!(
        args(&["--page", "power", "--focus", "standby"]),
        Some(Entry::Page(Page::Advanced))
    );
    assert_eq!(
        args(&["--page", "power", "--focus", "low_space_percent"]),
        Some(Entry::Page(Page::Advanced))
    );
    assert_eq!(
        args(&["--page", "power", "--focus", "lock_after"]),
        Some(Entry::Page(Page::ScreenSaverLock))
    );
    assert_eq!(
        args(&["--page", "power", "--focus", "screen_off_after"]),
        Some(Entry::Program("reprise-power.desktop")),
        "screen off is Power Options' own"
    );
}

#[test]
fn export_and_import_take_a_file_and_run_without_a_window() {
    let args = |v: &[&str]| backup_arg(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>());
    let file = || PathBuf::from("/home/u/s.toml");
    assert_eq!(
        args(&["--export", "/home/u/s.toml"]),
        Some(BackupVerb::Export(file()))
    );
    assert_eq!(
        args(&["--import=/home/u/s.toml"]),
        Some(BackupVerb::Import(file()))
    );
    assert_eq!(args(&["--export"]), None, "no file");
    assert_eq!(args(&["--page", "backup"]), None);
}

#[test]
fn the_focus_argument_names_a_control_on_the_page() {
    let args = |v: &[&str]| page_arg(v.iter().map(|s| s.to_string()));
    let mouse_keys = Some((
        Entry::Page(Page::PointerAlerts),
        Some("mouse_keys".to_string()),
    ));
    assert_eq!(
        args(&["--page", "accessibility", "--focus", "mouse_keys"]),
        mouse_keys
    );
    assert_eq!(
        args(&["--focus=mouse_keys", "--page=accessibility"]),
        mouse_keys
    );
    assert_eq!(args(&["--focus", "mouse_keys"]), None, "no page, no focus");
    assert_eq!(
        args(&["--page", "pointer-and-alerts", "--focus", "mouse_keys"]),
        mouse_keys
    );
}

#[test]
fn focus_on_a_moved_control_opens_its_new_page() {
    let args = |v: &[&str]| page_arg(v.iter().map(|s| s.to_string()));
    assert_eq!(
        args(&["--page", "text-and-keys", "--focus", "mouse_keys"]),
        Some((Entry::Page(Page::PointerAlerts), Some("mouse_keys".into())))
    );
    assert_eq!(
        args(&["--page", "advanced", "--focus", "clipboard_history"]),
        Some((Entry::Page(Page::Privacy), Some("clipboard_history".into())))
    );
    let mut a = app("moved-focus");
    a.open_page(Page::Privacy, Some("clipboard_history"));
    let page = a.sheet.open();
    assert_eq!(
        page.focus().map(|i| page.items[i].id),
        Some("clipboard_history")
    );
    assert_eq!(
        args(&["--page", "sound", "--focus", "nowhere"]),
        Some((Entry::Page(Page::Sound), Some("nowhere".into()))),
        "an id no page holds leaves the page as named"
    );
}

#[test]
fn a_handed_old_name_opens_its_renamed_page() {
    let mut a = app("old-name");
    assert!(a.request(&page_request("vision")));
    assert_eq!(registry::open(&a.sheet), Page::MagnifierFilters);
}

#[test]
fn a_request_with_a_control_opens_its_page_with_the_keyboard_on_it() {
    let mut a = app("focus-request");
    let req = |page: &str, id: &str| instance::Request {
        action: Some((PAGE_ACTION.into(), vec![page.into(), id.into()])),
        handoff: Default::default(),
    };
    let focused = |a: &SettingsApp| {
        let page = a.sheet.open();
        page.focus().map(|i| page.items[i].id)
    };
    assert!(a.request(&req("accessibility", "slow_keys")));
    assert_eq!(a.sheet.tabs.open, Page::TextKeys.index());
    assert!(
        !a.sheet.tabs.focused,
        "the page strip lets go of the keyboard"
    );
    assert_eq!(focused(&a), Some("slow_keys"));
    assert!(
        a.request(&req("accessibility", "sticky_keys")),
        "the open page still moves the focus"
    );
    assert_eq!(focused(&a), Some("sticky_keys"));
}

#[test]
fn a_page_handed_a_missing_control_opens_as_it_would_without_one() {
    let mut a = app("open-missing");
    a.open_page(Page::Keyboard, Some("missing"));
    assert_eq!(holders(&a), SIDE_LIST);
}
