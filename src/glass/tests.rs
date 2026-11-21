use reprisekit::classic::frame::{Chrome, Edges};
use reprisekit::glass::CORNER;

use super::*;

#[test]
fn a_square_topped_shape_cuts_only_its_bottom_corners() {
    let pane = Pane {
        shapes: vec![PaneShape {
            rect: Rect::new(0, 0, 40, 11),
            corner: &[5, 3, 2, 1, 1, 1],
            square_bottom: false,
            square_top: true,
        }],
        holes: vec![],
    };
    let rows = drawn_shapes(&pane, Material::Frosted, 1.0);
    assert_eq!(rows.first().map(|r| r.0.x), Some(0), "the top row is whole");
    assert_eq!(rows.last().map(|r| r.0.x), Some(5), "the bottom row is cut");
    let round = drawn_shapes(&pane, Material::Molten, 1.0);
    assert_eq!(round[0].1, [0.0, 0.0, 7.0, 7.0]);
}

fn fast(on: bool) -> Settings {
    Settings {
        fast_drag: on,
        ..Settings::default()
    }
}

fn drag() -> Interaction {
    Interaction::Move {
        window: crate::window::WindowId::next(),
        dx: 0.0,
        dy: 0.0,
        origin: None,
        was_snapped: None,
        start: crate::input::MoveStart::default(),
        press: (0.0, 0.0),
        snap: None,
        armed: false,
        bar: false,
    }
}

#[test]
fn a_drag_blurs_coarse_with_fast_drag_and_full_without() {
    assert_eq!(quality(&drag(), &fast(true)), Quality::Coarse);
    assert_eq!(quality(&drag(), &fast(false)), Quality::Full);
}

#[test]
fn a_resize_blurs_coarse_too() {
    let resize = Interaction::Resize {
        window: crate::window::WindowId::next(),
        edges: Edges::RIGHT,
        start: Rect::new(0, 0, 100, 100),
        cx: 0.0,
        cy: 0.0,
        keyboard: None,
    };
    assert_eq!(quality(&resize, &fast(true)), Quality::Coarse);
}

#[test]
fn once_the_drag_ends_the_blur_is_full() {
    assert_eq!(quality(&Interaction::Idle, &fast(true)), Quality::Full);
}

#[test]
fn a_flyout_pane_lies_under_the_border_and_leaves_the_panel_clear() {
    let pane = flyout_pane((112, 40), 8, &[3, 1, 1]);
    assert_eq!(pane.shapes.len(), 1);
    assert_eq!(pane.shapes[0].rect, Rect::new(0, 0, 112, 40));
    assert_eq!(pane.shapes[0].corner, &[3, 1, 1]);
    assert_eq!(pane.holes, vec![Rect::new(8, 8, 96, 24)]);
}

#[test]
fn a_frame_pane_is_the_whole_frame_round_cornered_around_its_client() {
    let m = reprisekit::theme::Theme::default().metrics;
    let g = FrameGeometry::layout(&m, 200, 100, false, Chrome::WINDOW);
    let p = frame_pane(&g, CORNER, false);
    assert_eq!(
        p.shapes,
        vec![PaneShape {
            rect: Rect::new(0, 0, g.total.0, g.total.1),
            corner: CORNER,
            square_bottom: false,
            square_top: false,
        }]
    );
    assert_eq!(p.holes, vec![g.client]);
    assert_eq!(p.extent(), Size::from(g.total));
}

#[test]
fn frosted_glass_steps_its_corners_as_the_art_and_molten_rounds_them() {
    let pane = Pane {
        shapes: vec![PaneShape {
            rect: Rect::new(10, 20, 40, 30),
            corner: &[3, 1],
            square_bottom: false,
            square_top: false,
        }],
        holes: Vec::new(),
    };
    assert_eq!(
        drawn_shapes(&pane, Material::Frosted, 1.0),
        vec![
            (Rect::new(13, 20, 34, 1), [0.0; 4]),
            (Rect::new(11, 21, 38, 1), [0.0; 4]),
            (Rect::new(10, 22, 40, 26), [0.0; 4]),
            (Rect::new(11, 48, 38, 1), [0.0; 4]),
            (Rect::new(13, 49, 34, 1), [0.0; 4]),
        ]
    );
    assert_eq!(
        drawn_shapes(&pane, Material::Molten, 1.0),
        vec![(Rect::new(10, 20, 40, 30), [3.0; 4])]
    );
}

#[test]
fn a_frames_glass_rounds_its_corners_above_scale_1_as_its_chrome_does() {
    let m = reprisekit::theme::Theme::default().metrics;
    let g = FrameGeometry::layout(&m, 200, 100, false, Chrome::WINDOW);
    let frame = frame_pane(&g, CORNER, false);
    let whole = vec![(Rect::new(0, 0, g.total.0, g.total.1), [7.0; 4])];
    for scale in [1.25, 1.5, 2.0, 3.0] {
        assert_eq!(
            drawn_shapes(&frame, Material::Frosted, scale),
            whole,
            "Frosted at {scale}: whole, round"
        );
        assert_eq!(drawn_shapes(&frame, Material::Smoked, scale), whole);
    }
    assert_eq!(
        drawn_shapes(&frame, Material::Frosted, 1.0).len(),
        9,
        "stepped at scale 1: 5, 3, 2 and 1 px in at each end, the middle whole"
    );
}

#[test]
fn a_square_bottomed_shape_cuts_its_top_corners_only() {
    let pane = Pane {
        shapes: vec![PaneShape {
            rect: Rect::new(10, 20, 40, 30),
            corner: &[3, 1],
            square_bottom: true,
            square_top: false,
        }],
        holes: Vec::new(),
    };
    assert_eq!(
        drawn_shapes(&pane, Material::Frosted, 1.0),
        vec![
            (Rect::new(13, 20, 34, 1), [0.0; 4]),
            (Rect::new(11, 21, 38, 1), [0.0; 4]),
            (Rect::new(10, 22, 40, 28), [0.0; 4]),
        ]
    );
    assert_eq!(
        drawn_shapes(&pane, Material::Molten, 1.0),
        vec![(Rect::new(10, 20, 40, 30), [3.0, 3.0, 0.0, 0.0])]
    );
}

#[test]
fn a_shape_square_at_both_ends_is_one_rect() {
    let pane = Pane {
        shapes: vec![PaneShape {
            rect: Rect::new(10, 20, 40, 30),
            corner: &[3, 1],
            square_bottom: true,
            square_top: true,
        }],
        holes: Vec::new(),
    };
    let whole = vec![(Rect::new(10, 20, 40, 30), [0.0; 4])];
    assert_eq!(drawn_shapes(&pane, Material::Frosted, 1.0), whole);
    assert_eq!(drawn_shapes(&pane, Material::Molten, 1.0), whole);
}

#[test]
fn a_maximized_frame_pane_is_square() {
    let m = reprisekit::theme::Theme::default().metrics;
    let g = FrameGeometry::layout(&m, 200, 100, true, Chrome::WINDOW);
    assert!(frame_pane(&g, CORNER, true).shapes[0].corner.is_empty());
}

#[test]
fn physical_rects_are_scaled_from_the_node_and_meet_without_gaps() {
    let at = Point::from((10.0, 20.0));
    assert_eq!(
        physical(Rect::new(3, 4, 5, 6), at, Scale::from(1.0)),
        Rectangle::new((13, 24).into(), (5, 6).into())
    );
    assert_eq!(
        physical(Rect::new(3, 4, 5, 6), at, Scale::from(2.0)),
        Rectangle::new((16, 28).into(), (10, 12).into())
    );
    let s = Scale::from(1.25);
    let a = physical(Rect::new(0, 0, 3, 3), at, s);
    let b = physical(Rect::new(3, 0, 3, 3), at, s);
    assert_eq!(a.loc.x + a.size.w, b.loc.x, "neighbours meet at 1.25");
}

fn solid(x: i32, y: i32, w: i32, h: i32, alpha: f32) -> SolidElem {
    SolidElem::new(
        Id::new(),
        Rectangle::new((x, y).into(), (w, h).into()),
        0usize,
        [0.5, 0.5, 0.5, alpha],
        smithay::backend::renderer::element::Kind::Unspecified,
    )
}

type SolidElem = smithay::backend::renderer::element::solid::SolidColorRenderElement;

#[test]
fn glass_under_an_opaque_cover_does_not_show() {
    let s = Scale::from(1.0);
    let glass = solid(10, 10, 100, 40, 0.5);
    let made = [glass.id().clone()];
    let cover = solid(0, 0, 200, 200, 1.0);
    assert!(!shows_glass(&made, &[cover, glass], s));
}

#[test]
fn glass_shows_past_a_cover_that_misses_part_of_it() {
    let s = Scale::from(1.0);
    let glass = solid(10, 10, 100, 40, 0.5);
    let made = [glass.id().clone()];
    let part = solid(0, 0, 100, 200, 1.0);
    let see_through = solid(0, 0, 200, 200, 0.9);
    assert!(shows_glass(&made, &[part, glass.clone()], s));
    assert!(
        shows_glass(&made, &[see_through, glass.clone()], s),
        "a translucent cover hides nothing"
    );
    let below = solid(0, 0, 200, 200, 1.0);
    assert!(shows_glass(&made, &[glass, below], s), "a cover beneath");
}

#[test]
fn a_frames_glass_reads_its_ring_and_not_its_client() {
    let m = reprisekit::theme::Theme::default().metrics;
    let g = FrameGeometry::layout(&m, 400, 300, false, Chrome::WINDOW);
    let pass = Pass {
        settings: Settings::default(),
        ..Pass::default()
    };
    let at = Point::from((100.0, 50.0));
    let reads = Reads::of(
        &frame_pane(&g, CORNER, false),
        at,
        (1000, 800).into(),
        1.0,
        &pass,
    );
    let has = |rects: &[Rectangle<i32, Physical>], x: i32, y: i32| {
        rects.iter().any(|r| r.contains((x, y)))
    };
    let c = g.client;
    let (cx, cy) = (100 + c.x + c.width / 2, 50 + c.y + c.height / 2);
    assert!(!has(&reads.needed, cx, cy), "the client's middle");
    assert!(
        !has(&reads.reach, cx, cy),
        "a change there shows in no glass"
    );
    assert!(has(&reads.needed, 100 + g.total.0 / 2, 52), "the caption");
}

