use super::*;
use repriseapp::{BTN_LEFT, BTN_RIGHT, KeyInput, Mods, PointerInput};
use reprisekit::interface::Interface;
use reprisekit::textedit::{EditKey, keysym};
use reprisekit::theme::FontConfig;
use std::path::Path;
use std::time::Duration;

fn ctx_with(theme: Theme) -> Ctx {
    Ctx {
        theme,
        text: TextRenderer::load(&FontConfig::default()).into(),
        icons: reprisekit::icontheme::IconCache::new(reprisekit::icontheme::IconTheme::load(
            "test",
            &[],
        )),
        caret_on: true,
        size: NOTE_SIZE,
        maximized: false,
        origin: (0, 0),
        wake: repriseapp::Waker::noop(),
        heads: Default::default(),
        want_tick: Default::default(),
        output_size: None,
        at_pick: false,
        text_size: 100,
    }
}

fn test_ctx() -> Ctx {
    ctx_with(Theme::default())
}

/// The program over notes kept in `dir`, started.
fn started(dir: &Path) -> Notes {
    let mut n = Notes::new(Some(dir.to_path_buf()), false);
    assert_eq!(
        n.start(repriseapp::Waker::noop()),
        repriseapp::Start::Continue
    );
    n
}

fn typed(text: &str) -> Event {
    Event::Key(KeyInput {
        sym: 0,
        utf8: Some(text.into()),
        mods: Mods::default(),
        edit: Some(EditKey::Insert(text.into())),
    })
}

fn char_key(c: char) -> Event {
    Event::Key(KeyInput {
        sym: c as u32,
        utf8: Some(c.to_string()),
        mods: Mods::default(),
        edit: None,
    })
}

fn sym_key(sym: u32) -> Event {
    Event::Key(KeyInput {
        sym,
        utf8: None,
        mods: Mods::default(),
        edit: None,
    })
}

fn press(x: i32, y: i32, button: u32) -> Event {
    Event::Pointer(PointerInput::Press {
        x,
        y,
        button,
        time: 0,
        mods: Mods::default(),
    })
}

fn release(x: i32, y: i32) -> Event {
    Event::Pointer(PointerInput::Release {
        x,
        y,
        button: BTN_LEFT,
        mods: Mods::default(),
    })
}

fn click(n: &mut Notes, win: WinId, (x, y): (i32, i32), ctx: &mut Ctx) -> Reply {
    n.event(win, press(x, y, BTN_LEFT), ctx);
    n.event(win, release(x, y), ctx)
}

fn later() -> Instant {
    Instant::now() + Autosave::DELAY + Duration::from_millis(50)
}

fn kept(dir: &Path) -> Vec<Saved> {
    store::load(dir)
}

#[test]
fn a_first_start_opens_one_empty_note_and_keeps_it() {
    let dir = tempfile::tempdir().unwrap();
    let n = started(dir.path());
    let kids = n.children();
    assert_eq!(kids.len(), 1);
    assert_eq!(kids[0].title, "Sticky Note");
    assert_eq!(kids[0].size, NOTE_SIZE);
    assert_eq!(n.parent(kids[0].id), None, "a window of its own");
    assert_eq!(
        kept(dir.path()).len(),
        1,
        "written before anything is typed"
    );
}

#[test]
fn a_login_start_with_no_notes_ends_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let mut n = Notes::new(Some(dir.path().to_path_buf()), true);
    assert_eq!(
        n.start(repriseapp::Waker::noop()),
        repriseapp::Start::Exit(0)
    );
    assert!(kept(dir.path()).is_empty(), "no note made");
}

#[test]
fn typing_is_written_after_a_pause() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = test_ctx();
    let mut n = started(dir.path());
    let win = n.notes[0].win;
    let reply = n.event(win, typed("Buy milk"), &mut ctx);
    assert!(reply.tick, "a tick comes back to write it");
    assert_eq!(kept(dir.path())[0].text, "", "not yet");
    assert!(!n.save_due(later()), "nothing waits after the write");
    assert_eq!(kept(dir.path())[0].text, "Buy milk");
    assert_eq!(n.children()[0].title, "Buy milk");
}

