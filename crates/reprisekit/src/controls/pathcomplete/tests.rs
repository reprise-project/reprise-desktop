use super::*;
use crate::interface::Interface;
use crate::interfaces::theme_for;
use crate::theme::{FontConfig, Theme};

fn entries(names: &[&str]) -> Vec<Entry> {
    names
        .iter()
        .map(|n| match n.strip_suffix('/') {
            Some(d) => Entry {
                name: d.into(),
                dir: true,
            },
            None => Entry {
                name: n.to_string(),
                dir: false,
            },
        })
        .collect()
}

fn typed(pc: &mut PathComplete, edit: &mut TextEdit, s: &str) {
    for ch in s.chars() {
        pc.edit(edit, EditKey::Insert(ch.to_string()));
    }
}

/// Answers the model's wants from `fs`, a folder part and its names per entry, until it wants
/// none.
fn serve(pc: &mut PathComplete, edit: &mut TextEdit, fs: &[(&str, &[&str])]) {
    while let Some(w) = pc.take_want() {
        let names = fs
            .iter()
            .find(|(d, _)| *d == w.dir)
            .map_or(&[][..], |f| f.1);
        pc.set_listing(edit, &w, entries(names));
    }
}

const HOME: (&str, &[&str]) = (
    "~/",
    &[
        "Documents/",
        "Downloads/",
        "Desktop/",
        "Docs.txt",
        ".config/",
        ".bashrc",
        "notes",
    ],
);

fn shown(edit: &TextEdit) -> (&str, &str) {
    let sel = edit.selection();
    (&edit.text[..sel.start], &edit.text[sel])
}

#[test]
fn split_finds_the_folder_part_and_the_name_being_typed() {
    assert_eq!(split("~/Doc", Scope::Rooted), Some((0, 2)));
    assert_eq!(split("/usr/sh", Scope::Rooted), Some((0, 5)));
    assert_eq!(split("gimp ~/Pic", Scope::Rooted), Some((5, 7)));
    assert_eq!(split("/home/u/My Doc", Scope::Rooted), Some((0, 8)));
    assert_eq!(
        split("firefox", Scope::Rooted),
        None,
        "a command is no path"
    );
    assert_eq!(split("rep", Scope::Relative), Some((0, 0)));
    assert_eq!(split("sub/x", Scope::Relative), Some((0, 4)));
    assert_eq!(split("~", Scope::Rooted), None, "a bare tilde");
    assert_eq!(split("~u", Scope::Rooted), None, "another user's home");
    assert_eq!(split("*.txt", Scope::Relative), None, "a pattern");
    assert_eq!(split("\"a\" \"b", Scope::Relative), None, "quoted names");
    assert_eq!(split("", Scope::Relative), None);
}

#[test]
fn typing_asks_for_the_folder_once_and_completes_the_common_prefix_selected() {
    let mut pc = PathComplete::new(Scope::Rooted);
    let mut edit = TextEdit::new("", false);
    typed(&mut pc, &mut edit, "~/");
    assert_eq!(pc.take_want().map(|w| w.dir), Some("~/".into()));
    assert_eq!(pc.take_want(), None, "asked once");
    pc.set_listing(
        &mut edit,
        &Want {
            dir: "~/".into(),
            era: 0,
        },
        entries(HOME.1),
    );
    typed(&mut pc, &mut edit, "Do");
    assert_eq!(pc.take_want(), None, "the same folder is not listed again");
    assert_eq!(pc.rows(), ["Documents/", "Downloads/", "Docs.txt"]);
    assert_eq!(shown(&edit), ("~/Do", ""), "no longer common prefix");
    typed(&mut pc, &mut edit, "cu");
    assert_eq!(shown(&edit), ("~/Docu", "ments/"), "the tail is selected");
    typed(&mut pc, &mut edit, "m");
    assert_eq!(shown(&edit), ("~/Docum", "ents/"), "typing over the tail");
}

