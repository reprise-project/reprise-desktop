use super::*;
use crate::clipboard::{TEXT_MIME, pasted_text, read_capped, text_mime};
use crate::deliver::{blink_target, input_blocked, modal_over, redraws, root_of, top_modal};
use crate::draw::{buffer_damage, copy_pixels, next_buffer, surface_damage};
use crate::reply::spawn_then;
use crate::runloop::turn;
use crate::sizing::{
    MAX_SIDE, at_pick, configured_size, draw_scale, held_size, opening_size, rebounded, regrown,
    sane_side,
};
use crate::windows::{
    Placement, flyout_wanted, main_wanted, output_containing, placement, reconcile,
};
use reprisefs::pipe::PasteBuf;
use reprisekit::icontheme::{IconCache, IconTheme};
use reprisekit::raster::Rect;
use reprisekit::text::TextRenderer;
use reprisekit::theme::Theme;
use smithay_client_toolkit::data_device_manager::ReadPipe;
use smithay_client_toolkit::reexports::calloop::EventLoop;
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;
use wayland_client::Connection;

fn types(t: &[&str]) -> Vec<String> {
    t.iter().map(|s| s.to_string()).collect()
}

#[test]
fn a_selection_offering_any_text_type_reads_its_best_one() {
    let browser = types(&["text/html", "text/plain", TEXT_MIME]);
    assert_eq!(text_mime(&browser), Some(TEXT_MIME));
    let x11 = types(&["TARGETS", "STRING", "UTF8_STRING"]);
    assert_eq!(text_mime(&x11), Some("UTF8_STRING"));
    assert_eq!(text_mime(&types(&["TEXT"])), Some("TEXT"));
    assert_eq!(text_mime(&types(&["STRING"])), Some("STRING"));
}

#[test]
fn a_string_paste_reads_as_latin1_and_the_others_as_utf8() {
    let buf = |b: &[u8]| {
        let mut buf = PasteBuf::default();
        assert!(buf.push(b));
        buf
    };
    assert_eq!(pasted_text("STRING", buf(b"caf\xe9")), "caf\u{e9}");
    assert_eq!(
        pasted_text("UTF8_STRING", buf("caf\u{e9}".as_bytes())),
        "caf\u{e9}"
    );
    assert_eq!(
        pasted_text("TEXT", buf("caf\u{e9}".as_bytes())),
        "caf\u{e9}"
    );
}

#[test]
fn an_image_or_files_on_the_clipboard_is_no_text() {
    assert_eq!(text_mime(&types(&["image/png", "image/bmp"])), None);
    assert_eq!(
        text_mime(&types(&["text/uri-list", "x-special/gnome-copied-files"])),
        None
    );
    assert_eq!(text_mime(&types(&["text/html"])), None);
    assert_eq!(text_mime(&[]), None);
}

#[test]
fn reply_merge_keeps_primary() {
    let first = Reply {
        copy_primary: Some("old".into()),
        paste_primary: false,
        ..Default::default()
    };
    let then = Reply {
        copy_primary: Some("new".into()),
        paste_primary: true,
        ..Default::default()
    };
    let merged = first.clone().and(then);
    assert_eq!(merged.copy_primary.as_deref(), Some("old"));
    assert!(merged.paste_primary);
    let filled = Reply::default().and(Reply {
        copy_primary: Some("new".into()),
        ..Default::default()
    });
    assert_eq!(filled.copy_primary.as_deref(), Some("new"));
}

#[test]
fn damage_rects_scale_to_buffer() {
    let rects = [Rect::new(1, 2, 3, 4), Rect::new(0, 0, 10, 5)];
    assert_eq!(buffer_damage(&rects, 2), vec![(2, 4, 6, 8), (0, 0, 20, 10)]);
}

#[test]
fn only_a_kept_canvas_or_a_blink_damages_part_of_the_buffer() {
    let rects = Some(vec![Rect::new(1, 2, 3, 4)]);
    let whole = vec![(0, 0, 40, 20)];
    let damage = |rects, fresh, partial| surface_damage(rects, fresh, partial, 2, (40, 20));
    assert_eq!(damage(rects.clone(), true, true), vec![(2, 4, 6, 8)]);
    assert_eq!(
        damage(rects.clone(), true, false),
        whole,
        "a canvas cleared for the draw damages the whole buffer"
    );
    assert_eq!(damage(rects, false, true), whole, "so does a new canvas");
    assert_eq!(damage(None, true, true), whole);
}

