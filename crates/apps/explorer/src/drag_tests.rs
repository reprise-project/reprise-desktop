//! `drag`'s tests: arming and starting a drag through synthetic pointer
//! events, the window as a target, the drops on a temporary folder, the drop
//! menu and the drag image. Kept out of `drag.rs` to keep it under 800 lines.

use super::*;
use crate::BTN_RIGHT;
use crate::tests::test_ctx;
use repriseapp::Mods as SeatMods;
use repriseapp::PointerInput;

/// A folder with a file and two subfolders, shown in Icons.
fn explorer(ctx: &mut Ctx) -> (tempfile::TempDir, Explorer) {
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::write(dir.path().join("a.txt"), "hello").expect("write");
    std::fs::create_dir(dir.path().join("Box")).expect("mkdir");
    std::fs::create_dir(dir.path().join("Crate")).expect("mkdir");
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), ctx);
    ex.sv.mode = Mode::Icons;
    (dir, ex)
}

fn index_of(ex: &Explorer, name: &str) -> usize {
    ex.items
        .iter()
        .position(|it| it.name == name)
        .unwrap_or_else(|| panic!("{name} is not in the listing"))
}

fn cell(ex: &Explorer, ctx: &Ctx, i: usize) -> Rect {
    let lay = ex.layout(ctx);
    shellview::layout(&ex.sv, lay.view, &ctx.theme.controls()).cells[i]
}

fn centre(ex: &Explorer, ctx: &Ctx, i: usize) -> (i32, i32) {
    let c = cell(ex, ctx, i);
    (c.x + c.width / 2, c.y + c.height / 2)
}

fn background(ex: &Explorer, ctx: &Ctx) -> (i32, i32) {
    let v = ex.layout(ctx).view;
    (v.x + v.width - 30, v.y + v.height - 30)
}

fn press(ex: &mut Explorer, ctx: &mut Ctx, at: (i32, i32), button: u32) {
    press_with(ex, ctx, at, button, SeatMods::default());
}

fn press_with(ex: &mut Explorer, ctx: &mut Ctx, at: (i32, i32), button: u32, mods: SeatMods) {
    ex.pointer(
        PointerInput::Press {
            x: at.0,
            y: at.1,
            button,
            time: 5000,
            mods,
        },
        ctx,
    );
}

fn ctrl() -> SeatMods {
    SeatMods {
        ctrl: true,
        shift: false,
        alt: false,
    }
}

fn selected(ex: &Explorer) -> Vec<String> {
    ex.sv
        .selection
        .set
        .iter()
        .filter_map(|&i| ex.items.get(i).map(|it| it.name.clone()))
        .collect()
}

fn select(ex: &mut Explorer, names: &[&str]) {
    for (n, name) in names.iter().enumerate() {
        let i = index_of(ex, name);
        ex.sv.selection.click(
            i,
            Mods {
                ctrl: n > 0,
                shift: false,
            },
        );
    }
}

fn motion(ex: &mut Explorer, ctx: &mut Ctx, at: (i32, i32)) -> Reply {
    ex.pointer(PointerInput::Motion { x: at.0, y: at.1 }, ctx)
}

fn release(ex: &mut Explorer, ctx: &mut Ctx, at: (i32, i32), button: u32, mods: SeatMods) {
    ex.pointer(
        PointerInput::Release {
            x: at.0,
            y: at.1,
            button,
            mods,
        },
        ctx,
    );
}

fn start(ex: &mut Explorer, ctx: &mut Ctx, from: &str, button: u32) -> Reply {
    let at = centre(ex, ctx, index_of(ex, from));
    press(ex, ctx, at, button);
    motion(ex, ctx, (at.0 + THRESHOLD + 1, at.1))
}

const FILES: Offered = Offered {
    files: true,
    link: false,
    text: true,
};

/// A drop of `paths` at `at` from a source allowing every action, then the put run to its end.
fn drop_at(
    ex: &mut Explorer,
    ctx: &mut Ctx,
    at: (i32, i32),
    paths: Vec<PathBuf>,
    action: DragAction,
) -> Reply {
    let reply = ex.drag_drop(at, Dropped::Files(paths), action, Actions::ALL, ctx);
    ex.finish_puts(ctx);
    reply
}

/// Our own drag of `from` dropped on `onto` with `action`, as the compositor delivers it.
fn drag_onto(ex: &mut Explorer, ctx: &mut Ctx, from: &str, onto: &str, action: DragAction) {
    start(ex, ctx, from, BTN_LEFT);
    let end = centre(ex, ctx, index_of(ex, onto));
    ex.drag_enter(end.0, end.1, FILES, true, ctx);
    let paths = ex.drag.as_ref().expect("dragging").paths.clone();
    drop_at(ex, ctx, end, paths, action);
    ex.drag_ended();
}

/// The press arms; past the threshold the drag goes to the compositor with
/// the selection's paths and an image held where the press landed.
#[test]
fn the_threshold_hands_the_drag_to_the_compositor() {
    let mut ctx = test_ctx();
    let (_dir, mut ex) = explorer(&mut ctx);
    let file = index_of(&ex, "a.txt");
    let at = centre(&ex, &ctx, file);

    press(&mut ex, &mut ctx, at, BTN_LEFT);
    let d = ex.drag.as_ref().expect("the press armed a drag");
    assert!(!d.started);
    assert_eq!(d.items, [file]);

    let r = motion(&mut ex, &mut ctx, (at.0 + THRESHOLD - 1, at.1));
    assert!(r.start_drag.is_none());

    let r = motion(&mut ex, &mut ctx, (at.0 + THRESHOLD + 1, at.1));
    let out = r.start_drag.expect("the drag started");
    assert_eq!(out.paths, [ex.items[file].path.clone()]);
    assert_eq!((out.image.width, out.image.height), (75, 75));
    let c = cell(&ex, &ctx, file);
    assert_eq!(out.grab, (at.0 - c.x, at.1 - c.y));
    assert!(ex.drag.as_ref().expect("still ours").started);

    let r = motion(&mut ex, &mut ctx, (at.0 + 40, at.1));
    assert!(r.start_drag.is_none());
    ex.drag_ended();
    assert!(ex.drag.is_none());
}

