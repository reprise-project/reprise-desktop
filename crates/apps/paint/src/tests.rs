use super::*;
use crate::bitmap::BLACK;
use crate::view::Lay;
use repriseapp::filedlg::Driven;
use repriseapp::{BTN_LEFT, BTN_RIGHT, KeyInput, Mods, PointerInput};
use reprisekit::controls::palette;
use reprisekit::controls::sizehandles;
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

pub(crate) fn press(x: i32, y: i32, button: u32) -> Event {
    Event::Pointer(PointerInput::Press {
        x,
        y,
        button,
        time: 0,
        mods: Mods::default(),
    })
}

pub(crate) fn release(x: i32, y: i32, button: u32) -> Event {
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

pub(crate) fn key(sym: u32, ctrl: bool) -> Event {
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
    assert!(p.doc.sel.is_none() && p.prompt.is_empty());
}

const JXL: &[u8] = &[0xff, 0x0a, 0, 0];
const AVIF: &[u8] = b"\0\0\0\x1cftypavif\0\0\0\0avifmif1miaf\0\0\x01\x0ameta";

/// Paint decoding JPEG XL and AVIF with a reprise-decode stand-in at `dir` that runs `body` after
/// reading its request.
fn with_fake_helper(dir: &Path, body: &str) -> Paint {
    use repriseapp::decode::{Format, HelperDecoder};
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join("reprise-decode");
    std::fs::write(&path, format!("#!/bin/sh\ncat >/dev/null\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut p = Paint::new(None);
    p.decoders = Decoders::built_in();
    for format in [Format::Jxl, Format::Avif] {
        (p.decoders).register(Box::new(HelperDecoder::at(format, path.clone())));
    }
    p
}

fn prompt_text(p: &Paint) -> Option<String> {
    p.prompt.up().map(|(b, _)| b.fields.text.clone())
}

#[test]
fn a_pasted_jpeg_xl_or_avif_is_decoded_by_reprise_decode_and_floats() {
    let dir = tempfile::tempdir().unwrap();
    // A 2x1 picture: black, then transparent.
    let reply = r"\000\002\000\000\000\001\000\000\000\000\000\000\377\000\000\000\000";
    for bytes in [JXL, AVIF] {
        let mut ctx = test_ctx();
        let mut p = with_fake_helper(dir.path(), &format!("printf '{reply}'"));
        p.event(MAIN, Event::PastedImage(bytes.to_vec()), &mut ctx);
        assert!(p.prompt.is_empty(), "{:?}", prompt_text(&p));
        let s = p.doc.sel.as_ref().expect("the paste is a selection");
        assert!(s.float.is_some());
        assert_eq!(s.rect, Rect::new(0, 0, 2, 1));
    }
}

#[test]
fn a_pasted_picture_reprise_decode_cannot_read_says_what_an_unreadable_png_does() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = test_ctx();
    let mut p = Paint::new(None);
    p.event(
        MAIN,
        Event::PastedImage(b"\x89PNG\r\n\x1a\ncut".to_vec()),
        &mut ctx,
    );
    let png = prompt_text(&p).expect("an unreadable PNG is refused");
    for body in ["exit 0", "kill -ABRT $$"] {
        for bytes in [JXL, AVIF] {
            let mut p = with_fake_helper(dir.path(), body);
            p.event(MAIN, Event::PastedImage(bytes.to_vec()), &mut ctx);
            assert_eq!(prompt_text(&p).as_ref(), Some(&png), "{body}");
            assert!(p.doc.sel.is_none());
        }
    }
}

#[test]
fn a_pasted_jpeg_xl_past_the_budget_is_refused_before_reprise_decode_runs() {
    let dir = tempfile::tempdir().unwrap();
    let ran = dir.path().join("ran");
    let mut ctx = test_ctx();
    let mut p = with_fake_helper(dir.path(), &format!("touch '{}'", ran.display()));
    // A codestream whose SizeHeader claims 20000x20000.
    let big = vec![0xff, 0x0a, 0xfc, 0x70, 0x22, 0x01];
    p.event(MAIN, Event::PastedImage(big), &mut ctx);
    assert_eq!(prompt_text(&p), Some(tr!("paste-failed")));
    assert!(!ran.exists(), "reprise-decode never started");
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
    assert!(p.dialog.is_some() && !p.prompt.is_empty(), "0 is refused");
    p.prompt.close();
    set_edit(&mut p.dialog.as_mut().unwrap().1, "height", "32");
    p.event(DIALOG, key(0xff0d, false), &mut ctx);
    assert!(p.dialog.is_none());
    assert_eq!((p.doc.img.w, p.doc.img.h), (64, 32));
}

/// The window point in the middle of `h`, the picture's sizing handle.
fn handle_point(p: &Paint, ctx: &Ctx, h: sizehandles::Handle) -> (i32, i32) {
    let r = p.handles(&p.lay(ctx)).rect(h);
    (r.x + r.width / 2, r.y + r.height / 2)
}

#[test]
fn the_corner_handle_grows_the_picture_into_the_background_colour_and_undo_takes_it_back() {
    let mut ctx = test_ctx();
    let mut p = Paint::new(None);
    p.doc.img = Bitmap::new(40, 30, WHITE);
    p.doc.img.set(0, 0, BLACK);
    p.doc.img.set(39, 29, BLACK);
    p.doc.secondary = 0xFFFF_0000;
    let (x, y) = handle_point(&p, &ctx, sizehandles::Handle::Corner);
    p.event(MAIN, press(x, y, BTN_LEFT), &mut ctx);
    p.event(MAIN, motion(x + 20, y + 10), &mut ctx);
    let l = p.lay(&ctx);
    assert_eq!(p.status(&l, &ctx).unwrap().0.panels[2].text, "60x40");
    assert_eq!(
        (p.doc.img.w, p.doc.img.h),
        (40, 30),
        "the size waits for the release"
    );
    p.event(MAIN, release(x + 20, y + 10, BTN_LEFT), &mut ctx);
    assert_eq!((p.doc.img.w, p.doc.img.h), (60, 40));
    assert_eq!(
        p.doc.img.get(0, 0),
        Some(BLACK),
        "the picture keeps its top-left"
    );
    assert_eq!(p.doc.img.get(39, 29), Some(BLACK));
    assert_eq!(
        p.doc.img.get(59, 39),
        Some(0xFFFF_0000),
        "new area is the background"
    );
    assert!(p.doc.dirty);
    p.command(menu::title::EDIT, menu::row::UNDO, &ctx);
    assert_eq!((p.doc.img.w, p.doc.img.h), (40, 30));
    assert_eq!(p.doc.img.get(39, 29), Some(BLACK));
}

#[test]
fn the_edge_handles_move_one_side_at_the_zoom_and_escape_drops_the_drag() {
    let mut ctx = test_ctx();
    let mut p = Paint::new(None);
    p.doc.img = Bitmap::new(40, 30, WHITE);
    p.zoom = 2;
    let (x, y) = handle_point(&p, &ctx, sizehandles::Handle::Right);
    p.event(MAIN, press(x, y, BTN_LEFT), &mut ctx);
    p.event(MAIN, motion(x - 20, y + 50), &mut ctx);
    p.event(MAIN, release(x - 20, y + 50, BTN_LEFT), &mut ctx);
    assert_eq!(
        (p.doc.img.w, p.doc.img.h),
        (30, 30),
        "two window px a picture px"
    );

    let (x, y) = handle_point(&p, &ctx, sizehandles::Handle::Bottom);
    p.event(MAIN, press(x, y, BTN_LEFT), &mut ctx);
    p.event(MAIN, motion(x + 50, y + 10), &mut ctx);
    p.event(MAIN, key(0xff1b, false), &mut ctx);
    assert!(p.sizing.is_none());
    p.event(MAIN, release(x + 50, y + 10, BTN_LEFT), &mut ctx);
    assert_eq!(
        (p.doc.img.w, p.doc.img.h),
        (30, 30),
        "Escape keeps the size"
    );
    assert_eq!(p.doc.hist.depth(), 1, "only the first drag is a step");
}

#[test]
fn a_sizing_handle_shows_its_arrow_over_it_and_through_its_drag() {
    use reprisekit::pointer::Shape;
    let mut ctx = test_ctx();
    let mut p = Paint::new(None);
    p.doc.img = Bitmap::new(40, 30, WHITE);
    let (x, y) = handle_point(&p, &ctx, sizehandles::Handle::Bottom);
    assert_eq!(
        p.event(MAIN, motion(x, y), &mut ctx).pointer,
        Shape::SResize
    );
    assert_eq!(
        p.event(MAIN, motion(x - 5, y - 10), &mut ctx).pointer,
        Shape::Default,
        "on the picture"
    );
    p.event(MAIN, press(x, y, BTN_LEFT), &mut ctx);
    let reply = p.event(MAIN, motion(x + 30, y + 12), &mut ctx);
    assert_eq!(reply.pointer, Shape::SResize, "off the handle");
}

#[test]
fn the_pointer_s_place_follows_a_sizing_drag() {
    let mut ctx = test_ctx();
    let mut p = Paint::new(None);
    p.doc.img = Bitmap::new(40, 30, WHITE);
    let (x, y) = handle_point(&p, &ctx, sizehandles::Handle::Bottom);
    p.event(MAIN, press(x, y, BTN_LEFT), &mut ctx);
    p.event(MAIN, motion(x + 30, y + 12), &mut ctx);
    let l = p.lay(&ctx);
    let (px, py) = p.to_picture(&l, x + 30, y + 12);
    assert_eq!(
        p.status(&l, &ctx).unwrap().0.panels[1].text,
        format!("{px},{py}")
    );
}

#[test]
fn a_sizing_drag_resting_past_the_view_scrolls_it_and_grows_the_picture_past_the_window() {
    let mut ctx = test_ctx();
    let mut p = Paint::new(None);
    p.doc.img = Bitmap::new(40, 30, WHITE);
    let (x, y) = handle_point(&p, &ctx, sizehandles::Handle::Right);
    p.event(MAIN, press(x, y, BTN_LEFT), &mut ctx);
    let edge = p.lay(&ctx).view.right();
    let reply = p.event(MAIN, motion(edge + 10, y), &mut ctx);
    assert!(reply.tick, "past the edge the view starts following");
    let reached = p.sizing.unwrap().size.0;
    let under = p.lay(&ctx).origin.0 + reached - (edge + 10);
    let mut ticks = 0;
    while p.sizing.unwrap().size.0 <= ctx.size.0 && p.event(MAIN, Event::Tick, &mut ctx).tick {
        ticks += 1;
    }
    let grown = p.sizing.unwrap().size.0;
    assert!(
        grown > ctx.size.0,
        "{grown} past the {} px window",
        ctx.size.0
    );
    assert_eq!(
        grown,
        reached + 11 * ticks,
        "a tick scrolls as far as the pointer rests past the edge"
    );
    assert_eq!(
        p.lay(&ctx).origin.0 + grown - (edge + 10),
        under,
        "the outline's edge stays where it was under the pointer"
    );
    p.event(MAIN, release(edge + 10, y, BTN_LEFT), &mut ctx);
    assert_eq!(p.doc.img.w, grown);
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
fn a_skew_past_the_largest_picture_is_refused_and_changes_nothing() {
    let mut ctx = test_ctx();
    let mut p = Paint::new(None);
    p.doc.img = Bitmap::new(1920, 1080, WHITE);
    p.open_dialog(dialogs::Kind::Stretch);
    let f = &mut p.dialog.as_mut().unwrap().1;
    set_edit(f, "skew-h", "89");
    set_edit(f, "skew-v", "89");
    p.event(DIALOG, key(0xff0d, false), &mut ctx);
    assert!(p.dialog.is_some(), "the dialog stays up");
    assert_eq!(
        p.prompt.up().map(|(b, _)| b.fields.text.as_str()),
        Some("The picture would be more than 16,384 pixels wide or high.")
    );
    assert_eq!((p.doc.img.w, p.doc.img.h), (1920, 1080));
    assert!(!p.doc.dirty);
}

#[test]
fn a_covered_window_ends_a_stroke_where_it_is_and_commits_it() {
    let mut ctx = test_ctx();
    let mut p = Paint::new(None);
    pick(&mut p, &mut ctx, Tool::Pencil);
    let l = p.lay(&ctx);
    let (a, b, c) = (at(&p, &l, 1, 3), at(&p, &l, 6, 3), at(&p, &l, 12, 3));
    p.event(MAIN, press(a.0, a.1, BTN_LEFT), &mut ctx);
    p.event(MAIN, motion(b.0, b.1), &mut ctx);
    repriseapp::cover(&mut p, MAIN, &ctx);
    p.event(MAIN, motion(c.0, c.1), &mut ctx);
    assert_eq!(p.doc.img.get(6, 3), Some(BLACK), "the stroke reaches x 6");
    assert_eq!(p.doc.img.get(7, 3), Some(WHITE), "and stops there");
    p.event(MAIN, key(0x7a, true), &mut ctx);
    assert_eq!(
        p.doc.img.get(3, 3),
        Some(WHITE),
        "Ctrl+Z takes the stroke back"
    );
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
        p.prompt.up(),
        Some((_, Prompt::SaveChanges(Then::New)))
    ));
    p.prompt.close();
    p.answered(Prompt::SaveChanges(Then::New), Some(1));
    assert!(!p.doc.dirty && !p.doc.hist.can_undo());
    assert_eq!(p.doc.img.get(3, 1), Some(WHITE));
}

/// The prompt up and its window.
fn prompt_up(p: &Paint) -> Option<(Prompt, WinId)> {
    Some((p.prompt.up()?.1.clone(), p.prompt.win()?))
}

/// A changed picture saved at `dir`/drawing.png.
fn changed_picture(dir: &Path, ctx: &mut Ctx) -> Paint {
    let mut p = Paint::new(None);
    pick(&mut p, ctx, Tool::Pencil);
    drag(&mut p, ctx, (1, 1), (6, 1), BTN_LEFT);
    p.path = Some(dir.join("drawing.png"));
    p
}

#[test]
fn a_draw_failure_waits_behind_save_changes_and_yes_still_quits() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = test_ctx();
    let mut p = changed_picture(dir.path(), &mut ctx);
    p.command(menu::title::FILE, menu::row::EXIT, &ctx);
    p.draw_failed("no memory");
    let (what, win) = prompt_up(&p).expect("a prompt is up");
    assert_eq!(what, Prompt::SaveChanges(Then::Quit));
    let reply = p.event(win, key(0xff0d, false), &mut ctx);
    assert!(reply.quit, "Yes saves and quits");
    assert!(dir.path().join("drawing.png").exists());
}

