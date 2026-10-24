//! The Advanced Attributes and Confirm Attribute Changes boxes.

use super::{ICON_PX, Subject, button, check, shared_state};
use crate::classic::widgets::{BUTTON_GAP, BUTTON_H, BUTTON_W, CheckState, LINE_H};
use crate::controls::form::{CHECK, Control, Form, Item, RADIO_ROW};
use crate::raster::{Canvas, Rect};
use crate::text::TextRenderer;
use crate::theme::TextUnits;

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
                RADIO_ROW,
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
