//! Which leaves an output draws none of: those lying wholly under opaque leaves above them.

use std::{cell::Cell, collections::HashSet};

use smithay::{
    backend::renderer::{Renderer, utils::RendererSurfaceStateUserData},
    output::Output,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point, Rectangle},
    wayland::{
        alpha_modifier::AlphaModifierSurfaceCachedState,
        compositor::{TraversalAction, with_surface_tree_downward},
    },
};

use crate::{
    Scene,
    node::{Content, Node, NodeId, Pixels},
    render::{Mode, Pass, View},
};

/// What an output shows ([`Scene::shows_of`]).
pub(crate) struct Shows {
    pub(crate) output: Output,
    /// The leaves it draws part of and every tree above one.
    pub(crate) shown: HashSet<NodeId>,
    /// Every leaf on it, covered or not.
    pub(crate) walked: HashSet<NodeId>,
}

/// Per output, the surface nodes it draws none of ([`Scene::covered_on`]).
pub(crate) struct Covered(Vec<(Output, Vec<NodeId>)>);

impl Covered {
    pub(crate) fn on(&self, node: NodeId, output: &Output) -> bool {
        (self.0.iter()).any(|(o, nodes)| o == output && nodes.contains(&node))
    }
}

impl<R: Renderer, D> Scene<R, D> {
    pub(crate) fn covered(&self) -> Covered {
        Covered(
            (self.outputs.iter())
                .map(|o| (o.clone(), self.covered_on(o)))
                .collect(),
        )
    }

    /// The surface nodes `output` draws none of ([`Scene::leaves_on`]).
    pub(crate) fn covered_on(&self, output: &Output) -> Vec<NodeId> {
        let Leaves::All(leaves) = self.leaves_on(output, None) else {
            return Vec::new();
        };
        (leaves.into_iter())
            .filter(|l| l.surface && l.covered)
            .map(|l| l.id)
            .collect()
    }

    /// The outputs where part of `nodes` shows: those [`Scene::outputs_of`] lists, less those on
    /// which every leaf under `nodes` is covered by the rule [`Scene::outputs_showing`] applies
    /// to surfaces.
    pub fn outputs_drawing(&self, nodes: &[NodeId]) -> Vec<Output> {
        let mut out: Vec<Output> = Vec::new();
        for &n in nodes {
            for o in self.outputs_of(n) {
                if !out.contains(&o) {
                    out.push(o);
                }
            }
        }
        out.retain(|o| match self.seek_on(o, &|n| nodes.contains(&n)) {
            Seek::NoView | Seek::Shown => true,
            Seek::Not(s) => nodes.iter().any(|n| s.shown.contains(n)),
        });
        out
    }

    /// Whether `output` shows part of a node `sought` takes, by [`Scene::shows_of`]'s rule. The
    /// walk ends at the first leaf of one that lies outside the opaque leaves above it; when none
    /// does, [`Seek::Not`] holds what the output shows, which may still hold one (glass reads it).
    pub(crate) fn seek_on(&self, output: &Output, sought: &dyn Fn(NodeId) -> bool) -> Seek {
        match self.leaves_on(output, Some(&|id| self.draws_for(id, sought))) {
            Leaves::NoView => Seek::NoView,
            Leaves::Stopped => Seek::Shown,
            Leaves::All(leaves) => Seek::Not(self.shows_of(output, leaves)),
        }
    }

    /// The nodes `output` shows part of: each leaf not covered ([`Scene::leaves_on`]), the trees
    /// above it and, for a hoisted node, the tree it draws in and those above.
    fn shows_of(&self, output: &Output, leaves: Vec<Leaf>) -> Shows {
        let mut s = Shows {
            output: output.clone(),
            shown: HashSet::new(),
            walked: HashSet::new(),
        };
        let parent = |id: NodeId| self.node(id).and_then(|n| n.parent).map(|p| p.0);
        for l in leaves {
            s.walked.insert(l.id);
            if l.covered {
                continue;
            }
            let mut at = Some(l.id);
            while let Some(id) = at.filter(|&id| s.shown.insert(id)) {
                at = parent(id);
            }
        }
        if let Some((into, hoisted)) = &self.hoist
            && hoisted.iter().any(|h| s.shown.contains(h))
        {
            s.shown
                .extend(std::iter::successors(Some(*into), |&id| parent(id)));
        }
        s
    }