/// The compositor taking the pointer as the drag starts doesn't end it; before the threshold it disarms.
#[test]
fn only_an_unstarted_drag_ends_when_the_pointer_leaves() {
    let mut ctx = test_ctx();
    let (_dir, mut ex) = explorer(&mut ctx);
    start(&mut ex, &mut ctx, "a.txt", BTN_LEFT);
    ex.pointer(PointerInput::Leave, &mut ctx);
    assert!(ex.drag.as_ref().is_some_and(|d| d.started));

    let (_dir, mut ex) = explorer(&mut ctx);
    let at = centre(&ex, &ctx, index_of(&ex, "a.txt"));
    press(&mut ex, &mut ctx, at, BTN_LEFT);
    ex.pointer(PointerInput::Leave, &mut ctx);
    assert!(ex.drag.is_none());
}

/// Over the window, only a folder not being dragged lights up; a file is no
/// target, and the background takes the drag for the folder shown.
#[test]
fn a_drag_over_the_window_lights_only_a_folder_it_can_enter() {
    let mut ctx = test_ctx();
    let (_dir, mut ex) = explorer(&mut ctx);
    let (file, box_, crate_) = (
        index_of(&ex, "a.txt"),
        index_of(&ex, "Box"),
        index_of(&ex, "Crate"),
    );
    start(&mut ex, &mut ctx, "Box", BTN_LEFT);
    let own = centre(&ex, &ctx, box_);
    let r = ex.drag_enter(own.0, own.1, FILES, true, &ctx);
    assert_eq!(r.drag_accept, Some(None), "a drag is not its own target");
    assert_eq!(ex.sv.drop_target, None);

    let at = centre(&ex, &ctx, file);
    let r = ex.drag_over(at.0, at.1, &ctx);
    assert_eq!(r.drag_accept, Some(None), "a file takes no drop");

    let at = centre(&ex, &ctx, crate_);
    let r = ex.drag_over(at.0, at.1, &ctx);
    let a = r.drag_accept.flatten().expect("the other folder takes it");
    assert_eq!(a.kind, DropKind::Uris);
    assert_eq!(a.preferred, Some(DragAction::Move), "a move inside a drive");
    assert_eq!(ex.sv.drop_target, Some(crate_));

    let at = background(&ex, &ctx);
    let r = ex.drag_over(at.0, at.1, &ctx);
    assert!(
        r.drag_accept.flatten().is_some(),
        "the folder shown takes it"
    );
    assert_eq!(ex.sv.drop_target, None, "and lights nothing");

    ex.drag_leave();
    assert!(ex.incoming.is_none() && ex.sv.drop_target.is_none());
}

/// Another program's drag is a copy until its files are known, then by where they live; plain text is refused.
#[test]
fn another_programs_drag_is_answered_once_its_files_arrive() {
    let mut ctx = test_ctx();
    let (dir, mut ex) = explorer(&mut ctx);
    let at = background(&ex, &ctx);
    let r = ex.drag_enter(at.0, at.1, FILES, false, &ctx);
    let a = r.drag_accept.flatten().expect("taken");
    assert_eq!(a.preferred, Some(DragAction::Copy));
    assert!(
        a.actions.copy && a.actions.ask && !a.actions.move_,
        "no move before the files are known: it may be links only"
    );

    let other = tempfile::tempdir_in(dir.path()).expect("a temp dir");
    let r = ex.drag_files(vec![other.path().join("x.txt")], &ctx);
    let a = r.drag_accept.flatten().expect("taken");
    assert_eq!(a.preferred, Some(DragAction::Move));

    // A folder dragged in from elsewhere cannot go into itself: the window shows `dir`.
    let r = ex.drag_files(vec![dir.path().to_path_buf()], &ctx);
    assert!(r.drag_accept.flatten().is_some(), "the first list stands");
    ex.drag_leave();
    ex.drag_enter(at.0, at.1, FILES, false, &ctx);
    let r = ex.drag_files(vec![dir.path().to_path_buf()], &ctx);
    assert_eq!(r.drag_accept, Some(None));

    let text = Offered {
        files: false,
        link: false,
        text: true,
    };
    let r = ex.drag_enter(at.0, at.1, text, false, &ctx);
    assert_eq!(r.drag_accept, Some(None), "text is refused");
}

#[test]
fn what_a_drag_drops_as() {
    let link = Offered {
        files: true,
        link: true,
        text: true,
    };
    assert_eq!(drop_kind(link), Some(DropKind::Uris));
    assert_eq!(drop_kind(FILES), Some(DropKind::Uris));
    let text = Offered {
        text: true,
        ..Offered::default()
    };
    assert_eq!(drop_kind(text), None);
    // A browser's link is only a copy, as is a URI list whose files came back empty.
    let copy = Actions {
        copy: true,
        move_: false,
        ask: false,
    };
    let tmp = Path::new("/tmp");
    let x = [PathBuf::from("/tmp/x")];
    assert_eq!(accept_for(link, None, tmp).actions, copy);
    let a = accept_for(link, Some(&x), tmp);
    assert_eq!(a.actions, copy);
    assert_eq!(a.preferred, Some(DragAction::Copy));
    assert_eq!(accept_for(FILES, Some(&[]), tmp).actions, copy);
    assert_eq!(accept_for(FILES, Some(&x), tmp).actions, Actions::ALL);
}

