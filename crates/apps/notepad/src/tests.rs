use super::*;
use crate::menu::{row, title};

#[test]
fn go_to_fits_its_text_under_every_interface() {
    let over = reprisekit::interfaces::form_overflows(|_, _, _| {
        vec![("Go To Line".into(), goto_form("12345"))]
    });
    assert!(over.is_empty(), "{}", over.join("\n"));
}
use repriseapp::filedlg::places;

#[test]
fn a_fallback_edit_font_is_antialiased() {
    let mono = edit_font();
    if mono.has_face("Lucida Console") {
        return;
    }
    let mut c = Canvas::new(120, 20);
    c.fill_rect(
        Rect::new(0, 0, 120, 20),
        reprisekit::theme::Color::rgb(255, 255, 255),
    );
    let r = Rect::new(0, 0, 120, 20);
    mono.draw(
        &mut c,
        r,
        "Things to do",
        false,
        reprisekit::theme::Color::rgb(0, 0, 0),
        reprisekit::text::Align::Left,
        EDIT_PX,
    );
    let grey = (0..20)
        .flat_map(|y| (0..120).map(move |x| (x, y)))
        .map(|(x, y)| c.get(x, y) & 0xff)
        .filter(|&v| v != 0 && v != 0xff)
        .count();
    assert!(
        grey > 0,
        "a fallback mono font draws grey edge pixels, not monochrome"
    );
}

fn label_of(item: &MenuItem) -> String {
    match item {
        MenuItem::Entry { label, .. } => label.clone(),
        MenuItem::Separator => panic!("that row is a separator"),
    }
}

fn label_at(t: usize, r: usize) -> String {
    label_of(&menus()[t].1[r])
}

#[test]
fn menu_constants_point_at_their_rows() {
    assert_eq!(label_at(title::FILE, row::NEW), "New");
    assert_eq!(label_at(title::FILE, row::SAVE), "Save");
    assert_eq!(label_at(title::FILE, row::EXIT), "Exit");
    assert_eq!(label_at(title::EDIT, row::UNDO), "Undo");
    assert_eq!(label_at(title::EDIT, row::FIND), "Find...");
    assert_eq!(label_at(title::EDIT, row::FIND_NEXT), "Find Next");
    assert_eq!(label_at(title::EDIT, row::REPLACE), "Replace...");
    assert_eq!(label_at(title::EDIT, row::GOTO), "Go To...");
    assert_eq!(label_at(title::EDIT, row::SELECT_ALL), "Select All");
    assert_eq!(label_at(title::EDIT, row::TIME_DATE), "Time/Date");
    assert_eq!(label_at(title::VIEW, row::STATUS_BAR), "Status Bar");
    assert_eq!(label_at(title::HELP, row::ABOUT), "About Notepad");
}

#[test]
fn the_bar_reads_the_caret_as_line_and_column() {
    let mut np = Notepad::new(None);
    assert_eq!(np.bar_text(), "Ln 1, Col 1");
    np.edit = MultiEdit::new("ab\tc");
    np.edit.place(multiedit::Pos { line: 0, col: 4 }, false);
    assert_eq!(np.bar_text(), "Ln 1, Col 5");
    np.edit = MultiEdit::new("one\ntwo");
    np.edit.place(multiedit::Pos { line: 1, col: 3 }, false);
    assert_eq!(np.bar_text(), "Ln 2, Col 4");
}

/// Showing the bar takes its height off the edit, so the two never overlap.
#[test]
fn showing_the_bar_shortens_the_edit() {
    let mut np = Notepad::new(None);
    assert!(!np.status_bar, "starts with the bar hidden");
    let tall = np.edit_height(400, menubar::BAR_H, 23);
    np.status_bar = true;
    let short = np.edit_height(400, menubar::BAR_H, 23);
    assert_eq!(tall - short, 23);
}

#[test]
fn the_status_bars_grip_resizes_the_window() {
    let mut ctx = test_ctx();
    let mut np = Notepad::new(None);
    np.status_bar = true;
    let (w, h) = ctx.size;
    let press = |x, y| PointerInput::Press {
        x,
        y,
        button: BTN_LEFT,
        time: 0,
        mods: repriseapp::Mods::default(),
    };
    assert!(np.pointer(press(w - 4, h - 4), &ctx).resize, "on the grip");
    assert!(
        !np.pointer(press(w - 40, h - 4), &ctx).resize,
        "on the panel beside it"
    );
    ctx.maximized = true;
    assert!(
        !np.pointer(press(w - 4, h - 4), &ctx).resize,
        "no grip while maximized"
    );
}

#[test]
fn a_double_click_in_the_text_selects_a_word_within_the_configured_time() {
    let mut ctx = test_ctx();
    ctx.theme.metrics.double_click_ms = 900;
    let mut np = Notepad::new(None);
    np.edit = MultiEdit::new("alpha beta");
    let el = np.edit_layout(&ctx);
    let (x, y) = (el.inner.x + 4, el.inner.y + 4);
    let press = |x, y, time| PointerInput::Press {
        x,
        y,
        button: BTN_LEFT,
        time,
        mods: repriseapp::Mods::default(),
    };
    let release = PointerInput::Release {
        x,
        y,
        button: BTN_LEFT,
        mods: repriseapp::Mods::default(),
    };
    np.pointer(press(x, y, 1000), &ctx);
    np.pointer(release, &ctx);
    np.pointer(press(x + 4, y, 1800), &ctx);
    assert_eq!(np.edit.selected_text(), "alpha ", "800 ms and 4 px apart");
    np.pointer(release, &ctx);
    np.pointer(press(x, y, 3000), &ctx);
    np.pointer(release, &ctx);
    np.pointer(press(x + 5, y, 3100), &ctx);
    assert!(!np.edit.has_selection(), "5 px apart is two clicks");
}

/// Opening Go To declares the child window; closing it withdraws it.
#[test]
fn go_to_opens_and_closes_as_a_child() {
    let mut np = Notepad::new(None);
    assert!(np.children().is_empty(), "nothing is open to start with");
    np.goto = Some(goto_form("1"));
    let kids = np.children();
    assert_eq!(kids.len(), 1);
    assert_eq!(kids[0].id, GOTO);
    assert!(kids[0].modal, "Go To is modal");
    assert!(kids[0].fixed, "and cannot be resized");
    np.goto = None;
    assert!(np.children().is_empty());
}

/// A line number outside the document does not move the caret.
#[test]
fn go_to_only_accepts_a_line_that_exists() {
    let mut np = Notepad::new(None);
    np.edit = MultiEdit::new("one\ntwo\nthree");
    assert!(np.go_to_line("2"), "line 2 exists");
    assert_eq!(np.edit.caret(), multiedit::Pos { line: 1, col: 0 });
    assert!(!np.go_to_line("9"), "line 9 does not");
    assert_eq!(
        np.edit.caret(),
        multiedit::Pos { line: 1, col: 0 },
        "and the caret stayed"
    );
    assert!(!np.go_to_line("x"), "nor does a non-number");
}

fn tr() -> TextRenderer {
    TextRenderer::load(&FontConfig::default())
}

#[test]
fn find_next_selects_a_match_and_reports_a_miss() {
    let mut np = Notepad::new(None);
    np.edit = MultiEdit::new("hello world hello");
    np.last_query = Some((
        "hello".into(),
        multiedit::FindOpts {
            match_case: true,
            down: true,
        },
    ));
    np.edit.place(multiedit::Pos { line: 0, col: 1 }, false);
    assert!(np.find_next());
    assert_eq!(
        np.edit.selection(),
        (
            multiedit::Pos { line: 0, col: 12 },
            multiedit::Pos { line: 0, col: 17 },
        )
    );
    assert!(np.notfound.is_none(), "a hit raises no box");

    np.last_query = Some((
        "zzz".into(),
        multiedit::FindOpts {
            match_case: true,
            down: true,
        },
    ));
    assert!(!np.find_next());
    assert_eq!(
        np.notfound.as_ref().map(|b| b.fields.text.as_str()),
        Some("Cannot find \"zzz\""),
        "a miss names the query"
    );
    assert_eq!(
        np.edit.selection(),
        (
            multiedit::Pos { line: 0, col: 12 },
            multiedit::Pos { line: 0, col: 17 },
        ),
        "a miss does not move the caret"
    );
}

