//! Open Log: a service's last journal lines, read only, newest at the bottom, each as its time
//! and its message ([`reprisefs::journal::lines`]).

use reprisebus::services::Manager;

use crate::services::LogError;
use reprisekit::classic::widgets::{BUTTON_H, BUTTON_W};
use reprisekit::controls::form::{self, Control, Form, Item};
use reprisekit::controls::multiedit::{MultiEdit, Pos};
use reprisekit::raster::Rect;
use reprisekit::tr;

pub const CLIENT: (i32, i32) = (600, 400);

const MARGIN: i32 = 8;
pub const TEXT: &str = "text";
pub const REFRESH: &str = "refresh";
pub const CLOSE: &str = "close";

pub struct Log {
    pub manager: Manager,
    /// The service shown, `pipewire.service`.
    pub name: String,
    is_12h: bool,
    pub form: Form,
    /// A read of the log is under way.
    pub reading: bool,
}

pub enum Outcome {
    None,
    Redraw,
    Refresh,
    Close,
}

impl Log {
    /// The window for service `name`, saying it reads until [`Log::show`].
    pub fn new(manager: Manager, name: &str, is_12h: bool) -> Log {
        let by = CLIENT.1 - MARGIN - BUTTON_H;
        let memo = Rect::new(MARGIN, MARGIN, CLIENT.0 - 2 * MARGIN, by - 2 * MARGIN);
        let refresh_x = CLIENT.0 - MARGIN - 2 * BUTTON_W - 6;
        let items = vec![
            Item::memo(TEXT, memo, "", true).read_only(),
            Item::button_at(REFRESH, refresh_x, by, BUTTON_W, &tr!("log-refresh"), true),
            Item::button(
                CLOSE,
                Rect::new(CLIENT.0 - MARGIN - BUTTON_W, by, BUTTON_W, BUTTON_H),
                &tr!("log-close"),
                true,
                true,
            ),
        ];
        let mut l = Log {
            manager,
            name: name.into(),
            is_12h,
            form: Form::new(items),
            reading: false,
        };
        l.reading();
        l
    }

    pub fn title(&self) -> String {
        tr!("log-title", name = crate::services::shown_name(&self.name))
    }

    pub fn reading(&mut self) {
        self.reading = true;
        self.put(&tr!("log-reading"));
    }

    /// Shows the entries the journal gave, the last line in view, or why it gave nothing.
    pub fn show(&mut self, read: Result<String, LogError>) {
        self.reading = false;
        let text = match read.map(|json| reprisefs::journal::lines(&json, self.is_12h)) {
            Ok(t) if t.trim().is_empty() => tr!("log-none"),
            Ok(t) => t.trim_end().to_string(),
            Err(LogError::NotAService) => tr!("log-not-a-service"),
            Err(LogError::NotStarted) => tr!("log-not-started"),
            Err(LogError::TookTooLong) => tr!("log-too-long"),
            Err(LogError::Unwatched) => tr!("log-unwatched"),
            Err(LogError::Exited(code, why)) => {
                with_why(tr!("log-exited", code = code.to_string()), &why)
            }
            Err(LogError::Killed(why)) => with_why(tr!("log-killed"), &why),
        };
        self.put(&text);
    }

    fn put(&mut self, text: &str) {
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
            form::Event::Pressed(REFRESH) if self.reading => Outcome::None,
            form::Event::Pressed(REFRESH) => {
                self.reading();
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

/// `text`, then journalctl's own words on stderr as a paragraph when it wrote any.
fn with_why(text: String, why: &str) -> String {
    match why.is_empty() {
        true => text,
        false => format!("{text}\n\n{why}"),
    }
}
