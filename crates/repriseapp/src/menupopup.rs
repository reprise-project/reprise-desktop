//! The edit menu on xdg_popups, so it shows whole past a short window's edges: one popup per
//! [`PopupMenu`] level, placed by the compositor through an xdg_positioner and handed back to the
//! chain in the window's coordinates once configured. Only the root takes a grab; submenus are
//! plain children of it, so the keyboard stays on the root.

use reprisekit::controls::menu::{PopupMenu, level_shadow};
use reprisekit::raster::{Canvas, Rect};
use smithay_client_toolkit::compositor::Region;
use smithay_client_toolkit::reexports::protocols::xdg::shell::client::xdg_positioner::{
    Anchor, ConstraintAdjustment, Gravity,
};
use smithay_client_toolkit::shell::xdg::popup::{Popup, PopupConfigure, PopupHandler};
use smithay_client_toolkit::shell::xdg::{XdgPositioner, XdgSurface};
use smithay_client_toolkit::shm::slot::{Buffer, SlotPool};
use wayland_client::protocol::{wl_shm, wl_surface};
use wayland_client::{Connection, QueueHandle};

use crate::{Host, Shell, WinId, copy_pixels};

/// The popups of the open menu, one per chain level, root first.
pub(crate) struct MenuPopups {
    owner: WinId,
    levels: Vec<LevelSurface>,
}

struct LevelSurface {
    popup: Popup,
    /// The chain level's path this popup shows.
    path: Vec<usize>,
    /// Where the compositor put it, from its parent's corner; none until configured.
    at: Option<(i32, i32)>,
    /// Field order is drop order: the buffer before its pool.
    buffer: Option<Buffer>,
    pool: Option<SlotPool>,
    drawn: Option<Drawn>,
}

/// What a level's last buffer showed: it draws again when any of it changes.
#[derive(Clone, PartialEq)]
struct Drawn {
    level: reprisekit::controls::menu::Level,
    underline: bool,
    scale: i32,
}

/// How a level's popup asks to be placed.
#[derive(Debug, PartialEq)]
pub(crate) struct Placement {
    pub size: (i32, i32),
    /// In the parent's coordinates: the window's for the root, the parent level's for a submenu.
    pub anchor_rect: Rect,
    pub anchor: Anchor,
    pub gravity: Gravity,
    pub offset: (i32, i32),
    pub adjust: ConstraintAdjustment,
}

/// Level `i` of `chain`, the root opened at `at`: the root grows right and down from its point
/// and flips or slides to stay on screen; a submenu opens right of its row, flipping left.
pub(crate) fn placement(chain: &PopupMenu, i: usize, at: (i32, i32)) -> Option<Placement> {
    let r = chain.levels().get(i)?.rect();
    let size = (r.width.max(1), r.height.max(1));
    if i == 0 {
        return Some(Placement {
            size,
            anchor_rect: Rect::new(at.0, at.1, 1, 1),
            anchor: Anchor::TopLeft,
            gravity: Gravity::BottomRight,
            offset: (0, 0),
            adjust: ConstraintAdjustment::FlipX
                | ConstraintAdjustment::FlipY
                | ConstraintAdjustment::SlideX
                | ConstraintAdjustment::SlideY,
        });
    }
    let (anchor_rect, dy) = chain.submenu_anchor(i)?;
    Some(Placement {
        size,
        anchor_rect,
        anchor: Anchor::TopRight,
        gravity: Gravity::BottomRight,
        offset: (0, dy),
        adjust: ConstraintAdjustment::FlipX
            | ConstraintAdjustment::SlideX
            | ConstraintAdjustment::SlideY,
    })
}

/// Each level's corner in the window's coordinates from where each popup was put against its
/// parent, up to the first one not configured yet.
pub(crate) fn origins(at: &[Option<(i32, i32)>]) -> Vec<(i32, i32)> {
    let mut out = Vec::new();
    let mut corner = (0, 0);
    for p in at {
        let Some((x, y)) = p else { break };
        corner = (corner.0 + x, corner.1 + y);
        out.push(corner);
    }
    out
}

/// How many of the popups showing `have` still show the chain's levels `want`.
pub(crate) fn kept(have: &[&[usize]], want: &[&[usize]]) -> usize {
    have.iter().zip(want).take_while(|(a, b)| a == b).count()
}

impl MenuPopups {
    /// The level showing on `surface`, and its corner in the owner's coordinates.
    fn level_on(&self, surface: &wl_surface::WlSurface) -> Option<(usize, (i32, i32))> {
        let i = self
            .levels
            .iter()
            .position(|l| l.popup.wl_surface() == surface)?;
        let at: Vec<_> = self.levels.iter().map(|l| l.at).collect();
        origins(&at).get(i).map(|o| (i, *o))
    }

