use crate::snap::{ActiveLayout, ZoneLayout};

use super::groups::{followed, partner_sides};
use super::*;

const WORK: Rect = Rect::new(0, 0, 1920, 1032);

fn all(_: WindowId) -> bool {
    true
}

fn k(output: &str) -> (String, usize) {
    (output.to_string(), 0)
}

fn halves() -> ZoneLayout {
    ZoneLayout::parse("1|1").unwrap()
}

#[test]
fn a_grouped_window_is_tiled_until_it_maximizes() {
    assert!(tiled(false, false, Some(1)));
    assert!(!tiled(true, false, Some(1)), "maximized has its own state");
    assert!(!tiled(false, true, Some(1)), "and fullscreen");
    assert!(!tiled(false, false, None), "out of every group");
}

#[test]
fn groups_record_their_arrangement() {
    let (a, b) = (WindowId::next(), WindowId::next());
    let mut s = Snapping::default();
    let g = s.new_group(a, "DP-1", &halves(), Splits::new(&halves(), WORK), 0);
    s.join_group(a, b, 1);
    assert_eq!(s.group_of(b), Some(g));
    let r: Vec<_> = s.recents.iter().cloned().collect();
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].members, vec![(0, a), (1, b)]);
    s.active
        .insert(k("DP-1"), ActiveLayout::new(halves(), true, WORK));
    s.key_zone(b, "DP-1", WORK, 0, false, all);
    assert_eq!(
        s.group_of(b),
        Some(g),
        "a step inside its own group keeps it"
    );
}

#[test]
fn a_drop_into_the_active_layout_joins_its_group() {
    let (a, b, c, d) = (
        WindowId::next(),
        WindowId::next(),
        WindowId::next(),
        WindowId::next(),
    );
    let mut s = Snapping::default();
    let quarters = ZoneLayout::parse("1:2|1:2").unwrap();
    s.apply(a, "DP-1", quarters.clone(), true, true, WORK, 0, all);
    let g = s.group_of(a);
    s.apply(b, "DP-1", quarters.clone(), true, true, WORK, 2, all);
    assert_eq!(s.group_of(b), g, "the quarters' group");
    assert_eq!(s.free_zones(b, 4, |_| true), vec![1, 3]);
    s.apply(c, "DP-1", quarters.clone(), true, true, WORK, 2, all);
    assert_eq!(s.group_of(c), g);
    assert_eq!(s.group_of(b), None, "covered, out of the group");
    s.apply(d, "DP-1", halves(), true, true, WORK, 0, all);
    assert_ne!(s.group_of(d), g, "another layout starts a group");
    s.apply(b, "DP-1", halves(), true, false, WORK, 1, all);
    assert_ne!(
        s.group_of(b),
        s.group_of(d),
        "a suggestion's pick starts its own"
    );
}

#[test]
fn a_layout_with_no_member_on_screen_is_not_active() {
    let (a, b) = (WindowId::next(), WindowId::next());
    let mut s = Snapping::default();
    let quarters = ZoneLayout::parse("1:2|1:2").unwrap();
    s.apply(a, "DP-1", quarters.clone(), true, true, WORK, 0, all);
    // `a` minimized, or another desktop shown.
    let away = |m: WindowId| m != a;
    let (layout, explicit, _) = s.active_target("DP-1", WORK, &away);
    assert_eq!(layout, halves(), "the halves, as with no layout");
    assert!(!explicit, "no zone targets");
    assert_eq!(
        s.outlined_target(
            "DP-1",
            WORK,
            crate::snap::BAND,
            true,
            None,
            crate::snap::EVERY_SIDE,
            &mut true,
            500,
            300,
            &away
        ),
        None,
        "a release off the edges drops nowhere"
    );
    assert!(
        (s.outlined_target(
            "DP-1",
            WORK,
            crate::snap::BAND,
            true,
            None,
            crate::snap::EVERY_SIDE,
            &mut true,
            500,
            300,
            &all
        ))
        .is_some(),
        "with `a` on screen it drops into a quarter"
    );
    s.apply(b, "DP-1", quarters, true, true, WORK, 1, away);
    assert_ne!(s.group_of(b), s.group_of(a), "`b` joins no hidden group");
}

#[test]
fn a_member_moved_to_another_desktop_leaves_its_group() {
    let (a, b, c) = (WindowId::next(), WindowId::next(), WindowId::next());
    let mut s = Snapping::default();
    let g = s.new_group(a, "DP-1", &halves(), Splits::new(&halves(), WORK), 0);
    s.join_group(a, b, 1);
    s.join_group(a, c, 1);
    let on = |b_at: Option<usize>| move |w: WindowId| if w == b { b_at } else { Some(0) };
    s.settle_desktops(on(None));
    assert_eq!(s.group_of(b), Some(g), "on every desktop: stays");
    s.settle_desktops(on(Some(0)));
    assert_eq!(s.group_of(b), Some(g), "still on the group's desktop");
    s.settle_desktops(on(Some(1)));
    assert_eq!(s.group_of(b), None, "moved to the second desktop");
    assert_eq!(s.group_of(a), Some(g));
    assert_eq!(s.group_of(c), Some(g));
    s.settle_desktops(|_| Some(1));
    assert_eq!(s.group_of(a), None);
    assert!(
        !s.active.contains_key(&k("DP-1")),
        "the layout went with the last member"
    );
}

