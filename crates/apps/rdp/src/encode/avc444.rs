//! The AVC444 colour split of MS-RDPEGFX 3.3.8.3.2 on the CPU: the main view carries luma and
//! subsampled chroma, the auxiliary view the chroma samples the main view drops.

use super::colour;
use super::hwmap;
use crate::stream::{ABGR8888, Rect, XBGR8888};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Views are whole macroblocks: the auxiliary luma interleaves U and V rows per 16 lines.
const MACROBLOCK: u32 = 16;

/// A 4:4:4 picture in the [`colour`] YUV, each plane `width * height` bytes.
#[cfg(test)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Planes444 {
    pub y: Vec<u8>,
    pub u: Vec<u8>,
    pub v: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// A 4:2:0 picture: `width * height` luma bytes, then `height / 2` rows of `width` bytes of
/// interleaved U and V.
#[cfg(test)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Nv12 {
    pub y: Vec<u8>,
    pub uv: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

#[cfg(test)]
impl Planes444 {
    /// Converts pixels of DRM `fourcc` (one of the stream's four) with rows `stride` bytes apart.
    pub fn from_rgb(
        data: &[u8],
        stride: usize,
        (width, height): (u32, u32),
        fourcc: u32,
    ) -> Planes444 {
        let rgb_first = matches!(fourcc, XBGR8888 | ABGR8888);
        let len = (width * height) as usize;
        let mut p = Planes444 {
            y: Vec::with_capacity(len),
            u: Vec::with_capacity(len),
            v: Vec::with_capacity(len),
            width,
            height,
        };
        for row in 0..height as usize {
            for &[a, g, c, _] in data[row * stride..][..width as usize * 4]
                .as_chunks::<4>()
                .0
            {
                let (r, b) = if rgb_first { (a, c) } else { (c, a) };
                let [y, u, v] = colour::yuv(r, g, b);
                p.y.push(y);
                p.u.push(u);
                p.v.push(v);
            }
        }
        p
    }
}

/// Pixels of DRM `fourcc` (one of the stream's four), rows `stride` bytes apart.
#[derive(Clone, Copy)]
pub struct Rgb<'a> {
    pub data: &'a [u8],
    pub stride: usize,
    pub size: (u32, u32),
    pub fourcc: u32,
}

/// A plane in memory, rows `stride` bytes apart.
pub struct PlaneMut<'a> {
    pub data: &'a mut [u8],
    pub stride: usize,
}

/// An NV12 view in memory, of [`view_size`].
pub struct Nv12Mut<'a> {
    pub y: PlaneMut<'a>,
    pub uv: PlaneMut<'a>,
}

/// `src`'s views in `main` and `aux` and its flat macroblocks. Main is 4:2:0 with rounded 2x2
/// chroma averages; aux carries the odd-column chroma and the B4/B5 rows inside `read`, and
/// main's bytes outside it.
pub fn views(src: Rgb, main: Nv12Mut, aux: Nv12Mut, read: Rect) -> Flat {
    let helpers = HELPERS.take(super::cpu_threads_for(src.size) - 1);
    views_in(src, main, aux, read, helpers.0, std::thread::Builder::new)
}

/// Band helper threads converting now, across every screen's encoder.
static HELPERS: std::sync::LazyLock<Budget> =
    std::sync::LazyLock::new(|| Budget::new(super::cpu_threads() - 1));

/// A count of threads shared by concurrent callers.
struct Budget {
    total: usize,
    busy: AtomicUsize,
}

/// Threads taken from a [`Budget`] until dropped.
struct Taken<'a>(usize, &'a Budget);

impl Budget {
    fn new(total: usize) -> Budget {
        Budget {
            total,
            busy: AtomicUsize::new(0),
        }
    }

    /// Up to `want` of the threads no one else holds.
    fn take(&self, want: usize) -> Taken<'_> {
        let mut got = 0;
        let _ = (self.busy).try_update(Ordering::Relaxed, Ordering::Relaxed, |busy| {
            got = want.min(self.total.saturating_sub(busy));
            Some(busy + got)
        });
        Taken(got, self)
    }
}

impl Drop for Taken<'_> {
    fn drop(&mut self) {
        self.1.busy.fetch_sub(self.0, Ordering::Relaxed);
    }
}

