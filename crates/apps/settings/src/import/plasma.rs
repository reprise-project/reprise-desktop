//! Plasma's settings from its KConfig files in `$XDG_CONFIG_HOME`: kdeglobals, kcminputrc,
//! kxkbrc, kwinrc, kcmfonts and the desktop's applet file.

use super::{Device, Launcher, Setting, other};
use std::path::{Path, PathBuf};

/// A group's path (`[A][B]` as `["A", "B"]`) and its entries.
type Group = (Vec<String>, Vec<(String, String)>);

/// A KConfig file: its groups in file order.
#[derive(Debug, Default)]
pub(super) struct KConfig {
    groups: Vec<Group>,
}

/// A value with KConfig's escapes (`\s`, `\t`, `\n`, `\\`) undone.
fn unescape(v: &str) -> String {
    let mut out = String::new();
    let mut chars = v.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => out.push(' '),
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some(c) => out.push(c),
            None => {}
        }
    }
    out
}

impl KConfig {
    /// `text` read; a `[$e]` value has `$HOME` replaced by `home`, a localized key (`Name[de]`)
    /// is skipped.
    pub(super) fn parse(text: &str, home: &str) -> KConfig {
        let mut groups: Vec<Group> = vec![(Vec::new(), Vec::new())];
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(inner) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                let path = inner.split("][").map(str::to_string).collect();
                groups.push((path, Vec::new()));
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim();
            let (key, expand) = match key.split_once('[') {
                Some((k, flags)) if flags.starts_with('$') => (k, flags.contains('e')),
                Some(_) => continue,
                None => (key, false),
            };
            let mut value = unescape(value.trim());
            if expand {
                value = value.replace("${HOME}", home).replace("$HOME", home);
            }
            if let Some(g) = groups.last_mut() {
                g.1.push((key.to_string(), value));
            }
        }
        KConfig { groups }
    }

    fn read(path: &Path, home: &str) -> Option<KConfig> {
        std::fs::read_to_string(path)
            .ok()
            .map(|t| KConfig::parse(&t, home))
    }

    /// Key `key` of group `group`; the last one written wins.
    pub(super) fn get(&self, group: &[&str], key: &str) -> Option<&str> {
        (self.groups.iter().rev())
            .filter(|(g, _)| g.iter().map(String::as_str).eq(group.iter().copied()))
            .find_map(|(_, entries)| {
                entries
                    .iter()
                    .rev()
                    .find(|(k, _)| k == key)
                    .map(|(_, v)| v.as_str())
            })
    }

    fn groups(&self) -> impl Iterator<Item = (&[String], &[(String, String)])> {
        self.groups
            .iter()
            .map(|(g, e)| (g.as_slice(), e.as_slice()))
    }
}

fn flag(v: Option<&str>) -> Option<bool> {
    match v?.trim() {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => None,
    }
}

fn num(v: Option<&str>) -> Option<f64> {
    v?.trim().parse().ok()
}

/// "1930" (KWin's HHMM) as "19:30".
fn hhmm(v: &str) -> Option<String> {
    let v = v.trim();
    let n: u32 = v.parse().ok()?;
    let (h, m) = (n / 100, n % 100);
    (v.len() == 4 && h < 24 && m < 60).then(|| format!("{h:02}:{m:02}"))
}

/// The picture a wallpaper package (`contents/images/1920x1080.jpg`, ...) shows: the largest one,
/// from `images_dark` when dark and the package has one.
fn package_picture(dir: &Path, dark: bool) -> Option<PathBuf> {
    let contents = dir.join("contents");
    let images = match dark && contents.join("images_dark").is_dir() {
        true => contents.join("images_dark"),
        false => contents.join("images"),
    };
    let area = |p: &Path| -> u64 {
        let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let (w, h) = stem.split_once('x').unwrap_or(("0", "0"));
        w.parse::<u64>().unwrap_or(0) * h.parse::<u64>().unwrap_or(0)
    };
    std::fs::read_dir(images)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .max_by_key(|p| (area(p), std::cmp::Reverse(p.clone())))
}

/// The desktop's picture and Reprise's position for its fill mode: the containment on the first
/// screen, else the first with a picture.
fn wallpaper(applets: &KConfig, dark: bool) -> Option<Setting> {
    let mut found: Option<(bool, String, Option<&str>)> = None;
    for (g, _) in applets.groups() {
        let [c, id, w, plugin, general] = g else {
            continue;
        };
        if c != "Containments"
            || w != "Wallpaper"
            || plugin != "org.kde.image"
            || general != "General"
        {
            continue;
        }
        let Some(image) = applets.get(&[c, id, w, plugin, general], "Image") else {
            continue;
        };
        let first = applets.get(&["Containments", id], "lastScreen") == Some("0");
        let mode = applets.get(&[c, id, w, plugin, general], "FillMode");
        if found.as_ref().is_none_or(|f| first && !f.0) {
            found = Some((first, image.to_string(), mode));
        }
    }
    let (_, image, mode) = found?;
    let path = match image.strip_prefix("file://") {
        Some(p) => String::from_utf8(reprisefs::trash::percent_decode(p)).ok()?,
        None => image,
    };
    let path = match Path::new(&path).is_dir() {
        true => package_picture(Path::new(&path), dark)?
            .to_string_lossy()
            .into_owned(),
        false => path,
    };
    let position = match mode.and_then(|m| m.trim().parse::<u32>().ok()) {
        Some(3..=5) => "tile",
        Some(6) => "center",
        _ => "stretch",
    };
    Some(Setting::Wallpaper { path, position })
}

