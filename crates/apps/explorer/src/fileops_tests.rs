//! The file operations' tests: the pure planning bits, and the window's operations over a real temporary folder.

use super::*;
use crate::commands::Command;
use crate::dialogs::Dialog;
use crate::place::Place;
use crate::tests::test_ctx;
use reprisefs::apps::AppEntry;
use reprisefs::sendto::SendToRow;
use std::ffi::OsString;

/// Alt+`c` as the seat delivers it: the access key `chooser_event` reads off `KeyInput`.
fn key_event_alt(c: char) -> repriseapp::Event {
    let utf8 = c.to_string();
    let edit = reprisekit::textedit::key_for(c as u32, false, false, Some(&utf8));
    repriseapp::Event::Key(repriseapp::KeyInput {
        sym: c as u32,
        utf8: Some(utf8),
        mods: repriseapp::Mods {
            ctrl: false,
            shift: false,
            alt: true,
        },
        edit,
    })
}

fn app(name: &str, id: &str, mimes: &[&str]) -> AppEntry {
    AppEntry {
        name: name.into(),
        exec_line: format!("/usr/bin/{id} %f"),
        terminal: false,
        id: format!("{id}.desktop"),
        icon: None,
        startup_wm_class: None,
        categories: Vec::new(),
        mime_types: mimes.iter().map(|m| (*m).to_string()).collect(),
        shown: true,
    }
}

#[test]
fn a_paste_moves_a_cut_copies_a_copy_and_ignores_a_cut_into_its_own_folder() {
    let dir = Path::new("/home/u/Documents");
    let here = dir.join("x.txt");
    let away = Path::new("/home/u/Desktop/x.txt");
    assert_eq!(plan_paste(true, false, away, dir), Some(DropOp::Move));
    assert_eq!(plan_paste(false, false, away, dir), Some(DropOp::Copy));
    assert_eq!(plan_paste(false, false, &here, dir), Some(DropOp::Copy));
    assert_eq!(plan_paste(true, false, &here, dir), None);
    assert_eq!(plan_paste(true, true, &here, dir), Some(DropOp::Shortcut));
}

/// New Folder creates the default name, selects it and opens the rename box on it.
#[test]
fn new_folder_creates_the_folder_and_starts_a_rename() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);

    ex.run(Command::NewFolder, &mut ctx);

    assert!(dir.path().join("New Folder").is_dir());
    let i = ex
        .items
        .iter()
        .position(|it| it.name == "New Folder")
        .expect("the new folder is listed");
    assert_eq!(ex.sv.edit.as_ref().map(|(j, _)| *j), Some(i));
    assert_eq!(ex.undo, Some("New"));
}

/// A New Text Document whose name appeared after the last listing leaves that file as it was and reports the clash.
#[test]
fn new_text_document_never_truncates_an_unlisted_file() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    let taken = dir.path().join("New Text Document.txt");
    std::fs::write(&taken, "keep me").expect("write");

    ex.new_item(false, &mut ctx);

    assert_eq!(std::fs::read_to_string(&taken).expect("read"), "keep me");
    assert!(
        matches!(ex.dialog, Some(Dialog::Message { .. })),
        "the clash is reported"
    );
    assert_eq!(ex.undo, None);
}

/// `$XDG_DATA_HOME` is process-wide, so tests that point the trash at a temporary home take turns.
static TRASH_ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Points `$XDG_DATA_HOME` at `dir` and `$XDG_CONFIG_HOME` at `dir/config` (the test's Recycle Bin
/// settings), with no drives (only the home trash is listed), and restores the old values on drop so a
/// test cannot leave the process pointing at a deleted tempdir. Held under [`TRASH_ENV`].
struct DataHome(Vec<(&'static str, Option<std::ffi::OsString>)>);

impl DataHome {
    fn set(dir: &Path) -> Self {
        let vars = [
            ("XDG_DATA_HOME", dir.to_path_buf()),
            ("XDG_CONFIG_HOME", dir.join("config")),
        ];
        let was = vars
            .iter()
            .map(|(name, _)| (*name, std::env::var_os(name)))
            .collect();
        for (name, value) in vars {
            // SAFETY: std serialises its own env access and only `TRASH_ENV` holders read these variables.
            unsafe { std::env::set_var(name, value) };
        }
        reprisefs::trash::set_mount_table(Some("/dev/null".into()));
        Self(was)
    }

    fn recycle_bin(&self, keys: &str) {
        let path = reprisefs::bincap::config_path();
        std::fs::create_dir_all(path.parent().unwrap()).expect("mkdir");
        std::fs::write(path, format!("[recycle_bin]\n{keys}")).expect("write");
    }
}

impl Drop for DataHome {
    fn drop(&mut self) {
        reprisefs::trash::set_mount_table(None);
        for (name, old) in self.0.drain(..) {
            // SAFETY: as in `DataHome::set`; still under `TRASH_ENV`.
            unsafe {
                match old {
                    Some(old) => std::env::set_var(name, old),
                    None => std::env::remove_var(name),
                }
            }
        }
    }
}

/// The default Recycle Bin settings for a test that deletes without a [`DataHome`], so the developer's
/// own `[recycle_bin]` cannot change what Delete does. Takes [`TRASH_ENV`].
struct DefaultBinSettings {
    _env: std::sync::MutexGuard<'static, ()>,
}

impl DefaultBinSettings {
    fn set() -> Self {
        let guard = TRASH_ENV.lock().unwrap_or_else(|e| e.into_inner());
        reprisefs::bincap::set_config_path(Some("/nonexistent/reprise/config.toml".into()));
        Self { _env: guard }
    }
}

impl Drop for DefaultBinSettings {
    fn drop(&mut self) {
        reprisefs::bincap::set_config_path(None);
    }
}

/// Puts `dir` back to `0o700` on drop, so a failed assertion in the chmod-0500 test cannot leave an undeletable tempdir.
struct Mode0700(PathBuf);

impl Drop for Mode0700 {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o700));
    }
}

/// The tree menu's Delete, Cut and Copy take the node's folder, not the file selected in the view.
#[test]
fn the_tree_menus_file_rows_take_the_node() {
    let _bin = DefaultBinSettings::set();
    let home = tempfile::Builder::new()
        .prefix("treemenu")
        .tempdir()
        .expect("a temp dir");
    let dir = home.path().join("folder");
    let sub = dir.join("Reports");
    std::fs::create_dir_all(&sub).expect("mkdir");
    std::fs::write(dir.join("a.txt"), "1").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.clone()), &mut ctx);
    ex.settings.show_hidden = true;
    ex.sv
        .selection
        .set_only(ex.items.iter().position(|it| it.name == "a.txt"));
    ex.build_tree(&mut ctx);
    ex.tree_follow(&Place::Folder(sub.clone()), &mut ctx);
    assert_eq!(ex.node_folder(), Some((sub.clone(), "Reports".to_string())));

    let reply = ex.run(Command::NodeCopy, &mut ctx);
    assert_eq!(reply.copy_files.map(|c| c.paths), Some(vec![sub.clone()]));

    ex.run(Command::NodeDelete, &mut ctx);
    match ex.dialog.as_ref() {
        Some(Dialog::Confirm(c)) => assert_eq!(c.paths, vec![sub.clone()]),
        other => panic!("no confirm box: {:?}", other.is_some()),
    }
    assert!(sub.exists());
    assert!(dir.join("a.txt").exists());
}

/// A delete prompt wears a prompt's caption (no icon, close box alone); Properties the ? box; the window its own.
#[test]
fn a_delete_prompt_has_the_prompt_caption() {
    use repriseapp::Caption;
    let _bin = DefaultBinSettings::set();
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::write(dir.path().join("a.txt"), "1").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.sv.selection.set_only(Some(0));
    ex.run(Command::Delete { erase: false }, &mut ctx);
    assert!(matches!(ex.dialog, Some(Dialog::Confirm(_))), "Delete asks");
    assert_eq!(ex.caption(crate::dialogs::DIALOG), Caption::Prompt);
    assert_eq!(ex.caption(crate::props_dialog::PROPS), Caption::Help);
    assert_eq!(ex.caption(repriseapp::MAIN), Caption::Window);
}

fn key_event_plain(sym: u32, edit: Option<reprisekit::textedit::EditKey>) -> repriseapp::Event {
    repriseapp::Event::Key(repriseapp::KeyInput {
        sym,
        utf8: None,
        mods: repriseapp::Mods::default(),
        edit,
    })
}

fn right() -> repriseapp::Event {
    key_event_plain(
        reprisekit::textedit::keysym::RIGHT,
        Some(reprisekit::textedit::EditKey::Right {
            select: false,
            word: false,
        }),
    )
}

fn enter() -> repriseapp::Event {
    key_event_plain(reprisekit::textedit::keysym::RETURN, None)
}

