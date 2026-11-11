//! The SPS, PPS and slice header syntax of H.264 7.3, as far as `h264_vaapi`, `h264_nvenc` and
//! libx264 with one reference code it.

use super::Error;
use super::bits::{Reader, Writer, escape, unescape};

/// What a slice header's syntax needs from the SPS.
pub struct Sps {
    pub log2_max_frame_num: u32,
}

/// The reference frames the merged stream holds: the last main and the last aux picture.
const REFS: u32 = 2;

fn unsupported(ok: bool, what: &'static str) -> Result<(), Error> {
    match ok {
        true => Ok(()),
        false => Err(Error::Unsupported(what)),
    }
}

/// Reads fields and writes them back unchanged unless replaced.
struct Rewrite<'a> {
    r: Reader<'a>,
    w: Writer,
}

impl Rewrite<'_> {
    fn u(&mut self, n: u32) -> Result<u32, Error> {
        let v = self.r.u(n)?;
        self.w.u(n, v);
        Ok(v)
    }

    fn flag(&mut self) -> Result<bool, Error> {
        Ok(self.u(1)? == 1)
    }

    fn ue(&mut self) -> Result<u32, Error> {
        let v = self.r.ue()?;
        self.w.ue(v);
        Ok(v)
    }

    fn replace_ue(&mut self, v: u32) -> Result<(), Error> {
        self.r.ue()?;
        self.w.ue(v);
        Ok(())
    }
}

/// `nal`, an SPS, with `max_num_ref_frames` and `max_dec_frame_buffering` [`REFS`] and without
/// HRD or pic_struct, and its fields. A level that fits a frame holds at least two in its DPB.
pub fn two_refs(nal: &[u8]) -> Result<(Vec<u8>, Sps), Error> {
    let rbsp = unescape(&nal[1..]);
    let mut c = Rewrite {
        r: Reader::new(&rbsp),
        w: Writer::default(),
    };
    unsupported(c.u(8)? == 100, "a profile other than High")?;
    c.u(8)?; // constraint_set flags
    c.u(8)?; // level_idc
    c.ue()?; // seq_parameter_set_id
    unsupported(c.ue()? == 1, "chroma other than 4:2:0")?;
    c.ue()?; // bit_depth_luma_minus8
    c.ue()?; // bit_depth_chroma_minus8
    c.flag()?; // qpprime_y_zero_transform_bypass_flag
    unsupported(!c.flag()?, "scaling matrices")?;
    let log2_max_frame_num_minus4 = c.ue()?;
    if log2_max_frame_num_minus4 > 12 {
        return Err(Error::Malformed("log2_max_frame_num over 16"));
    }
    let log2_max_frame_num = log2_max_frame_num_minus4 + 4;
    unsupported(c.ue()? == 2, "a POC type other than 2")?;
    c.replace_ue(REFS)?; // max_num_ref_frames
    c.flag()?; // gaps_in_frame_num_value_allowed_flag
    c.ue()?; // pic_width_in_mbs_minus1
    c.ue()?; // pic_height_in_map_units_minus1
    unsupported(c.flag()?, "field coding")?;
    c.flag()?; // direct_8x8_inference_flag
    if c.flag()? {
        for _ in 0..4 {
            c.ue()?; // frame_crop offsets
        }
    }
    unsupported(c.flag()?, "an SPS without VUI")?;
    if c.flag()? && c.u(8)? == 255 {
        c.u(32)?; // sar_width, sar_height
    }
    if c.flag()? {
        c.flag()?; // overscan_appropriate_flag
    }
    if c.flag()? {
        c.u(4)?; // video_format, video_full_range_flag
        if c.flag()? {
            c.u(24)?; // colour_primaries, transfer_characteristics, matrix_coefficients
        }
    }
    if c.flag()? {
        c.ue()?; // chroma_sample_loc_type_top_field
        c.ue()?; // chroma_sample_loc_type_bottom_field
    }
    if c.flag()? {
        c.u(32)?; // num_units_in_tick
        c.u(32)?; // time_scale
        c.flag()?; // fixed_frame_rate_flag
    }
    let nal_hrd = c.r.flag()?;
    if nal_hrd {
        skip_hrd(&mut c.r)?;
    }
    let vcl_hrd = c.r.flag()?;
    if vcl_hrd {
        skip_hrd(&mut c.r)?;
    }
    if nal_hrd || vcl_hrd {
        c.r.flag()?; // low_delay_hrd_flag
    }
    c.r.flag()?; // pic_struct_present_flag
    // HRD and pic_struct require the buffering period and pic timing SEI the merge drops.
    c.w.u(3, 0); // nal_hrd_parameters_present_flag, vcl_hrd_.., pic_struct_present_flag
    unsupported(c.flag()?, "an SPS without bitstream restrictions")?;
    c.flag()?; // motion_vectors_over_pic_boundaries_flag
    for _ in 0..5 {
        c.ue()?; // max_bytes_per_pic_denom .. max_num_reorder_frames
    }
    c.replace_ue(REFS)?; // max_dec_frame_buffering
    if !c.r.flag()? {
        return Err(Error::Malformed("fields after the SPS's VUI"));
    }
    c.w.flag(true); // rbsp_stop_one_bit
    let mut out = vec![nal[0]];
    out.extend(escape(&c.w.into_bytes()));
    Ok((out, Sps { log2_max_frame_num }))
}

