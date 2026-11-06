//! Virtual outputs: screens in the layout with no monitor behind them, drawn only into captures.
//! Each frame clock ticks only when asked, at the capture rate while captured, else once a second.

use std::time::{Duration, Instant};

use smithay::{
    output::{Mode, Output, PhysicalProperties, Scale, Subpixel},
    reexports::{
        calloop::{
            RegistrationToken,
            timer::{TimeoutAction, Timer},
        },
        wayland_server::backend::GlobalId,
    },
    utils::Transform,
};

use crate::{backend::Ask, compositor::State};

/// The highest rate a captured virtual screen draws at.
pub const MAX_RATE: u32 = 144;

/// How a new virtual screen runs and where it sits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spec {
    pub size: (i32, i32),
    /// mHz, as `wl_output` gives it.
    pub refresh: i32,
    pub scale: f64,
    pub transform: Transform,
    pub position: (i32, i32),
}

#[derive(Default)]
pub struct VirtualOutputs {
    screens: Vec<Screen>,
    made: u32,
}

struct Screen {
    output: Output,
    global: GlobalId,
    wants: Option<Wants>,
    /// The clock's deadline, armed while something waits for a frame.
    timer: Option<(RegistrationToken, Instant)>,
    last: Option<Instant>,
    /// The last frame's pointer was on it.
    pointer_shown: bool,
}

/// What the next tick does: answer frame callbacks only, or run a whole frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Wants {
    Callbacks,
    Frame,
}

/// What `ask` wants of the next tick; pointer and cursor changes matter only to a capture.
pub fn wants(ask: Ask, captured: bool) -> Option<Wants> {
    match ask {
        Ask::Pointer | Ask::Cursor if !captured => None,
        _ => Some(Wants::Frame),
    }
}

/// The time between a virtual screen's frames: `frame_rate` (1 to [`MAX_RATE`]) while
/// `captured`, else a second.
pub fn interval(captured: bool, frame_rate: u32) -> Duration {
    let hz = if captured {
        frame_rate.clamp(1, MAX_RATE)
    } else {
        1
    };
    Duration::from_secs(1) / hz
}

/// When the next tick fires: one interval after the `last`, and not before `now`.
pub fn next_tick(last: Option<Instant>, interval: Duration, now: Instant) -> Instant {
    last.map_or(now, |l| (l + interval).max(now))
}

/// A new virtual screen named `name` at `spec`, its global made and put in the scene; the caller
/// lays out.
pub fn create(state: &mut State, name: &str, spec: Spec) -> Output {
    let output = Output::new(
        name.into(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "Reprise".into(),
            model: "Virtual".into(),
            serial_number: String::new(),
        },
    );
    let mode = Mode {
        size: spec.size.into(),
        refresh: spec.refresh,
    };
    output.set_preferred(mode);
    output.change_current_state(
        Some(mode),
        Some(spec.transform),
        Some(Scale::Fractional(spec.scale)),
        Some(spec.position.into()),
    );
    crate::backend::mark_virtual(&output);
    crate::output::set_placement(&output, Some(spec.position.into()));
    let global = output.create_global::<State>(&state.dh);
    state.virtual_outputs.made += 1;
    state.virtual_outputs.screens.push(Screen {
        output: output.clone(),
        global,
        wants: Some(Wants::Frame),
        timer: None,
        last: None,
        pointer_shown: false,
    });
    state.scene.add_output(&output);
    log::info!(
        "output {name}: virtual, {}x{}@{:.3} scale {} at {:?}",
        spec.size.0,
        spec.size.1,
        f64::from(spec.refresh) / 1000.0,
        spec.scale,
        spec.position
    );
    arm(state, &output);
    output
}

/// A name no virtual screen made this session had: `HEADLESS-1`, `HEADLESS-2`, ...
pub fn next_name(state: &State) -> String {
    format!("HEADLESS-{}", state.virtual_outputs.made + 1)
}

/// Take `output` out of the scene, stop its clock and captures and retire its global; the caller
/// lays out. False when it is no virtual screen.
pub fn remove(state: &mut State, output: &Output) -> bool {
    let screens = &mut state.virtual_outputs.screens;
    let Some(i) = screens.iter().position(|s| &s.output == output) else {
        return false;
    };
    let s = screens.remove(i);
    if let Some((t, _)) = s.timer {
        state.handle.remove(t);
    }
    state.scene.remove_output(output);
    state.captures.output_gone(output);
    crate::backend::retire_global(state, s.global);
    log::info!("output {}: virtual, gone", output.name());
    true
}

fn screen_mut<'a>(state: &'a mut State, output: &Output) -> Option<&'a mut Screen> {
    (state.virtual_outputs.screens.iter_mut()).find(|s| &s.output == output)
}

/// Whether `output`'s last frame had the pointer on it.
pub fn pointer_shown(state: &State, output: &Output) -> bool {
    (state.virtual_outputs.screens.iter()).any(|s| &s.output == output && s.pointer_shown)
}

/// `backend::schedule` and the like on a virtual screen.
pub fn ask(state: &mut State, output: &Output, ask: Ask) {
    let captured = state.captures.shows_output(output);
    let Some(want) = wants(ask, captured) else {
        return;
    };
    want_tick(state, output, want);
}

