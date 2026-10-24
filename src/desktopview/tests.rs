use std::borrow::Cow;

use super::drives::{drive_tops, drive_trash_wants, read_drive_bin};
use super::layout::landing_cell;
use super::scan::{
    TRASH_MASK, home_trash_wants, icon_files, listed, migrate_launcher_keys, read_icons, remap,
    watch_wants,
};
use super::select::{band_crop, band_strip, source_of};
use super::*;
use crate::desktop::Target;
use crate::dirwatch::Want;
use crate::startmenu::MenuItem;

/// A home trash made after the watch started is seen through the data
/// folder, as is one moved away and back, and a file landing in it.
#[test]
fn the_home_trash_watch_follows_its_folder() {
    let data = std::env::temp_dir().join(format!("reprise-bin-watch-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&data);
    std::fs::create_dir_all(&data).unwrap();
    let root = data.join("Trash");
    let mut w = DirWatch::new().unwrap();
    w.sync(&home_trash_wants(&root));

    std::fs::write(data.join("recently-used.xbel"), "").unwrap();
    assert!(!w.drain(), "a file in the data folder: no rescan");
    std::fs::create_dir_all(root.join("files")).unwrap();
    assert!(w.drain(), "the trash made: rescan");
    w.sync(&home_trash_wants(&root));
    std::fs::write(root.join("files/a.txt"), "").unwrap();
    assert!(w.drain(), "a file trashed: rescan");

    let aside = data.join("Trash.bak");
    std::fs::rename(&root, &aside).unwrap();
    assert!(w.drain(), "the trash moved away: rescan");
    w.sync(&home_trash_wants(&root));
    std::fs::rename(&aside, &root).unwrap();
    assert!(w.drain(), "the trash moved back: rescan");
    w.sync(&home_trash_wants(&root));
    std::fs::remove_file(root.join("files/a.txt")).unwrap();
    assert!(w.drain(), "the bin emptied: rescan");

    let _ = std::fs::remove_dir_all(&data);
}

/// Draws the desktop menu, the place menus and the folder and file icon menus
/// under Classic into `target/`, for `scripts/pixels-unchanged.sh`.
#[test]
#[ignore]
fn render_desktop_menu() {
    use reprisekit::controls::menu::{MenuLayout, MenuStyle, draw_menu};
    use reprisekit::raster::{Canvas, write_preview};
    use reprisekit::text::TextRenderer;
    use reprisekit::theme::{FontConfig, Theme};
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let draw = |items: &[MenuItem], underline: bool| {
        let style =
            MenuStyle::context_for(&theme, items).fit(&text, items, theme.fonts.size_px, i32::MAX);
        let layout = MenuLayout::compute_styled(&style, items, usize::MAX);
        let mut c = Canvas::new(layout.width as u32, layout.height as u32);
        draw_menu(&mut c, &theme, &text, &layout, items, None, "", underline);
        c
    };
    write_preview(
        &draw(&desktop_menu(false, true, false).0, false),
        "desktopmenu-preview.png",
    );
    let places = [
        place_menu(desktop::Place::Documents, true).0,
        place_menu(desktop::Place::Computer, false).0,
        place_menu(desktop::Place::Home, true).0,
    ];
    let canvases: Vec<Canvas> = places.iter().map(|items| draw(items, true)).collect();
    let w: u32 = canvases.iter().map(|c| c.width + 8).sum();
    let h = canvases.iter().map(|c| c.height).max().unwrap_or(1);
    let mut all = Canvas::new(w, h);
    let mut x = 0;
    for c in &canvases {
        all.blit_canvas(x, 0, c);
        x += c.width as i32 + 8;
    }
    write_preview(&all, "placemenus-preview.png");
    for (name, items) in [
        (
            "folder-icon-menu",
            icon_menu(IconMenu {
                is_dir: true,
                ..Default::default()
            })
            .0,
        ),
        (
            "file-icon-menu",
            icon_menu(IconMenu {
                print: true,
                edit: true,
                ..Default::default()
            })
            .0,
        ),
    ] {
        write_preview(&draw(&items, false), &format!("{name}-preview.png"));
    }
}

#[test]
fn send_to_menu_greys_a_disabled_row() {
    let rows = reprisefs::sendto::rows(
        false,
        Some(PathBuf::from("/home/u/Documents")),
        &[PathBuf::from("/run/media/u/STICK")],
    );
    let (items, commands) = send_to_menu(&rows);
    assert_eq!(items.len(), 5);
    assert_eq!(commands[1], None, "no desktop folder, no Desktop row");
    assert_eq!(commands[3], Some(DesktopCommand::SendToDest(3)));
}

fn file(name: &str) -> DesktopEntry {
    let path = PathBuf::from("/d").join(name);
    DesktopEntry {
        name: name.into(),
        label: name.into(),
        icon: "text-x-generic".into(),
        target: Target::Open(path.clone()),
        path,
        shortcut: false,
        kind: desktop::Kind::File,
        is_dir: false,
        allow_launching: false,
    }
}

/// A desktop read, which runs off the loop, says which icons are folders (a link to one
/// included) and decodes the icon files they wear, so drops, menus and the first draw ask
/// neither the folders nor the files again.
#[test]
fn a_desktop_read_marks_folders_and_decodes_their_icon_files() {
    let dir = std::env::temp_dir().join(format!("reprise-desk-read-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("Art")).unwrap();
    std::fs::write(dir.join("notes.txt"), "").unwrap();
    std::os::unix::fs::symlink(dir.join("Art"), dir.join("Art link")).unwrap();
    let pic = dir.join(".cover.png");
    image::RgbaImage::from_pixel(64, 64, image::Rgba([0x20, 0xa0, 0x40, 0xff]))
        .save(&pic)
        .unwrap();
    reprisefs::meta::set_folder_icon(&dir.join("Art"), pic.to_str()).unwrap();
    let entries = read_icons(
        &dir,
        &apps::MimeDb::empty(),
        &BTreeMap::new(),
        &[],
        true,
        reprisefs::names::Extensions::Hide,
        false,
    );
    let dirs: Vec<(&str, bool)> = entries
        .iter()
        .map(|e| (e.name.as_str(), e.is_dir))
        .collect();
    assert_eq!(
        dirs,
        [("Art", true), ("Art link", true), ("notes.txt", false)]
    );
    let art = icon_files(&entries, 32);
    assert_eq!(art.len(), 1, "one file for the folder and its link");
    assert_eq!(art[0].0, pic.to_str().unwrap());
    let decoded = art[0].1.as_ref().expect("the picture");
    assert_eq!((decoded.width, decoded.get(8, 8)), (32, 0xff20_a040));
    let _ = std::fs::remove_dir_all(&dir);
}

/// `desktop_icons` off hides only the folder's files: the system icons
/// still list, and the folder is neither read nor watched.
#[test]
fn the_system_icons_list_with_the_desktop_files_off() {
    let system_icons: Vec<String> = ["documents", "recycle-bin"].map(String::from).into();
    let system = |name: &str| match name {
        "recycle-bin" => Some(desktop::recycle_bin(PathBuf::from("/t/files"), true)),
        _ => desktop::place(name),
    };
    let names = |entries: &[DesktopEntry]| -> Vec<String> {
        entries.iter().map(|e| e.label.clone()).collect()
    };
    let off = listed(&system_icons, false, system, || {
        panic!("the folder is not read with its files off")
    });
    assert_eq!(names(&off), ["Documents", "Recycle Bin"]);
    let on = listed(&system_icons, true, system, || vec![file("a.txt")]);
    assert_eq!(names(&on), ["Documents", "Recycle Bin", "a.txt"]);

    let dir = Path::new("/home/u/Desktop");
    let watched = |files: bool| -> Vec<PathBuf> {
        watch_wants(dir, files, &[])
            .into_iter()
            .map(|w| w.dir)
            .collect()
    };
    assert_eq!(watched(false), Vec::<PathBuf>::new(), "nothing to watch");
    assert_eq!(
        watched(true),
        [dir.to_path_buf(), PathBuf::from("/home/u")],
        "the folder and its parent"
    );
}

#[test]
fn a_drive_bin_read_watches_the_bins_there_and_says_whether_one_holds_something() {
    let t = std::env::temp_dir().join(format!("reprise-drive-bins-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&t);
    let (stick, card) = (t.join("STICK"), t.join("CARD"));
    std::fs::create_dir_all(&card).unwrap();
    let bins = reprisefs::trash::Bins::new(t.join("home"), vec![stick.clone(), card.clone()]);
    let bin = stick.join(format!(".Trash-{}", bins.uid));
    std::fs::create_dir_all(bin.join("files")).unwrap();
    std::fs::write(bin.join("files/a.txt"), "").unwrap();
    assert_eq!(drive_tops(&bins), [stick.clone(), card.clone()]);
    let (wants, full) = read_drive_bin(&bins, &stick);
    let dirs: Vec<PathBuf> = wants.into_iter().map(|w| w.dir).collect();
    assert_eq!(dirs, [stick.clone(), bin.join("files"), bin.clone()]);
    assert!(full, "a.txt is in STICK's bin");
    let (wants, full) = read_drive_bin(&bins, &card);
    let dirs: Vec<PathBuf> = wants.into_iter().map(|w| w.dir).collect();
    assert_eq!((dirs, full), (vec![card], false), "CARD has no bin");
    std::fs::remove_file(bin.join("files/a.txt")).unwrap();
    assert!(!read_drive_bin(&bins, &stick).1, "STICK's bin is empty");
    let _ = std::fs::remove_dir_all(&t);
}

#[test]
fn one_drive_list_read_runs_at_a_time_and_one_asked_for_meanwhile_reads_again() {
    let now = std::time::Instant::now();
    let mut bins = DriveBins::default();
    let first = bins.start(now).unwrap();
    assert_eq!(bins.start(now), None, "a read is under way");
    assert_eq!(
        bins.land(first, Vec::new()),
        (false, true, Vec::new()),
        "a read was asked for meanwhile"
    );
    let second = bins.start(now).unwrap();
    assert_eq!(bins.land(second, Vec::new()), (false, false, Vec::new()));
}

#[test]
fn a_drive_that_never_answers_keeps_its_one_read_and_the_others_still_land() {
    let want = |d: &str| Want {
        dir: PathBuf::from(d),
        mask: TRASH_MASK,
        only: Vec::new(),
    };
    let (a, b) = (PathBuf::from("/media/A"), PathBuf::from("/media/B"));
    let now = std::time::Instant::now();
    let mut bins = DriveBins::default();
    let first = bins.start(now).unwrap();
    let (changed, _, read) = bins.land(first, vec![a.clone(), b.clone()]);
    assert!(!changed);
    assert_eq!(read, [a.clone(), b.clone()]);
    assert!(bins.land_drive(b.clone(), first, vec![want("/media/B/.Trash-1000")], true));
    assert!(bins.full, "B's bin holds something while A never answers");
    assert_eq!(bins.wants, [want("/media/B/.Trash-1000")]);
    let id = bins.start(now).unwrap();
    let (_, _, read) = bins.land(id, vec![a.clone(), b.clone()]);
    assert_eq!(read, std::slice::from_ref(&b), "A's read is still out");
    assert!(bins.land_drive(a.clone(), first, vec![want("/media/A")], false));
    assert_eq!(bins.wants, [want("/media/A"), want("/media/B/.Trash-1000")]);
    let last = bins.start(now).unwrap();
    let (changed, _, _) = bins.land(last, vec![a.clone()]);
    assert!(changed && !bins.full, "B was ejected");
    assert!(
        !bins.land_drive(b, id, Vec::new(), true),
        "an answer from B is dropped"
    );
}

#[test]
fn a_drive_mounted_again_where_a_stuck_one_left_is_read_afresh() {
    let want = |d: &str| Want {
        dir: PathBuf::from(d),
        mask: TRASH_MASK,
        only: Vec::new(),
    };
    let a = PathBuf::from("/media/A");
    let now = std::time::Instant::now();
    let mut bins = DriveBins::default();
    let stuck = bins.start(now).unwrap();
    assert_eq!(
        bins.land(stuck, vec![a.clone()]).2,
        std::slice::from_ref(&a)
    );
    let gone = bins.start(now).unwrap();
    assert_eq!(bins.land(gone, Vec::new()).2, Vec::<PathBuf>::new());
    let back = bins.start(now).unwrap();
    let (_, _, read) = bins.land(back, vec![a.clone()]);
    assert_eq!(read, std::slice::from_ref(&a), "the new A is read");
    assert!(
        !bins.land_drive(a.clone(), stuck, vec![want("/media/A/old")], true),
        "the old A's late answer is dropped"
    );
    assert!(bins.land_drive(a.clone(), back, vec![want("/media/A/.Trash-1000")], true));
    assert_eq!(bins.wants, [want("/media/A/.Trash-1000")]);
    assert!(bins.full);
}

#[test]
fn a_drive_tab_read_still_out_starts_no_second_and_known_tabs_keep_mount_order() {
    let (a, b, c) = (
        PathBuf::from("/media/A"),
        PathBuf::from("/media/B"),
        PathBuf::from("/media/C"),
    );
    let tops = [a.clone(), b.clone(), c.clone()];
    let mut bins = DriveBins::default();
    let (first, read) = bins.start_tab_reads(&tops);
    assert_eq!(read, tops);
    assert!(bins.tabs(&tops).is_empty(), "no read has landed");
    bins.land_tab(c.clone(), first, Some(drive_tab("C")));
    bins.land_tab(a.clone(), first, Some(drive_tab("A")));
    let (second, read) = bins.start_tab_reads(&tops);
    assert_eq!(
        read,
        [a.clone(), c.clone()],
        "B never answered: its read is still out"
    );
    assert_eq!(bins.tabs(&tops), [drive_tab("A"), drive_tab("C")]);
    bins.land_tab(a.clone(), second, None);
    bins.land_tab(b, first, Some(drive_tab("B")));
    assert_eq!(
        bins.tabs(&tops),
        [drive_tab("B"), drive_tab("C")],
        "A has no bin now"
    );
    assert_eq!(
        bins.tabs(&[c]),
        [drive_tab("C")],
        "only drives still mounted"
    );
}

#[test]
fn a_drive_mounted_again_where_a_stuck_one_left_gets_its_tab_read_afresh() {
    let a = PathBuf::from("/media/A");
    let mut bins = DriveBins::default();
    let (stuck, _) = bins.start_tab_reads(std::slice::from_ref(&a));
    bins.drives_listed(&[]);
    bins.drives_listed(std::slice::from_ref(&a));
    let (back, read) = bins.start_tab_reads(std::slice::from_ref(&a));
    assert_eq!(read, std::slice::from_ref(&a), "the new A is read");
    bins.land_tab(a.clone(), stuck, Some(drive_tab("old")));
    assert!(
        bins.tabs(std::slice::from_ref(&a)).is_empty(),
        "the old A's late answer is dropped"
    );
    bins.land_tab(a.clone(), back, Some(drive_tab("A")));
    assert_eq!(bins.tabs(&[a]), [drive_tab("A")]);
}

fn drive_tab(label: &str) -> crate::binprops::Drive {
    crate::binprops::Drive {
        key: label.into(),
        label: label.into(),
        size: 1,
    }
}

#[test]
fn a_drive_top_the_user_cannot_read_is_not_watched() {
    use std::os::unix::fs::PermissionsExt;
    let top = std::env::temp_dir().join(format!("reprise-drive-tops-{}", std::process::id()));
    let (open, shut) = (top.join("open"), top.join("shut"));
    std::fs::create_dir_all(&open).unwrap();
    std::fs::create_dir_all(&shut).unwrap();
    std::fs::set_permissions(&shut, std::fs::Permissions::from_mode(0o300)).unwrap();
    let bins = reprisefs::trash::Bins::new(top.join("home"), vec![open.clone(), shut]);
    let dirs: Vec<PathBuf> = drive_tops(&bins)
        .iter()
        .flat_map(|top| drive_trash_wants(&bins, top))
        .map(|w| w.dir)
        .collect();
    let _ = std::fs::remove_dir_all(&top);
    assert_eq!(dirs, [open]);
}

#[test]
fn a_set_command_takes_the_clicked_icon_first_then_the_rest() {
    let docs = desktop::place("documents").unwrap();
    let bin = desktop::recycle_bin(PathBuf::from("/t/files"), true);
    let entries = vec![docs, file("a.txt"), file("b.txt"), bin];
    assert_eq!(
        aimed_in(&entries, &[0, 1, 2, 3], 2, false),
        vec![2, 0, 1, 3]
    );
    // Cut, Copy and Send To leave the places and the bin out of the rest.
    assert_eq!(aimed_in(&entries, &[0, 1, 2, 3], 2, true), vec![2, 1]);
    // A place clicked in the set still takes itself.
    assert_eq!(aimed_in(&entries, &[0, 1], 0, true), vec![0, 1]);
    // An index past the entries (a rescan shrank them) is dropped.
    assert_eq!(aimed_in(&entries, &[1, 9], 1, false), vec![1]);
}

#[test]
fn cell_key_owned_copy_matches_its_borrowed_key_until_the_caret_blinks() {
    let (label, text) = (String::from("Notes"), String::from("Notes"));
    let key = |on| CellKey {
        label: Cow::Borrowed(&label),
        icon: Cow::Borrowed("text-x-generic"),
        shortcut: false,
        cell: Rect::default(),
        selected: true,
        ring: false,
        cut: false,
        editing: Some((Cow::Borrowed(text.as_str()), 0..5, 5, on)),
        preedit: None,
    };
    let stored: CellKey<'static> = key(true).into_owned();
    assert!(matches!(stored.label, Cow::Owned(_)));
    assert!(stored == key(true));
    assert!(stored != key(false));
    let composing = CellKey {
        preedit: reprisekit::textedit::Preedit::new("ni".into(), 2, 2),
        ..key(true)
    };
    assert!(stored != composing, "a preedit redraws the box");
}

#[test]
fn remap_follows_the_file_and_drops_a_gone_entry() {
    let entry = |name: &str| DesktopEntry {
        name: name.into(),
        label: name.into(),
        icon: "text-x-generic".into(),
        target: Target::Open(PathBuf::from(name)),
        path: PathBuf::from(name),
        shortcut: false,
        kind: desktop::Kind::File,
        is_dir: false,
        allow_launching: false,
    };
    let bin = DesktopEntry {
        kind: desktop::Kind::RecycleBin,
        ..entry("Recycle Bin")
    };
    let old = [entry("b.txt"), entry("c.txt"), bin.clone()];
    let new = [bin, entry("a.txt"), entry("b.txt")];
    assert_eq!(
        remap(&old, &new),
        [Some(2), None, Some(0)],
        "b.txt moves to row 2, c.txt is gone, the bin moves to row 0"
    );
}

/// New Folder lands in a cell while a paste aimed at it waits on a mount that does not
/// answer: the paste, landing after, takes the next free cell and the folder keeps its own.
#[test]
fn a_late_answer_never_takes_a_cell_an_icon_landed_in_meanwhile() {
    let plain = |name: &str| DesktopEntry {
        name: name.into(),
        label: name.into(),
        icon: "folder".into(),
        target: Target::Open(PathBuf::from(name)),
        path: PathBuf::from(name),
        shortcut: false,
        kind: desktop::Kind::File,
        is_dir: true,
        allow_launching: false,
    };
    let area = Rect::new(0, 0, 800, 600);
    let square = desktop::Cell::square(desktop::CELL);
    let at = |c: (i32, i32)| desktop::cell_rect(square, area, c.0, c.1);
    let entries = [plain("New Folder")];
    let cells = [at((2, 1))];
    assert_eq!(
        landing_cell(square, area, &entries, &cells, &[], "report.pdf", (2, 1)),
        (2, 2),
        "the paste goes to the next free cell"
    );
    assert_eq!(
        landing_cell(square, area, &entries, &cells, &[], "New Folder", (2, 1)),
        (2, 1),
        "an icon is not in its own way"
    );
    assert_eq!(
        landing_cell(square, area, &[], &[], &[(2, 1)], "report.pdf", (2, 1)),
        (2, 2),
        "a cell landed in before the read shows it is taken"
    );
    assert_eq!(
        landing_cell(square, area, &entries, &cells, &[], "report.pdf", (4, 0)),
        (4, 0)
    );
}

#[test]
fn an_answer_lands_in_the_grown_grid_cell_holding_the_cell_it_was_asked_for() {
    let area = Rect::new(0, 0, 800, 600);
    let big = desktop::Cell::grown(reprisekit::theme::TextUnits::new(22, 11));
    assert_eq!(
        landing_cell(big, area, &[], &[], &[], "report.pdf", (4, 2)),
        (2, 1),
        "100% cell (4, 2) sits in cell (2, 1) of the 200% grid"
    );
    assert_eq!(
        landing_cell(big, area, &[], &[], &[(4, 2)], "notes.txt", (5, 2)),
        (2, 2),
        "an answer landed at 100% still claims its cell after the grid grew"
    );
}

#[test]
fn a_launchers_name_key_moves_to_its_file_name_once() {
    let launcher = |file: &str, name: &str| DesktopEntry {
        name: file.into(),
        label: name.into(),
        icon: "firefox".into(),
        target: Target::Launch(
            apps::parse_desktop(
                &format!("[Desktop Entry]\nType=Application\nName={name}\nExec=x\n"),
                file,
            )
            .unwrap(),
        ),
        path: PathBuf::from(file),
        shortcut: true,
        kind: desktop::Kind::File,
        is_dir: false,
        allow_launching: false,
    };
    let plain = |name: &str| DesktopEntry {
        name: name.into(),
        label: name.into(),
        icon: "text-x-generic".into(),
        target: Target::Open(PathBuf::from(name)),
        path: PathBuf::from(name),
        shortcut: false,
        kind: desktop::Kind::File,
        is_dir: false,
        allow_launching: false,
    };
    let entries = [
        launcher("firefox.desktop", "Firefox"),
        launcher("firefox (2).desktop", "Firefox"),
        launcher("notes.desktop", "notes.txt"),
        plain("notes.txt"),
    ];
    let mut positions = BTreeMap::from([
        ("Firefox".to_string(), (3, 1)),
        ("notes.txt".to_string(), (0, 5)),
    ]);
    assert!(migrate_launcher_keys(&entries, &mut positions));
    assert_eq!(
        positions,
        BTreeMap::from([
            ("firefox.desktop".to_string(), (3, 1)),
            ("notes.txt".to_string(), (0, 5)),
        ]),
        "the first Firefox takes the cell; notes.txt keeps its own"
    );
    assert!(!migrate_launcher_keys(&entries, &mut positions));
}

#[test]
fn recycle_bin_menu_greys_empty_and_entry_cannot_be_trashed() {
    use DesktopCommand::*;
    let (items, commands) = bin_menu(true);
    assert_eq!(
        commands,
        vec![
            Some(Open),
            Some(Explore),
            None,
            None,
            Some(CreateShortcut),
            None,
            Some(BinProperties)
        ]
    );
    assert!(
        matches!(&items[2], MenuItem::Entry { enabled: false, label, .. } if label == "Empty Recycle Bin")
    );
    assert!(matches!(items[0], MenuItem::Entry { bold: true, .. }));
    let (_, commands) = bin_menu(false);
    assert_eq!(commands[2], Some(EmptyBin));
    // The place menus: Documents and Home have Explore and Copy, and
    // Properties wherever there is a folder behind them; Computer has
    // Reprise Settings and About Reprise last, after a separator. All
    // three Delete and Rename.
    let (items, commands) = place_menu(desktop::Place::Documents, true);
    let folder_rows = vec![
        Some(Open),
        Some(Explore),
        None,
        Some(Copy),
        None,
        Some(CreateShortcut),
        Some(Trash),
        Some(Rename),
        None,
        Some(IconProperties),
    ];
    assert_eq!(commands, folder_rows);
    assert_eq!(place_menu(desktop::Place::Home, true), (items, commands));
    let (items, commands) = place_menu(desktop::Place::Computer, false);
    assert_eq!(
        commands,
        vec![
            Some(Open),
            None,
            Some(CreateShortcut),
            Some(Trash),
            Some(Rename),
            None,
            Some(RepriseSettings),
            Some(AboutReprise),
        ]
    );
    let rows: Vec<(String, Option<usize>, bool)> = items
        .iter()
        .filter_map(|i| match i {
            MenuItem::Entry {
                label,
                mnemonic,
                bold,
                ..
            } => Some((label.clone(), *mnemonic, *bold)),
            _ => None,
        })
        .collect();
    assert_eq!(
        rows,
        [
            ("Open".to_string(), Some(0), true),
            ("Create Shortcut".to_string(), Some(7), false),
            ("Delete".to_string(), Some(0), false),
            ("Rename".to_string(), Some(4), false),
            ("Reprise Settings".to_string(), Some(0), false),
            ("About Reprise".to_string(), Some(0), false),
        ]
    );
    let keys: std::collections::HashSet<char> = rows
        .iter()
        .map(|(l, m, _)| l.chars().nth(m.unwrap()).unwrap().to_ascii_lowercase())
        .collect();
    assert_eq!(keys.len(), rows.len(), "no two rows share an access key");
    let pc = desktop::place("computer").unwrap();
    assert_eq!(
        (pc.name.as_str(), pc.icon.as_str()),
        ("Computer", "computer")
    );
    assert!(matches!(&pc.target, Target::Uri(u) if u == "computer:///"));
    assert_eq!(pc.kind, desktop::Kind::Place(desktop::Place::Computer));
    assert!(desktop::place("my-music").is_none());
    let cmd = desktop::place_command(&pc);
    assert_eq!(cmd.as_deref(), Some("reprise-explorer Computer"));
    let bin = desktop::recycle_bin(PathBuf::from("/t/files"), false);
    assert_eq!(
        desktop::place_command(&bin).as_deref(),
        Some("reprise-explorer \"Recycle Bin\"")
    );
    assert_eq!(
        (bin.name.as_str(), bin.icon.as_str()),
        ("Recycle Bin", "user-trash-full")
    );
    assert_eq!(
        desktop::recycle_bin(PathBuf::from("/t/files"), true).icon,
        "user-trash"
    );
    assert_eq!(bin.kind, desktop::Kind::RecycleBin);
}

#[test]
fn a_slideshow_adds_next_desktop_background_after_refresh() {
    let (items, commands) = desktop_menu(false, true, true);
    assert_eq!(items.len(), 11);
    assert_eq!(commands[1], Some(DesktopCommand::Refresh));
    assert_eq!(commands[2], Some(DesktopCommand::NextBackground));
    assert!(matches!(
        &items[2],
        MenuItem::Entry { label, mnemonic: Some(13), enabled: true, .. }
            if label == "Next desktop background"
    ));
    assert!(matches!(items[3], MenuItem::Separator));
    assert_eq!(desktop_menu(false, true, false).0.len(), 10);
}

#[test]
fn with_the_files_off_the_desktop_menu_greys_new_and_paste() {
    let (items, commands) = desktop_menu(true, false, false);
    for i in [3, 4, 6] {
        assert!(
            matches!(items[i], MenuItem::Entry { enabled: false, .. }),
            "row {i} is greyed with the desktop's files off"
        );
        assert_eq!(commands[i], None, "row {i} does nothing");
    }
    // New keeps its submenu arrow, greyed.
    assert!(matches!(items[6], MenuItem::Entry { submenu: true, .. }));
    for i in [0, 1, 8, 9] {
        assert!(matches!(items[i], MenuItem::Entry { enabled: true, .. }));
    }
    assert_eq!(commands[1], Some(DesktopCommand::Refresh));
    assert_eq!(commands[8], Some(DesktopCommand::Settings));
    assert_eq!(commands[9], Some(DesktopCommand::AboutReprise));
}

#[test]
fn desktop_menu_lists_its_commands_with_separators() {
    let (items, commands) = desktop_menu(false, true, false);
    assert_eq!(items.len(), 10);
    assert_eq!(commands[0], Some(DesktopCommand::ArrangeIcons));
    assert!(matches!(items[0], MenuItem::Entry { submenu: true, .. }));
    assert_eq!(commands[1], Some(DesktopCommand::Refresh));
    assert!(matches!(items[2], MenuItem::Separator));
    // Paste and Paste Shortcut are there, greyed out with nothing
    // copied.
    assert!(matches!(items[3], MenuItem::Entry { enabled: false, .. }));
    assert!(matches!(items[4], MenuItem::Entry { enabled: false, .. }));
    assert_eq!(commands[3], None);
    assert!(matches!(items[5], MenuItem::Separator));
    assert_eq!(commands[6], Some(DesktopCommand::New));
    assert!(matches!(items[6], MenuItem::Entry { submenu: true, .. }));
    assert!(matches!(items[7], MenuItem::Separator));
    assert_eq!(commands[8], Some(DesktopCommand::Settings));
    assert!(matches!(
        &items[8],
        MenuItem::Entry { label, mnemonic: Some(0), .. } if label == "Reprise Settings"
    ));
    assert_eq!(commands[9], Some(DesktopCommand::AboutReprise));
    assert!(matches!(
        &items[9],
        MenuItem::Entry { label, mnemonic: Some(0), .. } if label == "About Reprise"
    ));
    let (items, commands) = new_menu();
    assert_eq!(items.len(), NEW_MENU_ICONS.len());
    assert_eq!(commands[0], Some(DesktopCommand::NewFolder));
    assert!(matches!(items[1], MenuItem::Separator));
    assert_eq!(commands[2], Some(DesktopCommand::NewTextDocument));
    let (items, commands) = arrange_menu(true);
    assert_eq!(
        commands,
        vec![
            Some(DesktopCommand::ArrangeByName),
            None,
            Some(DesktopCommand::AutoArrange)
        ]
    );
    assert!(matches!(items[2], MenuItem::Entry { checked: true, .. }));
    assert!(matches!(
        arrange_menu(false).0[2],
        MenuItem::Entry { checked: false, .. }
    ));
    // With something on the clipboard the Paste pair comes alive.
    let (items, commands) = desktop_menu(true, true, false);
    assert!(matches!(items[3], MenuItem::Entry { enabled: true, .. }));
    assert_eq!(commands[3], Some(DesktopCommand::Paste));
    assert_eq!(commands[4], Some(DesktopCommand::PasteShortcut));
    let (items, _) = desktop_menu(false, true, false);
    for i in [0, 1, 3, 4, 6, 8, 9] {
        assert!(matches!(
            items[i],
            MenuItem::Entry {
                mnemonic: Some(_),
                ..
            }
        ));
    }
}

#[test]
fn icon_menu_has_its_rows_with_open_bold() {
    use DesktopCommand::*;
    let (items, commands) = icon_menu(IconMenu::default());
    assert_eq!(
        commands,
        vec![
            Some(Open),
            Some(OpenWith),
            None,
            Some(SendTo),
            None,
            Some(Cut),
            Some(Copy),
            None,
            Some(CreateShortcut),
            Some(Trash),
            Some(Rename),
            None,
            Some(IconProperties)
        ]
    );
    assert!(matches!(items[0], MenuItem::Entry { bold: true, .. }));
    assert!(matches!(items[2], MenuItem::Separator));
    assert!(matches!(items[3], MenuItem::Entry { submenu: true, .. }));
    assert!(matches!(items[7], MenuItem::Separator));
    // A folder gets Explore and the greyed Sharing and Security...
    // under Open.
    let (items, commands) = icon_menu(IconMenu {
        is_dir: true,
        print: true,
        edit: true,
        ..Default::default()
    });
    assert_eq!(commands[1], Some(Explore));
    assert!(matches!(&items[1], MenuItem::Entry { label, .. } if label == "Explore"));
    assert!(matches!(&items[2], MenuItem::Separator));
    assert!(matches!(
        &items[3],
        MenuItem::Entry { label, enabled: false, .. } if label == "Sharing and Security..."
    ));
    assert_eq!(commands[3], None);
    assert_eq!(commands[5], Some(SendTo));
    assert_eq!(commands[7], Some(Cut));
    // A file gets Open With, a submenu, there instead, with its verbs
    // before it: Print, then Edit.
    let (items, commands) = icon_menu(IconMenu::default());
    assert_eq!(commands[1], Some(OpenWith));
    assert!(matches!(&items[1], MenuItem::Entry { submenu: true, .. }));
    let (_, commands) = icon_menu(IconMenu {
        print: true,
        edit: true,
        ..Default::default()
    });
    assert_eq!(
        &commands[..4],
        &[Some(Open), Some(Print), Some(Edit), Some(OpenWith)]
    );
    let (_, commands) = icon_menu(IconMenu {
        edit: true,
        ..Default::default()
    });
    assert_eq!(&commands[..3], &[Some(Open), Some(Edit), Some(OpenWith)]);
    // A .desktop file that is not a launcher gets Allow Launching
    // under Open With.
    let (items, commands) = icon_menu(IconMenu {
        edit: true,
        allow_launching: true,
        ..Default::default()
    });
    assert_eq!(commands[3], Some(AllowLaunching));
    assert!(matches!(
        &items[3],
        MenuItem::Entry { label, mnemonic: Some(6), .. } if label == "Allow Launching"
    ));
    let (items, commands) = open_with_menu(&[
        ("vim".into(), "Vim".into()),
        ("tj".into(), "Tom & Jerry".into()),
    ]);
    assert_eq!(
        commands,
        vec![
            Some(OpenWithApp("vim".into())),
            Some(OpenWithApp("tj".into())),
            None,
            Some(ChooseProgram)
        ]
    );
    assert!(
        matches!(&items[1], MenuItem::Entry { label, mnemonic: None, .. } if label == "Tom & Jerry")
    );
    assert!(matches!(items[2], MenuItem::Separator));
    let (items, commands) = open_with_menu(&[]);
    assert_eq!(commands, vec![Some(ChooseProgram)]);
    assert!(matches!(&items[0], MenuItem::Entry { label, .. } if label == "Choose Program..."));
}

#[test]
fn a_folder_offers_paste_and_a_launcher_a_greyed_run_as() {
    use DesktopCommand::*;
    let label = |items: &[MenuItem], i: usize| match &items[i] {
        MenuItem::Entry { label, enabled, .. } => (label.clone(), *enabled),
        _ => (String::new(), false),
    };
    // A folder's Paste sits under Copy, alone or in a set.
    for in_set in [false, true] {
        let (items, commands) = icon_menu(IconMenu {
            is_dir: true,
            in_set,
            ..Default::default()
        });
        let copy = commands.iter().position(|c| *c == Some(Copy)).unwrap();
        assert_eq!(label(&items, copy + 1), ("Paste".into(), true));
        assert_eq!(commands[copy + 1], Some(PasteInto));
        assert!(matches!(
            &items[copy + 1],
            MenuItem::Entry {
                mnemonic: Some(0),
                ..
            }
        ));
        assert!(matches!(items[copy + 2], MenuItem::Separator));
    }
    // A file's menu has no Paste.
    let (_, commands) = icon_menu(IconMenu::default());
    assert!(!commands.contains(&Some(PasteInto)));
    // A launcher's Run as... is greyed under Open; a document has none.
    let (items, commands) = icon_menu(IconMenu {
        launcher: true,
        ..Default::default()
    });
    assert_eq!(label(&items, 1), ("Run as...".into(), false));
    assert!(matches!(
        &items[1],
        MenuItem::Entry {
            mnemonic: Some(4),
            ..
        }
    ));
    assert_eq!(&commands[..3], &[Some(Open), None, None]);
    let (items, _) = icon_menu(IconMenu::default());
    assert!(
        items
            .iter()
            .all(|i| !matches!(i, MenuItem::Entry { label, .. } if label == "Run as..."))
    );
}

#[test]
fn a_launcher_has_paste_under_copy_and_no_open_with() {
    use DesktopCommand::*;
    let (items, commands) = icon_menu(IconMenu {
        launcher: true,
        edit: true,
        ..Default::default()
    });
    assert_eq!(
        commands,
        vec![
            Some(Open),
            None,
            Some(Edit),
            None,
            Some(SendTo),
            None,
            Some(Cut),
            Some(Copy),
            Some(PasteOpen),
            None,
            Some(CreateShortcut),
            Some(Trash),
            Some(Rename),
            None,
            Some(IconProperties)
        ]
    );
    assert!(matches!(
        &items[8],
        MenuItem::Entry { label, enabled: true, mnemonic: Some(0), .. } if label == "Paste"
    ));
}

#[test]
fn an_icon_in_a_set_keeps_its_own_menu_less_sharing() {
    use DesktopCommand::*;
    // A folder's drops the greyed Sharing and Security... and the
    // separator over it.
    let (items, commands) = icon_menu(IconMenu {
        is_dir: true,
        in_set: true,
        ..Default::default()
    });
    assert_eq!(
        commands,
        vec![
            Some(Open),
            Some(Explore),
            None,
            Some(SendTo),
            None,
            Some(Cut),
            Some(Copy),
            Some(PasteInto),
            None,
            Some(CreateShortcut),
            Some(Trash),
            Some(Rename),
            None,
            Some(IconProperties)
        ]
    );
    assert!(
        items
            .iter()
            .all(|i| !matches!(i, MenuItem::Entry { label, .. } if label.starts_with("Sharing")))
    );
    // A file's keeps its verbs and Open With.
    let set = icon_menu(IconMenu {
        print: true,
        allow_launching: true,
        in_set: true,
        ..Default::default()
    })
    .1;
    assert_eq!(
        set,
        icon_menu(IconMenu {
            print: true,
            allow_launching: true,
            ..Default::default()
        })
        .1
    );
    assert_eq!(
        &set[..4],
        &[
            Some(Open),
            Some(Print),
            Some(OpenWith),
            Some(AllowLaunching)
        ]
    );
}

#[test]
fn a_band_edge_cropped_from_its_strip_matches_one_drawn_in_place() {
    use reprisekit::classic::widgets::draw_focus_ring_screen;
    let ink = reprisekit::theme::Color::rgb(255, 255, 255);
    for scale in [1, 2] {
        for (edge, vertical) in [
            (Rect::new(10, 20, 7, 1), false),
            (Rect::new(11, 20, 7, 1), false),
            (Rect::new(-3, 4, 1, 9), true),
            (Rect::new(5, 8, 1, 6), true),
        ] {
            let mut direct = crate::raster::Canvas::new(
                (edge.width * scale) as u32,
                (edge.height * scale) as u32,
            );
            let full = Rect::new(0, 0, edge.width * scale, edge.height * scale);
            draw_focus_ring_screen(&mut direct, full, scale, ink, (edge.x, edge.y));
            let strip = band_strip(20, vertical, scale, ink);
            let crop = band_crop(edge, vertical, scale);
            assert_eq!((crop.width, crop.height), (full.width, full.height));
            for y in 0..full.height {
                for x in 0..full.width {
                    assert_eq!(
                        strip.get(crop.x + x, crop.y + y),
                        direct.get(x, y),
                        "edge {edge:?} at scale {scale}: px ({x}, {y}) matches the in-place ring"
                    );
                }
            }
        }
    }
}

#[test]
fn the_drop_menu_bolds_the_natural_row_and_greys_what_the_drag_disallows() {
    use crate::dragview::{Allowed, DropOp};
    let bold = |items: &[MenuItem]| -> Vec<bool> {
        items
            .iter()
            .map(|i| matches!(i, MenuItem::Entry { bold: true, .. }))
            .collect()
    };
    let (drop, drop_cmds) = drop_menu(false, DropOp::Move, Allowed::ALL);
    assert_eq!(drop_cmds[1], Some(DesktopCommand::DropMove));
    assert_eq!(bold(&drop), [false, true, false, false, false]);
    assert_eq!(drop_cmds[4], Some(DesktopCommand::DropCancel));
    // A drag that only allows a copy greys Move Here; one that only
    // allows a move greys Copy Here and Create Shortcuts Here.
    let enabled = |allowed| -> Vec<bool> {
        drop_menu(false, DropOp::Copy, allowed)
            .0
            .iter()
            .map(|i| matches!(i, MenuItem::Entry { enabled: true, .. }))
            .collect()
    };
    let only = |copy, moves| Allowed { copy, moves };
    assert_eq!(enabled(only(true, false)), [true, false, true, false, true]);
    assert_eq!(
        enabled(only(false, true)),
        [false, true, false, false, true]
    );
    // The row a drop would have taken is the bold one.
    assert_eq!(
        bold(&drop_menu(false, DropOp::Copy, Allowed::ALL).0),
        [true, false, false, false, false]
    );
    assert_eq!(
        bold(&drop_menu(false, DropOp::Shortcut, Allowed::ALL).0),
        [false, false, true, false, false]
    );
    // A system icon only moves or becomes a shortcut.
    let (drop, drop_cmds) = drop_menu(true, DropOp::Copy, Allowed::ALL);
    assert_eq!(
        drop_cmds,
        [
            Some(DesktopCommand::DropMove),
            Some(DesktopCommand::DropShortcut),
            None,
            Some(DesktopCommand::DropCancel),
        ]
    );
    assert!(matches!(drop[0], MenuItem::Entry { bold: true, .. }));
}

#[test]
fn a_drag_carries_the_grabbed_icon_first_then_its_set() {
    let entries = [file("a.txt"), file("b.txt"), file("c.txt")];
    let sources: Vec<String> = aimed_in(&entries, &[0, 1, 2], 1, false)
        .into_iter()
        .map(|i| match source_of(&entries[i]) {
            crate::dragview::DragSource::Icon { name, .. } => name,
            other => panic!("an icon drags as an icon, not {other:?}"),
        })
        .collect();
    assert_eq!(sources, ["b.txt", "a.txt", "c.txt"]);
}
