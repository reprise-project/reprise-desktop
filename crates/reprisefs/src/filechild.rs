//! The file dialog child's contract (`reprise-portal --file`): the [`Input`] it reads as TOML on
//! stdin and the [`Output`] it prints as TOML on stdout when it closes. The portal's FileChooser
//! and the compositor's Browse... buttons both start it.

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Open,
    Save,
    SaveFiles,
}

/// A Files of type pattern: a glob on the name, a MIME type, or the execute bit.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum Pattern {
    Glob(String),
    Mime(String),
    /// A file with an execute bit (`listing::Item::exec`); no FileChooser call can send it.
    Exec,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Filter {
    pub label: String,
    pub patterns: Vec<Pattern>,
}

/// A caller's extra question: a check box when it has no options, else a combo box.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub id: String,
    pub label: String,
    pub options: Vec<(String, String)>,
    pub initial: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
pub struct Input {
    pub parent: Option<String>,
    pub modal: bool,
    /// Always set; an `Option` only so `Input` has a `Default`.
    pub kind: Option<Kind>,
    pub folder_mode: bool,
    pub multiple: bool,
    pub title: String,
    pub accept_label: String,
    pub filters: Vec<Filter>,
    pub filter: usize,
    /// The folder the caller asked to start in; empty opens where the app's last dialog was
    /// confirmed.
    pub start: Vec<u8>,
    /// The requesting app's id, empty when unknown: what the dialog keeps its folder and view
    /// under.
    pub app_id: String,
    pub name: String,
    /// `name`'s real bytes, when it is not UTF-8.
    pub name_raw: Option<Vec<u8>>,
    /// SaveFiles' names, as bytes with their trailing NUL.
    pub files: Vec<Vec<u8>>,
    pub choices: Vec<Choice>,
    /// Every file with an execute bit wears the program icon (`application-x-executable`).
    pub exec_icon: bool,
}

/// What the dialog child answers. The default (nothing picked) is a
/// cancel, which is also what a killed or failed child reads as.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
pub struct Output {
    pub paths: Vec<Vec<u8>>,
    pub filter: Option<usize>,
    pub choices: Vec<(String, String)>,
}
