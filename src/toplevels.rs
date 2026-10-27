//! ext-foreign-toplevel-list: one handle per mapped window, so a portal backend can list and
//! capture windows and a bar can show them. The handle closes when the window unmaps.
//!
//! wlr-foreign-toplevel-management: the same windows with state and output, for docks and
//! task switchers. `State::refresh_toplevels` brings every handle up to date at the end of a
//! loop iteration that set `Toplevels::dirty`. Adapted from niri's `src/protocols/foreign_toplevel.rs` (GPL-3.0-or-later).

use std::collections::{BTreeSet, HashMap};

use smithay::{
    output::Output,
    reexports::{
        wayland_protocols_wlr::foreign_toplevel::v1::server::{
            zwlr_foreign_toplevel_handle_v1::{self, ZwlrForeignToplevelHandleV1},
            zwlr_foreign_toplevel_manager_v1::{self, ZwlrForeignToplevelManagerV1},
        },
        wayland_server::{
            Client, DataInit, DisplayHandle, New, Resource, backend::ClientId,
            protocol::wl_output::WlOutput,
        },
    },
    wayland::{
        foreign_toplevel_list::{
            ForeignToplevelHandle, ForeignToplevelListHandler, ForeignToplevelListState,
        },
        image_capture_source::ToplevelCaptureSourceState,
        {Dispatch2, GlobalDispatch2},
    },
};

use crate::{
    compositor::State, inputproto::proto::reprise_sandboxed_v1::RepriseSandboxedV1,
    window::WindowId,
};

const LOOKUP_TRIES: u8 = 3;

pub struct Toplevels {
    list: ForeignToplevelListState,
    handles: HashMap<WindowId, ForeignToplevelHandle>,
    pub capture: ToplevelCaptureSourceState,
    wlr: Wlr,
    /// The Flatpak app id of each listed window whose client runs in a sandbox.
    sandboxed: HashMap<WindowId, String>,
    /// Listed windows not yet looked up for `sandboxed`, with their failed lookups.
    unresolved: HashMap<WindowId, u8>,
    /// The portal's `reprise_sandboxed_v1` objects, told each change of `sandboxed_apps`.
    pub sandbox_watchers: Vec<RepriseSandboxedV1>,
    /// A window's title, app id, state, focus or output may have changed since the last
    /// `refresh_toplevels`; set by every scene redraw ask, focus change and retitle.
    pub dirty: bool,
    #[cfg(test)]
    refreshes: u32,
}

impl Toplevels {
    pub fn new(dh: &DisplayHandle) -> Toplevels {
        Toplevels {
            list: ForeignToplevelListState::new_with_filter::<State>(
                dh,
                crate::sandbox::filter("ext_foreign_toplevel_list_v1"),
            ),
            handles: HashMap::new(),
            capture: ToplevelCaptureSourceState::new_with_filter::<State, _>(
                dh,
                crate::sandbox::filter("ext_foreign_toplevel_image_capture_source_manager_v1"),
            ),
            wlr: Wlr::new(dh),
            sandboxed: HashMap::new(),
            unresolved: HashMap::new(),
            sandbox_watchers: Vec::new(),
            dirty: true,
            #[cfg(test)]
            refreshes: 0,
        }
    }

    pub fn window_of(&self, handle: &ForeignToplevelHandle) -> Option<WindowId> {
        self.handles
            .iter()
            .find(|(_, h)| h.matches(handle))
            .map(|(w, _)| *w)
    }

    /// The Flatpak app ids of the sandboxed clients with a listed window.
    pub fn sandboxed_apps(&self) -> BTreeSet<String> {
        self.sandboxed.values().cloned().collect()
    }

    /// Send the current `sandboxed_apps` to `watcher`.
    pub fn send_sandboxed(&self, watcher: &RepriseSandboxedV1) {
        for app in self.sandboxed_apps() {
            watcher.app(app);
        }
        watcher.done();
    }

    /// Take `got`, the lookup of `w`'s app, into `sandboxed`; a failed lookup stays unresolved
    /// for a later sync, at most [`LOOKUP_TRIES`] times.
    fn resolved(&mut self, w: WindowId, got: Result<Option<String>, ()>) {
        match got {
            Ok(app) => {
                self.unresolved.remove(&w);
                if let Some(app) = app {
                    let before = self.sandboxed_apps();
                    self.sandboxed.insert(w, app);
                    self.sandboxed_changed(&before);
                }
            }
            Err(()) => {
                let tries = self.unresolved.entry(w).or_default();
                *tries += 1;
                if *tries >= LOOKUP_TRIES {
                    self.unresolved.remove(&w);
                }
            }
        }
    }

