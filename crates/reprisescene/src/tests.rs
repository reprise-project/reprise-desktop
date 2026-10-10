//! Unit tests: tree order and every reorder verb, positions, enable, hit
//! testing, outputs and the primary output, element order and Ids, and the
//! damage Smithay's `OutputDamageTracker` reads off the elements.

use std::{cell::Cell, rc::Rc, sync::LazyLock};

use smithay::{
    backend::{
        allocator::{Fourcc, dmabuf::Dmabuf},
        renderer::{
            Color32F, ContextId, DebugFlags, Frame, ImportDma, ImportDmaWl, ImportMem, ImportMemWl,
            Renderer, RendererSuper, Texture, TextureFilter,
            damage::OutputDamageTracker,
            element::{Element, Id, Kind, solid::SolidColorRenderElement},
            sync::SyncPoint,
            test::{DummyError, DummyFramebuffer},
        },
    },
    output::{Mode, Output, PhysicalProperties, Subpixel},
    reexports::wayland_server::protocol::wl_buffer::WlBuffer,
    utils::{Buffer as BufferCoords, Logical, Physical, Point, Rectangle, Scale, Size, Transform},
    wayland::compositor::SurfaceData,
};

use crate::{CustomElement, CustomNode, Input, NodeId, NodeType, Scene, SceneElement, View};

// Smithay's DummyRenderer cannot import memory. This one can. It gets
// `ImportAll` from Smithay's blanket impl; were smithay's `use_system_lib`
// feature on, that impl would also want `ImportEgl` here.

static CONTEXT: LazyLock<ContextId<TestTexture>> = LazyLock::new(ContextId::new);

#[derive(Debug, Default)]
struct TestRenderer;

#[derive(Debug, Clone, PartialEq)]
struct TestTexture(Size<i32, BufferCoords>);

impl Texture for TestTexture {
    fn width(&self) -> u32 {
        self.0.w as u32
    }
    fn height(&self) -> u32 {
        self.0.h as u32
    }
    fn format(&self) -> Option<Fourcc> {
        Some(Fourcc::Argb8888)
    }
}

#[derive(Debug)]
struct TestFrame;

impl Frame for TestFrame {
    type Error = DummyError;
    type TextureId = TestTexture;
    fn context_id(&self) -> ContextId<TestTexture> {
        CONTEXT.clone()
    }
    fn clear(&mut self, _: Color32F, _: &[Rectangle<i32, Physical>]) -> Result<(), DummyError> {
        Ok(())
    }
    fn draw_solid(
        &mut self,
        _: Rectangle<i32, Physical>,
        _: &[Rectangle<i32, Physical>],
        _: Color32F,
    ) -> Result<(), DummyError> {
        Ok(())
    }
    fn render_texture_from_to(
        &mut self,
        _: &TestTexture,
        _: Rectangle<f64, BufferCoords>,
        _: Rectangle<i32, Physical>,
        _: &[Rectangle<i32, Physical>],
        _: &[Rectangle<i32, Physical>],
        _: Transform,
        _: f32,
    ) -> Result<(), DummyError> {
        Ok(())
    }
    fn transformation(&self) -> Transform {
        Transform::Normal
    }
    fn output_size(&self) -> Size<i32, Physical> {
        Size::default()
    }
    fn wait(&mut self, _: &SyncPoint) -> Result<(), DummyError> {
        Ok(())
    }
    fn finish(self) -> Result<SyncPoint, DummyError> {
        Ok(SyncPoint::default())
    }
}

impl RendererSuper for TestRenderer {
    type Error = DummyError;
    type TextureId = TestTexture;
    type Framebuffer<'buffer> = DummyFramebuffer;
    type Frame<'frame, 'buffer>
        = TestFrame
    where
        'buffer: 'frame,
        Self: 'frame;
}

impl Renderer for TestRenderer {
    fn context_id(&self) -> ContextId<TestTexture> {
        CONTEXT.clone()
    }
    fn downscale_filter(&mut self, _: TextureFilter) -> Result<(), DummyError> {
        Ok(())
    }
    fn upscale_filter(&mut self, _: TextureFilter) -> Result<(), DummyError> {
        Ok(())
    }
    fn set_debug_flags(&mut self, _: DebugFlags) {}
    fn debug_flags(&self) -> DebugFlags {
        DebugFlags::empty()
    }
    fn render<'frame, 'buffer>(
        &'frame mut self,
        _: &'frame mut DummyFramebuffer,
        _: Size<i32, Physical>,
        _: Transform,
    ) -> Result<TestFrame, DummyError>
    where
        'buffer: 'frame,
    {
        Ok(TestFrame)
    }
    fn wait(&mut self, _: &SyncPoint) -> Result<(), DummyError> {
        Ok(())
    }
}

impl ImportMem for TestRenderer {
    fn import_memory(
        &mut self,
        _: &[u8],
        _: Fourcc,
        size: Size<i32, BufferCoords>,
        _: bool,
    ) -> Result<TestTexture, DummyError> {
        Ok(TestTexture(size))
    }
    fn update_memory(
        &mut self,
        _: &TestTexture,
        _: &[u8],
        _: Rectangle<i32, BufferCoords>,
    ) -> Result<(), DummyError> {
        Ok(())
    }
    fn mem_formats(&self) -> Box<dyn Iterator<Item = Fourcc>> {
        Box::new([Fourcc::Argb8888].into_iter())
    }
}

impl ImportMemWl for TestRenderer {
    fn import_shm_buffer(
        &mut self,
        _: &WlBuffer,
        _: Option<&SurfaceData>,
        _: &[Rectangle<i32, BufferCoords>],
    ) -> Result<TestTexture, DummyError> {
        unimplemented!("no client buffers in these tests")
    }
}

impl ImportDma for TestRenderer {
    fn import_dmabuf(
        &mut self,
        _: &Dmabuf,
        _: Option<&[Rectangle<i32, BufferCoords>]>,
    ) -> Result<TestTexture, DummyError> {
        unimplemented!("no client buffers in these tests")
    }
}

impl ImportDmaWl for TestRenderer {}

type TestScene = Scene<TestRenderer, &'static str>;

const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
const HALF: [f32; 4] = [0.5, 0.0, 0.0, 0.5];

fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Logical> {
    Rectangle::new((x, y).into(), (w, h).into())
}

