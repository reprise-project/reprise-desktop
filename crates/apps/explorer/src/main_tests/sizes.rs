use super::*;

/// A divider dragged 40 px at 200% saves the column 20 px wider: widths are kept at 100%.
#[test]
fn a_column_drag_at_200_percent_saves_half_the_px() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ctx = test_ctx();
    ctx.theme = Theme::default().with_text_size(200);
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.resizing = Some((2, 300, 118));
    ex.pointer(repriseapp::PointerInput::Motion { x: 340, y: 50 }, &mut ctx);
    ex.pointer(
        repriseapp::PointerInput::Release {
            x: 340,
            y: 50,
            button: crate::BTN_LEFT,
            mods: repriseapp::Mods::default(),
        },
        &mut ctx,
    );
    assert_eq!(ex.sv.columns[2].width, 138);
    assert_eq!(ex.settings.columns[2], 138);
}

/// The splitter dragged to 300 px at 200% saves the Folders bar 150 wide: kept at 100%.
#[test]
fn a_splitter_drag_at_200_percent_saves_half_the_px() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ctx = test_ctx();
    ctx.theme = Theme::default().with_text_size(200);
    ctx.size = (1600, 900);
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.dragging = Some(reprisekit::controls::explorerbar::SPLITTER_W / 2);
    let x = 300 + reprisekit::controls::explorerbar::SPLITTER_W / 2;
    ex.pointer(repriseapp::PointerInput::Motion { x, y: 200 }, &mut ctx);
    assert_eq!(ex.explorer_bar.width, 150);
    ex.pointer(
        repriseapp::PointerInput::Release {
            x,
            y: 200,
            button: crate::BTN_LEFT,
            mods: repriseapp::Mods::default(),
        },
        &mut ctx,
    );
    assert_eq!(ex.settings.bar_width, 150);
    let bar = ex.layout(&ctx).bar.expect("bar");
    assert_eq!(bar.header.width, 300);
}

#[test]
fn settings_changes_defer_the_write() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ctx = test_ctx();
    let ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    assert!(!ex.settings_dirty);
    let mut app = Lazy {
        start: Start {
            place: ex.place.clone(),
            dir: None,
            select: Vec::new(),
            alone: None,
            connect: None,
            search: false,
            tabs: None,
        },
        inner: Some(ex),
        units: TextUnits::ONE,
    };
    ctx.at_pick = true;
    let _ = app.event(MAIN, Event::Resized, &mut ctx);
    let ex = app.inner.as_mut().expect("the window");
    assert!(!ex.settings_dirty, "the opening size marks nothing");
    ctx.at_pick = false;
    let _ = app.event(MAIN, Event::Resized, &mut ctx);
    let ex = app.inner.as_mut().expect("the window");
    assert!(
        ex.settings_dirty,
        "a resize the user gives marks the settings"
    );
    for cmd in [
        Command::Mode(reprisekit::controls::shellview::Mode::Details),
        Command::Sort(1),
        Command::ToggleFolders,
        Command::CloseBar,
        Command::ToggleStatusBar,
        Command::ToggleToolbar(0),
    ] {
        ex.flush_settings();
        let _ = ex.run(cmd.clone(), &mut ctx);
        assert!(ex.settings_dirty, "{cmd:?} marks the settings");
    }
}

#[test]
fn a_settings_change_is_written_a_moment_later_once_for_a_burst() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    let now = std::time::Instant::now();
    ex.save_settings_due(now, &ctx);
    assert_eq!(ex.settings_due, None, "nothing changed, nothing waits");
    let _ = ex.run(
        Command::Mode(reprisekit::controls::shellview::Mode::Details),
        &mut ctx,
    );
    ex.save_settings_due(now, &ctx);
    let due = ex.settings_due.expect("the change waits to be written");
    assert!(ex.settings_dirty, "not written at once");
    let _ = ex.run(Command::ToggleStatusBar, &mut ctx);
    ex.save_settings_due(now + crate::SETTINGS_SAVE_AFTER / 2, &ctx);
    assert_eq!(
        ex.settings_due,
        Some(due),
        "a change meanwhile joins that write"
    );
    assert!(ex.settings_dirty);
    ex.save_settings_due(due, &ctx);
    assert!(!ex.settings_dirty, "written once the wait is up");
    assert_eq!(ex.settings_due, None);
}

