use super::*;

#[test]
fn entering_a_folder_asks_for_a_listing() {
    let mut d = sample();
    d.set_listing(folder(1, None, vec![dir("Docs"), file("a.txt")]));
    let out = d.key(R, form::Key::Enter, None, Mods::default());
    assert!(
        matches!(out, Event::NeedListing(900)),
        "the entry's own id, not one this module minted: {out:?}"
    );
    assert_eq!(d.folder(), Some(1));
}

#[test]
fn back_returns_to_the_previous_folder() {
    let mut d = sample();
    d.set_listing(folder(1, None, vec![dir("Docs")]));
    d.set_listing(folder(2, Some(1), vec![file("b.txt")]));
    assert_eq!(d.folder(), Some(2));
    match d.nav(Nav::Back) {
        Event::NeedListing(id) => assert_eq!(id, 1),
        other => panic!("expected NeedListing(1), got {other:?}"),
    }
}

#[test]
fn relisting_the_shown_folder_adds_no_back_step() {
    let mut d = sample();
    d.set_listing(folder(1, None, vec![]));
    d.set_listing(folder(2, Some(1), vec![]));
    d.set_listing(folder(2, Some(1), vec![file("b.txt")]));
    assert!(
        matches!(d.nav(Nav::Back), Event::NeedListing(1)),
        "Back goes to folder 1"
    );
    assert!(
        matches!(d.nav(Nav::Back), Event::None),
        "one Back step only"
    );
}

#[test]
fn forward_returns_to_the_folder_back_left_until_another_folder_is_entered() {
    let mut d = sample();
    d.set_listing(folder(1, None, vec![]));
    d.set_listing(folder(2, Some(1), vec![]));
    assert!(
        matches!(d.nav(Nav::Forward), Event::None),
        "nothing ahead yet"
    );
    assert!(matches!(d.nav(Nav::Back), Event::NeedListing(1)));
    d.set_listing(folder(1, None, vec![]));
    assert!(matches!(d.nav(Nav::Forward), Event::NeedListing(2)));
    d.set_listing(folder(2, Some(1), vec![]));
    assert!(
        matches!(d.nav(Nav::Back), Event::NeedListing(1)),
        "Forward keeps the Back step"
    );
    d.set_listing(folder(1, None, vec![]));
    d.set_listing(folder(3, Some(1), vec![]));
    assert!(
        matches!(d.nav(Nav::Forward), Event::None),
        "entering folder 3 drops folder 2 from Forward"
    );
}

#[test]
fn backspace_goes_up_outside_the_file_name_field() {
    let mut d = sample();
    d.set_listing(folder(2, Some(1), vec![file("b.txt")]));
    let bs = || form::Key::Edit(EditKey::Backspace { word: false });
    d.focus = Focus::Shell;
    assert!(matches!(
        d.key(R, bs(), None, Mods::default()),
        Event::NeedListing(1)
    ));
    d.focus = Focus::FileName;
    d.set_file_name("b.txt");
    assert!(matches!(
        d.key(R, bs(), None, Mods::default()),
        Event::Redraw
    ));
    assert_eq!(d.file_name(), "", "the field deletes its selected name");
}

#[test]
fn up_is_greyed_without_a_parent() {
    let mut d = sample();
    d.set_listing(folder(1, None, vec![]));
    assert!(!d.can_go_up());
    d.set_listing(folder(2, Some(1), vec![]));
    assert!(d.can_go_up());
}
