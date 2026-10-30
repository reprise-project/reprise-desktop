//! wp-pointer-warp-v1: a client moves the pointer to a point of a surface in the tree it has
//! pointer focus on, given the serial of the latest enter.

use smithay::{
    backend::{input::InputTime, renderer::utils::with_renderer_surface_state},
    input::pointer::ClickGrab,
    reexports::wayland_server::protocol::{wl_pointer::WlPointer, wl_surface::WlSurface},
    utils::{Logical, Point, Rectangle, Serial, Size},
    wayland::{
        compositor::{
            RectangleKind, SubsurfaceCachedState, SurfaceAttributes, get_parent, with_states,
        },
        pointer_warp::PointerWarpHandler,
    },
};

use crate::{
    compositor::State,
    constraint::{inside, root_of},
};

type Rects = Vec<(RectangleKind, Rectangle<i32, Logical>)>;

#[derive(Debug, PartialEq, Eq)]
pub enum Refused {
    /// The shell or a grab other than a button's own holds the pointer.
    Busy,
    /// The surface is not in the tree under the pointer.
    Unfocused,
    Serial,
    /// Off the surface or its input region.
    Outside,
}

/// What a warp request is judged on.
struct Ask {
    busy: bool,
    /// Layout point of the target surface's (0, 0), when it shares the focused surface's tree.
    origin: Option<Point<f64, Logical>>,
    last_enter: Option<Serial>,
    serial: Serial,
    /// The target's size, none while unmapped.
    size: Option<Size<i32, Logical>>,
    region: Option<Rects>,
    at: Point<f64, Logical>,
}

/// The layout point a warp moves the pointer to.
fn judge(ask: Ask) -> Result<Point<f64, Logical>, Refused> {
    if ask.busy {
        return Err(Refused::Busy);
    }
    let origin = ask.origin.ok_or(Refused::Unfocused)?;
    if ask.last_enter != Some(ask.serial) {
        return Err(Refused::Serial);
    }
    let size = ask.size.ok_or(Refused::Outside)?;
    let on_surface = inside(&[(RectangleKind::Add, Rectangle::from_size(size))], ask.at);
    if !on_surface || ask.region.is_some_and(|r| !inside(&r, ask.at)) {
        return Err(Refused::Outside);
    }
    Ok(origin + ask.at)
}

/// Offset of `surface` from the root of its subsurface tree.
fn offset_in_tree(surface: &WlSurface) -> Point<i32, Logical> {
    let mut offset = Point::default();
    let mut s = surface.clone();
    while let Some(parent) = get_parent(&s) {
        offset += with_states(&s, |st| {
            st.cached_state
                .get::<SubsurfaceCachedState>()
                .current()
                .location
        });
        s = parent;
    }
    offset
}

impl State {
    /// Move the pointer to `at` on `surface` for its client, if the rules allow it.
    pub fn warp(
        &mut self,
        surface: &WlSurface,
        at: Point<f64, Logical>,
        serial: Serial,
    ) -> Result<(), Refused> {
        let pointer = self.seat.get_pointer().ok_or(Refused::Unfocused)?;
        let busy = !self.shell_leaves_pointer()
            || !self.menus.is_empty()
            || pointer
                .with_grab(|_, g| !g.is::<ClickGrab<State>>())
                .unwrap_or(false);
        let seat_focus = pointer.current_focus().and_then(|f| f.surface().cloned());
        let origin = self
            .pointers
            .focus
            .as_ref()
            .filter(|(f, _)| Some(f) == seat_focus.as_ref() && root_of(f) == root_of(surface))
            .map(|(f, o)| *o - offset_in_tree(f).to_f64() + offset_in_tree(surface).to_f64());
        let target = judge(Ask {
            busy,
            origin,
            last_enter: pointer.last_enter(),
            serial,
            size: with_renderer_surface_state(surface, |s| s.surface_size()).flatten(),
            region: with_states(surface, |st| {
                (st.cached_state.get::<SurfaceAttributes>().current())
                    .input_region
                    .as_ref()
                    .map(|r| r.rects.clone())
            }),
            at,
        })?;
        self.pointer_to(target, InputTime::now());
        self.refresh_constraint();
        Ok(())
    }
}

