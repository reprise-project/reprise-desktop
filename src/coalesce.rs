//! Client pointer and drag motion held back to one delivery per refresh interval.

use std::time::{Duration, Instant};

use smithay::{
    backend::input::InputTime,
    input::{
        dnd::{DndFocus, Source},
        pointer::{MotionEvent, PointerTarget, RelativeMotionEvent},
    },
    reexports::{
        calloop::timer::{TimeoutAction, Timer},
        wayland_server::protocol::wl_surface::WlSurface,
    },
    utils::{IsAlive, Logical, Point, Rectangle},
};

use crate::{compositor::State, dnd::DragSource, xdnd::X11Offer};

/// Motion owed to one client surface `S`; `O` is an X11 drag target's offer.
#[derive(Debug, Clone)]
pub enum Held<S, O = X11Offer> {
    /// `wl_pointer.motion` at the latest position and the relative deltas
    /// summed since the last delivery, closed by one `wl_pointer.frame`.
    Pointer {
        to: S,
        motion: Option<MotionEvent>,
        relative: Option<RelativeMotionEvent>,
    },
    /// `wl_data_device.motion`, or `XdndPosition` through an X11 target's
    /// offer, at the latest surface-local position.
    Drag {
        to: S,
        at: Point<f64, Logical>,
        time: InputTime,
        x11: Option<O>,
    },
}

/// Motion held for at most one surface. Whatever else reaches a client
/// (button, axis, enter, leave, keyboard focus) takes it out first
/// ([`Motions::take`]), so the client sees the true position before it.
#[derive(Debug)]
pub struct Motions<S, O = X11Offer> {
    held: Option<Held<S, O>>,
    taken_at: Option<Instant>,
}

impl<S, O> Default for Motions<S, O> {
    fn default() -> Self {
        Motions {
            held: None,
            taken_at: None,
        }
    }
}

impl<S: PartialEq, O> Motions<S, O> {
    /// Hold `event` for `to`. Returns motion held for another surface, to
    /// deliver before it.
    pub fn motion(&mut self, to: S, event: MotionEvent) -> Option<Held<S, O>> {
        let earlier = self.other_than_pointer(&to);
        match &mut self.held {
            Some(Held::Pointer { motion, .. }) => *motion = Some(event),
            _ => {
                self.held = Some(Held::Pointer {
                    to,
                    motion: Some(event),
                    relative: None,
                })
            }
        }
        earlier
    }

    /// Add `event`'s deltas to what is held for `to`; the sum carries the
    /// latest time. Returns motion held for another surface, as `motion`.
    pub fn relative(&mut self, to: S, event: RelativeMotionEvent) -> Option<Held<S, O>> {
        let earlier = self.other_than_pointer(&to);
        match &mut self.held {
            Some(Held::Pointer {
                relative: Some(sum),
                ..
            }) => {
                sum.delta += event.delta;
                sum.delta_unaccel += event.delta_unaccel;
                sum.time = event.time;
            }
            Some(Held::Pointer { relative, .. }) => *relative = Some(event),
            _ => {
                self.held = Some(Held::Pointer {
                    to,
                    motion: None,
                    relative: Some(event),
                })
            }
        }
        earlier
    }

    /// Hold a drag's motion over `to` at surface-local `at`, through `x11`
    /// when `to` is an X11 window.
    pub fn drag(
        &mut self,
        to: S,
        at: Point<f64, Logical>,
        time: InputTime,
        x11: Option<O>,
    ) -> Option<Held<S, O>> {
        let earlier = match &self.held {
            Some(Held::Drag { to: held, .. }) if *held == to => None,
            _ => self.held.take(),
        };
        self.held = Some(Held::Drag { to, at, time, x11 });
        earlier
    }

    /// Whether `to`'s pointer frame only closes held motion, whose delivery
    /// sends its own.
    pub fn frame_held(&self, to: &S) -> bool {
        matches!(&self.held, Some(Held::Pointer { to: held, .. }) if held == to)
    }

    pub fn is_held(&self) -> bool {
        self.held.is_some()
    }