#[test]
fn a_listing_that_lands_after_the_key_still_completes_inline() {
    let mut pc = PathComplete::new(Scope::Rooted);
    let mut edit = TextEdit::new("", false);
    typed(&mut pc, &mut edit, "~/Dow");
    serve(&mut pc, &mut edit, &[HOME]);
    assert_eq!(shown(&edit), ("~/Dow", "nloads/"));
    assert_eq!(pc.rows(), ["Downloads/"]);
}

#[test]
fn folders_come_first_then_files_and_hidden_names_only_after_a_dot() {
    let mut pc = PathComplete::new(Scope::Rooted);
    let mut edit = TextEdit::new("", false);
    typed(&mut pc, &mut edit, "~/");
    serve(&mut pc, &mut edit, &[HOME]);
    assert_eq!(
        pc.rows(),
        ["Desktop/", "Documents/", "Downloads/", "Docs.txt", "notes"]
    );
    typed(&mut pc, &mut edit, ".");
    assert_eq!(pc.rows(), [".config/", ".bashrc"]);
}

#[test]
fn matching_is_case_sensitive() {
    let mut pc = PathComplete::new(Scope::Rooted);
    let mut edit = TextEdit::new("", false);
    typed(&mut pc, &mut edit, "~/do");
    serve(&mut pc, &mut edit, &[HOME]);
    assert!(!pc.is_open(), "no name starts with a small d");
}

#[test]
fn at_most_ten_matches_show() {
    let names: Vec<String> = (0..14).map(|i| format!("f{i:02}")).collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut pc = PathComplete::new(Scope::Rooted);
    let mut edit = TextEdit::new("", false);
    typed(&mut pc, &mut edit, "/tmp/f");
    serve(&mut pc, &mut edit, &[("/tmp/", &names)]);
    assert_eq!(pc.rows().len(), MAX_ROWS);
    assert_eq!(shown(&edit), ("/tmp/f", ""), "f00 and f13 share only f");
}

#[test]
fn backspace_eats_the_tail_and_does_not_bring_it_back() {
    let mut pc = PathComplete::new(Scope::Rooted);
    let mut edit = TextEdit::new("", false);
    typed(&mut pc, &mut edit, "~/Dow");
    serve(&mut pc, &mut edit, &[HOME]);
    pc.edit(&mut edit, EditKey::Backspace { word: false });
    assert_eq!(edit.text, "~/Dow", "only the tail went");
    assert!(pc.is_open(), "the match still lists");
}

#[test]
fn down_and_enter_take_a_row_and_a_folder_lists_its_contents_next() {
    let mut pc = PathComplete::new(Scope::Rooted);
    let mut edit = TextEdit::new("", false);
    typed(&mut pc, &mut edit, "~/D");
    serve(&mut pc, &mut edit, &[HOME]);
    assert!(pc.nav(&mut edit, Key::Down));
    assert_eq!(pc.hot, Some(0));
    assert!(pc.nav(&mut edit, Key::Down));
    assert!(pc.nav(&mut edit, Key::Up));
    assert!(pc.nav(&mut edit, Key::Enter));
    assert_eq!(edit.text, "~/Desktop/");
    assert_eq!((edit.caret(), edit.has_selection()), (10, false));
    serve(
        &mut pc,
        &mut edit,
        &[HOME, ("~/Desktop/", &["a.txt", "b/"])],
    );
    assert_eq!(pc.rows(), ["b/", "a.txt"]);
    assert_eq!(
        edit.text, "~/Desktop/",
        "nothing completes inline on its own"
    );
}

#[test]
fn enter_with_nothing_highlighted_closes_and_goes_on_to_the_owner() {
    let mut pc = PathComplete::new(Scope::Rooted);
    let mut edit = TextEdit::new("", false);
    typed(&mut pc, &mut edit, "~/Dow");
    serve(&mut pc, &mut edit, &[HOME]);
    assert!(!pc.nav(&mut edit, Key::Enter));
    assert!(!pc.is_open());
    assert_eq!(
        edit.text, "~/Downloads/",
        "the shown completion is what runs"
    );
}

