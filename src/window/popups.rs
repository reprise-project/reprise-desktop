use smithay::{
    desktop::{PopupKind, PopupManager, find_popup_root_surface, get_popup_toplevel_coords},
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point, Rectangle},
    wayland::shell::xdg::PopupSurface,
};

use crate::compositor::State;
use crate::raster::Rect;

use super::WindowId;

impl State {
    pub(super) fn scene_surface_origin(&self, w: WindowId) -> (i32, i32) {
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
}
