//! The field frame's [`Part::EditBorder`] atlas: a 1 px line with square corners
//! around edit boxes, combo fields, list boxes and list and tree views. Two 5x5
//! frames stacked, Normal then Disabled, each a 2 px border (the line, then a
//! ring just inside it). Normal's ring is clear so the caller's fill runs up to
//! the line; Disabled is a greyed combo box's. Focus shows as the caret alone.
//! A scheme with a `list_border` (silver) gives list boxes, views and combo
//! boxes [`Part::ListBorder`] instead.

use crate::blue::Variant;
use crate::blue::scheme::Face;
use crate::raster::{Canvas, Insets, Rect};
use crate::style::{Layout, Part, PartImage, State};

pub(super) fn parts(v: Variant) -> Vec<(Part, PartImage)> {
    let s = v.scheme();
    let mut parts = vec![(
        Part::EditBorder,
        border(&s.edit_border.normal, &s.edit_border.disabled),
    )];
    if let Some(list) = &s.list_border {
        parts.push((Part::ListBorder, border(&list.normal, &list.disabled)));
    }
    parts
}

/// The Normal frame, `normal`'s line, over the greyed combo box's frame,
/// `greyed`'s line round its inner ring.
fn border(normal: &Face, greyed: &Face) -> PartImage {
    let mut a = Canvas::new(5, 10);
    a.ring_t(Rect::new(0, 0, 5, 5), normal.rim, normal.rim, 1);
    a.ring_t(Rect::new(0, 5, 5, 5), greyed.rim, greyed.rim, 1);
    a.ring_t(Rect::new(1, 6, 3, 3), greyed.inner, greyed.inner, 1);
    PartImage {
        border_only: true,
        ..PartImage::new(a, &[State::Normal, State::Disabled], Layout::Vertical)
            .sizing(Insets::new(2, 2, 2, 2))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(v: Variant, p: Part) -> Option<PartImage> {
        parts(v)
            .into_iter()
            .find(|(q, _)| *q == p)
            .map(|(_, img)| img)
    }

    fn px(img: &PartImage, state: State, x: i32, y: i32) -> u32 {
        let f = img.frame(state);
        img.atlas.get(f.x + x, f.y + y)
    }

    #[test]
    fn each_variant_lines_its_edits_in_its_scheme_colour() {
        for v in Variant::ALL {
            let want = v.scheme().edit_border.normal.rim.argb();
            let img = part(v, Part::EditBorder).unwrap();
            for (x, y) in [(0, 0), (4, 0), (0, 4), (4, 4), (2, 0), (0, 2)] {
                assert_eq!(px(&img, State::Normal, x, y), want, "{v:?} ({x}, {y})");
            }
            assert_eq!(px(&img, State::Normal, 1, 1), 0, "{v:?}: the ring is clear");
        }
    }

    #[test]
    fn only_silver_lines_its_lists_apart() {
        for v in Variant::ALL {
            if v != Variant::Silver {
                assert!(part(v, Part::ListBorder).is_none(), "{v:?}");
            }
        }
        let s = Variant::Silver.scheme();
        let list = s.list_border.as_ref().unwrap().normal.rim.argb();
        assert_ne!(
            list,
            s.edit_border.normal.rim.argb(),
            "the edit line differs"
        );
        let img = part(Variant::Silver, Part::ListBorder).unwrap();
        assert_eq!(px(&img, State::Normal, 0, 0), list);
        assert_eq!(px(&img, State::Normal, 4, 2), list);
    }

    #[test]
    fn a_greyed_combo_is_a_pale_line_over_a_paler_ring() {
        for v in Variant::ALL {
            for p in [Part::EditBorder, Part::ListBorder] {
                let Some(img) = part(v, p) else { continue };
                let (line, ring) = (
                    px(&img, State::Disabled, 0, 0),
                    px(&img, State::Disabled, 1, 1),
                );
                let luma = |c: u32| (c >> 16 & 0xFF) + (c >> 8 & 0xFF) + (c & 0xFF);
                assert!(
                    luma(ring) > luma(line),
                    "{v:?} {p:?}: {line:08X} {ring:08X}"
                );
                assert!(
                    luma(line) > luma(px(&img, State::Normal, 0, 0)),
                    "{v:?} {p:?}"
                );
                assert_eq!(px(&img, State::Disabled, 2, 2), 0, "{v:?} {p:?}");
            }
        }
    }
}
