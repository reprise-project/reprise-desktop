//! A managed toplevel window: scene nodes, frame, geometry, state.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use reprisekit::anim::HoverFade;
use reprisekit::classic::frame::{
    CaptionButton, Chrome, Edges, FrameGeometry, FrameHit, FrameState, frame_canvas_size,
};
use reprisekit::controls::captiontabs::{self, CaptionTabs, TabEntry};
use reprisekit::glass::{Material, shadow};
use reprisekit::icon::IconSource;
use reprisescene::{BufferId, Input, NodeId, SurfaceId, TreeId};
use smithay::{
    backend::renderer::utils::with_renderer_surface_state,
    desktop::{
        PopupKind, PopupManager, find_popup_root_surface, get_popup_toplevel_coords,
        utils::bbox_from_surface_tree,
    },
    reexports::{
        wayland_protocols::xdg::shell::server::xdg_toplevel,
        wayland_server::{Client, Resource, protocol::wl_surface::WlSurface},
    },
    utils::{Logical, Point, Rectangle, Serial},
    wayland::{
        compositor::with_states,
        shell::xdg::{PopupSurface, SurfaceCachedState, ToplevelSurface, XdgToplevelSurfaceData},
    },
    xwayland::{X11Surface, xwm::WmWindowType},
};

use crate::compositor::{DeskScene, SceneOwner, State};
use crate::glass::{Backdrop, frame_pane, sheet_pane};
use crate::input::Interaction;
use crate::mapfocus::OnMap;
use crate::output::upload_scaled;
use crate::raster::{Canvas, Rect};
use crate::theme::Metrics;
use crate::windowrules::{Position, Want, WindowState};

/// Glass shadow pieces per (scale, active, square corners), drawn once and shared; each window's
/// nodes take their own pixel copy.
#[derive(Default)]
pub struct ShadowCache(Vec<(ShadowKey, Rc<[Canvas; 8]>)>);

type ShadowKey = (i32, bool, bool);

impl ShadowCache {
    pub fn pieces(&mut self, scale: i32, active: bool, square: bool) -> Rc<[Canvas; 8]> {
        let key = (scale, active, square);
        if let Some((_, p)) = self.0.iter().find(|(k, _)| *k == key) {
            return p.clone();
        }
        let p: Rc<[Canvas; 8]> = Rc::new(std::array::from_fn(|i| {
            if square {
                reprisekit::glass::square::shadow_piece(active, i, scale)
            } else {
                shadow::piece(active, i, scale)
            }
        }));
        self.0.push((key, p.clone()));
        p
    }
}

/// Edges a drag-resize pins; an xdg client picks its own size, so a LEFT/TOP drag moves the frame
/// once the new size arrives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResizeAnchor {
    pub edges: Edges,
    pub right: i32,
    pub bottom: i32,
}

pub fn anchored_position(a: &ResizeAnchor, total: (i32, i32), current: (i32, i32)) -> (i32, i32) {
    let (mut x, mut y) = current;
    if a.edges.contains(Edges::LEFT) {
        x = a.right - total.0;
    }
    if a.edges.contains(Edges::TOP) {
        y = a.bottom - total.1;
    }
    (x, y)
}

/// Whether the anchor is done after the drag: the client committed the last configure sent (`acked`
/// vs `sent`); with none sent (X11), any resize commit is the last.
pub fn anchor_done(sent: Option<Serial>, acked: Option<Serial>, resized: bool) -> bool {
    match (sent, acked) {
        (Some(s), Some(a)) => a >= s,
        (Some(_), None) => false,
        (None, _) => resized,
    }
}

pub fn cascade_position(index: usize, area: Rect, m: &Metrics) -> (i32, i32) {
    let step = m.cascade_step * (index % 10) as i32;
    let x = area.x + m.cascade_origin + step;
    let y = area.y + m.cascade_origin + step;
    let max_y = area.bottom() - m.caption_height - 2 * m.frame_width;
    (x.min(area.right() - 64), y.min(max_y.max(area.y)))
}

pub fn cascade_from(prev: (i32, i32), size: (i32, i32), area: Rect, m: &Metrics) -> (i32, i32) {
    let (x, y) = (prev.0 + m.cascade_step, prev.1 + m.cascade_step);
    if x + size.0 > area.right() || y + size.1 > area.bottom() {
        (area.x + m.cascade_origin, area.y + m.cascade_origin)
    } else {
        (x, y)
    }
}

pub fn keep_inside(origin: (i32, i32), size: (i32, i32), area: Rect) -> (i32, i32) {
    let max_x = (area.right() - size.0).max(area.x);
    let max_y = (area.bottom() - size.1).max(area.y);
    (origin.0.clamp(area.x, max_x), origin.1.clamp(area.y, max_y))
}

pub fn below_top_bar(y: i32, p: &crate::barplace::BarPlacement) -> i32 {
    match p.edge {
        reprisekit::taskbar::Edge::Top => y.max(p.reserved.bottom()),
        _ => y,
    }
}

pub fn rect_below_top_bar(r: Rect, p: &crate::barplace::BarPlacement) -> Rect {
    Rect::new(r.x, below_top_bar(r.y, p), r.width, r.height)
}

pub fn fit_size(size: (i32, i32), area: Rect) -> (i32, i32) {
    (
        size.0.min(area.width).max(1),
        size.1.min(area.height).max(1),
    )
}

/// The largest client a normal window of `chrome` has in `area`: the xdg-toplevel bounds.
pub fn area_client(m: &Metrics, area: Rect, chrome: Chrome) -> (i32, i32) {
    FrameGeometry::client_size_for_total(m, area.width, area.height, false, chrome)
}

/// Client size for a first configure; a CSD client is asked for the whole frame. A normal size
/// fits `area` and goes under the client's `limits` only where they would not fit it.
pub fn first_configure_size(
    m: &Metrics,
    want: &Want,
    limits: SizeLimits,
    screen: Rect,
    area: Rect,
    chrome: Chrome,
) -> Option<(i32, i32)> {
    let resizable = limits.resizable();
    match want.state {
        Some(WindowState::Fullscreen) => Some((screen.width, screen.height)),
        Some(WindowState::Maximized) if resizable => Some(FrameGeometry::client_size_for_total(
            m,
            area.width,
            area.height,
            true,
            chrome,
        )),
        _ => want.size.map(|s| {
            let (w, h) = fit_size(s, area);
            let client = FrameGeometry::client_size_for_total(m, w, h, false, chrome);
            let (cw, ch) = crate::snap::commit_size(client, limits);
            let most = area_client(m, area, chrome);
            (cw.min(most.0), ch.min(most.1))
        }),
    }
}

/// Where a frame that was `was` and is now `now` goes to stay inside `area`: moved as little as
/// keeps it there, when it was inside and grew past an edge. `None` leaves it where it is.
pub fn grown_back_inside(was: Rect, now: Rect, area: Rect) -> Option<(i32, i32)> {
    let inside = |r: Rect| {
        r.x >= area.x && r.y >= area.y && r.right() <= area.right() && r.bottom() <= area.bottom()
    };
    let grew = now.width > was.width || now.height > was.height;
    if area.width <= 0 || !grew || !inside(was) || inside(now) {
        return None;
    }
    let to = keep_inside((now.x, now.y), (now.width, now.height), area);
    (to != (now.x, now.y)).then_some(to)
}

/// Where a frame that was `was` in work area `was_area` and is `now` in `area` after a look or
/// text size change goes: inside `area` as `keep_inside` puts it, when it was wholly on `screen`,
/// its frame or the taskbar grew, and it now leaves the work area. `None` leaves it where it is.
pub fn retheme_back_inside(
    was: Rect,
    now: Rect,
    screen: Rect,
    was_area: Rect,
    area: Rect,
) -> Option<(i32, i32)> {
    let within = |r: Rect, b: Rect| {
        r.x >= b.x && r.y >= b.y && r.right() <= b.right() && r.bottom() <= b.bottom()
    };
    let grew = now.width > was.width || now.height > was.height;
    let shrank = !within(was_area, area);
    if area.width <= 0 || !(grew || shrank) || !within(was, screen) || within(now, area) {
        return None;
    }
    let to = keep_inside((now.x, now.y), (now.width, now.height), area);
    (to != (now.x, now.y)).then_some(to)
}

/// A dialog's frame origin: centred on its owner (or on `area` if ownerless), pushed inside `area`.
pub fn centre_on(parent: Rect, size: (i32, i32), area: Rect) -> (i32, i32) {
    let x = parent.x + (parent.width - size.0) / 2;
    let y = parent.y + (parent.height - size.1) / 2;
    keep_inside((x, y), size, area)
}

/// Visit windows below `root` owner-first, siblings in `children`'s order; iterative with a visited
/// set so parent cycles (X11 transients) are safe.
fn walk_owned<T: Copy + Eq + std::hash::Hash, S: Copy>(
    root: T,
    state: S,
    mut children: impl FnMut(T) -> Vec<T>,
    mut visit: impl FnMut(T, S) -> S,
) {
    let mut seen = std::collections::HashSet::from([root]);
    let mut stack: Vec<(T, S)> = children(root)
        .into_iter()
        .rev()
        .map(|k| (k, state))
        .collect();
    while let Some((w, owner_state)) = stack.pop() {
        if !seen.insert(w) {
            continue;
        }
        let s = visit(w, owner_state);
        stack.extend(children(w).into_iter().rev().map(|k| (k, s)));
    }
}

/// Follow `modal` from `root` to the end of the chain: the window that takes focus in `root`'s
/// place. Stops at a revisited window.
fn modal_end<T: Copy + Eq + std::hash::Hash>(root: T, mut modal: impl FnMut(T) -> Option<T>) -> T {
    let mut seen = std::collections::HashSet::from([root]);
    let mut w = root;
    while let Some(k) = modal(w) {
        if !seen.insert(k) {
            break;
        }
        w = k;
    }
    w
}

fn owner_chain_has<T: Copy + Eq + std::hash::Hash>(
    w: T,
    owner: T,
    mut parent: impl FnMut(T) -> Option<T>,
) -> bool {
    let mut seen = std::collections::HashSet::from([w]);
    let mut at = w;
    while let Some(p) = parent(at) {
        if p == owner {
            return true;
        }
        if !seen.insert(p) {
            return false;
        }
        at = p;
    }
    false
}

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

/// The frames `redraw_frame` put off inside `State::batch_frames`, each once, in the order first
/// asked for.
#[derive(Default)]
pub struct FrameBatch(Option<Vec<WindowId>>);

impl FrameBatch {
    /// Start a batch; false if one is open already (the outer one draws).
    fn begin(&mut self) -> bool {
        if self.0.is_some() {
            return false;
        }
        self.0 = Some(Vec::new());
        true
    }

    /// Whether `w`'s redraw waits for the batch's end.
    fn put_off(&mut self, w: WindowId) -> bool {
        let Some(batch) = self.0.as_mut() else {
            return false;
        };
        if !batch.contains(&w) {
            batch.push(w);
        }
        true
    }

    fn end(&mut self) -> Vec<WindowId> {
        self.0.take().unwrap_or_default()
    }
}

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

