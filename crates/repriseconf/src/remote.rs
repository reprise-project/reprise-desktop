//! `[remote_desktop]`: reprise-rdp's settings, each key described in config.example.toml.
//! Each key that is missing or out of range keeps its default alone.

use crate::recording::Codec;
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

    /// The choice [`EncoderChoice::name`] gives `name`.
    pub fn from_name(name: &str) -> Option<EncoderChoice> {
        EncoderChoice::ALL.into_iter().find(|e| e.name() == name)
    }

    /// The encoder coding `codec`, as a status names it; None for `auto` and `none`.
    pub fn label(self, codec: Codec) -> Option<&'static str> {
        use Codec::*;
        Some(match (self, codec) {
            (EncoderChoice::Vaapi, H264) => "VA-API H.264",
            (EncoderChoice::Vaapi, H265) => "VA-API H.265",
            (EncoderChoice::Vaapi, Av1) => "VA-API AV1",
            (EncoderChoice::Nvenc, H264) => "NVENC H.264",
            (EncoderChoice::Nvenc, H265) => "NVENC H.265",
            (EncoderChoice::Nvenc, Av1) => "NVENC AV1",
            (EncoderChoice::Vulkan, H264) => "Vulkan H.264",
            (EncoderChoice::Vulkan, H265) => "Vulkan H.265",
            (EncoderChoice::Vulkan, Av1) => "Vulkan AV1",
            (EncoderChoice::Software, H264) => "Software H.264",
            (EncoderChoice::Software, H265) => "Software H.265",
            (EncoderChoice::Software, Av1) => "Software AV1",
            (EncoderChoice::Auto | EncoderChoice::None, _) => return None,
        })
    }

    /// The status while `using` codes `codec` for this choice, naming the chosen encoder it
    /// stands in for: "VA-API AV1 (NVENC unavailable)".
    pub fn status(self, using: EncoderChoice, codec: Codec) -> String {
        let label = using.label(codec).unwrap_or_default();
        let chosen = match self {
            EncoderChoice::Vaapi => "VA-API",
            EncoderChoice::Nvenc => "NVENC",
            EncoderChoice::Vulkan => "Vulkan",
            EncoderChoice::Software => codec.software_name(),
            EncoderChoice::Auto | EncoderChoice::None => return label.into(),
        };
        if self == using {
            label.into()
        } else {
            format!("{label} ({chosen} unavailable)")
        }
    }
}

/// What this computer's own screen does while a computer is connected.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LaptopScreen {
    /// It goes on showing the session.
    Show,
    /// It shows the lock, and the viewer works on screens sized to its window.
    #[default]
    Lock,
}

impl LaptopScreen {
    /// Its name in `config.toml`.
    pub fn name(self) -> &'static str {
        match self {
            LaptopScreen::Show => "show",
            LaptopScreen::Lock => "lock",
        }
    }
}

/// What a computer logging in does to those already connected.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SecondClient {
    /// It joins them.
    #[default]
    Join,
    /// It ends them.
    Replace,
}

impl SecondClient {
    /// Its name in `config.toml`.
    pub fn name(self) -> &'static str {
        match self {
            SecondClient::Join => "join",
            SecondClient::Replace => "replace",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RemoteDesktop {
    pub port: u16,
    pub quality: Quality,
    /// 30, 60, 120 or 144.
    pub frame_rate: u32,
    pub encoder: EncoderChoice,
    pub sound: bool,
    /// 1 to 16 connected at once.
    pub max_viewers: usize,
    pub lock_on_leave: bool,
    /// Seconds, 0 to 60, the lock waits when the last viewer left because the screen it showed
    /// went; 0 locks at once.
    pub lock_grace_s: u32,
    /// Whether the compositor borders what the server captures; `[privacy] capture_border` off
    /// turns it off as well.
    pub capture_border: bool,
    pub laptop_screen: LaptopScreen,
    pub second_client: SecondClient,
    /// AVC444 codes each view on its own encoder and merges the two streams; off codes both on
    /// one encoder.
    pub avc444_split_encoders: bool,
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
            lock_on_leave: true,
            lock_grace_s: 5,
            capture_border: false,
            laptop_screen: LaptopScreen::Lock,
            second_client: SecondClient::Join,
            avc444_split_encoders: true,
        }
    }
}

/// `[remote_desktop]` as `Config::from_file` reads it: an error naming the keys it can't read.
/// Programs read it through [`crate::current`].
pub(crate) fn checked(v: &toml::Value) -> Result<RemoteDesktop, String> {
    let t = v.as_table().ok_or("[remote_desktop] is not a table")?;
    match read(t, RemoteDesktop::default()) {
        (rd, bad) if bad.is_empty() => Ok(rd),
        (_, bad) => Err(format!("[remote_desktop] {} is not valid", bad.join(", "))),
    }
}

