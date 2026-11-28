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
    assert!(fitted < 0.2 && fitted > 0.15, "{fitted}");
    assert_eq!(
        v.status_texts(&ctx)[4],
        format!("{}%", (fitted * 100.0).round())
    );
    press(&mut v, u32::from(b'+'), &mut ctx);
    assert_eq!(v.view.zoom, Zoom::Scale(0.2));
    press(&mut v, keysym::KP_SUBTRACT, &mut ctx);
    assert_eq!(v.view.zoom, Zoom::Scale(0.15));
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
    assert_eq!(v.view.zoom, Zoom::Scale(0.2));
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
        copied: None,
        generation: 1,
        picture: Ok(Picture {
            frames: vec![frame(100), frame(250)],
            bytes: 10,
            dpi: None,
        }),
    });
    let first = v.frame_due.expect("the first frame ends");
    v.due(first);
    assert_eq!(v.frame, 1);
    assert_eq!(v.frame_due, Some(first + Duration::from_millis(250)));
    v.due(first + Duration::from_millis(250));
    assert_eq!(v.frame, 0);
    v.handle(Msg::Opened {
        copied: None,
        generation: 0,
        picture: Ok(Picture {
            frames: vec![frame(0)],
            bytes: 1,
            dpi: None,
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
    reprisefs::configfile::set_path(Some(root.path().join("config.toml")));
    let mut v = opened(pics.join("a.bmp"));
    let mut ctx = test_ctx();
    press(&mut v, keysym::DELETE, &mut ctx);
    let (b, (path, erase)) = v.delete.up().expect("the prompt is up");
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
    let win = v.delete.win().expect("its window");
    v.event(win, key(keysym::RETURN, Mods::default()), &mut ctx);
    reprisefs::trash::set_home_trash(None);
    reprisefs::trash::set_mount_table(None);
    reprisefs::configfile::set_path(None);
    assert!(v.delete.is_empty());
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
        dpi: None,
    });
    v.sync_menus();
    let row = &v.bar.menus[menu::title::FILE][menu::row::BACKGROUND];
    assert!(matches!(row, MenuItem::Entry { enabled: false, .. }));
}

#[test]
fn edit_opens_the_picture_in_paint_and_show_in_folder_selects_it_in_explorer() {
    let dir = folder(&[("dunes.bmp", 4, 4)]);
    let path = dir.path().join("dunes.bmp");
    let mut v = opened(path.clone());
    let ctx = test_ctx();
    let edit = v.command(menu::title::FILE, menu::row::EDIT_PICTURE, &ctx);
    assert_eq!(
        edit.spawn,
        [vec![
            std::ffi::OsString::from("reprise-paint"),
            path.clone().into()
        ]]
    );
    let shown = v.command(menu::title::FILE, menu::row::SHOW_IN_FOLDER, &ctx);
    assert_eq!(
        shown.spawn,
        [reprisefs::apps::open_location_argv(&path)],
        "Explorer opens on the folder with the picture selected"
    );
}

#[test]
fn edit_and_show_in_folder_grey_without_a_picture() {
    let mut v = Viewer::new(None);
    v.sync_menus();
    for row in [menu::row::EDIT_PICTURE, menu::row::SHOW_IN_FOLDER] {
        let item = &v.bar.menus[menu::title::FILE][row];
        assert!(matches!(item, MenuItem::Entry { enabled: false, .. }));
    }
}

#[test]
fn the_context_menu_opens_on_a_right_click_and_the_menu_key_and_takes_access_keys() {
    let dir = folder(&[("dunes.bmp", 4, 4)]);
    let path = dir.path().join("dunes.bmp");
    let mut v = opened(path.clone());
    let mut ctx = test_ctx();
    let right = Event::Pointer(PointerInput::Press {
        x: 160,
        y: 120,
        button: repriseapp::BTN_RIGHT,
        time: 0,
        mods: Mods::default(),
    });
    v.event(MAIN, right, &mut ctx);
    assert!(
        v.popup(MAIN, &ctx).is_some(),
        "a right-click opens the menu"
    );
    let f = Event::Key(KeyInput {
        sym: u32::from(b'f'),
        utf8: Some("f".into()),
        mods: Mods::default(),
        edit: None,
    });
    let shown = v.event(MAIN, f, &mut ctx);
    assert_eq!(shown.spawn, [reprisefs::apps::open_location_argv(&path)]);
    assert!(v.popup(MAIN, &ctx).is_none(), "choosing a row closes it");
    press(&mut v, keysym::MENU, &mut ctx);
    assert!(v.popup(MAIN, &ctx).is_some(), "the Menu key opens it");
    let e = Event::Key(KeyInput {
        sym: u32::from(b'e'),
        utf8: Some("e".into()),
        mods: Mods::default(),
        edit: None,
    });
    let edit = v.event(MAIN, e, &mut ctx);
    assert_eq!(
        edit.spawn,
        [vec![std::ffi::OsString::from("reprise-paint"), path.into()]]
    );
}

#[test]
fn a_file_that_is_no_picture_says_so_in_the_box() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("broken.png");
    std::fs::write(&p, b"not a picture").unwrap();
    let v = opened(p);
    assert_eq!(v.blank, Blank::Failed(load::cannot_open()));
    assert_eq!(v.blank_text(), load::cannot_open());
}