/// [`views`] in `helpers + 1` bands of macroblock rows. The caller converts every band no
/// helper thread built by `thread` picks up, so a failed spawn costs only time.
fn views_in(
    src: Rgb,
    main: Nv12Mut,
    aux: Nv12Mut,
    read: Rect,
    helpers: usize,
    thread: fn() -> std::thread::Builder,
) -> Flat {
    let (w, h) = view_size(src.size);
    let columns = w / MACROBLOCK;
    let mut blocks = vec![true; (columns * (h / MACROBLOCK)) as usize];
    if src.size.0 == 0 || src.size.1 == 0 {
        return Flat { columns, blocks };
    }
    let (w, h) = (w as usize, h as usize);
    for view in [&main, &aux] {
        for (p, rows) in [(&view.y, h), (&view.uv, h / 2)] {
            assert!(
                p.stride >= w && p.data.len() >= (rows - 1) * p.stride + w,
                "a view plane holds {rows} rows of {w} bytes"
            );
        }
    }
    let per = (h / MACROBLOCK as usize).div_ceil(helpers + 1);
    let rows = per * MACROBLOCK as usize;
    fn band(plane: PlaneMut<'_>, rows: usize) -> impl Iterator<Item = PlaneMut<'_>> {
        let stride = plane.stride;
        (plane.data.chunks_mut(rows * stride)).map(move |data| PlaneMut { data, stride })
    }
    let parts: Vec<Band> = (band(main.y, rows).zip(band(main.uv, rows / 2)))
        .zip(band(aux.y, rows).zip(band(aux.uv, rows / 2)))
        .zip(blocks.chunks_mut(per * columns as usize))
        .enumerate()
        .map(|(i, (((my, muv), (ay, auv)), blocks))| Band {
            top: i * rows,
            main: Nv12Mut { y: my, uv: muv },
            aux: Nv12Mut { y: ay, uv: auv },
            blocks,
        })
        .collect();
    let helpers = helpers.min(parts.len() - 1);
    let queue = Mutex::new(parts);
    let work = || {
        while let Some(b) = queue.lock().map_or(None, |mut q| q.pop()) {
            b.convert(src, read, w);
        }
    };
    std::thread::scope(|s| {
        for _ in 0..helpers {
            if thread().spawn_scoped(s, work).is_err() {
                break;
            }
        }
        work();
    });
    Flat { columns, blocks }
}

/// Whole macroblock rows of both views from `top` on, and their flat flags.
struct Band<'a> {
    top: usize,
    main: Nv12Mut<'a>,
    aux: Nv12Mut<'a>,
    blocks: &'a mut [bool],
}

impl Band<'_> {
    fn convert(self, src: Rgb, read: Rect, w: usize) {
        let Band {
            top,
            main,
            mut aux,
            blocks,
        } = self;
        let mb = MACROBLOCK as usize;
        let rows = blocks.len() / (w / mb) * mb;
        let last = src.size.1 as usize - 1;
        let (mut u, mut v) = ([vec![0; w], vec![0; w]], [vec![0; w], vec![0; w]]);
        let average = |a: u8, b: u8, c: u8, d: u8| {
            ((u32::from(a) + u32::from(b) + u32::from(c) + u32::from(d) + 2) / 4) as u8
        };
        for r in (0..rows).step_by(2) {
            for i in 0..2 {
                let luma = &mut main.y.data[(r + i) * main.y.stride..][..w];
                yuv_row(src, (top + r + i).min(last), luma, &mut u[i], &mut v[i]);
            }
            let (u0, u1, v0, v1) = (&u[0], &u[1], &v[0], &v[1]);
            let main_uv = &mut main.uv.data[r / 2 * main.uv.stride..][..w];
            let aux_uv = &mut aux.uv.data[r / 2 * aux.uv.stride..][..w];
            let flags = &mut blocks[r / mb * (w / mb)..][..w / mb];
            for x in (0..w).step_by(2) {
                main_uv[x] = average(u0[x], u0[x + 1], u1[x], u1[x + 1]);
                main_uv[x + 1] = average(v0[x], v0[x + 1], v1[x], v1[x + 1]);
                aux_uv[x] = u0[x + 1];
                aux_uv[x + 1] = v0[x + 1];
                let same =
                    |p: &[u8], q: &[u8]| p[x] == p[x + 1] && p[x] == q[x] && q[x] == q[x + 1];
                if !(same(u0, u1) && same(v0, v1)) {
                    flags[x / mb] = false;
                }
            }
            // B4 and B5: the odd row's U and V are auxiliary luma rows `half` and 8 + `half`.
            let (mb_top, half) = (r - r % mb, (r % mb) / 2);
            for (row, plane) in [(mb_top + half, u1), (mb_top + mb / 2 + half, v1)] {
                aux.y.data[row * aux.y.stride..][..w].copy_from_slice(plane);
            }
        }
        let rows_in =
            |y: usize| (read.y as usize..read.y.saturating_add(read.height) as usize).contains(&y);
        let (left, right) = (read.x as usize, read.x.saturating_add(read.width) as usize);
        for r in 0..rows {
            let (lo, hi) = match rows_in(top + r) {
                true => (left.min(w), right.min(w)),
                false => (w, w),
            };
            keep_outside(&main.y, &mut aux.y, r, lo, hi, w);
        }
        for c in 0..rows / 2 {
            let (lo, hi) = match rows_in(top + 2 * c) {
                true => (
                    left.next_multiple_of(2).min(w),
                    right.next_multiple_of(2).min(w),
                ),
                false => (w, w),
            };
            keep_outside(&main.uv, &mut aux.uv, c, lo, hi, w);
        }
    }
}

