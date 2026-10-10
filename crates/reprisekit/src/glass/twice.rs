//! The check every drawn part painted at scale 2 is held to: its scale-1 design twice over.

use crate::raster::Canvas;
use crate::shape::centre;

/// What a design pixel of a part is, across and down: two pixels of one
/// kind along an axis (the same band, say) shade into each other, two of
/// different kinds (a line and the body beside it) meet at a step.
pub(super) type Kind<'a> = &'a dyn Fn(i32, i32) -> (i32, i32);

pub(super) const ONE_KIND: Kind<'static> = &|_, _| (0, 0);

/// Pixel (`x`, `y`) of `big`, a `w` x `h` (design px) frame at scale 2 whose
/// top left lies at `(0, top2)`, beside the scale-1 pixels round its
/// middle's place in `one` (its frame from `(0, top1)`): its own design
/// pixel and, along each axis, the next one its middle lies towards when
/// that is of its `kind`. Within their range, a level either side for
/// rounding, and with `exact`, where those pixels are one colour, exactly
/// it. Where the next one lies past the frame's end or is of another kind,
/// the range reaches on past its own pixel by half the step from the pixel
/// on its other side, when that one is of its kind.
#[track_caller]
pub(super) fn twice_over(
    (one, top1): (&Canvas, i32),
    (big, top2): (&Canvas, i32),
    (w, h): (i32, i32),
    (x, y): (i32, i32),
    (exact, kind): (bool, Kind),
    what: &str,
) {
    let (ox, oy) = (x / 2, y / 2);
    let own = kind(ox, oy);
    // Along one axis: the design pixels to range over and, for a pixel
    // past the last of its kind, the one inside it.
    let axis = |v: i32, len: i32, same: &dyn Fn(i32) -> bool| {
        let o = v / 2;
        let next = if centre(v, 2) < f64::from(o) {
            o - 1
        } else {
            o + 1
        };
        if (0..len).contains(&next) && same(next) {
            return (vec![o, next], None);
        }
        let inner = 2 * o - next;
        let inner = ((0..len).contains(&inner) && same(inner)).then_some(inner);
        (vec![o], inner)
    };
    let (xs, ix) = axis(x, w, &|sx| kind(sx, oy).0 == own.0);
    let (ys, iy) = axis(y, h, &|sy| kind(ox, sy).1 == own.1);
    let px = |sx: i32, sy: i32| one.get(sx, top1 + sy);
    let got = big.get(x, top2 + y);
    let around: Vec<u32> = ys
        .iter()
        .flat_map(|&sy| xs.iter().map(move |&sx| px(sx, sy)))
        .collect();
    if exact && around.iter().all(|&p| p == around[0]) {
        assert_eq!(got, around[0], "{what}: {got:08X}");
        return;
    }
    let mut runs = Vec::new();
    if let Some(ix) = ix {
        runs.extend(ys.iter().map(|&sy| (px(ox, sy), px(ix, sy))));
    }
    if let Some(iy) = iy {
        runs.extend(xs.iter().map(|&sx| (px(sx, oy), px(sx, iy))));
    }
    for sh in [24, 16, 8, 0] {
        let ch = |p: u32| ((p >> sh) & 0xFF) as i32;
        let on = runs.iter().map(|&(e, i)| ch(e) + (ch(e) - ch(i)) / 2);
        let lo = around
            .iter()
            .map(|&p| ch(p))
            .chain(on.clone())
            .min()
            .unwrap();
        let hi = around.iter().map(|&p| ch(p)).chain(on).max().unwrap();
        assert!(
            (lo - 1..=hi + 1).contains(&ch(got)),
            "{what}: {got:08X} channel {sh} outside {lo}..={hi}"
        );
    }
}
