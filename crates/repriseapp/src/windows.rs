//! Making, syncing and sizing the main window, child windows and the flyout.

use crate::runloop::NoEvents;
use crate::sizing::{
    at_pick, bounded, configured_size, held_size, opening_size, rebounded, regrown, sane_side,
    sane_size,
};
use crate::{
    Caption, Child, Event, Flyout, Host, MAIN, MIN_SIZE, Scaling, Shell, Slot, WinId, access,
};
use reprisekit::cues::Cues;
use reprisekit::raster::{Canvas, Rect};
use smithay_client_toolkit::reexports::protocols::wp::keyboard_shortcuts_inhibit::zv1::client::zwp_keyboard_shortcuts_inhibitor_v1;
use smithay_client_toolkit::reexports::protocols::xdg::{
    dialog::v1::client::xdg_dialog_v1,
    foreign::{zv1::client::zxdg_imported_v1, zv2::client::zxdg_imported_v2},
    shell::client::xdg_toplevel,
    toplevel_tag::v1::client::xdg_toplevel_tag_manager_v1,
};
use smithay_client_toolkit::{
    activation::{ActivationHandler, RequestData},
    dispatch2::Dispatch2,
    shell::{
        WaylandSurface,
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
        xdg::window::{Window, WindowConfigure, WindowDecorations, WindowHandler},
    },
};
use wayland_client::{Connection, QueueHandle, protocol::wl_output};

impl Host {
    /// Ask for a token on the last input for the keyboard's window, which `new_token` presents
    /// for window `id`.
    pub(crate) fn raise(&self, id: WinId) {
        let (Some(activation), Some(seat)) = (&self.activation, &self.seat) else {
            return;
        };
        let surface = self
            .focus
            .and_then(|f| self.slots.iter().find(|s| s.id == f))
            .map(|s| s.window.wl_surface().clone());
        activation.request_token(
            &self.qh,
            RequestData {
                app_id: Some(self.app.app_id()),
                seat_and_serial: Some((seat.clone(), self.last_serial)),
                surface,
                udata: Some(id),
            },
        );
    }

    fn send_tag(&mut self, window: &Window, id: WinId, caption: Caption) {
        let tag = self.tag_of(id, caption);
        if !tag.is_empty() {
            send_tag(self.tag_manager.as_ref(), window, tag);
        }
    }

