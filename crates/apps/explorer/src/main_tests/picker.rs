use super::*;

#[test]
fn the_picker_resizes() {
    use reprisekit::controls::browsefolder::{CLIENT, MIN};
    let mut ctx = test_ctx();
    let (_dir, mut ex) = browse_fixture(&mut ctx);
    ex.open_browse(true, &mut ctx);
    let child = ex
        .children()
        .into_iter()
        .find(|c| c.id == DIALOG)
        .expect("the picker's window");
    assert!(!child.fixed, "it resizes");
    assert_eq!(child.size, CLIENT);
    assert_eq!(ex.min_size(DIALOG), Some(MIN));
    ctx.size = (CLIENT.0 + 100, CLIENT.1 + 50);
    ex.dialog_event(Event::Resized, &mut ctx);
    let (lay, _) = browse_state(&ex, &ctx);
    assert_eq!(lay.tree.frame.width, 286 + 100, "the tree takes the width");
    assert_eq!(lay.cancel.y, 317 + 50, "the buttons ride the bottom");
    let (gx, gy) = (lay.grip.x + 8, lay.grip.y + 8);
    let reply = ex.dialog_event(press_at(gx, gy, 5000), &mut ctx);
    assert!(reply.resize, "the grip resizes the window");
}

#[test]
fn the_status_bars_grip_resizes_the_window() {
    let mut ctx = test_ctx();
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    let (w, h) = ctx.size;
    let press = |x, y| repriseapp::PointerInput::Press {
        x,
        y,
        button: BTN_LEFT,
        time: 0,
        mods: repriseapp::Mods::default(),
    };
    assert!(
        ex.pointer(press(w - 4, h - 4), &mut ctx).resize,
        "on the grip"
    );
    assert!(
        !ex.pointer(press(w - 60, h - 4), &mut ctx).resize,
        "on the panel beside it"
    );
    ctx.maximized = true;
    assert!(
        !ex.pointer(press(w - 4, h - 4), &mut ctx).resize,
        "no grip while maximized"
    );
    assert!(!ex.status.grip, "nor one drawn");
}

#[test]
fn the_picker_has_the_help_caption() {
    let mut ctx = test_ctx();
    let (_dir, mut ex) = browse_fixture(&mut ctx);
    ex.open_browse(true, &mut ctx);
    assert_eq!(ex.caption(DIALOG), repriseapp::Caption::Help);
    assert_eq!(
        ex.caption(MAIN),
        repriseapp::Caption::Window,
        "the main window keeps its own"
    );
}

#[test]
fn the_pickers_cues_wait_for_a_key() {
    use reprisekit::cues::Cues;
    let mut ctx = test_ctx();
    let (_dir, mut ex) = browse_fixture(&mut ctx);
    let cues = |ex: &Explorer| match ex.dialog.as_ref() {
        Some(Dialog::Browse(b)) => b.bf.cues(),
        _ => panic!("the picker is up"),
    };
    ex.open_browse(true, &mut ctx);
    assert_eq!(cues(&ex), Cues::NONE, "opened with the pointer");
    ex.dialog_event(Event::Key(key_in(keysym::ALT_L)), &mut ctx);
    assert_eq!(cues(&ex), Cues::ALL, "Alt brings out both");
    ex.dialog = None;
    ex.keyed = true;
    ex.open_browse(true, &mut ctx);
    assert_eq!(cues(&ex), Cues::ALL, "opened from the keyboard");
}

#[test]
fn the_dialog_opens_on_the_items_folder() {
    let mut ctx = test_ctx();
    let (dir, mut ex) = browse_fixture(&mut ctx);
    ex.open_browse(true, &mut ctx);

    let Some(Dialog::Browse(b)) = ex.dialog.as_ref() else {
        panic!("the Browse dialog is what is up");
    };
    let picked =
        b.bf.tree
            .selected
            .as_deref()
            .and_then(|p| foldertree::place_of(&b.bf.tree, &b.places, p));
    assert_eq!(picked, Some(Place::Folder(dir.path().to_path_buf())));
    assert_eq!(b.bf.accept, "Move");
    assert!(!b.bf.accept_enabled, "a move into its own folder is greyed");
    let child = ex.children();
    let d = child.iter().find(|c| c.id == DIALOG).expect("the dialog");
    assert_eq!(d.title, "Move Items");
    assert_eq!(d.size, reprisekit::controls::browsefolder::CLIENT);
}

