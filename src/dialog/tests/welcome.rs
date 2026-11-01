use super::*;
use reprisekit::interface::Interface;
use reprisekit::interfaces::{draw_power_panel, theme_for};

#[test]
fn log_off_and_turn_off_answer_enter_and_access_keys() {
    let mut d = Dialog::log_off();
    assert_eq!(d.key(Key::Enter), Outcome::Act(Action::Exit));
    assert_eq!(
        d.key(Key::Edit(EditKey::Insert("l".into()))),
        Outcome::Act(Action::Exit)
    );
    assert_eq!(
        d.key(Key::Edit(EditKey::Insert("z".into()))),
        Outcome::Nothing
    );
    assert_eq!(d.key(Key::Escape), Outcome::Close);
    let mut t = Dialog::turn_off(&power());
    assert_eq!(
        t.key(Key::Enter),
        Outcome::EndSession(Goal::TurnOff, "systemctl poweroff".into())
    );
    assert_eq!(
        t.key(Key::Edit(EditKey::Insert("S".into()))),
        Outcome::StandBy("systemctl suspend".into())
    );
    assert_eq!(
        t.key(Key::Access('r')),
        Outcome::EndSession(Goal::Restart, "systemctl reboot".into())
    );
    assert_eq!(t.press(3), Outcome::Close);
    assert_eq!(t.press(9), Outcome::Nothing);
}

#[test]
fn welcome_panels_take_the_panels_layout() {
    let theme = Theme::default();
    let text = TextRenderer::load(&theme.fonts);
    let turn_off = Dialog::turn_off(&power());
    let t = DialogLayout::compute(&theme, &text, &turn_off);
    let lay = t.power.as_ref().expect("the Blue panel's layout");
    assert_eq!(t.size, lay.size);
    assert_eq!(t.caption, lay.title);
    assert!(t.frame.is_none() && t.lines.is_empty() && t.icon.is_none());
    assert_eq!(t.buttons[..3], lay.cells[..]);
    assert_eq!(t.buttons[3], lay.cancel);
    assert_eq!(
        t.hit(&turn_off, 156, 100, &text, theme.fonts.size_px),
        Some(DialogHit::Button(1))
    );
    assert_eq!(
        t.hit(&turn_off, 250, 175, &text, theme.fonts.size_px),
        Some(DialogHit::Button(3))
    );
    assert_eq!(
        t.hit(&turn_off, 100, 20, &text, theme.fonts.size_px),
        None,
        "the title band is no grip"
    );
    let grown = Theme::default().with_text_size(200);
    let g = DialogLayout::compute(&grown, &text, &turn_off);
    assert_eq!(g.size, (628, 400), "the panel grows with the text");
}

#[test]
fn under_glass_the_panels_are_framed_unless_they_are_always_blue() {
    use reprisekit::blue::powerpanel::{TileState, icon};
    let theme = theme_for(Interface::Glass);
    let text = TextRenderer::load(&theme.fonts);
    for (d, icons) in [(Dialog::log_off(), 2), (Dialog::turn_off(&power()), 3)] {
        let l = DialogLayout::compute(&theme, &text, &d);
        let client = l.frame.as_ref().expect("a frame").client;
        assert_eq!(client.width, GLASS_WELCOME_W, "{}", d.title);
        let row = &l.buttons[..icons];
        for r in row {
            assert_eq!((r.width, r.height), GLASS_WELCOME_CELL);
            assert_eq!(r.y, client.y + GLASS_WELCOME_TOP);
        }
        assert_eq!(
            row[0].x - client.x,
            client.right() - row[icons - 1].right(),
            "{}: the row is centred",
            d.title
        );
        let cancel = l.buttons[icons];
        assert_eq!((cancel.width, cancel.height), (BUTTON_W, BUTTON_H));
        assert_eq!(client.right() - cancel.right(), GLASS_WELCOME_MARGIN);
        let c = draw(
            &theme,
            &text,
            &l,
            None,
            None,
            &[],
            &d,
            true,
            None,
            Cues::NONE,
            true,
            None,
        );
        for (i, r) in row.iter().enumerate() {
            let choice = d.buttons[i].choice.expect("a big button");
            let tile = icon(choice, TileState::Normal, reprisekit::powerpanel::ICON, 1);
            let at = (r.x + (r.width - ICON_PX) / 2, r.y + WELCOME_ICON_Y);
            let mid = c.get(at.0 + 16, at.1 + 3);
            if d.buttons[i].enabled {
                assert_eq!(mid, tile.get(16, 3), "{}: tile {i}", d.title);
            } else {
                assert_ne!(mid, tile.get(16, 3), "{}: tile {i} is faded", d.title);
            }
        }
        let blue = d.clone().always_blue();
        let b = DialogLayout::compute(&theme, &text, &blue);
        assert!(b.frame.is_none() && b.power.is_some(), "{}", d.title);
    }
}

