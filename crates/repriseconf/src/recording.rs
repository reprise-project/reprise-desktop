//! `[recording]`: reprise-record's settings, each key described in docs/configuration.md.
//! Each key that is missing or out of range keeps its default alone.

use crate::remote::EncoderChoice;
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

/// How the encoder spends bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RateMode {
    /// A constant quality, the bitrate following the picture.
    Quality,
    /// A bitrate on average.
    Average,
    /// A bitrate held through each second, frames repeated on a still screen.
    Constant,
}

impl RateMode {
    pub const ALL: [RateMode; 3] = [RateMode::Quality, RateMode::Average, RateMode::Constant];

    /// Its name in `config.toml`.
    pub fn name(self) -> &'static str {
        match self {
            RateMode::Quality => "quality",
            RateMode::Average => "average",
            RateMode::Constant => "constant",
        }
    }
}

/// The codec what plays is recorded in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioCodec {
    Aac,
    Opus,
    Flac,
}

impl AudioCodec {
    pub const ALL: [AudioCodec; 3] = [AudioCodec::Aac, AudioCodec::Opus, AudioCodec::Flac];

    /// Its name in `config.toml`.
    pub fn name(self) -> &'static str {
        match self {
            AudioCodec::Aac => "aac",
            AudioCodec::Opus => "opus",
            AudioCodec::Flac => "flac",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            AudioCodec::Aac => "AAC",
            AudioCodec::Opus => "Opus",
            AudioCodec::Flac => "FLAC",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Recording {
    pub codec: Codec,
    /// Never [`EncoderChoice::None`].
    pub encoder: EncoderChoice,
    pub rate: RateMode,
    /// 1..100, for [`RateMode::Quality`].
    pub quality: u8,
    /// kbit/s, for [`RateMode::Average`] and [`RateMode::Constant`].
    pub bitrate: u32,
    /// The most frames a second; 0 at the screen's refresh rate.
    pub frame_rate: u32,
    /// Seconds from one keyframe to the next.
    pub keyframe_interval: u32,
    pub audio_codec: AudioCodec,
    /// kbit/s, for AAC and Opus.
    pub audio_bitrate: u32,
    /// Opus at a variable bitrate.
    pub audio_vbr: bool,
    pub flac_compression: u8,
}

/// The quality that codes H.264 at x264's CRF 23 and the hardware encoders' QP 24.
pub const DEFAULT_QUALITY: u8 = 55;

impl Default for Recording {
    fn default() -> Self {
        Recording {
            codec: Codec::H264,
            encoder: EncoderChoice::Auto,
            rate: RateMode::Quality,
            quality: DEFAULT_QUALITY,
            bitrate: 8000,
            frame_rate: 0,
            keyframe_interval: 4,
            audio_codec: AudioCodec::Aac,
            audio_bitrate: 160,
            audio_vbr: true,
            flac_compression: 5,
        }
    }
}

/// `[recording]`, read now. A missing, unreadable or non-TOML file gives the defaults; the keys
/// that fell back are logged in one warning.
pub fn recording() -> Recording {
    read_file(&crate::config_path()).unwrap_or_else(|e| {
        log::warn!("{e}; recordings use their defaults");
        Recording::default()
    })
}

fn read_file(path: &std::path::Path) -> Result<Recording, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let table =
        toml::from_str::<toml::Table>(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    let (r, bad) = match table.get("recording") {
        None => (Recording::default(), Vec::new()),
        Some(toml::Value::Table(t)) => read(t),
        Some(_) => (Recording::default(), vec!["recording"]),
    };
    if !bad.is_empty() {
        log::warn!(
            "{}: [recording] {} invalid, using the default",
            path.display(),
            bad.join(", ")
        );
    }
    Ok(r)
}

/// The `[recording]` table's settings, each invalid key at its default.
pub fn from_table(t: &toml::Table) -> Recording {
    read(t).0
}

fn read(t: &toml::Table) -> (Recording, Vec<&'static str>) {
    let d = Recording::default();
    let mut bad = Vec::new();
    let int = |v: &toml::Value, range: std::ops::RangeInclusive<i64>| {
        v.as_integer().filter(|n| range.contains(n))
    };
    let r = Recording {
        codec: field(t, "codec", &mut bad, d.codec, |v| v.clone().try_into().ok()),
        encoder: field(t, "encoder", &mut bad, d.encoder, |v| {
            v.clone()
                .try_into()
                .ok()
                .filter(|&c| c != EncoderChoice::None)
        }),
        rate: field(t, "rate", &mut bad, d.rate, |v| v.clone().try_into().ok()),
        quality: field(t, "quality", &mut bad, d.quality, |v| {
            int(v, 1..=100).map(|n| n as u8)
        }),
        bitrate: field(t, "bitrate", &mut bad, d.bitrate, |v| {
            int(v, 500..=100_000).map(|n| n as u32)
        }),
        frame_rate: field(t, "frame_rate", &mut bad, d.frame_rate, |v| {
            int(v, 0..=240).map(|n| n as u32)
        }),
        keyframe_interval: field(t, "keyframe_interval", &mut bad, d.keyframe_interval, |v| {
            int(v, 1..=60).map(|n| n as u32)
        }),
        audio_codec: field(t, "audio_codec", &mut bad, d.audio_codec, |v| {
            v.clone().try_into().ok()
        }),
        audio_bitrate: field(t, "audio_bitrate", &mut bad, d.audio_bitrate, |v| {
            int(v, 32..=512).map(|n| n as u32)
        }),
        audio_vbr: field(t, "audio_vbr", &mut bad, d.audio_vbr, toml::Value::as_bool),
        flac_compression: field(t, "flac_compression", &mut bad, d.flac_compression, |v| {
            int(v, 0..=12).map(|n| n as u8)
        }),
    };
    (r, bad)
}

/// `t[name]` through `parse`: `default` when it is absent or `parse` rejects it, noting `name`
/// in `bad` then.
fn field<T>(
    t: &toml::Table,
    name: &'static str,
    bad: &mut Vec<&'static str>,
    default: T,
    parse: impl FnOnce(&toml::Value) -> Option<T>,
) -> T {
    let Some(v) = t.get(name) else {
        return default;
    };
    parse(v).unwrap_or_else(|| {
        bad.push(name);
        default
    })
}

/// What `reprise-record --probe` found: the encoders that code each codec and the audio codecs
/// the MP4 muxer takes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Probed {
    pub video: Vec<(Codec, EncoderChoice)>,
    pub audio: Vec<AudioCodec>,
}

impl Probed {
    /// One line a find: `video h264 vaapi`, `audio opus`.
    pub fn to_text(&self) -> String {
        let video = (self.video.iter()).map(|(c, e)| format!("video {} {}\n", c.name(), e.name()));
        let audio = (self.audio.iter()).map(|a| format!("audio {}\n", a.name()));
        video.chain(audio).collect()
    }