    /// Window `id`'s xdg-toplevel-tag: its caption's, or `file-dialog` for a file dialog with a
    /// plain caption, whose size and place the compositor hands the next one.
    pub(crate) fn tag_of(&mut self, id: WinId, caption: Caption) -> &'static str {
        match caption {
            Caption::Window if self.app.drop_file_dialog(id).is_some() => "file-dialog",
            Caption::Window => "",
            Caption::Prompt => "prompt",
            Caption::Help => "help-dialog",
            Caption::Fixed => "fixed-dialog",
        }
    }

    /// Make `window` the child of the foreign `handle`, and modal to it
    /// when the program asks. Runs before the window's first commit, so
    /// it maps already centred on its owner. None when the compositor has
    /// no importer; the window then stays ownerless.
    fn import_parent(&self, handle: &str, window: &Window) -> Option<Foreign> {
        let surface = window.wl_surface();
        let imported = if let Some(v2) = &self.importer_v2 {
            let i = v2.import_toplevel(handle.to_string(), &self.qh, ForeignParent);
            i.set_parent_of(surface);
            Imported::V2(i)
        } else if let Some(v1) = &self.importer_v1 {
            let i = v1.import(handle.to_string(), &self.qh, ForeignParent);
            i.set_parent_of(surface);
            Imported::V1(i)
        } else {
            log::warn!("no xdg-foreign importer; the window stays ownerless");
            return None;
        };
        let dialog = if self.app.modal() {
            self.wm_dialog.as_ref().map(|d| {
                let x = d.get_xdg_dialog(window.xdg_toplevel(), &self.qh, NoEvents);
                x.set_modal();
                x
            })
        } else {
            None
        };
        Some(Foreign { imported, dialog })
    }

    fn new_slot(
        &self,
        id: WinId,
        window: Shell,
        scaling: Scaling,
        title: String,
        size: (i32, i32),
        caption: Caption,
    ) -> Slot {
        Slot {
            id,
            window,
            scaling,
            buffers: [None, None],
            attached: 0,
            pool: None,
            canvas: Canvas::new(0, 0),
            title,
            size,
            origin: (0, 0),
            maximized: false,
            asked: (None, None),
            own: false,
            bounds: None,
            wanted: size,
            picked: size,
            activated: true,
            configured: false,
            dirty: true,
            frame_pending: false,
            mapped: false,
            modal: false,
            caption,
            access: access::SlotAccess::new(id, self.a11y_bus.then_some(&self.a11y)),
            glass: None,
            glass_top: (0, 0),
            cues: Cues::opened(self.key_last),
            parent: None,
            spot: None,
            tip: None,
            tip_waits: false,
        }
    }

    fn make_main(&mut self) {
        let ((w, h), min) = opening_size(&*self.app);
        let surface = self.compositor.create_surface(&self.qh);
        let scaling = self.scaling(&surface);
        let window =
            self.xdg_shell
                .create_window(surface, WindowDecorations::RequestServer, &self.qh);
        let title = self.app.title();
        window.set_title(title.clone());
        window.set_app_id(self.app.app_id());
        window.set_min_size(Some((min.0 as u32, min.1 as u32)));
        if self.app.fixed() {
            window.set_max_size(Some((w as u32, h as u32)));
        }
        if let Some(handle) = self.app.foreign_parent() {
            self.foreign = self.import_parent(&handle, &window);
        }
        let caption = self.app.caption(MAIN);
        self.send_tag(&window, MAIN, caption);
        window.commit();
        let slot = self.new_slot(
            MAIN,
            Shell::Toplevel(window),
            scaling,
            title,
            (w, h),
            caption,
        );
        self.slots.insert(0, slot);
    }

    pub(crate) fn sync_main(&mut self) {
        let have = self.slots.iter().any(|s| s.id == MAIN);
        match main_wanted(self.app.hidden(), have) {
            Some(true) => self.make_main(),
            Some(false) => {
                // A toplevel must not be anyone's parent when destroyed.
                for s in self.slots.iter().filter(|s| s.id != MAIN) {
                    if let Some(w) = s.window.toplevel() {
                        w.xdg_toplevel().set_parent(None);
                        w.commit();
                    }
                }
                if self.focus == Some(MAIN) {
                    self.focus = None;
                }
                self.foreign = None;
                self.drop_menus(|w| w != MAIN);
                self.slots.retain(|s| s.id != MAIN);
            }
            None => {}
        }
    }

    /// Window `id`'s min and max client size as the program asks for them now: a fixed window
    /// pinned at its size, a resizable one held at its minimum. Both stay inside the bounds.
    fn limits(&self, id: WinId) -> Option<Limits> {
        let s = self.slots.iter().find(|s| s.id == id)?;
        let (min, max) = if id == MAIN {
            if self.app.fixed() {
                (s.size, Some(s.size))
            } else {
                (self.app.min_size(MAIN).map_or(MIN_SIZE, sane_size), None)
            }
        } else {
            let c = self.app.children().into_iter().find(|c| c.id == id)?;
            let size = sane_size(c.size);
            match c.fixed {
                true => (size, Some(size)),
                false => (self.app.min_size(id).map_or(size, sane_size), None),
            }
        };
        Some((bounded(min, s.bounds), max.map(|m| bounded(m, s.bounds))))
    }

    /// The size window `id`'s program picks for it now: the main window's opening size, a
    /// child's [`Child::size`].
    fn natural(&self, id: WinId) -> Option<(i32, i32)> {
        if id == MAIN {
            return Some(opening_size(&*self.app).0);
        }
        let c = self.app.children().into_iter().find(|c| c.id == id)?;
        Some(sane_size(c.size))
    }

    /// Fit window `id` to what its program asks now, after that changed (the text size, a
    /// child's size): a fixed window takes its size; a resizable one still at its program's
    /// pick takes the size the program picks now, one the user sized keeps its size; both at
    /// least the minimum. The compositor keeps its top-left; a maximized window keeps the
    /// screen's size. A window that changed size hears `Event::Resized`.
    pub(crate) fn refit(&mut self, id: WinId) {
        let Some((min, max)) = self.limits(id) else {
            return;
        };
        self.declare_limits(id);
        let natural = self.natural(id);
        let Some(s) = self.slot_mut(id) else {
            return;
        };
        if s.maximized {
            return;
        }
        let natural = natural.filter(|_| at_pick(s.size, s.picked, s.bounds));
        if let Some(n) = natural {
            s.picked = n;
        }
        let size = bounded(regrown(s.size, min, max, natural), s.bounds);
        if max.is_some() {
            s.picked = size;
        }
        if size == s.size {
            return;
        }
        s.size = size;
        s.buffers = [None, None];
        s.pool = None;
        s.dirty = true;
        s.own = true;
        self.send(id, Event::Resized);
    }

    /// Tell the compositor window `id`'s [`Host::limits`].
    fn declare_limits(&mut self, id: WinId) {
        let Some((min, max)) = self.limits(id) else {
            return;
        };
        let Some(w) = self.slot_mut(id).and_then(|s| s.window.toplevel()) else {
            return;
        };
        w.set_min_size(Some((min.0 as u32, min.1 as u32)));
        if let Some(max) = max {
            w.set_max_size(Some((max.0 as u32, max.1 as u32)));
        }
    }

    pub(crate) fn sync_children(&mut self) {
        // `MAIN` is the host's: a `Child` claiming that id is dropped
        // here, so it can never retitle the main window or make it modal.
        let want: Vec<Child> = self
            .app
            .children()
            .into_iter()
            .filter(|c| c.id != MAIN)
            .collect();
        let have: Vec<WinId> = self
            .slots
            .iter()
            .filter(|s| s.id != MAIN && s.window.toplevel().is_some())
            .map(|s| s.id)
            .collect();
        let refs: Vec<&Child> = want.iter().collect();
        let (drop, make) = reconcile(&have, &refs);
        if self.focus.is_some_and(|f| drop.contains(&f)) {
            self.focus = None;
        }
        // A toplevel must not be anyone's parent when destroyed.
        for s in self.slots.iter_mut() {
            if s.parent.is_some_and(|p| drop.contains(&p)) {
                if let Some(w) = s.window.toplevel() {
                    w.xdg_toplevel().set_parent(None);
                    w.commit();
                }
                s.parent = None;
            }
        }
        self.drop_menus(|w| w == MAIN || !drop.contains(&w));
        self.slots.retain(|s| s.id == MAIN || !drop.contains(&s.id));
        let app_id = self.app.app_id();
        for i in make {
            let c = &want[i];
            // `reconcile` does not dedupe `want`; one surface per id.
            if self.slots.iter().any(|s| s.id == c.id) {
                continue;
            }
            let wanted = self.app.parent(c.id);
            let parent_slot = wanted.and_then(|p| self.slots.iter().find(|s| s.id == p));
            let parent = match placement(wanted, parent_slot.map(|s| s.mapped)) {
                // xdg-shell drops a parent that is not mapped yet; `draw` calls back here once
                // it is.
                Placement::Wait => continue,
                Placement::Under(_) => parent_slot
                    .and_then(|s| s.window.toplevel())
                    .map(|w| w.xdg_toplevel().clone()),
                Placement::Alone => None,
            };
            let size = sane_size(c.size);
            let surface = self.compositor.create_surface(&self.qh);
            let scaling = self.scaling(&surface);
            let window =
                self.xdg_shell
                    .create_window(surface, WindowDecorations::RequestServer, &self.qh);
            window.set_title(c.title.clone());
            window.set_app_id(app_id.clone());
            let min = match c.fixed {
                true => size,
                false => self.app.min_size(c.id).map_or(size, sane_size),
            };
            window.set_min_size(Some((min.0 as u32, min.1 as u32)));
            if c.fixed {
                window.set_max_size(Some((size.0 as u32, size.1 as u32)));
            }
            window.xdg_toplevel().set_parent(parent.as_ref());
            if wanted.is_none() {
                self.join_tabs(&window, c.id);
            }
            let caption = self.app.caption(c.id);
            self.send_tag(&window, c.id, caption);
            window.commit();
            let mut slot = self.new_slot(
                c.id,
                Shell::Toplevel(window),
                scaling,
                c.title.clone(),
                size,
                caption,
            );
            slot.modal = c.modal;
            slot.parent = wanted;
            self.slots.push(slot);
        }
        let mut resized = Vec::new();
        for c in &want {
            self.reparent(c.id);
            let caption = self.app.caption(c.id);
            let tag = match self
                .slots
                .iter()
                .any(|s| s.id == c.id && s.caption != caption)
            {
                true => self.tag_of(c.id, caption),
                false => "",
            };
            if let Some(s) = self.slots.iter_mut().find(|s| s.id == c.id) {
                s.modal = c.modal;
                if s.wanted != sane_size(c.size) {
                    s.wanted = sane_size(c.size);
                    resized.push(c.id);
                }
                if s.title != c.title
                    && let Some(w) = s.window.toplevel()
                {
                    s.title = c.title.clone();
                    w.set_title(c.title.clone());
                }
                if s.caption != caption
                    && let Some(w) = s.window.toplevel()
                {
                    s.caption = caption;
                    send_tag(self.tag_manager.as_ref(), w, tag);
                }
            }
        }
        for id in resized {
            self.refit(id);
        }
    }

    /// Move child `id` over the window [`Extended::parent`](crate::Extended::parent) names now, once that one
    /// is mapped.
    fn reparent(&mut self, id: WinId) {
        let want = self.app.parent(id);
        let Some(s) = self.slots.iter().find(|s| s.id == id) else {
            return;
        };
        if s.parent == want {
            return;
        }
        let parent_slot = want.and_then(|p| self.slots.iter().find(|s| s.id == p));
        let parent = match placement(want, parent_slot.map(|s| s.mapped)) {
            Placement::Wait => return,
            Placement::Under(_) => parent_slot
                .and_then(|s| s.window.toplevel())
                .map(|w| w.xdg_toplevel().clone()),
            Placement::Alone => None,
        };
        if let Some(w) = s.window.toplevel() {
            w.xdg_toplevel().set_parent(parent.as_ref());
            w.commit();
        }
        if let Some(s) = self.slot_mut(id) {
            s.parent = want;
        }
    }

    /// Ask for unparented child `id`'s new `window` to join its
    /// [`Extended::tab_with`](crate::Extended::tab_with) window's tab group, before its first commit.
    fn join_tabs(&self, window: &Window, id: WinId) {
        let leader = self
            .app
            .tab_with(id)
            .and_then(|l| self.slots.iter().find(|s| s.id == l))
            .and_then(|s| s.window.toplevel());
        if let (Some(tabs), Some(leader)) = (&self.tab_groups, leader) {
            tabs.join(window.xdg_toplevel(), leader.xdg_toplevel());
        }
    }

    pub(crate) fn sync_inhibitor(&mut self) {
        let want = self
            .app
            .shortcuts_inhibited()
            .filter(|w| self.slots.iter().any(|s| s.id == *w));
        if self.inhibitor.as_ref().map(|(w, _)| *w) == want {
            return;
        }
        if let Some((_, i)) = self.inhibitor.take() {
            i.destroy();
        }
        let (Some(win), Some(manager), Some(seat)) = (want, &self.inhibit_manager, &self.seat)
        else {
            return;
        };
        let Some(s) = self.slots.iter().find(|s| s.id == win) else {
            return;
        };
        let i = manager.inhibit_shortcuts(s.window.wl_surface(), seat, &self.qh, InhibitorOf(win));
        self.inhibitor = Some((win, i));
    }

    pub(crate) fn sync_flyout(&mut self) {
        let have = self
            .slots
            .iter()
            .find(|s| s.window.toplevel().is_none())
            .map(|s| s.id);
        let (drop, make) = flyout_wanted(self.app.flyout(), have);
        if let Some(id) = drop {
            if self.focus == Some(id) {
                self.focus = None;
            }
            self.drop_menus(|w| w != id);
            self.slots.retain(|s| s.id != id);
        }
        if let Some(f) = make {
            self.make_flyout(f);
        }
    }

    fn make_flyout(&mut self, f: Flyout) {
        let Some(shell) = &self.layer_shell else {
            log::warn!("the compositor has no layer shell; the flyout cannot open");
            return;
        };
        let outputs: Vec<(wl_output::WlOutput, (i32, i32, i32, i32))> = self
            .output_state
            .outputs()
            .filter_map(|o| {
                let info = self.output_state.info(&o)?;
                let (x, y) = info.logical_position?;
                let (w, h) = info.logical_size?;
                Some((o, (x, y, w, h)))
            })
            .collect();
        let rects: Vec<_> = outputs.iter().map(|(_, r)| *r).collect();
        let (output, origin) = match output_containing(&rects, f.at) {
            Some(i) => (Some(&outputs[i].0), (rects[i].0, rects[i].1)),
            None => (None, (0, 0)),
        };
        let surface = self.compositor.create_surface(&self.qh);
        let scaling = self.scaling(&surface);
        let layer = shell.create_layer_surface(
            &self.qh,
            surface,
            Layer::Overlay,
            Some(self.app.app_id()),
            output,
        );
        // The whole output, taskbar included, so a click anywhere else lands here.
        layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
        layer.commit();
        let mut slot = self.new_slot(
            f.id,
            Shell::Layer(layer),
            scaling,
            f.title.into(),
            (0, 0),
            Caption::Window,
        );
        slot.origin = origin;
        self.slots.push(slot);
    }

    fn configured(
        &mut self,
        id: WinId,
        (w, h): (i32, i32),
        maximized: bool,
        qh: &QueueHandle<Host>,
    ) {
        let Some(s) = self.slot_mut(id) else {
            return;
        };
        s.maximized = maximized;
        let changed = (w, h) != s.size || !s.configured;
        if changed {
            s.size = (w, h);
            s.buffers = [None, None];
            s.pool = None;
        }
        s.configured = true;
        if changed {
            self.send(id, Event::Resized);
        }
        self.draw(id, qh);
    }

    pub(crate) fn begin_resize(&mut self, win: WinId) {
        let Some(seat) = &self.seat else {
            return;
        };
        if let Some(w) = self
            .slots
            .iter()
            .find(|s| s.id == win)
            .and_then(|s| s.window.toplevel())
        {
            w.resize(
                seat,
                self.press_serial,
                xdg_toplevel::ResizeEdge::BottomRight,
            );
        }
    }
}

