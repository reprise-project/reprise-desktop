//! Settings pages and separate settings programs with their search words. [`GROUPS`]
//! fixes the list's order, pages and program links alike; [`Page::find`] is the one
//! matcher Settings and Start share.

mod terms;

use crate::search::{all_words, word_starts};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Appearance,
    Background,
    ScreenSaverLock,
    FontsAndIcons,
    Effects,
    Taskbar,
    StartMenu,
    Search,
    Switcher,
    VirtualDesktops,
    WindowBehavior,
    Display,
    NightLight,
    Sound,
    Notifications,
    Keyboard,
    MouseTouchpad,
    Cursor,
    Tablet,
    TextKeys,
    MagnifierFilters,
    RemoteDesktop,
    Power,
    Startup,
    DefaultPrograms,
    Privacy,
    Advanced,
    Backup,
}

/// A row under a heading of the Settings list: a page, or a separate program by its
/// [`PROGRAMS`] id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Entry {
    Page(Page),
    Program(&'static str),
}

/// What a search matched: the label, a keyword, or a control's label (its id, for --focus).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Label,
    Keyword,
    Control(&'static str),
}

pub const GROUPS: &[(&str, &[Entry])] = &[
    (
        "Personalize",
        &[
            Entry::Page(Page::Appearance),
            Entry::Page(Page::Background),
            Entry::Page(Page::ScreenSaverLock),
            Entry::Page(Page::FontsAndIcons),
            Entry::Page(Page::Effects),
        ],
    ),
    (
        "Taskbar & Windows",
        &[
            Entry::Page(Page::Taskbar),
            Entry::Page(Page::StartMenu),
            Entry::Page(Page::Search),
            Entry::Page(Page::Switcher),
            Entry::Page(Page::VirtualDesktops),
            Entry::Page(Page::WindowBehavior),
        ],
    ),
    (
        "Display & Sound",
        &[
            Entry::Page(Page::Display),
            Entry::Page(Page::NightLight),
            Entry::Page(Page::Sound),
            Entry::Page(Page::Notifications),
        ],
    ),
    (
        "Devices",
        &[
            Entry::Page(Page::Keyboard),
            Entry::Page(Page::MouseTouchpad),
            Entry::Page(Page::Cursor),
            Entry::Page(Page::Tablet),
            Entry::Program("reprise-bluetooth.desktop"),
            Entry::Program("reprise-printers.desktop"),
        ],
    ),
    (
        "Accessibility",
        &[
            Entry::Page(Page::TextKeys),
            Entry::Page(Page::MagnifierFilters),
        ],
    ),
    (
        "Network",
        &[
            Entry::Program("reprise-network.desktop"),
            Entry::Page(Page::RemoteDesktop),
        ],
    ),
    (
        "System",
        &[
            Entry::Page(Page::Power),
            Entry::Page(Page::Startup),
            Entry::Page(Page::DefaultPrograms),
            Entry::Page(Page::Privacy),
            Entry::Program("reprise-datetime.desktop"),
            Entry::Program("reprise-accounts.desktop"),
            Entry::Program("reprise-updates.desktop"),
            Entry::Page(Page::Advanced),
            Entry::Page(Page::Backup),
        ],
    ),
];

/// Old `--page` names, each still opening its renamed page.
const ALIASES: &[(&str, Page)] = &[
    ("screen-saver", Page::ScreenSaverLock),
    ("launcher", Page::Search),
    ("desktops", Page::VirtualDesktops),
    ("windows", Page::WindowBehavior),
    ("mouse", Page::MouseTouchpad),
    ("pointers", Page::Cursor),
    ("accessibility", Page::TextKeys),
    ("vision", Page::MagnifierFilters),
    ("sounds", Page::Sound),
];

/// The list entry a `--page` name opens.
pub fn named(slug: &str) -> Option<Entry> {
    Page::named(slug).map(Entry::Page)
}

