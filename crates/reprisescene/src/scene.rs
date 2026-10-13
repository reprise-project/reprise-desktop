//! The tree: making nodes, ordering siblings, positions, enable,
//! reparenting, destroying, and each kind's own setters.

use std::sync::atomic::{AtomicU32, Ordering};

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            Color32F, Renderer, Texture,
            element::{
                memory::MemoryRenderBuffer, solid::SolidColorBuffer, texture::TextureRenderBuffer,
            },
        },
    },
    output::Output,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Buffer as BufferCoords, Logical, Point, Rectangle, Size, Transform},
};

use crate::node::{
    BufferId, BufferNode, Content, CustomId, CustomNode, Input, Node, NodeId, NodeType, Pixels,
    RectId, RectNode, SurfaceId, SurfaceNode, TreeId,
};

static NEXT_SCENE: AtomicU32 = AtomicU32::new(1);

struct Slot<R: Renderer, D> {
    generation: u32,
    node: Option<Node<R, D>>,
}

/// One retained scene: a root tree and everything under it, plus the
/// outputs it is shown on. A compositor holds several (the desktop, a
/// capture scene per window, offscreen pictures); a node belongs to one.
///
/// `R` is the renderer whose textures buffer nodes show and whose elements
/// custom nodes make. `D` is the host's tag type for [`Scene::set_data`].
pub struct Scene<R: Renderer, D = ()> {
    tag: u32,
    slots: Vec<Slot<R, D>>,
    free: Vec<u32>,
    root: TreeId,
    pub(crate) outputs: Vec<Output>,
    changes: u64,
    /// The tree the hoisted nodes draw above and those nodes, bottom
    /// first (`set_hoist`).
    pub(crate) hoist: Option<(NodeId, Vec<NodeId>)>,
}

impl<R: Renderer, D> Default for Scene<R, D> {
    fn default() -> Self {
        Self::new()
    }
}

impl<R: Renderer, D> Scene<R, D> {
    pub fn new() -> Self {
        let tag = NEXT_SCENE.fetch_add(1, Ordering::Relaxed);
        let root = NodeId {
            scene: tag,
            index: 0,
            generation: 0,
        };
        Scene {
            tag,
            slots: vec![Slot {
                generation: 0,
                node: Some(Node {
                    parent: None,
                    position: Point::default(),
                    enabled: true,
                    in_backdrop: true,
                    opacity: 1.0,
                    clip: None,
                    input: Input::All,
                    data: None,
                    content: Content::Tree(Vec::new()),
                }),
            }],
            free: Vec::new(),
            root: TreeId(root),
            outputs: Vec::new(),
            changes: 0,
            hoist: None,
        }
    }

    pub fn root(&self) -> TreeId {
        self.root
    }

    pub(crate) fn node(&self, id: NodeId) -> Option<&Node<R, D>> {
        debug_assert_eq!(id.scene, self.tag, "a node of another scene");
        if id.scene != self.tag {
            return None;
        }
        let slot = self.slots.get(id.index as usize)?;
        if slot.generation != id.generation {
            return None;
        }
        slot.node.as_ref()
    }

    /// How many changes the scene's nodes have had: while it stays the
    /// same, the scene draws as it did. Changes inside a custom node or a
    /// client surface do not count; their owners ask for frames.
    pub fn changes(&self) -> u64 {
        self.changes
    }

    pub(crate) fn node_mut(&mut self, id: NodeId) -> Option<&mut Node<R, D>> {
        self.changes += 1;
        self.node_untracked(id)
    }

    /// `node_mut` for bookkeeping that changes nothing drawn (which
    /// outputs a surface is on): not counted in `changes`.
    pub(crate) fn node_untracked(&mut self, id: NodeId) -> Option<&mut Node<R, D>> {
        debug_assert_eq!(id.scene, self.tag, "a node of another scene");
        if id.scene != self.tag {
            return None;
        }
        let slot = self.slots.get_mut(id.index as usize)?;
        if slot.generation != id.generation {
            return None;
        }
        slot.node.as_mut()
    }

    pub(crate) fn live(&self) -> impl Iterator<Item = (NodeId, &Node<R, D>)> {
        self.slots.iter().enumerate().filter_map(|(i, s)| {
            s.node.as_ref().map(|n| {
                (
                    NodeId {
                        scene: self.tag,
                        index: i as u32,
                        generation: s.generation,
                    },
                    n,
                )
            })
        })
    }