#[test]
fn find_rows_grey_on_an_empty_buffer() {
    let mut np = Notepad::new(None);
    let on = |np: &Notepad, r: usize| match &np.bar.menus[title::EDIT][r] {
        MenuItem::Entry { enabled, .. } => *enabled,
        MenuItem::Separator => panic!("that row is a separator"),
    };
    np.sync_menus();
    assert!(!on(&np, row::FIND), "Find is greyed with no text");
    assert!(!on(&np, row::FIND_NEXT));
    assert!(on(&np, row::REPLACE), "Replace stays live");
    assert!(on(&np, row::GOTO), "so does Go To");
    np.edit = MultiEdit::new("text");
    np.sync_menus();
    assert!(on(&np, row::FIND), "text enables Find");
    assert!(on(&np, row::FIND_NEXT));
}

#[test]
fn edit_rows_grey_until_there_is_a_selection_or_an_edit_to_undo() {
    let mut np = Notepad::new(None);
    np.edit = MultiEdit::new("text");
    let mut ctx = test_ctx();
    let on = |np: &Notepad, r: usize| match &np.bar.menus[title::EDIT][r] {
        MenuItem::Entry { enabled, .. } => *enabled,
        MenuItem::Separator => panic!("that row is a separator"),
    };
    let key = |edit| {
        Event::Key(KeyInput {
            sym: 0,
            utf8: None,
            mods: repriseapp::Mods::default(),
            edit: Some(edit),
        })
    };
    np.event(MAIN, Event::Focus(true), &mut ctx);
    for r in [row::UNDO, row::CUT, row::COPY, row::DELETE] {
        assert!(
            !on(&np, r),
            "row {r} is greyed with no selection and no edit"
        );
    }
    assert!(
        !on(&np, row::PASTE),
        "Paste is greyed with no text to paste"
    );
    assert!(
        !np.command(title::EDIT, row::PASTE, &ctx).paste,
        "a greyed Paste asks for nothing"
    );
    repriseapp::fake_clipboard_text(true);
    np.event(MAIN, Event::Focus(true), &mut ctx);
    assert!(
        on(&np, row::PASTE),
        "text on the clipboard makes Paste live"
    );
    assert!(np.command(title::EDIT, row::PASTE, &ctx).paste);
    repriseapp::fake_clipboard_text(false);
    assert_eq!(
        np.command(title::EDIT, row::COPY, &ctx).copy,
        None,
        "a greyed Copy copies nothing"
    );

    np.event(MAIN, key(EditKey::SelectAll), &mut ctx);
    for r in [row::CUT, row::COPY, row::DELETE] {
        assert!(on(&np, r), "row {r} is live over a selection");
    }
    assert!(!on(&np, row::UNDO), "selecting is not an edit");

    np.event(MAIN, key(EditKey::Insert("x".into())), &mut ctx);
    assert!(on(&np, row::UNDO), "typing makes Undo live");
    assert!(!on(&np, row::CUT), "the typed text replaced the selection");
}

fn right_press(x: i32, y: i32) -> PointerInput {
    PointerInput::Press {
        x,
        y,
        button: BTN_RIGHT,
        time: 0,
        mods: repriseapp::Mods::default(),
    }
}

fn typed(sym: u32, utf8: Option<&str>, shift: bool) -> Event {
    Event::Key(KeyInput {
        sym,
        utf8: utf8.map(str::to_string),
        mods: repriseapp::Mods {
            shift,
            ..Default::default()
        },
        edit: None,
    })
}

/// Where a press lands on the gap before `col` on the first line.
fn at_col(np: &Notepad, ctx: &Ctx, col: usize) -> (i32, i32) {
    let el = np.edit_layout(ctx);
    let x = multiedit::x_of(
        &np.mono,
        &np.metrics,
        &np.edit,
        multiedit::Pos { line: 0, col },
    );
    (el.inner.x + 1 + x - np.view.left, el.inner.y + 2)
}

fn context_rows(np: &Notepad) -> Vec<bool> {
    let tree = np
        .context
        .as_ref()
        .map(|m| m.tree())
        .expect("the context menu is open");
    tree.iter()
        .filter_map(|item| match item {
            MenuItem::Entry { enabled, .. } => Some(*enabled),
            MenuItem::Separator => None,
        })
        .collect()
}

#[test]
fn a_right_click_keeps_the_selection_under_it_and_moves_the_caret_outside_it() {
    let mut ctx = test_ctx();
    let mut np = Notepad::new(None);
    np.edit = MultiEdit::new("alpha beta gamma");
    np.edit.place(multiedit::Pos { line: 0, col: 6 }, false);
    np.edit.place(multiedit::Pos { line: 0, col: 10 }, true);

    let (x, y) = at_col(&np, &ctx, 8);
    np.event(MAIN, Event::Pointer(right_press(x, y)), &mut ctx);
    assert_eq!(np.edit.selected_text(), "beta", "the selection stays");
    assert_eq!(
        context_rows(&np),
        [false, true, true, false, true, true],
        "Undo and Paste greyed, Cut, Copy, Delete and Select All live"
    );
    np.event(MAIN, typed(keysym::ESCAPE, None, false), &mut ctx);
    assert!(np.context.is_none(), "Escape closes the menu");

    let (x, y) = at_col(&np, &ctx, 2);
    np.event(MAIN, Event::Pointer(right_press(x, y)), &mut ctx);
    assert!(
        !np.edit.has_selection(),
        "a click off the selection drops it"
    );
    assert_eq!(np.edit.caret(), multiedit::Pos { line: 0, col: 2 });
    assert_eq!(context_rows(&np), [false, false, false, false, false, true]);

    let (x, y) = at_col(&np, &ctx, 0);
    np.event(MAIN, Event::Pointer(right_press(x, y)), &mut ctx);
    assert!(np.context.is_none(), "a press off the menu only closes it");
    assert_eq!(np.edit.caret(), multiedit::Pos { line: 0, col: 2 });
}

#[test]
fn a_right_click_during_a_text_drag_ends_the_drag() {
    let mut ctx = test_ctx();
    let mut np = Notepad::new(None);
    np.edit = MultiEdit::new("alpha beta gamma");
    let (x0, y) = at_col(&np, &ctx, 0);
    let (x3, _) = at_col(&np, &ctx, 3);
    let (x8, _) = at_col(&np, &ctx, 8);
    np.event(
        MAIN,
        Event::Pointer(PointerInput::Press {
            x: x0,
            y,
            button: BTN_LEFT,
            time: 0,
            mods: repriseapp::Mods::default(),
        }),
        &mut ctx,
    );
    np.event(
        MAIN,
        Event::Pointer(PointerInput::Motion { x: x3, y }),
        &mut ctx,
    );
    np.event(MAIN, Event::Pointer(right_press(x3, y)), &mut ctx);
    assert!(np.context.is_some());
    np.event(
        MAIN,
        Event::Pointer(PointerInput::Release {
            x: x3,
            y,
            button: BTN_LEFT,
            mods: repriseapp::Mods::default(),
        }),
        &mut ctx,
    );
    np.event(MAIN, typed(keysym::ESCAPE, None, false), &mut ctx);
    np.event(
        MAIN,
        Event::Pointer(PointerInput::Motion { x: x8, y }),
        &mut ctx,
    );
    assert_eq!(
        np.edit.selected_text(),
        "alp",
        "motion with no button held after the menu leaves the selection"
    );
}

#[test]
fn the_context_menu_runs_the_edit_commands() {
    let mut ctx = test_ctx();
    let mut np = Notepad::new(None);
    np.edit = MultiEdit::new("alpha");
    repriseapp::fake_clipboard_text(true);
    let (x, y) = at_col(&np, &ctx, 1);
    let open = |np: &mut Notepad, ctx: &mut Ctx| {
        np.event(MAIN, Event::Pointer(right_press(x, y)), ctx);
    };

    open(&mut np, &mut ctx);
    assert!(
        context_rows(&np)[3],
        "text on the clipboard makes Paste live"
    );
    np.event(MAIN, typed(0x61, Some("a"), false), &mut ctx);
    assert!(np.context.is_none());
    assert_eq!(np.edit.selected_text(), "alpha", "Select All");

    open(&mut np, &mut ctx);
    let copy = np.event(MAIN, typed(0x63, Some("c"), false), &mut ctx);
    assert_eq!(copy.copy.as_deref(), Some("alpha"));

    open(&mut np, &mut ctx);
    let paste = np.event(MAIN, typed(0x70, Some("p"), false), &mut ctx);
    assert!(paste.paste);

    open(&mut np, &mut ctx);
    np.event(MAIN, typed(0x64, Some("d"), false), &mut ctx);
    assert_eq!(np.edit.text(), "", "Delete");
    assert!(np.dirty);

    open(&mut np, &mut ctx);
    np.event(MAIN, typed(0x75, Some("u"), false), &mut ctx);
    assert_eq!(np.edit.text(), "alpha", "Undo");
    repriseapp::fake_clipboard_text(false);
}