/// The host tells a text size change as a resize at the old size, then regrows a window still at
/// its opening size to the size `initial_size` gives; a window the user sized keeps its size.
#[test]
fn the_window_opens_at_the_text_size_and_keeps_its_size_in_100_units() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ctx = test_ctx();
    let ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    let mut app = Lazy {
        start: Start {
            place: ex.place.clone(),
            dir: None,
            select: Vec::new(),
            alone: None,
            connect: None,
            search: false,
            tabs: None,
        },
        inner: Some(ex),
        units: TextUnits::ONE,
    };
    let mut resized = |app: &mut Lazy, text_size: u32, size: (i32, i32)| {
        ctx.text_size = text_size;
        ctx.size = size;
        let _ = app.event(MAIN, Event::Resized, &mut ctx);
    };
    let saved = |app: &mut Lazy| {
        let ex = app.inner.as_mut().expect("the window");
        ex.flush_settings();
        ex.settings.window
    };
    resized(&mut app, 100, (792, 573));
    assert_eq!(saved(&mut app), (792, 573));
    resized(&mut app, 200, (792, 573));
    assert_eq!(app.initial_size(), (1584, 1146), "a picked size regrows");
    resized(&mut app, 200, (1584, 1146));
    assert_eq!(saved(&mut app), (792, 573));
    resized(&mut app, 200, (1000, 700));
    assert_eq!(saved(&mut app), (500, 350));
    assert_eq!(app.initial_size(), (1000, 700));
    resized(&mut app, 100, (1000, 700));
    assert_eq!(
        saved(&mut app),
        (1000, 700),
        "a size the user kept across the change is its 100% size"
    );
}

/// A maximized window's size is not saved; a hand size pending when it was maximized is.
#[test]
fn a_maximized_size_is_not_saved() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ctx = test_ctx();
    let ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    let mut app = Lazy {
        start: Start {
            place: ex.place.clone(),
            dir: None,
            select: Vec::new(),
            alone: None,
            connect: None,
            search: false,
            tabs: None,
        },
        inner: Some(ex),
        units: TextUnits::ONE,
    };
    let mut resized = |app: &mut Lazy, size: (i32, i32), maximized: bool| {
        ctx.size = size;
        ctx.maximized = maximized;
        let _ = app.event(MAIN, Event::Resized, &mut ctx);
    };
    let saved = |app: &mut Lazy| {
        let ex = app.inner.as_mut().expect("the window");
        ex.flush_settings();
        ex.settings.window
    };
    resized(&mut app, (900, 600), false);
    resized(&mut app, (1920, 1000), true);
    assert_eq!(saved(&mut app), (900, 600), "the pending hand size");
    resized(&mut app, (1920, 1000), true);
    assert_eq!(saved(&mut app), (900, 600), "not the screen's");
    resized(&mut app, (900, 600), false);
    resized(&mut app, (700, 500), false);
    assert_eq!(saved(&mut app), (700, 500));
}

/// A size the window takes at its pick, held inside a 1080-line work area at 200% or not, keeps
/// the saved size, so the next open at 100% is the size before; a size the user gives it is saved.
#[test]
fn a_held_opening_size_keeps_the_saved_size_and_a_users_size_is_saved() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ctx = test_ctx();
    let ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    let mut app = Lazy {
        start: Start {
            place: ex.place.clone(),
            dir: None,
            select: Vec::new(),
            alone: None,
            connect: None,
            search: false,
            tabs: None,
        },
        inner: Some(ex),
        units: TextUnits::ONE,
    };
    let mut resized = |app: &mut Lazy, text_size: u32, size: (i32, i32), at_pick: bool| {
        ctx.text_size = text_size;
        ctx.size = size;
        ctx.at_pick = at_pick;
        let _ = app.event(MAIN, Event::Resized, &mut ctx);
    };
    let saved = |app: &mut Lazy| {
        let ex = app.inner.as_mut().expect("the window");
        ex.flush_settings();
        ex.settings.window
    };
    let held = (1584, 1000);

    resized(&mut app, 200, (792, 573), true);
    assert_eq!(app.initial_size(), (1584, 1146));
    resized(&mut app, 200, held, true);
    assert_eq!(saved(&mut app), (792, 573), "a held pick keeps the default");
    resized(&mut app, 100, held, true);
    resized(&mut app, 100, (792, 573), true);
    assert_eq!(app.initial_size(), (792, 573));
    assert_eq!(saved(&mut app), (792, 573));

    resized(&mut app, 100, (900, 600), false);
    assert_eq!(
        saved(&mut app),
        (900, 600),
        "a size the user gives is saved"
    );
    resized(&mut app, 200, (1800, 1000), true);
    assert_eq!(app.initial_size(), (1800, 1200));
    assert_eq!(
        saved(&mut app),
        (900, 600),
        "a held pick keeps the user's size"
    );
    resized(&mut app, 200, (1400, 900), false);
    assert_eq!(
        saved(&mut app),
        (700, 450),
        "a size the user gives at 200% is saved"
    );
    resized(&mut app, 100, (1400, 900), false);
    assert_eq!(app.initial_size(), (700, 450));
}

