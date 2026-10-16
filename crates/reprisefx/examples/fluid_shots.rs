//! Writes `<look>.png`, `-half.png` (level 0.5) and `-bright.png` per fluid look after scripted
//! strokes: `fluid_shots <dir> [seconds] [flood]`, `flood` adding ten circling fingers.

use std::f32::consts::TAU;
use std::path::{Path, PathBuf};

use reprisefx::fluid::{FluidElement, FluidFx, FluidPool};
use reprisefx::headless;
use reprisekit::fluid::{Hands, Profile, Source};
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::element::Id;
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
use smithay::backend::renderer::utils::CommitCounter;
use smithay::backend::renderer::{Bind, ExportMem, ImportMem, Offscreen};
use smithay::render_elements;
use smithay::utils::{Rectangle, Transform};

render_elements! {
    El<=GlesRenderer>;
    Fluid=FluidElement,
}

const W: i32 = 1280;
const H: i32 = 720;
const HZ: f32 = 60.0;

fn wallpaper(r: &mut GlesRenderer, bright: bool) -> GlesTexture {
    let mut bytes = Vec::with_capacity((W * H * 4) as usize);
    for y in 0..H {
        for x in 0..W {
            let (u, v) = (x as f32 / W as f32, y as f32 / H as f32);
            let hill = (v - 0.7 - 0.08 * (u * 9.0).sin()).max(0.0) * 3.0;
            let c = if bright {
                [0.95 - 0.1 * v, 0.93 - 0.05 * v, 0.80 + 0.1 * u]
            } else {
                [0.20 + 0.3 * u, 0.35 + 0.2 * v, 0.55 - 0.2 * v]
            };
            let c = c.map(|k| ((k - hill * 0.3).clamp(0.0, 1.0) * 255.0) as u8);
            bytes.extend_from_slice(&[c[0], c[1], c[2], 255]);
        }
    }
    r.import_memory(&bytes, Fourcc::Abgr8888, (W, H).into(), false)
        .expect("import the wallpaper")
}

/// A finger's point at `t` in 0..1 of its stroke.
type Path2 = fn(f32) -> (f32, f32);

struct Stroke {
    finger: u32,
    from: u32,
    to: u32,
    path: Path2,
}

fn arc(cx: f32, cy: f32, r: f32, a0: f32, a1: f32, t: f32) -> (f32, f32) {
    let a = a0 + (a1 - a0) * t;
    (cx + r * a.cos(), cy + r * a.sin())
}

const STROKES: [Stroke; 6] = [
    Stroke {
        finger: 0,
        from: 0,
        to: 30,
        path: |t| arc(380.0, 520.0, 260.0, -2.4, -1.0, t),
    },
    Stroke {
        finger: 1,
        from: 0,
        to: 30,
        path: |t| arc(900.0, 200.0, 260.0, 0.7, 2.1, t),
    },
    Stroke {
        finger: 2,
        from: 70,
        to: 95,
        path: |t| (180.0 + 120.0 * t, 120.0 + 200.0 * t),
    },
    Stroke {
        finger: 3,
        from: 70,
        to: 95,
        path: |t| (1100.0 - 150.0 * t, 640.0 - 160.0 * t),
    },
    Stroke {
        finger: 4,
        from: 140,
        to: 170,
        path: |t| arc(640.0, 360.0, 90.0, 0.0, TAU * 0.4, t),
    },
    Stroke {
        finger: 5,
        from: 140,
        to: 170,
        path: |t| arc(640.0, 360.0, 90.0, TAU * 0.5, TAU * 0.9, t),
    },
];

fn ms(frame: u32) -> u32 {
    (frame as f32 * 1000.0 / HZ) as u32
}

