//! A view onto content larger than it, with a bar down its right edge and one along its bottom.
//! A bar shows while the content reaches past its edge, and one bar can make the other needed.
//! The host owns the content: each call that scrolls returns how far the view's offsets moved,
//! and the host moves the content back by that. [`Frame`] says where the view lies and how far
//! the content reaches, at 100%; the bars lay out in px from the view's inside there.

use std::time::Instant;

use crate::access::{self, Tree, accesskit};
use crate::classic::widgets::{BarHit, LINE_H, ScrollParts, bar_hit};
use crate::controls::scrollbar::PageScroll;
use crate::look::Paint;
use crate::look::widgets::{draw_hscrollbar, draw_scrollbar};
use crate::raster::{Canvas, Rect};
use accesskit::{NodeId, Orientation, Role};

/// Where a view lies and what it shows, at 100%.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    /// The view's inside, the bars included.
    pub inside: Rect,
    /// How far right and down the content reaches with the view unscrolled.
    pub reach: (i32, i32),
    /// The bars' width.
    pub bar: i32,
    /// What the page around the inside adds to the length a bar measures, both edges together.
    pub edge: i32,
}

impl Frame {
    /// Which bars show: down the right edge, along the bottom.
    pub fn bars(&self) -> (bool, bool) {
        let (right, bottom) = (self.inside.right(), self.inside.bottom());
        let (reach, low) = self.reach;
        let v = low > bottom;
        let h = reach > right - if v { self.bar } else { 0 };
        (v || (h && low > bottom - self.bar), h)
    }

    /// The inside less the bars: where the content shows.
    pub fn clip(&self) -> Rect {
        let (v, h) = self.taken();
        let r = self.inside;
        Rect::new(r.x, r.y, r.width - v, r.height - h)
    }

    /// How far the view scrolls across and down.
    pub fn range(&self) -> (i32, i32) {
        let (v, h) = self.taken();
        let across = (self.reach.0 - (self.inside.right() - v)).max(0);
        let down = match self.bars().0 {
            true => (self.reach.1 - (self.inside.bottom() - h)).max(0),
            false => 0,
        };
        (across, down)
    }

    /// How much shows down the view and how long the content is, as the bar down it measures.
    pub fn span(&self) -> (i32, i32) {
        let visible = self.inside.height + self.edge - self.taken().1;
        (visible, visible + self.range().1)
    }

    /// [`Frame::span`] across.
    pub fn hspan(&self) -> (i32, i32) {
        let visible = self.inside.width + self.edge - self.taken().0;
        (visible, visible + self.range().0)
    }

    /// The width the bar down the view and the one along it take, 0 for one not shown.
    fn taken(&self) -> (i32, i32) {
        let (v, h) = self.bars();
        (if v { self.bar } else { 0 }, if h { self.bar } else { 0 })
    }
}

/// The view's rects in px on the surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    /// Where the content shows: the inside less the bars.
    pub clip: Rect,
    pub bar: Option<Rect>,
    pub hbar: Option<Rect>,
}

