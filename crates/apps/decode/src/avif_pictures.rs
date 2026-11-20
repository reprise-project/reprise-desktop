//! Test AVIFs: AV1 pictures encoded by rav1e in a container written here, or whole files by
//! ravif. Shared by the unit tests and the tests that run the binary.
#![allow(dead_code)]

use rav1e::config::GrainTableSegment;
use rav1e::prelude::{
    ChromaSampling, ColorDescription, ColorPrimaries, Config, Context, EncoderConfig,
    EncoderStatus, MatrixCoefficients, PixelRange, SpeedSettings, TransferCharacteristics,
};

/// How one AV1 picture is coded.
#[derive(Clone, Copy)]
pub struct Coding {
    pub bits: u8,
    pub sampling: ChromaSampling,
    pub full_range: bool,
    /// CICP primaries, transfer and matrix in the sequence header.
    pub cicp: (u8, u8, u8),
    /// rav1e's quantizer: 0 codes every sample exactly.
    pub quantizer: usize,
    /// Film grain for the decoder to add, on luma and chroma alike.
    pub grain: bool,
}

impl Coding {
    pub const EXACT: Coding = Coding {
        bits: 8,
        sampling: ChromaSampling::Cs444,
        full_range: true,
        cicp: (1, 13, 6),
        quantizer: 0,
        grain: false,
    };
}

/// The luma weights of red and blue for CICP matrix `mc`; None for identity (GBR).
fn weights(mc: u8) -> Option<(f64, f64)> {
    match mc {
        0 => None,
        1 => Some((0.2126, 0.0722)),
        9 => Some((0.2627, 0.0593)),
        _ => Some((0.299, 0.114)),
    }
}

/// `w`x`h` pixels of RGB samples (up to `c.bits`) as YUV planes under `c`, chroma averaged over
/// its block when subsampled.
pub fn planes(w: usize, h: usize, rgb: &[[u16; 3]], c: Coding) -> [Vec<u16>; 3] {
    let max = f64::from((1u32 << c.bits) - 1);
    let k = f64::from(1u32 << (c.bits - 8));
    let (yo, ys, cs) = if c.full_range {
        (0.0, max, max)
    } else {
        (16.0 * k, 219.0 * k, 224.0 * k)
    };
    let mid = f64::from(1u32 << (c.bits - 1));
    let yuv: Vec<[f64; 3]> = rgb
        .iter()
        .map(|p| {
            let [r, g, b] = p.map(|v| f64::from(v) / max);
            match weights(c.cicp.2) {
                None => [g * ys + yo, b * ys + yo, r * ys + yo],
                Some((kr, kb)) => {
                    let y = kr * r + (1.0 - kr - kb) * g + kb * b;
                    let u = (b - y) / (2.0 * (1.0 - kb));
                    let v = (r - y) / (2.0 * (1.0 - kr));
                    [y * ys + yo, u * cs + mid, v * cs + mid]
                }
            }
        })
        .collect();
    let (sx, sy) = match c.sampling {
        ChromaSampling::Cs420 => (1, 1),
        ChromaSampling::Cs422 => (1, 0),
        _ => (0, 0),
    };
    let (cw, ch) = ((w + sx) >> sx, (h + sy) >> sy);
    let q = |v: f64| v.round().clamp(0.0, max) as u16;
    let luma = yuv.iter().map(|p| q(p[0])).collect();
    let chroma = |i: usize| {
        let mut out = Vec::with_capacity(cw * ch);
        for cy in 0..ch {
            for cx in 0..cw {
                let (mut sum, mut n) = (0.0, 0.0);
                for y in (cy << sy)..((cy + 1) << sy).min(h) {
                    for x in (cx << sx)..((cx + 1) << sx).min(w) {
                        sum += yuv[y * w + x][i];
                        n += 1.0;
                    }
                }
                out.push(q(sum / n));
            }
        }
        out
    };
    [luma, chroma(1), chroma(2)]
}

