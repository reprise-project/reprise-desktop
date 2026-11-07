use super::*;
use crate::access::accesskit::Role;
use crate::text::TextRenderer;
use crate::theme::Theme;

fn users(n: usize) -> Vec<LogonUser> {
    (0..n)
        .map(|i| LogonUser {
            name: format!("user{i}"),
            real_name: format!("User {i}"),
            picture: None,
            note: None,
        })
        .collect()
}

fn typed(m: &mut Logon, s: &str) {
    m.key(LogonKey::Text(s));
}

#[test]
fn classic_starts_typed_with_the_last_user_filled_in() {
    let m = Logon::new(Interface::Classic, users(3), true, Some("user1"));
    assert_eq!(m.screen, Screen::Typed);
    assert_eq!(m.name.text, "user1");
    assert_eq!(m.focus, Part::Password);
    assert!(m.password.is_secret());
    let fresh = Logon::new(Interface::Classic, users(3), true, None);
    assert_eq!((fresh.name.text.as_str(), fresh.focus), ("", Part::Name));
}

#[test]
fn blue_starts_on_the_tiles() {
    let m = Logon::new(Interface::Blue, users(3), true, Some("user2"));
    assert_eq!(m.screen, Screen::Tiles);
    assert_eq!(m.focus, Part::Tile(2), "the last user's tile is selected");
    assert_eq!(m.user(), None, "the tiles alone log nobody on");
}

#[test]
fn glass_starts_on_the_last_users_tile() {
    let m = Logon::new(Interface::Glass, users(3), true, Some("user2"));
    assert_eq!(m.screen, Screen::Tile(2));
    assert_eq!(m.focus, Part::Password);
    assert_eq!(m.user().as_deref(), Some("user2"));
    let unknown = Logon::new(Interface::Glass, users(3), true, Some("gone"));
    assert_eq!(unknown.screen, Screen::Tile(0));
    assert_eq!(unknown.name.text, "user0");
}

#[test]
fn show_users_off_starts_typed_everywhere() {
    for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
        let m = Logon::new(i, users(3), false, None);
        assert_eq!((m.screen, m.focus), (Screen::Typed, Part::Name), "{i:?}");
        let empty = Logon::new(i, Vec::new(), true, None);
        assert_eq!(empty.screen, Screen::Typed, "{i:?} with no users");
    }
}

#[test]
fn enter_with_a_password_submits_the_shown_user() {
    let mut m = Logon::new(Interface::Classic, Vec::new(), false, None);
    typed(&mut m, "alice");
    assert_eq!(m.key(LogonKey::Enter), None, "Enter on the name moves on");
    assert_eq!(m.focus, Part::Password);
    typed(&mut m, "hunter2");
    assert_eq!(m.password.text, "hunter2");
    let submit = Some(LogonAction::Submit {
        user: "alice".into(),
    });
    assert_eq!(m.key(LogonKey::Enter), submit);
    assert_eq!(m.click(Part::Submit), submit);

    let mut g = Logon::new(Interface::Glass, users(2), true, Some("user1"));
    typed(&mut g, "pw");
    assert_eq!(
        g.key(LogonKey::Enter),
        Some(LogonAction::Submit {
            user: "user1".into()
        }),
        "a tile submits its own user"
    );

    let mut b = Logon::new(Interface::Blue, users(2), true, None);
    assert_eq!(b.key(LogonKey::Down), None);
    assert_eq!(b.key(LogonKey::Enter), None, "Enter on a tile opens it");
    assert_eq!((b.screen, b.focus), (Screen::Tile(1), Part::Password));
    assert_eq!(b.user().as_deref(), Some("user1"));
}

#[test]
fn an_empty_name_never_submits() {
    let mut m = Logon::new(Interface::Classic, Vec::new(), false, None);
    m.focus = Part::Password;
    typed(&mut m, "secret");
    assert_eq!(m.key(LogonKey::Enter), None);
    assert_eq!(m.focus, Part::Name, "the empty name takes the focus");
    typed(&mut m, "   ");
    assert_eq!(m.click(Part::Submit), None, "blanks are no name");
}

