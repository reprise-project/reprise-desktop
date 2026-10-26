use super::gamma::{GammaStep, Lut, gamma_step, lut_from_blob};
use super::links::{LINK_TRIES, LinkStep, link_mode, link_step};
use super::outputs::{Scan, ScanAction, asks_scan, scan_action};
use super::render::{glass_hold, waits_for_frame};
use super::schedule::{Asked, pointer_ask_renders};
use super::steps::{MODE_TRIES, PendingMode, Step, next_step};
use super::{Ask, CrtcNow, pick_gpu};
use crate::config::OutputMode;
use crate::output::ModeInfo;
use smithay::utils::{Point, Rectangle};

fn asked(asks: &[Ask]) -> Asked {
    let mut a = Asked::default();
    for ask in asks {
        a.add(*ask);
    }
    a
}

#[test]
fn a_held_output_answers_frames_on_the_timer() {
    assert!(waits_for_frame(true, false, false, true));
    assert!(
        !waits_for_frame(true, false, true, true),
        "a held output shows no frame for its clients to wait on"
    );
    assert!(!waits_for_frame(true, true, false, true));
    assert!(!waits_for_frame(true, false, false, false));
    assert!(!waits_for_frame(false, false, false, true));
}

#[test]
fn a_pointer_ask_renders_only_the_output_the_pointer_is_on_or_left() {
    let left = Some(Rectangle::new((0, 0).into(), (1920, 1080).into()));
    let right = Some(Rectangle::new((1920, 0).into(), (1920, 1080).into()));
    let on_left = Point::from((960.0, 540.0));
    let right_edge = Point::from((1920.0, 0.0));
    assert!(pointer_ask_renders(left, on_left, false, false));
    assert!(
        !pointer_ask_renders(right, on_left, false, false),
        "the output away from the pointer has nothing to draw"
    );
    assert!(
        pointer_ask_renders(right, on_left, true, false),
        "the output the pointer left draws it away"
    );
    assert!(pointer_ask_renders(right, on_left, false, true));
    assert!(pointer_ask_renders(right, right_edge, false, false));
    assert!(!pointer_ask_renders(None, on_left, false, false));
}

#[test]
fn only_pointer_asks_make_a_pointer_only_frame() {
    assert!(asked(&[Ask::Pointer]).pointer_only);
    assert!(asked(&[Ask::Pointer, Ask::Pointer]).pointer_only);
    assert!(!asked(&[Ask::Pointer, Ask::Content]).pointer_only);
    assert!(!asked(&[Ask::Content, Ask::Pointer]).pointer_only);
    assert!(!asked(&[Ask::Scene]).pointer_only);
    let a = asked(&[Ask::Scene, Ask::Pointer]);
    assert!(a.redraw && a.scene && !a.pointer_only);
}

#[test]
fn only_catch_up_asks_make_a_catch_up_frame() {
    assert!(asked(&[Ask::CatchUp]).catch_up);
    assert!(!asked(&[Ask::CatchUp, Ask::Pointer]).catch_up);
    assert!(!asked(&[Ask::Content, Ask::CatchUp]).catch_up);
    let a = asked(&[Ask::CatchUp]);
    assert!(a.redraw && !a.scene && !a.pointer_only);
    assert_eq!(a.backdrop, crate::glass::Under::Nothing);
}

#[test]
fn only_scene_content_and_commit_asks_may_change_what_lies_under_glass() {
    use crate::glass::{Drawn, Under};
    use reprisescene::NodeId;
    use smithay::utils::Rectangle;
    let mut scene = crate::compositor::DeskScene::new();
    let (a, b): (NodeId, NodeId) = (
        scene.tree_create(scene.root()).into(),
        scene.tree_create(scene.root()).into(),
    );
    let drawn = |x, y, w, h, window| Drawn {
        r: Rectangle::new((x, y).into(), (w, h).into()),
        window: Some(window),
    };
    let under = |asks: &[Ask]| asked(asks).backdrop;
    let first = drawn(10, 10, 40, 30, a);
    assert_eq!(under(&[Ask::Content]), Under::Anywhere);
    assert_eq!(under(&[Ask::Scene]), Under::Anywhere);
    assert_eq!(under(&[Ask::Cursor, Ask::Content]), Under::Anywhere);
    assert_eq!(under(&[Ask::Commit(first), Ask::Content]), Under::Anywhere);
    assert_eq!(
        under(&[Ask::Pointer, Ask::Commit(first)]),
        Under::Within(first)
    );
    assert_eq!(
        under(&[Ask::Commit(first), Ask::Commit(drawn(100, 50, 10, 10, a))]),
        Under::Within(drawn(10, 10, 100, 50, a)),
        "two commits ask for the box around both"
    );
    assert_eq!(
        under(&[Ask::Commit(first), Ask::Commit(drawn(100, 50, 10, 10, b))]),
        Under::Within(Drawn {
            window: None,
            ..drawn(10, 10, 100, 50, a)
        }),
        "commits in two windows lie in neither alone"
    );
    assert_eq!(under(&[Ask::Pointer]), Under::Nothing);
    let cursor = asked(&[Ask::Cursor, Ask::Pointer]);
    assert!(cursor.redraw && !cursor.pointer_only && cursor.backdrop == Under::Nothing);
}