/// Steps a fresh pool through the strokes and `seconds` of flow.
fn stir(
    r: &mut GlesRenderer,
    fx: &FluidFx,
    profile: Profile,
    flavour: usize,
    seconds: f32,
    flood: bool,
) -> FluidPool {
    let mut pool = FluidPool::new();
    pool.fit(r, fx, (W, H).into()).expect("fit the pool");
    let mut hands = Hands::new(profile, flavour);
    let frames = (seconds * HZ) as u32;
    for frame in 0..frames {
        let t = ms(frame);
        for s in &STROKES {
            let at = |f: u32| (s.path)((f - s.from) as f32 / (s.to - s.from) as f32);
            if frame == s.from {
                hands.down(Source::Finger(s.finger), at(frame), t);
            } else if frame > s.from && frame <= s.to {
                hands.moved(Source::Finger(s.finger), at(frame), t);
                if frame == s.to {
                    hands.up(Source::Finger(s.finger), t);
                }
            }
        }
        for k in (10..20).filter(|_| flood) {
            let a = TAU * (t as f32 / 2000.0 + k as f32 / 10.0);
            let r = 60.0 + 25.0 * (k - 10) as f32;
            let at = (640.0 + r * a.cos(), 360.0 + 0.8 * r * a.sin());
            match frame {
                0 => hands.down(Source::Finger(k), at, t),
                _ => hands.moved(Source::Finger(k), at, t),
            }
        }
        if frame == 200 {
            hands.down(Source::Finger(9), (980.0, 560.0), t);
        }
        if frame == 205 {
            hands.up(Source::Finger(9), t);
        }
        let forces = hands.frame(t);
        pool.step(r, fx, &forces, 1.0, profile.look(), 1.0 / HZ)
            .expect("step");
    }
    pool
}

fn shoot(r: &mut GlesRenderer, el: FluidElement) -> Vec<u8> {
    let mut target: GlesTexture = r
        .create_buffer(Fourcc::Abgr8888, (W, H).into())
        .expect("a target");
    let mut tracker = OutputDamageTracker::new((W, H), 1.0, Transform::Normal);
    let mut fb = r.bind(&mut target).expect("bind the target");
    tracker
        .render_output(r, &mut fb, 0, &[El::from(el)], [0.0, 0.0, 0.0, 1.0])
        .expect("the frame renders");
    let m = r
        .copy_framebuffer(&fb, Rectangle::from_size((W, H).into()), Fourcc::Abgr8888)
        .expect("read back");
    r.map_texture(&m).expect("map").to_vec()
}

fn write(path: &Path, rgba: &[u8]) {
    let file = std::fs::File::create(path).expect("create the PNG");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), W as u32, H as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()
        .and_then(|mut w| w.write_image_data(rgba))
        .expect("write the PNG");
    println!("{}", path.display());
}

fn main() {
    let mut args = std::env::args().skip(1);
    let dir = PathBuf::from(args.next().expect("usage: fluid_shots <dir> [seconds]"));
    let seconds: f32 = args.next().map_or(5.0, |s| s.parse().expect("seconds"));
    let flood = args.next().is_some_and(|a| a == "flood");
    std::fs::create_dir_all(&dir).expect("the output directory");
    let mut r = headless::renderer().expect("a headless renderer");
    let fx = FluidFx::new(&mut r).expect("fluid on this device");
    let walls = [wallpaper(&mut r, false), wallpaper(&mut r, true)];
    let shots = [
        (Profile::Ink, 0, "ink-light"),
        (Profile::Ink, 1, "ink-dark"),
        (Profile::Glow, 0, "glow"),
        (Profile::Sparks, 0, "sparks"),
        (Profile::Neon, 0, "neon"),
    ];
    for (profile, flavour, name) in shots {
        let mut pool = stir(&mut r, &fx, profile, flavour, seconds, flood);
        pool.shade(&mut r, &fx, profile, flavour, seconds)
            .expect("shade");
        for (wall, level, suffix) in [(0, 1.0, ""), (0, 0.5, "-half"), (1, 1.0, "-bright")] {
            let geo = Rectangle::from_size((W, H).into());
            let el = pool
                .element(
                    &fx,
                    &Id::new(),
                    &walls[wall],
                    CommitCounter::default(),
                    geo,
                    profile.look(),
                    flavour,
                    level,
                )
                .expect("an element");
            let rgba = shoot(&mut r, el);
            write(&dir.join(format!("{name}{suffix}.png")), &rgba);
        }
    }
}
