//! Which client surfaces an output draws none of: those lying wholly under opaque leaves above
//! them.

use smithay::{
    backend::renderer::{Renderer, utils::RendererSurfaceStateUserData},
    output::Output,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point, Rectangle},
    wayland::compositor::{TraversalAction, with_surface_tree_downward},
};

use crate::{
    Scene,
    node::{Content, Node, NodeId, Pixels},
    render::{Mode, Pass, View},
};

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

    /// The surface nodes `output` draws none of, each under opaque leaves drawn above it at full
    /// opacity. None below a custom node the output shows counts: glass blurs what lies beneath.
    pub(crate) fn covered_on(&self, output: &Output) -> Vec<NodeId> {
        let Some(view) = View::of(output) else {
            return Vec::new();
        };
        let inset = rounding_inset(view.scale.x);
        let mut opaque: Vec<Rectangle<i32, Logical>> = Vec::new();
        let mut under_custom = false;
        let mut out = Vec::new();
        let pass = Pass {
            view: &view,
            mode: Mode::Screen,
            hoist: true,
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
                let Some(shown) = (Self::leaf_rect(n, pos))
                    .and_then(|r| r.intersection(view.area))
                    .and_then(cut)
                else {
                    return;
                };
                if under_custom {
                    return;
                }
                let hidden = shown.subtract_rects(opaque.iter().copied()).is_empty();
                match &n.content {
                    Content::Surface(_) if hidden => out.push(id),
                    Content::Custom(_) if !hidden => under_custom = true,
                    _ => {}
                }
                if hidden || alpha < 1.0 {
                    return;
                }
                for r in opaque_parts(n, pos) {
                    let r = cut(r).and_then(|r| shrink(r, inset, view.area));
                    opaque.extend(r.and_then(|r| r.intersection(view.area)));
                }
            },
        );
        out
    }
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

/// The opaque regions of `surface` and its mapped subsurfaces, the surface's origin at `pos`.
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
            let regions = data.opaque_regions().unwrap_or_default();
            out.extend(regions.iter().map(|r| Rectangle::new(r.loc + loc, r.size)));
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