/// Row `r` of `aux` takes `main`'s bytes outside `lo..hi`.
fn keep_outside(main: &PlaneMut, aux: &mut PlaneMut, r: usize, lo: usize, hi: usize, w: usize) {
    let (m, a) = (
        &main.data[r * main.stride..][..w],
        &mut aux.data[r * aux.stride..][..w],
    );
    a[..lo].copy_from_slice(&m[..lo]);
    a[hi..].copy_from_slice(&m[hi..]);
}

/// Row `row` of `src` as Y, U and V, its last pixel repeated to the planes' width.
fn yuv_row(src: Rgb, row: usize, y: &mut [u8], u: &mut [u8], v: &mut [u8]) {
    let width = src.size.0 as usize;
    let rgb_first = matches!(src.fourcc, XBGR8888 | ABGR8888);
    let pixels = src.data[row * src.stride..][..width * 4].as_chunks::<4>().0;
    let planes = (y.iter_mut()).zip(u.iter_mut()).zip(v.iter_mut());
    for (&[a, g, c, _], ((y, u), v)) in pixels.iter().zip(planes) {
        let (r, b) = if rgb_first { (a, c) } else { (c, a) };
        [*y, *u, *v] = colour::yuv(r, g, b);
    }
    for p in [y, u, v] {
        let edge = p[width - 1];
        p[width..].fill(edge);
    }
}

/// The size both views are coded at: `size` rounded up to whole macroblocks.
pub fn view_size((width, height): (u32, u32)) -> (u32, u32) {
    (
        width.next_multiple_of(MACROBLOCK),
        height.next_multiple_of(MACROBLOCK),
    )
}

/// The main and auxiliary views of `src`, both [`view_size`]; the padding repeats the last
/// column and row.
#[cfg(test)]
pub fn split_v1(src: &Planes444) -> (Nv12, Nv12) {
    let (w, h) = view_size((src.width, src.height));
    let (y, u, v) = (
        pad(&src.y, src, w, h),
        pad(&src.u, src, w, h),
        pad(&src.v, src, w, h),
    );
    let (w, h) = (w as usize, h as usize);
    let at = |x: usize, y: usize| y * w + x;
    let average = |p: &[u8], x, y| {
        let sum = [at(x, y), at(x + 1, y), at(x, y + 1), at(x + 1, y + 1)]
            .map(|i| u32::from(p[i]))
            .iter()
            .sum::<u32>();
        ((sum + 2) / 4) as u8
    };
    let mut main_uv = Vec::with_capacity(w * h / 2);
    let mut aux_uv = Vec::with_capacity(w * h / 2);
    for y in (0..h).step_by(2) {
        for x in (0..w).step_by(2) {
            main_uv.extend([average(&u, x, y), average(&v, x, y)]);
            aux_uv.extend([u[at(x + 1, y)], v[at(x + 1, y)]]);
        }
    }
    let mut aux_y = Vec::with_capacity(w * h);
    for row in 0..h {
        let (is_v, from) = aux_luma_source(row);
        let plane = if is_v { &v } else { &u };
        aux_y.extend_from_slice(&plane[from * w..][..w]);
    }
    let (width, height) = (w as u32, h as u32);
    let main = Nv12 {
        y,
        uv: main_uv,
        width,
        height,
    };
    let aux = Nv12 {
        y: aux_y,
        uv: aux_uv,
        width,
        height,
    };
    (main, aux)
}

/// `plane` of `src` at `w` by `h`, the last column and row repeated; black for an empty `src`.
#[cfg(test)]
fn pad(plane: &[u8], src: &Planes444, w: u32, h: u32) -> Vec<u8> {
    let (sw, sh) = (src.width as usize, src.height as usize);
    if sw == 0 || sh == 0 {
        return vec![0; (w * h) as usize];
    }
    let mut out = Vec::with_capacity((w * h) as usize);
    for y in 0..h as usize {
        let line = &plane[y.min(sh - 1) * sw..][..sw];
        out.extend_from_slice(line);
        out.resize(out.len() + w as usize - sw, line[sw - 1]);
    }
    out
}

