//! `$XDG_CONFIG_HOME/reprise/notepad.toml`: the font Format > Font picks, Word Wrap, the
//! status bar and File > Page Setup's paper, margins, header and footer.

use reprisekit::controls::fontdlg::Style;
use reprisekit::controls::pagesetup::{PAPERS, PageSetup};
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
    /// None until Page Setup is first OKed: the paper of the user's locale.
    #[serde(deserialize_with = "lenient_page")]
    pub page: Option<PageSetup>,
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
            page: None,
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

/// `[page]` read value by value: one that is missing, of the wrong type or out of range reads as
/// the locale's, so a bad page costs only itself.
fn lenient_page<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<PageSetup>, D::Error> {
    Ok(Some(page_from(&toml::Value::deserialize(d)?)))
}

fn page_from(v: &toml::Value) -> PageSetup {
    let base = repriseapp::print::locale_setup();
    let get = |k| v.as_table().and_then(|t| t.get(k));
    let paper = get("paper")
        .and_then(toml::Value::as_str)
        .filter(|p| PAPERS.iter().any(|x| x.media == *p))
        .map_or_else(|| base.paper.clone(), String::from);
    let margins = get("margins")
        .and_then(toml::Value::as_array)
        .and_then(|a| {
            a.iter()
                .map(|m| m.as_integer().and_then(|i| i32::try_from(i).ok()))
                .collect::<Option<Vec<i32>>>()
        })
        .and_then(|m| <[i32; 4]>::try_from(m).ok());
    let text = |k| {
        get(k)
            .and_then(toml::Value::as_str)
            .map_or_else(String::new, String::from)
    };
    let mut page = PageSetup {
        paper,
        landscape: get("landscape")
            .and_then(toml::Value::as_bool)
            .unwrap_or(false),
        margins: margins.unwrap_or(base.margins),
        header: text("header"),
        footer: text("footer"),
        ..base
    };
    if !page.fits() {
        page.margins = base.margins;
    }
    page
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
