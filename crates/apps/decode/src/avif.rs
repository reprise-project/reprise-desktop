//! An AVIF's first picture through rav1d, refused past a pixel budget, a heap budget or a
//! deadline.
//!
//! The container is read by [`crate::heif`]; each AV1 picture (a still, a sequence's first
//! sample, each tile of a grid, and the alpha plane) is decoded by rav1d's dav1d API on the
//! calling thread: one thread and a frame delay of one, so it starts no worker. rav1d takes no
//! allocator, so its heap is held by [`crate::heapcap`] as jxl-rs's is, and its frame size limit is
//! the pixel budget. rav1d cannot be stopped inside a picture, so the deadline is checked after
//! the container is read, between pictures and between its calls; past that the CPU time limit
//! and the program's kill stop it. rav1d's API is `extern "C"`, so a panic inside it aborts.
//!
//! YUV becomes RGB under `colr`'s CICP matrix and range (the AV1 sequence header's when there is
//! no `colr`), chroma upsampled bilinearly, samples of 10 and 12 bits kept until the 8-bit sRGB
//! is written. Primaries other than BT.709, PQ, HLG, linear and gamma transfers, and an RGB
//! matrix/TRC ICC profile, are turned to sRGB through [`ToSrgb`]; the SDR video transfers (BT.709,
//! BT.601, BT.2020) are shown as sRGB, as is a profile this cannot read. PQ's white is `clli`'s
//! brightest pixel, else `mdcv`'s peak, else 1000 nits.

use std::ffi::c_void;
use std::mem::MaybeUninit;
use std::ptr::NonNull;
use std::time::{Duration, Instant};

use jxl::api::{JxlPrimaries, JxlTransferFunction, JxlWhitePoint};
use rav1d::include::dav1d::data::Dav1dData;
use rav1d::include::dav1d::dav1d::{Dav1dContext, Dav1dSettings};
use rav1d::include::dav1d::headers::{
    DAV1D_PIXEL_LAYOUT_I400, DAV1D_PIXEL_LAYOUT_I420, DAV1D_PIXEL_LAYOUT_I422,
};
use rav1d::include::dav1d::picture::Dav1dPicture;
use rav1d::src::lib::{
    dav1d_close, dav1d_data_unref, dav1d_data_wrap, dav1d_default_settings, dav1d_flush,
    dav1d_get_picture, dav1d_open, dav1d_picture_unref, dav1d_send_data,
};
use rav1d::src::send_sync_non_null::SendSyncNonNull;

use crate::colour::ToSrgb;
use crate::heif::{self, Coded, Image, Nclx, Transform};
use crate::{Refused, Rgba, heapcap};

/// PQ's white when the file states no light level, in nits.
const PQ_WHITE: f32 = 1000.0;
/// The display HLG is shown on, in nits.
const HLG_DISPLAY: f32 = 1000.0;
/// What a decode may hold at once, per byte of the pixel budget: the RGBA, rav1d's picture (up to
/// 1.5 times the RGBA at 10 and 12 bits 4:4:4) and its film grain copy, or the RGBA and its
/// turned copy.
const HEAP_PER_PIXEL_BYTE: u64 = 4;
/// rav1d's own state beside its pictures, whatever their size, in bytes.
const DECODER_STATE: u64 = 4 << 20;
/// The largest sample of the RGB turned to sRGB through [`ToSrgb`].
const DEEP: u16 = 4095;

/// The heap a decode may hold under a pixel budget of `pixel_bytes`.
pub fn heap_budget(pixel_bytes: u64) -> u64 {
    (pixel_bytes.saturating_mul(HEAP_PER_PIXEL_BYTE)).saturating_add(DECODER_STATE)
}

/// The first picture of `bytes`, refused when it states more RGBA bytes than `pixel_bytes`, the
/// decode holds more than [`heap_budget`], or it takes longer than `time_limit`.
pub fn first_frame(bytes: &[u8], pixel_bytes: u64, time_limit: Duration) -> Result<Rgba, Refused> {
    let heap = usize::try_from(heap_budget(pixel_bytes)).unwrap_or(usize::MAX);
    let deadline = Instant::now() + time_limit;
    let decode = || {
        // A panic in the Rust around rav1d is a file it could not read; one inside rav1d aborts
        // before it gets here.
        std::panic::catch_unwind(|| decode(bytes, pixel_bytes, deadline))
            .unwrap_or(Err(Refused::Broken))
    };
    match heapcap::within(heap, decode) {
        Ok((Err(_), spent)) if spent.refused => Err(Refused::Memory),
        Ok((got, _)) => got,
        Err(heapcap::NotInstalled) => Err(Refused::Memory),
    }
}

fn decode(bytes: &[u8], pixel_bytes: u64, deadline: Instant) -> Result<Rgba, Refused> {
    let avif = heif::parse(bytes).ok_or(Refused::Broken)?;
    if Instant::now() >= deadline {
        return Err(Refused::Time);
    }
    let (w, h) = avif.size;
    if w == 0 || h == 0 {
        return Err(Refused::Broken);
    }
    if u64::from(w) * u64::from(h) * 4 > pixel_bytes {
        return Err(Refused::Size);
    }
    let frame_limit = u32::try_from(pixel_bytes / 4).unwrap_or(u32::MAX).max(1);
    let mut px = zeroed(w as usize * h as usize * 4)?;
    let mut out = Out {
        px: &mut px,
        width: w as usize,
        height: h as usize,
    };
    match &avif.alpha {
        Some(alpha) => each_picture(alpha, frame_limit, deadline, (w, h), |pic, at| {
            write_alpha(&mut out, pic, at)
        })?,
        None => out.px.iter_mut().skip(3).step_by(4).for_each(|a| *a = 255),
    }
    let mut turn: Option<Turn> = None;
    each_picture(&avif.colour, frame_limit, deadline, (w, h), |pic, at| {
        if turn.is_none() {
            turn = Some(Turn::of(&avif, pic)?);
        }
        let turn = turn.as_ref().ok_or(Refused::Broken)?;
        write_colour(
            &mut out,
            pic,
            at,
            turn,
            avif.premultiplied && avif.alpha.is_some(),
        );
        Ok(())
    })?;
    let mut rgba = Rgba {
        width: w,
        height: h,
        px,
    };
    for t in &avif.transforms {
        rgba = transform(rgba, *t)?;
    }
    Ok(rgba)
}

