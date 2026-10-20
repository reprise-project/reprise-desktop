//! A queue's PPD with its Default lines set from IPP defaults, as `lpadmin -o` writes them.
//! GTK print dialogs start from the PPD's defaults, not from the queue's IPP ones.

/// The defaults cupsd reports from the PPD itself: once the PPD carries one, a
/// queue option of the same name only adds a second, stale `*-default`.
pub const CARRIED: [&str; 2] = ["media", "sides"];

/// The PageSize Default lines: one choice moves all four.
const PAGE_KEYS: [&str; 4] = ["PageSize", "PageRegion", "PaperDimension", "ImageableArea"];

/// cupsd's order for the option that maps `sides`.
const SIDES_OPTIONS: [&str; 5] = [
    "Duplex",
    "JCLDuplex",
    "EFDuplex",
    "EFDuplexing",
    "KD03Duplex",
];

/// The options that switch a driver to grey and the choice that does, in
/// cupsd's order.
const GRAY: [(&str, &str); 5] = [
    ("ColorModel", "Gray"),
    ("HPColorMode", "grayscale"),
    ("BRMonoColor", "Mono"),
    ("CNIJSGrayScale", "1"),
    ("HPColorAsGray", "True"),
];

/// Adobe's size-matching tolerance, in points.
const EPSILON: f64 = 5.0;

/// Preset options by print-color-mode (monochrome, color) and print-quality
/// (draft, normal, high), as libcups builds them from `*APPrinterPreset`.
type Presets = [[Vec<(String, String)>; 3]; 2];

/// `ppd` with the Default lines of the IPP options in `defaults` (media,
/// sides, print-color-mode, print-quality and their values) set to the
/// choices they map to, and the options that found a choice. Colour and
/// quality also mark every option of the preset they pick, over the grey
/// switch and quality option. Every other byte stays as it was.
pub fn with_defaults(ppd: &[u8], defaults: &[(String, String)]) -> (Vec<u8>, Vec<String>) {
    let p = Parsed::new(ppd);
    let mut marks: Vec<(&str, &str)> = Vec::new();
    let mut applied = Vec::new();
    let value_of = |name: &str| {
        defaults
            .iter()
            .find(|(o, _)| o == name)
            .map(|(_, v)| v.as_str())
    };
    for (option, value) in defaults {
        let m = match option.as_str() {
            "media" => p.page_size(value).map(|c| ("PageSize", c)),
            "sides" => p.sides(value),
            "print-color-mode" => p.color(value),
            "print-quality" => p.quality(value),
            _ => None,
        };
        if let Some(m) = m {
            mark(&mut marks, m);
            applied.push(option.clone());
        }
    }
    let (color, quality) = (value_of("print-color-mode"), value_of("print-quality"));
    if color.is_some() || quality.is_some() {
        let preset: Vec<_> = p
            .preset(color, quality)
            .iter()
            .filter_map(|(o, c)| Some((p.option(o)?, p.find(o, c)?)))
            .collect();
        if !preset.is_empty() {
            for m in preset {
                mark(&mut marks, m);
            }
            for o in ["print-color-mode", "print-quality"] {
                if value_of(o).is_some() && !applied.iter().any(|a| a == o) {
                    applied.push(o.to_string());
                }
            }
        }
    }
    let marked = |key: &str| {
        let key = if PAGE_KEYS.contains(&key) {
            "PageSize"
        } else {
            key
        };
        marks.iter().find(|(k, _)| *k == key).map(|(_, c)| *c)
    };
    let mut out = Vec::with_capacity(ppd.len());
    let mut quoted = false;
    for line in lines(ppd) {
        let was_quoted = quoted;
        quoted ^= line.iter().filter(|&&b| b == b'"').count() % 2 == 1;
        let rewritten = (!was_quoted)
            .then(|| default_line(line))
            .flatten()
            .and_then(|(key, current)| {
                let choice = marked(key)?;
                (choice != current).then(|| {
                    let end = &line[line.trim_ascii_end().len()..];
                    [format!("*Default{key}: {choice}").as_bytes(), end].concat()
                })
            });
        out.extend_from_slice(rewritten.as_deref().unwrap_or(line));
    }
    (out, applied)
}

fn mark<'a>(marks: &mut Vec<(&'a str, &'a str)>, m: (&'a str, &'a str)) {
    marks.retain(|(k, _)| *k != m.0);
    marks.push(m);
}

