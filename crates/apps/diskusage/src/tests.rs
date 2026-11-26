use super::*;
use reprisekit::pointer::Interaction;
use std::fs;
use std::time::Duration;

fn test_ctx(theme: reprisekit::theme::Theme) -> Ctx {
    Ctx {
        theme,
        text: reprisekit::text::TextRenderer::load(&reprisekit::theme::FontConfig::default())
            .into(),
        icons: reprisekit::icontheme::IconCache::new(reprisekit::icontheme::IconTheme::load(
            "test",
            &[],
        )),
        caret_on: true,
        size: CLIENT,
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

/// The window on `root`, counted to the end.
fn counted(root: &Path) -> DiskUsage {
    let mut d = DiskUsage::new(root.to_path_buf());
    let waker = repriseapp::Waker::noop();
    d.trashed = Some(Poster::channel(waker.clone()));
    d.waker = Some(waker);
    d.rescan(root.to_path_buf());
    let until = Instant::now() + Duration::from_secs(10);
    while !d.scan.as_ref().unwrap().done() {
        assert!(Instant::now() < until, "the count ends");
        std::thread::sleep(Duration::from_millis(5));
    }
    d.sync();
    d
}

fn names(d: &DiskUsage) -> Vec<&str> {
    d.list
        .view
        .rows
        .lines
        .iter()
        .map(|l| l.name.as_str())
        .collect()
}

fn pick(d: &mut DiskUsage, name: &str) {
    let i = names(d).iter().position(|n| *n == name).unwrap();
    d.list.view.selection.set_only(Some(i));
}

/// Files whose sizes are whole 4 KiB blocks on every common file system, their bytes noise so
/// a compressing one stores them whole.
fn files(root: &Path) {
    let mut seed = 0x2545_f491_u32;
    for (name, kib) in [("video.mkv", 64), ("notes.txt", 8), ("photo.jpg", 32)] {
        let bytes: Vec<u8> = (0..kib * 1024)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                seed as u8
            })
            .collect();
        fs::write(root.join(name), bytes).unwrap();
    }
}

#[test]
fn opening_a_folder_goes_in_and_back_and_up_come_out() {
    let t = tempfile::tempdir().unwrap();
    let r = t.path();
    fs::create_dir_all(r.join("a/b")).unwrap();
    fs::write(r.join("a/b/f"), vec![1u8; 8192]).unwrap();
    fs::write(r.join("g"), b"g").unwrap();
    let mut d = counted(r);
    assert_eq!(names(&d), ["a", "g"]);
    pick(&mut d, "a");
    d.open_picked();
    assert_eq!(d.at, r.join("a"));
    assert_eq!(names(&d), ["b"]);
    pick(&mut d, "b");
    d.open_picked();
    assert_eq!(d.at, r.join("a/b"));
    d.button("back");
    assert_eq!(d.at, r.join("a"));
    d.button("up");
    assert_eq!(d.at, r);
    assert_eq!(d.root, r, "inside the count: no new one");
    d.button("up");
    assert_eq!(
        d.at,
        r.parent().unwrap(),
        "past the count's top: counted afresh"
    );
    assert_eq!(d.root, d.at);
}

fn press(d: &mut DiskUsage, ctx: &mut Ctx, (x, y): (i32, i32)) -> Reply {
    let ev = PointerInput::Press {
        x,
        y,
        button: BTN_LEFT,
        time: 0,
        mods: Default::default(),
    };
    d.event(MAIN, Event::Pointer(ev), ctx)
}

fn release(d: &mut DiskUsage, ctx: &mut Ctx, (x, y): (i32, i32)) -> Reply {
    let ev = PointerInput::Release {
        x,
        y,
        button: BTN_LEFT,
        mods: Default::default(),
    };
    d.event(MAIN, Event::Pointer(ev), ctx)
}

fn centre(r: Rect) -> (i32, i32) {
    (r.x + r.width / 2, r.y + r.height / 2)
}

