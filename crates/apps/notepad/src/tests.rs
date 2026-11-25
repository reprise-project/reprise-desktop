use super::*;
use crate::menu::{row, title};
use reprisekit::controls::filedlg::Event as FileEvent;
use reprisekit::icontheme::IconCache;
use reprisekit::theme::FontConfig;

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
    assert_eq!(label_at(title::FORMAT, row::WORD_WRAP), "Word Wrap");
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
    assert!(np.asks.is_empty(), "a hit raises no box");

    np.last_query = Some((
        "zzz".into(),
        multiedit::FindOpts {
            match_case: true,
            down: true,
        },
    ));
    assert!(!np.find_next());
    assert_eq!(
        np.asks.up().map(|(b, _)| b.fields.text.as_str()),
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
    for r in [row::UNDO, row::REDO, row::CUT, row::COPY, row::DELETE] {
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
    assert!(!on(&np, row::REDO), "nothing undone yet");

    np.event(MAIN, key(EditKey::Undo), &mut ctx);
    assert_eq!(np.edit.text(), "text");
    assert!(on(&np, row::REDO), "an undo makes Redo live");
    np.command(title::EDIT, row::REDO, &ctx);
    np.sync_menus();
    assert_eq!(np.edit.text(), "x", "Redo makes the typing again");
    assert!(!on(&np, row::REDO));
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
        &el,
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
        [false, false, true, true, false, true, true],
        "Undo, Redo and Paste greyed, Cut, Copy, Delete and Select All live"
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
    assert_eq!(
        context_rows(&np),
        [false, false, false, false, false, false, true]
    );

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
        context_rows(&np)[4],
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
    let caret_x = multiedit::x_of(&np.mono, &np.metrics, &np.edit, &el, np.edit.caret());
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
    np.metrics.follow_look(&ctx.theme.look.controls_at_100());
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
    let b = np.notfound_box("zzz");
    np.asks.ask(b, Ask::NotFound);
    let ids: Vec<_> = np.children().iter().map(|c| c.id).collect();
    assert!(ids.contains(&FIND));
    assert!(ids.contains(&np.asks.win().unwrap()));
    let box_child = np
        .children()
        .into_iter()
        .find(|c| Some(c.id) == np.asks.win())
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
fn a_miss_covers_the_window_that_searched() {
    let mut np = Notepad::new(None);
    let mut ctx = test_ctx();
    np.edit = MultiEdit::new("text");
    np.sync_menus();
    let opts = multiedit::FindOpts {
        match_case: false,
        down: true,
    };
    np.last_query = Some(("zzz".into(), opts));
    np.open_find();
    np.event(FIND, key(keysym::RETURN), &mut ctx);
    assert_eq!(np.parent(box_win(&np)), Some(FIND), "a miss from Find");
    np.answer_box(None);

    np.event(MAIN, key(keysym::F3), &mut ctx);
    assert_eq!(np.parent(box_win(&np)), Some(MAIN), "F3 in the document");
    np.answer_box(None);

    np.open_replace();
    np.event(REPLACE_WIN, key(keysym::RETURN), &mut ctx);
    assert_eq!(
        np.parent(box_win(&np)),
        Some(REPLACE_WIN),
        "a miss from Replace"
    );
    np.answer_box(None);
    assert_eq!(np.edit.text(), "text");
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
    assert!(np.asks.is_empty());
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
fn moving_the_caret_or_selection_leaves_a_saved_document_clean() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path().join("recovery");
    let mut ctx = test_ctx();
    let mut np = Notepad::new(None);
    np.rescue_dir = Some(dir.clone());
    np.edit = MultiEdit::new("alpha beta\ngamma");
    np.event(MAIN, Event::Focus(true), &mut ctx);
    let key = |sym, ctrl, shift, edit| {
        Event::Key(KeyInput {
            sym,
            utf8: None,
            mods: repriseapp::Mods {
                ctrl,
                shift,
                ..Default::default()
            },
            edit,
        })
    };
    let right = EditKey::Right {
        select: false,
        word: false,
    };
    np.event(
        MAIN,
        key(keysym::RIGHT, false, false, Some(right)),
        &mut ctx,
    );
    np.event(MAIN, key(keysym::DOWN, false, false, None), &mut ctx);
    np.event(MAIN, key(keysym::HOME, true, false, None), &mut ctx);
    let shift_end = EditKey::End { select: true };
    np.event(
        MAIN,
        key(keysym::END, false, true, Some(shift_end)),
        &mut ctx,
    );
    assert_eq!(np.edit.selected_text(), "alpha beta");
    np.event(MAIN, key(0, false, false, Some(EditKey::Undo)), &mut ctx);
    let (x, y) = at_col(&np, &ctx, 2);
    let (to, _) = at_col(&np, &ctx, 6);
    let mods = repriseapp::Mods::default();
    for p in [
        PointerInput::Press {
            x,
            y,
            button: BTN_LEFT,
            time: 0,
            mods,
        },
        PointerInput::Motion { x: to, y },
        PointerInput::Release {
            x: to,
            y,
            button: BTN_LEFT,
            mods,
        },
        PointerInput::Scroll { x, y, rows: 1 },
    ] {
        np.event(MAIN, Event::Pointer(p), &mut ctx);
    }
    assert_eq!(np.edit.selected_text(), "pha ", "the drag selected");
    assert!(!np.dirty, "no key or click changed the text");
    assert!(np.event(MAIN, Event::CloseRequest, &mut ctx).quit);
    np.rescue();
    assert!(recovered_in(&dir).is_empty(), "nothing to recover");

    let ch = key(0x78, false, false, Some(EditKey::Insert("x".into())));
    np.event(MAIN, ch, &mut ctx);
    assert!(np.dirty, "typing is a change");
    np.rescue();
    assert_eq!(recovered_in(&dir).len(), 1);
}

#[test]
fn new_on_a_dirty_document_prompts() {
    let mut np = Notepad::new(None);
    np.dirty = true;
    let kids_before = np.children().len();
    np.confirm_then(PendingAction::New);
    assert_eq!(np.children().len(), kids_before + 1);
    box_win(&np);
    assert!(matches!(np.asks.up(), Some((_, Ask::SaveChanges))));
}

#[test]
fn no_discards_and_runs_the_pending_action() {
    let mut np = Notepad::new(None);
    np.dirty = true;
    np.confirm_then(PendingAction::Quit);
    let reply = np.answer_box(Some(1));
    assert!(reply.quit);
    assert!(!np.pending.is_waiting());
}

#[test]
fn arrows_move_the_save_prompt_s_focus_and_enter_presses_it() {
    let mut np = Notepad::new(None);
    np.dirty = true;
    np.confirm_then(PendingAction::Quit);
    let win = box_win(&np);
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
    np.event(win, Event::Key(key(keysym::RIGHT, right())), &mut ctx);
    assert_eq!(focus(&np), 1, "Right moves from Yes to No");
    np.event(win, Event::Key(key(keysym::RIGHT, right())), &mut ctx);
    np.event(win, Event::Key(key(keysym::RIGHT, right())), &mut ctx);
    assert_eq!(focus(&np), 2, "Right stops at Cancel");
    np.event(win, Event::Key(key(keysym::RETURN, None)), &mut ctx);
    assert!(np.asks.is_empty(), "Enter on Cancel closed the prompt");
    assert!(np.dirty, "Cancel kept the changes");
    assert!(!np.pending.is_waiting(), "Cancel abandoned the quit");
}

#[test]
fn cancel_on_the_prompt_abandons_the_action() {
    let mut np = Notepad::new(None);
    np.dirty = true;
    np.confirm_then(PendingAction::Quit);
    let reply = np.answer_box(Some(2));
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
        let msg = np
            .host
            .asking()
            .expect("a box over the dialog says the folder is missing");
        assert_eq!(msg.title, sample_filedlg().title());
        assert!(np.asks.is_empty(), "not one over the main window");
    }
}

fn test_icons() -> IconCache {
    IconCache::new(reprisekit::icontheme::IconTheme::load("test", &[]))
}

/// The dialogs' hosts past `FileDlg`'s own step, driven with what the dialog gave out.
impl Notepad {
    fn open_outcome(&mut self, out: FileEvent, icons: &mut IconCache) -> Reply {
        let Some(dlg) = self.open.as_mut() else {
            return Reply::none();
        };
        let driven = self.host.handle(dlg, out, icons);
        self.open_driven(driven)
    }

    fn saveas_outcome(&mut self, out: FileEvent, icons: &mut IconCache) -> Reply {
        let Some(dlg) = self.saveas.as_mut() else {
            return Reply::none();
        };
        let driven = self.host.handle(dlg, out, icons);
        self.saveas_driven(driven)
    }

    fn open_accept(&mut self, name: &str, icons: &mut IconCache) -> Reply {
        self.open_outcome(FileEvent::Accept(vec![name.into()]), icons)
    }

    fn saveas_accept(&mut self, name: &str, icons: &mut IconCache) -> Reply {
        self.saveas_outcome(FileEvent::Accept(vec![name.into()]), icons)
    }
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
fn bom_utf16_and_cr_files_save_back_byte_for_byte() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let utf16le: Vec<u8> = [0xFF, 0xFE]
        .into_iter()
        .chain(
            "h\u{e9}\r\n\u{1F600}"
                .encode_utf16()
                .flat_map(u16::to_le_bytes),
        )
        .collect();
    let utf16be: Vec<u8> = [0xFE, 0xFF]
        .into_iter()
        .chain(
            "h\u{e9}\n\u{1F600}\n"
                .encode_utf16()
                .flat_map(u16::to_be_bytes),
        )
        .collect();
    let cases: [(&str, Vec<u8>, &str); 4] = [
        ("bom.txt", b"\xEF\xBB\xBFone\r\ntwo".to_vec(), "one\ntwo"),
        ("le.txt", utf16le, "h\u{e9}\n\u{1F600}"),
        ("be.txt", utf16be, "h\u{e9}\n\u{1F600}\n"),
        ("cr.txt", b"one\rtwo\r".to_vec(), "one\ntwo\n"),
    ];
    for (name, bytes, edited) in cases {
        let path = tmp.path().join(name);
        std::fs::write(&path, &bytes).expect("write");
        let mut np = Notepad::new(Some(path.clone()));
        assert_eq!(np.edit.text(), edited, "{name} opens without its BOM");
        std::fs::write(&path, b"").expect("truncate");
        np.stamp = Stamp::of(&path);
        assert!(np.save(), "{name} saves");
        assert_eq!(std::fs::read(&path).expect("read"), bytes, "{name}");
    }
}

#[test]
fn save_as_writes_the_encoding_picked_and_the_document_keeps_it() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("plain.txt");
    std::fs::write(&path, "a\r\nb").expect("write");
    let mut np = Notepad::new(Some(path.clone()));
    np.saveas_dialog();
    let encoding = |np: &Notepad| {
        np.saveas
            .as_ref()
            .and_then(FileDlg::extra)
            .and_then(|f| f.combo_selected("encoding"))
    };
    assert_eq!(encoding(&np), Some(0), "Save As starts on UTF-8");
    let form = np
        .saveas
        .as_mut()
        .and_then(FileDlg::extra_mut)
        .expect("row");
    if let Some(Control::Combo(c)) = form.item_mut("encoding").map(|i| &mut i.control) {
        c.selected = 3;
    }
    let out = tmp.path().join("be.txt");
    np.saveas_write(out.clone());
    assert!(np.saveas.is_none(), "the save closed Save As");
    let want: Vec<u8> = [0xFE, 0xFF, 0, b'a', 0, b'\r', 0, b'\n', 0, b'b'].to_vec();
    assert_eq!(std::fs::read(&out).expect("read"), want);
    assert_eq!(
        std::fs::read(&path).expect("read"),
        b"a\r\nb",
        "the old file stays"
    );

    np.edit = MultiEdit::new("c");
    assert!(np.save());
    assert_eq!(
        std::fs::read(&out).expect("read"),
        [0xFE, 0xFF, 0, b'c'],
        "Save keeps UTF-16 BE"
    );
    np.saveas_dialog();
    assert_eq!(
        encoding(&np),
        Some(3),
        "Save As starts on the document's encoding"
    );
}