    fn sandboxed_changed(&self, before: &BTreeSet<String>) {
        if *before != self.sandboxed_apps() {
            for w in &self.sandbox_watchers {
                self.send_sandboxed(w);
            }
        }
    }
}

impl ForeignToplevelListHandler for State {
    fn foreign_toplevel_list_state(&mut self) -> &mut ForeignToplevelListState {
        &mut self.toplevels.list
    }
}

impl State {
    /// List a mapped `w`, or send its title and app id when they changed;
    /// an unmapped window, an X11 menu or a tooltip is not listed.
    pub fn sync_toplevel(&mut self, w: WindowId) {
        let Some(win) = self.win(w).filter(|w| w.mapped && !w.override_redirect) else {
            return;
        };
        let (title, app_id) = (win.title(), win.app_id());
        let t = &mut self.toplevels;
        t.dirty = true;
        match t.handles.get(&w) {
            Some(h) => {
                if h.title() != title || h.app_id() != app_id {
                    h.send_title(&title);
                    h.send_app_id(&app_id);
                    h.send_done();
                }
            }
            None => {
                let h = t.list.new_toplevel::<State>(title, app_id);
                t.handles.insert(w, h);
                t.unresolved.insert(w, 0);
            }
        }
        self.resolve_sandboxed(w);
    }

    /// Look up unresolved `w`'s app while a portal watches the list: an X11 window costs an XRes
    /// round trip.
    fn resolve_sandboxed(&mut self, w: WindowId) {
        if !self.toplevels.sandbox_watchers.is_empty() {
            self.look_up_sandboxed(w);
        }
    }

    fn look_up_sandboxed(&mut self, w: WindowId) {
        if !self.toplevels.unresolved.contains_key(&w) {
            return;
        }
        let Some(win) = self.win(w) else {
            return;
        };
        let got = crate::sandbox::window_app(win);
        self.toplevels.resolved(w, got);
    }

    /// Bind `watcher` to the list, first looking up the windows mapped while none watched.
    pub fn watch_sandboxed(&mut self, watcher: RepriseSandboxedV1) {
        let pending: Vec<WindowId> = self.toplevels.unresolved.keys().copied().collect();
        for w in pending {
            self.look_up_sandboxed(w);
        }
        self.toplevels.send_sandboxed(&watcher);
        self.toplevels.sandbox_watchers.push(watcher);
    }

    pub fn withdraw_toplevel(&mut self, w: WindowId) {
        crate::capture::window_gone(self, w);
        if let Some(h) = self.toplevels.handles.remove(&w) {
            self.toplevels.list.remove_toplevel(&h);
        }
        let t = &mut self.toplevels;
        t.unresolved.remove(&w);
        let before = t.sandboxed_apps();
        if t.sandboxed.remove(&w).is_some() {
            t.sandboxed_changed(&before);
        }
    }
}

struct Wlr {
    dh: DisplayHandle,
    managers: Vec<ZwlrForeignToplevelManagerV1>,
    windows: HashMap<WindowId, WlrWindow>,
}

/// What the instances last heard about a window, and its handle in each
/// manager with the wl_outputs it was told it entered.
struct WlrWindow {
    shown: Shown,
    handles: Vec<(ZwlrForeignToplevelHandleV1, Vec<WlOutput>)>,
}

#[derive(Clone, PartialEq)]
struct Shown {
    title: String,
    app_id: String,
    states: Vec<u32>,
    output: Option<Output>,
}

impl Shown {
    fn states_bytes(&self) -> Vec<u8> {
        self.states.iter().flat_map(|s| s.to_ne_bytes()).collect()
    }
}

const WLR_VERSION: u32 = 3;

/// The manager global's filter: hidden from sandboxed clients.
pub struct WlrGlobal;

impl Wlr {
    fn new(dh: &DisplayHandle) -> Wlr {
        dh.create_global::<State, ZwlrForeignToplevelManagerV1, _>(WLR_VERSION, WlrGlobal);
        Wlr {
            dh: dh.clone(),
            managers: Vec::new(),
            windows: HashMap::new(),
        }
    }
}

