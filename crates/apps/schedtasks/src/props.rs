//! A task's Properties: its schedule, runs, command and files, read only. The sheet follows the
//! timer while it is open.

use crate::model::Task;
use reprisekit::controls::form::{Control, Form, Item};
use reprisekit::controls::propsheet::{self, PropSheet};
use reprisekit::raster::{Canvas, Rect};
use reprisekit::textedit::TextEdit;
use reprisekit::tr;

pub const CLIENT: (i32, i32) = (361, 419);

const LABEL_X: i32 = 18;
const VALUE_X: i32 = 110;
const VALUE_W: i32 = 232;
const PITCH: i32 = 22;

const NAME: &str = "name";
const SCHEDULE: &str = "schedule";
const SETTINGS: &str = "settings";
const NEXT: &str = "next";
const LAST: &str = "last";
const RESULT: &str = "result";
const STATUS: &str = "status";
const RUNS: &str = "runs";
const COMMAND: &str = "command";
const FILE: &str = "file";

/// The rows under the name, each a label and its value: (value id, label).
fn rows() -> [(&'static str, String); 6] {
    [
        (SCHEDULE, tr!("props-schedule")),
        (SETTINGS, tr!("props-settings")),
        (NEXT, tr!("props-next")),
        (LAST, tr!("props-last")),
        (RESULT, tr!("props-result")),
        (STATUS, tr!("props-status")),
    ]
}

fn page(icon: Option<Canvas>) -> Form {
    let mut items = vec![
        Item::picture("icon", Rect::new(18, 44, 32, 32), icon),
        Item::label_at(NAME, 70, 54, 272, ""),
        Item::new("sep1", Rect::new(18, 90, 324, 2), Control::Separator),
    ];
    let labels = [
        "schedule_label",
        "settings_label",
        "next_label",
        "last_label",
    ];
    let labels = labels.into_iter().chain(["result_label", "status_label"]);
    for (i, ((id, label), label_id)) in rows().into_iter().zip(labels).enumerate() {
        let y = 102 + i as i32 * PITCH;
        items.push(Item::label_at(label_id, LABEL_X, y, 90, &label));
        items.push(Item::label_at(id, VALUE_X, y, VALUE_W, ""));
    }
    items.extend([
        Item::new("sep2", Rect::new(18, 238, 324, 2), Control::Separator),
        Item::label_at("runs_label", LABEL_X, 252, 90, &tr!("props-runs")),
        Item::label_at(RUNS, VALUE_X, 252, VALUE_W, ""),
        Item::label_at("command_label", LABEL_X, 278, 90, &tr!("props-command")),
        Item::edit(COMMAND, Rect::new(VALUE_X, 274, VALUE_W, 21), "", true).read_only(),
        Item::label_at("file_label", LABEL_X, 304, 90, &tr!("props-file")),
        Item::edit(FILE, Rect::new(VALUE_X, 300, VALUE_W, 21), "", true).read_only(),
    ]);
    Form::new(items)
}

/// Puts `text` in read-only field `id`, its start in view, unless it holds it already, which keeps
/// its caret and selection.
fn set_field(page: &mut Form, id: &str, text: &str) {
    if let Some(Control::Edit { edit, first, .. }) = page.item_mut(id).map(|i| &mut i.control)
        && edit.text != text
    {
        *edit = TextEdit::new(text, false);
        edit.set_read_only(true);
        edit.set_selection(0, 0);
        first.set(0);
    }
}

pub struct Props {
    /// The timer shown, `backup.timer`.
    pub timer: String,
    pub sheet: PropSheet,
    name: String,
}

impl Props {
    pub fn new(t: &Task, icon: Option<Canvas>, is_12h: bool) -> Props {
        let tabs = [tr!("props-general")];
        let tabs: Vec<&str> = tabs.iter().map(String::as_str).collect();
        let mut sheet = PropSheet::new(&tabs, vec![page(icon)], CLIENT);
        sheet.set_apply_enabled(false);
        let mut p = Props {
            timer: t.timer.clone(),
            sheet,
            name: String::new(),
        };
        p.refresh(t, is_12h);
        p
    }

    pub fn title(&self) -> String {
        tr!("props-title", name = self.name.as_str())
    }

    pub fn refresh(&mut self, t: &Task, is_12h: bool) {
        self.name = t.name().to_string();
        let page = &mut self.sheet.pages[0];
        page.set_text(NAME, &self.name);
        page.set_text(SCHEDULE, &t.schedule(is_12h));
        page.set_text(SETTINGS, &t.settings().join("; "));
        let next = t.next_text(is_12h);
        page.set_text(NEXT, if next.is_empty() { "-" } else { &next });
        page.set_text(LAST, &t.last_text(is_12h));
        page.set_text(RESULT, &t.result_text());
        page.set_text(STATUS, &t.status());
        page.set_text(RUNS, &t.service);
        set_field(page, COMMAND, t.command.lines().next().unwrap_or_default());
        set_field(page, FILE, &t.file);
    }

    /// Whether `ev` closes the sheet: OK, Cancel or Escape. Apply stays greyed.
    pub fn closes(ev: &propsheet::Event) -> bool {
        matches!(ev, propsheet::Event::Ok | propsheet::Event::Cancel)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: u64 = 1_000_000;

    fn sample() -> Task {
        Task {
            timer: "backup.timer".into(),
            service: "backup.service".into(),
            description: "Copy the photos".into(),
            timer_state: "active".into(),
            file_state: "enabled".into(),
            service_state: "inactive".into(),
            calendar: vec![("OnCalendar".into(), "*-*-* 03:00:00".into())],
            next: None,
            last: Some(1_900_000 * S),
            result: "success".into(),
            command: "/usr/bin/rsync -a /home/u/Photos /mnt\n/usr/bin/true".into(),
            file: "/home/u/.config/systemd/user/backup.timer".into(),
            ..Task::default()
        }
    }

    fn text(p: &Props, id: &str) -> String {
        match p.sheet.pages[0].item(id).map(|i| &i.control) {
            Some(Control::Label { text, .. }) => text.clone(),
            Some(Control::Edit { edit, .. }) => edit.text.clone(),
            _ => panic!("no {id}"),
        }
    }

    #[test]
    fn the_sheet_fits_its_text() {
        let over = reprisekit::interfaces::form_overflows(|_, _, _| {
            let s = Props::new(&sample(), None, false).sheet;
            s.tabs.labels.into_iter().zip(s.pages).collect()
        });
        assert!(over.is_empty(), "{}", over.join("\n"));
    }

    #[test]
    fn the_rows_show_the_schedule_runs_and_command() {
        let p = Props::new(&sample(), None, false);
        assert_eq!(p.title(), "Copy the photos Properties");
        assert_eq!(text(&p, SCHEDULE), "Every day at 3:00");
        assert_eq!(text(&p, SETTINGS), "OnCalendar=*-*-* 03:00:00");
        assert_eq!(text(&p, NEXT), "-");
        assert_eq!(text(&p, RESULT), "Succeeded");
        assert_eq!(text(&p, STATUS), "Ready");
        assert_eq!(text(&p, RUNS), "backup.service");
        assert_eq!(
            text(&p, COMMAND),
            "/usr/bin/rsync -a /home/u/Photos /mnt",
            "the first command line"
        );
        assert_eq!(text(&p, FILE), "/home/u/.config/systemd/user/backup.timer");
    }

    #[test]
    fn the_sheet_follows_the_timer_and_keeps_its_command_read_only() {
        let mut p = Props::new(&sample(), None, false);
        p.refresh(
            &Task {
                timer_state: "inactive".into(),
                command: "/usr/bin/other".into(),
                ..sample()
            },
            false,
        );
        assert_eq!(text(&p, STATUS), "Disabled");
        assert_eq!(text(&p, COMMAND), "/usr/bin/other");
        match p.sheet.pages[0].item(COMMAND).map(|i| &i.control) {
            Some(Control::Edit { edit, .. }) => {
                assert!(edit.is_read_only());
                assert_eq!(edit.caret(), 0, "the command's start in view");
            }
            _ => panic!("no command field"),
        }
    }
}
