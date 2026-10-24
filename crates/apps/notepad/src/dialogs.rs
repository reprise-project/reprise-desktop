//! Go To, About and the error box, and the event loop the form dialogs share.

use super::*;

const GOTO_LINE: &str = "line";
const GOTO_OK: &str = "ok";
const GOTO_CANCEL: &str = "cancel";

pub(crate) fn goto_form(value: &str) -> Form {
    Form::new(vec![
        Item::new(
            "linelabel",
            Rect::new(8, 12, 72, 20),
            Control::Label {
                text: "&Line Number:".into(),
                enabled: true,
                align: reprisekit::text::Align::Left,
            },
        ),
        Item::new(
            GOTO_LINE,
            Rect::new(80, 12, 86, 20),
            Control::Edit {
                edit: TextEdit::new(value, true),
                first: std::cell::Cell::new(0),
                enabled: true,
            },
        ),
        Item::new(
            GOTO_OK,
            Rect::new(8, 56, widgets::BUTTON_W, widgets::BUTTON_H),
            Control::Button {
                label: "OK".into(),
                enabled: true,
                default: true,
            },
        ),
        Item::new(
            GOTO_CANCEL,
            Rect::new(114, 56, widgets::BUTTON_W, widgets::BUTTON_H),
            Control::Button {
                label: "Cancel".into(),
                enabled: true,
                default: false,
            },
        ),
    ])
}

pub(crate) type FormStep = oneshot::Step;

pub(crate) fn form_event(
    form: &mut Option<Form>,
    ev: &Event,
    text: &TextRenderer,
    px: u32,
    caret_on: bool,
) -> FormStep {
    match ev {
        Event::CloseRequest => FormStep::Closed,
        Event::Blink => {
            if let Some(f) = form {
                f.blink(caret_on);
            }
            FormStep::Redraw
        }
        Event::Key(k) => {
            let Some(key) = dialog_key(k) else {
                return FormStep::None;
            };
            let Some(f) = form else {
                return FormStep::None;
            };
            FormStep::Out(f.key(key))
        }
        Event::Pointer(PointerInput::Press {
            x,
            y,
            button: BTN_LEFT,
            ..
        }) => {
            let Some(f) = form else {
                return FormStep::None;
            };
            let hit = f.hit(*x, *y, text, px);
            FormStep::Out(f.click(hit))
        }
        Event::Pointer(PointerInput::Release {
            button: BTN_LEFT, ..
        }) => {
            let Some(f) = form else {
                return FormStep::None;
            };
            FormStep::Out(f.release())
        }
        Event::Pointer(PointerInput::Motion { x, y }) => {
            let Some(f) = form else {
                return FormStep::None;
            };
            let hit = f.hit(*x, *y, text, px);
            FormStep::Out(f.motion(*x, *y, hit))
        }
        Event::Pointer(PointerInput::Leave) => match form {
            Some(f) => FormStep::Out(f.leave()),
            None => FormStep::None,
        },
        Event::FieldDropped(e) => FormStep::Out(*e),
        Event::Pointer(PointerInput::Press {
            x,
            y,
            button: BTN_RIGHT,
            ..
        }) => match form {
            Some(f) => {
                let hit = f.hit(*x, *y, text, px);
                FormStep::Out(f.right_press(hit))
            }
            None => FormStep::None,
        },
        Event::Pasted(t) => match form {
            Some(f) => FormStep::Out(f.paste(t.clone())),
            None => FormStep::None,
        },
        _ => FormStep::None,
    }
}

/// A Notepad box with one OK button.
pub(crate) fn ok_box(text: String, icon: Option<Canvas>) -> MsgBox {
    MsgBox::new(
        "Notepad",
        msgbox::Fields {
            text,
            icon,
            buttons: vec!["OK".into()],
            default: 0,
            min_text_h: None,
            min_w: 0,
            max_w: repriseapp::dialog_max_w(),
        },
    )
}

