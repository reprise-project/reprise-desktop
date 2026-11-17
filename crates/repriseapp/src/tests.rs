use super::*;
use crate::clipboard::{TEXT_MIME, pasted_text, read_capped, text_mime};
use crate::deliver::{
    blink_target, input_blocked, modal_over, newly_covered, orphaned, redraws, root_of,
    stands_beside, takes_blinks, top_modal,
};
use crate::draw::{buffer_damage, copy_pixels, next_buffer, surface_damage};
use crate::reply::spawn_then;
use crate::runloop::turn;
use crate::sizing::{
    Fit, MAX_SIDE, at_pick, configured_size, draw_scale, held_size, opening_size, rebounded,
    regrown, sane_side,
};
use crate::windows::{
    Placement, decoration_mode, decorations, flyout_wanted, main_wanted, modality,
    output_containing, placement, reconcile, reparent_to,
};
use reprisefs::pipe::PasteBuf;
use reprisekit::icontheme::{IconCache, IconTheme};
use reprisekit::raster::Rect;
use reprisekit::text::TextRenderer;
use reprisekit::theme::Theme;
use smithay_client_toolkit::data_device_manager::ReadPipe;
use smithay_client_toolkit::reexports::calloop::EventLoop;
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::shell::xdg::window::{DecorationMode, WindowDecorations};
use std::ffi::OsString;
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
fn a_size_is_asked_once_until_it_changes() {
    let mut asked = (300, 200);
    let mut same = Reply::none();
    assert!(!same.ask_size(&mut asked, (300, 200)));
    assert_eq!((same.size, same.redraw), (None, false));

    let mut grown = Reply::none();
    assert!(grown.ask_size(&mut asked, (450, 300)));
    assert_eq!((grown.size, grown.redraw), (Some((450, 300)), true));
    assert_eq!(asked, (450, 300));

    let mut again = Reply::none();
    assert!(!again.ask_size(&mut asked, (450, 300)));
    assert_eq!(again.size, None);
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
    spawn_then(
        &["true".into()],
        None,
        None,
        &Default::default(),
        move |pid| {
            let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
            let ppid = stat
                .rsplit(") ")
                .next()
                .and_then(|rest| rest.split(' ').nth(1));
            tx.send(ppid.map(str::to_owned)).unwrap();
        },
    )
    .unwrap();
    let ppid = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
    assert_eq!(ppid, Some(std::process::id().to_string()));
}

