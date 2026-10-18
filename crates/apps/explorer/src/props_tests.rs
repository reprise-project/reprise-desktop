//! The Properties sheet's model: the rows a file and a folder get, their
//! geometry, what Apply writes, and the writes on a temporary folder.

use super::*;
use reprisekit::controls::filesheet::{ATTR_LABELS, ATTR_ONLY, ATTR_TREE, HIDDEN, NAME};

const FOLDER: reprisekit::controls::filesheet::Subject =
    reprisekit::controls::filesheet::Subject::Folder;
const RO_TRUE: reprisefs::props::AttrMoves = reprisefs::props::AttrMoves {
    read_only: Some(true),
    backup: None,
    index: None,
    executable: None,
};
const RO_FALSE: reprisefs::props::AttrMoves = reprisefs::props::AttrMoves {
    read_only: Some(false),
    backup: None,
    index: None,
    executable: None,
};
use reprisekit::controls::form::Item;
use reprisekit::raster::Canvas;
use reprisekit::textedit::TextEdit;
use reprisekit::theme::{FontConfig, TextUnits, Theme};
use std::path::PathBuf;

fn text() -> TextRenderer {
    TextRenderer::load(&FontConfig::default())
}

fn px() -> u32 {
    Theme::default().fonts.size_px
}

fn info(is_dir: bool) -> FileInfo {
    FileInfo {
        name: "New Text Document".into(),
        hidden_ext: ".zip".into(),
        path: PathBuf::from("/home/user/Desktop/New Text Document.zip"),
        is_dir,
        type_name: if is_dir {
            "File Folder".into()
        } else {
            "Compressed (zipped) Folder".into()
        },
        opens_with: (!is_dir).then(|| "Compressed (zipped) Folders".to_string()),
        opens_with_icon: None,
        location: "/home/user/Desktop".into(),
        size: 140,
        on_disk: 4096,
        contains: is_dir.then_some((0, 1)),
        created: Some(1_757_000_000),
        modified: 1_757_000_000,
        accessed: 1_757_000_000,
        read_only: false,
        executable: false,
        can_set_executable: !is_dir,
        hidden: false,
        listed: false,
        is_shortcut: false,
        robots: None,
        summary: None,
        own_icon: None,
        launcher: None,
        description: None,
    }
}

fn labels(rows: &[Row]) -> Vec<String> {
    rows.iter()
        .map(|r| match r {
            Row::Header | Row::Count { .. } => "<header>".to_string(),
            Row::Separator => "---".to_string(),
            Row::Pair { label, .. } => (*label).to_string(),
            Row::OpensWith { .. } => "Opens with:".to_string(),
            Row::Attributes { .. } => "Attributes:".to_string(),
        })
        .collect()
}

/// A file: icon and name, Type of file, Opens with, Location / Size / Size
/// on disk, the three times, Attributes, with separators between groups.
#[test]
fn a_files_rows_run_from_header_to_attributes() {
    let rows = rows(&info(false), true);
    assert_eq!(
        labels(&rows),
        [
            "<header>",
            "---",
            "Type of file:",
            "Opens with:",
            "---",
            "Location:",
            "Size:",
            "Size on disk:",
            "---",
            "Created:",
            "Modified:",
            "Accessed:",
            "---",
            "Attributes:",
        ]
    );
    assert!(matches!(
        rows.last(),
        Some(Row::Attributes {
            subject: filesheet::Subject::File,
            ..
        })
    ));
}

/// A folder: no Opens with, no separator between Type and Location, Contains
/// after the sizes, Created alone, the two attribute boxes stacked.
#[test]
fn a_folders_rows_drop_the_program_and_the_two_other_times() {
    let rows = rows(&info(true), true);
    assert_eq!(
        labels(&rows),
        [
            "<header>",
            "---",
            "Type:",
            "Location:",
            "Size:",
            "Size on disk:",
            "Contains:",
            "---",
            "Created:",
            "---",
            "Attributes:",
        ]
    );
    assert!(matches!(
        rows.last(),
        Some(Row::Attributes {
            subject: filesheet::Subject::Folder,
            ..
        })
    ));
    let contains = rows.iter().find_map(|r| match r {
        Row::Pair { label, value, .. } if *label == "Contains:" => Some(value.clone()),
        _ => None,
    });
    assert_eq!(contains.as_deref(), Some("0 Files, 1 Folder"));
}

/// Size on disk is the blocks the file takes, formatted like Size: a 4,096 byte file reads "4.00 KB (4,096 bytes)".
#[test]
fn size_on_disk_reads_like_size() {
    let rows = rows(&info(false), true);
    let disk = rows.iter().find_map(|r| match r {
        Row::Pair { label, value, .. } if *label == "Size on disk:" => Some(value.clone()),
        _ => None,
    });
    assert_eq!(disk.as_deref(), Some("4.00 KB (4,096 bytes)"));
    let size = rows.iter().find_map(|r| match r {
        Row::Pair { label, value, .. } if *label == "Size:" => Some(value.clone()),
        _ => None,
    });
    assert_eq!(size.as_deref(), Some("140 bytes"));
}

/// A file system with no birth time drops the Created row rather than showing the modification time twice.
#[test]
fn no_birth_time_drops_the_created_row() {
    let mut f = info(false);
    f.created = None;
    assert!(!labels(&rows(&f, true)).contains(&"Created:".to_string()));
}

fn props(info: FileInfo) -> Props {
    let rows = rows(&info, true);
    let name = info.name.clone();
    let (page, height) = general_page(&rows, &name, None, None);
    let sheet = PropSheet::new(&["General"], vec![page], (SHEET_W, height));
    Props {
        caption: name.clone(),
        opened_read_only: reprisekit::controls::filesheet::read_only(sheet.open()),
        opened_name: name,
        restore: None,
        set: None,
        robots: None,
        adv: None,
        summary: None,
        own_icon: None,
        icon_chosen: None,
        info,
        sheet,
    }
}

/// Which place gets a sheet with nothing selected, and its name field and
/// caption. Documents, Home and the Desktop wear their shell names while the
/// field keeps the real directory's, so OK can't rename `$HOME` to a label;
/// Computer and the Recycle Bin have no sheet.
#[test]
fn a_places_own_sheet_is_titled_by_its_shell_name_or_refused() {
    use crate::place::Place;
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ctx = crate::tests::test_ctx();
    let here = Place::Folder(dir.path().join("Reports"));
    std::fs::create_dir(dir.path().join("Reports")).expect("mkdir");
    let mut ex = crate::tests::test_explorer(here.clone(), &mut ctx);

    let (path, name, caption) = ex.place_sheet().expect("a folder has its own sheet");
    assert_eq!(path, dir.path().join("Reports"));
    assert_eq!((name.as_str(), caption.as_str()), ("Reports", "Reports"));

    for (place, caption) in [
        (Place::Documents, "Documents".to_string()),
        (Place::Home, reprisefs::places::home_label()),
        (Place::Desktop, "Desktop".to_string()),
    ] {
        let home = std::path::PathBuf::from(std::env::var_os("HOME").expect("HOME"));
        ex.place = place.clone();
        let (path, name, title) = ex.place_sheet().expect("a shell folder has a sheet");
        assert_eq!(path, place.path().expect("its directory"));
        assert_eq!(title, caption, "the caption is its shell name");
        let real = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        assert_eq!(name, real, "the field keeps the real directory name");
        assert!(path.starts_with(&home));
    }
    // Home: its field holds the real directory name, so OK cannot rename `$HOME` to the label.
    ex.place = Place::Home;
    let (_, name, caption) = ex.place_sheet().expect("Home has a sheet");
    assert_ne!(name, caption);

    for place in [Place::Computer, Place::RecycleBin] {
        ex.place = place.clone();
        assert!(ex.place_sheet().is_none(), "{place:?} has no folder sheet");
        assert_eq!(
            ex.properties(&mut ctx),
            repriseapp::Reply::none(),
            "{place:?}"
        );
        assert!(ex.props.is_none(), "{place:?} opened a sheet anyway");
    }
}