#[test]
fn tab_takes_the_tail_and_escape_drops_it() {
    let mut pc = PathComplete::new(Scope::Rooted);
    let mut edit = TextEdit::new("", false);
    typed(&mut pc, &mut edit, "~/Dow");
    serve(&mut pc, &mut edit, &[HOME]);
    assert!(pc.nav(&mut edit, Key::Tab));
    assert_eq!(shown(&edit), ("~/Downloads/", ""));
    assert_eq!(pc.take_want().map(|w| w.dir), Some("~/Downloads/".into()));

    let mut pc = PathComplete::new(Scope::Rooted);
    let mut edit = TextEdit::new("", false);
    typed(&mut pc, &mut edit, "~/Dow");
    serve(&mut pc, &mut edit, &[HOME]);
    assert!(pc.nav(&mut edit, Key::Escape));
    assert_eq!((edit.text.as_str(), pc.is_open()), ("~/Dow", false));
    assert!(
        !pc.nav(&mut edit, Key::Escape),
        "a second Escape is the owner's"
    );
    assert!(
        !pc.nav(&mut edit, Key::Tab),
        "nothing to take: Tab moves on"
    );
}

#[test]
fn a_caret_off_the_end_closes_the_list() {
    let mut pc = PathComplete::new(Scope::Rooted);
    let mut edit = TextEdit::new("", false);
    typed(&mut pc, &mut edit, "~/D");
    serve(&mut pc, &mut edit, &[HOME]);
    pc.edit(
        &mut edit,
        EditKey::Left {
            select: false,
            word: false,
        },
    );
    assert!(!pc.is_open());
}

#[test]
fn an_exact_file_name_closes_the_list() {
    let mut pc = PathComplete::new(Scope::Rooted);
    let mut edit = TextEdit::new("", false);
    typed(&mut pc, &mut edit, "~/notes");
    serve(&mut pc, &mut edit, &[HOME]);
    assert!(!pc.is_open());
}

#[test]
fn one_read_at_a_time_and_a_forgotten_folder_drops_its_late_listing() {
    let mut pc = PathComplete::new(Scope::Relative);
    let mut edit = TextEdit::new("", false);
    typed(&mut pc, &mut edit, "a");
    let first = pc.take_want().expect("lists the folder");
    typed(&mut pc, &mut edit, "/");
    assert_eq!(pc.take_want(), None, "waits for the first read");
    pc.forget();
    assert!(!pc.set_listing(&mut edit, &first, entries(&["ab"])));
    typed(&mut pc, &mut edit, "b");
    assert_eq!(pc.take_want().map(|w| w.dir), Some("a/".into()));
}

#[test]
fn a_click_takes_the_row_under_it() {
    let mut pc = PathComplete::new(Scope::Rooted);
    let mut edit = TextEdit::new("", false);
    typed(&mut pc, &mut edit, "~/Do");
    serve(&mut pc, &mut edit, &[HOME]);
    let cm = Theme::default().controls();
    let lay = layout(&pc, Rect::new(10, 10, 200, 21), None, &cm);
    let y = lay.list.y + 1 + lay.row_h + lay.row_h / 2;
    assert_eq!(hit(&pc, &lay, 20, y), Some(1));
    assert_eq!(hit(&pc, &lay, 20, lay.list.y), None, "the frame");
    pc.take(&mut edit, 1);
    assert_eq!(edit.text, "~/Downloads/");
}