/// The `[remote_desktop]` table's settings, each invalid key at its default.
pub fn from_table(t: &toml::Table) -> RemoteDesktop {
    read(t, RemoteDesktop::default()).0
}

/// `t`'s settings: a missing key at its default, an invalid one at its value in `k`.
fn read(t: &toml::Table, k: RemoteDesktop) -> (RemoteDesktop, Vec<&'static str>) {
    let d = RemoteDesktop::default();
    let mut bad = Vec::new();
    let rd = RemoteDesktop {
        port: key(t, "port", &mut bad, d.port, k.port, |v| {
            v.as_integer()
                .and_then(|p| u16::try_from(p).ok())
                .filter(|&p| p != 0)
        }),
        quality: key(t, "quality", &mut bad, d.quality, k.quality, |v| {
            v.clone().try_into().ok()
        }),
        frame_rate: key(t, "frame_rate", &mut bad, d.frame_rate, k.frame_rate, |v| {
            v.as_integer()
                .filter(|r| matches!(r, 30 | 60 | 120 | 144))
                .map(|r| r as u32)
        }),
        encoder: key(t, "encoder", &mut bad, d.encoder, k.encoder, |v| {
            v.clone().try_into().ok()
        }),
        sound: key(t, "sound", &mut bad, d.sound, k.sound, toml::Value::as_bool),
        max_viewers: key(
            t,
            "max_viewers",
            &mut bad,
            d.max_viewers,
            k.max_viewers,
            |v| {
                v.as_integer()
                    .filter(|n| (1..=16).contains(n))
                    .map(|n| n as usize)
            },
        ),
        lock_on_leave: key(
            t,
            "lock_on_leave",
            &mut bad,
            d.lock_on_leave,
            k.lock_on_leave,
            toml::Value::as_bool,
        ),
        lock_grace_s: key(
            t,
            "lock_grace_s",
            &mut bad,
            d.lock_grace_s,
            k.lock_grace_s,
            |v| {
                v.as_integer()
                    .filter(|s| (0..=60).contains(s))
                    .map(|s| s as u32)
            },
        ),
        capture_border: key(
            t,
            "capture_border",
            &mut bad,
            d.capture_border,
            k.capture_border,
            toml::Value::as_bool,
        ),
        laptop_screen: key(
            t,
            "laptop_screen",
            &mut bad,
            d.laptop_screen,
            k.laptop_screen,
            |v| v.clone().try_into().ok(),
        ),
        second_client: key(
            t,
            "second_client",
            &mut bad,
            d.second_client,
            k.second_client,
            |v| v.clone().try_into().ok(),
        ),
        avc444_split_encoders: key(
            t,
            "avc444_split_encoders",
            &mut bad,
            d.avc444_split_encoders,
            k.avc444_split_encoders,
            toml::Value::as_bool,
        ),
    };
    (rd, bad)
}

