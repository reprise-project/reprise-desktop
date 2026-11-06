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
        wayland_protocols::wp::presentation_time::server::wp_presentation_feedback::Kind as Feedback,
        wayland_server::backend::GlobalId,
    },
    utils::{Monotonic, Transform},
    wayland::presentation::Refresh,
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

/// What a virtual screen is for: the standby holding the desktop with no monitor, or one of the
/// screens a remote client works on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Standby,
    Remote,
}

#[derive(Default)]
pub struct VirtualOutputs {
    screens: Vec<Screen>,
    standbys_made: u32,
    remotes_made: u32,
}

struct Screen {
    output: Output,
    kind: Kind,
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
/// lays out. A Remote screen's position is its place in the remote block (`output::place_block`).
pub fn create(state: &mut State, name: &str, spec: Spec, kind: Kind) -> Output {
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
    output.user_data().insert_if_missing(|| kind);
    crate::output::set_placement(&output, Some(spec.position.into()));
    let global = output.create_global::<State>(&state.dh);
    let v = &mut state.virtual_outputs;
    match kind {
        Kind::Standby => v.standbys_made += 1,
        Kind::Remote => v.remotes_made += 1,
    }
    v.screens.push(Screen {
        output: output.clone(),
        kind,
        global,
        wants: Some(Wants::Frame),
        timer: None,
        last: None,
        pointer_shown: false,
    });
    state.scene.add_output(&output);
    log::info!(
        "output {name}: virtual {kind:?}, {}x{}@{:.3} scale {} at {:?}",
        spec.size.0,
        spec.size.1,
        f64::from(spec.refresh) / 1000.0,
        spec.scale,
        spec.position
    );
    arm(state, &output);
    output
}

/// A name no virtual screen of `kind` made this session had: `HEADLESS-1`, `HEADLESS-2`, ... for
/// the standby, `REMOTE-1`, ... for remote screens.
pub fn next_name(state: &State, kind: Kind) -> String {
    let v = &state.virtual_outputs;
    match kind {
        Kind::Standby => format!("HEADLESS-{}", v.standbys_made + 1),
        Kind::Remote => format!("REMOTE-{}", v.remotes_made + 1),
    }
}

/// Run virtual `output` at `size` and `scale`, as a monitor's mode change does, unless it already
/// does; the caller lays out once for a batch.
pub fn resize(state: &mut State, output: &Output, size: (i32, i32), scale: f64) {
    if screen_mut(state, output).is_none() {
        return;
    }
    let old = output.current_mode();
    let same_scale = output.current_scale().fractional_scale() == scale;
    if same_scale && old.is_some_and(|m| m.size == size.into()) {
        return;
    }
    let mode = Mode {
        size: size.into(),
        refresh: old.map_or(60_000, |m| m.refresh),
    };
    if let Some(old) = old.filter(|&m| m != mode) {
        output.delete_mode(old);
    }
    output.set_preferred(mode);
    output.change_current_state(Some(mode), None, Some(Scale::Fractional(scale)), None);
    log::info!(
        "output {}: virtual, now {}x{} scale {scale}",
        output.name(),
        size.0,
        size.1
    );
    crate::capture::constraints_changed(state, output);
    want_tick(state, output, Wants::Frame);
}

/// The remote screens, in the order they were made.
pub fn remote_outputs(state: &State) -> Vec<Output> {
    (state.virtual_outputs.screens.iter())
        .filter(|s| s.kind == Kind::Remote)
        .map(|s| s.output.clone())
        .collect()
}

/// Where the remote block starts: the rightmost right edge of the other outputs, at y 0.
pub fn remote_origin(state: &State) -> (i32, i32) {
    let right = (state.scene.outputs().iter())
        .filter(|o| !is_remote(o))
        .filter_map(reprisescene::output_rect)
        .map(|r| r.loc.x + r.size.w)
        .max();
    (right.unwrap_or(0), 0)
}

/// Whether `output` is a remote screen.
pub fn is_remote(output: &Output) -> bool {
    output.user_data().get::<Kind>() == Some(&Kind::Remote)
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

/// A frame with nothing to scan out: it does everything a monitor's flip does (animations, covers,
/// presentation feedback, frame callbacks, captures) at the tick.
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
    let states = Default::default();
    (state.scene).update_surfaces(&pass, &states);
    let mut feedback = state.scene.take_presentation_feedback(&pass, &states);
    let refresh = interval(captured, state.config.remote_desktop.frame_rate);
    feedback.presented::<_, Monotonic>(shows_at, Refresh::fixed(refresh), 0, Feedback::empty());
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
    use crate::harness::{Harness, Presented};

    fn remote(h: &mut Harness, size: (i32, i32), scale: f64, at: (i32, i32)) -> Output {
        let spec = Spec {
            size,
            refresh: 60_000,
            scale,
            transform: Transform::Normal,
            position: at,
        };
        let name = next_name(&h.state, Kind::Remote);
        let o = create(&mut h.state, &name, spec, Kind::Remote);
        h.state.update_layout();
        h.dispatch();
        o
    }

    fn names(h: &Harness) -> Vec<String> {
        h.state.scene.outputs().iter().map(|o| o.name()).collect()
    }

    #[test]
    fn a_remote_output_resizes_and_tells_its_captures() {
        let mut h = Harness::new();
        let o = remote(&mut h, (1920, 1080), 1.0, (0, 0));
        let mut client = h.connect_capture((1920, 1080));
        let _session = client.ext_session(&mut h);
        assert_eq!(client.buffer_sizes(), [(1920, 1080)]);
        resize(&mut h.state, &o, (2560, 1440), 1.5);
        h.state.update_layout();
        client.roundtrip(&mut h);
        assert_eq!(o.current_mode().map(|m| m.size), Some((2560, 1440).into()));
        assert_eq!(o.current_scale().fractional_scale(), 1.5);
        assert_eq!(o.modes().len(), 1, "a resized screen lists its one mode");
        assert_eq!(
            client.buffer_sizes().last(),
            Some(&(2560, 1440)),
            "the capture gets a 2560x1440 buffer"
        );
    }

    #[test]
    fn resizing_to_the_mode_it_runs_leaves_its_captures_alone() {
        let mut h = Harness::new();
        let o = remote(&mut h, (1920, 1080), 1.0, (0, 0));
        let mut client = h.connect_capture((1920, 1080));
        let _session = client.ext_session(&mut h);
        resize(&mut h.state, &o, (1920, 1080), 1.0);
        h.state.update_layout();
        client.roundtrip(&mut h);
        assert_eq!(
            client.buffer_sizes(),
            [(1920, 1080)],
            "one buffer size, sent once"
        );
    }

    #[test]
    fn remote_outputs_are_named_apart_from_the_standby() {
        let mut h = Harness::new();
        crate::standby::rdp_changed(&mut h.state, true);
        h.dispatch();
        remote(&mut h, (1920, 1080), 1.0, (0, 0));
        remote(&mut h, (1920, 1080), 1.0, (1920, 0));
        assert_eq!(names(&h), ["HEADLESS-1", "REMOTE-1", "REMOTE-2"]);
        assert_eq!(next_name(&h.state, Kind::Standby), "HEADLESS-2");
        let remotes = remote_outputs(&h.state);
        assert_eq!(remotes.len(), 2);
        assert!(remotes.iter().all(is_remote));
        assert!(!is_remote(&h.state.scene.outputs()[0]), "the standby");
    }

    #[test]
    fn no_standby_while_remote_outputs_hold_the_desktop() {
        let mut h = Harness::new();
        let dp = h.output("DP-1", 1920, 1080);
        crate::standby::rdp_changed(&mut h.state, true);
        h.dispatch();
        remote(&mut h, (1920, 1080), 1.0, (0, 0));
        h.state.scene.remove_output(&dp);
        h.state.output_removed(&dp);
        h.dispatch();
        assert_eq!(names(&h), ["REMOTE-1"]);
    }

    #[test]
    fn remote_outputs_sit_right_of_the_physical_layout() {
        let mut h = Harness::new();
        let dp = h.output("DP-1", 1920, 1080);
        let left = remote(&mut h, (1280, 1024), 1.0, (-1280, 0));
        let right = remote(&mut h, (1280, 1024), 1.0, (0, 0));
        assert_eq!(dp.current_location(), (0, 0).into(), "DP-1 stays put");
        assert_eq!(left.current_location(), (1920, 0).into());
        assert_eq!(right.current_location(), (3200, 0).into());
    }

    #[test]
    fn a_standby_frame_presents_the_commit_at_its_interval() {
        let mut h = Harness::new();
        crate::standby::rdp_changed(&mut h.state, true);
        h.dispatch();
        let mut c = h.connect();
        let t = h.map_toplevel(&mut c, "test.app", 300, 200);
        h.run_for(1100);
        let surface = c.side.shells[t.shell].surface.clone();
        let feedback = c.presentation_feedback(&surface);
        h.draw_self(&mut c, t, 0xff00_8000);
        h.run_for(1100);
        h.roundtrip(&mut c);
        assert_eq!(
            *feedback.lock().unwrap(),
            Some(Presented::Shown {
                refresh_ns: 1_000_000_000
            }),
            "shown at the next tick, an uncaptured standby's second apart"
        );
    }

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
