use super::*;
use crate::commands::{Command, LayoutPart};
use crate::place::Place;
use crate::tests::{test_ctx, test_explorer, wake_until};
use reprisekit::controls::commandbar::Hit;
use reprisekit::interface::Interface;
use std::sync::Arc;

fn glass(dark: bool) -> Ctx {
    let mut ctx = test_ctx();
    let g = reprisekit::glass::Settings {
        mode: match dark {
            true => reprisekit::glass::Mode::Dark,
            false => reprisekit::glass::Mode::Light,
        },
        ..Default::default()
    };
    ctx.theme = reprisekit::interfaces::theme_with(Interface::Glass, Default::default(), false, &g);
    ctx.text = TextRenderer::load(&ctx.theme.fonts).into();
    ctx
}

/// A folder holding a text file, a binary file and a subfolder, its window with the pane shown.
fn shown(ctx: &mut Ctx) -> (tempfile::TempDir, Explorer) {
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::write(dir.path().join("notes.txt"), "first line\nsecond line\n").expect("write");
    std::fs::write(dir.path().join("other.txt"), "other file\n").expect("write");
    std::fs::write(dir.path().join("blob.bin"), [0u8, 1, 2, 0, 0xFF]).expect("write");
    std::fs::create_dir(dir.path().join("sub")).expect("mkdir");
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), ctx);
    ex.run(Command::ToggleLayout(LayoutPart::PreviewPane), ctx);
    (dir, ex)
}

fn pick(ex: &mut Explorer, names: &[&str], ctx: &mut Ctx) {
    ex.sv.selection.set.clear();
    for n in names {
        let i = (ex.items.iter())
            .position(|it| it.name == *n)
            .expect("listed");
        ex.sv.selection.set.insert(i);
    }
    ex.refresh_chrome(ctx);
}

fn note(ex: &Explorer) -> Option<String> {
    match ex.pane.as_ref()?.qv.body() {
        Body::Note(t) => Some(t.clone()),
        _ => None,
    }
}

fn text(ex: &Explorer) -> Option<String> {
    match ex.pane.as_ref()?.qv.body() {
        Body::Text(e) => Some(e.text()),
        _ => None,
    }
}

#[test]
fn the_pane_names_nothing_picked_a_folder_and_several_items() {
    let mut ctx = glass(false);
    let (_dir, mut ex) = shown(&mut ctx);
    assert_eq!(note(&ex).as_deref(), Some("Select a file to preview."));
    pick(&mut ex, &["sub"], &mut ctx);
    assert_eq!(note(&ex).as_deref(), Some("No preview available."));
    pick(&mut ex, &["notes.txt", "other.txt"], &mut ctx);
    assert_eq!(note(&ex).as_deref(), Some("No preview available."));
    assert!(!ex.pane.as_ref().unwrap().reading(), "nothing to read");
}

#[test]
fn a_text_file_lands_off_the_loop_and_a_binary_one_has_no_preview() {
    let mut ctx = glass(false);
    let (_dir, mut ex) = shown(&mut ctx);
    pick(&mut ex, &["notes.txt"], &mut ctx);
    assert_eq!(
        note(&ex).as_deref(),
        Some(""),
        "blank while the read is out"
    );
    wake_until(&mut ex, &mut ctx, |ex| text(ex).is_some());
    assert_eq!(text(&ex).as_deref(), Some("first line\nsecond line\n"));
    pick(&mut ex, &["blob.bin"], &mut ctx);
    wake_until(&mut ex, &mut ctx, |ex| {
        note(ex).is_some_and(|t| !t.is_empty())
    });
    assert_eq!(note(&ex).as_deref(), Some("No preview available."));
}

#[test]
fn a_selection_change_while_a_read_is_out_waits_for_it_and_reads_only_the_latest() {
    let mut ctx = glass(false);
    let (_dir, mut ex) = shown(&mut ctx);
    let gate = Arc::new(std::sync::RwLock::new(()));
    let closed = gate.write().expect("lock");
    let installed = crate::panereader::Parts::installed();
    let real = installed.fetch.clone();
    let held = gate.clone();
    let parts = crate::panereader::Parts {
        fetch: Arc::new(move |p, cap, opened| {
            drop(held.read());
            real(p, cap, opened)
        }),
        ..installed
    };
    ex.pane = Some(Pane::with_parts(&ctx, parts));
    pick(&mut ex, &["notes.txt"], &mut ctx);
    for name in ["blob.bin", "notes.txt", "blob.bin", "other.txt"] {
        pick(&mut ex, &[name], &mut ctx);
    }
    let started = |ex: &Explorer| ex.pane.as_ref().map_or(0, |p| p.started);
    assert_eq!(started(&ex), 1, "one read out while it blocks");
    assert_eq!(note(&ex).as_deref(), Some(""), "blank for the file wanted");
    drop(closed);
    wake_until(&mut ex, &mut ctx, |ex| text(ex).is_some());
    assert_eq!(
        text(&ex).as_deref(),
        Some("other file\n"),
        "the latest, not the first"
    );
    assert_eq!(started(&ex), 2, "the first, then only the latest");
}

