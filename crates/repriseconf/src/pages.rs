//! The Settings sheet's pages and separate settings programs, with the search
//! words for each. [`GROUPS`] fixes the pages' order.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Appearance,
    Background,
    ScreenSaver,
    Display,
    NightLight,
    FontsAndIcons,
    Effects,
    Taskbar,
    StartMenu,
    Launcher,
    Switcher,
    Desktops,
    Windows,
    Keyboard,
    Mouse,
    Tablet,
    Pointers,
    Accessibility,
    Vision,
    Power,
    Startup,
    DefaultPrograms,
    Advanced,
}

pub const GROUPS: &[(&str, &[Page])] = &[
    (
        "Look",
        &[
            Page::Appearance,
            Page::Background,
            Page::ScreenSaver,
            Page::Display,
            Page::NightLight,
            Page::FontsAndIcons,
            Page::Effects,
        ],
    ),
    (
        "Desktop",
        &[
            Page::Taskbar,
            Page::StartMenu,
            Page::Launcher,
            Page::Switcher,
            Page::Desktops,
            Page::Windows,
        ],
    ),
    (
        "Input",
        &[
            Page::Keyboard,
            Page::Mouse,
            Page::Tablet,
            Page::Pointers,
            Page::Accessibility,
            Page::Vision,
        ],
    ),
    (
        "System",
        &[
            Page::Power,
            Page::Startup,
            Page::DefaultPrograms,
            Page::Advanced,
        ],
    ),
];

impl Page {
    pub fn all() -> impl Iterator<Item = Page> {
        GROUPS.iter().flat_map(|(_, pages)| pages.iter().copied())
    }

    pub fn at(i: usize) -> Option<Page> {
        Page::all().nth(i)
    }

    pub fn index(self) -> usize {
        Page::all()
            .position(|p| p == self)
            .expect("every page is in GROUPS")
    }

    pub fn label(self) -> &'static str {
        match self {
            Page::Appearance => "Appearance",
            Page::Background => "Background",
            Page::ScreenSaver => "Screen Saver",
            Page::Display => "Display",
            Page::NightLight => "Night Light",
            Page::FontsAndIcons => "Fonts & Icons",
            Page::Effects => "Effects",
            Page::Taskbar => "Taskbar",
            Page::StartMenu => "Start Menu",
            Page::Launcher => "Launcher",
            Page::Switcher => "Window Switcher",
            Page::Desktops => "Desktops",
            Page::Windows => "Windows",
            Page::Keyboard => "Keyboard",
            Page::Mouse => "Mouse",
            Page::Tablet => "Tablet",
            Page::Pointers => "Pointers",
            Page::Accessibility => "Accessibility",
            Page::Vision => "Vision",
            Page::Power => "Power",
            Page::Startup => "Startup",
            Page::DefaultPrograms => "Default Programs",
            Page::Advanced => "Advanced",
        }
    }

    /// The words a search finds the page by besides its label, lower case.
    pub fn keywords(self) -> &'static [&'static str] {
        match self {
            Page::Appearance => &[
                "theme",
                "interface",
                "classic",
                "blue",
                "glass",
                "colors",
                "colours",
                "scheme",
                "dark mode",
            ],
            Page::Background => &["wallpaper", "picture", "lock screen", "slideshow"],
            Page::ScreenSaver => &["screensaver", "starfield", "mystify", "photos", "idle"],
            Page::Display => &[
                "monitor",
                "screen",
                "resolution",
                "refresh rate",
                "scale",
                "scaling",
            ],
            Page::NightLight => &["blue light", "colour temperature", "color temperature"],
            Page::FontsAndIcons => &["font", "icon theme", "gtk", "qt", "dpi"],
            Page::Effects => &["animations", "transparency", "shadows"],
            Page::Taskbar => &["tray", "notification area", "panel"],
            Page::StartMenu => &["menu", "pinned"],
            Page::Launcher => &["search", "run"],
            Page::Switcher => &["alt tab", "alt+tab", "task switcher"],
            Page::Desktops => &["workspaces", "virtual desktops"],
            Page::Windows => &[
                "tiling",
                "snap",
                "snapping",
                "layouts",
                "tabs",
                "tab groups",
            ],
            Page::Keyboard => &[
                "shortcuts",
                "hotkeys",
                "keybindings",
                "keyboard layout",
                "repeat",
            ],
            Page::Mouse => &[
                "touchpad",
                "trackpad",
                "pointer speed",
                "natural scrolling",
                "scrolling",
                "double-click",
            ],
            Page::Tablet => &["pen", "drawing tablet", "pad", "express keys", "wacom"],
            Page::Pointers => &["cursor", "cursor theme"],
            Page::Accessibility => &[
                "sticky keys",
                "slow keys",
                "mouse keys",
                "text size",
                "large text",
                "a11y",
            ],
            Page::Vision => &[
                "color",
                "colour",
                "filter",
                "grayscale",
                "invert",
                "colour blindness",
                "color blindness",
                "magnifier",
                "zoom",
            ],
            Page::Power => &["idle", "sleep", "suspend", "standby", "lid", "screen off"],
            Page::Startup => &["autostart", "login", "session", "startup programs"],
            Page::DefaultPrograms => &[
                "default",
                "browser",
                "mail",
                "terminal",
                "open with",
                "file types",
            ],
            Page::Advanced => &["screenshots", "task manager", "thumbnails", "cache"],
        }
    }

    /// Its `--page` name: the label in lower case, "&" spelled "and",
    /// spaces as hyphens (`fonts-and-icons`).
    pub fn slug(self) -> String {
        self.label()
            .to_lowercase()
            .replace('&', "and")
            .replace(' ', "-")
    }

    pub fn named(name: &str) -> Option<Page> {
        Page::all().find(|p| p.slug() == name)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Program {
    pub id: &'static str,
    pub keywords: &'static [&'static str],
}

pub const PROGRAMS: &[Program] = &[
    Program {
        id: "reprise-network.desktop",
        keywords: &[
            "network", "wifi", "wi-fi", "wireless", "wlan", "ethernet", "wired", "vpn", "internet",
        ],
    },
    Program {
        id: "reprise-sound.desktop",
        keywords: &[
            "sound",
            "audio",
            "volume",
            "speakers",
            "microphone",
            "headphones",
        ],
    },
    Program {
        id: "reprise-bluetooth.desktop",
        keywords: &["bluetooth", "pairing"],
    },
    Program {
        id: "reprise-power.desktop",
        keywords: &["battery", "power profile", "performance", "power saver"],
    },
    Program {
        id: "reprise-datetime.desktop",
        keywords: &["clock", "date", "time", "time zone", "timezone"],
    },
    Program {
        id: "reprise-accounts.desktop",
        keywords: &["user", "users", "account", "password", "picture"],
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keywords_are_lower_case_and_no_page_repeats_its_label() {
        let words = Page::all()
            .flat_map(|p| p.keywords().iter().map(move |k| (p.label(), *k)))
            .chain(
                PROGRAMS
                    .iter()
                    .flat_map(|p| p.keywords.iter().map(|k| (p.id, *k))),
            );
        for (owner, k) in words {
            assert_eq!(k, k.to_lowercase(), "{owner}: {k}");
            assert_ne!(k, owner.to_lowercase(), "{owner} repeats its label");
        }
    }
}
