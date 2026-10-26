//! The window's tests: tabs, keys and the pointer over real PTYs, kept out of `main.rs`.

use super::*;
use alacritty_terminal::term::TermMode;
use repriseapp::{BTN_LEFT, Extended, Mods, PointerInput};
use reprisekit::textedit::keysym;
use std::path::Path;

fn args(a: &[&str]) -> impl Iterator<Item = OsString> {
    a.iter().map(OsString::from).collect::<Vec<_>>().into_iter()
}

#[test]
fn everything_after_dash_e_is_the_command() {
    assert_eq!(command(args(&[])), None);
    assert_eq!(command(args(&["-e"])), None);
    assert_eq!(
        command(args(&["-e", "top", "-d", "1"])),
        Some(vec!["top".into(), "-d".into(), "1".into()])
    );
    assert_eq!(
        command(args(&["-e", "sh", "-e", "x"])),
        Some(vec!["sh".into(), "-e".into(), "x".into()]),
        "a later -e belongs to the program"
    );
}

fn test_ctx() -> Ctx {
    Ctx {
        theme: Theme::default(),
        text: TextRenderer::load(&FontConfig::default()).into(),
        icons: reprisekit::icontheme::IconCache::new(reprisekit::icontheme::IconTheme::load(
            "test",
            &[],
        )),
        caret_on: true,
        size: (800, 600),
        maximized: false,
        origin: (0, 0),
        wake: repriseapp::Waker::noop(),
        heads: Default::default(),
        want_tick: Default::default(),
        output_size: None,
        at_pick: false,
        text_size: 100,
    }
}

fn argv(a: &[&str]) -> Vec<OsString> {
    a.iter().map(OsString::from).collect()
}

/// Where the window tests keep their state and settings: nowhere, so a quit saves nothing.
const NOWHERE: &str = "/nonexistent/reprise-terminal-test";

/// A started terminal whose first tab runs `cmd` and whose shell is `/bin/sh`.
fn terminal(cmd: &[&str]) -> Terminal {
    let settings = Settings {
        shell: "/bin/sh".into(),
        ..Settings::default()
    };
    let nowhere = Path::new(NOWHERE);
    let mut t = Terminal::with_state(settings, Some(argv(cmd)), nowhere.join("state.toml"));
    t.config = nowhere.join("terminal.toml");
    t.start(repriseapp::Waker::noop());
    t
}

fn launch(cmd: &[&str]) -> Launch {
    Launch {
        argv: Some(argv(cmd)),
        name: None,
    }
}