    /// The held motion, to deliver now.
    pub fn take(&mut self, now: Instant) -> Option<Held<S, O>> {
        let held = self.held.take()?;
        self.taken_at = Some(now);
        Some(held)
    }

    /// How long held motion waits: out of `interval` since the last
    /// delivery, nothing after a quiet one.
    pub fn wait(&self, now: Instant, interval: Duration) -> Duration {
        self.taken_at.map_or(Duration::ZERO, |t| {
            interval.saturating_sub(now.saturating_duration_since(t))
        })
    }

    fn other_than_pointer(&mut self, to: &S) -> Option<Held<S, O>> {
        match &self.held {
            Some(Held::Pointer { to: held, .. }) if held == to => None,
            _ => self.held.take(),
        }
    }
}

impl State {
    /// Hold `event` for `to` until the next delivery; a constrained pointer's
    /// motion goes at once, as every motion of a game's locked pointer counts.
    pub(crate) fn hold_motion(&mut self, to: &WlSurface, event: &MotionEvent) {
        if self.pointers.constrained.is_some() {
            self.flush_motion();
            PointerTarget::<State>::motion(to, &self.seat.clone(), self, event);
            return;
        }
        let earlier = self.pointers.motions.motion(to.clone(), event.clone());
        self.held(earlier);
    }

    /// As `hold_motion`, for `zwp_relative_pointer_v1` deltas.
    pub(crate) fn hold_relative(&mut self, to: &WlSurface, event: &RelativeMotionEvent) {
        if self.pointers.constrained.is_some() {
            self.flush_motion();
            PointerTarget::<State>::relative_motion(to, &self.seat.clone(), self, event);
            return;
        }
        let earlier = self.pointers.motions.relative(to.clone(), event.clone());
        self.held(earlier);
    }

    /// As `hold_motion`, for a drag over a client's surface; `x11` is the
    /// offer of an X11 window's, whose own `XdndStatus` pacing still applies.
    pub(crate) fn hold_drag(
        &mut self,
        to: &WlSurface,
        at: Point<f64, Logical>,
        time: InputTime,
        x11: Option<X11Offer>,
    ) {
        let earlier = self.pointers.motions.drag(to.clone(), at, time, x11);
        self.held(earlier);
    }

    /// Whether the drag source's actions changed since a target last heard of
    /// them; notes them as heard. A drag target is told of a change only
    /// with its motion, which then cannot wait.
    pub(crate) fn dnd_actions_changed(&mut self) -> bool {
        let actions = self
            .drag_source()
            .and_then(|s| s.metadata())
            .map(|m| m.dnd_actions.to_vec())
            .unwrap_or_default();
        let changed = actions != self.pointers.drag_actions;
        self.pointers.drag_actions = actions;
        changed
    }

    /// Deliver held motion now. Runs before anything else is sent to a
    /// client, and from the timer `held` arms.
    pub fn flush_motion(&mut self) {
        if let Some(t) = self.pointers.motion_timer.take() {
            self.handle.remove(t);
        }
        if let Some(held) = self.pointers.motions.take(Instant::now()) {
            self.deliver_motion(held);
        }
    }

    fn held(&mut self, earlier: Option<Held<WlSurface>>) {
        if let Some(held) = earlier {
            self.deliver_motion(held);
        }
        if self.pointers.motion_timer.is_some() || !self.pointers.motions.is_held() {
            return;
        }
        // The pointer's own location is behind the pointer's lock, which a
        // motion is held under: the timer starts at the fastest output's
        // interval and stretches to the one under the pointer when it fires.
        let wait = self
            .pointers
            .motions
            .wait(Instant::now(), self.motion_interval(None));
        match self
            .handle
            .insert_source(Timer::from_duration(wait), |_, _, state| {
                let interval = state.motion_interval(Some(state.pointer_location()));
                let rest = state.pointers.motions.wait(Instant::now(), interval);
                if !rest.is_zero() {
                    return TimeoutAction::ToDuration(rest);
                }
                state.pointers.motion_timer = None;
                state.flush_motion();
                TimeoutAction::Drop
            }) {
            Ok(t) => self.pointers.motion_timer = Some(t),
            Err(e) => {
                log::warn!("motion timer: {}", e.error);
                self.flush_motion();
            }
        }
    }

