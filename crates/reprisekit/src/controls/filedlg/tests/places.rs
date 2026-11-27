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

/// Six built-in tiles, then two pinned folders, the second missing, over folder 1.
fn bookmarked() -> FileDlg {
    let tile = |label: &str, enabled, pinned| Place {
        label: label.into(),
        enabled,
        pinned,
        ..Place::default()
    };
    let mut d = multi(DlgMode::Open, false);
    let built_in = [
        "Recent",
        "Home",
        "Desktop",
        "Documents",
        "Downloads",
        "Computer",
    ];
    let mut places: Vec<Place> = (built_in.iter()).map(|l| tile(l, true, false)).collect();
    places.extend([tile("Work", true, true), tile("Stick", false, true)]);
    let mut folders: Vec<Option<FolderId>> = (10..16).map(Some).collect();
    folders.extend([Some(20), None]);
    d.set_places(places, folders);
    d
}

fn lookin_rows(d: &FileDlg) -> Vec<(&str, bool)> {
    (d.lookin.items.iter())
        .map(|it| (it.text.as_str(), it.greyed))
        .collect()
}

#[test]
fn look_in_lists_the_pinned_folders_under_the_shown_one() {
    let mut d = bookmarked();
    assert_eq!(
        lookin_rows(&d),
        [("Folder", false), ("Work", false), ("Stick", true)]
    );
    assert_eq!(d.lookin.selected, 0, "the shown folder stays picked");
    d.click(R, Some(Hit::LookIn(combo::Hit::Drop)), Mods::default());
    let ev = d.click(R, Some(Hit::LookIn(combo::Hit::Row(1))), Mods::default());
    assert!(matches!(ev, Event::NeedListing(20)), "got {ev:?}");

    d.set_listing(folder(20, None, vec![]));
    assert_eq!(
        lookin_rows(&d).len(),
        3,
        "a new listing keeps the pinned rows"
    );
    d.click(R, Some(Hit::LookIn(combo::Hit::Drop)), Mods::default());
    let ev = d.click(R, Some(Hit::LookIn(combo::Hit::Row(2))), Mods::default());
    assert!(
        !matches!(ev, Event::NeedListing(_)),
        "a missing folder lists nothing, got {ev:?}"
    );

    d.set_places(vec![], vec![]);
    assert_eq!(
        lookin_rows(&d),
        [("Folder", false)],
        "unpinned, the rows go"
    );
}

#[test]
fn look_in_picks_the_shown_folders_crumb_and_crumbs_without_an_icon_wear_the_folders() {
    let mut d = bookmarked();
    let icon = |w: u32| Some(Canvas::new(w, w));
    let crumb = |id, label: &str, depth, icon| Crumb {
        id,
        label: label.into(),
        depth,
        icon,
    };
    let mut f = folder(31, Some(30), vec![]);
    f.crumbs = vec![
        crumb(30, "Home", 0, icon(3)),
        crumb(31, "Letters", 1, None),
        crumb(40, "Computer", 0, icon(4)),
    ];
    d.set_listing(f);
    d.set_folder_icon(icon(2));
    let rows: Vec<_> = (d.lookin.items.iter())
        .map(|it| {
            (
                it.text.as_str(),
                it.indent,
                it.icon.as_ref().map(|c| c.width),
            )
        })
        .collect();
    assert_eq!(
        rows,
        [
            ("Home", 0, Some(3)),
            ("Letters", 1, Some(2)),
            ("Computer", 0, Some(4)),
            ("Work", 0, Some(2)),
            ("Stick", 0, Some(2)),
        ]
    );
    assert_eq!(d.lookin.selected, 1, "the shown folder is picked");

    d.set_crumb_icons(vec![icon(5), None, icon(6)]);
    let widths: Vec<_> = (d.lookin.items.iter().take(3))
        .map(|it| it.icon.as_ref().map(|c| c.width))
        .collect();
    assert_eq!(widths, [Some(5), Some(2), Some(6)]);

    d.click(R, Some(Hit::LookIn(combo::Hit::Drop)), Mods::default());
    let ev = d.click(R, Some(Hit::LookIn(combo::Hit::Row(2))), Mods::default());
    assert!(matches!(ev, Event::NeedListing(40)), "got {ev:?}");

    let mut f = folder(40, None, vec![]);
    f.crumbs = vec![
        crumb(30, "Home", 0, None),
        crumb(40, "Computer", 0, None),
        crumb(40, "Local Disk (/)", 1, None),
    ];
    d.set_listing(f);
    assert_eq!(
        d.lookin.selected, 2,
        "the deepest row naming the folder is picked: / is its drive's row"
    );
}

/// The dialog with two pinned folders, the second missing: the places bar scrolled to them and
/// Look in open on them, under each interface at 1x and 2x.
#[test]
fn the_pinned_folders_are_drawn_as_pinned_in_every_interface() {
    use crate::interface::Interface;
    use crate::theme::Color;
    crate::pins::fonts();
    let mut got = Vec::new();
    let looks = [Interface::Classic, Interface::Blue, Interface::Glass];
    for (i, s) in looks.into_iter().flat_map(|i| [(i, 1), (i, 2)]) {
        let theme = crate::interfaces::theme_for(i).scaled(s);
        let text = TextRenderer::load(&theme.fonts);
        let p = theme.paint();
        let solid = |side: i32, colour| {
            let mut c = Canvas::new((side * s) as u32, (side * s) as u32);
            c.fill_rect(c.bounds(), colour);
            c
        };
        let mut d = bookmarked();
        let tile_icon = solid(32, Color::rgb(0x40, 0x80, 0xC0));
        d.set_place_icons(vec![Some(tile_icon); 8]);
        d.set_folder_icon(Some(solid(16, Color::rgb(0xE8, 0xC0, 0x50))));
        d.places.top = 8;
        d.click(R, Some(Hit::LookIn(combo::Hit::Drop)), Mods::default());
        let mut c = Canvas::new((CLIENT.0 * s) as u32, (CLIENT.1 * s) as u32);
        d.draw(&mut c, &text, &p, R, theme.fonts.size_px);
        got.push((format!("{i:?} @{s}"), crate::pins::fnv(&c)));
    }
    crate::pins::check("filedlg_pinned_folders", &got);
}

/// Writes target/filedlg-pinned-preview.png: [`bookmarked`] with Look in open under Classic, Blue
/// and Glass.
#[test]
#[ignore]
fn render_pinned_folders() {
    use crate::interface::Interface;
    let looks = [Interface::Classic, Interface::Blue, Interface::Glass];
    let mut out = Canvas::new((3 * CLIENT.0 + 20) as u32, CLIENT.1 as u32);
    for (k, i) in looks.into_iter().enumerate() {
        let theme = crate::interfaces::theme_for(i);
        let mut d = bookmarked();
        d.places.top = 8;
        d.click(R, Some(Hit::LookIn(combo::Hit::Drop)), Mods::default());
        let mut c = Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32);
        let text = TextRenderer::load(&theme.fonts);
        d.draw(&mut c, &text, &theme.paint(), R, theme.fonts.size_px);
        out.blit_canvas(k as i32 * (CLIENT.0 + 10), 0, &c);
    }
    crate::raster::write_preview(&out, "filedlg-pinned-preview.png");
}