#[test]
fn a_spawned_program_is_handed_over_unreaped_as_this_process_s_child() {
    let (tx, rx) = std::sync::mpsc::channel();
    spawn_then(&["true".into()], None, None, move |pid| {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
        let ppid = stat
            .rsplit(") ")
            .next()
            .and_then(|rest| rest.split(' ').nth(1));
        tx.send(ppid.map(str::to_owned)).unwrap();
    })
    .unwrap();
    let ppid = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
    assert_eq!(ppid, Some(std::process::id().to_string()));
}

struct Picker {
    dlg: reprisekit::controls::filedlg::FileDlg,
    host: filedlg::Host,
}

impl App for Picker {
    fn title(&self) -> String {
        String::new()
    }
    fn app_id(&self) -> String {
        String::new()
    }
    fn initial_size(&self) -> (i32, i32) {
        (600, 400)
    }
    fn event(&mut self, _: WinId, _: Event, _: &mut Ctx) -> Reply {
        Reply::default()
    }
    fn draw(&mut self, _: WinId, _: &mut Canvas, _: &Ctx) {}
}

impl crate::Extended for Picker {
    fn drop_file_dialog(&mut self, _: WinId) -> Option<FileDrop<'_>> {
        Some(FileDrop {
            dlg: &mut self.dlg,
            host: &mut self.host,
        })
    }
}

#[test]
fn a_file_dialogs_views_menu_is_the_windows_popup_while_open() {
    use reprisekit::controls::filedlg::{DlgMode, Fields, FileDlg, Hit};
    let ctx = Ctx {
        theme: Theme::default(),
        text: Rc::new(TextRenderer::load(&reprisekit::theme::FontConfig::default())),
        icons: reprisekit::icontheme::IconCache::new(reprisekit::icontheme::IconTheme::load(
            "test",
            &[],
        )),
        caret_on: true,
        size: (600, 400),
        maximized: false,
        origin: (0, 0),
        wake: Waker::noop(),
        heads: Default::default(),
        want_tick: Default::default(),
        output_size: None,
        at_pick: false,
        text_size: 100,
    };
    let fields = Fields {
        title: "Open".into(),
        accept_label: "Open".into(),
        filters: vec!["All Files (*.*)".into()],
        mode: DlgMode::Open,
        multiple: false,
    };
    let mut app = Picker {
        dlg: FileDlg::new(fields, vec![]),
        host: filedlg::Host::default(),
    };
    let spots = |app: &mut Picker| app.popup(MAIN, &ctx).map(|h| h.spots().len());
    assert_eq!(spots(&mut app), Some(0), "nothing to show while closed");
    let (w, h) = app.dlg.client();
    app.dlg.click(
        Rect::new(0, 0, w, h),
        Some(Hit::ToolBar(3)),
        reprisekit::selection::Mods::default(),
    );
    assert_eq!(spots(&mut app), Some(1));
}

struct Kept {
    kept: (i32, i32),
    min: Option<(i32, i32)>,
    fixed: bool,
}

impl App for Kept {
    fn title(&self) -> String {
        String::new()
    }
    fn app_id(&self) -> String {
        String::new()
    }
    fn initial_size(&self) -> (i32, i32) {
        self.kept
    }
    fn event(&mut self, _: WinId, _: Event, _: &mut Ctx) -> Reply {
        Reply::default()
    }
    fn draw(&mut self, _: WinId, _: &mut Canvas, _: &Ctx) {}
}

impl crate::Extended for Kept {
    fn fixed(&self) -> bool {
        self.fixed
    }
    fn min_size(&self, _: WinId) -> Option<(i32, i32)> {
        self.min
    }
}

#[test]
fn a_kept_size_under_the_minimum_opens_at_the_minimum() {
    let app = |kept, min, fixed| Kept { kept, min, fixed };
    let min = Some((544, 528));
    assert_eq!(
        opening_size(&app((760, 500), min, false)),
        ((760, 528), (544, 528))
    );
    assert_eq!(
        opening_size(&app((900, 700), min, false)),
        ((900, 700), (544, 528))
    );
    assert_eq!(
        opening_size(&app((50, 20), None, false)),
        (MIN_SIZE, MIN_SIZE)
    );
    assert_eq!(
        opening_size(&app((300, 200), min, true)),
        ((300, 200), (300, 200)),
        "a fixed window is pinned to its own size"
    );
}

