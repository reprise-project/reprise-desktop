use crate::icon::premultiply;
use crate::raster::Canvas;
use resvg::tiny_skia::{Rect, Transform};
use resvg::usvg::{self, Group, ImageKind, Node, Paint, Size, roxmltree};
use std::borrow::Cow;
use std::path::Path;
use std::sync::{Mutex, OnceLock, PoisonError};

/// Largest icon side accepted from a file or peer; bounds decode buffers at 4 MiB.
pub const MAX_ICON_DIMENSION: u32 = 1024;

/// RGBA8 straight-alpha to a premultiplied canvas; None if `rgba` is shorter than `w * h` pixels.
pub fn canvas_from_rgba(w: u32, h: u32, rgba: &[u8]) -> Option<Canvas> {
    if rgba.len() < w as usize * h as usize * 4 {
        return None;
    }
    let mut c = Canvas::new(w, h);
    for (i, px) in rgba
        .as_chunks::<4>()
        .0
        .iter()
        .take((w * h) as usize)
        .enumerate()
    {
        let straight =
            (px[3] as u32) << 24 | (px[0] as u32) << 16 | (px[1] as u32) << 8 | px[2] as u32;
        c.pixels[i] = premultiply(straight);
    }
    Some(c)
}

/// Decode a PNG of any colour type into a premultiplied canvas, refusing a side past `max_side`
/// before sizing the buffer.
pub fn png_to_canvas(bytes: &[u8], max_side: u32) -> Result<Canvas, String> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder
        .read_info()
        .map_err(|e| format!("bad png header: {e}"))?;
    // Checked before the buffer is sized, so a lying header costs nothing.
    let (hw, hh) = (reader.info().width, reader.info().height);
    if hw == 0 || hh == 0 || hw > max_side || hh > max_side {
        return Err(format!("refusing a {hw}x{hh} png"));
    }
    let size = reader
        .output_buffer_size()
        .ok_or("png output buffer overflows")?;
    let mut buf = vec![0u8; size];
    let info = reader
        .next_frame(&mut buf)
        .map_err(|e| format!("png data: {e}"))?;
    let (w, h) = (info.width, info.height);
    let rgba = png_rgba(&buf[..info.buffer_size()], info.color_type)
        .ok_or("unsupported png color type")?;
    canvas_from_rgba(w, h, &rgba).ok_or_else(|| "png data shorter than its header".into())
}

/// 8-bit PNG pixels of `color_type`, after `EXPAND`, as RGBA: RGBA passes through, other layouts
/// are widened into a copy. None for indexed, which `EXPAND` has already widened.
pub(crate) fn png_rgba(buf: &[u8], color_type: png::ColorType) -> Option<Cow<'_, [u8]>> {
    use png::ColorType as C;
    Some(match color_type {
        C::Rgba => Cow::Borrowed(buf),
        C::Rgb => buf
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect::<Vec<u8>>()
            .into(),
        C::GrayscaleAlpha => buf
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect::<Vec<u8>>()
            .into(),
        C::Grayscale => buf
            .iter()
            .flat_map(|&g| [g, g, g, 255])
            .collect::<Vec<u8>>()
            .into(),
        C::Indexed => return None,
    })
}

fn decode_png(path: &Path, bytes: &[u8], px: u32) -> Option<Canvas> {
    let c = match png_to_canvas(bytes, MAX_ICON_DIMENSION) {
        Ok(c) => c,
        Err(e) => {
            log::warn!("icon {}: {e}", path.display());
            return None;
        }
    };
    let (w, h) = (c.width, c.height);
    Some(if (w, h) == (px, px) {
        c
    } else if w > px || h > px {
        shrink_to_ink(&c, px)
    } else {
        c.resample_box(px, px)
    })
}

/// Alpha below which a pixel is not ink: a faint halo must not defeat the crop.
const INK_ALPHA: u32 = 16;

