//! A client's blur region (ext-background-effect-v1) under Glass: the window's frame glass runs
//! down under it, blurred and tinted as the frame is, and the client draws over it. A region
//! across the client's whole top opens the frame's client edges along it (`glass_top`).

use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Rectangle};
use smithay::wayland::background_effect::{
    BackgroundEffectSurfaceCachedState, Capability, ExtBackgroundEffectHandler,
};
use smithay::wayland::compositor::{RectangleKind, with_states};

use crate::compositor::State;
use crate::raster::Rect;

impl ExtBackgroundEffectHandler for State {
    fn capabilities(&self) -> Capability {
        if crate::glass::wanted(&self.config) {
            Capability::Blur
        } else {
            Capability::empty()
        }
    }
}

/// Past this many ops a region is taken as the bounding box of its adds.
const MAX_OPS: usize = 64;

type Op = (RectangleKind, Rectangle<i32, Logical>);

/// A window's last committed blur region ops and their `disjoint` rects, redone only when the
/// ops change.
#[derive(Default)]
pub struct BlurOps {
    ops: Vec<Op>,
    rects: Vec<Rectangle<i32, Logical>>,
}

impl BlurOps {
    fn update(&mut self, ops: &[Op]) {
        if !same_ops(&self.ops, ops) {
            self.ops = ops.to_vec();
            self.rects = disjoint(ops);
        }
    }
}

fn same_ops(a: &[Op], b: &[Op]) -> bool {
    let kind = |k: &RectangleKind| matches!(k, RectangleKind::Add);
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|((ka, ra), (kb, rb))| ra == rb && kind(ka) == kind(kb))
}

/// `surface`'s committed blur region as disjoint rects in client coordinates (the surface's
/// minus its window geometry's `origin`), cut to the `client` size.
pub fn region(
    surface: &WlSurface,
    cache: &mut BlurOps,
    origin: (i32, i32),
    client: (i32, i32),
) -> Vec<Rect> {
    with_states(surface, |s| {
        let mut state = s.cached_state.get::<BackgroundEffectSurfaceCachedState>();
        let ops = state.current().blur_region.as_ref().map(|r| &r.rects[..]);
        cache.update(ops.unwrap_or_default());
    });
    let area = Rectangle::<i32, Logical>::new(origin.into(), client.into());
    cache
        .rects
        .iter()
        .filter_map(|r| r.intersection(area))
        .map(|r| Rect::new(r.loc.x - origin.0, r.loc.y - origin.1, r.size.w, r.size.h))
        .collect()
}

/// A wl_region's adds and subtracts, applied in order, as rects that do not overlap.
fn disjoint(ops: &[Op]) -> Vec<Rectangle<i32, Logical>> {
    if ops.len() > MAX_OPS {
        return ops
            .iter()
            .filter(|(k, _)| matches!(k, RectangleKind::Add))
            .map(|&(_, r)| r)
            .reduce(|a, b| a.merge(b))
            .into_iter()
            .filter(|r| !r.is_empty())
            .collect();
    }
    let mut out: Vec<Rectangle<i32, Logical>> = Vec::new();
    for &(kind, r) in ops {
        match kind {
            RectangleKind::Add => {
                let fresh = r.subtract_rects(out.iter().copied());
                out.extend(fresh);
            }
            RectangleKind::Subtract => {
                out = out.into_iter().flat_map(|o| o.subtract_rect(r)).collect();
            }
        }
    }
    out.retain(|r| !r.is_empty());
    out
}

/// How many of a `width` wide client's top rows `rects` cover from edge to edge, unbroken.
pub fn glass_top(rects: &[Rect], width: i32) -> i32 {
    let mut y = 0;
    loop {
        let mut row: Vec<&Rect> = rects
            .iter()
            .filter(|r| r.y <= y && y < r.bottom())
            .collect();
        row.sort_by_key(|r| r.x);
        let mut x = 0;
        for r in &row {
            if r.x > x {
                break;
            }
            x = x.max(r.right());
        }
        if row.is_empty() || x < width {
            return y;
        }
        match row.iter().map(|r| r.bottom()).min() {
            Some(next) => y = next,
            None => return y,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Logical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    #[test]
    fn a_band_across_the_top_is_glass_that_many_rows_down() {
        assert_eq!(glass_top(&[Rect::new(0, 0, 400, 78)], 400), 78);
    }

    #[test]
    fn a_band_short_of_either_side_or_below_the_top_opens_nothing() {
        assert_eq!(glass_top(&[Rect::new(1, 0, 399, 78)], 400), 0);
        assert_eq!(glass_top(&[Rect::new(0, 0, 399, 78)], 400), 0);
        assert_eq!(glass_top(&[Rect::new(0, 5, 400, 78)], 400), 0);
        assert_eq!(glass_top(&[], 400), 0);
    }

    #[test]
    fn side_by_side_and_stacked_rects_add_up() {
        let rects = [
            Rect::new(0, 0, 150, 40),
            Rect::new(150, 0, 250, 20),
            Rect::new(150, 20, 250, 30),
        ];
        assert_eq!(glass_top(&rects, 400), 40, "the left one ends first");
    }

    #[test]
    fn a_subtract_cuts_a_hole_out_of_what_was_added() {
        let ops = [
            (RectangleKind::Add, r(0, 0, 100, 50)),
            (RectangleKind::Subtract, r(0, 30, 100, 20)),
            (RectangleKind::Add, r(50, 0, 100, 10)),
        ];
        let got = disjoint(&ops);
        let area: i32 = got.iter().map(|r| r.size.w * r.size.h).sum();
        assert_eq!(area, 100 * 30 + 50 * 10, "{got:?}");
        for (i, a) in got.iter().enumerate() {
            for b in &got[i + 1..] {
                assert!(!a.overlaps(*b), "{a:?} and {b:?} overlap");
            }
        }
    }

    #[test]
    fn a_region_past_the_op_cap_is_the_bounding_box_of_its_adds() {
        let mut ops: Vec<Op> = (0..10_000)
            .map(|i| (RectangleKind::Add, r(i % 100 * 3, i / 100 * 3, 2, 2)))
            .collect();
        ops.push((RectangleKind::Subtract, r(0, 0, 50, 50)));
        let started = std::time::Instant::now();
        assert_eq!(disjoint(&ops), vec![r(0, 0, 299, 299)]);
        assert!(started.elapsed() < std::time::Duration::from_millis(100));
    }

    #[test]
    fn an_unchanged_region_keeps_its_ops_and_rects() {
        let ops = [(RectangleKind::Add, r(0, 0, 100, 50))];
        let mut cache = BlurOps::default();
        cache.update(&ops);
        let kept = (cache.ops.as_ptr(), cache.rects.as_ptr());
        cache.update(&ops);
        assert_eq!((cache.ops.as_ptr(), cache.rects.as_ptr()), kept);
        cache.update(&[(RectangleKind::Subtract, r(0, 0, 100, 50))]);
        assert!(cache.rects.is_empty(), "a changed op list is redone");
    }
}
