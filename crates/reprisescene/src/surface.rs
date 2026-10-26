//! After an output renders: output enter/leave, primary output, preferred scale, frame callbacks
//! and presentation feedback for client surfaces, via Smithay's surface-tree helpers.

use std::time::Duration;

use smithay::{
    backend::renderer::{
        Renderer,
        element::{RenderElementState, RenderElementStates},
    },
    desktop::utils::{
        OutputPresentationFeedback, bbox_from_surface_tree, output_update,
        send_frames_surface_tree, surface_presentation_feedback_flags_from_states,
        take_presentation_feedback_surface_tree, update_surface_primary_scanout_output,
        with_surfaces_surface_tree,
    },
    output::Output,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Rectangle},
    wayland::{compositor::send_surface_state, fractional_scale::with_fractional_scale},
};

use crate::{
    Scene,
    node::{Content, NodeId, SurfaceNode},
    query::output_rect,
};

pub(crate) fn leave_all(s: &SurfaceNode) {
    for o in s.entered.iter().filter_map(|w| w.upgrade()) {
        output_update(&o, None, &s.surface);
    }
}

pub(crate) fn leave_output<R: Renderer, D>(scene: &mut Scene<R, D>, output: &Output) {
    for (id, surface) in scene.surface_nodes() {
        output_update(output, None, &surface);
        if let Some(Content::Surface(s)) = scene.node_untracked(id).map(|n| &mut n.content) {
            s.entered.retain(|w| w != output);
        }
    }
}

/// Smithay's primary-scanout compare, keeping the scene's own choice (largest overlap, ties to the
/// first-added output) once an element was presented on it.
fn prefer(
    primary: Option<&Output>,
) -> impl for<'a> Fn(
    &'a Output,
    &'a RenderElementState,
    &'a Output,
    &'a RenderElementState,
) -> &'a Output
+ '_ {
    move |current, _, next, _| if primary == Some(next) { next } else { current }
}

/// The client surfaces one frame of an output serves ([`Scene::surface_pass`]).
pub struct SurfacePass {
    output: Output,
    surfaces: Vec<PassSurface>,
}

struct PassSurface {
    id: NodeId,
    surface: WlSurface,
    /// The part of the output it shows, from its own top-left.
    overlap: Option<Rectangle<i32, Logical>>,
    primary: Option<Output>,
}

impl SurfacePass {
    fn shown(&self) -> impl Iterator<Item = &PassSurface> {
        self.surfaces.iter().filter(|p| p.overlap.is_some())
    }
}

impl<R: Renderer, D> Scene<R, D> {
    pub(crate) fn surface_nodes(&self) -> Vec<(NodeId, WlSurface)> {
        self.live()
            .filter_map(|(id, n)| match &n.content {
                Content::Surface(s) => Some((id, s.surface.clone())),
                _ => None,
            })
            .collect()
    }

    /// The outputs a commit of `surface` can change: those its nodes overlap now and those that
    /// showed it in their last frame. `None` when no node shows it.
    pub fn outputs_showing(&self, surface: &WlSurface) -> Option<Vec<Output>> {
        let mut found = false;
        let mut out: Vec<Output> = Vec::new();
        for (id, n) in self.live() {
            let Content::Surface(s) = &n.content else {
                continue;
            };
            if s.surface != *surface {
                continue;
            }
            found = true;
            let entered = s.entered.iter().filter_map(|w| w.upgrade());
            for o in self.outputs_of(id).into_iter().chain(entered) {
                if !out.contains(&o) {
                    out.push(o);
                }
            }
        }
        found.then_some(out)
    }

    /// The box around everything the nodes showing `surface` show; `None` when none is shown.
    pub fn surface_extents(&self, surface: &WlSurface) -> Option<Rectangle<i32, Logical>> {
        (self.live())
            .filter(|(_, n)| matches!(&n.content, Content::Surface(s) if s.surface == *surface))
            .filter_map(|(id, _)| self.extents(id))
            .reduce(|a, b| a.merge(b))
    }

    /// The client surfaces a frame of `output` serves, worked out once for
    /// [`Scene::update_surfaces`], [`Scene::take_presentation_feedback`] and
    /// [`Scene::send_frames`]. A surface away from the output that never entered it is left out.
    pub fn surface_pass(&self, output: &Output) -> SurfacePass {
        let area = output_rect(output);
        let mut surfaces = Vec::new();
        for (id, n) in self.live() {
            let Content::Surface(s) = &n.content else {
                continue;
            };
            let Some((pos, visible)) = self.coords(id) else {
                continue;
            };
            let extents =
                Some(bbox_from_surface_tree(&s.surface, pos)).filter(|e| visible && !e.is_empty());
            let overlap = area.zip(extents).and_then(|(a, e)| a.intersection(e));
            if overlap.is_none() && !s.entered.iter().any(|w| w == output) {
                continue;
            }
            surfaces.push(PassSurface {
                id,
                surface: s.surface.clone(),
                overlap: overlap.map(|mut r| {
                    r.loc -= pos;
                    r
                }),
                primary: extents.and_then(|e| self.primary_in(e)),
            });
        }
        SurfacePass {
            output: output.clone(),
            surfaces,
        }
    }

    pub fn update_surfaces(&mut self, pass: &SurfacePass, states: &RenderElementStates) {
        let output = &pass.output;
        let scale = output.current_scale();
        for p in &pass.surfaces {
            output_update(output, p.overlap, &p.surface);
            with_surfaces_surface_tree(&p.surface, |s, data| {
                update_surface_primary_scanout_output(
                    s,
                    output,
                    data,
                    None,
                    states,
                    prefer(p.primary.as_ref()),
                );
                if p.primary.as_ref() == Some(output) {
                    with_fractional_scale(data, |f| {
                        f.set_preferred_scale(scale.fractional_scale())
                    });
                    send_surface_state(s, data, scale.integer_scale(), output.current_transform());
                }
            });
            if let Some(Content::Surface(s)) = self.node_untracked(p.id).map(|n| &mut n.content) {
                s.entered.retain(|w| w.is_alive() && w != output);
                if p.overlap.is_some() {
                    s.entered.push(output.downgrade());
                }
            }
        }
    }

    pub fn send_frames(&self, pass: &SurfacePass, time: Duration, throttle: Option<Duration>) {
        let output = &pass.output;
        for p in pass.shown() {
            send_frames_surface_tree(&p.surface, output, time, throttle, |_, _| p.primary.clone());
        }
    }

    pub fn take_presentation_feedback(
        &self,
        pass: &SurfacePass,
        states: &RenderElementStates,
    ) -> OutputPresentationFeedback {
        let mut feedback = OutputPresentationFeedback::new(&pass.output);
        for p in pass.shown() {
            take_presentation_feedback_surface_tree(
                &p.surface,
                &mut feedback,
                |_, _| p.primary.clone(),
                |s, _| surface_presentation_feedback_flags_from_states(s, None, states),
            );
        }
        feedback
    }
}
