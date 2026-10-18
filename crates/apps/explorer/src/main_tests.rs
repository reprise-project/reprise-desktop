//! Navigation command tests over a temporary folder; kept out of `main.rs` to keep it under 800 lines.

use super::*;
use crate::commands::Command;
use reprisekit::controls::treeview::{self, Node};
use reprisekit::theme::FontConfig;

/// The process's scratch home for tests: state, cache, data (the Recycle Bin) and config.
fn scratch() -> PathBuf {
    std::env::temp_dir().join(format!("reprise-explorer-tests-{}", std::process::id()))
}

/// Point the Recycle Bin at the scratch home's trash, with no drives and default settings, so a
/// test that deletes or lists the bin never reaches the developer's own. Run once per process by
/// [`test_env`], and again by a test that pointed it elsewhere, as it ends.
pub(crate) fn scratch_bin() {
    reprisefs::trash::set_mount_table(Some("/dev/null".into()));
    reprisefs::bincap::set_config_path(Some(scratch().join("config.toml")));
}

/// Saves land in a scratch state dir, thumbnails in a scratch cache dir and the Recycle Bin in
/// [`scratch_bin`], never the developer's own; set once per process, before a test changes them.
pub(crate) fn test_env() {
    static SCRATCH: std::sync::Once = std::sync::Once::new();
    SCRATCH.call_once(|| {
        let state = scratch();
        // SAFETY: tests run on parallel threads, but std serialises its own
        // env access and the other `test_env` callers wait on the `Once`.
        unsafe {
            std::env::set_var("XDG_STATE_HOME", &state);
            std::env::set_var("XDG_CACHE_HOME", state.join("cache"));
            std::env::set_var("XDG_DATA_HOME", state.join("data"));
        }
        scratch_bin();
        // A script opened in a test runs under this terminal, which records its arguments.
        use std::os::unix::fs::PermissionsExt;
        std::fs::create_dir_all(&state).expect("mkdir");
        let term = state.join("term");
        let argv = state.join("term-argv");
        let script = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" >'{}'\nshift\nexec \"$@\"\n",
            argv.display()
        );
        std::fs::write(&term, script).expect("write");
        std::fs::set_permissions(&term, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        let config = state.join("config.toml");
        std::fs::write(&config, format!("terminal = \"{}\"\n", term.display())).expect("write");
        repriseconf::set_config_path(Some(config));
    });
}

/// A `Ctx` for the navigation commands, in [`test_env`]; its icon theme is a name nothing carries,
/// so every lookup answers `None`.
pub(crate) fn test_ctx() -> Ctx {
    test_env();
    Ctx {
        theme: Theme::default(),
        text: TextRenderer::load(&FontConfig::default()).into(),
        icons: reprisekit::icontheme::IconCache::new(reprisekit::icontheme::IconTheme::load(
            "test",
            &[],
        )),
        caret_on: true,
        size: (792, 573),
        maximized: false,
        origin: (0, 0),
        wake: repriseapp::Waker::noop(),
        heads: Default::default(),
        want_tick: Default::default(),
        at_pick: false,
        text_size: 100,
    }
}

/// An Explorer on default settings and no `.desktop` index.
pub(crate) fn test_explorer(start: Place, ctx: &mut Ctx) -> Explorer {
    Explorer::with_settings(start, Settings::default(), Vec::new(), None, ctx)
}

/// The address drop is built before the first listing lands and again when the mounts change; it
/// names the window's place, never the drop's first row.
#[test]
fn a_rebuilt_address_drop_names_the_place_shown() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ctx = test_ctx();
    let place = Place::Folder(dir.path().to_path_buf());
    let mut ex = test_explorer(place.clone(), &mut ctx);
    ex.build_address(&mut ctx);
    assert_eq!(ex.address.text(), place.address());
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
    let reply = ex.run(Command::Navigate(here.address()), &mut ctx);

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

/// Computer's view and its tree node read the drives scanned at the
/// last `/proc/mounts` change: neither stats the mounts again.
#[test]
fn computer_lists_the_cached_drives() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    let stick = PathBuf::from("/run/media/u/STICK");
    ex.drives = vec![reprisefs::places::Drive {
        mount: stick.clone(),
        device: "/dev/sdz1".into(),
        fs: "vfat".into(),
        label: Some("STICK".into()),
        removable: true,
        optical: false,
        total: 1000,
        free: 500,
    }];

    ex.go(Place::Computer, &mut ctx);
    assert!(
        ex.items.iter().any(|it| it.path == stick),
        "Computer lists /run/media/u/STICK"
    );
    ex.fill_node(vec![0, 2], &mut ctx);
    let computer = ex.tree.node(&[0, 2]).expect("Computer's node");
    assert!(
        computer
            .children
            .iter()
            .any(|n| n.label == "STICK (/run/media/u/STICK)"),
        "the tree lists STICK (/run/media/u/STICK)"
    );
}

#[test]
fn the_desktop_holds_documents_home_computer_and_the_bin() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ctx = test_ctx();
    let ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    let kids: Vec<Place> = ex.tree.roots[0]
        .children
        .iter()
        .map(|n| ex.tree_places[n.id as usize].clone())
        .collect();
    assert_eq!(
        kids[..4],
        [
            Place::Documents,
            Place::Home,
            Place::Computer,
            Place::RecycleBin
        ]
    );
    assert!(
        kids[4..].iter().all(|p| matches!(p, Place::Folder(_))),
        "the rest are the Desktop folder's subfolders: {kids:?}"
    );
    let labels: Vec<&str> = ex.tree.roots[0].children[..4]
        .iter()
        .map(|n| n.label.as_str())
        .collect();
    let home = reprisefs::places::home_label();
    assert_eq!(
        labels,
        ["Documents", home.as_str(), "Computer", "Recycle Bin"]
    );

    let drop: Vec<&Place> = ex
        .address_places
        .iter()
        .filter(|p| !matches!(p, Place::Folder(_)))
        .collect();
    assert_eq!(
        drop,
        [
            &Place::Desktop,
            &Place::Documents,
            &Place::Home,
            &Place::Computer,
            &Place::RecycleBin
        ]
    );
}

#[test]
fn computer_sorts_within_its_groups() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    let drive = |mount: &str, label: &str, removable: bool| reprisefs::places::Drive {
        mount: PathBuf::from(mount),
        device: "/dev/sdz1".into(),
        fs: "ext4".into(),
        label: Some(label.into()),
        removable,
        optical: false,
        total: 1000,
        free: 500,
    };
    ex.drives = vec![
        drive("/run/media/u/STICK", "AAA", true),
        drive("/", "AAB", false),
    ];
    ex.sv.sort = Some((0, true));

    ex.go(Place::Computer, &mut ctx);
    let first_of = |title: &str| {
        let g = ex.sv.groups.iter().find(|g| g.title == title);
        g.map(|g| ex.items[g.first].path.clone())
    };
    assert_eq!(
        first_of("Files Stored on This Computer"),
        std::env::var_os("HOME").map(PathBuf::from),
        "the documents folder heads Files Stored on This Computer"
    );
    assert_eq!(
        first_of("Hard Disk Drives"),
        Some(PathBuf::from("/")),
        "/ heads Hard Disk Drives"
    );
    assert_eq!(
        first_of("Devices with Removable Storage"),
        Some(PathBuf::from("/run/media/u/STICK")),
        "the stick heads Devices with Removable Storage"
    );
}

/// A bare `Item` for the pure sorting and Details tests.
fn item(name: &str, is_dir: bool, size: u64, kind: &str, modified: Option<i64>) -> Item {
    Item {
        name: name.to_string(),
        path: PathBuf::from("/tmp").join(name),
        is_dir,
        is_link: false,
        exec: false,
        program: false,
        hidden: false,
        size,
        modified,
        mime: None,
        kind: kind.to_string(),
        drive: None,
        trashed: None,
        shortcut: None,
    }
}

