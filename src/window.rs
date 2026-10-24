//! A managed toplevel window: scene nodes, frame, geometry, state.

mod draw;
mod geometry;
mod lifecycle;
mod owners;
mod place;
mod popups;
mod states;
#[cfg(test)]
mod tests;

use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use reprisekit::anim::HoverFade;
use reprisekit::classic::frame::{CaptionButton, Chrome, FrameGeometry, FrameHit};
use reprisekit::icon::IconSource;
use reprisescene::{BufferId, SurfaceId, TreeId};
use smithay::{
    desktop::utils::bbox_from_surface_tree,
    reexports::wayland_server::{Client, Resource, protocol::wl_surface::WlSurface},
    utils::{Logical, Rectangle, Serial},
    wayland::{
        compositor::with_states,
        shell::xdg::{SurfaceCachedState, ToplevelSurface, XdgToplevelSurfaceData},
    },
    xwayland::X11Surface,
};

use crate::compositor::State;
use crate::glass::Backdrop;
use crate::raster::{Canvas, Rect};
use crate::theme::Metrics;
use crate::windowrules::Want;

use draw::ShadowKey;
pub use draw::{FrameBatch, ShadowCache};
pub use geometry::SizeLimits;
use geometry::client_extent;
pub(crate) use geometry::{client_limits, x11_geometry};
pub use place::{
    ResizeAnchor, anchor_done, anchored_position, area_client, below_top_bar, cascade_from,
    cascade_position, centre_on, first_configure_size, fit_size, keep_inside, rect_below_top_bar,
};

pub const PROMPT_TAG: &str = "prompt";
pub const HELP_DIALOG_TAG: &str = "help-dialog";
pub const FIXED_DIALOG_TAG: &str = "fixed-dialog";
pub const FILE_DIALOG_TAG: &str = "file-dialog";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DialogTag {
    #[default]
    None,
    Prompt,
    Help,
    Fixed,
}

impl DialogTag {
    pub fn from_tag(tag: &str) -> DialogTag {
        match tag {
            PROMPT_TAG => DialogTag::Prompt,
            HELP_DIALOG_TAG => DialogTag::Help,
            FIXED_DIALOG_TAG => DialogTag::Fixed,
            _ => DialogTag::None,
        }
    }
}