/// Decodes each picture of `image` in turn, each frame at most `frame_limit` pixels, and hands it
/// to `put` with where its top left goes: a single picture and a grid must be `size`, and a
/// grid's tiles must cover it exactly. Its decoder, and the pictures it keeps for reuse, are gone
/// when it returns.
fn each_picture(
    image: &Image<'_>,
    frame_limit: u32,
    deadline: Instant,
    size: (u32, u32),
    mut put: impl FnMut(&Picture, (usize, usize)) -> Result<(), Refused>,
) -> Result<(), Refused> {
    let dav1d = &mut Dav1d::open(frame_limit)?;
    match image {
        Image::Coded(c) => {
            let pic = dav1d.picture(c, deadline)?;
            if pic.size() != size {
                return Err(Refused::Broken);
            }
            put(&pic, (0, 0))
        }
        Image::Grid {
            cols,
            rows,
            size: (gw, gh),
            tiles,
        } => {
            if (*gw, *gh) != size {
                return Err(Refused::Broken);
            }
            let mut tile_size = None;
            for (i, tile) in tiles.iter().enumerate() {
                let pic = dav1d.picture(tile, deadline)?;
                let (tw, th) = pic.size();
                let first = *tile_size.get_or_insert((tw, th));
                let covers = |t: u32, n: u32, side: u32| {
                    u64::from(t) * u64::from(n) >= u64::from(side)
                        && u64::from(t) * u64::from(n - 1) < u64::from(side)
                };
                if first != (tw, th) || !covers(tw, *cols, *gw) || !covers(th, *rows, *gh) {
                    return Err(Refused::Broken);
                }
                let (col, row) = (i as u32 % cols, i as u32 / cols);
                put(&pic, ((col * tw) as usize, (row * th) as usize))?;
            }
            Ok(())
        }
    }
}

/// The RGBA being written: `width` x `height` pixels.
struct Out<'a> {
    px: &'a mut [u8],
    width: usize,
    height: usize,
}

/// How a picture's samples become RGB: its matrix and range, and the turn to sRGB.
struct Turn {
    matrix: Matrix,
    full_range: bool,
    to_srgb: Option<ToSrgb>,
}

#[derive(Clone, Copy)]
enum Matrix {
    /// The planes are G, B and R.
    Identity,
    YCgCo,
    /// Luma weights of red and blue.
    Ycc(f32, f32),
}

impl Turn {
    fn of(avif: &heif::Avif<'_>, pic: &Picture) -> Result<Turn, Refused> {
        let nclx = avif.nclx.unwrap_or_else(|| pic.nclx());
        let rgb = primaries(nclx.primaries);
        let matrix = match nclx.matrix {
            0 => Matrix::Identity,
            1 => Matrix::Ycc(0.2126, 0.0722),
            4 => Matrix::Ycc(0.30, 0.11),
            7 => Matrix::Ycc(0.212, 0.087),
            8 => Matrix::YCgCo,
            9 | 10 => Matrix::Ycc(0.2627, 0.0593),
            12 | 13 => {
                let (p, w) = rgb
                    .clone()
                    .unwrap_or((JxlPrimaries::SRGB, JxlWhitePoint::D65));
                match crate::colour::rgb_to_xyz(p.to_xy_coords(), w.to_xy_coords()) {
                    Some(m) => Matrix::Ycc(m[1][0] as f32, m[1][2] as f32),
                    None => Matrix::Ycc(0.299, 0.114),
                }
            }
            _ => Matrix::Ycc(0.299, 0.114),
        };
        let to_srgb = match avif.icc {
            Some(profile) => ToSrgb::icc(profile)?,
            None => {
                let (tf, white) = match nclx.transfer {
                    4 => (JxlTransferFunction::Gamma(1.0 / 2.2), 1.0),
                    5 => (JxlTransferFunction::Gamma(1.0 / 2.8), 1.0),
                    8 => (JxlTransferFunction::Linear, 1.0),
                    16 => {
                        let peak = (avif.max_cll.map(f32::from))
                            .or(avif.mastering_peak.map(|p| p as f32 / 10000.0))
                            .filter(|&p| p >= 1.0);
                        (JxlTransferFunction::PQ, peak.unwrap_or(PQ_WHITE))
                    }
                    17 => (JxlTransferFunction::DCI, 1.0),
                    18 => (JxlTransferFunction::HLG, HLG_DISPLAY),
                    _ => (JxlTransferFunction::SRGB, 1.0),
                };
                let (p, w) = rgb.unwrap_or((JxlPrimaries::SRGB, JxlWhitePoint::D65));
                match ToSrgb::named(&tf, Some((&p, &w)), white)? {
                    Some(t) => Some(t.at_max(DEEP, white)?),
                    None => None,
                }
            }
        };
        let to_srgb = match to_srgb {
            Some(t) if t.max != DEEP => Some(t.at_max(DEEP, 1.0)?),
            other => other,
        };
        Ok(Turn {
            matrix,
            full_range: nclx.full_range,
            to_srgb,
        })
    }
}

/// CICP colour primaries and white; None for BT.709 (sRGB's), unspecified and ones not named.
fn primaries(code: u16) -> Option<(JxlPrimaries, JxlWhitePoint)> {
    let c = |rx, ry, gx, gy, bx, by| JxlPrimaries::Chromaticities {
        rx,
        ry,
        gx,
        gy,
        bx,
        by,
    };
    let illuminant_c = JxlWhitePoint::Chromaticity {
        wx: 0.310,
        wy: 0.316,
    };
    Some(match code {
        4 => (c(0.67, 0.33, 0.21, 0.71, 0.14, 0.08), illuminant_c),
        5 => (c(0.64, 0.33, 0.29, 0.60, 0.15, 0.06), JxlWhitePoint::D65),
        6 | 7 => (
            c(0.630, 0.340, 0.310, 0.595, 0.155, 0.070),
            JxlWhitePoint::D65,
        ),
        8 => (c(0.681, 0.319, 0.243, 0.692, 0.145, 0.049), illuminant_c),
        9 => (JxlPrimaries::BT2100, JxlWhitePoint::D65),
        11 => (JxlPrimaries::P3, JxlWhitePoint::DCI),
        12 => (JxlPrimaries::P3, JxlWhitePoint::D65),
        22 => (
            c(0.630, 0.340, 0.295, 0.605, 0.155, 0.077),
            JxlWhitePoint::D65,
        ),
        _ => return None,
    })
}

