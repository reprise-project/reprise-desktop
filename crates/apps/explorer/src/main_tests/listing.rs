use super::*;

#[test]
fn a_refresh_keeps_the_selection_when_its_index_moves() {
    let dir = tempfile::tempdir().expect("a temp dir");
    for name in ["b.txt", "c.txt"] {
        std::fs::write(dir.path().join(name), "x").expect("write");
    }
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    let c = ex
        .items
        .iter()
        .position(|it| it.name == "c.txt")
        .expect("c.txt");
    ex.sv.selection.set_only(Some(c));

    std::fs::write(dir.path().join("a.txt"), "x").expect("write");
    ex.refresh(&mut ctx);

    let c = ex
        .items
        .iter()
        .position(|it| it.name == "c.txt")
        .expect("c.txt");
    assert_eq!(ex.sv.selection.set.iter().copied().collect::<Vec<_>>(), [c]);
}

/// Two non-UTF-8 names can show as the same text; a refresh still selects only the file that was selected.
#[test]
fn a_refresh_keeps_the_selection_apart_when_two_names_show_alike() {
    use std::os::unix::ffi::OsStrExt;
    let dir = tempfile::tempdir().expect("a temp dir");
    for name in [b"a\xfe.txt".as_slice(), b"a\xff.txt".as_slice()] {
        std::fs::write(dir.path().join(std::ffi::OsStr::from_bytes(name)), "x").expect("write");
    }
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    assert_eq!(ex.items[0].name, ex.items[1].name);
    let picked = ex.items[1].path.clone();
    ex.sv.selection.set_only(Some(1));

    ex.refresh(&mut ctx);

    let chosen: Vec<&std::path::Path> = ex
        .sv
        .selection
        .set
        .iter()
        .map(|&i| ex.items[i].path.as_path())
        .collect();
    assert_eq!(
        chosen,
        [picked.as_path()],
        "only the picked file is selected"
    );
}

/// Expanding and collapsing the same node over and over must not grow
/// `tree_places`: `fill_node` renumbers the live nodes, so the table
/// stays the size of the tree rather than the size of its history.
#[test]
fn refilling_a_node_does_not_grow_the_place_table() {
    fn live(nodes: &[Node]) -> usize {
        nodes.iter().map(|n| 1 + live(&n.children)).sum()
    }
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::create_dir(dir.path().join("sub")).expect("mkdir");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    let root = vec![0];
    ex.fill_node(root.clone(), &mut ctx);
    let once = ex.tree_places.len();
    for _ in 0..5 {
        ex.fill_node(root.clone(), &mut ctx);
    }
    assert_eq!(ex.tree_places.len(), once);
    assert_eq!(ex.tree_places.len(), live(&ex.tree.roots));
}

/// A Folders node whose read does not answer (a dead mount) stays open and empty, is not read a
/// second time while it waits, and fills on the wake after the read lands.
#[test]
fn a_folders_node_read_late_fills_on_a_later_wake() {
    let dir = tempfile::Builder::new()
        .prefix("tree")
        .tempdir()
        .expect("a temp dir shown in the tree");
    let sub = dir.path().join("sub");
    std::fs::create_dir_all(sub.join("inner")).expect("mkdir");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    let place = Place::Folder(sub);
    let path = ex.tree_path_of(&place).expect("sub's node");
    ex.tree.node_mut(&path).expect("node").expanded = true;
    let read = ex.kids_reader(&path, place.clone());
    let (answer, gate) = std::sync::mpsc::channel::<()>();
    let job = repriseapp::listjob::Job::spawn(None, move || {
        gate.recv().ok();
        read()
    });
    ex.filling.push((place.clone(), job, false));

    ex.fill_node(path.clone(), &mut ctx);
    assert_eq!(ex.filling.len(), 1, "the node is read once");
    assert!(ex.tree.node(&path).expect("node").children.is_empty());

    answer.send(()).expect("send");
    wake_until(&mut ex, &mut ctx, |ex| ex.filling.is_empty());
    let node = ex.tree.node(&path).expect("node");
    assert!(node.expanded, "the node stays open");
    let labels: Vec<&str> = node.children.iter().map(|n| n.label.as_str()).collect();
    assert_eq!(labels, ["inner"]);
}

/// The Desktop node's children are read on a listing thread when the tree is built: a symlink
/// among them may point into a mount that never answers.
#[test]
fn building_the_tree_reads_the_desktop_off_the_loop() {
    use std::sync::{Arc, RwLock};
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    let gate = Arc::new(RwLock::new(()));
    let closed = gate.write().expect("lock");
    ex.held = Some(gate.clone());
    ex.build_tree(&mut ctx);
    let root = &ex.tree.roots[0];
    assert!(
        root.expanded && root.children.is_empty(),
        "nothing read yet"
    );

    drop(closed);
    ex.held = None;
    wake_until(&mut ex, &mut ctx, |ex| ex.filling.is_empty());
    assert!(!ex.tree.roots[0].children.is_empty());
}

