//! X11 programs through Xwayland: the X server, Reprise as its window
//! manager, and the clipboard between X11 and Wayland.
//!
//! Follows cosmic-comp's `src/xwayland.rs` (GPL-3.0): Smithay's
//! in-process `X11Wm`, started once Xwayland reports ready, so Reprise
//! keeps the X stacking order, override-redirect menus and icons in its
//! own hands. A managed X11 window becomes a `window::Window` of
//! `Kind::X11` when Xwayland ties a `wl_surface` to it
//! (`surface_associated`), maps on that surface's first buffer like an xdg
//! toplevel, and goes when the X window unmaps. Menus and tooltips
//! (override-redirect) are windows too, frameless at their own position.
//!
//! Xwayland draws at scale 1 and the scene scales its surfaces up on a
//! scaled output; X11 coordinates are layout coordinates.
//!
//! The X input focus and `_NET_WM_ICON` go through a second connection
//! on a thread of its own (`helper`).
//!
//! The server starts lazily: Reprise holds the display and its sockets
//! from login (`sockets`), so `DISPLAY` is good at once, and spawns
//! Xwayland on the first connection to them. Once no X11 program has been
//! connected for `helper::IDLE_GRACE` the server is stopped, and the
//! sockets wait for the next one. A server that dies after running
//! [`RESTART_AFTER`] waits on the same display likewise; one that dies
//! sooner is left down, so a server that cannot run is not started over
//! and over.

mod helper;
mod sockets;

use std::os::fd::OwnedFd;
use std::process::Stdio;
use std::time::{Duration, Instant};

use reprisekit::frame::Edges;
use smithay::{
    reexports::{
        calloop::{
            Interest, Mode, PostAction, RegistrationToken,
            channel::{self, Event as ChannelEvent},
            generic::Generic,
        },
        wayland_server::{DisplayHandle, protocol::wl_surface::WlSurface},
    },
    utils::{Logical, Rectangle},
    wayland::{
        selection::{
            SelectionTarget,
            data_device::{
                clear_data_device_selection, current_data_device_selection_userdata,
                request_data_device_client_selection, set_data_device_selection,
            },
            primary_selection::{
                clear_primary_selection, current_primary_selection_userdata,
                request_primary_client_selection, set_primary_selection,
            },
        },
        xwayland_shell::{XWaylandShellHandler, XWaylandShellState},
    },
    xwayland::{
        X11Surface, X11Wm, XWayland, XWaylandEvent, XwmHandler,
        xwm::{
            MwmDecorationsHint, MwmHints, Reorder, ResizeEdge, WmInputModel, WmWindowProperty,
            WmWindowType, X11Window, XwmId,
        },
    },
};

use crate::clipboard::{Owner, answer};
use crate::compositor::State;
use crate::window::{Kind, WindowId};

/// Xwayland and its window manager, kept on `State::xwayland`.
pub struct Xwayland {
    /// xwayland-shell-v1, through which Xwayland ties its surfaces to X
    /// windows; offered to Xwayland alone.
    shell: XWaylandShellState,
    /// The window manager, from Xwayland's ready signal until its
    /// connection drops.
    wm: Option<X11Wm>,
    /// The focus and icon connection, while the window manager runs.
    helper: Option<helper::Helper>,
    /// The Xwayland server's pid, which End Program never kills; 0 while
    /// none is known.
    pid: i32,
    /// The display, None once X11 programs cannot run.
    sockets: Option<sockets::Sockets>,
    /// The watches on the sockets while no server runs.
    watches: Vec<RegistrationToken>,
    /// The server's event source on the loop while one runs: removing it
    /// stops the server.
    source: Option<RegistrationToken>,
    /// When the last server was spawned (`RESTART_AFTER`).
    started: Option<Instant>,
    /// The mime types of the Wayland clipboard and primary selection, for
    /// a server that starts after they were set.
    selections: [Option<Vec<String>>; 2],
}

/// How long an X server must have run for its death to leave the display
/// waiting for the next X11 program.
const RESTART_AFTER: Duration = Duration::from_secs(5);

/// Whether a server that dies `ran` after its spawn is started again.
fn restarts(ran: Duration) -> bool {
    ran > RESTART_AFTER
}

fn slot(target: SelectionTarget) -> usize {
    match target {
        SelectionTarget::Clipboard => 0,
        SelectionTarget::Primary => 1,
    }
}