/// A sample plane of a picture.
struct Plane<'p> {
    b: &'p [u8],
    stride: usize,
    deep: bool,
}

impl Plane<'_> {
    fn at(&self, x: usize, y: usize) -> u16 {
        if self.deep {
            let i = y * self.stride + 2 * x;
            u16::from_ne_bytes([self.b[i], self.b[i + 1]])
        } else {
            u16::from(self.b[y * self.stride + x])
        }
    }
}

/// The two samples a chroma plane subsampled by `shift` holds about luma position `x`, and the
/// second one's weight, the chroma centred between its luma samples.
fn taps(x: usize, shift: u32, len: usize) -> (usize, usize, f32) {
    if shift == 0 {
        return (x, x, 0.0);
    }
    let k = x / 2;
    let last = len - 1;
    if x.is_multiple_of(2) {
        (k.saturating_sub(1).min(last), k.min(last), 0.75)
    } else {
        (k.min(last), (k + 1).min(last), 0.25)
    }
}

fn write_colour(out: &mut Out<'_>, pic: &Picture, (x0, y0): (usize, usize), t: &Turn, prem: bool) {
    let (pw, ph) = pic.size();
    let (pw, ph) = (pw as usize, ph as usize);
    let (w, h) = (pw.min(out.width - x0), ph.min(out.height - y0));
    let bpc = pic.bpc();
    let max = ((1u32 << bpc) - 1) as f32;
    let k = (1u32 << (bpc - 8)) as f32;
    let (y_off, y_scale, c_scale) = if t.full_range {
        (0.0, 1.0 / max, 1.0 / max)
    } else {
        (16.0 * k, 1.0 / (219.0 * k), 1.0 / (224.0 * k))
    };
    let c_off = (1u32 << (bpc - 1)) as f32;
    let planes = pic.planes();
    let (sx, sy) = pic.shift();
    let (cw, ch) = ((pw + sx as usize) >> sx, (ph + sy as usize) >> sy);
    for y in 0..h {
        let (r0, r1, rw) = taps(y, sy, ch);
        for x in 0..w {
            let luma = (f32::from(planes[0].at(x, y)) - y_off) * y_scale;
            let (rgb_n, i) = (
                match &planes[1..] {
                    [u, v] => {
                        let (c0, c1, cwt) = taps(x, sx, cw);
                        let chroma = |p: &Plane| {
                            let top = f32::from(p.at(c0, r0)) * (1.0 - cwt)
                                + f32::from(p.at(c1, r0)) * cwt;
                            let bottom = f32::from(p.at(c0, r1)) * (1.0 - cwt)
                                + f32::from(p.at(c1, r1)) * cwt;
                            top * (1.0 - rw) + bottom * rw
                        };
                        let (cb, cr) = (chroma(u), chroma(v));
                        match t.matrix {
                            Matrix::Identity => {
                                let s = |v: f32| (v - y_off) * y_scale;
                                [s(cr), luma, s(cb)]
                            }
                            Matrix::YCgCo => {
                                let (cg, co) = ((cb - c_off) * c_scale, (cr - c_off) * c_scale);
                                let tmp = luma - cg;
                                [tmp + co, luma + cg, tmp - co]
                            }
                            Matrix::Ycc(kr, kb) => {
                                let (cb, cr) = ((cb - c_off) * c_scale, (cr - c_off) * c_scale);
                                let r = luma + 2.0 * (1.0 - kr) * cr;
                                let b = luma + 2.0 * (1.0 - kb) * cb;
                                let g = (luma - kr * r - kb * b) / (1.0 - kr - kb);
                                [r, g, b]
                            }
                        }
                    }
                    _ => [luma; 3],
                },
                4 * ((y0 + y) * out.width + x0 + x),
            );
            let a = out.px[i + 3];
            let rgb_n = if prem {
                match a {
                    0 => [0.0; 3],
                    a => rgb_n.map(|v| v * 255.0 / f32::from(a)),
                }
            } else {
                rgb_n
            };
            let rgb = match &t.to_srgb {
                Some(s) => {
                    let q = rgb_n.map(|v| (v.clamp(0.0, 1.0) * f32::from(DEEP)).round() as u16);
                    let p = s.pixel(q, DEEP);
                    [p[0], p[1], p[2]]
                }
                None => rgb_n.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8),
            };
            out.px[i..i + 3].copy_from_slice(&rgb);
        }
    }
}

fn write_alpha(out: &mut Out<'_>, pic: &Picture, (x0, y0): (usize, usize)) -> Result<(), Refused> {
    let (pw, ph) = pic.size();
    let (w, h) = (
        (pw as usize).min(out.width - x0),
        (ph as usize).min(out.height - y0),
    );
    let bpc = pic.bpc();
    let max = ((1u32 << bpc) - 1) as f32;
    let k = (1u32 << (bpc - 8)) as f32;
    let (off, scale) = if pic.nclx().full_range {
        (0.0, 255.0 / max)
    } else {
        (16.0 * k, 255.0 / (219.0 * k))
    };
    let planes = pic.planes();
    for y in 0..h {
        for x in 0..w {
            let a = (f32::from(planes[0].at(x, y)) - off) * scale;
            out.px[4 * ((y0 + y) * out.width + x0 + x) + 3] = a.round().clamp(0.0, 255.0) as u8;
        }
    }
    Ok(())
}

/// `rgba` with `t` applied.
fn transform(rgba: Rgba, t: Transform) -> Result<Rgba, Refused> {
    let (w, h) = (rgba.width as usize, rgba.height as usize);
    let mut px = rgba.px;
    match t {
        Transform::Turn(0) => {}
        Transform::FlipTopBottom => {
            for y in 0..h / 2 {
                let (top, bottom) = px.split_at_mut((h - 1 - y) * w * 4);
                top[y * w * 4..(y + 1) * w * 4].swap_with_slice(&mut bottom[..w * 4]);
            }
        }
        Transform::FlipLeftRight => {
            for row in px.chunks_exact_mut(w * 4) {
                for x in 0..w / 2 {
                    let (l, r) = row.split_at_mut((w - 1 - x) * 4);
                    l[x * 4..x * 4 + 4].swap_with_slice(&mut r[..4]);
                }
            }
        }
        Transform::Turn(q) => {
            let (nw, nh) = if q == 2 { (w, h) } else { (h, w) };
            let mut turned = zeroed(px.len())?;
            for y in 0..h {
                for x in 0..w {
                    let (nx, ny) = match q {
                        1 => (y, w - 1 - x),
                        2 => (w - 1 - x, h - 1 - y),
                        _ => (h - 1 - y, x),
                    };
                    let (from, to) = (4 * (y * w + x), 4 * (ny * nw + nx));
                    turned[to..to + 4].copy_from_slice(&px[from..from + 4]);
                }
            }
            return Ok(Rgba {
                width: nw as u32,
                height: nh as u32,
                px: turned,
            });
        }
    }
    Ok(Rgba {
        width: rgba.width,
        height: rgba.height,
        px,
    })
}

