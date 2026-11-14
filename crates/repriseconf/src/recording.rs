//! `[recording]`: reprise-record's settings.

use serde::Deserialize;

/// The video codec a recording is coded in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Codec {
    H264,
    H265,
    Av1,
}

impl Codec {
    pub const ALL: [Codec; 3] = [Codec::H264, Codec::H265, Codec::Av1];

    /// Its name in `config.toml`.
    pub fn name(self) -> &'static str {
        match self {
            Codec::H264 => "h264",
            Codec::H265 => "h265",
            Codec::Av1 => "av1",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Codec::H264 => "H.264",
            Codec::H265 => "H.265",
            Codec::Av1 => "AV1",
        }
    }

    /// The CPU encoder's short name.
    pub fn software_name(self) -> &'static str {
        match self {
            Codec::H264 => "x264",
            Codec::H265 => "x265",
            Codec::Av1 => "SVT-AV1",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_software_encoder_reads_x264_x265_svt_av1_by_codec() {
        assert_eq!(
            Codec::ALL.map(Codec::software_name),
            ["x264", "x265", "SVT-AV1"]
        );
        assert_eq!(Codec::ALL.map(Codec::name), ["h264", "h265", "av1"]);
    }
}
