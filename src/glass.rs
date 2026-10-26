//! Glass's blurred backdrop on the scene: one `reprisefx` element per piece of glass, made each
//! frame by a scene node beneath the chrome it backs.
//! Glass blurs the output's backdrop with chrome left out (`GlassCtx::update_backdrop`); glass over
//! chrome or under other glass (`[glass] blur_frames`) blurs a backdrop of its own, with the glass
//! beneath it drawn in.

#[cfg(test)]
mod tests;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

use reprisefx::{Glass, GlassFrame, GlassFx, Lens, Picture, Quality, Shape, Source, Update};
use reprisekit::backdrop::{corner_radius, rounded_rects};
use reprisekit::classic::frame::FrameGeometry;
use reprisekit::glass::tint::Surface;
use reprisekit::glass::{Material, Settings};
use reprisescene::{
    CustomElement, CustomId, CustomNode, Input, NodeId, SceneElement, TreeId, View,
};
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            Bind, Color32F, Frame, Offscreen,
            damage::OutputDamageTracker,
            element::{
                Element, Id, Kind, NamespacedElement, RenderElement, UnderlyingStorage,
                utils::{CropRenderElement, Relocate, RelocateRenderElement},
            },
            gles::{GlesError, GlesFrame, GlesRenderer, GlesTexture},
            utils::{CommitCounter, DamageSet, OpaqueRegions},
        },
    },
    output::Output,
    utils::{
        Buffer as BufferCoords, Logical, Physical, Point, Rectangle, Scale, Size, Transform,
        user_data::UserDataMap,
    },
};

use crate::compositor::DeskScene;
use crate::config::Config;
use crate::input::Interaction;
use crate::raster::Rect;

#[derive(Default)]
pub struct GlassCtx {
    /// The shaders; None before the backend starts or when they did not compile (glass then draws
    /// nothing behind its art).
    fx: RefCell<Option<GlassFx>>,
    flip: RefCell<Option<reprisefx::FlipFx>>,
    strokes: Cell<Option<reprisefx::StrokeFx>>,
    pass: Cell<Pass>,
    /// The backdrop the frame being built blurs; None blurs the frame beneath each piece of glass.
    source: RefCell<Option<Source>>,
    backdrops: RefCell<HashMap<String, OutputBackdrop>>,
    glasses: RefCell<Vec<Weak<Glass>>>,
    made: RefCell<Vec<Id>>,
    nodes: RefCell<Vec<Placed>>,
    own: RefCell<Vec<(Rc<Glass>, Source)>>,
    /// Private backdrops by output and glass (a resize makes the glass a new node); the entry's
    /// `Weak` keeps the glass's address from being reused while it lives.
    beneath: RefCell<HashMap<(String, *const Glass), Beneath>>,
    /// The elements being made are a backdrop of its own for glass above, this many output pixels
    /// per texel: glass draws into it at that resolution.
    drawing_beneath: Cell<Option<i32>>,
}

struct Placed {
    node: CustomId,
    glass: Weak<Glass>,
    pane: Rc<RefCell<Pane>>,
}

struct Beneath {
    backdrop: OutputBackdrop,
    glass: Weak<Glass>,
    reuse: Reuse,
}

/// How long a private backdrop outlives its last use: glass dragged past other glass resumes it,
/// its damage tracker drawing only what changed in between.
const KEEP_IDLE: Duration = Duration::from_secs(2);

/// When glass last drew from its private backdrop.
#[derive(Debug, Default)]
struct Reuse {
    last: Option<Instant>,
    /// It drew from it in the last update.
    used: bool,
}

impl Reuse {
    fn drawn(&mut self, now: Instant) {
        self.last = Some(now);
        self.used = true;
    }

    /// Whether the glass drew from it in the last update.
    fn in_use(&self) -> bool {
        self.used
    }

    /// Whether to keep it after an update at `now` that did not draw from it.
    fn kept(&mut self, now: Instant) -> bool {
        self.used = false;
        self.last
            .is_some_and(|t| now.saturating_duration_since(t) <= KEEP_IDLE)
    }
}

smithay::render_elements! {
    /// What a backdrop draws: the output's cut to the pieces its glass reads; a private one the
    /// scene below its glass, cut to what feeds that glass and moved to the backdrop's top-left.
    Private<=GlesRenderer>;
    Moved=RelocateRenderElement<CropRenderElement<Shared>>,
    Cut=CropRenderElement<Shared>,
    Piece=NamespacedElement<CropRenderElement<Shared>>,
}

/// A scene element made once per walk and cut into each piece of the backdrop.
struct Shared(Rc<SceneElement<GlesRenderer>>);

impl Element for Shared {
    fn id(&self) -> &Id {
        self.0.id()
    }
    fn current_commit(&self) -> CommitCounter {
        self.0.current_commit()
    }
    fn location(&self, scale: Scale<f64>) -> Point<i32, Physical> {
        self.0.location(scale)
    }
    fn src(&self) -> Rectangle<f64, BufferCoords> {
        self.0.src()
    }
    fn transform(&self) -> Transform {
        self.0.transform()
    }
    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.0.geometry(scale)
    }
    fn damage_since(
        &self,
        scale: Scale<f64>,
        commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        self.0.damage_since(scale, commit)
    }
    fn opaque_regions(&self, scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        self.0.opaque_regions(scale)
    }
    fn alpha(&self) -> f32 {
        self.0.alpha()
    }
    fn kind(&self) -> Kind {
        self.0.kind()
    }
    fn is_framebuffer_effect(&self) -> bool {
        self.0.is_framebuffer_effect()
    }
}

impl RenderElement<GlesRenderer> for Shared {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, BufferCoords>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
        cache: Option<&UserDataMap>,
    ) -> Result<(), GlesError> {
        self.0.draw(frame, src, dst, damage, opaque_regions, cache)
    }
    fn underlying_storage(&self, renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        self.0.underlying_storage(renderer)
    }
    fn capture_framebuffer(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, BufferCoords>,
        dst: Rectangle<i32, Physical>,
        cache: &UserDataMap,
    ) -> Result<(), GlesError> {
        self.0.capture_framebuffer(frame, src, dst, cache)
    }
}