use reprisekit::pins::fnv;

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
    let file = reprisekit::crate_dir!().join("tests/pins/main_window.txt");
    reprisekit::pins::check_file(&file, &got);
}

/// Toolbar buttons by their place in the strip.
const PREVIOUS: usize = 0;
const NEXT: usize = 1;
const ZOOM_IN: usize = 3;
const ROTATE_CW: usize = 9;
const SLIDESHOW: usize = 11;
const DELETE_BUTTON: usize = 13;

fn button_at(v: &Viewer, i: usize, ctx: &Ctx) -> (i32, i32) {
    let r = v.chrome(ctx).expect("a toolbar").buttons[i];
    (r.x + r.width / 2, r.y + r.height / 2)
}

fn motion(v: &mut Viewer, (x, y): (i32, i32), ctx: &mut Ctx) -> Reply {
    v.event(MAIN, Event::Pointer(PointerInput::Motion { x, y }), ctx)
}

/// A left click on toolbar button `i`.
fn click(v: &mut Viewer, i: usize, ctx: &mut Ctx) -> Reply {
    let (x, y) = button_at(v, i, ctx);
    let (button, mods) = (repriseapp::BTN_LEFT, Mods::default());
    let down = PointerInput::Press {
        x,
        y,
        button,
        time: 0,
        mods,
    };
    v.event(MAIN, Event::Pointer(down), ctx);
    let up = PointerInput::Release { x, y, button, mods };
    v.event(MAIN, Event::Pointer(up), ctx)
}

/// The viewer on `path` in a window whose toolbar has its icons.
fn tooled(path: PathBuf, ctx: &mut Ctx) -> Viewer {
    let mut v = opened(path);
    v.follow_scale(MAIN, 1, ctx);
    v
}

#[test]
fn the_toolbar_buttons_run_their_menu_rows() {
    let dir = folder(&[("a.bmp", 60, 20), ("b.bmp", 4, 4)]);
    let mut ctx = test_ctx();
    let mut v = tooled(dir.path().join("a.bmp"), &mut ctx);
    click(&mut v, NEXT, &mut ctx);
    assert_eq!(name(&v), "b.bmp");
    click(&mut v, PREVIOUS, &mut ctx);
    assert_eq!(name(&v), "a.bmp");
    click(&mut v, ROTATE_CW, &mut ctx);
    assert_eq!(v.view.turns, 1);
    let lay = v.view_layout(&ctx);
    let before = lay.scale;
    click(&mut v, ZOOM_IN, &mut ctx);
    assert!(v.view_layout(&ctx).scale > before, "Zoom In zooms in");
    click(&mut v, DELETE_BUTTON, &mut ctx);
    assert!(v.delete.up().is_some(), "Delete asks first");
}

