use std::time::{Duration, Instant};

use super::*;

const RETURN: u32 = 0xff0d;

/// A prompt's controls: Remember, then Allow and Deny with the focus and the default on Deny.
fn prompt() -> Form {
    let mut f = Form::new(vec![
        Item::new(
            "remember",
            Rect::new(10, 10, 13, 13),
            Control::Check {
                label: "&Remember this choice".into(),
                checked: false,
                enabled: true,
                mixed: None,
            },
        ),
        Item::button("allow", Rect::new(10, 40, 75, 23), "&Allow", true, false),
        Item::button("deny", Rect::new(90, 40, 75, 23), "&Deny", true, true),
    ]);
    f.focus_id("deny");
    f
}

fn consent() -> Form {
    let mut f = prompt();
    f.set_consent();
    f
}

#[test]
fn a_bare_letter_presses_an_access_key_on_a_plain_form() {
    assert_eq!(prompt().key(Key::Char('a')), Event::Pressed("allow"));
}

#[test]
fn a_consent_form_takes_its_access_keys_with_alt_only() {
    let mut f = consent();
    assert_eq!(
        f.key(Key::Char('a')),
        Event::None,
        "a bare a presses nothing"
    );
    assert_eq!(f.key(Key::Char('r')), Event::None, "a bare r ticks nothing");
    assert_eq!(f.checked("remember"), Some(false));
    assert_eq!(f.key(Key::Access('r')), Event::Changed("remember"));
    assert_eq!(f.key(Key::Access('a')), Event::Pressed("allow"));
}

#[test]
fn enter_on_a_consent_form_presses_the_refusing_default() {
    let mut f = consent();
    assert_eq!(f.default_button().map(|i| f.items[i].id), Some("deny"));
    assert_eq!(f.key(Key::Enter), Event::Pressed("deny"));
}

#[test]
fn arrows_and_enter_still_press_the_focused_button_on_a_plain_form() {
    let mut f = prompt();
    assert_eq!(f.key(Key::Up), Event::Redraw);
    assert_eq!(f.key(Key::Enter), Event::Pressed("allow"));
}

#[test]
fn enter_and_space_on_a_consent_form_press_only_the_refusing_default() {
    let mut f = consent();
    assert_eq!(f.key(Key::Tab { back: true }), Event::Redraw);
    assert_eq!(f.focus().map(|i| f.items[i].id), Some("allow"));
    assert_eq!(f.key(Key::Enter), Event::Pressed("deny"));
    assert_eq!(f.key(Key::Space), Event::None);
    assert_eq!(f.key(Key::Tab { back: false }), Event::Redraw);
    assert_eq!(f.key(Key::Space), Event::Pressed("deny"));
}

#[test]
fn arrows_never_move_between_a_consent_forms_buttons() {
    let mut f = consent();
    for k in [Key::Up, Key::Left, Key::Down, Key::Right] {
        assert_eq!(f.key(k.clone()), Event::None, "{k:?}");
        assert_eq!(f.focus().map(|i| f.items[i].id), Some("deny"), "{k:?}");
    }
}

#[test]
fn space_and_arrows_change_no_check_box_or_radio_on_a_consent_form() {
    let build = |consent: bool| {
        let mut f = Form::new(vec![
            Item::radio("yes", 10, 10, "&Yes", "answer"),
            Item::radio("no", 10, 30, "&No", "answer").checked(true),
            Item::check("trust", 10, 50, "&Trust"),
            Item::button("ok", Rect::new(10, 80, 75, 23), "OK", true, true),
        ]);
        f.focus_id("no");
        if consent {
            f.set_consent();
        }
        f
    };
    let mut plain = build(false);
    assert_eq!(plain.key(Key::Up), Event::Changed("yes"));
    let mut f = build(true);
    for k in [Key::Up, Key::Left, Key::Down, Key::Right, Key::Space] {
        assert_eq!(f.key(k.clone()), Event::None, "{k:?}");
    }
    assert_eq!(f.checked("no"), Some(true), "No stays picked");
    f.focus_id("trust");
    assert_eq!(f.key(Key::Space), Event::None);
    assert_eq!(f.checked("trust"), Some(false), "the box stays clear");
    assert_eq!(f.key(Key::Access('t')), Event::Changed("trust"));
    assert_eq!(f.key(Key::Access('y')), Event::Changed("yes"));
}

/// Every sequence of up to four keys typed without Alt.
#[test]
fn no_sequence_of_keys_without_alt_grants_on_a_consent_form() {
    let keys = [
        Key::Up,
        Key::Down,
        Key::Left,
        Key::Right,
        Key::Home,
        Key::End,
        Key::Tab { back: false },
        Key::Tab { back: true },
        Key::Enter,
        Key::Space,
        Key::Char('a'),
        Key::Char('r'),
    ];
    let n = keys.len();
    for len in 1..=4u32 {
        for mut code in 0..n.pow(len) {
            let mut f = consent();
            let mut typed = Vec::new();
            for _ in 0..len {
                let k = keys[code % n].clone();
                code /= n;
                typed.push(k.clone());
                assert_ne!(f.key(k), Event::Pressed("allow"), "{typed:?}");
            }
        }
    }
}