/// An AV1 still of `planes` (luma, then chroma unless `c.sampling` is 4:0:0), a sequence header
/// and one key frame.
pub fn av1(w: usize, h: usize, planes: &[Vec<u16>], c: Coding) -> Vec<u8> {
    let (p, t, m) = c.cicp;
    let color_description = ColorDescription {
        color_primaries: match p {
            1 => ColorPrimaries::BT709,
            9 => ColorPrimaries::BT2020,
            12 => ColorPrimaries::SMPTE432,
            _ => ColorPrimaries::Unspecified,
        },
        transfer_characteristics: match t {
            1 => TransferCharacteristics::BT709,
            13 => TransferCharacteristics::SRGB,
            16 => TransferCharacteristics::SMPTE2084,
            18 => TransferCharacteristics::HLG,
            _ => TransferCharacteristics::Unspecified,
        },
        matrix_coefficients: match m {
            0 => MatrixCoefficients::Identity,
            1 => MatrixCoefficients::BT709,
            6 => MatrixCoefficients::BT601,
            9 => MatrixCoefficients::BT2020NCL,
            _ => MatrixCoefficients::Unspecified,
        },
    };
    let enc = EncoderConfig {
        width: w,
        height: h,
        bit_depth: usize::from(c.bits),
        chroma_sampling: c.sampling,
        pixel_range: if c.full_range {
            PixelRange::Full
        } else {
            PixelRange::Limited
        },
        color_description: Some(color_description),
        still_picture: true,
        quantizer: c.quantizer,
        min_quantizer: c.quantizer as u8,
        speed_settings: SpeedSettings::from_preset(10),
        film_grain_params: c.grain.then(|| vec![grain()]),
        ..EncoderConfig::default()
    };
    let cfg = Config::new().with_encoder_config(enc).with_threads(1);
    if c.bits == 8 {
        encode::<u8>(cfg, w, planes, c)
    } else {
        encode::<u16>(cfg, w, planes, c)
    }
}

/// Even grain of strength 40 over every luma level, chroma scaled as luma.
fn grain() -> GrainTableSegment {
    GrainTableSegment {
        start_time: 0,
        end_time: u64::MAX,
        scaling_points_y: [[0, 40], [255, 40]].into_iter().collect(),
        scaling_points_cb: Default::default(),
        scaling_points_cr: Default::default(),
        scaling_shift: 8,
        ar_coeff_lag: 0,
        ar_coeffs_y: Default::default(),
        ar_coeffs_cb: [0].into_iter().collect(),
        ar_coeffs_cr: [0].into_iter().collect(),
        ar_coeff_shift: 6,
        cb_mult: 128,
        cb_luma_mult: 192,
        cb_offset: 256,
        cr_mult: 128,
        cr_luma_mult: 192,
        cr_offset: 256,
        overlap_flag: true,
        chroma_scaling_from_luma: true,
        grain_scale_shift: 0,
        random_seed: 7391,
    }
}

fn encode<P: rav1e::Pixel>(cfg: Config, w: usize, planes: &[Vec<u16>], c: Coding) -> Vec<u8> {
    let mut ctx: Context<P> = cfg.new_context().unwrap();
    let mut frame = ctx.new_frame();
    let sx = usize::from(matches!(
        c.sampling,
        ChromaSampling::Cs420 | ChromaSampling::Cs422
    ));
    for (i, samples) in planes.iter().enumerate() {
        let pw = if i == 0 { w } else { (w + sx) >> sx };
        let bytes: Vec<u8> = if c.bits == 8 {
            samples.iter().map(|&v| v as u8).collect()
        } else {
            samples.iter().flat_map(|v| v.to_le_bytes()).collect()
        };
        let width = if c.bits == 8 { 1 } else { 2 };
        frame.planes[i].copy_from_raw_u8(&bytes, pw * width, width);
    }
    ctx.send_frame(frame).unwrap();
    ctx.flush();
    let mut out = Vec::new();
    loop {
        match ctx.receive_packet() {
            Ok(mut p) => out.append(&mut p.data),
            Err(EncoderStatus::Encoded) => {}
            Err(EncoderStatus::LimitReached) => break,
            Err(e) => panic!("{e:?}"),
        }
    }
    out
}

/// An AV1 still of RGB `rgb` under `c`.
pub fn av1_rgb(w: usize, h: usize, rgb: &[[u16; 3]], c: Coding) -> Vec<u8> {
    av1(w, h, &planes(w, h, rgb, c), c)
}

/// An 8-bit alpha plane as a monochrome AV1 still, coded exactly.
pub fn av1_alpha(w: usize, h: usize, alpha: &[u8]) -> Vec<u8> {
    let c = Coding {
        sampling: ChromaSampling::Cs400,
        cicp: (2, 2, 2),
        ..Coding::EXACT
    };
    av1(w, h, &[alpha.iter().map(|&a| u16::from(a)).collect()], c)
}

/// A box of `kind` holding `body`.
pub fn bx(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut b = ((body.len() + 8) as u32).to_be_bytes().to_vec();
    b.extend(kind);
    b.extend(body);
    b
}

/// A full box of `kind`, version and flags 0.
pub fn full(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    bx(kind, &[&[0, 0, 0, 0], body].concat())
}