/// Wait up to 5 s for `f`.
fn until(what: &str, mut f: impl FnMut() -> bool) {
    for _ in 0..500 {
        if f() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("timed out waiting until {what}");
}

fn key(sym: u32, ctrl: bool, shift: bool) -> Event {
    Event::Key(KeyInput {
        sym,
        utf8: None,
        mods: Mods {
            ctrl,
            shift,
            alt: false,
        },
        edit: None,
    })
}

fn centre(r: Rect) -> (i32, i32) {
    (r.x + r.width / 2, r.y + r.height / 2)
}

fn press(x: i32, y: i32, button: u32) -> Event {
    Event::Pointer(PointerInput::Press {
        x,
        y,
        button,
        time: 0,
        mods: Mods::default(),
    })
}

fn pids(t: &Terminal) -> Vec<u32> {
    t.w.tabs.list.iter().map(Tab::pid).collect()
}

#[test]
fn resize_reflows_all_tabs() {
    let mut ctx = test_ctx();
    let mut t = terminal(&["yes"]);
    t.spawn_tab(&launch(&["yes"]), None);
    t.spawn_tab(&launch(&["cat"]), None);
    for size in [(800, 600), (300, 200), (1000, 700), (120, 90)] {
        ctx.size = size;
        t.event(MAIN, Event::Resized, &mut ctx);
        t.event(MAIN, Event::Wake, &mut ctx);
        let m = termgrid::metrics(&t.font, t.px, 1);
        let (cols, rows) = termgrid::grid_size(&m, chrome::grid_rect(&ctx, false), 1);
        let want = (cols.max(2), rows.max(1));
        for tab in &t.w.tabs.list {
            assert_eq!(
                tab.size(),
                want,
                "every tab takes the {size:?} window's grid"
            );
        }
    }
    let mut c = Canvas::new(120, 90);
    t.draw(MAIN, &mut c, &ctx);
}

#[test]
fn exit_at_once_closes() {
    let mut ctx = test_ctx();
    let mut t = terminal(&["false"]);
    let scheme = t.scheme(&ctx.theme);
    let mut reaped = Vec::new();
    until("false exits", || {
        t.w.tabs.list.iter_mut().for_each(|tab| {
            tab.pump(&scheme);
        });
        reaped = t.reap();
        !reaped.is_empty()
    });
    assert_eq!(reaped, [("false".to_string(), 1)]);
    assert!(t.event(MAIN, Event::Wake, &mut ctx).quit);
}

#[test]
fn scale_change_resizes_pty() {
    let mut ctx = test_ctx();
    let mut t = terminal(&["cat"]);
    t.follow_scale(MAIN, 1, &mut ctx);
    let one = t.w.tabs.list[0].size();
    t.follow_scale(MAIN, 2, &mut ctx);
    assert_eq!(t.metrics.0, (2, t.px));
    let m = termgrid::metrics(&t.font, t.px, 2);
    let want = termgrid::grid_size(&m, chrome::grid_rect(&ctx, false), 2);
    assert_eq!(t.metrics.1, m, "the cell is taken again at scale 2");
    assert_eq!(t.w.fitted, Some((want, (m.cell_w, m.cell_h))));
    assert_eq!(t.w.tabs.list[0].size(), want);
    assert!(one.0 > 0 && want.0 > 0);
}

#[test]
fn new_tab_inherits_cwd() {
    let mut ctx = test_ctx();
    let mut t = terminal(&["/bin/sh", "-c", "cd / && exec cat"]);
    until("the first tab is in /", || {
        t.w.tabs.current().and_then(Tab::cwd).as_deref() == Some(Path::new("/"))
    });
    assert_eq!(
        t.new_window(),
        (argv(&["reprise-terminal"]), Some("/".into()))
    );
    t.event(MAIN, key(0x54, true, true), &mut ctx);
    assert_eq!(t.w.tabs.list.len(), 2, "Ctrl+Shift+T opens a tab");
    assert_eq!(t.w.tabs.strip.open, 1);
    until("the new tab starts in /", || {
        t.w.tabs.list[1].cwd().as_deref() == Some(Path::new("/"))
    });

    let osc7 = "printf '\\033]7;file://localhost/usr\\007'; exec cat";
    t.spawn_tab(&launch(&["/bin/sh", "-c", osc7]), None);
    until("the shell reports /usr", || {
        t.w.tabs.list[2].cwd().as_deref() == Some(Path::new("/usr"))
    });
    assert_ne!(
        cwd::cwd_of(t.w.tabs.list[2].pid()).as_deref(),
        Some(Path::new("/usr"))
    );
    t.new_tab(&Choice::Shell);
    until("the tab opened from it starts in /usr", || {
        t.w.tabs.list[3].cwd().as_deref() == Some(Path::new("/usr"))
    });
}

#[test]
fn shortcuts_switch_close_and_zoom() {
    let mut ctx = test_ctx();
    let mut t = terminal(&["cat"]);
    t.event(MAIN, key(0x54, true, true), &mut ctx);
    assert_eq!(t.w.tabs.strip.open, 1);
    t.event(MAIN, key(0xff09, true, false), &mut ctx);
    assert_eq!(t.w.tabs.strip.open, 0, "Ctrl+Tab wraps round");
    t.event(MAIN, key(0xff55, true, false), &mut ctx);
    assert_eq!(t.w.tabs.strip.open, 1, "Ctrl+PgUp goes back");
    t.event(MAIN, key(0xff09, true, true), &mut ctx);
    assert_eq!(t.w.tabs.strip.open, 0, "Ctrl+Shift+Tab goes back");
    let px = t.px;
    t.event(MAIN, key(0x3d, true, false), &mut ctx);
    assert_eq!(t.px, px + 1);
    t.event(MAIN, key(0x2d, true, false), &mut ctx);
    t.event(MAIN, key(0x2d, true, false), &mut ctx);
    assert_eq!(t.px, px - 1);
    t.event(MAIN, key(0x30, true, false), &mut ctx);
    assert_eq!(t.px, px);
    assert!(t.event(MAIN, key(0x56, true, true), &mut ctx).paste);
    assert!(t.event(MAIN, Event::Pasted("x".into()), &mut ctx).redraw);
    assert!(!t.event(MAIN, key(0x57, true, true), &mut ctx).quit);
    assert_eq!(t.w.tabs.list.len(), 1);
    assert!(
        t.event(MAIN, key(0x57, true, true), &mut ctx).quit,
        "the last tab closes the window"
    );
}

#[test]
fn middle_click_closes_tab() {
    let mut ctx = test_ctx();
    let mut t = terminal(&["cat"]);
    t.spawn_tab(&launch(&["cat"]), None);
    t.spawn_tab(&launch(&["cat"]), None);
    let keep = [pids(&t)[0], pids(&t)[2]];
    let (x, y) = centre(t.chrome_layout(&ctx).strip.tabs[1]);
    t.event(MAIN, press(x, y, 0x112), &mut ctx);
    assert_eq!(pids(&t), keep, "tab 2 closed");
    assert!(t.confirm.is_none());
}

#[test]
fn closing_a_busy_tab_asks_first() {
    let mut ctx = test_ctx();
    let mut t = terminal(&["cat"]);
    t.spawn_tab(&launch(&["/bin/sh", "-i"]), None);
    t.w.tabs.list[1].write(b"sleep 30\n".as_slice().into());
    until("sleep holds the foreground", || t.w.tabs.list[1].busy());
    assert!(!t.w.tabs.list[0].busy(), "cat is the tab's own program");
    let (x, y) = centre(t.chrome_layout(&ctx).strip.tabs[1]);
    t.event(MAIN, press(x, y, 0x112), &mut ctx);
    assert_eq!(t.w.tabs.list.len(), 2, "nothing closes before the answer");
    assert_eq!(t.children().first().map(|c| c.id), Some(CONFIRM));
    t.event(CONFIRM, key(keysym::RETURN, false, false), &mut ctx);
    assert_eq!(t.w.tabs.list.len(), 1, "Close closes the tab");
    assert!(t.children().is_empty());

    t.spawn_tab(&launch(&["/bin/sh", "-i"]), None);
    t.w.tabs.list[1].write(b"sleep 30\n".as_slice().into());
    until("sleep holds the foreground", || t.w.tabs.list[1].busy());
    assert!(!t.event(MAIN, Event::CloseRequest, &mut ctx).quit);
    assert!(t.confirm.is_some(), "closing the window asks too");
    t.event(CONFIRM, key(keysym::ESCAPE, false, false), &mut ctx);
    assert!(t.confirm.is_none());
    assert_eq!(t.w.tabs.list.len(), 2, "Cancel keeps every tab");
}

#[test]
fn tab_drag_never_moves_window() {
    let mut ctx = test_ctx();
    let mut t = terminal(&["cat"]);
    t.spawn_tab(&launch(&["cat"]), None);
    let before = pids(&t);
    let lay = t.chrome_layout(&ctx);
    let (x0, y) = centre(lay.strip.tabs[0]);
    let x1 = lay.strip.tabs[1].right() + 10;
    let mut replies = vec![t.event(MAIN, press(x0, y, BTN_LEFT), &mut ctx)];
    for x in [x0 + 2, x0 + 30, x1] {
        replies.push(t.event(
            MAIN,
            Event::Pointer(PointerInput::Motion { x, y }),
            &mut ctx,
        ));
        assert!(
            t.w.tabs.strip.dragging.is_some() || x == x0 + 2,
            "dragging past the threshold"
        );
    }
    replies.push(t.event(
        MAIN,
        Event::Pointer(PointerInput::Release {
            x: x1,
            y,
            button: BTN_LEFT,
            mods: Mods::default(),
        }),
        &mut ctx,
    ));
    assert_eq!(pids(&t), [before[1], before[0]], "the tabs swapped");
    assert_eq!(t.w.tabs.strip.open, 1, "the dragged tab stays open");
    assert!(t.w.tabs.strip.dragging.is_none());
    for r in replies {
        assert!(!r.resize && r.start_drag.is_none() && !r.quit, "{r:?}");
    }
}

#[test]
fn the_add_button_and_its_menu_open_tabs() {
    let mut ctx = test_ctx();
    let mut t = terminal(&["cat"]);
    let lay = t.chrome_layout(&ctx);
    let (x, y) = centre(lay.strip.add);
    t.event(MAIN, press(x, y, BTN_LEFT), &mut ctx);
    let release = PointerInput::Release {
        x,
        y,
        button: BTN_LEFT,
        mods: Mods::default(),
    };
    t.event(MAIN, Event::Pointer(release), &mut ctx);
    assert_eq!(t.w.tabs.list.len(), 2, "+ opens the shell");
    let lay = t.chrome_layout(&ctx);
    let (x, y) = centre(lay.strip.menu);
    t.event(MAIN, press(x, y, BTN_LEFT), &mut ctx);
    assert!(t.w.chrome.menu.is_some(), "the arrow opens the menu");
    t.event(MAIN, key(keysym::DOWN, false, false), &mut ctx);
    t.event(MAIN, key(keysym::RETURN, false, false), &mut ctx);
    assert!(t.w.chrome.menu.is_none());
    assert_eq!(t.w.tabs.list.len(), 3, "its first row opens the shell");
    assert_eq!(t.w.tabs.list[2].program(), "sh");
}

#[test]
fn the_new_tab_menu_has_menu_item_nodes_and_focus_follows_the_highlight() {
    use reprisekit::access::accesskit::Role;
    let mut ctx = test_ctx();
    let mut t = terminal(&["cat"]);
    let lay = t.chrome_layout(&ctx);
    let (x, y) = centre(lay.strip.menu);
    t.event(MAIN, press(x, y, BTN_LEFT), &mut ctx);
    let mut tree = Tree::window("Terminal", ctx.size);
    t.access(MAIN, &mut tree, &ctx);
    let menu = tree.focus();
    assert_eq!(
        tree.get(menu).map(|n| n.role()),
        Some(Role::Menu),
        "the open menu takes the focus"
    );
    let row = tree
        .find(Role::MenuItem, "sh")
        .expect("the shell row is a menu item");
    t.event(MAIN, key(keysym::DOWN, false, false), &mut ctx);
    let mut tree = Tree::window("Terminal", ctx.size);
    t.access(MAIN, &mut tree, &ctx);
    assert_eq!(tree.focus(), row, "Down focuses the shell row");
    t.event(MAIN, key(keysym::ESCAPE, false, false), &mut ctx);
    let mut tree = Tree::window("Terminal", ctx.size);
    t.access(MAIN, &mut tree, &ctx);
    assert_eq!(
        tree.find(Role::MenuItem, "sh"),
        None,
        "a closed menu leaves no nodes"
    );
}

#[test]
fn closing_a_tab_mid_press_drops_the_press() {
    let mut ctx = test_ctx();
    let mut t = terminal(&["cat"]);
    t.spawn_tab(&launch(&["cat"]), None);
    t.spawn_tab(&launch(&["cat"]), None);
    let before = pids(&t);
    let (x, y) = centre(t.chrome_layout(&ctx).strip.tabs[2]);
    t.event(MAIN, press(x, y, BTN_LEFT), &mut ctx);
    t.event(MAIN, key(0x57, true, true), &mut ctx);
    assert_eq!(pids(&t), before[..2], "Ctrl+Shift+W closed the held tab");
    for x in [x - 2, x - 6, x - 200] {
        t.event(
            MAIN,
            Event::Pointer(PointerInput::Motion { x, y }),
            &mut ctx,
        );
    }
    assert!(t.w.tabs.strip.dragging.is_none(), "no tab is dragged");
    assert_eq!(pids(&t), before[..2], "no tab moved");

    let (x, y) = centre(t.chrome_layout(&ctx).strip.closes[1]);
    t.event(MAIN, press(x, y, BTN_LEFT), &mut ctx);
    t.event(MAIN, key(0x57, true, true), &mut ctx);
    let release = PointerInput::Release {
        x: centre(t.chrome_layout(&ctx).strip.closes[0]).0,
        y,
        button: BTN_LEFT,
        mods: Mods::default(),
    };
    t.event(MAIN, Event::Pointer(release), &mut ctx);
    assert_eq!(pids(&t), before[..1], "the release closes nothing more");
}

#[test]
fn a_second_close_request_keeps_the_first_question() {
    let mut ctx = test_ctx();
    let mut t = terminal(&["cat"]);
    t.spawn_tab(&launch(&["/bin/sh", "-i"]), None);
    t.w.tabs.list[1].write(b"sleep 30\n".as_slice().into());
    until("sleep holds the foreground", || t.w.tabs.list[1].busy());
    let pid = t.w.tabs.list[1].pid();
    t.event(MAIN, key(0x57, true, true), &mut ctx);
    assert!(matches!(t.confirm, Some((Asking::Tab(p), _)) if p == pid));
    t.event(MAIN, Event::CloseRequest, &mut ctx);
    assert!(
        matches!(t.confirm, Some((Asking::Tab(p), _)) if p == pid),
        "the window's close leaves the tab's question up"
    );
}

#[test]
fn the_settings_button_opens_options() {
    let mut ctx = test_ctx();
    let mut t = terminal(&["cat"]);
    t.follow_scale(MAIN, 1, &mut ctx);
    let lay = t.chrome_layout(&ctx);
    let (x, y) = centre(*lay.buttons.last().unwrap());
    t.event(MAIN, press(x, y, BTN_LEFT), &mut ctx);
    let release = PointerInput::Release {
        x,
        y,
        button: BTN_LEFT,
        mods: Mods::default(),
    };
    assert!(t.event(MAIN, Event::Pointer(release), &mut ctx).redraw_all);
    assert_eq!(t.children().first().map(|c| c.id), Some(OPTIONS));
}

fn themed_ctx(theme: Theme) -> Ctx {
    Ctx {
        theme,
        text: TextRenderer::load(&FontConfig::default()).into(),
        icons: reprisekit::icontheme::IconCache::new(reprisekit::icontheme::IconTheme::load(
            "test",
            &[],
        )),
        caret_on: true,
        size: (640, 400),
        maximized: false,
        origin: (0, 0),
        wake: repriseapp::Waker::noop(),
        heads: Default::default(),
        want_tick: Default::default(),
        output_size: None,
        at_pick: false,
        text_size: 100,
    }
}

fn in_dir(dir: &std::path::Path, settings: Settings) -> Terminal {
    let mut t = Terminal::with_state(settings, None, dir.join("state.toml"));
    t.config = dir.join("config.toml");
    t.loaded = mtime(&t.config);
    t
}

#[test]
fn see_through_solid_outside_glass() {
    let dir = tempfile::tempdir().unwrap();
    let t = in_dir(
        dir.path(),
        Settings {
            see_through: true,
            opacity: 50,
            ..Settings::default()
        },
    );
    for i in [Interface::Classic, Interface::Blue] {
        let ctx = themed_ctx(reprisekit::interfaces::theme_for(i));
        let mut t = in_dir(dir.path(), t.settings.clone());
        let mut c = Canvas::new(640, 400);
        t.draw(repriseapp::MAIN, &mut c, &ctx);
        assert_eq!(t.w.snap.alpha, 255, "{i:?} draws a solid background");
        assert_eq!(c.take_glass_top(), 0, "{i:?} shows no glass");
    }
    let ctx = themed_ctx(reprisekit::interfaces::theme_for(Interface::Glass));
    let mut t = in_dir(dir.path(), t.settings.clone());
    let mut c = Canvas::new(640, 400);
    t.draw(repriseapp::MAIN, &mut c, &ctx);
    assert_eq!(t.w.snap.alpha, 127, "50% opacity");
    assert_eq!(c.take_glass_top(), 400, "the whole client is glass");
    t.settings.see_through = false;
    t.draw(repriseapp::MAIN, &mut c, &ctx);
    assert_eq!(t.w.snap.alpha, 255);
    assert_eq!(
        c.take_glass_top(),
        chrome::band_height(&ctx.theme, false),
        "the chrome's band stays glass"
    );
}

#[test]
fn the_toolbar_band_is_opaque_outside_glass() {
    let dir = tempfile::tempdir().unwrap();
    for i in [Interface::Classic, Interface::Blue] {
        let ctx = themed_ctx(reprisekit::interfaces::theme_for(i));
        let mut t = in_dir(dir.path(), Settings::default());
        let mut c = Canvas::new(640, 400);
        t.draw(repriseapp::MAIN, &mut c, &ctx);
        let lay = t.chrome_layout(&ctx);
        let (x, y) = (lay.toolbar.width / 2, lay.toolbar.height / 2);
        assert_eq!(
            c.get(x, y) >> 24,
            0xFF,
            "{i:?} fills the toolbar between its buttons"
        );
    }
}

#[test]
fn apply_updates_open_tabs() {
    let dir = tempfile::tempdir().unwrap();
    let mut t = in_dir(dir.path(), Settings::default());
    let cmd = ["sh", "-c", "seq 1 200; sleep 5"].map(OsString::from);
    t.cmd = Some(cmd.to_vec());
    t.start(repriseapp::Waker::noop());
    t.spawn_tab(&launch(&["sh", "-c", "seq 1 200; sleep 5"]), None);
    assert_eq!(t.w.tabs.list.len(), 2);
    until("the output reached every scrollback", || {
        t.w.tabs.list.iter().all(|tab| tab.history() >= 100)
    });

    t.apply(Settings {
        scrollback: 20,
        scheme: "light".into(),
        ..Settings::default()
    });
    let ctx = themed_ctx(Theme::default());
    let scheme = t.scheme(&ctx.theme);
    for tab in &t.w.tabs.list {
        assert_eq!(tab.history(), 20, "every open tab keeps the new scrollback");
        let mut snap = Snapshot::new(0, 0, scheme.fg, scheme.bg);
        tab.snapshot(&scheme, &mut snap);
        assert_eq!(snap.bg, schemes::scheme("light", Interface::Classic).bg);
    }
    assert_eq!(
        settings::load(&dir.path().join("config.toml")).scrollback,
        20,
        "Apply writes the settings file"
    );
}

#[test]
fn window_size_in_state() {
    let dir = tempfile::tempdir().unwrap();
    let mut t = in_dir(dir.path(), Settings::default());
    t.cmd = Some(vec!["sleep".into(), "5".into()]);
    t.start(repriseapp::Waker::noop());
    let mut ctx = themed_ctx(Theme::default());
    ctx.size = (900, 500);
    t.event(repriseapp::MAIN, Event::Resized, &mut ctx);
    assert!(
        t.event(repriseapp::MAIN, Event::CloseRequest, &mut ctx)
            .quit
    );
    let next = in_dir(dir.path(), Settings::default());
    assert_eq!(next.initial_size(), (900, 500));
}

#[test]
fn the_options_sheet_is_a_child_that_draws_under_every_interface() {
    let dir = tempfile::tempdir().unwrap();
    for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
        let mut t = in_dir(dir.path(), Settings::default());
        assert!(t.children().is_empty());
        let ctx = themed_ctx(reprisekit::interfaces::theme_for(i));
        t.forms = Some((ctx.text.clone(), ctx.forms()));
        assert!(t.open_options().redraw_all);
        let ids: Vec<_> = t.children().iter().map(|c| c.id).collect();
        assert_eq!(ids, [OPTIONS], "{i:?}");
        let (w, h) = t.children()[0].size;
        let mut c = Canvas::new(w as u32, h as u32);
        t.draw(OPTIONS, &mut c, &ctx);
        let face = ctx.forms().colors.button_face;
        let face =
            (0xFF << 24) | u32::from(face.r) << 16 | u32::from(face.g) << 8 | u32::from(face.b);
        assert!(
            (0..h).any(|y| (0..w).any(|x| c.get(x, y) != face)),
            "{i:?}: the sheet draws something on the face"
        );
    }
}

