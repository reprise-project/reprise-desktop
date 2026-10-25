//! The protocol handlers: one trait impl on `State` per Smithay module.

use smithay::{
    backend::{allocator::dmabuf::Dmabuf, renderer::utils::on_commit_buffer_handler},
    delegate_dispatch2,
    desktop::{
        PopupGrab, PopupKeyboardGrab, PopupKind, PopupUngrabStrategy, find_popup_root_surface,
    },
    input::{
        Seat, SeatHandler, SeatState,
        pointer::{CursorImageStatus, Focus},
    },
    reexports::{
        wayland_protocols::xdg::{
            decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode as DecorationMode,
            shell::server::xdg_toplevel,
        },
        wayland_protocols_misc::server_decoration::server::org_kde_kwin_server_decoration::{
            Mode as KdeMode, OrgKdeKwinServerDecoration,
        },
        wayland_server::{
            Client, Resource, WEnum,
            protocol::{
                wl_buffer::WlBuffer, wl_output::WlOutput, wl_seat::WlSeat, wl_surface::WlSurface,
            },
        },
    },
    utils::Serial,
    wayland::{
        buffer::BufferHandler,
        compositor::{
            CompositorClientState, CompositorHandler, CompositorState, get_parent,
            is_sync_subsurface, with_states,
        },
        dmabuf::{DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier},
        output::OutputHandler,
        selection::{data_device::set_data_device_focus, primary_selection::set_primary_focus},
        shell::kde::decoration::{KdeDecorationHandler, KdeDecorationState},
        shell::xdg::{
            PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
            decoration::XdgDecorationHandler,
            dialog::{ToplevelDialogHint, XdgDialogHandler},
        },
        shm::{ShmHandler, ShmState},
        xdg_toplevel_tag::XdgToplevelTagHandler,
    },
    xwayland::XWaylandClientData,
};

use crate::compositor::{ClientState, State};
use crate::popupgrab::PopupPointer;
use crate::window::WindowId;

impl CompositorHandler for State {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        if let Some(x) = client.get_data::<XWaylandClientData>() {
            return &x.compositor_state;
        }
        &client
            .get_data::<ClientState>()
            .expect("every client but Xwayland is inserted with ClientState")
            .compositor_state
    }

    fn new_surface(&mut self, surface: &WlSurface) {
        crate::surfaceproto::on_new_surface(surface);
    }

    fn commit(&mut self, surface: &WlSurface) {
        self.closing_commit(surface);
        // Before the buffer handler takes the new buffer.
        let (pixels, tree_kept) = crate::samepixels::check(surface);
        let view = crate::samepixels::view(surface);
        on_commit_buffer_handler::<Self>(surface);
        if let Some(b) = self.backend.as_mut() {
            b.shm_textures.committed(surface);
        }
        let view_kept = view == crate::samepixels::view(surface);
        let mut top = surface.clone();
        while let Some(parent) = get_parent(&top) {
            top = parent;
        }
        let window = self.window_for_surface(&top);
        let mark = window.and_then(|w| self.window_mark(w));
        // A synchronized subsurface's state applies with its parent's, whose own commit handles
        // the window; any other subsurface's commit can change the window's size too.
        if !is_sync_subsurface(surface)
            && let Some(w) = window
        {
            self.handle_commit(w);
            crate::capture::window_committed(self, w);
        }
        let window_kept =
            mark.is_some_and(|m| m.mapped) && mark == window.and_then(|w| self.window_mark(w));
        self.popups.commit(surface);
        if let Some(PopupKind::Xdg(popup)) = self.popups.find_popup(surface)
            && !popup.is_initial_configure_sent()
            && let Err(e) = popup.send_configure()
        {
            log::warn!("popup configure: {e}");
        }
        if let Some(w) = self.window_for_popup(surface) {
            self.sync_popups(w);
        }
        crate::layer::commit(self, surface);
        crate::sessionlock::commit(self, surface);
        self.ime_commit(surface);
        // Layer, lock and IME popup surfaces have no window mark, so they always redraw.
        let commit = crate::samepixels::Commit {
            pixels,
            view_kept,
            tree_kept,
            window_kept,
        };
        if commit.shows_nothing_new() {
            crate::backend::answer_frames(self);
        } else {
            self.redraw_surface(&top);
        }
    }

    fn destroyed(&mut self, surface: &WlSurface) {
        self.closing_destroy(surface);
    }
}

