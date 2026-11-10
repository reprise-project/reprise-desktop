//! Two single-reference H.264 streams, one per AVC444 view, rewritten into the one stream a
//! single decoder plays, each view predicting from its own previous picture.
#![cfg_attr(
    not(test),
    expect(dead_code, reason = "the two-encoder AVC444 path is its user")
)]

mod bits;
mod syntax;

use bits::nal_units;
use syntax::{Picture, Slice, Sps};

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
#[derive(Default)]
pub struct Merge {
    /// The main view's SPS as coded, and its fields.
    sps: Option<(Vec<u8>, Sps)>,
    /// The main view's PPS as coded, and its id.
    pps: Option<(Vec<u8>, u32)>,
    /// `frame_num` of the next picture.
    frame_num: u32,
    /// `frame_num` of the last main picture, None until a main IDR.
    last_main: Option<u32>,
    /// Whether an aux picture is long-term frame 0.
    aux_ref: bool,
}

impl Merge {
    /// The main view's next access unit (Annex B), as the merged stream codes it.
    pub fn main(&mut self, au: &[u8]) -> Result<Vec<u8>, Error> {
        let merged = self.merge_main(au);
        self.restart_on_error(merged)
    }

    /// The aux view's next access unit; its IDR becomes an I picture. An aux P picture needs
    /// an aux picture after the last main IDR.
    pub fn aux(&mut self, au: &[u8]) -> Result<Vec<u8>, Error> {
        let merged = self.merge_aux(au);
        self.restart_on_error(merged)
    }

    /// The encoders' references have moved past a picture the client never gets.
    fn restart_on_error<T>(&mut self, merged: Result<T, Error>) -> Result<T, Error> {
        if merged.is_err() {
            self.last_main = None;
            self.aux_ref = false;
        }
        merged
    }

    fn merge_main(&mut self, au: &[u8]) -> Result<Vec<u8>, Error> {
        let mut out = Vec::with_capacity(au.len() + 16);
        let mut slices = Vec::new();
        for nal in nal_units(au) {
            match nal[0] & 0x1f {
                SPS => {
                    let (rewritten, sps) = syntax::two_refs(nal)?;
                    self.sps = Some((nal.to_vec(), sps));
                    put(&mut out, &rewritten);
                }
                PPS => {
                    self.pps = Some((nal.to_vec(), syntax::pps_id(nal)?));
                    put(&mut out, nal);
                }
                _ => slices.extend(slice_nal(nal)?),
            }
        }
        let (slices, sps) = self.parse(&slices)?;
        let idr = slices[0].idr;
        let max = 1 << sps.log2_max_frame_num;
        let (frame_num, picture) = match (idr, self.last_main) {
            (true, _) => (0, Picture::MainIdr),
            (false, Some(last)) => {
                let drop = self.frame_num.wrapping_sub(last).wrapping_sub(1) & (max - 1);
                (self.frame_num, Picture::Main { drop })
            }
            (false, None) => return Err(Error::Order("a main non-IDR picture before a main IDR")),
        };
        for slice in &slices {
            put(&mut out, &slice.build(&picture, frame_num, sps));
        }
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
        let (slices, sps) = self.parse(&slices)?;
        if !self.aux_ref && slices.iter().any(Slice::is_p) {
            return Err(Error::Order(
                "an aux P picture with no aux since the main IDR",
            ));
        }
        let max = 1 << sps.log2_max_frame_num;
        let mut out = Vec::with_capacity(au.len() + 16);
        for slice in &slices {
            put(&mut out, &slice.build(&Picture::Aux, self.frame_num, sps));
        }
        self.aux_ref = true;
        self.frame_num = (self.frame_num + 1) & (max - 1);
        Ok(out)
    }

    /// One picture's slice NAL units, under the main view's parameter sets.
    fn parse(&self, nals: &[&[u8]]) -> Result<(Vec<Slice>, &Sps), Error> {
        let (Some((_, sps)), Some((_, pps_id))) = (&self.sps, &self.pps) else {
            return Err(Error::Order("a slice before the main view's SPS and PPS"));
        };
        let slices = (nals.iter())
            .map(|nal| Slice::parse(nal, sps))
            .collect::<Result<Vec<_>, _>>()?;
        let Some(first) = slices.first() else {
            return Err(Error::Malformed("an access unit without a slice"));
        };
        if slices.iter().any(|s| s.idr != first.idr) {
            return Err(Error::Malformed("IDR and non-IDR slices in one picture"));
        }
        if slices.iter().any(|s| s.pps_id != *pps_id) {
            return Err(Error::Unsupported("a slice under another PPS"));
        }
        Ok((slices, sps))
    }
}

