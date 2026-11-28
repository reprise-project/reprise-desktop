use super::*;
use reprisekit::controls::addressbar;

/// The address drop is built before the first listing lands and again when the mounts change; it
/// names the window's place, never the drop's first row.
#[test]
fn a_rebuilt_address_drop_names_the_place_shown() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ctx = test_ctx();
    let place = Place::Folder(dir.path().to_path_buf());
    let mut ex = test_explorer(place.clone(), &mut ctx);
    ex.build_address(&mut ctx);
    assert_eq!(ex.address.text(), place.address_in(&[]));
}

#[test]
fn navigating_to_the_current_address_re_lists_the_folder() {
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::write(dir.path().join("one.txt"), "1").expect("write");
    let mut ctx = test_ctx();
    let here = Place::Folder(dir.path().to_path_buf());
    let mut ex = test_explorer(here.clone(), &mut ctx);
    assert_eq!(ex.items.len(), 1);

    std::fs::write(dir.path().join("two.txt"), "2").expect("write");
    let reply = ex.run(Command::Navigate(here.address_in(&[])), &mut ctx);

    assert!(reply.redraw);
    assert_eq!(ex.items.len(), 2, "the new file shows without a refresh");
    assert!(!ex.history.can_back());
}

/// Enter on a typed folder that does not answer whether it exists (a dead mount) returns at
/// once with the window where it was, and goes there on the wake after the answer lands.
#[test]
fn a_typed_folder_answering_late_opens_on_a_later_wake() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let sub = dir.path().join("sub");
    std::fs::create_dir(&sub).expect("mkdir");
    let mut ctx = test_ctx();
    let here = Place::Folder(dir.path().to_path_buf());
    let mut ex = test_explorer(here.clone(), &mut ctx);
    let (answer, gate) = std::sync::mpsc::channel::<()>();

    let entered = std::time::Instant::now();
    let reply = ex.navigate(
        &sub.display().to_string(),
        move |d| {
            gate.recv_timeout(std::time::Duration::from_secs(20)).ok();
            d.is_dir()
        },
        &mut ctx,
    );
    assert!(
        entered.elapsed() < std::time::Duration::from_secs(10),
        "Enter returns before the folder answers"
    );
    assert_eq!(reply, Reply::none());
    assert_eq!(ex.place, here);

    answer.send(()).expect("send");
    wake_until(&mut ex, &mut ctx, |ex| ex.place != here);
    assert_eq!(ex.place, Place::Folder(sub));
    assert!(ex.history.can_back());
}

#[test]
fn up_and_back_select_the_folder_the_window_came_from() {
    let dir = tempfile::tempdir().expect("a temp dir");
    for name in ["a", "b", "c"] {
        std::fs::create_dir(dir.path().join(name)).expect("mkdir");
    }
    let mut ctx = test_ctx();
    let top = Place::Folder(dir.path().to_path_buf());
    let mut ex = test_explorer(top.clone(), &mut ctx);

    ex.run(Command::GoTo(Place::Folder(dir.path().join("b"))), &mut ctx);
    ex.run(Command::Up, &mut ctx);
    assert_eq!(ex.place, top);
    assert_eq!(ex.selected_paths(), [dir.path().join("b")], "Up selects b");

    ex.run(Command::GoTo(Place::Folder(dir.path().join("c"))), &mut ctx);
    ex.run(Command::Back, &mut ctx);
    assert_eq!(ex.place, top);
    assert_eq!(
        ex.selected_paths(),
        [dir.path().join("c")],
        "Back selects c"
    );
}

#[test]
fn a_blink_redraws_only_while_the_address_field_has_the_caret() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ctx = test_ctx();
    let ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    let mut app = Lazy {
        start: Start {
            place: ex.place.clone(),
            dir: None,
            select: Vec::new(),
            alone: None,
            connect: None,
            search: false,
            tabs: None,
        },
        inner: Some(ex),
        units: TextUnits::ONE,
    };
    let _ = app.event(MAIN, Event::Resized, &mut ctx);
    let _ = app.event(MAIN, Event::Focus(true), &mut ctx);
    app.inner.as_mut().expect("the window").focus = Focus::View;
    assert!(!app.event(MAIN, Event::Blink, &mut ctx).redraw);
    app.inner.as_mut().expect("the window").focus = Focus::Address;
    assert!(app.event(MAIN, Event::Blink, &mut ctx).redraw);
    let _ = app.event(MAIN, Event::Focus(false), &mut ctx);
    assert!(
        !app.event(MAIN, Event::Blink, &mut ctx).redraw,
        "nor while the window is without the keyboard"
    );
}

