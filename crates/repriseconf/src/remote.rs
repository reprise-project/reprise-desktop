//! `[remote_desktop]`: reprise-rdp's port, picture quality, frame rate, video encoder, sound and
//! how many viewers it takes.
//! Each key that is missing or out of range keeps its default alone.

use serde::Deserialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Quality {
    Speed,
    Balanced,
    Sharp,
}

impl Quality {
    /// Its name in `config.toml`.
    pub fn name(self) -> &'static str {
        match self {
            Quality::Speed => "speed",
            Quality::Balanced => "balanced",
            Quality::Sharp => "sharp",
        }
    }

    /// The H.264 constant quantizer the encoder runs at.
    pub fn qp(self) -> u8 {
        match self {
            Quality::Speed => 30,
            Quality::Balanced => 25,
            Quality::Sharp => 21,
        }
    }
}

/// The video encoder `[remote_desktop] encoder` asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EncoderChoice {
    /// The first H.264 encoder that works: VA-API, NVENC, Vulkan, then x264.
    Auto,
    Vaapi,
    Nvenc,
    Vulkan,
    /// x264 on the CPU.
    Software,
    /// RemoteFX bitmaps, no H.264.
    None,
}

impl EncoderChoice {
    pub const ALL: [EncoderChoice; 6] = [
        EncoderChoice::Auto,
        EncoderChoice::Vaapi,
        EncoderChoice::Nvenc,
        EncoderChoice::Vulkan,
        EncoderChoice::Software,
        EncoderChoice::None,
    ];

