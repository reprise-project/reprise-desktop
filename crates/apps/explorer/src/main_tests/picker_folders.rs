use super::*;

/// Typed text as the seat delivers it, one key per character.
fn type_in(ex: &mut Explorer, s: &str, ctx: &mut Ctx) {
    for ch in s.chars() {
        let utf8 = ch.to_string();
        let k = repriseapp::KeyInput {
            sym: ch as u32,
            utf8: Some(utf8.clone()),
            mods: repriseapp::Mods::default(),
            edit: reprisekit::textedit::key_for(ch as u32, false, false, Some(&utf8)),
        };
        ex.dialog_event(Event::Key(k), ctx);
    }
}

/// The picker's pick and its place, and the open rename box's text.
fn picker_pick(ex: &Explorer) -> (Option<Place>, Option<String>) {
    let Some(Dialog::Browse(b)) = ex.dialog.as_ref() else {
        panic!("the picker is up");
    };
    let place =
        b.bf.tree
            .selected
            .as_deref()
            .and_then(|p| foldertree::place_of(&b.bf.tree, &b.places, p));
    let typed = b.bf.tree.edit.as_ref().map(|(_, e)| e.text.clone());
    (place, typed)
}

/// The error box over the picker, as (title, text), while one is up.
fn picker_error(ex: &Explorer) -> Option<(String, String)> {
    let kid = ex
        .children()
        .into_iter()
        .find(|k| k.id == crate::browse::ERROR)?;
    assert!(kid.modal, "the error box is modal over the picker");
    Some((kid.title, ex.browse_error_fields()?.text))
}

#[test]
fn the_pickers_new_folder_is_named_in_place() {
    let mut ctx = test_ctx();
    let (dir, mut ex) = browse_fixture(&mut ctx);
    ex.open_browse(false, &mut ctx);
    click_at(&mut ex, 60, 328, &mut ctx);
    let made = dir.path().join("New Folder");
    assert_eq!(
        picker_pick(&ex),
        (Some(Place::Folder(made.clone())), Some("New Folder".into())),
        "the box opens on the new folder"
    );
    type_in(&mut ex, "sub", &mut ctx);
    ex.dialog_event(Event::Key(key_in(keysym::RETURN)), &mut ctx);
    assert!(made.is_dir(), "sub is taken");
    assert_eq!(
        picker_error(&ex),
        Some((
            "Error Renaming File or Folder".into(),
            "Cannot rename New Folder: A file with the name you specified already exists. \
             Specify a different file name."
                .into()
        ))
    );
    assert_eq!(picker_pick(&ex).1, None, "the box is closed under it");
    ex.browse_error_event(Event::Key(key_in(keysym::RETURN)));
    assert_eq!(picker_error(&ex), None, "OK closes it");
    assert_eq!(
        picker_pick(&ex).1.as_deref(),
        Some("New Folder"),
        "the box reopens on the name kept"
    );

    type_in(&mut ex, "Photos", &mut ctx);
    ex.dialog_event(Event::Key(key_in(keysym::RETURN)), &mut ctx);
    let photos = dir.path().join("Photos");
    assert!(photos.is_dir() && !made.exists(), "renamed on disk");
    assert_eq!(
        picker_pick(&ex),
        (Some(Place::Folder(photos.clone())), None),
        "the renamed folder stays the pick"
    );
    assert!(ex.dialog.is_some(), "Enter left the picker up");
    ex.undo_last(&mut ctx);
    assert!(
        made.is_dir() && !photos.exists(),
        "Undo takes the name back"
    );
}

fn picker_making(ex: &Explorer) -> bool {
    matches!(&ex.dialog, Some(Dialog::Browse(b)) if b.making.is_some())
}

