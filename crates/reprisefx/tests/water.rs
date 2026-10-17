//! Desktop water's field stepped on a real GLES renderer and read back.
//! Runs on the first hardware EGL device, or Mesa's software one with
//! `LIBGL_ALWAYS_SOFTWARE=1`; with no EGL device each test says so and
//! passes.

use reprisefx::headless;
use reprisefx::water::{Pool, WaterElement, WaterFx};
use reprisekit::water::{REFERENCE_HZ, Splash, Step};
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::element::Id;
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
use smithay::backend::renderer::utils::CommitCounter;
use smithay::backend::renderer::{Bind, ExportMem, ImportMem, Offscreen};
use smithay::render_elements;
use smithay::utils::{Physical, Rectangle, Size, Transform};

fn rig() -> Option<(GlesRenderer, WaterFx)> {
    let mut r = match headless::renderer() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("skipped: {e}");
            return None;
        }
    };
    let fx = WaterFx::new(&mut r).expect("the water shaders compile");
    Some((r, fx))
}

fn pool(r: &mut GlesRenderer, fx: &WaterFx, w: i32, h: i32) -> Pool {
    let mut p = Pool::new();
    assert!(
        p.fit(r, fx, Size::from((w, h))).unwrap(),
        "a new field is made"
    );
    p
}

fn still() -> Step {
    Step::new(1.0, REFERENCE_HZ)
}

fn calm() -> Step {
    Step::new(0.985, REFERENCE_HZ)
}

fn at(h: &[f32], w: i32, x: i32, y: i32) -> f32 {
    h[(y * w + x) as usize]
}

#[test]
fn a_new_field_is_flat() {
    let Some((mut r, fx)) = rig() else { return };
    let p = pool(&mut r, &fx, 32, 16);
    assert!(p.heights(&mut r).unwrap().iter().all(|&h| h == 0.0));
}

#[test]
fn heights_round_trip_through_the_packing() {
    let Some((mut r, fx)) = rig() else { return };
    let mut p = pool(&mut r, &fx, 32, 32);
    // No damping, one step: the disc's centre holds the splash depth.
    for depth in [-0.5f32, 0.25, -0.999] {
        p.flatten(&mut r, &fx).unwrap();
        p.step(
            &mut r,
            &fx,
            &[Splash::disc((16.5, 16.5), 4.0, depth)],
            &[],
            still(),
        )
        .unwrap();
        let h = p.heights(&mut r).unwrap();
        let c = at(&h, 32, 16, 16);
        assert!(
            (c - depth).abs() < 2.0 / 65535.0 * 4.0,
            "{depth} read back as {c}"
        );
    }
}

#[test]
fn a_disc_spreads_to_a_ring() {
    let Some((mut r, fx)) = rig() else { return };
    let mut p = pool(&mut r, &fx, 64, 64);
    p.step(
        &mut r,
        &fx,
        &[Splash::disc((32.5, 32.5), 3.0, -0.8)],
        &[],
        still(),
    )
    .unwrap();
    for _ in 0..12 {
        p.step(&mut r, &fx, &[], &[], still()).unwrap();
    }
    let h = p.heights(&mut r).unwrap();
    // The crest has moved out: the largest |height| along the row through
    // the centre lies 6 to 12 texels away, not at the centre.
    let (far, _) = (0..32)
        .map(|d| (d, at(&h, 64, 32 + d, 32).abs()))
        .fold((0, 0.0f32), |a, b| if b.1 > a.1 { b } else { a });
    assert!((6..=12).contains(&far), "crest at {far}");
    assert!(at(&h, 64, 32, 32).abs() < at(&h, 64, 32 + far, 32).abs());
}

/// How far from a disc's centre its leading crest lies after half a
/// second of undamped steps at `hz`: the outermost texel along the row
/// through the centre still moved by 1% of the splash.
fn front_after_half_a_second(r: &mut GlesRenderer, fx: &WaterFx, hz: f32) -> i32 {
    let mut p = pool(r, fx, 128, 128);
    let step = Step::new(1.0, hz);
    p.step(r, fx, &[Splash::disc((64.5, 64.5), 3.0, -0.8)], &[], step)
        .unwrap();
    for _ in 1..(hz / 2.0).round() as i32 {
        p.step(r, fx, &[], &[], step).unwrap();
    }
    let h = p.heights(r).unwrap();
    (0..64)
        .rev()
        .find(|&d| at(&h, 128, 64 + d, 64).abs() > 0.008)
        .expect("the splash moved the water")
}