/// A Folders node read again after a file operation (a folder made in it) reads on a listing
/// thread: one that does not answer (a dead mount) leaves the node as it was and fills on a wake.
#[test]
fn a_folders_refill_that_does_not_answer_fills_on_a_later_wake() {
    let dir = tempfile::Builder::new()
        .prefix("tree")
        .tempdir()
        .expect("a temp dir shown in the tree");
    std::fs::create_dir(dir.path().join("old")).expect("mkdir");
    let mut ctx = test_ctx();
    let here = Place::Folder(dir.path().to_path_buf());
    let mut ex = test_explorer(here.clone(), &mut ctx);
    let path = ex.tree_path_of(&here).expect("the folder's node");
    let labels = |ex: &Explorer| -> Vec<String> {
        let node = ex.tree.node(&path).expect("node");
        node.children.iter().map(|n| n.label.clone()).collect()
    };
    assert_eq!(labels(&ex), ["old"]);

    std::fs::create_dir(dir.path().join("new")).expect("mkdir");
    let gate = Arc::new(std::sync::RwLock::new(()));
    let closed = gate.write().expect("lock");
    ex.held = Some(gate.clone());
    ex.refill_tree_node(&here, &mut ctx);
    assert_eq!(
        labels(&ex),
        ["old"],
        "the node keeps its rows while the read waits"
    );
    assert_eq!(ex.filling.len(), 1);

    drop(closed);
    wake_until(&mut ex, &mut ctx, |ex| ex.filling.is_empty());
    assert_eq!(labels(&ex), ["new", "old"]);
}

/// A refill asked for while a read of the node that began before the change is out reads the node
/// again once that read lands.
#[test]
fn a_folders_refill_during_an_older_read_reads_the_node_again() {
    let dir = tempfile::Builder::new()
        .prefix("tree")
        .tempdir()
        .expect("a temp dir shown in the tree");
    std::fs::create_dir(dir.path().join("old")).expect("mkdir");
    let mut ctx = test_ctx();
    let here = Place::Folder(dir.path().to_path_buf());
    let mut ex = test_explorer(here.clone(), &mut ctx);
    let path = ex.tree_path_of(&here).expect("the folder's node");
    let labels = |ex: &Explorer| -> Vec<String> {
        let node = ex.tree.node(&path).expect("node");
        node.children.iter().map(|n| n.label.clone()).collect()
    };
    let read = ex.kids_reader(&path, here.clone());
    let (answer, gate) = std::sync::mpsc::channel::<()>();
    let (began, read_done) = std::sync::mpsc::channel::<()>();
    let job = repriseapp::listjob::Job::spawn(None, move || {
        let kids = read();
        began.send(()).ok();
        gate.recv().ok();
        kids
    });
    read_done.recv().expect("the older read ran");
    ex.filling.push((here.clone(), job, false));

    std::fs::create_dir(dir.path().join("new")).expect("mkdir");
    ex.refill_tree_node(&here, &mut ctx);
    assert_eq!(ex.filling.len(), 1, "the refill waits for the read out");

    answer.send(()).expect("send");
    for _ in 0..500 {
        if ex.filling.is_empty() && labels(&ex).len() == 2 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
        ex.wake(&mut ctx);
    }
    assert_eq!(labels(&ex), ["new", "old"]);
}

/// The shown folder's parent renamed away: which folders still exist is asked on a listing
/// thread, so on a dead mount the window stays where it is and goes to the nearest folder left on
/// the wake after the answer.
#[test]
fn a_gone_folder_whose_check_answers_late_is_followed_on_a_later_wake() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let shown = dir.path().join("a/b");
    std::fs::create_dir_all(&shown).expect("mkdir");
    let mut ctx = test_ctx();
    let here = Place::Folder(shown.clone());
    let mut ex = test_explorer(here.clone(), &mut ctx);
    // The folder watch would follow the rename in place to a2/b on a wake of its own.
    ex.watch = None;
    std::fs::rename(dir.path().join("a"), dir.path().join("a2")).expect("rename");

    let gate = Arc::new(std::sync::RwLock::new(()));
    let closed = gate.write().expect("lock");
    ex.held = Some(gate.clone());
    assert_eq!(ex.follow_gone(&shown, &mut ctx), Reply::none());
    assert_eq!(ex.place, here, "the window waits for the check");

    drop(closed);
    ex.held = None;
    wake_until(&mut ex, &mut ctx, |ex| ex.place != here);
    assert_eq!(ex.place, Place::Folder(dir.path().to_path_buf()));
}