#[test]
fn save_changes_raised_behind_a_draw_failure_is_up_after_its_ok_and_yes_still_quits() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = test_ctx();
    let mut p = changed_picture(dir.path(), &mut ctx);
    p.draw_failed("no memory");
    p.command(menu::title::FILE, menu::row::EXIT, &ctx);
    let (what, win) = prompt_up(&p).expect("a prompt is up");
    assert_eq!(what, Prompt::Error);
    assert!(!p.event(win, key(0xff0d, false), &mut ctx).quit);
    let (what, win) = prompt_up(&p).expect("Save changes is up after OK");
    assert_eq!(what, Prompt::SaveChanges(Then::Quit));
    let reply = p.event(win, key(0xff0d, false), &mut ctx);
    assert!(reply.quit, "Yes saves and quits");
    assert!(dir.path().join("drawing.png").exists());
}

/// A picture with a stroke and no file, whose `emergency` writes to `dir`.
fn edited_picture(dir: &Path, ctx: &mut Ctx) -> Paint {
    let mut p = Paint::new(None);
    p.rescue_dir = Some(dir.to_path_buf());
    pick(&mut p, ctx, Tool::Pencil);
    drag(&mut p, ctx, (1, 1), (6, 4), BTN_LEFT);
    p
}

#[test]
fn emergency_with_an_edited_picture_writes_a_recovery_file_with_its_pixels() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = test_ctx();
    let mut p = edited_picture(dir.path(), &mut ctx);
    p.emergency();
    let kept = reprisefs::recovery::found(dir.path(), "png");
    assert_eq!(kept.len(), 1);
    assert_eq!(file::load(&kept[0]).unwrap(), p.doc.img);
}

