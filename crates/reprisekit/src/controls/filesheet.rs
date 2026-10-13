//! General properties page for a file, folder or set, plus the Advanced Attributes
//! and Confirm Attribute Changes boxes. All coordinates are client coordinates.

use crate::classic::widgets::{BUTTON_GAP, BUTTON_H, BUTTON_W, CheckState, LINE_H};
use crate::controls::form::{CHECK, Control, Form, Item};
use crate::raster::{Canvas, Rect};
use crate::text::{DialogFont, TextRenderer};
use crate::textedit::TextEdit;
use crate::theme::TextUnits;

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
pub const READ_ONLY: &str = "readonly";
pub const HIDDEN: &str = "hidden";
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
    /// itself.
    Attributes {
        read_only: CheckState,
        hidden: CheckState,
        subject: Subject,
    },
}

fn tick(on: bool) -> CheckState {
    if on { CheckState::On } else { CheckState::Off }
}

pub fn rows(facts: &Facts) -> Vec<Row> {
    let pair = |ids, label, value: String| Row::Pair {
        ids,
        label,
        value,
        wraps: false,
    };
    let date = |ids, label, value: String| Row::Pair {
        ids,
        label,
        value,
        wraps: true,
    };
    let mut rows = vec![
        Row::Header,
        Row::Separator,
        pair(
            ("l_type", "v_type"),
            if facts.is_dir {
                "Type:"
            } else {
                "Type of file:"
            },
            facts.type_name.clone(),
        ),
    ];
    if let Some(app) = &facts.opens_with {
        rows.push(Row::OpensWith { name: app.clone() });
    } else if let Some(d) = &facts.description {
        rows.push(pair(("l_desc", "v_desc"), "Description:", d.clone()));
    }
    if !facts.is_dir {
        rows.push(Row::Separator);
    }
    rows.push(pair(
        ("l_loc", "v_loc"),
        "Location:",
        facts.location.clone(),
    ));
    rows.push(pair(("l_size", "v_size"), "Size:", facts.size.clone()));
    rows.push(pair(
        ("l_disk", "v_disk"),
        "Size on disk:",
        facts.on_disk.clone(),
    ));
    if let Some(has) = &facts.contains {
        rows.push(pair(("l_has", "v_has"), "Contains:", has.clone()));
    }
    rows.push(Row::Separator);
    if let Some(created) = &facts.created {
        rows.push(date(("l_made", "v_made"), "Created:", created.clone()));
    }
    if !facts.is_dir {
        rows.push(date(
            ("l_mod", "v_mod"),
            "Modified:",
            facts.modified.clone(),
        ));
        rows.push(date(
            ("l_acc", "v_acc"),
            "Accessed:",
            facts.accessed.clone(),
        ));
    }
    rows.push(Row::Separator);
    rows.push(Row::Attributes {
        read_only: if facts.is_dir {
            CheckState::Mixed
        } else {
            tick(facts.read_only)
        },
        hidden: tick(facts.hidden),
        subject: if facts.is_dir {
            Subject::Folder
        } else {
            Subject::File
        },
    });
    rows
}

pub fn set_rows(facts: &SetFacts) -> Vec<Row> {
    let pair = |ids, label, value: &String| Row::Pair {
        ids,
        label,
        value: value.clone(),
        wraps: false,
    };
    vec![
        Row::Count {
            text: facts.count.clone(),
        },
        Row::Separator,
        pair(("l_type", "v_type"), "Type:", &facts.type_text),
        pair(("l_loc", "v_loc"), "Location:", &facts.location),
        pair(("l_size", "v_size"), "Size:", &facts.size),
        pair(("l_disk", "v_disk"), "Size on disk:", &facts.on_disk),
        Row::Separator,
        Row::Attributes {
            read_only: facts.read_only,
            hidden: facts.hidden,
            subject: Subject::Set {
                has_dir: facts.has_dir,
            },
        },
    ]
}

fn button(id: &'static str, rect: Rect, text: &str, enabled: bool) -> Item {
    Item::button(id, rect, text, enabled, false)
}

fn check(id: &'static str, x: i32, y: i32, label: &str, state: CheckState) -> Item {
    Item::new(
        id,
        Rect::new(x, y, CHECK, CHECK),
        Control::Check {
            label: label.to_string(),
            checked: state == CheckState::On,
            enabled: true,
            mixed: (state == CheckState::Mixed).then_some(true),
        },
    )
}

pub fn general_page(
    rows: &[Row],
    name: &str,
    icon: Option<Canvas>,
    app: Option<Canvas>,
) -> (Form, i32) {
    let subject = rows
        .iter()
        .find_map(|r| match r {
            Row::Attributes { subject, .. } => Some(*subject),
            _ => None,
        })
        .unwrap_or(Subject::File);
    let value_x = subject.value_x();
    let mut items = Vec::new();
    let mut y = 0;
    let mut seps = 0;
    let mut height = 0;
    for row in rows {
        match row {
            Row::Header | Row::Count { .. } => {
                items.push(Item::picture(
                    "icon",
                    Rect::new(LABEL_X, ICON_Y, ICON_PX, ICON_PX),
                    icon.clone(),
                ));
                items.push(match row {
                    Row::Count { text } => {
                        Item::label_at(COUNT, FIELD.x, FIELD.y, SEP_RIGHT - FIELD.x, text)
                    }
                    _ => Item::new(
                        NAME,
                        FIELD,
                        Control::Edit {
                            edit: TextEdit::new(name, true),
                            first: std::cell::Cell::new(0),
                            enabled: true,
                        },
                    ),
                });
                y = FIELD.bottom() + FIELD_TO_SEP;
            }
            Row::Separator => {
                let id = SEP_IDS[seps.min(SEP_IDS.len() - 1)];
                seps += 1;
                items.push(Item::new(
                    id,
                    Rect::new(SEP_X, y, SEP_RIGHT - SEP_X, 2),
                    Control::Separator,
                ));
                y += SEP_TO_ROW;
            }
            Row::Pair {
                ids,
                label: l,
                value,
                wraps,
            } => {
                items.push(Item::label_at(ids.0, LABEL_X, y, value_x - LABEL_X, l));
                let w = SEP_RIGHT - value_x;
                items.push(if *wraps {
                    Item::lines(ids.1, Rect::new(value_x, y, w, ROW_PITCH), &[value])
                } else {
                    Item::label_at(ids.1, value_x, y, w, value)
                });
                y += ROW_PITCH;
            }
            Row::OpensWith { name } => {
                items.push(Item::label_at(
                    "l_opens",
                    LABEL_X,
                    y,
                    value_x - LABEL_X,
                    "Opens with:",
                ));
                items.push(Item::picture(
                    "opens_icon",
                    Rect::new(APP_ICON_X, y, 16, 16),
                    app.clone(),
                ));
                let change_x = ROW_BUTTON_RIGHT - ROW_BUTTON_W;
                items.push(Item::label_at(
                    "v_opens",
                    APP_NAME_X,
                    y,
                    change_x - BUTTON_GAP - APP_NAME_X,
                    name,
                ));
                items.push(button(
                    CHANGE,
                    Rect::new(change_x, y - ROW_BUTTON_DY, ROW_BUTTON_W, BUTTON_H),
                    "&Change...",
                    true,
                ));
                y += ROW_PITCH;
            }
            Row::Attributes {
                read_only,
                hidden,
                subject,
            } => {
                // A set with a folder in it drops the label's colon.
                let label = match subject {
                    Subject::Set { has_dir: true } => "Attributes",
                    _ => "Attributes:",
                };
                items.push(Item::label_at(
                    "l_attr",
                    LABEL_X,
                    y,
                    value_x - LABEL_X,
                    label,
                ));
                let by = y + CHECK_DY;
                items.push(check(READ_ONLY, CHECK_X, by, "&Read-only", *read_only));
                // Hidden stands for the dot in front of the name: a host
                // renames the file to set or clear it.
                let (hx, hy) = match subject {
                    Subject::File => (FILE_HIDDEN_X, by),
                    _ => (CHECK_X, by + CHECK_PITCH),
                };
                items.push(check(HIDDEN, hx, hy, "&Hidden", *hidden));
                let (right, w) = match subject {
                    Subject::Folder => (ROW_BUTTON_RIGHT, ROW_BUTTON_W),
                    _ => (FILE_ADVANCED_RIGHT, BUTTON_W),
                };
                items.push(button(
                    ADVANCED,
                    Rect::new(right - w, y - ROW_BUTTON_DY, w, BUTTON_H),
                    "A&dvanced...",
                    true,
                ));
                height = match subject {
                    Subject::File => y + TAIL_INLINE + PAGE_BOTTOM,
                    Subject::Folder => y + TAIL_STACKED + PAGE_BOTTOM,
                    Subject::Set { .. } => SET_SHEET_H,
                };
            }
        }
    }
    let mut form = Form::new(items).set_in(subject.font());
    form.focus_id(READ_ONLY);
    (form, height)
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

pub const SUMMARY_IDS: [&str; 6] = [
    "sum_title",
    "sum_subject",
    "sum_author",
    "sum_category",
    "sum_keywords",
    "sum_comments",
];
pub const SUMMARY_ADVANCED: &str = "sum_advanced";
const SUMMARY_LABELS: [&str; 6] = [
    "&Title:",
    "&Subject:",
    "A&uthor:",
    "&Category:",
    "&Keywords:",
    "Co&mments:",
];
const SUM_LABEL_X: i32 = 22;
const SUM_FIELD_X: i32 = 81;
const SUM_FIELD_W: i32 = 263;
const SUM_LABEL_DY: i32 = 3;
const SUM_ROWS: [i32; 5] = [41, 72, 103, 152, 183];
const SUM_FIELD_H: i32 = 21;
const SUM_COMMENTS: Rect = Rect::new(81, 214, 263, 80);
const SUM_SEP: Rect = Rect::new(22, 137, 318, 2);
const SUM_ADVANCED: Rect = Rect::new(244, 305, 99, BUTTON_H);

pub fn summary_page(values: Option<&[String; 6]>) -> Form {
    let live = values.is_some();
    let value = |k: usize| values.map_or("", |v| v[k].as_str());
    let mut items = Vec::new();
    for (k, &y) in SUM_ROWS.iter().enumerate() {
        items.push(Item::label_at(
            ["sum_l0", "sum_l1", "sum_l2", "sum_l3", "sum_l4"][k],
            SUM_LABEL_X,
            y + SUM_LABEL_DY,
            SUM_FIELD_X - SUM_LABEL_X,
            SUMMARY_LABELS[k],
        ));
        items.push(Item::edit(
            SUMMARY_IDS[k],
            Rect::new(SUM_FIELD_X, y, SUM_FIELD_W, SUM_FIELD_H),
            value(k),
            live,
        ));
        if k == 2 {
            items.push(Item::new("sum_sep", SUM_SEP, Control::Separator));
        }
    }
    items.push(Item::label_at(
        "sum_l5",
        SUM_LABEL_X,
        SUM_COMMENTS.y + SUM_LABEL_DY,
        SUM_FIELD_X - SUM_LABEL_X,
        SUMMARY_LABELS[5],
    ));
    items.push(Item::memo(SUMMARY_IDS[5], SUM_COMMENTS, value(5), live));
    items.push(button(
        SUMMARY_ADVANCED,
        SUM_ADVANCED,
        "Ad&vanced >>",
        false,
    ));
    Form::new(items)
}

pub fn summary_values(page: &Form) -> [String; 6] {
    SUMMARY_IDS.map(|id| match page.item(id).map(|i| &i.control) {
        Some(Control::Edit { edit, .. }) => edit.text.clone(),
        Some(Control::Memo(m)) => m.edit.text(),
        _ => String::new(),
    })
}

pub fn summary_moves(page: &Form, opened: &[String; 6]) -> Vec<(usize, String)> {
    summary_values(page)
        .into_iter()
        .enumerate()
        .filter(|(k, now)| *now != opened[*k])
        .collect()
}

pub fn summary_of(pages: &[Form]) -> Option<&Form> {
    pages.iter().find(|p| p.item(SUMMARY_IDS[0]).is_some())
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShortcutFacts {
    pub name: String,
    pub target_type: String,
    pub target_location: String,
    pub symlink: bool,
    pub fields: [String; 3],
}

pub const SHORTCUT_IDS: [&str; 3] = ["sc_target", "sc_start", "sc_comment"];
pub const FIND_TARGET: &str = "sc_find";
const SC_ICON: Rect = Rect::new(21, 42, ICON_PX, ICON_PX);
const SC_NAME: (i32, i32) = (87, 49);
const SC_VALUE_X: i32 = 100;
const SC_SEPS: [(i32, i32); 2] = [(80, SEP_RIGHT), (173, SEP_RIGHT + 1)];
const SC_ROWS: [i32; 2] = [92, 118];
const SC_FIELDS: [i32; 5] = [142, 186, 217, 248, 277];
const SC_FIELD_W: i32 = 240;
const SC_FIELD_H: i32 = 23;
const SC_LABEL_DY: i32 = 2;
const SC_BUTTONS: (i32, i32) = (58, 306);
const SC_BUTTON_W: i32 = 90;
const SC_BUTTON_PITCH: i32 = 96;

/// Greyed on every shortcut, each drawn with no state of its own:
/// * Shortcut key: a key that starts the launcher would be a
///   `[[keybinds]]` entry in the compositor's config naming the file by
///   path, which a rename or a move of the file leaves pointing nowhere,
///   so it reads "None".
/// * Run: nothing in a desktop entry says how the window opens, and a
///   program places its own first window; it reads "Normal window".
/// * Advanced...: its one choice is running as another user, and a
///   program started as another user cannot reach the session's display.
pub fn shortcut_page(facts: &ShortcutFacts, icon: Option<Canvas>) -> Form {
    let label_w = SC_VALUE_X - LABEL_X;
    let value_w = SC_FIELD_W;
    let field = |k: usize| Rect::new(SC_VALUE_X, SC_FIELDS[k], SC_FIELD_W, SC_FIELD_H);
    let label = |id: &'static str, k: usize, text: &str| {
        Item::label_at(id, LABEL_X, SC_FIELDS[k] + SC_LABEL_DY, label_w, text)
    };
    let edit = |id: &'static str, k: usize, value: &str, selected: bool, enabled: bool| {
        Item::new(
            id,
            field(k),
            Control::Edit {
                edit: TextEdit::new(value, selected),
                first: std::cell::Cell::new(0),
                enabled,
            },
        )
    };
    let own = !facts.symlink;
    let sep = |id: &'static str, (y, right): (i32, i32)| {
        Item::new(
            id,
            Rect::new(SEP_X, y, right - SEP_X, 2),
            Control::Separator,
        )
    };
    let mut run = Item::combo(
        "sc_run",
        SC_VALUE_X,
        SC_FIELDS[3],
        SC_FIELD_W,
        &["Normal window", "Minimized", "Maximized"],
        3,
    );
    if let Control::Combo(c) = &mut run.control {
        c.enabled = false;
    }
    let (bx, by) = SC_BUTTONS;
    let sc_button = |id: &'static str, k: i32, text: &str, enabled: bool| {
        button(
            id,
            Rect::new(bx + k * SC_BUTTON_PITCH, by, SC_BUTTON_W, BUTTON_H),
            text,
            enabled,
        )
    };
    let [target, start, comment] = &facts.fields;
    let mut form = Form::new(vec![
        Item::picture("icon", SC_ICON, icon),
        Item::label_at("sc_name", SC_NAME.0, SC_NAME.1, value_w, &facts.name),
        sep("sc_sep0", SC_SEPS[0]),
        Item::label_at("sc_type_l", LABEL_X, SC_ROWS[0], label_w, "Target type:"),
        Item::label_at(
            "sc_type",
            SC_VALUE_X,
            SC_ROWS[0],
            value_w,
            &facts.target_type,
        ),
        Item::label_at(
            "sc_where_l",
            LABEL_X,
            SC_ROWS[1],
            label_w,
            "Target location:",
        ),
        Item::label_at(
            "sc_where",
            SC_VALUE_X,
            SC_ROWS[1],
            value_w,
            &facts.target_location,
        ),
        label("sc_target_l", 0, "&Target:"),
        edit(SHORTCUT_IDS[0], 0, target, true, true),
        sep("sc_sep1", SC_SEPS[1]),
        label("sc_start_l", 1, "&Start in:").enabled(own),
        edit(SHORTCUT_IDS[1], 1, start, false, own),
        label("sc_key_l", 2, "Shortcut &key:").enabled(false),
        Item::new(
            "sc_key",
            field(2),
            Control::HotKey(crate::controls::hotkey::HotKey::new(None)),
        )
        .enabled(false),
        label("sc_run_l", 3, "&Run:").enabled(false),
        run,
        label("sc_comment_l", 4, "C&omment:").enabled(own),
        edit(SHORTCUT_IDS[2], 4, comment, false, own),
        sc_button(FIND_TARGET, 0, "&Find Target...", true),
        sc_button(CHANGE_ICON, 1, "&Change Icon...", own),
        sc_button("sc_advanced", 2, "A&dvanced...", false),
    ]);
    form.focus_id(SHORTCUT_IDS[0]);
    form
}