/// Yes on the save prompt with a path that cannot be written raises the error box and forgets
/// the action, so Exit does not sit armed.
#[test]
fn failed_save_on_the_prompt_raises_the_box_and_drops_the_action() {
    let mut np = Notepad::new(None);
    np.path = Some(std::env::temp_dir().join("reprise-no-such-dir/x.txt"));
    np.dirty = true;
    np.confirm_then(PendingAction::Quit);
    let reply = np.answer_box(Some(0));
    assert!(!reply.quit);
    assert!(!np.pending.is_waiting());
    assert!(np.dirty);
    assert!(
        np.asks
            .up()
            .is_some_and(|(b, _)| b.fields.text.starts_with("Cannot create the "))
    );
    assert_eq!(
        np.children().last().expect("the box").id,
        np.asks.win().expect("the box")
    );
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
    let win = box_win(&np);
    assert!(matches!(np.asks.up(), Some((_, Ask::Lossy(_)))));
    let enter = KeyInput {
        sym: keysym::RETURN,
        utf8: None,
        mods: repriseapp::Mods::default(),
        edit: None,
    };
    let mut ctx = test_ctx();
    np.event(win, Event::Key(enter), &mut ctx);
    assert!(np.asks.is_empty(), "Yes closed the prompt");
    assert_eq!(np.edit.text(), "caf\u{fffd}");
    assert_eq!(np.path, None);
    assert_eq!(std::fs::read(&latin1).expect("read"), b"caf\xe9");

    let missing = dir.join("missing.txt");
    let np = Notepad::new(Some(missing.clone()));
    assert_eq!(np.path, Some(missing));
}

