//! Glass drawn offscreen on a real GLES renderer, read back and checked
//! by pixel. Runs on the first hardware EGL device, or Mesa's software
//! one with `LIBGL_ALWAYS_SOFTWARE=1`; with no EGL device at all each
//! test says so and passes.

use reprisefx::{
    FlipCard, FlipCardElement, FlipFx, Glass, GlassElement, GlassFrame, GlassFx, Picture, Quality,
    Shape, Source, Update, blur, headless,
};
use reprisekit::flip::{self, Homography};
use reprisekit::glass::tint::{self, Surface};
use reprisekit::glass::{Material, Settings};
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::element::texture::{TextureBuffer, TextureRenderElement};
use smithay::backend::renderer::element::utils::{
    CropRenderElement, Relocate, RelocateRenderElement,
};
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture, ffi};
use smithay::backend::renderer::{Bind, ExportMem, ImportMem, Offscreen};
use smithay::render_elements;
use smithay::utils::{Physical, Point, Rectangle, Size, Transform};

render_elements! {
    El<=GlesRenderer>;
    Glass=GlassElement,
    Back=TextureRenderElement<GlesTexture>,
    Moved=RelocateRenderElement<GlassElement>,
    MovedBack=RelocateRenderElement<TextureRenderElement<GlesTexture>>,
    Card=FlipCardElement,
    Cropped=CropRenderElement<GlassElement>,
}

struct Rig {
    renderer: GlesRenderer,
    fx: GlassFx,
    target: GlesTexture,
    tracker: OutputDamageTracker,
    size: Size<i32, Physical>,
    transform: Transform,
    /// Frames rendered into `target`: after the first, the tracker draws
    /// only damage.
    frames: usize,
}

fn rig(w: i32, h: i32) -> Option<Rig> {
    rig_turned(w, h, Transform::Normal)
}

fn rig_turned(w: i32, h: i32, transform: Transform) -> Option<Rig> {
    let mut renderer = match headless::renderer() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("skipped: {e}");
            return None;
        }
    };
    let device = renderer
        .with_context(|gl| {
            // SAFETY: the context is current; GL_RENDERER is a static string.
            let name = unsafe { gl.GetString(ffi::RENDERER) };
            if name.is_null() {
                String::new()
            } else {
                // SAFETY: GL returns a NUL-terminated string it owns.
                unsafe { std::ffi::CStr::from_ptr(name.cast()) }
                    .to_string_lossy()
                    .into_owned()
            }
        })
        .unwrap_or_default();
    eprintln!("renderer: {device}");
    let fx = GlassFx::new(&mut renderer).expect("the glass shaders compile");
    let mode = transform.transform_size(Size::<i32, Physical>::from((w, h)));
    let target = renderer
        .create_buffer(Fourcc::Abgr8888, (mode.w, mode.h).into())
        .expect("an offscreen target");
    Some(Rig {
        renderer,
        fx,
        target,
        tracker: OutputDamageTracker::new(mode, 1.0, transform),
        size: (w, h).into(),
        transform,
        frames: 0,
    })
}

struct Shot {
    w: i32,
    px: Vec<[u8; 3]>,
}

impl Shot {
    fn at(&self, x: i32, y: i32) -> [u8; 3] {
        self.px[(y * self.w + x) as usize]
    }
}

impl Rig {
    fn backdrop(&mut self, f: impl Fn(i32, i32) -> [u8; 3]) -> TextureBuffer<GlesTexture> {
        self.backdrop_of(self.size, f)
    }

    fn backdrop_of(
        &mut self,
        size: Size<i32, Physical>,
        f: impl Fn(i32, i32) -> [u8; 3],
    ) -> TextureBuffer<GlesTexture> {
        let (w, h) = (size.w, size.h);
        let mut bytes = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let [r, g, b] = f(x, y);
                bytes.extend_from_slice(&[r, g, b, 255]);
            }
        }
        let t = self
            .renderer
            .import_memory(&bytes, Fourcc::Abgr8888, (w, h).into(), false)
            .expect("the backdrop uploads");
        TextureBuffer::from_texture(&self.renderer, t, 1, Transform::Normal, None)
    }

    fn render(
        &mut self,
        glass: Vec<GlassElement>,
        back: &TextureBuffer<GlesTexture>,
    ) -> (Shot, bool) {
        let mut els: Vec<El> = glass.into_iter().map(El::Glass).collect();
        els.push(El::Back(back_element(back)));
        self.render_elements(&els)
    }

    fn render_elements(&mut self, els: &[El]) -> (Shot, bool) {
        let age = self.frames.min(1);
        self.frames += 1;
        let size = self.transform.transform_size(self.size);
        let mut fb = self
            .renderer
            .bind(&mut self.target)
            .expect("bind the target");
        let drew = self
            .tracker
            .render_output(&mut self.renderer, &mut fb, age, els, [0.0, 0.0, 0.0, 1.0])
            .expect("the frame renders")
            .damage
            .is_some_and(|d| !d.is_empty());
        let mapping = self
            .renderer
            .copy_framebuffer(
                &fb,
                Rectangle::from_size((size.w, size.h).into()),
                Fourcc::Abgr8888,
            )
            .expect("read back");
        let bytes = self.renderer.map_texture(&mapping).expect("map").to_vec();
        let px = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| [p[0], p[1], p[2]])
            .collect();
        (Shot { w: size.w, px }, drew)
    }

    fn frame<'a>(
        &self,
        shapes: &'a [Shape],
        holes: &'a [Rectangle<i32, Physical>],
        s: &'a Settings,
    ) -> GlassFrame<'a> {
        GlassFrame {
            shapes,
            holes,
            settings: s,
            surface: Some(Surface::Active),
            output: self.size,
            scale: 1.0,
            alpha: 1.0,
            quality: Quality::Full,
            hold: false,
            source: None,
            downscale: 1,
        }
    }
}

fn back_element(back: &TextureBuffer<GlesTexture>) -> TextureRenderElement<GlesTexture> {
    TextureRenderElement::from_texture_buffer((0.0, 0.0), back, None, None, None, Kind::Unspecified)
}

fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Physical> {
    Rectangle::new((x, y).into(), (w, h).into())
}

/// Default glass with the streaks off, so a flat backdrop stays flat.
fn plain(material: Material) -> Settings {
    Settings {
        material,
        streaks: false,
        ..Settings::default()
    }
}

/// `tint.rs`'s formula over white: blur·255 + colour·C + afterglow·C
/// (white's luma is 1), at `strength` over the backdrop.
fn formula_over_white(s: &Settings, strength: f64) -> [f64; 3] {
    let (c, [cb, ab, bb]) = tint::material_glass(s);
    c.map(|c| 255.0 + strength * (bb * 255.0 + cb * c + ab * c - 255.0))
}

fn near(got: [u8; 3], want: [f64; 3], within: f64) -> bool {
    got.iter()
        .zip(want)
        .all(|(&g, w)| (f64::from(g) - w).abs() <= within)
}

