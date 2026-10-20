//! Node extents, the node at a point, and which outputs a node is on.

use smithay::{
    backend::renderer::Renderer,
    desktop::{
        WindowSurfaceType,
        utils::{bbox_from_surface_tree, under_from_surface_tree},
    },
    output::Output,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point, Rectangle},
};

use crate::{
    Scene,
    node::{Content, Node, NodeId},
};

#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub node: NodeId,
    /// The point from the leaf's top-left, logical; for a surface tree, from the (sub)surface hit.
    pub local: Point<f64, Logical>,
    /// For a surface tree, the (sub)surface whose input region took the point.
    pub surface: Option<WlSurface>,
}

/// An output's box in layout coordinates: position and mode, transformed and divided by scale. `None` without a mode.
pub fn output_rect(output: &Output) -> Option<Rectangle<i32, Logical>> {
    let mode = output.current_mode()?;
    let size = output
        .current_transform()
        .transform_size(mode.size)
        .to_f64()
        .to_logical(output.current_scale().fractional_scale())
        .to_i32_round();
    Some(Rectangle::new(output.current_location(), size))
}

impl<R: Renderer, D> Scene<R, D> {
    /// A leaf's box in layout coordinates, `pos` being its layout position. Trees have none.
    pub(crate) fn leaf_rect(
        n: &Node<R, D>,
        pos: Point<i32, Logical>,
    ) -> Option<Rectangle<i32, Logical>> {
        let r = match &n.content {
            Content::Tree(_) => return None,
            Content::Rect(r) => Rectangle::new(pos, r.size),
            Content::Buffer(b) => Rectangle::new(pos, b.size()),
            Content::Surface(s) => bbox_from_surface_tree(&s.surface, pos),
            Content::Custom(c) => Rectangle::new(pos, c.size()),
        };
        (!r.is_empty()).then_some(r)
    }

    /// The box around everything `node` shows; `None` when it shows nothing or is hidden (itself or an ancestor disabled).
    pub fn extents(&self, node: impl Into<NodeId>) -> Option<Rectangle<i32, Logical>> {
        let node = node.into();
        let (pos, visible) = self.coords(node)?;
        if !visible {
            return None;
        }
        let mut out: Option<Rectangle<i32, Logical>> = None;
        self.extents_under(node, pos - self.node(node)?.position, &mut out);
        out
    }

    fn extents_under(
        &self,
        id: NodeId,
        origin: Point<i32, Logical>,
        out: &mut Option<Rectangle<i32, Logical>>,
    ) {
        let Some(n) = self.node(id).filter(|n| n.enabled) else {
            return;
        };
        let pos = origin + n.position;
        if let Content::Tree(children) = &n.content {
            for c in children {
                self.extents_under(*c, pos, out);
            }
        } else if let Some(r) = Self::leaf_rect(n, pos) {
            *out = Some(out.map_or(r, |o| o.merge(r)));
        }
    }

    /// The topmost leaf at or under `from` that takes the pointer at `point`
    /// (layout coordinates). Disabled nodes, nodes hidden at `point` ([`Scene::set_hidden_in`])
    /// and [`Input::Passthrough`](crate::Input::Passthrough) leaves are skipped; a surface tree
    /// takes the point only inside an input region.
    /// `from`'s ancestors are not checked for enabled; their clips apply.
    pub fn node_at(&self, from: impl Into<NodeId>, point: Point<f64, Logical>) -> Option<Hit> {
        let from = from.into();
        if self
            .clip_above(from)
            .is_some_and(|c| !c.contains(point.to_i32_floor()))
        {
            return None;
        }
        self.at(from, self.origin_of(from), point)
    }

    fn at(
        &self,
        id: NodeId,
        origin: Point<i32, Logical>,
        point: Point<f64, Logical>,
    ) -> Option<Hit> {
        let at = Rectangle::new(point.to_i32_floor(), (1, 1).into());
        let n = self.node(id).filter(|n| n.enabled && !n.hidden_over(at))?;
        let pos = origin + n.position;
        let local = point - pos.to_f64();
        match &n.content {
            Content::Tree(_) if n.clip.is_some_and(|c| !c.contains(local.to_i32_floor())) => None,
            Content::Tree(children) => children.iter().rev().find_map(|c| self.at(*c, pos, point)),
            Content::Surface(s) => {
                if !n.input.accepts(local) {
                    return None;
                }
                let (surface, at) =
                    under_from_surface_tree(&s.surface, point, pos, WindowSurfaceType::ALL)?;
                Some(Hit {
                    node: id,
                    local: point - at.to_f64(),
                    surface: Some(surface),
                })
            }
            _ => {
                let r = Self::leaf_rect(n, pos)?;
                (r.contains(point.to_i32_floor()) && n.input.accepts(local)).then_some(Hit {
                    node: id,
                    local,
                    surface: None,
                })
            }
        }
    }

    /// Show the scene on `output`, after earlier outputs (the order breaks ties for
    /// [`Scene::primary_output`]). Position, mode, scale and transform are read when needed.
    pub fn add_output(&mut self, output: &Output) {
        if !self.outputs.contains(output) {
            self.outputs.push(output.clone());
        }
    }

    pub fn remove_output(&mut self, output: &Output) {
        self.outputs.retain(|o| o != output);
        crate::surface::leave_output(self, output);
    }

    pub fn outputs(&self) -> &[Output] {
        &self.outputs
    }

    pub fn outputs_of(&self, node: impl Into<NodeId>) -> Vec<Output> {
        let Some(e) = self.extents(node) else {
            return Vec::new();
        };
        self.outputs
            .iter()
            .filter(|o| output_rect(o).is_some_and(|r| r.overlaps(e)))
            .cloned()
            .collect()
    }

    /// The output showing the largest part of `node`, ties to the first added; `None` if none shows it.
    pub fn primary_output(&self, node: impl Into<NodeId>) -> Option<Output> {
        let e = self.extents(node)?;
        let mut best: Option<(i64, &Output)> = None;
        for o in &self.outputs {
            let Some(i) = output_rect(o).and_then(|r| r.intersection(e)) else {
                continue;
            };
            let area = i.size.w as i64 * i.size.h as i64;
            if best.is_none_or(|(a, _)| area > a) {
                best = Some((area, o));
            }
        }
        best.map(|(_, o)| o.clone())
    }
}