#[test]
fn emergency_with_a_saved_or_untouched_picture_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = test_ctx();
    let mut p = Paint::new(None);
    p.rescue_dir = Some(dir.path().to_path_buf());
    p.emergency();
    let mut p = edited_picture(dir.path(), &mut ctx);
    p.path = Some(dir.path().join("drawing.png"));
    p.event(MAIN, key(0x73, true), &mut ctx);
    p.emergency();
    assert!(reprisefs::recovery::found(dir.path(), "png").is_empty());
}

#[test]
fn yes_on_the_recovered_question_opens_the_picture_unsaved_and_a_save_removes_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = test_ctx();
    let mut p = edited_picture(dir.path(), &mut ctx);
    p.emergency();
    let kept = reprisefs::recovery::found(dir.path(), "png");

    let mut next = Paint::new(None);
    next.rescue_dir = Some(dir.path().to_path_buf());
    next.offer_recovered();
    let (what, win) = prompt_up(&next).expect("the question is up");
    assert_eq!(what, Prompt::Recovered(kept[0].clone()));
    next.event(win, key(0xff0d, false), &mut ctx);
    assert!(next.prompt.is_empty());
    assert_eq!(next.doc.img, p.doc.img);
    assert_eq!(next.path, None);
    assert!(next.doc.dirty);
    assert!(kept[0].exists(), "kept until the picture is saved");

    next.path = Some(dir.path().join("saved.png"));
    next.event(MAIN, key(0x73, true), &mut ctx);
    assert!(!kept[0].exists());
}

