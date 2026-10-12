//! Strokes drawn on a real GLES renderer and read back. Runs on the first hardware EGL device, or
//! Mesa's software one with `LIBGL_ALWAYS_SOFTWARE=1`; with no EGL device each test says so and
//! passes.

use reprisefx::{Stroke, StrokeElement, StrokeFx, Strokes, headless};
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
use smithay::backend::renderer::{Bind, ExportMem, Offscreen};
use smithay::render_elements;
use smithay::utils::{Physical, Rectangle, Size, Transform};

render_elements! {
    El<=GlesRenderer>;
    Strokes=StrokeElement,
}

const W: i32 = 40;
const H: i32 = 24;

fn rig() -> Option<(GlesRenderer, StrokeFx)> {
    let mut r = match headless::renderer() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("skipped: {e}");
            return None;
        }
    };
    let fx = StrokeFx::new(&mut r).expect("the stroke program links");
    Some((r, fx))
}

fn white(from: [f32; 2], to: [f32; 2], width: f32) -> Stroke {
    Stroke {
        from,
        to,
        width,
        color: [1.0; 4],
    }
}

/// Draw `strokes` at `at` on a `W` x `H` output turned by `t` over black, read back upright: the red
/// channel per pixel, row by row.
fn shoot(
    r: &mut GlesRenderer,
    fx: &StrokeFx,
    strokes: &[Stroke],
    at: (i32, i32),
    t: Transform,
) -> Vec<u8> {
    let mode = t.transform_size(Size::<i32, Physical>::from((W, H)));
    let mut target: GlesTexture = r
        .create_buffer(Fourcc::Abgr8888, (mode.w, mode.h).into())
        .unwrap();
    let mut tracker = OutputDamageTracker::new(mode, 1.0, t);
    let s = Strokes::default();
    let geometry = Rectangle::new(at.into(), (W - at.0, H - at.1).into());
    let el = s.element(fx, geometry, strokes, 1.0);
    {
        let mut fb = r.bind(&mut target).unwrap();
        tracker
            .render_output(r, &mut fb, 0, &[El::from(el)], [0.0, 0.0, 0.0, 1.0])
            .unwrap();
    }
    let fb = r.bind(&mut target).unwrap();
    let m = r
        .copy_framebuffer(
            &fb,
            Rectangle::from_size((mode.w, mode.h).into()),
            Fourcc::Abgr8888,
        )
        .unwrap();
    drop(fb);
    let px: Vec<u8> = r
        .map_texture(&m)
        .unwrap()
        .as_chunks::<4>()
        .0
        .iter()
        .map(|p| p[0])
        .collect();
    (0..H)
        .flat_map(|y| (0..W).map(move |x| (x, y)))
        .map(|(x, y)| {
            let (mx, my) = match t {
                Transform::Normal => (x, y),
                Transform::_90 => (y, W - 1 - x),
                _ => unimplemented!("shoot reads Normal and _90"),
            };
            px[(my * mode.w + mx) as usize]
        })
        .collect()
}

fn at(px: &[u8], x: i32, y: i32) -> u8 {
    px[(y * W + x) as usize]
}

#[test]
fn a_level_bar_fills_whole_pixels_and_nothing_else() {
    let Some((mut r, fx)) = rig() else { return };
    // From x 5 to 15 on the line y = 10, 2 wide: pixels 4..=15 by 9..=10 once the caps add 1.
    let px = shoot(
        &mut r,
        &fx,
        &[white([5.0, 10.0], [15.0, 10.0], 2.0)],
        (0, 0),
        Transform::Normal,
    );
    for y in 0..H {
        for x in 0..W {
            let inside = (4..16).contains(&x) && (9..11).contains(&y);
            assert_eq!(at(&px, x, y), if inside { 255 } else { 0 }, "({x}, {y})");
        }
    }
}

#[test]
fn a_dot_is_a_square_and_strokes_move_with_the_element() {
    let Some((mut r, fx)) = rig() else { return };
    let px = shoot(
        &mut r,
        &fx,
        &[white([4.0, 4.0], [4.0, 4.0], 2.0)],
        (10, 6),
        Transform::Normal,
    );
    let lit: Vec<(i32, i32)> = (0..H)
        .flat_map(|y| (0..W).map(move |x| (x, y)))
        .filter(|&(x, y)| at(&px, x, y) > 0)
        .collect();
    assert_eq!(lit, vec![(13, 9), (14, 9), (13, 10), (14, 10)]);
    assert!(lit.iter().all(|&(x, y)| at(&px, x, y) == 255));
}

#[test]
fn a_slanted_line_has_soft_edges_and_reads_the_same_on_a_turned_output() {
    let Some((mut r, fx)) = rig() else { return };
    let line = [white([3.5, 3.5], [33.5, 19.5], 1.0)];
    let px = shoot(&mut r, &fx, &line, (0, 0), Transform::Normal);
    assert!(
        px.iter().any(|&v| v > 0 && v < 255),
        "partly covered pixels on the edges"
    );
    assert_eq!(
        at(&px, 18, 11),
        255,
        "a pixel centred on the line is covered"
    );
    assert_eq!(at(&px, 18, 3), 0, "far off the line is black");
    let turned = shoot(&mut r, &fx, &line, (0, 0), Transform::_90);
    let off = px
        .iter()
        .zip(&turned)
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap();
    assert!(off <= 2, "{off} apart on the turned output");
}