fn prect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Physical> {
    Rectangle::new((x, y).into(), (w, h).into())
}

fn pt(x: f64, y: f64) -> Point<f64, Logical> {
    Point::from((x, y))
}

fn output(name: &str, x: i32, y: i32, w: i32, h: i32, scale: f64) -> Output {
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
        size: (w, h).into(),
        refresh: 60_000,
    };
    o.change_current_state(
        Some(mode),
        Some(Transform::Normal),
        Some(smithay::output::Scale::Fractional(scale)),
        Some((x, y).into()),
    );
    o.set_preferred(mode);
    o
}

fn view(w: i32, h: i32) -> View {
    View::new(rect(0, 0, w, h), 1.0)
}

fn ids(scene: &TestScene, v: &View) -> Vec<Id> {
    scene
        .render_elements(&mut TestRenderer, v)
        .iter()
        .map(|e| e.id().clone())
        .collect()
}

fn rect_id(scene: &TestScene, r: crate::RectId) -> Id {
    // A rect's element Id is its colour buffer's; read it off a render.
    let v = view(10_000, 10_000);
    let (pos, _) = scene.coords(r).unwrap();
    scene
        .render_elements(&mut TestRenderer, &v)
        .into_iter()
        .find(|e| {
            matches!(e, SceneElement::Solid(_))
                && e.geometry(Scale::from(1.0)).loc == pos.to_physical(1)
        })
        .map(|e| e.id().clone())
        .expect("the rect renders")
}

#[test]
fn children_stack_bottom_first_in_creation_order() {
    let mut s = TestScene::new();
    let root = s.root();
    let a = s.tree_create(root);
    let b = s.rect_create(root, (1, 1), RED);
    let c = s.buffer_create(root);
    assert_eq!(s.children(root), &[a.node(), b.node(), c.node()]);
    assert_eq!(s.parent(b), Some(root));
    assert_eq!(s.parent(root), None);
    assert_eq!(s.node_type(c), Some(NodeType::Buffer));
}

#[test]
fn changes_count_edits_and_not_reads() {
    let mut s = TestScene::new();
    let root = s.root();
    let r = s.rect_create(root, (4, 4), RED);
    let before = s.changes();
    let _ = (
        s.position(r),
        s.children(root).len(),
        s.extents(r),
        s.node_at(root, (1.0, 1.0).into()),
    );
    assert_eq!(s.changes(), before, "reading the scene changes nothing");
    s.set_position(r, 2, 0);
    assert!(s.changes() > before, "a move counts");
    let moved = s.changes();
    s.destroy(r);
    assert!(s.changes() > moved, "a destroy counts");
}

#[test]
fn raise_lower_and_place_reorder_siblings() {
    let mut s = TestScene::new();
    let root = s.root();
    let [a, b, c] = [0; 3].map(|_| s.tree_create(root).node());
    s.raise_to_top(a);
    assert_eq!(s.children(root), &[b, c, a], "raise_to_top puts a last");
    s.lower_to_bottom(a);
    assert_eq!(s.children(root), &[a, b, c], "lower_to_bottom puts a first");
    s.place_above(a, b);
    assert_eq!(s.children(root), &[b, a, c], "a right above b");
    s.place_below(c, b);
    assert_eq!(s.children(root), &[c, b, a], "c right below b");
    s.place_above(c, a);
    assert_eq!(s.children(root), &[b, a, c], "c right above the top node");
    s.place_below(a, a);
    assert_eq!(
        s.children(root),
        &[b, a, c],
        "placing a node by itself does nothing"
    );
}

#[test]
fn place_needs_a_shared_parent() {
    let mut s = TestScene::new();
    let root = s.root();
    let t = s.tree_create(root);
    let inner = s.tree_create(t);
    let outer = s.tree_create(root);
    s.place_above(inner, outer);
    s.place_below(outer, inner);
    assert_eq!(s.children(root), &[t.node(), outer.node()]);
    assert_eq!(s.children(t), &[inner.node()]);
}

#[test]
fn reparent_moves_to_the_top_and_keeps_the_relative_position() {
    let mut s = TestScene::new();
    let root = s.root();
    let a = s.tree_create(root);
    let b = s.tree_create(root);
    s.set_position(b, 100, 50);
    let a1 = s.tree_create(a);
    let leaf = s.rect_create(a, (5, 5), RED);
    s.set_position(leaf, 3, 4);
    s.reparent(leaf, b);
    assert_eq!(s.children(a), &[a1.node()]);
    assert_eq!(s.children(b), &[leaf.node()]);
    assert_eq!(s.parent(leaf), Some(b));
    assert_eq!(s.coords(leaf), Some(((103, 54).into(), true)));
    let b2 = s.tree_create(b);
    s.reparent(a1, b);
    assert_eq!(
        s.children(b),
        &[leaf.node(), b2.node(), a1.node()],
        "reparented on top"
    );
}

#[test]
fn reparent_under_itself_does_nothing() {
    let mut s = TestScene::new();
    let root = s.root();
    let a = s.tree_create(root);
    let b = s.tree_create(a);
    let c = s.tree_create(b);
    s.reparent(a, c);
    s.reparent(a, a);
    assert_eq!(s.parent(a), Some(root));
    assert_eq!(s.children(c), &[] as &[NodeId]);
}

#[test]
fn destroy_takes_the_subtree_and_stale_handles_do_nothing() {
    let mut s = TestScene::new();
    let root = s.root();
    let a = s.tree_create(root);
    let b = s.rect_create(a, (1, 1), RED);
    let keep = s.tree_create(root);
    s.destroy(a);
    assert!(!s.is_alive(a));
    assert!(!s.is_alive(b), "the child goes with its tree");
    assert_eq!(s.children(root), &[keep.node()]);
    // The slot is reused; the old handle still names nothing.
    let fresh = s.tree_create(root);
    assert!(s.is_alive(fresh));
    s.set_position(a, 9, 9);
    s.set_enabled(b, false);
    s.raise_to_top(a);
    assert_eq!(s.coords(a), None);
    assert_eq!(s.children(root), &[keep.node(), fresh.node()]);
    assert_eq!(s.coords(fresh), Some(((0, 0).into(), true)));
}