/// `n` zero bytes, or [`Refused::Memory`] when the heap budget refuses them.
fn zeroed(n: usize) -> Result<Vec<u8>, Refused> {
    let mut v = Vec::new();
    v.try_reserve_exact(n).map_err(|_| Refused::Memory)?;
    v.resize(n, 0);
    Ok(v)
}

/// A rav1d decoder: one thread and a frame delay of one. It logs to stderr.
struct Dav1d(Option<Dav1dContext>);

/// Leaves the bytes with their owner; rav1d is flushed of them before they go.
unsafe extern "C" fn keep(_: *const u8, _: Option<SendSyncNonNull<c_void>>) {}

const EAGAIN: i32 = -libc::EAGAIN;

impl Dav1d {
    fn open(frame_limit: u32) -> Result<Dav1d, Refused> {
        let mut s = MaybeUninit::<Dav1dSettings>::uninit();
        // SAFETY: dav1d_default_settings writes every field of `s`.
        let mut s = unsafe {
            dav1d_default_settings(NonNull::from(&mut s).cast());
            s.assume_init()
        };
        s.n_threads = 1;
        s.max_frame_delay = 1;
        s.frame_size_limit = frame_limit;
        s.all_layers = 0;
        let mut c = None;
        // SAFETY: `c` and `s` are live for the call; rav1d reads `s` and writes `c`.
        let r = unsafe { dav1d_open(Some(NonNull::from(&mut c)), Some(NonNull::from(&mut s))) };
        match (r.0, c) {
            (0, Some(c)) => Ok(Dav1d(Some(c))),
            (e, _) => Err(refusal(e)),
        }
    }

    /// The picture `coded` holds, rav1d flushed of its bytes before it returns.
    fn picture(&mut self, coded: &Coded<'_>, deadline: Instant) -> Result<Picture, Refused> {
        let joined;
        let bytes: &[u8] = match coded.0.as_slice() {
            [one] => one,
            many => {
                let len = many.iter().map(|e| e.len()).sum();
                let mut v = Vec::new();
                v.try_reserve_exact(len).map_err(|_| Refused::Memory)?;
                many.iter().for_each(|e| v.extend_from_slice(e));
                joined = v;
                &joined
            }
        };
        let got = self.decode(bytes, deadline);
        if let Some(c) = self.0 {
            // SAFETY: `c` is open; the flush drops every reference to `bytes`.
            unsafe { dav1d_flush(c) };
        }
        got
    }

    fn decode(&mut self, bytes: &[u8], deadline: Instant) -> Result<Picture, Refused> {
        let c = self.0.ok_or(Refused::Broken)?;
        let mut data = Dav1dData::default();
        let ptr = NonNull::new(bytes.as_ptr().cast_mut()).ok_or(Refused::Broken)?;
        // SAFETY: `ptr` is `bytes.len()` bytes that outlive the decode: `picture` flushes rav1d
        // of them before they can go, and `keep` frees nothing.
        let r = unsafe {
            dav1d_data_wrap(
                Some(NonNull::from(&mut data)),
                Some(ptr),
                bytes.len(),
                Some(keep),
                None,
            )
        };
        if r.0 != 0 {
            return Err(refusal(r.0));
        }
        let mut data = Data(data);
        let mut pic = Picture(MaybeUninit::zeroed());
        loop {
            if Instant::now() >= deadline {
                return Err(Refused::Time);
            }
            let left = data.0.sz;
            if left > 0 {
                // SAFETY: `c` is open and `data` is a live wrapped buffer.
                let r = unsafe { dav1d_send_data(Some(c), Some(NonNull::from(&mut data.0))) };
                if r.0 != 0 && r.0 != EAGAIN {
                    return Err(refusal(r.0));
                }
            }
            // SAFETY: `c` is open and `pic` is live for rav1d to write.
            let r = unsafe { dav1d_get_picture(Some(c), Some(NonNull::from(&mut pic.0).cast())) };
            match r.0 {
                0 => return Ok(pic),
                EAGAIN if left > 0 && data.0.sz < left => {}
                EAGAIN if left > 0 => return Err(Refused::Broken),
                EAGAIN => return Err(Refused::Broken),
                e => return Err(refusal(e)),
            }
        }
    }
}

impl Drop for Dav1d {
    fn drop(&mut self) {
        // SAFETY: closes the context opened in `open`, once.
        unsafe { dav1d_close(Some(NonNull::from(&mut self.0))) };
    }
}

fn refusal(e: i32) -> Refused {
    match -e {
        libc::ENOMEM => Refused::Memory,
        libc::ERANGE => Refused::Size,
        _ => Refused::Broken,
    }
}

/// Wrapped bytes given to rav1d, unreferenced when dropped.
struct Data(Dav1dData);

impl Drop for Data {
    fn drop(&mut self) {
        // SAFETY: `self.0` is a wrapped buffer or emptied by rav1d; unref accepts either.
        unsafe { dav1d_data_unref(Some(NonNull::from(&mut self.0))) };
    }
}

/// A decoded picture, unreferenced when dropped.
struct Picture(MaybeUninit<Dav1dPicture>);

impl Picture {
    fn get(&self) -> &Dav1dPicture {
        // SAFETY: a Picture is only returned once dav1d_get_picture filled it.
        unsafe { self.0.assume_init_ref() }
    }

    fn size(&self) -> (u32, u32) {
        let p = &self.get().p;
        (p.w.max(0) as u32, p.h.max(0) as u32)
    }

    fn bpc(&self) -> u32 {
        self.get().p.bpc.clamp(8, 16) as u32
    }