impl Xwayland {
    pub fn new(dh: &DisplayHandle) -> Xwayland {
        Xwayland {
            shell: XWaylandShellState::new::<State>(dh),
            wm: None,
            helper: None,
            pid: 0,
            sockets: None,
            watches: Vec::new(),
            source: None,
            started: None,
            selections: [None, None],
        }
    }

    /// The running window manager's id.
    pub fn wm_id(&self) -> Option<XwmId> {
        self.wm.as_ref().map(|wm| wm.id())
    }

    /// Read the running X11 drag's listed actions; they come back to
    /// `State::x11_drag_actions`.
    pub fn read_drag_actions(&self) {
        self.send(helper::Req::DragActions);
    }

    /// The Xwayland server's pid, 0 while none runs.
    pub fn pid(&self) -> i32 {
        self.pid
    }

    fn send(&self, req: helper::Req) {
        if let Some(h) = &self.helper {
            h.send(req);
        }
    }
}

/// Hold the first free display and wait for its first X11 program.
/// `DISPLAY` is known at once and goes into the session environment.
pub fn start(state: &mut State) {
    match sockets::Sockets::reserve(None) {
        Ok(s) => {
            log::info!("X11 display :{} waits for its first program", s.display());
            state.env.x11_display = Some(format!(":{}", s.display()));
            state.xwayland.sockets = Some(s);
            watch(state);
        }
        Err(e) => log::warn!("no X11 display; X11 programs will not run: {e}"),
    }
}

/// Watch the display's sockets: the first connection spawns Xwayland.
fn watch(state: &mut State) {
    let fds = match state.xwayland.sockets.as_ref().map(|s| s.watch_fds()) {
        Some(Ok(fds)) => fds,
        Some(Err(e)) => return give_up(state, &e.to_string()),
        None => return,
    };
    for fd in fds {
        let source = Generic::new(fd, Interest::READ, Mode::Level);
        let inserted = state.handle.insert_source(source, |_, _, state| {
            // `spawn` takes the watches off before the loop polls again;
            // both sockets may fire in one turn, and it runs once.
            state.handle.insert_idle(spawn);
            Ok(PostAction::Continue)
        });
        match inserted {
            Ok(token) => state.xwayland.watches.push(token),
            Err(e) => return give_up(state, &e.to_string()),
        }
    }
}

/// An X11 program connected: spawn Xwayland with the display's sockets,
/// so it answers the program.
fn spawn(state: &mut State) {
    if state.xwayland.watches.is_empty() {
        return;
    }
    for token in state.xwayland.watches.drain(..) {
        state.handle.remove(token);
    }
    let Some(sockets) = state.xwayland.sockets.as_mut() else {
        return;
    };
    let display = sockets.display();
    let handed = match sockets.hand_over() {
        Ok(h) => h,
        Err(e) => return give_up(state, &e.to_string()),
    };
    let spawned = crate::spawn::with_signals_unblocked(|| {
        XWayland::spawn(
            &state.dh,
            display,
            std::iter::empty::<(&str, &str)>(),
            handed.args(),
            false,
            Stdio::null(),
            Stdio::inherit(),
            |_| {},
        )
    });
    drop(handed);
    let (xwayland, client) = match spawned {
        Ok(x) => x,
        Err(e) => return give_up(state, &e.to_string()),
    };
    log::info!("an X11 program connected; starting Xwayland on :{display}");
    state.xwayland.started = Some(Instant::now());
    let inserted = state
        .handle
        .insert_source(xwayland, move |event, _, state| match event {
            XWaylandEvent::Ready {
                x11_socket,
                display_number,
            } => {
                match X11Wm::start_wm(state.handle.clone(), &state.dh, x11_socket, client.clone()) {
                    Ok(wm) => {
                        log::info!("xwayland ready on DISPLAY=:{display_number}");
                        state.xwayland.wm = Some(wm);
                        state.xwayland.pid = xwayland_pid().unwrap_or(0);
                        state.xwayland.helper = start_helper(state, display_number);
                        state.selections_to_new_x11();
                    }
                    Err(e) => log::warn!("X11 window manager: {e}"),
                }
            }
            XWaylandEvent::Error => {
                log::warn!("Xwayland exited while starting");
                state.handle.insert_idle(restart);
            }
        });
    match inserted {
        Ok(token) => state.xwayland.source = Some(token),
        Err(e) => give_up(state, &e.to_string()),
    }
}

