//! Image view: one picture in a box, fitted or zoomed, turned in quarter turns and panned under
//! scrollbars. Zoom and pan are in device px, so 100% is one picture pixel per screen pixel.

use crate::access::{self, Tree, accesskit};
use crate::classic::widgets::{BarHit, ScrollParts, axis_lit, bar_hit, scrollbar_parts};
use crate::controls::scrollbar::ThumbDrag;
use crate::look::widgets::{draw_hscrollbar, draw_scroll_corner, draw_scrollbar};
use crate::metrics::ControlMetrics;
use crate::paint::Paint;
use crate::pointer::{Axis, Pointer, ScrollHit};
use crate::raster::{Canvas, Rect};
use crate::text::{Align, TextRenderer};
use crate::theme::Color;
use accesskit::{NodeId, Role};

/// The zoom steps `+` and `-` go through, device px per picture px.
pub const ZOOMS: [f64; 23] = [
    0.01, 0.02, 0.03, 0.05, 0.08, 0.1, 0.15, 0.2, 0.25, 0.33, 0.5, 0.67, 1.0, 1.5, 2.0, 3.0, 4.0,
    6.0, 8.0, 12.0, 16.0, 24.0, 32.0,
];
/// Device px an arrow on a scrollbar, or an arrow key, pans.
pub const LINE: i32 = 32;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Zoom {
    /// As large as the box holds, never past 100%.
    Fit,
    /// Device px per picture px.
    Scale(f64),
}

pub struct ImageView {
    /// The picture's own size, before any turn.
    pub size: (u32, u32),
    pub zoom: Zoom,
    /// Quarter turns clockwise, 0 to 3.
    pub turns: u8,
    /// Device px of the zoomed picture scrolled off the box's left and top.
    pub left: i32,
    pub top: i32,
    pub bars: Pointer<(Axis, ScrollHit)>,
    thumb: ThumbDrag,
    /// A drag of the picture itself: where the press was (logical) and the pan then.
    grab: Option<((i32, i32), (i32, i32))>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    /// The box the picture shows in, logical px, the scrollbars left out.
    pub view: Rect,
    pub vscroll: Option<Rect>,
    pub hscroll: Option<Rect>,
    /// Device px per picture px.
    pub scale: f64,
    /// The turned picture at `scale`, device px.
    pub content: (i32, i32),
    /// Device px per logical px.
    pub s: i32,
    /// The bars' thickness, logical px, whether they show or not.
    bar: i32,
    thumb_min: i32,
}

impl Layout {
    /// The box in device px.
    pub fn device_view(&self) -> Rect {
        self.view.scaled(self.s)
    }

    /// Where the bars meet, when both show.
    pub fn corner(&self) -> Option<Rect> {
        match (self.vscroll, self.hscroll) {
            (Some(v), Some(h)) => Some(Rect::new(v.x, h.y, v.width, h.height)),
            _ => None,
        }
    }

    /// How far the picture pans across and down, device px.
    pub fn range(&self) -> (i32, i32) {
        let b = self.device_view();
        (
            (self.content.0 - b.width).max(0),
            (self.content.1 - b.height).max(0),
        )
    }
}

/// The largest scale at which `(w, h)` fits `room`, held to 1 so a small picture stays at its
/// own size; 1 for an empty picture or room.
pub fn fit_scale((w, h): (u32, u32), room: (i32, i32)) -> f64 {
    if w == 0 || h == 0 || room.0 <= 0 || room.1 <= 0 {
        return 1.0;
    }
    (room.0 as f64 / w as f64)
        .min(room.1 as f64 / h as f64)
        .min(1.0)
}

/// The zoom step after `scale` going in (`up`) or out; `scale` itself at either end.
pub fn step(scale: f64, up: bool) -> f64 {
    const EPS: f64 = 1e-6;
    let next = if up {
        ZOOMS.iter().copied().find(|z| *z > scale + EPS)
    } else {
        ZOOMS.iter().rev().copied().find(|z| *z < scale - EPS)
    };
    next.unwrap_or(scale)
}

/// `(w, h)` after `turns` quarter turns.
pub fn turned((w, h): (u32, u32), turns: u8) -> (u32, u32) {
    if turns % 2 == 1 { (h, w) } else { (w, h) }
}

/// The pixel of a `w` x `h` picture shown at `(x, y)` of it turned `turns` quarter turns
/// clockwise.
pub fn source_of(x: u32, y: u32, (w, h): (u32, u32), turns: u8) -> (u32, u32) {
    match turns % 4 {
        0 => (x, y),
        1 => (y, h - 1 - x),
        2 => (w - 1 - x, h - 1 - y),
        _ => (w - 1 - y, x),
    }
}

