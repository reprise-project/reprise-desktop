//! Two single-reference H.264 streams, one per AVC444 view, rewritten into the one stream a
//! single decoder plays, each view predicting from its own previous picture.

mod bits;
mod syntax;

use bits::nal_units;
use syntax::{Picture, Pps, Slice, Sps};

/// Why an access unit was not merged. The merged stream then waits for a main IDR.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// A stream feature the rewrite does not handle, such as POC type 0 or CAVLC.
    Unsupported(&'static str),
    /// Bytes that are not the syntax their NAL unit type says.
    Malformed(&'static str),
    /// A picture that cannot follow the ones merged before it.
    Order(&'static str),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Unsupported(what) => write!(f, "H.264 merge does not handle {what}"),
            Error::Malformed(what) => write!(f, "malformed H.264: {what}"),
            Error::Order(what) => write!(f, "H.264 merge out of order: {what}"),
        }
    }
}

impl std::error::Error for Error {}

/// Merges the access units of a main-view and an aux-view encoder, both FFmpeg's `h264_vaapi`
/// with one reference and no B-frames, fed in send order: each frame's main, then its aux.
///
/// The SPS allows two reference frames. A main picture is short-term and drops the main
/// picture before it (MMCO 1). An aux picture is never an IDR: it becomes long-term frame 0
/// (MMCO 4, MMCO 6) and predicts from long-term frame 0 alone, so a frame without an aux
/// leaves the last one in place. Slice data is copied unchanged.
///
/// One `Merge` lasts the session: reopened encoders start on IDRs, which it takes as a restart.
/// A frame with an `Err` must not be sent; the next IDR's `idr_pic_id` counts on that.
#[derive(Default)]
pub struct Merge {
    /// The main view's SPS as coded, and its fields.
    sps: Option<(Vec<u8>, Sps)>,
    /// The main view's PPS as coded, and its fields.
    pps: Option<(Vec<u8>, Pps)>,
    /// `frame_num` of the next picture.
    frame_num: u32,
    /// `frame_num` of the last main picture, None until a main IDR.
    last_main: Option<u32>,
    /// Whether an aux picture is long-term frame 0.
    aux_ref: bool,
    /// `idr_pic_id` if the last frame sent ended on a main IDR; the next IDR's must differ
    /// (7.4.3). Encoders restart theirs on a reopen, so the merge writes its own.
    sent_idr: Option<u16>,
    /// `idr_pic_id` if the current frame so far ends on a main IDR.
    frame_idr: Option<u16>,
    /// Whether the current frame had an `Err`, so it is not sent.
    frame_failed: bool,
}

impl Merge {
    /// The main view's next access unit (Annex B), as the merged stream codes it. Its
    /// parameter sets may change only on an IDR.
    pub fn main(&mut self, au: &[u8]) -> Result<Vec<u8>, Error> {
        if !self.frame_failed {
            self.sent_idr = self.frame_idr;
        }
        self.frame_failed = false;
        let merged = self.merge_main(au);
        self.restart_on_error(merged)
    }

    /// The aux view's next access unit; its IDR becomes an I picture. The first aux after a
    /// main IDR must be an IDR.
    pub fn aux(&mut self, au: &[u8]) -> Result<Vec<u8>, Error> {
        let merged = self.merge_aux(au);
        self.restart_on_error(merged)
    }

    /// The encoders' references have moved past a picture the client never gets.
    fn restart_on_error<T>(&mut self, merged: Result<T, Error>) -> Result<T, Error> {
        if merged.is_err() {
            self.last_main = None;
            self.aux_ref = false;
            self.frame_failed = true;
        }
        merged
    }

