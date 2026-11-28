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

#[test]
fn a_search_ends_the_crumbs_in_its_segment_and_the_folders_crumb_leaves_it() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let docs = dir.path().join("Documents");
    std::fs::create_dir(&docs).expect("mkdir");
    std::fs::write(docs.join("report.txt"), "x").expect("write");
    std::fs::write(docs.join("other.txt"), "x").expect("write");
    let mut ctx = glass(false);
    ctx.size = (1200, 573);
    let place = Place::Folder(docs);
    let mut ex = test_explorer(place.clone(), &mut ctx);
    ex.search_run = |q, cancel| {
        let mut find = std::process::Command::new("find");
        find.arg(&q.root);
        Some(reprisefs::search::collect(find, q, cancel))
    };
    let before = ex.crumbs.crumbs.clone();
    assert_eq!(labels(&ex).last(), Some(&"Documents"));

    ex.search_field
        .key(reprisekit::textedit::EditKey::Insert("report".into()));
    ex.focus = Focus::Search;
    ex.event(MAIN, Event::Key(key(keysym::RETURN, None, false)), &mut ctx);
    assert_eq!(ex.items.len(), 1, "the hit alone");
    let n = before.len();
    assert_eq!(
        ex.crumbs.crumbs[..n],
        before[..],
        "the folder's crumbs first"
    );
    assert_eq!(labels(&ex)[n..], ["Search results in Documents"]);
    assert_eq!(
        ex.search.caption(&[]).as_deref(),
        Some("Search results in Documents"),
        "the title keeps the search's"
    );

    let (x, y) = crumb_at(&ex, n, &ctx);
    click(&mut ex, x, y, &mut ctx);
    assert!(
        ex.search.shown().is_some(),
        "its own segment searches again"
    );
    assert_eq!(ex.items.len(), 1);
    assert_eq!(ex.crumbs.crumbs.len(), n + 1);

    let (x, y) = crumb_at(&ex, n - 1, &ctx);
    click(&mut ex, x, y, &mut ctx);
    assert_eq!(ex.search.caption(&[]), None, "the search is over");
    assert_eq!(ex.search_field.text(), "");
    assert_eq!(ex.items.len(), 2, "the folder is back");
    assert_eq!(ex.place, place);
    assert_eq!(ex.crumbs.crumbs, before, "the search's segment is gone");
}

#[test]
fn a_crumb_above_the_folder_leaves_the_search_for_its_place() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let docs = dir.path().join("Documents");
    std::fs::create_dir(&docs).expect("mkdir");
    let mut ctx = glass(false);
    ctx.size = (1200, 573);
    let mut ex = test_explorer(Place::Folder(docs.clone()), &mut ctx);
    ex.search.start(&Place::Folder(docs), "notes", false);
    assert!(ex.sync_search_crumb());
    let n = ex.crumbs.crumbs.len();
    let (x, y) = crumb_at(&ex, n - 3, &ctx);
    click(&mut ex, x, y, &mut ctx);
    assert_eq!(ex.place, Place::Folder(dir.path().to_path_buf()));
    assert!(ex.search.asked().is_none());
    assert_eq!(ex.search_crumb, None);
    assert_ne!(
        labels(&ex).last(),
        Some(&"Search results in Documents"),
        "the crumbs end in the new place"
    );
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
            // The search's segment and its chevron: the crumbs before it name the temp dir.
            let sl = shell(&ex, &ctx);
            let (px, cm) = (ctx.theme.fonts.size_px, ctx.theme.controls());
            let bl = breadcrumb::layout(&ex.crumbs, sl.address, &ctx.text, px, &cm);
            let seg = bl.segments.last().expect("the search's segment");
            assert_eq!(seg.crumb, n - 1);
            let r = seg.button.union(&seg.chevron);
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
