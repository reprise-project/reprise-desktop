use crate::icon::premultiply;
use crate::raster::Canvas;
use std::path::Path;

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
    // RGBA passes through; other layouts are widened into a copy.
    let rgba: std::borrow::Cow<[u8]> = match info.color_type {
        png::ColorType::Rgba => std::borrow::Cow::Borrowed(&buf[..info.buffer_size()]),
        png::ColorType::Rgb => buf[..info.buffer_size()]
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect::<Vec<u8>>()
            .into(),
        png::ColorType::GrayscaleAlpha => buf[..info.buffer_size()]
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect::<Vec<u8>>()
            .into(),
        png::ColorType::Grayscale => buf[..info.buffer_size()]
            .iter()
            .flat_map(|&g| [g, g, g, 255])
            .collect::<Vec<u8>>()
            .into(),
        // EXPAND turns indexed into RGB/RGBA; anything else is unsupported.
        png::ColorType::Indexed => return Err("unsupported png color type".into()),
    };
    canvas_from_rgba(w, h, &rgba).ok_or_else(|| "png data shorter than its header".into())
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

/// Render cost limits for an SVG icon: resvg has none, and a peer-named file can ask for
/// seconds of work in a few hundred bytes. Stock theme icons fit well inside.
pub(super) const SVG_MAX_BYTES: usize = 1 << 20;
/// Nodes drawn, counting a mask or clip path once per element using it.
pub(super) const SVG_MAX_NODES: usize = 10_000;
/// Groups drawn through an offscreen layer.
pub(super) const SVG_MAX_LAYERS: usize = 1_000;
/// Filter primitives applied, each over a layer up to 4x the canvas.
pub(super) const SVG_MAX_PRIMITIVES: usize = 256;
/// Path segments stroked or filled, dashes included.
pub(super) const SVG_MAX_SEGMENTS: f32 = 100_000.0;

/// Why `tree` is refused, or `None` within the limits. The walk stops at the first limit passed.
pub(super) fn svg_over_budget(tree: &resvg::usvg::Tree) -> Option<&'static str> {
    use resvg::usvg::{Group, Node, Path, filter::Kind};
    #[derive(Default)]
    struct Cost {
        nodes: usize,
        layers: usize,
        primitives: usize,
        segments: f32,
        over: Option<&'static str>,
    }
    impl Cost {
        fn check(&mut self) {
            if self.segments > SVG_MAX_SEGMENTS {
                self.over = Some("too many path segments");
            } else if self.nodes > SVG_MAX_NODES {
                self.over = Some("too many nodes");
            } else if self.layers > SVG_MAX_LAYERS {
                self.over = Some("too many layers");
            } else if self.primitives > SVG_MAX_PRIMITIVES {
                self.over = Some("too many filter primitives");
            }
        }
    }
    /// The segments `p` draws, plus dashes counted along the control polygon (never shorter
    /// than the curve).
    fn segments(p: &Path) -> f32 {
        let data = p.data();
        let mut n = data.verbs().len() as f32;
        if let Some(dash) = p.stroke().and_then(|s| s.dasharray()) {
            let period: f32 = dash.iter().sum();
            let length: f32 = data.points().windows(2).map(|w| w[0].distance(w[1])).sum();
            n += length / period.max(f32::MIN_POSITIVE) * dash.len() as f32;
        }
        n
    }
    fn walk(g: &Group, c: &mut Cost) {
        c.nodes += 1;
        if g.opacity().get() < 1.0
            || g.mask().is_some()
            || g.clip_path().is_some()
            || !g.filters().is_empty()
        {
            c.layers += 1;
        }
        for p in g.filters().iter().flat_map(|f| f.primitives()) {
            c.primitives += 1;
            // Per-pixel cost that grows with an attribute or a nested image; no stock theme
            // uses them.
            if matches!(
                p.kind(),
                Kind::ConvolveMatrix(_)
                    | Kind::Morphology(_)
                    | Kind::Turbulence(_)
                    | Kind::DiffuseLighting(_)
                    | Kind::SpecularLighting(_)
                    | Kind::DisplacementMap(_)
                    | Kind::Image(_)
            ) {
                c.over = Some("an unbounded filter primitive");
            }
        }
        c.check();
        for n in g.children() {
            if c.over.is_some() {
                return;
            }
            match n {
                Node::Group(child) => walk(child, c),
                other => {
                    c.nodes += 1;
                    if let Node::Path(p) = other {
                        c.segments += segments(p);
                    }
                    c.check();
                }
            }
            n.subroots(|root| {
                if c.over.is_none() {
                    walk(root, c)
                }
            });
        }
    }
    let mut cost = Cost::default();
    walk(tree.root(), &mut cost);
    cost.over
}

fn decode_svg(path: &Path, bytes: &[u8], px: u32) -> Option<Canvas> {
    use resvg::tiny_skia;
    use resvg::usvg;
    // No file references: the name comes from a peer, and usvg's default resolver would read
    // any `<image href>` path. Data URIs stay.
    let options = usvg::Options {
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_string: Box::new(|_, _| None),
            ..Default::default()
        },
        ..Default::default()
    };
    if bytes.len() > SVG_MAX_BYTES {
        log::warn!("icon {}: svg larger than an icon", path.display());
        return None;
    }
    let tree = match usvg::Tree::from_data(bytes, &options) {
        Ok(t) => t,
        Err(e) => {
            log::warn!("icon {}: bad svg: {e}", path.display());
            return None;
        }
    };
    if let Some(why) = svg_over_budget(&tree) {
        log::warn!("icon {}: refusing an svg with {why}", path.display());
        return None;
    }
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
    let mut pixmap = tiny_skia::Pixmap::new(px, px)?;
    let sx = px as f32 / size.width();
    let sy = px as f32 / size.height();
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(sx, sy),
        &mut pixmap.as_mut(),
    );
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
    let ext = path.extension().and_then(|e| e.to_str());
    if !ext.is_some_and(|e| e.eq_ignore_ascii_case("png") || e.eq_ignore_ascii_case("svg")) {
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
    // The content picks the decoder: themes ship PNGs named `.svg`.
    if bytes.starts_with(PNG_SIGNATURE) {
        decode_png(path, &bytes, px)
    } else {
        decode_svg(path, &bytes, px)
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

const MAX_ICON_BYTES: u64 = 8 << 20;

/// Read an icon file without stalling on a peer-chosen path: non-blocking open (FIFO), regular
/// files only, read capped at `MAX_ICON_BYTES`.
fn read_icon_file(path: &Path) -> std::io::Result<Vec<u8>> {
    use std::io::{Error, ErrorKind, Read};
    use std::os::unix::fs::OpenOptionsExt;
    let f = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY)
        .open(path)?;
    let meta = f.metadata()?;
    if !meta.file_type().is_file() {
        return Err(Error::new(ErrorKind::InvalidInput, "not a regular file"));
    }
    if meta.len() > MAX_ICON_BYTES {
        return Err(Error::new(ErrorKind::InvalidData, "larger than an icon"));
    }
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    f.take(MAX_ICON_BYTES).read_to_end(&mut bytes)?;
    Ok(bytes)
}
