use super::*;
use crate::bitmap::BLACK;
use crate::view::Lay;
use repriseapp::{BTN_LEFT, BTN_RIGHT, KeyInput, Mods, PointerInput};
use reprisekit::controls::palette;
use reprisekit::controls::toolbox::{self, Hit};
use reprisekit::interface::Interface;
use std::path::Path;

pub(crate) fn test_ctx() -> Ctx {
    Ctx {
        theme: Theme::default(),
        text: Rc::new(TextRenderer::load(&Theme::default().fonts)),
        icons: reprisekit::icontheme::IconCache::new(reprisekit::icontheme::IconTheme::load(
            "test",
            &[],
        )),
        caret_on: true,
        size: view::INITIAL_SIZE,
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

fn press(x: i32, y: i32, button: u32) -> Event {
    Event::Pointer(PointerInput::Press {
        x,
        y,
        button,
        time: 0,
        mods: Mods::default(),
    })
}

fn release(x: i32, y: i32, button: u32) -> Event {
    Event::Pointer(PointerInput::Release {
        x,
        y,
        button,
        mods: Mods::default(),
    })
}

fn motion(x: i32, y: i32) -> Event {
    Event::Pointer(PointerInput::Motion { x, y })
}

fn key(sym: u32, ctrl: bool) -> Event {
    Event::Key(KeyInput {
        sym,
        utf8: None,
        mods: Mods {
            ctrl,
            ..Mods::default()
        },
        edit: None,
    })
}

/// The window point over picture pixel `(x, y)`'s middle.
fn at(p: &Paint, l: &Lay, x: i32, y: i32) -> (i32, i32) {
    let r = p.on_window(l, Rect::new(x, y, 1, 1));
    (r.x + r.width / 2, r.y + r.height / 2)
}

fn drag(p: &mut Paint, ctx: &mut Ctx, from: (i32, i32), to: (i32, i32), button: u32) -> Reply {
    let l = p.lay(ctx);
    let (a, b) = (at(p, &l, from.0, from.1), at(p, &l, to.0, to.1));
    p.event(MAIN, press(a.0, a.1, button), ctx);
    p.event(MAIN, motion(b.0, b.1), ctx);
    p.event(MAIN, release(b.0, b.1, button), ctx)
}

fn set_edit(f: &mut Form, id: &str, s: &str) {
    if let Some(form::Control::Edit { edit, .. }) = f.item_mut(id).map(|i| &mut i.control) {
        *edit = reprisekit::textedit::TextEdit::new(s, false);
    }
}

fn tool_point(p: &Paint, ctx: &Ctx, tool: Tool) -> (i32, i32) {
    let l = p.lay(ctx);
    let tb = p.toolbox(1, ctx);
    let i = doc::TOOLS.iter().position(|&t| t == tool).unwrap();
    let r = toolbox::layout(&tb, l.tools).tools[i];
    (r.x + r.width / 2, r.y + r.height / 2)
}

fn pick(p: &mut Paint, ctx: &mut Ctx, tool: Tool) {
    let (x, y) = tool_point(p, ctx, tool);
    p.event(MAIN, press(x, y, BTN_LEFT), ctx);
    p.event(MAIN, release(x, y, BTN_LEFT), ctx);
    assert_eq!(p.tool, tool);
}

#[test]
fn a_press_on_the_tool_box_picks_the_tool_and_its_option() {
    let mut ctx = test_ctx();
    let mut p = Paint::new(None);
    pick(&mut p, &mut ctx, Tool::Line);
    let l = p.lay(&ctx);
    let tb = p.toolbox(1, &ctx);
    let lay = toolbox::layout(&tb, l.tools);
    let r = lay.options[3];
    assert_eq!(toolbox::hit(&lay, r.x + 1, r.y + 1), Some(Hit::Option(3)));
    p.event(MAIN, press(r.x + 1, r.y + 1, BTN_LEFT), &mut ctx);
    assert_eq!(p.doc.opts.width, view::WIDTHS[3]);
    assert_eq!(p.toolbox(1, &ctx).option, Some(3));
}

#[test]
fn swatches_set_the_first_color_on_the_left_and_the_second_on_the_right() {
    let mut ctx = test_ctx();
    let mut p = Paint::new(None);
    let l = p.lay(&ctx);
    let pal = p.palette();
    let lay = palette::layout(&pal, l.colors);
    let red = art::PALETTE.iter().position(|(_, n)| *n == "Red").unwrap();
    let blue = art::PALETTE.iter().position(|(_, n)| *n == "Blue").unwrap();
    let (r, b) = (lay.swatches[red], lay.swatches[blue]);
    p.event(MAIN, press(r.x + 4, r.y + 4, BTN_LEFT), &mut ctx);
    p.event(MAIN, press(b.x + 4, b.y + 4, BTN_RIGHT), &mut ctx);
    assert_eq!(p.doc.primary, 0xFFFF_0000);
    assert_eq!(p.doc.secondary, 0xFF00_00FF);
}

#[test]
fn a_drag_on_the_picture_draws_at_every_zoom() {
    for zoom in [1, 4] {
        let mut ctx = test_ctx();
        let mut p = Paint::new(None);
        p.zoom = zoom;
        pick(&mut p, &mut ctx, Tool::Line);
        drag(&mut p, &mut ctx, (2, 3), (9, 3), BTN_LEFT);
        for x in 2..=9 {
            assert_eq!(p.doc.img.get(x, 3), Some(BLACK), "zoom {zoom}, x {x}");
        }
        assert_eq!(p.doc.img.get(10, 3), Some(WHITE));
        assert!(p.doc.dirty && p.doc.hist.can_undo());
        p.event(MAIN, key(0x7a, true), &mut ctx);
        assert_eq!(p.doc.img.get(5, 3), Some(WHITE), "Ctrl+Z takes it back");
    }
}

#[test]
fn the_picker_sets_a_color_and_goes_back_to_the_tool_before() {
    let mut ctx = test_ctx();
    let mut p = Paint::new(None);
    p.doc.img.set(4, 4, 0xFF12_3456);
    pick(&mut p, &mut ctx, Tool::Brush);
    pick(&mut p, &mut ctx, Tool::Picker);
    drag(&mut p, &mut ctx, (4, 4), (4, 4), BTN_RIGHT);
    assert_eq!(p.doc.secondary, 0xFF12_3456);
    assert_eq!(p.tool, Tool::Brush);
}

#[test]
fn the_magnifier_zooms_in_on_the_point_and_back_out() {
    let mut ctx = test_ctx();
    let mut p = Paint::new(None);
    pick(&mut p, &mut ctx, Tool::Magnifier);
    drag(&mut p, &mut ctx, (300, 200), (300, 200), BTN_LEFT);
    assert_eq!(p.zoom, 4);
    let l = p.lay(&ctx);
    let mid = (l.view.x + l.view.width / 2, l.view.y + l.view.height / 2);
    let (x, y) = p.to_picture(&l, mid.0, mid.1);
    assert!(
        (x - 300).abs() <= 1 && (y - 200).abs() <= 1,
        "{x},{y} is in the middle"
    );
    drag(&mut p, &mut ctx, (300, 200), (300, 200), BTN_LEFT);
    assert_eq!(p.zoom, 1);
}

#[test]
fn the_grid_shows_from_four_times() {
    let ctx = test_ctx();
    let mut p = Paint::new(None);
    p.doc.img = Bitmap::new(20, 20, BLACK);
    for (zoom, grid) in [(2, false), (4, true)] {
        p.zoom = zoom;
        let l = p.lay(&ctx);
        let mut c = Canvas::new(ctx.size.0 as u32, ctx.size.1 as u32);
        p.draw(MAIN, &mut c, &ctx);
        let (x, y) = (l.origin.0, l.origin.1 + 1);
        let line = c.get(x, y) & 0xFF_FFFF == 0x80_8080;
        assert_eq!(line, grid, "zoom {zoom}");
        assert_eq!(
            c.get(x + 1, y + 1) & 0xFF_FFFF,
            0,
            "inside a pixel at zoom {zoom}"
        );
    }
}

#[test]
fn copy_puts_the_selection_on_the_clipboard_as_png_and_paste_floats_it() {
    let mut ctx = test_ctx();
    let mut p = Paint::new(None);
    p.doc.img.fill_rect(Rect::new(2, 2, 3, 2), BLACK);
    pick(&mut p, &mut ctx, Tool::Select);
    drag(&mut p, &mut ctx, (2, 2), (4, 3), BTN_LEFT);
    let reply = p.event(MAIN, key(0x63, true), &mut ctx);
    let png = reply.copy_image.expect("Ctrl+C copies the selection");
    let copied = file::decode(&png).unwrap();
    assert_eq!((copied.w, copied.h), (3, 2));
    assert!(copied.px.iter().all(|&c| c == BLACK));
    repriseapp::fake_clipboard_image(true);
    p.sync_menus();
    let reply = p.event(MAIN, key(0x76, true), &mut ctx);
    assert!(reply.paste_image, "Ctrl+V asks for the clipboard's picture");
    p.event(MAIN, Event::PastedImage(png.to_vec()), &mut ctx);
    let s = p.doc.sel.as_ref().expect("the paste is a selection");
    assert!(s.float.is_some());
    assert_eq!(s.rect, Rect::new(0, 0, 3, 2));
    p.event(MAIN, key(0xff0d, false), &mut ctx);
    assert_eq!(p.doc.img.get(0, 0), Some(BLACK), "Enter lays it down");
    repriseapp::fake_clipboard_image(false);
}

#[test]
fn an_empty_paste_changes_nothing() {
    let mut ctx = test_ctx();
    let mut p = Paint::new(None);
    p.event(MAIN, Event::PastedImage(Vec::new()), &mut ctx);
    assert!(p.doc.sel.is_none() && p.prompt.is_none());
}

#[test]
fn text_typed_in_the_box_lands_on_the_picture_when_clicked_outside() {
    let mut ctx = test_ctx();
    let mut p = Paint::new(None);
    pick(&mut p, &mut ctx, Tool::Text);
    assert!(
        p.fonts.is_some(),
        "the text toolbar opens with the text tool"
    );
    if p.families.is_empty() {
        reprisekit::pins::skipped("no fonts installed");
        return;
    }
    p.text.font.px = 20;
    drag(&mut p, &mut ctx, (10, 10), (120, 40), BTN_LEFT);
    assert_eq!(p.doc.text_box, Some(Rect::new(10, 10, 111, 31)));
    for ch in ["H", "i"] {
        p.event(
            MAIN,
            Event::Key(KeyInput {
                sym: ch.as_bytes()[0] as u32,
                utf8: Some(ch.into()),
                mods: Mods::default(),
                edit: Some(reprisekit::textedit::EditKey::Insert(ch.into())),
            }),
            &mut ctx,
        );
    }
    assert_eq!(p.text.edit.text(), "Hi");
    assert!(!p.doc.hist.can_undo(), "nothing is on the picture yet");
    drag(&mut p, &mut ctx, (300, 300), (300, 300), BTN_LEFT);
    assert!(p.doc.text_box.is_none());
    let inked = (10..120)
        .flat_map(|x| (10..40).map(move |y| (x, y)))
        .filter(|&(x, y)| p.doc.img.get(x, y) != Some(WHITE))
        .count();
    assert!(inked > 20, "{inked} pixels of text");
    assert!(p.doc.hist.can_undo());
}

#[test]
fn attributes_size_the_canvas_and_refuse_a_bad_number() {
    let mut ctx = test_ctx();
    let mut p = Paint::new(None);
    p.command(menu::title::IMAGE, menu::row::ATTRIBUTES, &ctx);
    let f = &mut p.dialog.as_mut().unwrap().1;
    set_edit(f, "width", "64");
    set_edit(f, "height", "0");
    p.event(DIALOG, key(0xff0d, false), &mut ctx);
    assert!(p.dialog.is_some() && p.prompt.is_some(), "0 is refused");
    p.prompt = None;
    set_edit(&mut p.dialog.as_mut().unwrap().1, "height", "32");
    p.event(DIALOG, key(0xff0d, false), &mut ctx);
    assert!(p.dialog.is_none());
    assert_eq!((p.doc.img.w, p.doc.img.h), (64, 32));
}

#[test]
fn resize_and_flip_change_the_whole_picture() {
    let mut ctx = test_ctx();
    let mut p = Paint::new(None);
    p.doc.img = Bitmap::new(10, 4, WHITE);
    p.doc.img.set(0, 0, BLACK);
    p.open_dialog(dialogs::Kind::Stretch);
    set_edit(&mut p.dialog.as_mut().unwrap().1, "resize-h", "200");
    p.event(DIALOG, key(0xff0d, false), &mut ctx);
    assert_eq!((p.doc.img.w, p.doc.img.h), (20, 4));
    p.open_dialog(dialogs::Kind::Flip);
    p.event(DIALOG, key(0xff0d, false), &mut ctx);
    assert_eq!(p.doc.img.get(19, 0), Some(BLACK), "flipped left to right");
    p.command(menu::title::IMAGE, menu::row::INVERT, &ctx);
    assert_eq!(p.doc.img.get(19, 0), Some(WHITE));
}

#[test]
fn a_saved_picture_reads_back_and_clears_the_changes() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = test_ctx();
    let mut p = Paint::new(None);
    pick(&mut p, &mut ctx, Tool::Pencil);
    drag(&mut p, &mut ctx, (1, 1), (6, 1), BTN_LEFT);
    let path = dir.path().join("drawing.png");
    p.path = Some(path.clone());
    p.event(MAIN, key(0x73, true), &mut ctx);
    assert!(!p.doc.dirty);
    let again = Paint::new(Some(path));
    assert_eq!(again.doc.img, p.doc.img);
    assert_eq!(again.title(), "drawing.png - Paint");
}

#[test]
fn new_on_a_changed_picture_asks_first() {
    let mut ctx = test_ctx();
    let mut p = Paint::new(None);
    drag(&mut p, &mut ctx, (1, 1), (6, 1), BTN_LEFT);
    p.event(MAIN, key(0x6e, true), &mut ctx);
    assert!(matches!(
        p.prompt,
        Some((_, Prompt::SaveChanges(Then::New)))
    ));
    p.prompt = None;
    p.answered(Prompt::SaveChanges(Then::New), Some(1));
    assert!(!p.doc.dirty && !p.doc.hist.can_undo());
    assert_eq!(p.doc.img.get(3, 1), Some(WHITE));
}

#[test]
fn save_as_adds_the_types_extension_only_when_missing() {
    use crate::file::Kind;
    let p = PathBuf::from("/home/u/Pictures/cat");
    assert_eq!(
        files::with_ext(p, Kind::Jpeg),
        PathBuf::from("/home/u/Pictures/cat.jpg")
    );
    let png = PathBuf::from("/home/u/Pictures/cat.PNG");
    assert_eq!(files::with_ext(png.clone(), Kind::Bmp), png);
}

#[test]
fn the_background_points_the_config_at_the_picture() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config.toml");
    let before = "[wallpaper]\nslideshow = \"/home/u/Pictures\"\nstyle = \"fill\"\n";
    std::fs::write(&config, before).unwrap();
    files::set_wallpaper(&config, Path::new("/home/u/Pictures/cat.png")).unwrap();
    let text = std::fs::read_to_string(&config).unwrap();
    assert!(
        text.contains("image = \"/home/u/Pictures/cat.png\""),
        "{text}"
    );
    assert!(!text.contains("slideshow"), "{text}");
    assert!(text.contains("style = \"fill\""), "{text}");
}

