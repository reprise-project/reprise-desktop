use super::*;
use crate::place::Place;
use crate::tests::test_ctx;
use reprisekit::controls::shellview::Mode;
use reprisekit::textedit::keysym;

fn key(sym: u32) -> Event {
    Event::Key(repriseapp::KeyInput {
        sym,
        utf8: (sym == keysym::SPACE).then(|| " ".to_string()),
        mods: repriseapp::Mods::default(),
        edit: reprisekit::textedit::key_for(sym, false, false, None),
    })
}

fn png(w: u32, h: u32) -> Vec<u8> {
    let mut c = Canvas::new(w, h);
    c.fill_rect(c.bounds(), reprisekit::theme::Color::rgb(0x20, 0x80, 0x40));
    reprisekit::raster::to_png(&c)
}

fn read_at(path: &Path, route: Route) -> Loaded {
    read(path, route, &Decoders::default_set(), None, (400, 300), 1)
}

#[test]
fn a_picture_decodes_fitted_text_reads_its_head_and_the_rest_keeps_the_card() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let d = dir.path();
    std::fs::write(d.join("a.png"), png(800, 200)).expect("write");
    std::fs::write(d.join("notes.txt"), "one\ntwo\n").expect("write");
    std::fs::write(d.join("README"), "plain words").expect("write");
    std::fs::write(d.join("blob.bin"), [0u8, 1, 2, 3]).expect("write");
    std::fs::write(d.join("empty"), "").expect("write");
    std::fs::write(d.join("big.txt"), "x".repeat(TEXT_BYTES + 1000)).expect("write");
    std::fs::write(d.join("broken.png"), "not a picture").expect("write");

    match read_at(&d.join("a.png"), Route::Picture) {
        Loaded::Picture(c) => assert_eq!((c.width, c.height), (400, 100), "fitted to the max"),
        _ => panic!("a.png is a picture"),
    }
    match read_at(&d.join("notes.txt"), Route::Text) {
        Loaded::Text(t) => assert_eq!(t, "one\ntwo\n"),
        _ => panic!("notes.txt is text"),
    }
    match read_at(&d.join("README"), Route::Other) {
        Loaded::Text(t) => assert_eq!(t, "plain words", "an untyped file sniffed as text"),
        _ => panic!("README is text"),
    }
    match read_at(&d.join("big.txt"), Route::Text) {
        Loaded::Text(t) => assert_eq!(t.len(), TEXT_BYTES, "the head alone"),
        _ => panic!("big.txt is text"),
    }
    for (name, route) in [
        ("blob.bin", Route::Other),
        ("empty", Route::Other),
        ("broken.png", Route::Picture),
        ("broken.png", Route::Font),
        ("missing", Route::Text),
    ] {
        assert!(
            matches!(read_at(&d.join(name), route), Loaded::Nothing),
            "{name} keeps the card"
        );
    }
    assert!(
        matches!(read_at(d, Route::Other), Loaded::Nothing),
        "a folder is not opened"
    );
}

#[test]
fn a_font_file_reads_as_its_sample_inside_the_max() {
    let Some(dir) = std::env::var_os("REPRISE_TEST_FONTS") else {
        return;
    };
    let tahoma = std::path::PathBuf::from(dir).join("tahoma.ttf");
    match read_at(&tahoma, Route::Font) {
        Loaded::Picture(c) => {
            assert!(
                c.width <= 400 && c.height <= 300,
                "{}x{}",
                c.width,
                c.height
            );
            assert!(
                c.height > 200,
                "the sample's rows down to the 72 px pangram"
            );
        }
        _ => panic!("tahoma.ttf has a sample"),
    }
}

#[test]
fn a_named_pipe_is_never_opened() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let fifo = dir.path().join("pipe");
    let made = std::process::Command::new("mkfifo").arg(&fifo).status();
    if !made.is_ok_and(|s| s.success()) {
        return;
    }
    assert!(matches!(read_at(&fifo, Route::Text), Loaded::Nothing));
}

fn explorer_over(names: &[&str], ctx: &mut Ctx) -> (tempfile::TempDir, Explorer) {
    let dir = tempfile::tempdir().expect("a temp dir");
    for n in names {
        std::fs::write(dir.path().join(n), "text of a file").expect("write");
    }
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), ctx);
    ex.sv.set_mode(Mode::Details);
    (dir, ex)
}

fn shown(ex: &Explorer) -> Option<String> {
    Some(ex.preview.as_ref()?.qv.facts.name.clone())
}

#[test]
fn space_previews_the_one_selected_file_and_closes_it_again() {
    let mut ctx = test_ctx();
    let (_dir, mut ex) = explorer_over(&["a.txt", "b.txt"], &mut ctx);
    ex.key(press_space(), &mut ctx);
    assert!(ex.preview.is_none(), "nothing selected, nothing to show");

    ex.sv.selection.select_all(2);
    ex.key(press_space(), &mut ctx);
    assert!(ex.preview.is_none(), "two selected: no preview");

    ex.sv.selection.set_only(Some(0));
    ex.key(press_space(), &mut ctx);
    assert_eq!(shown(&ex).as_deref(), Some("a.txt"));
    let child = ex.children().into_iter().find(|c| c.id == PREVIEW);
    let child = child.expect("the preview window is asked for");
    assert_eq!(child.title, "a.txt");
    assert!(!child.modal, "Explorer stays usable behind it");
    assert!(
        matches!(ex.preview.as_ref().unwrap().qv.body, Body::Text(_)),
        "the read landed within the wait"
    );

    ex.key(press_space(), &mut ctx);
    assert!(ex.preview.is_none(), "Space in the view closes it");
    assert!(ex.children().iter().all(|c| c.id != PREVIEW));
}

fn press_space() -> repriseapp::KeyInput {
    let Event::Key(k) = key(keysym::SPACE) else {
        unreachable!()
    };
    k
}

#[test]
fn the_arrows_in_the_preview_walk_the_views_order_and_move_its_selection() {
    let mut ctx = test_ctx();
    let (_dir, mut ex) = explorer_over(&["a.txt", "b.txt", "c.txt"], &mut ctx);
    ex.sv.selection.set_only(Some(0));
    ex.key(press_space(), &mut ctx);

    for (sym, want) in [
        (keysym::RIGHT, "b.txt"),
        (keysym::DOWN, "c.txt"),
        (keysym::DOWN, "c.txt"),
        (keysym::LEFT, "b.txt"),
        (keysym::UP, "a.txt"),
        (keysym::END, "c.txt"),
        (keysym::HOME, "a.txt"),
    ] {
        ex.preview_event(key(sym), &mut ctx);
        assert_eq!(shown(&ex).as_deref(), Some(want), "after {sym:#x}");
        let selected = ex.sv.selection.single().map(|i| ex.items[i].name.clone());
        assert_eq!(
            selected.as_deref(),
            Some(want),
            "the view's selection follows"
        );
    }
}

#[test]
fn escape_closes_and_enter_opens_the_file_and_closes() {
    let mut ctx = test_ctx();
    let (_dir, mut ex) = explorer_over(&["a.txt"], &mut ctx);
    ex.sv.selection.set_only(Some(0));
    ex.key(press_space(), &mut ctx);
    ex.preview_event(key(keysym::ESCAPE), &mut ctx);
    assert!(ex.preview.is_none());

    ex.key(press_space(), &mut ctx);
    let reply = ex.preview_event(key(keysym::RETURN), &mut ctx);
    assert!(ex.preview.is_none(), "Enter closes the preview");
    assert_eq!(reply.spawn.len(), 1, "and opens the file in its program");
}
