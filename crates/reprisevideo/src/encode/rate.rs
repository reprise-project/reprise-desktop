//! How an encoder spends bits: a quality on each codec's CRF or QP scale, or a bitrate, as each
//! backend's FFmpeg options.

use super::Codec;
use super::probe::Backend;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RateControl {
    /// 1..100, 100 the best.
    Quality(u8),
    /// kbit/s on average, peaks up to twice that.
    Average(u32),
    /// kbit/s held through each second.
    Constant(u32),
}

/// The quality that codes H.264 at x264's CRF 23 and the hardware encoders' QP 24.
pub const DEFAULT_QUALITY: u8 = 55;

/// What [`rate`] sets on an encoder: rates in bit/s and the context's quality, 0 when unset,
/// and its private options.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rate {
    pub bit_rate: usize,
    pub max_rate: usize,
    pub buffer_size: i32,
    /// The constant quality of an encoder with no `qp` option (VA-API AV1's q-index).
    pub global_quality: i32,
    pub options: Vec<(&'static str, String)>,
}

/// `quality` as the software encoder's constant rate factor for `codec`.
pub fn quality_crf(codec: Codec, quality: u8) -> u8 {
    match codec {
        Codec::H264 => on_scale(quality, 51, 23),
        Codec::H265 => on_scale(quality, 51, 28),
        Codec::Av1 => on_scale(quality, 63, 35),
    }
}

/// `quality` as the hardware encoders' constant QP for `codec` (AV1's q-index).
pub fn quality_qp(codec: Codec, quality: u8) -> u8 {
    match codec {
        Codec::H264 => on_scale(quality, 51, 24),
        Codec::H265 => on_scale(quality, 51, 28),
        Codec::Av1 => on_scale(quality, 255, 128),
    }
}

/// Straight lines through (1, `worst`), ([`DEFAULT_QUALITY`], `default`) and (100, 1): 0 is
/// lossless, which few players open.
fn on_scale(quality: u8, worst: u8, default: u8) -> u8 {
    let q = u32::from(quality.clamp(1, 100));
    let (worst, default, knee) = (
        u32::from(worst),
        u32::from(default),
        u32::from(DEFAULT_QUALITY),
    );
    let down = |from: u32, to: u32, at: u32, span: u32| from - ((from - to) * at + span / 2) / span;
    let v = match q <= knee {
        true => down(worst, default, q - 1, knee - 1),
        false => down(default, 1, q - knee, 100 - knee),
    };
    v as u8
}

pub fn constant_qp(backend: Backend, qp: u8) -> Vec<(&'static str, String)> {
    let qp = ("qp", qp.to_string());
    match backend {
        Backend::Vaapi { low_power } => {
            vec![("rc_mode", "CQP".into()), qp, low_power_pair(low_power)]
        }
        Backend::Nvenc => vec![("rc", "constqp".into()), qp],
        Backend::Vulkan => vec![("rc_mode", "cqp".into()), qp],
        Backend::Software => vec![qp],
    }
}

fn low_power_pair(low_power: bool) -> (&'static str, String) {
    ("low_power", if low_power { "1" } else { "0" }.into())
}

fn software_preset(codec: Codec) -> (&'static str, String) {
    let preset = match codec {
        Codec::H264 => "veryfast",
        Codec::H265 => "ultrafast",
        Codec::Av1 => "10",
    };
    ("preset", preset.into())
}

pub fn rate(backend: Backend, codec: Codec, rate: RateControl) -> Rate {
    let (kbit, constant) = match rate {
        RateControl::Quality(q) => {
            let qp = quality_qp(codec, q);
            let mut global_quality = 0;
            let options = match (backend, codec) {
                (Backend::Vaapi { low_power }, Codec::Av1) => {
                    global_quality = i32::from(qp);
                    vec![("rc_mode", "CQP".into()), low_power_pair(low_power)]
                }
                (Backend::Software, _) => {
                    vec![
                        software_preset(codec),
                        ("crf", quality_crf(codec, q).to_string()),
                    ]
                }
                (Backend::Nvenc, _) => {
                    let mut o = constant_qp(backend, qp);
                    o.push(("preset", "p4".into()));
                    o
                }
                _ => constant_qp(backend, qp),
            };
            return Rate {
                bit_rate: 0,
                max_rate: 0,
                buffer_size: 0,
                global_quality,
                options,
            };
        }
        RateControl::Average(kbit) => (kbit, false),
        RateControl::Constant(kbit) => (kbit, true),
    };
    let mode = |cbr: &'static str, vbr: &'static str| if constant { cbr } else { vbr }.to_string();
    let options = match backend {
        Backend::Vaapi { low_power } => {
            vec![("rc_mode", mode("CBR", "VBR")), low_power_pair(low_power)]
        }
        Backend::Nvenc => vec![("rc", mode("cbr", "vbr")), ("preset", "p4".into())],
        Backend::Vulkan => vec![("rc_mode", mode("cbr", "vbr"))],
        Backend::Software => {
            let mut o = vec![software_preset(codec)];
            match codec {
                // Filler holds the rate on a still screen.
                Codec::H264 if constant => o.push(("x264-params", "nal-hrd=cbr".into())),
                // SVT-AV1 runs CBR only with low-delay prediction; FFmpeg picks its CBR when
                // the maximum rate is the bit rate.
                Codec::Av1 if constant => o.push(("svtav1-params", "pred-struct=1".into())),
                _ => {}
            }
            o
        }
    };
    let bits = kbit as usize * 1000;
    let (max_rate, buffer) = match constant {
        true => (bits, bits),
        false => (2 * bits, 2 * bits),
    };
    Rate {
        bit_rate: bits,
        max_rate,
        buffer_size: i32::try_from(buffer).unwrap_or(i32::MAX),
        global_quality: 0,
        options,
    }
}

