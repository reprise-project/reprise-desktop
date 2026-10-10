//! The Add Item dialog the Start Menu page's Add... opens: pick a program or type a command.

use reprisefs::apps::{self, AppEntry};
use reprisekit::classic::widgets::{BUTTON_GAP, BUTTON_H, BUTTON_W};
use reprisekit::controls::form::{self, Control, Form, Item};
use reprisekit::controls::listbox::{self, ListBox};
use reprisekit::raster::{Canvas, Rect};
use reprisekit::textedit::TextEdit;

pub const CLIENT: (i32, i32) = (340, 319);

const MARGIN: i32 = 12;
const INNER_W: i32 = CLIENT.0 - 2 * MARGIN;
const LIST: Rect = Rect::new(MARGIN, 30, INNER_W, 10 * listbox::ROW_H + 4);
const LABEL_W: i32 = 64;
const FIELD_X: i32 = MARGIN + LABEL_W + 4;
const FIELD_W: i32 = CLIENT.0 - MARGIN - FIELD_X;
const ROW_Y: [i32; 2] = [LIST.y + LIST.height + 14, LIST.y + LIST.height + 42];
const LABEL_DY: i32 = 4;

const BTN_Y: i32 = CLIENT.1 - MARGIN - BUTTON_H;
const CANCEL_X: i32 = CLIENT.0 - MARGIN - BUTTON_W;
const OK_X: i32 = CANCEL_X - BUTTON_GAP - BUTTON_W;

pub const OK: &str = "ok";
pub const CANCEL: &str = "cancel";
pub const PROGRAMS: &str = "programs";
const NAME: &str = "name";
const COMMAND: &str = "command";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewItem {
    pub name: String,
    pub command: String,
    pub icon: Option<String>,
    pub app: Option<AppEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    None,
    Redraw,
    Accept(Box<NewItem>),
    Cancel,
}

pub struct AddItemDialog {
    pub form: Form,
    apps: Vec<AppEntry>,
    terminal: String,
    pub icons_due: bool,
}

fn label(id: &'static str, y: i32, text: &str) -> Item {
    Item::label(id, Rect::new(MARGIN, y + LABEL_DY, LABEL_W, 13), text)
}

fn edit(id: &'static str, y: i32) -> Item {
    Item::edit(id, Rect::new(FIELD_X, y, FIELD_W, 20), "", true)
}

fn button(id: &'static str, x: i32, text: &str, default: bool) -> Item {
    Item::button(
        id,
        Rect::new(x, BTN_Y, BUTTON_W, BUTTON_H),
        text,
        true,
        default,
    )
}

pub fn command_for(app: &AppEntry, terminal: &str) -> String {
    let argv: Vec<String> = apps::launch_argv(app, terminal)
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    apps::command_line(&argv)
}

impl AddItemDialog {
    /// nothing picked and both fields empty. Only the shown ones are
    /// listed: the app index also holds NoDisplay entries, which open
    /// files but are not started on their own.
    pub fn new(mut apps: Vec<AppEntry>, terminal: &str) -> AddItemDialog {
        apps.retain(|a| a.shown);
        let rows = apps.iter().map(|a| a.name.clone()).collect();
        let items = vec![
            Item::label(
                "intro",
                Rect::new(MARGIN, MARGIN, INNER_W, 13),
                "&Pick a program, or type a name and a command:",
            ),
            Item::new(
                PROGRAMS,
                LIST,
                Control::ListBox {
                    list: ListBox::new(rows, None),
                    icons: Vec::new(),
                },
            ),
            label("name_l", ROW_Y[0], "&Name:"),
            edit(NAME, ROW_Y[0]),
            label("command_l", ROW_Y[1], "&Command:"),
            edit(COMMAND, ROW_Y[1]),
            button(OK, OK_X, "OK", true),
            button(CANCEL, CANCEL_X, "Cancel", false),
        ];
        let mut d = AddItemDialog {
            form: Form::new(items),
            apps,
            terminal: terminal.to_string(),
            icons_due: true,
        };
        d.sync();
        d
    }

    pub fn set_icons(&mut self, get: &mut dyn FnMut(&str) -> Option<Canvas>) {
        let icons = self
            .apps
            .iter()
            .map(|a| get(a.icon.as_deref().unwrap_or("application-x-executable")))
            .collect();
        if let Some(Control::ListBox { icons: at, .. }) =
            self.form.item_mut(PROGRAMS).map(|i| &mut i.control)
        {
            *at = icons;
        }
        self.icons_due = false;
    }

    pub fn event(&mut self, ev: form::Event) -> Outcome {
        match ev {
            form::Event::Changed(PROGRAMS) => {
                if let Some(app) = self.picked() {
                    let (name, cmd) = (app.name.clone(), command_for(app, &self.terminal));
                    self.set_field(NAME, &name);
                    self.set_field(COMMAND, &cmd);
                }
                self.sync();
                Outcome::Redraw
            }
            form::Event::Changed(_) => {
                self.sync();
                Outcome::Redraw
            }
            form::Event::Pressed(OK) | form::Event::Default => match self.value() {
                Some(v) => Outcome::Accept(Box::new(v)),
                None => Outcome::None,
            },
            form::Event::Pressed(CANCEL) | form::Event::Cancel => Outcome::Cancel,
            form::Event::Redraw => Outcome::Redraw,
            _ => Outcome::None,
        }
    }

