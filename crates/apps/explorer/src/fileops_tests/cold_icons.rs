//! The boxes, menus, sheet picks and preview on a theme of cold SVG icons: each wears blank
//! stand-ins, then its icons once they land off the loop, and a name that decodes at no size
//! gives way to the next.

use super::*;
use crate::live::tests::{land_until, mid, svg_theme};
use reprisekit::controls::filesheet;
use reprisekit::controls::form::{Control, Form};
use reprisekit::controls::menu::MenuItem;

const DRAWN: u32 = 0xff20_40c0;
const OTHER: u32 = 0xff40_c020;
const THIRD: u32 = 0xffc0_4020;

fn picture(form: &Form, id: &str) -> Option<u32> {
    match form.item(id).map(|i| &i.control) {
        Some(Control::Picture { canvas, .. }) => mid(canvas),
        _ => None,
    }
}

fn confirm_icon(ex: &Explorer) -> Option<u32> {
    match &ex.dialog {
        Some(Dialog::Confirm(c)) => mid(&c.icon.icon()),
        _ => None,
    }
}

fn explorer_in(dir: &Path, ctx: &mut repriseapp::Ctx) -> Explorer {
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.to_path_buf()), ctx);
    ex.globs = std::sync::Arc::new(reprisefs::apps::MimeDb::empty());
    ex
}

fn app_with_icon(name: &str, id: &str, icon: &str) -> AppEntry {
    AppEntry {
        icon: Some(icon.into()),
        ..app(name, id, &["text/plain"])
    }
}

/// The error box, About and Advanced Attributes wear stand-ins, then their icons; the error
/// box's first name decodes at no size and gives way to its second.
#[test]
fn a_boxs_cold_icon_lands_and_a_broken_name_gives_way() {
    let (art, dir) = (tempfile::tempdir(), tempfile::tempdir());
    let (art, dir) = (art.expect("a temp dir"), dir.expect("a temp dir"));
    let mut ctx = test_ctx();
    let drawn = [
        ("dialog-warning", DRAWN),
        ("system-file-manager", OTHER),
        ("document-properties", THIRD),
    ];
    ctx.icons = svg_theme(art.path(), &drawn, &["dialog-error"]);
    let mut ex = explorer_in(dir.path(), &mut ctx);

    ex.message_box("Explorer", "Cannot.".into(), &mut ctx);
    assert_eq!(mid(&ex.error_icon()), Some(0), "the error box's stand-in");
    land_until(&mut ex, &mut ctx, |ex| mid(&ex.error_icon()) == Some(DRAWN));
    assert_eq!(mid(&ex.error_icon()), Some(DRAWN));

    ex.dialog = None;
    ex.about_box(&mut ctx);
    let about = |ex: &Explorer| match &ex.dialog {
        Some(Dialog::About(form, _)) => picture(form, "icon"),
        _ => None,
    };
    assert_eq!(about(&ex), Some(0), "About's stand-in");
    land_until(&mut ex, &mut ctx, |ex| about(ex) == Some(OTHER));
    assert_eq!(about(&ex), Some(OTHER));

    ex.dialog = None;
    ex.advanced_attributes(filesheet::Subject::File, None, &mut ctx);
    let advanced = |ex: &Explorer| match &ex.dialog {
        Some(Dialog::Advanced { form, .. }) => picture(form, "icon"),
        _ => None,
    };
    assert_eq!(advanced(&ex), Some(0), "Advanced Attributes' stand-in");
    land_until(&mut ex, &mut ctx, |ex| advanced(ex) == Some(THIRD));
    assert_eq!(advanced(&ex), Some(THIRD));
}