/// No X11 program has been connected for a while: stop the server. Its
/// window manager hears it go (`disconnected`), and the display waits
/// for the next program (`restart`).
///
/// A signal, not dropping Smithay's handle: that only marks Xwayland's
/// Wayland connection dead, and the connection closes on the next client
/// request Reprise reads, which an idle desktop may not send for long.
fn stop(state: &mut State) {
    let pid = state.xwayland.pid;
    if state.xwayland.source.is_none() || pid <= 0 {
        return;
    }
    log::info!(
        "no X11 program for {:?}; stopping Xwayland",
        helper::IDLE_GRACE
    );
    state.xwayland.helper = None;
    // SAFETY: plain kill(2) on the server's own pid, which Smithay reaps.
    if unsafe { libc::kill(pid, libc::SIGTERM) } != 0 {
        log::warn!("stopping Xwayland: {}", std::io::Error::last_os_error());
    }
}

/// After the X server went: hold the display again and wait for the next
/// program, unless it died within `RESTART_AFTER` of its start.
fn restart(state: &mut State) {
    if let Some(token) = state.xwayland.source.take() {
        state.handle.remove(token);
    }
    if !state.xwayland.watches.is_empty() || state.xwayland.sockets.is_none() {
        return;
    }
    let ran = state.xwayland.started.map(|t| t.elapsed());
    if !ran.is_some_and(restarts) {
        return give_up(
            state,
            &format!("Xwayland died within {RESTART_AFTER:?} of its start"),
        );
    }
    let Some(sockets) = state.xwayland.sockets.as_mut() else {
        return;
    };
    if let Err(e) = sockets.take_back() {
        log::warn!(
            "X11 display :{} is taken ({e}); trying another",
            sockets.display()
        );
        state.xwayland.sockets = None;
        state.env.x11_display = None;
        start(state);
        crate::spawn::export_session_env(&state.env);
        return;
    }
    watch(state);
}

/// X11 programs cannot run: let go of the display and take `DISPLAY` out
/// of the session.
fn give_up(state: &mut State, why: &str) {
    log::warn!("{why}; X11 programs will not run");
    for token in state.xwayland.watches.drain(..) {
        state.handle.remove(token);
    }
    state.xwayland.sockets = None;
    state.env.x11_display = None;
    crate::spawn::export_session_env(&state.env);
}

/// The pid of the Xwayland server Smithay spawned: our child named
/// `Xwayland` (Smithay keeps the `Child` to itself).
fn xwayland_pid() -> Option<i32> {
    let own = std::process::id() as i32;
    std::fs::read_dir("/proc")
        .ok()?
        .filter_map(|e| e.ok()?.file_name().to_str()?.parse::<i32>().ok())
        .find(|pid| {
            std::fs::read_to_string(format!("/proc/{pid}/stat"))
                .ok()
                .and_then(|s| {
                    parent_and_name(&s).map(|(ppid, name)| ppid == own && name == "Xwayland")
                })
                .unwrap_or(false)
        })
}

/// A `/proc/PID/stat` line's parent pid and command name. The name sits
/// in parentheses and may hold spaces and parentheses itself, so it ends
/// at the last `)`.
fn parent_and_name(stat: &str) -> Option<(i32, &str)> {
    let open = stat.find('(')?;
    let close = stat.rfind(')')?;
    let name = stat.get(open + 1..close)?;
    let mut rest = stat.get(close + 1..)?.split_whitespace();
    let _state = rest.next()?;
    let ppid = rest.next()?.parse().ok()?;
    Some((ppid, name))
}

/// The helper thread and the loop source its answers come back on.
fn start_helper(state: &mut State, display: u32) -> Option<helper::Helper> {
    let (tx, rx) = channel::channel();
    let source = state
        .handle
        .insert_source(rx, |event, _, state| match event {
            ChannelEvent::Msg(helper::Back::Icon(window, icon)) => {
                state.x11_icon_arrived(window, icon);
            }
            ChannelEvent::Msg(helper::Back::Idle) => stop(state),
            ChannelEvent::Msg(helper::Back::DragActions(actions)) => {
                state.x11_drag_actions(actions);
            }
            ChannelEvent::Closed => {}
        });
    if let Err(e) = source {
        log::warn!("X11 icons: {e}");
        return None;
    }
    match helper::Helper::spawn(display, tx) {
        Ok(h) => Some(h),
        Err(e) => {
            log::warn!("X11 focus connection: {e}");
            None
        }
    }
}