#[test]
fn browsing_leaves_the_folders_pane_alone() {
    let mut ctx = test_ctx();
    let (_dir, mut ex) = browse_fixture(&mut ctx);
    let tree = ex.tree.clone();
    let places = ex.tree_places.clone();
    ex.open_browse(true, &mut ctx);

    ex.dialog_event(Event::Key(key_in(keysym::RIGHT)), &mut ctx);
    ex.dialog_event(Event::Key(key_in(keysym::DOWN)), &mut ctx);

    assert!(ex.dialog.is_some(), "the picker is still up");
    assert_eq!(ex.tree, tree);
    assert_eq!(ex.tree_places, places);
}

/// The picker's walk to the items' folder and the judging of its pick read on listing threads:
/// on a folder that does not answer (a dead mount) the picker opens at once on the Desktop with
/// both buttons grey, and walks to the folder on the wake after the reads land.
#[test]
fn the_picker_opens_at_once_and_walks_to_a_folder_that_answers_late() {
    let mut ctx = test_ctx();
    let (dir, mut ex) = browse_fixture(&mut ctx);
    let gate = Arc::new(std::sync::RwLock::new(()));
    let closed = gate.write().expect("lock");
    ex.held = Some(gate.clone());
    ex.open_browse(true, &mut ctx);
    let picked = |ex: &Explorer| {
        let Some(Dialog::Browse(b)) = ex.dialog.as_ref() else {
            panic!("the Browse dialog is what is up");
        };
        let at = b.bf.tree.selected.as_deref();
        let place = at.and_then(|p| foldertree::place_of(&b.bf.tree, &b.places, p));
        (place, b.bf.new_folder_enabled, b.walking.is_some())
    };
    assert_eq!(picked(&ex), (Some(Place::Desktop), false, true));

    drop(closed);
    ex.held = None;
    wake_until(&mut ex, &mut ctx, |ex| !picked(ex).2);
    let here = Place::Folder(dir.path().to_path_buf());
    assert_eq!(picked(&ex), (Some(here), true, false));
}

#[test]
fn accept_queues_a_move_into_the_pick() {
    let mut ctx = test_ctx();
    let (dir, mut ex) = browse_fixture(&mut ctx);
    ex.open_browse(true, &mut ctx);
    let sub = Place::Folder(dir.path().join("sub"));
    let mounts = ex.drive_mounts.clone();
    let Some(Dialog::Browse(b)) = ex.dialog.as_mut() else {
        panic!("the Browse dialog is what is up");
    };
    let path = foldertree::path_of(&b.bf.tree, &b.places, &sub, &mounts);
    assert!(path.is_some(), "sub has a node under its folder");
    b.bf.tree.selected = path;
    let (accept, new_folder) = b.judge().expect("a new pick")();
    b.bf.set_enabled(accept, new_folder);

    ex.dialog_event(Event::Key(key_in(keysym::RETURN)), &mut ctx);

    assert!(ex.dialog.is_none(), "Move closes the picker");
    assert!(ex.put.is_some(), "the move runs on a job");
    ex.finish_puts(&mut ctx);
    assert!(dir.path().join("sub/a.txt").exists());
    assert!(!dir.path().join("a.txt").exists());
}

#[test]
fn the_task_pane_move_and_copy_rows_are_live() {
    let mut ctx = test_ctx();
    let (_dir, mut ex) = browse_fixture(&mut ctx);
    ex.build_taskpane(&mut ctx);

    let reprisekit::controls::taskpane::Body::Links(rows) = &ex.taskpane.sections[0].body else {
        panic!("the tasks section holds links");
    };
    let at = |text: &str| rows.iter().position(|r| r.text == text).expect(text);
    let (mv, cp) = (at("Move this file"), at("Copy this file"));
    assert!(rows[mv].enabled && rows[cp].enabled);
    assert_eq!(ex.pane_cmds[0][mv], Command::MoveTo);
    assert_eq!(ex.pane_cmds[0][cp], Command::CopyTo);
}