impl PointerWarpHandler for State {
    fn warp_pointer(
        &mut self,
        surface: WlSurface,
        _pointer: WlPointer,
        pos: Point<f64, Logical>,
        serial: Serial,
    ) {
        if let Err(why) = self.warp(&surface, pos, serial) {
            log::debug!("pointer warp on {surface:?} refused: {why:?}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ask() -> Ask {
        Ask {
            busy: false,
            origin: Some((100.0, 50.0).into()),
            last_enter: Some(Serial::from(7)),
            serial: Serial::from(7),
            size: Some((200, 100).into()),
            region: None,
            at: (20.0, 30.0).into(),
        }
    }

    #[test]
    fn a_warp_on_the_focused_surface_lands_at_its_point() {
        assert_eq!(judge(ask()), Ok((120.0, 80.0).into()));
    }

    #[test]
    fn a_warp_while_the_shell_holds_the_pointer_is_refused() {
        let a = Ask {
            busy: true,
            ..ask()
        };
        assert_eq!(judge(a), Err(Refused::Busy));
    }

    #[test]
    fn a_surface_outside_the_focused_tree_is_refused() {
        let a = Ask {
            origin: None,
            ..ask()
        };
        assert_eq!(judge(a), Err(Refused::Unfocused));
    }

    #[test]
    fn only_the_latest_enter_serial_is_accepted() {
        let a = Ask {
            serial: Serial::from(6),
            ..ask()
        };
        assert_eq!(judge(a), Err(Refused::Serial));
        let a = Ask {
            last_enter: None,
            ..ask()
        };
        assert_eq!(judge(a), Err(Refused::Serial), "no enter since the leave");
    }

    #[test]
    fn a_point_off_the_surface_is_refused() {
        for at in [(200.0, 10.0), (-0.5, 10.0), (10.0, 100.0)] {
            let a = Ask {
                at: at.into(),
                ..ask()
            };
            assert_eq!(judge(a), Err(Refused::Outside), "{at:?}");
        }
        let a = Ask {
            size: None,
            ..ask()
        };
        assert_eq!(judge(a), Err(Refused::Outside), "an unmapped surface");
        let a = Ask {
            at: (199.5, 99.5).into(),
            ..ask()
        };
        assert_eq!(judge(a), Ok((299.5, 149.5).into()), "the last pixel");
    }

    #[test]
    fn a_point_outside_the_input_region_is_refused() {
        let region = vec![
            (
                RectangleKind::Add,
                Rectangle::new((0, 0).into(), (200, 100).into()),
            ),
            (
                RectangleKind::Subtract,
                Rectangle::new((0, 0).into(), (50, 50).into()),
            ),
        ];
        let a = Ask {
            region: Some(region.clone()),
            ..ask()
        };
        assert_eq!(judge(a), Err(Refused::Outside));
        let a = Ask {
            region: Some(region),
            at: (60.0, 30.0).into(),
            ..ask()
        };
        assert_eq!(judge(a), Ok((160.0, 80.0).into()));
    }

    #[test]
    fn a_warp_moves_the_pointer_within_the_window_under_it_only() {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        let mut c = h.connect();
        let behind = h.map_toplevel(&mut c, "test.behind", 300, 200);
        let front = h.map_toplevel(&mut c, "test.front", 300, 200);
        let w = h.state.win(front.id).expect("the window");
        let (surface, (sx, sy)) = (w.wl_surface().clone(), w.surface_origin());
        let other = (h.state.win(behind.id).expect("the window").wl_surface()).clone();
        h.pointer_to(f64::from(sx + 150), f64::from(sy + 100));
        let enter = (h.state.seat.get_pointer())
            .and_then(|p| p.last_enter())
            .expect("an enter");

        assert_eq!(
            h.state.warp(&other, (10.0, 10.0).into(), enter),
            Err(Refused::Unfocused)
        );
        h.button(crate::harness::LEFT, true);
        assert_eq!(
            h.state.warp(&surface, (10.0, 20.0).into(), enter),
            Ok(()),
            "a button's own grab allows it"
        );
        assert_eq!(
            h.state.pointer_location(),
            Point::from((f64::from(sx + 10), f64::from(sy + 20)))
        );
    }
}