    pub(crate) fn children_of(&self, tree: NodeId) -> &[NodeId] {
        match self.node(tree).map(|n| &n.content) {
            Some(Content::Tree(c)) => c,
            _ => &[],
        }
    }

    fn children_mut(&mut self, tree: NodeId) -> Option<&mut Vec<NodeId>> {
        match self.node_mut(tree).map(|n| &mut n.content) {
            Some(Content::Tree(c)) => Some(c),
            _ => None,
        }
    }

    fn insert(&mut self, parent: TreeId, content: Content<R>) -> NodeId {
        let alive = self.node(parent.0).is_some();
        let node = Node {
            parent: alive.then_some(parent),
            position: Point::default(),
            enabled: true,
            in_backdrop: true,
            opacity: 1.0,
            clip: None,
            input: Input::All,
            data: None,
            content,
        };
        let id = match self.free.pop() {
            Some(index) => {
                let slot = &mut self.slots[index as usize];
                slot.node = Some(node);
                NodeId {
                    scene: self.tag,
                    index,
                    generation: slot.generation,
                }
            }
            None => {
                self.slots.push(Slot {
                    generation: 0,
                    node: Some(node),
                });
                NodeId {
                    scene: self.tag,
                    index: self.slots.len() as u32 - 1,
                    generation: 0,
                }
            }
        };
        if let Some(c) = self.children_mut(parent.0) {
            c.push(id);
        }
        id
    }

    pub fn tree_create(&mut self, parent: TreeId) -> TreeId {
        TreeId(self.insert(parent, Content::Tree(Vec::new())))
    }

    /// A rectangle of `size`, logical, filled with `color` (premultiplied
    /// RGBA), on top of `parent`'s children.
    pub fn rect_create(
        &mut self,
        parent: TreeId,
        size: impl Into<Size<i32, Logical>>,
        color: [f32; 4],
    ) -> RectId {
        let size = size.into();
        let buffer = SolidColorBuffer::new(size, Color32F::from(color));
        RectId(self.insert(parent, Content::Rect(RectNode { buffer, size })))
    }

    pub fn buffer_create(&mut self, parent: TreeId) -> BufferId {
        BufferId(self.insert(
            parent,
            Content::Buffer(BufferNode {
                pixels: Pixels::None,
                buffer_size: Size::default(),
                source: None,
                dest_size: None,
                opaque: Vec::new(),
            }),
        ))
    }

    /// A client surface and its subsurfaces, the surface's origin at the
    /// node's position. Popups are separate nodes the host places; an
    /// xdg toplevel's window geometry offset is the host's to apply.
    pub fn surface_create(&mut self, parent: TreeId, surface: WlSurface) -> SurfaceId {
        SurfaceId(self.insert(
            parent,
            Content::Surface(SurfaceNode {
                surface,
                entered: Vec::new(),
            }),
        ))
    }

    pub fn custom_create(&mut self, parent: TreeId, custom: Box<dyn CustomNode<R>>) -> CustomId {
        CustomId(self.insert(parent, Content::Custom(custom)))
    }

    pub fn is_alive(&self, node: impl Into<NodeId>) -> bool {
        self.node(node.into()).is_some()
    }

    pub fn node_type(&self, node: impl Into<NodeId>) -> Option<NodeType> {
        self.node(node.into()).map(|n| n.content.node_type())
    }

    pub fn as_tree(&self, node: NodeId) -> Option<TreeId> {
        (self.node_type(node)? == NodeType::Tree).then_some(TreeId(node))
    }

    pub fn as_rect(&self, node: NodeId) -> Option<RectId> {
        (self.node_type(node)? == NodeType::Rect).then_some(RectId(node))
    }

    pub fn as_buffer(&self, node: NodeId) -> Option<BufferId> {
        (self.node_type(node)? == NodeType::Buffer).then_some(BufferId(node))
    }

    pub fn as_surface(&self, node: NodeId) -> Option<SurfaceId> {
        (self.node_type(node)? == NodeType::Surface).then_some(SurfaceId(node))
    }

