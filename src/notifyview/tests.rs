use super::clock::{Due, due_step};
use super::layout::{droplet_ends, lay_out, lay_out_open, sheet_glass};
use super::motion::{
    Band, aim_stack, band, follow_slot, keeps_ticking, open_target, restack, still,
};
use super::paint::{frame_outputs, relative, slot_needs_icon};
use super::pointer::{hold_at, point_at, press_target, rest_point};
use super::reply::take_draft;
use super::stack::{
    Burst, chip_count, fill, fold_from, fold_in, join_burst, overflow, retint, shed, take_queued,
};
use super::warm::{land_warm, paint_warm_jobs};
use super::*;
use crate::config::EffectsConfig;
use crate::text::TextRenderer;
use crate::theme::Theme;
use reprisekit::anim::{Ease, Tween};
use reprisekit::controls::notifysheet::{
    SheetIcons, SheetLayout, WarmLayers, build_warm, place_stack,
};
use reprisekit::glass::notify::HALO_CORNER;
use reprisekit::glass::notify::WELL_CORNER;
use reprisekit::icon::IconSource;
use reprisekit::interface::Interface;
use reprisekit::interfaces::theme_for;
use reprisekit::style::{NotifyLook, SheetShape};
use reprisekit::textedit::TextEdit;
use smithay::utils::Rectangle;

fn text() -> TextRenderer {
    TextRenderer::load(&reprisekit::theme::FontConfig::default())
}

#[test]
fn glass_lays_out_a_sheet_and_classic_a_balloon() {
    let item = Item::Note(note(1, "Mail"));
    let lay = |t: &Theme| {
        lay_out(
            t,
            &text(),
            &item,
            false,
            Edge::Bottom,
            SheetShape::Lozenge,
            false,
            "now",
        )
    };
    assert!(matches!(lay(&theme_for(Interface::Glass)), Drawn::Sheet(_)));
    assert!(matches!(lay(&Theme::default()), Drawn::Balloon(_)));
}

#[test]
fn a_lozenge_pane_is_its_footprint_at_the_tree_origin() {
    let t = theme_for(Interface::Glass).scaled(2);
    let item = Item::Note(note(1, "Mail"));
    let Drawn::Sheet(lay) = lay_out(
        &t,
        &text(),
        &item,
        false,
        Edge::Bottom,
        SheetShape::Lozenge,
        false,
        "now",
    ) else {
        panic!()
    };
    let pane = sheet_glass(&lay, t.look.style().unwrap().notify.as_ref().unwrap(), 2);
    assert_eq!(pane.shapes.len(), 1);
    assert_eq!(
        pane.shapes[0].rect,
        Rect::new(0, 0, 360, lay.footprint.height / 2)
    );
    assert_eq!(pane.shapes[0].corner.len(), 25);
}

fn halo_sheet_layout(scale: i32) -> (SheetLayout, NotifyLook) {
    let t = theme_for(Interface::Glass).scaled(scale);
    let item = Item::Note(note(1, "Mail"));
    let Drawn::Sheet(lay) = lay_out(
        &t,
        &text(),
        &item,
        false,
        Edge::Bottom,
        SheetShape::Halo,
        false,
        "now",
    ) else {
        panic!()
    };
    (lay, t.look.style().unwrap().notify.unwrap())
}

#[test]
fn the_halo_pane_covers_the_breakout_without_overlap() {
    let (lay, look) = halo_sheet_layout(1);
    let pane = sheet_glass(&lay, &look, 1);
    assert_eq!(pane.shapes.len(), 2);
    assert_eq!(pane.shapes[0].rect, Rect::new(9, 0, 50, 25));
    assert!(pane.shapes.iter().all(|s| s.rect.x >= 0 && s.rect.y >= 0));
    assert!(!pane.shapes[0].rect.intersects(pane.shapes[1].rect));
    assert_eq!(pane.shapes[1].rect.bottom(), lay.footprint.height);
}

#[test]
fn the_halo_pane_is_the_well_top_and_the_sheet_in_logical_px() {
    let (lay, look) = halo_sheet_layout(2);
    let pane = sheet_glass(&lay, &look, 2);
    let [well, sheet] = &pane.shapes[..] else {
        panic!("{pane:?}")
    };
    assert_eq!(
        (well.rect, well.corner.len(), well.square_bottom),
        (Rect::new(9, 0, 50, 25), WELL_CORNER.len(), true)
    );
    assert_eq!(
        (sheet.rect, sheet.corner.len(), sheet.square_bottom),
        (
            Rect::new(0, 25, 360, lay.sheet.height / 2),
            HALO_CORNER.len(),
            false
        )
    );
}

#[test]
fn a_coloured_app_icon_tints_its_sheet_and_a_grey_one_does_not() {
    let icon = |px: u32| {
        [IconSource {
            width: 48,
            height: 48,
            pixels: vec![px; 48 * 48],
        }]
    };
    let glass = theme_for(Interface::Glass);
    assert_eq!(
        sheet_tint(&glass, &icon(0xFFD0_3030), 2),
        Some(Some(Color::rgb(0xD0, 0x30, 0x30)))
    );
    assert_eq!(sheet_tint(&glass, &icon(0xFF80_8080), 1), Some(None));
    assert_eq!(sheet_tint(&glass, &[], 1), Some(None));
    assert_eq!(sheet_tint(&Theme::default(), &icon(0xFFD0_3030), 1), None);
}

fn red_icon() -> Vec<IconSource> {
    vec![IconSource {
        width: 48,
        height: 48,
        pixels: vec![0xFFD0_3030; 48 * 48],
    }]
}

#[test]
fn a_burst_takes_its_tint_from_its_first_app_icon() {
    let glass = theme_for(Interface::Glass);
    let mut scene = crate::compositor::DeskScene::new();
    let mut b = balloon(&mut scene, 1);
    b.item = Item::Burst(Burst::new("Mail"));
    join_burst(&mut b, &note(2, "Mail"), &glass);
    assert_eq!(b.tint, Some(None));
    let mut red = note(3, "Mail");
    red.app_icon = red_icon();
    join_burst(&mut b, &red, &glass);
    assert_eq!(b.tint, Some(Some(Color::rgb(0xD0, 0x30, 0x30))));
    let mut blue = note(4, "Mail");
    blue.app_icon = vec![IconSource {
        width: 48,
        height: 48,
        pixels: vec![0xFF30_60D0; 48 * 48],
    }];
    join_burst(&mut b, &blue, &glass);
    assert_eq!(
        b.tint,
        Some(Some(Color::rgb(0xD0, 0x30, 0x30))),
        "the first icon's"
    );
}

#[test]
fn a_balloon_shown_before_a_switch_to_glass_takes_its_tint() {
    let mut scene = crate::compositor::DeskScene::new();
    let mut b = balloon(&mut scene, 1);
    let mut n = note(1, "Mail");
    n.app_icon = red_icon();
    b.item = Item::Note(n);
    let mut stack = [b];
    retint(&mut stack, &Theme::default());
    assert_eq!(stack[0].tint, None);
    retint(&mut stack, &theme_for(Interface::Glass));
    assert_eq!(stack[0].tint, Some(Some(Color::rgb(0xD0, 0x30, 0x30))));
    retint(&mut stack, &Theme::default());
    assert_eq!(stack[0].tint, None, "back to Classic");
}