#[test]
fn a_group_on_every_desktop_moves_to_the_desktop_shown() {
    let (a, b, c) = (WindowId::next(), WindowId::next(), WindowId::next());
    let mut s = Snapping::default();
    let thirds = ZoneLayout::parse("1|1|1").unwrap();
    s.apply(a, "DP-1", halves(), true, true, WORK, 0, all);
    s.apply(b, "DP-1", halves(), true, true, WORK, 1, all);
    let g = s.group_of(a).expect("snapped");
    s.desktop = 2;
    s.apply(c, "DP-1", thirds.clone(), true, true, WORK, 0, all);
    let sticky = |w: WindowId| (w == c).then_some(2);
    s.desktop = 1;
    s.settle_desktops(sticky);
    assert_eq!((s.group_of(a), s.group_of(b)), (Some(g), Some(g)));
    let on_1 = |m: WindowId| m != c;
    assert_eq!(s.active_target("DP-1", WORK, &on_1).0, halves());
    assert_eq!(
        s.landing("DP-1", &halves(), true, WORK, &on_1).0,
        Some(g),
        "a drop on the second desktop joins it"
    );
    assert!(
        !s.active.contains_key(&k("DP-1")),
        "the first desktop's layout went with it"
    );
    s.desktop = 2;
    s.settle_desktops(sticky);
    assert_eq!(
        s.active_target("DP-1", WORK, &all).0,
        thirds,
        "the third desktop keeps its own layout"
    );
    assert_eq!(s.group_of(a), Some(g));
}

#[test]
fn a_group_with_a_member_on_one_desktop_stays_there() {
    let (a, b) = (WindowId::next(), WindowId::next());
    let mut s = Snapping::default();
    s.apply(a, "DP-1", halves(), true, true, WORK, 0, all);
    s.apply(b, "DP-1", halves(), true, true, WORK, 1, all);
    let g = s.group_of(a).expect("snapped");
    s.desktop = 1;
    s.settle_desktops(|w| (w == b).then_some(0));
    assert_eq!((s.group_of(a), s.group_of(b)), (Some(g), Some(g)));
    let only_a = |m: WindowId| m == a;
    assert_eq!(
        s.landing("DP-1", &halves(), true, WORK, &only_a).0,
        None,
        "`b` is not on the second desktop: the group does not count there"
    );
    assert!(s.active.contains_key(&k("DP-1")));
}

#[test]
fn each_desktop_keeps_its_own_layout() {
    let (a, b) = (WindowId::next(), WindowId::next());
    let mut s = Snapping::default();
    let quarters = ZoneLayout::parse("1:2|1:2").unwrap();
    let thirds = ZoneLayout::parse("1|1|1").unwrap();
    s.apply(a, "DP-1", quarters.clone(), true, true, WORK, 0, all);
    // The second desktop shown: `a` is off screen.
    s.desktop = 1;
    let away = |m: WindowId| m != a;
    s.apply(b, "DP-1", thirds.clone(), true, true, WORK, 2, away);
    assert_eq!(s.active_target("DP-1", WORK, &away).0, thirds);
    s.desktop = 0;
    let back = |m: WindowId| m != b;
    let (layout, explicit, _) = s.active_target("DP-1", WORK, &back);
    assert_eq!(layout, quarters, "the first desktop's pick stayed");
    assert!(explicit);
    s.forget_window(b);
    assert!(
        !s.active.contains_key(&("DP-1".into(), 1)),
        "the second desktop's layout went with its last window"
    );
    assert!(s.active.contains_key(&k("DP-1")));
}

#[test]
fn a_suggestions_pick_brings_back_only_members_that_can_fill_a_zone() {
    let (a, b, c, d) = (
        WindowId::next(),
        WindowId::next(),
        WindowId::next(),
        WindowId::next(),
    );
    let mut s = Snapping::default();
    let thirds = ZoneLayout::parse("1|1|1").unwrap();
    let sug = Suggestion {
        layout: thirds,
        // `c` claims zone 1 first; `d` cannot fill one.
        filled: vec![(1, c), (1, b), (2, d), (0, a)],
    };
    let fills = |m: WindowId| m != d;
    let SuggestionPick { zones, members } =
        (s.apply_suggestion(a, "DP-1", &sug, WORK, 0, all, fills)).unwrap();
    assert_eq!(members, vec![(c, zones[1])], "one member per zone");
    assert_eq!(s.group_of(c), s.group_of(a));
    assert_eq!(s.group_of(b), None);
    assert_eq!(s.group_of(d), None, "it claims no zone");
    assert_eq!(s.free_zones(a, 3, all), vec![2]);
    assert!(
        (s.apply_suggestion(a, "DP-1", &sug, WORK, 5, all, fills)).is_none(),
        "a zone the layout lacks"
    );
}

#[test]
fn a_drop_joins_the_group_cut_as_the_active_layout() {
    let (a, b, c) = (WindowId::next(), WindowId::next(), WindowId::next());
    let mut s = Snapping::default();
    let quarters = ZoneLayout::parse("1:2|1:2").unwrap();
    s.apply(a, "DP-1", quarters.clone(), true, true, WORK, 0, all);
    // A suggestion's pick of the same layout starts a newer group.
    s.apply(b, "DP-1", quarters.clone(), true, false, WORK, 1, all);
    // The older group's border moves, and the active layout's with it.
    s.move_splits(a, WORK, &[(true, 960, 1100)]);
    let outline = s.active[&k("DP-1")].clone().zones(WORK).to_vec();
    assert_eq!(outline[3].x, 1100);
    let landed = s.apply(c, "DP-1", quarters, true, true, WORK, 3, all);
    assert_eq!(landed, Some(outline), "it lands where the outline showed");
    assert_eq!(s.group_of(c), s.group_of(a), "the group cut that way");
}