    fn motion_interval(&self, at: Option<Point<f64, Logical>>) -> Duration {
        let rates: Vec<_> = (self.scene.outputs().iter())
            .filter_map(|o| Some((reprisescene::output_rect(o)?, o.current_mode()?.refresh)))
            .collect();
        crate::framepace::interval(pace_refresh(&rates, at))
    }

    fn deliver_motion(&mut self, held: Held<WlSurface>) {
        let seat = self.seat.clone();
        match held {
            Held::Pointer {
                to,
                motion,
                relative,
            } if to.alive() => {
                if let Some(event) = relative {
                    PointerTarget::<State>::relative_motion(&to, &seat, self, &event);
                }
                if let Some(event) = motion {
                    PointerTarget::<State>::motion(&to, &seat, self, &event);
                }
                PointerTarget::<State>::frame(&to, &seat, self);
            }
            Held::Drag {
                to,
                at,
                time,
                x11: None,
            } if to.alive() => {
                DndFocus::<State>::motion::<DragSource>(&to, self, None, &seat, at, time);
            }
            Held::Drag {
                to,
                at,
                time,
                x11: Some(offer),
            } => {
                if let Some(x) = self.x11_window_of(&to) {
                    offer.motion(&x, self, &seat, at, time);
                }
            }
            _ => {}
        }
    }
}