#[test]
fn the_open_menu_goes_to_the_host_and_the_window_draws_without_it() {
    use reprisekit::controls::hosted::Anchor;
    let mut ctx = test_ctx();
    let mut np = Notepad::new(None);
    let mut before = Canvas::new(ctx.size.0 as u32, ctx.size.1 as u32);
    np.draw_main(&mut before, &ctx);
    assert!(np.popup(MAIN, &ctx).unwrap().spots().is_empty());

    np.event(MAIN, typed(keysym::F10, None, true), &mut ctx);
    let caret = np.context.as_ref().unwrap().rect();
    let spots = np.popup(MAIN, &ctx).unwrap().spots();
    assert_eq!(spots.len(), 1);
    assert_eq!(spots[0].anchor, Anchor::Point(caret.x, caret.y));
    np.popup(MAIN, &ctx).unwrap().host();
    let mut after = Canvas::new(ctx.size.0 as u32, ctx.size.1 as u32);
    np.draw_main(&mut after, &ctx);
    assert!(
        before.pixels == after.pixels,
        "the context menu is left out"
    );
    np.event(MAIN, typed(keysym::ESCAPE, None, false), &mut ctx);

    np.event(MAIN, typed(keysym::F10, None, false), &mut ctx);
    np.event(MAIN, typed(keysym::DOWN, None, false), &mut ctx);
    let lay = np.bar_layout(&ctx);
    let spots = np.popup(MAIN, &ctx).unwrap().spots();
    assert_eq!(spots[0].anchor, Anchor::Below(lay.frames[0]), "File's menu");
    np.popup(MAIN, &ctx).unwrap().host();
    let mut open = Canvas::new(ctx.size.0 as u32, ctx.size.1 as u32);
    np.draw_main(&mut open, &ctx);
    let r = spots[0].rect;
    assert_eq!(
        open.get(r.x + 10, r.bottom() - 4),
        before.get(r.x + 10, r.bottom() - 4),
        "the bar's menu is left out"
    );
    np.popup(MAIN, &ctx).unwrap().dismiss();
    assert!(np.bar.open.is_none(), "a dismissed menu closes the bar");
}

#[test]
fn a_narrow_window_s_menus_keep_their_rows_whole() {
    let widths = |size: (i32, i32)| {
        let mut ctx = test_ctx();
        ctx.size = size;
        ctx.output_size = Some((1920, 1080));
        let mut np = Notepad::new(None);
        np.event(MAIN, typed(keysym::F10, None, false), &mut ctx);
        np.event(MAIN, typed(keysym::DOWN, None, false), &mut ctx);
        let file = np.popup(MAIN, &ctx).unwrap().spots()[0].rect.width;
        np.event(MAIN, typed(keysym::ESCAPE, None, false), &mut ctx);
        np.event(MAIN, typed(keysym::ESCAPE, None, false), &mut ctx);
        np.event(MAIN, typed(keysym::F10, None, true), &mut ctx);
        let context = np.popup(MAIN, &ctx).unwrap().spots()[0].rect.width;
        (file, context)
    };
    let wide = widths((800, 600));
    assert!(wide.0 > 60 && wide.1 > 60, "rows need {wide:?} px");
    assert_eq!(widths((60, 600)), wide, "a 60 px window's menus as wide");
}

#[test]
fn shift_f10_opens_the_context_menu_under_the_caret_and_it_draws_over_the_text() {
    let mut ctx = test_ctx();
    let mut np = Notepad::new(None);
    np.edit = MultiEdit::new("alpha\nbeta");
    np.edit.place(multiedit::Pos { line: 1, col: 2 }, false);
    let mut before = Canvas::new(ctx.size.0 as u32, ctx.size.1 as u32);
    np.draw_main(&mut before, &ctx);

    np.event(MAIN, typed(keysym::F10, None, true), &mut ctx);
    assert!(np.bar.open.is_none(), "Shift+F10 is not the menu bar's F10");
    let chain = np.context.as_ref().expect("Shift+F10 opens the menu");
    let el = np.edit_layout(&ctx);
    let caret_x = multiedit::x_of(&np.mono, &np.metrics, &np.edit, np.edit.caret());
    assert_eq!(
        (chain.rect().x, chain.rect().y),
        (
            el.inner.x + 1 + caret_x,
            el.inner.y + 1 + 2 * np.metrics.line_h
        ),
        "the menu's corner sits under the caret"
    );
    let r = chain.rect();
    let mut after = Canvas::new(ctx.size.0 as u32, ctx.size.1 as u32);
    np.draw_main(&mut after, &ctx);
    let changed = (r.y..r.bottom())
        .flat_map(|y| (r.x..r.right()).map(move |x| (x, y)))
        .filter(|&(x, y)| before.get(x, y) != after.get(x, y))
        .count();
    assert!(changed > 0, "the menu draws over the text");
    assert_eq!(
        before.get(r.right() + 40, r.y - 4),
        after.get(r.right() + 40, r.y - 4),
        "and nowhere else"
    );

    np.event(MAIN, Event::Focus(false), &mut ctx);
    assert!(np.context.is_none(), "losing the keyboard closes it");
    np.event(MAIN, typed(keysym::MENU, None, false), &mut ctx);
    assert!(np.context.is_some(), "the Menu key opens it too");
}

/// Find's Cancel lights under the pointer and goes cold when the pointer leaves; held and
/// dragged off, it draws at rest and does not act on the release.
#[test]
fn a_find_button_lights_and_a_held_one_dragged_off_does_not_act() {
    use reprisekit::controls::form::Light;
    use reprisekit::pointer::Interaction;
    let mut find = Some(finddlg::find_form(&finddlg::FindFields {
        query: "zzz".into(),
        match_case: false,
        down: true,
    }));
    let (text, px) = (tr(), 11);
    let blue = reprisekit::interfaces::theme_for(reprisekit::interface::Interface::Blue);
    let f = find.as_ref().unwrap();
    let mut c = Canvas::new(400, 200);
    form::draw(&mut c, &text, &blue.paint(), f, false, px);
    let cancel = f.index_of(finddlg::CANCEL).unwrap();
    let r = f.items[cancel].rect;
    let (x, y) = (r.x + 5, r.y + 5);
    let lit = |f: &Option<Form>| f.as_ref().unwrap().lit(cancel, Light::Body);
    let step = |f: &mut Option<Form>, ev: Event| form_event(f, &ev, &text, px, true);
    let motion = |x, y| Event::Pointer(PointerInput::Motion { x, y });
    assert!(matches!(
        step(&mut find, motion(x, y)),
        FormStep::Out(form::Event::Redraw)
    ));
    assert_eq!(lit(&find), Interaction::Hot);
    let leave = Event::Pointer(PointerInput::Leave);
    assert!(matches!(
        step(&mut find, leave),
        FormStep::Out(form::Event::Redraw)
    ));
    assert_eq!(lit(&find), Interaction::Idle);
    let press = Event::Pointer(PointerInput::Press {
        x,
        y,
        button: BTN_LEFT,
        time: 0,
        mods: repriseapp::Mods::default(),
    });
    step(&mut find, press);
    assert_eq!(lit(&find), Interaction::Pressed);
    assert!(matches!(
        step(&mut find, motion(1, 1)),
        FormStep::Out(form::Event::Redraw)
    ));
    assert_eq!(lit(&find), Interaction::Idle, "dragged off, it is at rest");
    let release = Event::Pointer(PointerInput::Release {
        x: 1,
        y: 1,
        button: BTN_LEFT,
        mods: repriseapp::Mods::default(),
    });
    assert!(matches!(
        step(&mut find, release),
        FormStep::Out(form::Event::Redraw)
    ));
}

