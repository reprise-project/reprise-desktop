//! `config.toml` as the compositor runs it: [`repriseconf::config`] and the keys built from the
//! compositor's own types.

mod keybinds;
mod menu;
mod parse;
pub(crate) use parse::dropped_notice;

#[cfg(test)]
mod tests;

use crate::windowrules::WindowRule;
pub use keybinds::*;
pub use menu::*;
pub use repriseconf::config::*;
use smithay::input::keyboard::XkbConfig;

/// The shared [`repriseconf::config::Config`], reached through `Deref`, and the keys whose
/// types are the compositor's: binds, menus, window rules and tablet pads.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub base: repriseconf::config::Config,
    pub keybinds: Vec<Keybind>,
    pub global_shortcuts: Vec<GlobalShortcut>,
    pub startmenu: Vec<MenuEntry>,
    pub quicklaunch: Vec<QuickLaunchEntry>,
    pub window_rules: Vec<WindowRule>,
    pub tablet_pads: Vec<crate::tabletpad::PadConfig>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            base: repriseconf::config::Config::default(),
            keybinds: repriseconf::keybind::DEFAULTS
                .iter()
                .map(|(keys, action)| {
                    bind(
                        keys,
                        Action::from_parts(action, None).expect("default action"),
                    )
                })
                .collect(),
            global_shortcuts: Vec::new(),
            startmenu: default_startmenu(),
            quicklaunch: default_quicklaunch(),
            window_rules: Vec::new(),
            tablet_pads: Vec::new(),
        }
    }
}

impl std::ops::Deref for Config {
    type Target = repriseconf::config::Config;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl std::ops::DerefMut for Config {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}

pub fn xkb_config(kb: &KeyboardConfig) -> XkbConfig<'_> {
    XkbConfig {
        rules: &kb.rules,
        model: &kb.model,
        layout: &kb.layout,
        variant: &kb.variant,
        options: Some(kb.options.clone()).filter(|o| !o.is_empty()),
    }
}
