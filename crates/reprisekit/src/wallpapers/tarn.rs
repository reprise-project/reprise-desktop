//! Tarn: a mountain lake at first light. A faceted massif, a low hill line
//! and a spruce shore with a wooded spit, mirrored in the lake below with
//! breeze lines, a glitter path under the sun and mist off the water.

use super::land::{Img, SS, Sky, sil_cov, tops_for, trees};
use super::noise::land::{Field, fbm, noise, ridged};
use super::tone::{C, hex, lerp, mix, mul, mulc, quantise_grain, sat, screen, smooth};
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

struct Tarn {
    sky: Sky,
    rock: C,
    rock_lit: C,
    hills: C,
    snow: C,
    snow_shade: C,
    forest: C,
    fog: f32,
    water: C,
    mist: f32,
    mist_c: C,
    glitter: f32,
    glitter_c: C,
    grain: f32,
}

fn variant(var: &str, seed: u32) -> Tarn {
    let sky = Sky {
        zen: hex(0x354475),
        mid: hex(0xAE96B2),
        hor: hex(0xFAD2AE),
        hor_v: 0.585,
        sun: [0.40, 0.488],
        sun_r: 0.017,
        sun_core: hex(0xFFF7E4),
        glow: hex(0xFFD8A2),
        glow_k: 9.0,
        halo: hex(0xF2A48E),
        halo_k: 2.3,
        bloom: 0.35,
        rays: 0.4,
        cloud: 0.5,
        cloud_lit: hex(0xFFDCC0),
        cloud_dark: hex(0x9A86A8),
        cloud_v: [0.07, 0.32],
        stars: 0.0,
        milky: 0.0,
        seed,
    };
    let base = Tarn {
        sky,
        rock: hex(0x7A6A92),
        rock_lit: hex(0xD8A8A0),
        hills: hex(0x564870),
        snow: hex(0xFFE8DA),
        snow_shade: hex(0xA8A0C8),
        forest: hex(0x19172A),
        fog: 0.55,
        water: hex(0x262C4C),
        mist: 0.7,
        mist_c: hex(0xF8E0D0),
        glitter: 0.7,
        glitter_c: hex(0xFFE4B4),
        grain: 0.016,
    };
    match var {
        "day" => Tarn {
            sky: Sky {
                zen: hex(0x4878B0),
                mid: hex(0x98BCD6),
                hor: hex(0xE4EEEA),
                sun: [0.8, -0.3],
                sun_r: 0.0,
                glow: hex(0xFFF8E0),
                glow_k: 2.0,
                halo: hex(0xFFFFFF),
                halo_k: 1.2,
                bloom: 0.0,
                rays: 0.0,
                cloud: 0.4,
                cloud_lit: hex(0xF8FAF8),
                cloud_dark: hex(0xC0CEDA),
                ..base.sky
            },
            rock: hex(0x5E7488),
            rock_lit: hex(0xB4C4CC),
            hills: hex(0x3E5A66),
            snow: hex(0xF8FBFC),
            snow_shade: hex(0xB4C8DC),
            forest: hex(0x122A24),
            water: hex(0x163A44),
            mist: 0.3,
            mist_c: hex(0xF0F4F2),
            glitter: 0.0,
            ..base
        },
        "dusk" => Tarn {
            sky: Sky {
                zen: hex(0x1C1A42),
                mid: hex(0x8C4878),
                hor: hex(0xFF9656),
                sun: [0.40, 0.495],
                sun_r: 0.024,
                sun_core: hex(0xFFE4A6),
                glow: hex(0xFFA650),
                glow_k: 7.0,
                halo: hex(0xD84E68),
                halo_k: 1.9,
                bloom: 0.45,
                rays: 0.7,
                cloud: 0.75,
                cloud_lit: hex(0xFFAA70),
                cloud_dark: hex(0x58306A),
                ..base.sky
            },
            rock: hex(0x6A3460),
            rock_lit: hex(0xF08A70),
            hills: hex(0x3E1E44),
            snow: hex(0xFFC4A0),
            snow_shade: hex(0x9A5A86),
            forest: hex(0x120A1C),
            water: hex(0x1E1232),
            mist_c: hex(0xFFB48E),
            glitter: 1.0,
            glitter_c: hex(0xFFC070),
            ..base
        },
        "night" => Tarn {
            sky: Sky {
                zen: hex(0x040716),
                mid: hex(0x0C1432),
                hor: hex(0x203058),
                sun: [0.42, 0.15],
                sun_r: 0.014,
                sun_core: hex(0xF4F2EA),
                glow: hex(0x7C90C8),
                glow_k: 16.0,
                halo: hex(0x223464),
                halo_k: 3.5,
                bloom: 0.0,
                rays: 0.0,
                cloud: 0.2,
                cloud_lit: hex(0x7080A8),
                cloud_dark: hex(0x161E3C),
                stars: 1.0,
                milky: 1.0,
                ..base.sky
            },
            rock: hex(0x1E2A4A),
            rock_lit: hex(0x4A5C88),
            hills: hex(0x111A34),
            snow: hex(0x9AA8D0),
            snow_shade: hex(0x46557E),
            forest: hex(0x03050C),
            water: hex(0x040818),
            mist: 0.4,
            mist_c: hex(0x56669A),
            glitter: 0.6,
            glitter_c: hex(0xDCE4FF),
            grain: 0.014,
            ..base
        },
        "blue" => Tarn {
            sky: Sky {
                zen: hex(0x0A2A8C),
                mid: hex(0x386ACE),
                hor: hex(0xC4D6F2),
                sun: [0.40, 0.49],
                sun_r: 0.016,
                sun_core: hex(0xFFFEF2),
                glow: hex(0xF4F0C4),
                glow_k: 10.0,
                halo: hex(0x8CB0EA),
                halo_k: 2.2,
                bloom: 0.3,
                rays: 0.35,
                cloud: 0.5,
                cloud_lit: hex(0xEEF2FA),
                cloud_dark: hex(0x4670C6),
                ..base.sky
            },
            rock: hex(0x3A62C0),
            rock_lit: hex(0xB8CCF0),
            hills: hex(0x1E46A4),
            snow: hex(0xF6F9FF),
            snow_shade: hex(0x9CB4E6),
            forest: hex(0x08184A),
            water: hex(0x082268),
            mist_c: hex(0xE4ECFA),
            glitter: 0.6,
            glitter_c: hex(0xF6F2C8),
            ..base
        },
        _ => base,
    }
}