/// `t[name]` through `parse`; `missing` when it is absent, `invalid` when `parse` rejects it,
/// noting `name` in `bad`.
fn key<T>(
    t: &toml::Table,
    name: &'static str,
    bad: &mut Vec<&'static str>,
    missing: T,
    invalid: T,
    parse: impl FnOnce(&toml::Value) -> Option<T>,
) -> T {
    let Some(v) = t.get(name) else {
        return missing;
    };
    parse(v).unwrap_or_else(|| {
        bad.push(name);
        invalid
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stand_in_encoder_names_the_one_chosen_for_the_codec() {
        use EncoderChoice::*;
        assert_eq!(
            Nvenc.status(Vaapi, Codec::Av1),
            "VA-API AV1 (NVENC unavailable)"
        );
        assert_eq!(
            Software.status(Vaapi, Codec::Av1),
            "VA-API AV1 (SVT-AV1 unavailable)"
        );
        assert_eq!(Auto.status(Software, Codec::H265), "Software H.265");
        assert_eq!(Vulkan.status(Vulkan, Codec::H264), "Vulkan H.264");
        assert!(Auto.label(Codec::Av1).is_none());
        for e in EncoderChoice::ALL {
            assert_eq!(EncoderChoice::from_name(e.name()), Some(e));
        }
    }

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
                d.max_viewers,
                d.lock_on_leave,
                d.lock_grace_s,
                d.capture_border,
                d.laptop_screen
            ),
            (
                3389,
                Quality::Balanced,
                60,
                EncoderChoice::Auto,
                true,
                4,
                true,
                5,
                false,
                LaptopScreen::Lock
            )
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
        let (rd, bad) = read(&t, RemoteDesktop::default());
        assert_eq!(
            (rd.port, rd.sound, rd.quality),
            (3389, true, Quality::Balanced)
        );
        assert_eq!(bad, ["port", "quality", "sound"]);
    }

    #[test]
    fn lock_on_leave_reads_true_or_false() {
        for (text, want, invalid) in [
            ("false", false, false),
            ("true", true, false),
            ("0", true, true),
        ] {
            let t: toml::Table = toml::from_str(&format!("lock_on_leave = {text}")).unwrap();
            let (rd, bad) = read(&t, RemoteDesktop::default());
            assert_eq!(rd.lock_on_leave, want, "lock_on_leave = {text}");
            assert_eq!(
                bad.contains(&"lock_on_leave"),
                invalid,
                "lock_on_leave = {text}"
            );
        }
    }

    #[test]
    fn lock_grace_s_is_0_to_60() {
        for (text, want, invalid) in [
            ("0", 0, false),
            ("60", 60, false),
            ("61", 5, true),
            ("-1", 5, true),
            ("\"10\"", 5, true),
        ] {
            let t: toml::Table = toml::from_str(&format!("lock_grace_s = {text}")).unwrap();
            let (rd, bad) = read(&t, RemoteDesktop::default());
            assert_eq!(rd.lock_grace_s, want, "lock_grace_s = {text}");
            assert_eq!(
                bad.contains(&"lock_grace_s"),
                invalid,
                "lock_grace_s = {text}"
            );
        }
    }

    #[test]
    fn capture_border_reads_true_or_false() {
        for (text, want, invalid) in [
            ("true", true, false),
            ("false", false, false),
            ("\"on\"", false, true),
        ] {
            let t: toml::Table = toml::from_str(&format!("capture_border = {text}")).unwrap();
            let (rd, bad) = read(&t, RemoteDesktop::default());
            assert_eq!(rd.capture_border, want, "capture_border = {text}");
            assert_eq!(
                bad.contains(&"capture_border"),
                invalid,
                "capture_border = {text}"
            );
        }
    }

    #[test]
    fn avc444_split_encoders_is_on_unless_false() {
        for (text, want, invalid) in [
            ("false", false, false),
            ("true", true, false),
            ("\"off\"", true, true),
        ] {
            let t: toml::Table =
                toml::from_str(&format!("avc444_split_encoders = {text}")).unwrap();
            let (rd, bad) = read(&t, RemoteDesktop::default());
            assert_eq!(
                rd.avc444_split_encoders, want,
                "avc444_split_encoders = {text}"
            );
            assert_eq!(
                bad.contains(&"avc444_split_encoders"),
                invalid,
                "avc444_split_encoders = {text}"
            );
        }
        assert!(
            from_table(&toml::Table::new()).avc444_split_encoders,
            "a missing key"
        );
    }

    #[test]
    fn laptop_screen_reads_show_or_lock() {
        for (text, want, invalid) in [
            ("\"show\"", LaptopScreen::Show, false),
            ("\"lock\"", LaptopScreen::Lock, false),
            ("\"x\"", LaptopScreen::Lock, true),
        ] {
            let t: toml::Table = toml::from_str(&format!("laptop_screen = {text}")).unwrap();
            let (rd, bad) = read(&t, RemoteDesktop::default());
            assert_eq!(rd.laptop_screen, want, "laptop_screen = {text}");
            assert_eq!(
                bad.contains(&"laptop_screen"),
                invalid,
                "laptop_screen = {text}"
            );
        }
        for s in [LaptopScreen::Show, LaptopScreen::Lock] {
            let t: toml::Table =
                toml::from_str(&format!("laptop_screen = \"{}\"", s.name())).unwrap();
            assert_eq!(
                from_table(&t).laptop_screen,
                s,
                "{s:?} reads back by its name"
            );
        }
    }

    #[test]
    fn second_client_reads_join_and_replace_and_defaults_to_join() {
        let t: toml::Table = toml::from_str("second_client = \"replace\"").unwrap();
        assert_eq!(from_table(&t).second_client, SecondClient::Replace);
        assert_eq!(
            from_table(&toml::Table::new()).second_client,
            SecondClient::Join,
            "a missing key"
        );
        let t: toml::Table = toml::from_str("second_client = \"swap\"").unwrap();
        let (rd, bad) = read(&t, RemoteDesktop::default());
        assert_eq!(rd.second_client, SecondClient::Join);
        assert_eq!(bad, ["second_client"]);
        for c in [SecondClient::Join, SecondClient::Replace] {
            let t: toml::Table =
                toml::from_str(&format!("second_client = \"{}\"", c.name())).unwrap();
            assert_eq!(
                from_table(&t).second_client,
                c,
                "{c:?} reads back by its name"
            );
        }
    }

    #[test]
    fn frame_rate_is_30_60_120_or_144() {
        for (text, want) in [
            ("30", 30),
            ("120", 120),
            ("144", 144),
            ("45", 60),
            ("240", 60),
        ] {
            let t: toml::Table = toml::from_str(&format!("frame_rate = {text}")).unwrap();
            assert_eq!(from_table(&t).frame_rate, want, "frame_rate = {text}");
        }
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
            let (rd, bad) = read(&t, RemoteDesktop::default());
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
        let (rd, bad) = read(&t, RemoteDesktop::default());
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