/// Whether auxiliary luma row `row` carries V, and which row: in each macroblock row, rows 0
/// to 7 hold the odd U rows (B4) and rows 8 to 15 the odd V rows (B5).
#[cfg(test)]
fn aux_luma_source(row: usize) -> (bool, usize) {
    let half = MACROBLOCK as usize / 2;
    let (top, within) = (row - row % (2 * half), row % (2 * half));
    let is_v = within >= half;
    (is_v, top + 2 * (within % half) + 1)
}

/// The 4:4:4 picture a client rebuilds from the two views, at the views' size. It always takes
/// the reverse filter's value, skipping the spec's optional threshold of 30.
#[cfg(test)]
pub fn combine_v1(main: &Nv12, aux: &Nv12) -> Planes444 {
    assert_eq!((main.width, main.height), (aux.width, aux.height));
    let (w, h) = (main.width as usize, main.height as usize);
    let mut u = vec![0; w * h];
    let mut v = vec![0; w * h];
    for row in 0..h {
        let (is_v, to) = aux_luma_source(row);
        let plane = if is_v { &mut v } else { &mut u };
        plane[to * w..][..w].copy_from_slice(&aux.y[row * w..][..w]);
    }
    for y in (0..h).step_by(2) {
        for x in (0..w).step_by(2) {
            let c = y / 2 * w + x;
            let (at, right, below) = (y * w + x, y * w + x + 1, (y + 1) * w + x);
            u[right] = aux.uv[c];
            v[right] = aux.uv[c + 1];
            // The reverse filter: the main view carries the block's average.
            for (plane, average) in [(&mut u, main.uv[c]), (&mut v, main.uv[c + 1])] {
                let rest: i32 = [right, below, below + 1]
                    .map(|i| i32::from(plane[i]))
                    .iter()
                    .sum();
                plane[at] = (4 * i32::from(average) - rest).clamp(0, 255) as u8;
            }
        }
    }
    Planes444 {
        y: main.y.clone(),
        u,
        v,
        width: main.width,
        height: main.height,
    }
}

/// Per macroblock of the views, whether each 2x2 block in it has one colour, so that the main
/// view's chroma alone is the picture's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Flat {
    pub columns: u32,
    pub blocks: Vec<bool>,
}

impl Flat {
    /// `src`'s macroblocks, its last column and row repeated to [`view_size`].
    #[cfg(test)]
    pub fn of(src: &Planes444) -> Flat {
        let (w, h) = view_size((src.width, src.height));
        let (columns, rows) = (w / MACROBLOCK, h / MACROBLOCK);
        let at = |x: u32, y: u32| {
            let (x, y) = (x.min(src.width.max(1) - 1), y.min(src.height.max(1) - 1));
            (y * src.width + x) as usize
        };
        let chroma = |x, y| src.u.get(at(x, y)).zip(src.v.get(at(x, y)));
        let block_flat = |x, y| {
            let c = chroma(x, y);
            [(x + 1, y), (x, y + 1), (x + 1, y + 1)]
                .iter()
                .all(|&(x, y)| chroma(x, y) == c)
        };
        let mut blocks = Vec::with_capacity((columns * rows) as usize);
        for my in 0..rows {
            for mx in 0..columns {
                let (x0, y0) = (mx * MACROBLOCK, my * MACROBLOCK);
                blocks.push(
                    (y0..y0 + MACROBLOCK)
                        .step_by(2)
                        .all(|y| (x0..x0 + MACROBLOCK).step_by(2).all(|x| block_flat(x, y))),
                );
            }
        }
        Flat { columns, blocks }
    }

    /// Whether every macroblock `regions` touch is flat.
    pub fn covers(&self, regions: &[Rect]) -> bool {
        let rows = self.blocks.len() as u32 / self.columns.max(1);
        regions.iter().all(|r| {
            let (x0, y0) = (r.x / MACROBLOCK, r.y / MACROBLOCK);
            let x1 =
                r.x.saturating_add(r.width)
                    .div_ceil(MACROBLOCK)
                    .min(self.columns);
            let y1 = r.y.saturating_add(r.height).div_ceil(MACROBLOCK).min(rows);
            (y0..y1).all(|y| (x0..x1).all(|x| self.blocks[(y * self.columns + x) as usize]))
        })
    }
}

