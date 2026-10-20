//! Node handles and what a node holds.

use smithay::{
    backend::renderer::{
        Renderer,
        element::{
            RenderElement, memory::MemoryRenderBuffer, solid::SolidColorBuffer,
            texture::TextureRenderBuffer,
        },
    },
    output::WeakOutput,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Buffer as BufferCoords, Logical, Physical, Point, Rectangle, Scale, Size},
};

/// A node in one [`Scene`](crate::Scene): which scene, which slot, which
/// occupant of the slot. A handle outlives its node; verbs on a destroyed
/// node do nothing and queries return `None`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct NodeId {
    pub(crate) scene: u32,
    pub(crate) index: u32,
    pub(crate) generation: u32,
}

macro_rules! handle {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
        pub struct $name(pub(crate) NodeId);

        impl $name {
            pub fn node(self) -> NodeId {
                self.0
            }
        }

        impl From<$name> for NodeId {
            fn from(h: $name) -> NodeId {
                h.0
            }
        }
    };
}

handle!(
    /// A tree: a node that only holds children, bottom first.
    TreeId
);
handle!(RectId);
handle!(
    /// A buffer: a canvas or a texture, cropped and scaled.
    BufferId
);
handle!(SurfaceId);
handle!(CustomId);

/// What a node is, for a host that found a [`NodeId`] by hit testing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NodeType {
    Tree,
    Rect,
    Buffer,
    Surface,
    Custom,
}

/// Which pointer positions a leaf takes. Trees take none of their own.
#[derive(Default)]
pub enum Input {
    /// The whole node; for a surface tree, its surfaces' input regions.
    #[default]
    All,
    /// None: the pointer reaches whatever lies beneath (a shadow, a still,
    /// a glass backdrop).
    Passthrough,
    /// Only this node-local rectangle, logical (a panel inside its shadow).
    Rect(Rectangle<i32, Logical>),
    /// The node-local point is in when the test says so.
    Test(Box<dyn Fn(Point<f64, Logical>) -> bool>),
}

impl Input {
    pub(crate) fn accepts(&self, local: Point<f64, Logical>) -> bool {
        match self {
            Input::All => true,
            Input::Passthrough => false,
            Input::Rect(r) => r.contains(local.to_i32_floor()),
            Input::Test(f) => f(local),
        }
    }
}

impl std::fmt::Debug for Input {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Input::All => f.write_str("All"),
            Input::Passthrough => f.write_str("Passthrough"),
            Input::Rect(r) => write!(f, "Rect({r:?})"),
            Input::Test(_) => f.write_str("Test(..)"),
        }
    }
}

/// A host-made node: the Glass backdrop, or anything else the scene's own
/// leaves cannot draw. It reports its size for hit tests and output
/// overlap, and makes its render elements per output.
pub trait CustomNode<R: Renderer> {
    fn size(&self) -> Size<i32, Logical>;

    /// Whether glass shows this node through it, like the scene's own
    /// leaves: true for a node that stands for the wallpaper. Glass itself
    /// is never in its own backdrop, so the default is false.
    fn in_backdrop(&self) -> bool {
        false
    }

    /// Whether [`Scene::elements_below`](crate::Scene::elements_below)
    /// keeps this node: true for glass, which shows through glass above it.
    fn in_backdrop_below(&self) -> bool {
        self.in_backdrop()
    }

    /// Whether the node draws anything in `area` (logical, from its top-left):
    /// [`Scene::left_out_below`](crate::Scene::left_out_below) passes over a
    /// node whose size reaches the area but whose drawing does not.
    fn draws_in(&self, _area: Rectangle<i32, Logical>) -> bool {
        true
    }

    /// The node's elements, front to back, for a view at `scale` where the
    /// node's top-left lands at `location`, physical, from the view's
    /// top-left. `alpha` is the node's opacity times its ancestors'.
    fn elements(
        &self,
        renderer: &mut R,
        location: Point<f64, Physical>,
        scale: Scale<f64>,
        alpha: f32,
    ) -> Vec<CustomElement<R>>;
}

pub struct CustomElement<R: Renderer>(pub Box<dyn RenderElement<R>>);

