//! Renders glass over a test picture offscreen and writes it as
//! `target/glass_preview.png`, for looking at: a focused Frosted window, an
//! unfocused one, a Smoked one and a taskbar, with the streaks on: the
//! backdrop each lifts for its chrome, without the chrome.
//!
//! `cargo run -p reprisefx --example glass_preview [blur]`; set
//! `LIBGL_ALWAYS_SOFTWARE=1` to render on the CPU.

use reprisefx::{Glass, GlassElement, GlassFrame, GlassFx, Quality, Shape, headless};
use reprisekit::glass::tint::Surface;
use reprisekit::glass::{Material, Settings};
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::element::Id;
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::element::solid::SolidColorRenderElement;
use smithay::backend::renderer::element::texture::{TextureBuffer, TextureRenderElement};
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
use smithay::backend::renderer::utils::CommitCounter;
use smithay::backend::renderer::{Bind, ExportMem, ImportMem, Offscreen};
use smithay::render_elements;
use smithay::utils::{Physical, Rectangle, Transform};

render_elements! {
    El<=GlesRenderer>;
    Glass=GlassElement,
    Solid=SolidColorRenderElement,
    Back=TextureRenderElement<GlesTexture>,
}

const W: i32 = 1024;
const H: i32 = 640;

/// A busy picture, so the blur and the tint both show.
fn picture(x: i32, y: i32) -> [u8; 3] {
    let (fx, fy) = (x as f32, y as f32);
    let band = ((fx + fy * 0.6) / 90.0).floor() as i32;
    let mut c = match band.rem_euclid(5) {
        0 => [40, 90, 170],
        1 => [230, 180, 60],
        2 => [30, 140, 90],
        3 => [200, 70, 80],
        _ => [240, 240, 235],
    };
    let ring = ((fx - 620.0).hypot(fy - 260.0) / 24.0).floor() as i32;
    if ring % 3 == 0 && ring < 12 {
        c = [250, 250, 250];
    }
    if x % 64 == 0 || y % 64 == 0 {
        c = [20, 20, 24];
    }
    c
}

fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Physical> {
    Rectangle::new((x, y).into(), (w, h).into())
}

/// A window's glass: its frame with rounded top corners, client area left out.
struct Window {
    frame: [Shape; 1],
    client: [Rectangle<i32, Physical>; 1],
    surface: Surface,
    material: Material,
    glass: Glass,
}

impl Window {
    fn new(x: i32, y: i32, w: i32, h: i32, surface: Surface, material: Material) -> Window {
        Window {
            frame: [Shape {
                rect: rect(x, y, w, h),
                radii: [8.0, 8.0, 0.0, 0.0],
            }],
            client: [rect(x + 8, y + 30, w - 16, h - 38)],
            surface,
            material,
            glass: Glass::new(),
        }
    }
}

fn main() {
    let blur: u8 = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(2);
    let mut renderer = headless::renderer().expect("an EGL device");
    let fx = GlassFx::new(&mut renderer).expect("the glass shaders compile");

    let mut bytes = Vec::with_capacity((W * H * 4) as usize);
    for y in 0..H {
        for x in 0..W {
            bytes.extend_from_slice(&picture(x, y));
            bytes.push(255);
        }
    }
    let back = renderer
        .import_memory(&bytes, Fourcc::Abgr8888, (W, H).into(), false)
        .expect("the picture uploads");
    let back = TextureBuffer::from_texture(&renderer, back, 1, Transform::Normal, None);

    let windows = [
        Window::new(60, 60, 420, 300, Surface::Inactive, Material::Frosted),
        Window::new(300, 200, 440, 320, Surface::Active, Material::Frosted),
        Window::new(700, 80, 280, 260, Surface::Active, Material::Smoked),
    ];
    let taskbar = [Shape::rect(rect(0, H - 40, W, 40))];
    let taskbar_glass = Glass::new();

    // Front to back: the topmost window's client and glass first.
    let mut els: Vec<El> = Vec::new();
    let add = |els: &mut Vec<El>,
               shapes: &[Shape],
               holes: &[Rectangle<i32, Physical>],
               glass: &Glass,
               surface,
               material| {
        let s = Settings {
            blur,
            material,
            ..Settings::default()
        };
        let f = GlassFrame {
            shapes,
            holes,
            settings: &s,
            surface: Some(surface),
            output: (W, H).into(),
            scale: 1.0,
            alpha: 1.0,
            quality: Quality::Full,
            hold: false,
            source: None,
            downscale: 1,
        };
        els.extend(glass.element(&fx, &f).map(El::Glass));
    };
    add(
        &mut els,
        &taskbar,
        &[],
        &taskbar_glass,
        Surface::Taskbar,
        Material::Frosted,
    );
    for w in windows.iter().rev() {
        els.push(El::Solid(SolidColorRenderElement::new(
            Id::new(),
            w.client[0],
            CommitCounter::default(),
            [0.96, 0.96, 0.96, 1.0],
            Kind::Unspecified,
        )));
        add(
            &mut els, &w.frame, &w.client, &w.glass, w.surface, w.material,
        );
    }
    els.push(El::Back(TextureRenderElement::from_texture_buffer(
        (0.0, 0.0),
        &back,
        None,
        None,
        None,
        Kind::Unspecified,
    )));

    let mut target: GlesTexture = renderer
        .create_buffer(Fourcc::Abgr8888, (W, H).into())
        .expect("an offscreen target");
    let mut tracker = OutputDamageTracker::new((W, H), 1.0, Transform::Normal);
    let mut fb = renderer.bind(&mut target).expect("bind the target");
    tracker
        .render_output(&mut renderer, &mut fb, 0, &els, [0.0, 0.0, 0.0, 1.0])
        .expect("the frame renders");
    let mapping = renderer
        .copy_framebuffer(&fb, Rectangle::from_size((W, H).into()), Fourcc::Abgr8888)
        .expect("read back");
    let rgba = renderer.map_texture(&mapping).expect("map").to_vec();

    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target");
    std::fs::create_dir_all(&dir).expect("target/ exists");
    let path = dir.join("glass_preview.png");
    let file = std::fs::File::create(&path).expect("create the PNG");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), W as u32, H as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()
        .and_then(|mut w| w.write_image_data(&rgba))
        .expect("write the PNG");
    println!("{}", path.canonicalize().unwrap_or(path).display());
}
