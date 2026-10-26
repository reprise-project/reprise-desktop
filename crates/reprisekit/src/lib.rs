//! Reprise's widget toolkit: everything a Reprise program needs to draw
//! itself, with no compositor or display server in sight. Everything
//! renders into a CPU [`raster::Canvas`] that the host (the compositor, or
//! a Wayland client) puts on screen however it does.
//!
//! The root modules are shared by every interface: the canvas, text,
//! the theme data types, icons, the single-line edit model, selection,
//! fuzzy matching, the taskbar's model, the window frame's shape and the
//! accessibility tree. [`style`] is the parts-and-states engine every
//! skinned look draws through, [`paint`] holds the look a theme draws in
//! and the paint a widget gets, and [`look`] picks which look paints a
//! frame or the taskbar. [`interfaces`] builds the theme each interface
//! starts from and draws the task preview in its look. [`controls`] holds
//! the look-neutral controls, [`art`] the icons and glyphs every look
//! shares and [`schemes`] the colour schemes.
//! [`pointer`] tracks the part the pointer rests on or holds, [`cues`]
//! whether a window shows its keyboard cues.
//! [`metrics`] holds the sizes a look lays its common controls out with,
//! [`taskpreview`] the task preview panel's layout and hit tests,
//! [`overview`] the window overview's model, layout and hit test.
//! [`trackbar`] holds the trackbar's model, layout and hit test.
//! [`flip`] holds the 3D switcher's stack: layout, perspective, hit test.
//! [`startpanel`] holds the Glass Start panel's model, layout and access.
//! [`wallpaper`] places a wallpaper on a screen, [`wallpapers`] draws the
//! built-in ones, [`saver`] runs and draws
//! the screen savers.
//! [`toolkit`] writes the GTK3 theme and Qt palette that recolour other
//! programs in the scheme in force.
//! [`shape`] draws coverage, gradients and glyphs for the looks that
//! paint their parts from geometry at the output scale.
//! [`classic`] holds the paint drawn in the Classic look; [`blue`] holds
//! Reprise Blue's own palette and painters.

// First, so every module after it can call `tr!`.
#[macro_use]
pub mod i18n;

pub mod access;
pub mod anim;
pub mod art;
pub mod backdrop;
pub mod blue;
pub mod boxdraw;
pub mod classic;
pub mod colorfilter;
pub mod controls;
pub mod copyart;
pub mod cues;
pub mod emoji;
pub mod flip;
pub mod fluid;
pub mod flyout;
mod fontcache;
mod fontfallback;
pub mod frame;
pub mod fuzzy;
pub mod glass;
pub mod glob;
pub mod icon;
pub mod icontheme;
pub mod imeclient;
pub mod interface;
pub mod interfaces;
pub mod look;
pub mod metrics;
mod mutex;
pub mod overview;
pub mod paint;
#[cfg(test)]
mod pins;
pub mod pointer;
pub mod raster;
pub mod saver;
pub mod schemes;
pub mod selection;
pub mod shape;
pub mod snaplayouts;
pub mod startpanel;
pub mod style;
pub mod switcher;
pub mod taskbar;
pub mod taskbarfocus;
pub mod taskpreview;
pub mod text;
pub mod textedit;
pub mod theme;
mod themecache;
pub mod toolkit;
pub mod trackbar;
pub mod trayflyout;
pub mod wallpaper;
pub mod wallpapers;
pub mod water;