#[test]
fn a_press_dragged_off_its_button_runs_nothing() {
    let dir = folder(&[("a.bmp", 4, 4), ("b.bmp", 4, 4)]);
    let mut ctx = test_ctx();
    let mut v = tooled(dir.path().join("a.bmp"), &mut ctx);
    let (x, y) = button_at(&v, NEXT, &ctx);
    let (button, mods) = (repriseapp::BTN_LEFT, Mods::default());
    let down = PointerInput::Press {
        x,
        y,
        button,
        time: 0,
        mods,
    };
    v.event(MAIN, Event::Pointer(down), &mut ctx);
    assert_eq!(v.tools.bar.pressed, Some(NEXT));
    let up = PointerInput::Release {
        x: 160,
        y: 150,
        button,
        mods,
    };
    v.event(MAIN, Event::Pointer(up), &mut ctx);
    assert_eq!((name(&v), v.tools.bar.pressed), ("a.bmp".into(), None));
}

#[test]
fn a_button_greys_with_its_row_and_takes_no_press() {
    let dir = folder(&[("only.bmp", 4, 4)]);
    let mut ctx = test_ctx();
    let mut v = tooled(dir.path().join("only.bmp"), &mut ctx);
    let enabled = |v: &Viewer, i: usize| match &v.tools.bar.items[i] {
        reprisekit::controls::toolbar::Item::Button { enabled, .. } => *enabled,
        _ => panic!("button {i} is a separator"),
    };
    assert!(!enabled(&v, PREVIOUS) && !enabled(&v, NEXT), "one picture");
    assert!(enabled(&v, ZOOM_IN) && enabled(&v, SLIDESHOW));
    let mut tree = Tree::new(reprisekit::access::node(
        reprisekit::access::accesskit::Role::Window,
        Rect::new(0, 0, 320, 240),
    ));
    v.access(MAIN, &mut tree, &ctx);
    let button = reprisekit::access::accesskit::Role::Button;
    assert!(tree.find(button, "Rotate Clockwise").is_some());
    assert!(tree.find(button, "Next Picture").is_some());
    let r = click(&mut v, NEXT, &mut ctx);
    assert_eq!((name(&v), v.tools.bar.pressed), ("only.bmp".into(), None));
    assert!(!r.quit);
    v.event(MAIN, key(keysym::F11, Mods::default()), &mut ctx);
    assert!(v.chrome(&ctx).is_none(), "full screen shows no toolbar");
}

#[test]
fn a_rest_on_a_button_brings_up_its_name_and_key_and_a_key_takes_it_down() {
    let dir = folder(&[("a.bmp", 4, 4), ("b.bmp", 4, 4)]);
    let mut ctx = test_ctx();
    let mut v = tooled(dir.path().join("a.bmp"), &mut ctx);
    let at = button_at(&v, ZOOM_IN, &ctx);
    let r = motion(&mut v, at, &mut ctx);
    assert!(r.tick, "the rest ticks until the tip is due");
    assert_eq!(v.tools.bar.hot, Some(ZOOM_IN));
    assert!(v.event(MAIN, Event::Tick, &mut ctx).tick, "not due yet");
    let rest = v.tools.rest.as_mut().unwrap();
    rest.since -= Duration::from_secs(1);
    assert!(v.event(MAIN, Event::Tick, &mut ctx).redraw);
    assert!(v.tools.tip_up);
    assert_eq!(v.tip_text(ZOOM_IN).as_deref(), Some("Zoom In (+)"));
    assert_eq!(v.tip_text(SLIDESHOW).as_deref(), Some("Slide Show (F5)"));
    let r = motion(&mut v, (at.0 + 1, at.1), &mut ctx);
    assert!(
        v.tools.tip_up && !r.tick,
        "the tip stays while the pointer stays"
    );
    v.event(MAIN, key(keysym::CONTROL_L, Mods::default()), &mut ctx);
    assert!(!v.tools.tip_up && v.tools.rest.is_none());
}