fn row_value(ex: &crate::Explorer, id: &str) -> String {
    match ex
        .props
        .as_ref()
        .and_then(|p| p.sheet.open().item(id))
        .map(|i| &i.control)
    {
        Some(Control::Label { text, .. }) => text.clone(),
        _ => String::new(),
    }
}

/// A folder's sheet opens before its walk: Size, Size on disk and Contains
/// read "Calculating..." until the totals land, and the page keeps its height.
#[test]
fn a_folders_totals_arrive_after_the_sheet_opens() {
    use crate::place::Place;
    use reprisefs::props::{format_contains, format_size};
    let dir = tempfile::tempdir().expect("a temp dir");
    let sub = dir.path().join("sub");
    std::fs::create_dir_all(sub.join("inner")).expect("mkdir");
    std::fs::write(sub.join("inner/a.txt"), "hello").expect("write");
    let mut ctx = crate::tests::test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);

    ex.open_properties(&sub, "sub", "sub", &mut ctx);
    let size = ex.props.as_ref().expect("the sheet").size();
    for id in ["v_size", "v_disk", "v_has"] {
        assert_eq!(row_value(&ex, id), CALCULATING, "{id} before the walk");
    }

    ex.finish_puts(&mut ctx);
    assert_eq!(row_value(&ex, "v_has"), format_contains(1, 1));
    assert_eq!(row_value(&ex, "v_size"), format_size(5));
    assert_eq!(ex.props.as_ref().map(|p| p.size()), Some(size));
    assert_eq!(
        ex.props.as_ref().and_then(|p| p.info.contains),
        Some((1, 1))
    );
}

/// OK on Confirm Attribute Changes with the tree radio picked writes the tree; the sheet closes when it is done.
#[test]
fn the_tree_attribute_write_reaches_the_subfolders_and_closes_the_sheet() {
    use crate::place::Place;
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().expect("a temp dir");
    let full = dir.path().join("Reports");
    std::fs::create_dir_all(full.join("2026")).expect("mkdir");
    let mut ctx = crate::tests::test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.open_properties(&full, "Reports", "Reports", &mut ctx);
    ex.finish_puts(&mut ctx);

    let form = attr_form(
        &RO_TRUE,
        None,
        FOLDER,
        &ex.dialog_text,
        ex.dialog_px,
        TextUnits::ONE,
    );
    assert!(attr_tree_picked(&form));
    let a = AttrConfirm {
        set: None,
        new_name: None,
        attrs: RO_TRUE,
        hidden: None,
        close: true,
        form,
    };
    ex.attrs_confirmed(a, &mut ctx);
    ex.finish_puts(&mut ctx);
    assert!(
        ex.props.is_none(),
        "OK closes the sheet once the write is done"
    );
    let mode = |p: &Path| std::fs::metadata(p).expect("stat").permissions().mode();
    assert_eq!(
        mode(&full.join("2026")) & 0o222,
        0,
        "the subfolder is read-only"
    );
    for p in [full.join("2026"), full.clone()] {
        std::fs::set_permissions(&p, PermissionsExt::from_mode(0o755)).expect("chmod back");
    }
}

/// A folder's Hidden asks how far it reaches before anything is written.
/// Cancel writes nothing and clears the box; OK with the tree picked puts the
/// folder's dot on and lists its contents in `.hidden` files, leaving a `.git`
/// alone. Unhiding the tree takes the lists off and renames no dotfile.
#[test]
fn a_folders_hidden_asks_first_and_lists_its_tree() {
    use crate::place::Place;
    let dir = tempfile::tempdir().expect("a temp dir");
    let full = dir.path().join("Reports");
    std::fs::create_dir_all(full.join("sub")).expect("mkdir");
    std::fs::create_dir_all(full.join(".git")).expect("mkdir");
    std::fs::write(full.join("a.txt"), "a").expect("write");
    std::fs::write(full.join(".git").join("HEAD"), "ref").expect("write");
    let mut ctx = crate::tests::test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    let tick = |ex: &mut crate::Explorer, on: bool, ctx: &mut repriseapp::Ctx| {
        let p = ex.props.as_mut().expect("the sheet");
        p.sheet.open_mut().set_check(HIDDEN, on);
        ex.props_outcome(reprisekit::controls::propsheet::Event::Apply, ctx);
    };
    let label = |ex: &crate::Explorer| match &ex.dialog {
        Some(crate::dialogs::Dialog::AttrConfirm(a)) => match a.form.item("attr_verb") {
            Some(Item {
                control: Control::Label { text, .. },
                ..
            }) => text.clone(),
            _ => String::new(),
        },
        _ => panic!("Confirm Attribute Changes is up"),
    };
    ex.open_properties(&full, "Reports", "Reports", &mut ctx);
    ex.finish_puts(&mut ctx);
    tick(&mut ex, true, &mut ctx);
    assert_eq!(label(&ex), "hide");
    assert!(full.exists(), "nothing is written before the answer");
    ex.dialog = None;
    ex.attrs_cancelled();
    assert!(!filesheet::hidden(ex.props.as_ref().unwrap().sheet.open()));

    tick(&mut ex, true, &mut ctx);
    let Some(crate::dialogs::Dialog::AttrConfirm(a)) = ex.dialog.take() else {
        panic!("Confirm Attribute Changes is up");
    };
    assert!(attr_tree_picked(&a.form));
    ex.attrs_confirmed(a, &mut ctx);
    ex.finish_puts(&mut ctx);
    let hidden = dir.path().join(".Reports");
    let list = |d: &Path| std::fs::read_to_string(d.join(".hidden")).ok();
    assert_eq!(list(&hidden).as_deref(), Some("a.txt\nsub\n"));
    assert_eq!(list(&hidden.join(".git")), None);

    ex.open_properties(&hidden, ".Reports", ".Reports", &mut ctx);
    ex.finish_puts(&mut ctx);
    tick(&mut ex, false, &mut ctx);
    assert_eq!(label(&ex), "unhide");
    let a = ex.dialog.take().map(|d| match d {
        crate::dialogs::Dialog::AttrConfirm(a) => a,
        _ => unreachable!(),
    });
    ex.attrs_confirmed(a.unwrap(), &mut ctx);
    ex.finish_puts(&mut ctx);
    assert_eq!(list(&full), None);
    assert!(
        full.join(".git").join("HEAD").exists(),
        "the dotfile keeps its name"
    );
}