/// The wlr state values for a window's flags.
fn wlr_states(maximized: bool, minimized: bool, activated: bool, fullscreen: bool) -> Vec<u32> {
    use zwlr_foreign_toplevel_handle_v1::State as S;
    [
        (maximized, S::Maximized),
        (minimized, S::Minimized),
        (activated, S::Activated),
        (fullscreen, S::Fullscreen),
    ]
    .into_iter()
    .filter(|(on, _)| *on)
    .map(|(_, s)| s as u32)
    .collect()
}

impl WlrWindow {
    fn add_handle(
        &mut self,
        dh: &DisplayHandle,
        manager: &ZwlrForeignToplevelManagerV1,
        w: WindowId,
    ) {
        let Some(client) = manager.client() else {
            return;
        };
        let Ok(handle) = client.create_resource::<ZwlrForeignToplevelHandleV1, _, State>(
            dh,
            manager.version(),
            w,
        ) else {
            return;
        };
        manager.toplevel(&handle);
        handle.title(self.shown.title.clone());
        handle.app_id(self.shown.app_id.clone());
        handle.state(self.shown.states_bytes());
        let mut entered = Vec::new();
        enter_outputs(&handle, &client, self.shown.output.as_ref(), &mut entered);
        handle.done();
        self.handles.push((handle, entered));
    }

    fn update(&mut self, now: Shown) {
        let old = std::mem::replace(&mut self.shown, now);
        let new = &self.shown;
        for (handle, entered) in &mut self.handles {
            let mut changed = false;
            if old.title != new.title {
                handle.title(new.title.clone());
                changed = true;
            }
            if old.app_id != new.app_id {
                handle.app_id(new.app_id.clone());
                changed = true;
            }
            if old.states != new.states {
                handle.state(new.states_bytes());
                changed = true;
            }
            entered.retain(|o| o.is_alive());
            if old.output != new.output {
                for o in entered.drain(..) {
                    handle.output_leave(&o);
                }
                changed = true;
            }
            // A client may bind the output after the handle was made.
            if let Some(client) = handle.client() {
                changed |= enter_outputs(handle, &client, new.output.as_ref(), entered);
            }
            if changed {
                handle.done();
            }
        }
    }
}

/// Send `output_enter` for each of `client`'s `output` bindings not in
/// `entered` yet; whether any was sent.
fn enter_outputs(
    handle: &ZwlrForeignToplevelHandleV1,
    client: &Client,
    output: Option<&Output>,
    entered: &mut Vec<WlOutput>,
) -> bool {
    let Some(output) = output else {
        return false;
    };
    let mut sent = false;
    for o in output.client_outputs(client) {
        if !entered.contains(&o) {
            handle.output_enter(&o);
            entered.push(o);
            sent = true;
        }
    }
    sent
}

impl State {
    /// Bring every wlr foreign-toplevel handle up to date: new windows
    /// are announced, gone ones closed, changes sent. The focused window
    /// goes last, so a client sees the old one deactivated first.
    pub fn refresh_toplevels(&mut self) {
        if self.toplevels.wlr.managers.is_empty() && self.toplevels.wlr.windows.is_empty() {
            return;
        }
        if !std::mem::take(&mut self.toplevels.dirty) {
            return;
        }
        #[cfg(test)]
        {
            self.toplevels.refreshes += 1;
        }
        let mut now: Vec<(WindowId, Shown)> = self
            .windows
            .iter()
            .filter(|w| w.mapped && !w.override_redirect)
            .map(|w| {
                let r = w.frame_rect();
                let output =
                    self.output_at((r.x + r.width / 2) as f64, (r.y + r.height / 2) as f64);
                let shown = Shown {
                    title: w.title(),
                    app_id: w.app_id(),
                    states: wlr_states(
                        w.maximized,
                        w.hidden(),
                        self.focused == Some(w.id),
                        w.fullscreen,
                    ),
                    output,
                };
                (w.id, shown)
            })
            .collect();
        now.sort_by_key(|(w, _)| self.focused == Some(*w));
        let wlr = &mut self.toplevels.wlr;
        wlr.windows.retain(|w, data| {
            let keep = now.iter().any(|(n, _)| n == w);
            if !keep {
                for (h, _) in &data.handles {
                    h.closed();
                }
            }
            keep
        });
        for (w, shown) in now {
            match wlr.windows.get_mut(&w) {
                Some(data) if data.shown == shown => {}
                Some(data) => data.update(shown),
                None => {
                    let mut data = WlrWindow {
                        shown,
                        handles: Vec::new(),
                    };
                    for m in &wlr.managers {
                        data.add_handle(&wlr.dh, m, w);
                    }
                    wlr.windows.insert(w, data);
                }
            }
        }
    }