impl BufferHandler for State {
    fn buffer_destroyed(&mut self, _buffer: &WlBuffer) {}
}

impl ShmHandler for State {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

impl DmabufHandler for State {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        &mut self
            .backend
            .as_mut()
            .and_then(|b| b.dmabuf.as_mut())
            .expect("the dmabuf global exists only with the backend")
            .0
    }

    fn dmabuf_imported(
        &mut self,
        _global: &DmabufGlobal,
        dmabuf: Dmabuf,
        notifier: ImportNotifier,
    ) {
        if self
            .backend
            .as_mut()
            .is_some_and(|b| b.import_dmabuf(&dmabuf))
        {
            let _ = notifier.successful::<State>();
        } else {
            notifier.failed();
        }
    }
}

impl SeatHandler for State {
    type KeyboardFocus = WlSurface;
    type PointerFocus = crate::xdnd::PointerFocus;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<State> {
        &mut self.seat_state
    }

    fn cursor_image(&mut self, _seat: &Seat<Self>, image: CursorImageStatus) {
        self.cursor = image;
        self.redraw_cursor();
    }

    fn focus_changed(&mut self, seat: &Seat<Self>, focused: Option<&WlSurface>) {
        let client = focused.and_then(|s| self.dh.get_client(s.id()).ok());
        if let Some(c) = &client {
            crate::textinput::peek_text_inputs(self, c);
        }
        set_data_device_focus(&self.dh, seat, client.clone());
        set_primary_focus(&self.dh, seat, client);
        self.refresh_inhibitors(focused);
        self.ime_focus_changed(focused);
        // Checked once the focus change is done, outside the seat's locks.
        self.insert_idle_woken(|state| state.refresh_constraint());
    }
}

impl smithay::wayland::selection::ext_data_control::DataControlHandler for State {
    fn data_control_state(
        &mut self,
    ) -> &mut smithay::wayland::selection::ext_data_control::DataControlState {
        &mut self.ext_data_control_state
    }
}

impl smithay::wayland::selection::wlr_data_control::DataControlHandler for State {
    fn data_control_state(
        &mut self,
    ) -> &mut smithay::wayland::selection::wlr_data_control::DataControlState {
        &mut self.wlr_data_control_state
    }
}

impl OutputHandler for State {}

impl XdgShellHandler for State {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn client_pong(&mut self, client: smithay::wayland::shell::xdg::ShellClient) {
        self.pong(client);
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        surface.with_pending_state(|s| s.decoration_mode = Some(DecorationMode::ServerSide));
        let csd = kde_client_side(surface.wl_surface());
        let w = self.new_window(surface);
        if csd {
            self.set_csd(w, true);
        }
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        if let Some(w) = self.window_for_surface(surface.wl_surface()) {
            self.destroy_window(w);
        }
    }

    fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
        self.unconstrain_popup(&surface);
        if let Err(e) = self.popups.track_popup(PopupKind::Xdg(surface)) {
            log::warn!("popup: {e}");
        }
    }

    fn popup_destroyed(&mut self, surface: PopupSurface) {
        let gone = surface.wl_surface().clone();
        let Ok(root) = find_popup_root_surface(&PopupKind::Xdg(surface)) else {
            return;
        };
        crate::popupgrab::popup_gone(self, &gone, &root);
        if let Some(w) = self.window_for_surface(&root) {
            self.sync_popups(w);
            self.redraw_all();
        }
        crate::layer::popup_destroyed(self, &root);
    }

    /// A move must answer the press the seat still holds on this surface; any other serial is a
    /// client grabbing the pointer.
    fn move_request(&mut self, surface: ToplevelSurface, _seat: WlSeat, serial: Serial) {
        if let Some(w) = self.held_by(&surface, serial) {
            self.begin_move(w, true);
        }
    }

    fn resize_request(
        &mut self,
        surface: ToplevelSurface,
        _seat: WlSeat,
        serial: Serial,
        edges: xdg_toplevel::ResizeEdge,
    ) {
        if let Some(w) = self.held_by(&surface, serial) {
            self.begin_resize(w, reprisekit::frame::Edges(u32::from(edges)));
        }
    }

    /// A request before the map is applied by `map_window`, which reads the pending state itself.
    fn maximize_request(&mut self, surface: ToplevelSurface) {
        self.state_request(&surface, xdg_toplevel::State::Maximized, true);
    }

    fn unmaximize_request(&mut self, surface: ToplevelSurface) {
        self.state_request(&surface, xdg_toplevel::State::Maximized, false);
    }

    fn fullscreen_request(&mut self, surface: ToplevelSurface, _output: Option<WlOutput>) {
        self.state_request(&surface, xdg_toplevel::State::Fullscreen, true);
    }

    fn unfullscreen_request(&mut self, surface: ToplevelSurface) {
        self.state_request(&surface, xdg_toplevel::State::Fullscreen, false);
    }

    fn minimize_request(&mut self, surface: ToplevelSurface) {
        if let Some(w) = self.window_for_surface(surface.wl_surface())
            && self.win(w).is_some_and(|w| w.mapped)
        {
            self.set_minimized(w, true);
        }
    }

    fn parent_changed(&mut self, surface: ToplevelSurface) {
        if let Some(w) = self.window_for_surface(surface.wl_surface()) {
            self.on_parent_changed(w);
        }
    }

    fn reposition_request(
        &mut self,
        surface: PopupSurface,
        positioner: PositionerState,
        token: u32,
    ) {
        surface.with_pending_state(|state| state.positioner = positioner);
        self.unconstrain_popup(&surface);
        surface.send_repositioned(token);
    }

    /// During a drag the menu's grabs wait for the drag's end (`dnd`).
    fn grab(&mut self, surface: PopupSurface, _seat: WlSeat, serial: Serial) {
        // No client takes the keyboard under the lock.
        if self.is_locked() {
            return;
        }
        let kind = PopupKind::Xdg(surface);
        let Ok(root) = find_popup_root_surface(&kind) else {
            return;
        };
        let seat = self.seat.clone();
        let grab = match self.popups.grab_popup(root, kind, &seat, serial) {
            Ok(g) => g,
            Err(e) => {
                log::debug!("popup grab refused: {e:?}");
                return;
            }
        };
        if self.dnd.running() {
            self.dnd.hold_popup_grab(grab, serial);
        } else {
            self.set_popup_grab(grab, serial);
        }
    }

    fn title_changed(&mut self, surface: ToplevelSurface) {
        if let Some(w) = self.window_for_surface(surface.wl_surface()) {
            self.replan_unmapped(w);
            self.note_title(w);
        }
    }

    fn app_id_changed(&mut self, surface: ToplevelSurface) {
        if let Some(w) = self.window_for_surface(surface.wl_surface()) {
            self.note_app_id(w);
            self.replan_unmapped(w);
            self.note_title(w);
        }
    }
}

impl State {
    /// Give a popup's `grab` (asked for with `serial`) the keyboard and the pointer, unless another
    /// grab holds them.
    pub fn set_popup_grab(&mut self, mut grab: PopupGrab<State>, serial: Serial) {
        let seat = self.seat.clone();
        if let Some(keyboard) = seat.get_keyboard() {
            let held_by_other = keyboard.is_grabbed()
                && !(keyboard.has_grab(serial)
                    || keyboard.has_grab(grab.previous_serial().unwrap_or(serial)));
            if held_by_other && !crate::textinput::ime_holds_keyboard(&keyboard) {
                grab.ungrab(PopupUngrabStrategy::All);
                return;
            }
            self.flush_motion();
            keyboard.set_focus(self, grab.current_grab(), serial);
            keyboard.set_grab(self, PopupKeyboardGrab::new(&grab), serial);
        }
        if let Some(pointer) = seat.get_pointer() {
            let parent = grab.previous_serial().unwrap_or_else(|| grab.serial());
            if !crate::popupgrab::pointer_joins(&pointer, serial, parent) {
                grab.ungrab(PopupUngrabStrategy::All);
                return;
            }
            pointer.set_grab(self, PopupPointer::new(&grab), serial, Focus::Keep);
        }
    }
}

impl State {
    fn held_by(&mut self, surface: &ToplevelSurface, serial: Serial) -> Option<WindowId> {
        let w = self.window_for_surface(surface.wl_surface())?;
        let pointer = self.seat.get_pointer()?;
        let held = pointer.has_grab(serial)
            && pointer.grab_start_data().is_some_and(|d| {
                d.focus
                    .as_ref()
                    .and_then(|(f, _)| f.surface())
                    .is_some_and(|f| f.id().same_client_as(&surface.wl_surface().id()))
            });
        (held || self.take_for_grab(surface.wl_surface(), serial)).then_some(w)
    }