#[test]
fn the_picker_opens_on_home_for_an_item_in_home() {
    let Some(home) = Place::Home.path() else {
        return;
    };
    let mut ctx = test_ctx();
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.browse_items(true, vec![home.join("a.txt")], &mut ctx);

    let Some(Dialog::Browse(b)) = ex.dialog.as_ref() else {
        panic!("the Browse dialog is what is up");
    };
    let picked =
        b.bf.tree
            .selected
            .as_deref()
            .and_then(|p| foldertree::place_of(&b.bf.tree, &b.places, p));
    assert_eq!(picked, Some(Place::Home));
    assert!(!b.bf.accept_enabled, "a move into its own folder is greyed");
}

#[test]
fn the_picker_focuses_the_tree_when_the_items_folder_is_hidden() {
    let mut ctx = test_ctx();
    let dir = tempfile::Builder::new()
        .prefix("browse")
        .tempdir()
        .expect("a temp dir");
    let hidden = dir.path().join(".stash");
    std::fs::create_dir(&hidden).expect("mkdir");
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.settings.show_hidden = false;
    ex.browse_items(true, vec![hidden.join("a.txt")], &mut ctx);

    let Some(Dialog::Browse(b)) = ex.dialog.as_ref() else {
        panic!("the Browse dialog is what is up");
    };
    let picked =
        b.bf.tree
            .selected
            .as_deref()
            .and_then(|p| foldertree::place_of(&b.bf.tree, &b.places, p));
    assert_ne!(
        picked,
        Some(Place::Folder(hidden)),
        "the hidden folder has no row"
    );
    assert_eq!(
        b.bf.focus,
        reprisekit::controls::browsefolder::Focus::Tree,
        "the focus is on the tree"
    );
}

/// An icon set on a folder while the picker is open reaches the picker's tree when its folder is
/// read again for the change.
#[test]
fn the_picker_tree_wears_a_folder_icon_set_while_it_is_open() {
    const OWN: u32 = 0xff20_80a0;
    let mut ctx = test_ctx();
    let dir = tempfile::Builder::new()
        .prefix("browse")
        .tempdir()
        .expect("a temp dir");
    let sub = dir.path().join("sub");
    std::fs::create_dir(&sub).expect("mkdir");
    std::fs::write(dir.path().join("a.txt"), "").expect("write");
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.browse_items(true, vec![dir.path().join("a.txt")], &mut ctx);
    let worn = |ex: &Explorer| {
        let Some(Dialog::Browse(b)) = ex.dialog.as_ref() else {
            panic!("the Browse dialog is what is up");
        };
        let path = foldertree::path_of(&b.bf.tree, &b.places, &Place::Folder(sub.clone()), &[])
            .expect("the folder's row");
        let icon = b.bf.tree.node(&path)?.icon.clone()?;
        Some(icon.get(8, 8))
    };
    assert_ne!(worn(&ex), Some(OWN));

    let own = dir.path().join("own.png");
    let mut c = Canvas::new(16, 16);
    c.pixels.fill(OWN);
    std::fs::write(&own, reprisekit::raster::to_png(&c)).expect("write");
    let entry = format!("[Desktop Entry]\nIcon={}\n", own.display());
    std::fs::write(sub.join(".directory"), entry).expect("write");
    assert!(
        ex.refill_browse_node(&Place::Folder(dir.path().to_path_buf()), &mut ctx),
        "the folder is read again at once, so the picker is repainted"
    );
    assert_eq!(worn(&ex), Some(OWN));
}

#[test]
fn make_new_folder_refills_the_folders_pane() {
    let mut ctx = test_ctx();
    let (dir, mut ex) = browse_fixture(&mut ctx);
    let here = Place::Folder(dir.path().to_path_buf());
    ex.build_tree(&mut ctx);
    ex.show(here.clone(), &mut ctx);
    let labels = |ex: &Explorer| -> Vec<String> {
        let path = ex.tree_path_of(&here).expect("this folder's node");
        ex.tree
            .node(&path)
            .map(|n| n.children.iter().map(|c| c.label.clone()).collect())
            .unwrap_or_default()
    };
    assert_eq!(labels(&ex), ["sub"]);
    let a = ex
        .items
        .iter()
        .position(|it| it.name == "a.txt")
        .expect("a.txt");
    ex.sv.selection.set_only(Some(a));
    ex.open_browse(false, &mut ctx);
    let pick = |ex: &Explorer| match ex.dialog.as_ref() {
        Some(Dialog::Browse(b)) => b.bf.tree.selected.clone(),
        _ => None,
    };
    let before = pick(&ex);
    assert!(before.is_some(), "the picker opens with a pick");

    click_at(&mut ex, 60, 328, &mut ctx);

    assert!(dir.path().join("New Folder").is_dir());
    assert!(ex.dialog.is_some(), "the picker stays up");
    let made = match ex.dialog.as_ref() {
        Some(Dialog::Browse(b)) => {
            assert_eq!(b.bf.focus, reprisekit::controls::browsefolder::Focus::Tree);
            let p = b.bf.tree.selected.clone().expect("a pick");
            foldertree::place_of(&b.bf.tree, &b.places, &p)
        }
        _ => None,
    };
    assert_eq!(
        made,
        Some(Place::Folder(dir.path().join("New Folder"))),
        "the new folder is the pick, the tree has the focus"
    );
    assert_eq!(
        pick(&ex).map(|p| p[..p.len() - 1].to_vec()),
        before,
        "under the folder that was picked"
    );
    assert!(labels(&ex).contains(&"New Folder".to_string()));

    for _ in 0..2 {
        ex.dialog_event(Event::Key(key_in(keysym::ESCAPE)), &mut ctx);
    }
    assert!(ex.dialog.is_none());
    ex.undo_last(&mut ctx);
    assert_eq!(labels(&ex), ["sub"], "Undo takes it out of the pane");
}