#[test]
fn boxes_and_the_sheet_grow_with_the_text_size() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    let sizes = |ex: &mut Explorer, ctx: &mut Ctx| {
        ex.run(commands::Command::About, ctx);
        let about = ex.children()[0].size;
        ex.take_up();
        ex.open_properties(dir.path(), "d", "d", ctx);
        let sheet = ex.children()[0].size;
        ex.props = None;
        (about, sheet)
    };
    let (about, sheet) = sizes(&mut ex, &mut ctx);
    ctx.text_size = 150;
    ex.follow_forms(&ctx);
    assert_eq!(ex.forms.fonts.size_px, 17);
    let u = reprisekit::theme::TextUnits::new(17, 11);
    assert_eq!(sizes(&mut ex, &mut ctx), (u.size(about), u.size(sheet)));

    ex.run(commands::Command::About, &mut ctx);
    let Some(Dialog::About(form, _)) = ex.dialog() else {
        panic!("the About box is up");
    };
    let banner = match &form.item("banner").expect("the banner").control {
        reprisekit::controls::form::Control::Picture { canvas, .. } => canvas.as_ref(),
        _ => None,
    };
    assert_eq!(
        banner.map(|c| c.width as i32),
        Some(u.size(about).0),
        "the banner spans the grown box"
    );
}

#[test]
fn under_glass_the_closing_line_runs_under_the_nav_row_and_the_header_under_the_commands() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ctx = test_ctx();
    ctx.theme = reprisekit::interfaces::theme_for(reprisekit::interface::Interface::Glass);
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.sv.mode = shellview::Mode::Details;
    let mut c = Canvas::new(792, 573);
    window::draw(&ex, &mut c, &ctx);
    let lay = ex.layout(&ctx);
    let sl = ex.shell_layout(&ctx).expect("the shell layout");
    assert_eq!(lay.etch_y, sl.nav.bottom(), "the line closes the nav row");
    for y in [lay.etch_y, lay.etch_y + 1] {
        assert_eq!(c.get(400, y) >> 24, 0xFF, "closing line row {y}");
    }
    let header = ex.view_layout(lay.view, &ctx).body;
    assert_eq!(
        header.y,
        sl.command.bottom() - 1,
        "the header's top row lies under the command bar's last, as under a rebar's line"
    );
}

#[test]
fn under_glass_the_nav_row_is_clear_over_the_frames_glass() {
    use reprisekit::interface::Interface;
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ctx = test_ctx();
    ctx.size = (792, 573);
    for look in [Interface::Classic, Interface::Blue] {
        ctx.theme = reprisekit::interfaces::theme_for(look);
        let ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
        assert_eq!(ex.glass_top(&ctx), 0, "{look:?} keeps its rebar opaque");
    }
    ctx.theme = reprisekit::interfaces::theme_for(Interface::Glass);
    let ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    let lay = ex.layout(&ctx);
    let sl = ex.shell_layout(&ctx).expect("the shell layout");
    assert_eq!(ex.glass_top(&ctx), 34, "the nav row");
    assert_eq!(lay.etch_y, 34, "the closing line sits under the glass");
    let mut c = Canvas::new(792, 573);
    window::draw(&ex, &mut c, &ctx);
    assert_eq!(c.take_glass_top(), 34, "the drawn row reports its rows");
    let gap = sl.back_fwd.right();
    assert_eq!(
        c.get(gap, 17) >> 24,
        0,
        "the glass between the pill and the address"
    );
    assert_eq!(c.get(400, 1) >> 24, 0, "above the fields");
    let search = sl.search;
    let face = c.get(search.x + 4, search.y + search.height / 2) >> 24;
    assert!(
        (0x80..0xFF).contains(&face),
        "the search field lets the glass through, alpha {face:02X}"
    );
    assert_eq!(c.get(400, lay.etch_y) >> 24, 0xFF, "the closing line");
}

