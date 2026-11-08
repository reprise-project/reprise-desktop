use super::*;

#[test]
fn the_chip_reads_plus_n_more_and_fits_its_label() {
    assert_eq!(chip_label(2), "+2 more");
    let (theme, t) = (theme_for(Interface::Glass), text());
    let (w, h) = chip_layout(&theme, &t, 12);
    assert_eq!(h, 28);
    assert!(w > t.measure("+12 more", false, 12));
    let c = draw_chip(&theme, &t, 12, false);
    assert_eq!((c.width as i32, c.height as i32), (w, h));
    assert_ne!(fnv(&c), fnv(&draw_chip(&theme, &t, 12, true)), "hot lifts");
    let (w2, h2) = chip_layout(&theme.scaled(2), &t, 12);
    assert_eq!((w2 % 2, h2), (0, 2 * h), "whole design px at scale 2");
}

#[test]
fn the_stack_grows_away_from_the_tray_on_every_edge() {
    let u = Rect::new(0, 0, 1920, 1050);
    let three = [(360, 96); 3];
    let chip = Some((90, 28));
    assert_eq!(
        place_stack(u, &three, chip, Edge::Bottom),
        (
            vec![(1552, 946), (1552, 842), (1552, 738)],
            Some((1822, 702))
        )
    );
    assert_eq!(
        place_stack(u, &three, chip, Edge::Right).0,
        vec![(1552, 946), (1552, 842), (1552, 738)]
    );
    assert_eq!(
        place_stack(u, &three, None, Edge::Left).0,
        vec![(8, 946), (8, 842), (8, 738)]
    );
    assert_eq!(
        place_stack(u, &three, chip, Edge::Top),
        (vec![(1552, 8), (1552, 112), (1552, 216)], Some((1822, 320)))
    );
}

#[test]
fn a_stack_too_tall_stays_inside_the_usable_area() {
    let u = Rect::new(0, 30, 800, 300);
    let (at, _) = place_stack(u, &[(360, 200); 3], None, Edge::Bottom);
    assert!(at.iter().all(|&(_, y)| y >= 30), "{at:?}");
}

#[test]
fn an_arriving_sheet_rises_14_px_and_fades_in() {
    let rest = Rect::new(100, 500, 360, 120);
    assert_eq!(
        pose(0.0, rest, RISE, 1, 0.95),
        (Rect::new(109, 517, 342, 114), 0.0)
    );
    assert_eq!(pose(1.0, rest, RISE, 1, 0.95), (rest, 1.0));
    assert_eq!(
        pose(1.3, rest, RISE, 1, 0.95),
        (rest, 1.0),
        "k past 1 is clamped"
    );
}

#[test]
fn under_a_top_bar_the_sheet_rises_from_above() {
    assert_eq!(
        pose(0.0, Rect::new(100, 500, 360, 120), RISE, -1, 0.95),
        (Rect::new(109, 489, 342, 114), 0.0)
    );
}