#[test]
fn the_draw_scale_rounds_the_fraction_up_and_fits_max_side() {
    assert_eq!(draw_scale(Some(180), 1, (640, 480)), 2);
    assert_eq!(draw_scale(Some(150), 1, (640, 480)), 2);
    assert_eq!(draw_scale(Some(240), 1, (640, 480)), 2);
    assert_eq!(draw_scale(Some(120), 2, (640, 480)), 1, "the fraction wins");
    assert_eq!(draw_scale(None, 3, (640, 480)), 3);
    assert_eq!(draw_scale(None, 0, (640, 480)), 1);
    assert_eq!(
        draw_scale(None, 4, (MAX_SIDE / 2, 10)),
        2,
        "a side past MAX_SIDE"
    );
    assert_eq!(draw_scale(None, 4, (MAX_SIDE, 10)), 1);
}

/// A program at the text size has its forms theme as its own theme, the text grown once;
/// any other program grows its 100% theme for its forms.
#[test]
fn forms_grow_the_text_once() {
    let theme = Theme::default();
    let mut ctx = Ctx {
        theme: theme.clone(),
        text: Rc::new(TextRenderer::load(&theme.fonts)),
        icons: IconCache::new(IconTheme::load("test", &[])),
        caret_on: true,
        size: (40, 30),
        maximized: false,
        origin: (0, 0),
        wake: Waker::noop(),
        heads: Default::default(),
        want_tick: Default::default(),
        output_size: None,
        at_pick: false,
        text_size: 150,
    };
    let grown = reprisekit::theme::TextUnits::new(17, 11);
    assert_eq!(ctx.forms().fonts.size_px, 17);
    assert_eq!(ctx.forms().text, grown);
    ctx.theme = theme.with_text_size(150);
    assert_eq!(ctx.forms().fonts.size_px, 17);
    assert_eq!(ctx.forms().text, grown);
}

/// A text-sized program's theme is already at 150%: its form units, and the px a form drawn
/// at them hit-tests with, grow the 100% font once.
#[test]
fn a_text_sized_theme_grows_form_units_once() {
    let theme = Theme::default();
    let ctx = Ctx {
        theme: theme.with_text_size(150),
        text: Rc::new(TextRenderer::load(&theme.fonts)),
        icons: IconCache::new(IconTheme::load("test", &[])),
        caret_on: true,
        size: (40, 30),
        maximized: false,
        origin: (0, 0),
        wake: Waker::noop(),
        heads: Default::default(),
        want_tick: Default::default(),
        output_size: None,
        at_pick: false,
        text_size: 150,
    };
    assert_eq!(ctx.base_px(), 11);
    assert_eq!(ctx.form_units(), reprisekit::theme::TextUnits::new(17, 11));
    assert_eq!(ctx.form_units().px(ctx.base_px()), 17);
}

#[test]
fn a_window_at_scale_2_paints_glass_parts_at_2() {
    use reprisekit::interface::Interface;
    use reprisekit::look::widgets::draw_close_box;
    use reprisekit::raster::{Canvas, Rect};
    let theme = reprisekit::interfaces::theme_for(Interface::Glass);
    let mut ctx = Ctx {
        theme: theme.clone(),
        text: Rc::new(TextRenderer::load(&theme.fonts)),
        icons: IconCache::new(IconTheme::load("test", &[])),
        caret_on: true,
        size: (40, 30),
        maximized: false,
        origin: (0, 0),
        wake: Waker::noop(),
        heads: Default::default(),
        want_tick: Default::default(),
        output_size: None,
        at_pick: false,
        text_size: 100,
    };
    let r = Rect::new(5, 4, 30, 20);
    let mut program = Canvas::with_scale(40, 30, 2);
    ctx.at_scale(2, |ctx| draw_close_box(&mut program, &ctx.theme.paint(), r));
    let two = theme.scaled(2);
    let mut compositor = Canvas::new(80, 60);
    draw_close_box(&mut compositor, &two.paint(), r.scaled(2));
    assert!(
        program.pixels == compositor.pixels,
        "the program's device pixels are the compositor's"
    );
    let mut blocks = Canvas::with_scale(40, 30, 2);
    draw_close_box(&mut blocks, &theme.paint(), r);
    assert!(
        program.pixels != blocks.pixels,
        "not the scale-1 part in 2x2 blocks"
    );
    let split = (0..30_usize).any(|y| {
        (0..40).any(|x| {
            let at = |dx: usize, dy: usize| program.pixels[(2 * y + dy) * 80 + 2 * x + dx];
            [at(1, 0), at(0, 1), at(1, 1)]
                .iter()
                .any(|&p| p != at(0, 0))
        })
    });
    assert!(split, "some 2x2 block of device px is not one colour");
    assert!(ctx.theme.look == theme.look, "the scale-1 look is back");
}