#[test]
fn no_on_the_recovered_question_removes_the_file_and_leaves_the_picture() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = test_ctx();
    edited_picture(dir.path(), &mut ctx).emergency();
    let kept = reprisefs::recovery::found(dir.path(), "png");

    let mut next = Paint::new(None);
    next.rescue_dir = Some(dir.path().to_path_buf());
    next.offer_recovered();
    next.prompt.up_mut().expect("the question").fields.default = 1;
    let win = next.prompt.win().unwrap();
    next.event(win, key(0xff0d, false), &mut ctx);
    assert!(next.prompt.is_empty());
    assert!(!kept[0].exists());
    assert!(!next.doc.dirty);
    assert_eq!(next.doc.img.get(3, 3), Some(WHITE));
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
fn a_quoted_save_as_name_is_written_as_typed() {
    let dir = tempfile::tempdir().unwrap();
    let mut p = Paint::new(None);
    p.saveas_dialog();
    p.saveas.as_mut().unwrap().set_file_name("\"cat\"");
    let path = dir.path().join("cat");
    p.saveas_driven(Driven::Picked(path.clone()));
    assert!(path.exists(), "cat is written");
    assert!(!dir.path().join("cat.png").exists(), "no .png is added");
    assert!(file::load(&path).is_ok(), "as a PNG");
}

/// A one-frame 4x2 GIF at `path`.
fn write_gif(path: &Path) -> Vec<u8> {
    let mut bytes = Vec::new();
    let frame = image::RgbaImage::from_pixel(4, 2, image::Rgba([200, 30, 30, 255]));
    image::codecs::gif::GifEncoder::new(&mut bytes)
        .encode_frame(image::Frame::new(frame))
        .unwrap();
    std::fs::write(path, &bytes).unwrap();
    bytes
}