/// The picker's Make New Folder and its rename run on a listing thread: on a folder that does not
/// answer (a dead mount) the picker stays live, and each lands on the wake after it does.
#[test]
fn the_pickers_new_folder_and_rename_on_a_folder_that_answers_late_land_on_a_later_wake() {
    let mut ctx = test_ctx();
    let (dir, mut ex) = browse_fixture(&mut ctx);
    ex.open_browse(false, &mut ctx);
    let here = Some(Place::Folder(dir.path().to_path_buf()));
    let made = dir.path().join("New Folder");

    let gate = Arc::new(std::sync::RwLock::new(()));
    let closed = gate.write().expect("lock");
    ex.held = Some(gate.clone());
    click_at(&mut ex, 60, 328, &mut ctx);
    assert!(!made.exists(), "the folder waits for the thread");
    assert_eq!(picker_pick(&ex), (here.clone(), None));
    click_at(&mut ex, 60, 328, &mut ctx);
    drop(closed);
    ex.held = None;
    wake_until(&mut ex, &mut ctx, |ex| !picker_making(ex));
    assert_eq!(
        picker_pick(&ex),
        (Some(Place::Folder(made.clone())), Some("New Folder".into())),
        "the box opens on the new folder"
    );
    assert!(
        !dir.path().join("New Folder (2)").exists(),
        "a second click while the first is on disk makes nothing"
    );

    type_in(&mut ex, "Photos", &mut ctx);
    let closed = gate.write().expect("lock");
    ex.held = Some(gate.clone());
    ex.dialog_event(Event::Key(key_in(keysym::RETURN)), &mut ctx);
    assert!(made.is_dir(), "the rename waits for the thread");
    drop(closed);
    ex.held = None;
    wake_until(&mut ex, &mut ctx, |ex| !picker_making(ex));
    let photos = dir.path().join("Photos");
    assert!(photos.is_dir() && !made.exists(), "renamed on disk");
    assert_eq!(
        picker_pick(&ex),
        (Some(Place::Folder(photos)), None),
        "the renamed folder is the pick"
    );
}

/// The picker cancelled while its Make New Folder or rename is still on disk: the change lands
/// anyway, so its undo entry is kept.
#[test]
fn a_pickers_new_folder_and_rename_landing_after_cancel_keep_their_undo() {
    let mut ctx = test_ctx();
    let (dir, mut ex) = browse_fixture(&mut ctx);
    let made = dir.path().join("New Folder");
    let gate = Arc::new(std::sync::RwLock::new(()));

    ex.open_browse(false, &mut ctx);
    let closed = gate.write().expect("lock");
    ex.held = Some(gate.clone());
    click_at(&mut ex, 60, 328, &mut ctx);
    ex.dialog_event(Event::Key(key_in(keysym::ESCAPE)), &mut ctx);
    assert!(ex.dialog.is_none(), "Cancel closes the picker");
    drop(closed);
    ex.held = None;
    wake_until(&mut ex, &mut ctx, |ex| ex.undo.is_some());
    assert!(made.is_dir());
    assert_eq!(ex.undo, Some("New"));

    ex.open_browse(false, &mut ctx);
    click_at(&mut ex, 60, 328, &mut ctx);
    let second = dir.path().join("New Folder (2)");
    assert!(second.is_dir());
    type_in(&mut ex, "Photos", &mut ctx);
    let closed = gate.write().expect("lock");
    ex.held = Some(gate.clone());
    ex.dialog_event(Event::Key(key_in(keysym::RETURN)), &mut ctx);
    ex.dialog_event(Event::Key(key_in(keysym::ESCAPE)), &mut ctx);
    assert!(ex.dialog.is_none(), "Cancel closes the picker");
    drop(closed);
    ex.held = None;
    wake_until(&mut ex, &mut ctx, |ex| ex.undo == Some("Rename"));
    assert!(dir.path().join("Photos").is_dir() && !second.exists());
}