#[test]
fn a_tinted_balloon_keeps_its_tint_through_a_reposition_under_glass() {
    let glass = theme_for(Interface::Glass);
    let mut scene = crate::compositor::DeskScene::new();
    let mut b = balloon(&mut scene, 1);
    let mut n = note(1, "Mail");
    n.app_icon = red_icon();
    b.item = Item::Note(n);
    b.tint = Some(Some(Color::rgb(0x30, 0x60, 0xD0)));
    let mut stack = [b];
    retint(&mut stack, &glass);
    assert_eq!(
        stack[0].tint,
        Some(Some(Color::rgb(0x30, 0x60, 0xD0))),
        "the icon is not read again"
    );
    stack[0].tint = Some(None);
    retint(&mut stack, &glass);
    assert_eq!(stack[0].tint, Some(None), "a grey icon is not read again");
}

#[test]
fn relative_time_reads_now_then_minutes_then_hours() {
    let s = |n| relative(Duration::from_secs(n));
    assert_eq!(
        [s(0), s(59), s(61), s(3601)],
        ["now", "now", "1 min ago", "1 h ago"]
    );
}

fn note(id: u32, app: &str) -> Notification {
    let mut n = Notification::new(id, format!("n{id}"), "x".into());
    n.app = app.into();
    n
}

#[test]
fn the_queue_drops_its_oldest_past_the_cap() {
    let mut q = VecDeque::new();
    let mut dropped = Vec::new();
    for id in 1..=(MAX_QUEUED as u32 + 2) {
        dropped.extend(enqueue(&mut q, Item::Note(note(id, ""))));
    }
    assert_eq!(dropped, vec![1, 2]);
    assert_eq!(q.len(), MAX_QUEUED);
    assert!(q.front().is_some_and(|i| i.is(3)));
}

#[test]
fn a_critical_balloon_never_times_out() {
    let mut n = Notification::new(1, "Hi".into(), String::new());
    assert_eq!(timeout_ms(&n), Some(DEFAULT_MS));
    n.expire_timeout = 0;
    assert_eq!(timeout_ms(&n), Some(MAX_MS));
    n.expire_timeout = 100;
    assert_eq!(timeout_ms(&n), Some(MIN_MS));
    n.urgency = Urgency::Critical;
    assert_eq!(timeout_ms(&n), None);
}

/// An open reply row collapses once idle, but not while the pointer rests in the balloon; a
/// collapsed balloon expires on its own timeout.
#[test]
fn an_idle_reply_row_collapses_unless_the_pointer_is_in_the_balloon() {
    assert_eq!(due_step(true, false), Due::Collapse);
    assert_eq!(due_step(true, true), Due::Wait);
    assert_eq!(due_step(false, true), Due::Expire);
    assert_eq!(due_step(false, false), Due::Expire);
    assert_eq!(REPLY_IDLE, Duration::from_secs(120));
}

#[test]
fn reply_brings_back_its_notifications_draft_focused() {
    let typed = |t: &str| ReplyField {
        edit: TextEdit::new(t, false),
        focused: false,
    };
    let mut drafts = vec![(1, typed("see you")), (2, typed("ok"))];
    let back = take_draft(&mut drafts, 1);
    assert_eq!((back.edit.text.as_str(), back.focused), ("see you", true));
    assert_eq!(drafts.len(), 1);
    let fresh = take_draft(&mut drafts, 3);
    assert!(fresh.edit.text.is_empty() && fresh.focused);
}

#[test]
fn buttons_skip_the_default_action_and_menu_rows_name_every_action() {
    let mut n = Notification::new(1, "Hi".into(), String::new());
    n.actions = vec![
        ("default".into(), String::new()),
        ("open".into(), "Open".into()),
        ("x".into(), String::new()),
        (INLINE_REPLY.into(), String::new()),
    ];
    assert_eq!(button_labels(&n), ["Open", "x", "Reply"]);
    let rows = action_items(&n.actions);
    assert_eq!(rows[0], MenuItem::entry("Open"));
    assert_eq!(rows[2], MenuItem::entry("x"));
    assert_eq!(rows[3], MenuItem::entry("Reply"));
    let amp = action_items(&[("k".into(), "Save & Quit".into())]);
    assert_eq!(amp[0], MenuItem::entry("Save & Quit"));
    assert_eq!(amp[0].access_key(), Some('s'));
}

#[test]
fn the_fourth_from_one_program_in_the_window_bursts() {
    let t0 = Instant::now();
    let at = |s| t0 + Duration::from_secs(s);
    let mut r = VecDeque::new();
    assert!(!arrived(&mut r, "Mail", at(0)));
    assert!(!arrived(&mut r, "Chat", at(1)));
    assert!(!arrived(&mut r, "Mail", at(2)));
    assert!(!arrived(&mut r, "Mail", at(3)));
    assert!(arrived(&mut r, "Mail", at(4)), "the fourth");
    assert!(!arrived(&mut r, "Chat", at(5)));
    let mut r = VecDeque::new();
    for s in [0, 4, 8] {
        arrived(&mut r, "Mail", at(s));
    }
    assert!(!arrived(&mut r, "Mail", at(11)), "the first fell out");
    assert_eq!(r.len(), 3);
}

#[test]
fn a_burst_takes_its_programs_queued_balloons() {
    let mut q: VecDeque<Item> = [
        note(1, "Chat"),
        note(2, "Mail"),
        note(3, "Chat"),
        note(4, "Mail"),
    ]
    .into_iter()
    .map(Item::Note)
    .collect();
    let (taken, at) = take_queued(&mut q, "Mail");
    assert_eq!(taken.iter().map(|n| n.id).collect::<Vec<_>>(), [2, 4]);
    assert_eq!(at, Some(1));
    assert!(q.iter().all(|i| i.note().is_some_and(|n| n.app == "Chat")));
    assert_eq!(take_queued(&mut q, "Mail"), (Vec::new(), None));
}

#[test]
fn a_burst_counts_and_lists_its_newest() {
    let mut b = Burst::new("Mail");
    for id in 1..=5 {
        b.add(&note(id, "Mail"));
    }
    assert_eq!(b.title(), "5 new from Mail");
    assert_eq!(b.lines, ["n5", "n4", "n3"]);
    let mut empty = note(6, "Mail");
    empty.summary.clear();
    empty.body = "first line\nsecond".into();
    b.add(&empty);
    assert_eq!(b.lines[0], "first line");
}

#[test]
fn a_burst_keeps_the_first_app_icon_and_has_no_picture() {
    let src = |w: u32| IconSource {
        width: w,
        height: w,
        pixels: vec![0; (w * w) as usize],
    };
    let mut first = note(1, "Mail");
    first.image = vec![src(32)];
    let mut second = note(2, "Mail");
    second.app_icon = vec![src(16)];
    let mut b = Burst::new("Mail");
    b.add(&first);
    assert!(!Item::Burst(b.clone()).has_app_icon());
    b.add(&second);
    let item = Item::Burst(b);
    assert_eq!(item.app_icon()[0].width, 16);
    assert_eq!(item.icon()[0].width, 32, "Classic draws the first picture");
    assert!(item.image().is_empty());
}

fn items(ids: &[u32]) -> Vec<Item> {
    ids.iter().map(|&i| Item::Note(note(i, "Mail"))).collect()
}

#[test]
fn glass_shows_three_and_queues_the_fourth() {
    assert_eq!((visible_cap(true), visible_cap(false)), (3, 1));
    let mut q: VecDeque<Item> = items(&[4, 5]).into();
    assert!(fill(3, 3, &mut q).is_empty());
    assert_eq!(q.len(), 2);
}