#[test]
fn a_spawn_runs_in_its_folder_and_one_whose_folder_is_gone_still_runs() {
    let t = tempfile::tempdir().unwrap();
    let game = t.path().join("game");
    std::fs::create_dir(&game).unwrap();
    let run = |dir: &std::path::Path, out: &std::path::Path| {
        let (tx, rx) = std::sync::mpsc::channel();
        let pwd = format!("pwd -P > '{}'", out.display());
        let argv = ["sh".into(), "-c".into(), pwd.into()];
        spawn_then(&argv, Some(dir), None, &Default::default(), move |_| {
            tx.send(()).unwrap()
        })
        .unwrap();
        rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !std::fs::read_to_string(out).is_ok_and(|s| s.ends_with('\n')) {
            assert!(std::time::Instant::now() < end, "the program ran");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        std::fs::read_to_string(out).unwrap()
    };
    let real = |p: &std::path::Path| format!("{}\n", p.canonicalize().unwrap().display());
    assert_eq!(run(&game, &t.path().join("a")), real(&game));
    let here = real(&std::env::current_dir().unwrap());
    let gone = run(&t.path().join("gone"), &t.path().join("b"));
    assert_eq!(gone, here, "the default folder");
}

#[test]
fn reply_merge_starts_each_spawn_in_the_folder_it_asked_for() {
    let spawn = |argv: &str, dir: &str| Reply {
        spawn: vec![vec![argv.into()]],
        spawn_dir: Some(dir.into()),
        ..Default::default()
    };
    let same = spawn("a", "/a").and(spawn("b", "/a"));
    assert_eq!(
        same.spawn,
        [vec![OsString::from("a")], vec![OsString::from("b")]]
    );
    assert_eq!(same.spawn_dir, Some("/a".into()));
    assert_eq!(
        spawn("a", "/a").and(Reply::default()).spawn_dir,
        Some("/a".into())
    );
    let at = |argv: &str, dir: &str| (vec![OsString::from(argv)], Some(PathBuf::from(dir)));
    let apart = spawn("a", "/a").and(spawn("b", "/b"));
    assert!(apart.spawn.is_empty());
    assert_eq!(apart.spawn_each, [at("a", "/a"), at("b", "/b")]);
    let more = apart.and(spawn("c", "/c"));
    assert_eq!(more.spawn_each, [at("a", "/a"), at("b", "/b")]);
    assert_eq!(
        (more.spawn, more.spawn_dir),
        (vec![at("c", "/c").0], at("c", "/c").1)
    );
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

#[test]
fn a_covered_file_dialog_lets_go_of_its_marquee() {
    use reprisekit::controls::filedlg::{DlgMode, Event as E, Fields, FileDlg, Hit};
    use reprisekit::controls::shellview;
    let fields = Fields {
        title: "Open".into(),
        accept_label: "Open".into(),
        filters: vec!["All Files (*.*)".into()],
        mode: DlgMode::Open,
        multiple: true,
    };
    let mut app = Picker {
        dlg: FileDlg::new(fields, vec![]),
        host: filedlg::Host::default(),
    };
    let (w, h) = app.dlg.client();
    let r = Rect::new(0, 0, w, h);
    let background = Some(Hit::Shell(shellview::Hit::Background));
    let press = |d: &mut FileDlg| d.click_at(r, background, Default::default(), (w / 2, h / 2));
    press(&mut app.dlg);
    assert!(matches!(app.dlg.release(), E::Redraw), "the band was held");
    press(&mut app.dlg);
    crate::Extended::cancel_pointer(&mut app, MAIN);
    assert!(matches!(app.dlg.release(), E::None), "nothing is held");
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

/// Kept's window, laid out at 100% and grown by its forms' own units.
struct AtHundred(Kept);

impl App for AtHundred {
    fn title(&self) -> String {
        String::new()
    }
    fn app_id(&self) -> String {
        String::new()
    }
    fn initial_size(&self) -> (i32, i32) {
        self.0.kept
    }
    fn event(&mut self, _: WinId, _: Event, _: &mut Ctx) -> Reply {
        Reply::default()
    }
    fn draw(&mut self, _: WinId, _: &mut Canvas, _: &Ctx) {}
}

impl crate::Extended for AtHundred {
    fn text_sized(&self) -> bool {
        false
    }
}

#[test]
fn a_program_gets_its_theme_at_the_text_size_unless_it_opts_out() {
    let kept = || Kept {
        kept: (100, 100),
        min: None,
        fixed: false,
    };
    let theme = Theme::default();
    let sized = crate::look::sized(theme.clone(), &kept(), 150);
    assert_eq!(sized, theme.with_text_size(150));
    let at_100 = crate::look::sized(theme.clone(), &AtHundred(kept()), 150);
    assert_eq!(at_100, theme);
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
fn a_close_request_under_a_modal_child_is_dropped_and_one_on_the_child_arrives() {
    const SHEET: WinId = WinId::named("sheet");
    const ADVANCED: WinId = WinId::named("advanced");
    let parent = |w: WinId| match w {
        ADVANCED => Some(SHEET),
        _ => Some(MAIN),
    };
    let slots = [(MAIN, false), (SHEET, false), (ADVANCED, true)];
    let close = Event::CloseRequest;
    for win in [MAIN, SHEET] {
        let modal = modal_over(win, false, slots.into_iter(), parent);
        assert_eq!(modal, Some(ADVANCED), "the box gates {win:?}");
        assert!(input_blocked(&close, modal, win, false));
    }
    let modal = modal_over(ADVANCED, false, slots.into_iter(), parent);
    assert!(!input_blocked(&close, modal, ADVANCED, false));
    assert!(
        !input_blocked(&close, Some(ADVANCED), WinId::named("flyout"), true),
        "the compositor takes a flyout away under the box too"
    );
    let free = [(MAIN, false), (SHEET, false)];
    let modal = modal_over(SHEET, false, free.into_iter(), parent);
    assert!(!input_blocked(&close, modal, SHEET, false));
}

#[test]
fn a_progress_box_takes_input_beside_a_box_over_its_parent_and_not_under_its_own() {
    const PROGRESS: WinId = WinId::named("progress");
    const SHEET: WinId = WinId::named("sheet");
    const FAILED: WinId = WinId::named("failed");
    const OWN: WinId = WinId::named("own");
    let parent = |w: WinId| match w {
        OWN => Some(PROGRESS),
        _ => Some(MAIN),
    };
    assert!(stands_beside(Caption::Prompt, false));
    assert!(!stands_beside(Caption::Prompt, true), "a modal box blocks");
    assert!(!stands_beside(Caption::Help, false), "a sheet is gated");
    assert!(!stands_beside(Caption::Window, false));

    let slots = [
        (MAIN, false),
        (PROGRESS, false),
        (SHEET, false),
        (FAILED, true),
    ];
    let modal = modal_over(PROGRESS, true, slots.into_iter(), parent);
    assert_eq!(modal, None, "the box over the main window leaves it free");
    for ev in [Event::Pointer(PointerInput::Leave), Event::CloseRequest] {
        assert!(
            !input_blocked(&ev, modal, PROGRESS, false),
            "{ev:?} arrives"
        );
    }
    for win in [MAIN, SHEET] {
        assert_eq!(
            modal_over(win, false, slots.into_iter(), parent),
            Some(FAILED),
            "the box still gates {win:?}"
        );
    }

    let own = [
        (MAIN, false),
        (PROGRESS, false),
        (FAILED, true),
        (OWN, true),
    ];
    assert_eq!(
        modal_over(PROGRESS, true, own.into_iter(), parent),
        Some(OWN),
        "a box over the progress box gates it"
    );
}

#[test]
fn a_modal_child_asks_the_compositor_for_modality_and_a_plain_one_does_not() {
    assert_eq!(modality(false, true), Some(true), "a new modal child");
    assert_eq!(modality(false, false), None, "a new plain child");
    assert_eq!(modality(true, true), None, "a kept modal child asks once");
    assert_eq!(modality(true, false), Some(false), "a box turned plain");
}

#[test]
fn a_child_whose_parent_window_leaves_is_asked_to_close_once() {
    const SHEET: WinId = WinId::named("sheet");
    const ADVANCED: WinId = WinId::named("advanced");
    const FIND: WinId = WinId::named("find");
    let parent = |w: WinId| match w {
        ADVANCED => Some(SHEET),
        _ => Some(MAIN),
    };
    let open = [MAIN, FIND, SHEET, ADVANCED];
    let left = [MAIN, FIND, ADVANCED];
    assert_eq!(orphaned(&open, &left, parent), vec![ADVANCED]);
    assert!(
        orphaned(&left, &left, parent).is_empty(),
        "asked once, while its parent stays gone"
    );
    assert!(
        orphaned(&open, &[MAIN, FIND], parent).is_empty(),
        "the box left with its sheet"
    );
    let moved = |w: WinId| (w != MAIN).then_some(MAIN);
    assert!(
        orphaned(&open, &left, moved).is_empty(),
        "the box moved over the main window as the sheet left"
    );
}

#[test]
fn a_hidden_main_window_orphans_none_of_its_children() {
    const PROGRESS: WinId = WinId::named("progress");
    const FAILED: WinId = WinId::named("failed");
    let parent = |w: WinId| (w != MAIN).then_some(MAIN);
    assert!(orphaned(&[MAIN, PROGRESS, FAILED], &[PROGRESS, FAILED], parent).is_empty());
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
    assert_eq!(modal_over(MAIN, false, slots.into_iter(), parent), None);
    assert_eq!(
        modal_over(WinId::named("second"), false, slots.into_iter(), parent),
        Some(WinId::named("confirm"))
    );
    assert_eq!(
        modal_over(WinId::named("confirm"), false, slots.into_iter(), parent),
        Some(WinId::named("confirm"))
    );
}

#[test]
fn a_modal_child_covers_the_windows_it_newly_gates() {
    const LAYOUT: WinId = WinId::named("layout");
    const ERROR: WinId = WinId::named("error");
    let free = [(MAIN, None), (LAYOUT, None)];
    let boxed = [(MAIN, Some(ERROR)), (LAYOUT, Some(ERROR)), (ERROR, None)];
    assert_eq!(newly_covered(&free, &boxed), vec![MAIN, LAYOUT]);
    let dialog = [(MAIN, Some(LAYOUT)), (LAYOUT, None)];
    assert_eq!(
        newly_covered(&dialog, &boxed),
        vec![LAYOUT],
        "the main window was gated already"
    );
    assert!(newly_covered(&boxed, &dialog).is_empty(), "the box closed");
    assert!(newly_covered(&boxed, &boxed).is_empty());
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
fn a_child_opened_while_the_main_window_is_hidden_goes_under_it_once_it_maps() {
    let set = None;
    assert_eq!(
        reparent_to(set, Some(MAIN), None),
        None,
        "hidden: it stays unparented, with no request"
    );
    assert_eq!(
        reparent_to(set, Some(MAIN), Some(false)),
        None,
        "shown, not mapped yet"
    );
    let set = reparent_to(set, Some(MAIN), Some(true)).expect("mapped: it moves");
    assert_eq!(set, Some(MAIN));
    assert_eq!(
        reparent_to(set, Some(MAIN), Some(true)),
        None,
        "under it already"
    );
    assert_eq!(
        reparent_to(set, None, None),
        Some(None),
        "no parent wanted: unparented"
    );
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

#[test]
fn a_side_at_its_pick_follows_new_bounds_while_the_other_keeps_its_length() {
    let (pick, small, large) = ((552, 532), Some((944, 462)), Some((952, 486)));
    assert_eq!(rebounded((568, 462), pick, small, large), (568, 486));
    assert_eq!(rebounded((568, 486), pick, large, small), (568, 462));
}

// Settings' bounds at output scale 2 on a 1920x1080 screen. It picks 552x532 and the
// compositor asks the 568 px width its remembered frame gives.
const CLASSIC: (i32, i32) = (952, 486);
const BLUE: (i32, i32) = (952, 476);
const GLASS: (i32, i32) = (944, 462);

/// A Settings window opened under the interface whose bounds are `bounds`: the compositor
/// asks the remembered width and as much height as the bounds hold.
fn settings_opened(bounds: (i32, i32)) -> Fit {
    let pick = (552, 532);
    let fit = Fit {
        size: pick,
        picked: pick,
        bounds: None,
        asked: (None, None),
        own: false,
    };
    fit.configured((Some(568), Some(bounds.1)), Some(bounds), false)
}

/// After a switch the compositor sends the new bounds, asking a height inside them when the
/// window no longer fits and repeating its last ask otherwise.
fn switched(fit: Fit, bounds: (i32, i32)) -> Fit {
    let last = fit.asked.1.expect("asked a height");
    let asked = (
        fit.asked.0,
        Some(if fit.size.1 > bounds.1 {
            bounds.1
        } else {
            last
        }),
    );
    fit.configured(asked, Some(bounds), false)
        .configured(asked, None, false)
}

#[test]
fn an_interface_switch_leaves_a_window_held_by_its_bounds_at_the_size_a_fresh_open_gets() {
    let looks = [("Classic", CLASSIC), ("Blue", BLUE), ("Glass", GLASS)];
    for (from, was) in looks {
        for (to, now) in looks {
            let fit = switched(settings_opened(was), now);
            assert_eq!(
                fit.size,
                settings_opened(now).size,
                "{from} to {to} is {:?}",
                settings_opened(now).size
            );
        }
    }
    let there_and_back = switched(switched(settings_opened(CLASSIC), GLASS), CLASSIC);
    assert_eq!(there_and_back.size, (568, 486), "Glass back to Classic");
}

/// At 100% the 552x532 sheet fits under every interface and keeps its size.
#[test]
fn an_interface_switch_leaves_a_window_inside_its_bounds_at_its_size() {
    let (classic, glass) = ((1912, 1016), (1904, 992));
    let fit = Fit {
        size: (552, 532),
        picked: (552, 532),
        bounds: None,
        asked: (None, None),
        own: false,
    }
    .configured((None, None), Some(classic), false);
    assert_eq!(
        fit.configured((None, None), Some(glass), false).size,
        (552, 532)
    );
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
fn only_the_keyboards_window_blinks_its_caret_and_a_waiting_tip_blinks_anywhere() {
    let find = WinId::named("find");
    assert!(takes_blinks(MAIN, true, false, Some(MAIN)));
    assert!(
        !takes_blinks(MAIN, true, false, Some(find)),
        "a caret in a window without the keyboard"
    );
    assert!(
        !takes_blinks(MAIN, true, false, None),
        "nothing has the keyboard"
    );
    assert!(takes_blinks(MAIN, false, true, None), "a label tip waits");
    assert!(!takes_blinks(MAIN, false, false, Some(MAIN)));
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

#[test]
fn only_a_window_without_a_caption_asks_for_no_frame() {
    for c in [
        Caption::Window,
        Caption::Prompt,
        Caption::Help,
        Caption::Fixed,
    ] {
        assert_eq!(decorations(c), WindowDecorations::RequestServer, "{c:?}");
        assert_eq!(decoration_mode(c), DecorationMode::Server, "{c:?}");
    }
    assert_eq!(decorations(Caption::None), WindowDecorations::RequestClient);
    assert_eq!(decoration_mode(Caption::None), DecorationMode::Client);
}