    /// The chroma planes' horizontal and vertical subsampling shifts.
    fn shift(&self) -> (u32, u32) {
        match self.get().p.layout {
            DAV1D_PIXEL_LAYOUT_I420 => (1, 1),
            DAV1D_PIXEL_LAYOUT_I422 => (1, 0),
            _ => (0, 0),
        }
    }

    /// The sequence header's colour description.
    fn nclx(&self) -> Nclx {
        let p = self.get();
        // SAFETY: a decoded picture's sequence header lives as long as the picture.
        match p.seq_hdr.map(|s| unsafe { s.as_ref() }) {
            Some(s) => Nclx {
                primaries: s.pri as u16,
                transfer: s.trc as u16,
                matrix: s.mtrx as u16,
                full_range: s.color_range != 0,
            },
            None => Nclx {
                primaries: 2,
                transfer: 2,
                matrix: 2,
                full_range: false,
            },
        }
    }

    /// Luma, then the two chroma planes unless the picture is monochrome.
    fn planes(&self) -> Vec<Plane<'_>> {
        let p = self.get();
        let (w, h) = self.size();
        let (w, h) = (w as usize, h as usize);
        let deep = self.bpc() > 8;
        let (sx, sy) = self.shift();
        let plane = |i: usize, stride: isize, pw: usize, ph: usize| {
            let data = p.data[i]?;
            let stride = usize::try_from(stride).ok()?;
            let row = pw * if deep { 2 } else { 1 };
            let len = stride.checked_mul(ph.checked_sub(1)?)?.checked_add(row)?;
            // SAFETY: rav1d's plane `i` holds `ph` rows `stride` bytes apart, each at least `row`
            // bytes, for as long as the picture.
            let b = unsafe { std::slice::from_raw_parts(data.as_ptr().cast::<u8>(), len) };
            Some(Plane { b, stride, deep })
        };
        let mut out: Vec<Plane<'_>> = plane(0, p.stride[0], w, h).into_iter().collect();
        if p.p.layout != DAV1D_PIXEL_LAYOUT_I400 {
            let (cw, ch) = ((w + sx as usize) >> sx, (h + sy as usize) >> sy);
            out.extend(plane(1, p.stride[1], cw, ch));
            out.extend(plane(2, p.stride[1], cw, ch));
        }
        out
    }
}