/// The lines of `ppd`, each with its line ending.
fn lines(ppd: &[u8]) -> impl Iterator<Item = &[u8]> {
    ppd.split_inclusive(|&b| b == b'\n')
}

/// A `*DefaultKey: value` line as its key and value.
fn default_line(line: &[u8]) -> Option<(&str, &str)> {
    let rest = std::str::from_utf8(line.strip_prefix(b"*Default")?).ok()?;
    let end = rest.find(|c: char| c == ':' || c.is_whitespace())?;
    let (key, value) = rest.split_at(end);
    let value = value.trim_start().strip_prefix(':').unwrap_or(value).trim();
    Some((key, value))
}

/// The option choices of a PPD (`*PageSize A4/A4: ...` is PageSize's A4)
/// and each page size's dimensions in points.
struct Parsed {
    choices: Vec<(String, String)>,
    dims: Vec<(String, (f64, f64))>,
    presets: Presets,
}

impl Parsed {
    fn new(ppd: &[u8]) -> Parsed {
        let mut choices = Vec::new();
        let mut dims = Vec::new();
        let mut quoted = false;
        for line in lines(ppd) {
            let was_quoted = quoted;
            quoted ^= line.iter().filter(|&&b| b == b'"').count() % 2 == 1;
            if was_quoted {
                continue;
            }
            let Some((key, choice, value)) = choice_line(line) else {
                continue;
            };
            if key == "PaperDimension" {
                let mut n = value
                    .trim_matches('"')
                    .split_whitespace()
                    .map(str::parse::<f64>);
                if let (Some(Ok(w)), Some(Ok(h))) = (n.next(), n.next()) {
                    dims.push((choice.to_string(), (w, h)));
                }
            } else {
                choices.push((key.to_string(), choice.to_string()));
            }
        }
        let mut p = Parsed {
            choices,
            dims,
            presets: presets(ppd),
        };
        p.gray_presets();
        p
    }

    fn choices_of<'a>(&'a self, option: &'a str) -> impl Iterator<Item = &'a str> {
        self.choices
            .iter()
            .filter(move |(k, _)| k == option)
            .map(|(_, c)| c.as_str())
    }

    fn find<'a>(&'a self, option: &'a str, choice: &str) -> Option<&'a str> {
        self.choices_of(option)
            .find(|c| c.eq_ignore_ascii_case(choice))
    }

    /// `option` as this PPD spells it.
    fn option(&self, option: &str) -> Option<&str> {
        self.choices
            .iter()
            .find(|(k, _)| k == option)
            .map(|(k, _)| k.as_str())
    }

    /// With no monochrome preset, libcups makes them from the colour ones
    /// (or none, at normal quality) plus the grey switch.
    fn gray_presets(&mut self) {
        if self.presets[0].iter().any(|p| !p.is_empty()) {
            return;
        }
        let Some((option, gray)) = GRAY.into_iter().find(|(o, g)| self.find(o, g).is_some()) else {
            return;
        };
        for q in 0..3 {
            if self.presets[1][q].is_empty() && q != 1 {
                continue;
            }
            let mut mono = self.presets[1][q].clone();
            mono.retain(|(o, _)| o != option);
            mono.push((option.to_string(), gray.to_string()));
            self.presets[0][q] = mono;
        }
    }

    /// The preset cupsMarkOptions applies for a print-color-mode and
    /// print-quality, falling back to normal quality, then to colour.
    fn preset(&self, color: Option<&str>, quality: Option<&str>) -> &[(String, String)] {
        let t = &self.presets;
        let mut c = usize::from(color != Some("monochrome"));
        let mut q = quality.map_or(1, |q| {
            (q.trim().parse::<i64>().unwrap_or(0) - 3).clamp(0, 2) as usize
        });
        if t[c][q].is_empty() {
            if !t[c][1].is_empty() {
                q = 1;
            } else if !t[1][q].is_empty() {
                c = 1;
            } else {
                (c, q) = (1, 1);
            }
        }
        &t[c][q]
    }