#[test]
fn ascending_puts_folders_first_and_descending_reverses_it_all() {
    let items = [
        item("zeta", true, 0, "File Folder", Some(10)),
        item("b.txt", false, 300, "Text Document", Some(30)),
        item("A.txt", false, 100, "Text Document", Some(20)),
        item("c.bin", false, 200, "BIN File", Some(40)),
    ];
    let order = |col: usize, asc: bool| {
        crate::place::sort_order(&items, shellview::details_fields()[col], asc)
            .into_iter()
            .map(|i| items[i].name.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(order(0, true), ["zeta", "A.txt", "b.txt", "c.bin"]);
    assert_eq!(order(0, false), ["c.bin", "b.txt", "A.txt", "zeta"]);
    assert_eq!(order(1, true), ["zeta", "A.txt", "c.bin", "b.txt"]);
    assert_eq!(order(1, false), ["b.txt", "c.bin", "A.txt", "zeta"]);
    assert_eq!(order(2, true), ["zeta", "c.bin", "A.txt", "b.txt"]);
    assert_eq!(order(3, true), ["zeta", "A.txt", "b.txt", "c.bin"]);
    assert_eq!(order(3, false), ["c.bin", "b.txt", "A.txt", "zeta"]);
}

#[test]
fn a_symlink_to_a_folder_sorts_among_the_files() {
    let link = Item {
        is_link: true,
        ..item("b link", true, 7, "Shortcut", Some(5))
    };
    let items = [
        item("c", false, 1, "Text Document", Some(30)),
        link,
        item("a", false, 1, "Text Document", Some(20)),
        item("mfolder", true, 0, "File Folder", Some(10)),
    ];
    let order = |col: usize| {
        crate::place::sort_order(&items, shellview::details_fields()[col], true)
            .into_iter()
            .map(|i| items[i].name.as_str())
            .collect::<Vec<_>>()
    };
    assert_eq!(order(0), ["mfolder", "a", "b link", "c"], "Name");
    assert_eq!(order(1), ["mfolder", "a", "c", "b link"], "Size");
    assert_eq!(order(2), ["mfolder", "b link", "a", "c"], "Type");
    assert_eq!(order(3), ["mfolder", "b link", "a", "c"], "Date Modified");
}

#[test]
fn a_symlink_to_a_folder_lists_as_a_file_and_opens_the_folder() {
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::create_dir(dir.path().join("mfolder")).expect("mkdir");
    std::fs::write(dir.path().join("a.txt"), "x").expect("write");
    std::os::unix::fs::symlink("mfolder", dir.path().join("b link")).expect("symlink");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    let names: Vec<&str> = ex.sv.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["mfolder", "a.txt", "b link"]);
    use reprisekit::controls::listview::Rows;
    let fields = shellview::details_fields();
    let rows = shellview::DetailsRows(&ex.sv.entries, &fields);
    assert_eq!(rows.cell(0, 1), "", "a folder is not sized");
    assert_eq!(rows.cell(2, 1), "1 KB", "the link is");

    ex.sv.selection.set_only(Some(2));
    ex.open_selected(&mut ctx);
    let target = std::fs::canonicalize(dir.path().join("mfolder")).expect("canonical");
    assert_eq!(ex.place, Place::Folder(target));
}

#[test]
fn one_selected_shortcut_shows_its_location_in_the_status_bar() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let real = std::fs::canonicalize(dir.path()).expect("canonical");
    std::fs::create_dir(real.join("mfolder")).expect("mkdir");
    std::fs::write(real.join("a.txt"), "x").expect("write");
    std::os::unix::fs::symlink("mfolder/a.txt", real.join("b link")).expect("symlink");
    std::os::unix::fs::symlink("/etc", real.join("c etc")).expect("symlink");
    reprisefs::apps::write_launcher(
        &real.join("d.desktop"),
        "[Desktop Entry]\nType=Application\nName=d shell\nExec=/bin/sh -l\n",
    )
    .expect("write");
    reprisefs::apps::write_launcher(
        &real.join("e.desktop"),
        "[Desktop Entry]\nType=Link\nName=e web\nURL=https://example.org/\n",
    )
    .expect("write");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(real.clone()), &mut ctx);
    let mut status = |ex: &mut Explorer, sel: &[&str]| {
        ex.sv.selection.set.clear();
        for name in sel {
            let i = ex.items.iter().position(|it| it.name == *name).expect(name);
            ex.sv.selection.set.insert(i);
        }
        ex.refresh_chrome(&mut ctx);
        ex.status.panels[0].text.clone()
    };
    let mfolder = real.join("mfolder");
    assert_eq!(
        status(&mut ex, &["b link"]),
        format!("Location: {}", mfolder.display())
    );
    assert_eq!(status(&mut ex, &["c etc"]), "Location: /");
    assert_eq!(status(&mut ex, &["d.desktop"]), "Location: /bin");
    assert_eq!(status(&mut ex, &["e.desktop"]), "https://example.org/");
    assert_eq!(status(&mut ex, &["a.txt"]), "1 object(s) selected");
    assert_eq!(
        status(&mut ex, &["b link", "c etc"]),
        "2 object(s) selected"
    );
}

#[test]
fn a_sort_keeps_the_selection_when_its_index_moves() {
    let dir = tempfile::tempdir().expect("a temp dir");
    for name in ["a.txt", "b.txt", "c.txt"] {
        std::fs::write(dir.path().join(name), "x").expect("write");
    }
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    let a = ex
        .items
        .iter()
        .position(|it| it.name == "a.txt")
        .expect("a.txt");
    ex.sv.selection.set_only(Some(a));

    ex.sv.sort = Some((0, false));
    ex.resort();

    let names: Vec<&str> = ex
        .sv
        .selection
        .set
        .iter()
        .map(|&i| ex.items[i].name.as_str())
        .collect();
    assert_eq!(names, ["a.txt"]);
    assert_eq!(
        ex.sv.selection.focus,
        Some(2),
        "a.txt sorts last descending"
    );
}

#[test]
fn the_details_section_follows_the_selection() {
    let file = item("notes.txt", false, 1260, "Text Document", None);
    let folder = item("Reports", true, 0, "File Folder", None);

    let none: Vec<&Item> = vec![];
    assert_eq!(
        window::details_text("Documents", true, &none, None, true),
        [
            ("Documents".to_string(), true),
            ("System Folder".to_string(), false)
        ]
    );
    assert_eq!(
        window::details_text("Reports", false, &none, None, true)[1],
        ("File Folder".to_string(), false)
    );

    let one = window::details_text("Documents", false, &[&file], None, true);
    let gap = (String::new(), false);
    assert_eq!(one[0], gap);
    assert_eq!(one[1], ("notes.txt".to_string(), true));
    assert_eq!(one[2], ("Text Document".to_string(), false));
    assert_eq!(one[3], gap);
    assert_eq!(one[4], ("Size: 1.23 KB".to_string(), false));
    let one = window::details_text("Documents", false, &[&folder], None, true);
    assert_eq!(one.len(), 3);
    let dated = item("a.txt", false, 0, "Text Document", Some(1_757_795_100));
    let one = window::details_text("Documents", false, &[&dated], None, true);
    assert!(one[4].0.starts_with("Date Modified: "), "{:?}", one[4]);

    let pic = item("a.png", false, 1260, "PNG Image", Some(1_757_795_100));
    let one = window::details_text("Documents", false, &[&pic], Some((800, 600)), true);
    assert_eq!(
        one[3..5],
        [gap.clone(), ("Dimensions: 800 x 600".to_string(), false)]
    );
    assert_eq!(one[6], ("Size: 1.23 KB".to_string(), false));
    assert!(one[8].0.starts_with("Date Modified: "), "{:?}", one[8]);

    let many = window::details_text("Documents", false, &[&file, &folder, &file], None, true);
    assert_eq!(many[0], ("3 items selected".to_string(), true));
    assert_eq!(many[2], ("Size: 2.46 KB".to_string(), false));
}

#[test]
fn a_rename_writes_the_file_and_keeps_it_selected() {
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::write(dir.path().join("one.txt"), "1").expect("write");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.sv.selection.set_only(Some(0));

    ex.run(Command::Rename, &mut ctx);
    let (i, ed) = ex.sv.edit.clone().expect("the box is up");
    assert_eq!(i, 0);
    assert_eq!(ed.text, "one.txt");
    assert_eq!(ed.selection(), 0.."one".len(), "the stem, not the suffix");

    ex.sv.edit = Some((0, reprisekit::textedit::TextEdit::new("two.txt", false)));
    ex.commit_rename(&mut ctx);

    assert!(ex.sv.edit.is_none());
    assert!(dir.path().join("two.txt").is_file());
    assert!(!dir.path().join("one.txt").exists());
    assert_eq!(ex.items[0].name, "two.txt");
    assert_eq!(ex.sv.selection.set.iter().copied().collect::<Vec<_>>(), [0]);
}

