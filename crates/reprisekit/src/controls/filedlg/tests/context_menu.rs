use super::*;
use crate::textedit::keysym;

fn labels(d: &FileDlg, level: usize) -> Vec<String> {
    let c = d.context.as_ref().expect("the menu is open");
    d.context_items(c)[level]
        .0
        .iter()
        .map(|it| match it {
            crate::controls::menu::MenuItem::Entry { label, enabled, .. } => {
                format!("{label}{}", ["", " (greyed)"][usize::from(!enabled)])
            }
            _ => "-".into(),
        })
        .collect()
}

fn menu_key(d: &mut FileDlg, key: form::Key) -> Event {
    d.key(R, key, None, Mods::default())
}

#[test]
fn a_right_press_on_an_entry_selects_it_and_opens_its_menu() {
    let mut d = multi(DlgMode::Open, true);
    d.click(R, entry_hit(1), Mods::default());
    d.click(R, entry_hit(2), CTRL);
    assert!(matches!(
        d.right_click(R, entry_hit(2), (300, 120)),
        Event::Redraw
    ));
    assert_eq!(d.selected().len(), 2, "a press on the selection keeps it");
    assert_eq!(labels(&d, 0), ["Select", "-", "Delete", "Rename (greyed)"]);
    assert!(matches!(
        d.right_click(R, entry_hit(3), (300, 140)),
        Event::Redraw
    ));
    assert_eq!(
        d.file_name(),
        "c.txt",
        "a press beside it selects the entry alone"
    );
    assert_eq!(labels(&d, 0), ["Select", "-", "Delete", "Rename"]);
    match menu_key(&mut d, form::Key::Enter) {
        Event::Accept(names) => assert_eq!(names, ["c.txt"], "Enter takes the bold Select"),
        other => panic!("Select accepts the file, got {other:?}"),
    }
    assert!(d.context.is_none());
    d.right_click(R, entry_hit(0), (300, 100));
    assert!(
        matches!(
            menu_key(&mut d, form::Key::Char('s')),
            Event::NeedListing(900)
        ),
        "Select opens a folder"
    );
    d.right_click(R, entry_hit(1), (300, 100));
    assert!(matches!(
        menu_key(&mut d, form::Key::Char('m')),
        Event::Redraw
    ));
    assert!(d.shell.edit.is_some(), "Rename opens the box");
}

#[test]
fn a_lone_folder_offers_pin_or_unpin() {
    let mut d = multi(DlgMode::Open, true);
    d.right_click(R, entry_hit(0), (300, 100));
    assert_eq!(
        labels(&d, 0),
        ["Select", "-", "Pin to Places", "-", "Delete", "Rename"]
    );
    match menu_key(&mut d, form::Key::Char('i')) {
        Event::PinEntry { name, pin } => assert_eq!((name.as_str(), pin), ("sub", true)),
        other => panic!("Pin to Places pins the folder, got {other:?}"),
    }
    d.set_pinned_entries(vec!["sub".into()]);
    d.right_click(R, entry_hit(0), (300, 100));
    assert_eq!(labels(&d, 0)[2], "Unpin from Places");
    assert!(matches!(
        menu_key(&mut d, form::Key::Char('i')),
        Event::PinEntry { pin: false, .. }
    ));
    d.right_click(R, entry_hit(1), (300, 100));
    assert_eq!(labels(&d, 0), ["Select", "-", "Delete", "Rename"], "a file");
}

/// Three built-in tiles, then two pinned folders, the second missing.
fn pinned_dialog() -> FileDlg {
    let mut d = multi(DlgMode::Open, false);
    let tile = |label: &str, enabled, pinned| Place {
        label: label.into(),
        enabled,
        pinned,
        ..Place::default()
    };
    d.set_places(
        vec![
            tile("Home", true, false),
            tile("Desktop", true, false),
            tile("Computer", true, false),
            tile("Work", true, true),
            tile("Stick", false, true),
        ],
        vec![],
    );
    d
}