/// `src` turned `turns` quarter turns clockwise and box-filtered to `(w, h)`: the copy a host
/// makes off its UI thread for a zoom under 100%, which [`draw`] then shows pixel for pixel.
pub fn render(src: &Canvas, turns: u8, (w, h): (u32, u32)) -> Canvas {
    let size = (src.width, src.height);
    let (tw, th) = turned(size, turns);
    let sw = src.width as usize;
    Canvas::resample_box_from(tw, th, w, h, |x, y| {
        let (sx, sy) = source_of(x, y, size, turns);
        src.pixels[sy as usize * sw + sx as usize]
    })
}

impl ImageView {
    pub fn new(size: (u32, u32)) -> ImageView {
        ImageView {
            size,
            zoom: Zoom::Fit,
            turns: 0,
            left: 0,
            top: 0,
            bars: Pointer::new(),
            thumb: ThumbDrag::default(),
            grab: None,
        }
    }

    /// The picture's size as shown, after the turn.
    pub fn shown_size(&self) -> (u32, u32) {
        turned(self.size, self.turns)
    }

    /// Back to fitted, unturned and unpanned, for another picture of `size`.
    pub fn reset(&mut self, size: (u32, u32)) {
        *self = ImageView::new(size);
    }

    /// Turn a quarter turn clockwise (`cw`) or back. The pan restarts.
    pub fn turn(&mut self, cw: bool) {
        self.turns = (self.turns + if cw { 1 } else { 3 }) % 4;
        self.left = 0;
        self.top = 0;
    }

    /// Zoom to `scale` keeping the picture point under logical `at` there (the box's middle when
    /// `None`).
    pub fn zoom_to(&mut self, scale: f64, at: Option<(i32, i32)>, lay: &Layout) {
        let scale = scale.clamp(ZOOMS[0], ZOOMS[ZOOMS.len() - 1]);
        let b = lay.device_view();
        let (ax, ay) = at.map_or((b.width / 2, b.height / 2), |(x, y)| {
            (x * lay.s - b.x, y * lay.s - b.y)
        });
        let (ox, oy) = self.origin(lay);
        // The picture point (in turned picture px) under the anchor.
        let ux = (b.x + ax - ox) as f64 / lay.scale;
        let uy = (b.y + ay - oy) as f64 / lay.scale;
        self.zoom = Zoom::Scale(scale);
        self.left = (ux * scale).round() as i32 - ax;
        self.top = (uy * scale).round() as i32 - ay;
        let lay = self.relaid(lay);
        self.hold(&lay);
    }

    /// One [`ZOOMS`] step in or out around `at`.
    pub fn zoom_step(&mut self, up: bool, at: Option<(i32, i32)>, lay: &Layout) {
        self.zoom_to(step(lay.scale, up), at, lay);
    }

    /// Fitted to the box again.
    pub fn fit(&mut self) {
        self.zoom = Zoom::Fit;
        self.left = 0;
        self.top = 0;
    }

    /// Pan by device px, held to the picture. True when it moved.
    pub fn pan(&mut self, dx: i32, dy: i32, lay: &Layout) -> bool {
        let was = (self.left, self.top);
        self.left += dx;
        self.top += dy;
        self.hold(lay);
        was != (self.left, self.top)
    }

    fn hold(&mut self, lay: &Layout) {
        let (mx, my) = lay.range();
        self.left = self.left.clamp(0, mx);
        self.top = self.top.clamp(0, my);
    }

    /// `lay` again for this view's zoom: the bars may come or go with it.
    fn relaid(&self, lay: &Layout) -> Layout {
        let outer = Rect::new(
            lay.view.x,
            lay.view.y,
            lay.view.width + lay.vscroll.map_or(0, |r| r.width),
            lay.view.height + lay.hscroll.map_or(0, |r| r.height),
        );
        layout_with(self, outer, lay.s, lay.bar, lay.thumb_min)
    }

    /// The turned, zoomed picture's top-left in device px: centred on an axis it fits, panned on
    /// one it overflows.
    pub fn origin(&self, lay: &Layout) -> (i32, i32) {
        let b = lay.device_view();
        let at = |start: i32, room: i32, len: i32, pan: i32| {
            if len <= room {
                start + (room - len) / 2
            } else {
                start - pan.clamp(0, len - room)
            }
        };
        (
            at(b.x, b.width, lay.content.0, self.left),
            at(b.y, b.height, lay.content.1, self.top),
        )
    }

    /// The picture's rect in logical px, cut to the box.
    pub fn picture_rect(&self, lay: &Layout) -> Rect {
        let (x, y) = self.origin(lay);
        let s = lay.s.max(1);
        let r = Rect::new(
            x.div_euclid(s),
            y.div_euclid(s),
            (lay.content.0 + s - 1) / s,
            (lay.content.1 + s - 1) / s,
        );
        r.intersect(&lay.view)
    }

