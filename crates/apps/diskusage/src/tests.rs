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
    d.cleaned = Some(Poster::channel(waker.clone()));
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
    let ctx = test_ctx(reprisekit::theme::Theme::default());
    let mut d = counted(r);
    assert_eq!(names(&d), ["a", "g"]);
    pick(&mut d, "a");
    d.open_picked();
    assert_eq!(d.at, r.join("a"));
    assert_eq!(names(&d), ["b"]);
    pick(&mut d, "b");
    d.open_picked();
    assert_eq!(d.at, r.join("a/b"));
    d.button("back", &ctx);
    assert_eq!(d.at, r.join("a"));
    d.button("up", &ctx);
    assert_eq!(d.at, r);
    assert_eq!(d.root, r, "inside the count: no new one");
    d.button("up", &ctx);
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

/// Clean Up's places under `root`: a cache with a file and a thumbnail, and a bin holding one
/// file.
fn cleanup_places(root: &Path) -> Places {
    let cache = root.join("cache");
    fs::create_dir_all(cache.join("thumbnails/normal")).unwrap();
    fs::write(cache.join("thumbnails/normal/a.png"), vec![1u8; 8192]).unwrap();
    fs::write(cache.join("blob"), vec![1u8; 16384]).unwrap();
    let bin = root.join("Trash");
    fs::create_dir(&bin).unwrap();
    let gone = root.join("gone.txt");
    fs::write(&gone, vec![1u8; 4096]).unwrap();
    reprisefs::trash::trash_in(&bin, &gone).unwrap();
    Places {
        cache: Some(cache),
        bins: Some(reprisefs::trash::Bins::new(bin, Vec::new())),
        flatpak: None,
        flatpak_user: None,
        coredumpctl: None,
        uid: std::os::unix::fs::MetadataExt::uid(&fs::metadata(root).unwrap()),
        home: Some(root.join("home")),
        kept: Vec::new(),
    }
}

/// Wakes `d` until `done` holds.
fn wake_until(d: &mut DiskUsage, what: &str, done: impl Fn(&DiskUsage) -> bool) {
    let until = Instant::now() + Duration::from_secs(10);
    while !done(d) {
        assert!(Instant::now() < until, "{what}");
        std::thread::sleep(Duration::from_millis(5));
        d.wake();
    }
}

fn counted_cleanup(d: &mut DiskUsage, places: Places) {
    d.open_cleanup(places);
    wake_until(d, "every part is counted", |d| {
        (d.cleanup.as_ref()).is_some_and(|c| c.parts.iter().all(|(_, f)| f.is_some()))
    });
}

/// Whether button or check box `id` takes input.
fn on(f: &Form, id: &str) -> bool {
    use reprisekit::controls::form::Control;
    matches!(
        f.item(id).map(|i| &i.control),
        Some(Control::Button { enabled: true, .. } | Control::Check { enabled: true, .. })
    )
}

fn key(sym: u32) -> Event {
    Event::Key(KeyInput {
        sym,
        utf8: None,
        mods: Default::default(),
        edit: None,
    })
}

