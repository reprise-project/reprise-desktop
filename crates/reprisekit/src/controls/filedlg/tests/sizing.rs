use super::*;

#[test]
fn set_size_keeps_the_opening_width_and_takes_the_height() {
    let mut d = with_extra();
    let min = d.min_client();
    assert_eq!(d.client(), min);
    d.set_size((100, 100));
    assert_eq!(d.client(), (min.0, 100));
    d.set_size((min.0 + 168, min.1 + 55));
    assert_eq!(d.client(), (min.0 + 168, min.1 + 55));
}

/// At 200% with six places and the extra rows, a window held 150 px shorter than the
/// minimum: the shell view and places bar give the height, the rows under them stay inside.
#[test]
fn a_window_shorter_than_the_minimum_keeps_every_row_inside_it() {
    let t = text();
    let u = TextUnits::new(22, 11);
    let cm = ControlMetrics::CLASSIC.grown(u);
    let mut d = with_extra();
    d.places.places = (0..6)
        .map(|i| Place {
            label: format!("Place {i}"),
            icon: None,
            enabled: true,
            pinned: false,
        })
        .collect();
    d.set_text(u);
    let cs = ColorScheme::default();
    d.drawn_with(&Paint {
        text: u,
        ..classic_paint(&cs, 1)
    });
    let (w, h) = d.min_client();
    let base = d.layout(&t, 22, Rect::new(0, 0, w, h), &cm);
    let short = Rect::new(0, 0, w, h - 150);
    d.set_size((w, h - 150));
    assert_eq!(d.client(), (w, h - 150));
    let lay = d.layout(&t, 22, short, &cm);
    assert_eq!(lay.shell.frame.height, base.shell.frame.height - 150);
    assert_eq!(lay.places.height, base.places.height - 150);
    assert_eq!(lay.filetype.field.y, base.filetype.field.y - 150);
    assert_eq!(lay.cancel.y, base.cancel.y - 150);
    let hit = d.hit(&t, 22, short, 2 * FIELD_X + 5, 2 * (EXTRA_Y + 20) - 150 + 5);
    assert!(
        matches!(hit, Some(Hit::Extra(h)) if h.item == 1),
        "got {hit:?}"
    );

    let tiles = cm.places.least_tiles(6);
    assert_eq!(tiles, 6 * (3 + 32 + 2 * 30), "tiles with 32 px icons");
    let lay = d.layout(&t, 22, Rect::new(0, 0, w, 100), &cm);
    assert_eq!(lay.places.height, tiles, "the places bar keeps its tiles");
    let squeezed = placesbar::layout(&d.places, lay.places, &cm);
    assert_eq!(squeezed[5].bottom(), lay.places.bottom(), "all six fit");
    assert_eq!(
        lay.shell.frame.height,
        base.shell.frame.height - (base.places.height - tiles)
    );
}

/// Grown by (168, 55): the places bar takes the height, the shell view
/// both, Look in and the fields the width; the toolbar and buttons keep
/// to the right edge, the rows under the shell view to the bottom.
#[test]
fn a_grown_dialog_stretches_the_view_and_keeps_its_rows_to_the_corner() {
    let (dw, dh) = (168, 55);
    let t = text();
    let cm = ControlMetrics::CLASSIC;
    let mut d = with_extra();
    let (w, h) = d.client();
    let base = d.layout(&t, 11, Rect::new(0, 0, w, h), &cm);
    d.set_size((w + dw, h + dh));
    let big = Rect::new(0, 0, w + dw, h + dh);
    let lay = d.layout(&t, 11, big, &cm);
    let moved = |r: Rect, x: i32, y: i32| Rect::new(r.x + x, r.y + y, r.width, r.height);
    let grown = |r: Rect, x: i32, y: i32| Rect::new(r.x, r.y, r.width + x, r.height + y);
    assert_eq!(lay.places, grown(base.places, 0, dh), "places bar");
    assert_eq!(
        lay.shell.frame,
        grown(base.shell.frame, dw, dh),
        "shell view"
    );
    assert_eq!(lay.lookin.field, grown(base.lookin.field, dw, 0), "Look in");
    assert_eq!(lay.toolbar, moved(base.toolbar, dw, 0), "toolbar");
    assert_eq!(
        lay.filename,
        moved(grown(base.filename, dw, 0), 0, dh),
        "File name"
    );
    assert_eq!(
        lay.filetype.field,
        moved(grown(base.filetype.field, dw, 0), 0, dh),
        "Files of type"
    );
    assert_eq!(lay.accept, moved(base.accept, dw, dh), "accept button");
    assert_eq!(lay.cancel, moved(base.cancel, dw, dh), "Cancel");
    let hit = d.hit(&t, 11, big, FIELD_X + 5, EXTRA_Y + dh + 5);
    assert!(matches!(hit, Some(Hit::Extra(_))), "got {hit:?}");
}