/// Owned or fixed-dialog windows get the close box beside the icon; prompt and help dialogs get a
/// dialog caption without icon (help adds the ? box). The border resizes only if size hints allow.
pub fn chrome_for(owned: bool, resizable: bool, decorated: bool, tag: DialogTag) -> Chrome {
    let base = match tag {
        DialogTag::Help => Chrome::HELP_DIALOG,
        DialogTag::Prompt => Chrome::PROMPT,
        DialogTag::Fixed => Chrome::DIALOG,
        DialogTag::None if owned => Chrome::DIALOG,
        DialogTag::None => Chrome::WINDOW,
    };
    Chrome {
        resizable,
        decorated,
        ..base
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WindowId(u64);

impl WindowId {
    pub(crate) fn next() -> WindowId {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        WindowId(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

pub enum Kind {
    Xdg(ToplevelSurface),
    /// An X11 window shown through Xwayland; made once Xwayland ties a `wl_surface` to it.
    X11(Box<X11Surface>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Handle {
    Wl(WlSurface),
    X11(u32),
}

pub struct Window {
    pub id: WindowId,
    /// Order among task buttons with the same anchor: creation order until a drag renumbers it.
    pub seq: u64,
    /// Pinned slots before its task button; `usize::MAX` until a drag moves it.
    pub anchor: usize,
    pub kind: Kind,
    wl: WlSurface,
    /// X11 menu or tooltip (override-redirect): frameless, off the taskbar.
    pub override_redirect: bool,
    /// The client's own icon at its native size; `None` draws the theme's.
    pub icon: Option<IconSource>,
    /// `icon` resampled per size; emptied when the icon changes.
    icon_sizes: RefCell<Vec<(i32, Arc<Canvas>)>>,
    pub tree: TreeId,
    /// Shadow, backdrop and frame, below `content`; left out of what glass blurs
    /// (`glass::leave_out`).
    pub chrome: TreeId,
    /// The shadow's eight pieces; shown only under Glass around a restored frame.
    pub shadow: [BufferId; 8],
    shadow_key: Option<ShadowKey>,
    pub backdrop: TreeId,
    glass: Backdrop,
    /// The frame as four strips, so a huge client surface never costs a matching raster.
    pub frame: [BufferId; 4],
    /// A sheet's plain glass interior, one pixel stretched.
    sheet: BufferId,
    /// The client's blur region (`clientglass::region`), client coordinates.
    pub blur: Vec<Rect>,
    /// The frame's glass under each `blur` rect while Glass draws it.
    client_glass: Vec<BufferId>,
    /// `draw_sheet` drew the window as a sheet and `unsheet` has not put
    /// its frame's nodes back yet.
    pub sheeted: bool,
    /// Client surface tree at the client area's top-left, offset by its window geometry; popups
    /// above.
    pub content: TreeId,
    pub surface: SurfaceId,
    popups: Vec<(WlSurface, SurfaceId)>,
    pub x: i32,
    pub y: i32,
    /// `drag_to` moved the tree and `settle_move` has not caught up yet.
    pub move_pending: bool,
    pub client_w: i32,
    pub client_h: i32,
    pub geom: FrameGeometry,
    pub mapped: bool,
    pub maximized: bool,
    pub minimized: bool,
    /// The client asked for fullscreen: no chrome; its tree sits in `layers.fullscreen` while
    /// focused. `maximized` keeps its value underneath.
    pub fullscreen: bool,
    pub saved: Option<Rect>,
    pub saved_fullscreen: Option<Rect>,
    pub snapped: Option<Rect>,
    pub resize_anchor: Option<ResizeAnchor>,
    pub size_serial: Option<Serial>,
    pub pressed: Option<CaptionButton>,
    pub hot: Option<CaptionButton>,
    pub glow: [HoverFade; 3],
    /// The client draws its own decorations (CSD, or Motif hints drop the frame); no frame is
    /// drawn.
    pub csd: bool,
    /// Frame hidden by the `toggleframe` bind; the client fills the old outer rect.
    pub frameless: bool,
    pub dialog_tag: DialogTag,
    /// Tagged `file-dialog`: opens at and keeps the shared file dialog geometry (`filedlggeom`).
    pub file_dialog: bool,
    pub modal: bool,
    pub on_top: bool,
    pub frame_scale: i32,
    /// Capabilities last sent; `None` before the first send.
    pub wm_caps: Option<(bool, bool)>,
    pub placement: Option<Want>,
    pub icon_pending: bool,
    pub app_seen: String,
    pub titles: crate::retitle::WindowTitles,
    pub attention: bool,
    pub activate_on_map: bool,
    /// The launch that let this window take focus at map, returned if it withdraws still focused.
    pub launch: Option<crate::mapfocus::Launch>,
    /// Set to the current desktop at each map; a dialog goes with its owner.
    pub desktop: usize,
    pub sticky: bool,
    /// In a tab group showing another window: hidden like a minimized window.
    pub tab_hidden: bool,
    /// The `tab_group` its window rules gave it at map.
    pub tab_group: Option<String>,
    /// Pulled out of its tab group by hand: new windows of its `tab_group`
    /// pass it over until it is in a group again.
    pub tab_pulled_out: bool,
    /// The window of the same client whose tab group it joins at map (reprise-tabs-v1).
    pub tab_leader: Option<WindowId>,
}

impl Window {
    pub fn is_active(&self, focused: Option<WindowId>) -> bool {
        focused == Some(self.id)
    }

    pub fn hidden(&self) -> bool {
        self.minimized || self.tab_hidden
    }

    pub fn frame_rect(&self) -> Rect {
        Rect::new(self.x, self.y, self.geom.total.0, self.geom.total.1)
    }

    pub fn hit_at(&self, lx: f64, ly: f64, m: &Metrics) -> FrameHit {
        self.geom.hit(lx as i32 - self.x, ly as i32 - self.y, m)
    }

    pub fn wl_surface(&self) -> &WlSurface {
        &self.wl
    }

    pub fn xdg(&self) -> Option<&ToplevelSurface> {
        match &self.kind {
            Kind::Xdg(t) => Some(t),
            Kind::X11(_) => None,
        }
    }

    pub fn x11(&self) -> Option<&X11Surface> {
        match &self.kind {
            Kind::X11(x) => Some(x.as_ref()),
            Kind::Xdg(_) => None,
        }
    }

    /// None for an X11 window; its surfaces belong to Xwayland.
    pub fn wl_client(&self) -> Option<Client> {
        self.xdg().and_then(|t| t.wl_surface().client())
    }

    pub fn title(&self) -> String {
        match &self.kind {
            Kind::Xdg(_) => with_states(self.wl_surface(), |s| {
                s.data_map
                    .get::<XdgToplevelSurfaceData>()
                    .and_then(|d| d.lock().ok()?.title.clone())
            })
            .unwrap_or_default(),
            Kind::X11(x) => x.title(),
        }
    }

    pub fn app_id(&self) -> String {
        match &self.kind {
            Kind::Xdg(_) => with_states(self.wl_surface(), |s| {
                s.data_map
                    .get::<XdgToplevelSurfaceData>()
                    .and_then(|d| d.lock().ok()?.app_id.clone())
            })
            .unwrap_or_default(),
            Kind::X11(x) => {
                let class = x.class();
                if class.is_empty() {
                    x.instance()
                } else {
                    class
                }
            }
        }
    }

    pub fn own_icon(&self, px: i32) -> Option<Arc<Canvas>> {
        let src = self.icon.as_ref()?;
        let mut sizes = self.icon_sizes.borrow_mut();
        if let Some((_, c)) = sizes.iter().find(|(p, _)| *p == px) {
            return Some(c.clone());
        }
        let c = Arc::new(src.to_canvas(px));
        sizes.push((px, c.clone()));
        Some(c)
    }

    pub fn set_icon(&mut self, icon: Option<IconSource>) -> bool {
        if self.icon == icon {
            return false;
        }
        self.icon = icon;
        self.icon_sizes.borrow_mut().clear();
        true
    }

    /// Client size from the xdg geometry (surface size before one is set), clamped by
    /// `client_extent`.
    pub fn client_size(&self) -> (i32, i32) {
        let g = self.geometry();
        client_extent(g.size.w, g.size.h)
    }

    fn geometry(&self) -> Rectangle<i32, Logical> {
        let set = with_states(self.wl_surface(), |s| {
            s.cached_state
                .get::<SurfaceCachedState>()
                .current()
                .geometry
        });
        set.filter(|g| g.size.w > 0 && g.size.h > 0)
            .unwrap_or_else(|| bbox_from_surface_tree(self.wl_surface(), (0, 0)))
    }

    fn geometry_origin(&self) -> (i32, i32) {
        let g = self.geometry();
        (g.loc.x, g.loc.y)
    }

    pub fn client_origin(&self) -> (i32, i32) {
        (self.x + self.geom.client.x, self.y + self.geom.client.y)
    }

    /// Layout coordinates of the client surface's (0, 0); client surface-local points are relative
    /// to it.
    pub fn surface_origin(&self) -> (i32, i32) {
        let (cx, cy) = self.client_origin();
        let (gx, gy) = self.geometry_origin();
        (cx - gx, cy - gy)
    }

    pub fn size_limits(&self) -> SizeLimits {
        if let Kind::X11(x) = &self.kind {
            // Smithay keeps only the flagged WM_NORMAL_HINTS fields; an unset max is unbounded.
            let min = x.min_size().unwrap_or_default();
            let max = x.max_size().unwrap_or_default();
            return client_limits((min.w, min.h, max.w, max.h));
        }
        let (min, max) = with_states(self.wl_surface(), |s| {
            let mut c = s.cached_state.get::<SurfaceCachedState>();
            let c = c.current();
            (c.min_size, c.max_size)
        });
        client_limits((min.w, min.h, max.w, max.h))
    }

    pub fn handle(&self) -> Handle {
        match &self.kind {
            Kind::Xdg(t) => Handle::Wl(t.wl_surface().clone()),
            Kind::X11(x) => Handle::X11(x.window_id()),
        }
    }

    pub fn parent_handle(&self) -> Option<Handle> {
        match &self.kind {
            Kind::Xdg(t) => t.parent().map(Handle::Wl),
            Kind::X11(x) => x
                .is_transient_for()
                .filter(|&p| p != x11rb::NONE && p != x.window_id())
                .map(Handle::X11),
        }
    }

    pub fn is_owned(&self) -> bool {
        self.parent_handle().is_some()
    }

    pub fn is_modal(&self) -> bool {
        match &self.kind {
            Kind::Xdg(_) => self.modal,
            Kind::X11(x) => x.is_modal(),
        }
    }

    pub fn skips_taskbar(&self) -> bool {
        match &self.kind {
            Kind::Xdg(_) => matches!(self.dialog_tag, DialogTag::Prompt | DialogTag::Help),
            Kind::X11(x) => self.override_redirect || x.is_skip_taskbar(),
        }
    }

    pub fn snappable(&self) -> bool {
        crate::snap::snappable(crate::snap::SnapTraits {
            mapped: self.mapped,
            fullscreen: self.fullscreen,
            resizable: self.is_resizable(),
            owned: self.is_owned(),
            tab_hidden: self.tab_hidden,
        })
    }

    pub fn is_resizable(&self) -> bool {
        self.size_limits().resizable()
    }

    pub fn has_frame(&self) -> bool {
        !self.csd && !self.fullscreen && !self.override_redirect && !self.frameless
    }

    pub fn wants_focus_on_click(&self) -> bool {
        match &self.kind {
            Kind::X11(x) if self.override_redirect => {
                crate::xwayland::override_redirect_wants_focus(x.window_type())
            }
            _ => true,
        }
    }

    pub fn chrome(&self) -> Chrome {
        chrome_for(
            self.is_owned(),
            self.is_resizable(),
            self.has_frame(),
            self.dialog_tag,
        )
    }
}

impl State {
    pub fn win(&self, id: WindowId) -> Option<&Window> {
        self.windows.iter().find(|w| w.id == id)
    }

    pub fn win_mut(&mut self, id: WindowId) -> Option<&mut Window> {
        self.windows.iter_mut().find(|w| w.id == id)
    }

    pub fn client_pid(&self, w: WindowId) -> Option<i32> {
        let pid = match &self.win(w)?.kind {
            Kind::Xdg(t) => return self.surface_pid(t.wl_surface()),
            Kind::X11(x) => i32::try_from(x.pid()?).ok()?,
        };
        (pid > 0).then_some(pid)
    }

    /// The process of the Wayland client owning `surface`.
    pub fn surface_pid(&self, surface: &WlSurface) -> Option<i32> {
        let client = surface.client()?;
        #[cfg(test)]
        if let Some(pid) = client
            .get_data::<crate::compositor::ClientState>()
            .and_then(|d| d.pid)
        {
            return Some(pid);
        }
        let pid = client.get_credentials(&self.dh).ok()?.pid;
        (pid > 0).then_some(pid)
    }

    pub fn window_for_surface(&self, root: &WlSurface) -> Option<WindowId> {
        self.windows
            .iter()
            .find(|w| w.wl_surface() == root)
            .map(|w| w.id)
    }

    pub fn set_tag(&mut self, w: WindowId, tag: &str) {
        let file_dialog = tag == FILE_DIALOG_TAG;
        let tag = DialogTag::from_tag(tag);
        let Some(win) = self.win_mut(w) else {
            return;
        };
        if std::mem::replace(&mut win.file_dialog, file_dialog) != file_dialog {
            self.replan_unmapped(w);
        }
        let Some(win) = self.win_mut(w) else {
            return;
        };
        if std::mem::replace(&mut win.dialog_tag, tag) != tag {
            self.on_parent_changed(w);
        }
    }
}
