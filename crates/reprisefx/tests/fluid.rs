//! The fluid sim stepped on the first EGL device (`LIBGL_ALWAYS_SOFTWARE=1` for Mesa's) and read
//! back; with no EGL device or no half-float targets each test says so and passes.

use reprisefx::fluid::{FluidElement, FluidFx, FluidPool};
use reprisefx::headless;
use reprisekit::fluid::{DYE_DIV, Force, Hands, Look, Profile, SIM_DIV, Source};
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::element::Id;
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
use smithay::backend::renderer::utils::CommitCounter;
use smithay::backend::renderer::{Bind, ExportMem, ImportMem, Offscreen, Texture};
use smithay::render_elements;
use smithay::utils::{Rectangle, Size, Transform};

const DT: f32 = 1.0 / 60.0;

fn rig() -> Option<(GlesRenderer, FluidFx)> {
    let mut r = match headless::renderer() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("skipped: {e}");
            return None;
        }
    };
    match FluidFx::new(&mut r) {
        Ok(fx) => Some((r, fx)),
        Err(e) => {
            eprintln!("skipped: no fluid on this device: {e}");
            None
        }
    }
}

fn look() -> &'static Look {
    Profile::Ink.look()
}

fn pool(r: &mut GlesRenderer, fx: &FluidFx, w: i32, h: i32) -> FluidPool {
    let mut p = FluidPool::new();
    assert!(
        p.fit(r, fx, Size::from((w, h))).unwrap(),
        "a new pool is made"
    );
    p
}

fn splat(at: (f32, f32), vel: (f32, f32), radius: f32) -> Force {
    Force::Splat {
        from: at,
        at,
        vel,
        radius,
        colour: [1.0, 0.5, 0.25],
        amount: 1.0,
    }
}

/// The splat radius, as a multiple of the look's, that spans `px` on an output `shorter` px high.
fn px(px: f32, shorter: i32) -> f32 {
    px / (look().splat_radius * shorter as f32)
}

fn max_channel(t: [f32; 4]) -> f32 {
    t[0].max(t[1]).max(t[2])
}

fn centroid(dye: &[[f32; 4]], w: usize) -> (f32, f32) {
    let (mut sx, mut sy, mut s) = (0.0, 0.0, 0.0);
    for (i, t) in dye.iter().enumerate() {
        let m = t[0] + t[1] + t[2];
        sx += m * (i % w) as f32;
        sy += m * (i / w) as f32;
        s += m;
    }
    assert!(s > 0.0, "the pool holds dye");
    (sx / s, sy / s)
}

fn mean_abs(v: &[f32]) -> f32 {
    v.iter().map(|d| d.abs()).sum::<f32>() / v.len() as f32
}

#[test]
fn splat_puts_dye_where_it_lands() {
    let Some((mut r, fx)) = rig() else { return };
    let mut p = pool(&mut r, &fx, 640, 360);
    p.step(
        &mut r,
        &fx,
        &[splat((320.0, 180.0), (0.0, 0.0), 3.0)],
        1.0,
        look(),
        DT,
    )
    .unwrap();
    let dye = p.dye(&mut r).unwrap();
    let w = (640 / DYE_DIV) as usize;
    let h = (360 / DYE_DIV) as usize;
    assert_eq!(dye.len(), w * h);
    let centre = dye[(h / 2) * w + w / 2];
    assert!(
        max_channel(centre) > 0.5,
        "the centre holds dye: {centre:?}"
    );
    assert_eq!(dye[0], [0.0; 4], "the corner stays empty");
    assert_eq!(dye[w * h - 1], [0.0; 4], "the far corner stays empty");
}