    /// The PageSize a PWG media keyword names: the size cupsd listed under
    /// that keyword, else the first one of its dimensions.
    fn page_size(&self, keyword: &str) -> Option<&str> {
        let (rest, size) = keyword.rsplit_once('_')?;
        let name = rest.split_once('_').map_or(rest, |(_, n)| n);
        let want = pwg_dims(size)?;
        let fits = |c: &str| {
            self.dims
                .iter()
                .find(|(d, _)| d == c)
                .is_some_and(|(_, (w, h))| {
                    (w - want.0).abs() <= EPSILON && (h - want.1).abs() <= EPSILON
                })
        };
        let sizes = || {
            self.choices_of("PageSize")
                .filter(|c| !c.eq_ignore_ascii_case("Custom"))
        };
        sizes()
            .find(|c| unppdize(c) == name && fits(c))
            .or_else(|| sizes().find(|c| fits(c)))
    }

    fn sides(&self, sides: &str) -> Option<(&'static str, &str)> {
        let option = SIDES_OPTIONS
            .into_iter()
            .find(|o| self.choices_of(o).next().is_some())?;
        let names: &[&str] = match sides {
            "one-sided" => &["None", "False"],
            "two-sided-long-edge" => &["DuplexNoTumble", "LongEdge", "Top"],
            "two-sided-short-edge" => &["DuplexTumble", "ShortEdge", "Bottom"],
            _ => return None,
        };
        let choice = self
            .choices_of(option)
            .find(|c| names.iter().any(|n| c.eq_ignore_ascii_case(n)))?;
        Some((option, choice))
    }

    /// The grey switch for monochrome; for color, the same option's first
    /// choice that does not read as grey.
    fn color(&self, mode: &str) -> Option<(&'static str, &str)> {
        let (option, gray) = GRAY.into_iter().find(|(o, g)| self.find(o, g).is_some())?;
        let gray = self.find(option, gray)?;
        let choice = match mode {
            "monochrome" => gray,
            "color" => {
                let grey_word = |c: &str| {
                    let c = c.to_ascii_lowercase();
                    ["gray", "grey", "mono", "black"]
                        .iter()
                        .any(|w| c.contains(w))
                };
                let others = || self.choices_of(option).filter(|c| *c != gray);
                others()
                    .find(|c| !grey_word(c))
                    .or_else(|| others().next())?
            }
            _ => return None,
        };
        Some((option, choice))
    }

    /// An IPP print-quality (3 draft, 4 normal, 5 high) on the option cupsd
    /// reads print-quality-supported from.
    fn quality(&self, quality: &str) -> Option<(&'static str, &str)> {
        let option = ["OutputMode", "cupsPrintQuality"]
            .into_iter()
            .find(|o| self.choices_of(o).next().is_some())?;
        let names: &[&str] = match quality {
            "3" => &["draft", "fast"],
            "4" => &["normal"],
            "5" => &["best", "high"],
            _ => return None,
        };
        let choice = self
            .choices_of(option)
            .find(|c| names.iter().any(|n| c.eq_ignore_ascii_case(n)))?;
        Some((option, choice))
    }
}

/// The `*APPrinterPreset` table of `ppd`: the first preset for each slot,
/// skipping Photo or coated-paper presets below high quality, as libcups does.
fn presets(ppd: &[u8]) -> Presets {
    let text = String::from_utf8_lossy(ppd);
    let mut out = Presets::default();
    let starts = text
        .match_indices("*APPrinterPreset ")
        .map(|(i, _)| i)
        .filter(|&i| i == 0 || text.as_bytes()[i - 1] == b'\n');
    for at in starts {
        let rest = &text[at..];
        let Some(open) = rest
            .find(':')
            .and_then(|c| rest[c..].find('"').map(|q| c + q + 1))
        else {
            continue;
        };
        let Some(len) = rest[open..].find('"') else {
            continue;
        };
        let mut tokens = rest[open..open + len].split_whitespace();
        let mut pairs = Vec::new();
        while let (Some(k), Some(v)) = (tokens.next(), tokens.next()) {
            pairs.push((k, v));
        }
        let get = |name: &str| {
            pairs
                .iter()
                .rev()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| *v)
        };
        let Some(quality) = get("com.apple.print.preset.quality") else {
            continue;
        };
        let q = match quality {
            "low" => 0,
            "high" => 2,
            _ => 1,
        };
        let coated = get("com.apple.print.preset.media-front-coating")
            .is_some_and(|c| c != "none" && c != "autodetect");
        if q != 2 && (get("com.apple.print.preset.graphicsType") == Some("Photo") || coated) {
            continue;
        }
        let mono = match (
            get("com.apple.print.preset.output-mode"),
            get("*ColorModel"),
        ) {
            (Some(mode), _) => mode == "monochrome",
            (None, Some(model)) => model.eq_ignore_ascii_case("Gray"),
            (None, None) => false,
        };
        let slot = &mut out[usize::from(!mono)][q];
        if slot.is_empty() {
            *slot = pairs
                .iter()
                .filter_map(|(k, v)| Some((k.strip_prefix('*')?.to_string(), v.to_string())))
                .collect();
        }
    }
    out
}