#[test]
fn save_on_a_gif_opens_save_as_on_a_png_and_leaves_the_gif() {
    let dir = tempfile::tempdir().unwrap();
    let gif = dir.path().join("party.gif");
    let bytes = write_gif(&gif);
    let mut p = Paint::new(Some(gif.clone()));
    assert_eq!(p.path.as_deref(), Some(gif.as_path()), "the GIF opens");
    p.save();
    assert_eq!(std::fs::read(&gif).unwrap(), bytes, "the GIF is untouched");
    let dlg = p.saveas.as_ref().expect("Save As opens");
    assert_eq!(dlg.file_name(), "party.png");
    assert_eq!(dlg.filter(), 0, "Save as type is PNG");
    assert!(p.after_save.is_none());

    p.saveas = None;
    p.doc.dirty = true;
    let reply = p.answered(Prompt::SaveChanges(Then::Quit), Some(0));
    assert!(!reply.quit, "nothing quits before Save As is done");
    assert_eq!(std::fs::read(&gif).unwrap(), bytes);
    assert!(p.saveas.is_some());
    assert_eq!(p.after_save, Some(Then::Quit));
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

/// A 2x1 black picture as the clipboard gives it.
fn black_png() -> Vec<u8> {
    file::encode(&Bitmap::new(2, 1, BLACK), file::Kind::Png, 100).unwrap()
}

/// A paste is answered whenever the clipboard's owner writes it, so it can land while a box over
/// the picture is up.
#[test]
fn a_picture_pasted_under_open_asks_to_save_before_the_pick_replaces_it() {
    let dir = tempfile::tempdir().unwrap();
    let picked = dir.path().join("picked.png");
    file::save(&Bitmap::new(3, 3, WHITE), &picked, file::Kind::Png, 100).unwrap();
    let mut ctx = test_ctx();
    let mut p = Paint::new(None);
    p.command(menu::title::FILE, menu::row::OPEN, &ctx);
    assert!(p.open.is_some(), "Open is up at once over a clean picture");
    p.event(MAIN, Event::PastedImage(black_png()), &mut ctx);

    p.open_driven(Driven::Picked(picked.clone()));
    assert_eq!(
        p.doc.img.get(0, 0),
        Some(BLACK),
        "the pick leaves the paste"
    );
    let (what, _) = prompt_up(&p).expect("Save changes is up");
    assert_eq!(what, Prompt::SaveChanges(Then::Pick(picked.clone())));
    assert!(p.open.is_none(), "Open is not left up beside the prompt");

    p.prompt.close();
    p.answered(what, Some(1));
    assert_eq!(
        p.path.as_deref(),
        Some(picked.as_path()),
        "No opens the pick"
    );
    assert_eq!((p.doc.img.w, p.doc.img.h), (3, 3));
}

#[test]
fn no_on_save_changes_lets_go_of_a_picture_pasted_under_it() {
    let dir = tempfile::tempdir().unwrap();
    let picked = dir.path().join("picked.png");
    file::save(&Bitmap::new(3, 3, WHITE), &picked, file::Kind::Png, 100).unwrap();
    let mut ctx = test_ctx();
    let mut p = changed_picture(dir.path(), &mut ctx);
    p.command(menu::title::FILE, menu::row::OPEN, &ctx);
    let (what, _) = prompt_up(&p).expect("Save changes is up");
    p.event(MAIN, Event::PastedImage(black_png()), &mut ctx);
    p.prompt.close();
    p.answered(what, Some(1));

    p.open_driven(Driven::Picked(picked.clone()));
    assert!(
        prompt_up(&p).is_none(),
        "No was the answer for the paste too"
    );
    assert_eq!(p.path.as_deref(), Some(picked.as_path()));
}

#[test]
fn a_picture_pasted_under_save_as_is_in_the_file_saved_before_quitting() {
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = test_ctx();
    let mut p = Paint::new(None);
    pick(&mut p, &mut ctx, Tool::Pencil);
    drag(&mut p, &mut ctx, (1, 1), (6, 1), BTN_LEFT);
    p.event(MAIN, Event::CloseRequest, &mut ctx);
    let (what, _) = prompt_up(&p).expect("Save changes is up");
    p.prompt.close();
    p.answered(what, Some(0));
    assert!(
        p.saveas.is_some(),
        "Yes on an untitled picture opens Save As"
    );
    p.event(MAIN, Event::PastedImage(black_png()), &mut ctx);

    let path = dir.path().join("drawing.png");
    assert!(p.saveas_driven(Driven::Picked(path.clone())).quit);
    let saved = file::load(&path).unwrap();
    assert_eq!(saved.get(0, 0), Some(BLACK), "the paste is saved");
    assert_eq!(saved.get(3, 1), Some(BLACK), "with the stroke");
}

#[test]
fn a_picture_pasted_under_flip_and_rotate_is_flipped_with_the_rest() {
    let mut ctx = test_ctx();
    let mut p = Paint::new(None);
    p.doc.img = Bitmap::new(10, 4, WHITE);
    p.open_dialog(dialogs::Kind::Flip);
    p.event(MAIN, Event::PastedImage(black_png()), &mut ctx);
    p.event(DIALOG, key(0xff0d, false), &mut ctx);
    assert!(p.dialog.is_none());
    assert_eq!(p.doc.img.get(9, 0), Some(BLACK), "the paste is flipped");
    assert_eq!(p.doc.img.get(0, 0), Some(WHITE));
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

    p.doc.img = Bitmap::new(1920, 1080, WHITE);
    p.command(menu::title::EDIT, menu::row::SELECT_ALL, &ctx);
    p.hover = Some((1024, 768));
    let (bar, _) = p.status(&l, &ctx).unwrap();
    assert_eq!(bar.panels[1].text, "1024,768");
    assert_eq!(bar.panels[2].text, "1920x1080");
}

/// [`reprisekit::pins::check_file`] on `tests/pins/<name>.txt`.
#[track_caller]
fn check_pins(name: &str, got: &[(String, u64)]) {
    let path = reprisekit::crate_dir!()
        .join("tests/pins")
        .join(format!("{name}.txt"));
    reprisekit::pins::check_file(&path, got);
}

fn hash(c: &Canvas) -> u64 {
    c.pixels.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &p| {
        (h ^ u64::from(p)).wrapping_mul(0x0100_0000_01b3)
    })
}