pub fn ispe(w: u32, h: u32) -> Vec<u8> {
    full(b"ispe", &[w.to_be_bytes(), h.to_be_bytes()].concat())
}

pub fn nclx(p: u16, t: u16, m: u16, full_range: bool) -> Vec<u8> {
    let mut b = b"nclx".to_vec();
    for v in [p, t, m] {
        b.extend(v.to_be_bytes());
    }
    b.push(if full_range { 0x80 } else { 0 });
    bx(b"colr", &b)
}

pub fn icc(profile: &[u8]) -> Vec<u8> {
    bx(b"colr", &[b"prof", profile].concat())
}

pub fn irot(quarters: u8) -> Vec<u8> {
    bx(b"irot", &[quarters])
}

pub fn imir(mode: u8) -> Vec<u8> {
    bx(b"imir", &[mode])
}

pub fn clli(max_cll: u16) -> Vec<u8> {
    bx(
        b"clli",
        &[max_cll.to_be_bytes(), 0u16.to_be_bytes()].concat(),
    )
}

pub fn alpha_aux() -> Vec<u8> {
    full(b"auxC", b"urn:mpeg:mpegB:cicp:systems:auxiliary:alpha\0")
}

/// One item of a test file: its id, type, data and property boxes.
pub struct Item {
    pub id: u32,
    pub kind: [u8; 4],
    pub data: Vec<u8>,
    pub props: Vec<Vec<u8>>,
}

impl Item {
    pub fn av01(id: u32, data: Vec<u8>, props: Vec<Vec<u8>>) -> Item {
        Item {
            id,
            kind: *b"av01",
            data,
            props,
        }
    }
}

/// The data of a grid item of `rows` x `cols` tiles making a `w`x`h` picture.
pub fn grid(rows: u8, cols: u8, w: u16, h: u16) -> Vec<u8> {
    [
        &[0, 0, rows - 1, cols - 1][..],
        &w.to_be_bytes(),
        &h.to_be_bytes(),
    ]
    .concat()
}

/// A still AVIF of `items`, `primary` the shown one, with item references (type, from, to).
pub fn still(items: &[Item], primary: u32, refs: &[(&[u8; 4], u32, &[u32])]) -> Vec<u8> {
    let ftyp = bx(b"ftyp", b"avif\0\0\0\0avifmif1miaf");
    let meta = |mdat_at: u32| {
        let hdlr = full(b"hdlr", b"\0\0\0\0pict\0\0\0\0\0\0\0\0\0\0\0\0\0");
        let pitm = full(b"pitm", &(primary as u16).to_be_bytes());
        let mut iloc = vec![0x44, 0x00];
        iloc.extend((items.len() as u16).to_be_bytes());
        let mut at = mdat_at + 8;
        for it in items {
            iloc.extend((it.id as u16).to_be_bytes());
            iloc.extend([0, 0, 0, 1]);
            iloc.extend(at.to_be_bytes());
            iloc.extend((it.data.len() as u32).to_be_bytes());
            at += it.data.len() as u32;
        }
        let iloc = full(b"iloc", &iloc);
        let mut iinf = (items.len() as u16).to_be_bytes().to_vec();
        for it in items {
            let mut infe = vec![2, 0, 0, 0];
            infe.extend((it.id as u16).to_be_bytes());
            infe.extend([0, 0]);
            infe.extend(it.kind);
            infe.push(0);
            iinf.extend(bx(b"infe", &infe));
        }
        let iinf = full(b"iinf", &iinf);
        let mut iref = Vec::new();
        for (kind, from, to) in refs {
            let mut r = (*from as u16).to_be_bytes().to_vec();
            r.extend((to.len() as u16).to_be_bytes());
            to.iter().for_each(|t| r.extend((*t as u16).to_be_bytes()));
            iref.extend(bx(kind, &r));
        }
        let iref = full(b"iref", &iref);
        let mut ipco = Vec::new();
        let mut ipma = (items.len() as u32).to_be_bytes().to_vec();
        let mut n = 0u8;
        for it in items {
            ipma.extend((it.id as u16).to_be_bytes());
            ipma.push(it.props.len() as u8);
            for p in &it.props {
                ipco.extend(p);
                n += 1;
                ipma.push(n);
            }
        }
        let iprp = bx(
            b"iprp",
            &[bx(b"ipco", &ipco), full(b"ipma", &ipma)].concat(),
        );
        full(b"meta", &[hdlr, pitm, iloc, iinf, iref, iprp].concat())
    };
    let len = (ftyp.len() + meta(0).len()) as u32;
    let mdat: Vec<u8> = items.iter().flat_map(|i| i.data.clone()).collect();
    [ftyp, meta(len), bx(b"mdat", &mdat)].concat()
}