    fn owns(&self, surface: &wl_surface::WlSurface) -> bool {
        self.levels.iter().any(|l| l.popup.wl_surface() == surface)
    }

    /// Destroys the levels from `n` up, topmost first as the protocol asks.
    fn truncate(&mut self, n: usize) {
        while self.levels.len() > n {
            self.levels.pop();
        }
    }
}

impl Drop for MenuPopups {
    fn drop(&mut self) {
        self.truncate(0);
    }
}

impl Host {
    /// The window a menu popup's `surface` belongs to.
    pub(crate) fn menu_owner(&self, surface: &wl_surface::WlSurface) -> Option<WinId> {
        self.menu_popups
            .as_ref()
            .filter(|m| m.owns(surface))
            .map(|m| m.owner)
    }

    /// A point on a menu popup's `surface` in its owner's coordinates.
    pub(crate) fn menu_point(
        &self,
        surface: &wl_surface::WlSurface,
        (x, y): (i32, i32),
    ) -> Option<(WinId, (i32, i32))> {
        let m = self.menu_popups.as_ref()?;
        let (_, o) = m.level_on(surface)?;
        Some((m.owner, (o.0 + x, o.1 + y)))
    }

    /// Closes the popups of a menu whose window `live` says is gone, before the window goes: a
    /// popup must not outlive its parent.
    pub(crate) fn drop_menus(&mut self, live: impl Fn(WinId) -> bool) {
        if self.menu_popups.as_ref().is_some_and(|m| !live(m.owner)) {
            self.menu_popups = None;
        }
        self.edit_menu.retain(live);
    }

    /// Brings the popups in line with the edit menu: a popup per level, each placed and drawn.
    pub(crate) fn sync_menu(&mut self) {
        let Some((owner, menu, at)) = self.edit_menu.shown() else {
            self.menu_popups = None;
            return;
        };
        let chain = menu.chain().clone();
        let ready = self
            .slots
            .iter()
            .any(|s| s.id == owner && s.mapped && s.configured);
        if !ready {
            return;
        }
        if self.menu_popups.as_ref().is_some_and(|m| m.owner != owner) {
            self.menu_popups = None;
        }
        let popups = self.menu_popups.get_or_insert_with(|| MenuPopups {
            owner,
            levels: Vec::new(),
        });
        let have: Vec<&[usize]> = popups.levels.iter().map(|l| l.path.as_slice()).collect();
        let want: Vec<&[usize]> = chain.levels().iter().map(|l| l.path.as_slice()).collect();
        let keep = kept(&have, &want);
        popups.truncate(keep);
        let at_list: Vec<_> = popups.levels.iter().map(|l| l.at).collect();
        for (i, o) in origins(&at_list).into_iter().enumerate() {
            self.edit_menu.place(i, o);
        }
        for i in 0..chain.depth() {
            let n = self.menu_popups.as_ref().map_or(0, |m| m.levels.len());
            if i < n {
                continue;
            }
            // A submenu waits for its parent to show: it is placed against the parent's corner.
            let parent_shown = i == 0
                || self
                    .menu_popups
                    .as_ref()
                    .is_some_and(|m| m.levels[i - 1].buffer.is_some());
            if !parent_shown || !self.open_level(&chain, i, at) {
                break;
            }
        }
        self.draw_menu();
        self.publish(owner);
    }