/// Scale a bigger bitmap to `px` square around its ink bounding box (aspect kept, centred), so
/// a padded 128 px icon is not smaller than its neighbours.
fn shrink_to_ink(c: &Canvas, px: u32) -> Canvas {
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
    for y in 0..c.height {
        for x in 0..c.width {
            if c.get(x as i32, y as i32) >> 24 >= INK_ALPHA {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    if x0 >= x1 || y0 >= y1 {
        return c.resample_box(px, px);
    }
    let (bw, bh) = (x1 - x0, y1 - y0);
    let mut ink = Canvas::new(bw, bh);
    for y in 0..bh {
        for x in 0..bw {
            ink.pixels[(y * bw + x) as usize] = c.get((x0 + x) as i32, (y0 + y) as i32);
        }
    }
    // Longer side to `px`, the other in proportion (at least 1).
    let (tw, th) = if bw >= bh {
        (px, ((bh * px + bw / 2) / bw).max(1))
    } else {
        (((bw * px + bh / 2) / bh).max(1), px)
    };
    let scaled = ink.resample_box(tw, th);
    let mut out = Canvas::new(px, px);
    out.blit_over(((px - tw) / 2) as i32, ((px - th) / 2) as i32, &scaled);
    out
}

/// Cost limits for an SVG icon: usvg and resvg have none, and a file a sandboxed app ships can
/// ask for seconds of work in a few hundred bytes.
pub(super) const SVG_MAX_BYTES: usize = 1 << 20;
/// Work usvg may do to build an SVG's tree and resvg to draw it at its output size, together, in
/// about nanoseconds of one core's CPU time; the draw gets what the build left. The weights
/// below are resvg 0.49's.
pub(super) const SVG_MAX_WORK: f64 = 78e6;
/// Work to parse and convert one element, each `<use>` copy and each reference converted again
/// counted once more, and one path vertex.
const WORK_ELEMENT: f64 = 4_000.0;
const WORK_VERTEX: f64 = 300.0;
/// Work to draw one node, besides the pixels it covers.
const WORK_NODE: f64 = 6_000.0;
/// Work per pixel: a solid fill, a gradient fill plus each of its stops, and a pattern or
/// picture drawn bicubic.
const WORK_FILL: f64 = 3.0;
const WORK_GRADIENT: f64 = 18.0;
const WORK_STOP: f64 = 1.0;
const WORK_BICUBIC: f64 = 90.0;
/// Work per offscreen pixmap, besides its pixels: an isolated group's layer, and the pixmaps a
/// clip path and a mask over it each allocate and fill.
const WORK_LAYER_PASS: f64 = 2_000.0;
const WORK_CLIP_PASS: f64 = 8_000.0;
const WORK_MASK_PASS: f64 = 16_000.0;
/// Work per pixel of a layer (allocated, cleared, drawn back), of a clip path or mask over it
/// (two pixmaps or more allocated, filled and applied), and of a filter on it (its region
/// copied and drawn back).
const WORK_LAYER: f64 = 18.0;
const WORK_CLIP: f64 = 21.0;
const WORK_MASK: f64 = 34.0;
const WORK_FILTER: f64 = 25.0;
/// Work per pixel of the filter region for each primitive, by kind. A filter interpolating
/// linearRGB converts its result back, and each primitive reading the source converts it.
const WORK_LINEAR_RGB: f64 = 10.0;
const WORK_LINEAR_RGB_INPUT: f64 = 15.0;
/// A Gaussian blur's passes along rows and along columns. resvg blurs a standard deviation from
/// 2 px with box passes, a smaller one with IIR passes in f64 (converted from and back to
/// bytes), whose column pass misses the cache on rows a multiple of 256 px (2 KiB) long.
const WORK_BOX_PASS: f64 = 100.0;
const WORK_IIR: f64 = 18.0;
const WORK_IIR_ROWS: f64 = 270.0;
const WORK_IIR_COLUMNS: f64 = 270.0;
const WORK_IIR_COLUMNS_ALIASED: f64 = 660.0;
/// A drop shadow's flood, conversion and two draws, besides its blur.
const WORK_SHADOW: f64 = 70.0;
const WORK_OFFSET: f64 = 15.0;
/// A flood, and a picture's pixmap, besides what the picture draws.
const WORK_FILL_REGION: f64 = 2.0;
/// Composite, blend and displacement map: two inputs into one.
const WORK_TWO_INPUTS: f64 = 30.0;
const WORK_MERGE_INPUT: f64 = 13.0;
const WORK_COLOR_MATRIX: f64 = 35.0;
const WORK_COMPONENT_TRANSFER: f64 = 115.0;
const WORK_TILE: f64 = 18.0;
/// Morphology and convolution, besides each pixel they read per pixel.
const WORK_MORPHOLOGY: f64 = 15.0;
const WORK_MORPHOLOGY_TAP: f64 = 1.2;
const WORK_CONVOLVE: f64 = 50.0;
const WORK_CONVOLVE_TAP: f64 = 15.0;
/// Turbulence's lattice set up once, then work per pixel and per octave.
const WORK_TURBULENCE_INIT: f64 = 50_000.0;
const WORK_TURBULENCE: f64 = 115.0;
const WORK_OCTAVE: f64 = 190.0;
/// Lighting from a distant light, and from a point or spot one.
const WORK_DISTANT_LIGHT: f64 = 55.0;
const WORK_NEAR_LIGHT: f64 = 165.0;
/// Work per pixel row a path edge spans, and per pair of edges the rasterizer sorts.
const WORK_EDGE_ROW: f64 = 50.0;
const WORK_EDGE_PAIR: f64 = 0.35;
/// Bytes of every pixmap resvg allocates to draw an SVG, summed: a filter keeps each
/// primitive's result alive, and the work budget alone allows about 150 MiB of cheap ones.
const SVG_MAX_PIXMAP_BYTES: f64 = 64.0 * 1024.0 * 1024.0;
/// Bytes per pixel: a pixmap, a clip path's (and its alpha mask), a mask's (and two alpha masks),
/// a primitive's result and input copy, an IIR blur's f64 buffer, a drop shadow's extra pixmap.
const BYTES_PIXMAP: f64 = 4.0;
const BYTES_CLIP: f64 = 5.0;
const BYTES_MASK: f64 = 6.0;
const BYTES_PRIMITIVE: f64 = 8.0;
const BYTES_IIR: f64 = 8.0;
const BYTES_SHADOW: f64 = 4.0;
/// Embedded PNG, JPEG and SVG pictures drawn, each use of one counted again.
pub(super) const SVG_MAX_IMAGES: usize = 4;
/// Pixels of every embedded PNG and JPEG drawn, summed; each is decoded again per use.
pub(super) const SVG_MAX_PICTURE_PIXELS: usize = 256 * 256;
/// Pictures usvg may ask for while parsing, each decoded once, before the rest draw blank; it
/// asks for some it then drops.
const SVG_MAX_PICTURE_USES: usize = 2 * SVG_MAX_IMAGES;

/// Width and height of an embedded PNG or JPEG as the decoder resvg draws it with reads its
/// header; `None` when that fails or for another kind.
fn raster_size(kind: &ImageKind) -> Option<(u32, u32)> {
    match kind {
        ImageKind::PNG(data) => {
            let reader = png::Decoder::new(std::io::Cursor::new(data.as_slice()))
                .read_info()
                .ok()?;
            Some((reader.info().width, reader.info().height))
        }
        ImageKind::JPEG(data) => {
            let mut decoder = zune_jpeg::JpegDecoder::new(std::io::Cursor::new(data.as_slice()));
            decoder.decode_headers().ok()?;
            let (w, h) = decoder.dimensions()?;
            Some((u32::try_from(w).ok()?, u32::try_from(h).ok()?))
        }
        _ => None,
    }
}

/// Why `tree` is refused at `px` square within `budget` work, or `None` within the limits; the
/// walk stops once over. An embedded image is refused unless `images`.
pub(super) fn svg_over_budget(
    tree: &usvg::Tree,
    images: bool,
    px: u32,
    budget: f64,
) -> Option<&'static str> {
    let side = px as f32;
    let size = tree.size();
    let ts = Transform::from_scale(side / size.width(), side / size.height());
    let mut cost = Cost {
        no_images: !images,
        budget,
        ..Cost::default()
    };
    let canvas = (side, side);
    cost.pixmap(canvas, BYTES_PIXMAP);
    cost.children(tree.root(), ts, canvas, layer_bound(canvas));
    cost.over
}

/// Why [`svg_over_budget`] refuses `tree` when its work, or its pixmaps' bytes, do not shrink
/// with the canvas.
const PAST_BUDGET: &str = "past the render budget";
const PAST_PIXMAPS: &str = "pixmaps past the size limit";

/// The side `tree` is drawn at for `px` within `budget` work: `px`, or for one past it at `px`
/// the largest within it down to a quarter of `px`, scaled up after.
pub(super) fn svg_side(
    tree: &usvg::Tree,
    images: bool,
    px: u32,
    budget: f64,
) -> Result<u32, &'static str> {
    let floor = (px / 4).max(1);
    let mut side = px;
    loop {
        match svg_over_budget(tree, images, side, budget) {
            None => return Ok(side),
            Some(PAST_BUDGET | PAST_PIXMAPS) if side > floor => side = (side * 3 / 4).max(floor),
            Some(why) => return Err(why),
        }
    }
}

/// What drawing a tree asks of resvg, mirroring its walk: each node with the device transform it
/// is drawn with, the size of the pixmap it is drawn into and the bound resvg puts on a layer
/// under it.
#[derive(Default)]
struct Cost {
    no_images: bool,
    budget: f64,
    work: f64,
    /// Bytes of the pixmaps allocated so far.
    bytes: f64,
    images: usize,
    pixels: usize,
    over: Option<&'static str>,
}

/// A device-space rect's pixels within a `target`-sized pixmap.
fn area(r: Option<Rect>, target: (f32, f32)) -> f64 {
    r.map_or(0.0, |r| {
        r.width().min(target.0) as f64 * r.height().min(target.1) as f64
    })
}

/// resvg's bound on a layer drawn into a `target`-sized pixmap: five times it each way.
fn layer_bound(target: (f32, f32)) -> (f32, f32) {
    (5.0 * target.0, 5.0 * target.1)
}

/// The inverse of `ts`'s rotation and skew, as resvg takes them off a filtered group's layer;
/// None when resvg skips the group.
fn unrotation(ts: Transform) -> Option<Transform> {
    let sx = ts.sx.hypot(ts.ky);
    let sy = ts.kx.hypot(ts.sy);
    Transform::from_row(ts.sx / sx, ts.ky / sx, ts.kx / sy, ts.sy / sy, 0.0, 0.0).invert()
}

/// The size of a `bound`-sized rect's box transformed by `t`, rounded out as resvg does, plus a
/// margin for f32 error. Only `t`'s magnitudes count, so it grows steadily with a skew.
fn stretched(bound: (f32, f32), t: Transform) -> (f32, f32) {
    if !t.has_skew() {
        return bound;
    }
    let (w, h) = (f64::from(bound.0), f64::from(bound.1));
    let side = |a: f32, b: f32| {
        let v = f64::from(a.abs()) * w + f64::from(b.abs()) * h;
        let v = (v * (1.0 + 1e-6)).ceil() + 2.0;
        if v.is_finite() { v as f32 } else { f32::MAX }
    };
    (side(t.sx, t.kx), side(t.ky, t.sy))
}

impl Cost {
    fn add(&mut self, work: f64) {
        self.work += work;
        if self.work > self.budget && self.over.is_none() {
            self.over = Some(PAST_BUDGET);
        }
    }

    /// Counts a `size` pixmap of `bytes` per pixel and answers its pixels.
    fn pixmap(&mut self, size: (f32, f32), bytes: f64) -> f64 {
        let pixels = size.0 as f64 * size.1 as f64;
        self.bytes += pixels * bytes;
        if self.bytes > SVG_MAX_PIXMAP_BYTES && self.over.is_none() {
            self.over = Some(PAST_PIXMAPS);
        }
        pixels
    }

    fn children(&mut self, g: &Group, ts: Transform, target: (f32, f32), bound: (f32, f32)) {
        for n in g.children() {
            if self.over.is_some() {
                return;
            }
            self.add(WORK_NODE);
            match n {
                Node::Group(g) => self.group(g, ts, target, bound),
                Node::Path(p) => self.path(p, ts, target, bound),
                Node::Image(i) => self.image(i, ts, target),
                Node::Text(t) => self.group(t.flattened(), ts, target, bound),
            }
        }
    }

    fn group(&mut self, g: &Group, ts: Transform, target: (f32, f32), bound: (f32, f32)) {
        let ts = ts.pre_concat(g.transform());
        if !g.should_isolate() {
            return self.children(g, ts, target, bound);
        }
        // resvg draws a filtered group's layer without its rotation and skew, and stretches the
        // bound by their inverse: a steep skew asks for a far larger layer than the canvas.
        let (ts, bound) = match g.filters() {
            [] => (ts, bound),
            _ => {
                let Some(unrotate) = unrotation(ts) else {
                    return;
                };
                (unrotate.pre_concat(ts), stretched(bound, unrotate))
            }
        };
        let Some(bbox) = g.layer_bounding_box().transform(ts) else {
            return;
        };
        // A filter region is not padded for anti-aliasing: it clips.
        let (w, h) = match g.filters() {
            [] => (bbox.width().ceil() + 4.0, bbox.height().ceil() + 4.0),
            _ => (bbox.width().ceil().max(1.0), bbox.height().ceil().max(1.0)),
        };
        let layer = (w.min(bound.0), h.min(bound.1));
        let pixels = self.pixmap(layer, BYTES_PIXMAP);
        self.add(WORK_LAYER_PASS + pixels * WORK_LAYER);
        for f in g.filters() {
            self.filter(f, ts, layer);
        }
        self.children(g, ts, layer, bound);
        if let Some(c) = g.clip_path() {
            self.clip(c, ts, layer);
        }
        let mut mask = g.mask();
        while let Some(m) = mask {
            self.pixmap(layer, BYTES_MASK);
            self.add(WORK_MASK_PASS + pixels * WORK_MASK);
            self.children(m.root(), ts, layer, bound);
            mask = m.mask();
        }
    }

    /// A filter over a `layer`-sized region: resvg runs each primitive over all of it.
    fn filter(&mut self, f: &usvg::filter::Filter, ts: Transform, layer: (f32, f32)) {
        use resvg::usvg::filter::{ColorInterpolation, Input, Kind, LightSource};
        let pixels = self.pixmap(layer, BYTES_PIXMAP);
        let (sx, sy) = ts.get_scale();
        let blur = |x: f32, y: f32| blur(x * sx, y * sy, layer.0);
        // A morphology reads up to the whole region around each pixel.
        let taps = |r: f32, side: f32| (2.0 * r.round() + 1.0).min(side) as f64;
        let light = |l: LightSource| match l {
            LightSource::DistantLight(_) => WORK_DISTANT_LIGHT,
            _ => WORK_NEAR_LIGHT,
        };
        let linear =
            |p: &usvg::filter::Primitive| p.color_interpolation() == ColorInterpolation::LinearRGB;
        let converts = f.primitives().iter().any(linear);
        self.add(pixels * (WORK_FILTER + if converts { WORK_LINEAR_RGB } else { 0.0 }));
        for p in f.primitives() {
            if self.over.is_some() {
                return;
            }
            let work = match p.kind() {
                Kind::GaussianBlur(b) => blur(b.std_dev_x().get(), b.std_dev_y().get()),
                Kind::DropShadow(d) => blur(d.std_dev_x().get(), d.std_dev_y().get()) + WORK_SHADOW,
                Kind::Offset(_) => WORK_OFFSET,
                Kind::Flood(_) => WORK_FILL_REGION,
                Kind::Composite(_) | Kind::Blend(_) | Kind::DisplacementMap(_) => WORK_TWO_INPUTS,
                Kind::Merge(m) => m.inputs().len() as f64 * WORK_MERGE_INPUT,
                Kind::ColorMatrix(_) => WORK_COLOR_MATRIX,
                Kind::ComponentTransfer(_) => WORK_COMPONENT_TRANSFER,
                Kind::Tile(_) => WORK_TILE,
                Kind::Morphology(m) => {
                    let x = taps(m.radius_x().get() * sx, layer.0);
                    let y = taps(m.radius_y().get() * sy, layer.1);
                    WORK_MORPHOLOGY + x * y * WORK_MORPHOLOGY_TAP
                }
                Kind::ConvolveMatrix(c) => {
                    let taps = c.matrix().columns() as f64 * c.matrix().rows() as f64;
                    WORK_CONVOLVE + taps * WORK_CONVOLVE_TAP
                }
                Kind::Turbulence(t) => {
                    self.add(WORK_TURBULENCE_INIT);
                    WORK_TURBULENCE + t.num_octaves() as f64 * WORK_OCTAVE
                }
                Kind::DiffuseLighting(l) => light(l.light_source()),
                Kind::SpecularLighting(l) => light(l.light_source()),
                // Drawn into a pixmap of the region, at the device scale, which bounds its layers.
                Kind::Image(i) => {
                    self.add(WORK_LAYER_PASS);
                    self.children(i.root(), Transform::from_scale(sx, sy), layer, layer);
                    WORK_FILL_REGION
                }
            };
            let iir = |x: f32, y: f32| {
                let (x, y) = (x * sx, y * sy);
                let iir = x < 2.0 && y < 2.0 && (x, y) != (0.0, 0.0);
                if iir { BYTES_IIR } else { 0.0 }
            };
            let bytes = BYTES_PRIMITIVE
                + match p.kind() {
                    Kind::GaussianBlur(b) => iir(b.std_dev_x().get(), b.std_dev_y().get()),
                    Kind::DropShadow(d) => {
                        iir(d.std_dev_x().get(), d.std_dev_y().get()) + BYTES_SHADOW
                    }
                    _ => 0.0,
                };
            self.pixmap(layer, bytes);
            let sources = [Input::SourceGraphic, Input::SourceAlpha];
            let sources = sources.iter().filter(|i| p.kind().has_input(i)).count() as f64;
            let converted = if linear(p) {
                sources * WORK_LINEAR_RGB_INPUT
            } else {
                0.0
            };
            self.add(pixels * (work + converted));
        }
    }

    /// A clip path over a `layer`-sized pixmap: resvg fills its paths into a pixmap of the same
    /// size, and a child group with a clip path of its own into one more.
    fn clip(&mut self, c: &usvg::ClipPath, ts: Transform, layer: (f32, f32)) {
        let pixels = self.pixmap(layer, BYTES_CLIP);
        self.add(WORK_CLIP_PASS + pixels * WORK_CLIP);
        self.clip_children(c.root(), ts.pre_concat(c.transform()), layer);
        if let Some(c) = c.clip_path() {
            self.clip(c, ts, layer);
        }
    }

    fn clip_children(&mut self, g: &Group, ts: Transform, layer: (f32, f32)) {
        for n in g.children() {
            if self.over.is_some() {
                return;
            }
            self.add(WORK_NODE);
            match n {
                Node::Path(p) if p.is_visible() && p.fill().is_some() => {
                    self.add(area(p.bounding_box().transform(ts), layer) * WORK_FILL);
                    self.edges(p, ts, layer);
                }
                Node::Text(t) => self.clip_children(t.flattened(), ts, layer),
                Node::Group(g) => {
                    let ts = ts.pre_concat(g.transform());
                    if let Some(c) = g.clip_path() {
                        let pixels = self.pixmap(layer, BYTES_PIXMAP);
                        self.add(WORK_LAYER_PASS + pixels * WORK_LAYER);
                        self.clip_children(g, ts, layer);
                        self.clip(c, ts, layer);
                    } else {
                        self.clip_children(g, ts, layer);
                    }
                }
                _ => {}
            }
        }
    }

    fn path(&mut self, p: &usvg::Path, ts: Transform, target: (f32, f32), bound: (f32, f32)) {
        if !p.is_visible() {
            return;
        }
        if let Some(fill) = p.fill() {
            let covered = area(p.bounding_box().transform(ts), target);
            self.paint(fill.paint(), ts, covered, bound);
            self.edges(p, ts, target);
        }
        if let Some(stroke) = p.stroke() {
            let rows = rows(p, ts, target.1);
            let covered = area(p.stroke_bounding_box().transform(ts), target);
            self.paint(stroke.paint(), ts, covered, bound);
            // The stroker outlines each side of every segment, a dash a closed outline of its own.
            let n = segments(p);
            let (sx, sy) = ts.get_scale();
            let width = (stroke.width().get() * sx.max(sy)).min(target.1) as f64;
            let rows = 2.0 * rows + n * 2.0 * (width + 1.0);
            self.add(rows * WORK_EDGE_ROW + 4.0 * n * n * WORK_EDGE_PAIR);
        }
    }

    /// The rasterizer's work on `p`'s fill outline.
    fn edges(&mut self, p: &usvg::Path, ts: Transform, target: (f32, f32)) {
        let n = p.data().verbs().len() as f64;
        self.add(rows(p, ts, target.1) * WORK_EDGE_ROW + n * n * WORK_EDGE_PAIR);
    }

    fn paint(&mut self, paint: &Paint, ts: Transform, covered: f64, bound: (f32, f32)) {
        let gradient =
            |stops: usize| WORK_NODE + covered * (WORK_GRADIENT + stops as f64 * WORK_STOP);
        match paint {
            Paint::Color(_) => self.add(covered * WORK_FILL),
            Paint::LinearGradient(g) => self.add(gradient(g.stops().len())),
            Paint::RadialGradient(g) => self.add(gradient(g.stops().len())),
            Paint::Pattern(pattern) => {
                // resvg draws the tile at the device scale, whatever its size.
                let (sx, sy) = ts.pre_concat(pattern.transform()).get_scale();
                let rect = pattern.rect();
                let tile = ((rect.width() * sx).round(), (rect.height() * sy).round());
                let pixels = self.pixmap(tile, BYTES_PIXMAP);
                self.add(WORK_LAYER_PASS + pixels * WORK_LAYER + covered * WORK_BICUBIC);
                self.children(pattern.root(), Transform::from_scale(sx, sy), tile, bound);
            }
        }
    }

    fn image(&mut self, image: &usvg::Image, ts: Transform, target: (f32, f32)) {
        if !image.is_visible() {
            return;
        }
        if self.no_images {
            self.over = Some("an embedded image");
            return;
        }
        let kind = image.kind();
        if let ImageKind::SVG(tree) = kind {
            // resvg draws it through a layer the size of the pixmap, bounding the picture's own
            // layers by five times that; a [`blank`] draws nothing more.
            self.images += usize::from(tree.root().has_children());
            let pixels = self.pixmap(target, BYTES_PIXMAP);
            self.add(WORK_LAYER_PASS + pixels * WORK_LAYER);
            self.children(tree.root(), ts, target, layer_bound(target));
        } else {
            self.images += 1;
            let refused = match raster_size(kind) {
                None => Some("an unreadable embedded image"),
                Some((w, h)) if w > MAX_ICON_DIMENSION || h > MAX_ICON_DIMENSION => {
                    Some("an embedded image past the size limit")
                }
                Some((w, h)) => {
                    self.pixels += w as usize * h as usize;
                    (self.pixels > SVG_MAX_PICTURE_PIXELS)
                        .then_some("embedded pictures past the pixel budget")
                }
            };
            if refused.is_some() {
                self.over = refused;
            }
            let drawn = area(image.bounding_box().transform(ts), target);
            self.add(drawn * WORK_BICUBIC);
        }
        if self.images > SVG_MAX_IMAGES {
            self.over = Some("too many embedded images");
        }
    }
}

/// Work per pixel of a Gaussian blur of device standard deviations `x` and `y` over a region
/// `width` px wide; resvg skips an axis under 0.05 px, and the whole blur for two zeros.
fn blur(x: f32, y: f32, width: f32) -> f64 {
    let (rows, columns) = (
        f64::from(u8::from(x >= 0.05)),
        f64::from(u8::from(y >= 0.05)),
    );
    if x == 0.0 && y == 0.0 {
        return 0.0;
    }
    if x >= 2.0 || y >= 2.0 {
        return (rows + columns) * WORK_BOX_PASS;
    }
    // resvg's region can be a pixel wider than its rect.
    let aliased = (width.round() as u32 + 1) % 256 <= 2;
    let column = if aliased {
        WORK_IIR_COLUMNS_ALIASED
    } else {
        WORK_IIR_COLUMNS
    };
    WORK_IIR + rows * WORK_IIR_ROWS + columns * column
}

/// The segments `p` draws, plus dashes counted along the control polygon (never shorter than
/// the curve).
fn segments(p: &usvg::Path) -> f64 {
    let data = p.data();
    let mut n = data.verbs().len() as f64;
    if let Some(dash) = p.stroke().and_then(|s| s.dasharray()) {
        let period: f32 = dash.iter().sum();
        let length: f32 = data.points().windows(2).map(|w| w[0].distance(w[1])).sum();
        n += (length / period.max(f32::MIN_POSITIVE)) as f64 * dash.len() as f64;
    }
    n
}

/// Device pixel rows `p`'s edges span, each at most `height`, along its control polygon.
fn rows(p: &usvg::Path, ts: Transform, height: f32) -> f64 {
    let dy = |[a, b]: &[resvg::tiny_skia::Point; 2]| {
        let dy = ts.ky * (b.x - a.x) + ts.sy * (b.y - a.y);
        dy.abs().min(height) as f64 + 1.0
    };
    p.data().points().array_windows().map(dy).sum()
}

#[cfg(test)]
thread_local! {
    /// Pictures decoded, and bytes usvg handed the picture resolver, on this thread.
    pub(super) static SPENT: std::cell::Cell<(usize, usize)> = const { std::cell::Cell::new((0, 0)) };
}

/// Mime of the stand-in `data:` href [`hoist_pictures`] writes, its data the picture's index.
const PICTURE_MIME: &str = "image/x-reprise-picture";

/// An embedded picture's `data:` href, decoded the first time usvg asks for it, and its
/// [`blank`] once one stands in for it.
struct Picture {
    href: String,
    decoded: OnceLock<Decoded>,
    blank: OnceLock<Option<ImageKind>>,
}

/// A picture's size when known, what usvg draws with its PNG or JPEG pixels (None when left
/// out), and the work building it took when it is an SVG.
#[derive(Default)]
struct Decoded {
    size: Option<Size>,
    drawn: Option<(ImageKind, usize)>,
    work: f64,
}

/// An SVG's text with each `data:` href replaced by a stand-in, and the pictures they held.
struct Hoisted {
    text: String,
    pictures: Vec<Picture>,
}

/// Parses `text` as usvg does once its entities pass [`entities_over_budget`] (roxmltree
/// expands every reference again, with no limit), with the work usvg will do to build it,
/// refusing a document that work would take past `budget`.
pub(super) fn parse_xml(
    text: &str,
    budget: f64,
) -> Result<(roxmltree::Document<'_>, f64), &'static str> {
    if let Some(why) = entities_over_budget(text) {
        return Err(why);
    }
    let options = roxmltree::ParsingOptions {
        allow_dtd: true,
        ..Default::default()
    };
    let doc = roxmltree::Document::parse_with_options(text, options).map_err(|_| "bad xml")?;
    // A reference too deep to price counts as infinite, and NaN once multiplied by no vertices.
    let work = built_elements(&doc) * WORK_ELEMENT;
    if work.is_nan() || work > budget {
        return Err("too many elements once uses and references expand");
    }
    Ok((doc, work))
}

/// Elements usvg builds from `doc`, over-estimated: a `<use>` copies its target, a clip path,
/// mask or filter is converted for each element naming it, and another `url(#id)` or `href`
/// converts its target again for each element under the one naming it (each vertex, for a
/// marker). A `<style>` reference counts against every element, and a path vertex as the part
/// of an element its work is.
fn built_elements(doc: &roxmltree::Document) -> f64 {
    use std::collections::HashMap;
    type Node<'a, 'i> = roxmltree::Node<'a, 'i>;
    /// Elements under a node, what usvg builds from them, and the vertices a marker can be drawn
    /// at.
    #[derive(Clone, Copy, Default)]
    struct Built {
        elements: f64,
        built: f64,
        vertices: f64,
    }
    struct Walk<'a, 'i> {
        ids: HashMap<&'a str, Node<'a, 'i>>,
        /// None while a walk of the node is under way.
        seen: HashMap<roxmltree::NodeId, Option<Built>>,
    }
    const DEFINITIONS: [&str; 9] = [
        "defs",
        "clipPath",
        "filter",
        "linearGradient",
        "marker",
        "mask",
        "pattern",
        "radialGradient",
        "symbol",
    ];
    /// The ids `value` names through `url(#id)`.
    fn urls(value: &str) -> impl Iterator<Item = &str> {
        value.split("url(").skip(1).filter_map(|r| {
            let r = r.split(')').next()?.trim().trim_matches(['"', '\'']);
            r.strip_prefix('#')
        })
    }
    fn numbers(v: &str) -> f64 {
        let tokens = v.split(|c: char| !(c.is_ascii_digit() || c == '.'));
        tokens.filter(|t| !t.is_empty()).count() as f64
    }
    impl<'a, 'i> Walk<'a, 'i> {
        fn walk(&mut self, n: Node<'a, 'i>, depth: usize) -> Built {
            match self.seen.get(&n.id()) {
                Some(Some(known)) => return *known,
                // A reference back into itself, which usvg skips.
                Some(None) => return Built::default(),
                None if depth > 1024 => {
                    return Built {
                        built: f64::INFINITY,
                        ..Built::default()
                    };
                }
                None => {}
            }
            self.seen.insert(n.id(), None);
            let mut b = Built {
                elements: 1.0,
                built: 1.0,
                vertices: 0.0,
            };
            for c in n.children().filter(|c| c.is_element()) {
                let built = self.walk(c, depth + 1);
                b.elements += built.elements;
                b.vertices += built.vertices;
                // A definition is converted where it is named, not where it stands.
                b.built += if DEFINITIONS.contains(&c.tag_name().name()) {
                    built.elements
                } else {
                    built.built
                };
            }
            let (mut once, mut refs, mut marked) = (0.0, 0.0, 0.0);
            for a in n.attributes() {
                if matches!(a.name(), "d" | "points") {
                    let vertices = numbers(a.value()) / 2.0 + 1.0;
                    b.vertices += vertices;
                    b.built += vertices * WORK_VERTEX / WORK_ELEMENT;
                }
                let copy = a.name() == "href" && n.has_tag_name("use");
                // Each declaration of a `style` on its own, so `marker:none` names no marker.
                let declarations: Vec<(&str, &str)> = match a.name() {
                    "style" => (a.value().split(';'))
                        .filter_map(|d| d.split_once(':'))
                        .map(|(k, v)| (k.trim(), v))
                        .collect(),
                    name => vec![(name, a.value())],
                };
                for (name, value) in declarations {
                    let ids: Vec<&str> = if name == "href" {
                        value.strip_prefix('#').into_iter().collect()
                    } else {
                        urls(value).collect()
                    };
                    for id in ids {
                        let Some(t) = self.ids.get(id).copied() else {
                            continue;
                        };
                        let t = self.walk(t, depth + 1);
                        if copy {
                            b.elements += t.elements;
                            b.built += t.built;
                            b.vertices += t.vertices;
                        } else if name.starts_with("marker") {
                            marked += t.built;
                        } else if matches!(name, "clip-path" | "mask" | "filter") {
                            once += t.built;
                        } else {
                            refs += t.built;
                        }
                    }
                }
            }
            b.built += once + refs * b.elements + marked * b.vertices;
            self.seen.insert(n.id(), Some(b));
            b
        }
    }
    let mut ids = HashMap::new();
    for n in doc.descendants() {
        if let Some(id) = n.attribute("id") {
            ids.entry(id).or_insert(n);
        }
    }
    let mut w = Walk {
        ids,
        seen: HashMap::new(),
    };
    let root = w.walk(doc.root_element(), 0);
    let styles = doc.descendants().filter(|n| n.has_tag_name("style"));
    let mut built = root.built;
    for id in styles.flat_map(|n| n.text()).flat_map(urls) {
        if let Some(t) = w.ids.get(id).copied() {
            built += w.walk(t, 0).built * (root.elements + root.vertices);
        }
    }
    built
}

/// Why the entities `text` declares are refused: a value holding markup or a reference, or more
/// than `SVG_MAX_BYTES` once every reference expands to the longest value.
fn entities_over_budget(text: &str) -> Option<&'static str> {
    let space = |c: char| c.is_ascii_whitespace();
    let mut longest = 0;
    for decl in text.split("<!ENTITY").skip(1) {
        let decl = decl.trim_start_matches(space);
        let decl = decl
            .strip_prefix('%')
            .unwrap_or(decl)
            .trim_start_matches(space);
        let decl = decl
            .trim_start_matches(|c| !space(c))
            .trim_start_matches(space);
        // An external entity is never loaded.
        let Some(quote) = decl.chars().next().filter(|&c| c == '"' || c == '\'') else {
            continue;
        };
        let Some((value, _)) = decl[1..].split_once(quote) else {
            return Some("bad xml");
        };
        if value.contains(['<', '&']) {
            return Some("an entity holding markup");
        }
        longest = longest.max(value.len());
    }
    const BUILT_IN: [&str; 6] = ["#", "lt;", "gt;", "amp;", "apos;", "quot;"];
    let references = (text.split('&').skip(1))
        .filter(|r| !BUILT_IN.iter().any(|b| r.starts_with(b)))
        .count();
    (references.saturating_mul(longest) > SVG_MAX_BYTES).then_some("entities past the size limit")
}

/// Every `href` attribute holding a `data:` URL, with its value's range in the text.
fn data_hrefs<'a>(
    doc: &'a roxmltree::Document,
) -> impl Iterator<Item = (std::ops::Range<usize>, &'a str)> {
    doc.descendants()
        .flat_map(|n| n.attributes())
        .filter(|a| a.name() == "href" && data_url::DataUrl::process(a.value()).is_ok())
        .map(|a| (a.range_value(), a.value()))
}