#[test]
fn a_list_with_no_room_below_goes_over_the_field_and_scrolls() {
    let mut pc = PathComplete::new(Scope::Rooted);
    let mut edit = TextEdit::new("", false);
    typed(&mut pc, &mut edit, "~/");
    serve(&mut pc, &mut edit, &[HOME]);
    let cm = Theme::default().controls();
    let field = Rect::new(0, 60, 200, 21);
    let lay = layout(&pc, field, Some(Rect::new(0, 0, 200, 100)), &cm);
    let rows = ((60 - 2) / cm.combo_row) as usize;
    assert_eq!(lay.rows, rows);
    assert_eq!(lay.list.bottom(), field.y, "over the field");
    assert!(lay.scrollbar.is_some(), "five matches, fewer rows");
    let lay = layout(&pc, field, Some(Rect::new(0, 0, 200, 300)), &cm);
    assert_eq!((lay.rows, lay.list.y), (5, field.bottom()));
}

#[test]
fn the_wheel_scrolls_a_list_with_more_matches_than_rows_and_keys_bring_the_highlight_back() {
    let names: Vec<String> = (0..10).map(|i| format!("f{i}")).collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut pc = PathComplete::new(Scope::Rooted);
    let mut edit = TextEdit::new("", false);
    typed(&mut pc, &mut edit, "/tmp/");
    serve(&mut pc, &mut edit, &[("/tmp/", &names)]);
    assert!(!pc.wheel(10, 3), "every row shows: nothing to scroll");
    assert!(pc.wheel(4, 3));
    assert_eq!(pc.top(4), 3);
    assert!(pc.wheel(4, 30));
    assert_eq!(pc.top(4), 6, "no further than the last row");
    let cm = Theme::default().controls();
    let field = Rect::new(0, 0, 200, 21);
    let lay = layout(
        &pc,
        field,
        Some(Rect::new(0, 0, 200, 21 + 4 * cm.combo_row + 2)),
        &cm,
    );
    assert_eq!(lay.rows, 4);
    assert_eq!(
        hit(&pc, &lay, 20, lay.list.y + 1),
        Some(6),
        "the top row in view"
    );
    pc.nav(&mut edit, Key::Down);
    assert_eq!(
        (pc.hot, pc.top(4)),
        (Some(0), 0),
        "Down brings row 0 back in view"
    );
    assert!(pc.wheel(4, -3), "up at the top still takes the wheel");
    assert_eq!(pc.top(4), 0);
    pc.close();
    assert!(!pc.wheel(4, 3), "a closed list leaves it");
}

fn pc_for_draw() -> PathComplete {
    let mut pc = PathComplete::new(Scope::Rooted);
    let mut edit = TextEdit::new("", false);
    typed(&mut pc, &mut edit, "~/D");
    serve(&mut pc, &mut edit, &[HOME]);
    pc.nav(&mut edit, Key::Down);
    pc.nav(&mut edit, Key::Down);
    pc
}

fn fnv(c: &Canvas) -> u64 {
    (0..c.height as i32)
        .flat_map(|y| (0..c.width as i32).map(move |x| (x, y)))
        .map(|(x, y)| c.get(x, y))
        .fold(0xcbf2_9ce4_8422_2325, |h, w| {
            w.to_le_bytes()
                .iter()
                .fold(h, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
        })
}

#[test]
fn the_list_pins_in_each_interface_at_scale_1_and_2() {
    crate::pins::fonts();
    let pc = pc_for_draw();
    let text = TextRenderer::load(&FontConfig::default());
    let mut got = Vec::new();
    for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
        let theme = theme_for(i);
        let cm = theme.controls();
        let lay = layout(&pc, Rect::new(4, -17, 160, 21), None, &cm);
        for s in [1, 2] {
            let w = (lay.list.right() + 8) as u32;
            let h = (lay.list.bottom() + 8) as u32;
            let mut c = Canvas::with_scale(w * s as u32, h * s as u32, s);
            let p = Paint { s, ..theme.paint() };
            draw(&pc, &mut c, &text, &p, &lay, theme.fonts.size_px * s as u32);
            let cs = &theme.colors;
            let hot = Rect::new(
                lay.list.x + 1,
                lay.list.y + 1 + lay.row_h,
                lay.list.width - 2,
                lay.row_h,
            );
            assert_eq!(
                c.get((hot.right() - 2) * s, (hot.y + hot.height / 2) * s),
                cs.hilight.argb(),
                "{i:?} @{s}: the second row is lit"
            );
            assert_eq!(
                c.get((hot.right() - 2) * s, (hot.bottom() + lay.row_h / 2) * s),
                cs.window.argb(),
                "{i:?} @{s}: the third is not"
            );
            got.push((format!("{i:?} {s}"), fnv(&c)));
        }
    }
    crate::pins::check("controls/pathcomplete", &got);
}