/// Reads past `hrd_parameters()` (E.1.2).
fn skip_hrd(r: &mut Reader) -> Result<(), Error> {
    let cpb_cnt_minus1 = r.ue()?;
    if cpb_cnt_minus1 > 31 {
        return Err(Error::Malformed("a cpb_cnt_minus1 over 31"));
    }
    r.u(8)?; // bit_rate_scale, cpb_size_scale
    for _ in 0..=cpb_cnt_minus1 {
        r.ue()?; // bit_rate_value_minus1
        r.ue()?; // cpb_size_value_minus1
        r.flag()?; // cbr_flag
    }
    r.u(20)?; // the three delay lengths and time_offset_length
    Ok(())
}

/// What a slice header's syntax needs from the PPS.
pub struct Pps {
    pub id: u32,
    num_ref_idx_l0_default_active_minus1: u32,
    deblocking_fields: bool,
}

/// The fields of `nal`, a PPS, if [`Slice::parse`] reads its slice headers: CABAC, one slice
/// group, no weighted prediction, no redundant-picture fields.
pub fn pps(nal: &[u8]) -> Result<Pps, Error> {
    let rbsp = unescape(&nal[1..]);
    let mut r = Reader::new(&rbsp);
    let id = r.ue()?;
    r.ue()?; // seq_parameter_set_id
    unsupported(r.flag()?, "CAVLC")?;
    r.flag()?; // bottom_field_pic_order_in_frame_present_flag: no field under POC type 2
    unsupported(r.ue()? == 0, "slice groups")?;
    let num_ref_idx_l0_default_active_minus1 = r.ue()?;
    r.ue()?; // num_ref_idx_l1_default_active_minus1
    unsupported(!r.flag()?, "weighted prediction")?;
    r.u(2)?; // weighted_bipred_idc
    r.se()?; // pic_init_qp_minus26
    r.se()?; // pic_init_qs_minus26
    r.se()?; // chroma_qp_index_offset
    let deblocking_fields = r.flag()?;
    r.flag()?; // constrained_intra_pred_flag
    unsupported(!r.flag()?, "redundant pictures")?;
    Ok(Pps {
        id,
        num_ref_idx_l0_default_active_minus1,
        deblocking_fields,
    })
}

/// How a slice's picture is marked and predicted in the merged stream.
pub enum Picture {
    MainIdr {
        idr_pic_id: u16,
    },
    /// A main picture that drops the previous main, `difference_of_pic_nums_minus1` before it.
    Main {
        drop: u32,
    },
    Aux,
}

/// A slice header's fields the rewrite keeps, and the RBSP from its slice data on.
pub struct Slice {
    pub idr: bool,
    pub first_mb: u32,
    /// `frame_num` as the source encoder coded it.
    pub frame_num: u32,
    pps_id: u32,
    slice_type: u32,
    no_output_of_prior_pics: bool,
    cabac_init_idc: u32,
    qp_delta: i32,
    /// disable_deblocking_filter_idc and, unless it is 1, the alpha and beta offsets.
    deblocking: Option<(u32, i32, i32)>,
    data: Vec<u8>,
}

