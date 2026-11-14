//! Import from GNOME and Plasma: their settings read, never written, as the changes Settings' model
//! makes; the picked ones go into `config.toml` and the Quick Launch folder.

mod gnome;
mod plasma;

use crate::model::{self, CLICK_METHODS, Choices, HOT_CORNERS, POSITIONS, Settings};
use crate::{quickpage, welcome};
use reprisefs::apps::AppEntry;
use reprisefs::quicklaunch as ql;
use reprisekit::controls::importsheet::{Line, Source};
use reprisekit::controls::welcome::TEXT_SIZES;
use reprisekit::interface::Interface;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Device {
    Mouse,
    Touchpad,
}

/// A pinned program: a desktop entry, or the default program for a type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Launcher {
    Id(String),
    Default(&'static str),
}

/// One setting found on the other desktop, in Reprise's terms.
#[derive(Clone, Debug, PartialEq)]
pub enum Setting {
    /// A picture's path and one of [`POSITIONS`]' values.
    Wallpaper {
        path: String,
        position: &'static str,
    },
    Dark(bool),
    /// xkb's comma-separated lists.
    Layouts {
        layout: String,
        variant: String,
    },
    KbModel(String),
    KbOptions(String),
    LayoutPerWindow(bool),
    RepeatRate(i64),
    RepeatDelay(i64),
    /// Tenths, -10 to 10.
    Speed(Device, i64),
    Accel(Device, bool),
    Natural(Device, bool),
    TapToClick(bool),
    TwoFingerScroll(bool),
    TwoFingerTapRight(bool),
    DisableWhileTyping(bool),
    /// One of [`CLICK_METHODS`]' values.
    ClickMethod(&'static str),
    TouchpadEnabled(bool),
    DisableWithMouse(bool),
    /// Percent, one of [`TEXT_SIZES`].
    TextSize(u32),
    NightLight(bool),
    /// Kelvin.
    NightTemperature(i64),
    /// From and to as "21:00", or `None` for sunset to sunrise.
    NightSchedule(Option<(String, String)>),
    CursorTheme(String),
    CursorSize(i64),
    DoubleClick(i64),
    Desktops(i64),
    /// The top left corner opens the overview, or no corner does.
    HotCorner(bool),
    Favourite(Launcher),
    /// Not found but implied by favourites: the Quick Launch programs lead the task row.
    Pinned(bool),
    /// Found, but Reprise has no setting for it: what it is, its value and why.
    Other {
        what: String,
        value: String,
        why: &'static str,
    },
}

const NO_SETTING: &str = "no setting";
/// `[metrics] double_click_ms`'s largest value.
const CLOCK: &str = "the clock follows the formats locale";
const LOOK: &str = "Reprise's look sets it";
const IME: &str = "set up in the input method";

fn other(what: &str, value: &str, why: &'static str) -> Setting {
    Setting::Other {
        what: what.into(),
        value: value.into(),
        why,
    }
}

/// -1..1 as tenths.
fn tenths(v: f64) -> i64 {
    (v.clamp(-1.0, 1.0) * 10.0).round() as i64
}

/// The text size step nearest `p` percent.
fn percent(p: f64) -> u32 {
    (TEXT_SIZES.iter().copied())
        .min_by_key(|s| (f64::from(*s) - p).abs().round() as i64)
        .unwrap_or(100)
}

fn on(b: bool) -> String {
    if b { "On" } else { "Off" }.into()
}

fn label(choices: &Choices, v: &str) -> String {
    (choices.iter())
        .find(|(name, _)| *name == v)
        .map_or_else(|| v.to_string(), |(_, l)| l.to_string())
}

/// "us,de" with ",nodeadkeys" as "us, de (nodeadkeys)".
fn layouts_text(layout: &str, variant: &str) -> String {
    if layout.is_empty() {
        return "System default".into();
    }
    let mut variants = variant.split(',');
    let parts: Vec<String> = layout
        .split(',')
        .map(|l| match variants.next().filter(|v| !v.is_empty()) {
            Some(v) => format!("{l} ({v})"),
            None => l.to_string(),
        })
        .collect();
    parts.join(", ")
}

fn speed_text(t: i64) -> String {
    format!("{:.1}", t as f64 / 10.0)
}

fn device(d: Device, mouse: &'static str, pad: &'static str) -> &'static str {
    match d {
        Device::Mouse => mouse,
        Device::Touchpad => pad,
    }
}