#[test]
fn a_frames_glass_says_it_reads_beneath_its_ring_as_far_as_its_feed_and_not_its_clients_middle() {
    let m = reprisekit::theme::Theme::default().metrics;
    let g = FrameGeometry::layout(&m, 400, 300, false, Chrome::WINDOW);
    let pane = frame_pane(&g, CORNER, false);
    let c = g.client;
    let middle = Rectangle::new(
        (c.x + c.width / 2, c.y + c.height / 2).into(),
        (1, 1).into(),
    );
    for material in [Material::Frosted, Material::Smoked, Material::Molten] {
        for (quality, scale) in [
            (Quality::Full, 1.0),
            (Quality::Coarse, 1.0),
            (Quality::Full, 1.5),
            (Quality::Full, 2.0),
        ] {
            let pass = Pass {
                settings: Settings {
                    material,
                    ..Settings::default()
                },
                quality,
                output: (1600, 1200).into(),
                ..Pass::default()
            };
            let said = reads_beneath(&pane, &pass, scale);
            let at = Point::from((60.0 * scale, 30.0 * scale));
            let reads = Reads::of(&pane, at, pass.output, scale, &pass);
            let s = &pass.settings;
            let r = reprisefx::blur::reach(s.blur, material, quality, scale);
            for n in &reads.needed {
                let read = Rectangle::new(
                    (n.loc.x - r, n.loc.y - r).into(),
                    (n.size.w + 2 * r, n.size.h + 2 * r).into(),
                );
                let read = layout(read, (-60, -30).into(), scale);
                let far = read.loc + read.size.to_point() - Point::from((1, 1));
                for corner in [read.loc, far] {
                    let corner = Rectangle::new(corner, (1, 1).into());
                    assert!(
                        said.iter().any(|x| x.contains_rect(corner)),
                        "{material:?} {quality:?} x{scale}: {corner:?} of {read:?}"
                    );
                }
            }
            assert!(
                !said.iter().any(|r| r.overlaps(middle)),
                "{material:?} {quality:?} x{scale}: nothing under the client's middle"
            );
        }
    }
}

#[test]
fn a_frames_glass_is_fed_from_past_what_its_blur_reads() {
    let m = reprisekit::theme::Theme::default().metrics;
    let g = FrameGeometry::layout(&m, 400, 300, false, Chrome::WINDOW);
    let pane = frame_pane(&g, CORNER, false);
    for material in [Material::Frosted, Material::Smoked, Material::Molten] {
        for quality in [Quality::Full, Quality::Coarse] {
            let pass = Pass {
                settings: Settings {
                    material,
                    ..Settings::default()
                },
                quality,
                ..Pass::default()
            };
            let reads = Reads::of(&pane, (100.0, 50.0).into(), (1000, 800).into(), 1.0, &pass);
            let feed = reads.feed.unwrap();
            let s = &pass.settings;
            let r = reprisefx::blur::reach(s.blur, material, quality, 1.0);
            for n in &reads.needed {
                let read = Rectangle::new(
                    (n.loc.x - r, n.loc.y - r).into(),
                    (n.size.w + 2 * r, n.size.h + 2 * r).into(),
                )
                .intersection(Rectangle::from_size((1000, 800).into()))
                .unwrap();
                assert!(
                    feed.contains_rect(read),
                    "{material:?} {quality:?}: {read:?}"
                );
            }
        }
    }
}

fn test_output() -> Output {
    let o = Output::new(
        "test".into(),
        smithay::output::PhysicalProperties {
            size: (0, 0).into(),
            subpixel: smithay::output::Subpixel::Unknown,
            make: "Reprise".into(),
            model: "test".into(),
            serial_number: String::new(),
        },
    );
    let mode = smithay::output::Mode {
        size: (200, 120).into(),
        refresh: 60_000,
    };
    o.change_current_state(
        Some(mode),
        Some(Transform::Normal),
        Some(smithay::output::Scale::Integer(1)),
        Some((0, 0).into()),
    );
    o
}

fn red_over_a_caption(blur_frames: bool) -> Option<u32> {
    let mut renderer = match reprisefx::headless::renderer() {
        Ok(r) => r,
        Err(e) => {
            reprisekit::pins::skipped(&e);
            return None;
        }
    };
    let ctx = GlassCtx::new();
    ctx.set_renderer(&mut renderer);
    let mut scene = DeskScene::new();
    let root = scene.root();
    let desktop = scene.tree_create(root);
    scene.rect_create(desktop, (200, 120), [1.0; 4]);
    let windows = scene.tree_create(root);
    let lower = scene.tree_create(windows);
    let chrome = scene.tree_create(lower);
    leave_out(&mut scene, chrome);
    scene.rect_create(chrome, (200, 40), [0.0, 0.0, 0.0, 1.0]);
    let upper = scene.tree_create(windows);
    scene.set_position(upper, 40, 16);
    let chrome = scene.tree_create(upper);
    leave_out(&mut scene, chrome);
    let mut glass = Backdrop::new(&ctx);
    glass.set(&mut scene, &ctx, chrome, Some(sheet_pane((120, 80), &[])));
    let output = test_output();
    let settings = Settings {
        blur_frames,
        streaks: false,
        ..Settings::default()
    };
    let clear = Color32F::from([0.0, 0.0, 0.0, 1.0]);
    ctx.begin(&output, settings, Quality::Full, Hold::No);
    ctx.update_backdrop(&mut renderer, &scene, &output, clear, true);
    let elements = scene.render_elements_for(&mut renderer, &output);
    let mut target = crate::offscreen::Target::new(&mut renderer, (200, 120).into(), 1.0).unwrap();
    target.render(&mut renderer, &elements).unwrap();
    let canvas = target.read(&mut renderer).unwrap();
    Some((canvas.pixels[24 * 200 + 100] >> 16) & 0xFF)
}

#[test]
fn glass_over_another_windows_caption_shows_it_blurred_unless_blur_frames_is_off() {
    let Some(on) = red_over_a_caption(true) else {
        return;
    };
    assert!(on < 80, "the black caption shows through the glass: {on}");
    let off = red_over_a_caption(false).unwrap();
    assert!(
        off > 230,
        "off, glass shows the white under the caption: {off}"
    );
}

#[test]
fn closed_glass_leaves_the_placed_list_with_blur_frames_off() {
    let mut renderer = match reprisefx::headless::renderer() {
        Ok(r) => r,
        Err(e) => {
            reprisekit::pins::skipped(&e);
            return;
        }
    };
    let ctx = GlassCtx::new();
    ctx.set_renderer(&mut renderer);
    let mut scene = DeskScene::new();
    let root = scene.root();
    for _ in 0..100 {
        let chrome = scene.tree_create(root);
        leave_out(&mut scene, chrome);
        let mut glass = Backdrop::new(&ctx);
        glass.set(&mut scene, &ctx, chrome, Some(sheet_pane((40, 20), &[])));
        scene.destroy(chrome);
    }
    let output = test_output();
    let settings = Settings {
        blur_frames: false,
        ..Settings::default()
    };
    ctx.begin(&output, settings, Quality::Full, Hold::No);
    let clear = Color32F::from([0.0, 0.0, 0.0, 1.0]);
    ctx.update_backdrop(&mut renderer, &scene, &output, clear, true);
    assert_eq!(ctx.nodes.borrow().len(), 0);
}

/// A wallpaper-like node that counts the walks asking for its elements.
struct Counted(Rc<Cell<u32>>);

impl CustomNode<GlesRenderer> for Counted {
    fn size(&self) -> Size<i32, Logical> {
        (320, 200).into()
    }
    fn in_backdrop(&self) -> bool {
        true
    }
    fn elements(
        &self,
        _renderer: &mut GlesRenderer,
        _location: Point<f64, Physical>,
        _scale: Scale<f64>,
        _alpha: f32,
    ) -> Vec<CustomElement<GlesRenderer>> {
        self.0.set(self.0.get() + 1);
        Vec::new()
    }
}

#[test]
fn the_outputs_backdrop_walks_the_scene_once_for_pieces_apart() {
    let Ok(mut renderer) = reprisefx::headless::renderer() else {
        reprisekit::pins::skipped("no renderer");
        return;
    };
    let ctx = GlassCtx::new();
    ctx.set_renderer(&mut renderer);
    let mut scene = DeskScene::new();
    let root = scene.root();
    let walks = Rc::new(Cell::new(0));
    scene.custom_create(root, Box::new(Counted(walks.clone())));
    let windows = scene.tree_create(root);
    let _left = glass_window(&mut scene, &ctx, windows, (0, 0), (40, 30), None);
    let _right = glass_window(&mut scene, &ctx, windows, (260, 160), (40, 30), None);
    let output = test_output_of(320, 200);
    ctx.begin(&output, Settings::default(), Quality::Full, Hold::No);
    let (area, seen) = (Rectangle::from_size((320, 200).into()), (320, 200).into());
    let reads = ctx.reads_on(&scene, area, seen, 1.0, &[]);
    let cuts = pieces(reads.iter().filter_map(|r| r.feed).map(|f| shrunk(f, 2)));
    assert_eq!(cuts.len(), 2, "the two glasses read apart: {cuts:?}");
    let clear = Color32F::from([0.0, 0.0, 0.0, 1.0]);
    ctx.update_backdrop(&mut renderer, &scene, &output, clear, true);
    assert_eq!(walks.get(), 1);
}

fn test_output_of(w: i32, h: i32) -> Output {
    let o = test_output();
    let mode = smithay::output::Mode {
        size: (w, h).into(),
        refresh: 60_000,
    };
    o.change_current_state(Some(mode), None, None, None);
    o
}

