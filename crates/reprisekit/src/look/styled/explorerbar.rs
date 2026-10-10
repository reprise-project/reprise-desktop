//! An explorer bar through the style's [`Part::ExplorerBarHeader`],
//! [`Part::ExplorerBarClose`]: each drawer returns `false` when the style
//! lacks its parts, so the forwarder in [`crate::look::widgets`] falls
//! back to Classic.
//!
//! The strip's part reaches past the bottom of the strip's rect over the
//! line under it (its `outset`); the close box's frames carry the X.

use crate::look::styled::state;
use crate::pointer::Interaction;
use crate::raster::{Canvas, Rect};
use crate::style::{Part, State, Style};

/// [`crate::look::widgets::explorerbar::draw_strip`] through `st`.
pub fn draw_strip(st: &Style, c: &mut Canvas, header: Rect, s: i32) -> bool {
    if st.part(Part::ExplorerBarHeader).is_none() {
        return false;
    }
    st.draw(c, header, Part::ExplorerBarHeader, State::Normal, s);
    true
}

/// [`crate::look::widgets::explorerbar::draw_close`] through `st`.
pub fn draw_close(st: &Style, c: &mut Canvas, r: Rect, ptr: Interaction, s: i32) -> bool {
    if st.part(Part::ExplorerBarClose).is_none() {
        return false;
    }
    st.draw(c, r, Part::ExplorerBarClose, state(true, ptr), s);
    true
}

#[cfg(test)]
mod tests {
    use crate::blue::Variant;
    use crate::controls::explorerbar::{ExplorerBar, layout};
    use crate::interfaces::blue_theme;
    use crate::look::widgets::Interaction;
    use crate::look::widgets::explorerbar::{draw_close, draw_strip};
    use crate::raster::{Canvas, Rect};
    use crate::theme::Color;

    /// The strip's colour runs from its light left column to its right,
    /// every row alike, over a one-row line in the scheme's rim colour.
    #[test]
    fn a_blue_strip_runs_its_gradient_over_its_line() {
        let sum = |p: u32| (p >> 16 & 0xFF) + (p >> 8 & 0xFF) + (p & 0xFF);
        for v in Variant::ALL {
            let theme = blue_theme(v);
            let p = theme.paint();
            let sentinel = Color::rgb(1, 2, 3);
            let mut c = Canvas::new(300, 30);
            c.fill_rect(c.bounds(), sentinel);
            draw_strip(
                &mut c,
                &p,
                Rect::new(0, 0, 300, 30),
                Rect::new(0, 0, 300, 22),
            );
            for x in [0, 150, 299] {
                for y in 1..22 {
                    assert_eq!(c.get(x, y), c.get(x, 0), "{v:?} column {x} row {y}");
                }
            }
            assert!(
                sum(c.get(0, 0)) > sum(c.get(299, 0)),
                "{v:?} light at the left"
            );
            let line = v.scheme().explorer_bar_header.normal.rim.argb();
            assert_eq!(c.get(100, 22), line, "{v:?} the line");
            assert_eq!(
                c.get(100, 23),
                sentinel.argb(),
                "{v:?} the body is the host's"
            );
        }
    }

    /// At rest the box is the X alone; lit it is a raised box with its
    /// corners open round the X; held a sunken box, the X a pixel right.
    #[test]
    fn a_blue_close_box_draws_its_x_and_its_box() {
        let dark = |p: u32| (p >> 16 & 0xFF) + (p >> 8 & 0xFF) + (p & 0xFF) < 150;
        for v in Variant::ALL {
            let faces = &v.scheme().explorer_bar_close;
            let theme = blue_theme(v);
            let p = theme.paint();
            let sentinel = Color::rgb(1, 2, 3);
            let draw = |ptr| {
                let mut c = Canvas::new(20, 17);
                c.fill_rect(c.bounds(), sentinel);
                draw_close(&mut c, &p, Rect::new(0, 0, 20, 17), ptr);
                c
            };
            let rest = draw(Interaction::Idle);
            assert!(dark(rest.get(6, 5)), "{v:?} the X");
            assert_eq!(rest.get(1, 1), sentinel.argb(), "{v:?} no box at rest");
            let hot = draw(Interaction::Hot);
            let near = |a: u32, b: u32| {
                (0..4).all(|i| ((a >> (8 * i)) & 0xFF).abs_diff((b >> (8 * i)) & 0xFF) <= 8)
            };
            assert!(near(hot.get(0, 0), sentinel.argb()), "{v:?} open corner");
            assert_eq!(
                hot.get(4, 0),
                faces.hot.rim.argb(),
                "{v:?} the box's top edge"
            );
            assert!(!dark(hot.get(3, 2)), "{v:?} its lit face");
            assert!(dark(hot.get(6, 5)), "{v:?} the X lit");
            let held = draw(Interaction::Pressed);
            assert_eq!(
                held.get(4, 0),
                faces.pressed.rim.argb(),
                "{v:?} the held edge"
            );
            assert!(!dark(held.get(6, 5)), "{v:?} held, the X moves right");
            assert!(dark(held.get(7, 5)), "{v:?} the X held");
        }
    }

    /// Blue's bar has no ring: the strip from the bar's top, the body from
    /// its left edge under the line, the close box against the strip's top.
    #[test]
    fn a_blue_bar_lays_out_without_a_ring() {
        let cm = blue_theme(Variant::Blue).look.controls();
        let lay = layout(
            &ExplorerBar::new("Folders"),
            Rect::new(0, 83, 204, 300),
            &cm,
        );
        assert_eq!(lay.header, Rect::new(0, 83, 200, 22));
        assert_eq!(lay.body, Rect::new(0, 106, 200, 277));
        assert_eq!(lay.close, Rect::new(179, 83, 20, 17));
    }

    /// At 2 the strip and the close box are drawn again, not doubled:
    /// the box's rounded corner and the X's slanted edges hold 2x2 blocks
    /// that are not one colour.
    #[test]
    fn the_blue_explorer_bar_is_redrawn_at_scale_2() {
        for v in Variant::ALL {
            let theme = blue_theme(v).scaled(2);
            let p = theme.paint();
            let mut c = Canvas::new(120, 60);
            c.fill_rect(c.bounds(), theme.colors.button_face);
            draw_strip(
                &mut c,
                &p,
                Rect::new(0, 0, 120, 60),
                Rect::new(0, 0, 120, 44),
            );
            draw_close(&mut c, &p, Rect::new(60, 0, 40, 34), Interaction::Hot);
            let blocks = (0..17)
                .flat_map(|y| (30..50).map(move |x| (2 * x, 2 * y)))
                .filter(|&(x, y)| {
                    let a = c.get(x, y);
                    [(1, 0), (0, 1), (1, 1)]
                        .iter()
                        .any(|&(dx, dy)| c.get(x + dx, y + dy) != a)
                })
                .count();
            assert!(blocks > 20, "{v:?}: {blocks} mixed blocks at 2");
        }
    }
}
