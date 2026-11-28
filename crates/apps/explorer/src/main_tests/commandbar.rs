use super::*;
use crate::chrome::{BarFacts, BarMenu, BarPlace, Picked, commands_for};
use crate::commands::LayoutPart;
use reprisekit::controls::commandbar::{Drop, Hit};
use reprisekit::controls::menu::MenuItem;
use reprisekit::raster::Rect;

const BAR: Rect = Rect::new(0, 60, 792, 31);

fn labels(ex: &Explorer) -> Vec<&str> {
    ex.cmdbar.left.iter().map(|i| i.label.as_str()).collect()
}

fn facts(place: BarPlace, picked: Picked) -> BarFacts {
    BarFacts {
        place,
        picked,
        can_connect: true,
    }
}

fn runs(f: &BarFacts) -> Vec<(String, Command)> {
    commands_for(f)
        .into_iter()
        .map(|(item, act)| (item.label, act.run))
        .collect()
}

fn rows(p: &crate::window::Popup) -> Vec<String> {
    p.tree()
        .items()
        .iter()
        .filter_map(|it| match it {
            MenuItem::Entry { label, .. } => Some(label.replace('&', "")),
            _ => None,
        })
        .collect()
}

#[test]
fn nothing_selected_in_a_writable_folder_offers_new_folder() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.refresh_chrome(&mut ctx);
    assert_eq!(labels(&ex), ["Organize", "New folder"]);
    assert_eq!(ex.cmdbar.left[0].drop, Drop::Menu, "Organize opens a menu");
    assert_eq!(ex.cmdbar.left[1].drop, Drop::Split);
    assert_eq!(ex.cmdbar_acts[1].run, Command::NewFolder);

    ex.command_bar_press(Hit::ItemDrop(1), BAR, false, &mut ctx);
    let popup = ex.popup.as_ref().expect("New folder's drop opens");
    assert_eq!(popup.from_bar, Some(Hit::ItemDrop(1)));
    assert_eq!(
        rows(popup)[..3],
        ["Folder", "Shortcut...", "Text Document"],
        "the File menu's New rows"
    );
    assert_eq!(
        ex.cmdbar_shown().pressed,
        Some(Hit::ItemDrop(1)),
        "held while open"
    );
    ex.popup = None;
    assert_eq!(ex.cmdbar_shown().pressed, None);

    let read_only = facts(BarPlace::Network, Picked::Nothing);
    assert!(
        !runs(&read_only)
            .iter()
            .any(|(_, c)| *c == Command::NewFolder),
        "no New folder where nothing is written"
    );
}

#[test]
fn a_file_selected_offers_open_with_its_drop() {
    let mut ctx = test_ctx();
    let (_dir, mut ex) = browse_fixture(&mut ctx);
    ex.refresh_chrome(&mut ctx);
    assert_eq!(labels(&ex), ["Organize", "Open", "New folder"]);
    assert_eq!(ex.cmdbar.left[1].drop, Drop::Split);
    assert_eq!(ex.cmdbar_acts[1].run, Command::Open);
    assert_eq!(ex.cmdbar_acts[1].menu, Some(BarMenu::OpenWith));

    ex.command_bar_press(Hit::ItemDrop(1), BAR, false, &mut ctx);
    let popup = ex.popup.as_ref().expect("Open's drop opens");
    assert_eq!(
        rows(popup).last().map(String::as_str),
        Some("Choose Program...")
    );
    let l = ex.cmdbar_layout(BAR, &ctx);
    let drop = l.items[1].drop.expect("a drop part");
    assert_eq!(
        popup.origin(),
        (drop.x, drop.bottom()),
        "under the drop part"
    );

    let sub = ex
        .items
        .iter()
        .position(|it| it.name == "sub")
        .expect("sub");
    ex.sv.selection.set_only(Some(sub));
    ex.refresh_chrome(&mut ctx);
    assert_eq!(labels(&ex), ["Organize", "Open", "New folder"]);
    assert_eq!(
        ex.cmdbar.left[1].drop,
        Drop::None,
        "a folder has no Open with"
    );

    let archive = facts(
        BarPlace::Folder,
        Picked::File {
            image: true,
            archive: true,
        },
    );
    let names: Vec<String> = runs(&archive).into_iter().map(|r| r.0).collect();
    assert_eq!(
        names,
        [
            "Organize",
            "Open",
            "Mount",
            "Extract all files",
            "New folder"
        ]
    );
}

#[test]
fn a_removable_drive_offers_eject() {
    let drive = |eject| runs(&facts(BarPlace::Computer, Picked::Drive { eject }));
    assert_eq!(
        drive(Some(false))[1..3],
        [
            ("Properties".to_string(), Command::Properties),
            ("Eject".to_string(), Command::Eject)
        ]
    );
    assert_eq!(drive(Some(true))[2].0, "Disconnect", "a network share");
    assert!(
        !drive(None).iter().any(|(_, c)| *c == Command::Eject),
        "a fixed disk has no Eject"
    );
}

