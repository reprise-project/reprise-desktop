use super::*;
use crate::classic::widgets::{self, BarHit};
use crate::controls::hosted::{Anchor, Hosted, Spot};
use crate::controls::shellview::Field;
use crate::look::{Paint, classic_paint};
use crate::pointer::WHEEL_ROWS;
use crate::text::TextRenderer;
use crate::textedit::EditKey;
use crate::theme::{ColorScheme, Theme};
use std::rc::Rc;

const R: Rect = Rect::new(0, 0, CLIENT.0, CLIENT.1);

fn text() -> TextRenderer {
    TextRenderer::load(&crate::theme::FontConfig::default())
}

fn sample() -> FileDlg {
    FileDlg::new(
        Fields {
            title: "Open".into(),
            accept_label: "Open".into(),
            filters: vec!["All Files (*.*)".into()],
            mode: DlgMode::Open,
            multiple: false,
        },
        vec![Place {
            label: "My Computer".into(),
            icon: None,
            enabled: true,
        }],
    )
}

#[test]
fn alt_letters_focus_the_fields_and_press_the_accept_button() {
    let mut d = sample();
    let alt = |d: &mut FileDlg, c| d.key(R, form::Key::Access(c), None, Mods::default());
    assert!(matches!(alt(&mut d, 'i'), Event::Redraw));
    assert_eq!(d.focus, Focus::LookIn);
    alt(&mut d, 'T');
    assert_eq!(d.focus, Focus::FileType);
    d.filetype.open = Some(0);
    alt(&mut d, 'n');
    assert_eq!(d.focus, Focus::FileName);
    assert_eq!(d.filetype.open, None, "a key closes the dropped list");
    assert!(matches!(alt(&mut d, 'c'), Event::None), "Cancel has no key");
    d.filename.text = "a.txt".into();
    match alt(&mut d, 'o') {
        Event::Accept(names) => assert_eq!(names, ["a.txt"]),
        other => panic!("Alt+O accepts, got {other:?}"),
    }
    assert_eq!(d.focus, Focus::Accept);
}

#[test]
fn the_accept_button_takes_a_key_the_labels_leave_free() {
    let with = |label: &str| {
        let mut d = sample();
        d.fields.accept_label = label.into();
        d.accept_label().into_owned()
    };
    assert_eq!(with("Open"), "&Open");
    assert_eq!(with("Save"), "&Save");
    assert_eq!(with("Import"), "I&mport", "i is Look in's");
    assert_eq!(with("E&xport"), "E&xport", "the caller's own key stays");
}

#[test]
fn alt_underlines_the_labels_and_the_accept_button() {
    let d = sample();
    let t = text();
    let theme = Theme::default();
    let cs = &theme.colors;
    let shot = |keys: bool| {
        let mut c = Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32);
        let p = Paint {
            access_keys: keys,
            ..classic_paint(cs, 1)
        };
        d.draw(&mut c, &t, &p, R, 11);
        c
    };
    let (off, on) = (shot(false), shot(true));
    let lay = d.layout(&t, 11, R, &d.drawn.0.get());
    let ink = cs.button_text.argb();
    // Each label's key is underlined one px under the baseline, and only with Alt.
    for (r, label) in [
        (lay.lookin_label, LOOKIN_LABEL),
        (lay.filename_label, FILENAME_LABEL),
        (lay.filetype_label, FILETYPE_LABEL),
    ] {
        let (plain, m) = split_mnemonic(label);
        let base = t.baseline(r, false, 11).unwrap();
        let (dx, w) = t.span(&plain, m.unwrap(), false, 11);
        for x in r.x + dx..r.x + dx + w {
            assert_eq!(on.get(x, base + 1), ink, "{label}: underlined at x {x}");
            assert_ne!(off.get(x, base + 1), ink, "{label}: bare at x {x}");
        }
    }
    let differs = |r: Rect| {
        (r.y..r.bottom()).any(|y| (r.x..r.right()).any(|x| on.get(x, y) != off.get(x, y)))
    };
    assert!(differs(lay.accept), "Open's O is underlined");
    assert!(!differs(lay.cancel), "Cancel has no key");
    assert!(!differs(lay.shell.frame), "the view is untouched");
}

#[test]
fn a_type_label_carries_its_pattern_in_parentheses() {
    assert_eq!(type_pattern("Text Documents (*.txt)"), Some("*.txt"));
    assert_eq!(type_pattern("All Files (*.*)"), Some("*.*"));
    assert_eq!(
        type_pattern("Background Files (*.bmp;*.png) "),
        Some("*.bmp;*.png")
    );
    assert_eq!(type_pattern("Programs"), None);
    assert_eq!(
        type_pattern("Notes (draft)"),
        None,
        "no wildcard, no pattern"
    );
}

#[test]
fn a_pattern_keeps_names_any_of_its_globs_match() {
    assert!(pattern_keeps("*.txt", "notes.TXT"));
    assert!(!pattern_keeps("*.txt", "bash"));
    assert!(pattern_keeps("*.txt; *.log", "boot.log"));
    assert!(
        pattern_keeps("*.*", "bash"),
        "*.* keeps a name without a dot"
    );
    assert!(pattern_keeps("*", "bash"));
    assert!(pattern_keeps("b?sh", "bash"));
    assert!(!pattern_keeps("", "bash"));
}

#[test]
fn the_picked_type_gives_its_pattern() {
    let mut d = FileDlg::new(
        Fields {
            title: "Open".into(),
            accept_label: "Open".into(),
            filters: vec!["Text Documents (*.txt)".into(), "All Files (*.*)".into()],
            mode: DlgMode::Open,
            multiple: false,
        },
        vec![],
    );
    assert_eq!(d.picked_pattern().as_deref(), Some("*.txt"));
    d.set_filter(1);
    assert_eq!(d.picked_pattern().as_deref(), Some("*.*"));
}

/// The Views popup is 91 px tall (five 17 px rows and the 3 px border).
#[test]
fn views_popup_is_a_context_menu_under_its_button() {
    let text = text();
    let mut d = sample();
    let r = R;
    assert!(matches!(
        d.click(r, Some(Hit::ToolBar(TOOLBAR_VIEWS)), Mods::default()),
        Event::Redraw
    ));
    assert!(d.views_open);
    let lay = d.layout(&text, 11, r, &ControlMetrics::CLASSIC);
    let (p, menu) = lay
        .views
        .as_ref()
        .expect("the popup is laid out while open");
    let p = *p;
    assert_eq!(p.height, 5 * 17 + 6);
    assert_eq!(p.y, lay.toolbar_rects[TOOLBAR_VIEWS].bottom());
    assert_eq!(p.x, lay.toolbar_rects[TOOLBAR_VIEWS].x);
    let third = menu.rows[2];
    let (x, y) = (
        p.x + third.x + third.width / 2,
        p.y + third.y + third.height / 2,
    );
    assert_eq!(d.hit(&text, 11, r, x, y), Some(Hit::ViewsItem(2)));
    assert_eq!(
        d.hit(&text, 11, r, p.x + 1, p.y + 1),
        None,
        "the border is not a row"
    );
    assert_eq!(d.hit(&text, 11, r, p.right() + 5, y), None);
    let mut c = Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32);
    d.draw(
        &mut c,
        &text,
        &classic_paint(&ColorScheme::default(), 1),
        r,
        11,
    );
    assert!(matches!(
        d.click(r, Some(Hit::ViewsItem(4)), Mods::default()),
        Event::Redraw
    ));
    assert_eq!(d.shell.mode, Mode::Details);
    assert!(!d.views_open);
}

