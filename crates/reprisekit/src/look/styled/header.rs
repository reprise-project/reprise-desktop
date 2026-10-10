//! A list header through the style's [`Part::HeaderItem`],
//! [`Part::HeaderFiller`] and [`Part::HeaderSortArrow`]: each drawer
//! returns `None` or `false` when the style lacks its part, so the
//! forwarder in [`crate::look::widgets::header`] falls back to Classic.
//!
//! Rects are device px (already scaled by `p.s`).

use crate::look::Paint;
use crate::pointer::Interaction;
use crate::raster::Canvas;
use crate::raster::Rect;
use crate::style::{Part, State, Style};

pub fn draw_strip(st: &Style, c: &mut Canvas, p: &Paint, r: Rect) -> bool {
    if st.part(Part::HeaderFiller).is_none() {
        return false;
    }
    st.draw(c, r, Part::HeaderFiller, State::Normal, p.s);
    true
}

/// How much further in a sorted column's title sits, beside its arrow.
const SORTED_DX: i32 = 3;

/// One column's item in its state for `ptr`, a `sorted` column at rest
/// in the part's Checked frame where it has one; the rect its title goes
/// in: the part's content rect, [`SORTED_DX`] further in on a `sorted`
/// column whose arrow follows its title, and a pixel down and right
/// while held.
pub fn draw_item(
    st: &Style,
    c: &mut Canvas,
    p: &Paint,
    r: Rect,
    ptr: Interaction,
    sorted: bool,
) -> Option<Rect> {
    let img = st.part(Part::HeaderItem)?;
    let state = match super::state(true, ptr) {
        State::Normal if sorted && img.states.contains(&State::Checked) => State::Checked,
        s => s,
    };
    st.draw(c, r, Part::HeaderItem, state, p.s);
    let t = st.content_rect(Part::HeaderItem, r, p.s);
    let push = if state == State::Pressed { p.s } else { 0 };
    let after = sorted && !p.look.controls().header_arrow_over;
    let dx = if after { SORTED_DX * p.s } else { 0 };
    Some(Rect::new(
        t.x + push + dx,
        t.y + push,
        t.width - dx,
        t.height,
    ))
}

pub fn sort_arrow_width(st: &Style) -> Option<i32> {
    st.part(Part::HeaderSortArrow)
        .map(|img| img.frame(State::Normal).width / img.scale)
}

/// The sort arrow with its left edge at `x`, over the header `header`:
/// the frame is the header's height and carries the arrow on its rows,
/// the rising one in its Normal frame and the falling one in Checked.
pub fn draw_sort_arrow(
    st: &Style,
    c: &mut Canvas,
    p: &Paint,
    x: i32,
    header: Rect,
    asc: bool,
) -> bool {
    let Some(img) = st.part(Part::HeaderSortArrow) else {
        return false;
    };
    let state = if asc { State::Normal } else { State::Checked };
    let f = img.frame(state);
    let r = Rect::new(
        x,
        header.y,
        f.width / img.scale * p.s,
        f.height / img.scale * p.s,
    );
    st.draw(c, r, Part::HeaderSortArrow, state, p.s);
    true
}

#[cfg(test)]
mod tests {
    use crate::blue::Variant;
    use crate::interface::Interface;
    use crate::interfaces::{blue_theme, theme_for};
    use crate::look::styled::fixture::doubles_at_scale_1;
    use crate::look::widgets::header::{draw_item, draw_sort_arrow, draw_strip};
    use crate::pointer::Interaction;
    use crate::raster::{Canvas, EdgeStyle, Rect};

    /// The header's height under Blue, every frame's own.
    const H: i32 = 20;

    fn item(v: Variant, w: i32, ptr: Interaction) -> Canvas {
        let theme = blue_theme(v);
        let mut c = Canvas::new(w as u32, H as u32);
        draw_item(&mut c, &theme.paint(), Rect::new(0, 0, w, H), ptr, false);
        c
    }

    #[test]
    fn a_classic_column_is_a_raised_button_whatever_the_pointer_does() {
        let theme = theme_for(Interface::Classic);
        let p = theme.paint();
        let r = Rect::new(0, 0, 60, 17);
        let mut want = Canvas::new(60, 17);
        want.fill_rect(r, p.cs.button_face);
        want.edge_t(r, EdgeStyle::ButtonRaised, p.cs, 1);
        for ptr in [Interaction::Idle, Interaction::Hot, Interaction::Pressed] {
            let mut c = Canvas::new(60, 17);
            assert_eq!(draw_item(&mut c, &p, r, ptr, true), r, "the title's box");
            assert!(c == want, "{ptr:?} draws the raised button");
        }
    }