#[test]
fn a_plain_drop_moves_only_when_every_file_is_on_the_drive() {
    let home = tempfile::tempdir().expect("a temp dir");
    let here = home.path().join("x");
    std::fs::write(&here, "x").expect("write");
    let dir = home.path();
    let a = accept_for(FILES, Some(std::slice::from_ref(&here)), dir);
    assert_eq!(a.preferred, Some(DragAction::Move));
    let mixed = [here.clone(), PathBuf::from("/proc/version")];
    assert!(!all_on_device(&mixed, dir));
    assert_eq!(
        accept_for(FILES, Some(&mixed), dir).preferred,
        Some(DragAction::Copy)
    );
    assert!(!all_on_device(&[], dir));
}

#[test]
fn a_move_onto_a_folder_moves_the_file() {
    let mut ctx = test_ctx();
    let (dir, mut ex) = explorer(&mut ctx);

    drag_onto(&mut ex, &mut ctx, "a.txt", "Box", DragAction::Move);

    assert!(dir.path().join("Box/a.txt").is_file(), "it moved in");
    assert!(!dir.path().join("a.txt").exists(), "and left the folder");
    assert!(ex.drag.is_none() && ex.sv.drop_target.is_none());
    assert!(ex.dialog.is_none(), "nothing went wrong");
    assert_eq!(ex.undo, Some("Move"));
    assert!(!ex.items.iter().any(|it| it.name == "a.txt"));
}

#[test]
fn a_copy_onto_a_folder_leaves_the_original() {
    let mut ctx = test_ctx();
    let (dir, mut ex) = explorer(&mut ctx);

    drag_onto(&mut ex, &mut ctx, "a.txt", "Box", DragAction::Copy);

    assert!(dir.path().join("Box/a.txt").is_file());
    assert!(dir.path().join("a.txt").is_file(), "the original stays");
    assert_eq!(ex.undo, Some("Copy"));
}

/// Dropped on its own folder, a move does nothing and a copy lands beside the original.
#[test]
fn only_a_move_into_the_folder_it_came_from_does_nothing() {
    let mut ctx = test_ctx();
    let (dir, mut ex) = explorer(&mut ctx);
    let bg = background(&ex, &ctx);
    let paths = vec![dir.path().join("a.txt")];
    drop_at(&mut ex, &mut ctx, bg, paths.clone(), DragAction::Move);
    assert!(ex.undo.is_none(), "the move did something");
    assert_eq!(std::fs::read_dir(dir.path()).expect("list").count(), 3);

    drop_at(&mut ex, &mut ctx, bg, paths.clone(), DragAction::Copy);
    assert_eq!(ex.undo, Some("Copy"));
    assert_eq!(std::fs::read_dir(dir.path()).expect("list").count(), 4);

    // Move Here from the menu does nothing too, and tells the source it copied.
    ex.drag_drop(
        bg,
        Dropped::Files(paths),
        DragAction::Ask,
        Actions::ALL,
        &mut ctx,
    );
    let r = ex.run(Command::Drop(DropOp::Move), &mut ctx);
    assert_eq!(r.answer_drop, Some(Some(DragAction::Copy)));
    ex.finish_puts(&mut ctx);
    assert_eq!(std::fs::read_dir(dir.path()).expect("list").count(), 4);
}

/// Files from elsewhere dropped on the background land in the folder shown, selected, as a paste leaves them.
#[test]
fn a_drop_on_the_background_lands_in_the_folder_shown() {
    let mut ctx = test_ctx();
    let (dir, mut ex) = explorer(&mut ctx);
    let other = tempfile::tempdir().expect("a temp dir");
    let src = other.path().join("far.txt");
    std::fs::write(&src, "x").expect("write");

    let bg = background(&ex, &ctx);
    drop_at(&mut ex, &mut ctx, bg, vec![src.clone()], DragAction::Copy);

    assert!(dir.path().join("far.txt").is_file());
    assert!(src.is_file());
    assert_eq!(selected(&ex), ["far.txt"]);
}

/// A drop goes through Paste's path: a taken name asks with the replace prompt first.
#[test]
fn a_drop_onto_a_name_already_taken_asks_to_replace() {
    let mut ctx = test_ctx();
    let (dir, mut ex) = explorer(&mut ctx);
    std::fs::write(dir.path().join("Box/a.txt"), "older").expect("write");

    drag_onto(&mut ex, &mut ctx, "a.txt", "Box", DragAction::Copy);

    assert!(matches!(
        ex.dialog,
        Some(crate::dialogs::Dialog::Replace(_))
    ));
    let kept = std::fs::read_to_string(dir.path().join("Box/a.txt")).expect("read");
    assert_eq!(kept, "older", "nothing written yet");
}

/// A folder cannot be dropped into its own tree: `reprisefs` refuses and Explorer says so, as Paste does.
#[test]
fn dropping_a_folder_into_its_own_subtree_is_refused() {
    let mut ctx = test_ctx();
    let (dir, mut ex) = explorer(&mut ctx);
    let box_ = dir.path().join("Box");
    let inner = box_.join("Inner");
    std::fs::create_dir(&inner).expect("mkdir");

    ex.drop_into(std::slice::from_ref(&box_), &inner, DropOp::Move, &mut ctx);

    ex.finish_puts(&mut ctx);

    match ex.dialog.as_ref().expect("the error box is up") {
        crate::dialogs::Dialog::Message { title, text } => {
            assert_eq!(title, "Error Moving File or Folder");
            assert_eq!(
                text,
                "Cannot move Box: The destination folder is a subfolder of \
                 the source folder."
            );
        }
        _ => panic!("the refusal raises a message box"),
    }
    assert!(box_.is_dir() && !inner.join("Box").exists());
}