#[test]
fn a_file_gone_from_the_listing_clears_the_preview() {
    let mut ctx = glass(false);
    let (_dir, mut ex) = shown(&mut ctx);
    pick(&mut ex, &["notes.txt"], &mut ctx);
    wake_until(&mut ex, &mut ctx, |ex| text(ex).is_some());
    let i = ex.sv.selection.single().expect("one picked");
    ex.items.truncate(i);
    ex.refresh_chrome(&mut ctx);
    assert_eq!(note(&ex).as_deref(), Some("No preview available."));
}

#[test]
fn the_window_keeps_the_view_200_wide_taking_the_preview_panes_width_first() {
    let mut ctx = glass(false);
    let (_dir, mut ex) = shown(&mut ctx);
    ctx.size = (600, 573);
    let sl = ex.shell_layout(&ctx).expect("shell");
    let view = sl.view.width - 2 * reprisekit::classic::widgets::FIELD_PAD;
    assert_eq!(view, 200, "the view keeps its least width");
    assert_eq!(sl.preview.map(|r| r.width), Some(600 - 159 - 200));
    ctx.size = (300, 573);
    assert_eq!(
        ex.shell_layout(&ctx).and_then(|sl| sl.preview),
        None,
        "no room for it"
    );
    let min = ex.min_size(repriseapp::MAIN).expect("a least size").0;
    assert!(
        min >= 159 + 200 + 101,
        "{min}: room for both panes and the view"
    );
    ex.settings.preview_pane = false;
    assert!(ex.min_size(repriseapp::MAIN).expect("a least size").0 < min);
}

#[test]
fn the_command_bar_toggle_shows_the_pane_right_of_the_view_and_the_setting_keeps_it() {
    let mut ctx = glass(false);
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.refresh_chrome(&mut ctx);
    let sl = ex.shell_layout(&ctx).expect("shell");
    assert_eq!(sl.preview, None, "hidden at first");
    let wide = sl.view.width;
    ex.command_bar_press(Hit::PreviewPane, sl.command, false, &mut ctx);
    assert!(ex.settings.preview_pane && ex.settings_dirty);
    assert!(ex.cmdbar.pane_shown, "the toggle is drawn checked");
    let sl = ex.shell_layout(&ctx).expect("shell");
    let pane = sl.preview.expect("the pane shows");
    assert_eq!(
        (pane.right(), pane.width),
        (792, 261),
        "260 and the divider"
    );
    assert_eq!(sl.view.width, wide - 261);
    ex.command_bar_press(Hit::PreviewPane, sl.command, false, &mut ctx);
    assert!(ex.pane.is_none(), "hidden, its read goes too");
}

#[test]
fn the_divider_drags_clamps_and_keeps_its_width() {
    let mut ctx = glass(false);
    let (_dir, mut ex) = shown(&mut ctx);
    let pane = ex
        .shell_layout(&ctx)
        .and_then(|sl| sl.preview)
        .expect("pane");
    let y = pane.y + 40;
    let press = |x| repriseapp::PointerInput::Press {
        x,
        y,
        button: repriseapp::BTN_LEFT,
        time: 0,
        mods: Default::default(),
    };
    let release = |x| repriseapp::PointerInput::Release {
        x,
        y,
        button: repriseapp::BTN_LEFT,
        mods: Default::default(),
    };
    ex.settings_dirty = false;
    ex.pointer(press(pane.x + 2), &mut ctx);
    ex.pointer(
        repriseapp::PointerInput::Motion { x: pane.x - 98, y },
        &mut ctx,
    );
    ex.pointer(release(pane.x - 98), &mut ctx);
    assert_eq!(ex.settings.preview_width, 360);
    assert!(ex.settings_dirty, "kept in the state file");
    ex.pointer(press(792 - 361), &mut ctx);
    ex.pointer(repriseapp::PointerInput::Motion { x: 790, y }, &mut ctx);
    ex.pointer(release(790), &mut ctx);
    assert_eq!(ex.settings.preview_width, PANE_MIN_W);
    ex.pointer(press(792 - PANE_MIN_W - 1), &mut ctx);
    ex.pointer(repriseapp::PointerInput::Motion { x: 0, y }, &mut ctx);
    ex.pointer(release(0), &mut ctx);
    assert_eq!(
        ex.settings.preview_width,
        792 - 159 - 1 - 200,
        "the view keeps 200 px"
    );
}