/// A paste is answered whenever the clipboard's owner writes it, so it can land while the box
/// over the main window is up.
#[test]
fn a_paste_landing_under_the_open_dialog_asks_to_save_before_the_pick_replaces_it() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path();
    let picked = dir.join("picked.txt");
    std::fs::write(&picked, "picked").expect("write");
    let mut np = Notepad::new(None);
    np.open_dialog();
    np.host.folder = Some(np.host.folders.id(dir));
    let mut ctx = test_ctx();
    np.event(MAIN, Event::Pasted("pasted".into()), &mut ctx);

    np.open_accept("picked.txt", &mut test_icons());
    assert_eq!(np.edit.text(), "pasted", "the pick leaves the pasted text");
    assert!(matches!(np.asks.up(), Some((_, Ask::SaveChanges))));

    np.savechanges_answered(Some(1));
    assert_eq!(np.edit.text(), "picked", "Don't Save opens the pick");
    assert_eq!(np.path.as_deref(), Some(picked.as_path()));
}

#[test]
fn a_paste_landing_under_the_untitled_copy_question_asks_to_save_before_yes_replaces_it() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path();
    std::fs::write(dir.join("latin1.txt"), b"caf\xe9").expect("write");
    let mut np = Notepad::new(None);
    np.open_dialog();
    np.host.folder = Some(np.host.folders.id(dir));
    np.open_accept("latin1.txt", &mut test_icons());
    let win = box_win(&np);
    assert!(matches!(np.asks.up(), Some((_, Ask::Lossy(_)))));
    let mut ctx = test_ctx();
    np.event(MAIN, Event::Pasted("pasted".into()), &mut ctx);

    let enter = KeyInput {
        sym: keysym::RETURN,
        utf8: None,
        mods: repriseapp::Mods::default(),
        edit: None,
    };
    np.event(win, Event::Key(enter), &mut ctx);
    assert_eq!(np.edit.text(), "pasted", "Yes leaves the pasted text");
    assert!(matches!(np.asks.up(), Some((_, Ask::SaveChanges))));
    assert!(np.open.is_none(), "Open is not left up beside the prompt");

    np.savechanges_answered(Some(1));
    assert_eq!(np.edit.text(), "caf\u{fffd}", "Don't Save shows the copy");
    assert_eq!(np.path, None);
}

#[test]
fn a_device_or_folder_on_the_command_line_raises_the_box() {
    for p in ["/dev/zero", "/"] {
        let np = Notepad::new(Some(PathBuf::from(p)));
        assert_eq!(np.path, None);
        assert_eq!(np.edit.text(), "");
        let text = &np.asks.up().expect("the box").0.fields.text;
        assert_eq!(text, &format!("Cannot open {p}.\nIt is not a file."));
    }
}

#[test]
fn a_draw_failure_raises_the_box_with_the_systems_error() {
    let mut np = Notepad::new(None);
    np.draw_failed("Too many open files");
    assert_eq!(
        np.asks.up().expect("the box").0.fields.text,
        "Notepad cannot draw its window: Too many open files.\nClose some programs and try again."
    );
}

/// Types `name` into the dialog `win`'s File name field and presses Enter.
fn accept_typed(np: &mut Notepad, win: WinId, name: &str, ctx: &mut Ctx) {
    for ch in name.chars() {
        let utf8 = ch.to_string();
        let k = KeyInput {
            sym: ch as u32,
            utf8: Some(utf8.clone()),
            mods: repriseapp::Mods::default(),
            edit: reprisekit::textedit::key_for(ch as u32, false, false, Some(&utf8)),
        };
        np.event(win, Event::Key(k), ctx);
    }
    np.event(win, key(keysym::RETURN), ctx);
}

/// A name the Open dialog cannot read raises the box over the dialog, which stays up with the
/// document untouched and has the keyboard back once the box closes.
#[test]
fn a_failed_open_from_the_dialog_raises_the_box_over_it() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let missing = tmp.path().join("missing.txt");
    let mut np = Notepad::new(None);
    let mut ctx = test_ctx();
    np.open_dialog();
    accept_typed(&mut np, OPEN, &missing.display().to_string(), &mut ctx);
    assert!(np.open.is_some(), "the dialog stays up");
    assert_eq!(np.path, None);
    assert_eq!(
        np.asks.up().expect("the box").0.fields.text,
        format!(
            "{}\nFile not found.\nPlease verify the correct file name was given.",
            missing.display()
        )
    );
    let win = box_win(&np);
    assert_eq!(np.parent(win), Some(OPEN), "the box covers Open");
    np.event(win, key(keysym::RETURN), &mut ctx);
    assert!(np.asks.is_empty());
    assert!(np.open.is_some(), "OK goes back to Open");
}

