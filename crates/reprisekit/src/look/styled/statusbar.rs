//! Each drawer returns `false` when the style lacks its part, so the forwarder falls back to Classic.

use crate::look::Paint;
use crate::raster::{Canvas, Rect};
use crate::style::{Part, State, Style};

pub fn draw_bar(st: &Style, c: &mut Canvas, p: &Paint, r: Rect) -> bool {
    if st.part(Part::StatusBar).is_none() {
        return false;
    }
    st.draw(c, r, Part::StatusBar, State::Normal, p.s);
    true
}

pub fn has_pane(st: &Style) -> bool {
    st.part(Part::StatusPane).is_some()
}

/// Returns the pane's content rect: the text box starts at its top, the icon centres in it.
pub fn draw_pane(st: &Style, c: &mut Canvas, p: &Paint, r: Rect) -> Option<Rect> {
    if !has_pane(st) {
        return None;
    }
    st.draw(c, r, Part::StatusPane, State::Normal, p.s);
    Some(st.content_rect(Part::StatusPane, r, p.s))
}

#[cfg(test)]
mod tests {
    use crate::blue::Variant;
    use crate::interfaces::blue_theme;
    use crate::look::widgets::statusbar::draw_bar;
    use crate::raster::{Canvas, Rect};
    use crate::theme::Color;

    fn rgb(c: Color) -> u32 {
        0xFF00_0000 | u32::from(c.r) << 16 | u32::from(c.g) << 8 | u32::from(c.b)
    }

    fn luma(p: u32) -> u32 {
        (p >> 16 & 0xFF) + (p >> 8 & 0xFF) + (p & 0xFF)
    }

    fn shaded(v: Variant) -> bool {
        matches!(
            v,
            Variant::Blue | Variant::Olive | Variant::Silver | Variant::Steel
        )
    }

    #[test]
    fn a_blue_status_bar_has_its_own_ends() {
        for v in Variant::ALL {
            let f = v.scheme().status_bar.normal;
            let theme = blue_theme(v);
            let w = 300;
            let mut c = Canvas::new(w as u32, 23);
            draw_bar(&mut c, &theme.paint(), Rect::new(0, 0, w, 23));
            let face = c.get(150, 8);
            assert_eq!(c.get(150, 0), rgb(f.down[0].1), "{v:?}: the top line");
            assert!(luma(c.get(150, 0)) < luma(face), "{v:?}: a dark line");
            let last = f.down[f.down.len() - 1].1;
            assert_eq!(c.get(150, 22), rgb(last), "{v:?}: the bottom row");
            if shaded(v) {
                assert!(luma(c.get(0, 8)) > luma(face), "{v:?}: a paler left end");
                assert_ne!(c.get(w - 1, 8), face, "{v:?}: the right end");
                assert_eq!(c.get(w - 60, 8), face, "{v:?}: the face runs flat");
            } else {
                assert_eq!(c.get(0, 8), face, "{v:?}: the left end");
                assert_eq!(c.get(w - 1, 8), face, "{v:?}: the right end");
            }
        }
    }

    #[test]
    fn a_later_scheme_pane_divider_is_one_line() {
        for v in [
            Variant::Slate,
            Variant::Sapphire,
            Variant::Obsidian,
            Variant::Ember,
            Variant::Cobalt,
            Variant::Carbon,
        ] {
            let theme = blue_theme(v);
            let p = theme.paint();
            let st = crate::blue::style_for(v);
            let (w, h) = (60, 23);
            let r = Rect::new(0, 0, w, h);
            let mut bare = Canvas::new(w as u32, h as u32);
            draw_bar(&mut bare, &p, r);
            let mut c = bare.clone();
            assert!(super::draw_pane(&st, &mut c, &p, r).is_some());
            for y in 4..=20 {
                let (line, bar) = (c.get(w - 2, y), bare.get(w - 2, y));
                assert!(luma(line) < luma(bar), "{v:?}: the line at row {y}");
                assert!(luma(line) + 90 > luma(bar), "{v:?}: a light line at {y}");
                assert_eq!(c.get(w - 1, y), bare.get(w - 1, y), "{v:?}: after it");
            }
            for y in [3, 21] {
                assert_eq!(c.get(w - 2, y), bare.get(w - 2, y), "{v:?}: row {y}");
            }
        }
    }
}