/// A rename whose relist lands late (a read of the folder was already out, on a mount slow to
/// answer) selects the item under its new name once the listing of it lands.
#[test]
fn a_rename_whose_relist_lands_late_keeps_the_renamed_item_selected() {
    let dir = tempfile::tempdir().expect("a temp dir");
    for name in ["b.txt", "c.txt"] {
        std::fs::write(dir.path().join(name), "x").expect("write");
    }
    let mut ctx = test_ctx();
    let place = Place::Folder(dir.path().to_path_buf());
    let mut ex = test_explorer(place.clone(), &mut ctx);
    ex.sv.selection.set_only(Some(0));
    let before = ex.reader(place.clone(), &ctx)();
    let (answer, gate) = std::sync::mpsc::channel::<()>();
    ex.reading = Some(crate::Reading {
        key: (place, ex.settings.show_hidden, ctx.icons.scale()),
        job: repriseapp::listjob::Job::spawn(None, move || {
            gate.recv().ok();
            before
        }),
        again: false,
    });

    ex.held = Some(Arc::default());
    ex.sv.edit = Some((0, reprisekit::textedit::TextEdit::new("d.txt", false)));
    ex.commit_rename(&mut ctx);
    assert!(dir.path().join("d.txt").is_file());
    answer.send(()).expect("send");
    wake_until(&mut ex, &mut ctx, |ex| ex.reading.is_none());
    let names: Vec<&str> = ex.items.iter().map(|it| it.name.as_str()).collect();
    assert_eq!(names, ["c.txt", "d.txt"]);
    assert_eq!(ex.selected_paths(), [dir.path().join("d.txt")]);
}

#[test]
fn a_colliding_rename_is_refused_with_a_box() {
    let dir = tempfile::tempdir().expect("a temp dir");
    for name in ["one.txt", "two.txt"] {
        std::fs::write(dir.path().join(name), "x").expect("write");
    }
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);

    ex.sv.edit = Some((0, reprisekit::textedit::TextEdit::new("two.txt", false)));
    ex.commit_rename(&mut ctx);

    assert!(dir.path().join("one.txt").is_file());
    match ex.dialog.as_ref().expect("the box is up") {
        crate::dialogs::Dialog::Message { title, text } => {
            assert_eq!(title, "Rename");
            assert!(text.starts_with("Cannot rename one.txt: "), "{text}");
        }
        _ => panic!("the refused rename raises a message box"),
    }
}

/// The folder watch makes the listing live, so a name free when the
/// rename box opened can be taken by the time Enter lands. The write
/// refuses it instead of replacing whatever took it.
#[test]
fn a_rename_onto_a_name_taken_since_the_listing_is_refused() {
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::write(dir.path().join("a.txt"), "mine").expect("write");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.sv.edit = Some((0, reprisekit::textedit::TextEdit::new("b.txt", false)));

    // Something else lands under the new name while the box is open;
    // the view has not re-listed, so the name still looks free.
    std::fs::write(dir.path().join("b.txt"), "someone else").expect("write");
    ex.commit_rename(&mut ctx);

    match ex.dialog.as_ref().expect("the box is up") {
        crate::dialogs::Dialog::Message { title, text } => {
            assert_eq!(title, "Rename");
            assert_eq!(
                text,
                "Cannot rename a.txt: A file with the name you specified already \
                 exists. Specify a different file name."
            );
        }
        _ => panic!("the refused rename raises a message box"),
    }
    assert_eq!(
        std::fs::read_to_string(dir.path().join("b.txt")).expect("read"),
        "someone else"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.txt")).expect("read"),
        "mine"
    );
}

#[test]
fn renaming_a_folder_relabels_its_row_in_the_tree() {
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::create_dir(dir.path().join("sub")).expect("mkdir");
    let mut ctx = test_ctx();
    let here = Place::Folder(dir.path().to_path_buf());
    let mut ex = test_explorer(here.clone(), &mut ctx);
    // `tempdir` names start with a dot, so the tree hides the whole
    // chain until hidden entries are shown.
    ex.settings.show_hidden = true;
    ex.build_tree(&mut ctx);
    ex.show(here.clone(), &mut ctx);

    let path = ex.tree_path_of(&here).expect("this folder's node");
    let labels = |ex: &Explorer, path: &[usize]| -> Vec<String> {
        ex.tree
            .node(path)
            .map(|n| n.children.iter().map(|c| c.label.clone()).collect())
            .unwrap_or_default()
    };
    assert_eq!(labels(&ex, &path), ["sub"]);

    let i = ex
        .items
        .iter()
        .position(|it| it.name == "sub")
        .expect("sub");
    ex.sv.selection.set_only(Some(i));
    ex.sv.edit = Some((i, reprisekit::textedit::TextEdit::new("renamed", false)));
    ex.commit_rename(&mut ctx);

    assert!(dir.path().join("renamed").is_dir());
    let path = ex.tree_path_of(&here).expect("this folder's node");
    assert_eq!(labels(&ex, &path), ["renamed"], "the row follows the name");
    let mut child = path.clone();
    child.push(0);
    assert_eq!(
        ex.place_of(&child),
        Some(Place::Folder(dir.path().join("renamed")))
    );
    assert_eq!(
        ex.tree_path_of(&Place::Folder(dir.path().join("sub"))),
        None
    );
}

#[test]
fn escape_abandons_a_marquee_and_a_divider_drag() {
    let dir = tempfile::tempdir().expect("a temp dir");
    for name in ["a.txt", "b.txt"] {
        std::fs::write(dir.path().join(name), "x").expect("write");
    }
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    let escape = || repriseapp::KeyInput {
        sym: reprisekit::textedit::keysym::ESCAPE,
        utf8: None,
        mods: repriseapp::Mods::default(),
        edit: None,
    };

    let base: std::collections::BTreeSet<usize> = [0].into_iter().collect();
    ex.sv.selection.set = base.clone();
    ex.marquee_from = Some(((10, 10), base.clone(), true));
    ex.sv.marquee = Some(reprisekit::raster::Rect::new(10, 10, 50, 50));
    ex.sv.selection.set = [0, 1].into_iter().collect();
    ex.key(escape(), &mut ctx);
    assert!(ex.marquee_from.is_none() && ex.sv.marquee.is_none());
    assert_eq!(ex.sv.selection.set, base, "the selection is back");

    ex.resizing = Some((0, 100, 178));
    ex.sv.columns[0].width = 218;
    ex.key(escape(), &mut ctx);
    assert!(ex.resizing.is_none());
    assert_eq!(ex.sv.columns[0].width, 178, "the width is back");
}

/// A divider dragged 40 px at 200% saves the column 20 px wider: widths are kept at 100%.
#[test]
fn a_column_drag_at_200_percent_saves_half_the_px() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ctx = test_ctx();
    ctx.theme = Theme::default().with_text_size(200);
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.resizing = Some((2, 300, 118));
    ex.pointer(repriseapp::PointerInput::Motion { x: 340, y: 50 }, &mut ctx);
    ex.pointer(
        repriseapp::PointerInput::Release {
            x: 340,
            y: 50,
            button: crate::BTN_LEFT,
            mods: repriseapp::Mods::default(),
        },
        &mut ctx,
    );
    assert_eq!(ex.sv.columns[2].width, 138);
    assert_eq!(ex.settings.columns[2], 138);
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

#[test]
fn the_command_line_takes_a_place() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let here = Place::Folder(dir.path().to_path_buf());
    let args = |a: &[&std::ffi::OsStr]| parse_args(a.iter().map(|s| s.to_os_string())).place;
    assert_eq!(args(&[]), Place::Home);
    assert_eq!(args(&[dir.path().as_os_str()]), here);
    assert_eq!(args(&["Computer".as_ref()]), Place::Computer);
    assert_eq!(args(&["Recycle Bin".as_ref()]), Place::RecycleBin);
    assert_eq!(args(&["My Computer".as_ref()]), Place::Home);
}