#[test]
fn the_tip_comes_up_under_the_pointer_at_scale_2() {
    let dir = folder(&[("a.bmp", 4, 4), ("b.bmp", 4, 4)]);
    let mut ctx = test_ctx();
    let mut v = tooled(dir.path().join("a.bmp"), &mut ctx);
    let at = button_at(&v, ZOOM_IN, &ctx);
    motion(&mut v, at, &mut ctx);
    let draw = |v: &mut Viewer, ctx: &mut Ctx| {
        let mut c = Canvas::with_scale(320, 240, 2);
        ctx.at_scale(2, |ctx| v.draw(MAIN, &mut c, ctx));
        c
    };
    let bare = draw(&mut v, &mut ctx);
    v.tools.tip_up = true;
    let tipped = draw(&mut v, &mut ctx);
    let w = bare.device_size().0 as usize;
    let changed: Vec<(usize, usize)> = (bare.pixels.iter().zip(&tipped.pixels))
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .map(|(i, _)| (i % w, i / w))
        .collect();
    let left = changed.iter().map(|p| p.0).min().expect("the tip drew");
    let top = changed.iter().map(|p| p.1).min().unwrap();
    let below = reprisekit::controls::tooltip::INFO_BELOW_POINTER;
    assert_eq!(
        (left as i32, top as i32),
        ((at.0 - 1) * 2, (at.1 + below) * 2),
        "the tip's corner is under the pointer in device px"
    );
}

#[test]
fn the_toolbar_is_pinned_per_interface_and_scale() {
    reprisekit::pins::fonts();
    let dir = folder(&[("only.bmp", 200, 120)]);
    let mut got = Vec::new();
    for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
        for s in [1, 2] {
            let mut ctx = test_ctx();
            ctx.theme = reprisekit::interfaces::theme_for(i);
            let mut v = tooled(dir.path().join("only.bmp"), &mut ctx);
            let icon = reprisekit::art::icons::VIEWS_24.canvas_at(s);
            for item in &mut v.tools.bar.items {
                if let reprisekit::controls::toolbar::Item::Button { icon: slot, .. } = item {
                    *slot = Some(icon.clone());
                }
            }
            v.tools.bar.hot = Some(ZOOM_IN);
            let mut c = Canvas::with_scale(320, 240, s);
            ctx.at_scale(s, |ctx| v.draw(MAIN, &mut c, ctx));
            let ch = ctx.at_scale(s, |ctx| v.chrome(ctx)).unwrap();
            let band = Rect::new(0, 0, 320, ch.bottom).scaled(s);
            got.push((format!("{i:?}@{s}"), fnv(&c.copy_rect(band))));
        }
    }
    let file = reprisekit::crate_dir!().join("tests/pins/toolbar.txt");
    reprisekit::pins::check_file(&file, &got);
}

#[test]
fn the_desktop_defaults_open_every_type_the_viewer_declares() {
    let entry = include_str!("../../../../data/apps/reprise-imageviewer.desktop");
    let defaults = include_str!("../../../../data/reprise-mimeapps.list");
    let declared = entry.lines().find_map(|l| l.strip_prefix("MimeType="));
    let declared: Vec<&str> = declared
        .unwrap()
        .split(';')
        .filter(|t| !t.is_empty())
        .collect();
    assert!(declared.contains(&"image/jxl") && declared.contains(&"image/avif"));
    for t in declared {
        let line = format!("{t}=reprise-imageviewer.desktop");
        assert!(defaults.lines().any(|l| l == line), "{line} in the list");
    }
}