#[test]
fn the_status_bar_shows_the_pointer_and_the_selection_size() {
    let mut ctx = test_ctx();
    let mut p = Paint::new(None);
    pick(&mut p, &mut ctx, Tool::Select);
    drag(&mut p, &mut ctx, (5, 6), (14, 9), BTN_LEFT);
    let l = p.lay(&ctx);
    let (bar, _) = p.status(&l, &ctx).unwrap();
    assert_eq!(bar.panels[1].text, "14,9");
    assert_eq!(bar.panels[2].text, "10x4");
}

/// Pinned hashes in `tests/pins/<name>.txt`; `REPRISE_BLESS=1` rewrites the file.
fn check_pins(name: &str, got: &[(String, u64)]) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/pins")
        .join(format!("{name}.txt"));
    let text: String = got.iter().map(|(k, h)| format!("{k} {h:016x}\n")).collect();
    if std::env::var_os("REPRISE_BLESS").is_some_and(|v| v != "0") {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, text).unwrap();
        return;
    }
    let pinned = std::fs::read_to_string(&path).unwrap_or_default();
    assert_eq!(
        pinned, text,
        "{name}: rerun with REPRISE_BLESS=1 after checking the pictures"
    );
}

fn hash(c: &Canvas) -> u64 {
    c.pixels.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &p| {
        (h ^ u64::from(p)).wrapping_mul(0x0100_0000_01b3)
    })
}