#[test]
fn a_leaving_sheet_lets_the_queue_head_in() {
    let mut q: VecDeque<Item> = items(&[4, 5]).into();
    let came = fill(2, 3, &mut q);
    assert!(came.len() == 1 && came[0].is(4));
    assert!(q.len() == 1 && q[0].is(5));
}

#[test]
fn hovering_pauses_every_clock_and_leaving_resumes() {
    let t0 = Instant::now();
    let mut c = Clock::start(Some(7000), t0);
    c.pause(t0 + Duration::from_secs(2));
    assert_eq!(c.due(), None);
    c.resume(t0 + Duration::from_secs(10));
    assert_eq!(c.due(), Some(t0 + Duration::from_secs(15)));
}

#[test]
fn a_critical_clock_never_runs() {
    assert_eq!(Clock::start(None, Instant::now()).due(), None);
}

#[test]
fn the_chip_counts_the_queue() {
    use reprisekit::controls::notifysheet::chip_label;
    assert_eq!(
        (chip_count(5, 3), chip_count(0, 3), chip_count(5, 1)),
        (Some(5), None, None)
    );
    assert_eq!(chip_label(5), "+5 more");
}

#[test]
fn a_replaced_visible_note_keeps_its_slot() {
    let shown = items(&[1, 2, 3]);
    let refs: Vec<&Item> = shown.iter().collect();
    assert_eq!((slot_of(&refs, 2), slot_of(&refs, 9)), (Some(1), None));
}

#[test]
fn a_burst_takes_every_same_app_sheet_but_a_reply() {
    let (a, b, c, d) = (
        Item::Note(note(1, "Mail")),
        Item::Note(note(2, "Chat")),
        Item::Note(note(3, "Mail")),
        Item::Note(note(4, "Mail")),
    );
    assert_eq!(
        fold_from(&[(&a, false), (&b, false), (&c, true), (&d, false)], "Mail"),
        vec![0, 3]
    );
}

fn balloon(scene: &mut crate::compositor::DeskScene, id: u32) -> Balloon {
    let tree = scene.tree_create(scene.root());
    Balloon {
        item: Item::Note(note(id, "Mail")),
        at: (0, 0),
        scale: 1,
        tree,
        glass_tree: scene.tree_create(tree),
        backdrop: None,
        buffer: scene.buffer_create(tree),
        shown_at: Instant::now() - Duration::from_secs(300),
        clock: Clock::start(Some(DEFAULT_MS), Instant::now()),
        motion: Motion::settled_at(Instant::now()),
        drawn: None,
        open_drawn: 0.0,
        pane: None,
        held: None,
        hot: None,
        reply: None,
        pointer_in: false,
        serial: u64::from(id),
        tint: None,
        opening: SheetOpening::default(),
        bead: None,
        warm_gen: 0,
        glass: SheetGlass::default(),
        painted: None,
    }
}

/// Layers a worker painted for `b`'s Droplet, the job asking for them, and whether `b` would
/// still send that job.
fn warm_layers(b: &Balloon) -> (Option<WarmLayers>, impl Fn(&mut Balloon) -> bool + use<>) {
    let (job, wants) = warm_job(b);
    (Some(build_warm(job, &text())), wants)
}

/// A warm job for `b`'s Droplet, and whether `b` would still send it.
fn warm_job(b: &Balloon) -> (WarmJob, impl Fn(&mut Balloon) -> bool + use<>) {
    let (theme, text) = (theme_for(Interface::Glass), text());
    let ends = droplet_ends(
        &theme,
        &text,
        &b.item,
        false,
        Edge::Bottom,
        SheetShape::Droplet,
        "now",
    );
    let [shut, open] = ends.unwrap();
    let icons = SheetIcons::default();
    let job = SheetOpening::default()
        .warm_job(
            &SheetGlass::default(),
            &theme,
            &shut,
            &open,
            icons,
            None,
            true,
        )
        .unwrap();
    let wants = move |b: &mut Balloon| {
        b.opening
            .warm_job(
                &b.glass,
                &theme,
                &shut,
                &open,
                SheetIcons::default(),
                None,
                true,
            )
            .is_some()
    };
    (job, wants)
}

#[test]
fn a_warm_result_for_an_earlier_job_is_dropped() {
    let mut scene = crate::compositor::DeskScene::new();
    let mut stack = vec![balloon(&mut scene, 1)];
    stack[0].warm_gen = 2;
    let (built, wants) = warm_layers(&stack[0]);
    assert!(!land_warm(&mut stack, 1, 1, built));
    assert!(wants(&mut stack[0]), "nothing installed");
    let (built, wants) = warm_layers(&stack[0]);
    assert!(land_warm(&mut stack, 1, 2, built));
    assert!(!wants(&mut stack[0]), "the current job's layers are in");
}

#[test]
fn a_switch_away_from_droplets_frees_the_layers_and_glass_the_new_look_leaves_undrawn() {
    let mut scene = crate::compositor::DeskScene::new();
    let mut stack = vec![balloon(&mut scene, 1)];
    stack[0].item = droplet_note();
    let (theme, text) = (theme_for(Interface::Glass), text());
    let ends = droplet_ends(
        &theme,
        &text,
        &stack[0].item,
        false,
        Edge::Bottom,
        SheetShape::Droplet,
        "now",
    );
    let [shut, open] = ends.unwrap();
    let warm = |b: &mut Balloon, lit: bool| {
        let built = b.opening.warm(
            &mut b.glass,
            &theme,
            &text,
            &shut,
            &open,
            SheetIcons::default(),
            None,
            None,
            lit,
        );
        (built, b.opening.builds(), b.glass.builds())
    };
    assert_eq!(
        warm(&mut stack[0], true),
        (true, 1, 2),
        "layers and the lit ends"
    );
    assert_eq!(warm(&mut stack[0], false), (true, 2, 4), "the plain ends");
    shed(&mut stack, true, true);
    assert_eq!(
        warm(&mut stack[0], true),
        (false, 2, 4),
        "Droplets keep both"
    );
    shed(&mut stack, true, false);
    assert_eq!(
        warm(&mut stack[0], true),
        (true, 1, 4),
        "the layers went; sheets keep their glass"
    );
    shed(&mut stack, false, false);
    assert_eq!(warm(&mut stack[0], true), (true, 1, 2), "both went");
}

#[test]
fn a_warm_job_that_panics_is_landed_as_none_and_not_sent_again() {
    let mut scene = crate::compositor::DeskScene::new();
    let mut stack = vec![balloon(&mut scene, 1)];
    let (tx, jobs) = std::sync::mpsc::channel();
    for serial in [1, 2] {
        tx.send((serial, 1, warm_job(&stack[0]).0)).unwrap();
    }
    drop(tx);
    let calls = std::cell::Cell::new(0);
    let landed = std::cell::RefCell::new(Vec::new());
    paint_warm_jobs(
        jobs,
        |job, text| {
            calls.set(calls.get() + 1);
            if calls.get() == 1 {
                panic!("a job that panics");
            }
            build_warm(job, text)
        },
        |serial, generation, built| {
            landed.borrow_mut().push((serial, generation, built));
            true
        },
    );
    let mut landed = landed.into_inner();
    let ok: Vec<_> = landed.iter().map(|(s, _, b)| (*s, b.is_some())).collect();
    assert_eq!(
        ok,
        [(1, false), (2, true)],
        "the painter outlives the panic"
    );
    let (_, wants) = warm_job(&stack[0]);
    stack[0].warm_gen = 1;
    assert!(wants(&mut stack[0]), "the job is out");
    let (serial, generation, built) = landed.remove(0);
    assert!(!land_warm(&mut stack, serial, generation, built));
    assert!(
        !wants(&mut stack[0]),
        "its content paints in frame from now on"
    );
}

