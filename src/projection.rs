//! Applying a `Projection` (Extend, Duplicate, Only 1, Only 2) from the display switcher: the heads
//! as `repriseconf::projection` plans them, run through `State::apply_head` once modes settle, then
//! saved as the connected set's layout.

use std::time::Duration;

use repriseconf::projection::{self, Mode, Projection, Screen};
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::utils::Transform;

use crate::backend::HeadState;
use crate::compositor::State;
use crate::output::HeadRequest;

/// How long a pick waits for modes to settle (a modeset still committing) before it is dropped.
const SETTLE_TRIES: u32 = 40;
const SETTLE_STEP: Duration = Duration::from_millis(50);

fn screen(h: &HeadState) -> Screen {
    let o = &h.output;
    let at = o.current_location();
    Screen {
        name: o.name(),
        modes: (h.modes.iter())
            .map(|m| Mode {
                width: m.width,
                height: m.height,
                refresh_mhz: m.refresh_mhz,
                preferred: m.preferred,
            })
            .collect(),
        current: h.current.filter(|_| h.enabled),
        enabled: h.enabled,
        position: (at.x, at.y),
        scale: o.current_scale().fractional_scale(),
        turned: matches!(
            o.current_transform(),
            Transform::_90 | Transform::_270 | Transform::Flipped90 | Transform::Flipped270
        ),
    }
}

impl State {
    /// The connected monitors as the switcher counts them, in the Display page's order.
    fn screens(&self) -> (Vec<HeadState>, Vec<Screen>) {
        let heads = crate::backend::heads(self);
        let screens = heads.iter().map(screen).collect();
        (heads, screens)
    }

    fn primary_name(&self) -> Option<String> {
        (self.saved_primary().map(str::to_string))
            .or_else(|| self.config.primary_output_name().map(str::to_string))
    }

    /// What the monitors show now, and how many are connected.
    pub fn projection(&self) -> (Projection, usize) {
        let (_, screens) = self.screens();
        (projection::current(&screens), screens.len())
    }

    /// Run `p` on the connected monitors and save the result as their layout; a pick that changes
    /// nothing does nothing.
    pub fn project(&mut self, p: Projection) {
        self.project_when_settled(p, SETTLE_TRIES);
    }

    fn project_when_settled(&mut self, p: Projection, tries: u32) {
        if !crate::backend::session_active(self) {
            log::info!("display switcher: the session is off its seat, {p:?} is dropped");
            return;
        }
        if !crate::backend::modes_settle(self) {
            if tries == 0 {
                log::warn!("display switcher: displays busy, {p:?} is dropped");
                return;
            }
            let retry = Timer::from_duration(SETTLE_STEP);
            let armed = self.handle.insert_source(retry, move |_, _, state| {
                state.project_when_settled(p, tries - 1);
                TimeoutAction::Drop
            });
            if let Err(e) = armed {
                log::warn!("display switcher timer: {}", e.error);
            }
            return;
        }
        let (heads, screens) = self.screens();
        let primary = self.primary_name();
        let Some(plan) = projection::plan(&screens, primary.as_deref(), p) else {
            log::info!("display switcher: {p:?} needs more monitors");
            return;
        };
        if projection::current(&screens) == p {
            return;
        }
        let mut reqs: Vec<(HeadState, HeadRequest)> = (heads.into_iter().zip(plan))
            .map(|(h, w)| {
                let req = HeadRequest {
                    enabled: w.enabled,
                    mode: w.mode.filter(|&i| !h.enabled || Some(i) != h.current),
                    position: Some(w.position),
                    transform: h.output.current_transform(),
                    scale: w.scale,
                    adaptive_sync: h.vrr_on,
                };
                (h, req)
            })
            .collect();
        reqs.sort_by_key(|(_, r)| !r.enabled);
        log::info!("display switcher: {p:?}");
        for (h, req) in &reqs {
            if !self.apply_head(&h.output, req) {
                log::warn!("display switcher: {} refused {p:?}", h.output.name());
                break;
            }
        }
        self.update_layout();
        self.handle.insert_idle(|state| state.save_layout());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::Harness;
    use smithay::output::Output;

    fn pair(second: (i32, i32)) -> (Harness, Output, Output) {
        let mut h = Harness::new();
        let a = h.head("DP-1", 1920, 1080);
        let b = h.head("DP-2", second.0, second.1);
        assert_eq!(h.state.projection(), (Projection::Extend, 2));
        (h, a, b)
    }

    fn project(h: &mut Harness, p: Projection) {
        h.state.project(p);
        h.dispatch();
        crate::state::flush();
    }

    fn saved_places(h: &Harness) -> Vec<(i32, i32)> {
        let text = std::fs::read_to_string(&h.state.saved_layouts.path).unwrap_or_default();
        let layouts = repriseconf::displays::Layouts::parse(&text);
        (layouts.list.iter())
            .flat_map(|l| l.monitors.iter().map(|(_, m)| m.position))
            .collect()
    }

    #[test]
    fn duplicate_is_saved_as_the_pairs_layout() {
        let (mut h, a, b) = pair((1920, 1080));
        project(&mut h, Projection::Duplicate);
        assert_eq!(h.state.projection().0, Projection::Duplicate);
        assert_eq!(b.current_location(), a.current_location());
        assert_eq!(saved_places(&h), [(0, 0), (0, 0)]);
    }

    #[test]
    fn extend_after_duplicate_puts_the_displays_side_by_side_again() {
        let (mut h, a, b) = pair((1920, 1080));
        project(&mut h, Projection::Duplicate);
        project(&mut h, Projection::Extend);
        assert_eq!(h.state.projection().0, Projection::Extend);
        assert_eq!((b.current_location().x, b.current_location().y), (1920, 0));
        assert_eq!(h.state.own_outputs(), [a, b]);
        assert_eq!(h.state.bars.len(), 2);
    }

    #[test]
    fn only_2_turns_the_first_display_off_and_only_1_brings_it_back() {
        let (mut h, a, b) = pair((1920, 1080));
        project(&mut h, Projection::Only(1));
        assert_eq!(h.state.projection(), (Projection::Only(1), 2));
        assert_eq!(h.state.scene.outputs(), std::slice::from_ref(&b));
        project(&mut h, Projection::Only(0));
        assert_eq!(h.state.projection().0, Projection::Only(0));
        assert_eq!(h.state.scene.outputs(), [a]);
    }

    #[test]
    fn one_display_has_nothing_to_extend_or_copy() {
        let mut h = Harness::new();
        let a = h.head("DP-1", 1920, 1080);
        assert_eq!(h.state.projection(), (Projection::Only(0), 1));
        project(&mut h, Projection::Duplicate);
        assert_eq!(h.state.scene.outputs(), [a]);
    }
}
