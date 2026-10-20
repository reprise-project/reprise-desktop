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
        wl_data_offer, wl_data_source, wl_keyboard, wl_registry, wl_seat, wl_shm, wl_shm_pool,
        wl_surface::WlSurface,
    },
};
use wayland_protocols::xdg::{
    activation::v1::client::xdg_activation_v1::XdgActivationV1,
    shell::client::{xdg_popup, xdg_positioner, xdg_surface, xdg_toplevel, xdg_wm_base},
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
        let (server, client) = UnixStream::pair().expect("socket pair");
        let data = ClientState::of_stream(&server, None);
        self.state
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

    /// A `w`x`h` xdg popup on toplevel `t`, mapped with its first buffer, that asks for the
    /// seat's grab under the last keyboard enter's serial, as a menu does.
    pub fn map_popup(&mut self, c: &mut TestClient, t: Toplevel, w: i32, h: i32) -> Popup {
        let qh = c.qh();
        let wm = c.side.wm_base.clone().expect("xdg_wm_base");
        let surface = (c.side.compositor.as_ref())
            .expect("wl_compositor")
            .create_surface(&qh, ());
        let xdg = wm.get_xdg_surface(&surface, &qh, ());
        let positioner = wm.create_positioner(&qh, ());
        positioner.set_size(w, h);
        positioner.set_anchor_rect(0, 0, 1, 1);
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

    /// Destroy toplevel `t`'s role objects, as a client closing it does.
    pub fn destroy(&mut self, t: Toplevel) {
        let s = &self.side.shells[t.shell];
        s.toplevel.destroy();
        s.xdg.destroy();
        s.surface.destroy();
    }

    /// Destroy popup `p`'s role objects and keep its surface, as a toolkit hiding a menu does.
    pub fn hide_popup(&mut self, p: &Popup) {
        p.popup.destroy();
        p.xdg.destroy();
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
    pub shells: Vec<Shell>,
    /// Each key event the keyboard heard: its evdev code and whether it went down.
    pub keys: Vec<(u32, bool)>,
    /// The `shells` the compositor asked to close, in order.
    pub closes: Vec<usize>,
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
            xdg_toplevel::Event::Close => st.closes.push(i),
            _ => {}
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

#[test]
fn a_headless_state_maps_a_clients_toplevel_on_its_output() {
    let mut h = Harness::new();
    h.output("TEST-1", 1024, 768);
    assert_eq!(h.state.bars.len(), 1, "the output's taskbar");
    let mut c = h.connect();
    let t = h.map_toplevel(&mut c, "test.app", 300, 200);
    assert_eq!(h.state.focused, Some(t.id));
}
