//! Output and window capture for ext-image-copy-capture-v1 and
//! wlr-screencopy-v1: an output's elements, or a window's client surface
//! tree, rendered into the client's shm or dmabuf buffer. A session's frame is filled at once when
//! its source changed since the session's last delivered frame (and for the first); otherwise it
//! waits for the output's next drawn frame or the window's next commit that shows something new,
//! and a `copy_with_damage` waits likewise while its buffer holds the latest picture, so an idle
//! screen or window costs a screencast nothing. An output frame that would put nothing new in its
//! buffer waits on, undrawn, and a drawn frame only the pointer asked for ([`Drew::Pointer`])
//! reaches only captures that draw the pointer.
//!
//! An output stream (an ext session, or a wlr client's copies of one view) keeps a damage tracker
//! and, for each buffer the client cycles, what changed since that buffer last held a frame of it.
//! An ext frame reports what changed since the session's previous frame and draws, into its
//! buffer, that buffer's own damage plus what the client says it damaged (`damage_buffer`). A wlr
//! frame is drawn whole, as its client may touch the buffer between copies and has no request to
//! say so, and reports its buffer's own damage, so two consumers sharing a manager never take each
//! other's. A buffer not seen, a new size, scale, transform or clear colour, a frame that never
//! reached the client and a frame of another stream, a window or the pointer into the buffer make
//! it whole; so does a wlr copy into a buffer whose earlier copy is still in flight, as that one may
//! yet fail. Window and cursor frames draw and report all.
//! A pointer cursor session on an output copies the pointer's image and
//! follows it over the output. A session is ongoing (a screencast, not a
//! screenshot) from its second frame.
//!
//! The shm copy follows niri's `render_to_shm` in
//! `src/render_helpers/mod.rs` (GPL-3.0-or-later).

use std::cell::RefCell;
use std::collections::HashSet;
use std::ops::Range;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::Instant;

use smithay::{
    backend::{
        allocator::{Fourcc, Modifier, dmabuf::Dmabuf},
        renderer::{
            Bind, Color32F, Frame as _, Renderer,
            damage::OutputDamageTracker,
            element::{
                Element, Id, Kind, RenderElement, surface::render_elements_from_surface_tree,
            },
            gles::{GlesRenderer, GlesTarget},
            sync::SyncPoint,
        },
    },
    output::{Output, WeakOutput},
    reexports::{
        calloop::timer::{TimeoutAction, Timer},
        wayland_protocols_wlr::screencopy::v1::server::{
            zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1,
            zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
        },
        wayland_server::{
            Resource,
            protocol::{wl_buffer::WlBuffer, wl_pointer::WlPointer, wl_shm},
        },
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
    screencopy::{FrameInfo, Screencopy, ScreencopyBuffer, ScreencopyHandler},
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
    /// Ext frames whose render is in flight; one destroyed meanwhile never reaches its client.
    drawing: Vec<(FrameRef, SessionRef)>,
    readbacks: Readbacks,
    /// Windows with a [`State::wake_window_soon`] timer armed.
    waking: Vec<WindowId>,
    /// The wlr views copied lately, the least recent first.
    wlr_streams: Vec<(WlrView, Rc<RefCell<SessionDamage>>)>,
    /// The glass caches of the ext sessions copied lately, the least recent first.
    effects: Vec<(SessionRef, Rc<RefCell<crate::glass::Effects>>)>,
    /// Draws a test's captures, as a test has no backend.
    #[cfg(test)]
    pub renderer: Option<GlesRenderer>,
}

/// The renderer captures draw with, and the pool their shm frames are read back through.
fn capture_renderer<'a>(
    backend: &'a mut Option<crate::backend::Backend>,
    captures: &'a mut Captures,
) -> Result<(&'a mut GlesRenderer, &'a mut Readbacks), String> {
    #[cfg(test)]
    if backend.is_none()
        && let Some(r) = captures.renderer.as_mut()
    {
        return Ok((r, &mut captures.readbacks));
    }
    let b = backend.as_mut().ok_or("no backend")?;
    Ok((&mut b.renderer, &mut captures.readbacks))
}

/// The most ext sessions whose glass caches (GPU textures) live from one frame to the next; another
/// session's frame fills a fresh one.
const CACHED_SESSIONS: usize = 4;

/// The most wlr views whose damage is kept.
const WLR_VIEWS: usize = 8;

/// What a wlr client's copies show: through one manager, one region of an output, with or without
/// the pointer.
#[derive(PartialEq)]
struct WlrView {
    manager: ZwlrScreencopyManagerV1,
    output: Output,
    at: Point<i32, Physical>,
    size: Size<i32, Physical>,
    cursor: bool,
}

impl WlrView {
    fn of(info: &FrameInfo) -> WlrView {
        WlrView {
            manager: info.manager.clone(),
            output: info.output.clone(),
            at: info.region_loc,
            size: info.buffer_size,
            cursor: info.overlay_cursor,
        }
    }
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