pub fn shortcut_values(page: &Form) -> [String; 3] {
    SHORTCUT_IDS.map(|id| match page.item(id).map(|i| &i.control) {
        Some(Control::Edit { edit, .. }) => edit.text.clone(),
        _ => String::new(),
    })
}

pub fn shortcut_moves(page: &Form, opened: &[String; 3]) -> Vec<(usize, String)> {
    shortcut_values(page)
        .into_iter()
        .enumerate()
        .filter(|(k, now)| *now != opened[*k])
        .collect()
}

pub fn shortcut_of(pages: &[Form]) -> Option<&Form> {
    pages.iter().find(|p| p.item(SHORTCUT_IDS[0]).is_some())
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WebFacts {
    pub name: String,
    pub url: String,
}

const WD_ICON: Rect = Rect::new(21, 41, ICON_PX, ICON_PX);
const WD_NAME: (i32, i32) = (64, 49);
const WD_SEPS: [i32; 2] = [85, 184];
const WD_FIELD_X: i32 = 96;
const WD_FIELDS: [i32; 2] = [98, 126];
const WD_FIELD_W: i32 = 245;
const WD_LABEL_DY: i32 = 5;
const WD_VISITS_Y: i32 = 160;
const WD_VALUE_X: i32 = WD_FIELD_X;
const WD_CHANGE_ICON: Rect = Rect::new(250, 194, SC_BUTTON_W, BUTTON_H);

/// Shortcut key is greyed as it is on the Shortcut page ([`shortcut_page`]).
/// Visits reads "Unknown": nothing keeps a count of the times a page was
/// opened.
pub fn web_page(facts: &WebFacts, icon: Option<Canvas>) -> Form {
    let label_w = WD_FIELD_X - LABEL_X;
    let field = |k: usize| Rect::new(WD_FIELD_X, WD_FIELDS[k], WD_FIELD_W, SC_FIELD_H);
    let label = |id: &'static str, k: usize, text: &str| {
        Item::label_at(id, LABEL_X, WD_FIELDS[k] + WD_LABEL_DY, label_w, text)
    };
    let sep = |id: &'static str, y: i32| {
        Item::new(
            id,
            Rect::new(SEP_X, y, SEP_RIGHT - SEP_X, 2),
            Control::Separator,
        )
    };
    let mut form = Form::new(vec![
        Item::picture("icon", WD_ICON, icon),
        Item::label_at(
            "wd_name",
            WD_NAME.0,
            WD_NAME.1,
            SEP_RIGHT - WD_NAME.0,
            &facts.name,
        ),
        sep("wd_sep0", WD_SEPS[0]),
        label("wd_url_l", 0, "&URL:"),
        Item::new(
            SHORTCUT_IDS[0],
            field(0),
            Control::Edit {
                edit: TextEdit::new(&facts.url, true),
                first: std::cell::Cell::new(0),
                enabled: true,
            },
        ),
        label("wd_key_l", 1, "Shortcut &key:").enabled(false),
        Item::new(
            "wd_key",
            field(1),
            Control::HotKey(crate::controls::hotkey::HotKey::new(None)),
        )
        .enabled(false),
        Item::label_at("wd_visits_l", LABEL_X, WD_VISITS_Y, label_w, "Visits:"),
        Item::label_at(
            "wd_visits",
            WD_VALUE_X,
            WD_VISITS_Y,
            SEP_RIGHT - WD_VALUE_X,
            "Unknown",
        ),
        sep("wd_sep1", WD_SEPS[1]),
        button(CHANGE_ICON, WD_CHANGE_ICON, "&Change Icon...", true),
    ]);
    form.focus_id(SHORTCUT_IDS[0]);
    form
}

pub const CHANGE_ICON: &str = "cu_change_icon";
pub const CUSTOM_ICON: &str = "cu_icon";
pub const CUSTOM_PREVIEW: &str = "cu_preview";
pub const PREVIEW_PX: i32 = 96;

/// Greyed: the template list and "Also apply this template to all
/// subfolders" stand for a view per kind of folder, which nothing here
/// keeps, so the list shows no template and the box no tick; Choose
/// Picture... and Restore Default, as the picture Thumbnails view would
/// put on the folder is not kept either.
pub fn customize_page() -> Form {
    let mut items = vec![
        Item::group(
            "cu_g0",
            Rect::new(21, 47, 319, 90),
            "What kind of folder do you want?",
        ),
        Item::label_at(
            "cu_template_l",
            32,
            60,
            290,
            "Use this folder type as a &template:",
        )
        .enabled(false),
    ];
    let mut template = Item::combo("cu_template", 31, 80, 299, &[""], 1);
    if let Control::Combo(c) = &mut template.control {
        c.enabled = false;
    }
    items.push(template);
    items.push(
        Item::check(
            "cu_all",
            31,
            113,
            "Also apply this template to all &subfolders",
        )
        .enabled(false),
    );
    items.push(Item::group(
        "cu_g1",
        Rect::new(21, 151, 319, 134),
        "Folder pictures",
    ));
    items.push(Item::lines_at(
        "cu_pic_text",
        32,
        162,
        185,
        &[
            "For Thumbnails view, you can put a",
            "picture on this folder to remind you of",
            "the contents.",
        ],
    ));
    items.push(Item::label_at("cu_preview_l", 226, 162, 100, "Preview:"));
    items.push(Item::picture(
        CUSTOM_PREVIEW,
        Rect::new(225, 181, PREVIEW_PX, PREVIEW_PX),
        None,
    ));
    items.push(button(
        "cu_choose",
        Rect::new(31, 222, 98, BUTTON_H),
        "Choose &Picture...",
        false,
    ));
    items.push(button(
        "cu_restore",
        Rect::new(31, 251, 98, BUTTON_H),
        "&Restore Default",
        false,
    ));
    items.push(Item::group(
        "cu_g2",
        Rect::new(21, 299, 319, 78),
        "Folder icons",
    ));
    items.push(Item::lines_at(
        "cu_icon_text",
        32,
        312,
        300,
        &[
            "For all views except Thumbnails, you can change the standard",
            "\"folder\" icon to another icon.",
        ],
    ));
    items.push(button(
        CHANGE_ICON,
        Rect::new(31, 345, 98, BUTTON_H),
        "Change &Icon...",
        true,
    ));
    items.push(Item::picture(
        CUSTOM_ICON,
        Rect::new(251, 343, ICON_PX, ICON_PX),
        None,
    ));
    Form::new(items)
}

pub const CHANGE_ICON_SIZE: (i32, i32) = (290, 326);
pub const CHANGE_ICON_LIST: &str = "ci_list";
pub const CHANGE_ICON_OK: &str = "ci_ok";
pub const CHANGE_ICON_CANCEL: &str = "ci_cancel";
pub const CHANGE_ICON_RESTORE: &str = "ci_restore";
/// The standard folder's icon, which Restore Defaults picks and which
/// writes no icon of the folder's own.
pub const STANDARD_FOLDER_ICON: &str = "folder";

pub const FOLDER_ICON_NAMES: &[&str] = &[
    "folder",
    "folder-documents",
    "folder-download",
    "folder-music",
    "folder-pictures",
    "folder-videos",
    "folder-publicshare",
    "folder-templates",
    "folder-remote",
    "user-home",
    "user-desktop",
    "user-trash",
    "computer",
    "network-workgroup",
    "drive-harddisk",
    "drive-optical",
    "drive-removable-media",
    "media-flash",
    "applications-multimedia",
    "applications-graphics",
    "applications-internet",
    "applications-office",
    "applications-games",
    "applications-development",
    "applications-system",
    "preferences-desktop",
    "help-browser",
    "emblem-favorite",
    "emblem-important",
    "text-x-generic",
    "image-x-generic",
    "audio-x-generic",
    "video-x-generic",
    "package-x-generic",
    "system-shutdown",
];

