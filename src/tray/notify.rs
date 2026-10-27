//! What a `Notify` call carries beyond its summary: action pairs, honoured hints, body markup
//! reduced to plain text and the `image-data` icon. Pure, so the D-Bus handler only glues these
//! together.

use crate::raster::Canvas;
use repriseaudio::sounds::Sound;
use reprisekit::icon::IconSource;
use serde::de::{self, Deserialize, Deserializer, IgnoredAny, SeqAccess, Visitor};
use std::collections::HashMap;
use std::path::Path;
use zbus::zvariant::{Signature, Type};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Urgency {
    Low,
    #[default]
    Normal,
    Critical,
}

/// Action pairs kept per notification and the label length shown; keys are sent back as given, so a
/// longer one is dropped rather than cut.
pub const MAX_ACTIONS: usize = 16;
pub const ACTION_LABEL_CHARS: usize = 63;
const ACTION_KEY_BYTES: usize = 255;

pub const DEFAULT_ACTION: &str = "default";

pub const INLINE_REPLY: &str = "inline-reply";

/// What a balloon plays when its sender names no sound.
pub const DEFAULT_SOUND: &str = "message-new-instant";

pub fn actions(raw: &[String]) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for [key, label] in raw.as_chunks::<2>().0 {
        if key.is_empty() || key.len() > ACTION_KEY_BYTES || out.iter().any(|(k, _)| k == key) {
            continue;
        }
        out.push((key.clone(), super::clip(label.clone(), ACTION_LABEL_CHARS)));
        if out.len() == MAX_ACTIONS {
            break;
        }
    }
    out
}

#[derive(Debug, PartialEq)]
pub enum Hint<'a> {
    Str(String),
    Bool(bool),
    Int(i64),
    Image(ImageData<'a>),
    Other,
}

#[derive(Debug, PartialEq)]
pub struct ImageData<'a> {
    pub width: i32,
    pub height: i32,
    pub stride: i32,
    pub alpha: bool,
    pub bits: i32,
    pub channels: i32,
    pub data: &'a [u8],
}

impl Type for Hint<'_> {
    const SIGNATURE: &'static Signature = &Signature::Variant;
}

impl<'de> Deserialize<'de> for Hint<'de> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_any(HintVisitor)
    }
}

struct HintVisitor;

impl<'de> Visitor<'de> for HintVisitor {
    type Value = Hint<'de>;

    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("a notification hint variant")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Hint<'de>, A::Error> {
        let sig: Signature = seq
            .next_element()?
            .ok_or_else(|| de::Error::invalid_length(0, &self))?;
        let missing = || de::Error::invalid_length(1, &HintVisitor);
        Ok(match sig {
            Signature::Str => Hint::Str(seq.next_element()?.ok_or_else(missing)?),
            Signature::Bool => Hint::Bool(seq.next_element()?.ok_or_else(missing)?),
            Signature::U8 => Hint::Int(seq.next_element::<u8>()?.ok_or_else(missing)?.into()),
            Signature::I32 => Hint::Int(seq.next_element::<i32>()?.ok_or_else(missing)?.into()),
            Signature::U32 => Hint::Int(seq.next_element::<u32>()?.ok_or_else(missing)?.into()),
            s if s == *<(i32, i32, i32, bool, i32, i32, &[u8])>::SIGNATURE => {
                let (width, height, stride, alpha, bits, channels, data): (
                    i32,
                    i32,
                    i32,
                    bool,
                    i32,
                    i32,
                    &'de [u8],
                ) = seq.next_element()?.ok_or_else(missing)?;
                Hint::Image(ImageData {
                    width,
                    height,
                    stride,
                    alpha,
                    bits,
                    channels,
                    data,
                })
            }
            _ => {
                seq.next_element::<IgnoredAny>()?;
                Hint::Other
            }
        })
    }
}

#[derive(Debug, Default, PartialEq)]
pub struct Hints<'a> {
    pub urgency: Urgency,
    pub transient: bool,
    pub resident: bool,
    pub image: Option<ImageData<'a>>,
    pub image_path: Option<String>,
    pub desktop_entry: Option<String>,
    pub reply_placeholder: Option<String>,
    pub reply_send: Option<String>,
    /// `sound-file` (an absolute path), else `sound-name`.
    pub sound: Option<Sound>,
    pub suppress_sound: bool,
}

impl Hints<'_> {
    /// What the balloon plays: the hinted sound, else [`DEFAULT_SOUND`]; None under `suppress-sound`.
    pub fn balloon_sound(&self) -> Option<Sound> {
        (!self.suppress_sound)
            .then(|| (self.sound.clone()).unwrap_or_else(|| Sound::Name(DEFAULT_SOUND.into())))
    }
}

