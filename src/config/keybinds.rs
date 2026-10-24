use super::file::FileGlobalShortcut;
use crate::keys::{MOD_ALT, MOD_CTRL, MOD_LOGO, MOD_SHIFT};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Spawn(String),
    /// An argument vector run with no shell (a Quick Launch application entry, `reprisefs::apps::launch_argv`),
    /// and the desktop entry id it came from.
    Exec {
        argv: Vec<std::ffi::OsString>,
        app: String,
    },
    Terminal,
    Close,
    Cycle,
    Flip,
    Exit,
    Maximize,
    Minimize,
    Lock,
    StartMenu,
    Run,
    LogOff,
    TurnOff,
    WindowMenu,
    ShowDesktop,
    /// Hide or show the focused window's border and caption; it keeps its outer rect.
    ToggleFrame,
    AlwaysOnTop,
    Center,
    Tab(bool),
    Desktop(crate::desktops::Step),
    CarryToDesktop(crate::desktops::Step),
    AllDesktops,
    DoNotDisturb,
    NightLight,
    Snap(crate::snap::Dir),
    SnapLayouts,
    Zoom(crate::magnifier::ZoomStep),
    ColorFilter,
    Launcher,
    Overview,
    FocusTaskbar,
    FocusTray,
    ClipboardHistory,
    EmojiPicker,
    NextLayout,
    Screenshot,
    ScreenshotWindow,
    ScreenshotRegion,
    ScreenshotContent,
    VolumeUp,
    VolumeDown,
    Mute,
    MicMute,
    BrightnessUp,
    BrightnessDown,
    KbdBrightnessUp,
    KbdBrightnessDown,
    PlayPause,
    Next,
    Prev,
    Stop,
    Wifi,
}