#[test]
fn the_list_reads_as_a_list_box_with_the_highlight_selected() {
    let pc = pc_for_draw();
    let cm = Theme::default().controls();
    let lay = layout(&pc, Rect::new(0, 0, 160, 21), None, &cm);
    let mut tree = Tree::window("", (200, 300));
    access(&pc, &lay, &mut tree, crate::access::ROOT);
    let list = crate::access::Tree::id_of(crate::access::ROOT, "completions");
    assert_eq!(tree.get(list).map(|n| n.role()), Some(Role::ListBox));
    let second = tree
        .find(Role::ListBoxOption, "Documents/")
        .expect("an option");
    assert_eq!(tree.get(second).unwrap().is_selected(), Some(true));
    let first = tree
        .find(Role::ListBoxOption, "Desktop/")
        .expect("an option");
    assert_eq!(tree.get(first).unwrap().is_selected(), Some(false));
}

#[test]
fn the_file_dialogs_file_name_completes_from_its_folder_on_a_popup() {
    use crate::controls::filedlg::{self, DlgMode, Fields, FileDlg};
    use crate::controls::placesbar::Place;
    use crate::selection::Mods;
    let mut d = FileDlg::new(
        Fields {
            title: "Open".into(),
            accept_label: "Open".into(),
            filters: vec!["All Files (*.*)".into()],
            mode: DlgMode::Open,
            multiple: false,
        },
        vec![Place {
            label: "My Computer".into(),
            icon: None,
            enabled: true,
            pinned: false,
        }],
    );
    let (w, h) = d.client();
    let r = Rect::new(0, 0, w, h);
    let key = |d: &mut FileDlg, k| d.key(r, k, None, Mods::default());
    key(&mut d, form::Key::Edit(EditKey::Insert("n".into())));
    let (pc, edit) = d.completion();
    let want = pc.take_want().expect("lists the open folder");
    assert_eq!(want.dir, "", "relative to the open folder");
    pc.set_listing(edit, &want, entries(&["notes/", "new.txt", "old.txt"]));
    assert_eq!(pc.rows(), ["notes/", "new.txt"]);

    let text = TextRenderer::load(&FontConfig::default());
    let px = Theme::default().fonts.size_px;
    let mut tree = Tree::window("Open", (w, h));
    d.access(&mut tree, r, &text, px);
    assert!(
        tree.find(Role::ListBoxOption, "notes/").is_some(),
        "File name's matches read as a list"
    );
    let mut p = filedlg::Popped {
        dlg: &mut d,
        text: std::rc::Rc::new(text),
        px,
    };
    p.host();
    let spots = p.spots();
    assert_eq!(spots.len(), 1, "the list goes on a popup");
    assert!(matches!(spots[0].anchor, Anchor::Below(_)));

    assert!(matches!(
        key(&mut d, form::Key::Down),
        filedlg::Event::Redraw
    ));
    assert!(matches!(
        key(&mut d, form::Key::Enter),
        filedlg::Event::Redraw
    ));
    assert_eq!(
        d.file_name(),
        "notes/",
        "Enter takes the row and opens nothing"
    );
}