#[test]
fn a_close_writes_what_waits_and_ends_the_program() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = test_ctx();
    let mut n = started(dir.path());
    let win = n.notes[0].win;
    n.event(win, typed("x"), &mut ctx);
    let reply = n.event(win, Event::CloseRequest, &mut ctx);
    assert!(reply.quit);
    assert_eq!(kept(dir.path())[0].text, "x", "kept, not deleted");
}

#[test]
fn kept_notes_come_back_with_their_text_colour_size_and_own_place() {
    let dir = tempfile::tempdir().unwrap();
    for (id, text, colour) in [
        ("1", "one", Colour::Green),
        ("2", "two\nlines", Colour::Blue),
    ] {
        store::write(
            dir.path(),
            &Saved {
                id: id.into(),
                text: text.into(),
                colour,
                size: Some((300, 220)),
            },
        )
        .unwrap();
    }
    let mut n = Notes::new(Some(dir.path().to_path_buf()), true);
    assert_eq!(
        n.start(repriseapp::Waker::noop()),
        repriseapp::Start::Continue
    );
    let kids = n.children();
    let titles: Vec<&str> = kids.iter().map(|k| k.title.as_str()).collect();
    assert_eq!(titles, ["one", "two"]);
    assert!(kids.iter().all(|k| k.size == (300, 220)));
    assert_eq!(n.notes[1].edit.text(), "two\nlines");
    assert_eq!(n.notes[1].colour, Colour::Blue);
    assert_eq!(n.place_tag(kids[0].id).as_deref(), Some("note-1"));
    assert_eq!(n.place_tag(kids[1].id).as_deref(), Some("note-2"));
}

#[test]
fn a_resize_is_kept_as_the_notes_size() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = test_ctx();
    let mut n = started(dir.path());
    let win = n.notes[0].win;
    ctx.size = (260, 240);
    n.event(win, Event::Resized, &mut ctx);
    n.save_due(later());
    assert_eq!(kept(dir.path())[0].size, Some((260, 240)));
}

#[test]
fn the_colour_rows_mark_the_notes_colour_and_a_pick_recolours_it() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = test_ctx();
    let mut n = started(dir.path());
    let win = n.notes[0].win;
    n.event(win, press(60, 80, BTN_RIGHT), &mut ctx);
    let menu = n.menu.as_ref().expect("a right-click opens the menu");
    let radios: Vec<(String, bool)> = (menu.tree().iter())
        .filter_map(|item| match item {
            reprisekit::controls::menu::MenuItem::Entry { label, checked, .. } => {
                Some((label.replace('&', ""), *checked))
            }
            _ => None,
        })
        .skip(7)
        .collect();
    let want: Vec<(String, bool)> = [
        ("Yellow", true),
        ("Green", false),
        ("Pink", false),
        ("Purple", false),
        ("Blue", false),
        ("White", false),
        ("New Note", false),
        ("Delete Note", false),
    ]
    .iter()
    .map(|(l, r)| (l.to_string(), *r))
    .collect();
    assert_eq!(radios, want, "under Undo..Select All");
    n.event(win, char_key('g'), &mut ctx);
    assert!(n.menu.is_none(), "the pick closes it");
    assert_eq!(n.notes[0].colour, Colour::Green);
    n.save_due(later());
    assert_eq!(kept(dir.path())[0].colour, Colour::Green);
}

#[test]
fn the_menu_runs_undo_on_the_text() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = test_ctx();
    let mut n = started(dir.path());
    let win = n.notes[0].win;
    n.event(win, typed("abc"), &mut ctx);
    n.event(win, press(60, 80, BTN_RIGHT), &mut ctx);
    n.event(win, char_key('u'), &mut ctx);
    assert_eq!(n.notes[0].edit.text(), "");
}

#[test]
fn plus_opens_a_note_in_the_same_colour_and_ctrl_n_too() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = test_ctx();
    let mut n = started(dir.path());
    let win = n.notes[0].win;
    n.notes[0].colour = Colour::Pink;
    click(&mut n, win, (8, 8), &mut ctx);
    assert_eq!(n.notes.len(), 2);
    assert_eq!(n.notes[1].colour, Colour::Pink);
    n.event(
        win,
        Event::Key(KeyInput {
            sym: 0x6e,
            utf8: Some("n".into()),
            mods: Mods {
                ctrl: true,
                ..Mods::default()
            },
            edit: None,
        }),
        &mut ctx,
    );
    assert_eq!(n.notes.len(), 3);
    assert_eq!(kept(dir.path()).len(), 3);
    let ids: std::collections::BTreeSet<_> = n.notes.iter().map(|n| n.id.clone()).collect();
    assert_eq!(ids.len(), 3, "each its own id");
}