/// The parts of the output's backdrop to draw for glass fed from `feeds` (texels): overlapping
/// feeds joined, top first. Glass far apart (a caption and the taskbar) leaves what lies between
/// undrawn.
fn pieces(feeds: impl Iterator<Item = Rectangle<i32, Physical>>) -> Vec<Rectangle<i32, Physical>> {
    let mut out: Vec<Rectangle<i32, Physical>> = Vec::new();
    for mut f in feeds {
        while let Some(i) = out.iter().position(|p| p.overlaps(f)) {
            f = f.merge(out.swap_remove(i));
        }
        out.push(f);
    }
    out.sort_by_key(|r| (r.loc.y, r.loc.x));
    out
}

/// `r` (output pixels) in texels of `d` output pixels, rounded out.
fn shrunk(r: Rectangle<i32, Physical>, d: i32) -> Rectangle<i32, Physical> {
    let (x0, y0) = (r.loc.x.div_euclid(d), r.loc.y.div_euclid(d));
    let x1 = (r.loc.x + r.size.w + d - 1).div_euclid(d);
    let y1 = (r.loc.y + r.size.h + d - 1).div_euclid(d);
    Rectangle::new((x0, y0).into(), (x1 - x0, y1 - y0).into())
}

/// A shown glass node in stacking order, with what its glass reads.
struct Stacked {
    glass: Rc<Glass>,
    anchor: NodeId,
    reads: Reads,
}

struct OutputBackdrop {
    texture: GlesTexture,
    tracker: OutputDamageTracker,
    /// Its size, texels, and its view scale's bits.
    key: (Size<i32, Physical>, u64),
    /// The part of the output it holds, output pixels.
    area: Rectangle<i32, Physical>,
    drawn: bool,
    picture: Picture,
    hidden: bool,
    /// The settings and quality of the last update that drew it.
    look: Option<(Settings, Quality)>,
    /// A held frame left it as it was: what changed since waits for the next blur.
    skipped: bool,
    /// A held frame left a window's own glass showing its program as it was: it blurs again
    /// within a `[glass] blur_rate` interval (`reblur`).
    owed: bool,
    /// The clear colour it was last drawn on: the tracker sees a change only when it draws.
    clear: Option<Color32F>,
}

/// Whether a frame keeps the last blur (`[glass] blur_rate`'s cap is not up).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Hold {
    #[default]
    No,
    /// It keeps it; what the frame's asks changed under glass waits for the next blur.
    Keep(Under),
}

impl Hold {
    pub fn keeps(self) -> bool {
        self != Hold::No
    }
}

/// Where the asks for a frame may have changed what glass shows through it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Under {
    /// Only the pointer or the cursor asked.
    #[default]
    Nothing,
    /// Only a program's pixels within a box changed.
    Within(Drawn),
    Anywhere,
}

/// A program's commit that changed only pixels: the box they lie in (layout) and the layer
/// entry (window) showing them, if all of them lie in one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Drawn {
    pub r: Rectangle<i32, Logical>,
    pub window: Option<NodeId>,
}