/// The task manager's pinned programs, in order, from the first one that has any.
fn launchers(applets: &KConfig) -> Vec<Launcher> {
    let list = applets.groups().find_map(|(g, _)| {
        let [c, id, a, aid, conf, general] = g else {
            return None;
        };
        if c != "Containments" || a != "Applets" || conf != "Configuration" || general != "General"
        {
            return None;
        }
        let plugin = applets.get(&[c, id, a, aid], "plugin")?;
        if !plugin.starts_with("org.kde.plasma.")
            || !plugin.ends_with("tasks") && !plugin.ends_with("taskmanager")
        {
            return None;
        }
        applets
            .get(&[c, id, a, aid, conf, general], "launchers")
            .filter(|l| !l.is_empty())
    });
    let Some(list) = list else {
        return Vec::new();
    };
    list.split(',')
        .filter_map(|l| {
            let l = l.trim();
            if let Some(id) = l.strip_prefix("applications:") {
                return Some(Launcher::Id(id.to_string()));
            }
            if let Some(p) = l.strip_prefix("file://") {
                let name = Path::new(p).file_name()?.to_str()?;
                return name
                    .ends_with(".desktop")
                    .then(|| Launcher::Id(name.to_string()));
            }
            match l {
                "preferred://browser" => Some(Launcher::Default("x-scheme-handler/http")),
                "preferred://filemanager" => Some(Launcher::Default("inode/directory")),
                _ => None,
            }
        })
        .collect()
}

/// The digital clock's 12 or 24 hours, when set apart from the locale's.
fn clock(applets: &KConfig) -> Option<&'static str> {
    applets.groups().find_map(|(g, entries)| {
        let [c, _, a, _, conf, appearance] = g else {
            return None;
        };
        if c != "Containments"
            || a != "Applets"
            || conf != "Configuration"
            || appearance != "Appearance"
        {
            return None;
        }
        let v = entries.iter().find(|(k, _)| k == "use24hFormat")?;
        match v.1.trim() {
            "0" => Some("12-hour"),
            "2" => Some("24-hour"),
            _ => None,
        }
    })
}

/// A libinput device group's entries in kcminputrc (`[Libinput][vendor][product][name]`), the
/// first touchpad's and the first mouse's.
fn devices(input: &KConfig) -> Vec<(Device, &[(String, String)])> {
    let mut out: Vec<(Device, &[(String, String)])> = Vec::new();
    for (g, entries) in input.groups() {
        let [l, _, _, name] = g else {
            continue;
        };
        if l != "Libinput" {
            continue;
        }
        let pad = name.to_lowercase().contains("touchpad")
            || entries.iter().any(|(k, _)| k == "TapToClick");
        let dev = if pad { Device::Touchpad } else { Device::Mouse };
        if !out.iter().any(|(d, _)| *d == dev) {
            out.push((dev, entries));
        }
    }
    out
}