/// Escape disarms a drag not yet started, and the following release is no click.
#[test]
fn escape_disarms_a_drag_before_it_starts() {
    let mut ctx = test_ctx();
    let (_dir, mut ex) = explorer(&mut ctx);
    select(&mut ex, &["a.txt", "Crate"]);
    let at = centre(&ex, &ctx, index_of(&ex, "a.txt"));
    press(&mut ex, &mut ctx, at, BTN_LEFT);

    ex.key(
        repriseapp::KeyInput {
            sym: reprisekit::textedit::keysym::ESCAPE,
            utf8: None,
            mods: repriseapp::Mods::default(),
            edit: None,
        },
        &mut ctx,
    );
    assert!(ex.drag.is_none());
    release(&mut ex, &mut ctx, at, BTN_LEFT, SeatMods::default());
    assert_eq!(selected(&ex).len(), 2, "the held-back click is dropped");
}

/// Our own right drag asks with the plain drop's op as default; the picked row settles the drop at the source.
#[test]
fn our_right_drag_asks_with_the_plain_op_first() {
    let mut ctx = test_ctx();
    let (dir, mut ex) = explorer(&mut ctx);
    let r = start(&mut ex, &mut ctx, "a.txt", BTN_RIGHT);
    assert!(r.start_drag.is_some() && ex.popup.is_none());
    let onto = centre(&ex, &ctx, index_of(&ex, "Box"));
    ex.drag_enter(onto.0, onto.1, FILES, true, &ctx);
    let paths = vec![dir.path().join("a.txt")];
    drop_at(&mut ex, &mut ctx, onto, paths.clone(), DragAction::Ask);

    assert!(ex.popup.is_some(), "the drop menu is up");
    assert_eq!(
        ex.pending_drop,
        Some(PendingDrop {
            paths,
            links: Vec::new(),
            dir: dir.path().join("Box"),
            source: Actions::ALL,
        })
    );
    assert!(dir.path().join("a.txt").is_file(), "nothing yet");

    let r = ex.run(Command::Drop(DropOp::Copy), &mut ctx);
    assert_eq!(r.answer_drop, Some(Some(DragAction::Copy)));
    ex.finish_puts(&mut ctx);
    assert!(dir.path().join("Box/a.txt").is_file());
    assert!(dir.path().join("a.txt").is_file(), "Copy Here copies");
    assert!(ex.pending_drop.is_none());
}

/// Another program's Ask looks the same from the right button and a shortcut
/// drag, leading with the plain op; Create Shortcuts Here answers as a copy.
#[test]
fn another_programs_ask_leads_with_the_plain_op() {
    let mut ctx = test_ctx();
    let (dir, mut ex) = explorer(&mut ctx);
    let onto = centre(&ex, &ctx, index_of(&ex, "Box"));
    let paths = vec![dir.path().join("a.txt")];
    drop_at(&mut ex, &mut ctx, onto, paths, DragAction::Ask);
    assert!(ex.popup.is_some());
    let r = ex.run(Command::Drop(DropOp::Shortcut), &mut ctx);
    assert_eq!(r.answer_drop, Some(Some(DragAction::Copy)));
    ex.finish_puts(&mut ctx);
    let made: Vec<_> = std::fs::read_dir(dir.path().join("Box"))
        .expect("list")
        .filter_map(|e| e.ok())
        .collect();
    assert_eq!(made.len(), 1, "one shortcut");
    assert!(dir.path().join("a.txt").is_file());

    assert_eq!(ask_default(false, true), DropOp::Move);
    assert_eq!(ask_default(false, false), DropOp::Copy);
    assert_eq!(ask_default(true, true), DropOp::Shortcut);
}

/// Enter with no row highlighted runs the bold row: Move Here for a same-device drop.
#[test]
fn enter_on_the_drop_menu_runs_the_bold_row() {
    let mut ctx = test_ctx();
    let (dir, mut ex) = explorer(&mut ctx);
    start(&mut ex, &mut ctx, "a.txt", BTN_RIGHT);
    let onto = centre(&ex, &ctx, index_of(&ex, "Box"));
    ex.drag_enter(onto.0, onto.1, FILES, true, &ctx);
    let paths = vec![dir.path().join("a.txt")];
    drop_at(&mut ex, &mut ctx, onto, paths, DragAction::Ask);
    assert!(ex.popup.is_some(), "the drop menu is up");

    let r = ex.key(
        repriseapp::KeyInput {
            sym: reprisekit::textedit::keysym::RETURN,
            utf8: None,
            mods: repriseapp::Mods::default(),
            edit: None,
        },
        &mut ctx,
    );
    assert_eq!(r.answer_drop, Some(Some(DragAction::Move)));
    assert!(ex.popup.is_none());
    ex.finish_puts(&mut ctx);
    assert!(dir.path().join("Box/a.txt").is_file());
    assert!(!dir.path().join("a.txt").exists(), "Move Here moves");
}

/// The pointer leaving the window, or the keyboard going elsewhere, closes the drop menu and cancels at the source.
#[test]
fn the_drop_menu_closes_when_the_pointer_leaves() {
    let mut ctx = test_ctx();
    let (dir, mut ex) = explorer(&mut ctx);
    let onto = centre(&ex, &ctx, index_of(&ex, "Box"));
    let paths = vec![dir.path().join("a.txt")];
    drop_at(&mut ex, &mut ctx, onto, paths, DragAction::Ask);
    ex.pointer(PointerInput::Leave, &mut ctx);
    assert!(ex.popup.is_none());
    let r = ex.settle_drop(Reply::none());
    assert_eq!(r.answer_drop, Some(None));
}