/// Read-only and Hidden together on a folder applied down its tree both
/// land: the `.hidden` lists go in before the folders lose their write bits.
/// Unhiding that read-only tree can't take its lists off: the error box says
/// so over the sheet, read again under the name the folder took.
#[test]
fn read_only_and_hidden_land_together_down_a_tree() {
    use crate::place::Place;
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().expect("a temp dir");
    let full = dir.path().join("Reports");
    std::fs::create_dir_all(full.join("sub")).expect("mkdir");
    std::fs::write(full.join("sub").join("a.txt"), "a").expect("write");
    let mut ctx = crate::tests::test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    let confirm = |ex: &mut crate::Explorer, ctx: &mut repriseapp::Ctx| {
        ex.props_outcome(reprisekit::controls::propsheet::Event::Apply, ctx);
        let Some(crate::dialogs::Dialog::AttrConfirm(a)) = ex.dialog.take() else {
            panic!("Confirm Attribute Changes is up");
        };
        assert!(attr_tree_picked(&a.form));
        ex.attrs_confirmed(a, ctx);
        ex.finish_puts(ctx);
    };
    ex.open_properties(&full, "Reports", "Reports", &mut ctx);
    ex.finish_puts(&mut ctx);
    let p = ex.props.as_mut().expect("the sheet");
    p.sheet.open_mut().set_check(READ_ONLY, true);
    p.sheet.open_mut().set_check(HIDDEN, true);
    confirm(&mut ex, &mut ctx);

    let hidden = dir.path().join(".Reports");
    let list = |d: &Path| std::fs::read_to_string(d.join(".hidden")).ok();
    let mode = |p: &Path| std::fs::metadata(p).expect("stat").permissions().mode();
    let listed = (list(&hidden), list(&hidden.join("sub")));
    let locked = mode(&hidden.join("sub")) & 0o222;
    let first_box = ex.dialog.take();

    ex.open_properties(&hidden, ".Reports", ".Reports", &mut ctx);
    ex.finish_puts(&mut ctx);
    ex.props
        .as_mut()
        .expect("the sheet")
        .sheet
        .open_mut()
        .set_check(HIDDEN, false);
    confirm(&mut ex, &mut ctx);
    let second_box = ex.dialog.take();
    for p in [full.join("sub"), full.clone()] {
        std::fs::set_permissions(&p, PermissionsExt::from_mode(0o755)).expect("chmod back");
    }

    assert!(first_box.is_none(), "both writes land without an error box");
    assert_eq!(listed.0.as_deref(), Some("sub\n"));
    assert_eq!(listed.1.as_deref(), Some("a.txt\n"));
    assert_eq!(locked, 0, "the subfolder is read-only");
    match second_box {
        Some(crate::dialogs::Dialog::Message { title, text }) => {
            assert_eq!(title, "Properties");
            assert!(
                text.starts_with("Cannot change Reports: "),
                "the box names the folder: {text}"
            );
        }
        _ => panic!("a failed list write opens the error box"),
    }
    let sheet = ex
        .props
        .as_ref()
        .expect("the sheet stays open under the box");
    assert_eq!(sheet.info.path, full, "the sheet reads the folder again");
}

/// A name the folder already holds is refused with the taken wording; neither file moves.
#[test]
fn the_sheets_rename_refuses_a_taken_name() {
    use crate::place::Place;
    let dir = tempfile::tempdir().expect("a temp dir");
    let notes = dir.path().join("notes.txt");
    std::fs::write(&notes, "notes").expect("write");
    std::fs::write(dir.path().join("report.txt"), "report").expect("write");
    let mut ctx = crate::tests::test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.open_properties(&notes, "notes", "notes", &mut ctx);
    if let Some(Item {
        control: Control::Edit { edit, .. },
        ..
    }) = ex
        .props
        .as_mut()
        .expect("the sheet")
        .sheet
        .open_mut()
        .item_mut(NAME)
    {
        *edit = TextEdit::new("report", false);
    }
    ex.props_event(
        repriseapp::Event::Key(repriseapp::KeyInput {
            sym: reprisekit::textedit::keysym::RETURN,
            utf8: None,
            mods: repriseapp::Mods::default(),
            edit: None,
        }),
        &mut ctx,
    );

    match &ex.dialog {
        Some(crate::dialogs::Dialog::Message { text, .. }) => {
            assert_eq!(*text, reprisefs::names::taken_text("notes.txt"));
        }
        _ => panic!("a refusal box is up"),
    }
    assert!(ex.props.is_some(), "the sheet stays up");
    assert_eq!(
        std::fs::read_to_string(&notes).ok().as_deref(),
        Some("notes")
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("report.txt"))
            .ok()
            .as_deref(),
        Some("report")
    );
}

/// A folder renamed through the sheet's OK relabels its row in the Folders
/// tree, as the in-place rename does (both use `Explorer::after_rename`).
#[test]
fn the_sheets_rename_relabels_a_folders_row_in_the_tree() {
    use crate::place::Place;
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::create_dir(dir.path().join("sub")).expect("mkdir");
    let mut ctx = crate::tests::test_ctx();
    let here = Place::Folder(dir.path().to_path_buf());
    let mut ex = crate::tests::test_explorer(here.clone(), &mut ctx);
    // `tempdir` names start with a dot, so the tree hides the chain until hidden entries are shown.
    ex.settings.show_hidden = true;
    ex.build_tree(&mut ctx);
    ex.show(here.clone(), &mut ctx);
    let node = ex.tree_path_of(&here).expect("this folder's node");
    let labels = |ex: &crate::Explorer, path: &[usize]| -> Vec<String> {
        ex.tree
            .node(path)
            .map(|n| n.children.iter().map(|c| c.label.clone()).collect())
            .unwrap_or_default()
    };
    assert_eq!(labels(&ex, &node), ["sub"]);

    let i = ex
        .items
        .iter()
        .position(|it| it.name == "sub")
        .expect("sub");
    ex.sv.selection.set_only(Some(i));
    ex.properties(&mut ctx);
    assert_eq!(
        ex.props.as_ref().map(|p| p.title()),
        Some("sub Properties".to_string())
    );
    // Type the new name and press Enter, the sheet's OK.
    if let Some(Item {
        control: Control::Edit { edit, .. },
        ..
    }) = ex
        .props
        .as_mut()
        .expect("the sheet")
        .sheet
        .open_mut()
        .item_mut(NAME)
    {
        *edit = TextEdit::new("renamed", false);
    }
    ex.props_event(
        repriseapp::Event::Key(repriseapp::KeyInput {
            sym: reprisekit::textedit::keysym::RETURN,
            utf8: None,
            mods: repriseapp::Mods::default(),
            edit: None,
        }),
        &mut ctx,
    );

    assert!(
        dir.path().join("renamed").is_dir(),
        "the rename was written"
    );
    assert!(ex.props.is_none(), "OK closes the sheet");
    let node = ex.tree_path_of(&here).expect("this folder's node");
    assert_eq!(labels(&ex, &node), ["renamed"], "the row follows the name");
    let mut child = node.clone();
    child.push(0);
    assert_eq!(
        ex.place_of(&child),
        Some(Place::Folder(dir.path().join("renamed")))
    );
}