/// The refresh (mHz) motion is paced by: the output at `at`, else the
/// fastest; 0 with none.
fn pace_refresh(
    outputs: &[(Rectangle<i32, Logical>, i32)],
    at: Option<Point<f64, Logical>>,
) -> i32 {
    at.and_then(|p| {
        outputs
            .iter()
            .find(|(r, _)| r.to_f64().contains(p))
            .map(|&(_, hz)| hz)
    })
    .or_else(|| outputs.iter().map(|&(_, hz)| hz).max())
    .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use smithay::utils::SERIAL_COUNTER;

    const A: u8 = 1;
    const B: u8 = 2;

    type M = Motions<u8, u8>;

    fn motion(x: f64, ms: u32) -> MotionEvent {
        MotionEvent {
            location: (x, 10.0).into(),
            serial: SERIAL_COUNTER.next_serial(),
            time: InputTime::from_millis(ms),
        }
    }

    fn relative(dx: f64, ux: f64, ms: u32) -> RelativeMotionEvent {
        RelativeMotionEvent {
            delta: (dx, -dx).into(),
            delta_unaccel: (ux, -ux).into(),
            time: InputTime::from_millis(ms),
        }
    }

    #[test]
    fn many_motions_deliver_once_at_the_last_position_and_time() {
        let mut m = M::default();
        for i in 0..50 {
            assert!(
                m.motion(A, motion(f64::from(i), i)).is_none(),
                "nothing delivered early"
            );
        }
        let Some(Held::Pointer {
            to,
            motion: Some(last),
            relative: None,
        }) = m.take(Instant::now())
        else {
            panic!("expected one held pointer motion");
        };
        assert_eq!(to, A);
        assert_eq!(last.location, (49.0, 10.0).into());
        assert_eq!(last.time, InputTime::from_millis(49));
        assert!(m.take(Instant::now()).is_none(), "expected nothing left");
    }

    #[test]
    fn relative_deltas_sum_exactly_with_the_latest_time() {
        let mut m = M::default();
        m.relative(A, relative(0.5, 0.25, 1));
        m.motion(A, motion(3.0, 1));
        m.relative(A, relative(1.25, 1.0, 2));
        m.relative(A, relative(-0.125, 0.5, 3));
        let Some(Held::Pointer {
            relative: Some(sum),
            motion: Some(_),
            ..
        }) = m.take(Instant::now())
        else {
            panic!("expected held relative and absolute motion");
        };
        assert_eq!(sum.delta, (1.625, -1.625).into());
        assert_eq!(sum.delta_unaccel, (1.75, -1.75).into());
        assert_eq!(sum.time, InputTime::from_millis(3));
    }

    #[test]
    fn motion_for_another_surface_hands_back_the_held_one_first() {
        let mut m = M::default();
        m.motion(A, motion(1.0, 1));
        let earlier = m.motion(B, motion(2.0, 2));
        assert!(
            matches!(earlier, Some(Held::Pointer { to: A, motion: Some(ref e), .. }) if e.location.x == 1.0),
            "expected A's motion at x 1 handed back, got {earlier:?}"
        );
        assert!(m.frame_held(&B), "expected B's motion held");
        assert!(!m.frame_held(&A), "expected nothing held for A");
    }

    #[test]
    fn a_drag_and_a_pointer_motion_never_share_a_delivery() {
        let mut m = M::default();
        m.motion(A, motion(1.0, 1));
        assert!(
            m.drag(A, (4.0, 4.0).into(), InputTime::from_millis(2), None)
                .is_some()
        );
        assert!(
            m.drag(A, (5.0, 4.0).into(), InputTime::from_millis(3), None)
                .is_none()
        );
        let Some(Held::Drag { at, time, .. }) = m.take(Instant::now()) else {
            panic!("expected a held drag motion");
        };
        assert_eq!((at, time), ((5.0, 4.0).into(), InputTime::from_millis(3)));
    }

    #[test]
    fn an_x11_drag_delivers_once_through_its_offer_at_the_last_position() {
        const OFFER: u8 = 7;
        let mut m = M::default();
        for i in 0..50 {
            let at = (f64::from(i), 4.0).into();
            assert!(
                m.drag(A, at, InputTime::from_millis(i), Some(OFFER))
                    .is_none(),
                "nothing delivered early"
            );
        }
        let Some(Held::Drag {
            to: A,
            at,
            time,
            x11: Some(OFFER),
        }) = m.take(Instant::now())
        else {
            panic!("expected one held drag motion for A through offer 7");
        };
        assert_eq!((at, time), ((49.0, 4.0).into(), InputTime::from_millis(49)));
        assert!(m.take(Instant::now()).is_none(), "expected nothing left");
    }

    #[test]
    fn a_frame_closing_held_motion_is_left_to_the_delivery() {
        let mut m = M::default();
        assert!(
            !m.frame_held(&A),
            "expected a frame with nothing held to go"
        );
        m.motion(A, motion(1.0, 1));
        assert!(
            m.frame_held(&A),
            "expected the frame folded into the delivery"
        );
        m.take(Instant::now());
        assert!(!m.frame_held(&A), "expected the next frame to go");
    }

    #[test]
    fn delivery_waits_out_the_refresh_interval_after_the_last() {
        let interval = Duration::from_micros(6250);
        let mut m = M::default();
        let t0 = Instant::now();
        assert_eq!(
            m.wait(t0, interval),
            Duration::ZERO,
            "a first motion goes at once"
        );
        m.motion(A, motion(1.0, 1));
        m.take(t0);
        let t1 = t0 + Duration::from_millis(2);
        assert_eq!(m.wait(t1, interval), Duration::from_micros(4250));
        assert_eq!(m.wait(t0 + interval * 2, interval), Duration::ZERO);
    }

    #[test]
    fn motion_is_paced_by_the_output_under_the_pointer() {
        let left = (Rectangle::new((0, 0).into(), (1920, 1080).into()), 60_000);
        let right = (
            Rectangle::new((1920, 0).into(), (1920, 1080).into()),
            144_000,
        );
        let both = &[left, right];
        assert_eq!(pace_refresh(both, Some((100.0, 100.0).into())), 60_000);
        assert_eq!(pace_refresh(both, Some((2000.0, 100.0).into())), 144_000);
        assert_eq!(
            pace_refresh(both, None),
            144_000,
            "expected the fastest output with no pointer location"
        );
        assert_eq!(
            pace_refresh(both, Some((-5.0, 100.0).into())),
            144_000,
            "expected the fastest output off every output"
        );
        assert_eq!(pace_refresh(&[], None), 0);
    }
}
