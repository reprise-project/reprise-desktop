use super::*;
use crate::controls::fingerprints::tests::{hash_form, looks};
use crate::controls::form::{Control, Key};
use crate::interfaces;

fn dialog(added: &[Finger]) -> AddFinger {
    AddFinger::new(added.to_vec(), Scan::Press, 5, 1)
}

fn radio_lit(d: &AddFinger, f: Finger) -> bool {
    matches!(
        d.form.item(f.key()).map(|i| &i.control),
        Some(Control::Radio { enabled: true, .. })
    )
}

fn count(d: &AddFinger) -> String {
    match d.form.item(COUNT).map(|i| &i.control) {
        Some(Control::Label { text, .. }) => text.clone(),
        _ => String::new(),
    }
}

fn hint(d: &AddFinger) -> String {
    match d.form.item(HINT).map(|i| &i.control) {
        Some(Control::Lines(l)) => l.join(" "),
        _ => String::new(),
    }
}

#[test]
fn the_picker_offers_the_right_index_finger_first_and_greys_added_ones() {
    let d = dialog(&[]);
    assert_eq!(d.picked(), Some(Finger::RightIndex));
    let d = dialog(&[Finger::RightIndex, Finger::LeftThumb]);
    assert_eq!(d.picked(), Some(Finger::LeftIndex), "the first free one");
    assert!(!radio_lit(&d, Finger::RightIndex) && !radio_lit(&d, Finger::LeftThumb));
    assert!(radio_lit(&d, Finger::RightThumb));
}

#[test]
fn arrows_walk_the_fingers_of_both_hands() {
    let mut d = dialog(&[]);
    d.form.focus_id(Finger::RightIndex.key());
    let ev = d.form.key(Key::Up);
    d.event(ev);
    assert_eq!(d.picked(), Some(Finger::RightThumb));
    let ev = d.form.key(Key::Up);
    d.event(ev);
    assert_eq!(d.picked(), Some(Finger::LeftLittle));
}

#[test]
fn start_asks_for_the_picked_finger_and_waits_for_the_reader() {
    let mut d = dialog(&[]);
    assert!(matches!(
        d.event(form::Event::Pressed(START)),
        Outcome::Start(Finger::RightIndex)
    ));
    assert_eq!(d.step, Step::Waiting(Finger::RightIndex));
    assert_eq!(d.shown_text(), WAITING);
    assert!(matches!(d.event(form::Event::Cancel), Outcome::Stop));
}

#[test]
fn each_passed_scan_moves_the_bar_and_a_bad_one_says_what_to_do() {
    let mut d = dialog(&[]);
    d.event(form::Event::Default);
    d.started();
    assert_eq!(count(&d), "0 of 5 scans");
    assert!(
        d.shown_text()
            .starts_with("Put your right index finger on the reader")
    );
    d.scanned(Scanned::Passed);
    d.scanned(Scanned::Passed);
    assert_eq!(count(&d), "2 of 5 scans");
    assert_eq!(
        hint(&d),
        "Lift your finger, then put it on the reader again."
    );
    d.scanned(Scanned::NotCentred);
    assert_eq!(count(&d), "2 of 5 scans");
    assert_eq!(hint(&d), "Put your finger in the middle of the reader.");
    assert!(matches!(
        d.form.item(BAR).map(|i| &i.control),
        Some(Control::Progress { done: 2, of: 5 })
    ));
    assert!(matches!(
        d.event(form::Event::Pressed(CANCEL)),
        Outcome::Stop
    ));
    d.scanned(Scanned::Completed);
    assert_eq!(d.step, Step::Added(Finger::RightIndex));
    assert!(matches!(
        d.form.item(BAR).map(|i| &i.control),
        Some(Control::Progress { done: 5, of: 5 })
    ));
    assert!(matches!(d.event(form::Event::Default), Outcome::Close));
}