/// Shift+Delete's prompt asks for the entry's own icon at 32 px while it decodes; here that name
/// decodes at no size, and the prompt wears the Recycle Bin's once it lands.
#[test]
fn the_delete_prompt_wears_a_stand_in_and_a_broken_entry_icon_gives_way() {
    let _guard = lock(&TRASH_ENV);
    let (listed, art, dir) = (
        tempfile::tempdir(),
        tempfile::tempdir(),
        tempfile::tempdir(),
    );
    let (listed, art) = (listed.expect("a temp dir"), art.expect("a temp dir"));
    let dir = dir.expect("a temp dir");
    std::fs::write(dir.path().join("notes.txt"), "x").expect("write");
    let mut ctx = test_ctx();
    ctx.icons = svg_theme(listed.path(), &[("text-x-generic", OTHER)], &[]);
    let mut ex = explorer_in(dir.path(), &mut ctx);
    land_until(&mut ex, &mut ctx, |ex| {
        let icon = ex
            .sv
            .entries
            .first()
            .and_then(|e| e.icon.as_deref().cloned());
        mid(&icon) == Some(OTHER)
    });
    ctx.icons = svg_theme(
        art.path(),
        &[("user-trash-full", DRAWN)],
        &["text-x-generic"],
    );

    ex.sv.selection.set_only(Some(0));
    ex.run(Command::Delete { erase: true }, &mut ctx);
    crate::tests::wake_until(&mut ex, &mut ctx, |ex| ex.dialog.is_some());
    assert_eq!(confirm_icon(&ex), Some(0), "the prompt's stand-in");
    land_until(&mut ex, &mut ctx, |ex| confirm_icon(ex) == Some(DRAWN));
    assert_eq!(confirm_icon(&ex), Some(DRAWN));
}

/// The replace prompt's folder and its two files wear stand-ins, then their icons; the files'
/// first pick decodes at no size and gives way to their extension's name.
#[test]
fn the_replace_prompt_wears_its_cold_icons_once_they_land() {
    let (art, t) = (tempfile::tempdir(), tempfile::tempdir());
    let (art, t) = (art.expect("a temp dir"), t.expect("a temp dir"));
    let (src, dest) = (t.path().join("src"), t.path().join("dest"));
    for (d, text) in [(&src, "x"), (&dest, "old")] {
        std::fs::create_dir_all(d).expect("mkdir");
        std::fs::write(d.join("a.pdf"), text).expect("write");
    }
    let mut ctx = test_ctx();
    let drawn = [("folder", DRAWN), ("application-pdf", OTHER)];
    ctx.icons = svg_theme(art.path(), &drawn, &["text-x-generic"]);
    let mut ex = explorer_in(&dest, &mut ctx);
    let copy = |_: &Path| Some(DropOp::Copy);
    ex.put_all(&[src.join("a.pdf")], &dest, &copy, false, &mut ctx);
    crate::tests::wake_until(&mut ex, &mut ctx, |ex| ex.dialog.is_some());
    let worn = |ex: &Explorer| match &ex.dialog {
        Some(Dialog::Replace(r)) => match &r.fields.subject {
            replacebox::Subject::File { existing, incoming } => [
                mid(&r.fields.icon),
                mid(&existing.icon),
                mid(&incoming.icon),
            ],
            replacebox::Subject::Folder => panic!("a file clash shows both files"),
        },
        _ => [None; 3],
    };
    assert_eq!(worn(&ex), [Some(0); 3], "stand-ins");
    let want = [Some(DRAWN), Some(OTHER), Some(OTHER)];
    land_until(&mut ex, &mut ctx, |ex| worn(ex) == want);
    assert_eq!(worn(&ex), want);
}

/// The Open With chooser's icon and its programs' wear stand-ins, then their icons; a program
/// whose icon decodes at no size wears none.
#[test]
fn the_chooser_wears_its_cold_icons_once_they_land() {
    let (art, dir) = (tempfile::tempdir(), tempfile::tempdir());
    let (art, dir) = (art.expect("a temp dir"), dir.expect("a temp dir"));
    let path = dir.path().join("a.txt");
    std::fs::write(&path, "1").expect("write");
    let mut ctx = test_ctx();
    let drawn = [("system-run", DRAWN), ("editor-icon", OTHER)];
    ctx.icons = svg_theme(art.path(), &drawn, &["viewer-icon"]);
    let mut ex = explorer_in(dir.path(), &mut ctx);
    ex.app_index = vec![
        app_with_icon("Editor", "editor", "editor-icon"),
        app_with_icon("Viewer", "viewer", "viewer-icon"),
    ]
    .into();
    ex.choose_program_for(&path, false, &mut ctx);
    crate::tests::wake_until(&mut ex, &mut ctx, |ex| ex.dialog.is_some());
    let worn = |ex: &Explorer| match &ex.dialog {
        Some(Dialog::ChooseProgram(c)) => {
            let rows = (c.ow.list.rows.iter()).zip(&c.row_icons);
            let named = rows.filter_map(|(r, i)| {
                let name = reprisekit::controls::openwith::icon_name(r)?;
                name.ends_with("-icon").then(|| (name.to_string(), mid(i)))
            });
            let mut named: Vec<_> = named.collect();
            named.sort();
            (mid(&c.icon), named)
        }
        _ => (None, Vec::new()),
    };
    let named = |editor, viewer| {
        vec![
            ("editor-icon".to_string(), editor),
            ("viewer-icon".to_string(), viewer),
        ]
    };
    assert_eq!(worn(&ex), (Some(0), named(Some(0), Some(0))), "stand-ins");
    let want = (Some(DRAWN), named(Some(OTHER), None));
    land_until(&mut ex, &mut ctx, |ex| worn(ex) == want);
    assert_eq!(worn(&ex), want);
}