    pub fn as_custom(&self, node: NodeId) -> Option<CustomId> {
        (self.node_type(node)? == NodeType::Custom).then_some(CustomId(node))
    }

    pub fn parent(&self, node: impl Into<NodeId>) -> Option<TreeId> {
        self.node(node.into())?.parent
    }

    pub fn children(&self, tree: TreeId) -> &[NodeId] {
        self.children_of(tree.0)
    }

    pub fn set_position(&mut self, node: impl Into<NodeId>, x: i32, y: i32) {
        if let Some(n) = self.node_mut(node.into()) {
            n.position = Point::from((x, y));
        }
    }

    pub fn position(&self, node: impl Into<NodeId>) -> Option<Point<i32, Logical>> {
        self.node(node.into()).map(|n| n.position)
    }

    pub fn set_enabled(&mut self, node: impl Into<NodeId>, enabled: bool) {
        if let Some(n) = self.node_mut(node.into()) {
            n.enabled = enabled;
        }
    }

    pub fn enabled(&self, node: impl Into<NodeId>) -> bool {
        self.node(node.into()).is_some_and(|n| n.enabled)
    }

    /// Draw `node` and everything under it in the backdrop
    /// ([`Scene::backdrop_elements`]) or leave it out: chrome that lies
    /// over glass, and popups, stay out of what glass shows. On by
    /// default.
    pub fn set_in_backdrop(&mut self, node: impl Into<NodeId>, shown: bool) {
        if let Some(n) = self.node_mut(node.into()) {
            n.in_backdrop = shown;
        }
    }

    pub fn in_backdrop(&self, node: impl Into<NodeId>) -> bool {
        self.node(node.into()).is_some_and(|n| n.in_backdrop)
    }

    /// Opacity from 0 to 1. A tree's multiplies into every leaf under it,
    /// each drawn on its own (no group opacity).
    pub fn set_opacity(&mut self, node: impl Into<NodeId>, opacity: f32) {
        if let Some(n) = self.node_mut(node.into()) {
            n.opacity = opacity.clamp(0.0, 1.0);
        }
    }

    pub fn opacity(&self, node: impl Into<NodeId>) -> Option<f32> {
        self.node(node.into()).map(|n| n.opacity)
    }

    /// Draw and hit `tree`'s leaves only inside `clip` (node-local, so it moves with the
    /// tree); None draws them whole. Nested clips intersect.
    pub fn set_clip(&mut self, tree: TreeId, clip: Option<Rectangle<i32, Logical>>) {
        if let Some(n) = self.node_mut(tree.0) {
            n.clip = clip;
        }
    }

    pub fn clip(&self, tree: TreeId) -> Option<Rectangle<i32, Logical>> {
        self.node(tree.0)?.clip
    }

    /// The intersection of the clips of `node`'s ancestors, layout; None when none clips.
    pub(crate) fn clip_above(&self, node: NodeId) -> Option<Rectangle<i32, Logical>> {
        let mut out = None;
        let mut at = self.node(node).and_then(|n| n.parent);
        while let Some(p) = at {
            let Some(n) = self.node(p.0) else {
                break;
            };
            if let Some(c) = n.clip {
                let pos = self.coords(p).map(|(pos, _)| pos).unwrap_or_default();
                out = Some(meet(out, Rectangle::new(pos + c.loc, c.size)));
            }
            at = n.parent;
        }
        out
    }

    pub fn set_input(&mut self, node: impl Into<NodeId>, input: Input) {
        if let Some(n) = self.node_mut(node.into()) {
            n.input = input;
        }
    }

    pub fn set_data(&mut self, node: impl Into<NodeId>, data: Option<D>) {
        if let Some(n) = self.node_mut(node.into()) {
            n.data = data;
        }
    }

    pub fn data(&self, node: impl Into<NodeId>) -> Option<&D> {
        self.node(node.into())?.data.as_ref()
    }

    /// The nearest tag at or above `node`, and the node holding it: how a
    /// hit leaf finds the view that owns it.
    pub fn find_data(&self, node: impl Into<NodeId>) -> Option<(NodeId, &D)> {
        let mut at = Some(node.into());
        while let Some(id) = at {
            let n = self.node(id)?;
            if let Some(d) = &n.data {
                return Some((id, d));
            }
            at = n.parent.map(|p| p.0);
        }
        None
    }