/// Replaces each `data:` href of `doc` with a stand-in naming its picture: usvg decodes an href
/// again for every `<use>` copy of it, so a small file could ask for gigabytes.
fn hoist_pictures(doc: &roxmltree::Document) -> Result<Hoisted, &'static str> {
    let text = doc.input_text();
    let mut hrefs: Vec<_> = data_hrefs(doc).collect();
    hrefs.sort_by_key(|(r, _)| (r.start, r.end));
    let mut out = String::with_capacity(text.len());
    let (mut at, mut pictures) = (0, Vec::<Picture>::new());
    let mut index = std::collections::HashMap::new();
    for (range, value) in hrefs {
        let (Some(before), Some(_)) = (text.get(at..range.start), text.get(range.clone())) else {
            return Err("an embedded image out of place");
        };
        let i = *index.entry(value).or_insert_with(|| {
            pictures.push(Picture {
                href: value.to_owned(),
                decoded: OnceLock::new(),
                blank: OnceLock::new(),
            });
            pictures.len() - 1
        });
        out += before;
        out += &format!("data:{PICTURE_MIME},{i}");
        at = range.end;
    }
    out += &text[at..];
    Ok(Hoisted {
        text: out,
        pictures,
    })
}

/// The picture `href` holds, an SVG one built and drawn within `budget` work.
fn decode_picture(href: &str, budget: f64) -> Decoded {
    let mut d = Decoded::default();
    d.drawn = drawn_picture(href, budget, &mut d);
    d
}