/// At 200% text the client opens at twice its 100% minimum and every row, field and button
/// grows from the client's corner, the toolbar's buttons and the places bar's tiles too; a
/// host's extra rows draw and hit at their grown place.
#[test]
fn a_grown_text_size_grows_the_dialog() {
    let t = text();
    let u = TextUnits::new(22, 11);
    let mut d = with_extra();
    d.set_text(u);
    assert_eq!(d.min_client(), (2 * CLIENT.0, 2 * (CLIENT.1 + EXTRA_H)));
    assert_eq!(d.client(), d.min_client());
    let (w, h) = d.client();
    let r = Rect::new(0, 0, w, h);
    let cs = ColorScheme::default();
    let p = Paint {
        text: u,
        ..classic_paint(&cs, 1)
    };
    let mut c = Canvas::new(w as u32, h as u32);
    d.draw(&mut c, &t, &p, r, 22);
    let lay = d.layout(&t, 22, r, &ControlMetrics::CLASSIC.grown(u));
    assert_eq!(
        lay.lookin.field,
        Rect::new(198, 16, lay.lookin.field.width, 44)
    );
    assert_eq!(
        lay.toolbar,
        Rect::new(lay.toolbar.x, 16, lay.toolbar.width, 44)
    );
    assert_eq!(
        lay.toolbar_rects[0].width,
        2 * toolbar::BUTTON,
        "a toolbar button doubles"
    );
    let pm = ControlMetrics::CLASSIC.grown(u).places;
    assert_eq!(
        (pm.tile, pm.icon),
        (3 + 2 * 62, 64),
        "a places tile and its icon"
    );
    assert_eq!(
        lay.places,
        Rect::new(12, 74, 2 * placesbar::WIDTH, 2 * PLACES_H)
    );
    assert_eq!(lay.shell.frame.x, 2 * SHELL_X);
    assert_eq!(
        lay.filename,
        Rect::new(390, 788, w - 12 - 150 - 66 - 390, 44)
    );
    assert_eq!(lay.accept, Rect::new(w - 12 - 150, 788, 150, 46));
    assert_eq!(lay.cancel.y, 2 * FILETYPE_Y + CANCEL_DY);
    let (bx, by) = (2 * FIELD_X, 2 * EXTRA_Y);
    // The 13 px box centred down its 26 px row.
    assert_eq!(
        c.get(bx, by + 6),
        cs.button_shadow.argb(),
        "the extra check box's corner"
    );
    let hit = d.hit(&t, 22, r, bx + 5, by + 6 + 5);
    assert!(
        matches!(hit, Some(Hit::Extra(h)) if h.item == 0),
        "got {hit:?}"
    );
    let hit = d.hit(&t, 22, r, bx + 5, 2 * (EXTRA_Y + 20) + 5);
    assert!(
        matches!(hit, Some(Hit::Extra(h)) if h.item == 1),
        "got {hit:?}"
    );
}