#[test]
fn the_wheel_under_the_open_address_list_leaves_the_view() {
    use repriseapp::PointerInput;
    let dir = tempfile::tempdir().expect("a temp dir");
    for i in 0..200 {
        std::fs::write(dir.path().join(format!("f{i:03}.txt")), "1").expect("write");
    }
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.sv.set_mode(shellview::Mode::Icons);
    let view = ex.layout(&ctx).view;
    let wheel = PointerInput::Scroll {
        x: view.x + 20,
        y: view.y + 20,
        rows: reprisekit::pointer::WHEEL_ROWS,
    };
    assert!(ex.address.combo.toggle(), "the list opens");
    ex.pointer(wheel, &mut ctx);
    assert_eq!(ex.sv.top, 0, "the view stays under the open list");
    assert!(ex.address.combo.open.is_some(), "the list stays open");
    ex.address.combo.toggle();
    ex.pointer(wheel, &mut ctx);
    assert!(ex.sv.top > 0, "closed, the notch scrolls the view");
}

/// The address list's drop arrow, its rows' centres and the view, in window coordinates.
fn address_spots(ex: &Explorer, ctx: &Ctx) -> ((i32, i32), Vec<(i32, i32)>, Rect) {
    let lay = ex.layout(ctx);
    let px = ctx.theme.fonts.size_px;
    let r = lay.address.expect("the address bar");
    let al = addressbar::layout(&ex.address, &ctx.text, px, r, &ctx.theme.controls());
    let d = al.combo.drop;
    let list = al.combo.list;
    let rows = (list.y..list.bottom())
        .filter_map(
            |y| match addressbar::hit(&ex.address, &al, list.x + 20, y) {
                addressbar::Hit::Combo(combo::Hit::Row(i)) => Some((i, y)),
                _ => None,
            },
        )
        .fold(Vec::<(usize, i32)>::new(), |mut v, (i, y)| {
            if v.last().is_none_or(|l| l.0 != i) {
                v.push((i, y));
            }
            v
        })
        .into_iter()
        .map(|(_, y)| (list.x + 20, y + 2))
        .collect();
    ((d.x + d.width / 2, d.y + d.height / 2), rows, lay.view)
}

#[test]
fn the_open_address_list_closes_on_escape_and_on_a_press_elsewhere() {
    use repriseapp::PointerInput;
    let mut ctx = test_ctx();
    let (_dir, mut ex) = browse_fixture(&mut ctx);
    let press = |x, y, button| PointerInput::Press {
        x,
        y,
        button,
        time: 0,
        mods: repriseapp::Mods::default(),
    };
    let (drop, _, view) = address_spots(&ex, &ctx);
    let here = ex.address.text().to_string();

    ex.pointer(press(drop.0, drop.1, crate::BTN_LEFT), &mut ctx);
    assert!(
        ex.address.combo.open.is_some(),
        "the drop arrow opens the list"
    );
    ex.key(key_in(reprisekit::textedit::keysym::DOWN), &mut ctx);
    ex.key(key_in(reprisekit::textedit::keysym::ESCAPE), &mut ctx);
    assert_eq!(ex.address.combo.open, None, "Escape closes the list");
    assert_eq!(
        ex.address.text(),
        here,
        "the field shows where we are again"
    );
    assert_eq!(ex.focus, Focus::Address, "the field keeps the focus");

    ex.key(key_in(reprisekit::textedit::keysym::F4), &mut ctx);
    assert!(ex.address.combo.open.is_some(), "F4 opens the list");
    let (_, rows, _) = address_spots(&ex, &ctx);
    let below = (view.x + 5, view.bottom() - 5);
    assert!(
        rows.iter().all(|r| r.1 + 20 < below.1),
        "the press lands clear of the list"
    );
    ex.pointer(press(below.0, below.1, crate::BTN_LEFT), &mut ctx);
    assert_eq!(
        ex.address.combo.open, None,
        "a press in the view closes the list"
    );

    ex.pointer(press(drop.0, drop.1, crate::BTN_LEFT), &mut ctx);
    ex.pointer(press(below.0, below.1, crate::BTN_RIGHT), &mut ctx);
    assert_eq!(ex.address.combo.open, None, "a right press closes the list");
    assert!(ex.popup.is_none(), "and opens no menu");
}