#[test]
fn select_opens_the_folder_on_the_entry() {
    let dir = tempfile::tempdir().expect("a temp dir");
    for i in 0..60 {
        std::fs::write(dir.path().join(format!("f{i:02}.txt")), "x").expect("write");
    }
    let last = dir.path().join("f59.txt");
    let args = |a: &[&std::ffi::OsStr]| parse_args(a.iter().map(|s| s.to_os_string()));
    let start = args(&["--select".as_ref(), last.as_os_str()]);
    assert_eq!(start.place, Place::Folder(dir.path().to_path_buf()));
    assert_eq!(start.select.as_deref(), Some(last.as_path()));
    assert_eq!(
        args(&["--select".as_ref(), "f59.txt".as_ref()]),
        Start {
            place: Place::Home,
            dir: None,
            select: None
        }
    );

    let mut ctx = test_ctx();
    let mut app = Lazy {
        start,
        inner: None,
        units: TextUnits::ONE,
    };
    let _ = app.event(MAIN, Event::Resized, &mut ctx);
    let ex = app.inner.as_mut().expect("the window");
    let i = ex.index_of(&last).expect("listed");
    assert_eq!(ex.sv.selection.set.iter().copied().collect::<Vec<_>>(), [i]);
    assert_eq!(ex.sv.selection.focus, Some(i));
    let lay = ex.layout(&ctx);
    let sl = ex.view_layout(lay.view, &ctx);
    let cell = sl.cells[i];
    assert!(
        cell.y >= sl.body.y && cell.bottom() <= sl.body.bottom(),
        "{cell:?} inside {:?}",
        sl.body
    );
}

#[test]
fn settings_changes_defer_the_write() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ctx = test_ctx();
    let ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    assert!(!ex.settings_dirty);
    let mut app = Lazy {
        start: Start {
            place: ex.place.clone(),
            dir: None,
            select: None,
        },
        inner: Some(ex),
        units: TextUnits::ONE,
    };
    let _ = app.event(MAIN, Event::Resized, &mut ctx);
    let ex = app.inner.as_mut().expect("the window");
    assert!(ex.settings_dirty, "a resize marks the settings");
    for cmd in [
        Command::Mode(reprisekit::controls::shellview::Mode::Details),
        Command::Sort(1),
        Command::ToggleFolders,
        Command::CloseBar,
        Command::ToggleStatusBar,
        Command::ToggleToolbar(0),
    ] {
        ex.flush_settings();
        let _ = ex.run(cmd.clone(), &mut ctx);
        assert!(ex.settings_dirty, "{cmd:?} marks the settings");
    }
}

/// The host tells a text size change as a resize at the old size, then regrows a window still at
/// its opening size to the size `initial_size` gives; a window the user sized keeps its size.
#[test]
fn the_window_opens_at_the_text_size_and_keeps_its_size_in_100_units() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ctx = test_ctx();
    let ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    let mut app = Lazy {
        start: Start {
            place: ex.place.clone(),
            dir: None,
            select: None,
        },
        inner: Some(ex),
        units: TextUnits::ONE,
    };
    let mut resized = |app: &mut Lazy, text_size: u32, size: (i32, i32)| {
        ctx.text_size = text_size;
        ctx.size = size;
        let _ = app.event(MAIN, Event::Resized, &mut ctx);
    };
    let saved = |app: &mut Lazy| {
        let ex = app.inner.as_mut().expect("the window");
        ex.flush_settings();
        ex.settings.window
    };
    resized(&mut app, 100, (792, 573));
    assert_eq!(saved(&mut app), (792, 573));
    resized(&mut app, 200, (792, 573));
    assert_eq!(app.initial_size(), (1584, 1146), "a picked size regrows");
    resized(&mut app, 200, (1584, 1146));
    assert_eq!(saved(&mut app), (792, 573));
    resized(&mut app, 200, (1000, 700));
    assert_eq!(saved(&mut app), (500, 350));
    assert_eq!(app.initial_size(), (1000, 700));
    resized(&mut app, 100, (1000, 700));
    assert_eq!(
        saved(&mut app),
        (1000, 700),
        "a size the user kept across the change is its 100% size"
    );
}

/// A maximized window's size is not saved; a hand size pending when it was maximized is.
#[test]
fn a_maximized_size_is_not_saved() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ctx = test_ctx();
    let ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    let mut app = Lazy {
        start: Start {
            place: ex.place.clone(),
            dir: None,
            select: None,
        },
        inner: Some(ex),
        units: TextUnits::ONE,
    };
    let mut resized = |app: &mut Lazy, size: (i32, i32), maximized: bool| {
        ctx.size = size;
        ctx.maximized = maximized;
        let _ = app.event(MAIN, Event::Resized, &mut ctx);
    };
    let saved = |app: &mut Lazy| {
        let ex = app.inner.as_mut().expect("the window");
        ex.flush_settings();
        ex.settings.window
    };
    resized(&mut app, (900, 600), false);
    resized(&mut app, (1920, 1000), true);
    assert_eq!(saved(&mut app), (900, 600), "the pending hand size");
    resized(&mut app, (1920, 1000), true);
    assert_eq!(saved(&mut app), (900, 600), "not the screen's");
    resized(&mut app, (900, 600), false);
    resized(&mut app, (700, 500), false);
    assert_eq!(saved(&mut app), (700, 500));
}

/// A size the window takes at its pick, held inside a 1080-line work area at 200% or not, keeps
/// the saved size, so the next open at 100% is the size before; a size the user gives it is saved.
#[test]
fn a_held_opening_size_keeps_the_saved_size_and_a_users_size_is_saved() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ctx = test_ctx();
    let ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    let mut app = Lazy {
        start: Start {
            place: ex.place.clone(),
            dir: None,
            select: None,
        },
        inner: Some(ex),
        units: TextUnits::ONE,
    };
    let mut resized = |app: &mut Lazy, text_size: u32, size: (i32, i32), at_pick: bool| {
        ctx.text_size = text_size;
        ctx.size = size;
        ctx.at_pick = at_pick;
        let _ = app.event(MAIN, Event::Resized, &mut ctx);
    };
    let saved = |app: &mut Lazy| {
        let ex = app.inner.as_mut().expect("the window");
        ex.flush_settings();
        ex.settings.window
    };
    let held = (1584, 1000);

    resized(&mut app, 200, (792, 573), true);
    assert_eq!(app.initial_size(), (1584, 1146));
    resized(&mut app, 200, held, true);
    assert_eq!(saved(&mut app), (792, 573), "a held pick keeps the default");
    resized(&mut app, 100, held, true);
    resized(&mut app, 100, (792, 573), true);
    assert_eq!(app.initial_size(), (792, 573));
    assert_eq!(saved(&mut app), (792, 573));

    resized(&mut app, 100, (900, 600), false);
    assert_eq!(
        saved(&mut app),
        (900, 600),
        "a size the user gives is saved"
    );
    resized(&mut app, 200, (1800, 1000), true);
    assert_eq!(app.initial_size(), (1800, 1200));
    assert_eq!(
        saved(&mut app),
        (900, 600),
        "a held pick keeps the user's size"
    );
    resized(&mut app, 200, (1400, 900), false);
    assert_eq!(
        saved(&mut app),
        (700, 450),
        "a size the user gives at 200% is saved"
    );
    resized(&mut app, 100, (1400, 900), false);
    assert_eq!(app.initial_size(), (700, 450));
}

#[test]
fn boxes_and_the_sheet_grow_with_the_text_size() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    let sizes = |ex: &mut Explorer, ctx: &mut Ctx| {
        ex.run(commands::Command::About, ctx);
        let about = ex.children()[0].size;
        ex.dialog = None;
        ex.open_properties(dir.path(), "d", "d", ctx);
        let sheet = ex.children()[0].size;
        ex.props = None;
        (about, sheet)
    };
    let (about, sheet) = sizes(&mut ex, &mut ctx);
    ctx.text_size = 150;
    ex.follow_forms(&ctx);
    assert_eq!(ex.forms.fonts.size_px, 17);
    let u = reprisekit::theme::TextUnits::new(17, 11);
    assert_eq!(sizes(&mut ex, &mut ctx), (u.size(about), u.size(sheet)));

    ex.run(commands::Command::About, &mut ctx);
    let Some(Dialog::About(form)) = &ex.dialog else {
        panic!("the About box is up");
    };
    let banner = match &form.item("banner").expect("the banner").control {
        reprisekit::controls::form::Control::Picture { canvas, .. } => canvas.as_ref(),
        _ => None,
    };
    assert_eq!(
        banner.map(|c| c.width as i32),
        Some(u.size(about).0),
        "the banner spans the grown box"
    );
}

