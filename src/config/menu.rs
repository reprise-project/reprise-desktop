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

pub(crate) const PRINTERS: &str = "reprise-printers";
pub(crate) const PRINTERS_TIP: &str =
    "Add, remove, and configure local and network printers, and see what is printing.";
pub(crate) const HELP: &str = "xdg-open https://github.com/reprise-project/reprise-desktop";
pub(crate) const CONTROL_PANEL_TIP: &str = "Provides options for you to customize the appearance \
     and functionality of your computer, add or remove programs, and set up network connections \
     and user accounts.";

/// The default Start menu. Kept in sync with `default_menu` in
/// `crates/apps/settings/src/startmenu.rs`; both are held to `config.example.toml`'s block.
pub(super) fn default_startmenu() -> Vec<MenuEntry> {
    use MenuTarget as T;
    vec![
        MenuEntry::item("&Programs", "applications-other", T::Programs),
        MenuEntry::item("&Documents", "folder-documents", T::Documents),
        MenuEntry::item(
            "&Settings",
            "preferences-system",
            T::Submenu(vec![
                MenuEntry::item("&Control Panel", "preferences-other", T::Settings)
                    .tipped(CONTROL_PANEL_TIP),
                MenuEntry::Separator,
                MenuEntry::spawn(
                    "&Network Connections",
                    "network-workgroup",
                    "nm-connection-editor",
                )
                .tipped(
                    "Displays existing network connections on this computer and helps you \
                     create new ones",
                ),
                MenuEntry::spawn("&Printers", "printer", PRINTERS).tipped(PRINTERS_TIP),
                MenuEntry::spawn(
                    "&Taskbar and Start Menu",
                    "preferences-desktop",
                    "reprise-settings --page taskbar",
                )
                .tipped(
                    "Customize the Start Menu and the taskbar, such as the types of items to \
                     be displayed and how they should appear.",
                ),
            ]),
        ),
        MenuEntry::item(
            "Sear&ch",
            "system-search",
            T::Submenu(vec![
                MenuEntry::spawn(
                    "&For Files or Folders...",
                    "system-search",
                    "reprise-explorer \"$HOME\"",
                ),
                MenuEntry::spawn(
                    "On the &Internet...",
                    "web-browser",
                    "xdg-open https://duckduckgo.com/",
                ),
            ]),
        ),
        MenuEntry::spawn("&Help and Support", "help-browser", HELP),
        MenuEntry::item("&Run...", "system-run", T::Action(Action::Run)),
        MenuEntry::Separator,
        MenuEntry::item(
            "&Log Off {user}...",
            "gnome-session-logout",
            T::Action(Action::LogOff),
        ),
        MenuEntry::item(
            "T&urn Off Computer...",
            "system-shutdown",
            T::Action(Action::TurnOff),
        ),
    ]
}