/// Whether (`x`, `y`) takes the pointer: clear pixels of a styled caption's cut corner pass
/// through.
fn frame_takes(
    look: &reprisekit::paint::Look,
    geom: &FrameGeometry,
    active: bool,
    maximized: bool,
    x: i32,
    y: i32,
) -> bool {
    match look.style() {
        Some(style) => {
            reprisekit::style::frame::frame_opaque_at(style, geom, active, maximized, x, y)
        }
        None => true,
    }
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

    pub fn new_window(&mut self, toplevel: ToplevelSurface) -> WindowId {
        let wl = toplevel.wl_surface().clone();
        self.add_window(Kind::Xdg(toplevel), wl)
    }

    pub fn add_window(&mut self, kind: Kind, wl: WlSurface) -> WindowId {
        let id = WindowId::next();
        let scene = &mut self.scene;
        let tree = scene.tree_create(self.layers.windows);
        scene.set_enabled(tree, false);
        scene.set_data(tree, Some(SceneOwner::Window(id)));
        // Shadow pieces first so they are the lowest children (pointer-transparent), then the
        // backdrop, then the frame strips; all before `content`.
        let chrome = scene.tree_create(tree);
        crate::glass::leave_out(scene, chrome);
        let shadow = std::array::from_fn(|_| {
            let sb = scene.buffer_create(chrome);
            scene.set_input(sb, Input::Passthrough);
            scene.set_enabled(sb, false);
            sb
        });
        let backdrop = scene.tree_create(chrome);
        let frame = std::array::from_fn(|_| scene.buffer_create(chrome));
        let sheet = scene.buffer_create(chrome);
        scene.set_input(sheet, Input::Passthrough);
        scene.set_enabled(sheet, false);
        let content = scene.tree_create(tree);
        let surface = scene.surface_create(content, wl.clone());
        self.windows.push(Window {
            id,
            seq: crate::taskbar::next_seq(),
            anchor: usize::MAX,
            kind,
            wl,
            override_redirect: false,
            frameless: false,
            icon: None,
            icon_sizes: RefCell::default(),
            tree,
            chrome,
            shadow,
            shadow_key: None,
            backdrop,
            glass: Backdrop::new(&self.glass),
            frame,
            sheet,
            blur: Vec::new(),
            client_glass: Vec::new(),
            sheeted: false,
            content,
            surface,
            popups: Vec::new(),
            x: 0,
            y: 0,
            move_pending: false,
            client_w: 1,
            client_h: 1,
            geom: FrameGeometry::layout(&self.config.theme.metrics, 1, 1, false, Chrome::WINDOW),
            mapped: false,
            maximized: false,
            minimized: false,
            fullscreen: false,
            saved: None,
            saved_fullscreen: None,
            snapped: None,
            resize_anchor: None,
            size_serial: None,
            pressed: None,
            hot: None,
            glow: Default::default(),
            csd: false,
            dialog_tag: DialogTag::None,
            file_dialog: false,
            modal: false,
            on_top: false,
            frame_scale: 1,
            wm_caps: None,
            placement: None,
            icon_pending: false,
            app_seen: String::new(),
            titles: Default::default(),
            attention: false,
            activate_on_map: false,
            launch: None,
            desktop: 0,
            sticky: false,
            tab_hidden: false,
            tab_group: None,
            tab_pulled_out: false,
            tab_leader: None,
        });
        id
    }

    pub fn caption_clear(&self, r: Rect) -> Rect {
        self.output_at(f64::from(r.x + r.width / 2), f64::from(r.y))
            .map_or(r, |o| rect_below_top_bar(r, &self.bar_placement(&o)))
    }

    /// Put the frame at a saved rect's origin and return the client size that rect holds under the
    /// current chrome.
    fn restore_to(&mut self, w: WindowId, r: Rect) -> (i32, i32) {
        let m = self.config.theme.metrics.clone();
        let r = self.caption_clear(r);
        let Some(win) = self.win_mut(w) else {
            return (r.width, r.height);
        };
        win.x = r.x;
        win.y = r.y;
        FrameGeometry::client_size_for_total(&m, r.width, r.height, false, win.chrome())
    }

    pub fn relayout(&mut self, w: WindowId) {
        let m = self.config.theme.metrics.clone();
        let opens = reprisekit::look::client_glass(&self.config.theme, true).is_some();
        let Some(win) = self.win_mut(w) else {
            return;
        };
        let (cw, ch) = win.client_size();
        win.client_w = cw;
        win.client_h = ch;
        let glass_top = match opens {
            true => crate::clientglass::glass_top(&win.blur, cw),
            false => 0,
        };
        win.geom = FrameGeometry::layout(&m, cw, ch, win.maximized, win.chrome())
            .with_glass_top(glass_top);
        let (tree, content, surface, frame, on) = (
            win.tree,
            win.content,
            win.surface,
            win.frame,
            win.has_frame(),
        );
        let (x, y, client) = (win.x, win.y, win.geom.client);
        let (gx, gy) = win.geometry_origin();
        self.scene.set_position(tree, x, y);
        for sb in frame {
            self.scene.set_enabled(sb, on);
        }
        self.scene.set_position(content, client.x, client.y);
        self.scene.set_position(surface, -gx, -gy);
        self.sync_popups(w);
        self.redraw_frame(w);
        // X11 clients track their own absolute position, so every placement
        // change has to be sent back to them.
        self.place_x11(w);
        self.flip_window_changed(w);
    }

    /// Tell an X11 client where its client rect starts, keeping the size its X window was last
    /// given (a pending `request_size` must not be undone).
    pub(crate) fn place_x11(&mut self, w: WindowId) {
        let Some(win) = self.win(w) else {
            return;
        };
        let Some(x) = win.x11().filter(|_| !win.override_redirect) else {
            return;
        };
        let size = x.last_configure().size;
        self.configure_x11(w, size.w, size.h);
    }

    fn configure_x11(&self, w: WindowId, cw: i32, ch: i32) {
        let Some(win) = self.win(w) else {
            return;
        };
        let Some(x) = win.x11().filter(|_| !win.override_redirect) else {
            return;
        };
        let (cx, cy) = win.client_origin();
        let (gx, gy, gw, gh) = x11_geometry(cx, cy, cw, ch);
        let rect = Rectangle::new(
            (i32::from(gx), i32::from(gy)).into(),
            (i32::from(gw), i32::from(gh)).into(),
        );
        if rect != x.last_configure()
            && let Err(e) = x.configure(rect)
        {
            log::debug!("x11 configure: {e}");
        }
    }

    pub fn redraw_frame(&mut self, w: WindowId) {
        if self.frame_batch.put_off(w) {
            return;
        }
        self.place_backdrop(w);
        crate::access::frame_changed(self);
        if self.is_sheet(w) {
            let scale = self.draw_scale_for_window(w);
            if let Some(win) = self.win_mut(w) {
                win.frame_scale = scale;
            }
            self.place_shadow(w);
            self.draw_sheet(w);
            return;
        }
        self.unsheet(w);
        let Some(win) = self.win(w) else {
            return;
        };
        if !win.has_frame() {
            self.place_shadow(w);
            return;
        }
        let scale = self.draw_scale_for_window(w);
        if let Some(win) = self.win_mut(w) {
            win.frame_scale = scale;
        }
        self.place_shadow(w);
        self.draw_strips(w, false);
    }

    /// Run `f` with `redraw_frame` put off: each frame it asked for draws once afterwards, in its
    /// final state.
    pub fn batch_frames(&mut self, f: impl FnOnce(&mut Self)) {
        let outer = self.frame_batch.begin();
        f(self);
        if outer {
            for w in self.frame_batch.end() {
                self.redraw_frame(w);
            }
        }
    }

    /// Whether `w` shows as a sheet: peek hides it, it is not fullscreen, and the look draws
    /// translucent glass with a sheet.
    pub(crate) fn is_sheet(&self, w: WindowId) -> bool {
        self.peek.hides(w)
            && self.win(w).is_some_and(|x| !x.fullscreen)
            && crate::glass::wanted(&self.config)
            && reprisekit::look::sheet_edge(&self.config.theme).is_some()
    }

    /// Draw `w` as a sheet over its whole rect: edges in the four strips, plain glass stretched in
    /// between; the pointer passes through.
    fn draw_sheet(&mut self, w: WindowId) {
        self.thumbs_frame(w);
        let Some(scale) = self.win(w).map(|x| x.frame_scale.max(1)) else {
            return;
        };
        let theme = self.config.theme.scaled(scale);
        let Some(edge) = reprisekit::look::sheet_edge(&theme) else {
            return;
        };
        let Some(win) = self.win_mut(w) else {
            return;
        };
        win.sheeted = true;
        let (frame, middle) = (win.frame, win.sheet);
        let (fw, fh) = win.geom.total;
        let e = edge.min(fw / 2).min(fh / 2);
        let pieces = [
            Rect::new(0, 0, fw, e),
            Rect::new(0, e, e, fh - 2 * e),
            Rect::new(fw - e, e, e, fh - 2 * e),
            Rect::new(0, fh - e, fw, e),
        ];
        let whole = |at: (i32, i32)| Rect::new(-at.0, -at.1, fw * scale, fh * scale);
        for (sb, r) in frame.into_iter().zip(pieces) {
            if r.is_empty() {
                self.scene.set_enabled(sb, false);
                continue;
            }
            let mut c = Canvas::new((r.width * scale) as u32, (r.height * scale) as u32);
            reprisekit::look::draw_sheet(&mut c, &theme, whole((r.x * scale, r.y * scale)));
            upload_scaled(&mut self.scene, sb, &c, scale);
            self.scene.set_position(sb, r.x, r.y);
            self.scene.set_input(sb, Input::Passthrough);
            self.scene.set_enabled(sb, true);
        }
        let inside = Rect::new(e, e, fw - 2 * e, fh - 2 * e);
        if inside.is_empty() {
            self.scene.set_enabled(middle, false);
            return;
        }
        let mut c = Canvas::new(1, 1);
        reprisekit::look::draw_sheet(&mut c, &theme, whole((fw * scale / 2, fh * scale / 2)));
        self.scene.buffer_set_canvas(middle, &c.pixels, 1, 1);
        self.scene
            .buffer_set_dest_size(middle, Some((inside.width, inside.height).into()));
        self.scene.set_position(middle, inside.x, inside.y);
        self.scene.set_enabled(middle, true);
    }

    /// Restore the frame nodes `draw_sheet` took (`draw_strips` sets the caption's and bottom's hit
    /// tests).
    fn unsheet(&mut self, w: WindowId) {
        let Some(win) = self.win_mut(w).filter(|x| x.sheeted) else {
            return;
        };
        win.sheeted = false;
        let (middle, frame, on) = (win.sheet, win.frame, win.has_frame());
        self.scene.set_enabled(middle, false);
        for sb in frame {
            self.scene.set_enabled(sb, on);
            self.scene.set_input(sb, Input::All);
        }
    }

    /// The 3D switcher's cards came up or went: while up, frames on its output draw inactive and
    /// their shadows hide (`carded`).
    pub(crate) fn cards_changed(&mut self) {
        let ids: Vec<(WindowId, bool)> = (self.windows.iter())
            .map(|x| (x.id, x.minimized || Some(x.id) == self.focused))
            .collect();
        for (w, redraw) in ids {
            if redraw {
                self.redraw_frame(w);
            } else {
                self.place_shadow(w);
            }
        }
    }

    pub fn redraw_caption(&mut self, w: WindowId) {
        if self.is_sheet(w) {
            return;
        }
        if self.win(w).is_some_and(Window::has_frame) {
            self.draw_strips(w, true);
        }
        let shown = self.tabs.groups.shown_of(w);
        if shown != w && self.win(shown).is_some_and(Window::has_frame) {
            self.draw_strips(shown, true);
        }
    }

    fn draw_strips(&mut self, w: WindowId, only_top: bool) {
        self.thumbs_frame(w);
        let Some(scale) = self.win(w).map(|x| x.frame_scale) else {
            return;
        };
        let theme = self.config.theme.scaled(scale);
        let icon = {
            let State {
                windows,
                icons,
                app_index,
                ..
            } = self;
            windows
                .iter()
                .find(|x| x.id == w)
                .and_then(|win| crate::icon::window_icon(icons, app_index, win, theme.icon_size()))
        };
        let strip = self.strip_for(w, theme.icon_size());
        let Some(win) = self.win(w) else {
            return;
        };
        let now = std::time::Instant::now();
        let geom = win.geom.scaled(scale);
        let layout = self.layout_box(None);
        let limit = (layout.width * scale, layout.height * scale);
        let title = win.title();
        let active = win.is_active(self.focused) && !self.carded(w);
        let state = FrameState {
            title: &title,
            active,
            maximized: win.maximized,
            pressed: win.pressed,
            glow: reprisekit::frame::Glow(std::array::from_fn(|i| win.glow[i].level(now))),
            icon: icon.as_deref(),
            tabbed: strip.is_some(),
        };
        let tabs = strip.as_ref().map(|st| CaptionTabs {
            tabs: (st.tabs.iter())
                .map(|t| TabEntry {
                    title: &t.title,
                    icon: t.icon.as_deref(),
                })
                .collect(),
            selected: st.selected,
            hot: st.hot,
            held: st.held,
            ghost: st.ghost,
        });
        let strips = win.geom.strips();
        let count = if only_top { 1 } else { strips.len() };
        let mut drawn = Vec::new();
        for (i, r) in geom.strips().iter().enumerate().take(count) {
            if r.is_empty() {
                drawn.push((i, None));
                continue;
            }
            let (cw, ch) = frame_canvas_size((r.width, r.height), limit);
            let mut canvas = Canvas::new(cw, ch);
            let at = geom.translated(r.x, r.y);
            reprisekit::look::draw_frame(
                &mut canvas,
                &theme,
                &self.text,
                &self.caption_text,
                &at,
                &state,
            );
            if let Some(tabs) = &tabs {
                captiontabs::draw(
                    &mut canvas,
                    &theme,
                    &self.text,
                    &self.caption_text,
                    &at,
                    tabs,
                    active,
                );
            }
            drawn.push((i, Some(canvas)));
        }
        let (frame, logical, maximized) = (win.frame, win.geom, win.maximized);
        for (i, canvas) in drawn {
            let sb = frame[i];
            let Some(canvas) = canvas else {
                self.scene.buffer_clear(sb);
                continue;
            };
            upload_scaled(&mut self.scene, sb, &canvas, scale);
            self.scene.set_position(sb, strips[i].x, strips[i].y);
            if i == 0 || i == 3 {
                let look = self.config.theme.look.clone();
                let geom = logical;
                let at = strips[i];
                self.scene.set_input(
                    sb,
                    Input::Test(Box::new(move |p| {
                        frame_takes(
                            &look,
                            &geom,
                            active,
                            maximized,
                            p.x.floor() as i32 + at.x,
                            p.y.floor() as i32 + at.y,
                        )
                    })),
                );
            }
        }
    }

    fn place_backdrop(&mut self, w: WindowId) {
        let sheet = self.is_sheet(w);
        self.place_client_glass(w, sheet);
        let pane = match self.config.theme.glass() {
            Some(g) if self.config.glass.transparency => self
                .win(w)
                .filter(|win| sheet || win.has_frame())
                .map(|win| match sheet {
                    true => sheet_pane(win.geom.total, g.corner),
                    false => {
                        let mut p = frame_pane(&win.geom, g.corner, win.maximized);
                        p.holes = client_holes(win.geom.client, &win.blur);
                        p
                    }
                }),
            _ => None,
        };
        let over = self.config.glass.material == Material::Molten;
        let State {
            scene,
            windows,
            glass,
            ..
        } = self;
        if let Some(win) = windows.iter_mut().find(|x| x.id == w) {
            win.glass.set(scene, glass, win.backdrop, pane);
            stack_shadow(scene, &win.shadow, win.backdrop, over);
        }
    }

    /// Lay the frame's glass under the client's blur region when the look has it and the window
    /// shows its frame; the client is then left out of what glass blurs, so its own glass reads
    /// what lies behind the window.
    fn place_client_glass(&mut self, w: WindowId, sheet: bool) {
        let active = self.win(w).is_some_and(|x| x.is_active(self.focused)) && !self.carded(w);
        let scale = self.draw_scale_for_window(w);
        let theme = crate::theme::Theme {
            active,
            ..self.config.theme.scaled(scale)
        };
        let opens = reprisekit::look::client_glass(&theme, active).is_some();
        let State { scene, windows, .. } = self;
        let Some(win) = windows.iter_mut().find(|x| x.id == w) else {
            return;
        };
        let rects: Vec<Rect> = match opens {
            true if win.has_frame() && !sheet => win.blur.clone(),
            _ => Vec::new(),
        };
        while win.client_glass.len() > rects.len() {
            if let Some(b) = win.client_glass.pop() {
                scene.destroy(b);
            }
        }
        while win.client_glass.len() < rects.len() {
            let b = scene.buffer_create(win.chrome);
            scene.set_input(b, Input::Passthrough);
            win.client_glass.push(b);
        }
        let c = win.geom.client;
        for (&b, r) in win.client_glass.iter().zip(&rects) {
            let mut canvas = Canvas::new((r.width * scale) as u32, (r.height * scale) as u32);
            let band = Rect::new(-r.x, -r.y, c.width, r.bottom()).scaled(scale);
            let client_h = c.height * scale;
            reprisekit::look::draw_client_glass(&mut canvas, &theme, band, client_h, win.maximized);
            upload_scaled(scene, b, &canvas, scale);
            scene.set_position(b, c.x + r.x, c.y + r.y);
        }
        let leave_out = !rects.is_empty() && crate::glass::wanted(&self.config);
        scene.set_in_backdrop(win.content, !leave_out);
    }

    /// Show the shadow under Glass; none for maximized, frameless or non-Glass windows, and a sheet
    /// shows the unfocused one.
    fn place_shadow(&mut self, w: WindowId) {
        let spec = self.config.theme.glass();
        let glass = spec.is_some();
        let square = spec.is_some_and(|g| g.corner.is_empty());
        let focused = self.focused;
        let Some(win) = self.win(w) else {
            return;
        };
        let shadow = win.shadow;
        if !glass && shadow.iter().all(|&sb| !self.scene.enabled(sb)) {
            return;
        }
        let sheet = self.is_sheet(w);
        let on = glass && (win.has_frame() || sheet) && !self.carded(w);
        let (fw, fh) = win.geom.total;
        let active = win.is_active(focused) && !sheet;
        let placed = shadow::place(Rect::new(0, 0, fw, fh), active, win.maximized || !on);
        let scale = win.frame_scale.max(1);
        let key = (scale, active, square);
        // Upload only on a key change; all eight repaint, hidden ones too, since the key is kept
        // for all.
        let repaint = on && win.shadow_key != Some(key);
        let pieces = repaint.then(|| self.shadows.pieces(scale, active, square));
        for (i, p) in placed.iter().enumerate() {
            let sb = shadow[i];
            if let Some(pieces) = &pieces {
                let piece = &pieces[i];
                self.scene.buffer_set_canvas(
                    sb,
                    &piece.pixels,
                    piece.width as i32,
                    piece.height as i32,
                );
            }
            if !on || p.dest.is_empty() {
                self.scene.set_enabled(sb, false);
                continue;
            }
            let s = f64::from(scale);
            self.scene.buffer_set_source_box(
                sb,
                Some(Rectangle::new(
                    (f64::from(p.src.x) * s, f64::from(p.src.y) * s).into(),
                    (f64::from(p.src.width) * s, f64::from(p.src.height) * s).into(),
                )),
            );
            self.scene
                .buffer_set_dest_size(sb, Some((p.dest.width, p.dest.height).into()));
            self.scene.set_position(sb, p.dest.x, p.dest.y);
            self.scene.set_enabled(sb, true);
        }
        if repaint && let Some(win) = self.win_mut(w) {
            win.shadow_key = Some(key);
        }
    }

    pub fn refit_scale(&mut self, w: WindowId) {
        let Some(win) = self.win(w) else {
            return;
        };
        if !win.mapped || !win.has_frame() {
            return;
        }
        if self.draw_scale_for_window(w) != win.frame_scale {
            self.redraw_frame(w);
        }
    }

    /// Tell the client whether it is active; an X11 window also takes/gives the X input focus and
    /// raises in X stacking.
    pub fn set_activated(&mut self, w: WindowId, on: bool) {
        if let Some(x) = self.win(w).and_then(Window::x11).cloned() {
            self.activate_x11(&x, on);
            return;
        }
        if let Some(win) = self.win(w)
            && let Some(t) = win.xdg()
        {
            let changed = t.with_pending_state(|s| {
                if on {
                    s.states.set(xdg_toplevel::State::Activated)
                } else {
                    s.states.unset(xdg_toplevel::State::Activated)
                }
            });
            if changed && t.is_initial_configure_sent() {
                t.send_pending_configure();
            }
        }
    }

    pub fn move_to(&mut self, w: WindowId, x: i32, y: i32) {
        self.drag_to(w, x, y);
        self.settle_move(w);
    }

    /// Move the tree only; `settle_move` does the rest before the next render. Called per pointer
    /// motion.
    pub fn drag_to(&mut self, w: WindowId, x: i32, y: i32) {
        let Some(win) = self.win_mut(w) else {
            return;
        };
        win.x = x;
        win.y = y;
        win.move_pending = true;
        let tree = win.tree;
        self.scene.set_position(tree, x, y);
        self.redraw_all();
    }

    pub fn settle_move(&mut self, w: WindowId) {
        let Some(win) = self.win_mut(w) else {
            return;
        };
        if !std::mem::take(&mut win.move_pending) {
            return;
        }
        self.refit_scale(w);
        self.refresh_bounds(w);
        self.place_ime_popup();
        self.place_x11(w);
        self.follow_tabs(w, false);
    }

    /// Finish moves `drag_to` left pending; for a dragged window, track caption and snap targets
    /// and untile clients that left a zone.
    pub fn settle_moves(&mut self) {
        let dragged = match self.interaction {
            Interaction::Move { window, .. }
                if self.win(window).is_some_and(|w| w.move_pending) =>
            {
                Some(window)
            }
            _ => None,
        };
        let pending: Vec<WindowId> = self
            .windows
            .iter()
            .filter(|w| w.move_pending)
            .map(|w| w.id)
            .collect();
        for w in pending {
            self.settle_move(w);
        }
        let frame = dragged.and_then(|w| self.win(w)).map(|w| w.frame_rect());
        self.water_edge_settle(frame);
        if let Some(w) = dragged {
            self.track_move(w);
        }
        self.sync_tiled();
    }

    pub fn track_move(&mut self, w: WindowId) {
        if crate::shakeview::shaking(self, w) {
            crate::shakeview::hold_drops(self);
            return;
        }
        // Held over a caption, the window joins its group on release and snaps nowhere.
        if self.track_tab_drop(w) {
            log::debug!(target: "reprise::snapbarview", "motion {w:?}: over a caption, no bar");
            self.hide_snap_outline();
            crate::snapbarview::end(self);
            if let Interaction::Move { snap, .. } = &mut self.interaction {
                *snap = None;
            }
        } else {
            self.track_snap(w);
        }
    }

    /// Ask `w` for a client size; a normal window is never asked for less than its minimum or more
    /// than its maximum.
    pub fn request_size(&mut self, w: WindowId, cw: i32, ch: i32) {
        let (cw, ch) = match self.win(w) {
            Some(x) if !x.maximized && !x.fullscreen => {
                crate::snap::commit_size((cw, ch), x.size_limits())
            }
            _ => (cw, ch),
        };
        if self.win(w).is_some_and(|x| x.x11().is_some()) {
            self.configure_x11(w, cw, ch);
            return;
        }
        if let Some(win) = self.win(w)
            && let Some(t) = win.xdg()
        {
            t.with_pending_state(|s| s.size = Some((cw.max(1), ch.max(1)).into()));
            let sent = (t.is_initial_configure_sent())
                .then(|| t.send_pending_configure())
                .flatten();
            if let Some(serial) = sent
                && let Some(win) = self.win_mut(w)
            {
                win.size_serial = Some(serial);
            }
        }
    }

    pub fn parent_window(&self, w: WindowId) -> Option<WindowId> {
        let p = self.win(w)?.parent_handle()?;
        self.windows.iter().find(|x| x.handle() == p).map(|x| x.id)
    }

    pub fn is_owner_of(&self, owner: WindowId, w: WindowId) -> bool {
        owner_chain_has(w, owner, |x| self.parent_window(x))
    }

    pub fn stays_on_top(&self, w: WindowId) -> bool {
        let mut at = w;
        for _ in 0..=self.windows.len() {
            if self.win(at).is_some_and(|x| x.on_top) {
                return true;
            }
            match self.parent_window(at) {
                Some(p) => at = p,
                None => return false,
            }
        }
        false
    }

    pub fn set_on_top(&mut self, w: WindowId, on: bool) {
        let Some(win) = self.win_mut(w) else {
            return;
        };
        if on == win.on_top {
            return;
        }
        win.on_top = on;
        let tree = win.tree;
        self.restack();
        self.scene.raise_to_top(tree);
        self.raise_children(w);
        self.redraw_all();
    }

    fn owned_by(&self, w: WindowId) -> Vec<WindowId> {
        let Some(own) = self.win(w).map(Window::handle) else {
            return Vec::new();
        };
        self.windows
            .iter()
            .filter(|x| x.mapped && x.parent_handle().as_ref() == Some(&own))
            .map(|x| x.id)
            .collect()
    }

    pub fn modal_target(&self, w: WindowId) -> WindowId {
        modal_end(w, |x| {
            self.owned_by(x)
                .into_iter()
                .rev()
                .find(|&k| self.win(k).is_some_and(Window::is_modal))
        })
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

    pub fn on_parent_changed(&mut self, w: WindowId) {
        if !self.win(w).is_some_and(|x| x.mapped) {
            return;
        }
        let Some((cx, cy)) = self.win(w).map(Window::client_origin) else {
            return;
        };
        self.relayout(w);
        if let Some(c) = self.win(w).map(|x| x.geom.client) {
            self.move_to(w, cx - c.x, cy - c.y);
        }
    }

    /// Switch between our frame and the client's; the client area keeps its layout position.
    pub fn set_csd(&mut self, w: WindowId, csd: bool) {
        let Some(win) = self.win_mut(w) else {
            return;
        };
        if csd == win.csd {
            return;
        }
        let (cx, cy) = win.client_origin();
        win.csd = csd;
        if !win.mapped {
            // A client may set its decoration after the initial commit (GTK3's KDE decoration);
            // re-plan the size for the frame it now gets.
            if win.placement.is_some() {
                self.configure_initial(w);
            }
            return;
        }
        self.relayout(w);
        if let Some(c) = self.win(w).map(|x| x.geom.client) {
            self.move_to(w, cx - c.x, cy - c.y);
        }
    }

    /// Hide or show the frame at the user's request; the outer rect stays and the client resizes to
    /// fill it.
    pub fn set_frameless(&mut self, w: WindowId, on: bool) {
        let m = self.config.theme.metrics.clone();
        let Some(win) = self.win_mut(w) else {
            return;
        };
        if on == win.frameless || win.csd || win.fullscreen || win.override_redirect {
            return;
        }
        let total = win.frame_rect();
        win.frameless = on;
        if !win.mapped {
            return;
        }
        let maximized = win.maximized;
        let (cw, ch) = FrameGeometry::client_size_for_total(
            &m,
            total.width,
            total.height,
            false,
            win.chrome(),
        );
        self.cancel_interaction_for(w);
        if maximized {
            self.fit_maximized(w);
            return;
        }
        // Relayout before the size request so an X11 configure carries the new position.
        self.relayout(w);
        self.request_size(w, cw, ch);
    }

    pub fn set_maximized(&mut self, w: WindowId, on: bool) {
        let w = self.tabs.groups.shown_of(w);
        let Some(win) = self.win(w) else {
            return;
        };
        if on == win.maximized {
            return;
        }
        if on && (win.is_owned() || !win.is_resizable()) {
            return;
        }
        // Fullscreen owns the geometry; ignored until it ends.
        if win.fullscreen {
            return;
        }
        self.cancel_interaction_for(w);
        let Some(win) = self.win_mut(w) else {
            return;
        };
        win.snapped = None;
        win.resize_anchor = None;
        let restore = if on {
            win.saved = Some(win.frame_rect());
            win.maximized = true;
            None
        } else {
            win.maximized = false;
            win.saved.take()
        };
        let restore = restore.map(|r| self.restore_to(w, r));
        if let Some(win) = self.win(w) {
            match &win.kind {
                Kind::Xdg(t) => t.with_pending_state(|s| {
                    if on {
                        s.states.set(xdg_toplevel::State::Maximized);
                    } else {
                        s.states.unset(xdg_toplevel::State::Maximized);
                    }
                }),
                Kind::X11(x) => {
                    if let Err(e) = x.set_maximized(on) {
                        log::debug!("x11 maximized: {e}");
                    }
                }
            }
        }
        if on {
            self.fit_maximized(w);
        } else {
            self.relayout(w);
            match restore {
                Some((cw, ch)) => self.request_size(w, cw, ch),
                None => self.send_configure(w),
            }
        }
    }

    pub fn send_configure(&self, w: WindowId) {
        if let Some(t) = self.win(w).and_then(Window::xdg)
            && t.is_initial_configure_sent()
        {
            t.send_pending_configure();
        }
    }

    pub fn fit_maximized(&mut self, w: WindowId) {
        let out = self.fit_target(w);
        let m = self.config.theme.metrics.clone();
        let Some(win) = self.win_mut(w) else {
            return;
        };
        win.x = out.x;
        win.y = out.y;
        let (cw, ch) =
            FrameGeometry::client_size_for_total(&m, out.width, out.height, true, win.chrome());
        self.relayout(w);
        self.request_size(w, cw, ch);
    }

    pub fn fit_target(&self, w: WindowId) -> Rect {
        if self.win(w).is_some_and(|x| x.fullscreen) {
            self.output_box_for_window(w)
        } else {
            self.usable_area_for_window(w)
        }
    }

    /// The work areas changed: maximized, fullscreen and snapped windows refit to theirs, and
    /// every xdg toplevel hears its new bounds.
    pub fn refit_maximized(&mut self) {
        let bounded: Vec<WindowId> = (self.windows.iter())
            .map(|w| w.id)
            .filter(|&w| self.set_bounds(w))
            .collect();
        let fits: Vec<WindowId> = self
            .windows
            .iter()
            .filter(|w| w.mapped && (w.maximized || w.fullscreen))
            .map(|w| w.id)
            .collect();
        for w in fits {
            if self.win(w).map(Window::frame_rect) != Some(self.fit_target(w)) {
                self.fit_maximized(w);
            }
        }
        self.refit_snapped();
        // A window refit above already sent its bounds with its size; this sends the rest.
        for w in bounded {
            self.send_configure(w);
        }
    }

    pub fn set_fullscreen(&mut self, w: WindowId, on: bool) {
        if self.win(w).is_none_or(|x| x.fullscreen == on) {
            return;
        }
        self.cancel_interaction_for(w);
        let Some(win) = self.win_mut(w) else {
            return;
        };
        win.snapped = None;
        win.resize_anchor = None;
        win.fullscreen = on;
        if on && !win.maximized {
            win.saved_fullscreen = Some(win.frame_rect());
        }
        match &win.kind {
            Kind::Xdg(t) => t.with_pending_state(|s| {
                if on {
                    s.states.set(xdg_toplevel::State::Fullscreen);
                } else {
                    s.states.unset(xdg_toplevel::State::Fullscreen);
                }
            }),
            Kind::X11(x) => {
                if let Err(e) = x.set_fullscreen(on) {
                    log::debug!("x11 fullscreen: {e}");
                }
            }
        }
        let maximized = win.maximized;
        if on || maximized {
            self.fit_maximized(w);
        } else {
            let saved = self.win_mut(w).and_then(|x| x.saved_fullscreen.take());
            let restore = saved.map(|r| self.restore_to(w, r));
            self.relayout(w);
            match restore {
                Some((cw, ch)) => self.request_size(w, cw, ch),
                None => self.send_configure(w),
            }
        }
        self.restack();
        crate::idle::inhibition_changed(self);
    }

    /// Raise windows owned by `w` above it, recursively.
    pub fn raise_children(&mut self, w: WindowId) {
        let mut order = Vec::new();
        walk_owned(w, (), |x| self.owned_by(x), |k, ()| order.push(k));
        for k in order {
            if let Some(tree) = self.win(k).map(|x| x.tree) {
                self.scene.raise_to_top(tree);
            }
        }
    }

    pub(crate) fn show_children(&mut self, w: WindowId, on: bool) {
        let mut shown = Vec::new();
        walk_owned(
            w,
            on,
            |x| self.owned_by(x),
            |k, owner_shown| {
                let show = owner_shown && self.win(k).is_some_and(|k| !k.hidden());
                shown.push((k, show));
                show
            },
        );
        for (k, show) in shown {
            if let Some(tree) = self.win(k).map(|x| x.tree) {
                self.scene.set_enabled(tree, show);
            }
        }
    }

    pub fn set_minimized(&mut self, w: WindowId, on: bool) {
        let w = if on { self.tabs.groups.shown_of(w) } else { w };
        let shown = self.ripple_frame(w);
        let Some(win) = self.win_mut(w) else {
            return;
        };
        if on == win.minimized {
            return;
        }
        if on && win.is_owned() {
            return;
        }
        win.minimized = on;
        if let Some(x) = win.x11()
            && let Err(e) = x.set_hidden(on)
        {
            log::debug!("x11 hidden: {e}");
        }
        let (tree, mapped) = (win.tree, win.mapped);
        let here = self.on_current_desktop(w);
        self.scene.set_enabled(tree, !on && mapped && here);
        let ring = if on { shown } else { self.ripple_frame(w) };
        if let Some(r) = ring {
            self.water_ring(r);
        }
        self.show_children(w, !on && here);
        crate::idle::inhibition_changed(self);
        if on {
            self.set_activated(w, false);
            if self.focused == Some(w) {
                self.focused = None;
                self.focus_top();
            }
        } else {
            self.focus_window(w);
        }
        self.redraw_taskbars();
        self.redraw_all();
    }

    /// A normal window that took a size of its own after acking every configure keeps it in its
    /// pending state, so a later state-only configure (activation, bounds) does not ask it back
    /// to the size last sent.
    fn keep_own_size(&self, w: WindowId) {
        let Some(win) = self.win(w) else {
            return;
        };
        let held = win.maximized || win.fullscreen || win.snapped.is_some();
        if held
            || win.resize_anchor.is_some()
            || self.interaction.is_resize_of(w)
            || self.wm.snap.follows_drag(w)
        {
            return;
        }
        let Some(t) = win.xdg().filter(|t| !t.has_pending_changes()) else {
            return;
        };
        let own = win.client_size();
        let acked = t.with_committed_state(|s| s.cloned());
        t.with_pending_state(|s| {
            if acked.as_ref() == Some(&*s) && s.size.is_some_and(|z| (z.w, z.h) != own) {
                s.size = Some(own.into());
            }
        });
    }

    /// Drop minimized/maximized/fullscreen/snapped state on unmap. An xdg toplevel takes the
    /// cleared state with its next initial configure; X11's `_NET_WM_STATE` clears at once.
    fn reset_state(&mut self, w: WindowId) {
        let Some(win) = self.win_mut(w) else {
            return;
        };
        let was_minimized = std::mem::take(&mut win.minimized);
        win.fullscreen = false;
        win.maximized = false;
        win.saved = None;
        win.saved_fullscreen = None;
        win.snapped = None;
        match &win.kind {
            Kind::Xdg(t) => t.with_pending_state(|s| {
                s.states.unset(xdg_toplevel::State::Maximized);
                s.states.unset(xdg_toplevel::State::Fullscreen);
                s.size = None;
            }),
            Kind::X11(x) => {
                let cleared = x
                    .set_maximized(false)
                    .and_then(|()| x.set_fullscreen(false))
                    .and_then(|()| x.set_hidden(false));
                if let Err(e) = cleared {
                    log::debug!("x11 state: {e}");
                }
            }
        }
        if was_minimized {
            self.show_children(w, self.on_current_desktop(w));
        }
    }

    pub fn close(&mut self, w: WindowId) {
        match self.win(w).map(|x| &x.kind) {
            Some(Kind::Xdg(t)) => t.send_close(),
            Some(Kind::X11(x)) => {
                if let Err(e) = x.close() {
                    log::debug!("x11 close: {e}");
                }
            }
            None => return,
        }
        self.ping_on_close(w);
    }

    pub fn handle_commit(&mut self, w: WindowId) {
        self.apply_icon(w);
        self.window_alive(w);
        self.thumbs_commit(w);
        let Some(win) = self.win_mut(w) else {
            return;
        };
        let caps_changed = match win.kind {
            Kind::Xdg(_) => match win.send_capabilities() {
                Some(changed) => changed,
                None if win.mapped => {
                    // A null buffer unmaps; the next commit is initial again.
                    win.wm_caps = None;
                    self.unmap_window(w);
                    return;
                }
                None => {
                    self.configure_initial(w);
                    if let Some(t) = self.win(w).and_then(Window::xdg) {
                        t.send_configure();
                    }
                    return;
                }
            },
            Kind::X11(_) => false,
        };
        self.commit_buffer(w, caps_changed);
    }

    fn commit_buffer(&mut self, w: WindowId, caps_changed: bool) {
        let Some(win) = self.win_mut(w) else {
            return;
        };
        let blur =
            crate::clientglass::region(win.wl_surface(), win.geometry_origin(), win.client_size());
        let reglassed = blur != win.blur;
        win.blur = blur;
        let Some(win) = self.win(w) else {
            return;
        };
        let has_buffer = with_renderer_surface_state(win.wl_surface(), |s| s.buffer().is_some())
            .unwrap_or(false);
        match (win.mapped, has_buffer) {
            (false, true) => {
                self.map_window(w);
                return;
            }
            (true, false) => {
                self.unmap_window(w);
                return;
            }
            (false, false) => return,
            (true, true) => {}
        }
        if caps_changed {
            self.send_configure(w);
        }
        let Some(win) = self.win(w) else {
            return;
        };
        let resized = win.client_size() != (win.client_w, win.client_h)
            || (win.surface_origin() != self.scene_surface_origin(w));
        let was = win.frame_rect();
        if reglassed && !resized {
            self.relayout(w);
        }
        if resized {
            self.relayout(w);
            self.keep_grown_inside(w, was);
            if let Some(win) = self.win(w)
                && let Some(a) = win.resize_anchor
            {
                let (x, y) = anchored_position(&a, win.geom.total, (win.x, win.y));
                if (x, y) != (win.x, win.y) {
                    self.move_to(w, x, y);
                }
            }
            let sizing = self.interaction.is_resize_of(w) || self.wm.snap.follows_drag(w);
            self.follow_tabs(w, !sizing);
            self.keep_own_size(w);
        }
        // The commit for the last requested size usually lands after release, so the anchor
        // outlives the drag until `anchor_done`.
        if let Some(win) = self.win(w)
            && win.resize_anchor.is_some()
            && !self.interaction.is_resize_of(w)
            && !self.wm.snap.follows_drag(w)
        {
            let acked = (win.xdg())
                .and_then(|t| t.with_cached_state(|s| s.last_acked.as_ref().map(|c| c.serial)));
            if anchor_done(win.size_serial, acked, resized)
                && let Some(win) = self.win_mut(w)
            {
                win.resize_anchor = None;
                self.remember_file_dialog(w);
            }
        }
        self.refit_scale(w);
    }
}

impl Window {
    /// Update capabilities; returns whether they changed, or None if the initial configure is still
    /// to be sent.
    fn send_capabilities(&mut self) -> Option<bool> {
        let Kind::Xdg(toplevel) = &self.kind else {
            return Some(false);
        };
        // Recomputed every commit: the xdg parent is set after the initial commit.
        let owned = self.is_owned();
        let caps = (!owned, !owned && self.is_resizable());
        // Resending on every commit risks a configure / ack / commit loop.
        let caps_changed = self.wm_caps != Some(caps);
        if caps_changed {
            toplevel.with_pending_state(|s| {
                s.capabilities.unset(xdg_toplevel::WmCapabilities::Minimize);
                s.capabilities.unset(xdg_toplevel::WmCapabilities::Maximize);
                if caps.0 {
                    s.capabilities.set(xdg_toplevel::WmCapabilities::Minimize);
                }
                if caps.1 {
                    s.capabilities.set(xdg_toplevel::WmCapabilities::Maximize);
                }
            });
        }
        self.wm_caps = Some(caps);
        toplevel.is_initial_configure_sent().then_some(caps_changed)
    }
}

impl State {
    fn scene_surface_origin(&self, w: WindowId) -> (i32, i32) {
        self.win(w)
            .and_then(|x| self.scene.coords(x.surface))
            .map_or((0, 0), |(p, _)| (p.x, p.y))
    }

    pub fn sync_popups(&mut self, w: WindowId) {
        let Some(win) = self.win(w) else {
            return;
        };
        let content = win.content;
        let open: Vec<(PopupKind, Point<i32, Logical>)> =
            PopupManager::popups_for_surface(win.wl_surface()).collect();
        let mut kept = Vec::new();
        let mut old = win.popups.clone();
        for (popup, offset) in open {
            let surface = popup.wl_surface().clone();
            let node = match old.iter().position(|(s, _)| *s == surface) {
                Some(i) => old.swap_remove(i).1,
                None => self.scene.surface_create(content, surface.clone()),
            };
            // The content tree's origin is the window geometry origin, which popup offsets are
            // from.
            let at = offset - popup.geometry().loc;
            self.scene.set_position(node, at.x, at.y);
            self.scene.raise_to_top(node);
            kept.push((surface, node));
        }
        for (_, node) in old {
            self.scene.destroy(node);
        }
        if let Some(win) = self.win_mut(w) {
            win.popups = kept;
        }
    }

    pub fn window_for_popup(&self, surface: &WlSurface) -> Option<WindowId> {
        let kind = self.popups.find_popup(surface)?;
        let root = find_popup_root_surface(&kind).ok()?;
        self.window_for_surface(&root)
    }

    /// Fits a popup's pending geometry to its positioner: a window's popups inside the area the
    /// window fits to (the taskbar stacks above them), a panel's inside its output.
    pub fn unconstrain_popup(&self, popup: &PopupSurface) {
        let kind = PopupKind::Xdg(popup.clone());
        let bounds = find_popup_root_surface(&kind)
            .ok()
            .and_then(|root| self.popup_bounds(&root));
        let parent = get_popup_toplevel_coords(&kind);
        popup.with_pending_state(|s| {
            s.geometry = match bounds {
                Some((area, (ox, oy))) => {
                    let target = Rectangle::new(
                        (area.x - ox - parent.x, area.y - oy - parent.y).into(),
                        (area.width, area.height).into(),
                    );
                    s.positioner.get_unconstrained_geometry(target)
                }
                None => s.positioner.get_geometry(),
            };
        });
    }

    /// The layout rect a popup under `root` stays inside, and the layout point its offsets are
    /// from.
    fn popup_bounds(&self, root: &WlSurface) -> Option<(Rect, (i32, i32))> {
        if let Some(w) = self.window_for_surface(root) {
            let origin = self.win(w)?.client_origin();
            let area = Some(self.fit_target(w))
                .filter(|r| r.width > 0 && r.height > 0)
                .unwrap_or_else(|| self.output_box_for_window(w));
            return Some((area, origin));
        }
        self.layer_popup_bounds(root)
    }

    pub fn map_window(&mut self, w: WindowId) {
        if self.win(w).is_some_and(|x| x.override_redirect) {
            self.map_or_window(w);
            return;
        }
        self.note_app_id(w);
        self.watch_threads(w);
        let current = self.desktops.current;
        let Some(win) = self.win_mut(w) else {
            return;
        };
        win.mapped = true;
        win.desktop = current;
        // A client can gain an owner between first configure and map.
        let placeable = win.placeable();
        let mut want = win.placement.take().filter(|_| placeable);
        let (app, title) = (win.app(), win.title());
        // Windows of one app planned together can each plan the same saved
        // tab group; the one that maps as another group's takes its frame.
        let mut resized = false;
        if placeable
            && self
                .session
                .regrouping
                .frame(&app, &title, std::time::Instant::now())
                .is_some()
        {
            let fresh = self.plan_window(w);
            if fresh != want {
                let size = |x: &Option<Want>| x.as_ref().and_then(|x| x.size);
                resized = size(&fresh) != size(&want);
                want = fresh;
            }
        }
        if placeable {
            self.session.restoring.mapped(&app);
        }
        let out = self.placement_output(want.as_ref());
        let area = self.usable_area(out.as_ref());
        let owner = self
            .parent_window(w)
            .filter(|&p| self.win(p).is_some_and(|p| p.mapped));
        let resizable = self.win(w).is_some_and(Window::is_resizable);
        let position = want.as_ref().and_then(|x| x.position);
        let rescaled = want.as_ref().and_then(|x| x.rescaled);
        // X11 position read before `relayout` configures it.
        let x11_at = self
            .win(w)
            .and_then(Window::x11)
            .map(|x| x.last_configure().loc)
            .filter(|at| (at.x, at.y) != (0, 0));
        self.relayout(w);
        let Some(total) = self.win(w).map(|x| x.geom.total) else {
            return;
        };
        let m = self.config.theme.metrics.clone();
        // The wanted frame size, so a window that maps maximized keeps it to restore to.
        let mut size = None;
        let kept = self.file_dialog_position(w);
        let (mut x, mut y) = match (owner, position, kept) {
            (_, _, Some(at)) => at,
            (Some(p), _, None) => {
                let frame = self.win(p).map_or(area, Window::frame_rect);
                centre_on(frame, total, area)
            }
            (None, Some(pos), None) => {
                let s = match (want.as_ref().and_then(|x| x.size), rescaled) {
                    (Some(s), _) => fit_size(s, area),
                    (None, Some(sh)) => fit_size(sh.size(total), area),
                    (None, None) => total,
                };
                // A normal frame is at least as big as the client's minimum made it; a rescaled
                // one opened at the client's own size.
                let normal = !matches!(
                    want.as_ref().and_then(|x| x.state),
                    Some(WindowState::Maximized | WindowState::Fullscreen)
                );
                let s = if normal && rescaled.is_none() {
                    (s.0.max(total.0), s.1.max(total.1))
                } else {
                    s
                };
                size = Some(s);
                match pos {
                    Position::At(px, py) => {
                        let b = self.layout_box(out.as_ref());
                        keep_inside((b.x + px, b.y + py), s, area)
                    }
                    Position::Cascade => {
                        let app_id = self.win(w).map(Window::app).unwrap_or_default();
                        match self
                            .newest_of_app(&app_id, Some(w))
                            .and_then(|p| self.win(p))
                        {
                            Some(p) => cascade_from((p.x, p.y), s, area, &m),
                            None => cascade_position(0, area, &m),
                        }
                    }
                }
            }
            // A fixed-size ownerless window (polkit prompt, About box) centres on the screen.
            (None, None, None) if !resizable => centre_on(area, total, area),
            (None, None, None) => {
                let index = self
                    .windows
                    .iter()
                    .filter(|x| x.mapped && !x.override_redirect)
                    .count()
                    .saturating_sub(1);
                cascade_position(index, area, &m)
            }
        };
        // An X11 client that placed itself away from the origin keeps that place unless a rule
        // places it.
        if position.is_none()
            && let Some(at) = x11_at
            && let Some(win) = self.win(w)
        {
            let frame = Rect::new(
                at.x - win.geom.client.x,
                at.y - win.geom.client.y,
                total.0,
                total.1,
            );
            let frame = self.caption_clear(frame);
            (x, y) = (frame.x, frame.y);
        }
        let desktops = self.desktops.count;
        if let Some(win) = self.win_mut(w) {
            win.x = x;
            win.y = y;
            win.tab_group = want.as_ref().and_then(|x| x.tab_group.clone());
            if let Some(want) = &want {
                log::info!("window {:?}: placed at {x},{y} for {want:?}", win.app_id());
                win.apply_rule(want, desktops);
            }
        }
        self.relayout(w);
        // An X11 client can resize itself before map, and a regrouped window can have planned
        // another size: ask for the planned size again. A rescaled frame other than the client's
        // own size is asked for now.
        let again = resized || self.win(w).is_some_and(|x| x.x11().is_some());
        let ask = match (want.as_ref().and_then(|x| x.size), rescaled) {
            (Some(s), _) if again => Some(fit_size(s, area)),
            (None, Some(sh)) => Some(fit_size(sh.size(total), area)).filter(|&s| s != total),
            _ => None,
        };
        if let Some((fw, fh)) = ask
            && let Some(chrome) = self.win(w).map(Window::chrome)
        {
            let (cw, ch) = FrameGeometry::client_size_for_total(&m, fw, fh, false, chrome);
            self.request_size(w, cw, ch);
        }
        // A dialog whose owner is on another desktop, a rule's target desktop, or a hidden tab maps
        // there hidden.
        let here = self.on_current_desktop(w);
        let home = self.home(w);
        let shown = here && (home == w || !self.win(home).is_some_and(|h| h.tab_hidden));
        if let Some(tree) = self.win(w).map(|x| x.tree) {
            self.scene.set_enabled(tree, shown);
        }
        crate::idle::inhibition_changed(self);
        self.restack();
        let mut activated = self.win(w).is_some_and(|x| x.activate_on_map);
        let locked = self.is_locked();
        let bar_keys = self.bar_shared.keys.is_some();
        let on_map = crate::mapfocus::on_map(locked, here, bar_keys, &mut activated, |a| {
            self.takes_focus_on_map(w, a)
        });
        if let Some(win) = self.win_mut(w) {
            win.activate_on_map = activated;
        }
        match on_map {
            OnMap::Locked => self.raise_children(w),
            OnMap::Elsewhere => {
                // Its desktop's pager button asks for attention instead, unless a rule sent it
                // there.
                let ruled = want.as_ref().is_some_and(|x| x.desktop.is_some());
                if let Some(win) = self.win_mut(w) {
                    win.attention = !ruled;
                }
                self.place_behind_focus(w);
            }
            OnMap::Focus => self.focus_window(w),
            OnMap::Behind => self.place_behind_focus(w),
            OnMap::Attention => self.map_under_bar_keys(w),
        }
        // Pre-map state: the client's request (xdg pending / X11 `_NET_WM_STATE`) or the first
        // configure's plan; a rule's state wins. An xdg request is taken out of pending.
        let asked = self.win(w).map(|x| match &x.kind {
            Kind::Xdg(t) => t.with_pending_state(|s| {
                let asked = (
                    s.states.contains(xdg_toplevel::State::Maximized),
                    s.states.contains(xdg_toplevel::State::Fullscreen),
                );
                s.states.unset(xdg_toplevel::State::Maximized);
                s.states.unset(xdg_toplevel::State::Fullscreen);
                asked
            }),
            Kind::X11(x) => (x.is_maximized(), x.is_fullscreen()),
        });
        let (asked_maximized, asked_fullscreen) = asked.unwrap_or_default();
        let state = want.as_ref().and_then(|x| x.state);
        // Maximize before fullscreen, which would refuse it, so leaving fullscreen lands maximized.
        if state.map_or(asked_maximized, |s| s == WindowState::Maximized) {
            self.set_maximized(w, true);
            if let Some(win) = self.win_mut(w)
                && win.maximized
                && let Some(s) = size
            {
                win.saved = Some(Rect::new(x, y, s.0, s.1));
            }
        }
        if state.map_or(asked_fullscreen, |s| s == WindowState::Fullscreen) {
            self.set_fullscreen(w, true);
            if let Some(win) = self.win_mut(w)
                && !win.maximized
                && let Some(s) = size
            {
                win.saved_fullscreen = Some(Rect::new(x, y, s.0, s.1));
            }
        }
        if state == Some(WindowState::Minimized) {
            self.set_minimized(w, true);
        }
        // A window mapped without focus gets an inactive configure even if the state is unchanged,
        // since the client may have drawn active.
        if let Some(t) = self.win(w).and_then(Window::xdg)
            && t.is_initial_configure_sent()
        {
            t.send_configure();
        }
        if placeable {
            self.tab_on_map(w);
        }
        self.sync_toplevel(w);
        self.write_x11_state(w);
        self.window_opening(w);
        crate::peek::window_mapped(self, w);
        self.flip_join();
        self.overview_refresh();
        self.redraw_taskbars();
        self.redraw_all();
    }

    fn map_or_window(&mut self, w: WindowId) {
        let Some(win) = self.win_mut(w) else {
            return;
        };
        win.mapped = true;
        if let Some(x) = win.x11() {
            // `geometry()` is the buffer size at (0, 0); use where the client last put it.
            let at = x.last_configure().loc;
            (win.x, win.y) = (at.x, at.y);
        }
        let wants = win.wants_focus_on_click();
        let icon = win.x11().and_then(|x| x.window_type()) == Some(WmWindowType::Dnd);
        let surface = win.surface;
        if icon {
            self.scene.set_input(surface, Input::Passthrough);
        }
        self.relayout(w);
        let Some(tree) = self.win(w).map(|x| x.tree) else {
            return;
        };
        self.scene.set_enabled(tree, true);
        crate::idle::inhibition_changed(self);
        self.restack();
        self.scene.raise_to_top(tree);
        if wants {
            self.focus_window(w);
        }
        self.redraw_all();
    }

    /// The one place that clears a `WindowId` from every holder outside `State::windows`.
    pub fn forget_window(&mut self, w: WindowId) {
        let was_focused = self.focused == Some(w);
        self.cancel_interaction_for(w);
        self.end_task_press_for(w);
        self.focus_order.retain(|p| *p != w);
        let launch = self.win_mut(w).and_then(|x| x.launch.take());
        if self.focused == Some(w) {
            self.focused = None;
            if let Some(l) = launch {
                crate::mapfocus::give_back(l);
            }
        }
        if self.hot_frame == Some(w) {
            self.hot_frame = None;
        }
        if let Some(win) = self.win_mut(w) {
            win.hot = None;
            win.glow = Default::default();
        }
        self.fading_frames.retain(|p| *p != w);
        self.forget_task_window(w);
        self.close_window_menu_for(w);
        self.switcher_forget(crate::switcherview::Entry::Window(w));
        self.withdraw_toplevel(w);
        crate::taskpreview::forget_window(self, w);
        crate::snappickerview::forget_window(self, w);
        crate::snapbarview::forget_window(self, w);
        self.wm.snap.forget_window(w);
        crate::peek::forget_window(self, w);
        self.forget_tabs(w, was_focused);
        self.session.regrouping.forget(w);
        // The window is mid-teardown, still in the scene: the overview
        // drops it once the loop is idle.
        if self.overview.view.is_some() {
            self.handle.insert_idle(|state| state.overview_refresh());
        }
    }

    /// Light the caption button `hit` is on and put out any other window's; fades only when the hot
    /// button changes. Classic lights nothing.
    pub fn frame_hover(&mut self, window: Option<WindowId>, hit: FrameHit) {
        let styled = self.config.theme.look.style().is_some();
        let want = match hit {
            FrameHit::Button(b) if styled && window.is_some() => Some(b),
            _ => None,
        };
        if let Some(old) = self.hot_frame
            && Some(old) != window
            && self.win(old).is_some_and(|o| o.hot.is_some())
        {
            if let Some(o) = self.win_mut(old) {
                o.hot = None;
            }
            self.glow_to_hot(old);
        }
        self.hot_frame = None;
        let Some(window) = window else {
            return;
        };
        if self.win(window).is_some_and(|x| x.hot != want) {
            if let Some(x) = self.win_mut(window) {
                x.hot = want;
            }
            self.glow_to_hot(window);
        }
        if want.is_some() {
            self.hot_frame = Some(window);
        }
    }

    pub fn unmap_window(&mut self, w: WindowId) {
        // Before anything hides it: its still is taken as it shows.
        self.window_closing(w);
        let was_focused = self.focused == Some(w);
        self.remember_window(w, true);
        self.remember_file_dialog(w);
        self.forget_window(w);
        self.reset_state(w);
        let Some(win) = self.win_mut(w) else {
            return;
        };
        win.mapped = false;
        let tree = win.tree;
        self.scene.set_enabled(tree, false);
        crate::idle::inhibition_changed(self);
        if was_focused {
            self.focus_top();
        }
        self.redraw_taskbars();
        self.redraw_all();
    }

    pub fn destroy_window(&mut self, w: WindowId) {
        // A closing X11 menu or tooltip leaves the focus where it is.
        let refocus = self.focused == Some(w) || !self.win(w).is_some_and(|x| x.override_redirect);
        if self.win(w).is_some_and(|x| x.mapped) {
            self.remember_window(w, true);
            self.window_closing(w);
        }
        self.finish_window_anim(w);
        self.forget_window(w);
        if let Some(i) = self.windows.iter().position(|x| x.id == w) {
            let win = self.windows.remove(i);
            self.scene.destroy(win.tree);
        }
        self.close_ended_programs();
        if refocus {
            self.focus_top();
        }
        self.redraw_taskbars();
        self.redraw_all();
    }

    pub fn cancel_interaction_for(&mut self, w: WindowId) {
        let involved = self.interaction.acts_on(w);
        if let (
            true,
            Interaction::Resize {
                window, keyboard, ..
            },
        ) = (involved, self.interaction)
        {
            self.set_resizing(window, false);
            // Settled before the caller snaps, sizes or closes `w`, so the next size starts clean.
            if keyboard.is_some() {
                self.cancel_group_resize();
            } else {
                self.end_group_resize();
            }
        }
        if involved {
            self.interaction = Interaction::Idle;
            self.hide_snap_outline();
        }
        if let Some(win) = self.win_mut(w) {
            win.pressed = None;
            win.resize_anchor = None;
        }
        crate::input::caption_cancelled(&mut self.caption_clicks, w);
    }

    pub fn set_resizing(&mut self, w: WindowId, on: bool) {
        if let Some(t) = self.win(w).and_then(Window::xdg) {
            t.with_pending_state(|s| {
                if on {
                    s.states.set(xdg_toplevel::State::Resizing);
                } else {
                    s.states.unset(xdg_toplevel::State::Resizing);
                }
            });
            self.send_configure(w);
        }
        if !on {
            self.follow_tabs(w, true);
        }
    }

    pub fn output_box_for_window(&self, w: WindowId) -> Rect {
        let Some(r) = self.win(w).map(Window::frame_rect) else {
            return Rect::default();
        };
        let out = self.output_at((r.x + r.width / 2) as f64, (r.y + r.height / 2) as f64);
        let b = self.layout_box(out.as_ref());
        if b.width <= 0 {
            return r;
        }
        b
    }

    pub fn draw_scale_for_window(&self, w: WindowId) -> i32 {
        let Some(r) = self.win(w).map(Window::frame_rect) else {
            return 1;
        };
        let out = self.output_at((r.x + r.width / 2) as f64, (r.y + r.height / 2) as f64);
        self.draw_scale_at(out.as_ref())
    }

    pub fn centre_window(&mut self, w: WindowId, output: Option<&smithay::output::Output>) {
        let Some(win) = self.win(w) else {
            return;
        };
        if win.maximized || win.fullscreen || self.layout_box(output).width <= 0 {
            return;
        }
        self.cancel_interaction_for(w);
        let area = self.usable_area(output);
        let Some(win) = self.win_mut(w) else {
            return;
        };
        win.snapped = None;
        let r = win.frame_rect();
        let (x, y) = centre_on(area, (r.width, r.height), area);
        self.move_to(w, x, y);
    }

    /// A window that was inside the work area and grew past it on its own (a program growing
    /// with the text size) moves back in as little as it takes; one the user is sizing, or
    /// one already partly outside, stays put.
    fn keep_grown_inside(&mut self, w: WindowId, was: Rect) {
        let Some(win) = self.win(w) else {
            return;
        };
        let now = win.frame_rect();
        let user = win.resize_anchor.is_some()
            || win.maximized
            || win.fullscreen
            || self.interaction.is_resize_of(w)
            || self.wm.snap.follows_drag(w);
        let area = self.usable_area_for_window(w);
        if let Some((x, y)) = grown_back_inside(was, now, area).filter(|_| !user) {
            self.move_to(w, x, y);
        }
    }

    /// The screen and work area of the output under `r`'s centre; `None` off every output.
    pub(crate) fn screen_and_area(&self, r: Rect) -> Option<(Rect, Rect)> {
        let out = self.output_at(f64::from(r.x + r.width / 2), f64::from(r.y + r.height / 2));
        let screen = self.layout_box(out.as_ref());
        (screen.width > 0).then(|| (screen, self.usable_area(out.as_ref())))
    }

    /// After a look or text size change re-lays out `w`'s frame (it was `was` in work area
    /// `was_area`), a normal window that was on screen moves back inside the work area if its
    /// frame or the taskbar grew. Maximized and snapped ones refit on their own.
    pub(crate) fn keep_on_screen_inside(&mut self, w: WindowId, was: Rect, was_area: Rect) {
        let Some(win) = self.win(w) else {
            return;
        };
        if win.maximized || win.fullscreen || win.snapped.is_some() {
            return;
        }
        let now = win.frame_rect();
        let Some((screen, area)) = self.screen_and_area(was) else {
            return;
        };
        if let Some((x, y)) = retheme_back_inside(was, now, screen, was_area, area) {
            self.move_to(w, x, y);
        }
    }

    /// Send xdg toplevel `w` the bounds of the work area its frame's centre is on, when they
    /// differ from the ones it last heard.
    pub fn refresh_bounds(&mut self, w: WindowId) {
        if self.set_bounds(w) {
            self.send_configure(w);
        }
    }

    /// Put the bounds `refresh_bounds` sends into `w`'s pending state; true when they changed.
    fn set_bounds(&self, w: WindowId) -> bool {
        let Some(win) = self.win(w).filter(|w| w.mapped) else {
            return false;
        };
        let Some(t) = win.xdg() else {
            return false;
        };
        let r = win.frame_rect();
        let out = self.output_at((r.x + r.width / 2) as f64, (r.y + r.height / 2) as f64);
        let area = self.usable_area(out.as_ref());
        if out.is_none() || area.width <= 0 || area.height <= 0 {
            return false;
        }
        let most = area_client(&self.config.theme.metrics, area, win.chrome());
        t.with_pending_state(|s| {
            let was = s.bounds.replace(most.into());
            was != s.bounds
        })
    }

    pub fn usable_area_for_window(&self, w: WindowId) -> Rect {
        let Some(r) = self.win(w).map(Window::frame_rect) else {
            return Rect::default();
        };
        let out = self.output_at((r.x + r.width / 2) as f64, (r.y + r.height / 2) as f64);
        if self.layout_box(out.as_ref()).width <= 0 {
            return r;
        }
        self.usable_area(out.as_ref())
    }
}

/// A layout rect as X carries it: 16-bit origin, size >= 1, saturated.
pub(crate) fn x11_geometry(x: i32, y: i32, w: i32, h: i32) -> (i16, i16, u16, u16) {
    let pos = |v: i32| v.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
    let len = |v: i32| v.clamp(1, u16::MAX as i32) as u16;
    (pos(x), pos(y), len(w), len(h))
}

/// Clamp a client size to 1..=32767 per side so sums cannot overflow `i32`; wp_viewport lets a
/// client claim any size.
fn client_extent(w: i32, h: i32) -> (i32, i32) {
    let len = |v: i32| v.clamp(1, i16::MAX as i32);
    (len(w), len(h))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SizeLimits {
    pub min: (i32, i32),
    pub max: (Option<i32>, Option<i32>),
}

impl SizeLimits {
    pub fn resizable(self) -> bool {
        self.max != (Some(self.min.0), Some(self.min.1))
    }
}

/// From (min_w, min_h, max_w, max_h); a max <= 0 is unbounded as in xdg-shell.
pub(crate) fn client_limits(limits: (i32, i32, i32, i32)) -> SizeLimits {
    let (min_w, min_h, max_w, max_h) = limits;
    let max = |v: i32| (v > 0).then(|| v.min(i16::MAX as i32));
    SizeLimits {
        min: client_extent(min_w, min_h),
        max: (max(max_w), max(max_h)),
    }
}

/// Stack the shadow pieces just under the glass `backdrop` or (`over`) just above it, frame strips
/// on top. Molten's lens reads past the frame edge, so a shadow under the glass bends into the
/// caption as a dark line.
/// The parts of `client` (frame coordinates) the frame's glass leaves clear: all of it but the
/// `blur` rects (client coordinates).
fn client_holes(client: Rect, blur: &[Rect]) -> Vec<Rect> {
    let r = |x: Rect| Rectangle::<i32, Logical>::new((x.x, x.y).into(), (x.width, x.height).into());
    let glass = blur
        .iter()
        .map(|b| r(Rect::new(client.x + b.x, client.y + b.y, b.width, b.height)));
    r(client)
        .subtract_rects(glass)
        .into_iter()
        .map(|h| Rect::new(h.loc.x, h.loc.y, h.size.w, h.size.h))
        .collect()
}

fn stack_shadow(scene: &mut DeskScene, shadow: &[BufferId; 8], backdrop: TreeId, over: bool) {
    let Some(parent) = scene.parent(backdrop) else {
        return;
    };
    let children = scene.children(parent);
    let at = |n: NodeId| children.iter().position(|c| *c == n);
    let (Some(b), Some(first)) = (at(backdrop.into()), at(shadow[0].into())) else {
        return;
    };
    if (first > b) == over {
        return;
    }
    if over {
        let mut below: NodeId = backdrop.into();
        for &sb in shadow {
            scene.place_above(sb, below);
            below = sb.into();
        }
    } else {
        for &sb in shadow {
            scene.place_below(sb, backdrop);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_batch_draws_each_frame_once_at_the_outer_end() {
        use super::{FrameBatch, WindowId};
        let (a, b) = (WindowId::next(), WindowId::next());
        let mut batch = FrameBatch::default();
        assert!(!batch.put_off(a), "outside a batch a frame draws at once");
        assert!(batch.begin());
        assert!(batch.put_off(a) && batch.put_off(b) && batch.put_off(a));
        assert!(
            !batch.begin(),
            "a nested batch leaves the drawing to the outer one"
        );
        assert!(batch.put_off(b));
        assert_eq!(batch.end(), vec![a, b]);
        assert!(!batch.put_off(a), "the batch is closed after its end");
    }

    #[test]
    fn a_frame_never_starts_above_a_top_bars_foot() {
        use super::below_top_bar;
        use reprisekit::taskbar::Edge;
        let out = crate::raster::Rect::new(0, 0, 800, 600);
        let top = crate::barplace::place(out, Edge::Top, 30);
        assert_eq!(below_top_bar(5, &top), 30);
        assert_eq!(below_top_bar(120, &top), 120);
        let bottom = crate::barplace::place(out, Edge::Bottom, 30);
        assert_eq!(below_top_bar(-4, &bottom), -4);
    }

    #[test]
    fn a_rect_keeps_its_size_when_a_top_bar_pushes_it_down() {
        use super::rect_below_top_bar;
        use crate::raster::Rect;
        use reprisekit::taskbar::Edge;
        let out = Rect::new(0, 0, 800, 600);
        let r = Rect::new(40, 5, 300, 200);
        let top = crate::barplace::place(out, Edge::Top, 30);
        assert_eq!(rect_below_top_bar(r, &top), Rect::new(40, 30, 300, 200));
        let low = Rect::new(40, 90, 300, 200);
        assert_eq!(rect_below_top_bar(low, &top), low);
        for edge in [Edge::Bottom, Edge::Left, Edge::Right] {
            let p = crate::barplace::place(out, edge, 30);
            assert_eq!(rect_below_top_bar(r, &p), r, "{edge:?}");
        }
    }

    #[test]
    fn a_frame_may_start_under_an_auto_hidden_top_bars_reveal_line() {
        use super::below_top_bar;
        use reprisekit::taskbar::Edge;
        let out = crate::raster::Rect::new(0, 0, 800, 600);
        for hide in [0.0, 1.0] {
            let top = crate::barplace::place_hiding(out, Edge::Top, 30, Some(hide));
            assert_eq!(
                below_top_bar(0, &top),
                2,
                "only the reveal line, shown or hidden ({hide})"
            );
        }
    }

    #[test]
    fn molten_stacks_the_shadow_over_the_glass() {
        use super::{DeskScene, NodeId, stack_shadow};
        let mut scene = DeskScene::new();
        let chrome = scene.tree_create(scene.root());
        let shadow = std::array::from_fn(|_| scene.buffer_create(chrome));
        let backdrop = scene.tree_create(chrome);
        let frame: [_; 4] = std::array::from_fn(|_| scene.buffer_create(chrome));
        let ids = |shadow_first: bool| -> Vec<NodeId> {
            let s = shadow.iter().map(|&b| NodeId::from(b));
            let f = frame.iter().map(|&b| NodeId::from(b));
            let b = std::iter::once(NodeId::from(backdrop));
            if shadow_first {
                s.chain(b).chain(f).collect()
            } else {
                b.chain(s).chain(f).collect()
            }
        };
        assert_eq!(
            scene.children(chrome),
            ids(true),
            "made with the shadow lowest"
        );
        stack_shadow(&mut scene, &shadow, backdrop, true);
        assert_eq!(
            scene.children(chrome),
            ids(false),
            "Molten: over the backdrop"
        );
        stack_shadow(&mut scene, &shadow, backdrop, true);
        assert_eq!(scene.children(chrome), ids(false), "again: unchanged");
        stack_shadow(&mut scene, &shadow, backdrop, false);
        assert_eq!(scene.children(chrome), ids(true), "Frosted: under it again");
    }

    #[test]
    fn the_shadow_cache_draws_each_key_once() {
        use super::{Rc, ShadowCache, shadow};
        let mut cache = ShadowCache::default();
        let a = cache.pieces(1, true, false);
        assert!(
            Rc::ptr_eq(&a, &cache.pieces(1, true, false)),
            "same key: one drawing"
        );
        let keys = [(1, false, false), (2, true, false), (1, true, true)];
        for (scale, active, square) in keys {
            assert!(
                !Rc::ptr_eq(&a, &cache.pieces(scale, active, square)),
                "{scale} {active} {square}: a drawing of its own"
            );
        }
        assert_eq!(cache.0.len(), 4, "four keys, four drawings");
        let sq = cache.pieces(1, true, true);
        for i in 0..8 {
            assert_eq!(
                a[i].pixels,
                shadow::piece(true, i, 1).pixels,
                "rounded piece {i}"
            );
            assert_eq!(
                sq[i].pixels,
                reprisekit::glass::square::shadow_piece(true, i, 1).pixels,
                "square piece {i}"
            );
        }
        let hi = cache.pieces(2, true, false);
        assert_eq!(
            (hi[0].width, hi[0].height),
            (a[0].width * 2, a[0].height * 2),
            "scale 2 draws at twice the size"
        );
    }

    #[test]
    fn x11_geometry_saturates() {
        assert_eq!(super::x11_geometry(10, -20, 300, 200), (10, -20, 300, 200));
        assert_eq!(
            super::x11_geometry(40_000, -40_000, 0, 70_000),
            (i16::MAX, i16::MIN, 1, u16::MAX)
        );
    }

    #[test]
    fn client_extent_stays_within_1_to_32767() {
        assert_eq!(super::client_extent(640, 480), (640, 480));
        assert_eq!(super::client_extent(0, -5), (1, 1));
        assert_eq!(super::client_extent(i32::MAX, 40_000), (32767, 32767));
    }

    #[test]
    fn client_limits_keep_zero_max_unbounded_and_cap_at_32767() {
        let limits = |min, max| super::SizeLimits { min, max };
        assert_eq!(
            super::client_limits((320, 240, 640, 480)),
            limits((320, 240), (Some(640), Some(480)))
        );
        assert_eq!(
            super::client_limits((0, -1, 0, -5)),
            limits((1, 1), (None, None))
        );
        assert_eq!(
            super::client_limits((i32::MAX, i32::MAX, i32::MAX, 40_000)),
            limits((32767, 32767), (Some(32767), Some(32767)))
        );
    }

    use super::*;
    use crate::raster::Rect;
    use crate::theme::Metrics;

    #[test]
    fn the_frames_glass_leaves_clear_only_the_client_outside_its_blur_region() {
        let client = Rect::new(8, 30, 400, 300);
        assert_eq!(
            client_holes(client, &[]),
            vec![client],
            "no region: the whole client"
        );
        let holes = client_holes(client, &[Rect::new(0, 0, 400, 78)]);
        assert_eq!(holes, vec![Rect::new(8, 108, 400, 222)], "below the band");
        let holes = client_holes(client, &[Rect::new(0, 0, 400, 300)]);
        assert!(holes.is_empty(), "a region over all of it: {holes:?}");
    }

    #[test]
    fn modal_end_follows_the_chain_to_its_last_dialog() {
        let modal = |w: u32| match w {
            0 => Some(1),
            1 => Some(2),
            _ => None,
        };
        assert_eq!(modal_end(0, modal), 2);
        assert_eq!(modal_end(1, modal), 2);
        assert_eq!(modal_end(3, modal), 3);
    }

    #[test]
    fn modal_end_stops_on_a_parent_cycle() {
        let modal = |w: u32| Some(if w == 2 { 1 } else { w + 1 });
        assert_eq!(modal_end(0, modal), 2);
    }

    #[test]
    fn owner_chain_has_every_owner_up_the_chain_and_no_other() {
        let parent = |w: u32| match w {
            2 => Some(1),
            1 => Some(0),
            _ => None,
        };
        assert!(owner_chain_has(2, 1, parent), "expected 1 to own 2");
        assert!(owner_chain_has(2, 0, parent), "expected 0 to own 2");
        assert!(!owner_chain_has(1, 2, parent), "expected 2 not to own 1");
        assert!(!owner_chain_has(2, 3, parent), "expected 3 not to own 2");
        assert!(
            !owner_chain_has(2, 2, parent),
            "expected 2 not to own itself"
        );
    }

    #[test]
    fn owner_chain_has_ends_on_a_parent_cycle() {
        let parent = |w: u32| Some(if w == 2 { 1 } else { w + 1 });
        assert!(!owner_chain_has(0, 7, parent), "expected 7 not to own 0");
    }

    #[test]
    fn walk_owned_visits_owners_first_and_passes_their_state_down() {
        let kids = |w: usize| match w {
            0 => vec![1, 4],
            1 => vec![2, 3],
            _ => vec![],
        };
        let mut seen = Vec::new();
        walk_owned(0, 0, kids, |w, depth| {
            seen.push((w, depth));
            depth + 1
        });
        assert_eq!(seen, [(1, 0), (2, 1), (3, 1), (4, 0)]);
    }

    #[test]
    fn walk_owned_visits_each_window_of_a_parent_cycle_once() {
        let kids = |w: usize| match w {
            0 => vec![1],
            1 => vec![2],
            _ => vec![0, 1],
        };
        let mut seen = Vec::new();
        walk_owned(0, (), kids, |w, ()| seen.push(w));
        assert_eq!(seen, [1, 2]);
    }

    #[test]
    fn walk_owned_follows_a_100000_deep_chain() {
        let depth = 100_000;
        let mut last = 0;
        walk_owned(
            0,
            (),
            |w: usize| if w < depth { vec![w + 1] } else { vec![] },
            |w, ()| last = w,
        );
        assert_eq!(last, depth);
    }

    #[test]
    fn chrome_follows_ownership_and_size_hints() {
        use reprisekit::classic::frame::{ButtonSet, Chrome};
        let none = DialogTag::None;
        assert_eq!(chrome_for(false, true, true, none), Chrome::WINDOW);
        assert_eq!(chrome_for(true, false, true, none), Chrome::DIALOG);
        let c = chrome_for(true, true, true, none);
        assert_eq!(c.buttons, ButtonSet::Close);
        assert!(c.resizable);
        let c = chrome_for(false, false, true, none);
        assert_eq!(c.buttons, ButtonSet::MinMaxClose);
        assert!(!c.resizable);
        assert!(!chrome_for(false, true, false, none).decorated);
    }

    #[test]
    fn a_tagged_dialog_gets_its_caption_owned_or_not() {
        use reprisekit::classic::frame::Chrome;
        let (help, prompt) = (DialogTag::Help, DialogTag::Prompt);
        assert_eq!(chrome_for(false, false, true, help), Chrome::HELP_DIALOG);
        assert_eq!(chrome_for(true, false, true, help), Chrome::HELP_DIALOG);
        assert!(!chrome_for(false, false, false, help).decorated);
        assert_eq!(chrome_for(false, false, true, prompt), Chrome::PROMPT);
        assert_eq!(chrome_for(true, false, true, prompt), Chrome::PROMPT);
        assert_eq!(DialogTag::from_tag("prompt"), prompt);
        assert_eq!(DialogTag::from_tag("help-dialog"), help);
        assert_eq!(DialogTag::from_tag("dialog"), DialogTag::None);
    }

    #[test]
    fn a_fixed_dialog_has_the_close_box_alone() {
        use reprisekit::classic::frame::{ButtonSet, Chrome};
        let fixed = DialogTag::from_tag("fixed-dialog");
        assert_eq!(fixed, DialogTag::Fixed);
        let c = chrome_for(false, false, true, fixed);
        assert_eq!(c, Chrome::DIALOG);
        assert_eq!(c.buttons, ButtonSet::Close);
    }

    #[test]
    fn cascade_steps_and_wraps() {
        let m = Metrics::default();
        let area = Rect::new(100, 50, 800, 600);
        assert_eq!(cascade_position(0, area, &m), (132, 82));
        assert_eq!(cascade_position(1, area, &m), (156, 106));
        assert_eq!(cascade_position(10, area, &m), (132, 82));
        let small = Rect::new(0, 0, 300, 200);
        let (x, y) = cascade_position(9, small, &m);
        assert!(x < 300 && y + m.caption_height + 2 * m.frame_width <= 200);
    }

    #[test]
    fn cascade_uses_configured_metrics() {
        let m = Metrics {
            cascade_origin: 10,
            cascade_step: 5,
            ..Default::default()
        };
        let area = Rect::new(0, 0, 800, 600);
        assert_eq!(cascade_position(0, area, &m), (10, 10));
        assert_eq!(cascade_position(3, area, &m), (25, 25));
        assert_eq!(cascade_position(10, area, &m), (10, 10));
    }

    #[test]
    fn an_apps_next_window_cascades_from_its_newest_and_wraps_at_the_edge() {
        let m = Metrics::default();
        let area = Rect::new(0, 0, 1920, 1050);
        assert_eq!(cascade_from((300, 200), (640, 480), area, &m), (324, 224));
        assert_eq!(
            cascade_from((1270, 200), (640, 480), area, &m),
            (32, 32),
            "past the right edge"
        );
        assert_eq!(
            cascade_from((300, 560), (640, 480), area, &m),
            (32, 32),
            "past the bottom"
        );
    }

    #[test]
    fn a_remembered_frame_is_kept_inside_the_work_area() {
        let area = Rect::new(1920, 0, 1280, 994);
        assert_eq!(keep_inside((2000, 100), (640, 480), area), (2000, 100));
        assert_eq!(keep_inside((3000, 900), (640, 480), area), (2560, 514));
        assert_eq!(keep_inside((-50, -20), (640, 480), area), (1920, 0));
        assert_eq!(keep_inside((2000, 100), (2000, 480), area), (1920, 100));
        assert_eq!(fit_size((2000, 600), area), (1280, 600));
        assert_eq!(fit_size((800, 1200), area), (800, 994));
    }

    #[test]
    fn a_remembered_frame_reopens_at_the_same_size() {
        let m = Metrics::default();
        let screen = Rect::new(0, 0, 1920, 1080);
        let area = Rect::new(0, 0, 1920, 1050);
        let want = crate::windowrules::Remembered {
            output: "Virtual-1".into(),
            x: 32,
            y: 32,
            width: 831,
            height: 678,
            maximized: false,
            text_size: 100,
        }
        .want(true, 100);
        for decorated in [false, true] {
            let chrome = chrome_for(false, true, decorated, DialogTag::None);
            let (w, h) = first_configure_size(&m, &want, OPEN, screen, area, chrome)
                .expect("a remembered frame asks for a size");
            assert_eq!(
                FrameGeometry::layout(&m, w, h, false, chrome).total,
                (831, 678),
                "decorated: {decorated}"
            );
        }
        let csd = chrome_for(false, true, false, DialogTag::None);
        assert_eq!(
            first_configure_size(&m, &want, OPEN, screen, area, csd),
            Some((831, 678))
        );
    }

    const OPEN: SizeLimits = SizeLimits {
        min: (1, 1),
        max: (None, None),
    };

    #[test]
    fn a_remembered_frame_under_the_minimum_reopens_at_the_minimum() {
        let m = Metrics::default();
        let screen = Rect::new(0, 0, 1920, 1080);
        let area = Rect::new(0, 0, 1920, 1050);
        let remembered = |width, height| {
            crate::windowrules::Remembered {
                output: "Virtual-1".into(),
                x: 32,
                y: 32,
                width,
                height,
                maximized: false,
                text_size: 100,
            }
            .want(true, 100)
        };
        let limits = SizeLimits {
            min: (544, 528),
            max: (None, None),
        };
        let csd = chrome_for(false, true, false, DialogTag::None);
        let size = |want| first_configure_size(&m, &want, limits, screen, area, csd);
        assert_eq!(size(remembered(760, 500)), Some((760, 528)));
        assert_eq!(size(remembered(400, 300)), Some((544, 528)));
        assert_eq!(size(remembered(2400, 1400)), Some((1920, 1050)));
        let tall = SizeLimits {
            min: (1100, 1300),
            max: (None, None),
        };
        assert_eq!(
            first_configure_size(&m, &remembered(760, 500), tall, screen, area, csd),
            Some((1100, 1050)),
            "a minimum past the work area opens as tall as the area"
        );
        let decorated = chrome_for(false, true, true, DialogTag::None);
        assert_eq!(
            first_configure_size(&m, &remembered(760, 500), limits, screen, area, decorated)
                .map(|s| s.1),
            Some(528),
            "the minimum is the client's, inside the frame"
        );
    }

    /// A window growing with the text size past the taskbar moves up as far as it overran; one
    /// the user left partly off screen, or one that shrank, stays put.
    #[test]
    fn a_window_that_grew_past_the_work_area_moves_back_in() {
        let area = Rect::new(0, 0, 1920, 1050);
        let was = Rect::new(640, 160, 620, 700);
        let now = Rect::new(640, 160, 820, 960);
        assert_eq!(grown_back_inside(was, now, area), Some((640, 90)));
        let fits = Rect::new(640, 160, 700, 800);
        assert_eq!(grown_back_inside(was, fits, area), None);
        let off = Rect::new(1500, 160, 620, 700);
        assert_eq!(
            grown_back_inside(off, Rect { width: 820, ..off }, area),
            None
        );
        assert_eq!(grown_back_inside(now, was, area), None);
    }

    #[test]
    fn a_window_on_screen_comes_back_inside_only_after_its_frame_or_the_taskbar_grows() {
        let screen = Rect::new(0, 0, 1920, 1080);
        let old = Rect::new(0, 0, 1920, 1050);
        let area = Rect::new(0, 0, 1920, 1027);
        let was = Rect::new(312, 567, 375, 460);
        let grown = Rect::new(312, 567, 735, 920);
        assert_eq!(
            retheme_back_inside(was, grown, screen, area, area),
            Some((312, 107)),
            "the grown sheet ends on the taskbar's top"
        );
        let low = Rect::new(156, 557, 688, 480);
        assert_eq!(
            retheme_back_inside(low, low, screen, old, area),
            Some((156, 547)),
            "a window the taller taskbar now covers moves up"
        );
        assert_eq!(
            retheme_back_inside(low, low, screen, area, area),
            None,
            "one left over an unchanged taskbar stays"
        );
        assert_eq!(retheme_back_inside(was, was, screen, area, area), None);
        let off = Rect::new(1500, 567, 620, 460);
        assert_eq!(
            retheme_back_inside(off, Rect { height: 920, ..off }, screen, area, area),
            None,
            "one left partly off screen stays"
        );
        let tall = Rect::new(0, 100, 600, 1200);
        assert_eq!(
            retheme_back_inside(Rect::new(0, 100, 600, 500), tall, screen, area, area),
            Some((0, 0)),
            "one taller than the work area keeps its caption on screen"
        );
    }

    #[test]
    fn a_dialog_centres_on_its_owner() {
        let parent = Rect::new(100, 100, 400, 300);
        let area = Rect::new(0, 0, 1920, 1030);
        assert_eq!(centre_on(parent, (200, 100), area), (200, 200));
    }

    #[test]
    fn an_ownerless_dialog_centres_on_the_usable_area() {
        let area = Rect::new(0, 50, 1920, 1000);
        assert_eq!(centre_on(area, (378, 201), area), (771, 449));
    }

    #[test]
    fn a_dialog_is_pushed_back_inside_the_usable_area() {
        let area = Rect::new(0, 0, 1920, 1030);
        let parent = Rect::new(1800, 980, 120, 50);
        let (x, y) = centre_on(parent, (400, 200), area);
        assert!(x + 400 <= area.right(), "x {x} runs off the right");
        assert!(y + 200 <= area.bottom(), "y {y} runs off the bottom");
        let parent = Rect::new(-50, -40, 100, 80);
        let (x, y) = centre_on(parent, (400, 200), area);
        assert_eq!((x, y), (area.x, area.y));
    }

    #[test]
    fn a_dialog_larger_than_the_screen_starts_at_the_corner() {
        let area = Rect::new(0, 0, 300, 200);
        let parent = Rect::new(0, 0, 300, 200);
        assert_eq!(centre_on(parent, (800, 600), area), (area.x, area.y));
    }

    #[test]
    fn an_anchor_waits_for_the_last_size_asked_for() {
        let (s5, s7) = (Some(Serial::from(5)), Some(Serial::from(7)));
        assert!(
            !anchor_done(s7, s5, true),
            "an older size landing keeps the anchor for the newer one"
        );
        assert!(anchor_done(s7, s7, true), "the last size landed");
        assert!(anchor_done(s7, Some(Serial::from(8)), false));
        assert!(!anchor_done(s7, None, true), "nothing acked yet");
        assert!(anchor_done(None, None, true), "an X11 window's new size");
        assert!(!anchor_done(None, None, false));
    }

    #[test]
    fn anchored_position_pins_the_opposite_edge() {
        let right_bottom = (300, 200);
        let a = ResizeAnchor {
            edges: Edges::LEFT,
            right: right_bottom.0,
            bottom: right_bottom.1,
        };
        assert_eq!(anchored_position(&a, (120, 80), (50, 60)), (180, 60));
        let a = ResizeAnchor {
            edges: Edges::TOP,
            right: right_bottom.0,
            bottom: right_bottom.1,
        };
        assert_eq!(anchored_position(&a, (120, 80), (50, 60)), (50, 120));
        let a = ResizeAnchor {
            edges: Edges::LEFT | Edges::TOP,
            right: right_bottom.0,
            bottom: right_bottom.1,
        };
        assert_eq!(anchored_position(&a, (120, 80), (50, 60)), (180, 120));
        let a = ResizeAnchor {
            edges: Edges::RIGHT,
            right: right_bottom.0,
            bottom: right_bottom.1,
        };
        assert_eq!(anchored_position(&a, (120, 80), (50, 60)), (50, 60));
    }

    /// The layout rect of a 100x60 menu, and the usable area, once a toplevel asks for it to drop
    /// from its client's bottom-left corner (or to open leftwards from its top-left one when
    /// `leftwards`) with the taskbar on `edge` and the client 40 (80) px from the usable area's
    /// edge: far enough that the menu would fit on the output over the bar.
    fn menu_by_the_bar(
        edge: reprisekit::taskbar::Edge,
        leftwards: bool,
    ) -> (crate::raster::Rect, crate::raster::Rect) {
        use crate::raster::Rect;
        use smithay::desktop::{PopupKind, PopupManager};
        use wayland_protocols::xdg::shell::client::xdg_positioner::{
            Anchor, ConstraintAdjustment, Gravity,
        };
        let mut config = crate::config::Config::default();
        config.taskbar.position = edge;
        let mut h = crate::harness::Harness::with_config(config);
        let out = h.output("TEST-1", 1024, 768);
        let mut c = h.connect();
        let t = h.map_toplevel(&mut c, "test.app", 300, 200);
        let area = h.state.usable_area(Some(&out));
        let win = h.state.win(t.id).unwrap();
        let (cx, cy) = win.client_origin();
        let (fx, fy) = (win.x, win.y);
        let (x, y) = if leftwards {
            (area.x + 80 + fx - cx, fy)
        } else {
            (fx, area.y + area.height - 240 + fy - cy)
        };
        h.state.move_to(t.id, x, y);
        h.dispatch();
        h.map_popup_with(&mut c, t, 100, 60, |p| {
            let adjust = ConstraintAdjustment::FlipX
                | ConstraintAdjustment::FlipY
                | ConstraintAdjustment::SlideX
                | ConstraintAdjustment::SlideY;
            if leftwards {
                p.set_anchor_rect(0, 0, 1, 20);
                p.set_anchor(Anchor::TopLeft);
                p.set_gravity(Gravity::BottomLeft);
            } else {
                p.set_anchor_rect(0, 180, 40, 20);
                p.set_anchor(Anchor::BottomLeft);
                p.set_gravity(Gravity::BottomRight);
            }
            p.set_constraint_adjustment(adjust);
        });
        let win = h.state.win(t.id).unwrap();
        let (cx, cy) = win.client_origin();
        let open: Vec<_> = PopupManager::popups_for_surface(win.wl_surface()).collect();
        assert_eq!(open.len(), 1, "the menu is open");
        let PopupKind::Xdg(popup) = &open[0].0 else {
            panic!("an xdg popup");
        };
        let g = popup.with_pending_state(|s| s.geometry);
        let menu = Rect::new(cx + g.loc.x, cy + g.loc.y, g.size.w, g.size.h);
        (menu, area)
    }

    #[test]
    fn a_menu_dropping_onto_the_bottom_taskbar_opens_above_its_anchor() {
        let (menu, area) = menu_by_the_bar(reprisekit::taskbar::Edge::Bottom, false);
        assert!(area.height < 768, "the taskbar takes a strip: {area:?}");
        assert_eq!(
            menu.y + menu.height,
            area.y + area.height - 60,
            "the menu flips up to end at its anchor's top: {menu:?} in {area:?}"
        );
    }

    #[test]
    fn a_menu_opening_onto_a_left_taskbar_stays_right_of_it() {
        let (menu, area) = menu_by_the_bar(reprisekit::taskbar::Edge::Left, true);
        assert!(area.x >= 20, "the taskbar takes a strip: {area:?}");
        assert!(
            menu.x >= area.x,
            "the menu starts right of the bar at x >= {}: {menu:?}",
            area.x
        );
    }

    /// Two mapped toplevels on one output with the desktop holding the keyboard; the second's
    /// window marked override-redirect when `menu`.
    fn behind_the_desktop(
        menu: bool,
    ) -> (
        crate::harness::Harness,
        crate::harness::TestClient,
        crate::harness::Toplevel,
        crate::harness::Toplevel,
    ) {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        let mut c = h.connect();
        let a = h.map_toplevel(&mut c, "test.a", 300, 200);
        let b = h.map_toplevel(&mut c, "test.b", 100, 80);
        h.state.focus_desktop();
        assert_eq!(h.state.focused, None);
        if let Some(w) = h.state.win_mut(b.id) {
            w.override_redirect = menu;
        }
        (h, c, a, b)
    }

    #[test]
    fn a_closing_override_redirect_window_leaves_the_focus_where_it_is() {
        let (mut h, mut c, a, b) = behind_the_desktop(true);
        c.destroy(b);
        h.roundtrip(&mut c);
        assert!(h.state.win(b.id).is_none(), "the window went");
        assert_eq!(h.state.focused, None, "the desktop keeps the keyboard");
        assert!(h.state.win(a.id).is_some());
    }

    #[test]
    fn a_closing_window_hands_the_focus_to_the_top_one() {
        let (mut h, mut c, a, b) = behind_the_desktop(false);
        c.destroy(b);
        h.roundtrip(&mut c);
        assert!(h.state.win(b.id).is_none());
        assert_eq!(h.state.focused, Some(a.id));
    }

    #[test]
    fn a_window_moved_to_an_output_with_another_work_area_hears_its_bounds() {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        h.output("TEST-2", 1920, 1080);
        let mut c = h.connect();
        let t = h.map_toplevel(&mut c, "test.app", 300, 200);
        h.roundtrip(&mut c);
        let first = c.side.shells[t.shell].bounds;
        assert!(
            first.0 > 0 && first.0 < 1024,
            "bounds on the first output: {first:?}"
        );
        let two = h.state.scene.outputs()[1].clone();
        let b = h.state.layout_box(Some(&two));
        h.state.move_to(t.id, b.x + 100, b.y + 100);
        h.roundtrip(&mut c);
        let moved = c.side.shells[t.shell].bounds;
        assert!(
            moved.0 > 1024 && moved.1 > first.1,
            "bounds on the larger output: {moved:?}"
        );
        h.state.move_to(t.id, 100, 100);
        h.roundtrip(&mut c);
        assert_eq!(
            c.side.shells[t.shell].bounds, first,
            "and back on the first"
        );
    }

    #[test]
    fn a_window_hears_new_bounds_when_the_taskbar_moves_or_hides() {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        let mut c = h.connect();
        let t = h.map_toplevel(&mut c, "test.app", 300, 200);
        h.roundtrip(&mut c);
        let bottom = c.side.shells[t.shell].bounds;
        let mut cfg = h.state.config.clone();
        cfg.taskbar.position = reprisekit::taskbar::Edge::Left;
        crate::configwatch::reload(&mut h.state, cfg);
        h.roundtrip(&mut c);
        let left = c.side.shells[t.shell].bounds;
        assert!(
            left.0 < bottom.0 && left.1 > bottom.1,
            "a side bar takes width and gives height back: {bottom:?} -> {left:?}"
        );
        let mut cfg = h.state.config.clone();
        cfg.taskbar.auto_hide = true;
        crate::configwatch::reload(&mut h.state, cfg);
        h.roundtrip(&mut c);
        let hidden = c.side.shells[t.shell].bounds;
        assert!(
            hidden.0 > left.0 && hidden.1 == left.1,
            "a hiding bar gives its width back: {left:?} -> {hidden:?}"
        );
    }

    #[test]
    fn a_window_hears_new_bounds_when_its_output_changes_mode_or_scale() {
        use smithay::output::{Mode, Scale};
        let mut h = crate::harness::Harness::new();
        let o = h.output("TEST-1", 1024, 768);
        let mut c = h.connect();
        let t = h.map_toplevel(&mut c, "test.app", 300, 200);
        h.roundtrip(&mut c);
        let first = c.side.shells[t.shell].bounds;
        let mode = Mode {
            size: (1920, 1080).into(),
            refresh: 60_000,
        };
        o.change_current_state(Some(mode), None, None, None);
        h.state.update_layout();
        h.roundtrip(&mut c);
        let wide = c.side.shells[t.shell].bounds;
        assert!(
            wide.0 > first.0 && wide.1 > first.1,
            "a larger mode gives larger bounds: {first:?} -> {wide:?}"
        );
        o.change_current_state(None, None, Some(Scale::Fractional(2.0)), None);
        h.state.update_layout();
        h.roundtrip(&mut c);
        let scaled = c.side.shells[t.shell].bounds;
        assert!(
            scaled.0 < first.0 && scaled.1 < first.1,
            "scale 2 halves the work area: {wide:?} -> {scaled:?}"
        );
    }

    #[test]
    fn a_window_that_sized_itself_keeps_that_size_when_restored() {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        let mut c = h.connect();
        let t = h.map_toplevel(&mut c, "test.app", 300, 200);
        h.state.request_size(t.id, 400, 300);
        h.roundtrip(&mut c);
        assert_eq!(c.side.shells[t.shell].size, (400, 300), "the asked size");
        h.resize_self(&mut c, t, 350, 250);
        h.state.set_minimized(t.id, true);
        h.roundtrip(&mut c);
        h.state.set_minimized(t.id, false);
        h.roundtrip(&mut c);
        assert_eq!(
            c.side.shells[t.shell].size,
            (350, 250),
            "minimize and restore keep the size the program took"
        );
    }
}