#[test]
fn a_segment_splat_lays_dye_along_its_whole_length() {
    let Some((mut r, fx)) = rig() else { return };
    let mut p = pool(&mut r, &fx, 640, 360);
    let stroke = Force::Splat {
        from: (100.0, 180.0),
        at: (540.0, 180.0),
        vel: (0.0, 0.0),
        radius: px(10.0, 360),
        colour: [1.0, 0.5, 0.25],
        amount: 1.0,
    };
    p.step(&mut r, &fx, &[stroke], 1.0, look(), DT).unwrap();
    let dye = p.dye(&mut r).unwrap();
    let w = (640 / DYE_DIV) as usize;
    let row = 180 / DYE_DIV as usize;
    for x in [100, 200, 320, 440, 540] {
        let t = dye[row * w + x / DYE_DIV as usize];
        assert!(max_channel(t) > 0.9, "dye at x {x}: {t:?}");
    }
    let past = dye[row * w + 600 / DYE_DIV as usize];
    assert!(max_channel(past) < 0.01, "no dye past the end: {past:?}");
}

#[test]
fn a_splat_changes_only_the_dye_near_it() {
    let Some((mut r, fx)) = rig() else { return };
    let wash = Force::Splat {
        from: (320.0, 0.0),
        at: (320.0, 360.0),
        vel: (300.0, 0.0),
        radius: px(300.0, 360),
        colour: [0.2, 0.3, 0.4],
        amount: 1.0,
    };
    let (a, b) = ((100.0, 80.0), (500.0, 280.0));
    let radius = 10.0;
    let dots = [a, b].map(|at| splat(at, (0.0, 0.0), px(radius, 360)));
    let mut field = |forces: &[Force]| {
        let mut p = pool(&mut r, &fx, 640, 360);
        p.step(&mut r, &fx, &[wash], 1.0, look(), DT).unwrap();
        for _ in 0..3 {
            p.step(&mut r, &fx, &[], 1.0, look(), DT).unwrap();
        }
        p.step(&mut r, &fx, forces, 1.0, look(), DT).unwrap();
        p.dye(&mut r).unwrap()
    };
    let (plain, dotted) = (field(&[]), field(&dots));
    let w = (640 / DYE_DIV) as usize;
    // The step's advection carries each dot up to 5 px; past that its dye is a Gaussian tail.
    let drift = 6.0;
    let mut tail = 0;
    for (i, (t0, t1)) in plain.iter().zip(&dotted).enumerate() {
        let (x, y) = ((i % w) as i32 * DYE_DIV, (i / w) as i32 * DYE_DIV);
        let d = [a, b]
            .map(|at| (x as f32 - at.0).hypot(y as f32 - at.1))
            .into_iter()
            .fold(f32::MAX, f32::min);
        if d < 2.0 * radius {
            continue;
        }
        tail += usize::from(d < 3.0 * radius);
        let k = ((d - drift).max(0.0) / radius).powi(2);
        let room = (-k).exp() + 0.002;
        let moved = (0..4).map(|k| (t0[k] - t1[k]).abs()).fold(0.0, f32::max);
        assert!(
            moved < room,
            "texel ({x}, {y}) {d} px from a splat: {t0:?} -> {t1:?}"
        );
    }
    assert!(tail > 100, "the edge of each splat's reach is checked");
    for at in [a, b] {
        let i = (at.1 as usize / DYE_DIV as usize) * w + at.0 as usize / DYE_DIV as usize;
        assert!(
            dotted[i][0] > plain[i][0] + 0.5,
            "the splat at {at:?} landed"
        );
    }
}

#[test]
fn splat_lands_off_centre_where_it_was_put() {
    let Some((mut r, fx)) = rig() else { return };
    let mut p = pool(&mut r, &fx, 640, 360);
    p.step(
        &mut r,
        &fx,
        &[splat((160.0, 90.0), (0.0, 0.0), 3.0)],
        2.0,
        look(),
        DT,
    )
    .unwrap();
    let w = (640 / DYE_DIV) as usize;
    let (x, y) = centroid(&p.dye(&mut r).unwrap(), w);
    assert!((x - 160.0).abs() < 1.0, "x at 160 dye texels: {x}");
    assert!((y - 90.0).abs() < 1.0, "y at 90 dye texels: {y}");
}