#[test]
fn a_corner_drop_outlines_where_it_lands_with_two_groups() {
    let (a, b, c) = (WindowId::next(), WindowId::next(), WindowId::next());
    let mut s = Snapping::default();
    let quarters = ZoneLayout::parse("1:2|1:2").unwrap();
    s.apply(a, "DP-1", quarters.clone(), true, true, WORK, 1, all);
    // A suggestion's pick of the same layout starts a newer group.
    s.apply(b, "DP-1", quarters.clone(), true, false, WORK, 0, all);
    // The older group's border moves, and the active layout's with it.
    s.move_splits(a, WORK, &[(true, 960, 1100)]);
    // A release in the top-right corner.
    let (target, outline) = s
        .outlined_target(
            "DP-1",
            WORK,
            crate::snap::BAND,
            true,
            None,
            crate::snap::EVERY_SIDE,
            &mut true,
            1919,
            0,
            &all,
        )
        .unwrap();
    assert_eq!(target, Target::Corner { quarter: 2 });
    assert_eq!(
        outline,
        Rect::new(1100, 0, 820, 516),
        "the moved group's cut"
    );
    let (layout, _, zone) = s.target_layout("DP-1", WORK, target, &all).unwrap();
    let (joins, _) = s.landing("DP-1", &layout, true, WORK, &all);
    assert_eq!(joins, s.group_of(a));
    let landed = s.apply(c, "DP-1", layout, true, true, WORK, zone, all);
    assert_eq!(
        landed.map(|z| z[2]),
        Some(outline),
        "it lands on the outline"
    );
    assert_eq!(s.group_of(c), joins, "in the group the outline showed");
}

#[test]
fn a_suggestion_tile_outlines_where_its_pick_lands() {
    let (a, b, c) = (WindowId::next(), WindowId::next(), WindowId::next());
    let mut s = Snapping::default();
    let quarters = ZoneLayout::parse("1:2|1:2").unwrap();
    s.apply(a, "DP-1", quarters.clone(), true, true, WORK, 1, all);
    s.apply(b, "DP-1", quarters.clone(), true, false, WORK, 0, all);
    s.move_splits(a, WORK, &[(true, 960, 1100)]);
    let (joins, tile) = s.landing("DP-1", &quarters, false, WORK, &all);
    assert_eq!(joins, None, "a suggestion starts its own group");
    assert_eq!(tile[2].x, 1100, "cut as the active layout is");
    let landed = s.apply(c, "DP-1", quarters, true, false, WORK, 2, all);
    assert_eq!(landed, Some(tile));
}

#[test]
fn a_key_step_joins_the_group_a_drop_joins() {
    let (a, b) = (WindowId::next(), WindowId::next());
    let mut s = Snapping::default();
    let quarters = ZoneLayout::parse("1:2|1:2").unwrap();
    s.apply(a, "DP-1", quarters.clone(), true, true, WORK, 0, all);
    s.apply(b, "DP-1", quarters, true, false, WORK, 1, all);
    s.move_splits(a, WORK, &[(true, 960, 1100)]);
    let outline = s.active[&k("DP-1")].clone().zones(WORK).to_vec();
    // b steps out of its own, newer group into a's, cut as the
    // active layout is.
    assert_eq!(
        s.key_zone(b, "DP-1", WORK, 2, false, all).map(|l| l.zones),
        Some(outline)
    );
    assert_eq!(s.group_of(b), s.group_of(a));
    assert_eq!(
        s.key_zone(b, "DP-1", WORK, 4, false, all),
        None,
        "no fifth zone"
    );
}

#[test]
fn super_up_and_down_step_by_group_zone_after_a_split_move() {
    let (x, y, z) = (WindowId::next(), WindowId::next(), WindowId::next());
    let mut s = Snapping::default();
    let half_quarters = ZoneLayout::parse("1|1:2").unwrap();
    for (w, zone) in [(x, 0), (y, 1), (z, 2)] {
        s.apply(
            w,
            "DP-1",
            half_quarters.clone(),
            true,
            true,
            WORK,
            zone,
            all,
        );
    }
    s.move_splits(y, WORK, &[(true, 960, 1150)]);
    let (_, _, zones) = s.active_target("DP-1", WORK, &all);
    // The clients' own sizes leave the frames short of the moved split.
    let y_frame = Rect::new(1139, 0, 781, 516);
    let z_frame = Rect::new(1139, 516, 781, 516);
    assert_eq!(crate::snap::zone_of(&zones, y_frame), Some(1));
    let step = |w: WindowId, frame: Rect, dir: Dir| {
        let at = s.member_zone(w, "DP-1", &all);
        let cy = frame.y + frame.height / 2;
        crate::snap::key_step(WORK, &zones, at, false, false, cy, dir)
    };
    assert_eq!(step(y, y_frame, Dir::Down), KeyStep::Zone(2));
    assert_eq!(step(z, z_frame, Dir::Up), KeyStep::Zone(1));
}