/// Whether an X11 window's Motif hints leave the frame to us: a window
/// that drops the border or the title draws its own chrome.
pub fn decorated(hints: &MwmHints) -> bool {
    hints.decorations.is_none_or(|d| {
        d.contains(MwmDecorationsHint::ALL)
            || d.contains(MwmDecorationsHint::BORDER | MwmDecorationsHint::TITLE)
    })
}

/// Whether an override-redirect window takes the focus when it maps or
/// is clicked: not a menu, tooltip, notification or the like.
pub fn override_redirect_wants_focus(kind: Option<WmWindowType>) -> bool {
    !matches!(
        kind,
        Some(
            WmWindowType::Combo
                | WmWindowType::Desktop
                | WmWindowType::Dnd
                | WmWindowType::DropdownMenu
                | WmWindowType::Menu
                | WmWindowType::Notification
                | WmWindowType::PopupMenu
                | WmWindowType::Splash
                | WmWindowType::Tooltip
                | WmWindowType::Utility
        )
    )
}

/// The frame edges a `_NET_WM_MOVERESIZE` edge pulls.
fn edges(e: ResizeEdge) -> Edges {
    match e {
        ResizeEdge::Top => Edges::TOP,
        ResizeEdge::Bottom => Edges::BOTTOM,
        ResizeEdge::Left => Edges::LEFT,
        ResizeEdge::Right => Edges::RIGHT,
        ResizeEdge::TopLeft => Edges::TOP | Edges::LEFT,
        ResizeEdge::TopRight => Edges::TOP | Edges::RIGHT,
        ResizeEdge::BottomLeft => Edges::BOTTOM | Edges::LEFT,
        ResizeEdge::BottomRight => Edges::BOTTOM | Edges::RIGHT,
    }
}

impl State {
    /// The window of X11 window `x`, mapped or not.
    pub fn window_for_x11(&self, x: &X11Surface) -> Option<WindowId> {
        self.window_for_x11_id(x.window_id())
    }

    fn window_for_x11_id(&self, id: X11Window) -> Option<WindowId> {
        self.windows
            .iter()
            .find(|w| w.x11().is_some_and(|x| x.window_id() == id))
            .map(|w| w.id)
    }

    fn mapped_x11(&self, x: &X11Surface) -> Option<WindowId> {
        self.window_for_x11(x)
            .filter(|&w| self.win(w).is_some_and(|w| w.mapped))
    }

    /// Whether the active window is an X11 one: only then may X11 clients
    /// take or read the clipboard.
    fn x11_focused(&self) -> bool {
        self.focused
            .and_then(|w| self.win(w))
            .is_some_and(|w| w.x11().is_some())
    }

    /// Give X11 window `x` the X input focus and the top of the X
    /// stacking order, or take the focus back. X keeps menus and tooltips
    /// above the managed windows itself.
    pub fn activate_x11(&mut self, x: &X11Surface, on: bool) {
        if let Err(e) = x.set_activated(on) {
            log::debug!("x11 activate: {e}");
        }
        if !on {
            self.xwayland.send(helper::Req::Unfocus(x.window_id()));
            return;
        }
        if !x.is_override_redirect()
            && let Some(wm) = self.xwayland.wm.as_mut()
            && let Err(e) = wm.raise_window(x)
        {
            log::debug!("x11 raise: {e}");
        }
        let (input, take) = match x.input_model() {
            WmInputModel::None => return,
            WmInputModel::Passive => (true, false),
            WmInputModel::LocallyActive => (true, true),
            WmInputModel::GloballyActive => (false, true),
        };
        self.xwayland.send(helper::Req::Focus {
            window: x.window_id(),
            input,
            take,
        });
    }

    /// Write the window's maximized and fullscreen state back to its X11
    /// client, so one whose request was refused learns it.
    pub fn write_x11_state(&self, w: WindowId) {
        let Some(win) = self.win(w) else {
            return;
        };
        let Some(x) = win.x11() else {
            return;
        };
        let written = x
            .set_maximized(win.maximized)
            .and_then(|()| x.set_fullscreen(win.fullscreen));
        if let Err(e) = written {
            log::debug!("x11 state: {e}");
        }
    }

    /// The helper read X11 window `window`'s `_NET_WM_ICON`.
    fn x11_icon_arrived(&mut self, window: X11Window, icon: Option<reprisekit::icon::IconSource>) {
        let Some(w) = self.window_for_x11_id(window) else {
            return;
        };
        let changed = self.win_mut(w).is_some_and(|win| win.set_icon(icon));
        if changed && self.win(w).is_some_and(|w| w.mapped) {
            self.retitled(w, crate::retitle::Changes::ICON);
        }
    }