impl Drop for Picture {
    fn drop(&mut self) {
        // SAFETY: a zeroed or filled picture; unref leaves a zeroed one alone.
        unsafe { dav1d_picture_unref(Some(NonNull::from(&mut self.0).cast())) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::avif_pictures::*;
    use rav1e::prelude::ChromaSampling;

    const NO_HURRY: Duration = Duration::from_secs(60);
    const BUDGET: u64 = 1 << 24;

    fn rgba_of(rgb: &[[u16; 3]], bits: u8) -> Vec<u8> {
        let max = f64::from((1u32 << bits) - 1);
        let to8 = |v: u16| (f64::from(v) * 255.0 / max).round() as u8;
        rgb.iter()
            .flat_map(|p| [to8(p[0]), to8(p[1]), to8(p[2]), 255])
            .collect()
    }

    /// The largest difference between two RGBA buffers' samples.
    fn off(a: &[u8], b: &[u8]) -> u8 {
        assert_eq!(a.len(), b.len());
        a.iter()
            .zip(b)
            .map(|(x, y)| x.abs_diff(*y))
            .max()
            .unwrap_or(0)
    }

    /// The largest difference between two `w`-pixel-wide RGBA buffers' samples away from the
    /// edges of [`blocks`], where upsampled chroma mixes two blocks.
    fn off_inside(a: &[u8], b: &[u8], w: usize) -> u8 {
        let inside =
            |i: &usize| (2..6).contains(&(i / 4 % w % 8)) && (2..6).contains(&(i / 4 / w % 8));
        (0..a.len())
            .filter(inside)
            .map(|i| a[i].abs_diff(b[i]))
            .max()
            .unwrap_or(0)
    }

    /// `w`x`h` RGB in 8x8 blocks of one colour each, so subsampled chroma holds them inside.
    fn blocks(w: usize, h: usize, bits: u8) -> Vec<[u16; 3]> {
        let max = (1u32 << bits) - 1;
        (0..w * h)
            .map(|i| {
                let n = ((i % w / 8) * 7 + (i / w / 8) * 13) as u32;
                let n = n.wrapping_mul(2_654_435_761);
                [n >> 5, n >> 13, n >> 21].map(|v| (v % (max + 1)) as u16)
            })
            .collect()
    }

    fn decoded(file: &[u8]) -> Rgba {
        first_frame(file, BUDGET, NO_HURRY).unwrap()
    }

    #[test]
    fn an_8_bit_444_picture_decodes_to_its_colours() {
        let rgb = noise_rgb(48, 32, 8);
        let file = single(av1_rgb(48, 32, &rgb, Coding::EXACT), vec![ispe(48, 32)]);
        let p = decoded(&file);
        assert_eq!((p.width, p.height), (48, 32));
        let got = off(&p.px, &rgba_of(&rgb, 8));
        assert!(got <= 4, "YUV rounding only, {got} off");
    }

    #[test]
    fn a_10_and_12_bit_420_limited_range_picture_decodes_to_its_colours() {
        for (bits, mc) in [(10, 1), (12, 9)] {
            let c = Coding {
                bits,
                sampling: ChromaSampling::Cs420,
                full_range: false,
                cicp: (2, 2, mc),
                ..Coding::EXACT
            };
            let rgb = blocks(64, 40, bits);
            let file = single(
                av1_rgb(64, 40, &rgb, c),
                vec![ispe(64, 40), nclx(1, 13, u16::from(mc), false)],
            );
            let got = off_inside(&decoded(&file).px, &rgba_of(&rgb, bits), 64);
            assert!(got <= 2, "{bits}-bit: {got} off");
        }
    }

    #[test]
    fn a_422_picture_and_a_grey_one_decode() {
        let c = Coding {
            sampling: ChromaSampling::Cs422,
            ..Coding::EXACT
        };
        let rgb = blocks(32, 16, 8);
        let file = single(av1_rgb(32, 16, &rgb, c), vec![ispe(32, 16)]);
        let got = off_inside(&decoded(&file).px, &rgba_of(&rgb, 8), 32);
        assert!(got <= 4, "{got} off");
        let grey: Vec<u8> = (0..32 * 16).map(|i| (i * 3 % 256) as u8).collect();
        let file = single(av1_alpha(32, 16, &grey), vec![ispe(32, 16)]);
        let want: Vec<u8> = grey.iter().flat_map(|&g| [g, g, g, 255]).collect();
        let got = off(&decoded(&file).px, &want);
        assert!(got <= 2, "grey g becomes (g, g, g, 255): {got} off");
    }

    #[test]
    fn the_sequence_headers_colour_holds_without_colr() {
        let c = Coding {
            full_range: false,
            cicp: (1, 13, 1),
            ..Coding::EXACT
        };
        let rgb = noise_rgb(16, 16, 8);
        let file = single(av1_rgb(16, 16, &rgb, c), vec![ispe(16, 16)]);
        let got = off(&decoded(&file).px, &rgba_of(&rgb, 8));
        assert!(got <= 4, "{got} off");
    }

    fn with_alpha(premultiplied: bool) -> (Vec<u8>, Vec<u8>) {
        let (w, h) = (32, 24);
        let alpha: Vec<u8> = (0..w * h).map(|i| (i % w * 8) as u8).collect();
        let colour = [200u16, 100, 50];
        let stored: Vec<[u16; 3]> = alpha
            .iter()
            .map(|&a| match premultiplied {
                true => colour.map(|c| (u32::from(c) * u32::from(a) / 255) as u16),
                false => colour,
            })
            .collect();
        let items = [
            Item::av01(1, av1_rgb(w, h, &stored, Coding::EXACT), vec![ispe(32, 24)]),
            Item::av01(2, av1_alpha(w, h, &alpha), vec![ispe(32, 24), alpha_aux()]),
        ];
        let mut refs: Vec<(&[u8; 4], u32, &[u32])> = vec![(b"auxl", 2, &[1])];
        if premultiplied {
            refs.push((b"prem", 1, &[2]));
        }
        let want = alpha.iter().flat_map(|&a| [200, 100, 50, a]).collect();
        (still(&items, 1, &refs), want)
    }

    #[test]
    fn the_alpha_plane_decodes_and_premultiplied_colour_comes_out_divided() {
        let (file, want) = with_alpha(false);
        let got = decoded(&file).px;
        assert!(off(&got, &want) <= 2);
        let (file, want) = with_alpha(true);
        let got = decoded(&file).px;
        for (g, w) in got.as_chunks::<4>().0.iter().zip(want.as_chunks::<4>().0) {
            assert!(g[3].abs_diff(w[3]) <= 2, "{g:?} for {w:?}");
            if w[3] >= 64 {
                assert!(off(&g[..3], &w[..3]) <= 6, "{g:?} for {w:?}");
            }
        }
    }

    #[test]
    fn ravifs_files_decode() {
        let (w, h) = (40, 30);
        let rgba: Vec<u8> = (0..w * h)
            .flat_map(|i| {
                [
                    (i % w * 6) as u8,
                    (i / w * 8) as u8,
                    90,
                    (255 - i % 200) as u8,
                ]
            })
            .collect();
        for ten in [false, true] {
            let p = decoded(&ravif(w, h, &rgba, ten));
            assert_eq!((p.width, p.height), (w as u32, h as u32));
            let got = off(&p.px, &rgba);
            assert!(got <= 4, "ten bits {ten}: {got} off");
        }
    }

    #[test]
    fn rotation_and_mirroring_apply_in_their_order() {
        // 4x2: red, green / blue, white in its left and right halves of each row.
        let (r, g, b, w) = ([255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 255]);
        let rgb: Vec<[u16; 3]> = (0..8)
            .map(|i| match (i % 4 < 2, i < 4) {
                (true, true) => r,
                (false, true) => g,
                (true, false) => b,
                (false, false) => w,
            })
            .collect();
        let data = av1_rgb(4, 2, &rgb, Coding::EXACT);
        let corners = |props: Vec<Vec<u8>>| {
            let p = decoded(&single(data.clone(), [vec![ispe(4, 2)], props].concat()));
            let at = |x: u32, y: u32| {
                let i = 4 * (y * p.width + x) as usize;
                [p.px[i], p.px[i + 1], p.px[i + 2]].map(|v| v > 128)
            };
            let (mx, my) = (p.width - 1, p.height - 1);
            (
                (p.width, p.height),
                [at(0, 0), at(mx, 0), at(0, my), at(mx, my)],
            )
        };
        let (t, f) = (true, false);
        let (red, green, blue, white) = ([t, f, f], [f, t, f], [f, f, t], [t, t, t]);
        assert_eq!(corners(vec![]), ((4, 2), [red, green, blue, white]));
        assert_eq!(
            corners(vec![irot(1)]),
            ((2, 4), [green, white, red, blue]),
            "a quarter turn anticlockwise"
        );
        assert_eq!(corners(vec![imir(0)]), ((4, 2), [blue, white, red, green]));
        assert_eq!(corners(vec![imir(1)]), ((4, 2), [green, red, white, blue]));
        assert_eq!(
            corners(vec![irot(1), imir(1)]),
            ((2, 4), [white, green, blue, red]),
            "turned, then left and right exchanged"
        );
        assert_eq!(corners(vec![irot(3)]), ((2, 4), [blue, red, white, green]));
    }

    #[test]
    fn a_sequence_shows_its_first_frame() {
        let red = vec![[255u16, 0, 0]; 16 * 16];
        let blue = vec![[0u16, 0, 255]; 16 * 16];
        let frames = [red.clone(), blue].map(|f| av1_rgb(16, 16, &f, Coding::EXACT));
        let p = decoded(&sequence(16, 16, &frames));
        assert!(off(&p.px, &rgba_of(&red, 8)) <= 2);
    }

    #[test]
    fn a_grid_is_put_together_from_its_tiles_and_cropped() {
        let colours = [[255u16, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 0]];
        let mut items: Vec<Item> = colours
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let data = av1_rgb(32, 32, &vec![*c; 32 * 32], Coding::EXACT);
                Item::av01(i as u32 + 2, data, vec![ispe(32, 32)])
            })
            .collect();
        items.push(Item {
            id: 1,
            kind: *b"grid",
            data: grid(2, 2, 50, 40),
            props: vec![ispe(50, 40)],
        });
        let file = still(&items, 1, &[(b"dimg", 1, &[2, 3, 4, 5])]);
        let p = decoded(&file);
        assert_eq!((p.width, p.height), (50, 40));
        let at = |x: u32, y: u32| {
            let i = 4 * (y * 50 + x) as usize;
            [p.px[i], p.px[i + 1], p.px[i + 2]]
        };
        let corners = [at(0, 0), at(49, 0), at(0, 39), at(49, 39)];
        for (got, want) in corners.iter().zip(colours) {
            assert!(off(got, &want.map(|v| v as u8)) <= 2, "{corners:?}");
        }
        let short = still(&items, 1, &[(b"dimg", 1, &[2, 3, 4])]);
        assert_eq!(
            first_frame(&short, BUDGET, NO_HURRY).err(),
            Some(Refused::Broken)
        );
        items[4].data = grid(2, 2, 70, 40);
        let wide = still(&items, 1, &[(b"dimg", 1, &[2, 3, 4, 5])]);
        assert_eq!(
            first_frame(&wide, BUDGET, NO_HURRY).err(),
            Some(Refused::Broken),
            "a grid wider than its ispe"
        );
    }