#[test]
fn destroying_the_root_empties_it() {
    let mut s = TestScene::new();
    let root = s.root();
    let a = s.tree_create(root);
    s.destroy(root);
    assert!(s.is_alive(root));
    assert!(!s.is_alive(a));
    assert!(s.children(root).is_empty());
}

#[test]
#[should_panic(expected = "another scene")]
fn a_node_of_another_scene_is_a_bug() {
    let mut s1 = TestScene::new();
    let s2 = TestScene::new();
    let a = s1.tree_create(s1.root());
    let _ = s2.is_alive(a);
}

#[test]
fn coords_add_up_through_nested_trees() {
    let mut s = TestScene::new();
    let root = s.root();
    let a = s.tree_create(root);
    s.set_position(a, 10, 20);
    let b = s.tree_create(a);
    s.set_position(b, -3, 5);
    let leaf = s.buffer_create(b);
    s.set_position(leaf, 1, 1);
    assert_eq!(s.position(leaf), Some((1, 1).into()));
    assert_eq!(s.coords(leaf), Some(((8, 26).into(), true)));
    s.set_position(a, 0, 0);
    assert_eq!(s.coords(leaf), Some(((-2, 6).into(), true)));
}

#[test]
fn a_disabled_ancestor_hides_a_node() {
    let mut s = TestScene::new();
    let root = s.root();
    let a = s.tree_create(root);
    let r = s.rect_create(a, (10, 10), RED);
    s.set_enabled(a, false);
    assert!(s.enabled(r), "the node's own flag stays");
    assert_eq!(s.coords(r), Some(((0, 0).into(), false)));
    assert_eq!(s.extents(r), None);
    assert!(
        s.render_elements(&mut TestRenderer, &view(100, 100))
            .is_empty()
    );
    assert_eq!(s.node_at(root, pt(5.0, 5.0)), None);
    s.set_enabled(a, true);
    assert_eq!(
        s.render_elements(&mut TestRenderer, &view(100, 100)).len(),
        1
    );
    assert_eq!(
        s.node_at(root, pt(5.0, 5.0)).map(|h| h.node),
        Some(r.node())
    );
}

#[test]
fn extents_cover_the_visible_leaves() {
    let mut s = TestScene::new();
    let root = s.root();
    let t = s.tree_create(root);
    s.set_position(t, 100, 100);
    let a = s.rect_create(t, (10, 10), RED);
    let b = s.rect_create(t, (10, 10), RED);
    s.set_position(b, 30, 40);
    let hidden = s.rect_create(t, (500, 500), RED);
    s.set_enabled(hidden, false);
    assert_eq!(s.extents(t), Some(rect(100, 100, 40, 50)));
    assert_eq!(s.extents(a), Some(rect(100, 100, 10, 10)));
    let empty = s.tree_create(root);
    assert_eq!(s.extents(empty), None, "an empty tree shows nothing");
}

#[test]
fn find_data_walks_up_to_the_nearest_tag() {
    let mut s = TestScene::new();
    let root = s.root();
    let view_tree = s.tree_create(root);
    s.set_data(view_tree, Some("taskbar"));
    let inner = s.tree_create(view_tree);
    let leaf = s.buffer_create(inner);
    assert_eq!(s.find_data(leaf), Some((view_tree.node(), &"taskbar")));
    s.set_data(inner, Some("button"));
    assert_eq!(s.find_data(leaf), Some((inner.node(), &"button")));
    assert_eq!(s.data(leaf), None);
    s.set_data(inner, None);
    s.set_data(view_tree, None);
    assert_eq!(s.find_data(leaf), None);
}

#[test]
fn node_at_finds_the_topmost_leaf_with_local_coordinates() {
    let mut s = TestScene::new();
    let root = s.root();
    let below = s.rect_create(root, (100, 100), RED);
    let t = s.tree_create(root);
    s.set_position(t, 50, 50);
    let above = s.rect_create(t, (20, 20), RED);
    s.set_position(above, 5, 5);
    let h = s.node_at(root, pt(60.5, 57.0)).unwrap();
    assert_eq!(h.node, above.node());
    assert_eq!(h.local, pt(5.5, 2.0));
    assert_eq!(h.surface, None);
    assert_eq!(s.node_at(root, pt(54.9, 54.9)).unwrap().node, below.node());
    // The right and bottom edges are outside.
    assert_eq!(s.node_at(root, pt(75.0, 60.0)).unwrap().node, below.node());
    assert_eq!(
        s.node_at(root, pt(74.99, 74.99)).unwrap().node,
        above.node()
    );
    assert_eq!(s.node_at(root, pt(100.0, 10.0)), None);
    assert_eq!(s.node_at(root, pt(-0.5, 10.0)), None);
    s.raise_to_top(below);
    assert_eq!(s.node_at(root, pt(60.0, 60.0)).unwrap().node, below.node());
}

#[test]
fn node_at_skips_passthrough_and_honours_input_regions() {
    let mut s = TestScene::new();
    let root = s.root();
    let below = s.rect_create(root, (100, 100), RED);
    let panel = s.buffer_create(root);
    s.buffer_set_canvas(panel, &[0; 100 * 100], 100, 100);
    s.set_input(panel, Input::Passthrough);
    assert_eq!(
        s.node_at(root, pt(50.0, 50.0)).unwrap().node,
        below.node(),
        "passes through"
    );
    s.set_input(panel, Input::Rect(rect(10, 10, 80, 80)));
    assert_eq!(s.node_at(root, pt(50.0, 50.0)).unwrap().node, panel.node());
    assert_eq!(
        s.node_at(root, pt(5.0, 5.0)).unwrap().node,
        below.node(),
        "the shadow lets it through"
    );
    let seen = Rc::new(Cell::new(pt(-1.0, -1.0)));
    let seen2 = seen.clone();
    s.set_input(
        panel,
        Input::Test(Box::new(move |p| {
            seen2.set(p);
            p.x < 50.0
        })),
    );
    assert_eq!(s.node_at(root, pt(20.0, 30.0)).unwrap().node, panel.node());
    assert_eq!(
        seen.get(),
        pt(20.0, 30.0),
        "the test gets node-local coordinates"
    );
    assert_eq!(s.node_at(root, pt(70.0, 30.0)).unwrap().node, below.node());
}