impl<R: Renderer> CustomElement<R> {
    pub fn new(element: impl RenderElement<R> + 'static) -> Self {
        CustomElement(Box::new(element))
    }

    pub(crate) fn inner(&self) -> &dyn RenderElement<R> {
        &*self.0
    }
}

impl<R: Renderer> std::fmt::Debug for CustomElement<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("CustomElement").field(self.0.id()).finish()
    }
}

pub(crate) enum Pixels<R: Renderer> {
    None,
    /// A CPU canvas, ARGB8888 premultiplied, uploaded on first render and
    /// again after each change. Kept for the node's life so its element Id
    /// stays the same across replacements.
    Canvas(MemoryRenderBuffer),
    Texture(TextureRenderBuffer<R::TextureId>),
}

pub(crate) struct BufferNode<R: Renderer> {
    pub(crate) pixels: Pixels<R>,
    pub(crate) buffer_size: Size<i32, BufferCoords>,
    pub(crate) source: Option<Rectangle<f64, BufferCoords>>,
    pub(crate) dest_size: Option<Size<i32, Logical>>,
    /// Node-local, logical: what the buffer covers with opaque pixels.
    pub(crate) opaque: Vec<Rectangle<i32, Logical>>,
}

impl<R: Renderer> BufferNode<R> {
    /// The size drawn and hit-tested: the dest size, else the source box's,
    /// else the pixels'. Nothing shown and no dest size is empty.
    pub(crate) fn size(&self) -> Size<i32, Logical> {
        if let Some(s) = self.dest_size {
            return s;
        }
        if matches!(self.pixels, Pixels::None) {
            return Size::default();
        }
        match self.source {
            Some(src) => Size::from((src.size.w.round() as i32, src.size.h.round() as i32)),
            None => Size::from((self.buffer_size.w, self.buffer_size.h)),
        }
    }
}

pub(crate) struct SurfaceNode {
    pub(crate) surface: WlSurface,
    /// Outputs this tree's surfaces were sent `enter` for, so a destroyed
    /// or removed node can send `leave`.
    pub(crate) entered: Vec<WeakOutput>,
}

pub(crate) struct RectNode {
    pub(crate) buffer: SolidColorBuffer,
    pub(crate) size: Size<i32, Logical>,
}

pub(crate) enum Content<R: Renderer> {
    Tree(Vec<NodeId>),
    Rect(RectNode),
    Buffer(BufferNode<R>),
    Surface(SurfaceNode),
    Custom(Box<dyn CustomNode<R>>),
}

impl<R: Renderer> Content<R> {
    pub(crate) fn node_type(&self) -> NodeType {
        match self {
            Content::Tree(_) => NodeType::Tree,
            Content::Rect(_) => NodeType::Rect,
            Content::Buffer(_) => NodeType::Buffer,
            Content::Surface(_) => NodeType::Surface,
            Content::Custom(_) => NodeType::Custom,
        }
    }
}

pub(crate) struct Node<R: Renderer, D> {
    /// `None` for the root, and for a node made under a dead parent.
    pub(crate) parent: Option<TreeId>,
    pub(crate) position: Point<i32, Logical>,
    pub(crate) enabled: bool,
    /// Drawn by [`crate::Scene::backdrop_elements`]; false leaves the node
    /// and everything under it out of the backdrop.
    pub(crate) in_backdrop: bool,
    pub(crate) opacity: f32,
    /// A tree's clip, node-local: its leaves draw and hit only inside it.
    pub(crate) clip: Option<Rectangle<i32, Logical>>,
    /// Layout: the node and everything under it are left out of any view
    /// lying wholly inside it, and out of hit tests at points in it.
    pub(crate) hidden_in: Option<Rectangle<i32, Logical>>,
    pub(crate) input: Input,
    pub(crate) data: Option<D>,
    pub(crate) content: Content<R>,
}

impl<R: Renderer, D> Node<R, D> {
    /// Whether `area` (layout) lies inside the node's `hidden_in`.
    pub(crate) fn hidden_over(&self, area: Rectangle<i32, Logical>) -> bool {
        self.hidden_in.is_some_and(|h| h.contains_rect(area))
    }
}