#[test]
fn a_hosted_list_or_views_menu_draws_alone_as_in_the_dialog_and_is_hit_where_it_was_put() {
    let text = Rc::new(text());
    let theme = Theme::default();
    let paint = classic_paint(&theme.colors, 1);
    let shot = |d: &FileDlg| {
        let mut c = Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32);
        d.draw(&mut c, &text, &paint, R, 11);
        c
    };
    let opens = [
        Hit::LookIn(combo::Hit::Drop),
        Hit::FileType(combo::Hit::Drop),
        Hit::ToolBar(TOOLBAR_VIEWS),
    ];
    for open in opens {
        let mut d = sample();
        d.fields.filters = vec!["Text (*.txt)".into(), "All Files (*.*)".into()];
        d.filetype = Combo::new(&["Text (*.txt)", "All Files (*.*)"], 0, 2);
        d.set_listing(folder(1, None, vec![dir("Docs")]));
        let popped = |d: &mut FileDlg| -> Vec<Spot> {
            Popped {
                dlg: d,
                text: text.clone(),
                px: 11,
            }
            .spots()
        };
        assert!(popped(&mut d).is_empty(), "{open:?}: nothing while closed");
        d.click(R, Some(open), Mods::default());
        let whole = shot(&d);
        let a = popped(&mut d)[0].rect;
        let mut p = Popped {
            dlg: &mut d,
            text: text.clone(),
            px: 11,
        };
        let mut own = Canvas::new(a.width as u32, a.height as u32);
        p.draw_level(0, &mut own, (-a.x, -a.y), &theme, &text);
        for y in 0..a.height {
            for x in 0..a.width {
                assert_eq!(
                    own.get(x, y),
                    whole.get(a.x + x, a.y + y),
                    "{open:?} ({x}, {y})"
                );
            }
        }
        p.host();
        let spot = p.spots()[0];
        let b = spot.rect;
        assert_eq!((b.width, b.height), (a.width, a.height), "{open:?}");
        let home = match spot.anchor {
            Anchor::Below(under) => (under.x, under.bottom()),
            Anchor::Point(x, y) => (x, y),
            Anchor::Beside { .. } => panic!("{open:?}: a root level"),
        };
        assert_eq!((b.x, b.y), home, "{open:?}");
        assert_eq!(b, a, "{open:?}: where the dialog showed it");
        p.place(0, (900, 700));
        let bare = shot(&d);
        assert!(
            (0..a.height).any(|y| bare.get(a.x + 4, a.y + y) != whole.get(a.x + 4, a.y + y)),
            "{open:?}: the dialog leaves it out"
        );
        let lay = d.layout(&text, 11, R, &ControlMetrics::CLASSIC);
        let (at, want) = match open {
            Hit::LookIn(_) => ((905, 702), Hit::LookIn(combo::Hit::Row(0))),
            Hit::FileType(_) => (
                (905, 702 + d.filetype.row_h(&ControlMetrics::CLASSIC)),
                Hit::FileType(combo::Hit::Row(1)),
            ),
            _ => {
                let (r, menu) = lay.views.as_ref().unwrap();
                assert_eq!((r.x, r.y), (900, 700));
                let row = menu.rows[2];
                (
                    (r.x + row.x + row.width / 2, r.y + row.y + row.height / 2),
                    Hit::ViewsItem(2),
                )
            }
        };
        assert_eq!(d.hit(&text, 11, R, at.0, at.1), Some(want), "{open:?}");
    }
}

#[test]
fn a_hosted_extra_list_draws_alone_as_in_the_dialog_and_is_hit_where_it_was_put() {
    let text = Rc::new(text());
    let theme = Theme::default();
    let paint = classic_paint(&theme.colors, 1);
    let mut d = with_extra();
    let (w, h) = d.min_client();
    d.set_size((w, h + 40));
    let r = Rect::new(0, 0, w, h + 40);
    let shot = |d: &FileDlg| {
        let mut c = Canvas::new(w as u32, (h + 40) as u32);
        d.draw(&mut c, &text, &paint, r, 11);
        c
    };
    let popped = |d: &mut FileDlg| -> Vec<Spot> {
        Popped {
            dlg: d,
            text: text.clone(),
            px: 11,
        }
        .spots()
    };
    shot(&d);
    assert!(popped(&mut d).is_empty(), "nothing while closed");
    let (ox, oy) = d.extra_origin(r);
    assert_eq!(oy, 40, "the rows sit 40 px lower in the taller client");
    let f = d.extra().unwrap().rect("extra-combo").unwrap();
    let field = Rect::new(f.x + ox, f.y + oy, f.width, f.height);
    let drop = d.hit(&text, 11, r, field.right() - 5, field.y + 5);
    assert!(matches!(drop, Some(Hit::Extra(_))), "{drop:?}");
    d.click(r, drop, Mods::default());
    let whole = shot(&d);
    let spot = popped(&mut d)[0];
    assert_eq!(spot.anchor, Anchor::Below(field), "under the drawn field");
    let a = spot.rect;
    assert_eq!((a.x, a.y), (field.x, field.bottom()));
    let mut p = Popped {
        dlg: &mut d,
        text: text.clone(),
        px: 11,
    };
    let mut own = Canvas::new(a.width as u32, a.height as u32);
    p.draw_level(0, &mut own, (-a.x, -a.y), &theme, &text);
    for y in 0..a.height {
        for x in 0..a.width {
            assert_eq!(own.get(x, y), whole.get(a.x + x, a.y + y), "({x}, {y})");
        }
    }
    p.host();
    p.place(0, (600, 700));
    assert_eq!(
        p.spots()[0].rect.y,
        700,
        "the list's spot is where it was put"
    );
    let bare = shot(&d);
    assert!(
        (0..a.height).any(|y| bare.get(a.x + 4, a.y + y) != whole.get(a.x + 4, a.y + y)),
        "the dialog leaves it out"
    );
    let row = 702 + Combo::new(&["a"], 0, 1).row_h(&ControlMetrics::CLASSIC);
    let pick = d.hit(&text, 11, r, 605, row);
    d.click(r, pick, Mods::default());
    let f = d.extra().unwrap();
    assert_eq!(f.combo_selected("extra-combo"), Some(1), "the second row");
    assert!(popped(&mut d).is_empty(), "the pick closes it");
}

/// The toolbar's four buttons run from its left edge and end 84 px
/// short of the client's right edge; the Views popup is laid out only
/// while it is open.
#[test]
fn toolbar_ends_84_px_short_of_the_right_edge() {
    let lay = multi(DlgMode::Open, false).layout(&text(), 11, R, &ControlMetrics::CLASSIC);
    assert_eq!(lay.toolbar_rects[0].x, lay.toolbar.x);
    assert_eq!(lay.toolbar_rects[TOOLBAR_VIEWS].right(), CLIENT.0 - 84);
    assert_eq!(lay.toolbar.right(), CLIENT.0 - 84);
    assert!(lay.views.is_none(), "no popup while it is closed");
}

/// End and a typed letter scroll the entry they reach into view once, in every mode.
#[test]
fn a_key_scrolls_the_entry_it_reaches_into_view() {
    let names = |i: usize| format!("{}{i:04}.png", if i < 1000 { "a" } else { "b" });
    for mode in [
        Mode::Thumbnails,
        Mode::Tiles,
        Mode::Icons,
        Mode::List,
        Mode::Details,
    ] {
        let mut d = sample();
        d.set_listing(folder(
            1,
            None,
            (0..2000).map(|i| file(&names(i))).collect(),
        ));
        d.shell.set_mode(mode);
        d.focus = Focus::Shell;
        let cm = ControlMetrics::CLASSIC;
        let shown = |d: &FileDlg, i: usize| {
            let lay = d.shell_layout(R, &cm);
            let cell = lay.cells[i];
            let body = lay.body;
            body.x <= cell.x
                && cell.right() <= body.right()
                && body.y <= cell.y
                && cell.bottom() <= body.bottom()
        };
        d.key(R, form::Key::End, None, Mods::default());
        assert!(shown(&d, 1999), "{mode:?}: End shows the last entry");
        d.key(R, form::Key::Home, None, Mods::default());
        d.key(R, form::Key::Char('b'), Some('b'), Mods::default());
        assert_eq!(d.shell.selection.focus, Some(1000));
        assert!(
            shown(&d, 1000),
            "{mode:?}: the typed letter's entry is shown"
        );
    }
}

/// `top` is px in Icons and rows in Details, so a Views pick starts
/// the new mode scrolled to the top-left.
#[test]
fn views_pick_resets_the_scroll() {
    let mut d = sample();
    d.shell.mode = Mode::Icons;
    d.shell.top = 400;
    d.shell.left = 30;
    d.click(R, Some(Hit::ToolBar(TOOLBAR_VIEWS)), Mods::default());
    d.click(R, Some(Hit::ViewsItem(4)), Mods::default());
    assert_eq!(d.shell.mode, Mode::Details);
    assert_eq!((d.shell.top, d.shell.left), (0, 0), "top and left are 0");
}