#[test]
fn frosted_sky_over_white_is_the_tint_formula() {
    let Some(mut r) = rig(200, 120) else { return };
    let white = r.backdrop(|_, _| [255; 3]);
    let s = plain(Material::Frosted);
    let glass = Glass::new();
    let shapes = [Shape::rect(rect(40, 30, 120, 60))];
    let el = glass.element(&r.fx, &r.frame(&shapes, &[], &s)).unwrap();
    let (p, _) = r.render(vec![el], &white);
    let want = formula_over_white(&s, 1.0);
    for (x, y) in [(100, 60), (41, 31), (158, 88)] {
        assert!(
            near(p.at(x, y), want, 2.0),
            "({x}, {y}) is {want:?}: {:?}",
            p.at(x, y)
        );
    }
    assert_eq!(
        p.at(20, 10),
        [255; 3],
        "outside the glass the backdrop shows"
    );
}

#[test]
fn smoked_over_white_is_darker_and_an_unfocused_window_keeps_it_dark() {
    let Some(mut r) = rig(200, 120) else { return };
    let white = r.backdrop(|_, _| [255; 3]);
    let s = plain(Material::Smoked);
    let (focused, unfocused) = (Glass::new(), Glass::new());
    let a = [Shape::rect(rect(10, 10, 80, 60))];
    let b = [Shape::rect(rect(110, 10, 80, 60))];
    let mut fb = r.frame(&b, &[], &s);
    fb.surface = Some(Surface::Inactive);
    let els = vec![
        focused.element(&r.fx, &r.frame(&a, &[], &s)).unwrap(),
        unfocused.element(&r.fx, &fb).unwrap(),
    ];
    let (p, _) = r.render(els, &white);
    let (on, off) = (p.at(50, 40), p.at(150, 40));
    assert!(
        near(on, formula_over_white(&s, 1.0), 2.0),
        "focused: {on:?}"
    );
    assert!(
        near(off, formula_over_white(&s, tint::SMOKE_INACTIVE), 2.0),
        "unfocused, at SMOKE_INACTIVE: {off:?}"
    );
    // The blended art's plain glass over white (tint.rs's own tests): the
    // shader's exact formula lands within a few levels of it.
    assert!(
        near(on, [100.0, 109.0, 114.0], 6.0),
        "focused near the art's: {on:?}"
    );
    assert!(
        near(off, [127.0, 136.0, 143.0], 9.0),
        "unfocused near the art's: {off:?}"
    );
    let frosted = formula_over_white(&plain(Material::Frosted), 1.0);
    assert!(
        on.iter()
            .zip(frosted)
            .all(|(&s, f)| f64::from(s) + 60.0 < f),
        "{on:?} is darker than Frosted's {frosted:?}"
    );
}

/// The blurred row through a black-to-white edge at `x = w/2`, with no
/// tint: the distance between where it passes 15.9% and 84.1% of white,
/// which is two sigmas of the blur on screen.
fn edge_spread(r: &mut Rig, blur: u8, quality: Quality) -> f64 {
    let (w, h) = (r.size.w, r.size.h);
    let back = r.backdrop(|x, _| if x < w / 2 { [0; 3] } else { [255; 3] });
    let s = Settings {
        blur,
        ..plain(Material::Frosted)
    };
    let glass = Glass::new();
    let shapes = [Shape::rect(rect(0, 0, w, h))];
    let mut f = r.frame(&shapes, &[], &s);
    f.surface = None;
    f.quality = quality;
    let el = glass.element(&r.fx, &f).unwrap();
    r.frames = 0;
    let (p, _) = r.render(vec![el], &back);
    let row: Vec<f64> = (0..w).map(|x| f64::from(p.at(x, h / 2)[0])).collect();
    let cross = |level: f64| {
        let i = row
            .iter()
            .position(|&v| v >= level)
            .expect("the row reaches the level");
        let (a, b) = (row[i - 1], row[i]);
        (i - 1) as f64 + (level - a) / (b - a)
    };
    cross(0.841 * 255.0) - cross(0.159 * 255.0)
}

#[test]
fn each_strength_spreads_an_edge_by_its_sigma() {
    let Some(mut r) = rig(400, 40) else { return };
    for blur in 1..=5u8 {
        let sigma = f64::from(blur::sigma(blur, Material::Frosted));
        // On screen: the Gaussian at twice its half-resolution sigma, with
        // the 2x2 mean and the linear upscale adding about one pixel².
        let want = 2.0 * (4.0 * sigma * sigma + 1.0).sqrt();
        let got = edge_spread(&mut r, blur, Quality::Full);
        assert!(
            (got - want).abs() <= 0.08 * want,
            "blur {blur}: the edge spreads over {want:.1} px, got {got:.1}"
        );
        let coarse = edge_spread(&mut r, blur, Quality::Coarse);
        assert!(
            (coarse - want).abs() <= 0.15 * want,
            "blur {blur}, coarse: about {want:.1} px, got {coarse:.1}"
        );
    }
}

#[test]
fn a_bright_spot_spreads_no_farther_than_the_reach() {
    let Some(mut r) = rig(240, 240) else { return };
    let back = r.backdrop(|x, y| {
        if (112..128).contains(&x) && (112..128).contains(&y) {
            [255; 3]
        } else {
            [0; 3]
        }
    });
    for blur in 1..=5u8 {
        let s = Settings {
            blur,
            ..plain(Material::Frosted)
        };
        let glass = Glass::new();
        let shapes = [Shape::rect(rect(0, 0, 240, 240))];
        let mut f = r.frame(&shapes, &[], &s);
        f.surface = None;
        let el = glass.element(&r.fx, &f).unwrap();
        r.frames = 0;
        let (p, _) = r.render(vec![el], &back);
        let reach = blur::reach(blur, Material::Frosted, Quality::Full);
        let sigma = 2.0 * blur::sigma(blur, Material::Frosted);
        let lit = |d: i32| p.at(128 + d, 120)[0];
        assert!(
            lit(sigma.round() as i32) > 0,
            "blur {blur}: light {sigma} px past the spot"
        );
        assert_eq!(lit(reach), 0, "blur {blur}: none {reach} px past it");
        assert!(lit(0) < 255, "blur {blur}: the spot is spread");
    }
}

#[test]
fn holes_and_the_margin_show_the_backdrop_untouched() {
    let Some(mut r) = rig(200, 160) else { return };
    let pattern = |x: i32, y: i32| {
        [
            (x * 5 % 256) as u8,
            (y * 3 % 256) as u8,
            ((x ^ y) * 7 % 256) as u8,
        ]
    };
    let back = r.backdrop(pattern);
    let s = Settings::default();
    let glass = Glass::new();
    let shapes = [Shape::rect(rect(40, 30, 120, 100))];
    let holes = [rect(50, 60, 100, 60)];
    let el = glass.element(&r.fx, &r.frame(&shapes, &holes, &s)).unwrap();
    let (p, _) = r.render(vec![el], &back);
    for (x, y) in [(50, 60), (100, 90), (149, 119), (60, 70)] {
        assert_eq!(p.at(x, y), pattern(x, y), "({x}, {y}) is in the hole");
    }
    for (x, y) in [(39, 30), (30, 100), (170, 140), (100, 131)] {
        assert_eq!(p.at(x, y), pattern(x, y), "({x}, {y}) is past the glass");
    }
    assert_ne!(
        p.at(45, 45),
        pattern(45, 45),
        "the frame around the hole is glass"
    );
    assert_ne!(p.at(100, 125), pattern(100, 125), "below the hole is glass");
}