#[test]
fn flow_carries_dye_along_velocity() {
    let Some((mut r, fx)) = rig() else { return };
    let mut p = pool(&mut r, &fx, 640, 360);
    let w = (640 / DYE_DIV) as usize;
    p.step(
        &mut r,
        &fx,
        &[splat((320.0, 180.0), (0.0, 0.0), 3.0)],
        1.0,
        look(),
        0.0,
    )
    .unwrap();
    let before = centroid(&p.dye(&mut r).unwrap(), w);
    p.step(
        &mut r,
        &fx,
        &[splat((320.0, 180.0), (500.0, 0.0), 3.0)],
        1.0,
        look(),
        DT,
    )
    .unwrap();
    for _ in 1..30 {
        p.step(&mut r, &fx, &[], 1.0, look(), DT).unwrap();
    }
    let after = centroid(&p.dye(&mut r).unwrap(), w);
    assert!(
        after.0 - before.0 >= 5.0,
        "dye moves right at least 5 texels: {before:?} -> {after:?}"
    );
    assert!(
        (after.1 - before.1).abs() <= 1.0,
        "dye keeps its row within 1 texel: {before:?} -> {after:?}"
    );
}

fn max_abs(v: &[f32]) -> f32 {
    v.iter().fold(0.0, |m, d| m.max(d.abs()))
}

// dt 0 projects without advecting, which would bring divergence back; twenty Jacobi iterations
// leave a faint far field, so only the splat's own divergence must go.
#[test]
fn projection_leaves_little_divergence() {
    let Some((mut r, fx)) = rig() else { return };
    let push = [splat((320.0, 180.0), (3000.0, 1500.0), px(13.0, 360))];
    let mut raw = pool(&mut r, &fx, 640, 360);
    raw.set_pressure_iters(0);
    raw.step(&mut r, &fx, &push, 1.0, look(), 0.0).unwrap();
    let before = raw.divergence(&mut r, &fx).unwrap();
    assert!(max_abs(&before) > 1.0, "the push alone diverges");

    let mut p = pool(&mut r, &fx, 640, 360);
    p.step(&mut r, &fx, &push, 1.0, look(), 0.0).unwrap();
    let after = p.divergence(&mut r, &fx).unwrap();
    assert!(
        max_abs(&after) < max_abs(&before) * 0.05,
        "projection leaves under 5% of the peak divergence: {} -> {}",
        max_abs(&before),
        max_abs(&after)
    );
    assert!(
        mean_abs(&after) < mean_abs(&before) * 0.3,
        "projection leaves under 30% of the mean divergence: {} -> {}",
        mean_abs(&before),
        mean_abs(&after)
    );
}

#[test]
fn swirl_turns_counter_clockwise_on_screen() {
    let Some((mut r, fx)) = rig() else { return };
    let mut p = pool(&mut r, &fx, 640, 360);
    let w = (640 / DYE_DIV) as usize;
    p.step(
        &mut r,
        &fx,
        &[splat((370.0, 180.0), (0.0, 0.0), 3.0)],
        1.0,
        look(),
        0.0,
    )
    .unwrap();
    let before = centroid(&p.dye(&mut r).unwrap(), w);
    let swirl = Force::Swirl {
        centre: (320.0, 180.0),
        radius: 100.0,
        spin: 3.0,
    };
    p.step(&mut r, &fx, &[swirl], 1.0, look(), DT).unwrap();
    for _ in 1..30 {
        p.step(&mut r, &fx, &[], 1.0, look(), DT).unwrap();
    }
    let after = centroid(&p.dye(&mut r).unwrap(), w);
    assert!(
        before.1 - after.1 >= 5.0,
        "dye right of the centre moves up: {before:?} -> {after:?}"
    );
}

#[test]
fn dye_halves_over_its_half_life() {
    let Some((mut r, fx)) = rig() else { return };
    let mut p = pool(&mut r, &fx, 640, 360);
    let w = (640 / DYE_DIV) as usize;
    let at = 90 * w + 160;
    p.step(
        &mut r,
        &fx,
        &[splat((320.0, 180.0), (0.0, 0.0), 3.0)],
        1.0,
        look(),
        0.0,
    )
    .unwrap();
    let full = p.dye(&mut r).unwrap()[at][0];
    p.step(&mut r, &fx, &[], 1.0, look(), look().dye_half_life)
        .unwrap();
    let half = p.dye(&mut r).unwrap()[at][0];
    assert!((half / full - 0.5).abs() < 0.01, "{full} -> {half}");
}

