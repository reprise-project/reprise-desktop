//! Window icons: the client's own (xdg-toplevel-icon, or an X11 window's
//! `_NET_WM_ICON` via `xwayland::helper`), else the app id's from the icon theme
//! or its `.desktop` entry (`taskbarview::app_icon`). Formats, resampling and the
//! fallback glyph are `reprisekit::icon`.

use std::sync::Arc;

use reprisekit::icon::{
    DRM_FORMAT_ARGB8888, DRM_FORMAT_XRGB8888, ICON_SIZE, IconSource, PREFERRED_SOURCE, convert,
};
use reprisekit::icontheme::IconCache;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::XdgToplevel;
use smithay::reexports::wayland_server::DisplayHandle;
use smithay::reexports::wayland_server::protocol::{
    wl_buffer::WlBuffer, wl_shm, wl_surface::WlSurface,
};
use smithay::wayland::compositor::with_states;
use smithay::wayland::shm::with_buffer_contents;
use smithay::wayland::xdg_toplevel_icon::{
    ToplevelIconCachedState, XdgToplevelIconHandler, XdgToplevelIconManager,
};

use crate::compositor::State;
use crate::raster::Canvas;
use crate::window::{Window, WindowId};

/// The xdg-toplevel-icon global, asking for the caption's size at scales 1 to 3.
pub fn manager(dh: &DisplayHandle) -> XdgToplevelIconManager {
    let mut m = XdgToplevelIconManager::new::<State>(dh);
    m.replace_icon_sizes([ICON_SIZE, 2 * ICON_SIZE, 3 * ICON_SIZE]);
    m
}

/// The icon `win` shows at `px`; None draws the fallback glyph. Shared with the
/// caches, so a redraw copies no pixels.
pub fn window_icon(
    icons: &mut IconCache,
    index: &[reprisefs::apps::AppEntry],
    win: &Window,
    px: i32,
) -> Option<Arc<Canvas>> {
    if let Some(icon) = win.own_icon(px) {
        return Some(icon);
    }
    crate::taskbarview::app_icon(icons, index, &win.app_id(), px)
}

/// Read a wl_shm icon buffer's pixels at their native size. None for an
/// unsupported format or a buffer that does not hold what it claims.
fn from_shm(buffer: &WlBuffer) -> Option<IconSource> {
    with_buffer_contents(buffer, |ptr, len, d| {
        let format = match d.format {
            wl_shm::Format::Argb8888 => DRM_FORMAT_ARGB8888,
            wl_shm::Format::Xrgb8888 => DRM_FORMAT_XRGB8888,
            // Every other wl_shm format code is its DRM fourcc.
            f => f as u32,
        };
        // SAFETY: `with_buffer_contents` hands over the pool's mapping,
        // readable for `len` bytes while this closure runs.
        let pool = unsafe { std::slice::from_raw_parts(ptr, len) };
        let (w, h) = (d.width.max(0) as usize, d.height.max(0) as usize);
        let pixels = convert(
            pool.get(d.offset.max(0) as usize..)?,
            w,
            h,
            d.stride.max(0) as usize,
            format,
        )?;
        Some(IconSource {
            width: w as u32,
            height: h as u32,
            pixels,
        })
    })
    .ok()
    .flatten()
}

fn width_of(buffer: &WlBuffer) -> i32 {
    with_buffer_contents(buffer, |_, _, d| d.width).unwrap_or(0)
}

impl State {
    /// A commit of `w`'s surface applies a newly set icon: the buffer whose width
    /// is closest to 32 px is kept at native size. None that reads falls back to the
    /// app id's.
    pub fn apply_icon(&mut self, w: WindowId) {
        let Some(win) = self.win_mut(w) else {
            return;
        };
        if !std::mem::take(&mut win.icon_pending) {
            return;
        }
        let source = with_states(win.wl_surface(), |s| {
            let mut cached = s.cached_state.get::<ToplevelIconCachedState>();
            cached
                .current()
                .buffers()
                .iter()
                .min_by_key(|(b, _)| (width_of(b) - PREFERRED_SOURCE).abs())
                .and_then(|(b, _)| from_shm(b))
        });
        if win.set_icon(source) && win.mapped {
            self.retitled(w, crate::retitle::Changes::ICON);
        }
    }
}

/// xdg-toplevel-icon is double-buffered on the surface, so the commit applies
/// it (`apply_icon`).
impl XdgToplevelIconHandler for State {
    fn set_icon(&mut self, _toplevel: XdgToplevel, wl_surface: WlSurface) {
        if let Some(w) = self.window_for_surface(&wl_surface)
            && let Some(win) = self.win_mut(w)
        {
            win.icon_pending = true;
        }
    }
}
