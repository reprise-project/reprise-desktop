//! Connectors for tests: an output in `State::test_heads` is a head that `disable` parks and
//! `enable` drives again, with the one mode the harness gave it.

use smithay::output::Output;

use super::HeadState;
use crate::{compositor::State, output::ModeInfo};

pub(super) fn heads(state: &State) -> Vec<HeadState> {
    let mut heads: Vec<HeadState> = (state.test_heads.iter())
        .map(|o| {
            let enabled = state.scene.outputs().contains(o);
            HeadState {
                output: o.clone(),
                enabled,
                modes: o.preferred_mode().map_or(Vec::new(), |m| {
                    vec![ModeInfo {
                        width: m.size.w,
                        height: m.size.h,
                        refresh_mhz: m.refresh,
                        preferred: true,
                    }]
                }),
                current: enabled.then_some(0),
                vrr_capable: false,
                vrr_on: false,
            }
        })
        .collect();
    heads.sort_by_key(|h| h.output.name());
    heads
}

pub(super) fn enable(state: &mut State, output: &Output) -> bool {
    if !state.test_heads.contains(output) {
        return false;
    }
    if !state.scene.outputs().contains(output) {
        state.scene.add_output(output);
    }
    true
}

pub(super) fn disable(state: &mut State, output: &Output) -> bool {
    if !state.test_heads.contains(output) || !state.scene.outputs().contains(output) {
        return false;
    }
    state.scene.remove_output(output);
    true
}