#[test]
fn a_super_arrow_step_swaps_with_the_member_holding_the_zone() {
    let (x, y, z) = (WindowId::next(), WindowId::next(), WindowId::next());
    let mut s = Snapping::default();
    let half_quarters = ZoneLayout::parse("1|1:2").unwrap();
    for (w, zone) in [(x, 0), (y, 1), (z, 2)] {
        s.apply(
            w,
            "DP-1",
            half_quarters.clone(),
            true,
            true,
            WORK,
            zone,
            all,
        );
    }
    s.move_splits(y, WORK, &[(true, 960, 1150)]);
    let g = s.group_of(x);
    let zones = |s: &Snapping| [x, y, z].map(|w| s.member_zone(w, "DP-1", &all));
    let step = |s: &mut Snapping, w: WindowId, dir: Dir| {
        let (_, _, rects) = s.active_target("DP-1", WORK, &all);
        let at = s.member_zone(w, "DP-1", &all);
        let KeyStep::Zone(i) = crate::snap::key_step(WORK, &rects, at, false, true, 0, dir) else {
            panic!("{dir:?} steps into a zone");
        };
        s.key_zone(w, "DP-1", WORK, i, false, all).unwrap().swapped
    };
    assert_eq!(step(&mut s, y, Dir::Down), Some((z, 1)), "z takes y's zone");
    assert_eq!(zones(&s), [Some(0), Some(2), Some(1)]);
    assert_eq!(step(&mut s, y, Dir::Up), Some((z, 2)));
    assert_eq!(zones(&s), [Some(0), Some(1), Some(2)]);
    assert_eq!(step(&mut s, z, Dir::Up), Some((y, 2)), "z moves up, y down");
    assert_eq!(zones(&s), [Some(0), Some(2), Some(1)]);
    assert!(
        [x, y, z].iter().all(|&w| s.group_of(w) == g),
        "all stay grouped"
    );
}

#[test]
fn super_left_and_right_swap_halves() {
    let (a, b) = (WindowId::next(), WindowId::next());
    let mut s = Snapping::default();
    s.apply(a, "DP-1", halves(), true, true, WORK, 0, all);
    s.apply(b, "DP-1", halves(), true, true, WORK, 1, all);
    let l = s.key_zone(a, "DP-1", WORK, 1, false, all).unwrap();
    assert_eq!(l.swapped, Some((b, 0)), "Right: a and b trade halves");
    assert_eq!(s.member_zone(b, "DP-1", &all), Some(0));
    let l = s.key_zone(a, "DP-1", WORK, 0, false, all).unwrap();
    assert_eq!(l.swapped, Some((b, 1)), "Left: back again");
    assert_eq!(s.group_of(a), s.group_of(b));
}

#[test]
fn a_super_arrow_snap_leaves_free_only_the_zones_no_member_holds() {
    let (a, b) = (WindowId::next(), WindowId::next());
    let mut s = Snapping::default();
    let l = s.key_zone(a, "DP-1", WORK, 0, false, all).unwrap();
    assert_eq!(s.free_zones(a, l.zones.len(), all), vec![1]);
    let l = s.key_zone(b, "DP-1", WORK, 1, false, all).unwrap();
    assert_eq!(s.free_zones(b, l.zones.len(), all), vec![]);
    let l = s.key_zone(a, "DP-1", WORK, 1, false, all).unwrap();
    assert_eq!(l.swapped, Some((b, 0)));
    assert_eq!(s.free_zones(a, l.zones.len(), all), vec![], "a swap");
}

#[test]
fn a_step_from_outside_a_zone_covers_the_occupant() {
    let (a, b, c) = (WindowId::next(), WindowId::next(), WindowId::next());
    let mut s = Snapping::default();
    s.apply(a, "DP-1", halves(), true, true, WORK, 0, all);
    s.apply(b, "DP-1", halves(), true, true, WORK, 1, all);
    let l = s.key_zone(c, "DP-1", WORK, 1, false, all).unwrap();
    assert_eq!(l.swapped, None, "a free window");
    assert_eq!(s.group_of(b), None, "b leaves, covered");
    let l = s.key_zone(a, "DP-1", WORK, 1, true, all).unwrap();
    assert_eq!(l.swapped, None, "a maximized window");
    assert_eq!(s.group_of(c), None, "c leaves, covered");
}

#[test]
fn a_window_outside_the_active_layouts_groups_has_no_member_zone() {
    let (a, b) = (WindowId::next(), WindowId::next());
    let mut s = Snapping::default();
    let thirds = ZoneLayout::parse("1|1|1").unwrap();
    s.apply(a, "DP-1", halves(), true, true, WORK, 1, all);
    assert_eq!(s.member_zone(a, "DP-1", &all), Some(1));
    assert_eq!(s.member_zone(a, "DP-2", &all), None, "another output");
    assert_eq!(s.member_zone(b, "DP-1", &all), None, "in no group");
    s.apply(b, "DP-1", thirds, true, true, WORK, 2, all);
    assert_eq!(s.member_zone(b, "DP-1", &all), Some(2));
    assert_eq!(
        s.member_zone(a, "DP-1", &all),
        None,
        "its halves are no longer the active layout"
    );
}

#[test]
fn a_drop_covers_only_an_occupant_on_screen() {
    let (a, b, c, d) = (
        WindowId::next(),
        WindowId::next(),
        WindowId::next(),
        WindowId::next(),
    );
    let mut s = Snapping::default();
    s.apply(a, "DP-1", halves(), true, true, WORK, 0, all);
    s.join_group(a, b, 1);
    // b is minimized or on another desktop: it keeps its zone.
    let shown = |m: WindowId| m != b;
    s.apply(c, "DP-1", halves(), true, true, WORK, 1, shown);
    assert_eq!(s.group_of(b), s.group_of(a));
    assert_eq!(s.group_of(c), s.group_of(a));
    s.apply(d, "DP-1", halves(), true, true, WORK, 0, shown);
    assert_eq!(s.group_of(a), None, "a on screen is covered");
    assert_eq!(s.group_of(d), s.group_of(b));
}