#[test]
fn dye_fades_at_144_hz() {
    let Some((mut r, fx)) = rig() else { return };
    let mut p = pool(&mut r, &fx, 640, 360);
    let w = (640 / DYE_DIV) as usize;
    let at = 90 * w + 160;
    p.step(
        &mut r,
        &fx,
        &[splat((320.0, 180.0), (0.0, 0.0), 3.0)],
        1.0,
        look(),
        0.0,
    )
    .unwrap();
    let start = p.dye(&mut r).unwrap()[at][0];
    for _ in 0..600 {
        p.step(&mut r, &fx, &[], 1.0, look(), 1.0 / 144.0).unwrap();
    }
    let end = p.dye(&mut r).unwrap()[at][0];
    let want = 0.5f32.powf(600.0 / 144.0 / look().dye_half_life);
    assert!(
        (end / start - want).abs() < 0.02,
        "{start} -> {end}, want {want} of it"
    );
}

#[test]
fn clear_empties_the_pool() {
    let Some((mut r, fx)) = rig() else { return };
    let mut p = pool(&mut r, &fx, 640, 360);
    p.step(
        &mut r,
        &fx,
        &[splat((320.0, 180.0), (400.0, 0.0), 3.0)],
        1.0,
        look(),
        DT,
    )
    .unwrap();
    p.step(&mut r, &fx, &[], 1.0, look(), DT).unwrap();
    p.step(&mut r, &fx, &[Force::Clear], 1.0, look(), DT)
        .unwrap();
    p.step(&mut r, &fx, &[], 1.0, look(), DT).unwrap();
    assert!(p.dye(&mut r).unwrap().iter().all(|t| *t == [0.0; 4]));
    assert!(p.divergence(&mut r, &fx).unwrap().iter().all(|d| *d == 0.0));
}

#[test]
fn a_clear_drops_only_the_forces_before_it() {
    let Some((mut r, fx)) = rig() else { return };
    let mut p = pool(&mut r, &fx, 640, 360);
    let forces = [
        splat((100.0, 100.0), (0.0, 0.0), 3.0),
        Force::Clear,
        splat((500.0, 260.0), (0.0, 0.0), 3.0),
    ];
    p.step(&mut r, &fx, &forces, 1.0, look(), DT).unwrap();
    let w = (640 / DYE_DIV) as usize;
    let (x, y) = centroid(&p.dye(&mut r).unwrap(), w);
    assert!(
        (x - 250.0).abs() < 1.0 && (y - 130.0).abs() < 1.0,
        "only the later splat: {x}, {y}"
    );
}

#[test]
fn more_forces_than_one_pass_holds_all_land() {
    let Some((mut r, fx)) = rig() else { return };
    let mut p = pool(&mut r, &fx, 640, 360);
    let forces: Vec<_> = (0..20)
        .map(|i| splat((20.0 + 30.0 * i as f32, 180.0), (0.0, 0.0), 2.0))
        .collect();
    p.step(&mut r, &fx, &forces, 1.0, look(), DT).unwrap();
    let dye = p.dye(&mut r).unwrap();
    let w = (640 / DYE_DIV) as usize;
    for i in 0..20 {
        let x = (10 + 15 * i) as usize;
        let t = dye[90 * w + x];
        assert!(
            max_channel(t) > 0.5,
            "splat {i} lands at dye texel {x}: {t:?}"
        );
    }
}