    /// Its name in `config.toml`.
    pub fn name(self) -> &'static str {
        match self {
            EncoderChoice::Auto => "auto",
            EncoderChoice::Vaapi => "vaapi",
            EncoderChoice::Nvenc => "nvenc",
            EncoderChoice::Vulkan => "vulkan",
            EncoderChoice::Software => "software",
            EncoderChoice::None => "none",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RemoteDesktop {
    pub port: u16,
    pub quality: Quality,
    /// 30 or 60.
    pub frame_rate: u32,
    pub encoder: EncoderChoice,
    pub sound: bool,
    /// 1 to 16 connected at once.
    pub max_viewers: usize,
}

impl Default for RemoteDesktop {
    fn default() -> Self {
        RemoteDesktop {
            port: 3389,
            quality: Quality::Balanced,
            frame_rate: 60,
            encoder: EncoderChoice::Auto,
            sound: true,
            max_viewers: 4,
        }
    }
}

/// `[remote_desktop]`, read now. An unreadable file or section gives the defaults; the keys that
/// fell back are logged in one warning.
pub fn remote_desktop() -> RemoteDesktop {
    let path = crate::config_path();
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let table = match toml::from_str::<toml::Table>(&text) {
        Ok(t) => t,
        Err(e) => {
            log::warn!("{}: {e}; remote desktop uses its defaults", path.display());
            return RemoteDesktop::default();
        }
    };
    let (rd, bad) = match table.get("remote_desktop") {
        None => (RemoteDesktop::default(), Vec::new()),
        Some(toml::Value::Table(t)) => read(t),
        Some(_) => (RemoteDesktop::default(), vec!["remote_desktop"]),
    };
    if !bad.is_empty() {
        log::warn!(
            "{}: [remote_desktop] {} invalid, using the default",
            path.display(),
            bad.join(", ")
        );
    }
    rd
}

/// The `[remote_desktop]` table's settings, each invalid key at its default.
pub fn from_table(t: &toml::Table) -> RemoteDesktop {
    read(t).0
}

fn read(t: &toml::Table) -> (RemoteDesktop, Vec<&'static str>) {
    let d = RemoteDesktop::default();
    let mut bad = Vec::new();
    let rd = RemoteDesktop {
        port: key(t, "port", &mut bad, d.port, |v| {
            v.as_integer()
                .and_then(|p| u16::try_from(p).ok())
                .filter(|&p| p != 0)
        }),
        quality: key(t, "quality", &mut bad, d.quality, |v| {
            v.clone().try_into().ok()
        }),
        frame_rate: key(t, "frame_rate", &mut bad, d.frame_rate, |v| {
            v.as_integer()
                .filter(|r| matches!(r, 30 | 60))
                .map(|r| r as u32)
        }),
        encoder: key(t, "encoder", &mut bad, d.encoder, |v| {
            v.clone().try_into().ok()
        }),
        sound: key(t, "sound", &mut bad, d.sound, toml::Value::as_bool),
        max_viewers: key(t, "max_viewers", &mut bad, d.max_viewers, |v| {
            v.as_integer()
                .filter(|n| (1..=16).contains(n))
                .map(|n| n as usize)
        }),
    };
    (rd, bad)
}

/// `t[name]` through `parse`; `default` when it is missing, and also when `parse` rejects it,
/// noting `name` in `bad`.
fn key<T>(
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_section_gives_the_defaults() {
        let d = from_table(&toml::Table::new());
        assert_eq!(
            (
                d.port,
                d.quality,
                d.frame_rate,
                d.encoder,
                d.sound,
                d.max_viewers
            ),
            (3389, Quality::Balanced, 60, EncoderChoice::Auto, true, 4)
        );
    }

    #[test]
    fn a_bad_key_falls_back_alone() {
        let t: toml::Table = toml::from_str(
            "port = 70000
frame_rate = 30
quality = \"sharp\"",
        )
        .unwrap();
        let d = from_table(&t);
        assert_eq!(
            (d.port, d.frame_rate, d.quality),
            (3389, 30, Quality::Sharp)
        );
    }

    #[test]
    fn port_0_and_a_sound_that_is_not_true_or_false_are_invalid() {
        let t: toml::Table =
            toml::from_str("port = 0\nsound = \"off\"\nquality = \"ultra\"").unwrap();
        let (rd, bad) = read(&t);
        assert_eq!(
            (rd.port, rd.sound, rd.quality),
            (3389, true, Quality::Balanced)
        );
        assert_eq!(bad, ["port", "quality", "sound"]);
    }

    #[test]
    fn frame_rate_is_30_or_60() {
        let t: toml::Table = toml::from_str("frame_rate = 45").unwrap();
        assert_eq!(from_table(&t).frame_rate, 60);
    }

    #[test]
    fn max_viewers_is_1_to_16() {
        for (text, want) in [
            ("1", 1),
            ("16", 16),
            ("0", 4),
            ("17", 4),
            ("-1", 4),
            ("\"2\"", 4),
        ] {
            let t: toml::Table = toml::from_str(&format!("max_viewers = {text}")).unwrap();
            let (rd, bad) = read(&t);
            assert_eq!(rd.max_viewers, want, "max_viewers = {text}");
            let invalid: &[&str] = if want == 4 { &["max_viewers"] } else { &[] };
            assert_eq!(bad, invalid, "max_viewers = {text}");
        }
    }

    #[test]
    fn each_quality_reads_back_by_its_name() {
        for q in [Quality::Speed, Quality::Balanced, Quality::Sharp] {
            let t: toml::Table = toml::from_str(&format!("quality = \"{}\"", q.name())).unwrap();
            assert_eq!(from_table(&t).quality, q);
        }
    }

    #[test]
    fn an_unknown_encoder_falls_back_to_auto() {
        let t: toml::Table = toml::from_str("encoder = \"quicksync\"").unwrap();
        let (rd, bad) = read(&t);
        assert_eq!(rd.encoder, EncoderChoice::Auto);
        assert_eq!(bad, ["encoder"], "named in the one warning");
        for e in EncoderChoice::ALL {
            let t: toml::Table = toml::from_str(&format!("encoder = \"{}\"", e.name())).unwrap();
            assert_eq!(from_table(&t).encoder, e);
        }
    }

    #[test]
    fn quality_sets_the_quantizer() {
        assert_eq!(
            [
                Quality::Speed.qp(),
                Quality::Balanced.qp(),
                Quality::Sharp.qp()
            ],
            [30, 25, 21]
        );
    }
}