#[test]
fn a_rounded_corner_leaves_its_outside_clear() {
    let Some(mut r) = rig(160, 120) else { return };
    let white = r.backdrop(|_, _| [255; 3]);
    let s = plain(Material::Smoked);
    let glass = Glass::new();
    let shapes = [Shape {
        rect: rect(20, 20, 100, 60),
        radii: [12.0, 0.0, 6.0, 0.0],
    }];
    let el = glass.element(&r.fx, &r.frame(&shapes, &[], &s)).unwrap();
    let (p, _) = r.render(vec![el], &white);
    let glass_px = p.at(70, 50);
    assert!(glass_px[0] < 140, "the middle is glass: {glass_px:?}");
    assert_eq!(p.at(20, 20), [255; 3], "the top-left corner is cut");
    assert_eq!(p.at(22, 22), [255; 3], "and in from it");
    assert_eq!(p.at(23, 20), [255; 3], "along the top edge");
    assert_eq!(p.at(26, 26), glass_px, "inside the arc is glass");
    assert_eq!(p.at(119, 20), glass_px, "the top-right corner is square");
    assert_eq!(p.at(20, 79), glass_px, "the bottom-left corner is square");
    assert_eq!(p.at(119, 79), [255; 3], "the bottom-right corner is cut");
    let edge = p.at(20 + 3, 20 + 3);
    assert!(
        edge[0] > glass_px[0] && edge[0] < 255,
        "a pixel the arc crosses is part glass: {edge:?}"
    );
}

fn half_backdrop(r: &mut Rig, f: impl Fn(i32, i32) -> [u8; 3]) -> GlesTexture {
    let (w, h) = ((r.size.w + 1) / 2, (r.size.h + 1) / 2);
    let bytes: Vec<u8> = (0..h)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .flat_map(|(x, y)| {
            let [r, g, b] = f(x, y);
            [r, g, b, 255]
        })
        .collect();
    r.renderer
        .import_memory(&bytes, Fourcc::Abgr8888, (w, h).into(), false)
        .expect("the backdrop uploads")
}

fn blur_into(
    r: &mut Rig,
    picture: &mut Picture,
    backdrop: &GlesTexture,
    changed: &[Rectangle<i32, Physical>],
    s: &Settings,
    quality: Quality,
    hold: bool,
) -> Source {
    let u = Update {
        backdrop,
        changed,
        settings: s,
        quality,
        needed: None,
        output: r.size,
        scale: 1.0,
        hold,
    };
    picture
        .update(&mut r.renderer, &r.fx, &u)
        .expect("the picture blurs");
    picture.source().expect("a drawn picture")
}

#[test]
fn a_source_is_drawn_in_place_of_the_frame_and_only_its_changes_redraw() {
    let Some(mut r) = rig(320, 200) else { return };
    // The frame under the glass is black (chrome); the backdrop is white.
    let black = r.backdrop(|_, _| [0; 3]);
    let s = plain(Material::Frosted);
    let glass = Glass::new();
    let shapes = [Shape::rect(rect(20, 20, 240, 160))];
    let mut picture = Picture::new();
    let white = half_backdrop(&mut r, |_, _| [255; 3]);
    let src = blur_into(&mut r, &mut picture, &white, &[], &s, Quality::Full, false);
    let f = GlassFrame {
        source: Some(&src),
        ..r.frame(&shapes, &[], &s)
    };
    let (before, _) = r.render(vec![glass.element(&r.fx, &f).unwrap()], &black);
    assert!(
        near(before.at(160, 100), formula_over_white(&s, 1.0), 2.0),
        "the glass shows the source: {:?}",
        before.at(160, 100)
    );
    assert_eq!(before.at(10, 10), [0; 3], "the frame shows outside it");

    let (_, drew) = r.render(vec![glass.element(&r.fx, &f).unwrap()], &black);
    assert!(!drew, "nothing changed: nothing is drawn");

    // A change past the glass's reach leaves it.
    let src = blur_into(
        &mut r,
        &mut picture,
        &white,
        &[rect(310, 0, 10, 10)],
        &s,
        Quality::Full,
        false,
    );
    let f = GlassFrame {
        source: Some(&src),
        ..r.frame(&shapes, &[], &s)
    };
    let (_, drew) = r.render(vec![glass.element(&r.fx, &f).unwrap()], &black);
    assert!(!drew, "a change past the reach is not the glass's");

    // A grey square under one corner redraws the glass near it alone.
    let square = |x: i32, y: i32| (20..28).contains(&x) && (20..28).contains(&y);
    let grey = half_backdrop(
        &mut r,
        |x, y| if square(x, y) { [128; 3] } else { [255; 3] },
    );
    let src = blur_into(
        &mut r,
        &mut picture,
        &grey,
        &[rect(40, 40, 16, 16)],
        &s,
        Quality::Full,
        false,
    );
    let f = GlassFrame {
        source: Some(&src),
        ..r.frame(&shapes, &[], &s)
    };
    let (p, drew) = r.render(vec![glass.element(&r.fx, &f).unwrap()], &black);
    assert!(drew, "a change within the reach redraws the glass");
    assert!(
        p.at(48, 48)[0] < before.at(48, 48)[0],
        "over the square: {:?}",
        p.at(48, 48)
    );
    assert_eq!(
        p.at(250, 150),
        before.at(250, 150),
        "far from it the glass is as it was"
    );
}

#[test]
fn glass_over_a_source_cut_to_a_crop_draws_there_what_it_draws_whole() {
    let Some(mut r) = rig(320, 200) else { return };
    let black = r.backdrop(|_, _| [0; 3]);
    let s = plain(Material::Molten);
    let mut picture = Picture::new();
    let stripes = half_backdrop(&mut r, |x, y| {
        [(x * 9 % 256) as u8, (y * 5 % 256) as u8, 90]
    });
    let src = blur_into(
        &mut r,
        &mut picture,
        &stripes,
        &[],
        &s,
        Quality::Full,
        false,
    );
    let shapes = [Shape {
        rect: rect(20, 20, 240, 160),
        radii: [12.0; 4],
    }];
    let holes = [rect(40, 50, 200, 110)];
    let f = GlassFrame {
        source: Some(&src),
        ..r.frame(&shapes, &holes, &s)
    };
    let whole = Glass::new().element(&r.fx, &f).unwrap();
    let (want, _) = r.render(vec![whole], &black);
    let crop = rect(13, 31, 150, 90);
    let glass = Glass::new().element(&r.fx, &f).unwrap();
    let cut = CropRenderElement::from_element(glass, 1.0, crop).unwrap();
    r.frames = 0;
    let (got, _) = r.render_elements(&[El::Cropped(cut), El::Back(back_element(&black))]);
    for y in crop.loc.y..crop.loc.y + crop.size.h {
        for x in crop.loc.x..crop.loc.x + crop.size.w {
            assert_eq!(got.at(x, y), want.at(x, y), "at ({x}, {y})");
        }
    }
    assert_eq!(got.at(200, 30), [0; 3], "past the crop the frame shows");
}