#[test]
fn escape_on_a_glass_tile_goes_to_the_tiles() {
    let mut g = Logon::new(Interface::Glass, users(3), true, Some("user1"));
    typed(&mut g, "half");
    assert_eq!(g.key(LogonKey::Escape), None);
    assert_eq!((g.screen, g.focus), (Screen::Tiles, Part::Tile(1)));
    assert!(g.password.text.is_empty(), "the password is wiped");
    assert_eq!(g.click(Part::OtherUser), None);
    assert_eq!((g.screen, g.focus), (Screen::Typed, Part::Name));
    assert_eq!(
        g.key(LogonKey::Escape),
        None,
        "typed goes back to the tiles"
    );
    assert_eq!(g.screen, Screen::Tiles);

    let mut b = Logon::new(Interface::Blue, users(3), true, None);
    b.click(Part::Tile(2));
    b.ask(Prompt {
        text: "Code:".into(),
        echo: true,
    });
    assert_eq!(
        b.key(LogonKey::Escape),
        Some(LogonAction::Cancel),
        "leaving a waiting prompt cancels it"
    );
    assert_eq!((b.screen, b.prompt.clone()), (Screen::Tiles, None));

    let mut c = Logon::new(Interface::Classic, users(3), true, Some("user0"));
    typed(&mut c, "pw");
    assert_eq!(c.key(LogonKey::Escape), Some(LogonAction::Cancel));
    assert!(c.name.text.is_empty() && c.password.text.is_empty());
    assert_eq!(c.focus, Part::Name);
}

#[test]
fn down_past_the_column_scrolls_the_first_tile() {
    let mut b = Logon::new(Interface::Blue, users(8), true, None);
    for _ in 0..COLUMN {
        b.key(LogonKey::Down);
    }
    assert_eq!(b.focus, Part::Tile(COLUMN));
    assert_eq!(b.first_tile, 1);
    for _ in 0..10 {
        b.key(LogonKey::Down);
    }
    assert_eq!((b.focus, b.first_tile), (Part::Tile(7), 8 - COLUMN));
    for _ in 0..7 {
        b.key(LogonKey::Up);
    }
    assert_eq!((b.focus, b.first_tile), (Part::Tile(0), 0));
    let last = Logon::new(Interface::Blue, users(8), true, Some("user7"));
    assert_eq!(
        last.first_tile,
        8 - COLUMN,
        "the start keeps the last user in view"
    );
}

#[test]
fn fail_clears_the_password_and_keeps_the_name() {
    let mut m = Logon::new(Interface::Classic, Vec::new(), false, Some("alice"));
    typed(&mut m, "wrong");
    m.set_busy(true);
    m.fail(text::incorrect());
    assert!(m.password.text.is_empty());
    assert!(m.password.is_secret());
    assert_eq!(m.name.text, "alice");
    assert_eq!(m.message.as_deref(), Some(text::incorrect().as_str()));
    assert_eq!(
        (m.screen, m.focus, m.busy),
        (Screen::Typed, Part::Password, false)
    );

    let mut g = Logon::new(Interface::Glass, users(2), true, Some("user1"));
    g.ask(Prompt {
        text: "Code:".into(),
        echo: true,
    });
    typed(&mut g, "123");
    g.fail(text::incorrect());
    assert!(
        g.password.is_secret(),
        "an echoed answer goes back to a secret field"
    );
    assert_eq!((g.screen, g.focus), (Screen::Message, Part::MessageOk));
    assert_eq!(g.key(LogonKey::Enter), None);
    assert_eq!((g.screen, g.focus), (Screen::Tile(1), Part::Password));
    assert_eq!(g.name.text, "user1");
}

#[test]
fn fail_and_its_ok_focus_the_screens_home_when_it_has_no_password_field() {
    let mut blue = Logon::new(Interface::Blue, users(3), true, Some("user1"));
    blue.fail(text::power_refused());
    assert_eq!((blue.screen, blue.focus), (Screen::Tiles, Part::Tile(1)));

    let mut glass = Logon::new(Interface::Glass, users(3), true, None);
    glass.key(LogonKey::Escape);
    glass.fail(text::power_refused());
    assert_eq!(
        (glass.screen, glass.focus),
        (Screen::Message, Part::MessageOk)
    );
    glass.key(LogonKey::Enter);
    assert_eq!((glass.screen, glass.focus), (Screen::Tiles, Part::Tile(0)));
}

#[test]
fn a_logon_keys_debug_hides_the_typed_text() {
    let text = format!("{:?}", LogonKey::Text("hunter2"));
    let insert = format!("{:?}", LogonKey::Edit(EditKey::Insert("hunter2".into())));
    assert_eq!(text, "Text(<redacted>)");
    assert_eq!(insert, "Edit(Insert(<redacted>))");
    let backspace = LogonKey::Edit(EditKey::Backspace { word: true });
    assert_eq!(format!("{backspace:?}"), "Edit(Backspace { word: true })");
    assert_eq!(
        format!("{:?}", LogonKey::Tab { back: true }),
        "Tab { back: true }"
    );
}