/// A Save As write that fails raises its box over Save As, also when it follows Yes on the
/// overwrite question.
#[test]
fn a_failed_save_from_save_as_raises_the_box_over_it() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut np = Notepad::new(None);
    let mut ctx = test_ctx();
    np.edit = MultiEdit::new("new");
    np.saveas_dialog();
    // Past NAME_MAX, so the write fails as root too.
    let long = tmp.path().join(format!("{}.txt", "n".repeat(300)));
    accept_typed(&mut np, SAVEAS, &long.display().to_string(), &mut ctx);
    assert!(np.saveas.is_some(), "Save As stays up");
    assert!(matches!(np.asks.up(), Some((_, Ask::Error))));
    let win = box_win(&np);
    assert_eq!(np.parent(win), Some(SAVEAS), "a write that failed");
    np.event(win, key(keysym::RETURN), &mut ctx);
    assert!(np.asks.is_empty());

    let sub = tmp.path().join("sub");
    std::fs::create_dir(&sub).expect("mkdir");
    let taken = sub.join("taken.txt");
    std::fs::write(&taken, "old").expect("write");
    np.saveas_dialog();
    accept_typed(&mut np, SAVEAS, &taken.display().to_string(), &mut ctx);
    assert_eq!(overwriting(&np), Some(taken.as_path()));
    std::fs::remove_dir_all(&sub).expect("the folder goes, so Yes cannot write");
    let win = box_win(&np);
    np.event(win, key(keysym::TAB), &mut ctx);
    np.event(win, key(keysym::RETURN), &mut ctx);
    assert!(
        matches!(np.asks.up(), Some((_, Ask::Error))),
        "Tab, Enter: Yes failed"
    );
    assert_eq!(np.parent(box_win(&np)), Some(SAVEAS), "Yes that failed");
    assert!(np.saveas.is_some());
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
    assert_eq!(overwriting(&np), Some(taken.as_path()));
    assert_eq!(
        std::fs::read_to_string(&taken).unwrap(),
        "old",
        "nothing written yet"
    );
    assert!(np.saveas.is_some(), "Save As stays open behind the prompt");
    box_win(&np);
    let f = &np.asks.up().unwrap().0.fields;
    assert_eq!(f.buttons, ["&Yes", "&No"]);
    assert_eq!(f.default, 1, "defaults to No");

    np.answer_box(Some(1));
    assert!(np.asks.is_empty());
    assert_eq!(std::fs::read_to_string(&taken).unwrap(), "old");
    assert!(np.saveas.is_some());

    np.saveas_outcome(
        reprisekit::controls::filedlg::Event::Accept(vec![name]),
        &mut test_icons(),
    );
    np.answer_box(Some(0));
    assert_eq!(std::fs::read_to_string(&taken).unwrap(), "new");
    assert_eq!(np.path.as_deref(), Some(taken.as_path()));
    assert!(np.asks.is_empty());
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
    let win = box_win(&np);
    np.event(win, key(keysym::LEFT, left()), &mut ctx);
    assert_eq!(focus(&np), 0, "Left moves from No to Yes");
    np.event(win, key(keysym::LEFT, left()), &mut ctx);
    assert_eq!(focus(&np), 0, "Left stops at Yes");
    np.event(win, key(keysym::RETURN, None), &mut ctx);
    assert!(np.asks.is_empty(), "Enter closed the prompt");
    assert_eq!(
        std::fs::read_to_string(&taken).unwrap(),
        "new",
        "Enter on Yes wrote over the file"
    );
    let _ = std::fs::remove_file(&taken);
}

#[test]
fn y_and_n_answer_the_replace_prompt_bare_or_with_alt() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let taken = tmp.path().join("taken.txt");
    let mut ctx = test_ctx();
    let letter = |ch: char, alt| {
        let utf8 = ch.to_string();
        Event::Key(KeyInput {
            sym: ch as u32,
            utf8: Some(utf8.clone()),
            mods: repriseapp::Mods {
                alt,
                ..Default::default()
            },
            edit: reprisekit::textedit::key_for(ch as u32, false, false, Some(&utf8)),
        })
    };
    for (ch, alt, replaced) in [
        ('n', false, false),
        ('y', false, true),
        ('y', true, true),
        ('n', true, false),
    ] {
        std::fs::write(&taken, "old").expect("write");
        let mut np = Notepad::new(None);
        np.edit = MultiEdit::new("new");
        np.saveas = Some(sample_filedlg());
        np.host.folder = Some(np.host.folders.id(tmp.path()));
        np.saveas_accept("taken.txt", &mut test_icons());
        assert_eq!(overwriting(&np), Some(taken.as_path()));
        assert_eq!(focus(&np), 1, "No has the focus");
        np.event(box_win(&np), letter(ch, alt), &mut ctx);
        assert!(np.asks.is_empty(), "{ch} (Alt {alt}) answers the prompt");
        assert_eq!(
            std::fs::read_to_string(&taken).unwrap(),
            if replaced { "new" } else { "old" },
            "{ch} (Alt {alt})"
        );
        assert_eq!(np.saveas.is_some(), !replaced, "{ch} (Alt {alt}): Save As");
    }
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
    assert_eq!(overwriting(&np), Some(link.as_path()));
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
    assert!(
        matches!(np.asks.up(), Some((_, Ask::SaveChanges))),
        "the save prompt opened first"
    );
    let reply = np.answer_box(Some(0));
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
    assert!(np.asks.is_empty(), "no box");
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
fn ctrl_f_and_ctrl_h_bring_their_open_window_forward_with_its_text() {
    let mut np = Notepad::new(None);
    let mut ctx = test_ctx();
    np.edit = MultiEdit::new("text");
    np.sync_menus();
    let ctrl = |sym: u32| {
        Event::Key(KeyInput {
            sym,
            utf8: None,
            mods: repriseapp::Mods {
                ctrl: true,
                ..Default::default()
            },
            edit: None,
        })
    };
    let typing = |s: &str| {
        Event::Key(KeyInput {
            sym: 0,
            utf8: Some(s.into()),
            mods: repriseapp::Mods::default(),
            edit: Some(EditKey::Insert(s.into())),
        })
    };
    assert_eq!(np.event(MAIN, ctrl(0x66), &mut ctx).raise, None);
    np.event(FIND, typing("typed"), &mut ctx);
    let r = np.event(MAIN, ctrl(0x66), &mut ctx);
    assert_eq!(r.raise, Some(FIND), "Ctrl+F raises the open Find");
    assert_eq!(
        Notepad::field(np.find.as_ref().unwrap(), finddlg::QUERY),
        "typed"
    );

    assert_eq!(np.event(MAIN, ctrl(0x68), &mut ctx).raise, None);
    assert!(np.find.is_none(), "Replace opens in Find's place");
    np.event(REPLACE_WIN, typing("typed"), &mut ctx);
    let r = np.event(MAIN, ctrl(0x68), &mut ctx);
    assert_eq!(r.raise, Some(REPLACE_WIN), "Ctrl+H raises the open Replace");
    assert_eq!(
        Notepad::field(np.replace.as_ref().unwrap(), finddlg::QUERY),
        "typed"
    );
}