/// Blurring the changed parts of a picture gives what blurring all of it
/// does, near a change and far from it, at either quality.
#[test]
fn a_picture_redrawn_where_it_changed_is_the_picture_drawn_whole() {
    let Some(mut r) = rig(240, 160) else { return };
    let s = Settings {
        blur: 3,
        ..Settings::default()
    };
    let all = [Shape::rect(rect(0, 0, 240, 160))];
    let corner = [Shape::rect(rect(0, 0, 70, 50))];
    let black = r.backdrop(|_, _| [0; 3]);
    // One change near the corner's glass, one far from it.
    let changed = [rect(90, 50, 30, 24), rect(196, 120, 20, 20)];
    let after = |x: i32, y: i32| {
        if (45..60).contains(&x) && (25..37).contains(&y) {
            [250, 20, 90]
        } else if (98..108).contains(&x) && (60..70).contains(&y) {
            [10, 240, 30]
        } else {
            detail(x, y)
        }
    };
    for quality in [Quality::Full, Quality::Coarse] {
        let output = r.size;
        let frame = |shapes, src| GlassFrame {
            shapes,
            holes: &[],
            settings: &s,
            surface: Some(Surface::Active),
            output,
            scale: 1.0,
            alpha: 1.0,
            quality,
            hold: false,
            source: Some(src),
            downscale: 1,
        };
        let mut parts = Picture::new();
        let first = half_backdrop(&mut r, detail);
        let src = blur_into(&mut r, &mut parts, &first, &[], &s, quality, false);
        r.frames = 0;
        let el = Glass::new().element(&r.fx, &frame(&all, &src)).unwrap();
        r.render(vec![el], &black);
        let second = half_backdrop(&mut r, after);
        let src = blur_into(&mut r, &mut parts, &second, &changed, &s, quality, false);
        r.frames = 0;
        let el = Glass::new().element(&r.fx, &frame(&corner, &src)).unwrap();
        let (got_corner, _) = r.render(vec![el], &black);
        r.frames = 0;
        let el = Glass::new().element(&r.fx, &frame(&all, &src)).unwrap();
        let (got, _) = r.render(vec![el], &black);

        let mut whole = Picture::new();
        let src = blur_into(&mut r, &mut whole, &second, &[], &s, quality, false);
        r.frames = 0;
        let el = Glass::new().element(&r.fx, &frame(&corner, &src)).unwrap();
        let (want_corner, _) = r.render(vec![el], &black);
        r.frames = 0;
        let el = Glass::new().element(&r.fx, &frame(&all, &src)).unwrap();
        let (want, _) = r.render(vec![el], &black);
        for (what, got, want) in [("corner", &got_corner, &want_corner), ("all", &got, &want)] {
            let (d, x, y) = worst(got, want);
            assert_eq!(
                d,
                0,
                "{quality:?}, {what}: ({x}, {y}) is {:?}, {:?} drawn whole",
                got.at(x, y),
                want.at(x, y)
            );
        }
    }
}

/// A picture that serves part of the output (glass over another window's
/// frame) blurs only the changes that reach that part, keeps the others,
/// and blurs them once a later update needs where they are: it then draws
/// what a picture blurred whole draws, there and where it was needed.
#[test]
fn a_picture_blurs_a_change_once_it_is_needed() {
    let Some(mut r) = rig(240, 160) else { return };
    let s = Settings::default();
    let black = r.backdrop(|_, _| [0; 3]);
    let left = [rect(0, 0, 100, 160)];
    let right = [rect(140, 0, 100, 160)];
    let glass_left = [Shape::rect(rect(10, 10, 80, 140))];
    let glass_right = [Shape::rect(rect(150, 10, 80, 140))];
    // A square on the right, far from the left's reach.
    let after = |x: i32, y: i32| {
        if (90..105).contains(&x) && (30..50).contains(&y) {
            [250, 20, 90]
        } else {
            detail(x, y)
        }
    };
    let changed = [rect(180, 60, 30, 40)];
    let first = half_backdrop(&mut r, detail);
    let second = half_backdrop(&mut r, after);
    let shot = |r: &mut Rig, shapes: &[Shape], src: &Source| {
        let f = GlassFrame {
            source: Some(src),
            ..r.frame(shapes, &[], &s)
        };
        r.frames = 0;
        let el = Glass::new().element(&r.fx, &f).unwrap();
        r.render(vec![el], &black).0
    };
    let update = |r: &mut Rig, p: &mut Picture, back, changed, needed| {
        let u = Update {
            backdrop: back,
            changed,
            settings: &s,
            quality: Quality::Full,
            output: r.size,
            scale: 1.0,
            hold: false,
            needed,
        };
        p.update(&mut r.renderer, &r.fx, &u)
            .expect("the picture blurs");
        p.source().expect("a drawn picture")
    };
    let mut part = Picture::new();
    update(&mut r, &mut part, &first, &[], Some(&left[..]));
    let src = update(&mut r, &mut part, &second, &changed[..], Some(&left[..]));
    assert!(
        !part.is_stale(),
        "the change is not needed: nothing is stale"
    );
    let left_before = shot(&mut r, &glass_left, &src);
    let src = update(&mut r, &mut part, &second, &[], Some(&right[..]));
    let got = shot(&mut r, &glass_right, &src);

    let mut whole = Picture::new();
    let src = update(&mut r, &mut whole, &second, &[], None);
    let want = shot(&mut r, &glass_right, &src);
    let want_left = shot(&mut r, &glass_left, &src);
    let (d, x, y) = worst(&got, &want);
    assert_eq!(
        d,
        0,
        "once needed: ({x}, {y}) is {:?}, {:?} blurred whole",
        got.at(x, y),
        want.at(x, y)
    );
    let (d, x, y) = worst(&left_before, &want_left);
    assert_eq!(d, 0, "where it was needed: ({x}, {y})");
}

/// Glass over a source draws what glass over a frame holding the same
/// backdrop does: a frame of 2x2 blocks downscales to the source's
/// texels exactly.
#[test]
fn glass_over_a_source_is_glass_over_the_same_frame() {
    let Some(mut r) = rig(240, 160) else { return };
    let s = Settings::default();
    let shapes = [Shape {
        rect: rect(40, 30, 140, 90),
        radii: [14.0, 0.0, 8.0, 4.0],
    }];
    let holes = [rect(56, 54, 100, 50)];
    let frame = r.backdrop(|x, y| detail(x / 2, y / 2));
    let f = GlassFrame {
        surface: Some(Surface::Active),
        ..r.frame(&shapes, &holes, &s)
    };
    r.frames = 0;
    let (want, _) = r.render(vec![Glass::new().element(&r.fx, &f).unwrap()], &frame);
    let mut picture = Picture::new();
    let back = half_backdrop(&mut r, detail);
    let src = blur_into(&mut r, &mut picture, &back, &[], &s, Quality::Full, false);
    let f = GlassFrame {
        source: Some(&src),
        ..f
    };
    r.frames = 0;
    let (got, _) = r.render(vec![Glass::new().element(&r.fx, &f).unwrap()], &frame);
    let (d, x, y) = worst(&got, &want);
    assert!(
        d <= 1,
        "({x}, {y}) is {:?} over the source, {:?} over the frame",
        got.at(x, y),
        want.at(x, y)
    );
}

