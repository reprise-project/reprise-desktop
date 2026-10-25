use super::gamma::{GammaStep, Lut, gamma_step, lut_from_blob};
use super::links::{LINK_TRIES, LinkStep, link_mode, link_step};
use super::outputs::asks_scan;
use super::render::waits_for_frame;
use super::schedule::{Asked, pointer_ask_renders};
use super::steps::{Step, next_step};
use super::{Ask, CrtcNow};
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
fn only_scene_content_and_commit_asks_may_change_what_lies_under_glass() {
    use crate::glass::Under;
    use smithay::utils::Rectangle;
    let drawn = Rectangle::new((10, 10).into(), (40, 30).into());
    let under = |asks: &[Ask]| asked(asks).backdrop;
    assert_eq!(under(&[Ask::Content]), Under::Anywhere);
    assert_eq!(under(&[Ask::Scene]), Under::Anywhere);
    assert_eq!(under(&[Ask::Cursor, Ask::Content]), Under::Anywhere);
    assert_eq!(under(&[Ask::Commit(drawn), Ask::Content]), Under::Anywhere);
    assert_eq!(
        under(&[Ask::Pointer, Ask::Commit(drawn)]),
        Under::Within(drawn)
    );
    let other = Rectangle::new((100, 50).into(), (10, 10).into());
    assert_eq!(
        under(&[Ask::Commit(drawn), Ask::Commit(other)]),
        Under::Within(Rectangle::new((10, 10).into(), (100, 50).into())),
        "two commits ask for the box around both"
    );
    assert_eq!(under(&[Ask::Pointer]), Under::Nothing);
    let cursor = asked(&[Ask::Cursor, Ask::Pointer]);
    assert!(cursor.redraw && !cursor.pointer_only && cursor.backdrop == Under::Nothing);
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
    let mut config = true;
    let mut steps = Vec::new();
    while let Some(step) = next_step(reset, rescan, unprobed, reenable > 0, config) {
        steps.push(step);
        match step {
            Step::Reset => reset = false,
            Step::Probe if rescan => (rescan, unprobed) = (false, Some(2)),
            Step::Probe => unprobed = unprobed.map(|n| n - 1),
            Step::Scan => unprobed = None,
            Step::Reenable => reenable -= 1,
            Step::Config => config = false,
        }
        assert!(steps.len() < 20, "never ends: {steps:?}");
    }
    use Step::*;
    assert_eq!(
        steps,
        [Reset, Probe, Probe, Probe, Scan, Reenable, Reenable, Config],
        "the first probe step lists three connectors and probes one"
    );
    assert_eq!(
        next_step(false, true, Some(1), true, true),
        Some(Probe),
        "a new uevent starts the scan over"
    );
    assert_eq!(next_step(false, false, Some(0), true, true), Some(Scan));
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