#[test]
fn busy_ignores_everything_but_power() {
    let mut m = Logon::new(Interface::Classic, Vec::new(), false, Some("alice"));
    typed(&mut m, "pw");
    m.set_busy(true);
    for key in [
        LogonKey::Text("x"),
        LogonKey::Edit(EditKey::Backspace { word: false }),
        LogonKey::Enter,
        LogonKey::Escape,
        LogonKey::Tab { back: false },
    ] {
        assert_eq!(m.key(key.clone()), None, "{key:?}");
    }
    assert_eq!((m.password.text.as_str(), m.focus), ("pw", Part::Password));
    for part in [Part::Submit, Part::Cancel, Part::Name, Part::Layout] {
        assert_eq!(m.click(part), None, "{part:?}");
    }
    assert_eq!(m.click(Part::Power), Some(LogonAction::Power));
}

#[test]
fn tab_cycles_the_classic_dialog_in_reading_order() {
    let mut m = Logon::new(Interface::Classic, Vec::new(), false, None);
    let mut seen = vec![m.focus];
    for _ in 0..5 {
        m.key(LogonKey::Tab { back: false });
        seen.push(m.focus);
    }
    use Part::*;
    assert_eq!(seen, [Name, Password, Submit, Cancel, Power, Name]);
    m.key(LogonKey::Tab { back: true });
    assert_eq!(m.focus, Power);
    assert_eq!(m.key(LogonKey::Enter), Some(LogonAction::Power));
}

fn classic_layout(m: &Logon) -> LogonLayout {
    let theme = Theme::default();
    let text = TextRenderer::load(&theme.fonts);
    crate::classic::logon::layout(m, &theme, &text, Rect::new(0, 0, 1024, 768))
}

#[test]
fn hit_finds_each_part_of_the_classic_dialog() {
    let mut m = Logon::new(Interface::Classic, Vec::new(), false, None);
    m.layout = Some("EN".into());
    let l = classic_layout(&m);
    let parts: Vec<Part> = l.parts.iter().map(|(p, _)| *p).collect();
    use Part::*;
    assert_eq!(parts, [Name, Password, Submit, Cancel, Power, Layout]);
    for &(part, r) in &l.parts {
        assert_eq!(hit(&l, r.x + r.width / 2, r.y + r.height / 2), Some(part));
        assert_eq!(hit(&l, r.x, r.y), Some(part), "{part:?}'s corner");
    }
    let d = l.dialog.expect("Classic draws a dialog");
    assert_eq!(hit(&l, d.x + 2, d.y + 2), None, "the frame is no part");
    assert_eq!(hit(&l, 5, 5), None, "nor is the desktop");
    assert_eq!(m.click(Layout), Some(LogonAction::NextLayout));
    let none = classic_layout(&Logon::new(Interface::Classic, Vec::new(), false, None));
    assert_eq!(none.rect(Layout), None, "no badge without a layout");
}

#[test]
fn access_names_every_field_and_button() {
    let mut m = Logon::new(Interface::Classic, Vec::new(), false, Some("alice"));
    typed(&mut m, "pw");
    m.fail(text::incorrect());
    let l = classic_layout(&m);
    let mut tree = Tree::new(ax::node(Role::Window, l.output));
    access(&m, &l, &mut tree);
    let find = |role: Role| {
        tree.nodes()
            .filter(|(_, n)| n.role() == role)
            .map(|(id, n)| {
                (
                    id,
                    n.label().map(str::to_owned),
                    n.value().map(str::to_owned),
                )
            })
            .collect::<Vec<_>>()
    };
    let dialog = find(Role::Dialog);
    assert_eq!(dialog[0].1.as_deref(), Some("Log On to Reprise"));
    let name = &find(Role::TextInput)[0];
    assert_eq!(
        (name.1.as_deref(), name.2.as_deref()),
        (Some("User name:"), Some("alice"))
    );
    let pw = &find(Role::PasswordInput)[0];
    assert_eq!(pw.1.as_deref(), Some("Password:"));
    assert_eq!(tree.focus(), pw.0, "the password has the focus");
    let ok = &find(Role::DefaultButton)[0];
    assert_eq!(ok.1.as_deref(), Some("OK"));
    let buttons: Vec<_> = find(Role::Button).into_iter().filter_map(|b| b.1).collect();
    assert_eq!(buttons, ["Cancel", "Shut Down..."]);
    let msg = &find(Role::Label)[0];
    assert_eq!(
        msg.2.as_deref(),
        Some("The user name or password is incorrect.")
    );

    m.password.key(EditKey::Insert("abc".into()));
    let mut again = Tree::new(ax::node(Role::Window, l.output));
    access(&m, &l, &mut again);
    let value = again
        .nodes()
        .find(|(_, n)| n.role() == Role::PasswordInput)
        .and_then(|(_, n)| n.value().map(str::to_owned));
    assert_eq!(
        value.as_deref(),
        Some("\u{25cf}\u{25cf}\u{25cf}"),
        "bullets, never the value"
    );
}
