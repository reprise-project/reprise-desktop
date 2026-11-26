//! `$XDG_CONFIG_HOME/reprise/notepad.toml`: the font Format > Font picks, Word Wrap and the
//! status bar.

use reprisekit::controls::fontdlg::Style;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Settings {
    /// Empty is the fixed-pitch face plain text is set in ([`TextRenderer::monospace`]).
    ///
    /// [`TextRenderer::monospace`]: reprisekit::text::TextRenderer::monospace
    pub font: String,
    pub bold: bool,
    pub italic: bool,
    /// Points at 100% text size.
    pub size: u32,
    pub word_wrap: bool,
    pub status_bar: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            font: String::new(),
            bold: false,
            italic: false,
            size: 10,
            word_wrap: false,
            status_bar: false,
        }
    }
}

impl Settings {
    pub(crate) fn style(&self) -> Style {
        match (self.bold, self.italic) {
            (false, false) => Style::Regular,
            (false, true) => Style::Italic,
            (true, false) => Style::Bold,
            (true, true) => Style::BoldItalic,
        }
    }
}

pub(crate) fn path() -> Option<PathBuf> {
    Some(
        reprisexdg::config_home()?
            .join("reprise")
            .join("notepad.toml"),
    )
}

/// The settings at `path`; the defaults where it is missing or does not read.
pub(crate) fn load(path: &Path) -> Settings {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Settings::default();
    };
    toml::from_str(&text).unwrap_or_else(|e| {
        log::warn!("{}: {e}", path.display());
        Settings::default()
    })
}

pub(crate) fn save(path: &Path, s: &Settings) {
    let made = path.parent().map_or(Ok(()), std::fs::create_dir_all);
    let written = made
        .and_then(|()| toml::to_string(s).map_err(std::io::Error::other))
        .and_then(|text| reprisefs::ops::write_atomic(path, text.as_bytes()));
    if let Err(e) = written {
        log::warn!("cannot write {}: {e}", path.display());
    }
}