#[test]
fn a_warm_result_for_a_balloon_gone_is_dropped() {
    let mut scene = crate::compositor::DeskScene::new();
    let mut stack = vec![balloon(&mut scene, 1)];
    let (built, _) = warm_layers(&stack[0]);
    assert!(!land_warm(&mut stack, 9, 0, built));
}

#[test]
fn a_sliding_sheet_takes_the_pointer_where_it_is_drawn() {
    let mut scene = crate::compositor::DeskScene::new();
    let mut b = balloon(&mut scene, 1);
    b.at = (100, 400);
    let t0 = Instant::now();
    b.motion.slide_by(90.0, t0, 250);
    assert_eq!(b.hit_at(t0), (100, 490));
    assert_eq!(b.hit_at(t0 + Duration::from_millis(300)), (100, 400));
}

/// Three Glass sheets stacked up from y 600, newest nearest the tray; returns the slot pitch.
fn sheets_at(scene: &mut crate::compositor::DeskScene) -> (NotifyView, i32) {
    let theme = theme_for(Interface::Glass);
    let mut stack: Vec<Balloon> = [3, 2, 1].map(|id| balloon(scene, id)).into();
    let mut pitch = 0;
    for (i, b) in stack.iter_mut().enumerate() {
        let d = lay_out(
            &theme,
            &text(),
            &b.item,
            false,
            Edge::Bottom,
            SheetShape::Lozenge,
            false,
            "now",
        );
        pitch = d.footprint().1 + 8;
        b.at = (100, 600 - i as i32 * pitch);
        b.drawn = Some(d);
    }
    let v = NotifyView {
        stack,
        ..NotifyView::default()
    };
    (v, pitch)
}

#[test]
fn closing_the_outermost_sheet_under_a_still_pointer_lets_the_stack_go() {
    let mut scene = crate::compositor::DeskScene::new();
    let (mut v, pitch) = sheets_at(&mut scene);
    let (x, y) = (200.0, f64::from(600 - 2 * pitch + 20));
    let now = Instant::now();
    point_at(&mut v, x, y, true, now);
    assert!(hovered(&v), "held while the pointer is on a sheet");
    v.stack.remove(2);
    assert!(hold_at(&mut v, x, y, now));
    assert!(!hovered(&v), "nothing rests under the pointer");
}

#[test]
fn a_sheet_sliding_into_a_closed_slot_keeps_the_stack_held() {
    let mut scene = crate::compositor::DeskScene::new();
    let (mut v, pitch) = sheets_at(&mut scene);
    let (x, y) = (200.0, f64::from(600 - pitch + 20));
    let now = Instant::now();
    point_at(&mut v, x, y, true, now);
    v.stack.remove(1);
    v.stack[1].at.1 += pitch;
    v.stack[1].motion.slide_by(-f64::from(pitch), now, 250);
    assert!(!hold_at(&mut v, x, y, now));
    assert!(
        hovered(&v),
        "held while the older sheet slides under the pointer"
    );
}

#[test]
fn a_halo_breakout_holds_the_stack_and_the_corner_beside_it_does_not() {
    let mut scene = crate::compositor::DeskScene::new();
    let mut b = balloon(&mut scene, 1);
    b.at = (100, 600);
    b.drawn = Some(Drawn::Sheet(halo_sheet_layout(1).0));
    let mut v = NotifyView {
        stack: vec![b],
        ..NotifyView::default()
    };
    let now = Instant::now();
    let (well, beside) = ((130.0, 610.0), (400.0, 610.0));
    point_at(&mut v, well.0, well.1, true, now);
    assert!(hovered(&v), "the well above the sheet is the sheet");
    assert!(!hold_at(&mut v, well.0, well.1, now));
    assert!(hold_at(&mut v, beside.0, beside.1, now));
    assert!(!hovered(&v), "the bare corner right of the well lets go");
}

#[test]
fn a_sheet_sliding_away_from_a_still_pointer_lets_the_stack_go_once_settled() {
    let mut scene = crate::compositor::DeskScene::new();
    let (mut v, pitch) = sheets_at(&mut scene);
    let (x, y) = (200.0, f64::from(600 - 2 * pitch + 20));
    let now = Instant::now();
    point_at(&mut v, x, y, true, now);
    v.stack.remove(0);
    for b in &mut v.stack {
        b.at.1 += pitch;
        b.motion.slide_by(-f64::from(pitch), now, 250);
    }
    assert!(!hold_at(&mut v, x, y, now));
    assert!(hovered(&v), "held while a sheet is drawn under the pointer");
    assert!(hold_at(&mut v, x, y, now + Duration::from_millis(300)));
    assert!(
        !hovered(&v),
        "let go once the sheets settle clear of the pointer"
    );
}

#[test]
fn a_sheet_arriving_under_a_still_pointer_holds_the_stack() {
    let mut scene = crate::compositor::DeskScene::new();
    let (mut v, pitch) = sheets_at(&mut scene);
    let (x, y) = (200.0, f64::from(600 - pitch + 20));
    assert!(hold_at(&mut v, x, y, Instant::now()));
    assert!(hovered(&v), "held with no motion over the stack");
}

#[test]
fn a_press_with_no_motion_before_it_holds_the_stack() {
    let mut scene = crate::compositor::DeskScene::new();
    let (mut v, pitch) = sheets_at(&mut scene);
    let (x, y) = (200.0, f64::from(600 - pitch + 20));
    let hit = press_target(&mut v, x, y, Instant::now());
    assert!(matches!(hit, Some((1, _, NotifyHit::Body, _))));
    assert!(hovered(&v), "held from the press on");
}

/// Layout point inside sheet `i`'s close spot, at its rest place.
fn close_spot(v: &NotifyView, i: usize) -> (f64, f64) {
    let b = &v.stack[i];
    let d = b.drawn.as_ref().unwrap();
    let (c, (ox, oy)) = (d.parts().close, d.origin());
    (
        f64::from(b.at.0 + c.x - ox + 2),
        f64::from(b.at.1 + c.y - oy + 2),
    )
}

/// The newest sheet closes: the others ease down a slot over 250 ms.
fn close_newest(v: &mut NotifyView, pitch: i32, now: Instant) {
    v.stack.remove(0);
    for b in &mut v.stack {
        b.at.1 += pitch;
        b.motion.slide_by(-f64::from(pitch), now, 250);
    }
}

#[test]
fn a_tick_before_a_slide_ends_runs_once_more_to_re_check_the_pointer() {
    let mut scene = crate::compositor::DeskScene::new();
    let (mut v, pitch) = sheets_at(&mut scene);
    let now = Instant::now();
    close_newest(&mut v, pitch, now);
    let (tick, later) = (
        now + Duration::from_millis(249),
        now + Duration::from_millis(251),
    );
    assert!(
        keeps_ticking(&v, still(&v, tick), later),
        "settled after the tick began"
    );
    assert!(
        !keeps_ticking(&v, still(&v, later), later),
        "the next tick ends it"
    );
}