impl WindowHandler for Host {
    fn request_close(&mut self, _: &Connection, _: &QueueHandle<Self>, window: &Window) {
        let Some(id) = self.slot_of(window.wl_surface()) else {
            return;
        };
        self.send(id, Event::CloseRequest);
    }

    fn configure(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        window: &Window,
        configure: WindowConfigure,
        _: u32,
    ) {
        let Some(id) = self.slot_of(window.wl_surface()) else {
            return;
        };
        let asked = (
            configure.new_size.0.map(|v| sane_side(v.get().into())),
            configure.new_size.1.map(|v| sane_side(v.get().into())),
        );
        let bounds = configure
            .suggested_bounds
            .filter(|&(w, h)| w > 0 && h > 0)
            .map(|(w, h)| sane_size((w as i32, h as i32)));
        let s = self.slot_mut(id).expect("slot");
        let (size, own) = configured_size(s.size, asked, s.asked, s.own, configure.is_resizing());
        let moved_bounds = bounds.is_some() && bounds != s.bounds;
        let size = match moved_bounds && size == s.size {
            true => rebounded(size, s.picked, s.bounds, bounds),
            false => size,
        };
        if bounds.is_some() {
            s.bounds = bounds;
        }
        let size = held_size(size, asked, s.bounds);
        s.asked = asked;
        s.own = own;
        if s.mapped {
            s.activated = configure.is_activated();
        }
        if moved_bounds {
            self.declare_limits(id);
        }
        self.configured(id, size, configure.is_maximized(), qh);
    }
}

