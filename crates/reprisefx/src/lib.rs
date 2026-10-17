//! Reprise Glass's backdrop: what lies behind glass chrome, blurred,
//! streaked and tinted, drawn by GLES shaders as one Smithay render element
//! per piece of glass.
//!
//! # How a frame draws glass
//!
//! Without a [`Source`], [`GlassElement`] is a framebuffer effect
//! (`Element::is_framebuffer_effect`). When anything below it changed within
//! its geometry, or the element did, the damage tracker damages its whole
//! geometry and calls `capture_framebuffer` before `draw`:
//!
//! 1. capture: blit the element's part of the framebuffer, grown to the
//!    downscaled grid ([`blur::capture_rect`]), into a half-resolution
//!    texture (a quarter for the coarse blur or at output scale 2 and up);
//!    two Gaussian passes
//!    ([`blur::SIGMA`] per `[glass] blur`); blend the streaks
//!    (`reprisekit::glass::streak`) over the result. The textures live in
//!    the tracker's per-element cache and are remade only when their size changes.
//! 2. draw: each [`Shape`] draws the blurred picture through the glass
//!    program: a rounded rectangle with per-corner radii (anti-aliased
//!    distance field), holes left out of the drawn damage, and the tint of
//!    [`tint`] per pixel.
//!
//! The element's geometry is its shapes' bounds grown by [`blur::reach`], so
//! a change that close recaptures it. A frame with no damage under the glass
//! captures nothing and draws from the cached picture.
//!
//! A capture sees everything beneath the element, glass included. A host
//! that keeps chrome out of glass draws its own backdrop per output and blurs
//! it once into the output's [`Picture`], redrawing only the texels that read
//! a change. Glass given its [`Source`] is then a plain element drawing from
//! the picture, damaged only where the picture changed.
//!
//! # What the host does
//!
//! - Once per renderer: [`GlassFx::new`] (compiles the programs).
//! - Per piece of glass: a [`Glass`] (one element Id).
//! - Per output with its own backdrop: a [`Picture`]; after each backdrop
//!   render, [`Picture::update`] with where it changed.
//! - Per frame and output: [`Glass::element`] with a [`GlassFrame`] (shapes
//!   and holes in output-local physical pixels, `[glass]` settings, tint
//!   surface, output size and scale, opacity, quality, hold, the picture's
//!   source if any). Put the element directly below the chrome it backs.
//! - Glass beneath other glass, drawn into that glass's own backdrop: a
//!   second [`Glass`] with [`GlassFrame::downscale`] the
//!   [`blur::backdrop_downscale`], the shapes laid out on the output, and a
//!   source.
//!
//! `[glass] fast_drag`: pass [`blur::quality`]`(dragging, fast_drag)`. A
//! quality change changes the element's commit, so the drag's end re-blurs
//! at full quality in the next frame.
//!
//! `[glass] blur_rate`: while only client commits changed what lies under
//! glass and its last blur is younger than `1 / blur_rate` seconds, pass
//! `hold: true` to [`Picture::update`]: the picture keeps its blur and the
//! changes, and [`Picture::is_stale`] turns true. When the cap's timer
//! fires, call [`Picture::refresh`] and schedule a frame. Glass that copies
//! the frame holds through [`GlassFrame::hold`], [`Glass::is_stale`] and
//! [`Glass::refresh`]; a move, resize or new blur captures even when held.
//!
//! # Materials
//!
//! Frosted and Smoked differ only in the tint's colour, shares and the
//! streaks' gain. Molten blurs lighter ([`blur::MOLTEN_SIGMA`]) and its shapes
//! carry a [`Lens`]: the shader bends the backdrop lookup near the outer edge
//! along the distance field's normal and lights a rim ([`lens`]). The
//! element's geometry grows by the lens's reach too.
//!
//! # Rotated outputs and region captures
//!
//! A rotated or flipped output draws into a framebuffer in the panel's
//! orientation, so the picture lies in that orientation; the draw maps each
//! shape, its corners and the streaks through the frame's transform
//! (`space`). Shapes and holes are kept relative to the element (`dst`), so a
//! region capture that relocates every element shows the glass where it is
//! on the output.
//!
//! # Other effects
//!
//! - [`flip`]: the 3D switcher's tilted cards, a window's picture in perspective.
//! - [`colorfx`]: the screen's colour filter, the frame through one colour map per pixel.
//! - [`water`]: desktop water's height field. A [`Pool`] per output holds it
//!   in two packed RGBA8 textures that take turns, steps it with the frame's
//!   splashes (`reprisekit::water`) while windows hold their texels flat, and
//!   makes its normals once per frame. [`WaterFx`] holds the programs, once per renderer.
//! - [`fluid`]: the fluid styles' stable-fluids sim: a [`FluidPool`] of half-float grids per
//!   output, stepped with the frame's forces (`reprisekit::fluid`). [`FluidFx`] holds the programs.
//! - [`strokes`]: flat bars at any angle in one batch, the Starfield's stars and Mystify's lines.

pub mod blur;
pub mod colorfx;
mod element;
pub mod flip;
pub mod fluid;
mod fluid_gl;
mod fluid_shaders;
pub mod headless;
pub mod lens;
mod picture;
mod shaders;
mod space;
pub mod strokes;
pub mod tint;
pub mod water;

pub use blur::Quality;
pub use colorfx::{ColorElement, ColorFx};
pub use element::{Glass, GlassElement, GlassFrame, GlassFx, Shape};
pub use flip::{FlipCard, FlipCardElement, FlipFx, mipmaps};
pub use fluid::{FluidElement, FluidFx, FluidPool};
pub use lens::Lens;
pub use picture::{Drawing, Picture, Source, Update};
pub use strokes::{Stroke, StrokeElement, StrokeFx, Strokes};
pub use tint::Tint;
pub use water::{Pool, WaterElement, WaterFx};