#[test]
fn a_drop_into_a_layout_on_another_output_starts_a_group_there() {
    let (a, b) = (WindowId::next(), WindowId::next());
    let mut s = Snapping::default();
    s.apply(a, "DP-1", halves(), true, true, WORK, 0, all);
    s.join_group(a, b, 1);
    let first = s.group_of(a);
    s.apply(b, "DP-2", halves(), false, true, WORK, 0, all);
    let second = s.group_of(b);
    assert!(second.is_some() && second != first, "not DP-1's group");
    assert_eq!(s.member_zone(b, "DP-2", &all), Some(0));
    assert_eq!(s.group_of(a), first, "DP-1 keeps its group");
    assert!(s.active.contains_key(&k("DP-1")));
    s.apply(a, "DP-2", halves(), false, true, WORK, 1, all);
    assert_eq!(s.group_of(a), second, "joins the group on DP-2");
    assert!(
        !s.active.contains_key(&k("DP-1")),
        "DP-1's layout went with its last window"
    );
}

#[test]
fn a_cancelled_move_puts_the_window_back_in_its_group() {
    let (a, b, c) = (WindowId::next(), WindowId::next(), WindowId::next());
    let mut s = Snapping::default();
    s.apply(a, "DP-1", halves(), true, true, WORK, 0, all);
    s.join_group(a, b, 1);
    let g = s.group_of(b);
    s.begin_move(b);
    s.leave_group(b);
    s.cancel_move(b);
    assert_eq!(s.group_of(b), g);
    assert_eq!(s.free_zones(a, 2, |_| true), Vec::<usize>::new());
    // Alone in its group, the group and the active layout went with
    // it; both come back.
    let thirds = ZoneLayout::parse("1|1|1").unwrap();
    s.apply(c, "DP-2", thirds.clone(), true, true, WORK, 2, all);
    let g = s.group_of(c);
    s.begin_move(c);
    s.leave_group(c);
    assert!(!s.active.contains_key(&k("DP-2")));
    s.cancel_move(c);
    assert_eq!(s.group_of(c), g);
    assert_eq!(s.active.get(&k("DP-2")).map(|l| &l.layout), Some(&thirds));
    assert_eq!(
        s.recents.iter().next().map(|r| r.members.clone()),
        Some(vec![(2, c)])
    );
    s.cancel_move(c);
    assert_eq!(s.group_of(c), g, "a second cancel changes nothing");
}

#[test]
fn a_released_or_closed_window_has_no_move_to_cancel() {
    let (a, b) = (WindowId::next(), WindowId::next());
    let mut s = Snapping::default();
    s.apply(a, "DP-1", halves(), true, true, WORK, 0, all);
    s.join_group(a, b, 1);
    s.begin_move(b);
    s.end_move();
    s.leave_group(b);
    s.cancel_move(b);
    assert_eq!(s.group_of(b), None, "released: nothing to put back");
    s.begin_move(a);
    s.forget_window(a);
    s.cancel_move(a);
    assert_eq!(s.group_of(a), None, "closed: not brought back");
    assert!(!s.active.contains_key(&k("DP-1")));
}

#[test]
fn a_follower_riding_two_sides_takes_both_from_its_first_frame() {
    let (tr, bl) = (WindowId::next(), WindowId::next());
    let tr_start = Rect::new(960, 0, 960, 516);
    let partners = [
        Partner {
            window: tr,
            side: Edges::RIGHT,
            edge: Edges::LEFT,
            start: tr_start,
        },
        Partner {
            window: bl,
            side: Edges::BOTTOM,
            edge: Edges::TOP,
            start: Rect::new(0, 516, 960, 516),
        },
        // Picked second, after the first side had moved it.
        Partner {
            window: tr,
            side: Edges::BOTTOM,
            edge: Edges::BOTTOM,
            start: Rect::new(1000, 0, 920, 516),
        },
    ];
    let by = partner_sides(&partners);
    assert_eq!(by.len(), 2, "each follower once");
    let (w, start, sides) = &by[0];
    assert_eq!((*w, *start), (tr, tr_start), "its first entry's frame");
    let sized = Rect::new(0, 0, 1040, 600);
    assert_eq!(
        followed(*start, sides, sized),
        (Rect::new(1040, 0, 880, 600), Edges::LEFT | Edges::BOTTOM)
    );
    assert_eq!(
        followed(by[1].1, &by[1].2, sized),
        (Rect::new(0, 600, 960, 432), Edges::TOP)
    );
}