/// Right moves the delete prompt's focus from Yes to No, and Enter presses No: the prompt closes, the file stays.
#[test]
fn an_arrow_moves_the_delete_prompt_s_focus_and_enter_presses_it() {
    let _bin = DefaultBinSettings::set();
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::write(dir.path().join("a.txt"), "1").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.sv.selection.set_only(Some(0));
    ex.run(Command::Delete { erase: true }, &mut ctx);
    assert!(matches!(ex.dialog, Some(Dialog::Confirm(_))), "Delete asks");
    ex.dialog_event(right(), &mut ctx);
    assert!(
        matches!(ex.dialog, Some(Dialog::Confirm(_))),
        "Right leaves the prompt up"
    );
    ex.dialog_event(enter(), &mut ctx);
    ex.finish_puts(&mut ctx);
    assert!(ex.dialog.is_none(), "Enter on No closes the prompt");
    assert!(dir.path().join("a.txt").exists(), "Enter on No keeps a.txt");
}

/// A drive's node offers no file rows: Cut, Copy and Delete do nothing.
#[test]
fn the_tree_menus_file_rows_skip_a_drive() {
    let home = tempfile::Builder::new()
        .prefix("treedrive")
        .tempdir()
        .expect("a temp dir");
    let dir = home.path().join("folder");
    let stick = dir.join("Stick");
    std::fs::create_dir_all(&stick).expect("mkdir");
    std::fs::write(stick.join("a.txt"), "1").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.clone()), &mut ctx);
    ex.settings.show_hidden = true;
    ex.build_tree(&mut ctx);
    ex.tree_follow(&Place::Folder(stick.clone()), &mut ctx);
    assert!(ex.node_folder().is_some(), "a plain folder has file rows");

    ex.drive_mounts = vec![stick.clone()];
    assert_eq!(ex.node_folder(), None);
    assert!(ex.run(Command::NodeCut, &mut ctx).copy_files.is_none());
    ex.run(Command::NodeDelete, &mut ctx);
    assert!(ex.dialog.is_none(), "Delete opens no confirm box");
    assert!(stick.join("a.txt").exists());
}

#[test]
fn delete_trashes_the_selection_and_undo_restores_it() {
    let _guard = TRASH_ENV.lock().unwrap_or_else(|e| e.into_inner());
    let home = tempfile::tempdir().expect("a temp dir");
    let _data_home = DataHome::set(home.path());
    let dir = home.path().join("folder");
    std::fs::create_dir(&dir).expect("mkdir");
    std::fs::write(dir.join("a.txt"), "1").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.clone()), &mut ctx);
    ex.sv.selection.set_only(Some(0));

    ex.run(Command::Delete { erase: false }, &mut ctx);
    assert!(ex.dialog.is_some());
    assert!(dir.join("a.txt").exists());

    ex.delete_now(&[dir.join("a.txt")], false, &mut ctx);
    ex.finish_puts(&mut ctx);

    assert!(!dir.join("a.txt").exists());
    assert_eq!(
        std::fs::read_dir(home.path().join("Trash/files"))
            .expect("the trash")
            .count(),
        1
    );
    assert_eq!(ex.undo, Some("Delete"));
    assert!(ex.items.is_empty(), "the view re-listed without it");

    ex.run(Command::Undo, &mut ctx);

    assert_eq!(
        std::fs::read_to_string(dir.join("a.txt")).expect("read"),
        "1"
    );
    assert_eq!(ex.items.len(), 1, "the view re-listed with it back");
    assert_eq!(ex.undo, None);
}

/// A delete that makes room in a full Recycle Bin (1% of the drive here) erases its oldest item, and
/// that item's Undo leaves the stack with it instead of failing on the erased one.
#[test]
fn a_delete_that_purges_the_bin_drops_the_purged_items_undo() {
    let _guard = TRASH_ENV.lock().unwrap_or_else(|e| e.into_inner());
    let home = tempfile::tempdir().expect("a temp dir");
    let data_home = DataHome::set(home.path());
    data_home.recycle_bin("max_percent = 1\n");
    let dir = home.path().join("folder");
    std::fs::create_dir(&dir).expect("mkdir");
    let cap = reprisefs::bincap::drive_size(home.path()) / 100;
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.clone()), &mut ctx);
    for name in ["a.iso", "b.iso", "c.iso"] {
        let path = dir.join(name);
        std::fs::File::create(&path)
            .and_then(|f| f.set_len(cap * 2 / 5))
            .expect("a sparse file");
        ex.delete_now(&[path], false, &mut ctx);
        ex.finish_puts(&mut ctx);
    }
    let trashed: Vec<_> = std::fs::read_dir(home.path().join("Trash/files"))
        .expect("the trash")
        .flatten()
        .map(|e| e.file_name())
        .collect();
    assert_eq!(trashed.len(), 2, "the oldest was erased: {trashed:?}");
    assert!(!trashed.iter().any(|n| n == "a.iso"));

    ex.run(Command::Undo, &mut ctx);
    ex.run(Command::Undo, &mut ctx);

    assert!(dir.join("b.iso").exists() && dir.join("c.iso").exists());
    assert_eq!(ex.undo, None, "a.iso's undo went with it");
    assert!(ex.dialog.is_none(), "no error box");
}

/// An item bigger than its whole Recycle Bin (1% of the drive here) raises "too big for the Recycle
/// Bin" before anything is deleted: No leaves it and trashes the rest of the batch, Yes erases it.
#[test]
fn an_item_too_big_for_the_bin_asks_to_be_deleted_for_good() {
    use reprisekit::controls::form;
    let _guard = TRASH_ENV.lock().unwrap_or_else(|e| e.into_inner());
    let home = tempfile::tempdir().expect("a temp dir");
    let data_home = DataHome::set(home.path());
    data_home.recycle_bin("max_percent = 1\n");
    let dir = home.path().join("folder");
    std::fs::create_dir(&dir).expect("mkdir");
    let cap = reprisefs::bincap::drive_size(home.path()) / 100;
    let (big, small) = (dir.join("big.iso"), dir.join("small.txt"));
    std::fs::File::create(&big)
        .and_then(|f| f.set_len(cap + 1))
        .expect("a sparse file");
    std::fs::write(&small, "s").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.clone()), &mut ctx);

    ex.delete_now(&[big.clone(), small.clone()], false, &mut ctx);
    ex.finish_puts(&mut ctx);
    let Some(Dialog::Confirm(c)) = &ex.dialog else {
        panic!("the question is up")
    };
    assert_eq!(
        c.text,
        "'big.iso' is too big for the Recycle Bin. Do you want to delete it permanently?"
    );
    assert_eq!(c.title, "Confirm File Delete");
    assert_eq!(c.buttons(), ["&Yes", "Yes to &All", "&No", "Cancel"]);
    assert!(
        big.exists() && small.exists(),
        "nothing goes before the answer"
    );

    ex.dialog_outcome(form::Event::Pressed("btn2"), &mut ctx);
    ex.finish_puts(&mut ctx);
    assert!(ex.dialog.is_none());
    assert!(big.exists(), "No leaves it");
    assert!(!small.exists(), "the rest of the batch is trashed");

    ex.delete_now(std::slice::from_ref(&big), false, &mut ctx);
    ex.finish_puts(&mut ctx);
    let Some(Dialog::Confirm(c)) = &ex.dialog else {
        panic!("the question is up")
    };
    assert_eq!(c.buttons(), ["&Yes", "&No"], "one item: Yes or No");
    ex.dialog_outcome(form::Event::Default, &mut ctx);
    ex.finish_puts(&mut ctx);
    assert!(!big.exists(), "Yes erases it");
    let trashed: Vec<_> = std::fs::read_dir(home.path().join("Trash/files"))
        .expect("the trash")
        .flatten()
        .map(|e| e.file_name())
        .collect();
    assert_eq!(trashed, ["small.txt"]);
}

/// A drive set to remove files immediately asks the plain delete question and erases; with confirmation off nothing asks.
#[test]
fn remove_immediately_erases_and_the_prompt_can_be_turned_off() {
    let _guard = TRASH_ENV.lock().unwrap_or_else(|e| e.into_inner());
    let home = tempfile::tempdir().expect("a temp dir");
    let data_home = DataHome::set(home.path());
    data_home.recycle_bin("remove_immediately = true\n");
    let dir = home.path().join("folder");
    std::fs::create_dir(&dir).expect("mkdir");
    std::fs::write(dir.join("a.txt"), "1").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.clone()), &mut ctx);
    ex.sv.selection.set_only(Some(0));

    ex.run(Command::Delete { erase: false }, &mut ctx);
    let Some(Dialog::Confirm(c)) = &ex.dialog else {
        panic!("the prompt is up")
    };
    assert!(c.erase);
    assert_eq!(c.text, "Are you sure you want to delete 'a.txt'?");
    ex.dialog = None;

    data_home.recycle_bin("remove_immediately = true\nconfirm = false\n");
    ex.run(Command::Delete { erase: false }, &mut ctx);
    assert!(ex.dialog.is_none(), "no prompt");
    ex.finish_puts(&mut ctx);
    assert!(!dir.join("a.txt").exists());
    assert!(!home.path().join("Trash/files").exists(), "nothing trashed");
}