/// Set `rate`'s bit rate, maximum rate, buffer size and quality on an encoder not yet opened.
pub fn apply(rate: &Rate, ctx: &mut ffmpeg::encoder::video::Video) {
    if rate.bit_rate > 0 {
        ctx.set_bit_rate(rate.bit_rate);
    }
    if rate.max_rate > 0 {
        ctx.set_max_bit_rate(rate.max_rate);
    }
    if rate.buffer_size > 0 {
        // SAFETY: a field of a context not yet opened.
        unsafe { (*ctx.as_mut_ptr()).rc_buffer_size = rate.buffer_size };
    }
    if rate.global_quality > 0 {
        // SAFETY: a field of a context not yet opened.
        unsafe { (*ctx.as_mut_ptr()).global_quality = rate.global_quality };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(p: &[(&'static str, &str)]) -> Vec<(&'static str, String)> {
        p.iter().map(|&(k, v)| (k, v.to_string())).collect()
    }

    #[test]
    fn the_default_quality_codes_h264_at_crf_23_and_qp_24() {
        assert_eq!(quality_crf(Codec::H264, DEFAULT_QUALITY), 23);
        assert_eq!(quality_qp(Codec::H264, DEFAULT_QUALITY), 24);
        let sw = rate(
            Backend::Software,
            Codec::H264,
            RateControl::Quality(DEFAULT_QUALITY),
        );
        assert_eq!(sw.options, pairs(&[("preset", "veryfast"), ("crf", "23")]));
        let va = rate(
            Backend::Vaapi { low_power: true },
            Codec::H264,
            RateControl::Quality(DEFAULT_QUALITY),
        );
        assert_eq!(
            va.options,
            pairs(&[("rc_mode", "CQP"), ("qp", "24"), ("low_power", "1")])
        );
        assert_eq!((sw.bit_rate, va.bit_rate), (0, 0));
    }

    #[test]
    fn vaapi_av1_takes_its_q_index_as_the_contexts_quality() {
        let lp = Backend::Vaapi { low_power: true };
        let av1 = rate(lp, Codec::Av1, RateControl::Quality(DEFAULT_QUALITY));
        assert_eq!(
            av1.options,
            pairs(&[("rc_mode", "CQP"), ("low_power", "1")])
        );
        assert_eq!(av1.global_quality, 128);
        let h265 = rate(lp, Codec::H265, RateControl::Quality(DEFAULT_QUALITY));
        assert!(h265.options.contains(&("qp", "28".into())));
        assert_eq!(h265.global_quality, 0, "H.265 takes its qp option");
        let codec = ffmpeg::encoder::find_by_name("libx264").expect("libx264");
        let mut ctx = ffmpeg::codec::Context::new_with_codec(codec)
            .encoder()
            .video()
            .unwrap();
        apply(&av1, &mut ctx);
        // SAFETY: reading a field of a context not yet opened.
        assert_eq!(unsafe { (*ctx.as_ptr()).global_quality }, 128);
    }

    #[test]
    fn quality_runs_down_each_codecs_scale_and_never_reaches_lossless() {
        for codec in Codec::ALL {
            let crf: Vec<u8> = (1..=100).map(|q| quality_crf(codec, q)).collect();
            assert!(crf.windows(2).all(|w| w[0] >= w[1]), "{}", codec.label());
            assert_eq!(*crf.last().unwrap(), 1);
        }
        assert_eq!(quality_crf(Codec::Av1, 1), 63);
        assert_eq!(quality_qp(Codec::Av1, 1), 255);
    }

    #[test]
    fn average_and_constant_set_each_backends_mode_and_the_rate() {
        let c = rate(Backend::Nvenc, Codec::H265, RateControl::Constant(8000));
        assert_eq!(c.options, pairs(&[("rc", "cbr"), ("preset", "p4")]));
        assert_eq!(
            (c.bit_rate, c.max_rate, c.buffer_size),
            (8_000_000, 8_000_000, 8_000_000)
        );
        let a = rate(Backend::Vulkan, Codec::Av1, RateControl::Average(8000));
        assert_eq!(a.options, pairs(&[("rc_mode", "vbr")]));
        assert_eq!((a.max_rate, a.buffer_size), (16_000_000, 16_000_000));
        let x = rate(Backend::Software, Codec::H264, RateControl::Constant(2000));
        assert!(x.options.contains(&("x264-params", "nal-hrd=cbr".into())));
        let s = rate(Backend::Software, Codec::Av1, RateControl::Constant(2000));
        assert!(
            s.options
                .contains(&("svtav1-params", "pred-struct=1".into()))
        );
    }
}