impl Setting {
    fn what(&self) -> String {
        use Setting::*;
        match self {
            Wallpaper { .. } => "Wallpaper",
            Dark(_) => "Dark style",
            Layouts { .. } => "Keyboard layouts",
            KbModel(_) => "Keyboard model",
            KbOptions(_) => "Keyboard options",
            LayoutPerWindow(_) => "Layout per window",
            RepeatRate(_) => "Key repeat rate",
            RepeatDelay(_) => "Key repeat delay",
            Speed(d, _) => device(*d, "Mouse speed", "Touchpad speed"),
            Accel(d, _) => device(*d, "Mouse acceleration", "Touchpad acceleration"),
            Natural(d, _) => device(*d, "Mouse natural scrolling", "Touchpad natural scrolling"),
            TapToClick(_) => "Tap to click",
            TwoFingerScroll(_) => "Two-finger scrolling",
            TwoFingerTapRight(_) => "Two-finger tap right-clicks",
            DisableWhileTyping(_) => "Touchpad off while typing",
            ClickMethod(_) => "Touchpad click method",
            TouchpadEnabled(_) => "Touchpad",
            DisableWithMouse(_) => "Touchpad off with a mouse",
            TextSize(_) => "Text size",
            NightLight(_) => "Night light",
            NightTemperature(_) => "Night light warmth",
            NightSchedule(_) => "Night light hours",
            CursorTheme(_) => "Pointer theme",
            CursorSize(_) => "Pointer size",
            DoubleClick(_) => "Double-click time",
            Desktops(_) => "Virtual desktops",
            HotCorner(_) => "Hot corner",
            Favourite(_) => "Quick Launch",
            Pinned(_) => "Quick Launch as task buttons",
            Other { what, .. } => return what.clone(),
        }
        .into()
    }

    /// Lay the setting over `s`. Favourites and greyed rows touch nothing.
    fn apply(&self, s: &mut Settings) {
        use Setting::*;
        match self {
            Wallpaper { path, position } => {
                s.wallpaper = path.clone();
                s.wallpaper_position = position.to_string();
            }
            Dark(d) => welcome::set_dark(s, *d),
            Layouts { layout, variant } => {
                s.kb_layout = layout.clone();
                s.kb_variant = variant.clone();
            }
            KbModel(m) => s.kb_model = m.clone(),
            KbOptions(o) => s.kb_options = o.clone(),
            LayoutPerWindow(b) => s.layout_per_window = *b,
            RepeatRate(r) => s.repeat_rate = (*r).clamp(0, i32::MAX.into()),
            RepeatDelay(d) => s.repeat_delay = (*d).clamp(0, i32::MAX.into()),
            Speed(Device::Mouse, v) => s.mouse_speed = *v,
            Speed(Device::Touchpad, v) => s.touchpad_speed = *v,
            Accel(Device::Mouse, b) => s.mouse_accel = *b,
            Accel(Device::Touchpad, b) => s.touchpad_accel = *b,
            Natural(Device::Mouse, b) => s.mouse_natural = *b,
            Natural(Device::Touchpad, b) => s.touchpad_natural = *b,
            TapToClick(b) => s.tap_to_click = *b,
            TwoFingerScroll(b) => s.two_finger_scroll = *b,
            TwoFingerTapRight(b) => s.two_finger_tap_right_click = *b,
            DisableWhileTyping(b) => s.disable_while_typing = *b,
            ClickMethod(m) => s.click_method = m.to_string(),
            TouchpadEnabled(b) => s.touchpad_enabled = *b,
            DisableWithMouse(b) => s.disable_with_mouse = *b,
            TextSize(p) => s.text_size = (*p).into(),
            NightLight(b) => s.nl_enabled = *b,
            NightTemperature(k) => s.nl_temperature = (*k).clamp(1000, 6500),
            NightSchedule(None) => s.nl_schedule = "sun".into(),
            NightSchedule(Some((from, to))) => {
                s.nl_schedule = "fixed".into();
                s.nl_from = from.clone();
                s.nl_to = to.clone();
            }
            CursorTheme(t) => s.cursor_theme = t.clone(),
            CursorSize(n) => {
                s.cursor_size = (*n).clamp(0, repriseconf::config::CURSOR_SIZE_MAX.into());
            }
            DoubleClick(ms) => {
                s.double_click_ms = (*ms).clamp(0, repriseconf::config::METRIC_MAX.into())
            }
            Desktops(n) => s.desktops = (*n).clamp(1, model::DESKTOPS_MAX),
            HotCorner(b) => s.hot_corner = if *b { "top-left" } else { "none" }.into(),
            Pinned(b) => s.taskbar_pinned = *b,
            Favourite(_) | Other { .. } => {}
        }
    }

