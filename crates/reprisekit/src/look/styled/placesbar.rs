//! Places bar panel and tile frames through the style's parts; drawers return
//! `false` when the style lacks one.

use crate::raster::{Canvas, Rect};
use crate::style::{Part, State, Style};

pub fn draw_panel(st: &Style, c: &mut Canvas, r: Rect, s: i32) -> bool {
    if st.part(Part::PlacesBar).is_none() {
        return false;
    }
    st.draw(c, r, Part::PlacesBar, State::Normal, s);
    true
}

pub fn draw_tile(st: &Style, c: &mut Canvas, band: Rect, state: State, s: i32) -> bool {
    if st.part(Part::PlacesTile).is_none() {
        return false;
    }
    st.draw(c, band, Part::PlacesTile, state, s);
    true
}

#[cfg(test)]
mod tests {
    use crate::blue::Variant;
    use crate::interfaces::blue_theme;
    use crate::look::widgets::placesbar::draw_panel;
    use crate::raster::{Canvas, Rect};

    fn lum(p: u32) -> u32 {
        (p >> 16 & 0xFF) + (p >> 8 & 0xFF) + (p & 0xFF)
    }

    #[test]
    fn a_blue_places_bar_frames_a_light_fill() {
        for v in Variant::ALL {
            let r = v.scheme().places_bar.normal.rim;
            let rim = 0xFF00_0000 | u32::from(r.r) << 16 | u32::from(r.g) << 8 | u32::from(r.b);
            let theme = blue_theme(v);
            let p = theme.paint();
            let mut c = Canvas::new(87, 338);
            draw_panel(&mut c, &p, Rect::new(0, 0, 87, 338));
            assert_eq!(c.get(0, 100), rim, "{v:?} left line");
            assert_eq!(c.get(1, 100), 0xFFFF_FFFF, "{v:?} white inside it");
            assert!(lum(c.get(40, 150)) < lum(0xFFFF_FFFF), "{v:?} fill");
            assert!(
                lum(c.get(40, 150)) > lum(rim),
                "{v:?} fill lighter than the rim"
            );
            for y in 4..334 {
                assert_eq!(c.get(86, y), c.get(86, 100), "{v:?} right edge at {y}");
            }
            assert!(lum(c.get(86, 100)) < lum(rim), "{v:?} right edge darker");
            assert!(
                lum(c.get(86, 337)) > lum(c.get(86, 100)),
                "{v:?} bottom-right corner"
            );
            assert!(lum(c.get(0, 0)) > lum(rim), "{v:?} top-left corner");
        }
    }

    #[test]
    fn the_blue_places_bar_is_drawn_anew_at_scale_2() {
        let theme = blue_theme(Variant::Blue).scaled(2);
        let p = theme.paint();
        let mut c = Canvas::new(80, 80);
        draw_panel(&mut c, &p, Rect::new(0, 0, 80, 80));
        assert_eq!(c.get(0, 40), c.get(1, 40), "the rim is 2 px");
        assert_eq!(c.get(2, 40), 0xFFFF_FFFF, "white inside it");
        assert_eq!(c.get(3, 40), 0xFFFF_FFFF, "2 px of white");
        assert_ne!(c.get(4, 40), 0xFFFF_FFFF, "then the fill");
        let right: Vec<u32> = (0..10).map(|y| c.get(79, y)).collect();
        assert!(
            (0..10).step_by(2).any(|y| right[y] != right[y + 1]),
            "the shade's fade is not doubled: {right:x?}"
        );
    }

    /// A 67 px Blue tile over the bar's panel at scale `d` in `v`: plain, lit, selected,
    /// selected and lit, held.
    fn blue_tiles(v: Variant, d: i32) -> Vec<Canvas> {
        use crate::look::widgets::placesbar::draw_tile;
        use crate::style::State;
        let theme = blue_theme(v).scaled(d);
        let p = theme.paint();
        let r = Rect::new(0, 0, 87, 67).scaled(d);
        [
            None,
            Some(State::Hot),
            Some(State::Checked),
            Some(State::CheckedHot),
            Some(State::Pressed),
        ]
        .into_iter()
        .map(|state| {
            let mut c = Canvas::new(r.width as u32, r.height as u32);
            draw_panel(&mut c, &p, r);
            if let Some(state) = state {
                draw_tile(&mut c, &p, r, state);
            }
            c
        })
        .collect()
    }

    #[test]
    fn a_blue_tile_tells_each_state_apart_inside_the_panels_rim() {
        for v in Variant::ALL {
            for d in [1, 2] {
                let tiles = blue_tiles(v, d);
                for i in 0..tiles.len() {
                    for j in i + 1..tiles.len() {
                        assert!(tiles[i].pixels != tiles[j].pixels, "{v:?} @{d}: {i} vs {j}");
                    }
                }
                let (w, h) = (87 * d, 71 * d);
                let rim: Vec<(i32, i32)> = (0..3 * d)
                    .flat_map(|k| (0..h).flat_map(move |y| [(k, y), (w - 1 - k, y)]))
                    .chain(
                        (0..3 * d).flat_map(|k| (0..w).flat_map(move |x| [(x, k), (x, h - 1 - k)])),
                    )
                    .collect();
                for (n, t) in tiles.iter().enumerate().skip(1) {
                    for &(x, y) in &rim {
                        assert_eq!(
                            t.get(x, y),
                            tiles[0].get(x, y),
                            "{v:?} @{d} state {n} at ({x}, {y})"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn blue_places_tiles_pin_at_scale_1_and_2() {
        const STATES: [&str; 5] = ["plain", "hot", "selected", "selected hot", "held"];
        let got: Vec<_> = Variant::ALL
            .into_iter()
            .flat_map(|v| {
                [1, 2].into_iter().flat_map(move |d| {
                    STATES
                        .iter()
                        .zip(blue_tiles(v, d))
                        .map(move |(s, c)| (format!("{v:?} {s} {d}"), crate::pins::fnv(&c)))
                })
            })
            .collect();
        crate::pins::check("blue/places_tiles", &got);
    }
}