#[test]
fn fit_resizes_and_clears() {
    let Some((mut r, fx)) = rig() else { return };
    let mut p = pool(&mut r, &fx, 640, 360);
    p.step(
        &mut r,
        &fx,
        &[splat((320.0, 180.0), (0.0, 0.0), 3.0)],
        1.0,
        look(),
        DT,
    )
    .unwrap();
    assert!(
        p.fit(&mut r, &fx, Size::from((800, 600))).unwrap(),
        "a new size remakes the pool"
    );
    assert_eq!(
        p.sim_size(),
        Some(Size::from((800 / SIM_DIV, 600 / SIM_DIV)))
    );
    assert_eq!(
        p.dye_size(),
        Some(Size::from((800 / DYE_DIV, 600 / DYE_DIV)))
    );
    let dye = p.dye(&mut r).unwrap();
    assert_eq!(dye.len(), (800 / DYE_DIV * 600 / DYE_DIV) as usize);
    assert!(dye.iter().all(|t| *t == [0.0; 4]), "the new pool is empty");
    assert!(
        !p.fit(&mut r, &fx, Size::from((800, 600))).unwrap(),
        "the same size keeps it"
    );
}

#[test]
fn every_change_moves_the_commit() {
    let Some((mut r, fx)) = rig() else { return };
    let mut p = pool(&mut r, &fx, 64, 64);
    let a = p.commit();
    p.step(&mut r, &fx, &[], 1.0, look(), DT).unwrap();
    let b = p.commit();
    assert_ne!(a, b, "a step");
    p.clear(&mut r, &fx).unwrap();
    assert_ne!(b, p.commit(), "a clear");
}

render_elements! {
    El<=GlesRenderer>;
    Fluid=FluidElement,
}

const PROFILES: [Profile; 4] = [Profile::Ink, Profile::Glow, Profile::Sparks, Profile::Neon];

/// A `w` x `h` opaque texture, `f` giving each pixel.
fn picture(r: &mut GlesRenderer, w: i32, h: i32, f: impl Fn(i32, i32) -> [u8; 3]) -> GlesTexture {
    let mut bytes = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let [cr, cg, cb] = f(x, y);
            bytes.extend_from_slice(&[cr, cg, cb, 255]);
        }
    }
    r.import_memory(&bytes, Fourcc::Abgr8888, (w, h).into(), false)
        .unwrap()
}

/// Draws `el` on a `w` x `h` output and reads it back RGB, row by row.
fn shoot(r: &mut GlesRenderer, el: FluidElement, w: i32, h: i32) -> Vec<[u8; 3]> {
    let mut target: GlesTexture = r.create_buffer(Fourcc::Abgr8888, (w, h).into()).unwrap();
    let mut tracker = OutputDamageTracker::new((w, h), 1.0, Transform::Normal);
    {
        let mut fb = r.bind(&mut target).unwrap();
        tracker
            .render_output(r, &mut fb, 0, &[El::from(el)], [0.0, 0.0, 0.0, 1.0])
            .unwrap();
    }
    let fb = r.bind(&mut target).unwrap();
    let m = r
        .copy_framebuffer(&fb, Rectangle::from_size((w, h).into()), Fourcc::Abgr8888)
        .unwrap();
    drop(fb);
    r.map_texture(&m)
        .unwrap()
        .as_chunks::<4>()
        .0
        .iter()
        .map(|p| [p[0], p[1], p[2]])
        .collect()
}

/// Shades `p` as `profile` and draws it over `wall` at `level`.
fn render(
    r: &mut GlesRenderer,
    fx: &FluidFx,
    p: &mut FluidPool,
    wall: &GlesTexture,
    (profile, flavour): (Profile, usize),
    level: f32,
) -> Vec<[u8; 3]> {
    let size = wall.size();
    p.shade(r, fx, profile, flavour, 1.0).unwrap();
    let geo = Rectangle::from_size((size.w, size.h).into());
    let el = p
        .element(
            fx,
            &Id::new(),
            wall,
            CommitCounter::default(),
            geo,
            profile.look(),
            flavour,
            level,
        )
        .expect("a fitted pool has an element");
    shoot(r, el, size.w, size.h)
}

fn flavours() -> impl Iterator<Item = (Profile, usize)> {
    PROFILES
        .into_iter()
        .flat_map(|p| (0..p.look().palettes.len()).map(move |f| (p, f)))
}