#[test]
fn clean_up_removes_only_the_checked_parts_and_only_after_a_yes() {
    let t = tempfile::tempdir().unwrap();
    let data = t.path().join("data");
    fs::create_dir(&data).unwrap();
    let mut d = counted(&data);
    let cache = t.path().join("cache");
    counted_cleanup(&mut d, cleanup_places(t.path()));
    let c = d.cleanup.as_ref().unwrap();
    let checked = |id| c.form.checked(id);
    assert_eq!(checked("recycle-bin"), Some(true));
    assert_eq!(checked("thumbnails"), Some(true));
    assert_eq!(
        checked("caches"),
        Some(false),
        "the caches wait for a check"
    );
    let text = c.confirm_text();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        lines[0],
        "Are you sure you want to permanently remove these files?"
    );
    assert!(lines[2].starts_with("Recycle Bin: "));
    let thumbs = cache.join("thumbnails");
    assert!(
        lines[3].starts_with(&format!("Thumbnails ({}): ", thumbs.display())),
        "a folder part names its folder: {}",
        lines[3]
    );
    assert_eq!(lines.len(), 4, "the caches are not named: {text}");

    d.ask_clean();
    let win = d.clean_ask.win().expect("Clean Up asks first");
    assert_eq!(repriseapp::Extended::parent(&d, win), Some(CLEANUP));
    let mut ctx = test_ctx(reprisekit::theme::Theme::default());
    d.event(win, key(keysym::RETURN), &mut ctx);
    assert!(d.clean_ask.is_empty(), "Enter answers No");
    assert!(d.cleanup.is_some(), "No goes back to Clean Up");
    assert!(cache.join("thumbnails/normal/a.png").exists());
    assert_eq!(
        fs::read_dir(t.path().join("Trash/files")).unwrap().count(),
        1
    );

    d.ask_clean();
    let parts = d.clean_ask.close().unwrap();
    let freed = parts.iter().map(|(_, f)| f.size).sum();
    d.clean_answered(parts, Some(0));
    assert!(d.cleanup.is_none() && d.cleaning);
    assert!(!on(&d.form, "cleanup"), "one removal at a time");
    wake_until(&mut d, "the removal ends", |d| !d.cleaning);
    assert_eq!(fs::read_dir(cache.join("thumbnails")).unwrap().count(), 0);
    assert_eq!(
        fs::read_dir(t.path().join("Trash/files")).unwrap().count(),
        0
    );
    assert!(cache.join("blob").exists(), "an unchecked part stays");
    assert!(!d.note.is_shown());
    let (b, _) = d.cleaned_box.up().expect("a box says what went");
    assert_eq!(
        b.fields.text,
        format!("Clean Up freed {}.", reprisefs::props::short_size(freed))
    );
}

#[test]
fn the_box_after_a_removal_names_the_parts_that_failed() {
    let t = tempfile::tempdir().unwrap();
    let data = t.path().join("data");
    fs::create_dir(&data).unwrap();
    let mut d = counted(&data);
    counted_cleanup(&mut d, cleanup_places(t.path()));
    let c = d.cleanup.as_ref().unwrap();
    let mut parts: Vec<(Part, Found)> = (c.picked().into_iter())
        .map(|(p, f)| (p, f.clone()))
        .collect();
    let freed = parts.iter().map(|(_, f)| f.size).sum();
    let dumps = Found {
        size: 1 << 20,
        removable: false,
        folder: None,
        items: Vec::new(),
    };
    parts.push((Part::Coredumps, dumps));
    d.clean_answered(parts, Some(0));
    wake_until(&mut d, "the removal ends", |d| !d.cleaning);
    let (b, _) = d.cleaned_box.up().expect("a box says what went");
    let text = &b.fields.text;
    let lines: Vec<&str> = text.lines().collect();
    let size = reprisefs::props::short_size(freed);
    assert_eq!(
        lines[..4],
        [
            format!("Clean Up freed {size}.").as_str(),
            "",
            "Some files could not be removed.",
            ""
        ],
        "a failed crash dump part frees nothing: {text}"
    );
    assert!(lines[4].starts_with("Crash dumps: "), "{text}");
}

#[test]
fn a_part_with_nothing_to_remove_greys_and_ok_waits_for_a_check() {
    let t = tempfile::tempdir().unwrap();
    let mut d = counted(t.path());
    fs::create_dir(t.path().join("p")).unwrap();
    let places = Places {
        cache: Some(t.path().join("no-cache")),
        coredumpctl: Some("/bin/false".into()),
        ..cleanup_places(&t.path().join("p"))
    };
    counted_cleanup(&mut d, places);
    let c = d.cleanup.as_mut().unwrap();
    assert!(!on(&c.form, "thumbnails"));
    assert_eq!(c.form.checked("thumbnails"), Some(false));
    assert!(on(&c.form, cleanup::OK));
    c.form.set_check("recycle-bin", false);
    c.sync();
    assert!(!on(&c.form, cleanup::OK));
    assert!(c.picked().is_empty());
    let u = reprisekit::theme::TextUnits::new(26, 13);
    let crash_row = u.len(62 + 3 * 20 + 6);
    assert!(!on(&c.form, "coredumps"));
    assert!(
        c.point(u.len(40), crash_row, u),
        "the crash dumps' row at 200%"
    );
    assert!(
        format!("{:?}", c.form.item("about").unwrap().control).contains("The system keeps them"),
        "a greyed row still tells why"
    );
    c.sync();
    assert!(
        format!("{:?}", c.form.item("about").unwrap().control).contains("The system keeps them"),
        "a check with no focus move keeps the row pointed at"
    );
}

