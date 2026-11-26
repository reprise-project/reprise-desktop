//! A trackbar's model, layout, hit test and `access` part, shared by every look. A trackbar grows
//! with the text as the other small controls do: its layout takes the whole multiple
//! ([`TextUnits::small`](crate::theme::TextUnits::small)) and lays out the thumb, channel and ticks
//! that many times their 100% size, so the painters draw from the layout alone.

use crate::access::{self, Tree, accesskit};
use crate::raster::Rect;
use accesskit::{NodeId, Role};

/// The control's height at 100% text: the thumb's 21 rows plus the ticks under it.
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

/// A vertical trackbar's width at 100% text for a thumb `thumb_w` wide: the ticks and a gap either
/// side of it.
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
    /// The position of 100 % on a volume slider that runs past it; the positions above are drawn
    /// shaded.
    pub full_at: Option<usize>,
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
            full_at: None,
        }
    }

    /// The same trackbar shaded past position `full_at`, which stands for 100 %; `None` is plain.
    pub fn boosted(self, full_at: Option<usize>) -> TrackBar {
        TrackBar { full_at, ..self }
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
    /// [`TrackBar::full_at`].
    pub full_at: Option<usize>,
    /// The whole multiple of its 100% size the thumb, channel and ticks are laid out at.
    pub grow: i32,
}

/// How far before its centre a thumb `len` px long starts when it is `k` times a 100% thumb: the
/// 100% thumb's half, grown, so the grown thumb's middle columns sit on the grown tick.
fn half(len: i32, k: i32) -> i32 {
    k * (len / k / 2)
}

