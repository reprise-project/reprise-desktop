use super::*;
use reprisekit::interface::Interface;
use reprisekit::interfaces::{draw_power_panel, theme_for};
use reprisekit::powerpanel::Updates;

#[test]
fn log_off_and_turn_off_answer_enter_and_access_keys() {
    let mut d = log_off();
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
    let mut t = turn_off();
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
    let turn_off = turn_off();
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
    for (d, icons) in [(log_off(), 2), (turn_off(), 3)] {
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
            &d,
            true,
            None,
            Cues::NONE,
            true,
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
fn restart_into_is_a_button_after_cancel_left_of_it_under_glass() {
    let mut d = turn_off();
    assert!(d.offer_restart_into(true));
    assert!(!d.offer_restart_into(true), "already offered");
    assert_eq!(d.buttons.len(), 5);
    assert_eq!(d.buttons[3].action, DialogAction::Cancel);
    assert_eq!(d.buttons[4].action, DialogAction::RestartInto);
    assert_eq!(d.press(4), Outcome::RestartInto { keys: false });
    assert_eq!(d.key(Key::Access('i')), Outcome::RestartInto { keys: true });
    let classic = Theme::default();
    let text = TextRenderer::load(&classic.fonts);
    let l = DialogLayout::compute(&classic, &text, &d);
    assert_eq!(Some(l.buttons[4]), l.power.as_ref().unwrap().restart_into);
    let glass = theme_for(Interface::Glass);
    let g = DialogLayout::compute(&glass, &text, &d);
    let (cancel, into) = (g.buttons[3], g.buttons[4]);
    assert_eq!(into.y, cancel.y);
    assert_eq!(
        cancel.x - into.right(),
        reprisekit::classic::widgets::BUTTON_GAP,
        "left of Cancel"
    );
    if let Style::Welcome(w) = &mut d.style {
        w.panel.focus = reprisekit::powerpanel::Part::RestartInto;
    }
    assert!(d.offer_restart_into(false));
    assert_eq!(d.buttons.len(), 4);
    assert_eq!(d.buttons[3].action, DialogAction::Cancel);
    assert!(d.buttons[3].default, "the focus went to Cancel");
}

fn assert_draws_as_the_power_panel(theme: &Theme, d: &Dialog, cancel: Option<&Theme>) {
    use reprisekit::powerpanel::Part;
    let text = TextRenderer::load(&theme.fonts);
    let l = DialogLayout::compute(theme, &text, d);
    let lay = l.power.as_ref().unwrap();
    let Style::Welcome(w) = &d.style else {
        unreachable!("a Welcome panel");
    };
    let paint = cancel.map(Theme::paint);
    for lit in [
        None,
        Some((2, Interaction::Hot)),
        Some((3, Interaction::Pressed)),
    ] {
        let c = draw(
            theme,
            &text,
            &l,
            None,
            None,
            d,
            true,
            lit,
            Cues::ALL,
            true,
            true,
            None,
        );
        let mut want = Canvas::new(lay.size.0 as u32, lay.size.1 as u32);
        let part = lit.map(|(i, how)| (if i < 3 { Part::Big(i) } else { Part::Cancel }, how));
        draw_power_panel(
            &mut want,
            &text,
            reprisekit::interfaces::power_panel_variant(&theme.look),
            paint.as_ref(),
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

#[test]
fn under_classic_the_panel_draws_classics_cancel_unless_always_blue() {
    let classic = Theme::default();
    let d = turn_off();
    assert_draws_as_the_power_panel(&classic, &d, Some(&classic));
    assert_draws_as_the_power_panel(&classic, &d.always_blue(), None);
}

#[test]
fn under_blue_the_panel_draws_blues_cancel() {
    let blue = theme_for(Interface::Blue);
    let d = turn_off();
    assert_draws_as_the_power_panel(&blue, &d, None);
    assert_draws_as_the_power_panel(&blue, &d.always_blue(), None);
}

fn focused(d: &Dialog) -> Option<usize> {
    d.buttons.iter().position(|b| b.default)
}

#[test]
fn keys_walk_the_focus_and_the_buttons_follow_it() {
    let mut d = log_off();
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

    let mut d = turn_off();
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
    let mut d = Dialog::turn_off(
        &PowerConfig {
            standby: String::new(),
            hibernate: "systemctl hibernate".into(),
            ..power()
        },
        false,
    );
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
    let mut plain = Dialog::turn_off(
        &PowerConfig {
            hibernate: String::new(),
            ..power()
        },
        false,
    );
    assert_eq!(
        plain.welcome_shift(true),
        Outcome::Nothing,
        "no hibernate command"
    );
}

#[test]
fn welcome_buttons_tip_their_description_but_not_when_greyed() {
    let l = log_off();
    assert!(l.buttons[0].tip.is_none(), "greyed Switch User has none");
    assert_eq!(
        l.buttons[1].tip.as_deref(),
        Some("Closes your programs and ends your Reprise session.")
    );
    assert!(l.buttons[2].tip.is_none(), "Cancel has none");
}

fn turn_off_with_updates() -> Dialog {
    let mut d = turn_off();
    assert!(d.offer_updates(true, false));
    d
}

fn ends(goal: Goal, updates: Updates) -> Outcome {
    let cmd = match goal {
        Goal::Restart => "systemctl reboot",
        _ => "systemctl poweroff",
    };
    Outcome::EndSessionUpdates {
        goal,
        cmd: cmd.into(),
        updates,
    }
}

#[test]
fn a_prepared_update_turns_turn_off_and_restart_into_their_install_variants() {
    let mut t = turn_off_with_updates();
    assert_eq!(t.key(Key::Enter), ends(Goal::TurnOff, Updates::Install));
    assert_eq!(t.press(4), Outcome::Redraw, "the check box");
    assert_eq!(
        t.key(Key::Access('r')),
        ends(Goal::Restart, Updates::Skip),
        "unticked by the user"
    );
    assert_eq!(
        t.key(Key::Edit(EditKey::Insert("S".into()))),
        Outcome::StandBy("systemctl suspend".into()),
        "Stand By installs nothing"
    );
    assert!(
        !log_off().offer_updates(true, false),
        "Log Off has no check box"
    );
    assert!(
        !Dialog::greeter_turn_off().offer_updates(true, false),
        "nor the login screen's"
    );
    assert!(t.offer_updates(false, true));
    assert_eq!(
        t.key(Key::Access('u')),
        Outcome::EndSession(Goal::TurnOff, "systemctl poweroff".into())
    );
}

#[test]
fn a_box_that_comes_after_the_first_frame_shows_unticked_and_installs_nothing() {
    let mut t = turn_off();
    assert!(t.offer_updates(true, true));
    assert!(matches!(&t.style, Style::Welcome(w) if !w.panel.installs_updates()));
    assert_eq!(
        t.key(Key::Enter),
        Outcome::EndSession(Goal::TurnOff, "systemctl poweroff".into()),
        "an untouched late box leaves the update as it is"
    );
    let mut t = turn_off();
    t.offer_updates(true, true);
    assert_eq!(
        t.press(2),
        Outcome::EndSession(Goal::Restart, "systemctl reboot".into()),
        "the box landing under a press keeps it a plain Restart"
    );
}

#[test]
fn a_ticked_box_greys_restart_into_and_lays_out_beside_it() {
    let mut t = turn_off();
    assert!(t.offer_restart_into(true));
    assert!(t.offer_updates(true, false));
    let into = t
        .buttons
        .iter()
        .position(|b| b.action == DialogAction::RestartInto);
    let into = into.expect("Restart Into...");
    assert!(!t.buttons[into].enabled, "greyed while the box is ticked");
    assert_eq!(t.key(Key::Access('i')), Outcome::Nothing);
    let check = t
        .buttons
        .iter()
        .position(|b| b.action == DialogAction::ToggleUpdates);
    assert_eq!(t.press(check.unwrap()), Outcome::Redraw);
    assert!(t.buttons[into].enabled, "live once unticked");
    let theme = Theme::default();
    let text = TextRenderer::load(&theme.fonts);
    let glass = theme_for(Interface::Glass);
    let g = DialogLayout::compute(&glass, &text, &t);
    let (cancel, into, check) = (g.buttons[3], g.buttons[4], g.buttons[5]);
    assert!(
        check.right() < into.x && into.right() < cancel.x,
        "{check:?} {into:?} {cancel:?}"
    );
    let l = DialogLayout::compute(&theme, &text, &t);
    let lay = l.power.as_ref().unwrap();
    assert_eq!(
        (l.buttons[4], l.buttons[5]),
        (lay.restart_into.unwrap(), lay.updates.unwrap())
    );
}

#[test]
fn the_check_box_sits_left_of_cancel_in_the_panel_and_the_framed_dialog() {
    let t = turn_off_with_updates();
    let theme = Theme::default();
    let text = TextRenderer::load(&theme.fonts);
    let l = DialogLayout::compute(&theme, &text, &t);
    let lay = l.power.as_ref().unwrap();
    assert_eq!(Some(l.buttons[4]), lay.updates);
    assert_eq!(
        l.hit(&t, 15, 175, &text, theme.fonts.size_px),
        Some(DialogHit::Button(4))
    );
    let glass = theme_for(Interface::Glass);
    let g = DialogLayout::compute(&glass, &text, &t);
    let client = g.frame.as_ref().expect("a frame").client;
    let (cancel, check) = (g.buttons[3], g.buttons[4]);
    assert_eq!((check.y, check.height), (cancel.y, cancel.height));
    assert_eq!(check.x, client.x + GLASS_WELCOME_MARGIN);
    assert!(check.right() < cancel.x);
    assert_eq!(
        g.size,
        DialogLayout::compute(&glass, &text, &turn_off()).size
    );
}

#[test]
fn the_framed_dialog_draws_and_names_the_check_box() {
    let t = turn_off_with_updates();
    let glass = theme_for(Interface::Glass);
    let text = TextRenderer::load(&glass.fonts);
    let l = DialogLayout::compute(&glass, &text, &t);
    let with = render(&glass, &text, &l, &t);
    let plain = turn_off();
    let without = render(
        &glass,
        &text,
        &DialogLayout::compute(&glass, &text, &plain),
        &plain,
    );
    let r = l.buttons[4];
    let differs = (r.y..r.bottom())
        .flat_map(|y| (r.x..r.right()).map(move |x| (x, y)))
        .filter(|&(x, y)| with.get(x, y) != without.get(x, y))
        .count();
    assert!(differs > 60, "the box and its label ({differs} px)");
    let (tree, _) = tree_of(&t);
    let check = tree
        .nodes()
        .map(|(_, n)| n)
        .find(|n| n.role() == reprisekit::access::accesskit::Role::CheckBox)
        .expect("a check box");
    assert_eq!(check.label(), Some("Install updates"));
    assert_eq!(
        check.toggled(),
        Some(reprisekit::access::accesskit::Toggled::True)
    );
}
