//! The Shortcut page for launchers and symlinks, and the Web Document page.

use super::{CHANGE_ICON, ICON_PX, LABEL_X, SEP_RIGHT, button, field_texts, moves, sep};
use crate::classic::widgets::BUTTON_H;
use crate::controls::form::{Control, Form, Item};
use crate::raster::{Canvas, Rect};
use crate::text::escape_mnemonic;
use crate::textedit::TextEdit;

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
        Item::label_at(
            "sc_name",
            SC_NAME.0,
            SC_NAME.1,
            value_w,
            &escape_mnemonic(&facts.name),
        ),
        sep("sc_sep0", SC_SEPS[0].0, SC_SEPS[0].1),
        Item::label_at("sc_type_l", LABEL_X, SC_ROWS[0], label_w, "Target type:"),
        Item::label_at(
            "sc_type",
            SC_VALUE_X,
            SC_ROWS[0],
            value_w,
            &escape_mnemonic(&facts.target_type),
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
            &escape_mnemonic(&facts.target_location),
        ),
        label("sc_target_l", 0, "&Target:"),
        edit(SHORTCUT_IDS[0], 0, target, true, true),
        sep("sc_sep1", SC_SEPS[1].0, SC_SEPS[1].1),
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
    field_texts(page, SHORTCUT_IDS)
}

pub fn shortcut_moves(page: &Form, opened: &[String; 3]) -> Vec<(usize, String)> {
    moves(shortcut_values(page), opened)
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
    let mut form = Form::new(vec![
        Item::picture("icon", WD_ICON, icon),
        Item::label_at(
            "wd_name",
            WD_NAME.0,
            WD_NAME.1,
            SEP_RIGHT - WD_NAME.0,
            &escape_mnemonic(&facts.name),
        ),
        sep("wd_sep0", WD_SEPS[0], SEP_RIGHT),
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
        sep("wd_sep1", WD_SEPS[1], SEP_RIGHT),
        button(CHANGE_ICON, WD_CHANGE_ICON, "&Change Icon...", true),
    ]);
    form.focus_id(SHORTCUT_IDS[0]);
    form
}