    /// `node`'s position in layout coordinates, and whether it and every
    /// ancestor are enabled. A node cut off from the root counts as hidden.
    pub fn coords(&self, node: impl Into<NodeId>) -> Option<(Point<i32, Logical>, bool)> {
        let mut id = node.into();
        let mut pos = Point::default();
        let mut enabled = true;
        loop {
            let n = self.node(id)?;
            pos += n.position;
            enabled &= n.enabled;
            match n.parent {
                Some(p) => id = p.0,
                None => return Some((pos, enabled && id == self.root.0)),
            }
        }
    }

    pub(crate) fn origin_of(&self, node: NodeId) -> Point<i32, Logical> {
        self.node(node)
            .and_then(|n| n.parent)
            .and_then(|p| self.coords(p))
            .map(|(p, _)| p)
            .unwrap_or_default()
    }

    fn siblings_mut(&mut self, node: NodeId) -> Option<&mut Vec<NodeId>> {
        let parent = self.node(node)?.parent?;
        self.children_mut(parent.0)
    }

    /// Draw `nodes` (bottom first) above every child of `into`, as if they
    /// were its top children; empty `nodes` ends it. Only what
    /// [`Scene::render_elements`] and [`Scene::backdrop_elements`] give
    /// moves: parents, siblings order, positions and flags stay, and
    /// [`Scene::node_at`] and [`Scene::render_elements_of`] see the tree as
    /// it is. A changed hoist counts as a change.
    pub fn set_hoist(&mut self, into: TreeId, nodes: Vec<NodeId>) {
        let hoist = (!nodes.is_empty()).then_some((into.0, nodes));
        if self.hoist != hoist {
            self.hoist = hoist;
            self.changes += 1;
        }
    }

    pub fn raise_to_top(&mut self, node: impl Into<NodeId>) {
        let node = node.into();
        if let Some(s) = self.siblings_mut(node) {
            s.retain(|c| *c != node);
            s.push(node);
        }
    }

    pub fn lower_to_bottom(&mut self, node: impl Into<NodeId>) {
        let node = node.into();
        if let Some(s) = self.siblings_mut(node) {
            s.retain(|c| *c != node);
            s.insert(0, node);
        }
    }

    /// Put `node` right above `sibling`. Both must share a parent; else
    /// nothing moves.
    pub fn place_above(&mut self, node: impl Into<NodeId>, sibling: impl Into<NodeId>) {
        self.place(node.into(), sibling.into(), 1);
    }

    /// Put `node` right below `sibling`. Both must share a parent; else
    /// nothing moves.
    pub fn place_below(&mut self, node: impl Into<NodeId>, sibling: impl Into<NodeId>) {
        self.place(node.into(), sibling.into(), 0);
    }

    fn place(&mut self, node: NodeId, sibling: NodeId, offset: usize) {
        if node == sibling {
            return;
        }
        let (Some(a), Some(b)) = (self.node(node), self.node(sibling)) else {
            return;
        };
        if a.parent.is_none() || a.parent != b.parent {
            return;
        }
        if let Some(s) = self.siblings_mut(node) {
            s.retain(|c| *c != node);
            let at = s.iter().position(|c| *c == sibling).unwrap_or(s.len());
            s.insert(at + offset, node);
        }
    }

    /// Move `node` under `parent`, on top of its children, keeping its
    /// relative position. Moving a tree under itself does nothing.
    pub fn reparent(&mut self, node: impl Into<NodeId>, parent: TreeId) {
        let node = node.into();
        let Some(n) = self.node(node) else {
            return;
        };
        if n.parent == Some(parent) || self.as_tree(parent.0).is_none() {
            return;
        }
        let mut up = Some(parent);
        while let Some(t) = up {
            if t.0 == node {
                return;
            }
            up = self.node(t.0).and_then(|n| n.parent);
        }
        if let Some(s) = self.siblings_mut(node) {
            s.retain(|c| *c != node);
        }
        if let Some(c) = self.children_mut(parent.0) {
            c.push(node);
        }
        if let Some(n) = self.node_mut(node) {
            n.parent = Some(parent);
        }
    }