#[test]
fn a_pinned_tile_opens_unpins_and_moves() {
    let mut d = pinned_dialog();
    let tile = |i| Some(Hit::Places(i));
    d.right_click(R, tile(1), (20, 100));
    assert!(d.context.is_none(), "a built-in tile has no menu");
    d.right_click(R, tile(3), (20, 300));
    assert_eq!(
        labels(&d, 0),
        [
            "Open",
            "-",
            "Unpin from Places",
            "-",
            "Move Up (greyed)",
            "Move Down"
        ]
    );
    assert!(matches!(
        menu_key(&mut d, form::Key::Char('d')),
        Event::ShiftPlace { tile: 3, up: false }
    ));
    d.right_click(R, tile(4), (20, 300));
    assert_eq!(
        labels(&d, 0),
        [
            "Open (greyed)",
            "-",
            "Unpin from Places",
            "-",
            "Move Up",
            "Move Down (greyed)"
        ],
        "a missing folder can still be unpinned"
    );
    assert!(matches!(
        menu_key(&mut d, form::Key::Char('i')),
        Event::UnpinPlace(4)
    ));
    d.right_click(R, tile(3), (20, 300));
    assert!(matches!(
        menu_key(&mut d, form::Key::Enter),
        Event::NeedPlace(3)
    ));
    assert_eq!(d.places.selected, Some(3));
}

#[test]
fn pinned_tiles_never_grow_the_dialog() {
    let mut d = multi(DlgMode::Open, false);
    let built_in = Place {
        label: "Home".into(),
        enabled: true,
        ..Place::default()
    };
    d.set_places(vec![built_in.clone(); 6], vec![]);
    let min = d.min_client();
    let pinned = Place {
        pinned: true,
        ..built_in
    };
    let mut many = vec![d.places()[0].clone(); 6];
    many.extend(vec![pinned; 10]);
    d.set_places(many, vec![]);
    assert_eq!(d.min_client(), min);
    let cm = ControlMetrics::CLASSIC;
    let bar = d.places_rect(R, &cm);
    let wheel = Some(Hit::Places(0));
    assert!(matches!(d.wheel(R, wheel, WHEEL_ROWS), Event::Redraw));
    assert_eq!(d.places.top, 1, "a notch scrolls a tile");
    d.places.selected = Some(1);
    d.focus = Focus::Places;
    for _ in 0..14 {
        d.key(R, form::Key::Down, None, Mods::default());
    }
    assert_eq!(d.places.selected, Some(15));
    let shown = (bar.height / cm.places.tile) as usize;
    assert_eq!(
        d.places.top,
        16 - shown,
        "the keyboard brings the last tile into view"
    );
}

#[test]
fn the_background_menu_switches_the_view_refreshes_and_makes_a_folder() {
    let text = text();
    let mut d = multi(DlgMode::Open, false);
    d.right_click(R, Some(Hit::Shell(shellview::Hit::Background)), (200, 100));
    assert!(
        d.selected().is_empty(),
        "the background press clears the selection"
    );
    assert_eq!(labels(&d, 0), ["View", "-", "Refresh", "-", "New"]);
    let levels = d.context_levels(&text, 11, R, TextUnits::ONE);
    let root = &levels[0];
    let row = |l: &context::Level, i: usize| {
        let r = l.layout.rows[i];
        (l.rect.x + r.x + r.width / 2, l.rect.y + r.y + r.height / 2)
    };
    let (x, y) = row(root, 0);
    let over = d.hit(&text, 11, R, x, y);
    assert_eq!(
        over,
        Some(Hit::Menu {
            level: 0,
            row: Some(0)
        })
    );
    assert!(
        matches!(d.hover(over), Event::Redraw),
        "resting on View opens it"
    );
    assert_eq!(
        labels(&d, 1),
        ["Thumbnails", "Tiles", "Icons", "List", "Details"]
    );
    let levels = d.context_levels(&text, 11, R, TextUnits::ONE);
    assert!(
        levels[1].rect.x >= root.rect.right() - 6,
        "the submenu opens beside its row"
    );
    let (x, y) = row(&levels[1], 4);
    let details = d.hit(&text, 11, R, x, y);
    assert!(matches!(
        d.click(R, details, Mods::default()),
        Event::Redraw
    ));
    assert_eq!(d.shell.mode, Mode::Details);
    assert!(d.context.is_none());

    d.right_click(R, Some(Hit::Shell(shellview::Hit::Background)), (400, 300));
    assert!(matches!(
        menu_key(&mut d, form::Key::Char('e')),
        Event::Refresh
    ));
    d.right_click(R, Some(Hit::Shell(shellview::Hit::Background)), (400, 300));
    assert!(
        matches!(menu_key(&mut d, form::Key::Char('w')), Event::Redraw),
        "New opens"
    );
    assert!(
        matches!(menu_key(&mut d, form::Key::Enter), Event::CreateFolder(_)),
        "its Folder is lit"
    );

    d.right_click(R, Some(Hit::Shell(shellview::Hit::Background)), (400, 300));
    assert!(matches!(
        d.click(R, Some(Hit::Places(0)), Mods::default()),
        Event::Redraw
    ));
    assert!(
        d.context.is_none(),
        "a press elsewhere closes it and does nothing else"
    );
    assert!(d.places.selected.is_none());
}