impl Under {
    pub fn and(self, other: Under) -> Under {
        match (self, other) {
            (Under::Nothing, u) | (u, Under::Nothing) => u,
            (Under::Within(a), Under::Within(b)) => Under::Within(Drawn {
                r: a.r.merge(b.r),
                window: a.window.filter(|_| a.window == b.window),
            }),
            _ => Under::Anywhere,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Pass {
    pub settings: Settings,
    pub output: Size<i32, Physical>,
    /// The output's name, hashed: glass keeps its damage per output ([`GlassFrame::screen`]).
    pub screen: u64,
    pub quality: Quality,
    pub hold: Hold,
}

impl GlassCtx {
    pub fn new() -> Rc<GlassCtx> {
        Rc::new(GlassCtx::default())
    }

    pub fn set_renderer(&self, renderer: &mut GlesRenderer) {
        let fx = GlassFx::new(renderer)
            .map_err(|e| log::warn!("glass: no blur, the shaders did not compile: {e}"))
            .ok();
        *self.fx.borrow_mut() = fx;
        let flip = reprisefx::FlipFx::new(renderer)
            .map_err(|e| log::warn!("3D switcher: no cards, the shader did not compile: {e}"))
            .ok();
        *self.flip.borrow_mut() = flip;
        let strokes = reprisefx::StrokeFx::new(renderer)
            .map_err(|e| {
                log::warn!("screen savers: no stars or lines, the shader did not compile: {e}")
            })
            .ok();
        self.strokes.set(strokes);
    }

    /// See [`GlassFx::on_streaks`].
    pub fn on_streaks(&self, wake: impl Fn() + Send + Sync + 'static) {
        if let Some(fx) = self.fx.borrow().as_ref() {
            fx.on_streaks(wake);
        }
    }

    /// See [`GlassFx::fill_streaks`].
    pub fn fill_streaks(&self, fill: reprisefx::Fill) {
        if let Some(fx) = self.fx.borrow().as_ref() {
            fx.fill_streaks(fill);
        }
    }

    pub fn flip_fx(&self) -> Option<reprisefx::FlipFx> {
        self.flip.borrow().clone()
    }

    pub fn stroke_fx(&self) -> Option<reprisefx::StrokeFx> {
        self.strokes.get()
    }

    pub fn begin(&self, output: &Output, settings: Settings, quality: Quality, hold: Hold) {
        *self.source.borrow_mut() = None;
        self.own.borrow_mut().clear();
        self.made.borrow_mut().clear();
        let size = output
            .current_mode()
            .map(|m| output.current_transform().transform_size(m.size))
            .unwrap_or_default();
        let mut screen = std::hash::DefaultHasher::new();
        output.name().hash(&mut screen);
        self.pass.set(Pass {
            settings,
            output: size,
            screen: screen.finish(),
            quality,
            hold,
        });
    }

    /// Draw what changed in `output`'s backdrop, blur what that changed, and have the frame begun
    /// for it draw its glass from the picture. With `wanted` false every backdrop is dropped.
    pub fn update_backdrop(
        &self,
        renderer: &mut GlesRenderer,
        scene: &DeskScene,
        output: &Output,
        clear: Color32F,
        wanted: bool,
    ) {
        if !wanted {
            self.backdrops.borrow_mut().clear();
            self.beneath.borrow_mut().clear();
            return;
        }
        self.nodes
            .borrow_mut()
            .retain(|n| n.glass.strong_count() > 0 && scene.is_alive(n.node));
        let (Some(area), Some(mode)) = (reprisescene::output_rect(output), output.current_mode())
        else {
            return;
        };
        let seen = output.current_transform().transform_size(mode.size);
        let scale = output.current_scale().fractional_scale();
        let name = output.name();
        let own = match self.pass.get().settings.blur_frames {
            true => self.drawn_beneath(scene, area, seen, scale),
            false => Vec::new(),
        };
        {
            let mut backdrops = self.backdrops.borrow_mut();
            if backdrops
                .get(&name)
                .is_none_or(|b| b.key != OutputBackdrop::key(seen, scale))
            {
                let whole = Rectangle::from_size(seen);
                let Some(fresh) = OutputBackdrop::new(renderer, whole, scale, &name) else {
                    return;
                };
                backdrops.insert(name.clone(), fresh);
            }
            let Some(b) = backdrops.get_mut(&name) else {
                return;
            };
            let d = reprisefx::blur::backdrop_downscale(scale);
            let view = View::new(area, scale / f64::from(d));
            let reads = self.reads_on(scene, area, seen, scale, &own);
            let needed: Vec<_> = reads.iter().flat_map(|r| r.needed.clone()).collect();
            // Only the backdrop the glass reads is drawn: what changes elsewhere (the water under
            // Rain each frame) waits until glass moves over it, which moves the cuts.
            let cuts = pieces(reads.iter().filter_map(|r| r.feed).map(|f| shrunk(f, d)));
            let walked: Vec<Rc<SceneElement<GlesRenderer>>> = match cuts.is_empty() {
                true => Vec::new(),
                false => (scene.backdrop_elements(renderer, &view).into_iter())
                    .map(Rc::new)
                    .collect(),
            };
            let mut elements: Vec<Private> = Vec::new();
            for (i, &cut) in cuts.iter().enumerate() {
                // The damage tracker keeps one state per id: an element in a later piece is
                // namespaced by it.
                let cut_out = walked.iter().filter_map(|e| {
                    CropRenderElement::from_element(Shared(e.clone()), view.scale, cut)
                });
                elements.extend(cut_out.map(|e| match i {
                    0 => Private::Cut(e),
                    _ => Private::Piece(NamespacedElement::new(e, i)),
                }));
            }
            *self.source.borrow_mut() =
                self.redraw(renderer, b, &elements, clear, output, Some(&needed));
            let pass = self.pass.get();
            b.look = Some((pass.settings, pass.quality));
            b.skipped = false;
            b.owed = false;
        }
        if self.pass.get().settings.blur_frames {
            self.update_beneath(renderer, scene, output, clear, &own);
        } else {
            self.beneath.borrow_mut().retain(|(o, _), _| *o != name);
        }
    }

    /// What the glass shown on the output at `area` reads from its picture, glass drawn from its
    /// own backdrop (`own`) aside: only changes there are drawn and blurred, the rest wait until
    /// glass reads them.
    fn reads_on(
        &self,
        scene: &DeskScene,
        area: Rectangle<i32, Logical>,
        seen: Size<i32, Physical>,
        scale: f64,
        own: &[Stacked],
    ) -> Vec<Reads> {
        self.reads_where(scene, area, seen, scale, |n| {
            !own.iter()
                .any(|o| Weak::as_ptr(&n.glass) == Rc::as_ptr(&o.glass))
        })
    }

    /// What the glass shown on the output at `area` that `keep` picks reads from its picture.
    fn reads_where(
        &self,
        scene: &DeskScene,
        area: Rectangle<i32, Logical>,
        seen: Size<i32, Physical>,
        scale: f64,
        keep: impl Fn(&Placed) -> bool,
    ) -> Vec<Reads> {
        let pass = self.pass.get();
        self.nodes
            .borrow()
            .iter()
            .filter(|n| n.glass.strong_count() > 0 && scene.is_alive(n.node))
            .filter(|n| keep(n))
            .filter_map(|n| {
                let Some((at, true)) = scene.coords(n.node) else {
                    return None;
                };
                let pane = n.pane.borrow();
                Rectangle::new(at, pane.extent()).overlaps(area).then(|| {
                    let at = (at - area.loc).to_f64().to_physical(scale);
                    Reads::of(&pane, at, seen, scale, &pass)
                })
            })
            .collect()
    }

    /// Whether glass shown on `output` reads its backdrop where `d` drew. The drawing window's
    /// own glass is left out: it may show its program a blur interval late.
    fn reads_within(&self, scene: &DeskScene, output: &Output, d: Drawn) -> bool {
        let (Some(area), Some(mode)) = (reprisescene::output_rect(output), output.current_mode())
        else {
            return true;
        };
        let Some(r) = d.r.intersection(area) else {
            return false;
        };
        let seen = output.current_transform().transform_size(mode.size);
        let scale = output.current_scale().fractional_scale();
        let r = Rectangle::new(r.loc - area.loc, r.size).to_physical_precise_up(scale);
        let others = |n: &Placed| d.window.is_none() || anchor(scene, n.node.into()) != d.window;
        (self.reads_where(scene, area, seen, scale, others).iter())
            .any(|reads| reads.reach.iter().any(|x| x.overlaps(r)))
    }

    /// The glass shown on the output at `area` that draws from a backdrop of its own, bottom
    /// first: glass with other glass drawn above it, or with left-out chrome beneath it.
    fn drawn_beneath(
        &self,
        scene: &DeskScene,
        area: Rectangle<i32, Logical>,
        seen: Size<i32, Physical>,
        scale: f64,
    ) -> Vec<Stacked> {
        let pass = self.pass.get();
        let nodes = self.nodes.borrow();
        let shown = scene.leaves_in(area);
        let mut stacked: Vec<(usize, &Placed)> = nodes
            .iter()
            .filter_map(|n| Some((shown.iter().position(|&l| l == n.node.into())?, n)))
            .collect();
        // Bottom first: a lower glass's backdrop is final before glass above draws that glass.
        stacked.sort_by_key(|&(i, _)| std::cmp::Reverse(i));
        let stacked: Vec<Stacked> = stacked
            .into_iter()
            .filter_map(|(_, n)| {
                let (Some(glass), Some((at, true)), Some(anchor)) = (
                    n.glass.upgrade(),
                    scene.coords(n.node),
                    anchor(scene, n.node.into()),
                ) else {
                    return None;
                };
                let at = (at - area.loc).to_f64().to_physical(scale);
                let reads = Reads::of(&n.pane.borrow(), at, seen, scale, &pass);
                Some(Stacked {
                    glass,
                    anchor,
                    reads,
                })
            })
            .collect();
        let own: Vec<bool> = stacked
            .iter()
            .enumerate()
            .map(|(i, n)| {
                // Glass above draws this glass from this glass's own backdrop: the output's holds
                // the clients of the windows above it.
                let drawn_above = stacked[i + 1..].iter().any(|u| {
                    u.anchor != n.anchor
                        && u.reads
                            .reach
                            .iter()
                            .any(|r| n.reads.needed.iter().any(|m| m.overlaps(*r)))
                });
                drawn_above
                    || n.reads
                        .reach
                        .iter()
                        .any(|r| scene.left_out_below(n.anchor, layout(*r, area.loc, scale)))
            })
            .collect();
        stacked
            .into_iter()
            .zip(own)
            .filter_map(|(n, own)| own.then_some(n))
            .collect()
    }

    fn update_beneath(
        &self,
        renderer: &mut GlesRenderer,
        scene: &DeskScene,
        output: &Output,
        clear: Color32F,
        own: &[Stacked],
    ) {
        let (Some(area), Some(mode)) = (reprisescene::output_rect(output), output.current_mode())
        else {
            return;
        };
        let seen = output.current_transform().transform_size(mode.size);
        let scale = output.current_scale().fractional_scale();
        let name = output.name();
        let pass = self.pass.get();
        let mut beneath = self.beneath.borrow_mut();
        let mut used = Vec::new();
        let now = Instant::now();
        let d = reprisefx::blur::backdrop_downscale(scale);
        let s = &pass.settings;
        let f = reprisefx::blur::texel(s.blur, s.material, pass.quality, scale);
        let whole = Rectangle::from_size(seen);
        for n in own {
            let (glass, anchor, reads) = (n.glass.clone(), n.anchor, &n.reads);
            let held = reads.feed.map_or(whole, |r| {
                let r = shrunk(r, f);
                let r = Rectangle::new(r.loc.upscale(f), r.size.upscale(f));
                r.intersection(whole).unwrap_or(r)
            });
            let key = (name.clone(), Rc::as_ptr(&glass));
            if beneath
                .get(&key)
                .is_none_or(|b| b.backdrop.key != OutputBackdrop::key(held.size, scale))
            {
                let Some(backdrop) = OutputBackdrop::new(renderer, held, scale, &name) else {
                    continue;
                };
                let glass = Rc::downgrade(&glass);
                let reuse = Reuse::default();
                beneath.insert(
                    key.clone(),
                    Beneath {
                        backdrop,
                        glass,
                        reuse,
                    },
                );
            }
            let Some(b) = beneath.get_mut(&key) else {
                continue;
            };
            // Moved glass of the same size keeps the textures: the tracker sees every element move.
            b.backdrop.area = held;
            let cut = shrunk(held, d);
            let view = View::new(area, scale / f64::from(d));
            self.drawing_beneath.set(Some(d));
            let elements: Vec<Private> = scene
                .elements_below(renderer, anchor, &view)
                .into_iter()
                .filter_map(|e| {
                    CropRenderElement::from_element(Shared(Rc::new(e)), view.scale, cut)
                })
                .map(|e| {
                    let to = Point::from((-cut.loc.x, -cut.loc.y));
                    Private::Moved(RelocateRenderElement::from_element(
                        e,
                        to,
                        Relocate::Relative,
                    ))
                })
                .collect();
            self.drawing_beneath.set(None);
            let needed = Some(&reads.needed[..]);
            if let Some(source) =
                self.redraw(renderer, &mut b.backdrop, &elements, clear, output, needed)
            {
                b.reuse.drawn(now);
                self.own.borrow_mut().push((glass, source));
                used.push(key);
            }
        }
        beneath.retain(|k, b| {
            k.0 != name || used.contains(k) || (b.reuse.kept(now) && b.glass.strong_count() > 0)
        });
    }

    /// Drop the private backdrops on `output` its glass did not draw from in the last update:
    /// the output stopped rendering, so no drag resumes them soon.
    pub fn output_stopped(&self, output: &Output) {
        let name = output.name();
        (self.beneath.borrow_mut()).retain(|(o, _), b| *o != name || b.reuse.in_use());
    }

    /// Drop `output`'s backdrop and the private backdrops on it.
    pub fn output_removed(&self, output: &Output) {
        let name = output.name();
        self.backdrops.borrow_mut().remove(&name);
        (self.beneath.borrow_mut()).retain(|(o, _), _| *o != name);
    }

    fn redraw<E: RenderElement<GlesRenderer>>(
        &self,
        renderer: &mut GlesRenderer,
        b: &mut OutputBackdrop,
        elements: &[E],
        clear: Color32F,
        output: &Output,
        needed: Option<&[Rectangle<i32, Physical>]>,
    ) -> Option<Source> {
        let name = output.name();
        let mode = output.current_mode()?;
        let fx = self.fx.borrow();
        let fx = fx.as_ref()?;
        let d = reprisefx::blur::backdrop_downscale(output.current_scale().fractional_scale());
        // Without a capturing element the damage is known before drawing, so the picture's blur
        // can draw the backdrop inside its own frame.
        let inside = !elements.iter().any(Element::is_framebuffer_effect);
        let age = usize::from(b.drawn && (!inside || b.clear == Some(clear)));
        let rendered = match inside {
            true => b
                .tracker
                .damage_output(age, elements)
                .map(|(damage, _)| damage.cloned().unwrap_or_default())
                .map_err(|e| format!("{e:?}")),
            false => renderer
                .bind(&mut b.texture)
                .map_err(|e| e.to_string())
                .and_then(|mut fb| {
                    b.tracker
                        .render_output(renderer, &mut fb, age, elements, clear)
                        .map(|r| r.damage.cloned().unwrap_or_default())
                        .map_err(|e| format!("{e:?}"))
                }),
        };
        let damage = match rendered {
            Ok(damage) => damage,
            Err(e) => {
                log::warn!("glass: the backdrop of {name} did not render: {e}");
                b.drawn = false;
                return None;
            }
        };
        b.drawn = true;
        b.clear = Some(clear);
        let at = b.area.loc;
        let changed: Vec<Rectangle<i32, Physical>> = damage
            .iter()
            .map(|r| {
                Rectangle::new(
                    (at.x + r.loc.x * d, at.y + r.loc.y * d).into(),
                    (r.size.w * d, r.size.h * d).into(),
                )
            })
            .collect();
        b.picture.place(Some(b.area));
        let pass = self.pass.get();
        let update = Update {
            backdrop: &b.texture,
            changed: &changed,
            settings: &pass.settings,
            quality: pass.quality,
            output: output.current_transform().transform_size(mode.size),
            scale: output.current_scale().fractional_scale(),
            hold: pass.hold.keeps(),
            needed,
        };
        let view = Scale::from(f64::from_bits(b.key.1));
        let mut draw =
            |frame: &mut GlesFrame<'_, '_>| draw_damaged(frame, elements, view, &damage, clear);
        let draw: Option<reprisefx::Drawing<'_>> =
            (inside && !damage.is_empty()).then_some(&mut draw);
        if let Err(e) = b.picture.update_drawing(renderer, fx, &update, draw) {
            log::warn!("glass: the backdrop of {name} did not blur: {e}");
            b.drawn = false;
            return None;
        }
        b.picture.source()
    }

    pub fn update_screen_backdrop(
        &self,
        renderer: &mut GlesRenderer,
        scene: &DeskScene,
        output: &Output,
        clear: Color32F,
        wanted: bool,
    ) {
        let name = output.name();
        let pass = self.pass.get();
        let same = |b: &OutputBackdrop| {
            let seen = pass.output;
            let scale = output.current_scale().fractional_scale();
            b.key == OutputBackdrop::key(seen, scale)
                && b.look == Some((pass.settings, pass.quality))
        };
        if wanted
            && let Some(b) = self.backdrops.borrow_mut().get_mut(&name)
            && (b.hidden || (pass.hold.keeps() && b.drawn && same(b)))
        {
            // A held blur shows the picture as it was, so its backdrops need no drawing either.
            if let (false, Hold::Keep(under)) = (b.hidden, pass.hold) {
                match under {
                    Under::Anywhere => b.skipped = true,
                    Under::Within(d) if self.reads_within(scene, output, d) => b.skipped = true,
                    Under::Within(d) if d.window.is_some() => {
                        let any = Drawn { window: None, ..d };
                        b.owed |= self.reads_within(scene, output, any);
                    }
                    _ => {}
                }
            }
            *self.source.borrow_mut() = b.picture.source();
            let mut own = self.own.borrow_mut();
            for ((o, _), b) in self.beneath.borrow().iter() {
                if let (true, Some(g), Some(s)) = (
                    *o == name && b.reuse.in_use(),
                    b.glass.upgrade(),
                    b.backdrop.picture.source(),
                ) {
                    own.push((g, s));
                }
            }
            return;
        }
        self.update_backdrop(renderer, scene, output, clear, wanted);
    }

    /// Note whether the screen frame drawing `elements` shows any of the output's glass. True when
    /// glass shows again after frames that hid it all; the caller then rebuilds the elements.
    pub fn settle<E: Element>(&self, output: &Output, elements: &[E], scale: Scale<f64>) -> bool {
        let mut backdrops = self.backdrops.borrow_mut();
        let Some(b) = backdrops.get_mut(&output.name()) else {
            return false;
        };
        let shown = shows_glass(&self.made.borrow(), elements, scale);
        let resumed = b.hidden && shown;
        b.hidden = !shown;
        if resumed {
            let mut pass = self.pass.get();
            pass.hold = Hold::No;
            self.pass.set(pass);
            self.made.borrow_mut().clear();
        }
        resumed
    }

    pub fn any_stale(&self) -> bool {
        let backdrops = self.backdrops.borrow();
        let shown = |o: &String| backdrops.get(o).is_some_and(|b| !b.hidden);
        backdrops
            .values()
            .any(|b| !b.hidden && (b.skipped || b.picture.is_stale()))
            || self
                .beneath
                .borrow()
                .iter()
                .any(|((o, _), b)| shown(o) && b.reuse.in_use() && b.backdrop.picture.is_stale())
            || self
                .glasses
                .borrow()
                .iter()
                .filter_map(Weak::upgrade)
                .any(|g| g.is_stale())
    }

    /// Whether a held frame left `output`'s glass behind its own window's program.
    pub fn owes(&self, output: &Output) -> bool {
        (self.backdrops.borrow().get(&output.name())).is_some_and(|b| !b.hidden && b.owed)
    }

    /// As `owes`, settling the debt: the caller asks the frame that blurs.
    pub fn take_owed(&self, output: &Output) -> bool {
        let owes = self.owes(output);
        if let Some(b) = self.backdrops.borrow_mut().get_mut(&output.name()) {
            b.owed = false;
        }
        owes
    }

    pub fn refresh_stale(&self) -> bool {
        let mut any = false;
        let mut backdrops = self.backdrops.borrow_mut();
        for b in backdrops.values_mut() {
            if !b.hidden && (b.skipped || b.picture.is_stale()) {
                b.skipped = false;
                b.picture.refresh();
                any = true;
            }
        }
        for ((o, _), b) in self.beneath.borrow_mut().iter_mut() {
            let shown = backdrops.get(o).is_some_and(|b| !b.hidden);
            if shown && b.reuse.in_use() && b.backdrop.picture.is_stale() {
                b.backdrop.picture.refresh();
                any = true;
            }
        }
        let mut glasses = self.glasses.borrow_mut();
        glasses.retain(|g| g.strong_count() > 0);
        for g in glasses.iter().filter_map(Weak::upgrade) {
            if g.is_stale() {
                g.refresh();
                any = true;
            }
        }
        any
    }
}

/// Whether any of the glass elements `made` shows among `elements`; one not among them (drawn into
/// another picture) counts as shown.
fn shows_glass<E: Element>(made: &[Id], elements: &[E], scale: Scale<f64>) -> bool {
    let mut opaque: Vec<Rectangle<i32, Physical>> = Vec::new();
    let mut found = 0;
    for e in elements {
        let geometry = e.geometry(scale);
        if made.contains(e.id()) {
            if !geometry.subtract_rects(opaque.iter().copied()).is_empty() {
                return true;
            }
            found += 1;
        }
        opaque.extend(e.opaque_regions(scale).into_iter().map(|mut r| {
            r.loc += geometry.loc;
            r
        }));
    }
    found < made.len()
}

/// `elements` (front to back) drawn where `damage` lies into the frame's draw binding, as the
/// damage tracker draws them: hidden parts skipped, the clear under no opaque region.
fn draw_damaged<E: RenderElement<GlesRenderer>>(
    frame: &mut GlesFrame<'_, '_>,
    elements: &[E],
    scale: Scale<f64>,
    damage: &[Rectangle<i32, Physical>],
    clear: Color32F,
) -> Result<(), GlesError> {
    let output = Rectangle::from_size(frame.output_size());
    let mut opaque: Vec<Rectangle<i32, Physical>> = Vec::new();
    let mut shown = Vec::new();
    for e in elements {
        let geometry = e.geometry(scale);
        let Some(visible) = geometry.intersection(output) else {
            continue;
        };
        if visible.subtract_rects(opaque.iter().copied()).is_empty() {
            continue;
        }
        let parts: Vec<_> = damage
            .iter()
            .filter_map(|d| d.intersection(geometry))
            .collect();
        let parts = Rectangle::subtract_rects_many_in_place(parts, opaque.iter().copied());
        let start = opaque.len();
        opaque.extend(
            e.opaque_regions(scale)
                .into_iter()
                .map(|mut r| {
                    r.loc += geometry.loc;
                    r
                })
                .filter_map(|r| r.intersection(output)),
        );
        shown.push((e, geometry, parts, start..opaque.len()));
    }
    let cleared = Rectangle::subtract_rects_many_in_place(damage.to_vec(), opaque.iter().copied());
    frame.clear(clear, &cleared)?;
    for (e, geometry, mut parts, own) in shown.into_iter().rev() {
        if parts.is_empty() {
            continue;
        }
        let local = |mut r: Rectangle<i32, Physical>| {
            r.loc -= geometry.loc;
            r
        };
        parts.iter_mut().for_each(|r| *r = local(*r));
        let own: Vec<_> = opaque[own].iter().map(|r| local(*r)).collect();
        e.draw(frame, e.src(), geometry, &parts, &own, None)?;
    }
    Ok(())
}

impl OutputBackdrop {
    /// The key of a backdrop holding `size` output pixels.
    fn key(size: Size<i32, Physical>, scale: f64) -> (Size<i32, Physical>, u64) {
        let d = reprisefx::blur::backdrop_downscale(scale);
        let texels = Size::from(((size.w + d - 1) / d, (size.h + d - 1) / d));
        (texels, (scale / f64::from(d)).to_bits())
    }

    /// A backdrop holding `area` of the output, output pixels.
    fn new(
        renderer: &mut GlesRenderer,
        area: Rectangle<i32, Physical>,
        scale: f64,
        name: &str,
    ) -> Option<OutputBackdrop> {
        let key = OutputBackdrop::key(area.size, scale);
        let (size, view) = (key.0, f64::from_bits(key.1));
        let texture: GlesTexture = renderer
            .create_buffer(Fourcc::Abgr8888, (size.w, size.h).into())
            .map_err(|e| log::warn!("glass: no backdrop for {name}: {e}"))
            .ok()?;
        Some(OutputBackdrop {
            texture,
            tracker: OutputDamageTracker::new(size, view, Transform::Normal),
            key,
            area,
            drawn: false,
            picture: Picture::new(),
            hidden: false,
            look: None,
            skipped: false,
            owed: false,
            clear: None,
        })
    }
}

#[derive(Debug, PartialEq)]
struct Reads {
    needed: Vec<Rectangle<i32, Physical>>,
    reach: Vec<Rectangle<i32, Physical>>,
    /// The bounds of `reach` grown by the blur's reach again: what a private backdrop holds. None
    /// for glass off the output.
    feed: Option<Rectangle<i32, Physical>>,
}

impl Reads {
    fn of(
        pane: &Pane,
        at: Point<f64, Physical>,
        seen: Size<i32, Physical>,
        scale: f64,
        pass: &Pass,
    ) -> Reads {
        let s = &pass.settings;
        let to = |r: Rect| physical(r, at, Scale::from(scale));
        let holes: Vec<_> = pane.holes.iter().map(|&h| to(h)).collect();
        let (texel, blur) = (
            reprisefx::blur::texel(s.blur, s.material, pass.quality, scale),
            reprisefx::blur::reach(s.blur, s.material, pass.quality, scale),
        );
        let output = Rectangle::from_size(seen);
        let mut out = Reads {
            needed: Vec::new(),
            reach: Vec::new(),
            feed: None,
        };
        for shape in &pane.shapes {
            let r = to(shape.rect);
            let bend = Lens::new(s.material, r, seen, scale).map_or(0, |l| l.reach());
            for part in r.subtract_rects(holes.iter().copied()) {
                let grow = |g: i32| {
                    Rectangle::new(
                        (part.loc.x - g, part.loc.y - g).into(),
                        (part.size.w + 2 * g, part.size.h + 2 * g).into(),
                    )
                    .intersection(output)
                };
                out.needed.extend(grow(bend + texel));
                out.reach.extend(grow(bend + blur));
            }
        }
        out.feed = out
            .reach
            .iter()
            .copied()
            .reduce(|a, b| a.merge(b))
            .and_then(|r| {
                Rectangle::new(
                    (r.loc.x - blur, r.loc.y - blur).into(),
                    (r.size.w + 2 * blur, r.size.h + 2 * blur).into(),
                )
                .intersection(output)
            });
        out
    }
}

fn layout(
    r: Rectangle<i32, Physical>,
    origin: Point<i32, Logical>,
    scale: f64,
) -> Rectangle<i32, Logical> {
    let x0 = (f64::from(r.loc.x) / scale).floor() as i32;
    let y0 = (f64::from(r.loc.y) / scale).floor() as i32;
    let x1 = (f64::from(r.loc.x + r.size.w) / scale).ceil() as i32;
    let y1 = (f64::from(r.loc.y + r.size.h) / scale).ceil() as i32;
    Rectangle::new(origin + Point::from((x0, y0)), (x1 - x0, y1 - y0).into())
}

/// The layer entry `node` belongs to: its ancestor right under a layer tree; None for a node under
/// no layer.
pub fn anchor(scene: &DeskScene, node: NodeId) -> Option<NodeId> {
    let root = scene.root();
    let mut id = node;
    loop {
        let parent = scene.parent(id)?;
        if scene.parent(parent) == Some(root) {
            return Some(id);
        }
        id = parent.into();
    }
}

pub fn wanted(config: &Config) -> bool {
    config.theme.glass().is_some() && config.glass.transparency
}

pub fn quality(interaction: &Interaction, settings: &Settings) -> Quality {
    let dragging = matches!(
        interaction,
        Interaction::Move { .. }
            | Interaction::Resize { .. }
            | Interaction::DialogMove { .. }
            | Interaction::DialogResize { .. }
    );
    reprisefx::blur::quality(dragging, settings.fast_drag)
}

pub fn leave_out(scene: &mut DeskScene, node: impl Into<NodeId>) {
    scene.set_in_backdrop(node, false);
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PaneShape {
    pub rect: Rect,
    /// The style's corner table: entry `i` is how many pixels row `i` from the top or bottom loses
    /// on each side; empty for square corners.
    pub corner: &'static [i32],
    pub square_bottom: bool,
    pub square_top: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Pane {
    /// The glass. Shapes must not overlap: each draws on its own.
    pub shapes: Vec<PaneShape>,
    pub holes: Vec<Rect>,
}

impl Pane {
    fn extent(&self) -> Size<i32, Logical> {
        let (w, h) = self.shapes.iter().fold((0, 0), |(w, h), s| {
            (w.max(s.rect.right()), h.max(s.rect.bottom()))
        });
        (w, h).into()
    }
}

pub fn frame_pane(geom: &FrameGeometry, corner: &'static [i32], maximized: bool) -> Pane {
    Pane {
        shapes: vec![PaneShape {
            rect: Rect::new(0, 0, geom.total.0, geom.total.1),
            corner: if maximized { &[] } else { corner },
            square_bottom: false,
            square_top: false,
        }],
        holes: vec![geom.client],
    }
}

pub fn sheet_pane(size: (i32, i32), corner: &'static [i32]) -> Pane {
    Pane {
        shapes: vec![PaneShape {
            rect: Rect::new(0, 0, size.0, size.1),
            corner,
            square_bottom: false,
            square_top: false,
        }],
        holes: vec![],
    }
}

fn flyout_pane(size: (i32, i32), border: i32, corner: &'static [i32]) -> Pane {
    let all = Rect::new(0, 0, size.0, size.1);
    Pane {
        shapes: vec![PaneShape {
            rect: all,
            corner,
            square_bottom: false,
            square_top: false,
        }],
        holes: vec![all.inset(border)],
    }
}

pub fn flyout_glass(config: &Config, size: (i32, i32)) -> Option<Pane> {
    let theme = &config.theme;
    reprisekit::flyout::look(theme)?;
    let g = theme.glass().filter(|_| wanted(config))?;
    Some(flyout_pane(
        size,
        reprisekit::flyout::border(theme),
        g.corner,
    ))
}

/// Show a flyout canvas with its shadow in `buffer`; only the `size` panel takes the pointer.
pub fn place_flyout(
    scene: &mut DeskScene,
    buffer: reprisescene::BufferId,
    theme: &reprisekit::theme::Theme,
    size: (i32, i32),
) {
    let m = reprisekit::flyout::margin(theme);
    scene.set_position(buffer, -m.left, -m.top);
    scene.set_input(
        buffer,
        Input::Rect(Rectangle::new((m.left, m.top).into(), size.into())),
    );
}

/// `pane`'s shapes as drawn at `scale`, with corner radii (tl, tr, br, bl): whole and rounded under
/// Molten or above scale 1, else cut into the table's rows. A square-bottomed or square-topped
/// shape keeps those corners square in both.
pub(crate) fn drawn_shapes(pane: &Pane, material: Material, scale: f64) -> Vec<(Rect, [f32; 4])> {
    let fine = crate::output::draw_scale(scale as f32) > 1;
    if material == Material::Molten || fine {
        return round_shapes(pane);
    }
    pane.shapes
        .iter()
        .flat_map(|s| {
            let r = s.rect;
            // Square-bottomed: the top half of a shape twice as tall. Square-topped: the bottom half.
            let (corner, h, skip) = match (s.square_top, s.square_bottom) {
                (true, true) => (&[][..], r.height, 0),
                (true, false) => (s.corner, 2 * r.height, r.height),
                (false, true) => (s.corner, 2 * r.height, 0),
                (false, false) => (s.corner, r.height, 0),
            };
            rounded_rects(r.width, h, corner)
                .into_iter()
                .filter_map(move |p| {
                    let y0 = p.y.max(skip);
                    let y1 = p.bottom().min(skip + r.height);
                    (y1 > y0).then(|| {
                        (
                            Rect::new(r.x + p.x, r.y + y0 - skip, p.width, y1 - y0),
                            [0.0; 4],
                        )
                    })
                })
        })
        .collect()
}

/// `pane`'s shapes whole, with corner radii (tl, tr, br, bl) from their tables.
fn round_shapes(pane: &Pane) -> Vec<(Rect, [f32; 4])> {
    pane.shapes
        .iter()
        .map(|s| {
            let c = corner_radius(s.corner);
            let t = if s.square_top { 0.0 } else { c };
            let b = if s.square_bottom { 0.0 } else { c };
            (s.rect, [t, t, b, b])
        })
        .collect()
}

/// `r` in physical pixels with each edge rounded on its own, so neighbouring rects meet without a
/// gap.
pub fn physical(
    r: Rect,
    location: Point<f64, Physical>,
    scale: Scale<f64>,
) -> Rectangle<i32, Physical> {
    let x = |v: i32| (location.x + f64::from(v) * scale.x).round() as i32;
    let y = |v: i32| (location.y + f64::from(v) * scale.y).round() as i32;
    let (x0, y0) = (x(r.x), y(r.y));
    Rectangle::new(
        (x0, y0).into(),
        (x(r.right()) - x0, y(r.bottom()) - y0).into(),
    )
}

pub struct Backdrop {
    glass: Rc<Glass>,
    /// The node, the tree it is in and its pane, which a new pane updates in place.
    node: Option<(CustomId, TreeId, Rc<RefCell<Pane>>)>,
    pane: Option<Pane>,
    surface: Rc<Cell<Surface>>,
}

impl Backdrop {
    pub fn new(ctx: &GlassCtx) -> Backdrop {
        let glass = Rc::new(Glass::new());
        ctx.glasses.borrow_mut().push(Rc::downgrade(&glass));
        Backdrop {
            glass,
            node: None,
            pane: None,
            surface: Rc::new(Cell::new(Surface::Active)),
        }
    }

    /// Whose chrome lies over the glass, for the lift of its backdrop: a focused or unfocused
    /// frame, or the taskbar. Focused until set; takes effect with the next frame drawn.
    pub fn set_surface(&self, surface: Surface) {
        self.surface.set(surface);
    }

    pub fn set(
        &mut self,
        scene: &mut DeskScene,
        ctx: &Rc<GlassCtx>,
        tree: TreeId,
        pane: Option<Pane>,
    ) {
        if pane == self.pane {
            return;
        }
        if let (Some(p), Some((node, t, shared))) = (&pane, &self.node)
            && *t == tree
            && scene.is_alive(*node)
        {
            *shared.borrow_mut() = p.clone();
            self.pane = pane;
            return;
        }
        if let Some((node, _, _)) = self.node.take() {
            scene.destroy(node);
        }
        // Only shown glass is listed: a hidden one kept stale would ask for frames forever.
        ctx.glasses
            .borrow_mut()
            .retain(|g| !std::ptr::eq(g.as_ptr(), Rc::as_ptr(&self.glass)));
        if let Some(p) = &pane {
            // A glass kept from an earlier node blurs its first frame afresh rather than holding.
            self.glass.refresh();
            ctx.glasses.borrow_mut().push(Rc::downgrade(&self.glass));
            let shared = Rc::new(RefCell::new(p.clone()));
            let node = scene.custom_create(
                tree,
                Box::new(GlassNode {
                    ctx: ctx.clone(),
                    glass: self.glass.clone(),
                    beneath: Glass::new(),
                    pane: shared.clone(),
                    surface: self.surface.clone(),
                }),
            );
            scene.set_input(node, Input::Passthrough);
            ctx.nodes.borrow_mut().push(Placed {
                node,
                glass: Rc::downgrade(&self.glass),
                pane: shared.clone(),
            });
            self.node = Some((node, tree, shared));
        }
        self.pane = pane;
    }
}

struct GlassNode {
    ctx: Rc<GlassCtx>,
    glass: Rc<Glass>,
    /// The glass drawn into the backdrops of glass above it: its own handle, so its damage and its
    /// downscaled key stay apart from the screen's.
    beneath: Glass,
    pane: Rc<RefCell<Pane>>,
    surface: Rc<Cell<Surface>>,
}

impl CustomNode<GlesRenderer> for GlassNode {
    fn size(&self) -> Size<i32, Logical> {
        self.pane.borrow().extent()
    }

    fn in_backdrop_below(&self) -> bool {
        true
    }

    fn draws_in(&self, area: Rectangle<i32, Logical>) -> bool {
        let pane = self.pane.borrow();
        let logical = |r: Rect| Rectangle::new((r.x, r.y).into(), (r.width, r.height).into());
        let holes: Vec<Rectangle<i32, Logical>> = pane.holes.iter().map(|&h| logical(h)).collect();
        (pane.shapes.iter())
            .filter_map(|s| logical(s.rect).intersection(area))
            .any(|part| !part.subtract_rects(holes.iter().copied()).is_empty())
    }

    fn elements(
        &self,
        _renderer: &mut GlesRenderer,
        location: Point<f64, Physical>,
        scale: Scale<f64>,
        alpha: f32,
    ) -> Vec<CustomElement<GlesRenderer>> {
        let fx = self.ctx.fx.borrow();
        let Some(fx) = fx.as_ref() else {
            return Vec::new();
        };
        let pass = self.ctx.pass.get();
        let own = self.ctx.own.borrow();
        let own = own.iter().find(|(g, _)| Rc::ptr_eq(g, &self.glass));
        let shared = self.ctx.source.borrow();
        let source = own.map(|(_, s)| s).or(shared.as_ref());
        // In a backdrop beneath other glass the shapes are laid out on the output and drawn whole:
        // stepped corner rows would be half a pixel tall.
        let beneath = self.ctx.drawing_beneath.get();
        let pane = self.pane.borrow();
        let (glass, downscale, location, scale, drawn) = if let Some(d) = beneath {
            let k = f64::from(d);
            let location = location.upscale(k);
            let scale = Scale::from((scale.x * k, scale.y * k));
            (&self.beneath, d, location, scale, round_shapes(&pane))
        } else {
            let drawn = drawn_shapes(&pane, pass.settings.material, scale.x);
            (&*self.glass, 1, location, scale, drawn)
        };
        let shapes: Vec<Shape> = drawn
            .into_iter()
            .map(|(rect, radii)| Shape {
                rect: physical(rect, location, scale),
                radii: radii.map(|r| r * scale.x as f32),
            })
            .collect();
        let holes: Vec<Rectangle<i32, Physical>> = pane
            .holes
            .iter()
            .map(|&r| physical(r, location, scale))
            .collect();
        let element = glass.element(
            fx,
            &GlassFrame {
                shapes: &shapes,
                holes: &holes,
                settings: &pass.settings,
                surface: Some(self.surface.get()),
                output: pass.output,
                screen: pass.screen,
                scale: scale.x,
                alpha,
                quality: pass.quality,
                hold: pass.hold.keeps(),
                source,
                downscale,
            },
        );
        if let (Some(e), None) = (&element, beneath) {
            self.ctx.made.borrow_mut().push(e.id().clone());
        }
        element.map(CustomElement::new).into_iter().collect()
    }
}