impl Page {
    /// The pages in list order.
    pub fn all() -> impl Iterator<Item = Page> {
        GROUPS.iter().flat_map(|(_, entries)| {
            entries.iter().filter_map(|e| match e {
                Entry::Page(p) => Some(*p),
                Entry::Program(_) => None,
            })
        })
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
            Page::ScreenSaverLock => "Screen Saver & Lock",
            Page::Display => "Display",
            Page::NightLight => "Night Light",
            Page::FontsAndIcons => "Fonts & Icons",
            Page::Effects => "Effects",
            Page::Taskbar => "Taskbar",
            Page::StartMenu => "Start Menu",
            Page::Search => "Search",
            Page::Switcher => "Window Switcher",
            Page::VirtualDesktops => "Virtual Desktops",
            Page::WindowBehavior => "Window Behavior",
            Page::Keyboard => "Keyboard",
            Page::MouseTouchpad => "Mouse & Touchpad",
            Page::Tablet => "Tablet",
            Page::Cursor => "Cursor",
            Page::TextKeys => "Text & Keys",
            Page::MagnifierFilters => "Magnifier & Filters",
            Page::Sound => "Sound",
            Page::Notifications => "Notifications",
            Page::Power => "Power",
            Page::Startup => "Startup",
            Page::DefaultPrograms => "Default Programs",
            Page::Privacy => "Privacy",
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
            Page::Background => &[
                "wallpaper",
                "picture",
                "slideshow",
                "extensions",
                "widgets",
                "water",
            ],
            Page::ScreenSaverLock => &[
                "screensaver",
                "starfield",
                "mystify",
                "photos",
                "idle",
                "lock",
                "lock screen",
                "away",
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
                "freesync",
            ],
            Page::NightLight => &["blue light", "colour temperature", "color temperature"],
            Page::FontsAndIcons => &["font", "antialias", "smoothing", "icon theme", "gtk", "qt"],
            Page::Effects => &["animations", "shadows"],
            Page::Taskbar => &[
                "tray",
                "notification area",
                "panel",
                "auto-hide",
                "autohide",
                "peek",
                "previews",
            ],
            Page::StartMenu => &["menu", "pinned", "quick launch", "banner"],
            Page::Search => &["launcher", "run"],
            Page::Switcher => &[
                "alt tab",
                "alt+tab",
                "task switcher",
                "overview",
                "hot corner",
                "3d",
                "flip",
            ],
            Page::VirtualDesktops => &["workspaces", "desktops"],
            Page::WindowBehavior => &[
                "windows",
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
            Page::MouseTouchpad => &[
                "touchpad",
                "trackpad",
                "pointer speed",
                "natural scrolling",
                "scrolling",
                "double-click",
                "acceleration",
            ],
            Page::Tablet => &["pen", "drawing tablet", "pad", "express keys", "wacom"],
            Page::Cursor => &["pointers", "cursor theme", "large cursor"],
            Page::TextKeys => &[
                "accessibility",
                "sticky keys",
                "slow keys",
                "mouse keys",
                "text size",
                "large text",
                "a11y",
            ],
            Page::MagnifierFilters => &[
                "vision",
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
            Page::Sound => &[
                "sounds",
                "sound theme",
                "bell",
                "beep",
                "volume",
                "boost",
                "louder",
            ],
            Page::Notifications => &[
                "balloons",
                "notification sounds",
                "do not disturb",
                "dnd",
                "quiet hours",
                "popups",
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
            Page::Privacy => &[
                "location",
                "clipboard",
                "recent files",
                "history",
                "screen sharing",
            ],
            Page::Advanced => &["screenshots", "task manager", "thumbnails", "cache"],
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

    /// The page `name` opens: its slug, or an old name the page went by.
    pub fn named(name: &str) -> Option<Page> {
        Page::all().find(|p| p.slug() == name).or_else(|| {
            ALIASES
                .iter()
                .find(|(old, _)| *old == name)
                .map(|&(_, p)| p)
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Program {
    pub id: &'static str,
    /// The program's name, as search and menus show it.
    pub label: &'static str,
    /// The link row's text where the heading above says the rest; [`Program::row_label`].
    pub short_label: Option<&'static str>,
    pub keywords: &'static [&'static str],
}

impl Program {
    /// The link row's text; the list adds the "...".
    pub fn row_label(&self) -> &'static str {
        self.short_label.unwrap_or(self.label)
    }

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

/// The program `id` (a desktop entry id) names.
pub fn program(id: &str) -> Option<&'static Program> {
    PROGRAMS.iter().find(|p| p.id == id)
}

/// The list's link rows in [`GROUPS`] order; a link's number is its place here.
pub fn links() -> impl Iterator<Item = &'static Program> {
    GROUPS.iter().flat_map(|(_, entries)| {
        entries.iter().filter_map(|e| match e {
            Entry::Program(id) => Some(program(id).expect("every link is in PROGRAMS")),
            Entry::Page(_) => None,
        })
    })
}

pub const PROGRAMS: &[Program] = &[
    Program {
        id: "reprise-network.desktop",
        label: "Network Connections",
        short_label: Some("Connections"),
        keywords: &[
            "network", "wifi", "wi-fi", "wireless", "wlan", "ethernet", "wired", "vpn", "internet",
        ],
    },
    Program {
        id: "reprise-sound.desktop",
        label: "Sounds and Audio Devices",
        short_label: None,
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
        label: "Bluetooth",
        short_label: None,
        keywords: &["bluetooth", "pairing"],
    },
    Program {
        id: "reprise-power.desktop",
        label: "Power Options",
        short_label: None,
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
        label: "Date and Time",
        short_label: None,
        keywords: &["clock", "date", "time", "time zone", "timezone"],
    },
    Program {
        id: "reprise-accounts.desktop",
        label: "User Accounts",
        short_label: None,
        keywords: &["user", "users", "account", "password", "picture"],
    },
    Program {
        id: "reprise-printers.desktop",
        label: "Printers",
        short_label: None,
        keywords: &["printer", "printing", "print queue", "cups"],
    },
    Program {
        id: "reprise-updates.desktop",
        label: "Software Updates",
        short_label: None,
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
    fn the_slugs_the_compositor_and_portal_pass_are_current() {
        for s in [
            "night-light",
            "keyboard",
            "taskbar",
            "background",
            "text-and-keys",
        ] {
            assert_eq!(Page::named(s).map(Page::slug).as_deref(), Some(s));
        }
    }

    #[test]
    fn a_page_is_found_by_label_then_keyword_then_control_label() {
        assert_eq!(Page::NightLight.find("night"), Some(Hit::Label));
        assert_eq!(Page::FontsAndIcons.find("icons fonts"), Some(Hit::Label));
        assert_eq!(Page::FontsAndIcons.find("smoothing"), Some(Hit::Keyword));
        assert_eq!(
            Page::FontsAndIcons.find("font smoothing"),
            Some(Hit::Keyword)
        );
        assert_eq!(Page::FontsAndIcons.find("antialias"), Some(Hit::Keyword));
        assert_eq!(Page::MouseTouchpad.find("mouse speed"), Some(Hit::Keyword));
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
    fn the_switcher_page_is_found_by_3d_and_flip() {
        assert_eq!(Page::Switcher.find("3d"), Some(Hit::Keyword));
        assert_eq!(Page::Switcher.find("flip"), Some(Hit::Keyword));
    }

    #[test]
    fn link_rows_name_programs_in_tree_order() {
        let labels: Vec<&str> = links().map(|p| p.label).collect();
        assert_eq!(
            labels,
            [
                "Bluetooth",
                "Printers",
                "Network Connections",
                "Date and Time",
                "User Accounts",
                "Software Updates"
            ]
        );
        let rows: Vec<&str> = links().map(Program::row_label).collect();
        assert_eq!(rows[2], "Connections", "the Network heading says the rest");
        for (_, entries) in GROUPS {
            for e in *entries {
                if let Entry::Program(id) = e {
                    assert!(program(id).is_some(), "{id} is in PROGRAMS");
                }
            }
        }
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
