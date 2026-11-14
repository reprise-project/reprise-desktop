//! Connectors for tests: an output in `State::test_heads` is a head that `disable` parks and
//! `enable` drives again, with the modes the harness gave it. `State::test_modes` says what a
//! mode change does.

use smithay::output::Output;

use super::HeadState;
use crate::{compositor::State, output::ModeInfo};

pub(super) fn heads(state: &State) -> Vec<HeadState> {
    let mut heads: Vec<HeadState> = (state.test_heads.iter())
        .map(|o| {
            let enabled = state.scene.outputs().contains(o);
            let modes = o.modes();
            HeadState {
                output: o.clone(),
                enabled,
                modes: (modes.iter())
                    .map(|m| ModeInfo {
                        width: m.size.w,
                        height: m.size.h,
                        refresh_mhz: m.refresh,
                        preferred: o.preferred_mode() == Some(*m),
                    })
                    .collect(),
                current: (modes.iter())
                    .position(|m| Some(*m) == o.current_mode())
                    .filter(|_| enabled),
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

/// What a test head does with a mode change.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Modes {
    /// Refused: no device to set it on.
    #[default]
    Refused,
    /// Refused until `idle` runs the retries, as while a commit holds the display.
    Busy,
    Taken,
}

pub(super) fn set_mode(state: &mut State, output: &Output, index: usize) -> bool {
    let Some(&mode) = output.modes().get(index) else {
        return false;
    };
    if !state.test_heads.contains(output) || state.test_modes != Modes::Taken {
        return false;
    }
    output.change_current_state(Some(mode), None, None, None);
    true
}

pub(super) fn retry_mode(state: &mut State, output: &Output, index: usize) {
    if state.test_modes == Modes::Busy {
        state.test_mode_retries.push((output.clone(), index));
    }
}

/// The display goes idle: mode changes take, and the ones `retry_mode` queued run as
/// `Step::Mode` runs them.
pub fn idle(state: &mut State) {
    state.test_modes = Modes::Taken;
    for (output, index) in std::mem::take(&mut state.test_mode_retries) {
        if set_mode(state, &output, index) {
            state.update_layout();
        }
    }
}
