use std::cell::RefCell;

use reprisekit::classic::frame::{Chrome, FrameGeometry};
use reprisescene::Input;
use smithay::{
    backend::renderer::utils::with_renderer_surface_state,
    reexports::{
        wayland_protocols::xdg::shell::server::xdg_toplevel,
        wayland_server::protocol::wl_surface::WlSurface,
    },
    wayland::shell::xdg::ToplevelSurface,
    xwayland::xwm::WmWindowType,
};

use crate::compositor::{SceneOwner, State};
use crate::glass::Backdrop;
use crate::input::Interaction;
use crate::mapfocus::OnMap;
use crate::raster::Rect;
use crate::windowrules::{Position, Want, WindowState};

use super::{
    DialogTag, Kind, LastFrame, Window, WindowId, anchor_done, anchored_position, cascade_from,
    cascade_position, centre_on, fit_size, keep_inside,
};

impl State {
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
            blur_ops: Default::default(),
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
            last_frame: None,
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
            attention: None,
            urgent: false,
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
        let (surface, origin, client) = (
            win.wl_surface().clone(),
            win.geometry_origin(),
            win.client_size(),
        );
        let blur = crate::clientglass::region(&surface, &mut win.blur_ops, origin, client);
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
        let last = win.last_frame.take();
        let flicker = last.filter(LastFrame::flicker);
        // A client can gain an owner between first configure and map.
        let placeable = win.placeable();
        let mut want = (win.placement.take()).filter(|_| placeable && flicker.is_none());
        let (app, title) = (win.app(), win.title());
        // Windows of one app planned together can each plan the same saved
        // tab group; the one that maps as another group's takes its frame.
        let mut resized = false;
        if placeable
            && flicker.is_none()
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
        // A remap goes back where the window was unless a rule places it.
        let remap = last.filter(|_| !matches!(position, Some(Position::At(..))));
        if let Some(l) = remap {
            size = Some((l.normal.width, l.normal.height));
        }
        let kept = (self.file_dialog_position(w)).or(remap.map(|l| (l.normal.x, l.normal.y)));
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
            && remap.is_none()
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
        let on_map =
            crate::mapfocus::on_map(locked, here, bar_keys, &mut activated, |a| match flicker {
                Some(l) => l.focused || a,
                None => self.takes_focus_on_map(w, a),
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
                if !ruled {
                    self.ask_attention(w);
                }
                self.place_behind_focus(w);
            }
            OnMap::Focus => self.focus_window(w),
            OnMap::Behind => {
                if flicker.is_none() {
                    self.ask_attention(w);
                }
                self.place_behind_focus(w);
            }
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
        let (was_maximized, was_fullscreen) =
            last.map_or((false, false), |l| (l.maximized, l.fullscreen));
        if state.map_or(asked_maximized || was_maximized, |s| {
            s == WindowState::Maximized
        }) {
            self.set_maximized(w, true);
            if let Some(win) = self.win_mut(w)
                && win.maximized
                && let Some(s) = size
            {
                win.saved = Some(Rect::new(x, y, s.0, s.1));
            }
        }
        if state.map_or(asked_fullscreen || was_fullscreen, |s| {
            s == WindowState::Fullscreen
        }) {
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

    pub fn unmap_window(&mut self, w: WindowId) {
        // Before anything hides it: its still is taken as it shows.
        self.window_closing(w);
        let was_focused = self.focused == Some(w);
        if let Some(win) = self.win_mut(w) {
            win.last_frame = Some(LastFrame {
                normal: win.normal_frame(),
                maximized: win.maximized,
                fullscreen: win.fullscreen,
                focused: was_focused,
                at: std::time::Instant::now(),
            });
        }
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