#[test]
fn a_grown_dialog_draws_its_corner_unchanged() {
    let (dw, dh) = (168, 55);
    let t = text();
    let cs = ColorScheme::default();
    let p = classic_paint(&cs, 1);
    let draw = |d: &FileDlg| {
        let (w, h) = d.client();
        let mut c = Canvas::new(w as u32, h as u32);
        d.draw(&mut c, &t, &p, Rect::new(0, 0, w, h), 11);
        c
    };
    let mut d = multi(DlgMode::Open, false);
    let small = draw(&d);
    d.set_size((CLIENT.0 + dw, CLIENT.1 + dh));
    let big = draw(&d);
    // From 20 px left of the accept button: past the fields' ends.
    let (x0, y0) = (CLIENT.0 - MARGIN - BUTTON_W - 20, FILENAME_Y - 4);
    for y in y0..CLIENT.1 {
        for x in x0..CLIENT.0 {
            assert_eq!(big.get(x + dw, y + dh), small.get(x, y), "at ({x}, {y})");
        }
    }
}

/// A press on the grip asks the host to resize; Cancel's right end,
/// over the grip's square, stays Cancel's.
#[test]
fn the_size_grip_follows_the_corner_and_asks_for_a_resize() {
    let t = text();
    let mut d = sample();
    assert_eq!(
        d.hit(&t, 11, R, CLIENT.0 - 4, CLIENT.1 - 4),
        Some(Hit::Grip)
    );
    assert_eq!(
        d.hit(&t, 11, R, CLIENT.0 - 10, CLIENT.1 - 10),
        Some(Hit::Cancel)
    );
    d.set_size((CLIENT.0 + 100, CLIENT.1 + 100));
    let big = Rect::new(0, 0, CLIENT.0 + 100, CLIENT.1 + 100);
    let hit = d.hit(&t, 11, big, big.right() - 4, big.bottom() - 4);
    assert_eq!(hit, Some(Hit::Grip));
    assert!(matches!(d.click(big, hit, Mods::default()), Event::Resize));
}

#[test]
fn choice_combo_list_fits_the_grown_dialog() {
    let d = with_extra();
    let (_, h) = d.client();
    let it = d.extra().unwrap().item("extra-combo").unwrap();
    let form::Control::Combo(c) = &it.control else {
        panic!("combo")
    };
    assert!(
        combo::layout(c, it.rect, &ControlMetrics::CLASSIC)
            .list
            .bottom()
            <= h
    );
}

/// At 200% the Look in field's 32 px folder icon sits whole 12 px in and 6 px down, and the
/// folder's name starts past it.
#[test]
fn at_200_percent_the_look_in_icon_grows() {
    let t = text();
    let u = TextUnits::new(22, 11);
    let green = crate::theme::Color::rgb(0x20, 0x80, 0x20);
    let mut d = sample();
    let mut f = folder(1, None, vec![]);
    let mut icon = Canvas::new(32, 32);
    icon.fill_rect(Rect::new(0, 0, 32, 32), green);
    f.crumbs[0].icon = Some(icon);
    d.set_listing(f);
    d.set_text(u);
    let (w, h) = d.client();
    let r = Rect::new(0, 0, w, h);
    let cs = ColorScheme::default();
    let p = Paint {
        text: u,
        ..classic_paint(&cs, 1)
    };
    let mut c = Canvas::new(w as u32, h as u32);
    d.draw(&mut c, &t, &p, r, 22);
    let lay = d.layout(&t, 22, r, &ControlMetrics::CLASSIC.grown(u));
    let field = lay.lookin.field;
    let (x, y) = (field.x + 12, field.y + 6);
    for (px, py) in [(x, y), (x + 31, y + 31)] {
        assert_eq!(c.get(px, py), green.argb(), "icon corner ({px}, {py})");
    }
    assert_ne!(c.get(x + 32, y + 16), green.argb(), "past the icon");
    assert_ne!(c.get(x + 16, y + 32), green.argb(), "under the icon");
    let mut bare = sample();
    bare.set_listing(folder(1, None, vec![]));
    bare.set_text(u);
    let plain = bare.layout(&t, 22, r, &ControlMetrics::CLASSIC.grown(u));
    assert_eq!(
        lay.lookin.text.x - plain.lookin.text.x,
        48,
        "the name moves past the icon and its 6 px gap"
    );
}

