//! pointer-constraints-v1 and relative-pointer-v1: a client locks the pointer or confines it to a
//! region, and reads raw motion. A constraint holds only while the pointer is over its surface and
//! region, the window has the keyboard, and nothing of the compositor's wants the pointer; losing
//! any releases it on the next motion.

use smithay::{
    backend::{input::InputTime, renderer::utils::with_renderer_surface_state},
    input::pointer::{PointerHandle, RelativeMotionEvent},
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point, Rectangle},
    wayland::{
        compositor::{RectangleKind, get_parent},
        pointer_constraints::{
            ConstraintRemove, PointerConstraint, PointerConstraintsHandler, with_pointer_constraint,
        },
    },
};

use crate::{compositor::State, input::Interaction};

enum Held {
    Free,
    Locked,
    Confined(Option<Vec<(RectangleKind, Rectangle<i32, Logical>)>>),
}

impl State {
    pub fn pointer_moved(
        &mut self,
        delta: Point<f64, Logical>,
        unaccel: Point<f64, Logical>,
        time: InputTime,
    ) {
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        if !self.is_locked() {
            pointer.relative_motion(
                self,
                self.pointers.focus.clone().map(|(s, o)| (s.into(), o)),
                &RelativeMotionEvent {
                    delta,
                    delta_unaccel: unaccel,
                    time,
                },
            );
        }
        self.refresh_constraint();
        match self.constrain(&pointer, delta) {
            Some(pos) => {
                self.pointer_to(pos, time);
                self.refresh_constraint();
            }
            None => pointer.frame(self),
        }
    }

    fn constrain(
        &self,
        pointer: &PointerHandle<State>,
        delta: Point<f64, Logical>,
    ) -> Option<Point<f64, Logical>> {
        let pos = self.pointer_location();
        let Some(surface) = self.pointers.constrained.as_ref() else {
            return Some(pos + delta);
        };
        let held = with_pointer_constraint(surface, pointer, |c| match c {
            Some(c) if c.is_active() => match &*c {
                PointerConstraint::Locked(_) => Held::Locked,
                PointerConstraint::Confined(c) => {
                    Held::Confined(c.region().map(|r| r.rects.clone()))
                }
            },
            _ => Held::Free,
        });
        match held {
            Held::Free => Some(pos + delta),
            Held::Locked => None,
            Held::Confined(rects) => {
                // Read outside the constraint's closure: it holds the surface's state lock, which
                // `surface_rect` takes too.
                let rects = rects.unwrap_or_else(|| surface_rect(surface));
                let Some(origin) = self.focus_origin(surface) else {
                    return Some(pos + delta);
                };
                Some(confine(&rects, pos - origin, pos + delta - origin) + origin)
            }
        }
    }

    fn focus_origin(&self, surface: &WlSurface) -> Option<Point<f64, Logical>> {
        self.pointers
            .focus
            .as_ref()
            .filter(|(s, _)| s == surface)
            .map(|(_, o)| *o)
    }

    fn wanted_constraint(&self) -> Option<WlSurface> {
        // Menus, the switcher, the launcher, panels and dialogs take the keyboard without moving
        // the seat's focus off the window, so they are checked on their own.
        if self.is_locked()
            || self.interaction != Interaction::Idle
            || !self.menus.is_empty()
            || self.switcher.is_open()
            || self.launcher.is_some()
            || self.start_panel.is_some()
            || self.panels.clipview.is_some()
            || self.panels.emojiview.is_some()
            || self.panels.region.is_some()
            || crate::notifyview::reply_focused(self)
            || self.dialog_has_keyboard()
        {
            return None;
        }
        let pointer = self.seat.get_pointer()?;
        let surface = pointer.current_focus()?.surface()?.clone();
        let origin = self.focus_origin(&surface)?;
        let keyboard = self.seat.get_keyboard()?.current_focus()?;
        if root_of(&surface) != root_of(&keyboard) {
            return None;
        }
        let at = (self.pointer_location() - origin).to_i32_round();
        with_pointer_constraint(&surface, &pointer, |c| {
            c.is_some_and(|c| c.region().is_none_or(|r| r.contains(at)))
        })
        .then_some(surface)
    }

    pub fn refresh_constraint(&mut self) {
        let next = self.wanted_constraint();
        if next == self.pointers.constrained {
            return;
        }
        if let Some(prev) = self.pointers.constrained.take() {
            self.release_constraint(&prev);
        }
        if let (Some(s), Some(pointer)) = (&next, self.seat.get_pointer()) {
            log::debug!("pointer constraint on {s:?} active");
            with_pointer_constraint(s, &pointer, |c| {
                if let Some(c) = c {
                    c.activate();
                }
            });
        }
        self.pointers.constrained = next;
    }

    fn release_constraint(&mut self, surface: &WlSurface) {
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        log::debug!("pointer constraint on {surface:?} released");
        let hint = with_pointer_constraint(surface, &pointer, |c| {
            let c = c?;
            let hint = match &*c {
                PointerConstraint::Locked(l) if c.is_active() => l.cursor_position_hint(),
                _ => None,
            };
            c.deactivate();
            hint
        });
        if let (Some(hint), Some(origin)) = (hint, self.focus_origin(surface)) {
            pointer.set_location(origin + hint);
            self.redraw_content();
        }
    }
}