    /// A Wayland client (or Reprise) set the selection offering
    /// `mime_types`, None when it cleared it: X11 clients see it too.
    pub fn wayland_selection_to_x11(
        &mut self,
        target: SelectionTarget,
        mime_types: Option<Vec<String>>,
    ) {
        self.xwayland.selections[slot(target)].clone_from(&mime_types);
        if let Some(wm) = self.xwayland.wm.as_mut()
            && let Err(e) = wm.new_selection(target, mime_types)
        {
            log::warn!("X11 clipboard: {e}");
        }
    }

    /// A server that just started learns the Wayland selections set
    /// before it.
    fn selections_to_new_x11(&mut self) {
        for target in [SelectionTarget::Clipboard, SelectionTarget::Primary] {
            if let Some(mimes) = self.xwayland.selections[slot(target)].clone() {
                self.wayland_selection_to_x11(target, Some(mimes));
            }
        }
    }

    /// A Wayland client reads the selection an X11 client set.
    pub fn x11_selection_to_wayland(&mut self, target: SelectionTarget, mime: String, fd: OwnedFd) {
        if let Some(wm) = self.xwayland.wm.as_mut()
            && let Err(e) = wm.send_selection(target, mime, fd)
        {
            log::warn!("X11 clipboard: {e}");
        }
    }

    /// An X11 client asked for `window`'s state `on` or off: applied when
    /// its window is mapped, else kept in its `_NET_WM_STATE` for
    /// `map_window` to read.
    fn x11_maximize(&mut self, x: &X11Surface, on: bool) {
        match self.mapped_x11(x) {
            Some(w) => {
                self.set_maximized(w, on);
                self.write_x11_state(w);
            }
            None => {
                if let Err(e) = x.set_maximized(on) {
                    log::debug!("x11 maximized: {e}");
                }
            }
        }
    }

    fn x11_fullscreen(&mut self, x: &X11Surface, on: bool) {
        match self.mapped_x11(x) {
            Some(w) => {
                self.set_fullscreen(w, on);
                self.write_x11_state(w);
            }
            None => {
                if let Err(e) = x.set_fullscreen(on) {
                    log::debug!("x11 fullscreen: {e}");
                }
            }
        }
    }

    /// Whether an X11 `_NET_WM_MOVERESIZE` for `w` may start a move or
    /// resize. The message carries no serial, so it needs a button down on
    /// the seat and `w` holding the focus.
    fn x11_grab_allowed(&self, w: WindowId) -> bool {
        self.buttons_down > 0 && self.focused == Some(w)
    }
}

impl XWaylandShellHandler for State {
    fn xwayland_shell_state(&mut self) -> &mut XWaylandShellState {
        &mut self.xwayland.shell
    }

    /// Xwayland draws X11 window `x` into `wl`: it becomes a window, which
    /// maps on `wl`'s first buffer. Its frame follows its Motif hints, and
    /// its icon is fetched.
    fn surface_associated(&mut self, _xwm: XwmId, wl: WlSurface, x: X11Surface) {
        if let Some(old) = self.window_for_x11(&x) {
            self.destroy_window(old);
        }
        let or = x.is_override_redirect();
        let csd = !or && !decorated(&x.motif_hints());
        let id = x.window_id();
        let w = self.add_window(Kind::X11(Box::new(x)), wl);
        if let Some(win) = self.win_mut(w) {
            win.override_redirect = or;
            win.csd = csd;
        }
        self.xwayland.send(helper::Req::Icon(id));
        // The class and title are known now and the surface has no buffer
        // yet: the first one has the size the rules or the app's memory
        // give it.
        if !or {
            self.configure_initial(w);
        }
        // The tie can come after the surface already has its buffer.
        self.handle_commit(w);
    }
}

/// The size an X11 window asking for `want` gets: each side from 1 px to
/// twice the layout's side (`layout`), but never under 4096 or over
/// 16384 px. Xwayland allocates the whole window, so a client asking for
/// 32767x32767 would otherwise take gigabytes.
fn cap_x11_size(want: (i32, i32), layout: (i32, i32)) -> (i32, i32) {
    let limit = |l: i32| l.saturating_mul(2).clamp(4096, 16384);
    (
        want.0.clamp(1, limit(layout.0)),
        want.1.clamp(1, limit(layout.1)),
    )
}