    /// Whether leaf `id` draws as part of a node `of` takes: itself, a tree above it or, for a
    /// hoisted node, the tree it draws in or one above that.
    fn draws_for(&self, id: NodeId, of: &dyn Fn(NodeId) -> bool) -> bool {
        let up = |from| std::iter::successors(Some(from), |&n| self.node(n)?.parent.map(|p| p.0));
        let mut hoisted = false;
        for n in up(id) {
            if of(n) {
                return true;
            }
            hoisted |= self.hoist.as_ref().is_some_and(|(_, h)| h.contains(&n));
        }
        hoisted
            && self
                .hoist
                .as_ref()
                .is_some_and(|(into, _)| up(*into).any(of))
    }

    /// The leaves `output` shows part of, each covered when it lies under opaque leaves drawn
    /// above it at full opacity and outside what every custom node the output draws reads
    /// ([`CustomNode::reads_beneath`](crate::CustomNode::reads_beneath)). A custom node that
    /// cannot say reads all of the output beneath it. Ends at the first leaf outside the opaque
    /// leaves above it that `stop` takes.
    fn leaves_on(&self, output: &Output, stop: Option<&dyn Fn(NodeId) -> bool>) -> Leaves {
        let Some(view) = View::of(output) else {
            return Leaves::NoView;
        };
        let stopped = Cell::new(false);
        let inset = rounding_inset(view.scale.x);
        let mut opaque: Vec<Rectangle<i32, Logical>> = Vec::new();
        let mut shown: Vec<Shown> = Vec::new();
        let mut customs: Vec<Custom> = Vec::new();
        // The opaque rects meeting the leaf: the others subtract nothing.
        let mut near: Vec<Rectangle<i32, Logical>> = Vec::new();
        let mut order = 0;
        let pass = Pass {
            view: &view,
            mode: Mode::Screen,
            hoist: true,
            stop: Some(&stopped),
        };
        self.walk(
            self.root().node(),
            Point::default(),
            1.0,
            false,
            pass,
            None,
            &mut |id, n, pos, alpha, _, clip| {
                let cut = |r: Rectangle<i32, Logical>| clip.map_or(Some(r), |c| r.intersection(c));
                let Some(rect) = (Self::leaf_rect(n, pos))
                    .and_then(|r| r.intersection(view.area))
                    .and_then(cut)
                else {
                    return;
                };
                order += 1;
                near.clear();
                near.extend(opaque.iter().filter(|o| o.overlaps(rect)));
                let under =
                    !near.is_empty() && rect.subtract_rects(near.iter().copied()).is_empty();
                if !under && stop.is_some_and(|f| f(id)) {
                    stopped.set(true);
                    return;
                }
                shown.push(Shown {
                    id,
                    surface: matches!(n.content, Content::Surface(_)),
                    rect,
                    under,
                    order,
                });
                if let Content::Custom(c) = &n.content {
                    customs.push(Custom {
                        shown: rect,
                        hidden: under,
                        order,
                        reads: (c.reads_beneath(view.scale.x)).map(|rs| {
                            (rs.into_iter())
                                .map(|r| Rectangle::new(r.loc + pos, r.size))
                                .collect()
                        }),
                    });
                }
                if under || alpha < 1.0 {
                    return;
                }
                for r in opaque_parts(n, pos) {
                    let r = cut(r).and_then(|r| shrink(r, inset, view.area));
                    opaque.extend(r.and_then(|r| r.intersection(view.area)));
                }
            },
        );
        if stopped.get() {
            return Leaves::Stopped;
        }
        let (reads, from) = reads(customs);
        Leaves::All(
            (shown.into_iter())
                .map(|s| Leaf {
                    id: s.id,
                    surface: s.surface,
                    covered: s.under && s.order < from && !reads.iter().any(|r| r.overlaps(s.rect)),
                })
                .collect(),
        )
    }
}

/// What [`Scene::leaves_on`] walked.
enum Leaves {
    NoView,
    /// A leaf its `stop` took.
    Stopped,
    All(Vec<Leaf>),
}

/// What [`Scene::seek_on`] found.
pub(crate) enum Seek {
    NoView,
    Shown,
    Not(Shows),
}

/// A leaf in walk order (front first) and the part of it on the output.
struct Shown {
    id: NodeId,
    surface: bool,
    rect: Rectangle<i32, Logical>,
    /// Wholly under opaque leaves above it.
    under: bool,
    order: usize,
}

/// A leaf an output shows part of ([`Scene::leaves_on`]).
struct Leaf {
    id: NodeId,
    surface: bool,
    /// Drawn by none of the output's pixels.
    covered: bool,
}