#[test]
fn about_opens_the_shared_box_with_the_explorer_name() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.run(commands::Command::About, &mut ctx);

    let children = ex.children();
    let about = children.iter().find(|c| c.id == DIALOG).expect("the box");
    assert_eq!(about.title, "About Explorer");
    assert_eq!(about.size, reprisekit::controls::aboutbox::CLIENT);
    assert!(about.fixed && about.modal);

    let Some(Dialog::About(form)) = ex.dialog.as_ref() else {
        panic!("the About box is what is up");
    };
    let product = form.item("product").expect("the product row");
    match &product.control {
        reprisekit::controls::form::Control::Label { text, .. } => assert_eq!(text, "Explorer"),
        other => panic!("expected a label, got {other:?}"),
    }
    assert_eq!(
        form.default_button().map(|i| form.items[i].id),
        Some(reprisekit::controls::aboutbox::OK)
    );

    let esc = repriseapp::KeyInput {
        sym: reprisekit::textedit::keysym::ESCAPE,
        utf8: None,
        mods: repriseapp::Mods::default(),
        edit: None,
    };
    ex.dialog_event(Event::Key(esc), &mut ctx);
    assert!(ex.dialog.is_none());
    assert!(ex.children().iter().all(|c| c.id != DIALOG));
}

#[test]
fn a_marquee_counts_its_entries_in_the_status_bar() {
    let dir = tempfile::tempdir().expect("a temp dir");
    for name in ["a.txt", "b.txt", "c.txt"] {
        std::fs::write(dir.path().join(name), "x").expect("write");
    }
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.sv.set_mode(reprisekit::controls::shellview::Mode::List);
    let sl = ex.view_layout(ex.layout(&ctx).view, &ctx);
    let centre = |i: usize| {
        let c = sl.cells[i];
        (c.x + c.width / 2, c.y + c.height / 2)
    };
    ex.pointer(
        repriseapp::PointerInput::Press {
            x: sl.body.right() - 2,
            y: sl.body.bottom() - 2,
            button: BTN_LEFT,
            time: 5000,
            mods: repriseapp::Mods::default(),
        },
        &mut ctx,
    );
    let (x, y) = centre(1);
    ex.pointer(repriseapp::PointerInput::Motion { x, y }, &mut ctx);
    assert_eq!(ex.status.panels[0].text, "2 object(s) selected");
    let (x, y) = centre(0);
    ex.pointer(repriseapp::PointerInput::Motion { x, y }, &mut ctx);
    assert_eq!(ex.status.panels[0].text, "3 object(s) selected");
}

/// A key as the seat delivers it, with the edit key the toolkit derives
/// from it.
fn key_in(sym: u32) -> repriseapp::KeyInput {
    repriseapp::KeyInput {
        sym,
        utf8: None,
        mods: repriseapp::Mods::default(),
        edit: reprisekit::textedit::key_for(sym, false, false, None),
    }
}

/// An Explorer on a temp folder holding `a.txt` and the folder `sub`,
/// with `a.txt` selected. `tempdir` names start with a dot, so hidden
/// entries show and the picker's tree can walk down to it.
fn browse_fixture(ctx: &mut Ctx) -> (tempfile::TempDir, Explorer) {
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::write(dir.path().join("a.txt"), "x").expect("write");
    std::fs::create_dir(dir.path().join("sub")).expect("mkdir");
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), ctx);
    ex.settings.show_hidden = true;
    let a = ex
        .items
        .iter()
        .position(|it| it.name == "a.txt")
        .expect("a.txt");
    ex.sv.selection.set_only(Some(a));
    (dir, ex)
}

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

/// The picker's layout and a copy of its tree, for a test to aim at.
fn browse_state(
    ex: &Explorer,
    ctx: &Ctx,
) -> (reprisekit::controls::browsefolder::Layout, TreeView) {
    let Some(Dialog::Browse(b)) = ex.dialog.as_ref() else {
        panic!("the Browse dialog is what is up");
    };
    let lay =
        b.bf.layout(&ex.dialog_text, ex.dialog_px, &ctx.theme.controls());
    (lay, b.bf.tree.clone())
}

fn press_at(x: i32, y: i32, time: u32) -> Event {
    Event::Pointer(repriseapp::PointerInput::Press {
        x,
        y,
        button: BTN_LEFT,
        time,
        mods: repriseapp::Mods::default(),
    })
}

fn release_at(x: i32, y: i32) -> Event {
    Event::Pointer(repriseapp::PointerInput::Release {
        x,
        y,
        button: BTN_LEFT,
        mods: repriseapp::Mods::default(),
    })
}

/// A left click on the picker at (`x`, `y`): the press, then the release
/// a button acts on.
fn click_at(ex: &mut Explorer, x: i32, y: i32, ctx: &mut Ctx) -> Reply {
    ex.dialog_event(press_at(x, y, 5000), ctx);
    ex.dialog_event(release_at(x, y), ctx)
}

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

