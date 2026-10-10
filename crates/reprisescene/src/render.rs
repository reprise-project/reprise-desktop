//! A scene as Smithay render elements for one view: an output, or any
//! layout rectangle at a scale for an offscreen picture.

use std::cell::Cell;

use smithay::{
    backend::renderer::{
        ImportAll, ImportMem, Renderer, Texture,
        element::{
            Element, Id, Kind, RenderElement, UnderlyingStorage,
            memory::MemoryRenderBufferRenderElement,
            solid::SolidColorRenderElement,
            surface::{WaylandSurfaceRenderElement, render_elements_from_surface_tree},
            texture::TextureRenderElement,
        },
        utils::{CommitCounter, DamageSet, OpaqueRegions},
    },
    output::Output,
    utils::{
        Buffer as BufferCoords, Logical, Physical, Point, Rectangle, Scale, Size, Transform,
        user_data::UserDataMap,
    },
};

use crate::{
    Scene,
    node::{BufferNode, Content, CustomElement, NodeId, Pixels},
    query::output_rect,
};

/// What to render: a layout rectangle at a scale. An output's view is its
/// layout box at its scale; an offscreen picture picks its own.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub area: Rectangle<i32, Logical>,
    pub scale: Scale<f64>,
}

impl View {
    pub fn new(area: Rectangle<i32, Logical>, scale: impl Into<Scale<f64>>) -> Self {
        View {
            area,
            scale: scale.into(),
        }
    }

    /// `output`'s layout box at its scale; `None` without a mode.
    pub fn of(output: &Output) -> Option<View> {
        Some(View::new(
            output_rect(output)?,
            output.current_scale().fractional_scale(),
        ))
    }

    /// `pos`, layout, as physical pixels from the view's top-left.
    pub fn to_physical(&self, pos: Point<i32, Logical>) -> Point<f64, Physical> {
        (pos - self.area.loc).to_f64().to_physical(self.scale)
    }
}

/// How one walk of the scene goes: the view, which nodes it keeps and
/// whether the hoist applies.
#[derive(Clone, Copy)]
struct Pass<'a> {
    view: &'a View,
    mode: Mode<'a>,
    hoist: bool,
}

/// Which nodes a walk keeps.
#[derive(Clone, Copy)]
enum Mode<'a> {
    /// Everything shown: the screen.
    Screen,
    /// What glass shows through: no subtree left out of the backdrop
    /// and no custom node but those [`CustomNode::in_backdrop`] keeps.
    Backdrop,
    /// What lies under `anchor`'s subtree as the screen draws it: the
    /// subtree and everything above it skipped, the subtrees left out of
    /// the backdrop kept, the custom nodes as in `Backdrop`. `passed`
    /// turns true once the walk is past the anchor.
    Below {
        anchor: NodeId,
        passed: &'a Cell<bool>,
    },
}

impl Mode<'_> {
    fn backdrop(self) -> bool {
        matches!(self, Mode::Backdrop)
    }
}

/// A leaf a walk reached: the node, its layout position and opacity, and
/// whether it lies in a subtree left out of the backdrop.
type Visit<'v, R, D> = dyn FnMut(&crate::node::Node<R, D>, Point<i32, Logical>, f32, bool) + 'v;

/// A buffer's element with the node's opaque region, physical, from the
/// element's top-left.
#[derive(Debug)]
pub struct BufferElement<E> {
    inner: E,
    opaque: Vec<Rectangle<i32, Physical>>,
}

impl<E: Element> Element for BufferElement<E> {
    fn id(&self) -> &Id {
        self.inner.id()
    }
    fn current_commit(&self) -> CommitCounter {
        self.inner.current_commit()
    }
    fn location(&self, scale: Scale<f64>) -> Point<i32, Physical> {
        self.inner.location(scale)
    }
    fn src(&self) -> Rectangle<f64, BufferCoords> {
        self.inner.src()
    }
    fn transform(&self) -> Transform {
        self.inner.transform()
    }
    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.inner.geometry(scale)
    }
    fn damage_since(
        &self,
        scale: Scale<f64>,
        commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        self.inner.damage_since(scale, commit)
    }
    fn opaque_regions(&self, scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        if self.opaque.is_empty() {
            self.inner.opaque_regions(scale)
        } else if self.inner.alpha() < 1.0 {
            OpaqueRegions::default()
        } else {
            OpaqueRegions::from_slice(&self.opaque)
        }
    }
    fn alpha(&self) -> f32 {
        self.inner.alpha()
    }
    fn kind(&self) -> Kind {
        self.inner.kind()
    }
}

