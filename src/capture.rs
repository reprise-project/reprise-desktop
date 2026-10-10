//! Output and window capture for ext-image-copy-capture-v1 and
//! wlr-screencopy-v1: an output's elements, or a window's client surface
//! tree, rendered into the client's shm or dmabuf buffer. A session's
//! first frame is filled at once; later ones, and every
//! `copy_with_damage`, wait for the output's next drawn frame or the
//! window's next commit, so an idle screen costs a screencast nothing.
//! A pointer cursor session on an output copies the pointer's image and
//! follows it over the output.
//!
//! The shm copy follows niri's `render_to_shm` in
//! `src/render_helpers/mod.rs` (GPL-3.0-or-later).

use std::sync::atomic::{AtomicBool, Ordering};

use smithay::{
    backend::{
        allocator::{Fourcc, Modifier, dmabuf::Dmabuf},
        renderer::{
            Bind, Color32F, ExportMem, Offscreen,
            damage::OutputDamageTracker,
            element::{Kind, surface::render_elements_from_surface_tree},
            gles::{GlesRenderer, GlesTexture},
        },
    },
    output::{Output, WeakOutput},
    reexports::wayland_server::protocol::{wl_buffer::WlBuffer, wl_pointer::WlPointer, wl_shm},
    utils::{Physical, Point, Rectangle, Scale, Size, Transform},
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
    compositor::State,
    render::{OutputElement, Parts, clear_color, cursor_image, output_elements},
    screencopy::{Screencopy, ScreencopyBuffer, ScreencopyHandler},
    window::WindowId,
};

/// The side of a cursor session's buffer, in logical pixels: room for
/// the largest pointer image the themes ship.
const CURSOR_BOX: i32 = 64;

/// The open ext sessions and the frames waiting for an output's next frame.
#[derive(Default)]
pub struct Captures {
    sessions: Vec<Session>,
    cursor_sessions: Vec<CursorSession>,
    waiting: Vec<Waiting>,
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

