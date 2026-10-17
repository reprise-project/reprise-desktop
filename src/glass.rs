//! Glass's blurred backdrop on the scene: one `reprisefx` element per piece of glass, made each
//! frame by a scene node beneath the chrome it backs.
//! Glass blurs the output's backdrop with chrome left out (`GlassCtx::update_backdrop`); glass over
//! chrome or under other glass (`[glass] blur_frames`) blurs a backdrop of its own, with the glass
//! beneath it drawn in.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use reprisefx::{Glass, GlassFrame, GlassFx, Lens, Picture, Quality, Shape, Source, Update};
use reprisekit::backdrop::{corner_radius, rounded_rects};
use reprisekit::classic::frame::FrameGeometry;
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
            element::{Element, Id, RenderElement, utils::CropRenderElement},
            gles::{GlesError, GlesFrame, GlesRenderer, GlesTexture},
        },
    },
    output::Output,
    utils::{Logical, Physical, Point, Rectangle, Scale, Size, Transform},
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

/// How many updates a private backdrop outlives its last use: glass dragged past other glass
/// resumes it, its damage tracker drawing only what changed in between.
const KEEP_IDLE: u32 = 120;

/// When glass draws from its private backdrop and where that backdrop's picture blurs.
#[derive(Debug, Default)]
struct Reuse {
    /// What the glass read in the last update that drew it from the backdrop, and under which look.
    last: Option<(Vec<Rectangle<i32, Physical>>, Look)>,
    /// Updates of the output since then.
    idle: u32,
}

/// What a picture's blur depends on besides the backdrop: settings, quality, output size and scale.
type Look = (Settings, Quality, Size<i32, Physical>, u64);

impl Reuse {
    /// Where this update blurs for glass reading `needed`: everywhere once it moved or resized
    /// under the same look, so a dragged glass's backdrop blurs once, not at each new place.
    fn blurs<'a>(
        &self,
        needed: &'a [Rectangle<i32, Physical>],
        look: &Look,
    ) -> Option<&'a [Rectangle<i32, Physical>]> {
        match &self.last {
            Some((last, l)) if l == look && last != needed => None,
            _ => Some(needed),
        }
    }

    /// The part of the backdrop to draw (texels): `feed` while the glass stays put, `whole` once
    /// it moved. A new part needs no more: the tracker redraws each element whose cut changed.
    fn crop(
        &self,
        needed: &[Rectangle<i32, Physical>],
        feed: Rectangle<i32, Physical>,
        whole: Rectangle<i32, Physical>,
    ) -> Rectangle<i32, Physical> {
        match &self.last {
            Some((last, _)) if last != needed => whole,
            _ => feed,
        }
    }

    fn drawn(&mut self, needed: &[Rectangle<i32, Physical>], look: Look) {
        self.last = Some((needed.to_vec(), look));
        self.idle = 0;
    }

    /// Whether the glass drew from it in the last update.
    fn in_use(&self) -> bool {
        self.last.is_some() && self.idle == 0
    }

    /// Whether to keep it after an update that did not draw from it.
    fn kept(&mut self) -> bool {
        self.idle += 1;
        self.idle <= KEEP_IDLE
    }
}

