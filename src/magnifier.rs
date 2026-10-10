//! The magnifier and the colour filter: an output's picture zoomed 1x to 8x in half steps,
//! drawn through `[accessibility] color_filter`.
//!
//! The pointer stays in layout space, so hit testing, focus and drags do not know about the
//! zoom. Relative motion on a zoomed output is divided by the level, and the cursor is drawn at
//! normal size at the pointer's layout point (`Zoom::on_screen`).
//!
//! The view push-pans with the pointer; under `magnifier_follow = "pointer+focus"` it also pans
//! smoothly to the caret or focus (`State::magnifier_focus`), and the pointer wins whenever it
//! moves (`track`). While zoomed that way the compositor enters the focused client's text input
//! (`sync_caret_reports`) so clients report their caret without an input method.
//!
//! A zoomed or filtered output renders its scene, without the pointer, into an output-sized
//! texture (`offscreen::Target`) with its own damage tracker, then shows the view's part of it
//! stretched across the output through the filter (`reprisefx::ColorElement`), with the
//! unfiltered pointer over it. The capture protocols and Print Screen build their own elements
//! and stay unzoomed and unfiltered. At 1x with no filter no texture exists.

use std::time::{Duration, Instant};

use reprisekit::colorfilter::{Filter, Matrix};
use smithay::{
    backend::renderer::{
        Color32F, Renderer, Texture,
        element::{Id, Kind, texture::TextureRenderElement},
        gles::{GlesRenderer, GlesTexture},
        utils::DamageBag,
    },
    output::Output,
    utils::{Buffer, Logical, Physical, Point, Rectangle, Size, Transform},
};

use crate::compositor::State;
use crate::offscreen::{Target, physical_size};
use crate::raster::Rect;
use crate::render::{OutputElement, Parts};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZoomStep {
    In,
    Out,
    Reset,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Follow {
    Pointer,
    #[default]
    PointerAndFocus,
}

impl Follow {
    pub fn name(self) -> &'static str {
        match self {
            Follow::Pointer => "pointer",
            Follow::PointerAndFocus => "pointer+focus",
        }
    }

    pub fn parse(name: &str) -> Option<Follow> {
        [Follow::Pointer, Follow::PointerAndFocus]
            .into_iter()
            .find(|f| f.name() == name)
    }

    pub fn focus(self) -> bool {
        self == Follow::PointerAndFocus
    }
}

/// Levels in half steps: 2 is 1x, 16 is 8x.
const MIN_HALVES: u32 = 2;
const MAX_HALVES: u32 = 16;

/// Frames of the zoomed texture's damage kept; an older buffer is drawn whole.
const DAMAGE_KEPT: usize = 8;

const PAN: Duration = Duration::from_millis(180);

/// After the zoom turns on, how long a newly appearing caret or focus (the text input entered
/// for it) counts as where things are, not as a move to follow.
const SETTLE: Duration = Duration::from_millis(500);

pub type Area = (f64, f64, f64, f64);

/// One output's zoom: the level and the view's top-left in output-local logical pixels. The
/// view is the output's size divided by the level.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Zoom {
    halves: u32,
    pub x: f64,
    pub y: f64,
}

impl Default for Zoom {
    fn default() -> Zoom {
        Zoom::OFF
    }
}

impl Zoom {
    pub const OFF: Zoom = Zoom {
        halves: MIN_HALVES,
        x: 0.0,
        y: 0.0,
    };

    pub fn level(&self) -> f64 {
        f64::from(self.halves) / 2.0
    }

    pub fn active(&self) -> bool {
        self.halves > MIN_HALVES
    }

    pub fn view_size(&self, size: (f64, f64)) -> (f64, f64) {
        (size.0 / self.level(), size.1 / self.level())
    }

    pub fn on_screen(self, p: (f64, f64)) -> (f64, f64) {
        ((p.0 - self.x) * self.level(), (p.1 - self.y) * self.level())
    }

    pub fn at_screen(self, s: (f64, f64)) -> (f64, f64) {
        (self.x + s.0 / self.level(), self.y + s.1 / self.level())
    }

    /// The zoom after `step`, the pointer's point `p` staying where the screen shows it.
    pub fn step(&self, step: ZoomStep, p: (f64, f64), size: (f64, f64)) -> Zoom {
        let halves = match step {
            ZoomStep::In => (self.halves + 1).min(MAX_HALVES),
            ZoomStep::Out => (self.halves - 1).max(MIN_HALVES),
            ZoomStep::Reset => MIN_HALVES,
        };
        let s = self.on_screen(p);
        let s = (s.0.clamp(0.0, size.0), s.1.clamp(0.0, size.1));
        let level = f64::from(halves) / 2.0;
        Zoom {
            halves,
            x: p.0 - s.0 / level,
            y: p.1 - s.1 / level,
        }
        .clamped(size)
    }