#[test]
fn find_and_replace_reopen_with_the_last_query_selected_so_typing_replaces_it() {
    use reprisekit::textedit::keysym;
    let mut np = Notepad::new(None);
    let mut ctx = test_ctx();
    np.edit = MultiEdit::new("zebra fox");
    np.sync_menus();
    let key = |sym: u32, ctrl: bool, edit: Option<EditKey>| {
        Event::Key(KeyInput {
            sym,
            utf8: None,
            mods: repriseapp::Mods {
                ctrl,
                ..Default::default()
            },
            edit,
        })
    };
    let typing = |s: &str| key(0, false, Some(EditKey::Insert(s.into())));
    let query = |np: &Notepad, win: WinId| {
        let open = if win == FIND { &np.find } else { &np.replace };
        Notepad::field(open.as_ref().expect("the dialog is open"), finddlg::QUERY)
    };

    for (ctrl_sym, win) in [(0x66, FIND), (0x68, REPLACE_WIN)] {
        np.event(MAIN, key(ctrl_sym, true, None), &mut ctx);
        np.event(win, typing("zebra"), &mut ctx);
        np.event(win, key(keysym::RETURN, false, None), &mut ctx);
        np.event(win, key(keysym::ESCAPE, false, None), &mut ctx);
        assert!(np.find.is_none() && np.replace.is_none(), "Esc closes it");

        np.event(MAIN, key(ctrl_sym, true, None), &mut ctx);
        assert_eq!(query(&np, win), "zebra", "it reopens with the last query");
        np.event(win, typing("fox"), &mut ctx);
        assert_eq!(query(&np, win), "fox", "typing replaces the selected query");

        let r = np.event(MAIN, key(ctrl_sym, true, None), &mut ctx);
        assert_eq!(r.raise, Some(win));
        np.event(win, typing("owl"), &mut ctx);
        assert_eq!(
            query(&np, win),
            "owl",
            "bringing the open dialog forward selects its query too"
        );
        np.event(win, key(keysym::ESCAPE, false, None), &mut ctx);
    }
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

#[test]
fn write_recovery_round_trips_into_a_private_dir() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path().join("reprise").join("notepad");
    let crlf = Format {
        line_end: document::LineEnd::Crlf,
        ..Format::default()
    };
    let kept = write_recovery(&dir, 42, "notes", &["one", "two"], crlf).expect("write");
    assert_eq!(kept, dir.join("recovery-42-notes.txt"));
    assert_eq!(recovered_in(&dir), std::slice::from_ref(&kept));
    assert_eq!(read_text(&kept).expect("read").0, "one\r\ntwo");
    let mode = std::fs::metadata(&dir).expect("dir").permissions().mode();
    assert_eq!(mode & 0o777, 0o700);
}

#[test]
fn a_rescued_document_is_offered_at_the_next_start_and_its_file_goes_once_saved() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path().join("recovery");
    let mut np = Notepad::new(None);
    np.rescue_dir = Some(dir.clone());
    np.edit = MultiEdit::new("unsaved");
    np.dirty = true;
    np.rescue();
    let kept = recovered_in(&dir);
    assert_eq!(kept.len(), 1);

    let mut next = Notepad::new(None);
    next.rescue_dir = Some(dir.clone());
    next.offer_recovered();
    let win = box_win(&next);
    let mut ctx = test_ctx();
    next.event(win, typed(keysym::RETURN, None, false), &mut ctx);
    assert!(next.asks.is_empty(), "Yes closed the prompt");
    assert_eq!(next.edit.text(), "unsaved");
    assert_eq!(next.path, None);
    assert!(next.dirty);
    assert!(kept[0].exists(), "kept until the text is saved");

    next.saveas_write(tmp.path().join("saved.txt"));
    assert!(
        !kept[0].exists(),
        "saving elsewhere deletes the recovery file"
    );
    assert!(recovered_in(&dir).is_empty());
}

#[test]
fn declining_a_recovered_document_sends_it_to_the_recycle_bin_and_offers_the_next() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path().join("recovery");
    let trash = tmp.path().join("Trash");
    reprisefs::trash::set_home_trash(Some(trash.clone()));
    reprisefs::trash::set_mount_table(Some("/dev/null".into()));
    reprisefs::configfile::set_path(Some(tmp.path().join("config.toml")));
    let first = write_recovery(&dir, 1, "a", &["a"], Format::default()).expect("write");
    let second = write_recovery(&dir, 2, "b", &["b"], Format::default()).expect("write");
    let mut np = Notepad::new(None);
    np.rescue_dir = Some(dir.clone());
    np.offer_recovered();
    let mut ctx = test_ctx();
    np.asks.up_mut().expect("the prompt").fields.default = 1;
    np.event(
        np.asks.win().unwrap(),
        typed(keysym::RETURN, None, false),
        &mut ctx,
    );
    reprisefs::trash::set_home_trash(None);
    reprisefs::trash::set_mount_table(None);
    reprisefs::configfile::set_path(None);
    assert!(
        !first.exists(),
        "No takes the file out of the recovery folder"
    );
    assert_eq!(
        std::fs::read_to_string(trash.join("files/recovery-1-a.txt")).ok(),
        Some("a".into()),
        "the declined text waits in the Recycle Bin"
    );
    assert_eq!(
        recovered_file(&np),
        Some(&second),
        "the next file is offered"
    );
    assert_eq!(np.edit.text(), "");
}