/// The settings Plasma's files under `config` hold that Reprise reads, in the sheet's order.
/// `home` stands in for `$HOME` in a `[$e]` value.
pub(super) fn settings(config: &Path, home: &str) -> Vec<Setting> {
    let read = |name: &str| KConfig::read(&config.join(name), home).unwrap_or_default();
    let globals = read("kdeglobals");
    let input = read("kcminputrc");
    let xkb = read("kxkbrc");
    let kwin = read("kwinrc");
    let fonts = read("kcmfonts");
    let applets = read("plasma-org.kde.plasma.desktop-appletsrc");
    let mut out = Vec::new();

    let scheme = globals.get(&["General"], "ColorScheme");
    let look = globals.get(&["KDE"], "LookAndFeelPackage");
    let dark = scheme.or(look).map(|s| s.to_lowercase().contains("dark"));
    if let Some(w) = wallpaper(&applets, dark == Some(true)) {
        out.push(w);
    }
    if let Some(d) = dark {
        out.push(Setting::Dark(d));
    }

    if flag(xkb.get(&["Layout"], "Use")) == Some(true) {
        if let Some(l) = xkb.get(&["Layout"], "LayoutList").filter(|l| !l.is_empty()) {
            let variant = xkb.get(&["Layout"], "VariantList").unwrap_or("");
            out.push(Setting::Layouts {
                layout: l.to_string(),
                variant: variant.to_string(),
            });
        }
        if let Some(m) = xkb.get(&["Layout"], "Model").filter(|m| !m.is_empty()) {
            out.push(Setting::KbModel(m.to_string()));
        }
        match xkb.get(&["Layout"], "SwitchMode") {
            Some("Global") => out.push(Setting::LayoutPerWindow(false)),
            Some("Window" | "WinClass") => out.push(Setting::LayoutPerWindow(true)),
            _ => {}
        }
    }
    if flag(xkb.get(&["Layout"], "ResetOldOptions")) == Some(true) {
        let opts = xkb.get(&["Layout"], "Options").unwrap_or("");
        out.push(Setting::KbOptions(opts.to_string()));
    }
    if input.get(&["Keyboard"], "KeyRepeat") == Some("nothing") {
        out.push(other("Key repeat", "Off", super::NO_SETTING));
    }
    if let Some(r) = num(input.get(&["Keyboard"], "RepeatRate")) {
        out.push(Setting::RepeatRate(r.round() as i64));
    }
    if let Some(d) = num(input.get(&["Keyboard"], "RepeatDelay")) {
        out.push(Setting::RepeatDelay(d.round() as i64));
    }

    for (dev, entries) in devices(&input) {
        let get = |key: &str| {
            entries
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.as_str())
        };
        if let Some(v) = num(get("PointerAcceleration")) {
            out.push(Setting::Speed(dev, super::tenths(v)));
        }
        match get("PointerAccelerationProfile").map(str::trim) {
            Some("1") => out.push(Setting::Accel(dev, false)),
            Some("2") => out.push(Setting::Accel(dev, true)),
            _ => {}
        }
        if let Some(b) = flag(get("NaturalScroll")) {
            out.push(Setting::Natural(dev, b));
        }
        if flag(get("LeftHanded")) == Some(true) {
            out.push(other("Left-handed buttons", "On", super::NO_SETTING));
        }
        if dev == Device::Mouse {
            continue;
        }
        if let Some(b) = flag(get("TapToClick")) {
            out.push(Setting::TapToClick(b));
        }
        match get("ScrollMethod").map(str::trim) {
            Some("1") => out.push(Setting::TwoFingerScroll(true)),
            Some("2") => out.push(Setting::TwoFingerScroll(false)),
            _ => {}
        }
        if let Some(b) = flag(get("LmrTapButtonMap")) {
            out.push(Setting::TwoFingerTapRight(!b));
        }
        if let Some(b) = flag(get("DisableWhileTyping")) {
            out.push(Setting::DisableWhileTyping(b));
        }
        match get("ClickMethod").map(str::trim) {
            Some("1") => out.push(Setting::ClickMethod("areas")),
            Some("2") => out.push(Setting::ClickMethod("clickfinger")),
            _ => {}
        }
        if let Some(b) = flag(get("Enabled")) {
            out.push(Setting::TouchpadEnabled(b));
        }
        if let Some(b) = flag(get("DisableEventsOnExternalMouse")) {
            out.push(Setting::DisableWithMouse(b));
        }
    }
    if let Some(ms) = num(globals.get(&["KDE"], "DoubleClickInterval")) {
        out.push(Setting::DoubleClick(ms.round() as i64));
    }

    let dpi = num(fonts.get(&["General"], "forceFontDPIWayland"))
        .or_else(|| num(fonts.get(&["General"], "forceFontDPI")))
        .filter(|d| *d > 0.0);
    if let Some(d) = dpi {
        out.push(Setting::TextSize(super::percent(d / 96.0 * 100.0)));
    }
    if let Some(c) = clock(&applets) {
        out.push(other("Clock", c, super::CLOCK));
    }
    if let Some(t) = input.get(&["Mouse"], "cursorTheme") {
        out.push(Setting::CursorTheme(t.to_string()));
    }
    if let Some(n) = num(input.get(&["Mouse"], "cursorSize")) {
        out.push(Setting::CursorSize(n.round() as i64));
    }
    if let Some(t) = globals.get(&["Icons"], "Theme") {
        out.push(other("Icon theme", t, super::LOOK));
    }
    if let Some(f) = globals.get(&["General"], "font") {
        let family = f.split(',').next().unwrap_or(f);
        out.push(other("Font", family, super::LOOK));
    }

    let night = ["NightColor"];
    if let Some(b) = flag(kwin.get(&night, "Active")) {
        out.push(Setting::NightLight(b));
    }
    if let Some(k) = num(kwin.get(&night, "NightTemperature")) {
        out.push(Setting::NightTemperature(k.round() as i64));
    }
    match kwin.get(&night, "Mode") {
        Some("Automatic" | "Location") => out.push(Setting::NightSchedule(None)),
        Some("Times") => {
            let from = kwin.get(&night, "EveningBeginFixed").and_then(hhmm);
            let to = kwin.get(&night, "MorningBeginFixed").and_then(hhmm);
            let from = from.unwrap_or_else(|| "18:00".into());
            let to = to.unwrap_or_else(|| "06:00".into());
            out.push(Setting::NightSchedule(Some((from, to))));
        }
        Some("Constant") => out.push(other("Night light hours", "Always on", super::NO_SETTING)),
        _ => {}
    }
    if let Some(n) = num(kwin.get(&["Desktops"], "Number")) {
        out.push(Setting::Desktops(n.round() as i64));
    }

    for l in launchers(&applets) {
        out.push(Setting::Favourite(l));
    }
    out
}
