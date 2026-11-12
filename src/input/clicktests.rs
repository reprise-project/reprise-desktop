use super::*;
use crate::backend::Ask;
use crate::harness::{Harness, LEFT, TestClient};

/// TEST-1 with TEST-2 on its left, a window on TEST-1 focused, a file on the desktop and two
/// keyboard layouts (a tray cell). Menu and flyout shadows fall right and down, so none from
/// TEST-1 reaches TEST-2. The clock timer is off: a minute's redraw changes both bars.
fn on_first_of_two(interface: &str) -> (Harness, TestClient, WindowId) {
    let config = crate::config::Config::parse(&format!(
        "[appearance]\ninterface = \"{interface}\"\n[glass]\nfast_drag = false\n\
         [keyboard]\nlayout = \"us,de\"\n"
    ))
    .expect("config");
    let mut h = Harness::with_config(config);
    h.state.apply_keyboard();
    h.output("TEST-1", 1024, 768);
    h.add_output("TEST-2", 1024, 768, Some((-1024, 0)));
    h.dispatch();
    if let Some(t) = h.state.clock_timer.take() {
        h.state.handle.remove(t);
    }
    let dir = crate::harness::scratch().join(format!("click-asks-{interface}"));
    std::fs::create_dir_all(&dir).expect("the desktop folder");
    std::fs::write(dir.join("a.txt"), "a").expect("a desktop file");
    crate::desktopview::create(&mut h.state, dir, Default::default(), None);
    h.state.rescan_desktop();
    let mut c = h.connect();
    let a = h.map_toplevel(&mut c, "app", 300, 200);
    let (x, y) = first(&h).loc.into();
    h.state.move_to(a.id, x + 300, y + 100);
    h.dispatch();
    h.run_for(700);
    h.state.finish_window_anims();
    (h, c, a.id)
}

fn first(h: &Harness) -> Rectangle<i32, Logical> {
    (h.state.scene.outputs().iter())
        .find(|o| o.name() == "TEST-1")
        .and_then(reprisescene::output_rect)
        .expect("TEST-1's place")
}

/// A point on TEST-1 whose hit `want` takes.
fn find(h: &Harness, want: impl Fn(&State, &Hit) -> bool) -> (f64, f64) {
    let r = first(h);
    for y in (r.loc.y..r.loc.y + r.size.h).step_by(4) {
        for x in (r.loc.x..r.loc.x + r.size.w).step_by(4) {
            let p = (f64::from(x), f64::from(y));
            if want(&h.state, &h.state.scene_hit(p.into())) {
                return p;
            }
        }
    }
    panic!("no such target on TEST-1");
}

fn outputs(asked: &[(String, Ask)]) -> Vec<&str> {
    let mut on: Vec<&str> = (asked.iter())
        .filter(|(_, a)| !matches!(a, Ask::Pointer | Ask::Cursor))
        .map(|(o, _)| o.as_str())
        .collect();
    on.sort();
    on.dedup();
    on
}

/// Click `button` on the target `want` finds and let what it starts settle: no output but
/// TEST-1 is asked for a frame, and TEST-1 is when `redraws`. A click that changes nothing may
/// still meet a caret blink or a clock tick there.
fn click(
    h: &mut Harness,
    what: &str,
    button: u32,
    redraws: bool,
    want: impl Fn(&State, &Hit) -> bool,
) {
    let (x, y) = find(h, want);
    h.pointer_to(x, y);
    crate::backend::take_asks();
    h.button(button, true);
    h.button(button, false);
    h.run_for(400);
    asked_first(&format!("{what} at {x},{y}"), redraws);
}

/// The asks since the last call went to TEST-1 alone, and to it when `redraws`.
fn asked_first(what: &str, redraws: bool) {
    let asked = crate::backend::take_asks();
    let on = outputs(&asked);
    assert!(
        on.iter().all(|&o| o == "TEST-1"),
        "{what} asks only TEST-1: {asked:?}"
    );
    assert!(
        !redraws || on.contains(&"TEST-1"),
        "{what} asks TEST-1: {asked:?}"
    );
}

/// An open menu's row whose item `want` takes.
fn row(want: impl Fn(&str, bool) -> bool) -> impl Fn(&State, &Hit) -> bool {
    move |s, hit| match hit {
        Hit::Menu {
            depth,
            row: Some(r),
        } => matches!(
            s.menus.get(*depth).and_then(|m| m.items.get(*r)),
            Some(reprisekit::controls::menu::MenuItem::Entry { label, checked, .. }) if want(label, *checked)
        ),
        _ => false,
    }
}

fn hit(want: impl Fn(&Hit) -> bool) -> impl Fn(&State, &Hit) -> bool {
    move |_, h| want(h)
}