#[test]
fn the_open_address_list_lights_the_row_under_the_pointer_and_enter_takes_it() {
    use repriseapp::PointerInput;
    let mut ctx = test_ctx();
    let (_dir, mut ex) = browse_fixture(&mut ctx);
    assert!(ex.address.combo.toggle(), "the list opens");
    let (_, rows, _) = address_spots(&ex, &ctx);
    let documents = ex
        .address_places
        .iter()
        .position(|p| *p == Place::Documents)
        .expect("Documents in the list");
    let (x, y) = rows[documents];
    ex.pointer(PointerInput::Motion { x, y }, &mut ctx);
    assert_eq!(
        ex.address.combo.open,
        Some(documents),
        "the row under the pointer lights"
    );

    ex.key(key_in(reprisekit::textedit::keysym::DOWN), &mut ctx);
    assert_eq!(
        ex.address.combo.open,
        Some(documents + 1),
        "Down moves the highlight"
    );
    ex.key(key_in(reprisekit::textedit::keysym::UP), &mut ctx);
    ex.key(key_in(reprisekit::textedit::keysym::RETURN), &mut ctx);
    assert_eq!(ex.address.combo.open, None, "Enter closes the list");
    assert_eq!(ex.place, Place::Documents, "and goes to the lit row");
}

/// browse_fixture's window with nothing selected and the Address field holding the folder's path
/// and a `/`, typed, its completion list showing sub/ and a.txt.
fn completing(ctx: &mut Ctx) -> (tempfile::TempDir, Explorer) {
    use reprisekit::textedit::keysym;
    let (dir, mut ex) = browse_fixture(ctx);
    ex.sv.selection.set_only(None);
    ex.focus = Focus::Address;
    let typed = format!("{}/", dir.path().display());
    ex.address.set_place(&typed[..typed.len() - 1], None);
    ex.key(key_in(keysym::END), ctx);
    ex.key(
        repriseapp::KeyInput {
            sym: '/' as u32,
            utf8: Some("/".into()),
            mods: repriseapp::Mods::default(),
            edit: reprisekit::textedit::key_for('/' as u32, false, false, Some("/")),
        },
        ctx,
    );
    wake_until(&mut ex, ctx, |ex| ex.address.complete.is_open());
    assert_eq!(ex.address.complete.rows(), ["sub/", "a.txt"]);
    assert_eq!(ex.address.text(), typed);
    (dir, ex)
}

#[test]
fn a_press_on_a_file_closes_the_address_completion_and_selects_the_file() {
    use repriseapp::PointerInput;
    let mut ctx = test_ctx();
    let (_dir, mut ex) = completing(&mut ctx);
    let a = ex
        .items
        .iter()
        .position(|it| it.name == "a.txt")
        .expect("a.txt");
    let sl = ex.view_layout(ex.layout(&ctx).view, &ctx);
    let c = sl.cells[a];
    let (x, y) = (c.x + c.width / 2, c.y + c.height / 2);
    let px = ctx.theme.fonts.size_px;
    let r = ex.layout(&ctx).address.expect("the address bar");
    let al = addressbar::layout(&ex.address, &ctx.text, px, r, &ctx.theme.controls());
    assert!(
        !al.complete.list.contains(x, y),
        "the press lands clear of the list"
    );
    let press = PointerInput::Press {
        x,
        y,
        button: crate::BTN_LEFT,
        time: 0,
        mods: repriseapp::Mods::default(),
    };
    assert!(ex.pointer(press, &mut ctx).redraw);
    assert!(!ex.address.complete.is_open(), "the list closed");
    assert_eq!(
        ex.sv.selection.single(),
        Some(a),
        "the press selected a.txt"
    );
    ex.key(key_in(reprisekit::textedit::keysym::F2), &mut ctx);
    assert_eq!(
        ex.sv.edit.as_ref().map(|e| e.0),
        Some(a),
        "F2 renames a.txt"
    );
}

