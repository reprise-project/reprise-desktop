//! Glass's blurred backdrop on the scene: one `reprisefx` element per piece of glass, made each
//! frame by a scene node beneath the chrome it backs.
//! Glass blurs the output's backdrop with chrome left out (`GlassCtx::update_backdrop`); glass over
//! chrome (`[glass] blur_frames`) blurs a backdrop of its own.

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
            Bind, Color32F, Offscreen,
            damage::OutputDamageTracker,
            element::{Element, Id},
            gles::{GlesRenderer, GlesTexture},
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
    beneath: RefCell<HashMap<(String, CustomId), Beneath>>,
}

struct Placed {
    node: CustomId,
    glass: Weak<Glass>,
    pane: Pane,
}

struct Beneath {
    backdrop: OutputBackdrop,
    glass: Weak<Glass>,
}

struct OutputBackdrop {
    texture: GlesTexture,
    tracker: OutputDamageTracker,
    key: (Size<i32, Physical>, u64),
    drawn: bool,
    picture: Picture,
    hidden: bool,
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
            let elements = scene.backdrop_elements(renderer, &View::new(area, scale / 2.0));
            *self.source.borrow_mut() = self.redraw(renderer, b, &elements, clear, output, None);
        }
        if self.pass.get().settings.blur_frames {
            self.update_beneath(renderer, scene, output, clear);
        } else {
            self.beneath.borrow_mut().retain(|(o, _), _| *o != name);
        }
    }

    fn update_beneath(
        &self,
        renderer: &mut GlesRenderer,
        scene: &DeskScene,
        output: &Output,
        clear: Color32F,
    ) {
        let (Some(area), Some(mode)) = (reprisescene::output_rect(output), output.current_mode())
        else {
            return;
        };
        let seen = output.current_transform().transform_size(mode.size);
        let scale = output.current_scale().fractional_scale();
        let name = output.name();
        let pass = self.pass.get();
        let mut nodes = self.nodes.borrow_mut();
        nodes.retain(|n| n.glass.strong_count() > 0 && scene.is_alive(n.node));
        let mut beneath = self.beneath.borrow_mut();
        let mut used = Vec::new();
        for n in nodes.iter() {
            let (Some(glass), Some((at, true)), Some(anchor)) = (
                n.glass.upgrade(),
                scene.coords(n.node),
                anchor(scene, n.node.into()),
            ) else {
                continue;
            };
            let at = (at - area.loc).to_f64().to_physical(scale);
            let reads = Reads::of(&n.pane, at, seen, scale, &pass);
            if !reads
                .reach
                .iter()
                .any(|r| scene.left_out_below(anchor, layout(*r, area.loc, scale)))
            {
                continue;
            }
            let key = (name.clone(), n.node);
            if beneath
                .get(&key)
                .is_none_or(|b| b.backdrop.key != OutputBackdrop::key(seen, scale))
            {
                let Some(backdrop) = OutputBackdrop::new(renderer, seen, scale, &name) else {
                    continue;
                };
                let glass = Rc::downgrade(&glass);
                beneath.insert(key.clone(), Beneath { backdrop, glass });
            }
            let Some(b) = beneath.get_mut(&key).map(|b| &mut b.backdrop) else {
                continue;
            };
            let view = View::new(area, scale / 2.0);
            let elements = scene.elements_below(renderer, anchor, &view);
            if let Some(source) =
                self.redraw(renderer, b, &elements, clear, output, Some(&reads.needed))
            {
                self.own.borrow_mut().push((glass, source));
                used.push(key);
            }
        }
        beneath.retain(|k, _| k.0 != name || used.contains(k));
    }

    fn redraw(
        &self,
        renderer: &mut GlesRenderer,
        b: &mut OutputBackdrop,
        elements: &[SceneElement<GlesRenderer>],
        clear: Color32F,
        output: &Output,
        needed: Option<&[Rectangle<i32, Physical>]>,
    ) -> Option<Source> {
        let name = output.name();
        let age = usize::from(b.drawn);
        let rendered = renderer
            .bind(&mut b.texture)
            .map_err(|e| e.to_string())
            .and_then(|mut fb| {
                b.tracker
                    .render_output(renderer, &mut fb, age, elements, clear)
                    .map(|r| r.damage.cloned().unwrap_or_default())
                    .map_err(|e| format!("{e:?}"))
            });
        let changed: Vec<Rectangle<i32, Physical>> = match rendered {
            Ok(damage) => {
                b.drawn = true;
                damage
                    .into_iter()
                    .map(|d| {
                        Rectangle::new(
                            (d.loc.x * 2, d.loc.y * 2).into(),
                            (d.size.w * 2, d.size.h * 2).into(),
                        )
                    })
                    .collect()
            }
            Err(e) => {
                log::warn!("glass: the backdrop of {name} did not render: {e}");
                b.drawn = false;
                return None;
            }
        };
        let pass = self.pass.get();
        let mode = output.current_mode()?;
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
        let fx = self.fx.borrow();
        let fx = fx.as_ref()?;
        if let Err(e) = b.picture.update(renderer, fx, &update) {
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
        if wanted
            && let Some(b) = self.backdrops.borrow().get(&name)
            && b.hidden
        {
            *self.source.borrow_mut() = b.picture.source();
            let mut own = self.own.borrow_mut();
            for ((o, _), b) in self.beneath.borrow().iter() {
                if let (true, Some(g), Some(s)) =
                    (*o == name, b.glass.upgrade(), b.backdrop.picture.source())
                {
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
            .any(|b| !b.hidden && b.picture.is_stale())
            || self
                .beneath
                .borrow()
                .iter()
                .any(|((o, _), b)| shown(o) && b.backdrop.picture.is_stale())
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
            if !b.hidden && b.picture.is_stale() {
                b.picture.refresh();
                any = true;
            }
        }
        for ((o, _), b) in self.beneath.borrow_mut().iter_mut() {
            let shown = backdrops.get(o).is_some_and(|b| !b.hidden);
            if shown && b.backdrop.picture.is_stale() {
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

impl OutputBackdrop {
    fn key(seen: Size<i32, Physical>, scale: f64) -> (Size<i32, Physical>, u64) {
        let size = Size::from(((seen.w + 1) / 2, (seen.h + 1) / 2));
        (size, (scale / 2.0).to_bits())
    }

    fn new(
        renderer: &mut GlesRenderer,
        seen: Size<i32, Physical>,
        scale: f64,
        name: &str,
    ) -> Option<OutputBackdrop> {
        let key = OutputBackdrop::key(seen, scale);
        let (size, half) = (key.0, scale / 2.0);
        let texture: GlesTexture = renderer
            .create_buffer(Fourcc::Abgr8888, (size.w, size.h).into())
            .map_err(|e| log::warn!("glass: no backdrop for {name}: {e}"))
            .ok()?;
        Some(OutputBackdrop {
            texture,
            tracker: OutputDamageTracker::new(size, half, Transform::Normal),
            key,
            drawn: false,
            picture: Picture::new(),
            hidden: false,
        })
    }
}

struct Reads {
    needed: Vec<Rectangle<i32, Physical>>,
    reach: Vec<Rectangle<i32, Physical>>,
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
            reprisefx::blur::texel(s.blur, s.material, pass.quality),
            reprisefx::blur::reach(s.blur, s.material, pass.quality),
        );
        let output = Rectangle::from_size(seen);
        let mut out = Reads {
            needed: Vec::new(),
            reach: Vec::new(),
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
/// Molten or above scale 1, else cut into the table's rows. A square-bottomed shape keeps square
/// bottom corners in both.
pub(crate) fn drawn_shapes(pane: &Pane, material: Material, scale: f64) -> Vec<(Rect, [f32; 4])> {
    let fine = crate::output::draw_scale(scale as f32) > 1;
    pane.shapes
        .iter()
        .flat_map(|s| {
            let r = s.rect;
            match material {
                _ if material == Material::Molten || fine => {
                    let c = corner_radius(s.corner);
                    let t = if s.square_top { 0.0 } else { c };
                    let b = if s.square_bottom { 0.0 } else { c };
                    vec![(r, [t, t, b, b])]
                }
                Material::Frosted | Material::Smoked | Material::Molten => {
                    // Square-bottomed: the top half of a shape twice as tall. Square-topped: the
                    // bottom half.
                    let h = if s.square_bottom || s.square_top {
                        2 * r.height
                    } else {
                        r.height
                    };
                    let skip = if s.square_top { r.height } else { 0 };
                    rounded_rects(r.width, h, s.corner)
                        .into_iter()
                        .filter_map(|p| {
                            let y0 = p.y.max(skip);
                            let y1 = p.bottom().min(skip + r.height);
                            (y1 > y0).then(|| {
                                (
                                    Rect::new(r.x + p.x, r.y + y0 - skip, p.width, y1 - y0),
                                    [0.0; 4],
                                )
                            })
                        })
                        .collect()
                }
            }
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
    node: Option<CustomId>,
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
        if let Some(node) = self.node.take() {
            scene.destroy(node);
        }
        if let Some(p) = &pane {
            let node = scene.custom_create(
                tree,
                Box::new(GlassNode {
                    ctx: ctx.clone(),
                    glass: self.glass.clone(),
                    size: p.extent(),
                    pane: p.clone(),
                }),
            );
            scene.set_input(node, Input::Passthrough);
            ctx.nodes.borrow_mut().push(Placed {
                node,
                glass: Rc::downgrade(&self.glass),
                pane: p.clone(),
            });
            self.node = Some(node);
        }
        self.pane = pane;
    }
}

struct GlassNode {
    ctx: Rc<GlassCtx>,
    glass: Rc<Glass>,
    pane: Pane,
    size: Size<i32, Logical>,
}

impl CustomNode<GlesRenderer> for GlassNode {
    fn size(&self) -> Size<i32, Logical> {
        self.size
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
        let shapes: Vec<Shape> = drawn_shapes(&self.pane, pass.settings.material, scale.x)
            .into_iter()
            .map(|(rect, radii)| Shape {
                rect: physical(rect, location, scale),
                radii: radii.map(|r| r * scale.x as f32),
            })
            .collect();
        let holes: Vec<Rectangle<i32, Physical>> = self
            .pane
            .holes
            .iter()
            .map(|&r| physical(r, location, scale))
            .collect();
        let element = self.glass.element(
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
            },
        );
        if let Some(e) = &element {
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
}