#[test]
fn a_selection_drag_held_below_the_edit_scrolls_on_ticks_until_released() {
    let mut ctx = test_ctx();
    let mut np = Notepad::new(None);
    np.edit = MultiEdit::new(&"line\n".repeat(200));
    let lay = np.edit_layout(&ctx);
    let x = lay.inner.x + 20;
    np.pointer(
        PointerInput::Press {
            x,
            y: lay.inner.y + 4,
            button: BTN_LEFT,
            time: 0,
            mods: repriseapp::Mods::default(),
        },
        &ctx,
    );
    let below = lay.inner.bottom() + 30;
    let r = np.pointer(PointerInput::Motion { x, y: below }, &ctx);
    assert!(r.tick && r.redraw);
    assert_eq!(np.view.top, 0);
    for top in 1..=3 {
        let r = np.event(MAIN, Event::Tick, &mut ctx);
        assert!(r.tick && r.redraw);
        assert_eq!(np.view.top, top);
    }
    assert_eq!(np.edit.caret().line, 3 + lay.rows - 1);
    np.pointer(
        PointerInput::Release {
            x,
            y: below,
            button: BTN_LEFT,
            mods: repriseapp::Mods::default(),
        },
        &ctx,
    );
    let r = np.event(MAIN, Event::Tick, &mut ctx);
    assert!(!r.tick, "released, the ticks stop");
    assert_eq!(np.view.top, 3);
}

#[test]
fn under_glass_the_menu_bar_is_clear_over_the_frames_glass() {
    use reprisekit::interface::Interface;
    let mut ctx = test_ctx();
    ctx.size = (400, 300);
    let np = Notepad::new(None);
    for (look, rows) in [
        (Interface::Classic, 0),
        (Interface::Blue, 0),
        (Interface::Glass, 1),
    ] {
        ctx.theme = reprisekit::interfaces::theme_for(look);
        let mut c = Canvas::new(400, 300);
        np.draw_main(&mut c, &ctx);
        let bar = np.bar_layout(&ctx).bar;
        let alpha = c.get(390, bar.y + bar.height / 2) >> 24;
        assert_eq!(
            c.take_glass_top(),
            rows * bar.bottom(),
            "{look:?} glass rows"
        );
        assert_eq!(
            alpha,
            0xFF * (1 - rows as u32),
            "{look:?} bar past its titles"
        );
        assert_eq!(
            c.get(200, bar.bottom() + 20) >> 24,
            0xFF,
            "{look:?} the text box"
        );
    }
}

#[test]
fn the_text_boxs_scrollbar_lights_under_the_pointer_and_a_leave_clears_it() {
    use reprisekit::pointer::Interaction;
    let mut ctx = test_ctx();
    ctx.theme = reprisekit::interfaces::theme_for(reprisekit::interface::Interface::Blue);
    let mut np = Notepad::new(None);
    np.metrics.follow_look(&ctx.theme.look.controls());
    np.edit = MultiEdit::new(&"line\n".repeat(200));
    let sb = np
        .edit_layout(&ctx)
        .vscroll
        .expect("200 lines overflow the box");
    let (x, y) = (sb.x + 8, sb.y + 2);
    let up = (Axis::Vertical, ScrollHit::Back);
    assert!(np.pointer(PointerInput::Motion { x, y }, &ctx).redraw);
    assert_eq!(np.view.bars.of(up), Interaction::Hot);
    let press = PointerInput::Press {
        x,
        y,
        button: BTN_LEFT,
        time: 0,
        mods: repriseapp::Mods::default(),
    };
    np.pointer(press, &ctx);
    assert_eq!(np.view.bars.of(up), Interaction::Pressed);
    let release = PointerInput::Release {
        x,
        y,
        button: BTN_LEFT,
        mods: repriseapp::Mods::default(),
    };
    assert!(np.pointer(release, &ctx).redraw);
    assert_eq!(np.view.bars.of(up), Interaction::Hot);
    assert!(np.pointer(PointerInput::Leave, &ctx).redraw);
    assert_eq!(np.view.bars.lit(), None);
}

#[test]
fn the_text_boxs_thumb_drags_and_its_track_pages() {
    let ctx = test_ctx();
    let mut np = Notepad::new(None);
    np.edit = MultiEdit::new(&"line\n".repeat(200));
    let el = np.edit_layout(&ctx);
    let sb = el.vscroll.expect("200 lines overflow the box");
    let last = np.edit.lines().len() - el.rows;
    let press = |x, y| PointerInput::Press {
        x,
        y,
        button: BTN_LEFT,
        time: 0,
        mods: repriseapp::Mods::default(),
    };
    let release = PointerInput::Release {
        x: 0,
        y: 0,
        button: BTN_LEFT,
        mods: repriseapp::Mods::default(),
    };
    np.pointer(press(sb.x + 8, sb.bottom() - 20), &ctx);
    assert_eq!(np.view.top, el.rows, "the track below the thumb: a page");
    np.pointer(release, &ctx);
    let thumb_y = |np: &Notepad| {
        let el = np.edit_layout(&ctx);
        (sb.y..sb.bottom())
            .find(|&y| {
                matches!(
                    multiedit::bar_hit(&np.mono, &np.metrics, &np.edit, &el, np.view, sb.x + 8, y),
                    Some((_, widgets::BarHit::Thumb(_)))
                )
            })
            .expect("a thumb")
    };
    let y = thumb_y(&np);
    np.pointer(press(sb.x + 8, y), &ctx);
    assert_eq!(np.view.top, el.rows, "the press on the thumb moves nothing");
    np.pointer(PointerInput::Motion { x: 0, y: 5000 }, &ctx);
    assert_eq!(np.view.top, last, "dragged past the end, off the bar");
    np.pointer(release, &ctx);
    np.pointer(PointerInput::Motion { x: sb.x + 8, y }, &ctx);
    assert_eq!(np.view.top, last, "let go: the motion moves nothing");
}

#[test]
fn the_not_found_box_is_its_own_child() {
    let mut np = Notepad::new(None);
    np.find = Some(finddlg::find_form(&finddlg::FindFields {
        query: "zzz".into(),
        match_case: false,
        down: true,
    }));
    np.notfound = Some(np.notfound_box("zzz"));
    let ids: Vec<_> = np.children().iter().map(|c| c.id).collect();
    assert!(ids.contains(&FIND));
    assert!(ids.contains(&NOTFOUND));
    let box_child = np
        .children()
        .into_iter()
        .find(|c| c.id == NOTFOUND)
        .expect("the box");
    assert!(box_child.modal, "the box is modal");
    let find_child = np
        .children()
        .into_iter()
        .find(|c| c.id == FIND)
        .expect("find");
    assert!(!find_child.modal, "Find is modeless");
}

#[test]
fn replace_swaps_the_match_then_advances() {
    let mut np = Notepad::new(None);
    np.edit = MultiEdit::new("a a a");
    np.last_query = Some((
        "a".into(),
        multiedit::FindOpts {
            match_case: true,
            down: true,
        },
    ));
    assert!(np.find_next(), "select the first match");
    np.replace_once("a", "b", true);
    assert_eq!(np.edit.text(), "b a a");

    // Select "a" directly and ask to replace "z": the selection does
    // not match the query, so the guard must refuse to swap it.
    np.edit.place(multiedit::Pos { line: 0, col: 2 }, false);
    np.edit.place(multiedit::Pos { line: 0, col: 3 }, true);
    assert_eq!(np.edit.selected_text(), "a");
    np.replace_once("z", "x", true);
    assert_eq!(
        np.edit.text(),
        "b a a",
        "a query that misses the selection replaces nothing"
    );
}

/// With Match Case off, the search can select text differing in case from the query; Replace
/// must still recognise it as the match rather than silently skipping past it.
#[test]
fn replace_once_ignores_case_when_match_case_is_off() {
    let mut np = Notepad::new(None);
    np.edit = MultiEdit::new("Cat cat");
    np.last_query = Some((
        "cat".into(),
        multiedit::FindOpts {
            match_case: false,
            down: true,
        },
    ));
    assert!(np.find_next(), "case-insensitively selects \"Cat\"");
    assert_eq!(np.edit.selected_text(), "Cat");
    np.replace_once("cat", "dog", false);
    assert_eq!(np.edit.text(), "dog cat");
}

