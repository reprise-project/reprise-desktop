use super::*;
use repriseapp::{MAIN, Mods};

fn test_ctx() -> Ctx {
    Ctx {
        theme: reprisekit::theme::Theme::default(),
        text: TextRenderer::load(&FontConfig::default()).into(),
        icons: reprisekit::icontheme::IconCache::new(reprisekit::icontheme::IconTheme::load(
            "test",
            &[],
        )),
        caret_on: true,
        size: charmap::CLIENT,
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

fn window() -> CharMapWindow {
    let mut w = CharMapWindow::new(vec!["Sans".into()], 0);
    w.map = CharMap::new('!'..='~');
    charmap::follow(&mut w.form, &w.map);
    w
}

fn key(sym: u32, ctrl: bool) -> Event {
    let utf8 = char::from_u32(sym)
        .filter(|c| c.is_ascii_graphic())
        .map(String::from);
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

fn press(at: (i32, i32), time: u32) -> Event {
    Event::Pointer(PointerInput::Press {
        x: at.0,
        y: at.1,
        button: BTN_LEFT,
        time,
        mods: Mods::default(),
    })
}

#[test]
fn arrows_move_the_pick_and_enter_adds_it_to_copy() {
    let (mut w, mut ctx) = (window(), test_ctx());
    assert_eq!(w.map.picked(), Some('!'));
    w.event(MAIN, key(keysym::RIGHT, false), &mut ctx);
    w.event(MAIN, key(keysym::DOWN, false), &mut ctx);
    let want = char::from(b'!' + 1 + charmap::COLS as u8);
    assert_eq!(w.map.picked(), Some(want));
    w.event(MAIN, key(keysym::RETURN, false), &mut ctx);
    assert_eq!(w.form.edit_text(charmap::TO_COPY), want.to_string());
    let reply = w.event(MAIN, key(0x63, true), &mut ctx);
    assert_eq!(
        reply.copy,
        Some(want.to_string()),
        "Ctrl+C on the grid copies the pick"
    );
}

#[test]
fn a_double_click_appends_and_copy_puts_the_field_on_the_clipboard() {
    let (mut w, mut ctx) = (window(), test_ctx());
    let lay = w.grid(&ctx).expect("the grid");
    let r = lay.cell_rect(&w.map, 2).expect("in view");
    let at = (r.x + 3, r.y + 3);
    w.event(MAIN, press(at, 1000), &mut ctx);
    assert_eq!(w.map.picked(), Some('#'));
    assert_eq!(
        w.form.edit_text(charmap::TO_COPY),
        "",
        "one click only picks"
    );
    w.event(MAIN, press(at, 1100), &mut ctx);
    assert_eq!(w.form.edit_text(charmap::TO_COPY), "#");
    w.form_event(form::Event::Pressed(charmap::SELECT));
    assert_eq!(w.form.edit_text(charmap::TO_COPY), "##");
    let reply = w.form_event(form::Event::Pressed(charmap::COPY));
    assert_eq!(reply.copy.as_deref(), Some("##"));
}

#[test]
fn tab_leaves_the_grid_for_the_form_and_comes_back() {
    let (mut w, mut ctx) = (window(), test_ctx());
    assert!(w.grid_focus);
    w.event(MAIN, key(keysym::TAB, false), &mut ctx);
    assert!(!w.grid_focus && w.form.focus().is_some());
    for _ in 0..w.form.items.len() {
        if w.grid_focus {
            break;
        }
        w.event(MAIN, key(keysym::TAB, false), &mut ctx);
    }
    assert!(w.grid_focus, "Tab past the last control reaches the grid");
}

#[test]
fn a_search_narrows_the_grid() {
    let mut w = window();
    if let Some(form::Control::Edit { edit, .. }) =
        w.form.item_mut(charmap::SEARCH).map(|i| &mut i.control)
    {
        edit.key(EditKey::Insert("digit".into()));
    }
    w.form_event(form::Event::Changed(charmap::SEARCH));
    assert_eq!(
        w.map.shown,
        ['0', '1', '2', '3', '4', '5', '6', '7', '8', '9']
    );
    assert_eq!(w.map.picked(), Some('0'));
}