#[test]
fn node_at_from_a_subtree_uses_layout_coordinates() {
    let mut s = TestScene::new();
    let root = s.root();
    let t = s.tree_create(root);
    s.set_position(t, 200, 100);
    let inner = s.tree_create(t);
    let r = s.rect_create(inner, (10, 10), RED);
    let other = s.rect_create(root, (1000, 1000), RED);
    assert_eq!(s.node_at(inner, pt(205.0, 105.0)).unwrap().node, r.node());
    assert_eq!(
        s.node_at(inner, pt(5.0, 5.0)),
        None,
        "nothing of the subtree there"
    );
    assert_eq!(s.node_at(root, pt(5.0, 5.0)).unwrap().node, other.node());
}

#[test]
fn a_buffer_hits_at_its_dest_size() {
    let mut s = TestScene::new();
    let root = s.root();
    let b = s.buffer_create(root);
    assert_eq!(
        s.node_at(root, pt(1.0, 1.0)),
        None,
        "an empty buffer has no size"
    );
    s.buffer_set_canvas(b, &[0; 40 * 20], 40, 20);
    assert_eq!(s.buffer_size(b), Some((40, 20).into()));
    s.buffer_set_dest_size(b, Some((20, 10).into()));
    assert!(s.node_at(root, pt(19.0, 9.0)).is_some());
    assert!(s.node_at(root, pt(21.0, 9.0)).is_none());
    s.buffer_set_dest_size(b, None);
    s.buffer_set_source_box(
        b,
        Some(Rectangle::new((10.0, 0.0).into(), (30.0, 5.0).into())),
    );
    assert_eq!(
        s.buffer_size(b),
        Some((30, 5).into()),
        "the source box sets the natural size"
    );
}

#[test]
fn outputs_of_lists_the_outputs_a_node_overlaps() {
    let mut s = TestScene::new();
    let root = s.root();
    let left = output("L", 0, 0, 1000, 800, 1.0);
    let right = output("R", 1000, 0, 2000, 1600, 2.0);
    s.add_output(&left);
    s.add_output(&right);
    s.add_output(&left);
    assert_eq!(
        s.outputs(),
        &[left.clone(), right.clone()],
        "an output is added once"
    );
    let r = s.rect_create(root, (100, 100), RED);
    s.set_position(r, 950, 10);
    assert_eq!(s.outputs_of(r), vec![left.clone(), right.clone()]);
    s.set_position(r, 900, 10);
    assert_eq!(
        s.outputs_of(r),
        vec![left.clone()],
        "touching the edge is not on it"
    );
    s.set_position(r, 1500, 700);
    assert_eq!(s.outputs_of(r), vec![right.clone()]);
    s.set_position(r, 2000, 900);
    assert!(
        s.outputs_of(r).is_empty(),
        "the 2x output is 1000x800 logical"
    );
    s.set_enabled(r, false);
    s.set_position(r, 10, 10);
    assert!(s.outputs_of(r).is_empty(), "a hidden node is on no output");
    s.remove_output(&left);
    s.set_enabled(r, true);
    assert!(s.outputs_of(r).is_empty());
}

#[test]
fn primary_output_is_the_largest_overlap_ties_to_the_first_added() {
    let mut s = TestScene::new();
    let root = s.root();
    let left = output("L", 0, 0, 1000, 800, 1.0);
    let right = output("R", 1000, 0, 1000, 800, 1.0);
    s.add_output(&left);
    s.add_output(&right);
    let r = s.rect_create(root, (100, 100), RED);
    s.set_position(r, 940, 0);
    assert_eq!(
        s.primary_output(r),
        Some(left.clone()),
        "60 of 100 on the left"
    );
    s.set_position(r, 960, 0);
    assert_eq!(
        s.primary_output(r),
        Some(right.clone()),
        "60 of 100 on the right"
    );
    s.set_position(r, 950, 0);
    assert_eq!(
        s.primary_output(r),
        Some(left.clone()),
        "a tie goes to the output added first"
    );
    let mut s2 = TestScene::new();
    let r2 = s2.rect_create(s2.root(), (100, 100), RED);
    s2.set_position(r2, 950, 0);
    s2.add_output(&right);
    s2.add_output(&left);
    assert_eq!(
        s2.primary_output(r2),
        Some(right.clone()),
        "added first, so it wins the tie"
    );
    s.set_position(r, 5000, 0);
    assert_eq!(s.primary_output(r), None);
}

#[test]
fn elements_come_front_to_back() {
    let mut s = TestScene::new();
    let root = s.root();
    let bottom = s.rect_create(root, (10, 10), RED);
    let t = s.tree_create(root);
    let mid = s.rect_create(t, (10, 10), RED);
    s.set_position(mid, 20, 0);
    let mid_top = s.rect_create(t, (10, 10), RED);
    s.set_position(mid_top, 40, 0);
    let top = s.rect_create(root, (10, 10), RED);
    s.set_position(top, 60, 0);
    let v = view(100, 100);
    let [b, m, mt, tp] = [bottom, mid, mid_top, top].map(|r| rect_id(&s, r));
    assert_eq!(
        ids(&s, &v),
        vec![tp.clone(), mt.clone(), m.clone(), b.clone()]
    );
    s.raise_to_top(t);
    assert_eq!(
        ids(&s, &v),
        vec![mt.clone(), m.clone(), tp.clone(), b.clone()]
    );
    s.place_below(mid_top, mid);
    assert_eq!(ids(&s, &v), vec![m, mt, tp, b], "Ids stay with their nodes");
}

