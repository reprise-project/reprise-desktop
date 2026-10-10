//! Where a frame's pixels lie in the framebuffer it draws into.
//!
//! Elements place themselves in the frame's own space: physical pixels from the top-left of the
//! output as seen (`Frame::output_size`). A rotated or flipped output draws into a framebuffer in
//! the panel's orientation; the glass copies out of it, so its picture lies that way too.

use smithay::utils::{Physical, Point, Rectangle, Size, Transform};

/// One frame's mapping from its own space to its framebuffer's. Smithay's `transform_rect_in`
/// follows the projection for every transform, but `transform_point_in` and `invert` name the two
/// flipped quarter turns the other way round, so everything here goes through rects.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Space {
    size: Size<i32, Physical>,
    /// `Frame::transformation`, the inverse of the output's.
    frame: Transform,
}

/// Between the projection's naming of the flipped quarter turns and `transform_point_in`'s.
fn swap_flipped(t: Transform) -> Transform {
    match t {
        Transform::Flipped90 => Transform::Flipped270,
        Transform::Flipped270 => Transform::Flipped90,
        other => other,
    }
}

impl Space {
    /// The space of a frame drawn with `transform` at `size` (the frame's `transformation` and
    /// `output_size`).
    pub fn new(transform: Transform, size: Size<i32, Physical>) -> Space {
        Space {
            size,
            frame: transform,
        }
    }

    /// The framebuffer's size.
    pub fn fb_size(self) -> Size<i32, Physical> {
        self.frame.transform_size(self.size)
    }

    /// `r` of the frame, in the framebuffer.
    pub fn in_fb(self, r: Rectangle<i32, Physical>) -> Rectangle<i32, Physical> {
        self.frame.transform_rect_in(r, &self.size)
    }

    /// `r` of the framebuffer, in the frame.
    pub fn in_frame(self, r: Rectangle<i32, Physical>) -> Rectangle<i32, Physical> {
        // As rect maps, the quarter turns undo each other and every flip undoes itself.
        let back = match self.frame {
            Transform::_90 => Transform::_270,
            Transform::_270 => Transform::_90,
            other => other,
        };
        back.transform_rect_in(r, &self.fb_size())
    }

    /// Corner radii (top-left, top-right, bottom-right, bottom-left of a shape as seen) for the
    /// same shape in the framebuffer.
    pub fn radii_in_fb(self, radii: [f32; 4]) -> [f32; 4] {
        let corners: [Point<i32, Physical>; 4] =
            [(0, 0).into(), (1, 0).into(), (1, 1).into(), (0, 1).into()];
        let unit = Size::from((1, 1));
        let mut out = [0.0; 4];
        for (corner, r) in corners.iter().zip(radii) {
            let at = self
                .frame
                .transform_rect_in(Rectangle::new(*corner, Size::default()), &unit)
                .loc;
            let i = corners.iter().position(|c| *c == at).expect("a corner");
            out[i] = r;
        }
        out
    }

    /// The source transform that draws a texture in framebuffer orientation upright into the frame.
    pub fn upright(self) -> Transform {
        self.frame.invert()
    }

    /// The source transform that draws a texture in the frame's orientation into one in framebuffer
    /// orientation (through an untransformed offscreen frame).
    pub fn toward_fb(self) -> Transform {
        swap_flipped(self.frame)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Transform; 8] = [
        Transform::Normal,
        Transform::_90,
        Transform::_180,
        Transform::_270,
        Transform::Flipped,
        Transform::Flipped90,
        Transform::Flipped180,
        Transform::Flipped270,
    ];

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Physical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    fn projected(t: Transform, size: Size<i32, Physical>, p: (f64, f64)) -> (f64, f64) {
        let (w, h) = (f64::from(size.w), f64::from(size.h));
        let fb = t.transform_size(size);
        let (u, v) = (2.0 * p.0 / w - 1.0, 1.0 - 2.0 * p.1 / h);
        let m = t.matrix();
        let (a, b) = (f64::from(m.x_axis.x), f64::from(m.x_axis.y));
        let (c, d) = (f64::from(m.y_axis.x), f64::from(m.y_axis.y));
        let (x, y) = (a * u + c * v, -(b * u + d * v));
        (
            (x + 1.0) / 2.0 * f64::from(fb.w),
            (y + 1.0) / 2.0 * f64::from(fb.h),
        )
    }

    #[test]
    fn a_rect_lands_where_the_projection_draws_it() {
        let size = Size::from((300, 200));
        let r = rect(10, 20, 60, 40);
        for t in ALL {
            let s = Space::new(t, size);
            let fb = s.in_fb(r);
            let a = projected(t, size, (10.0, 20.0));
            let b = projected(t, size, (70.0, 60.0));
            let px = |v: f64| v.round() as i32;
            let want = rect(
                px(a.0.min(b.0)),
                px(a.1.min(b.1)),
                px((a.0 - b.0).abs()),
                px((a.1 - b.1).abs()),
            );
            assert_eq!(fb, want, "{t:?}");
            assert_eq!(s.fb_size(), t.transform_size(size), "{t:?}");
            assert_eq!(s.in_frame(fb), r, "{t:?}: back where it was");
        }
    }

    #[test]
    fn a_quarter_turn_lays_the_frame_on_its_side() {
        let s = Space::new(Transform::_90, (300, 200).into());
        assert_eq!(s.fb_size(), Size::from((200, 300)));
        assert_eq!(s.in_fb(rect(0, 0, 10, 20)), rect(180, 0, 20, 10));
        let s = Space::new(Transform::Flipped90, (300, 200).into());
        assert_eq!(s.in_fb(rect(0, 0, 10, 20)), rect(180, 290, 20, 10));
    }

    #[test]
    fn each_corner_keeps_its_radius() {
        let radii = [1.0, 2.0, 3.0, 4.0];
        let size = Size::from((300, 200));
        for t in ALL {
            let s = Space::new(t, size);
            let fb = s.radii_in_fb(radii);
            let shape = (40.0, 30.0, 100.0, 60.0);
            let corners = [
                (shape.0, shape.1),
                (shape.0 + shape.2, shape.1),
                (shape.0 + shape.2, shape.1 + shape.3),
                (shape.0, shape.1 + shape.3),
            ];
            let mapped = corners.map(|c| projected(t, size, c));
            let (x0, y0) = mapped
                .iter()
                .fold((f64::MAX, f64::MAX), |m, p| (m.0.min(p.0), m.1.min(p.1)));
            for (i, p) in mapped.iter().enumerate() {
                let j = match (p.0 > x0, p.1 > y0) {
                    (false, false) => 0,
                    (true, false) => 1,
                    (true, true) => 2,
                    (false, true) => 3,
                };
                assert_eq!(fb[j], radii[i], "{t:?}: corner {i} lands on {j}");
            }
        }
        let s = Space::new(Transform::_90, size);
        assert_eq!(
            s.radii_in_fb([7.0, 7.0, 0.0, 0.0]),
            [0.0, 7.0, 7.0, 0.0],
            "a frame's rounded top runs down the framebuffer's right side"
        );
    }
}