/// The bounds of `regions` grown to whole macroblocks: the part of the auxiliary view clients
/// read.
pub fn read_rect(regions: &[Rect]) -> Rect {
    let x = regions.iter().map(|r| r.x).min().unwrap_or(0);
    let y = regions.iter().map(|r| r.y).min().unwrap_or(0);
    let right = regions.iter().map(|r| r.x + r.width).max().unwrap_or(0);
    let bottom = regions.iter().map(|r| r.y + r.height).max().unwrap_or(0);
    let x = x / MACROBLOCK * MACROBLOCK;
    let y = y / MACROBLOCK * MACROBLOCK;
    Rect {
        x,
        y,
        width: right.next_multiple_of(MACROBLOCK).saturating_sub(x),
        height: bottom.next_multiple_of(MACROBLOCK).saturating_sub(y),
    }
}

/// `aux` showing `main`'s picture outside `read` (whole macroblocks), which clients never read
/// there: a view predicted from the other then codes only what changed.
#[cfg(test)]
pub fn keep_main_outside(main: &Nv12, aux: &mut Nv12, read: Rect) {
    let w = main.width as usize;
    let inside = |x: u32, y: u32| {
        x >= read.x && x < read.x + read.width && y >= read.y && y < read.y + read.height
    };
    for y in 0..main.height {
        for x in 0..main.width {
            if !inside(x, y) {
                let at = y as usize * w + x as usize;
                aux.y[at] = main.y[at];
            }
        }
    }
    for y in (0..main.height).step_by(2) {
        for x in (0..main.width).step_by(2) {
            if !inside(x, y) {
                let at = y as usize / 2 * w + x as usize;
                aux.uv[at..at + 2].copy_from_slice(&main.uv[at..at + 2]);
            }
        }
    }
}

