//! A headless [`State`] for tests: no backend, listening socket or session bus. Outputs go in as
//! a driven connector's do ([`Harness::output`]); clients connect over a socket pair.

use std::{os::fd::AsFd, os::unix::net::UnixStream, sync::Arc, time::Duration};

use smithay::{
    backend::input::{ButtonState, InputTime},
    output::{Mode, Output, PhysicalProperties, Scale, Subpixel},
    reexports::calloop::EventLoop,
    utils::{Point, Transform},
};
use wayland_client::{
    self as wc, Connection, EventQueue, QueueHandle,
    protocol::{
        wl_buffer::WlBuffer, wl_compositor::WlCompositor, wl_data_device, wl_data_device_manager,
        wl_data_offer, wl_data_source, wl_keyboard, wl_output::WlOutput, wl_registry, wl_seat,
        wl_shm, wl_shm_pool, wl_surface::WlSurface,
    },
};
use wayland_protocols::{
    ext::session_lock::v1::client::{
        ext_session_lock_manager_v1::ExtSessionLockManagerV1,
        ext_session_lock_surface_v1::{self, ExtSessionLockSurfaceV1},
        ext_session_lock_v1::ExtSessionLockV1,
    },
    wp::text_input::zv3::client::{
        zwp_text_input_manager_v3::ZwpTextInputManagerV3, zwp_text_input_v3::ZwpTextInputV3,
    },
    xdg::{
        activation::v1::client::xdg_activation_v1::XdgActivationV1,
        shell::client::{xdg_popup, xdg_positioner, xdg_surface, xdg_toplevel, xdg_wm_base},
    },
};
use wayland_protocols_misc::{
    zwp_input_method_v2::client::{
        zwp_input_method_keyboard_grab_v2::{self as im_grab, ZwpInputMethodKeyboardGrabV2},
        zwp_input_method_manager_v2::ZwpInputMethodManagerV2,
        zwp_input_method_v2::{self as im, ZwpInputMethodV2},
    },
    zwp_virtual_keyboard_v1::client::{
        zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
        zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
    },
};

use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::{self, ZwlrLayerShellV1},
    zwlr_layer_surface_v1::{self, ZwlrLayerSurfaceV1},
};

use crate::{
    compositor::{ClientState, Host, State},
    config::Config,
    window::WindowId,
};

/// `BTN_LEFT`.
pub const LEFT: u32 = 0x110;

/// The test process's own folder for the files a [`State`] saves; removed as the process exits.
pub fn scratch() -> std::path::PathBuf {
    static ONCE: std::sync::Once = std::sync::Once::new();
    extern "C" fn remove() {
        let _ = std::fs::remove_dir_all(dir());
    }
    fn dir() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("reprise-test-{}", std::process::id()))
    }
    // SAFETY: `remove` is a plain function that touches no state torn down before exit handlers.
    ONCE.call_once(|| unsafe {
        libc::atexit(remove);
    });
    dir()
}

pub struct Harness {
    pub event_loop: EventLoop<'static, State>,
    pub state: State,
}

impl Harness {
    pub fn new() -> Harness {
        Harness::with_config(Config::default())
    }

    pub fn with_config(config: Config) -> Harness {
        let event_loop: EventLoop<'static, State> = EventLoop::try_new().expect("event loop");
        let config_path = scratch().join("config.toml");
        let state = State::on_display(
            config,
            config_path,
            event_loop.handle(),
            event_loop.get_signal(),
            "wayland-test".into(),
            Host::Bare,
        )
        .expect("headless state");
        Harness { event_loop, state }
    }

    /// One turn of the loop: what is ready runs, then the turn's end (`State::turn_end`).
    pub fn dispatch(&mut self) {
        self.event_loop
            .dispatch(Some(Duration::ZERO), &mut self.state)
            .expect("dispatch");
        self.state.turn_end();
    }

    /// Turns of the loop for `ms`, so timers that long fire.
    pub fn run_for(&mut self, ms: u64) {
        let end = std::time::Instant::now() + Duration::from_millis(ms);
        while std::time::Instant::now() < end {
            self.event_loop
                .dispatch(Some(Duration::from_millis(5)), &mut self.state)
                .expect("dispatch");
            self.state.turn_end();
        }
    }