/// A window closed while a picker's rename left by Cancel is still on disk waits for it and
/// keeps its undo until it quits.
#[test]
fn closing_waits_for_a_pickers_rename_left_by_cancel() {
    let mut ctx = test_ctx();
    let (dir, mut ex) = browse_fixture(&mut ctx);
    ex.open_browse(false, &mut ctx);
    click_at(&mut ex, 60, 328, &mut ctx);
    type_in(&mut ex, "Photos", &mut ctx);
    let gate = Arc::new(std::sync::RwLock::new(()));
    let closed = gate.write().expect("lock");
    ex.held = Some(gate.clone());
    ex.dialog_event(Event::Key(key_in(keysym::RETURN)), &mut ctx);
    ex.dialog_event(Event::Key(key_in(keysym::ESCAPE)), &mut ctx);
    assert!(ex.dialog.is_none(), "Cancel closes the picker");
    assert!(!ex.close(&mut ctx).quit, "the window waits for the rename");

    drop(closed);
    ex.held = None;
    for _ in 0..500 {
        if ex.wake(&mut ctx).quit {
            assert_eq!(
                ex.undo,
                Some("Rename"),
                "the rename's undo lands before the quit"
            );
            assert!(dir.path().join("Photos").is_dir());
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("the window quits once the rename lands");
}

/// A rename refused after the wait shows its error box over the picker, and OK reopens the box.
#[test]
fn a_pickers_rename_refused_late_shows_the_error_box() {
    let mut ctx = test_ctx();
    let (dir, mut ex) = browse_fixture(&mut ctx);
    ex.open_browse(false, &mut ctx);
    click_at(&mut ex, 60, 328, &mut ctx);
    type_in(&mut ex, "sub", &mut ctx);
    let gate = Arc::new(std::sync::RwLock::new(()));
    let closed = gate.write().expect("lock");
    ex.held = Some(gate.clone());
    ex.dialog_event(Event::Key(key_in(keysym::RETURN)), &mut ctx);
    assert_eq!(picker_error(&ex), None, "no box before the answer");
    drop(closed);
    ex.held = None;
    wake_until(&mut ex, &mut ctx, |ex| !picker_making(ex));
    assert!(dir.path().join("New Folder").is_dir());
    assert_eq!(
        picker_error(&ex).map(|(title, _)| title),
        Some("Error Renaming File or Folder".into())
    );
    ex.browse_error_event(Event::Key(key_in(keysym::RETURN)));
    assert_eq!(picker_pick(&ex).1.as_deref(), Some("New Folder"));
}

#[test]
fn a_click_on_move_names_the_new_folder_and_moves() {
    let mut ctx = test_ctx();
    let (dir, mut ex) = browse_fixture(&mut ctx);
    ex.open_browse(true, &mut ctx);
    click_at(&mut ex, 60, 328, &mut ctx);
    let (lay, _) = browse_state(&ex, &ctx);
    let (x, y) = (lay.accept.x + 10, lay.accept.y + 10);

    type_in(&mut ex, "sub", &mut ctx);
    click_at(&mut ex, x, y, &mut ctx);
    assert!(picker_error(&ex).is_some(), "the error box is up");
    assert!(ex.put.is_none(), "a refused name moves nothing");
    ex.browse_error_event(Event::CloseRequest);
    assert_eq!(
        picker_pick(&ex).1.as_deref(),
        Some("New Folder"),
        "reopened"
    );

    type_in(&mut ex, "Photos", &mut ctx);
    click_at(&mut ex, x, y, &mut ctx);
    assert!(ex.dialog.is_none(), "the click went on to Move");
    ex.finish_puts(&mut ctx);
    assert!(dir.path().join("Photos/a.txt").exists());
    assert!(!dir.path().join("New Folder").exists());
}

#[test]
fn the_pickers_new_folder_has_no_plus_box() {
    let mut ctx = test_ctx();
    let (_dir, mut ex) = browse_fixture(&mut ctx);
    ex.open_browse(false, &mut ctx);
    click_at(&mut ex, 60, 328, &mut ctx);
    ex.dialog_event(Event::Key(key_in(keysym::ESCAPE)), &mut ctx);
    let (lay, tree) = browse_state(&ex, &ctx);
    let path = tree.selected.clone().expect("the new folder is the pick");
    assert!(
        !tree.node(&path).expect("its node").has_children,
        "an empty folder cannot open"
    );
    let rows = tree.visible();
    let i = tree.selected_row(&rows).expect("in the rows");
    let depth = rows[i].depth as i32;
    let body = lay.tree.body;
    let (bx, by) = (
        body.x - tree.left + 3 + depth * 19 - 15,
        body.y + (i - tree.top) as i32 * 17 + 4,
    );
    let mut c = Canvas::new(
        reprisekit::controls::browsefolder::CLIENT.0 as u32,
        reprisekit::controls::browsefolder::CLIENT.1 as u32,
    );
    ex.draw_dialog(&mut c, &ctx);
    let white = ctx.theme.colors.window.argb();
    assert!(
        (bx..bx + 9).all(|x| (by..by + 9).all(|y| c.get(x, y) == white)),
        "no box at ({bx}, {by})"
    );
}

#[test]
fn the_pickers_buttons_act_on_the_release() {
    use repriseapp::PointerInput;
    let mut ctx = test_ctx();
    let (_dir, mut ex) = browse_fixture(&mut ctx);
    ex.open_browse(true, &mut ctx);
    let (lay, _) = browse_state(&ex, &ctx);
    let (x, y) = (lay.cancel.x + 10, lay.cancel.y + 10);
    ex.dialog_event(press_at(x, y, 5000), &mut ctx);
    assert!(ex.dialog.is_some(), "the press leaves it up");
    ex.dialog_event(
        Event::Pointer(PointerInput::Motion { x: 5, y: 5 }),
        &mut ctx,
    );
    ex.dialog_event(release_at(5, 5), &mut ctx);
    assert!(ex.dialog.is_some(), "a release off Cancel too");
    click_at(&mut ex, x, y, &mut ctx);
    assert!(ex.dialog.is_none(), "a release on it closes it");
}

#[test]
fn the_pickers_tree_takes_the_wheel_and_a_thumb_drag() {
    use repriseapp::PointerInput;
    let mut ctx = test_ctx();
    let (dir, mut ex) = browse_fixture(&mut ctx);
    for i in 0..20 {
        std::fs::create_dir(dir.path().join(format!("folder {i:02}"))).expect("mkdir");
    }
    ex.open_browse(true, &mut ctx);
    let top = |ex: &Explorer| match ex.dialog.as_ref() {
        Some(Dialog::Browse(b)) => b.bf.tree.top,
        _ => panic!("the picker is up"),
    };
    let (lay, _) = browse_state(&ex, &ctx);
    let body = lay.tree.body;
    if let Some(Dialog::Browse(b)) = ex.dialog.as_mut() {
        b.bf.tree.top = 0;
    }
    let (x, y) = (body.x + 20, body.y + 20);
    ex.dialog_event(
        Event::Pointer(PointerInput::Scroll { x, y, rows: 3 }),
        &mut ctx,
    );
    assert_eq!(top(&ex), 3, "the wheel moved the tree three rows");

    let (lay, tree) = browse_state(&ex, &ctx);
    let sb = lay.tree.scrollbar.expect("the tree overflows its box");
    let tx = sb.x + sb.width / 2;
    let ty = (sb.y..sb.bottom())
        .find(|&y| {
            matches!(
                tree.hit(&lay.tree, tx, y),
                reprisekit::controls::treeview::Hit::VBar(
                    reprisekit::classic::widgets::BarHit::Thumb(_)
                )
            )
        })
        .expect("a thumb on the bar");
    ex.dialog_event(press_at(tx, ty, 5000), &mut ctx);
    ex.dialog_event(
        Event::Pointer(PointerInput::Motion { x: tx, y: ty + 30 }),
        &mut ctx,
    );
    let dragged = top(&ex);
    assert!(dragged > 3, "the tree follows the thumb");
    ex.dialog_event(
        Event::Pointer(PointerInput::Release {
            x: tx,
            y: ty + 30,
            button: BTN_LEFT,
            mods: repriseapp::Mods::default(),
        }),
        &mut ctx,
    );
    ex.dialog_event(
        Event::Pointer(PointerInput::Motion { x: tx, y: ty + 90 }),
        &mut ctx,
    );
    assert_eq!(top(&ex), dragged, "a released thumb stays put");
}

#[test]
fn a_click_on_a_picker_row_opens_it() {
    let mut ctx = test_ctx();
    let (dir, mut ex) = browse_fixture(&mut ctx);
    std::fs::create_dir(dir.path().join("sub/inner")).expect("mkdir");
    ex.open_browse(true, &mut ctx);
    let sub = Place::Folder(dir.path().join("sub"));
    let (_, tree) = browse_state(&ex, &ctx);
    let Some(Dialog::Browse(b)) = ex.dialog.as_ref() else {
        panic!("the picker is up");
    };
    let rows = tree.visible();
    let at = rows
        .iter()
        .position(|r| foldertree::place_of(&b.bf.tree, &b.places, &r.path) == Some(sub.clone()))
        .expect("sub's row");
    let sub_path = rows[at].path.clone();
    if let Some(Dialog::Browse(b)) = ex.dialog.as_mut() {
        b.bf.tree.top = at - 1;
    }
    let (lay, tree) = browse_state(&ex, &ctx);
    let y = lay.tree.body.y + (at - tree.top) as i32 * lay.tree.row_h + 8;
    let x = (lay.tree.body.x..lay.tree.body.right())
        .find(|&x| tree.hit(&lay.tree, x, y) == reprisekit::controls::treeview::Hit::Row(at))
        .expect("sub's icon")
        + 2;
    ex.dialog_event(press_at(x, y, 5000), &mut ctx);
    let Some(Dialog::Browse(b)) = ex.dialog.as_ref() else {
        panic!("the picker is up");
    };
    assert_eq!(b.bf.tree.selected, Some(sub_path.clone()), "picked");
    assert!(
        b.bf.tree
            .node(&sub_path)
            .is_some_and(|n| n.expanded && n.children.len() == 1),
        "opened and filled"
    );
}

#[test]
fn a_double_click_on_a_picker_plus_box_turns_it_once() {
    let mut ctx = test_ctx();
    let (dir, mut ex) = browse_fixture(&mut ctx);
    std::fs::create_dir(dir.path().join("sub/inner")).expect("mkdir");
    ex.open_browse(true, &mut ctx);
    let sub = Place::Folder(dir.path().join("sub"));
    let Some(Dialog::Browse(b)) = ex.dialog.as_mut() else {
        panic!("the picker is up");
    };
    let rows = b.bf.tree.visible();
    let at = rows
        .iter()
        .position(|r| foldertree::place_of(&b.bf.tree, &b.places, &r.path) == Some(sub.clone()))
        .expect("sub's row");
    let sub_path = rows[at].path.clone();
    b.bf.tree.top = at - 1;
    let (lay, tree) = browse_state(&ex, &ctx);
    let y = lay.tree.body.y + (at - tree.top) as i32 * lay.tree.row_h + 8;
    let x = (lay.tree.body.x..lay.tree.body.right())
        .find(|&x| tree.hit(&lay.tree, x, y) == reprisekit::controls::treeview::Hit::Expander(at))
        .expect("sub's plus box");
    let open = |ex: &Explorer| match ex.dialog.as_ref() {
        Some(Dialog::Browse(b)) => b.bf.tree.node(&sub_path).is_some_and(|n| n.expanded),
        _ => panic!("the picker is up"),
    };
    assert!(!open(&ex));
    let t = 5000;
    ex.dialog_event(press_at(x, y, t), &mut ctx);
    ex.dialog_event(press_at(x, y, t + 100), &mut ctx);
    assert!(open(&ex), "a double click turns it once");
    ex.dialog_event(press_at(x, y, t + 1000), &mut ctx);
    ex.dialog_event(press_at(x, y, t + 2000), &mut ctx);
    assert!(open(&ex), "two clicks turn it twice");
}