#[test]
fn a_reader_scrolls_a_file_in_and_focuses_the_buttons_without_pressing_them() {
    let t = text();
    let px = Theme::default().fonts.size_px;
    let mut d = sample();
    d.set_listing(folder(
        1,
        None,
        (0..200).map(|i| file(&format!("f{i}.txt"))).collect(),
    ));
    d.shell.set_mode(Mode::List);
    let cm = ControlMetrics::CLASSIC;
    let lay = d.layout(&t, px, R, &cm);
    let mid = |r: Rect| (r.x + r.width / 2, r.y + r.height / 2);
    let shell = mid(d.shell_layout(R, &cm).body);
    let picked = d.shell.selection.focus;
    assert!(matches!(d.reveal(&t, px, R, shell, 199), Event::Redraw));
    let cells = d.shell_layout(R, &cm);
    assert!(
        cells.cells[199].right() <= cells.body.right(),
        "the last file shows"
    );
    assert!(matches!(d.reveal(&t, px, R, shell, 199), Event::None));
    assert_eq!(d.shell.selection.focus, picked, "the pick stays");
    assert!(matches!(
        d.focus_at(&t, px, R, mid(lay.cancel)),
        Event::Redraw
    ));
    assert_eq!(
        d.focus,
        Focus::Cancel,
        "Cancel takes the keyboard and cancels nothing"
    );
    d.filename.text = "a.txt".into();
    assert!(matches!(
        d.focus_at(&t, px, R, mid(lay.accept)),
        Event::Redraw
    ));
    assert_eq!(
        d.focus,
        Focus::Accept,
        "Open takes the keyboard and opens nothing"
    );
    assert!(matches!(
        d.focus_at(&t, px, R, mid(lay.accept)),
        Event::None
    ));
    d.lookin.open = Some(0);
    assert!(matches!(
        d.focus_at(&t, px, R, mid(lay.filetype.field)),
        Event::Redraw
    ));
    assert_eq!((d.focus, d.lookin.open), (Focus::FileType, None));
}

fn folder(id: FolderId, parent: Option<FolderId>, entries: Vec<shellview::Entry>) -> Folder {
    Folder {
        id,
        parent,
        crumbs: vec![Crumb {
            id,
            label: "Folder".into(),
            depth: 0,
            icon: None,
        }],
        entries,
    }
}

fn dir(name: &str) -> shellview::Entry {
    shellview::Entry {
        id: 900,
        name: name.into(),
        is_dir: true,
        kind: "File Folder".into(),
        ..Default::default()
    }
}

fn file(name: &str) -> shellview::Entry {
    shellview::Entry {
        name: name.into(),
        kind: "Text Document".into(),
        ..Default::default()
    }
}

fn multi(mode: DlgMode, multiple: bool) -> FileDlg {
    let mut d = FileDlg::new(
        Fields {
            title: "Open".into(),
            accept_label: "Open".into(),
            filters: vec!["All Files".into(), "Text".into()],
            mode,
            multiple,
        },
        vec![],
    );
    d.set_listing(folder(
        1,
        None,
        vec![dir("sub"), file("a.txt"), file("b.txt"), file("c.txt")],
    ));
    d
}

const CTRL: Mods = Mods {
    ctrl: true,
    shift: false,
};
const SHIFT: Mods = Mods {
    ctrl: false,
    shift: true,
};

fn entry_hit(i: usize) -> Option<Hit> {
    Some(Hit::Shell(shellview::Hit::Entry(i)))
}

#[test]
fn ctrl_and_shift_clicks_select_several_files_when_multiple() {
    let mut d = multi(DlgMode::Open, true);
    d.click(R, entry_hit(1), Mods::default());
    d.click(R, entry_hit(3), CTRL);
    assert_eq!(d.file_name(), "\"c.txt\" \"a.txt\"");
    d.click(R, entry_hit(2), SHIFT);
    assert_eq!(
        d.file_name(),
        "\"b.txt\" \"a.txt\"",
        "shift ranges from the anchor"
    );
}

#[test]
fn several_names_start_at_the_focused_file() {
    let mut d = multi(DlgMode::Open, true);
    d.click(R, entry_hit(3), Mods::default());
    d.click(R, entry_hit(1), CTRL);
    d.click(R, entry_hit(2), CTRL);
    assert_eq!(d.file_name(), "\"b.txt\" \"c.txt\" \"a.txt\"");
    d.click(R, entry_hit(1), CTRL);
    assert_eq!(
        d.file_name(),
        "\"b.txt\" \"c.txt\"",
        "a file clicked out of the pick starts the walk"
    );
}

#[test]
fn folder_mode_has_no_files_of_type_row() {
    let mut d = multi(DlgMode::Folder, false);
    let t = text();
    let theme = Theme::default();
    let cs = &theme.colors;
    let mut c = Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32);
    c.fill_rect(c.bounds(), cs.button_face);
    d.draw(&mut c, &t, &classic_paint(cs, 1), R, 11);
    for y in FILETYPE_Y..FILETYPE_Y + FILETYPE_H {
        for x in ROW_LABEL_X..FIELD_X + field_w(CLIENT.0) {
            assert_eq!(
                c.get(x, y),
                cs.button_face.argb(),
                "({x}, {y}) is bare dialog face"
            );
        }
    }
    assert!(
        !matches!(
            d.hit(&t, 11, R, FIELD_X + 20, FILETYPE_Y + 10),
            Some(Hit::FileType(_))
        ),
        "no Files of type combo to hit"
    );
    d.focus = Focus::FileName;
    d.key(R, form::Key::Tab { back: false }, None, Mods::default());
    assert_eq!(d.focus, Focus::Accept, "Tab goes from File name to Select");
    d.key(R, form::Key::Tab { back: true }, None, Mods::default());
    assert_eq!(d.focus, Focus::FileName, "and back");
}

#[test]
fn without_multiple_ctrl_click_replaces_the_selection() {
    let mut d = multi(DlgMode::Open, false);
    d.click(R, entry_hit(1), Mods::default());
    d.click(R, entry_hit(3), CTRL);
    assert_eq!(d.file_name(), "c.txt");
}

#[test]
fn a_folder_in_a_multi_selection_stays_out_of_the_field() {
    let mut d = multi(DlgMode::Open, true);
    d.click(R, entry_hit(0), Mods::default());
    d.click(R, entry_hit(1), CTRL);
    d.click(R, entry_hit(2), CTRL);
    assert_eq!(d.file_name(), "\"b.txt\" \"a.txt\"");
}

#[test]
fn keyboard_multi_selection() {
    let mut d = multi(DlgMode::Open, true);
    d.click(R, entry_hit(1), Mods::default());
    d.key(R, form::Key::Down, None, SHIFT);
    assert_eq!(d.file_name(), "\"b.txt\" \"a.txt\"");
    d.key(R, form::Key::Down, None, CTRL);
    d.key(R, form::Key::Space, None, CTRL);
    assert_eq!(d.file_name(), "\"c.txt\" \"a.txt\" \"b.txt\"");
}

#[test]
fn accept_carries_every_selected_name() {
    let mut d = multi(DlgMode::Open, true);
    d.click(R, entry_hit(1), Mods::default());
    d.click(R, entry_hit(2), CTRL);
    match d.click(R, Some(Hit::Accept), Mods::default()) {
        Event::Accept(names) => assert_eq!(names, vec!["b.txt", "a.txt"]),
        e => panic!("expected Accept, got {e:?}"),
    }
}

#[test]
fn quoted_names_split_and_a_plain_name_stays_whole() {
    assert_eq!(
        split_names("\"a b.txt\" \"c.txt\""),
        vec!["a b.txt", "c.txt"]
    );
    assert_eq!(split_names("a b.txt"), vec!["a b.txt"]);
    assert_eq!(split_names("  "), Vec::<String>::new());
    assert_eq!(
        split_names("x \"y\""),
        vec!["x \"y\""],
        "quotes split only from the start"
    );
}

#[test]
fn folder_mode_accepts_the_selected_subfolder_from_the_button() {
    let mut d = multi(DlgMode::Folder, false);
    d.click(R, entry_hit(0), Mods::default());
    match d.click(R, Some(Hit::Accept), Mods::default()) {
        Event::Accept(names) => assert_eq!(names, vec!["sub"]),
        e => panic!("expected Accept, got {e:?}"),
    }
}