    /// A `w`x`h` output at scale 1 put in the layout, as `backend::enable` does once it drives
    /// a connector.
    pub fn output(&mut self, name: &str, w: i32, h: i32) -> Output {
        let output = Output::new(
            name.into(),
            PhysicalProperties {
                size: (0, 0).into(),
                subpixel: Subpixel::Unknown,
                make: "Reprise".into(),
                model: "Test".into(),
                serial_number: "0".into(),
            },
        );
        let mode = Mode {
            size: (w, h).into(),
            refresh: 60_000,
        };
        output.set_preferred(mode);
        output.change_current_state(
            Some(mode),
            Some(Transform::Normal),
            Some(Scale::Fractional(1.0)),
            None,
        );
        let _global = output.create_global::<State>(&self.state.dh);
        self.state.scene.add_output(&output);
        self.state.output_added(&output);
        self.dispatch();
        output
    }

    /// Move the pointer to (`x`, `y`) and run the hover work its settle timer would.
    pub fn pointer_to(&mut self, x: f64, y: f64) {
        self.state.pointer_to(Point::from((x, y)), InputTime::now());
        self.state.settle_pointer();
        self.dispatch();
    }

    pub fn button(&mut self, button: u32, pressed: bool) {
        let s = if pressed {
            ButtonState::Pressed
        } else {
            ButtonState::Released
        };
        self.state.handle_button(button, s, InputTime::now());
        self.dispatch();
    }

    pub fn click(&mut self, x: f64, y: f64) {
        self.pointer_to(x, y);
        self.button(LEFT, true);
        self.button(LEFT, false);
    }

    /// Key `code` (an evdev code) down or up on the seat's keyboard.
    pub fn key(&mut self, code: u32, pressed: bool) {
        self.state
            .handle_key((code + 8).into(), pressed, InputTime::now());
        self.dispatch();
    }

    /// A client on one end of a socket pair, its registry read.
    pub fn connect(&mut self) -> TestClient {
        self.connect_with(None)
    }

    /// A client [`State::client_pid`] takes for process `pid`; pick one above the kernel's
    /// pid limit so no real process answers to it.
    pub fn connect_as(&mut self, pid: i32) -> TestClient {
        self.connect_with(Some(pid))
    }

    fn connect_with(&mut self, pid: Option<i32>) -> TestClient {
        let (server, client) = UnixStream::pair().expect("socket pair");
        let mut data = ClientState::of_stream(&server, None);
        data.pid = pid;
        let server = self
            .state
            .dh
            .insert_client(server, Arc::new(data))
            .expect("client");
        let conn = Connection::from_socket(client).expect("connection");
        let queue = conn.new_event_queue();
        let qh = queue.handle();
        conn.display().get_registry(&qh, ());
        let mut c = TestClient {
            queue,
            side: ClientSide::default(),
            server,
        };
        self.roundtrip(&mut c);
        c.bind_globals();
        self.roundtrip(&mut c);
        c
    }

    /// Requests and events back and forth until both sides have nothing left.
    pub fn roundtrip(&mut self, c: &mut TestClient) {
        for _ in 0..4 {
            c.queue.flush().expect("client flush");
            self.dispatch();
            if let Some(guard) = c.queue.prepare_read() {
                let _ = guard.read();
            }
            c.queue
                .dispatch_pending(&mut c.side)
                .expect("client dispatch");
        }
    }

    /// An xdg toplevel of `w`x`h` from `c`, configured and mapped with its first buffer. Its
    /// client draws each size a configure asks for.
    pub fn map_toplevel(&mut self, c: &mut TestClient, app_id: &str, w: i32, h: i32) -> Toplevel {
        let before: Vec<WindowId> = self.state.windows.iter().map(|w| w.id).collect();
        let shell = c.toplevel(app_id);
        self.roundtrip(c);
        let id = (self.state.windows.iter())
            .map(|w| w.id)
            .find(|id| !before.contains(id))
            .expect("the toplevel's window");
        c.side.shells[shell].size = (w, h);
        let surface = c.side.shells[shell].surface.clone();
        attach(
            c.side.shm.as_ref().expect("wl_shm"),
            &c.qh(),
            &surface,
            w,
            h,
        );
        self.roundtrip(c);
        assert!(
            self.state.win(id).is_some_and(|w| w.mapped),
            "the toplevel maps with its first buffer"
        );
        Toplevel { id, shell }
    }

