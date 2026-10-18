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

/// What an icon whose file is not decoded yet shows meanwhile.
pub const STAND_IN: &str = "application-x-executable";

/// Icon files a job is decoding, each with the px asked for since its job started.
pub type Decoding = std::collections::HashMap<String, Vec<i32>>;

/// The jobs to start for `wanted` (path, px): one per file not decoding yet, with all its sizes.
/// A file already decoding only notes the sizes, for a job of its own once the first lands: a
/// file on a dead mount holds one job whatever surfaces and sizes ask for it.
fn plan_decodes(decoding: &mut Decoding, wanted: Vec<(String, i32)>) -> Vec<(String, Vec<i32>)> {
    let mut start: Vec<(String, Vec<i32>)> = Vec::new();
    for (file, px) in wanted {
        if let Some(asked) = decoding.get_mut(&file) {
            if !asked.contains(&px) {
                asked.push(px);
            }
            continue;
        }
        match start.iter_mut().find(|(f, _)| *f == file) {
            Some((_, sizes)) if !sizes.contains(&px) => sizes.push(px),
            Some(_) => {}
            None => start.push((file, vec![px])),
        }
    }
    for (file, sizes) in &start {
        decoding.insert(file.clone(), sizes.clone());
    }
    start
}

impl State {
    /// Decodes the icon files the loop asked [`IconCache::get_unread`] for off the loop (`plan_decodes`). Runs
    /// after each dispatch of the loop.
    pub fn decode_wanted_icons(&mut self) {
        let wanted = self.icons.take_wanted();
        if wanted.is_empty() {
            return;
        }
        for (file, sizes) in plan_decodes(&mut self.icon_decodes, wanted) {
            self.spawn_icon_decode(file, sizes);
        }
    }

    fn spawn_icon_decode(&mut self, file: String, sizes: Vec<i32>) {
        let scale = self.icons.scale();
        self.spawn_fs_job(move || {
            let path = std::path::Path::new(&file);
            let art: Vec<(i32, Option<Canvas>)> = (sizes.into_iter())
                .map(|px| (px, reprisekit::icontheme::decode_file(path, px, scale)))
                .collect();
            Box::new(move |state: &mut State| state.icon_decoded(file, art, scale))
        });
    }

    fn icon_decoded(&mut self, file: String, art: Vec<(i32, Option<Canvas>)>, scale: i32) {
        let asked = self.icon_decodes.remove(&file).unwrap_or_default();
        let late: Vec<i32> = (asked.into_iter())
            .filter(|px| !art.iter().any(|(p, _)| p == px))
            .collect();
        let puts = art.iter().map(|(px, c)| (file.clone(), *px, c.clone()));
        self.icons.put_files_at(puts.collect(), scale);
        self.icon_landed(&file, &art);
        if !late.is_empty() {
            self.icon_decodes.insert(file.clone(), late.clone());
            self.spawn_icon_decode(file, late);
        }
    }

    /// Icon file `file` decoded at the sizes of `art`: every surface showing it draws again.
    fn icon_landed(&mut self, file: &str, art: &[(i32, Option<Canvas>)]) {
        for (px, c) in art {
            self.menu_icon_landed(file, *px, c);
        }
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_decoding_holds_one_job_whatever_sizes_ask_for_it() {
        let mut decoding = Decoding::new();
        let w = |f: &str, px| (f.to_string(), px);
        let start = plan_decodes(
            &mut decoding,
            vec![
                w("/srv/a.png", 16),
                w("/srv/a.png", 32),
                w("/srv/b.png", 16),
                w("/srv/a.png", 16),
            ],
        );
        assert_eq!(
            start,
            [
                ("/srv/a.png".to_string(), vec![16, 32]),
                ("/srv/b.png".to_string(), vec![16])
            ]
        );
        let again = plan_decodes(
            &mut decoding,
            vec![w("/srv/a.png", 48), w("/srv/a.png", 16)],
        );
        assert!(again.is_empty(), "a decoding file starts no second job");
        assert_eq!(
            decoding["/srv/a.png"],
            [16, 32, 48],
            "the late size waits for the first job"
        );
    }
}