#[test]
fn a_window_at_scale_2_paints_the_blue_status_bar_at_2() {
    use reprisekit::interface::Interface;
    use reprisekit::look::widgets::statusbar::draw_bar;
    use reprisekit::raster::{Canvas, Rect};
    let theme = reprisekit::interfaces::theme_for(Interface::Blue);
    let mut ctx = Ctx {
        theme: theme.clone(),
        text: Rc::new(TextRenderer::load(&theme.fonts)),
        icons: IconCache::new(IconTheme::load("test", &[])),
        caret_on: true,
        size: (120, 30),
        maximized: false,
        origin: (0, 0),
        wake: Waker::noop(),
        heads: Default::default(),
        want_tick: Default::default(),
        output_size: None,
        at_pick: false,
        text_size: 100,
    };
    let r = Rect::new(0, 8, 120, 22);
    let mut program = Canvas::with_scale(120, 30, 2);
    ctx.at_scale(2, |ctx| draw_bar(&mut program, &ctx.theme.paint(), r));
    let mut compositor = Canvas::new(240, 60);
    draw_bar(&mut compositor, &theme.scaled(2).paint(), r.scaled(2));
    assert!(
        program.pixels == compositor.pixels,
        "the program's device pixels are the compositor's"
    );
    let mut blocks = Canvas::with_scale(120, 30, 2);
    draw_bar(&mut blocks, &theme.paint(), r);
    assert!(
        program.pixels != blocks.pixels,
        "not the scale-1 bar in 2x2 blocks"
    );
}

#[test]
fn a_modal_child_gates_input_but_not_the_flyout() {
    let key = Event::Pointer(PointerInput::Leave);
    assert!(input_blocked(
        &key,
        Some(WinId::named("error")),
        MAIN,
        false
    ));
    assert!(!input_blocked(
        &key,
        Some(WinId::named("error")),
        WinId::named("error"),
        false
    ));
    assert!(
        !input_blocked(
            &key,
            Some(WinId::named("error")),
            WinId::named("flyout"),
            true
        ),
        "a flyout covers the modal child, so it must still take input"
    );
    assert!(!input_blocked(
        &Event::Resized,
        Some(WinId::named("error")),
        MAIN,
        false
    ));
    assert!(!input_blocked(&key, None, MAIN, false));
}

#[test]
fn only_the_windows_a_reply_draws_again_are_read_again() {
    let none = Reply::none();
    assert!(!redraws(&none, MAIN, MAIN), "a reply that draws nothing");
    let own = Reply::redraw();
    assert!(redraws(&own, WinId::named("dlg"), WinId::named("dlg")));
    assert!(!redraws(&own, WinId::named("dlg"), MAIN), "another window");
    let named = Reply {
        redraw_wins: vec![MAIN],
        ..Reply::none()
    };
    assert!(redraws(&named, WinId::named("dlg"), MAIN));
    assert!(!redraws(&named, WinId::named("dlg"), WinId::named("dlg")));
    let all = Reply {
        redraw_all: true,
        ..Reply::none()
    };
    assert!(redraws(&all, WinId::named("dlg"), MAIN));
}

#[test]
fn a_file_uri_names_a_local_absolute_path_only() {
    let list = b"file:///tmp/a%20b\r\nfile://localhost/etc/x\r\nfile://LOCALHOST/y\r\n\
        file://other/etc/passwd\r\nfile://host\r\nfile:relative\r\n";
    assert_eq!(
        reprisefs::fileclip::decode(URI_LIST, list).paths,
        vec![
            PathBuf::from("/tmp/a b"),
            PathBuf::from("/etc/x"),
            PathBuf::from("/y")
        ]
    );
}