#[test]
fn a_keyboard_pick_replaces_a_clicked_folders_name_before_enter() {
    let mut d = multi(DlgMode::Open, false);
    d.click(R, entry_hit(0), Mods::default());
    assert_eq!(d.file_name(), "sub");
    d.key(R, form::Key::End, None, Mods::default());
    assert_eq!(d.file_name(), "c.txt");
    match d.key(R, form::Key::Enter, None, Mods::default()) {
        Event::Accept(names) => assert_eq!(names, vec!["c.txt"]),
        e => panic!("expected Accept of c.txt, got {e:?}"),
    }
    d.click(R, entry_hit(0), Mods::default());
    d.key(R, form::Key::Char('b'), Some('b'), Mods::default());
    assert_eq!(d.file_name(), "b.txt", "a typed letter picks too");
}

#[test]
fn folder_mode_enter_on_a_subfolder_opens_it() {
    let mut d = multi(DlgMode::Folder, false);
    d.click(R, entry_hit(0), Mods::default());
    assert!(matches!(
        d.key(R, form::Key::Enter, None, Mods::default()),
        Event::NeedListing(_)
    ));
}

#[test]
fn folder_mode_with_nothing_picked_accepts_the_open_folder() {
    let mut d = multi(DlgMode::Folder, false);
    d.shell.selection.set_only(None);
    d.filename.set_selected("");
    match d.click(R, Some(Hit::Accept), Mods::default()) {
        Event::Accept(names) => assert_eq!(names, vec!["."]),
        e => panic!("expected Accept, got {e:?}"),
    }
}

#[test]
fn folder_mode_accepts_the_open_folder_right_after_listing() {
    let mut d = multi(DlgMode::Folder, false);
    assert_eq!(
        d.shell.selection.focus, None,
        "a folder listing starts with nothing selected"
    );
    match d.click(R, Some(Hit::Accept), Mods::default()) {
        Event::Accept(names) => assert_eq!(names, vec!["."]),
        e => panic!("expected Accept, got {e:?}"),
    }
}

#[test]
fn the_files_of_type_list_opens_above_when_it_would_pass_the_bottom() {
    let mut d = multi(DlgMode::Open, false);
    let t = text();
    let field = d.layout(&t, 11, R, &ControlMetrics::CLASSIC).filetype.field;
    d.click(R, Some(Hit::FileType(combo::Hit::Drop)), Mods::default());
    let list = d.layout(&t, 11, R, &ControlMetrics::CLASSIC).filetype.list;
    assert!(
        list.bottom() <= R.bottom(),
        "the list ends inside the client"
    );
    assert_eq!(list.bottom(), field.y, "it sits right on top of the field");
    let hit = d.hit(&t, 11, R, list.x + 10, list.y + 3);
    assert_eq!(
        hit,
        Some(Hit::FileType(combo::Hit::Row(0))),
        "its first row takes the click"
    );
}

fn save_dlg() -> FileDlg {
    let mut d = multi(DlgMode::Save, false);
    d.set_file_name("report.pdf");
    d
}

#[test]
fn a_paste_goes_into_file_name_only_while_it_has_the_focus() {
    let mut d = save_dlg();
    d.focus = Focus::FileName;
    d.filename.select_all();
    assert!(d.paste("notes.txt\nsecond line"));
    assert_eq!(
        d.file_name(),
        "notes.txt",
        "its first line replaces the selection"
    );
    d.focus = Focus::Shell;
    assert!(!d.paste("other"));
    assert_eq!(d.file_name(), "notes.txt");
}

/// A typed `*` or `?` pattern is not accepted: the host applies it, which
/// filters the view and leaves the pattern in the field.
#[test]
fn a_typed_pattern_filters_instead_of_accepting() {
    let mut d = multi(DlgMode::Open, false);
    d.set_file_name("*");
    match d.key(R, form::Key::Enter, None, Mods::default()) {
        Event::Wildcard { dir, name } => assert_eq!((dir.as_str(), name.as_str()), ("", "*")),
        e => panic!("expected Wildcard, got {e:?}"),
    }
    d.apply_wildcard("*");
    assert_eq!(d.wildcard(), Some("*"));
    assert_eq!(d.file_name(), "*");
    d.set_file_name("/etc/*.conf");
    let name = match d.click(R, Some(Hit::Accept), Mods::default()) {
        Event::Wildcard { dir, name } => {
            assert_eq!((dir.as_str(), name.as_str()), ("/etc/", "/etc/*.conf"));
            name
        }
        e => panic!("expected Wildcard, got {e:?}"),
    };
    assert_eq!(d.wildcard(), Some("*"), "the old filter until applied");
    assert_eq!(d.file_name(), "/etc/*.conf", "the field as typed");
    d.apply_wildcard(&name);
    assert_eq!(d.wildcard(), Some("*.conf"));
    assert_eq!(d.file_name(), "*.conf", "the field keeps the pattern alone");
    d.set_file_name("/e*/a.txt");
    assert!(
        matches!(
            d.key(R, form::Key::Enter, None, Mods::default()),
            Event::Accept(_)
        ),
        "a pattern in the folder part is a name"
    );
}

#[test]
fn picking_a_type_drops_the_typed_pattern() {
    let mut d = multi(DlgMode::Open, false);
    d.set_file_name("*.txt");
    d.apply_wildcard("*.txt");
    assert_eq!(d.wildcard(), Some("*.txt"));
    d.key(R, form::Key::Tab { back: false }, None, Mods::default());
    assert!(matches!(
        d.key(R, form::Key::Down, None, Mods::default()),
        Event::FilterChanged(1)
    ));
    assert_eq!(d.wildcard(), None);
}

#[test]
fn a_listed_name_or_folder_mode_takes_no_pattern() {
    let mut d = multi(DlgMode::Open, false);
    d.set_listing(folder(2, None, vec![file("why?.txt")]));
    d.set_file_name("why?.txt");
    assert!(matches!(
        d.key(R, form::Key::Enter, None, Mods::default()),
        Event::Accept(_)
    ));
    assert_eq!(d.wildcard(), None);
    let mut d = multi(DlgMode::Folder, false);
    d.set_file_name("*");
    assert!(matches!(
        d.key(R, form::Key::Enter, None, Mods::default()),
        Event::Accept(_)
    ));
}

#[test]
fn entering_a_clicked_folder_puts_the_suggested_name_back() {
    let mut d = save_dlg();
    d.click(R, entry_hit(0), Mods::default());
    assert_eq!(d.file_name(), "sub", "a click shows the folder");
    assert!(matches!(
        d.key(R, form::Key::Enter, None, Mods::default()),
        Event::NeedListing(_)
    ));
    assert_eq!(d.file_name(), "report.pdf", "the suggested name is back");
}

#[test]
fn entering_a_typed_folder_name_empties_the_field() {
    let mut d = multi(DlgMode::Open, false);
    d.shell.selection.set_only(Some(0));
    d.filename.set_selected("");
    d.typed.clear();
    for c in "sub".chars() {
        d.key(
            R,
            form::Key::Edit(crate::textedit::EditKey::Insert(c.to_string())),
            None,
            Mods::default(),
        );
    }
    assert!(matches!(
        d.key(R, form::Key::Enter, None, Mods::default()),
        Event::NeedListing(_)
    ));
    assert_eq!(d.file_name(), "");
}

#[test]
fn folder_mode_enter_then_select_picks_the_entered_folder() {
    let mut d = multi(DlgMode::Folder, false);
    d.click(R, entry_hit(0), Mods::default());
    assert!(matches!(
        d.key(R, form::Key::Enter, None, Mods::default()),
        Event::NeedListing(_)
    ));
    d.set_listing(folder(2, Some(1), vec![]));
    match d.click(R, Some(Hit::Accept), Mods::default()) {
        Event::Accept(names) => assert_eq!(names, vec!["."]),
        e => panic!("expected Accept(.), got {e:?}"),
    }
}

