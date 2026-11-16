//! The Start menu a config without `[[startmenu]]` gets, as the file would spell it; the
//! compositor and reprise-settings both convert it, and `config.example.toml` shows it.

use super::file::FileMenuEntry;

pub const PRINTERS: &str = "reprise-printers";
pub const PRINTERS_TIP: &str =
    "Add, remove, and configure local and network printers, and see what is printing.";
pub const HELP: &str = "xdg-open https://github.com/reprise-project/reprise-desktop";
pub const CONTROL_PANEL_TIP: &str = "Provides options for you to customize the appearance \
     and functionality of your computer, add or remove programs, and set up network connections \
     and user accounts.";

/// The default `[[startmenu]]` entries.
pub fn default_startmenu_entries() -> Vec<FileMenuEntry> {
    let row = |label: &str, icon: &str| FileMenuEntry {
        label: Some(label.into()),
        icon: Some(icon.into()),
        ..FileMenuEntry::default()
    };
    let submenu = |label, icon, kind: &str| FileMenuEntry {
        submenu: Some(kind.into()),
        ..row(label, icon)
    };
    let command = |label, icon, cmd: &str| FileMenuEntry {
        command: Some(cmd.into()),
        ..row(label, icon)
    };
    let action = |label, icon, name: &str| FileMenuEntry {
        action: Some(name.into()),
        ..row(label, icon)
    };
    let folder = |label, icon, items| FileMenuEntry {
        items: Some(items),
        ..row(label, icon)
    };
    let tipped = |e: FileMenuEntry, tip: &str| FileMenuEntry {
        tip: Some(tip.into()),
        ..e
    };
    let separator = || FileMenuEntry {
        separator: Some(true),
        ..FileMenuEntry::default()
    };
    vec![
        submenu("&Programs", "applications-other", "programs"),
        submenu("&Documents", "folder-documents", "documents"),
        folder(
            "&Settings",
            "preferences-system",
            vec![
                tipped(
                    submenu("&Control Panel", "preferences-other", "settings"),
                    CONTROL_PANEL_TIP,
                ),
                separator(),
                tipped(
                    command(
                        "&Network Connections",
                        "network-workgroup",
                        "reprise-network",
                    ),
                    "Displays existing network connections on this computer and helps you \
                     create new ones",
                ),
                tipped(command("&Printers", "printer", PRINTERS), PRINTERS_TIP),
                tipped(
                    command(
                        "&Taskbar and Start Menu",
                        "preferences-desktop",
                        "reprise-settings --page taskbar",
                    ),
                    "Customize the Start Menu and the taskbar, such as the types of items to \
                     be displayed and how they should appear.",
                ),
            ],
        ),
        folder(
            "Sear&ch",
            "system-search",
            vec![
                command(
                    "&For Files or Folders...",
                    "system-search",
                    "reprise-explorer --search \"$HOME\"",
                ),
                command(
                    "On the &Internet...",
                    "web-browser",
                    "xdg-open https://duckduckgo.com/",
                ),
            ],
        ),
        command("&Help and Support", "help-browser", HELP),
        action("&Run...", "system-run", "run"),
        separator(),
        action("&Log Off {user}...", "gnome-session-logout", "logoff"),
        action("T&urn Off Computer...", "system-shutdown", "turnoff"),
    ]
}