impl Slice {
    /// `nal`, a P or I reference slice under `pps`, a PPS [`pps`] accepts. Its P slices
    /// must have one active reference: with more, CABAC slice data codes `ref_idx_l0`.
    pub fn parse(nal: &[u8], sps: &Sps, pps: &Pps) -> Result<Slice, Error> {
        let idr = nal[0] & 0x1f == 5;
        unsupported(nal[0] >> 5 != 0, "a non-reference picture")?;
        let rbsp = unescape(&nal[1..]);
        let mut r = Reader::new(&rbsp);
        let first_mb = r.ue()?;
        let slice_type = r.ue()?;
        if slice_type > 9 {
            return Err(Error::Malformed("a slice_type over 9"));
        }
        unsupported(matches!(slice_type % 5, 0 | 2), "slices other than P and I")?;
        let pps_id = r.ue()?;
        unsupported(pps_id == pps.id, "a slice under another PPS")?;
        let frame_num = r.u(sps.log2_max_frame_num)?;
        if idr {
            r.ue()?; // idr_pic_id
        }
        if slice_type.is_multiple_of(5) {
            let mut active_minus1 = pps.num_ref_idx_l0_default_active_minus1;
            if r.flag()? {
                active_minus1 = r.ue()?;
            }
            unsupported(active_minus1 == 0, "more than one active reference")?;
            unsupported(!r.flag()?, "a reordered reference list")?;
        }
        let mut no_output_of_prior_pics = false;
        if idr {
            no_output_of_prior_pics = r.flag()?;
            unsupported(!r.flag()?, "a long-term IDR")?;
        } else if r.flag()? {
            // The encoder's MMCOs drop its own previous picture; build writes the merged ones.
            loop {
                match r.ue()? {
                    0 => break,
                    1 | 2 | 4 | 6 => _ = r.ue()?,
                    3 => _ = (r.ue()?, r.ue()?),
                    5 => {}
                    _ => return Err(Error::Malformed("an MMCO over 6")),
                }
            }
        }
        let cabac_init_idc = if slice_type % 5 == 2 { 0 } else { r.ue()? };
        let qp_delta = r.se()?;
        let deblocking = match pps.deblocking_fields {
            false => None,
            true => match r.ue()? {
                1 => Some((1, 0, 0)),
                idc @ (0 | 2) => Some((idc, r.se()?, r.se()?)),
                _ => return Err(Error::Malformed("a disable_deblocking_filter_idc over 2")),
            },
        };
        while !r.aligned() {
            if !r.flag()? {
                return Err(Error::Malformed("a cabac_alignment_one_bit of 0"));
            }
        }
        Ok(Slice {
            idr,
            first_mb,
            frame_num,
            pps_id,
            slice_type,
            no_output_of_prior_pics,
            cabac_init_idc,
            qp_delta,
            deblocking,
            data: r.rest().to_vec(),
        })
    }

    pub fn is_p(&self) -> bool {
        self.slice_type.is_multiple_of(5)
    }

