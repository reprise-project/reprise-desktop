//! Client surfaces in a scene, with a real client: hit testing through the
//! surface's input region, `wl_surface.enter`/`leave` as the node moves,
//! hides and goes, and frame callbacks only from the primary output.

use std::{
    io::Write,
    os::{fd::AsFd, unix::net::UnixStream},
    sync::Arc,
    time::Duration,
};

use reprisescene::{Scene, SceneElement, View};
use smithay::{
    backend::renderer::{
        element::{Element, Id, RenderElementStates},
        test::DummyRenderer,
        utils::on_commit_buffer_handler,
    },
    output::{Mode, Output, PhysicalProperties, Subpixel},
    reexports::wayland_server::{
        Client, Display,
        backend::{ClientData, ClientId, DisconnectReason},
        protocol::{wl_buffer, wl_surface::WlSurface},
    },
    utils::{Point, Rectangle, Transform},
    wayland::{
        buffer::BufferHandler,
        compositor::{CompositorClientState, CompositorHandler, CompositorState},
        output::OutputHandler,
        shm::{ShmHandler, ShmState},
    },
};
use wayland_client::{
    Connection, Dispatch, EventQueue, QueueHandle, delegate_noop,
    protocol::{
        wl_buffer::WlBuffer, wl_callback, wl_compositor::WlCompositor, wl_output::WlOutput,
        wl_region::WlRegion, wl_registry, wl_shm, wl_shm::WlShm, wl_shm_pool::WlShmPool,
        wl_surface,
    },
};

struct Server {
    compositor: CompositorState,
    shm: ShmState,
    surfaces: Vec<WlSurface>,
}

#[derive(Default)]
struct ClientState {
    compositor: CompositorClientState,
}

impl ClientData for ClientState {
    fn initialized(&self, _: ClientId) {}
    fn disconnected(&self, _: ClientId, _: DisconnectReason) {}
}

impl CompositorHandler for Server {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor
    }
    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        &client.get_data::<ClientState>().unwrap().compositor
    }
    fn new_surface(&mut self, surface: &WlSurface) {
        self.surfaces.push(surface.clone());
    }
    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);
    }
}

impl BufferHandler for Server {
    fn buffer_destroyed(&mut self, _: &wl_buffer::WlBuffer) {}
}

impl ShmHandler for Server {
    fn shm_state(&self) -> &ShmState {
        &self.shm
    }
}

impl OutputHandler for Server {}

smithay::delegate_dispatch2!(Server);

#[derive(Default)]
struct Peer {
    compositor: Option<WlCompositor>,
    shm: Option<WlShm>,
    outputs: Vec<WlOutput>,
    events: Vec<String>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for Peer {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "wl_compositor" => {
                    state.compositor = Some(registry.bind(name, version.min(6), qh, ()))
                }
                "wl_shm" => state.shm = Some(registry.bind(name, 1, qh, ())),
                "wl_output" => state
                    .outputs
                    .push(registry.bind(name, version.min(4), qh, ())),
                _ => {}
            }
        }
    }
}