    /// Push panning: the view moves just far enough to hold `p`. Its last screen pixel is the
    /// far edge, so the cursor pushing right or down still shows.
    pub fn follow(&self, p: (f64, f64), size: (f64, f64)) -> Zoom {
        let (w, h) = self.view_size(size);
        let (w, h) = (w - 1.0 / self.level(), h - 1.0 / self.level());
        let pan = |v: f64, p: f64, len: f64| {
            if p < v {
                p
            } else if p > v + len {
                p - len
            } else {
                v
            }
        };
        Zoom {
            halves: self.halves,
            x: pan(self.x, p.0, w),
            y: pan(self.y, p.1, h),
        }
        .clamped(size)
    }

    /// The view moved just far enough to hold `r` an eighth of the view in from the edges.
    /// Along an axis where `r` is longer than that room, a view within `r` stays, else it lines
    /// up with `r`'s start.
    pub fn hold(&self, r: Area, size: (f64, f64)) -> Zoom {
        let (w, h) = self.view_size(size);
        let axis = |v: f64, len: f64, a: f64, alen: f64| {
            let margin = len / 8.0;
            if alen + 2.0 * margin <= len {
                if a - margin < v {
                    a - margin
                } else if a + alen + margin > v + len {
                    a + alen + margin - len
                } else {
                    v
                }
            } else if v >= a && v + len <= a + alen {
                v
            } else {
                a
            }
        };
        Zoom {
            halves: self.halves,
            x: axis(self.x, w, r.0, r.2),
            y: axis(self.y, h, r.1, r.3),
        }
        .clamped(size)
    }

    fn clamped(self, size: (f64, f64)) -> Zoom {
        let (w, h) = self.view_size(size);
        Zoom {
            halves: self.halves,
            x: self.x.clamp(0.0, (size.0 - w).max(0.0)),
            y: self.y.clamp(0.0, (size.1 - h).max(0.0)),
        }
    }

