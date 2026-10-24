//! The General page: its rows, its form and what a host reads back from it.

use super::{
    ADVANCED, APP_ICON_X, APP_NAME_X, CHANGE, CHECK_DY, CHECK_PITCH, CHECK_X, COUNT, EXECUTABLE,
    Executable, FIELD, FIELD_TO_SEP, FILE_ADVANCED_RIGHT, FILE_HIDDEN_X, Facts, HIDDEN, ICON_PX,
    ICON_Y, LABEL_X, NAME, NAME_FIELD, PAGE_BOTTOM, READ_ONLY, ROW_BUTTON_DY, ROW_BUTTON_RIGHT,
    ROW_BUTTON_W, ROW_PITCH, Row, SEP_IDS, SEP_RIGHT, SEP_TO_ROW, SET_SHEET_H, SetFacts, Subject,
    TAIL_INLINE, TAIL_STACKED, button, check, check_enabled, sep, tick,
};
use crate::classic::widgets::{BUTTON_GAP, BUTTON_H, BUTTON_W, CheckState};
use crate::controls::form::{Control, Form, Item};
use crate::raster::{Canvas, Rect};
use crate::text::escape_mnemonic;
use crate::textedit::TextEdit;

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
        executable: Some(Executable {
            on: facts.executable && !facts.is_dir,
            enabled: facts.can_set_executable && !facts.is_dir,
        }),
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
            executable: None,
            subject: Subject::Set {
                has_dir: facts.has_dir,
            },
        },
    ]
}

/// A set's 32 px picture at `s` device px a px; [`SET_ICON`] names its small icon.
pub fn set_picture(s: i32) -> Canvas {
    let s = s.max(1);
    let mut c = Canvas::new((ICON_PX * s) as u32, (ICON_PX * s) as u32);
    crate::art::icons::SET.paint(&mut c, 0, 0, s);
    c
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
                    Row::Count { text } => Item::label_at(
                        COUNT,
                        FIELD.x,
                        FIELD.y,
                        SEP_RIGHT - FIELD.x,
                        &escape_mnemonic(text),
                    ),
                    _ => Item::new(
                        NAME,
                        FIELD,
                        Control::Edit {
                            edit: TextEdit::new(name, true),
                            first: std::cell::Cell::new(0),
                            enabled: true,
                        },
                    )
                    .named(NAME_FIELD),
                });
                y = FIELD.bottom() + FIELD_TO_SEP;
            }
            Row::Separator => {
                let id = SEP_IDS[seps.min(SEP_IDS.len() - 1)];
                seps += 1;
                items.push(sep(id, y, SEP_RIGHT));
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
                    Item::label_at(ids.1, value_x, y, w, &escape_mnemonic(value))
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
                // Change... holds its place: a longer name runs onto a second line beside it.
                items.push(Item::lines(
                    "v_opens",
                    Rect::new(APP_NAME_X, y, change_x - BUTTON_GAP - APP_NAME_X, ROW_PITCH),
                    &[name],
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
                executable,
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
                if let Some(x) = executable {
                    items.push(check_enabled(
                        EXECUTABLE,
                        CHECK_X,
                        hy + CHECK_PITCH,
                        "&Executable",
                        tick(x.on),
                        x.enabled,
                    ));
                }
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

/// The Executable box's tick; false on a page without one.
pub fn executable(page: &Form) -> bool {
    page.checked(EXECUTABLE).unwrap_or(false)
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