    /// `t`'s client draws itself at `w`x`h` without being asked, as a program fitting new
    /// bounds or a new text size does.
    pub fn resize_self(&mut self, c: &mut TestClient, t: Toplevel, w: i32, h: i32) {
        c.side.shells[t.shell].size = (w, h);
        let surface = c.side.shells[t.shell].surface.clone();
        attach(
            c.side.shm.as_ref().expect("wl_shm"),
            &c.qh(),
            &surface,
            w,
            h,
        );
        self.roundtrip(c);
    }

    /// A `w`x`h` xdg popup on toplevel `t`, mapped with its first buffer, that asks for the
    /// seat's grab under the last keyboard enter's serial, as a menu does.
    pub fn map_popup(&mut self, c: &mut TestClient, t: Toplevel, w: i32, h: i32) -> Popup {
        self.map_popup_with(c, t, w, h, |p| p.set_anchor_rect(0, 0, 1, 1))
    }

    /// [`Harness::map_popup`] with `place` setting the positioner's anchor, gravity and
    /// constraint adjustments.
    pub fn map_popup_with(
        &mut self,
        c: &mut TestClient,
        t: Toplevel,
        w: i32,
        h: i32,
        place: impl FnOnce(&xdg_positioner::XdgPositioner),
    ) -> Popup {
        let qh = c.qh();
        let wm = c.side.wm_base.clone().expect("xdg_wm_base");
        let surface = (c.side.compositor.as_ref())
            .expect("wl_compositor")
            .create_surface(&qh, ());
        let xdg = wm.get_xdg_surface(&surface, &qh, ());
        let positioner = wm.create_positioner(&qh, ());
        positioner.set_size(w, h);
        place(&positioner);
        let popup = xdg.get_popup(Some(&c.side.shells[t.shell].xdg), &positioner, &qh, ());
        positioner.destroy();
        let seat = c.side.seat.as_ref().expect("wl_seat");
        popup.grab(seat, c.side.enter_serial);
        surface.commit();
        self.roundtrip(c);
        attach(c.side.shm.as_ref().expect("wl_shm"), &qh, &surface, w, h);
        self.roundtrip(c);
        Popup {
            surface,
            xdg,
            popup,
        }
    }

    /// A `wl_surface` from `c` with no role yet, as the compositor sees it.
    pub fn bare_surface(
        &mut self,
        c: &mut TestClient,
    ) -> smithay::reexports::wayland_server::protocol::wl_surface::WlSurface {
        use wc::Proxy;
        let surface = (c.side.compositor.as_ref())
            .expect("wl_compositor")
            .create_surface(&c.qh(), ());
        self.roundtrip(c);
        c.server
            .object_from_protocol_id(&self.state.dh, surface.id().protocol_id())
            .expect("the surface on the compositor's side")
    }

    /// A flyout as `repriseapp` opens one: an overlay layer surface over the whole output that
    /// holds the keyboard outright, configured and mapped with its first buffer.
    pub fn map_flyout(&mut self, c: &mut TestClient) -> WlSurface {
        let qh = c.qh();
        let surface = (c.side.compositor.as_ref())
            .expect("wl_compositor")
            .create_surface(&qh, ());
        let layer = (c.side.layer_shell.as_ref())
            .expect("zwlr_layer_shell_v1")
            .get_layer_surface(
                &surface,
                None,
                zwlr_layer_shell_v1::Layer::Overlay,
                "test.flyout".into(),
                &qh,
                surface.clone(),
            );
        use zwlr_layer_surface_v1::{Anchor, KeyboardInteractivity};
        layer.set_anchor(Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right);
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
        surface.commit();
        self.roundtrip(c);
        self.roundtrip(c);
        surface
    }

    /// A locker's lock from `c` with one lock surface on the client's first `wl_output`,
    /// configured and mapped with its first buffer.
    pub fn map_lock_surface(&mut self, c: &mut TestClient) -> (ExtSessionLockV1, WlSurface) {
        let qh = c.qh();
        let lock = (c.side.lock_manager.as_ref())
            .expect("ext_session_lock_manager_v1")
            .lock(&qh, ());
        let surface = (c.side.compositor.as_ref())
            .expect("wl_compositor")
            .create_surface(&qh, ());
        let output = c.side.outputs.first().expect("a wl_output");
        lock.get_lock_surface(&surface, output, &qh, surface.clone());
        self.roundtrip(c);
        self.roundtrip(c);
        (lock, surface)
    }
}

