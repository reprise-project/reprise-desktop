//! GNOME's settings from `dconf dump /`: the keys a user changed, each a GVariant in text form.

use super::{Device, Launcher, Setting, other};
use std::collections::HashMap;

/// A GVariant as `dconf dump` prints it, as far as the keys read here need.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Value {
    Str(String),
    Bool(bool),
    Num(f64),
    /// An array or a tuple.
    List(Vec<Value>),
}

impl Value {
    fn str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }
    fn bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }
    fn num(&self) -> Option<f64> {
        match self {
            Value::Num(n) => Some(*n),
            _ => None,
        }
    }
    fn list(&self) -> &[Value] {
        match self {
            Value::List(l) => l,
            _ => &[],
        }
    }
}

/// GVariant text: strings, booleans, numbers (a type word before them skipped), arrays and tuples.
pub(super) fn parse_value(text: &str) -> Option<Value> {
    let mut p = Parser {
        s: text.as_bytes(),
        at: 0,
    };
    let v = p.value()?;
    p.space();
    (p.at == p.s.len()).then_some(v)
}

struct Parser<'a> {
    s: &'a [u8],
    at: usize,
}

const TYPE_WORDS: [&str; 11] = [
    "byte", "int16", "uint16", "int32", "uint32", "int64", "uint64", "double", "boolean", "string",
    "handle",
];

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.at).copied()
    }

    fn space(&mut self) {
        while self.peek().is_some_and(|c| c.is_ascii_whitespace()) {
            self.at += 1;
        }
    }

    fn word(&mut self) -> &str {
        let start = self.at;
        while self
            .peek()
            .is_some_and(|c| c.is_ascii_alphanumeric() || b"_.-+".contains(&c))
        {
            self.at += 1;
        }
        std::str::from_utf8(&self.s[start..self.at]).unwrap_or("")
    }

    fn value(&mut self) -> Option<Value> {
        self.space();
        match self.peek()? {
            b'@' => {
                while self.peek().is_some_and(|c| !c.is_ascii_whitespace()) {
                    self.at += 1;
                }
                self.value()
            }
            b'\'' | b'"' => self.string().map(Value::Str),
            b'[' => self.items(b']'),
            b'(' => self.items(b')'),
            _ => {
                let w = self.word().to_string();
                match w.as_str() {
                    "true" => Some(Value::Bool(true)),
                    "false" => Some(Value::Bool(false)),
                    w if TYPE_WORDS.contains(&w) => self.value(),
                    w => w.parse().ok().map(Value::Num),
                }
            }
        }
    }

    fn items(&mut self, close: u8) -> Option<Value> {
        self.at += 1;
        let mut out = Vec::new();
        loop {
            self.space();
            if self.peek()? == close {
                self.at += 1;
                return Some(Value::List(out));
            }
            out.push(self.value()?);
            self.space();
            match self.peek()? {
                b',' => self.at += 1,
                c if c == close => {}
                _ => return None,
            }
        }
    }

    fn string(&mut self) -> Option<String> {
        let quote = self.peek()?;
        self.at += 1;
        let mut out = Vec::new();
        loop {
            let c = self.peek()?;
            self.at += 1;
            match c {
                b'\\' => {
                    let e = self.peek()?;
                    self.at += 1;
                    match e {
                        b'n' => out.push(b'\n'),
                        b't' => out.push(b'\t'),
                        b'u' => {
                            let hex =
                                std::str::from_utf8(self.s.get(self.at..self.at + 4)?).ok()?;
                            let ch = char::from_u32(u32::from_str_radix(hex, 16).ok()?)?;
                            self.at += 4;
                            out.extend(ch.to_string().as_bytes());
                        }
                        e => out.push(e),
                    }
                }
                c if c == quote => return String::from_utf8(out).ok(),
                c => out.push(c),
            }
        }
    }
}

/// `dconf dump /`'s keys by directory (no leading slash) and name.
pub(super) fn parse_dump(text: &str) -> HashMap<(String, String), Value> {
    let mut out = HashMap::new();
    let mut dir = String::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(d) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            dir = d.trim_matches('/').to_string();
        } else if let Some((k, v)) = line.split_once('=')
            && let Some(v) = parse_value(v)
        {
            out.insert((dir.clone(), k.trim().to_string()), v);
        }
    }
    out
}