    fn state_request(&mut self, surface: &ToplevelSurface, state: xdg_toplevel::State, on: bool) {
        let Some(w) = self.window_for_surface(surface.wl_surface()) else {
            return;
        };
        if self.win(w).is_some_and(|w| w.mapped) {
            match state {
                xdg_toplevel::State::Fullscreen => {
                    if self.tabs.ask_fullscreen(w, on) {
                        self.set_fullscreen(w, on);
                    }
                }
                _ => self.set_maximized(w, on),
            }
        } else {
            surface.with_pending_state(|s| {
                if on {
                    s.states.set(state);
                } else {
                    s.states.unset(state);
                }
            });
        }
        if surface.is_initial_configure_sent() {
            surface.send_pending_configure();
        }
    }
}

impl XdgToplevelTagHandler for State {
    fn set_tag(&mut self, toplevel: xdg_toplevel::XdgToplevel, tag: String) {
        let w = self
            .windows
            .iter()
            .find(|w| w.xdg().is_some_and(|t| t.xdg_toplevel() == &toplevel))
            .map(|w| w.id);
        if let Some(w) = w {
            State::set_tag(self, w, &tag);
            self.redraw_all();
        }
    }
}

impl XdgDialogHandler for State {
    /// Unsetting modal changes nothing until the next focus change.
    fn dialog_hint_changed(&mut self, toplevel: ToplevelSurface, hint: ToplevelDialogHint) {
        let modal = hint == ToplevelDialogHint::Modal;
        if let Some(w) = self.window_for_surface(toplevel.wl_surface())
            && let Some(win) = self.win_mut(w)
        {
            win.modal = modal;
        }
        if modal
            && !self.is_locked()
            && let Some(f) = self.focused
            && self.modal_target(f) != f
        {
            self.focus_window(f);
        }
    }
}

impl XdgDecorationHandler for State {
    fn new_decoration(&mut self, toplevel: ToplevelSurface) {
        toplevel.with_pending_state(|s| s.decoration_mode = Some(DecorationMode::ServerSide));
    }

    fn request_mode(&mut self, toplevel: ToplevelSurface, mode: DecorationMode) {
        let csd = mode == DecorationMode::ClientSide;
        toplevel.with_pending_state(|s| s.decoration_mode = Some(mode));
        if toplevel.is_initial_configure_sent() {
            toplevel.send_pending_configure();
        }
        if let Some(w) = self.window_for_surface(toplevel.wl_surface()) {
            self.set_csd(w, csd);
        }
    }

    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        server_side(&toplevel);
        if let Some(w) = self.window_for_surface(toplevel.wl_surface()) {
            self.set_csd(w, false);
        }
    }
}

/// A client asked through KDE server-decoration to draw its own frame; read by `new_toplevel` when
/// the request came before the toplevel.
struct KdeClientSide(std::cell::Cell<bool>);

fn kde_client_side(surface: &WlSurface) -> bool {
    with_states(surface, |s| {
        s.data_map.get::<KdeClientSide>().is_some_and(|c| c.0.get())
    })
}

/// GTK3 speaks KDE server-decoration instead of xdg-decoration. The manager advertises server-side
/// as its default, so a GTK3 window comes up without its own title bar.
impl KdeDecorationHandler for State {
    fn kde_decoration_state(&self) -> &KdeDecorationState {
        &self.kde_decoration_state
    }

    fn request_mode(
        &mut self,
        surface: &WlSurface,
        decoration: &OrgKdeKwinServerDecoration,
        mode: WEnum<KdeMode>,
    ) {
        let WEnum::Value(mode) = mode else {
            return;
        };
        decoration.mode(mode);
        let csd = mode == KdeMode::Client;
        with_states(surface, |s| {
            s.data_map
                .insert_if_missing(|| KdeClientSide(std::cell::Cell::new(false)));
            if let Some(c) = s.data_map.get::<KdeClientSide>() {
                c.0.set(csd);
            }
        });
        if let Some(w) = self.window_for_surface(surface) {
            self.set_csd(w, csd);
        }
    }
}

fn server_side(toplevel: &ToplevelSurface) {
    toplevel.with_pending_state(|s| s.decoration_mode = Some(DecorationMode::ServerSide));
    if toplevel.is_initial_configure_sent() {
        toplevel.send_pending_configure();
    }
}

delegate_dispatch2!(State);