    /// Opens level `i`'s popup; `false` when it could not be made.
    fn open_level(&mut self, chain: &PopupMenu, i: usize, at: (i32, i32)) -> bool {
        let Some(p) = placement(chain, i, at) else {
            return false;
        };
        let Some(owner) = self.menu_popups.as_ref().map(|m| m.owner) else {
            return false;
        };
        let positioner = match XdgPositioner::new(&self.xdg_shell) {
            Ok(p) => p,
            Err(e) => {
                log::warn!("menu positioner: {e}");
                return false;
            }
        };
        positioner.set_size(p.size.0, p.size.1);
        let a = p.anchor_rect;
        positioner.set_anchor_rect(a.x, a.y, a.width, a.height);
        positioner.set_anchor(p.anchor);
        positioner.set_gravity(p.gravity);
        positioner.set_offset(p.offset.0, p.offset.1);
        positioner.set_constraint_adjustment(p.adjust);
        let surface = self.compositor.create_surface(&self.qh);
        let parent = match i {
            0 => self
                .slots
                .iter()
                .find(|s| s.id == owner)
                .and_then(|s| match &s.window {
                    Shell::Toplevel(w) => Some(w.xdg_surface().clone()),
                    Shell::Layer(_) => None,
                }),
            _ => self
                .menu_popups
                .as_ref()
                .map(|m| m.levels[i - 1].popup.xdg_surface().clone()),
        };
        let popup = match Popup::from_surface(
            parent.as_ref(),
            &positioner,
            &self.qh,
            surface,
            &self.xdg_shell,
        ) {
            Ok(p) => p,
            Err(e) => {
                log::warn!("menu popup: {e}");
                return false;
            }
        };
        if parent.is_none()
            && let Some(Shell::Layer(l)) =
                self.slots.iter().find(|s| s.id == owner).map(|s| &s.window)
        {
            l.get_popup(popup.xdg_popup());
        }
        popup
            .xdg_surface()
            .set_window_geometry(0, 0, p.size.0, p.size.1);
        // The shadow past the menu's edges lets presses through.
        if let Ok(region) = Region::new(&self.compositor) {
            region.add(0, 0, p.size.0, p.size.1);
            popup
                .wl_surface()
                .set_input_region(Some(region.wl_region()));
        }
        if i == 0
            && let Some(seat) = &self.seat
        {
            popup.xdg_popup().grab(seat, self.input_serial);
        }
        popup.wl_surface().commit();
        let path = chain.levels()[i].path.clone();
        if let Some(m) = self.menu_popups.as_mut() {
            m.levels.push(LevelSurface {
                popup,
                path,
                at: None,
                buffer: None,
                pool: None,
                drawn: None,
            });
        }
        true
    }

    /// Draws each configured level whose look changed since its last buffer.
    fn draw_menu(&mut self) {
        let Some(owner) = self.menu_popups.as_ref().map(|m| m.owner) else {
            return;
        };
        let Some(menu) = self.edit_menu.menu(owner) else {
            return;
        };
        let chain = menu.chain().clone();
        let Some(slot) = self.slots.iter().find(|s| s.id == owner) else {
            return;
        };
        let scale = slot.scaling.drawn.max(1);
        let underline = slot.cues.keys;
        let shm = &self.shm;
        let ctx = &mut self.ctx;
        let Some(popups) = self.menu_popups.as_mut() else {
            return;
        };
        for (i, ls) in popups.levels.iter_mut().enumerate() {
            let Some(level) = chain.levels().get(i) else {
                break;
            };
            if ls.at.is_none() {
                continue;
            }
            let now = Drawn {
                level: level.clone(),
                underline,
                scale,
            };
            if ls.drawn.as_ref() == Some(&now) {
                continue;
            }
            let r = level.rect();
            let (w, h) = (r.width.max(1), r.height.max(1));
            let canvas = ctx.at_scale(scale, |ctx| {
                let m = level_shadow(&ctx.theme);
                let mut c = Canvas::with_scale((w + m) as u32, (h + m) as u32, scale);
                chain.draw_level(i, (-r.x, -r.y), &mut c, &ctx.theme, &ctx.text, underline);
                c
            });
            let (bw, bh) = (canvas.width as i32 * scale, canvas.height as i32 * scale);
            let stride = bw * 4;
            let len = ((bh * stride) as usize + 63) & !63;
            let pool = match &mut ls.pool {
                Some(p) => p,
                None => match SlotPool::new(len.max(64), shm) {
                    Ok(p) => ls.pool.insert(p),
                    Err(e) => {
                        log::warn!("menu pool: {e}");
                        continue;
                    }
                },
            };
            let (buffer, target) =
                match pool.create_buffer(bw, bh, stride, wl_shm::Format::Argb8888) {
                    Ok(b) => b,
                    Err(e) => {
                        log::warn!("menu buffer: {e}");
                        continue;
                    }
                };
            copy_pixels(target, &canvas.pixels);
            let surface = ls.popup.wl_surface();
            surface.set_buffer_scale(scale);
            surface.damage_buffer(0, 0, bw, bh);
            if buffer.attach_to(surface).is_err() {
                continue;
            }
            surface.commit();
            ls.buffer = Some(buffer);
            ls.drawn = Some(now);
        }
    }
}

impl Host {
    /// The look changed: every level draws again.
    pub(crate) fn menu_relooked(&mut self) {
        for l in self
            .menu_popups
            .iter_mut()
            .flat_map(|m| m.levels.iter_mut())
        {
            l.drawn = None;
        }
    }
}