#[test]
fn a_size_takes_its_window_off_its_zone_only_by_a_side_no_member_followed() {
    let (a, b) = (WindowId::next(), WindowId::next());
    let (left, right) = (Rect::new(0, 0, 960, 1032), Rect::new(960, 0, 960, 1032));
    let mut s = Snapping::default();
    s.new_group(a, "DP-1", &halves(), Splits::new(&halves(), WORK), 0);
    s.join_group(a, b, 1);
    let size = |s: &mut Snapping, edges: Edges, last: Rect, followed: bool| {
        s.resize = Some((a, left, edges));
        s.last = Some(last);
        s.partners = (followed.then_some(Partner {
            window: b,
            side: Edges::RIGHT,
            edge: Edges::LEFT,
            start: right,
        }))
        .into_iter()
        .collect();
        s.end_resize()
    };
    let wider = Rect::new(0, 0, 1040, 1032);
    let (followers, end) = size(&mut s, Edges::RIGHT, wider, true);
    assert_eq!(followers, vec![b]);
    let end = end.expect("the size moved its window");
    assert_eq!(end.moves, vec![(true, 960, 1040)]);
    assert!(
        !end.off_zone,
        "the shared side moved: both stay in their zones"
    );
    assert_eq!(
        (s.resize, s.last, s.partners.len()),
        (None, None, 0),
        "nothing of the size is left"
    );
    let shorter = Rect::new(0, 100, 1040, 932);
    let end = size(&mut s, Edges::RIGHT | Edges::TOP, shorter, true).1;
    assert!(end.expect("moved").off_zone, "the top side had no follower");
    let end = size(&mut s, Edges::RIGHT, wider, false).1;
    assert!(
        end.expect("moved").off_zone,
        "an ungrouped window's side moved"
    );
    let end = size(&mut s, Edges::RIGHT, left, false).1;
    assert!(!end.expect("sized").off_zone, "a size that moved nothing");
    s.resize = None;
    assert_eq!(s.end_resize(), (Vec::new(), None), "no side picked yet");
}

#[test]
fn a_window_shares_a_group_only_with_its_members() {
    let (a, b, c, d) = (
        WindowId::next(),
        WindowId::next(),
        WindowId::next(),
        WindowId::next(),
    );
    let mut s = Snapping::default();
    s.new_group(a, "DP-1", &halves(), Splits::new(&halves(), WORK), 0);
    s.join_group(a, b, 1);
    s.new_group(
        c,
        "DP-1",
        &ZoneLayout::parse("1|1|1").unwrap(),
        Splits::new(&ZoneLayout::parse("1|1|1").unwrap(), WORK),
        0,
    );
    assert!(s.shares_group(a, a));
    assert!(s.shares_group(a, b), "a member");
    assert!(!s.shares_group(a, c), "another group's");
    assert!(!s.shares_group(a, d), "in no group");
    assert!(!s.shares_group(d, WindowId::next()), "neither grouped");
}

#[test]
fn free_zones_count_only_the_groups_own_members() {
    let (a, b, old0, old1, shut) = (
        WindowId::next(),
        WindowId::next(),
        WindowId::next(),
        WindowId::next(),
        WindowId::next(),
    );
    let mut s = Snapping::default();
    // Halves from before sit on the quarters' top corners.
    s.new_group(old0, "DP-1", &halves(), Splits::new(&halves(), WORK), 0);
    s.join_group(old0, old1, 1);
    let quarters = ZoneLayout::parse("1:2|1:2").unwrap();
    s.new_group(a, "DP-1", &quarters, Splits::new(&quarters, WORK), 0);
    let live = |m: WindowId| m != shut;
    assert_eq!(
        s.free_zones(a, 4, live),
        vec![1, 2, 3],
        "the halves fill none"
    );
    s.join_group(a, b, 3);
    s.join_group(a, shut, 1);
    assert_eq!(
        s.free_zones(a, 4, live),
        vec![1, 2],
        "a member not on screen fills nothing"
    );
    assert_eq!(s.free_zones(WindowId::next(), 2, live), vec![0, 1]);
}

#[test]
fn the_active_layout_goes_with_the_last_grouped_window() {
    let (a, b) = (WindowId::next(), WindowId::next());
    let mut s = Snapping::default();
    let work = Rect::new(0, 0, 1920, 1032);
    s.active
        .insert(k("DP-1"), ActiveLayout::new(halves(), true, work));
    s.new_group(a, "DP-1", &halves(), Splits::new(&halves(), WORK), 0);
    s.join_group(a, b, 1);
    s.leave_group(a);
    assert!(s.active.contains_key(&k("DP-1")), "b still holds a zone");
    s.forget_window(b);
    assert!(!s.active.contains_key(&k("DP-1")));
    assert_eq!(
        s.recents.iter().count(),
        0,
        "the group's entry emptied with it"
    );
}

#[test]
fn a_new_layout_stays_when_its_window_was_the_only_one_grouped() {
    let a = WindowId::next();
    let mut s = Snapping::default();
    let work = Rect::new(0, 0, 1920, 1032);
    let thirds = ZoneLayout::parse("1|1|1").unwrap();
    s.apply(a, "DP-1", halves(), true, false, work, 0, all);
    let zones = s.apply(a, "DP-1", thirds.clone(), true, false, work, 2, all);
    assert_eq!(zones, Some(thirds.zones(work)));
    assert_eq!(s.active.get(&k("DP-1")).map(|l| &l.layout), Some(&thirds));
    assert_eq!(s.groups.len(), 1);
    assert_eq!(s.groups[0].layout, thirds);
    assert_eq!(s.groups[0].members, vec![(2, a)]);
}

#[test]
fn outputs_gone_drop_their_layouts() {
    let a = WindowId::next();
    let mut s = Snapping::default();
    let work = Rect::new(0, 0, 1920, 1032);
    s.active
        .insert(k("DP-1"), ActiveLayout::new(halves(), true, work));
    s.active
        .insert(k("DP-2"), ActiveLayout::new(halves(), true, work));
    s.new_group(a, "DP-1", &halves(), Splits::new(&halves(), WORK), 0);
    s.new_group(
        WindowId::next(),
        "DP-2",
        &halves(),
        Splits::new(&halves(), WORK),
        0,
    );
    s.outputs_gone(&["DP-1".to_string()]);
    assert!(s.active.contains_key(&k("DP-1")));
    assert!(!s.active.contains_key(&k("DP-2")));
}

