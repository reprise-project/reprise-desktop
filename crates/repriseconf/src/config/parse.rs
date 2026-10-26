//! Reading `config.toml` into a validated [`Config`](super::Config).

use super::file::{AutoFlag, FileAppearance, FileConfig};
use super::{
    CURSOR_SIZE_MAX, Config, EFFECTS_MS_MAX, EffectsConfig, FontSmoothing, HotCorner,
    IdleOverrides, MAX_POSITION, MenuAnimation, OutputConfig, OutputMode, PREVIEW_FPS_MAX,
    PointerConfig, PowerAction, SNAP_DISTANCE_MAX, SWITCH_MS_MAX, StartMenuStyle, SwitchStyle,
    SwitcherConfig, TaskbarButtons, TaskbarConfig, Transform, WORKSPACES_MAX, quarter_scale,
    switcher_size,
};
use reprisekit::interface::Interface;
use reprisekit::theme::{Color, FontConfig};
use std::path::{Path, PathBuf};

fn apply_pointer(
    cfg: &mut PointerConfig,
    section: &str,
    speed: Option<f64>,
    acceleration: Option<bool>,
    natural_scroll: Option<bool>,
) -> Result<(), String> {
    if let Some(v) = speed {
        if !(-1.0..=1.0).contains(&v) {
            return Err(format!("{section}.speed must be between -1 and 1"));
        }
        cfg.speed = v;
    }
    if let Some(v) = acceleration {
        cfg.acceleration = v;
    }
    if let Some(v) = natural_scroll {
        cfg.natural_scroll = v;
    }
    Ok(())
}

/// Largest `[metrics]` value, pixels or milliseconds: everything past it
/// is a typo, and the consumers do their arithmetic unclamped.
const METRIC_MAX: i32 = 10_000;

pub(super) fn home_path(s: &str, home: Option<&std::ffi::OsStr>) -> Result<PathBuf, String> {
    let path = match (s.strip_prefix("~/"), home) {
        (Some(rest), Some(home)) => Path::new(home).join(rest),
        (Some(_), None) => return Err(format!("{s:?} starts with ~/ and HOME is not set")),
        (None, _) => PathBuf::from(s),
    };
    if path.is_absolute() {
        Ok(path)
    } else {
        Err(format!("{s:?} must be an absolute path or start with ~/"))
    }
}

pub(super) fn wallpaper_image(
    s: &str,
    home: Option<&std::ffi::OsStr>,
) -> Result<Option<PathBuf>, String> {
    if s.is_empty() {
        return Ok(None);
    }
    if s.starts_with("~/") {
        return home_path(s, home)
            .map(Some)
            .map_err(|e| format!("wallpaper.image {e}"));
    }
    Ok(Some(PathBuf::from(s)))
}

/// A glass-tinted built-in resolves to a concrete variant, so a new glass colour
/// or mode is a new path and the wallpaper redraws.
fn builtin_path(
    b: reprisekit::wallpapers::Builtin,
    glass: &reprisekit::glass::Settings,
) -> PathBuf {
    PathBuf::from(reprisekit::interfaces::resolve_wallpaper(b, glass).uri())
}

/// `text` read by `build`, with every key that fails on its own left out: a table's
/// failing keys alone (`taskbar.position`), any other top-level key whole. Returns what
/// `build` made and the dropped keys; Err only when `text` is not TOML.
pub fn lenient<T>(
    text: &str,
    build: impl Fn(FileConfig) -> Result<T, String>,
) -> Result<(T, Vec<(String, String)>), String> {
    let mut table: toml::Table = toml::from_str(text).map_err(|e| e.to_string())?;
    let from_table =
        |t: toml::Table| build(t.try_into().map_err(|e: toml::de::Error| e.to_string())?);
    let alone = |key: &str, value: toml::Value| toml::Table::from_iter([(key.to_string(), value)]);
    let mut dropped = Vec::new();
    for (key, value) in table.iter_mut() {
        let Err(e) = from_table(alone(key, value.clone())) else {
            continue;
        };
        if let toml::Value::Table(inner) = value {
            let bad: Vec<(String, String)> = (inner.iter())
                .filter_map(|(name, v)| {
                    let one = toml::Value::Table(alone(name, v.clone()));
                    from_table(alone(key, one)).err().map(|e| (name.clone(), e))
                })
                .collect();
            let mut kept = inner.clone();
            for (name, _) in &bad {
                kept.remove(name);
            }
            // Keys that fail only together (`night_light.from` = `to`) drop the table whole.
            if !bad.is_empty() && from_table(alone(key, toml::Value::Table(kept.clone()))).is_ok() {
                for (name, e) in bad {
                    dropped.push((format!("{key}.{name}"), e.trim_end().to_string()));
                }
                *inner = kept;
                continue;
            }
        }
        dropped.push((key.clone(), e.trim_end().to_string()));
    }
    table.retain(|key, _| !dropped.iter().any(|(k, _)| k == key));
    Ok((from_table(table)?, dropped))
}

