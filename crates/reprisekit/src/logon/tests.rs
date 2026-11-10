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
            stock: i,
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
fn show_users_off_offers_no_switch_user_but_the_lock_still_does() {
    let theme = Theme::default();
    let text = TextRenderer::load(&theme.fonts);
    let output = Rect::new(0, 0, 1024, 768);
    for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
        let mut m = Logon::new(i, users(3), false, None);
        assert!(!m.offers_switch_user(), "{i:?}");
        assert!(
            !m.stops().contains(&Part::SwitchUser),
            "{i:?}: Tab skips it"
        );
        let l = crate::interfaces::logon::logon_layout(&m, &theme, &text, output);
        assert_eq!(l.rect(Part::SwitchUser), None, "{i:?}: not drawn");
        m.click(Part::SwitchUser);
        assert_eq!(m.pick_user(), None, "{i:?}");
        assert_eq!(m.screen, Screen::Typed, "{i:?}: the typed form stays");
        let mut lock = Logon::locked(i, users(1).remove(0), true, true);
        let l = crate::interfaces::logon::logon_layout(&lock, &theme, &text, output);
        assert!(
            l.rect(Part::SwitchUser).is_some(),
            "{i:?}: the lock shows it"
        );
        assert_eq!(lock.click(Part::SwitchUser), Some(LogonAction::SwitchUser));
    }
}