#[test]
fn ctrl_click_off_leaves_the_remaining_file_in_the_field() {
    let mut d = multi(DlgMode::Open, true);
    d.click(R, entry_hit(1), Mods::default());
    d.click(R, entry_hit(3), CTRL);
    d.click(R, entry_hit(3), CTRL);
    assert_eq!(d.file_name(), "a.txt");
    match d.click(R, Some(Hit::Accept), Mods::default()) {
        Event::Accept(names) => assert_eq!(names, vec!["a.txt"]),
        e => panic!("expected Accept, got {e:?}"),
    }
}

#[test]
fn a_name_with_quotes_is_one_name_outside_multi_open() {
    let mut d = save_dlg();
    d.set_file_name("The \"Best\" Recipe.html");
    d.shell.selection.set_only(None);
    match d.key(R, form::Key::Enter, None, Mods::default()) {
        Event::Accept(names) => assert_eq!(names, vec!["The \"Best\" Recipe.html"]),
        e => panic!("expected Accept, got {e:?}"),
    }
    d.set_file_name(" spaced ");
    match d.key(R, form::Key::Enter, None, Mods::default()) {
        Event::Accept(names) => assert_eq!(names, vec![" spaced "]),
        e => panic!("expected Accept, got {e:?}"),
    }
}

#[test]
fn many_filters_open_a_list_that_stays_inside_the_top() {
    let filters: Vec<String> = (0..40).map(|i| format!("Type {i}")).collect();
    let mut d = FileDlg::new(
        Fields {
            title: "Open".into(),
            accept_label: "Open".into(),
            filters,
            mode: DlgMode::Open,
            multiple: false,
        },
        vec![],
    );
    d.click(R, Some(Hit::FileType(combo::Hit::Drop)), Mods::default());
    let list = d
        .layout(&text(), 11, R, &ControlMetrics::CLASSIC)
        .filetype
        .list;
    assert!(
        list.y >= 0,
        "the list starts at y {} inside the client",
        list.y
    );
    assert!(list.bottom() <= R.bottom());
}

#[test]
fn an_open_filter_list_takes_clicks_over_the_choice_rows() {
    let mut d = with_extra();
    let (w, h) = d.client();
    let big = Rect::new(0, 0, w, h);
    d.click(big, Some(Hit::FileType(combo::Hit::Drop)), Mods::default());
    let list = d
        .layout(&text(), 11, big, &ControlMetrics::CLASSIC)
        .filetype
        .list;
    let row1 = list.y + 1 + d.filetype.row_h(&ControlMetrics::CLASSIC) + 2;
    assert!(row1 > EXTRA_Y, "the second row lies over the extra rows");
    let hit = d.hit(&text(), 11, big, list.x + 20, row1);
    assert_eq!(hit, Some(Hit::FileType(combo::Hit::Row(1))));
    let ev = d.motion(&text(), 11, big, list.x + 20, row1);
    assert!(matches!(ev, Event::Redraw), "got {ev:?}");
    assert_eq!(d.filetype.open, Some(1));
}

#[test]
fn double_clicking_a_folder_enters_it_in_every_mode() {
    for mode in [DlgMode::Open, DlgMode::Save, DlgMode::Folder] {
        let mut d = multi(mode, false);
        d.click(R, entry_hit(0), Mods::default());
        assert!(
            matches!(d.double_click(R, entry_hit(0)), Event::NeedListing(900)),
            "{mode:?}"
        );
    }
}

#[test]
fn double_clicking_a_file_accepts_it() {
    let mut d = multi(DlgMode::Open, true);
    d.click(R, entry_hit(2), Mods::default());
    match d.double_click(R, entry_hit(2)) {
        Event::Accept(names) => assert_eq!(names, vec!["b.txt"]),
        e => panic!("expected Accept, got {e:?}"),
    }
    let mut f = multi(DlgMode::Folder, false);
    assert!(
        matches!(f.double_click(R, entry_hit(2)), Event::Redraw | Event::None),
        "no file pick in folder mode"
    );
}

#[test]
fn set_filter_picks_the_row_and_clamps() {
    let mut d = multi(DlgMode::Open, false);
    d.set_filter(1);
    assert_eq!(d.filter(), 1);
    d.set_filter(9);
    assert_eq!(d.filter(), 1, "clamped to the last row");
}

#[test]
fn picking_another_filter_says_so() {
    let mut d = multi(DlgMode::Open, false);
    d.focus = Focus::FileType;
    assert!(matches!(
        d.key(R, form::Key::Down, None, Mods::default()),
        Event::FilterChanged(1)
    ));
    assert!(matches!(
        d.key(R, form::Key::Down, None, Mods::default()),
        Event::Redraw | Event::None
    ));
}

fn with_extra() -> FileDlg {
    let mut d = multi(DlgMode::Open, false);
    let w = field_w(CLIENT.0);
    let form = form::Form::new(vec![
        form::Item::check("extra-check", FIELD_X, EXTRA_Y, "Open read-only"),
        form::Item::combo(
            "extra-combo",
            FIELD_X,
            EXTRA_Y + 20,
            w,
            &["UTF-8", "Latin-1"],
            2,
        ),
    ]);
    d.set_extra(form, EXTRA_H, &ControlMetrics::CLASSIC);
    d
}

const EXTRA_H: i32 = 70;

#[test]
fn a_tight_extra_grows_to_show_up_to_four_list_rows() {
    let w = field_w(CLIENT.0);
    let combo_of = |n: usize| {
        let names: Vec<String> = (0..n).map(|i| format!("row {i}")).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        form::Form::new(vec![form::Item::combo("c", FIELD_X, EXTRA_Y, w, &refs, n)])
    };
    for (n, shown) in [(2, 2), (3, 3), (9, 4)] {
        let mut d = multi(DlgMode::Open, false);
        d.set_extra(combo_of(n), 29, &ControlMetrics::CLASSIC);
        let it = d.extra().unwrap().item("c").unwrap();
        let form::Control::Combo(c) = &it.control else {
            panic!("combo")
        };
        assert_eq!(c.rows, shown, "{n} options show {shown} rows");
        let list = combo::layout(c, it.rect, &ControlMetrics::CLASSIC).list;
        assert_eq!(
            d.client().1,
            list.bottom(),
            "the client grows to the list's bottom"
        );
    }
}

#[test]
fn extra_rows_grow_the_client_and_leave_the_rest_in_place() {
    let d = with_extra();
    assert_eq!(d.client(), (CLIENT.0, CLIENT.1 + EXTRA_H));
    let big = Rect::new(0, 0, CLIENT.0, CLIENT.1 + EXTRA_H);
    let t = text();
    let base = multi(DlgMode::Open, false).layout(&t, 11, R, &ControlMetrics::CLASSIC);
    let grown = d.layout(&t, 11, big, &ControlMetrics::CLASSIC);
    assert_eq!(grown.filename, base.filename, "File name field unchanged");
    assert_eq!(grown.accept, base.accept, "accept button unchanged");
    assert_eq!(grown.cancel, base.cancel, "Cancel unchanged");
    assert_eq!(grown.shell.frame, base.shell.frame, "shell view unchanged");
}

#[test]
fn a_click_on_an_extra_row_reaches_it() {
    let mut d = with_extra();
    let big = Rect::new(0, 0, CLIENT.0, CLIENT.1 + EXTRA_H);
    let hit = d.hit(&text(), 11, big, FIELD_X + 5, EXTRA_Y + 5);
    assert!(matches!(hit, Some(Hit::Extra(_))), "got {hit:?}");
    d.click(big, hit, Mods::default());
    assert_eq!(d.extra().unwrap().checked("extra-check"), Some(true));
}

#[test]
fn an_extra_rows_key_wins_and_the_labels_pick_around_it() {
    let mut d = multi(DlgMode::Open, false);
    let form = form::Form::new(vec![
        form::Item::check("subfolders", FIELD_X, EXTRA_Y, "&Include subfolders"),
        form::Item::check("names", FIELD_X, EXTRA_Y + 20, "&Name files"),
    ]);
    d.set_extra(form, EXTRA_H, &ControlMetrics::CLASSIC);
    d.fields.accept_label = "Load".into();
    let alt = |d: &mut FileDlg, c| d.key(R, form::Key::Access(c), None, Mods::default());
    alt(&mut d, 'i');
    assert_eq!(d.extra().unwrap().checked("subfolders"), Some(true));
    assert_eq!(d.focus, Focus::Extra);
    alt(&mut d, 'n');
    assert_eq!(d.extra().unwrap().checked("names"), Some(true));
    let keys = d.labels();
    assert_eq!(keys.lookin, "&Look in:");
    assert_eq!(keys.filename, "&File name:");
    assert_eq!(keys.filetype, "Files of &type:");
    assert_eq!(d.accept_label(), "L&oad", "l is Look in's now");
    alt(&mut d, 'l');
    assert_eq!(d.focus, Focus::LookIn);
    alt(&mut d, 'f');
    assert_eq!(d.focus, Focus::FileName);
}

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
    let squeezed = placesbar::layout(&d.places, lay.places, &cm.places);
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