#[test]
fn a_swipe_reader_asks_for_swipes() {
    let mut d = AddFinger::new(Vec::new(), Scan::Swipe, 0, 1);
    d.event(form::Event::Default);
    d.started();
    assert!(d.shown_text().starts_with("Swipe your right index finger"));
    assert_eq!(count(&d), "", "no count while the reader doesn't say");
    d.scanned(Scanned::TooShort);
    assert_eq!(hint(&d), "The swipe was too short. Swipe again.");
}

#[test]
fn a_failed_add_says_why_and_try_again_starts_the_same_finger() {
    let mut d = dialog(&[]);
    d.form.focus_id(Finger::LeftRing.key());
    let ev = d.form.key(Key::Space);
    d.event(ev);
    d.event(form::Event::Default);
    d.started();
    d.scanned(Scanned::Duplicate);
    assert_eq!(
        d.shown_text(),
        "That fingerprint is already added, as another finger or for another user."
    );
    assert!(matches!(
        d.event(form::Event::Pressed(RETRY)),
        Outcome::Start(Finger::LeftRing)
    ));
    d.refused(Refusal::NotAuthorized);
    assert_eq!(d.shown_text(), "You weren't allowed to add a fingerprint.");
    assert_eq!(count(&d), "", "no count once it failed");
    assert!(matches!(
        d.event(form::Event::Pressed(CLOSE)),
        Outcome::Close
    ));
}

#[test]
fn every_fprintd_answer_reads_as_its_scan() {
    assert_eq!(Scanned::from_key("enroll-stage-passed"), Scanned::Passed);
    assert_eq!(Scanned::from_key("enroll-completed"), Scanned::Completed);
    assert_eq!(
        Scanned::from_key("enroll-remove-and-retry"),
        Scanned::LiftAndRetry
    );
    assert_eq!(Scanned::from_key("enroll-something-new"), Scanned::Error);
}

/// The dialog in each step, by name.
fn steps() -> Vec<(&'static str, AddFinger)> {
    let pick = dialog(&[Finger::LeftThumb]);
    let mut waiting = dialog(&[]);
    waiting.event(form::Event::Default);
    let mut scanning = dialog(&[]);
    scanning.event(form::Event::Default);
    scanning.started();
    scanning.scanned(Scanned::Passed);
    scanning.scanned(Scanned::Passed);
    scanning.scanned(Scanned::Retry);
    let mut added = dialog(&[]);
    added.event(form::Event::Default);
    added.started();
    added.scanned(Scanned::Completed);
    let mut failed = dialog(&[]);
    failed.event(form::Event::Default);
    failed.refused(Refusal::InUse);
    vec![
        ("pick", pick),
        ("waiting", waiting),
        ("scanning", scanning),
        ("added", added),
        ("failed", failed),
    ]
}

#[test]
fn everything_sits_inside_the_client() {
    for (name, d) in steps() {
        let (w, h) = d.client();
        for i in &d.form.items {
            let r = i.rect;
            assert!(
                r.x >= 0 && r.y >= 0 && r.right() <= w && r.bottom() <= h,
                "{name}: {} at {r:?}",
                i.id
            );
        }
    }
}

#[test]
fn the_dialog_fits_its_text_under_every_interface() {
    let over = interfaces::form_overflows(|_, _, _| {
        steps()
            .into_iter()
            .map(|(name, d)| (name.to_string(), d.form))
            .collect()
    });
    assert!(over.is_empty(), "{}", over.join("\n"));
}

#[test]
fn the_dialog_pins_its_pixels_in_every_step_and_look() {
    crate::pins::fonts();
    let mut got = Vec::new();
    for (look, base) in looks() {
        for (step, d) in steps() {
            for s in [1, 2] {
                got.push((
                    format!("{look} {step} {s}"),
                    hash_form(&base, &d.form, d.client(), s),
                ));
            }
        }
    }
    crate::pins::check("add_finger", &got);
}