    /// The scrollbar part under logical `(x, y)`, for lighting.
    pub fn bar_at(&self, lay: &Layout, x: i32, y: i32) -> Option<(Axis, ScrollHit)> {
        bar_press_at(self, lay, x, y).and_then(|(a, h)| Some((a, h.part()?)))
    }

    /// A left press at logical `(x, y)`: a scrollbar steps, pages or takes its thumb, and a press
    /// on a picture larger than the box grabs it to drag. True when the view changed.
    pub fn press(&mut self, x: i32, y: i32, lay: &Layout) -> bool {
        if let Some((axis, hit)) = bar_press_at(self, lay, x, y) {
            let lit = self.bars.press_bar(hit.part().map(|p| (axis, p)));
            let b = lay.device_view();
            let page = if axis == Axis::Horizontal {
                b.width
            } else {
                b.height
            };
            let by = self.thumb.press(axis, hit).by(LINE as i64, page as i64);
            let moved = match axis {
                Axis::Horizontal => self.pan(by as i32, 0, lay),
                Axis::Vertical => self.pan(0, by as i32, lay),
            };
            return moved || lit;
        }
        if lay.view.contains(x, y) && lay.range() != (0, 0) {
            self.grab = Some(((x, y), (self.left, self.top)));
        }
        false
    }

    /// The pointer at logical `(x, y)`: a held thumb or a grabbed picture pans, else the bars
    /// light. True when the view changed.
    pub fn motion(&mut self, x: i32, y: i32, lay: &Layout) -> bool {
        if self.thumb.holding() {
            for axis in [Axis::Horizontal, Axis::Vertical] {
                let Some((parts, visible, total)) = span(self, lay, axis) else {
                    continue;
                };
                if let Some(to) = self.thumb.pos(axis, &parts, (x, y), visible, total) {
                    let was = (self.left, self.top);
                    match axis {
                        Axis::Horizontal => self.left = to as i32,
                        Axis::Vertical => self.top = to as i32,
                    }
                    self.hold(lay);
                    return was != (self.left, self.top);
                }
            }
            return false;
        }
        if let Some(((px, py), (l, t))) = self.grab {
            let was = (self.left, self.top);
            self.left = l - (x - px) * lay.s;
            self.top = t - (y - py) * lay.s;
            self.hold(lay);
            return was != (self.left, self.top);
        }
        self.bars.motion(self.bar_at(lay, x, y))
    }

    /// The left button's release. True when a lit part changed.
    pub fn release(&mut self) -> bool {
        self.grab = None;
        self.thumb.release();
        self.bars.release_bar()
    }

    pub fn leave(&mut self) -> bool {
        self.bars.leave()
    }

    /// Whether a drag of the picture or a thumb is under way.
    pub fn dragging(&self) -> bool {
        self.grab.is_some() || self.thumb.holding()
    }
}

/// Lay `v` out in logical `r` at `s` device px per logical px, its bars `cm.scroll` thick.
pub fn layout(v: &ImageView, r: Rect, s: i32, cm: &ControlMetrics) -> Layout {
    layout_with(v, r, s.max(1), cm.scroll, cm.thumb_min)
}

fn layout_with(v: &ImageView, r: Rect, s: i32, bar: i32, thumb_min: i32) -> Layout {
    let shown = v.shown_size();
    let (bw, bh) = (r.width * s, r.height * s);
    let scale = match v.zoom {
        Zoom::Fit => fit_scale(shown, (bw, bh)),
        Zoom::Scale(z) => z,
    };
    let len = |n: u32| ((n as f64 * scale).round() as i32).max(1);
    let content = (len(shown.0), len(shown.1));
    let (mut vbar, mut hbar) = (false, false);
    if v.zoom != Zoom::Fit {
        vbar = content.1 > bh;
        hbar = content.0 > bw - if vbar { bar * s } else { 0 };
        vbar = vbar || (hbar && content.1 > bh - bar * s);
    }
    let w = r.width - if vbar { bar } else { 0 };
    let h = r.height - if hbar { bar } else { 0 };
    Layout {
        view: Rect::new(r.x, r.y, w.max(0), h.max(0)),
        vscroll: vbar.then(|| Rect::new(r.x + w, r.y, bar, h.max(0))),
        hscroll: hbar.then(|| Rect::new(r.x, r.y + h, w.max(0), bar)),
        scale,
        content,
        s,
        bar,
        thumb_min,
    }
}