/// A client's xdg popup.
pub struct Popup {
    pub surface: WlSurface,
    pub xdg: xdg_surface::XdgSurface,
    pub popup: xdg_popup::XdgPopup,
}

/// A client's toplevel: the window it made and its index in [`ClientSide::shells`].
#[derive(Clone, Copy, Debug)]
pub struct Toplevel {
    pub id: WindowId,
    pub shell: usize,
}

pub struct TestClient {
    pub queue: EventQueue<ClientSide>,
    pub side: ClientSide,
    /// The compositor's side of the connection.
    pub server: smithay::reexports::wayland_server::Client,
}

impl TestClient {
    pub fn qh(&self) -> QueueHandle<ClientSide> {
        self.queue.handle()
    }

    fn bind_globals(&mut self) {
        let qh = self.qh();
        let Some(registry) = self.side.registry.clone() else {
            return;
        };
        for (name, interface, version) in self.side.globals.clone() {
            match interface.as_str() {
                "wl_compositor" => {
                    self.side.compositor = Some(registry.bind(name, version.min(6), &qh, ()));
                }
                "wl_shm" => self.side.shm = Some(registry.bind(name, 1, &qh, ())),
                "xdg_wm_base" => {
                    self.side.wm_base = Some(registry.bind(name, version.min(6), &qh, ()));
                }
                "xdg_activation_v1" => {
                    self.side.activation = Some(registry.bind(name, 1, &qh, ()));
                }
                "wl_seat" => {
                    let seat: wl_seat::WlSeat = registry.bind(name, version.min(7), &qh, ());
                    self.side.keyboard = Some(seat.get_keyboard(&qh, ()));
                    self.side.seat = Some(seat);
                }
                "wl_data_device_manager" => {
                    self.side.data_manager = Some(registry.bind(name, version.min(3), &qh, ()));
                }
                "zwp_input_method_manager_v2" => {
                    self.side.im_manager = Some(registry.bind(name, 1, &qh, ()));
                }
                "zwp_text_input_manager_v3" => {
                    self.side.ti_manager = Some(registry.bind(name, 1, &qh, ()));
                }
                "zwp_virtual_keyboard_manager_v1" => {
                    self.side.vk_manager = Some(registry.bind(name, 1, &qh, ()));
                }
                "zwlr_layer_shell_v1" => {
                    self.side.layer_shell = Some(registry.bind(name, version.min(4), &qh, ()));
                }
                "ext_session_lock_manager_v1" => {
                    self.side.lock_manager = Some(registry.bind(name, 1, &qh, ()));
                }
                "wl_output" => self.side.outputs.push(registry.bind(name, 1, &qh, ())),
                _ => {}
            }
        }
    }

    /// A toplevel's surfaces with the initial commit, no buffer yet; its index in `shells`.
    fn toplevel(&mut self, app_id: &str) -> usize {
        let qh = self.qh();
        let surface = (self.side.compositor.as_ref())
            .expect("wl_compositor")
            .create_surface(&qh, ());
        let xdg = (self.side.wm_base.as_ref())
            .expect("xdg_wm_base")
            .get_xdg_surface(&surface, &qh, ());
        let i = self.side.shells.len();
        let toplevel = xdg.get_toplevel(&qh, i);
        toplevel.set_app_id(app_id.into());
        surface.commit();
        self.side.shells.push(Shell {
            surface,
            xdg,
            toplevel,
            pending: (0, 0),
            size: (0, 0),
            bounds: (0, 0),
        });
        i
    }

    /// Ask to raise toplevel `t` with activation `token`, as a program handed one does.
    pub fn activate(&self, token: &str, t: Toplevel) {
        (self.side.activation.as_ref())
            .expect("xdg_activation_v1")
            .activate(token.into(), &self.side.shells[t.shell].surface);
    }