/// What the client updates from picture `n` after an IDR, inside the even size: all of it on the
/// IDR, else the damage in whole macroblocks, as clients pair aux rows from a region's top.
pub fn regions(size: (u32, u32), n: u32, damage: &[Rect]) -> Vec<Rect> {
    let (w, h) = hwmap::even(size);
    if n == 0 {
        return match w > 0 && h > 0 {
            true => vec![Rect::full((w, h))],
            false => Vec::new(),
        };
    }
    damage
        .iter()
        .filter_map(|r| {
            let (right, bottom) = (r.x.saturating_add(r.width), r.y.saturating_add(r.height));
            if r.x >= right.min(w) || r.y >= bottom.min(h) {
                return None;
            }
            let (x, y) = (r.x / MACROBLOCK * MACROBLOCK, r.y / MACROBLOCK * MACROBLOCK);
            let right = right.min(w).next_multiple_of(MACROBLOCK).min(w);
            let bottom = bottom.min(h).next_multiple_of(MACROBLOCK).min(h);
            Some(Rect {
                x,
                y,
                width: right - x,
                height: bottom - y,
            })
        })
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::stream::XRGB8888;

    /// xorshift64, so the pictures are the same on every run.
    struct Rng(u64);

    impl Rng {
        fn byte(&mut self) -> u8 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0.to_le_bytes()[3]
        }

        fn plane(&mut self, len: usize) -> Vec<u8> {
            (0..len).map(|_| self.byte()).collect()
        }
    }

    fn random((width, height): (u32, u32), seed: u64) -> Planes444 {
        let mut rng = Rng(seed);
        let len = (width * height) as usize;
        Planes444 {
            y: rng.plane(len),
            u: rng.plane(len),
            v: rng.plane(len),
            width,
            height,
        }
    }

    pub(crate) fn rgb(size: (u32, u32), pixel: impl Fn(u32, u32) -> (u8, u8, u8)) -> Planes444 {
        let (w, h) = size;
        let data: Vec<u8> = (0..h)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .flat_map(|(x, y)| {
                let (r, g, b) = pixel(x, y);
                [b, g, r, 0xff]
            })
            .collect();
        Planes444::from_rgb(&data, w as usize * 4, size, XRGB8888)
    }

    pub(crate) fn crop(p: &Planes444, (width, height): (u32, u32)) -> Planes444 {
        let rows = |plane: &[u8]| -> Vec<u8> {
            (0..height as usize)
                .flat_map(|y| &plane[y * p.width as usize..][..width as usize])
                .copied()
                .collect()
        };
        Planes444 {
            y: rows(&p.y),
            u: rows(&p.u),
            v: rows(&p.v),
            width,
            height,
        }
    }

    fn row(plane: &[u8], width: u32, y: u32) -> &[u8] {
        &plane[(y * width) as usize..][..width as usize]
    }

    /// `back` is `src` except at even-row, even-column chroma samples, which come back from the
    /// rounded 2x2 average the main view carries and may be off by 2.
    fn assert_returns(back: &Planes444, src: &Planes444) {
        assert_eq!((back.width, back.height), (src.width, src.height));
        assert_eq!(back.y, src.y, "luma comes back exactly");
        for (name, b, s) in [("U", &back.u, &src.u), ("V", &back.v, &src.v)] {
            for y in 0..src.height {
                for x in 0..src.width {
                    let at = (y * src.width + x) as usize;
                    let off = b[at].abs_diff(s[at]);
                    let most = if x % 2 == 0 && y % 2 == 0 { 2 } else { 0 };
                    assert!(
                        off <= most,
                        "{name} at ({x}, {y}) is {} for {}, off by at most {most}",
                        b[at],
                        s[at]
                    );
                }
            }
        }
    }

    #[test]
    fn split_then_combine_returns_the_picture() {
        for seed in 1..4 {
            let src = random((64, 48), seed);
            let (main, aux) = split_v1(&src);
            assert_returns(&combine_v1(&main, &aux), &src);
        }
    }

    #[test]
    fn the_split_matches_the_spec() {
        let n = |x: u32, y: u32| (y * 16 + x) as u8;
        let src = Planes444 {
            y: (0..256).map(|i| (i % 16 * 16 + i / 16) as u8).collect(),
            u: (0..16)
                .flat_map(|y| (0..16).map(move |x| n(x, y)))
                .collect(),
            v: (0..16)
                .flat_map(|y| (0..16).map(move |x| 255 - n(x, y)))
                .collect(),
            width: 16,
            height: 16,
        };
        let (main, aux) = split_v1(&src);
        assert_eq!(
            (main.width, main.height, aux.width, aux.height),
            (16, 16, 16, 16)
        );
        assert_eq!(main.y, src.y, "B1 is the luma");
        // The block at (2..4, 4..6): U 66, 67, 82, 83 and V 189, 188, 173, 172.
        let block = 2 * 16 + 2;
        assert_eq!(
            main.uv[block..block + 2],
            [75, 181],
            "B2 and B3: the 2x2 average, rounded"
        );
        for (i, want) in [(0, 1), (3, 7), (7, 15)] {
            let u: Vec<u8> = (0..16).map(|x| n(x, want)).collect();
            assert_eq!(
                row(&aux.y, 16, i),
                u,
                "B4: auxiliary luma row {i} is U row {want}"
            );
            let v: Vec<u8> = u.iter().map(|u| 255 - u).collect();
            assert_eq!(
                row(&aux.y, 16, 8 + i),
                v,
                "B5: auxiliary luma row {} is V row {want}",
                8 + i
            );
        }
        assert_eq!(
            row(&aux.uv, 16, 2),
            [
                65, 190, 67, 188, 69, 186, 71, 184, 73, 182, 75, 180, 77, 178, 79, 176
            ],
            "B6 and B7: auxiliary chroma row 2 is U and V of row 4, odd columns"
        );
        for r in 0..8 {
            for c in 0..8 {
                let at = (r * 16 + 2 * c) as usize;
                let (x, y) = (2 * c, 2 * r);
                let sum = |p: &[u8]| {
                    [(x, y), (x + 1, y), (x, y + 1), (x + 1, y + 1)]
                        .iter()
                        .map(|&(x, y)| u32::from(p[(y * 16 + x) as usize]))
                        .sum::<u32>()
                };
                let average = |p| ((sum(p) + 2) / 4) as u8;
                assert_eq!(main.uv[at..at + 2], [average(&src.u), average(&src.v)]);
                assert_eq!(aux.uv[at..at + 2], [n(x + 1, y), 255 - n(x + 1, y)]);
            }
        }
    }

    #[test]
    fn odd_sizes_are_padded_like_the_spec() {
        let size = (13, 21);
        let src = random(size, 7);
        let (main, aux) = split_v1(&src);
        assert_eq!((main.width, main.height), (16, 32), "whole macroblocks");
        assert_eq!((aux.width, aux.height), (16, 32));
        let padded = |plane: &[u8], y: u32| -> Vec<u8> {
            let line = row(plane, 13, y.min(20));
            line.iter().chain([line[12]; 3].iter()).copied().collect()
        };
        assert_eq!(
            row(&main.y, 16, 25),
            padded(&src.y, 25),
            "the last row and column repeat"
        );
        assert_eq!(
            row(&aux.y, 16, 16),
            padded(&src.u, 17),
            "B4 of the second macroblock row"
        );
        assert_eq!(
            row(&aux.y, 16, 18),
            padded(&src.u, 21),
            "U rows past the picture repeat"
        );
        assert_eq!(
            row(&aux.y, 16, 24),
            padded(&src.v, 17),
            "B5 of the second macroblock row"
        );
        assert_returns(&crop(&combine_v1(&main, &aux), size), &src);
    }

    #[test]
    fn from_rgb_converts_each_pixel_of_padded_rows_in_either_byte_order() {
        let (red, blue) = ([0, 0, 255, 0], [255, 0, 0, 0]);
        let data = [red, blue, [9; 4], red, red, [9; 4]].concat();
        let [ry, ru, rv] = colour::yuv(255, 0, 0);
        let [by, bu, bv] = colour::yuv(0, 0, 255);
        let p = Planes444::from_rgb(&data, 12, (2, 2), XRGB8888);
        assert_eq!(p.y, [ry, by, ry, ry]);
        assert_eq!(p.u, [ru, bu, ru, ru]);
        assert_eq!(p.v, [rv, bv, rv, rv]);
        let p = Planes444::from_rgb(&data, 12, (2, 2), ABGR8888);
        assert_eq!(p.y, [by, ry, by, by], "R, G, B, A in memory");
    }

    /// A view of `size` in planes whose rows are wider than it, filled with 0xAA.
    fn padded_view((w, h): (u32, u32)) -> [(Vec<u8>, usize); 2] {
        let (w, h) = (w as usize, h as usize);
        [(w + 7, h), (w + 3, h / 2)].map(|(stride, rows)| (vec![0xAA; stride * rows], stride))
    }

    fn read_view(planes: &[(Vec<u8>, usize); 2], (w, h): (u32, u32)) -> Nv12 {
        let rows = |(data, stride): &(Vec<u8>, usize), n: u32| -> Vec<u8> {
            (0..n as usize)
                .flat_map(|r| &data[r * stride..][..w as usize])
                .copied()
                .collect()
        };
        Nv12 {
            y: rows(&planes[0], h),
            uv: rows(&planes[1], h / 2),
            width: w,
            height: h,
        }
    }

    fn nv12([y, uv]: &mut [(Vec<u8>, usize); 2]) -> Nv12Mut<'_> {
        Nv12Mut {
            y: PlaneMut {
                data: &mut y.0,
                stride: y.1,
            },
            uv: PlaneMut {
                data: &mut uv.0,
                stride: uv.1,
            },
        }
    }

    /// Checks `views_in` on `helpers` against the reference split of a random picture with greys
    /// on the left, whose chroma is flat, and colours on the right.
    fn check_bands(
        cases: &[((u32, u32), Rect)],
        helpers: &[usize],
        thread: fn() -> std::thread::Builder,
    ) {
        let mut rng = Rng(11);
        for &(size, read) in cases {
            let (w, h) = (size.0 as usize, size.1 as usize);
            let stride = w * 4 + 12;
            let mut data = rng.plane(stride * h);
            for row in data.chunks_exact_mut(stride) {
                for px in row[..w * 4].as_chunks_mut::<4>().0[..w / 2].iter_mut() {
                    *px = [px[0]; 4];
                }
            }
            for fourcc in [XRGB8888, ABGR8888] {
                let planes = Planes444::from_rgb(&data, stride, size, fourcc);
                let (main, mut aux) = split_v1(&planes);
                keep_main_outside(&main, &mut aux, read);
                let flat = Flat::of(&planes);
                let view = view_size(size);
                for &n in helpers {
                    let (mut m, mut a) = (padded_view(view), padded_view(view));
                    let src = Rgb {
                        data: &data,
                        stride,
                        size,
                        fourcc,
                    };
                    let got = views_in(src, nv12(&mut m), nv12(&mut a), read, n, thread);
                    let at = format!("{size:?} {fourcc:#x} read {read:?} on {n} helpers");
                    assert_eq!(read_view(&m, view), main, "the main view of {at}");
                    assert_eq!(read_view(&a, view), aux, "the auxiliary view of {at}");
                    assert_eq!(got, flat, "the flat macroblocks of {at}");
                }
            }
        }
    }

    fn rect(x: u32, y: u32, width: u32, height: u32) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn banded_views_equal_the_split_at_odd_sizes() {
        let cases = [
            ((13, 21), rect(0, 0, 16, 32)),
            ((33, 18), rect(16, 0, 16, 32)),
            ((100, 50), rect(16, 16, 48, 32)),
            ((250, 171), rect(32, 48, 96, 64)),
            ((64, 64), rect(0, 0, 0, 0)),
            ((1, 1), rect(0, 0, 16, 16)),
            ((33, 18), rect(32, 16, 32, 32)),
            ((250, 171), rect(240, 160, 32, 32)),
        ];
        check_bands(&cases, &[0, 1, 2, 3, 8], std::thread::Builder::new);
    }

    #[test]
    fn bands_whose_threads_fail_to_start_are_converted_by_the_caller() {
        // A stack larger than the address space: every spawn fails.
        let failing = || std::thread::Builder::new().stack_size(1 << 48);
        check_bands(&[((250, 171), rect(32, 48, 96, 64))], &[1, 3], failing);
    }

    #[test]
    fn a_budget_hands_out_only_the_threads_no_one_holds() {
        let budget = Budget::new(3);
        let first = budget.take(5);
        assert_eq!(first.0, 3, "at most the budget");
        assert_eq!(budget.take(2).0, 0, "none while the first holds them all");
        drop(first);
        let again = budget.take(2);
        assert_eq!(again.0, 2, "returned on drop");
        assert_eq!(budget.take(2).0, 1, "the rest");
    }

    #[test]
    fn an_empty_picture_splits_into_black_views() {
        let empty = Planes444 {
            y: Vec::new(),
            u: Vec::new(),
            v: Vec::new(),
            width: 0,
            height: 0,
        };
        let (main, aux) = split_v1(&empty);
        assert_eq!((main.width, main.height, aux.y.len()), (0, 0, 0));
    }

    #[test]
    fn a_macroblock_is_flat_when_each_2x2_block_has_one_colour() {
        let red = (255, 0, 0);
        // Greys anywhere; red in a 2x2-aligned block of macroblock (0, 0), and a red column
        // at x 17 that splits blocks of macroblock (1, 0).
        let picture = rgb((40, 20), |x, y| match (x, y) {
            (4..6, 2..4) | (17, _) => red,
            _ => ((x * 6) as u8, (x * 6) as u8, (x * 6) as u8),
        });
        let flat = Flat::of(&picture);
        assert_eq!(flat.columns, 3);
        assert_eq!(flat.blocks, [true, false, true, true, false, true]);
        let rect = |x, y, width, height| Rect {
            x,
            y,
            width,
            height,
        };
        assert!(flat.covers(&[rect(0, 0, 16, 32)]));
        assert!(flat.covers(&[rect(32, 0, 16, 32), rect(0, 16, 16, 16)]));
        assert!(!flat.covers(&[rect(0, 0, 16, 16), rect(16, 16, 2, 2)]));
        let edge = rgb((33, 18), |x, _| if x == 32 { red } else { (9, 9, 9) });
        assert_eq!(
            Flat::of(&edge).blocks,
            [true, true, true, true, true, true],
            "the padding repeats the last column, so its blocks are flat"
        );
    }

    #[test]
    fn the_aux_view_shows_the_main_view_outside_the_read_rect() {
        let src = random((40, 40), 3);
        let (main, aux) = split_v1(&src);
        let regions = [
            Rect {
                x: 16,
                y: 0,
                width: 4,
                height: 2,
            },
            Rect {
                x: 20,
                y: 20,
                width: 10,
                height: 4,
            },
        ];
        let read = read_rect(&regions);
        assert_eq!(
            read,
            Rect {
                x: 16,
                y: 0,
                width: 16,
                height: 32
            }
        );
        let mut kept = aux.clone();
        keep_main_outside(&main, &mut kept, read);
        for y in 0..48 {
            for x in 0..48 {
                let at = (y * 48 + x) as usize;
                let inside = (16..32).contains(&x) && y < 32;
                let want = if inside { &aux } else { &main };
                assert_eq!(kept.y[at], want.y[at], "luma at ({x}, {y})");
                if x % 2 == 0 && y % 2 == 0 {
                    let c = (y / 2 * 48 + x) as usize;
                    assert_eq!(kept.uv[c..c + 2], want.uv[c..c + 2], "chroma at ({x}, {y})");
                }
            }
        }
    }

    #[test]
    fn regions_are_whole_macroblocks_inside_the_even_picture() {
        let rect = |x, y, width, height| Rect {
            x,
            y,
            width,
            height,
        };
        assert_eq!(
            regions((101, 51), 0, &[]),
            [rect(0, 0, 100, 50)],
            "the IDR: the whole even picture"
        );
        assert_eq!(
            regions((101, 51), 3, &[rect(17, 20, 1, 1), rect(90, 40, 11, 11)]),
            [rect(16, 16, 16, 16), rect(80, 32, 20, 18)]
        );
        assert_eq!(regions((101, 51), 3, &[rect(100, 0, 1, 51)]), []);
        assert_eq!(
            regions(
                (64, 64),
                1,
                &[rect(u32::MAX - 1, 0, 2, 2), rect(8, 8, u32::MAX, 1)]
            ),
            [rect(0, 0, 64, 16)],
            "edges past u32 saturate"
        );
    }
}