#[test]
fn a_held_picture_keeps_its_blur_until_a_refresh() {
    let Some(mut r) = rig(160, 120) else { return };
    let black = r.backdrop(|_, _| [0; 3]);
    let s = plain(Material::Frosted);
    let glass = Glass::new();
    let shapes = [Shape::rect(rect(30, 30, 100, 60))];
    let mut picture = Picture::new();
    let grey = half_backdrop(&mut r, |_, _| [128; 3]);
    let src = blur_into(&mut r, &mut picture, &grey, &[], &s, Quality::Full, true);
    let f = GlassFrame {
        source: Some(&src),
        ..r.frame(&shapes, &[], &s)
    };
    let (first, _) = r.render(vec![glass.element(&r.fx, &f).unwrap()], &black);
    assert!(!picture.is_stale(), "the first update draws, held or not");

    let white = half_backdrop(&mut r, |_, _| [255; 3]);
    let all = [rect(0, 0, 160, 120)];
    let src = blur_into(&mut r, &mut picture, &white, &all, &s, Quality::Full, true);
    let f = GlassFrame {
        source: Some(&src),
        ..r.frame(&shapes, &[], &s)
    };
    let (p, drew) = r.render(vec![glass.element(&r.fx, &f).unwrap()], &black);
    assert!(!drew, "a held picture redraws no glass");
    assert_eq!(
        p.at(80, 60),
        first.at(80, 60),
        "a held picture keeps its blur"
    );
    assert!(picture.is_stale(), "and says so");

    picture.refresh();
    let src = blur_into(&mut r, &mut picture, &white, &[], &s, Quality::Full, true);
    let f = GlassFrame {
        source: Some(&src),
        ..r.frame(&shapes, &[], &s)
    };
    let (p, _) = r.render(vec![glass.element(&r.fx, &f).unwrap()], &black);
    assert!(
        near(p.at(80, 60), formula_over_white(&s, 1.0), 2.0),
        "a refresh catches up: {:?}",
        p.at(80, 60)
    );
    assert!(!picture.is_stale());
}

#[test]
fn a_held_blur_waits_for_a_refresh() {
    let Some(mut r) = rig(160, 120) else { return };
    let grey = r.backdrop(|_, _| [128; 3]);
    let s = plain(Material::Frosted);
    let glass = Glass::new();
    let shapes = [Shape::rect(rect(30, 30, 100, 60))];
    let f = r.frame(&shapes, &[], &s);
    let (first, _) = r.render(vec![glass.element(&r.fx, &f).unwrap()], &grey);
    let (_, drew) = r.render(vec![glass.element(&r.fx, &f).unwrap()], &grey);
    assert!(!drew, "nothing changed: nothing is drawn");

    let white = r.backdrop(|_, _| [255; 3]);
    let held = GlassFrame { hold: true, ..f };
    let (p, _) = r.render(vec![glass.element(&r.fx, &held).unwrap()], &white);
    assert_eq!(
        p.at(80, 60),
        first.at(80, 60),
        "a held glass keeps its blur"
    );
    assert_eq!(
        p.at(10, 10),
        [255; 3],
        "while the backdrop changes around it"
    );
    assert!(glass.is_stale(), "and says so");

    glass.refresh();
    let (p, _) = r.render(vec![glass.element(&r.fx, &held).unwrap()], &white);
    assert!(
        near(p.at(80, 60), formula_over_white(&s, 1.0), 2.0),
        "a refresh catches up: {:?}",
        p.at(80, 60)
    );
    assert!(!glass.is_stale());

    let moved = [Shape::rect(rect(32, 30, 100, 60))];
    let (p, _) = r.render(
        vec![
            glass
                .element(
                    &r.fx,
                    &GlassFrame {
                        shapes: &moved,
                        ..held
                    },
                )
                .unwrap(),
        ],
        &grey,
    );
    assert_eq!(
        p.at(80, 60),
        first.at(80, 60),
        "a move blurs again, held or not"
    );
}

#[test]
fn only_frosted_glass_shows_the_streaks() {
    let Some(mut r) = rig(640, 360) else { return };
    let black = r.backdrop(|_, _| [0; 3]);
    let mut lit = |material, streaks| {
        let s = Settings {
            material,
            streaks,
            ..Settings::default()
        };
        let glass = Glass::new();
        let shapes = [Shape::rect(rect(0, 0, 640, 360))];
        let mut f = r.frame(&shapes, &[], &s);
        f.surface = None;
        r.frames = 0;
        let (p, _) = r.render(vec![glass.element(&r.fx, &f).unwrap()], &black);
        p.px.iter()
            .map(|c| u64::from(c[0]) + u64::from(c[1]) + u64::from(c[2]))
            .sum::<u64>()
    };
    assert!(
        lit(Material::Frosted, true) > lit(Material::Frosted, false),
        "Frosted shows streaks over black"
    );
    for material in [Material::Smoked, Material::Molten] {
        assert_eq!(
            lit(material, true),
            lit(material, false),
            "{material:?} shows no streaks"
        );
    }
}

/// The rig's output as seen, from `els` rendered into its turned target:
/// a first frame with a backdrop that names each pixel (red = x, green =
/// y, through the same projection) says where each target pixel lies.
fn seen(r: &mut Rig, els: &[El]) -> Shot {
    let size = r.size;
    assert!(size.w <= 256 && size.h <= 256, "names fit a byte");
    let names = r.backdrop_of(size, |x, y| [x as u8, y as u8, 0]);
    r.frames = 0;
    let (where_, _) = r.render_elements(&[El::Back(back_element(&names))]);
    r.frames = 0;
    let (turned, _) = r.render_elements(els);
    let mut px = vec![[0u8; 3]; (size.w * size.h) as usize];
    let mut filled = vec![false; px.len()];
    for (name, p) in where_.px.iter().zip(&turned.px) {
        let i = (i32::from(name[1]) * size.w + i32::from(name[0])) as usize;
        px[i] = *p;
        filled[i] = true;
    }
    assert!(filled.iter().all(|f| *f), "every pixel is named once");
    Shot { w: size.w, px }
}

fn detail(x: i32, y: i32) -> [u8; 3] {
    [
        ((x * 7 + y * 3) % 256) as u8,
        (if (x / 8 + y / 12) % 2 == 0 { 40 } else { 220 }),
        ((x * y) % 251) as u8,
    ]
}

fn worst(a: &Shot, b: &Shot) -> (u8, i32, i32) {
    let mut out = (0, 0, 0);
    for (i, (p, q)) in a.px.iter().zip(&b.px).enumerate() {
        let d = p
            .iter()
            .zip(q)
            .map(|(p, q)| p.abs_diff(*q))
            .max()
            .unwrap_or(0);
        if d > out.0 {
            out = (d, i as i32 % a.w, i as i32 / a.w);
        }
    }
    out
}

