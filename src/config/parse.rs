//! Reading `config.toml` into the compositor's [`Config`]: [`repriseconf::config::Config`]
//! validates the shared keys, this the ones built from the compositor's types.

use super::{
    Action, Config, Keybind, MenuEntry, MenuTarget, QuickLaunchEntry, WORKSPACES_MAX,
    global_shortcuts, parse_keybind,
};
use crate::windowrules::{WindowRule, WindowState};
use repriseconf::config::file::{
    FileConfig, FileKeybind, FileMenuEntry, FileQuickLaunch, FileWindowRule,
};
use std::path::{Path, PathBuf};

const UNREADABLE_NOTICE: &str = "Reprise could not read config.toml and started with the \
                                 defaults. `journalctl --user -t reprise` names the error.";

impl Config {
    pub fn default_path() -> PathBuf {
        repriseconf::config_path()
    }

    /// The whole file or an error: what a live reload and a config write check against.
    pub fn parse(text: &str) -> Result<Config, String> {
        Config::from_file(toml::from_str(text).map_err(|e| e.to_string())?)
    }

    /// `text` with every key that fails on its own left at its default; see
    /// [`repriseconf::config::lenient`].
    pub fn parse_lenient(text: &str) -> Result<(Config, Vec<(String, String)>), String> {
        repriseconf::config::lenient(text, Config::from_file)
    }