    #[test]
    fn an_alpha_grid_larger_than_the_picture_is_unreadable() {
        let mut items = vec![
            Item::av01(
                1,
                av1_rgb(10, 10, &noise_rgb(10, 10, 8), Coding::EXACT),
                vec![ispe(10, 10)],
            ),
            Item {
                id: 2,
                kind: *b"grid",
                data: grid(4, 4, 16, 16),
                props: vec![ispe(16, 16), alpha_aux()],
            },
        ];
        let tiles: Vec<u32> = (3..19).collect();
        for &id in &tiles {
            items.push(Item::av01(
                id,
                av1_alpha(4, 4, &[128; 16]),
                vec![ispe(4, 4)],
            ));
        }
        let file = still(&items, 1, &[(b"auxl", 2, &[1]), (b"dimg", 2, &tiles)]);
        let far = Instant::now() + NO_HURRY;
        assert_eq!(decode(&file, BUDGET, far).err(), Some(Refused::Broken));
    }

    #[test]
    fn a_pq_picture_decodes_to_display_light_in_srgb() {
        // 203 nits is PQ 0.5807; at a brightest pixel of 1000 nits it is 0.203 of white, sRGB 124.
        let c = Coding {
            bits: 10,
            cicp: (9, 16, 9),
            ..Coding::EXACT
        };
        let code = (0.5807 * 1023.0f64).round() as u16;
        let rgb = vec![[code; 3]; 16 * 16];
        let props = vec![ispe(16, 16), nclx(9, 16, 9, true), clli(1000)];
        let p = decoded(&single(av1_rgb(16, 16, &rgb, c), props));
        assert!(
            p.px[..3].iter().all(|&v| v.abs_diff(124) <= 3),
            "203 nits at 1000 is sRGB 124: {:?}",
            &p.px[..4]
        );
    }

    #[test]
    fn a_display_p3_picture_decodes_to_srgb_by_cicp_or_icc() {
        use crate::colour::tests::{colorants_of, profile, srgb_para};
        let red_in_p3 = vec![[234u16, 51, 36]; 16 * 16];
        let data = av1_rgb(16, 16, &red_in_p3, Coding::EXACT);
        let p3 = colorants_of(&JxlPrimaries::P3, &JxlWhitePoint::D65);
        let srgb = colorants_of(&JxlPrimaries::SRGB, &JxlWhitePoint::D65);
        for (name, colr, turned) in [
            ("cicp", nclx(12, 13, 6, true), true),
            ("icc", icc(&profile(p3, &srgb_para())), true),
            ("srgb icc", icc(&profile(srgb, &srgb_para())), false),
        ] {
            let px = decoded(&single(data.clone(), vec![ispe(16, 16), colr])).px;
            let red = px[0] >= 247 && px[1] <= 8 && px[2] <= 8;
            let kept = off(&px[..3], &[234, 51, 36]) <= 2;
            assert!(if turned { red } else { kept }, "{name}: {:?}", &px[..3]);
        }
    }

    #[test]
    fn a_picture_past_the_pixel_budget_is_refused_before_decoding() {
        let file = single(vec![0x12, 0, 0x0a, 0x0a], vec![ispe(20_000, 20_000)]);
        let (got, spent) =
            heapcap::within(usize::MAX, || first_frame(&file, BUDGET, NO_HURRY)).unwrap();
        assert_eq!(got.err(), Some(Refused::Size));
        assert!(
            spent.peak < 1 << 20,
            "nothing of its size was held: {}",
            spent.peak
        );
        let data = av1_rgb(512, 512, &noise_rgb(512, 512, 8), Coding::EXACT);
        let lying = single(data, vec![ispe(16, 16)]);
        assert_eq!(
            first_frame(&lying, 256 * 256 * 4, NO_HURRY).err(),
            Some(Refused::Size),
            "rav1d holds the frame to the budget whatever ispe says"
        );
    }

    #[test]
    fn a_truncated_or_foreign_file_is_unreadable() {
        let file = single(
            av1_rgb(32, 32, &noise_rgb(32, 32, 8), Coding::EXACT),
            vec![ispe(32, 32)],
        );
        for cut in [file.len() - 10, file.len() / 2, 40, 0] {
            assert_eq!(
                first_frame(&file[..cut], BUDGET, NO_HURRY).err(),
                Some(Refused::Broken),
                "cut at {cut}"
            );
        }
        let mut hevc = file.clone();
        let at = hevc.windows(4).position(|w| w == b"av01").unwrap();
        hevc[at..at + 4].copy_from_slice(b"hvc1");
        assert_eq!(
            first_frame(&hevc, BUDGET, NO_HURRY).err(),
            Some(Refused::Broken)
        );
    }

    /// `w`x`w` RGB in 256-pixel squares of one colour each, which codes quickly.
    fn squares(w: usize, bits: u8) -> Vec<[u16; 3]> {
        let max = (1u32 << bits) - 1;
        (0..w * w)
            .map(|i| {
                let n = ((i % w / 256) * 3 + (i / w / 256) * 5) as u32 * 37;
                [n, n * 3, n * 7].map(|v| (v % (max + 1)) as u16)
            })
            .collect()
    }

    /// The pictures of `w`x`w` made to hold much while decoding, encoded side by side.
    fn heavy(w: usize) -> Vec<(&'static str, Vec<u8>)> {
        std::thread::scope(|s| {
            let made: Vec<_> = (0..4).map(|n| s.spawn(move || heavy_one(w, n))).collect();
            made.into_iter().map(|m| m.join().unwrap()).collect()
        })
    }

