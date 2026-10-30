use smithay::{
    output::Output,
    reexports::calloop::timer::{TimeoutAction, Timer},
    utils::{Logical, Point, Rectangle},
};

use crate::compositor::State;
use crate::glass::{Drawn, Under};

use super::render::render;
use super::{Head, head_of, surface_mut};

pub fn schedule(state: &mut State, output: &Output) {
    state.toplevels.dirty = true;
    request(state, output, Ask::Scene);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ask {
    Scene,
    Content,
    /// A program's commit that changed only pixels.
    Commit(Drawn),
    Pointer,
    /// A new cursor image: drawn over everything, so glass has nothing new under it.
    Cursor,
    /// Glass a held frame left behind its own window's program (`GlassCtx::owes`): blurs at once
    /// without moving `[glass] blur_rate`'s cap.
    CatchUp,
    /// The frame a timed commit shows in: the commit asks for what it changes once it applies.
    Timed,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Asked {
    pub(super) redraw: bool,
    pub(super) pointer_only: bool,
    /// Only `Ask::CatchUp` asked.
    pub(super) catch_up: bool,
    pub(super) scene: bool,
    /// Where the asks may have changed what glass shows: nowhere for the pointer and the cursor.
    pub(super) backdrop: Under,
}

impl Asked {
    pub(super) fn add(&mut self, ask: Ask) {
        self.pointer_only = (self.pointer_only || !self.redraw) && ask == Ask::Pointer;
        self.catch_up = (self.catch_up || !self.redraw) && ask == Ask::CatchUp;
        self.redraw = true;
        self.scene |= ask == Ask::Scene;
        self.backdrop = self.backdrop.and(match ask {
            Ask::Scene | Ask::Content => Under::Anywhere,
            Ask::Commit(d) => Under::Within(d),
            Ask::Pointer | Ask::Cursor | Ask::CatchUp | Ask::Timed => Under::Nothing,
        });
    }
}

#[cfg(test)]
thread_local! {
    static ASKED: std::cell::RefCell<Vec<(String, Ask)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The asks made on this thread since the last call, by output name.
#[cfg(test)]
pub fn take_asks() -> Vec<(String, Ask)> {
    ASKED.with_borrow_mut(std::mem::take)
}

fn request(state: &mut State, output: &Output, ask: Ask) {
    #[cfg(test)]
    ASKED.with_borrow_mut(|a| a.push((output.name(), ask)));
    let Some(head) = head_of(output) else {
        return;
    };
    let Some(s) = surface_mut(state, head) else {
        return;
    };
    if !s.needs_redraw {
        s.asked_at = Some(std::time::Instant::now());
    }
    let mut asked = Asked {
        redraw: s.needs_redraw,
        pointer_only: s.pointer_only,
        catch_up: s.catch_up,
        scene: s.scene_changed,
        backdrop: s.backdrop_changed,
    };
    asked.add(ask);
    s.needs_redraw = asked.redraw;
    s.pointer_only = asked.pointer_only;
    s.catch_up = asked.catch_up;
    s.scene_changed = asked.scene;
    s.backdrop_changed = asked.backdrop;
    queue_render(state, head);
}

/// Whether a pointer ask needs a render on the output at `geo`: `pointer` is on it or its last
/// frame `shows` it, or a zoom follows it. Any other output would render an empty frame for every
/// pointer report; a scene edit asks the outputs it changes itself.
pub(super) fn pointer_ask_renders(
    geo: Option<Rectangle<i32, Logical>>,
    pointer: Point<f64, Logical>,
    shows: bool,
    zooms: bool,
) -> bool {
    shows || zooms || geo.is_some_and(|g| g.to_f64().contains(pointer))
}

/// Render `head` from an idle callback, unless one is armed or a frame is in flight. A frame
/// asked from an idle callback renders on the next turn (`State::insert_idle_woken`).
pub(super) fn queue_render(state: &mut State, head: Head) {
    let Some(s) = surface_mut(state, head) else {
        return;
    };
    if !s.track.idle() || s.idle_scheduled {
        return;
    }
    s.idle_scheduled = true;
    state.insert_idle_woken(move |state| {
        if let Some(s) = surface_mut(state, head) {
            s.idle_scheduled = false;
        }
        render(state, head);
    });
}

/// Ask for a desktop water step's frame on `output` that starts at or after `at`
/// (`framepace::ask_at`). Glass re-blurs the moving water at `[glass] blur_rate`, like a
/// program's content.
pub fn schedule_step(state: &mut State, output: &Output, at: std::time::Instant) {
    let Some(head) = head_of(output) else {
        return;
    };
    let Some(s) = surface_mut(state, head) else {
        return;
    };
    let interval = crate::framepace::interval(output.current_mode().map_or(0, |m| m.refresh));
    let now = std::time::Instant::now();
    match crate::framepace::ask_at(at, now, s.last_flip, !s.track.idle(), interval) {
        Some(wait) => arm_frame(state, head, output, now + wait, Ask::Content),
        None => request(state, output, Ask::Content),
    }
}

/// Ask for the frame on `output` that shows at the first refresh at or after `at`
/// (`framepace::start_for`): a timed commit's.
pub fn schedule_shows_at(state: &mut State, output: &Output, at: std::time::Instant) {
    let Some(head) = head_of(output) else {
        return;
    };
    let Some(s) = surface_mut(state, head) else {
        return;
    };
    let interval = crate::framepace::interval(output.current_mode().map_or(0, |m| m.refresh));
    let start = crate::framepace::start_for(at, s.last_flip, interval);
    if start <= std::time::Instant::now() {
        request(state, output, Ask::Timed);
    } else {
        arm_frame(state, head, output, start, Ask::Timed);
    }
}

/// Ask for a frame on `output` at `fires`; of two such asks, the earlier stands, and the
/// later's source asks again after that frame.
fn arm_frame(state: &mut State, head: Head, output: &Output, fires: std::time::Instant, ask: Ask) {
    let Some(s) = surface_mut(state, head) else {
        return;
    };
    let armed = s.frame_timer;
    match armed {
        Some((_, at, _)) if at <= fires => return,
        Some((t, _, _)) => state.handle.remove(t),
        None => {}
    }
    let wait = fires.saturating_duration_since(std::time::Instant::now());
    let token = state
        .handle
        .insert_source(Timer::from_duration(wait), move |_, _, state| {
            let output = surface_mut(state, head).map(|s| {
                s.frame_timer = None;
                s.output.clone()
            });
            if let Some(output) = output {
                request(state, &output, ask);
            }
            TimeoutAction::Drop
        });
    let Some(s) = surface_mut(state, head) else {
        return;
    };
    match token {
        Ok(t) => s.frame_timer = Some((t, fires, ask)),
        Err(e) => {
            s.frame_timer = None;
            log::warn!("frame timer: {}", e.error);
            request(state, output, ask);
        }
    }
}

/// Ask for a pointer frame (`Ask::Pointer`) on the outputs where the pointer, now at `pointer`,
/// needs one (`pointer_ask_renders`). The caller reads `pointer`: inside a pointer grab's
/// callbacks, reading it here would lock the pointer twice.
pub fn schedule_pointer(state: &mut State, pointer: Point<f64, Logical>) {
    let outputs: Vec<Output> = state.scene.outputs().to_vec();
    for o in &outputs {
        let wanted = head_of(o)
            .and_then(|head| state.backend.as_ref()?.surfaces.get(&head))
            .is_some_and(|s| {
                pointer_ask_renders(
                    reprisescene::output_rect(o),
                    pointer,
                    s.pointer_shown,
                    state.magnifier.zooms(o),
                )
            });
        if wanted {
            request(state, o, Ask::Pointer);
        }
    }
}

/// The outputs a new cursor image changes: those whose last frame drew the cursor and those a
/// zoom follows it on. An output the pointer just reached has a pointer ask pending
/// (`schedule_pointer`), whose frame draws the image as it is by then. Reads no pointer state:
/// smithay sets the cursor from inside the pointer's lock.
pub fn cursor_outputs(state: &State) -> Vec<Output> {
    let backend = state.backend.as_ref();
    let shown = |o: &Output| {
        head_of(o)
            .and_then(|head| backend?.surfaces.get(&head))
            .is_some_and(|s| s.pointer_shown)
    };
    let outputs = state.scene.outputs().iter();
    outputs
        .filter(|o| shown(o) || state.magnifier.zooms(o))
        .cloned()
        .collect()
}

pub fn schedule_all(state: &mut State, ask: Ask) {
    let outputs: Vec<Output> = state.scene.outputs().to_vec();
    schedule_on(state, &outputs, ask);
}

pub fn schedule_on(state: &mut State, outputs: &[Output], ask: Ask) {
    // A scene change may move, map, restate or refocus a window (`refresh_toplevels`).
    if ask == Ask::Scene {
        state.toplevels.dirty = true;
    }
    for o in outputs {
        request(state, o, ask);
    }
}