    pub fn value(&self) -> Option<NewItem> {
        let name = self.field(NAME).trim();
        let command = self.field(COMMAND).trim();
        if name.is_empty() || command.is_empty() {
            return None;
        }
        let app = self
            .picked()
            .filter(|a| command_for(a, &self.terminal) == command);
        Some(NewItem {
            name: name.to_string(),
            command: command.to_string(),
            icon: app.and_then(|a| a.icon.clone()),
            app: app.cloned(),
        })
    }

    fn picked(&self) -> Option<&AppEntry> {
        match self.form.item(PROGRAMS).map(|i| &i.control) {
            Some(Control::ListBox { list, .. }) => self.apps.get(list.selected?),
            _ => None,
        }
    }

    fn field(&self, id: &str) -> &str {
        match self.form.item(id).map(|i| &i.control) {
            Some(Control::Edit { edit, .. }) => &edit.text,
            _ => "",
        }
    }

    fn set_field(&mut self, id: &str, text: &str) {
        if let Some(Control::Edit { edit, first, .. }) =
            self.form.item_mut(id).map(|i| &mut i.control)
        {
            *edit = TextEdit::new(text, false);
            first.set(0);
        }
    }

    fn sync(&mut self) {
        let valid = self.value().is_some();
        let focused = self.form.focus() == self.form.index_of(OK);
        self.form.set_enabled(OK, valid);
        // A greyed button is no tab stop, so the focus cannot stay on it.
        if focused && !valid {
            self.form.focus_edge(false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reprisekit::textedit::EditKey;

    fn app(name: &str, exec: &str, terminal: bool) -> AppEntry {
        let text = format!(
            "[Desktop Entry]\nType=Application\nName={name}\nExec={exec}\nIcon={}\nTerminal={terminal}\n",
            name.to_lowercase()
        );
        apps::parse_desktop(&text, &format!("{}.desktop", name.to_lowercase())).unwrap()
    }

    fn dialog() -> AddItemDialog {
        AddItemDialog::new(
            vec![app("Editor", "gedit %U", false), app("Top", "htop", true)],
            "foot",
        )
    }

    fn pick(d: &mut AddItemDialog, i: usize) -> Outcome {
        if let Some(Control::ListBox { list, .. }) =
            d.form.item_mut(PROGRAMS).map(|i| &mut i.control)
        {
            list.selected = Some(i);
        }
        d.event(form::Event::Changed(PROGRAMS))
    }

    fn typed(d: &mut AddItemDialog, id: &str, text: &str) -> Outcome {
        d.form.focus_id(id);
        let ev = d.form.key(form::Key::Edit(EditKey::Insert(text.into())));
        d.event(ev)
    }

    #[test]
    fn a_picked_program_fills_the_name_and_its_command() {
        let mut d = dialog();
        assert_eq!(
            d.event(form::Event::Default),
            Outcome::None,
            "OK waits for both fields"
        );
        pick(&mut d, 0);
        let v = d.value().unwrap();
        assert_eq!((v.name.as_str(), v.command.as_str()), ("Editor", "gedit"));
        assert_eq!(v.icon.as_deref(), Some("editor"));
        assert_eq!(v.app.map(|a| a.id), Some("editor.desktop".into()));
        pick(&mut d, 1);
        assert_eq!(
            d.value().unwrap().command,
            "foot -e htop",
            "in the terminal"
        );
    }

    #[test]
    fn a_command_of_ones_own_drops_the_program() {
        let mut d = dialog();
        pick(&mut d, 0);
        typed(&mut d, COMMAND, " --new-window");
        let v = d.value().unwrap();
        assert_eq!(v.command, "gedit --new-window");
        assert_eq!((v.app, v.icon), (None, None));
        let mut d = dialog();
        typed(&mut d, NAME, "Files");
        typed(&mut d, COMMAND, "nautilus");
        assert!(
            matches!(d.event(form::Event::Pressed(OK)), Outcome::Accept(v) if v.command == "nautilus")
        );
    }

    #[test]
    fn a_nodisplay_entry_is_not_listed() {
        let helper = apps::parse_desktop_any(
            "[Desktop Entry]\nType=Application\nName=Helper\nExec=helper %f\nNoDisplay=true\n",
            "helper.desktop",
        )
        .unwrap();
        let mut d = AddItemDialog::new(
            vec![
                app("Editor", "gedit %U", false),
                helper,
                app("Top", "htop", false),
            ],
            "foot",
        );
        let Some(Control::ListBox { list, .. }) = d.form.item(PROGRAMS).map(|i| &i.control) else {
            panic!("the program list");
        };
        assert_eq!(list.items.len(), 2);
        pick(&mut d, 1);
        assert_eq!(d.value().unwrap().name, "Top");
    }

    #[test]
    fn everything_sits_inside_the_client() {
        let d = dialog();
        for i in &d.form.items {
            let r = i.rect;
            assert!(
                r.x >= 0 && r.y >= 0 && r.right() <= CLIENT.0 && r.bottom() <= CLIENT.1,
                "{} at {r:?}",
                i.id
            );
        }
        assert_eq!(d.form.item(OK).unwrap().rect, Rect::new(172, 284, 75, 23));
    }
}