#[test]
fn each_recovery_prompt_names_its_document_and_counts_them() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path().to_path_buf();
    let untitled = tr!("untitled");
    write_recovery(&dir, 1, "notes", &["milk"], Format::default()).expect("write");
    write_recovery(
        &dir,
        2,
        &untitled,
        &[
            "",
            "  Dear Sam, thanks for the books you lent me last week  ",
            "x",
        ],
        Format::default(),
    )
    .expect("write");
    write_recovery(&dir, 3, &untitled, &["", " "], Format::default()).expect("write");
    let mut np = Notepad::new(None);
    np.rescue_dir = Some(dir.clone());
    np.offer_recovered();
    let texts: Vec<String> = (np.asks.iter())
        .filter(|(_, ask)| matches!(ask, Ask::Recovered(_)))
        .map(|(b, _)| b.fields.text.clone())
        .collect();
    assert_eq!(
        texts,
        [
            "The unsaved text of notes was recovered (1 of 3).\nDo you want to open it?",
            "The unsaved text of Untitled (\"Dear Sam, thanks for the books you lent...\") \
             was recovered (2 of 3).\nDo you want to open it?",
            "The unsaved text of Untitled was recovered (3 of 3).\nDo you want to open it?",
        ]
    );

    let lone = tempfile::tempdir().expect("tempdir");
    write_recovery(lone.path(), 4, "notes", &["milk"], Format::default()).expect("write");
    let mut np = Notepad::new(None);
    np.rescue_dir = Some(lone.path().to_path_buf());
    np.offer_recovered();
    assert_eq!(
        np.asks.up().map(|(b, _)| b.fields.text.as_str()),
        Some("The unsaved text of notes was recovered.\nDo you want to open it?")
    );
}

#[test]
fn a_save_over_a_file_changed_on_disk_asks_first() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("notes.txt");
    std::fs::write(&path, "mine").expect("write");
    let mut np = Notepad::new(Some(path.clone()));
    std::fs::write(&path, "theirs").expect("write");
    let f = std::fs::File::options()
        .write(true)
        .open(&path)
        .expect("open");
    f.set_modified(std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1))
        .expect("set mtime");
    np.edit = MultiEdit::new("edited");
    np.dirty = true;

    assert!(!np.save());
    box_win(&np);
    assert!(matches!(np.asks.up(), Some((_, Ask::Changed))));
    assert_eq!(std::fs::read_to_string(&path).expect("read"), "theirs");
    assert!(np.dirty);

    let mut ctx = test_ctx();
    np.asks.up_mut().expect("the prompt").fields.default = 0;
    np.event(
        np.asks.win().unwrap(),
        typed(keysym::RETURN, None, false),
        &mut ctx,
    );
    assert!(np.asks.is_empty(), "Yes closed the prompt");
    assert_eq!(std::fs::read_to_string(&path).expect("read"), "edited");
    assert!(!np.dirty);
    assert!(np.save(), "the save took the file's new stamp");
}

#[test]
fn the_german_strings_translate_the_menus_and_fall_back_to_english() {
    use reprisekit::i18n::Catalog;
    let de = include_str!("../../../../data/locale/de/notepad.ftl");
    let en = include_str!("../locale/en.ftl");
    let c = Catalog::from_texts(&[("de".parse().unwrap(), de), ("en".parse().unwrap(), en)]);
    assert_eq!(c.tr_args("menu-file", &[]), "&Datei");
    assert_eq!(
        c.tr_args("window-title", &[("name", "brief.txt".into())]),
        "brief.txt - Editor"
    );
    assert_eq!(
        c.tr_args("menu-word-wrap", &[]),
        "&Word Wrap",
        "untranslated: English"
    );
}

#[test]
fn word_wrap_folds_long_lines_into_the_box_and_off_draws_as_before() {
    let ctx = test_ctx();
    let mut np = Notepad::new(None);
    np.edit = MultiEdit::new(&"word ".repeat(200));
    let mut before = Canvas::new(ctx.size.0 as u32, ctx.size.1 as u32);
    np.draw_main(&mut before, &ctx);
    assert!(np.edit_layout(&ctx).hscroll.is_some());
    let checked = |np: &Notepad| match &np.bar.menus[title::FORMAT][row::WORD_WRAP] {
        MenuItem::Entry { checked, .. } => *checked,
        MenuItem::Separator => unreachable!(),
    };
    np.command(title::FORMAT, row::WORD_WRAP, &ctx);
    assert!(checked(&np));
    let lay = np.edit_layout(&ctx);
    assert_eq!(lay.hscroll, None);
    assert!(lay.vscroll.is_some(), "the vertical bar stays");
    assert!(lay.total > 1, "{} rows", lay.total);
    np.command(title::FORMAT, row::WORD_WRAP, &ctx);
    assert!(!checked(&np));
    let mut after = Canvas::new(ctx.size.0 as u32, ctx.size.1 as u32);
    np.draw_main(&mut after, &ctx);
    assert!(before.pixels == after.pixels);
}

#[test]
fn only_the_window_with_the_keyboard_draws_and_blinks_its_caret() {
    let mut ctx = test_ctx();
    let mut np = Notepad::new(None);
    let draw = |np: &mut Notepad, ctx: &Ctx| {
        let mut c = Canvas::new(ctx.size.0 as u32, ctx.size.1 as u32);
        np.draw_main(&mut c, ctx);
        c
    };
    assert!(
        !np.event(MAIN, Event::Blink, &mut ctx).redraw,
        "a blink without the keyboard redraws nothing"
    );
    let unfocused = draw(&mut np, &ctx);
    np.event(MAIN, Event::Focus(true), &mut ctx);
    assert!(np.event(MAIN, Event::Blink, &mut ctx).redraw);
    assert_ne!(
        draw(&mut np, &ctx).pixels,
        unfocused.pixels,
        "the caret shows with the keyboard"
    );
    np.event(MAIN, Event::Focus(false), &mut ctx);
    assert_eq!(draw(&mut np, &ctx).pixels, unfocused.pixels);
}