/// Undo of a copy onto a drive that removes files immediately asks before erasing the copy: No
/// keeps it and the Undo row, Yes erases it with nothing put in the bin.
#[test]
fn undo_of_a_copy_on_a_remove_immediately_drive_asks_then_erases() {
    use reprisekit::controls::form;
    let _guard = TRASH_ENV.lock().unwrap_or_else(|e| e.into_inner());
    let home = tempfile::tempdir().expect("a temp dir");
    let data_home = DataHome::set(home.path());
    data_home.recycle_bin("remove_immediately = true\n");
    let dir = home.path().join("folder");
    std::fs::create_dir(&dir).expect("mkdir");
    let copy = dir.join("copy.txt");
    std::fs::write(&copy, "1").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.clone()), &mut ctx);
    ex.undos
        .push(reprisefs::undo::Undoable::Copied(copy.clone()));
    ex.sync_undo();

    ex.run(Command::Undo, &mut ctx);
    let Some(Dialog::Confirm(c)) = &ex.dialog else {
        panic!("the question is up")
    };
    assert_eq!(
        c.text,
        "'copy.txt' is on a drive that removes files immediately. Do you want to delete it permanently?"
    );
    assert_eq!(c.buttons(), ["&Yes", "&No"]);
    let no = reprisekit::controls::msgbox::button_id(1);
    ex.dialog_outcome(form::Event::Pressed(no), &mut ctx);
    assert!(ex.dialog.is_none());
    assert!(copy.exists(), "No keeps the copy");
    assert_eq!(ex.undo, Some("Copy"), "and the Undo row");

    ex.run(Command::Undo, &mut ctx);
    ex.dialog_outcome(form::Event::Default, &mut ctx);
    assert!(ex.dialog.is_none());
    assert!(!copy.exists(), "Yes erases it");
    assert!(!home.path().join("Trash/files").exists(), "nothing trashed");
    assert_eq!(ex.undo, None);
}

/// Restore in the Recycle Bin puts the selected item back (with nothing selected, the rest). Undo no
/// longer offers to restore what Restore already put back.
#[test]
fn restore_puts_the_selection_back_then_everything() {
    let _guard = TRASH_ENV.lock().unwrap_or_else(|e| e.into_inner());
    let home = tempfile::tempdir().expect("a temp dir");
    let _data_home = DataHome::set(home.path());
    let dir = home.path().join("folder");
    std::fs::create_dir(&dir).expect("mkdir");
    for n in ["a.txt", "b.txt"] {
        std::fs::write(dir.join(n), n).expect("write");
    }
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.clone()), &mut ctx);
    ex.delete_now(&[dir.join("a.txt")], false, &mut ctx);
    ex.delete_now(&[dir.join("b.txt")], false, &mut ctx);
    ex.finish_puts(&mut ctx);
    ex.run(Command::GoTo(Place::RecycleBin), &mut ctx);
    assert_eq!(ex.items.len(), 2);

    let b = ex
        .items
        .iter()
        .position(|it| it.name == "b.txt")
        .expect("b");
    ex.sv.selection.set_only(Some(b));
    assert!(ex.run(Command::Open, &mut ctx).spawn.is_empty());
    assert!(
        ex.props.take().is_some(),
        "Open showed the Properties sheet"
    );
    ex.run(Command::Restore, &mut ctx);
    ex.finish_puts(&mut ctx);

    assert_eq!(
        std::fs::read_to_string(dir.join("b.txt")).expect("b"),
        "b.txt"
    );
    assert!(!dir.join("a.txt").exists(), "only the selection came back");
    assert_eq!(ex.items.len(), 1, "the bin re-listed without it");
    assert_eq!(ex.undo, Some("Delete"), "a's delete is still undoable");

    ex.sv.selection.set_only(None);
    ex.run(Command::Restore, &mut ctx);
    ex.finish_puts(&mut ctx);

    assert_eq!(
        std::fs::read_to_string(dir.join("a.txt")).expect("a"),
        "a.txt"
    );
    assert!(ex.items.is_empty());
    assert_eq!(ex.undo, None);
    assert!(ex.dialog.is_none(), "no error box");
}

/// A file on another drive goes to that drive's own trash. The Recycle Bin lists it beside the home
/// trash's items, Restore puts it back, Empty clears both.
#[test]
fn the_recycle_bin_holds_every_drives_trash() {
    use std::os::unix::fs::MetadataExt;
    let _guard = TRASH_ENV.lock().unwrap_or_else(|e| e.into_inner());
    let home = tempfile::tempdir().expect("a temp dir");
    let _data_home = DataHome::set(home.path());
    let (dir, drive) = (home.path().join("folder"), home.path().join("drive"));
    std::fs::create_dir(&dir).expect("mkdir");
    std::fs::create_dir(&drive).expect("mkdir");
    std::fs::write(dir.join("h.txt"), "h").expect("write");
    std::fs::write(drive.join("d.txt"), "d").expect("write");
    let table = home.path().join("mounts");
    let rows = format!(
        "/dev/a {} ext4 rw 0 0\n/dev/b {} vfat rw 0 0\n",
        home.path().display(),
        drive.display()
    );
    std::fs::write(&table, rows).expect("write");
    reprisefs::trash::set_mount_table(Some(table));
    let uid = std::fs::metadata(home.path()).expect("stat").uid();
    let drive_files = drive.join(format!(".Trash-{uid}/files"));
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.clone()), &mut ctx);

    ex.delete_now(&[dir.join("h.txt"), drive.join("d.txt")], false, &mut ctx);
    ex.finish_puts(&mut ctx);
    assert!(drive_files.join("d.txt").exists(), "on the drive's trash");
    assert!(home.path().join("Trash/files/h.txt").exists());

    ex.run(Command::GoTo(Place::RecycleBin), &mut ctx);
    assert_eq!(ex.items.len(), 2);
    let d = ex
        .items
        .iter()
        .position(|it| it.name == "d.txt")
        .expect("d");
    assert_eq!(ex.sv.entries[d].folder, drive.display().to_string());
    ex.sv.selection.set_only(Some(d));
    ex.run(Command::Restore, &mut ctx);
    ex.finish_puts(&mut ctx);
    assert_eq!(
        std::fs::read_to_string(drive.join("d.txt")).expect("d"),
        "d"
    );
    assert_eq!(ex.items.len(), 1);

    ex.delete_now(&[drive.join("d.txt")], false, &mut ctx);
    ex.finish_puts(&mut ctx);
    assert_eq!(ex.items.len(), 2, "the bin re-listed with it");
    ex.empty_bin_now(&mut ctx);
    ex.finish_puts(&mut ctx);
    assert!(ex.items.is_empty());
    assert_eq!(std::fs::read_dir(&drive_files).expect("files").count(), 0);
    assert!(ex.dialog.is_none(), "no error box");
}

#[test]
fn an_open_recycle_bin_lists_a_drives_trash_once_udisks_reports_it_mounted() {
    use reprisefs::udisks::{Snapshot, Volume};
    use std::os::unix::fs::MetadataExt;
    let _guard = TRASH_ENV.lock().unwrap_or_else(|e| e.into_inner());
    let home = tempfile::tempdir().expect("a temp dir");
    let _data_home = DataHome::set(home.path());
    let drive = home.path().join("stick");
    let uid = std::fs::metadata(home.path()).expect("stat").uid();
    let files = drive.join(format!(".Trash-{uid}/files"));
    std::fs::create_dir_all(&files).expect("mkdir");
    std::fs::write(files.join("d.txt"), "d").expect("write");
    let table = home.path().join("mounts");
    let home_row = format!("/dev/a {} ext4 rw 0 0\n", home.path().display());
    std::fs::write(&table, &home_row).expect("write");
    reprisefs::trash::set_mount_table(Some(table.clone()));
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::RecycleBin, &mut ctx);
    assert!(ex.items.is_empty());
    let listed = |ex: &Explorer| ex.items.iter().any(|it| it.name == "d.txt");
    let stick = |mounts: Vec<PathBuf>, label: &str| Snapshot {
        volumes: vec![Volume {
            path: "/block/sdx1".into(),
            device: "/dev/sdx1".into(),
            label: Some(label.into()),
            filesystem: true,
            mounts,
            ..Default::default()
        }],
        ..Default::default()
    };

    ex.volumes_changed(stick(Vec::new(), "STICK"), &mut ctx);
    std::fs::write(
        &table,
        format!("{home_row}/dev/sdx1 {} vfat rw 0 0\n", drive.display()),
    )
    .expect("write");
    ex.volumes_changed(stick(vec![drive.clone()], "STICK"), &mut ctx);
    assert!(
        listed(&ex),
        "the stick's trashed file is listed once it mounts"
    );

    std::fs::write(&table, &home_row).expect("write");
    let reply = ex.volumes_changed(stick(vec![drive.clone()], "RENAMED"), &mut ctx);
    assert_eq!(
        reply,
        Reply::none(),
        "a change that moves no mount re-lists nothing"
    );
    assert!(listed(&ex));

    ex.volumes_changed(stick(Vec::new(), "RENAMED"), &mut ctx);
    assert!(!listed(&ex), "its file goes once the stick unmounts");
}