#[test]
fn focus_reloads_changed_settings() {
    let dir = tempfile::tempdir().unwrap();
    let mut t = in_dir(dir.path(), Settings::default());
    t.cmd = Some(vec!["sleep".into(), "5".into()]);
    t.start(repriseapp::Waker::noop());
    let mut ctx = themed_ctx(Theme::default());
    let config = dir.path().join("config.toml");
    let write = |scrollback, secs| {
        let s = Settings {
            scrollback,
            scheme: "dark".into(),
            ..Settings::default()
        };
        settings::save(&config, &s).unwrap();
        let at = std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs);
        std::fs::File::options()
            .write(true)
            .open(&config)
            .unwrap()
            .set_modified(at)
            .unwrap();
    };
    write(300, 1_000_000);
    t.event(repriseapp::MAIN, Event::Focus(true), &mut ctx);
    assert_eq!(t.settings.scrollback, 300);
    assert_eq!(t.settings.scheme, "dark");
    write(400, 1_000_000);
    t.event(repriseapp::MAIN, Event::Focus(true), &mut ctx);
    assert_eq!(t.settings.scrollback, 300, "an unchanged mtime is not read");
    write(500, 2_000_000);
    t.event(repriseapp::MAIN, Event::Focus(true), &mut ctx);
    assert_eq!(t.settings.scrollback, 500);
}