/// Apply lights when the name or Read-only box differs from the file, and
/// goes out when undone. An emptied name is not a change.
#[test]
fn apply_follows_the_name_and_the_read_only_box() {
    let mut p = props(info(false));
    assert!(!p.dirty());
    assert_eq!(p.name(), "New Text Document");
    let set_name = |p: &mut Props, s: &str| {
        if let Some(Item {
            control: Control::Edit { edit, .. },
            ..
        }) = p.sheet.open_mut().item_mut(NAME)
        {
            *edit = TextEdit::new(s, false);
        }
    };
    set_name(&mut p, "Notes");
    assert!(p.dirty());
    set_name(&mut p, "New Text Document");
    assert!(!p.dirty());
    set_name(&mut p, "  ");
    assert!(!p.dirty(), "an empty name has nothing to write");
    set_name(&mut p, "New Text Document");

    if let Some(Item {
        control: Control::Check { checked, .. },
        ..
    }) = p.sheet.open_mut().item_mut(READ_ONLY)
    {
        *checked = true;
    }
    assert!(p.dirty());
    assert_eq!(p.read_only(), Some(true));
    assert_eq!(p.title(), "New Text Document Properties");
}

/// The Executable box writes the execute bit when it moves off the file's; a folder's is greyed.
#[test]
fn executable_writes_only_when_it_moves() {
    use reprisekit::controls::filesheet::EXECUTABLE;
    let mut p = props(info(false));
    assert_eq!(p.executable(), None);
    p.sheet.open_mut().set_check(EXECUTABLE, true);
    assert_eq!(p.executable(), Some(true));
    assert_eq!(p.attrs().executable, Some(true));
    assert!(p.dirty());
    p.sheet.open_mut().set_check(EXECUTABLE, false);
    assert!(!p.dirty());
    let folder = props(info(true));
    assert!(matches!(
        folder.general().item(EXECUTABLE).map(|i| &i.control),
        Some(Control::Check {
            enabled: false,
            checked: false,
            ..
        })
    ));
}

/// Ticking Hidden asks for the dotted name; a folder's Read-only writes nothing until it leaves the indeterminate state.
#[test]
fn hidden_renames_and_an_indeterminate_box_writes_nothing() {
    let mut p = props(info(false));
    p.sheet.open_mut().set_check(HIDDEN, true);
    assert_eq!(p.new_name().as_deref(), Some(".New Text Document.zip"));
    assert_eq!(p.hidden(), Some(true));
    assert!(p.dirty());

    // A file its folder's .hidden lists opens ticked and keeps its name; clearing the box writes Hidden alone.
    let mut listed = info(false);
    (listed.hidden, listed.listed) = (true, true);
    let mut p = props(listed);
    assert!(filesheet::hidden(p.sheet.open()));
    assert!(!p.dirty());
    p.sheet.open_mut().set_check(HIDDEN, false);
    assert_eq!((p.new_name(), p.hidden()), (None, Some(false)));
    assert!(p.dirty());

    let mut p = props(info(true));
    assert_eq!(p.read_only(), None);
    assert!(!p.dirty());
    p.sheet.open_mut().set_check(READ_ONLY, false);
    assert_eq!(
        p.read_only(),
        Some(false),
        "off is a change from indeterminate"
    );
}

/// The confirm box a folder's attributes raise: verb, question and the radio pair with "subfolders and files" picked.
#[test]
fn the_attribute_question_names_the_change_and_the_scope() {
    let f = attr_form(&RO_TRUE, None, FOLDER, &text(), px(), TextUnits::ONE);
    let label_of = |id: &str| match f.item(id).map(|i| &i.control) {
        Some(Control::Label { text, .. }) => text.clone(),
        Some(Control::Radio { label, .. }) => label.clone(),
        _ => String::new(),
    };
    assert_eq!(
        label_of("attr_intro"),
        "You have chosen to make the following attribute changes:"
    );
    assert_eq!(label_of("attr_verb"), "set read-only");
    assert_eq!(label_of(ATTR_TREE), ATTR_LABELS[1]);
    assert_eq!(label_of(ATTR_ONLY), ATTR_LABELS[0]);
    assert!(attr_tree_picked(&f), "opens on the tree radio");
    assert_eq!(
        attr_form(&RO_FALSE, None, FOLDER, &text(), px(), TextUnits::ONE)
            .item("attr_verb")
            .map(|i| match &i.control {
                Control::Label { text, .. } => text.clone(),
                _ => String::new(),
            })
            .as_deref(),
        Some("unset read-only")
    );
    assert_eq!(f.default_button().map(|i| f.items[i].id), Some(ATTR_OK));
}

/// Writes `target/props-file.png` and `target/props-folder.png` for a visual check.
#[test]
#[ignore]
fn render_the_sheets() {
    let theme = Theme {
        access_keys: true,
        ..Theme::default()
    };
    let t = text();
    for (dir, out) in [(false, "props-file.png"), (true, "props-folder.png")] {
        let mut info = info(dir);
        info.name = if dir { "New Folder".into() } else { info.name };
        let mut p = props(info);
        p.sheet.set_apply_enabled(false);
        let (w, h) = p.sheet.size;
        let mut c = Canvas::new(w as u32, h as u32);
        c.fill_rect(c.bounds(), theme.colors.button_face);
        reprisekit::controls::propsheet::draw(&mut c, &t, &theme.paint(), &p.sheet, true, px());
        reprisekit::raster::write_preview(&c, out);
    }
}

