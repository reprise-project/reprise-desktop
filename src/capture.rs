//! Output and window capture for ext-image-copy-capture-v1 and
//! wlr-screencopy-v1: an output's elements, or a window's client surface
//! tree, rendered into the client's shm or dmabuf buffer. A session's
//! frame is filled at once when its source changed since the last one
//! (and for the first); otherwise it, and every `copy_with_damage`,
//! waits for the output's next drawn frame or the window's next commit,
//! so an idle screen costs a screencast nothing. An output session's
//! frame reports the damage since its previous one; others report all.
//! A pointer cursor session on an output copies the pointer's image and
//! follows it over the output. A session is ongoing (a screencast, not a
//! screenshot) from its second frame.
//!
//! The shm copy follows niri's `render_to_shm` in
//! `src/render_helpers/mod.rs` (GPL-3.0-or-later).

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::Instant;

use smithay::{
    backend::{
        allocator::{Fourcc, Modifier, dmabuf::Dmabuf},
        renderer::{
            Bind, Color32F,
            damage::OutputDamageTracker,
            element::{Element, Kind, surface::render_elements_from_surface_tree},
            gles::GlesRenderer,
            sync::SyncPoint,
        },
    },
    output::{Output, WeakOutput},
    reexports::{
        calloop::timer::{TimeoutAction, Timer},
        wayland_server::protocol::{wl_buffer::WlBuffer, wl_pointer::WlPointer, wl_shm},
    },
    utils::{Buffer, Physical, Point, Rectangle, Scale, Size, Transform},
    wayland::{
        dmabuf::get_dmabuf,
        foreign_toplevel_list::ForeignToplevelHandle,
        image_capture_source::{
            ImageCaptureSource, ImageCaptureSourceHandler, OutputCaptureSourceHandler,
            OutputCaptureSourceState, ToplevelCaptureSourceHandler, ToplevelCaptureSourceState,
        },
        image_copy_capture::{
            BufferConstraints, CaptureFailureReason, CursorSession, CursorSessionRef,
            DmabufConstraints, Frame, FrameRef, ImageCopyCaptureHandler, ImageCopyCaptureState,
            Session, SessionRef,
        },
        shm,
    },
};

use crate::{
    captureborder::{Captured, WlrKey},
    compositor::State,
    readback::{Key, Readbacks, Slot},
    render::{OutputElement, Parts, clear_color, cursor_image, output_elements},
    screencopy::{Screencopy, ScreencopyBuffer, ScreencopyHandler},
    window::WindowId,
};

/// The side of a cursor session's buffer, in logical pixels: room for
/// the largest pointer image the themes ship.
const CURSOR_BOX: i32 = 64;

/// What a capture is drawn at: buffer size, scale and transform.
type View = (Size<i32, Physical>, f64, Transform);

/// The open ext sessions and the frames waiting for an output's next frame.
#[derive(Default)]
pub struct Captures {
    sessions: Vec<Session>,
    cursor_sessions: Vec<CursorSession>,
    waiting: Vec<Waiting>,
    readbacks: Readbacks,
}

enum Waiting {
    Ext { session: SessionRef, frame: Frame },
    Wlr(Screencopy),
}

impl Waiting {
    fn shows(&self) -> Option<Shown> {
        match self {
            Waiting::Ext { session, .. } => shown(&session.source()),
            Waiting::Wlr(s) => Some(Shown::Output(s.info.output.clone())),
        }
    }
}

#[derive(Clone, PartialEq)]
enum Shown {
    Output(Output),
    Window(WindowId),
}

fn shown(source: &ImageCaptureSource) -> Option<Shown> {
    let data = source.user_data();
    if let Some(o) = data.get::<WeakOutput>() {
        return o.upgrade().map(Shown::Output);
    }
    data.get::<WindowId>().map(|w| Shown::Window(*w))
}

impl Captures {
    pub fn output_gone(&mut self, output: &Output) {
        let gone = Some(Shown::Output(output.clone()));
        self.stop(|s| s.is_none() || *s == gone);
    }

    /// Stop every session whose source `stops` holds for, failing its
    /// waiting frames.
    fn stop(&mut self, stops: impl Fn(&Option<Shown>) -> bool) {
        self.waiting.retain(|w| !stops(&w.shows()));
        self.sessions.retain(|s| !stops(&shown(&s.source())));
        self.cursor_sessions.retain(|s| !stops(&shown(&s.source())));
    }

    /// The source `on` drew or committed: its sessions' next frames are
    /// filled at once.
    fn changed(&self, on: &Option<Shown>) {
        for s in self.sessions.iter().filter(|s| shown(&s.source()) == *on) {
            fresh(s).mark();
        }
    }

    /// Whether a session or a waiting frame shows `output`.
    pub fn shows_output(&self, output: &Output) -> bool {
        let on = Some(Shown::Output(output.clone()));
        self.sessions.iter().any(|s| shown(&s.source()) == on)
            || self.waiting.iter().any(|w| w.shows() == on)
    }

    /// What the ongoing ext sessions show, those for the remote desktop server only `with_remote`.
    pub fn targets(&self, with_remote: bool) -> Vec<Captured<Output, WindowId>> {
        self.sessions
            .iter()
            .filter(|s| asked(s).ongoing())
            .filter(|s| with_remote || !for_remote_desktop(s))
            .filter_map(|s| match shown(&s.source())? {
                Shown::Output(o) => Some(Captured::Output(o)),
                Shown::Window(w) => Some(Captured::Window(w)),
            })
            .collect()
    }