impl<R: Renderer, E: RenderElement<R>> RenderElement<R> for BufferElement<E> {
    fn draw(
        &self,
        frame: &mut R::Frame<'_, '_>,
        src: Rectangle<f64, BufferCoords>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
        cache: Option<&UserDataMap>,
    ) -> Result<(), R::Error> {
        self.inner
            .draw(frame, src, dst, damage, opaque_regions, cache)
    }
    fn underlying_storage(&self, renderer: &mut R) -> Option<UnderlyingStorage<'_>> {
        self.inner.underlying_storage(renderer)
    }
}

/// One element of a scene, as [`Scene::render_elements`] makes them.
pub enum SceneElement<R: Renderer> {
    Solid(SolidColorRenderElement),
    Canvas(BufferElement<MemoryRenderBufferRenderElement<R>>),
    Texture(BufferElement<TextureRenderElement<R::TextureId>>),
    Surface(WaylandSurfaceRenderElement<R>),
    Custom(CustomElement<R>),
}

impl<R: Renderer> std::fmt::Debug for SceneElement<R>
where
    R::TextureId: std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SceneElement::Solid(e) => f.debug_tuple("Solid").field(e).finish(),
            SceneElement::Canvas(e) => f.debug_tuple("Canvas").field(e.inner.id()).finish(),
            SceneElement::Texture(e) => f.debug_tuple("Texture").field(e.inner.id()).finish(),
            SceneElement::Surface(e) => f.debug_tuple("Surface").field(e).finish(),
            SceneElement::Custom(e) => f.debug_tuple("Custom").field(e).finish(),
        }
    }
}

/// Calls `$f` on the element inside any variant.
macro_rules! each {
    ($self:expr, $e:ident => $f:expr) => {
        match $self {
            SceneElement::Solid($e) => $f,
            SceneElement::Canvas($e) => $f,
            SceneElement::Texture($e) => $f,
            SceneElement::Surface($e) => $f,
            SceneElement::Custom(c) => {
                let $e = c.inner();
                $f
            }
        }
    };
}

impl<R> Element for SceneElement<R>
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Texture + Clone + Send + 'static,
{
    fn id(&self) -> &Id {
        each!(self, e => e.id())
    }
    fn current_commit(&self) -> CommitCounter {
        each!(self, e => e.current_commit())
    }
    fn location(&self, scale: Scale<f64>) -> Point<i32, Physical> {
        each!(self, e => e.location(scale))
    }
    fn src(&self) -> Rectangle<f64, BufferCoords> {
        each!(self, e => e.src())
    }
    fn transform(&self) -> Transform {
        each!(self, e => e.transform())
    }
    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        each!(self, e => e.geometry(scale))
    }
    fn damage_since(
        &self,
        scale: Scale<f64>,
        commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        each!(self, e => e.damage_since(scale, commit))
    }
    fn opaque_regions(&self, scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        each!(self, e => e.opaque_regions(scale))
    }
    fn alpha(&self) -> f32 {
        each!(self, e => e.alpha())
    }
    fn kind(&self) -> Kind {
        each!(self, e => e.kind())
    }
    fn is_framebuffer_effect(&self) -> bool {
        each!(self, e => e.is_framebuffer_effect())
    }
}

