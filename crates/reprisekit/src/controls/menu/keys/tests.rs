use super::super::tests::{nested_ctx, nested_tree, wide_theme};
use super::*;
use crate::controls::menu::{MenuAction, MenuTree, PopupMenu};
use crate::text::TextRenderer;
use crate::theme::FontConfig;

#[test]
fn keysyms_read_as_menu_keys_with_their_keypad_twins() {
    let read = |sym| MenuKey::from_keysym(sym, None);
    for (sym, key) in [
        (keysym::UP, MenuKey::Up),
        (keysym::KP_UP, MenuKey::Up),
        (keysym::KP_DOWN, MenuKey::Down),
        (keysym::KP_LEFT, MenuKey::Left),
        (keysym::KP_RIGHT, MenuKey::Right),
        (keysym::HOME, MenuKey::Home),
        (keysym::KP_HOME, MenuKey::Home),
        (keysym::END, MenuKey::End),
        (keysym::KP_END, MenuKey::End),
        (keysym::RETURN, MenuKey::Enter),
        (keysym::KP_ENTER, MenuKey::Enter),
        (keysym::SPACE, MenuKey::Enter),
        (keysym::ESCAPE, MenuKey::Escape),
    ] {
        assert_eq!(read(sym), Some(key), "keysym {sym:#x}");
    }
    assert_eq!(
        MenuKey::from_keysym(0x6f, Some("o")),
        Some(MenuKey::Char('o'))
    );
    assert_eq!(MenuKey::from_keysym(0x2c, Some(",")), None);
    assert_eq!(read(keysym::TAB), None);
}

#[test]
fn enter_runs_the_default_row_when_nothing_is_highlighted() {
    let rows = |enabled: bool| {
        vec![
            MenuItem::item("&Copy Here"),
            MenuItem::item("&Move Here").bold(true).enabled(enabled),
            MenuItem::item("Create &Shortcuts Here"),
            MenuItem::Separator,
            MenuItem::item("Cancel"),
        ]
    };
    let act =
        |items: &[MenuItem], current| key_act(items, current, MenuKey::Enter, KeyStops::Enabled);
    assert_eq!(
        act(&rows(true), None),
        KeyAct::SelectAnd(1, Box::new(KeyAct::Activate(1)))
    );
    assert_eq!(act(&rows(true), Some(0)), KeyAct::Activate(0));
    assert_eq!(act(&rows(false), None), KeyAct::Nothing);
    let plain = [MenuItem::item("&Open"), MenuItem::item("&Delete")];
    assert_eq!(act(&plain, None), KeyAct::Nothing);
    let sub = [MenuItem::item("Send &To").bold(true).submenu(true)];
    assert_eq!(
        act(&sub, None),
        KeyAct::SelectAnd(0, Box::new(KeyAct::OpenSubmenu(0)))
    );
}

#[test]
fn access_keys_pick_or_cycle_and_escape_closes() {
    let items = [
        MenuItem::item("&Open"),
        MenuItem::item("&Delete"),
        MenuItem::item("&Details"),
    ];
    let act = |current, key| key_act(&items, current, key, KeyStops::Enabled);
    assert_eq!(
        act(None, MenuKey::Char('o')),
        KeyAct::SelectAnd(0, Box::new(KeyAct::Activate(0)))
    );
    assert_eq!(act(None, MenuKey::Char('d')), KeyAct::Select(Some(1)));
    assert_eq!(act(Some(1), MenuKey::Char('d')), KeyAct::Select(Some(2)));
    assert_eq!(act(Some(2), MenuKey::Char('D')), KeyAct::Select(Some(1)));
    assert_eq!(act(None, MenuKey::Escape), KeyAct::CloseTopmost);
}

#[test]
fn home_and_end_light_the_first_and_last_rows_of_a_popup() {
    let theme = wide_theme();
    let text = TextRenderer::load(&FontConfig::default());
    let cx = nested_ctx(&theme, &text);
    let t = nested_tree();
    let mut p = PopupMenu::open(&t, (100, 50), &cx, false);
    assert_eq!(p.key(&t, MenuKey::End, &cx), MenuAction::Redraw);
    assert_eq!(p.hovered(), Some(3), "End passes the separator to Refresh");
    assert_eq!(p.key(&t, MenuKey::Home, &cx), MenuAction::Redraw);
    assert_eq!(p.hovered(), Some(0));
}

#[test]
fn space_runs_the_lit_row_of_a_popup_like_enter() {
    let theme = wide_theme();
    let text = TextRenderer::load(&FontConfig::default());
    let cx = nested_ctx(&theme, &text);
    let t = nested_tree();
    let mut p = PopupMenu::open(&t, (100, 50), &cx, true);
    let space = MenuKey::from_keysym(keysym::SPACE, Some(" ")).expect("Space is a menu key");
    assert_eq!(p.key(&t, space, &cx), MenuAction::Chosen(vec![0]));
    p.key(&t, MenuKey::Down, &cx);
    assert_eq!(p.key(&t, space, &cx), MenuAction::Redraw);
    assert_eq!(p.depth(), 2, "Space opens a submenu row");
    assert_eq!(p.hovered(), Some(0), "the submenu opens on its first row");
}

#[test]
fn a_letter_two_rows_share_cycles_them_in_a_popup_and_runs_only_a_unique_one() {
    let theme = wide_theme();
    let text = TextRenderer::load(&FontConfig::default());
    let cx = nested_ctx(&theme, &text);
    let t = MenuTree::flat(vec![
        MenuItem::item("&Open"),
        MenuItem::item("&Delete"),
        MenuItem::item("&Details"),
    ]);
    let mut p = PopupMenu::open(&t, (100, 50), &cx, false);
    assert_eq!(p.key(&t, MenuKey::Char('d'), &cx), MenuAction::Redraw);
    assert_eq!(p.hovered(), Some(1));
    assert_eq!(p.key(&t, MenuKey::Char('d'), &cx), MenuAction::Redraw);
    assert_eq!(p.hovered(), Some(2));
    assert_eq!(p.key(&t, MenuKey::Char('d'), &cx), MenuAction::Redraw);
    assert_eq!(p.hovered(), Some(1), "the cycle wraps");
    assert_eq!(
        p.key(&t, MenuKey::Char('o'), &cx),
        MenuAction::Chosen(vec![0])
    );
}