fn sized(name: &str, size: u64) -> shellview::Entry {
    shellview::Entry { size, ..file(name) }
}

fn names(d: &FileDlg) -> Vec<&str> {
    d.entries().iter().map(|e| e.name.as_str()).collect()
}

#[test]
fn a_restored_view_opens_in_its_mode_and_sort() {
    let mut d = multi(DlgMode::Open, false);
    d.restore(Remembered {
        mode: Mode::Details,
        sort: Field::Size,
        ascending: false,
    });
    d.set_listing(folder(
        2,
        None,
        vec![sized("a", 1), sized("b", 3), dir("c")],
    ));
    assert_eq!(d.view(), Mode::Details);
    assert_eq!(d.shell.sort, Some((1, false)));
    assert_eq!(names(&d), ["b", "a", "c"]);
}

#[test]
fn a_sort_column_the_dialog_does_not_show_restores_as_name_ascending() {
    let mut d = multi(DlgMode::Open, false);
    d.restore(Remembered {
        mode: Mode::Thumbnails,
        sort: Field::Deleted,
        ascending: false,
    });
    assert_eq!(d.view(), Mode::Thumbnails);
    assert_eq!(d.shell.sort, Some((0, true)));
}

#[test]
fn a_views_pick_and_a_column_click_change_what_is_remembered() {
    let mut d = multi(DlgMode::Open, false);
    d.set_listing(folder(2, None, vec![sized("a", 1)]));
    assert_eq!(d.remembered(), Remembered::default());
    d.views_open = true;
    let details = views_items().iter().position(|(_, m)| *m == Mode::Details);
    d.click(R, details.map(Hit::ViewsItem), Mods::default());
    let date = Some(Hit::Shell(shellview::Hit::Header(3)));
    for _ in 0..2 {
        d.click(R, date, Mods::default());
        d.release();
    }
    assert_eq!(
        d.remembered(),
        Remembered {
            mode: Mode::Details,
            sort: Field::Modified,
            ascending: false,
        }
    );
}

#[test]
fn remembered_text_round_trips_and_bad_values_keep_defaults() {
    let r = Remembered {
        mode: Mode::Tiles,
        sort: Field::Kind,
        ascending: false,
    };
    assert_eq!(Remembered::parse(&r.text()), r);
    assert_eq!(Remembered::parse(""), Remembered::default());
    assert_eq!(
        Remembered::parse("view = \"huge\"\nsort = 7\nascending = maybe\n\u{0}"),
        Remembered::default()
    );
    assert_eq!(
        Remembered::parse("view = \"icons\""),
        Remembered {
            mode: Mode::Icons,
            ..Remembered::default()
        }
    );
}

/// A folder lists by Name, ascending, folders first. A click on a
/// column title sorts by it, ascending; a second click on it reverses
/// the whole order, files first. The selection stays on its entry.
#[test]
fn a_column_title_click_sorts_and_a_second_reverses() {
    let mut d = multi(DlgMode::Open, false);
    d.set_listing(folder(
        2,
        None,
        vec![
            sized("b.txt", 10),
            dir("Zeta"),
            sized("a.txt", 300),
            dir("alpha"),
            sized("C.txt", 20),
        ],
    ));
    assert_eq!(d.shell.sort, Some((0, true)));
    assert_eq!(names(&d), ["alpha", "Zeta", "a.txt", "b.txt", "C.txt"]);
    d.shell.selection.set_only(Some(3));
    let size = Some(Hit::Shell(shellview::Hit::Header(1)));
    d.click(R, size, Mods::default());
    d.release();
    assert_eq!(d.shell.sort, Some((1, true)));
    assert_eq!(names(&d), ["alpha", "Zeta", "b.txt", "C.txt", "a.txt"]);
    assert_eq!(d.shell.selection.focus, Some(2), "b.txt stays selected");
    d.click(R, size, Mods::default());
    d.release();
    assert_eq!(d.shell.sort, Some((1, false)));
    assert_eq!(names(&d), ["a.txt", "C.txt", "b.txt", "Zeta", "alpha"]);
    d.set_listing(folder(3, None, vec![dir("x"), sized("y", 1)]));
    assert_eq!(names(&d), ["y", "x"], "the next folder keeps the sort");
    assert_eq!(
        d.shell.selection.focus,
        Some(0),
        "and opens on its first entry"
    );
}

/// The extra rows are drawn as a shifted copy, yet their hit test
/// still reads the look they were drawn in: after a Blue draw, the
/// check box's label 16 px in hits it.
#[test]
fn a_blue_extra_check_box_is_hit_on_its_label_16_px_in() {
    let d = with_extra();
    let big = Rect::new(0, 0, CLIENT.0, CLIENT.1 + EXTRA_H);
    let theme = crate::interfaces::theme_for(crate::interface::Interface::Blue);
    let mut c = Canvas::new(CLIENT.0 as u32, (CLIENT.1 + EXTRA_H) as u32);
    d.draw(&mut c, &text(), &theme.paint(), big, 11);
    let hit = d.hit(&text(), 11, big, FIELD_X + 16, EXTRA_Y + 5);
    assert!(
        matches!(hit, Some(Hit::Extra(h)) if h.item == 0),
        "the label 16 px in hits the check box, got {hit:?}"
    );
}

#[test]
fn a_cut_extra_label_tips_its_whole_text_after_a_blink() {
    const LONG: &str = "Character encoding used when the document is written back to disk";
    let mut d = multi(DlgMode::Open, false);
    let form = form::Form::new(vec![form::Item::label_at(
        "long", FIELD_X, EXTRA_Y, 60, LONG,
    )]);
    d.set_extra(form, EXTRA_H, &ControlMetrics::CLASSIC);
    let big = Rect::new(0, 0, CLIENT.0, CLIENT.1 + EXTRA_H);
    let theme = Theme::default();
    let t = text();
    let draw = |d: &FileDlg| {
        let mut c = Canvas::new(big.width as u32, big.height as u32);
        d.draw(&mut c, &t, &theme.paint(), big, 11);
        c.take_tip().map(|tip| (tip.text, tip.at))
    };
    assert_eq!(draw(&d), None);
    let (x, y) = (FIELD_X + 20, EXTRA_Y + 5);
    assert!(matches!(d.motion(&t, 11, big, x, y), Event::None));
    assert!(
        matches!(d.blink(), Event::Redraw),
        "the blink brings the tip up"
    );
    assert_eq!(d.extra().unwrap().label_tip().as_deref(), Some(LONG));
    assert_eq!(
        draw(&d),
        Some((LONG.to_string(), (x, y))),
        "the drawn rows report the tip at the pointer"
    );
    assert!(matches!(d.leave(), Event::Redraw), "leaving takes it down");
    assert_eq!(d.extra().unwrap().label_tip(), None);
}

