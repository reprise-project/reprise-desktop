use super::*;

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

fn sized(name: &str, size: u64) -> shellview::Entry {
    shellview::Entry { size, ..file(name) }
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

#[test]
fn a_relist_of_the_folder_shown_keeps_the_selection_rename_and_scroll_by_name() {
    let mut d = multi(DlgMode::Open, true);
    d.shell.set_mode(Mode::Details);
    d.click(R, entry_hit(1), Mods::default());
    d.click(R, entry_hit(3), CTRL);
    d.shell.edit = Some((3, TextEdit::rename("c.txt", false)));
    d.shell.top = 1;
    d.set_listing(folder(
        1,
        None,
        vec![file("0.txt"), dir("sub"), file("a.txt"), file("c.txt")],
    ));
    let picked: Vec<&str> = d.selected().iter().map(|e| e.name.as_str()).collect();
    assert_eq!(picked, ["a.txt", "c.txt"], "b.txt left, the rest kept");
    let name = |i: Option<usize>| i.map(|i| d.entries()[i].name.as_str());
    assert_eq!(name(d.shell.selection.focus), Some("c.txt"));
    assert_eq!(name(d.shell.selection.anchor), Some("a.txt"));
    assert_eq!(name(d.shell.edit.as_ref().map(|(i, _)| *i)), Some("c.txt"));
    assert_eq!(
        d.shell.top, 0,
        "the scroll is clamped to the shorter listing"
    );

    assert_eq!(d.file_name(), "\"c.txt\" \"a.txt\"");
    d.set_listing(folder(1, None, vec![dir("sub"), file("a.txt")]));
    assert!(d.shell.edit.is_none(), "the entry being renamed left");
    assert_eq!(d.file_name(), "a.txt", "File name loses the name that left");
    let picked: Vec<&str> = d.selected().iter().map(|e| e.name.as_str()).collect();
    assert_eq!(picked, ["a.txt"]);
    assert_eq!(d.shell.selection.focus, None);
}

#[test]
fn another_folder_starts_on_its_first_entry() {
    let mut d = multi(DlgMode::Open, false);
    d.click(R, entry_hit(2), Mods::default());
    d.shell.edit = Some((2, TextEdit::rename("b.txt", false)));
    d.set_listing(folder(2, Some(1), vec![file("b.txt"), file("x.txt")]));
    assert_eq!(d.shell.selection.set, [0].into());
    assert!(d.shell.edit.is_none());
}