#[test]
fn waves_travel_as_far_a_second_at_any_rate() {
    let Some((mut r, fx)) = rig() else { return };
    let at60 = front_after_half_a_second(&mut r, &fx, 60.0);
    let at144 = front_after_half_a_second(&mut r, &fx, 144.0);
    let at240 = front_after_half_a_second(&mut r, &fx, 240.0);
    let at30 = front_after_half_a_second(&mut r, &fx, 30.0);
    assert!(
        (at144 - at60).abs() <= 1,
        "{at60} at 60 Hz, {at144} at 144 Hz"
    );
    assert!(
        (at240 - at60).abs() <= 1,
        "{at60} at 60 Hz, {at240} at 240 Hz"
    );
    // Below the reference rate the speed holds at its stable limit:
    // about half as far in half the steps.
    assert!(
        (at30 - at60 / 2).abs() <= 3,
        "{at60} at 60 Hz, {at30} at 30 Hz"
    );
}

#[test]
fn an_obstacle_stays_flat_and_reflects() {
    let Some((mut r, fx)) = rig() else { return };
    let block: Rectangle<i32, Physical> = Rectangle::new((40, 0).into(), (8, 64).into());
    let run = |r: &mut GlesRenderer, obstacles: &[Rectangle<i32, Physical>]| {
        let mut p = pool(r, &fx, 64, 64);
        p.step(
            r,
            &fx,
            &[Splash::disc((30.5, 32.5), 3.0, -0.8)],
            obstacles,
            still(),
        )
        .unwrap();
        for _ in 0..20 {
            p.step(r, &fx, &[], obstacles, still()).unwrap();
        }
        p.heights(r).unwrap()
    };
    let open = run(&mut r, &[]);
    let walled = run(&mut r, &[block]);
    for y in 0..64 {
        for x in 40..48 {
            assert_eq!(at(&walled, 64, x, y), 0.0, "inside the obstacle at {x},{y}");
        }
    }
    // Left of the wall the reflected wave adds to what open water holds.
    let diff: f32 = (20..40)
        .map(|x| (at(&walled, 64, x, 32) - at(&open, 64, x, 32)).abs())
        .sum();
    assert!(diff > 0.05, "the wall sends the wave back: {diff}");
}

#[test]
fn an_obstacle_is_laid_flat_after_the_step_it_holds() {
    let Some((mut r, fx)) = rig() else { return };
    let blocks: [Rectangle<i32, Physical>; 2] = [
        Rectangle::new((10, 12).into(), (6, 6).into()),
        Rectangle::new((18, 4).into(), (3, 9).into()),
    ];
    let splash = [Splash::disc((14.5, 12.5), 7.0, -0.8)];
    let mut open = pool(&mut r, &fx, 32, 32);
    open.step(&mut r, &fx, &splash, &[], calm()).unwrap();
    let mut walled = pool(&mut r, &fx, 32, 32);
    walled.step(&mut r, &fx, &splash, &blocks, calm()).unwrap();
    let (open, walled) = (
        open.heights(&mut r).unwrap(),
        walled.heights(&mut r).unwrap(),
    );
    for y in 0..32 {
        for x in 0..32 {
            let want = if blocks.iter().any(|b| b.contains((x, y))) {
                0.0
            } else {
                at(&open, 32, x, y)
            };
            assert_eq!(at(&walled, 32, x, y), want, "at {x},{y}");
        }
    }
}

#[test]
fn damping_settles_the_field() {
    let Some((mut r, fx)) = rig() else { return };
    let mut p = pool(&mut r, &fx, 32, 32);
    p.step(
        &mut r,
        &fx,
        &[Splash::disc((16.5, 16.5), 3.0, -0.8)],
        &[],
        calm(),
    )
    .unwrap();
    for _ in 0..240 {
        p.step(&mut r, &fx, &[], &[], calm()).unwrap();
    }
    let peak = p
        .heights(&mut r)
        .unwrap()
        .iter()
        .fold(0.0f32, |a, h| a.max(h.abs()));
    assert!(peak < 0.02, "4 s of calm leaves {peak}");
    p.flatten(&mut r, &fx).unwrap();
    assert!(p.heights(&mut r).unwrap().iter().all(|&h| h == 0.0));
}