fn bar(want: impl Fn(&TaskbarHit) -> bool) -> impl Fn(&State, &Hit) -> bool {
    move |_, h| matches!(h, Hit::Taskbar { hit, .. } if want(hit))
}

#[test]
fn a_click_on_each_kind_of_target_asks_only_the_output_it_lies_on_in_classic() {
    clicks_ask_only_their_output("classic");
}

#[test]
fn a_click_on_each_kind_of_target_asks_only_the_output_it_lies_on_in_blue() {
    clicks_ask_only_their_output("blue");
}

#[test]
fn a_click_on_each_kind_of_target_asks_only_the_output_it_lies_on_in_glass() {
    clicks_ask_only_their_output("glass");
}

/// Click each kind of target on TEST-1 in turn. Every bar lists every window, so a focus change
/// rightly redraws TEST-2's bar: focus moves only between the clicks.
fn clicks_ask_only_their_output(interface: &str) {
    use crate::dialog::DialogHit;
    use reprisekit::startpanel::Spot;
    let (mut h, _c, a) = on_first_of_two(interface);
    let frame = |part: FrameHit| {
        hit(move |h| {
            *h == Hit::Frame {
                window: a,
                hit: part,
            }
        })
    };
    click(&mut h, "the caption", LEFT, false, frame(FrameHit::Title));
    let client = hit(|h| {
        matches!(
            h,
            Hit::Surface {
                window: Some(_),
                ..
            }
        )
    });
    click(&mut h, "the client", LEFT, false, client);
    let max = frame(FrameHit::Button(CaptionButton::Maximize));
    click(&mut h, "Maximize", LEFT, true, max);
    click(
        &mut h,
        "the window menu",
        BTN_RIGHT,
        true,
        frame(FrameHit::Title),
    );
    click(
        &mut h,
        "its Restore row",
        LEFT,
        true,
        row(|l, _| l == "Restore"),
    );
    let task = bar(|t| matches!(t, TaskbarHit::Task(_)));
    click(&mut h, "a task button's menu", BTN_RIGHT, true, task);
    h.state.close_menu();

    h.state.focus_desktop();
    h.run_for(400);
    let icon = hit(|h| matches!(h, Hit::Desktop { index: Some(_) }));
    let desktop = hit(|h| *h == Hit::Desktop { index: None });
    click(&mut h, "a desktop icon", LEFT, true, &icon);
    click(&mut h, "the empty desktop", LEFT, true, &desktop);
    click(&mut h, "the icon's menu", BTN_RIGHT, true, &icon);
    click(
        &mut h,
        "its Rename row",
        LEFT,
        true,
        row(|l, _| l == "Rename"),
    );
    click(&mut h, "the desktop menu", BTN_RIGHT, true, &desktop);
    click(
        &mut h,
        "its Refresh row",
        LEFT,
        true,
        row(|l, _| l == "Refresh"),
    );

    click(
        &mut h,
        "Start",
        LEFT,
        true,
        bar(|t| *t == TaskbarHit::Start),
    );
    let programs = row(|l, _| l.contains("Programs"));
    let all = |s: &State, h: &Hit| {
        *h == Hit::StartPanel {
            spot: Some(Spot::AllPrograms),
        } || programs(s, h)
    };
    click(&mut h, "All Programs", LEFT, true, all);
    let tray = bar(|t| matches!(t, TaskbarHit::Tray(_)));
    click(&mut h, "the layout cell", LEFT, true, tray);
    click(
        &mut h,
        "its checked row",
        LEFT,
        true,
        row(|_, checked| checked),
    );
    click(
        &mut h,
        "the clock",
        LEFT,
        true,
        bar(|t| *t == TaskbarHit::Clock),
    );
    let next = hit(|h| {
        *h == Hit::Calendar {
            hit: reprisekit::controls::calendar::PopupHit::Next,
        }
    });
    click(&mut h, "the calendar's next month", LEFT, true, next);

    h.state.open_dialog(crate::dialog::Dialog::run(&[]));
    h.run_for(400);
    let sheet = hit(|h| matches!(h, Hit::Dialog { button: None, .. }));
    click(&mut h, "a sheet, closing the calendar", LEFT, true, sheet);
    let field = |h: &Hit| {
        matches!(
            h,
            Hit::Dialog {
                button: Some(DialogHit::Field(_)),
                ..
            }
        )
    };
    click(&mut h, "a sheet's focused field", LEFT, false, hit(field));
    let button = |h: &Hit| {
        matches!(
            h,
            Hit::Dialog {
                button: Some(DialogHit::Button(_)),
                ..
            }
        )
    };
    let (x, y) = find(&h, hit(button));
    h.pointer_to(x, y);
    crate::backend::take_asks();
    h.button(LEFT, true);
    asked_first("a press on a sheet's button", true);
}