#[test]
fn a_refit_moves_the_groups_on_every_layout_of_the_output() {
    let (a, b, c) = (WindowId::next(), WindowId::next(), WindowId::next());
    let mut s = Snapping::default();
    let thirds = ZoneLayout::parse("1|1|1").unwrap();
    let old = Rect::new(0, 0, 1920, 1032);
    let new = Rect::new(0, 48, 1920, 1032);
    s.apply(a, "DP-1", halves(), true, false, old, 0, all);
    s.join_group(a, b, 1);
    s.apply(c, "DP-1", thirds.clone(), true, false, old, 2, all);
    let mut moves = s.refit("DP-1", new);
    moves.sort_by_key(|m| m.1.x);
    let (h, t) = (halves().zones(new), thirds.zones(new));
    assert_eq!(moves, vec![(a, h[0]), (b, h[1]), (c, t[2])]);
    assert!(s.refit("DP-1", new).is_empty(), "laid on the new area once");
}

#[test]
fn a_group_keeps_its_own_splits_through_a_refit() {
    let (a, b, c, d) = (
        WindowId::next(),
        WindowId::next(),
        WindowId::next(),
        WindowId::next(),
    );
    let mut s = Snapping::default();
    let thirds = ZoneLayout::parse("1|1|1").unwrap();
    s.apply(a, "DP-1", halves(), true, false, WORK, 0, all);
    s.join_group(a, b, 1);
    s.move_splits(a, WORK, &[(true, 960, 1100)]);
    assert_eq!(
        s.active[&k("DP-1")].clone().zones(WORK)[0].right(),
        1100,
        "the active layout's split moves with its group"
    );
    // Thirds become active; the halves keep their moved split.
    s.apply(c, "DP-1", thirds.clone(), true, false, WORK, 0, all);
    s.join_group(c, d, 1);
    s.move_splits(c, WORK, &[(true, 640, 700)]);
    s.move_splits(a, WORK, &[(true, 1100, 1200)]);
    let mut active = s.active[&k("DP-1")].clone();
    assert_eq!(
        active.zones(WORK)[1],
        Rect::new(700, 0, 580, 1032),
        "the halves' resize leaves the thirds' split"
    );
    // The bar moves to the left side, 60 wide: 1200 of 1920 is 1163
    // of 1860 (1162.5 rounded up) from x 60, 700 is 678, 1280 is 1240.
    let moved = Rect::new(60, 0, 1860, 1032);
    let mut moves = s.refit("DP-1", moved);
    moves.sort_by_key(|m| (m.1.x, m.1.width));
    assert_eq!(
        moves,
        vec![
            (c, Rect::new(60, 0, 678, 1032)),
            (a, Rect::new(60, 0, 1163, 1032)),
            (d, Rect::new(738, 0, 562, 1032)),
            (b, Rect::new(1223, 0, 697, 1032)),
        ]
    );
    assert!(
        s.refit("DP-1", moved).is_empty(),
        "laid on the new area once"
    );
    let back = s.refit("DP-1", WORK);
    assert!(
        back.contains(&(b, Rect::new(1200, 0, 720, 1032))),
        "back on the first area, the split is where the drag left it"
    );
}

#[test]
fn a_size_cut_short_by_a_snap_leaves_the_next_press_on_an_edge_in_its_zone() {
    use crate::harness::{Harness, LEFT};
    const KEY_LEFT: u32 = 105;
    const KEY_RIGHT: u32 = 106;
    const KEY_LEFTMETA: u32 = 125;
    let mut h = Harness::new();
    h.output("TEST-1", 1024, 768);
    let mut c = h.connect();
    let t = h.map_toplevel(&mut c, "test.app", 300, 200);
    let snap = |h: &mut Harness, c: &mut crate::harness::TestClient, key| {
        h.key(KEY_LEFTMETA, true);
        h.key(key, true);
        h.key(key, false);
        h.key(KEY_LEFTMETA, false);
        h.roundtrip(c);
    };
    snap(&mut h, &mut c, KEY_LEFT);
    let r = h.state.win(t.id).expect("window").frame_rect();
    assert_eq!((r.x, r.width), (0, 512), "the left half");
    let mid = f64::from(r.y + r.height / 2);
    h.pointer_to(f64::from(r.right()) - 1.0, mid);
    h.button(LEFT, true);
    assert!(
        h.state.interaction.is_resize_of(t.id),
        "the right edge sizes"
    );
    h.pointer_to(f64::from(r.right()) + 59.0, mid);
    h.roundtrip(&mut c);
    snap(&mut h, &mut c, KEY_RIGHT);
    h.button(LEFT, false);
    let r = h.state.win(t.id).expect("window").frame_rect();
    assert_eq!((r.x, r.width), (512, 512), "the right half");
    let in_zone = |h: &Harness| {
        h.state.wm.snap.group_of(t.id).is_some()
            && h.state.win(t.id).is_some_and(|w| w.snapped.is_some())
    };
    assert!(in_zone(&h), "snapped into the right half");
    h.pointer_to(f64::from(r.x), mid);
    h.button(LEFT, true);
    assert!(
        h.state.interaction.is_resize_of(t.id),
        "the left edge sizes"
    );
    h.button(LEFT, false);
    h.roundtrip(&mut c);
    assert!(in_zone(&h), "a press without a move keeps the zone");
}