/// The field's RMS height at the end of each of `seconds` seconds after one
/// click, stepped at `hz` with `damping` (per reference step).
fn rms_each_second(
    r: &mut GlesRenderer,
    fx: &WaterFx,
    damping: f32,
    hz: f32,
    seconds: i32,
) -> Vec<f32> {
    let mut p = pool(r, fx, 32, 32);
    let step = Step::new(damping, hz);
    p.step(r, fx, &[Splash::disc((16.5, 16.5), 3.0, -0.8)], &[], step)
        .unwrap();
    let per_second = hz as i32;
    let mut out = Vec::new();
    for i in 1..=seconds * per_second {
        p.step(r, fx, &[], &[], step).unwrap();
        if i % per_second == 0 {
            let h = p.heights(r).unwrap();
            out.push((h.iter().map(|v| v * v).sum::<f32>() / h.len() as f32).sqrt());
        }
    }
    out
}

#[test]
fn damping_settles_the_field_alike_at_240_hertz() {
    let Some((mut r, fx)) = rig() else { return };
    // Calm settles in about 2 s, Lively in about 5.
    for (damping, seconds) in [(0.985, 4), (0.995, 6)] {
        let at60 = rms_each_second(&mut r, &fx, damping, 60.0, seconds);
        let at240 = rms_each_second(&mut r, &fx, damping, 240.0, seconds);
        assert!(
            at240.windows(2).all(|w| w[1] < w[0]),
            "damping {damping}: the field keeps settling at 240 Hz, {at240:?}"
        );
        for (a, b) in at60.iter().zip(&at240) {
            assert!(
                (b - a).abs() <= 0.3 * a + 0.0005,
                "damping {damping}, RMS each second: {at60:?} at 60 Hz, {at240:?} at 240 Hz"
            );
        }
    }
}

#[test]
fn a_resized_pool_starts_flat() {
    let Some((mut r, fx)) = rig() else { return };
    let mut p = pool(&mut r, &fx, 32, 32);
    p.step(
        &mut r,
        &fx,
        &[Splash::disc((16.5, 16.5), 3.0, -0.8)],
        &[],
        still(),
    )
    .unwrap();
    assert!(
        !p.fit(&mut r, &fx, Size::from((32, 32))).unwrap(),
        "same size keeps the field"
    );
    assert!(
        p.fit(&mut r, &fx, Size::from((48, 20))).unwrap(),
        "another size remakes it"
    );
    assert_eq!(p.size(), Some(Size::from((48, 20))));
    assert!(p.heights(&mut r).unwrap().iter().all(|&h| h == 0.0));
}

render_elements! {
    El<=GlesRenderer>;
    Water=WaterElement,
}

/// A `w` x `h` RGB texture, `f` giving each pixel.
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

/// Draw `el` on a `w` x `h` output turned by `t` and read the output
/// back as seen (upright), RGB, row by row.
fn shoot(r: &mut GlesRenderer, el: WaterElement, w: i32, h: i32, t: Transform) -> Vec<[u8; 3]> {
    let mode = t.transform_size(Size::<i32, Physical>::from((w, h)));
    let mut target: GlesTexture = r
        .create_buffer(Fourcc::Abgr8888, (mode.w, mode.h).into())
        .unwrap();
    let mut tracker = OutputDamageTracker::new(mode, 1.0, t);
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
    let px: Vec<[u8; 3]> = r
        .map_texture(&m)
        .unwrap()
        .as_chunks::<4>()
        .0
        .iter()
        .map(|p| [p[0], p[1], p[2]])
        .collect();
    // Back to upright: read the turned framebuffer through the transform.
    (0..h)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .map(|(x, y)| {
            let (mx, my) = match t {
                Transform::Normal => (x, y),
                // Turned a quarter, the target is h x w and upright
                // (x, y) lies at column y, row w - 1 - x.
                Transform::_90 => (y, w - 1 - x),
                _ => unimplemented!("shoot reads Normal and _90"),
            };
            px[(my * mode.w + mx) as usize]
        })
        .collect()
}

fn checker(x: i32, y: i32) -> [u8; 3] {
    if (x / 4 + y / 4) % 2 == 0 {
        [200, 40, 40]
    } else {
        [20, 90, 220]
    }
}