const SPS: u8 = 7;
const PPS: u8 = 8;

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

    /// 150 frames of two 128x128 views from `h264_vaapi` at QP 30 with Reprise's options; both
    /// views are IDRs at frames 0, 6 and 120.
    const MAIN: &[u8] = fixture!("main.h264");
    const AUX: &[u8] = fixture!("aux.h264");
    /// The fixture merged, frames in [`SKIPPED`] without an aux view.
    const MERGED: &[u8] = fixture!("merged.h264");
    /// One IDR from libx264, which codes POC type 0.
    const X264: &[u8] = fixture!("x264.h264");
    const FRAMES: usize = 150;
    const SKIPPED: [usize; 14] = [4, 9, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 100, 121];

    /// A single-slice stream's access units, each slice with the NAL units before it.
    fn access_units(stream: &[u8]) -> Vec<Vec<u8>> {
        let mut aus = Vec::new();
        let mut au = Vec::new();
        for nal in nal_units(stream) {
            au.extend_from_slice(&[0, 0, 0, 1]);
            au.extend_from_slice(nal);
            if matches!(nal[0] & 0x1f, 1 | 5) {
                aus.push(std::mem::take(&mut au));
            }
        }
        aus
    }

    /// The fixture's pictures in send order: (aux, index in its own stream).
    fn plan() -> Vec<(bool, usize)> {
        let mut aux = 0;
        let mut plan = Vec::new();
        for t in 0..FRAMES {
            plan.push((false, t));
            if !SKIPPED.contains(&t) {
                plan.push((true, aux));
                aux += 1;
            }
        }
        plan
    }

    /// The fixture's views merged, one access unit per picture.
    fn merged() -> Vec<Vec<u8>> {
        let (main, aux) = (access_units(MAIN), access_units(AUX));
        let mut merge = Merge::default();
        plan()
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

    #[test]
    fn merges_the_fixture_views_into_the_expected_stream() {
        let merged = merged().concat();
        let first = merged.iter().zip(MERGED).position(|(a, b)| a != b);
        assert_eq!(first, None, "first differing byte");
        assert_eq!(merged.len(), MERGED.len());
    }

    #[test]
    fn the_merged_stream_decodes_to_each_views_own_pictures() {
        ::ffmpeg::util::log::set_level(::ffmpeg::util::log::Level::Quiet);
        let main = decode(&access_units(MAIN));
        let aux = decode(&access_units(AUX));
        assert_eq!((main.len(), aux.len()), (FRAMES, FRAMES - SKIPPED.len()));
        let merged = decode(&merged());
        let plan = plan();
        assert_eq!(merged.len(), plan.len());
        for (k, (is_aux, i)) in plan.into_iter().enumerate() {
            let own = if is_aux { &aux[i] } else { &main[i] };
            let view = if is_aux { "aux" } else { "main" };
            assert!(merged[k] == *own, "picture {k}, {view} {i}");
        }
    }

    #[test]
    fn an_aux_p_picture_with_no_aux_since_the_main_idr_is_refused() {
        let (main, aux) = (access_units(MAIN), access_units(AUX));
        let mut merge = Merge::default();
        merge.main(&main[0]).unwrap();
        merge.aux(&aux[0]).unwrap();
        merge.main(&main[1]).unwrap();
        merge.main(&main[6]).unwrap();
        assert!(matches!(merge.aux(&aux[1]), Err(Error::Order(_))));
    }

    #[test]
    fn after_a_refused_picture_only_a_main_idr_restarts_the_stream() {
        let (main, aux) = (access_units(MAIN), access_units(AUX));
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
        let (main, mut aux) = (access_units(MAIN), access_units(AUX));
        let mut merge = Merge::default();
        merge.main(&main[0]).unwrap();
        assert_eq!(aux[0][4] & 0x1f, 7, "the aux IDR starts with its SPS");
        aux[0][7] ^= 1;
        assert!(matches!(merge.aux(&aux[0]), Err(Error::Unsupported(_))));
    }
}