#[test]
fn a_key_moving_the_focus_off_the_address_field_closes_its_completion() {
    use reprisekit::textedit::keysym;
    for (sym, shift) in [
        (keysym::TAB, false),
        (keysym::ISO_LEFT_TAB, true),
        (keysym::F3, false),
    ] {
        let mut ctx = test_ctx();
        let (_dir, mut ex) = completing(&mut ctx);
        let mut k = key_in(sym);
        k.mods.shift = shift;
        ex.key(k, &mut ctx);
        assert_ne!(ex.focus, Focus::Address, "{sym:#x} moves the focus");
        assert!(!ex.address.complete.is_open(), "{sym:#x} closes the list");
    }
}

/// The folder typed into Address is still being read when a press on a file takes the focus off
/// the field: the listing that lands after leaves the list shut, and the next key opens it.
#[test]
fn a_press_off_the_address_field_while_its_folder_reads_keeps_the_late_list_shut() {
    use repriseapp::PointerInput;
    use reprisekit::controls::pathcomplete::Entry;
    use reprisekit::textedit::EditKey;
    let mut ctx = test_ctx();
    let (dir, mut ex) = browse_fixture(&mut ctx);
    ex.sv.selection.set_only(None);
    ex.focus = Focus::Address;
    let folder = dir.path().display().to_string();
    ex.address.set_place(&folder, None);
    let edit = ex.address.combo.edit.as_mut().expect("the address field");
    edit.set_selection(folder.len(), folder.len());
    ex.address.complete.edit(edit, EditKey::Insert("/".into()));
    let want = (ex.address.complete.take_want()).expect("the typed folder is read");
    assert!(
        !ex.address.complete.is_open(),
        "nothing shows while it reads"
    );

    let a = ex
        .items
        .iter()
        .position(|it| it.name == "a.txt")
        .expect("a.txt");
    let sl = ex.view_layout(ex.layout(&ctx).view, &ctx);
    let c = sl.cells[a];
    let press = PointerInput::Press {
        x: c.x + c.width / 2,
        y: c.y + c.height / 2,
        button: crate::BTN_LEFT,
        time: 0,
        mods: repriseapp::Mods::default(),
    };
    ex.pointer(press, &mut ctx);
    assert_ne!(ex.focus, Focus::Address, "the press takes the focus");

    let entries = vec![
        Entry {
            name: "sub".into(),
            dir: true,
        },
        Entry {
            name: "a.txt".into(),
            dir: false,
        },
    ];
    let edit = ex.address.combo.edit.as_mut().expect("the address field");
    ex.address.complete.set_listing(edit, &want, entries);
    assert!(
        !ex.address.complete.is_open(),
        "the listing that lands leaves the list shut"
    );

    ex.focus = Focus::Address;
    ex.key(
        repriseapp::KeyInput {
            sym: 's' as u32,
            utf8: Some("s".into()),
            mods: repriseapp::Mods::default(),
            edit: reprisekit::textedit::key_for('s' as u32, false, false, Some("s")),
        },
        &mut ctx,
    );
    assert_eq!(
        ex.address.complete.rows(),
        ["sub/"],
        "the next key opens the list"
    );
}

/// Ctrl+L, like Alt+D, takes the focus from the view to the Address field with its text selected,
/// so typing replaces the path.
#[test]
fn ctrl_l_focuses_the_address_with_its_text_selected() {
    let mut ctx = test_ctx();
    let (_dir, mut ex) = browse_fixture(&mut ctx);
    ex.focus = Focus::View;
    ex.key(
        repriseapp::KeyInput {
            sym: 'l' as u32,
            utf8: None,
            mods: repriseapp::Mods {
                ctrl: true,
                ..Default::default()
            },
            edit: reprisekit::textedit::key_for('l' as u32, true, false, None),
        },
        &mut ctx,
    );
    assert_eq!(ex.focus, Focus::Address);
    let edit = ex.address.combo.edit.as_ref().expect("the field's edit");
    assert_eq!(
        edit.selection(),
        0..ex.address.text().len(),
        "the whole path"
    );
}

