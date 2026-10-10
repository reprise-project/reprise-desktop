//! Named pointer images from the `[cursor]` xcursor theme, loaded once per
//! name and scale and kept as render buffers. A name the theme lacks falls
//! back to "default", and a theme with neither to a drawn arrow.

use std::collections::HashMap;

use smithay::{
    backend::{allocator::Fourcc, renderer::element::memory::MemoryRenderBuffer},
    utils::{Logical, Point, Transform},
};
use xcursor::{
    CursorTheme,
    parser::{Image, parse_xcursor},
};

#[derive(Clone)]
pub struct CursorImage {
    pub buffer: MemoryRenderBuffer,
    pub hotspot: Point<i32, Logical>,
}

pub struct Cursors {
    theme: CursorTheme,
    size: u32,
    loaded: HashMap<(String, i32), CursorImage>,
    known: HashMap<&'static str, bool>,
    /// The image a running drag shows in place of the named cursor
    /// (`dnd`): drag cursors have xcursor names no `CursorIcon` has.
    pub drag: Option<&'static str>,
}

impl Cursors {
    pub fn new(theme_size: (String, u32)) -> Cursors {
        let (theme, size) = theme_size;
        Cursors {
            theme: CursorTheme::load(&theme),
            size,
            loaded: HashMap::new(),
            known: HashMap::new(),
            drag: None,
        }
    }

    pub fn has(&mut self, name: &'static str) -> bool {
        let theme = &self.theme;
        *self
            .known
            .entry(name)
            .or_insert_with(|| theme.load_icon(name).is_some())
    }

    pub fn get(&mut self, name: &str, scale: i32) -> &CursorImage {
        let key = (name.to_string(), scale);
        if !self.loaded.contains_key(&key) {
            let pixel_size = self.size * scale.max(1) as u32;
            let image = [name, "default", "left_ptr"]
                .into_iter()
                .find_map(|n| self.load(n, pixel_size))
                .unwrap_or_else(|| {
                    log::warn!("cursor: no {name:?} or default in the theme; drawing an arrow");
                    arrow(pixel_size)
                });
            self.loaded.insert(key.clone(), to_cursor(&image, scale));
        }
        &self.loaded[&key]
    }

    fn load(&self, name: &str, size: u32) -> Option<Image> {
        let path = self.theme.load_icon(name)?;
        let data = std::fs::read(path).ok()?;
        let images = parse_xcursor(&data)?;
        nearest(&images, size).cloned()
    }
}

fn nearest(images: &[Image], size: u32) -> Option<&Image> {
    images
        .iter()
        .min_by_key(|i| (i64::from(i.size) - i64::from(size)).abs())
}

/// `image`'s pixels as DRM `Argb8888` bytes. An xcursor file holds little-endian premultiplied ARGB words, which in memory are exactly that: use `pixels_rgba` as read (the parser's name). `pixels_argb` moves alpha to the front, which would read blue for black and take alpha from red.
fn drm_bytes(image: &Image) -> &[u8] {
    &image.pixels_rgba
}

fn to_cursor(image: &Image, scale: i32) -> CursorImage {
    let scale = scale.max(1);
    let buffer = MemoryRenderBuffer::from_slice(
        drm_bytes(image),
        Fourcc::Argb8888,
        (image.width as i32, image.height as i32),
        scale,
        Transform::Normal,
        None,
    );
    CursorImage {
        buffer,
        hotspot: (image.xhot as i32 / scale, image.yhot as i32 / scale).into(),
    }
}

fn arrow(size: u32) -> Image {
    let h = size.max(8);
    let w = h * 2 / 3;
    let mut argb = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        let row_w = if y < h * 2 / 3 { y.min(w) } else { 0 };
        for x in 0..row_w {
            let edge = x == 0 || x + 1 == row_w || y + 1 == h * 2 / 3;
            let v = if edge { 0xff } else { 0x00 };
            let i = ((y * w + x) * 4) as usize;
            argb[i..i + 4].copy_from_slice(&[v, v, v, 0xff]);
        }
    }
    Image {
        size: h,
        width: w,
        height: h,
        xhot: 0,
        yhot: 0,
        delay: 0,
        pixels_rgba: argb,
        pixels_argb: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(size: u32) -> Image {
        Image {
            size,
            width: size,
            height: size,
            xhot: 0,
            yhot: 0,
            delay: 0,
            pixels_rgba: Vec::new(),
            pixels_argb: Vec::new(),
        }
    }

    fn xcursor_file(w: u32, h: u32, pixels: &[u32]) -> Vec<u8> {
        let mut f = Vec::new();
        let mut put = |v: u32| f.extend_from_slice(&v.to_le_bytes());
        f_magic(&mut put);
        for v in [16, 0x1_0000, 1] {
            put(v);
        }
        for v in [0xfffd_0002, w, 28] {
            put(v);
        }
        for v in [36, 0xfffd_0002, w, 1, w, h, 0, 0, 0] {
            put(v);
        }
        for &p in pixels {
            put(p);
        }
        f
    }

    fn f_magic(put: &mut impl FnMut(u32)) {
        put(u32::from_le_bytes(*b"Xcur"));
    }

    #[test]
    fn a_theme_pixel_reaches_the_buffer_as_drawn() {
        let file = xcursor_file(3, 1, &[0xff00_0000, 0xffff_ffff, 0x8000_0000]);
        let images = parse_xcursor(&file).expect("the file parses");
        assert_eq!(
            drm_bytes(&images[0]),
            &[0, 0, 0, 0xff, 0xff, 0xff, 0xff, 0xff, 0, 0, 0, 0x80],
            "Argb8888 bytes are blue, green, red, alpha: black stays black and opaque"
        );
    }

    #[test]
    fn nearest_picks_the_closest_nominal_size() {
        let images = [image(24), image(32), image(48)];
        assert_eq!(nearest(&images, 30).map(|i| i.size), Some(32));
        assert_eq!(nearest(&images, 64).map(|i| i.size), Some(48));
        assert_eq!(nearest(&[], 24).map(|i| i.size), None);
    }

    #[test]
    fn the_drawn_arrow_is_opaque_at_its_tip_and_clear_beside_it() {
        let a = arrow(24);
        assert_eq!((a.width, a.height), (16, 24));
        let px = |x: u32, y: u32| {
            let i = ((y * a.width + x) * 4) as usize;
            drm_bytes(&a)[i + 3]
        };
        assert_eq!(px(0, 5), 0xff, "the left edge is drawn");
        assert_eq!(px(10, 2), 0, "right of the diagonal is clear");
    }
}