#[test]
fn the_pane_pins_light_and_dark_at_scale_1_and_2() {
    reprisekit::pins::fonts();
    let mut got = Vec::new();
    for dark in [false, true] {
        let mut ctx = glass(dark);
        let (_dir, mut ex) = shown(&mut ctx);
        let look = if dark { "dark" } else { "light" };
        for state in ["nothing", "text"] {
            if state == "text" {
                pick(&mut ex, &["notes.txt"], &mut ctx);
                wake_until(&mut ex, &mut ctx, |ex| text(ex).is_some());
            }
            let r = ex
                .shell_layout(&ctx)
                .and_then(|sl| sl.preview)
                .expect("pane");
            for s in [1, 2] {
                let mut c = Canvas::with_scale(792, 573, s);
                ctx.at_scale(s, |ctx| crate::window::draw(&ex, &mut c, ctx));
                got.push((format!("{look} {state} @{s}"), fnv_in(&c, r)));
            }
        }
    }
    let path = reprisekit::crate_dir!().join("tests/pins/preview_pane.txt");
    reprisekit::pins::check_file(&path, &got);
}

use reprisekit::pins::fnv_in;

#[test]
fn a_blocked_listing_pool_does_not_block_the_pane() {
    let mut ctx = glass(false);
    let (_dir, mut ex) = shown(&mut ctx);
    let gate = Arc::new(std::sync::RwLock::new(()));
    let _closed = gate.write().expect("lock");
    ex.held = Some(gate.clone());
    pick(&mut ex, &["notes.txt"], &mut ctx);
    wake_until(&mut ex, &mut ctx, |ex| text(ex).is_some());
    assert_eq!(text(&ex).as_deref(), Some("first line\nsecond line\n"));
    ex.held = None;
}

/// A pane whose reads block until the returned lock's write guard drops.
fn gated(ctx: &Ctx) -> (Pane, Arc<std::sync::RwLock<()>>) {
    let gate = Arc::new(std::sync::RwLock::new(()));
    let installed = crate::panereader::Parts::installed();
    let real = installed.fetch.clone();
    let held = gate.clone();
    let parts = crate::panereader::Parts {
        fetch: Arc::new(move |p, cap, opened| {
            drop(held.read());
            real(p, cap, opened)
        }),
        ..installed
    };
    (Pane::with_parts(ctx, parts), gate)
}

#[test]
fn a_file_picked_behind_a_stalled_read_says_it_has_no_preview_until_its_own_lands() {
    let mut ctx = glass(false);
    let (_dir, mut ex) = shown(&mut ctx);
    let (pane, gate) = gated(&ctx);
    let closed = gate.write().expect("lock");
    ex.pane = Some(pane);
    pick(&mut ex, &["notes.txt"], &mut ctx);
    pick(&mut ex, &["other.txt"], &mut ctx);
    let now = std::time::Instant::now();
    assert_eq!(ex.pane_stall(now), (false, true), "waits blank, ticking");
    assert_eq!(note(&ex).as_deref(), Some(""));
    assert_eq!(ex.pane_stall(now + STALLED), (true, false));
    assert_eq!(note(&ex).as_deref(), Some("No preview available."));
    pick(&mut ex, &["blob.bin"], &mut ctx);
    pick(&mut ex, &["other.txt"], &mut ctx);
    assert_eq!(
        ex.pane_stall(now + STALLED),
        (true, false),
        "at once while it stalls"
    );
    assert_eq!(ex.pane.as_ref().map(|p| p.started), Some(1), "one read out");
    drop(closed);
    wake_until(&mut ex, &mut ctx, |ex| text(ex).is_some());
    assert_eq!(
        text(&ex).as_deref(),
        Some("other file\n"),
        "its own read lands"
    );
}

#[test]
fn the_file_read_out_waits_blank_without_a_stall_note() {
    let mut ctx = glass(false);
    let (_dir, mut ex) = shown(&mut ctx);
    let (pane, gate) = gated(&ctx);
    let closed = gate.write().expect("lock");
    ex.pane = Some(pane);
    pick(&mut ex, &["notes.txt"], &mut ctx);
    let later = std::time::Instant::now() + STALLED;
    assert_eq!(ex.pane_stall(later), (false, false));
    assert_eq!(note(&ex).as_deref(), Some(""));
    drop(closed);
    wake_until(&mut ex, &mut ctx, |ex| text(ex).is_some());
}