/// Cancel, or the menu closing without a row, cancels the drop at the source.
#[test]
fn a_drop_menu_closed_without_a_row_cancels_the_drop() {
    let mut ctx = test_ctx();
    let (dir, mut ex) = explorer(&mut ctx);
    let onto = centre(&ex, &ctx, index_of(&ex, "Box"));
    let paths = vec![dir.path().join("a.txt")];

    drop_at(&mut ex, &mut ctx, onto, paths.clone(), DragAction::Ask);
    let r = ex.run(Command::DropCancel, &mut ctx);
    assert_eq!(r.answer_drop, Some(None));

    drop_at(&mut ex, &mut ctx, onto, paths, DragAction::Ask);
    let r = ex.settle_drop(Reply::none());
    assert!(r.answer_drop.is_none(), "the menu is still up");
    ex.popup = None;
    let r = ex.settle_drop(Reply::none());
    assert_eq!(r.answer_drop, Some(None));
    assert!(ex.pending_drop.is_none());
    assert!(!dir.path().join("Box/a.txt").exists());
}

/// A row answers with an action the source allows; a shortcut falls back to a move for a move-only source.
#[test]
fn a_menu_row_answers_with_what_the_source_allows() {
    let copy_only = Actions {
        copy: true,
        move_: false,
        ask: false,
    };
    let move_only = Actions {
        copy: false,
        move_: true,
        ask: false,
    };
    assert_eq!(answer_for(DropOp::Copy, copy_only), Some(DragAction::Copy));
    assert_eq!(answer_for(DropOp::Move, copy_only), None);
    assert_eq!(
        answer_for(DropOp::Shortcut, move_only),
        Some(DragAction::Move)
    );
    assert_eq!(answer_for(DropOp::Shortcut, Actions::default()), None);
}