#[test]
fn apply_draws_every_row_again() {
    let ctx = test_ctx();
    let mut t = terminal(&["cat"]);
    let (w, h) = ctx.size;
    let mut c = Canvas::new(w as u32, h as u32);
    t.draw(MAIN, &mut c, &ctx);
    c.take_damage();
    t.draw(MAIN, &mut c, &ctx);
    assert!(
        c.take_damage().is_some(),
        "an unchanged frame reports its rects"
    );
    t.take(Settings {
        scheme: "light".into(),
        ..t.settings.clone()
    });
    t.draw(MAIN, &mut c, &ctx);
    assert_eq!(c.take_damage(), None, "the whole canvas changed");
    let bg = schemes::scheme("light", Interface::Classic).bg;
    let px = c.get(w / 2, h - 5);
    assert_eq!(
        px & 0xFFFFFF,
        u32::from(bg.r) << 16 | u32::from(bg.g) << 8 | u32::from(bg.b)
    );
}

#[test]
fn leaving_glass_draws_a_see_through_grid_solid() {
    let mut ctx = themed_ctx(reprisekit::interfaces::theme_for(Interface::Glass));
    let mut t = terminal(&["cat"]);
    t.settings.see_through = true;
    let (w, h) = ctx.size;
    let mut c = Canvas::new(w as u32, h as u32);
    t.draw(MAIN, &mut c, &ctx);
    assert!(c.get(w / 2, h - 5) >> 24 < 255, "see-through under Glass");
    ctx.theme = Theme::default();
    t.draw(MAIN, &mut c, &ctx);
    for y in (chrome::band_height(&ctx.theme, false)..h).step_by(7) {
        assert_eq!(c.get(w / 2, y) >> 24, 255, "row at {y} is solid");
    }
}

/// A logical point in grid cell (`col`, `row`) at scale 1, in its left half or its right.
fn cell_point(t: &mut Terminal, ctx: &Ctx, col: i32, row: i32, right: bool) -> (i32, i32) {
    let m = t.metrics(1);
    let g = chrome::grid_rect(ctx, false);
    let x = if right {
        m.cell_w * 3 / 4
    } else {
        m.cell_w / 4
    };
    (
        g.x + col * m.cell_w + x,
        g.y + row * m.cell_h + m.cell_h / 2,
    )
}

fn row_text(t: &Terminal, row: usize) -> String {
    let mut snap = Snapshot::new(0, 0, Color::rgb(0, 0, 0), Color::rgb(0, 0, 0));
    let scheme = schemes::scheme("reprise", Interface::Classic);
    t.w.tabs.current().unwrap().snapshot(&scheme, &mut snap);
    snap.row_text(row)
}

fn pointer(t: &mut Terminal, ctx: &mut Ctx, p: PointerInput) -> Reply {
    t.event(MAIN, Event::Pointer(p), ctx)
}

/// Press at cell `from`, drag to cell `to` and release there, with `mods` held.
fn drag(t: &mut Terminal, ctx: &mut Ctx, from: (i32, i32), to: (i32, i32), mods: Mods) -> Reply {
    let (x, y) = cell_point(t, ctx, from.0, from.1, false);
    let press = PointerInput::Press {
        x,
        y,
        button: BTN_LEFT,
        time: 0,
        mods,
    };
    pointer(t, ctx, press);
    let (x, y) = cell_point(t, ctx, to.0, to.1, true);
    pointer(t, ctx, PointerInput::Motion { x, y });
    let release = PointerInput::Release {
        x,
        y,
        button: BTN_LEFT,
        mods,
    };
    pointer(t, ctx, release)
}

#[test]
fn a_selection_copies_to_primary_and_shift_overrides_mouse_mode() {
    let mut ctx = test_ctx();
    let mut t = terminal(&["sh", "-c", "printf 'abc def'; sleep 5"]);
    t.follow_scale(MAIN, 1, &mut ctx);
    until("the text shows", || row_text(&t, 0) == "abc def");
    let reply = drag(&mut t, &mut ctx, (1, 0), (5, 0), Mods::default());
    assert_eq!(reply.copy_primary.as_deref(), Some("bc de"));
    assert!(!reply.paste_primary);

    let mut t = terminal(&[
        "sh",
        "-c",
        "printf '\\033[?1000h\\033[?1006habc def'; sleep 5",
    ]);
    t.follow_scale(MAIN, 1, &mut ctx);
    until("the program takes the mouse", || {
        t.w.tabs
            .current()
            .unwrap()
            .mode()
            .contains(TermMode::SGR_MOUSE)
            && row_text(&t, 0) == "abc def"
    });
    let reply = drag(&mut t, &mut ctx, (1, 0), (5, 0), Mods::default());
    assert_eq!(reply.copy_primary, None, "the press went to the program");
    assert_eq!(t.w.tabs.current().unwrap().selection(), None);
    let shift = Mods {
        shift: true,
        ..Mods::default()
    };
    let reply = drag(&mut t, &mut ctx, (1, 0), (5, 0), shift);
    assert_eq!(reply.copy_primary.as_deref(), Some("bc de"));
}

#[test]
fn middle_click_pastes_the_primary_selection() {
    let mut ctx = test_ctx();
    let mut t = terminal(&["cat"]);
    t.follow_scale(MAIN, 1, &mut ctx);
    let (x, y) = cell_point(&mut t, &ctx, 2, 2, false);
    assert!(t.event(MAIN, press(x, y, 0x112), &mut ctx).paste_primary);
    t.event(MAIN, Event::PastedPrimary("from primary".into()), &mut ctx);
    until("cat echoes it", || row_text(&t, 0) == "from primary");
}

#[test]
fn the_wheel_moves_the_scrollback() {
    let mut ctx = test_ctx();
    let mut t = terminal(&["sh", "-c", "seq 100; sleep 5"]);
    t.follow_scale(MAIN, 1, &mut ctx);
    until("the lines scroll off", || {
        t.w.tabs.current().unwrap().with_term(|term| {
            use alacritty_terminal::grid::Dimensions;
            term.history_size() > 10
        })
    });
    let (x, y) = cell_point(&mut t, &ctx, 0, 0, false);
    pointer(&mut t, &mut ctx, PointerInput::Scroll { x, y, rows: -3 });
    let offset = |t: &Terminal| {
        t.w.tabs
            .current()
            .unwrap()
            .with_term(|term| term.grid().display_offset())
    };
    assert_eq!(offset(&t), 3);
    pointer(&mut t, &mut ctx, PointerInput::Scroll { x, y, rows: 3 });
    assert_eq!(offset(&t), 0);
}