#[test]
fn a_sheet_slid_out_from_under_a_still_pointer_loses_its_lit_close() {
    let mut scene = crate::compositor::DeskScene::new();
    let (mut v, pitch) = sheets_at(&mut scene);
    let (x, y) = close_spot(&v, 1);
    let now = Instant::now();
    point_at(&mut v, x, y, true, now);
    assert_eq!(v.stack[1].hot, Some(NotifyHit::Close));
    close_newest(&mut v, pitch, now);
    assert_eq!(
        rest_point(&mut v, x, y, true, now),
        None,
        "kept while sliding"
    );
    let settled = now + Duration::from_millis(300);
    assert_eq!(rest_point(&mut v, x, y, true, settled), Some((true, false)));
    assert_eq!(v.stack[0].hot, None, "the sheet slid away is dark");
    assert!(!v.stack[0].pointer_in);
}

#[test]
fn a_sheet_slid_under_a_still_pointer_lights_its_close() {
    let mut scene = crate::compositor::DeskScene::new();
    let (mut v, pitch) = sheets_at(&mut scene);
    let (x, y) = close_spot(&v, 1);
    let now = Instant::now();
    point_at(&mut v, x, y, true, now);
    close_newest(&mut v, pitch, now);
    rest_point(&mut v, x, y, true, now + Duration::from_millis(300));
    assert_eq!(v.stack[1].hot, Some(NotifyHit::Close));
    assert!(v.stack[1].pointer_in && hovered(&v));
}

/// Places and layouts for three Droplets by the taskbar, sheet `open` (if any) open.
fn droplet_slots(open: Option<usize>) -> Vec<((i32, i32), Drawn)> {
    let area = Rect::new(0, 0, 1920, 1040);
    let drawn: Vec<Drawn> = (0..3)
        .map(|i| {
            let t = if open == Some(i) { 1.0 } else { 0.0 };
            Drawn::Sheet(droplet_at(t, SheetShape::Droplet).0)
        })
        .collect();
    let footprints: Vec<(i32, i32)> = drawn.iter().map(Drawn::footprint).collect();
    let (spots, _) = place_stack(area, &footprints, None, Edge::Bottom);
    spots.into_iter().zip(drawn).collect()
}

fn place(v: &mut NotifyView, slots: &[((i32, i32), Drawn)]) {
    for (b, (at, d)) in v.stack.iter_mut().zip(slots) {
        b.at = *at;
        b.drawn = Some(d.clone());
    }
}

fn droplets(scene: &mut crate::compositor::DeskScene) -> NotifyView {
    let mut v = NotifyView {
        stack: [3, 2, 1].map(|id| balloon(scene, id)).into(),
        ..NotifyView::default()
    };
    for b in &mut v.stack {
        b.item = droplet_note();
    }
    place(&mut v, &droplet_slots(None));
    v
}

#[test]
fn a_droplet_slid_away_from_a_still_pointer_closes() {
    let (mut scene, e) = (crate::compositor::DeskScene::new(), glass_effects());
    let mut v = droplets(&mut scene);
    let (x, y) = close_spot(&v, 1);
    let now = Instant::now();
    point_at(&mut v, x, y, true, now);
    aim_stack(&mut v.stack, &e, now);
    assert_eq!(v.stack[1].motion.open.target(), 1.0);
    let pitch = v.stack[0].at.1 - v.stack[1].at.1;
    let open = now + Duration::from_millis(300);
    close_newest(&mut v, pitch, open);
    let settled = open + Duration::from_millis(300);
    rest_point(&mut v, x, y, true, settled);
    assert!(aim_stack(&mut v.stack, &e, settled));
    assert_eq!(v.stack[0].motion.open.target(), 0.0, "slid away, it closes");
    assert_eq!(v.stack[1].motion.open.target(), 1.0, "slid under, it opens");
}

#[test]
fn a_droplet_opening_under_a_still_pointer_stays_open_once_its_neighbours_move() {
    let (mut scene, e) = (crate::compositor::DeskScene::new(), glass_effects());
    let mut v = droplets(&mut scene);
    let (shut, open) = (droplet_slots(None), droplet_slots(Some(1)));
    let (at, d) = &shut[1];
    let ((ox, oy), (w, h)) = (d.origin(), d.size());
    assert_ne!(shut[2].0, open[2].0, "the older sheet is pushed");
    let now = Instant::now();
    let settled = now + Duration::from_millis(300);
    let mut tried = 0;
    for (lx, ly) in (0..w)
        .step_by(3)
        .flat_map(|lx| (0..h).step_by(3).map(move |ly| (lx, ly)))
    {
        if d.hit(lx, ly).is_none() {
            continue;
        }
        tried += 1;
        let (x, y) = (
            f64::from(at.0 + lx - ox) + 0.5,
            f64::from(at.1 + ly - oy) + 0.5,
        );
        place(&mut v, &shut);
        for b in &mut v.stack {
            b.motion = Motion::settled_at(now);
        }
        point_at(&mut v, x, y, true, now);
        aim_stack(&mut v.stack, &e, now);
        assert_eq!(v.stack[1].motion.open.target(), 1.0, "opens at ({x}, {y})");
        place(&mut v, &open);
        assert!(rest_point(&mut v, x, y, true, settled).is_some());
        assert!(
            !aim_stack(&mut v.stack, &e, settled),
            "nothing turns at ({x}, {y})"
        );
        assert!(v.stack[1].pointer_in, "still on it at ({x}, {y})");
    }
    assert!(tried > 100, "{tried} points on the shut sheet");
}

#[test]
fn a_picture_beside_the_slot_leaves_the_slot_to_the_stock_icon() {
    let theme = theme_for(Interface::Glass);
    let mut n = note(1, "Music");
    n.image = vec![IconSource {
        width: 48,
        height: 48,
        pixels: vec![0xFF00_00FF; 48 * 48],
    }];
    let lay = |shape| match lay_out(
        &theme,
        &text(),
        &Item::Note(n.clone()),
        false,
        Edge::Bottom,
        shape,
        false,
        "now",
    ) {
        Drawn::Sheet(l) => l,
        Drawn::Balloon(_) => unreachable!("Glass draws sheets"),
    };
    assert!(!slot_needs_icon(&lay(SheetShape::Lozenge), true));
    assert!(slot_needs_icon(&lay(SheetShape::Droplet), true));
    assert!(slot_needs_icon(&lay(SheetShape::Halo), true));
    assert!(slot_needs_icon(&lay(SheetShape::Lozenge), false));
}

#[test]
fn a_droplet_opening_under_a_still_pointer_keeps_the_stack_held() {
    let mut scene = crate::compositor::DeskScene::new();
    let theme = theme_for(Interface::Glass);
    let mut b = balloon(&mut scene, 1);
    if let Item::Note(n) = &mut b.item {
        n.actions = vec![("later".into(), "Later".into())];
        n.body = "Lunch at noon? The usual place by the river. Bring the photos.".into();
    }
    let lay = |t| {
        lay_out_open(
            &theme,
            &text(),
            &b.item,
            false,
            Edge::Bottom,
            SheetShape::Droplet,
            "now",
            t,
        )
        .0
    };
    let (shut, half) = (lay(0.0), lay(0.5));
    let bottom = 1000;
    b.at = (1550, bottom - shut.footprint().1);
    b.drawn = Some(shut);
    let mut v = NotifyView {
        stack: vec![b],
        ..NotifyView::default()
    };
    let (x, y) = (1700.0, f64::from(bottom - 10));
    let now = Instant::now();
    point_at(&mut v, x, y, true, now);
    v.stack[0].at.1 = bottom - half.footprint().1;
    v.stack[0].drawn = Some(half);
    assert!(!hold_at(&mut v, x, y, now));
    assert!(hovered(&v), "held while the sheet grows away from the bar");
    assert!(matches!(
        press_target(&mut v, x, y, now),
        Some((0, _, NotifyHit::Body, _))
    ));
}