impl Dispatch<wl_surface::WlSurface, ()> for Peer {
    fn event(
        state: &mut Self,
        _: &wl_surface::WlSurface,
        event: wl_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let name =
            |o: &WlOutput, s: &Self| ["L", "R"][s.outputs.iter().position(|x| x == o).unwrap()];
        match event {
            wl_surface::Event::Enter { output } => {
                state.events.push(format!("enter {}", name(&output, state)))
            }
            wl_surface::Event::Leave { output } => {
                state.events.push(format!("leave {}", name(&output, state)))
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_callback::WlCallback, ()> for Peer {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        event: wl_callback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { .. } = event {
            state.events.push("frame".into());
        }
    }
}

delegate_noop!(Peer: ignore WlCompositor);
delegate_noop!(Peer: ignore WlShm);
delegate_noop!(Peer: ignore WlShmPool);
delegate_noop!(Peer: ignore WlBuffer);
delegate_noop!(Peer: WlRegion);
delegate_noop!(Peer: ignore WlOutput);

struct World {
    display: Display<Server>,
    server: Server,
    conn: Connection,
    queue: EventQueue<Peer>,
    peer: Peer,
    client_surface: Option<wl_surface::WlSurface>,
    left: Output,
    right: Output,
}

fn output(name: &str, x: i32) -> Output {
    let o = Output::new(
        name.into(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "Reprise".into(),
            model: "test".into(),
            serial_number: String::new(),
        },
    );
    let mode = Mode {
        size: (100, 100).into(),
        refresh: 60_000,
    };
    o.change_current_state(
        Some(mode),
        Some(Transform::Normal),
        None,
        Some((x, 0).into()),
    );
    o.set_preferred(mode);
    o
}

impl World {
    fn new() -> World {
        let display = Display::<Server>::new().unwrap();
        let dh = display.handle();
        let server = Server {
            compositor: CompositorState::new::<Server>(&dh),
            shm: ShmState::new::<Server>(&dh, vec![]),
            surfaces: Vec::new(),
        };
        let left = output("L", 0);
        let right = output("R", 100);
        left.create_global::<Server>(&dh);
        right.create_global::<Server>(&dh);
        let (a, b) = UnixStream::pair().unwrap();
        a.set_nonblocking(true).unwrap();
        b.set_nonblocking(true).unwrap();
        dh.clone()
            .insert_client(a, Arc::new(ClientState::default()))
            .unwrap();
        let conn = Connection::from_socket(b).unwrap();
        let queue = conn.new_event_queue();
        conn.display().get_registry(&queue.handle(), ());
        let mut w = World {
            display,
            server,
            conn,
            queue,
            peer: Peer::default(),
            client_surface: None,
            left,
            right,
        };
        w.pump();
        assert!(w.peer.compositor.is_some() && w.peer.shm.is_some() && w.peer.outputs.len() == 2);
        w
    }

    fn pump(&mut self) {
        for _ in 0..4 {
            self.conn.flush().unwrap();
            self.display.dispatch_clients(&mut self.server).unwrap();
            self.display.flush_clients().unwrap();
            if let Some(guard) = self.conn.prepare_read() {
                let _ = guard.read();
            }
            self.queue.dispatch_pending(&mut self.peer).unwrap();
        }
    }

    fn surface(&mut self) -> WlSurface {
        let qh = self.queue.handle();
        let compositor = self.peer.compositor.clone().unwrap();
        let surface = compositor.create_surface(&qh, ());
        let mut file = tempfile::tempfile().unwrap();
        file.write_all(&[0xff; 40 * 40 * 4]).unwrap();
        let pool = self
            .peer
            .shm
            .as_ref()
            .unwrap()
            .create_pool(file.as_fd(), 40 * 40 * 4, &qh, ());
        let buffer = pool.create_buffer(0, 40, 40, 160, wl_shm::Format::Argb8888, &qh, ());
        let region = compositor.create_region(&qh, ());
        region.add(0, 0, 20, 40);
        surface.attach(Some(&buffer), 0, 0);
        surface.set_input_region(Some(&region));
        surface.frame(&qh, ());
        surface.commit();
        self.client_surface = Some(surface);
        self.pump();
        self.server.surfaces.last().cloned().unwrap()
    }

    fn request_frame(&mut self) {
        let surface = self.client_surface.clone().unwrap();
        surface.frame(&self.queue.handle(), ());
        surface.commit();
        self.pump();
    }

    fn rendered(&mut self, scene: &mut Scene<DummyRenderer>) -> Vec<String> {
        let none = RenderElementStates::default();
        scene.update_surfaces(&self.left, &none);
        scene.update_surfaces(&self.right, &none);
        self.events()
    }

    fn events(&mut self) -> Vec<String> {
        self.pump();
        std::mem::take(&mut self.peer.events)
    }
}

fn pt(x: f64, y: f64) -> Point<f64, smithay::utils::Logical> {
    Point::from((x, y))
}

#[test]
fn a_surface_takes_the_pointer_only_in_its_input_region() {
    let mut w = World::new();
    let surface = w.surface();
    let mut scene = Scene::<DummyRenderer>::new();
    let root = scene.root();
    let desktop = scene.rect_create(root, (200, 100), [0.0, 0.0, 0.0, 1.0]);
    let t = scene.tree_create(root);
    scene.set_position(t, 60, 0);
    let s = scene.surface_create(t, surface.clone());
    scene.set_position(s, 10, 10);
    assert_eq!(
        scene.extents(s),
        Some(smithay::utils::Rectangle::new(
            (70, 10).into(),
            (40, 40).into()
        ))
    );
    let hit = scene.node_at(root, pt(75.5, 20.0)).unwrap();
    assert_eq!(hit.node, s.node());
    assert_eq!(hit.surface.as_ref(), Some(&surface));
    assert_eq!(hit.local, pt(5.5, 10.0));
    assert_eq!(
        scene.node_at(root, pt(95.0, 20.0)).unwrap().node,
        desktop.node(),
        "right of the input region"
    );
    scene.set_input(s, reprisescene::Input::Passthrough);
    assert_eq!(
        scene.node_at(root, pt(75.0, 20.0)).unwrap().node,
        desktop.node()
    );
    assert_eq!(scene.surface(s), Some(&surface));
    let view = View::new(
        smithay::utils::Rectangle::new((50, 0).into(), (100, 100).into()),
        2.0,
    );
    let els = scene.render_elements(&mut DummyRenderer, &view);
    assert!(
        matches!(els[0], SceneElement::Surface(_)),
        "the surface is in front"
    );
    assert!(matches!(els[1], SceneElement::Solid(_)));
    assert_eq!(els[0].id(), &Id::from(&surface));
    assert_eq!(
        els[0].geometry(view.scale),
        Rectangle::new((40, 20).into(), (80, 80).into())
    );
}

#[test]
fn a_surface_enters_and_leaves_outputs_as_it_moves_hides_and_goes() {
    let mut w = World::new();
    let surface = w.surface();
    let mut scene = Scene::<DummyRenderer>::new();
    scene.add_output(&w.left);
    scene.add_output(&w.right);
    let s = scene.surface_create(scene.root(), surface);
    scene.set_position(s, 70, 10);
    assert_eq!(w.rendered(&mut scene), ["enter L", "enter R"]);
    assert_eq!(w.rendered(&mut scene), [] as [String; 0], "no repeats");
    scene.set_position(s, 150, 10);
    assert_eq!(w.rendered(&mut scene), ["leave L"]);
    scene.set_enabled(s, false);
    assert_eq!(w.rendered(&mut scene), ["leave R"]);
    scene.set_enabled(s, true);
    assert_eq!(w.rendered(&mut scene), ["enter R"]);
    scene.destroy(s);
    assert_eq!(w.events(), ["leave R"], "destroying the node leaves");
}

#[test]
fn a_surface_node_is_found_by_its_surface() {
    let mut w = World::new();
    let a = w.surface();
    let b = w.surface();
    let mut scene = Scene::<DummyRenderer>::new();
    let t = scene.tree_create(scene.root());
    let s = scene.surface_create(t, a.clone());
    assert_eq!(scene.surface_node(&a), Some(s));
    assert_eq!(scene.surface_node(&b), None, "b has no node");
    scene.destroy(s);
    assert_eq!(scene.surface_node(&a), None, "its node is gone");
}

#[test]
fn removing_an_output_makes_its_surfaces_leave() {
    let mut w = World::new();
    let surface = w.surface();
    let mut scene = Scene::<DummyRenderer>::new();
    scene.add_output(&w.left);
    let s = scene.surface_create(scene.root(), surface);
    scene.set_position(s, 10, 10);
    assert_eq!(w.rendered(&mut scene), ["enter L"]);
    scene.remove_output(&w.left);
    assert_eq!(w.events(), ["leave L"]);
}

#[test]
fn frames_come_from_the_primary_output_only() {
    let mut w = World::new();
    let surface = w.surface();
    let mut scene = Scene::<DummyRenderer>::new();
    scene.add_output(&w.left);
    scene.add_output(&w.right);
    let s = scene.surface_create(scene.root(), surface);
    scene.set_position(s, 70, 10);
    assert_eq!(scene.primary_output(s), Some(w.left.clone()));
    w.rendered(&mut scene);
    scene.send_frames(&w.right, Duration::from_millis(16), None);
    assert_eq!(
        w.events(),
        [] as [String; 0],
        "the right output is not primary"
    );
    scene.send_frames(&w.left, Duration::from_millis(16), None);
    assert_eq!(w.events(), ["frame"]);
    w.request_frame();
    scene.set_enabled(s, false);
    scene.send_frames(&w.left, Duration::from_millis(32), None);
    assert_eq!(w.events(), [] as [String; 0]);
    scene.set_enabled(s, true);
    scene.send_frames(&w.left, Duration::from_millis(48), None);
    assert_eq!(w.events(), ["frame"]);
    let view = View::of(&w.left).unwrap();
    assert_eq!(view.area.size, (100, 100).into());
}