    fn from_file(mut file: FileConfig) -> Result<Config, String> {
        let keybinds = file.keybinds.take();
        let globals = file.global_shortcuts.take();
        let startmenu = file.startmenu.take();
        let quicklaunch = file.quicklaunch.take();
        let rules = file.window_rules.take();
        let pads = file.tablet_pads.take();
        let mut cfg = Config {
            base: repriseconf::config::Config::from_file(file)?,
            ..Config::default()
        };
        if let Some(binds) = keybinds {
            for b in &binds {
                parse_keybind(&b.keys)?;
            }
            let layered = repriseconf::keybind::layered(
                binds,
                |b| (&b.keys, &b.action),
                |keys, action| FileKeybind {
                    keys: keys.into(),
                    action: action.into(),
                    command: None,
                },
            );
            cfg.keybinds = layered
                .iter()
                .map(|b| {
                    let (mods, keysym) = parse_keybind(&b.keys)?;
                    Ok(Keybind {
                        mods,
                        keysym,
                        action: Action::from_parts(&b.action, b.command.as_deref())?,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
        }
        if let Some(list) = globals {
            cfg.global_shortcuts = global_shortcuts(list);
        }
        if let Some(entries) = startmenu {
            cfg.startmenu = entries
                .iter()
                .map(menu_entry)
                .collect::<Result<Vec<_>, String>>()?;
        }
        if let Some(entries) = quicklaunch {
            cfg.quicklaunch = entries
                .iter()
                .map(quicklaunch_entry)
                .collect::<Result<Vec<_>, String>>()?;
        }
        if let Some(rules) = rules {
            cfg.window_rules = rules
                .into_iter()
                .enumerate()
                .map(|(i, r)| {
                    let at = format!("window_rules #{} ({:?})", i + 1, r.app_id);
                    window_rule(r, &at)
                })
                .collect::<Result<_, _>>()?;
        }
        if let Some(pads) = pads {
            cfg.tablet_pads = pads
                .into_iter()
                .enumerate()
                .map(|(i, v)| {
                    let at = format!("tablet_pads #{}", i + 1);
                    let pad: crate::tabletpad::FilePad = v
                        .try_into()
                        .map_err(|e: toml::de::Error| format!("{at}: {e}"))?;
                    pad.parse(&at)
                })
                .collect::<Result<_, _>>()?;
        }
        Ok(cfg)
    }

    /// Load for startup. A top-level key that does not parse is dropped to its
    /// default and a non-TOML file yields all defaults. Returns a notice when anything was dropped.
    pub fn load(path: Option<&Path>) -> (Config, Option<String>) {
        let path = path
            .map(Path::to_path_buf)
            .unwrap_or_else(Config::default_path);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                log::info!("no config at {}; using defaults", path.display());
                return (Config::default(), None);
            }
            Err(e) => {
                log::error!("config {}: {e}; using defaults", path.display());
                return (Config::default(), Some(UNREADABLE_NOTICE.to_string()));
            }
        };
        match Config::parse_lenient(&text) {
            Ok((c, dropped)) if dropped.is_empty() => {
                log::info!("loaded config {}", path.display());
                (c, None)
            }
            Ok((c, dropped)) => {
                for (key, e) in &dropped {
                    log::warn!("config {}: {key}: {e}; using its defaults", path.display());
                }
                let keys: Vec<&str> = dropped.iter().map(|(k, _)| k.as_str()).collect();
                let notice = format!(
                    "Reprise ignored {} in config.toml and used the defaults there. \
                     `journalctl --user -t reprise` names the errors.",
                    keys.join(", ")
                );
                (c, Some(notice))
            }
            Err(e) => {
                log::error!("config {}: {e}; using defaults", path.display());
                (Config::default(), Some(UNREADABLE_NOTICE.to_string()))
            }
        }
    }
}

fn quicklaunch_entry(f: &FileQuickLaunch) -> Result<QuickLaunchEntry, String> {
    let action = match (&f.action, &f.command) {
        (Some(a), cmd) => Action::from_parts(a, cmd.as_deref())?,
        (None, Some(cmd)) => Action::Spawn(cmd.clone()),
        (None, None) => {
            return Err(format!(
                "quicklaunch entry {:?} needs a command or action",
                f.label
            ));
        }
    };
    Ok(QuickLaunchEntry {
        label: f.label.clone(),
        icon: f.icon.clone().filter(|s| !s.is_empty()),
        action,
    })
}

fn menu_entry(f: &FileMenuEntry) -> Result<MenuEntry, String> {
    if f.separator == Some(true) {
        return Ok(MenuEntry::Separator);
    }
    let label = f
        .label
        .clone()
        .ok_or("startmenu entry needs a label or separator = true")?;
    let target = match (&f.action, &f.command, &f.submenu, &f.items) {
        (Some(a), cmd, None, None) => MenuTarget::Action(Action::from_parts(a, cmd.as_deref())?),
        (None, Some(cmd), None, None) => MenuTarget::Action(Action::Spawn(cmd.clone())),
        (None, None, Some(kind), None) => match kind.as_str() {
            "programs" => MenuTarget::Programs,
            "settings" => MenuTarget::Settings,
            "documents" => MenuTarget::Documents,
            other => {
                return Err(format!(
                    "startmenu entry {label:?}: unknown submenu {other:?}, expected programs|settings|documents"
                ));
            }
        },
        (None, None, None, Some(items)) => MenuTarget::Submenu(
            items
                .iter()
                .map(menu_entry)
                .collect::<Result<Vec<_>, String>>()?,
        ),
        // A label alone is a folder whose rows were all taken out.
        (None, None, None, None) => MenuTarget::Submenu(Vec::new()),
        _ => {
            return Err(format!(
                "startmenu entry {label:?}: command/action, submenu and items are mutually exclusive"
            ));
        }
    };
    Ok(MenuEntry::Item {
        label,
        icon: f.icon.clone().filter(|s| !s.is_empty()),
        target,
        tip: f.tip.clone().filter(|s| !s.is_empty()),
    })
}

fn window_rule(f: FileWindowRule, at: &str) -> Result<WindowRule, String> {
    if f.app_id.is_empty() {
        return Err(format!("{at}: app_id is empty"));
    }
    let size = match f.size {
        None => None,
        Some([w, h]) if (1..=32767).contains(&w) && (1..=32767).contains(&h) => {
            Some((w as i32, h as i32))
        }
        Some([w, h]) => {
            return Err(format!(
                "{at}: size must be two numbers from 1 to 32767, got [{w}, {h}]"
            ));
        }
    };
    let span = -32768..=32767;
    let position = match f.position {
        None => None,
        Some([x, y]) if span.contains(&x) && span.contains(&y) => Some((x as i32, y as i32)),
        Some([x, y]) => {
            return Err(format!(
                "{at}: position must be two numbers from -32768 to 32767, got [{x}, {y}]"
            ));
        }
    };
    let state = f
        .state
        .map(|s| {
            WindowState::parse(&s).ok_or_else(|| {
                format!(
                    "{at}: state {s:?} is not one of {}",
                    WindowState::ALL.map(WindowState::name).join(", ")
                )
            })
        })
        .transpose()?;
    let desktop = f
        .desktop
        .map(|n| {
            usize::try_from(n)
                .ok()
                .filter(|n| (1..=WORKSPACES_MAX).contains(n))
                .map(|n| n - 1)
                .ok_or_else(|| {
                    format!("{at}: desktop must be between 1 and {WORKSPACES_MAX}, got {n}")
                })
        })
        .transpose()?;
    Ok(WindowRule {
        app_id: f.app_id,
        title: f.title,
        size,
        position,
        output: f.output,
        state,
        desktop,
        always_on_top: f.always_on_top,
        sticky: f.sticky,
        tab_group: f.tab_group,
    })
}