    /// The wlr captures with a `copy_with_damage` frame waiting.
    pub fn wlr_waiting(&self) -> Vec<WlrKey> {
        self.waiting
            .iter()
            .filter_map(|w| match w {
                Waiting::Wlr(s) if s.with_damage && s.is_alive() => Some(wlr_key(s)),
                _ => None,
            })
            .collect()
    }
}

/// `output` runs a new mode: its sessions get the new buffer size.
pub fn constraints_changed(state: &mut State, output: &Output) {
    let sessions: Vec<SessionRef> = state
        .captures
        .sessions
        .iter()
        .filter(|s| matches!(shown(&s.source()), Some(Shown::Output(o)) if &o == output))
        .map(|s| (**s).clone())
        .collect();
    for s in sessions {
        if let Some(c) = state.capture_constraints(&s.source()) {
            s.update_constraints(c);
        }
    }
}

pub fn output_drawn(state: &mut State, output: &Output) {
    let on = Some(Shown::Output(output.clone()));
    state.captures.changed(&on);
    for w in take_waiting(state, |s| *s == on) {
        match w {
            Waiting::Ext { session, frame } => fill_ext(state, output, &session, frame),
            Waiting::Wlr(s) => fill_wlr(state, s),
        }
    }
}

/// `w` committed: a new size goes to its sessions as new constraints,
/// and the frames waiting on it are filled.
pub fn window_committed(state: &mut State, w: WindowId) {
    if state.captures.sessions.is_empty() {
        return;
    }
    let on = Some(Shown::Window(w));
    state.captures.changed(&on);
    if let Some(c) = window_constraints(state, w) {
        for s in &state.captures.sessions {
            if shown(&s.source()) == on {
                advertise(s, &c);
            }
        }
    }
    for waiting in take_waiting(state, |s| *s == on) {
        if let Waiting::Ext { session, frame } = waiting {
            fill_window(state, w, &session, frame);
        }
    }
}

pub fn window_gone(state: &mut State, w: WindowId) {
    let on = Some(Shown::Window(w));
    state.captures.stop(|s| *s == on);
    crate::privacy::captures_changed(state);
    crate::captureborder::refresh(state);
}

fn take_waiting(state: &mut State, on: impl Fn(&Option<Shown>) -> bool) -> Vec<Waiting> {
    let (now, rest): (Vec<Waiting>, Vec<Waiting>) = std::mem::take(&mut state.captures.waiting)
        .into_iter()
        .partition(|w| on(&w.shows()));
    state.captures.waiting = rest;
    now
}

/// The pointer moved or changed: tell each cursor session where it is on
/// its output, or that it left.
pub fn pointer_moved(state: &mut State) {
    if state.captures.cursor_sessions.is_empty() {
        return;
    }
    let pointer = state.pointer_location();
    for s in &state.captures.cursor_sessions {
        let Some(Shown::Output(output)) = shown(&s.source()) else {
            continue;
        };
        let at = reprisescene::output_rect(&output)
            .filter(|r| r.to_f64().contains(pointer))
            .map(|r| {
                let scale = output.current_scale().fractional_scale();
                let p = (pointer - r.loc.to_f64()).to_physical(scale).to_i32_round();
                (p.x, p.y).into()
            });
        s.set_cursor_pos(at);
    }
}

enum Target<'a> {
    Shm(&'a WlBuffer),
    Dmabuf(Dmabuf),
}

