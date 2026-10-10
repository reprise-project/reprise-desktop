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

impl<R: Renderer, D> Scene<R, D> {
    pub(crate) fn surface_nodes(&self) -> Vec<(NodeId, WlSurface)> {
        self.live()
            .filter_map(|(id, n)| match &n.content {
                Content::Surface(s) => Some((id, s.surface.clone())),
                _ => None,
            })
            .collect()
    }

    fn surfaces_on(&self, output: &Output) -> Vec<(WlSurface, Option<Output>)> {
        let Some(area) = output_rect(output) else {
            return Vec::new();
        };
        self.surface_nodes()
            .into_iter()
            .filter(|(id, _)| self.extents(*id).is_some_and(|e| e.overlaps(area)))
            .map(|(id, s)| (s, self.primary_output(id)))
            .collect()
    }

    pub fn update_surfaces(&mut self, output: &Output, states: &RenderElementStates) {
        let area = output_rect(output);
        for (id, surface) in self.surface_nodes() {
            let Some((pos, visible)) = self.coords(id) else {
                continue;
            };
            let overlap = area
                .filter(|_| visible)
                .and_then(|a| a.intersection(bbox_from_surface_tree(&surface, pos)))
                .map(|mut r| {
                    r.loc -= pos;
                    r
                });
            output_update(output, overlap, &surface);
            let primary = if visible {
                self.primary_output(id)
            } else {
                None
            };
            let scale = output.current_scale();
            with_surfaces_surface_tree(&surface, |s, data| {
                update_surface_primary_scanout_output(
                    s,
                    output,
                    data,
                    None,
                    states,
                    prefer(primary.as_ref()),
                );
                if primary.as_ref() == Some(output) {
                    with_fractional_scale(data, |f| {
                        f.set_preferred_scale(scale.fractional_scale())
                    });
                    send_surface_state(s, data, scale.integer_scale(), output.current_transform());
                }
            });
            if let Some(Content::Surface(s)) = self.node_untracked(id).map(|n| &mut n.content) {
                s.entered.retain(|w| w.is_alive() && w != output);
                if overlap.is_some() {
                    s.entered.push(output.downgrade());
                }
            }
        }
    }

    pub fn send_frames(&self, output: &Output, time: Duration, throttle: Option<Duration>) {
        for (surface, primary) in self.surfaces_on(output) {
            send_frames_surface_tree(&surface, output, time, throttle, |_, _| primary.clone());
        }
    }

    pub fn take_presentation_feedback(
        &self,
        output: &Output,
        states: &RenderElementStates,
    ) -> OutputPresentationFeedback {
        let mut feedback = OutputPresentationFeedback::new(output);
        for (surface, primary) in self.surfaces_on(output) {
            take_presentation_feedback_surface_tree(
                &surface,
                &mut feedback,
                |_, _| primary.clone(),
                |s, _| surface_presentation_feedback_flags_from_states(s, None, states),
            );
        }
        feedback
    }
}