/// The picture a `data:` href holds, as usvg reads it, and its PNG or JPEG pixels, setting `d`'s
/// size and work once known; None for GIF, WebP, a side past 1024, or an SVG embedding pictures
/// or over `budget`.
fn drawn_picture(href: &str, budget: f64, d: &mut Decoded) -> Option<(ImageKind, usize)> {
    #[cfg(test)]
    SPENT.with(|s| s.set((s.get().0 + 1, s.get().1)));
    let url = data_url::DataUrl::process(href).ok()?;
    let (data, _) = url.decode_to_vec().ok()?;
    let no_pictures = usvg::Options {
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        },
        ..Default::default()
    };
    let mime = format!("{}/{}", url.mime_type().type_, url.mime_type().subtype);
    // usvg reads a UTF-8 payload of these types as an SVG, with its own parser.
    if matches!(mime.as_str(), "image/svg+xml" | "text/plain")
        && let Ok(text) = std::str::from_utf8(&data)
    {
        let (doc, work) = parse_xml(text, budget).ok()?;
        if data_hrefs(&doc).next().is_some() {
            return None;
        }
        let tree = usvg::Tree::from_xmltree(&doc, &no_pictures).ok()?;
        d.work = work;
        d.size = Some(tree.size());
        // At 1 px only its nodes count: the icon's own walk prices its pixels where it is drawn.
        return svg_over_budget(&tree, true, 1, budget - work)
            .is_none()
            .then_some((ImageKind::SVG(tree), 0));
    }
    let read = usvg::ImageHrefResolver::default_data_resolver();
    let kind = read(&mime, data.into(), &no_pictures)?;
    let (w, h) = raster_size(&kind)?;
    d.size = Size::from_wh(w as f32, h as f32);
    (w <= MAX_ICON_DIMENSION && h <= MAX_ICON_DIMENSION).then_some((kind, w as usize * h as usize))
}