/// Delete on a drive that cannot hold a trash asks to delete for good, and says why.
#[test]
fn delete_on_a_drive_with_no_recycle_bin_says_so() {
    use std::os::unix::fs::PermissionsExt;
    let _guard = TRASH_ENV.lock().unwrap_or_else(|e| e.into_inner());
    let home = tempfile::tempdir().expect("a temp dir");
    let _data_home = DataHome::set(home.path());
    let drive = home.path().join("drive");
    std::fs::create_dir(&drive).expect("mkdir");
    std::fs::write(drive.join("d.txt"), "d").expect("write");
    let table = home.path().join("mounts");
    let rows = format!(
        "/dev/a {} ext4 rw 0 0\n/dev/b {} vfat ro 0 0\n",
        home.path().display(),
        drive.display()
    );
    std::fs::write(&table, rows).expect("write");
    reprisefs::trash::set_mount_table(Some(table));
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(drive.clone()), &mut ctx);
    std::fs::set_permissions(&drive, std::fs::Permissions::from_mode(0o555)).expect("chmod");
    ex.sv.selection.set_only(Some(0));

    ex.run(Command::Delete { erase: false }, &mut ctx);

    std::fs::set_permissions(&drive, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    let Some(Dialog::Confirm(c)) = ex.dialog.as_ref() else {
        panic!("a confirm box");
    };
    assert!(c.erase);
    assert_eq!(
        c.text,
        "This drive has no Recycle Bin. Are you sure you want to delete 'd.txt'?"
    );
}

/// The Recycle Bin lists where each item came from and when it went, in its columns, status bar and
/// sheet, whose Restore puts the item back. Delete there asks, then deletes the item and its record for
/// good, and Undo no longer offers it; Copy copies nothing.
#[test]
fn the_bin_shows_origins_and_deletes_for_good() {
    use reprisekit::controls::{form, propsheet, shellview};
    let _guard = TRASH_ENV.lock().unwrap_or_else(|e| e.into_inner());
    let home = tempfile::tempdir().expect("a temp dir");
    let _data_home = DataHome::set(home.path());
    let dir = home.path().join("folder");
    std::fs::create_dir(&dir).expect("mkdir");
    for n in ["a.txt", "b.txt"] {
        std::fs::write(dir.join(n), n).expect("write");
    }
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.clone()), &mut ctx);
    ex.delete_now(&[dir.join("a.txt"), dir.join("b.txt")], false, &mut ctx);
    ex.finish_puts(&mut ctx);
    ex.run(Command::GoTo(Place::RecycleBin), &mut ctx);
    assert_eq!(ex.items.len(), 2);
    assert_eq!(ex.sv.fields, shellview::bin_fields());
    let shown = dir.display().to_string();
    assert!(ex.sv.entries.iter().all(|e| e.folder == shown));
    assert!(ex.sv.entries.iter().all(|e| !e.deleted.is_empty()));
    let at = |ex: &Explorer, n: &str| ex.items.iter().position(|it| it.name == n).expect(n);

    let a = at(&ex, "a.txt");
    ex.sv.selection.set_only(Some(a));
    ex.refresh_chrome(&mut ctx);
    let status = &ex.status.panels[0].text;
    assert!(
        status.starts_with(&format!("Original Location: {shown} Type: ")),
        "{status}"
    );
    assert!(ex.run(Command::Copy, &mut ctx).copy_files.is_none());

    ex.run(Command::Properties, &mut ctx);
    let p = ex.props.as_ref().expect("the item's sheet");
    assert_eq!(p.restore, ex.bin_item(a));
    assert!(matches!(
        p.sheet.open().item("v_origin").map(|i| &i.control),
        Some(form::Control::Lines(lines)) if lines == &["folder"]
    ));
    let pressed = propsheet::Event::Page(0, form::Event::Pressed(crate::props_dialog::RESTORE));
    ex.props_outcome(pressed, &mut ctx);
    ex.finish_puts(&mut ctx);
    assert!(ex.props.is_none(), "Restore closed the sheet");
    assert_eq!(
        std::fs::read_to_string(dir.join("a.txt")).expect("a"),
        "a.txt"
    );
    assert_eq!(ex.items.len(), 1);

    let b = at(&ex, "b.txt");
    let files = ex.items[b].path.clone();
    let info = ex.items[b]
        .trashed
        .as_ref()
        .expect("b's record")
        .info
        .clone();
    ex.sv.selection.set_only(Some(b));
    ex.run(Command::Delete { erase: false }, &mut ctx);
    let Some(Dialog::Confirm(c)) = ex.dialog.take() else {
        panic!("Delete asks first");
    };
    assert!(c.erase, "the bin's Delete is for good");
    assert_eq!(c.title, "Confirm File Delete");
    assert!(c.text.contains("delete 'b.txt'?"), "{}", c.text);
    let items = c
        .bin
        .expect("raised in the bin, it keeps the items as listed");
    assert_eq!(items, vec![ex.bin_item(b).expect("b")]);
    ex.erase_from_bin(items, &mut ctx);
    ex.finish_puts(&mut ctx);
    assert!(std::fs::symlink_metadata(&files).is_err() && !info.exists());
    assert!(!dir.join("b.txt").exists());
    assert!(ex.items.is_empty());
    assert_eq!(ex.undo, None, "nothing is left to put back");
    assert!(ex.dialog.is_none(), "no error box");
}

/// An item replaced in the bin under the same name since it was listed is left alone: Yes on the prompt
/// over the old one refuses, Restore on its sheet refuses, and the sheet closes on the next re-listing.
#[test]
fn the_bin_acts_only_on_the_item_it_listed() {
    let _guard = TRASH_ENV.lock().unwrap_or_else(|e| e.into_inner());
    let home = tempfile::tempdir().expect("a temp dir");
    let _data_home = DataHome::set(home.path());
    let (one, two) = (home.path().join("one"), home.path().join("two"));
    for d in [&one, &two] {
        std::fs::create_dir(d).expect("mkdir");
        std::fs::write(d.join("a.txt"), d.display().to_string()).expect("write");
    }
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(one.clone()), &mut ctx);
    ex.delete_now(&[one.join("a.txt")], false, &mut ctx);
    ex.finish_puts(&mut ctx);
    ex.run(Command::GoTo(Place::RecycleBin), &mut ctx);
    ex.sv.selection.set_only(Some(0));
    let listed = ex.items[0].trashed.as_deref().cloned().expect("a record");
    ex.run(Command::Properties, &mut ctx);
    ex.run(Command::Delete { erase: false }, &mut ctx);

    reprisefs::trash::restore_free(&listed).expect("put back");
    reprisefs::trash::trash(&two.join("a.txt"), None).expect("trash two");
    ex.dialog_event(key_event_alt('y'), &mut ctx);
    ex.finish_puts(&mut ctx);
    assert!(listed.files.exists(), "the new item stays");
    assert!(
        matches!(ex.dialog, Some(Dialog::Message { .. })),
        "an error box says why"
    );
    ex.dialog = None;

    assert!(ex.props.is_none(), "the sheet went with its item");
    let stale = crate::fileops::BinItem {
        path: listed.files.clone(),
        record: Some(listed.clone()),
    };
    ex.restore_items(vec![stale], &mut ctx);
    ex.finish_puts(&mut ctx);
    assert!(listed.files.exists() && !two.join("a.txt").exists());
    assert!(matches!(ex.dialog, Some(Dialog::Message { .. })));
}