    fn heavy_one(w: usize, n: usize) -> (&'static str, Vec<u8>) {
        let deep = |bits, sampling| Coding {
            bits,
            sampling,
            quantizer: 40,
            ..Coding::EXACT
        };
        let picture = |w, c: Coding| av1_rgb(w, w, &squares(w, c.bits), c);
        let side = w as u32;
        match n {
            0 => {
                let data = picture(w, deep(8, ChromaSampling::Cs420));
                ("8-bit 4:2:0", single(data, vec![ispe(side, side)]))
            }
            1 => {
                let alpha: Vec<u8> = (0..w * w).map(|i| (i % w / 8) as u8).collect();
                let items = [
                    Item::av01(
                        1,
                        picture(w, deep(12, ChromaSampling::Cs444)),
                        vec![ispe(side, side), irot(1)],
                    ),
                    Item::av01(
                        2,
                        av1_alpha(w, w, &alpha),
                        vec![ispe(side, side), alpha_aux()],
                    ),
                ];
                let refs: [(&[u8; 4], u32, &[u32]); 2] = [(b"auxl", 2, &[1]), (b"prem", 1, &[2])];
                ("12-bit 4:4:4 with alpha, turned", still(&items, 1, &refs))
            }
            2 => {
                let half = w / 2;
                let tile = |id| {
                    let data = picture(half, deep(8, ChromaSampling::Cs420));
                    Item::av01(id, data, vec![ispe(side / 2, side / 2)])
                };
                let mut items: Vec<Item> = (2..6).map(tile).collect();
                items.push(Item {
                    id: 1,
                    kind: *b"grid",
                    data: grid(2, 2, w as u16, w as u16),
                    props: vec![ispe(side, side)],
                });
                ("grid", still(&items, 1, &[(b"dimg", 1, &[2, 3, 4, 5])]))
            }
            _ => {
                let c = Coding {
                    grain: true,
                    ..deep(12, ChromaSampling::Cs444)
                };
                (
                    "12-bit 4:4:4 film grain",
                    single(picture(w, c), vec![ispe(side, side)]),
                )
            }
        }
    }

    /// first_frame holds each decode to its own heap budget; the outer unlimited one only
    /// measures it.
    #[test]
    fn a_picture_at_nine_tenths_of_the_pixel_budget_decodes_within_its_heap_budget() {
        let w = 2048;
        let budget = (w * w * 4) as u64 * 10 / 9;
        for (name, file) in heavy(w) {
            let (got, spent) =
                heapcap::within(usize::MAX, || first_frame(&file, budget, NO_HURRY)).unwrap();
            eprintln!("{name}: peak {:.2}x", spent.peak as f64 / budget as f64);
            assert!(got.is_ok(), "{name}: {:?}", got.err());
            assert!(!spent.refused, "{name}");
            assert!(
                spent.peak as u64 <= heap_budget(budget),
                "{name} held {}",
                spent.peak
            );
        }
    }

    const SWEEP_SIDE: usize = 512;
    const SWEEP_BUDGET: u64 = 1 << 22;
    const SWEEP_TEST: &str =
        "avif::tests::a_decode_refused_any_one_large_allocation_fails_as_memory_or_aborts";
    /// Set in the sweep's child processes to the file to decode and the large allocation to
    /// refuse.
    const SWEEP_REFUSE: &str = "REPRISE_DECODE_AVIF_SWEEP";

    fn hash(px: &[u8]) -> u64 {
        use std::hash::{DefaultHasher, Hash, Hasher};
        let mut h = DefaultHasher::new();
        px.hash(&mut h);
        h.finish()
    }

    /// Each heavy picture decoded once for every large allocation it makes, that one refused, in
    /// a child process of its own: each gives the picture, [`Refused::Memory`], or an abort
    /// (where rav1d allocates infallibly), which reprise-decode's caller reads as too large.
    #[test]
    fn a_decode_refused_any_one_large_allocation_fails_as_memory_or_aborts() {
        use std::os::unix::process::ExitStatusExt;
        if let Some(asked) = std::env::var_os(SWEEP_REFUSE) {
            let asked = asked.into_string().unwrap();
            let (nth, path) = asked.split_once(' ').unwrap();
            let file = std::fs::read(path).unwrap();
            let refuse = |f| heapcap::within_refusing(usize::MAX, nth.parse().unwrap(), f);
            match refuse(|| first_frame(&file, SWEEP_BUDGET, NO_HURRY))
                .unwrap()
                .0
            {
                Ok(p) => println!("SWEPT picture {}", hash(&p.px)),
                Err(e) => println!("SWEPT {e:?}"),
            }
            return;
        }
        let (mut decoded, mut refused, mut aborted) = (0, 0, 0);
        let path = std::env::temp_dir().join(format!("reprise-avif-sweep-{}", std::process::id()));
        for (name, file) in heavy(SWEEP_SIDE) {
            std::fs::write(&path, &file).unwrap();
            let (whole, all) =
                heapcap::within(usize::MAX, || first_frame(&file, SWEEP_BUDGET, NO_HURRY)).unwrap();
            let whole = whole.unwrap_or_else(|e| panic!("{name}: {e:?}")).px;
            let picture = format!("SWEPT picture {}", hash(&whole));
            assert!(all.large > 0, "{name}");
            for nth in 0..all.large {
                let out = std::process::Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", SWEEP_TEST, "--nocapture", "--test-threads=1"])
                    .env(SWEEP_REFUSE, format!("{nth} {}", path.display()))
                    .stderr(std::process::Stdio::null())
                    .output()
                    .unwrap();
                let said = String::from_utf8_lossy(&out.stdout);
                let said = said.lines().find_map(|l| Some(&l[l.find("SWEPT")?..]));
                match (out.status.signal(), said) {
                    (Some(libc::SIGABRT), _) => aborted += 1,
                    (None, Some("SWEPT Memory")) => refused += 1,
                    (None, Some(p)) if p == picture => decoded += 1,
                    other => panic!("{name}: allocation {nth} refused: {other:?}"),
                }
            }
        }
        let _ = std::fs::remove_file(&path);
        eprintln!("refusals: {decoded} decoded, {refused} Memory, {aborted} aborted");
    }
}