    fn faces(v: Variant) -> &'static crate::blue::scheme::HeaderItemFaces {
        &v.scheme().header_item
    }

    fn top(g: crate::blue::scheme::Grad) -> u32 {
        g[0].1.argb()
    }

    fn luma(p: u32) -> u32 {
        (p >> 16 & 0xFF) + (p >> 8 & 0xFF) + (p & 0xFF)
    }

    /// At rest a navy, olive green or silver column is flat over three
    /// rows darkening to its bottom, with its divider down the last two
    /// columns from row 3 to row 15, at every width.
    #[test]
    fn a_blue_column_at_rest_ends_in_its_divider() {
        for v in [Variant::Blue, Variant::Olive, Variant::Silver] {
            let face = top(faces(v).normal.down);
            for w in [20, 96, 180] {
                let c = item(v, w, Interaction::Idle);
                assert_eq!(c.get(w / 2, 5), face, "{v:?} {w}: the face");
                let band: Vec<u32> = (16..H).map(|y| luma(c.get(w / 2, y))).collect();
                assert!(band.windows(2).all(|p| p[0] > p[1]), "{v:?} {w}: {band:?}");
                for y in 3..=15 {
                    let (shadow, light) = (c.get(w - 2, y), c.get(w - 1, y));
                    assert!(luma(shadow) < luma(face), "{v:?} {w}: shadow row {y}");
                    assert!(luma(light) > luma(shadow), "{v:?} {w}: light row {y}");
                }
                assert_eq!(c.get(w - 2, 2), face, "{v:?} {w}: over the divider");
                assert_eq!(c.get(w - 2, 16), face, "{v:?} {w}: under the divider");
            }
        }
    }

    /// At rest a Slate, Sapphire, Obsidian or Ember column is flat over one
    /// darker row, its divider one column down its last from row 3 to row
    /// 15, fading out over rows 2 and 16, at every width.
    #[test]
    fn a_later_scheme_column_at_rest_ends_in_a_one_column_divider() {
        for v in [
            Variant::Slate,
            Variant::Sapphire,
            Variant::Obsidian,
            Variant::Ember,
        ] {
            let face = top(faces(v).normal.down);
            for w in [20, 96, 180] {
                let c = item(v, w, Interaction::Idle);
                assert_eq!(c.get(w / 2, 5), face, "{v:?} {w}: the face");
                assert_eq!(c.get(w / 2, 18), face, "{v:?} {w}: over the bottom row");
                assert!(
                    luma(c.get(w / 2, 19)) < luma(face),
                    "{v:?} {w}: the bottom row"
                );
                let line = c.get(w - 1, 8);
                assert!(luma(line) < luma(face), "{v:?} {w}: the divider");
                for y in 3..=15 {
                    assert_eq!(c.get(w - 1, y), line, "{v:?} {w}: divider row {y}");
                    assert_eq!(c.get(w - 2, y), face, "{v:?} {w}: beside the divider");
                }
                for y in [2, 16] {
                    let end = luma(c.get(w - 1, y));
                    assert!(
                        luma(line) < end && end < luma(face),
                        "{v:?} {w}: row {y} fades"
                    );
                }
                assert_eq!(c.get(w - 1, 1), face, "{v:?} {w}: over the divider");
            }
        }
    }

    /// Under the pointer a navy, olive green or silver column pales, its
    /// divider goes and the bottom three rows light up as a band whose
    /// bottom-left corner is rounded.
    #[test]
    fn a_blue_column_under_the_pointer_lights_its_band() {
        for v in [Variant::Blue, Variant::Olive, Variant::Silver] {
            let face = top(faces(v).hot.down);
            let w = 96;
            let c = item(v, w, Interaction::Hot);
            assert_eq!(c.get(40, 5), face, "{v:?}: the lit face");
            assert_eq!(c.get(w - 2, 5), face, "{v:?}: no divider");
            for y in 17..H {
                assert_ne!(c.get(40, y), face, "{v:?}: band row {y}");
                assert_eq!(c.get(40, y), c.get(w - 8, y), "{v:?}: band row {y} is flat");
            }
            assert_ne!(
                c.get(0, H - 1),
                c.get(40, H - 1),
                "{v:?}: the rounded corner"
            );
        }
    }

    /// Under the pointer a Slate, Sapphire, Obsidian or Ember column pales
    /// and keeps its divider, over a flat band above its bottom row.
    #[test]
    fn a_later_scheme_column_under_the_pointer_lights_a_flat_band() {
        for v in [
            Variant::Slate,
            Variant::Sapphire,
            Variant::Obsidian,
            Variant::Ember,
        ] {
            let face = top(faces(v).hot.down);
            let w = 96;
            let c = item(v, w, Interaction::Hot);
            assert_eq!(c.get(40, 5), face, "{v:?}: the lit face");
            assert!(
                luma(c.get(w - 1, 5)) < luma(face),
                "{v:?}: the divider stays"
            );
            for y in 17..=18 {
                assert_ne!(c.get(40, y), face, "{v:?}: band row {y}");
                for x in [0, 41, w - 8] {
                    assert_eq!(c.get(x, y), c.get(40, y), "{v:?}: the band at {x}, {y}");
                }
            }
        }
    }

    /// Held, a column sinks into an outline with a shadow along its top
    /// and left, and its title's box moves a pixel down and right.
    #[test]
    fn a_blue_column_held_sinks() {
        for v in Variant::ALL {
            let f = &faces(v).pressed;
            let (outline, shadow) = (f.rim.argb(), top(f.down));
            let w = 120;
            let c = item(v, w, Interaction::Pressed);
            assert_eq!(c.get(0, 5), outline, "{v:?}: left outline");
            assert_eq!(c.get(w - 1, 5), outline, "{v:?}: right outline");
            assert_eq!(c.get(40, H - 1), outline, "{v:?}: bottom outline");
            assert_eq!(c.get(1, 5), shadow, "{v:?}: the shadow inside it");
            assert_eq!(c.get(40, 0), shadow, "{v:?}: the shadow along the top");
            assert!(luma(c.get(40, 10)) > luma(shadow), "{v:?}: the face");
        }
        let theme = blue_theme(Variant::Blue);
        let r = Rect::new(0, 0, 120, H);
        let mut c = Canvas::new(120, H as u32);
        let rest = draw_item(&mut c, &theme.paint(), r, Interaction::Idle, false);
        let held = draw_item(&mut c, &theme.paint(), r, Interaction::Pressed, false);
        assert_eq!(rest, Rect::new(3, 2, 117, 17), "the title's box at rest");
        assert_eq!(
            (held.x, held.y),
            (rest.x + 1, rest.y + 1),
            "held, a pixel on"
        );
        let sorted = draw_item(&mut c, &theme.paint(), r, Interaction::Idle, true);
        assert_eq!(
            sorted.x,
            rest.x + 3,
            "a sorted column's title sits 3 px further in"
        );
        for (v, want) in [
            (Variant::Slate, Rect::new(0, 2, 120, 17)),
            (Variant::Sapphire, Rect::new(0, 2, 120, 17)),
            (Variant::Obsidian, Rect::new(0, 2, 120, 17)),
            (Variant::Ember, Rect::new(3, 2, 117, 15)),
        ] {
            let theme = blue_theme(v);
            let rest = draw_item(&mut c, &theme.paint(), r, Interaction::Idle, false);
            assert_eq!(rest, want, "{v:?}: the title's box at rest");
        }
    }

    /// The arrow is 9 px wide on the header's rows 7 to 11: rising, a
    /// whole pixel on top widening to all nine; falling, upside down. It
    /// inks in the scheme's `gray_text`.
    #[test]
    fn a_blue_sort_arrow_rises_and_falls() {
        for v in Variant::ALL {
            let theme = blue_theme(v);
            let ink = v.scheme().system.gray_text.argb();
            for asc in [true, false] {
                let mut c = Canvas::new(9, H as u32);
                draw_sort_arrow(&mut c, &theme.paint(), 0, Rect::new(0, 0, 60, H), asc);
                let (tip, base) = if asc { (7, 11) } else { (11, 7) };
                assert_eq!(c.get(4, tip), ink, "{v:?} {asc}: the tip");
                assert_eq!(c.get(2, tip), 0, "{v:?} {asc}: beside the tip");
                assert!(
                    (0..9).all(|x| c.get(x, base) == ink),
                    "{v:?} {asc}: the base"
                );
                assert!((0..9).all(|x| c.get(x, 6) == 0 && c.get(x, 12) == 0));
            }
        }
    }

    #[test]
    fn a_blue_header_past_its_last_column_has_its_own_face() {
        for v in Variant::ALL {
            let down = v.scheme().header_filler.normal.down;
            let theme = blue_theme(v);
            let mut c = Canvas::new(50, H as u32);
            draw_strip(&mut c, &theme.paint(), Rect::new(0, 0, 50, H));
            assert_eq!(c.get(25, 5), top(down), "{v:?}");
            assert_eq!(c.get(25, H - 1), down[down.len() - 1].1.argb(), "{v:?}");
        }
    }

    /// At paint scale 2 the header is drawn again at 2: the lit band's
    /// rounded corner and the arrow's sides come out smooth rather than
    /// as the scale-1 drawing doubled.
    #[test]
    fn a_blue_header_is_redrawn_at_paint_scale_2() {
        let theme = blue_theme(Variant::Blue);
        let draw = |theme: &crate::theme::Theme| {
            let p = theme.paint();
            let mut c = Canvas::new(96 * p.s as u32, H as u32 * p.s as u32);
            draw_strip(&mut c, &p, Rect::new(0, 0, 96, H).scaled(p.s));
            let r = Rect::new(0, 0, 70, H).scaled(p.s);
            draw_item(&mut c, &p, r, Interaction::Hot, true);
            draw_sort_arrow(&mut c, &p, 40 * p.s, r, true);
            c
        };
        let (one, two) = (draw(&theme), draw(&theme.scaled(2)));
        assert_eq!((two.width, two.height), (192, 2 * H as u32));
        assert!(!doubles_at_scale_1(&one, &two));
    }
}