/// An SVG of `size` (1x1 when unknown) that draws nothing. It stands in for a picture left out:
/// usvg would drop the element, leaving a filter or mask over it no box and logging a warning.
fn blank(size: Option<Size>) -> Option<ImageKind> {
    let (w, h) = size.map_or((1.0, 1.0), |s| (s.width(), s.height()));
    let text = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}"/>"#);
    let tree = usvg::Tree::from_str(&text, &usvg::Options::default()).ok()?;
    Some(ImageKind::SVG(tree))
}

/// usvg's tree of the SVG `text` and the work building it took. Its embedded pictures are
/// decoded when usvg first asks for one, and one past the picture budgets draws [`blank`];
/// unless `images`, any refuses the SVG.
fn svg_tree(text: &str, images: bool) -> Result<(resvg::usvg::Tree, f64), String> {
    let (doc, work) = parse_xml(text, SVG_MAX_WORK)?;
    if data_hrefs(&doc).next().is_none() {
        return tree_with(&doc, work, &[]);
    }
    if !images {
        return Err("an embedded image".into());
    }
    let hoisted = hoist_pictures(&doc)?;
    drop(doc);
    let (doc, work) = parse_xml(&hoisted.text, SVG_MAX_WORK)?;
    let stand_in = format!("data:{PICTURE_MIME},");
    if data_hrefs(&doc).any(|(_, v)| !v.starts_with(&stand_in)) {
        return Err("an embedded image out of place".into());
    }
    tree_with(&doc, work, &hoisted.pictures)
}

