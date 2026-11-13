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
    ex.dragging = true;
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
            select: None,
            connect: None,
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
            select: None,
            connect: None,
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
            select: None,
            connect: None,
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
            select: None,
            connect: None,
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
        ex.dialog = None;
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
    let Some(Dialog::About(form, _)) = &ex.dialog else {
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
fn under_glass_the_closing_line_sits_above_the_list_header() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ctx = test_ctx();
    ctx.theme = reprisekit::interfaces::theme_for(reprisekit::interface::Interface::Glass);
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.sv.mode = shellview::Mode::Details;
    let mut c = Canvas::new(792, 573);
    window::draw(&ex, &mut c, &ctx);
    let lay = ex.layout(&ctx);
    let header = ex.view_layout(lay.view, &ctx).body;
    let x = header.x + 4;
    let style = ctx.theme.look.style().expect("Glass has a style");
    let rebar = style.part(reprisekit::style::Part::Rebar).expect("a rebar");
    let line = rebar.atlas.get(0, rebar.atlas.height as i32 - 1);
    assert_eq!(
        c.get(x, lay.etch_y),
        line,
        "closing line at y {}",
        lay.etch_y
    );
    assert_eq!(header.y, lay.etch_y + 1, "the header starts under the line");
    assert_ne!(c.get(x, header.y), line, "the header's top row");
}

#[test]
fn under_glass_the_rebar_is_clear_over_the_frames_glass() {
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
    assert_eq!(ex.glass_top(&ctx), 78, "menu, toolbar and address bands");
    assert_eq!(lay.etch_y, 78, "the closing line sits under the glass");
    let mut c = Canvas::new(792, 573);
    window::draw(&ex, &mut c, &ctx);
    assert_eq!(c.take_glass_top(), 78, "the drawn rebar reports its rows");
    let gap = lay.menu.bottom();
    assert_eq!(c.get(400, gap) >> 24, 0, "the rows between bands are clear");
    assert_eq!(c.get(500, 40) >> 24, 0, "the toolbar band past its buttons");
    let search = window::search_field(&ex, &ctx).expect("the search field");
    assert_eq!(
        c.get(search.x + search.width / 2, search.y + search.height / 2) >> 24,
        0xFF,
        "the search field stays opaque"
    );
    let field = lay.address.expect("the address bar");
    assert_eq!(
        c.get(field.x + field.width / 2, field.y + field.height / 2) >> 24,
        0xFF,
        "the address field stays opaque"
    );
    let x = field.x + field.width / 2;
    let above = (0..field.y + field.height / 2)
        .rev()
        .find(|&y| c.get(x, y) >> 24 != 0xFF)
        .expect("the field starts below the toolbar");
    assert_eq!(
        [c.get(x, above) >> 24, c.get(x, above - 1) >> 24],
        [0, 0],
        "no glow over the field's border"
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