#[test]
fn a_consent_form_counts_keys_and_presses_only_500_ms_after_it_gets_the_keyboard() {
    let mut f = consent();
    let t = Instant::now();
    assert!(!f.consent_key(RETURN, t), "before it ever had the keyboard");
    f.consent_key_up(RETURN);
    assert!(!f.consent_press(t));
    f.consent_focused(t);
    let early = t + Duration::from_millis(499);
    assert!(!f.consent_key(RETURN, early));
    f.consent_key_up(RETURN);
    assert!(!f.consent_press(early));
    let armed = t + CONSENT_ARM;
    assert!(f.consent_key(RETURN, armed));
    assert!(f.consent_press(armed));
}

#[test]
fn a_key_first_down_while_the_form_was_shut_counts_only_after_it_comes_up() {
    let mut f = consent();
    let t = Instant::now();
    f.consent_focused(t);
    assert!(!f.consent_key(RETURN, t + Duration::from_millis(100)));
    let later = t + Duration::from_secs(2);
    assert!(
        !f.consent_key(RETURN, later),
        "its repeat still counts for nothing"
    );
    assert!(f.consent_key(0x61, later), "another key counts");
    f.consent_key_up(RETURN);
    assert!(f.consent_key(RETURN, later), "a fresh press counts");
}

#[test]
fn a_consent_form_waits_again_each_time_it_gets_the_keyboard() {
    let mut f = consent();
    let t = Instant::now();
    f.consent_focused(t);
    assert!(f.consent_press(t + CONSENT_ARM));
    let back = t + Duration::from_secs(5);
    f.consent_focused(back);
    assert!(!f.consent_press(back + Duration::from_millis(200)));
    assert!(f.consent_press(back + CONSENT_ARM));
}

#[test]
fn a_consent_form_built_again_keeps_its_wait() {
    let mut old = consent();
    let t = Instant::now();
    old.consent_focused(t);
    let mut new = consent();
    new.carry(&old);
    assert!(!new.consent_press(t + Duration::from_millis(100)));
    assert!(
        new.consent_press(t + CONSENT_ARM),
        "it arms when the old one would"
    );
}

#[test]
fn a_plain_form_counts_every_key_and_press() {
    let mut f = prompt();
    let t = Instant::now();
    assert!(f.consent_key(RETURN, t));
    assert!(f.consent_press(t));
}

/// The last draw's fit still widens Allow for its label, and `hit` reads that fit too.
#[test]
fn a_tree_published_between_a_move_and_its_draw_routes_a_readers_click_to_the_named_button() {
    use crate::access::{Act, Gesture, Published, ROOT, Request, Route, Tree, gesture};
    let mut f = Form::new(vec![
        Item::button(
            "allow",
            Rect::new(10, 40, 75, 23),
            "&Allow this request for good",
            true,
            false,
        ),
        Item::button("deny", Rect::new(200, 40, 75, 23), "&Deny", true, true),
    ]);
    f.set_consent();
    drawn(&f, &Theme::default());
    assert!(
        f.rendered.fit.borrow().iter().any(|&d| d != (0, 0)),
        "the draw widens Allow for its label"
    );
    let tree = |f: &Form| {
        let mut t = Tree::window("", (800, 100));
        f.access(&mut t, ROOT, &tr(), PX);
        t
    };
    let published = |f: &Form| {
        let mut p = Published::default();
        p.update(tree(f));
        p
    };
    let deny = (tree(&f).find(Role::DefaultButton, "Deny")).unwrap();
    let click = |p: &Published| match p.route(Request {
        node: deny,
        act: Act::Click,
    }) {
        Route::Act(t) => match gesture(Act::Click, t) {
            Some(Gesture::Click((x, y))) => (x as i32, y as i32),
            g => panic!("a click on a button clicks, not {g:?}"),
        },
        r => panic!("a click on a button acts, not {r:?}"),
    };
    let hit = |f: &Form, (x, y): (i32, i32)| f.hit(x, y, &tr(), PX);
    let named = |f: &Form, at| hit(f, at).map(|h| f.items[h.item].id);
    let before = published(&f);

    let (a, d) = (f.rect("allow").unwrap(), f.rect("deny").unwrap());
    let dx = (d.x + d.width / 2) - (a.x + a.width / 2);
    for id in ["allow", "deny"] {
        f.item_mut(id).unwrap().rect.x += dx;
    }
    assert_eq!(
        named(&f, click(&before)),
        Some("allow"),
        "the tree from before the move aims Deny's click at Allow"
    );
    let at = click(&published(&f));
    assert_eq!(named(&f, at), Some("deny"));
    f.click(hit(&f, at));
    assert_eq!(f.release(), Event::Pressed("deny"));
}