#[test]
fn a_toolbar_press_survives_a_closed_tab() {
    let mut ctx = test_ctx();
    let mut t = terminal(&["cat"]);
    t.spawn_tab(&launch(&["cat"]), None);
    t.follow_scale(MAIN, 1, &mut ctx);
    let (x, y) = centre(*t.chrome_layout(&ctx).buttons.last().unwrap());
    t.event(MAIN, press(x, y, BTN_LEFT), &mut ctx);
    t.event(MAIN, key(0x57, true, true), &mut ctx);
    assert_eq!(t.w.tabs.list.len(), 1, "Ctrl+Shift+W closed a tab");
    assert_eq!(t.w.chrome.bar.pressed, Some(t.w.chrome.bar.items.len() - 1));
    let release = PointerInput::Release {
        x,
        y,
        button: BTN_LEFT,
        mods: Mods::default(),
    };
    t.event(MAIN, Event::Pointer(release), &mut ctx);
    assert_eq!(t.w.chrome.bar.pressed, None);
    assert_eq!(
        t.children().first().map(|c| c.id),
        Some(OPTIONS),
        "the click still opens Options"
    );
}

/// A 1 MB paste into a program reading raw bytes reaches the PTY as one write and arrives whole.
fn paste_arrives(bracketed: bool) {
    const MB: usize = 1 << 20;
    let marks = if bracketed { 12 } else { 0 };
    let script = format!(
        "{} stty raw -echo; echo ready; head -c {} | wc -c; sleep 5",
        if bracketed {
            "printf '\\033[?2004h';"
        } else {
            ""
        },
        MB + marks
    );
    let mut ctx = test_ctx();
    let mut t = terminal(&["sh", "-c", &script]);
    t.settings.confirm_multiline_paste = false;
    t.follow_scale(MAIN, 1, &mut ctx);
    until("the program reads raw", || row_text(&t, 0) == "ready");
    let mode = t.w.tabs.current().unwrap().mode();
    assert_eq!(mode.contains(TermMode::BRACKETED_PASTE), bracketed);
    let text = format!("{}\n", "a".repeat(1023)).repeat(1024);
    assert_eq!(text.len(), MB);
    t.w.tabs.current().unwrap().written.borrow_mut().clear();
    let start = std::time::Instant::now();
    t.event(MAIN, Event::Pasted(text), &mut ctx);
    // Only an O(n^2) pass gets near this; a linear one takes a few ms even unoptimised.
    let took = start.elapsed();
    assert!(
        took < std::time::Duration::from_millis(250),
        "the paste took {took:?}"
    );
    let lens: Vec<usize> =
        t.w.tabs
            .current()
            .unwrap()
            .written
            .borrow()
            .iter()
            .map(Vec::len)
            .collect();
    assert_eq!(lens, [MB + marks], "one write of the whole paste");
    let count = (MB + marks).to_string();
    until("every byte arrives", || {
        (0..4).any(|r| row_text(&t, r).trim() == count)
    });
}

#[test]
fn a_multi_line_paste_asks_first() {
    let mut ctx = test_ctx();
    let mut t = terminal(&["cat"]);
    let written = |t: &Terminal| t.w.tabs.list[0].written.borrow().concat();
    t.event(MAIN, Event::Pasted("one line".into()), &mut ctx);
    assert_eq!(written(&t), b"one line", "a single line goes straight in");
    t.w.tabs.list[0].written.borrow_mut().clear();

    t.event(MAIN, Event::Pasted("echo a\necho b\n".into()), &mut ctx);
    assert!(written(&t).is_empty(), "nothing goes in before the answer");
    let Some((Asking::Paste { .. }, b)) = &t.confirm else {
        panic!("the paste asks");
    };
    assert!(
        b.fields.text.contains("echo a\necho b"),
        "{}",
        b.fields.text
    );
    t.event(CONFIRM, key(keysym::ESCAPE, false, false), &mut ctx);
    assert!(t.confirm.is_none());
    assert!(written(&t).is_empty(), "Cancel drops the paste");

    t.event(MAIN, Event::Pasted("echo a\necho b\n".into()), &mut ctx);
    t.event(CONFIRM, key(keysym::RETURN, false, false), &mut ctx);
    assert_eq!(written(&t), b"echo a\recho b\r", "Paste sends it");
    t.w.tabs.list[0].written.borrow_mut().clear();

    t.settings.confirm_multiline_paste = false;
    t.event(MAIN, Event::Pasted("x\ny".into()), &mut ctx);
    assert!(t.confirm.is_none(), "the setting turns the question off");
    assert_eq!(written(&t), b"x\ry");
}

#[test]
fn a_paste_question_shows_the_first_lines() {
    let text = format!("{}\n", "x".repeat(70)).repeat(8);
    let q = paste_question(&text);
    let shown: Vec<&str> = q.lines().filter(|l| l.starts_with('x')).collect();
    assert_eq!(shown.len(), PASTE_SHOWN.0);
    assert_eq!(shown[0], format!("{}...", "x".repeat(PASTE_SHOWN.1)));
    assert!(
        q.contains("\n...\n"),
        "more lines than shown end in an ellipsis"
    );
}

#[test]
fn big_paste_chunks() {
    paste_arrives(false);
    paste_arrives(true);
}

#[test]
fn find_opens_beside_the_window_and_closes_with_it() {
    let mut ctx = test_ctx();
    let mut t = terminal(&["cat"]);
    t.event(MAIN, key(0x46, true, true), &mut ctx);
    assert!(t.children().iter().any(|c| c.id == FIND && !c.modal));
    t.event(FIND, Event::CloseRequest, &mut ctx);
    assert!(t.children().iter().all(|c| c.id != FIND));
}

#[test]
fn a_miss_says_so() {
    let mut ctx = test_ctx();
    let mut t = terminal(&["cat"]);
    t.event(MAIN, key(0x46, true, true), &mut ctx);
    t.event(FIND, Event::Pasted("zzqq-not-there".into()), &mut ctx);
    t.event(FIND, key(0xff0d, false, false), &mut ctx);
    assert!(t.children().iter().any(|c| c.id == NOTFOUND));
    t.event(NOTFOUND, key(0xff0d, false, false), &mut ctx);
    assert!(t.children().iter().all(|c| c.id != NOTFOUND));
}

#[test]
fn switching_tabs_moves_the_search() {
    let mut ctx = test_ctx();
    let mut t = terminal(&["cat"]);
    t.spawn_tab(&launch(&["cat"]), None);
    t.w.tabs.strip.open = 0;
    t.w.tabs.current().unwrap().find("x", false, true);
    assert!(t.w.tabs.list[0].searching());
    t.event(MAIN, key(0xff09, true, false), &mut ctx);
    assert_eq!(t.w.tabs.strip.open, 1);
    assert!(
        !t.w.tabs.list[0].searching(),
        "the old tab stops highlighting"
    );
}

