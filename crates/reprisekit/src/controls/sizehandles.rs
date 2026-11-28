//! A raster editor's sizing handles: one on the sheet's right edge, one on its bottom edge and one
//! on its bottom-right corner, each just outside the sheet. A drag on one gives the sheet a new
//! width, height or both; while it runs a dotted outline shows the size it would give. Layout is
//! logical window px; `draw` scales by `s`.

use crate::access::{self, Tree, accesskit};
use crate::look::Paint;
use crate::look::widgets::draw_focus_ring;
use crate::pointer::Shape;
use crate::raster::{Canvas, Rect};
use accesskit::{NodeId, Role};

/// A handle's side.
pub const HANDLE: i32 = 3;
/// How far past a handle's square a press still takes it.
const SLOP: i32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    Right,
    Bottom,
    Corner,
}

impl Handle {
    pub const ALL: [Handle; 3] = [Handle::Right, Handle::Bottom, Handle::Corner];

    /// The pointer over the handle and through its drag.
    pub fn shape(self) -> Shape {
        match self {
            Handle::Right => Shape::EResize,
            Handle::Bottom => Shape::SResize,
            Handle::Corner => Shape::SeResize,
        }
    }

    fn moves(self) -> (bool, bool) {
        match self {
            Handle::Right => (true, false),
            Handle::Bottom => (false, true),
            Handle::Corner => (true, true),
        }
    }
}

/// A drag on a handle: where it started and where the pointer is now, in window px, and the
/// sheet's size then and now, in sheet px.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Drag {
    pub handle: Handle,
    pub start: (i32, i32),
    pub at: (i32, i32),
    pub from: (i32, i32),
    pub size: (i32, i32),
}

impl Drag {
    pub fn new(handle: Handle, start: (i32, i32), from: (i32, i32)) -> Drag {
        Drag {
            handle,
            start,
            at: start,
            from,
            size: from,
        }
    }

    /// How far the pointer lies past `view`'s edges, in window px, along the axes the handle
    /// moves: negative past the left or top, 0 inside.
    pub fn past(&self, view: Rect) -> (i32, i32) {
        let out = |at: i32, lo: i32, hi: i32| {
            if at < lo {
                at - lo
            } else if at >= hi {
                at - hi + 1
            } else {
                0
            }
        };
        let (mx, my) = self.handle.moves();
        (
            if mx {
                out(self.at.0, view.x, view.right())
            } else {
                0
            },
            if my {
                out(self.at.1, view.y, view.bottom())
            } else {
                0
            },
        )
    }

    /// The sheet scrolled `by` window px under the resting pointer, as when the view follows a
    /// drag past its edge: the size follows as [`Drag::motion`] does; true when it changed.
    pub fn scrolled(&mut self, by: (i32, i32), unit: i32, max: i32) -> bool {
        self.start = (self.start.0 - by.0, self.start.1 - by.1);
        self.motion(self.at, unit, max)
    }

    /// Follows the pointer to `at`, a sheet px every `unit` window px, each side held to
    /// `1..=max`; true when the size changed.
    pub fn motion(&mut self, at: (i32, i32), unit: i32, max: i32) -> bool {
        self.at = at;
        let unit = unit.max(1);
        let side = |from: i32, d: i32| (from + (2 * d + unit).div_euclid(2 * unit)).clamp(1, max);
        let (mx, my) = self.handle.moves();
        let size = (
            if mx {
                side(self.from.0, at.0 - self.start.0)
            } else {
                self.from.0
            },
            if my {
                side(self.from.1, at.1 - self.start.1)
            } else {
                self.from.1
            },
        );
        std::mem::replace(&mut self.size, size) != size
    }