/// The tree menu's Delete on the shown folder: the re-listing after the delete finds the folder
/// gone and the window goes to the folder above.
#[test]
fn deleting_the_shown_folder_goes_to_the_folder_above() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let shown = dir.path().join("shown");
    std::fs::create_dir_all(shown.join("inner")).expect("mkdir");
    let mut ctx = test_ctx();
    let here = Place::Folder(shown.clone());
    let mut ex = test_explorer(here.clone(), &mut ctx);

    ex.delete_now(std::slice::from_ref(&shown), true, &mut ctx);
    ex.finish_puts(&mut ctx);
    wake_until(&mut ex, &mut ctx, |ex| ex.place != here);

    assert!(!shown.exists());
    assert_eq!(ex.place, Place::Folder(dir.path().to_path_buf()));
}

#[test]
fn a_refresh_keeps_the_scroll_position() {
    let dir = tempfile::tempdir().expect("a temp dir");
    for i in 0..80 {
        std::fs::write(dir.path().join(format!("f{i:03}.txt")), "x").expect("write");
    }
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.sv.mode = reprisekit::controls::shellview::Mode::Details;
    ex.sv.top = 20;

    std::fs::write(dir.path().join("f999.txt"), "x").expect("write");
    ex.refresh(&mut ctx);
    assert_eq!(ex.items.len(), 81, "the new file is listed");
    assert_eq!(ex.sv.top, 20, "the view stayed where it was");

    for i in 0..80 {
        std::fs::remove_file(dir.path().join(format!("f{i:03}.txt"))).expect("rm");
    }
    ex.refresh(&mut ctx);
    assert_eq!(ex.items.len(), 1);
    assert_eq!(ex.sv.top, 0);

    ex.sv.top = 0;
    let up = ex.place.parent().expect("a parent");
    ex.go(up, &mut ctx);
    assert_eq!(ex.sv.top, 0);
}

/// Long enough for the watch's one-second gap between wakes and its coalescing.
fn wait_for_watch() {
    std::thread::sleep(std::time::Duration::from_millis(1500));
}

#[test]
fn the_shown_folder_is_followed_when_renamed_and_left_for_its_parent_when_deleted() {
    let top = tempfile::tempdir().expect("a temp dir");
    let (old, new) = (top.path().join("old"), top.path().join("new"));
    std::fs::create_dir(&old).expect("mkdir");
    std::fs::write(old.join("a.txt"), "1").expect("write");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(old.clone()), &mut ctx);

    std::fs::rename(&old, &new).expect("rename");
    wait_for_watch();
    ex.wake(&mut ctx);
    assert_eq!(ex.place, Place::Folder(new.clone()));
    assert_eq!(ex.items.len(), 1);
    assert!(
        !ex.history.can_back(),
        "the new name takes the old one's place"
    );

    std::fs::remove_dir_all(&new).expect("rm");
    wait_for_watch();
    ex.wake(&mut ctx);
    assert_eq!(ex.place, Place::Folder(top.path().to_path_buf()));
    assert!(!ex.history.can_back());
}

#[test]
fn a_folder_above_the_shown_one_renamed_is_followed_and_moved_away_is_left_for_the_nearest() {
    let top = tempfile::tempdir().expect("a temp dir");
    let shown = top.path().join("a").join("shown");
    std::fs::create_dir_all(&shown).expect("mkdir");
    std::fs::write(shown.join("a.txt"), "1").expect("write");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(shown.clone()), &mut ctx);
    // `tempdir` names start with a dot, which the tree hides by default.
    ex.settings.show_hidden = true;
    ex.build_tree(&mut ctx);
    ex.show(Place::Folder(shown), &mut ctx);

    std::fs::rename(top.path().join("a"), top.path().join("z")).expect("rename");
    wait_for_watch();
    ex.wake(&mut ctx);
    let renamed = Place::Folder(top.path().join("z").join("shown"));
    assert_eq!(ex.place, renamed);
    assert_eq!(ex.items.len(), 1);
    assert!(!ex.history.can_back());
    let node = ex
        .tree_path_of(&renamed)
        .expect("the new path in the Folders pane");
    assert_eq!(ex.tree.selected, Some(node), "and selected there");
    assert_eq!(
        ex.tree_path_of(&Place::Folder(top.path().join("a"))),
        None,
        "the old name is gone from the pane"
    );

    let away = tempfile::tempdir().expect("a temp dir");
    std::fs::rename(top.path().join("z"), away.path().join("z")).expect("mv");
    wait_for_watch();
    ex.wake(&mut ctx);
    assert_eq!(ex.place, Place::Folder(top.path().to_path_buf()));
    assert!(!ex.history.can_back());
}