pub fn change_icon_form(
    source: &str,
    mut icons: crate::controls::iconlist::IconList,
    restore: bool,
) -> Form {
    let cm = crate::controls::metrics::ControlMetrics::CLASSIC;
    let list_rect = Rect::new(11, 77, 267, 198);
    icons.follow(&crate::controls::iconlist::layout(list_rect, &cm));
    let mut items = vec![
        Item::label_at("ci_file_l", 11, 11, 270, "&Look for icons in this file:").enabled(false),
        Item::edit("ci_file", Rect::new(11, 29, 185, 21), source, false),
        button(
            "ci_browse",
            Rect::new(203, 29, BUTTON_W, BUTTON_H),
            "&Browse...",
            false,
        ),
        Item::label_at(
            "ci_list_l",
            11,
            59,
            270,
            "&Select an icon from the list below:",
        ),
        Item::new(
            CHANGE_ICON_LIST,
            list_rect,
            Control::IconList(Box::new(icons)),
        ),
        button(
            CHANGE_ICON_RESTORE,
            Rect::new(11, 292, 105, BUTTON_H),
            "&Restore Defaults",
            true,
        ),
        Item::button(
            CHANGE_ICON_OK,
            Rect::new(122, 292, BUTTON_W, BUTTON_H),
            "OK",
            true,
            true,
        ),
        button(
            CHANGE_ICON_CANCEL,
            Rect::new(203, 292, BUTTON_W, BUTTON_H),
            "Cancel",
            true,
        ),
    ];
    if !restore {
        items.retain(|i| i.id != CHANGE_ICON_RESTORE);
    }
    let mut form = Form::new(items);
    form.focus_id(CHANGE_ICON_LIST);
    form
}

pub fn change_icon_picked(form: &Form) -> Option<String> {
    match form.item(CHANGE_ICON_LIST).map(|i| &i.control) {
        Some(Control::IconList(l)) => l.picked().map(str::to_string),
        _ => None,
    }
}

pub fn change_icon_restore(form: &mut Form) {
    let cm = form.controls();
    let Some(it) = form.item_mut(CHANGE_ICON_LIST) else {
        return;
    };
    let lay = crate::controls::iconlist::layout(it.placed(&cm), &cm);
    if let Control::IconList(l) = &mut it.control {
        l.pick(STANDARD_FOLDER_ICON);
        l.follow(&lay);
    }
}

/// A greyed paragraph broken into `lines` at `(x, y)`, `w` wide: it rewraps when a line is wider.
fn greyed_note(
    items: &mut Vec<Item>,
    id: &'static str,
    (x, y, w): (i32, i32, i32),
    lines: &[&str],
) {
    items.push(Item::lines_at(id, x, y, w, lines).enabled(false));
}

pub fn no_summary_page() -> Form {
    Form::new(vec![
        Item::lines_at(
            "ns_text",
            22,
            44,
            310,
            &[
                "There are no summary properties in common among the selected",
                "sources.",
            ],
        ),
        button(
            "ns_simple",
            Rect::new(244, 410, 99, BUTTON_H),
            "<< &Simple",
            false,
        ),
    ])
}

const SHARE_X: i32 = 71;
const SHARE_W: i32 = 262;

/// private". Nothing on it maps to anything here: a folder is shared
/// with other users by its permissions and on the network by a file
/// server such as Samba, which Reprise does not set up, so the check
/// box shows no state.
pub fn sharing_page(local: Option<Canvas>, network: Option<Canvas>) -> Form {
    let mut items = vec![
        Item::group(
            "sh_g0",
            Rect::new(21, 43, 318, 108),
            "Local sharing and security",
        ),
        Item::picture("sh_local", Rect::new(31, 54, ICON_PX, ICON_PX), local),
    ];
    greyed_note(
        &mut items,
        "sh_a0",
        (SHARE_X, 53, SHARE_W),
        &[
            "To share this folder with other users of this computer",
            "only, drag it to the Public folder.",
        ],
    );
    greyed_note(
        &mut items,
        "sh_b0",
        (SHARE_X, 92, SHARE_W),
        &[
            "To make this folder and its subfolders private,",
            "select the following check box.",
        ],
    );
    items.push(Item::check("sh_private", 70, 129, "&Make this folder private").enabled(false));
    items.push(Item::group(
        "sh_g1",
        Rect::new(21, 161, 318, 150),
        "Network sharing and security",
    ));
    items.push(Item::picture(
        "sh_network",
        Rect::new(31, 171, ICON_PX, ICON_PX),
        network,
    ));
    greyed_note(
        &mut items,
        "sh_c0",
        (SHARE_X, 170, SHARE_W),
        &[
            "This computer shares no folders on the network.",
            "A file server such as Samba, set up outside",
            "Reprise, shares folders.",
        ],
    );
    greyed_note(
        &mut items,
        "sh_d0",
        (SHARE_X, 237, SHARE_W),
        &[
            "If you understand the security risks, add this folder",
            "to the file server's shares yourself.",
        ],
    );
    greyed_note(
        &mut items,
        "sh_more",
        (29, 287, 300),
        &["Learn more about sharing and security."],
    );
    Form::new(items)
}

/// The Compatibility page of a set with a shortcut in it, greyed: the
/// paragraph, the compatibility mode box and its list, the display and
/// input boxes. Programs here run on the system they were built for, so
/// nothing maps to them and every box shows no state.
pub fn compat_page() -> Form {
    let mut items = Vec::new();
    greyed_note(
        &mut items,
        "cp_l0",
        (22, 40, 310),
        &[
            "If you have problems with this program and it worked correctly",
            "on an earlier system, select the compatibility mode that",
            "matches that earlier system.",
        ],
    );
    items.push(Item::group(
        "cp_g0",
        Rect::new(21, 103, 315, 73),
        "Compatibility mode",
    ));
    items.push(
        Item::check(
            "cp_mode",
            37,
            124,
            "&Run this program in compatibility mode for:",
        )
        .enabled(false),
    );
    let mut mode = Item::combo("cp_list", 37, 144, 228, &[""], 1);
    if let Control::Combo(c) = &mut mode.control {
        c.enabled = false;
    }
    items.push(mode);
    items.push(Item::group(
        "cp_g1",
        Rect::new(21, 197, 315, 92),
        "Display settings",
    ));
    for (id, y, label) in [
        ("cp_256", 216, "Run in 256 colo&rs"),
        ("cp_640", 241, "Run in 640 x 480 screen resolutio&n"),
        ("cp_themes", 265, "Disable &visual themes"),
    ] {
        items.push(Item::check(id, 37, y, label).enabled(false));
    }
    items.push(Item::group(
        "cp_g2",
        Rect::new(21, 311, 315, 40),
        "Input settings",
    ));
    items.push(
        Item::check(
            "cp_text",
            37,
            327,
            "Turn off advanced text services for this program",
        )
        .enabled(false),
    );
    greyed_note(
        &mut items,
        "cp_more",
        (22, 365, 300),
        &["Learn more about program compatibility."],
    );
    Form::new(items)
}

/// What the General page of an item in the Recycle Bin says, already
/// worded. Nothing on it can be changed: the item is renamed or edited
/// only once Restore has put it back.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BinFacts {
    pub type_name: String,
    pub origin: String,
    pub size: String,
    pub deleted: Option<String>,
    pub created: Option<String>,
    pub read_only: bool,
    pub hidden: bool,
}

pub const RESTORE: &str = "restore";
pub const BIN_SHEET_H: i32 = 418;
const BIN_ICON_Y: i32 = 40;
const BIN_NAME: (i32, i32) = (63, 47);
const BIN_VALUE_X: i32 = 85;
const BIN_DATE_X: i32 = 115;
const BIN_SEPS: [i32; 3] = [81, 166, 229];
const BIN_BLOCKS: [i32; 3] = [94, 179, 242];
const BIN_PITCH: i32 = 21;
const BIN_CHECK_X: [i32; 2] = [112, 204];
const BIN_CHECK_Y: [i32; 4] = [243, 262, 282, 301];
const BIN_RESTORE: Rect = Rect::new(265, 344, BUTTON_W, BUTTON_H);

/// Read-only and Hidden show the item's own state. Archive, Compressed
/// and Encrypt contents stand for attributes nothing maps to here, so
/// they are drawn unchecked.
pub fn bin_page(facts: &BinFacts, name: &str, icon: Option<Canvas>) -> Form {
    let mut items = vec![
        Item::picture(
            "icon",
            Rect::new(LABEL_X, BIN_ICON_Y, ICON_PX, ICON_PX),
            icon,
        ),
        Item::label_at(NAME, BIN_NAME.0, BIN_NAME.1, SEP_RIGHT - BIN_NAME.0, name),
    ];
    for (id, y) in SEP_IDS.iter().zip(BIN_SEPS) {
        items.push(Item::new(
            id,
            Rect::new(SEP_X, y, SEP_RIGHT - SEP_X, 2),
            Control::Separator,
        ));
    }
    let mut pair = |ids: (&'static str, &'static str), label, value: &str, x, y| {
        items.push(Item::label_at(ids.0, LABEL_X, y, x - LABEL_X, label));
        items.push(Item::label_at(ids.1, x, y, SEP_RIGHT - x, value));
    };
    let [first, second, attrs] = BIN_BLOCKS;
    pair(
        ("l_type", "v_type"),
        "Type:",
        &facts.type_name,
        BIN_VALUE_X,
        first,
    );
    pair(
        ("l_origin", "v_origin"),
        "Origin:",
        &facts.origin,
        BIN_VALUE_X,
        first + BIN_PITCH,
    );
    pair(
        ("l_size", "v_size"),
        "Size:",
        &facts.size,
        BIN_VALUE_X,
        first + 2 * BIN_PITCH,
    );
    let dates = [
        (("l_deleted", "v_deleted"), "Deleted:", &facts.deleted),
        (("l_made", "v_made"), "Created:", &facts.created),
    ];
    let mut y = second;
    for (ids, label, value) in dates {
        if let Some(v) = value {
            pair(ids, label, v, BIN_DATE_X, y);
            y += BIN_PITCH;
        }
    }
    items.push(Item::label_at(
        "l_attr",
        LABEL_X,
        attrs,
        BIN_CHECK_X[0] - LABEL_X,
        "Attributes",
    ));
    let [left, right] = BIN_CHECK_X;
    for (id, x, y, label, checked) in [
        (
            READ_ONLY,
            left,
            BIN_CHECK_Y[0],
            "&Read-only",
            facts.read_only,
        ),
        (HIDDEN, right, BIN_CHECK_Y[0], "&Hidden", facts.hidden),
        ("archive", left, BIN_CHECK_Y[1], "Archi&ve", false),
        ("compress", left, BIN_CHECK_Y[2], "Co&mpressed", false),
        ("encrypt", left, BIN_CHECK_Y[3], "&Encrypt contents", false),
    ] {
        items.push(Item::new(
            id,
            Rect::new(x, y, CHECK, CHECK),
            Control::Check {
                label: label.to_string(),
                checked,
                enabled: false,
                mixed: None,
            },
        ));
    }
    items.push(button(RESTORE, BIN_RESTORE, "Restore", true));
    Form::new(items)
}

pub fn typed_name(page: &Form) -> &str {
    match page.item(NAME).map(|i| &i.control) {
        Some(Control::Edit { edit, .. }) => &edit.text,
        _ => "",
    }
}

pub fn read_only(page: &Form) -> CheckState {
    page.check_state(READ_ONLY).unwrap_or_default()
}

pub fn hidden(page: &Form) -> bool {
    page.checked(HIDDEN).unwrap_or(false)
}

pub fn hidden_state(page: &Form) -> CheckState {
    page.check_state(HIDDEN).unwrap_or_default()
}

pub fn shared_state(on: Option<bool>) -> CheckState {
    match on {
        Some(true) => CheckState::On,
        Some(false) => CheckState::Off,
        None => CheckState::Mixed,
    }
}

pub fn set_moves(page: &Form, opened: (CheckState, CheckState)) -> (Option<bool>, Option<bool>) {
    let moved = |now: CheckState, was: CheckState| match now {
        CheckState::Mixed => None,
        now if now == was => None,
        now => Some(now.is_on()),
    };
    (
        moved(read_only(page), opened.0),
        moved(hidden_state(page), opened.1),
    )
}

pub fn shown_name(file_name: &str, ext: &str, hidden: bool) -> String {
    let name = file_name.strip_suffix(ext).unwrap_or(file_name);
    let name = if hidden {
        name.strip_prefix('.').unwrap_or(name)
    } else {
        name
    };
    name.to_string()
}

pub fn wanted_name(typed: &str, hidden: bool, ext: &str) -> String {
    let dot = if hidden { "." } else { "" };
    format!("{dot}{typed}{ext}")
}

pub const ADV_OK: &str = "adv_ok";
pub const ADV_CANCEL: &str = "adv_cancel";
pub const ADV_ARCHIVE: &str = "adv_archive";
pub const ADV_INDEX: &str = "adv_index";

pub fn robots_chosen(form: &Form) -> [CheckState; 2] {
    [ADV_ARCHIVE, ADV_INDEX].map(|id| form.check_state(id).unwrap_or_default())
}