/// usvg's tree of `doc`, whose stand-in hrefs index `pictures`, and `work` plus the work
/// building its SVG pictures took; each picture gets what the budget has left.
fn tree_with(
    doc: &roxmltree::Document,
    work: f64,
    pictures: &[Picture],
) -> Result<(resvg::usvg::Tree, f64), String> {
    #[derive(Default)]
    struct Spent {
        calls: usize,
        drawn: usize,
        pixels: usize,
        work: f64,
    }
    let spent = Mutex::new(Spent {
        work,
        ..Spent::default()
    });
    // No file references: the name comes from a peer, and usvg's default resolver would read
    // any `<image href>` path.
    let options = usvg::Options {
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: Box::new(|mime, data, _| {
                #[cfg(test)]
                SPENT.with(|s| s.set((s.get().0, s.get().1 + data.len())));
                let mut spent = spent.lock().unwrap_or_else(PoisonError::into_inner);
                spent.calls += 1;
                if mime != PICTURE_MIME {
                    return None;
                }
                let index: usize = std::str::from_utf8(&data).ok()?.parse().ok()?;
                let p = pictures.get(index)?;
                let open = spent.calls <= SVG_MAX_PICTURE_USES && spent.drawn < SVG_MAX_IMAGES;
                let decoded = match p.decoded.get() {
                    None if open => {
                        let d = decode_picture(&p.href, SVG_MAX_WORK - spent.work);
                        spent.work += d.work;
                        Some(p.decoded.get_or_init(|| d))
                    }
                    decoded => decoded,
                };
                if let Some((kind, pixels)) = decoded.and_then(|d| d.drawn.as_ref())
                    && open
                    && spent.pixels + pixels <= SVG_MAX_PICTURE_PIXELS
                {
                    spent.drawn += 1;
                    spent.pixels += pixels;
                    return Some(kind.clone());
                }
                let size = decoded.and_then(|d| d.size);
                p.blank.get_or_init(|| blank(size)).clone()
            }),
            resolve_string: Box::new(|_, _| None),
        },
        ..Default::default()
    };
    let tree = usvg::Tree::from_xmltree(doc, &options).map_err(|e| e.to_string())?;
    drop(options);
    let work = spent
        .into_inner()
        .unwrap_or_else(PoisonError::into_inner)
        .work;
    Ok((tree, work))
}

