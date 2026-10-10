//! Flow: evenly spaced streamlines of a smooth noise-angle field, drawn as
//! tapered strokes weighted by a tone field. Inside a disc the field turns a
//! quarter turn, so the disc shows by stroke direction alone; strokes are cut
//! at its edge, and those outside run at partial opacity so the disc leads.

use std::f32::consts::PI;

use super::noise::Rng;
use super::noise::form::{Field, fbm};
use super::tone::{
    C, Ramp, hex, lit, mix, mul, oklch, quantise, sat as clamp01, smooth as smoothstep, to_srgb,
};
use super::{Variant, each, par_bands};

pub(super) fn render(w: usize, h: usize, v: Variant, seed: u32) -> Vec<u32> {
    let pal = match v {
        Variant::Named(n) => named(n),
        Variant::Hue { hue, dark } => from_hue(hue as f32, dark),
        Variant::Glass => named("night"),
    };
    quantise(&draw(w, h, &pal, seed), w, seed)
}

fn dist(x: f32, y: f32, cx: f32, cy: f32) -> f32 {
    ((x - cx).powi(2) + (y - cy).powi(2)).sqrt()
}

/// A palette from an OKLCH hue: light has a mid-tone ground with pale ink and a warm disc;
/// dark a near-black ground with ink in the hue and a brighter warm disc.
fn from_hue(hue: f32, dark: bool) -> FlowPalette {
    let s = |l: f32, c: f32, h: f32| oklch(l, c, h).map(to_srgb);
    // The disc is gold, unless the hue is near gold: then its opposite.
    let warm = if (lit(hue, 180.0) - hue).abs() < 40.0 {
        hue + 180.0
    } else {
        80.0
    };
    if dark {
        FlowPalette {
            bg: [(0.0, s(0.17, 0.02, hue)), (1.0, s(0.22, 0.035, hue))],
            ink: [
                (0.0, s(0.45, 0.10, lit(hue, -10.0))),
                (0.55, s(0.68, 0.13, hue)),
                (1.0, s(0.72, 0.12, lit(hue, -40.0))),
            ],
            disc: [(0.0, s(0.80, 0.11, warm)), (1.0, s(0.84, 0.10, warm))],
            ink_a: 0.55,
        }
    } else {
        FlowPalette {
            bg: [(0.0, s(0.36, 0.07, hue)), (1.0, s(0.44, 0.08, hue))],
            ink: [
                (0.0, s(0.62, 0.08, hue)),
                (0.6, s(0.80, 0.07, lit(hue, 10.0))),
                (1.0, s(0.94, 0.03, lit(hue, 20.0))),
            ],
            disc: [(0.0, s(0.90, 0.07, warm)), (1.0, s(0.80, 0.12, warm))],
            ink_a: 0.6,
        }
    }
}

struct FlowPalette {
    bg: [(f32, C); 2],
    ink: [(f32, C); 3],
    disc: [(f32, C); 2],
    ink_a: f32,
}

fn named(v: &str) -> FlowPalette {
    match v {
        "night" => FlowPalette {
            bg: [(0.0, hex(0x16161e)), (1.0, hex(0x1f2233))],
            ink: [
                (0.0, hex(0x3d59a1)),
                (0.55, hex(0x7aa2f7)),
                (1.0, hex(0xbb9af7)),
            ],
            disc: [(0.0, hex(0xffb07a)), (1.0, hex(0xf0c27a))],
            ink_a: 0.55,
        },
        "ink" => FlowPalette {
            bg: [(0.0, hex(0xf3eee4)), (1.0, hex(0xe9e2d5))],
            ink: [
                (0.0, hex(0x1d2a3a)),
                (0.6, hex(0x24384d)),
                (1.0, hex(0x2f4a5f)),
            ],
            disc: [(0.0, hex(0x1f6f78)), (1.0, hex(0x2a8a8a))],
            ink_a: 0.7,
        },
        "teal" => FlowPalette {
            bg: [(0.0, hex(0x0c4247)), (1.0, hex(0x14585a))],
            ink: [
                (0.0, hex(0x4f9a90)),
                (0.6, hex(0x8cc7b8)),
                (1.0, hex(0xd6eee3)),
            ],
            disc: [(0.0, hex(0xf0d9a0)), (1.0, hex(0xe5b865))],
            ink_a: 0.6,
        },
        _ => FlowPalette {
            bg: [(0.0, hex(0x17110f)), (1.0, hex(0x231915))],
            ink: [
                (0.0, hex(0x6b3a24)),
                (0.6, hex(0xc36a3a)),
                (1.0, hex(0xf0b072)),
            ],
            disc: [(0.0, hex(0xf6ead2)), (1.0, hex(0xe8dcc4))],
            ink_a: 0.6,
        },
    }
}

struct Stroke {
    pts: Vec<(f32, f32)>,
    in_disc: bool,
}

