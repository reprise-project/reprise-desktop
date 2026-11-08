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