    /// Remove `node` and everything under it. Surfaces under it leave the
    /// outputs they were on. The root only loses its children.
    pub fn destroy(&mut self, node: impl Into<NodeId>) {
        let node = node.into();
        if self.node(node).is_none() {
            return;
        }
        if node == self.root.0 {
            for c in self.children_of(node).to_vec() {
                self.destroy(c);
            }
            return;
        }
        if let Some(s) = self.siblings_mut(node) {
            s.retain(|c| *c != node);
        }
        self.free_subtree(node);
    }

    fn free_subtree(&mut self, node: NodeId) {
        let slot = &mut self.slots[node.index as usize];
        let Some(n) = slot.node.take() else {
            return;
        };
        slot.generation = slot.generation.wrapping_add(1);
        self.free.push(node.index);
        match n.content {
            Content::Tree(children) => {
                for c in children {
                    self.free_subtree(c);
                }
            }
            Content::Surface(s) => crate::surface::leave_all(&s),
            _ => {}
        }
    }

    pub fn rect_set_size(&mut self, rect: RectId, size: impl Into<Size<i32, Logical>>) {
        if let Some(Content::Rect(r)) = self.node_mut(rect.0).map(|n| &mut n.content) {
            let size = size.into();
            if r.size != size {
                r.size = size;
                r.buffer.resize(size);
            }
        }
    }

    pub fn rect_set_color(&mut self, rect: RectId, color: [f32; 4]) {
        if let Some(Content::Rect(r)) = self.node_mut(rect.0).map(|n| &mut n.content) {
            let color = Color32F::from(color);
            if r.buffer.color() != color {
                r.buffer.set_color(color);
            }
        }
    }

    pub fn rect_size(&self, rect: RectId) -> Option<Size<i32, Logical>> {
        match self.node(rect.0).map(|n| &n.content) {
            Some(Content::Rect(r)) => Some(r.size),
            _ => None,
        }
    }

    pub fn rect_color(&self, rect: RectId) -> Option<[f32; 4]> {
        match self.node(rect.0).map(|n| &n.content) {
            Some(Content::Rect(r)) => Some(r.buffer.color().components()),
            _ => None,
        }
    }

    fn buffer_mut(&mut self, buffer: BufferId) -> Option<&mut BufferNode<R>> {
        match self.node_mut(buffer.0).map(|n| &mut n.content) {
            Some(Content::Buffer(b)) => Some(b),
            _ => None,
        }
    }

    fn buffer_ref(&self, buffer: BufferId) -> Option<&BufferNode<R>> {
        match self.node(buffer.0).map(|n| &n.content) {
            Some(Content::Buffer(b)) => Some(b),
            _ => None,
        }
    }

    /// Show a canvas: `width * height` ARGB8888 premultiplied pixels, row
    /// by row. The same size as the last canvas keeps the node's element
    /// and damages it whole; another size makes a new element.
    pub fn buffer_set_canvas(&mut self, buffer: BufferId, pixels: &[u32], width: i32, height: i32) {
        let Some(b) = self.buffer_mut(buffer) else {
            return;
        };
        if width <= 0 || height <= 0 || pixels.len() != (width * height) as usize {
            debug_assert!(
                false,
                "a {width}x{height} canvas with {} pixels",
                pixels.len()
            );
            return;
        }
        let size = Size::<i32, BufferCoords>::from((width, height));
        if let (Pixels::Canvas(m), true) = (&mut b.pixels, b.buffer_size == size) {
            let mut ctx = m.render();
            let _ = ctx.draw(|mem| {
                for (d, s) in mem.as_chunks_mut::<4>().0.iter_mut().zip(pixels) {
                    d.copy_from_slice(&s.to_le_bytes());
                }
                Ok::<_, ()>(vec![Rectangle::from_size(size)])
            });
        } else {
            let bytes: Vec<u8> = pixels.iter().flat_map(|p| p.to_le_bytes()).collect();
            b.pixels = Pixels::Canvas(MemoryRenderBuffer::from_slice(
                &bytes,
                Fourcc::Argb8888,
                size,
                1,
                Transform::Normal,
                None,
            ));
            b.buffer_size = size;
        }
    }