/// Wakes `ex` until `done` holds, for a read let go of on a listing thread. The read can queue
/// behind other tests' reads in the process-wide listing pool, so it gets a whole [`LIST_WAIT`].
pub(crate) fn wake_until(ex: &mut Explorer, ctx: &mut Ctx, done: impl Fn(&Explorer) -> bool) {
    let end = std::time::Instant::now() + LIST_WAIT;
    while !done(ex) {
        assert!(std::time::Instant::now() < end, "the read landed");
        std::thread::sleep(std::time::Duration::from_millis(10));
        ex.wake(ctx);
    }
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
fn a_click_in_the_folders_pane_opens_the_folder() {
    use repriseapp::PointerInput;
    let mut ctx = test_ctx();
    let dir = tempfile::tempdir().expect("a temp dir");
    for d in ["a/inner", "b/inner"] {
        std::fs::create_dir_all(dir.path().join(d)).expect("mkdir");
    }
    let here = Place::Folder(dir.path().to_path_buf());
    let mut ex = test_explorer(here.clone(), &mut ctx);
    ex.settings.show_hidden = true;
    ex.build_tree(&mut ctx);
    ex.show(here.clone(), &mut ctx);
    let (a, b) = (
        Place::Folder(dir.path().join("a")),
        Place::Folder(dir.path().join("b")),
    );
    let aim = |ex: &mut Explorer, place: &Place, ctx: &Ctx| {
        let path = ex.tree_path_of(place).expect("the folder's node");
        let i = ex
            .tree
            .visible()
            .iter()
            .position(|r| r.path == path)
            .expect("a visible row");
        ex.tree.top = i.saturating_sub(1);
        let frame = ex.layout(ctx).tree.expect("the Folders pane");
        let px = ctx.theme.fonts.size_px;
        let tl = treeview::layout(&ex.tree, &ctx.text, px, frame, &ctx.theme.controls());
        let y = tl.body.y + (i - ex.tree.top) as i32 * treeview::ROW_H + 8;
        let x = (tl.body.x..tl.body.right())
            .find(|&x| ex.tree.hit(&tl, x, y) == treeview::Hit::Row(i))
            .expect("the row's icon");
        (x + 2, y, i, path)
    };
    let open = |ex: &Explorer, path: &[usize]| ex.tree.node(path).is_some_and(|n| n.expanded);
    let press = |x, y| PointerInput::Press {
        x,
        y,
        button: BTN_LEFT,
        time: 0,
        mods: repriseapp::Mods::default(),
    };

    let (x, y, i, a_path) = aim(&mut ex, &a, &ctx);
    assert!(ex.pointer(PointerInput::Motion { x, y }, &mut ctx).redraw);
    assert_eq!(ex.tree.hot, Some(i), "a's label is hot");
    ex.pointer(press(x, y), &mut ctx);
    assert_eq!(ex.place, a, "the view went to a");
    assert!(open(&ex, &a_path), "a opened");
    let here_path = ex.tree_path_of(&here).expect("here's node");
    assert!(open(&ex, &here_path), "here holds a, so it stays open");

    let (x, y, _, b_path) = aim(&mut ex, &b, &ctx);
    ex.pointer(press(x, y), &mut ctx);
    assert_eq!(ex.place, b);
    assert!(open(&ex, &b_path), "b opened");
    assert!(!open(&ex, &a_path), "a closed");
    ex.pointer(PointerInput::Leave, &mut ctx);
    assert_eq!(ex.tree.hot, None, "nothing is hot once the pointer leaves");
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

#[test]
fn the_views_scrollbar_lights_under_the_pointer_and_a_leave_clears_it() {
    use repriseapp::PointerInput;
    use reprisekit::pointer::Interaction;
    let dir = tempfile::tempdir().expect("a temp dir");
    for i in 0..200 {
        std::fs::write(dir.path().join(format!("f{i:03}.txt")), "1").expect("write");
    }
    let mut ctx = test_ctx();
    ctx.theme = reprisekit::interfaces::theme_for(reprisekit::interface::Interface::Blue);
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    let view = ex.layout(&ctx).view;
    let sl = ex.view_layout(view, &ctx);
    let (x, y, part) = (view.y..view.bottom())
        .flat_map(|y| (view.x..view.right()).map(move |x| (x, y)))
        .find_map(|(x, y)| {
            let bar = shellview::hit(&ex.sv, &sl, x, y).bar()?;
            (bar.1 != reprisekit::classic::widgets::ScrollHit::Thumb).then_some((x, y, bar))
        })
        .expect("200 files overflow the view");
    assert!(ex.pointer(PointerInput::Motion { x, y }, &mut ctx).redraw);
    assert_eq!(ex.sv.bars.of(part), Interaction::Hot);
    let press = PointerInput::Press {
        x,
        y,
        button: crate::BTN_LEFT,
        time: 0,
        mods: repriseapp::Mods::default(),
    };
    ex.pointer(press, &mut ctx);
    assert_eq!(ex.sv.bars.of(part), Interaction::Pressed);
    ex.pointer(PointerInput::Leave, &mut ctx);
    assert_eq!(
        ex.sv.bars.of(part),
        Interaction::Pressed,
        "held through a leave"
    );
    let release = PointerInput::Release {
        x,
        y,
        button: crate::BTN_LEFT,
        mods: repriseapp::Mods::default(),
    };
    ex.pointer(release, &mut ctx);
    ex.pointer(PointerInput::Leave, &mut ctx);
    assert_eq!(ex.sv.bars.lit(), None);
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

#[test]
fn the_views_thumb_drags_and_its_track_pages() {
    use repriseapp::PointerInput;
    use reprisekit::classic::widgets::BarHit;
    let dir = tempfile::tempdir().expect("a temp dir");
    for i in 0..200 {
        std::fs::write(dir.path().join(format!("f{i:03}.txt")), "1").expect("write");
    }
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.sv.set_mode(shellview::Mode::Icons);
    let view = ex.layout(&ctx).view;
    let find = |ex: &Explorer, ctx: &Ctx, want: fn(shellview::Hit) -> bool| {
        let sl = ex.view_layout(view, ctx);
        (view.y..view.bottom())
            .flat_map(|y| (view.x..view.right()).map(move |x| (x, y)))
            .find(|&(x, y)| want(shellview::hit(&ex.sv, &sl, x, y)))
            .expect("200 files overflow the view")
    };
    let press = |x, y| PointerInput::Press {
        x,
        y,
        button: crate::BTN_LEFT,
        time: 0,
        mods: repriseapp::Mods::default(),
    };
    let release = || PointerInput::Release {
        x: 0,
        y: 0,
        button: crate::BTN_LEFT,
        mods: repriseapp::Mods::default(),
    };
    let (x, y) = find(&ex, &ctx, |h| {
        h == shellview::Hit::VBar(BarHit::PageForward)
    });
    ex.pointer(press(x, y), &mut ctx);
    let paged = ex.sv.top;
    assert!(paged > 0, "the track below the thumb pages on");
    ex.pointer(release(), &mut ctx);
    let (x, y) = find(&ex, &ctx, |h| {
        matches!(h, shellview::Hit::VBar(BarHit::Thumb(_)))
    });
    ex.pointer(press(x, y), &mut ctx);
    assert_eq!(ex.sv.top, paged, "the press on the thumb moves nothing");
    ex.pointer(PointerInput::Motion { x: 0, y: 9000 }, &mut ctx);
    let end = ex.sv.top;
    assert!(end > paged, "dragged past the end, off the view");
    ex.pointer(PointerInput::Motion { x, y: view.y }, &mut ctx);
    assert!(ex.sv.top < end, "and back up");
    ex.pointer(release(), &mut ctx);
    let after = ex.sv.top;
    ex.pointer(PointerInput::Motion { x, y: 9000 }, &mut ctx);
    assert_eq!(ex.sv.top, after, "let go: the motion moves nothing");
}

#[test]
fn a_details_column_lights_under_the_pointer() {
    use repriseapp::PointerInput;
    use reprisekit::pointer::Interaction;
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::write(dir.path().join("a.txt"), "1").expect("write");
    let mut ctx = test_ctx();
    ctx.theme = reprisekit::interfaces::theme_for(reprisekit::interface::Interface::Blue);
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.sv.set_mode(shellview::Mode::Details);
    let view = ex.layout(&ctx).view;
    let sl = ex.view_layout(view, &ctx);
    let (x, y) = (view.y..view.bottom())
        .flat_map(|y| (view.x..view.right()).map(move |x| (x, y)))
        .find(|&(x, y)| shellview::hit(&ex.sv, &sl, x, y) == shellview::Hit::Header(1))
        .expect("a second column");
    assert!(ex.pointer(PointerInput::Motion { x, y }, &mut ctx).redraw);
    assert_eq!(ex.sv.header.of(1), Interaction::Hot);
    let press = PointerInput::Press {
        x,
        y,
        button: crate::BTN_LEFT,
        time: 0,
        mods: repriseapp::Mods::default(),
    };
    ex.pointer(press, &mut ctx);
    assert_eq!(ex.sv.header.of(1), Interaction::Pressed);
    let release = PointerInput::Release {
        x,
        y,
        button: crate::BTN_LEFT,
        mods: repriseapp::Mods::default(),
    };
    ex.pointer(release, &mut ctx);
    assert_eq!(
        ex.sv.header.of(1),
        Interaction::Hot,
        "hot again after the release"
    );
    ex.pointer(PointerInput::Leave, &mut ctx);
    assert_eq!(ex.sv.header.lit(), None);
}

#[test]
fn thumb_buttons_step_back_and_forward() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let sub = dir.path().join("sub");
    std::fs::create_dir(&sub).expect("mkdir");
    let mut ctx = test_ctx();
    let top = Place::Folder(dir.path().to_path_buf());
    let mut ex = test_explorer(top.clone(), &mut ctx);
    let _ = ex.run(Command::GoTo(Place::Folder(sub.clone())), &mut ctx);
    let thumb = |button| repriseapp::PointerInput::Press {
        x: 0,
        y: 0,
        button,
        time: 0,
        mods: repriseapp::Mods::default(),
    };
    ex.pointer(thumb(0x113), &mut ctx);
    assert_eq!(ex.place, top, "Back returns to the folder above");
    ex.pointer(thumb(0x114), &mut ctx);
    assert_eq!(ex.place, Place::Folder(sub), "Forward returns to sub");
}

#[test]
fn a_launcher_lists_runs_and_renames_by_its_name() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("a.foot.desktop");
    reprisefs::apps::write_launcher(
        &path,
        "[Desktop Entry]\nType=Application\nName=Terminal\nExec=foot\nIcon=utilities-terminal\n",
    )
    .expect("write");
    std::fs::write(dir.path().join("notes.txt"), "x").expect("write");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    let names: Vec<&str> = ex.sv.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["notes.txt", "Terminal"], "sorted by the shown name");
    assert!(ex.sv.entries[1].link, "the shortcut arrow");
    assert_eq!(
        reprisefs::icons::item_icon(&ex.items[1], &[], &|_| true),
        "utilities-terminal"
    );

    ex.sv.selection.set_only(Some(1));
    let reply = ex.open_selected(&mut ctx);
    assert_eq!(reply.spawn, [vec![std::ffi::OsString::from("foot")]]);

    ex.properties(&mut ctx);
    let p = ex.props.as_ref().expect("the sheet");
    assert_eq!(p.title(), "Terminal Properties");
    assert_eq!(
        reprisekit::controls::filesheet::typed_name(p.general()),
        "a.foot.desktop",
        "the field holds the file name"
    );
    ex.props = None;

    ex.run(Command::Rename, &mut ctx);
    let (_, ed) = ex.sv.edit.clone().expect("the box is up");
    assert_eq!(ed.text, "Terminal");
    assert!(ed.all_selected());
    ex.sv.edit = Some((1, reprisekit::textedit::TextEdit::new("Shell", false)));
    ex.commit_rename(&mut ctx);
    assert!(path.is_file(), "the file keeps its name");
    let text = std::fs::read_to_string(&path).expect("read");
    assert!(text.contains("Name=Shell\n"), "{text}");
    assert_eq!(ex.sv.entries[1].name, "Shell");
}