#[test]
fn the_window_is_pinned_in_each_interface_at_scale_one_and_two() {
    reprisekit::pins::fonts();
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

fn typed(p: &mut Paint, ctx: &mut Ctx, s: &str) {
    for ch in s.chars() {
        let ch = ch.to_string();
        p.event(
            MAIN,
            Event::Key(KeyInput {
                sym: ch.chars().next().unwrap() as u32,
                utf8: Some(ch.clone()),
                mods: Mods::default(),
                edit: Some(reprisekit::textedit::EditKey::Insert(ch)),
            }),
            ctx,
        );
    }
}

#[test]
fn the_text_caret_inverts_the_picture_so_a_stem_under_it_shows_white() {
    reprisekit::pins::fonts();
    let mut ctx = test_ctx();
    let mut p = Paint::new(None);
    p.doc.img = Bitmap::new(160, 100, WHITE);
    p.doc.img.fill_rect(Rect::new(10, 10, 1, 4), BLACK);
    p.doc.primary = 0xFFFF_0000;
    p.doc.opts.opaque = false;
    pick(&mut p, &mut ctx, Tool::Text);
    p.text.font = textbox::Font {
        family: "Tahoma".into(),
        px: 13,
        bold: false,
    };
    drag(&mut p, &mut ctx, (10, 10), (90, 25), BTN_LEFT);
    let fit = p.text.fit(p.doc.text_box.unwrap());
    assert_eq!(fit.x, 10, "the box starts on the stem");
    let k = p.text.caret(fit.width);
    assert_eq!((k.x, k.y), (0, 0), "an empty box has its caret at 0");
    let (off, on) = (p.shown(fit, false), p.shown(fit, true));
    for y in 0..fit.height {
        for x in 0..fit.width {
            let was = off.get(x, y).unwrap();
            let want = if x == 0 && y < k.height {
                was ^ 0x00FF_FFFF
            } else {
                was
            };
            assert_eq!(on.get(x, y), Some(want), "({x}, {y})");
        }
    }
    assert_eq!(off.get(0, 0), Some(BLACK), "the stem shows through the box");
    assert_eq!(on.get(0, 0), Some(WHITE), "and shows white under the caret");
    let foot = k.height - 1;
    assert_eq!(off.get(0, foot), Some(WHITE));
    assert_eq!(
        on.get(0, foot),
        Some(BLACK),
        "below the stem the caret is black, not the primary colour"
    );
}

#[test]
fn typed_text_wraps_at_the_box_and_lands_with_the_same_breaks() {
    reprisekit::pins::fonts();
    let mut ctx = test_ctx();
    let mut p = Paint::new(None);
    p.doc.img = Bitmap::new(160, 100, WHITE);
    pick(&mut p, &mut ctx, Tool::Text);
    p.text.font = textbox::Font {
        family: "Tahoma".into(),
        px: 13,
        bold: false,
    };
    drag(&mut p, &mut ctx, (10, 10), (90, 25), BTN_LEFT);
    let b = p.doc.text_box.unwrap();
    typed(&mut p, &mut ctx, "the quick brown fox jumps over");
    let lh = p.text.line_h();
    let fit = p.text.fit(b);
    assert_eq!(p.text.edit.lines().len(), 1, "one line, wrapped");
    assert!(fit.height >= 3 * lh, "it wraps into rows: {fit:?}");
    assert_eq!(fit.width, b.width, "the box keeps its width");
    let last = fit.height - lh;
    assert_eq!(
        p.text.caret(b.width).y,
        last,
        "the caret ends on the last row"
    );
    p.event(MAIN, key(reprisekit::textedit::keysym::UP, false), &mut ctx);
    assert_eq!(p.text.caret(b.width).y, last - lh, "Up goes up a row");
    let shown = p.shown(fit, false);
    for row in 0..3 {
        let inked = (0..fit.width)
            .flat_map(|x| (row * lh..(row + 1) * lh).map(move |y| (x, y)))
            .filter(|&(x, y)| shown.get(x, y).is_some_and(|v| v != WHITE))
            .count();
        assert!(inked > 10, "row {row} holds text: {inked} pixels");
    }
    drag(&mut p, &mut ctx, (150, 90), (150, 90), BTN_LEFT);
    assert!(p.doc.text_box.is_none());
    for y in 0..fit.height {
        for x in 0..fit.width {
            assert_eq!(
                p.doc.img.get(fit.x + x, fit.y + y),
                shown.get(x, y),
                "the picture holds the box's rows at ({x}, {y})"
            );
        }
    }
    let mut c = Canvas::new(160, 100);
    for y in 0..100 {
        for x in 0..160 {
            c.put_raw(x, y, p.doc.img.get(x, y).unwrap());
        }
    }
    check_pins("textbox", &[("wrapped".into(), hash(&c))]);
}

#[test]
fn a_click_opens_a_line_tall_box_that_widens_as_text_is_typed() {
    reprisekit::pins::fonts();
    let mut ctx = test_ctx();
    let mut p = Paint::new(None);
    p.doc.img = Bitmap::new(160, 100, WHITE);
    pick(&mut p, &mut ctx, Tool::Text);
    p.text.font = textbox::Font {
        family: "Tahoma".into(),
        px: 13,
        bold: false,
    };
    drag(&mut p, &mut ctx, (20, 10), (20, 10), BTN_LEFT);
    let lh = p.text.line_h();
    let open = p.text.fit(p.doc.text_box.expect("a click opens a box"));
    assert_eq!(
        (open.x, open.y, open.height),
        (20, 10, lh),
        "one line tall at the click"
    );
    assert!(open.width >= 13, "room for a character: {open:?}");
    typed(&mut p, &mut ctx, "Hi there friend");
    let one = p.text.fit(p.doc.text_box.unwrap());
    assert!(one.width > open.width, "the box widens: {one:?}");
    assert_eq!((one.x, one.height), (20, lh), "one row, still at the click");
    let shown = p.shown(one, false);
    let inked = (0..one.width)
        .flat_map(|x| (0..lh).map(move |y| (x, y)))
        .filter(|&(x, y)| shown.get(x, y).is_some_and(|v| v != WHITE))
        .count();
    assert!(inked > 30, "the text shows: {inked} pixels");
    typed(&mut p, &mut ctx, " and some more words");
    let wide = p.text.fit(p.doc.text_box.unwrap());
    assert_eq!(wide.right(), 160, "it stops at the picture's right edge");
    assert!(wide.height >= 2 * lh, "and wraps there: {wide:?}");
    p.event(
        MAIN,
        key(reprisekit::textedit::keysym::RETURN, false),
        &mut ctx,
    );
    let down = p.text.fit(p.doc.text_box.unwrap());
    assert_eq!(down.height, wide.height + lh, "Enter adds a row below");
    drag(&mut p, &mut ctx, (150, 90), (150, 90), BTN_LEFT);
    assert!(p.doc.text_box.is_none());
    assert!(p.doc.hist.can_undo(), "the text is on the picture");
}

#[test]
fn the_magnifiers_zoom_labels_keep_the_100_percent_font_at_every_text_size() {
    reprisekit::pins::fonts();
    let mut got = Vec::new();
    for s in [1, 2] {
        let mut at_100 = None;
        for pct in [100, 150, 200] {
            let mut ctx = test_ctx();
            ctx.size = (480, 480);
            ctx.theme = Theme::default().with_text_size(pct);
            ctx.text_size = pct;
            for n in view::MAGNIFY {
                let g = view::zoom_glyph(&format!("{n}x"), s, &ctx);
                let (w, h) = (32 * s, 12 * s);
                let inked = |x: i32, y: i32| g.pixels[(y * w + x) as usize] >> 24 != 0;
                assert!(
                    (0..w).all(|x| !inked(x, 0) && !inked(x, h - 1))
                        && (0..h).all(|y| !inked(0, y) && !inked(w - 1, y)),
                    "{n}x at {pct}% and scale {s} keeps clear of its cell's edges"
                );
            }
            let mut p = Paint::new(None);
            pick(&mut p, &mut ctx, Tool::Magnifier);
            let r = toolbox::layout(&p.toolbox(1, &ctx), p.lay(&ctx).tools).well;
            let mut c = Canvas::with_scale(480, 480, s);
            ctx.at_scale(s, |ctx| p.draw(MAIN, &mut c, ctx));
            if std::env::var_os("REPRISE_PAINT_PREVIEW").is_some() {
                reprisekit::raster::write_preview(&c, &format!("paint-zoom-{pct}-{s}"));
            }
            let well = hash(&c.copy_rect(r));
            assert_eq!(
                *at_100.get_or_insert(well),
                well,
                "the zoom labels at {pct}% and scale {s} draw as at 100%"
            );
            got.push((format!("{pct}% {s}"), well));
        }
    }
    check_pins("zoom", &got);
}

#[test]
fn opened_and_saved_pictures_go_to_the_recent_files_as_paint_s() {
    let dir = tempfile::tempdir().unwrap();
    let xbel = dir.path().join("recently-used.xbel");
    let (opened, saved_as) = (dir.path().join("opened.png"), dir.path().join("saved.png"));
    file::save(&Bitmap::new(3, 3, WHITE), &opened, file::Kind::Png, 100).unwrap();
    let mut ctx = test_ctx();
    let mut p = Paint::new(None);
    p.recent = Recent::at(DESKTOP, Some(xbel.clone()));
    p.open_driven(Driven::Picked(opened.clone()));
    p.recent.settle();
    let mut recent = reprisefs::recent::RecentFiles::at(Some(xbel.clone()));
    assert_eq!(recent.current(10), std::slice::from_ref(&opened));

    std::fs::remove_file(&xbel).unwrap();
    drag(&mut p, &mut ctx, (1, 1), (2, 1), BTN_LEFT);
    p.save();
    p.saveas_driven(Driven::Picked(saved_as.clone()));
    p.recent.settle();
    let marks = recent.bookmarks().to_vec();
    let mut paths: Vec<_> = marks.iter().map(|b| b.path.clone()).collect();
    paths.sort();
    assert_eq!(paths, [opened, saved_as], "Save and Save As record");
    for b in &marks {
        assert!(
            (b.apps.iter()).any(|(_, exec)| exec.starts_with("'reprise-paint ")),
            "recorded as Paint's: {b:?}"
        );
    }
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

#[test]
fn saving_a_read_only_picture_says_so_and_ok_opens_save_as_on_its_name() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let pic = dir.path().join("dunes.bmp");
    file::save(&Bitmap::new(3, 3, WHITE), &pic, file::Kind::Bmp, 100).unwrap();
    std::fs::set_permissions(&pic, std::fs::Permissions::from_mode(0o400)).unwrap();
    // Root writes through any mode, so there is nothing to check.
    if std::fs::OpenOptions::new().write(true).open(&pic).is_ok() {
        return;
    }
    let bytes = std::fs::read(&pic).unwrap();
    let mut p = Paint::new(Some(pic.clone()));
    p.doc.dirty = true;
    let reply = p.answered(Prompt::SaveChanges(Then::Quit), Some(0));
    assert!(!reply.quit);
    let (what, _) = prompt_up(&p).expect("the read-only box");
    assert_eq!(what, Prompt::ReadOnly(Some(Then::Quit)));
    assert!(prompt_text(&p).is_some_and(|t| t.starts_with("dunes.bmp is read-only.\n")));
    p.prompt.close();
    p.answered(what, Some(0));
    let dlg = p.saveas.as_ref().expect("OK opens Save As");
    assert_eq!((dlg.file_name(), dlg.filter()), ("dunes.bmp", 2));
    assert_eq!(p.after_save, Some(Then::Quit));
    assert_eq!(std::fs::read(&pic).unwrap(), bytes);
}

#[test]
fn save_as_onto_a_read_only_picture_says_so_before_asking_to_replace_it() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let pic = dir.path().join("dunes.png");
    file::save(&Bitmap::new(3, 3, WHITE), &pic, file::Kind::Png, 100).unwrap();
    std::fs::set_permissions(&pic, std::fs::Permissions::from_mode(0o400)).unwrap();
    // Root writes through any mode, so there is nothing to check.
    if std::fs::OpenOptions::new().write(true).open(&pic).is_ok() {
        return;
    }
    let bytes = std::fs::read(&pic).unwrap();
    let mut p = Paint::new(None);
    p.saveas_dialog();
    p.saveas_driven(Driven::Picked(pic.clone()));
    let (what, _) = prompt_up(&p).expect("a box");
    assert_eq!(what, Prompt::Error);
    assert!(prompt_text(&p).is_some_and(|t| t.starts_with("dunes.png is read-only.\n")));
    assert!(p.saveas.is_some());
    assert_eq!(std::fs::read(&pic).unwrap(), bytes);
}

