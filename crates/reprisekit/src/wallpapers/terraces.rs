//! Terraces: hills cut into contour terraces, seen from above in a low raking light.

use super::land::Img;
use super::noise::hash_land;
use super::noise::land::{Field, fbm, noise, rnd};
use super::tone::{C, hex, lerp, mix, mul, mulc, quantise_grain, sat, smooth};
use super::{Variant, par_rows};

pub(super) fn render(w: usize, h: usize, v: Variant, seed: u32) -> Vec<u32> {
    let var = match v {
        Variant::Named(n) => n,
        _ => "spring",
    };
    let p = variant(var);
    let img = draw(w, h, &p, seed);
    quantise_grain(&img.px, w, p.grain, seed)
}

struct Terraces {
    fields: [C; 5],
    flood: C,
    flood_hi: C,
    flood_amt: f32,
    lit: C,
    shade: C,
    hedge: C,
    shadow_len: f32,
    river: C,
    river_deep: C,
    bank: C,
    tree: C,
    tree_lit: C,
    cloud_shadow: f32,
    light: [f32; 2],
    grain: f32,
}

fn variant(var: &str) -> Terraces {
    let base = Terraces {
        fields: [
            hex(0x93BE64),
            hex(0x7FAE5A),
            hex(0xA2C76E),
            hex(0x8AB862),
            hex(0xD8CC66),
        ],
        flood: hex(0x7DB6C6),
        flood_hi: hex(0xC8E2E0),
        flood_amt: 0.5,
        lit: hex(0xF4F4D0),
        shade: hex(0x2F5240),
        hedge: hex(0x3D6640),
        shadow_len: 1.8,
        river: hex(0x6FB3B0),
        river_deep: hex(0x2F7682),
        bank: hex(0xEBE2BC),
        tree: hex(0x2A4E36),
        tree_lit: hex(0x5F9152),
        cloud_shadow: 0.2,
        light: [-0.6, -0.8],
        grain: 0.02,
    };
    match var {
        "harvest" => Terraces {
            fields: [
                hex(0xDDAA4E),
                hex(0xD39E48),
                hex(0xE4BC66),
                hex(0xC99446),
                hex(0xB0623A),
            ],
            flood: hex(0x86AAAA),
            flood_hi: hex(0xD4DECE),
            flood_amt: 0.2,
            lit: hex(0xFFF0C4),
            shade: hex(0x6A3A26),
            hedge: hex(0x5E5A2A),
            river: hex(0x4F8E8C),
            river_deep: hex(0x2A5C66),
            bank: hex(0xF0E0B4),
            tree: hex(0x4A5226),
            tree_lit: hex(0x8A9442),
            ..base
        },
        "dusk" => Terraces {
            fields: [
                hex(0x8A6CA0),
                hex(0x7E6298),
                hex(0x967AA8),
                hex(0x8466A0),
                hex(0xC08AA4),
            ],
            flood: hex(0xC88A8C),
            flood_hi: hex(0xF0B08A),
            flood_amt: 0.25,
            lit: hex(0xFFBE82),
            shade: hex(0x2A1C4A),
            hedge: hex(0x3E2C5E),
            shadow_len: 3.4,
            river: hex(0xF09A68),
            river_deep: hex(0xB85E5E),
            bank: hex(0xF4C8A0),
            tree: hex(0x281C42),
            tree_lit: hex(0x9A6080),
            cloud_shadow: 0.16,
            light: [-0.94, -0.34],
            ..base
        },
        "frost" => Terraces {
            fields: [
                hex(0xE6ECF4),
                hex(0xDCE4EF),
                hex(0xEEF2F8),
                hex(0xE0E8F2),
                hex(0xC8D4E6),
            ],
            flood: hex(0x9FB4D2),
            flood_hi: hex(0xDCE6F4),
            flood_amt: 0.35,
            lit: hex(0xFFFFFF),
            shade: hex(0x5E74A0),
            hedge: hex(0x7A8CB0),
            shadow_len: 2.4,
            river: hex(0x34466C),
            river_deep: hex(0x1E2A48),
            bank: hex(0xF6F8FC),
            tree: hex(0x2A3A58),
            tree_lit: hex(0x5A6C90),
            cloud_shadow: 0.14,
            ..base
        },
        "blue" => Terraces {
            fields: [
                hex(0x2D60CC),
                hex(0x2858C0),
                hex(0x3468D6),
                hex(0x2F5FC8),
                hex(0x6E9AE6),
            ],
            flood: hex(0x6A96E2),
            flood_hi: hex(0xC0D2F2),
            flood_amt: 0.35,
            lit: hex(0xE6EEFA),
            shade: hex(0x0A2268),
            hedge: hex(0x143A90),
            river: hex(0xDCE6F4),
            river_deep: hex(0xA6BCE2),
            bank: hex(0xF2F5FB),
            tree: hex(0x2E6A2E),
            tree_lit: hex(0x6EAE4A),
            ..base
        },
        _ => base,
    }
}