/// Resizing a column or sorting in the Recycle Bin leaves the folder columns and sort settings alone; a folder still saves both.
#[test]
fn the_bins_columns_and_sort_are_not_saved_as_the_folders() {
    let _guard = TRASH_ENV.lock().unwrap_or_else(|e| e.into_inner());
    let home = tempfile::tempdir().expect("a temp dir");
    let _data_home = DataHome::set(home.path());
    let dir = home.path().join("folder");
    std::fs::create_dir(&dir).expect("mkdir");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.clone()), &mut ctx);
    let (columns, sort) = (ex.settings.columns.clone(), ex.settings.sort);
    let release = |ex: &mut Explorer, ctx: &mut Ctx| {
        ex.pointer(
            repriseapp::PointerInput::Release {
                x: 0,
                y: 0,
                button: crate::BTN_LEFT,
                mods: repriseapp::Mods::default(),
            },
            ctx,
        );
    };

    ex.run(Command::GoTo(Place::RecycleBin), &mut ctx);
    ex.resizing = Some((1, 100, 180));
    ex.sv.columns[1].width = 240;
    release(&mut ex, &mut ctx);
    ex.run(Command::Sort(4), &mut ctx);
    assert_eq!(ex.sv.sort, Some((4, true)), "the bin sorts by Type");
    assert_eq!(ex.settings.columns, columns, "folder widths kept");
    assert_eq!(ex.settings.sort, sort, "folder sort kept");

    ex.run(Command::GoTo(Place::Folder(dir)), &mut ctx);
    assert_eq!(ex.sv.columns.len(), 4);
    ex.resizing = Some((1, 100, 94));
    ex.sv.columns[1].width = 120;
    release(&mut ex, &mut ctx);
    ex.run(Command::Sort(2), &mut ctx);
    assert_eq!(ex.settings.columns[1], 120, "a folder saves its widths");
    assert_eq!(ex.settings.sort, (2, true), "a folder saves its sort");
}

/// Emptying the Recycle Bin drops the undos that would have restored a trashed file and keeps every
/// other one; a failed empty leaves the stack alone.
#[test]
fn emptying_the_bin_drops_only_the_trash_undos() {
    use std::os::unix::fs::PermissionsExt;
    let _guard = TRASH_ENV.lock().unwrap_or_else(|e| e.into_inner());
    let home = tempfile::tempdir().expect("a temp dir");
    let _data_home = DataHome::set(home.path());
    let dir = home.path().join("folder");
    std::fs::create_dir(&dir).expect("mkdir");
    std::fs::write(dir.join("a.txt"), "1").expect("write");
    std::fs::rename(dir.join("a.txt"), dir.join("moved.txt")).expect("rename");
    std::fs::write(dir.join("b.txt"), "2").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.clone()), &mut ctx);

    ex.undos.push(Undoable::Move {
        from: dir.join("a.txt"),
        to: dir.join("moved.txt"),
        listed: false,
    });
    ex.delete_now(&[dir.join("b.txt")], false, &mut ctx);
    ex.finish_puts(&mut ctx);
    assert_eq!(ex.undo, Some("Delete"));

    let files = home.path().join("Trash/files");
    let _writable_again = Mode0700(files.clone());
    let mode = |m: u32| {
        std::fs::set_permissions(&files, std::fs::Permissions::from_mode(m)).expect("chmod")
    };
    mode(0o500);
    ex.empty_bin_now(&mut ctx);
    ex.finish_puts(&mut ctx);
    assert!(
        matches!(ex.dialog, Some(crate::dialogs::Dialog::Message { .. })),
        "the failure is reported"
    );
    assert_eq!(std::fs::read_dir(&files).expect("the trash").count(), 1);
    assert_eq!(ex.undo, Some("Delete"), "nothing was forgotten");

    mode(0o700);
    ex.dialog = None;
    ex.empty_bin_now(&mut ctx);
    ex.finish_puts(&mut ctx);

    assert_eq!(std::fs::read_dir(&files).expect("the trash").count(), 0);
    assert_eq!(ex.undo, Some("Move"));
}

#[test]
fn a_paste_into_the_same_folder_lands_as_a_copy() {
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::write(dir.path().join("a.txt"), "1").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);

    let reply = ex.run(Command::Paste, &mut ctx);
    assert!(reply.paste_files, "the clipboard was asked for");
    ex.pasted(
        FileClip {
            cut: false,
            paths: vec![dir.path().join("a.txt")],
        },
        &mut ctx,
    );
    assert!(
        ex.put.is_some(),
        "the copy runs on a job, not in the handler"
    );
    ex.finish_puts(&mut ctx);

    assert!(dir.path().join("Copy of a.txt").exists());
    assert!(ex.items.iter().any(|it| it.name == "Copy of a.txt"));
    assert_eq!(ex.undo, Some("Copy"));
}

#[test]
fn create_shortcut_writes_a_launcher_beside_the_file() {
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::write(dir.path().join("a.txt"), "1").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.sv.selection.set_only(Some(0));

    ex.run(Command::CreateShortcut, &mut ctx);
    ex.finish_puts(&mut ctx);

    let text = std::fs::read_to_string(dir.path().join("Shortcut to a.txt.desktop"))
        .expect("the shortcut");
    assert!(text.starts_with("[Desktop Entry]\nType=Application\n"));
    assert!(text.contains("Name=Shortcut to a.txt\n"));
    let target = dir.path().join("a.txt");
    let exec = reprisefs::apps::exec_arg(&target.to_string_lossy());
    assert!(text.contains(&format!("Exec=xdg-open {exec}\n")), "{text}");
}

/// Cut puts the selection on the clipboard as a cut and fades its entries; Copy does neither.
#[test]
fn cut_marks_its_entries_and_copy_does_not() {
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::write(dir.path().join("a.txt"), "1").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.sv.selection.set_only(Some(0));

    let reply = ex.run(Command::Cut, &mut ctx);
    assert_eq!(
        reply.copy_files,
        Some(FileClip {
            cut: true,
            paths: vec![dir.path().join("a.txt")],
        })
    );
    assert!(ex.sv.entries[0].cut);

    let reply = ex.run(Command::Copy, &mut ctx);
    assert_eq!(reply.copy_files.map(|c| c.cut), Some(false));
    assert!(!ex.sv.entries[0].cut);
}

/// A rename joins the undo stack: Edit's first row offers it and Undo restores the old name.
#[test]
fn undo_reverses_a_rename() {
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::write(dir.path().join("a.txt"), "1").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);

    ex.sv.edit = Some((0, reprisekit::textedit::TextEdit::new("b.txt", false)));
    ex.commit_rename(&mut ctx);
    assert_eq!(ex.undo, Some("Rename"));

    ex.run(Command::Undo, &mut ctx);

    assert!(dir.path().join("a.txt").is_file());
    assert!(!dir.path().join("b.txt").exists());
}

/// The folder watch fires as New > Folder writes its folder; that re-listing must not take the rename box away.
#[test]
fn a_refresh_keeps_an_open_rename_box() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.run(Command::NewFolder, &mut ctx);
    assert!(ex.sv.edit.is_some(), "the rename box opened");

    std::fs::write(dir.path().join("a.txt"), "1").expect("write");
    ex.refresh(&mut ctx);

    let (i, _) = ex.sv.edit.as_ref().expect("the rename box is still up");
    assert_eq!(ex.items[*i].name, "New Folder");
}

/// The clipboard answers after Paste asked, so the window may have walked off in between. The Recycle
/// Bin has a real path, so "has a path" is not enough: the paste drops rather than writing files into the trash.
#[test]
fn a_paste_that_arrives_after_navigating_away_is_dropped() {
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::write(dir.path().join("a.txt"), "1").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);

    ex.run(Command::Paste, &mut ctx);
    ex.run(Command::GoTo(Place::RecycleBin), &mut ctx);
    let reply = ex.pasted(
        FileClip {
            cut: false,
            paths: vec![dir.path().join("a.txt")],
        },
        &mut ctx,
    );
    assert_eq!(reply, Reply::none());
    assert!(ex.put.is_none(), "nothing was started");
    assert!(!dir.path().join("Copy of a.txt").exists());
    assert!(!ex.run(Command::Paste, &mut ctx).paste_files);
}

/// A folder copied into itself would recurse for ever; reprisefs refuses and Explorer says so in an error box.
#[test]
fn pasting_a_folder_into_itself_is_refused_with_an_error_box() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let reports = dir.path().join("Reports");
    std::fs::create_dir(&reports).expect("mkdir");
    std::fs::write(reports.join("x.txt"), "1").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(reports.clone()), &mut ctx);

    ex.run(Command::Paste, &mut ctx);
    ex.pasted(
        FileClip {
            cut: false,
            paths: vec![reports.clone()],
        },
        &mut ctx,
    );
    ex.finish_puts(&mut ctx);

    match ex.dialog.as_ref().expect("the error box is up") {
        crate::dialogs::Dialog::Message { title, text } => {
            assert_eq!(title, "Error Copying File or Folder");
            assert_eq!(
                text,
                "Cannot copy Reports: The destination folder is a subfolder of \
                 the source folder."
            );
        }
        _ => panic!("the refusal raises a message box"),
    }
    assert_eq!(
        std::fs::read_dir(&reports).expect("read").count(),
        1,
        "nothing was created"
    );
}

