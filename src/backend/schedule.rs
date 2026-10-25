use smithay::{
    output::Output,
    reexports::{
        calloop::timer::{TimeoutAction, Timer},
        drm::control::crtc,
    },
    utils::{Logical, Point, Rectangle},
};

use crate::compositor::State;
use crate::glass::Under;

use super::render::render;
use super::{crtc_of, surface_mut};

pub fn schedule(state: &mut State, output: &Output) {
    request(state, output, Ask::Scene);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ask {
    Scene,
    Content,
    /// A program's commit that changed only pixels within this box (layout).
    Commit(Rectangle<i32, Logical>),
    Pointer,
    /// A new cursor image: drawn over everything, so glass has nothing new under it.
    Cursor,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Asked {
    pub(super) redraw: bool,
    pub(super) pointer_only: bool,
    pub(super) scene: bool,
    /// Where the asks may have changed what glass shows: nowhere for the pointer and the cursor.
    pub(super) backdrop: Under,
}

impl Asked {
    pub(super) fn add(&mut self, ask: Ask) {
        self.pointer_only = (self.pointer_only || !self.redraw) && ask == Ask::Pointer;
        self.redraw = true;
        self.scene |= ask == Ask::Scene;
        self.backdrop = self.backdrop.and(match ask {
            Ask::Scene | Ask::Content => Under::Anywhere,
            Ask::Commit(r) => Under::Within(r),
            Ask::Pointer | Ask::Cursor => Under::Nothing,
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
    let Some(crtc) = crtc_of(output) else {
        return;
    };
    let Some(s) = surface_mut(state, crtc) else {
        return;
    };
    let mut asked = Asked {
        redraw: s.needs_redraw,
        pointer_only: s.pointer_only,
        scene: s.scene_changed,
        backdrop: s.backdrop_changed,
    };
    asked.add(ask);
    s.needs_redraw = asked.redraw;
    s.pointer_only = asked.pointer_only;
    s.scene_changed = asked.scene;
    s.backdrop_changed = asked.backdrop;
    queue_render(state, crtc);
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

/// Render `crtc` from an idle callback, unless one is armed or a frame is in flight. A frame
/// asked from an idle callback renders on the next turn (`State::insert_idle_woken`).
pub(super) fn queue_render(state: &mut State, crtc: crtc::Handle) {
    let Some(s) = surface_mut(state, crtc) else {
        return;
    };
    if !s.track.idle() || s.idle_scheduled {
        return;
    }
    s.idle_scheduled = true;
    state.insert_idle_woken(move |state| {
        if let Some(s) = surface_mut(state, crtc) {
            s.idle_scheduled = false;
        }
        render(state, crtc);
    });
}

/// Ask for a desktop water step's frame on `output` that starts at or after `at`
/// (`framepace::ask_at`). A wait already armed stands. Glass re-blurs the moving water at
/// `[glass] blur_rate`, like a program's content.
pub fn schedule_step(state: &mut State, output: &Output, at: std::time::Instant) {
    let Some(crtc) = crtc_of(output) else {
        return;
    };
    let Some(s) = surface_mut(state, crtc) else {
        return;
    };
    if s.step_timer.is_some() {
        return;
    }
    let interval = crate::framepace::interval(output.current_mode().map_or(0, |m| m.refresh));
    let now = std::time::Instant::now();
    let Some(wait) = crate::framepace::ask_at(at, now, s.last_flip, !s.track.idle(), interval)
    else {
        request(state, output, Ask::Content);
        return;
    };
    let token = state
        .handle
        .insert_source(Timer::from_duration(wait), move |_, _, state| {
            let output = surface_mut(state, crtc).map(|s| {
                s.step_timer = None;
                s.output.clone()
            });
            if let Some(output) = output {
                request(state, &output, Ask::Content);
            }
            TimeoutAction::Drop
        });
    match token {
        Ok(t) => {
            if let Some(s) = surface_mut(state, crtc) {
                s.step_timer = Some(t);
            }
        }
        Err(e) => {
            log::warn!("step timer: {}", e.error);
            request(state, output, Ask::Content);
        }
    }
}

/// Ask for a pointer frame (`Ask::Pointer`) on the outputs where the pointer, now at `pointer`,
/// needs one (`pointer_ask_renders`). The caller reads `pointer`: inside a pointer grab's
/// callbacks, reading it here would lock the pointer twice.
pub fn schedule_pointer(state: &mut State, pointer: Point<f64, Logical>) {
    let outputs: Vec<Output> = state.scene.outputs().to_vec();
    for o in &outputs {
        let wanted = crtc_of(o)
            .and_then(|crtc| state.backend.as_ref()?.surfaces.get(&crtc))
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
        crtc_of(o)
            .and_then(|crtc| backend?.surfaces.get(&crtc))
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
    for o in outputs {
        request(state, o, ask);
    }
}