#[test]
fn crumbs_for_a_place_without_a_path_show_one_root_crumb() {
    use crate::chrome::crumbs_for;
    for (place, target) in [
        (Place::Computer, "computer:///"),
        (Place::RecycleBin, "trash:///"),
        (Place::network(), "network:///"),
        (Place::Desktop, ""),
    ] {
        let crumbs = crumbs_for(&place, &[], &[]);
        assert_eq!(crumbs.len(), 1, "{place:?}");
        assert_eq!(crumbs[0].label, place.title(), "{place:?}");
        if !target.is_empty() {
            assert_eq!(crumbs[0].target, target, "{place:?}");
            let (back, _) = Place::read_address(target, None, &[]).expect("reads back");
            assert_eq!(back, place);
        }
    }
    let server = Place::Network("smb://box".to_string());
    let crumbs = crumbs_for(&server, &[], &[]);
    let labels: Vec<&str> = crumbs.iter().map(|c| c.label.as_str()).collect();
    assert_eq!(labels, [Place::network().title().as_str(), "box"]);
}

#[test]
fn crumbs_for_a_home_subfolder_start_at_home() {
    use crate::chrome::crumbs_for;
    let home = PathBuf::from(std::env::var_os("HOME").expect("HOME is set"));
    let music = home.join("Music").join("Albums");
    let crumbs = crumbs_for(&Place::Folder(music.clone()), &[], &[]);
    let got: Vec<(&str, &str)> = (crumbs.iter())
        .map(|c| (c.label.as_str(), c.target.as_str()))
        .collect();
    let home_label = Place::Home.title();
    let (home_s, music_s) = (
        home.display().to_string(),
        home.join("Music").display().to_string(),
    );
    let albums_s = music.display().to_string();
    assert_eq!(
        got,
        [
            (home_label.as_str(), home_s.as_str()),
            ("Music", music_s.as_str()),
            ("Albums", albums_s.as_str()),
        ]
    );
}

#[test]
fn crumbs_for_a_path_outside_home_start_at_computer_and_name_the_drive() {
    use crate::chrome::crumb_places;
    let places = crumb_places(
        &Place::Folder(PathBuf::from("/usr/lib")),
        &[PathBuf::from("/")],
    );
    assert_eq!(
        places,
        [
            Place::Computer,
            Place::Folder(PathBuf::from("/")),
            Place::Folder(PathBuf::from("/usr")),
            Place::Folder(PathBuf::from("/usr/lib")),
        ]
    );
}

#[test]
fn a_chevron_menu_on_a_dead_mount_opens_empty_and_does_not_block() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let real = std::fs::canonicalize(dir.path()).expect("canonical");
    std::fs::create_dir(real.join("inner")).expect("mkdir");
    let mut ctx = test_ctx();
    let here = Place::Folder(real.clone());
    let mut ex = test_explorer(here.clone(), &mut ctx);
    let last = crate::chrome::crumb_places(&here, &ex.drive_mounts).len() - 1;
    let gate = Arc::new(std::sync::RwLock::new(()));
    let closed = gate.write().expect("lock");
    ex.held = Some(gate.clone());
    let started = std::time::Instant::now();
    ex.open_crumb_menu(last, (10, 30), &mut ctx);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(1),
        "no wait on the read"
    );
    let rows = |ex: &Explorer| -> Vec<String> {
        let p = ex.popup.as_ref().expect("the menu is open");
        (p.menu().items().iter())
            .filter_map(|it| match it {
                reprisekit::controls::menu::MenuItem::Entry { label, enabled, .. } => {
                    Some(format!("{label} {enabled}"))
                }
                _ => None,
            })
            .collect()
    };
    assert_eq!(
        rows(&ex),
        ["(Empty) false"],
        "open, empty, while the read is out"
    );

    drop(closed);
    ex.held = None;
    wake_until(&mut ex, &mut ctx, |ex| rows(ex) == ["inner true"]);
}