/// `hit` lays the dialog out at the sizes of its last draw: after a
/// draw in a look with a 17 px scrollbar, the List view's bar starts
/// 17 px up from the view's bottom, a row that is file list at 16.
#[test]
fn hit_follows_the_scrollbar_thickness_of_the_last_draw() {
    let mut d = multi(DlgMode::Open, false);
    let many = (0..60).map(|i| file(&format!("f{i}.txt"))).collect();
    d.set_listing(folder(1, None, many));
    let thick = ControlMetrics {
        scroll: 17,
        ..ControlMetrics::CLASSIC
    };
    let hb = d
        .shell_layout(R, &thick)
        .hscroll
        .expect("60 files overflow");
    assert_eq!(hb.height, 17);
    let (x, y) = (hb.x + 3, hb.y);
    let back = Some(Hit::Shell(shellview::Hit::HBar(BarHit::Back)));
    assert_eq!(
        d.hit(&text(), 11, R, x, y),
        Some(Hit::Shell(shellview::Hit::Background)),
        "Classic's 16 px bar starts a row lower"
    );
    use crate::look::styled::fixture::{one_part_style, solid_frames};
    use crate::style::{Layout as Stack, Part, PartImage, State};
    let caption = PartImage::new(
        solid_frames(&[crate::theme::Color::rgb(0, 0, 0)]),
        &[State::Normal],
        Stack::Vertical,
    );
    let style = one_part_style(Part::Caption, caption).with_controls(thick);
    let look = crate::look::Look::Styled(std::sync::Arc::new(style));
    let theme = Theme::default();
    let p = Paint {
        cs: &theme.colors,
        look: &look,
        s: 1,
        back: crate::look::Backdrop::Face,
        active: true,
        access_keys: true,
        text: crate::theme::TextUnits::ONE,
    };
    let mut c = Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32);
    d.draw(&mut c, &text(), &p, R, 11);
    assert_eq!(d.hit(&text(), 11, R, x, y), back);
}

#[test]
fn tab_walks_into_the_extra_rows_after_files_of_type() {
    let mut d = with_extra();
    d.focus = Focus::FileType;
    d.key(R, form::Key::Tab { back: false }, None, Mods::default());
    assert_eq!(d.focus, Focus::Extra);
    d.key(R, form::Key::Tab { back: false }, None, Mods::default());
    assert_eq!(d.focus, Focus::Extra, "second extra control");
    d.key(R, form::Key::Tab { back: false }, None, Mods::default());
    assert_eq!(d.focus, Focus::Accept);
}

#[test]
fn extra_rows_pixels() {
    let d = with_extra();
    let (w, h) = d.client();
    let t = text();
    let theme = Theme::default();
    let cs = &theme.colors;
    let mut c = Canvas::new(w as u32, h as u32);
    d.draw(&mut c, &t, &classic_paint(cs, 1), Rect::new(0, 0, w, h), 11);
    let mut base = Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32);
    multi(DlgMode::Open, false).draw(&mut base, &t, &classic_paint(cs, 1), R, 11);
    for y in 0..CLIENT.1 {
        for x in 0..CLIENT.0 {
            let grip = x >= CLIENT.0 - 16 && y >= CLIENT.1 - 16;
            if (y >= EXTRA_Y - 1 && x >= FIELD_X - 1) || grip {
                continue;
            }
            assert_eq!(
                c.get(x, y),
                base.get(x, y),
                "pixel ({x}, {y}) outside the extra rows unchanged"
            );
        }
    }
    assert_eq!(
        c.get(FIELD_X, EXTRA_Y),
        cs.button_shadow.argb(),
        "check box edge at (FIELD_X, EXTRA_Y)"
    );
    assert_eq!(
        c.get(FIELD_X + 6, EXTRA_Y + 6),
        cs.window.argb(),
        "check box well"
    );
    assert_eq!(
        c.get(FIELD_X, EXTRA_Y + 20),
        cs.button_shadow.argb(),
        "combo edge 20 px lower"
    );
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
fn arrow_keys_step_the_listing_before_any_draw() {
    let mut d = sample();
    d.set_listing(folder(1, None, vec![dir("Docs"), file("a.txt")]));
    d.focus = Focus::Shell;
    d.shell.selection.set_only(Some(0));
    assert!(matches!(
        d.key(R, form::Key::Down, None, Mods::default()),
        Event::Redraw
    ));
    assert_eq!(d.shell.selection.single(), Some(1));
}

#[test]
fn space_on_the_focused_accept_button_accepts_and_is_text_in_the_name_field() {
    let mut d = sample();
    d.set_listing(folder(1, None, vec![dir("Docs"), file("a.txt")]));
    d.set_file_name("a.txt");
    let space = || form::Key::Edit(EditKey::Insert(" ".into()));
    d.focus = Focus::FileName;
    assert!(matches!(
        d.key(R, space(), Some(' '), Mods::default()),
        Event::Redraw
    ));
    assert_eq!(
        d.file_name(),
        " ",
        "Space in the name field is text, typed over the selected name"
    );
    d.set_file_name("a.txt");
    d.focus = Focus::Accept;
    match d.key(R, space(), Some(' '), Mods::default()) {
        Event::Accept(names) => assert_eq!(names, vec!["a.txt"]),
        other => panic!("expected Accept([a.txt]), got {other:?}"),
    }
    match d.key(R, form::Key::Space, None, Mods::default()) {
        Event::Accept(names) => assert_eq!(names, vec!["a.txt"]),
        other => panic!("expected Accept([a.txt]) from a flat Space, got {other:?}"),
    }
}