/// A window at `at` whose left-out chrome holds a `size` sheet of glass, over a black rect of
/// `under` when given (glass on screen covers it).
fn glass_window(
    scene: &mut DeskScene,
    ctx: &Rc<GlassCtx>,
    windows: TreeId,
    at: (i32, i32),
    size: (i32, i32),
    under: Option<(i32, i32)>,
) -> (TreeId, Backdrop) {
    let window = scene.tree_create(windows);
    scene.set_position(window, at.0, at.1);
    let chrome = scene.tree_create(window);
    leave_out(scene, chrome);
    if let Some(under) = under {
        scene.rect_create(chrome, under, [0.0, 0.0, 0.0, 1.0]);
    }
    let mut glass = Backdrop::new(ctx);
    glass.set(scene, ctx, chrome, Some(sheet_pane(size, &[])));
    (window, glass)
}

/// The red channel of `points` on a 320x200 output after one frame of `scene`.
fn red_at(
    renderer: &mut GlesRenderer,
    ctx: &GlassCtx,
    scene: &DeskScene,
    material: Material,
    points: &[(i32, i32)],
) -> Vec<u32> {
    let output = test_output_of(320, 200);
    let settings = Settings {
        material,
        streaks: false,
        ..Settings::default()
    };
    let clear = Color32F::from([0.0, 0.0, 0.0, 1.0]);
    ctx.begin(&output, settings, Quality::Full, Hold::No);
    ctx.update_backdrop(renderer, scene, &output, clear, true);
    let elements = scene.render_elements_for(renderer, &output);
    let mut target = crate::offscreen::Target::new(renderer, (320, 200).into(), 1.0).unwrap();
    target.render(renderer, &elements).unwrap();
    let canvas = target.read(renderer).unwrap();
    points
        .iter()
        .map(|&(x, y)| (canvas.pixels[(y * 320 + x) as usize] >> 16) & 0xFF)
        .collect()
}

#[test]
fn glass_over_other_glass_shows_that_glass_and_not_what_it_covers() {
    for material in [Material::Frosted, Material::Smoked, Material::Molten] {
        let Ok(mut renderer) = reprisefx::headless::renderer() else {
            reprisekit::pins::skipped("no renderer");
            return;
        };
        let ctx = GlassCtx::new();
        ctx.set_renderer(&mut renderer);
        let mut scene = DeskScene::new();
        let root = scene.root();
        let desktop = scene.tree_create(root);
        scene.rect_create(desktop, (320, 200), [1.0; 4]);
        let windows = scene.tree_create(root);
        let lower = Some((200, 120));
        let _lower = glass_window(&mut scene, &ctx, windows, (20, 20), (200, 120), lower);
        let _upper = glass_window(&mut scene, &ctx, windows, (120, 80), (180, 100), None);
        let [lower_alone, both] = red_at(
            &mut renderer,
            &ctx,
            &scene,
            material,
            &[(60, 50), (170, 110)],
        )[..] else {
            unreachable!()
        };
        // The lift under a frame's glass takes up to 20 levels of red off white.
        assert!(
            lower_alone > 200,
            "{material:?}: the lower glass shows the white desktop: {lower_alone}"
        );
        assert!(
            both > 200,
            "{material:?}: through the upper glass the lower glass shows, not the black under it: {both}"
        );
    }
}

/// How many pieces of glass draw from a backdrop of their own with a 320x30 bar at the bottom
/// of a 320x200 output and, above the bar, a window whose frame ends at the bar's top edge.
fn own_beside_a_bar(maximized: bool) -> usize {
    let ctx = GlassCtx::new();
    let mut scene = DeskScene::new();
    let root = scene.root();
    let desktop = scene.tree_create(root);
    scene.rect_create(desktop, (320, 200), [1.0; 4]);
    let windows = scene.tree_create(root);
    let m = reprisekit::theme::Theme::default().metrics;
    let border = if maximized { 0 } else { m.frame_width };
    let g = FrameGeometry::layout(&m, 320 - 2 * border, 120, maximized, Chrome::WINDOW);
    let window = scene.tree_create(windows);
    scene.set_position(window, 0, 170 - g.total.1);
    let chrome = scene.tree_create(window);
    leave_out(&mut scene, chrome);
    let mut frame = Backdrop::new(&ctx);
    frame.set(
        &mut scene,
        &ctx,
        chrome,
        Some(frame_pane(&g, CORNER, maximized)),
    );
    let (_bar, _glass) = glass_window(&mut scene, &ctx, windows, (0, 170), (320, 30), None);
    let (area, seen) = (Rectangle::from_size((320, 200).into()), (320, 200).into());
    ctx.pass.set(Pass {
        settings: Settings {
            blur_frames: true,
            ..Settings::default()
        },
        ..Pass::default()
    });
    ctx.drawn_beneath(&scene, area, seen, 1.0).len()
}

#[test]
fn a_bar_beside_a_maximized_window_draws_from_the_outputs_backdrop() {
    assert_eq!(
        own_beside_a_bar(true),
        0,
        "the bar's blur reaches only the client, where the window's glass draws nothing"
    );
    assert_eq!(
        own_beside_a_bar(false),
        2,
        "the bar's blur reaches the window's bottom border, whose glass lies under the bar"
    );
}

#[test]
fn the_outputs_blur_reads_only_for_glass_drawn_from_it() {
    let ctx = GlassCtx::new();
    let mut scene = DeskScene::new();
    let root = scene.root();
    let desktop = scene.tree_create(root);
    scene.rect_create(desktop, (320, 200), [1.0; 4]);
    let windows = scene.tree_create(root);
    let _lower = glass_window(&mut scene, &ctx, windows, (20, 20), (120, 80), None);
    let (area, seen) = (Rectangle::from_size((320, 200).into()), (320, 200).into());
    ctx.pass.set(Pass {
        settings: Settings {
            blur_frames: true,
            ..Settings::default()
        },
        ..Pass::default()
    });
    let own = ctx.drawn_beneath(&scene, area, seen, 1.0);
    assert!(own.is_empty(), "glass alone draws from the output's blur");
    let alone = ctx.reads_on(&scene, area, seen, 1.0, &own);
    assert!(!alone.is_empty());
    let (upper, _upper) = glass_window(&mut scene, &ctx, windows, (100, 60), (120, 80), None);
    let own = ctx.drawn_beneath(&scene, area, seen, 1.0);
    assert_eq!(
        own.len(),
        2,
        "overlapping glass draws from backdrops of its own"
    );
    assert_eq!(ctx.reads_on(&scene, area, seen, 1.0, &own), Vec::new());
    scene.destroy(upper);
    let own = ctx.drawn_beneath(&scene, area, seen, 1.0);
    assert!(own.is_empty(), "alone again");
    assert_eq!(ctx.reads_on(&scene, area, seen, 1.0, &own), alone);
}

#[test]
fn glass_beneath_other_glass_blurs_a_backdrop_without_the_windows_above_it() {
    let Ok(mut renderer) = reprisefx::headless::renderer() else {
        reprisekit::pins::skipped("no renderer");
        return;
    };
    let ctx = GlassCtx::new();
    ctx.set_renderer(&mut renderer);
    let mut scene = DeskScene::new();
    let root = scene.root();
    let desktop = scene.tree_create(root);
    scene.rect_create(desktop, (320, 200), [1.0; 4]);
    let windows = scene.tree_create(root);
    let (_, lower) = glass_window(&mut scene, &ctx, windows, (20, 20), (120, 80), None);
    let (upper, _upper) = glass_window(&mut scene, &ctx, windows, (100, 60), (120, 80), None);
    let lower = Rc::as_ptr(&lower.glass);
    let own = |ctx: &GlassCtx| {
        ctx.beneath
            .borrow()
            .get(&("test".into(), lower))
            .is_some_and(|b| b.reuse.in_use())
    };
    red_at(&mut renderer, &ctx, &scene, Material::Frosted, &[]);
    assert!(own(&ctx), "under the upper glass");
    let (area, seen) = (Rectangle::from_size((320, 200).into()), (320, 200).into());
    let feed = ctx
        .drawn_beneath(&scene, area, seen, 1.0)
        .iter()
        .find(|n| Rc::as_ptr(&n.glass) == lower)
        .and_then(|n| n.reads.feed)
        .expect("the lower glass reads a feed");
    let size = |ctx: &GlassCtx| {
        use smithay::backend::renderer::Texture;
        let b = ctx.beneath.borrow();
        b[&("test".into(), lower)].backdrop.texture.size()
    };
    let texels = shrunk(feed, 2).size;
    assert_eq!(
        size(&ctx),
        (texels.w, texels.h).into(),
        "the private backdrop holds what feeds its glass"
    );
    scene.set_position(upper, 200, 120);
    red_at(&mut renderer, &ctx, &scene, Material::Frosted, &[]);
    assert!(
        !own(&ctx),
        "past the upper glass's reach the output's backdrop does"
    );
}

#[test]
fn glass_beneath_other_glass_draws_its_own_backdrop_into_it() {
    let Ok(mut renderer) = reprisefx::headless::renderer() else {
        reprisekit::pins::skipped("no renderer");
        return;
    };
    let ctx = GlassCtx::new();
    ctx.set_renderer(&mut renderer);
    let mut scene = DeskScene::new();
    let root = scene.root();
    let desktop = scene.tree_create(root);
    scene.rect_create(desktop, (320, 200), [1.0; 4]);
    let windows = scene.tree_create(root);
    let bottom = scene.tree_create(windows);
    let caption = scene.tree_create(bottom);
    leave_out(&mut scene, caption);
    scene.rect_create(caption, (320, 70), [0.0, 0.0, 0.0, 1.0]);
    let middle = scene.tree_create(windows);
    let top = scene.tree_create(windows);
    // The top glass is made first: its backdrop still waits for the middle glass's.
    let _top = glass_window(&mut scene, &ctx, top, (120, 20), (180, 100), None);
    let _middle = glass_window(&mut scene, &ctx, middle, (20, 10), (200, 120), None);
    let red = red_at(&mut renderer, &ctx, &scene, Material::Frosted, &[(170, 35)]);
    assert!(
        red[0] < 80,
        "the middle glass shows the black caption beneath it, through the top glass too: {}",
        red[0]
    );
}