pub fn robots_states(read: Option<[Option<bool>; 2]>) -> Option<[CheckState; 2]> {
    read.map(|r| r.map(shared_state))
}

pub fn robots_moves(opened: [CheckState; 2], chosen: [CheckState; 2]) -> [Option<bool>; 2] {
    [0, 1].map(|k| match chosen[k] {
        CheckState::Mixed => None,
        now if now == opened[k] => None,
        now => Some(now.is_on()),
    })
}

pub fn advanced_size(subject: Subject) -> (i32, i32) {
    if subject.reaches_folders() {
        (378, 285)
    } else {
        (378, 263)
    }
}

struct AdvancedGrid {
    lines: &'static [i32],
    groups: [i32; 2],
    checks: [i32; 4],
    details_y: i32,
    buttons_y: i32,
}

const FILE_ADVANCED: AdvancedGrid = AdvancedGrid {
    lines: &[22],
    groups: [62, 148],
    checks: [83, 104, 168, 191],
    details_y: 186,
    buttons_y: 229,
};
const FOLDER_ADVANCED: AdvancedGrid = AdvancedGrid {
    lines: &[16, 34, 47],
    groups: [82, 170],
    checks: [103, 126, 191, 213],
    details_y: 209,
    buttons_y: 251,
};
const ADV_ICON: (i32, i32) = (11, 12);
const ADV_TEXT_X: i32 = 54;
const ADV_TEXT_W: i32 = 313;
const ADV_GROUP_X: i32 = 11;
const ADV_GROUP_W: i32 = 357;
const ADV_GROUP_H: i32 = 70;
const ADV_TITLE_DX: i32 = 9;
const ADV_TITLE_DY: i32 = -5;
const ADV_CHECK_X: i32 = 21;
const ADV_DETAILS_X: i32 = 285;
const ADV_OK_X: i32 = 213;

/// The archive and index boxes stand for whether backups and search
/// indexers take the file (`robots`, in that order: the freedesktop
/// `user.xdg.robots.backup` and `.index` attributes), live, and
/// three-state where a set's items differ. With `robots` None, the file
/// system keeps no such attributes and both are greyed and unticked.
/// Compress and encrypt have no file-system bit here that Reprise reads
/// or writes, so they are always greyed and unticked.
pub fn advanced_form(
    subject: Subject,
    icon: Option<Canvas>,
    robots: Option<[CheckState; 2]>,
    text: &TextRenderer,
    px: u32,
    u: TextUnits,
) -> Form {
    let is_dir = subject.reaches_folders();
    let (grid, what) = if is_dir {
        (&FOLDER_ADVANCED, "folder")
    } else {
        (&FILE_ADVANCED, "file")
    };
    let mut lines = vec![match subject {
        Subject::Set { has_dir: true } => {
            "Choose the settings you want for the selected items".to_string()
        }
        _ => format!("Choose the options you want for this {what}."),
    }];
    if is_dir {
        lines.extend(crate::text::wrap(
            text,
            "When you apply these changes you will be asked whether they should reach all \
             subfolders and files as well.",
            u.len(ADV_TEXT_W),
            px,
        ));
    }
    let mut items = vec![Item::picture(
        "icon",
        Rect::new(ADV_ICON.0, ADV_ICON.1, ICON_PX, ICON_PX),
        icon,
    )];
    const LINE_IDS: [&str; 3] = ["adv_l0", "adv_l1", "adv_l2"];
    for ((id, y), line) in LINE_IDS.iter().zip(grid.lines).zip(&lines) {
        items.push(Item::label_at(id, ADV_TEXT_X, *y, ADV_TEXT_W, line));
    }
    let titles = [
        (
            ("adv_g0", "adv_gap0", "adv_t0"),
            "Archive and Index attributes",
        ),
        (
            ("adv_g1", "adv_gap1", "adv_t1"),
            "Compress or Encrypt attributes",
        ),
    ];
    for (((id, gap, title_id), title), y) in titles.into_iter().zip(grid.groups) {
        // The title sits a pixel further left than `Control::Group` puts
        // its own, so the frame goes in untitled and the title over the
        // gap it punches.
        let tw = u.design(text.measure(title, false, px));
        let tx = ADV_GROUP_X + ADV_TITLE_DX;
        items.push(Item::group(
            id,
            Rect::new(ADV_GROUP_X, y, ADV_GROUP_W, ADV_GROUP_H),
            "",
        ));
        items.push(Item::new(
            gap,
            Rect::new(tx - 2, y, tw + 4, 2),
            Control::Blank,
        ));
        items.push(Item::group_title(title_id, tx, y + ADV_TITLE_DY, tw, title));
    }
    let labels = [
        format!(
            "{} is ready for &archiving",
            if is_dir { "Folder" } else { "File" }
        ),
        format!("Allow this {what} to be &indexed for fast searching"),
        "&Compress contents to save disk space".to_string(),
        "&Encrypt contents to secure data".to_string(),
    ];
    const CHECK_IDS: [&str; 4] = [ADV_ARCHIVE, ADV_INDEX, "adv_compress", "adv_encrypt"];
    for (k, ((id, y), label)) in CHECK_IDS
        .into_iter()
        .zip(grid.checks)
        .zip(&labels)
        .enumerate()
    {
        match robots.and_then(|r| r.get(k).copied()) {
            Some(state) => items.push(check(id, ADV_CHECK_X, y, label, state)),
            None => items.push(Item::check(id, ADV_CHECK_X, y, label).enabled(false)),
        }
    }
    items.push(button(
        "adv_details",
        Rect::new(ADV_DETAILS_X, grid.details_y, BUTTON_W, BUTTON_H),
        "&Details",
        false,
    ));
    items.push(Item::button(
        ADV_OK,
        Rect::new(ADV_OK_X, grid.buttons_y, BUTTON_W, BUTTON_H),
        "OK",
        true,
        true,
    ));
    items.push(button(
        ADV_CANCEL,
        Rect::new(
            ADV_OK_X + BUTTON_W + BUTTON_GAP,
            grid.buttons_y,
            BUTTON_W,
            BUTTON_H,
        ),
        "Cancel",
        true,
    ));
    Form::new(items)
}

pub const ATTR_OK: &str = "attr_ok";
pub const ATTR_CANCEL: &str = "attr_cancel";
pub const ATTR_TREE: &str = "attr_tree";
pub const ATTR_ONLY: &str = "attr_only";

pub const ATTR_SIZE: (i32, i32) = (386, 216);
const ATTR_TEXT_X: i32 = 11;
const ATTR_CHANGE_X: i32 = 56;
const ATTR_RIGHT: i32 = 22;
const ATTR_INTRO_Y: i32 = 12;
const ATTR_CHANGE_Y: i32 = 38;
const ATTR_QUESTION_Y: i32 = 77;
const ATTR_RADIO: (i32, i32) = (54, 117);
const ATTR_RADIO_PITCH: i32 = 23;
const ATTR_BUTTON_Y: i32 = 181;
const ATTR_BUTTON_RIGHT: i32 = 11;
const ATTR_INTRO: &str = "You have chosen to make the following attribute changes:";
const ATTR_QUESTION: &str = "Do you want to apply this change to this folder only, or do you \
                             want to apply it to all subfolders and files as well?";
pub const ATTR_LABELS: [&str; 2] = [
    "Apply changes to this folder only",
    "Apply changes to this folder, subfolders and files",
];
const SET_ATTR_QUESTION: &str = "Do you want to apply this change to the selected items only, or \
                                 do you want to apply it to all subfolders and files as well?";
pub const SET_ATTR_LABELS: [&str; 2] = [
    "Apply changes to the selected items only",
    "Apply changes to the selected items, subfolders and files",
];

pub fn attr_form(verb: &str, subject: Subject, text: &TextRenderer, px: u32, u: TextUnits) -> Form {
    let (question, labels) = match subject {
        Subject::Set { .. } => (SET_ATTR_QUESTION, SET_ATTR_LABELS),
        _ => (ATTR_QUESTION, ATTR_LABELS),
    };
    let width = ATTR_SIZE.0 - ATTR_TEXT_X - ATTR_RIGHT;
    let mut items = vec![
        Item::label_at("attr_intro", ATTR_TEXT_X, ATTR_INTRO_Y, width, ATTR_INTRO),
        Item::label_at("attr_verb", ATTR_CHANGE_X, ATTR_CHANGE_Y, width, verb),
    ];
    for (i, line) in crate::text::wrap(text, question, u.len(width), px)
        .into_iter()
        .enumerate()
    {
        items.push(Item::label_at(
            if i == 0 { "attr_q0" } else { "attr_q1" },
            ATTR_TEXT_X,
            ATTR_QUESTION_Y + i as i32 * LINE_H,
            width,
            &line,
        ));
    }
    for (i, (id, text)) in [(ATTR_ONLY, labels[0]), (ATTR_TREE, labels[1])]
        .into_iter()
        .enumerate()
    {
        items.push(Item::new(
            id,
            Rect::new(
                ATTR_RADIO.0,
                ATTR_RADIO.1 + i as i32 * ATTR_RADIO_PITCH,
                CHECK,
                CHECK,
            ),
            Control::Radio {
                label: text.to_string(),
                group: "attr",
                checked: i == 1,
                enabled: true,
            },
        ));
    }
    let bx = ATTR_SIZE.0 - ATTR_BUTTON_RIGHT - 2 * BUTTON_W - BUTTON_GAP;
    items.push(Item::button(
        ATTR_OK,
        Rect::new(bx, ATTR_BUTTON_Y, BUTTON_W, BUTTON_H),
        "OK",
        true,
        true,
    ));
    items.push(button(
        ATTR_CANCEL,
        Rect::new(
            bx + BUTTON_W + BUTTON_GAP,
            ATTR_BUTTON_Y,
            BUTTON_W,
            BUTTON_H,
        ),
        "Cancel",
        true,
    ));
    Form::new(items)
}