/// A commit that shows nothing new: its frame callbacks go at the next tick.
pub fn answer_frames(state: &mut State) {
    let outputs: Vec<Output> = (state.virtual_outputs.screens.iter())
        .map(|s| s.output.clone())
        .collect();
    for o in outputs {
        want_tick(state, &o, Wants::Callbacks);
    }
}

fn want_tick(state: &mut State, output: &Output, want: Wants) {
    let Some(s) = screen_mut(state, output) else {
        return;
    };
    s.wants = s.wants.max(Some(want));
    arm(state, output);
}

/// Arm the clock for the next tick, or earlier than armed when a capture began meanwhile.
fn arm(state: &mut State, output: &Output) {
    let captured = state.captures.shows_output(output);
    let interval = interval(captured, state.config.remote_desktop.frame_rate);
    let now = Instant::now();
    let Some(s) = screen_mut(state, output) else {
        return;
    };
    let at = next_tick(s.last, interval, now);
    match s.timer {
        Some((_, armed)) if armed <= at => return,
        Some((t, _)) => state.handle.remove(t),
        None => {}
    }
    let o = output.clone();
    let token = state
        .handle
        .insert_source(Timer::from_deadline(at), move |_, _, state| {
            tick(state, &o);
            TimeoutAction::Drop
        });
    let Some(s) = screen_mut(state, output) else {
        return;
    };
    match token {
        Ok(t) => s.timer = Some((t, at)),
        Err(e) => {
            s.timer = None;
            log::warn!("{}: frame clock: {}", output.name(), e.error);
        }
    }
}

fn tick(state: &mut State, output: &Output) {
    let Some(s) = screen_mut(state, output) else {
        return;
    };
    s.timer = None;
    s.last = Some(Instant::now());
    match s.wants.take() {
        None => {}
        Some(Wants::Callbacks) => crate::backend::send_frame_callbacks(state, output),
        Some(Wants::Frame) => frame(state, output),
    }
}

/// A frame with nothing to scan out: what animates steps, surfaces enter or leave, the lock, fade
/// and sleep covers count it shown, frame callbacks go and the captures waiting for it fill.
fn frame(state: &mut State, output: &Output) {
    state.settle_moves();
    let shows_at = state.now();
    crate::surfaceproto::before_frame(state, output, shows_at);
    crate::backend::advance(state, output);
    let captured = state.captures.shows_output(output);
    if captured && let Some(b) = state.backend.as_mut() {
        state.water.step(&mut b.renderer, output);
    }
    let pointer = state.pointer_location();
    let on = reprisescene::output_rect(output).is_some_and(|g| g.to_f64().contains(pointer));
    if let Some(s) = screen_mut(state, output) {
        s.pointer_shown = on;
    }
    let pass = state.scene.surface_pass(output);
    (state.scene).update_surfaces(&pass, &Default::default());
    crate::sessionlock::rendered(state, output, true, false);
    crate::sessionlock::presented(state, output);
    crate::power::rendered(state, output, true, false);
    crate::power::presented(state, output);
    crate::sessionfade::presented(state, output);
    crate::backend::send_frame_callbacks(state, output);
    crate::capture::output_drawn(state, output);
    if captured && let Some(at) = state.water.next_step(output) {
        crate::backend::schedule_step(state, output, at);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_captured_screen_draws_at_the_frame_rate_up_to_144_and_else_once_a_second() {
        assert_eq!(interval(true, 60), Duration::from_secs(1) / 60);
        assert_eq!(interval(true, 144), Duration::from_secs(1) / 144);
        assert_eq!(interval(true, 360), Duration::from_secs(1) / 144);
        assert_eq!(interval(true, 0), Duration::from_secs(1));
        assert_eq!(interval(false, 60), Duration::from_secs(1));
        assert_eq!(interval(false, 144), Duration::from_secs(1));
    }

    #[test]
    fn the_next_tick_is_an_interval_after_the_last_and_never_in_the_past() {
        let now = Instant::now();
        let i = Duration::from_millis(100);
        assert_eq!(next_tick(None, i, now), now);
        assert_eq!(next_tick(Some(now - i * 3), i, now), now);
        assert_eq!(
            next_tick(Some(now - Duration::from_millis(40)), i, now),
            now + Duration::from_millis(60)
        );
    }

    #[test]
    fn pointer_and_cursor_changes_draw_only_a_captured_screen() {
        for ask in [Ask::Pointer, Ask::Cursor] {
            assert_eq!(wants(ask, false), None, "{ask:?}");
            assert_eq!(wants(ask, true), Some(Wants::Frame), "{ask:?}");
        }
        for ask in [Ask::Scene, Ask::Content, Ask::Timed, Ask::CatchUp] {
            assert_eq!(wants(ask, false), Some(Wants::Frame), "{ask:?}");
        }
    }

    #[test]
    fn a_frame_ask_outranks_a_callback_ask() {
        assert_eq!(
            Some(Wants::Callbacks).max(Some(Wants::Frame)),
            Some(Wants::Frame)
        );
        assert_eq!(None.max(Some(Wants::Callbacks)), Some(Wants::Callbacks));
    }
}