#[test]
fn a_text_size_switch_draws_the_preview_and_details_panes_afresh() {
    let mut ctx = glass(false);
    let (_dir, mut ex) = shown(&mut ctx);
    ex.follow_scale(1, &mut ctx);
    pick(&mut ex, &["notes.txt"], &mut ctx);
    wake_until(&mut ex, &mut ctx, |ex| text(ex).is_some());
    let mono = |ex: &Explorer| ex.pane.as_ref().map(|p| p.mono_px());
    assert_eq!(mono(&ex), Some(13));
    ctx.theme = ctx.theme.with_text_size(200);
    ex.follow_scale(1, &mut ctx);
    wake_until(&mut ex, &mut ctx, |ex| text(ex).is_some());
    assert_eq!(mono(&ex), Some(26), "the text box set at the new size");
    assert_eq!(ex.details.facts.title, "notes.txt");
}

#[test]
fn a_switch_into_glass_shows_the_selection_in_the_preview_pane() {
    let mut ctx = glass(false);
    let (_dir, mut ex) = shown(&mut ctx);
    let glass_theme = ctx.theme.clone();
    ctx.theme = reprisekit::interfaces::theme_for(Interface::Classic);
    ex.follow_scale(1, &mut ctx);
    pick(&mut ex, &["notes.txt"], &mut ctx);
    assert!(ex.pane.is_none(), "Classic has no preview pane");
    ctx.theme = glass_theme;
    ex.follow_scale(1, &mut ctx);
    wake_until(&mut ex, &mut ctx, |ex| text(ex).is_some());
    assert_eq!(ex.details.facts.title, "notes.txt");
}

/// A pane whose reads each wait for one send on the returned channel.
fn stepped(ctx: &Ctx) -> (Pane, std::sync::mpsc::Sender<()>) {
    let (step, steps) = std::sync::mpsc::channel::<()>();
    let steps = std::sync::Mutex::new(steps);
    let installed = crate::panereader::Parts::installed();
    let real = installed.fetch.clone();
    let parts = crate::panereader::Parts {
        fetch: Arc::new(move |p, cap, opened| {
            let _ = steps.lock().expect("steps").recv();
            real(p, cap, opened)
        }),
        ..installed
    };
    (Pane::with_parts(ctx, parts), step)
}

#[test]
fn the_stall_note_stays_while_the_waiting_files_own_read_runs() {
    let mut ctx = glass(false);
    let (_dir, mut ex) = shown(&mut ctx);
    let (pane, step) = stepped(&ctx);
    ex.pane = Some(pane);
    pick(&mut ex, &["notes.txt"], &mut ctx);
    pick(&mut ex, &["other.txt"], &mut ctx);
    let later = std::time::Instant::now() + STALLED;
    assert_eq!(ex.pane_stall(later), (true, false));
    step.send(()).expect("step");
    wake_until(&mut ex, &mut ctx, |ex| {
        ex.pane.as_ref().is_some_and(|p| p.started == 2)
    });
    assert_eq!(note(&ex).as_deref(), Some("No preview available."));
    step.send(()).expect("step");
    wake_until(&mut ex, &mut ctx, |ex| text(ex).is_some());
    assert_eq!(text(&ex).as_deref(), Some("other file\n"));
}

#[test]
fn a_look_or_text_size_switch_keeps_the_one_read_out() {
    let mut ctx = glass(false);
    let (_dir, mut ex) = shown(&mut ctx);
    ex.follow_scale(1, &mut ctx);
    let (pane, gate) = gated(&ctx);
    let fetch = pane.parts.fetch.clone();
    let closed = gate.write().expect("lock");
    ex.pane = Some(pane);
    pick(&mut ex, &["notes.txt"], &mut ctx);
    let glass_theme = ctx.theme.clone();
    ctx.theme = ctx.theme.with_text_size(200);
    ex.follow_scale(1, &mut ctx);
    ctx.theme = reprisekit::interfaces::theme_for(Interface::Classic);
    ex.follow_scale(1, &mut ctx);
    ctx.theme = glass_theme.with_text_size(200);
    ex.follow_scale(1, &mut ctx);
    let p = ex.pane.as_ref().expect("the pane");
    assert!(Arc::ptr_eq(&p.parts.fetch, &fetch), "the same reader");
    assert_eq!((p.started, p.reading()), (1, true), "one read out");
    assert_eq!(p.mono_px(), 26);
    drop(closed);
    wake_until(&mut ex, &mut ctx, |ex| text(ex).is_some());
    assert_eq!(text(&ex).as_deref(), Some("first line\nsecond line\n"));
    assert_eq!(ex.pane.as_ref().map(|p| p.started), Some(1));
}