impl XwmHandler for State {
    fn xwm_state(&mut self, _xwm: XwmId) -> &mut X11Wm {
        self.xwayland
            .wm
            .as_mut()
            .expect("the window manager runs while it sends events")
    }

    fn new_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    fn new_override_redirect_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    /// Every managed window may map; `map_window` places it on its first
    /// buffer. A window created larger than `cap_x11_size` allows is
    /// shrunk first, before Xwayland allocates its picture.
    fn map_window_request(&mut self, _xwm: XwmId, window: X11Surface) {
        let geo = window.geometry();
        let area = self.output_area();
        let (w, h) = cap_x11_size((geo.size.w, geo.size.h), (area.size.w, area.size.h));
        if (w, h) != (geo.size.w, geo.size.h) {
            log::info!(
                "x11 window {}: {}x{} capped to {w}x{h}",
                window.window_id(),
                geo.size.w,
                geo.size.h
            );
            if let Err(e) = window.configure(Rectangle::new(geo.loc, (w, h).into())) {
                log::debug!("x11 configure: {e}");
            }
        }
        if let Err(e) = window.set_mapped(true) {
            log::warn!("x11 map: {e}");
        }
    }

    fn mapped_override_redirect_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    /// Xwayland drops the surface of an unmapped window and draws a new
    /// one when it maps again, so the window goes with it.
    fn unmapped_window(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(w) = self.window_for_x11(&window) {
            self.destroy_window(w);
        }
    }

    fn destroyed_window(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(w) = self.window_for_x11(&window) {
            self.destroy_window(w);
        }
    }

    /// We own the position of a mapped window; only its size is
    /// negotiable, and not while it fills its output. Before the map the
    /// client gets what it asks for, within `cap_x11_size`. Smithay answers
    /// with a configure of the result either way.
    fn configure_request(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        x: Option<i32>,
        y: Option<i32>,
        w: Option<u32>,
        h: Option<u32>,
        _reorder: Option<Reorder>,
    ) {
        let last = window.last_configure();
        let size = |v: Option<u32>, cur: i32| v.map_or(cur, |v| i32::try_from(v).unwrap_or(cur));
        let area = self.output_area();
        let (cw, ch) = cap_x11_size(
            (size(w, last.size.w), size(h, last.size.h)),
            (area.size.w, area.size.h),
        );
        match self.mapped_x11(&window) {
            Some(id) => {
                let fitted = self.win(id).is_some_and(|w| w.maximized || w.fullscreen);
                if !fitted && (cw, ch) != (last.size.w, last.size.h) {
                    self.request_size(id, cw, ch);
                }
            }
            None => {
                let rect: Rectangle<i32, Logical> = Rectangle::new(
                    (x.unwrap_or(last.loc.x), y.unwrap_or(last.loc.y)).into(),
                    (cw, ch).into(),
                );
                if let Err(e) = window.configure(rect) {
                    log::debug!("x11 configure: {e}");
                }
            }
        }
    }

    /// A menu or tooltip moves itself; follow it.
    fn configure_notify(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        geometry: Rectangle<i32, Logical>,
        _above: Option<X11Window>,
    ) {
        if !window.is_override_redirect() {
            return;
        }
        let Some(w) = self.mapped_x11(&window) else {
            return;
        };
        if let Some(win) = self.win_mut(w) {
            (win.x, win.y) = (geometry.loc.x, geometry.loc.y);
        }
        self.relayout(w);
        self.redraw_all();
    }

    fn property_notify(&mut self, _xwm: XwmId, window: X11Surface, property: WmWindowProperty) {
        let Some(w) = self.window_for_x11(&window) else {
            return;
        };
        match property {
            WmWindowProperty::Title | WmWindowProperty::Class => self.note_title(w),
            WmWindowProperty::MotifHints if !window.is_override_redirect() => {
                self.set_csd(w, !decorated(&window.motif_hints()));
                self.redraw_all();
            }
            WmWindowProperty::TransientFor => {
                self.on_parent_changed(w);
                self.redraw_taskbars();
                self.redraw_all();
            }
            // New size hints can make a window fixed-size: its chrome
            // follows.
            WmWindowProperty::NormalHints => {
                if self.win(w).is_some_and(|w| w.mapped) {
                    self.relayout(w);
                    self.redraw_all();
                }
            }
            WmWindowProperty::Other(atom) => self
                .xwayland
                .send(helper::Req::Property(window.window_id(), atom)),
            _ => {}
        }
    }

