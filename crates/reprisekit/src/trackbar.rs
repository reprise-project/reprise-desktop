//! A trackbar's model, layout, hit test and `access` part, shared by every look.

use crate::access::{self, Tree, accesskit};
use crate::raster::Rect;
use accesskit::{NodeId, Role};

/// The control's height: the thumb's 21 rows plus the ticks under it.
pub const HEIGHT: i32 = 26;
pub(crate) const THUMB_W: i32 = 11;
pub(crate) const THUMB_H: i32 = 21;
const CHANNEL_Y: i32 = 7;
const CHANNEL_H: i32 = 4;
const TRACK_INSET: i32 = 5;
/// A vertical trackbar's width with Classic's thumb: 3 px ticks (the first and last 4 px), a 1 px
/// gap, the 21 px thumb, a 1 px gap and the ticks again.
pub const VERTICAL_WIDTH: i32 = vertical_width(VTHUMB_W);
const VTHUMB_W: i32 = 21;
const VTHUMB_H: i32 = 11;
const VTHUMB_X: i32 = 5;

/// A vertical trackbar's width for a thumb `thumb_w` wide: the ticks and a gap either side of it.
pub const fn vertical_width(thumb_w: i32) -> i32 {
    thumb_w + 2 * VTHUMB_X
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrackBar {
    pub pos: usize,
    pub count: usize,
    pub enabled: bool,
    /// How far Page Up and Page Down move the thumb, in positions.
    pub page: usize,
    /// A tick under every this many positions, and under the last.
    pub tick_every: usize,
    /// Whether it has ticks at all.
    pub ticks: bool,
}

impl TrackBar {
    /// A trackbar of `count` positions at `pos`, a tick under each, paging by 3.
    pub fn new(pos: usize, count: usize) -> TrackBar {
        TrackBar {
            pos: pos.min(count.saturating_sub(1)),
            count: count.max(1),
            enabled: true,
            page: 3,
            tick_every: 1,
            ticks: true,
        }
    }

    /// The same trackbar with no ticks.
    pub fn untick(self) -> TrackBar {
        TrackBar {
            ticks: false,
            ..self
        }
    }

    /// The same trackbar paging by `page` positions, with a tick under every `tick_every`th
    /// position and the last.
    pub fn paged(self, page: usize, tick_every: usize) -> TrackBar {
        TrackBar {
            page,
            tick_every: tick_every.max(1),
            ..self
        }
    }

    /// Move by `delta` positions. Returns whether it moved.
    pub fn step(&mut self, delta: i32) -> bool {
        let last = self.count.max(1) as i32 - 1;
        let next = (self.pos as i32 + delta).clamp(0, last) as usize;
        let moved = next != self.pos;
        self.pos = next;
        moved
    }

    pub fn set(&mut self, pos: usize) -> bool {
        let next = pos.min(self.count.max(1) - 1);
        let moved = next != self.pos;
        self.pos = next;
        moved
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub bounds: Rect,
    pub channel: Rect,
    pub thumb: Rect,
    /// The x of each position (the thumb's centre there); the y on a vertical trackbar, position 0
    /// lowest.
    pub ticks: Vec<i32>,
    /// A tick is drawn under every this many positions, and the last.
    pub tick_every: usize,
    /// Whether ticks are drawn at all.
    pub ticked: bool,
    pub vertical: bool,
}

/// Each position's offset along a track `len` px long from position 0's end: rounded, or with
/// `floor` (a vertical track, counted up from its bottom) the fraction dropped.
fn offsets(tb: &TrackBar, len: i32, floor: bool) -> Vec<i32> {
    let track = (len - 2 * TRACK_INSET - 1).max(0);
    let n = tb.count.max(1) as i32;
    let half = if floor { 0 } else { n - 1 };
    (0..n)
        .map(|i| {
            TRACK_INSET
                + if n > 1 {
                    (i * track * 2 + half) / (2 * (n - 1))
                } else {
                    0
                }
        })
        .collect()
}

/// Lay a trackbar out with its top-left at (`x`, `y`) and its channel `w` wide.
pub fn layout(tb: &TrackBar, x: i32, y: i32, w: i32) -> Layout {
    let channel = Rect::new(x, y + CHANNEL_Y, w, CHANNEL_H);
    let ticks: Vec<i32> = offsets(tb, w, false).into_iter().map(|o| x + o).collect();
    let cx = ticks[tb.pos.min(ticks.len() - 1)];
    let thumb = Rect::new(cx - THUMB_W / 2, y, THUMB_W, THUMB_H);
    Layout {
        bounds: Rect::new(x, y, w, HEIGHT),
        channel,
        thumb,
        ticks,
        tick_every: tb.tick_every.max(1),
        ticked: tb.ticks,
        vertical: false,
    }
}

/// Lay a vertical trackbar out with its top-left at (`x`, `y`), its channel `h` tall and
/// [`VERTICAL_WIDTH`] wide, with Classic's thumb.
pub fn layout_vertical(tb: &TrackBar, x: i32, y: i32, h: i32) -> Layout {
    layout_vertical_with(tb, x, y, h, VTHUMB_W)
}

/// [`layout_vertical`] with a thumb `thumb_w` wide (a look's
/// [`ControlMetrics::vtrack_thumb`](crate::metrics::ControlMetrics::vtrack_thumb)): the trackbar is
/// [`vertical_width`] of it, and the channel stays down the thumb's middle.
pub fn layout_vertical_with(tb: &TrackBar, x: i32, y: i32, h: i32, thumb_w: i32) -> Layout {
    let channel = Rect::new(x + VTHUMB_X + (thumb_w - CHANNEL_H) / 2, y, CHANNEL_H, h);
    let bottom = y + h - 1;
    let ticks: Vec<i32> = offsets(tb, h, true)
        .into_iter()
        .map(|o| bottom - o)
        .collect();
    let cy = ticks[tb.pos.min(ticks.len() - 1)];
    let thumb = Rect::new(x + VTHUMB_X, cy - VTHUMB_H / 2, thumb_w, VTHUMB_H);
    Layout {
        bounds: Rect::new(x, y, vertical_width(thumb_w), h),
        channel,
        thumb,
        ticks,
        tick_every: tb.tick_every.max(1),
        ticked: tb.ticks,
        vertical: true,
    }
}

/// `lay` with its thumb at position `pos` (clamped to the last): what laying the trackbar out again
/// at `pos` gives.
pub fn at_position(lay: &Layout, pos: usize) -> Layout {
    let c = lay.ticks[pos.min(lay.ticks.len() - 1)];
    let mut t = lay.thumb;
    if lay.vertical {
        t.y = c - t.height / 2;
    } else {
        t.x = c - t.width / 2;
    }
    Layout {
        thumb: t,
        ..lay.clone()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Thumb,
    /// The channel on position 0's side of the thumb (left, or below on a vertical trackbar).
    Left,
    Right,
}

pub fn hit(lay: &Layout, x: i32, y: i32) -> Option<Hit> {
    if lay.thumb.contains(x, y) {
        return Some(Hit::Thumb);
    }
    if !lay.bounds.contains(x, y) {
        return None;
    }
    let before = if lay.vertical {
        y >= lay.thumb.bottom()
    } else {
        x < lay.thumb.x
    };
    Some(if before { Hit::Left } else { Hit::Right })
}

/// The position nearest a pointer x (its y on a vertical trackbar), for a drag.
pub fn position_at(lay: &Layout, x: i32) -> usize {
    lay.ticks
        .iter()
        .enumerate()
        .min_by_key(|&(_, &t)| (t - x).abs())
        .map_or(0, |(i, _)| i)
}

/// A vertical trackbar's ticks, logical: at every `every`th position and the last, one each side of
/// the thumb, `len` px long (`end_len` for the first and last), reaching out from a 1 px gap beside
/// the thumb rect.
pub fn vertical_ticks(lay: &Layout, every: usize, len: i32, end_len: i32) -> Vec<Rect> {
    if !lay.ticked {
        return Vec::new();
    }
    let (left, right) = (lay.bounds.x + 3, lay.bounds.right() - 4);
    let n = lay.ticks.len();
    let mut out = Vec::new();
    for (i, &ty) in lay.ticks.iter().enumerate() {
        let end = i == 0 || i + 1 == n;
        if i % every.max(1) != 0 && i + 1 != n {
            continue;
        }
        let len = if end { end_len } else { len };
        out.push(Rect::new(left + 1 - len, ty, len, 1));
        out.push(Rect::new(right, ty, len, 1));
    }
    out
}

/// The trackbar's `access` part: a slider at its tick index, 0 to the last tick.
pub fn access(
    tb: &TrackBar,
    lay: &Layout,
    tree: &mut Tree,
    parent: NodeId,
    key: impl std::hash::Hash,
) -> NodeId {
    let last = tb.count.saturating_sub(1) as f64;
    let mut n = access::range(
        Role::Slider,
        lay.bounds,
        tb.pos as f64,
        0.0,
        last,
        tb.enabled,
    );
    n.set_orientation(if lay.vertical {
        accesskit::Orientation::Vertical
    } else {
        accesskit::Orientation::Horizontal
    });
    tree.push(parent, key, n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_zero_count_trackbar_stays_at_position_zero() {
        let mut tb = TrackBar {
            pos: 0,
            count: 0,
            ..TrackBar::new(0, 1)
        };
        assert!(!tb.step(1));
        assert!(!tb.step(-1));
        assert!(!tb.set(3));
        assert_eq!(tb.pos, 0);
    }

    #[test]
    fn ticks_land_on_their_pixels() {
        let tb = TrackBar::new(13, 18);
        let l = layout(&tb, 69, 315, 71);
        assert_eq!(l.channel, Rect::new(69, 322, 71, 4));
        assert_eq!(
            l.ticks,
            vec![
                74, 78, 81, 85, 88, 92, 95, 99, 102, 106, 109, 113, 116, 120, 123, 127, 130, 134
            ]
        );
        assert_eq!(l.thumb, Rect::new(115, 315, 11, 21));
        assert_eq!(hit(&l, 120, 320), Some(Hit::Thumb));
        assert_eq!(hit(&l, 80, 320), Some(Hit::Left));
        assert_eq!(hit(&l, 130, 330), Some(Hit::Right));
        assert_eq!(position_at(&l, 100), 7);
        let one = layout(&TrackBar::new(0, 1), 0, 0, 71);
        assert_eq!(one.ticks, vec![5]);
    }

    #[test]
    fn a_vertical_trackbars_ticks_drop_the_fraction_from_the_bottom() {
        let l = layout_vertical(&TrackBar::new(0, 7), 486, 424, 75);
        assert_eq!(l.ticks, vec![493, 483, 472, 461, 451, 440, 429]);
        assert_eq!(l.channel, Rect::new(499, 424, 4, 75));
        assert_eq!(l.thumb, Rect::new(491, 488, 21, 11));
    }
}