#[test]
fn shift_f10_opens_the_menu_from_the_keyboard_and_the_arrows_walk_it() {
    let mut d = multi(DlgMode::Open, false);
    d.focus = Focus::Shell;
    d.click(R, entry_hit(2), Mods::default());
    assert!(matches!(
        d.shortcut(R, keysym::F10, false, true, false),
        Some(Event::Redraw)
    ));
    let c = d.context.clone().unwrap();
    let cm = d.drawn.0.get();
    let cell = d.shell_layout(R, &cm).cells[2];
    assert_eq!(c.at, (cell.x + 8, cell.bottom()), "under the focused entry");
    assert!(c.keys);
    assert_eq!(c.hot[0], Some(0), "Select is lit");
    menu_key(&mut d, form::Key::Down);
    menu_key(&mut d, form::Key::Down);
    assert_eq!(
        d.context.as_ref().unwrap().hot[0],
        Some(3),
        "past the separator to Rename"
    );
    assert!(matches!(menu_key(&mut d, form::Key::Escape), Event::Redraw));
    assert!(d.context.is_none());

    d.shell.selection.clear();
    d.shortcut(R, keysym::MENU, false, false, false);
    let lay = d.shell_layout(R, &cm);
    assert_eq!(
        d.context.as_ref().unwrap().at,
        (lay.body.x + 8, lay.body.y + 8)
    );
    menu_key(&mut d, form::Key::Right);
    assert_eq!(d.context.as_ref().unwrap().sub, Some(0), "Right opens View");
    menu_key(&mut d, form::Key::Escape);
    assert_eq!(
        d.context.as_ref().unwrap().sub,
        None,
        "Escape closes the submenu first"
    );
    menu_key(&mut d, form::Key::Escape);
    assert!(d.context.is_none());

    d.focus = Focus::FileName;
    assert!(
        d.shortcut(R, keysym::MENU, false, false, false).is_none(),
        "File name's is the edit menu"
    );
}

#[test]
fn a_hosted_menu_and_its_submenu_open_where_the_dialog_shows_them() {
    let text = Rc::new(text());
    let mut d = multi(DlgMode::Open, false);
    d.right_click(R, Some(Hit::Shell(shellview::Hit::Background)), (400, 300));
    d.key(R, form::Key::Down, None, Mods::default());
    d.key(R, form::Key::Right, None, Mods::default());
    let levels = d.context_levels(&text, 11, R, TextUnits::ONE);
    let mut p = Popped {
        dlg: &mut d,
        text: text.clone(),
        px: 11,
    };
    let spots = p.spots();
    assert_eq!(spots.len(), 2);
    assert_eq!(spots[0].rect, levels[0].rect);
    assert_eq!(spots[0].anchor, Anchor::Point(400, 300));
    assert!(matches!(spots[1].anchor, Anchor::Beside { .. }));
    p.host();
    p.place(0, (10, 20));
    p.place(1, (200, 20));
    let levels = d.context_levels(&text, 11, R, TextUnits::ONE);
    assert_eq!((levels[0].rect.x, levels[1].rect.x), (10, 200));
    let mut p = Popped {
        dlg: &mut d,
        text: text.clone(),
        px: 11,
    };
    p.dismiss();
    assert!(d.context.is_none());
}