    /// A clipboard selection offering `mimes`, set while the client has the keyboard. The source
    /// answers no reads.
    pub fn copy(&mut self, mimes: &[&str]) -> wl_data_source::WlDataSource {
        let qh = self.qh();
        let manager = self
            .side
            .data_manager
            .as_ref()
            .expect("wl_data_device_manager");
        let seat = self.side.seat.as_ref().expect("wl_seat");
        let device = manager.get_data_device(seat, &qh, ());
        let source = manager.create_data_source(&qh, ());
        for m in mimes {
            source.offer(m.to_string());
        }
        device.set_selection(Some(&source), 0);
        source
    }

    fn seat(&self) -> &wl_seat::WlSeat {
        self.side.seat.as_ref().expect("wl_seat")
    }

    /// An input method on the seat, as fcitx5 asks for one; its index in `ims`. The manager is
    /// bound at connect, while no input method hides its global.
    pub fn input_method(&mut self) -> usize {
        let qh = self.qh();
        let i = self.side.ims.len();
        let obj = (self.side.im_manager.as_ref())
            .expect("zwp_input_method_manager_v2")
            .get_input_method(self.seat(), &qh, i);
        self.side.ims.push(InputMethod {
            obj,
            events: Vec::new(),
            grab: None,
            grab_keys: Vec::new(),
            grab_mods: Vec::new(),
        });
        i
    }

    pub fn destroy_input_method(&mut self, i: usize) {
        self.side.ims[i].obj.destroy();
    }

    /// Input method `i` takes the keyboard grab.
    pub fn grab_keyboard(&mut self, i: usize) {
        let qh = self.qh();
        let im = &mut self.side.ims[i];
        im.grab = Some(im.obj.grab_keyboard(&qh, i));
    }

    /// Input method `i` releases its keyboard grab.
    pub fn release_grab(&mut self, i: usize) {
        if let Some(g) = self.side.ims[i].grab.take() {
            g.release();
        }
    }

    /// A text input on the seat; its index in `text_inputs`.
    pub fn text_input(&mut self) -> usize {
        let qh = self.qh();
        let ti = (self.side.ti_manager.as_ref())
            .expect("zwp_text_input_manager_v3")
            .get_text_input(self.seat(), &qh, ());
        self.side.text_inputs.push(ti);
        self.side.text_inputs.len() - 1
    }

    pub fn destroy_text_input(&self, i: usize) {
        self.side.text_inputs[i].destroy();
    }

    /// A virtual keyboard under the seat's keymap; its index in `vks`.
    pub fn virtual_keyboard(&mut self) -> usize {
        let qh = self.qh();
        let vk = (self.side.vk_manager.as_ref())
            .expect("zwp_virtual_keyboard_manager_v1")
            .create_virtual_keyboard(self.seat(), &qh, ());
        let (fd, size) = self.side.keymap.as_ref().expect("the seat's keymap");
        vk.keymap(wl_keyboard::KeymapFormat::XkbV1.into(), fd.as_fd(), *size);
        self.side.vks.push(vk);
        self.side.vks.len() - 1
    }

    /// A virtual keyboard under keymap `text` (xkb text format); its index in `vks`.
    pub fn virtual_keyboard_with(&mut self, text: &str) -> usize {
        let qh = self.qh();
        let vk = (self.side.vk_manager.as_ref())
            .expect("zwp_virtual_keyboard_manager_v1")
            .create_virtual_keyboard(self.seat(), &qh, ());
        let bytes = [text.as_bytes(), b"\0"].concat();
        let fd = crate::eis::sealed_memfd(&bytes).expect("keymap memfd");
        let size = u32::try_from(bytes.len()).expect("keymap size");
        vk.keymap(wl_keyboard::KeymapFormat::XkbV1.into(), fd.as_fd(), size);
        self.side.vks.push(vk);
        self.side.vks.len() - 1
    }

    /// Key `code` (an evdev code) down or up on virtual keyboard `i`.
    pub fn vk_key(&self, i: usize, code: u32, pressed: bool) {
        self.side.vks[i].key(0, code, u32::from(pressed));
    }

    /// Destroy popup `p`'s role objects and keep its surface, as a toolkit hiding a menu does.
    pub fn hide_popup(&mut self, p: &Popup) {
        p.popup.destroy();
        p.xdg.destroy();
    }