    pub fn buffer_set_texture(&mut self, buffer: BufferId, renderer: &R, texture: R::TextureId) {
        let Some(b) = self.buffer_mut(buffer) else {
            return;
        };
        let size = texture.size();
        match &mut b.pixels {
            Pixels::Texture(t) => {
                t.update_from_texture(renderer, texture, 1, Transform::Normal, None)
            }
            _ => {
                b.pixels = Pixels::Texture(TextureRenderBuffer::from_texture(
                    renderer,
                    texture,
                    1,
                    Transform::Normal,
                    None,
                ))
            }
        }
        b.buffer_size = size;
    }

    /// Show a texture of the same size as the one shown, damaged only in
    /// `damage`, buffer pixels. Another size, or no texture shown yet, damages it whole.
    pub fn buffer_set_texture_with_damage(
        &mut self,
        buffer: BufferId,
        renderer: &R,
        texture: R::TextureId,
        damage: &[Rectangle<i32, BufferCoords>],
    ) {
        let Some(b) = self.buffer_mut(buffer) else {
            return;
        };
        match &mut b.pixels {
            Pixels::Texture(t) if b.buffer_size == texture.size() => {
                let mut ctx = t.render();
                let _ = ctx.draw(|tex| {
                    *tex = texture;
                    Ok::<_, ()>(damage.to_vec())
                });
            }
            _ => self.buffer_set_texture(buffer, renderer, texture),
        }
    }

    pub fn buffer_clear(&mut self, buffer: BufferId) {
        if let Some(b) = self.buffer_mut(buffer) {
            b.pixels = Pixels::None;
            b.buffer_size = Size::default();
        }
    }

    pub fn buffer_set_source_box(
        &mut self,
        buffer: BufferId,
        source: Option<Rectangle<f64, BufferCoords>>,
    ) {
        if let Some(b) = self.buffer_mut(buffer) {
            b.source = source.filter(|s| s.size.w > 0.0 && s.size.h > 0.0);
        }
    }

    pub fn buffer_set_dest_size(&mut self, buffer: BufferId, size: Option<Size<i32, Logical>>) {
        if let Some(b) = self.buffer_mut(buffer) {
            b.dest_size = size.filter(|s| s.w > 0 && s.h > 0);
        }
    }

    /// Where the buffer is opaque, node-local, logical. Opaque parts let
    /// the renderer skip what lies beneath.
    pub fn buffer_set_opaque_region(
        &mut self,
        buffer: BufferId,
        region: Vec<Rectangle<i32, Logical>>,
    ) {
        if let Some(b) = self.buffer_mut(buffer) {
            b.opaque = region;
        }
    }

    pub fn buffer_size(&self, buffer: BufferId) -> Option<Size<i32, Logical>> {
        self.buffer_ref(buffer).map(|b| b.size())
    }

    pub fn buffer_has_pixels(&self, buffer: BufferId) -> bool {
        self.buffer_ref(buffer)
            .is_some_and(|b| !matches!(b.pixels, Pixels::None))
    }

    pub fn surface(&self, node: SurfaceId) -> Option<&WlSurface> {
        match self.node(node.0).map(|n| &n.content) {
            Some(Content::Surface(s)) => Some(&s.surface),
            _ => None,
        }
    }

    /// A surface node showing `surface`, if one does; any one of them
    /// when several do.
    pub fn surface_node(&self, surface: &WlSurface) -> Option<SurfaceId> {
        self.live().find_map(|(id, n)| match &n.content {
            Content::Surface(s) if s.surface == *surface => Some(SurfaceId(id)),
            _ => None,
        })
    }

    pub fn custom(&self, node: CustomId) -> Option<&dyn CustomNode<R>> {
        match self.node(node.0).map(|n| &n.content) {
            Some(Content::Custom(c)) => Some(&**c),
            _ => None,
        }
    }
}

impl<R: Renderer, D> Drop for Scene<R, D> {
    fn drop(&mut self) {
        let root = self.root;
        self.destroy(root);
    }
}

/// `clip` narrowed by `r`; an empty result keeps its place so nothing passes it.
pub(crate) fn meet(
    clip: Option<Rectangle<i32, Logical>>,
    r: Rectangle<i32, Logical>,
) -> Rectangle<i32, Logical> {
    match clip {
        None => r,
        Some(c) => c
            .intersection(r)
            .unwrap_or_else(|| Rectangle::new(c.loc, Size::default())),
    }
}