#[test]
fn under_glass_the_closing_line_sits_above_the_list_header() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ctx = test_ctx();
    ctx.theme = reprisekit::interfaces::theme_for(reprisekit::interface::Interface::Glass);
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.sv.mode = shellview::Mode::Details;
    let mut c = Canvas::new(792, 573);
    window::draw(&ex, &mut c, &ctx);
    let lay = ex.layout(&ctx);
    let header = ex.view_layout(lay.view, &ctx).body;
    let x = header.x + 4;
    let style = ctx.theme.look.style().expect("Glass has a style");
    let rebar = style.part(reprisekit::style::Part::Rebar).expect("a rebar");
    let line = rebar.atlas.get(0, rebar.atlas.height as i32 - 1);
    assert_eq!(
        c.get(x, lay.etch_y),
        line,
        "closing line at y {}",
        lay.etch_y
    );
    assert_eq!(header.y, lay.etch_y + 1, "the header starts under the line");
    assert_ne!(c.get(x, header.y), line, "the header's top row");
}

#[test]
fn under_glass_the_rebar_is_clear_over_the_frames_glass() {
    use reprisekit::interface::Interface;
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ctx = test_ctx();
    ctx.size = (792, 573);
    for look in [Interface::Classic, Interface::Blue] {
        ctx.theme = reprisekit::interfaces::theme_for(look);
        let ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
        assert_eq!(ex.glass_top(&ctx), 0, "{look:?} keeps its rebar opaque");
    }
    ctx.theme = reprisekit::interfaces::theme_for(Interface::Glass);
    let ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    let lay = ex.layout(&ctx);
    assert_eq!(ex.glass_top(&ctx), 78, "menu, toolbar and address bands");
    assert_eq!(lay.etch_y, 78, "the closing line sits under the glass");
    let mut c = Canvas::new(792, 573);
    window::draw(&ex, &mut c, &ctx);
    assert_eq!(c.take_glass_top(), 78, "the drawn rebar reports its rows");
    let gap = lay.menu.bottom();
    assert_eq!(c.get(400, gap) >> 24, 0, "the rows between bands are clear");
    assert_eq!(c.get(780, 40) >> 24, 0, "the toolbar band past its buttons");
    let field = lay.address.expect("the address bar");
    assert_eq!(
        c.get(field.x + field.width / 2, field.y + field.height / 2) >> 24,
        0xFF,
        "the address field stays opaque"
    );
    let x = field.x + field.width / 2;
    let above = (0..field.y + field.height / 2)
        .rev()
        .find(|&y| c.get(x, y) >> 24 != 0xFF)
        .expect("the field starts below the toolbar");
    assert_eq!(
        [c.get(x, above) >> 24, c.get(x, above - 1) >> 24],
        [0, 0],
        "no glow over the field's border"
    );
    assert_eq!(c.get(400, lay.etch_y) >> 24, 0xFF, "the closing line");
}

#[test]
fn computer_gains_and_loses_a_locked_stick_as_udisks_reports_it() {
    use reprisefs::udisks::{Disk, Snapshot, Volume};
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Computer, &mut ctx);
    let stick = PathBuf::from("/dev/sdx");
    let listed = |ex: &Explorer| ex.items.iter().any(|it| it.path == stick);
    let plugged = Snapshot {
        disks: vec![Disk {
            path: "/drives/stick".into(),
            removable: true,
            ..Default::default()
        }],
        volumes: vec![Volume {
            path: "/block/sdx".into(),
            device: "/dev/sdx".into(),
            disk: Some("/drives/stick".into()),
            label: Some("VAULT".into()),
            fs_type: "crypto_LUKS".into(),
            encrypted: true,
            ..Default::default()
        }],
    };

    let reply = ex.volumes_changed(plugged.clone(), &mut ctx);
    assert!(reply.redraw);
    assert!(listed(&ex), "the locked stick is listed once plugged in");

    let reply = ex.volumes_changed(plugged, &mut ctx);
    assert_eq!(
        reply,
        Reply::none(),
        "an unchanged snapshot re-lists nothing"
    );

    ex.volumes_changed(Snapshot::default(), &mut ctx);
    assert!(!listed(&ex), "the stick's row goes once it is pulled out");
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

/// Opening an executable script runs it in its own folder; a +x picture and a script without
/// the execute bit open in their programs, and a script whose interpreter is missing gets a box.
#[test]
fn opening_an_executable_script_runs_it_in_its_folder() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().expect("a temp dir");
    let put = |name: &str, bytes: &[u8], mode: u32| {
        let p = dir.path().join(name);
        std::fs::write(&p, bytes).expect("write");
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode)).expect("chmod");
        p
    };
    put("a.sh", b"#!/bin/sh\ntouch ran\n", 0o755);
    put("b.sh", b"#!/no/such/shell\n", 0o755);
    let photo = put("c.png", b"\x89PNG\r\n\x1a\n", 0o755);
    let plain = put("d.sh", b"#!/bin/sh\ntouch ran\n", 0o644);
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    let names: Vec<&str> = ex.sv.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["a.sh", "b.sh", "c.png", "d.sh"]);
    let mut open = |i: usize, ex: &mut Explorer| {
        ex.sv.selection.set_only(Some(i));
        ex.open_selected(&mut ctx)
    };

    assert!(open(0, &mut ex).spawn.is_empty(), "run, not opened");
    let marker = dir.path().join("ran");
    let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !marker.exists() {
        assert!(
            std::time::Instant::now() < end,
            "the script ran in its folder"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(ex.dialog.is_none());
    let state = std::env::temp_dir().join(format!("reprise-explorer-tests-{}", std::process::id()));
    let term_argv = std::fs::read_to_string(state.join("term-argv")).expect("the terminal ran");
    let a_sh = dir.path().join("a.sh");
    let term_argv: Vec<&str> = term_argv.lines().collect();
    assert_eq!(
        term_argv[..3],
        ["-e", "sh", "-c"],
        "the config's terminal keeps it open"
    );
    assert_eq!(
        term_argv.last(),
        a_sh.to_str().as_ref(),
        "the script is the last argument"
    );

    open(1, &mut ex);
    match ex.dialog.take().expect("the error box is up") {
        crate::dialogs::Dialog::Message { title, text } => {
            assert_eq!(title, "b.sh");
            assert_eq!(
                text,
                "Cannot run b.sh: The program that runs it was not found."
            );
        }
        _ => panic!("a failed run raises a message box"),
    }

    for (i, path) in [(2, photo), (3, plain)] {
        let spawn = open(i, &mut ex).spawn;
        assert_eq!(spawn.len(), 1);
        assert_eq!(spawn[0].last(), Some(&path.into_os_string()), "opened");
    }
}

/// Opening a program file reads it on a listing thread: on a dead mount the window goes on at
/// once, and a run that fails raises its box on the wake after the answer.
#[test]
fn a_program_that_answers_late_raises_its_box_on_a_later_wake() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().expect("a temp dir");
    let p = dir.path().join("b.sh");
    std::fs::write(
        &p,
        b"#!/no/such/shell
",
    )
    .expect("write");
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.sv.selection.set_only(Some(0));

    let gate = Arc::new(std::sync::RwLock::new(()));
    let closed = gate.write().expect("lock");
    ex.held = Some(gate.clone());
    assert_eq!(ex.open_selected(&mut ctx), Reply::none());
    assert!(ex.dialog.is_none(), "no box before the answer");

    drop(closed);
    ex.held = None;
    wake_until(&mut ex, &mut ctx, |ex| ex.dialog.is_some());
    match ex.dialog.take().expect("the error box is up") {
        crate::dialogs::Dialog::Message { title, text } => {
            assert_eq!(title, "b.sh");
            assert_eq!(
                text,
                "Cannot run b.sh: The program that runs it was not found."
            );
        }
        _ => panic!("a failed run raises a message box"),
    }
}