impl LayerShellHandler for Host {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface) {
        if let Some(id) = self.slot_of(layer.wl_surface()) {
            self.send(id, Event::CloseRequest);
        }
    }

    fn configure(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _: u32,
    ) {
        let Some(id) = self.slot_of(layer.wl_surface()) else {
            return;
        };
        let (w, h) = configure.new_size;
        let size = (sane_side(w.into()), sane_side(h.into()));
        self.configured(id, size, false, qh);
    }
}

/// A window's min client size and, for a fixed one, its max.
type Limits = ((i32, i32), Option<(i32, i32)>);

impl ActivationHandler for Host {
    /// The window a token was asked for ([`Host::raise`]).
    type RequestUdata = Option<WinId>;
    fn new_token(&mut self, token: String, data: &RequestData<Option<WinId>>) {
        let surface = data
            .udata
            .and_then(|id| self.slots.iter().find(|s| s.id == id))
            .map(|s| s.window.wl_surface());
        if let (Some(surface), Some(activation)) = (surface, &self.activation) {
            activation.activate::<Host>(surface, token);
        }
    }
}

pub(crate) fn send_tag(
    manager: Option<&xdg_toplevel_tag_manager_v1::XdgToplevelTagManagerV1>,
    window: &Window,
    tag: &str,
) {
    if let Some(m) = manager {
        m.set_toplevel_tag(window.xdg_toplevel(), tag.into());
    }
}