impl PointerConstraintsHandler for State {
    fn new_constraint(&mut self, _surface: &WlSurface, _pointer: &PointerHandle<Self>) {
        self.refresh_constraint();
    }

    fn remove_constraint(
        &mut self,
        surface: &WlSurface,
        _pointer: &PointerHandle<Self>,
        _why: ConstraintRemove,
    ) {
        // The client destroyed it, or Smithay released it as the pointer left; runs under the
        // pointer's lock on a leave, so it only forgets.
        if self.pointers.constrained.as_ref() == Some(surface) {
            self.pointers.constrained = None;
        }
    }
}

fn root_of(surface: &WlSurface) -> WlSurface {
    let mut root = surface.clone();
    while let Some(parent) = get_parent(&root) {
        root = parent;
    }
    root
}

fn surface_rect(surface: &WlSurface) -> Vec<(RectangleKind, Rectangle<i32, Logical>)> {
    let size = with_renderer_surface_state(surface, |s| s.surface_size())
        .flatten()
        .unwrap_or_default();
    vec![(RectangleKind::Add, Rectangle::from_size(size))]
}

fn inside(rects: &[(RectangleKind, Rectangle<i32, Logical>)], p: Point<f64, Logical>) -> bool {
    let p = Point::<i32, Logical>::from((p.x.floor() as i32, p.y.floor() as i32));
    let mut inside = false;
    for (kind, r) in rects {
        if r.contains(p) {
            inside = matches!(kind, RectangleKind::Add);
        }
    }
    inside
}

pub fn confine(
    rects: &[(RectangleKind, Rectangle<i32, Logical>)],
    from: Point<f64, Logical>,
    to: Point<f64, Logical>,
) -> Point<f64, Logical> {
    if inside(rects, to) {
        return to;
    }
    if !inside(rects, from) {
        return from;
    }
    let clamped = rects
        .iter()
        .filter(|(k, r)| matches!(k, RectangleKind::Add) && inside(&[(*k, *r)], from))
        .map(|(_, r)| {
            let max_x = f64::from(r.loc.x + r.size.w) - 1.0 / 256.0;
            let max_y = f64::from(r.loc.y + r.size.h) - 1.0 / 256.0;
            Point::from((
                to.x.clamp(f64::from(r.loc.x), max_x),
                to.y.clamp(f64::from(r.loc.y), max_y),
            ))
        })
        .find(|p| inside(rects, *p));
    if let Some(p) = clamped {
        return p;
    }
    [Point::from((to.x, from.y)), Point::from((from.x, to.y))]
        .into_iter()
        .find(|p| inside(rects, *p))
        .unwrap_or(from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn add(x: i32, y: i32, w: i32, h: i32) -> (RectangleKind, Rectangle<i32, Logical>) {
        (
            RectangleKind::Add,
            Rectangle::new((x, y).into(), (w, h).into()),
        )
    }

    fn sub(x: i32, y: i32, w: i32, h: i32) -> (RectangleKind, Rectangle<i32, Logical>) {
        (
            RectangleKind::Subtract,
            Rectangle::new((x, y).into(), (w, h).into()),
        )
    }

    fn p(x: f64, y: f64) -> Point<f64, Logical> {
        (x, y).into()
    }

    #[test]
    fn a_move_inside_the_region_is_kept() {
        let r = [add(0, 0, 100, 50)];
        assert_eq!(confine(&r, p(10.0, 10.0), p(40.5, 20.0)), p(40.5, 20.0));
    }

    #[test]
    fn a_move_past_an_edge_slides_along_it() {
        let r = [add(0, 0, 100, 50)];
        let q = confine(&r, p(90.0, 10.0), p(130.0, 30.0));
        assert!(q.x < 100.0 && q.x > 99.0, "stops at the right edge: {q:?}");
        assert_eq!(q.y, 30.0, "the other axis still moves");
        let q = confine(&r, p(5.0, 5.0), p(-20.0, -20.0));
        assert_eq!(q, p(0.0, 0.0));
    }

    #[test]
    fn a_cut_out_stops_the_pointer_at_its_edge() {
        let r = [add(0, 0, 100, 100), sub(40, 0, 20, 100)];
        let q = confine(&r, p(30.0, 50.0), p(45.0, 60.0));
        assert_eq!(q, p(30.0, 60.0), "only the axis that stays clear moves");
        let q = confine(&r, p(30.0, 50.0), p(45.0, 50.0));
        assert_eq!(q, p(30.0, 50.0));
    }

    #[test]
    fn a_pointer_already_outside_does_not_move() {
        let r = [add(0, 0, 10, 10)];
        assert_eq!(confine(&r, p(20.0, 20.0), p(30.0, 30.0)), p(20.0, 20.0));
    }

    #[test]
    fn a_later_rect_decides_the_point() {
        let r = [add(0, 0, 10, 10), sub(0, 0, 5, 5), add(2, 2, 1, 1)];
        assert!(inside(&r, p(2.5, 2.5)));
        assert!(!inside(&r, p(1.0, 1.0)));
        assert!(inside(&r, p(7.0, 7.0)));
    }
}