    /// What `s` says for this setting, as the sheet shows it.
    fn shown(&self, s: &Settings) -> String {
        use Setting::*;
        match self {
            Wallpaper { .. } if s.wallpaper.is_empty() => "None".into(),
            // A built-in picture always fills the screen.
            Wallpaper { .. } if s.wallpaper.starts_with("reprise:") => s.wallpaper.clone(),
            Wallpaper { .. } => format!(
                "{} ({})",
                s.wallpaper,
                label(POSITIONS, &s.wallpaper_position)
            ),
            Dark(_) => on(welcome::dark(s)),
            Layouts { .. } => layouts_text(&s.kb_layout, &s.kb_variant),
            KbModel(_) if s.kb_model.is_empty() => "System default".into(),
            KbModel(_) => s.kb_model.clone(),
            KbOptions(_) if s.kb_options.is_empty() => "None".into(),
            KbOptions(_) => s.kb_options.clone(),
            LayoutPerWindow(_) => on(s.layout_per_window),
            RepeatRate(_) => format!("{} a second", s.repeat_rate),
            RepeatDelay(_) => format!("{} ms", s.repeat_delay),
            Speed(Device::Mouse, _) => speed_text(s.mouse_speed),
            Speed(Device::Touchpad, _) => speed_text(s.touchpad_speed),
            Accel(Device::Mouse, _) => on(s.mouse_accel),
            Accel(Device::Touchpad, _) => on(s.touchpad_accel),
            Natural(Device::Mouse, _) => on(s.mouse_natural),
            Natural(Device::Touchpad, _) => on(s.touchpad_natural),
            TapToClick(_) => on(s.tap_to_click),
            TwoFingerScroll(_) => on(s.two_finger_scroll),
            TwoFingerTapRight(_) => on(s.two_finger_tap_right_click),
            DisableWhileTyping(_) => on(s.disable_while_typing),
            ClickMethod(_) => label(CLICK_METHODS, &s.click_method),
            TouchpadEnabled(_) => on(s.touchpad_enabled),
            DisableWithMouse(_) => on(s.disable_with_mouse),
            TextSize(_) => format!("{}%", s.text_size),
            NightLight(_) => on(s.nl_enabled),
            NightTemperature(_) => format!("{} K", s.nl_temperature),
            NightSchedule(_) if s.nl_schedule == "sun" => "Sunset to sunrise".into(),
            NightSchedule(_) => format!("{} to {}", s.nl_from, s.nl_to),
            CursorTheme(_) if s.cursor_theme.is_empty() => "The session's".into(),
            CursorTheme(_) => s.cursor_theme.clone(),
            CursorSize(_) if s.cursor_size == 0 => "The session's".into(),
            CursorSize(_) => format!("{} px", s.cursor_size),
            DoubleClick(_) => format!("{} ms", s.double_click_ms),
            Desktops(_) => s.desktops.to_string(),
            HotCorner(_) => label(HOT_CORNERS, &s.hot_corner),
            Pinned(_) => on(s.taskbar_pinned),
            Favourite(_) | Other { .. } => String::new(),
        }
    }
}

/// What the import looks up on this machine.
pub trait System {
    /// Desktop entry `id`, installed and shown, and its file.
    fn app(&self, id: &str) -> Option<(AppEntry, PathBuf)>;
    /// The default program for `mime`, and its file.
    fn default_app(&self, mime: &str) -> Option<(AppEntry, PathBuf)>;
    fn is_file(&self, path: &Path) -> bool;
    /// The Quick Launch folder's file names.
    fn quick_launch(&self) -> Vec<String>;
}

/// This session's programs, files and Quick Launch folder.
pub struct Live {
    apps: Vec<AppEntry>,
    quick: Option<PathBuf>,
}