#[test]
fn a_close_press_on_an_opening_droplet_is_taken() {
    let mut scene = crate::compositor::DeskScene::new();
    let (mut v, _) = sheets_at(&mut scene);
    let now = Instant::now();
    let began = now - Duration::from_millis(100);
    v.stack[0].motion.open = Tween::new(0.0, 1.0, began, 250, Ease::OutCubic);
    let (x, y) = close_spot(&v, 0);
    assert!(matches!(
        press_target(&mut v, x, y, now),
        Some((0, _, NotifyHit::Close, _))
    ));
}

#[test]
fn a_close_press_before_an_opening_droplet_shows_its_close_is_ignored() {
    let mut scene = crate::compositor::DeskScene::new();
    let (mut v, _) = sheets_at(&mut scene);
    let now = Instant::now();
    v.stack[0].motion.open = Tween::new(0.0, 1.0, now, 250, Ease::OutCubic);
    let (x, y) = close_spot(&v, 0);
    assert!(press_target(&mut v, x, y, now).is_none());
}

#[test]
fn a_droplet_dismissed_mid_open_is_gone_on_its_close_time() {
    let (t0, e) = (Instant::now(), glass_effects());
    let mut m = Motion::settled_at(t0);
    m.aim_open(1.0, &e, t0);
    let t1 = t0 + Duration::from_millis(10);
    m.leave(&e, t1);
    let gone = t1 + Duration::from_millis(u64::from(e.window_close_ms));
    assert!(e.window_close_ms < e.window_open_ms - 10);
    assert!(m.settled(gone));
}

#[test]
fn a_close_press_on_a_sliding_sheet_is_ignored_and_its_body_still_takes_one() {
    let mut scene = crate::compositor::DeskScene::new();
    let (mut v, _) = sheets_at(&mut scene);
    let now = Instant::now();
    let (x, y) = close_spot(&v, 0);
    assert!(matches!(
        press_target(&mut v, x, y, now),
        Some((0, _, NotifyHit::Close, _))
    ));
    v.stack[0].motion.slide_by(-10.0, now, 250);
    let y = y - 10.0;
    assert!(press_target(&mut v, x, y, now).is_none());
    assert!(matches!(
        press_target(&mut v, 200.0, y + 20.0, now),
        Some((0, _, NotifyHit::Body, _))
    ));
}

fn ids(items: &[Item]) -> Vec<u32> {
    items
        .iter()
        .filter_map(|i| i.note().map(|n| n.id))
        .collect()
}

#[test]
fn the_newest_sheet_stays_nearest_the_tray_across_caps_3_1_3() {
    let mut stack = items(&[3, 2, 1]);
    let mut queue: VecDeque<Item> = items(&[4]).into();
    let mut set_cap = |stack: &mut Vec<Item>, cap| {
        for item in overflow(stack, cap, |_| false) {
            queue.push_front(item);
        }
        for item in fill(stack.len(), cap, &mut queue) {
            stack.insert(0, item);
        }
        ids(&queue.iter().cloned().collect::<Vec<_>>())
    };
    assert_eq!(set_cap(&mut stack, 1), vec![2, 3, 4]);
    assert_eq!(ids(&stack), vec![1]);
    assert_eq!(set_cap(&mut stack, 3), vec![4]);
    assert_eq!(ids(&stack), vec![3, 2, 1]);
}

#[test]
fn a_smaller_cap_keeps_the_replying_sheet() {
    let mut stack = vec![1, 2, 3];
    assert_eq!(overflow(&mut stack, 1, |&n| n == 2), vec![1, 3]);
    assert_eq!(stack, vec![2]);
}

#[test]
fn a_replace_redraws_in_its_slot_and_leaves_the_others() {
    let mut scene = crate::compositor::DeskScene::new();
    let mut stack: Vec<Balloon> = [1, 2, 3].map(|id| balloon(&mut scene, id)).into();
    let trees: Vec<TreeId> = stack.iter().map(|b| b.tree).collect();
    let mut n = note(2, "Mail");
    n.summary = "edited".into();
    renew(&mut stack[1], n, 9, None);
    let ids: Vec<u32> = stack
        .iter()
        .filter_map(|b| b.item.note())
        .map(|n| n.id)
        .collect();
    assert_eq!(ids, [1, 2, 3]);
    assert_eq!(
        stack.iter().map(|b| b.tree).collect::<Vec<_>>(),
        trees,
        "same trees, no arrival"
    );
    assert_eq!(stack[1].item.note().unwrap().summary, "edited");
    assert_eq!(stack[1].serial, 9);
    assert!(
        stack[1].shown_at.elapsed() < Duration::from_secs(60),
        "its age starts over"
    );
    assert_eq!((stack[0].serial, stack[2].serial), (1, 3));
}

#[test]
fn a_new_burst_takes_the_oldest_taken_slot() {
    let mut stack = vec!['a', 'b', 'c', 'd', 'e'];
    let gone = fold_in(&mut stack, &[1, 3], Some('B'));
    assert_eq!(stack, ['a', 'c', 'B', 'e']);
    assert_eq!(gone, ['d', 'b']);
    let mut stack = vec!['a', 'b', 'c'];
    fold_in(&mut stack, &[2], Some('B'));
    assert_eq!(stack, ['a', 'b', 'B']);
}

#[test]
fn hiding_balloons_clears_every_visible_sheet() {
    let (mut stack, mut leaving) = (vec![1, 2, 3], vec![9]);
    assert_eq!(take_all(&mut stack, &mut leaving), vec![1, 2, 3, 9]);
    assert!(stack.is_empty() && leaving.is_empty());
}

fn glass_effects() -> EffectsConfig {
    EffectsConfig::default_for(Interface::Glass)
}

#[test]
fn with_window_animations_off_sheets_appear_settled() {
    let t0 = Instant::now();
    let off = EffectsConfig {
        window_animation: false,
        ..glass_effects()
    };
    let m = Motion::arrive(&off, t0);
    assert!(m.settled(t0));
    assert_eq!(m.k.value(t0), 1.0);
}

#[test]
fn a_sheet_arrives_over_the_window_open_time_and_leaves_over_the_close_time() {
    let (t0, e) = (Instant::now(), glass_effects());
    let mut m = Motion::arrive(&e, t0);
    assert_eq!(m.k.value(t0), 0.0);
    assert!(
        !m.settled(t0 + Duration::from_millis(249)) && m.settled(t0 + Duration::from_millis(250))
    );
    let t1 = t0 + Duration::from_millis(300);
    m.leave(&e, t1);
    assert!(!m.settled(t1 + Duration::from_millis(199)));
    assert_eq!(m.k.value(t1 + Duration::from_millis(200)), 0.0);
}

#[test]
fn a_slide_retargets_from_its_current_offset() {
    let t0 = Instant::now();
    let mut m = Motion::settled_at(t0);
    m.slide_by(104.0, t0, 250);
    assert_eq!(396.0 + m.offset(t0), 500.0, "starts where it was");
    let mid = t0 + Duration::from_millis(100);
    let seen = 396.0 + m.offset(mid);
    m.slide_by(-104.0, mid, 250);
    assert!((500.0 + m.offset(mid) - seen).abs() < 1e-9, "no jump");
    assert_eq!(m.offset(mid + Duration::from_millis(250)), 0.0);
}