fn stir(r: &mut GlesRenderer, fx: &FluidFx, p: &mut FluidPool, w: f32, h: f32) {
    let forces = [
        splat((w * 0.4, h * 0.5), (300.0, -100.0), 4.0),
        splat((w * 0.6, h * 0.5), (-300.0, 100.0), 4.0),
        Force::Swirl {
            centre: (w * 0.5, h * 0.5),
            radius: w * 0.2,
            spin: 4.0,
        },
    ];
    p.step(r, fx, &forces, 1.0, look(), DT).unwrap();
    for _ in 0..20 {
        p.step(r, fx, &[], 1.0, look(), DT).unwrap();
    }
}

#[test]
fn level_zero_shows_the_wallpaper() {
    let Some((mut r, fx)) = rig() else { return };
    let (w, h) = (128, 96);
    let paint = |x: i32, y: i32| [(x * 2) as u8, (y * 2 + 30) as u8, ((x + y) % 256) as u8];
    let wall = picture(&mut r, w, h, paint);
    let mut p = pool(&mut r, &fx, w, h);
    stir(&mut r, &fx, &mut p, w as f32, h as f32);
    for pf in flavours() {
        let got = render(&mut r, &fx, &mut p, &wall, pf, 0.0);
        for y in 0..h {
            for x in 0..w {
                let (g, want) = (got[(y * w + x) as usize], paint(x, y));
                for k in 0..3 {
                    assert!(
                        g[k].abs_diff(want[k]) <= 1,
                        "{pf:?} at {x},{y}: {g:?}, want the wallpaper's {want:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn full_level_dims_toward_the_backdrop() {
    let Some((mut r, fx)) = rig() else { return };
    let (w, h) = (96, 96);
    let wall = picture(&mut r, w, h, |_, _| [128; 3]);
    let mut p = pool(&mut r, &fx, w, h);
    for pf in flavours() {
        let look = pf.0.look();
        let want = look.backdrop[pf.1].map(|b| {
            let grey = 128.0 / 255.0;
            (grey + (b - grey) * look.dim) * 255.0
        });
        let got = render(&mut r, &fx, &mut p, &wall, pf, 1.0);
        // Tile centres: Neon's empty tiles keep their outline.
        for y in [12, 36, 60, 84] {
            for x in [12, 36, 60, 84] {
                let g = got[(y * w + x) as usize];
                for k in 0..3 {
                    assert!(
                        (f32::from(g[k]) - want[k]).abs() <= 2.0,
                        "{pf:?} at {x},{y}: {g:?}, want {want:?}"
                    );
                }
            }
        }
    }
}

fn mean_diff(a: &[[u8; 3]], b: &[[u8; 3]]) -> f32 {
    let sum: u32 = a
        .iter()
        .zip(b)
        .map(|(a, b)| (0..3).map(|k| u32::from(a[k].abs_diff(b[k]))).sum::<u32>())
        .sum();
    sum as f32 / (a.len() * 3) as f32
}

#[test]
fn looks_differ() {
    let Some((mut r, fx)) = rig() else { return };
    let (w, h) = (256, 192);
    let wall = picture(&mut r, w, h, |x, y| [(x / 2) as u8, (y / 2) as u8, 90]);
    let mut p = pool(&mut r, &fx, w, h);
    stir(&mut r, &fx, &mut p, w as f32, h as f32);
    let crops: Vec<Vec<[u8; 3]>> = PROFILES
        .iter()
        .map(|&pf| {
            let got = render(&mut r, &fx, &mut p, &wall, (pf, 0), 1.0);
            (64..128)
                .flat_map(|y| (96..160).map(move |x| (x, y)))
                .map(|(x, y)| got[(y * w + x) as usize])
                .collect()
        })
        .collect();
    for i in 0..4 {
        for j in i + 1..4 {
            let d = mean_diff(&crops[i], &crops[j]);
            assert!(
                d > 4.0,
                "{:?} and {:?} differ by a mean {d} per channel, want over 4",
                PROFILES[i],
                PROFILES[j]
            );
        }
    }
}

fn luma(c: [u8; 3]) -> f32 {
    0.2126 * f32::from(c[0]) + 0.7152 * f32::from(c[1]) + 0.0722 * f32::from(c[2])
}

#[test]
fn glow_is_brighter_on_edges_than_inside() {
    let Some((mut r, fx)) = rig() else { return };
    let (w, h) = (256, 256);
    let wall = picture(&mut r, w, h, |_, _| [40; 3]);
    let mut p = pool(&mut r, &fx, w, h);
    let disc = Force::Splat {
        from: (128.0, 128.0),
        at: (128.0, 128.0),
        vel: (0.0, 0.0),
        radius: 40.0 / (Profile::Glow.look().splat_radius * 256.0),
        colour: Profile::Glow.look().palettes[0][1],
        amount: 40.0,
    };
    p.step(&mut r, &fx, &[disc], 1.0, Profile::Glow.look(), 0.0)
        .unwrap();
    let got = render(&mut r, &fx, &mut p, &wall, (Profile::Glow, 0), 1.0);
    let ring = |lo: f32, hi: f32| {
        let mut v = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let d = ((x - 128) as f32).hypot((y - 128) as f32);
                if (lo..hi).contains(&d) {
                    v.push(luma(got[(y * w + x) as usize]));
                }
            }
        }
        v.iter().sum::<f32>() / v.len() as f32
    };
    let (inside, edge) = (ring(0.0, 25.0), ring(50.0, 95.0));
    assert!(
        edge > inside + 10.0,
        "edge ring luma {edge}, inside {inside}: the edge is brighter"
    );
}

/// The share of the dye grid holding at least `dye` in some channel.
fn covered(dye: &[[f32; 4]], least: f32) -> f32 {
    dye.iter().filter(|t| max_channel(**t) >= least).count() as f32 / dye.len() as f32
}

/// Three fingers circling twice in `secs` on a 1920x1080 output, 120 Hz events and 60 Hz
/// frames, then `drift` seconds with no input; the dye at the end.
fn stir_three(
    r: &mut GlesRenderer,
    fx: &FluidFx,
    profile: Profile,
    secs: f32,
    drift: f32,
) -> Vec<[f32; 4]> {
    let mut p = pool(r, fx, 1920, 1080);
    let mut hands = Hands::new(profile, 0);
    let at = |k: u32, t: f32| {
        let a = std::f32::consts::TAU * (2.0 * t / secs + k as f32 / 3.0);
        (
            960.0 + (k as f32 - 1.0) * 300.0 + 200.0 * a.cos(),
            540.0 + 200.0 * a.sin(),
        )
    };
    for k in 0..3 {
        hands.down(Source::Finger(k), at(k, 0.0), 0);
    }
    let frames = ((secs + drift) * 60.0) as u32;
    for f in 1..=frames {
        let ms = f * 1000 / 60;
        for e in [ms - 8, ms] {
            let t = e as f32 / 1000.0;
            for k in 0..3 {
                if t <= secs {
                    hands.moved(Source::Finger(k), at(k, t), e);
                }
            }
        }
        if (ms as f32 / 1000.0) >= secs && hands.active() {
            for k in 0..3 {
                hands.up(Source::Finger(k), ms);
            }
        }
        let forces = hands.frame(ms);
        p.step(r, fx, &forces, 1.0, profile.look(), DT).unwrap();
    }
    p.dye(r).unwrap()
}

#[test]
fn three_fingers_stirring_two_seconds_cover_well_under_half_the_screen() {
    let Some((mut r, fx)) = rig() else { return };
    for profile in [Profile::Ink, Profile::Glow, Profile::Sparks, Profile::Neon] {
        let dye = stir_three(&mut r, &fx, profile, 2.0, 0.0);
        let (faint, plain) = (covered(&dye, 0.05), covered(&dye, 0.2));
        assert!(
            faint < 0.35,
            "{profile:?}: {faint} of the screen holds faint dye"
        );
        assert!(
            plain > 0.1,
            "{profile:?}: {plain} of the screen holds plain dye"
        );
        let dye = stir_three(&mut r, &fx, profile, 2.0, 3.0);
        let plain = covered(&dye, 0.2);
        assert!(
            plain < 0.45,
            "{profile:?}: {plain} holds plain dye 3 s later"
        );
    }
}

/// Ten fingers circling a 640x360 output for `secs`, with `profile`'s palette.
fn flood(
    r: &mut GlesRenderer,
    fx: &FluidFx,
    (profile, flavour): (Profile, usize),
    secs: f32,
) -> FluidPool {
    let mut p = pool(r, fx, 640, 360);
    let mut hands = Hands::new(profile, flavour);
    let at = |k: u32, ms: u32| {
        let a = std::f32::consts::TAU * (ms as f32 / 2000.0 + k as f32 / 10.0);
        let rad = 30.0 + 12.0 * k as f32;
        (320.0 + rad * a.cos(), 180.0 + 0.8 * rad * a.sin())
    };
    for k in 0..10 {
        hands.down(Source::Finger(k), at(k, 0), 0);
    }
    for f in 1..=(secs * 60.0) as u32 {
        let ms = f * 1000 / 60;
        for k in 0..10 {
            hands.moved(Source::Finger(k), at(k, ms), ms);
        }
        let forces = hands.frame(ms);
        p.step(r, fx, &forces, 1.0, profile.look(), DT).unwrap();
    }
    p
}

/// The pixels over dye of at least `least`, of a 640x360 shot of `p`.
fn dyed(r: &mut GlesRenderer, p: &FluidPool, shot: &[[u8; 3]], least: f32) -> Vec<[u8; 3]> {
    let dye = p.dye(r).unwrap();
    let w = (640 / DYE_DIV) as usize;
    shot.iter()
        .enumerate()
        .filter(|(i, _)| {
            let (x, y) = (i % 640 / DYE_DIV as usize, i / 640 / DYE_DIV as usize);
            max_channel(dye[y * w + x]) >= least
        })
        .map(|(_, c)| *c)
        .collect()
}

fn chroma(c: [u8; 3]) -> f32 {
    (c.iter().max().unwrap() - c.iter().min().unwrap()) as f32
}

/// Share of `px` for which `f` holds.
fn share(px: &[[u8; 3]], f: impl Fn(&[u8; 3]) -> bool) -> f32 {
    px.iter().filter(|c| f(c)).count() as f32 / px.len() as f32
}

fn mean_chroma(px: &[[u8; 3]]) -> f32 {
    px.iter().map(|c| chroma(*c)).sum::<f32>() / px.len() as f32
}

#[test]
fn ten_fingers_stirring_leave_light_ink_coloured_not_black() {
    let Some((mut r, fx)) = rig() else { return };
    let wall = picture(&mut r, 640, 360, |_, _| [128; 3]);
    let mut p = flood(&mut r, &fx, (Profile::Ink, 0), 6.0);
    let shot = render(&mut r, &fx, &mut p, &wall, (Profile::Ink, 0), 1.0);
    let px = dyed(&mut r, &p, &shot, 0.3);
    let dark = share(&px, |c| *c.iter().max().unwrap() < 50);
    assert!(dark < 0.05, "{dark} of the dyed pixels are near black");
    assert!(mean_chroma(&px) > 80.0, "chroma {}", mean_chroma(&px));
}

#[test]
fn ten_fingers_stirring_leave_dark_ink_and_neon_coloured_not_white() {
    let Some((mut r, fx)) = rig() else { return };
    let wall = picture(&mut r, 640, 360, |_, _| [128; 3]);
    for look in [(Profile::Ink, 1), (Profile::Neon, 0)] {
        let mut p = flood(&mut r, &fx, look, 6.0);
        let shot = render(&mut r, &fx, &mut p, &wall, look, 1.0);
        let px = dyed(&mut r, &p, &shot, 0.3);
        let white = share(&px, |c| *c.iter().min().unwrap() > 190);
        assert!(
            white < 0.02,
            "{look:?}: {white} of the dyed pixels are near white"
        );
        assert!(
            mean_chroma(&px) > 50.0,
            "{look:?}: chroma {}",
            mean_chroma(&px)
        );
    }
}