impl<'a> Target<'a> {
    fn of(buffer: &'a WlBuffer) -> Target<'a> {
        match get_dmabuf(buffer) {
            Ok(d) => Target::Dmabuf(d.clone()),
            Err(_) => Target::Shm(buffer),
        }
    }
}

/// Render `output`'s picture from `origin` (physical pixels from its
/// top-left), `size` large, into `target`, its damage taken from `tracked`
/// when given. The debug overlay shows only with `[debug] overlay_in_captures`.
fn render_into(
    state: &mut State,
    output: &Output,
    origin: Point<i32, Physical>,
    size: Size<i32, Physical>,
    with_cursor: bool,
    target: Target<'_>,
    tracked: Option<&mut SessionDamage>,
) -> Result<Drawn, String> {
    let shown = state.config.debug.overlay_in_captures;
    state.without_debug_panels(shown, |state| {
        render_scene_into(state, output, origin, size, with_cursor, target, tracked)
    })
}

fn render_scene_into(
    state: &mut State,
    output: &Output,
    origin: Point<i32, Physical>,
    size: Size<i32, Physical>,
    with_cursor: bool,
    target: Target<'_>,
    tracked: Option<&mut SessionDamage>,
) -> Result<Drawn, String> {
    let pointer = state.pointer_location();
    let State {
        backend,
        scene,
        cursor,
        saver,
        cursors,
        config,
        glass,
        interaction,
        captures,
        ..
    } = state;
    let cursor = crate::saverview::cursor(saver, cursor);
    let b = backend.as_mut().ok_or("no backend")?;
    crate::gpufence::ready(&b.renderer)?;
    // A capture shows the backdrop as it is now: nothing to hold.
    glass.begin(
        output,
        config.glass,
        crate::glass::quality(interaction, &config.glass),
        crate::glass::Hold::No,
    );
    let renderer = &mut b.renderer;
    let clear = clear_color(&config.theme.colors);
    let wanted = crate::glass::wanted(config);
    glass.update_backdrop(renderer, scene, output, clear, wanted);
    let elements = output_elements(
        renderer,
        Parts {
            scene,
            cursor,
            cursors,
            pointer,
        },
        output,
        origin,
        with_cursor,
    );
    let scale = output.current_scale().fractional_scale();
    let at = (size, scale, output.current_transform());
    let damage = tracked.map(|t| t.since_last(at, clear, &elements));
    let mut drawn = draw(
        renderer,
        &mut captures.readbacks,
        at,
        &elements,
        clear,
        target,
    )?;
    if let Some(damage) = damage {
        drawn.damage = damage;
    }
    Ok(drawn)
}

/// A capture rendered into its buffer, waiting for the GPU to finish.
/// An shm buffer is filled once it has.
struct Drawn {
    sync: SyncPoint,
    shm: Option<ShmCopy>,
    /// What changed since the session's last frame, in buffer pixels.
    damage: Vec<Rectangle<i32, Buffer>>,
}

struct ShmCopy {
    buffer: WlBuffer,
    slot: Slot,
    size: Size<i32, Physical>,
}

fn draw(
    renderer: &mut GlesRenderer,
    readbacks: &mut Readbacks,
    (size, scale, transform): View,
    elements: &[OutputElement],
    clear: Color32F,
    target: Target<'_>,
) -> Result<Drawn, String> {
    let mut tracker = OutputDamageTracker::new(size, scale, transform);
    match target {
        Target::Dmabuf(mut dmabuf) => {
            let mut fb = renderer.bind(&mut dmabuf).map_err(|e| e.to_string())?;
            let result = tracker
                .render_output(renderer, &mut fb, 0, elements, clear)
                .map_err(|e| format!("{e:?}"))?;
            crate::gpufence::issued(renderer, &result.sync);
            Ok(Drawn {
                sync: result.sync,
                shm: None,
                damage: vec![whole(size)],
            })
        }
        Target::Shm(buffer) => {
            let fourcc = shm::with_buffer_contents(buffer, |_, _, data| {
                if data.width != size.w || data.height != size.h {
                    return Err("shm buffer of the wrong size".to_string());
                }
                match data.format {
                    wl_shm::Format::Xrgb8888 => Ok(Fourcc::Xrgb8888),
                    wl_shm::Format::Argb8888 => Ok(Fourcc::Argb8888),
                    f => Err(format!("shm format {f:?} not offered")),
                }
            })
            .map_err(|e| format!("{e:?}"))??;
            let mut slot = readbacks.slot(renderer, Key { size, fourcc })?;
            let mut fb = renderer.bind(slot.texture()).map_err(|e| e.to_string())?;
            tracker
                .render_output(renderer, &mut fb, 0, elements, clear)
                .map_err(|e| format!("{e:?}"))?;
            drop(fb);
            slot.read(renderer)?;
            let sync = crate::gpufence::sync_now(renderer)?;
            crate::gpufence::issued(renderer, &sync);
            Ok(Drawn {
                sync,
                shm: Some(ShmCopy {
                    buffer: buffer.clone(),
                    slot,
                    size,
                }),
                damage: vec![whole(size)],
            })
        }
    }
}

impl ShmCopy {
    fn fill(&self, renderer: &mut GlesRenderer) -> Result<(), String> {
        self.slot.bytes(renderer, |bytes| self.copy(bytes))?
    }

    fn copy(&self, bytes: &[u8]) -> Result<(), String> {
        let row = self.size.w as usize * 4;
        let rows = self.size.h as usize;
        if bytes.len() < row * rows {
            return Err("short texture read".into());
        }
        shm::with_buffer_contents_mut(&self.buffer, |pool, pool_len, data| {
            let offset = usize::try_from(data.offset).map_err(|_| "negative offset")?;
            let stride = usize::try_from(data.stride).map_err(|_| "negative stride")?;
            let end = stride
                .checked_mul(rows.saturating_sub(1))
                .and_then(|n| n.checked_add(row))
                .and_then(|n| n.checked_add(offset));
            if stride < row || end.is_none_or(|end| end > pool_len) {
                return Err("buffer does not fit its pool".to_string());
            }
            for y in 0..rows {
                // SAFETY: `pool` is the client's mapped pool of `pool_len`
                // bytes, and the check above keeps every row inside it;
                // `bytes` holds `rows` rows of `row` bytes.
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        bytes.as_ptr().add(y * row),
                        pool.add(offset + y * stride),
                        row,
                    );
                }
            }
            Ok(())
        })
        .map_err(|e| format!("{e:?}"))?
    }
}

/// Runs `done` once the GPU has finished `drawn` and an shm buffer holds
/// it, or with an error when the render is not done by the deadline. The
/// loop goes on meanwhile.
fn when_done(
    state: &mut State,
    drawn: Drawn,
    done: impl FnOnce(&mut State, Result<(), String>) + 'static,
) {
    let sync = drawn.sync.clone();
    let deadline = crate::gpufence::CAPTURE_DEADLINE;
    crate::gpufence::when_reached(&state.handle, sync, deadline, move |state, reached| {
        let filled = match (reached, &drawn.shm, state.backend.as_mut()) {
            (false, _, _) => Err(format!("the render did not finish in {deadline:?}")),
            (true, None, _) => Ok(()),
            (true, Some(copy), Some(b)) => copy.fill(&mut b.renderer),
            (true, Some(_), None) => Err("no backend".into()),
        };
        // A slot whose render was not reached may still be in flight: dropped, never reused.
        if let (true, Some(copy)) = (reached, drawn.shm) {
            keep(state, copy.slot);
        }
        done(state, filled);
    });
}