#[test]
fn a_hoisted_node_draws_above_its_tree_and_nowhere_else() {
    let mut s = TestScene::new();
    let root = s.root();
    let windows = s.tree_create(root);
    let on_top = s.tree_create(root);
    let low = s.tree_create(windows);
    let low_rect = s.rect_create(low, (20, 20), RED);
    let high = s.rect_create(windows, (20, 20), RED);
    s.set_position(high, 10, 10);
    let pinned = s.rect_create(on_top, (10, 10), RED);
    s.set_position(pinned, 50, 0);
    let bar = s.rect_create(root, (10, 10), RED);
    s.set_position(bar, 80, 0);
    let v = view(100, 100);
    let [l, h, p, b] = [low_rect, high, pinned, bar].map(|r| rect_id(&s, r));
    let before = vec![b.clone(), p.clone(), h.clone(), l.clone()];
    assert_eq!(ids(&s, &v), before);

    let changes = s.changes();
    s.set_hoist(on_top, vec![low.into()]);
    assert!(s.changes() > changes, "a new hoist counts as a change");
    let changes = s.changes();
    s.set_hoist(on_top, vec![low.into()]);
    assert_eq!(s.changes(), changes, "the same hoist again changes nothing");
    assert_eq!(
        ids(&s, &v),
        vec![b.clone(), l.clone(), p.clone(), h.clone()],
        "over the pinned rect, under the bar, gone from its own place"
    );
    assert_eq!(s.children(windows), &[low.into(), high.into()]);
    assert_eq!(
        s.node_at(root, (15.0, 15.0).into()).map(|x| x.node),
        Some(high.into()),
        "the pointer still finds the stacked order"
    );

    s.set_enabled(windows, false);
    assert_eq!(
        ids(&s, &v),
        vec![b.clone(), p.clone()],
        "a hidden ancestor hides it"
    );
    s.set_enabled(windows, true);
    s.set_hoist(on_top, vec![]);
    assert_eq!(ids(&s, &v), before, "back in place");
}

#[test]
fn elements_sit_where_the_view_puts_them() {
    let mut s = TestScene::new();
    let root = s.root();
    let t = s.tree_create(root);
    s.set_position(t, 1000, 100);
    let r = s.rect_create(t, (40, 20), RED);
    s.set_position(r, 10, 10);
    let v = View::new(rect(1000, 0, 1000, 800), 2.0);
    let els = s.render_elements(&mut TestRenderer, &v);
    assert_eq!(els.len(), 1);
    assert_eq!(els[0].geometry(v.scale), prect(20, 220, 80, 40));
    let off = View::new(rect(0, 0, 1000, 800), 1.0);
    assert!(
        s.render_elements(&mut TestRenderer, &off).is_empty(),
        "outside the view"
    );
    let o = output("O", 1000, 0, 2000, 1600, 2.0);
    assert_eq!(View::of(&o), Some(v));
    assert_eq!(s.render_elements_for(&mut TestRenderer, &o).len(), 1);
}

#[test]
fn a_subtree_renders_alone_where_it_stands_whatever_hides_it() {
    let mut s = TestScene::new();
    let root = s.root();
    let layer = s.tree_create(root);
    s.set_position(layer, 100, 0);
    let below = s.rect_create(layer, (10, 10), RED);
    let w = s.tree_create(layer);
    s.set_position(w, 10, 20);
    let shown = s.rect_create(w, (10, 10), RED);
    let off = s.rect_create(w, (10, 10), RED);
    s.set_position(off, 50, 50);
    s.set_enabled(off, false);
    let above = s.rect_create(root, (500, 500), RED);
    s.set_enabled(layer, false);
    s.set_enabled(w, false);
    s.set_opacity(w, 0.0);
    let v = View::new(rect(100, 0, 100, 100), 2.0);
    assert_eq!(
        s.render_elements(&mut TestRenderer, &v).len(),
        1,
        "the cover"
    );
    let els = s.render_elements_of(&mut TestRenderer, w, &v);
    assert_eq!(els.len(), 1, "the subtree's enabled leaf alone");
    assert_eq!(els[0].geometry(v.scale), prect(20, 40, 20, 20));
    assert_eq!(els[0].alpha(), 1.0);
    let _ = (below, shown, above);
}

#[test]
fn opacity_multiplies_down_and_zero_draws_nothing() {
    let mut s = TestScene::new();
    let root = s.root();
    let t = s.tree_create(root);
    let r = s.rect_create(t, (10, 10), RED);
    let b = s.buffer_create(t);
    s.buffer_set_canvas(b, &[0xff00_0000; 4], 2, 2);
    s.set_position(b, 20, 0);
    s.set_opacity(t, 0.5);
    s.set_opacity(b, 0.5);
    let els = s.render_elements(&mut TestRenderer, &view(100, 100));
    let alphas: Vec<f32> = els.iter().map(|e| e.alpha()).collect();
    assert_eq!(alphas, vec![0.25, 0.5], "buffer 0.5 x 0.5, rect 0.5");
    s.set_opacity(r, 0.0);
    assert_eq!(
        s.render_elements(&mut TestRenderer, &view(100, 100)).len(),
        1
    );
}

#[test]
fn a_canvas_and_a_texture_render_cropped_and_scaled() {
    let mut s = TestScene::new();
    let root = s.root();
    let b = s.buffer_create(root);
    s.buffer_set_canvas(b, &[0; 8 * 4], 8, 4);
    s.buffer_set_dest_size(b, Some((4, 2).into()));
    let els = s.render_elements(&mut TestRenderer, &view(100, 100));
    assert!(matches!(els[0], SceneElement::Canvas(_)));
    assert_eq!(els[0].geometry(Scale::from(1.0)), prect(0, 0, 4, 2));
    assert_eq!(
        els[0].src(),
        Rectangle::new((0.0, 0.0).into(), (8.0, 4.0).into())
    );
    s.buffer_set_source_box(
        b,
        Some(Rectangle::new((2.0, 1.0).into(), (4.0, 2.0).into())),
    );
    let els = s.render_elements(&mut TestRenderer, &view(100, 100));
    assert_eq!(
        els[0].src(),
        Rectangle::new((2.0, 1.0).into(), (4.0, 2.0).into())
    );
    s.buffer_set_texture(b, &TestRenderer, TestTexture((16, 16).into()));
    s.buffer_set_source_box(b, None);
    s.buffer_set_dest_size(b, None);
    let els = s.render_elements(&mut TestRenderer, &view(100, 100));
    assert!(matches!(els[0], SceneElement::Texture(_)));
    assert_eq!(els[0].geometry(Scale::from(1.0)), prect(0, 0, 16, 16));
    s.buffer_clear(b);
    assert!(!s.buffer_has_pixels(b));
    assert!(
        s.render_elements(&mut TestRenderer, &view(100, 100))
            .is_empty()
    );
}

