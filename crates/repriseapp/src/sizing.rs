//! Window size arithmetic: clamps, bounds, minimums and what a configure leaves.

use crate::{App, MAIN};

/// The largest window side the host draws, in logical px. The shm
/// buffer's `h * w * 4` then stays within `i32` (2^30 at most).
pub(crate) const MAX_SIDE: i32 = 16384;

const _: () = assert!((MAX_SIDE as i64) * (MAX_SIDE as i64) * 4 <= i32::MAX as i64);

/// The smallest a resizable main window may be made, logical px.
pub const MIN_SIZE: (i32, i32) = (120, 60);

/// A window side from the compositor or the program, clamped into
/// `1..=MAX_SIDE` so the canvas and the shm buffer stay allocatable.
pub(crate) fn sane_side(v: i64) -> i32 {
    v.clamp(1, i64::from(MAX_SIDE)) as i32
}

pub(crate) fn sane_size((w, h): (i32, i32)) -> (i32, i32) {
    (sane_side(i64::from(w)), sane_side(i64::from(h)))
}

/// `size` held inside `bounds`, side by side.
pub(crate) fn bounded(size: (i32, i32), bounds: Option<(i32, i32)>) -> (i32, i32) {
    match bounds {
        Some((bw, bh)) => (size.0.min(bw), size.1.min(bh)),
        None => size,
    }
}

/// The size a window of `now` takes for limits `min` and `max`: a fixed window (`max`) its
/// size; a resizable one at its program's pick the size the program picks now (`natural`),
/// else its own size; either no smaller than `min`.
pub(crate) fn regrown(
    now: (i32, i32),
    min: (i32, i32),
    max: Option<(i32, i32)>,
    natural: Option<(i32, i32)>,
) -> (i32, i32) {
    let size = natural.unwrap_or(now);
    max.unwrap_or((size.0.max(min.0), size.1.max(min.1)))
}

/// Whether a window of `size` is at the size its program picked (`picked`), held inside
/// `bounds`: neither the user nor the compositor sized it since.
pub(crate) fn at_pick(size: (i32, i32), picked: (i32, i32), bounds: Option<(i32, i32)>) -> bool {
    size == bounded(picked, bounds)
}

/// The size a window of `size` takes when its bounds go from `was` to `now`: one at its
/// program's pick under `was` stays at the pick, any other keeps its size; either held inside
/// `now`.
pub(crate) fn rebounded(
    size: (i32, i32),
    picked: (i32, i32),
    was: Option<(i32, i32)>,
    now: Option<(i32, i32)>,
) -> (i32, i32) {
    match at_pick(size, picked, was) {
        true => bounded(picked, now),
        false => bounded(size, now),
    }
}

/// The main window's opening client size and its floor: a resizable window
/// opens no smaller than its minimum, whatever size the program kept.
pub(crate) fn opening_size(app: &dyn App) -> ((i32, i32), (i32, i32)) {
    let size = sane_size(app.initial_size());
    if app.fixed() {
        return (size, size);
    }
    let min = app.min_size(MAIN).map_or(MIN_SIZE, sane_size);
    ((size.0.max(min.0), size.1.max(min.1)), min)
}

/// The size a configure leaves a window at, and whether the program
/// still owns it. `asked` is what the configure asks per side (`None`
/// leaves that side to the window), `last` what the one before asked.
/// A size the program picked (`own`) stands against a configure that
/// repeats `last` outside a resize drag: that one is about the window's
/// state, and the compositor still has the old size in it. Any other
/// size asked for is the compositor's, or the user's, and wins.
pub(crate) fn configured_size(
    now: (i32, i32),
    asked: (Option<i32>, Option<i32>),
    last: (Option<i32>, Option<i32>),
    own: bool,
    resizing: bool,
) -> ((i32, i32), bool) {
    if own && asked == last && !resizing {
        return (now, true);
    }
    ((asked.0.unwrap_or(now.0), asked.1.unwrap_or(now.1)), false)
}

/// The size a configure leaves a window at, held inside `bounds` on each side the window
/// picks itself (`asked` leaves it); a side the compositor asks for is its own.
pub(crate) fn held_size(
    size: (i32, i32),
    asked: (Option<i32>, Option<i32>),
    bounds: Option<(i32, i32)>,
) -> (i32, i32) {
    let held = bounded(size, bounds);
    (
        asked.0.map_or(held.0, |_| size.0),
        asked.1.map_or(held.1, |_| size.1),
    )
}

/// The integer scale a window of logical size `(w, h)` is drawn at: the
/// fractional scale rounded up when the compositor sends one, else its
/// buffer scale, lowered until neither side of the buffer passes
/// `MAX_SIDE`.
pub(crate) fn draw_scale(fraction_120: Option<u32>, buffer_scale: i32, (w, h): (i32, i32)) -> i32 {
    let want = fraction_120.map_or(buffer_scale, |n| n.div_ceil(120) as i32);
    want.min(MAX_SIDE / w.max(h).max(1)).max(1)
}