/// Returns `slot` to the pool and frees the pool's slots once idle.
fn keep(state: &mut State, slot: Slot) {
    let readbacks = &mut state.captures.readbacks;
    readbacks.give(slot, Instant::now());
    if !readbacks.arm() {
        return;
    }
    let timer = Timer::from_duration(crate::readback::IDLE);
    let armed = state.handle.insert_source(timer, |_, _, state| {
        let next = match state.backend.as_mut() {
            Some(b) => state
                .captures
                .readbacks
                .expire(&mut b.renderer, Instant::now()),
            None => None,
        };
        match next {
            Some(at) => TimeoutAction::ToInstant(at),
            None => {
                state.captures.readbacks.disarm();
                TimeoutAction::Drop
            }
        }
    });
    if let Err(e) = armed {
        log::warn!("capture buffers: no expiry timer: {e}");
        state.captures.readbacks.disarm();
    }
}

/// The scale `w` is captured at: its output's, where its frame's centre
/// lies.
fn window_scale(state: &State, w: WindowId) -> f64 {
    let Some(r) = state.win(w).map(crate::window::Window::frame_rect) else {
        return 1.0;
    };
    state
        .output_at((r.x + r.width / 2) as f64, (r.y + r.height / 2) as f64)
        .map_or(1.0, |o| o.current_scale().fractional_scale())
}

fn window_size(state: &State, w: WindowId) -> Option<Size<i32, Physical>> {
    let win = state.win(w).filter(|w| w.mapped)?;
    let (cw, ch) = win.client_size();
    let size = Size::<i32, smithay::utils::Logical>::from((cw, ch))
        .to_f64()
        .to_physical(window_scale(state, w))
        .to_i32_round();
    (size.w > 0 && size.h > 0).then_some(size)
}

/// Render `w`'s client surface tree, frame and popups left out, its
/// window geometry's top-left at 0,0, into `target`. Under the lock the
/// frame is blank: the window's own surfaces are not what the screen
/// shows.
fn render_window(state: &mut State, w: WindowId, target: Target<'_>) -> Result<Drawn, String> {
    let size = window_size(state, w).ok_or("the window is not mapped")?;
    let scale = window_scale(state, w);
    let locked = state.is_locked();
    let win = state.win(w).ok_or("the window is gone")?;
    let surface = win.wl_surface().clone();
    let (sx, sy) = win.surface_origin();
    let (cx, cy) = win.client_origin();
    let at = Point::<i32, smithay::utils::Logical>::from((sx - cx, sy - cy))
        .to_f64()
        .to_physical(scale)
        .to_i32_round();
    let b = state.backend.as_mut().ok_or("no backend")?;
    crate::gpufence::ready(&b.renderer)?;
    let renderer = &mut b.renderer;
    let readbacks = &mut state.captures.readbacks;
    let elements: Vec<OutputElement> = if locked {
        Vec::new()
    } else {
        render_elements_from_surface_tree(
            renderer,
            &surface,
            at,
            Scale::from(scale),
            1.0,
            Kind::Unspecified,
        )
        .into_iter()
        .map(OutputElement::Surface)
        .collect()
    };
    let at = (size, scale, Transform::Normal);
    draw(
        renderer,
        readbacks,
        at,
        &elements,
        Color32F::TRANSPARENT,
        target,
    )
}

/// Sends `c` to `session` when its size is not the one last sent.
fn advertise(session: &SessionRef, c: &BufferConstraints) {
    if session
        .current_constraints()
        .is_none_or(|old| old.size != c.size)
    {
        session.update_constraints(c.clone());
    }
}

fn fill_window(state: &mut State, w: WindowId, session: &SessionRef, frame: Frame) {
    let buffer = frame.buffer();
    if window_size(state, w).is_some_and(|size| !fits(&buffer, size)) {
        // A move onto an output of another scale resizes the capture without a commit.
        if let Some(c) = window_constraints(state, w) {
            advertise(session, &c);
        }
        return frame.fail(CaptureFailureReason::BufferConstraints);
    }
    let drawn = render_window(state, w, Target::of(&buffer));
    if drawn.is_ok() {
        fresh(session).clear();
    }
    let what = format!("capture of window {w:?}");
    complete(state, drawn, frame, Transform::Normal, what, |_| ());
}

/// Sends `frame` ready with its damage once `drawn` is done, or fails it
/// if the render failed or does not finish; `then` runs first, told
/// whether it went ready.
fn complete(
    state: &mut State,
    drawn: Result<Drawn, String>,
    frame: Frame,
    transform: Transform,
    what: String,
    then: impl FnOnce(bool) + 'static,
) {
    let now = state.now();
    let drawn = match drawn {
        Ok(d) => d,
        Err(e) => {
            log::warn!("{what}: {e}");
            then(false);
            return frame.fail(CaptureFailureReason::Unknown);
        }
    };
    let damage = drawn.damage.clone();
    when_done(state, drawn, move |_, done| match done {
        Ok(()) => {
            then(true);
            frame.success(transform, damage, now);
        }
        Err(e) => {
            log::warn!("{what}: {e}");
            then(false);
            frame.fail(CaptureFailureReason::Unknown);
        }
    });
}

fn window_constraints(state: &State, w: WindowId) -> Option<BufferConstraints> {
    let size = window_size(state, w)?;
    Some(constraints(state, (size.w, size.h).into()))
}