    /// A wlr foreign-toplevel client's request on window `w`.
    fn wlr_toplevel_request(
        &mut self,
        w: WindowId,
        request: zwlr_foreign_toplevel_handle_v1::Request,
    ) {
        use zwlr_foreign_toplevel_handle_v1::Request as R;
        if self.win(w).is_none_or(|x| !x.mapped) {
            return;
        }
        match request {
            R::SetMaximized => self.set_maximized(w, true),
            R::UnsetMaximized => self.set_maximized(w, false),
            R::SetMinimized => self.set_minimized(w, true),
            R::UnsetMinimized if !self.tabs.groups.is_hidden(w) => self.set_minimized(w, false),
            R::SetFullscreen { .. } if self.tabs.ask_fullscreen(w, true) => {
                self.set_fullscreen(w, true);
            }
            R::UnsetFullscreen if self.tabs.ask_fullscreen(w, false) => {
                self.set_fullscreen(w, false);
            }
            R::SetFullscreen { .. } | R::UnsetFullscreen => {}
            // A hidden tab reads as minimized; bringing it back shows it in
            // its group's frame.
            R::Activate { .. } | R::UnsetMinimized => {
                if self.win(w).is_some_and(|x| x.minimized && !x.tab_hidden) {
                    self.set_minimized(w, false);
                } else {
                    self.focus_window(w);
                }
                self.redraw_taskbars();
                self.redraw_all();
            }
            R::Close => self.close(w),
            _ => {}
        }
    }
}

impl GlobalDispatch2<ZwlrForeignToplevelManagerV1, State> for WlrGlobal {
    fn bind(
        &self,
        state: &mut State,
        dh: &DisplayHandle,
        _client: &Client,
        resource: New<ZwlrForeignToplevelManagerV1>,
        data_init: &mut DataInit<'_, State>,
    ) {
        let manager = data_init.init(resource, WlrGlobal);
        state.toplevels.dirty = true;
        let wlr = &mut state.toplevels.wlr;
        for (w, data) in &mut wlr.windows {
            data.add_handle(dh, &manager, *w);
        }
        wlr.managers.push(manager);
    }

    fn can_view(&self, client: &Client) -> bool {
        crate::sandbox::client_may_see(client, "zwlr_foreign_toplevel_manager_v1")
    }
}

impl Dispatch2<ZwlrForeignToplevelManagerV1, State> for WlrGlobal {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        resource: &ZwlrForeignToplevelManagerV1,
        request: zwlr_foreign_toplevel_manager_v1::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        if let zwlr_foreign_toplevel_manager_v1::Request::Stop = request {
            resource.finished();
            state.toplevels.wlr.managers.retain(|m| m != resource);
        }
    }

    fn destroyed(
        &self,
        state: &mut State,
        _client: ClientId,
        resource: &ZwlrForeignToplevelManagerV1,
    ) {
        state.toplevels.wlr.managers.retain(|m| m != resource);
    }
}