impl PopupHandler for Host {
    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        popup: &Popup,
        config: PopupConfigure,
    ) {
        let Some(m) = self.menu_popups.as_mut() else {
            return;
        };
        let Some(ls) = m.levels.iter_mut().find(|l| &l.popup == popup) else {
            return;
        };
        ls.at = Some(config.position);
        // A moved level draws again; the reader's tree learns where it went.
        ls.drawn = None;
        let owner = m.owner;
        if let Some(s) = self.slot_mut(owner) {
            s.access.state.dirty = true;
        }
    }

    fn done(&mut self, _: &Connection, _: &QueueHandle<Self>, popup: &Popup) {
        if self
            .menu_popups
            .as_ref()
            .is_some_and(|m| m.levels.iter().any(|l| &l.popup == popup))
        {
            self.menu_popups = None;
            self.edit_menu.dismiss();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reprisekit::controls::menu::{MenuCtx, MenuItem, MenuKey, MenuTree};
    use reprisekit::text::TextRenderer;
    use reprisekit::theme::{FontConfig, Theme};

    fn tree() -> MenuTree {
        let mut t = MenuTree::new();
        t.push(MenuItem::item("&Undo"));
        t.push_sub(
            MenuItem::item("&Arrange"),
            MenuTree::flat(vec![MenuItem::item("&Name"), MenuItem::item("&Size")]),
        );
        t
    }

    fn chain(t: &MenuTree, theme: &Theme, text: &TextRenderer) -> PopupMenu {
        let cx = MenuCtx {
            text,
            theme,
            px: theme.fonts.size_px,
            within: Rect::new(0, 0, 1920, 1080),
        };
        let mut p = PopupMenu::open(t, (40, 30), &cx, true);
        p.key(t, MenuKey::Down, &cx);
        p.key(t, MenuKey::Right, &cx);
        p
    }

    #[test]
    fn the_root_grows_from_its_point_and_flips_or_slides_on_both_axes() {
        let (theme, text) = (Theme::default(), TextRenderer::load(&FontConfig::default()));
        let t = tree();
        let p = chain(&t, &theme, &text);
        let root = placement(&p, 0, (40, 30)).expect("the root's placement");
        let r = p.levels()[0].rect();
        assert_eq!(root.size, (r.width, r.height));
        assert_eq!(root.anchor_rect, Rect::new(40, 30, 1, 1));
        assert_eq!(
            (root.anchor, root.gravity),
            (Anchor::TopLeft, Gravity::BottomRight)
        );
        assert_eq!(
            root.adjust,
            ConstraintAdjustment::FlipX
                | ConstraintAdjustment::FlipY
                | ConstraintAdjustment::SlideX
                | ConstraintAdjustment::SlideY
        );
    }

    #[test]
    fn a_submenu_opens_off_its_row_and_flips_only_sideways() {
        let (theme, text) = (Theme::default(), TextRenderer::load(&FontConfig::default()));
        let t = tree();
        let p = chain(&t, &theme, &text);
        let sub = placement(&p, 1, (40, 30)).expect("the submenu's placement");
        let (rect, dy) = p.submenu_anchor(1).unwrap();
        assert_eq!((sub.anchor_rect, sub.offset), (rect, (0, dy)));
        assert_eq!(
            (sub.anchor, sub.gravity),
            (Anchor::TopRight, Gravity::BottomRight)
        );
        assert!(!sub.adjust.contains(ConstraintAdjustment::FlipY));
        assert!(sub.adjust.contains(ConstraintAdjustment::FlipX));
        assert_eq!(placement(&p, 2, (40, 30)), None, "no third level");
    }

    #[test]
    fn each_level_sits_at_the_sum_of_the_positions_up_to_it() {
        assert_eq!(
            origins(&[Some((40, -200)), Some((150, 17)), None, Some((1, 1))]),
            [(40, -200), (190, -183)],
            "levels past an unconfigured one have no corner yet"
        );
        assert_eq!(origins(&[None, Some((1, 1))]), []);
    }

    #[test]
    fn popups_showing_the_chains_levels_are_kept_and_the_rest_go() {
        let (root, a, b): (&[usize], &[usize], &[usize]) = (&[], &[1], &[2]);
        assert_eq!(kept(&[root, a], &[root, a]), 2);
        assert_eq!(kept(&[root, a], &[root, b]), 1, "another submenu opened");
        assert_eq!(kept(&[root, a], &[root]), 1, "the submenu closed");
        assert_eq!(kept(&[root], &[root, a]), 1, "a submenu to add");
        assert_eq!(kept(&[], &[root]), 0);
    }
}