#[test]
fn an_opened_picture_goes_to_the_recent_files_and_its_neighbours_and_an_unreadable_one_do_not() {
    let dir = folder(&[("a.bmp", 4, 4), ("b.bmp", 4, 4)]);
    let xbel = dir.path().join("recently-used.xbel");
    let a = dir.path().join("a.bmp");
    let mut v = Viewer::new(None);
    let bad = dir.path().join("bad.bmp");
    std::fs::write(&bad, "not a picture").unwrap();
    v.recent = Recent::at(DESKTOP, Some(xbel.clone()));
    v.show(bad);
    v.show(a.clone());
    v.go(Step::Next);
    assert_eq!(v.folder.current(), Some(dir.path().join("b.bmp").as_path()));
    v.recent.settle();
    let marks = reprisefs::recent::RecentFiles::at(Some(xbel))
        .bookmarks()
        .to_vec();
    assert_eq!(marks.len(), 1, "{marks:?}");
    assert_eq!(marks[0].path, a);
    assert!(
        (marks[0].apps.iter()).any(|(_, exec)| exec.starts_with("'reprise-imageviewer ")),
        "recorded as Image Viewer's: {marks:?}"
    );
}

#[test]
fn the_window_grows_once_to_show_the_picture_it_opened_on_at_full_size() {
    let dir = folder(&[("dunes.bmp", 600, 500), ("tiny.bmp", 4, 4)]);
    let mut v = opened(dir.path().join("dunes.bmp"));
    let mut ctx = test_ctx();
    ctx.at_pick = true;
    let b = v.box_rect(&ctx);
    let chrome = (ctx.size.0 - b.width, ctx.size.1 - b.height);
    let r = v.event(MAIN, Event::Wake, &mut ctx);
    assert_eq!(r.size, Some((600 + chrome.0, 500 + chrome.1)));
    assert_eq!(v.event(MAIN, Event::Wake, &mut ctx).size, None, "once");

    let mut small = opened(dir.path().join("tiny.bmp"));
    assert_eq!(
        small.event(MAIN, Event::Wake, &mut ctx).size,
        None,
        "a small picture leaves the window at its own size"
    );
    let mut sized = opened(dir.path().join("dunes.bmp"));
    ctx.at_pick = false;
    assert_eq!(
        sized.event(MAIN, Event::Wake, &mut ctx).size,
        None,
        "a size the user or the compositor gave stands"
    );
}

#[test]
fn page_setup_and_print_sit_above_exit_in_the_file_menu() {
    let file = &menu::menus()[menu::title::FILE].1;
    let label = |r: usize| match &file[r] {
        reprisekit::controls::menu::MenuItem::Entry { label, .. } => label.clone(),
        _ => String::new(),
    };
    assert_eq!(label(menu::row::PAGE_SETUP), "Page Setup...");
    assert_eq!(label(menu::row::PREVIEW), "Print Preview...");
    assert_eq!(label(menu::row::PRINT), "Print...");
    assert_eq!(label(menu::row::EXIT), "Exit");
}

/// Points `$XDG_CACHE_HOME`, where copies out of compressed folders go, at a scratch folder once
/// per process.
fn scratch_cache() {
    static SCRATCH: std::sync::Once = std::sync::Once::new();
    SCRATCH.call_once(|| {
        let dir = std::env::temp_dir().join(format!("imageviewer-cache-{}", std::process::id()));
        // SAFETY: std serialises its own env access; no other test reads this variable.
        unsafe { std::env::set_var("XDG_CACHE_HOME", dir) };
    });
}