#[test]
fn the_panel_draws_as_the_blue_power_panel_with_the_pointer_on_its_parts() {
    use reprisekit::powerpanel::Part;
    let theme = Theme::default();
    let text = TextRenderer::load(&theme.fonts);
    let d = Dialog::turn_off(&power());
    let l = DialogLayout::compute(&theme, &text, &d);
    let lay = l.power.as_ref().unwrap();
    let Style::Welcome(w) = &d.style else {
        unreachable!("a Welcome panel");
    };
    for lit in [
        None,
        Some((2, Interaction::Hot)),
        Some((3, Interaction::Pressed)),
    ] {
        let c = draw(
            &theme,
            &text,
            &l,
            None,
            None,
            &[],
            &d,
            true,
            lit,
            Cues::ALL,
            true,
            None,
        );
        let mut want = Canvas::new(lay.size.0 as u32, lay.size.1 as u32);
        let part = lit.map(|(i, how)| (if i < 3 { Part::Big(i) } else { Part::Cancel }, how));
        draw_power_panel(
            &mut want,
            &text,
            reprisekit::blue::Variant::Blue,
            &w.panel,
            lay,
            part,
            Cues::ALL,
            theme.fonts.size_px,
            1,
        );
        assert_eq!(c.pixels, want.pixels, "{lit:?}");
    }
}

fn focused(d: &Dialog) -> Option<usize> {
    d.buttons.iter().position(|b| b.default)
}

#[test]
fn keys_walk_the_focus_and_the_buttons_follow_it() {
    let mut d = Dialog::log_off();
    assert_eq!(focused(&d), Some(1), "Log Off has the focus");
    assert_eq!(d.key(Key::Left), Outcome::Nothing, "Switch User is greyed");
    assert_eq!(d.key(Key::Tab { back: false }), Outcome::Redraw);
    assert_eq!(focused(&d), Some(2));
    d.key(Key::Tab { back: false });
    assert_eq!(
        focused(&d),
        Some(1),
        "Tab wraps past the greyed Switch User"
    );

    let mut d = Dialog::turn_off(&power());
    let order: Vec<_> = (0..4)
        .map(|_| {
            d.key(Key::Tab { back: false });
            focused(&d)
        })
        .collect();
    assert_eq!(order, [Some(2), Some(3), Some(0), Some(1)]);
    d.key(Key::Left);
    assert_eq!(focused(&d), Some(0), "Left from Turn Off to Stand By");
    let theme = Theme::default();
    let text = TextRenderer::load(&theme.fonts);
    let l = DialogLayout::compute(&theme, &text, &d);
    assert_eq!(
        d.focus_at(&l, 242, 100, &text, theme.fonts.size_px),
        Outcome::Redraw,
        "a reader focuses Restart"
    );
    assert_eq!(focused(&d), Some(2));
    assert_eq!(
        d.key(Key::Space),
        Outcome::EndSession(Goal::Restart, "systemctl reboot".into())
    );
}

#[test]
fn shift_turns_stand_by_into_hibernate() {
    let mut d = Dialog::turn_off(&PowerConfig {
        standby: String::new(),
        hibernate: "systemctl hibernate".into(),
        ..power()
    });
    assert!(!d.buttons[0].enabled, "no standby command greys Stand By");
    assert_eq!(d.welcome_shift(true), Outcome::Redraw);
    assert_eq!(d.buttons[0].label, "&Hibernate");
    assert!(d.buttons[0].enabled && d.buttons[0].tip.is_some());
    assert_eq!(
        d.key(Key::Char('h')),
        Outcome::Hibernate("systemctl hibernate".into())
    );
    assert_eq!(d.welcome_shift(false), Outcome::Redraw);
    assert_eq!(d.buttons[0].label, "&Stand By");
    let mut plain = Dialog::turn_off(&PowerConfig {
        hibernate: String::new(),
        ..power()
    });
    assert_eq!(
        plain.welcome_shift(true),
        Outcome::Nothing,
        "no hibernate command"
    );
}

#[test]
fn welcome_buttons_tip_their_description_but_not_when_greyed() {
    let l = Dialog::log_off();
    assert!(l.buttons[0].tip.is_none(), "greyed Switch User has none");
    assert_eq!(
        l.buttons[1].tip.as_deref(),
        Some("Closes your programs and ends your Reprise session.")
    );
    assert!(l.buttons[2].tip.is_none(), "Cancel has none");
}