/// Replace All rewrites the document and counts, and never opens the not-found box for a query
/// it did replace.
#[test]
fn replace_all_rewrites_the_document() {
    let mut np = Notepad::new(None);
    np.edit = MultiEdit::new("a a\na");
    let n = np.replace_every(
        "a",
        "bb",
        &multiedit::FindOpts {
            match_case: true,
            down: true,
        },
    );
    assert_eq!(n, 3);
    assert_eq!(np.edit.text(), "bb bb\nbb");
    assert!(np.notfound.is_none());
}

#[test]
fn replace_and_find_do_not_share_the_screen() {
    let mut np = Notepad::new(None);
    np.edit = MultiEdit::new("text");
    np.open_find();
    assert!(np.find.is_some());
    np.open_replace();
    assert!(np.replace.is_some());
    assert!(np.find.is_none(), "Find gave up the slot");
    np.open_find();
    assert!(np.replace.is_none(), "and takes it back");
}

#[test]
fn about_shows_notepads_product_row_and_icon() {
    let mut np = Notepad::new(None);
    np.app_icon = Some(Canvas::new(32, 32));
    let f = np.about_fields();
    assert_eq!(f.product, "Notepad");
    assert!(f.icon.is_some(), "Notepad fills the icon slot");
    assert!(
        !f.memory_label.is_empty(),
        "the system rows are filled, not blank"
    );
    let form = aboutbox::form(&f, &tr(), 11);
    assert!(form.item("icon").is_some());
}

#[test]
fn about_opens_as_a_fixed_modal_child() {
    let mut np = Notepad::new(None);
    np.about = true;
    let kid = np
        .children()
        .into_iter()
        .find(|c| c.id == ABOUT)
        .expect("the about child");
    assert_eq!(kid.size, aboutbox::CLIENT);
    assert!(kid.fixed && kid.modal);
}

#[test]
fn dialogs_grow_with_the_text_size() {
    let mut np = Notepad::new(None);
    let mut ctx = test_ctx();
    ctx.text_size = 150;
    np.follow(&ctx);
    assert_eq!(np.forms.fonts.size_px, 17);
    assert_eq!(np.forms.text, reprisekit::theme::TextUnits::new(17, 11));
    np.open_find();
    np.goto = Some(goto_form("1"));
    let size = |id| np.children().into_iter().find(|c| c.id == id).unwrap().size;
    assert_eq!(size(FIND), (547, 158));
    assert_eq!(size(GOTO), (414, 147));
}

#[test]
fn the_window_opens_at_the_text_size() {
    let mut np = Notepad::new(None);
    assert_eq!(np.initial_size(), (568, 328));
    let mut ctx = test_ctx();
    ctx.text_size = 200;
    np.follow(&ctx);
    assert_eq!(np.initial_size(), (1136, 656));
}

/// At 200% text the window's theme is at the text size: the menu bar and the status bar
/// grow, the text box takes what is left, and Open opens at its grown size.
#[test]
fn the_bars_and_the_file_dialog_grow_with_the_text_size() {
    let mut np = Notepad::new(None);
    let mut ctx = test_ctx();
    ctx.text_size = 200;
    ctx.theme = Theme::default().with_text_size(200);
    np.follow(&ctx);
    np.status_bar = true;
    let r = np.edit_rect(&ctx);
    assert_eq!((r.y, r.bottom()), (40, 328 - 44));
    let (bar, sr) = np.status(&ctx).expect("the status bar is on");
    assert_eq!(sr, Rect::new(0, 328 - 44, 568, 44));
    let panels = statusbar::layout(&bar, sr, ctx.theme.text);
    assert_eq!(panels[1].width, 712);
    np.open_dialog();
    let kid = np.children().into_iter().find(|c| c.id == OPEN).unwrap();
    assert_eq!(kid.size, (2 * CLIENT.0, 2 * CLIENT.1));
}

#[test]
fn typing_marks_the_document_dirty() {
    let mut np = Notepad::new(None);
    assert!(!np.dirty, "a fresh document is clean");
    np.edit = MultiEdit::new("x");
    np.dirty = true;
    let tmp = tempfile::tempdir().expect("tempdir");
    np.path = Some(tmp.path().join("dirty.txt"));
    np.save();
    assert!(!np.dirty, "a successful save cleans it");
}

#[test]
fn new_on_a_dirty_document_prompts() {
    let mut np = Notepad::new(None);
    np.dirty = true;
    let kids_before = np.children().len();
    np.pending = Pending::waiting(PendingAction::New);
    np.savechanges = true;
    assert_eq!(np.children().len(), kids_before + 1);
    assert_eq!(np.children().last().expect("the prompt").id, SAVECHANGES);
}

#[test]
fn no_discards_and_runs_the_pending_action() {
    let mut np = Notepad::new(None);
    np.dirty = true;
    np.pending = Pending::waiting(PendingAction::Quit);
    np.savechanges = true;
    let reply = np.savechanges_outcome(form::Event::Pressed(msgbox::button_id(1)));
    assert!(reply.quit);
    assert!(!np.pending.is_waiting());
}

#[test]
fn arrows_move_the_save_prompt_s_focus_and_enter_presses_it() {
    let mut np = Notepad::new(None);
    np.dirty = true;
    np.confirm_then(PendingAction::Quit);
    let key = |sym, edit| KeyInput {
        sym,
        utf8: None,
        mods: repriseapp::Mods::default(),
        edit,
    };
    let right = || {
        Some(EditKey::Right {
            select: false,
            word: false,
        })
    };
    let mut ctx = test_ctx();
    np.event(
        SAVECHANGES,
        Event::Key(key(keysym::RIGHT, right())),
        &mut ctx,
    );
    assert_eq!(np.savechanges_focus, 1, "Right moves from Yes to No");
    np.event(
        SAVECHANGES,
        Event::Key(key(keysym::RIGHT, right())),
        &mut ctx,
    );
    np.event(
        SAVECHANGES,
        Event::Key(key(keysym::RIGHT, right())),
        &mut ctx,
    );
    assert_eq!(np.savechanges_focus, 2, "Right stops at Cancel");
    np.event(SAVECHANGES, Event::Key(key(keysym::RETURN, None)), &mut ctx);
    assert!(!np.savechanges, "Enter on Cancel closed the prompt");
    assert!(np.dirty, "Cancel kept the changes");
    assert!(!np.pending.is_waiting(), "Cancel abandoned the quit");
}

#[test]
fn cancel_on_the_prompt_abandons_the_action() {
    let mut np = Notepad::new(None);
    np.dirty = true;
    np.pending = Pending::waiting(PendingAction::Quit);
    np.savechanges = true;
    let reply = np.savechanges_outcome(form::Event::Pressed(msgbox::button_id(2)));
    assert!(!reply.quit);
    assert!(np.dirty, "still dirty");
    assert!(!np.pending.is_waiting());
}

#[test]
fn a_clean_document_exits_without_a_prompt() {
    let mut np = Notepad::new(None);
    assert!(!np.dirty);
    let reply = np.confirm_then(PendingAction::Quit);
    assert!(reply.quit, "a clean document quits immediately");
    assert!(np.children().is_empty(), "no prompt opens");
}