#[test]
fn a_change_away_from_glass_is_blurred_once_glass_reads_it() {
    let Ok(mut renderer) = reprisefx::headless::renderer() else {
        reprisekit::pins::skipped("no renderer");
        return;
    };
    let scene_with = |ctx: &Rc<GlassCtx>| {
        let mut scene = DeskScene::new();
        let root = scene.root();
        let desktop = scene.tree_create(root);
        scene.rect_create(desktop, (320, 200), [1.0; 4]);
        let spot = scene.tree_create(desktop);
        scene.set_position(spot, 240, 20);
        scene.rect_create(spot, (60, 40), [0.0, 0.0, 0.0, 1.0]);
        scene.set_enabled(spot, false);
        let windows = scene.tree_create(root);
        let (window, glass) = glass_window(&mut scene, ctx, windows, (10, 120), (100, 60), None);
        (scene, spot, window, glass)
    };
    let points: Vec<(i32, i32)> = (0..10).map(|i| (226 + i * 9, 30)).collect();
    let ctx = GlassCtx::new();
    ctx.set_renderer(&mut renderer);
    let (mut scene, spot, window, _glass) = scene_with(&ctx);
    red_at(&mut renderer, &ctx, &scene, Material::Frosted, &[]);
    scene.set_enabled(spot, true);
    red_at(&mut renderer, &ctx, &scene, Material::Frosted, &[]);
    scene.set_position(window, 210, 0);
    let moved = red_at(&mut renderer, &ctx, &scene, Material::Frosted, &points);
    let fresh_ctx = GlassCtx::new();
    fresh_ctx.set_renderer(&mut renderer);
    let (mut fresh, spot, window, _glass) = scene_with(&fresh_ctx);
    fresh.set_enabled(spot, true);
    fresh.set_position(window, 210, 0);
    let fresh = red_at(
        &mut renderer,
        &fresh_ctx,
        &fresh,
        Material::Frosted,
        &points,
    );
    assert!(
        moved.iter().any(|&r| r < 200),
        "the spot shows blurred: {moved:?}"
    );
    assert_eq!(
        moved, fresh,
        "as blurred as a first frame of the same scene"
    );
}

/// The red channel of the output backdrop's texel at output pixel `at`.
fn backdrop_red(renderer: &mut GlesRenderer, ctx: &GlassCtx, at: (i32, i32)) -> u32 {
    use smithay::backend::renderer::{ExportMem, Texture};
    let mut t = ctx.backdrops.borrow()["test"].texture.clone();
    let size = t.size();
    let fb = renderer.bind(&mut t).unwrap();
    let mapping = renderer
        .copy_framebuffer(&fb, Rectangle::from_size(size), Fourcc::Abgr8888)
        .unwrap();
    drop(fb);
    let bytes = renderer.map_texture(&mapping).unwrap();
    let (x, y) = (at.0 / 2, at.1 / 2);
    u32::from(bytes[((y * size.w + x) * 4) as usize])
}

#[test]
fn overlapping_feeds_join_into_one_piece_and_apart_ones_stay_apart() {
    let r = |x, y, w, h| Rectangle::<i32, Physical>::new((x, y).into(), (w, h).into());
    assert_eq!(
        pieces([r(0, 90, 160, 10), r(10, 10, 30, 20), r(30, 25, 30, 20)].into_iter()),
        vec![r(10, 10, 50, 35), r(0, 90, 160, 10)],
        "the two upper feeds overlap; the bar's stays apart, last as the lowest"
    );
    assert_eq!(
        pieces([r(0, 0, 10, 10), r(20, 0, 10, 10), r(5, 5, 20, 2)].into_iter()),
        vec![r(0, 0, 30, 10)],
        "a feed meeting two pieces joins all three"
    );
    assert_eq!(pieces(std::iter::empty()), Vec::new());
}

#[test]
fn glass_far_apart_leaves_the_backdrop_between_undrawn() {
    let Ok(mut renderer) = reprisefx::headless::renderer() else {
        reprisekit::pins::skipped("no renderer");
        return;
    };
    let ctx = GlassCtx::new();
    ctx.set_renderer(&mut renderer);
    let mut scene = DeskScene::new();
    let root = scene.root();
    let desktop = scene.tree_create(root);
    scene.rect_create(desktop, (320, 200), [1.0; 4]);
    let windows = scene.tree_create(root);
    let (window, _glass) = glass_window(&mut scene, &ctx, windows, (0, 0), (320, 20), None);
    let _bar = glass_window(&mut scene, &ctx, windows, (0, 180), (320, 20), None);
    red_at(&mut renderer, &ctx, &scene, Material::Frosted, &[]);
    assert!(
        backdrop_red(&mut renderer, &ctx, (160, 10)) > 230,
        "under the top glass"
    );
    assert!(
        backdrop_red(&mut renderer, &ctx, (160, 190)) > 230,
        "under the bottom glass"
    );
    assert!(
        backdrop_red(&mut renderer, &ctx, (160, 100)) < 20,
        "between them the backdrop is left clear"
    );
    scene.set_position(window, 0, 90);
    red_at(&mut renderer, &ctx, &scene, Material::Frosted, &[]);
    assert!(
        backdrop_red(&mut renderer, &ctx, (160, 100)) > 230,
        "glass moved there draws it"
    );
}

#[test]
fn the_outputs_backdrop_draws_only_what_its_glass_reads() {
    let Ok(mut renderer) = reprisefx::headless::renderer() else {
        reprisekit::pins::skipped("no renderer");
        return;
    };
    let ctx = GlassCtx::new();
    ctx.set_renderer(&mut renderer);
    let mut scene = DeskScene::new();
    let root = scene.root();
    let desktop = scene.tree_create(root);
    scene.rect_create(desktop, (320, 200), [1.0; 4]);
    let windows = scene.tree_create(root);
    let (window, _glass) = glass_window(&mut scene, &ctx, windows, (10, 10), (80, 50), None);
    red_at(&mut renderer, &ctx, &scene, Material::Frosted, &[]);
    assert!(
        backdrop_red(&mut renderer, &ctx, (50, 30)) > 230,
        "under the glass"
    );
    assert!(
        backdrop_red(&mut renderer, &ctx, (300, 180)) < 20,
        "far from the glass the backdrop is left clear"
    );
    scene.set_position(window, 220, 130);
    red_at(&mut renderer, &ctx, &scene, Material::Frosted, &[]);
    assert!(
        backdrop_red(&mut renderer, &ctx, (300, 180)) > 230,
        "glass moved there draws it"
    );
}

#[test]
fn a_held_frame_leaves_the_backdrop_undrawn_until_the_next_blur() {
    let Ok(mut renderer) = reprisefx::headless::renderer() else {
        reprisekit::pins::skipped("no renderer");
        return;
    };
    let ctx = GlassCtx::new();
    ctx.set_renderer(&mut renderer);
    let mut scene = DeskScene::new();
    let root = scene.root();
    let desktop = scene.tree_create(root);
    scene.rect_create(desktop, (320, 200), [1.0; 4]);
    let spot = scene.tree_create(desktop);
    scene.set_position(spot, 40, 40);
    scene.rect_create(spot, (60, 40), [0.0, 0.0, 0.0, 1.0]);
    scene.set_enabled(spot, false);
    let windows = scene.tree_create(root);
    let _glass = glass_window(&mut scene, &ctx, windows, (20, 20), (120, 80), None);
    let output = test_output_of(320, 200);
    let settings = Settings {
        streaks: false,
        ..Settings::default()
    };
    let clear = Color32F::from([0.0, 0.0, 0.0, 1.0]);
    let frame = |renderer: &mut GlesRenderer, scene: &DeskScene, hold: Hold| {
        ctx.begin(&output, settings, Quality::Full, hold);
        ctx.update_screen_backdrop(renderer, scene, &output, clear, true);
    };
    frame(&mut renderer, &scene, Hold::No);
    scene.set_enabled(spot, true);
    frame(&mut renderer, &scene, Hold::Keep(Under::Anywhere));
    assert!(
        backdrop_red(&mut renderer, &ctx, (70, 60)) > 230,
        "the held frame drew nothing into the backdrop"
    );
    assert!(ctx.any_stale(), "the skipped change waits for a blur");
    assert!(ctx.refresh_stale());
    frame(&mut renderer, &scene, Hold::No);
    assert!(
        backdrop_red(&mut renderer, &ctx, (70, 60)) < 20,
        "the next free frame draws the spot"
    );
    assert!(!ctx.any_stale());
}

#[test]
fn a_capture_after_a_held_frame_keeps_the_blur_the_screen_shows() {
    let Ok(mut renderer) = reprisefx::headless::renderer() else {
        reprisekit::pins::skipped("no renderer");
        return;
    };
    let ctx = GlassCtx::new();
    ctx.set_renderer(&mut renderer);
    let mut scene = DeskScene::new();
    let root = scene.root();
    let desktop = scene.tree_create(root);
    scene.rect_create(desktop, (320, 200), [1.0; 4]);
    let spot = scene.tree_create(desktop);
    scene.set_position(spot, 40, 40);
    scene.rect_create(spot, (60, 40), [0.0, 0.0, 0.0, 1.0]);
    scene.set_enabled(spot, false);
    let windows = scene.tree_create(root);
    let _glass = glass_window(&mut scene, &ctx, windows, (20, 20), (120, 80), None);
    let output = test_output_of(320, 200);
    let settings = Settings {
        streaks: false,
        ..Settings::default()
    };
    let clear = Color32F::from([0.0, 0.0, 0.0, 1.0]);
    let frame = |renderer: &mut GlesRenderer, scene: &DeskScene, hold: Hold| {
        ctx.begin(&output, settings, Quality::Full, hold);
        ctx.update_screen_backdrop(renderer, scene, &output, clear, true);
    };
    frame(&mut renderer, &scene, Hold::No);
    assert_eq!(ctx.capture_hold(&output), Hold::No, "nothing held");
    scene.set_enabled(spot, true);
    frame(&mut renderer, &scene, Hold::Keep(Under::Anywhere));
    let capture = ctx.capture_hold(&output);
    assert!(capture.keeps(), "the screen held a change");
    frame(&mut renderer, &scene, capture);
    assert!(
        backdrop_red(&mut renderer, &ctx, (70, 60)) > 230,
        "the capture blurred nothing the screen held"
    );
    assert!(ctx.refresh_stale());
    assert_eq!(ctx.capture_hold(&output), Hold::No, "the blur is due");
    frame(&mut renderer, &scene, Hold::No);
    assert!(
        backdrop_red(&mut renderer, &ctx, (70, 60)) < 20,
        "the capture draws the spot"
    );
}