#[test]
fn a_held_selection_ends_with_its_tab() {
    let mut ctx = test_ctx();
    let mut t = terminal(&["sh", "-c", "printf 'abc def'; sleep 5"]);
    t.spawn_tab(&launch(&["sh", "-c", "printf 'uvw xyz'; sleep 5"]), None);
    t.follow_scale(MAIN, 1, &mut ctx);
    until("the text shows", || row_text(&t, 0) == "uvw xyz");
    // Presses far apart in time, so none pairs into a double click.
    let clock = std::cell::Cell::new(0);
    let left = |t: &mut Terminal, ctx: &mut Ctx, col: i32, press: bool| {
        let (x, y) = cell_point(t, ctx, col, 0, !press);
        if !press {
            pointer(t, ctx, PointerInput::Motion { x, y });
        }
        let p = match press {
            true => PointerInput::Press {
                x,
                y,
                button: BTN_LEFT,
                time: clock.replace(clock.get() + 10_000),
                mods: Mods::default(),
            },
            false => PointerInput::Release {
                x,
                y,
                button: BTN_LEFT,
                mods: Mods::default(),
            },
        };
        pointer(t, ctx, p)
    };

    let tab = key(0xff09, true, false);
    t.event(MAIN, tab.clone(), &mut ctx);
    left(&mut t, &mut ctx, 1, true);
    left(&mut t, &mut ctx, 2, false);
    t.event(MAIN, tab.clone(), &mut ctx);
    left(&mut t, &mut ctx, 1, true);
    t.event(MAIN, tab, &mut ctx);
    assert_eq!(t.w.tabs.strip.open, 0, "Ctrl+Tab switched tabs mid-drag");
    let reply = left(&mut t, &mut ctx, 5, false);
    assert_eq!(reply.copy_primary, None, "the switch ended the drag");
    assert_eq!(
        t.w.tabs.current().unwrap().selection().as_deref(),
        Some("bc"),
        "the open tab's selection stays"
    );

    t.spawn_tab(&launch(&["cat"]), None);
    t.w.tabs.select(0);
    left(&mut t, &mut ctx, 1, true);
    let (x, y) = centre(t.chrome_layout(&ctx).strip.tabs[2]);
    t.event(MAIN, press(x, y, 0x112), &mut ctx);
    assert_eq!(t.w.tabs.list.len(), 2, "middle-click closed another tab");
    let reply = left(&mut t, &mut ctx, 5, false);
    assert_eq!(reply.copy_primary.as_deref(), Some("bc de"));

    left(&mut t, &mut ctx, 1, true);
    t.event(MAIN, key(0x57, true, true), &mut ctx);
    assert_eq!(t.w.tabs.list.len(), 1, "Ctrl+Shift+W closed the held tab");
    let reply = left(&mut t, &mut ctx, 5, false);
    assert_eq!(
        reply.copy_primary, None,
        "closing the open tab ended the drag"
    );
}

#[test]
fn every_reported_press_reports_its_release() {
    let mut ctx = test_ctx();
    let mut t = terminal(&["sh", "-c", "printf '\\033[?1000h\\033[?1006h'; sleep 5"]);
    t.follow_scale(MAIN, 1, &mut ctx);
    until("the program takes the mouse", || {
        t.w.tabs
            .current()
            .unwrap()
            .mode()
            .contains(TermMode::SGR_MOUSE)
    });
    let (x, y) = cell_point(&mut t, &ctx, 0, 0, false);
    let release = |button| {
        Event::Pointer(PointerInput::Release {
            x,
            y,
            button,
            mods: Mods::default(),
        })
    };
    t.w.tabs.current().unwrap().written.borrow_mut().clear();
    t.event(MAIN, press(x, y, BTN_LEFT), &mut ctx);
    t.event(MAIN, press(x, y, repriseapp::BTN_RIGHT), &mut ctx);
    t.event(MAIN, release(repriseapp::BTN_RIGHT), &mut ctx);
    t.event(MAIN, release(BTN_LEFT), &mut ctx);
    let written = t.w.tabs.current().unwrap().written.borrow().concat();
    assert_eq!(
        String::from_utf8(written).unwrap(),
        "\x1b[<0;1;1M\x1b[<2;1;1M\x1b[<2;1;1m\x1b[<0;1;1m"
    );
}

#[test]
fn a_broken_file_keeps_the_live_settings() {
    let dir = tempfile::tempdir().unwrap();
    let live = Settings {
        scrollback: 300,
        ..Settings::default()
    };
    let mut t = in_dir(dir.path(), live.clone());
    let mut ctx = themed_ctx(Theme::default());
    std::fs::write(
        dir.path().join("config.toml"),
        "scrollback = 500\nsize_px = [",
    )
    .unwrap();
    t.event(MAIN, Event::Focus(true), &mut ctx);
    assert_eq!(
        t.settings, live,
        "a file that does not parse changes nothing"
    );
}

#[test]
fn the_text_size_from_the_file_stays_in_the_zoom_range() {
    let dir = tempfile::tempdir().unwrap();
    let mut t = in_dir(
        dir.path(),
        Settings {
            size_px: 0,
            ..Settings::default()
        },
    );
    assert_eq!((t.px, t.settings.size_px), (ZOOM.0, ZOOM.0));
    t.take(Settings {
        size_px: 5000,
        ..Settings::default()
    });
    assert_eq!((t.px, t.settings.size_px), (ZOOM.1, ZOOM.1));
}

#[test]
fn a_reload_from_a_child_window_redraws_the_main_one() {
    let dir = tempfile::tempdir().unwrap();
    let mut t = in_dir(dir.path(), Settings::default());
    let mut ctx = themed_ctx(Theme::default());
    assert!(
        !t.event(CONFIRM, Event::Focus(true), &mut ctx).redraw_all,
        "nothing changed"
    );
    let s = Settings {
        scheme: "dark".into(),
        ..Settings::default()
    };
    settings::save(&dir.path().join("config.toml"), &s).unwrap();
    assert!(t.event(CONFIRM, Event::Focus(true), &mut ctx).redraw_all);
    assert_eq!(t.settings.scheme, "dark");
}

#[test]
fn only_a_switch_ends_the_search() {
    let mut ctx = test_ctx();
    let mut t = terminal(&["cat"]);
    t.spawn_tab(&launch(&["cat"]), None);
    t.w.tabs.select(0);
    t.w.tabs.list[0].find("x", false, true);
    let (x, y) = centre(t.chrome_layout(&ctx).strip.tabs[0]);
    t.event(MAIN, press(x, y, BTN_LEFT), &mut ctx);
    assert!(
        t.w.tabs.list[0].searching(),
        "a click on the open tab keeps it"
    );
    t.waker = None;
    t.event(MAIN, key(0x54, true, true), &mut ctx);
    assert_eq!(t.w.tabs.list.len(), 2, "no tab could start");
    assert!(t.w.tabs.list[0].searching(), "a failed new tab keeps it");
}

#[test]
fn a_palette_query_answers_the_palette_colour() {
    let ctx = test_ctx();
    let mut t = terminal(&[
        "sh",
        "-c",
        "printf '\\033]4;196;?\\007\\033]4;244;?\\007'; sleep 5",
    ]);
    let scheme = t.scheme(&ctx.theme);
    let answers =
        |t: &Terminal| String::from_utf8(t.w.tabs.list[0].written.borrow().concat()).unwrap();
    until("both queries are answered", || {
        t.w.tabs.list[0].pump(&scheme);
        answers(&t).matches("\x1b]4;").count() == 2
    });
    let text = answers(&t);
    assert!(
        text.contains("4;196;rgb:ffff/0000/0000"),
        "196 is the cube's red: {text:?}"
    );
    assert!(
        text.contains("4;244;rgb:8080/8080/8080"),
        "244 is a grey ramp step: {text:?}"
    );
}

#[test]
fn a_long_title_is_cut() {
    let mut ctx = test_ctx();
    let script = "printf '\\033]0;'; head -c 5000 /dev/zero | tr '\\0' x; printf '\\007'; sleep 5";
    let mut t = terminal(&["sh", "-c", script]);
    until("the title arrives", || {
        t.event(MAIN, Event::Wake, &mut ctx);
        t.w.tabs.list[0].title().starts_with('x')
    });
    assert_eq!(t.w.tabs.list[0].title(), "x".repeat(tab::TITLE_MAX));
}

#[test]
fn a_program_s_clipboard_store_is_copied() {
    let mut ctx = test_ctx();
    let script =
        "printf '\\033]52;c;aGVsbG8=\\007\\033]52;p;d29ybGQ=\\007\\033]52;c;?\\007'; sleep 5";
    let mut t = terminal(&["sh", "-c", script]);
    let (mut copy, mut primary) = (None, None);
    until("both stores arrive", || {
        let reply = t.event(MAIN, Event::Wake, &mut ctx);
        copy = reply.copy.or(copy.take());
        primary = reply.copy_primary.or(primary.take());
        copy.is_some() && primary.is_some()
    });
    assert_eq!(copy.as_deref(), Some("hello"));
    assert_eq!(primary.as_deref(), Some("world"));
    assert!(
        t.w.tabs.list[0].written.borrow().is_empty(),
        "a load request is not answered"
    );
}

