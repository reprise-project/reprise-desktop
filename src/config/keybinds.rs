use crate::keys::{MOD_ALT, MOD_CTRL, MOD_LOGO, MOD_SHIFT};
use repriseconf::config::file::FileGlobalShortcut;
use repriseconf::keybind::{DESKTOP_CARRY_ITEMS, DESKTOP_ITEMS, TASKBAR_ITEMS};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Spawn(String),
    /// An argument vector run with no shell (a Quick Launch application entry, `reprisefs::apps::launch_argv`),
    /// or the entry's refusal, the desktop entry id it came from and its `Path=` folder.
    Exec {
        argv: Result<Vec<std::ffi::OsString>, reprisefs::apps::ExecRefused>,
        app: String,
        dir: Option<std::path::PathBuf>,
    },
    Terminal,
    /// Open Explorer on the home folder.
    Explorer,
    Close,
    Cycle,
    /// The switcher over the focused window's app's windows alone.
    CycleApp,
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
    /// Task button `n` (from 0) of the row on the pointer's output, as a click on it.
    TaskItem(usize),
    /// Hide or show the focused window's border and caption; it keeps its outer rect.
    ToggleFrame,
    AlwaysOnTop,
    /// Open or close the focused window's mini view.
    MiniView,
    Center,
    Tab(bool),
    Desktop(crate::desktops::Step),
    CarryToDesktop(crate::desktops::Step),
    AllDesktops,
    DoNotDisturb,
    /// Open or close the notification history on the newest notification that can be answered.
    Notifications,
    NightLight,
    Snap(crate::snap::Dir),
    SnapLayouts,
    DisplaySwitcher,
    Zoom(crate::magnifier::ZoomStep),
    ColorFilter,
    ScreenReader,
    Launcher,
    TaskManager,
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
    RecordScreen,
    RecordRegion,
    ColorPicker,
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
    /// Turn the touchpad on (true) or off (false), or flip it (None).
    Touchpad(Option<bool>),
    Airplane,
    /// Turn `[display] auto_rotate` off (locked) or back on.
    RotationLock,
}

impl Action {
    /// The config name of a compositor action (the inverse of `from_parts`);
    /// None for `Spawn` and `Exec`, which are commands, not names.
    pub fn name(&self) -> Option<&'static str> {
        match self {
            Action::TaskItem(n) => TASKBAR_ITEMS.get(*n).copied(),
            Action::Desktop(crate::desktops::Step::To(n)) => DESKTOP_ITEMS.get(*n).copied(),
            Action::CarryToDesktop(crate::desktops::Step::To(n)) => {
                DESKTOP_CARRY_ITEMS.get(*n).copied()
            }
            a => NAMED_ACTIONS.iter().find(|(_, b)| b == a).map(|(n, _)| *n),
        }
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
        if action == "spawn" {
            return Ok(Action::Spawn(
                command.ok_or("spawn needs a command")?.to_string(),
            ));
        }
        if let Some((_, a)) = NAMED_ACTIONS.iter().find(|(n, _)| *n == action) {
            return Ok(a.clone());
        }
        let at = |names: &[&str]| names.iter().position(|n| *n == action);
        if let Some(n) = at(&TASKBAR_ITEMS) {
            Ok(Action::TaskItem(n))
        } else if let Some(n) = at(&DESKTOP_ITEMS) {
            Ok(Action::Desktop(crate::desktops::Step::To(n)))
        } else if let Some(n) = at(&DESKTOP_CARRY_ITEMS) {
            Ok(Action::CarryToDesktop(crate::desktops::Step::To(n)))
        } else {
            Err(format!("unknown action {action:?}"))
        }
    }
}