fn decode_svg(path: &Path, bytes: &[u8], px: u32, images: bool) -> Option<Canvas> {
    use resvg::tiny_skia;
    if bytes.len() > SVG_MAX_BYTES {
        log::warn!("icon {}: svg larger than an icon", path.display());
        return None;
    }
    let tree = std::str::from_utf8(bytes)
        .map_err(|_| "not utf-8".to_string())
        .and_then(|text| svg_tree(text, images));
    let (tree, built) = match tree {
        Ok(t) => t,
        Err(why) => {
            log::warn!("icon {}: svg refused: {why}", path.display());
            return None;
        }
    };
    let size = tree.size();
    let bound = MAX_ICON_DIMENSION as f32;
    if size.width() <= 0.0 || size.height() <= 0.0 || size.width() > bound || size.height() > bound
    {
        log::warn!(
            "icon {}: refusing a {}x{} svg",
            path.display(),
            size.width(),
            size.height()
        );
        return None;
    }
    let side = match svg_side(&tree, images, px, SVG_MAX_WORK - built) {
        Ok(side) => side,
        Err(why) => {
            log::warn!("icon {}: svg refused at {px} px: {why}", path.display());
            return None;
        }
    };
    if side < px {
        log::debug!("icon {}: drawn at {side} px for {px}", path.display());
    }
    let mut pixmap = tiny_skia::Pixmap::new(side, side)?;
    let sx = side as f32 / size.width();
    let sy = side as f32 / size.height();
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(sx, sy),
        &mut pixmap.as_mut(),
    );
    if side < px {
        let mut full = tiny_skia::Pixmap::new(px, px)?;
        let paint = tiny_skia::PixmapPaint {
            quality: tiny_skia::FilterQuality::Bilinear,
            ..Default::default()
        };
        let grow = px as f32 / side as f32;
        let grow = tiny_skia::Transform::from_scale(grow, grow);
        full.draw_pixmap(0, 0, pixmap.as_ref(), &paint, grow, None);
        pixmap = full;
    }
    let mut c = Canvas::new(px, px);
    for (i, p) in pixmap.pixels().iter().enumerate() {
        c.pixels[i] = (p.alpha() as u32) << 24
            | (p.red() as u32) << 16
            | (p.green() as u32) << 8
            | p.blue() as u32;
    }
    Some(c)
}

