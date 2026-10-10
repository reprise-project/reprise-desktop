//! Ridges: eight folds of hills recede down a valley toward a low sun, each paler with distance,
//! with mist in the valley floors and spruce on the nearest folds.

use super::land::{Img, SS, Sky, col_min, sil_cov, tops_for, trees};
use super::noise::land::{Field, fbm, ridged};
use super::tone::{C, hex, lerp, mix, mul, quantise_grain, screen, smooth};
use super::{Variant, par_rows};

pub(super) fn render(w: usize, h: usize, v: Variant, seed: u32) -> Vec<u32> {
    let var = match v {
        Variant::Named(n) => n,
        _ => "dawn",
    };
    let p = variant(var, seed);
    let img = draw(w, h, &p, seed);
    quantise_grain(&img.px, w, p.grain, seed)
}

struct Ridges {
    sky: Sky,
    far: C,
    near: C,
    fog: f32,
    mist: f32,
    mist_c: C,
    wisp: f32,
    rim: C,
    rim_w: f32,
    valley_u: f32,
    grain: f32,
}

fn variant(var: &str, seed: u32) -> Ridges {
    let sky = Sky {
        zen: hex(0x27305E),
        mid: hex(0x9A7FA8),
        hor: hex(0xFFC99A),
        hor_v: 0.58,
        sun: [0.09, 0.468],
        sun_r: 0.021,
        sun_core: hex(0xFFF6E2),
        glow: hex(0xFFD49A),
        glow_k: 8.0,
        halo: hex(0xF0957E),
        halo_k: 2.0,
        bloom: 0.45,
        rays: 0.35,
        cloud: 0.55,
        cloud_lit: hex(0xFFD2AE),
        cloud_dark: hex(0x8E7AA6),
        cloud_v: [0.1, 0.36],
        stars: 0.0,
        milky: 0.0,
        seed,
    };
    let base = Ridges {
        sky,
        far: hex(0xB18FB0),
        near: hex(0x1C1430),
        fog: 0.72,
        mist: 0.6,
        mist_c: hex(0xFFDCC4),
        wisp: 0.75,
        rim: hex(0xFFD0A0),
        rim_w: 0.0016,
        valley_u: 0.08,
        grain: 0.016,
    };
    match var {
        "day" => Ridges {
            sky: Sky {
                zen: hex(0x3A68A8),
                mid: hex(0x86AACA),
                hor: hex(0xDDE6E4),
                sun: [-0.6, -0.3],
                sun_r: 0.0,
                glow: hex(0xFFF4D4),
                glow_k: 3.0,
                halo: hex(0xB8D0E4),
                halo_k: 1.6,
                bloom: 0.0,
                rays: 0.0,
                cloud: 0.4,
                cloud_lit: hex(0xF6F7F4),
                cloud_dark: hex(0xB4C4D2),
                ..base.sky
            },
            far: hex(0x9AB2C2),
            near: hex(0x172C2E),
            fog: 0.7,
            mist: 0.5,
            mist_c: hex(0xF0F3EE),
            rim: hex(0x303030),
            rim_w: 0.0012,
            ..base
        },
        "dusk" => Ridges {
            sky: Sky {
                zen: hex(0x1F1D48),
                mid: hex(0xA4507A),
                hor: hex(0xFF9A58),
                sun: [0.08, 0.472],
                sun_r: 0.028,
                sun_core: hex(0xFFE8B4),
                glow: hex(0xFFA850),
                glow_k: 6.5,
                halo: hex(0xE25868),
                halo_k: 1.7,
                bloom: 0.55,
                rays: 0.55,
                cloud: 0.75,
                cloud_lit: hex(0xFFB07A),
                cloud_dark: hex(0x6A3A6C),
                ..base.sky
            },
            far: hex(0xC86C84),
            near: hex(0x150C22),
            fog: 0.68,
            mist_c: hex(0xFFB088),
            rim: hex(0xFFB868),
            ..base
        },
        "night" => Ridges {
            sky: Sky {
                zen: hex(0x040716),
                mid: hex(0x0D1636),
                hor: hex(0x26365E),
                sun: [0.55, 0.16],
                sun_r: 0.016,
                sun_core: hex(0xF2F0E6),
                glow: hex(0x8AA0D0),
                glow_k: 14.0,
                halo: hex(0x283A6C),
                halo_k: 3.0,
                bloom: 0.0,
                rays: 0.0,
                cloud: 0.25,
                cloud_lit: hex(0x8090B8),
                cloud_dark: hex(0x1A2242),
                stars: 1.0,
                milky: 1.0,
                ..base.sky
            },
            far: hex(0x2C3C64),
            near: hex(0x03050C),
            fog: 0.5,
            mist: 0.45,
            mist_c: hex(0x6676A6),
            wisp: 0.5,
            rim: hex(0x9AAAD8),
            rim_w: 0.0012,
            grain: 0.014,
            ..base
        },
        "blue" => Ridges {
            sky: Sky {
                zen: hex(0x082684),
                mid: hex(0x2C5EC8),
                hor: hex(0xB2CCF0),
                sun: [0.09, 0.47],
                sun_r: 0.019,
                sun_core: hex(0xFFFEF2),
                glow: hex(0xF4F0C4),
                glow_k: 9.0,
                halo: hex(0x86AAE8),
                halo_k: 2.0,
                bloom: 0.4,
                rays: 0.3,
                cloud: 0.45,
                cloud_lit: hex(0xEAF0FA),
                cloud_dark: hex(0x3A62BC),
                ..base.sky
            },
            far: hex(0x7494DA),
            near: hex(0x061650),
            fog: 0.7,
            mist_c: hex(0xDCE6F8),
            rim: hex(0xF0F4C8),
            ..base
        },
        _ => base,
    }
}