/// A custom node the output shows part of, in walk order (front first).
struct Custom {
    shown: Rectangle<i32, Logical>,
    /// Wholly under opaque leaves on the screen.
    hidden: bool,
    order: usize,
    /// Layout; None for anywhere beneath.
    reads: Option<Vec<Rectangle<i32, Logical>>>,
}

/// What the custom nodes drawn read: the rectangles glass reads, and the walk order past which
/// everything is read. Glass reads a backdrop that can hold nodes stacked above it and leave
/// opaque chrome out, so its reads count at any depth; glass hidden on the screen still draws
/// into the backdrop of glass reading it.
fn reads(mut customs: Vec<Custom>) -> (Vec<Rectangle<i32, Logical>>, usize) {
    let mut reads: Vec<Rectangle<i32, Logical>> = Vec::new();
    let mut from = usize::MAX;
    while let Some(i) =
        (customs.iter()).position(|c| !c.hidden || reads.iter().any(|r| r.overlaps(c.shown)))
    {
        let c = customs.swap_remove(i);
        match c.reads {
            Some(rs) => reads.extend(rs),
            None => from = from.min(c.order),
        }
    }
    (reads, from)
}

/// The layout rectangles leaf `n` at `pos` draws opaque at full opacity, as its elements report
/// them; none for a custom node or a buffer with no opaque region set.
fn opaque_parts<R: Renderer, D>(
    n: &Node<R, D>,
    pos: Point<i32, Logical>,
) -> Vec<Rectangle<i32, Logical>> {
    match &n.content {
        Content::Rect(r) if r.buffer.color().is_opaque() => vec![Rectangle::new(pos, r.size)],
        Content::Buffer(b) if !matches!(b.pixels, Pixels::None) => (b.opaque.iter())
            .filter_map(|r| r.intersection(Rectangle::from_size(b.size())))
            .map(|r| Rectangle::new(r.loc + pos, r.size))
            .collect(),
        Content::Surface(s) => surface_opaque(&s.surface, pos),
        _ => Vec::new(),
    }
}

/// The opaque regions of `surface` and its mapped subsurfaces, the surface's origin at `pos`. As
/// in Smithay's surface element, a surface its alpha modifier fades below 1 has none of its own.
fn surface_opaque(surface: &WlSurface, pos: Point<i32, Logical>) -> Vec<Rectangle<i32, Logical>> {
    let mut out = Vec::new();
    with_surface_tree_downward(
        surface,
        pos,
        |_, states, loc: &Point<i32, Logical>| {
            let Some(data) = states.data_map.get::<RendererSurfaceStateUserData>() else {
                return TraversalAction::SkipChildren;
            };
            let data = data.lock().unwrap();
            let Some(view) = data.view() else {
                return TraversalAction::SkipChildren;
            };
            let loc = *loc + view.offset;
            let faded = (states.cached_state.get::<AlphaModifierSurfaceCachedState>())
                .current()
                .multiplier_f32()
                .is_some_and(|m| m < 1.0);
            if !faded {
                let regions = data.opaque_regions().unwrap_or_default();
                out.extend(regions.iter().map(|r| Rectangle::new(r.loc + loc, r.size)));
            }
            TraversalAction::DoChildren(loc)
        },
        |_, _, _| {},
        |_, _, _| true,
    );
    out
}

/// How far, logical, an opaque edge inside the view moves in at `scale`: elements round their
/// edges to whole physical pixels, which at a fractional scale can move them by one.
fn rounding_inset(scale: f64) -> i32 {
    if scale.fract() == 0.0 {
        0
    } else {
        (1.0 / scale).ceil() as i32
    }
}

/// `r` with each edge inside `area` moved `inset` in; None when nothing is left.
fn shrink(
    r: Rectangle<i32, Logical>,
    inset: i32,
    area: Rectangle<i32, Logical>,
) -> Option<Rectangle<i32, Logical>> {
    let (a0, a1) = (area.loc, area.loc + area.size.to_point());
    let (mut p0, mut p1) = (r.loc, r.loc + r.size.to_point());
    if p0.x > a0.x {
        p0.x += inset;
    }
    if p0.y > a0.y {
        p0.y += inset;
    }
    if p1.x < a1.x {
        p1.x -= inset;
    }
    if p1.y < a1.y {
        p1.y -= inset;
    }
    (p1.x > p0.x && p1.y > p0.y).then(|| Rectangle::from_extremities(p0, p1))
}