/// `file` with the box of `kind` in its `meta` replaced by `with`, a whole box.
pub fn replaced_in_meta(file: &[u8], kind: &[u8; 4], with: &[u8]) -> Vec<u8> {
    let boxes = |b: &[u8]| {
        let mut out = Vec::new();
        let mut at = 0;
        while at < b.len() {
            let len = u32::from_be_bytes(b[at..at + 4].try_into().unwrap()) as usize;
            out.push(b[at..at + len].to_vec());
            at += len;
        }
        out
    };
    let mut out = Vec::new();
    for top in boxes(file) {
        if &top[4..8] != b"meta" {
            out.extend(top);
            continue;
        }
        let inner: Vec<u8> = boxes(&top[12..])
            .into_iter()
            .flat_map(|b| if &b[4..8] == kind { with.to_vec() } else { b })
            .collect();
        out.extend(full(b"meta", &inner));
    }
    out
}

/// A still AVIF of one picture with `props`.
pub fn single(data: Vec<u8>, props: Vec<Vec<u8>>) -> Vec<u8> {
    still(&[Item::av01(1, data, props)], 1, &[])
}

/// An image sequence of `frames`, each an AV1 still of `w`x`h`, with no `meta` picture.
pub fn sequence(w: u16, h: u16, frames: &[Vec<u8>]) -> Vec<u8> {
    let ftyp = bx(b"ftyp", b"avis\0\0\0\0avismsf1miafavif");
    let moov = |mdat_at: u32| {
        let tkhd = full(
            b"tkhd",
            &[[0; 8].as_slice(), &1u32.to_be_bytes(), &[0; 68]].concat(),
        );
        let hdlr = full(b"hdlr", b"\0\0\0\0pict\0\0\0\0\0\0\0\0\0\0\0\0\0");
        let mut entry = vec![0; 24];
        entry[7] = 1;
        entry.extend(w.to_be_bytes());
        entry.extend(h.to_be_bytes());
        entry.extend([0; 50]);
        entry.extend(nclx(1, 13, 6, true));
        let stsd = full(
            b"stsd",
            &[&1u32.to_be_bytes()[..], &bx(b"av01", &entry)].concat(),
        );
        let mut stsz = 0u32.to_be_bytes().to_vec();
        stsz.extend((frames.len() as u32).to_be_bytes());
        frames
            .iter()
            .for_each(|f| stsz.extend((f.len() as u32).to_be_bytes()));
        let stsc = full(b"stsc", &[1u32, 1, 1, 1].map(u32::to_be_bytes).concat());
        let mut stco = (frames.len() as u32).to_be_bytes().to_vec();
        let mut at = mdat_at + 8;
        for f in frames {
            stco.extend(at.to_be_bytes());
            at += f.len() as u32;
        }
        let stbl = bx(
            b"stbl",
            &[stsd, full(b"stsz", &stsz), stsc, full(b"stco", &stco)].concat(),
        );
        let minf = bx(b"minf", &stbl);
        let mdia = bx(b"mdia", &[hdlr, minf].concat());
        bx(b"moov", &bx(b"trak", &[tkhd, mdia].concat()))
    };
    let len = (ftyp.len() + moov(0).len()) as u32;
    [ftyp, moov(len), bx(b"mdat", &frames.concat())].concat()
}

/// `w`x`h` noise RGBA, opaque.
pub fn noise_rgb(w: usize, h: usize, bits: u8) -> Vec<[u16; 3]> {
    let max = (1u32 << bits) - 1;
    (0..w * h)
        .map(|i| {
            let n = (i as u32).wrapping_mul(2_654_435_761);
            [n >> 8, n >> 16, n >> 24].map(|v| (v % (max + 1)) as u16)
        })
        .collect()
}

/// An 8-bit RGBA AVIF by ravif.
pub fn ravif(w: usize, h: usize, rgba: &[u8], ten_bit: bool) -> Vec<u8> {
    let px: Vec<ravif::RGBA8> = (rgba.as_chunks::<4>().0.iter())
        .map(|&[r, g, b, a]| ravif::RGBA8::new(r, g, b, a))
        .collect();
    let depth = if ten_bit {
        ravif::BitDepth::Ten
    } else {
        ravif::BitDepth::Eight
    };
    ravif::Encoder::new()
        .with_quality(100.0)
        .with_alpha_quality(100.0)
        .with_speed(10)
        .with_bit_depth(depth)
        .with_num_threads(Some(1))
        .encode_rgba(ravif::Img::new(&px[..], w, h))
        .unwrap()
        .avif_file
}