/// A `*Option Choice/Text: value` line as option, choice and value.
fn choice_line(line: &[u8]) -> Option<(&str, &str, &str)> {
    let line = std::str::from_utf8(line.strip_prefix(b"*")?).ok()?;
    let (head, value) = line.split_once(':')?;
    let (key, spec) = head.split_once(char::is_whitespace)?;
    if key.is_empty() || key.starts_with(['%', '?']) || key.starts_with("Default") {
        return None;
    }
    let choice = spec.split('/').next()?.trim();
    (!choice.is_empty()).then_some((key, choice, value.trim()))
}

/// The dimensions in points of a PWG size part: "8.5x11in", "210x297mm".
fn pwg_dims(size: &str) -> Option<(f64, f64)> {
    let (dims, per_unit) = if let Some(d) = size.strip_suffix("in") {
        (d, 72.0)
    } else {
        (size.strip_suffix("mm")?, 72.0 / 25.4)
    };
    let (w, h) = dims.split_once('x')?;
    Some((
        w.parse::<f64>().ok()? * per_unit,
        h.parse::<f64>().ok()? * per_unit,
    ))
}

/// A PPD choice name as cupsd spells it in a PWG keyword: lowercase, with
/// dashes between words and before digits ("A4.Fullbleed" is
/// "a-4.fullbleed").
fn unppdize(ppd: &str) -> String {
    let b = ppd.as_bytes();
    let dashed = |c: u8| c == b'-' || c == b'_' || c == b'.';
    if b.first().is_some_and(u8::is_ascii_lowercase)
        && b.windows(2)
            .all(|w| !w[1].is_ascii_uppercase() && !(w[1] != b'-' && dashed(w[1])) && w != b"--")
        && !ppd.ends_with('-')
    {
        return ppd.to_string();
    }
    let mut out = String::new();
    let mut nodash = true;
    for (i, &c) in b.iter().enumerate() {
        if c.is_ascii_alphanumeric() || c == b'.' || c == b'_' {
            out.push(c.to_ascii_lowercase() as char);
            nodash = false;
        } else if c == b'-' && !nodash {
            out.push('-');
            nodash = true;
        }
        let next = b.get(i + 1).copied().unwrap_or(0);
        if !nodash
            && (!c.is_ascii_uppercase() && c.is_ascii_alphanumeric() && next.is_ascii_uppercase()
                || !c.is_ascii_digit() && next.is_ascii_digit())
        {
            out.push('-');
            nodash = true;
        }
    }
    out.trim_end_matches('-').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const LASER: &str = "*PPD-Adobe: \"4.3\"\r
*ColorDevice: True\r
*JCLOpenUI *PageSize/Page Size: PickOne\r
*DefaultPageSize: Letter\r
*PageSize Letter/Letter: \"@PJL SET PAPER=LETTER<0A>\"\r
*PageSize Letter.Fullbleed/Letter (Borderless): \"@PJL SET PAPER=LETTER<0A>\"\r
*PageSize A4/A4: \"@PJL SET PAPER=A4<0A>\"\r
*PageSize A4.Fullbleed/A4 (Borderless): \"@PJL SET PAPER=A4<0A>\"\r
*JCLCloseUI: *PageSize\r
*DefaultPageRegion: Letter\r
*PageRegion Letter/Letter: \"\"\r
*PageRegion A4/A4: \"\"\r
*DefaultImageableArea: Letter\r
*ImageableArea Letter/Letter: \"18 36 594 756\"\r
*ImageableArea A4/A4: \"18 36 577 806\"\r
*DefaultPaperDimension: Letter\r
*PaperDimension Letter/Letter: \"612 792\"\r
*PaperDimension Letter.Fullbleed/Letter (Borderless): \"612 792\"\r
*PaperDimension A4/A4: \"595 842\"\r
*PaperDimension A4.Fullbleed/A4 (Borderless): \"595 842\"\r
*OpenUI *Duplex/Double-Sided Printing: PickOne\r
*DefaultDuplex: None\r
*Duplex None/Off: \"\"\r
*Duplex DuplexNoTumble/Long-Edge binding: \"\"\r
*Duplex DuplexTumble/Short-Edge binding: \"\"\r
*CloseUI: *Duplex\r
*OpenUI *ColorModel/Color Mode: PickOne\r
*DefaultColorModel: RGB\r
*ColorModel Gray/Grayscale: \"\"\r
*ColorModel RGB/Color: \"\"\r
*CloseUI: *ColorModel\r
*OpenUI *cupsPrintQuality/Print Quality: PickOne\r
*DefaultcupsPrintQuality: Normal\r
*cupsPrintQuality Draft/Draft: \"\"\r
*cupsPrintQuality Normal/Normal: \"\"\r
*cupsPrintQuality High/High: \"\"\r
*CloseUI: *cupsPrintQuality\r
*Footer: \"one\r
*DefaultPageSize: Legal\r
two\"\r
";

    fn set(ppd: &str, defaults: &[(&str, &str)]) -> (String, Vec<String>) {
        let d: Vec<(String, String)> = defaults
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let (out, applied) = with_defaults(ppd.as_bytes(), &d);
        (String::from_utf8(out).expect("UTF-8"), applied)
    }

    fn defaults_in(ppd: &str) -> Vec<&str> {
        ppd.lines()
            .filter(|l| l.starts_with("*Default"))
            .map(str::trim_end)
            .collect()
    }

    #[test]
    fn media_moves_all_four_page_size_defaults() {
        let (out, applied) = set(LASER, &[("media", "iso_a4_210x297mm")]);
        assert_eq!(applied, ["media"]);
        assert_eq!(
            defaults_in(&out),
            [
                "*DefaultPageSize: A4",
                "*DefaultPageRegion: A4",
                "*DefaultImageableArea: A4",
                "*DefaultPaperDimension: A4",
                "*DefaultDuplex: None",
                "*DefaultColorModel: RGB",
                "*DefaultcupsPrintQuality: Normal",
                "*DefaultPageSize: Legal",
            ]
        );
        assert_eq!(out.len(), LASER.len() - 4 * "Letter".len() + 4 * "A4".len());
        assert!(
            out.contains("*DefaultPageSize: A4\r\n"),
            "line endings stay"
        );
    }

    #[test]
    fn a_borderless_keyword_picks_the_borderless_size() {
        let (out, _) = set(LASER, &[("media", "om_a-4.fullbleed_209.9x297.04mm")]);
        assert!(out.contains("*DefaultPageSize: A4.Fullbleed\r\n"));
        let (out, _) = set(LASER, &[("media", "oe_letter.fullbleed_8.5x11in")]);
        assert!(out.contains("*DefaultPageSize: Letter.Fullbleed\r\n"));
    }

    #[test]
    fn sides_color_and_quality_set_their_options() {
        let (out, applied) = set(
            LASER,
            &[
                ("sides", "two-sided-long-edge"),
                ("print-color-mode", "monochrome"),
                ("print-quality", "5"),
            ],
        );
        assert_eq!(applied, ["sides", "print-color-mode", "print-quality"]);
        assert!(out.contains("*DefaultDuplex: DuplexNoTumble\r\n"));
        assert!(out.contains("*DefaultColorModel: Gray\r\n"));
        assert!(out.contains("*DefaultcupsPrintQuality: High\r\n"));
        let (back, _) = set(
            &out,
            &[("print-color-mode", "color"), ("sides", "one-sided")],
        );
        assert!(back.contains("*DefaultColorModel: RGB\r\n"));
        assert!(back.contains("*DefaultDuplex: None\r\n"));
    }

    #[test]
    fn a_value_the_ppd_has_no_choice_for_changes_nothing() {
        let (out, applied) = set(
            LASER,
            &[
                ("media", "iso_a3_297x420mm"),
                ("print-quality", "7"),
                ("output-bin", "face-up"),
            ],
        );
        assert!(applied.is_empty());
        assert_eq!(out, LASER);
        let plain = "*PPD-Adobe: \"4.3\"\n*DefaultPageSize: A4\n";
        let (out, applied) = set(
            plain,
            &[("sides", "one-sided"), ("print-color-mode", "color")],
        );
        assert!(applied.is_empty());
        assert_eq!(out, plain);
    }

    /// The colour and quality part of the PPD cupsd writes for an IPP
    /// Everywhere queue.
    const EVERYWHERE: &str = "*PPD-Adobe: \"4.3\"
*ColorDevice: True
*OpenUI *cupsPrintQuality: PickOne
*OrderDependency: 10 AnySetup *cupsPrintQuality
*en.Translation cupsPrintQuality/Print Quality: \"\"
*DefaultcupsPrintQuality: Normal
*cupsPrintQuality Draft: \"<</HWResolution[300 300]>>setpagedevice\"
*en.cupsPrintQuality Draft/Draft: \"\"
*cupsPrintQuality Normal: \"<</HWResolution[600 600]>>setpagedevice\"
*en.cupsPrintQuality Normal/Normal: \"\"
*cupsPrintQuality High: \"<</HWResolution[1200 1200]>>setpagedevice\"
*en.cupsPrintQuality High/High: \"\"
*CloseUI: *cupsPrintQuality
*OpenUI *ColorModel: PickOne
*OrderDependency: 10 AnySetup *ColorModel
*en.Translation ColorModel/Color Mode: \"\"
*ColorModel Gray: \"<</cupsColorSpace 18/cupsBitsPerColor 8/cupsColorOrder 0/cupsCompression 0>>setpagedevice\"
*en.ColorModel Gray/Grayscale: \"\"
*ColorModel RGB: \"<</cupsColorSpace 19/cupsBitsPerColor 8/cupsColorOrder 0/cupsCompression 0>>setpagedevice\"
*en.ColorModel RGB/Color: \"\"
*DefaultColorModel: RGB
*CloseUI: *ColorModel
*APPrinterPreset Gray_with_Paper_Auto-Detect_-_Draft/Draft B&W: \"
  *cupsPrintQuality Draft *ColorModel Gray
  com.apple.print.preset.graphicsType General
  com.apple.print.preset.quality low
  com.apple.print.preset.media-front-coating autodetect
  com.apple.print.preset.output-mode monochrome\"
*End
*APPrinterPreset Gray_with_Paper_Auto-Detect/Black and White: \"
  *cupsPrintQuality Normal *ColorModel Gray
  com.apple.print.preset.graphicsType General
  com.apple.print.preset.quality mid
  com.apple.print.preset.media-front-coating autodetect
  com.apple.print.preset.output-mode monochrome\"
*End
*APPrinterPreset Color_with_Paper_Auto-Detect/Color: \"
  *cupsPrintQuality Normal *ColorModel RGB
  com.apple.print.preset.graphicsType General
  com.apple.print.preset.quality mid
  com.apple.print.preset.media-front-coating autodetect
  com.apple.print.preset.output-mode color\"
*End
*APPrinterPreset Photo_with_Paper_Auto-Detect/Photo: \"
  *cupsPrintQuality High *ColorModel RGB
  com.apple.print.preset.graphicsType Photo
  com.apple.print.preset.quality high
  com.apple.print.preset.media-front-coating autodetect
  com.apple.print.preset.output-mode color\"
*End
";

    fn color_and_quality(ppd: &str, color: &str, quality: &str) -> Vec<String> {
        let (out, applied) = set(
            ppd,
            &[("print-color-mode", color), ("print-quality", quality)],
        );
        assert_eq!(applied, ["print-color-mode", "print-quality"]);
        defaults_in(&out).into_iter().map(String::from).collect()
    }

    #[test]
    fn colour_and_quality_mark_the_preset_they_pick() {
        assert_eq!(
            color_and_quality(EVERYWHERE, "monochrome", "3"),
            [
                "*DefaultcupsPrintQuality: Draft",
                "*DefaultColorModel: Gray"
            ]
        );
        assert_eq!(
            color_and_quality(EVERYWHERE, "color", "5"),
            ["*DefaultcupsPrintQuality: High", "*DefaultColorModel: RGB"]
        );
    }

    #[test]
    fn a_missing_preset_falls_back_to_normal_quality() {
        assert_eq!(
            color_and_quality(EVERYWHERE, "monochrome", "5"),
            [
                "*DefaultcupsPrintQuality: Normal",
                "*DefaultColorModel: Gray"
            ]
        );
        assert_eq!(
            color_and_quality(EVERYWHERE, "color", "3"),
            [
                "*DefaultcupsPrintQuality: Normal",
                "*DefaultColorModel: RGB"
            ]
        );
    }

    #[test]
    fn photo_and_coated_presets_count_only_at_high_quality() {
        let ppd = "*PPD-Adobe: \"4.3\"
*OpenUI *OutputMode: PickOne
*DefaultOutputMode: Normal
*OutputMode Draft/Draft: \"\"
*OutputMode Normal/Normal: \"\"
*OutputMode Best/Best: \"\"
*CloseUI: *OutputMode
*OpenUI *MediaType: PickOne
*DefaultMediaType: Plain
*MediaType Plain/Plain Paper: \"\"
*MediaType Glossy/Photo Paper: \"\"
*CloseUI: *MediaType
*OpenUI *ColorModel: PickOne
*DefaultColorModel: RGB
*ColorModel Gray/Grayscale: \"\"
*ColorModel RGB/Color: \"\"
*CloseUI: *ColorModel
*APPrinterPreset Photo_Draft/Photo Draft: \"*OutputMode Draft *ColorModel RGB com.apple.print.preset.graphicsType Photo com.apple.print.preset.quality low\"
*End
*APPrinterPreset Glossy_Normal/Glossy: \"*OutputMode Normal *MediaType Glossy com.apple.print.preset.quality mid com.apple.print.preset.media-front-coating glossy\"
*End
*APPrinterPreset Plain_Draft/Fast Draft: \"*OutputMode Draft *MediaType Plain com.apple.print.preset.quality low\"
*End
*APPrinterPreset Glossy_Best/Photo on Photo Paper: \"*OutputMode Best *MediaType Glossy com.apple.print.preset.graphicsType Photo com.apple.print.preset.quality high com.apple.print.preset.media-front-coating glossy\"
*End
";
        let (out, _) = set(
            ppd,
            &[("print-color-mode", "color"), ("print-quality", "5")],
        );
        assert_eq!(
            defaults_in(&out),
            [
                "*DefaultOutputMode: Best",
                "*DefaultMediaType: Glossy",
                "*DefaultColorModel: RGB"
            ]
        );
        let (out, _) = set(
            ppd,
            &[("print-color-mode", "color"), ("print-quality", "3")],
        );
        assert_eq!(
            defaults_in(&out),
            [
                "*DefaultOutputMode: Draft",
                "*DefaultMediaType: Plain",
                "*DefaultColorModel: RGB"
            ],
            "the Photo draft preset is passed over"
        );
        let (out, _) = set(
            ppd,
            &[("print-color-mode", "monochrome"), ("print-quality", "3")],
        );
        assert_eq!(
            defaults_in(&out),
            [
                "*DefaultOutputMode: Draft",
                "*DefaultMediaType: Plain",
                "*DefaultColorModel: Gray"
            ],
            "with no grey preset, the colour one plus the grey switch"
        );
    }

    #[test]
    fn a_preset_without_a_quality_is_passed_over() {
        let ppd = format!(
            "{LASER}*APPrinterPreset ChartsAndGraphs/Charts & Graphs: \"
\t*ColorModel RGB
\tcom.apple.print.preset.graphicsType General\"
*End
"
        );
        let (out, _) = set(
            &ppd,
            &[("print-color-mode", "monochrome"), ("print-quality", "5")],
        );
        assert!(out.contains("*DefaultColorModel: Gray\r\n"));
        assert!(out.contains("*DefaultcupsPrintQuality: High\r\n"));
    }

    #[test]
    fn choice_names_spell_as_cupsd_keywords() {
        assert_eq!(unppdize("A4.Fullbleed"), "a-4.fullbleed");
        assert_eq!(unppdize("Letter"), "letter");
        assert_eq!(unppdize("EnvC5"), "env-c-5");
        assert_eq!(unppdize("ISOB5"), "isob-5");
        assert_eq!(unppdize("letter"), "letter");
        assert_eq!(pwg_dims("8.5x11in"), Some((612.0, 792.0)));
        assert_eq!(pwg_dims("8.5x11"), None);
    }
}