#[test]
fn the_dialogs_take_the_renderer_a_font_change_loads() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ctx.text = TextRenderer::load(&FontConfig {
        family: vec!["DejaVu Sans".into()],
        ..FontConfig::default()
    })
    .into();
    ex.follow_forms(&ctx);
    assert!(Rc::ptr_eq(&ex.dialog_text, &ctx.text));
}

/// The bar's ring columns left of the splitter take it too, and the drag moves it from where
/// it was pressed.
#[test]
fn a_press_beside_the_splitter_grabs_it_where_it_lies() {
    use reprisekit::interface::Interface;
    let dir = tempfile::tempdir().expect("a temp dir");
    for look in [Interface::Classic, Interface::Blue] {
        for size in [100, 200] {
            let mut ctx = test_ctx();
            ctx.theme = reprisekit::interfaces::theme_for(look).with_text_size(size);
            ctx.size = (1600, 900);
            let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
            let bar = ex.layout(&ctx).bar.expect("the Folders bar");
            let (x, y) = (bar.body.right(), bar.body.y + 50);
            ex.pointer(
                repriseapp::PointerInput::Press {
                    x,
                    y,
                    button: crate::BTN_LEFT,
                    time: 0,
                    mods: repriseapp::Mods::default(),
                },
                &mut ctx,
            );
            ex.pointer(repriseapp::PointerInput::Motion { x: x + 40, y }, &mut ctx);
            let moved = ex.layout(&ctx).bar.expect("the Folders bar");
            assert_eq!(
                moved.splitter.x,
                bar.splitter.x + 40,
                "{look:?} at {size}%: the splitter follows the press"
            );
        }
    }
}

/// Glass, light or dark, at `pct` text.
fn glass_at(dark: bool, pct: u32) -> Ctx {
    let mut ctx = test_ctx();
    let g = reprisekit::glass::Settings {
        mode: match dark {
            true => reprisekit::glass::Mode::Dark,
            false => reprisekit::glass::Mode::Light,
        },
        ..Default::default()
    };
    let theme = reprisekit::interfaces::theme_with(
        reprisekit::interface::Interface::Glass,
        Default::default(),
        false,
        &g,
    );
    ctx.text = TextRenderer::load(&theme.fonts).into();
    ctx.theme = theme.with_text_size(pct);
    ctx.size = (1100, 600);
    ctx
}

/// At 200% the nav row's Back, Forward and drop, the search field's magnifier and clear button,
/// and the command bar's icons and drop arrows are twice their size, as the text is.
#[test]
fn at_200_percent_the_glass_nav_row_and_command_bar_grow_with_the_text() {
    use reprisekit::controls::searchfield;
    reprisekit::pins::fonts();
    let mut got = Vec::new();
    for dark in [false, true] {
        let mut ctx = glass_at(dark, 200);
        let dir = tempfile::tempdir().expect("a temp dir");
        let docs = dir.path().join("Documents");
        std::fs::create_dir(&docs).expect("mkdir");
        let mut ex = test_explorer(Place::Folder(docs), &mut ctx);
        ex.run(
            Command::ToggleLayout(crate::commands::LayoutPart::PreviewPane),
            &mut ctx,
        );
        let sl = ex.shell_layout(&ctx).expect("Glass takes the shell layout");
        assert_eq!((sl.buttons.back.width, sl.buttons.recent.width), (50, 30));
        assert_eq!(sl.back_fwd.width, 2 * (57 + 15));
        let clear = searchfield::clear_rect(sl.search, ctx.theme.text);
        assert_eq!(clear.width, 42);
        let look = if dark { "dark" } else { "light" };
        for s in [1, 2] {
            let mut c = Canvas::with_scale(1100, 600, s);
            ctx.at_scale(s, |ctx| window::draw(&ex, &mut c, ctx));
            for (part, r) in [
                ("buttons", sl.back_fwd),
                ("search", sl.search),
                ("command bar", sl.command),
            ] {
                got.push((
                    format!("{look} {part} @{s}"),
                    reprisekit::pins::fnv_in(&c, r),
                ));
            }
        }
    }
    let path = reprisekit::crate_dir!().join("tests/pins/text_200.txt");
    reprisekit::pins::check_file(&path, &got);
}
