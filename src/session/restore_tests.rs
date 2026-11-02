use std::time::Instant;

use crate::harness::{Harness, TestClient, Toplevel};
use crate::raster::Rect;
use crate::snap::ZoneLayout;
use crate::window::WindowId;

use super::{SavedApp, SavedTabs};
use crate::sessionsnap::SavedSnap;

type Saved = (Vec<SavedApp>, Vec<SavedTabs>, Vec<SavedSnap>);

fn desk() -> (Harness, TestClient) {
    let mut h = Harness::new();
    h.output("DP-1", 1920, 1080);
    h.state.app_index = vec![
        reprisefs::apps::parse_desktop(
            "[Desktop Entry]\nType=Application\nName=Foot\nExec=foot\n",
            "foot.desktop",
        )
        .unwrap(),
    ];
    let c = h.connect();
    (h, c)
}

/// The session `h` would restore at the next login.
fn saved(h: &mut Harness) -> Saved {
    super::save(&mut h.state);
    let s = &h.state.session;
    (s.apps.clone(), s.tabs.clone(), s.snaps.clone())
}

/// A fresh desktop restoring `saved`, as if restore had just started its apps.
fn restoring((apps, tabs, snaps): Saved) -> (Harness, TestClient) {
    let (mut h, c) = desk();
    let s = &mut h.state.session;
    (s.apps, s.tabs, s.snaps) = (apps.clone(), tabs, snaps);
    super::await_windows(&mut h.state, &apps, Instant::now());
    (h, c)
}

fn foot(h: &mut Harness, c: &mut TestClient, title: &str) -> WindowId {
    let t: Toplevel = h.map_titled(c, "foot", title, (500, 400));
    t.id
}

fn halves() -> ZoneLayout {
    ZoneLayout::parse("1|1").unwrap()
}

fn frame(h: &Harness, w: WindowId) -> Rect {
    h.state.win(w).unwrap().frame_rect()
}

/// A session with foot windows "left" and "right" snapped side by side, and one titled `loose`
/// outside them when given.
fn snapped_pair_and(loose: Option<&str>) -> (Saved, [Rect; 2]) {
    let (mut h, mut c) = desk();
    if let Some(t) = loose {
        foot(&mut h, &mut c, t);
    }
    let left = foot(&mut h, &mut c, "left");
    let right = foot(&mut h, &mut c, "right");
    let out = h.state.scene.outputs()[0].clone();
    h.state.apply_layout(&out, halves(), false, left, 0);
    h.state.apply_layout(&out, halves(), false, right, 1);
    h.roundtrip(&mut c);
    assert!(h.state.wm.snap.shares_group(left, right));
    let zones = [frame(&h, left), frame(&h, right)];
    (saved(&mut h), zones)
}

fn snapped_pair() -> (Saved, [Rect; 2]) {
    snapped_pair_and(None)
}

#[test]
fn a_two_tab_group_comes_back_in_its_order_with_its_tab_shown() {
    let (mut h, mut c) = desk();
    let a = foot(&mut h, &mut c, "a");
    let b = foot(&mut h, &mut c, "b");
    h.state.join_tabs(b, a, usize::MAX, false);
    h.state.select_tab(a);
    let s = saved(&mut h);
    assert_eq!(s.1.len(), 1, "one tab group saved");
    drop(h);

    let (mut h, mut c) = restoring(s);
    let b = foot(&mut h, &mut c, "b");
    let a = foot(&mut h, &mut c, "a");
    assert_eq!(
        h.state.tabs.groups.members_of(a),
        [a, b],
        "the saved tab order"
    );
    assert_eq!(h.state.tabs.groups.shown_of(a), a, "the tab that showed");
}

#[test]
fn a_tab_group_whose_other_window_stays_away_is_no_group() {
    let (mut h, mut c) = desk();
    let a = foot(&mut h, &mut c, "a");
    let b = foot(&mut h, &mut c, "b");
    h.state.join_tabs(b, a, usize::MAX, false);
    let (mut h, mut c) = restoring(saved(&mut h));
    let a = foot(&mut h, &mut c, "a");
    assert!(
        h.state.tabs.groups.group_of(a).is_none(),
        "a group of one dissolves"
    );
}

