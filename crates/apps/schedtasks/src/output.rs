//! View Output: the journal's last lines for a task's service, read only, newest at the bottom,
//! each as its time in the list's form and its message.

use reprisekit::classic::widgets::{BUTTON_H, BUTTON_W};
use reprisekit::controls::form::{self, Control, Form, Item};
use reprisekit::controls::multiedit::{MultiEdit, Pos};
use reprisekit::raster::Rect;
use reprisekit::tr;

pub const CLIENT: (i32, i32) = (600, 400);
/// The lines asked of the journal.
pub const LINES: u32 = 200;

const MARGIN: i32 = 8;
pub const TEXT: &str = "text";
pub const REFRESH: &str = "refresh";
pub const CLOSE: &str = "close";

pub struct Output {
    /// The service shown, `backup.service`.
    pub service: String,
    name: String,
    is_12h: bool,
    pub form: Form,
}

pub enum Outcome {
    None,
    Redraw,
    Refresh,
    Close,
}

impl Output {
    /// The window for `service` of task `name`, saying it reads until [`Output::show`].
    pub fn new(service: &str, name: &str, is_12h: bool) -> Output {
        let by = CLIENT.1 - MARGIN - BUTTON_H;
        let memo = Rect::new(MARGIN, MARGIN, CLIENT.0 - 2 * MARGIN, by - 2 * MARGIN);
        let refresh_x = CLIENT.0 - MARGIN - 2 * BUTTON_W - 6;
        let items = vec![
            Item::memo(TEXT, memo, "", true).read_only(),
            Item::button_at(
                REFRESH,
                refresh_x,
                by,
                BUTTON_W,
                &tr!("output-refresh"),
                true,
            ),
            Item::button(
                CLOSE,
                Rect::new(CLIENT.0 - MARGIN - BUTTON_W, by, BUTTON_W, BUTTON_H),
                &tr!("output-close"),
                true,
                true,
            ),
        ];
        let mut o = Output {
            service: service.into(),
            name: name.into(),
            is_12h,
            form: Form::new(items),
        };
        o.put(&tr!("output-reading"));
        o
    }

    pub fn title(&self) -> String {
        tr!("output-title", name = self.name.as_str())
    }

    /// Shows the entries the journal gave ([`journal`]), the last line in view, or why it gave
    /// nothing.
    pub fn show(&mut self, read: Result<String, String>) {
        let text = match read.map(|json| reprisefs::journal::lines(&json, self.is_12h)) {
            Ok(t) if t.trim().is_empty() => tr!("output-none"),
            Ok(t) => t.trim_end().to_string(),
            Err(why) => tr!("output-unread", why = why),
        };
        self.put(&text);
    }

    pub(crate) fn put(&mut self, text: &str) {
        if let Some(Control::Memo(m)) = self.form.item_mut(TEXT).map(|i| &mut i.control) {
            m.edit = MultiEdit::new(text);
            m.edit.set_read_only(true);
            m.edit.place(
                Pos {
                    line: usize::MAX,
                    col: 0,
                },
                false,
            );
        }
    }

    #[cfg(test)]
    pub fn text(&self) -> String {
        match self.form.item(TEXT).map(|i| &i.control) {
            Some(Control::Memo(m)) => m.edit.text(),
            _ => String::new(),
        }
    }

    pub fn event(&mut self, ev: form::Event) -> Outcome {
        match ev {
            form::Event::Pressed(REFRESH) => {
                self.put(&tr!("output-reading"));
                Outcome::Refresh
            }
            form::Event::Pressed(CLOSE) | form::Event::Default | form::Event::Cancel => {
                Outcome::Close
            }
            form::Event::None => Outcome::None,
            _ => Outcome::Redraw,
        }
    }
}

/// The last [`LINES`] entries the journals the user may read hold for user unit `unit`, its own
/// output and the manager's lines about it, as `journalctl --output=json` writes them.
pub fn journal(unit: &str) -> Result<String, String> {
    let out = std::process::Command::new("journalctl")
        .arg(format!("--user-unit={unit}"))
        .arg(format!("--lines={LINES}"))
        .args(["--no-pager", "--quiet"])
        .args(reprisefs::journal::JSON_ARGS)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        let why = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(match why.is_empty() {
            true => out.status.to_string(),
            false => why,
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}