/// Each position's offset along a track `len` px long from position 0's end, `inset` px in from
/// either end, for ticks `k` px thick: rounded, or with `floor` (a vertical track, counted up from
/// its bottom) the fraction dropped.
fn offsets(tb: &TrackBar, len: i32, floor: bool, inset: i32, k: i32) -> Vec<i32> {
    let track = (len - 2 * inset - k).max(0);
    let n = tb.count.max(1) as i32;
    let half = if floor { 0 } else { n - 1 };
    (0..n)
        .map(|i| {
            inset
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
    layout_grown(tb, x, y, w, 1)
}

/// [`layout`] with the thumb, channel, ticks and [`HEIGHT`] `k` times their 100% size.
pub fn layout_grown(tb: &TrackBar, x: i32, y: i32, w: i32, k: i32) -> Layout {
    let k = k.max(1);
    let channel = Rect::new(x, y + k * CHANNEL_Y, w, k * CHANNEL_H);
    let ticks: Vec<i32> = offsets(tb, w, false, k * TRACK_INSET, k)
        .into_iter()
        .map(|o| x + o)
        .collect();
    let cx = ticks[tb.pos.min(ticks.len() - 1)];
    let (tw, th) = (k * THUMB_W, k * THUMB_H);
    let thumb = Rect::new(cx - half(tw, k), y, tw, th);
    Layout {
        bounds: Rect::new(x, y, w, k * HEIGHT),
        channel,
        thumb,
        ticks,
        tick_every: tb.tick_every.max(1),
        ticked: tb.ticks,
        vertical: false,
        full_at: tb.full_at,
        grow: k,
    }
}

/// Lay a vertical trackbar out with its top-left at (`x`, `y`), its channel `h` tall and
/// [`VERTICAL_WIDTH`] wide, with Classic's thumb.
pub fn layout_vertical(tb: &TrackBar, x: i32, y: i32, h: i32) -> Layout {
    layout_vertical_with(tb, x, y, h, VTHUMB_W, 1)
}

/// [`layout_vertical`] with a thumb `thumb_w` wide (a look's
/// [`ControlMetrics::vtrack_thumb`](crate::metrics::ControlMetrics::vtrack_thumb)), grown `k`
/// times: the trackbar is `k` times [`vertical_width`] of it, and the channel stays down the
/// thumb's middle.
pub fn layout_vertical_with(tb: &TrackBar, x: i32, y: i32, h: i32, thumb_w: i32, k: i32) -> Layout {
    layout_vertical_sized(tb, x, y, h, (thumb_w, VTHUMB_H), k)
}

/// [`layout_vertical_with`] with a thumb `w` x `h` at 100% text, the thumb, channel and ticks `k`
/// times their 100% size and the trackbar `k` times [`vertical_width`] wide: the end positions
/// stay far enough in that the thumb never leaves the channel's ends.
pub fn layout_vertical_sized(
    tb: &TrackBar,
    x: i32,
    y: i32,
    h: i32,
    (thumb_w, thumb_h): (i32, i32),
    k: i32,
) -> Layout {
    let k = k.max(1);
    let (thumb_w, thumb_h) = (k * thumb_w, k * thumb_h);
    let channel = Rect::new(
        x + k * VTHUMB_X + k * ((thumb_w / k - CHANNEL_H) / 2),
        y,
        k * CHANNEL_H,
        h,
    );
    // A tick's top row: a grown tick is `k` rows tall.
    let bottom = y + h - k;
    let ticks: Vec<i32> = offsets(tb, h, true, (k * TRACK_INSET).max(half(thumb_h, k)), k)
        .into_iter()
        .map(|o| bottom - o)
        .collect();
    let cy = ticks[tb.pos.min(ticks.len() - 1)];
    let thumb = Rect::new(x + k * VTHUMB_X, cy - half(thumb_h, k), thumb_w, thumb_h);
    Layout {
        bounds: Rect::new(x, y, k * vertical_width(thumb_w / k), h),
        channel,
        thumb,
        ticks,
        tick_every: tb.tick_every.max(1),
        ticked: tb.ticks,
        vertical: true,
        full_at: tb.full_at,
        grow: k,
    }
}

/// `lay` with its thumb at position `pos` (clamped to the last): what laying the trackbar out again
/// at `pos` gives.
pub fn at_position(lay: &Layout, pos: usize) -> Layout {
    let c = lay.ticks[pos.min(lay.ticks.len() - 1)];
    let mut t = lay.thumb;
    if lay.vertical {
        t.y = c - half(t.height, lay.grow);
    } else {
        t.x = c - half(t.width, lay.grow);
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
/// the thumb rect; each length, the gap and the tick's 1 px row grow by [`Layout::grow`].
pub fn vertical_ticks(lay: &Layout, every: usize, len: i32, end_len: i32) -> Vec<Rect> {
    if !lay.ticked {
        return Vec::new();
    }
    let k = lay.grow;
    let (left, right) = (lay.bounds.x + 4 * k, lay.bounds.right() - 4 * k);
    let n = lay.ticks.len();
    let mut out = Vec::new();
    for (i, &ty) in lay.ticks.iter().enumerate() {
        let end = i == 0 || i + 1 == n;
        if i % every.max(1) != 0 && i + 1 != n {
            continue;
        }
        let len = k * if end { end_len } else { len };
        out.push(Rect::new(left - len, ty, len, k));
        out.push(Rect::new(right, ty, len, k));
    }
    out
}

/// A vertical trackbar's boost zone, logical: the channel above the 100 % position and the inner
/// columns it is shaded in, then a mark either side of the thumb at 100 %, as long as an end tick.
/// None on a plain or horizontal trackbar.
pub fn boost_zone(lay: &Layout, end_len: i32) -> Option<(Rect, [Rect; 2])> {
    let full = *lay.ticks.get(lay.full_at?)?;
    if !lay.vertical {
        return None;
    }
    let (ch, k) = (lay.channel, lay.grow);
    let shade = Rect::new(
        ch.x + k,
        ch.y + k,
        (ch.width - 2 * k).max(0),
        (full - ch.y).max(0),
    );
    let (left, right) = (lay.bounds.x + 4 * k, lay.bounds.right() - 4 * k);
    let len = k * end_len;
    Some((
        shade,
        [
            Rect::new(left - len, full, len, k),
            Rect::new(right, full, len, k),
        ],
    ))
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
    fn the_boost_zone_runs_from_the_channel_top_to_the_100_percent_mark() {
        let tb = TrackBar::new(0, 31).boosted(Some(20));
        let l = layout_vertical(&tb, 10, 10, 140);
        let full = l.ticks[20];
        let (shade, marks) = boost_zone(&l, 4).expect("a boosted vertical trackbar");
        assert_eq!(shade, Rect::new(l.channel.x + 1, 11, 2, full - 10));
        assert_eq!(marks[0], Rect::new(10, full, 4, 1));
        assert_eq!(marks[1], Rect::new(l.bounds.right() - 4, full, 4, 1));
        assert_eq!(
            boost_zone(&layout_vertical(&TrackBar::new(0, 21), 10, 10, 140), 4),
            None
        );
        assert_eq!(boost_zone(&layout(&tb, 0, 0, 100), 4), None);
    }

    #[test]
    fn a_grown_layout_doubles_the_thumb_channel_ticks_and_height() {
        let tb = TrackBar::new(3, 11);
        let one = layout(&tb, 0, 0, 71);
        let two = layout_grown(&tb, 0, 0, 142, 2);
        assert_eq!(two.thumb, Rect::new(2 * one.thumb.x, 0, 22, 42));
        assert_eq!(two.channel, Rect::new(0, 14, 142, 8));
        assert_eq!(two.bounds.height, 2 * HEIGHT);
        assert_eq!(
            two.ticks,
            one.ticks.iter().map(|t| 2 * t).collect::<Vec<_>>()
        );
        let last = at_position(&two, 10).thumb;
        assert_eq!(
            last,
            layout_grown(&TrackBar::new(10, 11), 0, 0, 142, 2).thumb
        );
        assert_eq!(
            last.right(),
            142,
            "the last thumb ends on the channel's end"
        );

        let up = TrackBar::new(2, 7).boosted(Some(5));
        let one = layout_vertical_with(&up, 0, 0, 71, 21, 1);
        let two = layout_vertical_with(&up, 0, 0, 142, 21, 2);
        assert_eq!(two.bounds, Rect::new(0, 0, 2 * VERTICAL_WIDTH, 142));
        assert_eq!(two.channel, Rect::new(2 * one.channel.x, 0, 8, 142));
        assert_eq!(two.thumb, Rect::new(10, 2 * one.thumb.y, 42, 22));
        assert_eq!(
            two.ticks,
            one.ticks.iter().map(|t| 2 * t).collect::<Vec<_>>()
        );
        let double = |r: &Rect| Rect::new(2 * r.x, 2 * r.y, 2 * r.width, 2 * r.height);
        assert_eq!(
            vertical_ticks(&two, 1, 3, 4),
            vertical_ticks(&one, 1, 3, 4)
                .iter()
                .map(double)
                .collect::<Vec<_>>()
        );
        let (shade, marks) = boost_zone(&two, 4).expect("a boosted trackbar");
        let (shade_1, marks_1) = boost_zone(&one, 4).expect("a boosted trackbar");
        assert_eq!(shade, double(&shade_1));
        assert_eq!(marks, marks_1.map(|r| double(&r)));
    }

    #[test]
    fn a_vertical_trackbars_ticks_drop_the_fraction_from_the_bottom() {
        let l = layout_vertical(&TrackBar::new(0, 7), 486, 424, 75);
        assert_eq!(l.ticks, vec![493, 483, 472, 461, 451, 440, 429]);
        assert_eq!(l.channel, Rect::new(499, 424, 4, 75));
        assert_eq!(l.thumb, Rect::new(491, 488, 21, 11));
    }
}