/// The view laid out in `inside`, its [`Frame::inside`] in px, with bars `w` px wide.
pub fn layout(f: &Frame, inside: Rect, w: i32) -> Layout {
    let (v, h) = f.bars();
    let (vw, hw) = (if v { w } else { 0 }, if h { w } else { 0 });
    let bar = Rect::new(inside.right() - w, inside.y, w, inside.height - hw);
    let hbar = Rect::new(inside.x, inside.bottom() - w, inside.width - vw, w);
    Layout {
        clip: Rect::new(inside.x, inside.y, inside.width - vw, inside.height - hw),
        bar: v.then_some(bar),
        hbar: h.then_some(hbar),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Down(BarHit),
    Across(BarHit),
}

/// How far the view's offsets moved, across and down, at 100%.
pub type Moved = (i32, i32);

#[derive(Clone, Debug, PartialEq)]
pub struct ScrollView {
    /// The scroll down; its `short` is the host's to keep.
    pub down: PageScroll,
    /// The scroll across, as `down`.
    pub across: PageScroll,
}

impl Default for ScrollView {
    fn default() -> ScrollView {
        ScrollView {
            down: PageScroll::default(),
            across: PageScroll::across(),
        }
    }
}

impl ScrollView {
    /// How far the view is scrolled across and down.
    pub fn offset(&self) -> (i32, i32) {
        (self.across.top, self.down.top)
    }

    /// Scroll so `top` of the content is above the view, held to its range.
    pub fn scroll_to(&mut self, f: &Frame, top: i32) -> i32 {
        let top = top.clamp(0, f.range().1);
        let dy = top - self.down.top;
        self.down.top = top;
        dy
    }

    /// Scroll so `left` of the content is left of the view, held to its range.
    pub fn scroll_across(&mut self, f: &Frame, left: i32) -> i32 {
        let left = left.clamp(0, f.range().0);
        let dx = left - self.across.top;
        self.across.top = left;
        dx
    }

    /// Scroll as little as it takes for `r`, a 100% rect where the content is now, to show
    /// whole in the view; its top-left shows when it is larger than the view.
    pub fn reveal(&mut self, f: &Frame, r: Rect) -> Moved {
        let c = f.clip();
        let down = PageScroll::reveal((c.y, c.bottom()), (r.y, r.bottom()));
        let across = PageScroll::reveal((c.x, c.right()), (r.x, r.right()));
        let dy = self.scroll_to(f, self.down.top + down);
        (self.scroll_across(f, self.across.top + across), dy)
    }

    pub fn hit(&self, f: &Frame, lay: &Layout, thumb_min: i32, x: i32, y: i32) -> Option<Hit> {
        if let Some(bar) = lay.bar.filter(|b| b.contains(x, y)) {
            let parts = self.bar_parts(f, bar, thumb_min);
            return Some(Hit::Down(bar_hit(&parts, x, y, false)));
        }
        let bar = lay.hbar.filter(|b| b.contains(x, y))?;
        let parts = self.hbar_parts(f, bar, thumb_min);
        Some(Hit::Across(bar_hit(&parts, x, y, true)))
    }

    /// The bar down the view laid out in `bar`.
    pub fn bar_parts(&self, f: &Frame, bar: Rect, thumb_min: i32) -> ScrollParts {
        let (visible, total) = f.span();
        self.down.parts(bar, visible, total, thumb_min)
    }

    /// The bar along the view laid out in `bar`.
    pub fn hbar_parts(&self, f: &Frame, bar: Rect, thumb_min: i32) -> ScrollParts {
        let (visible, total) = f.hspan();
        self.across.parts(bar, visible, total, thumb_min)
    }

    /// A press on a bar: an arrow scrolls a line, the track a view, the thumb is grabbed.
    pub fn press(&mut self, f: &Frame, hit: Hit) -> Moved {
        match hit {
            Hit::Down(h) => {
                let to = self.down.press(h, f.span().0);
                (0, self.scroll_to(f, to))
            }
            Hit::Across(h) => {
                let to = self.across.press(h, f.hspan().0);
                (self.scroll_across(f, to), 0)
            }
        }
    }

    /// The pointer at (`x`, `y`) while a thumb is held: the view follows it. None unless a thumb
    /// is held on a bar that shows.
    pub fn drag(
        &mut self,
        f: &Frame,
        lay: &Layout,
        thumb_min: i32,
        x: i32,
        y: i32,
    ) -> Option<Moved> {
        if self.across.holding() {
            let parts = self.hbar_parts(f, lay.hbar?, thumb_min);
            let (visible, total) = f.hspan();
            let left = self.across.drag(&parts, (x, y), visible, total)?;
            return Some((self.scroll_across(f, left), 0));
        }
        let parts = self.bar_parts(f, lay.bar?, thumb_min);
        let (visible, total) = f.span();
        let top = self.down.drag(&parts, (x, y), visible, total)?;
        Some((0, self.scroll_to(f, top)))
    }

    /// The pointer moved onto `hit`, None off the bars. True when a bar's look changed.
    pub fn motion(&mut self, hit: Option<Hit>) -> bool {
        let (down, across) = match hit {
            Some(Hit::Down(h)) => (Some(h), None),
            Some(Hit::Across(h)) => (None, Some(h)),
            None => (None, None),
        };
        self.down.motion(down) | self.across.motion(across)
    }

    pub fn leave(&mut self) -> bool {
        self.down.leave() | self.across.leave()
    }

    /// The pointer let go: true when a thumb was held or a bar's look changed.
    pub fn release(&mut self) -> bool {
        self.down.release() | self.across.release()
    }

    /// Whether the pointer holds a thumb or an arrow.
    pub fn holding(&self) -> bool {
        self.down.holding() || self.across.holding() || self.ticking()
    }

    /// Whether an arrow is held: the host sends [`ScrollView::tick`] until it lets go.
    pub fn ticking(&self) -> bool {
        self.down.ticking() || self.across.ticking()
    }

    /// A host tick at `now`: a held arrow steps once it repeats, while the pointer is on it.
    pub fn tick(&mut self, f: &Frame, now: Instant) -> Moved {
        let dy = (self.down.tick(now)).map_or(0, |to| self.scroll_to(f, to));
        let dx = (self.across.tick(now)).map_or(0, |to| self.scroll_across(f, to));
        (dx, dy)
    }

    /// The wheel turned `rows` rows (down positive): down a line a row, or across a view that
    /// scrolls only that way.
    pub fn wheel(&mut self, f: &Frame, rows: i32) -> Moved {
        if f.range().1 == 0 {
            return self.wheel_across(f, rows);
        }
        (0, self.scroll_to(f, self.down.top + rows * LINE_H))
    }

    pub fn wheel_across(&mut self, f: &Frame, rows: i32) -> Moved {
        (self.scroll_across(f, self.across.top + rows * LINE_H), 0)
    }

    pub fn draw(&self, c: &mut Canvas, p: &Paint, f: &Frame, lay: &Layout) {
        if let Some(bar) = lay.bar {
            let (visible, total) = f.span();
            let (top, lit) = (self.down.top as usize, self.down.lit());
            let (visible, total) = (visible as usize, total as usize);
            draw_scrollbar(c, p, bar.scaled(p.s), top, visible, total, lit);
        }
        if let Some(bar) = lay.hbar {
            let (visible, total) = f.hspan();
            let (left, lit) = (self.across.top as usize, self.across.lit());
            let (visible, total) = (visible as usize, total as usize);
            draw_hscrollbar(c, p, bar.scaled(p.s), left, visible, total, lit);
        }
    }

    /// The view's `access` part under `parent`: a scroll view node over the clip, which the host
    /// hangs the scrolled content under, and a scroll bar node per bar. Returns the view's node.
    pub fn access(
        &self,
        f: &Frame,
        lay: &Layout,
        tree: &mut Tree,
        parent: NodeId,
        key: &str,
    ) -> NodeId {
        let mut view = access::node(Role::ScrollView, lay.clip);
        view.set_clips_children();
        let id = tree.push(parent, (key, "view"), view);
        let (across, down) = f.range();
        let bars = [
            (lay.bar, Orientation::Vertical, self.down.top, down),
            (lay.hbar, Orientation::Horizontal, self.across.top, across),
        ];
        for (bar, orientation, at, range) in bars {
            let Some(bar) = bar else { continue };
            let mut n = access::node(Role::ScrollBar, bar);
            n.set_orientation(orientation);
            n.set_numeric_value(at.into());
            n.set_min_numeric_value(0.0);
            n.set_max_numeric_value(range.into());
            tree.push(parent, (key, orientation as u8), n);
        }
        id
    }
}

#[cfg(test)]
mod tests;
