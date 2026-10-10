//! Window arrangement for the taskbar menu: Cascade, Tile Horizontally and
//! Tile Vertically. Pure geometry; the compositor un-maximizes the windows
//! and applies the frame rects these return, top row first.

use crate::raster::Rect;

/// Cascade `count` windows across `usable`: each the same size, offset from
/// the last by `step` down and right, so the last slot ends at the area's
/// bottom-right corner. `step` is the caption height (`cascade_step`).
/// Windows never shrink below a third of the area: a long list first
/// tightens the step, then, at one pixel a step, wraps back to the corner.
pub fn cascade(count: usize, usable: Rect, step: i32) -> Vec<Rect> {
    if count == 0 {
        return Vec::new();
    }
    let n = count as i32;
    let step = step.max(1);
    // The spread leaves at least a third of the area for the windows.
    let max_spread_x = (usable.width * 2 / 3).max(0);
    let max_spread_y = (usable.height * 2 / 3).max(0);
    let slots = n.min(max_spread_x.min(max_spread_y) + 1);
    let gaps = (slots - 1).max(1);
    let step_x = step.min(max_spread_x / gaps);
    let step_y = step.min(max_spread_y / gaps);
    let w = (usable.width - step_x * (slots - 1)).max(1);
    let h = (usable.height - step_y * (slots - 1)).max(1);
    (0..n)
        .map(|i| {
            let k = i % slots;
            Rect::new(usable.x + k * step_x, usable.y + k * step_y, w, h)
        })
        .collect()
}

/// Tile `count` windows in horizontal bands, each the full width and a
/// share of the height ("Tile Windows Horizontally").
pub fn tile_horizontal(count: usize, usable: Rect) -> Vec<Rect> {
    strips(count, usable, false)
}

/// Tile `count` windows in vertical columns, each the full height and a
/// share of the width ("Tile Windows Vertically").
pub fn tile_vertical(count: usize, usable: Rect) -> Vec<Rect> {
    strips(count, usable, true)
}

/// `count` equal strips of `usable`, side by side when `vertical`, stacked
/// otherwise. The last strip takes any remainder so the strips meet the
/// area's far edge exactly. A strip is at least one pixel, so a list longer
/// than the area is wide wraps back to the first strip.
fn strips(count: usize, usable: Rect, vertical: bool) -> Vec<Rect> {
    if count == 0 {
        return Vec::new();
    }
    let n = count as i32;
    let total = if vertical {
        usable.width
    } else {
        usable.height
    };
    let slots = n.min(total.max(1));
    let base = total / slots;
    (0..n)
        .map(|i| {
            let k = i % slots;
            let start = k * base;
            let len = if k == slots - 1 { total - start } else { base }.max(1);
            if vertical {
                Rect::new(usable.x + start, usable.y, len, usable.height)
            } else {
                Rect::new(usable.x, usable.y + start, usable.width, len)
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cascade_steps_from_the_corner_and_shares_the_bottom_right() {
        let u = Rect::new(0, 0, 800, 600);
        let r = cascade(3, u, 24);
        assert_eq!(r.len(), 3);
        assert_eq!((r[0].x, r[0].y), (0, 0));
        assert_eq!((r[1].x, r[1].y), (24, 24));
        assert_eq!((r[2].x, r[2].y), (48, 48));
        // All the same size; the last one's far corner is the area's.
        assert_eq!(r[0].width, r[2].width);
        assert_eq!(r[2].right(), u.right());
        assert_eq!(r[2].bottom(), u.bottom());
    }

    #[test]
    fn cascade_stops_spreading_for_a_long_list() {
        let u = Rect::new(0, 0, 300, 300);
        let r = cascade(50, u, 24);
        // Never below a third of the area.
        assert!(r[0].width >= 100, "{}", r[0].width);
        assert!(r.last().unwrap().right() <= u.right());
    }

    #[test]
    fn cascade_wraps_when_the_list_outnumbers_the_pixels() {
        let u = Rect::new(10, 20, 30, 30);
        let r = cascade(100, u, 24);
        assert_eq!(r.len(), 100);
        assert!(
            r.iter().all(|w| w.width >= 10 && w.height >= 10),
            "every window at least 10x10"
        );
        assert!(
            r.iter()
                .all(|w| w.right() <= u.right() && w.bottom() <= u.bottom()),
            "every window ends inside (40, 50)"
        );
    }

    #[test]
    fn tiles_stay_inside_when_the_list_outnumbers_the_pixels() {
        let u = Rect::new(10, 20, 50, 40);
        for r in [tile_horizontal(100, u), tile_vertical(100, u)] {
            assert_eq!(r.len(), 100);
            assert!(
                r.iter().all(|w| w.x >= u.x && w.y >= u.y),
                "every tile starts at or after (10, 20)"
            );
            assert!(
                r.iter()
                    .all(|w| w.right() <= u.right() && w.bottom() <= u.bottom()),
                "every tile ends at or before (60, 60)"
            );
        }
    }

    #[test]
    fn horizontal_tiles_stack_full_width() {
        let u = Rect::new(10, 20, 800, 600);
        let r = tile_horizontal(3, u);
        assert!(r.iter().all(|w| w.x == 10 && w.width == 800));
        assert_eq!(r[0].y, 20);
        assert_eq!(r[1].y, 20 + 200);
        assert_eq!(r[2].bottom(), u.bottom());
    }

    #[test]
    fn vertical_tiles_sit_side_by_side_full_height() {
        let u = Rect::new(0, 0, 900, 500);
        let r = tile_vertical(3, u);
        assert!(r.iter().all(|w| w.y == 0 && w.height == 500));
        assert_eq!(r[0].x, 0);
        assert_eq!(r[1].x, 300);
        assert_eq!(r[2].right(), u.right());
    }

    #[test]
    fn one_window_fills_the_area() {
        let u = Rect::new(5, 5, 400, 300);
        assert_eq!(tile_horizontal(1, u), vec![u]);
        assert_eq!(tile_vertical(1, u), vec![u]);
    }

    #[test]
    fn no_windows_gives_nothing() {
        let u = Rect::new(0, 0, 100, 100);
        assert!(cascade(0, u, 24).is_empty());
        assert!(tile_horizontal(0, u).is_empty());
    }
}
