use super::*;

/// A disabled tile is drawn (glyph kept, label greyed) but has nowhere to navigate.
#[test]
fn a_disabled_place_refuses_selection_and_activation() {
    let mut d = FileDlg::new(
        Fields {
            title: "Open".into(),
            accept_label: "Open".into(),
            mode: DlgMode::Open,
            multiple: false,
            filters: vec!["All Files (*.*)".into()],
        },
        vec![Place {
            label: "My Network Places".into(),
            icon: None,
            enabled: false,
            pinned: false,
        }],
    );
    assert!(matches!(
        d.click(R, Some(Hit::Places(0)), Mods::default()),
        Event::None
    ));
    assert!(
        d.places.selected.is_none(),
        "a greyed tile does not take the selection"
    );
    d.focus = Focus::Places;
    d.places.selected = Some(0);
    assert!(
        matches!(
            d.key(R, form::Key::Enter, None, Mods::default()),
            Event::None
        ),
        "and refuses activation even if something selected it"
    );
}

#[test]
fn a_places_tile_asks_for_its_place() {
    let mut d = sample();
    let out = d.click(R, Some(Hit::Places(0)), Mods::default());
    assert!(matches!(out, Event::NeedPlace(0)), "got {out:?}");
    let out = d.key(R, form::Key::Enter, None, Mods::default());
    assert!(matches!(out, Event::NeedPlace(0)), "got {out:?}");
}

/// Under a look that lights the pointer a place lights under it and shows held until the
/// button comes up; Classic asks for no redraw.
#[test]
fn a_place_lights_under_the_pointer_and_shows_held() {
    use crate::interface::Interface;
    for (i, lit) in [(Interface::Glass, true), (Interface::Classic, false)] {
        let mut d = sample();
        d.drawn_with(&crate::interfaces::theme_for(i).paint());
        let redraw = |e: Event| matches!(e, Event::Redraw);
        assert_eq!(redraw(d.hover(Some(Hit::Places(0)))), lit, "{i:?} lights");
        assert_eq!(d.places.hot, Some(0));
        assert!(!redraw(d.hover(Some(Hit::Places(0)))), "{i:?} stays lit");
        d.click(R, Some(Hit::Places(0)), Mods::default());
        assert_eq!(d.places.pressed, Some(0), "{i:?} held");
        assert_eq!(redraw(d.release()), lit, "{i:?} let go");
        assert_eq!(d.places.pressed, None);
        assert_eq!(redraw(d.leave()), lit, "{i:?} left");
        assert_eq!(d.places.hot, None);
    }
}

#[test]
fn arrow_keys_skip_a_disabled_place() {
    let place = |enabled| Place {
        label: "Place".into(),
        icon: None,
        enabled,
        pinned: false,
    };
    let mut d = FileDlg::new(
        Fields {
            title: "Open".into(),
            accept_label: "Open".into(),
            mode: DlgMode::Open,
            multiple: false,
            filters: vec!["All Files (*.*)".into()],
        },
        vec![place(false), place(true), place(false), place(true)],
    );
    d.focus = Focus::Places;
    let key = |d: &mut FileDlg, k| {
        d.key(R, k, None, Mods::default());
        d.places.selected
    };
    assert_eq!(key(&mut d, form::Key::Down), Some(1));
    assert_eq!(key(&mut d, form::Key::Down), Some(3));
    assert_eq!(key(&mut d, form::Key::Down), Some(3));
    assert_eq!(key(&mut d, form::Key::Up), Some(1));
    assert_eq!(key(&mut d, form::Key::Up), Some(1));
}

/// The lit tile is the shown folder's, whether a tile, Back, Up or a typed path reached it.
#[test]
fn the_places_bar_lights_the_tile_of_the_folder_shown() {
    const RECENT: FolderId = 10;
    const DESKTOP: FolderId = 11;
    const OTHER: FolderId = 12;
    let place = |label: &str| Place {
        label: label.into(),
        icon: None,
        enabled: true,
        pinned: false,
    };
    let mut d = FileDlg::new(
        Fields {
            title: "Open".into(),
            accept_label: "Open".into(),
            mode: DlgMode::Open,
            multiple: false,
            filters: vec!["All Files (*.*)".into()],
        },
        vec![place("My Recent Documents"), place("Desktop")],
    );
    d.set_listing(folder(DESKTOP, Some(OTHER), vec![]));
    d.set_place_folders(vec![Some(RECENT), Some(DESKTOP)]);
    assert_eq!(d.places.selected, Some(1), "opening on Desktop lights it");

    assert!(matches!(
        d.click(R, Some(Hit::Places(0)), Mods::default()),
        Event::NeedPlace(0)
    ));
    d.set_listing(folder(RECENT, None, vec![]));
    assert_eq!(d.places.selected, Some(0));

    assert!(matches!(d.nav(Nav::Back), Event::NeedListing(DESKTOP)));
    d.set_listing(folder(DESKTOP, Some(OTHER), vec![]));
    assert_eq!(d.places.selected, Some(1), "Back to Desktop lights Desktop");

    d.set_listing(folder(OTHER, None, vec![]));
    assert_eq!(
        d.places.selected, None,
        "a folder no tile names lights none"
    );
}

#[test]
fn down_on_an_empty_places_bar_does_not_panic() {
    let mut d = FileDlg::new(
        Fields {
            title: "Open".into(),
            accept_label: "Open".into(),
            mode: DlgMode::Open,
            multiple: false,
            filters: vec!["All Files (*.*)".into()],
        },
        Vec::new(),
    );
    d.focus = Focus::Places;
    d.places.selected = Some(0);
    let _ = d.key(R, form::Key::Down, None, Mods::default());
}
