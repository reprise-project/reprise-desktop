//! A queue's media, sides, color and quality settings and their labels.

use super::*;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Setting {
    pub choices: Vec<String>,
    pub default: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub media: Setting,
    pub sides: Setting,
    pub color: Setting,
    pub quality: Setting,
}

impl Options {
    pub(super) fn from_group(g: &ipp::Group) -> Options {
        let setting = |name: &str| {
            let text = |v: &Value| match v {
                Value::Int(i) | Value::Enum(i) => Some(i.to_string()),
                Value::Str(_, s) => Some(s.clone()),
                _ => None,
            };
            let default = format!("{name}-default");
            Setting {
                choices: g
                    .get(&format!("{name}-supported"))
                    .unwrap_or_default()
                    .iter()
                    .filter_map(text)
                    .collect(),
                // cupsd lists a queue option before the PPD's default of the
                // same name; the PPD's is the one print dialogs start on.
                default: g
                    .attrs
                    .iter()
                    .rfind(|a| a.name == default)
                    .and_then(|a| a.values.first())
                    .and_then(text),
            }
        };
        Options {
            media: setting("media"),
            sides: setting("sides"),
            color: setting("print-color-mode"),
            quality: setting("print-quality"),
        }
    }
}

pub fn options(c: &Client, name: &str) -> Result<Options> {
    let req = Request::new(GET_PRINTER_ATTRIBUTES)
        .op_attr("printer-uri", Value::uri(&printer_uri(name)))
        .requested(&[
            "media-supported",
            "media-default",
            "sides-supported",
            "sides-default",
            "print-color-mode-supported",
            "print-color-mode-default",
            "print-quality-supported",
            "print-quality-default",
        ]);
    let r = c.call("/", req)?;
    Ok(r.first(PRINTER)
        .map(Options::from_group)
        .unwrap_or_default())
}

/// A PWG media keyword as a size name: "iso_a4_210x297mm" reads
/// "A4 (210 x 297 mm)", "na_letter_8.5x11in" "Letter (8.5 x 11 in)".
pub fn media_label(k: &str) -> String {
    let parts: Vec<&str> = k.split('_').collect();
    if parts.len() < 3 {
        return k.to_string();
    }
    let size = parts[parts.len() - 1];
    let name = parts[1..parts.len() - 1].join(" ");
    let name: Vec<String> = name
        .split(['-', ' '])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let code = w.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
                && w.chars().skip(1).all(|c| c.is_ascii_digit());
            if code || w.len() <= 2 && w.chars().all(|c| c.is_ascii_alphabetic()) {
                w.to_uppercase()
            } else {
                let mut c = w.chars();
                c.next()
                    .map(|f| f.to_uppercase().chain(c).collect())
                    .unwrap_or_default()
            }
        })
        .collect();
    let (dims, unit) = if let Some(d) = size.strip_suffix("mm") {
        (d, "mm")
    } else if let Some(d) = size.strip_suffix("in") {
        (d, "in")
    } else {
        (size, "")
    };
    match dims.split_once('x') {
        Some((w, h)) if !unit.is_empty() => format!("{} ({w} x {h} {unit})", name.join(" ")),
        _ => name.join(" "),
    }
}

pub fn sides_label(k: &str) -> String {
    match k {
        "one-sided" => "Off".into(),
        "two-sided-long-edge" => "Long edge (standard)".into(),
        "two-sided-short-edge" => "Short edge (flip up)".into(),
        k => reason_words(k),
    }
}

pub fn color_label(k: &str) -> String {
    match k {
        "monochrome" => "Black and white".into(),
        "color" => "Color".into(),
        "auto" => "Automatic".into(),
        "auto-monochrome" => "Automatic black and white".into(),
        "process-monochrome" => "Black and white (process)".into(),
        k => reason_words(k),
    }
}

pub fn quality_label(k: &str) -> String {
    match k {
        "3" => "Draft".into(),
        "4" => "Normal".into(),
        "5" => "High".into(),
        k => k.into(),
    }
}