#[test]
fn a_flat_field_draws_the_wallpaper_texel_for_texel() {
    let Some((mut r, fx)) = rig() else { return };
    let wall = picture(&mut r, 64, 48, checker);
    let p = pool(&mut r, &fx, 16, 12);
    let geo = Rectangle::from_size((64, 48).into());
    let el = p
        .element(
            &fx,
            &Id::new(),
            &wall,
            CommitCounter::default(),
            geo,
            12.0,
            0.3,
            1.0,
        )
        .unwrap();
    let got = shoot(&mut r, el, 64, 48, Transform::Normal);
    for y in 0..48 {
        for x in 0..64 {
            assert_eq!(got[(y * 64 + x) as usize], checker(x, y), "at {x},{y}");
        }
    }
}

#[test]
fn a_flat_field_on_a_turned_output_is_the_wallpaper() {
    let Some((mut r, fx)) = rig() else { return };
    let wall = picture(&mut r, 64, 48, checker);
    let p = pool(&mut r, &fx, 16, 12);
    let geo = Rectangle::from_size((64, 48).into());
    let el = p
        .element(
            &fx,
            &Id::new(),
            &wall,
            CommitCounter::default(),
            geo,
            12.0,
            0.3,
            1.0,
        )
        .unwrap();
    let got = shoot(&mut r, el, 64, 48, Transform::_90);
    assert_eq!(got[0], checker(0, 0));
    assert_eq!(got[(47 * 64 + 63) as usize], checker(63, 47));
    assert_eq!(got[(10 * 64 + 5) as usize], checker(5, 10));
    // The checker reads the same turned half way round; a picture that
    // names each pixel does not.
    let name = |x: i32, y: i32| [(x * 4) as u8, (y * 5) as u8, 128];
    let wall = picture(&mut r, 64, 48, name);
    let el = p
        .element(
            &fx,
            &Id::new(),
            &wall,
            CommitCounter::default(),
            geo,
            12.0,
            0.3,
            1.0,
        )
        .unwrap();
    let got = shoot(&mut r, el, 64, 48, Transform::_90);
    for y in 0..48 {
        for x in 0..64 {
            assert_eq!(got[(y * 64 + x) as usize], name(x, y), "at {x},{y}");
        }
    }
}

#[test]
fn a_slope_shifts_the_wallpaper() {
    let Some((mut r, fx)) = rig() else { return };
    // Left half red, right half blue; a ridge along x = 8 (field) bends
    // the lookup so pixels left of the border pick up the other colour.
    let wall = picture(&mut r, 64, 16, |x, _| {
        if x < 32 { [255, 0, 0] } else { [0, 0, 255] }
    });
    let mut p = pool(&mut r, &fx, 16, 4);
    p.step(
        &mut r,
        &fx,
        &[Splash::segment((8.0, -2.0), (8.0, 6.0), 2.0, 0.9)],
        &[],
        still(),
    )
    .unwrap();
    p.shade(&mut r, &fx, 4.0).unwrap();
    let geo = Rectangle::from_size((64, 16).into());
    let flat = {
        let q = pool(&mut r, &fx, 16, 4);
        shoot(
            &mut r,
            q.element(
                &fx,
                &Id::new(),
                &wall,
                CommitCounter::default(),
                geo,
                6.0,
                0.0,
                1.0,
            )
            .unwrap(),
            64,
            16,
            Transform::Normal,
        )
    };
    let bent = shoot(
        &mut r,
        p.element(
            &fx,
            &Id::new(),
            &wall,
            CommitCounter::default(),
            geo,
            6.0,
            0.0,
            1.0,
        )
        .unwrap(),
        64,
        16,
        Transform::Normal,
    );
    let changed = (0..64)
        .filter(|&x| bent[(8 * 64 + x) as usize] != flat[(8 * 64 + x) as usize])
        .count();
    assert!(
        (1..=12).contains(&changed),
        "{changed} pixels moved across the border"
    );
}

#[test]
fn a_splash_near_the_top_bends_only_the_top() {
    let Some((mut r, fx)) = rig() else { return };
    let wall = picture(&mut r, 64, 64, checker);
    let geo = Rectangle::from_size((64, 64).into());
    let flat = {
        let q = pool(&mut r, &fx, 16, 16);
        shoot(
            &mut r,
            q.element(
                &fx,
                &Id::new(),
                &wall,
                CommitCounter::default(),
                geo,
                6.0,
                0.0,
                1.0,
            )
            .unwrap(),
            64,
            64,
            Transform::Normal,
        )
    };
    // A disc 3.5 texels from the field's top edge: output rows 0 to 22.
    let mut p = pool(&mut r, &fx, 16, 16);
    p.step(
        &mut r,
        &fx,
        &[Splash::disc((8.5, 3.5), 2.0, 0.9)],
        &[],
        still(),
    )
    .unwrap();
    p.shade(&mut r, &fx, 4.0).unwrap();
    let bent = shoot(
        &mut r,
        p.element(
            &fx,
            &Id::new(),
            &wall,
            CommitCounter::default(),
            geo,
            6.0,
            0.0,
            1.0,
        )
        .unwrap(),
        64,
        64,
        Transform::Normal,
    );
    let moved = |rows: std::ops::Range<i32>| {
        rows.flat_map(|y| (0..64).map(move |x| (y * 64 + x) as usize))
            .filter(|&i| bent[i] != flat[i])
            .count()
    };
    assert!(moved(0..32) > 0, "the top half bends");
    assert_eq!(moved(32..64), 0, "the bottom half stays");
}