/// The item menu's Open With and Send To rows wear stand-ins, then their icons; a row whose icon
/// decodes at no size wears none.
#[test]
fn the_item_menus_rows_wear_their_cold_icons_once_they_land() {
    let (art, dir) = (tempfile::tempdir(), tempfile::tempdir());
    let (art, dir) = (art.expect("a temp dir"), dir.expect("a temp dir"));
    std::fs::write(dir.path().join("a.txt"), "1").expect("write");
    let mut ctx = test_ctx();
    let drawn = [("editor-icon", DRAWN), ("application-zip", OTHER)];
    ctx.icons = svg_theme(art.path(), &drawn, &["viewer-icon", "internet-mail"]);
    let mut ex = explorer_in(dir.path(), &mut ctx);
    ex.app_index = vec![
        app_with_icon("Editor", "editor", "editor-icon"),
        app_with_icon("Viewer", "viewer", "viewer-icon"),
    ]
    .into();
    crate::tests::wake_until(&mut ex, &mut ctx, |ex| !ex.items.is_empty());
    for it in &mut ex.items {
        it.mime = Some("text/plain".into());
    }
    ex.entry_menu_at(0, (10, 10), &mut ctx, false);
    let worn = |ex: &Explorer| -> Vec<(String, Option<u32>)> {
        let t = ex.popup.as_ref().expect("the menu is up").tree();
        let subs = (0..t.items().len()).filter_map(|r| t.child(r));
        (subs.flat_map(|sub| sub.items().iter()))
            .filter_map(|i| match i {
                MenuItem::Entry { label, icon, .. } if ROWS.contains(&label.as_str()) => {
                    Some((label.clone(), mid(icon)))
                }
                _ => None,
            })
            .collect()
    };
    let rows = |icons: [Option<u32>; 4]| -> Vec<(String, Option<u32>)> {
        ROWS.iter().map(|l| l.to_string()).zip(icons).collect()
    };
    assert_eq!(worn(&ex), rows([Some(0); 4]), "stand-ins");
    let want = rows([Some(DRAWN), None, Some(OTHER), None]);
    land_until(&mut ex, &mut ctx, |ex| worn(ex) == want);
    assert_eq!(worn(&ex), want);
}

/// The Open With and Send To rows [`the_item_menus_rows_wear_their_cold_icons_once_they_land`]
/// looks at, in the menu's order.
const ROWS: [&str; 4] = [
    "Editor",
    "Viewer",
    "Compressed (zipped) Folder",
    "Mail Recipient",
];