const KEY_LEFT: u32 = 105;
const KEY_RIGHT: u32 = 106;
const KEY_UP: u32 = 103;
const KEY_DOWN: u32 = 108;
const KEY_LEFTMETA: u32 = 125;

/// Super and the arrow `key`, then the client's answer.
fn super_arrow(h: &mut crate::harness::Harness, c: &mut crate::harness::TestClient, key: u32) {
    h.key(KEY_LEFTMETA, true);
    h.key(key, true);
    h.key(key, false);
    h.key(KEY_LEFTMETA, false);
    h.roundtrip(c);
}

#[test]
fn a_snapped_window_held_within_the_slop_keeps_its_zone_and_group() {
    use crate::harness::{Harness, LEFT};
    let mut h = Harness::new();
    h.output("TEST-1", 1024, 768);
    let mut c = h.connect();
    let t = h.map_toplevel(&mut c, "test.app", 300, 200);
    super_arrow(&mut h, &mut c, KEY_LEFT);
    let zone = h.state.win(t.id).expect("window").frame_rect();
    assert_eq!((zone.x, zone.width), (0, 512), "the left half");
    let (x, y) = (f64::from(zone.x + 60), f64::from(zone.y + 10));
    h.pointer_to(x, y);
    h.button(LEFT, true);
    assert!(
        matches!(h.state.interaction, Interaction::Move { window, .. } if window == t.id),
        "the caption moves"
    );
    let slop = f64::from(crate::desktop::DRAG_PX) - 1.0;
    h.pointer_to(x + slop, y + slop);
    h.roundtrip(&mut c);
    let w = h.state.win(t.id).expect("window");
    assert_eq!(w.frame_rect(), zone, "a jitter moves nothing");
    assert!(w.snapped.is_some(), "still snapped");
    assert!(h.state.wm.snap.group_of(t.id).is_some(), "still grouped");
    h.pointer_to(x + 40.0, y + 40.0);
    h.roundtrip(&mut c);
    assert!(
        h.state.wm.snap.group_of(t.id).is_none(),
        "a real move leaves the group"
    );
    assert_ne!(
        h.state.win(t.id).expect("window").frame_rect().width,
        512,
        "and takes back the size from before the snap"
    );
    h.button(LEFT, false);
}

#[test]
fn super_arrows_step_through_the_zones_to_the_next_output_and_back_out() {
    let mut h = crate::harness::Harness::new();
    h.output("TEST-1", 1024, 768);
    h.output("TEST-2", 1024, 768);
    let mut c = h.connect();
    let t = h.map_toplevel(&mut c, "test.app", 300, 200);
    let before = h.state.win(t.id).expect("window").frame_rect();
    let frame = |h: &crate::harness::Harness| {
        let r = h.state.win(t.id).expect("window").frame_rect();
        (r.x, r.width)
    };
    let base = (h.state.scene.outputs().iter())
        .map(|o| h.state.layout_box(Some(o)))
        .map(|b| b.x)
        .collect::<Vec<_>>();
    let (first, second) = (base[0].min(base[1]), base[0].max(base[1]));
    let on_first = before.x < second;
    assert!(on_first, "the window maps on the left output");

    super_arrow(&mut h, &mut c, KEY_LEFT);
    assert_eq!(frame(&h), (first, 512), "the left half");
    super_arrow(&mut h, &mut c, KEY_RIGHT);
    assert_eq!(frame(&h), (first + 512, 512), "the right half");
    super_arrow(&mut h, &mut c, KEY_RIGHT);
    assert_eq!(frame(&h), (second, 512), "the next output's left half");
    super_arrow(&mut h, &mut c, KEY_UP);
    assert!(h.state.win(t.id).expect("window").maximized, "maximized");
    super_arrow(&mut h, &mut c, KEY_DOWN);
    let w = h.state.win(t.id).expect("window");
    assert!(!w.maximized && w.snapped.is_some(), "back in its zone");
    assert_eq!(frame(&h), (second, 512));
    super_arrow(&mut h, &mut c, KEY_DOWN);
    let w = h.state.win(t.id).expect("window");
    assert!(w.snapped.is_none(), "unsnapped");
    assert_eq!(
        (w.frame_rect().width, w.frame_rect().height),
        (before.width, before.height),
        "the size from before the snap"
    );
}

#[test]
fn a_keyboard_size_of_a_snapped_windows_outer_edge_takes_it_out_of_its_group() {
    const KEY_ENTER: u32 = 28;
    let mut h = crate::harness::Harness::new();
    h.output("TEST-1", 1024, 768);
    let mut c = h.connect();
    let t = h.map_toplevel(&mut c, "test.app", 300, 200);
    super_arrow(&mut h, &mut c, KEY_LEFT);
    assert!(
        h.state.wm.snap.group_of(t.id).is_some(),
        "snapped into the left half"
    );
    h.state.begin_key_size(t.id);
    for key in [KEY_LEFT, KEY_RIGHT, KEY_ENTER] {
        h.key(key, true);
        h.key(key, false);
        h.roundtrip(&mut c);
    }
    assert!(
        !h.state.interaction.is_resize_of(t.id),
        "Enter ends the size"
    );
    let w = h.state.win(t.id).expect("window");
    assert!(w.frame_rect().x > 0, "the left edge moved in");
    assert!(w.snapped.is_none(), "no size from before the snap");
    assert!(
        h.state.wm.snap.group_of(t.id).is_none(),
        "out of its group, as a pointer size of that edge"
    );
}