/// The selection shows while the window or its Find window holds the keyboard, so a found
/// match stays lit, and hides once the keyboard leaves both.
#[test]
fn the_selection_shows_while_notepad_or_its_find_window_has_the_keyboard() {
    let mut ctx = test_ctx();
    let mut np = Notepad::new(None);
    np.edit = MultiEdit::new("one two");
    np.edit.place(multiedit::Pos { line: 0, col: 4 }, false);
    np.edit.place(multiedit::Pos { line: 0, col: 7 }, true);
    np.find = Some(finddlg::find_form(&finddlg::FindFields {
        query: "two".into(),
        match_case: false,
        down: true,
    }));
    let hl = ctx.theme.colors.hilight.argb();
    let lit = |np: &mut Notepad, ctx: &Ctx| {
        let mut c = Canvas::new(ctx.size.0 as u32, ctx.size.1 as u32);
        np.draw_main(&mut c, ctx);
        c.pixels.contains(&hl)
    };
    np.event(MAIN, Event::Focus(true), &mut ctx);
    assert!(lit(&mut np, &ctx), "with the keyboard");
    np.event(MAIN, Event::Focus(false), &mut ctx);
    assert!(!lit(&mut np, &ctx), "the keyboard elsewhere");
    let r = np.event(FIND, Event::Focus(true), &mut ctx);
    assert!(r.redraw_all, "Find taking the keyboard redraws the edit");
    assert!(lit(&mut np, &ctx), "Find has the keyboard");
    np.event(FIND, Event::Focus(false), &mut ctx);
    assert!(!lit(&mut np, &ctx), "Find lost it too");
    np.event(FIND, Event::Focus(true), &mut ctx);
    np.find = None;
    assert!(!lit(&mut np, &ctx), "Find closed");
}

#[test]
fn time_date_writes_the_time_then_the_locales_short_date() {
    let at = reprisefs::locale::mktime(2026, 11, 14, 21, 5, 0);
    let stamp = crate::window::local_stamp(at);
    let date = reprisefs::locale::short_date(&reprisefs::locale::localtime(at));
    let time = stamp.strip_suffix(&date).map(str::trim_end);
    let want = if reprisefs::locale::is_12h() {
        "9:05 PM"
    } else {
        "21:05"
    };
    assert_eq!(time, Some(want), "{stamp:?}");
}

/// The window of the box up, which is the last child.
fn box_win(np: &Notepad) -> WinId {
    let win = np.asks.win().expect("a box is up");
    assert_eq!(np.children().last().map(|c| c.id), Some(win));
    win
}

/// The button the box up has the focus on.
fn focus(np: &Notepad) -> usize {
    np.asks.up().expect("a box is up").0.fields.default
}

fn overwriting(np: &Notepad) -> Option<&Path> {
    match np.asks.up() {
        Some((_, Ask::Overwrite(p))) => Some(p),
        _ => None,
    }
}

fn recovered_file(np: &Notepad) -> Option<&PathBuf> {
    match np.asks.up() {
        Some((_, Ask::Recovered(f))) => Some(f),
        _ => None,
    }
}

impl Notepad {
    /// Answers the box up with `button`, `None` for Escape and the close box.
    pub(crate) fn answer_box(&mut self, button: Option<usize>) -> Reply {
        let ask = self.asks.close().expect("a box is up");
        self.answered(ask, button)
    }
}

#[test]
fn save_changes_with_an_error_raised_behind_it_keeps_the_pending_action_and_yes_runs_it() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("notes.txt");
    std::fs::write(&path, "old").expect("write");
    let mut np = Notepad::new(Some(path.clone()));
    np.edit = MultiEdit::new("new");
    np.dirty = true;
    np.confirm_then(PendingAction::Quit);
    np.confirm_then(PendingAction::Quit);
    assert_eq!(np.asks.len(), 1, "Save changes is asked once");
    np.draw_failed("Too many open files");
    assert!(
        matches!(np.asks.up(), Some((_, Ask::SaveChanges))),
        "the error waits behind Save changes"
    );
    let mut ctx = test_ctx();
    let win = box_win(&np);
    let reply = np.event(win, typed(keysym::RETURN, None, false), &mut ctx);
    assert!(reply.quit, "Yes saved and ran the pending quit");
    assert_eq!(std::fs::read_to_string(&path).expect("read"), "new");
    assert!(matches!(np.asks.up(), Some((_, Ask::Error))));
}

#[test]
fn two_recovered_files_ask_in_turn() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path().to_path_buf();
    write_recovery(&dir, 1, "a", &["a"], Format::default()).expect("write");
    write_recovery(&dir, 2, "b", &["b"], Format::default()).expect("write");
    let mut ctx = test_ctx();
    let mut np = Notepad::new(None);
    np.rescue_dir = Some(dir.clone());
    np.offer_recovered();
    assert_eq!(np.asks.len(), 2, "the second waits behind the first");
    let (first, win) = (recovered_file(&np).cloned().unwrap(), box_win(&np));
    np.asks.up_mut().unwrap().fields.default = 1;
    np.event(win, typed(keysym::RETURN, None, false), &mut ctx);
    assert!(!first.exists(), "No deleted the first");
    let second = recovered_file(&np).cloned().expect("the second is up");
    assert_ne!(box_win(&np), win, "in a fresh window");
    np.answer_box(None);
    assert!(np.asks.is_empty());
    assert!(second.exists(), "closing the box keeps the file");

    write_recovery(&dir, 3, "c", &["c"], Format::default()).expect("write");
    let mut np = Notepad::new(None);
    np.rescue_dir = Some(dir.clone());
    np.offer_recovered();
    np.answer_box(Some(0));
    assert!(np.asks.is_empty(), "Yes on one offers no more");
    assert_eq!(recovered_in(&dir).len(), 2, "and keeps both files");
}

#[test]
fn the_overwrite_question_is_withdrawn_when_save_as_closes() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let taken = tmp.path().join("taken.txt");
    std::fs::write(&taken, "old").expect("write");
    let mut np = Notepad::new(None);
    np.edit = MultiEdit::new("new");
    np.saveas = Some(sample_filedlg());
    np.host.folder = Some(np.host.folders.id(tmp.path()));
    np.saveas_accept("taken.txt", &mut test_icons());
    let win = box_win(&np);
    assert_eq!(np.parent(win), Some(SAVEAS), "the question covers Save As");
    np.saveas_outcome(FileEvent::Cancel, &mut test_icons());
    assert!(np.saveas.is_none());
    assert!(np.asks.is_empty(), "the question went with Save As");
    assert_eq!(std::fs::read_to_string(&taken).unwrap(), "old");
}

#[test]
fn an_icon_that_decodes_late_reaches_a_waiting_box() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path().to_path_buf();
    write_recovery(&dir, 1, "a", &["a"], Format::default()).expect("write");
    write_recovery(&dir, 2, "b", &["b"], Format::default()).expect("write");
    let mut np = Notepad::new(None);
    np.rescue_dir = Some(dir);
    np.offer_recovered();
    assert!(np.asks.iter().all(|(b, _)| b.fields.icon.is_none()));
    np.warn_icon = Some(Canvas::new(32, 32));
    np.prime(&mut test_ctx());
    assert!(
        np.asks.iter().all(|(b, _)| b.fields.icon.is_some()),
        "the box up and the one waiting both have the icon"
    );
}

