use super::*;
use repriseapp::{MAIN, Mods};

fn test_ctx() -> Ctx {
    Ctx {
        theme: reprisekit::theme::Theme::default(),
        text: reprisekit::text::TextRenderer::load(&reprisekit::theme::FontConfig::default())
            .into(),
        icons: reprisekit::icontheme::IconCache::new(reprisekit::icontheme::IconTheme::load(
            "test",
            &[],
        )),
        caret_on: true,
        size: (400, 300),
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

fn key(sym: u32, ch: Option<char>, ctrl: bool) -> Event {
    let utf8 = ch.map(String::from);
    Event::Key(KeyInput {
        sym,
        edit: reprisekit::textedit::key_for(sym, ctrl, false, utf8.as_deref()),
        utf8,
        mods: Mods {
            ctrl,
            ..Default::default()
        },
    })
}

fn typed(app: &mut Calculator, ctx: &mut Ctx, keys: &str) {
    for ch in keys.chars() {
        let sym = match ch {
            '\n' => keysym::RETURN,
            c => c as u32,
        };
        app.event(MAIN, key(sym, Some(ch).filter(|c| *c != '\n'), false), ctx);
    }
}

#[test]
fn typed_keys_work_the_sum_and_ctrl_c_copies_it() {
    let (mut app, mut ctx) = (Calculator::new(), test_ctx());
    typed(&mut app, &mut ctx, "12*3\n");
    assert_eq!(app.calc.display(), "36");
    let reply = app.event(MAIN, key(0x63, Some('c'), true), &mut ctx);
    assert_eq!(reply.copy.as_deref(), Some("36"));
    let reply = app.event(MAIN, key(0x76, Some('v'), true), &mut ctx);
    assert!(reply.paste, "Ctrl+V asks for the clipboard");
    app.event(MAIN, Event::Pasted("2.5".into()), &mut ctx);
    assert_eq!(app.calc.display(), "2.5");
}

#[test]
fn the_view_menu_switches_mode_and_history_and_resizes() {
    let (mut app, mut ctx) = (Calculator::new(), test_ctx());
    app.event(MAIN, Event::Resized, &mut ctx);
    let standard = app.asked;
    let reply = app.command(title::VIEW, row::SCIENTIFIC);
    assert!(reply.redraw);
    let reply = app.event(MAIN, Event::Resized, &mut ctx);
    assert!(reply.size.is_some_and(|(w, _)| w > standard.0), "{reply:?}");
    assert!(app.form.item("sin").is_some(), "the Scientific keys show");
    typed(&mut app, &mut ctx, "2+3*4\n");
    assert_eq!(app.calc.display(), "14");
    app.event(MAIN, key(0x68, Some('h'), true), &mut ctx);
    assert!(app.history && app.form.item(calculator::HISTORY_LIST).is_some());
    let MenuItem::Entry { checked, .. } = &app.bar.menus[title::VIEW][row::SCIENTIFIC] else {
        unreachable!()
    };
    assert!(checked);
}

#[test]
fn a_pad_button_presses_its_input() {
    let (mut app, mut ctx) = (Calculator::new(), test_ctx());
    app.event(MAIN, Event::Resized, &mut ctx);
    for id in ["7", "mul", "6", "eq"] {
        app.press(id);
    }
    assert_eq!(app.calc.display(), "42");
    app.press("ms");
    assert_eq!(app.calc.memory, Some(42.0));
}