    /// The view as a source rectangle in a texture of the output at `scale`.
    fn source(&self, size: (f64, f64), scale: f64) -> Rectangle<f64, Logical> {
        let (w, h) = self.view_size(size);
        Rectangle::new(
            (self.x * scale, self.y * scale).into(),
            (w * scale, h * scale).into(),
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Pan {
    from: (f64, f64),
    to: (f64, f64),
    started: Instant,
}

impl Pan {
    fn at(&self, now: Instant) -> ((f64, f64), bool) {
        let t = now.saturating_duration_since(self.started).as_secs_f64() / PAN.as_secs_f64();
        if t >= 1.0 {
            return (self.to, true);
        }
        let e = 1.0 - (1.0 - t).powi(3);
        let mix = |a: f64, b: f64| a + (b - a) * e;
        (
            (mix(self.from.0, self.to.0), mix(self.from.1, self.to.1)),
            false,
        )
    }
}

/// What a zoomed view last saw of the pointer and focus, and the pan under way.
#[derive(Clone, Copy, Debug)]
struct Track {
    pointer: Option<(f64, f64)>,
    focus: Option<Area>,
    pan: Option<Pan>,
    since: Instant,
}

impl Track {
    fn new(now: Instant) -> Track {
        Track {
            pointer: None,
            focus: None,
            pan: None,
            since: now,
        }
    }
}

/// One frame's view at `now`: a moved `pointer` (None off the output) pushes the view and stops
/// any pan; otherwise a moved `focus` starts a pan. A focus counts once it moves, or when it
/// shows up after the zoom settled (`SETTLE`).
fn track(
    zoom: Zoom,
    t: &mut Track,
    pointer: Option<(f64, f64)>,
    focus: Option<Area>,
    size: (f64, f64),
    now: Instant,
) -> Zoom {
    let moved = pointer.is_some() && pointer != t.pointer;
    t.pointer = pointer;
    let settled = now.saturating_duration_since(t.since) >= SETTLE;
    let refocused = focus.is_some() && focus != t.focus && (t.focus.is_some() || settled);
    t.focus = focus;
    let mut zoom = zoom;
    if let (true, Some(p)) = (moved, pointer) {
        t.pan = None;
        return zoom.follow(p, size);
    }
    if let (true, Some(f)) = (refocused, focus) {
        // Held from where a pan under way ends, so typing faster than a pan keeps it going.
        let (x, y) = t.pan.map_or((zoom.x, zoom.y), |p| p.to);
        let want = Zoom { x, y, ..zoom }.hold(f, size);
        if (want.x, want.y) != (x, y) {
            t.pan = Some(Pan {
                from: (zoom.x, zoom.y),
                to: (want.x, want.y),
                started: now,
            });
        }
    }
    if let Some(pan) = t.pan {
        let ((x, y), done) = pan.at(now);
        (zoom.x, zoom.y) = (x, y);
        if done {
            t.pan = None;
        }
    }
    zoom
}

/// Every output's zoom and frame texture, and the filter program.
#[derive(Default)]
pub struct Magnifier {
    outputs: Vec<OutputZoom>,
    /// The colour map program, compiled on the first filtered frame; Some(None) if it failed.
    color: Option<Option<reprisefx::ColorFx>>,
    /// The filter `color-filter` turns back on.
    last_filter: Option<Filter>,
}

struct OutputZoom {
    output: Output,
    zoom: Zoom,
    track: Track,
    frame: Option<Frame>,
}

struct Frame {
    target: Target,
    /// The screen element, kept across frames for its damage; a new filter takes a new one.
    id: Id,
    size: Size<i32, Physical>,
    scale: f64,
    clear: Color32F,
    damage: DamageBag<i32, Buffer>,
    /// The view the screen showed last; another one is drawn whole.
    shown: Option<Zoom>,
    filter: Option<Matrix>,
}

impl Magnifier {
    fn zoom_of(&self, output: &Output) -> Zoom {
        self.outputs
            .iter()
            .find(|z| z.output == *output)
            .map_or(Zoom::OFF, |z| z.zoom)
    }

    pub fn zooms(&self, output: &Output) -> bool {
        self.zoom_of(output).active()
    }

    pub fn any_zoomed(&self) -> bool {
        self.outputs.iter().any(|z| z.zoom.active())
    }

    pub fn panning(&self, output: &Output) -> bool {
        self.outputs
            .iter()
            .any(|z| z.output == *output && z.track.pan.is_some())
    }

    pub fn retain(&mut self, outputs: &[Output]) {
        self.outputs.retain(|z| outputs.contains(&z.output));
    }

    fn color_fx(&mut self, renderer: &mut GlesRenderer) -> Option<reprisefx::ColorFx> {
        self.color
            .get_or_insert_with(|| {
                let fx = reprisefx::ColorFx::new(renderer)
                    .map_err(|e| log::warn!("color filter: the shader did not compile: {e}"))
                    .ok();
                if fx.is_some() {
                    log::info!("color filter: shader compiled");
                }
                fx
            })
            .clone()
    }
}

fn geometry(output: &Output) -> Option<(Rectangle<i32, Logical>, (f64, f64))> {
    let geo = reprisescene::output_rect(output)?;
    Some((geo, (f64::from(geo.size.w), f64::from(geo.size.h))))
}

pub fn filter_matrix(a: &crate::config::AccessibilityConfig) -> Option<Matrix> {
    reprisekit::colorfilter::matrix(a.color_filter, f64::from(a.color_filter_intensity) / 100.0)
}

impl State {
    pub fn zoom(&mut self, step: ZoomStep) {
        let pointer = self.pointer_location();
        let Some(output) = self
            .scene
            .outputs()
            .iter()
            .find(|o| reprisescene::output_rect(o).is_some_and(|r| r.to_f64().contains(pointer)))
            .cloned()
        else {
            return;
        };
        let Some((geo, size)) = geometry(&output) else {
            return;
        };
        let m = &mut self.magnifier;
        m.retain(self.scene.outputs());
        let p = (
            pointer.x - f64::from(geo.loc.x),
            pointer.y - f64::from(geo.loc.y),
        );
        let old = m.zoom_of(&output);
        let zoom = old.step(step, p, size);
        if zoom == old {
            return;
        }
        let now = Instant::now();
        match m.outputs.iter_mut().find(|z| z.output == output) {
            Some(z) => {
                if !old.active() {
                    z.track = Track::new(now);
                }
                z.zoom = zoom;
                z.track.pan = None;
            }
            None => m.outputs.push(OutputZoom {
                output: output.clone(),
                zoom,
                track: Track::new(now),
                frame: None,
            }),
        }
        log::info!("magnifier: {}x", zoom.level());
        self.sync_caret_reports();
        crate::backend::schedule(self, &output);
    }

    /// Enter the focused client's text input as an input method would while zoomed and
    /// following the caret, so it reports its caret; leave it otherwise, as entering takes a
    /// client's own compose handling away in some toolkits.
    pub fn sync_caret_reports(&mut self) {
        use smithay::wayland::text_input::TextInputSeat;
        let want =
            self.config.accessibility.magnifier_follow.focus() && self.magnifier.any_zoomed();
        let ti = self.seat.text_input();
        if ti.compositor_input_method() != want {
            log::info!("magnifier: text inputs report their caret: {want}");
            ti.set_compositor_input_method(want);
        }
    }

    /// Turn the colour filter off, or back on to the last one (Grayscale when none); live and
    /// in config.toml.
    pub fn toggle_color_filter(&mut self) {
        let a = &mut self.config.accessibility;
        let next = if a.color_filter == Filter::None {
            *self.magnifier.last_filter.get_or_insert_with(|| {
                crate::state::load()
                    .color_filter_last
                    .as_deref()
                    .and_then(Filter::parse)
                    .filter(|f| *f != Filter::None)
                    .unwrap_or(Filter::Grayscale)
            })
        } else {
            self.magnifier.last_filter = Some(a.color_filter);
            crate::state::save_color_filter_last(a.color_filter.name());
            Filter::None
        };
        a.color_filter = next;
        log::info!("color filter: {}", next.name());
        let key = repriseconf::key("accessibility", "color_filter");
        crate::configwrite::save_value(&self.config_path, key, next.name().into());
        self.redraw_all();
    }

    /// A relative motion of `delta` as layout motion: divided by the level of a zoomed output
    /// under the pointer.
    pub fn zoom_motion(&self, delta: Point<f64, Logical>) -> Point<f64, Logical> {
        let pointer = self.pointer_location();
        match self.zoomed_at(pointer) {
            Some((_, zoom)) => (delta.x / zoom.level(), delta.y / zoom.level()).into(),
            None => delta,
        }
    }

    /// An absolute device's point `pos` as a screen point: on a zoomed output, the layout point
    /// the zoom shows there.
    pub fn zoom_absolute(&self, pos: Point<f64, Logical>) -> Point<f64, Logical> {
        match self.zoomed_at(pos) {
            Some((geo, zoom)) => {
                let origin = geo.loc.to_f64();
                let p = zoom.at_screen((pos.x - origin.x, pos.y - origin.y));
                origin + Point::from(p)
            }
            None => pos,
        }
    }

    fn zoomed_at(&self, pos: Point<f64, Logical>) -> Option<(Rectangle<i32, Logical>, Zoom)> {
        self.magnifier.outputs.iter().find_map(|z| {
            let geo = reprisescene::output_rect(&z.output)?;
            (z.zoom.active() && geo.to_f64().contains(pos)).then_some((geo, z.zoom))
        })
    }

    /// What a zoomed `output`'s view holds when the keyboard moves it, output-local: the
    /// focused control of Reprise's own surface, else the client's text caret, else the focused
    /// window. None unless following focus, or when it is on another output.
    pub fn magnifier_focus(&self, output: &Output) -> Option<Area> {
        if !self.config.accessibility.magnifier_follow.focus() || !self.magnifier.zooms(output) {
            return None;
        }
        let r = crate::access::focus_rect(self)
            .or_else(|| self.text_caret())
            .or_else(|| {
                let w = self.win(self.focused?)?;
                Some(w.frame_rect())
            })?;
        let geo = reprisescene::output_rect(output)?;
        let on = Rect::new(geo.loc.x, geo.loc.y, geo.size.w, geo.size.h);
        // A caret may be 0 px wide: its middle says which output it is on.
        if !on.contains(r.x + r.width / 2, r.y + r.height / 2) {
            return None;
        }
        Some((
            f64::from(r.x - on.x),
            f64::from(r.y - on.y),
            f64::from(r.width),
            f64::from(r.height),
        ))
    }
}

/// `output`'s elements for its next frame: `render::output_elements` at 1x with no `filter`;
/// otherwise the pointer over the view's part of the unzoomed picture drawn through `filter`
/// (`clear` where nothing covers it). A zoomed view first follows the pointer and `focus`
/// (`track`).
pub fn output_elements(
    magnifier: &mut Magnifier,
    renderer: &mut GlesRenderer,
    parts: Parts<'_>,
    output: &Output,
    clear: Color32F,
    filter: Option<Matrix>,
    focus: Option<Area>,
) -> Vec<OutputElement> {
    let at = magnifier.outputs.iter().position(|z| z.output == *output);
    let zoomed = at.is_some_and(|i| magnifier.outputs[i].zoom.active());
    let geometry = geometry(output);
    let (Some((geo, size)), true) = (geometry, zoomed || filter.is_some()) else {
        if let Some(i) = at {
            magnifier.outputs.remove(i);
        }
        return crate::render::output_elements(renderer, parts, output, (0, 0).into(), true);
    };
    let fx = match filter {
        Some(_) => magnifier.color_fx(renderer),
        None => None,
    };
    let filter = filter.filter(|_| fx.is_some());
    let i = at.unwrap_or_else(|| {
        magnifier.outputs.push(OutputZoom {
            output: output.clone(),
            zoom: Zoom::OFF,
            track: Track::new(Instant::now()),
            frame: None,
        });
        magnifier.outputs.len() - 1
    });
    let z = &mut magnifier.outputs[i];
    let Parts {
        scene,
        cursor,
        cursors,
        pointer,
    } = parts;
    let p = (
        pointer.x - f64::from(geo.loc.x),
        pointer.y - f64::from(geo.loc.y),
    );
    let on_output = geo.to_f64().contains(pointer);
    if zoomed {
        let seen = on_output.then_some(p);
        let now = Instant::now();
        z.zoom = track(z.zoom, &mut z.track, seen, focus, size, now);
        if let (Some(pan), Some((x, y, w, h))) = (z.track.pan, focus)
            && pan.started == now
        {
            log::debug!(
                "magnifier: panning to hold {x:.0},{y:.0} {w:.0}x{h:.0}, view from {:.0},{:.0}",
                pan.to.0,
                pan.to.1
            );
        }
    }
    let scale = output.current_scale().fractional_scale();
    let px = physical_size(geo.size, scale);
    let frame = match z.frame.take() {
        Some(f) if f.size == px && f.scale == scale && f.clear == clear => f,
        _ => match Target::new(renderer, px, scale) {
            Ok(target) => Frame {
                target: target.clear_to(clear),
                id: Id::new(),
                size: px,
                scale,
                clear,
                damage: DamageBag::new(DAMAGE_KEPT),
                shown: None,
                filter: None,
            },
            Err(e) => {
                log::warn!("magnifier: {e}");
                z.zoom = Zoom::OFF;
                let parts = Parts {
                    scene,
                    cursor,
                    cursors,
                    pointer,
                };
                return crate::render::output_elements(
                    renderer,
                    parts,
                    output,
                    (0, 0).into(),
                    true,
                );
            }
        },
    };
    let frame = z.frame.insert(frame);
    let unzoomed = crate::render::output_elements(
        renderer,
        Parts {
            scene,
            cursor,
            cursors,
            pointer,
        },
        output,
        (0, 0).into(),
        false,
    );
    match frame.target.render(renderer, &unzoomed) {
        Ok(damage) => frame.damage.add(damage),
        Err(e) => log::warn!("magnifier: {e}"),
    }
    drop(unzoomed);
    if frame.shown != Some(z.zoom) {
        frame.shown = Some(z.zoom);
        frame
            .damage
            .add([Rectangle::from_size((px.w, px.h).into())]);
    }
    if frame.filter != filter {
        // Another map: a new element, drawn whole.
        frame.filter = filter;
        frame.id = Id::new();
    }
    let mut out = Vec::new();
    if on_output {
        let at = Point::from(z.zoom.on_screen(p));
        out.extend(crate::render::pointer_elements(
            renderer,
            Parts {
                scene,
                cursor,
                cursors,
                pointer,
            },
            output,
            at,
        ));
    }
    let screen = screen_element(
        renderer,
        frame.target.texture(),
        frame.id.clone(),
        &frame.damage,
        z.zoom.source(size, scale),
        geo.size,
    );
    out.push(match (filter, fx) {
        (Some(m), Some(fx)) => {
            OutputElement::Filtered(fx.element(screen, frame.target.texture().clone(), &m))
        }
        _ => OutputElement::Zoom(screen),
    });
    out
}

/// The texture's `source` part stretched across an output `size` logical pixels large, opaque,
/// with the texture's `damage`.
fn screen_element(
    renderer: &GlesRenderer,
    texture: &GlesTexture,
    id: Id,
    damage: &DamageBag<i32, Buffer>,
    source: Rectangle<f64, Logical>,
    size: Size<i32, Logical>,
) -> TextureRenderElement<GlesTexture> {
    let whole = Rectangle::from_size(texture.size());
    TextureRenderElement::from_texture_with_damage(
        id,
        renderer.context_id(),
        (0.0, 0.0),
        texture.clone(),
        1,
        Transform::Normal,
        None,
        Some(source),
        Some(size),
        Some(vec![whole]),
        damage.snapshot(),
        Kind::Unspecified,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: (f64, f64) = (1920.0, 1080.0);

    fn at(halves: u32, x: f64, y: f64) -> Zoom {
        Zoom { halves, x, y }
    }

    #[test]
    fn steps_by_half_from_one_to_eight() {
        let mut z = Zoom::OFF;
        let mut levels = Vec::new();
        for _ in 0..16 {
            z = z.step(ZoomStep::In, (0.0, 0.0), SIZE);
            levels.push(z.level());
        }
        assert_eq!(levels[..3], [1.5, 2.0, 2.5]);
        assert_eq!(z.level(), 8.0, "the level stops at 8x");
        for _ in 0..20 {
            z = z.step(ZoomStep::Out, (0.0, 0.0), SIZE);
        }
        assert_eq!(z, Zoom::OFF, "the level stops at 1x");
        assert!(!z.active());
    }

    #[test]
    fn reset_goes_back_to_one() {
        let z = at(12, 400.0, 300.0).step(ZoomStep::Reset, (500.0, 400.0), SIZE);
        assert_eq!(z, Zoom::OFF);
    }

    #[test]
    fn stepping_keeps_the_pointers_point_on_the_same_screen_spot() {
        let p = (600.0, 300.0);
        let z = Zoom::OFF
            .step(ZoomStep::In, p, SIZE)
            .step(ZoomStep::In, p, SIZE);
        assert_eq!(z.level(), 2.0);
        assert_eq!(
            z.on_screen(p),
            p,
            "at 2x the pointer's point stays at 600,300"
        );
        assert_eq!((z.x, z.y), (300.0, 150.0));
    }

    #[test]
    fn stepping_near_an_edge_keeps_the_view_inside() {
        let z = Zoom::OFF.step(ZoomStep::In, (1919.0, 1079.0), SIZE);
        let (w, h) = z.view_size(SIZE);
        assert!(z.x + w <= 1920.0 && z.y + h <= 1080.0);
        assert!(z.x >= 0.0 && z.y >= 0.0);
    }

    #[test]
    fn the_view_stays_put_while_the_pointer_is_inside_it() {
        let z = at(4, 400.0, 200.0);
        assert_eq!(
            z.follow((900.0, 500.0), SIZE),
            z,
            "the 960x540 view holds 900,500"
        );
    }

    #[test]
    fn the_pointer_pushes_the_view_at_its_edge() {
        let z = at(4, 400.0, 200.0);
        let right = z.follow((1400.0, 300.0), SIZE);
        assert_eq!(
            (right.x, right.y),
            (440.5, 200.0),
            "x 1400 shows on the view's last screen column"
        );
        assert_eq!(right.on_screen((1400.0, 300.0)).0, 1919.0);
        let up = z.follow((500.0, 150.0), SIZE);
        assert_eq!((up.x, up.y), (400.0, 150.0), "the top edge follows y 150");
    }

    #[test]
    fn panning_stops_at_the_outputs_edge() {
        let z = at(16, 1600.0, 900.0).follow((2000.0, 1200.0), SIZE);
        assert_eq!((z.x, z.y), (1680.0, 945.0), "an 8x view ends at 1920,1080");
        let z = at(16, 100.0, 100.0).follow((-50.0, -50.0), SIZE);
        assert_eq!((z.x, z.y), (0.0, 0.0), "and starts at 0,0");
    }

    #[test]
    fn the_screen_and_the_view_map_both_ways() {
        let z = at(6, 100.0, 50.0);
        assert_eq!(
            z.on_screen((110.0, 60.0)),
            (30.0, 30.0),
            "3x: 10 px in the view is 30 on screen"
        );
        assert_eq!(z.at_screen((30.0, 30.0)), (110.0, 60.0));
    }

    #[test]
    fn follow_names_round_trip() {
        for f in [Follow::Pointer, Follow::PointerAndFocus] {
            assert_eq!(Follow::parse(f.name()), Some(f));
        }
        assert_eq!(Follow::parse("caret"), None);
        assert!(!Follow::Pointer.focus() && Follow::PointerAndFocus.focus());
    }

    /// A 4x view of 480x270 at 400,200: margin 60 across and 33.75 down.
    #[test]
    fn holding_a_caret_moves_the_view_just_enough() {
        let z = at(8, 400.0, 200.0);
        assert_eq!(z.hold((600.0, 300.0, 2.0, 16.0), SIZE), z, "inside: no pan");
        let right = z.hold((900.0, 300.0, 2.0, 16.0), SIZE);
        assert_eq!(
            (right.x, right.y),
            (482.0, 200.0),
            "the caret 60 in from the right"
        );
        let below = z.hold((600.0, 500.0, 2.0, 16.0), SIZE);
        assert_eq!((below.x, below.y), (400.0, 279.75));
        let left = z.hold((100.0, 300.0, 2.0, 16.0), SIZE);
        assert_eq!(left.x, 40.0, "60 in from the left");
        let corner = z.hold((10.0, 10.0, 2.0, 16.0), SIZE);
        assert_eq!((corner.x, corner.y), (0.0, 0.0), "kept inside the output");
    }

    #[test]
    fn holding_a_big_window_shows_its_corner_unless_the_view_lies_in_it() {
        let z = at(8, 400.0, 200.0);
        let window = (1000.0, 600.0, 800.0, 400.0);
        let to = z.hold(window, SIZE);
        assert_eq!((to.x, to.y), (1000.0, 600.0), "its top-left corner");
        let inside = at(8, 1200.0, 700.0);
        assert_eq!(
            inside.hold(window, SIZE),
            inside,
            "already showing part of it"
        );
    }

    #[test]
    fn a_pan_eases_out_and_arrives() {
        let t0 = Instant::now();
        let pan = Pan {
            from: (0.0, 0.0),
            to: (100.0, 50.0),
            started: t0,
        };
        assert_eq!(pan.at(t0), ((0.0, 0.0), false));
        let ((x, _), done) = pan.at(t0 + PAN / 2);
        assert!(!done && x > 50.0, "past halfway at half time: {x}");
        assert_eq!(pan.at(t0 + PAN), ((100.0, 50.0), true));
    }

    #[test]
    fn the_view_pans_to_a_caret_that_moves_and_the_pointer_wins() {
        let t0 = Instant::now();
        let mut t = Track::new(t0);
        let ptr = Some((500.0, 300.0));
        let mut z = at(8, 400.0, 200.0);
        let caret = |x| Some((x, 300.0, 2.0, 16.0));
        z = track(z, &mut t, ptr, caret(600.0), SIZE, t0);
        assert_eq!((z.x, z.y), (400.0, 200.0), "the pointer was first seen");
        // The caret moves out of view to the right: a pan starts.
        let t1 = t0 + SETTLE;
        z = track(z, &mut t, ptr, caret(950.0), SIZE, t1);
        assert!(t.pan.is_some());
        assert_eq!(z.x, 400.0, "the pan starts where the view is");
        z = track(z, &mut t, ptr, caret(950.0), SIZE, t1 + PAN / 2);
        assert!(z.x > 400.0 && z.x < 532.0, "under way: {}", z.x);
        z = track(z, &mut t, ptr, caret(950.0), SIZE, t1 + PAN);
        assert_eq!(z.x, 532.0, "the caret 60 in from the right edge");
        assert!(t.pan.is_none());
        // The caret stays: the still pointer does not pull the view back.
        z = track(z, &mut t, ptr, caret(950.0), SIZE, t1 + PAN * 2);
        assert_eq!(z.x, 532.0);
        // The caret moves away again, and the pointer moves at once.
        z = track(
            z,
            &mut t,
            Some((520.0, 300.0)),
            caret(100.0),
            SIZE,
            t1 + PAN * 3,
        );
        assert!(t.pan.is_none(), "the pointer wins");
        assert_eq!(z.x, 520.0, "pushed by the pointer, not panned to the caret");
    }

    #[test]
    fn typing_during_a_pan_keeps_it_going() {
        let t0 = Instant::now();
        let mut t = Track::new(t0);
        let ptr = Some((500.0, 300.0));
        let caret = |x| Some((x, 300.0, 2.0, 16.0));
        let mut z = track(at(8, 400.0, 200.0), &mut t, ptr, caret(600.0), SIZE, t0);
        let t1 = t0 + SETTLE;
        z = track(z, &mut t, ptr, caret(950.0), SIZE, t1);
        let first = t.pan.unwrap();
        // A Backspace lands a quarter into the pan; its caret fits the
        // view the pan ends at.
        z = track(z, &mut t, ptr, caret(940.0), SIZE, t1 + PAN / 4);
        assert_eq!(t.pan, Some(first), "the same pan");
        // One past it extends the pan from where the view is.
        track(z, &mut t, ptr, caret(1100.0), SIZE, t1 + PAN / 2);
        let next = t.pan.unwrap();
        assert_eq!(next.to.0, 682.0);
        assert!(
            next.from.0 > 400.0 && next.from.0 < 532.0,
            "{:?}",
            next.from
        );
    }

    #[test]
    fn a_caret_showing_up_right_after_the_zoom_is_not_followed() {
        let t0 = Instant::now();
        let mut t = Track::new(t0);
        let z = at(8, 400.0, 200.0);
        let ptr = Some((500.0, 300.0));
        let z = track(z, &mut t, ptr, None, SIZE, t0);
        let far = Some((1500.0, 900.0, 2.0, 16.0));
        let z = track(z, &mut t, ptr, far, SIZE, t0 + SETTLE / 2);
        assert!(t.pan.is_none() && z.x == 400.0, "taken as where it is");
        let mut late = Track::new(t0);
        late.pointer = ptr;
        track(z, &mut late, ptr, far, SIZE, t0 + SETTLE);
        assert!(late.pan.is_some(), "after settling, a new focus pans");
    }

    /// A GPU renderer, or None (the test then passes without one).
    fn renderer() -> Option<GlesRenderer> {
        match reprisefx::headless::renderer() {
            Ok(r) => Some(r),
            Err(e) => {
                eprintln!("skipped: {e}");
                None
            }
        }
    }

    fn solid(
        at: (i32, i32),
        colour: [f32; 4],
    ) -> smithay::backend::renderer::element::solid::SolidColorRenderElement {
        use smithay::backend::renderer::{
            element::solid::SolidColorRenderElement, utils::CommitCounter,
        };
        SolidColorRenderElement::new(
            Id::new(),
            Rectangle::new(at.into(), (4, 4).into()),
            CommitCounter::default(),
            colour,
            Kind::Unspecified,
        )
    }

    #[test]
    fn the_screen_shows_the_view_stretched_across_it() {
        let Some(mut r) = renderer() else {
            return;
        };
        // The unzoomed picture: red on the left half, blue on the right.
        let red = [1.0, 0.0, 0.0, 1.0];
        let blue = [0.0, 0.0, 1.0, 1.0];
        let mut picture = Target::new(&mut r, (8, 4).into(), 1.0).unwrap();
        picture
            .render(&mut r, &[solid((0, 0), red), solid((4, 0), blue)])
            .unwrap();
        // At 2x the screen shows half the picture's width: from x 0 the
        // red half fills it, from x 4 the blue one. The one column next to
        // the far edge of the view blends with the other half.
        let mut damage = DamageBag::new(DAMAGE_KEPT);
        damage.add([Rectangle::from_size((8, 4).into())]);
        let mut screen_at = |x: f64| {
            let e = screen_element(
                &r,
                picture.texture(),
                Id::new(),
                &damage,
                at(4, x, 0.0).source((8.0, 4.0), 1.0),
                (8, 4).into(),
            );
            let mut screen = Target::new(&mut r, (8, 4).into(), 1.0).unwrap();
            screen.render(&mut r, &[e]).unwrap();
            screen.read(&mut r).unwrap()
        };
        let left = screen_at(0.0);
        let right = screen_at(4.0);
        for y in 0..4 {
            for x in 0..7 {
                assert_eq!(left.pixels[y * 8 + x], 0xffff_0000, "red at {x},{y}");
                let x = x + 1;
                assert_eq!(right.pixels[y * 8 + x], 0xff00_00ff, "blue at {x},{y}");
            }
        }
    }

    /// Each filter's shader gives what its matrix gives on the CPU, within
    /// a level of rounding, over a picture of pure and mixed colours.
    #[test]
    fn the_filtered_screen_is_the_picture_through_the_matrix() {
        let Some(mut r) = renderer() else {
            return;
        };
        let colours = [
            [1.0, 0.0, 0.0, 1.0],
            [0.0, 0.6, 0.2, 1.0],
            [0.2, 0.4, 1.0, 1.0],
            [0.5, 0.5, 0.5, 1.0],
        ];
        let mut picture = Target::new(&mut r, (16, 4).into(), 1.0).unwrap();
        let tiles: Vec<_> = colours
            .iter()
            .enumerate()
            .map(|(i, c)| solid((4 * i as i32, 0), *c))
            .collect();
        picture.render(&mut r, &tiles).unwrap();
        let plain = picture.read(&mut r).unwrap();
        let fx = reprisefx::ColorFx::new(&mut r).unwrap();
        let mut damage = DamageBag::new(DAMAGE_KEPT);
        damage.add([Rectangle::from_size((16, 4).into())]);
        for f in Filter::ALL.into_iter().skip(1) {
            let m = reprisekit::colorfilter::matrix(f, 1.0).unwrap();
            let e = screen_element(
                &r,
                picture.texture(),
                Id::new(),
                &damage,
                Zoom::OFF.source((16.0, 4.0), 1.0),
                (16, 4).into(),
            );
            let e = fx.element(e, picture.texture().clone(), &m);
            let mut screen = Target::new(&mut r, (16, 4).into(), 1.0).unwrap();
            screen.render(&mut r, &[e]).unwrap();
            let got = screen.read(&mut r).unwrap();
            for (i, (&g, &p)) in got.pixels.iter().zip(&plain.pixels).enumerate() {
                let want = m.apply_argb(p);
                let ch = |v: u32, s: u32| ((v >> s) & 0xff) as i32;
                let off = [24, 16, 8, 0]
                    .iter()
                    .map(|&s| (ch(g, s) - ch(want, s)).abs())
                    .max()
                    .unwrap();
                assert!(
                    off <= 1,
                    "{f:?} pixel {i}: {g:08x}, the matrix gives {want:08x}"
                );
            }
        }
    }

    #[test]
    fn the_source_is_the_view_in_device_pixels() {
        let z = at(4, 400.0, 200.0);
        assert_eq!(
            z.source(SIZE, 1.5),
            Rectangle::new((600.0, 300.0).into(), (1440.0, 810.0).into()),
            "2x at scale 1.5: a 960x540 view from 400,200"
        );
    }
}
