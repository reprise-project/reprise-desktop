use super::*;

fn peak_lum(c: &Canvas, xs: std::ops::Range<i32>, ys: std::ops::Range<i32>) -> i32 {
    ys.flat_map(|y| xs.clone().map(move |x| (x, y)))
        .map(|(x, y)| lum(c.get(x, y)))
        .max()
        .unwrap_or(0)
}

#[test]
fn a_glass_pager_lights_the_current_desktop_at_its_foot() {
    use crate::taskbar::PagerMark;
    for small in [false, true] {
        let (c, cells, bar) = glass_pager(PagerMark::Light, None, small);
        let (r0, r1) = (cells[0], cells[1]);
        assert_eq!(r1.width, 23, "a pager cell is 23 px");
        let foot = r1.bottom() - 1;
        assert!(
            lum(c.get(r1.x + 11, foot)) > lum(c.get(r0.x + 11, foot)) + 20,
            "small {small}: the current cell's foot is lit"
        );
        let peak = (r1.x..r1.right())
            .max_by_key(|&x| lum(c.get(x, foot)))
            .unwrap();
        assert_eq!(
            peak,
            r1.x + 11,
            "small {small}: the light peaks at the cell's middle column"
        );
        for (x, y) in (0..3).flat_map(|x| (0..3).map(move |y| (x, y))) {
            assert_eq!(
                c.get(r1.x + x, r1.y + y),
                bar.get(r1.x + x, r1.y + y),
                "small {small}: no frame at the current cell's top left ({x}, {y})"
            );
        }
        let rows = r1.y..r1.bottom() - 7;
        assert!(
            peak_lum(&c, r0.x..r0.right(), rows.clone()) < peak_lum(&c, r1.x..r1.right(), rows),
            "small {small}: another desktop's number is dimmer than the current one's"
        );
    }
}

#[test]
fn a_glass_pager_tile_is_short_and_centred() {
    use crate::taskbar::PagerMark;
    for (small, top, h) in [(false, 8, 24), (true, 5, 20)] {
        for hot in [None, Some(1), Some(2)] {
            let (c, cells, bar) = glass_pager(PagerMark::Tile, hot, small);
            let mut lit = vec![cells[1]];
            lit.extend(hot.map(|i| cells[i]));
            for r in lit {
                for y in (0..top).chain(top + h..r.height) {
                    for x in r.x..r.right() {
                        assert_eq!(
                            c.get(x, r.y + y),
                            bar.get(x, r.y + y),
                            "small {small} hot {hot:?}: no tile at ({x}, {y}) outside rows {top}..{}",
                            top + h
                        );
                    }
                }
                let mid = r.y + top + h / 2;
                assert_ne!(
                    c.get(r.x + 1, mid),
                    bar.get(r.x + 1, mid),
                    "small {small} hot {hot:?}: the tile starts 1 px in"
                );
                assert_ne!(
                    c.get(r.x + 21, mid),
                    bar.get(r.x + 21, mid),
                    "small {small} hot {hot:?}: the tile ends 1 px before the cell does"
                );
                for x in [r.x, r.x + 22] {
                    assert_eq!(
                        c.get(x, mid),
                        bar.get(x, mid),
                        "small {small} hot {hot:?}: the tile is 21 px wide, column {x} is bar"
                    );
                }
            }
            let mid = cells[0].y + top + h / 2;
            assert_eq!(
                c.get(cells[0].x + 1, mid),
                bar.get(cells[0].x + 1, mid),
                "small {small} hot {hot:?}: another desktop at rest has no tile"
            );
        }
    }
}

#[test]
fn a_hot_glass_pager_cell_lights_its_edges() {
    use crate::taskbar::PagerMark;
    let (c, cells, bar) = glass_pager(PagerMark::Light, Some(2), false);
    let r = cells[2];
    let mid = r.y + r.height / 2;
    for x in [r.x, r.x + 1, r.right() - 2, r.right() - 1] {
        assert_ne!(c.get(x, mid), bar.get(x, mid), "edge column {x} is lit");
    }
    let foot = r.bottom() - 1;
    assert_eq!(
        c.get(r.x + 11, foot),
        bar.get(r.x + 11, foot),
        "no foot light"
    );
    let (hot, cells, _) = glass_pager(PagerMark::Light, Some(1), false);
    let (rest, _, _) = glass_pager(PagerMark::Light, None, false);
    let r = cells[1];
    for x in [r.x, r.right() - 1] {
        assert_ne!(
            hot.get(x, mid),
            bar.get(x, mid),
            "current: edge column {x} is lit"
        );
    }
    assert_eq!(
        hot.get(r.x + 11, foot),
        rest.get(r.x + 11, foot),
        "current: the foot light stays"
    );
}

#[test]
fn blue_ignores_the_pager_mark() {
    use crate::taskbar::PagerMark;
    let theme = theme_for(Interface::Blue);
    let s = theme.look.style().unwrap();
    let (light, cells, bar) = pager_bar(&theme, PagerMark::Light, None);
    let (tile, _, _) = pager_bar(&theme, PagerMark::Tile, None);
    assert_eq!(light.pixels, tile.pixels, "both marks draw alike");
    let r = cells[1];
    let mut want = bar.clone();
    s.draw(&mut want, r, Part::TaskButton, State::Checked, 1);
    for (x, y) in [
        (r.x + 1, r.y + 2),
        (r.x, r.y),
        (r.right() - 1, r.bottom() - 1),
    ] {
        assert_eq!(
            light.get(x, y),
            want.get(x, y),
            "the current cell is a Checked button at ({x}, {y})"
        );
    }
}