impl<R> RenderElement<R> for SceneElement<R>
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Texture + Clone + Send + 'static,
{
    fn draw(
        &self,
        frame: &mut R::Frame<'_, '_>,
        src: Rectangle<f64, BufferCoords>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
        cache: Option<&UserDataMap>,
    ) -> Result<(), R::Error> {
        each!(self, e => RenderElement::<R>::draw(e, frame, src, dst, damage, opaque_regions, cache))
    }
    fn underlying_storage(&self, renderer: &mut R) -> Option<UnderlyingStorage<'_>> {
        each!(self, e => RenderElement::<R>::underlying_storage(e, renderer))
    }
    fn capture_framebuffer(
        &self,
        frame: &mut R::Frame<'_, '_>,
        src: Rectangle<f64, BufferCoords>,
        dst: Rectangle<i32, Physical>,
        cache: &UserDataMap,
    ) -> Result<(), R::Error> {
        each!(self, e => RenderElement::<R>::capture_framebuffer(e, frame, src, dst, cache))
    }
}

/// The part of `region` (node-local, logical) a node of `size` drawn at
/// `location` covers in whole physical pixels, from the element's
/// rounded top-left.
fn opaque_physical(
    region: &[Rectangle<i32, Logical>],
    size: Size<i32, Logical>,
    location: Point<f64, Physical>,
    scale: Scale<f64>,
) -> Vec<Rectangle<i32, Physical>> {
    let base = location.to_i32_round::<i32>();
    region
        .iter()
        .filter_map(|r| r.intersection(Rectangle::from_size(size)))
        .filter_map(|r| {
            let x0 = (location.x + r.loc.x as f64 * scale.x).ceil() as i32 - base.x;
            let y0 = (location.y + r.loc.y as f64 * scale.y).ceil() as i32 - base.y;
            let x1 = (location.x + (r.loc.x + r.size.w) as f64 * scale.x).floor() as i32 - base.x;
            let y1 = (location.y + (r.loc.y + r.size.h) as f64 * scale.y).floor() as i32 - base.y;
            (x1 > x0 && y1 > y0).then(|| Rectangle::new((x0, y0).into(), (x1 - x0, y1 - y0).into()))
        })
        .collect()
}