/// `dunes.bmp`, read-only, in a fresh folder; None under root, which writes through any mode.
fn read_only_picture() -> Option<(tempfile::TempDir, PathBuf)> {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let pic = dir.path().join("dunes.bmp");
    file::save(&Bitmap::new(3, 3, WHITE), &pic, file::Kind::Bmp, 100).unwrap();
    std::fs::set_permissions(&pic, std::fs::Permissions::from_mode(0o400)).unwrap();
    let writable = std::fs::OpenOptions::new().write(true).open(&pic).is_ok();
    (!writable).then_some((dir, pic))
}

#[test]
fn saving_a_copy_out_of_a_zip_asks_to_update_it_and_no_opens_save_as() {
    let Some((dir, pic)) = read_only_picture() else {
        return;
    };
    let bytes = std::fs::read(&pic).unwrap();
    let mut p = Paint::new(Some(pic.clone()));
    p.zip_entry = Some((dir.path().join("Photos.zip"), "dunes.bmp".into()));
    p.doc.dirty = true;
    p.answered(Prompt::SaveChanges(Then::Quit), Some(0));
    let (what, _) = prompt_up(&p).expect("the update box");
    assert_eq!(what, Prompt::Update(Some(Then::Quit)));
    assert_eq!(
        prompt_text(&p).as_deref(),
        Some(
            "dunes.bmp came out of the compressed folder Photos.zip.\n\
             Do you want to update Photos.zip with your changes?"
        )
    );
    p.prompt.close();
    p.answered(what, Some(1));
    let dlg = p.saveas.as_ref().expect("No opens Save As");
    assert_eq!(dlg.file_name(), "dunes.bmp");
    assert_eq!(p.after_save, Some(Then::Quit));
    assert_eq!(std::fs::read(&pic).unwrap(), bytes);
}

#[test]
fn yes_on_the_update_box_says_why_a_zip_cannot_be_written() {
    let Some((dir, pic)) = read_only_picture() else {
        return;
    };
    let zip = dir.path().join("Photos.zip");
    std::fs::write(&zip, "not a zip").unwrap();
    let mut p = Paint::new(Some(pic));
    p.zip_entry = Some((zip.clone(), "dunes.bmp".into()));
    p.doc.dirty = true;
    p.save();
    let (what, _) = prompt_up(&p).expect("the update box");
    assert_eq!(what, Prompt::Update(None));
    p.prompt.close();
    let reply = p.answered(what, Some(0));
    assert!(!reply.quit);
    assert_eq!(
        prompt_text(&p).as_deref(),
        Some("Cannot update Photos.zip: only a zip compressed folder can be written into.")
    );
    assert!(p.doc.dirty, "the changes are kept");
    assert_eq!(std::fs::read(&zip).unwrap(), b"not a zip");
}