/// OK writes the new name (its verdict rule, with the extension the icon hides put back) and the Read-only bit.
#[test]
fn the_sheets_writes_land_on_disk() {
    use std::os::unix::fs::PermissionsExt;
    let base = std::env::temp_dir().join(format!("explorer-props-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let file = base.join("notes.txt");
    std::fs::write(&file, "hello").unwrap();
    std::fs::set_permissions(&file, PermissionsExt::from_mode(0o644)).unwrap();

    let info = reprisefs::props::inspect(&file, "notes", &[]).unwrap();
    assert_eq!(info.hidden_ext, ".txt");
    let old = format!("{}{}", info.name, info.hidden_ext);
    let wanted = format!("{}{}", "report", info.hidden_ext);
    assert_eq!(
        reprisefs::names::verdict(&old, &wanted, &["notes.txt".to_string()]),
        reprisefs::names::Verdict::Rename("report.txt".to_string())
    );
    std::fs::rename(&file, base.join(&wanted)).unwrap();
    let file = base.join(&wanted);
    assert!(file.is_file());

    reprisefs::props::set_read_only(&file, true).unwrap();
    assert!(
        reprisefs::props::inspect(&file, "report", &[])
            .unwrap()
            .read_only
    );
    assert_eq!(
        std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o444
    );
    reprisefs::props::set_read_only(&file, false).unwrap();
    assert!(
        !reprisefs::props::inspect(&file, "report", &[])
            .unwrap()
            .read_only
    );

    // A folder with something in it is asked about; an empty one is written without a question.
    let full = base.join("Reports");
    std::fs::create_dir_all(full.join("2026")).unwrap();
    let empty = base.join("Empty");
    std::fs::create_dir_all(&empty).unwrap();
    assert!(folder_has_contents(&full));
    assert!(!folder_has_contents(&empty));
    reprisefs::props::set_read_only_tree(&full, true).unwrap();
    assert!(
        reprisefs::props::inspect(&full.join("2026"), "2026", &[])
            .unwrap()
            .read_only,
        "the tree write reaches the subfolder"
    );
    reprisefs::props::set_read_only_tree(&full, false).unwrap();
    std::fs::remove_dir_all(&base).unwrap();
}

/// Change... opens the chooser with "always" checked and greyed, so it only writes the file type's default.
#[test]
fn change_opens_the_chooser_with_always_checked_and_greyed() {
    use crate::dialogs::Dialog;
    use crate::place::Place;
    use reprisekit::controls::openwith;
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("notes.txt");
    std::fs::write(&path, "notes").expect("write");
    let mut ctx = crate::tests::test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.choose_program_for(&path, true, &mut ctx);
    match ex.dialog.as_mut() {
        Some(Dialog::ChooseProgram(c)) => {
            assert_eq!(
                c.ow.always,
                openwith::Check {
                    on: true,
                    enabled: false
                }
            );
            assert!(c.set_default);
            assert_eq!(
                c.ow.key(reprisekit::controls::form::Key::Access('b')),
                openwith::Event::Browse,
                "Browse... stays live"
            );
        }
        other => panic!("expected the chooser, got {:?}", other.is_some()),
    }
}

/// The Confirm Attribute Changes box's hit test reads the look it was drawn
/// in: in Blue, a click 16 px right of a radio's left edge picks it.
#[test]
fn a_blue_attribute_box_picks_a_radio_by_its_label_16_px_in() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ctx = crate::tests::test_ctx();
    ctx.theme = reprisekit::interfaces::theme_for(reprisekit::interface::Interface::Blue);
    let mut ex =
        crate::tests::test_explorer(crate::Place::Folder(dir.path().to_path_buf()), &mut ctx);
    let form = attr_form(
        &RO_TRUE,
        None,
        FOLDER,
        &ex.dialog_text,
        ex.dialog_px,
        TextUnits::ONE,
    );
    let only = form.item(ATTR_ONLY).expect("the this-folder radio").rect;
    ex.dialog = Some(crate::dialogs::Dialog::AttrConfirm(AttrConfirm {
        set: None,
        new_name: None,
        attrs: RO_TRUE,
        hidden: None,
        close: false,
        form,
    }));
    let mut c = Canvas::new(ATTR_SIZE.0 as u32, ATTR_SIZE.1 as u32);
    ex.draw_dialog(&mut c, &ctx);
    ex.dialog_event(
        repriseapp::Event::Pointer(repriseapp::PointerInput::Press {
            x: only.x + 16,
            y: only.y + 5,
            button: crate::BTN_LEFT,
            time: 0,
            mods: Default::default(),
        }),
        &mut ctx,
    );
    ex.dialog_event(
        repriseapp::Event::Pointer(repriseapp::PointerInput::Release {
            x: only.x + 16,
            y: only.y + 5,
            button: crate::BTN_LEFT,
            mods: Default::default(),
        }),
        &mut ctx,
    );
    let Some(crate::dialogs::Dialog::AttrConfirm(a)) = &ex.dialog else {
        panic!("the box stays up");
    };
    assert!(
        !attr_tree_picked(&a.form),
        "the click picks the this-folder radio"
    );
}

/// The attribute box gets the pointer's motion and leave: a radio lights
/// under the pointer, and nothing stays lit after press, release and leave. A
/// push button pressed and dragged off does not act on the release.
#[test]
fn the_attribute_box_is_left_unlit_after_press_release_and_leave() {
    use reprisekit::controls::form::Light;
    use reprisekit::pointer::Interaction;
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ctx = crate::tests::test_ctx();
    ctx.theme = reprisekit::interfaces::theme_for(reprisekit::interface::Interface::Blue);
    let mut ex =
        crate::tests::test_explorer(crate::Place::Folder(dir.path().to_path_buf()), &mut ctx);
    let form = attr_form(
        &RO_TRUE,
        None,
        FOLDER,
        &ex.dialog_text,
        ex.dialog_px,
        TextUnits::ONE,
    );
    let only = form.index_of(ATTR_ONLY).expect("the this-folder radio");
    let cancel = form.index_of(ATTR_CANCEL).expect("Cancel");
    let (r, cr) = (form.items[only].rect, form.items[cancel].rect);
    ex.dialog = Some(crate::dialogs::Dialog::AttrConfirm(AttrConfirm {
        set: None,
        new_name: None,
        attrs: RO_TRUE,
        hidden: None,
        close: false,
        form,
    }));
    let mut c = Canvas::new(ATTR_SIZE.0 as u32, ATTR_SIZE.1 as u32);
    ex.draw_dialog(&mut c, &ctx);
    let form = |ex: &crate::Explorer| match &ex.dialog {
        Some(crate::dialogs::Dialog::AttrConfirm(a)) => a.form.clone(),
        _ => panic!("the box stays up"),
    };
    use repriseapp::{Event, PointerInput};
    let (x, y) = (r.x + 16, r.y + 5);
    let press = |x, y| {
        Event::Pointer(PointerInput::Press {
            x,
            y,
            button: crate::BTN_LEFT,
            time: 0,
            mods: Default::default(),
        })
    };
    let release = |x, y| {
        Event::Pointer(PointerInput::Release {
            x,
            y,
            button: crate::BTN_LEFT,
            mods: Default::default(),
        })
    };
    assert!(
        ex.dialog_event(Event::Pointer(PointerInput::Motion { x, y }), &mut ctx)
            .redraw
    );
    assert_eq!(form(&ex).lit(only, Light::Body), Interaction::Hot);
    ex.dialog_event(press(x, y), &mut ctx);
    ex.dialog_event(release(x, y), &mut ctx);
    assert!(
        ex.dialog_event(Event::Pointer(PointerInput::Leave), &mut ctx)
            .redraw
    );
    assert_eq!(
        form(&ex).lit(only, Light::Body),
        Interaction::Idle,
        "nothing stays lit"
    );
    ex.dialog_event(press(cr.x + 5, cr.y + 5), &mut ctx);
    assert_eq!(form(&ex).lit(cancel, Light::Body), Interaction::Pressed);
    ex.dialog_event(
        Event::Pointer(PointerInput::Motion { x: 2, y: 2 }),
        &mut ctx,
    );
    assert_eq!(form(&ex).lit(cancel, Light::Body), Interaction::Idle);
    ex.dialog_event(release(2, 2), &mut ctx);
    assert!(!form(&ex).holding());
}

/// Properties over several entries opens the combined sheet, named after the
/// focused one: its count arrives with the folder walk, and Read-only over a
/// non-empty folder asks how far it reaches first.
#[test]
fn properties_over_a_multi_selection_opens_the_combined_sheet() {
    use crate::place::Place;
    use reprisefs::props::format_contains;
    use reprisekit::classic::widgets::CheckState;
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::write(dir.path().join("a.txt"), "aa").expect("write");
    std::fs::write(dir.path().join("b.txt"), "bbb").expect("write");
    std::fs::create_dir(dir.path().join("sub")).expect("mkdir");
    std::fs::write(dir.path().join("sub/c.txt"), "c").expect("write");
    let mut ctx = crate::tests::test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    let at = |ex: &crate::Explorer, name: &str| {
        ex.items
            .iter()
            .position(|i| i.path.file_name().is_some_and(|n| n == name))
            .expect(name)
    };
    let (a, b, sub) = (at(&ex, "a.txt"), at(&ex, "b.txt"), at(&ex, "sub"));
    ex.sv.selection.set = [a, b, sub].into_iter().collect();
    ex.sv.selection.focus = Some(b);

    ex.properties(&mut ctx);
    let p = ex.props.as_ref().expect("the combined sheet");
    let first = ex.items[b].name.clone();
    assert_eq!(p.title(), format!("{first}, ... Properties"));
    assert_eq!(
        p.info.path,
        dir.path().join("b.txt"),
        "the focused one leads"
    );
    assert_eq!(row_value(&ex, "v_type"), "Multiple Types");
    assert_eq!(row_value(&ex, filesheet::COUNT), CALCULATING);
    ex.finish_puts(&mut ctx);
    assert_eq!(row_value(&ex, filesheet::COUNT), format_contains(3, 1));

    let p = ex.props.as_mut().expect("the sheet");
    p.sheet
        .open_mut()
        .set_check_state(READ_ONLY, CheckState::On);
    ex.props_outcome(reprisekit::controls::propsheet::Event::Apply, &mut ctx);
    let Some(crate::dialogs::Dialog::AttrConfirm(a)) = ex.dialog.take() else {
        panic!("a folder with something in it asks first");
    };
    assert_eq!(a.set.as_ref().map(Vec::len), Some(3));
    ex.attrs_confirmed(a, &mut ctx);
    ex.finish_puts(&mut ctx);
    let mode = |p: &Path| std::fs::metadata(p).expect("stat").permissions().mode();
    assert_eq!(
        mode(&dir.path().join("sub/c.txt")) & 0o222,
        0,
        "down the tree"
    );
    let p = ex.props.as_ref().expect("Apply keeps the sheet");
    assert_eq!(p.set.as_ref().and_then(|s| s.read_only), Some(true));
    assert!(!p.dirty(), "the sheet shows what was written");
    for f in ["a.txt", "b.txt", "sub/c.txt", "sub"] {
        let path = dir.path().join(f);
        std::fs::set_permissions(&path, PermissionsExt::from_mode(0o755)).expect("chmod back");
    }
}

/// Hidden over a set of files hides each by its leading dot; OK closes the sheet once they are renamed.
#[test]
fn hidden_over_a_set_renames_each_item() {
    use crate::place::Place;
    use reprisekit::classic::widgets::CheckState;
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::write(dir.path().join("a.txt"), "a").expect("write");
    std::fs::write(dir.path().join("b.txt"), "b").expect("write");
    let mut ctx = crate::tests::test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    let n = ex.items.len();
    ex.sv.selection.select_all(n);
    ex.properties(&mut ctx);
    let p = ex.props.as_mut().expect("the combined sheet");
    p.sheet.open_mut().set_check_state(HIDDEN, CheckState::On);
    assert!(p.dirty());
    ex.props_outcome(reprisekit::controls::propsheet::Event::Ok, &mut ctx);
    ex.finish_puts(&mut ctx);
    assert!(ex.props.is_none(), "OK closes the sheet");
    for f in [".a.txt", ".b.txt"] {
        assert!(dir.path().join(f).exists(), "{f}");
    }
}

/// A folder's sheet has Sharing after General; a file and a shortcut together
/// get Compatibility and a Summary with nothing in common. General's writes
/// still go through with another page open.
#[test]
fn folder_and_shortcut_set_sheets_get_their_tabs() {
    use crate::place::Place;
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::create_dir(dir.path().join("sub")).expect("mkdir");
    std::fs::write(dir.path().join("a.txt"), "a").expect("write");
    let app = "[Desktop Entry]\nType=Application\nName=App\nExec=app\n";
    reprisefs::apps::write_launcher(&dir.path().join("app.desktop"), app).expect("write");
    let mut ctx = crate::tests::test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.open_properties(&dir.path().join("sub"), "sub", "sub", &mut ctx);
    let labels = |ex: &crate::Explorer| {
        ex.props
            .as_ref()
            .expect("a sheet")
            .sheet
            .tabs
            .labels
            .clone()
    };
    assert_eq!(labels(&ex), ["General", "Sharing", "Customize"]);

    let p = ex.props.as_mut().expect("the sheet");
    p.sheet.open_tab(1);
    p.general_mut().set_check(HIDDEN, true);
    assert!(p.dirty(), "Hidden on General moved with Sharing open");

    ex.props = None;
    let n = ex.items.len();
    ex.sv.selection.set = (0..n)
        .filter(|&i| ex.items[i].path.extension().is_some())
        .collect();
    ex.properties(&mut ctx);
    assert_eq!(labels(&ex), ["General", "Compatibility", "Summary"]);
}

/// Advanced Attributes opens on the flags the file carries; its OK hands the
/// archive box to the sheet, which lights Apply, and Apply writes the flag.
/// Without user attributes on the file system both boxes are greyed.
#[test]
fn advanced_attributes_hands_the_archive_box_to_apply() {
    use crate::place::Place;
    use reprisekit::controls::filesheet::{ADV_ARCHIVE, ADVANCED, robots_chosen};
    let dir = tempfile::tempdir().expect("a temp dir");
    let f = dir.path().join("a.txt");
    std::fs::write(&f, "a").expect("write");
    if !reprisefs::meta::supported(&f) {
        return;
    }
    let mut ctx = crate::tests::test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.open_properties(&f, "a.txt", "a.txt", &mut ctx);
    ex.props_outcome(
        reprisekit::controls::propsheet::Event::Page(
            0,
            reprisekit::controls::form::Event::Pressed(ADVANCED),
        ),
        &mut ctx,
    );
    let Some(crate::dialogs::Dialog::Advanced { form, live, .. }) = ex.dialog.as_mut() else {
        panic!("Advanced Attributes is up");
    };
    assert!(*live);
    assert_eq!(robots_chosen(form), [CheckState::On, CheckState::On]);
    form.focus_id(ADV_ARCHIVE);
    form.key(reprisekit::controls::form::Key::Space);
    let Some(crate::dialogs::Dialog::Advanced { form, .. }) = ex.dialog.take() else {
        unreachable!()
    };
    ex.advanced_chosen(&form);
    assert!(ex.props.as_ref().is_some_and(|p| p.dirty()), "Apply lights");
    ex.props_outcome(reprisekit::controls::propsheet::Event::Apply, &mut ctx);
    ex.finish_puts(&mut ctx);
    assert_eq!(
        reprisefs::meta::robots(&f, reprisefs::meta::BACKUP),
        Some(false)
    );
    assert!(!ex.props.as_ref().is_some_and(|p| p.dirty()), "written");
}

/// A file's Summary tab opens on what the file says about itself; a typed
/// field lights Apply, and Apply writes it where desktop search tools read it.
#[test]
fn a_files_summary_writes_its_fields() {
    use crate::place::Place;
    use reprisekit::controls::filesheet::SUMMARY_IDS;
    let dir = tempfile::tempdir().expect("a temp dir");
    let f = dir.path().join("a.txt");
    std::fs::write(&f, "a").expect("write");
    if !reprisefs::meta::supported(&f) {
        return;
    }
    reprisefs::meta::set_summary(&f, reprisefs::meta::AUTHOR, "Ann").expect("xattr");
    let mut ctx = crate::tests::test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.open_properties(&f, "a.txt", "a.txt", &mut ctx);
    let p = ex.props.as_mut().expect("the sheet");
    assert_eq!(p.sheet.tabs.labels, ["General", "Summary"]);
    p.sheet.open_tab(1);
    let page = p.sheet.open_mut();
    assert!(matches!(
        page.item(SUMMARY_IDS[2]).map(|i| &i.control),
        Some(Control::Edit { edit, .. }) if edit.text == "Ann"
    ));
    page.focus_id(SUMMARY_IDS[0]);
    page.key(reprisekit::controls::form::Key::Char('Q'));
    assert!(p.dirty());
    ex.props_outcome(reprisekit::controls::propsheet::Event::Apply, &mut ctx);
    ex.finish_puts(&mut ctx);
    assert_eq!(
        reprisefs::meta::get(&f, reprisefs::meta::TITLE).expect("read"),
        Some("Q".to_string())
    );
    assert!(!ex.props.as_ref().is_some_and(|p| p.dirty()), "written");
}

/// Ctrl+V asks for the clipboard and lands in the focused Comments memo;
/// Ctrl+C and Ctrl+X hand its selection back, Ctrl+X removing it.
#[test]
fn the_sheets_fields_take_the_clipboard() {
    use crate::place::Place;
    use reprisekit::controls::filesheet::SUMMARY_IDS;
    use reprisekit::textedit::{EditKey, keysym};
    let dir = tempfile::tempdir().expect("a temp dir");
    let f = dir.path().join("a.txt");
    std::fs::write(&f, "a").expect("write");
    let mut ctx = crate::tests::test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.open_properties(&f, "a.txt", "a.txt", &mut ctx);
    let p = ex.props.as_mut().expect("the sheet");
    p.sheet.open_tab(1);
    p.sheet.open_mut().focus_id(SUMMARY_IDS[5]);
    let ctrl = |sym: u32| {
        repriseapp::Event::Key(repriseapp::KeyInput {
            sym,
            utf8: None,
            mods: repriseapp::Mods {
                ctrl: true,
                ..Default::default()
            },
            edit: reprisekit::textedit::key_for(sym, true, false, None),
        })
    };
    let comments = |ex: &crate::Explorer| {
        ex.props
            .as_ref()
            .and_then(|p| p.sheet.open().focused_edit())
            .and_then(|e| e.copy_text())
    };
    assert!(ex.props_event(ctrl(u32::from(b'v')), &mut ctx).paste);
    ex.props_event(repriseapp::Event::Pasted("one\ntwo".into()), &mut ctx);
    assert!(ex.props.as_ref().is_some_and(|p| p.dirty()), "Apply lights");
    assert_eq!(comments(&ex).as_deref(), Some(""), "nothing selected");
    ex.props
        .as_mut()
        .expect("the sheet")
        .sheet
        .open_mut()
        .key(reprisekit::controls::form::Key::Edit(EditKey::SelectAll));
    let copy = ex.props_event(ctrl(u32::from(b'c')), &mut ctx).copy;
    assert_eq!(copy.as_deref(), Some("one\ntwo"));
    let cut = ex.props_event(ctrl(u32::from(b'x')), &mut ctx).copy;
    assert_eq!(cut.as_deref(), Some("one\ntwo"));
    assert_eq!(comments(&ex).as_deref(), Some(""));
    let left = ex.props_event(ctrl(keysym::HOME), &mut ctx).copy;
    assert_eq!(left, None, "no clipboard for another key");
}

/// Change Icon... opens on the folder's icon; its OK hands the pick to the
/// sheet, which lights Apply, and Apply writes the folder's .directory.
/// Restore Defaults and OK then take the icon off again.
#[test]
fn change_icon_gives_a_folder_its_own_icon() {
    use crate::place::Place;
    use reprisekit::controls::filesheet::{CHANGE_ICON, CHANGE_ICON_OK, CHANGE_ICON_RESTORE};
    use reprisekit::controls::form;
    let dir = tempfile::tempdir().expect("a temp dir");
    let sub = dir.path().join("sub");
    std::fs::create_dir(&sub).expect("mkdir");
    let mut ctx = crate::tests::test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.open_properties(&sub, "sub", "sub", &mut ctx);
    ex.finish_puts(&mut ctx);
    let page = |ev| reprisekit::controls::propsheet::Event::Page(2, ev);
    ex.props_outcome(page(form::Event::Pressed(CHANGE_ICON)), &mut ctx);
    let Some(crate::dialogs::Dialog::ChangeIcon { form, name }) = ex.dialog.as_mut() else {
        panic!("Change Icon is up");
    };
    assert_eq!(name, "sub");
    assert!(matches!(
        form.item("ci_list").map(|i| &i.control),
        Some(Control::IconList(l)) if l.picked() == Some("folder")
    ));
    ex.dialog = None;
    ex.icon_chosen(Some("folder-music".to_string()), &mut ctx);
    assert!(ex.props.as_ref().is_some_and(|p| p.dirty()), "Apply lights");
    ex.props_outcome(reprisekit::controls::propsheet::Event::Apply, &mut ctx);
    ex.finish_puts(&mut ctx);
    assert_eq!(
        reprisefs::meta::folder_icon(&sub).as_deref(),
        Some("folder-music")
    );

    ex.props_outcome(page(form::Event::Pressed(CHANGE_ICON)), &mut ctx);
    ex.dialog_outcome(form::Event::Pressed(CHANGE_ICON_RESTORE), &mut ctx);
    ex.dialog_outcome(form::Event::Pressed(CHANGE_ICON_OK), &mut ctx);
    ex.props_outcome(reprisekit::controls::propsheet::Event::Ok, &mut ctx);
    ex.finish_puts(&mut ctx);
    assert_eq!(
        reprisefs::meta::folder_icon(&sub),
        None,
        "the standard folder"
    );
}

#[test]
fn the_wheel_scrolls_the_change_icon_list() {
    use crate::Explorer;
    use crate::place::Place;
    use repriseapp::{Event, PointerInput};
    use reprisekit::controls::filesheet::CHANGE_ICON;
    use reprisekit::controls::form;
    use reprisekit::controls::iconlist::IconList;
    use reprisekit::pointer::WHEEL_ROWS;
    let dir = tempfile::tempdir().expect("a temp dir");
    let sub = dir.path().join("sub");
    std::fs::create_dir(&sub).expect("mkdir");
    let mut ctx = crate::tests::test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.open_properties(&sub, "sub", "sub", &mut ctx);
    ex.finish_puts(&mut ctx);
    let page = |ev| reprisekit::controls::propsheet::Event::Page(2, ev);
    ex.props_outcome(page(form::Event::Pressed(CHANGE_ICON)), &mut ctx);
    fn list(ex: &mut Explorer) -> (reprisekit::raster::Rect, &mut IconList) {
        match ex.dialog.as_mut() {
            Some(crate::dialogs::Dialog::ChangeIcon { form, .. }) => {
                let it = form.item_mut("ci_list").expect("the list");
                match &mut it.control {
                    Control::IconList(l) => (it.rect, l),
                    _ => unreachable!(),
                }
            }
            _ => panic!("Change Icon is up"),
        }
    }
    let (r, l) = list(&mut ex);
    let n = 80;
    l.names = (0..n).map(|k| format!("i{k}")).collect();
    l.icons = vec![None; n];
    l.left = 0;
    ex.dialog_event(
        Event::Pointer(PointerInput::Scroll {
            x: r.x + 20,
            y: r.y + 20,
            rows: WHEEL_ROWS,
        }),
        &mut ctx,
    );
    assert_eq!(list(&mut ex).1.left, 1, "one notch, one column");
}

/// A launcher's sheet opens on Shortcut. A typed Start in lights Apply, and
/// Apply writes Path= keeping the file's other lines; Find Target opens the
/// program's folder; Change Icon's pick is written as Icon=.
#[test]
fn a_launchers_shortcut_tab_writes_its_keys() {
    use crate::place::Place;
    use reprisekit::controls::filesheet::{
        CHANGE_ICON, CHANGE_ICON_RESTORE, FIND_TARGET, SHORTCUT_IDS,
    };
    use reprisekit::controls::form;
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("shell.desktop");
    reprisefs::apps::write_launcher(
        &path,
        "[Desktop Entry]\nType=Application\nName=Shell\nComment[de]=Eine Shell\nExec=/bin/sh\n",
    )
    .expect("write");
    let mut ctx = crate::tests::test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.open_properties(&path, "Shell", "Shell", &mut ctx);
    let p = ex.props.as_mut().expect("the sheet");
    assert_eq!(
        p.sheet.tabs.labels,
        ["General", "Shortcut", "Compatibility"]
    );
    assert_eq!(p.sheet.tabs.open, 1);
    let page = p.sheet.open_mut();
    page.focus_id(SHORTCUT_IDS[1]);
    for c in "/tmp".chars() {
        page.key(form::Key::Char(c));
    }
    assert!(p.dirty(), "Apply lights");
    ex.props_outcome(reprisekit::controls::propsheet::Event::Apply, &mut ctx);
    ex.finish_puts(&mut ctx);
    assert_eq!(
        std::fs::read_to_string(&path).expect("read"),
        "[Desktop Entry]\nType=Application\nName=Shell\nComment[de]=Eine Shell\nExec=/bin/sh\nPath=/tmp\n"
    );

    let on_page = |ev| reprisekit::controls::propsheet::Event::Page(1, ev);
    let reply = ex.props_outcome(on_page(form::Event::Pressed(FIND_TARGET)), &mut ctx);
    assert_eq!(
        reply.spawn,
        [vec![
            std::ffi::OsString::from("reprise-explorer"),
            "--select".into(),
            "/bin/sh".into()
        ]]
    );
    assert!(ex.props.is_some(), "the sheet stays up");

    ex.props_outcome(on_page(form::Event::Pressed(CHANGE_ICON)), &mut ctx);
    let Some(crate::dialogs::Dialog::ChangeIcon { form, .. }) = ex.dialog.as_ref() else {
        panic!("Change Icon is up");
    };
    assert!(form.item(CHANGE_ICON_RESTORE).is_none(), "a launcher's box");
    ex.dialog = None;
    ex.icon_chosen(Some("utilities-terminal".to_string()), &mut ctx);
    ex.props_outcome(reprisekit::controls::propsheet::Event::Ok, &mut ctx);
    ex.finish_puts(&mut ctx);
    assert!(ex.props.is_none(), "OK closed the sheet");
    let text = std::fs::read_to_string(&path).expect("read");
    assert!(
        text.ends_with("Path=/tmp\nIcon=utilities-terminal\n"),
        "{text}"
    );
}

/// A symlink's sheet: General (Type "Shortcut", Description its target's
/// name) and Shortcut, open on Shortcut with the target. Find Target selects
/// the target; Apply with a new Target points the link there.
#[test]
fn a_symlinks_shortcut_tab_points_it_elsewhere() {
    use crate::place::Place;
    use reprisekit::controls::filesheet::{FIND_TARGET, SHORTCUT_IDS};
    use reprisekit::controls::form;
    let dir = tempfile::tempdir().expect("a temp dir");
    let real = std::fs::canonicalize(dir.path()).expect("canonical");
    std::fs::create_dir(real.join("Documents")).expect("mkdir");
    std::fs::create_dir(real.join("Music")).expect("mkdir");
    let link = real.join("Docs");
    std::os::unix::fs::symlink("Documents", &link).expect("symlink");
    let mut ctx = crate::tests::test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(real.clone()), &mut ctx);
    ex.open_properties(&link, "Docs", "Docs", &mut ctx);
    let p = ex.props.as_mut().expect("the sheet");
    assert_eq!(p.sheet.tabs.labels, ["General", "Shortcut"]);
    assert_eq!(p.sheet.tabs.open, 1);
    let label = |page: &form::Form, id: &str| match page.item(id).map(|i| &i.control) {
        Some(form::Control::Label { text, .. }) => text.clone(),
        _ => panic!("{id}"),
    };
    assert_eq!(label(&p.sheet.pages[0], "v_type"), "Shortcut");
    assert_eq!(label(&p.sheet.pages[0], "v_desc"), "Documents");
    assert_eq!(label(p.sheet.open(), "sc_type"), "File Folder");

    let on_page = |ev| reprisekit::controls::propsheet::Event::Page(1, ev);
    let reply = ex.props_outcome(on_page(form::Event::Pressed(FIND_TARGET)), &mut ctx);
    assert_eq!(
        reply.spawn,
        [vec![
            std::ffi::OsString::from("reprise-explorer"),
            "--select".into(),
            real.join("Documents").into_os_string()
        ]]
    );

    let p = ex.props.as_mut().expect("the sheet stays up");
    let page = p.sheet.open_mut();
    page.focus_id(SHORTCUT_IDS[0]);
    page.key(form::Key::Edit(reprisekit::textedit::EditKey::SelectAll));
    for c in "Music".chars() {
        page.key(form::Key::Char(c));
    }
    assert!(p.dirty(), "Apply lights");
    ex.props_outcome(reprisekit::controls::propsheet::Event::Ok, &mut ctx);
    ex.finish_puts(&mut ctx);
    assert!(ex.props.is_none(), "OK closed the sheet");
    assert_eq!(
        std::fs::read_link(&link).expect("still a link"),
        std::path::PathBuf::from("Music")
    );
}