    /// Destroy toplevel `t`'s role objects, as a client closing it does.
    pub fn destroy(&mut self, t: Toplevel) {
        let s = &self.side.shells[t.shell];
        s.toplevel.destroy();
        s.xdg.destroy();
        s.surface.destroy();
    }
}

/// Attach a `w`x`h` buffer to `surface` and commit it.
fn attach(shm: &wl_shm::WlShm, qh: &QueueHandle<ClientSide>, surface: &WlSurface, w: i32, h: i32) {
    let size = w * h * 4;
    // SAFETY: plain syscall; the name is a valid C string.
    let raw = unsafe { libc::memfd_create(c"reprise-test".as_ptr(), libc::MFD_CLOEXEC) };
    assert!(raw >= 0, "memfd");
    // SAFETY: `raw` is a fresh fd nothing else owns.
    let fd = unsafe { <std::os::fd::OwnedFd as std::os::fd::FromRawFd>::from_raw_fd(raw) };
    let file = std::fs::File::from(fd);
    file.set_len(size as u64).expect("memfd size");
    let pool = shm.create_pool(file.as_fd(), size, qh, ());
    let buffer = pool.create_buffer(0, w, h, w * 4, wl_shm::Format::Argb8888, qh, ());
    pool.destroy();
    surface.attach(Some(&buffer), 0, 0);
    surface.damage_buffer(0, 0, w, h);
    surface.commit();
}

pub struct Shell {
    pub surface: WlSurface,
    pub xdg: xdg_surface::XdgSurface,
    pub toplevel: xdg_toplevel::XdgToplevel,
    /// The size the last toplevel configure asked for; (0, 0) leaves it to the client.
    pending: (i32, i32),
    /// The buffer's size; (0, 0) until the toplevel maps.
    pub size: (i32, i32),
    /// The bounds the last toplevel configure carried; (0, 0) before any.
    pub bounds: (i32, i32),
}

/// What an input method heard, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImHeard {
    Activate,
    Deactivate,
    Done,
    Unavailable,
}

pub struct InputMethod {
    obj: ZwpInputMethodV2,
    pub events: Vec<ImHeard>,
    grab: Option<ZwpInputMethodKeyboardGrabV2>,
    /// Each key its keyboard grab heard: its evdev code and whether it went down.
    pub grab_keys: Vec<(u32, bool)>,
    /// The depressed modifiers of each modifiers event its keyboard grab heard.
    pub grab_mods: Vec<u32>,
}

impl InputMethod {
    /// Whether the input method stands activated after its last `done`.
    pub fn active(&self) -> bool {
        let (mut pending, mut active) = (None, false);
        for e in &self.events {
            match e {
                ImHeard::Activate => pending = Some(true),
                ImHeard::Deactivate => pending = Some(false),
                ImHeard::Done => active = pending.take().unwrap_or(active),
                ImHeard::Unavailable => {}
            }
        }
        active
    }
}

#[derive(Default)]
pub struct ClientSide {
    registry: Option<wl_registry::WlRegistry>,
    globals: Vec<(u32, String, u32)>,
    compositor: Option<WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,
    activation: Option<XdgActivationV1>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    seat: Option<wl_seat::WlSeat>,
    data_manager: Option<wl_data_device_manager::WlDataDeviceManager>,
    /// The surface the keyboard last entered and has not left.
    pub keyboard_focus: Option<WlSurface>,
    enter_serial: u32,
    /// The seat's keymap as the keyboard got it: the file and its size.
    keymap: Option<(std::os::fd::OwnedFd, u32)>,
    im_manager: Option<ZwpInputMethodManagerV2>,
    ti_manager: Option<ZwpTextInputManagerV3>,
    vk_manager: Option<ZwpVirtualKeyboardManagerV1>,
    pub ims: Vec<InputMethod>,
    pub text_inputs: Vec<ZwpTextInputV3>,
    pub vks: Vec<ZwpVirtualKeyboardV1>,
    pub shells: Vec<Shell>,
    /// Each key event the keyboard heard: its evdev code and whether it went down.
    pub keys: Vec<(u32, bool)>,
    /// The `shells` the compositor asked to close, in order.
    pub closes: Vec<usize>,
    layer_shell: Option<ZwlrLayerShellV1>,
    /// The layer surfaces the compositor closed, by their `wl_surface`.
    pub layers_closed: Vec<WlSurface>,
    lock_manager: Option<ExtSessionLockManagerV1>,
    outputs: Vec<WlOutput>,
}