/// Change Icon's pick wears a stand-in on Customize's picture and its 96 px preview, then the
/// icon; a pick that decodes at no size gives way to the standard folder. Change...'s program
/// wears a stand-in beside its name, then its icon, or none where it decodes at no size.
#[test]
fn a_sheets_picks_wear_their_cold_icons_once_they_land_and_a_broken_one_gives_way() {
    let (art, dir) = (tempfile::tempdir(), tempfile::tempdir());
    let (art, dir) = (art.expect("a temp dir"), dir.expect("a temp dir"));
    let music = dir.path().join("music");
    std::fs::create_dir(&music).expect("mkdir");
    let notes = dir.path().join("notes.txt");
    std::fs::write(&notes, "x").expect("write");
    let mut ctx = test_ctx();
    let drawn = [
        ("folder", DRAWN),
        ("folder-music", OTHER),
        ("editor-icon", DRAWN),
        ("viewer-icon", THIRD),
    ];
    let broken = ["folder-pictures", "player-icon"];
    ctx.icons = svg_theme(art.path(), &drawn, &broken);
    let mut ex = explorer_in(dir.path(), &mut ctx);
    ex.open_properties(&music, "music", "music", &mut ctx);
    land_until(&mut ex, &mut ctx, |ex| ex.props.is_some());
    let worn = |ex: &Explorer, id: &str| {
        let p = ex.props.as_ref().expect("the sheet is up");
        (p.sheet.pages.iter()).find_map(|page| page.item(id).map(|_| picture(page, id)))
    };
    let custom = |ex: &Explorer| {
        [filesheet::CUSTOM_ICON, filesheet::CUSTOM_PREVIEW].map(|id| worn(ex, id).flatten())
    };

    ex.icon_chosen(Some("folder-music".into()), &mut ctx);
    assert_eq!(custom(&ex), [Some(0); 2], "stand-ins");
    land_until(&mut ex, &mut ctx, |ex| custom(ex) == [Some(OTHER); 2]);
    assert_eq!(custom(&ex), [Some(OTHER); 2]);
    ex.icon_chosen(Some("folder-pictures".into()), &mut ctx);
    land_until(&mut ex, &mut ctx, |ex| custom(ex) == [Some(DRAWN); 2]);
    assert_eq!(custom(&ex), [Some(DRAWN); 2], "the standard folder");

    ex.props = None;
    ex.globs = std::sync::Arc::new(reprisefs::apps::MimeDb::parse(["50:text/plain:*.txt\n"]));
    ex.app_index = vec![app_with_icon("Editor", "editor", "editor-icon")].into();
    ex.open_properties(&notes, "notes.txt", "notes.txt", &mut ctx);
    land_until(&mut ex, &mut ctx, |ex| ex.props.is_some());
    assert_eq!(worn(&ex, "opens_icon"), Some(Some(DRAWN)), "Editor's");
    ex.props_set_opens_with("Viewer".into(), Some("viewer-icon".into()), &mut ctx);
    assert_eq!(worn(&ex, "opens_icon"), Some(Some(0)), "a stand-in");
    land_until(&mut ex, &mut ctx, |ex| {
        worn(ex, "opens_icon") == Some(Some(THIRD))
    });
    assert_eq!(worn(&ex, "opens_icon"), Some(Some(THIRD)));
    ex.props_set_opens_with("Player".into(), Some("player-icon".into()), &mut ctx);
    land_until(&mut ex, &mut ctx, |ex| worn(ex, "opens_icon") == Some(None));
    assert_eq!(
        worn(&ex, "opens_icon"),
        Some(None),
        "a broken icon leaves none"
    );
}

/// The preview of an entry its view drew no 48 px icon for wears a stand-in, then the file's
/// icon; the first pick decodes at no size and gives way to the extension's name.
#[test]
fn the_preview_card_wears_its_cold_icon_once_it_lands() {
    let (art, dir) = (tempfile::tempdir(), tempfile::tempdir());
    let (art, dir) = (art.expect("a temp dir"), dir.expect("a temp dir"));
    std::fs::write(dir.path().join("a.pdf"), "x").expect("write");
    let mut ctx = test_ctx();
    ctx.icons = svg_theme(
        art.path(),
        &[("application-pdf", DRAWN)],
        &["text-x-generic"],
    );
    let mut ex = explorer_in(dir.path(), &mut ctx);
    crate::tests::wake_until(&mut ex, &mut ctx, |ex| !ex.sv.entries.is_empty());
    ex.sv.entries[0].icon48 = None;
    ex.sv.selection.set_only(Some(0));
    ex.toggle_preview(&mut ctx);
    let worn = |ex: &Explorer| {
        let icon = ex
            .preview
            .as_ref()
            .expect("the preview is up")
            .qv
            .icon
            .clone();
        mid(&icon.map(|c| (*c).clone()))
    };
    assert_eq!(worn(&ex), Some(0), "a stand-in");
    land_until(&mut ex, &mut ctx, |ex| worn(ex) == Some(DRAWN));
    assert_eq!(worn(&ex), Some(DRAWN));
}