impl Live {
    pub fn new() -> Live {
        Live {
            apps: reprisefs::apps::scan(&reprisefs::apps::default_dirs()),
            quick: ql::dir(),
        }
    }
}

impl Default for Live {
    fn default() -> Live {
        Live::new()
    }
}

impl System for Live {
    fn app(&self, id: &str) -> Option<(AppEntry, PathBuf)> {
        let a = self.apps.iter().find(|a| a.id == id && a.shown)?;
        Some((a.clone(), reprisefs::apps::find_file(id)?))
    }
    fn default_app(&self, mime: &str) -> Option<(AppEntry, PathBuf)> {
        let a = reprisefs::props::default_app_entry(mime, &self.apps)?;
        Some((a.clone(), reprisefs::apps::find_file(&a.id)?))
    }
    fn is_file(&self, path: &Path) -> bool {
        path.is_file()
    }
    fn quick_launch(&self) -> Vec<String> {
        self.quick.as_deref().map(ql::files).unwrap_or_default()
    }
}

/// What a checked row writes.
#[derive(Clone, Debug, PartialEq)]
pub enum Write {
    Config(Setting),
    /// A program and its desktop entry's file.
    QuickLaunch(AppEntry, PathBuf),
}

/// A desktop's settings as the sheet lists them, each change with what it writes.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub name: &'static str,
    /// The rows: changes first, then greyed.
    pub lines: Vec<Line>,
    /// Parallel to the changes among `lines`.
    pub writes: Vec<Write>,
}

impl Plan {
    pub fn source(&self) -> Source {
        Source {
            name: self.name.into(),
            lines: self.lines.clone(),
        }
    }
}

const PICTURES: [&str; 5] = ["jpg", "jpeg", "png", "bmp", "dib"];

/// Whether the wallpaper decodes pictures of `path`'s kind.
fn shows(path: &str) -> bool {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    ext.is_some_and(|e| PICTURES.contains(&e.as_str()))
}

/// Why a found wallpaper can't be taken, if it can't.
fn picture_refused(path: &str, sys: &dyn System) -> Option<&'static str> {
    if !shows(path) {
        return Some("Reprise shows JPEG, PNG and BMP pictures");
    }
    (!sys.is_file(Path::new(path))).then_some("the file is missing")
}

/// The rows `found` makes against `config` (config.toml's text): each setting that would change
/// something, in order, then the ones Reprise can't take, greyed with why.
pub fn plan(name: &'static str, found: &[Setting], config: &str, sys: &dyn System) -> Plan {
    let s = model::load(config);
    let mut changes: Vec<(Line, Write)> = Vec::new();
    let mut greyed: Vec<Line> = Vec::new();
    let grey = |what: String, why: &str, value: String| Line {
        what,
        now: why.into(),
        new: value,
        greyed: true,
    };
    let quick = sys.quick_launch();
    let mut favourites = 0;
    let pinned = Setting::Pinned(true);
    let found = found.iter().chain(
        found
            .iter()
            .any(|f| matches!(f, Setting::Favourite(_)))
            .then_some(&pinned),
    );
    for f in found {
        let what = f.what();
        match f {
            Setting::Other { value, why, .. } => greyed.push(grey(what, why, value.clone())),
            Setting::Favourite(l) => {
                let app = match l {
                    Launcher::Id(id) => sys.app(id),
                    Launcher::Default(mime) => sys.default_app(mime),
                };
                let Some((app, file)) = app else {
                    let value = match l {
                        Launcher::Id(id) => id.trim_end_matches(".desktop").to_string(),
                        Launcher::Default(mime) => format!("the default program for {mime}"),
                    };
                    greyed.push(grey(what, "not installed", value));
                    continue;
                };
                let taken = quick.contains(&app.id)
                    || changes
                        .iter()
                        .any(|(_, w)| matches!(w, Write::QuickLaunch(a, _) if a.id == app.id));
                if taken {
                    continue;
                }
                favourites += 1;
                let line = Line {
                    what,
                    now: String::new(),
                    new: app.name.clone(),
                    greyed: false,
                };
                changes.push((line, Write::QuickLaunch(app, file)));
            }
            Setting::Pinned(_) if favourites == 0 => {}
            f => {
                let why = match f {
                    Setting::Wallpaper { path, .. } => picture_refused(path, sys),
                    Setting::Dark(_) if crate::appearancepage::interface(&s) == Interface::Blue => {
                        Some("Blue comes in light colours only")
                    }
                    _ => None,
                };
                let mut t = s.clone();
                f.apply(&mut t);
                if let Some(why) = why {
                    let value = match f {
                        Setting::Wallpaper { path, .. } => path.clone(),
                        Setting::Dark(d) => on(*d),
                        _ => f.shown(&t),
                    };
                    greyed.push(grey(what, why, value));
                    continue;
                }
                if t == s {
                    continue;
                }
                let line = Line {
                    what,
                    now: f.shown(&s),
                    new: f.shown(&t),
                    greyed: false,
                };
                changes.push((line, Write::Config(f.clone())));
            }
        }
    }
    let (mut lines, writes): (Vec<Line>, Vec<Write>) = changes.into_iter().unzip();
    lines.extend(greyed);
    Plan {
        name,
        lines,
        writes,
    }
}

