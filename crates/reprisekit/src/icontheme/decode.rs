use crate::icon::premultiply;
use crate::raster::Canvas;
use resvg::usvg::{ImageKind, Size, roxmltree};
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

/// Why `tree` is refused, or `None` within the limits; the walk stops at the first one passed. An
/// embedded image is refused unless `images`.
pub(super) fn svg_over_budget(tree: &resvg::usvg::Tree, images: bool) -> Option<&'static str> {
    use resvg::usvg::{Group, Node, Path, filter::Kind};
    #[derive(Default)]
    struct Cost {
        no_images: bool,
        nodes: usize,
        layers: usize,
        primitives: usize,
        segments: f32,
        images: usize,
        pixels: usize,
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
            } else if self.images > SVG_MAX_IMAGES {
                self.over = Some("too many embedded images");
            }
        }
        fn image(&mut self, kind: &ImageKind) {
            if let ImageKind::SVG(tree) = kind {
                // resvg draws it through a layer the size of the canvas; a [`blank`] draws
                // nothing more.
                self.layers += 1;
                self.images += usize::from(tree.root().has_children());
                return;
            }
            self.images += 1;
            self.over = match raster_size(kind) {
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
                    match other {
                        Node::Path(p) => c.segments += segments(p),
                        Node::Image(_) if c.no_images => c.over = Some("an embedded image"),
                        Node::Image(image) => c.image(image.kind()),
                        _ => {}
                    }
                    if c.over.is_none() {
                        c.check();
                    }
                }
            }
            n.subroots(|root| {
                if c.over.is_none() {
                    walk(root, c)
                }
            });
        }
    }
    let mut cost = Cost {
        no_images: !images,
        ..Cost::default()
    };
    walk(tree.root(), &mut cost);
    cost.over
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

/// A picture's size when known, and what usvg draws with its PNG or JPEG pixels; None when left
/// out.
struct Decoded {
    size: Option<Size>,
    drawn: Option<(ImageKind, usize)>,
}

/// An SVG's text with each `data:` href replaced by a stand-in, and the pictures they held.
struct Hoisted {
    text: String,
    pictures: Vec<Picture>,
}

/// Parses `text` as usvg does once its entities pass [`entities_over_budget`]: roxmltree
/// expands every reference again, with no limit.
fn parse_xml(text: &str) -> Result<roxmltree::Document<'_>, &'static str> {
    if let Some(why) = entities_over_budget(text) {
        return Err(why);
    }
    let options = roxmltree::ParsingOptions {
        allow_dtd: true,
        ..Default::default()
    };
    roxmltree::Document::parse_with_options(text, options).map_err(|_| "bad xml")
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

fn decode_picture(href: &str) -> Decoded {
    let mut size = None;
    let drawn = drawn_picture(href, &mut size);
    Decoded { size, drawn }
}

/// The picture a `data:` href holds, as usvg reads it, and its PNG or JPEG pixels, setting `size`
/// once read; None for GIF, WebP, a side past 1024, or an SVG embedding pictures or over budget.
fn drawn_picture(href: &str, size: &mut Option<Size>) -> Option<(ImageKind, usize)> {
    use resvg::usvg;
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
        let doc = parse_xml(text).ok()?;
        if data_hrefs(&doc).next().is_some() {
            return None;
        }
        let tree = usvg::Tree::from_xmltree(&doc, &no_pictures).ok()?;
        *size = Some(tree.size());
        return svg_over_budget(&tree, true)
            .is_none()
            .then_some((ImageKind::SVG(tree), 0));
    }
    let read = usvg::ImageHrefResolver::default_data_resolver();
    let kind = read(&mime, data.into(), &no_pictures)?;
    let (w, h) = raster_size(&kind)?;
    *size = Size::from_wh(w as f32, h as f32);
    (w <= MAX_ICON_DIMENSION && h <= MAX_ICON_DIMENSION).then_some((kind, w as usize * h as usize))
}

/// An SVG of `size` (1x1 when unknown) that draws nothing. It stands in for a picture left out:
/// usvg would drop the element, leaving a filter or mask over it no box and logging a warning.
fn blank(size: Option<Size>) -> Option<ImageKind> {
    use resvg::usvg;
    let (w, h) = size.map_or((1.0, 1.0), |s| (s.width(), s.height()));
    let text = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}"/>"#);
    let tree = usvg::Tree::from_str(&text, &usvg::Options::default()).ok()?;
    Some(ImageKind::SVG(tree))
}

/// usvg's tree of the SVG `text`. Its embedded pictures are decoded when usvg first asks for
/// one, and one past the picture budgets draws [`blank`]; unless `images`, any refuses the SVG.
fn svg_tree(text: &str, images: bool) -> Result<resvg::usvg::Tree, String> {
    let doc = parse_xml(text)?;
    if data_hrefs(&doc).next().is_none() {
        return tree_with(&doc, &[]);
    }
    if !images {
        return Err("an embedded image".into());
    }
    let hoisted = hoist_pictures(&doc)?;
    drop(doc);
    let doc = parse_xml(&hoisted.text)?;
    let stand_in = format!("data:{PICTURE_MIME},");
    if data_hrefs(&doc).any(|(_, v)| !v.starts_with(&stand_in)) {
        return Err("an embedded image out of place".into());
    }
    tree_with(&doc, &hoisted.pictures)
}

/// usvg's tree of `doc`, whose stand-in hrefs index `pictures`.
fn tree_with(doc: &roxmltree::Document, pictures: &[Picture]) -> Result<resvg::usvg::Tree, String> {
    use resvg::usvg;
    #[derive(Default)]
    struct Spent {
        calls: usize,
        drawn: usize,
        pixels: usize,
    }
    let spent = Mutex::new(Spent::default());
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
                let decoded = if open {
                    Some(p.decoded.get_or_init(|| decode_picture(&p.href)))
                } else {
                    p.decoded.get()
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
    usvg::Tree::from_xmltree(doc, &options).map_err(|e| e.to_string())
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
    let tree = match tree {
        Ok(t) => t,
        Err(why) => {
            log::warn!("icon {}: svg refused: {why}", path.display());
            return None;
        }
    };
    if let Some(why) = svg_over_budget(&tree, images) {
        log::warn!("icon {}: svg refused: {why}", path.display());
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