#[test]
fn computer_offers_system_properties() {
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Computer, &mut ctx);
    ex.sv.selection.set_only(None);
    ex.refresh_chrome(&mut ctx);
    let acts: Vec<&Command> = ex.cmdbar_acts.iter().map(|a| &a.run).collect();
    assert!(acts.contains(&&Command::SystemInformation));
    assert!(acts.contains(&&Command::Settings), "Open Settings");
    assert!(!acts.contains(&&Command::NewFolder));
    let no_gio = BarFacts {
        can_connect: false,
        ..facts(BarPlace::Computer, Picked::Nothing)
    };
    assert_eq!(
        runs(&no_gio).into_iter().map(|r| r.0).collect::<Vec<_>>(),
        ["Organize", "System properties", "Open Settings"],
        "Connect to server only where gio connects"
    );
}

#[test]
fn the_recycle_bin_offers_empty_and_restore_and_an_archive_extract_all() {
    let bin = |n, picked| runs(&facts(BarPlace::RecycleBin(n), picked));
    assert_eq!(bin(3, Picked::Nothing)[1].1, Command::EmptyRecycleBin);
    assert_eq!(
        bin(0, Picked::Nothing).len(),
        1,
        "an empty bin: Organize alone"
    );
    assert_eq!(
        bin(3, Picked::Several { archives: false })[1].1,
        Command::Restore
    );
    assert_eq!(
        runs(&facts(BarPlace::Archive, Picked::Nothing))[1].1,
        Command::ExtractAll
    );
}

#[test]
fn organize_layout_toggles_menu_bar_details_and_navigation_panes() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.refresh_chrome(&mut ctx);
    let blue = (ex.settings.folders, ex.settings.status_bar);
    let open = |ex: &mut Explorer, ctx: &mut Ctx| {
        ex.command_bar_press(Hit::Item(0), BAR, false, ctx);
        let popup = ex.popup.take().expect("Organize opens");
        assert_eq!(popup.from_bar, Some(Hit::Item(0)));
        let layout = rows(&popup)
            .iter()
            .position(|r| r == "Layout")
            .expect("a Layout row");
        let row = popup
            .tree()
            .items()
            .iter()
            .enumerate()
            .filter(|(_, it)| matches!(it, MenuItem::Entry { .. }))
            .nth(layout)
            .map(|(i, _)| i)
            .unwrap();
        let checks: Vec<bool> = (popup.tree().child(row).unwrap().items().iter())
            .map(|it| matches!(it, MenuItem::Entry { checked: true, .. }))
            .collect();
        (popup, row, checks)
    };
    let (popup, row, checks) = open(&mut ex, &mut ctx);
    assert_eq!(
        checks,
        [false, true, true],
        "menu bar hidden, both panes shown"
    );
    for (i, part) in [
        LayoutPart::MenuBar,
        LayoutPart::DetailsPane,
        LayoutPart::NavPane,
    ]
    .into_iter()
    .enumerate()
    {
        let cmd = popup.command(&[row, i]);
        assert_eq!(cmd, Command::ToggleLayout(part));
        ex.run(cmd, &mut ctx);
    }
    let s = &ex.settings;
    assert_eq!(
        (s.menu_bar, s.details_pane, s.nav_pane),
        (true, false, false)
    );
    assert_eq!(
        (s.folders, s.status_bar),
        blue,
        "Blue's keys stay as they were"
    );
    assert!(ex.settings_dirty, "the toggles are saved");
    let (_, _, checks) = open(&mut ex, &mut ctx);
    assert_eq!(checks, [true, false, false]);
}

#[test]
fn views_click_cycles_and_its_arrow_opens_the_list_and_help_does_nothing() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.refresh_chrome(&mut ctx);
    let before = ex.sv.mode;
    ex.command_bar_press(Hit::ViewsCycle, BAR, false, &mut ctx);
    assert_eq!(ex.sv.mode, crate::commands::next_mode(before));
    assert!(ex.popup.is_none());
    ex.command_bar_press(Hit::ViewsDrop, BAR, false, &mut ctx);
    let popup = ex.popup.take().expect("the modes' list");
    assert_eq!(rows(&popup).len(), 5);
    assert_eq!(popup.from_bar, Some(Hit::ViewsDrop));
    assert_eq!(
        ex.command_bar_press(Hit::Help, BAR, false, &mut ctx),
        Reply::none()
    );
    assert!(ex.popup.is_none(), "help opens nothing");
}

#[test]
fn the_more_menu_holds_the_folded_commands() {
    let mut ctx = test_ctx();
    let (_dir, mut ex) = browse_fixture(&mut ctx);
    ex.refresh_chrome(&mut ctx);
    let narrow = Rect::new(0, 60, 230, 31);
    let l = ex.cmdbar_layout(narrow, &ctx);
    let folded = ex.cmdbar.folded(&l);
    assert!(!folded.is_empty(), "a narrow bar folds commands");
    ex.command_bar_press(Hit::More, narrow, false, &mut ctx);
    let popup = ex.popup.as_ref().expect("» opens");
    let want: Vec<String> = ex.cmdbar.left[folded.clone()]
        .iter()
        .map(|i| i.label.clone())
        .collect();
    assert_eq!(rows(popup), want);
    let last = folded.len() - 1;
    assert_eq!(
        popup.command(&[last]),
        Command::NewFolder,
        "a folded command runs"
    );
}