pub fn attr_tree_picked(form: &Form) -> bool {
    matches!(
        form.item(ATTR_TREE).map(|i| &i.control),
        Some(Control::Radio { checked: true, .. })
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controls::form::Key;
    use crate::look::classic_paint;
    use crate::theme::{FontConfig, Theme};

    fn text() -> TextRenderer {
        TextRenderer::load(&FontConfig::default())
    }

    fn px() -> u32 {
        Theme::default().fonts.size_px
    }

    /// The Summary page's labels fit their column in DejaVu Sans, which is wider than Tahoma:
    /// the fields move right of "Comments:".
    #[test]
    fn the_summary_labels_fit_in_a_wider_face() {
        let family = "DejaVu Sans";
        let text = TextRenderer::load(&FontConfig {
            family: vec![family.into()],
            ..FontConfig::default()
        });
        if !text.has_face(family) {
            return;
        }
        let page = summary_page(None);
        page.drawn_with(&Theme::default().paint());
        assert_eq!(page.overflows(&text, px()), Vec::new());
        let comments = page.rect(SUMMARY_IDS[5]).unwrap();
        assert!(comments.x > SUM_COMMENTS.x, "{comments:?}");
        assert_eq!(comments.right(), SUM_COMMENTS.right());
    }

    #[test]
    fn a_long_date_wraps_inside_its_row() {
        let family = "DejaVu Sans";
        let text = TextRenderer::load(&FontConfig {
            family: vec![family.into()],
            ..FontConfig::default()
        });
        if !text.has_face(family) {
            return;
        }
        let date = "Wednesday, September 30, 2026, 12:00:00 PM";
        let mut f = facts(false);
        f.modified = date.into();
        let (page, _) = general_page(&rows(&f), "a", None, None);
        page.drawn_with(&Theme::default().paint());
        let r = page.rect("v_mod").unwrap();
        assert!(text.measure(date, false, px()) > r.width);
        assert_eq!(page.overflows(&text, px()), Vec::new());
        assert_eq!(page.rect("l_acc").unwrap().y, r.y + ROW_PITCH);
    }

    fn facts(is_dir: bool) -> Facts {
        Facts {
            is_dir,
            type_name: if is_dir {
                "File Folder"
            } else {
                "Text Document"
            }
            .into(),
            opens_with: (!is_dir).then(|| "Notepad".to_string()),
            description: None,
            location: "/home/user/Desktop".into(),
            size: "8 bytes (8 bytes)".into(),
            on_disk: "4.00 KB (4,096 bytes)".into(),
            contains: is_dir.then(|| "1 Files, 1 Folders".to_string()),
            created: Some("Monday, September 14, 2026, 3:43:34 PM".into()),
            modified: "Monday, September 14, 2026, 3:43:53 PM".into(),
            accessed: "Sunday, September 20, 2026, 12:24:12 PM".into(),
            read_only: false,
            hidden: false,
        }
    }

    fn page(is_dir: bool) -> (Form, i32) {
        general_page(&rows(&facts(is_dir)), "New Folder", None, None)
    }

    #[test]
    fn a_files_page_lands_on_its_grid() {
        let (page, height) = page(false);
        let rect = |id: &str| page.item(id).expect(id).rect;
        assert_eq!(rect(NAME), Rect::new(94, 48, 246, 23));
        assert_eq!(rect("icon"), Rect::new(21, 41, 32, 32));
        let seps = ["sep0", "sep1", "sep2", "sep3"].map(&rect);
        assert_eq!(seps.map(|r| r.y), [82, 147, 238, 329]);
        assert!(seps.iter().all(|r| (r.x, r.right()) == (21, 339)));
        assert_eq!(rect("l_type"), Rect::new(21, 95, 76, 13));
        assert_eq!(rect("v_type"), Rect::new(97, 95, 242, 13));
        assert_eq!(rect("l_opens").y, 121);
        assert_eq!(rect("opens_icon"), Rect::new(95, 121, 16, 16));
        assert_eq!(rect("v_opens").x, 120);
        assert_eq!(rect(CHANGE), Rect::new(234, 118, 105, 23));
        assert_eq!(
            ["l_loc", "l_size", "l_disk"].map(|id| rect(id).y),
            [160, 186, 212]
        );
        assert_eq!(
            ["l_made", "l_mod", "l_acc"].map(|id| rect(id).y),
            [251, 277, 303]
        );
        assert_eq!(rect("l_attr").y, 342);
        assert_eq!(rect(READ_ONLY), Rect::new(94, 343, 13, 13));
        assert_eq!(rect(HIDDEN), Rect::new(174, 343, 13, 13));
        assert_eq!(rect(ADVANCED), Rect::new(265, 339, 75, 23));
        assert_eq!(height, 478);
    }

    #[test]
    fn a_folders_page_lands_on_its_grid() {
        let (page, height) = page(true);
        let rect = |id: &str| page.item(id).expect(id).rect;
        assert_eq!(rect(NAME), Rect::new(94, 48, 246, 23));
        assert_eq!(
            ["sep0", "sep1", "sep2"].map(|id| rect(id).y),
            [82, 225, 264]
        );
        for (i, id) in ["l_type", "l_loc", "l_size", "l_disk", "l_has"]
            .into_iter()
            .enumerate()
        {
            assert_eq!(rect(id), Rect::new(21, 95 + 26 * i as i32, 73, 13), "{id}");
        }
        assert_eq!(rect("v_type").x, 94);
        assert_eq!(rect("l_made").y, 238);
        assert_eq!(rect("l_attr").y, 277);
        assert_eq!(rect(READ_ONLY), Rect::new(94, 278, 13, 13));
        assert_eq!(rect(HIDDEN), Rect::new(94, 301, 13, 13));
        assert_eq!(rect(ADVANCED), Rect::new(234, 274, 105, 23));
        assert_eq!(height, 424);
    }

    fn set_facts(has_dir: bool) -> SetFacts {
        SetFacts {
            has_dir,
            count: "2 Files, 2 Folders".into(),
            type_text: "Multiple Types".into(),
            location: "All in /home/user/Desktop".into(),
            size: "148 bytes (148 bytes)".into(),
            on_disk: "8.00 KB (8,192 bytes)".into(),
            read_only: CheckState::Off,
            hidden: CheckState::Mixed,
        }
    }

    fn set_page(has_dir: bool) -> (Form, i32) {
        general_page(&set_rows(&set_facts(has_dir)), "", None, None)
    }

    #[test]
    fn a_sets_page_lands_on_its_grid() {
        for (has_dir, value_x, attr) in [(false, 94, "Attributes:"), (true, 97, "Attributes")] {
            let (page, height) = set_page(has_dir);
            let rect = |id: &str| page.item(id).expect(id).rect;
            assert!(page.item(NAME).is_none(), "no name field");
            assert_eq!(rect("icon"), Rect::new(21, 41, 32, 32));
            assert_eq!((rect(COUNT).x, rect(COUNT).y), (94, 48));
            assert_eq!(["sep0", "sep1"].map(|id| rect(id).y), [82, 199]);
            assert!(page.item("sep2").is_none());
            for (i, id) in ["l_type", "l_loc", "l_size", "l_disk"]
                .into_iter()
                .enumerate()
            {
                assert_eq!((rect(id).x, rect(id).y), (21, 95 + 26 * i as i32), "{id}");
            }
            assert_eq!(rect("v_type").x, value_x, "has_dir {has_dir}");
            assert_eq!(rect("l_attr").y, 212);
            assert!(matches!(
                &page.item("l_attr").unwrap().control,
                Control::Label { text, .. } if text == attr
            ));
            assert_eq!(rect(READ_ONLY), Rect::new(94, 213, 13, 13));
            assert_eq!(rect(HIDDEN), Rect::new(94, 236, 13, 13));
            assert_eq!(rect(ADVANCED), Rect::new(265, 209, 75, 23));
            assert_eq!(
                height, 424,
                "a folder page's; a Summary tab makes the sheet 478"
            );
        }
    }

    #[test]
    fn folder_column_pages_are_set_in_the_second_dialog_font() {
        let page_font = |rows: Vec<Row>| general_page(&rows, "x", None, None).0.font;
        assert_eq!(page_font(rows(&facts(true))), DialogFont::ShellDlg);
        assert_eq!(page_font(rows(&facts(false))), DialogFont::Ui);
        assert_eq!(set_page(false).0.font, DialogFont::ShellDlg);
        assert_eq!(set_page(true).0.font, DialogFont::Ui);
    }

    #[test]
    fn a_sets_boxes_have_three_states_only_where_its_items_differ() {
        let (mut page, _) = set_page(true);
        assert_eq!(read_only(&page), CheckState::Off);
        assert_eq!(hidden_state(&page), CheckState::Mixed);
        page.focus_id(READ_ONLY);
        page.key(Key::Space);
        page.key(Key::Space);
        assert_eq!(read_only(&page), CheckState::Off, "Read-only flips");
        page.focus_id(HIDDEN);
        let mut seen = Vec::new();
        for _ in 0..3 {
            page.key(Key::Space);
            seen.push(hidden_state(&page));
        }
        assert_eq!(seen, [CheckState::Off, CheckState::On, CheckState::Mixed]);
    }

    #[test]
    fn a_sets_page_inks_on_its_grid() {
        let t = text();
        if !crate::text::test_tahoma() {
            return;
        }
        let theme = Theme::default();
        let cs = &theme.colors;
        let (page, height) = set_page(true);
        let mut c = Canvas::new(SHEET_W as u32, height as u32);
        c.fill_rect(c.bounds(), cs.button_face);
        crate::controls::form::draw(&mut c, &t, &classic_paint(cs, 1), &page, false, px());
        let ink = |id: &str| {
            let r = page.item(id).unwrap().rect;
            let hits: Vec<(i32, i32)> = (r.y..r.bottom())
                .flat_map(|y| (r.x..r.right()).map(move |x| (x, y)))
                .filter(|&(x, y)| c.get(x, y) == cs.button_text.argb())
                .collect();
            (
                hits.iter().map(|p| p.0).min().unwrap(),
                hits.iter().map(|p| p.1).min().unwrap(),
            )
        };
        let got = [COUNT, "l_type", "v_type", "v_loc", "v_disk", "l_attr"].map(|id| (id, ink(id)));
        let want = [
            (COUNT, (94, 50)),
            ("l_type", (21, 98)),
            ("v_type", (97, 97)),
            ("v_loc", (97, 123)),
            ("v_disk", (97, 175)),
            ("l_attr", (21, 214)),
        ];
        assert_eq!(got, want);
    }

    #[test]
    fn a_sets_boxes_speak_of_the_selected_items() {
        let (t, px) = (text(), px());
        let line = |f: &Form, id: &str| match &f.item(id).expect(id).control {
            Control::Label { text, .. } | Control::Radio { label: text, .. } => text.clone(),
            other => panic!("{other:?}"),
        };
        for (has_dir, first) in [
            (false, "Choose the options you want for this file."),
            (true, "Choose the settings you want for the selected items"),
        ] {
            let set = Subject::Set { has_dir };
            let adv = advanced_form(set, None, None, &t, px, TextUnits::ONE);
            assert_eq!(line(&adv, "adv_l0"), first);
            assert_eq!(advanced_size(set), advanced_size(Subject::single(has_dir)));
            assert_eq!(adv.item("adv_l1").is_some(), has_dir);
        }
        let set = Subject::Set { has_dir: true };
        let f = attr_form("set read-only", set, &t, px, TextUnits::ONE);
        assert_eq!(line(&f, ATTR_ONLY), SET_ATTR_LABELS[0]);
        assert_eq!(line(&f, ATTR_TREE), SET_ATTR_LABELS[1]);
        assert!(line(&f, "attr_q0").contains("the selected items only"));
        let folder = attr_form("set read-only", Subject::Folder, &t, px, TextUnits::ONE);
        assert_eq!(line(&folder, ATTR_ONLY), ATTR_LABELS[0]);
    }

    #[test]
    fn the_archive_and_index_boxes_are_live_over_what_was_read() {
        let (t, px) = (text(), px());
        let read = robots_states(Some([Some(true), None])).unwrap();
        assert_eq!(read, [CheckState::On, CheckState::Mixed]);
        let mut f = advanced_form(
            Subject::Set { has_dir: true },
            None,
            Some(read),
            &t,
            px,
            TextUnits::ONE,
        );
        let enabled = |f: &Form, id: &str| {
            matches!(
                f.item(id).map(|i| &i.control),
                Some(Control::Check { enabled: true, .. })
            )
        };
        assert!(enabled(&f, ADV_ARCHIVE) && enabled(&f, ADV_INDEX));
        assert!(!enabled(&f, "adv_compress") && !enabled(&f, "adv_encrypt"));
        assert_eq!(f.focus().map(|i| f.items[i].id), Some(ADV_ARCHIVE));
        assert_eq!(robots_chosen(&f), read);
        f.key(Key::Space);
        assert_eq!(robots_moves(read, robots_chosen(&f)), [Some(false), None]);
        f.focus_id(ADV_INDEX);
        f.key(Key::Space);
        assert_eq!(
            robots_moves(read, robots_chosen(&f)),
            [Some(false), Some(false)]
        );
        let greyed = advanced_form(Subject::File, None, None, &t, px, TextUnits::ONE);
        assert!(!enabled(&greyed, ADV_ARCHIVE));
        assert_eq!(greyed.check_state(ADV_ARCHIVE), Some(CheckState::Off));
    }

    #[test]
    #[ignore]
    fn render_set_sheet() {
        for (name, theme) in [
            ("filesheet-set.png", Theme::default()),
            (
                "filesheet-set-blue.png",
                crate::interfaces::theme_for(crate::interface::Interface::Blue),
            ),
        ] {
            let t = text();
            let (page, height) = set_page(true);
            let sheet = crate::controls::propsheet::PropSheet::new(
                &["General"],
                vec![page],
                (SHEET_W, height),
            );
            let mut c = Canvas::new(SHEET_W as u32, height as u32);
            c.fill_rect(c.bounds(), theme.colors.button_face);
            crate::controls::propsheet::draw(&mut c, &t, &theme.paint(), &sheet, true, px());
            crate::raster::write_preview(&c, name);
        }
    }

    #[test]
    fn a_page_opens_on_read_only() {
        let focused = |f: &Form| f.focus().map(|i| f.items[i].id);
        for (mut p, _) in [page(false), page(true), set_page(false)] {
            assert_eq!(focused(&p), Some(READ_ONLY));
            if p.item(NAME).is_some() {
                let back = if p.item(CHANGE).is_some() { 2 } else { 1 };
                for _ in 0..back {
                    p.key(Key::Tab { back: true });
                }
                assert_eq!(focused(&p), Some(NAME));
                let Some(Control::Edit { edit, .. }) = p.item(NAME).map(|i| &i.control) else {
                    panic!("the name field");
                };
                assert_eq!(edit.selected_text(), "New Folder");
            }
        }
    }

    #[test]
    fn each_kind_of_sheet_gets_its_tabs() {
        let set = |document, shortcut, folder| {
            sheet_tabs(Over::Set {
                document,
                shortcut,
                folder,
            })
        };
        assert_eq!(sheet_tabs(Over::File { shortcut: false }), [Tab::Summary]);
        assert_eq!(sheet_tabs(Over::File { shortcut: true }), []);
        assert_eq!(
            sheet_tabs(Over::Launcher),
            [Tab::Shortcut, Tab::Compatibility]
        );
        assert_eq!(sheet_tabs(Over::Folder), [Tab::Sharing, Tab::Customize]);
        assert_eq!(set(true, false, false), [Tab::Summary]);
        assert_eq!(set(true, false, true), [Tab::Summary]);
        assert_eq!(set(false, true, false), [Tab::Compatibility]);
        assert_eq!(set(true, true, false), [Tab::Compatibility, Tab::NoSummary]);
        assert_eq!(set(false, false, true), [Tab::Customize]);
    }

    #[test]
    fn the_customize_page_lands_on_its_grid() {
        let p = customize_page();
        let rect = |id: &str| p.item(id).expect(id).rect;
        assert_eq!(
            ["cu_g0", "cu_g1", "cu_g2"].map(rect),
            [
                Rect::new(21, 47, 319, 90),
                Rect::new(21, 151, 319, 134),
                Rect::new(21, 299, 319, 78)
            ]
        );
        assert_eq!(rect("cu_template"), Rect::new(31, 80, 299, 21));
        assert_eq!(rect("cu_all"), Rect::new(31, 113, 13, 13));
        assert_eq!(rect(CUSTOM_PREVIEW), Rect::new(225, 181, 96, 96));
        assert_eq!(rect("cu_choose"), Rect::new(31, 222, 98, 23));
        assert_eq!(rect("cu_restore"), Rect::new(31, 251, 98, 23));
        assert_eq!(rect(CHANGE_ICON), Rect::new(31, 345, 98, 23));
        assert_eq!(rect(CUSTOM_ICON), Rect::new(251, 343, 32, 32));
        assert_eq!(p.focus().map(|i| p.items[i].id), Some(CHANGE_ICON));
    }

    #[test]
    fn the_change_icon_box_picks_an_icon() {
        use crate::controls::iconlist::IconList;
        let names: Vec<String> = FOLDER_ICON_NAMES.iter().map(|n| n.to_string()).collect();
        let n = names.len();
        let mut list = IconList::new(names, vec![None; n], None);
        list.pick("folder-music");
        let mut f = change_icon_form("hicolor", list, true);
        let rect = |f: &Form, id: &str| f.item(id).expect(id).rect;
        assert_eq!(rect(&f, "ci_file"), Rect::new(11, 29, 185, 21));
        assert_eq!(rect(&f, CHANGE_ICON_LIST), Rect::new(11, 77, 267, 198));
        assert_eq!(rect(&f, CHANGE_ICON_OK), Rect::new(122, 292, 75, 23));
        assert_eq!(change_icon_picked(&f).as_deref(), Some("folder-music"));
        assert_eq!(f.focus().map(|i| f.items[i].id), Some(CHANGE_ICON_LIST));
        assert_eq!(
            f.key(Key::Down),
            crate::controls::form::Event::Changed(CHANGE_ICON_LIST)
        );
        assert_eq!(change_icon_picked(&f).as_deref(), Some("folder-pictures"));
        if let Some(Control::IconList(l)) = f.item_mut(CHANGE_ICON_LIST).map(|i| &mut i.control) {
            l.left = 2;
        }
        change_icon_restore(&mut f);
        assert_eq!(
            change_icon_picked(&f).as_deref(),
            Some(STANDARD_FOLDER_ICON)
        );
        let Some(Control::IconList(l)) = f.item(CHANGE_ICON_LIST).map(|i| &i.control) else {
            unreachable!()
        };
        assert_eq!(l.left, 0, "the picked column shows");
    }

    #[test]
    fn the_summary_page_lands_on_its_grid_and_reports_its_moves() {
        let values: [String; 6] =
            ["Report", "", "Ann", "", "q3", "first\nsecond"].map(String::from);
        let mut p = summary_page(Some(&values));
        let rect = |p: &Form, id: &str| p.item(id).expect(id).rect;
        assert_eq!(
            SUMMARY_IDS[..5]
                .iter()
                .map(|id| rect(&p, id).y)
                .collect::<Vec<_>>(),
            [41, 72, 103, 152, 183]
        );
        assert!(SUMMARY_IDS[..5].iter().all(|id| (
            rect(&p, id).x,
            rect(&p, id).width,
            rect(&p, id).height
        ) == (81, 263, 21)));
        assert_eq!(rect(&p, "sum_l0"), Rect::new(22, 44, 59, 13));
        assert_eq!(rect(&p, "sum_sep"), Rect::new(22, 137, 318, 2));
        assert_eq!(rect(&p, SUMMARY_IDS[5]), Rect::new(81, 214, 263, 80));
        assert_eq!(rect(&p, SUMMARY_ADVANCED), Rect::new(244, 305, 99, 23));
        assert!(matches!(
            p.item(SUMMARY_ADVANCED).map(|i| &i.control),
            Some(Control::Button { enabled: false, .. })
        ));
        assert_eq!(summary_values(&p), values);
        assert_eq!(p.focus().map(|i| p.items[i].id), Some(SUMMARY_IDS[0]));
        p.key(Key::Edit(crate::textedit::EditKey::End { select: false }));
        p.key(Key::Char('!'));
        assert_eq!(summary_moves(&p, &values), [(0, "Report!".to_string())]);

        let greyed = summary_page(None);
        assert_eq!(greyed.focus(), None, "nothing on it is live");
        assert_eq!(summary_values(&greyed), <[String; 6]>::default());
    }

    fn shortcut_facts() -> ShortcutFacts {
        ShortcutFacts {
            name: "Firefox".into(),
            target_type: "Application".into(),
            target_location: "bin".into(),
            symlink: false,
            fields: [
                "firefox %u".into(),
                "/home/user".into(),
                "Browse the web".into(),
            ],
        }
    }

    fn symlink_facts() -> ShortcutFacts {
        ShortcutFacts {
            name: "Folder Link".into(),
            target_type: "File Folder".into(),
            target_location: "user".into(),
            symlink: true,
            fields: ["/home/user/Documents".into(), String::new(), String::new()],
        }
    }

    fn web_facts() -> WebFacts {
        WebFacts {
            name: "Web Page".into(),
            url: "http://www.example.com/".into(),
        }
    }

    #[test]
    fn a_shortcuts_page_has_its_description_in_opens_withs_place() {
        let link = Facts {
            type_name: "Shortcut".into(),
            opens_with: None,
            description: Some("Documents".into()),
            ..facts(false)
        };
        let (page, height) = general_page(&rows(&link), "Folder Link", None, None);
        let rect = |id: &str| page.item(id).expect(id).rect;
        assert_eq!(rect("l_desc"), Rect::new(21, 121, 76, 13));
        assert_eq!(rect("v_desc"), Rect::new(97, 121, 242, 13));
        assert!(page.item(CHANGE).is_none());
        assert_eq!(rect("l_loc").y, 160);
        assert_eq!(height, 478);
        let opens = Facts {
            description: Some("unused".into()),
            ..facts(false)
        };
        let (page, _) = general_page(&rows(&opens), "a", None, None);
        assert!(page.item("l_desc").is_none(), "a program shows instead");
    }

    #[test]
    fn symlink_and_web_link_sheets_open_on_their_own_tab() {
        assert_eq!(sheet_tabs(Over::Symlink { runs: false }), [Tab::Shortcut]);
        assert_eq!(
            sheet_tabs(Over::Symlink { runs: true }),
            [Tab::Shortcut, Tab::Compatibility]
        );
        assert_eq!(sheet_tabs(Over::WebLink), [Tab::WebDocument]);
        let (general, height) = page(false);
        let s = sheet(general, height, &sheet_tabs(Over::WebLink), |_| {
            web_page(&web_facts(), None)
        });
        assert_eq!(s.tabs.labels, ["General", "Web Document"]);
        assert_eq!(s.size, (SHEET_W, SHORTCUT_SHEET_H));
        assert_eq!(s.tabs.open, 1);
        assert!(shortcut_of(&s.pages).is_some_and(|p| std::ptr::eq(p, s.open())));
        let (general, height) = page(false);
        let s = sheet(
            general,
            height,
            &sheet_tabs(Over::Symlink { runs: false }),
            |_| shortcut_page(&symlink_facts(), None),
        );
        assert_eq!(s.tabs.labels, ["General", "Shortcut"]);
        assert_eq!(s.tabs.open, 1);
    }

    #[test]
    fn a_symlinks_shortcut_page_has_its_target_as_its_one_field() {
        let facts = symlink_facts();
        let mut p = shortcut_page(&facts, None);
        let text = |id: &str| match p.item(id).map(|i| &i.control) {
            Some(Control::Label { text, .. }) => text.clone(),
            _ => panic!("{id} is no label"),
        };
        assert_eq!(text("sc_type"), "File Folder");
        assert_eq!(text("sc_where"), "user");
        let live = |p: &Form, id: &str| match p.item(id).map(|i| &i.control) {
            Some(
                Control::Edit { enabled, .. }
                | Control::Button { enabled, .. }
                | Control::Label { enabled, .. },
            ) => *enabled,
            _ => panic!("{id}"),
        };
        for id in [
            SHORTCUT_IDS[1],
            SHORTCUT_IDS[2],
            "sc_start_l",
            "sc_comment_l",
            CHANGE_ICON,
        ] {
            assert!(!live(&p, id), "{id} is greyed");
        }
        assert!(live(&p, SHORTCUT_IDS[0]) && live(&p, FIND_TARGET));
        assert!(live(&p, "sc_target_l"));
        assert_eq!(p.focus().map(|i| p.items[i].id), Some(SHORTCUT_IDS[0]));
        p.key(Key::Edit(crate::textedit::EditKey::End { select: false }));
        p.key(Key::Char('s'));
        assert_eq!(
            shortcut_moves(&p, &facts.fields),
            [(0, "/home/user/Documentss".to_string())]
        );
        let launcher = shortcut_page(&shortcut_facts(), None);
        assert!(live(&launcher, SHORTCUT_IDS[1]) && live(&launcher, CHANGE_ICON));
    }

    #[test]
    fn the_web_page_lands_on_its_grid_and_reports_its_url() {
        let facts = web_facts();
        let mut p = web_page(&facts, None);
        let rect = |p: &Form, id: &str| p.item(id).expect(id).rect;
        assert_eq!(rect(&p, "icon"), Rect::new(21, 41, 32, 32));
        assert_eq!((rect(&p, "wd_name").x, rect(&p, "wd_name").y), (64, 49));
        assert_eq!(rect(&p, "wd_sep0"), Rect::new(21, 85, 318, 2));
        assert_eq!(rect(&p, "wd_sep1"), Rect::new(21, 184, 318, 2));
        assert_eq!(rect(&p, SHORTCUT_IDS[0]), Rect::new(96, 98, 245, 23));
        assert_eq!(rect(&p, "wd_key"), Rect::new(96, 126, 245, 23));
        assert_eq!(rect(&p, "wd_url_l"), Rect::new(21, 103, 75, 13));
        assert_eq!(rect(&p, "wd_key_l").y, 131);
        assert_eq!(rect(&p, "wd_visits_l"), Rect::new(21, 160, 75, 13));
        assert_eq!(rect(&p, "wd_visits").x, 96);
        assert_eq!(rect(&p, CHANGE_ICON), Rect::new(250, 194, 90, 23));
        assert!(matches!(
            p.item("wd_key").map(|i| &i.control),
            Some(Control::HotKey(h)) if !h.enabled
        ));
        assert!(matches!(
            p.item("wd_key_l").map(|i| &i.control),
            Some(Control::Label { enabled: false, .. })
        ));
        assert_eq!(p.focus().map(|i| p.items[i].id), Some(SHORTCUT_IDS[0]));
        assert_eq!(
            p.focused_edit().and_then(|e| e.copy_text()).as_deref(),
            Some("http://www.example.com/")
        );
        let opened = [facts.url.clone(), String::new(), String::new()];
        assert_eq!(shortcut_values(&p), opened);
        p.key(Key::Edit(crate::textedit::EditKey::End { select: false }));
        p.key(Key::Char('a'));
        assert_eq!(
            shortcut_moves(&p, &opened),
            [(0, "http://www.example.com/a".to_string())]
        );
    }

    #[test]
    fn the_web_page_draws_on_its_grid() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let (general, height) = page(false);
        let s = sheet(general, height, &sheet_tabs(Over::WebLink), |_| {
            web_page(&web_facts(), None)
        });
        let mut c = Canvas::new(SHEET_W as u32, s.size.1 as u32);
        c.fill_rect(c.bounds(), cs.button_face);
        crate::controls::propsheet::draw(&mut c, &text(), &classic_paint(cs, 1), &s, true, px());
        let at = |x, y| c.get(x, y);
        for y in [85, 184] {
            assert!(
                (21..338).all(|x| at(x, y) == cs.button_shadow.argb()),
                "shadow row {y}"
            );
            assert!(
                (21..338).all(|x| at(x, y + 1) == cs.button_hilight.argb()),
                "light row {y}"
            );
        }
        assert_eq!(at(150, 98), cs.button_shadow.argb(), "URL's frame top");
        assert_eq!(at(150, 99), cs.button_dk_shadow.argb());
        assert_eq!(at(96, 110), cs.button_shadow.argb(), "URL's frame left");
        assert_eq!(at(150, 120), cs.button_hilight.argb(), "URL's frame bottom");
        assert_eq!(at(340, 110), cs.button_hilight.argb(), "URL's frame right");
        assert_eq!(at(260, 194), cs.button_hilight.argb(), "Change Icon's top");
        if !crate::text::test_tahoma() {
            return;
        }
        let ink = cs.button_text.argb();
        let first = |x0: i32, x1: i32, y0: i32, y1: i32| {
            let top = (y0..y1).find(|&y| (x0..x1).any(|x| at(x, y) == ink));
            let left = (x0..x1).find(|&x| (y0..y1).any(|y| at(x, y) == ink));
            (left.expect("ink"), top.expect("ink"))
        };
        assert_eq!(first(15, 60, 100, 116), (21, 106), "URL:");
        assert_eq!(first(15, 60, 158, 175), (21, 163), "Visits:");
        assert_eq!(first(55, 70, 45, 70), (64, 52), "the name's capital");
        assert_eq!(first(90, 160, 158, 175).0, 96, "Unknown");
    }

    #[test]
    fn a_launchers_sheet_opens_on_its_shortcut_tab() {
        let (general, height) = page(false);
        let s = sheet(general, height, &sheet_tabs(Over::Launcher), |_| {
            shortcut_page(&shortcut_facts(), None)
        });
        assert_eq!(s.size, (SHEET_W, SHORTCUT_SHEET_H));
        assert_eq!(s.pages.len(), 3);
        assert_eq!(s.tabs.open, 1);
        assert!(shortcut_of(&s.pages).is_some_and(|p| std::ptr::eq(p, s.open())));
    }

    #[test]
    fn the_shortcut_page_lands_on_its_grid_and_reports_its_moves() {
        let facts = shortcut_facts();
        let mut p = shortcut_page(&facts, None);
        let rect = |p: &Form, id: &str| p.item(id).expect(id).rect;
        assert_eq!(rect(&p, "icon"), Rect::new(21, 42, 32, 32));
        assert_eq!(rect(&p, "sc_name").x, 87);
        assert_eq!(rect(&p, "sc_name").y, 49);
        assert_eq!(rect(&p, "sc_sep0"), Rect::new(21, 80, 318, 2));
        assert_eq!(rect(&p, "sc_sep1"), Rect::new(21, 173, 319, 2));
        assert_eq!(rect(&p, "sc_type_l"), Rect::new(21, 92, 79, 13));
        assert_eq!(rect(&p, "sc_where"), Rect::new(100, 118, 240, 13));
        let fields = [
            SHORTCUT_IDS[0],
            SHORTCUT_IDS[1],
            "sc_key",
            "sc_run",
            SHORTCUT_IDS[2],
        ];
        assert_eq!(fields.map(|id| rect(&p, id).y), [142, 186, 217, 248, 277]);
        assert!(
            fields
                .iter()
                .all(|id| (rect(&p, id).x, rect(&p, id).width) == (100, 240))
        );
        assert_eq!(rect(&p, "sc_run").height, 21);
        assert_eq!(rect(&p, "sc_comment_l"), Rect::new(21, 279, 79, 13));
        assert_eq!(
            [FIND_TARGET, CHANGE_ICON, "sc_advanced"].map(|id| rect(&p, id)),
            [
                Rect::new(58, 306, 90, 23),
                Rect::new(154, 306, 90, 23),
                Rect::new(250, 306, 90, 23)
            ]
        );
        let greyed = |id: &str| match p.item(id).map(|i| &i.control) {
            Some(Control::HotKey(h)) => !h.enabled,
            Some(Control::Combo(c)) => !c.enabled,
            Some(Control::Button { enabled, .. }) => !enabled,
            _ => false,
        };
        assert!(greyed("sc_key") && greyed("sc_run") && greyed("sc_advanced"));
        assert!(!greyed(FIND_TARGET) && !greyed(CHANGE_ICON));
        assert_eq!(p.focus().map(|i| p.items[i].id), Some(SHORTCUT_IDS[0]));
        assert_eq!(
            p.focused_edit().and_then(|e| e.copy_text()).as_deref(),
            Some("firefox %u")
        );
        assert_eq!(shortcut_values(&p), facts.fields);
        p.focus_id(SHORTCUT_IDS[2]);
        p.key(Key::Edit(crate::textedit::EditKey::End { select: false }));
        p.key(Key::Char('!'));
        assert_eq!(
            shortcut_moves(&p, &facts.fields),
            [(2, "Browse the web!".to_string())]
        );
    }

    #[test]
    fn the_shortcut_page_draws_on_its_grid() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let (general, height) = page(false);
        let s = sheet(general, height, &sheet_tabs(Over::Launcher), |_| {
            shortcut_page(&shortcut_facts(), None)
        });
        let mut c = Canvas::new(SHEET_W as u32, s.size.1 as u32);
        c.fill_rect(c.bounds(), cs.button_face);
        crate::controls::propsheet::draw(&mut c, &text(), &classic_paint(cs, 1), &s, true, px());
        let at = |x, y| c.get(x, y);
        for y in [80, 173] {
            assert!(
                (21..338).all(|x| at(x, y) == cs.button_shadow.argb()),
                "shadow row {y}"
            );
            assert!(
                (21..338).all(|x| at(x, y + 1) == cs.button_hilight.argb()),
                "light row {y}"
            );
        }
        assert_eq!(at(150, 142), cs.button_shadow.argb(), "Target's frame top");
        assert_eq!(at(150, 143), cs.button_dk_shadow.argb());
        assert_eq!(at(100, 150), cs.button_shadow.argb(), "Target's frame left");
        assert_eq!(
            at(150, 164),
            cs.button_hilight.argb(),
            "Target's frame bottom"
        );
        assert_eq!(
            at(339, 150),
            cs.button_hilight.argb(),
            "Target's frame right"
        );
        assert_eq!(
            at(100, 306),
            cs.button_hilight.argb(),
            "Find Target's top edge"
        );
        if !crate::text::test_tahoma() {
            return;
        }
        let ink_top = |x0: i32, x1: i32, y0: i32| {
            (y0..y0 + 13)
                .find(|&y| (x0..x1).any(|x| at(x, y) == cs.button_text.argb()))
                .expect("ink")
        };
        assert_eq!(ink_top(21, 27, 92), 95, "Target type:");
        assert_eq!(ink_top(21, 27, 144), 147, "Target:");
        assert_eq!(ink_top(87, 92, 49), 52, "the name");
        let name_left = (80..100).find(|&x| (49..62).any(|y| at(x, y) == cs.button_text.argb()));
        assert_eq!(name_left, Some(87), "the name's first ink");
    }

    #[test]
    fn a_launchers_change_icon_box_has_no_restore() {
        use crate::controls::iconlist::IconList;
        let list = IconList::new(vec!["firefox".into()], vec![None], None);
        let mut list = list;
        list.pick("firefox");
        let f = change_icon_form("hicolor", list, false);
        assert!(f.item(CHANGE_ICON_RESTORE).is_none());
        assert_eq!(
            f.item(CHANGE_ICON_OK).unwrap().rect,
            Rect::new(122, 292, 75, 23)
        );
        assert_eq!(change_icon_picked(&f).as_deref(), Some("firefox"));
    }

    #[test]
    #[ignore]
    fn render_shortcut_page() {
        let t = text();
        let theme = Theme::default();
        let (general, height) = page(false);
        let s = sheet(general, height, &sheet_tabs(Over::Launcher), |_| {
            shortcut_page(&shortcut_facts(), None)
        });
        let mut c = Canvas::new(SHEET_W as u32, s.size.1 as u32);
        c.fill_rect(c.bounds(), theme.colors.button_face);
        crate::controls::propsheet::draw(&mut c, &t, &theme.paint(), &s, true, px());
        crate::raster::write_preview(&c, "filesheet-shortcut.png");
    }

    #[test]
    #[ignore]
    fn render_link_pages() {
        let t = text();
        let theme = Theme::default();
        let pages: [(Over, Form, &str); 2] = [
            (
                Over::Symlink { runs: false },
                shortcut_page(&symlink_facts(), None),
                "filesheet-symlink.png",
            ),
            (
                Over::WebLink,
                web_page(&web_facts(), None),
                "filesheet-web.png",
            ),
        ];
        for (over, own, file) in pages {
            let (general, height) = page(false);
            let mut own = Some(own);
            let s = sheet(general, height, &sheet_tabs(over), |_| {
                own.take().expect("one own page")
            });
            let mut c = Canvas::new(SHEET_W as u32, s.size.1 as u32);
            c.fill_rect(c.bounds(), theme.colors.button_face);
            crate::controls::propsheet::draw(&mut c, &t, &theme.paint(), &s, true, px());
            crate::raster::write_preview(&c, file);
        }
    }

    #[test]
    #[ignore]
    fn render_summary_page() {
        let t = text();
        let theme = Theme::default();
        let (general, _) = page(false);
        let values: [String; 6] =
            ["Report", "", "Ann", "", "q3", "first\nsecond"].map(String::from);
        let mut s = sheet(general, 478, &[Tab::Summary], |_| {
            summary_page(Some(&values))
        });
        s.open_tab(1);
        let mut c = Canvas::new(SHEET_W as u32, s.size.1 as u32);
        c.fill_rect(c.bounds(), theme.colors.button_face);
        crate::controls::propsheet::draw(&mut c, &t, &theme.paint(), &s, true, px());
        crate::raster::write_preview(&c, "filesheet-summary.png");
    }

    #[test]
    fn a_sheet_is_as_tall_as_its_tallest_page() {
        let (general, _) = page(true);
        let s = sheet(general.clone(), 424, &[Tab::Sharing], |_| Form::new(vec![]));
        assert_eq!(s.size, (SHEET_W, 424));
        assert_eq!(s.pages.len(), 2);
        let s = sheet(general.clone(), 424, &[Tab::Compatibility], |_| {
            Form::new(vec![])
        });
        assert_eq!(s.size.1, 437);
        let s = sheet(general, 424, &[Tab::Compatibility, Tab::NoSummary], |_| {
            Form::new(vec![])
        });
        assert_eq!(s.size.1, 478);
    }

    #[test]
    fn the_sharing_page_lands_on_its_grid() {
        let p = sharing_page(None, None);
        let rect = |id: &str| p.item(id).expect(id).rect;
        assert_eq!(rect("sh_g0"), Rect::new(21, 43, 318, 108));
        assert_eq!(rect("sh_g1"), Rect::new(21, 161, 318, 150));
        assert_eq!(rect("sh_local"), Rect::new(31, 54, 32, 32));
        assert_eq!(rect("sh_network"), Rect::new(31, 171, 32, 32));
        assert_eq!(
            ["sh_a0", "sh_b0", "sh_c0", "sh_d0"].map(|id| (rect(id).y, rect(id).height)),
            [(53, 26), (92, 26), (170, 39), (237, 26)]
        );
        assert_eq!(rect("sh_private"), Rect::new(70, 129, 13, 13));
        assert_eq!(p.focus(), None, "nothing on it is live");
        lines_fit(&p);
    }

    #[test]
    fn the_compatibility_page_lands_on_its_grid() {
        let p = compat_page();
        let rect = |id: &str| p.item(id).expect(id).rect;
        assert_eq!(
            ["cp_g0", "cp_g1", "cp_g2"].map(rect),
            [
                Rect::new(21, 103, 315, 73),
                Rect::new(21, 197, 315, 92),
                Rect::new(21, 311, 315, 40)
            ]
        );
        assert_eq!(rect("cp_mode"), Rect::new(37, 124, 13, 13));
        assert_eq!((rect("cp_list").x, rect("cp_list").y), (37, 144));
        assert_eq!(
            ["cp_256", "cp_640", "cp_themes", "cp_text"].map(|id| rect(id).y),
            [216, 241, 265, 327]
        );
        assert_eq!(p.focus(), None, "nothing on it is live");
        lines_fit(&p);
    }

    fn lines_fit(p: &Form) {
        let t = text();
        if !crate::text::test_tahoma() {
            return;
        }
        for it in &p.items {
            let lines = match &it.control {
                Control::Label { text: s, .. } => std::slice::from_ref(s),
                Control::Lines(l) | Control::GreyedLines(l) => l.as_slice(),
                _ => continue,
            };
            for s in lines {
                let w = t.measure(s, false, px());
                assert!(
                    w <= it.rect.width,
                    "{} is {w} px in {}",
                    it.id,
                    it.rect.width
                );
            }
        }
    }

    #[test]
    #[ignore]
    fn render_tab_pages() {
        let t = text();
        let theme = Theme::default();
        let (general, _) = page(true);
        let mut s = sheet(general, 424, &[Tab::Sharing, Tab::Compatibility], |_| {
            Form::new(vec![])
        });
        for (i, name) in [(1, "filesheet-sharing.png"), (2, "filesheet-compat.png")] {
            s.open_tab(i);
            let mut c = Canvas::new(SHEET_W as u32, s.size.1 as u32);
            c.fill_rect(c.bounds(), theme.colors.button_face);
            crate::controls::propsheet::draw(&mut c, &t, &theme.paint(), &s, true, px());
            crate::raster::write_preview(&c, name);
        }
    }

    #[test]
    #[ignore]
    fn render_file_sheet() {
        let t = text();
        let theme = Theme::default();
        let (page, height) = page(false);
        let sheet =
            crate::controls::propsheet::PropSheet::new(&["General"], vec![page], (SHEET_W, height));
        let mut c = Canvas::new(SHEET_W as u32, height as u32);
        c.fill_rect(c.bounds(), theme.colors.button_face);
        crate::controls::propsheet::draw(&mut c, &t, &theme.paint(), &sheet, true, px());
        crate::raster::write_preview(&c, "filesheet-file.png");
    }

    #[test]
    fn a_folders_read_only_box_has_three_states() {
        let (mut folder, _) = page(true);
        assert!(matches!(
            folder.item(HIDDEN).map(|i| &i.control),
            Some(Control::Check { enabled: true, .. })
        ));
        assert!(matches!(
            folder.item(ADVANCED).map(|i| &i.control),
            Some(Control::Button { enabled: true, .. })
        ));
        assert_eq!(read_only(&folder), CheckState::Mixed);
        folder.focus_id(READ_ONLY);
        let mut seen = Vec::new();
        for _ in 0..3 {
            folder.key(Key::Space);
            seen.push(read_only(&folder));
        }
        assert_eq!(seen, [CheckState::Off, CheckState::On, CheckState::Mixed]);
        let (mut file, _) = page(false);
        assert_eq!(read_only(&file), CheckState::Off);
        file.focus_id(READ_ONLY);
        file.key(Key::Space);
        file.key(Key::Space);
        assert_eq!(read_only(&file), CheckState::Off, "a file's box flips");
    }

    #[test]
    fn hidden_stands_for_the_leading_dot() {
        assert_eq!(shown_name(".notes.txt", ".txt", true), "notes");
        assert_eq!(shown_name("notes.txt", ".txt", false), "notes");
        assert_eq!(wanted_name("notes", true, ".txt"), ".notes.txt");
        assert_eq!(wanted_name("notes", false, ""), "notes");
    }

    #[test]
    fn advanced_attributes_land_on_their_grid() {
        let (t, px) = (text(), px());
        for (is_dir, size, lines, groups, checks, details, ok) in [
            (
                false,
                (378, 263),
                vec![22],
                [62, 148],
                [83, 104, 168, 191],
                186,
                229,
            ),
            (
                true,
                (378, 285),
                vec![16, 34, 47],
                [82, 170],
                [103, 126, 191, 213],
                209,
                251,
            ),
        ] {
            let subject = Subject::single(is_dir);
            assert_eq!(advanced_size(subject), size);
            let f = advanced_form(subject, None, None, &t, px, TextUnits::ONE);
            let rect = |id: &str| f.item(id).expect(id).rect;
            assert_eq!(rect("icon"), Rect::new(11, 12, 32, 32));
            let ys: Vec<i32> = ["adv_l0", "adv_l1", "adv_l2"]
                .iter()
                .filter_map(|id| f.item(id).map(|i| i.rect.y))
                .collect();
            assert_eq!(ys, lines, "dir {is_dir}");
            assert_eq!(rect("adv_g0"), Rect::new(11, groups[0], 357, 70));
            assert_eq!(rect("adv_g1").y, groups[1]);
            assert_eq!((rect("adv_t0").x, rect("adv_t0").y), (20, groups[0] - 5));
            for (id, y) in ["adv_archive", "adv_index", "adv_compress", "adv_encrypt"]
                .into_iter()
                .zip(checks)
            {
                assert_eq!(rect(id), Rect::new(21, y, 13, 13), "{id}");
                assert!(matches!(
                    f.item(id).map(|i| &i.control),
                    Some(Control::Check {
                        enabled: false,
                        checked: false,
                        ..
                    })
                ));
            }
            assert_eq!(rect("adv_details"), Rect::new(285, details, 75, 23));
            assert_eq!(rect(ADV_OK), Rect::new(213, ok, 75, 23));
            assert_eq!(rect(ADV_CANCEL), Rect::new(294, ok, 75, 23));
            assert_eq!(f.default_button().map(|i| f.items[i].id), Some(ADV_OK));
        }
    }

    #[test]
    fn an_advanced_group_title_breaks_its_top_line() {
        let t = text();
        if !crate::text::test_tahoma() {
            return;
        }
        let theme = Theme::default();
        let cs = &theme.colors;
        let f = advanced_form(Subject::File, None, None, &t, px(), TextUnits::ONE);
        let (w, h) = advanced_size(Subject::File);
        let mut c = Canvas::new(w as u32, h as u32);
        c.fill_rect(c.bounds(), cs.button_face);
        crate::controls::form::draw(&mut c, &t, &classic_paint(cs, 1), &f, false, px());
        let g = f.item("adv_g0").unwrap().rect;
        let tw = t.measure("Archive and Index attributes", false, px());
        assert_eq!(c.get(g.x + 6, g.y), cs.button_shadow.argb(), "line before");
        assert_eq!(c.get(g.x + 7, g.y), cs.button_face.argb(), "the break");
        let back = g.x + 9 + tw + 2;
        assert_eq!(c.get(back - 1, g.y), cs.button_face.argb(), "break end");
        assert_eq!(c.get(back, g.y), cs.button_shadow.argb(), "line after");
        let ink = (g.x..g.x + 20)
            .find(|&x| (g.y - 5..g.y + 8).any(|y| c.get(x, y) == cs.button_text.argb()));
        assert_eq!(ink, Some(g.x + 9), "the title's first ink");
    }

    fn bin_facts() -> BinFacts {
        BinFacts {
            type_name: "Rich Text Document".into(),
            origin: "Desktop".into(),
            size: "4.00 KB".into(),
            deleted: Some("Monday, September 14, 2026, 3:43:39 PM".into()),
            created: Some("Monday, September 14, 2026, 3:43:34 PM".into()),
            read_only: false,
            hidden: false,
        }
    }

    #[test]
    fn a_bin_items_page_lands_on_its_grid() {
        let page = bin_page(&bin_facts(), "New Rich Text Document", None);
        let rect = |id: &str| page.item(id).expect(id).rect;
        assert_eq!(rect("icon"), Rect::new(21, 40, 32, 32));
        assert_eq!((rect(NAME).x, rect(NAME).y), (63, 47));
        assert!(matches!(
            page.item(NAME).map(|i| &i.control),
            Some(Control::Label { .. })
        ));
        let seps = ["sep0", "sep1", "sep2"].map(&rect);
        assert_eq!(seps.map(|r| r.y), [81, 166, 229]);
        assert!(seps.iter().all(|r| (r.x, r.right()) == (21, 339)));
        for (id, y) in [("l_type", 94), ("l_origin", 115), ("l_size", 136)] {
            assert_eq!((rect(id).x, rect(id).y), (21, y), "{id}");
        }
        assert_eq!(rect("v_origin").x, 85);
        for (id, y) in [("l_deleted", 179), ("l_made", 200)] {
            assert_eq!((rect(id).x, rect(id).y), (21, y), "{id}");
        }
        assert_eq!(rect("v_deleted").x, 115);
        assert_eq!(rect("l_attr").y, 242);
        for (id, x, y) in [
            (READ_ONLY, 112, 243),
            (HIDDEN, 204, 243),
            ("archive", 112, 262),
            ("compress", 112, 282),
            ("encrypt", 112, 301),
        ] {
            assert_eq!(rect(id), Rect::new(x, y, 13, 13), "{id}");
            assert!(
                matches!(
                    page.item(id).map(|i| &i.control),
                    Some(Control::Check { enabled: false, .. })
                ),
                "{id} is greyed"
            );
        }
        assert_eq!(rect(RESTORE), Rect::new(265, 344, 75, 23));
        assert_eq!((SHEET_W, BIN_SHEET_H), (361, 418));
    }

    #[test]
    fn a_bin_item_without_a_date_drops_the_row() {
        let facts = BinFacts {
            deleted: None,
            ..bin_facts()
        };
        let page = bin_page(&facts, "x", None);
        assert!(page.item("l_deleted").is_none());
        assert_eq!(page.item("l_made").map(|i| i.rect.y), Some(179));
    }

    #[test]
    fn a_bin_items_page_inks_on_its_grid() {
        let t = text();
        if !crate::text::test_tahoma() {
            return;
        }
        let theme = Theme::default();
        let cs = &theme.colors;
        let page = bin_page(&bin_facts(), "New Rich Text Document", None);
        let mut c = Canvas::new(SHEET_W as u32, BIN_SHEET_H as u32);
        c.fill_rect(c.bounds(), cs.button_face);
        crate::controls::form::draw(&mut c, &t, &classic_paint(cs, 1), &page, false, px());
        let ink = |r: Rect| {
            let hits: Vec<(i32, i32)> = (r.y..r.bottom())
                .flat_map(|y| (r.x..r.right()).map(move |x| (x, y)))
                .filter(|&(x, y)| c.get(x, y) == cs.button_text.argb())
                .collect();
            (
                hits.iter().map(|p| p.0).min(),
                hits.iter().map(|p| p.1).min(),
            )
        };
        let at = |id: &str| page.item(id).unwrap().rect;
        let got: Vec<_> = [
            NAME,
            "l_type",
            "v_type",
            "l_origin",
            "v_origin",
            "l_size",
            "v_size",
            "l_deleted",
            "v_deleted",
            "l_made",
            "v_made",
            "l_attr",
        ]
        .into_iter()
        .map(|id| (id, ink(at(id))))
        .collect();
        let want = [
            (NAME, (63, 49)),
            ("l_type", (21, 97)),
            ("v_type", (85, 96)),
            ("l_origin", (21, 118)),
            ("v_origin", (85, 117)),
            ("l_size", (21, 139)),
            ("v_size", (85, 139)),
            ("l_deleted", (21, 181)),
            ("v_deleted", (115, 181)),
            ("l_made", (21, 202)),
            ("v_made", (115, 202)),
            ("l_attr", (21, 244)),
        ]
        .map(|(id, (x, y))| (id, (Some(x), Some(y))));
        assert_eq!(got, want);
    }

    #[test]
    #[ignore]
    fn render_bin_item_sheet() {
        let theme = Theme::default();
        let t = text();
        let page = bin_page(&bin_facts(), "New Rich Text Document", None);
        let sheet = crate::controls::propsheet::PropSheet::new(
            &["General"],
            vec![page],
            (SHEET_W, BIN_SHEET_H),
        );
        let mut c = Canvas::new(SHEET_W as u32, BIN_SHEET_H as u32);
        c.fill_rect(c.bounds(), theme.colors.button_face);
        crate::controls::propsheet::draw(
            &mut c,
            &t,
            &classic_paint(&theme.colors, 1),
            &sheet,
            true,
            px(),
        );
        crate::raster::write_preview(&c, "filesheet-bin.png");
    }

    #[test]
    fn a_blue_advanced_group_title_inks_like_a_group_box() {
        let t = text();
        if !crate::text::test_tahoma() {
            return;
        }
        let blue = crate::interfaces::theme_for(crate::interface::Interface::Blue);
        let cs = &blue.colors;
        let f = advanced_form(Subject::File, None, None, &t, px(), TextUnits::ONE);
        let (w, h) = advanced_size(Subject::File);
        let mut c = Canvas::new(w as u32, h as u32);
        c.fill_rect(c.bounds(), cs.button_face);
        crate::controls::form::draw(&mut c, &t, &blue.paint(), &f, false, px());
        let title = f.item("adv_t0").unwrap().rect;
        let count = |ink: u32| {
            (title.y..title.bottom())
                .flat_map(|y| (title.x..title.right()).map(move |x| (x, y)))
                .filter(|&(x, y)| c.get(x, y) == ink)
                .count()
        };
        let label = crate::blue::Variant::Blue.scheme().inks.group_label.argb();
        assert!(count(label) > 0, "the title inks the group box label");
        assert_eq!(count(cs.button_text.argb()), 0, "no black in the title");
    }
}