    fn merge_main(&mut self, au: &[u8]) -> Result<Vec<u8>, Error> {
        let mut out = Vec::with_capacity(au.len() + 16);
        let (mut sps, mut pps, mut slices) = (None, None, Vec::new());
        for nal in nal_units(au) {
            match nal[0] & 0x1f {
                SPS => {
                    let (rewritten, fields) = syntax::two_refs(nal)?;
                    sps = Some((nal.to_vec(), fields));
                    put(&mut out, &rewritten);
                }
                PPS => {
                    pps = Some((nal.to_vec(), syntax::pps(nal)?));
                    put(&mut out, nal);
                }
                _ => slices.extend(slice_nal(nal)?),
            }
        }
        let (Some((_, fields)), Some((_, pps_fields))) = (
            sps.as_ref().or(self.sps.as_ref()),
            pps.as_ref().or(self.pps.as_ref()),
        ) else {
            return Err(Error::Order("a slice before the main view's SPS and PPS"));
        };
        let slices = parse(&slices, fields, pps_fields)?;
        let idr = slices[0].idr;
        let max = 1 << fields.log2_max_frame_num;
        let (frame_num, picture) = match (idr, self.last_main) {
            (true, _) => (
                0,
                Picture::MainIdr {
                    idr_pic_id: u16::from(self.sent_idr == Some(0)),
                },
            ),
            (false, Some(last)) => {
                let drop = self.frame_num.wrapping_sub(last).wrapping_sub(1) & (max - 1);
                (self.frame_num, Picture::Main { drop })
            }
            (false, None) => return Err(Error::Order("a main non-IDR picture before a main IDR")),
        };
        if !idr && (changed(&sps, &self.sps) || changed(&pps, &self.pps)) {
            return Err(Error::Unsupported("a parameter set change outside an IDR"));
        }
        for slice in &slices {
            put(&mut out, &slice.build(&picture, frame_num, fields));
        }
        self.sps = sps.or(self.sps.take());
        self.pps = pps.or(self.pps.take());
        self.frame_idr = match picture {
            Picture::MainIdr { idr_pic_id } => Some(idr_pic_id),
            _ => None,
        };
        self.last_main = Some(frame_num);
        self.aux_ref &= !idr;
        self.frame_num = (frame_num + 1) & (max - 1);
        Ok(out)
    }

    fn merge_aux(&mut self, au: &[u8]) -> Result<Vec<u8>, Error> {
        if self.last_main.is_none() {
            return Err(Error::Order("an aux picture before a main IDR"));
        }
        let mut slices = Vec::new();
        for nal in nal_units(au) {
            let main = match nal[0] & 0x1f {
                SPS => self.sps.as_ref().map(|s| &s.0),
                PPS => self.pps.as_ref().map(|p| &p.0),
                _ => {
                    slices.extend(slice_nal(nal)?);
                    continue;
                }
            };
            if main.is_none_or(|main| main[..] != *nal) {
                return Err(Error::Unsupported(
                    "aux parameter sets other than the main's",
                ));
            }
        }
        let (Some((_, sps)), Some((_, pps))) = (&self.sps, &self.pps) else {
            return Err(Error::Order("a slice before the main view's SPS and PPS"));
        };
        let slices = parse(&slices, sps, pps)?;
        if !self.aux_ref && !slices[0].idr {
            return Err(Error::Order(
                "an aux picture other than an IDR first after a main IDR",
            ));
        }
        let max = 1 << sps.log2_max_frame_num;
        let mut out = Vec::with_capacity(au.len() + 16);
        for slice in &slices {
            put(&mut out, &slice.build(&Picture::Aux, self.frame_num, sps));
        }
        self.aux_ref = true;
        self.frame_idr = None;
        self.frame_num = (self.frame_num + 1) & (max - 1);
        Ok(out)
    }
}

/// One picture's slice NAL units: the first at macroblock 0, then in raster order, all with
/// one source `frame_num`.
fn parse(nals: &[&[u8]], sps: &Sps, pps: &Pps) -> Result<Vec<Slice>, Error> {
    let slices = (nals.iter())
        .map(|nal| Slice::parse(nal, sps, pps))
        .collect::<Result<Vec<_>, _>>()?;
    let Some(first) = slices.first() else {
        return Err(Error::Malformed("an access unit without a slice"));
    };
    let one_picture = first.first_mb == 0
        && slices.windows(2).all(|w| w[0].first_mb < w[1].first_mb)
        && (slices.iter()).all(|s| s.idr == first.idr && s.frame_num == first.frame_num);
    if !one_picture {
        return Err(Error::Malformed("more than one picture in an access unit"));
    }
    Ok(slices)
}

const SPS: u8 = 7;
const PPS: u8 = 8;

/// Whether `new`, a parameter set an access unit carries, differs from `old`.
fn changed<T>(new: &Option<(Vec<u8>, T)>, old: &Option<(Vec<u8>, T)>) -> bool {
    (new.as_ref()).is_some_and(|(new, _)| old.as_ref().is_none_or(|(old, _)| old != new))
}