impl Dispatch2<ZwlrForeignToplevelHandleV1, State> for WindowId {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        _resource: &ZwlrForeignToplevelHandleV1,
        request: zwlr_foreign_toplevel_handle_v1::Request,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        state.wlr_toplevel_request(*self, request);
    }

    fn destroyed(
        &self,
        state: &mut State,
        _client: ClientId,
        resource: &ZwlrForeignToplevelHandleV1,
    ) {
        if let Some(data) = state.toplevels.wlr.windows.get_mut(self) {
            data.handles.retain(|(h, _)| h != resource);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zwlr_foreign_toplevel_handle_v1::State as S;

    #[test]
    fn states_list_the_set_flags_in_protocol_order() {
        assert_eq!(wlr_states(false, false, false, false), Vec::<u32>::new());
        assert_eq!(
            wlr_states(true, false, true, false),
            vec![S::Maximized as u32, S::Activated as u32]
        );
        assert_eq!(
            wlr_states(false, true, false, true),
            vec![S::Minimized as u32, S::Fullscreen as u32]
        );
    }

    #[test]
    fn idle_turns_with_a_manager_bound_do_no_refresh_work() {
        let mut h = crate::harness::Harness::new();
        h.output("o", 800, 600);
        let mut c = h.connect();
        let t = h.map_toplevel(&mut c, "app", 200, 100);
        let _manager = c.watch_toplevels();
        h.roundtrip(&mut c);
        assert!(
            h.state.toplevels.wlr.windows.contains_key(&t.id),
            "the bound manager hears of the window"
        );
        h.state.toplevels.refreshes = 0;
        for _ in 0..10 {
            h.dispatch();
        }
        assert_eq!(h.state.toplevels.refreshes, 0, "idle turns refresh nothing");
        h.state.set_maximized(t.id, true);
        h.dispatch();
        assert_eq!(h.state.toplevels.refreshes, 1, "a maximize refreshes once");
        assert_eq!(
            h.state.toplevels.wlr.windows[&t.id].shown.states,
            vec![S::Maximized as u32, S::Activated as u32]
        );
    }

    #[test]
    fn a_sandboxed_app_is_listed_while_it_has_a_window() {
        let mut h = crate::harness::Harness::new();
        h.output("o", 800, 600);
        let mut steam = h.connect_in_flatpak("com.valvesoftware.Steam");
        let mut idle = h.connect_in_flatpak("org.example.Sync");
        let mut host = h.connect();
        let t = h.map_toplevel(&mut steam, "steam", 200, 100);
        h.map_toplevel(&mut host, "foot", 200, 100);
        h.roundtrip(&mut idle);
        assert!(
            h.state.toplevels.sandboxed_apps().is_empty(),
            "nothing is looked up while no portal watches"
        );
        let mut portal = h.connect_portal();
        let _watcher = portal.watch_sandboxed();
        h.roundtrip(&mut portal);
        assert_eq!(
            h.state.toplevels.sandboxed_apps(),
            BTreeSet::from(["com.valvesoftware.Steam".to_string()])
        );
        assert_eq!(
            portal.side.sandboxed,
            vec![Some("com.valvesoftware.Steam".into()), None],
            "the first list holds the windows mapped before the watch"
        );
        steam.destroy(t);
        h.roundtrip(&mut steam);
        assert!(h.state.toplevels.sandboxed_apps().is_empty());
    }

    #[test]
    fn a_failed_lookup_is_retried_on_later_syncs_up_to_three_times() {
        let mut h = crate::harness::Harness::new();
        let w = WindowId::next();
        let t = &mut h.state.toplevels;
        t.unresolved.insert(w, 0);
        for _ in 0..LOOKUP_TRIES - 1 {
            t.resolved(w, Err(()));
            assert!(t.unresolved.contains_key(&w), "retried");
        }
        t.resolved(w, Err(()));
        assert!(
            !t.unresolved.contains_key(&w),
            "given up after {LOOKUP_TRIES}"
        );
        t.unresolved.insert(w, 1);
        t.resolved(w, Ok(Some("org.example.App".into())));
        assert!(!t.unresolved.contains_key(&w));
        assert_eq!(
            t.sandboxed_apps(),
            BTreeSet::from(["org.example.App".to_string()])
        );
    }

    #[test]
    fn the_portal_hears_a_sandboxed_app_come_and_go_with_its_window() {
        let mut h = crate::harness::Harness::new();
        h.output("o", 800, 600);
        let mut portal = h.connect_portal();
        let watcher = portal.watch_sandboxed();
        h.roundtrip(&mut portal);
        assert_eq!(portal.side.sandboxed, vec![None], "an empty set first");
        let mut app = h.connect_in_flatpak("org.example.App");
        let t = h.map_toplevel(&mut app, "app", 200, 100);
        h.roundtrip(&mut portal);
        app.destroy(t);
        h.roundtrip(&mut app);
        h.roundtrip(&mut portal);
        assert_eq!(
            portal.side.sandboxed,
            vec![None, Some("org.example.App".into()), None, None],
            "the app with its window, then an empty set once it is gone"
        );
        watcher.destroy();
        h.roundtrip(&mut portal);
        assert!(h.state.toplevels.sandbox_watchers.is_empty());
    }
}
