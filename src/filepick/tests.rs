use super::run::read_folder;
use super::*;
use crate::dialog::Outcome;
use crate::filedlggeom::OWN_APP;
use crate::raster::Rect;
use reprisekit::art::viewsicon;
use reprisekit::controls::form::Key;
use reprisekit::controls::shellview::{Entry, Mode};
use reprisekit::icontheme::IconCache;
use reprisekit::textedit::EditKey;
use reprisekit::theme::TextUnits;
use std::collections::BTreeSet;
use std::os::unix::fs::PermissionsExt;

const R: Rect = Rect::new(0, 0, filedlg::CLIENT.0, filedlg::CLIENT.1);

fn cache() -> IconCache {
    IconCache::new(reprisekit::icontheme::IconTheme::load("test", &[]))
}

#[test]
fn views_without_a_theme_icon_draws_at_the_dialogs_scale() {
    for scale in [1, 2] {
        let views = icons(&mut cache(), scale, TextUnits::ONE).toolbar[TOOLBAR_VIEWS]
            .clone()
            .expect("the drawn Views icon");
        assert_eq!(
            (views.width, views.height, views.scale()),
            (16 * scale as u32, 16 * scale as u32, 1)
        );
        assert_eq!(
            views.pixels,
            viewsicon::draw16_at(scale).pixels,
            "scale {scale}"
        );
    }
}

/// A text size change fetches the toolbar's icons grown, the drawn Views icon 32 px at 200%,
/// and relists the folder only for List and Details' grown icons.
#[test]
fn a_text_size_change_grows_the_toolbar_icons_and_restyles_the_entries() {
    let dir = sandbox("icons-grown");
    let mut cache = cache();
    let mut p = FilePicker::run_command(&dir, icons(&mut cache, 1, TextUnits::ONE), true);
    settle(&mut p);
    let views = |p: &FilePicker| p.icons.toolbar[TOOLBAR_VIEWS].as_ref().map(|c| c.width);
    assert!(!p.follow_scale(1, &mut cache), "nothing changed");
    assert_eq!(views(&p), Some(16));
    p.dlg.set_text(TextUnits::new(22, 11));
    assert!(p.follow_scale(1, &mut cache));
    assert_eq!(views(&p), Some(32));
    assert_eq!(p.take_requests().len(), 1, "one relisting");
    assert!(p.restyling.is_some(), "for the entries' icons only");
}

/// Runs the picker's queued requests the way the host's thread would,
/// returning the last outcome that did something.
fn settle(p: &mut FilePicker) -> Outcome {
    settle_with(p, &mut cache())
}

fn settle_with(p: &mut FilePicker, icons: &mut IconCache) -> Outcome {
    let mut out = Outcome::Nothing;
    for r in p.take_requests() {
        match p.reply(r.run(), icons, 1) {
            Outcome::Nothing => {}
            o => out = o,
        }
    }
    out
}

fn names(p: &FilePicker) -> Vec<&str> {
    p.dlg.entries().iter().map(|e| e.name.as_str()).collect()
}