/// Glass with a rounded corner of each size, a hole and the streaks, on
/// an output of `size`, tinted for an active frame or (None) the
/// backdrop alone.
fn turned_glass(
    fx: &GlassFx,
    glass: &Glass,
    size: Size<i32, Physical>,
    s: &Settings,
    surface: Option<Surface>,
) -> GlassElement {
    let shapes = [Shape {
        rect: rect(40, 30, 140, 90),
        radii: [14.0, 0.0, 8.0, 4.0],
    }];
    let holes = [rect(56, 54, 100, 50)];
    glass
        .element(
            fx,
            &GlassFrame {
                shapes: &shapes,
                holes: &holes,
                settings: s,
                surface,
                output: size,
                scale: 1.0,
                alpha: 1.0,
                quality: Quality::Full,
                hold: false,
                source: None,
                downscale: 1,
            },
        )
        .unwrap()
}

type Scene = (fn(i32, i32) -> [u8; 3], Option<Surface>);

#[test]
fn a_turned_output_shows_the_glass_as_an_upright_one_does() {
    let (w, h) = (240, 160);
    let s = Settings::default();
    // Over detail, tinted; and over black, the streaks alone.
    let scenes: [Scene; 2] = [(detail, Some(Surface::Active)), (|_, _| [0; 3], None)];
    let Some(mut upright) = rig(w, h) else { return };
    let mut want = Vec::new();
    for (paint, surface) in scenes {
        let back = upright.backdrop(paint);
        let glass = Glass::new();
        let el = turned_glass(&upright.fx, &glass, upright.size, &s, surface);
        want.push(seen(
            &mut upright,
            &[El::Glass(el), El::Back(back_element(&back))],
        ));
    }
    assert_ne!(want[0].at(110, 40), detail(110, 40), "the glass blurs");
    assert_eq!(
        want[0].at(40, 30),
        detail(40, 30),
        "its top-left corner is cut"
    );
    assert_eq!(
        want[0].at(100, 80),
        detail(100, 80),
        "the hole shows the backdrop"
    );
    let lit = want[1].px.iter().filter(|p| p[0] > 8).count();
    assert!(lit > 1000, "the streaks show over black: {lit} px");
    for t in [
        Transform::_90,
        Transform::_180,
        Transform::_270,
        Transform::Flipped,
        Transform::Flipped90,
        Transform::Flipped180,
        Transform::Flipped270,
    ] {
        let Some(mut r) = rig_turned(w, h, t) else {
            return;
        };
        for ((paint, surface), want) in scenes.into_iter().zip(&want) {
            let back = r.backdrop(paint);
            let glass = Glass::new();
            let el = turned_glass(&r.fx, &glass, r.size, &s, surface);
            let got = seen(&mut r, &[El::Glass(el), El::Back(back_element(&back))]);
            let (d, x, y) = worst(&got, want);
            assert!(
                d <= 3,
                "{t:?}, {surface:?}: ({x}, {y}) is {:?} on the turned output, {:?} upright",
                got.at(x, y),
                want.at(x, y)
            );
        }
    }
}

#[test]
fn a_region_capture_shows_the_glass_in_place() {
    let (w, h) = (240, 160);
    let s = Settings {
        blur: 1,
        material: Material::Frosted,
        ..Settings::default()
    };
    // The region holds the glass and its reach, so it reads what the
    // whole output does.
    let origin: Point<i32, Physical> = (12, 8).into();
    let region: Size<i32, Physical> = (200, 136).into();
    for t in [Transform::Normal, Transform::_90] {
        let Some(mut whole) = rig_turned(w, h, t) else {
            return;
        };
        let back = whole.backdrop(detail);
        let glass = Glass::new();
        let el = turned_glass(&whole.fx, &glass, whole.size, &s, Some(Surface::Active));
        let full = seen(&mut whole, &[El::Glass(el), El::Back(back_element(&back))]);

        let Some(mut part) = rig_turned(region.w, region.h, t) else {
            return;
        };
        let back = part.backdrop_of((w, h).into(), detail);
        let glass = Glass::new();
        let el = turned_glass(&part.fx, &glass, (w, h).into(), &s, Some(Surface::Active));
        let shift = Point::default() - origin;
        let els = [
            El::Moved(RelocateRenderElement::from_element(
                el,
                shift,
                Relocate::Relative,
            )),
            El::MovedBack(RelocateRenderElement::from_element(
                back_element(&back),
                shift,
                Relocate::Relative,
            )),
        ];
        let got = seen(&mut part, &els);
        let cropped = Shot {
            w: region.w,
            px: (0..region.h)
                .flat_map(|y| (0..region.w).map(move |x| (x, y)))
                .map(|(x, y)| full.at(x + origin.x, y + origin.y))
                .collect(),
        };
        let (d, x, y) = worst(&got, &cropped);
        assert!(
            d <= 1,
            "{t:?}: ({x}, {y}) of the region is {:?}, {:?} on the whole output",
            got.at(x, y),
            cropped.at(x, y)
        );
    }
}

/// Molten glass over `back` at `shapes`, with no tint (the host's art
/// tints), and the same under Frosted.
fn molten_and_frosted(
    r: &mut Rig,
    back: &smithay::backend::renderer::element::texture::TextureBuffer<GlesTexture>,
    shapes: &[Shape],
) -> (Shot, Shot) {
    let mut out = Vec::new();
    for material in [Material::Molten, Material::Frosted] {
        let s = plain(material);
        let glass = Glass::new();
        let mut f = r.frame(shapes, &[], &s);
        f.surface = None;
        r.frames = 0;
        out.push(r.render(vec![glass.element(&r.fx, &f).unwrap()], back).0);
    }
    let frosted = out.pop().unwrap();
    (out.pop().unwrap(), frosted)
}

#[test]
fn molten_bends_what_lies_past_its_edge_into_it() {
    let Some(mut r) = rig(200, 120) else { return };
    // Dark above the glass, white from 4 px above its top edge down.
    let back = r.backdrop(|_, y| if y < 26 { [0; 3] } else { [255; 3] });
    let shapes = [Shape::rect(rect(40, 30, 120, 60))];
    let (molten, frosted) = molten_and_frosted(&mut r, &back, &shapes);
    let (m, f) = (molten.at(100, 34)[0], frosted.at(100, 34)[0]);
    assert!(
        u16::from(m) + 100 < u16::from(f),
        "4 px in, Molten shows the dark band from 20 px out: {m} against {f}"
    );
    assert_eq!(
        molten.at(100, 60),
        [255; 3],
        "past the band the backdrop lies where it is"
    );
}

#[test]
fn molten_lights_a_rim_except_on_the_output_border() {
    let Some(mut r) = rig(200, 120) else { return };
    let grey = r.backdrop(|_, _| [100; 3]);
    // A bar along the output's bottom: its top edge bends, the others lie
    // on the border.
    let shapes = [Shape::rect(rect(0, 80, 200, 40))];
    let (molten, frosted) = molten_and_frosted(&mut r, &grey, &shapes);
    let middle = molten.at(100, 100)[0];
    assert_eq!(
        frosted.at(100, 80),
        frosted.at(100, 100),
        "Frosted has none"
    );
    assert!(
        molten.at(100, 80)[0] > middle + 60,
        "the top edge's rim: {:?} over {middle}",
        molten.at(100, 80)
    );
    for (x, y) in [(100, 119), (0, 100), (199, 100)] {
        assert!(
            molten.at(x, y)[0].abs_diff(middle) <= 1,
            "({x}, {y}) on the border has no rim: {:?}",
            molten.at(x, y)
        );
    }
}