fn draw(w: usize, h: usize, p: &Tarn, seed: u32) -> Img {
    let mut img = Img::new(w, h);
    let hf = h as f32;
    let uv = |x: usize, y: usize| {
        (
            (x as f32 + 0.5 - w as f32 * 0.5) / hf,
            (y as f32 + 0.5) / hf,
        )
    };
    let px = 1.0 / hf;
    let v0 = 0.585f32;
    let y0 = v0 * hf;
    let sf = p.sky.fields(&img);
    let peak_u = -0.3f32;
    let mtn_far = tops_for(w, h, |u| {
        let r = ridged(u * 2.8 + 1.7, 0.5, 7, seed + 200);
        let wd = if u < peak_u { 0.2 } else { 0.42 };
        let massif = (-((u - peak_u) / wd).powi(2)).exp();
        let shoulder = (-((u - 0.6) / 0.38).powi(2)).exp();
        v0 - 0.035
            - 0.27 * massif * (0.62 + 0.38 * r)
            - 0.045 * shoulder * (0.55 + 0.45 * r)
            - 0.018 * r
    });
    let mtn_near = tops_for(w, h, |u| {
        let f = fbm(u * 2.0, 3.3, 5, seed + 210) * 0.5 + 0.5;
        let r = ridged(u * 4.0, 1.1, 5, seed + 211);
        v0 - 0.012 - 0.05 * f - 0.02 * r - 0.03 * smooth(0.5, 1.0, u)
    });
    let spit_at = |u: f32| smooth(0.5, 0.95, u);
    let shore = tops_for(w, h, |u| {
        let spit = spit_at(u);
        let ground =
            v0 - 0.004 - 0.008 * spit - 0.003 * (fbm(u * 14.0, 0.0, 3, seed + 220) * 0.5 + 0.5);
        let th = trees(u, 0.0021, lerp(0.016, 0.07, spit), seed + 221, |cx| {
            sat(0.45
                + 0.6 * smooth(-0.2, 0.25, fbm(cx * 4.0, 0.5, 3, seed + 222))
                + 1.6 * spit_at(cx))
        });
        ground - th
    });
    let sun_u = p.sky.sun[0];
    let light = if sun_u > peak_u { 1.0 } else { -1.0 };
    let wisps = Field::new(w, h, (hf / 270.0).max(2.0), |x, y| {
        let (u, v) = ((x - w as f32 * 0.5) / hf, y / hf);
        let n = fbm(
            u * 1.8 + fbm(u * 0.8, v * 5.0, 2, seed + 231) * 0.6,
            v * 30.0,
            4,
            seed + 230,
        );
        smooth(-0.05, 0.5, n)
    });
    let snow_line = |u: f32| v0 - 0.15 + 0.02 * fbm(u * 6.0, 0.0, 3, seed + 240);
    // Spurs radiate down from the summit; evaluated coarse.
    let pv = 0.29f32;
    let face = |u: f32, v: f32| {
        let (du, dv) = (u - peak_u, v - pv + 0.04);
        let a = du.atan2(dv);
        let r = (du * du + dv * dv).sqrt();
        ridged(
            a * 6.0 + 0.6 * fbm(r * 6.0, a, 2, seed + 243),
            r * 4.0,
            4,
            seed + 241,
        )
    };
    let fstep = (hf / 540.0).max(2.0);
    let rows = (y0 as usize + 4).min(h);
    let lit_f = Field::new(w, rows, fstep, |x, y| {
        let (u, v) = ((x - w as f32 * 0.5) / hf, y / hf);
        let e = 0.002;
        // Raw slope: a thresholded field would interpolate into steps.
        (face(u + e, v) - face(u - e, v)) * light
    });
    let relief = Field::new(w, rows, fstep, |x, y| {
        face((x - w as f32 * 0.5) / hf, y / hf)
    });
    let hz_col: Vec<C> = (0..w)
        .map(|x| p.sky.base(img.uv(x, 0).0, p.sky.hor_v * 0.97))
        .collect();
    let ymax = (y0.ceil() as usize + 1).min(h);
    par_rows(&mut img.px[..ymax * w], w, |y, row| {
        for x in 0..w {
            let (u, v) = uv(x, y);
            let (xf, yf) = (x as f32 + 0.5, y as f32 + 0.5);
            let mut c = p.sky.at(&sf, xf, yf, u, v, px);
            let hz = hz_col[x];
            let cf = sil_cov(&mtn_far, x, v, px);
            if cf > 0.0 {
                let top = mtn_far[x * SS + 1];
                let big = sat(0.5 + (u - peak_u - (v - pv) * 0.5) * light * 12.0);
                let lit = sat(smooth(-0.03, 0.03, lit_f.at(xf, yf)) * 0.65 + big * 0.5 - 0.1);
                let rel = relief.at(xf, yf);
                let sn = smooth(0.0, 0.0025, snow_line(u) - v + 0.09 * (rel - 0.45));
                let rock = mix(p.rock, p.rock_lit, lit * 0.8);
                let snow = mix(p.snow_shade, p.snow, lit);
                let mut mc = mix(rock, snow, sn);
                mc = mix(mc, hz, p.fog * 0.5 + 0.35 * smooth(top + 0.04, v0, v));
                c = mix(c, mc, cf);
            }
            let cn = sil_cov(&mtn_near, x, v, px);
            if cn > 0.0 {
                let mut mc = mix(p.hills, hz, p.fog * 0.4);
                mc = mix(mc, mix(hz, p.mist_c, 0.5), smooth(v0 - 0.06, v0, v) * 0.6);
                c = mix(c, mc, cn);
            }
            let cs = sil_cov(&shore, x, v, px);
            if cs > 0.0 {
                let mc = mix(p.forest, hz, 0.1 * p.fog);
                c = mix(c, mc, cs);
            }
            row[x] = c;
        }
    });
    // Calm and ruffled patches vary slowly, so they come from a coarse field.
    let calm_f = Field::new(w, h, (hf / 270.0).max(2.0), |x, y| {
        let (u, v) = ((x - w as f32 * 0.5) / hf, y / hf);
        let wv = (v - v0).max(0.002);
        smooth(
            0.2,
            0.6,
            fbm(u / wv * 0.06, 0.22 / wv, 3, seed + 250) * 0.5 + 0.5,
        )
    });
    let (top, bottom) = img.px.split_at_mut(ymax * w);
    let above = Img {
        w,
        h,
        px: top.to_vec(),
    };
    par_rows(bottom, w, |k, row| {
        let y = ymax + k;
        for (x, out) in row.iter_mut().enumerate() {
            let (u, v) = above.uv(x, y);
            let wv = (v - v0).max(px * 0.5);
            let (pu, pz) = (u / wv, 1.0 / wv);
            let calm = calm_f.at(x as f32 + 0.5, y as f32 + 0.5);
            let rip = fbm(pu * 0.8, pz * 9.0, 3, seed + 251);
            let amp = (0.1 + 0.9 * calm) * wv * hf * 0.05;
            let sy = 2.0 * y0 - (y as f32 + 0.5) + rip * amp;
            let sx = x as f32 + 0.5 + rip * amp * 0.2;
            let mut c = above.sample(sx - 0.5, sy.min(y0 - 1.0) - 0.5);
            let streak =
                smooth(0.3, 0.75, calm) * smooth(0.1, 0.6, noise(pu * 0.35, pz * 3.0, seed + 252));
            c = mix(c, mix(p.sky.zen, p.sky.mid, 0.55), streak * 0.3);
            c = mulc(c, [0.8, 0.84, 0.9]);
            c = mix(c, p.water, smooth(0.0, 0.36, wv).powf(0.8) * 0.78);
            if p.glitter > 0.0 {
                let path = (-((u - sun_u) / (0.012 + wv * 0.3)).powi(2)).exp();
                if path > 0.01 {
                    let g = noise(pu * 2.5, pz * 46.0, seed + 253);
                    let sp =
                        smooth(0.5, 0.8, g) * path * p.glitter * (1.0 - smooth(0.0, 0.4, wv) * 0.6);
                    c = screen(c, mul(p.glitter_c, sp));
                }
            }
            *out = c;
        }
    });
    par_rows(&mut img.px, w, |y, row| {
        let v = (y as f32 + 0.5) / hf;
        let g = ((v - v0) / 0.04).powi(2);
        for x in 0..w {
            let (u, _) = uv(x, y);
            let mut c = row[x];
            if g < 8.0 {
                let hz = hz_col[x];
                let d = wisps.at(x as f32 + 0.5, y as f32 + 0.5) * (-g).exp() * p.mist;
                let band = (-((v - v0) / 0.01).powi(2)).exp() * 0.3 * p.mist;
                c = mix(c, mix(hz, p.mist_c, 0.6), (d * 0.75 + band).min(0.95));
            }
            row[x] = if v > v0 {
                mix(c, p.sky.bloom_at(c, u, 2.0 * v0 - v), 0.7)
            } else {
                p.sky.bloom_at(c, u, v)
            };
        }
    });
    img
}