const INTERFACE: &str = "org/gnome/desktop/interface";
const MOUSE: &str = "org/gnome/desktop/peripherals/mouse";
const TOUCHPAD: &str = "org/gnome/desktop/peripherals/touchpad";
const KEYBOARD: &str = "org/gnome/desktop/peripherals/keyboard";
const SOURCES: &str = "org/gnome/desktop/input-sources";
const BACKGROUND: &str = "org/gnome/desktop/background";
const COLOR: &str = "org/gnome/settings-daemon/plugins/color";
const WM: &str = "org/gnome/desktop/wm/preferences";

/// A dconf `file://` URI's path, or the text when it is a plain path.
fn uri_path(uri: &str) -> Option<String> {
    let path = match uri.strip_prefix("file://") {
        Some(p) => String::from_utf8(reprisefs::trash::percent_decode(p)).ok()?,
        None if uri.starts_with('/') => uri.to_string(),
        None => return None,
    };
    Some(path)
}

/// "20.5" hours as "20:30".
fn hours(h: f64) -> String {
    let m = (h.rem_euclid(24.0) * 60.0).round() as i64 % (24 * 60);
    format!("{:02}:{:02}", m / 60, m % 60)
}

/// The settings `dump` holds that Reprise reads, in the sheet's order.
pub(super) fn settings(dump: &str) -> Vec<Setting> {
    let keys = parse_dump(dump);
    let get = |dir: &str, key: &str| keys.get(&(dir.to_string(), key.to_string()));
    let mut out = Vec::new();

    let dark = get(INTERFACE, "color-scheme")
        .and_then(Value::str)
        .map(|s| s == "prefer-dark");
    let picture = |key: &str| get(BACKGROUND, key).and_then(Value::str).and_then(uri_path);
    let dark_picture = picture("picture-uri-dark").filter(|p| super::shows(p));
    let picture = match dark {
        Some(true) => dark_picture.or_else(|| picture("picture-uri")),
        _ => picture("picture-uri"),
    };
    let options = get(BACKGROUND, "picture-options").and_then(Value::str);
    if options != Some("none")
        && let Some(path) = picture
    {
        let position = match options {
            Some("centered") => "center",
            Some("wallpaper") => "tile",
            _ => "stretch",
        };
        out.push(Setting::Wallpaper { path, position });
    }
    if let Some(d) = dark {
        out.push(Setting::Dark(d));
    }

    if let Some(sources) = get(SOURCES, "sources") {
        let (mut layouts, mut variants) = (Vec::new(), Vec::new());
        for s in sources.list() {
            match s.list() {
                [Value::Str(kind), Value::Str(id)] if kind == "xkb" => {
                    let (l, v) = id.split_once('+').unwrap_or((id, ""));
                    layouts.push(l.to_string());
                    variants.push(v.to_string());
                }
                [Value::Str(kind), Value::Str(id)] => {
                    out.push(other("Input method", &format!("{kind}: {id}"), super::IME));
                }
                _ => {}
            }
        }
        if !layouts.is_empty() {
            out.push(Setting::Layouts {
                layout: layouts.join(","),
                variant: variants.join(","),
            });
        }
    }
    if let Some(opts) = get(SOURCES, "xkb-options") {
        let opts: Vec<&str> = opts.list().iter().filter_map(Value::str).collect();
        out.push(Setting::KbOptions(opts.join(",")));
    }
    if let Some(b) = get(SOURCES, "per-window").and_then(Value::bool) {
        out.push(Setting::LayoutPerWindow(b));
    }
    if get(KEYBOARD, "repeat").and_then(Value::bool) == Some(false) {
        out.push(other("Key repeat", "Off", super::NO_SETTING));
    }
    if let Some(ms) = get(KEYBOARD, "repeat-interval").and_then(Value::num)
        && ms > 0.0
    {
        out.push(Setting::RepeatRate((1000.0 / ms).round() as i64));
    }
    if let Some(ms) = get(KEYBOARD, "delay").and_then(Value::num) {
        out.push(Setting::RepeatDelay(ms.round() as i64));
    }

    for (dir, dev) in [(MOUSE, Device::Mouse), (TOUCHPAD, Device::Touchpad)] {
        if let Some(v) = get(dir, "speed").and_then(Value::num) {
            out.push(Setting::Speed(dev, super::tenths(v)));
        }
        match get(dir, "accel-profile").and_then(Value::str) {
            Some("flat") => out.push(Setting::Accel(dev, false)),
            Some("adaptive") => out.push(Setting::Accel(dev, true)),
            _ => {}
        }
        if let Some(b) = get(dir, "natural-scroll").and_then(Value::bool) {
            out.push(Setting::Natural(dev, b));
        }
    }
    if get(MOUSE, "left-handed").and_then(Value::bool) == Some(true) {
        out.push(other("Left-handed buttons", "On", super::NO_SETTING));
    }
    if let Some(ms) = get(MOUSE, "double-click").and_then(Value::num) {
        out.push(Setting::DoubleClick(ms.round() as i64));
    }
    let touchpad_bool = |key: &str| get(TOUCHPAD, key).and_then(Value::bool);
    if let Some(b) = touchpad_bool("tap-to-click") {
        out.push(Setting::TapToClick(b));
    }
    if let Some(b) = touchpad_bool("two-finger-scrolling-enabled") {
        out.push(Setting::TwoFingerScroll(b));
    }
    match get(TOUCHPAD, "tap-button-map").and_then(Value::str) {
        Some("lrm") => out.push(Setting::TwoFingerTapRight(true)),
        Some("lmr") => out.push(Setting::TwoFingerTapRight(false)),
        _ => {}
    }
    if let Some(b) = touchpad_bool("disable-while-typing") {
        out.push(Setting::DisableWhileTyping(b));
    }
    match get(TOUCHPAD, "click-method").and_then(Value::str) {
        Some("fingers") => out.push(Setting::ClickMethod("clickfinger")),
        Some("areas") => out.push(Setting::ClickMethod("areas")),
        Some("default") => out.push(Setting::ClickMethod("default")),
        _ => {}
    }
    match get(TOUCHPAD, "send-events").and_then(Value::str) {
        Some("enabled") => {
            out.push(Setting::TouchpadEnabled(true));
            out.push(Setting::DisableWithMouse(false));
        }
        Some("disabled") => out.push(Setting::TouchpadEnabled(false)),
        Some("disabled-on-external-mouse") => {
            out.push(Setting::TouchpadEnabled(true));
            out.push(Setting::DisableWithMouse(true));
        }
        _ => {}
    }

    if let Some(f) = get(INTERFACE, "text-scaling-factor").and_then(Value::num) {
        out.push(Setting::TextSize(super::percent(f * 100.0)));
    }
    if let Some(c) = get(INTERFACE, "clock-format").and_then(Value::str) {
        let shown = if c == "12h" { "12-hour" } else { "24-hour" };
        out.push(other("Clock", shown, super::CLOCK));
    }
    if let Some(t) = get(INTERFACE, "cursor-theme").and_then(Value::str) {
        out.push(Setting::CursorTheme(t.to_string()));
    }
    if let Some(n) = get(INTERFACE, "cursor-size").and_then(Value::num) {
        out.push(Setting::CursorSize(n.round() as i64));
    }
    if let Some(b) = get(INTERFACE, "enable-hot-corners").and_then(Value::bool) {
        out.push(Setting::HotCorner(b));
    }
    for (key, what) in [("gtk-theme", "GTK theme"), ("icon-theme", "Icon theme")] {
        if let Some(t) = get(INTERFACE, key).and_then(Value::str) {
            out.push(other(what, t, super::LOOK));
        }
    }
    if let Some(f) = get(INTERFACE, "font-name").and_then(Value::str) {
        out.push(other("Font", f, super::LOOK));
    }
    if let Some(b) = get(WM, "button-layout").and_then(Value::str) {
        out.push(other("Title bar buttons", b, super::LOOK));
    }

    if let Some(b) = get(COLOR, "night-light-enabled").and_then(Value::bool) {
        out.push(Setting::NightLight(b));
    }
    if let Some(k) = get(COLOR, "night-light-temperature").and_then(Value::num) {
        out.push(Setting::NightTemperature(k.round() as i64));
    }
    let auto = get(COLOR, "night-light-schedule-automatic").and_then(Value::bool);
    let from = get(COLOR, "night-light-schedule-from").and_then(Value::num);
    let to = get(COLOR, "night-light-schedule-to").and_then(Value::num);
    match (auto, from, to) {
        (Some(true), ..) => out.push(Setting::NightSchedule(None)),
        (_, Some(f), Some(t)) => out.push(Setting::NightSchedule(Some((hours(f), hours(t))))),
        _ => {}
    }

    let dynamic = get("org/gnome/mutter", "dynamic-workspaces").and_then(Value::bool);
    if dynamic == Some(false)
        && let Some(n) = get(WM, "num-workspaces").and_then(Value::num)
    {
        out.push(Setting::Desktops(n.round() as i64));
    }

    if let Some(apps) = get("org/gnome/shell", "favorite-apps") {
        for id in apps.list().iter().filter_map(Value::str) {
            out.push(Setting::Favourite(Launcher::Id(id.to_string())));
        }
    }
    out
}