/// A main window's owner in another program: the imported handle and,
/// when modal, the xdg-dialog object. Held only to stay alive; dropping
/// it destroys both, which undoes the parent and the modality.
pub(crate) struct Foreign {
    imported: Imported,
    dialog: Option<xdg_dialog_v1::XdgDialogV1>,
}

enum Imported {
    V2(zxdg_imported_v2::ZxdgImportedV2),
    V1(zxdg_imported_v1::ZxdgImportedV1),
}

impl Drop for Foreign {
    fn drop(&mut self) {
        if let Some(d) = self.dialog.take() {
            d.destroy();
        }
        match &self.imported {
            Imported::V2(i) => i.destroy(),
            Imported::V1(i) => i.destroy(),
        }
    }
}

struct ForeignParent;

impl Dispatch2<zxdg_imported_v2::ZxdgImportedV2, Host> for ForeignParent {
    fn event(
        &self,
        _: &mut Host,
        _: &zxdg_imported_v2::ZxdgImportedV2,
        event: zxdg_imported_v2::Event,
        _: &Connection,
        _: &QueueHandle<Host>,
    ) {
        if let zxdg_imported_v2::Event::Destroyed = event {
            log::info!("the foreign parent is gone; the window stays, ownerless");
        }
    }
}

impl Dispatch2<zxdg_imported_v1::ZxdgImportedV1, Host> for ForeignParent {
    fn event(
        &self,
        _: &mut Host,
        _: &zxdg_imported_v1::ZxdgImportedV1,
        event: zxdg_imported_v1::Event,
        _: &Connection,
        _: &QueueHandle<Host>,
    ) {
        if let zxdg_imported_v1::Event::Destroyed = event {
            log::info!("the foreign parent is gone; the window stays, ownerless");
        }
    }
}