#[test]
fn a_modal_child_refuses_drags_over_the_windows_behind_it() {
    let enter = Event::DragEnter {
        x: 0,
        y: 0,
        offered: Offered::default(),
        source: Actions::ALL,
        action: None,
        own: false,
    };
    let motion = Event::DragMotion {
        x: 0,
        y: 0,
        action: None,
    };
    assert!(input_blocked(
        &enter,
        Some(WinId::named("error")),
        MAIN,
        false
    ));
    assert!(input_blocked(
        &motion,
        Some(WinId::named("error")),
        MAIN,
        false
    ));
    let drop = Event::DragDrop {
        x: 0,
        y: 0,
        data: Dropped::Text(String::new()),
        action: DragAction::Copy,
        source: Actions::ALL,
    };
    assert!(input_blocked(
        &drop,
        Some(WinId::named("error")),
        MAIN,
        false
    ));
    assert!(!input_blocked(
        &enter,
        Some(WinId::named("error")),
        WinId::named("error"),
        false
    ));
    assert!(!input_blocked(
        &Event::DragLeave,
        Some(WinId::named("error")),
        MAIN,
        false
    ));
}

#[test]
fn the_newest_modal_child_is_the_one_that_gates() {
    let slots = [
        (MAIN, false),
        (WinId::named("open"), true),
        (WinId::named("find"), false),
        (WinId::named("error"), true),
    ];
    assert_eq!(top_modal(slots.into_iter()), Some(WinId::named("error")));
    assert_eq!(
        top_modal(slots[..3].iter().copied()),
        Some(WinId::named("open"))
    );
    assert_eq!(top_modal([(MAIN, false)].into_iter()), None);
}

#[test]
fn a_modal_child_gates_only_the_windows_it_sits_over() {
    const CONFIRM: WinId = WinId::named("confirm");
    const SECOND: WinId = WinId::named("second");
    let parent = |w: WinId| match w {
        CONFIRM => Some(SECOND),
        SECOND => None,
        _ => Some(MAIN),
    };
    assert_eq!(
        root_of(WinId::named("confirm"), parent),
        WinId::named("second")
    );
    assert_eq!(root_of(WinId::named("find"), parent), MAIN);
    assert_eq!(root_of(MAIN, parent), MAIN);
    let slots = [
        (MAIN, false),
        (WinId::named("second"), false),
        (WinId::named("confirm"), true),
    ];
    assert_eq!(modal_over(MAIN, slots.into_iter(), parent), None);
    assert_eq!(
        modal_over(WinId::named("second"), slots.into_iter(), parent),
        Some(WinId::named("confirm"))
    );
    assert_eq!(
        modal_over(WinId::named("confirm"), slots.into_iter(), parent),
        Some(WinId::named("confirm"))
    );
}

#[test]
fn a_child_waits_for_its_parents_first_frame() {
    assert_eq!(placement(Some(MAIN), Some(false)), Placement::Wait);
    assert_eq!(placement(Some(MAIN), Some(true)), Placement::Under(MAIN));
    assert_eq!(
        placement(Some(MAIN), None),
        Placement::Alone,
        "a hidden main window parents nothing"
    );
    assert_eq!(placement(None, Some(false)), Placement::Alone);
}

#[test]
fn numbered_window_ids_never_meet_named_ones() {
    assert_eq!(window_id(3), window_id(3));
    assert_ne!(window_id(3), window_id(4));
    assert_ne!(window_id(0), MAIN);
}

#[test]
fn reply_merge_joins_the_windows_to_redraw_and_keeps_the_first_raise() {
    let first = Reply {
        redraw_wins: vec![WinId::named("a")],
        raise: Some(WinId::named("a")),
        ..Default::default()
    };
    let then = Reply {
        redraw_wins: vec![WinId::named("b")],
        raise: Some(WinId::named("b")),
        ..Default::default()
    };
    let merged = first.and(then);
    assert_eq!(merged.redraw_wins, [WinId::named("a"), WinId::named("b")]);
    assert_eq!(merged.raise, Some(WinId::named("a")));
}