/// Every action a fixed config name stands for, with the name; the numbered ones take theirs
/// from `TASKBAR_ITEMS`, `DESKTOP_ITEMS` and `DESKTOP_CARRY_ITEMS`.
pub(crate) const NAMED_ACTIONS: &[(&str, Action)] = &[
    ("terminal", Action::Terminal),
    ("explorer", Action::Explorer),
    ("close", Action::Close),
    ("cycle", Action::Cycle),
    ("cycle-app", Action::CycleApp),
    ("flip", Action::Flip),
    ("exit", Action::Exit),
    ("maximize", Action::Maximize),
    ("minimize", Action::Minimize),
    ("lock", Action::Lock),
    ("startmenu", Action::StartMenu),
    ("run", Action::Run),
    ("logoff", Action::LogOff),
    ("turnoff", Action::TurnOff),
    ("windowmenu", Action::WindowMenu),
    ("showdesktop", Action::ShowDesktop),
    ("toggleframe", Action::ToggleFrame),
    ("alwaysontop", Action::AlwaysOnTop),
    ("mini-view", Action::MiniView),
    ("center", Action::Center),
    ("tab-next", Action::Tab(false)),
    ("tab-prev", Action::Tab(true)),
    ("desktop-prev", Action::Desktop(crate::desktops::Step::Prev)),
    ("desktop-next", Action::Desktop(crate::desktops::Step::Next)),
    (
        "desktop-carry-prev",
        Action::CarryToDesktop(crate::desktops::Step::Prev),
    ),
    (
        "desktop-carry-next",
        Action::CarryToDesktop(crate::desktops::Step::Next),
    ),
    ("all-desktops", Action::AllDesktops),
    ("do-not-disturb", Action::DoNotDisturb),
    ("notifications", Action::Notifications),
    ("night-light", Action::NightLight),
    ("snap-left", Action::Snap(crate::snap::Dir::Left)),
    ("snap-right", Action::Snap(crate::snap::Dir::Right)),
    ("snap-up", Action::Snap(crate::snap::Dir::Up)),
    ("snap-down", Action::Snap(crate::snap::Dir::Down)),
    ("snap-layouts", Action::SnapLayouts),
    ("display-switcher", Action::DisplaySwitcher),
    ("zoom-in", Action::Zoom(crate::magnifier::ZoomStep::In)),
    ("zoom-out", Action::Zoom(crate::magnifier::ZoomStep::Out)),
    (
        "zoom-reset",
        Action::Zoom(crate::magnifier::ZoomStep::Reset),
    ),
    ("color-filter", Action::ColorFilter),
    ("screen-reader", Action::ScreenReader),
    ("launcher", Action::Launcher),
    ("task-manager", Action::TaskManager),
    ("overview", Action::Overview),
    ("focus-taskbar", Action::FocusTaskbar),
    ("focus-tray", Action::FocusTray),
    ("clipboard-history", Action::ClipboardHistory),
    ("emoji-picker", Action::EmojiPicker),
    ("next-layout", Action::NextLayout),
    ("screenshot", Action::Screenshot),
    ("screenshot-window", Action::ScreenshotWindow),
    ("screenshot-region", Action::ScreenshotRegion),
    ("screenshot-content", Action::ScreenshotContent),
    ("record-screen", Action::RecordScreen),
    ("record-region", Action::RecordRegion),
    ("color-picker", Action::ColorPicker),
    ("volume-up", Action::VolumeUp),
    ("volume-down", Action::VolumeDown),
    ("mute", Action::Mute),
    ("mic-mute", Action::MicMute),
    ("brightness-up", Action::BrightnessUp),
    ("brightness-down", Action::BrightnessDown),
    ("kbd-brightness-up", Action::KbdBrightnessUp),
    ("kbd-brightness-down", Action::KbdBrightnessDown),
    ("play-pause", Action::PlayPause),
    ("next", Action::Next),
    ("prev", Action::Prev),
    ("stop", Action::Stop),
    ("wifi", Action::Wifi),
    ("touchpad", Action::Touchpad(None)),
    ("touchpad-on", Action::Touchpad(Some(true))),
    ("touchpad-off", Action::Touchpad(Some(false))),
    ("airplane-mode", Action::Airplane),
    ("rotation-lock", Action::RotationLock),
];

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