/// Write `picked` (rows of a [`Plan`]): config settings into the file at `config` through Settings'
/// model, as Apply would, programs into the Quick Launch folder `quick` after its own. Every
/// program's entry is read first, so one that can't be read writes nothing at all.
pub fn write(config: &Path, quick: Option<&Path>, picked: &[&Write]) -> Result<(), String> {
    let settings: Vec<&Setting> = (picked.iter())
        .filter_map(|w| match w {
            Write::Config(s) => Some(s),
            Write::QuickLaunch(..) => None,
        })
        .collect();
    let apps: Vec<(&AppEntry, String)> = (picked.iter())
        .filter_map(|w| match w {
            Write::QuickLaunch(a, file) => Some((a, file)),
            Write::Config(_) => None,
        })
        .map(|(a, file)| {
            let text = reprisefs::apps::read_desktop_file(file)
                .map_err(|e| format!("{}: {e}", file.display()))?;
            Ok((a, text))
        })
        .collect::<Result<_, String>>()?;
    let dir = match apps.is_empty() {
        true => None,
        false => Some(quick.ok_or("there is no Quick Launch folder")?),
    };
    if !settings.is_empty() {
        let written = repriseconf::edit(config, |text| {
            let base = model::load(text);
            let mut want = base.clone();
            for s in &settings {
                s.apply(&mut want);
            }
            model::save(text, &want, &base)
        });
        written.map_err(|e| e.to_string())??;
    }
    let Some(dir) = dir else {
        return Ok(());
    };
    let mut order: Vec<String> = quickpage::load(dir).into_iter().map(|l| l.file).collect();
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    // The program's own entry, under its own name, so the taskbar matches its windows.
    for (a, text) in apps {
        let name = ql::unique_name(dir, &a.id);
        std::fs::write(dir.join(&name), text).map_err(|e| e.to_string())?;
        order.push(name);
    }
    ql::write_order(dir, &order).map_err(|e| e.to_string())
}

/// `dconf dump /`'s output, `None` without dconf.
fn dconf_dump() -> Option<String> {
    let out = std::process::Command::new("dconf")
        .args(["dump", "/"])
        .stderr(std::process::Stdio::null())
        .output();
    match out {
        Ok(o) if o.status.success() => String::from_utf8(o.stdout).ok(),
        Ok(o) => {
            log::warn!("dconf dump / exited with {}", o.status);
            None
        }
        Err(e) => {
            log::info!("no GNOME settings read: dconf: {e}");
            None
        }
    }
}

fn modified(path: &Path) -> SystemTime {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH)
}

/// The settings GNOME and Plasma left in this account, the desktop used last first; one that left
/// none is not listed.
pub fn find() -> Vec<(&'static str, Vec<Setting>)> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return Vec::new();
    };
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"));
    let mut found = Vec::new();
    if let Some(dump) = dconf_dump() {
        let s = gnome::settings(&dump);
        found.push((modified(&config.join("dconf/user")), "GNOME", s));
    }
    let s = plasma::settings(&config, &home.to_string_lossy());
    found.push((modified(&config.join("kdeglobals")), "Plasma", s));
    found.retain(|(_, _, s)| !s.is_empty());
    found.sort_by_key(|(when, ..)| std::cmp::Reverse(*when));
    found.into_iter().map(|(_, n, s)| (n, s)).collect()
}

#[cfg(test)]
mod tests;
