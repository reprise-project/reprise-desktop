use super::*;
use reprisekit::controls::breadcrumb;

/// A left click at `(x, y)` through [`Explorer::event`], which keeps the crumbs in step with the
/// search after it.
fn click(ex: &mut Explorer, x: i32, y: i32, ctx: &mut Ctx) {
    let mods = Default::default();
    let press = PointerInput::Press {
        x,
        y,
        button: BTN_LEFT,
        time: 0,
        mods,
    };
    ex.event(MAIN, Event::Pointer(press), ctx);
    let release = PointerInput::Release {
        x,
        y,
        button: BTN_LEFT,
        mods,
    };
    ex.event(MAIN, Event::Pointer(release), ctx);
}

/// The centre of crumb `i`'s button.
fn crumb_at(ex: &Explorer, i: usize, ctx: &Ctx) -> (i32, i32) {
    let sl = shell(ex, ctx);
    let (px, cm) = (ctx.theme.fonts.size_px, ctx.theme.controls());
    let bl = breadcrumb::layout(&ex.crumbs, sl.address, &ctx.text, px, &cm);
    let seg = bl.segments.iter().find(|s| s.crumb == i).expect("shown");
    (
        seg.button.x + seg.button.width / 2,
        seg.button.y + seg.button.height / 2,
    )
}

fn labels(ex: &Explorer) -> Vec<&str> {
    ex.crumbs.crumbs.iter().map(|c| c.label.as_str()).collect()
}

/// A folder `Documents` with report.txt and other.txt, Explorer in it under Glass light and
/// searching with `find`.
fn searching_docs() -> (tempfile::TempDir, Explorer, Ctx) {
    let dir = tempfile::tempdir().expect("a temp dir");
    let docs = dir.path().join("Documents");
    std::fs::create_dir(&docs).expect("mkdir");
    std::fs::write(docs.join("report.txt"), "x").expect("write");
    std::fs::write(docs.join("other.txt"), "x").expect("write");
    let mut ctx = glass(false);
    ctx.size = (1200, 573);
    let mut ex = test_explorer(Place::Folder(docs), &mut ctx);
    ex.search_run = |q, cancel| {
        let mut find = std::process::Command::new("find");
        find.arg(&q.root);
        Some(reprisefs::search::collect(find, q, cancel))
    };
    ex.search_field
        .key(reprisekit::textedit::EditKey::Insert("report".into()));
    ex.focus = Focus::Search;
    ex.event(MAIN, Event::Key(key(keysym::RETURN, None, false)), &mut ctx);
    assert_eq!(ex.items.len(), 1, "the hit alone");
    (dir, ex, ctx)
}

#[test]
fn a_search_shows_one_segment_behind_the_magnifier_and_back_leaves_it() {
    let (dir, mut ex, mut ctx) = searching_docs();
    assert_eq!(labels(&ex), ["Search results in Documents"]);
    assert!(ex.crumbs.searching, "the magnifier for the folder's icon");
    assert_eq!(
        ex.search.caption(&[]).as_deref(),
        Some("Search results in Documents"),
        "the title keeps the search's"
    );

    let (x, y) = crumb_at(&ex, 0, &ctx);
    click(&mut ex, x, y, &mut ctx);
    assert!(ex.search.shown().is_some(), "the segment searches again");
    assert_eq!(ex.items.len(), 1);

    ex.event(MAIN, Event::Key(key(keysym::LEFT, None, true)), &mut ctx);
    assert_eq!(ex.search.caption(&[]), None, "Back leaves the search");
    assert_eq!(ex.items.len(), 2, "the folder is back");
    assert_eq!(ex.place, Place::Folder(dir.path().join("Documents")));
    assert!(!ex.crumbs.searching);
    assert_eq!(
        labels(&ex).last(),
        Some(&"Documents"),
        "the folder's crumbs"
    );
}

#[test]
fn alt_up_and_a_subfolder_leave_the_search() {
    let (dir, mut ex, mut ctx) = searching_docs();
    ex.event(MAIN, Event::Key(key(keysym::UP, None, true)), &mut ctx);
    assert_eq!(ex.place, Place::Folder(dir.path().to_path_buf()));
    assert!(ex.search.asked().is_none());
    assert!(!ex.crumbs.searching);
    assert_ne!(labels(&ex), ["Search results in Documents"]);

    let (dir, mut ex, mut ctx) = searching_docs();
    let sub = dir.path().join("Documents").join("Old");
    std::fs::create_dir(&sub).expect("mkdir");
    ex.run(Command::GoTo(Place::Folder(sub.clone())), &mut ctx);
    ex.sync_search_crumb();
    assert_eq!(ex.place, Place::Folder(sub));
    assert!(ex.search.asked().is_none(), "a chevron row leaves it too");
    assert_eq!(labels(&ex).last(), Some(&"Old"));
}

#[test]
fn escape_in_the_search_box_leaves_the_search() {
    let (_dir, mut ex, mut ctx) = searching_docs();
    ex.focus = Focus::Search;
    let esc = || Event::Key(key(keysym::ESCAPE, None, false));
    ex.event(MAIN, esc(), &mut ctx);
    assert_eq!(ex.search_field.text(), "", "the first empties the box");
    ex.event(MAIN, esc(), &mut ctx);
    assert!(ex.search.asked().is_none(), "the second leaves the search");
    assert_eq!(ex.items.len(), 2);
    assert!(!ex.crumbs.searching);
    assert_eq!(labels(&ex).last(), Some(&"Documents"));
}

#[test]
fn enter_on_the_unchanged_address_leaves_the_search() {
    let (dir, mut ex, mut ctx) = searching_docs();
    ex.event(MAIN, Event::Key(key('d' as u32, Some("d"), true)), &mut ctx);
    assert_eq!(ex.focus, Focus::Address);
    ex.event(MAIN, Event::Key(key(keysym::RETURN, None, false)), &mut ctx);
    assert_eq!(ex.place, Place::Folder(dir.path().join("Documents")));
    assert!(ex.search.asked().is_none(), "the folder listed again");
    assert_eq!(ex.items.len(), 2);
    assert!(!ex.crumbs.searching);
}

#[test]
fn the_search_crumb_pins_light_and_dark_at_scale_1_and_2() {
    reprisekit::pins::fonts();
    let mut got = Vec::new();
    for dark in [false, true] {
        let mut ctx = glass(dark);
        let dir = tempfile::tempdir().expect("a temp dir");
        let docs = dir.path().join("Documents");
        std::fs::create_dir(&docs).expect("mkdir");
        let mut ex = test_explorer(Place::Folder(docs.clone()), &mut ctx);
        ex.search.start(&Place::Folder(docs), "notes", false);
        ex.sync_search_crumb();
        let look = if dark { "dark" } else { "light" };
        let n = ex.crumbs.crumbs.len();
        for (state, hot) in [
            ("rest", None),
            ("hot", Some(breadcrumb::Part::Segment(n - 1))),
        ] {
            ex.crumbs.hot = hot;
            let r = shell(&ex, &ctx).address;
            for s in [1, 2] {
                let mut c = Canvas::with_scale(792, 573, s);
                ctx.at_scale(s, |ctx| window::draw(&ex, &mut c, ctx));
                got.push((format!("{look} {state} @{s}"), crop_hash(&c, r)));
            }
        }
    }
    let path = reprisekit::crate_dir!().join("tests/pins/search_crumb.txt");
    reprisekit::pins::check_file(&path, &got);
}