    /// What [`Probed::to_text`] wrote; a line it does not know is skipped.
    pub fn parse(text: &str) -> Probed {
        let mut probed = Probed::default();
        for line in text.lines() {
            match line.split_whitespace().collect::<Vec<_>>()[..] {
                ["video", codec, encoder] => {
                    let codec = Codec::ALL.into_iter().find(|c| c.name() == codec);
                    let encoder = (EncoderChoice::ALL.into_iter())
                        .find(|e| e.name() == encoder && *e != EncoderChoice::None);
                    if let (Some(c), Some(e)) = (codec, encoder) {
                        probed.video.push((c, e));
                    }
                }
                ["audio", codec] => {
                    if let Some(a) = AudioCodec::ALL.into_iter().find(|a| a.name() == codec) {
                        probed.audio.push(a);
                    }
                }
                _ => {}
            }
        }
        probed
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

    #[test]
    fn an_empty_section_gives_the_defaults() {
        let r = from_table(&toml::Table::new());
        assert_eq!(r, Recording::default());
        assert_eq!(
            (r.codec, r.encoder, r.rate, r.quality, r.bitrate),
            (
                Codec::H264,
                EncoderChoice::Auto,
                RateMode::Quality,
                55,
                8000
            )
        );
        assert_eq!((r.frame_rate, r.keyframe_interval), (0, 4));
        assert_eq!(
            (
                r.audio_codec,
                r.audio_bitrate,
                r.audio_vbr,
                r.flac_compression
            ),
            (AudioCodec::Aac, 160, true, 5)
        );
    }

    #[test]
    fn a_bad_key_falls_back_alone_and_the_compositor_still_parses_the_file() {
        let t: toml::Table = toml::from_str(
            "codec = \"vp9\"\nencoder = \"none\"\nframe_rate = 1000\nbitrate = 3000\n",
        )
        .unwrap();
        let (r, bad) = read(&t);
        assert_eq!(
            (r.codec, r.encoder, r.frame_rate, r.bitrate),
            (Codec::H264, EncoderChoice::Auto, 0, 3000)
        );
        assert_eq!(bad, ["codec", "encoder", "frame_rate"]);
        assert!(crate::config::Config::parse("[recording]\ncodec = \"av1\"\n").is_ok());
    }

    #[test]
    fn a_value_past_its_range_falls_back() {
        for (text, ok) in [
            ("quality = 0", false),
            ("quality = 101", false),
            ("quality = 100", true),
            ("keyframe_interval = 61", false),
            ("keyframe_interval = 60", true),
            ("flac_compression = 13", false),
            ("flac_compression = 12", true),
            ("audio_bitrate = 31", false),
            ("audio_bitrate = 32", true),
            ("bitrate = 499", false),
            ("frame_rate = 240", true),
        ] {
            let t: toml::Table = toml::from_str(text).unwrap();
            assert_eq!(read(&t).1.is_empty(), ok, "{text}");
        }
    }

    #[test]
    fn every_name_reads_back() {
        for c in Codec::ALL {
            let t: toml::Table = toml::from_str(&format!("codec = \"{}\"", c.name())).unwrap();
            assert_eq!(from_table(&t).codec, c);
        }
        for m in RateMode::ALL {
            let t: toml::Table = toml::from_str(&format!("rate = \"{}\"", m.name())).unwrap();
            assert_eq!(from_table(&t).rate, m);
        }
        for a in AudioCodec::ALL {
            let t: toml::Table =
                toml::from_str(&format!("audio_codec = \"{}\"", a.name())).unwrap();
            assert_eq!(from_table(&t).audio_codec, a);
        }
    }

    #[test]
    fn the_probe_text_reads_back_what_it_wrote() {
        let p = Probed {
            video: vec![
                (Codec::H264, EncoderChoice::Vaapi),
                (Codec::Av1, EncoderChoice::Software),
            ],
            audio: vec![AudioCodec::Aac, AudioCodec::Flac],
        };
        assert_eq!(
            p.to_text(),
            "video h264 vaapi\nvideo av1 software\naudio aac\naudio flac\n"
        );
        assert_eq!(Probed::parse(&p.to_text()), p);
        let noisy = Probed::parse("video vp9 vaapi\nvideo h264 none\nnoise\naudio opus\n");
        assert_eq!(
            noisy,
            Probed {
                video: vec![],
                audio: vec![AudioCodec::Opus]
            }
        );
    }
}
