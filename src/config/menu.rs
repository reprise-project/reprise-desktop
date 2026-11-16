use super::Action;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuickLaunchEntry {
    pub label: String,
    pub icon: Option<String>,
    pub action: Action,
}

pub(super) fn default_quicklaunch() -> Vec<QuickLaunchEntry> {
    let entry = |label: &str, icon: &str, action: Action| QuickLaunchEntry {
        label: label.into(),
        icon: Some(icon.into()),
        action,
    };
    vec![
        entry(
            "Launch Web Browser",
            "web-browser",
            Action::Spawn("xdg-open https://duckduckgo.com/".into()),
        ),
        entry("Show Desktop", "user-desktop", Action::ShowDesktop),
        entry("Terminal", "utilities-terminal", Action::Terminal),
    ]
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuTarget {
    Action(Action),
    Programs,
    Settings,
    Documents,
    Submenu(Vec<MenuEntry>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuEntry {
    Separator,
    Item {
        label: String,
        icon: Option<String>,
        target: MenuTarget,
        tip: Option<String>,
    },
}

impl MenuEntry {
    pub(crate) fn item(label: &str, icon: &str, target: MenuTarget) -> MenuEntry {
        MenuEntry::Item {
            label: label.into(),
            icon: Some(icon.into()),
            target,
            tip: None,
        }
    }

    pub(crate) fn spawn(label: &str, icon: &str, cmd: &str) -> MenuEntry {
        MenuEntry::item(label, icon, MenuTarget::Action(Action::Spawn(cmd.into())))
    }

    pub(crate) fn tipped(self, tip: &str) -> MenuEntry {
        match self {
            MenuEntry::Item {
                label,
                icon,
                target,
                ..
            } => MenuEntry::Item {
                label,
                icon,
                target,
                tip: Some(tip.into()),
            },
            sep => sep,
        }
    }

    pub fn tip(&self) -> Option<&str> {
        match self {
            MenuEntry::Item { tip, .. } => tip.as_deref(),
            MenuEntry::Separator => None,
        }
    }

    pub fn opens_submenu(&self) -> bool {
        !matches!(
            self,
            MenuEntry::Separator
                | MenuEntry::Item {
                    target: MenuTarget::Action(_),
                    ..
                }
        )
    }
}

/// The default Start menu, [`repriseconf::config::default_startmenu_entries`] converted.
pub(super) fn default_startmenu() -> Vec<MenuEntry> {
    (repriseconf::config::default_startmenu_entries().iter())
        .map(super::parse::menu_entry)
        .collect::<Result<_, _>>()
        .expect("the default Start menu parses")
}
