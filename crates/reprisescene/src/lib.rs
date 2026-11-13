//! A retained scene tree for the compositor, turned into Smithay render
//! elements per output.
//!
//! A [`Scene`] holds a root tree; trees hold children bottom first. Leaves
//! are rects, buffers (a CPU canvas or a texture), client surface trees and
//! host-made custom nodes. Handles ([`NodeId`] and the typed [`TreeId`],
//! [`RectId`], [`BufferId`], [`SurfaceId`], [`CustomId`]) are plain values;
//! a verb on a destroyed node does nothing. A compositor keeps several
//! independent scenes (the desktop, a capture scene per window, offscreen
//! pictures).
//!
//! Each frame, [`Scene::render_elements`] gives the elements for an output
//! front to back, and Smithay's `OutputDamageTracker` works out the damage
//! from their Ids, commit counters and geometry. After the output renders,
//! [`Scene::update_surfaces`], [`Scene::send_frames`] and
//! [`Scene::take_presentation_feedback`] serve the client surfaces on it,
//! from one [`Scene::surface_pass`].
//!
//! A surface wholly under opaque leaves takes no frames from that output; the
//! primary output is the one drawing it whose overlap with its box is largest.

mod cover;
mod node;
mod query;
mod render;
mod scene;
mod surface;

pub use node::{
    BufferId, CustomElement, CustomId, CustomNode, Input, NodeId, NodeType, RectId, SurfaceId,
    TreeId,
};
pub use query::{Hit, copied, output_rect, set_copied};
pub use render::{BufferElement, SceneElement, View};
pub use scene::{Prepared, Scene};
pub use surface::SurfacePass;

#[cfg(test)]
mod tests;