#[test]
fn make_new_folder_keeps_open_subfolders_and_undoes() {
    let mut ctx = test_ctx();
    let (dir, mut ex) = browse_fixture(&mut ctx);
    std::fs::create_dir(dir.path().join("sub/inner")).expect("mkdir");
    ex.open_browse(false, &mut ctx);
    let sub = Place::Folder(dir.path().join("sub"));
    let mounts = ex.drive_mounts.clone();
    let sub_open = |ex: &Explorer| match ex.dialog.as_ref() {
        Some(Dialog::Browse(b)) => foldertree::path_of(&b.bf.tree, &b.places, &sub, &mounts)
            .and_then(|p| b.bf.tree.node(&p).map(|n| n.expanded)),
        _ => panic!("the picker is up"),
    };
    for k in [
        keysym::TAB,
        keysym::TAB,
        keysym::DOWN,
        keysym::RIGHT,
        keysym::UP,
    ] {
        ex.dialog_event(Event::Key(key_in(k)), &mut ctx);
    }
    assert_eq!(sub_open(&ex), Some(true), "sub is open before");

    click_at(&mut ex, 60, 328, &mut ctx);

    let made = dir.path().join("New Folder");
    assert!(made.is_dir());
    assert_eq!(sub_open(&ex), Some(true), "sub stays open");
    assert_eq!(ex.undo, Some("New"), "Edit's first row offers the undo");
    ex.undo_last(&mut ctx);
    assert!(!made.exists(), "Undo takes the new folder back");
}

#[test]
fn a_folder_move_refills_the_folders_panes_source_and_target() {
    let mut ctx = test_ctx();
    let (dir, mut ex) = browse_fixture(&mut ctx);
    let (from, to) = (dir.path().join("from"), dir.path().join("to"));
    std::fs::create_dir_all(from.join("moved")).expect("mkdir");
    std::fs::create_dir(&to).expect("mkdir");
    ex.build_tree(&mut ctx);
    for d in [&from, &to] {
        ex.tree_follow(&Place::Folder(d.clone()), &mut ctx);
    }
    let labels = |ex: &Explorer, d: &Path| -> Vec<String> {
        let path = ex
            .tree_path_of(&Place::Folder(d.to_path_buf()))
            .expect("the folder's node");
        ex.tree
            .node(&path)
            .map(|n| n.children.iter().map(|c| c.label.clone()).collect())
            .unwrap_or_default()
    };
    assert_eq!(labels(&ex, &from), ["moved"]);

    let op = reprisefs::ops::DropOp::Move;
    ex.put_all(&[from.join("moved")], &to, &|_| Some(op), false, &mut ctx);
    ex.finish_puts(&mut ctx);

    assert!(to.join("moved").is_dir());
    assert!(labels(&ex, &from).is_empty(), "the source node lost it");
    assert_eq!(labels(&ex, &to), ["moved"], "the target node shows it");

    ex.undo_last(&mut ctx);
    assert!(from.join("moved").is_dir());
    assert_eq!(
        labels(&ex, &from),
        ["moved"],
        "Undo puts it back under the source"
    );
    assert!(labels(&ex, &to).is_empty(), "and takes it from the target");
}