#[test]
fn opaque_region_is_reported_in_whole_physical_pixels() {
    let mut s = TestScene::new();
    let root = s.root();
    let b = s.buffer_create(root);
    s.buffer_set_canvas(b, &[0; 100], 10, 10);
    s.set_position(b, 1, 0);
    s.buffer_set_opaque_region(b, vec![rect(1, 1, 8, 8), rect(-5, -5, 6, 6)]);
    let v = View::new(rect(0, 0, 100, 100), 1.5);
    let els = s.render_elements(&mut TestRenderer, &v);
    // At x = 1.5 physical the element starts at 2; (1,1)-(9,9) is
    // (3.0,1.5)-(15.0,13.5), whole pixels (3,2)-(15,13), so (1,2) 12x11.
    // The second rect is clipped to (0,0) 1x1: (1.5,0)-(3,1.5) gives
    // (2,0)-(3,1), so (0,0) 1x1.
    let opaque: Vec<_> = els[0].opaque_regions(v.scale).into_iter().collect();
    assert_eq!(opaque, vec![prect(1, 2, 12, 11), prect(0, 0, 1, 1)]);
    s.set_opacity(b, 0.9);
    let els = s.render_elements(&mut TestRenderer, &v);
    assert!(
        els[0].opaque_regions(v.scale).into_iter().next().is_none(),
        "translucent hides nothing"
    );
}

struct Square(Id);

impl CustomNode<TestRenderer> for Square {
    fn size(&self) -> Size<i32, Logical> {
        (30, 30).into()
    }
    fn elements(
        &self,
        _: &mut TestRenderer,
        location: Point<f64, Physical>,
        scale: Scale<f64>,
        alpha: f32,
    ) -> Vec<CustomElement<TestRenderer>> {
        let geo = Rectangle::new(
            location.to_i32_round(),
            Size::from((30, 30)).to_physical_precise_round(scale),
        );
        let e = SolidColorRenderElement::new(
            self.0.clone(),
            geo,
            0usize,
            Color32F::from(RED) * alpha,
            Kind::Unspecified,
        );
        vec![CustomElement::new(e)]
    }
}

#[test]
fn a_custom_node_renders_and_hits_like_a_leaf() {
    let mut s = TestScene::new();
    let root = s.root();
    let below = s.rect_create(root, (100, 100), RED);
    let id = Id::new();
    let c = s.custom_create(root, Box::new(Square(id.clone())));
    s.set_position(c, 10, 10);
    s.set_opacity(c, 0.5);
    let els = s.render_elements(&mut TestRenderer, &view(100, 100));
    assert_eq!(els[0].id(), &id);
    assert_eq!(els[0].geometry(Scale::from(1.0)), prect(10, 10, 30, 30));
    assert_eq!(els[0].alpha(), 0.5);
    assert_eq!(s.node_at(root, pt(15.0, 15.0)).unwrap().node, c.node());
    s.set_input(c, Input::Passthrough);
    assert_eq!(s.node_at(root, pt(15.0, 15.0)).unwrap().node, below.node());
    assert_eq!(s.as_custom(c.node()), Some(c));
    assert_eq!(s.custom(c).map(|n| n.size()), Some((30, 30).into()));
}

#[test]
fn the_backdrop_leaves_out_the_marked_subtrees_and_custom_nodes() {
    let mut s = TestScene::new();
    let root = s.root();
    s.rect_create(root, (100, 100), RED);
    let chrome = s.tree_create(root);
    let frame = s.rect_create(chrome, (50, 50), RED);
    s.custom_create(root, Box::new(Square(Id::new())));
    s.set_in_backdrop(chrome, false);
    assert!(!s.in_backdrop(chrome));
    assert!(s.in_backdrop(frame), "the flag is the subtree's own");
    let ids = |els: Vec<SceneElement<TestRenderer>>| -> Vec<Id> {
        els.iter().map(|e| e.id().clone()).collect()
    };
    let full = ids(s.render_elements(&mut TestRenderer, &view(100, 100)));
    assert_eq!(full.len(), 3, "the screen draws all three");
    let back = ids(s.backdrop_elements(&mut TestRenderer, &view(100, 100)));
    assert_eq!(back.len(), 1, "only the wallpaper: {back:?}");
    assert_eq!(back[0], full[2], "with the screen's Id");
    s.set_in_backdrop(chrome, true);
    let back = s.backdrop_elements(&mut TestRenderer, &view(100, 100));
    assert_eq!(back.len(), 2, "the frame is back, the custom node never");
}

/// Two windows on a wallpaper under a popup layer, each window a tree
/// holding its chrome (left out of the backdrop: glass and a frame) and
/// a client. Ids: the wallpaper, the lower window's frame and client, the
/// upper's, the popup.
struct Stack {
    s: TestScene,
    lower: crate::TreeId,
    upper: crate::TreeId,
    ids: [Id; 6],
}

fn stack() -> Stack {
    let mut s = TestScene::new();
    let root = s.root();
    let layer = s.tree_create(root);
    let wall = s.rect_create(layer, (100, 100), RED);
    let windows = s.tree_create(root);
    let window = |s: &mut TestScene, x: i32| {
        let tree = s.tree_create(windows);
        s.set_position(tree, x, 10);
        let chrome = s.tree_create(tree);
        s.set_in_backdrop(chrome, false);
        s.custom_create(chrome, Box::new(Square(Id::new())));
        let frame = s.rect_create(chrome, (40, 40), RED);
        let client = s.rect_create(tree, (30, 30), RED);
        s.set_position(client, 4, 4);
        (tree, frame, client)
    };
    let (lower, lower_frame, lower_client) = window(&mut s, 10);
    let (upper, upper_frame, upper_client) = window(&mut s, 30);
    let menus = s.tree_create(root);
    s.set_in_backdrop(menus, false);
    let popup = s.rect_create(menus, (10, 10), RED);
    s.set_position(popup, 85, 85);
    let ids = [
        wall,
        lower_frame,
        lower_client,
        upper_frame,
        upper_client,
        popup,
    ]
    .map(|r| rect_id(&s, r));
    Stack {
        s,
        lower,
        upper,
        ids,
    }
}

#[test]
fn what_lies_below_a_subtree_keeps_the_chrome_and_leaves_out_the_glass() {
    let Stack {
        s,
        lower,
        upper,
        ids,
    } = stack();
    let [wall, lower_frame, lower_client, ..] = ids;
    let below = |node| -> Vec<Id> {
        s.elements_below(&mut TestRenderer, node, &view(100, 100))
            .iter()
            .map(|e| e.id().clone())
            .collect()
    };
    assert_eq!(
        below(upper),
        vec![lower_client, lower_frame, wall.clone()],
        "the lower window with its frame and without its glass; nothing of the upper or above"
    );
    assert_eq!(
        below(lower),
        vec![wall],
        "only the wallpaper under the lower"
    );
}