fn draw(w: usize, h: usize, p: &Ridges, seed: u32) -> Img {
    let mut img = Img::new(w, h);
    let hf = h as f32;
    let px = 1.0 / hf;
    let sf = p.sky.fields(&img);
    const N: usize = 8;
    struct Layer {
        t: f32,
        base: f32,
        amp: f32,
        tops: Vec<f32>,
        col: C,
    }
    let uc = p.valley_u;
    let layers: Vec<Layer> = (0..N)
        .map(|i| {
            let t = i as f32 / (N - 1) as f32;
            let base = lerp(0.49, 1.09, t.powf(1.3));
            let amp = lerp(0.05, 0.14, t);
            let freq = lerp(4.5, 1.2, t);
            let side = if i % 2 == 0 { -1.0 } else { 1.0 };
            let rise = lerp(0.012, 0.27, t.powf(1.2));
            let s = seed.wrapping_add(100 + i as u32 * 17);
            let tree_h = if t > 0.55 {
                lerp(0.007, 0.05, (t - 0.55) / 0.45)
            } else {
                0.0
            };
            let tops = tops_for(w, h, |u| {
                let r = ridged(u * freq + 3.1 * i as f32, 0.37 * i as f32, 6, s);
                let f = fbm(u * freq, 1.3 * i as f32, 5, s + 1) * 0.5 + 0.5;
                let prof = lerp(r, f, t.powf(0.6));
                let spur = smooth(-0.25, 0.85, side * (u - uc));
                let mut top = base - amp * prof - rise * spur;
                if tree_h > 0.0 && top < 0.985 {
                    top -= trees(u, tree_h * 0.13, tree_h, s + 5, |cx| {
                        smooth(-0.05, 0.12, fbm(cx * 3.5, i as f32, 3, s + 6))
                    });
                }
                top
            });
            Layer {
                t,
                base,
                amp,
                tops,
                col: mix(p.far, p.near, t.powf(0.7)),
            }
        })
        .collect();
    let haze: Vec<C> = (0..w)
        .map(|x| {
            let (u, _) = img.uv(x, 0);
            p.sky.base(u, p.sky.hor_v * 0.97)
        })
        .collect();
    let wisps = Field::new(w, h, (hf / 270.0).max(2.0), |x, y| {
        let (u, v) = ((x - w as f32 * 0.5) / hf, y / hf);
        let n = fbm(
            u * 2.2 + fbm(u, v * 6.0, 2, seed + 71) * 0.5,
            v * 26.0,
            4,
            seed + 70,
        );
        smooth(0.0, 0.5, n)
    });
    let sun_u = p.sky.sun[0];
    let min_top: Vec<f32> = (0..w)
        .map(|x| {
            layers
                .iter()
                .map(|l| col_min(&l.tops, x))
                .fold(f32::MAX, f32::min)
        })
        .collect();
    let mut out: Vec<C> = vec![[0.0; 3]; w * h];
    par_rows(&mut out, w, |y, row| {
        for x in 0..w {
            let (u, v) = img.uv(x, y);
            let (xf, yf) = (x as f32 + 0.5, y as f32 + 0.5);
            let mut c = p.sky.at(&sf, xf, yf, u, v, px);
            if v + 2.0 * px < min_top[x] {
                row[x] = p.sky.bloom_at(c, u, v);
                continue;
            }
            let hz = haze[x];
            let sunk = (-(u - sun_u).abs() * 3.0).exp();
            let first = layers
                .iter()
                .rposition(|l| {
                    v - 2.0 * px
                        > l.tops[x * SS..x * SS + SS]
                            .iter()
                            .copied()
                            .fold(f32::MIN, f32::max)
                })
                .unwrap_or(0);
            for (i, l) in layers.iter().enumerate().skip(first) {
                let cov = sil_cov(&l.tops, x, v, px);
                if cov > 0.0 {
                    let t = l.t;
                    let top = l.tops[x * SS + 1];
                    let mut lc = mix(l.col, hz, p.fog * (1.0 - t).powf(1.5));
                    let m = smooth(l.base - l.amp * 1.2, l.base + 0.02, v)
                        * p.mist
                        * (1.0 - t).powf(0.9);
                    lc = mix(lc, mix(hz, p.mist_c, 0.5), m);
                    let e = (v - top).max(0.0);
                    let rim = (-e / p.rim_w).exp() * sunk * (1.0 - t).powf(0.7);
                    lc = screen(lc, mul(p.rim, rim * 0.7));
                    c = mix(c, lc, cov);
                }
                if p.wisp > 0.0 && (1..N - 2).contains(&i) {
                    let wv = l.base - l.amp * 0.1;
                    let ws = 0.012 + 0.025 * l.t;
                    let g = ((v - wv) / ws).powi(2);
                    if g < 6.0 {
                        let d = wisps.at(xf, yf) * (-g).exp() * p.wisp;
                        c = mix(c, mix(hz, p.mist_c, 0.6), d * 0.8);
                    }
                }
            }
            row[x] = p.sky.bloom_at(c, u, v);
        }
    });
    img.px = out;
    img
}
