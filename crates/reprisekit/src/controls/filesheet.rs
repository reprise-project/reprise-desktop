//! General properties page for a file, folder or set, plus the Advanced Attributes
//! and Confirm Attribute Changes boxes. All coordinates are client coordinates.

use crate::classic::widgets::CheckState;
use crate::controls::form::{CHECK, Control, Form, Item};
use crate::raster::{Canvas, Rect};
use crate::text::DialogFont;

mod attrs;
mod bin;
mod customize;
mod general;
mod inert;
mod links;
mod summary;

pub use attrs::{
    ADV_ARCHIVE, ADV_CANCEL, ADV_INDEX, ADV_OK, ATTR_CANCEL, ATTR_LABELS, ATTR_OK, ATTR_ONLY,
    ATTR_SIZE, ATTR_TREE, SET_ATTR_LABELS, advanced_form, advanced_size, attr_form,
    attr_tree_picked, robots_chosen, robots_moves, robots_states,
};
pub use bin::{BIN_SHEET_H, BinFacts, RESTORE, bin_page};
pub use customize::{
    CHANGE_ICON, CHANGE_ICON_CANCEL, CHANGE_ICON_LIST, CHANGE_ICON_OK, CHANGE_ICON_RESTORE,
    CHANGE_ICON_SIZE, CUSTOM_ICON, CUSTOM_PREVIEW, FOLDER_ICON_NAMES, PREVIEW_PX,
    STANDARD_FOLDER_ICON, change_icon_form, change_icon_picked, change_icon_restore,
    customize_page,
};
pub use general::{
    executable, general_page, hidden, hidden_state, read_only, rows, set_moves, set_picture,
    set_rows, shared_state, shown_name, typed_name, wanted_name,
};
pub use inert::{compat_page, sharing_page};
pub use links::{
    FIND_TARGET, SHORTCUT_IDS, ShortcutFacts, WebFacts, shortcut_moves, shortcut_of, shortcut_page,
    shortcut_values, web_page,
};
pub use summary::{
    SUMMARY_ADVANCED, SUMMARY_IDS, no_summary_page, summary_moves, summary_of, summary_page,
    summary_values,
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Facts {
    pub is_dir: bool,
    pub type_name: String,
    pub opens_with: Option<String>,
    pub description: Option<String>,
    pub location: String,
    pub size: String,
    pub on_disk: String,
    pub contains: Option<String>,
    pub created: Option<String>,
    pub modified: String,
    pub accessed: String,
    pub read_only: bool,
    /// The owner's execute bit; a folder's box is unticked and greyed.
    pub executable: bool,
    /// The Executable box takes a click.
    pub can_set_executable: bool,
    /// The name starts with a dot, which is what hides a file here.
    pub hidden: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SetFacts {
    pub has_dir: bool,
    pub count: String,
    pub type_text: String,
    pub location: String,
    pub size: String,
    pub on_disk: String,
    pub read_only: CheckState,
    pub hidden: CheckState,
}

pub const CALCULATING: &str = "Calculating...";

pub const NAME: &str = "name";
/// What a reader calls the name field, which no label names.
pub const NAME_FIELD: &str = "Name";
pub const READ_ONLY: &str = "readonly";
pub const HIDDEN: &str = "hidden";
pub const EXECUTABLE: &str = "executable";
pub const ADVANCED: &str = "advanced";
pub const CHANGE: &str = "change";
pub const COUNT: &str = "count";
pub const SET_ICON: &str = "edit-copy";

pub const SHEET_W: i32 = 361;
const LABEL_X: i32 = 21;
const FILE_VALUE_X: i32 = 97;
const FOLDER_VALUE_X: i32 = 94;
const SEP_X: i32 = 21;
const SEP_RIGHT: i32 = 339;
const FIELD: Rect = Rect::new(94, 48, 246, 23);
const ICON_Y: i32 = 41;
pub const ICON_PX: i32 = 32;
const FIELD_TO_SEP: i32 = 11;
const SEP_TO_ROW: i32 = 13;
const ROW_PITCH: i32 = 26;
const APP_ICON_X: i32 = 95;
const APP_NAME_X: i32 = 120;
const ROW_BUTTON_W: i32 = 105;
const ROW_BUTTON_RIGHT: i32 = 339;
const FILE_ADVANCED_RIGHT: i32 = 340;
const ROW_BUTTON_DY: i32 = 3;
const CHECK_X: i32 = 94;
const CHECK_DY: i32 = 1;
const CHECK_PITCH: i32 = 23;
const FILE_HIDDEN_X: i32 = 174;
const TAIL_INLINE: i32 = 100;
const TAIL_STACKED: i32 = 111;
const PAGE_BOTTOM: i32 = 36;
const SET_SHEET_H: i32 = FOLDER_SHEET_H;

const SEP_IDS: [&str; 4] = ["sep0", "sep1", "sep2", "sep3"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Subject {
    File,
    Folder,
    Set { has_dir: bool },
}

impl Subject {
    pub fn single(is_dir: bool) -> Subject {
        if is_dir {
            Subject::Folder
        } else {
            Subject::File
        }
    }

    pub fn reaches_folders(self) -> bool {
        matches!(self, Subject::Folder | Subject::Set { has_dir: true })
    }

    pub fn font(self) -> DialogFont {
        match self {
            Subject::Folder | Subject::Set { has_dir: false } => DialogFont::ShellDlg,
            Subject::File | Subject::Set { has_dir: true } => DialogFont::Ui,
        }
    }

    fn value_x(self) -> i32 {
        match self {
            Subject::Folder | Subject::Set { has_dir: false } => FOLDER_VALUE_X,
            Subject::File | Subject::Set { has_dir: true } => FILE_VALUE_X,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    Header,
    Count {
        text: String,
    },
    Separator,
    Pair {
        ids: (&'static str, &'static str),
        label: &'static str,
        value: String,
        /// The value may run onto a second line, inside its row's pitch: a long date.
        wraps: bool,
    },
    OpensWith {
        name: String,
    },
    /// "Attributes:" with Read-only and Hidden, and Advanced... A box
    /// that starts indeterminate has three states. A file's two boxes
    /// share a line; a folder's and a set's stack. A folder's Read-only
    /// starts indeterminate, since the bit means nothing for the folder
    /// itself. A file's and a folder's Executable goes on a line of its
    /// own below them; a set has none.
    Attributes {
        read_only: CheckState,
        hidden: CheckState,
        executable: Option<Executable>,
        subject: Subject,
    },
}

/// The Executable box: ticked, and whether it takes a click.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Executable {
    pub on: bool,
    pub enabled: bool,
}

fn button(id: &'static str, rect: Rect, text: &str, enabled: bool) -> Item {
    Item::button(id, rect, text, enabled, false)
}

fn check(id: &'static str, x: i32, y: i32, label: &str, state: CheckState) -> Item {
    check_enabled(id, x, y, label, state, true)
}

fn check_enabled(
    id: &'static str,
    x: i32,
    y: i32,
    label: &str,
    state: CheckState,
    enabled: bool,
) -> Item {
    Item::new(
        id,
        Rect::new(x, y, CHECK, CHECK),
        Control::Check {
            label: label.to_string(),
            checked: state == CheckState::On,
            enabled,
            mixed: (state == CheckState::Mixed).then_some(true),
        },
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Summary,
    NoSummary,
    Sharing,
    Compatibility,
    Customize,
    Shortcut,
    WebDocument,
}

impl Tab {
    pub fn label(self) -> &'static str {
        match self {
            Tab::Summary | Tab::NoSummary => "Summary",
            Tab::Sharing => "Sharing",
            Tab::Compatibility => "Compatibility",
            Tab::Customize => "Customize",
            Tab::Shortcut => "Shortcut",
            Tab::WebDocument => "Web Document",
        }
    }

    pub fn height(self) -> i32 {
        match self {
            Tab::Summary | Tab::NoSummary => SUMMARY_SHEET_H,
            Tab::Shortcut | Tab::WebDocument => SHORTCUT_SHEET_H,
            Tab::Compatibility => COMPAT_SHEET_H,
            Tab::Sharing | Tab::Customize => FOLDER_SHEET_H,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Over {
    File {
        shortcut: bool,
    },
    Launcher,
    Symlink {
        runs: bool,
    },
    WebLink,
    Folder,
    Set {
        document: bool,
        shortcut: bool,
        folder: bool,
    },
}

pub fn sheet_tabs(over: Over) -> Vec<Tab> {
    match over {
        Over::File { shortcut: false } => vec![Tab::Summary],
        Over::File { shortcut: true } => vec![],
        Over::Launcher | Over::Symlink { runs: true } => vec![Tab::Shortcut, Tab::Compatibility],
        Over::Symlink { runs: false } => vec![Tab::Shortcut],
        Over::WebLink => vec![Tab::WebDocument],
        Over::Folder => vec![Tab::Sharing, Tab::Customize],
        Over::Set {
            document,
            shortcut,
            folder,
        } => {
            let mut tabs = Vec::new();
            if shortcut {
                tabs.push(Tab::Compatibility);
            }
            match (shortcut, document || folder) {
                (true, true) => tabs.push(Tab::NoSummary),
                (false, true) if document => tabs.push(Tab::Summary),
                _ => {}
            }
            if folder && !document && !shortcut {
                tabs.push(Tab::Customize);
            }
            tabs
        }
    }
}

pub fn sheet(
    general: Form,
    height: i32,
    tabs: &[Tab],
    mut own: impl FnMut(Tab) -> Form,
) -> crate::controls::propsheet::PropSheet {
    let mut labels = vec!["General"];
    let mut pages = vec![general];
    let mut h = height;
    for &tab in tabs {
        labels.push(tab.label());
        pages.push(match tab {
            Tab::Summary | Tab::Shortcut | Tab::WebDocument => own(tab),
            Tab::NoSummary => no_summary_page(),
            Tab::Sharing => sharing_page(None, None),
            Tab::Customize => customize_page(),
            Tab::Compatibility => compat_page(),
        });
        h = h.max(tab.height());
    }
    let mut s = crate::controls::propsheet::PropSheet::new(&labels, pages, (SHEET_W, h));
    if let Some(k) = tabs
        .iter()
        .position(|&t| matches!(t, Tab::Shortcut | Tab::WebDocument))
    {
        s.open_tab(k + 1);
    }
    s
}

pub const PAGE_ICONS: [(&str, &str); 2] = [
    ("sh_local", "computer"),
    ("sh_network", "network-workgroup"),
];

pub fn fill_pictures(
    sheet: &mut crate::controls::propsheet::PropSheet,
    mut get: impl FnMut(&str) -> Option<Canvas>,
) {
    for page in &mut sheet.pages {
        for (id, name) in PAGE_ICONS {
            if let Some(it) = page.item_mut(id)
                && let Control::Picture { canvas, .. } = &mut it.control
            {
                *canvas = get(name);
            }
        }
    }
}

pub const FOLDER_SHEET_H: i32 = 424;
pub const SUMMARY_SHEET_H: i32 = 478;
pub const COMPAT_SHEET_H: i32 = 437;
pub const SHORTCUT_SHEET_H: i32 = 478;

#[cfg(test)]
mod tests;