smithay::render_elements! {
    /// What a backdrop draws: the output's cut to what its glass reads; a private one the scene
    /// below its glass, cut so while the glass stays put.
    Private<=GlesRenderer>;
    Whole=SceneElement<GlesRenderer>,
    Cut=CropRenderElement<SceneElement<GlesRenderer>>,
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
    key: (Size<i32, Physical>, u64),
    drawn: bool,
    picture: Picture,
    hidden: bool,
    /// The settings and quality of the last update that drew it.
    look: Option<(Settings, Quality)>,
    /// A held frame left it as it was: what changed since waits for the next blur.
    skipped: bool,
    /// The clear colour it was last drawn on: the tracker sees a change only when it draws.
    clear: Option<Color32F>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Pass {
    pub settings: Settings,
    pub output: Size<i32, Physical>,
    pub quality: Quality,
    pub hold: bool,
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

    pub fn flip_fx(&self) -> Option<reprisefx::FlipFx> {
        self.flip.borrow().clone()
    }

    pub fn stroke_fx(&self) -> Option<reprisefx::StrokeFx> {
        self.strokes.get()
    }

    pub fn begin(&self, output: &Output, settings: Settings, quality: Quality, hold: bool) {
        *self.source.borrow_mut() = None;
        self.own.borrow_mut().clear();
        self.made.borrow_mut().clear();
        let size = output
            .current_mode()
            .map(|m| output.current_transform().transform_size(m.size))
            .unwrap_or_default();
        self.pass.set(Pass {
            settings,
            output: size,
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
                let Some(fresh) = OutputBackdrop::new(renderer, seen, scale, &name) else {
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
            // Rain each frame) waits until glass moves over it, which moves the cut.
            let cut = reads
                .iter()
                .filter_map(|r| r.feed)
                .reduce(|a, b| a.merge(b))
                .map(|f| shrunk(f, d));
            let elements: Vec<Private> = scene
                .backdrop_elements(renderer, &view)
                .into_iter()
                .filter_map(|e| {
                    let cut = cut?;
                    CropRenderElement::from_element(e, view.scale, cut).map(Private::Cut)
                })
                .collect();
            *self.source.borrow_mut() =
                self.redraw(renderer, b, &elements, clear, output, Some(&needed));
            let pass = self.pass.get();
            b.look = Some((pass.settings, pass.quality));
            b.skipped = false;
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
        let pass = self.pass.get();
        self.nodes
            .borrow()
            .iter()
            .filter(|n| n.glass.strong_count() > 0 && scene.is_alive(n.node))
            .filter(|n| {
                !own.iter()
                    .any(|o| Weak::as_ptr(&n.glass) == Rc::as_ptr(&o.glass))
            })
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
        let mut nodes = self.nodes.borrow_mut();
        nodes.retain(|n| n.glass.strong_count() > 0 && scene.is_alive(n.node));
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
        for n in own {
            let (glass, anchor, reads) = (n.glass.clone(), n.anchor, &n.reads);
            let key = (name.clone(), Rc::as_ptr(&glass));
            if beneath
                .get(&key)
                .is_none_or(|b| b.backdrop.key != OutputBackdrop::key(seen, scale))
            {
                let Some(backdrop) = OutputBackdrop::new(renderer, seen, scale, &name) else {
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
            let whole = Rectangle::from_size(b.backdrop.key.0);
            let d = reprisefx::blur::backdrop_downscale(scale);
            let feed = reads.feed.map_or(whole, |f| shrunk(f, d));
            let crop = b.reuse.crop(&reads.needed, feed, whole);
            let view = View::new(area, scale / f64::from(d));
            self.drawing_beneath.set(Some(d));
            let elements: Vec<Private> = scene
                .elements_below(renderer, anchor, &view)
                .into_iter()
                .filter_map(|e| match crop == whole {
                    true => Some(Private::Whole(e)),
                    false => CropRenderElement::from_element(e, view.scale, crop).map(Private::Cut),
                })
                .collect();
            self.drawing_beneath.set(None);
            let look = (pass.settings, pass.quality, seen, scale.to_bits());
            let blurs = b.reuse.blurs(&reads.needed, &look);
            if let Some(source) =
                self.redraw(renderer, &mut b.backdrop, &elements, clear, output, blurs)
            {
                b.reuse.drawn(&reads.needed, look);
                self.own.borrow_mut().push((glass, source));
                used.push(key);
            }
        }
        beneath.retain(|k, b| {
            k.0 != name || used.contains(k) || (b.reuse.kept() && b.glass.strong_count() > 0)
        });
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
        let changed: Vec<Rectangle<i32, Physical>> = damage
            .iter()
            .map(|r| {
                Rectangle::new(
                    (r.loc.x * d, r.loc.y * d).into(),
                    (r.size.w * d, r.size.h * d).into(),
                )
            })
            .collect();
        let pass = self.pass.get();
        let update = Update {
            backdrop: &b.texture,
            changed: &changed,
            settings: &pass.settings,
            quality: pass.quality,
            output: output.current_transform().transform_size(mode.size),
            scale: output.current_scale().fractional_scale(),
            hold: pass.hold,
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
            && (b.hidden || (pass.hold && b.drawn && same(b)))
        {
            // A held blur shows the picture as it was, so its backdrops need no drawing either.
            b.skipped |= !b.hidden;
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
            pass.hold = false;
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
    fn key(seen: Size<i32, Physical>, scale: f64) -> (Size<i32, Physical>, u64) {
        let d = reprisefx::blur::backdrop_downscale(scale);
        let size = Size::from(((seen.w + d - 1) / d, (seen.h + d - 1) / d));
        (size, (scale / f64::from(d)).to_bits())
    }

    fn new(
        renderer: &mut GlesRenderer,
        seen: Size<i32, Physical>,
        scale: f64,
        name: &str,
    ) -> Option<OutputBackdrop> {
        let key = OutputBackdrop::key(seen, scale);
        let (size, view) = (key.0, f64::from_bits(key.1));
        let texture: GlesTexture = renderer
            .create_buffer(Fourcc::Abgr8888, (size.w, size.h).into())
            .map_err(|e| log::warn!("glass: no backdrop for {name}: {e}"))
            .ok()?;
        Some(OutputBackdrop {
            texture,
            tracker: OutputDamageTracker::new(size, view, Transform::Normal),
            key,
            drawn: false,
            picture: Picture::new(),
            hidden: false,
            look: None,
            skipped: false,
            clear: None,
        })
    }
}

#[derive(Debug, PartialEq)]
struct Reads {
    needed: Vec<Rectangle<i32, Physical>>,
    reach: Vec<Rectangle<i32, Physical>>,
    /// The bounds of `reach` grown by the blur's reach again: what a private backdrop draws while
    /// its glass stays put. None for glass off the output.
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
fn anchor(scene: &DeskScene, node: NodeId) -> Option<NodeId> {
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

pub fn flyout_pane(size: (i32, i32), border: i32, corner: &'static [i32]) -> Pane {
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
}

impl Backdrop {
    pub fn new(ctx: &GlassCtx) -> Backdrop {
        let glass = Rc::new(Glass::new());
        ctx.glasses.borrow_mut().push(Rc::downgrade(&glass));
        Backdrop {
            glass,
            node: None,
            pane: None,
        }
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
        if let Some(p) = &pane {
            let shared = Rc::new(RefCell::new(p.clone()));
            let node = scene.custom_create(
                tree,
                Box::new(GlassNode {
                    ctx: ctx.clone(),
                    glass: self.glass.clone(),
                    beneath: Glass::new(),
                    pane: shared.clone(),
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
}

impl CustomNode<GlesRenderer> for GlassNode {
    fn size(&self) -> Size<i32, Logical> {
        self.pane.borrow().extent()
    }

    fn in_backdrop_below(&self) -> bool {
        true
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
                surface: None,
                output: pass.output,
                scale: scale.x,
                alpha,
                quality: pass.quality,
                hold: pass.hold,
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

#[cfg(test)]
mod tests {
    use super::*;
    use reprisekit::classic::frame::{Chrome, Edges};
    use reprisekit::glass::CORNER;

    #[test]
    fn a_square_topped_shape_cuts_only_its_bottom_corners() {
        let pane = Pane {
            shapes: vec![PaneShape {
                rect: Rect::new(0, 0, 40, 11),
                corner: &[5, 3, 2, 1, 1, 1],
                square_bottom: false,
                square_top: true,
            }],
            holes: vec![],
        };
        let rows = drawn_shapes(&pane, Material::Frosted, 1.0);
        assert_eq!(rows.first().map(|r| r.0.x), Some(0), "the top row is whole");
        assert_eq!(rows.last().map(|r| r.0.x), Some(5), "the bottom row is cut");
        let round = drawn_shapes(&pane, Material::Molten, 1.0);
        assert_eq!(round[0].1, [0.0, 0.0, 7.0, 7.0]);
    }

    fn fast(on: bool) -> Settings {
        Settings {
            fast_drag: on,
            ..Settings::default()
        }
    }

    fn drag() -> Interaction {
        Interaction::Move {
            window: crate::window::WindowId::next(),
            dx: 0.0,
            dy: 0.0,
            origin: None,
            was_snapped: None,
            start: crate::input::MoveStart::default(),
            press: (0.0, 0.0),
            snap: None,
            armed: false,
            bar: false,
        }
    }

    #[test]
    fn a_drag_blurs_coarse_with_fast_drag_and_full_without() {
        assert_eq!(quality(&drag(), &fast(true)), Quality::Coarse);
        assert_eq!(quality(&drag(), &fast(false)), Quality::Full);
    }

    #[test]
    fn a_resize_blurs_coarse_too() {
        let resize = Interaction::Resize {
            window: crate::window::WindowId::next(),
            edges: Edges::RIGHT,
            start: Rect::new(0, 0, 100, 100),
            cx: 0.0,
            cy: 0.0,
            keyboard: None,
        };
        assert_eq!(quality(&resize, &fast(true)), Quality::Coarse);
    }

    #[test]
    fn once_the_drag_ends_the_blur_is_full() {
        assert_eq!(quality(&Interaction::Idle, &fast(true)), Quality::Full);
    }

    #[test]
    fn a_flyout_pane_lies_under_the_border_and_leaves_the_panel_clear() {
        let pane = flyout_pane((112, 40), 8, &[3, 1, 1]);
        assert_eq!(pane.shapes.len(), 1);
        assert_eq!(pane.shapes[0].rect, Rect::new(0, 0, 112, 40));
        assert_eq!(pane.shapes[0].corner, &[3, 1, 1]);
        assert_eq!(pane.holes, vec![Rect::new(8, 8, 96, 24)]);
    }

    #[test]
    fn a_frame_pane_is_the_whole_frame_round_cornered_around_its_client() {
        let m = reprisekit::theme::Theme::default().metrics;
        let g = FrameGeometry::layout(&m, 200, 100, false, Chrome::WINDOW);
        let p = frame_pane(&g, CORNER, false);
        assert_eq!(
            p.shapes,
            vec![PaneShape {
                rect: Rect::new(0, 0, g.total.0, g.total.1),
                corner: CORNER,
                square_bottom: false,
                square_top: false,
            }]
        );
        assert_eq!(p.holes, vec![g.client]);
        assert_eq!(p.extent(), Size::from(g.total));
    }

    #[test]
    fn frosted_glass_steps_its_corners_as_the_art_and_molten_rounds_them() {
        let pane = Pane {
            shapes: vec![PaneShape {
                rect: Rect::new(10, 20, 40, 30),
                corner: &[3, 1],
                square_bottom: false,
                square_top: false,
            }],
            holes: Vec::new(),
        };
        assert_eq!(
            drawn_shapes(&pane, Material::Frosted, 1.0),
            vec![
                (Rect::new(13, 20, 34, 1), [0.0; 4]),
                (Rect::new(11, 21, 38, 1), [0.0; 4]),
                (Rect::new(10, 22, 40, 26), [0.0; 4]),
                (Rect::new(11, 48, 38, 1), [0.0; 4]),
                (Rect::new(13, 49, 34, 1), [0.0; 4]),
            ]
        );
        assert_eq!(
            drawn_shapes(&pane, Material::Molten, 1.0),
            vec![(Rect::new(10, 20, 40, 30), [3.0; 4])]
        );
    }

    #[test]
    fn a_frames_glass_rounds_its_corners_above_scale_1_as_its_chrome_does() {
        let m = reprisekit::theme::Theme::default().metrics;
        let g = FrameGeometry::layout(&m, 200, 100, false, Chrome::WINDOW);
        let frame = frame_pane(&g, CORNER, false);
        let whole = vec![(Rect::new(0, 0, g.total.0, g.total.1), [7.0; 4])];
        for scale in [1.25, 1.5, 2.0, 3.0] {
            assert_eq!(
                drawn_shapes(&frame, Material::Frosted, scale),
                whole,
                "Frosted at {scale}: whole, round"
            );
            assert_eq!(drawn_shapes(&frame, Material::Smoked, scale), whole);
        }
        assert_eq!(
            drawn_shapes(&frame, Material::Frosted, 1.0).len(),
            9,
            "stepped at scale 1: 5, 3, 2 and 1 px in at each end, the middle whole"
        );
    }

    #[test]
    fn a_square_bottomed_shape_cuts_its_top_corners_only() {
        let pane = Pane {
            shapes: vec![PaneShape {
                rect: Rect::new(10, 20, 40, 30),
                corner: &[3, 1],
                square_bottom: true,
                square_top: false,
            }],
            holes: Vec::new(),
        };
        assert_eq!(
            drawn_shapes(&pane, Material::Frosted, 1.0),
            vec![
                (Rect::new(13, 20, 34, 1), [0.0; 4]),
                (Rect::new(11, 21, 38, 1), [0.0; 4]),
                (Rect::new(10, 22, 40, 28), [0.0; 4]),
            ]
        );
        assert_eq!(
            drawn_shapes(&pane, Material::Molten, 1.0),
            vec![(Rect::new(10, 20, 40, 30), [3.0, 3.0, 0.0, 0.0])]
        );
    }

    #[test]
    fn a_shape_square_at_both_ends_is_one_rect() {
        let pane = Pane {
            shapes: vec![PaneShape {
                rect: Rect::new(10, 20, 40, 30),
                corner: &[3, 1],
                square_bottom: true,
                square_top: true,
            }],
            holes: Vec::new(),
        };
        let whole = vec![(Rect::new(10, 20, 40, 30), [0.0; 4])];
        assert_eq!(drawn_shapes(&pane, Material::Frosted, 1.0), whole);
        assert_eq!(drawn_shapes(&pane, Material::Molten, 1.0), whole);
    }

    #[test]
    fn a_maximized_frame_pane_is_square() {
        let m = reprisekit::theme::Theme::default().metrics;
        let g = FrameGeometry::layout(&m, 200, 100, true, Chrome::WINDOW);
        assert!(frame_pane(&g, CORNER, true).shapes[0].corner.is_empty());
    }

    #[test]
    fn physical_rects_are_scaled_from_the_node_and_meet_without_gaps() {
        let at = Point::from((10.0, 20.0));
        assert_eq!(
            physical(Rect::new(3, 4, 5, 6), at, Scale::from(1.0)),
            Rectangle::new((13, 24).into(), (5, 6).into())
        );
        assert_eq!(
            physical(Rect::new(3, 4, 5, 6), at, Scale::from(2.0)),
            Rectangle::new((16, 28).into(), (10, 12).into())
        );
        let s = Scale::from(1.25);
        let a = physical(Rect::new(0, 0, 3, 3), at, s);
        let b = physical(Rect::new(3, 0, 3, 3), at, s);
        assert_eq!(a.loc.x + a.size.w, b.loc.x, "neighbours meet at 1.25");
    }

    fn solid(x: i32, y: i32, w: i32, h: i32, alpha: f32) -> SolidElem {
        SolidElem::new(
            Id::new(),
            Rectangle::new((x, y).into(), (w, h).into()),
            0usize,
            [0.5, 0.5, 0.5, alpha],
            smithay::backend::renderer::element::Kind::Unspecified,
        )
    }

    type SolidElem = smithay::backend::renderer::element::solid::SolidColorRenderElement;

    #[test]
    fn glass_under_an_opaque_cover_does_not_show() {
        let s = Scale::from(1.0);
        let glass = solid(10, 10, 100, 40, 0.5);
        let made = [glass.id().clone()];
        let cover = solid(0, 0, 200, 200, 1.0);
        assert!(!shows_glass(&made, &[cover, glass], s));
    }

    #[test]
    fn glass_shows_past_a_cover_that_misses_part_of_it() {
        let s = Scale::from(1.0);
        let glass = solid(10, 10, 100, 40, 0.5);
        let made = [glass.id().clone()];
        let part = solid(0, 0, 100, 200, 1.0);
        let see_through = solid(0, 0, 200, 200, 0.9);
        assert!(shows_glass(&made, &[part, glass.clone()], s));
        assert!(
            shows_glass(&made, &[see_through, glass.clone()], s),
            "a translucent cover hides nothing"
        );
        let below = solid(0, 0, 200, 200, 1.0);
        assert!(shows_glass(&made, &[glass, below], s), "a cover beneath");
    }

    #[test]
    fn a_frames_glass_reads_its_ring_and_not_its_client() {
        let m = reprisekit::theme::Theme::default().metrics;
        let g = FrameGeometry::layout(&m, 400, 300, false, Chrome::WINDOW);
        let pass = Pass {
            settings: Settings::default(),
            ..Pass::default()
        };
        let at = Point::from((100.0, 50.0));
        let reads = Reads::of(
            &frame_pane(&g, CORNER, false),
            at,
            (1000, 800).into(),
            1.0,
            &pass,
        );
        let has = |rects: &[Rectangle<i32, Physical>], x: i32, y: i32| {
            rects.iter().any(|r| r.contains((x, y)))
        };
        let c = g.client;
        let (cx, cy) = (100 + c.x + c.width / 2, 50 + c.y + c.height / 2);
        assert!(!has(&reads.needed, cx, cy), "the client's middle");
        assert!(
            !has(&reads.reach, cx, cy),
            "a change there shows in no glass"
        );
        assert!(has(&reads.needed, 100 + g.total.0 / 2, 52), "the caption");
    }

    #[test]
    fn a_frames_glass_is_fed_from_past_what_its_blur_reads() {
        let m = reprisekit::theme::Theme::default().metrics;
        let g = FrameGeometry::layout(&m, 400, 300, false, Chrome::WINDOW);
        let pane = frame_pane(&g, CORNER, false);
        for material in [Material::Frosted, Material::Smoked, Material::Molten] {
            for quality in [Quality::Full, Quality::Coarse] {
                let pass = Pass {
                    settings: Settings {
                        material,
                        ..Settings::default()
                    },
                    quality,
                    ..Pass::default()
                };
                let reads = Reads::of(&pane, (100.0, 50.0).into(), (1000, 800).into(), 1.0, &pass);
                let feed = reads.feed.unwrap();
                let s = &pass.settings;
                let r = reprisefx::blur::reach(s.blur, material, quality, 1.0);
                for n in &reads.needed {
                    let read = Rectangle::new(
                        (n.loc.x - r, n.loc.y - r).into(),
                        (n.size.w + 2 * r, n.size.h + 2 * r).into(),
                    )
                    .intersection(Rectangle::from_size((1000, 800).into()))
                    .unwrap();
                    assert!(
                        feed.contains_rect(read),
                        "{material:?} {quality:?}: {read:?}"
                    );
                }
            }
        }
    }

    fn test_output() -> Output {
        let o = Output::new(
            "test".into(),
            smithay::output::PhysicalProperties {
                size: (0, 0).into(),
                subpixel: smithay::output::Subpixel::Unknown,
                make: "Reprise".into(),
                model: "test".into(),
                serial_number: String::new(),
            },
        );
        let mode = smithay::output::Mode {
            size: (200, 120).into(),
            refresh: 60_000,
        };
        o.change_current_state(
            Some(mode),
            Some(Transform::Normal),
            Some(smithay::output::Scale::Integer(1)),
            Some((0, 0).into()),
        );
        o
    }

    fn red_over_a_caption(blur_frames: bool) -> Option<u32> {
        let mut renderer = match reprisefx::headless::renderer() {
            Ok(r) => r,
            Err(e) => {
                eprintln!("skipped: {e}");
                return None;
            }
        };
        let ctx = GlassCtx::new();
        ctx.set_renderer(&mut renderer);
        let mut scene = DeskScene::new();
        let root = scene.root();
        let desktop = scene.tree_create(root);
        scene.rect_create(desktop, (200, 120), [1.0; 4]);
        let windows = scene.tree_create(root);
        let lower = scene.tree_create(windows);
        let chrome = scene.tree_create(lower);
        leave_out(&mut scene, chrome);
        scene.rect_create(chrome, (200, 40), [0.0, 0.0, 0.0, 1.0]);
        let upper = scene.tree_create(windows);
        scene.set_position(upper, 40, 16);
        let chrome = scene.tree_create(upper);
        leave_out(&mut scene, chrome);
        let mut glass = Backdrop::new(&ctx);
        glass.set(&mut scene, &ctx, chrome, Some(sheet_pane((120, 80), &[])));
        let output = test_output();
        let settings = Settings {
            blur_frames,
            streaks: false,
            ..Settings::default()
        };
        let clear = Color32F::from([0.0, 0.0, 0.0, 1.0]);
        ctx.begin(&output, settings, Quality::Full, false);
        ctx.update_backdrop(&mut renderer, &scene, &output, clear, true);
        let elements = scene.render_elements_for(&mut renderer, &output);
        let mut target =
            crate::offscreen::Target::new(&mut renderer, (200, 120).into(), 1.0).unwrap();
        target.render(&mut renderer, &elements).unwrap();
        let canvas = target.read(&mut renderer).unwrap();
        Some((canvas.pixels[24 * 200 + 100] >> 16) & 0xFF)
    }

    #[test]
    fn glass_over_another_windows_caption_shows_it_blurred_unless_blur_frames_is_off() {
        let Some(on) = red_over_a_caption(true) else {
            return;
        };
        assert!(on < 80, "the black caption shows through the glass: {on}");
        let off = red_over_a_caption(false).unwrap();
        assert!(
            off > 230,
            "off, glass shows the white under the caption: {off}"
        );
    }

    fn test_output_of(w: i32, h: i32) -> Output {
        let o = test_output();
        let mode = smithay::output::Mode {
            size: (w, h).into(),
            refresh: 60_000,
        };
        o.change_current_state(Some(mode), None, None, None);
        o
    }

    /// A window at `at` whose left-out chrome holds a `size` sheet of glass, over a black rect of
    /// `under` when given (glass on screen covers it).
    fn glass_window(
        scene: &mut DeskScene,
        ctx: &Rc<GlassCtx>,
        windows: TreeId,
        at: (i32, i32),
        size: (i32, i32),
        under: Option<(i32, i32)>,
    ) -> (TreeId, Backdrop) {
        let window = scene.tree_create(windows);
        scene.set_position(window, at.0, at.1);
        let chrome = scene.tree_create(window);
        leave_out(scene, chrome);
        if let Some(under) = under {
            scene.rect_create(chrome, under, [0.0, 0.0, 0.0, 1.0]);
        }
        let mut glass = Backdrop::new(ctx);
        glass.set(scene, ctx, chrome, Some(sheet_pane(size, &[])));
        (window, glass)
    }

    /// The red channel of `points` on a 320x200 output after one frame of `scene`.
    fn red_at(
        renderer: &mut GlesRenderer,
        ctx: &GlassCtx,
        scene: &DeskScene,
        material: Material,
        points: &[(i32, i32)],
    ) -> Vec<u32> {
        let output = test_output_of(320, 200);
        let settings = Settings {
            material,
            streaks: false,
            ..Settings::default()
        };
        let clear = Color32F::from([0.0, 0.0, 0.0, 1.0]);
        ctx.begin(&output, settings, Quality::Full, false);
        ctx.update_backdrop(renderer, scene, &output, clear, true);
        let elements = scene.render_elements_for(renderer, &output);
        let mut target = crate::offscreen::Target::new(renderer, (320, 200).into(), 1.0).unwrap();
        target.render(renderer, &elements).unwrap();
        let canvas = target.read(renderer).unwrap();
        points
            .iter()
            .map(|&(x, y)| (canvas.pixels[(y * 320 + x) as usize] >> 16) & 0xFF)
            .collect()
    }

    #[test]
    fn glass_over_other_glass_shows_that_glass_and_not_what_it_covers() {
        for material in [Material::Frosted, Material::Smoked, Material::Molten] {
            let Ok(mut renderer) = reprisefx::headless::renderer() else {
                eprintln!("skipped: no renderer");
                return;
            };
            let ctx = GlassCtx::new();
            ctx.set_renderer(&mut renderer);
            let mut scene = DeskScene::new();
            let root = scene.root();
            let desktop = scene.tree_create(root);
            scene.rect_create(desktop, (320, 200), [1.0; 4]);
            let windows = scene.tree_create(root);
            let lower = Some((200, 120));
            let _lower = glass_window(&mut scene, &ctx, windows, (20, 20), (200, 120), lower);
            let _upper = glass_window(&mut scene, &ctx, windows, (120, 80), (180, 100), None);
            let [lower_alone, both] = red_at(
                &mut renderer,
                &ctx,
                &scene,
                material,
                &[(60, 50), (170, 110)],
            )[..] else {
                unreachable!()
            };
            assert!(
                lower_alone > 230,
                "{material:?}: the lower glass shows the white desktop"
            );
            assert!(
                both > 230,
                "{material:?}: through the upper glass the lower glass shows, not the black under it: {both}"
            );
        }
    }

    #[test]
    fn the_outputs_blur_reads_only_for_glass_drawn_from_it() {
        let ctx = GlassCtx::new();
        let mut scene = DeskScene::new();
        let root = scene.root();
        let desktop = scene.tree_create(root);
        scene.rect_create(desktop, (320, 200), [1.0; 4]);
        let windows = scene.tree_create(root);
        let _lower = glass_window(&mut scene, &ctx, windows, (20, 20), (120, 80), None);
        let (area, seen) = (Rectangle::from_size((320, 200).into()), (320, 200).into());
        ctx.pass.set(Pass {
            settings: Settings {
                blur_frames: true,
                ..Settings::default()
            },
            ..Pass::default()
        });
        let own = ctx.drawn_beneath(&scene, area, seen, 1.0);
        assert!(own.is_empty(), "glass alone draws from the output's blur");
        let alone = ctx.reads_on(&scene, area, seen, 1.0, &own);
        assert!(!alone.is_empty());
        let (upper, _upper) = glass_window(&mut scene, &ctx, windows, (100, 60), (120, 80), None);
        let own = ctx.drawn_beneath(&scene, area, seen, 1.0);
        assert_eq!(
            own.len(),
            2,
            "overlapping glass draws from backdrops of its own"
        );
        assert_eq!(ctx.reads_on(&scene, area, seen, 1.0, &own), Vec::new());
        scene.destroy(upper);
        let own = ctx.drawn_beneath(&scene, area, seen, 1.0);
        assert!(own.is_empty(), "alone again");
        assert_eq!(ctx.reads_on(&scene, area, seen, 1.0, &own), alone);
    }

    #[test]
    fn glass_beneath_other_glass_blurs_a_backdrop_without_the_windows_above_it() {
        let Ok(mut renderer) = reprisefx::headless::renderer() else {
            eprintln!("skipped: no renderer");
            return;
        };
        let ctx = GlassCtx::new();
        ctx.set_renderer(&mut renderer);
        let mut scene = DeskScene::new();
        let root = scene.root();
        let desktop = scene.tree_create(root);
        scene.rect_create(desktop, (320, 200), [1.0; 4]);
        let windows = scene.tree_create(root);
        let (_, lower) = glass_window(&mut scene, &ctx, windows, (20, 20), (120, 80), None);
        let (upper, _upper) = glass_window(&mut scene, &ctx, windows, (100, 60), (120, 80), None);
        let lower = Rc::as_ptr(&lower.glass);
        let own = |ctx: &GlassCtx| {
            ctx.beneath
                .borrow()
                .get(&("test".into(), lower))
                .is_some_and(|b| b.reuse.in_use())
        };
        red_at(&mut renderer, &ctx, &scene, Material::Frosted, &[]);
        assert!(own(&ctx), "under the upper glass");
        scene.set_position(upper, 200, 120);
        red_at(&mut renderer, &ctx, &scene, Material::Frosted, &[]);
        assert!(
            !own(&ctx),
            "past the upper glass's reach the output's backdrop does"
        );
    }

    #[test]
    fn glass_beneath_other_glass_draws_its_own_backdrop_into_it() {
        let Ok(mut renderer) = reprisefx::headless::renderer() else {
            eprintln!("skipped: no renderer");
            return;
        };
        let ctx = GlassCtx::new();
        ctx.set_renderer(&mut renderer);
        let mut scene = DeskScene::new();
        let root = scene.root();
        let desktop = scene.tree_create(root);
        scene.rect_create(desktop, (320, 200), [1.0; 4]);
        let windows = scene.tree_create(root);
        let bottom = scene.tree_create(windows);
        let caption = scene.tree_create(bottom);
        leave_out(&mut scene, caption);
        scene.rect_create(caption, (320, 70), [0.0, 0.0, 0.0, 1.0]);
        let middle = scene.tree_create(windows);
        let top = scene.tree_create(windows);
        // The top glass is made first: its backdrop still waits for the middle glass's.
        let _top = glass_window(&mut scene, &ctx, top, (120, 20), (180, 100), None);
        let _middle = glass_window(&mut scene, &ctx, middle, (20, 10), (200, 120), None);
        let red = red_at(&mut renderer, &ctx, &scene, Material::Frosted, &[(170, 35)]);
        assert!(
            red[0] < 80,
            "the middle glass shows the black caption beneath it, through the top glass too: {}",
            red[0]
        );
    }

    #[test]
    fn a_change_away_from_glass_is_blurred_once_glass_reads_it() {
        let Ok(mut renderer) = reprisefx::headless::renderer() else {
            eprintln!("skipped: no renderer");
            return;
        };
        let scene_with = |ctx: &Rc<GlassCtx>| {
            let mut scene = DeskScene::new();
            let root = scene.root();
            let desktop = scene.tree_create(root);
            scene.rect_create(desktop, (320, 200), [1.0; 4]);
            let spot = scene.tree_create(desktop);
            scene.set_position(spot, 240, 20);
            scene.rect_create(spot, (60, 40), [0.0, 0.0, 0.0, 1.0]);
            scene.set_enabled(spot, false);
            let windows = scene.tree_create(root);
            let (window, glass) =
                glass_window(&mut scene, ctx, windows, (10, 120), (100, 60), None);
            (scene, spot, window, glass)
        };
        let points: Vec<(i32, i32)> = (0..10).map(|i| (226 + i * 9, 30)).collect();
        let ctx = GlassCtx::new();
        ctx.set_renderer(&mut renderer);
        let (mut scene, spot, window, _glass) = scene_with(&ctx);
        red_at(&mut renderer, &ctx, &scene, Material::Frosted, &[]);
        scene.set_enabled(spot, true);
        red_at(&mut renderer, &ctx, &scene, Material::Frosted, &[]);
        scene.set_position(window, 210, 0);
        let moved = red_at(&mut renderer, &ctx, &scene, Material::Frosted, &points);
        let fresh_ctx = GlassCtx::new();
        fresh_ctx.set_renderer(&mut renderer);
        let (mut fresh, spot, window, _glass) = scene_with(&fresh_ctx);
        fresh.set_enabled(spot, true);
        fresh.set_position(window, 210, 0);
        let fresh = red_at(
            &mut renderer,
            &fresh_ctx,
            &fresh,
            Material::Frosted,
            &points,
        );
        assert!(
            moved.iter().any(|&r| r < 200),
            "the spot shows blurred: {moved:?}"
        );
        assert_eq!(
            moved, fresh,
            "as blurred as a first frame of the same scene"
        );
    }

    /// The red channel of the output backdrop's texel at output pixel `at`.
    fn backdrop_red(renderer: &mut GlesRenderer, ctx: &GlassCtx, at: (i32, i32)) -> u32 {
        use smithay::backend::renderer::{ExportMem, Texture};
        let mut t = ctx.backdrops.borrow()["test"].texture.clone();
        let size = t.size();
        let fb = renderer.bind(&mut t).unwrap();
        let mapping = renderer
            .copy_framebuffer(&fb, Rectangle::from_size(size), Fourcc::Abgr8888)
            .unwrap();
        drop(fb);
        let bytes = renderer.map_texture(&mapping).unwrap();
        let (x, y) = (at.0 / 2, at.1 / 2);
        u32::from(bytes[((y * size.w + x) * 4) as usize])
    }

    #[test]
    fn the_outputs_backdrop_draws_only_what_its_glass_reads() {
        let Ok(mut renderer) = reprisefx::headless::renderer() else {
            eprintln!("skipped: no renderer");
            return;
        };
        let ctx = GlassCtx::new();
        ctx.set_renderer(&mut renderer);
        let mut scene = DeskScene::new();
        let root = scene.root();
        let desktop = scene.tree_create(root);
        scene.rect_create(desktop, (320, 200), [1.0; 4]);
        let windows = scene.tree_create(root);
        let (window, _glass) = glass_window(&mut scene, &ctx, windows, (10, 10), (80, 50), None);
        red_at(&mut renderer, &ctx, &scene, Material::Frosted, &[]);
        assert!(
            backdrop_red(&mut renderer, &ctx, (50, 30)) > 230,
            "under the glass"
        );
        assert!(
            backdrop_red(&mut renderer, &ctx, (300, 180)) < 20,
            "far from the glass the backdrop is left clear"
        );
        scene.set_position(window, 220, 130);
        red_at(&mut renderer, &ctx, &scene, Material::Frosted, &[]);
        assert!(
            backdrop_red(&mut renderer, &ctx, (300, 180)) > 230,
            "glass moved there draws it"
        );
    }

    #[test]
    fn a_held_frame_leaves_the_backdrop_undrawn_until_the_next_blur() {
        let Ok(mut renderer) = reprisefx::headless::renderer() else {
            eprintln!("skipped: no renderer");
            return;
        };
        let ctx = GlassCtx::new();
        ctx.set_renderer(&mut renderer);
        let mut scene = DeskScene::new();
        let root = scene.root();
        let desktop = scene.tree_create(root);
        scene.rect_create(desktop, (320, 200), [1.0; 4]);
        let spot = scene.tree_create(desktop);
        scene.set_position(spot, 40, 40);
        scene.rect_create(spot, (60, 40), [0.0, 0.0, 0.0, 1.0]);
        scene.set_enabled(spot, false);
        let windows = scene.tree_create(root);
        let _glass = glass_window(&mut scene, &ctx, windows, (20, 20), (120, 80), None);
        let output = test_output_of(320, 200);
        let settings = Settings {
            streaks: false,
            ..Settings::default()
        };
        let clear = Color32F::from([0.0, 0.0, 0.0, 1.0]);
        let frame = |renderer: &mut GlesRenderer, scene: &DeskScene, hold: bool| {
            ctx.begin(&output, settings, Quality::Full, hold);
            ctx.update_screen_backdrop(renderer, scene, &output, clear, true);
        };
        frame(&mut renderer, &scene, false);
        scene.set_enabled(spot, true);
        frame(&mut renderer, &scene, true);
        assert!(
            backdrop_red(&mut renderer, &ctx, (70, 60)) > 230,
            "the held frame drew nothing into the backdrop"
        );
        assert!(ctx.any_stale(), "the skipped change waits for a blur");
        assert!(ctx.refresh_stale());
        frame(&mut renderer, &scene, false);
        assert!(
            backdrop_red(&mut renderer, &ctx, (70, 60)) < 20,
            "the next free frame draws the spot"
        );
        assert!(!ctx.any_stale());
    }

    #[test]
    fn glass_missing_from_the_elements_counts_as_shown() {
        let s = Scale::from(1.0);
        let glass = solid(10, 10, 100, 40, 0.5);
        let made = [glass.id().clone()];
        assert!(shows_glass(&made, &[solid(0, 0, 200, 200, 1.0)], s));
        assert!(
            !shows_glass(&[], &[solid(0, 0, 200, 200, 1.0)], s),
            "no glass at all"
        );
    }

    fn rects(r: &[(i32, i32, i32, i32)]) -> Vec<Rectangle<i32, Physical>> {
        r.iter()
            .map(|&(x, y, w, h)| Rectangle::new((x, y).into(), (w, h).into()))
            .collect()
    }

    #[test]
    fn a_private_backdrop_draws_what_feeds_its_glass_while_it_stays_put() {
        let here = rects(&[(10, 10, 100, 60)]);
        let there = rects(&[(30, 10, 100, 60)]);
        let feed = Rectangle::new((0, 0).into(), (70, 50).into());
        let whole = Rectangle::from_size((320, 100).into());
        let look = look(Material::Frosted);
        let mut reuse = Reuse::default();
        assert_eq!(reuse.crop(&here, feed, whole), feed, "first drawn");
        reuse.drawn(&here, look);
        assert_eq!(reuse.crop(&here, feed, whole), feed, "put");
        assert_eq!(reuse.crop(&there, feed, whole), whole, "moved: all of it");
        reuse.drawn(&there, look);
        assert_eq!(reuse.crop(&there, feed, whole), feed, "put again");
    }

    fn look(material: Material) -> Look {
        let settings = Settings {
            material,
            ..Settings::default()
        };
        (settings, Quality::Full, (640, 200).into(), 1f64.to_bits())
    }

    #[test]
    fn a_private_backdrop_blurs_where_its_glass_reads_until_the_glass_moves() {
        let here = rects(&[(10, 10, 100, 60)]);
        let there = rects(&[(30, 10, 100, 60)]);
        let look = look(Material::Frosted);
        let mut reuse = Reuse::default();
        assert_eq!(reuse.blurs(&here, &look), Some(&here[..]), "first drawn");
        reuse.drawn(&here, look);
        assert_eq!(reuse.blurs(&here, &look), Some(&here[..]), "put");
        assert_eq!(reuse.blurs(&there, &look), None, "moved: all of it, once");
        reuse.drawn(&there, look);
        assert_eq!(reuse.blurs(&there, &look), Some(&there[..]));
    }

    #[test]
    fn a_new_look_blurs_where_the_glass_reads_even_as_it_moves() {
        let here = rects(&[(10, 10, 100, 60)]);
        let there = rects(&[(30, 10, 100, 60)]);
        let mut reuse = Reuse::default();
        reuse.drawn(&here, look(Material::Molten));
        let frosted = look(Material::Frosted);
        assert_eq!(reuse.blurs(&there, &frosted), Some(&there[..]));
        let coarse = (frosted.0, Quality::Coarse, frosted.2, frosted.3);
        assert_eq!(reuse.blurs(&there, &coarse), Some(&there[..]));
    }

    #[test]
    fn a_private_backdrop_outlives_its_use_for_a_while_without_being_drawn_from() {
        let here = rects(&[(10, 10, 100, 60)]);
        let look = look(Material::Frosted);
        let mut reuse = Reuse::default();
        assert!(!reuse.in_use(), "never drawn");
        reuse.drawn(&here, look);
        assert!(reuse.in_use());
        assert!(reuse.kept());
        assert!(!reuse.in_use(), "unused this update");
        for _ in 1..KEEP_IDLE {
            assert!(reuse.kept());
        }
        assert!(!reuse.kept(), "dropped after {KEEP_IDLE} updates");
        reuse.drawn(&here, look);
        assert!(
            reuse.in_use() && reuse.kept(),
            "a use starts the count again"
        );
    }

    /// What `glass_over_glass_draws_each_frame_as_a_fresh_start_would` lays out: a lower window with a
    /// caption and a sheet of glass, an upper window with glass, over a desktop with a stripe.
    #[derive(Clone, Copy)]
    struct Layout {
        stripe: [f32; 4],
        /// Over the bare clear colour right of the desktop.
        patch: (i32, i32),
        lower: (i32, i32),
        upper: (i32, i32),
        upper_size: (i32, i32),
        lower_on_top: bool,
        material: Material,
    }

    type Step = (&'static str, fn(&mut Layout));

    struct Laid {
        scene: DeskScene,
        ctx: Rc<GlassCtx>,
        stripe: reprisescene::RectId,
        patch: reprisescene::RectId,
        lower: (TreeId, Backdrop, TreeId),
        upper: (TreeId, Backdrop, TreeId),
    }

    fn lay(l: &Layout, renderer: &mut GlesRenderer) -> Laid {
        let ctx = GlassCtx::new();
        ctx.set_renderer(renderer);
        let mut scene = DeskScene::new();
        let root = scene.root();
        let desktop = scene.tree_create(root);
        scene.rect_create(desktop, (430, 200), [1.0; 4]);
        let stripe = scene.rect_create(desktop, (640, 30), l.stripe);
        scene.set_position(stripe, 0, 60);
        let patch = scene.rect_create(desktop, (90, 30), [0.9, 0.8, 0.1, 1.0]);
        scene.set_position(patch, l.patch.0, l.patch.1);
        let windows = scene.tree_create(root);
        let mut window = |at: (i32, i32), size: (i32, i32), caption: bool| {
            let window = scene.tree_create(windows);
            scene.set_position(window, at.0, at.1);
            let chrome = scene.tree_create(window);
            leave_out(&mut scene, chrome);
            if caption {
                scene.rect_create(chrome, (size.0, 20), [0.0, 0.0, 0.3, 1.0]);
            }
            let mut glass = Backdrop::new(&ctx);
            glass.set(&mut scene, &ctx, chrome, Some(sheet_pane(size, &[])));
            (window, glass, chrome)
        };
        let lower = window(l.lower, (150, 110), true);
        let upper = window(l.upper, l.upper_size, false);
        if l.lower_on_top {
            scene.raise_to_top(lower.0);
        }
        Laid {
            scene,
            ctx,
            stripe,
            patch,
            lower,
            upper,
        }
    }

    fn frame(renderer: &mut GlesRenderer, laid: &Laid, material: Material) -> Vec<u32> {
        let output = test_output_of(640, 200);
        let settings = Settings {
            material,
            ..Settings::default()
        };
        let clear = Color32F::from([0.0, 0.0, 0.0, 1.0]);
        laid.ctx.begin(&output, settings, Quality::Full, false);
        laid.ctx
            .update_backdrop(renderer, &laid.scene, &output, clear, true);
        let elements = laid.scene.render_elements_for(renderer, &output);
        let mut target = crate::offscreen::Target::new(renderer, (640, 200).into(), 1.0).unwrap();
        target.render(renderer, &elements).unwrap();
        target.read(renderer).unwrap().pixels
    }

    #[test]
    fn glass_over_glass_draws_each_frame_as_a_fresh_start_would() {
        let Ok(mut renderer) = reprisefx::headless::renderer() else {
            eprintln!("skipped: no renderer");
            return;
        };
        let mut l = Layout {
            stripe: [0.8, 0.1, 0.1, 1.0],
            patch: (520, 120),
            lower: (20, 20),
            upper: (100, 50),
            upper_size: (150, 110),
            lower_on_top: false,
            material: Material::Frosted,
        };
        let mut live = lay(&l, &mut renderer);
        let steps: [Step; 13] = [
            ("as laid", |_| {}),
            ("the upper dragged", |l| l.upper = (110, 56)),
            ("dragged on", |l| l.upper = (124, 64)),
            ("off the lower glass", |l| l.upper = (460, 60)),
            ("the stripe beneath changed while apart", |l| {
                l.stripe = [0.1, 0.7, 0.1, 1.0]
            }),
            ("back over it", |l| l.upper = (110, 56)),
            ("the lower raised", |l| l.lower_on_top = true),
            ("the upper resized", |l| l.upper_size = (120, 90)),
            ("Molten", |l| l.material = Material::Molten),
            ("the lower moved", |l| l.lower = (60, 30)),
            ("Frosted again", |l| l.material = Material::Frosted),
            ("a patch far from both moved", |l| l.patch = (540, 10)),
            ("both moved where the patch was", |l| {
                l.lower = (470, 100);
                l.upper = (490, 110);
            }),
        ];
        for (what, step) in steps {
            step(&mut l);
            let s = &mut live.scene;
            s.rect_set_color(live.stripe, l.stripe);
            s.set_position(live.patch, l.patch.0, l.patch.1);
            s.set_position(live.lower.0, l.lower.0, l.lower.1);
            s.set_position(live.upper.0, l.upper.0, l.upper.1);
            let top = if l.lower_on_top {
                live.lower.0
            } else {
                live.upper.0
            };
            s.raise_to_top(top);
            let pane = Some(sheet_pane(l.upper_size, &[]));
            live.upper.1.set(s, &live.ctx, live.upper.2, pane);
            let drawn = frame(&mut renderer, &live, l.material);
            let start = lay(&l, &mut renderer);
            let fresh = frame(&mut renderer, &start, l.material);
            if what.ends_with("apart") {
                let kept = live.ctx.beneath.borrow();
                assert!(
                    kept.len() == 2 && kept.values().all(|b| !b.reuse.in_use()),
                    "both private backdrops kept, unused"
                );
            }
            let differ = drawn.iter().zip(&fresh).filter(|(a, b)| a != b).count();
            assert_eq!(differ, 0, "{what}: pixels unlike a fresh start's");
        }
    }

    #[test]
    fn what_lies_in_the_overlay_layer_does_not_blur_into_glass_beneath_it() {
        let Ok(mut renderer) = reprisefx::headless::renderer() else {
            eprintln!("skipped: no renderer");
            return;
        };
        let ctx = GlassCtx::new();
        ctx.set_renderer(&mut renderer);
        let mut scene = DeskScene::new();
        let layers = crate::compositor::Layers::new(&mut scene);
        scene.rect_create(layers.background, (320, 200), [1.0; 4]);
        let bar = scene.tree_create(layers.taskbar);
        scene.set_position(bar, 20, 20);
        leave_out(&mut scene, bar);
        let mut glass = Backdrop::new(&ctx);
        glass.set(&mut scene, &ctx, bar, Some(sheet_pane((280, 120), &[])));
        let ghost = scene.rect_create(layers.overlay, (40, 40), [0.0, 0.0, 0.0, 1.0]);
        scene.set_position(ghost, 140, 60);
        let red = red_at(
            &mut renderer,
            &ctx,
            &scene,
            Material::Frosted,
            &[(160, 80), (134, 80), (40, 80)],
        );
        assert_eq!(red[0], 0, "the drag image shows");
        assert_eq!(
            red[1], red[2],
            "no halo: the glass beside it reads as the glass far from it"
        );
    }

    /// The red channel just left of a black|white edge under a sheet of glass in a tree at
    /// `opacity`, over another window's glass when `over_glass`, and how much glass the frame drew.
    fn faded_edge(material: Material, opacity: f32, over_glass: bool) -> Option<(u32, usize)> {
        let mut renderer = reprisefx::headless::renderer().ok()?;
        let ctx = GlassCtx::new();
        ctx.set_renderer(&mut renderer);
        let mut scene = DeskScene::new();
        let root = scene.root();
        let desktop = scene.tree_create(root);
        scene.rect_create(desktop, (320, 200), [1.0; 4]);
        scene.rect_create(desktop, (160, 200), [0.0, 0.0, 0.0, 1.0]);
        let windows = scene.tree_create(root);
        let _under =
            over_glass.then(|| glass_window(&mut scene, &ctx, windows, (60, 20), (200, 160), None));
        let sheet = scene.tree_create(root);
        scene.set_position(sheet, 100, 50);
        scene.set_opacity(sheet, opacity);
        let chrome = scene.tree_create(sheet);
        leave_out(&mut scene, chrome);
        let mut glass = Backdrop::new(&ctx);
        glass.set(&mut scene, &ctx, chrome, Some(sheet_pane((120, 100), &[])));
        let red = red_at(&mut renderer, &ctx, &scene, material, &[(157, 100)])[0];
        Some((red, ctx.made.borrow().len()))
    }

    #[test]
    fn glass_in_a_faded_tree_blurs_in_proportion_to_the_trees_opacity() {
        for material in [Material::Frosted, Material::Smoked, Material::Molten] {
            for over_glass in [false, true] {
                let Some((blurred, _)) = faded_edge(material, 1.0, over_glass) else {
                    eprintln!("skipped: no renderer");
                    return;
                };
                let (half, _) = faded_edge(material, 0.5, over_glass).unwrap();
                let (bare, _) = faded_edge(material, 0.0, over_glass).unwrap();
                let what = format!("{material:?}, over glass {over_glass}");
                assert!(
                    blurred > bare,
                    "{what}: the glass blurs white into the black"
                );
                let mid = (blurred + bare) / 2;
                assert!(
                    half.abs_diff(mid) <= 2,
                    "{what}: at 0.5 the edge is halfway, {mid}: {half}"
                );
            }
        }
    }

    #[test]
    fn glass_in_a_tree_at_opacity_0_makes_no_element() {
        let Some((red, made)) = faded_edge(Material::Molten, 0.0, false) else {
            eprintln!("skipped: no renderer");
            return;
        };
        assert_eq!(made, 0, "no glass element");
        assert_eq!(red, 0, "the black shows unblurred");
    }
}
