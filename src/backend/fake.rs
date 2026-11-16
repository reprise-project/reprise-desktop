//! Connectors for tests: an output in `State::test_heads` is a head that `disable` parks and
//! `enable` drives again, with the modes the harness gave it. `State::test_modes` says what a
//! mode change does.

use smithay::output::Output;

use super::{HeadState, VrrSet};
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
                vrr_capable: state.test_vrr.capable,
                vrr_on: state.test_vrr.on.contains(&o.name()),
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

/// What test heads do with VRR: `capable` ones take it unless `busy`.
#[derive(Debug, Default)]
pub struct Vrr {
    pub capable: bool,
    pub busy: bool,
    /// The outputs running VRR.
    pub on: Vec<String>,
    /// Each change taken, in order.
    pub set: Vec<(String, bool)>,
}

fn driven(state: &State, output: &Output) -> bool {
    state.test_heads.contains(output) && state.scene.outputs().contains(output)
}

pub(super) fn try_vrr(state: &mut State, output: &Output, on: bool) -> VrrSet {
    if !driven(state, output) {
        return VrrSet::Refused;
    }
    let (v, name) = (&mut state.test_vrr, output.name());
    if v.on.contains(&name) == on {
        return VrrSet::Done;
    }
    if on && !v.capable {
        return VrrSet::Refused;
    }
    if v.busy {
        return VrrSet::Busy;
    }
    match on {
        true => v.on.push(name.clone()),
        false => v.on.retain(|n| *n != name),
    }
    v.set.push((name, on));
    VrrSet::Done
}

pub(super) fn vrr_of(state: &State, output: &Output) -> Option<(bool, bool)> {
    let v = &state.test_vrr;
    driven(state, output).then(|| (v.capable, v.on.contains(&output.name())))
}

/// The display goes idle: mode and VRR changes take, and the ones refused while busy run as
/// `Step::Mode` and `on_quiet` run them.
pub fn idle(state: &mut State) {
    state.test_modes = Modes::Taken;
    state.test_vrr.busy = false;
    if state.vrr_stale {
        state.sync_vrr();
    }
    for (output, index) in std::mem::take(&mut state.test_mode_retries) {
        if set_mode(state, &output, index) {
            state.update_layout();
        }
    }
}