#[test]
fn x_deletes_an_empty_note_at_once_and_asks_for_one_with_text() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = test_ctx();
    let mut n = started(dir.path());
    let first = n.notes[0].win;
    let second = n.new_note(Colour::Yellow);
    let x = (NOTE_SIZE.0 - 8, 8);
    let reply = click(&mut n, second, x, &mut ctx);
    assert!(!reply.quit);
    assert_eq!(n.notes.len(), 1, "an empty note goes without a word");
    n.event(first, typed("keep me?"), &mut ctx);
    click(&mut n, first, x, &mut ctx);
    assert!(n.confirm.is_some(), "a note with text asks");
    assert_eq!(n.parent(CONFIRM), Some(first));
    assert_eq!(n.caption(CONFIRM), Caption::Prompt);
    let mut c = Canvas::new(400, 200);
    n.draw(CONFIRM, &mut c, &ctx);
    n.event(CONFIRM, sym_key(keysym::ESCAPE), &mut ctx);
    assert!(n.confirm.is_none());
    assert_eq!(n.notes.len(), 1, "Escape keeps it");
    click(&mut n, first, x, &mut ctx);
    n.draw(CONFIRM, &mut c, &ctx);
    let reply = n.event(CONFIRM, sym_key(keysym::RETURN), &mut ctx);
    assert!(n.notes.is_empty());
    assert!(reply.quit, "the last note gone ends the program");
    assert!(
        std::fs::read_dir(dir.path()).unwrap().next().is_none(),
        "its files went too"
    );
}

#[test]
fn a_note_has_no_frame_and_its_strip_and_edges_move_and_resize_it() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = test_ctx();
    let mut n = started(dir.path());
    let win = n.notes[0].win;
    assert_eq!(n.caption(win), Caption::None);
    let (w, h) = NOTE_SIZE;
    assert_eq!(n.grip(win, (w / 2, 10), NOTE_SIZE), Some(Grip::Move));
    assert_eq!(n.grip(win, (8, 8), NOTE_SIZE), None, "New");
    assert_eq!(n.grip(win, (w / 2, h / 2), NOTE_SIZE), None, "the text");
    let corner = Grip::Resize(reprisekit::frame::Edges::BOTTOM | reprisekit::frame::Edges::RIGHT);
    assert_eq!(n.grip(win, (w - 1, h - 1), NOTE_SIZE), Some(corner));
    n.open_menu(win, (20, 40), false, &ctx);
    assert_eq!(
        n.grip(win, (w / 2, 10), NOTE_SIZE),
        None,
        "the menu takes the press"
    );
    assert_eq!(n.grip(CONFIRM, (1, 1), NOTE_SIZE), None);
}

fn fnv(c: &Canvas) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for v in [c.width, c.height]
        .into_iter()
        .chain(c.pixels.iter().copied())
    {
        for b in v.to_le_bytes() {
            h = (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3);
        }
    }
    h
}

#[test]
fn a_note_pins_in_every_interface_at_scale_1_and_2() {
    reprisekit::pins::fonts();
    let mut got = Vec::new();
    for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
        for scale in [1, 2] {
            let mut ctx = ctx_with(reprisekit::interfaces::theme_for(i));
            let mut n = Notes::new(None, false);
            let win = n.add(Saved {
                id: "1".into(),
                text: "Call the plumber\nabout the sink\n\n1\n2\n3\n4\n5\n6\n7\n8".into(),
                colour: Colour::Yellow,
                size: None,
            });
            n.event(win, Event::Focus(true), &mut ctx);
            n.event(
                win,
                Event::Pointer(PointerInput::Motion { x: 8, y: 8 }),
                &mut ctx,
            );
            let mut c = Canvas::with_scale(NOTE_SIZE.0 as u32, NOTE_SIZE.1 as u32, scale);
            ctx.at_scale(scale, |ctx| n.draw(win, &mut c, ctx));
            got.push((format!("{i:?}@{scale}"), fnv(&c)));
        }
    }
    let want: [(&str, u64); 6] = [
        ("Classic@1", 0x6529_c56c_3ac9_98d8),
        ("Classic@2", 0xeec7_fbaf_8a12_90d3),
        ("Blue@1", 0xc62b_c70d_a5dd_72fd),
        ("Blue@2", 0x6660_a4b1_c6fb_1991),
        ("Glass@1", 0x2f00_3064_3925_1969),
        ("Glass@2", 0x1d5a_f944_6268_5865),
    ];
    let want: Vec<(String, u64)> = want.iter().map(|(k, h)| (k.to_string(), *h)).collect();
    assert_eq!(got, want);
}