/// A `size` buffer in shm, or a dmabuf on the render node once there is one.
fn constraints(state: &State, size: Size<i32, smithay::utils::Buffer>) -> BufferConstraints {
    BufferConstraints {
        size,
        shm: vec![wl_shm::Format::Xrgb8888, wl_shm::Format::Argb8888],
        dma: state.backend.as_ref().map(|b| DmabufConstraints {
            node: b.render_node,
            formats: dmabuf_formats(&b.renderer),
        }),
    }
}

/// Copy the pointer's image, its top-left at the buffer's, into the
/// cursor session's frame, and send its hotspot.
fn fill_cursor(state: &mut State, session: &CursorSessionRef, frame: Frame) {
    let Some(Shown::Output(output)) = shown(&session.source()) else {
        return frame.fail(CaptureFailureReason::Stopped);
    };
    let scale = output.current_scale().fractional_scale();
    let side = cursor_side(&output);
    let pointer = state.pointer_location();
    let State {
        backend,
        scene,
        cursor,
        saver,
        cursors,
        captures,
        ..
    } = state;
    let cursor = crate::saverview::cursor(saver, cursor);
    let Some(b) = backend.as_mut() else {
        return frame.fail(CaptureFailureReason::Unknown);
    };
    if let Err(e) = crate::gpufence::ready(&b.renderer) {
        log::warn!("cursor capture on {}: {e}", output.name());
        return frame.fail(CaptureFailureReason::Unknown);
    }
    let renderer = &mut b.renderer;
    let parts = Parts {
        scene,
        cursor,
        cursors,
        pointer,
    };
    let (elements, hotspot) = cursor_image(renderer, parts, Scale::from(scale));
    let at = ((side, side).into(), scale, Transform::Normal);
    let buffer = frame.buffer();
    let drawn = draw(
        renderer,
        &mut captures.readbacks,
        at,
        &elements,
        Color32F::TRANSPARENT,
        Target::of(&buffer),
    );
    let what = format!("cursor capture on {}", output.name());
    let session = session.clone();
    complete(state, drawn, frame, Transform::Normal, what, move |ready| {
        if ready {
            session.set_cursor_hotspot((hotspot.x, hotspot.y));
        }
    });
}

/// Whether `buffer` is `size`. A frame asked before its source resized keeps the old size,
/// which smithay lets through when it is larger.
fn fits(buffer: &WlBuffer, size: Size<i32, Physical>) -> bool {
    smithay::backend::renderer::buffer_dimensions(buffer)
        .is_none_or(|b| (b.w, b.h) == (size.w, size.h))
}

fn cursor_side(output: &Output) -> i32 {
    CURSOR_BOX * output.current_scale().fractional_scale().ceil() as i32
}

fn fill_ext(state: &mut State, output: &Output, session: &SessionRef, frame: Frame) {
    let Some(mode) = output.current_mode() else {
        return frame.fail(CaptureFailureReason::Stopped);
    };
    let buffer = frame.buffer();
    if !fits(&buffer, mode.size) {
        return frame.fail(CaptureFailureReason::BufferConstraints);
    }
    let drawn = render_into(
        state,
        output,
        (0, 0).into(),
        mode.size,
        session.draw_cursor(),
        Target::of(&buffer),
        Some(&mut session_damage(session).borrow_mut()),
    );
    if drawn.is_ok() {
        fresh(session).clear();
    }
    let what = format!("capture of {}", output.name());
    let session = session.clone();
    complete(
        state,
        drawn,
        frame,
        output.current_transform(),
        what,
        move |ready| {
            if !ready {
                session_damage(&session).borrow_mut().forget();
            }
        },
    );
}

fn fill_wlr(state: &mut State, screencopy: Screencopy) {
    if !screencopy.is_alive() {
        return;
    }
    let info = screencopy.info.clone();
    let buffer = match &screencopy.buffer {
        ScreencopyBuffer::Shm(b) => Target::Shm(b),
        ScreencopyBuffer::Dmabuf(d) => Target::Dmabuf(d.clone()),
    };
    let drawn = render_into(
        state,
        &info.output,
        info.region_loc,
        info.buffer_size,
        info.overlay_cursor,
        buffer,
        None,
    );
    let name = info.output.name();
    let now = state.now();
    // Dropping the screencopy fails its frame.
    match drawn {
        Ok(drawn) => when_done(state, drawn, move |_, done| match done {
            Ok(()) if screencopy.is_alive() => screencopy.submit(now),
            Ok(()) => {}
            Err(e) => log::warn!("screencopy of {name}: {e}"),
        }),
        Err(e) => log::warn!("screencopy of {name}: {e}"),
    }
}

impl State {
    /// Ask `w`'s client for a frame for a capture waiting on its commit;
    /// one minimized or off screen gets no frame callbacks otherwise.
    fn wake_window(&mut self, w: WindowId) {
        let Some(win) = self.win(w) else {
            return;
        };
        let r = win.frame_rect();
        let Some(output) = self.output_at((r.x + r.width / 2) as f64, (r.y + r.height / 2) as f64)
        else {
            return;
        };
        smithay::desktop::utils::send_frames_surface_tree(
            win.wl_surface(),
            &output,
            self.now(),
            Some(std::time::Duration::ZERO),
            |_, _| Some(output.clone()),
        );
    }
}

/// Whether a session's source changed since its last delivered frame. A
/// new session starts changed, so its first frame is filled at once.
struct Fresh(AtomicBool);