#[test]
fn the_confirm_box_names_each_runtime_flatpak_would_remove() {
    use reprisefs::cleanup::Found;
    let t = tempfile::tempdir().unwrap();
    let places = Places {
        flatpak: Some("/bin/false".into()),
        ..cleanup_places(t.path())
    };
    let mut c = Cleanup::new(places, 1);
    for p in [Part::RecycleBin, Part::Thumbnails, Part::Caches] {
        c.found(
            p,
            Found {
                size: 0,
                removable: true,
                folder: None,
                items: Vec::new(),
            },
        );
    }
    let runtime = reprisefs::cleanup::Runtime {
        id: "org.example.Platform".into(),
        arch: "x86_64".into(),
        branch: "1".into(),
        system: true,
    };
    c.found(
        Part::Flatpak,
        Found {
            size: 2048,
            removable: true,
            folder: None,
            items: vec![runtime],
        },
    );
    assert_eq!(
        c.form.checked("flatpak"),
        Some(false),
        "runtimes wait for a check"
    );
    c.form.set_check("flatpak", true);
    assert_eq!(
        c.confirm_text(),
        "Are you sure you want to permanently remove these files?\n\n\
         Unused Flatpak runtimes: 2.00 KB\n- org.example.Platform/x86_64/1 (system installation)"
    );
}

/// The window in each look at 100% over three files, the second picked, its path fixed, and the
/// Clean Up box over it with every part counted.
fn drawn() -> Vec<(reprisekit::interface::Interface, Canvas, Canvas)> {
    use reprisefs::cleanup::Found;
    use reprisekit::interface::Interface;
    let t = tempfile::tempdir().unwrap();
    files(t.path());
    let mut d = counted(t.path());
    d.list.view.selection.set_only(Some(1));
    d.form.set_text("path", "/home/u/Videos");
    let places = Places {
        flatpak: Some("/usr/bin/flatpak".into()),
        coredumpctl: Some("/usr/bin/coredumpctl".into()),
        ..cleanup_places(t.path())
    };
    let mut box_ = Cleanup::new(places, 1);
    let found = |size: u64, removable: bool| Found {
        size,
        removable,
        folder: None,
        items: Vec::new(),
    };
    box_.found(Part::RecycleBin, found(48 << 20, true));
    box_.found(Part::Thumbnails, found(12 << 20, true));
    box_.found(Part::Caches, found(1536 << 20, true));
    box_.found(Part::Coredumps, found(300 << 20, false));
    box_.found(Part::Flatpak, found(0, true));
    let size = cleanup::client(box_.parts.len());
    d.cleanup = Some(box_);
    [Interface::Classic, Interface::Blue, Interface::Glass]
        .into_iter()
        .map(|i| {
            let ctx = test_ctx(reprisekit::interfaces::theme_for(i));
            let mut c = Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32);
            d.draw_main(&mut c, &ctx);
            let mut b = Canvas::new(size.0 as u32, size.1 as u32);
            d.draw_cleanup(&mut b, &ctx);
            (i, c, b)
        })
        .collect()
}

fn hash(c: &Canvas) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for y in 0..c.height as i32 {
        for x in 0..c.width as i32 {
            for b in c.get(x, y).to_le_bytes() {
                h = (h ^ b as u64).wrapping_mul(0x0100_0000_01b3);
            }
        }
    }
    h
}

/// With the test fonts only.
#[test]
fn the_window_and_clean_up_sit_on_their_pixels_in_every_look() {
    reprisekit::pins::fonts();
    let (mut window, mut boxes) = (Vec::new(), Vec::new());
    for (i, c, b) in drawn() {
        window.push((format!("{i:?}"), hash(&c)));
        boxes.push((format!("{i:?}"), hash(&b)));
    }
    let dir = reprisekit::crate_dir!().join("tests/pins");
    reprisekit::pins::check_file(&dir.join("window.txt"), &window);
    reprisekit::pins::check_file(&dir.join("cleanup.txt"), &boxes);
}