pub fn hints(mut raw: HashMap<String, Hint<'_>>) -> Hints<'_> {
    let mut take = |names: &[&str]| names.iter().find_map(|n| raw.remove(*n));
    let urgency = match take(&["urgency"]) {
        Some(Hint::Int(0)) => Urgency::Low,
        Some(Hint::Int(2)) => Urgency::Critical,
        _ => Urgency::Normal,
    };
    let flag = |h: Option<Hint>| matches!(h, Some(Hint::Bool(true)) | Some(Hint::Int(1)));
    let text = |h: Option<Hint>| match h {
        Some(Hint::Str(s)) if !s.is_empty() => Some(s),
        _ => None,
    };
    let transient = flag(take(&["transient"]));
    let resident = flag(take(&["resident"]));
    let image = match take(&["image-data", "image_data", "icon_data"]) {
        Some(Hint::Image(i)) => Some(i),
        _ => None,
    };
    let image_path = text(take(&["image-path", "image_path"]));
    let desktop_entry = text(take(&["desktop-entry"]));
    let reply_placeholder = text(take(&["x-kde-reply-placeholder-text"]));
    let reply_send = text(take(&["x-kde-reply-submit-button-text"]));
    let file = text(take(&["sound-file"]))
        .filter(|p| p.starts_with('/'))
        .map(|p| Sound::File(p.into()));
    let name = text(take(&["sound-name"])).map(Sound::Name);
    let suppress_sound = flag(take(&["suppress-sound"]));
    Hints {
        urgency,
        transient,
        resident,
        image,
        image_path,
        desktop_entry,
        reply_placeholder,
        reply_send,
        sound: file.or(name),
        suppress_sound,
    }
}

/// Notification pictures show at most 96 px a side and are scaled down to it. `image-data`'s
/// bytes are already in memory, so only its arithmetic needs bounding.
const MAX_IMAGE_SIDE: i32 = 8192;
/// A picture file's side; its decoder's memory is bounded by [`MAX_PICTURE_ALLOC`].
const MAX_PICTURE_SIDE: u32 = 4096;
/// Bytes a picture file's decoder may allocate: 8-bit RGBA at [`MAX_PICTURE_SIDE`] square.
const MAX_PICTURE_ALLOC: u64 = MAX_PICTURE_SIDE as u64 * MAX_PICTURE_SIDE as u64 * 4;

/// `image-data` decoded at each of [`super::NOTIFY_SIZES`], fitted inside
/// a square canvas with its aspect kept and the rest clear.
pub fn image_sources(img: &ImageData) -> Vec<IconSource> {
    let ok = (1..=MAX_IMAGE_SIDE).contains(&img.width)
        && (1..=MAX_IMAGE_SIDE).contains(&img.height)
        && img.bits == 8
        && (img.channels == 3 || img.channels == 4)
        && (img.channels == 4) == img.alpha;
    if !ok {
        return Vec::new();
    }
    let (w, h, ch) = (
        img.width as usize,
        img.height as usize,
        img.channels as usize,
    );
    let stride = img.stride.max(0) as usize;
    if stride < w * ch || img.data.len() < stride * (h - 1) + w * ch {
        return Vec::new();
    }
    let pixel = |x: u32, y: u32| {
        let o = y as usize * stride + x as usize * ch;
        let p = &img.data[o..o + ch];
        let a = if ch == 4 { p[3] } else { 0xFF };
        reprisekit::icon::premultiply(u32::from_be_bytes([a, p[0], p[1], p[2]]))
    };
    super::NOTIFY_SIZES
        .iter()
        .map(|&px| {
            let side = px as u32;
            let fit = |n: usize| ((n as u64 * side as u64 / w.max(h) as u64) as u32).max(1);
            let (fw, fh) = (fit(w), fit(h));
            let img: Canvas = Canvas::resample_box_from(w as u32, h as u32, fw, fh, pixel);
            let mut out = Canvas::new(side, side);
            let (ox, oy) = ((side - fw) / 2, (side - fh) / 2);
            for y in 0..fh {
                let to = ((oy + y) * side + ox) as usize;
                let from = (y * fw) as usize;
                out.pixels[to..to + fw as usize]
                    .copy_from_slice(&img.pixels[from..from + fw as usize]);
            }
            IconSource::from(out)
        })
        .collect()
}

/// A picture file the icon loader leaves alone, a JPEG cover say, as [`image_sources`] gives.
pub fn picture_file_sources(path: &Path) -> Vec<IconSource> {
    let Ok(img) = decode_picture(path) else {
        return Vec::new();
    };
    let rgba = img.into_rgba8();
    let (w, h) = (rgba.width() as i32, rgba.height() as i32);
    image_sources(&ImageData {
        width: w,
        height: h,
        stride: w * 4,
        alpha: true,
        bits: 8,
        channels: 4,
        data: rgba.as_raw(),
    })
}

fn decode_picture(path: &Path) -> image::ImageResult<image::DynamicImage> {
    crate::wallpaper::decode_with(path, MAX_PICTURE_SIDE, Some(MAX_PICTURE_ALLOC))
}

pub fn desktop_id(hint: &str) -> Option<String> {
    let ok = !hint.is_empty() && hint.len() <= 255 && !hint.contains('/') && !hint.starts_with('.');
    ok.then(|| format!("{}.desktop", hint.trim_end_matches(".desktop")))
}

pub fn plain_text(markup: &str) -> String {
    let mut out = String::with_capacity(markup.len());
    let mut rest = markup;
    while let Some(i) = rest.find(['<', '&']) {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        if rest.starts_with('<') {
            let Some(end) = rest.find('>') else {
                out.push_str(rest);
                return out;
            };
            let tag = rest[1..end].trim();
            let name = tag
                .trim_start_matches('/')
                .split(|c: char| c.is_whitespace() || c == '/')
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();
            match name.as_str() {
                "br" => out.push('\n'),
                "img" => out.push_str(&plain_text(&attribute(tag, "alt").unwrap_or_default())),
                _ => {}
            }
            rest = &rest[end + 1..];
        } else {
            let (ch, used) = entity(rest).unwrap_or(('&', 1));
            out.push(ch);
            rest = &rest[used..];
        }
    }
    out.push_str(rest);
    out
}

fn entity(s: &str) -> Option<(char, usize)> {
    // A byte search: the first 12 bytes may end inside a character.
    let end = s.bytes().take(12).position(|b| b == b';')?;
    let name = &s[1..end];
    let ch = match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        _ => {
            let num = name.strip_prefix('#')?;
            let code = match num.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => num.parse().ok()?,
            };
            char::from_u32(code)?
        }
    };
    Some((ch, end + 1))
}