impl ClientSide {
    /// The text of the keymap the keyboard last got.
    pub fn keymap_text(&self) -> String {
        use std::os::unix::fs::FileExt;
        let (fd, size) = self.keymap.as_ref().expect("a keymap");
        let mut text = vec![0; *size as usize];
        std::fs::File::from(fd.try_clone().expect("keymap fd"))
            .read_exact_at(&mut text, 0)
            .expect("keymap read");
        String::from_utf8_lossy(&text)
            .trim_end_matches('\0')
            .to_string()
    }
}

impl wc::Dispatch<wl_registry::WlRegistry, ()> for ClientSide {
    fn event(
        st: &mut Self,
        registry: &wl_registry::WlRegistry,
        ev: wl_registry::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        st.registry = Some(registry.clone());
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = ev
        {
            st.globals.push((name, interface, version));
        }
    }
}

impl wc::Dispatch<xdg_wm_base::XdgWmBase, ()> for ClientSide {
    fn event(
        _: &mut Self,
        base: &xdg_wm_base::XdgWmBase,
        ev: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = ev {
            base.pong(serial);
        }
    }
}

impl wc::Dispatch<xdg_surface::XdgSurface, ()> for ClientSide {
    fn event(
        st: &mut Self,
        xdg: &xdg_surface::XdgSurface,
        ev: xdg_surface::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let xdg_surface::Event::Configure { serial } = ev else {
            return;
        };
        xdg.ack_configure(serial);
        let Some(s) = st.shells.iter_mut().find(|s| &s.xdg == xdg) else {
            return;
        };
        if s.size == (0, 0) {
            return;
        }
        let (w, h) = s.pending;
        if w > 0 && h > 0 && (w, h) != s.size {
            s.size = (w, h);
            let shm = st.shm.as_ref().expect("wl_shm");
            attach(shm, qh, &s.surface, w, h);
        } else {
            s.surface.commit();
        }
    }
}

impl wc::Dispatch<xdg_toplevel::XdgToplevel, usize> for ClientSide {
    fn event(
        st: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        ev: xdg_toplevel::Event,
        &i: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match ev {
            xdg_toplevel::Event::Configure { width, height, .. } => {
                st.shells[i].pending = (width, height);
            }
            xdg_toplevel::Event::ConfigureBounds { width, height } => {
                st.shells[i].bounds = (width, height);
            }
            xdg_toplevel::Event::Close => st.closes.push(i),
            _ => {}
        }
    }
}

impl wc::Dispatch<ZwlrLayerSurfaceV1, WlSurface> for ClientSide {
    fn event(
        st: &mut Self,
        layer: &ZwlrLayerSurfaceV1,
        ev: zwlr_layer_surface_v1::Event,
        surface: &WlSurface,
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match ev {
            zwlr_layer_surface_v1::Event::Configure {
                serial,
                width,
                height,
            } => {
                layer.ack_configure(serial);
                let shm = st.shm.as_ref().expect("wl_shm");
                attach(shm, qh, surface, width as i32, height as i32);
            }
            zwlr_layer_surface_v1::Event::Closed => st.layers_closed.push(surface.clone()),
            _ => {}
        }
    }
}

impl wc::Dispatch<ExtSessionLockSurfaceV1, WlSurface> for ClientSide {
    fn event(
        st: &mut Self,
        lock_surface: &ExtSessionLockSurfaceV1,
        ev: ext_session_lock_surface_v1::Event,
        surface: &WlSurface,
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let ext_session_lock_surface_v1::Event::Configure {
            serial,
            width,
            height,
        } = ev
        {
            lock_surface.ack_configure(serial);
            let shm = st.shm.as_ref().expect("wl_shm");
            attach(shm, qh, surface, width as i32, height as i32);
        }
    }
}