/// Decode a PNG or SVG and resample to `px` square. XPM is not decoded. Sizes past
/// `MAX_ICON_DIMENSION` are refused.
pub fn decode(path: &Path, px: u32) -> Option<Canvas> {
    if px == 0 || px > MAX_ICON_DIMENSION {
        return None;
    }
    // Gate on the extension before opening: a client-supplied path like `/dev/zero` must not
    // be read.
    if !is_icon_file(path) {
        log::debug!("icon {}: unsupported file type", path.display());
        return None;
    }
    let bytes = match read_icon_file(path) {
        Ok(b) => b,
        Err(e) => {
            log::warn!("icon {}: {e}", path.display());
            return None;
        }
    };
    decode_read(path, &bytes, px, true)
}

/// Whether `path` names a file [`decode`] reads: a `.png` or `.svg`.
pub fn is_icon_file(path: &Path) -> bool {
    let ext = path.extension().and_then(|e| e.to_str());
    ext.is_some_and(|e| e.eq_ignore_ascii_case("png") || e.eq_ignore_ascii_case("svg"))
}

/// [`decode`] for the bytes of an icon file a peer named, read elsewhere within
/// [`MAX_ICON_BYTES`]. An SVG embedding a raster image is refused unless `images`.
pub fn decode_bytes(path: &Path, bytes: &[u8], px: u32, images: bool) -> Option<Canvas> {
    decode_read(path, bytes, px, images)
}

fn decode_read(path: &Path, bytes: &[u8], px: u32, images: bool) -> Option<Canvas> {
    if px == 0 || px > MAX_ICON_DIMENSION {
        return None;
    }
    // The content picks the decoder: themes ship PNGs named `.svg`.
    if bytes.starts_with(PNG_SIGNATURE) {
        decode_png(path, bytes, px)
    } else {
        decode_svg(path, bytes, px, images)
    }
}

/// Icon file `path`'s image as [`IconCache::get`](super::IconCache::get) loads it for `px` at
/// `scale`, for a host that reads user files off its event loop and hands the result to
/// [`IconCache::put_file`](super::IconCache::put_file).
pub fn decode_file(path: &Path, px: i32, scale: i32) -> Option<Canvas> {
    let dpx = u32::try_from(px.checked_mul(scale.max(1))?).ok()?;
    decode(path, dpx).map(|c| c.at_scale(scale))
}

const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

/// The most of an icon file read.
pub const MAX_ICON_BYTES: u64 = 8 << 20;

/// Read an icon file without stalling on a peer-chosen path: held `O_PATH` until it is known to
/// be a regular file (a FIFO or device is never opened), read capped at `MAX_ICON_BYTES`.
fn read_icon_file(path: &Path) -> std::io::Result<Vec<u8>> {
    use std::io::{Error, ErrorKind, Read};
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;
    let held = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_PATH)
        .open(path)?;
    let meta = held.metadata()?;
    if !meta.file_type().is_file() {
        return Err(Error::new(ErrorKind::InvalidInput, "not a regular file"));
    }
    if meta.len() > MAX_ICON_BYTES {
        return Err(Error::new(ErrorKind::InvalidData, "larger than an icon"));
    }
    let f = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY)
        .open(format!("/proc/self/fd/{}", held.as_raw_fd()))?;
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    f.take(MAX_ICON_BYTES).read_to_end(&mut bytes)?;
    Ok(bytes)
}