#[test]
fn chrome_below_a_subtree_is_found_where_it_meets_an_area() {
    let Stack {
        mut s,
        lower,
        upper,
        ..
    } = stack();
    assert!(
        s.left_out_below(upper, rect(45, 20, 10, 10)),
        "the lower frame runs to x 50"
    );
    assert!(
        !s.left_out_below(upper, rect(60, 20, 10, 10)),
        "past the lower frame only the wallpaper"
    );
    assert!(
        !s.left_out_below(lower, rect(0, 0, 100, 100)),
        "nothing is left out under the lowest window"
    );
    s.set_enabled(lower, false);
    assert!(
        !s.left_out_below(upper, rect(45, 20, 10, 10)),
        "a hidden window is not there"
    );
}

struct Pond(Id);

impl CustomNode<TestRenderer> for Pond {
    fn size(&self) -> Size<i32, Logical> {
        (100, 100).into()
    }
    fn in_backdrop(&self) -> bool {
        true
    }
    fn elements(
        &self,
        r: &mut TestRenderer,
        location: Point<f64, Physical>,
        scale: Scale<f64>,
        alpha: f32,
    ) -> Vec<CustomElement<TestRenderer>> {
        Square(self.0.clone()).elements(r, location, scale, alpha)
    }
}

#[test]
fn a_custom_node_may_ask_to_be_in_the_backdrop() {
    let mut s = TestScene::new();
    let root = s.root();
    let id = Id::new();
    s.custom_create(root, Box::new(Pond(id.clone())));
    s.custom_create(root, Box::new(Square(Id::new())));
    let back = s.backdrop_elements(&mut TestRenderer, &view(100, 100));
    assert_eq!(back.len(), 1, "the pond, not the glass");
    assert_eq!(back[0].id(), &id);
}

/// Drives Smithay's damage tracker over a scene, one frame per call.
struct Tracker(OutputDamageTracker, View);

impl Tracker {
    fn new(w: i32, h: i32, scale: f64) -> Self {
        let size = Size::<i32, Logical>::from((w, h))
            .to_f64()
            .to_physical(scale)
            .to_i32_round();
        Tracker(
            OutputDamageTracker::new(size, scale, Transform::Normal),
            View::new(rect(0, 0, w, h), scale),
        )
    }

    fn frame(&mut self, s: &TestScene) -> Vec<Rectangle<i32, Physical>> {
        let els = s.render_elements(&mut TestRenderer, &self.1);
        let (damage, _) = self.0.damage_output(1, &els).unwrap();
        damage.cloned().unwrap_or_default()
    }
}

fn area(damage: &[Rectangle<i32, Physical>]) -> i64 {
    let mut seen: Vec<Rectangle<i32, Physical>> = Vec::new();
    let mut total = 0;
    for d in damage {
        let parts = Rectangle::subtract_rects_many([*d], seen.iter().copied());
        total += parts
            .iter()
            .map(|r| r.size.w as i64 * r.size.h as i64)
            .sum::<i64>();
        seen.push(*d);
    }
    total
}

fn covers_exactly(damage: &[Rectangle<i32, Physical>], want: &[Rectangle<i32, Physical>]) -> bool {
    let uncovered = want
        .iter()
        .flat_map(|w| Rectangle::subtract_rects_many([*w], damage.iter().copied()))
        .count();
    uncovered == 0 && area(damage) == area(want)
}

#[test]
fn an_unchanged_scene_has_no_damage() {
    let mut s = TestScene::new();
    let root = s.root();
    let r = s.rect_create(root, (10, 10), RED);
    s.set_position(r, 5, 5);
    let mut t = Tracker::new(100, 100, 1.0);
    assert!(
        covers_exactly(&t.frame(&s), &[prect(0, 0, 100, 100)]),
        "the first frame is whole"
    );
    assert!(t.frame(&s).is_empty());
    assert!(t.frame(&s).is_empty());
}

#[test]
fn moving_a_node_damages_its_old_and_new_place() {
    let mut s = TestScene::new();
    let root = s.root();
    let tree = s.tree_create(root);
    let r = s.rect_create(tree, (10, 10), HALF);
    s.set_position(r, 5, 5);
    let mut t = Tracker::new(100, 100, 1.0);
    t.frame(&s);
    s.set_position(tree, 40, 0);
    let d = t.frame(&s);
    assert!(
        covers_exactly(&d, &[prect(5, 5, 10, 10), prect(45, 5, 10, 10)]),
        "{d:?}"
    );
}

#[test]
fn moving_damages_in_physical_pixels() {
    let mut s = TestScene::new();
    let root = s.root();
    let r = s.rect_create(root, (10, 10), HALF);
    let mut t = Tracker::new(100, 100, 2.0);
    t.frame(&s);
    s.set_position(r, 50, 0);
    let d = t.frame(&s);
    assert!(
        covers_exactly(&d, &[prect(0, 0, 20, 20), prect(100, 0, 20, 20)]),
        "{d:?}"
    );
}

#[test]
fn disabling_or_destroying_damages_the_place_it_left() {
    let mut s = TestScene::new();
    let root = s.root();
    let r = s.rect_create(root, (10, 10), HALF);
    s.set_position(r, 20, 30);
    let mut t = Tracker::new(100, 100, 1.0);
    t.frame(&s);
    s.set_enabled(r, false);
    assert!(covers_exactly(&t.frame(&s), &[prect(20, 30, 10, 10)]));
    assert!(t.frame(&s).is_empty());
    s.set_enabled(r, true);
    assert!(covers_exactly(&t.frame(&s), &[prect(20, 30, 10, 10)]));
    s.destroy(r);
    assert!(covers_exactly(&t.frame(&s), &[prect(20, 30, 10, 10)]));
}

