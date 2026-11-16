use super::*;
use crate::textedit::EditKey;
use crate::theme::TextUnits;
use reprisekit::controls::form::{Form, Key};

/// Which dialog a `FormDialog` is, so its events reach the right code.
#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    /// Programs Log Off, Turn Off or Restart waits for; `session_end_event`.
    EndSession,
    /// Location prompt for this desktop id; `location_event`.
    Location(String),
}

/// A form dialog: its size at 100% text, its form, and which dialog it is. The client sits
/// at `FORM_CLIENT` plus `frame_d`, which is the form's origin.
#[derive(Clone, Debug, PartialEq)]
pub struct FormDialog {
    pub size: (i32, i32),
    pub form: Box<Form>,
    pub kind: FormKind,
    /// Offset of the look's frame from the built client area (`client_shift`); set by
    /// [`FormDialog::fit_frame`].
    pub frame_d: (i32, i32),
    /// What a plain form laid out around measured text is built from; [`Dialog::fit`] builds
    /// it again for the text size and font it is laid out in.
    pub measured: Option<Measured>,
}

/// A plain form whose rows and size come from its text as measured and wrapped.
#[derive(Clone, Debug, PartialEq)]
pub enum Measured {
    EndSession(reprisekit::controls::endsession::Fields),
    Location(reprisekit::controls::locationprompt::Fields),
}

impl Measured {
    /// The client size at 100% and the form, for text set at `px` that grows by `u`.
    fn build(
        &self,
        text: &TextRenderer,
        px: u32,
        u: TextUnits,
    ) -> ((i32, i32), reprisekit::controls::form::Form) {
        use reprisekit::controls::{endsession, locationprompt};
        match self {
            Measured::EndSession(f) => (
                endsession::client(f, text, px, u),
                endsession::form(f, text, px, u),
            ),
            Measured::Location(f) => (
                locationprompt::client(f, text, px, u),
                locationprompt::form(f, text, px, u),
            ),
        }
    }
}

impl FormDialog {
    /// The form `m` builds for text set at `px` that grows by `u`.
    pub fn measured(
        m: Measured,
        kind: FormKind,
        text: &TextRenderer,
        px: u32,
        u: TextUnits,
    ) -> FormDialog {
        let (size, form) = m.build(text, px, u);
        FormDialog {
            measured: Some(m),
            ..FormDialog::from_client(size, form, kind)
        }
    }

    /// Build a measured form again for text set at `px` that grows by `u`, keeping what its
    /// controls hold; the frame is fitted again after.
    pub(super) fn remeasure(&mut self, text: &TextRenderer, px: u32, u: TextUnits) {
        let Some(m) = &self.measured else {
            return;
        };
        let ((w, h), mut form) = m.build(text, px, u);
        form.carry(&self.form);
        let (x, y) = FORM_CLIENT;
        form.origin = FORM_CLIENT;
        self.size = (w + 2 * x, h + x + y);
        *self.form = form;
        self.frame_d = (0, 0);
    }

    pub fn from_client(
        (w, h): (i32, i32),
        mut form: reprisekit::controls::form::Form,
        kind: FormKind,
    ) -> FormDialog {
        let (x, y) = FORM_CLIENT;
        form.origin = FORM_CLIENT;
        FormDialog::plain((w + 2 * x, h + x + y), form, kind)
    }

    /// A form dialog of outer `size`, the form's origin at its client's corner.
    pub fn plain(
        size: (i32, i32),
        form: reprisekit::controls::form::Form,
        kind: FormKind,
    ) -> FormDialog {
        FormDialog {
            size,
            form: Box::new(form),
            kind,
            frame_d: (0, 0),
            measured: None,
        }
    }

    /// The outer size at the text size `u`: the client grows, the frame and caption keep theirs.
    pub fn grown_size(&self, u: TextUnits) -> (i32, i32) {
        let pad = self.pad();
        let (w, h) = u.size((self.size.0 - pad.0, self.size.1 - pad.1));
        (w + pad.0, h + pad.1)
    }

    /// The frame and caption around the client, at every text size.
    fn pad(&self) -> (i32, i32) {
        let (x, y) = FORM_CLIENT;
        let (dx, dy) = self.frame_d;
        (2 * (x + dx), x + y + dx + dy)
    }

    /// Fit the dialog to a frame whose client area sits `d` from where it was built; idempotent.
    pub fn fit_frame(&mut self, d: (i32, i32)) {
        let (dx, dy) = (d.0 - self.frame_d.0, d.1 - self.frame_d.1);
        if (dx, dy) == (0, 0) {
            return;
        }
        self.size.0 += 2 * dx;
        self.size.1 += dx + dy;
        self.frame_d = d;
        let f = &mut self.form;
        f.origin = (f.origin.0 + dx, f.origin.1 + dy);
    }

    pub fn open_page(&self) -> &Form {
        &self.form
    }

    pub fn open_page_mut(&mut self) -> &mut Form {
        &mut self.form
    }

    /// Puts `icon` in the form's "icon" picture, for an icon that landed after the form was
    /// built. True when the picture changed.
    pub fn set_icon(&mut self, icon: crate::raster::Canvas) -> bool {
        use reprisekit::controls::form::Control;
        let form = &mut self.form;
        match form.item("icon").map(|i| &i.control) {
            Some(Control::Picture { canvas, .. }) if canvas.as_ref() != Some(&icon) => {}
            _ => return false,
        }
        form.set_picture("icon", icon);
        true
    }

    pub(crate) fn event(&self, ev: reprisekit::controls::form::Event) -> Outcome {
        match self.kind {
            FormKind::EndSession => session_end_event(ev),
            FormKind::Location(ref id) => location_event(self, id.clone(), ev),
        }
    }

    pub(super) fn key(&mut self, key: Key) -> Outcome {
        // Backspace goes in as an edit key; the form turns it back into
        // `Key::Backspace` (and an insert into `Char` or `Space`) when no
        // edit field has the focus.
        let k = match key {
            Key::Backspace => Key::Edit(EditKey::Backspace { word: false }),
            k => k,
        };
        let ev = self.form.key(k);
        self.event(ev)
    }

    pub(super) fn click(&mut self, hit: Option<DialogHit>) -> Outcome {
        let h = match hit {
            Some(DialogHit::Control(h)) => Some(h),
            _ => None,
        };
        let ev = self.form.click(h);
        self.event(ev)
    }
}

/// The session-end prompt's events: End Now and Cancel go to the host.
fn session_end_event(ev: reprisekit::controls::form::Event) -> Outcome {
    use reprisekit::controls::endsession::choice;
    use reprisekit::controls::form::Event;
    match choice(&ev) {
        Some(c) => Outcome::SessionEnd(c),
        None if ev == Event::None => Outcome::Nothing,
        None => Outcome::Redraw,
    }
}

/// The location prompt's events: Allow, Deny and closing it answer for
/// the program `desktop_id`.
fn location_event(
    fd: &FormDialog,
    desktop_id: String,
    ev: reprisekit::controls::form::Event,
) -> Outcome {
    use reprisekit::controls::form::Event;
    use reprisekit::controls::locationprompt;
    let answer = locationprompt::answer(&fd.form, &ev);
    match answer {
        Some(answer) => Outcome::Location { desktop_id, answer },
        None if ev == Event::None => Outcome::Nothing,
        None => Outcome::Redraw,
    }
}