#[test]
fn left_and_right_step_through_the_tiles_as_up_and_down_do() {
    let left = LogonKey::Edit(EditKey::Left {
        select: false,
        word: false,
    });
    let right = LogonKey::Edit(EditKey::Right {
        select: false,
        word: false,
    });
    let mut g = Logon::new(Interface::Glass, users(2), true, None);
    g.key(LogonKey::Escape);
    let mut seen = vec![g.focus];
    for key in [&right, &right, &right, &left, &left, &left] {
        g.key(key.clone());
        seen.push(g.focus);
    }
    use Part::{OtherUser, Tile};
    let row = [
        Tile(0),
        Tile(1),
        OtherUser,
        OtherUser,
        Tile(1),
        Tile(0),
        Tile(0),
    ];
    assert_eq!(seen, row, "Glass's row ends at Other user");
    g.key(LogonKey::Down);
    g.key(LogonKey::Down);
    assert_eq!(g.focus, OtherUser, "Down reaches Other user too");

    let mut b = Logon::new(Interface::Blue, users(3), true, None);
    b.key(right.clone());
    assert_eq!(b.focus, Tile(1), "Blue's column");
    b.key(left.clone());
    assert_eq!(b.focus, Tile(0));

    let mut blue_tile = Logon::new(Interface::Blue, users(3), true, None);
    blue_tile.click(Tile(1));
    let glass_tile = Logon::new(Interface::Glass, users(3), true, Some("user1"));
    for mut m in [blue_tile, glass_tile] {
        let i = m.interface;
        typed(&mut m, "ab");
        let at = (m.screen, m.focus);
        m.key(left.clone());
        assert_eq!(m.password.caret(), 1, "{i:?}: Left moves the caret");
        m.key(right.clone());
        assert_eq!(m.password.caret(), 2, "{i:?}: Right moves it back");
        assert_eq!((m.screen, m.focus), at, "{i:?}: the tile stays");
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
fn glass_ok_puts_the_failure_text_away() {
    let mut g = Logon::new(Interface::Glass, users(2), true, Some("user0"));
    typed(&mut g, "wrong");
    g.fail(text::incorrect());
    g.click(Part::MessageOk);
    assert_eq!(g.screen, Screen::Tile(0));
    assert_eq!(g.message, None, "the tile shows no failure text after OK");
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
fn busy_ignores_everything_but_tab_and_power() {
    let mut m = Logon::new(Interface::Classic, Vec::new(), false, Some("alice"));
    typed(&mut m, "pw");
    m.set_busy(true);
    let ignored = [
        LogonKey::Text("x"),
        LogonKey::Edit(EditKey::Backspace { word: false }),
        LogonKey::Enter,
        LogonKey::Escape,
        LogonKey::Up,
        LogonKey::Down,
    ];
    for key in &ignored {
        assert_eq!(m.key(key.clone()), None, "{key:?}");
    }
    assert_eq!((m.password.text.as_str(), m.focus), ("pw", Part::Password));
    for part in [Part::Submit, Part::Cancel, Part::Name, Part::Layout] {
        assert_eq!(m.click(part), None, "{part:?}");
    }
    assert_eq!(m.click(Part::Power), Some(LogonAction::Power));

    assert_eq!(m.key(LogonKey::Tab { back: false }), None);
    assert_eq!(m.focus, Part::Submit, "Tab moves on while busy");
    assert_eq!(m.key(LogonKey::Enter), None, "the greyed OK takes no Enter");
    m.key(LogonKey::Tab { back: true });
    assert_eq!(m.focus, Part::Password, "Shift+Tab moves back");
    m.key(LogonKey::Tab { back: true });
    m.key(LogonKey::Tab { back: true });
    assert_eq!(m.focus, Part::Power, "Shut Down... is reachable");
    assert_eq!(m.key(LogonKey::Enter), Some(LogonAction::Power));
    for key in &ignored[..2] {
        m.key(LogonKey::Tab { back: false });
        m.key(key.clone());
    }
    assert_eq!(m.name.text, "alice", "no text goes in while busy");
    assert_eq!(m.password.text, "pw");
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

#[test]
fn the_power_menu_arrow_is_named_for_its_options() {
    let m = Logon::new(Interface::Glass, users(1), true, None);
    let l = LogonLayout {
        output: Rect::new(0, 0, 1024, 768),
        parts: vec![(Part::PowerMenu, Rect::new(990, 730, 20, 24))],
        dialog: None,
    };
    let mut tree = Tree::new(ax::node(Role::Window, l.output));
    access(&m, &l, &mut tree);
    let label = tree
        .nodes()
        .find(|(_, n)| n.role() == Role::Button)
        .and_then(|(_, n)| n.label().map(str::to_owned));
    assert_eq!(label.as_deref(), Some("Shut down options"));
}

#[test]
fn the_admin_heading_differs_from_the_login_heading() {
    use crate::interfaces::logon::{draw_logon, logon_layout};
    let output = Rect::new(0, 0, 1024, 768);
    for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
        let theme = crate::interfaces::theme_for(i);
        let text = TextRenderer::load(&theme.fonts);
        let drawn = |m: &Logon| {
            let l = logon_layout(m, &theme, &text, output);
            let mut c = Canvas::new(output.width as u32, output.height as u32);
            draw_logon(&mut c, m, &l, &theme, text.for_output(1), 1);
            (l, c.pixels)
        };
        let login = Logon::new(i, Vec::new(), false, None);
        let mut off = login.clone();
        off.admin(PowerAction::PowerOff);
        let mut reboot = login.clone();
        reboot.admin(PowerAction::Reboot);
        let ((l0, c0), (l1, c1), (l2, c2)) = (drawn(&login), drawn(&off), drawn(&reboot));
        assert_eq!(l0, l1, "{i:?}: the form keeps the login form's parts");
        assert_eq!(l1, l2, "{i:?}: either action, the same parts");
        assert_ne!(c0, c1, "{i:?}: the administrator's heading is drawn");
        assert_ne!(
            c1, c2,
            "{i:?}: shutting down and restarting read differently"
        );
    }
}

#[test]
fn the_admin_form_starts_empty_and_escape_leaves_it() {
    for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
        let mut m = Logon::new(i, users(2), true, Some("user1"));
        m.fail(text::incorrect());
        m.admin(PowerAction::PowerOff);
        assert_eq!(m.purpose, Purpose::Admin(PowerAction::PowerOff), "{i:?}");
        assert_eq!(
            (
                m.screen,
                m.name.text.as_str(),
                m.focus,
                m.message.as_deref()
            ),
            (Screen::Typed, "", Part::Name, None),
            "{i:?}: the typed form, empty, with no old message"
        );
        assert_eq!(m.heading(), Some(text::admin_shut_down()), "{i:?}");
        typed(&mut m, "anna");
        assert_eq!(m.key(LogonKey::Escape), Some(LogonAction::Cancel), "{i:?}");
        assert_eq!((m.purpose, m.heading()), (Purpose::Login, None), "{i:?}");
        let start = match i {
            Interface::Classic => Screen::Typed,
            _ => Screen::Tiles,
        };
        assert_eq!(m.screen, start, "{i:?}");
        assert_eq!(m.name.text, "", "{i:?}: the administrator's name is gone");
    }
    let mut g = Logon::new(Interface::Glass, users(2), true, None);
    g.admin(PowerAction::Reboot);
    assert_eq!(g.click(Part::SwitchUser), Some(LogonAction::Cancel));
    assert_eq!((g.purpose, g.screen), (Purpose::Login, Screen::Tiles));
    let mut c = Logon::new(Interface::Classic, Vec::new(), false, None);
    c.admin(PowerAction::Reboot);
    assert_eq!(c.click(Part::Cancel), Some(LogonAction::Cancel));
    assert_eq!(c.purpose, Purpose::Login);
}

#[test]
fn leaving_the_admin_form_gives_the_focus_back_to_what_had_it() {
    let mut b = Logon::new(Interface::Blue, users(3), true, None);
    b.key(LogonKey::Down);
    b.admin(PowerAction::PowerOff);
    b.key(LogonKey::Escape);
    assert_eq!((b.screen, b.focus), (Screen::Tiles, Part::Tile(1)));

    let mut g = Logon::new(Interface::Glass, users(3), true, Some("user2"));
    g.admin(PowerAction::Reboot);
    g.key(LogonKey::Escape);
    assert_eq!(
        (g.screen, g.focus),
        (Screen::Tiles, Part::Tile(2)),
        "an open tile's user"
    );

    let mut other = Logon::new(Interface::Glass, users(3), true, None);
    other.click(Part::OtherUser);
    other.admin(PowerAction::PowerOff);
    other.key(LogonKey::Escape);
    assert_eq!(other.focus, Part::OtherUser, "Other user's form");

    let mut power = Logon::new(Interface::Blue, users(3), true, Some("user1"));
    power.focus = Part::Power;
    power.admin(PowerAction::PowerOff);
    power.admin(PowerAction::Reboot);
    power.key(LogonKey::Escape);
    assert_eq!(power.focus, Part::Power, "the button that asked for it");
}

#[test]
fn going_down_keeps_the_form_busy_and_says_so() {
    let mut m = Logon::new(Interface::Classic, Vec::new(), false, None);
    assert_eq!(m.busy_text(), text::verifying());
    m.admin(PowerAction::Reboot);
    m.going_down();
    assert!(m.busy);
    assert_eq!(m.busy_text(), text::restarting());
    assert_eq!(m.key(LogonKey::Escape), None, "nothing leaves it");
    m.admin(PowerAction::PowerOff);
    m.going_down();
    assert_eq!(m.busy_text(), text::shutting_down());
}

#[test]
fn a_failure_after_going_down_gives_the_form_back() {
    for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
        let mut m = Logon::new(i, users(2), true, None);
        m.admin(PowerAction::PowerOff);
        typed(&mut m, "anna");
        m.going_down();
        m.fail(text::power_failed());
        assert!(!m.busy, "{i:?}");
        assert_eq!(m.message, Some(text::power_failed()), "{i:?}");
        m.set_busy(true);
        assert_eq!(
            m.busy_text(),
            text::verifying(),
            "{i:?}: the next try verifies"
        );
    }
}

#[test]
fn a_remote_note_reads_connected_remotely() {
    assert_eq!(UserNote::Remote.text(), "Connected remotely");
    assert_eq!(UserNote::LoggedOn.text(), "Logged on");
    let mut us = users(2);
    us[0].note = Some(UserNote::LoggedOn);
    us[1].note = Some(UserNote::Remote);
    let m = Logon::new(Interface::Glass, us, true, None);
    let l = LogonLayout {
        output: Rect::new(0, 0, 1024, 768),
        parts: vec![
            (Part::Tile(0), Rect::new(100, 100, 100, 100)),
            (Part::Tile(1), Rect::new(300, 100, 100, 100)),
        ],
        dialog: None,
    };
    let mut tree = Tree::new(ax::node(Role::Window, l.output));
    access(&m, &l, &mut tree);
    let notes: Vec<Option<String>> = tree
        .nodes()
        .filter(|(_, n)| n.role() == Role::ListItem)
        .map(|(_, n)| n.description().map(str::to_owned))
        .collect();
    assert_eq!(
        notes,
        [Some("Logged on".into()), Some("Connected remotely".into())]
    );
}

#[test]
fn the_admin_heading_is_read_out_with_the_form() {
    let mut m = Logon::new(Interface::Classic, Vec::new(), false, None);
    m.admin(PowerAction::PowerOff);
    let l = classic_layout(&m);
    let mut tree = Tree::new(ax::node(Role::Window, l.output));
    access(&m, &l, &mut tree);
    let said = tree
        .nodes()
        .find(|(_, n)| n.role() == Role::Label)
        .and_then(|(_, n)| n.value().map(str::to_owned));
    assert_eq!(said, Some(text::admin_shut_down()));
}

const ALL: [Interface; 3] = [Interface::Classic, Interface::Blue, Interface::Glass];

fn locked(i: Interface, switch_user: bool) -> Logon {
    Logon::locked(i, users(1).remove(0), switch_user, true)
}

#[test]
fn the_lock_opens_on_its_users_password_with_the_name_fixed() {
    for i in ALL {
        let m = locked(i, true);
        let screen = match i {
            Interface::Classic => Screen::Typed,
            _ => Screen::Tile(0),
        };
        assert_eq!((m.screen, m.focus), (screen, Part::Password), "{i:?}");
        assert_eq!(m.user().as_deref(), Some("user0"), "{i:?}");
        assert!(m.name.is_read_only(), "{i:?}: the name is fixed");
        assert_eq!(m.users.len(), 1, "{i:?}: the lock lists only its user");
        assert_eq!(m.users[0].note, Some(UserNote::Locked), "{i:?}");
    }
    assert_eq!(UserNote::Locked.text(), "Locked");
}

#[test]
fn escape_and_cancel_on_the_lock_only_clear_the_password() {
    for i in ALL {
        let mut m = locked(i, true);
        let screen = m.screen;
        typed(&mut m, "half");
        assert_eq!(m.key(LogonKey::Escape), Some(LogonAction::Cancel), "{i:?}");
        assert!(m.password.text.is_empty(), "{i:?}: the password is wiped");
        assert_eq!((m.screen, m.focus), (screen, Part::Password), "{i:?}");
        assert_eq!(m.name.text, "user0", "{i:?}: the name stays");
        assert!(m.name.is_read_only(), "{i:?}: and stays fixed");
        typed(&mut m, "again");
        m.click(Part::Cancel);
        assert!(m.password.text.is_empty(), "{i:?}: Cancel wipes it too");
        assert_eq!((m.screen, m.name.text.as_str()), (screen, "user0"));
        m.click(Part::Name);
        typed(&mut m, "x");
        m.key(LogonKey::Edit(EditKey::Backspace { word: true }));
        assert_eq!(m.name.text, "user0", "{i:?}: nothing edits the name");
    }
}

#[test]
fn the_lock_offers_switch_user_only_when_the_host_says_so() {
    for i in ALL {
        let mut on = locked(i, true);
        assert!(on.stops().contains(&Part::SwitchUser), "{i:?}");
        assert_eq!(on.click(Part::SwitchUser), Some(LogonAction::SwitchUser));
        assert_eq!(on.users.len(), 1, "{i:?}: the lock still lists one user");
        assert_ne!(
            on.screen,
            Screen::Tiles,
            "{i:?}: the tiles are the greeter's"
        );
        let off = locked(i, false);
        assert!(!off.stops().contains(&Part::SwitchUser), "{i:?}");
        for p in off.stops() {
            assert!(
                !matches!(p, Part::OtherUser | Part::Tile(_)),
                "{i:?}: {p:?}"
            );
        }
        assert!(
            off.stops().contains(&Part::Power),
            "{i:?}: the power button stays"
        );
    }
}

#[test]
fn a_failed_unlock_keeps_the_user_fixed() {
    for i in ALL {
        let mut m = locked(i, true);
        typed(&mut m, "wrong");
        let sent = m.key(LogonKey::Enter);
        assert_eq!(
            sent,
            Some(LogonAction::Submit {
                user: "user0".into()
            }),
            "{i:?}"
        );
        m.set_busy(true);
        m.fail(text::incorrect());
        assert!(m.password.text.is_empty(), "{i:?}");
        assert!(m.name.is_read_only(), "{i:?}");
        if i == Interface::Glass {
            assert_eq!(m.screen, Screen::Message);
            m.click(Part::MessageOk);
            assert_eq!((m.screen, m.focus), (Screen::Tile(0), Part::Password));
        }
        assert_eq!(m.user().as_deref(), Some("user0"), "{i:?}");
    }
}

#[test]
fn the_classic_lock_says_it_is_locked_and_names_its_dialog() {
    let m = locked(Interface::Classic, false);
    assert_eq!(m.said(), Some(text::locked_heading()));
    assert_ne!(m.title(), text::title());
    let l = classic_layout(&m);
    let mut tree = Tree::new(ax::node(Role::Window, l.output));
    access(&m, &l, &mut tree);
    let dialog = tree.nodes().find(|(_, n)| n.role() == Role::Dialog);
    assert_eq!(
        dialog.and_then(|(_, n)| n.label().map(str::to_owned)),
        Some(text::unlock_title())
    );
    for i in [Interface::Blue, Interface::Glass] {
        assert_eq!(locked(i, false).said(), None, "{i:?}: the tile says Locked");
    }
}

#[test]
fn a_hint_shows_while_no_message_does_and_outlasts_escape() {
    for i in ALL {
        let mut m = locked(i, false);
        let heading = m.said();
        m.hint = Some("Place your finger".into());
        assert_eq!(m.said().as_deref(), Some("Place your finger"), "{i:?}");
        m.info("Failed to match fingerprint".into());
        assert_eq!(m.said().as_deref(), Some("Failed to match fingerprint"));
        m.key(LogonKey::Escape);
        assert_eq!(m.said().as_deref(), Some("Place your finger"), "{i:?}");
        m.hint = None;
        assert_eq!(m.said(), heading, "{i:?}");
    }
}

#[test]
fn a_lock_without_power_shows_and_answers_no_power_button() {
    let theme = Theme::default();
    let text = TextRenderer::load(&theme.fonts);
    let output = Rect::new(0, 0, 1024, 768);
    let power = |p: &Part| matches!(p, Part::Power | Part::PowerMenu);
    for i in ALL {
        let mut m = Logon::locked(i, users(1).remove(0), true, false);
        assert!(!m.offers_power(), "{i:?}");
        assert!(!m.stops().iter().any(power), "{i:?}: Tab never reaches it");
        let l = crate::interfaces::logon::logon_layout(&m, &theme, &text, output);
        assert!(!l.parts.iter().any(|(p, _)| power(p)), "{i:?}: not drawn");
        assert_eq!(m.click(Part::Power), None, "{i:?}");
        assert_eq!(m.click(Part::PowerMenu), None, "{i:?}");
        m.focus = Part::Power;
        m.set_busy(true);
        assert_eq!(m.key(LogonKey::Enter), None, "{i:?}: nor while busy");
        let with = Logon::locked(i, users(1).remove(0), true, true);
        let l = crate::interfaces::logon::logon_layout(&with, &theme, &text, output);
        assert!(l.rect(Part::Power).is_some(), "{i:?}: offered, it shows");
    }
}

#[test]
fn switch_users_screen_opens_on_the_tiles_in_blue_and_glass() {
    for i in [Interface::Blue, Interface::Glass] {
        let mut m = Logon::new(i, users(3), true, Some("user1"));
        m.click(Part::Tile(1));
        typed(&mut m, "half");
        assert_eq!(m.pick_user(), None, "{i:?}: nothing was waiting");
        assert_eq!(m.screen, Screen::Tiles, "{i:?}");
        assert!(m.password.text.is_empty(), "{i:?}: the password is wiped");
        m.click(Part::Tile(2));
        m.ask(Prompt {
            text: "Code:".into(),
            echo: true,
        });
        assert_eq!(m.pick_user(), Some(LogonAction::Cancel), "{i:?}");
        assert_eq!((m.screen, m.prompt.clone()), (Screen::Tiles, None), "{i:?}");
    }
    let mut classic = Logon::new(Interface::Classic, users(3), true, Some("user1"));
    assert_eq!(classic.pick_user(), None);
    assert_eq!(classic.screen, Screen::Typed, "Classic has no tiles");
    let mut busy = Logon::new(Interface::Glass, users(3), true, Some("user1"));
    busy.set_busy(true);
    busy.pick_user();
    assert_eq!(busy.screen, Screen::Tile(1), "a check in progress stays");
    let mut lock = Logon::locked(Interface::Glass, users(1).remove(0), true, true);
    lock.pick_user();
    assert_eq!(
        lock.screen,
        Screen::Tile(0),
        "the lock lists no one to pick"
    );
}

#[test]
fn a_locks_field_holds_512_bytes_under_a_prompt_and_after_a_failure_too() {
    let long = "p".repeat(600);
    for i in ALL {
        let mut m = locked(i, false);
        typed(&mut m, &long);
        assert_eq!(m.password.text.len(), UNLOCK_MAX, "{i:?}");
        m.set_busy(true);
        m.fail(text::incorrect());
        m.click(Part::MessageOk);
        typed(&mut m, &long);
        assert_eq!(m.password.text.len(), UNLOCK_MAX, "{i:?}: after a failure");
        for echo in [false, true] {
            m.ask(Prompt {
                text: "Code:".into(),
                echo,
            });
            typed(&mut m, &long);
            assert_eq!(m.password.text.len(), UNLOCK_MAX, "{i:?}: echo {echo}");
        }
    }
    let mut login = Logon::new(Interface::Classic, Vec::new(), false, Some("alice"));
    typed(&mut login, &long);
    assert_eq!(
        login.password.text.len(),
        600,
        "the login screen's is not capped"
    );
}

#[test]
fn a_lock_without_switch_user_never_answers_it() {
    for i in ALL {
        let mut m = locked(i, false);
        assert_eq!(m.click(Part::SwitchUser), None, "{i:?}");
        m.focus = Part::SwitchUser;
        assert_eq!(m.key(LogonKey::Enter), None, "{i:?}: nor a left focus");
    }
}

#[test]
fn the_blue_lock_tabs_through_the_bottom_band_left_to_right() {
    let mut m = locked(Interface::Blue, true);
    let mut seen = vec![m.focus];
    for _ in 0..4 {
        m.key(LogonKey::Tab { back: false });
        seen.push(m.focus);
    }
    use Part::*;
    assert_eq!(seen, [Password, Submit, Power, SwitchUser, Password]);
}

#[test]
fn the_blue_and_glass_locks_read_out_the_locked_tile_and_their_buttons() {
    let theme = Theme::default();
    let fonts = TextRenderer::load(&theme.fonts);
    let output = Rect::new(0, 0, 1024, 768);
    for i in [Interface::Blue, Interface::Glass] {
        for power in [true, false] {
            let mut user = users(1).remove(0);
            user.real_name = "User Zero".into();
            let m = Logon::locked(i, user, true, power);
            let l = crate::interfaces::logon::logon_layout(&m, &theme, &fonts, output);
            let mut tree = Tree::new(ax::node(Role::Window, l.output));
            access(&m, &l, &mut tree);
            let tile = tree.nodes().find(|(_, n)| n.role() == Role::ListItem);
            let tile = tile.map(|(_, n)| (n.label().map(str::to_owned), n.description()));
            let tile = tile.map(|(l, d)| (l, d.map(str::to_owned)));
            assert_eq!(
                tile,
                Some((Some("User Zero".into()), Some("Locked".into()))),
                "{i:?}"
            );
            let buttons: Vec<String> = (tree.nodes())
                .filter(|(_, n)| n.role() == Role::Button)
                .filter_map(|(_, n)| n.label().map(str::to_owned))
                .collect();
            assert!(
                buttons.iter().any(|b| b == "Switch User"),
                "{i:?}: {buttons:?}"
            );
            let powers = [text::turn_off(), text::power_off(), text::power_options()];
            let named = buttons.iter().any(|b| powers.contains(b));
            assert_eq!(named, power, "{i:?} power {power}: {buttons:?}");
        }
    }
}