#[test]
fn molten_lights_its_edges_from_up_and_to_the_left() {
    let Some(mut r) = rig(200, 120) else { return };
    let grey = r.backdrop(|_, _| [100; 3]);
    let shapes = [Shape::rect(rect(40, 30, 120, 60))];
    let (molten, _) = molten_and_frosted(&mut r, &grey, &shapes);
    let [top, bottom, left, right] =
        [(100, 30), (100, 89), (40, 60), (159, 60)].map(|(x, y)| molten.at(x, y)[0]);
    assert!(
        top > bottom,
        "the top edge faces the light: {top} over {bottom}"
    );
    assert!(
        bottom > left,
        "the bottom bounces it back: {bottom} over {left}"
    );
    assert!(
        left > right,
        "the left edge leans toward it: {left} over {right}"
    );
}

#[test]
fn molten_splits_colours_where_it_bends_and_saturates_the_rest() {
    let Some(mut r) = rig(200, 120) else { return };
    // Dark above, white from 4 px above the glass's top edge down: 13 px
    // in, green reads the line, red from past it, blue from short of it.
    let back = r.backdrop(|_, y| if y < 26 { [0; 3] } else { [255; 3] });
    let shapes = [Shape::rect(rect(40, 30, 120, 60))];
    let (molten, _) = molten_and_frosted(&mut r, &back, &shapes);
    let [red, _, blue] = molten.at(100, 43);
    assert!(
        blue > red + 4,
        "blue over red 13 px in: {:?}",
        molten.at(100, 43)
    );
    let pink = r.backdrop(|_, _| [180, 120, 120]);
    let (molten, frosted) = molten_and_frosted(&mut r, &pink, &shapes);
    let spread = |c: [u8; 3]| c[0] - c[1];
    assert!(
        spread(molten.at(100, 60)) > spread(frosted.at(100, 60)) + 15,
        "Molten's middle is richer: {:?} against {:?}",
        molten.at(100, 60),
        frosted.at(100, 60)
    );
}

fn quadrants(r: &mut Rig) -> GlesTexture {
    let mut bytes = Vec::with_capacity(64 * 48 * 4);
    for y in 0..48 {
        for x in 0..64 {
            let c = match (x < 32, y < 24) {
                (true, true) => [255, 0, 0],
                (false, true) => [0, 255, 0],
                (true, false) => [0, 0, 255],
                (false, false) => [255, 255, 255],
            };
            bytes.extend_from_slice(&c);
            bytes.push(255);
        }
    }
    r.renderer
        .import_memory(&bytes, Fourcc::Abgr8888, (64, 48).into(), false)
        .expect("the picture uploads")
}

#[test]
fn an_unturned_card_shows_its_picture_scaled_into_place() {
    let Some(mut r) = rig(200, 120) else { return };
    let fx = FlipFx::new(&mut r.renderer).expect("the card shader compiles");
    let tex = quadrants(&mut r);
    // Twice the size, top-left at (20, 10): 128x96.
    let m = Homography([[2.0, 0.0, 20.0], [0.0, 2.0, 10.0], [0.0, 0.0, 1.0]]);
    let card = FlipCard::new();
    let el = card.element(&fx, &tex, m, 1.0, false).unwrap();
    let (p, _) = r.render_elements(&[El::Card(el)]);
    for (x, y, want) in [
        (21, 11, [255, 0, 0]),
        (146, 11, [0, 255, 0]),
        (21, 104, [0, 0, 255]),
        (146, 104, [255, 255, 255]),
    ] {
        assert_eq!(p.at(x, y), want, "({x}, {y})");
    }
    for (x, y) in [(19, 50), (149, 50), (80, 9), (80, 107)] {
        assert_eq!(p.at(x, y), [0, 0, 0], "({x}, {y}) is off the card");
    }
}

#[test]
fn a_turned_card_lands_where_its_map_puts_each_quadrant() {
    let Some(mut r) = rig(320, 240) else { return };
    let fx = FlipFx::new(&mut r.renderer).expect("the card shader compiles");
    let tex = quadrants(&mut r);
    let area = reprisekit::raster::Rect::new(0, 0, 320, 240);
    let cards = flip::layout(area, &[(64, 48)], 0.0, &flip::GEOMETRY);
    let m = cards[0].to_screen;
    let card = FlipCard::new();
    let el = card.element(&fx, &tex, m, 1.0, false).unwrap();
    let (p, _) = r.render_elements(&[El::Card(el)]);
    for (u, v, want) in [
        (16.0, 12.0, [255, 0, 0]),
        (48.0, 12.0, [0, 255, 0]),
        (16.0, 36.0, [0, 0, 255]),
        (48.0, 36.0, [255, 255, 255]),
    ] {
        let (x, y) = m.map(u, v).unwrap();
        let got = p.at(x as i32, y as i32);
        assert_eq!(got, want, "picture point ({u}, {v}) at ({x:.1}, {y:.1})");
    }
    let b = cards[0].bounds;
    assert_eq!(
        p.at(b.x - 2, b.y - 2),
        [0, 0, 0],
        "outside the bounds is clear"
    );
    // The right edge is shorter: above its top corner, inside the bounds,
    // lies nothing.
    let (x, y) = m.map(64.0, 0.0).unwrap();
    assert_eq!(
        p.at(x as i32 - 1, y as i32 - 2),
        [0, 0, 0],
        "the turned edge leaves a gap"
    );
}

#[test]
fn a_half_faded_card_is_half_as_bright() {
    let Some(mut r) = rig(100, 80) else { return };
    let fx = FlipFx::new(&mut r.renderer).expect("the card shader compiles");
    let tex = quadrants(&mut r);
    let m = Homography([[1.0, 0.0, 10.0], [0.0, 1.0, 10.0], [0.0, 0.0, 1.0]]);
    let card = FlipCard::new();
    let el = card.element(&fx, &tex, m, 0.5, false).unwrap();
    let (p, _) = r.render_elements(&[El::Card(el)]);
    let [red, ..] = p.at(20, 20);
    assert!((126..=129).contains(&red), "half red over black: {red}");
}

#[test]
fn a_card_fades_its_pictures_glass_toward_clear_and_keeps_the_rest() {
    let Some(mut r) = rig(40, 20) else { return };
    let fx = FlipFx::new(&mut r.renderer).expect("the card shader compiles");
    // Left half glass (white at alpha 0.4, premultiplied), right half
    // opaque white.
    let mut bytes = Vec::with_capacity(32 * 16 * 4);
    for _ in 0..16 {
        for x in 0..32 {
            let px = if x < 16 { [102; 4] } else { [255; 4] };
            bytes.extend_from_slice(&px);
        }
    }
    let tex = r
        .renderer
        .import_memory(&bytes, Fourcc::Abgr8888, (32, 16).into(), false)
        .expect("the picture uploads");
    let m = Homography([[1.0, 0.0, 4.0], [0.0, 1.0, 2.0], [0.0, 0.0, 1.0]]);
    let card = FlipCard::new();
    let el = card.element(&fx, &tex, m, 1.0, false).unwrap();
    let (p, _) = r.render_elements(&[El::Card(el)]);
    // 102 at alpha 0.4 keeps 0.25 + 0.75 * 0.4 of itself: 56.
    let [glass, ..] = p.at(12, 10);
    assert!((55..=57).contains(&glass), "the glass over black: {glass}");
    assert_eq!(p.at(28, 10), [255, 255, 255], "opaque white stays");
}