#[test]
fn a_flyout_is_made_dropped_or_left() {
    let f = Flyout {
        id: WinId::named("flyout"),
        at: (10, 20),
        title: "Volume",
    };
    assert_eq!(flyout_wanted(Some(f), None), (None, Some(f)));
    assert_eq!(
        flyout_wanted(None, Some(WinId::named("flyout"))),
        (Some(WinId::named("flyout")), None)
    );
    assert_eq!(
        flyout_wanted(Some(f), Some(WinId::named("flyout"))),
        (None, None)
    );
    let g = Flyout {
        id: WinId::named("other"),
        at: (0, 0),
        title: "Other",
    };
    assert_eq!(
        flyout_wanted(Some(g), Some(WinId::named("flyout"))),
        (Some(WinId::named("flyout")), Some(g))
    );
}

#[test]
fn the_flyout_goes_on_the_output_under_its_point() {
    let outputs = [(0, 0, 1920, 1080), (1920, 0, 1280, 1024)];
    assert_eq!(output_containing(&outputs, (2000, 1000)), Some(1));
    assert_eq!(output_containing(&outputs, (5, 1079)), Some(0));
    assert_eq!(output_containing(&outputs, (5000, 5)), None);
    let left = [(-1920, -1080, 1920, 1080)];
    assert_eq!(output_containing(&left, Flyout::UNDER_POINTER), None);
}

#[test]
fn a_size_the_program_picked_outlasts_a_repeated_configure() {
    let remembered = (Some(400), Some(300));
    assert_eq!(
        configured_size((200, 300), remembered, (None, None), false, false),
        ((400, 300), false)
    );
    assert_eq!(
        configured_size((296, 300), remembered, remembered, true, false),
        ((296, 300), true)
    );
    assert_eq!(
        configured_size((296, 300), remembered, remembered, true, true),
        ((400, 300), false)
    );
    assert_eq!(
        configured_size((296, 300), (Some(500), Some(300)), remembered, true, false),
        ((500, 300), false)
    );
    assert_eq!(
        configured_size((296, 300), (None, None), (None, None), true, false),
        ((296, 300), true)
    );
}

/// A text size change refits an open window: a fixed one to the size its program asks
/// for; a resizable one at its program's pick to the size the program picks now, larger
/// or smaller; one the user sized keeps its size, held to at least its new minimum.
#[test]
fn an_open_window_takes_what_its_program_asks() {
    assert_eq!(
        regrown((398, 424), (398, 424), Some((597, 636)), None),
        (597, 636)
    );
    assert_eq!(regrown((900, 700), (816, 792), None, None), (900, 792));
    assert_eq!(regrown((900, 900), (816, 792), None, None), (900, 900));
    assert_eq!(
        regrown((1088, 1000), (544, 528), None, Some((760, 680))),
        (760, 680),
        "a picked size shrinks back"
    );
    assert_eq!(
        regrown((760, 680), (1088, 1056), None, Some((1088, 1056))),
        (1088, 1056),
        "and grows"
    );
}

/// A window is at its program's pick while its size is that pick held inside the bounds;
/// any other size is the user's or the compositor's.
#[test]
fn a_window_is_at_its_pick_until_sized_otherwise() {
    assert!(at_pick((1088, 1056), (1088, 1056), None));
    assert!(at_pick((1088, 1000), (1088, 1056), Some((1900, 1000))));
    assert!(!at_pick((1200, 1000), (1088, 1056), Some((1900, 1000))));
}

#[test]
fn a_window_at_its_pick_stays_at_it_under_other_bounds() {
    let (pick, small, large) = ((1088, 1056), Some((1366, 700)), Some((1914, 1026)));
    assert_eq!(rebounded((1088, 700), pick, small, large), (1088, 1026));
    assert_eq!(rebounded((1088, 1026), pick, large, small), (1088, 700));
    assert_eq!(rebounded((1088, 700), pick, small, None), pick);
    assert_eq!(
        rebounded((1000, 650), pick, small, large),
        (1000, 650),
        "a size the user picked stays"
    );
}

/// Settings sized by the compositor at 952x528 grows to its 200% minimum under the old
/// bounds, then hears the smaller ones the taller caption and taskbar leave.
#[test]
fn a_window_grown_past_new_bounds_is_held_inside_them() {
    let (pick, was, now) = ((760, 680), Some((1912, 1016)), Some((1912, 990)));
    assert_eq!(rebounded((1088, 1016), pick, was, now), (1088, 990));
}

