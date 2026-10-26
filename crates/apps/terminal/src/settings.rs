//! `$XDG_CONFIG_HOME/reprise/terminal.toml`: what the Options sheet sets.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Cursor {
    #[default]
    Block,
    Bar,
    Underline,
}

/// Where a window keeps its tabs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TabPosition {
    /// A strip under the toolbar.
    #[default]
    Below,
    /// The title bar: each tab is a window of one caption tab group.
    Caption,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Empty runs the login shell from the password database.
    pub shell: String,
    /// Takes effect for windows opened after a change.
    pub tab_position: TabPosition,
    pub cursor: Cursor,
    pub blink: bool,
    pub scrollback: usize,
    /// Ask before a paste holding a line end goes to a program not in bracketed-paste mode.
    pub confirm_multiline_paste: bool,
    /// Empty picks the first installed of [`crate::FONTS`].
    pub font: String,
    pub size_px: u32,
    pub scheme: String,
    pub see_through: bool,
    /// The see-through background's opacity, in percent.
    pub opacity: u8,
    pub profiles: Vec<Profile>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            shell: String::new(),
            tab_position: TabPosition::Below,
            cursor: Cursor::Block,
            blink: true,
            scrollback: 10_000,
            confirm_multiline_paste: true,
            font: String::new(),
            size_px: 13,
            scheme: "reprise".into(),
            see_through: false,
            opacity: 80,
            profiles: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Profile {
    pub name: String,
    pub command: String,
}

pub fn path() -> PathBuf {
    repriseconf::xdg_home("XDG_CONFIG_HOME", ".config")
        .join("reprise")
        .join("terminal.toml")
}

/// The settings in `path`, the defaults when it is missing; an error when it cannot be read or
/// does not parse.
pub fn try_load(path: &Path) -> Result<Settings, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => toml::from_str(&text).map_err(|e| e.to_string()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Settings::default()),
        Err(e) => Err(e.to_string()),
    }
}

/// [`try_load`], with the defaults for a file that cannot be read.
pub fn load(path: &Path) -> Settings {
    try_load(path).unwrap_or_else(|e| {
        log::warn!("{}: {e}; running on the defaults", path.display());
        Settings::default()
    })
}

pub fn save(path: &Path, s: &Settings) -> io::Result<()> {
    let text = toml::to_string(s).map_err(io::Error::other)?;
    repriseconf::write_atomic(path, &text)
}

/// What the window keeps between runs, in `$XDG_STATE_HOME/reprise/terminal.toml`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    /// The client size, logical px; none until the window was sized by hand.
    pub window: Option<(i32, i32)>,
}

pub fn state_path() -> PathBuf {
    repriseconf::xdg_home("XDG_STATE_HOME", ".local/state")
        .join("reprise")
        .join("terminal.toml")
}

impl State {
    pub fn load(path: &Path) -> State {
        let state: State = std::fs::read_to_string(path)
            .ok()
            .and_then(|t| toml::from_str(&t).ok())
            .unwrap_or_default();
        State {
            window: state
                .window
                .map(|(w, h)| (w.max(repriseapp::MIN_SIZE.0), h.max(repriseapp::MIN_SIZE.1))),
        }
    }

    pub fn save(&self, path: &Path) {
        let written = toml::to_string(self)
            .map_err(io::Error::other)
            .and_then(|text| repriseconf::write_atomic(path, &text));
        if let Err(e) = written {
            log::warn!("{}: {e}", path.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let s = load(&dir.path().join("terminal.toml"));
        assert_eq!(s.scrollback, 10_000);
        assert_eq!(s.size_px, 13);
        assert_eq!(s.scheme, "reprise");
        assert!(!s.see_through);
        assert_eq!(s.tab_position, TabPosition::Below);
        assert_eq!(s, Settings::default());
    }

    #[test]
    fn unknown_keys_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("terminal.toml");
        std::fs::write(
            &p,
            "scrollback = 500\ntab_colour = \"red\"\n[later]\nx = 1\n",
        )
        .unwrap();
        let s = load(&p);
        assert_eq!(s.scrollback, 500);
        assert_eq!(s.size_px, 13, "keys the file leaves out keep their default");
    }

    #[test]
    fn roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("reprise").join("terminal.toml");
        let s = Settings {
            shell: "/bin/zsh".into(),
            tab_position: TabPosition::Caption,
            cursor: Cursor::Bar,
            blink: false,
            scrollback: 2000,
            confirm_multiline_paste: false,
            font: "Iosevka".into(),
            size_px: 15,
            scheme: "light".into(),
            see_through: true,
            opacity: 60,
            profiles: vec![Profile {
                name: "Top".into(),
                command: "htop".into(),
            }],
        };
        save(&p, &s).unwrap();
        assert_eq!(load(&p), s);
    }

    #[test]
    fn a_broken_file_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("terminal.toml");
        assert_eq!(
            try_load(&p),
            Ok(Settings::default()),
            "a missing file is the defaults"
        );
        std::fs::write(&p, "scrollback = 500\nsize_px = [").unwrap();
        assert!(try_load(&p).is_err());
        assert_eq!(
            load(&p),
            Settings::default(),
            "startup runs on the defaults"
        );
    }

    #[test]
    fn save_replaces_the_file_whole() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("terminal.toml");
        std::fs::write(&p, "scrollback = 500\n").unwrap();
        let link = dir.path().join("link.toml");
        std::fs::hard_link(&p, &link).unwrap();
        save(&p, &Settings::default()).unwrap();
        assert_eq!(load(&p), Settings::default());
        assert_eq!(
            std::fs::read_to_string(&link).unwrap(),
            "scrollback = 500\n",
            "a reader holding the old file never sees it rewritten in place"
        );
    }

    #[test]
    fn window_size_in_state() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("reprise").join("terminal.toml");
        assert_eq!(State::load(&p), State::default(), "no file, no size");
        State {
            window: Some((900, 500)),
        }
        .save(&p);
        assert_eq!(State::load(&p).window, Some((900, 500)));
        State {
            window: Some((3, 3)),
        }
        .save(&p);
        assert_eq!(
            State::load(&p).window,
            Some(repriseapp::MIN_SIZE),
            "a size below the floor is lifted to it"
        );
    }
}