/// `Photos.zip` holding `a.bmp`, `b.bmp`, `notes.txt` and `sub/c.bmp`, and Explorer's copy of
/// its `a.bmp`.
fn zip_of_photos() -> (tempfile::TempDir, PathBuf, PathBuf) {
    scratch_cache();
    let dir = folder(&[("b.bmp", 4, 4), ("a.bmp", 2, 2)]);
    std::fs::create_dir(dir.path().join("sub")).unwrap();
    std::fs::write(dir.path().join("sub/c.bmp"), bmp(3, 3)).unwrap();
    let zip = dir.path().join("Photos.zip");
    let made = std::process::Command::new("bsdtar")
        .args(["-c", "--format", "zip", "-f"])
        .arg(&zip)
        .arg("-C")
        .arg(dir.path())
        .args(["b.bmp", "notes.txt", "a.bmp", "sub/c.bmp"])
        .status()
        .unwrap();
    assert!(made.success());
    let copy = reprisefs::archive::scratch().unwrap().join("a.bmp");
    std::fs::write(&copy, bmp(2, 2)).unwrap();
    reprisefs::archivecopy::set_origin(&copy, &zip, Path::new("a.bmp")).unwrap();
    (dir, zip, copy)
}

#[test]
fn next_and_previous_walk_the_pictures_of_the_compressed_folder_a_copy_came_out_of() {
    let (_dir, zip, copy) = zip_of_photos();
    let mut v = opened(copy.clone());
    assert_eq!(v.folder.position(), "1 of 2");
    assert_eq!(v.folder.current(), Some(copy.as_path()));
    v.go(Step::Next);
    let b = v.folder.current().unwrap().to_path_buf();
    assert_eq!(b.file_name().unwrap(), "b.bmp");
    assert!(
        reprisefs::archive::is_copy(&b),
        "b.bmp is copied out to show"
    );
    assert_eq!(v.picture.as_ref().map(|p| p.size()), Some((4, 4)));
    assert_eq!(v.folder.position(), "2 of 2");
    v.go(Step::Previous);
    let a = v.folder.current().unwrap().to_path_buf();
    assert_eq!(a.file_name().unwrap(), "a.bmp");
    assert_ne!(a, copy, "a.bmp is copied out again");
    assert_eq!(v.picture.as_ref().map(|p| p.size()), Some((2, 2)));
    assert_eq!(
        v.folder.pictures[1],
        zip.join("b.bmp"),
        "b.bmp stands as its address again"
    );
    let until = Instant::now() + Duration::from_secs(10);
    while b.exists() && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!b.exists(), "b.bmp's copy goes once another picture shows");
}

#[test]
fn a_picture_out_of_a_compressed_folder_offers_no_delete() {
    let (_dir, _zip, copy) = zip_of_photos();
    let mut v = opened(copy.clone());
    v.sync_menus();
    let enabled = match &v.bar.menus[menu::title::EDIT][menu::row::DELETE] {
        MenuItem::Entry { enabled, .. } => *enabled,
        _ => panic!("Delete is a row"),
    };
    assert!(!enabled, "Delete is greyed");
    v.ask_delete(false);
    assert!(v.delete.is_empty() && copy.exists(), "Delete does nothing");
}

#[test]
fn a_compressed_folders_pictures_are_those_beside_it_by_name_in_name_order() {
    let entry = |p: &str| reprisefs::archive::Entry {
        path: p.into(),
        kind: reprisefs::archive::Kind::File,
        size: 1,
        modified: None,
        mode: 0o644,
    };
    let entries = [
        entry("img10.png"),
        entry("img9.png"),
        entry(".hidden.png"),
        entry("notes.txt"),
        entry("sub/deeper.png"),
    ];
    let picked = walk::pictures(&entries, Path::new(""));
    let names: Vec<_> = picked
        .iter()
        .map(|e| e.path.display().to_string())
        .collect();
    assert_eq!(
        names,
        ["img10.png", "img9.png"],
        "listing order, sorted when shown"
    );
    let (_dir, zip, copy) = zip_of_photos();
    let mut w = walk::Walk::of(&copy).expect("a copy that says where it came from");
    let listed = w.listed(picked);
    assert_eq!(listed, [zip.join("img9.png"), zip.join("img10.png")]);
}