/// A window that picks its own size stays inside the compositor's bounds: a Settings sheet
/// grown past a 1080-line screen opens as tall as the work area. A side the compositor asks
/// for (maximized, fullscreen) is its own.
#[test]
fn a_size_the_window_picks_stays_inside_the_bounds() {
    let bounds = Some((1914, 1026));
    assert_eq!(held_size((1200, 1300), (None, None), bounds), (1200, 1026));
    assert_eq!(held_size((2100, 900), (None, None), bounds), (1914, 900));
    assert_eq!(
        held_size((1920, 1080), (Some(1920), Some(1080)), bounds),
        (1920, 1080)
    );
    assert_eq!(
        held_size((1200, 1300), (Some(1200), None), bounds),
        (1200, 1026)
    );
    assert_eq!(held_size((1200, 1300), (None, None), None), (1200, 1300));
}

#[test]
fn pixels_land_in_shm_byte_order() {
    let pixels = [0xAABB_CCDDu32, 0x0102_0304];
    let mut target = [0u8; 8];
    copy_pixels(&mut target, &pixels);
    assert_eq!(target, [0xDD, 0xCC, 0xBB, 0xAA, 0x04, 0x03, 0x02, 0x01]);
    let mut short = [0u8; 4];
    copy_pixels(&mut short, &pixels);
    assert_eq!(short, [0xDD, 0xCC, 0xBB, 0xAA]);
}

fn child(id: WinId) -> Child {
    Child {
        id,
        title: format!("{id:?}"),
        size: (300, 150),
        fixed: true,
        modal: false,
    }
}

#[test]
fn a_paste_writer_that_never_writes_is_dropped_at_the_timeout() {
    use smithay_client_toolkit::reexports::calloop::EventLoop;
    use std::io::{ErrorKind, Write};

    let mut event_loop: EventLoop<Option<bool>> = EventLoop::try_new().expect("event loop");
    let (reader, mut writer) = std::io::pipe().expect("pipe");
    read_capped(
        &event_loop.handle(),
        ReadPipe::from(std::os::fd::OwnedFd::from(reader)),
        Duration::from_millis(30),
        |got: &mut Option<bool>, buf: Option<PasteBuf>| *got = Some(buf.is_some()),
    );
    let mut got = None;
    for _ in 0..10 {
        event_loop
            .dispatch(Duration::from_millis(20), &mut got)
            .expect("dispatch");
    }
    let err = writer.write(b"x").expect_err("the read end is closed");
    assert_eq!(err.kind(), ErrorKind::BrokenPipe);
    assert_eq!(got, Some(false), "a timed-out read reports no bytes");
}

#[test]
fn a_run_of_missing_shm_buffers_is_logged_once_and_told_once() {
    let mut t = crate::draw::ShmTrouble::default();
    assert!(!t.drawn(), "nothing to recover from");
    assert_eq!(t.failed(), (true, false), "the first failure is logged");
    for _ in 2..crate::draw::SHM_TELL_AFTER {
        assert_eq!(t.failed(), (false, false));
    }
    assert_eq!(t.failed(), (false, true), "the program hears of it once");
    assert_eq!(t.failed(), (false, false));
    assert!(t.drawn(), "a frame ends the run");
    assert_eq!(t.failed(), (true, false), "the next run is logged again");
}

#[test]
fn the_blink_goes_to_the_keyboard_then_the_pointer_then_nowhere() {
    assert_eq!(
        blink_target(Some(WinId::named("find")), Some(MAIN)),
        Some(WinId::named("find"))
    );
    assert_eq!(blink_target(None, Some(MAIN)), Some(MAIN));
    assert_eq!(blink_target(None, None), None);
}

#[test]
fn frames_alternate_between_two_buffers() {
    assert_eq!(next_buffer([None, None], 0), (0, true));
    assert_eq!(next_buffer([Some(false), None], 0), (1, true));
    assert_eq!(next_buffer([Some(true), Some(false)], 1), (0, false));
    assert_eq!(next_buffer([Some(false), Some(true)], 0), (1, false));
    assert_eq!(next_buffer([Some(false), Some(false)], 0), (1, true));
}

#[test]
fn a_window_side_is_clamped_between_1_and_16384() {
    assert_eq!(sane_side(i64::from(u32::MAX)), 16384);
    assert_eq!(sane_side(i64::from(1u32 << 31)), 16384);
    assert_eq!(sane_side(-300), 1);
    assert_eq!(sane_side(0), 1);
    assert_eq!(sane_side(640), 640);
}