struct InhibitorOf(WinId);

impl Dispatch2<zwp_keyboard_shortcuts_inhibitor_v1::ZwpKeyboardShortcutsInhibitorV1, Host>
    for InhibitorOf
{
    fn event(
        &self,
        host: &mut Host,
        _: &zwp_keyboard_shortcuts_inhibitor_v1::ZwpKeyboardShortcutsInhibitorV1,
        event: zwp_keyboard_shortcuts_inhibitor_v1::Event,
        _: &Connection,
        _: &QueueHandle<Host>,
    ) {
        if let zwp_keyboard_shortcuts_inhibitor_v1::Event::Inactive = event {
            host.send(self.0, Event::ShortcutsInactive);
        }
    }
}

pub(crate) fn reconcile(have: &[WinId], want: &[&Child]) -> (Vec<WinId>, Vec<usize>) {
    let drop = have
        .iter()
        .filter(|id| !want.iter().any(|c| c.id == **id))
        .copied()
        .collect();
    let make = want
        .iter()
        .enumerate()
        .filter(|(_, c)| !have.contains(&c.id))
        .map(|(i, _)| i)
        .collect();
    (drop, make)
}

pub(crate) static TAB_GROUPS: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Whether the compositor puts a window into another's caption tab group
/// ([`Extended::tab_with`](crate::Extended::tab_with)), known once [`run`](crate::run) connected, before
/// [`App::start`](crate::App::start).
pub fn tab_groups() -> bool {
    TAB_GROUPS.load(std::sync::atomic::Ordering::Relaxed)
}

/// Where a child whose program names `parent` opens, given whether that parent's window is
/// mapped (`None`: it has no window).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Placement {
    Wait,
    Under(WinId),
    Alone,
}

pub(crate) fn placement(parent: Option<WinId>, mapped: Option<bool>) -> Placement {
    match (parent, mapped) {
        (Some(_), Some(false)) => Placement::Wait,
        (Some(p), Some(true)) => Placement::Under(p),
        _ => Placement::Alone,
    }
}

pub(crate) fn flyout_wanted(
    want: Option<Flyout>,
    have: Option<WinId>,
) -> (Option<WinId>, Option<Flyout>) {
    match (want, have) {
        (Some(f), Some(id)) if f.id == id => (None, None),
        (want, have) => (have, want),
    }
}

pub(crate) fn output_containing(outputs: &[(i32, i32, i32, i32)], at: (i32, i32)) -> Option<usize> {
    outputs
        .iter()
        .position(|&(x, y, w, h)| Rect::new(x, y, w, h).contains(at.0, at.1))
}

pub(crate) fn main_wanted(hidden: bool, have: bool) -> Option<bool> {
    match (hidden, have) {
        (false, false) => Some(true),
        (true, true) => Some(false),
        _ => None,
    }
}