    /// The slice NAL unit as `picture` at `frame_num`, a reference picture with one active
    /// reference.
    pub fn build(&self, picture: &Picture, frame_num: u32, sps: &Sps) -> Vec<u8> {
        let idr_pic_id = match picture {
            Picture::MainIdr { idr_pic_id } => Some(*idr_pic_id),
            _ => None,
        };
        let mut w = Writer::default();
        w.u(1, 0); // forbidden_zero_bit
        w.u(2, 3); // nal_ref_idc
        w.u(5, if idr_pic_id.is_some() { 5 } else { 1 });
        w.ue(self.first_mb);
        w.ue(self.slice_type);
        w.ue(self.pps_id);
        w.u(sps.log2_max_frame_num, frame_num);
        if let Some(id) = idr_pic_id {
            w.ue(id.into());
        }
        if self.is_p() {
            w.flag(true); // num_ref_idx_active_override_flag
            w.ue(0);
            let aux = matches!(picture, Picture::Aux);
            w.flag(aux); // ref_pic_list_modification_flag_l0
            if aux {
                // modification_of_pic_nums_idc 2 with long_term_pic_num 0, then 3 to end.
                [2, 0, 3].into_iter().for_each(|v| w.ue(v));
            }
        }
        match picture {
            Picture::MainIdr { .. } => {
                w.flag(self.no_output_of_prior_pics);
                w.flag(false); // long_term_reference_flag
            }
            Picture::Main { drop } => {
                w.flag(true); // adaptive_ref_pic_marking_mode_flag
                [1, *drop, 0].into_iter().for_each(|v| w.ue(v));
            }
            Picture::Aux => {
                w.flag(true);
                // MMCO 4 with max_long_term_frame_idx_plus1 1, MMCO 6 with long_term_frame_idx 0.
                [4, 1, 6, 0, 0].into_iter().for_each(|v| w.ue(v));
            }
        }
        if self.slice_type % 5 != 2 {
            w.ue(self.cabac_init_idc);
        }
        w.se(self.qp_delta);
        if let Some((idc, alpha, beta)) = self.deblocking {
            w.ue(idc);
            if idc != 1 {
                w.se(alpha);
                w.se(beta);
            }
        }
        while !w.aligned() {
            w.flag(true); // cabac_alignment_one_bit
        }
        let header = w.into_bytes();
        let mut rbsp = header[1..].to_vec();
        rbsp.extend_from_slice(&self.data);
        let mut nal = vec![header[0]];
        nal.extend(escape(&rbsp));
        nal
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `h264_nvenc`'s SPS for 1920x1080 with Reprise's options: NAL HRD, pic_struct, two refs.
    const NVENC_SPS: [u8; 39] = [
        0x67, 0x64, 0x00, 0x2a, 0xac, 0x2b, 0x60, 0x3c, 0x01, 0x13, 0x60, 0x2d, 0xc0, 0x40, 0x40,
        0x50, 0x00, 0x00, 0x3e, 0x80, 0x00, 0x1d, 0x4c, 0x0e, 0x00, 0x00, 0x07, 0xa1, 0x20, 0x00,
        0x07, 0xa1, 0x20, 0xbb, 0xca, 0x23, 0xe3, 0x85, 0x5c,
    ];

    /// Every field of an SPS in the subset [`two_refs`] reads, HRD fields named `hrd ...`.
    struct Fields<'a> {
        r: Reader<'a>,
        out: Vec<(&'static str, u32)>,
    }

    impl Fields<'_> {
        fn u(&mut self, name: &'static str, n: u32) -> u32 {
            let v = self.r.u(n).unwrap();
            self.out.push((name, v));
            v
        }

        fn ue(&mut self, name: &'static str) -> u32 {
            let v = self.r.ue().unwrap();
            self.out.push((name, v));
            v
        }

        fn hrd(&mut self) {
            let cpb_cnt = self.ue("hrd cpb_cnt_minus1") + 1;
            self.u("hrd bit_rate_scale", 4);
            self.u("hrd cpb_size_scale", 4);
            for _ in 0..cpb_cnt {
                self.ue("hrd bit_rate_value_minus1");
                self.ue("hrd cpb_size_value_minus1");
                self.u("hrd cbr_flag", 1);
            }
            for name in [
                "hrd initial_cpb_removal_delay_length_minus1",
                "hrd cpb_removal_delay_length_minus1",
                "hrd dpb_output_delay_length_minus1",
                "hrd time_offset_length",
            ] {
                self.u(name, 5);
            }
        }
    }