#[test]
fn nothing_open_and_nothing_wanted_is_a_no_op() {
    let (drop, make) = reconcile(&[], &[]);
    assert!(drop.is_empty() && make.is_empty());
}

#[test]
fn a_new_child_is_created() {
    let want = [child(WinId::named("find"))];
    let (drop, make) = reconcile(&[], &want.iter().collect::<Vec<_>>());
    assert!(drop.is_empty());
    assert_eq!(make, vec![0]);
}

#[test]
fn a_child_that_went_away_is_destroyed() {
    let (drop, make) = reconcile(&[WinId::named("find")], &[]);
    assert_eq!(drop, vec![WinId::named("find")]);
    assert!(make.is_empty());
}

#[test]
fn a_child_that_is_still_wanted_is_left_alone() {
    let want = [child(WinId::named("find")), child(WinId::named("replace"))];
    let (drop, make) = reconcile(&[WinId::named("find")], &want.iter().collect::<Vec<_>>());
    assert!(drop.is_empty());
    assert_eq!(make, vec![1]);
}

#[test]
fn one_child_swapped_for_another() {
    let want = [child(WinId::named("replace"))];
    let (drop, make) = reconcile(&[WinId::named("find")], &want.iter().collect::<Vec<_>>());
    assert_eq!(drop, vec![WinId::named("find")]);
    assert_eq!(make, vec![0]);
}

#[test]
fn a_shown_main_window_that_is_missing_is_created() {
    assert_eq!(main_wanted(false, false), Some(true));
}

#[test]
fn a_hidden_main_window_that_exists_is_dropped() {
    assert_eq!(main_wanted(true, true), Some(false));
}

#[test]
fn a_main_window_that_already_matches_is_left_alone() {
    assert_eq!(main_wanted(false, true), None);
    assert_eq!(main_wanted(true, false), None);
}

#[test]
fn a_ping_from_another_thread_wakes_the_loop() {
    use smithay_client_toolkit::reexports::calloop::{EventLoop, ping::make_ping};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let mut event_loop: EventLoop<()> = EventLoop::try_new().expect("event loop");
    let (ping, source) = make_ping().expect("ping");
    let woke = Arc::new(AtomicBool::new(false));
    let flag = woke.clone();
    event_loop
        .handle()
        .insert_source(source, move |_, _, _: &mut ()| {
            flag.store(true, Ordering::SeqCst);
        })
        .expect("ping source");

    let waker = Waker(Arc::new(ping));
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(20));
        waker.wake();
    });

    for _ in 0..20 {
        event_loop
            .dispatch(Duration::from_millis(50), &mut ())
            .expect("dispatch");
        if woke.load(Ordering::SeqCst) {
            return;
        }
    }
    panic!("the ping never reached the loop");
}

#[test]
fn a_protocol_error_ends_the_loop_instead_of_spinning() {
    use std::io::Write;
    use std::os::unix::net::UnixStream;
    let (client, mut server) = UnixStream::pair().expect("socket pair");
    let conn = Connection::from_socket(client).expect("connection");
    let mut event_loop: EventLoop<()> = EventLoop::try_new().expect("event loop");
    WaylandSource::new(conn.clone(), conn.new_event_queue::<()>())
        .insert(event_loop.handle())
        .expect("wayland source");
    // wl_display.error(object 1, code 0, "boom"), then the compositor hangs up.
    let words: [u32; 7] = [1, 28 << 16, 1, 0, 5, u32::from_le_bytes(*b"boom"), 0];
    let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    server.write_all(&bytes).expect("error event");
    drop(server);
    for _ in 0..100 {
        if let Err(e) = turn(&mut event_loop, &mut (), &conn) {
            assert!(
                e.contains("protocol error"),
                "the loop ends on the protocol error, got {e}"
            );
            return;
        }
    }
    panic!("the loop kept turning after the protocol error");
}

#[test]
fn a_panicking_step_rescues_its_data_before_the_panic_goes_on() {
    let mut saved = Vec::new();
    let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        runloop::rescued(
            &mut saved,
            |s: &mut Vec<&str>| {
                s.push("typed");
                panic!("draw failed");
            },
            |s| s.push("rescued"),
        )
    }));
    assert!(out.is_err(), "the panic carries on past the rescue");
    assert_eq!(saved, ["typed", "rescued"]);
}