/// A cut of two files pasted where the second is gone: the first moves, the second fails with an error
/// box, and the one undo holds only the move that happened.
#[test]
fn a_paste_whose_second_file_fails_keeps_the_first_and_names_the_second() {
    let home = tempfile::tempdir().expect("a temp dir");
    let from = home.path().join("from");
    let dir = home.path().join("to");
    std::fs::create_dir_all(&from).expect("mkdir");
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(from.join("a.txt"), "a").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.clone()), &mut ctx);

    ex.run(Command::Paste, &mut ctx);
    ex.pasted(
        FileClip {
            cut: true,
            paths: vec![from.join("a.txt"), from.join("gone.txt")],
        },
        &mut ctx,
    );
    ex.finish_puts(&mut ctx);

    assert!(dir.join("a.txt").is_file() && !from.join("a.txt").exists());
    match ex.dialog.as_ref().expect("the error box is up") {
        Dialog::Message { title, text } => {
            assert_eq!(title, "Error Moving File or Folder");
            assert!(text.starts_with("Cannot move gone.txt: "), "{text}");
        }
        _ => panic!("the failure raises a message box"),
    }
    assert_eq!(ex.undo, Some("Move"));
    ex.run(Command::Undo, &mut ctx);
    assert!(from.join("a.txt").is_file(), "the one move is undone");
    assert_eq!(ex.undo, None, "and that was the whole batch");
}

#[test]
fn a_file_pasted_onto_a_folders_name_lands_numbered() {
    let home = tempfile::tempdir().expect("a temp dir");
    let (from, dir) = (home.path().join("from"), home.path().join("to"));
    std::fs::create_dir_all(&from).expect("mkdir");
    std::fs::create_dir_all(dir.join("b.txt")).expect("mkdir");
    std::fs::write(from.join("b.txt"), "b").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.clone()), &mut ctx);
    ex.run(Command::Paste, &mut ctx);
    ex.pasted(
        FileClip {
            cut: false,
            paths: vec![from.join("b.txt")],
        },
        &mut ctx,
    );
    assert!(ex.dialog.is_none(), "nothing to ask");
    ex.finish_puts(&mut ctx);
    assert!(dir.join("b.txt").is_dir());
    assert_eq!(
        std::fs::read_to_string(dir.join("b (2).txt")).expect("read"),
        "b"
    );
}

#[test]
fn erasing_a_symlink_to_a_folder_removes_only_the_link() {
    let home = tempfile::tempdir().expect("a temp dir");
    let target = home.path().join("target");
    let dir = home.path().join("dir");
    std::fs::create_dir_all(&target).expect("mkdir");
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(target.join("x.txt"), "x").expect("write");
    let link = dir.join("link");
    std::os::unix::fs::symlink(&target, &link).expect("symlink");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.clone()), &mut ctx);

    ex.delete_now(std::slice::from_ref(&link), true, &mut ctx);
    ex.finish_puts(&mut ctx);

    assert!(!link.exists() && std::fs::symlink_metadata(&link).is_err());
    assert!(target.join("x.txt").is_file(), "the target is untouched");
    assert!(ex.dialog.is_none());
}

/// Undo of a move whose source folder vanished fails, says so, and keeps the batch, so Undo again once
/// the folder is back finishes the job.
#[test]
fn undo_after_the_source_folder_vanished_keeps_the_batch_for_a_retry() {
    let home = tempfile::tempdir().expect("a temp dir");
    let from = home.path().join("from");
    let dir = home.path().join("to");
    std::fs::create_dir_all(&from).expect("mkdir");
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(from.join("a.txt"), "a").expect("write");
    std::fs::write(from.join("b.txt"), "b").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.clone()), &mut ctx);

    ex.run(Command::Paste, &mut ctx);
    ex.pasted(
        FileClip {
            cut: true,
            paths: vec![from.join("a.txt"), from.join("b.txt")],
        },
        &mut ctx,
    );
    ex.finish_puts(&mut ctx);
    assert_eq!(ex.undo, Some("Move"));

    std::fs::remove_dir_all(&from).expect("rmdir");
    ex.run(Command::Undo, &mut ctx);
    match ex.dialog.as_ref().expect("the failure is reported") {
        Dialog::Message { title, text } => {
            assert_eq!(title, "Error");
            assert!(text.starts_with("Cannot undo: "), "{text}");
        }
        _ => panic!("a message box"),
    }
    assert_eq!(ex.undo, Some("Move"), "the batch stays for a retry");
    assert!(dir.join("a.txt").is_file() && dir.join("b.txt").is_file());

    ex.dialog = None;
    std::fs::create_dir_all(&from).expect("mkdir");
    ex.run(Command::Undo, &mut ctx);
    assert!(from.join("a.txt").is_file() && from.join("b.txt").is_file());
    assert_eq!(ex.undo, None);
}

/// A move takes an item off its old folder's `.hidden` and Undo Move puts it back, so it is hidden
/// again where it came from. One the list never named stays off it.
#[test]
fn undo_of_a_move_lists_a_hidden_item_again() {
    let home = tempfile::tempdir().expect("a temp dir");
    let from = home.path().join("from");
    let dir = home.path().join("to");
    std::fs::create_dir_all(&from).expect("mkdir");
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(from.join("a.txt"), "a").expect("write");
    std::fs::write(from.join("b.txt"), "b").expect("write");
    std::fs::write(from.join(".hidden"), "a.txt\n").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.clone()), &mut ctx);

    ex.run(Command::Paste, &mut ctx);
    ex.pasted(
        FileClip {
            cut: true,
            paths: vec![from.join("a.txt"), from.join("b.txt")],
        },
        &mut ctx,
    );
    ex.finish_puts(&mut ctx);
    let list = || std::fs::read_to_string(from.join(".hidden")).unwrap_or_default();
    assert_eq!(list(), "", "the move let go of the line");

    ex.run(Command::Undo, &mut ctx);
    assert!(from.join("a.txt").is_file() && from.join("b.txt").is_file());
    assert_eq!(list(), "a.txt\n");
}

#[test]
fn send_to_zip_packs_the_set_into_one_archive_named_after_the_focus() {
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::write(dir.path().join("a.txt"), "1").expect("write");
    std::fs::write(dir.path().join("b.txt"), "2").expect("write");
    std::fs::write(dir.path().join("b.zip"), "").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    let at = |name: &str| {
        ex.items
            .iter()
            .position(|it| it.path.ends_with(name))
            .unwrap()
    };
    let (a, b) = (at("a.txt"), at("b.txt"));
    ex.sv.selection.set_only(Some(a));
    ex.sv.selection.set.insert(b);
    ex.sv.selection.focus = Some(b);
    ex.send_to = vec![SendToRow {
        label: "Compressed (zipped) Folder".into(),
        icon: "application-zip",
        dest: SendTo::Zip,
        enabled: true,
    }];

    let reply = ex.send_to(0, &mut ctx);
    let d = dir.path();
    assert_eq!(
        reply.spawn,
        vec![vec![
            OsString::from("bsdtar"),
            "-a".into(),
            "-cf".into(),
            d.join("b (2).zip").into(),
            "-C".into(),
            d.into(),
            "--".into(),
            "b.txt".into(),
            "a.txt".into(),
        ]],
        "one archive, the focused item first and naming it"
    );
}

#[test]
fn send_to_a_folder_copies_on_a_job() {
    let _guard = TRASH_ENV.lock().unwrap_or_else(|e| e.into_inner());
    let home = tempfile::tempdir().expect("a temp dir");
    let _data_home = DataHome::set(home.path());
    let dir = tempfile::tempdir().expect("a temp dir");
    let dest = tempfile::tempdir().expect("a temp dir");
    std::fs::write(dir.path().join("a.txt"), "1").expect("write");
    std::fs::write(dir.path().join("b.txt"), "2").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.sv.selection.set_only(Some(0));
    ex.sv.selection.set.insert(1);
    ex.send_to = vec![SendToRow {
        label: "Docs".into(),
        icon: "folder-documents",
        dest: SendTo::Folder(dest.path().to_path_buf()),
        enabled: true,
    }];

    ex.send_to(0, &mut ctx);
    assert!(
        ex.put.is_some(),
        "the copy runs on a job, not in the handler"
    );
    ex.finish_puts(&mut ctx);

    assert!(dest.path().join("a.txt").exists());
    assert!(dest.path().join("b.txt").exists());
    assert!(dir.path().join("a.txt").exists(), "Send To never moves");
    assert_eq!(ex.undo, Some("Copy"));
    ex.run(Command::Undo, &mut ctx);
    ex.finish_puts(&mut ctx);
    assert!(!dest.path().join("a.txt").exists(), "one undo takes both");
    assert!(!dest.path().join("b.txt").exists(), "one undo takes both");
}

#[test]
fn the_empty_bin_prompt_counts_the_bin() {
    let dir = tempfile::tempdir().expect("a temp dir");
    for name in ["a.txt", "b.txt", "c.txt"] {
        std::fs::write(dir.path().join(name), "1").expect("write");
    }
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.confirm_empty_bin(&mut ctx);
    match ex.dialog.as_ref() {
        Some(Dialog::Confirm(c)) => assert_eq!(
            c.text.replace('\n', " "),
            "Are you sure you want to delete these 3 items?"
        ),
        other => panic!("no confirm box: {:?}", other.is_some()),
    }
}