#[test]
fn a_repaint_in_the_same_slot_starts_no_slide_and_arms_no_tick() {
    let t0 = Instant::now();
    let mut scene = crate::compositor::DeskScene::new();
    let mut v = NotifyView::default();
    v.stack.push(balloon(&mut scene, 1));
    follow_slot(
        &mut v.stack[0].motion,
        (400, 100),
        (400, 100),
        false,
        t0,
        250,
    );
    assert!(still(&v, t0), "settled, so no tick is armed");
    follow_slot(
        &mut v.stack[0].motion,
        (400, 100),
        (296, 100),
        false,
        t0,
        250,
    );
    assert!(!still(&v, t0));
    assert_eq!(v.stack[0].motion.offset(t0), 104.0);
}

#[test]
fn a_sheet_growing_in_place_keeps_its_anchored_edge_still() {
    let t0 = Instant::now();
    let mut up = Motion::settled_at(t0);
    follow_slot(&mut up, (400, 100), (360, 140), false, t0, 250);
    assert!(up.settled(t0), "a stack growing up holds the bottom");
    let mut down = Motion::settled_at(t0);
    follow_slot(&mut down, (40, 100), (40, 140), true, t0, 250);
    assert!(down.settled(t0), "a stack growing down holds the top");
    follow_slot(&mut down, (40, 100), (80, 100), true, t0, 250);
    assert_eq!(down.offset(t0), -40.0);
}

#[test]
fn a_hovered_droplet_opens_and_a_replying_one_stays_open() {
    assert_eq!(
        [
            open_target(true, false),
            open_target(false, true),
            open_target(false, false)
        ],
        [1.0, 1.0, 0.0]
    );
}

#[test]
fn a_droplet_arrives_over_the_drip_time() {
    let (t0, e) = (Instant::now(), glass_effects());
    let m = Motion::drip(&e, t0);
    assert!(
        !m.settled(t0 + Duration::from_millis(599)) && m.settled(t0 + Duration::from_millis(600))
    );
    let off = EffectsConfig {
        window_animation: false,
        ..e
    };
    assert!(Motion::drip(&off, t0).settled(t0));
}

#[test]
fn a_leaving_droplet_poses_like_any_sheet() {
    let (t0, e) = (Instant::now(), glass_effects());
    let mut m = Motion::drip(&e, t0);
    assert!(m.drip);
    m.leave(&e, t0 + Duration::from_millis(100));
    assert!(!m.drip);
}

#[test]
fn a_droplet_opens_over_the_open_time_and_closes_over_the_close_time() {
    let (t0, e) = (Instant::now(), glass_effects());
    let ms = Duration::from_millis;
    let mut m = Motion::settled_at(t0);
    assert!(m.aim_open(1.0, &e, t0));
    assert!(!m.aim_open(1.0, &e, t0 + ms(10)), "already on its way");
    assert!(!m.settled(t0 + ms(249)) && m.settled(t0 + ms(250)));
    assert_eq!(m.open.value(t0 + ms(250)), 1.0);
    let t1 = t0 + ms(300);
    m.aim_open(0.0, &e, t1);
    assert!(!m.settled(t1 + ms(199)));
    assert_eq!(m.open.value(t1 + ms(200)), 0.0);
    let off = EffectsConfig {
        window_animation: false,
        ..e
    };
    let mut m = Motion::settled_at(t0);
    m.aim_open(1.0, &off, t0);
    assert!(m.settled(t0));
}

fn droplet_note() -> Item {
    let mut n = note(1, "Chat");
    n.body = "Lunch at noon?\nThe usual place.\nBring the photos.".into();
    Item::Note(n)
}

fn droplet_at(t: f64, shape: SheetShape) -> (SheetLayout, Option<[SheetLayout; 2]>) {
    let (drawn, ends) = lay_out_open(
        &theme_for(Interface::Glass),
        &text(),
        &droplet_note(),
        false,
        Edge::Bottom,
        shape,
        "now",
        t,
    );
    let Drawn::Sheet(lay) = drawn else {
        panic!("a sheet")
    };
    (lay, ends)
}

#[test]
fn an_opening_droplet_lays_out_between_its_ends() {
    let (shut, ends) = droplet_at(0.0, SheetShape::Droplet);
    assert!(!shut.expanded);
    assert_eq!(shut.sheet.height, 68);
    assert_eq!(ends.as_ref().map(|[c, _]| c), Some(&shut));
    let (open, ends) = droplet_at(1.0, SheetShape::Droplet);
    assert!(open.expanded);
    assert_eq!(ends.as_ref().map(|[_, e]| e), Some(&open));
    let (half, ends) = droplet_at(0.5, SheetShape::Droplet);
    let [c, e] = ends.expect("both ends while it opens");
    assert_eq!((c, &e), (shut, &open));
    assert!(half.sheet.height > 68 && half.sheet.height < e.sheet.height);
    let (lozenge, none) = droplet_at(0.5, SheetShape::Lozenge);
    assert!(none.is_none() && !lozenge.expanded);
}

#[test]
fn a_droplet_opening_by_the_taskbar_keeps_its_bottom_still() {
    let t0 = Instant::now();
    let area = Rect::new(0, 0, 1920, 1040);
    let size = |t| {
        let fp = droplet_at(t, SheetShape::Droplet).0.footprint;
        (fp.width, fp.height)
    };
    let (shut, open) = (size(0.0), size(1.0));
    let slot = |s: (i32, i32)| place_stack(area, &[s], None, Edge::Bottom).0[0];
    let (was, now_at) = (slot(shut), slot(open));
    assert_eq!(was.1 + shut.1, now_at.1 + open.1);
    let mut m = Motion::settled_at(t0);
    follow_slot(&mut m, (was.1, shut.1), (now_at.1, open.1), false, t0, 250);
    assert!(m.settled(t0), "no dip toward the taskbar");
}

#[test]
fn a_droplet_pane_is_its_footprint_with_the_droplet_corner() {
    let t = theme_for(Interface::Glass).scaled(2);
    let Drawn::Sheet(lay) = lay_out(
        &t,
        &text(),
        &droplet_note(),
        false,
        Edge::Bottom,
        SheetShape::Droplet,
        false,
        "now",
    ) else {
        panic!()
    };
    let pane = sheet_glass(&lay, t.look.style().unwrap().notify.as_ref().unwrap(), 2);
    assert_eq!(pane.shapes.len(), 1);
    assert_eq!(pane.shapes[0].rect, Rect::new(0, 0, 360, 68));
    assert_eq!(
        pane.shapes[0].corner,
        &reprisekit::glass::notify::DROPLET_CORNER[..]
    );
}

#[test]
fn critical_and_unnamed_notifications_never_fold() {
    assert!(foldable(&note(1, "Mail")));
    assert!(!foldable(&note(1, "")));
    let mut crit = note(1, "Mail");
    crit.urgency = Urgency::Critical;
    assert!(!foldable(&crit));
    let mut own = note(OWN_ID, OWN_APP);
    own.id = OWN_ID;
    assert!(!foldable(&own));
}

/// Each tree's index among the scene root's children, bottom first.
fn depth(scene: &crate::compositor::DeskScene, b: &Balloon) -> usize {
    let root = scene.root();
    let at = scene
        .children(root)
        .iter()
        .position(|c| *c == b.tree.into());
    at.expect("a sheet's tree under the root")
}

