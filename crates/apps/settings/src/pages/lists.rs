//! The Startup, Shortcuts and location-access lists.

use super::{COLUMNS, LOCATION_COLUMNS, SHORTCUT_COLUMNS};
use crate::model::Settings;
use reprisefs::autostart::Entry;
use reprisekit::controls::form;
use reprisekit::controls::listview::{self, Column, ListView, Row};
use reprisekit::look::Paint;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::TextRenderer;
use reprisekit::theme::Theme;

fn columns(cols: &[(&str, i32)]) -> Vec<Column> {
    cols.iter()
        .map(|(title, w)| Column::new(title, *w))
        .collect()
}

/// Drop a selection the refilled rows no longer have and pull `top` back inside them.
fn keep_in_rows(lv: &mut ListView) {
    if lv.selection.focus.is_some_and(|i| i >= lv.rows.len()) {
        lv.selection.set_only(None);
    }
    lv.top = lv.top.min(lv.rows.len().saturating_sub(1));
}

pub fn new_list() -> ListView {
    ListView {
        columns: columns(&COLUMNS),
        checks: Some(Vec::new()),
        ..ListView::default()
    }
}

pub fn fit_list_columns(
    lv: &mut ListView,
    cm: &reprisekit::controls::metrics::ControlMetrics,
    rect: Rect,
) {
    lv.fill_beside_bar(1, rect, cm);
}

pub fn fill_list(lv: &mut ListView, entries: &[Entry], checks: &[bool]) {
    lv.rows = entries
        .iter()
        .map(|e| Row {
            icon: None,
            cells: vec![
                e.name.clone(),
                e.exec.clone(),
                crate::autostart::location(e).into(),
            ],
        })
        .collect();
    lv.checks = Some(checks.to_vec());
    keep_in_rows(lv);
}

pub fn new_shortcut_list() -> ListView {
    ListView {
        columns: columns(&SHORTCUT_COLUMNS),
        ..ListView::default()
    }
}

/// Give Action what is left beside the look's scrollbar, so the list that scrolls grows no horizontal bar.
pub fn fit_shortcut_columns(
    lv: &mut ListView,
    cm: &reprisekit::controls::metrics::ControlMetrics,
    rect: Rect,
) {
    lv.fill_beside_bar(1, rect, cm);
}

/// Refill `lv` from `s` in [`crate::model::row`]'s order: the desktop's binds
/// (a spawn shows its command after the action), each program shortcut as
/// "Program: description", then each program refused one. `name` gives a
/// program's name for its app id.
pub fn fill_shortcuts(lv: &mut ListView, s: &Settings, name: &dyn Fn(&str) -> String) {
    let row = |keys: String, what: String| Row {
        icon: None,
        cells: vec![keys, what],
    };
    let binds = s.shortcuts.iter().map(|sc| {
        let what = if sc.action == "spawn" {
            format!("spawn {}", sc.command)
        } else {
            sc.action.clone()
        };
        row(sc.keys.clone(), what)
    });
    let programs = s.programs.iter().map(|p| {
        let keys = if p.keys.is_empty() { "None" } else { &p.keys };
        row(keys.into(), crate::model::program_text(p, &name(&p.app)))
    });
    let denied =
        (s.denied.iter()).map(|app| row(String::new(), format!("{}: not allowed", name(app))));
    lv.rows = binds.chain(programs).chain(denied).collect();
    keep_in_rows(lv);
}

pub fn new_location_list() -> ListView {
    ListView {
        columns: columns(&LOCATION_COLUMNS),
        ..ListView::default()
    }
}

pub fn fit_location_columns(
    lv: &mut ListView,
    cm: &reprisekit::controls::metrics::ControlMetrics,
    rect: Rect,
) {
    lv.fill_beside_bar(0, rect, cm);
}

pub fn fill_locations(lv: &mut ListView, answers: &[(String, bool)]) {
    lv.rows = answers
        .iter()
        .map(|(name, allow)| Row {
            icon: None,
            cells: vec![
                name.clone(),
                if *allow { "Allowed" } else { "Denied" }.into(),
            ],
        })
        .collect();
    keep_in_rows(lv);
}

/// The location list while it is read: one row saying so, nothing picked.
pub fn show_locations_loading(lv: &mut ListView) {
    lv.rows = vec![Row {
        icon: None,
        cells: vec!["Loading...".into(), String::new()],
    }];
    lv.selection.set_only(None);
    lv.top = 0;
}

/// Draw a side list (Startup, Shortcuts, the file types or the Advanced page's location answers)
/// over the page the sheet already drew; `rect` is its [`list`](super::list),
/// [`shortcuts`](super::shortcuts) or [`locations`](super::locations) where the page sits now.
pub fn draw_list(
    c: &mut Canvas,
    text: &TextRenderer,
    theme: &Theme,
    lv: &ListView,
    rect: Rect,
    px: u32,
    s: i32,
) {
    listview::draw(
        c,
        text,
        &Paint { s, ..theme.paint() },
        lv,
        &listview::layout(lv, rect, &theme.controls()),
        None,
        px,
    );
}

/// The key the Startup list takes from a press, or `None` when it is not the
/// list's own. An editing key is degraded first: the space bar reaches a host
/// as an insert, and the list is not a form control.
pub fn list_key(key: &form::Key) -> Option<form::Key> {
    let flat = match key {
        form::Key::Edit(e) => form::flat_key(e)?,
        other => other.clone(),
    };
    matches!(
        flat,
        form::Key::Up
            | form::Key::Down
            | form::Key::Home
            | form::Key::End
            | form::Key::PageUp
            | form::Key::PageDown
            | form::Key::Space
    )
    .then_some(flat)
}