/// A scratch folder of this process's own: test runs of other checkouts
/// share the temp dir.
fn sandbox(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("reprise-filepick-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("Pictures")).expect("mkdir");
    for f in ["dunes", ".hidden", "Pictures/meadow"] {
        std::fs::write(dir.join(f), "#!/bin/sh\n").expect("write");
        std::fs::set_permissions(dir.join(f), std::fs::Permissions::from_mode(0o755))
            .expect("chmod");
    }
    std::fs::write(dir.join("notes.txt"), "x").expect("write");
    std::fs::set_permissions(
        dir.join("notes.txt"),
        std::fs::Permissions::from_mode(0o644),
    )
    .expect("chmod");
    dir
}

/// A typed ".." or "a/../b" enters the folder it names, whose Up is
/// the folder above it.
#[test]
fn a_typed_dot_dot_enters_the_folder_above() {
    let dir = sandbox("dotdot");
    let mut p = FilePicker::run_command(&dir.join("Pictures"), Icons::default(), true);
    settle(&mut p);
    p.dlg.set_file_name("..");
    p.key(R, Key::Enter);
    assert_eq!(settle(&mut p), Outcome::Redraw);
    assert_eq!(p.current(), Some(dir.as_path()));
    assert_eq!(names(&p), ["Pictures", "dunes"]);
    p.click(R, Some(filedlg::Hit::ToolBar(1)));
    assert_eq!(p.current(), dir.parent());

    p.dlg
        .set_file_name(&format!("{}/Pictures/../Pictures", dir.display()));
    p.key(R, Key::Enter);
    settle(&mut p);
    assert_eq!(p.current(), Some(dir.join("Pictures").as_path()));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn browse_opens_in_the_view_its_own_table_keeps() {
    let dir = sandbox("remember");
    let file = dir.join("state").join("filedialog.toml");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(
        &file,
        "[reprise]\nview = \"details\"\nsort = \"size\"\nascending = false\n\
         [other]\nview = \"icons\"\n",
    )
    .unwrap();
    let mut p = FilePicker::run_command(&dir, Icons::default(), true);
    p.remember_in(file.clone());
    assert_eq!(p.dlg.view(), Mode::List, "the file is read off the loop");
    settle(&mut p);
    assert_eq!(p.dlg.view(), Mode::Details);
    p.click(R, Some(filedlg::Hit::ToolBar(3)));
    p.click(R, Some(filedlg::Hit::ViewsItem(0)));
    crate::state::flush();
    let text = std::fs::read_to_string(&file).unwrap();
    let mine = repriseconf::filedialog::section(&text, OWN_APP);
    assert!(
        mine.contains("view = \"thumbnails\"\n") && mine.contains("sort = \"size\"\n"),
        "{text}"
    );
    assert_eq!(
        repriseconf::filedialog::section(&text, "other"),
        "view = \"icons\"\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn pinned_folders_follow_the_built_in_tiles() {
    let dir = sandbox("pins");
    let mut p = FilePicker::run_command(&dir, Icons::default(), true);
    let built_in = p.dlg.places().len();
    let pin = |path: PathBuf, present| reprisefs::pins::Pin {
        label: reprisefs::places::dialog_label(&path),
        path,
        present,
    };
    let pins = [pin(dir.clone(), true), pin(dir.join("gone"), false)];
    assert!(p.set_pins(&pins));
    assert!(!p.set_pins(&pins), "the same pins change nothing");
    let tiles: Vec<(bool, bool)> = (p.dlg.places().iter())
        .map(|t| (t.enabled, t.pinned))
        .collect();
    assert_eq!(tiles[built_in..], [(true, true), (false, true)]);
    assert_eq!(p.event(Event::NeedPlace(built_in + 1)), Outcome::Nothing);
    assert_eq!(p.event(Event::NeedPlace(built_in)), Outcome::Redraw);
    assert!(p.set_pins(&[]));
    assert_eq!(p.dlg.places().len(), built_in);
}

#[test]
fn a_view_picked_before_the_stored_one_arrives_wins() {
    let dir = sandbox("remember-early");
    let file = dir.join("state").join("filedialog.toml");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, "[reprise]\nview = \"details\"\n").unwrap();
    let mut p = FilePicker::run_command(&dir, Icons::default(), true);
    p.remember_in(file.clone());
    let replies: Vec<Reply> = p.take_requests().into_iter().map(Request::run).collect();
    p.click(R, Some(filedlg::Hit::ToolBar(3)));
    p.click(R, Some(filedlg::Hit::ViewsItem(0)));
    for r in replies {
        p.reply(r, &mut cache(), 1);
    }
    assert_eq!(p.dlg.view(), Mode::Thumbnails);
    crate::state::flush();
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(
        repriseconf::filedialog::section(&text, OWN_APP).contains("view = \"thumbnails\"\n"),
        "the picked view is stored: {text}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn typing_fills_the_file_name() {
    let dir = sandbox("type");
    let mut p = FilePicker::run_command(&dir, Icons::default(), true);
    p.key(R, Key::Edit(EditKey::Insert("D".into())));
    p.key(R, Key::Edit(EditKey::Insert("u".into())));
    assert_eq!(p.dlg.file_name(), "Du");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Run's Browse lists executables under "Programs", everything under
/// "All Files", and hands the pick to Run.
#[test]
fn run_browse_lists_programs_and_hands_the_pick_to_run() {
    let dir = sandbox("run");
    std::fs::write(dir.join("tool"), "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(dir.join("tool"), std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut p = FilePicker::run_command(&dir, Icons::default(), true);
    settle(&mut p);
    assert_eq!(names(&p), ["Pictures", "dunes", "tool"]);
    // Files of type -> All Files: Down on the focused combo picks it.
    p.key(R, Key::Tab { back: false });
    p.key(R, Key::Down);
    settle(&mut p);
    assert_eq!(names(&p), ["Pictures", "dunes", "notes.txt", "tool"]);
    p.dlg.set_file_name("tool");
    p.key(R, Key::Enter);
    assert_eq!(settle(&mut p), Outcome::RunPick(dir.join("tool")));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A typed pattern lists what it matches and every folder; one with
/// a folder part enters that folder, the field keeping the pattern.
/// One whose folder part is no folder leaves the dialog as it was.
#[test]
fn a_typed_pattern_filters_the_listing() {
    let dir = sandbox("wildcard");
    let mut p = FilePicker::run_command(&dir, Icons::default(), true);
    settle(&mut p);
    p.dlg.set_file_name("*.txt");
    p.key(R, Key::Enter);
    assert_eq!(settle(&mut p), Outcome::Redraw);
    assert_eq!(names(&p), ["Pictures", "notes.txt"]);
    p.dlg.set_file_name("Pictures/m*");
    p.key(R, Key::Enter);
    assert_eq!(settle(&mut p), Outcome::Redraw);
    assert_eq!(p.current(), Some(dir.join("Pictures").as_path()));
    assert_eq!(names(&p), ["meadow"]);
    assert_eq!(p.dlg.file_name(), "m*");
    p.dlg.set_file_name("../dunes/*");
    p.key(R, Key::Enter);
    assert_eq!(
        settle(&mut p),
        Outcome::Nothing,
        "a file is no folder to list"
    );
    assert_eq!(p.dlg.file_name(), "../dunes/*", "the field as typed");
    assert_eq!(p.dlg.wildcard(), Some("m*"), "the filter stays");
    assert_eq!(names(&p), ["meadow"], "the listing stays");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A places tile enters the folder `place_dirs` gives it; a tile
/// index with none leaves the folder on screen.
#[test]
fn a_places_tile_enters_its_folder() {
    let dir = sandbox("place");
    let mut p = FilePicker::run_command(&dir, Icons::default(), true);
    assert_eq!(p.event(Event::NeedPlace(6)), Outcome::Nothing);
    assert_eq!(p.current(), Some(dir.as_path()));
    assert_eq!(p.event(Event::NeedPlace(5)), Outcome::Redraw);
    assert_eq!(p.current(), Some(Path::new("/")));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A file picked in My Recent Documents is the file at its own path.
#[test]
fn a_recent_file_picks_its_own_path() {
    let dir = sandbox("recent");
    let mut p = FilePicker::run_command(&dir, Icons::default(), true);
    settle(&mut p);
    let recent = p.folders.id(Path::new(reprisefs::places::RECENT));
    p.list(recent);
    p.take_requests();
    let file = dir.join("dunes");
    p.reply(
        Reply {
            seq: p.listing.unwrap(),
            scale: 1,
            list: 16,
            id: Some(recent),
            path: PathBuf::from(reprisefs::places::RECENT),
            pattern: None,
            found: Found::Folder(read_folder(&dir, Some(&[]), None)),
            art: Vec::new(),
            view: None,
        },
        &mut cache(),
        1,
    );
    assert!(!p.dlg.can_go_up(), "Recent has no folder above it");
    p.dlg.set_file_name("nope");
    p.key(R, Key::Enter);
    assert!(
        p.take_requests().is_empty(),
        "a name not listed goes nowhere"
    );
    p.dlg.set_file_name("dunes");
    p.key(R, Key::Enter);
    let reqs = p.take_requests();
    assert_eq!(reqs[0].path, file);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_double_click_enters_a_folder() {
    let dir = sandbox("double");
    let mut p = FilePicker::run_command(&dir, Icons::default(), true);
    settle(&mut p);
    let hit = Some(filedlg::Hit::Shell(
        reprisekit::controls::shellview::Hit::Entry(0),
    ));
    p.double_click(R, hit);
    settle(&mut p);
    assert_eq!(p.current(), Some(dir.join("Pictures").as_path()));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Entering a folder shows it empty at once and reads it only when
/// the host runs the request, so a folder that never answers leaves
/// the dialog working.
#[test]
fn a_folder_shows_empty_until_its_listing_lands() {
    let dir = sandbox("pending");
    let mut p = FilePicker::run_command(&dir, Icons::default(), true);
    assert_eq!(p.current(), Some(dir.as_path()));
    assert_eq!(names(&p), Vec::<&str>::new());
    assert_eq!(settle(&mut p), Outcome::Redraw);
    assert_eq!(names(&p), ["Pictures", "dunes"]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A listing that lands after the user moved on is dropped: the
/// folder picked last stays on screen.
#[test]
fn a_stale_listing_is_dropped() {
    let dir = sandbox("stale");
    let mut p = FilePicker::run_command(&dir, Icons::default(), true);
    settle(&mut p);
    p.dlg.set_file_name("Pictures");
    p.key(R, Key::Enter);
    settle(&mut p);
    // Back to `dir`, then into Pictures again before `dir`'s listing
    // lands.
    let back = p.dlg.nav(reprisekit::pointer::Nav::Back);
    p.event(back);
    let slow = p.take_requests();
    let pictures = p.folders.id(&dir.join("Pictures"));
    p.event(Event::NeedListing(pictures));
    assert_eq!(settle(&mut p), Outcome::Redraw);
    for r in slow {
        assert_eq!(
            p.reply(r.run(), &mut cache(), 1),
            Outcome::Nothing,
            "stale reply changes nothing"
        );
    }
    assert_eq!(p.current(), Some(dir.join("Pictures").as_path()));
    assert_eq!(names(&p), ["meadow"]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Relisting a folder whose listing has not answered asks for no second job: the first
/// one's reply fills the dialog, so a folder on a dead mount strands one thread, not one per
/// relist.
#[test]
fn relisting_a_folder_still_being_read_queues_no_second_read() {
    let dir = sandbox("relist");
    let mut p = FilePicker::run_command(&dir, Icons::default(), true);
    let first = p.take_requests();
    assert_eq!(first.len(), 1);
    let here = p.dlg.folder().unwrap();
    p.event(Event::NeedListing(here));
    p.event(Event::NeedListing(here));
    assert!(
        p.take_requests().is_empty(),
        "the pending read serves the relists"
    );
    for r in first {
        assert_eq!(p.reply(r.run(), &mut cache(), 1), Outcome::Redraw);
    }
    assert_eq!(names(&p), ["Pictures", "dunes"]);
    p.event(Event::NeedListing(here));
    assert_eq!(p.take_requests().len(), 1, "an answered read is read again");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_folder_due_again_is_read_again_and_another_is_left() {
    let dir = sandbox("due");
    let mut p = FilePicker::run_command(&dir, Icons::default(), true);
    settle(&mut p);
    assert!(!p.relist(&BTreeSet::from([dir.join("Pictures")])));
    assert!(p.take_requests().is_empty(), "another folder is left alone");
    assert!(p.relist(&BTreeSet::from([dir.clone()])));
    let again = p.take_requests();
    assert_eq!(again.len(), 1, "the folder shown is read again");
    for r in again {
        assert_eq!(p.reply(r.run(), &mut cache(), 1), Outcome::Redraw);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Calls [`FilePicker::watch_due`] as the loop does on each wake of the watch, running what
/// it queues, until `done` holds of the picker.
fn until_watched(p: &mut FilePicker, done: impl Fn(&FilePicker) -> bool) {
    for _ in 0..1000 {
        if p.watch_due() {
            settle(p);
        }
        if done(p) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("the picker never caught up with the folder");
}

#[test]
fn a_folder_made_in_the_folder_shown_lists_and_the_folder_follows_its_rename() {
    let dir = sandbox("watch");
    let mut p = FilePicker::run_command(&dir, Icons::default(), true);
    p.watch_with(reprisefs::folderwatch::Watch::new(|| {}).unwrap());
    settle(&mut p);
    assert!(p.dlg.select_named("dunes"));
    std::fs::create_dir(dir.join("made")).unwrap();
    until_watched(&mut p, |p| names(p).contains(&"made"));
    let picked: Vec<&str> = p.dlg.selected().iter().map(|e| e.name.as_str()).collect();
    assert_eq!(picked, ["dunes"], "the selection stays on its entry");

    let pictures = p.folders.id(&dir.join("Pictures"));
    p.event(Event::NeedListing(pictures));
    settle(&mut p);
    std::fs::rename(dir.join("Pictures"), dir.join("Photos")).unwrap();
    let inside = p.folders.id(&dir.join("Photos"));
    until_watched(&mut p, |p| p.dlg.folder() == Some(inside));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_file_dialog_keys_make_rename_and_relist_off_the_loop() {
    use reprisekit::textedit::keysym;
    let dir = sandbox("keys");
    let mut p = FilePicker::run_command(&dir, Icons::default(), true);
    settle(&mut p);
    assert_eq!(
        p.shortcut(R, keysym::F5, false, false, false),
        Some(Outcome::Redraw)
    );
    let again = p.take_requests();
    assert_eq!(again.len(), 1, "F5 reads the folder again");
    for r in again {
        p.reply(r.run(), &mut cache(), 1);
    }

    let n = 'N' as u32;
    assert_eq!(p.shortcut(R, n, true, true, false), Some(Outcome::Redraw));
    let ops = p.take_ops();
    assert_eq!(ops.len(), 1, "the folder is made off the loop");
    for op in ops {
        p.op_done(op.run());
    }
    settle(&mut p);
    assert!(dir.join("New Folder").is_dir());
    assert_eq!(
        p.dlg.focused_edit().map(|e| e.text.as_str()),
        Some("New Folder")
    );
    p.key(R, Key::Edit(EditKey::Insert("Art".into())));
    p.key(R, Key::Enter);
    for op in p.take_ops() {
        p.op_done(op.run());
    }
    settle(&mut p);
    assert!(dir.join("Art").is_dir(), "Enter renames it");
    let picked: Vec<&str> = p.dlg.selected().iter().map(|e| e.name.as_str()).collect();
    assert_eq!(picked, ["Art"]);
    for shift in [false, true] {
        assert_eq!(
            p.shortcut(R, keysym::DELETE, false, shift, false),
            Some(Outcome::AskDelete {
                paths: vec![dir.join("Art")],
                shift,
            }),
            "Delete asks as the desktop does"
        );
    }

    assert_eq!(
        p.shortcut(R, keysym::F2, false, false, false),
        Some(Outcome::Redraw)
    );
    p.key(R, Key::Edit(EditKey::Insert("/".into())));
    p.key(R, Key::Enter);
    let done: Vec<OpDone> = p.take_ops().into_iter().map(Op::run).collect();
    assert!(
        matches!(&done[..], [OpDone::Refused { why, .. }] if why.starts_with("A file name cannot")),
        "{done:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A subfolder's own icon, and an icon file it names, are read with the listing on the
/// job's thread: the reply wears them even when the folders and the file are gone by the
/// time the listing lands, so the loop never opens them (a dead mount would hang it).
#[test]
fn a_folders_own_icon_comes_with_the_listing() {
    let dir = sandbox("own-icon");
    let (folder, music, cover) = (0xff20_40c0, 0xffc0_4020, 0xff20_a040);
    let mut icons = flat_icons(
        &dir.join(".icons"),
        &[("folder", folder), ("folder-music", music)],
    );
    let art = dir.join("Art");
    let pic = dir.join("cover.png");
    image::RgbaImage::from_pixel(48, 48, image::Rgba([0x20, 0xa0, 0x40, 0xff]))
        .save(&pic)
        .unwrap();
    std::fs::create_dir(&art).unwrap();
    reprisefs::meta::set_folder_icon(&dir.join("Pictures"), Some("folder-music")).unwrap();
    reprisefs::meta::set_folder_icon(&art, pic.to_str()).unwrap();
    let mut p = FilePicker::run_command(&dir, Icons::default(), true);
    let replies: Vec<Reply> = p.take_requests().into_iter().map(Request::run).collect();
    std::fs::remove_dir_all(dir.join("Pictures")).unwrap();
    std::fs::remove_dir_all(&art).unwrap();
    std::fs::remove_file(&pic).unwrap();
    for r in replies {
        p.reply(r, &mut icons, 1);
    }
    let worn = |name: &str| {
        let e = p.dlg.entries().iter().find(|e| e.name == name).unwrap();
        [&e.icon, &e.icon32, &e.icon48].map(|c| c.as_ref().map(|c| c.get(8, 8)))
    };
    assert_eq!(
        worn("Pictures"),
        [Some(music); 3],
        "Pictures wears folder-music"
    );
    assert_eq!(worn("Art"), [Some(cover); 3], "Art wears its picture");
    let _ = std::fs::remove_dir_all(&dir);
}

/// At 125% and 150% text List and Details' icons are fetched at their grown column, 20 and
/// 25 px, theme icons and icon files alike; back at 100% they are 16 px again.
#[test]
fn list_icons_follow_the_text_size() {
    let dir = sandbox("icons-text");
    let mut icons = flat_icons(&dir.join(".icons"), &[("folder", 0xff20_40c0)]);
    let art = dir.join("Art");
    let pic = dir.join("cover.png");
    image::RgbaImage::from_pixel(48, 48, image::Rgba([0x20, 0xa0, 0x40, 0xff]))
        .save(&pic)
        .unwrap();
    std::fs::create_dir(&art).unwrap();
    reprisefs::meta::set_folder_icon(&art, pic.to_str()).unwrap();
    let opened = super::icons(&mut icons, 1, TextUnits::ONE);
    let mut p = FilePicker::run_command(&dir, opened, true);
    settle_with(&mut p, &mut icons);
    let widths = |p: &FilePicker, name: &str| {
        let e = p.dlg.entries().iter().find(|e| e.name == name).unwrap();
        [&e.icon, &e.icon32, &e.icon48].map(|c| c.as_ref().map(|c| c.width))
    };
    let at = |list: u32| [Some(list), Some(32), Some(48)];
    assert_eq!(widths(&p, "Pictures"), at(16));
    for (u, list) in [(TextUnits::new(14, 11), 20), (TextUnits::new(17, 11), 25)] {
        p.dlg.set_text(u);
        assert!(p.follow_scale(1, &mut icons));
        settle_with(&mut p, &mut icons);
        assert_eq!(
            widths(&p, "Pictures"),
            at(list),
            "a theme icon at {list} px"
        );
        assert_eq!(widths(&p, "Art"), at(list), "an icon file at {list} px");
    }
    p.dlg.set_text(TextUnits::ONE);
    assert!(p.follow_scale(1, &mut icons));
    settle_with(&mut p, &mut icons);
    assert_eq!(widths(&p, "Art"), at(16));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn icons_take_the_dialogs_draw_scale_and_follow_a_new_one() {
    let dir = sandbox("icons-2x");
    let mut icons = flat_icons(&dir.join(".icons"), &[("folder", 0xff20_40c0)]);
    let art = dir.join("Art");
    let pic = dir.join("cover.png");
    image::RgbaImage::from_pixel(48, 48, image::Rgba([0x20, 0xa0, 0x40, 0xff]))
        .save(&pic)
        .unwrap();
    std::fs::create_dir(&art).unwrap();
    reprisefs::meta::set_folder_icon(&art, pic.to_str()).unwrap();
    let opened = super::icons(&mut icons, 2, TextUnits::ONE);
    let size = |c: &Option<Canvas>| c.as_ref().map(|c| (c.width, c.scale()));
    assert_eq!(
        size(&opened.folder),
        Some((32, 1)),
        "device pixels on a scale-1 canvas"
    );
    let mut p = FilePicker::run_command(&dir, opened, true);
    settle_with(&mut p, &mut icons);
    let widths = |p: &FilePicker, name: &str| {
        let e = p.dlg.entries().iter().find(|e| e.name == name).unwrap();
        [&e.icon, &e.icon32, &e.icon48].map(|c| c.as_ref().map(|c| c.width))
    };
    let at = |k: u32| [Some(16 * k), Some(32 * k), Some(48 * k)];
    assert_eq!(widths(&p, "Pictures"), at(2));
    assert_eq!(
        widths(&p, "Art"),
        at(2),
        "an icon file decoded off the loop"
    );

    assert!(p.follow_scale(1, &mut icons));
    assert_eq!(size(&p.icons.folder), Some((16, 1)));
    assert_eq!(widths(&p, "Art"), at(2), "until the relisting lands");
    assert_eq!(settle_with(&mut p, &mut icons), Outcome::Redraw);
    assert_eq!(widths(&p, "Pictures"), at(1));
    assert_eq!(widths(&p, "Art"), at(1));
    assert!(!p.follow_scale(1, &mut icons));
    assert!(p.take_requests().is_empty(), "nothing to relist");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The Look in folder comes at 16 px at 100% and 32 px at 200%, at the scale times that.
#[test]
fn the_look_in_folder_grows_with_the_text() {
    let dir = sandbox("lookin-grown");
    let mut cache = flat_icons(&dir.join(".icons"), &[("folder", 0xff20_40c0)]);
    let two = TextUnits::new(22, 11);
    let width = |scale: i32, u: TextUnits, cache: &mut IconCache| {
        icons(cache, scale, u).folder.map(|c| c.width)
    };
    assert_eq!(width(1, TextUnits::ONE, &mut cache), Some(16));
    assert_eq!(width(1, two, &mut cache), Some(32));
    assert_eq!(width(2, two, &mut cache), Some(64));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A new icon theme gives the Look in folder and the listed entries its icons, the entries
/// keeping their places while their icons arrive.
#[test]
fn a_new_icon_theme_gives_the_dialog_and_its_entries_its_icons() {
    let dir = sandbox("icons-theme");
    std::fs::create_dir(dir.join("Sub")).unwrap();
    let (blue, green) = (0xff20_40c0, 0xff20_a040);
    let mut flat = flat_icons(&dir.join(".icons"), &[("folder", blue)]);
    let mut p = FilePicker::run_command(&dir, icons(&mut flat, 1, TextUnits::ONE), true);
    settle_with(&mut p, &mut flat);
    let worn = |p: &FilePicker| {
        let e = p.dlg.entries().iter().find(|e| e.name == "Sub").unwrap();
        [&e.icon, &e.icon32, &e.icon48].map(|c| c.as_ref().map(|c| c.get(4, 4)))
    };
    let folder = |p: &FilePicker| p.icons.folder.as_ref().map(|c| c.get(4, 4));
    assert_eq!(worn(&p), [Some(blue); 3]);
    assert_eq!(folder(&p), Some(blue));
    assert!(!p.follow_scale(1, &mut flat), "the same theme");

    let mut other = flat_icons(&dir.join(".other"), &[("folder", green)]);
    assert!(p.follow_scale(1, &mut other));
    assert_eq!(folder(&p), Some(green));
    assert_eq!(worn(&p), [Some(blue); 3], "until the icons arrive");
    assert_eq!(settle_with(&mut p, &mut other), Outcome::Redraw);
    assert_eq!(worn(&p), [Some(green); 3]);
    assert!(!p.follow_scale(1, &mut other));
    let _ = std::fs::remove_dir_all(&dir);
}

/// An icon theme drawing `names` in their flat colours at 16, 32 and 48 px under `root`.
fn flat_icons(root: &Path, names: &[(&str, u32)]) -> IconCache {
    let theme = root.join("flat");
    let dirs = ["16x16/apps", "32x32/apps", "48x48/apps"];
    let mut index = format!("[Icon Theme]\nName=Flat\nDirectories={}\n", dirs.join(","));
    for (d, px) in dirs.iter().zip([16, 32, 48]) {
        index += &format!("\n[{d}]\nSize={px}\nType=Fixed\n");
        std::fs::create_dir_all(theme.join(d)).unwrap();
        for (name, argb) in names {
            let [a, r, g, b] = argb.to_be_bytes();
            image::RgbaImage::from_pixel(px, px, image::Rgba([r, g, b, a]))
                .save(theme.join(d).join(format!("{name}.png")))
                .unwrap();
        }
    }
    std::fs::write(theme.join("index.theme"), index).unwrap();
    IconCache::new(reprisekit::icontheme::IconTheme::load(
        "flat",
        &[root.to_path_buf()],
    ))
}

/// Entries carry their icon at all three sizes and Tiles' lines: a program wears the
/// executable's icon, any other file its type's.
#[test]
fn entries_wear_their_icons_at_every_size_and_tiles_lines() {
    let dir = sandbox("tiles");
    let (folder, program, text) = (0xff20_40c0, 0xffc0_4020, 0xff20_a040);
    let mut icons = flat_icons(
        &dir.join(".icons"),
        &[
            ("folder", folder),
            ("application-x-executable", program),
            ("text-x-generic", text),
        ],
    );
    let mut p = FilePicker::run_command(&dir, Icons::default(), true);
    p.dlg.set_filter(1);
    p.sync_filter();
    settle_with(&mut p, &mut icons);
    let entry = |name: &str| {
        p.dlg
            .entries()
            .iter()
            .find(|e| e.name == name)
            .cloned()
            .expect(name)
    };
    let worn = |e: &Entry| {
        [&e.icon, &e.icon32, &e.icon48].map(|c| c.as_ref().map(|c| (c.width, c.get(8, 8))))
    };
    let sizes = |argb| [Some((16, argb)), Some((32, argb)), Some((48, argb))];
    let pictures = entry("Pictures");
    assert_eq!(worn(&pictures), sizes(folder));
    assert!(pictures.detail.is_empty(), "a folder's tile names it alone");
    let dunes = entry("dunes");
    assert_eq!(worn(&dunes), sizes(program));
    assert_eq!(dunes.detail, [dunes.kind.clone(), "1 KB".into()]);
    let notes = entry("notes.txt");
    assert_eq!(worn(&notes), sizes(text));
    assert_eq!(notes.detail, [notes.kind.clone(), "1 KB".into()]);
    let _ = std::fs::remove_dir_all(&dir);
}

fn fnv(c: &Canvas, r: Rect) -> u64 {
    let px = (r.y..r.bottom()).flat_map(|y| (r.x..r.right()).map(move |x| c.get(x, y)));
    px.fold(0xcbf2_9ce4_8422_2325, |h, p| {
        (h ^ u64::from(p)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// Where `argb` lands in `c`: its bounding box.
fn found(c: &Canvas, argb: u32) -> Option<Rect> {
    let mut hit: Option<(i32, i32, i32, i32)> = None;
    for y in 0..c.height as i32 {
        for x in 0..c.width as i32 {
            if c.get(x, y) == argb {
                let b = hit.get_or_insert((x, y, x, y));
                *b = (b.0.min(x), b.1.min(y), b.2.max(x), b.3.max(y));
            }
        }
    }
    hit.map(|(l, t, r, b)| Rect::new(l, t, r - l + 1, b - t + 1))
}

/// At scale 2 the pictures are fetched for the frame's 188 device pixels and fill it there.
#[test]
fn a_scale_2_listing_draws_its_thumbnails_at_the_frames_device_size() {
    const SAND: u32 = 0xffd2_b48c;
    let dir = sandbox("thumbs-2x");
    let pic = dir.join("dunes.png");
    std::fs::write(&pic, b"never decoded here").unwrap();
    let mut p = FilePicker::run_command(&dir, Icons::default(), true);
    p.dlg.set_filter(1);
    p.sync_filter();
    for r in p.take_requests() {
        p.reply(r.run(), &mut cache(), 2);
    }
    p.click(R, Some(filedlg::Hit::ToolBar(3)));
    p.click(R, Some(filedlg::Hit::ViewsItem(0)));
    let (generation, px, _) = p.take_pictures().expect("Thumbnails fetches the pictures");
    assert_eq!(px, 188, "the frame's inside at scale 2");
    let large = Canvas::from_pixels(256, 128, vec![SAND; 256 * 128]);
    assert!(p.thumbs(generation, vec![(pic.clone(), large, Some((800, 400)))]));

    let theme = reprisekit::theme::Theme::default().scaled(2);
    let text = reprisekit::text::TextRenderer::load(&theme.fonts);
    let mut c = Canvas::new(R.width as u32 * 2, R.height as u32 * 2);
    p.dlg
        .draw(&mut c, &text, &theme.paint(), R, theme.fonts.size_px);
    let drawn = found(&c, SAND).expect("the thumbnail is drawn");
    assert_eq!((drawn.width, drawn.height), (188, 94));
    let frame = Rect::new(drawn.x - 2, drawn.y - 49, 192, 192);
    assert_eq!(c.get(frame.x, frame.y), theme.colors.button_face.argb());
    assert_eq!(fnv(&c, frame), 0x262c_b03d_ad0d_133d, "the scale-2 frame");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_new_draw_scale_fetches_the_pictures_again_at_its_size() {
    let dir = sandbox("thumbs-rescale");
    let pic = dir.join("dunes.png");
    std::fs::write(&pic, b"never decoded here").unwrap();
    let mut cache = cache();
    let mut p = FilePicker::run_command(&dir, icons(&mut cache, 1, TextUnits::ONE), true);
    p.dlg.set_filter(1);
    p.sync_filter();
    settle(&mut p);
    p.click(R, Some(filedlg::Hit::ToolBar(3)));
    p.click(R, Some(filedlg::Hit::ViewsItem(0)));
    let (generation, px, _) = p.take_pictures().expect("Thumbnails fetches the pictures");
    assert_eq!(px, 94);
    let thumb = |w| vec![(pic.clone(), Canvas::new(w, w / 2), Some((800, 400)))];
    assert!(p.thumbs(generation, thumb(94)));
    assert!(!p.follow_scale(1, &mut cache), "the scale it fetched at");

    assert!(p.follow_scale(2, &mut cache));
    let shown = |p: &FilePicker| p.dlg.entries().iter().any(|e| e.thumb.is_some());
    assert!(!shown(&p), "a scale-1 thumbnail would draw at half size");
    let (again, px, paths) = p.take_pictures().expect("fetched again");
    assert_eq!((px, paths), (188, vec![pic.clone()]));
    assert!(!p.thumbs(generation, thumb(94)), "fetched at the old scale");
    assert!(p.thumbs(again, thumb(188)));
    let e = p.dlg.entries().iter().find(|e| e.thumb.is_some()).unwrap();
    assert_eq!(e.thumb.as_ref().unwrap().device_size(), (188, 94));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A thumbnail without a picture size (a document's page) fills its frame at its own shape
/// and leaves the Tiles lines as they were.
#[test]
fn a_thumbnail_without_a_picture_size_keeps_the_tiles_lines() {
    let dir = sandbox("thumbs-unsized");
    let pic = dir.join("dunes.png");
    std::fs::write(&pic, b"never decoded here").unwrap();
    let mut p = FilePicker::run_command(&dir, Icons::default(), true);
    p.dlg.set_filter(1);
    p.sync_filter();
    settle(&mut p);
    p.click(R, Some(filedlg::Hit::ToolBar(3)));
    p.click(R, Some(filedlg::Hit::ViewsItem(0)));
    let (generation, ..) = p.take_pictures().expect("Thumbnails fetches the files");
    let entry = |p: &FilePicker| {
        (p.dlg.entries().iter())
            .find(|e| e.name == "dunes.png")
            .cloned()
            .unwrap()
    };
    let before = entry(&p).detail;
    let page = Canvas::new(64, 94);
    assert!(p.thumbs(generation, vec![(pic.clone(), page, None)]));
    let e = entry(&p);
    let t = e.thumb.as_ref().expect("the thumbnail");
    assert_eq!((t.width, t.height), (64, 94));
    assert_eq!(e.detail, before);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Thumbnails asks for the listing's pictures once; their thumbnails fill the frames, at the
/// size that fits there, and give the picture's size to Tiles. A relisted folder drops the
/// earlier listing's.
#[test]
fn thumbnails_fetch_the_pictures_once_and_show_their_thumbnails() {
    const SAND: u32 = 0xffd2_b48c;
    let dir = sandbox("thumbs");
    let pic = dir.join("dunes.png");
    std::fs::write(&pic, b"never decoded here").unwrap();
    let mut p = FilePicker::run_command(&dir, Icons::default(), true);
    p.dlg.set_filter(1);
    p.sync_filter();
    settle(&mut p);
    assert_eq!(p.take_pictures(), None, "List fetches nothing");
    p.click(R, Some(filedlg::Hit::ToolBar(3)));
    p.click(R, Some(filedlg::Hit::ViewsItem(0)));
    assert_eq!(p.dlg.view(), Mode::Thumbnails);
    let (generation, px, paths) = p.take_pictures().expect("Thumbnails fetches the pictures");
    assert_eq!(paths, std::slice::from_ref(&pic));
    assert_eq!(px, 94, "the frame's inside at scale 1");
    assert_eq!(p.take_pictures(), None, "once per listing");
    let thumb = || {
        vec![(
            pic.clone(),
            Canvas::from_pixels(128, 64, vec![SAND; 128 * 64]),
            Some((800, 400)),
        )]
    };
    assert!(!p.thumbs(generation + 1, thumb()), "not this listing's");
    assert!(p.thumbs(generation, thumb()));
    let e = p
        .dlg
        .entries()
        .iter()
        .find(|e| e.name == "dunes.png")
        .unwrap();
    let t = e.thumb.as_ref().expect("the thumbnail");
    assert_eq!((t.width, t.height), (94, 47));
    assert_eq!(e.detail, ["800 x 400".to_string(), e.kind.clone()]);

    let theme = reprisekit::theme::Theme::default();
    let text = reprisekit::text::TextRenderer::load(&theme.fonts);
    let mut c = Canvas::new(R.width as u32, R.height as u32);
    p.dlg
        .draw(&mut c, &text, &theme.paint(), R, theme.fonts.size_px);
    let drawn = found(&c, SAND).expect("the thumbnail is drawn");
    assert_eq!((drawn.width, drawn.height), (94, 47));
    // The picture's cell: its frame and name. The Look in and places labels name the
    // test's folder and user.
    let cell = Rect::new(drawn.x - 9, drawn.y - 33, 112, 136);
    reprisekit::pins::fonts();
    assert_eq!(fnv(&c, cell), 0xdb3b_8c32_df81_6bd0, "Browse in Thumbnails");

    p.event(Event::NeedListing(p.dlg.folder().unwrap()));
    settle(&mut p);
    let (relisted, ..) = p.take_pictures().expect("the new listing's pictures");
    assert!(relisted > generation);
    assert!(!p.thumbs(generation, thumb()), "the earlier listing's");
    let _ = std::fs::remove_dir_all(&dir);
}