#[test]
fn partial_damage_is_exact() {
    let mut ctx = test_ctx();
    let mut t = terminal(&["cat"]);
    t.follow_scale(MAIN, 1, &mut ctx);
    let (w, h) = ctx.size;
    let mut c = Canvas::new(w as u32, h as u32);
    t.draw(MAIN, &mut c, &ctx);
    assert_eq!(c.take_damage(), None, "the first frame is drawn whole");
    t.draw(MAIN, &mut c, &ctx);
    assert_eq!(
        c.take_damage(),
        Some(vec![Rect::default()]),
        "an unchanged frame damages nothing"
    );

    let m = t.metrics(1);
    let grid = chrome::grid_rect(&ctx, false);
    let row = |r: i32| Rect::new(grid.x, grid.y + r * m.cell_h, grid.width, m.cell_h);
    let cursor_row = |t: &Terminal| {
        t.w.tabs
            .current()
            .unwrap()
            .with_term(|term| term.grid().cursor.point.line.0)
    };
    t.w.tabs.current().unwrap().write(b"x".as_slice().into());
    until("cat's echo shows", || {
        row_text(&t, 0) == "x" && cursor_row(&t) == 0
    });
    t.draw(MAIN, &mut c, &ctx);
    let gap = t.chrome_layout(&ctx).gap;
    assert_eq!(
        c.take_damage(),
        Some(vec![row(0), gap]),
        "only the changed row and the gap over it"
    );

    t.w.tabs.current().unwrap().write(b"\n".as_slice().into());
    until("cat writes the line back", || {
        row_text(&t, 1) == "x" && cursor_row(&t) == 2
    });
    t.draw(MAIN, &mut c, &ctx);
    assert_eq!(
        c.take_damage(),
        Some(vec![row(0), row(1), row(2), gap]),
        "the row the cursor left, the new line, the row it entered"
    );

    t.w.chrome.dirty = true;
    t.draw(MAIN, &mut c, &ctx);
    let band = chrome::band_height(&ctx.theme, false);
    assert_eq!(
        c.take_damage(),
        Some(vec![Rect::new(0, 0, w, band)]),
        "a chrome change damages only the band"
    );

    let ctx = themed_ctx(reprisekit::interfaces::theme_for(Interface::Glass));
    let (w, h) = ctx.size;
    let mut c = Canvas::new(w as u32, h as u32);
    let band = chrome::band_height(&ctx.theme, false);
    for frame in 0..3 {
        t.draw(MAIN, &mut c, &ctx);
        assert_eq!(
            c.take_glass_top(),
            band,
            "frame {frame} reports the glass band"
        );
        if frame > 0 {
            assert_eq!(
                c.take_damage(),
                Some(vec![Rect::default()]),
                "frame {frame} is kept"
            );
        } else {
            c.take_damage();
        }
    }
}

/// A started terminal like [`terminal`] whose compositor keeps tab groups and whose tabs go in
/// the caption.
fn captioned(cmd: &[&str]) -> Terminal {
    let mut t = terminal(cmd);
    t.settings.tab_position = TabPosition::Caption;
    t.tab_groups = true;
    t.w.caption = t.caption_mode();
    t
}

const NEW_TAB: (u32, bool, bool) = (0x54, true, true);
const CLOSE_TAB: (u32, bool, bool) = (0x57, true, true);
const FIND_KEY: (u32, bool, bool) = (0x46, true, true);

fn keys(k: (u32, bool, bool)) -> Event {
    key(k.0, k.1, k.2)
}

#[test]
fn in_caption_mode_a_new_tab_is_a_window_that_joins_this_ones_group() {
    let mut ctx = test_ctx();
    let mut t = captioned(&["cat"]);
    let (second, third, fourth) = (
        repriseapp::window_id(1),
        repriseapp::window_id(2),
        repriseapp::window_id(3),
    );
    t.event(MAIN, keys(NEW_TAB), &mut ctx);
    assert_eq!(t.order, [MAIN, second]);
    assert_eq!(t.parent(second), None, "a window of its own");
    assert_eq!(t.tab_with(second), Some(MAIN));
    assert!(
        t.children()
            .iter()
            .any(|c| c.id == second && !c.fixed && !c.modal)
    );
    assert_eq!(
        t.pane(MAIN).unwrap().tabs.list.len(),
        1,
        "the first window keeps one tab"
    );
    assert_eq!(t.pane(second).unwrap().tabs.list.len(), 1);
    t.event(second, keys(NEW_TAB), &mut ctx);
    assert_eq!(t.order, [MAIN, second, third]);
    assert_eq!(t.tab_with(third), Some(second));
    t.event(MAIN, keys(NEW_TAB), &mut ctx);
    assert_eq!(
        t.order,
        [MAIN, fourth, second, third],
        "a new tab goes right after the window it came from"
    );
}

#[test]
fn ctrl_tab_in_caption_mode_raises_the_next_window_going_round() {
    let mut ctx = test_ctx();
    let mut t = captioned(&["cat"]);
    t.event(MAIN, keys(NEW_TAB), &mut ctx);
    t.event(repriseapp::window_id(1), keys(NEW_TAB), &mut ctx);
    let (second, third) = (repriseapp::window_id(1), repriseapp::window_id(2));
    let next = |t: &mut Terminal, ctx: &mut Ctx, win, shift| {
        t.event(win, key(0xff09, true, shift), ctx).raise
    };
    assert_eq!(next(&mut t, &mut ctx, MAIN, false), Some(second));
    assert_eq!(
        next(&mut t, &mut ctx, MAIN, true),
        Some(third),
        "back round"
    );
    assert_eq!(next(&mut t, &mut ctx, third, false), Some(MAIN));
    assert_eq!(t.order.len(), 3, "raising opens and closes nothing");
}

#[test]
fn the_program_quits_with_its_last_window() {
    let mut ctx = test_ctx();
    let mut t = captioned(&["cat"]);
    t.event(MAIN, keys(NEW_TAB), &mut ctx);
    let second = repriseapp::window_id(1);
    assert!(!t.event(MAIN, keys(CLOSE_TAB), &mut ctx).quit);
    assert!(t.hidden(), "the first window closes");
    assert_eq!(t.order, [second], "the second stays");
    assert!(t.children().iter().any(|c| c.id == second));
    assert!(
        t.event(second, keys(CLOSE_TAB), &mut ctx).quit,
        "the last window closing quits"
    );
}

#[test]
fn find_and_options_open_over_the_window_that_asked() {
    let mut ctx = test_ctx();
    let mut t = captioned(&["cat"]);
    t.event(MAIN, keys(NEW_TAB), &mut ctx);
    let second = repriseapp::window_id(1);
    t.event(second, keys(FIND_KEY), &mut ctx);
    assert_eq!(t.parent(FIND), Some(second));
    t.event(MAIN, keys(FIND_KEY), &mut ctx);
    assert_eq!(
        t.parent(FIND),
        Some(MAIN),
        "an open Find moves to the window that asks"
    );
    assert!(t.enter(second));
    t.open_options();
    assert_eq!(t.parent(OPTIONS), Some(second));
    assert_eq!(t.parent(PROFILE), Some(second));
    t.event(second, keys(CLOSE_TAB), &mut ctx);
    assert_eq!(
        t.parent(OPTIONS),
        Some(MAIN),
        "Options moves to a window still open"
    );
    assert!(t.find.is_some(), "Find over the other window stays");
}

#[test]
fn a_busy_window_asks_over_itself_before_closing() {
    let mut ctx = test_ctx();
    let mut t = captioned(&["cat"]);
    t.event(MAIN, keys(NEW_TAB), &mut ctx);
    let second = repriseapp::window_id(1);
    assert!(t.enter(second));
    t.w.tabs.list[0].write(b"sleep 30\n".as_slice().into());
    until("sleep holds the foreground", || {
        t.pane(second).unwrap().tabs.list[0].busy()
    });
    t.event(second, Event::CloseRequest, &mut ctx);
    assert_eq!(t.parent(CONFIRM), Some(second));
    t.event(CONFIRM, key(keysym::RETURN, false, false), &mut ctx);
    assert_eq!(t.order, [MAIN], "Close closes that window alone");
}