#[test]
fn a_held_frame_water_stepped_in_changed_what_lies_under_glass_anywhere() {
    use crate::glass::{Hold, Under};
    let wait = Some(std::time::Duration::from_millis(10));
    assert_eq!(
        glass_hold(wait, Under::Nothing, true),
        Hold::Keep(Under::Anywhere)
    );
    assert_eq!(
        glass_hold(wait, Under::Nothing, false),
        Hold::Keep(Under::Nothing)
    );
    assert_eq!(glass_hold(None, Under::Nothing, true), Hold::No);
}

#[test]
fn a_frame_resets_the_asks() {
    let mut a = asked(&[Ask::Content]);
    a.redraw = false;
    a.add(Ask::Pointer);
    assert!(a.pointer_only, "a stale pointer_only=false does not stick");
}

const READY: CrtcNow = CrtcNow {
    active: true,
    off: false,
    unlit: false,
    flipping: false,
};

fn not_ready() -> [(CrtcNow, &'static str); 4] {
    [
        (
            CrtcNow {
                active: false,
                ..READY
            },
            "session away",
        ),
        (CrtcNow { off: true, ..READY }, "output off"),
        (
            CrtcNow {
                flipping: true,
                ..READY
            },
            "flip in flight",
        ),
        (
            CrtcNow {
                unlit: true,
                ..READY
            },
            "no frame since power-on",
        ),
    ]
}

#[test]
fn a_ramp_the_crtc_holds_is_never_committed() {
    let warm = Lut::Ramp(vec![1, 2, 3]);
    for (crtc, _) in not_ready().into_iter().chain([(READY, "ready")]) {
        assert_eq!(
            gamma_step(&Lut::Identity, Some(&Lut::Identity), crtc),
            GammaStep::Skip
        );
        assert_eq!(gamma_step(&warm, Some(&warm), crtc), GammaStep::Skip);
    }
}

#[test]
fn a_new_ramp_commits_on_a_ready_crtc_else_waits() {
    let warm = Lut::Ramp(vec![1, 2, 3]);
    let warmer = Lut::Ramp(vec![1, 2, 2]);
    assert_eq!(
        gamma_step(&warm, Some(&Lut::Identity), READY),
        GammaStep::Commit
    );
    assert_eq!(gamma_step(&warmer, Some(&warm), READY), GammaStep::Commit);
    assert_eq!(
        gamma_step(&Lut::Identity, Some(&warm), READY),
        GammaStep::Commit
    );
    for (crtc, why) in not_ready() {
        assert_eq!(
            gamma_step(&warm, Some(&Lut::Identity), crtc),
            GammaStep::Wait,
            "{why}"
        );
    }
}

#[test]
fn another_masters_lut_is_replaced_before_the_first_frame() {
    let unlit = CrtcNow {
        unlit: true,
        ..READY
    };
    assert_eq!(gamma_step(&Lut::Identity, None, unlit), GammaStep::Commit);
    for (crtc, why) in not_ready().into_iter().take(3) {
        assert_eq!(
            gamma_step(&Lut::Identity, None, crtc),
            GammaStep::Wait,
            "{why}"
        );
    }
}

#[test]
fn a_gamma_lut_reads_back_as_identity_ours_or_unknown() {
    let warm = Lut::Ramp(vec![1, 2, 3]);
    assert_eq!(
        lut_from_blob(0, Some(7), Some(warm.clone())),
        Some(Lut::Identity)
    );
    assert_eq!(
        lut_from_blob(7, Some(7), Some(warm.clone())),
        Some(warm.clone())
    );
    assert_eq!(
        lut_from_blob(9, Some(7), Some(warm)),
        None,
        "another master's blob"
    );
    assert_eq!(lut_from_blob(9, None, None), None);
}

#[test]
fn a_good_link_is_never_retrained() {
    for tries in 0..=LINK_TRIES + 1 {
        assert_eq!(link_step(false, tries), LinkStep::Good);
    }
}

#[test]
fn a_bad_link_is_retrained_three_times_in_a_row_then_left() {
    assert_eq!(LINK_TRIES, 3);
    for tries in 0..LINK_TRIES {
        assert_eq!(link_step(true, tries), LinkStep::Retrain, "try {tries}");
    }
    assert_eq!(link_step(true, LINK_TRIES), LinkStep::GiveUp);
    assert_eq!(link_step(true, LINK_TRIES + 1), LinkStep::GaveUp);
}

#[test]
fn deferred_work_runs_one_step_at_a_time_reset_probes_scan_enables_config() {
    let mut reset = true;
    let mut rescan = true;
    let mut unprobed: Option<usize> = None;
    let mut reenable = 2;
    let mut mode = true;
    let mut config = true;
    let mut steps = Vec::new();
    while let Some(step) = next_step(reset, rescan, unprobed, reenable > 0, mode, config) {
        steps.push(step);
        match step {
            Step::Reset => reset = false,
            Step::Probe if rescan => (rescan, unprobed) = (false, Some(2)),
            Step::Probe => unprobed = unprobed.map(|n| n - 1),
            Step::Scan => unprobed = None,
            Step::Reenable => reenable -= 1,
            Step::Mode => mode = false,
            Step::Config => config = false,
        }
        assert!(steps.len() < 20, "never ends: {steps:?}");
    }
    use Step::*;
    assert_eq!(
        steps,
        [
            Reset, Probe, Probe, Probe, Scan, Reenable, Reenable, Mode, Config
        ],
        "the first probe step lists three connectors and probes one"
    );
    assert_eq!(
        next_step(false, true, Some(1), true, true, true),
        Some(Probe),
        "a new uevent starts the scan over"
    );
    assert_eq!(
        next_step(false, false, Some(0), true, true, true),
        Some(Scan)
    );
}

#[test]
fn a_refused_mode_is_due_at_once_then_backs_off_and_gives_up() {
    let t0 = std::time::Instant::now();
    let secs = std::time::Duration::from_secs;
    let mut p = PendingMode::new(7);
    assert!(p.due(t0), "the first try goes once the display is idle");
    assert_eq!(p.waits_until(t0), None);
    let mut now = t0;
    let mut waits = Vec::new();
    while let Some((next, wait)) = p.refused(now) {
        assert!(!next.due(now), "a refusal waits before the next try");
        assert_eq!(next.waits_until(now), Some(now + wait));
        assert!(next.due(now + wait));
        assert_eq!(next.mode, 7);
        waits.push(wait.as_secs());
        (p, now) = (next, now + wait + secs(1));
        assert!(waits.len() < 20, "never gives up");
    }
    assert_eq!(waits, [1, 2, 4, 8, 16], "{MODE_TRIES} tries in all");
}

#[test]
fn a_link_retrains_its_mode_while_listed_else_the_closest_one() {
    let m = |width, height, hz: i32, preferred| ModeInfo {
        width,
        height,
        refresh_mhz: hz * 1000,
        preferred,
    };
    let current = m(3840, 2160, 60, false);
    let lower = [
        m(3840, 2160, 30, false),
        m(3840, 2160, 24, false),
        m(1920, 1080, 60, true),
    ];
    let with_current = [m(1920, 1080, 60, true), current];
    assert_eq!(
        link_mode(&with_current, &current, None),
        None,
        "still listed"
    );
    assert_eq!(
        link_mode(&lower, &current, None),
        Some(0),
        "same size, nearest refresh"
    );
    let want = OutputMode {
        width: 1920,
        height: 1080,
        refresh_mhz: None,
    };
    assert_eq!(
        link_mode(&lower, &current, Some(&want)),
        Some(2),
        "the configured mode"
    );
    let smaller = [m(2560, 1440, 60, false), m(1920, 1080, 60, true)];
    assert_eq!(
        link_mode(&smaller, &current, None),
        Some(1),
        "the preferred mode"
    );
    assert_eq!(
        link_mode(&smaller[..1], &current, None),
        Some(0),
        "else the first"
    );
    assert_eq!(link_mode(&[], &current, None), None, "nothing to switch to");
}

#[test]
fn a_retrain_waits_for_a_ready_crtc() {
    assert!(READY.ready());
    for (crtc, why) in not_ready() {
        assert!(!crtc.ready(), "{why}");
    }
}

#[test]
fn only_a_hotplug_uevent_asks_for_a_connector_scan() {
    assert!(asks_scan(Some("1".as_ref())), "a hotplug scans");
    assert!(
        !asks_scan(None),
        "the synthetic change of a VT switch does not"
    );
    assert!(!asks_scan(Some("0".as_ref())));
}

#[test]
fn a_changed_mode_list_refreshes_the_output_holding_the_crtc() {
    assert_eq!(
        scan_action(true, false, Scan::Changed, true),
        ScanAction::Refresh
    );
    assert_eq!(
        scan_action(false, true, Scan::Changed, true),
        ScanAction::Refresh
    );
    assert_eq!(
        scan_action(true, false, Scan::Changed, false),
        ScanAction::Refresh
    );
}

#[test]
fn a_changed_mode_list_on_an_unheld_crtc_connects_it() {
    assert_eq!(
        scan_action(false, false, Scan::Changed, true),
        ScanAction::Connect
    );
}

#[test]
fn a_connector_without_modes_is_parked_out_of_the_layout() {
    assert_eq!(
        scan_action(false, false, Scan::Connected, false),
        ScanAction::Park
    );
    assert_eq!(
        scan_action(false, false, Scan::Connected, true),
        ScanAction::Connect
    );
}

#[test]
fn a_connect_on_a_held_crtc_is_ignored_and_a_disconnect_always_applies() {
    assert_eq!(
        scan_action(true, false, Scan::Connected, true),
        ScanAction::Ignore
    );
    assert_eq!(
        scan_action(false, true, Scan::Connected, true),
        ScanAction::Ignore
    );
    assert_eq!(
        scan_action(true, false, Scan::Disconnected, true),
        ScanAction::Disconnect
    );
    assert_eq!(
        scan_action(false, false, Scan::Disconnected, true),
        ScanAction::Disconnect
    );
}

#[test]
fn a_drm_fd_that_keeps_failing_ends_the_session_and_leaves_the_loop() {
    use smithay::reexports::calloop::ping::make_ping;
    use std::cell::Cell;
    use std::rc::Rc;

    let mut h = crate::harness::Harness::new();
    let (ping, source) = make_ping().expect("ping");
    let errors = Cell::new(0);
    let calls = Rc::new(Cell::new(0));
    let token = Rc::new(Cell::new(None));
    let (seen, own) = (calls.clone(), token.clone());
    let t = h
        .state
        .handle
        .insert_source(source, move |(), _, state| {
            seen.set(seen.get() + 1);
            let e = smithay::backend::drm::DrmError::DeviceInactive;
            super::drm_error(state, &errors, own.get(), &e);
        })
        .expect("source");
    token.set(Some(t));
    for _ in 0..super::DRM_ERRORS_GONE - 1 {
        ping.ping();
        h.dispatch();
    }
    assert!(
        !h.state.session_fade.fading_out(),
        "errors below the limit end nothing"
    );
    for _ in 0..3 {
        ping.ping();
        h.dispatch();
    }
    assert_eq!(
        calls.get(),
        super::DRM_ERRORS_GONE,
        "the source is removed at the limit"
    );
    assert!(
        h.state.session_fade.fading_out(),
        "the session fades out to end"
    );
}

#[test]
fn the_configured_gpu_is_driven_only_when_the_seat_has_it() {
    assert_eq!(pick_gpu(Some(2), Some(1), &[1, 2]), Some(2));
    assert_eq!(
        pick_gpu(Some(3), Some(1), &[1, 2]),
        Some(1),
        "a GPU the seat lacks falls back to the primary"
    );
    assert_eq!(pick_gpu(None, Some(1), &[1, 2]), Some(1));
    assert_eq!(pick_gpu(None, None, &[2, 1]), Some(2));
    assert_eq!(pick_gpu::<u8>(None, None, &[]), None);
}