pub(crate) fn test_ctx() -> Ctx {
    Ctx {
        theme: Theme::default(),
        text: tr().into(),
        icons: reprisekit::icontheme::IconCache::new(reprisekit::icontheme::IconTheme::load(
            "test",
            &[],
        )),
        caret_on: true,
        size: (568, 328),
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

#[test]
fn a_typed_path_into_a_missing_folder_shows_a_box_and_keeps_the_dialog() {
    let dir = std::env::temp_dir();
    for save in [false, true] {
        let mut np = Notepad::new(None);
        np.host.folder = Some(np.host.folders.id(&dir));
        if save {
            np.saveas = Some(sample_filedlg());
            np.saveas_accept("reprise-no-such-dir/x.txt", &mut test_icons());
            assert!(np.saveas.is_some(), "Save As stays open");
        } else {
            np.open = Some(sample_filedlg());
            np.open_accept("reprise-no-such-dir/x.txt", &mut test_icons());
            assert!(np.open.is_some(), "Open stays open");
        }
        assert!(np.error.is_some(), "a box says the folder is missing");
    }
}

fn test_icons() -> IconCache {
    IconCache::new(reprisekit::icontheme::IconTheme::load("test", &[]))
}

/// A `FileDlg` in the state `open_outcome`/`saveas_outcome` expect to find one in mid-flow:
/// unlisted (no test here needs a listing) but otherwise exactly what
/// `open_dialog`/`saveas_dialog` build.
fn sample_filedlg() -> FileDlg {
    FileDlg::new(
        Fields {
            title: "Open".into(),
            accept_label: "Open".into(),
            mode: DlgMode::Open,
            multiple: false,
            filters: vec!["Text Documents (*.txt)".into(), "All Files (*.*)".into()],
        },
        places(),
    )
}

#[test]
fn a_crlf_file_is_saved_with_its_breaks() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("crlf.txt");
    std::fs::write(&path, "one\r\ntwo\r\n").expect("write");
    let mut np = Notepad::new(Some(path.clone()));
    assert_eq!(np.edit.text(), "one\ntwo\n", "the edit itself holds LF");
    assert!(np.save());
    assert_eq!(std::fs::read(&path).expect("read"), b"one\r\ntwo\r\n");

    let lf = tmp.path().join("lf.txt");
    std::fs::write(&lf, "one\ntwo\n").expect("write");
    let mut np = Notepad::new(Some(lf.clone()));
    assert!(np.save());
    assert_eq!(std::fs::read(&lf).expect("read"), b"one\ntwo\n");

    let mixed = tmp.path().join("mixed.txt");
    std::fs::write(&mixed, "one\r\ntwo\nthree").expect("write");
    let mut np = Notepad::new(Some(mixed.clone()));
    assert!(np.save());
    assert_eq!(
        std::fs::read(&mixed).expect("read"),
        b"one\r\ntwo\r\nthree",
        "a mixed file saves CRLF throughout"
    );
}

#[test]
fn write_document_keeps_mode_and_symlinks() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path();
    let real = dir.join("real.txt");
    std::fs::write(&real, "old").expect("write");
    std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o600)).expect("chmod");
    write_document(&real, &["new"], "\n").expect("save");
    assert_eq!(std::fs::read_to_string(&real).expect("read"), "new");
    assert_eq!(
        std::fs::metadata(&real).expect("stat").permissions().mode() & 0o777,
        0o600
    );

    let link = dir.join("link.txt");
    std::os::unix::fs::symlink(&real, &link).expect("symlink");
    write_document(&link, &["via link"], "\n").expect("save");
    assert!(
        std::fs::symlink_metadata(&link)
            .expect("lstat")
            .file_type()
            .is_symlink()
    );
    assert_eq!(std::fs::read_to_string(&real).expect("read"), "via link");
    assert!(std::fs::read_dir(dir).expect("dir").flatten().all(|e| {
        !e.file_name()
            .to_string_lossy()
            .starts_with(".reprise-notepad-")
    }));
}