impl Config {
    /// The whole file or an error.
    pub fn parse(text: &str) -> Result<Config, String> {
        Config::from_file(toml::from_str(text).map_err(|e| e.to_string())?)
    }

    /// `text` with every top-level key that fails on its own left at its default; see [`lenient`].
    pub fn parse_lenient(text: &str) -> Result<(Config, Vec<(String, String)>), String> {
        lenient(text, Config::from_file)
    }

    /// The defaults of a file naming `interface` and nothing else.
    pub fn default_for(interface: Interface) -> Config {
        Config::from_file(FileConfig {
            appearance: Some(FileAppearance {
                interface: Some(interface.as_str().into()),
                color_scheme: None,
            }),
            ..FileConfig::default()
        })
        .expect("an interface alone is a valid file")
    }

    /// `file` validated. The keys whose types are the compositor's (`keybinds`,
    /// `global_shortcuts`, `startmenu`, `quicklaunch`, `window_rules`, `tablet_pads`) are
    /// left to it.
    pub fn from_file(file: FileConfig) -> Result<Config, String> {
        let mut cfg = Config::default();
        if let Some(t) = file.terminal {
            cfg.terminal = t;
        }
        if let Some(v) = file.script_terminal {
            cfg.script_terminal = reprisefs::program::ScriptEnd::parse(&v)
                .ok_or_else(|| format!("script_terminal = {v:?} is not stay or close"))?;
        }
        if let Some(v) = file.script_open {
            cfg.script_open = reprisefs::program::ScriptOpen::parse(&v)
                .ok_or_else(|| format!("script_open = {v:?} is not run or terminal"))?;
        }
        if let Some(v) = file.confirm_scripts {
            cfg.confirm_scripts = v;
        }
        cfg.task_manager = file.task_manager;
        if file.renderer.is_some() {
            cfg.retired.push("renderer".into());
        }
        let appearance = file.appearance.unwrap_or_default();
        if let Some(s) = appearance.interface {
            match Interface::parse(&s) {
                Some(i) => cfg.interface = i,
                None => log::warn!("[appearance] interface = {s:?} is unknown; using classic"),
            }
        }
        cfg.color_scheme = appearance.color_scheme;
        if cfg.interface == Interface::Blue
            && let Some(name) = cfg.color_scheme.as_deref()
        {
            match reprisekit::blue::Variant::from_name(name) {
                Some(v) => cfg.blue_variant = v,
                None => {
                    log::warn!(
                        "[appearance] color_scheme = {name:?} is not a Blue scheme; using Blue"
                    )
                }
            }
        }
        if let Some(g) = file.glass {
            if let Some(name) = g.color {
                cfg.glass.color = reprisekit::glass::color_named(&name).ok_or_else(|| {
                    format!("[glass] color = {name:?} is neither a colour name nor #RRGGBB")
                })?;
            }
            if let Some(i) = g.intensity {
                if i > 100 {
                    return Err(format!("[glass] intensity must be 0 to 100, got {i}"));
                }
                cfg.glass.intensity = i;
            }
            if let Some(t) = g.transparency {
                cfg.glass.transparency = t;
            }
            if let Some(b) = g.blur {
                let max = reprisekit::glass::BLUR_MAX;
                if !(1..=max).contains(&b) {
                    return Err(format!("[glass] blur must be 1 to {max}, got {b}"));
                }
                cfg.glass.blur = b;
            }
            if let Some(on) = g.streaks {
                cfg.glass.streaks = on;
            }
            if let Some(f) = g.fast_drag {
                cfg.glass.fast_drag = f;
            }
            if let Some(f) = g.blur_frames {
                cfg.glass.blur_frames = f;
            }
            if let Some(r) = g.blur_rate {
                cfg.glass.blur_rate = r;
            }
            if let Some(o) = g.orb_glow {
                if o > 100 {
                    return Err(format!("[glass] orb_glow must be 0 to 100, got {o}"));
                }
                cfg.glass.orb_glow = o;
            }
            if let Some(m) = g.mode {
                cfg.glass.mode = reprisekit::glass::Mode::parse(&m)
                    .ok_or_else(|| format!("[glass] mode = {m:?} is not light or dark"))?;
            }
            if let Some(m) = g.material {
                cfg.glass.material = reprisekit::glass::Material::parse(&m).ok_or_else(|| {
                    format!("[glass] material = {m:?} is not frosted, smoked or molten")
                })?;
            }
            if let Some(f) = g.frame {
                cfg.glass.frame = reprisekit::glass::Frame::parse(&f)
                    .ok_or_else(|| format!("[glass] frame = {f:?} is not rounded or square"))?;
            }
            if let Some(n) = g.notification_style {
                cfg.glass.notification =
                    reprisekit::glass::NotifyStyle::parse(&n).ok_or_else(|| {
                        format!(
                            "[glass] notification_style = {n:?} is not lozenge, halo or droplet"
                        )
                    })?;
            }
        }
        cfg.taskbar = TaskbarConfig::default_for(cfg.interface);
        cfg.startmenu_style = StartMenuStyle::default_for(cfg.interface);
        if let Some(s) = &file.startmenu_style {
            cfg.startmenu_style = StartMenuStyle::parse(s)
                .ok_or_else(|| format!("startmenu_style = {s:?} is not classic or glass"))?;
        }
        cfg.switcher = SwitcherConfig::default_for(cfg.interface);
        cfg.window_shake = cfg.interface == Interface::Glass;
        cfg.effects = EffectsConfig::default_for(cfg.interface);
        if let Some(t) = file.taskbar {
            if let Some(p) = t.position {
                cfg.taskbar.position =
                    reprisekit::taskbar::Edge::from_name(&p).ok_or_else(|| {
                        format!("[taskbar] position = {p:?} is not bottom, top, left or right")
                    })?;
            }
            if let Some(b) = t.buttons {
                cfg.taskbar.buttons = TaskbarButtons::parse(&b).ok_or_else(|| {
                    format!("[taskbar] buttons = {b:?} is not combine, when_full, never or group")
                })?;
            }
            let flags = [
                (t.pinned, &mut cfg.taskbar.pinned),
                (t.small_icons, &mut cfg.taskbar.small_icons),
                (t.show_desktop_button, &mut cfg.taskbar.show_desktop_button),
                (t.peek, &mut cfg.taskbar.peek),
                (t.tray_overflow, &mut cfg.taskbar.tray_overflow),
                (t.auto_hide, &mut cfg.taskbar.auto_hide),
                (t.reveal_on_edge, &mut cfg.taskbar.reveal_on_edge),
                (t.reveal_on_keys, &mut cfg.taskbar.reveal_on_keys),
                (t.reveal_on_attention, &mut cfg.taskbar.reveal_on_attention),
            ];
            for (v, at) in flags {
                if let Some(v) = v {
                    *at = v;
                }
            }
            if let Some(f) = t.attention_flash {
                use super::appearance::ATTENTION_FLASH_MAX;
                cfg.taskbar.attention_flash = u32::try_from(f)
                    .ok()
                    .filter(|f| *f <= ATTENTION_FLASH_MAX)
                    .ok_or_else(|| {
                        format!(
                            "[taskbar] attention_flash must be between 0 and {ATTENTION_FLASH_MAX} seconds, got {f}"
                        )
                    })?;
            }
            if let Some(w) = t.side_width {
                cfg.taskbar.side_width =
                    i32::try_from(w).ok().filter(|w| *w >= 0).ok_or_else(|| {
                        format!(
                            "[taskbar] side_width must be between 0 and {}, got {w}",
                            i32::MAX
                        )
                    })?;
            }
        }
        cfg.theme = reprisekit::interfaces::theme_with(cfg.interface, cfg.blue_variant, &cfg.glass);
        // Classic and Blue already run small icons at their usual height. A
        // `[metrics] taskbar_height` below still wins over the small bar.
        if cfg.interface == Interface::Glass && cfg.taskbar.small_icons {
            reprisekit::interfaces::small_bar(&mut cfg.theme, &cfg.glass);
        }
        if cfg.interface == Interface::Classic
            && let Some(name) = cfg.color_scheme.as_deref()
        {
            match reprisekit::schemes::scheme(name) {
                Some(colors) => cfg.theme.colors = colors,
                None => log::warn!("[appearance] color_scheme = {name:?} is not a Classic scheme"),
            }
        }
        for (name, value) in file.colors.unwrap_or_default() {
            let c =
                Color::parse(&value).ok_or_else(|| format!("bad color {value:?} for {name}"))?;
            if !cfg.theme.colors.set(&name, c) {
                return Err(format!("unknown color name {name:?}"));
            }
        }
        for (name, value) in file.metrics.unwrap_or_default() {
            if !(0..=METRIC_MAX).contains(&value) {
                return Err(format!(
                    "metrics.{name} must be between 0 and {METRIC_MAX}, got {value}"
                ));
            }
            if !cfg.theme.metrics.set(&name, value) {
                return Err(format!("unknown metric name {name:?}"));
            }
        }
        // The tray centres an icon in a well 6 px inside the bar.
        let min_taskbar = reprisekit::icon::ICON_SIZE + 6;
        if cfg.theme.metrics.taskbar_height < min_taskbar {
            return Err(format!(
                "metrics.taskbar_height must be >= {min_taskbar}, got {}",
                cfg.theme.metrics.taskbar_height
            ));
        }
        if let Some(f) = file.fonts {
            if let Some(fam) = f.family {
                cfg.theme.fonts.family = fam;
            }
            if let Some(s) = f.size_px {
                let (min, max) = (FontConfig::SIZE_PX_MIN, FontConfig::SIZE_PX_MAX);
                if !(min..=max).contains(&s) {
                    return Err(format!(
                        "fonts.size_px must be between {min} and {max}, got {s}"
                    ));
                }
                cfg.theme.fonts.size_px = s;
            }
            if let Some(a) = f.antialias {
                cfg.theme.fonts.antialias = a;
            }
            cfg.theme.fonts.shell_dlg = f.shell_dlg.filter(|s| !s.trim().is_empty());
        }
        if let Some(b) = file.startmenu_banner {
            cfg.startmenu_banner = b;
        }
        if let Some(v) = file.desktop_icons {
            cfg.desktop_icons = v;
        }
        if let Some(v) = file.desktop_extensions {
            cfg.desktop_extensions = reprisefs::names::Extensions::parse(&v).ok_or_else(|| {
                format!("desktop_extensions = {v:?} is not hide, hide_launchers or show")
            })?;
        }
        if let Some(v) = file.start_icon {
            cfg.start_icon = v;
        }
        if let Some(v) = file.system_icons {
            cfg.system_icons = crate::system_icons::current_list(v.iter().map(String::as_str));
        }
        cfg.wallpaper.lock_screen = cfg.interface != Interface::Classic;
        let mut image_set = false;
        if let Some(w) = file.wallpaper {
            if let Some(v) = w.lock_screen {
                cfg.wallpaper.lock_screen = v;
            }
            image_set = w.image.is_some();
            cfg.wallpaper.image = match w.image {
                Some(s) => match reprisekit::wallpapers::Builtin::from_uri(&s) {
                    Some(b) => Some(builtin_path(
                        b.map_err(|e| format!("wallpaper.image {e}"))?,
                        &cfg.glass,
                    )),
                    None => wallpaper_image(&s, std::env::var_os("HOME").as_deref())?,
                },
                None => None,
            };
            if let Some(p) = w.position {
                cfg.wallpaper.position =
                    reprisekit::wallpaper::Position::parse(&p).ok_or_else(|| {
                        format!("wallpaper.position {p:?} is not center, tile or stretch")
                    })?;
            }
            cfg.wallpaper.slideshow = w
                .slideshow
                .map(|s| wallpaper_image(&s, std::env::var_os("HOME").as_deref()))
                .transpose()
                .map_err(|e| e.replace("wallpaper.image", "wallpaper.slideshow"))?
                .flatten();
            if let Some(v) = w.interval {
                if !(1..=1440).contains(&v) {
                    return Err("wallpaper.interval must be 1 to 1440 minutes".into());
                }
                cfg.wallpaper.interval = v as u32;
            }
            if let Some(v) = w.shuffle {
                cfg.wallpaper.shuffle = v;
            }
            if let Some(v) = w.water {
                cfg.wallpaper.water = reprisekit::water::Water::parse(&v).ok_or_else(|| {
                    format!("wallpaper.water {v:?} is not off, calm, lively or rain")
                })?;
            }
            if let Some(v) = w.water_rate {
                let rate = match &v {
                    toml::Value::String(s) => reprisekit::water::Rate::parse(s),
                    toml::Value::Integer(n) => reprisekit::water::Rate::parse(&n.to_string()),
                    _ => None,
                };
                cfg.wallpaper.water_rate = rate.ok_or_else(|| {
                    format!("wallpaper.water_rate {v} is not \"refresh\" or 30 to 240")
                })?;
            }
        }
        if !image_set {
            cfg.wallpaper.image = reprisekit::interfaces::default_wallpaper(cfg.interface)
                .map(|b| builtin_path(b, &cfg.glass));
        }
        if cfg
            .wallpaper
            .image
            .as_deref()
            .and_then(|p| reprisekit::wallpapers::Builtin::from_uri(p.to_str()?)?.ok())
            .is_some()
        {
            cfg.wallpaper.position = reprisekit::wallpaper::Position::Stretch;
        }
        if let Some(f) = file.screensaver {
            let ss = &mut cfg.screensaver;
            if let Some(n) = f.name {
                ss.kind = reprisekit::saver::Kind::parse(&n).ok_or_else(|| {
                    format!(
                        "screensaver.name {n:?} is not blank, starfield, mystify, logo or photos"
                    )
                })?;
            }
            let num = |key: &str, v: Option<i64>, (lo, hi): (u32, u32)| match v {
                Some(v) if v < lo as i64 || v > hi as i64 => {
                    Err(format!("screensaver.{key} must be {lo} to {hi}"))
                }
                v => Ok(v.map(|v| v as u32)),
            };
            use reprisekit::saver::Options as O;
            let o = &mut ss.options;
            if let Some(v) = num("speed", f.speed, O::SPEED)? {
                o.speed = v;
            }
            if let Some(v) = num("stars", f.stars, O::STARS)? {
                o.stars = v;
            }
            if let Some(v) = num("shapes", f.shapes, O::SHAPES)? {
                o.shapes = v;
            }
            if let Some(v) = num("photo_seconds", f.photo_seconds, O::PHOTO_SECONDS)? {
                o.photo_seconds = v;
            }
            if let Some(v) = f.cycle_colors {
                o.cycle_colors = v;
            }
            ss.folder = f
                .folder
                .map(|s| wallpaper_image(&s, std::env::var_os("HOME").as_deref()))
                .transpose()
                .map_err(|e| e.replace("wallpaper.image", "screensaver.folder"))?
                .flatten();
        }
        if let Some(i) = file.icons
            && let Some(t) = i.theme
        {
            cfg.icons.theme = t;
        }
        if let Some(t) = file.toolkits {
            if let Some(v) = t.gtk_theme {
                cfg.toolkits.gtk_theme = v;
            }
            if let Some(v) = t.qt_platform_theme {
                cfg.toolkits.qt_platform_theme = v;
            }
            if let Some(v) = t.gtk4_colors {
                cfg.toolkits.gtk4_colors = v;
            }
            if let Some(s) = t.font_smoothing {
                match FontSmoothing::parse(&s) {
                    Some(f) => cfg.toolkits.font_smoothing = f,
                    None => {
                        log::warn!("[toolkits] font_smoothing = {s:?} is unknown; using grayscale")
                    }
                }
            }
        }
        if let Some(c) = file.cursor {
            if let Some(t) = c.theme {
                cfg.cursor.theme = t;
            }
            if let Some(v) = c.size {
                if v > CURSOR_SIZE_MAX {
                    return Err(format!("cursor.size must be 0 to {CURSOR_SIZE_MAX}"));
                }
                cfg.cursor.size = v;
            }
        }
        if let Some(v) = file.adaptive_sync {
            cfg.adaptive_sync = v;
        }
        if let Some(s) = file.window_snapping {
            if let Some(v) = s.enabled {
                cfg.window_snapping.enabled = v;
            }
            if let Some(v) = s.picker {
                cfg.window_snapping.picker = v;
            }
            if let Some(v) = s.bar {
                cfg.window_snapping.bar = v;
            }
            if let Some(specs) = s.layouts {
                if specs.len() > reprisekit::snaplayouts::ZONE_MAX {
                    return Err(format!(
                        "window_snapping: layouts has at most {} entries, got {}",
                        reprisekit::snaplayouts::ZONE_MAX,
                        specs.len()
                    ));
                }
                cfg.window_snapping.layouts = (specs.iter())
                    .map(|s| reprisekit::snaplayouts::ZoneLayout::parse(s))
                    .collect::<Result<_, _>>()
                    .map_err(|e| format!("window_snapping: layouts: {e}"))?;
            }
            if let Some(d) = s.distance {
                cfg.window_snapping.distance = i32::try_from(d)
                    .ok()
                    .filter(|d| (1..=SNAP_DISTANCE_MAX).contains(d))
                    .ok_or_else(|| {
                        format!(
                            "window_snapping: distance must be between 1 and {SNAP_DISTANCE_MAX}, got {d}"
                        )
                    })?;
            }
        }
        let display = file.display.unwrap_or_default();
        if let Some(all) = display.all_gpus {
            cfg.all_gpus = all;
        }
        if let Some(gpu) = display.gpu {
            let path = std::path::PathBuf::from(&gpu);
            if !path.is_absolute() {
                return Err(format!(
                    "display.gpu {gpu:?} is not a path like \"/dev/dri/card1\""
                ));
            }
            cfg.gpu = Some(path);
        }
        if let Some(v) = file.screenshot.and_then(|s| s.save) {
            cfg.screenshot.save = v;
        }
        if let Some(p) = file.previews {
            if let Some(v) = p.taskbar {
                cfg.previews.taskbar = v;
            }
            if let Some(n) = p.fps {
                if !(1..=PREVIEW_FPS_MAX as i64).contains(&n) {
                    return Err(format!(
                        "previews.fps must be between 1 and {PREVIEW_FPS_MAX}, got {n}"
                    ));
                }
                cfg.previews.fps = n as u32;
            }
        }
        if let Some(s) = file.switcher {
            if let Some(v) = s.style {
                cfg.switcher.style = v.parse()?;
            }
            if let Some(v) = s.size {
                cfg.switcher.size = switcher_size(&v).ok_or_else(|| {
                    format!(
                        "switcher.size {v:?} is not \"small\", \"medium\", \"large\" or \"huge\""
                    )
                })?;
            }
            let flags = [
                (s.preview, &mut cfg.switcher.preview),
                (s.desktop, &mut cfg.switcher.desktop),
                (s.peek, &mut cfg.switcher.peek),
                (s.flip, &mut cfg.switcher.flip),
            ];
            for (v, at) in flags {
                if let Some(v) = v {
                    *at = v;
                }
            }
        }
        if let Some(v) = file.overview.and_then(|o| o.hot_corner) {
            cfg.hot_corner = HotCorner::from_name(&v).ok_or_else(|| {
                format!(
                    "overview.hot_corner {v:?} is not \"none\", \"top-left\", \"top-right\", \"bottom-left\" or \"bottom-right\""
                )
            })?;
        }
        if let Some(g) = file.gestures {
            if let Some(v) = g.enabled {
                cfg.gestures.enabled = v;
            }
            if let Some(n) = g.fingers {
                if n != 3 && n != 4 {
                    return Err(format!("gestures.fingers must be 3 or 4, got {n}"));
                }
                cfg.gestures.fingers = n as u32;
            }
            let ranged =
                |key: &str, v: Option<i64>, lo: u32, hi: u32| -> Result<Option<u32>, String> {
                    v.map(|n| {
                        u32::try_from(n)
                            .ok()
                            .filter(|n| (lo..=hi).contains(n))
                            .ok_or_else(|| format!("gestures.{key} must be {lo} to {hi}, got {n}"))
                    })
                    .transpose()
                };
            if let Some(n) = ranged("distance", g.distance, 100, 2000)? {
                cfg.gestures.distance = n;
            }
            if let Some(n) = ranged("commit_at", g.commit_at, 5, 95)? {
                cfg.gestures.commit_at = n;
            }
            if let Some(n) = ranged("flick_speed", g.flick_speed, 50, 5000)? {
                cfg.gestures.flick_speed = n;
            }
        }
        if let Some(c) = file.clipboard {
            if let Some(v) = c.history {
                cfg.clipboard.history = v;
            }
            if let Some(v) = c.terminal_apps {
                cfg.clipboard.terminal_apps = v;
            }
            if let Some(v) = c.trusted_apps {
                cfg.clipboard.trusted_apps = v;
            }
        }
        if let Some(p) = file.privacy {
            if let Some(v) = p.recent_files {
                cfg.recent_files = v;
            }
            if let Some(v) = p.location {
                cfg.location = v;
            }
            if let Some(v) = p.capture_border {
                cfg.capture_border = v;
            }
        }
        if let Some(d) = file.debug {
            if let Some(v) = d.overlay {
                cfg.debug.overlay = v;
            }
            if let Some(v) = d.overlay_in_captures {
                cfg.debug.overlay_in_captures = v;
            }
        }
        if let Some(v) = file.launcher.and_then(|l| l.search_files) {
            cfg.launcher_files = v;
        }
        if let Some(n) = file.night_light {
            n.apply(&mut cfg.night_light)?;
        }
        if let Some(w) = file.workspaces {
            if let Some(n) = w.count {
                cfg.workspaces = usize::try_from(n)
                    .ok()
                    .filter(|n| (1..=WORKSPACES_MAX).contains(n))
                    .ok_or_else(|| {
                        format!("workspaces: count must be between 1 and {WORKSPACES_MAX}, got {n}")
                    })?;
            }
            if let Some(a) = w.animation {
                cfg.workspace_animation.style = SwitchStyle::from_name(&a).ok_or_else(|| {
                    format!(
                        "workspaces.animation {a:?} is not one of {}",
                        SwitchStyle::ALL
                            .iter()
                            .map(|s| format!("{:?}", s.name()))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                })?;
            }
            if let Some(p) = w.pager {
                use reprisekit::taskbar::PagerMark;
                cfg.pager_mark = PagerMark::from_name(&p).ok_or_else(|| {
                    format!(
                        "workspaces.pager {p:?} is not one of {}",
                        PagerMark::ALL
                            .iter()
                            .map(|m| format!("{:?}", m.name()))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                })?;
            }
            if let Some(ms) = w.animation_ms {
                cfg.workspace_animation.ms = u32::try_from(ms)
                    .ok()
                    .filter(|ms| *ms <= SWITCH_MS_MAX)
                    .ok_or_else(|| {
                        format!(
                            "workspaces: animation_ms must be between 0 and {SWITCH_MS_MAX}, got {ms}"
                        )
                    })?;
            }
        }
        if let Some(e) = file.effects {
            let ms = |v: Option<i64>, key: &str, at: &mut u32| -> Result<(), String> {
                if let Some(v) = v {
                    *at = u32::try_from(v)
                        .ok()
                        .filter(|v| *v <= EFFECTS_MS_MAX)
                        .ok_or_else(|| {
                            format!(
                                "effects: {key} must be between 0 and {EFFECTS_MS_MAX}, got {v}"
                            )
                        })?;
                }
                Ok(())
            };
            let fx = &mut cfg.effects;
            if let Some(v) = e.window_animation {
                fx.window_animation = v;
            }
            if let Some(v) = e.hover_fade {
                fx.hover_fade = v;
            }
            if let Some(v) = e.taskbar_slide {
                fx.taskbar_slide = v;
            }
            if let Some(v) = e.session_fade {
                fx.session_fade = v;
            }
            if let Some(name) = &e.menu_animation {
                fx.menu_animation = MenuAnimation::from_name(name).ok_or_else(|| {
                    format!(
                        "effects: menu_animation must be \"none\", \"fade\" or \"slide\", got {name:?}"
                    )
                })?;
            }
            ms(e.window_open_ms, "window_open_ms", &mut fx.window_open_ms)?;
            ms(
                e.window_close_ms,
                "window_close_ms",
                &mut fx.window_close_ms,
            )?;
            ms(e.hover_in_ms, "hover_in_ms", &mut fx.hover_in_ms)?;
            ms(e.hover_out_ms, "hover_out_ms", &mut fx.hover_out_ms)?;
            ms(
                e.taskbar_hide_ms,
                "taskbar_hide_ms",
                &mut fx.taskbar_hide_ms,
            )?;
            ms(
                e.session_fade_in_ms,
                "session_fade_in_ms",
                &mut fx.session_fade_in_ms,
            )?;
            ms(
                e.session_fade_out_ms,
                "session_fade_out_ms",
                &mut fx.session_fade_out_ms,
            )?;
            if let Some(s) = e.window_scale {
                if !(0.8..=1.0).contains(&s) {
                    return Err(format!(
                        "effects: window_scale must be between 0.8 and 1.0, got {s}"
                    ));
                }
                fx.window_scale = s;
            }
        }
        if let Some(a) = file.accessibility {
            a.apply(&mut cfg.accessibility)?;
        }
        if let Some(w) = file.windows {
            if let Some(v) = w.remember {
                cfg.remember_windows = v;
            }
            if let Some(v) = w.tabs {
                cfg.window_tabs = v;
            }
            match w.shake {
                Some(AutoFlag::Flag(v)) => cfg.window_shake = v,
                Some(AutoFlag::Word(w)) if w == "auto" => {}
                Some(AutoFlag::Word(w)) => {
                    return Err(format!(
                        "[windows] shake = {w:?} is not true, false or \"auto\""
                    ));
                }
                None => {}
            }
        }
        if let Some(s) = file.session {
            if let Some(v) = s.restore {
                cfg.session.restore = v;
            }
            if let Some(v) = s.exclude {
                cfg.session.exclude = v;
            }
            if let Some(v) = s.end_processes {
                cfg.session.end_processes = v;
            }
        }
        if let Some(outs) = file.outputs {
            cfg.outputs = outs
                .into_iter()
                .map(|o| {
                    let asked = o.scale.unwrap_or(1.0);
                    if !(1.0..=4.0).contains(&asked) {
                        return Err(format!(
                            "outputs.{}: scale must be between 1 and 4, got {asked}",
                            o.name
                        ));
                    }
                    let scale = quarter_scale(asked as f32);
                    if f64::from(scale) != asked {
                        log::warn!(
                            "outputs.{}: scale {asked} is not a multiple of 0.25; using {scale}",
                            o.name
                        );
                    }
                    let mode = o
                        .mode
                        .as_deref()
                        .map(OutputMode::parse)
                        .transpose()
                        .map_err(|e| format!("outputs.{}: {e}", o.name))?;
                    let transform = match o.transform.as_deref() {
                        None => Transform::Normal,
                        Some(s) => Transform::parse(s).ok_or_else(|| {
                            format!(
                                "outputs.{}: transform {s:?} is not one of {}",
                                o.name,
                                Transform::ALL.map(Transform::name).join(", ")
                            )
                        })?,
                    };
                    if o.color_profile.is_some() {
                        cfg.retired
                            .push(format!("outputs.{}.color_profile", o.name));
                    }
                    if o.hdr.is_some() {
                        cfg.retired.push(format!("outputs.{}.hdr", o.name));
                    }
                    let position = o
                        .position
                        .map(|[x, y]| {
                            let max = i64::from(MAX_POSITION);
                            if (-max..=max).contains(&x) && (-max..=max).contains(&y) {
                                Ok((x as i32, y as i32))
                            } else {
                                Err(format!(
                                    "outputs.{}: position must be within {max} of 0, got [{x}, {y}]",
                                    o.name
                                ))
                            }
                        })
                        .transpose()?;
                    Ok(OutputConfig {
                        name: o.name,
                        scale,
                        mode,
                        adaptive_sync: o.adaptive_sync,
                        transform,
                        position,
                        primary: o.primary.unwrap_or(false),
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
        }
        if let Some(k) = file.keyboard {
            if let Some(v) = k.rules {
                cfg.keyboard.rules = v;
            }
            if let Some(v) = k.model {
                cfg.keyboard.model = v;
            }
            if let Some(v) = k.layout {
                cfg.keyboard.layout = v;
            }
            if let Some(v) = k.variant {
                cfg.keyboard.variant = v;
            }
            if let Some(v) = k.options {
                cfg.keyboard.options = v;
            }
            if let Some(v) = k.repeat_rate {
                if v < 0 {
                    return Err("keyboard.repeat_rate must be >= 0".into());
                }
                cfg.keyboard.repeat_rate = v;
            }
            if let Some(v) = k.repeat_delay {
                if v < 0 {
                    return Err("keyboard.repeat_delay must be >= 0".into());
                }
                cfg.keyboard.repeat_delay = v;
            }
        }
        if let Some(m) = file.mouse {
            apply_pointer(
                &mut cfg.mouse,
                "mouse",
                m.speed,
                m.acceleration,
                m.natural_scroll,
            )?;
        }
        if let Some(t) = file.touchpad {
            apply_pointer(
                &mut cfg.touchpad.pointer,
                "touchpad",
                t.speed,
                t.acceleration,
                t.natural_scroll,
            )?;
            if let Some(v) = t.tap_to_click {
                cfg.touchpad.tap_to_click = v;
            }
            if let Some(v) = t.two_finger_scroll {
                cfg.touchpad.two_finger_scroll = v;
            }
            if let Some(v) = t.two_finger_tap_right_click {
                cfg.touchpad.two_finger_tap_right_click = v;
            }
            if let Some(v) = t.disable_while_typing {
                cfg.touchpad.disable_while_typing = v;
            }
        }
        if let Some(t) = file.touchscreen {
            cfg.touchscreen.output = t.output.filter(|o| !o.is_empty());
        }
        if let Some(p) = file.power {
            if let Some(v) = p.standby {
                cfg.power.standby = v;
            }
            if let Some(v) = p.poweroff {
                cfg.power.poweroff = v;
            }
            if let Some(v) = p.reboot {
                cfg.power.reboot = v;
            }
            if let Some(v) = p.hibernate {
                cfg.power.hibernate = v;
            }
            let action = |name: &str, v: Option<String>| -> Result<Option<PowerAction>, String> {
                v.map(|s| PowerAction::parse(&s).map_err(|e| format!("power.{name}: {e}")))
                    .transpose()
            };
            if let Some(a) = action("lid_close", p.lid_close)? {
                cfg.power.lid_close = a;
            }
            if let Some(a) = action("power_button", p.power_button)? {
                cfg.power.power_button = a;
            }
            if let Some(a) = action("sleep_button", p.sleep_button)? {
                cfg.power.sleep_button = a;
            }
            if let Some(v) = p.lock_on_sleep {
                cfg.power.lock_on_sleep = v;
            }
            if let Some(v) = p.locker {
                cfg.power.locker = v;
            }
            if let Some(a) = action("critical_action", p.critical_action)? {
                if matches!(
                    a,
                    PowerAction::Ask | PowerAction::ScreenOff | PowerAction::Lock
                ) {
                    return Err(format!(
                        "power.critical_action cannot be {:?}: nothing, standby, hibernate or turnoff",
                        a.name()
                    ));
                }
                cfg.power.critical_action = a;
            }
            let percent = |name: &str, v: Option<i64>| -> Result<Option<u32>, String> {
                match v {
                    None => Ok(None),
                    Some(s) if (0..=100).contains(&s) => Ok(Some(s as u32)),
                    Some(s) => Err(format!("power.{name} must be 0..=100, got {s}")),
                }
            };
            if let Some(v) = percent("battery_low", p.battery_low)? {
                cfg.power.battery_low = v;
            }
            if let Some(v) = percent("battery_critical", p.battery_critical)? {
                cfg.power.battery_critical = v;
            }
            if let Some(v) = p.always_show_icon {
                cfg.power.always_show_icon = v;
            }
        }
        if let Some(i) = file.idle {
            let seconds = |name: &str, v: Option<i64>| -> Result<Option<u32>, String> {
                v.map(|s| {
                    u32::try_from(s)
                        .map_err(|_| format!("idle.{name} must be 0..={}, got {s}", u32::MAX))
                })
                .transpose()
            };
            if let Some(v) = seconds("lock_after", i.lock_after)? {
                cfg.idle.lock_after = v;
            }
            if let Some(v) = seconds("screen_off_after", i.screen_off_after)? {
                cfg.idle.screen_off_after = v;
            }
            if let Some(v) = seconds("blank_after", i.blank_after)? {
                cfg.idle.blank_after = v;
            }
            if let Some(v) = seconds("suspend_after", i.suspend_after)? {
                cfg.idle.suspend_after = v;
            }
            if let Some(v) = seconds("lock_fade", i.lock_fade)? {
                cfg.idle.lock_fade = v;
            }
            if let Some(b) = i.battery {
                cfg.idle_battery = IdleOverrides {
                    lock_after: seconds("battery.lock_after", b.lock_after)?,
                    screen_off_after: seconds("battery.screen_off_after", b.screen_off_after)?,
                    blank_after: seconds("battery.blank_after", b.blank_after)?,
                    suspend_after: seconds("battery.suspend_after", b.suspend_after)?,
                };
            }
        }
        // Last: the text size scales the theme `[metrics]` and `[fonts]` made.
        cfg.theme = cfg.theme.with_text_size(cfg.accessibility.text_size);
        Ok(cfg)
    }
}