/// The `axis` bar's parts in logical px and its visible and total device px.
fn span(v: &ImageView, lay: &Layout, axis: Axis) -> Option<(ScrollParts, usize, usize)> {
    let b = lay.device_view();
    let (rect, pos, visible, total, horizontal) = match axis {
        Axis::Horizontal => (lay.hscroll?, v.left, b.width, lay.content.0, true),
        Axis::Vertical => (lay.vscroll?, v.top, b.height, lay.content.1, false),
    };
    let (visible, total) = (visible.max(0) as usize, total.max(0) as usize);
    let parts = scrollbar_parts(
        rect,
        1,
        pos.max(0) as usize,
        visible,
        total,
        horizontal,
        lay.thumb_min,
    );
    Some((parts, visible, total))
}

fn bar_press_at(v: &ImageView, lay: &Layout, x: i32, y: i32) -> Option<(Axis, BarHit)> {
    for (axis, rect) in [
        (Axis::Vertical, lay.vscroll),
        (Axis::Horizontal, lay.hscroll),
    ] {
        if !rect.is_some_and(|r| r.contains(x, y)) {
            continue;
        }
        let (parts, _, _) = span(v, lay, axis)?;
        return match bar_hit(&parts, x, y, axis == Axis::Horizontal) {
            BarHit::Track => None,
            hit => Some((axis, hit)),
        };
    }
    None
}

/// What [`draw`] shows the picture from: the decoded picture, and a copy of it already turned
/// and scaled to the layout's `content`, when the host has one ([`render`]).
#[derive(Clone, Copy)]
pub struct Source<'a> {
    pub picture: &'a Canvas,
    pub rendered: Option<&'a Canvas>,
}

/// Draw the box on `ground` with the picture over it, then the scrollbars. A `rendered` copy of
/// the layout's size is copied as it is; otherwise the nearest picture pixel shows.
pub fn draw(c: &mut Canvas, p: &Paint, v: &ImageView, src: Source, lay: &Layout, ground: Color) {
    c.fill_rect(lay.view, ground);
    let (ox, oy) = v.origin(lay);
    let b = lay.device_view();
    let pic = Rect::new(ox, oy, lay.content.0, lay.content.1).intersect(&b);
    let picture = src.picture;
    let size = (picture.width, picture.height);
    let shown = v.shown_size();
    if !pic.is_empty() && size.0 > 0 && size.1 > 0 {
        let rendered = src
            .rendered
            .filter(|r| (r.width as i32, r.height as i32) == lay.content && r.scale() == 1);
        match rendered {
            Some(r) => {
                let w = r.width as usize;
                c.compose_device(pic, |x, y| {
                    r.pixels[(y - oy) as usize * w + (x - ox) as usize]
                });
            }
            None => {
                let sw = picture.width as usize;
                let scale = lay.scale;
                let at = |d: i32, n: u32| (((d as f64 + 0.5) / scale) as u32).min(n - 1);
                c.compose_device(pic, |x, y| {
                    let (tx, ty) = (at(x - ox, shown.0), at(y - oy, shown.1));
                    let (sx, sy) = source_of(tx, ty, size, v.turns);
                    picture.pixels[sy as usize * sw + sx as usize]
                });
            }
        }
    }
    draw_bars(c, p, v, lay);
}

fn draw_bars(c: &mut Canvas, p: &Paint, v: &ImageView, lay: &Layout) {
    let b = lay.device_view();
    if let Some(sb) = lay.vscroll {
        let lit = axis_lit(v.bars.lit(), Axis::Vertical);
        let (visible, total) = (b.height as usize, lay.content.1 as usize);
        draw_scrollbar(c, p, sb, v.top.max(0) as usize, visible, total, lit);
    }
    if let Some(hb) = lay.hscroll {
        let lit = axis_lit(v.bars.lit(), Axis::Horizontal);
        let (visible, total) = (b.width as usize, lay.content.0 as usize);
        draw_hscrollbar(c, p, hb, v.left.max(0) as usize, visible, total, lit);
    }
    if let Some(corner) = lay.corner() {
        draw_scroll_corner(c, p, corner, false);
    }
}

/// The box on `ground` with `message` in its middle, for a picture still loading or one that
/// cannot be shown.
pub fn draw_message(
    c: &mut Canvas,
    text: &TextRenderer,
    r: Rect,
    message: &str,
    (ground, ink): (Color, Color),
    px: u32,
) {
    c.fill_rect(r, ground);
    let line = Rect::new(
        r.x,
        r.y + (r.height - px as i32) / 2,
        r.width,
        px as i32 + 4,
    );
    let shown = text.ellipsize(message, r.width, false, px);
    text.draw(c, line, &shown, false, ink, Align::Center, px);
}

/// The picture as an image node named `name` under `parent`.
pub fn access(tree: &mut Tree, parent: NodeId, v: &ImageView, lay: &Layout, name: &str) -> NodeId {
    tree.push(
        parent,
        "picture",
        access::titled(Role::Image, v.picture_rect(lay), name),
    )
}

#[cfg(test)]
#[path = "imageview_tests.rs"]
mod tests;