#[test]
fn a_tab_position_change_reaches_new_windows_only() {
    let mut ctx = test_ctx();
    let mut t = terminal(&["cat"]);
    t.tab_groups = true;
    t.take(Settings {
        tab_position: TabPosition::Caption,
        ..t.settings.clone()
    });
    assert!(
        t.caption_mode(),
        "a window opened now keeps its tabs in the caption"
    );
    assert!(!t.w.caption);
    t.event(MAIN, keys(NEW_TAB), &mut ctx);
    assert_eq!(t.w.tabs.list.len(), 2, "the open window keeps its strip");
    assert_eq!(t.order, [MAIN]);
}

#[test]
fn without_the_compositor_s_tab_groups_the_tabs_stay_below_the_toolbar() {
    let nowhere = Path::new(NOWHERE);
    let settings = Settings {
        shell: "/bin/sh".into(),
        tab_position: TabPosition::Caption,
        ..Settings::default()
    };
    let mut t = Terminal::with_state(settings, Some(argv(&["cat"])), nowhere.join("state.toml"));
    t.start(repriseapp::Waker::noop());
    assert!(!t.w.caption);
}

fn release(x: i32, y: i32) -> Event {
    Event::Pointer(PointerInput::Release {
        x,
        y,
        button: BTN_LEFT,
        mods: Mods::default(),
    })
}

#[test]
fn the_grid_sits_a_gap_of_terminal_background_under_the_chrome_in_both_tab_modes() {
    for caption in [false, true] {
        let mut ctx = test_ctx();
        let mut t = match caption {
            true => captioned(&["cat"]),
            false => terminal(&["cat"]),
        };
        t.follow_scale(MAIN, 1, &mut ctx);
        let (w, h) = ctx.size;
        let mut c = Canvas::new(w as u32, h as u32);
        t.draw(MAIN, &mut c, &ctx);
        let lay = t.chrome_layout(&ctx);
        assert_eq!(lay.gap, Rect::new(0, lay.band, w, chrome::GRID_GAP));
        assert_eq!(lay.grid.y, lay.gap.bottom(), "caption {caption}");
        let bg = t.w.snap.bg;
        let want = 0xFF00_0000 | u32::from(bg.r) << 16 | u32::from(bg.g) << 8 | u32::from(bg.b);
        for y in lay.gap.y..lay.gap.bottom() {
            assert_eq!(c.get(w / 2, y), want, "caption {caption}: gap row {y}");
        }
    }
}

#[test]
fn in_caption_mode_the_toolbar_leads_with_new_tab_and_the_grid_takes_the_strip() {
    let mut ctx = test_ctx();
    let mut t = captioned(&["cat"]);
    t.follow_scale(MAIN, 1, &mut ctx);
    let lay = t.chrome_layout(&ctx);
    assert_eq!(lay.band, lay.toolbar.height, "no strip under the toolbar");
    assert_eq!(lay.grid.y, lay.toolbar.height + chrome::GRID_GAP);
    assert!(matches!(
        t.w.chrome.bar.items[0],
        toolbar::Item::DropButton {
            id: chrome::NEW,
            ..
        }
    ));
    let r = lay.buttons[0];
    let (arrow, main) = (
        (r.right() - 3, r.y + r.height / 2),
        (r.x + 4, r.y + r.height / 2),
    );
    t.event(MAIN, press(arrow.0, arrow.1, BTN_LEFT), &mut ctx);
    assert!(t.w.chrome.menu.is_some(), "the arrow opens the menu");
    t.event(MAIN, key(keysym::ESCAPE, false, false), &mut ctx);
    assert!(t.w.chrome.menu.is_none());
    t.event(MAIN, press(main.0, main.1, 0x111), &mut ctx);
    assert!(t.w.chrome.menu.is_some(), "a right-click opens it too");
    t.event(MAIN, key(keysym::ESCAPE, false, false), &mut ctx);
    t.event(MAIN, press(main.0, main.1, BTN_LEFT), &mut ctx);
    t.event(MAIN, release(main.0, main.1), &mut ctx);
    assert_eq!(t.order.len(), 2, "New Tab opens a window");
}

fn fnv(c: &Canvas) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for v in [c.width, c.height]
        .into_iter()
        .chain(c.pixels.iter().copied())
    {
        for b in v.to_le_bytes() {
            h = (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3);
        }
    }
    h
}

#[test]
fn the_caption_mode_toolbar_pins_in_every_interface() {
    if !reprisekit::text::test_tahoma() {
        return;
    }
    let mut got = Vec::new();
    for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
        let mut ctx = themed_ctx(reprisekit::interfaces::theme_for(i));
        let mut t = captioned(&["cat"]);
        t.follow_scale(MAIN, 1, &mut ctx);
        let mut c = Canvas::new(640, 400);
        t.draw(MAIN, &mut c, &ctx);
        let band = t.chrome_layout(&ctx).band;
        let mut top = Canvas::new(640, band as u32);
        top.blit_canvas(0, 0, &c);
        got.push((format!("{i:?}"), fnv(&top)));
    }
    let want: [(&str, u64); 3] = [
        ("Classic", 0x30ba_a6b4_d116_ac75),
        ("Blue", 0x4eda_45d7_f42f_2eba),
        ("Glass", 0xa36d_f98e_ba6b_7591),
    ];
    let want: Vec<(String, u64)> = want.iter().map(|(k, h)| (k.to_string(), *h)).collect();
    assert_eq!(got, want);
}

#[test]
fn output_in_one_caption_tab_redraws_only_its_window() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = test_ctx();
    let mut t = captioned(&["cat"]);
    t.config = dir.path().join("config.toml");
    t.event(MAIN, keys(NEW_TAB), &mut ctx);
    t.event(MAIN, keys(NEW_TAB), &mut ctx);
    let (second, third) = (repriseapp::window_id(1), repriseapp::window_id(2));
    assert_eq!(t.order, [MAIN, third, second]);
    let (w, h) = ctx.size;
    let mut c = Canvas::new(w as u32, h as u32);
    let mut settle = |t: &mut Terminal, ctx: &mut Ctx| {
        for w in [MAIN, second, third] {
            t.draw(w, &mut c, ctx);
        }
        for _ in 0..20 {
            t.event(MAIN, Event::Wake, ctx);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        for w in [MAIN, second, third] {
            t.draw(w, &mut c, ctx);
        }
    };
    settle(&mut t, &mut ctx);
    assert!(
        t.event(MAIN, Event::Wake, &mut ctx).redraw_wins.is_empty(),
        "a wake with no output draws nothing"
    );
    t.enter(second);
    t.w.tabs.list[0].write(b"hello\n".as_slice().into());
    let mut drawn = Vec::new();
    until("cat echoes", || {
        let reply = t.event(MAIN, Event::Wake, &mut ctx);
        assert!(!reply.redraw_all);
        drawn.extend(reply.redraw_wins);
        !drawn.is_empty()
    });
    assert_eq!(drawn, [second], "the idle windows stay as drawn");
    settle(&mut t, &mut ctx);

    t.enter(MAIN);
    t.open_options();
    let o = t.options.as_mut().unwrap();
    o.sheet.pages[0].set_check("blink", false);
    o.sheet.set_apply_enabled(true);
    let alt_a = Event::Key(KeyInput {
        sym: u32::from(b'a'),
        utf8: None,
        mods: Mods {
            alt: true,
            ..Mods::default()
        },
        edit: Some(reprisekit::textedit::EditKey::Insert("a".into())),
    });
    assert!(
        t.event(OPTIONS, alt_a, &mut ctx).redraw_all,
        "Apply draws every window"
    );
    assert!(!t.settings.blink, "Apply took the settings");
    for w in [MAIN, second, third] {
        assert!(t.pane(w).unwrap().full, "{w:?} draws whole");
    }
}