#[test]
fn the_window_is_pinned_in_each_interface_at_scale_one_and_two() {
    if !reprisekit::text::test_tahoma() {
        reprisekit::pins::skipped("REPRISE_TEST_FONTS is not set");
        return;
    }
    let mut got = Vec::new();
    for look in [Interface::Classic, Interface::Blue, Interface::Glass] {
        for s in [1, 2] {
            let mut ctx = test_ctx();
            ctx.size = (480, 360);
            ctx.theme = reprisekit::interfaces::theme_for(look);
            let mut p = Paint::new(None);
            p.doc.img = Bitmap::new(200, 120, WHITE);
            pick(&mut p, &mut ctx, Tool::Rect);
            p.doc.opts.fill = doc::FillStyle::Both;
            p.doc.secondary = 0xFFFF_FF00;
            drag(&mut p, &mut ctx, (10, 10), (60, 40), BTN_LEFT);
            pick(&mut p, &mut ctx, Tool::Brush);
            drag(&mut p, &mut ctx, (80, 20), (150, 90), BTN_LEFT);
            let mut c = Canvas::with_scale(480, 360, s);
            ctx.at_scale(s, |ctx| p.draw(MAIN, &mut c, ctx));
            if std::env::var_os("REPRISE_PAINT_PREVIEW").is_some() {
                reprisekit::raster::write_preview(&c, &format!("paint-{look:?}-{s}"));
            }
            got.push((format!("{look:?} {s}"), hash(&c)));
        }
    }
    check_pins("window", &got);
}