/// Rows in order, the default bold and a row the source cannot settle greyed.
#[test]
fn the_drop_menus_rows() {
    let labels = |default, source| {
        crate::menus::drop_menu(default, source)
            .items()
            .iter()
            .map(|i| match i {
                reprisekit::controls::menu::MenuItem::Entry {
                    label,
                    bold,
                    enabled,
                    ..
                } => (label.clone(), *bold, *enabled),
                reprisekit::controls::menu::MenuItem::Separator => ("-".to_string(), false, true),
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        labels(DropOp::Move, Actions::ALL),
        [
            ("Copy Here".to_string(), false, true),
            ("Move Here".to_string(), true, true),
            ("Create Shortcuts Here".to_string(), false, true),
            ("-".to_string(), false, true),
            ("Cancel".to_string(), false, true),
        ]
    );
    let copy_only = Actions {
        copy: true,
        move_: false,
        ask: true,
    };
    let rows = labels(DropOp::Shortcut, copy_only);
    assert!(rows[2].1, "Create Shortcuts Here is the default");
    assert!(!rows[1].2, "Move Here is greyed");

    let m = crate::menus::drop_menu(DropOp::Move, Actions::ALL);
    assert_eq!(m.command(&[0]), Command::Drop(DropOp::Copy));
    assert_eq!(m.command(&[1]), Command::Drop(DropOp::Move));
    assert_eq!(m.command(&[2]), Command::Drop(DropOp::Shortcut));
    assert_eq!(m.command(&[4]), Command::DropCancel);
}

/// A browser's link dropped on a folder becomes a link file named after its
/// title, numbered past an existing one; the files of a mixed drop land too.
#[test]
fn a_dropped_link_becomes_a_link_file() {
    let mut ctx = test_ctx();
    let (dir, mut ex) = explorer(&mut ctx);
    let other = tempfile::tempdir().expect("a temp dir");
    let far = other.path().join("far.txt");
    std::fs::write(&far, "x").expect("write");
    let link = Uri::Url {
        url: "https://example.org/page".into(),
        title: Some("Example Page".into()),
    };
    let bg = background(&ex, &ctx);
    let r = ex.drag_drop(
        bg,
        Dropped::Uris(vec![link.clone()]),
        DragAction::Copy,
        Actions::ALL,
        &mut ctx,
    );
    assert!(r.redraw, "the drop highlight goes at once");
    ex.drag_drop(
        bg,
        Dropped::Uris(vec![link, Uri::File(far)]),
        DragAction::Copy,
        Actions::ALL,
        &mut ctx,
    );
    ex.finish_puts(&mut ctx);
    let text = std::fs::read_to_string(dir.path().join("Example Page.desktop")).expect("read");
    assert!(text.contains("Type=Link\n") && text.contains("URL=https://example.org/page\n"));
    assert!(dir.path().join("Example Page (2).desktop").is_file());
    assert!(dir.path().join("far.txt").is_file(), "the file came too");
    assert!(ex.items.iter().any(|it| it.name == "Example Page.desktop"));
}

/// A local file a browser's link names is copied, never moved; the drop menu greys Move Here for it.
#[test]
fn a_browsers_link_to_a_file_is_only_copied() {
    let mut ctx = test_ctx();
    let (dir, mut ex) = explorer(&mut ctx);
    let other = tempfile::tempdir_in(dir.path()).expect("a temp dir");
    let far = other.path().join("far.txt");
    std::fs::write(&far, "x").expect("write");
    let link = Offered {
        files: true,
        link: true,
        text: true,
    };
    let bg = background(&ex, &ctx);
    ex.drag_enter(bg.0, bg.1, link, false, &ctx);
    ex.drag_files(vec![far.clone()], &ctx);
    ex.drag_drop(
        bg,
        Dropped::Uris(vec![Uri::File(far.clone())]),
        DragAction::Move,
        Actions::ALL,
        &mut ctx,
    );
    ex.finish_puts(&mut ctx);
    assert!(dir.path().join("far.txt").is_file(), "it came");
    assert!(far.is_file(), "and stayed where it was");

    ex.drag_enter(bg.0, bg.1, link, false, &ctx);
    ex.drag_drop(
        bg,
        Dropped::Uris(vec![Uri::File(far.clone())]),
        DragAction::Ask,
        Actions::ALL,
        &mut ctx,
    );
    let p = ex.pending_drop.as_ref().expect("the drop menu asks");
    assert!(p.source.copy && !p.source.move_);
}

/// A drag dropped elsewhere and still awaiting its finish lends its files to no other drag.
#[test]
fn another_drag_does_not_take_our_dropped_drags_files() {
    let mut ctx = test_ctx();
    let (_dir, mut ex) = explorer(&mut ctx);
    start(&mut ex, &mut ctx, "Box", BTN_LEFT);
    let at = background(&ex, &ctx);
    let r = ex.drag_enter(at.0, at.1, FILES, false, &ctx);
    assert!(ex.incoming.as_ref().is_some_and(|i| i.paths.is_none()));
    let a = r.drag_accept.flatten().expect("taken");
    assert!(!a.actions.move_, "its files are not known yet");
}

#[test]
fn a_move_only_source_writes_no_link() {
    let mut ctx = test_ctx();
    let (dir, mut ex) = explorer(&mut ctx);
    let bg = background(&ex, &ctx);
    let move_only = Actions {
        copy: false,
        move_: true,
        ask: true,
    };
    let r = ex.drag_drop(
        bg,
        Dropped::Uris(vec![Uri::Url {
            url: "https://example.org/".into(),
            title: None,
        }]),
        DragAction::Ask,
        move_only,
        &mut ctx,
    );
    ex.finish_puts(&mut ctx);
    assert_eq!(r.answer_drop, Some(None));
    assert_eq!(std::fs::read_dir(dir.path()).expect("list").count(), 3);
}

#[test]
fn write_links_keeps_going_past_a_failure() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let links = [
        WebLink {
            url: "https://a.org/".into(),
            title: Some("..hidden".into()),
        },
        WebLink {
            url: "https://b.org/".into(),
            title: None,
        },
    ];
    assert_eq!(write_links(&links, dir.path()), Ok(()));
    assert!(dir.path().join("hidden.desktop").is_file());
    assert!(dir.path().join("b.org.desktop").is_file());
    let gone = dir.path().join("gone");
    let err = write_links(&links, &gone).expect_err("no folder");
    assert!(err.starts_with("Cannot create hidden.desktop"), "{err}");
}

/// The image stacks every dragged icon where it sits in the view, held where the press landed.
#[test]
fn the_drag_image_stacks_every_dragged_icon() {
    for mode in [Mode::Icons, Mode::Tiles] {
        let mut ctx = test_ctx();
        let (_dir, mut ex) = explorer(&mut ctx);
        ex.sv.mode = mode;
        select(&mut ex, &["a.txt", "Box", "Crate"]);
        let cells: Vec<Rect> = ["a.txt", "Box", "Crate"]
            .iter()
            .map(|n| cell(&ex, &ctx, index_of(&ex, n)))
            .collect();
        let at = centre(&ex, &ctx, index_of(&ex, "a.txt"));
        press(&mut ex, &mut ctx, at, BTN_LEFT);
        let (img, grab) = ex.drag_image(&mut ctx, 1).expect("an image");

        let bounds = cells
            .iter()
            .map(|c| Rect::new(c.x, c.y, iconview::CELL, iconview::CELL))
            .reduce(|a, b| a.union(&b))
            .expect("cells")
            .intersect(&Rect::new(at.0 - 128, at.1 - 128, 256, 256));
        assert_eq!(
            (img.width as i32, img.height as i32),
            (bounds.width, bounds.height),
            "{mode:?}"
        );
        assert_eq!(grab, (at.0 - bounds.x, at.1 - bounds.y), "{mode:?}");
        // Each cell's icon is in the image, faded: a pixel inside its icon square is drawn, at most half opaque.
        let icon_in = |c: &&Rect| {
            let sq = Rect::new(c.x + 21, c.y + 2, 32, 32);
            sq.intersect(&bounds) == sq
        };
        let whole: Vec<&Rect> = cells.iter().filter(icon_in).collect();
        assert!(whole.len() >= 2, "{mode:?}: at least two icons whole");
        for c in whole {
            let (x0, y0) = (c.x - bounds.x, c.y - bounds.y);
            let alphas: Vec<u32> = (0..32)
                .flat_map(|y| (0..32).map(move |x| (x, y)))
                .map(|(x, y)| img.get(x0 + 21 + x, y0 + 2 + y) >> 24)
                .collect();
            assert!(alphas.iter().any(|&a| a > 0), "{mode:?}: an icon at {c:?}");
            assert!(alphas.iter().all(|&a| a <= 128), "{mode:?}: faded");
        }
    }
}

/// Drawn again at another scale the image is that scale's, its icons fetched at it rather than
/// the listing's stretched, and the pointer holds the same point of it.
#[test]
fn the_drag_image_draws_again_at_another_scale() {
    const SMALL: u32 = 0xffc0_2020;
    const BIG: u32 = 0xff20_40c0;
    let themes = tempfile::tempdir().expect("a temp dir");
    let theme = themes.path().join("flat");
    let mut index = "[Icon Theme]\nName=Flat\nDirectories=32x32/places,64x64/places\n".to_string();
    for (px, argb) in [(32u32, SMALL), (64, BIG)] {
        let dir = format!("{px}x{px}/places");
        index += &format!("\n[{dir}]\nSize={px}\nType=Fixed\n");
        std::fs::create_dir_all(theme.join(&dir)).expect("mkdir");
        let mut c = Canvas::new(px, px);
        c.pixels.fill(argb);
        let png = reprisekit::raster::to_png(&c);
        std::fs::write(theme.join(&dir).join("folder.png"), png).expect("png");
    }
    std::fs::write(theme.join("index.theme"), index).expect("index");
    let mut ctx = test_ctx();
    ctx.icons = reprisekit::icontheme::IconCache::new(reprisekit::icontheme::IconTheme::load(
        "flat",
        &[themes.path().to_path_buf()],
    ));
    let (_dir, mut ex) = explorer(&mut ctx);
    select(&mut ex, &["Box"]);
    let c = cell(&ex, &ctx, index_of(&ex, "Box"));
    let at = centre(&ex, &ctx, index_of(&ex, "Box"));
    press(&mut ex, &mut ctx, at, BTN_LEFT);
    let (one, grab) = ex.drag_image(&mut ctx, 1).expect("an image");
    let (two, grab2) = ex.drag_image(&mut ctx, 2).expect("an image at scale 2");

    assert_eq!(grab2, grab);
    assert_eq!(two.scale(), 2);
    assert_eq!((two.width, two.height), (one.width, one.height));
    // The folder icon's middle, in the image's logical px.
    let (x, y) = (
        c.x - (at.0 - grab.0) + 21 + 16,
        c.y - (at.1 - grab.1) + 2 + 16,
    );
    let rgb = |img: &Canvas, s: i32| {
        let dw = img.device_size().0 as i32;
        let [_, r, _, b] = img.pixels[(y * s * dw + x * s) as usize].to_be_bytes();
        (r, b)
    };
    let (r, b) = rgb(&one, 1);
    assert!(r > b, "scale 1 wears the 32 px folder");
    let (r, b) = rgb(&two, 2);
    assert!(b > r, "scale 2 wears the 64 px folder");
}

/// At a larger text size every mode's image is made of whole grown icon cells inside the grown square: the dragged cells in the icon modes, the pressed entry's centred on the press in List and Details.
#[test]
fn the_drag_image_grows_with_the_text_size() {
    const EXPECTED_CELL: [(u32, (i32, i32)); 2] = [(150, (116, 95)), (200, (150, 112))];
    let modes = [
        Mode::Icons,
        Mode::Tiles,
        Mode::Thumbnails,
        Mode::List,
        Mode::Details,
    ];
    for (pct, (w, h)) in EXPECTED_CELL {
        for mode in modes {
            let mut ctx = test_ctx();
            ctx.theme = reprisekit::theme::Theme::default().with_text_size(pct);
            let (_dir, mut ex) = explorer(&mut ctx);
            ex.sv.mode = mode;
            select(&mut ex, &["a.txt", "Box", "Crate"]);
            let at = centre(&ex, &ctx, index_of(&ex, "a.txt"));
            press(&mut ex, &mut ctx, at, BTN_LEFT);
            let (img, grab) = ex.drag_image(&mut ctx, 1).expect("an image");
            let side = 256 * pct as i32 / 100;
            let window = Rect::new(at.0 - side / 2, at.1 - side / 2, side, side);
            let bounds = if matches!(mode, Mode::List | Mode::Details) {
                Rect::new(at.0 - w / 2, at.1 - h / 2, w, h)
            } else {
                ["a.txt", "Box", "Crate"]
                    .iter()
                    .map(|n| cell(&ex, &ctx, index_of(&ex, n)))
                    .map(|c| Rect::new(c.x, c.y, w, h))
                    .filter(|c| window.intersect(c) == *c)
                    .reduce(|a, b| a.union(&b))
                    .expect("a whole cell")
            };
            assert_eq!(
                (img.width as i32, img.height as i32),
                (bounds.width, bounds.height),
                "{mode:?} at {pct}%"
            );
            assert_eq!(
                grab,
                (at.0 - bounds.x, at.1 - bounds.y),
                "{mode:?} at {pct}%"
            );
        }
    }
}

/// The press's selection change is held back over an already selected entry, so dragging one of several carries all.
#[test]
fn a_press_on_a_selected_entry_drags_the_whole_selection() {
    let mut ctx = test_ctx();
    let (_dir, mut ex) = explorer(&mut ctx);
    select(&mut ex, &["a.txt", "Crate"]);

    let r = start(&mut ex, &mut ctx, "a.txt", BTN_LEFT);
    let out = r.start_drag.expect("started");
    assert_eq!(
        out.paths.len(),
        2,
        "the press did not collapse the selection"
    );
    assert_eq!(selected(&ex).len(), 2);
}

/// The same press released without a drag is a plain click collapsing the selection to the entry.
#[test]
fn a_release_without_a_drag_applies_the_click_the_press_held_back() {
    let mut ctx = test_ctx();
    let (_dir, mut ex) = explorer(&mut ctx);
    select(&mut ex, &["a.txt", "Crate"]);
    let at = centre(&ex, &ctx, index_of(&ex, "a.txt"));

    press(&mut ex, &mut ctx, at, BTN_LEFT);
    assert_eq!(
        selected(&ex).len(),
        2,
        "still both while the button is down"
    );
    release(&mut ex, &mut ctx, at, BTN_LEFT, SeatMods::default());

    assert_eq!(selected(&ex), ["a.txt"]);
}

/// A Ctrl-press on a selected entry doesn't deselect it under the drag; the toggle waits for a release without drag.
#[test]
fn ctrl_on_a_selected_entry_drags_it_and_only_toggles_on_a_click() {
    let mut ctx = test_ctx();
    let (_dir, mut ex) = explorer(&mut ctx);
    select(&mut ex, &["a.txt", "Crate"]);
    let at = centre(&ex, &ctx, index_of(&ex, "a.txt"));
    press_with(&mut ex, &mut ctx, at, BTN_LEFT, ctrl());
    let d = ex.drag.as_ref().expect("armed");
    assert_eq!(d.items.len(), 2, "the entry pressed is still in the drag");
    assert!(d.pending_click.is_some(), "the toggle is held back");
    let r = motion(&mut ex, &mut ctx, (at.0 + THRESHOLD + 1, at.1));
    assert_eq!(r.start_drag.expect("started").paths.len(), 2);

    // Ctrl-press and release without travelling does toggle the entry out.
    let mut ctx = test_ctx();
    let (_dir, mut ex) = explorer(&mut ctx);
    select(&mut ex, &["a.txt", "Crate"]);
    let at = centre(&ex, &ctx, index_of(&ex, "a.txt"));
    press_with(&mut ex, &mut ctx, at, BTN_LEFT, ctrl());
    release(&mut ex, &mut ctx, at, BTN_LEFT, ctrl());

    assert_eq!(selected(&ex), ["Crate"]);
    assert!(ex.drag.is_none());
}

/// A Ctrl-press outside the selection adds it at once and drags the larger selection.
#[test]
fn ctrl_on_an_unselected_entry_adds_it_and_drags_both() {
    let mut ctx = test_ctx();
    let (_dir, mut ex) = explorer(&mut ctx);
    select(&mut ex, &["Crate"]);
    let at = centre(&ex, &ctx, index_of(&ex, "a.txt"));

    press_with(&mut ex, &mut ctx, at, BTN_LEFT, ctrl());

    let d = ex.drag.as_ref().expect("armed");
    assert_eq!(d.items.len(), 2);
    assert!(d.pending_click.is_none(), "nothing left to apply");
    assert_eq!(selected(&ex).len(), 2);
}

/// A right press that never travels is a plain right click: the release opens the context menu.
#[test]
fn a_right_press_without_a_drag_still_opens_the_item_menu() {
    let mut ctx = test_ctx();
    let (_dir, mut ex) = explorer(&mut ctx);
    let file = index_of(&ex, "a.txt");
    let at = centre(&ex, &ctx, file);

    press(&mut ex, &mut ctx, at, BTN_RIGHT);
    release(&mut ex, &mut ctx, at, BTN_RIGHT, SeatMods::default());

    assert!(ex.popup.is_some(), "the item menu opened on the release");
    assert!(ex.drag.is_none());
    assert_eq!(
        ex.sv.selection.set.iter().copied().collect::<Vec<_>>(),
        [file]
    );
}

/// A read-only place arms no drag, so its entries' menu opens on the press.
#[test]
fn a_right_click_in_a_read_only_place_opens_the_item_menu() {
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Computer, &mut ctx);
    ex.sv.mode = Mode::Icons;
    assert!(!ex.items.is_empty(), "Computer lists something");
    let at = centre(&ex, &ctx, 0);

    press(&mut ex, &mut ctx, at, BTN_RIGHT);
    release(&mut ex, &mut ctx, at, BTN_RIGHT, SeatMods::default());

    assert!(ex.popup.is_some(), "the drive's menu opened");
    assert_eq!(ex.sv.selection.set.iter().copied().collect::<Vec<_>>(), [0]);
}