    pub fn live(&self) -> (usize, usize) {
        let windows = self
            .sessions
            .iter()
            .filter(|s| matches!(shown(&s.source()), Some(Shown::Window(_))))
            .count();
        (self.sessions.len() - windows, windows)
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
    let constraints = window_constraints(state, w);
    for s in &state.captures.sessions {
        if shown(&s.source()) == on
            && let Some(c) = &constraints
            && s.current_constraints().is_none_or(|old| old.size != c.size)
        {
            s.update_constraints(c.clone());
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
    crate::privacy::sessions_changed(state);
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
/// top-left), `size` large, into `target`. The debug overlay shows only
/// with `[debug] overlay_in_captures`.
fn render_into(
    state: &mut State,
    output: &Output,
    origin: Point<i32, Physical>,
    size: Size<i32, Physical>,
    with_cursor: bool,
    target: Target<'_>,
) -> Result<(), String> {
    let shown = state.config.debug.overlay_in_captures;
    state.without_debug_panels(shown, |state| {
        render_scene_into(state, output, origin, size, with_cursor, target)
    })
}

fn render_scene_into(
    state: &mut State,
    output: &Output,
    origin: Point<i32, Physical>,
    size: Size<i32, Physical>,
    with_cursor: bool,
    target: Target<'_>,
) -> Result<(), String> {
    let pointer = state.pointer_location();
    let State {
        backend,
        scene,
        cursor,
        cursors,
        config,
        glass,
        interaction,
        ..
    } = state;
    let b = backend.as_mut().ok_or("no backend")?;
    // A capture shows the backdrop as it is now: nothing to hold.
    glass.begin(
        output,
        config.glass,
        crate::glass::quality(interaction, &config.glass),
        false,
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
    draw(renderer, at, &elements, clear, target)
}

fn draw(
    renderer: &mut GlesRenderer,
    (size, scale, transform): (Size<i32, Physical>, f64, Transform),
    elements: &[OutputElement],
    clear: Color32F,
    target: Target<'_>,
) -> Result<(), String> {
    let mut tracker = OutputDamageTracker::new(size, scale, transform);
    match target {
        Target::Dmabuf(mut dmabuf) => {
            let mut fb = renderer.bind(&mut dmabuf).map_err(|e| e.to_string())?;
            let result = tracker
                .render_output(renderer, &mut fb, 0, elements, clear)
                .map_err(|e| format!("{e:?}"))?;
            // The client reads the buffer once told it is ready.
            result.sync.wait().map_err(|e| format!("{e:?}"))
        }
        Target::Shm(buffer) => shm::with_buffer_contents_mut(buffer, |pool, pool_len, data| {
            let fourcc = match data.format {
                wl_shm::Format::Xrgb8888 => Fourcc::Xrgb8888,
                wl_shm::Format::Argb8888 => Fourcc::Argb8888,
                f => return Err(format!("shm format {f:?} not offered")),
            };
            if data.width != size.w || data.height != size.h {
                return Err("shm buffer of the wrong size".into());
            }
            let row = size.w as usize * 4;
            let rows = size.h as usize;
            let offset = usize::try_from(data.offset).map_err(|_| "negative offset")?;
            let stride = usize::try_from(data.stride).map_err(|_| "negative stride")?;
            let end = stride
                .checked_mul(rows.saturating_sub(1))
                .and_then(|n| n.checked_add(row))
                .and_then(|n| n.checked_add(offset));
            if stride < row || end.is_none_or(|end| end > pool_len) {
                return Err("buffer does not fit its pool".into());
            }
            let mut texture: GlesTexture = renderer
                .create_buffer(fourcc, size.to_logical(1).to_buffer(1, Transform::Normal))
                .map_err(|e| e.to_string())?;
            let mut fb = renderer.bind(&mut texture).map_err(|e| e.to_string())?;
            tracker
                .render_output(renderer, &mut fb, 0, elements, clear)
                .map_err(|e| format!("{e:?}"))?;
            let mapping = renderer
                .copy_framebuffer(&fb, Rectangle::from_size((size.w, size.h).into()), fourcc)
                .map_err(|e| e.to_string())?;
            let bytes = renderer.map_texture(&mapping).map_err(|e| e.to_string())?;
            if bytes.len() < row * rows {
                return Err("short texture read".into());
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
        .map_err(|e| format!("{e:?}"))?,
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
fn render_window(state: &mut State, w: WindowId, target: Target<'_>) -> Result<(), String> {
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
    let renderer = &mut b.renderer;
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
    draw(renderer, at, &elements, Color32F::TRANSPARENT, target)
}

fn fill_window(state: &mut State, w: WindowId, session: &SessionRef, frame: Frame) {
    let buffer = frame.buffer();
    match render_window(state, w, Target::of(&buffer)) {
        Ok(()) => {
            delivered(session).store(true, Ordering::Relaxed);
            frame.success(Transform::Normal, None, state.now());
        }
        Err(e) => {
            log::warn!("capture of window {w:?}: {e}");
            frame.fail(CaptureFailureReason::Unknown);
        }
    }
}

fn window_constraints(state: &State, w: WindowId) -> Option<BufferConstraints> {
    let size = window_size(state, w)?;
    let b = state.backend.as_ref()?;
    Some(BufferConstraints {
        size: (size.w, size.h).into(),
        shm: vec![wl_shm::Format::Xrgb8888, wl_shm::Format::Argb8888],
        dma: Some(DmabufConstraints {
            node: b.render_node,
            formats: dmabuf_formats(&b.renderer),
        }),
    })
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
        cursors,
        ..
    } = state;
    let Some(b) = backend.as_mut() else {
        return frame.fail(CaptureFailureReason::Unknown);
    };
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
    match draw(
        renderer,
        at,
        &elements,
        Color32F::TRANSPARENT,
        Target::of(&buffer),
    ) {
        Ok(()) => {
            session.set_cursor_hotspot((hotspot.x, hotspot.y));
            frame.success(Transform::Normal, None, state.now());
        }
        Err(e) => {
            log::warn!("cursor capture on {}: {e}", output.name());
            frame.fail(CaptureFailureReason::Unknown);
        }
    }
}

fn cursor_side(output: &Output) -> i32 {
    CURSOR_BOX * output.current_scale().fractional_scale().ceil() as i32
}

fn fill_ext(state: &mut State, output: &Output, session: &SessionRef, frame: Frame) {
    let Some(mode) = output.current_mode() else {
        return frame.fail(CaptureFailureReason::Stopped);
    };
    let buffer = frame.buffer();
    match render_into(
        state,
        output,
        (0, 0).into(),
        mode.size,
        session.draw_cursor(),
        Target::of(&buffer),
    ) {
        Ok(()) => {
            delivered(session).store(true, Ordering::Relaxed);
            frame.success(output.current_transform(), None, state.now());
        }
        Err(e) => {
            log::warn!("capture of {}: {e}", output.name());
            frame.fail(CaptureFailureReason::Unknown);
        }
    }
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
    match render_into(
        state,
        &info.output,
        info.region_loc,
        info.buffer_size,
        info.overlay_cursor,
        buffer,
    ) {
        Ok(()) => screencopy.submit(state.now()),
        // Dropping it fails the frame.
        Err(e) => log::warn!("screencopy of {}: {e}", info.output.name()),
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

fn delivered(session: &SessionRef) -> &AtomicBool {
    session
        .user_data()
        .insert_if_missing_threadsafe(|| AtomicBool::new(false));
    session
        .user_data()
        .get::<AtomicBool>()
        .expect("inserted above")
}

impl ScreencopyHandler for State {
    fn screencopy_frame(&mut self, screencopy: Screencopy) {
        crate::privacy::legacy_frame(self);
        if screencopy.with_damage && screencopy.info.output.current_mode().is_some() {
            let output = screencopy.info.output.clone();
            if first_copy(&screencopy) {
                return fill_wlr(self, screencopy);
            }
            self.captures.waiting.push(Waiting::Wlr(screencopy));
            crate::backend::schedule(self, &output);
        } else {
            fill_wlr(self, screencopy);
        }
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
        let b = self.backend.as_ref()?;
        Some(BufferConstraints {
            size: (size.w, size.h).into(),
            shm: vec![wl_shm::Format::Xrgb8888, wl_shm::Format::Argb8888],
            dma: Some(DmabufConstraints {
                node: b.render_node,
                formats: dmabuf_formats(&b.renderer),
            }),
        })
    }

    fn new_session(&mut self, session: Session) {
        self.captures.sessions.push(session);
        crate::privacy::sessions_changed(self);
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
        let shows = shown(&session.source());
        let first = !delivered(session).load(Ordering::Relaxed);
        match shows {
            None => frame.fail(CaptureFailureReason::Stopped),
            Some(Shown::Output(o)) if first => fill_ext(self, &o, session, frame),
            Some(Shown::Window(w)) if first => fill_window(self, w, session, frame),
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
        crate::privacy::sessions_changed(self);
        self.captures
            .waiting
            .retain(|w| !matches!(w, Waiting::Ext { session: s, .. } if *s == session));
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