/// A save through a symlink renames a new file over the link's target, so the link stays and a
/// failed write never truncates the target.
#[test]
fn write_document_replaces_a_symlink_target_and_writes_hard_links_in_place() {
    use std::os::unix::fs::MetadataExt;
    let dir = std::env::temp_dir().join(format!("reprise-notepad-links-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("store")).expect("mkdir");
    let real = dir.join("store/doc.txt");
    let link = dir.join("doc.txt");
    std::fs::write(&real, "old").expect("write");
    std::os::unix::fs::symlink("store/doc.txt", &link).expect("symlink");
    let before = std::fs::metadata(&real).expect("stat").ino();

    write_document(&link, &["new"], "\n").expect("save");

    let lstat = std::fs::symlink_metadata(&link).expect("lstat");
    assert!(lstat.file_type().is_symlink(), "the link stays a symlink");
    assert_eq!(std::fs::read_to_string(&real).expect("read"), "new");
    assert_ne!(
        std::fs::metadata(&real).expect("stat").ino(),
        before,
        "the target is a new inode renamed over the old one"
    );

    let other = dir.join("store/other.txt");
    std::fs::hard_link(&real, &other).expect("hard link");
    let before = std::fs::metadata(&real).expect("stat").ino();
    write_document(&other, &["both"], "\n").expect("save");
    assert_eq!(std::fs::metadata(&real).expect("stat").ino(), before);
    assert_eq!(std::fs::read_to_string(&real).expect("read"), "both");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn write_document_keeps_xattrs() {
    let dir = std::env::temp_dir().join(format!("reprise-notepad-xattr-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    let doc = dir.join("doc.txt");
    std::fs::write(&doc, "old").expect("write");
    // A file system without `user.` xattrs has nothing to keep.
    if reprisefs::apps::set_trusted(&doc).is_err() {
        std::fs::remove_dir_all(&dir).ok();
        return;
    }

    write_document(&doc, &["new"], "\n").expect("save");

    assert_eq!(std::fs::read_to_string(&doc).expect("read"), "new");
    assert!(reprisefs::apps::is_trusted(&doc), "user.* xattr kept");
    std::fs::remove_dir_all(&dir).ok();
}

/// A symlink planted at a temp-file name beside the document is never followed: the file it
/// points at keeps its bytes.
#[test]
fn write_document_leaves_a_planted_temp_symlink_alone() {
    let dir = std::env::temp_dir().join(format!("reprise-notepad-plant-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    let doc = dir.join("doc.txt");
    let victim = dir.join("victim.txt");
    std::fs::write(&doc, "old").expect("write");
    std::fs::write(&victim, "keep").expect("write");
    let planted = dir.join(format!(".doc.txt.{}.tmp", std::process::id()));
    std::os::unix::fs::symlink(&victim, &planted).expect("symlink");

    write_document(&doc, &["new"], "\n").expect("save");

    assert_eq!(std::fs::read_to_string(&doc).expect("read"), "new");
    assert_eq!(std::fs::read_to_string(&victim).expect("read"), "keep");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn write_document_refuses_a_read_only_file() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("reprise-notepad-ro-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    let doc = dir.join("doc.txt");
    std::fs::write(&doc, "old").expect("write");
    std::fs::set_permissions(&doc, std::fs::Permissions::from_mode(0o444)).expect("chmod");
    // Root writes through any mode, so there is nothing to check.
    if std::fs::OpenOptions::new().write(true).open(&doc).is_ok() {
        std::fs::remove_dir_all(&dir).ok();
        return;
    }

    let err = write_document(&doc, &["new"], "\n").expect_err("save must fail");

    assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
    assert_eq!(std::fs::read_to_string(&doc).expect("read"), "old");
    assert_eq!(std::fs::read_dir(&dir).expect("dir").count(), 1);
    std::fs::remove_dir_all(&dir).ok();
}

/// Yes on the save prompt with a path that cannot be written raises the error box and forgets
/// the action, so Exit does not sit armed.
#[test]
fn failed_save_on_the_prompt_raises_the_box_and_drops_the_action() {
    let mut np = Notepad::new(None);
    np.path = Some(std::env::temp_dir().join("reprise-no-such-dir/x.txt"));
    np.dirty = true;
    np.pending = Pending::waiting(PendingAction::Quit);
    np.savechanges = true;
    let reply = np.savechanges_yes();
    assert!(!reply.quit);
    assert!(!np.pending.is_waiting());
    assert!(np.dirty);
    assert!(
        np.error
            .as_ref()
            .is_some_and(|b| b.fields.text.starts_with("Cannot create the "))
    );
    assert_eq!(np.children().last().expect("the box").id, ERRORBOX);
}

#[test]
fn a_file_that_is_not_utf8_opens_as_an_untitled_copy() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path();
    let latin1 = dir.join("latin1.txt");
    std::fs::write(&latin1, b"caf\xe9").expect("write");
    let mut np = Notepad::new(Some(latin1.clone()));
    assert_eq!(np.path, None);
    assert_eq!(np.edit.text(), "");
    assert_eq!(np.children().last().expect("the prompt").id, LOSSY);
    let enter = KeyInput {
        sym: keysym::RETURN,
        utf8: None,
        mods: repriseapp::Mods::default(),
        edit: None,
    };
    let mut ctx = test_ctx();
    np.event(LOSSY, Event::Key(enter), &mut ctx);
    assert!(np.lossy.is_none(), "Yes closed the prompt");
    assert_eq!(np.edit.text(), "caf\u{fffd}");
    assert_eq!(np.path, None);
    assert_eq!(std::fs::read(&latin1).expect("read"), b"caf\xe9");

    let missing = dir.join("missing.txt");
    let np = Notepad::new(Some(missing.clone()));
    assert_eq!(np.path, Some(missing));
}

#[test]
fn a_device_or_folder_on_the_command_line_raises_the_box() {
    for p in ["/dev/zero", "/"] {
        let np = Notepad::new(Some(PathBuf::from(p)));
        assert_eq!(np.path, None);
        assert_eq!(np.edit.text(), "");
        let text = &np.error.as_ref().expect("the box").fields.text;
        assert_eq!(text, &format!("Cannot open {p}.\nIt is not a file."));
    }
}

/// A name the Open dialog cannot read raises the box over the dialog, which stays up with the
/// document untouched.
#[test]
fn a_failed_open_from_the_dialog_raises_the_box_over_it() {
    let mut np = Notepad::new(None);
    np.open_dialog();
    let dir = std::env::temp_dir();
    np.host.folder = Some(np.host.folders.id(&dir));
    let missing = dir.join("reprise-notepad-open-missing.txt");
    std::fs::remove_file(&missing).ok();
    np.open_accept("reprise-notepad-open-missing.txt", &mut test_icons());
    assert!(np.open.is_some(), "the dialog stays up");
    assert_eq!(np.path, None);
    assert_eq!(
        np.error.as_ref().expect("the box").fields.text,
        format!(
            "{}\nFile not found.\nPlease verify the correct file name was given.",
            missing.display()
        )
    );
}

#[test]
fn a_listed_subdirectory_carries_a_resolvable_id() {
    let mut np = Notepad::new(None);
    let dir = std::env::temp_dir();
    let real = np.host.folders.id(&dir);
    assert_eq!(np.host.folders.path(real), Some(dir.as_path()));
    let child = np.host.folders.id(&dir.join("child"));
    assert_ne!(child, real, "a second path gets a handle of its own");
    assert_eq!(
        np.host.folders.path(child),
        Some(dir.join("child").as_path())
    );
}

/// File > Save on a never-saved document opens Save As.
#[test]
fn save_on_an_untitled_document_opens_save_as() {
    let ctx = test_ctx();
    let mut np = Notepad::new(None);
    assert!(np.path.is_none());
    np.command(title::FILE, row::SAVE, &ctx);
    assert!(np.saveas.is_some(), "Save As opened");
}

#[test]
fn the_handle_table_resolves_folder_ids() {
    let mut np = Notepad::new(None);
    let home = std::env::temp_dir();
    let id = np.host.folders.id(&home);
    assert_eq!(np.host.folders.path(id), Some(home.as_path()));
    assert_eq!(
        np.host.folders.id(&home),
        id,
        "the same path gets the same handle"
    );
}

/// Opening the dialog declares the child window.
#[test]
fn open_declares_its_child_window() {
    let mut np = Notepad::new(None);
    assert!(np.children().is_empty());
    np.open = Some(sample_filedlg());
    let kids = np.children();
    assert_eq!(kids.len(), 1);
    assert_eq!(kids[0].id, OPEN);
    assert_eq!(kids[0].size, reprisekit::controls::filedlg::CLIENT);
}

#[test]
fn save_as_writes_adopts_the_path_and_runs_the_pending_action() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path();
    let mut np = Notepad::new(None);
    np.edit = MultiEdit::new("hello");
    np.dirty = true;
    np.pending = Pending::waiting(PendingAction::Quit);
    np.saveas = Some(sample_filedlg());
    let id = np.host.folders.id(dir);
    np.host.folder = Some(id);
    let reply = np.saveas_outcome(
        reprisekit::controls::filedlg::Event::Accept(vec!["out.txt".into()]),
        &mut test_icons(),
    );
    let written = dir.join("out.txt");
    assert_eq!(std::fs::read_to_string(&written).expect("written"), "hello");
    assert_eq!(np.path.as_deref(), Some(written.as_path()));
    assert!(!np.dirty);
    assert!(reply.quit, "the pending quit runs once the save succeeds");
}

#[test]
fn save_as_onto_an_existing_file_asks_before_replacing() {
    let dir = std::env::temp_dir();
    let taken = dir.join(format!("reprise-replace-{}.txt", std::process::id()));
    std::fs::write(&taken, "old").expect("write");
    let name = taken.file_name().unwrap().to_string_lossy().into_owned();
    let mut np = Notepad::new(None);
    np.edit = MultiEdit::new("new");
    np.dirty = true;
    np.saveas = Some(sample_filedlg());
    let id = np.host.folders.id(&dir);
    np.host.folder = Some(id);
    np.saveas_outcome(
        reprisekit::controls::filedlg::Event::Accept(vec![name.clone()]),
        &mut test_icons(),
    );
    assert_eq!(np.overwrite.as_deref(), Some(taken.as_path()));
    assert_eq!(
        std::fs::read_to_string(&taken).unwrap(),
        "old",
        "nothing written yet"
    );
    assert!(np.saveas.is_some(), "Save As stays open behind the prompt");
    assert_eq!(np.children().last().expect("the prompt").id, OVERWRITE);
    let f = np.overwrite_fields(&taken);
    assert_eq!(f.buttons, ["Yes", "No"]);
    assert_eq!(f.default, 1, "defaults to No");

    np.overwrite_outcome(form::Event::Default);
    assert!(np.overwrite.is_none());
    assert_eq!(std::fs::read_to_string(&taken).unwrap(), "old");
    assert!(np.saveas.is_some());

    np.saveas_outcome(
        reprisekit::controls::filedlg::Event::Accept(vec![name]),
        &mut test_icons(),
    );
    np.overwrite_outcome(form::Event::Pressed(msgbox::button_id(0)));
    assert_eq!(std::fs::read_to_string(&taken).unwrap(), "new");
    assert_eq!(np.path.as_deref(), Some(taken.as_path()));
    assert!(np.overwrite.is_none());
    assert!(np.saveas.is_none());
    let _ = std::fs::remove_file(&taken);
}

#[test]
fn an_arrow_moves_the_replace_prompt_s_focus_and_enter_presses_it() {
    let dir = std::env::temp_dir();
    let taken = dir.join(format!("reprise-replace-keys-{}.txt", std::process::id()));
    std::fs::write(&taken, "old").expect("write");
    let name = taken.file_name().unwrap().to_string_lossy().into_owned();
    let mut np = Notepad::new(None);
    np.edit = MultiEdit::new("new");
    np.saveas = Some(sample_filedlg());
    let id = np.host.folders.id(&dir);
    np.host.folder = Some(id);
    np.saveas_outcome(
        reprisekit::controls::filedlg::Event::Accept(vec![name]),
        &mut test_icons(),
    );
    let key = |sym, edit| {
        Event::Key(KeyInput {
            sym,
            utf8: None,
            mods: repriseapp::Mods::default(),
            edit,
        })
    };
    let left = || {
        Some(EditKey::Left {
            select: false,
            word: false,
        })
    };
    let mut ctx = test_ctx();
    np.event(OVERWRITE, key(keysym::LEFT, left()), &mut ctx);
    assert_eq!(np.overwrite_focus, 0, "Left moves from No to Yes");
    np.event(OVERWRITE, key(keysym::LEFT, left()), &mut ctx);
    assert_eq!(np.overwrite_focus, 0, "Left stops at Yes");
    np.event(OVERWRITE, key(keysym::RETURN, None), &mut ctx);
    assert!(np.overwrite.is_none(), "Enter closed the prompt");
    assert_eq!(
        std::fs::read_to_string(&taken).unwrap(),
        "new",
        "Enter on Yes wrote over the file"
    );
    let _ = std::fs::remove_file(&taken);
}

#[test]
fn save_as_onto_a_dangling_symlink_asks_before_replacing() {
    let dir = std::env::temp_dir();
    let link = dir.join(format!("reprise-dangling-{}.txt", std::process::id()));
    let _ = std::fs::remove_file(&link);
    std::os::unix::fs::symlink(dir.join("reprise-no-such-target"), &link).expect("symlink");
    let name = link.file_name().unwrap().to_string_lossy().into_owned();
    let mut np = Notepad::new(None);
    np.saveas = Some(sample_filedlg());
    np.host.folder = Some(np.host.folders.id(&dir));
    np.saveas_outcome(
        reprisekit::controls::filedlg::Event::Accept(vec![name]),
        &mut test_icons(),
    );
    assert_eq!(np.overwrite.as_deref(), Some(link.as_path()));
    let _ = std::fs::remove_file(&link);
}

#[test]
fn open_replaces_the_buffer() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path();
    let p = dir.join("reprise-open-test.txt");
    std::fs::write(&p, "loaded").expect("write");
    let mut np = Notepad::new(None);
    np.open = Some(sample_filedlg());
    let id = np.host.folders.id(dir);
    np.host.folder = Some(id);
    np.open_outcome(
        reprisekit::controls::filedlg::Event::Accept(vec!["reprise-open-test.txt".into()]),
        &mut test_icons(),
    );
    assert_eq!(np.edit.text(), "loaded");
    assert_eq!(np.path.as_deref(), Some(p.as_path()));
    assert!(!np.dirty);
}

#[test]
fn cancelling_save_as_abandons_the_pending_quit() {
    let mut np = Notepad::new(None);
    np.edit = MultiEdit::new("hello");
    np.dirty = true;
    np.confirm_then(PendingAction::Quit);
    assert!(np.savechanges, "the save prompt opened first");
    let reply = np.savechanges_outcome(form::Event::Default);
    assert!(np.saveas.is_some(), "an untitled document opens Save As");
    assert!(np.pending.is_waiting(), "still pending, waiting on Save As");
    assert!(!reply.quit);
    let reply = np.saveas_outcome(
        reprisekit::controls::filedlg::Event::Cancel,
        &mut test_icons(),
    );
    assert!(np.saveas.is_none());
    assert!(
        !np.pending.is_waiting(),
        "cancelling abandons the pending quit"
    );
    assert!(!reply.quit);
}

#[test]
fn entering_a_listed_subfolder_lists_it() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path();
    let sub = dir.join("Sub");
    std::fs::create_dir_all(&sub).expect("mkdir");
    std::fs::write(sub.join("inside.txt"), "from the subfolder").expect("write");

    let mut np = Notepad::new(None);
    np.path = Some(dir.join("placeholder.txt"));
    np.open_dialog();
    let mut ctx = test_ctx();
    np.open_event(Event::Blink, &mut ctx);
    let parent_id = np.host.folder.expect("start_dir was listed");
    let sub_id = np.host.folders.id(&sub);
    assert_ne!(sub_id, parent_id);

    let enter = KeyInput {
        sym: keysym::RETURN,
        utf8: None,
        mods: repriseapp::Mods::default(),
        edit: None,
    };
    np.open_event(Event::Key(enter.clone()), &mut ctx);
    assert_eq!(
        np.open.as_ref().expect("still open").folder(),
        Some(sub_id),
        "the dialog now believes it is listing the subfolder"
    );

    np.open_event(Event::Key(enter), &mut ctx);
    assert_eq!(np.edit.text(), "from the subfolder");
    assert_eq!(np.path.as_deref(), Some(sub.join("inside.txt").as_path()));
}

fn key(sym: u32) -> Event {
    Event::Key(KeyInput {
        sym,
        utf8: None,
        mods: repriseapp::Mods::default(),
        edit: None,
    })
}

#[test]
fn the_file_type_filters_open_and_save_as() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path();
    std::fs::create_dir(dir.join("Sub")).expect("mkdir");
    for f in ["a.txt", "bash", "b.conf"] {
        std::fs::write(dir.join(f), "").expect("write");
    }
    for save in [false, true] {
        let mut np = Notepad::new(None);
        np.path = Some(dir.join("a.txt"));
        let mut ctx = test_ctx();
        let mut send = |np: &mut Notepad, ev: Event| {
            if save {
                np.saveas_event(ev, &mut ctx);
            } else {
                np.open_event(ev, &mut ctx);
            }
        };
        if save {
            np.saveas_dialog();
        } else {
            np.open_dialog();
        }
        let names = |np: &Notepad| {
            let dlg = if save { &np.saveas } else { &np.open };
            let mut n: Vec<String> = dlg
                .as_ref()
                .expect("open")
                .entries()
                .iter()
                .map(|e| e.name.clone())
                .collect();
            n.sort();
            n
        };
        send(&mut np, Event::Blink);
        assert_eq!(
            names(&np),
            ["Sub", "a.txt"],
            "Text Documents (save: {save})"
        );
        send(&mut np, key(keysym::TAB));
        send(&mut np, key(keysym::DOWN));
        assert_eq!(
            names(&np),
            ["Sub", "a.txt", "b.conf", "bash"],
            "All Files (save: {save})"
        );
    }
}

#[test]
fn a_typed_wildcard_filters_the_open_dialog() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path();
    for f in ["a.txt", "bash"] {
        std::fs::write(dir.join(f), "").expect("write");
    }
    let mut np = Notepad::new(None);
    np.path = Some(dir.join("a.txt"));
    np.open_dialog();
    let mut ctx = test_ctx();
    np.open_event(Event::Blink, &mut ctx);
    np.open_event(
        Event::Key(KeyInput {
            sym: '*' as u32,
            utf8: Some("*".into()),
            mods: repriseapp::Mods::default(),
            edit: Some(EditKey::Insert("*".into())),
        }),
        &mut ctx,
    );
    np.open_event(key(keysym::RETURN), &mut ctx);
    assert!(np.error.is_none(), "no box");
    let dlg = np.open.as_ref().expect("the dialog stays open");
    assert_eq!(dlg.file_name(), "*");
    let mut names: Vec<&str> = dlg.entries().iter().map(|e| e.name.as_str()).collect();
    names.sort();
    assert_eq!(names, ["a.txt", "bash"]);
}

#[test]
fn the_query_field_keeps_its_caret_between_keystrokes() {
    let mut np = Notepad::new(None);
    let ctx = test_ctx();
    np.open_find();
    let insert = |ch: char| form::Key::Edit(EditKey::Insert(ch.to_string()));
    let left = form::Key::Edit(EditKey::Left {
        select: false,
        word: false,
    });

    for ch in "helo".chars() {
        let out = np.find.as_mut().expect("find is open").key(insert(ch));
        np.find_outcome(out, &ctx);
    }
    assert_eq!(
        Notepad::field(np.find.as_ref().unwrap(), finddlg::QUERY),
        "helo"
    );

    let out = np.find.as_mut().unwrap().key(left);
    np.find_outcome(out, &ctx);

    let out = np.find.as_mut().unwrap().key(insert('l'));
    np.find_outcome(out, &ctx);
    assert_eq!(
        Notepad::field(np.find.as_ref().unwrap(), finddlg::QUERY),
        "hello",
        "the \"l\" lands where the caret sits"
    );
}

#[test]
fn ctrl_f_is_refused_on_an_empty_buffer_but_opens_find_on_text() {
    let mut np = Notepad::new(None);
    let ctx = test_ctx();
    let ctrl_f = KeyInput {
        sym: 0x66,
        utf8: Some("f".into()),
        mods: repriseapp::Mods {
            ctrl: true,
            shift: false,
            alt: false,
        },
        edit: None,
    };
    np.accel(&ctrl_f, &ctx);
    assert!(
        np.find.is_none(),
        "Find is greyed on an empty buffer; Ctrl+F must not open it"
    );

    np.edit = MultiEdit::new("text");
    np.sync_menus();
    np.accel(&ctrl_f, &ctx);
    assert!(np.find.is_some(), "text enables Find; Ctrl+F opens it");
}

#[test]
fn replace_that_matches_nothing_does_not_scroll() {
    let ctx = test_ctx();
    let mut np = Notepad::new(None);
    np.edit = MultiEdit::new(&"line\n".repeat(200));
    np.view.top = 50;
    np.replace = Some(finddlg::replace_form(&finddlg::ReplaceFields {
        query: "zzz".into(),
        with: "x".into(),
        match_case: true,
    }));
    np.replace_outcome(form::Event::Pressed(finddlg::REPLACE), &ctx);
    assert_eq!(np.view.top, 50, "nothing matched, nothing scrolled");
}