/// The list fetches its icons at 16 px at 100% text and 32 px at 200%, and draws them whole.
#[test]
fn the_list_icons_grow_with_the_text() {
    use reprisekit::theme::Color;
    const MARK: Color = Color::rgb(1, 2, 3);
    reprisekit::pins::fonts();
    let t = tempfile::tempdir().unwrap();
    files(t.path());
    fs::create_dir(t.path().join("folder")).unwrap();
    let mut d = counted(t.path());
    d.form.set_text("path", "/home/u/Videos");
    let mut pins = Vec::new();
    for (pct, px) in [(100, 16), (200, 32)] {
        let theme = reprisekit::interfaces::theme_for(reprisekit::interface::Interface::Classic)
            .with_text_size(pct);
        let mut ctx = test_ctx(theme);
        ctx.size = ctx.theme.text.size(CLIENT);
        let mark = |px: i32| {
            let mut c = Canvas::new(px as u32, px as u32);
            c.fill_rect(c.bounds(), MARK);
            Some(c)
        };
        let names = ["folder", "text-x-generic", "drive-harddisk"];
        let icons = (names.iter())
            .flat_map(|n| [16, 32].map(|px| (n.to_string(), px, mark(px))))
            .collect();
        let cache_theme = ctx.icons.theme();
        ctx.icons.put_icons(&cache_theme, icons);
        d.load_icons(&mut ctx);
        let mut c = Canvas::new(ctx.size.0 as u32, ctx.size.1 as u32);
        d.draw_main(&mut c, &ctx);
        let marked = (0..c.height as i32)
            .flat_map(|y| (0..c.width as i32).map(move |x| (x, y)))
            .filter(|&(x, y)| c.get(x, y) == MARK.argb())
            .count();
        assert_eq!(
            marked,
            4 * (px * px) as usize,
            "four {px} px icons at {pct}%"
        );
        pins.push((format!("text {pct}"), hash(&c)));
    }
    let dir = reprisekit::crate_dir!().join("tests/pins");
    reprisekit::pins::check_file(&dir.join("icons.txt"), &pins);
}

/// Writes `target/diskusage-{classic,blue,glass}.png` and `cleanup-*.png` for eyeballing; run
/// with `--ignored`.
#[test]
#[ignore]
fn render_diskusage_preview() {
    for (i, c, b) in drawn() {
        let name = format!("{i:?}").to_lowercase();
        reprisekit::raster::write_preview(&c, &format!("diskusage-{name}.png"));
        reprisekit::raster::write_preview(&b, &format!("cleanup-{name}.png"));
    }
}

#[test]
fn browse_shows_the_folder_its_dialog_picks() {
    let t = tempfile::tempdir().unwrap();
    let r = t.path();
    fs::create_dir_all(r.join("a")).unwrap();
    fs::create_dir_all(r.join("elsewhere/x")).unwrap();
    let mut ctx = test_ctx(reprisekit::theme::Theme::default());
    let mut d = counted(&r.join("a"));
    d.button("browse", &ctx);
    let dlg = d.pick.as_mut().expect("Browse... opens the folder dialog");
    assert_eq!(dlg.mode(), DlgMode::Folder);
    assert!(d.children().iter().any(|c| c.id == PICK && c.modal));
    let dlg = d.pick.as_mut().unwrap();
    dlg.set_file_name(&r.join("elsewhere").display().to_string());
    let enter = Event::Key(KeyInput {
        sym: keysym::RETURN,
        utf8: None,
        mods: Default::default(),
        edit: None,
    });
    d.event(PICK, enter, &mut ctx);
    assert!(d.pick.is_none(), "the dialog closes on the pick");
    assert_eq!(d.at, r.join("elsewhere"));
    d.button("back", &ctx);
    assert_eq!(d.at, r.join("a"), "Back returns to the folder before");
}

#[test]
fn cleanup_before_the_folder_opens_clean_up() {
    let args = |a: &[&str]| parse_args(a.iter().map(OsString::from).collect());
    assert_eq!(args(&["--cleanup", "/home"]), (true, Some("/home".into())));
    assert_eq!(args(&["/home"]), (false, Some("/home".into())));
    assert_eq!(args(&["--cleanup"]), (true, None));
    assert_eq!(
        args(&["home"]),
        (false, None),
        "a relative folder is not taken"
    );
}