#[test]
fn a_frame_held_for_the_cursor_alone_leaves_nothing_to_blur_later() {
    let Ok(mut renderer) = reprisefx::headless::renderer() else {
        reprisekit::pins::skipped("no renderer");
        return;
    };
    let ctx = GlassCtx::new();
    ctx.set_renderer(&mut renderer);
    let mut scene = DeskScene::new();
    let root = scene.root();
    let desktop = scene.tree_create(root);
    scene.rect_create(desktop, (320, 200), [1.0; 4]);
    let windows = scene.tree_create(root);
    let _glass = glass_window(&mut scene, &ctx, windows, (20, 20), (120, 80), None);
    let output = test_output_of(320, 200);
    let settings = Settings {
        streaks: false,
        ..Settings::default()
    };
    let clear = Color32F::from([0.0, 0.0, 0.0, 1.0]);
    for hold in [
        Hold::No,
        Hold::Keep(Under::Nothing),
        Hold::Keep(Under::Nothing),
    ] {
        ctx.begin(&output, settings, Quality::Full, hold);
        ctx.update_screen_backdrop(&mut renderer, &scene, &output, clear, true);
    }
    assert!(
        !ctx.any_stale(),
        "no reblur frame is owed for a change drawn over the glass"
    );
}

#[test]
fn a_frame_held_for_a_commit_away_from_glass_leaves_nothing_to_blur_later() {
    let Ok(mut renderer) = reprisefx::headless::renderer() else {
        reprisekit::pins::skipped("no renderer");
        return;
    };
    let ctx = GlassCtx::new();
    ctx.set_renderer(&mut renderer);
    let mut scene = DeskScene::new();
    let root = scene.root();
    let desktop = scene.tree_create(root);
    scene.rect_create(desktop, (320, 200), [1.0; 4]);
    let windows = scene.tree_create(root);
    let _glass = glass_window(&mut scene, &ctx, windows, (20, 20), (120, 80), None);
    let output = test_output_of(320, 200);
    let settings = Settings {
        streaks: false,
        ..Settings::default()
    };
    let clear = Color32F::from([0.0, 0.0, 0.0, 1.0]);
    let frame = |renderer: &mut GlesRenderer, hold: Hold| {
        ctx.begin(&output, settings, Quality::Full, hold);
        ctx.update_screen_backdrop(renderer, &scene, &output, clear, true);
    };
    frame(&mut renderer, Hold::No);
    let away = Drawn {
        r: Rectangle::new((250, 150).into(), (60, 40).into()),
        window: None,
    };
    frame(&mut renderer, Hold::Keep(Under::Within(away)));
    assert!(
        !ctx.any_stale(),
        "no reblur frame is owed for a commit outside every glass's reach"
    );
    let over = Drawn {
        r: Rectangle::new((60, 60).into(), (20, 20).into()),
        window: None,
    };
    frame(&mut renderer, Hold::Keep(Under::Within(over)));
    assert!(ctx.any_stale(), "a commit under the glass waits for a blur");
}

#[test]
fn a_frame_held_for_a_commit_in_a_glass_window_owes_a_blur_only_to_other_glass() {
    let Ok(mut renderer) = reprisefx::headless::renderer() else {
        reprisekit::pins::skipped("no renderer");
        return;
    };
    let ctx = GlassCtx::new();
    ctx.set_renderer(&mut renderer);
    let mut scene = DeskScene::new();
    let root = scene.root();
    let desktop = scene.tree_create(root);
    scene.rect_create(desktop, (320, 200), [1.0; 4]);
    let windows = scene.tree_create(root);
    let (video, _glass) = glass_window(&mut scene, &ctx, windows, (20, 20), (120, 80), None);
    let output = test_output_of(320, 200);
    let settings = Settings {
        streaks: false,
        ..Settings::default()
    };
    let clear = Color32F::from([0.0, 0.0, 0.0, 1.0]);
    let frame = |renderer: &mut GlesRenderer, scene: &DeskScene, hold: Hold| {
        ctx.begin(&output, settings, Quality::Full, hold);
        ctx.update_screen_backdrop(renderer, scene, &output, clear, true);
    };
    let inside = Drawn {
        r: Rectangle::new((40, 40).into(), (80, 40).into()),
        window: Some(video.into()),
    };
    frame(&mut renderer, &scene, Hold::No);
    frame(&mut renderer, &scene, Hold::Keep(Under::Within(inside)));
    assert!(
        !ctx.any_stale(),
        "the window's own glass shows its program at the next blur"
    );
    let (_above, _glass_above) =
        glass_window(&mut scene, &ctx, windows, (100, 30), (120, 80), None);
    frame(&mut renderer, &scene, Hold::No);
    frame(&mut renderer, &scene, Hold::Keep(Under::Within(inside)));
    assert!(
        ctx.any_stale(),
        "glass of another window over the commit waits for a blur"
    );
}

#[test]
fn a_frame_held_for_a_commit_in_its_own_glass_owes_that_glass_a_catch_up() {
    let Ok(mut renderer) = reprisefx::headless::renderer() else {
        reprisekit::pins::skipped("no renderer");
        return;
    };
    let ctx = GlassCtx::new();
    ctx.set_renderer(&mut renderer);
    let mut scene = DeskScene::new();
    let root = scene.root();
    let desktop = scene.tree_create(root);
    scene.rect_create(desktop, (320, 200), [1.0; 4]);
    let windows = scene.tree_create(root);
    let (video, _glass) = glass_window(&mut scene, &ctx, windows, (20, 20), (120, 80), None);
    let output = test_output_of(320, 200);
    let settings = Settings {
        streaks: false,
        ..Settings::default()
    };
    let clear = Color32F::from([0.0, 0.0, 0.0, 1.0]);
    let frame = |renderer: &mut GlesRenderer, hold: Hold| {
        ctx.begin(&output, settings, Quality::Full, hold);
        ctx.update_screen_backdrop(renderer, &scene, &output, clear, true);
    };
    let drawn = |x, y| Drawn {
        r: Rectangle::new((x, y).into(), (80, 40).into()),
        window: Some(video.into()),
    };
    frame(&mut renderer, Hold::No);
    frame(&mut renderer, Hold::Keep(Under::Nothing));
    assert!(!ctx.owes(&output), "the pointer alone owes nothing");
    frame(&mut renderer, Hold::Keep(Under::Within(drawn(200, 150))));
    assert!(!ctx.owes(&output), "pixels no glass reads owe nothing");
    frame(&mut renderer, Hold::Keep(Under::Within(drawn(40, 40))));
    assert!(ctx.owes(&output), "the window's own glass catches up");
    assert!(!ctx.any_stale(), "with no reblur frame at once");
    frame(&mut renderer, Hold::No);
    assert!(!ctx.owes(&output), "a frame that blurs settles it");
    frame(&mut renderer, Hold::Keep(Under::Within(drawn(40, 40))));
    assert!(ctx.take_owed(&output));
    assert!(!ctx.owes(&output), "the catch-up asked settles it");
}

#[test]
fn glass_missing_from_the_elements_counts_as_shown() {
    let s = Scale::from(1.0);
    let glass = solid(10, 10, 100, 40, 0.5);
    let made = [glass.id().clone()];
    assert!(shows_glass(&made, &[solid(0, 0, 200, 200, 1.0)], s));
    assert!(
        !shows_glass(&[], &[solid(0, 0, 200, 200, 1.0)], s),
        "no glass at all"
    );
}

#[test]
fn a_private_backdrop_outlives_its_use_for_a_while_without_being_drawn_from() {
    let start = Instant::now();
    let mut reuse = Reuse::default();
    assert!(!reuse.in_use(), "never drawn");
    reuse.drawn(start);
    assert!(reuse.in_use());
    assert!(reuse.kept(start + Duration::from_millis(16)));
    assert!(!reuse.in_use(), "unused this update");
    assert!(reuse.kept(start + KEEP_IDLE));
    assert!(
        !reuse.kept(start + KEEP_IDLE + Duration::from_millis(1)),
        "dropped {KEEP_IDLE:?} after its last use"
    );
    let later = start + 2 * KEEP_IDLE;
    reuse.drawn(later);
    assert!(
        reuse.in_use() && reuse.kept(later + KEEP_IDLE),
        "a use starts the time again"
    );
}

/// What `glass_over_glass_draws_each_frame_as_a_fresh_start_would` lays out: a lower window with a
/// caption and a sheet of glass, an upper window with glass, over a desktop with a stripe.
#[derive(Clone, Copy)]
struct Layout {
    stripe: [f32; 4],
    /// Over the bare clear colour right of the desktop.
    patch: (i32, i32),
    lower: (i32, i32),
    upper: (i32, i32),
    upper_size: (i32, i32),
    lower_on_top: bool,
    material: Material,
}