fn attribute(tag: &str, name: &str) -> Option<String> {
    let at = tag.find(&format!("{name}="))? + name.len() + 1;
    let rest = &tag[at..];
    let quote = rest.chars().next().filter(|c| *c == '"' || *c == '\'')?;
    let body = &rest[1..];
    Some(body[..body.find(quote)?].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::zvariant::{LE, Value, serialized::Context, to_bytes};

    /// A PNG of `w` x `h` with no image data: a decoder that reads past the header fails on it.
    fn png_header(w: u32, h: u32) -> Vec<u8> {
        fn crc(bytes: &[u8]) -> u32 {
            let mut c = !0u32;
            for &b in bytes {
                c ^= u32::from(b);
                for _ in 0..8 {
                    c = if c & 1 != 0 {
                        0xEDB8_8320 ^ (c >> 1)
                    } else {
                        c >> 1
                    };
                }
            }
            !c
        }
        let mut chunk = b"IHDR".to_vec();
        chunk.extend(w.to_be_bytes());
        chunk.extend(h.to_be_bytes());
        chunk.extend([8, 6, 0, 0, 0]);
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend(13u32.to_be_bytes());
        png.extend(&chunk);
        png.extend(crc(&chunk).to_be_bytes());
        png
    }

    #[test]
    fn a_picture_file_past_the_side_cap_is_refused_from_its_header() {
        let path = std::env::temp_dir().join(format!("reprise-notify-{}.png", std::process::id()));
        std::fs::write(&path, png_header(4097, 4097)).unwrap();
        let got = decode_picture(&path);
        std::fs::remove_file(&path).unwrap();
        assert!(
            matches!(got, Err(image::ImageError::Limits(_))),
            "refused for its size: {got:?}"
        );
    }

    #[test]
    fn markup_becomes_plain_text() {
        assert_eq!(plain_text("<b>Bold</b> and <i>it</i>"), "Bold and it");
        assert_eq!(
            plain_text("See <a href=\"https://x.org/?a=1&amp;b=2\">the site</a>"),
            "See the site"
        );
        assert_eq!(plain_text("a &lt; b &amp;&amp; c &gt; d"), "a < b && c > d");
        assert_eq!(plain_text("&quot;q&quot; &apos;&#65;&#x42;"), "\"q\" 'AB");
        assert_eq!(plain_text("one<br/>two<BR>three"), "one\ntwo\nthree");
        assert_eq!(plain_text("<img src=\"x.png\" alt=\"a cat\"/>!"), "a cat!");
        assert_eq!(plain_text("x < y"), "x < y");
        assert_eq!(plain_text("fish & chips &bogus;"), "fish & chips &bogus;");
        assert_eq!(
            plain_text("&\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}"),
            "&\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}"
        );
    }

    #[test]
    fn actions_pair_up_and_drop_what_cannot_come_back() {
        let raw: Vec<String> = [
            "default", "", "open", "Open", "open", "Again", "", "Blank", "odd",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(
            actions(&raw),
            vec![
                ("default".to_string(), String::new()),
                ("open".to_string(), "Open".to_string())
            ]
        );
        let many: Vec<String> = (0..100)
            .flat_map(|i| [format!("k{i}"), "L".into()])
            .collect();
        assert_eq!(actions(&many).len(), MAX_ACTIONS);
        let long = vec!["k".repeat(300), "Label".into()];
        assert!(actions(&long).is_empty());
    }

    fn round_trip(map: HashMap<&str, Value<'_>>) -> Vec<u8> {
        let ctxt = Context::new_dbus(LE, 0);
        to_bytes(ctxt, &map).unwrap().to_vec()
    }

    fn rgba(w: i32, h: i32) -> ImageData<'static> {
        let data: &'static [u8] = vec![255u8; (w * h * 4) as usize].leak();
        ImageData {
            width: w,
            height: h,
            stride: w * 4,
            alpha: true,
            bits: 8,
            channels: 4,
            data,
        }
    }

    #[test]
    fn image_data_decodes_at_every_notification_size() {
        let srcs = image_sources(&rgba(40, 40));
        assert_eq!(
            srcs.iter().map(|s| s.width).collect::<Vec<_>>(),
            [16, 32, 48, 96]
        );
    }

    #[test]
    fn a_wide_image_keeps_its_aspect() {
        let s = &image_sources(&rgba(160, 90))[3];
        assert_eq!((s.width, s.height), (96, 96), "square canvas");
        assert_eq!(s.pixels[0] >> 24, 0, "letterbox rows are clear");
        assert_eq!(
            s.pixels[48 * 96 + 48] >> 24,
            0xFF,
            "the picture fills the middle"
        );
    }

    #[test]
    fn hints_read_urgency_flags_names_and_image_data() {
        let pixels: Vec<u8> = vec![255, 0, 0, 255, 0, 0, 255, 128];
        let mut map: HashMap<&str, Value> = HashMap::new();
        map.insert("urgency", Value::U8(2));
        map.insert("transient", Value::Bool(true));
        map.insert("resident", Value::Bool(false));
        map.insert("desktop-entry", Value::from("org.example.Mail"));
        map.insert("image-path", Value::from("/tmp/x.png"));
        map.insert(
            "image-data",
            Value::from((2i32, 1i32, 8i32, true, 8i32, 4i32, pixels.clone())),
        );
        map.insert("x-other", Value::from(vec![1u32, 2, 3]));
        let bytes = round_trip(map);
        let data = zbus::zvariant::serialized::Data::new(&bytes[..], Context::new_dbus(LE, 0));
        let (raw, _): (HashMap<String, Hint>, _) = data.deserialize().unwrap();
        assert_eq!(raw.get("x-other"), Some(&Hint::Other));
        let h = hints(raw);
        assert_eq!(h.urgency, Urgency::Critical);
        assert!(h.transient && !h.resident);
        assert_eq!(h.desktop_entry.as_deref(), Some("org.example.Mail"));
        assert_eq!(h.image_path.as_deref(), Some("/tmp/x.png"));
        let img = h.image.expect("image-data");
        assert_eq!((img.width, img.height, img.channels), (2, 1, 4));
        assert_eq!(img.data, &pixels[..]);
        let src = image_sources(&img);
        assert_eq!(
            src.iter().map(|s| s.width).collect::<Vec<_>>(),
            [16, 32, 48, 96]
        );
        assert_eq!(src[0].pixels[4 * 16], 0xFFFF_0000);
        assert_eq!(
            src[0].pixels[4 * 16 + 15],
            reprisekit::icon::premultiply(0x8000_00FF)
        );
    }

    #[test]
    fn sound_hints_pick_a_file_over_a_name_and_suppress_silences_both() {
        let read = |map: HashMap<&str, Value>| {
            let bytes = round_trip(map);
            let data = zbus::zvariant::serialized::Data::new(&bytes[..], Context::new_dbus(LE, 0));
            let (raw, _): (HashMap<String, Hint>, _) = data.deserialize().unwrap();
            let h = hints(raw);
            (h.sound.clone(), h.balloon_sound())
        };
        let mut map: HashMap<&str, Value> = HashMap::new();
        map.insert("sound-name", Value::from("bell"));
        map.insert("sound-file", Value::from("/usr/share/sounds/x.oga"));
        let file = Sound::File("/usr/share/sounds/x.oga".into());
        assert_eq!(read(map), (Some(file.clone()), Some(file)));

        let mut map: HashMap<&str, Value> = HashMap::new();
        map.insert("sound-name", Value::from("bell"));
        map.insert("sound-file", Value::from("relative.oga"));
        let bell = Sound::Name("bell".into());
        assert_eq!(read(map), (Some(bell.clone()), Some(bell)));

        let default = Sound::Name(DEFAULT_SOUND.into());
        assert_eq!(read(HashMap::new()), (None, Some(default)));

        let mut map: HashMap<&str, Value> = HashMap::new();
        map.insert("sound-name", Value::from("bell"));
        map.insert("suppress-sound", Value::Bool(true));
        assert_eq!(read(map).1, None);
    }

    #[test]
    fn older_hint_names_and_integer_flags_still_count() {
        let mut map: HashMap<&str, Value> = HashMap::new();
        map.insert("urgency", Value::U8(0));
        map.insert("transient", Value::I32(1));
        map.insert("image_path", Value::from("dialog-information"));
        let bytes = round_trip(map);
        let data = zbus::zvariant::serialized::Data::new(&bytes[..], Context::new_dbus(LE, 0));
        let (raw, _): (HashMap<String, Hint>, _) = data.deserialize().unwrap();
        let h = hints(raw);
        assert_eq!(h.urgency, Urgency::Low);
        assert!(h.transient);
        assert_eq!(h.image_path.as_deref(), Some("dialog-information"));
    }

    #[test]
    fn image_data_that_does_not_fit_its_bytes_is_refused() {
        let data = [0u8; 12];
        let img = |w, h, stride, ch| ImageData {
            width: w,
            height: h,
            stride,
            alpha: ch == 4,
            bits: 8,
            channels: ch,
            data: &data,
        };
        assert_eq!(
            image_sources(&img(2, 1, 8, 4)).len(),
            crate::tray::NOTIFY_SIZES.len()
        );
        assert!(image_sources(&img(2, 2, 8, 4)).is_empty(), "short by a row");
        assert!(
            image_sources(&img(2, 1, 4, 4)).is_empty(),
            "stride under a row"
        );
        assert!(image_sources(&img(0, 1, 8, 4)).is_empty());
        assert!(image_sources(&img(1_000_000, 1, 8, 4)).is_empty());
        assert_eq!(
            image_sources(&img(2, 2, 6, 3)).len(),
            crate::tray::NOTIFY_SIZES.len(),
            "RGB rows of 6"
        );
    }

    #[test]
    fn a_picture_file_larger_than_it_shows_is_scaled_to_each_size() {
        let dir = crate::harness::scratch();
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("notify-picture-4k.png");
        image::RgbaImage::from_pixel(3840, 2160, image::Rgba([0x20, 0x40, 0x80, 0xFF]))
            .save(&path)
            .unwrap();
        let sources = picture_file_sources(&path);
        assert_eq!(sources.len(), crate::tray::NOTIFY_SIZES.len());
        let large = sources.last().unwrap();
        assert_eq!((large.width, large.height), (96, 96));
        assert_ne!(large.pixels[48 * 96 + 48], 0, "the picture is drawn");
        let big = image::RgbaImage::new(1, 5000);
        let tall = dir.join("notify-picture-tall.png");
        big.save(&tall).unwrap();
        assert!(
            picture_file_sources(&tall).is_empty(),
            "a side past 4096 is refused"
        );
    }

    #[test]
    fn a_desktop_entry_hint_names_a_file_in_the_data_dirs() {
        assert_eq!(desktop_id("firefox").as_deref(), Some("firefox.desktop"));
        assert_eq!(
            desktop_id("org.gnome.Nautilus.desktop").as_deref(),
            Some("org.gnome.Nautilus.desktop")
        );
        assert_eq!(desktop_id("../etc/passwd"), None);
        assert_eq!(desktop_id(""), None);
    }
}