impl wc::Dispatch<wl_keyboard::WlKeyboard, ()> for ClientSide {
    fn event(
        st: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        ev: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match ev {
            wl_keyboard::Event::Key { key, state, .. } => {
                let down = state == wc::WEnum::Value(wl_keyboard::KeyState::Pressed);
                st.keys.push((key, down));
            }
            wl_keyboard::Event::Enter {
                serial, surface, ..
            } => {
                st.enter_serial = serial;
                st.keyboard_focus = Some(surface);
            }
            wl_keyboard::Event::Leave { surface, .. } => {
                st.keyboard_focus.take_if(|f| *f == surface);
            }
            wl_keyboard::Event::Keymap { fd, size, .. } => st.keymap = Some((fd, size)),
            _ => {}
        }
    }
}

impl wc::Dispatch<ZwpInputMethodV2, usize> for ClientSide {
    fn event(
        st: &mut Self,
        _: &ZwpInputMethodV2,
        ev: im::Event,
        &i: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let heard = match ev {
            im::Event::Activate => ImHeard::Activate,
            im::Event::Deactivate => ImHeard::Deactivate,
            im::Event::Done => ImHeard::Done,
            im::Event::Unavailable => ImHeard::Unavailable,
            _ => return,
        };
        st.ims[i].events.push(heard);
    }
}

impl wc::Dispatch<ZwpInputMethodKeyboardGrabV2, usize> for ClientSide {
    fn event(
        st: &mut Self,
        _: &ZwpInputMethodKeyboardGrabV2,
        ev: im_grab::Event,
        &i: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match ev {
            im_grab::Event::Key { key, state, .. } => {
                let down = state == wc::WEnum::Value(wl_keyboard::KeyState::Pressed);
                st.ims[i].grab_keys.push((key, down));
            }
            im_grab::Event::Modifiers { mods_depressed, .. } => {
                st.ims[i].grab_mods.push(mods_depressed);
            }
            _ => {}
        }
    }
}

impl wc::Dispatch<wl_data_device::WlDataDevice, ()> for ClientSide {
    fn event(
        _: &mut Self,
        _: &wl_data_device::WlDataDevice,
        _: wl_data_device::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }

    wc::event_created_child!(ClientSide, wl_data_device::WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (wl_data_offer::WlDataOffer, ()),
    ]);
}

wc::delegate_noop!(ClientSide: ignore WlCompositor);
wc::delegate_noop!(ClientSide: ignore wl_data_device_manager::WlDataDeviceManager);
wc::delegate_noop!(ClientSide: ignore wl_data_offer::WlDataOffer);
wc::delegate_noop!(ClientSide: ignore wl_data_source::WlDataSource);
wc::delegate_noop!(ClientSide: ignore WlSurface);
wc::delegate_noop!(ClientSide: ignore wl_shm::WlShm);
wc::delegate_noop!(ClientSide: ignore wl_shm_pool::WlShmPool);
wc::delegate_noop!(ClientSide: ignore WlBuffer);
wc::delegate_noop!(ClientSide: ignore wl_seat::WlSeat);
wc::delegate_noop!(ClientSide: ignore XdgActivationV1);
wc::delegate_noop!(ClientSide: ignore xdg_positioner::XdgPositioner);
wc::delegate_noop!(ClientSide: ignore xdg_popup::XdgPopup);
wc::delegate_noop!(ClientSide: ignore ZwpInputMethodManagerV2);
wc::delegate_noop!(ClientSide: ignore ZwpTextInputManagerV3);
wc::delegate_noop!(ClientSide: ignore ZwpTextInputV3);
wc::delegate_noop!(ClientSide: ignore ZwpVirtualKeyboardManagerV1);
wc::delegate_noop!(ClientSide: ignore ZwpVirtualKeyboardV1);
wc::delegate_noop!(ClientSide: ignore ZwlrLayerShellV1);
wc::delegate_noop!(ClientSide: ignore ExtSessionLockManagerV1);
wc::delegate_noop!(ClientSide: ignore ExtSessionLockV1);
wc::delegate_noop!(ClientSide: ignore WlOutput);

#[test]
fn a_headless_state_maps_a_clients_toplevel_on_its_output() {
    let mut h = Harness::new();
    h.output("TEST-1", 1024, 768);
    assert_eq!(h.state.bars.len(), 1, "the output's taskbar");
    let mut c = h.connect();
    let t = h.map_toplevel(&mut c, "test.app", 300, 200);
    assert_eq!(h.state.focused, Some(t.id));
}
