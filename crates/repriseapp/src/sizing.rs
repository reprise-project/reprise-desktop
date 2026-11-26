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

/// The size a window of `now` takes for limits `min` and `max`, and the size it was given that
/// `min` holds it above: a fixed window (`max`) its size; a resizable one at its program's pick
/// the size the program picks now (`natural`); any other the size the user or the compositor
/// gave it (`given`, else `now`). Each is no smaller than `min`, and a given size `min` outgrew
/// comes back once `min` drops.
pub(crate) fn regrown(
    now: (i32, i32),
    given: Option<(i32, i32)>,
    min: (i32, i32),
    max: Option<(i32, i32)>,
    natural: Option<(i32, i32)>,
) -> ((i32, i32), Option<(i32, i32)>) {
    let grown = |(w, h): (i32, i32)| (w.max(min.0), h.max(min.1));
    match (max, natural) {
        (Some(max), _) => (max, None),
        (None, Some(natural)) => (grown(natural), None),
        (None, None) => {
            let given = given.unwrap_or(now);
            let size = grown(given);
            (size, (size != given).then_some(given))
        }
    }
}

/// Whether a window of `size` is at the size its program picked (`picked`), held inside
/// `bounds`: neither the user nor the compositor sized it since.
pub(crate) fn at_pick(size: (i32, i32), picked: (i32, i32), bounds: Option<(i32, i32)>) -> bool {
    size == bounded(picked, bounds)
}

/// The size a window of `size` takes when its bounds go from `was` to `now`, side by side: a
/// side at its program's pick under `was` stays at the pick, any other keeps its length; either
/// held inside `now`.
pub(crate) fn rebounded(
    size: (i32, i32),
    picked: (i32, i32),
    was: Option<(i32, i32)>,
    now: Option<(i32, i32)>,
) -> (i32, i32) {
    let held = bounded(picked, was);
    let side = |v: i32, pick: i32, held: i32| if v == held { pick } else { v };
    bounded(
        (
            side(size.0, picked.0, held.0),
            side(size.1, picked.1, held.1),
        ),
        now,
    )
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

/// What a window's configures leave it at: its size, the program's pick, the compositor's
/// last bounds and asked size, whether the size is the program's (`own`), the size the
/// program last sized the window away from (`left`), and the size the user gave it that its
/// minimum holds it above (`given`, see [`regrown`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Fit {
    pub size: (i32, i32),
    pub picked: (i32, i32),
    pub bounds: Option<(i32, i32)>,
    pub asked: (Option<i32>, Option<i32>),
    pub own: bool,
    pub left: Option<(i32, i32)>,
    pub given: Option<(i32, i32)>,
}

impl Fit {
    /// The fit after a configure asking `asked` with `bounds`. A size new bounds alone give
    /// ([`rebounded`]) is the window's own, so configures that repeat the old ask leave it. A
    /// configure the compositor sent before it saw the program's own size asks for `left`,
    /// which leaves the window's own size too. Only a size the compositor gives drops `given`.
    pub(crate) fn configured(
        self,
        asked: (Option<i32>, Option<i32>),
        bounds: Option<(i32, i32)>,
        resizing: bool,
    ) -> Fit {
        let stale = self.left.map(|(w, h)| (Some(w), Some(h))) == Some(asked);
        let last = if stale { asked } else { self.asked };
        let (size, mut own) = configured_size(self.size, asked, last, self.own, resizing);
        let moved_bounds = bounds.is_some() && bounds != self.bounds;
        let size = match moved_bounds && size == self.size {
            true => {
                let to = rebounded(size, self.picked, self.bounds, bounds);
                own |= to != size;
                to
            }
            false => size,
        };
        let bounds = bounds.or(self.bounds);
        let size = held_size(size, asked, bounds);
        Fit {
            size,
            bounds,
            asked,
            own,
            left: self.left.filter(|_| own),
            given: self.given.filter(|_| own || size == self.size),
            ..self
        }
    }

    /// The fit after the program's limits `min` and `max` or its pick `natural` changed: a
    /// fixed window takes its size; a resizable one still at its program's pick takes the size
    /// the program picks now, one the user sized the size they gave it ([`regrown`]); both at
    /// least `min`. A new size is the program's own, sized away from the old one (`left`).
    pub(crate) fn refitted(
        self,
        min: (i32, i32),
        max: Option<(i32, i32)>,
        natural: Option<(i32, i32)>,
    ) -> Fit {
        let natural = natural.filter(|_| at_pick(self.size, self.picked, self.bounds));
        let (size, given) = regrown(self.size, self.given, min, max, natural);
        let size = bounded(size, self.bounds);
        let picked = match max {
            Some(_) => size,
            None => natural.unwrap_or(self.picked),
        };
        let fit = Fit {
            picked,
            given,
            ..self
        };
        match size == self.size {
            true => fit,
            false => Fit {
                size,
                own: true,
                left: Some(self.size),
                ..fit
            },
        }
    }
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
