//! Menus and drop lists on xdg_popups, so they show whole past a short window's edges: the edit
//! menu and whatever a program's [`App::popup`] has open ([`Hosted`]). One popup per level,
//! placed by the compositor through an xdg_positioner and handed back to the model in the
//! window's coordinates once configured; input on a popup reaches the owner as the window's own.
//! Only the root takes a grab; submenus are plain children of it, so the keyboard stays on the
//! root.

use reprisekit::controls::hosted::{Anchor as Opens, Hosted, Spot};
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

use crate::editmenu::EditMenu;
use crate::{App, Ctx, Host, Shell, Slot, WinId, copy_pixels};

/// The popups of the open menu, one per level, root first.
pub(crate) struct MenuPopups {
    owner: WinId,
    levels: Vec<LevelSurface>,
}

struct LevelSurface {
    popup: Popup,
    /// The level as it was when its popup was made.
    spot: Spot,
    /// Where the compositor put it, from its parent's corner; none until configured.
    at: Option<(i32, i32)>,
    /// Field order is drop order: the buffer before its pool.
    buffer: Option<Buffer>,
    pool: Option<SlotPool>,
    /// The buffer shows the model as it is.
    fresh: bool,
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

/// How `spot`'s popup asks to be placed: a root off a point grows right and down and flips or
/// slides on both axes; one under a rect flips above it; a submenu opens right of its row,
/// flipping left.
pub(crate) fn placement(spot: &Spot) -> Placement {
    let r = spot.rect;
    let size = (r.width.max(1), r.height.max(1));
    let slide = ConstraintAdjustment::SlideX | ConstraintAdjustment::SlideY;
    match spot.anchor {
        Opens::Point(x, y) => Placement {
            size,
            anchor_rect: Rect::new(x, y, 1, 1),
            anchor: Anchor::TopLeft,
            gravity: Gravity::BottomRight,
            offset: (0, 0),
            adjust: ConstraintAdjustment::FlipX | ConstraintAdjustment::FlipY | slide,
        },
        Opens::Below(rect) => Placement {
            size,
            anchor_rect: Rect::new(rect.x, rect.y, rect.width.max(1), rect.height.max(1)),
            anchor: Anchor::BottomLeft,
            gravity: Gravity::BottomRight,
            offset: (0, 0),
            adjust: ConstraintAdjustment::FlipY | slide,
        },
        Opens::Beside { rect, dy } => Placement {
            size,
            anchor_rect: rect,
            anchor: Anchor::TopRight,
            gravity: Gravity::BottomRight,
            offset: (0, dy),
            adjust: ConstraintAdjustment::FlipX | slide,
        },
    }
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

/// How many popups made for `have` still fit the levels `want`: same anchor, same size. A
/// placed level moves, so its corner does not count.
pub(crate) fn kept(have: &[Spot], want: &[Spot]) -> usize {
    let same = |a: &Spot, b: &Spot| {
        a.anchor == b.anchor && (a.rect.width, a.rect.height) == (b.rect.width, b.rect.height)
    };
    have.iter()
        .zip(want)
        .take_while(|(a, b)| same(a, b))
        .count()
}

/// The menu on popups: the edit menu over a field of `ids`, else the first of `ids` whose
/// program has one open.
fn hosted<'a>(
    edit: &'a mut EditMenu,
    app: &'a mut dyn App,
    ctx: &mut Ctx,
    slots: &[Slot],
    ids: &[WinId],
) -> Option<(WinId, Box<dyn Hosted + 'a>)> {
    if edit.hosted().is_some_and(|(w, _)| ids.contains(&w)) {
        return edit.hosted();
    }
    let mut ask = |app: &mut dyn App, id: WinId| {
        if let Some(s) = slots.iter().find(|s| s.id == id) {
            ctx.aim(s);
        }
        app.popup(id, ctx).is_some_and(|h| !h.spots().is_empty())
    };
    let id = ids.iter().copied().find(|&id| ask(app, id))?;
    app.popup(id, ctx).map(|h| (id, h))
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

    fn spots(&self) -> Vec<Spot> {
        self.levels.iter().map(|l| l.spot).collect()
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

    /// Whether window `id`'s menu shows on popups.
    pub(crate) fn has_menu(&self, id: WinId) -> bool {
        self.menu_popups.as_ref().is_some_and(|m| m.owner == id)
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

    /// The menu's popups draw again: what they show changed.
    pub(crate) fn menu_changed(&mut self, owner: WinId) {
        if let Some(m) = self.menu_popups.as_mut().filter(|m| m.owner == owner) {
            for l in &mut m.levels {
                l.fresh = false;
            }
        }
    }

    /// The open menu's owner and levels, after the model hears that it is hosted and where its
    /// configured levels are. Before each window draws, so the draw leaves the menu out and
    /// lays it out where it shows.
    pub(crate) fn place_menu(&mut self) -> Option<(WinId, Vec<Spot>)> {
        let ids: Vec<WinId> = self
            .slots
            .iter()
            .filter(|s| s.mapped && s.configured)
            .map(|s| s.id)
            .collect();
        let (owner, mut h) = hosted(
            &mut self.edit_menu,
            &mut *self.app,
            &mut self.ctx,
            &self.slots,
            &ids,
        )?;
        h.host();
        if let Some(m) = self.menu_popups.as_ref().filter(|m| m.owner == owner) {
            let keep = kept(&m.spots(), &h.spots());
            let at: Vec<_> = m.levels[..keep].iter().map(|l| l.at).collect();
            for (i, o) in origins(&at).into_iter().enumerate() {
                h.place(i, o);
            }
        }
        let spots = h.spots();
        Some((owner, spots))
    }

    /// Brings the popups in line with the open menu: a popup per level, each placed and drawn.
    pub(crate) fn sync_menu(&mut self) {
        let Some((owner, spots)) = self.place_menu() else {
            self.menu_popups = None;
            return;
        };
        if self.menu_popups.as_ref().is_some_and(|m| m.owner != owner) {
            self.menu_popups = None;
        }
        let popups = self.menu_popups.get_or_insert_with(|| MenuPopups {
            owner,
            levels: Vec::new(),
        });
        let keep = kept(&popups.spots(), &spots);
        let changed = keep < popups.levels.len();
        popups.truncate(keep);
        for (i, spot) in spots.iter().enumerate().skip(keep) {
            // A submenu waits for its parent to show: it is placed against the parent's corner.
            let parent_shown = i == 0
                || self
                    .menu_popups
                    .as_ref()
                    .is_some_and(|m| m.levels[i - 1].buffer.is_some());
            if !parent_shown || !self.open_level(spot, i) {
                break;
            }
        }
        self.draw_menu();
        if changed || keep < spots.len() {
            self.publish(owner);
        }
    }

    /// Opens level `i`'s popup for `spot`; `false` when it could not be made.
    fn open_level(&mut self, spot: &Spot, i: usize) -> bool {
        let p = placement(spot);
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
        if let Some(m) = self.menu_popups.as_mut() {
            m.levels.push(LevelSurface {
                popup,
                spot: *spot,
                at: None,
                buffer: None,
                pool: None,
                fresh: false,
            });
        }
        true
    }

    /// Draws each configured level whose buffer is behind the model, at the owner's scale and
    /// with its access-key cue.
    fn draw_menu(&mut self) {
        let Some(owner) = self.menu_popups.as_ref().map(|m| m.owner) else {
            return;
        };
        let Some(slot) = self.slots.iter().find(|s| s.id == owner) else {
            return;
        };
        let scale = slot.scaling.drawn.max(1);
        let underline = slot.cues.keys;
        let Some((_, h)) = hosted(
            &mut self.edit_menu,
            &mut *self.app,
            &mut self.ctx,
            &self.slots,
            &[owner],
        ) else {
            return;
        };
        let spots = h.spots();
        let shm = &self.shm;
        let ctx = &mut self.ctx;
        let Some(popups) = self.menu_popups.as_mut() else {
            return;
        };
        ctx.theme.access_keys = underline;
        for (i, ls) in popups.levels.iter_mut().enumerate() {
            let Some(spot) = spots.get(i) else {
                break;
            };
            if ls.at.is_none() || ls.fresh {
                continue;
            }
            let r = spot.rect;
            let (w, h_) = (r.width.max(1), r.height.max(1));
            let canvas = ctx.at_scale(scale, |ctx| {
                let m = h.shadow(&ctx.theme);
                let mut c = Canvas::with_scale((w + m) as u32, (h_ + m) as u32, scale);
                h.draw_level(i, &mut c, (-r.x, -r.y), &ctx.theme, &ctx.text);
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
            ls.fresh = true;
        }
        ctx.theme.access_keys = false;
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
            l.fresh = false;
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
        // A moved level draws again; the window and the reader's tree learn where it went.
        ls.fresh = false;
        let owner = m.owner;
        if let Some(s) = self.slot_mut(owner) {
            s.access.state.dirty = true;
            s.dirty = true;
        }
    }

    fn done(&mut self, _: &Connection, _: &QueueHandle<Self>, popup: &Popup) {
        let Some(owner) = self
            .menu_popups
            .as_ref()
            .filter(|m| m.levels.iter().any(|l| &l.popup == popup))
            .map(|m| m.owner)
        else {
            return;
        };
        self.menu_popups = None;
        if let Some((_, mut h)) = hosted(
            &mut self.edit_menu,
            &mut *self.app,
            &mut self.ctx,
            &self.slots,
            &[owner],
        ) {
            h.dismiss();
        }
        if let Some(s) = self.slot_mut(owner) {
            s.dirty = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reprisekit::controls::menu::{MenuCtx, MenuItem, MenuKey, MenuTree, PopupMenu};
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
        let spots = p.spots();
        let root = placement(&spots[0]);
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
    fn a_root_under_a_rect_hangs_from_its_left_end_and_flips_above_it() {
        let title = Rect::new(30, 2, 37, 18);
        let below = placement(&Spot {
            rect: Rect::new(30, 20, 150, 90),
            anchor: Opens::Below(title),
        });
        assert_eq!(below.size, (150, 90));
        assert_eq!(below.anchor_rect, title);
        assert_eq!(
            (below.anchor, below.gravity, below.offset),
            (Anchor::BottomLeft, Gravity::BottomRight, (0, 0))
        );
        assert_eq!(
            below.adjust,
            ConstraintAdjustment::FlipY
                | ConstraintAdjustment::SlideX
                | ConstraintAdjustment::SlideY,
            "it flips above the rect, never beside it"
        );
    }

    #[test]
    fn a_submenu_opens_off_its_row_and_flips_only_sideways() {
        let (theme, text) = (Theme::default(), TextRenderer::load(&FontConfig::default()));
        let t = tree();
        let p = chain(&t, &theme, &text);
        let sub = placement(&p.spots()[1]);
        let (rect, dy) = p.submenu_anchor(1).unwrap();
        assert_eq!((sub.anchor_rect, sub.offset), (rect, (0, dy)));
        assert_eq!(
            (sub.anchor, sub.gravity),
            (Anchor::TopRight, Gravity::BottomRight)
        );
        assert!(!sub.adjust.contains(ConstraintAdjustment::FlipY));
        assert!(sub.adjust.contains(ConstraintAdjustment::FlipX));
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
    fn popups_are_kept_while_their_level_opens_off_the_same_anchor_at_the_same_size() {
        let spot = |x, w, anchor| Spot {
            rect: Rect::new(x, 20, w, 90),
            anchor,
        };
        let file = Opens::Below(Rect::new(4, 2, 30, 18));
        let edit = Opens::Below(Rect::new(34, 2, 30, 18));
        let row = Opens::Beside {
            rect: Rect::new(3, 22, 140, 17),
            dy: -3,
        };
        let other_row = Opens::Beside {
            rect: Rect::new(3, 39, 140, 17),
            dy: -3,
        };
        let have = [spot(4, 150, file), spot(150, 120, row)];
        assert_eq!(
            kept(&have, &[spot(600, 150, file), spot(9, 120, row)]),
            2,
            "a placed level moved, nothing else"
        );
        assert_eq!(
            kept(&have, &[spot(4, 150, file), spot(150, 120, other_row)]),
            1
        );
        assert_eq!(kept(&have, &[spot(4, 150, file)]), 1, "the submenu closed");
        assert_eq!(
            kept(&have, &[spot(34, 150, edit)]),
            0,
            "the next title's menu"
        );
        assert_eq!(kept(&have, &[spot(4, 160, file)]), 0, "the menu grew");
        assert_eq!(kept(&[], &have), 0);
    }
}