fn draw(w: usize, h: usize, pal: &FlowPalette, seed: u32) -> Vec<C> {
    let aspect = w as f32 / h as f32;
    let (hw, hh) = (aspect * 0.5 + 0.02, 0.52);
    let s = seed;
    let (cx, cy, cr) = (0.42, -0.08, 0.25);
    let angle = Field::build((-hw, -hh), (hw, hh), 1.0 / 200.0, |x, y| {
        let n = fbm(x * 0.95 + 4.0, y * 0.95 - 2.0, 3, s ^ 71);
        -0.2 + 0.35 * y + 2.3 * n
    });
    let tone = Field::build((-hw, -hh), (hw, hh), 1.0 / 100.0, |x, y| {
        let n = fbm(x * 0.9 - 7.0, y * 0.9 + 3.0, 3, s ^ 97);
        clamp01(0.5 + 1.2 * n + 0.25 * y)
    });
    let in_disc = |x: f32, y: f32| dist(x, y, cx, cy) < cr;
    let dir = |x: f32, y: f32| {
        let mut a = angle.at(x, y);
        if in_disc(x, y) {
            a += PI * 0.5;
        }
        (a.cos(), a.sin())
    };
    // Evenly spaced streamlines (separation test against a point grid).
    let dsep = 0.0078;
    let dtest = dsep * 0.55;
    let step = 0.0016;
    let gw = ((2.0 * hw) / dsep).ceil() as usize + 1;
    let gh = ((2.0 * hh) / dsep).ceil() as usize + 1;
    let mut grid: Vec<Vec<(f32, f32, u32)>> = vec![Vec::new(); gw * gh];
    let cell = |x: f32, y: f32| -> Option<(usize, usize)> {
        let i = ((x + hw) / dsep).floor();
        let j = ((y + hh) / dsep).floor();
        (i >= 0.0 && j >= 0.0 && (i as usize) < gw && (j as usize) < gh)
            .then_some((i as usize, j as usize))
    };
    let free = |grid: &Vec<Vec<(f32, f32, u32)>>, x: f32, y: f32, d: f32, own: u32| -> bool {
        let Some((ci, cj)) = cell(x, y) else {
            return false;
        };
        for j in cj.saturating_sub(1)..(cj + 2).min(gh) {
            for i in ci.saturating_sub(1)..(ci + 2).min(gw) {
                for &(px, py, id) in &grid[j * gw + i] {
                    if id != own && (px - x).powi(2) + (py - y).powi(2) < d * d {
                        return false;
                    }
                }
            }
        }
        true
    };
    let mut rng = Rng(seed as u64 ^ 0xf10e);
    let mut seeds: Vec<(f32, f32)> = Vec::new();
    {
        let n = (2.0 * hw / dsep) as usize;
        let m = (2.0 * hh / dsep) as usize;
        for j in 0..m {
            for i in 0..n {
                seeds.push((
                    -hw + (i as f32 + rng.f()) * dsep,
                    -hh + (j as f32 + rng.f()) * dsep,
                ));
            }
        }
        for i in (1..seeds.len()).rev() {
            let k = (rng.next_u64() % (i as u64 + 1)) as usize;
            seeds.swap(i, k);
        }
    }
    let mut strokes: Vec<Stroke> = Vec::new();
    let max_steps = 900;
    for &(sx, sy) in &seeds {
        if !free(&grid, sx, sy, dsep, u32::MAX) {
            continue;
        }
        let side = in_disc(sx, sy);
        let id = strokes.len() as u32;
        let mut halves: [Vec<(f32, f32)>; 2] = [Vec::new(), Vec::new()];
        for (h, sign) in [1.0f32, -1.0].iter().enumerate() {
            let (mut x, mut y) = (sx, sy);
            let mut prev = None::<(f32, f32)>;
            for _ in 0..max_steps {
                let (mut dx, mut dy) = dir(x, y);
                if let Some((px0, py0)) = prev {
                    if dx * px0 + dy * py0 < 0.0 {
                        dx = -dx;
                        dy = -dy;
                    }
                } else {
                    dx *= sign;
                    dy *= sign;
                }
                let (mx, my) = (x + dx * step * 0.5, y + dy * step * 0.5);
                let (mut ex, mut ey) = dir(mx, my);
                if ex * dx + ey * dy < 0.0 {
                    ex = -ex;
                    ey = -ey;
                }
                let (nx, ny) = (x + ex * step, y + ey * step);
                if nx.abs() > hw || ny.abs() > hh || in_disc(nx, ny) != side {
                    if in_disc(nx, ny) != side {
                        halves[h].push((nx, ny));
                    }
                    break;
                }
                if !free(&grid, nx, ny, dtest, id) {
                    break;
                }
                halves[h].push((nx, ny));
                prev = Some((ex, ey));
                x = nx;
                y = ny;
            }
        }
        let mut pts: Vec<(f32, f32)> = halves[1].iter().rev().copied().collect();
        pts.push((sx, sy));
        pts.extend(halves[0].iter().copied());
        if (pts.len() as f32) * step < 0.03 {
            continue;
        }
        for &(x, y) in pts.iter().step_by(2) {
            if let Some((i, j)) = cell(x, y) {
                grid[j * gw + i].push((x, y, id));
            }
        }
        strokes.push(Stroke { pts, in_disc: side });
    }

    // Per pixel, the strongest stroke coverage and its colour. Every band of rows
    // walks the segments in the same order, so ties resolve alike at any thread count.
    let scale = h as f32; // picture units -> px
    let ink = Ramp::new(&pal.ink);
    let disc_r = Ramp::new(&pal.disc);
    let base_w = 0.0034; // picture units at full tone
    struct Seg {
        a: (f32, f32),
        b: (f32, f32),
        r: f32,
        a_k: f32,
        alpha: f32,
        in_disc: bool,
        c: C,
        rows: (usize, usize),
        cols: (usize, usize),
    }
    let mut segs: Vec<Seg> = Vec::new();
    for st in &strokes {
        let n = st.pts.len();
        let total = n as f32;
        for k in 0..n - 1 {
            let (ax, ay) = st.pts[k];
            let (bx, by) = st.pts[k + 1];
            let t = (k as f32 + 0.5) / total;
            let taper = smoothstep(0.0, 0.12, t) * smoothstep(0.0, 0.12, 1.0 - t);
            let tn = tone.at(ax, ay);
            let tn = if st.in_disc { 0.45 + 0.35 * tn } else { tn };
            let alpha = if st.in_disc { 1.0 } else { pal.ink_a };
            let wdt = base_w * (0.12 + 0.88 * tn * tn) * (0.35 + 0.65 * taper);
            let c = if st.in_disc {
                disc_r.at(clamp01(0.5 + (ay - cy) / cr * 0.5))
            } else {
                ink.at(clamp01(0.5 + ax / (2.0 * hw) + 0.25 * (tn - 0.5)))
            };
            let (ax, ay) = (ax * scale + w as f32 * 0.5, ay * scale + h as f32 * 0.5);
            let (bx, by) = (bx * scale + w as f32 * 0.5, by * scale + h as f32 * 0.5);
            let r = (wdt * scale * 0.5).max(0.35);
            let a_k = (r / 0.5).min(1.0);
            let r = r.max(0.5);
            let x0 = (ax.min(bx) - r - 1.0).floor().max(0.0) as usize;
            let x1 = ((ax.max(bx) + r + 1.0).ceil() as usize).min(w);
            let y0 = (ay.min(by) - r - 1.0).floor().max(0.0) as usize;
            let y1 = ((ay.max(by) + r + 1.0).ceil() as usize).min(h);
            segs.push(Seg {
                a: (ax, ay),
                b: (bx, by),
                r,
                a_k,
                alpha,
                in_disc: st.in_disc,
                c,
                rows: (y0, y1),
                cols: (x0, x1),
            });
        }
    }
    let mut ink_px: Vec<(f32, C)> = vec![(0.0, [0.0; 3]); w * h];
    par_bands(&mut ink_px, w, |first, band| {
        let last = first + band.len() / w;
        for sg in &segs {
            let (y0, y1) = (sg.rows.0.max(first), sg.rows.1.min(last));
            if y0 >= y1 {
                continue;
            }
            let ((ax, ay), (bx, by), r) = (sg.a, sg.b, sg.r);
            let (dx, dy) = (bx - ax, by - ay);
            let l2 = (dx * dx + dy * dy).max(1e-9);
            for yy in y0..y1 {
                let py = yy as f32 + 0.5;
                for xx in sg.cols.0..sg.cols.1 {
                    let pxx = xx as f32 + 0.5;
                    let tt = (((pxx - ax) * dx + (py - ay) * dy) / l2).clamp(0.0, 1.0);
                    let (qx, qy) = (ax + tt * dx - pxx, ay + tt * dy - py);
                    let d = (qx * qx + qy * qy).sqrt();
                    let mut cv = clamp01(r + 0.5 - d) * sg.a_k;
                    if cv <= 0.0 {
                        continue;
                    }
                    let ux = (pxx - w as f32 * 0.5) / scale;
                    let uy = (py - h as f32 * 0.5) / scale;
                    let sd = (dist(ux, uy, cx, cy) - cr) * scale;
                    let side_cov = if sg.in_disc {
                        clamp01(0.5 - sd)
                    } else {
                        clamp01(0.5 + sd)
                    };
                    cv *= side_cov * sg.alpha;
                    let o = &mut band[(yy - first) * w + xx];
                    if cv > o.0 {
                        *o = (cv, sg.c);
                    }
                }
            }
        }
    });
    let bg = Ramp::new(&pal.bg);
    each(w, h, |i, j, x, y| {
        let r = dist(x, y, cx, cy);
        let vign = 1.0 - 0.12 * smoothstep(0.3, 1.1, dist(x, y, 0.1, 0.0));
        let b = mul(bg.at(clamp01(0.5 + y * 0.8 - x * 0.2)), vign);
        let inside = clamp01(0.5 - (r - cr) * scale);
        let b = mix(b, bg.at(1.0), inside * 0.6);
        let (cov, col) = ink_px[j * w + i];
        mix(b, col, cov)
    })
}