#[test]
fn cancel_drops_the_pastes_waiting_behind_the_running_one() {
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::write(dir.path().join("a.txt"), "1").expect("write");
    std::fs::write(dir.path().join("b.txt"), "2").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    for name in ["a.txt", "b.txt"] {
        ex.run(Command::Paste, &mut ctx);
        ex.pasted(
            FileClip {
                cut: false,
                paths: vec![dir.path().join(name)],
            },
            &mut ctx,
        );
    }
    assert_eq!(ex.put_queue.len(), 1, "the second paste waits");

    ex.cancel_puts();
    assert!(ex.put_queue.is_empty());
    ex.finish_puts(&mut ctx);

    assert!(
        !dir.path().join("Copy of b.txt").exists(),
        "the waiting paste never runs"
    );
}

/// Close during a copy hides the window and quits once the copy has landed and recorded its undo.
#[test]
fn close_waits_for_a_copy_in_flight() {
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::write(dir.path().join("a.txt"), "1").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.run(Command::Paste, &mut ctx);
    ex.pasted(
        FileClip {
            cut: false,
            paths: vec![dir.path().join("a.txt")],
        },
        &mut ctx,
    );
    assert!(ex.put.is_some());

    let reply = ex.run(Command::Close, &mut ctx);
    assert!(!reply.quit, "the copy is still out");
    assert!(ex.closing);

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let quit = loop {
        if ex.wake(&mut ctx).quit {
            break true;
        }
        if std::time::Instant::now() > deadline {
            break false;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    };
    assert!(quit, "the wake after the copy quits");
    assert!(dir.path().join("Copy of a.txt").exists());
    assert_eq!(ex.undo, Some("Copy"));
}

#[test]
fn close_when_idle_quits_at_once() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    assert!(ex.run(Command::Close, &mut ctx).quit);
    assert!(!ex.closing);
}

/// Open With on one entry of a multi-selection runs the program in its app scope and raises the
/// chooser on the entry the menu was built for.
#[test]
fn open_with_runs_the_program_as_its_entry_on_the_entry_the_menu_was_built_for() {
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::write(dir.path().join("a.txt"), "1").expect("write");
    std::fs::write(dir.path().join("b.txt"), "2").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.app_index = vec![app("Editor", "editor", &["text/plain"])];
    for it in &mut ex.items {
        it.mime = Some("text/plain".into());
    }
    let b = ex
        .items
        .iter()
        .position(|it| it.name == "b.txt")
        .expect("b.txt is listed");
    ex.sv.selection.set = (0..ex.items.len()).collect();
    assert_eq!(ex.build_open_with(b, &mut ctx).len(), 1);

    let reply = ex.run(Command::OpenWith(0), &mut ctx);
    let argv = reply.spawn.first().expect("the program runs");
    assert_eq!(
        argv.last(),
        Some(&dir.path().join("b.txt").into_os_string())
    );
    assert_eq!(
        reply.spawn_app.as_deref(),
        Some("editor.desktop"),
        "in the program's app scope"
    );

    ex.run(Command::ChooseProgram, &mut ctx);
    match ex.dialog.as_ref() {
        Some(Dialog::ChooseProgram(c)) => assert_eq!(c.path, dir.path().join("b.txt")),
        other => panic!("no chooser: {:?}", other.is_some()),
    }
    match ex.dialog.as_ref() {
        Some(Dialog::ChooseProgram(c)) => {
            assert_eq!(
                c.row_icons.len(),
                c.ow.list.rows.len(),
                "one icon slot per row"
            );
            assert!(c.ow.always.enabled, "Open With leaves the box live");
        }
        other => panic!("expected the chooser, got {:?}", other.is_some()),
    }
}

/// A chooser opened from the keyboard shows its cues from the start; one opened with the pointer waits for a key.
#[test]
fn a_chooser_opened_by_key_shows_its_cues() {
    use reprisekit::cues::Cues;
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("a.txt");
    std::fs::write(&path, "1").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    let cues = |ex: &crate::Explorer| match ex.dialog.as_ref() {
        Some(Dialog::ChooseProgram(c)) => c.input.cues(),
        other => panic!("expected the chooser, got {:?}", other.is_some()),
    };
    ex.choose_program_for(&path, false, &mut ctx);
    assert_eq!(cues(&ex), Cues::NONE, "opened with the pointer");
    ex.keyed = true;
    ex.choose_program_for(&path, false, &mut ctx);
    assert_eq!(cues(&ex), Cues::ALL, "opened from the keyboard");
}

/// OK in the chooser acts on release: a press only arms it, the release over it runs the program.
#[test]
fn chooser_ok_runs_the_program_on_release() {
    use repriseapp::{Event, Mods, PointerInput};
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("a.txt");
    std::fs::write(&path, "1").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.app_index = vec![app("Editor", "editor", &["text/plain"])];
    ex.choose_program_for(&path, false, &mut ctx);
    let ok = match ex.dialog.as_ref() {
        Some(Dialog::ChooseProgram(c)) => {
            c.ow.layout(&ex.dialog_text, ex.dialog_px, (0, 0), &ctx.theme.controls())
                .ok
        }
        other => panic!("expected the chooser, got {:?}", other.is_some()),
    };
    let (x, y) = (ok.x + ok.width / 2, ok.y + ok.height / 2);
    let press = Event::Pointer(PointerInput::Press {
        x,
        y,
        button: crate::BTN_LEFT,
        time: 1000,
        mods: Mods::default(),
    });
    let reply = ex.dialog_event(press, &mut ctx);
    assert!(reply.spawn.is_empty(), "the press only arms OK");
    let release = Event::Pointer(PointerInput::Release {
        x,
        y,
        button: crate::BTN_LEFT,
        mods: Mods::default(),
    });
    let reply = ex.dialog_event(release, &mut ctx);
    let argv = reply.spawn.first().expect("the release runs the program");
    assert_eq!(argv.last(), Some(&path.clone().into_os_string()));
    assert_eq!(reply.spawn_app.as_deref(), Some("editor.desktop"));
    assert!(ex.dialog.is_none(), "the chooser closes");
}

#[test]
fn ok_on_a_browsed_program_runs_it_on_the_file() {
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::write(dir.path().join("b.txt"), "").unwrap();
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.choose_program_for(&dir.path().join("b.txt"), false, &mut ctx);
    let c = match ex.dialog.take() {
        Some(Dialog::ChooseProgram(c)) => c,
        _ => panic!("expected the chooser"),
    };
    let reply = ex.choose_program_ok(*c, "/usr/bin/nano".into(), true, &mut ctx);
    let argv = reply.spawn.first().expect("one command");
    assert_eq!(argv[0], std::ffi::OsString::from("/usr/bin/nano"));
    assert_eq!(argv[1], dir.path().join("b.txt").into_os_string());
}

#[test]
fn browse_opens_the_program_picker_over_the_chooser() {
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::write(dir.path().join("b.txt"), "").unwrap();
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.choose_program_for(&dir.path().join("b.txt"), false, &mut ctx);
    ex.chooser_event(key_event_alt('b'), &mut ctx);
    let kids = ex.children();
    assert!(
        kids.iter()
            .any(|k| k.id == crate::dialogs::PROGRAM_BROWSE && k.modal),
        "the picker is a modal child"
    );
    assert!(
        kids.iter().any(|k| k.id == crate::dialogs::DIALOG),
        "the chooser stays up under it"
    );
}

fn key_event_escape() -> repriseapp::Event {
    repriseapp::Event::Key(repriseapp::KeyInput {
        sym: reprisekit::textedit::keysym::ESCAPE,
        utf8: None,
        mods: repriseapp::Mods {
            ctrl: false,
            shift: false,
            alt: false,
        },
        edit: None,
    })
}

/// Two files and a folder pasted into `dest` where all three names are taken: the replace prompt comes up before anything is written.
fn paste_onto_taken_names(
    ctx: &mut repriseapp::Ctx,
) -> (tempfile::TempDir, std::path::PathBuf, Explorer) {
    let t = tempfile::tempdir().expect("a temp dir");
    let (src, dest) = (t.path().join("src"), t.path().join("dest"));
    for d in [&src, &dest] {
        std::fs::create_dir_all(d.join("Reports")).expect("mkdir");
        std::fs::write(d.join("Reports/r.txt"), d.to_string_lossy().as_bytes()).expect("write");
    }
    for (name, body) in [("a.txt", "new a"), ("b.txt", "new b")] {
        std::fs::write(src.join(name), body).expect("write");
        std::fs::write(dest.join(name), "old").expect("write");
    }
    let mut ex = crate::tests::test_explorer(Place::Folder(dest.clone()), ctx);
    ex.run(Command::Paste, ctx);
    ex.pasted(
        FileClip {
            cut: false,
            paths: ["a.txt", "b.txt", "Reports"]
                .iter()
                .map(|n| src.join(n))
                .collect(),
        },
        ctx,
    );
    assert!(ex.put.is_none(), "nothing runs before the answer");
    (t, dest, ex)
}