/// `nal` if it is a slice; None for SEI and access unit delimiters, which the merged stream
/// leaves out.
fn slice_nal(nal: &[u8]) -> Result<Option<&[u8]>, Error> {
    match nal[0] & 0x1f {
        1 | 5 => Ok(Some(nal)),
        6 | 9 => Ok(None),
        _ => Err(Error::Unsupported(
            "NAL units other than slices, SEI and parameter sets",
        )),
    }
}

/// `nal` after a four-byte start code.
fn put(out: &mut Vec<u8>, nal: &[u8]) {
    out.extend_from_slice(&[0, 0, 0, 1]);
    out.extend_from_slice(nal);
}

#[cfg(test)]
mod tests {
    use super::bits::{Reader, Writer, escape, unescape};
    use super::*;

    macro_rules! fixture {
        ($name:literal) => {
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/avc444/",
                $name
            ))
        };
    }

    /// Two 128x128 views from `h264_vaapi` at QP 30 with Reprise's options, and the frames
    /// sent without an aux view.
    struct Fixture {
        main: &'static [u8],
        aux: &'static [u8],
        frames: usize,
        skipped: &'static [usize],
    }

    /// One slice per picture; both views are IDRs at frames 0, 6 and 120.
    const ONE_SLICE: Fixture = Fixture {
        main: fixture!("main.h264"),
        aux: fixture!("aux.h264"),
        frames: 150,
        skipped: &[4, 9, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 100, 121],
    };
    /// Three slices per picture; both views are IDRs at frames 0 and 10.
    const THREE_SLICES: Fixture = Fixture {
        main: fixture!("main-slices.h264"),
        aux: fixture!("aux-slices.h264"),
        frames: 24,
        skipped: &[4, 15],
    };
    /// [`ONE_SLICE`] merged.
    const MERGED: &[u8] = fixture!("merged.h264");
    /// One IDR from libx264, which codes POC type 0.
    const X264: &[u8] = fixture!("x264.h264");
    /// The fixtures' `log2_max_frame_num`; their PPS id is 0.
    const LOG2_MAX_FRAME_NUM: u32 = 8;

    /// A stream's access units, each picture's slices with the NAL units before them.
    fn access_units(stream: &[u8]) -> Vec<Vec<u8>> {
        let mut aus = Vec::new();
        let mut au = Vec::new();
        let mut sliced = false;
        for nal in nal_units(stream) {
            let slice = matches!(nal[0] & 0x1f, 1 | 5);
            // first_mb_in_slice 0 is the one-bit code 1.
            if sliced && (!slice || nal[1] & 0x80 != 0) {
                aus.push(std::mem::take(&mut au));
                sliced = false;
            }
            put(&mut au, nal);
            sliced |= slice;
        }
        aus.push(au);
        aus
    }

    /// An access unit of `nals`.
    fn au<'a>(nals: impl IntoIterator<Item = &'a [u8]>) -> Vec<u8> {
        let mut au = Vec::new();
        nals.into_iter().for_each(|nal| put(&mut au, nal));
        au
    }

    /// A NAL unit with `header` and the RBSP `fields` writes.
    fn nal(header: u8, fields: impl FnOnce(&mut Writer)) -> Vec<u8> {
        let mut w = Writer::default();
        fields(&mut w);
        let mut nal = vec![header];
        nal.extend(escape(&w.into_bytes()));
        nal
    }

    /// A P slice under the fixtures' parameter sets, overriding the PPS's
    /// `num_ref_idx_l0_active_minus1` with `active` if given.
    fn p_slice(nal_ref_idc: u8, active: Option<u32>) -> Vec<u8> {
        au([&nal(nal_ref_idc << 5 | 1, |w| {
            [0, 5, 0].into_iter().for_each(|v| w.ue(v)); // first_mb_in_slice, P, pps id
            w.u(LOG2_MAX_FRAME_NUM, 1);
            w.flag(active.is_some());
            active.into_iter().for_each(|v| w.ue(v));
            w.flag(false); // ref_pic_list_modification_flag_l0
            if nal_ref_idc != 0 {
                w.flag(false); // adaptive_ref_pic_marking_mode_flag
            }
            w.ue(0); // cabac_init_idc
            w.se(0);
            while !w.aligned() {
                w.flag(true);
            }
            w.u(8, 0x80);
        })[..]])
    }

    /// The `idr_pic_id` of the last slice in `au`, an IDR.
    fn idr_pic_id(au: &[u8]) -> u32 {
        let nal = *nal_units(au).last().unwrap();
        assert_eq!(nal[0] & 0x1f, 5, "an IDR slice");
        let rbsp = unescape(&nal[1..]);
        let mut r = Reader::new(&rbsp);
        (0..3).for_each(|_| _ = r.ue().unwrap());
        r.u(LOG2_MAX_FRAME_NUM).unwrap();
        r.ue().unwrap()
    }

    /// The fixture's pictures in send order: (aux, index in its own stream).
    fn plan(f: &Fixture) -> Vec<(bool, usize)> {
        let mut aux = 0;
        let mut plan = Vec::new();
        for t in 0..f.frames {
            plan.push((false, t));
            if !f.skipped.contains(&t) {
                plan.push((true, aux));
                aux += 1;
            }
        }
        plan
    }

    /// The fixture's views merged, one access unit per picture.
    fn merged(f: &Fixture) -> Vec<Vec<u8>> {
        let (main, aux) = (access_units(f.main), access_units(f.aux));
        let mut merge = Merge::default();
        plan(f)
            .into_iter()
            .map(|(is_aux, i)| match is_aux {
                false => merge.main(&main[i]).unwrap(),
                true => merge.aux(&aux[i]).unwrap(),
            })
            .collect()
    }

    /// Each picture FFmpeg's software decoder makes of `aus`, as its cropped Y, U and V planes.
    fn decode(aus: &[Vec<u8>]) -> Vec<Vec<u8>> {
        let codec = ::ffmpeg::decoder::find(::ffmpeg::codec::Id::H264).unwrap();
        let mut decoder = ::ffmpeg::codec::Context::new_with_codec(codec)
            .decoder()
            .video()
            .unwrap();
        let mut pictures = Vec::new();
        let mut frame = ::ffmpeg::frame::Video::empty();
        let mut drain = |decoder: &mut ::ffmpeg::decoder::Video| {
            while decoder.receive_frame(&mut frame).is_ok() {
                let mut planes = Vec::new();
                for p in 0..3 {
                    let shift = usize::from(p > 0);
                    let (w, h) = (frame.width() as usize, frame.height() as usize);
                    let (stride, data) = (frame.stride(p), frame.data(p));
                    for y in 0..h >> shift {
                        planes.extend_from_slice(&data[y * stride..][..w >> shift]);
                    }
                }
                pictures.push(planes);
            }
        };
        for au in aus {
            decoder.send_packet(&::ffmpeg::Packet::copy(au)).unwrap();
            drain(&mut decoder);
        }
        decoder.send_eof().unwrap();
        drain(&mut decoder);
        pictures
    }

    fn assert_decodes_to_each_views_own_pictures(f: &Fixture) {
        ::ffmpeg::util::log::set_level(::ffmpeg::util::log::Level::Quiet);
        let main = decode(&access_units(f.main));
        let aux = decode(&access_units(f.aux));
        assert_eq!(
            (main.len(), aux.len()),
            (f.frames, f.frames - f.skipped.len())
        );
        let merged = decode(&merged(f));
        let plan = plan(f);
        assert_eq!(merged.len(), plan.len());
        for (k, (is_aux, i)) in plan.into_iter().enumerate() {
            let own = if is_aux { &aux[i] } else { &main[i] };
            let view = if is_aux { "aux" } else { "main" };
            assert!(merged[k] == *own, "picture {k}, {view} {i}");
        }
    }

    #[test]
    fn merges_the_fixture_views_into_the_expected_stream() {
        let merged = merged(&ONE_SLICE).concat();
        let first = merged.iter().zip(MERGED).position(|(a, b)| a != b);
        assert_eq!(first, None, "first differing byte");
        assert_eq!(merged.len(), MERGED.len());
    }

    #[test]
    fn the_merged_stream_decodes_to_each_views_own_pictures() {
        assert_decodes_to_each_views_own_pictures(&ONE_SLICE);
    }

    #[test]
    fn a_merged_three_slice_stream_decodes_to_each_views_own_pictures() {
        assert_eq!(nal_units(&access_units(THREE_SLICES.main)[1]).len(), 3);
        assert_decodes_to_each_views_own_pictures(&THREE_SLICES);
    }

    #[test]
    fn the_first_aux_after_a_main_idr_must_be_an_idr() {
        let (main, aux) = (access_units(ONE_SLICE.main), access_units(ONE_SLICE.aux));
        let mut merge = Merge::default();
        merge.main(&main[0]).unwrap();
        let aux_i = merge.aux(&aux[0]).unwrap();
        merge.main(&main[1]).unwrap();
        merge.main(&main[6]).unwrap();
        assert!(matches!(merge.aux(&aux[1]), Err(Error::Order(_))));
        merge.main(&main[6]).unwrap();
        assert_eq!(nal_units(&aux_i)[0][0] & 0x1f, 1, "a non-IDR I picture");
        assert!(matches!(merge.aux(&aux_i), Err(Error::Order(_))));
    }

    #[test]
    fn after_a_refused_picture_only_a_main_idr_restarts_the_stream() {
        let (main, aux) = (access_units(ONE_SLICE.main), access_units(ONE_SLICE.aux));
        let mut merge = Merge::default();
        merge.main(&main[0]).unwrap();
        merge.aux(&aux[0]).unwrap();
        assert!(matches!(
            merge.main(&main[1][..6]),
            Err(Error::Malformed(_))
        ));
        assert!(matches!(merge.main(&main[2]), Err(Error::Order(_))));
        assert!(matches!(merge.aux(&aux[1]), Err(Error::Order(_))));
        merge.main(&main[6]).unwrap();
        merge.aux(&aux[5]).unwrap();
        merge.main(&main[7]).unwrap();
        merge.aux(&aux[6]).unwrap();
    }

    #[test]
    fn a_poc_type_0_stream_is_unsupported() {
        let mut merge = Merge::default();
        assert!(matches!(merge.main(X264), Err(Error::Unsupported(_))));
    }

    #[test]
    fn an_aux_sps_other_than_the_main_one_is_unsupported() {
        let (main, mut aux) = (access_units(ONE_SLICE.main), access_units(ONE_SLICE.aux));
        let mut merge = Merge::default();
        merge.main(&main[0]).unwrap();
        assert_eq!(aux[0][4] & 0x1f, 7, "the aux IDR starts with its SPS");
        aux[0][7] ^= 1;
        assert!(matches!(merge.aux(&aux[0]), Err(Error::Unsupported(_))));
    }

    #[test]
    fn a_p_slice_with_more_than_one_active_reference_is_unsupported() {
        let main = access_units(ONE_SLICE.main);
        let mut merge = Merge::default();
        merge.main(&main[0]).unwrap();
        merge.main(&p_slice(2, Some(0))).unwrap();
        assert!(matches!(
            merge.main(&p_slice(2, Some(1))),
            Err(Error::Unsupported(_))
        ));

        let pps = nal(0x68, |w| {
            [0, 0].into_iter().for_each(|v| w.ue(v)); // pps id, sps id
            w.flag(true); // CABAC
            w.flag(false);
            w.ue(0); // num_slice_groups_minus1
            w.ue(1); // num_ref_idx_l0_default_active_minus1
            w.ue(0);
            w.u(3, 0); // weighted_pred_flag, weighted_bipred_idc
            (0..3).for_each(|_| w.se(0));
            w.u(3, 0); // deblocking_filter_control_present_flag, ..
            w.flag(true); // rbsp_stop_one_bit
        });
        let idr = nal_units(&main[0]);
        let sps = idr.iter().find(|n| n[0] & 0x1f == SPS).unwrap();
        merge
            .main(&au([sps, &pps[..], idr.last().unwrap()]))
            .unwrap();
        merge.main(&p_slice(2, Some(0))).unwrap();
        assert!(matches!(
            merge.main(&p_slice(2, None)),
            Err(Error::Unsupported(_))
        ));
    }

    #[test]
    fn a_non_reference_picture_is_unsupported() {
        let main = access_units(ONE_SLICE.main);
        let mut merge = Merge::default();
        merge.main(&main[0]).unwrap();
        assert!(matches!(
            merge.main(&p_slice(0, None)),
            Err(Error::Unsupported(_))
        ));
    }

    #[test]
    fn consecutive_main_idrs_get_distinct_idr_pic_ids() {
        let (main, aux) = (access_units(ONE_SLICE.main), access_units(ONE_SLICE.aux));
        let mut merge = Merge::default();
        let first = idr_pic_id(&merge.main(&main[0]).unwrap());
        let second = idr_pic_id(&merge.main(&main[0]).unwrap());
        assert_ne!(first, second);
        assert!(merge.main(&main[1][..6]).is_err());
        let third = idr_pic_id(&merge.main(&main[0]).unwrap());
        assert_ne!(third, second);
        merge.main(&main[1]).unwrap();
        assert!(merge.aux(&aux[1]).is_err(), "the frame is not sent");
        assert_ne!(idr_pic_id(&merge.main(&main[0]).unwrap()), third);
    }

    #[test]
    fn two_pictures_in_one_access_unit_are_malformed() {
        let main = access_units(ONE_SLICE.main);
        let mut merge = Merge::default();
        merge.main(&main[0]).unwrap();
        assert!(matches!(
            merge.main(&[&main[1][..], &main[2]].concat()),
            Err(Error::Malformed(_))
        ));

        let main = access_units(THREE_SLICES.main);
        merge.main(&main[0]).unwrap();
        let reversed = au(nal_units(&main[1]).into_iter().rev());
        assert!(matches!(merge.main(&reversed), Err(Error::Malformed(_))));
    }

    #[test]
    fn an_sps_with_log2_max_frame_num_over_16_is_malformed() {
        let sps = nal(0x67, |w| {
            [100, 0, 30].into_iter().for_each(|v| w.u(8, v)); // High, level 3
            [0, 1, 0, 0].into_iter().for_each(|v| w.ue(v)); // id, 4:2:0, 8-bit
            w.u(2, 0); // qpprime_y_zero_transform_bypass_flag, seq_scaling_matrix_present_flag
            w.ue(u32::MAX - 1); // log2_max_frame_num_minus4
            w.flag(true);
        });
        let mut merge = Merge::default();
        assert!(matches!(
            merge.main(&au([&sps[..]])),
            Err(Error::Malformed(_))
        ));
    }

    #[test]
    fn a_parameter_set_change_outside_an_idr_is_unsupported() {
        let main = access_units(ONE_SLICE.main);
        let mut merge = Merge::default();
        merge.main(&main[0]).unwrap();
        let sps = nal_units(&main[0])[0].to_vec();
        assert_eq!(sps[0] & 0x1f, SPS);
        merge
            .main(&[au([&sps[..]]), main[1].clone()].concat())
            .unwrap();
        let mut level = sps;
        level[3] ^= 1;
        assert!(matches!(
            merge.main(&[au([&level[..]]), main[2].clone()].concat()),
            Err(Error::Unsupported(_))
        ));
    }

    #[test]
    fn mutated_access_units_are_refused_without_a_panic() {
        let streams = [
            ONE_SLICE.main,
            ONE_SLICE.aux,
            THREE_SLICES.main,
            THREE_SLICES.aux,
            X264,
        ];
        let streams: Vec<_> = streams.into_iter().map(access_units).collect();
        let (mut ok, mut err) = (0, 0);
        for seed in 1..=3000_u64 {
            let mut x = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15);
            let mut below = |n: usize| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                (x % n as u64) as usize
            };
            let mut merge = Merge::default();
            for step in 0..1 + below(8) {
                let stream = &streams[below(streams.len())];
                let mut au = stream[if step == 0 { 0 } else { below(stream.len()) }].clone();
                for _ in 0..below(4) {
                    let i = below(au.len());
                    match below(4) {
                        0 => au[i] ^= 1 << below(8),
                        1 => au[i] = [0, 1, 3, 0xff][below(4)],
                        2 => au.truncate(i),
                        _ => au.insert(i, [0, 3][below(2)]),
                    }
                    if au.is_empty() {
                        break;
                    }
                }
                let merged = match below(2) {
                    0 => merge.main(&au),
                    _ => merge.aux(&au),
                };
                if merged.is_ok() { ok += 1 } else { err += 1 }
            }
        }
        assert!(ok > 100 && err > 100, "{ok} merged, {err} refused");
    }
}