#[test]
fn an_unchanged_field_reports_no_damage() {
    use smithay::backend::renderer::element::Element;
    let Some((mut r, fx)) = rig() else { return };
    let wall = picture(&mut r, 8, 8, |_, _| [0; 3]);
    let mut p = pool(&mut r, &fx, 2, 2);
    let id = Id::new();
    let geo = Rectangle::from_size((8, 8).into());
    let a = p
        .element(
            &fx,
            &id,
            &wall,
            CommitCounter::default(),
            geo,
            1.0,
            0.0,
            1.0,
        )
        .unwrap();
    let b = p
        .element(
            &fx,
            &id,
            &wall,
            CommitCounter::default(),
            geo,
            1.0,
            0.0,
            1.0,
        )
        .unwrap();
    let scale = smithay::utils::Scale::from(1.0);
    assert!(b.damage_since(scale, Some(a.current_commit())).is_empty());
    p.shade(&mut r, &fx, 1.0).unwrap();
    let c = p
        .element(
            &fx,
            &id,
            &wall,
            CommitCounter::default(),
            geo,
            1.0,
            0.0,
            1.0,
        )
        .unwrap();
    assert_eq!(
        c.damage_since(scale, Some(a.current_commit())).len(),
        1,
        "a step redraws all of it"
    );
    let mut wallpaper = CommitCounter::default();
    wallpaper.increment();
    let d = p
        .element(&fx, &id, &wall, wallpaper, geo, 1.0, 0.0, 1.0)
        .unwrap();
    assert_eq!(
        d.damage_since(scale, Some(c.current_commit())).len(),
        1,
        "a new wallpaper redraws all of it"
    );
}

/// The wallpaper drawn through `p` on a 64 x 48 output.
fn through(r: &mut GlesRenderer, fx: &WaterFx, p: &Pool, wall: &GlesTexture) -> Vec<[u8; 3]> {
    let geo = Rectangle::from_size((64, 48).into());
    let el = p
        .element(
            fx,
            &Id::new(),
            wall,
            CommitCounter::default(),
            geo,
            12.0,
            0.3,
            1.0,
        )
        .unwrap();
    shoot(r, el, 64, 48, Transform::Normal)
}

#[test]
fn steps_and_normals_in_one_frame_match_a_frame_each() {
    let Some((mut r, fx)) = rig() else { return };
    let wall = picture(&mut r, 64, 48, checker);
    let obstacles = [Rectangle::new((10, 2).into(), (3, 4).into())];
    let laid = [
        vec![Splash::disc((4.5, 4.5), 2.0, -0.8)],
        vec![Splash::segment((2.0, 9.0), (14.0, 9.0), 1.5, 0.6)],
        vec![],
        vec![Splash::disc((12.5, 8.5), 1.0, 0.5)],
        vec![],
    ];
    for n in [1, 2, 3, 5] {
        let mut apart = pool(&mut r, &fx, 16, 12);
        let mut chained = pool(&mut r, &fx, 16, 12);
        for _ in 0..2 {
            for s in &laid[..n] {
                apart.step(&mut r, &fx, s, &obstacles, calm()).unwrap();
            }
            apart.shade(&mut r, &fx, 4.0).unwrap();
            chained
                .advance(&mut r, &fx, &laid[..n], &obstacles, calm(), 4.0)
                .unwrap();
        }
        assert_eq!(
            chained.heights(&mut r).unwrap(),
            apart.heights(&mut r).unwrap(),
            "{n} steps a frame: the same field"
        );
        assert_eq!(
            through(&mut r, &fx, &chained, &wall),
            through(&mut r, &fx, &apart, &wall),
            "{n} steps a frame: the same normals"
        );
    }
}