#[test]
fn a_new_canvas_damages_the_buffer() {
    let mut s = TestScene::new();
    let root = s.root();
    let b = s.buffer_create(root);
    s.set_position(b, 10, 10);
    s.buffer_set_canvas(b, &[0; 20 * 10], 20, 10);
    let mut t = Tracker::new(100, 100, 1.0);
    let id = s.render_elements(&mut TestRenderer, &t.1)[0].id().clone();
    t.frame(&s);
    s.buffer_set_canvas(b, &[1; 20 * 10], 20, 10);
    assert_eq!(
        s.render_elements(&mut TestRenderer, &t.1)[0].id(),
        &id,
        "same size keeps the Id"
    );
    assert!(covers_exactly(&t.frame(&s), &[prect(10, 10, 20, 10)]));
    assert!(t.frame(&s).is_empty());
    s.buffer_set_canvas(b, &[1; 30 * 10], 30, 10);
    assert!(
        covers_exactly(&t.frame(&s), &[prect(10, 10, 30, 10)]),
        "a wider canvas damages its new size"
    );
}

#[test]
fn a_new_texture_damages_whole_or_where_told() {
    let mut s = TestScene::new();
    let root = s.root();
    let b = s.buffer_create(root);
    s.buffer_set_texture(b, &TestRenderer, TestTexture((50, 50).into()));
    let mut t = Tracker::new(100, 100, 1.0);
    t.frame(&s);
    s.buffer_set_texture(b, &TestRenderer, TestTexture((50, 50).into()));
    assert!(covers_exactly(&t.frame(&s), &[prect(0, 0, 50, 50)]));
    s.buffer_set_texture_with_damage(
        b,
        &TestRenderer,
        TestTexture((50, 50).into()),
        &[Rectangle::new((10, 20).into(), (5, 5).into())],
    );
    assert!(covers_exactly(&t.frame(&s), &[prect(10, 20, 5, 5)]));
    s.buffer_set_texture_with_damage(b, &TestRenderer, TestTexture((50, 50).into()), &[]);
    assert!(
        t.frame(&s).is_empty(),
        "a swap with no damage draws nothing"
    );
}

#[test]
fn opacity_colour_and_size_changes_damage_the_node() {
    let mut s = TestScene::new();
    let root = s.root();
    let r = s.rect_create(root, (10, 10), HALF);
    s.set_position(r, 50, 50);
    let mut t = Tracker::new(100, 100, 1.0);
    t.frame(&s);
    s.set_opacity(r, 0.5);
    assert!(
        covers_exactly(&t.frame(&s), &[prect(50, 50, 10, 10)]),
        "opacity"
    );
    s.rect_set_color(r, [0.0, 0.0, 0.5, 0.5]);
    assert!(
        covers_exactly(&t.frame(&s), &[prect(50, 50, 10, 10)]),
        "colour"
    );
    s.rect_set_color(r, [0.0, 0.0, 0.5, 0.5]);
    assert!(t.frame(&s).is_empty(), "the same colour again");
    s.rect_set_size(r, (20, 10));
    assert!(
        covers_exactly(&t.frame(&s), &[prect(50, 50, 20, 10)]),
        "size"
    );
    assert_eq!(s.rect_size(r), Some((20, 10).into()));
    assert_eq!(s.rect_color(r), Some([0.0, 0.0, 0.5, 0.5]));
}

#[test]
fn source_box_and_dest_size_changes_damage_the_buffer() {
    let mut s = TestScene::new();
    let root = s.root();
    let b = s.buffer_create(root);
    s.buffer_set_canvas(b, &[0; 40 * 40], 40, 40);
    let mut t = Tracker::new(100, 100, 1.0);
    t.frame(&s);
    s.buffer_set_source_box(
        b,
        Some(Rectangle::new((0.0, 0.0).into(), (20.0, 20.0).into())),
    );
    s.buffer_set_dest_size(b, Some((40, 40).into()));
    assert!(
        covers_exactly(&t.frame(&s), &[prect(0, 0, 40, 40)]),
        "same place, new crop"
    );
    s.buffer_set_dest_size(b, Some((10, 10).into()));
    assert!(
        covers_exactly(&t.frame(&s), &[prect(0, 0, 40, 40)]),
        "shrunk from 40x40"
    );
}

#[test]
fn restacking_overlapping_nodes_damages_them() {
    let mut s = TestScene::new();
    let root = s.root();
    let a = s.rect_create(root, (20, 20), HALF);
    let b = s.rect_create(root, (20, 20), HALF);
    s.set_position(b, 10, 10);
    let mut t = Tracker::new(100, 100, 1.0);
    t.frame(&s);
    s.raise_to_top(a);
    let d = t.frame(&s);
    assert!(!d.is_empty());
    let bbox = d.iter().copied().reduce(|x, y| x.merge(y)).unwrap();
    assert!(prect(0, 0, 30, 30).contains_rect(bbox), "{d:?}");
}

#[test]
fn hoisting_a_node_and_ending_it_damage_the_node() {
    let mut s = TestScene::new();
    let root = s.root();
    let windows = s.tree_create(root);
    let on_top = s.tree_create(root);
    let a = s.rect_create(windows, (20, 20), HALF);
    let b = s.rect_create(windows, (20, 20), HALF);
    s.set_position(b, 10, 10);
    let far = s.rect_create(on_top, (10, 10), HALF);
    s.set_position(far, 80, 80);
    let mut t = Tracker::new(100, 100, 1.0);
    t.frame(&s);
    s.set_hoist(on_top, vec![a.into()]);
    let d = t.frame(&s);
    let covered = |d: &[Rectangle<i32, Physical>]| {
        Rectangle::subtract_rects_many([prect(0, 0, 20, 20)], d.iter().copied()).is_empty()
    };
    assert!(covered(&d), "the hoisted node redraws: {d:?}");
    assert!(t.frame(&s).is_empty(), "a held hoist draws as it did");
    s.set_hoist(on_top, vec![]);
    let d = t.frame(&s);
    assert!(covered(&d), "ending the hoist redraws it: {d:?}");
}

#[test]
fn an_opaque_node_hides_damage_beneath_it() {
    let mut s = TestScene::new();
    let root = s.root();
    let under = s.rect_create(root, (10, 10), HALF);
    let cover = s.rect_create(root, (50, 50), RED);
    let _ = cover;
    let mut t = Tracker::new(100, 100, 1.0);
    t.frame(&s);
    s.rect_set_color(under, [0.0, 0.5, 0.0, 0.5]);
    assert!(
        t.frame(&s).is_empty(),
        "an opaque rect above covers the change"
    );
}