    fn maximize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        self.x11_maximize(&window, true);
    }

    fn unmaximize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        self.x11_maximize(&window, false);
    }

    fn fullscreen_request(&mut self, _xwm: XwmId, window: X11Surface) {
        self.x11_fullscreen(&window, true);
    }

    fn unfullscreen_request(&mut self, _xwm: XwmId, window: X11Surface) {
        self.x11_fullscreen(&window, false);
    }

    fn minimize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(w) = self.mapped_x11(&window) {
            self.set_minimized(w, true);
        }
    }

    fn unminimize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(w) = self.mapped_x11(&window) {
            self.set_minimized(w, false);
        }
    }

    fn resize_request(&mut self, _xwm: XwmId, window: X11Surface, _button: u32, edge: ResizeEdge) {
        if let Some(w) = self.mapped_x11(&window)
            && self.x11_grab_allowed(w)
        {
            self.begin_resize(w, edges(edge));
        }
    }

    fn move_request(&mut self, _xwm: XwmId, window: X11Surface, _button: u32) {
        if let Some(w) = self.mapped_x11(&window)
            && self.x11_grab_allowed(w)
        {
            self.begin_move(w, true);
        }
    }

    /// An X11 client reads the Wayland selections only while an X11
    /// window is active.
    fn allow_selection_access(&mut self, _xwm: XwmId, _selection: SelectionTarget) -> bool {
        self.x11_focused()
    }

    /// An X11 client reads a Wayland selection: a client's is asked of
    /// it, Reprise's own is written from its offers.
    fn send_selection(
        &mut self,
        _xwm: XwmId,
        selection: SelectionTarget,
        mime_type: String,
        fd: OwnedFd,
    ) {
        let own = match selection {
            SelectionTarget::Clipboard => current_data_device_selection_userdata(&self.seat)
                .and_then(|o| match &*o {
                    Owner::Reprise(offers) => answer(offers, &mime_type).cloned(),
                    Owner::X11(_) => None,
                }),
            SelectionTarget::Primary => {
                current_primary_selection_userdata(&self.seat).and_then(|o| match &*o {
                    Owner::Reprise(offers) => answer(offers, &mime_type).cloned(),
                    Owner::X11(_) => None,
                })
            }
        };
        if let Some(bytes) = own {
            reprisefs::pipe::write_detached(fd, bytes);
            return;
        }
        let read = match selection {
            SelectionTarget::Clipboard => {
                request_data_device_client_selection(&self.seat, mime_type, fd)
                    .map_err(|e| e.to_string())
            }
            SelectionTarget::Primary => request_primary_client_selection(&self.seat, mime_type, fd)
                .map_err(|e| e.to_string()),
        };
        if let Err(e) = read {
            log::debug!("X11 reads the {selection:?} selection: {e}");
        }
    }

    /// An X11 client took a selection: Wayland clients get it, while an
    /// X11 window is active, as from any focused client.
    fn new_selection(&mut self, xwm: XwmId, selection: SelectionTarget, mime_types: Vec<String>) {
        if !self.x11_focused() {
            return;
        }
        self.xwayland.selections[slot(selection)] = None;
        match selection {
            SelectionTarget::Clipboard => {
                self.clip.offered.clone_from(&mime_types);
                set_data_device_selection(&self.dh, &self.seat, mime_types, Owner::X11(xwm));
                self.clipboard_changed();
            }
            SelectionTarget::Primary => {
                set_primary_selection(&self.dh, &self.seat, mime_types, Owner::X11(xwm));
            }
        }
    }

    fn cleared_selection(&mut self, xwm: XwmId, selection: SelectionTarget) {
        match selection {
            SelectionTarget::Clipboard => {
                if current_data_device_selection_userdata(&self.seat).is_some_and(|o| o.is_x11(xwm))
                {
                    self.clip.offered.clear();
                    clear_data_device_selection(&self.dh, &self.seat);
                    self.clipboard_changed();
                }
            }
            SelectionTarget::Primary => {
                if current_primary_selection_userdata(&self.seat).is_some_and(|o| o.is_x11(xwm)) {
                    clear_primary_selection(&self.dh, &self.seat);
                }
            }
        }
    }

    /// An X11 window answered its `_NET_WM_PING` (`hang`).
    fn ping_acked(&mut self, _xwm: XwmId, window: X11Surface, _timestamp: u32) {
        self.x11_pong(window.window_id());
    }

    /// The X server went away: its windows and the selections its
    /// programs held with it. The display waits for the next program from
    /// the loop's next turn (`restart`).
    fn disconnected(&mut self, xwm: XwmId) {
        log::info!("the X11 window manager lost Xwayland");
        self.cleared_selection(xwm, SelectionTarget::Clipboard);
        self.cleared_selection(xwm, SelectionTarget::Primary);
        self.xwayland.wm = None;
        self.xwayland.helper = None;
        self.xwayland.pid = 0;
        let gone: Vec<WindowId> = self
            .windows
            .iter()
            .filter(|w| w.x11().is_some())
            .map(|w| w.id)
            .collect();
        for w in gone {
            self.destroy_window(w);
        }
        self.handle.insert_idle(restart);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_x11_window_is_capped_to_twice_the_layout() {
        let layout = (2560, 1600);
        assert_eq!(
            cap_x11_size((800, 600), layout),
            (800, 600),
            "an ordinary size passes"
        );
        assert_eq!(
            cap_x11_size((32767, 32767), layout),
            (5120, 4096),
            "twice the layout, at least 4096"
        );
        assert_eq!(cap_x11_size((0, -5), layout), (1, 1), "at least one pixel");
        assert_eq!(
            cap_x11_size((40000, 40000), (10000, 10000)),
            (16384, 16384),
            "never over 16384"
        );
        assert_eq!(
            cap_x11_size((5000, 5000), (1, 1)),
            (4096, 4096),
            "no outputs yet: 4096"
        );
    }

    #[test]
    fn a_server_that_dies_soon_after_its_start_is_left_down() {
        assert!(!restarts(Duration::from_secs(1)), "a crash at startup");
        assert!(!restarts(RESTART_AFTER), "the limit itself");
        assert!(restarts(Duration::from_secs(60)), "a server that ran");
    }

    #[test]
    fn a_stat_line_gives_the_parent_and_the_name() {
        assert_eq!(
            parent_and_name("4242 (Xwayland) S 977 4242 977 0 -1 4194560"),
            Some((977, "Xwayland"))
        );
        assert_eq!(
            parent_and_name("12 (a (b) c) R 1 12 12"),
            Some((1, "a (b) c")),
            "the name ends at the last parenthesis"
        );
        assert_eq!(parent_and_name("12 (cut"), None);
    }

    /// Motif hints: dropping either the border or the title leaves the
    /// chrome to the client; no hint, or ALL, keeps ours.
    #[test]
    fn a_window_without_border_or_title_draws_its_own_chrome() {
        let hints = |d: Option<MwmDecorationsHint>| MwmHints {
            decorations: d,
            ..Default::default()
        };
        assert!(decorated(&hints(None)));
        assert!(decorated(&hints(Some(MwmDecorationsHint::ALL))));
        assert!(decorated(&hints(Some(
            MwmDecorationsHint::BORDER | MwmDecorationsHint::TITLE | MwmDecorationsHint::MENU
        ))));
        assert!(!decorated(&hints(Some(MwmDecorationsHint::TITLE))));
        assert!(!decorated(&hints(Some(MwmDecorationsHint::BORDER))));
        assert!(!decorated(&hints(Some(MwmDecorationsHint::empty()))));
    }

    #[test]
    fn menus_and_tooltips_never_take_the_focus() {
        for t in [
            WmWindowType::Menu,
            WmWindowType::PopupMenu,
            WmWindowType::DropdownMenu,
            WmWindowType::Tooltip,
            WmWindowType::Combo,
            WmWindowType::Notification,
        ] {
            assert!(!override_redirect_wants_focus(Some(t)), "{t:?}");
        }
        assert!(override_redirect_wants_focus(None));
        assert!(override_redirect_wants_focus(Some(WmWindowType::Normal)));
        assert!(override_redirect_wants_focus(Some(WmWindowType::Dialog)));
    }

    #[test]
    fn a_resize_edge_pulls_its_frame_edges() {
        assert_eq!(edges(ResizeEdge::Left), Edges::LEFT);
        assert_eq!(edges(ResizeEdge::BottomRight), Edges::BOTTOM | Edges::RIGHT);
        assert_eq!(edges(ResizeEdge::TopLeft), Edges::TOP | Edges::LEFT);
    }
}