#[test]
fn a_left_right_snap_pair_comes_back_into_its_zones_in_one_group() {
    let (s, zones) = snapped_pair();
    assert_eq!(s.2.len(), 1, "one snap group saved");
    let (mut h, mut c) = restoring(s);
    let right = foot(&mut h, &mut c, "right");
    let left = foot(&mut h, &mut c, "left");
    h.roundtrip(&mut c);
    assert!(h.state.wm.snap.shares_group(left, right), "one group again");
    assert_eq!(frame(&h, left), zones[0], "the left zone");
    assert_eq!(frame(&h, right), zones[1], "the right zone");
}

#[test]
fn snapped_windows_back_under_new_titles_fill_their_zones_in_saved_order() {
    let (s, zones) = snapped_pair();
    let (mut h, mut c) = restoring(s);
    let first = foot(&mut h, &mut c, "~");
    let second = foot(&mut h, &mut c, "~");
    h.roundtrip(&mut c);
    assert!(h.state.wm.snap.shares_group(first, second));
    assert_eq!([frame(&h, first), frame(&h, second)], zones);
}

#[test]
fn a_snapped_window_whose_partner_stays_away_snaps_back_alone() {
    let (s, zones) = snapped_pair();
    let (mut h, mut c) = restoring(s);
    let right = foot(&mut h, &mut c, "right");
    h.roundtrip(&mut c);
    assert!(h.state.wm.snap.group_of(right).is_some());
    assert_eq!(frame(&h, right), zones[1]);
}

#[test]
fn a_window_back_with_the_title_of_one_outside_the_snap_group_takes_no_zone() {
    let (s, zones) = snapped_pair_and(Some("notes"));
    let (mut h, mut c) = restoring(s);
    let notes = foot(&mut h, &mut c, "notes");
    let left = foot(&mut h, &mut c, "~");
    h.roundtrip(&mut c);
    assert!(h.state.wm.snap.group_of(notes).is_none());
    assert_eq!(
        frame(&h, left),
        zones[0],
        "the first free zone by saved order"
    );
}

#[test]
fn untitled_windows_fill_the_tab_group_first_and_then_the_snap_zones() {
    let (mut h, mut c) = desk();
    let a = foot(&mut h, &mut c, "a");
    let b = foot(&mut h, &mut c, "b");
    h.state.join_tabs(b, a, usize::MAX, false);
    let left = foot(&mut h, &mut c, "left");
    let right = foot(&mut h, &mut c, "right");
    let out = h.state.scene.outputs()[0].clone();
    h.state.apply_layout(&out, halves(), false, left, 0);
    h.state.apply_layout(&out, halves(), false, right, 1);
    let s = saved(&mut h);
    assert_eq!((s.1.len(), s.2.len()), (1, 1));
    let (mut h, mut c) = restoring(s);
    let w: Vec<WindowId> = (0..4).map(|_| foot(&mut h, &mut c, "~")).collect();
    h.roundtrip(&mut c);
    let snap = &h.state.wm.snap;
    let tabs = &h.state.tabs.groups;
    assert_eq!(
        tabs.members_of(w[0]),
        [w[0], w[1]],
        "the first two are the tabs"
    );
    assert!(snap.group_of(w[0]).is_none() && snap.group_of(w[1]).is_none());
    assert!(snap.shares_group(w[2], w[3]), "the next two the snap pair");
    assert!(tabs.group_of(w[2]).is_none() && tabs.group_of(w[3]).is_none());
}

#[test]
fn a_snap_group_goes_back_to_its_desktop() {
    let (mut s, _) = snapped_pair();
    s.2[0].desktop = 1;
    let (mut h, mut c) = restoring(s);
    let left = foot(&mut h, &mut c, "left");
    let right = foot(&mut h, &mut c, "right");
    assert_eq!(h.state.win(left).unwrap().desktop, 1);
    assert_eq!(h.state.win(right).unwrap().desktop, 1);
    assert!(h.state.wm.snap.shares_group(left, right));
}

#[test]
fn a_snap_group_whose_monitor_is_gone_opens_its_windows_unsnapped() {
    let (mut s, _) = snapped_pair();
    s.2[0].monitor = "Gone|Monitor|1".into();
    let (mut h, mut c) = restoring(s);
    let left = foot(&mut h, &mut c, "left");
    let right = foot(&mut h, &mut c, "right");
    assert!(h.state.wm.snap.group_of(left).is_none());
    assert!(h.state.wm.snap.group_of(right).is_none());
    assert!(h.state.win(left).unwrap().snapped.is_none());
}