/// A place that takes no writes (Computer's drives, the bin's contents) has nothing to drag.
#[test]
fn a_press_in_a_read_only_place_arms_nothing() {
    let mut ctx = test_ctx();
    for place in [Place::Computer, Place::RecycleBin] {
        let mut ex = crate::tests::test_explorer(place.clone(), &mut ctx);
        ex.sv.mode = Mode::Icons;

        ex.arm_drag(0, BTN_LEFT, 10, 10);

        assert!(ex.drag.is_none(), "{place:?} armed a drag");
    }
    // The same call in a folder arms one, so the guard stopped it above, not an empty listing.
    let (_dir, mut ex) = explorer(&mut ctx);
    ex.arm_drag(index_of(&ex, "a.txt"), BTN_LEFT, 10, 10);
    assert!(ex.drag.is_some());
}

/// A drive under Computer is a directory with a mount path, so `is_dir`
/// alone would light it as a drop target; its place is read-only, so neither it nor its background is one.
#[test]
fn a_read_only_place_is_no_drop_target() {
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Computer, &mut ctx);
    ex.sv.mode = Mode::Icons;
    assert!(!ex.items.is_empty(), "Computer lists something");
    assert!(
        ex.items[0].is_dir,
        "and its first entry looks like a folder"
    );
    let at = centre(&ex, &ctx, 0);
    assert_eq!(ex.drag_target(at.0, at.1, &[], &ctx), None);
    let bg = background(&ex, &ctx);
    assert_eq!(ex.drag_target(bg.0, bg.1, &[], &ctx), None);
}