/// Whether `reply` spawns something that opens `path`.
fn opens(reply: &Reply, path: &std::path::Path) -> bool {
    reply
        .spawn
        .iter()
        .any(|argv| argv.iter().any(|a| std::path::Path::new(a) == path))
}

#[test]
fn a_file_opened_late_still_opens_when_the_window_closes_first() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let p = dir.path().join("a.txt");
    std::fs::write(&p, "text").expect("write");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.sv.selection.set_only(Some(0));

    let gate = Arc::new(std::sync::RwLock::new(()));
    let closed = gate.write().expect("lock");
    ex.held = Some(gate.clone());
    assert_eq!(ex.open_selected(&mut ctx), Reply::none());
    assert!(!ex.close().quit, "the window waits for the opening");

    drop(closed);
    ex.held = None;
    let mut spawned = false;
    for _ in 0..500 {
        let r = ex.wake(&mut ctx);
        spawned |= opens(&r, &p);
        if r.quit {
            assert!(spawned, "the file opens before the window quits");
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("the window quits once the opening answers");
}

#[test]
fn a_file_opened_late_opens_when_another_answer_lands_in_the_same_wake() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let p = dir.path().join("a.txt");
    std::fs::write(&p, "text").expect("write");
    let sub = dir.path().join("sub");
    std::fs::create_dir(&sub).expect("mkdir");
    let mut ctx = test_ctx();
    let here = Place::Folder(dir.path().to_path_buf());
    let mut ex = test_explorer(here.clone(), &mut ctx);
    let at = ex
        .items
        .iter()
        .position(|it| it.path == p)
        .expect("a.txt listed");
    ex.sv.selection.set_only(Some(at));

    let gate = Arc::new(std::sync::RwLock::new(()));
    let closed = gate.write().expect("lock");
    ex.held = Some(gate.clone());
    assert_eq!(ex.open_selected(&mut ctx), Reply::none());
    ex.held = None;
    let typed = gate.clone();
    let reply = ex.navigate(
        &sub.display().to_string(),
        move |d| {
            drop(typed.read());
            d.is_dir()
        },
        &mut ctx,
    );
    assert_eq!(reply, Reply::none());

    drop(closed);
    // Both answers land before the wake that takes them.
    std::thread::sleep(std::time::Duration::from_millis(500));
    let mut spawned = false;
    for _ in 0..500 {
        spawned |= opens(&ex.wake(&mut ctx), &p);
        if ex.place != here && spawned {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(ex.place, Place::Folder(sub));
    assert!(spawned, "the file opens in its program");
}

/// A command-line folder whose check has not answered opens the window there at once; a late
/// "missing" sends it Home with nothing to go back to.
#[test]
fn a_start_folder_answering_late_opens_the_window_at_once() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let gone = dir.path().join("gone");
    let mut ctx = test_ctx();
    let start = parse_args([gone.clone().into_os_string()]);
    assert_eq!(start.dir.as_deref(), Some(gone.as_path()));
    let (answer, gate) = std::sync::mpsc::channel::<()>();

    let opened = std::time::Instant::now();
    let mut ex = Explorer::open(
        start,
        move |d| {
            gate.recv_timeout(std::time::Duration::from_secs(20)).ok();
            d.is_dir()
        },
        test_explorer,
        &mut ctx,
    );
    assert!(
        opened.elapsed() < std::time::Duration::from_secs(10),
        "the window opens before the folder answers"
    );
    assert_eq!(ex.place, Place::Folder(gone));

    answer.send(()).expect("send");
    wake_until(&mut ex, &mut ctx, |ex| ex.place == Place::Home);
    assert_eq!(ex.place, Place::Home);
    assert!(!ex.history.can_back());
}

#[test]
fn a_start_folder_that_answers_opens_there_and_selects_its_entry() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let file = dir.path().join("a.txt");
    std::fs::write(&file, "x").expect("write");
    let mut ctx = test_ctx();
    let open = |a: &[&std::ffi::OsStr], ctx: &mut Ctx| {
        let start = parse_args(a.iter().map(|s| s.to_os_string()));
        Explorer::open(start, |d| d.is_dir(), test_explorer, ctx)
    };

    let ex = open(&["--select".as_ref(), file.as_os_str()], &mut ctx);
    assert_eq!(ex.place, Place::Folder(dir.path().to_path_buf()));
    let i = ex.index_of(&file).expect("listed");
    assert_eq!(ex.sv.selection.set.iter().copied().collect::<Vec<_>>(), [i]);

    let missing = open(&[dir.path().join("gone").as_os_str()], &mut ctx);
    assert_eq!(missing.place, Place::Home);
    assert!(missing.checking.is_none());
}

#[test]
fn the_dialogs_take_the_renderer_a_font_change_loads() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ctx.text = TextRenderer::load(&FontConfig {
        family: vec!["DejaVu Sans".into()],
        ..FontConfig::default()
    })
    .into();
    ex.follow_forms(&ctx);
    assert!(Rc::ptr_eq(&ex.dialog_text, &ctx.text));
}

/// The drive scan stats every mount on a listing thread: on a mount that does not answer the
/// window keeps the drives it has, and takes the scan on the wake after it lands.
#[test]
fn a_drive_scan_that_answers_late_lands_on_a_later_wake() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    let scanned = reprisefs::places::drives();
    ex.drives.clear();
    ex.mounts_seen.clear();

    let gate = Arc::new(std::sync::RwLock::new(()));
    let closed = gate.write().expect("lock");
    ex.held = Some(gate.clone());
    ex.refresh(&mut ctx);
    assert!(ex.drives.is_empty(), "the drives wait for the scan");
    assert!(ex.scanning.is_some());
    ex.refresh(&mut ctx);
    drop(closed);
    ex.held = None;
    wake_until(&mut ex, &mut ctx, |ex| ex.scanning.is_none());
    assert!(
        !ex.drives.is_empty() || scanned.is_empty(),
        "the scan landed"
    );
    assert!(
        !ex.mounts_seen.is_empty(),
        "the mount table it scanned is kept"
    );
}

/// The opening drive scan runs on a listing thread too: a window opened while a mount does not
/// answer lists its folder and takes the drives on the wake after the scan lands.
#[test]
fn the_opening_drive_scan_that_answers_late_lands_on_a_later_wake() {
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::write(dir.path().join("one.txt"), "1").expect("write");
    let mut ctx = test_ctx();
    let scanned = reprisefs::places::drives();
    let start = Place::Folder(dir.path().to_path_buf());
    let mut ex = Explorer::unopened(
        start.clone(),
        Settings::default(),
        Vec::new(),
        None,
        &mut ctx,
    );

    let gate = Arc::new(std::sync::RwLock::new(()));
    let closed = gate.write().expect("lock");
    ex.held = Some(gate.clone());
    ex.first_show(start, &mut ctx);
    assert!(ex.drives.is_empty(), "the drives wait for the scan");
    assert!(ex.scanning.is_some());
    wake_until(&mut ex, &mut ctx, |ex| ex.items.len() == 1);
    drop(closed);
    ex.held = None;
    wake_until(&mut ex, &mut ctx, |ex| ex.scanning.is_none());
    let mounts =
        |d: &[reprisefs::places::Drive]| d.iter().map(|d| d.mount.clone()).collect::<Vec<_>>();
    assert_eq!(
        mounts(&ex.drives),
        mounts(&scanned),
        "free space moves between two scans"
    );
}