/// A web link's sheet: General (Type "Internet Shortcut") and Web Document,
/// open on it with the URL; OK with a new URL writes URL= back.
#[test]
fn a_web_links_web_document_tab_writes_its_url() {
    use crate::place::Place;
    use reprisekit::controls::filesheet::SHORTCUT_IDS;
    use reprisekit::controls::form;
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("ex.desktop");
    let text = reprisefs::apps::link_text("Example", "https://example.org/").expect("text");
    reprisefs::apps::write_launcher(&path, &text).expect("write");
    let mut ctx = crate::tests::test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.open_properties(&path, "Example", "Example", &mut ctx);
    let p = ex.props.as_mut().expect("the sheet");
    assert_eq!(p.sheet.tabs.labels, ["General", "Web Document"]);
    assert_eq!(p.sheet.tabs.open, 1);
    let page = p.sheet.open_mut();
    assert_eq!(
        page.focused_edit().and_then(|e| e.copy_text()).as_deref(),
        Some("https://example.org/")
    );
    page.focus_id(SHORTCUT_IDS[0]);
    page.key(form::Key::Edit(reprisekit::textedit::EditKey::End {
        select: false,
    }));
    for c in "news".chars() {
        page.key(form::Key::Char(c));
    }
    ex.props_outcome(reprisekit::controls::propsheet::Event::Ok, &mut ctx);
    ex.finish_puts(&mut ctx);
    assert!(ex.props.is_none(), "OK closed the sheet");
    assert_eq!(
        std::fs::read_to_string(&path).expect("read"),
        text.replace("URL=https://example.org/", "URL=https://example.org/news")
    );
}