#[test]
fn a_focused_glass_look_in_rings_right_of_its_icon() {
    use crate::interface::Interface;
    let text = text();
    let theme = crate::interfaces::theme_for(Interface::Glass);
    let mut d = sample();
    let mut f = folder(1, None, vec![]);
    let mut icon = Canvas::new(16, 16);
    icon.fill_rect(
        Rect::new(0, 0, 16, 16),
        crate::theme::Color::rgb(0x20, 0x80, 0x20),
    );
    f.crumbs[0].icon = Some(icon);
    d.set_listing(f);
    let draw = |d: &FileDlg| {
        let mut c = Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32);
        d.draw(&mut c, &text, &theme.paint(), R, 11);
        c
    };
    let plain = draw(&d);
    d.key(R, form::Key::Tab { back: true }, None, Mods::default());
    assert_eq!(d.focus, Focus::LookIn);
    let focused = draw(&d);
    let field = d.layout(&text, 11, R, &theme.look.controls()).lookin.field;
    let changed: Vec<i32> = (field.y..field.bottom())
        .flat_map(|y| (field.x..field.right()).map(move |x| (x, y)))
        .filter(|&(x, y)| plain.get(x, y) != focused.get(x, y))
        .map(|(x, _)| x)
        .collect();
    let left = changed.iter().copied().min().expect("a ring");
    let icon_right = field.x + LOOKIN_ICON_X + LOOKIN_ICON_PX;
    assert!(
        left > icon_right,
        "the ring starts at {left}, right of the icon"
    );
}

/// Under Blue every row sits a pixel higher than Classic's; the columns
/// stay put and the places bar's tiles are 71 px.
#[test]
fn blues_rows_sit_a_pixel_higher() {
    let text = text();
    let mut d = multi(DlgMode::Open, false);
    let place = |l: &str| Place {
        label: l.into(),
        icon: None,
        enabled: true,
        pinned: false,
    };
    d.places.places = vec![place("Desktop"), place("My Computer")];
    let blue = crate::interfaces::blue_theme(crate::blue::Variant::Blue);
    let c = d.layout(&text, 11, R, &ControlMetrics::CLASSIC);
    let b = d.layout(&text, 11, R, &blue.look.controls());
    let up = |r: Rect| Rect::new(r.x, r.y - 1, r.width, r.height);
    assert_eq!(
        b.lookin.field,
        Rect::new(R.x + LOOKIN_X, 7, c.lookin.field.width, 22)
    );
    assert_eq!(b.lookin_label, up(c.lookin_label));
    assert_eq!(b.toolbar, up(c.toolbar));
    assert_eq!(b.places, Rect::new(MARGIN, 36, placesbar::WIDTH, PLACES_H));
    assert_eq!(
        b.places_rects[1],
        Rect::new(MARGIN, 36 + 71, placesbar::WIDTH, 71)
    );
    assert_eq!(b.shell.frame, up(c.shell.frame));
    assert_eq!(b.filename, up(c.filename));
    assert_eq!(b.filename_label, up(c.filename_label));
    assert_eq!(b.filetype.field, up(c.filetype.field));
    assert_eq!(b.filetype_label, up(c.filetype_label));
    assert_eq!(b.accept, up(c.accept));
    assert_eq!(b.cancel, up(c.cancel));
}

/// A folder `label` names, files of `filters` and an `accept` button, in `mode`.
fn long_inputs(mode: DlgMode, accept: &str, label: &str, filters: &[&str]) -> FileDlg {
    let places = [
        "My Recent Documents",
        "Alexandra-Katharina's Home",
        "Desktop",
        "Documents",
        "Downloads",
        "Computer",
    ];
    let mut d = FileDlg::new(
        Fields {
            title: accept.into(),
            accept_label: accept.into(),
            filters: filters.iter().map(|f| f.to_string()).collect(),
            mode,
            multiple: false,
        },
        places
            .iter()
            .map(|l| Place {
                label: l.to_string(),
                icon: None,
                enabled: true,
                pinned: false,
            })
            .collect(),
    );
    let mut f = folder(1, Some(0), vec![dir("Drafts"), file("notes.txt")]);
    f.crumbs[0].label = label.into();
    d.set_listing(f);
    d
}