fn draw(w: usize, h: usize, p: &Terraces, seed: u32) -> Img {
    let mut img = Img::new(w, h);
    let hf = h as f32;
    let uv = |x: usize, y: usize| {
        (
            (x as f32 + 0.5 - w as f32 * 0.5) / hf,
            (y as f32 + 0.5) / hf,
        )
    };
    let px = 1.0 / hf;
    let step = (hf / 300.0).max(2.0);
    let half = w as f32 * 0.5;
    let rv = |u: f32| 0.6 + 0.13 * (u * 2.3 + 0.4).sin() + 0.05 * fbm(u * 1.4, 9.0, 3, seed + 300);
    let rw = |u: f32| 0.011 + 0.004 * noise(u * 3.0, 4.0, seed + 304);
    let hgt = Field::new(w, h, step, |x, y| {
        let (u, v) = ((x - half) / hf, y / hf);
        let qx = fbm(u * 0.9, v * 0.9, 3, seed + 301);
        let qy = fbm(u * 0.9 + 5.2, v * 0.9 + 1.3, 3, seed + 302);
        let n = fbm(u * 1.15 + 0.8 * qx, v * 1.15 + 0.8 * qy, 5, seed + 303) * 0.75 + 0.5;
        let d = ((v - rv(u)) / 0.2).abs();
        let floor = smooth(0.0, 1.0, d);
        0.06 + n * lerp(0.4, 1.0, floor) + 0.1 * floor
    });
    let divs = Field::new(w, h, step, |x, y| {
        let (u, v) = ((x - half) / hf, y / hf);
        fbm(u * 2.2, v * 2.2, 2, seed + 310)
    });
    let forest = Field::new(w, h, step * 2.0, |x, y| {
        let (u, v) = ((x - half) / hf, y / hf);
        fbm(u * 2.0, v * 2.0, 3, seed + 320)
    });
    let clouds = Field::new(w, h, step * 2.0, |x, y| {
        let (u, v) = ((x - half) / hf, y / hf);
        let n = fbm(
            u * 1.0 + 0.3 * fbm(u, v, 2, seed + 331),
            v * 1.5,
            4,
            seed + 330,
        );
        smooth(0.08, 0.4, n)
    });
    let mut hb = vec![0f32; w * h];
    let mut ab = vec![0f32; w * h];
    par_rows(&mut hb, w, |y, row| {
        for (x, o) in row.iter_mut().enumerate() {
            *o = hgt.at(x as f32 + 0.5, y as f32 + 0.5);
        }
    });
    par_rows(&mut ab, w, |y, row| {
        for (x, o) in row.iter_mut().enumerate() {
            *o = divs.at(x as f32 + 0.5, y as f32 + 0.5);
        }
    });
    const LEVELS: f32 = 19.0;
    let rv_col: Vec<f32> = (0..w).map(|x| rv(uv(x, 0).0)).collect();
    let rw_col: Vec<f32> = (0..w).map(|x| rw(uv(x, 0).0)).collect();
    const DIVS: f32 = 6.0;
    let l = {
        let n = (p.light[0] * p.light[0] + p.light[1] * p.light[1]).sqrt();
        [p.light[0] / n, p.light[1] / n]
    };
    let riser = 0.0026f32;
    let lip = 0.0010f32;
    let hedge_w = 0.0007f32;
    let gradient = |b: &[f32], x: usize, y: usize| {
        let gx = (b[y * w + (x + 1).min(w - 1)] - b[y * w + x.saturating_sub(1)]) * 0.5 * hf;
        let gy = (b[(y + 1).min(h - 1) * w + x] - b[y.saturating_sub(1) * w + x]) * 0.5 * hf;
        (gx, gy)
    };
    par_rows(&mut img.px, w, |y, row| {
        for x in 0..w {
            let i = y * w + x;
            let (u, v) = uv(x, y);
            let hc = hb[i];
            let (gxu, gyu) = gradient(&hb, x, y);
            let gm = (gxu * gxu + gyu * gyu).sqrt().max(1e-4);
            let (dx, dy) = (-gxu / gm, -gyu / gm);
            let facing = dx * l[0] + dy * l[1];
            let t = hc * LEVELS;
            let level = t.floor();
            let frac = t - level;
            let d_up = (1.0 - frac) / (LEVELS * gm);
            let d_dn = frac / (LEVELS * gm);
            let a = ab[i] * DIVS + level * 0.37;
            let field = a.floor();
            let fa = a - field;
            let (ax, ay) = gradient(&ab, x, y);
            let am = (ax * ax + ay * ay).sqrt().max(1e-4) * DIVS;
            let d_hedge = fa.min(1.0 - fa) / am;
            let k = hash_land(level as i32, field as i32, seed + 340);
            let mut c = p.fields[[0, 1, 2, 3, 0, 1, 2, 4][(k % 8) as usize]];
            let flooded = ((k >> 8) & 255) as f32 / 255.0 < p.flood_amt * smooth(0.42, 0.25, hc);
            if flooded {
                let sk =
                    sat(0.5 - 0.6 * (u * l[0] + v * l[1])
                        + 0.12 * noise(u * 2.0, v * 6.0, seed + 341));
                c = mix(p.flood, p.flood_hi, sk);
            } else {
                // Furrows along the contour, faded where they would alias.
                let rows = 6.0;
                let spacing_px = 1.0 / (LEVELS * rows * gm * px);
                let fade =
                    smooth(3.0, 6.0, spacing_px) * if (k >> 16) & 1 == 0 { 1.0 } else { 0.0 };
                let tri = ((t * rows).fract() - 0.5).abs() * 2.0;
                c = mul(c, 1.0 - 0.06 * fade * tri);
            }
            let broad = facing * smooth(0.0, 2.5, gm);
            c = mix(
                c,
                if broad > 0.0 { p.lit } else { p.shade },
                broad.abs() * 0.3,
            );
            let hl = sat((hedge_w - d_hedge) / px + 0.5);
            c = mix(c, p.hedge, hl * 0.75);
            if facing < 0.0 {
                let reach = riser * (1.0 + p.shadow_len * -facing);
                let s = sat((reach - d_up) / px + 0.5);
                c = mix(c, mulc(c, mix(p.shade, [1.0; 3], 0.35)), s * 0.85);
            }
            let wall = sat((riser - d_up) / px + 0.5);
            let wall_c = if facing >= 0.0 {
                mix(c, p.lit, 0.3 + 0.55 * facing)
            } else {
                mix(c, p.shade, 0.45 - 0.45 * facing)
            };
            c = mix(c, wall_c, wall);
            let lipc = sat((lip - d_dn) / px + 0.5) * sat(-facing * 1.5 + 0.3);
            c = mix(c, p.lit, lipc * 0.7);
            let dr = (v - rv_col[x]).abs() - rw_col[x];
            if dr < 0.01 {
                let bank = sat((0.0035 - dr) / px + 0.5);
                c = mix(c, p.bank, bank);
                let wet = sat(-dr / px + 0.5);
                if wet > 0.0 {
                    let depth = smooth(0.0, 0.009, -dr);
                    let mut wc = mix(p.river, p.river_deep, depth);
                    let rip = noise(u * 40.0, v * 160.0, seed + 342);
                    wc = mix(wc, p.flood_hi, smooth(0.45, 0.9, rip) * 0.3);
                    c = mix(c, wc, wet);
                }
            }
            row[x] = c;
        }
    });
    let cell = 0.0058f32;
    let gw = (w as f32 * px / cell).ceil() as i32 + 3;
    let gh = (1.0 / cell).ceil() as i32 + 3;
    let u0 = -(w as f32 * 0.5) * px;
    let cells: Vec<Option<[f32; 3]>> = (0..gh)
        .flat_map(|j| (0..gw).map(move |i| (i, j)))
        .map(|(i, j)| {
            let s = seed + 350;
            let tx = u0 + (i as f32 - 1.0 + 0.15 + 0.7 * rnd(i, j, s)) * cell;
            let ty = (j as f32 - 1.0 + 0.15 + 0.7 * rnd(i, j, s + 1)) * cell;
            let (dx_px, dy_px) = (tx * hf + half, ty * hf);
            let f = forest.at(dx_px, dy_px);
            let dr = (ty - rv(tx)).abs() - rw(tx);
            let dens =
                smooth(0.2, 0.38, f) + 0.8 * smooth(0.012, 0.005, dr) * smooth(0.0035, 0.006, dr);
            if dr < 0.005 || rnd(i, j, s + 2) > dens {
                return None;
            }
            Some([tx, ty, cell * (0.42 + 0.3 * rnd(i, j, s + 3))])
        })
        .collect();
    let sh = [-l[0] * cell * 0.55, -l[1] * cell * 0.55];
    par_rows(&mut img.px, w, |y, row| {
        for (x, out) in row.iter_mut().enumerate() {
            let (u, v) = uv(x, y);
            let ci = ((u - u0) / cell).floor() as i32 + 1;
            let cj = (v / cell).floor() as i32 + 1;
            let mut shadow = 0.0f32;
            let mut crown: Option<(f32, f32, f32)> = None;
            for j in cj - 1..=cj + 1 {
                for i in ci - 1..=ci + 1 {
                    if i < 0 || j < 0 || i >= gw || j >= gh {
                        continue;
                    }
                    let Some([tx, ty, r]) = cells[(j * gw + i) as usize] else {
                        continue;
                    };
                    let ds = ((u - tx - sh[0]).powi(2) + (v - ty - sh[1]).powi(2)).sqrt();
                    shadow = shadow.max(sat((r * 1.05 - ds) / px + 0.5));
                    let d = ((u - tx).powi(2) + (v - ty).powi(2)).sqrt();
                    let cov = sat((r - d) / px + 0.5);
                    // Later rows (lower on screen) sit in front.
                    if cov > 0.0 && crown.is_none_or(|(_, _, cy)| ty > cy) {
                        let dl = ((u - tx - l[0] * r * 0.4).powi(2)
                            + (v - ty - l[1] * r * 0.4).powi(2))
                        .sqrt();
                        let lit = sat((r * 0.6 - dl) / px + 0.5);
                        crown = Some((cov, lit, ty));
                    }
                }
            }
            if shadow == 0.0 && crown.is_none() {
                continue;
            }
            let mut c = *out;
            c = mix(c, mulc(c, mix(p.shade, [1.0; 3], 0.3)), shadow * 0.8);
            if let Some((cov, lit, _)) = crown {
                c = mix(c, mix(p.tree, p.tree_lit, lit * 0.8), cov);
            }
            *out = c;
        }
    });
    if p.cloud_shadow > 0.0 {
        par_rows(&mut img.px, w, |y, row| {
            for (x, c) in row.iter_mut().enumerate() {
                let cs = clouds.at(x as f32 + 0.5, y as f32 + 0.5) * p.cloud_shadow * 2.0;
                if cs > 0.0 {
                    *c = mix(*c, mulc(*c, mix(p.shade, [1.0; 3], 0.5)), cs);
                }
            }
        });
    }
    img
}