    fn sees(&self, drew: Drew) -> bool {
        match self {
            Waiting::Ext { session, .. } => drew.seen(session.draw_cursor()),
            Waiting::Wlr(s) => drew.seen(s.info.overlay_cursor),
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
        self.wlr_streams.retain(|(v, _)| v.output != *output);
    }

    /// `buffer` now holds a frame of the output stream `holder`, or of a capture of no output
    /// stream: the other streams forget it.
    fn held(&self, buffer: &WlBuffer, holder: Option<&RefCell<SessionDamage>>) {
        let ext =
            (self.sessions.iter()).filter_map(|s| s.user_data().get::<RefCell<SessionDamage>>());
        let wlr = self.wlr_streams.iter().map(|(_, d)| &**d);
        held_by(ext.chain(wlr), holder, buffer);
    }

    /// The client destroyed `buffer`: no stream keeps it, nor the client's memory it maps.
    pub fn buffer_gone(&self, buffer: &WlBuffer) {
        let ext =
            (self.sessions.iter()).filter_map(|s| s.user_data().get::<RefCell<SessionDamage>>());
        let wlr = self.wlr_streams.iter().map(|(_, d)| &**d);
        for s in ext.chain(wlr) {
            s.borrow_mut().gone(buffer);
        }
    }

    /// Stop every session whose source `stops` holds for, failing its waiting frames and letting go
    /// of its buffers and glass caches.
    fn stop(&mut self, stops: impl Fn(&Option<Shown>) -> bool) {
        self.waiting.retain(|w| !stops(&w.shows()));
        let (stopped, kept): (Vec<Session>, Vec<Session>) = std::mem::take(&mut self.sessions)
            .into_iter()
            .partition(|s| stops(&shown(&s.source())));
        self.sessions = kept;
        // smithay keeps a stopped session, with what its user data holds, until its client destroys it.
        for s in &stopped {
            if let Some(d) = s.user_data().get::<RefCell<SessionDamage>>() {
                d.borrow_mut().forget();
            }
        }
        self.effects
            .retain(|(s, _)| !stopped.iter().any(|x| x == s));
        self.cursor_sessions.retain(|s| !stops(&shown(&s.source())));
    }

    /// The source `on` drew or committed what `drew` says: the next frames of its sessions that
    /// show it are filled at once.
    fn changed(&self, on: &Option<Shown>, drew: Drew) {
        for s in self.sessions.iter().filter(|s| shown(&s.source()) == *on) {
            if drew.seen(s.draw_cursor()) {
                fresh(s).mark();
            }
        }
    }

    /// Whether a session or a waiting frame shows `output`.
    pub fn shows_output(&self, output: &Output) -> bool {
        let on = Some(Shown::Output(output.clone()));
        self.sessions.iter().any(|s| shown(&s.source()) == on)
            || self.waiting.iter().any(|w| w.shows() == on)
    }

    /// What the ongoing ext sessions show, those for the remote desktop server only `with_remote`,
    /// and those of the screen recorder's output `recorded` (an area of which it records) none.
    pub fn targets(
        &self,
        with_remote: bool,
        recorded: Option<&Output>,
    ) -> Vec<Captured<Output, WindowId>> {
        self.sessions
            .iter()
            .filter(|s| asked(s).ongoing())
            .filter(|s| with_remote || !for_remote_desktop(s))
            .filter_map(|s| match shown(&s.source())? {
                Shown::Output(o) if for_recorder(s) && recorded == Some(&o) => None,
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

/// `output` runs a new mode or transform: its sessions get the new buffer size.
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

/// What a drawn frame of an output may show new, as what asked for it says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Drew {
    /// Only the pointer moved or changed.
    Pointer,
    /// Anything may have changed.
    Scene,
}

impl Drew {
    /// Whether a capture, drawing the pointer `with_cursor`, may see the change.
    fn seen(self, with_cursor: bool) -> bool {
        self == Drew::Scene || with_cursor
    }
}

/// How a monitor's frame ended, with what its asks changed: on the screen, drawn with nothing new
/// for the screen, or lost on its way (a render error, a send or flip that failed).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ended {
    Shown(Drew),
    Empty(Drew),
    Lost(Drew),
}

/// Fills `output`'s waiting captures unless the frame was lost; returns whether they are owed the
/// scene after. An empty frame fills them too: a zoomed screen shows less than they do.
pub fn frame_ended(state: &mut State, output: &Output, owed: bool, ended: Ended) -> bool {
    let (drew, told) = match ended {
        Ended::Shown(drew) | Ended::Empty(drew) => (drew, true),
        Ended::Lost(drew) => (drew, false),
    };
    if told {
        output_drawn(state, output, drew);
    }
    match drew {
        Drew::Scene => !told,
        Drew::Pointer => owed,
    }
}

pub fn output_drawn(state: &mut State, output: &Output, drew: Drew) {
    let on = Some(Shown::Output(output.clone()));
    state.captures.changed(&on, drew);
    for w in take_waiting(state, |w| w.shows() == on && w.sees(drew)) {
        match w {
            Waiting::Ext { session, frame } => fill_ext(state, output, &session, frame),
            Waiting::Wlr(s) => {
                if let Some(waits) = fill_wlr(state, s) {
                    state.captures.waiting.push(Waiting::Wlr(waits));
                }
            }
        }
    }
}

/// A commit in `w`'s surface tree applied; it `drew` unless it showed nothing new. A drawn commit
/// makes its sessions' next frames due. Once the window's whole state is `applied` (any commit but
/// a synchronized subsurface's), a new size goes to its sessions as new constraints and its due
/// frames are filled; the others keep waiting, and the client is asked for its next frame.
pub fn window_committed(state: &mut State, w: WindowId, drew: bool, applied: bool) {
    if state.captures.sessions.is_empty() {
        return;
    }
    let on = Some(Shown::Window(w));
    if drew {
        state.captures.changed(&on, Drew::Scene);
    }
    if !applied {
        return;
    }
    if let Some(c) = window_constraints(state, w) {
        for s in &state.captures.sessions {
            if shown(&s.source()) == on {
                advertise(s, &c);
            }
        }
    }
    let (due, unchanged): (Vec<Waiting>, Vec<Waiting>) = take_waiting(state, |x| x.shows() == on)
        .into_iter()
        .partition(|w| matches!(w, Waiting::Ext { session, .. } if fresh(session).get()));
    if !unchanged.is_empty() {
        state.captures.waiting.extend(unchanged);
        state.wake_window_soon(w);
    }
    for waiting in due {
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

/// A capture frame waits for `output`'s next change. A monitor is asked for a frame, which it draws
/// only if something changed; a virtual screen, whose every tick counts as drawn, for none.
fn wait_on(state: &mut State, output: &Output) {
    match crate::backend::is_virtual(output) {
        true => crate::virtualout::capture_waits(state, output),
        false => crate::backend::schedule(state, output),
    }
}

fn take_waiting(state: &mut State, taken: impl Fn(&Waiting) -> bool) -> Vec<Waiting> {
    let (now, rest): (Vec<Waiting>, Vec<Waiting>) = std::mem::take(&mut state.captures.waiting)
        .into_iter()
        .partition(taken);
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

/// A frame of an output stream: the stream's damage, the client's buffer and how it asked.
struct Stream<'a> {
    damage: &'a RefCell<SessionDamage>,
    buffer: &'a WlBuffer,
    ask: Ask,
}

enum Ask {
    /// An ext frame, with what the client says it damaged in its buffer and its session's glass
    /// caches.
    Ext {
        asked: Vec<Rectangle<i32, Buffer>>,
        effects: Rc<RefCell<crate::glass::Effects>>,
    },
    /// A wlr frame; one that `waits` (`copy_with_damage`) is not drawn while its buffer holds the
    /// latest picture.
    Wlr { waits: bool },
}

/// Render `output`'s picture from `origin` (physical pixels from its
/// top-left), into a `size` buffer turned by `transform` from the layout,
/// into `target`, as much of it as `stream` needs. None when the frame waits: its buffer holds the
/// latest picture, and an ext client damaged nothing in it. The debug overlay shows only with
/// `[debug] overlay_in_captures`.
fn render_into(
    state: &mut State,
    output: &Output,
    origin: Point<i32, Physical>,
    buffer: (Size<i32, Physical>, Transform),
    with_cursor: bool,
    target: Target<'_>,
    stream: Stream<'_>,
) -> Result<Option<Drawn>, String> {
    let shown = state.config.debug.overlay_in_captures;
    state.without_debug_panels(shown, |state| {
        render_scene_into(state, output, origin, buffer, with_cursor, target, stream)
    })
}

fn render_scene_into(
    state: &mut State,
    output: &Output,
    origin: Point<i32, Physical>,
    (size, transform): (Size<i32, Physical>, Transform),
    with_cursor: bool,
    target: Target<'_>,
    stream: Stream<'_>,
) -> Result<Option<Drawn>, String> {
    let pointer = state.pointer_location();
    let fitted = state.fitted_source(output);
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
        copy_frames,
        ..
    } = state;
    let cursor = crate::saverview::cursor(saver, cursor);
    let (renderer, readbacks) = capture_renderer(backend, captures)?;
    crate::gpufence::ready(renderer)?;
    // A capture shows the blur its screen shows: one the screen held for `[glass] blur_rate` is
    // kept, the rest drawn as it is now.
    let drawn_from = fitted.as_ref().unwrap_or(output);
    glass.begin(
        drawn_from,
        config.glass,
        crate::glass::quality(interaction, &config.glass),
        glass.capture_hold(drawn_from),
    );
    let clear = clear_color(&config.theme.colors);
    let wanted = crate::glass::wanted(config);
    glass.update_screen_backdrop(renderer, scene, drawn_from, clear, wanted);
    let parts = Parts {
        scene,
        cursor,
        cursors,
        pointer,
    };
    let shown = (output, fitted.as_ref());
    let (elements, clear) = scene_elements(
        renderer,
        copy_frames,
        parts,
        shown,
        origin,
        with_cursor,
        clear,
    );
    let scale = output.current_scale().fractional_scale();
    let at = (size, scale, transform);
    let mut damage = stream.damage.borrow_mut();
    let changed = damage.changed(at, clear, &elements);
    let (drawn, reported) = match stream.ask {
        Ask::Ext { asked, effects } => {
            let mut region = damage.take(at, stream.buffer, &asked);
            if region.is_empty() {
                return Ok(None);
            }
            grow_over_effects(&mut region, &elements, scale);
            let mut effects = effects.borrow_mut();
            let ids: HashSet<&Id> = elements.iter().map(Element::id).collect();
            effects.retain(|id, _| ids.contains(id));
            let drawn = draw_region(
                renderer,
                readbacks,
                at,
                &elements,
                clear,
                target,
                (&region, &mut effects),
            );
            (drawn, changed)
        }
        Ask::Wlr { waits } => {
            let Some(since) = damage.wlr_frame(at, stream.buffer, waits) else {
                return Ok(None);
            };
            (
                draw(renderer, readbacks, at, &elements, clear, target),
                since,
            )
        }
    };
    let mut drawn = drawn?;
    drawn.damage = reported.into_iter().map(|r| to_buffer(at, r)).collect();
    Ok(Some(drawn))
}

/// `region` grown to cover whole each framebuffer effect among `elements` it reaches, until none
/// reaches past it: such an effect reads everything beneath it.
fn grow_over_effects<E: Element>(
    region: &mut Vec<Rectangle<i32, Physical>>,
    elements: &[E],
    scale: f64,
) {
    loop {
        let mut grew = false;
        for e in elements.iter().filter(|e| e.is_framebuffer_effect()) {
            let geometry = e.geometry(Scale::from(scale));
            let reached = region.iter().any(|r| r.overlaps(geometry));
            let whole =
                Rectangle::subtract_rects_many([geometry], region.iter().copied()).is_empty();
            if reached && !whole {
                region.push(geometry);
                grew = true;
            }
        }
        if !grew {
            return;
        }
    }
}

/// What a capture of `output` draws from `origin`, and the colour around it: a copy of another
/// size (`fitted`, the output it copies) shows that frame fitted on black, as its screen does.
pub(crate) fn scene_elements(
    renderer: &mut GlesRenderer,
    frames: &mut crate::mirror::CopyFrames,
    parts: Parts<'_>,
    (output, fitted): (&Output, Option<&Output>),
    origin: Point<i32, Physical>,
    with_cursor: bool,
    clear: Color32F,
) -> (Vec<OutputElement>, Color32F) {
    match fitted {
        Some(source) => {
            let shown = (source, output);
            let elements =
                crate::mirror::elements(renderer, frames, parts, shown, origin, with_cursor);
            (elements, Color32F::BLACK)
        }
        None => {
            let elements = output_elements(renderer, parts, output, origin, with_cursor);
            (elements, clear)
        }
    }
}

/// A capture rendered into its buffer, waiting for the GPU to finish.
/// An shm buffer is filled once it has.
struct Drawn {
    sync: SyncPoint,
    shm: Option<ShmCopy>,
    /// What the frame reports changed, in buffer pixels.
    damage: Vec<Rectangle<i32, Buffer>>,
}

struct ShmCopy {
    buffer: WlBuffer,
    slot: Slot,
    size: Size<i32, Physical>,
    /// The parts of the picture the client's buffer takes; the rest it holds already.
    copied: Vec<Rectangle<i32, Buffer>>,
}

/// The rows of a `size` picture that `rects` reach.
fn rows_of(rects: &[Rectangle<i32, Buffer>], size: Size<i32, Physical>) -> Range<i32> {
    let area = Rectangle::from_size((size.w, size.h).into());
    let reached = rects.iter().filter_map(|r| r.intersection(area));
    let (top, bottom) = reached.fold((size.h, 0), |(t, b), r| {
        (t.min(r.loc.y), b.max(r.loc.y + r.size.h))
    });
    top..bottom.max(top)
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
            let fourcc = shm_fourcc(buffer, size)?;
            let mut slot = readbacks.slot(renderer, Key { size, fourcc })?;
            let mut fb = renderer.bind(slot.texture()).map_err(|e| e.to_string())?;
            tracker
                .render_output(renderer, &mut fb, 0, elements, clear)
                .map_err(|e| format!("{e:?}"))?;
            drop(fb);
            slot.read(renderer, 0..size.h)?;
            let sync = crate::gpufence::sync_now(renderer)?;
            crate::gpufence::issued(renderer, &sync);
            Ok(Drawn {
                sync,
                shm: Some(ShmCopy {
                    buffer: buffer.clone(),
                    slot,
                    size,
                    copied: vec![whole(size)],
                }),
                damage: vec![whole(size)],
            })
        }
    }
}

/// Draws `elements` into `target` where `region` (output pixels) lies, the rest of a dma-buf left
/// as it was; the caches of the framebuffer effects drawn are `effects`. An shm buffer's frame is
/// drawn into a pooled texture, and only `region` copied into the buffer: the texture's other
/// pixels are never read, unless a framebuffer effect, which reads past its own place, is drawn,
/// and then it is drawn whole.
fn draw_region(
    renderer: &mut GlesRenderer,
    readbacks: &mut Readbacks,
    view: View,
    elements: &[OutputElement],
    clear: Color32F,
    target: Target<'_>,
    (region, effects): (&[Rectangle<i32, Physical>], &mut crate::glass::Effects),
) -> Result<Drawn, String> {
    let size = view.0;
    let drawing = (elements, clear);
    match target {
        Target::Dmabuf(mut dmabuf) => {
            let mut fb = renderer.bind(&mut dmabuf).map_err(|e| e.to_string())?;
            let sync = paint(renderer, &mut fb, view, drawing, region, effects)?;
            crate::gpufence::issued(renderer, &sync);
            Ok(Drawn {
                sync,
                shm: None,
                damage: Vec::new(),
            })
        }
        Target::Shm(buffer) => {
            let fourcc = shm_fourcc(buffer, size)?;
            let mut slot = readbacks.slot(renderer, Key { size, fourcc })?;
            let mut fb = renderer.bind(slot.texture()).map_err(|e| e.to_string())?;
            let whole = [output_rect(view)];
            let drawn = match elements.iter().any(|e| e.is_framebuffer_effect()) {
                true => &whole[..],
                false => region,
            };
            let _ = paint(renderer, &mut fb, view, drawing, drawn, effects)?;
            drop(fb);
            let copied: Vec<_> = region.iter().map(|r| to_buffer(view, *r)).collect();
            slot.read(renderer, rows_of(&copied, size))?;
            let sync = crate::gpufence::sync_now(renderer)?;
            crate::gpufence::issued(renderer, &sync);
            Ok(Drawn {
                sync,
                shm: Some(ShmCopy {
                    buffer: buffer.clone(),
                    slot,
                    size,
                    copied,
                }),
                damage: Vec::new(),
            })
        }
    }
}

/// Draws `elements` over `clear` into `fb` where `region` (output pixels) lies, each pixel once
/// however many of its rectangles cover it.
fn paint<E: RenderElement<GlesRenderer>>(
    renderer: &mut GlesRenderer,
    fb: &mut GlesTarget<'_>,
    (size, scale, transform): View,
    (elements, clear): (&[E], Color32F),
    region: &[Rectangle<i32, Physical>],
    effects: &mut crate::glass::Effects,
) -> Result<SyncPoint, String> {
    let mut frame = renderer
        .render(fb, size, transform.invert())
        .map_err(|e| e.to_string())?;
    let scale = Scale::from(scale);
    let region = disjoint(region);
    crate::glass::draw_damaged(&mut frame, elements, scale, &region, clear, Some(effects))
        .map_err(|e| e.to_string())?;
    frame.finish().map_err(|e| e.to_string())
}

/// `rects` cut into rectangles that cover the same pixels without overlapping: an element is
/// drawn once per rectangle, and a translucent one drawn twice over a pixel darkens it.
fn disjoint(rects: &[Rectangle<i32, Physical>]) -> Vec<Rectangle<i32, Physical>> {
    let mut out: Vec<Rectangle<i32, Physical>> = Vec::new();
    for r in rects {
        let new = Rectangle::subtract_rects_many([*r], out.iter().copied());
        out.extend(new);
    }
    out
}

/// The fourcc of `buffer`, an shm buffer that must be `size`.
fn shm_fourcc(buffer: &WlBuffer, size: Size<i32, Physical>) -> Result<Fourcc, String> {
    shm::with_buffer_contents(buffer, |_, _, data| {
        if data.width != size.w || data.height != size.h {
            return Err("shm buffer of the wrong size".to_string());
        }
        match data.format {
            wl_shm::Format::Xrgb8888 => Ok(Fourcc::Xrgb8888),
            wl_shm::Format::Argb8888 => Ok(Fourcc::Argb8888),
            f => Err(format!("shm format {f:?} not offered")),
        }
    })
    .map_err(|e| format!("{e:?}"))?
}

impl ShmCopy {
    fn fill(&self, renderer: &mut GlesRenderer) -> Result<(), String> {
        let rows = rows_of(&self.copied, self.size);
        let first = rows.start;
        self.slot
            .bytes(renderer, rows, |bytes| self.copy(bytes, first))?
    }

    /// Copies the picture's rows from `first` on, which `bytes` holds, into the buffer.
    fn copy(&self, bytes: &[u8], first: i32) -> Result<(), String> {
        let row = self.size.w as usize * 4;
        let rows = self.size.h as usize;
        let read = rows_of(&self.copied, self.size);
        if bytes.len() < read.len() * row {
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
            // SAFETY: `pool` is the client's mapped pool of `pool_len`
            // bytes, and the check above keeps every row from `offset` inside it.
            unsafe {
                copy_rects(
                    (bytes, first),
                    pool.add(offset),
                    stride,
                    self.size,
                    &self.copied,
                )
            };
            Ok(())
        })
        .map_err(|e| format!("{e:?}"))?
    }
}

/// Copies the parts of a `size` picture of 4-byte pixels that `rects` cover into `out`, whose rows
/// are `stride` bytes apart; `picture` holds its rows from `first` on.
///
/// # Safety
/// `picture` holds every row from `first` that `rects` reach; `out` is valid for writes of
/// `size.h` rows of `size.w` pixels, `stride` apart, and `stride` is at least a row.
unsafe fn copy_rects(
    (picture, first): (&[u8], i32),
    out: *mut u8,
    stride: usize,
    size: Size<i32, Physical>,
    rects: &[Rectangle<i32, Buffer>],
) {
    let row = size.w as usize * 4;
    let area = Rectangle::from_size((size.w, size.h).into());
    for r in rects.iter().filter_map(|r| r.intersection(area)) {
        let (x, w) = (r.loc.x as usize * 4, r.size.w as usize * 4);
        for y in r.loc.y as usize..(r.loc.y + r.size.h) as usize {
            let from = &picture[(y - first as usize) * row + x..][..w];
            // SAFETY: `r` lies inside the picture, so the row's part lies inside `out`.
            unsafe { std::ptr::copy_nonoverlapping(from.as_ptr(), out.add(y * stride + x), w) };
        }
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
        let filled = match (reached, &drawn.shm) {
            (false, _) => Err(format!("the render did not finish in {deadline:?}")),
            (true, None) => Ok(()),
            (true, Some(copy)) => capture_renderer(&mut state.backend, &mut state.captures)
                .and_then(|(renderer, _)| copy.fill(renderer)),
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
        let next = match capture_renderer(&mut state.backend, &mut state.captures) {
            Ok((renderer, readbacks)) => readbacks.expire(renderer, Instant::now()),
            Err(_) => None,
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

/// Render `w`'s client surface tree, frame and popups left out, its geometry's top-left at 0,0,
/// into `target`; blank under its side's lock, as the screen shows the lock and not the window.
fn render_window(state: &mut State, w: WindowId, target: Target<'_>) -> Result<Drawn, String> {
    let size = window_size(state, w).ok_or("the window is not mapped")?;
    let scale = window_scale(state, w);
    let locked = state.window_locked(w);
    let win = state.win(w).ok_or("the window is gone")?;
    let surface = win.wl_surface().clone();
    let (sx, sy) = win.surface_origin();
    let (cx, cy) = win.client_origin();
    let at = Point::<i32, smithay::utils::Logical>::from((sx - cx, sy - cy))
        .to_f64()
        .to_physical(scale)
        .to_i32_round();
    let (renderer, readbacks) = capture_renderer(&mut state.backend, &mut state.captures)?;
    crate::gpufence::ready(renderer)?;
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
    state.captures.held(&buffer, None);
    let frame_ref = FrameRef::clone(&frame);
    if drawn.is_ok() {
        fresh(session).clear();
        state
            .captures
            .drawing
            .push((frame_ref.clone(), session.clone()));
    }
    let what = format!("capture of window {w:?}");
    let session = session.clone();
    complete(
        state,
        drawn,
        frame,
        Transform::Normal,
        what,
        move |state, ready| {
            state.captures.drawing.retain(|(f, _)| *f != frame_ref);
            if !ready {
                fresh(&session).mark();
            }
        },
    );
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
    then: impl FnOnce(&mut State, bool) + 'static,
) {
    let now = state.now();
    let drawn = match drawn {
        Ok(d) => d,
        Err(e) => {
            log::warn!("{what}: {e}");
            then(state, false);
            return frame.fail(CaptureFailureReason::Unknown);
        }
    };
    let damage = drawn.damage.clone();
    when_done(state, drawn, move |state, done| match done {
        Ok(()) => {
            then(state, true);
            frame.success(transform, damage, now);
        }
        Err(e) => {
            log::warn!("{what}: {e}");
            then(state, false);
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
    let buffer = frame.buffer();
    state.captures.held(&buffer, None);
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
    let Ok((renderer, readbacks)) = capture_renderer(backend, captures) else {
        return frame.fail(CaptureFailureReason::Unknown);
    };
    if let Err(e) = crate::gpufence::ready(renderer) {
        log::warn!("cursor capture on {}: {e}", output.name());
        return frame.fail(CaptureFailureReason::Unknown);
    }
    let parts = Parts {
        scene,
        cursor,
        cursors,
        pointer,
    };
    let (elements, hotspot) = cursor_image(renderer, parts, Scale::from(scale));
    let at = ((side, side).into(), scale, Transform::Normal);
    let drawn = draw(
        renderer,
        readbacks,
        at,
        &elements,
        Color32F::TRANSPARENT,
        Target::of(&buffer),
    );
    let what = format!("cursor capture on {}", output.name());
    let session = session.clone();
    complete(
        state,
        drawn,
        frame,
        Transform::Normal,
        what,
        move |_, ready| {
            if ready {
                session.set_cursor_hotspot((hotspot.x, hotspot.y));
            }
        },
    );
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

/// An ext output capture's buffer size: the output upright, as laid out,
/// so a consumer needs no transform. wlr-screencopy keeps the panel's.
fn upright_size(output: &Output) -> Option<Size<i32, Physical>> {
    let mode = output.current_mode()?;
    Some(output.current_transform().transform_size(mode.size))
}

fn fill_ext(state: &mut State, output: &Output, session: &SessionRef, frame: Frame) {
    let Some(size) = upright_size(output) else {
        return frame.fail(CaptureFailureReason::Stopped);
    };
    let buffer = frame.buffer();
    if !fits(&buffer, size) {
        return frame.fail(CaptureFailureReason::BufferConstraints);
    }
    let effects = recent(
        &mut state.captures.effects,
        session.clone(),
        CACHED_SESSIONS,
    );
    let stream = Stream {
        damage: session_damage(session),
        buffer: &buffer,
        ask: Ask::Ext {
            asked: frame.damage(),
            effects,
        },
    };
    let drawn = match render_into(
        state,
        output,
        (0, 0).into(),
        (size, Transform::Normal),
        session.draw_cursor(),
        Target::of(&buffer),
        stream,
    ) {
        Ok(Some(d)) => Ok(d),
        Ok(None) => {
            fresh(session).clear();
            let session = session.clone();
            return state.captures.waiting.push(Waiting::Ext { session, frame });
        }
        Err(e) => Err(e),
    };
    state.captures.held(&buffer, Some(session_damage(session)));
    let frame_ref = FrameRef::clone(&frame);
    if drawn.is_ok() {
        fresh(session).clear();
        state
            .captures
            .drawing
            .push((frame_ref.clone(), session.clone()));
    }
    let what = format!("capture of {}", output.name());
    let session = session.clone();
    complete(
        state,
        drawn,
        frame,
        Transform::Normal,
        what,
        move |state, ready| {
            state.captures.drawing.retain(|(f, _)| *f != frame_ref);
            if !ready {
                unseen(&session);
            }
        },
    );
}

/// Fills `screencopy`, or hands it back when it waits for a change.
fn fill_wlr(state: &mut State, screencopy: Screencopy) -> Option<Screencopy> {
    if !screencopy.is_alive() {
        return None;
    }
    let info = screencopy.info.clone();
    let buffer = match &screencopy.buffer {
        ScreencopyBuffer::Shm(b) => Target::Shm(b),
        ScreencopyBuffer::Dmabuf(d) => Target::Dmabuf(d.clone()),
    };
    let damage = wlr_stream(state, &info);
    let stream = Stream {
        damage: &damage,
        buffer: &screencopy.wl_buffer,
        ask: Ask::Wlr {
            waits: screencopy.with_damage,
        },
    };
    let drawn = render_into(
        state,
        &info.output,
        info.region_loc,
        (info.buffer_size, info.output.current_transform()),
        info.overlay_cursor,
        buffer,
        stream,
    );
    let name = info.output.name();
    let now = state.now();
    let buffer = screencopy.wl_buffer.clone();
    if !matches!(drawn, Ok(None)) {
        state.captures.held(&buffer, Some(&damage));
    }
    // Dropping the screencopy fails its frame.
    match drawn {
        Ok(Some(drawn)) => {
            damage.borrow_mut().copying(&buffer);
            let changed = drawn.damage.clone();
            when_done(state, drawn, move |_, done| {
                let sent = done.is_ok() && screencopy.is_alive();
                damage.borrow_mut().landed(&buffer, sent);
                match done {
                    Ok(()) if sent => screencopy.submit(now, &changed),
                    Ok(()) => {}
                    Err(e) => log::warn!("screencopy of {name}: {e}"),
                }
            })
        }
        Ok(None) => return Some(screencopy),
        Err(e) => {
            damage.borrow_mut().lost(&buffer);
            log::warn!("screencopy of {name}: {e}");
        }
    }
    None
}

/// The damage of the wlr client's copies of `info`'s view through `info.manager`.
fn wlr_stream(state: &mut State, info: &FrameInfo) -> Rc<RefCell<SessionDamage>> {
    let streams = &mut state.captures.wlr_streams;
    streams.retain(|(v, _)| v.manager.is_alive());
    recent(streams, WlrView::of(info), WLR_VIEWS)
}

/// The value `kept` holds for `key`, made if missing and moved last; past `cap` entries the first
/// goes.
fn recent<K: PartialEq, V: Clone + Default>(kept: &mut Vec<(K, V)>, key: K, cap: usize) -> V {
    let entry = match kept.iter().position(|(k, _)| *k == key) {
        Some(i) => kept.remove(i),
        None => (key, V::default()),
    };
    let value = entry.1.clone();
    kept.push(entry);
    if kept.len() > cap {
        kept.remove(0);
    }
    value
}

impl State {
    /// The output `w`'s frame callbacks for a waiting capture go out on: where its frame's
    /// centre lies.
    fn wake_output(&self, w: WindowId) -> Option<Output> {
        let r = self.win(w)?.frame_rect();
        self.output_at((r.x + r.width / 2) as f64, (r.y + r.height / 2) as f64)
    }

    /// Ask `w`'s client for a frame for a capture waiting on its commit;
    /// one minimized, covered or off screen gets no frame callbacks otherwise.
    fn wake_window(&mut self, w: WindowId) {
        let Some(output) = self.wake_output(w) else {
            return;
        };
        let Some(win) = self.win(w) else {
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

    /// [`State::wake_window`] one refresh from now, while a capture still waits on `w`: its
    /// client committed nothing new, and asked again at once it would commit again at once.
    fn wake_window_soon(&mut self, w: WindowId) {
        let Some(output) = self.wake_output(w) else {
            return;
        };
        if self.captures.waking.contains(&w) {
            return;
        }
        let refresh = output.current_mode().map_or(0, |m| m.refresh);
        let timer = Timer::from_duration(crate::framepace::interval(refresh));
        let armed = self.handle.insert_source(timer, move |_, _, state| {
            state.captures.waking.retain(|x| *x != w);
            let on = Some(Shown::Window(w));
            if state.captures.waiting.iter().any(|x| x.shows() == on) {
                state.wake_window(w);
            }
            TimeoutAction::Drop
        });
        match armed {
            Ok(_) => self.captures.waking.push(w),
            Err(e) => log::warn!("capture of window {w:?}: no wake timer: {}", e.error),
        }
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

/// Whether `session`'s client marked itself the screen recorder's capture connection.
fn for_recorder(session: &SessionRef) -> bool {
    session.client().is_some_and(|c| {
        c.get_data::<crate::compositor::ClientState>()
            .is_some_and(|d| d.recorder_capture.load(Ordering::Relaxed))
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

/// The most buffers a stream keeps damage for; one cycled less recently is drawn whole.
const STREAM_BUFFERS: usize = 8;
/// The most rectangles a buffer's damage keeps; past it, their bounding box.
const BUFFER_RECTS: usize = 16;

/// An output stream's damage, kept from one frame to the next.
struct SessionDamage<B = WlBuffer> {
    tracked: Option<(View, Color32F, OutputDamageTracker)>,
    /// The buffers seen, least recently filled first, each with what changed since, in output
    /// pixels.
    buffers: Vec<(B, Vec<Rectangle<i32, Physical>>)>,
    /// The buffers of wlr copies drawn and not yet landed, one entry per copy.
    in_flight: Vec<B>,
}

impl<B> Default for SessionDamage<B> {
    fn default() -> Self {
        SessionDamage {
            tracked: None,
            buffers: Vec::new(),
            in_flight: Vec::new(),
        }
    }
}

/// A client's buffer as a stream keeps it.
trait Kept: PartialEq + Clone {
    fn alive(&self) -> bool;
}

impl Kept for WlBuffer {
    fn alive(&self) -> bool {
        self.is_alive()
    }
}

impl<B: Kept> SessionDamage<B> {
    /// What `elements` changed since the last call, in output pixels, which every buffer seen
    /// takes too: all of it on the first call and after `view` or `clear` changed, which also
    /// forgets the buffers.
    fn changed<E: Element>(
        &mut self,
        view: View,
        clear: Color32F,
        elements: &[E],
    ) -> Vec<Rectangle<i32, Physical>> {
        let (size, scale, transform) = view;
        if self
            .tracked
            .as_ref()
            .is_none_or(|(v, c, _)| *v != view || *c != clear)
        {
            let tracker = OutputDamageTracker::new(size, scale, transform);
            self.tracked = Some((view, clear, tracker));
            self.buffers.clear();
        }
        let (_, _, tracker) = self.tracked.as_mut().expect("set above");
        let changed = match tracker.damage_output(1, elements) {
            Ok((damage, _)) => damage.cloned().unwrap_or_default(),
            Err(_) => vec![output_rect(view)],
        };
        self.buffers.retain(|(b, _)| b.alive());
        for (_, stale) in &mut self.buffers {
            add(stale, &changed);
        }
        changed
    }

    /// Where a `view` frame into `buffer` draws, in output pixels: what changed since it last held
    /// a frame (all of it for a buffer not seen) and `asked`, what the client says changed in it
    /// (buffer pixels; past [`BUFFER_RECTS`], their bounds). The buffer then holds the latest frame.
    fn take(
        &mut self,
        view: View,
        buffer: &B,
        asked: &[Rectangle<i32, Buffer>],
    ) -> Vec<Rectangle<i32, Physical>> {
        let mut region = match self.buffers.iter().position(|(b, _)| b == buffer) {
            Some(i) => self.buffers.remove(i).1,
            None => vec![output_rect(view)],
        };
        let mut cut = Vec::new();
        add(&mut cut, &within(asked, view.0));
        region.extend(cut.into_iter().map(|r| from_buffer(view, r)));
        if self.buffers.len() >= STREAM_BUFFERS {
            self.buffers.remove(0);
        }
        self.buffers.push((buffer.clone(), Vec::new()));
        region
    }

    /// What a wlr `view` frame into `buffer` reports, as [`SessionDamage::take`] gives it, or
    /// whole while an earlier copy into it may yet fail; None, with nothing taken, when the frame
    /// `waits` and the buffer holds the latest picture.
    fn wlr_frame(
        &mut self,
        view: View,
        buffer: &B,
        waits: bool,
    ) -> Option<Vec<Rectangle<i32, Physical>>> {
        let in_flight = self.in_flight.contains(buffer);
        let current =
            !in_flight && (self.buffers.iter()).any(|(b, stale)| b == buffer && stale.is_empty());
        if waits && current {
            return None;
        }
        let since = self.take(view, buffer, &[]);
        Some(match in_flight {
            true => vec![output_rect(view)],
            false => since,
        })
    }

    /// A wlr copy into `buffer` was drawn and goes out once the GPU is done.
    fn copying(&mut self, buffer: &B) {
        self.in_flight.push(buffer.clone());
    }

    /// A wlr copy into `buffer` reached its client, or not when `sent` is false.
    fn landed(&mut self, buffer: &B, sent: bool) {
        if let Some(i) = self.in_flight.iter().position(|b| b == buffer) {
            self.in_flight.remove(i);
        }
        if !sent {
            self.lost(buffer);
        }
    }

    /// `buffer` holds no frame of this stream known: its next is drawn whole.
    fn lost(&mut self, buffer: &B) {
        self.buffers.retain(|(b, _)| b != buffer);
    }

    /// The client destroyed `buffer`.
    fn gone(&mut self, buffer: &B) {
        self.lost(buffer);
        self.in_flight.retain(|b| b != buffer);
    }

    /// The frame last tracked never reached the client: the next is all damaged, in every buffer.
    fn forget(&mut self) {
        self.tracked = None;
        self.buffers.clear();
    }
}

/// `buffer` now holds a frame of `holder`, or of no output stream: every other of `streams` forgets
/// it.
fn held_by<'a, B: Kept + 'a>(
    streams: impl IntoIterator<Item = &'a RefCell<SessionDamage<B>>>,
    holder: Option<&RefCell<SessionDamage<B>>>,
    buffer: &B,
) {
    for s in streams {
        if holder.is_none_or(|h| !std::ptr::eq(s, h)) {
            s.borrow_mut().lost(buffer);
        }
    }
}

/// The parts of `rects` inside a `size` buffer.
fn within(
    rects: &[Rectangle<i32, Buffer>],
    size: Size<i32, Physical>,
) -> Vec<Rectangle<i32, Buffer>> {
    let cut = |r: &Rectangle<i32, Buffer>| {
        let (x, y) = (r.loc.x.clamp(0, size.w), r.loc.y.clamp(0, size.h));
        let right = r.loc.x.saturating_add(r.size.w).clamp(0, size.w);
        let bottom = r.loc.y.saturating_add(r.size.h).clamp(0, size.h);
        (right > x && bottom > y)
            .then(|| Rectangle::new((x, y).into(), (right - x, bottom - y).into()))
    };
    rects.iter().filter_map(cut).collect()
}

/// `changed` added to `stale`, kept to [`BUFFER_RECTS`] rectangles.
fn add<K>(stale: &mut Vec<Rectangle<i32, K>>, changed: &[Rectangle<i32, K>]) {
    stale.extend_from_slice(changed);
    if stale.len() > BUFFER_RECTS {
        let bounds = stale.iter().copied().reduce(|a, b| a.merge(b));
        stale.clear();
        stale.extend(bounds);
    }
}

/// The whole of a `view` capture, in output pixels.
fn output_rect((size, _, transform): View) -> Rectangle<i32, Physical> {
    Rectangle::from_size(transform.transform_size(size))
}

/// `r`, in output pixels, in the pixels of a `view` capture's buffer, as DrmCompositor maps element
/// geometry.
fn to_buffer(view: View, r: Rectangle<i32, Physical>) -> Rectangle<i32, Buffer> {
    let (size, _, transform) = view;
    let to_buffer = transform.invert();
    let r = to_buffer.transform_rect_in(r, &to_buffer.transform_size(size));
    Rectangle::new((r.loc.x, r.loc.y).into(), (r.size.w, r.size.h).into())
}

/// [`to_buffer`] undone.
fn from_buffer(view: View, r: Rectangle<i32, Buffer>) -> Rectangle<i32, Physical> {
    let (size, _, transform) = view;
    let to_buffer = transform.invert();
    // `transform_rect_in` undoes a flipped transform with itself, a rotation with its inverse.
    let back = match to_buffer.flipped() {
        true => to_buffer,
        false => to_buffer.invert(),
    };
    let r = Rectangle::<i32, Physical>::new((r.loc.x, r.loc.y).into(), (r.size.w, r.size.h).into());
    back.transform_rect_in(r, &size)
}

fn whole(size: Size<i32, Physical>) -> Rectangle<i32, Buffer> {
    Rectangle::from_size((size.w, size.h).into())
}

/// `session`'s frame drawn last never reached its client: its next is filled at once, and whole.
fn unseen(session: &SessionRef) {
    session_damage(session).borrow_mut().forget();
    fresh(session).mark();
}

fn session_damage(session: &SessionRef) -> &RefCell<SessionDamage> {
    session
        .user_data()
        .insert_if_missing(|| RefCell::new(SessionDamage::<WlBuffer>::default()));
    session
        .user_data()
        .get::<RefCell<SessionDamage>>()
        .expect("inserted above")
}

impl ScreencopyHandler for State {
    fn screencopy_frame(&mut self, screencopy: Screencopy) {
        let key = wlr_key(&screencopy);
        let was_live = crate::captureborder::wlr_live(self, &key);
        let client = screencopy.wl_client();
        if let Some(waits) = fill_wlr(self, screencopy) {
            let output = waits.info.output.clone();
            self.captures.waiting.push(Waiting::Wlr(waits));
            // Only the request asks: a retry that asked again would ask after every frame.
            wait_on(self, &output);
        }
        crate::captureborder::wlr_frame(self, key.clone());
        // A recording already bordered and lighting the dot changes neither with its frames.
        if was_live && crate::captureborder::wlr_live(self, &key) {
            return;
        }
        crate::privacy::frame_taken(self, client.as_ref(), false);
        crate::captureborder::refresh(self);
        crate::privacy::captures_changed(self);
    }

    fn screencopy_gone(&mut self, frame: &ZwlrScreencopyFrameV1) {
        (self.captures.waiting).retain(|w| !matches!(w, Waiting::Wlr(s) if s.is(frame)));
    }
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
        let size = upright_size(&output)?;
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
        self.image_copy_capture_state.cleanup();
    }

    fn frame(&mut self, session: &SessionRef, frame: Frame) {
        if asked(session).frame() {
            crate::privacy::captures_changed(self);
            crate::captureborder::refresh(self);
        }
        let shows = shown(&session.source());
        let window = matches!(shows, Some(Shown::Window(_)));
        let taken = shows.is_some();
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
                wait_on(self, &o);
            }
            Some(Shown::Window(w)) => {
                self.captures.waiting.push(Waiting::Ext {
                    session: session.clone(),
                    frame,
                });
                self.wake_window(w);
            }
        }
        // An ongoing session already lights the sharing dot; only its first frame is a still.
        if taken && !asked(session).ongoing() {
            crate::privacy::frame_taken(self, session.client().as_ref(), window);
        }
    }

    fn frame_aborted(&mut self, frame: FrameRef) {
        self.captures
            .waiting
            .retain(|w| !matches!(w, Waiting::Ext { frame: f, .. } if *f == frame));
        let drawing = &mut self.captures.drawing;
        if let Some(i) = drawing.iter().position(|(f, _)| *f == frame) {
            let (_, session) = drawing.swap_remove(i);
            unseen(&session);
        }
    }

    fn session_destroyed(&mut self, session: SessionRef) {
        self.captures.sessions.retain(|s| *s != session);
        self.captures.effects.retain(|(s, _)| *s != session);
        // smithay keeps every session, with what its user data holds, until told to let go.
        self.image_copy_capture_state.cleanup();
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
            Bind, Color32F, ExportMem, Offscreen,
            element::{Element, Id, Kind, solid::SolidColorRenderElement},
            gles::GlesTexture,
        },
        utils::{Buffer, Physical, Point, Rectangle, Scale, Size, Transform},
    };

    use std::cell::RefCell;
    use std::rc::Rc;

    use super::{Fresh, Kept, SessionDamage, View};
    use crate::harness::Harness;
    use crate::render::{OutputElement, Parts, output_elements};

    impl Kept for u32 {
        fn alive(&self) -> bool {
            true
        }
    }

    type Rects = Vec<Rectangle<i32, Physical>>;

    fn hd() -> View {
        (Size::new(1280, 720), 1.0, Transform::Normal)
    }
    const CLEAR: Color32F = Color32F::new(0.0, 0.0, 0.0, 1.0);

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Physical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    fn whole(w: i32, h: i32) -> Rects {
        vec![rect(0, 0, w, h)]
    }

    fn solid(id: &Id, (x, y, w, h): (i32, i32, i32, i32)) -> SolidColorRenderElement {
        SolidColorRenderElement::new(
            id.clone(),
            rect(x, y, w, h),
            0,
            [1.0, 0.0, 0.0, 1.0],
            Kind::Unspecified,
        )
    }

    /// Whether `damage` covers every pixel of `r`.
    fn covers(damage: &[Rectangle<i32, Physical>], r: Rectangle<i32, Physical>) -> bool {
        Rectangle::subtract_rects_many([r], damage.iter().copied()).is_empty()
    }

    /// The pixels of `damage`, each counted once.
    fn area(damage: &[Rectangle<i32, Physical>]) -> i32 {
        let mut seen: Rects = Vec::new();
        let mut n = 0;
        for r in damage {
            for part in Rectangle::subtract_rects_many([*r], seen.iter().copied()) {
                n += part.size.w * part.size.h;
            }
            seen.push(*r);
        }
        n
    }

    fn physical(r: crate::raster::Rect) -> Rectangle<i32, Physical> {
        rect(r.x, r.y, r.width, r.height)
    }

    /// One frame of `d` into `buffer` showing `elements`: what it reports and what it draws.
    fn frame(
        d: &mut SessionDamage<u32>,
        view: View,
        buffer: u32,
        elements: &[SolidColorRenderElement],
    ) -> (Rects, Rects) {
        let changed = d.changed(view, CLEAR, elements);
        (changed, d.take(view, &buffer, &[]))
    }

    #[test]
    fn a_first_output_capture_frame_is_fully_damaged() {
        let id = Id::new();
        let mut d = SessionDamage::<u32>::default();
        let (changed, drawn) = frame(&mut d, hd(), 1, &[solid(&id, (10, 10, 20, 20))]);
        assert_eq!(changed, whole(1280, 720), "the whole 1280x720 buffer");
        assert_eq!(drawn, whole(1280, 720), "drawn whole");
    }

    #[test]
    fn a_second_frame_into_the_same_buffer_after_a_small_change_draws_and_reports_only_it() {
        let (a, b) = (Id::new(), Id::new());
        let mut d = SessionDamage::<u32>::default();
        let still = solid(&a, (500, 300, 200, 100));
        frame(
            &mut d,
            hd(),
            1,
            &[solid(&b, (10, 10, 2, 16)), still.clone()],
        );
        let (changed, drawn) = frame(&mut d, hd(), 1, &[solid(&b, (12, 10, 2, 16)), still]);
        assert!(
            covers(&changed, rect(10, 10, 4, 16)),
            "the caret's old and new place: {changed:?}"
        );
        assert!(area(&changed) <= 4 * 16, "and nothing else: {changed:?}");
        assert_eq!(
            area(&drawn),
            area(&changed),
            "the frame draws what it reports: {drawn:?}"
        );
        assert!(covers(&changed, rect(12, 10, 2, 16)) && covers(&drawn, rect(10, 10, 4, 16)));
    }

    #[test]
    fn a_buffer_reused_after_other_frames_draws_the_union_of_their_changes() {
        let id = Id::new();
        let caret = |x| [solid(&id, (x, 10, 2, 16))];
        let mut d = SessionDamage::<u32>::default();
        for buffer in [1, 2, 3] {
            frame(&mut d, hd(), buffer, &caret(0));
        }
        let (changed, drawn) = frame(&mut d, hd(), 1, &caret(100));
        assert_eq!(area(&changed), 2 * 2 * 16, "one move");
        assert_eq!(
            area(&drawn),
            area(&changed),
            "buffer 1 missed nothing else: {drawn:?}"
        );
        frame(&mut d, hd(), 2, &caret(200));
        frame(&mut d, hd(), 3, &caret(300));
        let (changed, drawn) = frame(&mut d, hd(), 1, &caret(300));
        assert_eq!(changed, Rects::new(), "nothing moved since the last frame");
        for x in [100, 200] {
            assert!(
                covers(&drawn, rect(x, 10, 2, 16)),
                "the caret left {x}: {drawn:?}"
            );
        }
        assert!(
            covers(&drawn, rect(300, 10, 2, 16)),
            "and reached 300: {drawn:?}"
        );
        assert_eq!(
            area(&drawn),
            3 * 2 * 16,
            "the two frames into buffers 2 and 3, no more"
        );
    }

    #[test]
    fn a_buffer_not_seen_is_drawn_whole() {
        let id = Id::new();
        let mut d = SessionDamage::<u32>::default();
        frame(&mut d, hd(), 1, &[solid(&id, (10, 10, 20, 20))]);
        let (changed, drawn) = frame(&mut d, hd(), 2, &[solid(&id, (10, 10, 20, 20))]);
        assert_eq!(changed, Rects::new(), "nothing changed");
        assert_eq!(drawn, whole(1280, 720), "but buffer 2 holds no frame yet");
    }

    #[test]
    fn a_new_size_scale_or_transform_draws_every_buffer_whole() {
        let id = Id::new();
        let at = |d: &mut SessionDamage<u32>, view: View, buffer| {
            frame(d, view, buffer, &[solid(&id, (10, 10, 20, 20))])
        };
        let fhd = (Size::new(1920, 1080), 1.0, Transform::Normal);
        let scaled = (Size::new(1920, 1080), 2.0, Transform::Normal);
        let turned = (Size::new(1920, 1080), 2.0, Transform::_90);
        let mut d = SessionDamage::<u32>::default();
        at(&mut d, hd(), 1);
        at(&mut d, hd(), 2);
        for (view, what, w, h) in [
            (fhd, "a new size", 1920, 1080),
            (scaled, "a new scale", 1920, 1080),
            (turned, "a new transform, in output pixels", 1080, 1920),
        ] {
            for buffer in [1, 2] {
                let (changed, drawn) = at(&mut d, view, buffer);
                if buffer == 1 {
                    assert_eq!(changed, whole(w, h), "{what} reports it all");
                }
                assert_eq!(drawn, whole(w, h), "{what} draws buffer {buffer} whole");
            }
        }
    }

    #[test]
    fn a_frame_that_never_reached_the_client_leaves_every_buffer_drawn_whole() {
        let id = Id::new();
        let mut d = SessionDamage::<u32>::default();
        frame(&mut d, hd(), 1, &[solid(&id, (10, 10, 20, 20))]);
        frame(&mut d, hd(), 2, &[solid(&id, (10, 10, 20, 20))]);
        d.forget();
        for buffer in [1, 2] {
            let (_, drawn) = frame(&mut d, hd(), buffer, &[solid(&id, (10, 10, 20, 20))]);
            assert_eq!(drawn, whole(1280, 720), "buffer {buffer} is drawn whole");
        }
    }

    #[test]
    fn what_the_client_says_it_damaged_is_drawn_too() {
        let id = Id::new();
        let mut d = SessionDamage::<u32>::default();
        frame(&mut d, hd(), 1, &[solid(&id, (10, 10, 20, 20))]);
        d.changed(hd(), CLEAR, &[solid(&id, (10, 10, 20, 20))]);
        let asked = Rectangle::<i32, Buffer>::new((600, 400).into(), (8, 8).into());
        let drawn = d.take(hd(), &1, &[asked]);
        assert_eq!(drawn, vec![rect(600, 400, 8, 8)], "the client's rectangle");
    }

    #[test]
    fn a_buffer_keeps_at_most_sixteen_rectangles_then_their_bounds() {
        let id = Id::new();
        let mut d = SessionDamage::<u32>::default();
        frame(&mut d, hd(), 1, &[solid(&id, (0, 0, 2, 2))]);
        frame(&mut d, hd(), 2, &[solid(&id, (0, 0, 2, 2))]);
        for i in 1..=20 {
            frame(&mut d, hd(), 2, &[solid(&id, (i * 10, i * 10, 2, 2))]);
        }
        let (_, drawn) = frame(&mut d, hd(), 1, &[solid(&id, (200, 200, 2, 2))]);
        assert!(drawn.len() <= 16, "{} rectangles", drawn.len());
        assert!(covers(&drawn, rect(0, 0, 2, 2)) && covers(&drawn, rect(200, 200, 2, 2)));
    }

    #[test]
    fn buffer_and_output_pixels_map_back_and_forth_under_every_transform() {
        let r = Rectangle::<i32, Buffer>::new((10, 20).into(), (30, 40).into());
        for transform in [
            Transform::Normal,
            Transform::_90,
            Transform::_180,
            Transform::_270,
            Transform::Flipped,
            Transform::Flipped90,
            Transform::Flipped180,
            Transform::Flipped270,
        ] {
            let view = (Size::new(1920, 1080), 1.0, transform);
            let out = super::from_buffer(view, r);
            assert!(
                super::output_rect(view).contains_rect(out),
                "{transform:?}: {out:?} lies on the output"
            );
            assert_eq!(super::to_buffer(view, out), r, "{transform:?} maps back");
        }
    }

    /// The elements of a capture of `output` as the harness shows it, the pointer drawn at `pointer`.
    fn capture_elements(
        renderer: &mut smithay::backend::renderer::gles::GlesRenderer,
        h: &mut Harness,
        output: &smithay::output::Output,
        pointer: Option<Point<f64, smithay::utils::Logical>>,
    ) -> Vec<OutputElement> {
        let s = &mut h.state;
        let parts = Parts {
            scene: &s.scene,
            cursor: &s.cursor,
            cursors: &mut s.cursors,
            pointer: pointer.unwrap_or_default(),
        };
        output_elements(renderer, parts, output, Point::default(), pointer.is_some())
    }

    #[test]
    fn moving_a_window_damages_only_its_old_and_new_place() {
        let Ok(mut renderer) = reprisefx::headless::renderer() else {
            reprisekit::pins::skipped("no renderer");
            return;
        };
        let mut h = Harness::new();
        let output = h.output("TEST-1", 1280, 720);
        let mut c = h.connect();
        let t = h.map_toplevel(&mut c, "test.app", 200, 100);
        h.state.move_to(t.id, 100, 100);
        h.dispatch();
        let mut d = SessionDamage::<u32>::default();
        let before = h.state.win(t.id).unwrap().frame_rect();
        let elements = capture_elements(&mut renderer, &mut h, &output, None);
        d.changed(hd(), CLEAR, &elements);
        h.state.move_to(t.id, 400, 300);
        h.dispatch();
        let after = h.state.win(t.id).unwrap().frame_rect();
        let elements = capture_elements(&mut renderer, &mut h, &output, None);
        let damage = d.changed(hd(), CLEAR, &elements);
        assert!(
            covers(&damage, physical(before)),
            "the old place {before:?}: {damage:?}"
        );
        assert!(
            covers(&damage, physical(after)),
            "the new place {after:?}: {damage:?}"
        );
        assert!(
            !damage.iter().any(|r| r.overlaps(rect(0, 0, 50, 50))),
            "(0,0)-(50,50) is untouched: {damage:?}"
        );
    }

    #[test]
    fn a_menu_the_compositor_opens_is_damage() {
        let Ok(mut renderer) = reprisefx::headless::renderer() else {
            reprisekit::pins::skipped("no renderer");
            return;
        };
        let mut h = Harness::new();
        let output = h.output("TEST-1", 1280, 720);
        h.dispatch();
        let mut d = SessionDamage::<u32>::default();
        let elements = capture_elements(&mut renderer, &mut h, &output, None);
        d.changed(hd(), CLEAR, &elements);
        let bar = h.state.bars[0].tree;
        h.state.toggle_start(bar);
        h.state
            .menu_open_step(std::time::Instant::now() + std::time::Duration::from_secs(5));
        h.dispatch();
        let m = h.state.menus.last().expect("the start menu is open");
        let menu = rect(m.x, m.y, m.layout.width, m.layout.height);
        let elements = capture_elements(&mut renderer, &mut h, &output, None);
        let damage = d.changed(hd(), CLEAR, &elements);
        assert!(covers(&damage, menu), "the menu at {menu:?}: {damage:?}");
    }

    #[test]
    fn a_pointer_drawn_in_damages_where_it_was_and_where_it_is() {
        let Ok(mut renderer) = reprisefx::headless::renderer() else {
            reprisekit::pins::skipped("no renderer");
            return;
        };
        let mut h = Harness::new();
        let output = h.output("TEST-1", 1280, 720);
        h.dispatch();
        let mut d = SessionDamage::<u32>::default();
        let at = |x, y| Some(Point::from((x, y)));
        let elements = capture_elements(&mut renderer, &mut h, &output, at(300.0, 300.0));
        d.changed(hd(), CLEAR, &elements);
        let elements = capture_elements(&mut renderer, &mut h, &output, at(900.0, 500.0));
        let damage = d.changed(hd(), CLEAR, &elements);
        assert!(!damage.is_empty(), "the pointer moved");
        for p in [(300, 300), (900, 500)] {
            assert!(
                damage.iter().any(|r| r.contains(p)),
                "the pointer's place {p:?}: {damage:?}"
            );
        }
        assert!(area(&damage) < 2 * 128 * 128, "and little else: {damage:?}");
    }

    /// A `w`x`h` texture filled with `colour`, bound.
    fn filled(
        renderer: &mut smithay::backend::renderer::gles::GlesRenderer,
        (w, h): (i32, i32),
        colour: Color32F,
    ) -> GlesTexture {
        use smithay::backend::renderer::{Frame, Renderer};
        let mut texture: GlesTexture = renderer
            .create_buffer(smithay::backend::allocator::Fourcc::Abgr8888, (w, h).into())
            .expect("a texture");
        let mut fb = renderer.bind(&mut texture).expect("bound");
        let mut frame = renderer
            .render(&mut fb, (w, h).into(), Transform::Normal)
            .expect("a frame");
        frame.clear(colour, &[rect(0, 0, w, h)]).expect("cleared");
        let _ = frame.finish().expect("finished");
        drop(fb);
        texture
    }

    /// The RGBA bytes of `texture`.
    fn pixels(
        renderer: &mut smithay::backend::renderer::gles::GlesRenderer,
        texture: &mut GlesTexture,
        (w, h): (i32, i32),
    ) -> Vec<u8> {
        let fb = renderer.bind(texture).expect("bound");
        let mapping = renderer
            .copy_framebuffer(
                &fb,
                Rectangle::from_size((w, h).into()),
                smithay::backend::allocator::Fourcc::Abgr8888,
            )
            .expect("read");
        drop(fb);
        renderer.map_texture(&mapping).expect("mapped").to_vec()
    }

    #[test]
    fn a_frame_draws_only_its_region_and_leaves_the_rest_of_the_buffer() {
        let Ok(mut renderer) = reprisefx::headless::renderer() else {
            reprisekit::pins::skipped("no renderer");
            return;
        };
        let size = (64, 48);
        let view: View = (Size::new(64, 48), 1.0, Transform::Normal);
        let stale = Color32F::new(0.0, 0.0, 1.0, 1.0);
        let mut texture = filled(&mut renderer, size, stale);
        let id = Id::new();
        let elements = [OutputElement::Border(solid(&id, (0, 0, 64, 48)))];
        let region = [rect(8, 8, 4, 4)];
        let mut fb = renderer.bind(&mut texture).expect("bound");
        let mut effects = Default::default();
        let _ = super::paint(
            &mut renderer,
            &mut fb,
            view,
            (&elements, CLEAR),
            &region,
            &mut effects,
        )
        .expect("drawn");
        drop(fb);
        let px = pixels(&mut renderer, &mut texture, size);
        let at = |x: usize, y: usize| px[(y * 64 + x) * 4..][..3].to_vec();
        assert_eq!(
            at(9, 9),
            [255, 0, 0],
            "inside the region the frame is drawn"
        );
        for (x, y) in [(0, 0), (7, 9), (12, 9), (9, 12), (63, 47)] {
            assert_eq!(
                at(x, y),
                [0, 0, 255],
                "({x},{y}) keeps what the buffer held"
            );
        }
    }

    #[test]
    fn overlapping_parts_of_the_region_blend_a_translucent_element_once() {
        let Ok(mut renderer) = reprisefx::headless::renderer() else {
            reprisekit::pins::skipped("no renderer");
            return;
        };
        let size = (32, 32);
        let view: View = (Size::new(32, 32), 1.0, Transform::Normal);
        let mut texture = filled(&mut renderer, size, CLEAR);
        let id = Id::new();
        let half = SolidColorRenderElement::new(
            id,
            rect(0, 0, 32, 32),
            0,
            [0.5, 0.0, 0.0, 0.5],
            Kind::Unspecified,
        );
        let elements = [OutputElement::Border(half)];
        let region = [rect(0, 0, 16, 16), rect(8, 8, 16, 16), rect(8, 8, 4, 4)];
        let mut fb = renderer.bind(&mut texture).expect("bound");
        let mut effects = Default::default();
        let _ = super::paint(
            &mut renderer,
            &mut fb,
            view,
            (&elements, CLEAR),
            &region,
            &mut effects,
        )
        .expect("drawn");
        drop(fb);
        let px = pixels(&mut renderer, &mut texture, size);
        let at = |x: usize, y: usize| px[(y * 32 + x) * 4..][..3].to_vec();
        assert_eq!(
            at(10, 10),
            at(2, 2),
            "where the parts overlap, as where one lies"
        );
        assert_eq!(at(20, 20), at(2, 2));
    }

    #[test]
    fn an_shm_buffer_takes_only_the_drawn_region() {
        let size = Size::new(4, 3);
        let picture: Vec<u8> = (0..4 * 3 * 4).map(|i| i as u8 + 1).collect();
        let stride = 4 * 4 + 8;
        let mut out = vec![0u8; stride * 3];
        let region = [Rectangle::<i32, Buffer>::new((1, 1).into(), (2, 5).into())];
        // SAFETY: `out` holds 3 rows `stride` apart, each wider than the picture's.
        unsafe {
            super::copy_rects(
                (&picture[4 * 4..], 1),
                out.as_mut_ptr(),
                stride,
                size,
                &region,
            )
        };
        for y in 0..3 {
            for x in 0..4 {
                let got = &out[y * stride + x * 4..][..4];
                let want: &[u8] = if (1..3).contains(&x) && y >= 1 {
                    &picture[(y * 4 + x) * 4..][..4]
                } else {
                    &[0; 4]
                };
                assert_eq!(got, want, "pixel ({x},{y})");
            }
        }
        assert!(
            out[16..stride].iter().all(|&b| b == 0),
            "the stride's padding is left"
        );
    }

    #[test]
    fn only_the_rows_the_drawn_region_reaches_are_read_back() {
        let size = Size::new(1280, 720);
        let r = |x, y, w, h| Rectangle::<i32, Buffer>::new((x, y).into(), (w, h).into());
        assert_eq!(super::rows_of(&[r(10, 100, 2, 16)], size), 100..116);
        assert_eq!(
            super::rows_of(&[r(10, 100, 2, 16), r(500, 40, 9, 9)], size),
            40..116
        );
        assert_eq!(
            super::rows_of(&[r(0, 700, 5, 50)], size),
            700..720,
            "cut at the bottom"
        );
        assert!(
            super::rows_of(&[], size).is_empty(),
            "nothing drawn, nothing read"
        );
    }

    #[test]
    fn a_frame_destroyed_while_drawn_leaves_the_next_whole_and_filled_at_once() {
        let mut h = Harness::new();
        h.output("TEST-1", 64, 48);
        let mut client = h.connect_capture((64, 48));
        let mut s = client.ext_session(&mut h);
        let session = h.state.captures.sessions[0].as_ref();
        super::fresh(&session).clear();
        client.ext_frame(&mut h, &mut s);
        let tracked = || super::session_damage(&session).borrow().tracked.is_some();
        let none: &[SolidColorRenderElement] = &[];
        super::session_damage(&session)
            .borrow_mut()
            .changed(hd(), CLEAR, none);

        client.ext_frame(&mut h, &mut s);
        assert!(
            tracked(),
            "a destroyed frame that was not drawn keeps the damage"
        );

        let Some(super::Waiting::Ext { frame, .. }) = h.state.captures.waiting.first() else {
            panic!("the frame waits for the output");
        };
        let drawn = (super::FrameRef::clone(frame), session.clone());
        h.state.captures.waiting.clear();
        h.state.captures.drawing.push(drawn);
        client.ext_frame(&mut h, &mut s);
        assert!(!tracked(), "the next frame is whole");
        assert!(h.state.captures.drawing.is_empty());
        assert!(
            h.state.captures.waiting.is_empty(),
            "the client never got the picture drawn, so the next frame does not wait for a change"
        );
    }

    #[test]
    fn a_copy_with_damage_into_a_buffer_without_the_latest_picture_does_not_wait() {
        let mut h = Harness::new();
        h.output("TEST-1", 64, 48);
        let mut client = h.connect_capture((64, 48));
        let _first = client.wlr_copy_with_damage(&mut h);
        let _second = client.wlr_copy_with_damage(&mut h);
        assert!(
            h.state.captures.waiting.is_empty(),
            "a buffer that never held a frame is filled at once"
        );
    }

    #[test]
    fn a_buffer_a_window_or_pointer_capture_fills_is_whole_back_in_an_output_session() {
        let mut h = Harness::new();
        h.output("TEST-1", 64, 64);
        let mut c = h.connect();
        let t = h.map_toplevel(&mut c, "test.app", 64, 64);
        let size = super::window_size(&h.state, t.id).expect("mapped");
        assert_eq!((size.w, size.h), (64, 64), "a window the output's size");
        let mut client = h.connect_capture((64, 64));
        let mut output = client.ext_session(&mut h);
        let session = h.state.captures.sessions[0].as_ref();
        let mut others = [client.window_session(&mut h), client.cursor_session(&mut h)];
        let view: View = (Size::new(64, 64), 1.0, Transform::Normal);
        for (other, what) in others.iter_mut().zip(["window", "pointer"]) {
            let buffer = client.argb_buffer();
            super::fresh(&session).clear();
            client.ext_frame_into(&mut h, &mut output, &buffer);
            let Some(super::Waiting::Ext { frame, .. }) = h.state.captures.waiting.first() else {
                panic!("the output frame waits");
            };
            let ours = frame.buffer();
            let held = || {
                let d = super::session_damage(&session).borrow();
                d.buffers.iter().any(|(b, _)| *b == ours)
            };
            {
                let mut d = super::session_damage(&session).borrow_mut();
                let none: &[SolidColorRenderElement] = &[];
                d.changed(view, CLEAR, none);
                d.take(view, &ours, &[]);
            }
            assert!(held(), "the output session's picture is in the buffer");
            client.ext_frame_into(&mut h, other, &buffer);
            assert!(!held(), "a {what} capture into it draws over that picture");
        }
    }

    #[test]
    fn a_capture_client_that_goes_leaves_none_of_its_buffers_held() {
        let mut h = Harness::new();
        h.output("TEST-1", 64, 48);
        let mut client = h.connect_capture((64, 48));
        let mut s = client.ext_session(&mut h);
        let session = h.state.captures.sessions[0].as_ref();
        super::fresh(&session).clear();
        let buffer = client.buffer();
        client.ext_frame_into(&mut h, &mut s, &buffer);
        let Some(super::Waiting::Ext { frame, .. }) = h.state.captures.waiting.first() else {
            panic!("the frame waits");
        };
        let ours = frame.buffer();
        let view: View = (Size::new(64, 48), 1.0, Transform::Normal);
        {
            let mut d = super::session_damage(&session).borrow_mut();
            let none: &[SolidColorRenderElement] = &[];
            d.changed(view, CLEAR, none);
            d.take(view, &ours, &[]);
        }
        buffer.destroy();
        client.roundtrip(&mut h);
        assert!(
            super::session_damage(&session).borrow().buffers.is_empty(),
            "a destroyed buffer is let go"
        );
        client.ext_end(&mut h, s);
        assert!(
            h.state.image_copy_capture_state.sessions().is_empty(),
            "a destroyed session is let go, with all it keeps"
        );
    }

    #[test]
    fn only_the_ext_sessions_copied_last_keep_their_glass_caches() {
        let mut h = Harness::new();
        h.output("TEST-1", 64, 48);
        let mut client = h.connect_capture((64, 48));
        let mut sessions: Vec<_> = (0..=super::CACHED_SESSIONS)
            .map(|_| client.ext_session(&mut h))
            .collect();
        for s in &mut sessions {
            client.ext_frame(&mut h, s);
        }
        let refs: Vec<super::SessionRef> = (h.state.captures.sessions.iter())
            .map(|s| s.as_ref())
            .collect();
        let cached = |h: &Harness| -> Vec<usize> {
            (h.state.captures.effects.iter())
                .map(|(s, _)| refs.iter().position(|r| r == s).expect("a session"))
                .collect()
        };
        assert_eq!(cached(&h), [1, 2, 3, 4], "0 copied least lately");
        client.ext_frame(&mut h, &mut sessions[0]);
        assert_eq!(
            cached(&h),
            [2, 3, 4, 0],
            "0 copied again, 1 the least lately"
        );
        let three = sessions.remove(3);
        client.ext_end(&mut h, three);
        assert_eq!(
            cached(&h),
            [2, 4, 0],
            "a closed session's caches go with it"
        );
    }

    /// A harness whose captures draw; None without a renderer.
    fn drawing() -> Option<Harness> {
        let Ok(renderer) = reprisefx::headless::renderer() else {
            reprisekit::pins::skipped("no renderer");
            return None;
        };
        let mut h = Harness::new();
        h.state.captures.renderer = Some(renderer);
        Some(h)
    }

    #[test]
    fn a_waiting_copy_with_damage_retried_with_nothing_new_asks_for_no_frame() {
        let Some(mut h) = drawing() else { return };
        let output = h.output("TEST-1", 64, 48);
        let mut client = h.connect_capture((64, 48));
        let buffer = client.buffer();
        let first = client.wlr_copy_with_damage_into(&mut h, &buffer);
        land(&mut h);
        client.roundtrip(&mut h);
        assert!(
            client.wlr_ready(&first).is_some(),
            "the first copy comes ready"
        );
        let _second = client.wlr_copy_with_damage_into(&mut h, &buffer);
        assert_eq!(
            h.state.captures.waiting.len(),
            1,
            "a copy into the buffer holding the latest picture waits"
        );
        crate::backend::take_asks();
        super::output_drawn(&mut h.state, &output, super::Drew::Scene);
        assert_eq!(
            h.state.captures.waiting.len(),
            1,
            "nothing changed: it waits on"
        );
        assert_eq!(
            crate::backend::take_asks(),
            [],
            "and asks for no frame, which would retry it again"
        );
    }

    /// Turns the loop until every capture drawn has reached its client.
    fn land(h: &mut Harness) {
        h.until("the captures drawn land", |s| {
            s.captures.drawing.is_empty()
                && (s.captures.wlr_streams.iter()).all(|(_, d)| d.borrow().in_flight.is_empty())
        });
    }

    #[test]
    fn a_frame_only_the_pointer_changed_fills_only_captures_that_draw_the_pointer() {
        let Some(mut h) = drawing() else { return };
        let output = h.output("TEST-1", 64, 48);
        let mut client = h.connect_capture((64, 48));
        let mut bare = client.ext_session(&mut h);
        let mut with = client.ext_session_with_pointer(&mut h);
        for s in [&mut bare, &mut with] {
            client.ext_frame(&mut h, s);
        }
        land(&mut h);
        for s in [&mut bare, &mut with] {
            client.ext_frame(&mut h, s);
        }
        assert_eq!(
            h.state.captures.waiting.len(),
            2,
            "both second frames wait for a change"
        );
        let bare_session = h.state.captures.sessions[0].as_ref();
        super::output_drawn(&mut h.state, &output, super::Drew::Pointer);
        let waiting: Vec<bool> = (h.state.captures.waiting.iter())
            .map(|w| matches!(w, super::Waiting::Ext { session, .. } if *session == bare_session))
            .collect();
        assert_eq!(
            waiting,
            [true],
            "the capture without the pointer has nothing new and waits on"
        );
        assert!(!super::fresh(&bare_session).get());
        super::output_drawn(&mut h.state, &output, super::Drew::Scene);
        assert!(
            h.state.captures.waiting.is_empty(),
            "a frame anything else asked for fills it"
        );
    }

    /// A zoomed monitor shows part of the picture, so a change outside its view draws an empty
    /// frame; a capture shows the whole picture.
    #[test]
    fn an_empty_frame_fills_the_captures_waiting_on_its_monitor_and_a_lost_one_leaves_them_owed() {
        use super::{Drew, Ended, frame_ended};
        let Some(mut h) = drawing() else { return };
        let output = h.output("TEST-1", 64, 48);
        let mut client = h.connect_capture((64, 48));
        let mut s = client.ext_session(&mut h);
        client.ext_frame(&mut h, &mut s);
        land(&mut h);
        client.ext_frame(&mut h, &mut s);
        assert_eq!(h.state.captures.waiting.len(), 1, "the next frame waits");
        let mut c = h.connect();
        h.map_toplevel(&mut c, "test.app", 20, 20);
        let owed = frame_ended(&mut h.state, &output, false, Ended::Lost(Drew::Scene));
        assert!(
            owed,
            "a lost scene frame leaves the captures owed the scene"
        );
        assert_eq!(h.state.captures.waiting.len(), 1, "the frame waits on");
        let owed = frame_ended(&mut h.state, &output, owed, Ended::Empty(Drew::Scene));
        assert!(
            h.state.captures.waiting.is_empty(),
            "an empty scene frame fills it with the window"
        );
        assert!(!owed, "and pays the scene back");
    }

    #[test]
    fn a_frame_with_nothing_new_for_its_buffer_waits_for_a_change() {
        let Some(mut h) = drawing() else { return };
        let output = h.output("TEST-1", 64, 48);
        let mut client = h.connect_capture((64, 48));
        let mut s = client.ext_session(&mut h);
        let buffer = client.buffer();
        client.ext_frame_into(&mut h, &mut s, &buffer);
        land(&mut h);
        client.ext_frame_clean_into(&mut h, &mut s, &buffer);
        super::output_drawn(&mut h.state, &output, super::Drew::Scene);
        assert_eq!(
            h.state.captures.waiting.len(),
            1,
            "the output drew nothing the capture shows, and its buffer holds the latest picture"
        );
        assert!(h.state.captures.drawing.is_empty(), "nothing is drawn");
        let mut c = h.connect();
        h.map_toplevel(&mut c, "test.app", 20, 20);
        super::output_drawn(&mut h.state, &output, super::Drew::Scene);
        assert!(
            h.state.captures.waiting.is_empty(),
            "a change it shows fills it"
        );
    }

    #[test]
    fn a_tick_only_the_pointer_asked_for_shows_a_virtual_screens_captures_nothing_new() {
        let Some(mut h) = drawing() else { return };
        let o = remote_screen(&mut h);
        let mut client = h.connect_capture((64, 48));
        let mut s = client.ext_session(&mut h);
        client.ext_frame(&mut h, &mut s);
        let session = h.state.captures.sessions[0].as_ref();
        assert!(!super::fresh(&session).get(), "the first frame is filled");
        crate::virtualout::ask(&mut h.state, &o, crate::backend::Ask::Pointer);
        h.run_for(100);
        assert!(
            !super::fresh(&session).get(),
            "the pointer is not in the capture: its next frame waits for a change"
        );
        crate::virtualout::ask(&mut h.state, &o, crate::backend::Ask::Scene);
        h.run_for(100);
        assert!(super::fresh(&session).get(), "a change fills it at once");
    }

    #[test]
    fn a_scene_edit_that_asked_no_frame_shows_on_a_virtual_screens_next_pointer_tick() {
        let Some(mut h) = drawing() else { return };
        let o = remote_screen(&mut h);
        let mut client = h.connect_capture((64, 48));
        let mut s = client.ext_session(&mut h);
        client.ext_frame(&mut h, &mut s);
        let session = h.state.captures.sessions[0].as_ref();
        assert!(!super::fresh(&session).get(), "the first frame is filled");
        let root = h.state.scene.root();
        h.state
            .scene
            .rect_create(root, (4, 4), [1.0, 0.0, 0.0, 1.0]);
        crate::virtualout::ask(&mut h.state, &o, crate::backend::Ask::Pointer);
        h.run_for(100);
        assert!(
            super::fresh(&session).get(),
            "the pointer tick shows the scene edit to captures that leave the pointer out"
        );
    }

    #[test]
    fn a_waiting_copy_is_let_go_once_its_frame_or_its_client_goes() {
        let Some(mut h) = drawing() else { return };
        h.output("TEST-1", 64, 48);
        let mut client = h.connect_capture((64, 48));
        let buffer = client.buffer();
        let first = client.wlr_copy_with_damage_into(&mut h, &buffer);
        land(&mut h);
        client.roundtrip(&mut h);
        assert!(
            client.wlr_ready(&first).is_some(),
            "the first copy comes ready"
        );
        for _ in 0..3 {
            let waits = client.wlr_copy_with_damage_into(&mut h, &buffer);
            assert_eq!(h.state.captures.waiting.len(), 1, "a copy waits");
            waits.destroy();
            client.roundtrip(&mut h);
        }
        assert!(
            h.state.captures.waiting.is_empty(),
            "a copy whose frame is destroyed stops waiting, and lets go of its buffer"
        );
        let _last = client.wlr_copy_with_damage_into(&mut h, &buffer);
        let Some(super::Waiting::Wlr(s)) = h.state.captures.waiting.first() else {
            panic!("the copy waits");
        };
        let id = s.client().expect("its client");
        h.state.drop_client(id);
        h.dispatch();
        assert!(
            h.state.captures.waiting.is_empty(),
            "nor does a copy whose client went"
        );
    }

    /// A 64x48 remote screen, its first frames over.
    fn remote_screen(h: &mut Harness) -> smithay::output::Output {
        let spec = crate::virtualout::Spec {
            size: (64, 48),
            refresh: 60_000,
            scale: 1.0,
            transform: Transform::Normal,
            position: (0, 0),
        };
        let kind = crate::virtualout::Kind::Remote;
        let o = crate::virtualout::create(&mut h.state, "REMOTE-1", spec, kind);
        h.state.update_layout();
        h.until("the wallpaper shows", |s| s.wallpaper_ready(&o));
        h.run_for(1100);
        o
    }

    #[test]
    fn captures_waiting_on_an_idle_virtual_screen_ask_for_no_tick_and_a_change_fills_them() {
        let Some(mut h) = drawing() else { return };
        let _o = remote_screen(&mut h);
        let mut client = h.connect_capture((64, 48));
        let mut ext = client.ext_session(&mut h);
        let buffer = client.buffer();
        // The first frames light the sharing dot, which the screen shows at a later tick.
        client.ext_frame(&mut h, &mut ext);
        let mut copy = client.wlr_copy_with_damage_into(&mut h, &buffer);
        for _ in 0..3 {
            h.run_for(300);
            crate::backend::take_asks();
            client.ext_frame(&mut h, &mut ext);
            copy = client.wlr_copy_with_damage_into(&mut h, &buffer);
        }
        let ext_waits = |h: &Harness| {
            (h.state.captures.waiting.iter()).any(|w| matches!(w, super::Waiting::Ext { .. }))
        };
        assert!(
            ext_waits(&h) && client.wlr_ready(&copy).is_none(),
            "on a still screen an ext frame and a wlr copy wait for a change"
        );
        assert_eq!(
            crate::backend::take_asks(),
            [],
            "and ask the virtual screen for no tick, which would show them nothing new"
        );
        h.run_for(200);
        client.roundtrip(&mut h);
        assert!(
            ext_waits(&h) && client.wlr_ready(&copy).is_none(),
            "they wait on"
        );
        let mut c = h.connect();
        h.map_toplevel(&mut c, "test.app", 20, 20);
        h.run_for(100);
        land(&mut h);
        client.roundtrip(&mut h);
        assert!(
            !ext_waits(&h) && client.wlr_ready(&copy).is_some(),
            "a window mapped fills both at the next tick"
        );
    }

    #[test]
    fn a_copy_with_damage_that_waits_asks_its_monitor_for_a_frame() {
        let Some(mut h) = drawing() else { return };
        h.output("TEST-1", 64, 48);
        let mut client = h.connect_capture((64, 48));
        let buffer = client.buffer();
        let first = client.wlr_copy_with_damage_into(&mut h, &buffer);
        land(&mut h);
        client.roundtrip(&mut h);
        assert!(
            client.wlr_ready(&first).is_some(),
            "the first copy comes ready"
        );
        // The first copy that waits borders the recording, which asks for a frame of its own.
        client.wlr_copy_with_damage_into(&mut h, &buffer).destroy();
        client.roundtrip(&mut h);
        crate::backend::take_asks();
        let _again = client.wlr_copy_with_damage_into(&mut h, &buffer);
        assert_eq!(h.state.captures.waiting.len(), 1, "a copy waits");
        assert_eq!(
            crate::backend::take_asks(),
            [("TEST-1".to_string(), crate::backend::Ask::Scene)],
            "and asks the monitor for a frame, which it draws only if anything changed"
        );
    }

    #[test]
    fn a_copy_with_damage_into_a_buffer_whose_copy_is_on_its_way_comes_whole_at_once() {
        let Some(mut h) = drawing() else { return };
        h.output("TEST-1", 64, 48);
        let mut client = h.connect_capture((64, 48));
        let buffer = client.buffer();
        let first = client.wlr_send_copy_with_damage(&buffer);
        let second = client.wlr_send_copy_with_damage(&buffer);
        client.roundtrip(&mut h);
        land(&mut h);
        client.roundtrip(&mut h);
        let whole = Some(&[(0, 0, 64, 48)][..]);
        assert_eq!(
            client.wlr_ready(&first),
            whole,
            "a buffer not seen comes whole"
        );
        assert_eq!(
            client.wlr_ready(&second),
            whole,
            "a copy asked while the first was on its way does not wait, and comes whole"
        );
    }

    #[test]
    fn a_destroyed_buffer_a_wlr_copy_filled_is_let_go() {
        let Some(mut h) = drawing() else { return };
        h.output("TEST-1", 64, 48);
        let mut client = h.connect_capture((64, 48));
        let buffer = client.buffer();
        let first = client.wlr_copy_with_damage_into(&mut h, &buffer);
        land(&mut h);
        client.roundtrip(&mut h);
        assert!(client.wlr_ready(&first).is_some(), "the copy comes ready");
        let held = |h: &Harness| -> usize {
            (h.state.captures.wlr_streams.iter())
                .map(|(_, d)| d.borrow().buffers.len())
                .sum()
        };
        assert_eq!(held(&h), 1, "the view keeps the buffer it filled");
        buffer.destroy();
        client.roundtrip(&mut h);
        assert_eq!(held(&h), 0, "a destroyed buffer is let go");
    }

    #[test]
    fn a_window_frame_destroyed_while_drawn_leaves_the_next_filled_at_once() {
        let Some(mut h) = drawing() else { return };
        h.output("TEST-1", 1024, 768);
        let mut c = h.connect();
        let t = h.map_toplevel(&mut c, "test.app", 200, 100);
        let size = super::window_size(&h.state, t.id).expect("mapped");
        let mut client = h.connect_capture((size.w, size.h));
        let mut s = client.window_session(&mut h);
        let session = h.state.captures.sessions[0].as_ref();
        client.ext_frame(&mut h, &mut s);
        assert!(
            h.state.captures.waiting.is_empty(),
            "the first frame is filled"
        );
        assert!(
            !super::fresh(&session).get(),
            "and the next waits for a change"
        );

        h.draw_self(&mut c, t, 0xff20_4060);
        let buffer = client.buffer();
        client.ext_frame_dropped(&mut h, &mut s, &buffer);
        assert!(
            h.state.captures.drawing.is_empty(),
            "the frame drawn is forgotten once destroyed"
        );
        client.ext_frame(&mut h, &mut s);
        assert!(
            h.state.captures.waiting.is_empty(),
            "the client never got the change drawn, so the next frame does not wait for another"
        );
    }

    #[test]
    fn a_frame_whose_render_fails_leaves_the_next_whole_and_filled_at_once() {
        let mut h = Harness::new();
        h.output("TEST-1", 64, 48);
        let mut client = h.connect_capture((64, 48));
        let mut s = client.ext_session(&mut h);
        let session = h.state.captures.sessions[0].as_ref();
        let none: &[SolidColorRenderElement] = &[];
        super::session_damage(&session)
            .borrow_mut()
            .changed(hd(), CLEAR, none);
        client.ext_frame(&mut h, &mut s);
        assert!(
            super::session_damage(&session).borrow().tracked.is_none(),
            "a render with no backend fails, and the next frame is whole"
        );
        assert!(super::fresh(&session).get(), "and filled at once");
    }

    #[test]
    fn a_stopped_session_lets_go_of_its_buffers_and_glass_caches() {
        let mut h = Harness::new();
        let output = h.output("TEST-1", 64, 48);
        let mut client = h.connect_capture((64, 48));
        let mut s = client.ext_session(&mut h);
        let session = h.state.captures.sessions[0].as_ref();
        client.ext_frame(&mut h, &mut s);
        assert_eq!(
            h.state.captures.effects.len(),
            1,
            "a copied session has glass caches"
        );
        super::fresh(&session).clear();
        client.ext_frame(&mut h, &mut s);
        let Some(super::Waiting::Ext { frame, .. }) = h.state.captures.waiting.first() else {
            panic!("the frame waits");
        };
        let ours = frame.buffer();
        let view: View = (Size::new(64, 48), 1.0, Transform::Normal);
        {
            let mut d = super::session_damage(&session).borrow_mut();
            let none: &[SolidColorRenderElement] = &[];
            d.changed(view, CLEAR, none);
            d.take(view, &ours, &[]);
        }
        h.state.captures.output_gone(&output);
        assert!(
            super::session_damage(&session).borrow().buffers.is_empty(),
            "a session stopped and not yet destroyed keeps no buffer"
        );
        assert!(h.state.captures.effects.is_empty(), "nor glass caches");
    }

    #[test]
    fn a_closed_pointer_session_is_let_go() {
        let mut h = Harness::new();
        h.output("TEST-1", 64, 48);
        let mut client = h.connect_capture((64, 48));
        let s = client.cursor_session(&mut h);
        assert_eq!(h.state.image_copy_capture_state.cursor_sessions().len(), 1);
        client.ext_end(&mut h, s);
        assert!(
            h.state
                .image_copy_capture_state
                .cursor_sessions()
                .is_empty(),
            "a destroyed pointer session is let go, with all it keeps"
        );
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
    fn a_window_commit_without_a_new_buffer_leaves_its_capture_frame_waiting() {
        let mut h = Harness::new();
        h.output("TEST-1", 1024, 768);
        let mut c = h.connect();
        let t = h.map_toplevel(&mut c, "test.app", 200, 100);
        let size = super::window_size(&h.state, t.id).expect("mapped");
        let mut client = h.connect_capture((size.w, size.h));
        let mut s = client.window_session(&mut h);
        let session = h.state.captures.sessions[0].as_ref();
        super::fresh(&session).clear();
        client.ext_frame(&mut h, &mut s);
        assert_eq!(h.state.captures.waiting.len(), 1, "the frame waits");

        c.side.shells[t.shell].surface.commit();
        h.roundtrip(&mut c);
        assert_eq!(
            h.state.captures.waiting.len(),
            1,
            "a bare commit leaves it waiting"
        );
        assert!(!super::fresh(&session).get(), "and the next frame not due");

        h.draw_self(&mut c, t, 0xff20_4060);
        assert!(h.state.captures.waiting.is_empty(), "a new buffer fills it");
    }

    #[test]
    fn an_output_capture_is_the_turned_output_upright_at_any_scale() {
        let mut h = Harness::new();
        let output = h.output("TEST-1", 1920, 1080);
        let mut client = h.connect_capture((1920, 1080));
        let _session = client.ext_session(&mut h);
        assert_eq!(client.buffer_sizes(), [(1920, 1080)]);
        for (transform, scale, size) in [
            (Transform::_90, 1.0, (1080, 1920)),
            (Transform::_180, 1.0, (1920, 1080)),
            (Transform::_270, 2.0, (1080, 1920)),
            (Transform::Flipped270, 1.5, (1080, 1920)),
            (Transform::Normal, 1.0, (1920, 1080)),
        ] {
            let req = crate::output::HeadRequest {
                enabled: true,
                mode: None,
                position: None,
                transform,
                scale,
                adaptive_sync: None,
            };
            assert!(h.state.apply_head(&output, &req), "{transform:?} applies");
            client.roundtrip(&mut h);
            assert_eq!(
                client.buffer_sizes().last(),
                Some(&size),
                "{transform:?} at scale {scale}: a {size:?} buffer"
            );
        }
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

    /// A wlr frame of `d` into `buffer` showing `elements`: what it reports, None while it waits.
    fn wlr(
        d: &mut SessionDamage<u32>,
        buffer: u32,
        waits: bool,
        elements: &[SolidColorRenderElement],
    ) -> Option<Rects> {
        d.changed(hd(), CLEAR, elements);
        d.wlr_frame(hd(), &buffer, waits)
    }

    #[test]
    fn two_wlr_consumers_of_one_view_both_take_a_change_on_an_idle_screen() {
        let id = Id::new();
        let caret = |x| [solid(&id, (x, 10, 2, 16))];
        let mut d = SessionDamage::<u32>::default();
        for buffer in [1, 2] {
            wlr(&mut d, buffer, true, &caret(0));
        }
        assert_eq!(wlr(&mut d, 1, true, &caret(0)), None, "1 waits while idle");
        assert_eq!(wlr(&mut d, 2, true, &caret(0)), None, "and so does 2");
        let first = wlr(&mut d, 1, true, &caret(100)).expect("the change reaches 1");
        let second = wlr(&mut d, 2, true, &caret(100)).expect("and 2");
        assert_eq!(
            area(&first),
            2 * 2 * 16,
            "the caret's two places: {first:?}"
        );
        assert_eq!(second, first, "2 measured from its own last frame");
    }

    #[test]
    fn a_wlr_copy_into_a_buffer_whose_last_copy_is_in_flight_reports_it_whole() {
        let id = Id::new();
        let caret = |x| [solid(&id, (x, 10, 2, 16))];
        let mut d = SessionDamage::<u32>::default();
        wlr(&mut d, 1, false, &caret(0));
        d.copying(&1);
        d.landed(&1, true);
        wlr(&mut d, 1, false, &caret(100));
        d.copying(&1);
        assert_eq!(
            wlr(&mut d, 1, true, &caret(200)),
            Some(whole(1280, 720)),
            "the copy before may yet fail"
        );
        d.copying(&1);
        d.landed(&1, true);
        d.landed(&1, true);
        let since = wlr(&mut d, 1, true, &caret(300)).expect("a change");
        assert_eq!(area(&since), 2 * 2 * 16, "both landed: {since:?}");
    }

    #[test]
    fn a_wlr_frame_reports_what_changed_since_its_buffer_last_held_a_frame() {
        let id = Id::new();
        let caret = |x| [solid(&id, (x, 10, 2, 16))];
        let mut d = SessionDamage::<u32>::default();
        wlr(&mut d, 1, false, &caret(0));
        wlr(&mut d, 2, false, &caret(0));
        wlr(&mut d, 1, false, &caret(100));
        let into_2 = wlr(&mut d, 2, false, &caret(200)).expect("drawn");
        for x in [0, 100, 200] {
            assert!(
                covers(&into_2, rect(x, 10, 2, 16)),
                "the frame into 1 may yet be lost, so {x} too: {into_2:?}"
            );
        }
        d.lost(&1);
        assert_eq!(
            wlr(&mut d, 1, true, &caret(200)),
            Some(whole(1280, 720)),
            "the lost frame's buffer goes whole"
        );
        assert_eq!(
            wlr(&mut d, 2, true, &caret(200)),
            None,
            "the other kept its frame"
        );
    }

    #[test]
    fn a_buffer_filled_under_another_wlr_view_is_whole_back_under_the_first() {
        let id = Id::new();
        let still = [solid(&id, (10, 10, 20, 20))];
        let (a, b) = (RefCell::default(), RefCell::default());
        let views: [&RefCell<SessionDamage<u32>>; 2] = [&a, &b];
        let copy = |view: &RefCell<SessionDamage<u32>>| {
            let reported = wlr(&mut view.borrow_mut(), 1, true, &still);
            if reported.is_some() {
                super::held_by(views, Some(view), &1);
            }
            reported
        };
        assert_eq!(copy(&a), Some(whole(1280, 720)), "a first copy");
        assert_eq!(copy(&a), None, "an unchanged view waits");
        assert_eq!(copy(&b), Some(whole(1280, 720)), "the buffer is new to b");
        assert_eq!(
            copy(&a),
            Some(whole(1280, 720)),
            "the buffer holds b's picture"
        );
    }

    #[test]
    fn a_stream_keeps_eight_buffers_and_draws_one_filled_nine_frames_ago_whole() {
        let id = Id::new();
        let still = [solid(&id, (10, 10, 20, 20))];
        let mut d = SessionDamage::<u32>::default();
        for buffer in 1..=9 {
            frame(&mut d, hd(), buffer, &still);
        }
        let (_, drawn) = frame(&mut d, hd(), 2, &still);
        assert_eq!(drawn, Rects::new(), "2 is among the last eight");
        let (_, drawn) = frame(&mut d, hd(), 1, &still);
        assert_eq!(drawn, whole(1280, 720), "1 was dropped for 9");
    }

    #[test]
    fn a_flood_of_client_damage_is_drawn_as_its_bounds() {
        let id = Id::new();
        let still = [solid(&id, (10, 10, 20, 20))];
        let mut d = SessionDamage::<u32>::default();
        frame(&mut d, hd(), 1, &still);
        d.changed(hd(), CLEAR, &still);
        let flood: Vec<Rectangle<i32, Buffer>> = (0..100_000)
            .map(|i| Rectangle::new((i % 1000, i / 1000).into(), (1, 1).into()))
            .collect();
        let drawn = d.take(hd(), &1, &flood);
        assert_eq!(drawn, vec![rect(0, 0, 1000, 100)], "their bounding box");
    }

    #[test]
    fn client_damage_past_its_buffer_is_cut_to_it() {
        let id = Id::new();
        let still = [solid(&id, (10, 10, 20, 20))];
        let mut d = SessionDamage::<u32>::default();
        frame(&mut d, hd(), 1, &still);
        d.changed(hd(), CLEAR, &still);
        let r = |x, y, w, h| Rectangle::<i32, Buffer>::new((x, y).into(), (w, h).into());
        let asked = [r(1, 0, i32::MAX, 1), r(-5, -5, 10, 10), r(1290, 5, 3, 4)];
        let drawn = d.take(hd(), &1, &asked);
        assert_eq!(drawn, vec![rect(1, 0, 1279, 1), rect(0, 0, 5, 5)]);
    }

    #[test]
    fn past_eight_wlr_views_the_one_copied_least_lately_goes() {
        let mut kept: Vec<(u32, Rc<RefCell<SessionDamage<u32>>>)> = Vec::new();
        for view in 0..8 {
            super::recent(&mut kept, view, super::WLR_VIEWS);
        }
        let zero = super::recent(&mut kept, 0, super::WLR_VIEWS);
        super::recent(&mut kept, 8, super::WLR_VIEWS);
        let views: Vec<u32> = kept.iter().map(|(v, _)| *v).collect();
        assert_eq!(
            views,
            [2, 3, 4, 5, 6, 7, 0, 8],
            "1 went, 0 was copied again"
        );
        assert!(Rc::ptr_eq(&zero, &kept[6].1), "0 kept its damage");
    }

    /// The bounds of the pixels of a `w`x`h` RGBA picture that are red.
    fn red_bounds(px: &[u8], (w, h): (i32, i32)) -> Rectangle<i32, Buffer> {
        let mut bounds: Option<Rectangle<i32, Buffer>> = None;
        for y in 0..h {
            for x in 0..w {
                if px[((y * w + x) * 4) as usize..][..3] == [255, 0, 0] {
                    let p = Rectangle::new((x, y).into(), (1, 1).into());
                    bounds = Some(bounds.map_or(p, |b| b.merge(p)));
                }
            }
        }
        bounds.expect("red pixels")
    }

    #[test]
    fn output_pixels_land_in_the_buffer_where_to_buffer_puts_them_under_every_transform() {
        use smithay::backend::renderer::damage::OutputDamageTracker;
        let Ok(mut renderer) = reprisefx::headless::renderer() else {
            reprisekit::pins::skipped("no renderer");
            return;
        };
        let id = Id::new();
        let r = rect(5, 3, 7, 4);
        let elements = [OutputElement::Border(solid(&id, (5, 3, 7, 4)))];
        for transform in [
            Transform::Normal,
            Transform::_90,
            Transform::_180,
            Transform::_270,
            Transform::Flipped,
            Transform::Flipped90,
            Transform::Flipped180,
            Transform::Flipped270,
        ] {
            let view: View = (Size::new(64, 48), 1.0, transform);
            let mut texture = filled(&mut renderer, (64, 48), CLEAR);
            let mut fb = renderer.bind(&mut texture).expect("bound");
            OutputDamageTracker::new(view.0, 1.0, transform)
                .render_output(&mut renderer, &mut fb, 0, &elements, CLEAR)
                .expect("drawn whole");
            drop(fb);
            let px = pixels(&mut renderer, &mut texture, (64, 48));
            let found = red_bounds(&px, (64, 48));
            assert_eq!(
                super::to_buffer(view, r),
                found,
                "{transform:?}, drawn whole"
            );
            assert_eq!(super::from_buffer(view, found), r, "{transform:?} back");

            let mut texture = filled(&mut renderer, (64, 48), CLEAR);
            let mut fb = renderer.bind(&mut texture).expect("bound");
            let mut effects = Default::default();
            let drawing = (&elements[..], CLEAR);
            let _ = super::paint(&mut renderer, &mut fb, view, drawing, &[r], &mut effects)
                .expect("drawn in its region");
            drop(fb);
            let px = pixels(&mut renderer, &mut texture, (64, 48));
            let found = red_bounds(&px, (64, 48));
            assert_eq!(
                super::to_buffer(view, r),
                found,
                "{transform:?}, in a region"
            );
        }
    }

    smithay::render_elements! {
        Glazed<=smithay::backend::renderer::gles::GlesRenderer>;
        Glass=reprisefx::GlassElement,
        Solid=SolidColorRenderElement,
    }

    #[test]
    fn glass_in_a_partial_capture_draws_as_a_whole_capture_does() {
        glass_drawn_in_part_as_whole(true);
    }

    #[test]
    fn a_session_past_the_glass_cache_cap_draws_glass_in_part_as_a_whole_capture_does() {
        glass_drawn_in_part_as_whole(false);
    }

    /// Three frames of glass over a moving patch, each drawn only where the buffer needs it, end
    /// as the last drawn whole. The glass caches are `kept` from frame to frame, as for an ext
    /// session among those copied last, or new each frame, as for one past them.
    fn glass_drawn_in_part_as_whole(kept: bool) {
        let Ok(mut renderer) = reprisefx::headless::renderer() else {
            reprisekit::pins::skipped("no renderer");
            return;
        };
        let fx = reprisefx::GlassFx::new(&mut renderer).expect("the glass shaders compile");
        let size = (320, 200);
        let view: View = (Size::new(320, 200), 1.0, Transform::Normal);
        let settings = reprisekit::glass::Settings {
            material: reprisekit::glass::Material::Frosted,
            streaks: false,
            ..Default::default()
        };
        let shapes = [reprisefx::Shape::rect(rect(120, 70, 80, 60))];
        let glass = reprisefx::Glass::new();
        let (patch, back) = (Id::new(), Id::new());
        let elements = |at: (i32, i32)| -> Vec<Glazed> {
            let f = reprisefx::GlassFrame {
                shapes: &shapes,
                holes: &[],
                settings: &settings,
                surface: None,
                output: view.0,
                screen: 0,
                scale: 1.0,
                alpha: 1.0,
                quality: reprisefx::Quality::Full,
                hold: false,
                source: None,
                downscale: 1,
            };
            let solid = |id: &Id, r, colour| {
                SolidColorRenderElement::new(id.clone(), r, 0, colour, Kind::Unspecified)
            };
            vec![
                Glazed::Glass(glass.element(&fx, &f).expect("glass")),
                Glazed::Solid(solid(
                    &patch,
                    rect(at.0, at.1, 12, 12),
                    [0.0, 0.0, 0.0, 1.0],
                )),
                Glazed::Solid(solid(&back, rect(0, 0, 320, 200), [1.0, 1.0, 1.0, 1.0])),
            ]
        };
        let mut d = SessionDamage::<u32>::default();
        let mut effects = crate::glass::Effects::default();
        let mut target = filled(&mut renderer, size, CLEAR);
        let scribble = rect(166, 108, 4, 4);
        for (at, asked) in [
            ((140, 90), None),
            ((170, 105), None),
            ((170, 105), Some(scribble)),
        ] {
            let shown = elements(at);
            d.changed(view, CLEAR, &shown);
            let asked: Vec<Rectangle<i32, Buffer>> = asked
                .into_iter()
                .map(|r| {
                    use smithay::backend::renderer::{Frame, Renderer};
                    let mut fb = renderer.bind(&mut target).expect("bound");
                    let mut frame = renderer
                        .render(&mut fb, view.0, Transform::Normal)
                        .expect("a frame");
                    frame
                        .clear(Color32F::new(1.0, 0.0, 0.0, 1.0), &[r])
                        .expect("scribbled");
                    let _ = frame.finish().expect("finished");
                    Rectangle::new((r.loc.x, r.loc.y).into(), (r.size.w, r.size.h).into())
                })
                .collect();
            let mut region = d.take(view, &1, &asked);
            super::grow_over_effects(&mut region, &shown, 1.0);
            let geometry = shown[0].geometry(Scale::from(1.0));
            assert!(covers(&region, geometry), "{region:?} over the glass");
            if at != (140, 90) {
                assert!(area(&region) < 320 * 200, "{region:?} is part of the frame");
            }
            if !kept {
                effects = Default::default();
            }
            let mut fb = renderer.bind(&mut target).expect("bound");
            let drawing = (&shown[..], CLEAR);
            let _ = super::paint(&mut renderer, &mut fb, view, drawing, &region, &mut effects)
                .expect("drawn");
            drop(fb);
        }
        let partial = pixels(&mut renderer, &mut target, size);
        let shown = elements((170, 105));
        let mut fresh = filled(&mut renderer, size, CLEAR);
        let mut fb = renderer.bind(&mut fresh).expect("bound");
        smithay::backend::renderer::damage::OutputDamageTracker::new(view.0, 1.0, view.2)
            .render_output(&mut renderer, &mut fb, 0, &shown, CLEAR)
            .expect("drawn whole as the screen is");
        drop(fb);
        let fresh = pixels(&mut renderer, &mut fresh, size);
        let differ = partial
            .chunks(4)
            .zip(fresh.chunks(4))
            .filter(|(a, b)| a != b);
        assert_eq!(differ.count(), 0, "pixels unlike a whole capture's");
    }
}