#[test]
fn enter_on_a_file_accepts_its_name() {
    let mut d = sample();
    d.set_listing(folder(1, None, vec![dir("Docs"), file("a.txt")]));
    d.set_file_name("a.txt");
    match d.key(R, form::Key::Enter, None, Mods::default()) {
        Event::Accept(names) => assert_eq!(names, vec!["a.txt"]),
        other => panic!("expected Accept, got {other:?}"),
    }
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

#[test]
fn escape_cancels() {
    let mut d = sample();
    assert!(matches!(
        d.key(R, form::Key::Escape, None, Mods::default()),
        Event::Cancel
    ));
}

#[test]
fn set_toolbar_icons_fills_the_four_items_in_order() {
    let mut d = sample();
    let icon = || Some(Canvas::new(1, 1));
    d.set_toolbar_icons(icon(), None, icon(), None);
    let width_of = |item: &toolbar::Item| match item {
        toolbar::Item::Button { icon, .. } | toolbar::Item::DropButton { icon, .. } => {
            icon.as_ref().map(|c| c.width)
        }
        toolbar::Item::Separator => None,
    };
    let got: Vec<Option<u32>> = d.toolbar.items.iter().map(width_of).collect();
    assert_eq!(
        got,
        vec![Some(1), None, Some(1), Some(16)],
        "Back and New Folder the host's, Up none, Views the drawn 16 px"
    );
}

/// `combo_key` has no Escape arm, so Escape must not reach it.
#[test]
fn escape_closes_an_open_combo_instead_of_the_dialog() {
    let mut d = sample();
    d.lookin.open = Some(0);
    let ev = d.key(R, form::Key::Escape, None, Mods::default());
    assert!(matches!(ev, Event::Redraw), "got {ev:?}");
    assert!(d.lookin.open.is_none(), "the list closed");
}

/// `activate` would accept the File name field and end the dialog.
#[test]
fn enter_commits_an_open_combo_instead_of_accepting() {
    let mut d = sample();
    d.filename.set_selected("a.txt");
    d.lookin.open = Some(0);
    let ev = d.key(R, form::Key::Enter, None, Mods::default());
    assert!(
        !matches!(ev, Event::Accept(_) | Event::Cancel),
        "the dialog must not end while a list is dropped: {ev:?}"
    );
    assert!(d.lookin.open.is_none(), "the list closed");
}

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
fn a_stale_entry_hit_is_ignored() {
    let mut d = sample();
    d.set_listing(folder(1, None, vec![file("a.txt"), file("b.txt")]));
    let hit = Some(Hit::Shell(shellview::Hit::Entry(1)));
    d.set_listing(folder(2, None, vec![]));
    d.click(R, hit, Mods::default());
    assert_eq!(d.shell.selection.single(), None);
    assert_eq!(d.file_name(), "");
    d.shell.selection.set_only(Some(1));
    assert!(matches!(
        d.key(R, form::Key::Enter, None, Mods::default()),
        Event::None
    ));
}

#[test]
fn an_out_of_range_views_row_keeps_the_mode() {
    let mut d = sample();
    d.click(R, Some(Hit::ToolBar(TOOLBAR_VIEWS)), Mods::default());
    d.click(R, Some(Hit::ViewsItem(9)), Mods::default());
    assert_eq!(d.shell.mode, Mode::List);
    assert!(!d.views_open);
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

#[test]
fn tab_wraps_the_focus_order() {
    let mut d = sample();
    assert_eq!(d.focus, Focus::FileName);
    d.key(R, form::Key::Tab { back: true }, None, Mods::default());
    assert_eq!(d.focus, Focus::LookIn);
    d.key(R, form::Key::Tab { back: false }, None, Mods::default());
    assert_eq!(d.focus, Focus::FileName);
    d.key(R, form::Key::Tab { back: false }, None, Mods::default());
    assert_eq!(d.focus, Focus::FileType);
}

#[test]
fn arrow_keys_skip_a_disabled_place() {
    let place = |enabled| Place {
        label: "Place".into(),
        icon: None,
        enabled,
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

/// Left, Right, Home and End arrive as edit keys and walk the view's cells as the arrows do.
#[test]
fn edit_arrow_keys_walk_the_icon_view() {
    let mut d = multi(DlgMode::Open, false);
    let names: Vec<String> = (0..12).map(|i| format!("f{i:02}.txt")).collect();
    d.set_listing(folder(1, None, names.iter().map(|n| file(n)).collect()));
    d.shell.set_mode(Mode::Icons);
    d.focus = Focus::Shell;
    d.shell.selection.set_only(Some(0));
    let mut key = |k: EditKey| {
        d.key(R, form::Key::Edit(k), None, Mods::default());
        d.shell.selection.focus
    };
    assert_eq!(
        key(EditKey::Right {
            select: false,
            word: false
        }),
        Some(1)
    );
    assert_eq!(
        key(EditKey::Left {
            select: false,
            word: false
        }),
        Some(0)
    );
    assert_eq!(key(EditKey::End { select: false }), Some(11));
    assert_eq!(key(EditKey::Home { select: false }), Some(0));
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

#[test]
fn the_wheel_scrolls_the_view_and_an_open_list() {
    let filters: Vec<String> = (0..40).map(|i| format!("Type {i}")).collect();
    let mut d = FileDlg::new(
        Fields {
            title: "Open".into(),
            accept_label: "Open".into(),
            filters,
            mode: DlgMode::Open,
            multiple: false,
        },
        vec![],
    );
    let names: Vec<String> = (0..60).map(|i| format!("f{i:02}.txt")).collect();
    d.set_listing(folder(1, None, names.iter().map(|n| file(n)).collect()));
    d.shell.set_mode(Mode::Icons);
    let view = Some(Hit::Shell(shellview::Hit::Background));
    assert!(matches!(d.wheel(R, view, WHEEL_ROWS), Event::Redraw));
    assert!(d.shell.top > 0, "the view scrolled down");
    assert!(matches!(d.wheel(R, view, -WHEEL_ROWS), Event::Redraw));
    assert_eq!(d.shell.top, 0, "and back up");
    assert!(matches!(d.wheel(R, view, -WHEEL_ROWS), Event::None));
    assert!(matches!(
        d.wheel(R, Some(Hit::Accept), WHEEL_ROWS),
        Event::None
    ));
    assert!(matches!(d.wheel(R, None, WHEEL_ROWS), Event::None));

    let on_type = Some(Hit::FileType(combo::Hit::Field));
    assert!(
        matches!(d.wheel(R, on_type, WHEEL_ROWS), Event::None),
        "an unfocused Files of type ignores the wheel"
    );
    d.click(R, Some(Hit::FileType(combo::Hit::Drop)), Mods::default());
    assert!(matches!(d.wheel(R, view, WHEEL_ROWS), Event::Redraw));
    assert_eq!(
        d.filetype.top, WHEEL_ROWS as usize,
        "the open list scrolled"
    );
    assert_eq!(d.shell.top, 0, "the view under the pointer stayed");
    assert_eq!(d.filetype.selected, 0, "scrolling picks nothing");

    d.click(R, Some(Hit::FileType(combo::Hit::Drop)), Mods::default());
    assert!(d.filetype.open.is_none());
    assert!(matches!(
        d.wheel(R, on_type, WHEEL_ROWS),
        Event::FilterChanged(1)
    ));
    assert!(matches!(
        d.wheel(R, on_type, -2 * WHEEL_ROWS),
        Event::FilterChanged(0)
    ));
}

fn name_field(d: &FileDlg, t: &TextRenderer, px: u32) -> Rect {
    d.layout(t, px, R, &d.drawn.0.get()).filename
}

#[test]
fn text_dropped_on_file_name_lands_at_the_drop_caret_and_takes_the_focus() {
    let (t, px) = (text(), Theme::default().fonts.size_px);
    let mut d = sample();
    d.set_file_name("ab.txt");
    d.focus = Focus::Shell;
    let f = name_field(&d, &t, px);
    let x = f.x + widgets::FIELD_PAD + t.measure("ab", false, px);
    assert!(
        !d.drag_over(&t, px, R, f.x, f.y - 20),
        "the view takes no text"
    );
    assert!(d.drag_over(&t, px, R, x, f.y + 5));
    assert!(d.drop_text(&t, px, R, &crate::textedit::DropLine::Text("c".into())));
    assert_eq!(d.file_name(), "abc.txt");
    assert_eq!(d.filename.selected_text(), "c");
    assert_eq!(d.focus, Focus::FileName);
    assert!(
        !d.drop_text(&t, px, R, &crate::textedit::DropLine::Text("x".into())),
        "the drop ended the drag"
    );
    assert_eq!(d.typed, "abc.txt", "the dropped text counts as typed");
}

#[test]
fn dropped_files_replace_file_name_quoted_only_where_several_are_picked() {
    let (t, px) = (text(), Theme::default().fonts.size_px);
    let names = ["a.txt".to_string(), "/tmp/my b.txt".to_string()];
    let mut one = sample();
    one.set_file_name("old");
    let f = name_field(&one, &t, px);
    assert!(!one.drop_names(&names), "no drag rests on the field");
    assert!(one.drag_over(&t, px, R, f.x + 10, f.y + 5));
    assert!(one.drop_names(&names));
    assert_eq!(one.file_name(), "a.txt");
    assert_eq!(one.filename.selected_text(), "a.txt");
    let mut many = multi(DlgMode::Open, true);
    assert!(many.drag_over(&t, px, R, f.x + 10, f.y + 5));
    assert!(many.drop_names(&names));
    assert_eq!(many.file_name(), r#""a.txt" "/tmp/my b.txt""#);
    assert_eq!(
        split_names(many.file_name()),
        names,
        "Accept takes them back apart"
    );
}

#[test]
fn a_drag_resting_at_file_names_edge_scrolls_it_and_draws_a_drop_caret() {
    let (t, theme) = (text(), Theme::default());
    let px = theme.fonts.size_px;
    let mut d = sample();
    d.set_file_name(&"long name ".repeat(12));
    let f = name_field(&d, &t, px);
    let (y, left) = (f.y + 5, f.x + 2);
    assert!(d.drag_over(&t, px, R, f.x + 40, y));
    assert_eq!(d.drag_scroll(&t, px, R), None, "the middle rests still");
    let end = d.name_drop.unwrap().1;
    assert!(end > 0, "the caret at the end scrolled the name");
    assert!(d.drag_over(&t, px, R, left, y));
    assert!(d.drag_tick(&t, px, R));
    assert_eq!(d.name_drop, Some((left, end - 1)));
    let shot = |d: &FileDlg| {
        let mut c = Canvas::new(R.width as u32, R.height as u32);
        d.draw(&mut c, &t, &classic_paint(&theme.colors, 1), R, px);
        c
    };
    let dropping = shot(&d);
    assert!(d.drag_leave());
    assert!(!d.drag_tick(&t, px, R), "no drag, no step");
    assert_ne!(dropping.pixels, shot(&d).pixels);
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

#[test]
fn hits_on_an_unchanged_dialog_lay_the_shell_view_out_once() {
    let t = text();
    let px = Theme::default().fonts.size_px;
    let mut d = sample();
    d.set_listing(folder(
        1,
        None,
        (0..20_000).map(|i| file(&format!("f{i}.txt"))).collect(),
    ));
    let before = shellview::LAYOUTS.get();
    let body = d.shell_layout(R, &ControlMetrics::CLASSIC).body;
    for i in 0..100 {
        d.hit(&t, px, R, body.x + i % body.width, body.y + body.height / 2);
    }
    assert_eq!(
        shellview::LAYOUTS.get() - before,
        1,
        "100 hits lay the view out once"
    );
    d.shell.top = 1;
    d.hit(&t, px, R, body.x, body.y);
    assert_eq!(
        shellview::LAYOUTS.get() - before,
        2,
        "a scroll lays it out again"
    );
}
