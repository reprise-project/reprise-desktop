//! Places bar panel and selected tile through the style's parts; drawers return
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

pub fn draw_selected(st: &Style, c: &mut Canvas, band: Rect, s: i32) -> bool {
    if st.part(Part::PlacesTile).is_none() {
        return false;
    }
    st.draw(c, band, Part::PlacesTile, State::Checked, s);
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
}
