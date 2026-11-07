use super::*;
use repriseapp::{KeyInput, Mods, PointerInput};
use reprisekit::controls::imageview::Zoom;
use reprisekit::interface::Interface;
use reprisekit::textedit::keysym;
use reprisekit::theme::FontConfig;
use std::sync::Arc;

fn test_ctx() -> Ctx {
    Ctx {
        theme: Theme::default(),
        text: TextRenderer::load(&FontConfig::default()).into(),
        icons: reprisekit::icontheme::IconCache::new(reprisekit::icontheme::IconTheme::load(
            "test",
            &[],
        )),
        caret_on: true,
        size: (320, 240),
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

/// A `w` x `h` 24-bit BMP, each pixel's red its x and green its y.
fn bmp(w: u32, h: u32) -> Vec<u8> {
    let row = (w * 3).div_ceil(4) * 4;
    let size = 54 + row * h;
    let mut b = Vec::new();
    b.extend_from_slice(b"BM");
    b.extend_from_slice(&size.to_le_bytes());
    b.extend_from_slice(&[0, 0, 0, 0]);
    b.extend_from_slice(&54u32.to_le_bytes());
    b.extend_from_slice(&40u32.to_le_bytes());
    b.extend_from_slice(&w.to_le_bytes());
    b.extend_from_slice(&h.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&24u16.to_le_bytes());
    b.extend_from_slice(&[0; 24]);
    for y in (0..h).rev() {
        let mut line: Vec<u8> = (0..w).flat_map(|x| [0x40, y as u8, x as u8]).collect();
        line.resize(row as usize, 0);
        b.extend_from_slice(&line);
    }
    b
}

/// A folder of pictures named `names` (each `w` x `h` from its `bmp`) and a note.
fn folder(names: &[(&str, u32, u32)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (n, w, h) in names {
        std::fs::write(dir.path().join(n), bmp(*w, *h)).unwrap();
    }
    std::fs::write(dir.path().join("notes.txt"), "not a picture").unwrap();
    dir
}

/// The viewer on `path`, its picture decoded and its folder listed on the spot.
fn opened(path: PathBuf) -> Viewer {
    let mut v = Viewer::new(Some(path));
    v.load_current();
    v.list_folder();
    v.sync_menus();
    v
}

fn key(sym: u32, mods: Mods) -> Event {
    Event::Key(KeyInput {
        sym,
        utf8: None,
        mods,
        edit: None,
    })
}

fn press(v: &mut Viewer, sym: u32, ctx: &mut Ctx) -> Reply {
    v.event(MAIN, key(sym, Mods::default()), ctx)
}

fn ctrl(v: &mut Viewer, sym: u32, ctx: &mut Ctx) -> Reply {
    let mods = Mods {
        ctrl: true,
        ..Mods::default()
    };
    v.event(MAIN, key(sym, mods), ctx)
}

fn name(v: &Viewer) -> String {
    v.folder
        .current()
        .and_then(Path::file_name)
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[test]
fn the_arrows_home_end_and_the_wheel_walk_the_folder_in_name_order() {
    let dir = folder(&[("b10.bmp", 4, 4), ("b9.bmp", 4, 4), ("A.bmp", 4, 4)]);
    let mut v = opened(dir.path().join("b9.bmp"));
    let mut ctx = test_ctx();
    assert_eq!(
        (name(&v), v.folder.position()),
        ("b9.bmp".into(), "2 of 3".into())
    );
    press(&mut v, keysym::RIGHT, &mut ctx);
    assert_eq!(name(&v), "b10.bmp");
    press(&mut v, keysym::RIGHT, &mut ctx);
    assert_eq!(name(&v), "A.bmp", "past the last comes the first");
    press(&mut v, keysym::END, &mut ctx);
    assert_eq!(name(&v), "b10.bmp");
    press(&mut v, keysym::HOME, &mut ctx);
    assert_eq!(name(&v), "A.bmp");
    let wheel = |rows| {
        Event::Pointer(PointerInput::Scroll {
            x: 100,
            y: 100,
            rows,
        })
    };
    v.event(MAIN, wheel(3), &mut ctx);
    assert_eq!(name(&v), "b9.bmp");
    v.event(MAIN, wheel(-1), &mut ctx);
    assert_eq!(name(&v), "b9.bmp", "a third of a notch waits");
    v.event(MAIN, wheel(-2), &mut ctx);
    assert_eq!(name(&v), "A.bmp");
    assert_eq!(v.title(), "A.bmp - Image Viewer");
    assert!(v.picture.is_some());
}

#[test]
fn a_new_picture_opens_fitted_and_unturned() {
    let dir = folder(&[("a.bmp", 600, 400), ("b.bmp", 30, 20)]);
    let mut v = opened(dir.path().join("a.bmp"));
    let mut ctx = test_ctx();
    ctrl(&mut v, u32::from(b'r'), &mut ctx);
    press(&mut v, u32::from(b'+'), &mut ctx);
    assert_eq!(v.view.turns, 1);
    press(&mut v, keysym::RIGHT, &mut ctx);
    assert_eq!((v.view.zoom, v.view.turns), (Zoom::Fit, 0));
    assert_eq!(v.view.size, (30, 20));
}

#[test]
fn zoom_keys_step_fit_and_show_actual_size() {
    let dir = folder(&[("big.bmp", 1200, 900)]);
    let mut v = opened(dir.path().join("big.bmp"));
    let mut ctx = test_ctx();
    let lay = v.view_layout(&ctx);
    let fitted = lay.scale;
    assert!(fitted < 0.25 && fitted > 0.15, "{fitted}");
    assert_eq!(
        v.status_texts(&ctx)[4],
        format!("{}%", (fitted * 100.0).round())
    );
    press(&mut v, u32::from(b'+'), &mut ctx);
    assert_eq!(v.view.zoom, Zoom::Scale(0.25));
    press(&mut v, keysym::KP_SUBTRACT, &mut ctx);
    assert_eq!(v.view.zoom, Zoom::Scale(0.2));
    ctrl(&mut v, u32::from(b'1'), &mut ctx);
    assert_eq!(v.view.zoom, Zoom::Scale(1.0));
    assert_eq!(v.status_texts(&ctx)[4], "100%");
    let lay = v.view_layout(&ctx);
    assert!(lay.hscroll.is_some() && lay.vscroll.is_some());
    ctrl(&mut v, u32::from(b'0'), &mut ctx);
    assert_eq!(v.view.zoom, Zoom::Fit);
}

#[test]
fn ctrl_and_the_wheel_zoom_at_the_pointer() {
    let dir = folder(&[("big.bmp", 1200, 900), ("next.bmp", 4, 4)]);
    let mut v = opened(dir.path().join("big.bmp"));
    let mut ctx = test_ctx();
    v.event(MAIN, key(keysym::CONTROL_L, Mods::default()), &mut ctx);
    let wheel = |rows| Event::Pointer(PointerInput::Scroll { x: 50, y: 60, rows });
    v.event(MAIN, wheel(-3), &mut ctx);
    assert_eq!(v.view.zoom, Zoom::Scale(0.25));
    assert_eq!(name(&v), "big.bmp", "the wheel zooms instead of stepping");
    v.event(
        MAIN,
        Event::KeyUp(KeyInput {
            sym: keysym::CONTROL_L,
            utf8: None,
            mods: Mods::default(),
            edit: None,
        }),
        &mut ctx,
    );
    v.event(MAIN, wheel(3), &mut ctx);
    assert_eq!(name(&v), "next.bmp");
}

#[test]
fn rotating_turns_the_view_and_swaps_the_dimensions_shown() {
    let dir = folder(&[("wide.bmp", 60, 20)]);
    let mut v = opened(dir.path().join("wide.bmp"));
    let mut ctx = test_ctx();
    assert_eq!(v.status_texts(&ctx)[2], "60 x 20");
    ctrl(&mut v, u32::from(b'r'), &mut ctx);
    assert_eq!(v.status_texts(&ctx)[2], "20 x 60");
    ctrl(&mut v, u32::from(b'l'), &mut ctx);
    ctrl(&mut v, u32::from(b'l'), &mut ctx);
    assert_eq!(v.view.turns, 3);
    let on_disk = std::fs::read(dir.path().join("wide.bmp")).unwrap();
    assert_eq!(on_disk, bmp(60, 20), "the file is left as it was");
}

#[test]
fn a_zoom_under_100_percent_shows_the_scaled_copy() {
    let dir = folder(&[("big.bmp", 1200, 900)]);
    let mut v = opened(dir.path().join("big.bmp"));
    let ctx = test_ctx();
    let mut c = Canvas::new(320, 240);
    v.draw(MAIN, &mut c, &ctx);
    let lay = v.view_layout(&ctx);
    let (key, copy) = v.rendered.as_ref().expect("a copy for the fitted size");
    assert_eq!(key.size, lay.content);
    assert_eq!((copy.width as i32, copy.height as i32), lay.content);
}

#[test]
fn f11_and_a_double_click_go_full_screen_and_escape_comes_back() {
    let dir = folder(&[("a.bmp", 40, 30)]);
    let mut v = opened(dir.path().join("a.bmp"));
    let mut ctx = test_ctx();
    let r = press(&mut v, keysym::F11, &mut ctx);
    assert_eq!(r.fullscreen, Some(true));
    assert_eq!(v.box_rect(&ctx), Rect::new(0, 0, 320, 240));
    assert_eq!(
        press(&mut v, keysym::ESCAPE, &mut ctx).fullscreen,
        Some(false)
    );
    let click = |time| {
        Event::Pointer(PointerInput::Press {
            x: 160,
            y: 120,
            button: repriseapp::BTN_LEFT,
            time,
            mods: Mods::default(),
        })
    };
    v.event(MAIN, click(1000), &mut ctx);
    assert_eq!(v.event(MAIN, click(1100), &mut ctx).fullscreen, Some(true));
}

#[test]
fn a_slideshow_goes_full_screen_and_steps_every_three_seconds() {
    let dir = folder(&[("1.bmp", 4, 4), ("2.bmp", 4, 4)]);
    let mut v = opened(dir.path().join("1.bmp"));
    let mut ctx = test_ctx();
    assert_eq!(press(&mut v, keysym::F5, &mut ctx).fullscreen, Some(true));
    let due = v.slide_due.expect("a slide is due");
    v.due(due - Duration::from_millis(1));
    assert_eq!(name(&v), "1.bmp");
    v.due(due);
    assert_eq!(name(&v), "2.bmp");
    assert_eq!(v.slide_due, Some(due + SLIDE));
    v.due(due + SLIDE);
    assert_eq!(name(&v), "1.bmp", "and round again");
    assert_eq!(
        press(&mut v, keysym::ESCAPE, &mut ctx).fullscreen,
        Some(false)
    );
    assert_eq!(v.slide_due, None);
}

#[test]
fn a_moving_picture_steps_its_frames_as_they_come_due() {
    let mut v = Viewer::new(None);
    let frame = |ms| (Arc::new(Canvas::new(2, 2)), ms);
    v.generation = 1;
    v.handle(Msg::Opened {
        generation: 1,
        picture: Ok(Picture {
            frames: vec![frame(100), frame(250)],
            bytes: 10,
        }),
    });
    let first = v.frame_due.expect("the first frame ends");
    v.due(first);
    assert_eq!(v.frame, 1);
    assert_eq!(v.frame_due, Some(first + Duration::from_millis(250)));
    v.due(first + Duration::from_millis(250));
    assert_eq!(v.frame, 0);
    v.handle(Msg::Opened {
        generation: 0,
        picture: Ok(Picture {
            frames: vec![frame(0)],
            bytes: 1,
        }),
    });
    assert_eq!(
        v.picture.as_ref().map(|p| p.frames.len()),
        Some(2),
        "a stale decode is dropped"
    );
}

#[test]
fn delete_asks_then_sends_the_picture_to_the_recycle_bin_and_shows_the_next() {
    let root = tempfile::tempdir().unwrap();
    let pics = root.path().join("Pictures");
    std::fs::create_dir(&pics).unwrap();
    for n in ["a.bmp", "b.bmp"] {
        std::fs::write(pics.join(n), bmp(4, 4)).unwrap();
    }
    let trash = root.path().join("Trash");
    reprisefs::trash::set_home_trash(Some(trash.clone()));
    reprisefs::trash::set_mount_table(Some("/dev/null".into()));
    reprisefs::bincap::set_config_path(Some(root.path().join("config.toml")));
    let mut v = opened(pics.join("a.bmp"));
    let mut ctx = test_ctx();
    press(&mut v, keysym::DELETE, &mut ctx);
    let (b, path, erase) = v.delete.as_ref().expect("the prompt is up");
    assert_eq!(
        (path.as_path(), *erase),
        (pics.join("a.bmp").as_path(), false)
    );
    assert_eq!(b.title, "Confirm File Delete");
    assert_eq!(
        b.fields.text,
        "Are you sure you want to send 'a.bmp' to the Recycle Bin?"
    );
    assert_eq!(b.fields.default, 0, "Enter is Yes");
    v.event(DELETE, key(keysym::RETURN, Mods::default()), &mut ctx);
    reprisefs::trash::set_home_trash(None);
    reprisefs::trash::set_mount_table(None);
    reprisefs::bincap::set_config_path(None);
    assert!(v.delete.is_none());
    assert!(!pics.join("a.bmp").exists());
    assert!(trash.join("files/a.bmp").exists());
    assert_eq!((name(&v), v.folder.pictures.len()), ("b.bmp".into(), 1));
    assert!(v.picture.is_some());
}

#[test]
fn set_as_desktop_background_points_the_wallpaper_at_the_picture() {
    let dir = folder(&[("dunes.bmp", 4, 4)]);
    let mut v = opened(dir.path().join("dunes.bmp"));
    v.config = dir.path().join("config.toml");
    std::fs::write(
        &v.config,
        "# mine\n[wallpaper]\nslideshow = \"~/Pictures\"\nposition = \"center\"\n",
    )
    .unwrap();
    let ctx = test_ctx();
    v.command(menu::title::FILE, menu::row::BACKGROUND, &ctx);
    let text = std::fs::read_to_string(&v.config).unwrap();
    let cfg = repriseconf::config::Config::parse(&text).unwrap();
    assert_eq!(
        cfg.wallpaper.image.as_deref(),
        Some(dir.path().join("dunes.bmp").as_path())
    );
    assert_eq!(cfg.wallpaper.slideshow, None);
    assert!(text.starts_with("# mine\n") && text.contains("position = \"center\""));
}

#[test]
fn a_format_the_desktop_cannot_show_greys_set_as_background() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("a.webp");
    std::fs::write(&p, b"").unwrap();
    let mut v = Viewer::new(Some(p));
    v.picture = Some(Picture {
        frames: vec![(Arc::new(Canvas::new(1, 1)), 0)],
        bytes: 0,
    });
    v.sync_menus();
    let row = &v.bar.menus[menu::title::FILE][menu::row::BACKGROUND];
    assert!(matches!(row, MenuItem::Entry { enabled: false, .. }));
}

#[test]
fn a_file_that_is_no_picture_says_so_in_the_box() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("broken.png");
    std::fs::write(&p, b"not a picture").unwrap();
    let v = opened(p);
    assert_eq!(v.blank, Blank::Failed(load::CANNOT_OPEN.into()));
    assert_eq!(v.blank_text(), load::CANNOT_OPEN);
}

fn fnv(c: &Canvas) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325_u64;
    for p in &c.pixels {
        for b in p.to_le_bytes() {
            h = (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3);
        }
    }
    h
}

#[test]
fn the_main_window_is_pinned_per_interface_and_scale() {
    reprisekit::pins::fonts();
    let dir = folder(&[("a.bmp", 200, 120), ("b.bmp", 4, 4)]);
    let mut got = Vec::new();
    for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
        for s in [1, 2] {
            let mut v = opened(dir.path().join("a.bmp"));
            let mut ctx = test_ctx();
            ctx.theme = reprisekit::interfaces::theme_for(i);
            ctrl(&mut v, u32::from(b'r'), &mut ctx);
            ctrl(&mut v, u32::from(b'1'), &mut ctx);
            let mut c = Canvas::with_scale(320, 240, s);
            v.follow_scale(MAIN, s, &mut ctx);
            ctx.at_scale(s, |ctx| v.draw(MAIN, &mut c, ctx));
            got.push((format!("{i:?}@{s}"), fnv(&c)));
        }
    }
    let want: [(&str, u64); 6] = [
        ("Classic@1", 0xad48_f3a9_99c1_ff50),
        ("Classic@2", 0x5a5d_d328_abb7_7165),
        ("Blue@1", 0x9849_84eb_d85e_a379),
        ("Blue@2", 0x14d7_6ff6_50e8_9052),
        ("Glass@1", 0x56f0_ac80_7e5f_f7a5),
        ("Glass@2", 0xd1d9_2189_8e1b_c85b),
    ];
    let want: Vec<(String, u64)> = want.iter().map(|(k, h)| (k.to_string(), *h)).collect();
    assert_eq!(got, want);
}