#[test]
fn a_card_draws_a_dark_one_pixel_outline_all_round_at_any_size() {
    let Some(mut r) = rig(80, 40) else { return };
    let fx = FlipFx::new(&mut r.renderer).expect("the card shader compiles");
    let bytes = vec![255u8; 64 * 32 * 4];
    let tex = r
        .renderer
        .import_memory(&bytes, Fourcc::Abgr8888, (64, 32).into(), false)
        .expect("the picture uploads");
    // Opaque white, at full size from (4, 2) and at half size from
    // (12, 6): either way its outermost pixel is half black.
    for (s, x0, y0) in [(1.0, 4, 2), (0.5, 12, 6)] {
        let m = Homography([
            [s, 0.0, f64::from(x0)],
            [0.0, s, f64::from(y0)],
            [0.0, 0.0, 1.0],
        ]);
        let card = FlipCard::new();
        let el = card.element(&fx, &tex, m, 1.0, false).unwrap();
        let (p, _) = r.render_elements(&[El::Card(el)]);
        let (x1, y1) = (x0 + (64.0 * s) as i32 - 1, y0 + (32.0 * s) as i32 - 1);
        let mid = (y0 + y1) / 2;
        for (x, y) in [
            (x0, mid),
            (x1, mid),
            ((x0 + x1) / 2, y0),
            ((x0 + x1) / 2, y1),
            (x0, y0),
        ] {
            let [v, ..] = p.at(x, y);
            assert!(
                (126..=129).contains(&v),
                "scale {s}: outline at ({x}, {y}) is {v}"
            );
        }
        for (x, y) in [(x0 + 1, mid), (x1 - 1, mid), ((x0 + x1) / 2, y0 + 1)] {
            assert_eq!(
                p.at(x, y),
                [255; 3],
                "scale {s}: ({x}, {y}) inside the outline"
            );
        }
    }
}

#[test]
fn a_fine_checker_drawn_at_a_quarter_size_is_an_even_grey() {
    let Some(mut r) = rig(64, 48) else { return };
    let fx = FlipFx::new(&mut r.renderer).expect("the card shader compiles");
    if !fx.trilinear() {
        eprintln!("skipped: no GLES 3, cards sample without mipmaps");
        return;
    }
    let mut bytes = Vec::with_capacity(128 * 128 * 4);
    for y in 0..128 {
        for x in 0..128 {
            let v = if (x + y) % 2 == 0 { 255 } else { 0 };
            bytes.extend_from_slice(&[v, v, v, 255]);
        }
    }
    let tex = r
        .renderer
        .import_memory(&bytes, Fourcc::Abgr8888, (128, 128).into(), false)
        .expect("the picture uploads");
    reprisefx::mipmaps(&mut r.renderer, &tex).expect("the mipmaps build");
    // A quarter size, 32x32, off the pixel grid.
    let m = Homography([[0.25, 0.0, 8.3], [0.0, 0.25, 6.3], [0.0, 0.0, 1.0]]);
    let card = FlipCard::new();
    let el = card.element(&fx, &tex, m, 1.0, true).unwrap();
    let (p, _) = r.render_elements(&[El::Card(el)]);
    for y in 9..37 {
        for x in 11..39 {
            let [v, ..] = p.at(x, y);
            assert!((112..=143).contains(&v), "({x}, {y}) is {v}, not grey");
        }
    }
}

/// Glass shrunk into a half-resolution target (a backdrop beneath other
/// glass) draws, at each target pixel, the mean of the 2x2 output pixels
/// the glass draws there on the output, for every material; past its
/// shapes and in its holes it draws nothing.
#[test]
fn glass_shrunk_to_half_resolution_is_the_glass_on_the_output_downscaled() {
    let Some(mut r) = rig(240, 160) else { return };
    let ramp = |x: i32, y: i32| [(x * 2) as u8, (y * 3) as u8, (x + y) as u8];
    let shapes = [Shape {
        rect: rect(41, 31, 141, 91),
        radii: [14.0, 0.0, 8.0, 4.0],
    }];
    let holes = [rect(57, 55, 100, 51)];
    let black = r.backdrop(|_, _| [0; 3]);
    let half: Size<i32, Physical> = (120, 80).into();
    let half_black = r.backdrop_of(half, |_, _| [0; 3]);
    let half_target: GlesTexture = r
        .renderer
        .create_buffer(Fourcc::Abgr8888, (half.w, half.h).into())
        .expect("a half-resolution target");
    for (material, within) in [
        (Material::Frosted, 2),
        (Material::Smoked, 2),
        (Material::Molten, 6),
    ] {
        let s = plain(material);
        let mut picture = Picture::new();
        let back = half_backdrop(&mut r, ramp);
        let src = blur_into(&mut r, &mut picture, &back, &[], &s, Quality::Full, false);
        let f = GlassFrame {
            source: Some(&src),
            ..r.frame(&shapes, &holes, &s)
        };
        r.frames = 0;
        let (full, _) = r.render(vec![Glass::new().element(&r.fx, &f).unwrap()], &black);

        let shrunk = GlassFrame { downscale: 2, ..f };
        let el = Glass::new().element(&r.fx, &shrunk).unwrap();
        let output = std::mem::replace(&mut r.size, half);
        let target = std::mem::replace(&mut r.target, half_target.clone());
        let tracker = std::mem::replace(
            &mut r.tracker,
            OutputDamageTracker::new(half, 1.0, Transform::Normal),
        );
        r.frames = 0;
        let (got, _) = r.render(vec![el], &half_black);
        (r.size, r.target, r.tracker) = (output, target, tracker);

        // Clear of the edge's anti-aliasing and Molten's rim.
        let inset = |r: Rectangle<i32, Physical>, by: i32| {
            Rectangle::new(
                (r.loc.x + by, r.loc.y + by).into(),
                (r.size.w - 2 * by, r.size.h - 2 * by).into(),
            )
        };
        let grow = |r: Rectangle<i32, Physical>, by: i32| inset(r, -by);
        let inner = inset(shapes[0].rect, 16);
        for y in 0..half.h {
            for x in 0..half.w {
                let block = rect(2 * x, 2 * y, 2, 2);
                let at = got.at(x, y);
                if !grow(shapes[0].rect, 1).overlaps(block)
                    || inset(holes[0], 1).contains_rect(block)
                {
                    assert_eq!(at, [0; 3], "{material:?}: nothing at ({x}, {y})");
                } else if inner.contains_rect(block) && !grow(holes[0], 1).overlaps(block) {
                    let mean = [0, 1, 2].map(|c| {
                        let sum: u32 = [(0, 0), (1, 0), (0, 1), (1, 1)]
                            .iter()
                            .map(|(dx, dy)| u32::from(full.at(2 * x + dx, 2 * y + dy)[c]))
                            .sum();
                        f64::from(sum) / 4.0
                    });
                    assert!(
                        near(at, mean, f64::from(within)),
                        "{material:?}: ({x}, {y}) is {at:?}, the output's mean {mean:?}"
                    );
                }
            }
        }
    }
}