impl Default for Fresh {
    fn default() -> Fresh {
        Fresh(AtomicBool::new(true))
    }
}

impl Fresh {
    fn mark(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    fn clear(&self) {
        self.0.store(false, Ordering::Relaxed);
    }

    fn get(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// How many frames a session asked for, counting to two.
#[derive(Default)]
struct Asked(AtomicU8);

impl Asked {
    /// Count a frame; true for the second, which makes the session ongoing.
    fn frame(&self) -> bool {
        let n = self.0.load(Ordering::Relaxed);
        if n < 2 {
            self.0.store(n + 1, Ordering::Relaxed);
        }
        n == 1
    }

    fn ongoing(&self) -> bool {
        self.0.load(Ordering::Relaxed) >= 2
    }
}

fn asked(session: &SessionRef) -> &Asked {
    session
        .user_data()
        .insert_if_missing_threadsafe(Asked::default);
    session.user_data().get::<Asked>().expect("inserted above")
}

/// Whether `session`'s client marked itself the remote desktop server's capture connection.
fn for_remote_desktop(session: &SessionRef) -> bool {
    session.client().is_some_and(|c| {
        c.get_data::<crate::compositor::ClientState>()
            .is_some_and(|d| d.remote_capture.load(Ordering::Relaxed))
    })
}

fn wlr_key(screencopy: &Screencopy) -> WlrKey {
    let info = &screencopy.info;
    (info.output.clone(), info.region, screencopy.client())
}

fn fresh(session: &SessionRef) -> &Fresh {
    session
        .user_data()
        .insert_if_missing_threadsafe(Fresh::default);
    session.user_data().get::<Fresh>().expect("inserted above")
}

/// An output session's damage tracker, kept from one delivered frame to the next.
#[derive(Default)]
struct SessionDamage(Option<(View, Color32F, OutputDamageTracker)>);

impl SessionDamage {
    /// The buffer regions `elements` changed since the last call: the whole
    /// buffer on the first call and after `view` or `clear` changed.
    fn since_last<E: Element>(
        &mut self,
        view: View,
        clear: Color32F,
        elements: &[E],
    ) -> Vec<Rectangle<i32, Buffer>> {
        let (size, scale, transform) = view;
        if self
            .0
            .as_ref()
            .is_none_or(|(v, c, _)| *v != view || *c != clear)
        {
            let tracker = OutputDamageTracker::new(size, scale, transform);
            self.0 = Some((view, clear, tracker));
        }
        let (_, _, tracker) = self.0.as_mut().expect("set above");
        let Ok((damage, _)) = tracker.damage_output(1, elements) else {
            return vec![whole(size)];
        };
        // Output space to buffer pixels, as DrmCompositor maps element geometry.
        let to_buffer = transform.invert();
        let area = to_buffer.transform_size(size);
        damage
            .into_iter()
            .flatten()
            .map(|r| {
                let r = to_buffer.transform_rect_in(*r, &area);
                Rectangle::new((r.loc.x, r.loc.y).into(), (r.size.w, r.size.h).into())
            })
            .collect()
    }

    /// The frame last tracked never reached the client: the next is all damaged.
    fn forget(&mut self) {
        self.0 = None;
    }
}

fn whole(size: Size<i32, Physical>) -> Rectangle<i32, Buffer> {
    Rectangle::from_size((size.w, size.h).into())
}

fn session_damage(session: &SessionRef) -> &RefCell<SessionDamage> {
    session
        .user_data()
        .insert_if_missing(|| RefCell::new(SessionDamage::default()));
    session
        .user_data()
        .get::<RefCell<SessionDamage>>()
        .expect("inserted above")
}

impl ScreencopyHandler for State {
    fn screencopy_frame(&mut self, screencopy: Screencopy) {
        let key = wlr_key(&screencopy);
        if screencopy.with_damage && screencopy.info.output.current_mode().is_some() {
            let output = screencopy.info.output.clone();
            if first_copy(&screencopy) {
                fill_wlr(self, screencopy);
            } else {
                self.captures.waiting.push(Waiting::Wlr(screencopy));
                crate::backend::schedule(self, &output);
            }
        } else {
            fill_wlr(self, screencopy);
        }
        crate::captureborder::wlr_frame(self, key);
        crate::privacy::captures_changed(self);
    }
}

/// The first `copy_with_damage` of an output since start: nothing to
/// compare against, so it is filled at once.
fn first_copy(screencopy: &Screencopy) -> bool {
    let output = &screencopy.info.output;
    output
        .user_data()
        .insert_if_missing_threadsafe(|| AtomicBool::new(false));
    !output
        .user_data()
        .get::<AtomicBool>()
        .expect("inserted above")
        .swap(true, Ordering::Relaxed)
}

impl ImageCaptureSourceHandler for State {}

impl OutputCaptureSourceHandler for State {
    fn output_capture_source_state(&mut self) -> &mut OutputCaptureSourceState {
        &mut self.output_capture_source_state
    }

    fn output_source_created(&mut self, source: ImageCaptureSource, output: &Output) {
        source.user_data().insert_if_missing(|| output.downgrade());
    }
}

impl ToplevelCaptureSourceHandler for State {
    fn toplevel_capture_source_state(&mut self) -> &mut ToplevelCaptureSourceState {
        &mut self.toplevels.capture
    }

    /// A source over a listed window. One for a handle no longer listed
    /// stays untagged, so its sessions stop at once.
    fn toplevel_source_created(
        &mut self,
        source: ImageCaptureSource,
        toplevel: ForeignToplevelHandle,
    ) {
        if let Some(w) = self.toplevels.window_of(&toplevel) {
            source.user_data().insert_if_missing(|| w);
        }
    }
}

impl ImageCopyCaptureHandler for State {
    fn image_copy_capture_state(&mut self) -> &mut ImageCopyCaptureState {
        &mut self.image_copy_capture_state
    }

    fn capture_constraints(&mut self, source: &ImageCaptureSource) -> Option<BufferConstraints> {
        let output = match shown(source)? {
            Shown::Output(o) => o,
            Shown::Window(w) => return window_constraints(self, w),
        };
        let size = output.current_mode()?.size;
        Some(constraints(self, (size.w, size.h).into()))
    }

    fn new_session(&mut self, session: Session) {
        self.captures.sessions.push(session);
    }

    fn cursor_capture_constraints(
        &mut self,
        source: &ImageCaptureSource,
        _pointer: &WlPointer,
    ) -> Option<BufferConstraints> {
        // A window's capture carries no pointer.
        let Shown::Output(output) = shown(source)? else {
            return None;
        };
        let side = cursor_side(&output);
        Some(BufferConstraints {
            size: (side, side).into(),
            shm: vec![wl_shm::Format::Argb8888],
            dma: None,
        })
    }

    fn new_cursor_session(&mut self, session: CursorSession) {
        self.captures.cursor_sessions.push(session);
        pointer_moved(self);
    }

    fn cursor_frame(&mut self, session: &CursorSessionRef, frame: Frame) {
        fill_cursor(self, session, frame);
    }

    fn cursor_session_destroyed(&mut self, session: CursorSessionRef) {
        self.captures.cursor_sessions.retain(|s| *s != session);
    }

    fn frame(&mut self, session: &SessionRef, frame: Frame) {
        if asked(session).frame() {
            crate::privacy::captures_changed(self);
            crate::captureborder::refresh(self);
        }
        let shows = shown(&session.source());
        let changed = fresh(session).get();
        match shows {
            None => frame.fail(CaptureFailureReason::Stopped),
            Some(Shown::Output(o)) if changed => fill_ext(self, &o, session, frame),
            Some(Shown::Window(w)) if changed => fill_window(self, w, session, frame),
            Some(Shown::Output(o)) => {
                self.captures.waiting.push(Waiting::Ext {
                    session: session.clone(),
                    frame,
                });
                crate::backend::schedule(self, &o);
            }
            Some(Shown::Window(w)) => {
                self.captures.waiting.push(Waiting::Ext {
                    session: session.clone(),
                    frame,
                });
                self.wake_window(w);
            }
        }
    }

    fn frame_aborted(&mut self, frame: FrameRef) {
        self.captures
            .waiting
            .retain(|w| !matches!(w, Waiting::Ext { frame: f, .. } if *f == frame));
    }

    fn session_destroyed(&mut self, session: SessionRef) {
        self.captures.sessions.retain(|s| *s != session);
        crate::privacy::captures_changed(self);
        self.captures
            .waiting
            .retain(|w| !matches!(w, Waiting::Ext { session: s, .. } if *s == session));
        crate::captureborder::refresh(self);
    }
}

/// The renderer's dmabuf render formats grouped by fourcc.
fn dmabuf_formats(renderer: &GlesRenderer) -> Vec<(Fourcc, Vec<Modifier>)> {
    let mut out: Vec<(Fourcc, Vec<Modifier>)> = Vec::new();
    for f in renderer.egl_context().dmabuf_render_formats().iter() {
        match out.iter_mut().find(|(code, _)| *code == f.code) {
            Some((_, mods)) => mods.push(f.modifier),
            None => out.push((f.code, vec![f.modifier])),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use smithay::{
        backend::renderer::{
            Color32F,
            element::{Id, Kind, solid::SolidColorRenderElement},
        },
        utils::{Buffer, Physical, Point, Rectangle, Size, Transform},
    };

    use super::{Fresh, SessionDamage, View};
    use crate::harness::Harness;
    use crate::render::{Parts, output_elements};

    fn hd() -> View {
        (Size::new(1280, 720), 1.0, Transform::Normal)
    }
    const CLEAR: Color32F = Color32F::new(0.0, 0.0, 0.0, 1.0);

    fn whole(w: i32, h: i32) -> Vec<Rectangle<i32, Buffer>> {
        vec![Rectangle::new((0, 0).into(), (w, h).into())]
    }

    fn solid(id: &Id, (x, y, w, h): (i32, i32, i32, i32)) -> SolidColorRenderElement {
        let geo = Rectangle::<i32, Physical>::new((x, y).into(), (w, h).into());
        SolidColorRenderElement::new(id.clone(), geo, 0, [1.0, 0.0, 0.0, 1.0], Kind::Unspecified)
    }

    /// Whether `damage` covers every pixel of `r`.
    fn covers(damage: &[Rectangle<i32, Buffer>], r: Rectangle<i32, Buffer>) -> bool {
        Rectangle::subtract_rects_many([r], damage.iter().copied()).is_empty()
    }

    fn buffer_rect(r: crate::raster::Rect) -> Rectangle<i32, Buffer> {
        Rectangle::new((r.x, r.y).into(), (r.width, r.height).into())
    }

    #[test]
    fn a_first_output_capture_frame_is_fully_damaged() {
        let id = Id::new();
        let mut d = SessionDamage::default();
        let damage = d.since_last(hd(), CLEAR, &[solid(&id, (10, 10, 20, 20))]);
        assert_eq!(damage, whole(1280, 720), "the whole 1280x720 buffer");
    }

    #[test]
    fn moving_a_window_damages_only_its_old_and_new_place() {
        let Ok(mut renderer) = reprisefx::headless::renderer() else {
            eprintln!("skipped: no renderer");
            return;
        };
        let mut h = Harness::new();
        let output = h.output("TEST-1", 1280, 720);
        let mut c = h.connect();
        let t = h.map_toplevel(&mut c, "test.app", 200, 100);
        h.state.move_to(t.id, 100, 100);
        h.dispatch();
        let mut elements = |h: &mut Harness| {
            let s = &mut h.state;
            let parts = Parts {
                scene: &s.scene,
                cursor: &s.cursor,
                cursors: &mut s.cursors,
                pointer: Point::default(),
            };
            output_elements(&mut renderer, parts, &output, Point::default(), false)
        };
        let mut d = SessionDamage::default();
        let before = h.state.win(t.id).unwrap().frame_rect();
        d.since_last(hd(), CLEAR, &elements(&mut h));
        h.state.move_to(t.id, 400, 300);
        h.dispatch();
        let after = h.state.win(t.id).unwrap().frame_rect();
        let damage = d.since_last(hd(), CLEAR, &elements(&mut h));
        assert!(
            covers(&damage, buffer_rect(before)),
            "the old place {before:?}: {damage:?}"
        );
        assert!(
            covers(&damage, buffer_rect(after)),
            "the new place {after:?}: {damage:?}"
        );
        let corner = Rectangle::new((0, 0).into(), (50, 50).into());
        assert!(
            !damage.iter().any(|r| r.overlaps(corner)),
            "(0,0)-(50,50) is untouched: {damage:?}"
        );
    }

    #[test]
    fn an_unchanged_output_reports_no_damage_when_forced() {
        let id = Id::new();
        let mut d = SessionDamage::default();
        d.since_last(hd(), CLEAR, &[solid(&id, (10, 10, 20, 20))]);
        let damage = d.since_last(hd(), CLEAR, &[solid(&id, (10, 10, 20, 20))]);
        assert_eq!(damage, Vec::new(), "nothing changed since the last frame");
    }

    #[test]
    fn a_mode_change_reports_full_damage() {
        let id = Id::new();
        let mut d = SessionDamage::default();
        d.since_last(hd(), CLEAR, &[solid(&id, (10, 10, 20, 20))]);
        let fhd = (Size::new(1920, 1080), 1.0, Transform::Normal);
        let damage = d.since_last(fhd, CLEAR, &[solid(&id, (10, 10, 20, 20))]);
        assert_eq!(damage, whole(1920, 1080), "the whole 1920x1080 buffer");
        let scaled = (Size::new(1920, 1080), 2.0, Transform::Normal);
        let damage = d.since_last(scaled, CLEAR, &[solid(&id, (10, 10, 20, 20))]);
        assert_eq!(damage, whole(1920, 1080), "a new scale damages it all");
        let rotated = (Size::new(1920, 1080), 2.0, Transform::_90);
        let damage = d.since_last(rotated, CLEAR, &[solid(&id, (10, 10, 20, 20))]);
        assert_eq!(
            damage,
            whole(1920, 1080),
            "a new transform damages it all, in buffer pixels"
        );
    }

    #[test]
    fn a_frame_that_never_reached_the_client_leaves_the_next_fully_damaged() {
        let id = Id::new();
        let mut d = SessionDamage::default();
        d.since_last(hd(), CLEAR, &[solid(&id, (10, 10, 20, 20))]);
        d.forget();
        let damage = d.since_last(hd(), CLEAR, &[solid(&id, (10, 10, 20, 20))]);
        assert_eq!(damage, whole(1280, 720), "the whole 1280x720 buffer");
    }

    #[test]
    fn a_window_moved_onto_a_scale_2_output_needs_a_buffer_twice_its_size_without_a_commit() {
        let mut h = Harness::new();
        h.output("TEST-1", 1024, 768);
        let b = h.output("TEST-2", 2048, 1536);
        b.change_current_state(
            None,
            None,
            Some(smithay::output::Scale::Fractional(2.0)),
            None,
        );
        h.state.update_layout();
        let mut c = h.connect();
        let t = h.map_toplevel(&mut c, "test.app", 200, 100);
        h.state.move_to(t.id, 100, 100);
        let on_1 = super::window_size(&h.state, t.id).expect("mapped");
        let b_at = reprisescene::output_rect(&b).expect("TEST-2's place").loc;
        h.state.move_to(t.id, b_at.x + 100, b_at.y + 100);
        let on_2 = super::window_size(&h.state, t.id).expect("mapped");
        assert_eq!(
            (on_2.w, on_2.h),
            (2 * on_1.w, 2 * on_1.h),
            "{on_1:?} at scale 1 doubles at scale 2"
        );
    }

    #[test]
    fn a_change_between_frames_is_kept_for_the_next_request() {
        let f = Fresh::default();
        assert!(f.get(), "a new session's first frame is filled at once");
        f.clear();
        assert!(!f.get(), "a delivered frame leaves the next one waiting");
        f.mark();
        assert!(
            f.get(),
            "a commit with no frame waiting fills the next at once"
        );
        f.mark();
        f.clear();
        assert!(!f.get());
    }
}