#[test]
fn a_drop_on_the_address_bar_goes_into_its_field() {
    let mut ctx = test_ctx();
    let (_dir, mut ex) = explorer(&mut ctx);
    let r = ex.layout(&ctx).address.expect("the address bar shows");
    let text = Offered {
        files: false,
        link: false,
        text: true,
    };
    let at = (r.x + r.width / 2, r.y + r.height / 2);
    let a = ex.drag_enter(at.0, at.1, text, false, &ctx);
    let a = a.drag_accept.flatten().expect("the field takes text");
    assert_eq!(a.kind, DropKind::Text);
    assert!(ex.address.combo.drop_x.is_some(), "the drop caret shows");
    ex.address.combo.edit = Some(reprisekit::textedit::TextEdit::new("", false));
    let r = ex.drag_drop(
        at,
        Dropped::Text("/tmp".into()),
        DragAction::Ask,
        Actions::ALL,
        &mut ctx,
    );
    assert_eq!(r.answer_drop, Some(Some(DragAction::Copy)));
    assert_eq!(ex.address.text(), "/tmp");
    assert_eq!(ex.focus, crate::Focus::Address);
    assert!(ex.address.combo.drop_x.is_none());

    let f = ex.drag_enter(at.0, at.1, FILES, false, &ctx);
    assert_eq!(
        f.drag_accept.flatten().map(|a| a.kind),
        Some(DropKind::Files),
        "files drop their paths"
    );
    ex.drag_leave();
    assert!(ex.address.combo.drop_x.is_none());
}