type Step = (&'static str, fn(&mut Layout));

struct Laid {
    scene: DeskScene,
    ctx: Rc<GlassCtx>,
    stripe: reprisescene::RectId,
    patch: reprisescene::RectId,
    lower: (TreeId, Backdrop, TreeId),
    upper: (TreeId, Backdrop, TreeId),
}

fn lay(l: &Layout, renderer: &mut GlesRenderer) -> Laid {
    let ctx = GlassCtx::new();
    ctx.set_renderer(renderer);
    ctx.clock.set(Some(Instant::now()));
    let mut scene = DeskScene::new();
    let root = scene.root();
    let desktop = scene.tree_create(root);
    scene.rect_create(desktop, (430, 200), [1.0; 4]);
    let stripe = scene.rect_create(desktop, (640, 30), l.stripe);
    scene.set_position(stripe, 0, 60);
    let patch = scene.rect_create(desktop, (90, 30), [0.9, 0.8, 0.1, 1.0]);
    scene.set_position(patch, l.patch.0, l.patch.1);
    let windows = scene.tree_create(root);
    let mut window = |at: (i32, i32), size: (i32, i32), caption: bool| {
        let window = scene.tree_create(windows);
        scene.set_position(window, at.0, at.1);
        let chrome = scene.tree_create(window);
        leave_out(&mut scene, chrome);
        if caption {
            scene.rect_create(chrome, (size.0, 20), [0.0, 0.0, 0.3, 1.0]);
        }
        let mut glass = Backdrop::new(&ctx);
        glass.set(&mut scene, &ctx, chrome, Some(sheet_pane(size, &[])));
        (window, glass, chrome)
    };
    let lower = window(l.lower, (150, 110), true);
    let upper = window(l.upper, l.upper_size, false);
    if l.lower_on_top {
        scene.raise_to_top(lower.0);
    }
    Laid {
        scene,
        ctx,
        stripe,
        patch,
        lower,
        upper,
    }
}

fn frame(renderer: &mut GlesRenderer, laid: &Laid, material: Material) -> Vec<u32> {
    let output = test_output_of(640, 200);
    let settings = Settings {
        material,
        ..Settings::default()
    };
    let clear = Color32F::from([0.0, 0.0, 0.0, 1.0]);
    laid.ctx.begin(&output, settings, Quality::Full, Hold::No);
    laid.ctx
        .update_backdrop(renderer, &laid.scene, &output, clear, true);
    let elements = laid.scene.render_elements_for(renderer, &output);
    let mut target = crate::offscreen::Target::new(renderer, (640, 200).into(), 1.0).unwrap();
    target.render(renderer, &elements).unwrap();
    target.read(renderer).unwrap().pixels
}

#[test]
fn glass_over_glass_draws_each_frame_as_a_fresh_start_would() {
    let Ok(mut renderer) = reprisefx::headless::renderer() else {
        reprisekit::pins::skipped("no renderer");
        return;
    };
    let mut l = Layout {
        stripe: [0.8, 0.1, 0.1, 1.0],
        patch: (520, 120),
        lower: (20, 20),
        upper: (100, 50),
        upper_size: (150, 110),
        lower_on_top: false,
        material: Material::Frosted,
    };
    let mut live = lay(&l, &mut renderer);
    let steps: [Step; 13] = [
        ("as laid", |_| {}),
        ("the upper dragged", |l| l.upper = (110, 56)),
        ("dragged on", |l| l.upper = (124, 64)),
        ("off the lower glass", |l| l.upper = (460, 60)),
        ("the stripe beneath changed while apart", |l| {
            l.stripe = [0.1, 0.7, 0.1, 1.0]
        }),
        ("back over it", |l| l.upper = (110, 56)),
        ("the lower raised", |l| l.lower_on_top = true),
        ("the upper resized", |l| l.upper_size = (120, 90)),
        ("Molten", |l| l.material = Material::Molten),
        ("the lower moved", |l| l.lower = (60, 30)),
        ("Frosted again", |l| l.material = Material::Frosted),
        ("a patch far from both moved", |l| l.patch = (540, 10)),
        ("both moved where the patch was", |l| {
            l.lower = (470, 100);
            l.upper = (490, 110);
        }),
    ];
    for (what, step) in steps {
        step(&mut l);
        let s = &mut live.scene;
        s.rect_set_color(live.stripe, l.stripe);
        s.set_position(live.patch, l.patch.0, l.patch.1);
        s.set_position(live.lower.0, l.lower.0, l.lower.1);
        s.set_position(live.upper.0, l.upper.0, l.upper.1);
        let top = if l.lower_on_top {
            live.lower.0
        } else {
            live.upper.0
        };
        s.raise_to_top(top);
        let pane = Some(sheet_pane(l.upper_size, &[]));
        live.upper.1.set(s, &live.ctx, live.upper.2, pane);
        let drawn = frame(&mut renderer, &live, l.material);
        let start = lay(&l, &mut renderer);
        let fresh = frame(&mut renderer, &start, l.material);
        if what.ends_with("apart") {
            let kept = live.ctx.beneath.borrow();
            assert!(
                kept.len() == 2 && kept.values().all(|b| !b.reuse.in_use()),
                "both private backdrops kept, unused"
            );
        }
        let differ = drawn.iter().zip(&fresh).filter(|(a, b)| a != b).count();
        assert_eq!(differ, 0, "{what}: pixels unlike a fresh start's");
    }
}

#[test]
fn an_output_that_stops_rendering_drops_the_private_backdrops_not_drawn_from() {
    let Ok(mut renderer) = reprisefx::headless::renderer() else {
        reprisekit::pins::skipped("no renderer");
        return;
    };
    let mut l = Layout {
        stripe: [0.8, 0.1, 0.1, 1.0],
        patch: (520, 120),
        lower: (20, 20),
        upper: (100, 50),
        upper_size: (150, 110),
        lower_on_top: false,
        material: Material::Frosted,
    };
    let mut live = lay(&l, &mut renderer);
    frame(&mut renderer, &live, l.material);
    let covered = live.ctx.beneath.borrow().len();
    assert!(
        covered > 0,
        "glass over glass draws from a private backdrop"
    );
    let output = test_output_of(640, 200);
    live.ctx.output_stopped(&output);
    assert_eq!(
        live.ctx.beneath.borrow().len(),
        covered,
        "backdrops drawn from in the last update stay"
    );
    l.upper = (460, 60);
    live.scene.set_position(live.upper.0, l.upper.0, l.upper.1);
    frame(&mut renderer, &live, l.material);
    assert_eq!(
        live.ctx.beneath.borrow().len(),
        covered,
        "kept unused while rendering"
    );
    live.ctx.output_stopped(&output);
    assert!(
        live.ctx.beneath.borrow().is_empty(),
        "dropped once its output stops"
    );
}

#[test]
fn a_removed_output_drops_its_backdrops() {
    let Ok(mut renderer) = reprisefx::headless::renderer() else {
        reprisekit::pins::skipped("no renderer");
        return;
    };
    let l = Layout {
        stripe: [0.8, 0.1, 0.1, 1.0],
        patch: (520, 120),
        lower: (20, 20),
        upper: (100, 50),
        upper_size: (150, 110),
        lower_on_top: false,
        material: Material::Frosted,
    };
    let live = lay(&l, &mut renderer);
    frame(&mut renderer, &live, l.material);
    assert!(live.ctx.backdrops.borrow().get("test").is_some());
    assert!(!live.ctx.beneath.borrow().is_empty());
    live.ctx.output_removed(&test_output_of(640, 200));
    assert!(live.ctx.backdrops.borrow().get("test").is_none());
    assert!(live.ctx.beneath.borrow().is_empty());
}

#[test]
fn glass_kept_while_hidden_resumes_its_private_backdrop_in_a_new_tree() {
    let Ok(mut renderer) = reprisefx::headless::renderer() else {
        reprisekit::pins::skipped("no renderer");
        return;
    };
    let l = Layout {
        stripe: [0.8, 0.1, 0.1, 1.0],
        patch: (520, 120),
        lower: (20, 20),
        upper: (100, 50),
        upper_size: (150, 110),
        lower_on_top: false,
        material: Material::Frosted,
    };
    let mut live = lay(&l, &mut renderer);
    let glass = Rc::as_ptr(&live.upper.1.glass);
    let texture = |live: &Laid| {
        let kept = live.ctx.beneath.borrow();
        kept.iter()
            .find(|((_, g), _)| *g == glass)
            .map(|(_, b)| b.backdrop.texture.tex_id())
    };
    let listed = |live: &Laid| {
        live.ctx
            .glasses
            .borrow()
            .iter()
            .filter(|g| g.as_ptr() == glass)
            .count()
    };
    frame(&mut renderer, &live, l.material);
    let first = texture(&live).expect("the upper glass draws from a private backdrop");
    assert_eq!(listed(&live), 1);

    let pane = Some(sheet_pane(l.upper_size, &[]));
    live.upper
        .1
        .set(&mut live.scene, &live.ctx, live.upper.2, None);
    live.scene.destroy(live.upper.0);
    frame(&mut renderer, &live, l.material);
    assert_eq!(listed(&live), 0, "hidden glass is not listed");
    assert_eq!(texture(&live), Some(first), "kept while hidden");

    let s = &mut live.scene;
    let window = s.tree_create(s.root());
    s.set_position(window, l.upper.0, l.upper.1);
    let chrome = s.tree_create(window);
    leave_out(s, chrome);
    live.upper.1.set(s, &live.ctx, chrome, pane);
    let drawn = frame(&mut renderer, &live, l.material);
    assert_eq!(texture(&live), Some(first), "resumed, not made afresh");
    assert_eq!(listed(&live), 1, "listed once shown again");
    let start = lay(&l, &mut renderer);
    let fresh = frame(&mut renderer, &start, l.material);
    let differ = drawn.iter().zip(&fresh).filter(|(a, b)| a != b).count();
    assert_eq!(differ, 0, "pixels unlike a fresh start's");
}

#[test]
fn what_lies_in_the_overlay_layer_does_not_blur_into_glass_beneath_it() {
    let Ok(mut renderer) = reprisefx::headless::renderer() else {
        reprisekit::pins::skipped("no renderer");
        return;
    };
    let ctx = GlassCtx::new();
    ctx.set_renderer(&mut renderer);
    let mut scene = DeskScene::new();
    let layers = crate::compositor::Layers::new(&mut scene);
    scene.rect_create(layers.background, (320, 200), [1.0; 4]);
    let bar = scene.tree_create(layers.taskbar);
    scene.set_position(bar, 20, 20);
    leave_out(&mut scene, bar);
    let mut glass = Backdrop::new(&ctx);
    glass.set(&mut scene, &ctx, bar, Some(sheet_pane((280, 120), &[])));
    let ghost = scene.rect_create(layers.overlay, (40, 40), [0.0, 0.0, 0.0, 1.0]);
    scene.set_position(ghost, 140, 60);
    let red = red_at(
        &mut renderer,
        &ctx,
        &scene,
        Material::Frosted,
        &[(160, 80), (134, 80), (40, 80)],
    );
    assert_eq!(red[0], 0, "the drag image shows");
    assert_eq!(
        red[1], red[2],
        "no halo: the glass beside it reads as the glass far from it"
    );
}

/// The red channel just left of a black|white edge under a sheet of glass in a tree at
/// `opacity`, over another window's glass when `over_glass`, and how much glass the frame drew.
fn faded_edge(material: Material, opacity: f32, over_glass: bool) -> Option<(u32, usize)> {
    let mut renderer = reprisefx::headless::renderer().ok()?;
    let ctx = GlassCtx::new();
    ctx.set_renderer(&mut renderer);
    let mut scene = DeskScene::new();
    let root = scene.root();
    let desktop = scene.tree_create(root);
    scene.rect_create(desktop, (320, 200), [1.0; 4]);
    scene.rect_create(desktop, (160, 200), [0.0, 0.0, 0.0, 1.0]);
    let windows = scene.tree_create(root);
    let _under =
        over_glass.then(|| glass_window(&mut scene, &ctx, windows, (60, 20), (200, 160), None));
    let sheet = scene.tree_create(root);
    scene.set_position(sheet, 100, 50);
    scene.set_opacity(sheet, opacity);
    let chrome = scene.tree_create(sheet);
    leave_out(&mut scene, chrome);
    let mut glass = Backdrop::new(&ctx);
    glass.set(&mut scene, &ctx, chrome, Some(sheet_pane((120, 100), &[])));
    let red = red_at(&mut renderer, &ctx, &scene, material, &[(157, 100)])[0];
    Some((red, ctx.made.borrow().len()))
}

#[test]
fn glass_in_a_faded_tree_blurs_in_proportion_to_the_trees_opacity() {
    for material in [Material::Frosted, Material::Smoked, Material::Molten] {
        for over_glass in [false, true] {
            let Some((blurred, _)) = faded_edge(material, 1.0, over_glass) else {
                reprisekit::pins::skipped("no renderer");
                return;
            };
            let (half, _) = faded_edge(material, 0.5, over_glass).unwrap();
            let (bare, _) = faded_edge(material, 0.0, over_glass).unwrap();
            let what = format!("{material:?}, over glass {over_glass}");
            assert!(
                blurred > bare,
                "{what}: the glass blurs white into the black"
            );
            let mid = (blurred + bare) / 2;
            assert!(
                half.abs_diff(mid) <= 2,
                "{what}: at 0.5 the edge is halfway, {mid}: {half}"
            );
        }
    }
}

#[test]
fn glass_in_a_tree_at_opacity_0_makes_no_element() {
    let Some((red, made)) = faded_edge(Material::Molten, 0.0, false) else {
        reprisekit::pins::skipped("no renderer");
        return;
    };
    assert_eq!(made, 0, "no glass element");
    assert_eq!(red, 0, "the black shows unblurred");
}

#[test]
fn only_a_coarse_frame_that_holds_nothing_trails() {
    assert!(trails(Quality::Coarse, Hold::No));
    assert!(
        !trails(Quality::Full, Hold::No),
        "no drag, or fast_drag off"
    );
    assert!(
        !trails(Quality::Coarse, Hold::Keep(Under::Anywhere)),
        "a held frame keeps its blur as before"
    );
}

/// White under a 120x80 sheet of glass at (20, 20), with a black spot at (40, 40) left hidden.
fn spot_under_glass(ctx: &Rc<GlassCtx>) -> (DeskScene, TreeId, TreeId, Backdrop) {
    let mut scene = DeskScene::new();
    let root = scene.root();
    let desktop = scene.tree_create(root);
    scene.rect_create(desktop, (320, 200), [1.0; 4]);
    let spot = scene.tree_create(desktop);
    scene.set_position(spot, 40, 40);
    scene.rect_create(spot, (60, 40), [0.0, 0.0, 0.0, 1.0]);
    scene.set_enabled(spot, false);
    let windows = scene.tree_create(root);
    let (window, glass) = glass_window(&mut scene, ctx, windows, (20, 20), (120, 80), None);
    (scene, spot, window, glass)
}

/// What one screen frame did: the red channel of the points asked for, whether it showed the last
/// pictures (`trails`) and whether the update after it changed them.
struct Screen {
    red: Vec<u32>,
    late: bool,
    changed: bool,
}

fn trail_settings() -> Settings {
    Settings {
        streaks: false,
        ..Settings::default()
    }
}

/// One screen frame of `scene` on `output` as the backend draws it at `quality`.
fn screen_frame(
    renderer: &mut GlesRenderer,
    ctx: &GlassCtx,
    scene: &DeskScene,
    output: &Output,
    quality: Quality,
    points: &[(i32, i32)],
) -> Screen {
    let clear = Color32F::from([0.0, 0.0, 0.0, 1.0]);
    ctx.begin(output, trail_settings(), quality, Hold::No);
    let late = trails(quality, Hold::No) && ctx.show_last(scene, output);
    if !late {
        ctx.update_screen_backdrop(renderer, scene, output, clear, true);
    }
    let size = ctx.pass.get().output;
    let mut elements = scene.render_elements_for(renderer, output);
    let scale = Scale::from(output.current_scale().fractional_scale());
    if ctx.settle(output, &elements, scale) {
        drop(elements);
        ctx.update_backdrop(renderer, scene, output, clear, true);
        elements = scene.render_elements_for(renderer, output);
    }
    let mut target = crate::offscreen::Target::new(renderer, size, 1.0).unwrap();
    target.render(renderer, &elements).unwrap();
    let canvas = target.read(renderer).unwrap();
    drop(elements);
    let changed = late && ctx.update_late(renderer, scene, output, clear);
    let red = (points.iter())
        .map(|&(x, y)| (canvas.pixels[(y * size.w + x) as usize] >> 16) & 0xFF)
        .collect();
    Screen { red, late, changed }
}

fn shown_picture(ctx: &GlassCtx) -> Option<u64> {
    ctx.source.borrow().as_ref().map(|s| s.version().0)
}

#[test]
fn a_trailing_frame_shows_the_last_picture_and_blurs_the_change_after_it() {
    let Ok(mut renderer) = reprisefx::headless::renderer() else {
        reprisekit::pins::skipped("no renderer");
        return;
    };
    let ctx = GlassCtx::new();
    ctx.set_renderer(&mut renderer);
    let (mut scene, spot, _window, _glass) = spot_under_glass(&ctx);
    let output = test_output_of(320, 200);
    let at = [(70, 60)];
    let first = screen_frame(&mut renderer, &ctx, &scene, &output, Quality::Coarse, &at);
    assert!(
        !first.late,
        "a drag's frame with no picture yet draws its glass first"
    );
    let texture = ctx.backdrops.borrow()["test"].texture.tex_id();
    let picture = shown_picture(&ctx);
    scene.set_enabled(spot, true);
    let late = screen_frame(&mut renderer, &ctx, &scene, &output, Quality::Coarse, &at);
    assert!(late.late, "a drag's frame trails");
    assert_eq!(late.red, first.red, "it shows the picture as it was");
    assert!(late.changed, "the update after it blurred the spot");
    assert!(
        backdrop_red(&mut renderer, &ctx, (70, 60)) < 20,
        "the spot is in the backdrop"
    );
    let next = screen_frame(&mut renderer, &ctx, &scene, &output, Quality::Coarse, &at);
    assert!(
        next.red[0] < first.red[0],
        "the next frame shows it: {:?}",
        next.red
    );
    assert!(!next.changed, "nothing changed since: no frame is asked");
    assert_eq!(
        ctx.backdrops.borrow()["test"].texture.tex_id(),
        texture,
        "the backdrop keeps its texture"
    );
    assert_eq!(shown_picture(&ctx), picture, "and glass its picture");
}

#[test]
fn the_frame_after_a_drag_draws_fresh_glass() {
    let Ok(mut renderer) = reprisefx::headless::renderer() else {
        reprisekit::pins::skipped("no renderer");
        return;
    };
    let points: Vec<(i32, i32)> = (0..10).map(|i| (60 + i * 9, 50)).collect();
    let ctx = GlassCtx::new();
    ctx.set_renderer(&mut renderer);
    let (mut scene, spot, window, _glass) = spot_under_glass(&ctx);
    let output = test_output_of(320, 200);
    screen_frame(&mut renderer, &ctx, &scene, &output, Quality::Full, &[]);
    scene.set_position(window, 30, 24);
    let started = screen_frame(&mut renderer, &ctx, &scene, &output, Quality::Coarse, &[]);
    assert!(
        !started.late,
        "the drag's first frame draws its glass first: the full blur read less"
    );
    scene.set_position(window, 40, 28);
    scene.set_enabled(spot, true);
    let last = screen_frame(
        &mut renderer,
        &ctx,
        &scene,
        &output,
        Quality::Coarse,
        &points,
    );
    assert!(last.late, "the drag's next frame trails");
    let ended = screen_frame(&mut renderer, &ctx, &scene, &output, Quality::Full, &points);
    assert!(
        !ended.late,
        "the frame after the drag draws its glass first"
    );
    let fresh_ctx = GlassCtx::new();
    fresh_ctx.set_renderer(&mut renderer);
    let (mut fresh, spot, window, _glass) = spot_under_glass(&fresh_ctx);
    fresh.set_position(window, 40, 28);
    fresh.set_enabled(spot, true);
    let fresh = screen_frame(
        &mut renderer,
        &fresh_ctx,
        &fresh,
        &output,
        Quality::Full,
        &points,
    );
    assert_ne!(last.red, fresh.red, "the drag's last frame trailed");
    assert_eq!(ended.red, fresh.red, "as a first frame of the same scene");
}

#[test]
fn a_new_size_scale_or_look_updates_the_picture_before_the_frame() {
    let Ok(mut renderer) = reprisefx::headless::renderer() else {
        reprisekit::pins::skipped("no renderer");
        return;
    };
    let ctx = GlassCtx::new();
    ctx.set_renderer(&mut renderer);
    let (scene, _spot, _window, _glass) = spot_under_glass(&ctx);
    let output = test_output_of(320, 200);
    let settings = trail_settings();
    let shows_last = |output: &Output, settings: Settings| {
        ctx.begin(output, settings, Quality::Coarse, Hold::No);
        ctx.show_last(&scene, output)
    };
    assert!(!shows_last(&output, settings), "no picture yet");
    screen_frame(&mut renderer, &ctx, &scene, &output, Quality::Coarse, &[]);
    assert!(
        shows_last(&output, settings),
        "the same output and settings show the last picture"
    );
    let scheme = Settings {
        color: reprisekit::theme::Color::rgb(200, 40, 40),
        ..settings
    };
    assert!(!shows_last(&output, scheme), "another colour");
    let blur = Settings {
        blur: settings.blur % 5 + 1,
        ..settings
    };
    assert!(!shows_last(&output, blur), "another blur");
    let wider = test_output_of(400, 200);
    assert!(!shows_last(&wider, settings), "another size");
    let doubled = test_output_of(320, 200);
    doubled.change_current_state(None, None, Some(smithay::output::Scale::Integer(2)), None);
    assert!(!shows_last(&doubled, settings), "another scale");
}

#[test]
fn a_coarse_pass_draws_the_backdrop_a_lead_past_what_its_glass_reads() {
    let Ok(mut renderer) = reprisefx::headless::renderer() else {
        reprisekit::pins::skipped("no renderer");
        return;
    };
    let ctx = GlassCtx::new();
    ctx.set_renderer(&mut renderer);
    let (scene, _spot, _window, _glass) = spot_under_glass(&ctx);
    let output = test_output_of(320, 200);
    let pass = Pass {
        settings: trail_settings(),
        output: (320, 200).into(),
        quality: Quality::Coarse,
        ..Pass::default()
    };
    let feed = Reads::of(
        &sheet_pane((120, 80), &[]),
        (20.0, 20.0).into(),
        pass.output,
        1.0,
        &pass,
    )
    .feed
    .unwrap();
    let past = (feed.loc.x + feed.size.w + LATE_LEAD / 2, 60);
    screen_frame(&mut renderer, &ctx, &scene, &output, Quality::Full, &[]);
    assert!(
        backdrop_red(&mut renderer, &ctx, past) < 20,
        "a full pass leaves {past:?} clear"
    );
    screen_frame(&mut renderer, &ctx, &scene, &output, Quality::Coarse, &[]);
    assert!(
        backdrop_red(&mut renderer, &ctx, past) > 230,
        "a coarse pass draws it"
    );
}

/// A capture's backdrop update of `output` during a drag, as `capture::render_scene_into` makes it.
fn capture_update(renderer: &mut GlesRenderer, ctx: &GlassCtx, scene: &DeskScene, output: &Output) {
    let clear = Color32F::from([0.0, 0.0, 0.0, 1.0]);
    let hold = ctx.capture_hold(output);
    ctx.begin(output, trail_settings(), Quality::Coarse, hold);
    ctx.update_screen_backdrop(renderer, scene, output, clear, true);
}

#[test]
fn a_capture_between_trailing_frames_of_a_still_drag_changes_no_picture() {
    let Ok(mut renderer) = reprisefx::headless::renderer() else {
        reprisekit::pins::skipped("no renderer");
        return;
    };
    let ctx = GlassCtx::new();
    ctx.set_renderer(&mut renderer);
    let (mut scene, _spot, window, lower) = spot_under_glass(&ctx);
    let windows = scene.parent(window).unwrap();
    let _upper = glass_window(&mut scene, &ctx, windows, (100, 60), (120, 80), None);
    let output = test_output_of(320, 200);
    let lower = ("test".to_string(), Rc::as_ptr(&lower.glass));
    let private = |ctx: &GlassCtx| ctx.beneath.borrow()[&lower].backdrop.texture.tex_id();
    screen_frame(&mut renderer, &ctx, &scene, &output, Quality::Coarse, &[]);
    let first = screen_frame(&mut renderer, &ctx, &scene, &output, Quality::Coarse, &[]);
    assert!(
        first.late && !first.changed,
        "a still drag trails and changes nothing"
    );
    let texture = private(&ctx);
    capture_update(&mut renderer, &ctx, &scene, &output);
    let next = screen_frame(&mut renderer, &ctx, &scene, &output, Quality::Coarse, &[]);
    assert!(next.late, "the frame after the capture trails");
    assert!(
        !next.changed,
        "the capture read what the screen reads: no frame is asked"
    );
    assert_eq!(
        private(&ctx),
        texture,
        "the lower glass keeps its private backdrop"
    );
}

#[test]
fn hidden_glass_neither_trails_nor_draws_its_backdrop() {
    let Ok(mut renderer) = reprisefx::headless::renderer() else {
        reprisekit::pins::skipped("no renderer");
        return;
    };
    let ctx = GlassCtx::new();
    ctx.set_renderer(&mut renderer);
    let (mut scene, spot, _window, _glass) = spot_under_glass(&ctx);
    let output = test_output_of(320, 200);
    screen_frame(&mut renderer, &ctx, &scene, &output, Quality::Coarse, &[]);
    let cover = scene.tree_create(scene.root());
    scene.rect_create(cover, (320, 200), [0.0, 0.0, 1.0, 1.0]);
    let hiding = screen_frame(&mut renderer, &ctx, &scene, &output, Quality::Coarse, &[]);
    assert!(hiding.late, "the frame that covers the glass trails");
    scene.set_enabled(spot, true);
    let hidden = screen_frame(&mut renderer, &ctx, &scene, &output, Quality::Coarse, &[]);
    assert!(
        !hidden.late && !hidden.changed,
        "covered glass does not trail"
    );
    assert!(
        backdrop_red(&mut renderer, &ctx, (70, 60)) > 230,
        "the spot under covered glass is not drawn"
    );
    scene.set_enabled(cover, false);
    let shown = screen_frame(
        &mut renderer,
        &ctx,
        &scene,
        &output,
        Quality::Coarse,
        &[(70, 60)],
    );
    assert!(!shown.late, "glass shown again draws first");
    assert!(
        backdrop_red(&mut renderer, &ctx, (70, 60)) < 20,
        "the spot is drawn once glass shows"
    );
}

#[test]
fn glass_past_what_the_last_update_drew_draws_first() {
    let Ok(mut renderer) = reprisefx::headless::renderer() else {
        reprisekit::pins::skipped("no renderer");
        return;
    };
    let ctx = GlassCtx::new();
    ctx.set_renderer(&mut renderer);
    let (mut scene, _spot, window, _glass) = spot_under_glass(&ctx);
    let windows = scene.parent(window).unwrap();
    let output = test_output_of(320, 200);
    let frame = |scene: &DeskScene, renderer: &mut GlesRenderer| {
        screen_frame(renderer, &ctx, scene, &output, Quality::Coarse, &[]).late
    };
    frame(&scene, &mut renderer);
    scene.set_position(window, 20 + LATE_LEAD / 2, 20);
    assert!(
        frame(&scene, &mut renderer),
        "a move inside the lead trails"
    );
    scene.set_position(window, 20 + LATE_LEAD / 2 + LATE_LEAD + 8, 20);
    assert!(
        !frame(&scene, &mut renderer),
        "a move past the lead draws first"
    );
    assert!(
        frame(&scene, &mut renderer),
        "and the next frame trails again"
    );
    let _new = glass_window(&mut scene, &ctx, windows, (10, 140), (60, 40), None);
    assert!(!frame(&scene, &mut renderer), "new glass draws first");
}

#[test]
fn a_late_change_past_the_glass_but_inside_the_lead_asks_no_frame() {
    let Ok(mut renderer) = reprisefx::headless::renderer() else {
        reprisekit::pins::skipped("no renderer");
        return;
    };
    let ctx = GlassCtx::new();
    ctx.set_renderer(&mut renderer);
    let (mut scene, _spot, _window, _glass) = spot_under_glass(&ctx);
    let pass = Pass {
        settings: trail_settings(),
        output: (320, 200).into(),
        quality: Quality::Coarse,
        ..Pass::default()
    };
    let reads = Reads::of(
        &sheet_pane((120, 80), &[]),
        (20.0, 20.0).into(),
        pass.output,
        1.0,
        &pass,
    );
    let reach = reads
        .reach
        .iter()
        .map(|r| r.loc.x + r.size.w)
        .max()
        .unwrap();
    // Moved inside the cut: one shown, hidden or moved into it damages all those behind it.
    let beside = scene.tree_create(scene.root());
    scene.set_position(beside, reach + 4, 20);
    scene.rect_create(beside, (8, 20), [0.0, 0.0, 0.0, 1.0]);
    let output = test_output_of(320, 200);
    screen_frame(&mut renderer, &ctx, &scene, &output, Quality::Coarse, &[]);
    screen_frame(&mut renderer, &ctx, &scene, &output, Quality::Coarse, &[]);
    let version = ctx.source.borrow().as_ref().map(Source::version);
    scene.set_position(beside, reach + 4, 50);
    let late = screen_frame(&mut renderer, &ctx, &scene, &output, Quality::Coarse, &[]);
    assert!(late.late, "the frame trails");
    assert!(
        backdrop_red(&mut renderer, &ctx, (reach + 8, 64)) < 20,
        "the late update draws the change past the glass's reach"
    );
    assert_ne!(
        ctx.source.borrow().as_ref().map(Source::version),
        version,
        "and blurs it"
    );
    assert!(!late.changed, "no glass reads it: no frame is asked");
}