impl<R, D> Scene<R, D>
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Texture + Clone + Send + 'static,
{
    /// The elements `view` shows, front to back as Smithay's damage
    /// tracker takes them: the root's top child first, a tree's children
    /// from the top, a surface tree's surfaces as Smithay orders them.
    /// Disabled subtrees, fully transparent leaves and leaves outside the
    /// view make none. Each leaf's element keeps its Id from frame to
    /// frame, so the tracker damages a moved, hidden, restacked or
    /// changed node in its old and new places.
    pub fn render_elements(&self, renderer: &mut R, view: &View) -> Vec<SceneElement<R>> {
        self.elements(renderer, view, Mode::Screen)
    }

    /// What glass shows through, for `view`: [`Scene::render_elements`]
    /// without the subtrees [`Scene::set_in_backdrop`] left out and
    /// without custom nodes (the glass itself) other than those
    /// [`CustomNode::in_backdrop`] keeps. Its Ids are the same as
    /// the full render's, so a damage tracker of its own sees only what
    /// changed in the backdrop.
    pub fn backdrop_elements(&self, renderer: &mut R, view: &View) -> Vec<SceneElement<R>> {
        self.elements(renderer, view, Mode::Backdrop)
    }

    /// What the screen draws under `node`'s subtree, for `view`: the
    /// subtree and everything above it left out, the subtrees
    /// [`Scene::set_in_backdrop`] left out kept (another window's frame
    /// beneath glass), custom nodes as [`Scene::backdrop_elements`] keeps
    /// them. Its Ids are the full render's.
    pub fn elements_below(
        &self,
        renderer: &mut R,
        node: impl Into<NodeId>,
        view: &View,
    ) -> Vec<SceneElement<R>> {
        let passed = Cell::new(false);
        let mode = Mode::Below {
            anchor: node.into(),
            passed: &passed,
        };
        self.elements(renderer, view, mode)
    }

    /// Whether a leaf the screen draws under `node`'s subtree, in a
    /// subtree [`Scene::set_in_backdrop`] left out, meets `area` (layout):
    /// whether [`Scene::elements_below`] there differs from the backdrop.
    /// Makes no elements.
    pub fn left_out_below(&self, node: impl Into<NodeId>, area: Rectangle<i32, Logical>) -> bool {
        let passed = Cell::new(false);
        let pass = Pass {
            view: &View::new(area, 1.0),
            mode: Mode::Below {
                anchor: node.into(),
                passed: &passed,
            },
            hoist: true,
        };
        let mut hit = false;
        self.walk(
            self.root().node(),
            Point::default(),
            1.0,
            false,
            pass,
            &mut |_, _, _, left_out| hit |= left_out,
        );
        hit
    }

    /// [`Scene::render_elements`] for `output`'s view; none without a mode.
    pub fn render_elements_for(&self, renderer: &mut R, output: &Output) -> Vec<SceneElement<R>> {
        View::of(output).map_or_else(Vec::new, |v| self.render_elements(renderer, &v))
    }

    /// The elements of `node`'s subtree alone, where it stands in the
    /// layout, as if nothing else were in the scene: for a picture of one
    /// window. The node is drawn whatever its own and its ancestors'
    /// enabled flags and opacity say; the nodes under it keep theirs.
    pub fn render_elements_of(
        &self,
        renderer: &mut R,
        node: impl Into<NodeId>,
        view: &View,
    ) -> Vec<SceneElement<R>> {
        let id = node.into();
        let mut out = Vec::new();
        if let Some(n) = self.node(id) {
            let origin = self.origin_of(id);
            let pass = Pass {
                view,
                mode: Mode::Screen,
                hoist: false,
            };
            self.walk_node(
                n,
                origin + n.position,
                1.0,
                false,
                pass,
                &mut |n, pos, alpha, _| {
                    out.extend(Self::leaf_element(renderer, n, pos, alpha, view))
                },
            );
        }
        out
    }

    fn elements(&self, renderer: &mut R, view: &View, mode: Mode) -> Vec<SceneElement<R>> {
        let mut out = Vec::new();
        let pass = Pass {
            view,
            mode,
            hoist: true,
        };
        self.walk(
            self.root().node(),
            Point::default(),
            1.0,
            false,
            pass,
            &mut |n, pos, alpha, _| out.extend(Self::leaf_element(renderer, n, pos, alpha, view)),
        );
        out
    }

    /// Visit the leaves `pass` keeps under `id` front to back (the top
    /// child first), `origin` being its parent's layout position, `alpha`
    /// the opacity above it and `left_out` whether an ancestor is left out
    /// of the backdrop.
    fn walk(
        &self,
        id: NodeId,
        origin: Point<i32, Logical>,
        alpha: f32,
        left_out: bool,
        pass: Pass,
        visit: &mut Visit<'_, R, D>,
    ) {
        let hoist = self.hoist.as_ref().filter(|_| pass.hoist);
        if hoist.is_some_and(|(_, nodes)| nodes.contains(&id)) {
            return;
        }
        let Some(n) = self
            .node(id)
            .filter(|n| n.enabled && (n.in_backdrop || !pass.mode.backdrop()))
        else {
            return;
        };
        if let Mode::Below { anchor, passed } = pass.mode
            && anchor == id
        {
            passed.set(true);
            return;
        }
        let alpha = alpha * n.opacity;
        if alpha <= 0.0 {
            return;
        }
        let left_out = left_out || !n.in_backdrop;
        if let Some((_, nodes)) = hoist.filter(|(into, _)| *into == id) {
            for &h in nodes.iter().rev() {
                if let Mode::Below { anchor, passed } = pass.mode
                    && anchor == h
                {
                    passed.set(true);
                    continue;
                }
                if let (Some(hn), Some((a, out))) = (self.node(h), self.shown(h, pass.mode)) {
                    let pos = self.origin_of(h) + hn.position;
                    self.walk_node(hn, pos, a, out, pass, visit);
                }
            }
        }
        self.walk_node(n, origin + n.position, alpha, left_out, pass, visit);
    }

    /// The opacity `id` draws at with every ancestor's, and whether it or
    /// one of them is left out of the backdrop, if it and they are all
    /// enabled (and in the backdrop, for the backdrop's walk) up to the
    /// root; None if any hides it.
    fn shown(&self, id: NodeId, mode: Mode) -> Option<(f32, bool)> {
        let mut id = id;
        let mut alpha = 1.0;
        let mut left_out = false;
        loop {
            let n = self.node(id)?;
            if !n.enabled || (mode.backdrop() && !n.in_backdrop) {
                return None;
            }
            alpha *= n.opacity;
            left_out |= !n.in_backdrop;
            match n.parent {
                Some(p) => id = p.0,
                None => {
                    return (id == self.root().node() && alpha > 0.0).then_some((alpha, left_out));
                }
            }
        }
    }

    /// `walk` for `n` at layout position `pos` and opacity `alpha`, its
    /// own flags already applied.
    fn walk_node(
        &self,
        n: &crate::node::Node<R, D>,
        pos: Point<i32, Logical>,
        alpha: f32,
        left_out: bool,
        pass: Pass,
        visit: &mut Visit<'_, R, D>,
    ) {
        if let Content::Tree(children) = &n.content {
            for c in children.iter().rev() {
                self.walk(*c, pos, alpha, left_out, pass, visit);
            }
            return;
        }
        match pass.mode {
            Mode::Screen => {}
            Mode::Below { passed, .. } if !passed.get() => return,
            Mode::Backdrop | Mode::Below { .. } => {
                if matches!(&n.content, Content::Custom(c) if !c.in_backdrop()) {
                    return;
                }
            }
        }
        if !Self::leaf_rect(n, pos).is_some_and(|r| r.overlaps(pass.view.area)) {
            return;
        }
        visit(n, pos, alpha, left_out);
    }

    /// The elements of leaf `n` at layout position `pos` and opacity
    /// `alpha` in `view`.
    fn leaf_element(
        renderer: &mut R,
        n: &crate::node::Node<R, D>,
        pos: Point<i32, Logical>,
        alpha: f32,
        view: &View,
    ) -> Vec<SceneElement<R>> {
        let location = view.to_physical(pos);
        match &n.content {
            Content::Tree(_) => Vec::new(),
            Content::Rect(r) => vec![SceneElement::Solid(SolidColorRenderElement::from_buffer(
                &r.buffer,
                location.to_i32_round(),
                view.scale,
                alpha,
                Kind::Unspecified,
            ))],
            Content::Buffer(b) => buffer_element(renderer, b, location, view.scale, alpha)
                .into_iter()
                .collect(),
            Content::Surface(s) => {
                render_elements_from_surface_tree::<R, WaylandSurfaceRenderElement<R>>(
                    renderer,
                    &s.surface,
                    location.to_i32_round(),
                    view.scale,
                    alpha,
                    Kind::Unspecified,
                )
                .into_iter()
                .map(SceneElement::Surface)
                .collect()
            }
            Content::Custom(c) => c
                .elements(renderer, location, view.scale, alpha)
                .into_iter()
                .map(SceneElement::Custom)
                .collect(),
        }
    }
}