fn asked(ex: &Explorer) -> Option<(&'static str, String)> {
    match &ex.dialog {
        Some(Dialog::Replace(r)) => Some((r.fields.title(), r.fields.name.clone())),
        _ => None,
    }
}

/// Yes replaces the first file, No keeps the second, and the folder prompt's Yes merges.
#[test]
fn the_replace_prompt_asks_per_clash_and_carries_out_each_answer() {
    let mut ctx = test_ctx();
    let (_t, dest, mut ex) = paste_onto_taken_names(&mut ctx);
    assert_eq!(
        asked(&ex),
        Some(("Confirm File Replace", "a.txt".to_string()))
    );
    ex.dialog_event(key_event_alt('y'), &mut ctx);
    assert_eq!(
        asked(&ex),
        Some(("Confirm File Replace", "b.txt".to_string()))
    );
    ex.dialog_event(key_event_alt('n'), &mut ctx);
    assert_eq!(
        asked(&ex),
        Some(("Confirm Folder Replace", "Reports".to_string()))
    );
    ex.dialog_event(key_event_alt('y'), &mut ctx);
    assert!(ex.dialog.is_none());
    ex.finish_puts(&mut ctx);

    let read = |n: &str| std::fs::read_to_string(dest.join(n)).expect("read");
    assert_eq!(read("a.txt"), "new a");
    assert_eq!(read("b.txt"), "old");
    assert!(
        read("Reports/r.txt").ends_with("src"),
        "the merge replaced r.txt"
    );
    assert!(!dest.join("a (2).txt").exists() && !dest.join("Copy of a.txt").exists());
    assert_eq!(ex.undo, None, "a replacement cannot be undone");
}

/// Right twice moves the replace prompt's focus from Yes past All to No and Enter presses No; the next
/// question's focus is back on Yes.
#[test]
fn arrows_move_the_replace_prompt_s_focus_and_enter_presses_it() {
    let mut ctx = test_ctx();
    let (_t, dest, mut ex) = paste_onto_taken_names(&mut ctx);
    ex.dialog_event(right(), &mut ctx);
    ex.dialog_event(right(), &mut ctx);
    assert_eq!(
        asked(&ex),
        Some(("Confirm File Replace", "a.txt".to_string())),
        "the arrows leave a.txt's question up"
    );
    ex.dialog_event(enter(), &mut ctx);
    assert_eq!(
        asked(&ex),
        Some(("Confirm File Replace", "b.txt".to_string())),
        "Enter on No answers a.txt"
    );
    ex.dialog_event(enter(), &mut ctx);
    assert_eq!(
        asked(&ex),
        Some(("Confirm Folder Replace", "Reports".to_string())),
        "Enter on Yes answers b.txt"
    );
    ex.dialog_event(key_event_alt('y'), &mut ctx);
    ex.finish_puts(&mut ctx);
    let read = |n: &str| std::fs::read_to_string(dest.join(n)).expect("read");
    assert_eq!(read("a.txt"), "old", "No kept a.txt");
    assert_eq!(read("b.txt"), "new b", "Yes replaced b.txt");
}

#[test]
fn yes_to_all_replaces_the_rest_and_cancel_puts_nothing() {
    let mut ctx = test_ctx();
    let (_t, dest, mut ex) = paste_onto_taken_names(&mut ctx);
    ex.dialog_event(key_event_alt('a'), &mut ctx);
    assert!(ex.dialog.is_none(), "no more questions");
    ex.finish_puts(&mut ctx);
    assert_eq!(
        std::fs::read_to_string(dest.join("b.txt")).expect("read"),
        "new b"
    );

    let (_t, dest, mut ex) = paste_onto_taken_names(&mut ctx);
    ex.dialog_event(key_event_alt('y'), &mut ctx);
    ex.dialog_event(key_event_escape(), &mut ctx);
    assert!(ex.dialog.is_none() && ex.put.is_none());
    assert_eq!(
        std::fs::read_to_string(dest.join("a.txt")).expect("read"),
        "old"
    );
}

/// An error box raised while a replace prompt is up waits for it, and a second batch's prompt comes
/// after the first's: neither is lost.
#[test]
fn a_box_or_a_second_prompt_waits_for_the_prompt_up() {
    let mut ctx = test_ctx();
    let (_t, dest, mut ex) = paste_onto_taken_names(&mut ctx);
    ex.message_box(
        "Error Copying File or Folder",
        "Cannot copy x".into(),
        &mut ctx,
    );
    assert!(asked(&ex).is_some(), "the prompt stays up");

    let other = tempfile::tempdir().expect("a temp dir");
    std::fs::write(other.path().join("a.txt"), "other a").expect("write");
    ex.run(Command::Paste, &mut ctx);
    ex.pasted(
        FileClip {
            cut: false,
            paths: vec![other.path().join("a.txt")],
        },
        &mut ctx,
    );
    assert_eq!(
        asked(&ex),
        Some(("Confirm File Replace", "a.txt".to_string())),
        "still the first batch's question"
    );
    ex.dialog_event(key_event_escape(), &mut ctx);
    assert_eq!(
        asked(&ex),
        Some(("Confirm File Replace", "a.txt".to_string())),
        "the second batch asks next"
    );
    assert!(matches!(&ex.dialog, Some(Dialog::Replace(r)) if !r.fields.many));
    ex.dialog_event(key_event_alt('y'), &mut ctx);
    match &ex.dialog {
        Some(Dialog::Message { text, .. }) => assert_eq!(text, "Cannot copy x"),
        _ => panic!("the waiting box comes last"),
    }
    ex.finish_puts(&mut ctx);
    assert_eq!(
        std::fs::read_to_string(dest.join("a.txt")).expect("read"),
        "other a"
    );
}

/// A Yes on a prompt whose file changed since it was asked numbers instead of replacing, and records its undo.
#[test]
fn a_stale_yes_lands_numbered_with_its_undo() {
    let mut ctx = test_ctx();
    let (_t, dest, mut ex) = paste_onto_taken_names(&mut ctx);
    std::fs::remove_file(dest.join("a.txt")).expect("remove");
    std::fs::write(dest.join("a.txt"), "another batch's").expect("write");
    ex.dialog_event(key_event_alt('y'), &mut ctx);
    ex.dialog_event(key_event_alt('n'), &mut ctx);
    ex.dialog_event(key_event_alt('n'), &mut ctx);
    ex.finish_puts(&mut ctx);
    assert_eq!(
        std::fs::read_to_string(dest.join("a.txt")).expect("read"),
        "another batch's"
    );
    assert_eq!(
        std::fs::read_to_string(dest.join("a (2).txt")).expect("read"),
        "new a"
    );
    assert_eq!(ex.undo, Some("Copy"), "the numbered copy can be undone");
}

/// Paste on a launcher opens each clipboard file in its program, in its app scope, and copies
/// nothing into the folder.
#[test]
fn paste_on_a_launcher_opens_the_clipboards_files_in_its_program() {
    let dir = tempfile::tempdir().expect("a temp dir");
    reprisefs::apps::write_launcher(
        &dir.path().join("editor.desktop"),
        "[Desktop Entry]\nType=Application\nName=Editor\nExec=editor %f\n",
    )
    .expect("write");
    std::fs::write(dir.path().join("a.txt"), "1").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    let launcher = ex
        .items
        .iter()
        .position(|it| it.shortcut.is_some())
        .expect("the launcher is listed");

    ex.entry_menu_at(launcher, (0, 0), &mut ctx);
    let reply = ex.run(Command::PasteOpen, &mut ctx);
    assert!(reply.paste_files, "the clipboard was asked for");
    let paths = vec![dir.path().join("a.txt"), PathBuf::from("/srv/b.txt")];
    let reply = ex.pasted(FileClip { cut: true, paths }, &mut ctx);
    assert_eq!(
        reply.spawn,
        [
            vec![OsString::from("editor"), dir.path().join("a.txt").into()],
            vec![OsString::from("editor"), OsString::from("/srv/b.txt")],
        ]
    );
    assert_eq!(reply.spawn_app.as_deref(), Some("editor.desktop"));
    assert!(ex.put.is_none(), "nothing is copied");
    assert!(dir.path().join("a.txt").exists(), "a cut file stays put");

    let reply = ex.run(Command::Paste, &mut ctx);
    assert!(reply.paste_files);
    ex.pasted(
        FileClip {
            cut: false,
            paths: vec![dir.path().join("a.txt")],
        },
        &mut ctx,
    );
    assert!(
        ex.put.is_some(),
        "a later Paste copies into the folder again"
    );
}