#[test]
fn a_top_button_acts_on_its_release_and_not_when_let_go_off_it() {
    let t = tempfile::tempdir().unwrap();
    let r = t.path();
    fs::create_dir_all(r.join("a/b")).unwrap();
    let mut d = counted(r);
    pick(&mut d, "a");
    d.open_picked();
    let mut ctx = test_ctx(reprisekit::interfaces::theme_for(
        reprisekit::interface::Interface::Classic,
    ));
    d.draw_main(&mut Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32), &ctx);
    let up = centre(d.form.rect("up").unwrap());
    let back = centre(d.form.rect("back").unwrap());
    let list = centre(d.list_rect(&ctx));

    press(&mut d, &mut ctx, up);
    let off = PointerInput::Motion {
        x: list.0,
        y: list.1,
    };
    d.event(MAIN, Event::Pointer(off), &mut ctx);
    release(&mut d, &mut ctx, list);
    assert_eq!(d.at, r.join("a"), "Up let go over the list stays put");

    press(&mut d, &mut ctx, back);
    release(&mut d, &mut ctx, back);
    assert_eq!(d.at, r, "Back clicked goes back");
}

fn motion(d: &mut DiskUsage, ctx: &mut Ctx, (x, y): (i32, i32)) -> Reply {
    d.event(MAIN, Event::Pointer(PointerInput::Motion { x, y }), ctx)
}

#[test]
fn a_pointer_click_on_a_column_title_sorts_by_it_and_a_second_flips_in_every_look() {
    use reprisekit::interface::Interface;
    for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
        let t = tempfile::tempdir().unwrap();
        files(t.path());
        let mut d = counted(t.path());
        let mut ctx = test_ctx(reprisekit::interfaces::theme_for(i));
        let r = d.list_rect(&ctx);
        let lay = d.list.layout(r, &ctx);
        let title = (lay.header.x + 4, lay.header.y + lay.header.height / 2);
        assert_eq!(
            d.list.view.hit(&lay, title.0, title.1).column(),
            Some(model::NAME)
        );
        press(&mut d, &mut ctx, title);
        assert_eq!(
            d.list.view.header.of(model::NAME),
            Interaction::Pressed,
            "{i:?}: the pressed title is held"
        );
        assert_eq!(
            names(&d),
            ["video.mkv", "photo.jpg", "notes.txt"],
            "{i:?}: the press alone sorts nothing"
        );
        assert!(release(&mut d, &mut ctx, title).redraw, "{i:?}");
        assert_eq!(names(&d), ["notes.txt", "photo.jpg", "video.mkv"], "{i:?}");
        assert_eq!(
            d.list.view.sort,
            Some((model::NAME, true)),
            "{i:?}: Name A to Z draws the rising arrow"
        );
        press(&mut d, &mut ctx, title);
        release(&mut d, &mut ctx, title);
        assert_eq!(
            names(&d),
            ["video.mkv", "photo.jpg", "notes.txt"],
            "{i:?}: a second click flips"
        );
        assert_eq!(d.list.view.sort, Some((model::NAME, false)), "{i:?}");
    }
}

#[test]
fn a_classic_title_released_off_it_or_a_divider_drag_sorts_nothing() {
    let t = tempfile::tempdir().unwrap();
    files(t.path());
    let mut d = counted(t.path());
    let mut ctx = test_ctx(reprisekit::theme::Theme::default());
    let r = d.list_rect(&ctx);
    let lay = d.list.layout(r, &ctx);
    let y = lay.header.y + lay.header.height / 2;
    let title = (lay.header.x + 4, y);
    let below = (title.0, lay.body.y + lay.body.height - 2);
    press(&mut d, &mut ctx, title);
    assert!(motion(&mut d, &mut ctx, below).redraw, "the title lifts");
    release(&mut d, &mut ctx, below);
    assert_eq!(names(&d), ["video.mkv", "photo.jpg", "notes.txt"]);

    let width = d.list.view.columns[model::NAME].width;
    let edge = (
        lay.header.x + lay.col_w(&d.list.view.columns[model::NAME]),
        y,
    );
    assert_eq!(
        d.list.view.hit(&lay, edge.0, edge.1),
        reprisekit::controls::listview::Hit::Divider(model::NAME)
    );
    press(&mut d, &mut ctx, edge);
    motion(&mut d, &mut ctx, (edge.0 + 30, y));
    release(&mut d, &mut ctx, (edge.0 + 30, y));
    assert!(
        d.list.view.columns[model::NAME].width > width,
        "the drag widens Name"
    );
    assert_eq!(names(&d), ["video.mkv", "photo.jpg", "notes.txt"]);
    assert_eq!(d.list.view.sort, Some((model::SIZE, false)));
}