/// Ctrl+V, Ctrl+S, a file name and Enter typed while the paste is out: once it lands, Save As
/// opens and saves under that name.
#[test]
fn a_name_typed_ahead_of_save_as_during_a_paste_saves_under_it() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("typed.txt");
    let mut np = Notepad::new(None);
    let mut ctx = test_ctx();
    let mut wait = repriseapp::PasteWait::default();
    let mut asked = Vec::new();
    let boxes = |np: &Notepad| -> Vec<WinId> {
        let kids = np.children().into_iter();
        kids.filter(|c| c.modal).map(|c| c.id).collect()
    };
    let gate = |np: &Notepad, win: WinId| boxes(np).last().copied().filter(|_| win == MAIN);
    let mut send = |np: &mut Notepad,
                    win: WinId,
                    ev: Event,
                    asked: &mut Vec<_>,
                    w: &mut repriseapp::PasteWait| {
        let key = matches!(ev, Event::Key(_) | Event::KeyUp(_));
        if key && gate(np, win).is_some() {
            return;
        }
        if np.event(win, ev, &mut ctx).paste {
            asked.push(w.begin(win));
        }
    };
    let typed = |ch: char| KeyInput {
        sym: ch as u32,
        utf8: Some(ch.to_string()),
        mods: repriseapp::Mods::default(),
        edit: Some(EditKey::Insert(ch.to_string())),
    };
    let ctrl = |ch: char| KeyInput {
        sym: ch as u32,
        utf8: None,
        mods: repriseapp::Mods {
            ctrl: true,
            ..Default::default()
        },
        edit: (ch == 'v').then_some(EditKey::Paste),
    };
    repriseapp::fake_clipboard_text(true);
    let mut keys = vec![ctrl('v'), ctrl('s')];
    keys.extend(path.to_str().expect("utf-8 path").chars().map(typed));
    keys.push(KeyInput {
        sym: keysym::RETURN,
        utf8: None,
        mods: repriseapp::Mods::default(),
        edit: None,
    });
    for k in keys {
        if let Some(ev) = wait.hold(MAIN, Event::Key(k)) {
            send(&mut np, MAIN, ev, &mut asked, &mut wait);
        }
    }
    repriseapp::fake_clipboard_text(false);
    assert_eq!(asked.len(), 1, "Ctrl+V asked for the clipboard");
    wait.land(asked[0], Some(Event::Pasted("pasted".into())));
    while let Some((win, held)) = wait.pop() {
        let repriseapp::Held::Event(ev) = held else {
            continue;
        };
        let before = boxes(&np);
        let to = wait.aim(win, &ev, gate(&np, win));
        send(&mut np, to, ev, &mut asked, &mut wait);
        wait.opened(&before, &boxes(&np));
    }
    assert!(np.saveas.is_none(), "Enter saved and closed Save As");
    assert_eq!(std::fs::read_to_string(&path).expect("saved"), "pasted");
}

#[test]
fn opened_and_saved_files_go_to_the_recent_files_as_notepad_s() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path();
    let xbel = dir.join("recently-used.xbel");
    let (opened, saved_as) = (dir.join("opened.txt"), dir.join("saved.txt"));
    std::fs::write(&opened, "loaded").expect("write");
    let mut np = Notepad::new(None);
    np.recent = Recent::at(DESKTOP, Some(xbel.clone()));
    np.open = Some(sample_filedlg());
    let id = np.host.folders.id(dir);
    np.host.folder = Some(id);
    np.open_outcome(
        FileEvent::Accept(vec!["opened.txt".into()]),
        &mut test_icons(),
    );
    np.recent.settle();
    let mut recent = reprisefs::recent::RecentFiles::at(Some(xbel.clone()));
    assert_eq!(recent.current(10), std::slice::from_ref(&opened));

    std::fs::remove_file(&xbel).expect("remove");
    np.edit = MultiEdit::new("edited");
    np.dirty = true;
    assert!(np.save());
    np.saveas = Some(sample_filedlg());
    np.host.folder = Some(id);
    np.saveas_outcome(
        FileEvent::Accept(vec!["saved.txt".into()]),
        &mut test_icons(),
    );
    np.recent.settle();
    let marks = recent.bookmarks().to_vec();
    let mut paths: Vec<_> = marks.iter().map(|b| b.path.clone()).collect();
    paths.sort();
    assert_eq!(paths, [opened, saved_as], "Save and Save As record");
    for b in &marks {
        assert!(
            (b.apps.iter()).any(|(_, exec)| exec.starts_with("'reprise-notepad ")),
            "recorded as Notepad's: {b:?}"
        );
    }
}

#[test]
fn an_alt_letter_no_menu_title_owns_types_nothing() {
    let mut ctx = test_ctx();
    let mut np = Notepad::new(None);
    let letter = |ch: char, alt| {
        let utf8 = ch.to_string();
        Event::Key(KeyInput {
            sym: ch as u32,
            utf8: Some(utf8.clone()),
            mods: repriseapp::Mods {
                alt,
                ..Default::default()
            },
            edit: reprisekit::textedit::key_for(ch as u32, false, false, Some(&utf8)),
        })
    };
    np.event(MAIN, letter('q', true), &mut ctx);
    assert_eq!(np.edit.text(), "", "Alt+Q types nothing");
    assert!(!np.dirty);
    np.event(MAIN, letter('q', false), &mut ctx);
    assert_eq!(np.edit.text(), "q");
}

#[test]
fn a_window_reopens_with_its_files_absolute_path_and_an_untitled_one_with_none() {
    assert_eq!(Notepad::new(None).reopen(MAIN), None);
    let mut np = Notepad::new(None);
    np.path = Some(PathBuf::from("/d/a b.txt"));
    assert_eq!(np.reopen(MAIN).as_deref(), Some("/d/a b.txt"));
    np.path = Some(PathBuf::from("notes.txt"));
    let cwd = std::env::current_dir().unwrap();
    assert_eq!(
        np.reopen(MAIN),
        cwd.join("notes.txt").to_str().map(String::from)
    );
    assert_eq!(np.reopen(ABOUT), None, "only the main window");
}