#[test]
fn a_closed_sheet_goes_under_every_sheet_in_the_stack() {
    let mut scene = crate::compositor::DeskScene::new();
    let now = Instant::now();
    let mut stack: Vec<Balloon> = (1..=3).map(|id| balloon(&mut scene, id)).collect();
    let mut gone = stack.remove(1);
    gone.motion.leave(&glass_effects(), now);
    let leaving = vec![gone];
    restack(&mut scene, &stack, &leaving, now);
    let low = depth(&scene, &leaving[0]);
    assert!(stack.iter().all(|b| depth(&scene, b) > low));
    assert_eq!(low, 0, "the leaving tree is the lowest of the three");
}

#[test]
fn a_foreign_overlay_node_between_sheets_keeps_its_index() {
    let mut scene = crate::compositor::DeskScene::new();
    let now = Instant::now();
    let mut gone = balloon(&mut scene, 1);
    let foreign = scene.tree_create(scene.root());
    let stack = vec![balloon(&mut scene, 2)];
    gone.motion.leave(&glass_effects(), now);
    let mut new = balloon(&mut scene, 3);
    new.motion = Motion::arrive(&glass_effects(), now);
    let stack = vec![new, stack.into_iter().next().unwrap()];
    restack(&mut scene, &stack, &[gone], now);
    let root = scene.root();
    let at = scene
        .children(root)
        .iter()
        .position(|c| *c == foreign.into());
    assert_eq!(at, Some(1), "the foreign node stays second from the bottom");
}

#[test]
fn an_arriving_sheet_sits_under_resting_ones_and_over_leaving_ones() {
    let mut scene = crate::compositor::DeskScene::new();
    let now = Instant::now();
    let mut leaving = vec![balloon(&mut scene, 1)];
    leaving[0].motion.leave(&glass_effects(), now);
    let mut stack = vec![balloon(&mut scene, 2), balloon(&mut scene, 3)];
    let mut new = balloon(&mut scene, 4);
    new.motion = Motion::arrive(&glass_effects(), now);
    stack.insert(0, new);
    restack(&mut scene, &stack, &leaving, now);
    let order: Vec<usize> = [&leaving[0], &stack[0], &stack[1], &stack[2]]
        .iter()
        .map(|b| depth(&scene, b))
        .collect();
    assert_eq!(
        order,
        vec![0, 1, 2, 3],
        "leaving, arriving, then the resting two"
    );
    let done = now + Duration::from_secs(10);
    assert_eq!(band(&stack[0].motion, false, done), Band::Resting);
}

#[test]
fn a_notification_click_hands_its_program_a_token_that_raises_its_window() {
    use crate::tray::{TrayCommand, TrayEvent};
    use std::os::fd::AsRawFd;
    const PID: i32 = i32::MAX - 637;
    let mut h = crate::harness::Harness::new();
    h.output("TEST-1", 1024, 768);
    let (tx, wake, heard) = crate::tray::headless(&mut h.state);
    let mut c = h.connect();
    let mail = h.map_toplevel(&mut c, "mail", 300, 200);
    let other = h.map_toplevel(&mut c, "other", 300, 200);
    assert_eq!(h.state.focused, Some(other.id));

    let mut n = note(7, "Mail");
    n.actions = vec![("default".into(), "Open".into())];
    n.pid = Some(PID);
    tx.send(TrayEvent::Notify(n)).expect("send");
    // SAFETY: `wake` is an open eventfd and the buffer a live 8-byte local.
    let wrote = unsafe { libc::write(wake.as_raw_fd(), (&1u64 as *const u64).cast(), 8) };
    assert_eq!(wrote, 8);
    h.dispatch();
    invoke(&mut h.state, 7, "default");

    let token = std::iter::from_fn(|| heard.try_recv().ok())
        .find_map(|cmd| match cmd {
            TrayCommand::ActionInvoked { id: 7, key, token } if key == "default" => token,
            _ => None,
        })
        .expect("ActionInvoked carries a token");
    c.activate(&token, mail);
    h.roundtrip(&mut c);
    assert_eq!(
        h.state.focused,
        Some(mail.id),
        "the token raises the window"
    );
}

#[test]
fn a_notification_click_lets_its_senders_next_window_take_focus() {
    use crate::tray::TrayEvent;
    use std::os::fd::AsRawFd;
    const SENDER: i32 = i32::MAX - 638;
    const STRANGER: i32 = i32::MAX - 639;
    let mut h = crate::harness::Harness::new();
    h.output("TEST-1", 1024, 768);
    let (tx, wake, _heard) = crate::tray::headless(&mut h.state);
    let mut editor = h.connect();
    let typing = h.map_toplevel(&mut editor, "editor", 300, 200);
    let mut stranger = h.connect_as(STRANGER);
    h.map_toplevel(&mut stranger, "stranger", 300, 200);
    assert_eq!(
        h.state.focused,
        Some(typing.id),
        "another program's window maps behind the focused one"
    );

    let mut n = note(8, "Mail");
    n.actions = vec![("default".into(), "Open".into())];
    n.pid = Some(SENDER);
    tx.send(TrayEvent::Notify(n)).expect("send");
    // SAFETY: `wake` is an open eventfd and the buffer a live 8-byte local.
    let wrote = unsafe { libc::write(wake.as_raw_fd(), (&1u64 as *const u64).cast(), 8) };
    assert_eq!(wrote, 8);
    h.dispatch();
    invoke(&mut h.state, 8, "default");

    let mut sender = h.connect_as(SENDER);
    let mail = h.map_toplevel(&mut sender, "mail", 300, 200);
    assert_eq!(
        h.state.focused,
        Some(mail.id),
        "the clicked notification's program maps its window in front"
    );
}

#[test]
fn a_sheet_draws_and_asks_for_frames_only_on_its_output() {
    let config = crate::config::Config::parse("[appearance]\ninterface = \"glass\"\n").unwrap();
    let mut h = crate::harness::Harness::with_config(config);
    let a = h.output("TEST-1", 1024, 768);
    let b = h.output("TEST-2", 1024, 768);
    show(&mut h.state, note(1, "Mail"));
    h.run_for(50);
    let v = &h.state.notify;
    assert_eq!(frame_outputs(v), Some(vec![a.clone()]));
    let scene = &h.state.scene;
    let tree = v.stack[0].tree;
    let canvas = scene.extents(tree).expect("the sheet's canvas");
    let beside = reprisescene::output_rect(&b).expect("TEST-2's place");
    assert!(
        canvas.overlaps(beside),
        "the shadow reaches past TEST-1's edge: {canvas:?}"
    );
    let (at, _) = scene.coords(tree).expect("the sheet's place");
    let clip = scene.clip(tree).expect("a clip");
    assert_eq!(
        Rectangle::new(clip.loc + at, clip.size),
        reprisescene::output_rect(&a).expect("TEST-1's place")
    );
}

#[test]
fn a_low_urgency_balloon_and_the_sound_key_off_are_silent() {
    let mut n = note(1, "Mail");
    n.sound = Some(repriseaudio::sounds::Sound::Name("bell".into()));
    assert_eq!(balloon_sound(&n, true), n.sound.as_ref());
    assert_eq!(balloon_sound(&n, false), None);
    n.urgency = Urgency::Critical;
    assert_eq!(balloon_sound(&n, true), n.sound.as_ref());
    n.urgency = Urgency::Low;
    assert_eq!(balloon_sound(&n, true), None);
}
