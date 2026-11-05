//! Settings pages and separate settings programs with their search words. [`GROUPS`]
//! fixes the pages' order; [`Page::find`] is the one matcher Settings and Start share.

mod terms;

use crate::search::{all_words, word_starts};

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
    Sounds,
    Power,
    Startup,
    DefaultPrograms,
    RemoteDesktop,
    Advanced,
    Backup,
}

/// What a search matched: the label, a keyword, or a control's label (its id, for --focus).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Label,
    Keyword,
    Control(&'static str),
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
            Page::Sounds,
            Page::Power,
            Page::Startup,
            Page::DefaultPrograms,
            Page::RemoteDesktop,
            Page::Advanced,
            Page::Backup,
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
            Page::Sounds => "Sounds",
            Page::Power => "Power",
            Page::Startup => "Startup",
            Page::DefaultPrograms => "Default Programs",
            Page::RemoteDesktop => "Remote Desktop",
            Page::Advanced => "Advanced",
            Page::Backup => "Backup",
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
                "transparency",
                "blur",
                "high contrast",
            ],
            Page::Background => &["wallpaper", "picture", "slideshow", "extensions"],
            Page::ScreenSaver => &[
                "screensaver",
                "starfield",
                "mystify",
                "photos",
                "idle",
                "lock",
                "lock screen",
            ],
            Page::Display => &[
                "monitor",
                "screen",
                "resolution",
                "refresh rate",
                "scale",
                "scaling",
                "dpi",
                "vrr",
                "variable refresh rate",
                "adaptive sync",
            ],
            Page::NightLight => &["blue light", "colour temperature", "color temperature"],
            Page::FontsAndIcons => &["font", "icon theme", "gtk", "qt"],
            Page::Effects => &["animations", "shadows"],
            Page::Taskbar => &[
                "tray",
                "notification area",
                "panel",
                "auto-hide",
                "autohide",
                "peek",
            ],
            Page::StartMenu => &["menu", "pinned", "quick launch"],
            Page::Launcher => &["search", "run"],
            Page::Switcher => &[
                "alt tab",
                "alt+tab",
                "task switcher",
                "overview",
                "hot corner",
            ],
            Page::Desktops => &["workspaces", "virtual desktops", "widgets"],
            Page::Windows => &[
                "tiling",
                "snap",
                "snapping",
                "layouts",
                "tabs",
                "tab groups",
                "window position",
                "placement",
            ],
            Page::Keyboard => &[
                "shortcuts",
                "hotkeys",
                "keybindings",
                "keyboard layout",
                "repeat",
                "language",
            ],
            Page::Mouse => &[
                "touchpad",
                "trackpad",
                "pointer speed",
                "natural scrolling",
                "scrolling",
                "double-click",
                "acceleration",
            ],
            Page::Tablet => &["pen", "drawing tablet", "pad", "express keys", "wacom"],
            Page::Pointers => &["cursor", "cursor theme", "large cursor"],
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
            Page::Sounds => &[
                "sound theme",
                "notification sounds",
                "bell",
                "beep",
                "volume",
                "boost",
                "louder",
                "notifications",
            ],
            Page::Power => &["idle", "screen off", "free space", "storage"],
            Page::Startup => &["autostart", "login", "session", "startup programs"],
            Page::DefaultPrograms => &[
                "default",
                "browser",
                "mail",
                "terminal",
                "scripts",
                "open with",
                "file types",
            ],
            Page::RemoteDesktop => &["remote", "rdp", "remmina", "desktop sharing"],
            Page::Advanced => &[
                "screenshots",
                "task manager",
                "thumbnails",
                "cache",
                "privacy",
                "location",
                "clipboard",
            ],
            Page::Backup => &["export", "import", "back up", "transfer", "move settings"],
        }
    }

    /// Each labelled control on the page as (control id, label as shown), for search and --focus.
    pub fn terms(self) -> &'static [(&'static str, &'static str)] {
        terms::of(self)
    }

    /// Matches when every word of `query` starts a word of the label, a keyword or a
    /// control label; a control's hit names the first control holding a word the page lacks.
    pub fn find(self, query: &str) -> Option<Hit> {
        if let Some(hit) = find_by_name(query, self.label(), self.keywords()) {
            return Some(hit);
        }
        let page: Vec<&str> = [self.label()]
            .into_iter()
            .chain(self.keywords().iter().copied())
            .collect();
        let all: Vec<&str> = page
            .iter()
            .copied()
            .chain(self.terms().iter().map(|&(_, l)| l))
            .collect();
        if !all_words(query, &all) {
            return None;
        }
        let rest: Vec<&str> = query
            .split_whitespace()
            .filter(|w| !page.iter().any(|h| word_starts(w, h)))
            .collect();
        self.terms()
            .iter()
            .find(|&&(_, l)| rest.iter().any(|w| word_starts(w, l)))
            .map(|&(id, _)| Hit::Control(id))
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

impl Program {
    /// `name` is the program's desktop-entry name; a program has no control hits.
    pub fn find(&self, query: &str, name: &str) -> Option<Hit> {
        find_by_name(query, name, self.keywords)
    }
}

fn find_by_name(query: &str, name: &str, keywords: &[&str]) -> Option<Hit> {
    if all_words(query, &[name]) {
        return Some(Hit::Label);
    }
    let hay: Vec<&str> = [name].into_iter().chain(keywords.iter().copied()).collect();
    all_words(query, &hay).then_some(Hit::Keyword)
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
        keywords: &[
            "battery",
            "power profile",
            "performance",
            "power saver",
            "sleep",
            "suspend",
            "standby",
            "lid",
        ],
    },
    Program {
        id: "reprise-datetime.desktop",
        keywords: &["clock", "date", "time", "time zone", "timezone"],
    },
    Program {
        id: "reprise-accounts.desktop",
        keywords: &["user", "users", "account", "password", "picture"],
    },
    Program {
        id: "reprise-printers.desktop",
        keywords: &["printer", "printing", "print queue", "cups"],
    },
    Program {
        id: "reprise-updates.desktop",
        keywords: &["upgrade", "packages", "flatpak", "firmware"],
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

    #[test]
    fn a_page_is_found_by_label_then_keyword_then_control_label() {
        assert_eq!(Page::NightLight.find("night"), Some(Hit::Label));
        assert_eq!(Page::FontsAndIcons.find("icons fonts"), Some(Hit::Label));
        assert_eq!(Page::Mouse.find("mouse speed"), Some(Hit::Keyword));
        assert_eq!(
            Page::DefaultPrograms.find("default browser"),
            Some(Hit::Keyword)
        );
        assert_eq!(
            Page::Keyboard.find("delay"),
            Some(Hit::Control("repeat_delay"))
        );
        assert_eq!(
            Page::Keyboard.find("keyboard delay"),
            Some(Hit::Control("repeat_delay")),
            "the label takes one word, a control the other"
        );
        assert_eq!(
            Page::Switcher.find("picture size"),
            Some(Hit::Control("switcher_size"))
        );
        assert_eq!(Page::Keyboard.find("delay zzz"), None);
        assert_eq!(Page::Keyboard.find(""), None);
        assert_eq!(Page::Keyboard.find("  "), None);
    }

    #[test]
    fn a_program_is_found_by_its_name_then_its_keywords() {
        let power = PROGRAMS
            .iter()
            .find(|p| p.id == "reprise-power.desktop")
            .unwrap();
        assert_eq!(
            power.find("options power", "Power Options"),
            Some(Hit::Label)
        );
        assert_eq!(power.find("sleep", "Power Options"), Some(Hit::Keyword));
        assert_eq!(
            power.find("power sleep", "Power Options"),
            Some(Hit::Keyword)
        );
        assert_eq!(power.find("delay", "Power Options"), None);
        assert_eq!(power.find("", "Power Options"), None);
    }
}