impl Notepad {
    pub(crate) fn go_to_line(&mut self, value: &str) -> bool {
        let Ok(n) = value.trim().parse::<usize>() else {
            return false;
        };
        if n == 0 || n > self.edit.lines().len() {
            return false;
        }
        self.edit.place(
            multiedit::Pos {
                line: n - 1,
                col: 0,
            },
            false,
        );
        true
    }

    pub(crate) fn field(form: &Form, id: &str) -> String {
        match &form.item(id).expect("the field exists").control {
            Control::Edit { edit, .. } => edit.text.clone(),
            other => panic!("{id} is {other:?}, not an edit"),
        }
    }

    pub(crate) fn goto_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        let px = ctx.forms().fonts.size_px;
        match form_event(&mut self.goto, &ev, &ctx.text, px, ctx.caret_on) {
            FormStep::Closed => {
                self.goto = None;
                Reply::redraw_all()
            }
            FormStep::Redraw => Reply::redraw(),
            FormStep::None => Reply::none(),
            FormStep::Out(out) => self.goto_outcome(out, ctx),
        }
    }

    fn goto_outcome(&mut self, out: form::Event, ctx: &Ctx) -> Reply {
        match out {
            form::Event::Cancel | form::Event::Pressed(GOTO_CANCEL) => {
                self.goto = None;
                Reply::redraw_all()
            }
            form::Event::Default | form::Event::Pressed(GOTO_OK) => {
                let value = self
                    .goto
                    .as_ref()
                    .map(|f| Notepad::field(f, GOTO_LINE))
                    .unwrap_or_default();
                if self.go_to_line(&value) {
                    self.scroll_to_caret(ctx);
                    self.goto = None;
                }
                Reply::redraw_all()
            }
            form::Event::None => Reply::none(),
            _ => Reply::redraw(),
        }
    }

    pub(crate) fn error_box(&self, text: &str) -> MsgBox {
        ok_box(text.into(), self.warn_icon.clone())
    }

    pub(crate) fn about_fields(&self) -> aboutbox::Fields {
        repriseapp::about::fields("Notepad", self.app_icon.clone())
    }

    /// Laid out for the 100% font; the banner is drawn at the grown size.
    pub(crate) fn about_form(&self) -> Form {
        let (text, px) = (&self.dialog_text, self.dialog_px);
        let mut f = aboutbox::form(&self.about_fields(), text, px);
        let u = self.forms.text;
        let w = u.len(aboutbox::CLIENT.0);
        f.set_picture("banner", aboutbox::banner_wide(text, px, w, u));
        f
    }

    pub(crate) fn msgbox_size(&self, fields: &msgbox::Fields) -> (i32, i32) {
        let u = self.forms.text;
        u.size(msgbox::client(
            fields,
            &self.dialog_text,
            self.forms.fonts.size_px,
            u,
        ))
    }

    pub(crate) fn error_event(&mut self, ev: Event) -> Reply {
        let Some(b) = &mut self.error else {
            return Reply::none();
        };
        match b.event(&ev, &self.dialog_text, &self.forms) {
            oneshot::Outcome::Closed(_) => {
                self.error = None;
                Reply::redraw_all()
            }
            oneshot::Outcome::Redraw => Reply::redraw(),
            oneshot::Outcome::None => Reply::none(),
        }
    }

    /// About's own loop.
    pub(crate) fn about_event(&mut self, ev: Event) -> Reply {
        let step = oneshot::event(&ev, &self.dialog_text, &self.forms, || self.about_form());
        match step {
            FormStep::Closed => {
                self.about = false;
                Reply::redraw_all()
            }
            FormStep::Redraw => Reply::redraw(),
            FormStep::None => Reply::none(),
            FormStep::Out(out) => self.about_outcome(out),
        }
    }

    fn about_outcome(&mut self, out: form::Event) -> Reply {
        match out {
            form::Event::Cancel | form::Event::Default | form::Event::Pressed(aboutbox::OK) => {
                self.about = false;
                Reply::redraw_all()
            }
            form::Event::None => Reply::none(),
            _ => Reply::redraw(),
        }
    }
}
