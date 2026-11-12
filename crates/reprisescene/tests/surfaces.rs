//! Client surfaces in a scene, with a real client: hit testing through the
//! surface's input region, `wl_surface.enter`/`leave` as the node moves,
//! hides and goes, and frame callbacks only from the primary output.

use std::{
    cell::Cell,
    io::Write,
    os::{fd::AsFd, unix::net::UnixStream},
    rc::Rc,
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

    /// A 100x100 surface whose buffer is in `format`, with `opaque` as its opaque region; not
    /// the surface [`World::request_frame`] asks frames for.
    fn cover(&mut self, format: wl_shm::Format, opaque: Option<(i32, i32, i32, i32)>) -> WlSurface {
        let qh = self.queue.handle();
        let compositor = self.peer.compositor.clone().unwrap();
        let surface = compositor.create_surface(&qh, ());
        let mut file = tempfile::tempfile().unwrap();
        file.write_all(&[0x80; 100 * 100 * 4]).unwrap();
        let shm = self.peer.shm.as_ref().unwrap();
        let pool = shm.create_pool(file.as_fd(), 100 * 100 * 4, &qh, ());
        let buffer = pool.create_buffer(0, 100, 100, 400, format, &qh, ());
        if let Some((x, y, w, h)) = opaque {
            let region = compositor.create_region(&qh, ());
            region.add(x, y, w, h);
            surface.set_opaque_region(Some(&region));
        }
        surface.attach(Some(&buffer), 0, 0);
        surface.commit();
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
        let pass = scene.surface_pass(&self.left);
        scene.update_surfaces(&pass, &none);
        let pass = scene.surface_pass(&self.right);
        scene.update_surfaces(&pass, &none);
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
fn a_surface_is_shown_by_the_outputs_it_overlaps_and_those_it_was_last_drawn_on() {
    let mut w = World::new();
    let surface = w.surface();
    let other = w.surface();
    let mut scene = Scene::<DummyRenderer>::new();
    scene.add_output(&w.left);
    scene.add_output(&w.right);
    assert_eq!(scene.outputs_showing(&surface), None, "no node shows it");
    let s = scene.surface_create(scene.root(), surface.clone());
    scene.set_position(s, 10, 10);
    w.rendered(&mut scene);
    assert_eq!(scene.outputs_showing(&surface), Some(vec![w.left.clone()]));
    scene.set_position(s, 150, 10);
    assert_eq!(
        scene.outputs_showing(&surface),
        Some(vec![w.right.clone(), w.left.clone()]),
        "the left output still shows its old place"
    );
    w.rendered(&mut scene);
    assert_eq!(scene.outputs_showing(&surface), Some(vec![w.right.clone()]));
    scene.set_enabled(s, false);
    w.rendered(&mut scene);
    assert_eq!(scene.outputs_showing(&surface), Some(vec![]), "hidden");
    assert_eq!(scene.outputs_showing(&other), None);
}

/// A watch over hiding a tree whose surface `gone` emptied after the right output drew it: the
/// rects it reports, by the outputs they meet.
fn watched_outputs_after(gone: impl FnOnce(&mut World, wl_surface::WlSurface)) -> Vec<String> {
    let mut w = World::new();
    let surface = w.surface();
    let mut scene = Scene::<DummyRenderer>::new();
    scene.add_output(&w.left);
    scene.add_output(&w.right);
    let t = scene.tree_create(scene.root());
    let s = scene.surface_create(t, surface);
    scene.set_position(s, 150, 10);
    assert_eq!(w.rendered(&mut scene), ["enter R"]);
    let client = w.client_surface.clone().unwrap();
    gone(&mut w, client);
    assert_eq!(scene.extents(s), None, "its client state is gone");
    assert!(scene.watch());
    scene.set_enabled(t, false);
    let rects = scene.watched();
    [&w.left, &w.right]
        .into_iter()
        .filter(|o| {
            let r = reprisescene::output_rect(o).unwrap();
            rects.iter().any(|x| x.overlaps(r))
        })
        .map(|o| o.name())
        .collect()
}

#[test]
fn a_watch_sees_a_surface_that_dropped_its_buffer_where_it_was_drawn() {
    let seen = watched_outputs_after(|w, s| {
        s.attach(None, 0, 0);
        s.commit();
        w.pump();
    });
    assert_eq!(seen, ["R"]);
}

#[test]
fn a_watch_sees_a_destroyed_surface_where_it_was_drawn() {
    let seen = watched_outputs_after(|w, s| {
        s.destroy();
        w.pump();
    });
    assert_eq!(seen, ["R"]);
}

#[test]
fn a_surface_hidden_on_an_output_is_not_shown_by_it_and_takes_no_frames_from_it() {
    let mut w = World::new();
    let surface = w.surface();
    let mut scene = Scene::<DummyRenderer>::new();
    scene.add_output(&w.left);
    scene.add_output(&w.right);
    let t = scene.tree_create(scene.root());
    let s = scene.surface_create(t, surface.clone());
    scene.set_position(s, 70, 10);
    assert_eq!(w.rendered(&mut scene), ["enter L", "enter R"]);
    scene.set_hidden_in(t, Some(Rectangle::new((0, 0).into(), (100, 100).into())));
    assert_eq!(
        scene.outputs_showing(&surface),
        Some(vec![w.right.clone()]),
        "the left output drew it last frame but draws it no more"
    );
    assert_eq!(scene.outputs_of(s), vec![w.right.clone()]);
    assert_eq!(scene.primary_output(s), Some(w.right.clone()));
    assert_eq!(
        w.rendered(&mut scene),
        [] as [String; 0],
        "it stays on both outputs"
    );
    let at = Duration::from_millis(16);
    scene.send_frames(&scene.surface_pass(&w.left), at, None);
    assert_eq!(w.events(), [] as [String; 0], "no frame from the left");
    scene.send_frames(&scene.surface_pass(&w.right), at, None);
    assert_eq!(w.events(), ["frame"]);
    scene.set_hidden_in(t, None);
    assert_eq!(
        scene.outputs_showing(&surface),
        Some(vec![w.left.clone(), w.right.clone()])
    );
    assert_eq!(scene.primary_output(s), Some(w.left.clone()));
}

/// A custom node drawing nothing, standing in for glass.
struct Pane;

impl reprisescene::CustomNode<DummyRenderer> for Pane {
    fn size(&self) -> smithay::utils::Size<i32, smithay::utils::Logical> {
        (100, 100).into()
    }

    fn elements(
        &self,
        _: &mut DummyRenderer,
        _: Point<f64, smithay::utils::Physical>,
        _: smithay::utils::Scale<f64>,
        _: f32,
    ) -> Vec<reprisescene::CustomElement<DummyRenderer>> {
        Vec::new()
    }
}

#[test]
fn a_surface_under_opaque_leaves_is_not_shown_by_that_output_and_takes_no_frames_from_it() {
    let mut w = World::new();
    let surface = w.surface();
    let (left, right) = (w.left.clone(), w.right.clone());
    let mut scene = Scene::<DummyRenderer>::new();
    scene.add_output(&left);
    scene.add_output(&right);
    let root = scene.root();
    let s = scene.surface_create(root, surface.clone());
    scene.set_position(s, 10, 10);
    assert_eq!(w.rendered(&mut scene), ["enter L"]);
    let cover = scene.rect_create(root, (100, 100), [0.0, 0.0, 0.0, 1.0]);
    assert_eq!(
        scene.outputs_showing(&surface),
        Some(vec![]),
        "under an opaque rect"
    );
    assert_eq!(
        scene.outputs_of(s),
        vec![left.clone()],
        "it still lies on the left output"
    );
    assert_eq!(scene.primary_output(s), None);
    assert_eq!(
        w.rendered(&mut scene),
        [] as [String; 0],
        "it stays entered"
    );
    scene.send_frames(&scene.surface_pass(&left), Duration::from_millis(16), None);
    assert_eq!(w.events(), [] as [String; 0], "no frame under the cover");

    let shown = Some(vec![left.clone()]);
    scene.set_opacity(cover, 0.9);
    assert_eq!(
        scene.outputs_showing(&surface),
        shown,
        "a cover drawn translucent"
    );
    scene.set_opacity(cover, 1.0);
    scene.rect_set_color(cover, [0.0, 0.0, 0.0, 0.5]);
    assert_eq!(
        scene.outputs_showing(&surface),
        shown,
        "a translucent colour"
    );
    scene.rect_set_color(cover, [0.0, 0.0, 0.0, 1.0]);
    scene.rect_set_size(cover, (49, 100));
    assert_eq!(
        scene.outputs_showing(&surface),
        shown,
        "a cover a pixel short"
    );
    scene.rect_set_size(cover, (100, 100));
    scene.raise_to_top(s);
    assert_eq!(
        scene.outputs_showing(&surface),
        shown,
        "raised above the cover"
    );
    scene.raise_to_top(cover);
    let glass = scene.custom_create(root, Box::new(Pane));
    scene.set_position(glass, 150, 0);
    assert_eq!(
        scene.outputs_showing(&surface),
        Some(vec![]),
        "glass on the other output"
    );
    scene.set_position(glass, 90, 90);
    assert_eq!(
        scene.outputs_showing(&surface),
        shown,
        "glass above on its output"
    );
    scene.destroy(glass);
    assert_eq!(scene.outputs_showing(&surface), Some(vec![]));

    scene.set_position(s, 70, 10);
    assert_eq!(scene.outputs_showing(&surface), Some(vec![right.clone()]));
    assert_eq!(
        scene.primary_output(s),
        Some(right.clone()),
        "the output that draws it"
    );
    scene.set_position(s, 10, 10);
    scene.set_enabled(cover, false);
    assert_eq!(scene.outputs_showing(&surface), shown);
    assert_eq!(scene.primary_output(s), Some(left.clone()));
    scene.send_frames(&scene.surface_pass(&left), Duration::from_millis(32), None);
    assert_eq!(
        w.events(),
        ["frame"],
        "the frame it asked for under the cover"
    );
}

/// Glass of `size` reading its rect grown by `grow` beneath it.
struct Reading {
    size: (i32, i32),
    grow: Rc<Cell<i32>>,
}

impl reprisescene::CustomNode<DummyRenderer> for Reading {
    fn size(&self) -> smithay::utils::Size<i32, smithay::utils::Logical> {
        self.size.into()
    }

    fn reads_beneath(&self, _: f64) -> Option<Vec<Rectangle<i32, smithay::utils::Logical>>> {
        let g = self.grow.get();
        let (w, h) = self.size;
        Some(vec![Rectangle::new(
            (-g, -g).into(),
            (w + 2 * g, h + 2 * g).into(),
        )])
    }

    fn elements(
        &self,
        _: &mut DummyRenderer,
        _: Point<f64, smithay::utils::Physical>,
        _: smithay::utils::Scale<f64>,
        _: f32,
    ) -> Vec<reprisescene::CustomElement<DummyRenderer>> {
        Vec::new()
    }
}

#[test]
fn a_surface_under_opaque_leaves_is_covered_unless_glass_shown_or_read_reads_it() {
    let mut w = World::new();
    let surface = w.surface();
    let left = w.left.clone();
    let mut scene = Scene::<DummyRenderer>::new();
    scene.add_output(&left);
    let root = scene.root();
    let s = scene.surface_create(root, surface.clone());
    scene.set_position(s, 10, 10);
    let cover = scene.rect_create(root, (100, 100), [0.0, 0.0, 0.0, 1.0]);
    let (covered, shown) = (Some(vec![]), Some(vec![left.clone()]));
    let reading = |scene: &mut Scene<DummyRenderer>, size, at: (i32, i32), grow: &Rc<Cell<i32>>| {
        let n = scene.custom_create(
            root,
            Box::new(Reading {
                size,
                grow: grow.clone(),
            }),
        );
        scene.set_position(n, at.0, at.1);
        n
    };

    let bar_grow = Rc::new(Cell::new(10));
    let bar = reading(&mut scene, (100, 20), (0, 80), &bar_grow);
    assert_eq!(
        scene.outputs_showing(&surface),
        covered,
        "glass above whose reads end 20 px short"
    );
    bar_grow.set(35);
    assert_eq!(
        scene.outputs_showing(&surface),
        shown,
        "glass above reading 5 px of it"
    );
    bar_grow.set(10);

    let low_grow = Rc::new(Cell::new(25));
    let low = reading(&mut scene, (40, 5), (10, 60), &low_grow);
    scene.place_below(low, cover);
    assert_eq!(
        scene.outputs_showing(&surface),
        covered,
        "glass under the cover that no glass reads"
    );
    scene.set_position(low, 10, 72);
    assert_eq!(
        scene.outputs_showing(&surface),
        shown,
        "glass under the cover read by the glass above, reading it"
    );
    scene.destroy(low);
    scene.destroy(bar);

    scene.rect_set_size(cover, (100, 60));
    let beneath = reading(&mut scene, (100, 20), (0, 80), &bar_grow);
    scene.lower_to_bottom(beneath);
    assert_eq!(scene.outputs_showing(&surface), covered, "glass beneath");
    bar_grow.set(35);
    assert_eq!(
        scene.outputs_showing(&surface),
        shown,
        "glass beneath it reading it: its backdrop can hold what lies above"
    );
    scene.destroy(beneath);
    let pane = scene.custom_create(root, Box::new(Pane));
    scene.lower_to_bottom(pane);
    assert_eq!(
        scene.outputs_showing(&surface),
        covered,
        "a custom node that reads all beneath it, beneath it"
    );
}

#[test]
fn a_surface_covers_what_lies_under_its_opaque_region_or_its_buffer_without_alpha() {
    use wl_shm::Format::{Argb8888, Xrgb8888};
    let mut w = World::new();
    let surface = w.surface();
    let mut scene = Scene::<DummyRenderer>::new();
    scene.add_output(&w.left);
    let root = scene.root();
    let s = scene.surface_create(root, surface.clone());
    scene.set_position(s, 10, 10);
    for (format, opaque, covered) in [
        (Argb8888, None, false),
        (Argb8888, Some((0, 0, 100, 30)), false),
        (Argb8888, Some((0, 0, 100, 100)), true),
        (Xrgb8888, None, true),
    ] {
        let c = w.cover(format, opaque);
        let node = scene.surface_create(root, c);
        let showing = scene.outputs_showing(&surface).unwrap();
        assert_eq!(
            showing.is_empty(),
            covered,
            "{format:?} with opaque region {opaque:?}"
        );
        scene.destroy(node);
    }
}

#[test]
fn at_a_fractional_scale_a_cover_reaches_a_pixel_past_the_surface_it_covers() {
    let mut w = World::new();
    let surface = w.surface();
    let left = w.left.clone();
    left.change_current_state(
        None,
        None,
        Some(smithay::output::Scale::Fractional(1.5)),
        None,
    );
    let mut scene = Scene::<DummyRenderer>::new();
    scene.add_output(&left);
    let root = scene.root();
    let s = scene.surface_create(root, surface.clone());
    scene.set_position(s, 10, 10);
    let cover = scene.rect_create(root, (50, 50), [0.0, 0.0, 0.0, 1.0]);
    assert_eq!(
        scene.outputs_showing(&surface),
        Some(vec![left.clone()]),
        "edge on edge"
    );
    scene.rect_set_size(cover, (51, 51));
    assert_eq!(scene.outputs_showing(&surface), Some(vec![]));
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
    scene.send_frames(
        &scene.surface_pass(&w.right),
        Duration::from_millis(16),
        None,
    );
    assert_eq!(
        w.events(),
        [] as [String; 0],
        "the right output is not primary"
    );
    scene.send_frames(
        &scene.surface_pass(&w.left),
        Duration::from_millis(16),
        None,
    );
    assert_eq!(w.events(), ["frame"]);
    w.request_frame();
    scene.set_enabled(s, false);
    scene.send_frames(
        &scene.surface_pass(&w.left),
        Duration::from_millis(32),
        None,
    );
    assert_eq!(w.events(), [] as [String; 0]);
    scene.set_enabled(s, true);
    scene.send_frames(
        &scene.surface_pass(&w.left),
        Duration::from_millis(48),
        None,
    );
    assert_eq!(w.events(), ["frame"]);
    let view = View::of(&w.left).unwrap();
    assert_eq!(view.area.size, (100, 100).into());
}