#[test]
fn every_file_dialog_fits_its_text_under_every_interface() {
    let folder = "Quarterly Reports for the Northern Region (final drafts, 2026)";
    let filters = [
        "Images (*.png, *.jpg, *.jpeg, *.gif, *.bmp, *.webp, *.svg)",
        "OpenDocument Text and Word Processor Documents (*.odt;*.docx)",
        "All Files (*.*)",
    ];
    let over = crate::interfaces::overflows(|_, theme, text| {
        let mut save = long_inputs(DlgMode::Save, "&Save", folder, &filters);
        save.set_file_name(
            "Quarterly report for the northern region, second draft with comments.odt",
        );
        let dialogs = [
            (
                "Open",
                long_inputs(DlgMode::Open, "&Open", folder, &filters),
            ),
            ("Save", save),
            (
                "Folder",
                long_inputs(DlgMode::Folder, "Select Folder", folder, &[]),
            ),
        ];
        let mut over = Vec::new();
        for (name, mut d) in dialogs {
            d.set_text(theme.text);
            d.drawn_with(&theme.paint());
            for (part, by) in d.overflows(text, theme.fonts.size_px) {
                over.push((name.to_string(), part, by));
            }
        }
        over
    });
    assert!(over.is_empty(), "{}", over.join("\n"));
}

/// The open lists widen to their widest row as far as the right margin; the fields stay.
#[test]
fn the_lists_widen_to_their_widest_row_up_to_the_right_margin() {
    let t = text();
    let cm = ControlMetrics::CLASSIC;
    let images = "Images (*.png, *.jpg, *.jpeg, *.gif, *.bmp, *.webp, *.svg)";
    let d = long_inputs(
        DlgMode::Open,
        "Open",
        "Reports",
        &[images, "All Files (*.*)"],
    );
    let lay = d.layout(&t, 11, R, &cm);
    let list = lay.filetype.list;
    assert_eq!(lay.filetype.field.width, field_w(CLIENT.0));
    assert_eq!(list.x, FIELD_X);
    assert_eq!(list.width, 1 + 2 + t.measure(images, false, 11) + 3);
    assert!(list.right() < CLIENT.0 - MARGIN);
    assert_eq!(
        lay.lookin.list.width, lay.lookin.field.width,
        "a short folder name"
    );

    let longer = format!("{images} {images}");
    let d = long_inputs(DlgMode::Open, "Open", &longer, &[&longer]);
    let lay = d.layout(&t, 11, R, &cm);
    assert_eq!(lay.filetype.list.right(), CLIENT.0 - MARGIN);
    assert_eq!(lay.lookin.list.right(), CLIENT.0 - MARGIN);
}

/// Open and Cancel share their width: 75 px, or what a longer label needs.
#[test]
fn a_long_accept_label_widens_both_buttons_and_narrows_the_fields() {
    let t = text();
    let cm = ControlMetrics::CLASSIC;
    let lay = sample().layout(&t, 11, R, &cm);
    assert_eq!((lay.accept.width, lay.cancel.width), (BUTTON_W, BUTTON_W));
    let label = "Import Selected Layers";
    let d = long_inputs(DlgMode::Open, label, "Reports", &["All Files (*.*)"]);
    let lay = d.layout(&t, 11, R, &cm);
    let w = t.measure(label, false, 11) + 6;
    assert!(w > BUTTON_W);
    assert_eq!((lay.accept.width, lay.cancel.width), (w, w));
    assert_eq!(lay.accept.right(), CLIENT.0 - MARGIN);
    assert_eq!(lay.filename.right(), lay.accept.x - FIELD_BUTTON_GAP);
}