    fn fields(nal: &[u8]) -> Vec<(&'static str, u32)> {
        let rbsp = unescape(&nal[1..]);
        let mut f = Fields {
            r: Reader::new(&rbsp),
            out: Vec::new(),
        };
        f.u("profile_idc", 8);
        f.u("constraint_set_flags", 8);
        f.u("level_idc", 8);
        f.ue("seq_parameter_set_id");
        f.ue("chroma_format_idc");
        f.ue("bit_depth_luma_minus8");
        f.ue("bit_depth_chroma_minus8");
        f.u("qpprime_y_zero_transform_bypass_flag", 1);
        f.u("seq_scaling_matrix_present_flag", 1);
        f.ue("log2_max_frame_num_minus4");
        f.ue("pic_order_cnt_type");
        f.ue("max_num_ref_frames");
        f.u("gaps_in_frame_num_value_allowed_flag", 1);
        f.ue("pic_width_in_mbs_minus1");
        f.ue("pic_height_in_map_units_minus1");
        f.u("frame_mbs_only_flag", 1);
        f.u("direct_8x8_inference_flag", 1);
        if f.u("frame_cropping_flag", 1) == 1 {
            (0..4).for_each(|_| _ = f.ue("frame_crop_offset"));
        }
        f.u("vui_parameters_present_flag", 1);
        if f.u("aspect_ratio_info_present_flag", 1) == 1 && f.u("aspect_ratio_idc", 8) == 255 {
            f.u("sar", 32);
        }
        if f.u("overscan_info_present_flag", 1) == 1 {
            f.u("overscan_appropriate_flag", 1);
        }
        if f.u("video_signal_type_present_flag", 1) == 1 {
            f.u("video_format", 3);
            f.u("video_full_range_flag", 1);
            if f.u("colour_description_present_flag", 1) == 1 {
                f.u("colour_primaries", 8);
                f.u("transfer_characteristics", 8);
                f.u("matrix_coefficients", 8);
            }
        }
        if f.u("chroma_loc_info_present_flag", 1) == 1 {
            f.ue("chroma_sample_loc_type_top_field");
            f.ue("chroma_sample_loc_type_bottom_field");
        }
        if f.u("timing_info_present_flag", 1) == 1 {
            f.u("num_units_in_tick", 32);
            f.u("time_scale", 32);
            f.u("fixed_frame_rate_flag", 1);
        }
        let nal_hrd = f.u("nal_hrd_parameters_present_flag", 1) == 1;
        if nal_hrd {
            f.hrd();
        }
        let vcl_hrd = f.u("vcl_hrd_parameters_present_flag", 1) == 1;
        if vcl_hrd {
            f.hrd();
        }
        if nal_hrd || vcl_hrd {
            f.u("hrd low_delay_hrd_flag", 1);
        }
        f.u("pic_struct_present_flag", 1);
        f.u("bitstream_restriction_flag", 1);
        f.u("motion_vectors_over_pic_boundaries_flag", 1);
        f.ue("max_bytes_per_pic_denom");
        f.ue("max_bits_per_mb_denom");
        f.ue("log2_max_mv_length_horizontal");
        f.ue("log2_max_mv_length_vertical");
        f.ue("max_num_reorder_frames");
        f.ue("max_dec_frame_buffering");
        assert!(f.r.flag().unwrap(), "rbsp_stop_one_bit");
        while !f.r.aligned() {
            assert!(!f.r.flag().unwrap(), "rbsp_alignment_zero_bit");
        }
        assert!(f.r.rest().is_empty(), "bytes after the RBSP trailing bits");
        f.out
    }

    #[test]
    fn an_sps_with_hrd_and_pic_struct_keeps_every_other_field() {
        let coded = fields(&NVENC_SPS);
        for flag in ["nal_hrd_parameters_present_flag", "pic_struct_present_flag"] {
            assert!(coded.contains(&(flag, 1)), "NVENC codes {flag} 1");
        }
        assert!(coded.contains(&("max_num_ref_frames", 2)));
        let expected: Vec<_> = (coded.into_iter())
            .filter(|(name, _)| !name.starts_with("hrd "))
            .map(|(name, v)| match name {
                "nal_hrd_parameters_present_flag"
                | "vcl_hrd_parameters_present_flag"
                | "pic_struct_present_flag" => (name, 0),
                _ => (name, v),
            })
            .collect();
        let (rewritten, sps) = two_refs(&NVENC_SPS).unwrap();
        assert_eq!(sps.log2_max_frame_num, 8);
        assert_eq!(fields(&rewritten), expected);
        assert_eq!(
            two_refs(&rewritten).unwrap().0,
            rewritten,
            "a rewritten SPS stays as it is"
        );
    }
}