fn edit_key(sym: u32, edit: EditKey) -> Event {
    Event::Key(KeyInput {
        sym,
        utf8: None,
        mods: Mods::default(),
        edit: Some(edit),
    })
}

const PARAGRAPH: &str = "Call the plumber about the sink before Friday, and ask whether the old tap \
                         can stay or needs replacing too";

#[test]
fn a_long_paragraph_wraps_and_rewraps_when_the_note_is_resized() {
    let mut ctx = test_ctx();
    let mut n = Notes::new(None, false);
    let win = n.add(Saved {
        id: "1".into(),
        text: PARAGRAPH.into(),
        colour: Colour::Yellow,
        size: None,
    });
    n.event(win, Event::Focus(true), &mut ctx);
    let rows = |n: &Notes, ctx: &Ctx| {
        let lay = n.edit_layout(n.note(win).unwrap(), ctx);
        assert_eq!(lay.hscroll, None, "a note never scrolls sideways");
        lay.total
    };
    let narrow = rows(&n, &ctx);
    assert!(narrow > 2, "{narrow} rows");
    n.event(
        win,
        edit_key(keysym::END, EditKey::End { select: false }),
        &mut ctx,
    );
    let end = n.notes[0].edit.caret();
    assert!(end.col < PARAGRAPH.len(), "End stays on the first row");
    n.event(win, sym_key(keysym::DOWN), &mut ctx);
    n.event(
        win,
        edit_key(keysym::HOME, EditKey::Home { select: false }),
        &mut ctx,
    );
    assert_eq!(
        n.notes[0].edit.caret().col,
        end.col + 1,
        "the second row's start"
    );
    ctx.size = (NOTE_SIZE.0 * 2, NOTE_SIZE.1);
    n.event(win, Event::Resized, &mut ctx);
    let wide = rows(&n, &ctx);
    assert!(
        wide < narrow,
        "{wide} rows at twice the width, {narrow} before"
    );
}

#[test]
fn a_wrapped_note_pins_in_every_interface_at_scale_1_and_2() {
    reprisekit::pins::fonts();
    let mut got = Vec::new();
    for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
        for scale in [1, 2] {
            let mut ctx = ctx_with(reprisekit::interfaces::theme_for(i));
            let mut n = Notes::new(None, false);
            let win = n.add(Saved {
                id: "1".into(),
                text: format!("{PARAGRAPH}\n\n{PARAGRAPH}\n\n{PARAGRAPH}"),
                colour: Colour::Green,
                size: None,
            });
            n.event(win, Event::Focus(true), &mut ctx);
            n.event(
                win,
                edit_key(keysym::END, EditKey::End { select: false }),
                &mut ctx,
            );
            n.event(
                win,
                Event::Pointer(PointerInput::Motion { x: 8, y: 8 }),
                &mut ctx,
            );
            let mut c = Canvas::with_scale(NOTE_SIZE.0 as u32, NOTE_SIZE.1 as u32, scale);
            ctx.at_scale(scale, |ctx| n.draw(win, &mut c, ctx));
            got.push((format!("{i:?}@{scale}"), fnv(&c)));
        }
    }
    let want: [(&str, u64); 6] = [
        ("Classic@1", 0x9cd8_3236_e964_9fc5),
        ("Classic@2", 0x9675_554e_2085_de61),
        ("Blue@1", 0xa22d_5490_24e3_7314),
        ("Blue@2", 0x397f_19b4_19c0_dcd4),
        ("Glass@1", 0x2162_d30d_b930_ff7d),
        ("Glass@2", 0xad4a_aff2_c985_76f3),
    ];
    let want: Vec<(String, u64)> = want.iter().map(|(k, h)| (k.to_string(), *h)).collect();
    assert_eq!(got, want);
}