#[test]
fn the_menus_and_the_rename_box_are_drawn_as_pinned() {
    crate::pins::fonts();
    let text = text();
    let mut got = Vec::new();
    for s in [1, 2] {
        let shot = |d: &FileDlg| {
            let mut c = Canvas::new((CLIENT.0 * s) as u32, (CLIENT.1 * s) as u32);
            d.draw(
                &mut c,
                &text,
                &classic_paint(&ColorScheme::default(), s),
                R,
                11,
            );
            c
        };
        let mut d = multi(DlgMode::Open, false);
        d.focus = Focus::Shell;
        d.click(R, entry_hit(1), Mods::default());
        d.shortcut(R, keysym::MENU, false, false, false);
        got.push((format!("item menu {s}"), fnv(&shot(&d))));
        d.right_click(R, Some(Hit::Shell(shellview::Hit::Background)), (300, 200));
        d.key(R, form::Key::Down, None, Mods::default());
        d.key(R, form::Key::Right, None, Mods::default());
        got.push((format!("view submenu {s}"), fnv(&shot(&d))));
        d.context = None;
        d.click(R, entry_hit(1), Mods::default());
        d.shortcut(R, keysym::F2, false, false, false);
        got.push((format!("rename box {s}"), fnv(&shot(&d))));
    }
    crate::pins::check("filedlg_menu", &got);
}

/// The boxes a host shows over the dialog: Delete's questions on the shell's delete
/// templates, and an error on a message box.
#[test]
fn the_boxes_over_the_dialog_are_drawn_as_pinned() {
    use crate::controls::dlgtemplate::{self, Template};
    use crate::controls::msgbox;
    use crate::theme::TextUnits;
    crate::pins::fonts();
    let text = text();
    let yes_no = [dlgtemplate::YES.to_string(), dlgtemplate::NO.to_string()];
    let mut icon = Canvas::new(32, 32);
    icon.fill_rect(icon.bounds(), crate::theme::Color::rgb(0xc0, 0x40, 0x20));
    let prompt = |template, question: &str, text_h| {
        let lay = dlgtemplate::layout(
            template,
            &text,
            11,
            &dlgtemplate::Fields {
                text: question,
                icon: true,
                field: false,
                buttons: &yes_no,
                text_h,
            },
            TextUnits::ONE,
        )
        .client();
        (
            lay.size,
            dlgtemplate::form(
                &lay,
                Some(icon.clone()),
                &yes_no,
                dlgtemplate::delete_default_index(&yes_no, true),
            ),
        )
    };
    let error = msgbox::Fields {
        text: "A file name cannot contain any of the following characters:\n/".into(),
        icon: Some(icon.clone()),
        buttons: vec!["OK".into()],
        default: 0,
        min_text_h: None,
        min_w: 0,
        max_w: 0,
    };
    let boxes = [
        (
            "erase file",
            prompt(
                Template::MessageBox,
                "Are you sure you want to delete 'a.txt'?",
                None,
            ),
        ),
        (
            "erase folder",
            prompt(
                Template::MessageBox,
                "Are you sure you want to remove the folder 'Reports' and all its contents?",
                Some(dlgtemplate::FOLDER_TEXT_H),
            ),
        ),
        (
            "erase several",
            prompt(
                Template::MultiDelete,
                "Are you sure you want to delete these 3 items?",
                None,
            ),
        ),
        (
            "error",
            (
                msgbox::client(&error, &text, 11, TextUnits::ONE),
                msgbox::form(&error, &text, 11, TextUnits::ONE),
            ),
        ),
    ];
    let cs = ColorScheme::default();
    let mut got = Vec::new();
    for s in [1, 2] {
        let p = classic_paint(&cs, s);
        for (name, ((w, h), f)) in &boxes {
            let mut c = Canvas::new((w * s) as u32, (h * s) as u32);
            c.fill_rect(c.bounds(), cs.button_face);
            crate::controls::form::draw(&mut c, &text, &p, f, true, 11);
            got.push((format!("{name} {s}"), fnv(&c)));
        }
    }
    crate::pins::check("filedlg_box", &got);
}

pub(super) fn fnv(c: &Canvas) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for v in [c.width, c.height]
        .into_iter()
        .chain(c.pixels.iter().copied())
    {
        for b in v.to_le_bytes() {
            h = (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3);
        }
    }
    h
}