impl Action {
    /// The config name of a compositor action (the inverse of `from_parts`);
    /// None for `Spawn` and `Exec`, which are commands, not names.
    pub fn name(&self) -> Option<&'static str> {
        Some(match self {
            Action::Spawn(_) | Action::Exec { .. } => return None,
            Action::Terminal => "terminal",
            Action::Close => "close",
            Action::Cycle => "cycle",
            Action::Flip => "flip",
            Action::Exit => "exit",
            Action::Maximize => "maximize",
            Action::Minimize => "minimize",
            Action::Lock => "lock",
            Action::StartMenu => "startmenu",
            Action::Run => "run",
            Action::LogOff => "logoff",
            Action::TurnOff => "turnoff",
            Action::WindowMenu => "windowmenu",
            Action::ShowDesktop => "showdesktop",
            Action::ToggleFrame => "toggleframe",
            Action::AlwaysOnTop => "alwaysontop",
            Action::Center => "center",
            Action::Tab(false) => "tab-next",
            Action::Tab(true) => "tab-prev",
            Action::Desktop(crate::desktops::Step::Prev) => "desktop-prev",
            Action::Desktop(crate::desktops::Step::Next) => "desktop-next",
            Action::CarryToDesktop(crate::desktops::Step::Prev) => "desktop-carry-prev",
            Action::CarryToDesktop(crate::desktops::Step::Next) => "desktop-carry-next",
            Action::AllDesktops => "all-desktops",
            Action::DoNotDisturb => "do-not-disturb",
            Action::NightLight => "night-light",
            Action::Snap(crate::snap::Dir::Left) => "snap-left",
            Action::Snap(crate::snap::Dir::Right) => "snap-right",
            Action::Snap(crate::snap::Dir::Up) => "snap-up",
            Action::Snap(crate::snap::Dir::Down) => "snap-down",
            Action::SnapLayouts => "snap-layouts",
            Action::Zoom(crate::magnifier::ZoomStep::In) => "zoom-in",
            Action::Zoom(crate::magnifier::ZoomStep::Out) => "zoom-out",
            Action::Zoom(crate::magnifier::ZoomStep::Reset) => "zoom-reset",
            Action::ColorFilter => "color-filter",
            Action::Launcher => "launcher",
            Action::Overview => "overview",
            Action::FocusTaskbar => "focus-taskbar",
            Action::FocusTray => "focus-tray",
            Action::ClipboardHistory => "clipboard-history",
            Action::EmojiPicker => "emoji-picker",
            Action::NextLayout => "next-layout",
            Action::Screenshot => "screenshot",
            Action::ScreenshotWindow => "screenshot-window",
            Action::ScreenshotRegion => "screenshot-region",
            Action::ScreenshotContent => "screenshot-content",
            Action::VolumeUp => "volume-up",
            Action::VolumeDown => "volume-down",
            Action::Mute => "mute",
            Action::MicMute => "mic-mute",
            Action::BrightnessUp => "brightness-up",
            Action::BrightnessDown => "brightness-down",
            Action::KbdBrightnessUp => "kbd-brightness-up",
            Action::KbdBrightnessDown => "kbd-brightness-down",
            Action::PlayPause => "play-pause",
            Action::Next => "next",
            Action::Prev => "prev",
            Action::Stop => "stop",
            Action::Wifi => "wifi",
        })
    }

    /// What a launcher file for this action holds: the command of a
    /// `Spawn`, else the action's name. None for an `Exec`, which only a
    /// launcher file already holds.
    pub fn launch(&self) -> Option<reprisefs::apps::Launch<'_>> {
        Some(match self {
            Action::Spawn(cmd) => reprisefs::apps::Launch::Exec(cmd),
            Action::Exec { .. } => return None,
            a => reprisefs::apps::Launch::Action(a.name().unwrap_or_default()),
        })
    }

    pub fn from_parts(action: &str, command: Option<&str>) -> Result<Action, String> {
        Ok(match action {
            "spawn" => Action::Spawn(command.ok_or("spawn needs a command")?.to_string()),
            "terminal" => Action::Terminal,
            "close" => Action::Close,
            "cycle" => Action::Cycle,
            "flip" => Action::Flip,
            "exit" => Action::Exit,
            "maximize" => Action::Maximize,
            "minimize" => Action::Minimize,
            "lock" => Action::Lock,
            "startmenu" => Action::StartMenu,
            "run" => Action::Run,
            "logoff" => Action::LogOff,
            "turnoff" => Action::TurnOff,
            "windowmenu" => Action::WindowMenu,
            "showdesktop" => Action::ShowDesktop,
            "toggleframe" => Action::ToggleFrame,
            "alwaysontop" => Action::AlwaysOnTop,
            "center" => Action::Center,
            "tab-next" => Action::Tab(false),
            "tab-prev" => Action::Tab(true),
            "desktop-prev" => Action::Desktop(crate::desktops::Step::Prev),
            "desktop-next" => Action::Desktop(crate::desktops::Step::Next),
            "desktop-carry-prev" => Action::CarryToDesktop(crate::desktops::Step::Prev),
            "desktop-carry-next" => Action::CarryToDesktop(crate::desktops::Step::Next),
            "all-desktops" => Action::AllDesktops,
            "do-not-disturb" => Action::DoNotDisturb,
            "night-light" => Action::NightLight,
            "snap-left" => Action::Snap(crate::snap::Dir::Left),
            "snap-right" => Action::Snap(crate::snap::Dir::Right),
            "snap-up" => Action::Snap(crate::snap::Dir::Up),
            "snap-down" => Action::Snap(crate::snap::Dir::Down),
            "snap-layouts" => Action::SnapLayouts,
            "zoom-in" => Action::Zoom(crate::magnifier::ZoomStep::In),
            "zoom-out" => Action::Zoom(crate::magnifier::ZoomStep::Out),
            "zoom-reset" => Action::Zoom(crate::magnifier::ZoomStep::Reset),
            "color-filter" => Action::ColorFilter,
            "launcher" => Action::Launcher,
            "overview" => Action::Overview,
            "focus-taskbar" => Action::FocusTaskbar,
            "focus-tray" => Action::FocusTray,
            "clipboard-history" => Action::ClipboardHistory,
            "emoji-picker" => Action::EmojiPicker,
            "next-layout" => Action::NextLayout,
            "screenshot" => Action::Screenshot,
            "screenshot-window" => Action::ScreenshotWindow,
            "screenshot-region" => Action::ScreenshotRegion,
            "screenshot-content" => Action::ScreenshotContent,
            "volume-up" => Action::VolumeUp,
            "volume-down" => Action::VolumeDown,
            "mute" => Action::Mute,
            "mic-mute" => Action::MicMute,
            "brightness-up" => Action::BrightnessUp,
            "brightness-down" => Action::BrightnessDown,
            "kbd-brightness-up" => Action::KbdBrightnessUp,
            "kbd-brightness-down" => Action::KbdBrightnessDown,
            "play-pause" => Action::PlayPause,
            "next" => Action::Next,
            "prev" => Action::Prev,
            "stop" => Action::Stop,
            "wifi" => Action::Wifi,
            other => return Err(format!("unknown action {other:?}")),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Keybind {
    pub mods: u32,
    pub keysym: u32,
    pub action: Action,
}

/// One `[[global_shortcuts]]` entry bound through the portal (reprise-portal
/// writes these; Settings edits them). `chord` is None when `keys` is empty,
/// unparsable or modifier-only; the entry stays and takes no key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GlobalShortcut {
    pub app: String,
    pub id: String,
    pub description: String,
    pub keys: String,
    pub chord: Option<(u32, u32)>,
}

impl Keybind {
    /// A modifier-only chord ("Shift+Alt_L") fires on release via `keyroute::ChordState`.
    pub fn is_chord(&self) -> bool {
        crate::keyroute::modifier_key(self.keysym).is_some()
    }
}

/// `[[global_shortcuts]]` as entries. A repeated (app, id) keeps the first and
/// unbindable keys leave the entry unassigned; both are logged, neither fails the file.
pub(super) fn global_shortcuts(list: Vec<FileGlobalShortcut>) -> Vec<GlobalShortcut> {
    let mut out: Vec<GlobalShortcut> = Vec::new();
    for e in list {
        if out.iter().any(|o| o.app == e.app && o.id == e.id) {
            log::warn!(
                "global_shortcuts: {} {} is listed twice; the first counts",
                e.app,
                e.id
            );
            continue;
        }
        let chord = if e.keys.is_empty() {
            None
        } else if repriseconf::keybind::modifier_only(&e.keys) {
            log::warn!(
                "global_shortcuts: {} {}: {:?} is modifier-only",
                e.app,
                e.id,
                e.keys
            );
            None
        } else {
            parse_keybind(&e.keys)
                .inspect_err(|err| log::warn!("global_shortcuts: {} {}: {err}", e.app, e.id))
                .ok()
        };
        out.push(GlobalShortcut {
            app: e.app,
            id: e.id,
            description: e.description,
            keys: e.keys,
            chord,
        });
    }
    out
}

/// "Mod+Mod+key" as (`keys::MOD_*` bits, lowercase keysym).
pub fn parse_keybind(s: &str) -> Result<(u32, u32), String> {
    let mut mods = 0u32;
    let parts: Vec<&str> = s.split('+').map(str::trim).collect();
    let (key, modparts) = parts.split_last().ok_or("empty keybind")?;
    if key.is_empty() {
        return Err(format!("keybind {s:?} has no key"));
    }
    for m in modparts {
        mods |= match m.to_ascii_lowercase().as_str() {
            "super" | "logo" | "mod4" | "win" => MOD_LOGO,
            "ctrl" | "control" => MOD_CTRL,
            "alt" | "mod1" => MOD_ALT,
            "shift" => MOD_SHIFT,
            other => return Err(format!("unknown modifier {other:?} in {s:?}")),
        };
    }
    let sym = crate::keys::from_name(key).ok_or_else(|| format!("unknown key {key:?} in {s:?}"))?;
    Ok((mods, crate::keys::to_lower(sym)))
}

pub(super) fn bind(keys: &str, action: Action) -> Keybind {
    let (mods, keysym) = parse_keybind(keys).expect("default keybind");
    Keybind {
        mods,
        keysym,
        action,
    }
}