#[test]
fn a_header_click_sorts_and_a_second_flips() {
    let t = tempfile::tempdir().unwrap();
    files(t.path());
    let mut d = counted(t.path());
    assert_eq!(names(&d), ["video.mkv", "photo.jpg", "notes.txt"]);
    d.sort = (Sort::Name, false);
    d.sync();
    assert_eq!(names(&d), ["notes.txt", "photo.jpg", "video.mkv"]);
    d.sort = (Sort::Size, true);
    d.sync();
    assert_eq!(names(&d), ["notes.txt", "photo.jpg", "video.mkv"]);
    let shares: Vec<f64> = d.list.view.rows.lines.iter().map(|l| l.share).collect();
    assert_eq!(shares, [8.0 / 104.0, 32.0 / 104.0, 64.0 / 104.0]);
    assert_eq!(d.list.view.rows.bar(2, SHARE), Some(64.0 / 104.0));
    assert_eq!(d.list.view.rows.bar(2, model::SIZE), None);
}

/// Move to Recycle Bin, with the confirmation off, lands in a throwaway home trash and takes
/// the entry and its size out of the count.
#[test]
fn move_to_recycle_bin_takes_the_entry_out_of_the_count() {
    let t = tempfile::tempdir().unwrap();
    let r = t.path().join("data");
    fs::create_dir(&r).unwrap();
    files(&r);
    let config = t.path().join("config.toml");
    fs::write(&config, "[recycle_bin]\nconfirm = false\n").unwrap();
    reprisefs::configfile::set_path(Some(config));
    reprisefs::trash::set_home_trash(Some(t.path().join("Trash")));
    reprisefs::trash::set_mount_table(Some("/dev/null".into()));
    let mut d = counted(&r);
    let before = d.scan.as_ref().unwrap().tree().node(ROOT).size;
    pick(&mut d, "photo.jpg");
    d.ask_trash(r.join("photo.jpg"));
    assert!(d.confirm.is_empty(), "asked nothing");
    let until = Instant::now() + Duration::from_secs(10);
    while names(&d).contains(&"photo.jpg") {
        assert!(Instant::now() < until, "the move lands");
        std::thread::sleep(Duration::from_millis(5));
        d.wake();
    }
    assert!(t.path().join("Trash/files/photo.jpg").exists());
    assert!(!r.join("photo.jpg").exists());
    let after = d.scan.as_ref().unwrap().tree().node(ROOT).size;
    assert_eq!(before - after, 32 * 1024);

    fs::write(
        t.path().join("config.toml"),
        "[recycle_bin]\nconfirm = true\n",
    )
    .unwrap();
    pick(&mut d, "notes.txt");
    d.ask_trash(r.join("notes.txt"));
    let (b, path) = d.confirm.up().expect("asks first");
    assert_eq!(path, &r.join("notes.txt"));
    assert_eq!(
        b.fields.text,
        "Are you sure you want to send 'notes.txt' to the Recycle Bin?"
    );
    reprisefs::configfile::set_path(None);
    reprisefs::trash::set_home_trash(None);
    reprisefs::trash::set_mount_table(None);
}

/// The window in each look at 100% over three files, the second picked, its path fixed.
fn drawn() -> Vec<(reprisekit::interface::Interface, Canvas)> {
    use reprisekit::interface::Interface;
    let t = tempfile::tempdir().unwrap();
    files(t.path());
    let mut d = counted(t.path());
    d.list.view.selection.set_only(Some(1));
    d.form.set_text("path", "/home/u/Videos");
    [Interface::Classic, Interface::Blue, Interface::Glass]
        .into_iter()
        .map(|i| {
            let ctx = test_ctx(reprisekit::interfaces::theme_for(i));
            let mut c = Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32);
            d.draw_main(&mut c, &ctx);
            (i, c)
        })
        .collect()
}

/// With the test fonts only.
#[test]
fn the_window_sits_on_its_pixels_in_every_look() {
    reprisekit::pins::fonts();
    let mut pins = Vec::new();
    for (i, c) in drawn() {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for y in 0..c.height as i32 {
            for x in 0..c.width as i32 {
                for b in c.get(x, y).to_le_bytes() {
                    h = (h ^ b as u64).wrapping_mul(0x0100_0000_01b3);
                }
            }
        }
        pins.push((format!("{i:?}"), h));
    }
    let file = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/pins/window.txt");
    reprisekit::pins::check_file(std::path::Path::new(file), &pins);
}

/// Writes `target/diskusage-{classic,blue,glass}.png` for eyeballing; run with `--ignored`.
#[test]
#[ignore]
fn render_diskusage_preview() {
    for (i, c) in drawn() {
        let name = format!("{i:?}").to_lowercase();
        reprisekit::raster::write_preview(&c, &format!("diskusage-{name}.png"));
    }
}