    /// The size changed since the press.
    pub fn resized(&self) -> bool {
        self.size != self.from
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub right: Rect,
    pub bottom: Rect,
    pub corner: Rect,
}

impl Layout {
    pub fn rect(&self, h: Handle) -> Rect {
        match h {
            Handle::Right => self.right,
            Handle::Bottom => self.bottom,
            Handle::Corner => self.corner,
        }
    }
}

/// The handles around `sheet`, the sheet's rect on the window.
pub fn layout(sheet: Rect) -> Layout {
    let mid = |at: i32, len: i32| at + (len - HANDLE) / 2;
    Layout {
        right: Rect::new(sheet.right(), mid(sheet.y, sheet.height), HANDLE, HANDLE),
        bottom: Rect::new(mid(sheet.x, sheet.width), sheet.bottom(), HANDLE, HANDLE),
        corner: Rect::new(sheet.right(), sheet.bottom(), HANDLE, HANDLE),
    }
}

/// The handle under `(x, y)`.
pub fn hit(lay: &Layout, x: i32, y: i32) -> Option<Handle> {
    Handle::ALL
        .into_iter()
        .find(|&h| lay.rect(h).inset(-SLOP).contains(x, y))
}

/// The handles in the look's highlight colour.
pub fn draw(c: &mut Canvas, p: &Paint, lay: &Layout) {
    for h in Handle::ALL {
        c.fill_rect(lay.rect(h).scaled(p.s), p.cs.hilight);
    }
}

/// The dotted outline just outside `sheet`, the rect a running drag would give the sheet.
pub fn draw_outline(c: &mut Canvas, p: &Paint, sheet: Rect) {
    draw_focus_ring(c, p, sheet.inset(-1).scaled(p.s), p.cs.window_text);
}

/// A splitter per handle, named for the sides it moves.
pub fn access(lay: &Layout, tree: &mut Tree, parent: NodeId) {
    for h in Handle::ALL {
        let (name, label) = match h {
            Handle::Right => ("size-right", tr!("size-handle-right")),
            Handle::Bottom => ("size-bottom", tr!("size-handle-bottom")),
            Handle::Corner => ("size-corner", tr!("size-handle-corner")),
        };
        tree.push(
            parent,
            name,
            access::titled(Role::Splitter, lay.rect(h), &label),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interface::Interface;
    use crate::interfaces::theme_for;
    use crate::theme::{Color, Theme};

    const SHEET: Rect = Rect::new(4, 4, 40, 20);

    fn drawn(theme: &Theme, dragging: Option<Rect>) -> Canvas {
        let s = theme.scale.max(1);
        let mut c = Canvas::new((60 * s) as u32, (40 * s) as u32);
        let p = theme.paint();
        c.fill_rect(c.bounds(), p.cs.button_shadow);
        c.fill_rect(SHEET.scaled(s), Color::rgb(0xFF, 0xFF, 0xFF));
        draw(&mut c, &p, &layout(SHEET));
        if let Some(r) = dragging {
            draw_outline(&mut c, &p, r);
        }
        c
    }

    use crate::pins::fnv;

    #[test]
    fn each_state_is_pinned_in_every_look_at_scale_1_and_2() {
        let mut got = Vec::new();
        let looks = [
            ("classic", Theme::default()),
            ("blue", theme_for(Interface::Blue)),
            ("glass", theme_for(Interface::Glass)),
        ];
        for (name, theme) in looks {
            for s in [1, 2] {
                let theme = theme.scaled(s);
                let grown = Rect::new(SHEET.x, SHEET.y, 50, 28);
                for (state, dragging) in [("rest", None), ("dragging", Some(grown))] {
                    let c = drawn(&theme, dragging);
                    got.push((format!("{name} {s} {state}"), fnv(&c)));
                }
            }
        }
        crate::pins::check("controls/size_handles", &got);
    }

    #[test]
    fn the_handles_sit_outside_the_right_edge_the_bottom_edge_and_the_corner() {
        let lay = layout(SHEET);
        assert_eq!(lay.right, Rect::new(44, 12, 3, 3));
        assert_eq!(lay.bottom, Rect::new(22, 24, 3, 3));
        assert_eq!(lay.corner, Rect::new(44, 24, 3, 3));
        let theme = Theme::default().scaled(2);
        let c = drawn(&theme, None);
        let hilight = theme.colors.hilight.argb();
        for h in Handle::ALL {
            let r = lay.rect(h).scaled(2);
            assert_eq!(c.get(r.x, r.y), hilight, "{h:?} top-left at scale 2");
            assert_eq!(
                c.get(r.right() - 1, r.bottom() - 1),
                hilight,
                "{h:?} bottom-right"
            );
        }
    }

    #[test]
    fn hit_takes_each_handle_and_a_pixel_around_it() {
        let lay = layout(SHEET);
        assert_eq!(hit(&lay, 45, 13), Some(Handle::Right));
        assert_eq!(hit(&lay, 43, 11), Some(Handle::Right), "one px of slop");
        assert_eq!(hit(&lay, 23, 25), Some(Handle::Bottom));
        assert_eq!(hit(&lay, 45, 25), Some(Handle::Corner));
        assert_eq!(hit(&lay, 47, 27), Some(Handle::Corner));
        assert_eq!(hit(&lay, 30, 14), None, "the sheet itself");
        assert_eq!(
            hit(&lay, 45, 4),
            None,
            "the right edge away from its handle"
        );
    }

    #[test]
    fn a_drag_moves_only_the_sides_its_handle_holds_and_keeps_them_in_range() {
        let mut d = Drag::new(Handle::Right, (45, 13), (40, 20));
        assert!(d.motion((55, 30), 1, 100));
        assert_eq!(d.size, (50, 20), "the right handle keeps the height");
        let mut d = Drag::new(Handle::Bottom, (23, 25), (40, 20));
        d.motion((0, 20), 1, 100);
        assert_eq!(d.size, (40, 15), "the bottom handle keeps the width");
        let mut d = Drag::new(Handle::Corner, (45, 25), (40, 20));
        d.motion((45 + 9, 25 - 7), 4, 100);
        assert_eq!(d.size, (42, 18), "a sheet px per 4 window px, rounded");
        assert!(!d.motion((45 + 8, 25 - 9), 4, 100), "still 42x18");
        d.motion((-500, 1000), 1, 100);
        assert_eq!(d.size, (1, 100), "held to 1..=max");
        assert!(d.resized());
        d.motion((45, 25), 1, 100);
        assert!(!d.resized(), "back where it started");
    }

    #[test]
    fn a_drag_past_the_view_reads_how_far_along_its_axes_and_follows_a_scroll() {
        let view = Rect::new(10, 10, 100, 80);
        let mut d = Drag::new(Handle::Right, (60, 40), (50, 30));
        d.motion((130, 200), 1, 1000);
        assert_eq!(d.past(view), (21, 0), "the right handle ignores the height");
        let mut c = Drag::new(Handle::Corner, (60, 40), (50, 30));
        c.motion((5, 95), 1, 1000);
        assert_eq!(c.past(view), (-5, 6));
        c.motion((50, 50), 1, 1000);
        assert_eq!(c.past(view), (0, 0));
        assert!(d.scrolled((16, 0), 2, 1000));
        assert_eq!(
            d.size,
            (50 + (130 + 16 - 60) / 2, 30),
            "16 px scrolled under a resting pointer count as 16 px moved"
        );
    }

    #[test]
    fn each_handle_shows_the_resize_arrow_of_the_sides_it_moves() {
        assert_eq!(Handle::Right.shape(), Shape::EResize);
        assert_eq!(Handle::Bottom.shape(), Shape::SResize);
        assert_eq!(Handle::Corner.shape(), Shape::SeResize);
    }

    #[test]
    fn access_names_a_splitter_per_handle() {
        let lay = layout(SHEET);
        let mut tree = Tree::new(access::node(Role::Window, Rect::new(0, 0, 60, 40)));
        access(&lay, &mut tree, access::ROOT);
        for name in ["Resize width", "Resize height", "Resize width and height"] {
            assert!(tree.find(Role::Splitter, name).is_some(), "{name}");
        }
    }
}