fn buffer_element<R>(
    renderer: &mut R,
    b: &BufferNode<R>,
    location: Point<f64, Physical>,
    scale: Scale<f64>,
    alpha: f32,
) -> Option<SceneElement<R>>
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Texture + Clone + Send + 'static,
{
    let size = b.size();
    // Always explicit: without one, Smithay crops to the dest size.
    let s = b
        .source
        .unwrap_or_else(|| Rectangle::from_size(b.buffer_size.to_f64()));
    let src = Some(Rectangle::<f64, Logical>::new(
        (s.loc.x, s.loc.y).into(),
        (s.size.w, s.size.h).into(),
    ));
    let opaque = opaque_physical(&b.opaque, size, location, scale);
    match &b.pixels {
        Pixels::None => None,
        Pixels::Canvas(m) => {
            match MemoryRenderBufferRenderElement::from_buffer(
                renderer,
                location,
                m,
                Some(alpha),
                src,
                Some(size),
                Kind::Unspecified,
            ) {
                Ok(inner) => Some(SceneElement::Canvas(BufferElement { inner, opaque })),
                Err(e) => {
                    log::warn!("scene: canvas upload failed: {e}");
                    None
                }
            }
        }
        Pixels::Texture(t) => Some(SceneElement::Texture(BufferElement {
            inner: TextureRenderElement::from_texture_render_buffer(
                location,
                t,
                Some(alpha),
                src,
                Some(size),
                Kind::Unspecified,
            ),
            opaque,
        })),
    }
}
