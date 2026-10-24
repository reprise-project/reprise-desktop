use super::*;
use crate::textedit::EditKey;
use crate::theme::TextUnits;
use reprisekit::controls::form::Key;
use reprisekit::controls::propsheet::{Event as SheetEvent, PropSheet, StripKey};

/// What a form dialog holds: a property sheet (tabs over one page each,
/// the sheet's buttons along the bottom), or one plain form.
#[derive(Clone, Debug, PartialEq)]
pub enum FormBody {
    Sheet(Box<PropSheet>),
    Plain(Box<reprisekit::controls::form::Form>),
}

impl FormBody {
    pub fn pages(&self) -> &[reprisekit::controls::form::Form] {
        match self {
            FormBody::Sheet(s) => &s.pages,
            FormBody::Plain(f) => std::slice::from_ref(&**f),
        }
    }

    pub fn pages_mut(&mut self) -> &mut [reprisekit::controls::form::Form] {
        match self {
            FormBody::Sheet(s) => &mut s.pages,
            FormBody::Plain(f) => std::slice::from_mut(&mut **f),
        }
    }
}

/// Which dialog a `FormDialog` is, so its events reach the right code.
#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    /// Recycle Bin Properties; `binprops::event`.
    RecycleBin(crate::binprops::BinSheet),
    /// Confirm Attribute Changes; `attr_event`.
    Attributes(AttrConfirm),
    /// File or folder Properties; `props_event`.
    Properties(Box<SheetBase>),
    /// Multi-selection Properties; `set_event`.
    SetProperties(SetBase),
    /// Advanced Attributes; `advanced_event`. True when its archive and index boxes are live.
    AdvancedAttributes(bool),
    /// Change Icon; `change_icon_event`.
    ChangeIcon,
    /// Confirm File/Folder Replace over a drop's batch; `replace_event`.
    Replace(Box<crate::dragview::Batch>),
    /// Programs Log Off, Turn Off or Restart waits for; `session_end_event`.
    EndSession,
    /// Location prompt for this desktop id; `location_event`.
    Location(String),
}

/// A form dialog: its size at 100% text, what it holds, and which dialog it is. The client sits
/// at `FORM_CLIENT` plus `frame_d`, which is the sheet's or the form's origin.
#[derive(Clone, Debug, PartialEq)]
pub struct FormDialog {
    pub size: (i32, i32),
    pub body: FormBody,
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
    Replace(reprisekit::controls::replacebox::Fields),
    EndSession(reprisekit::controls::endsession::Fields),
    Location(reprisekit::controls::locationprompt::Fields),
    Attributes {
        verb: String,
        subject: reprisekit::controls::filesheet::Subject,
    },
    Advanced {
        subject: reprisekit::controls::filesheet::Subject,
        robots: Option<[CheckState; 2]>,
    },
}

impl Measured {
    /// The client size at 100% and the form, for text set at `px` that grows by `u`.
    fn build(
        &self,
        text: &TextRenderer,
        px: u32,
        u: TextUnits,
    ) -> ((i32, i32), reprisekit::controls::form::Form) {
        use reprisekit::controls::{endsession, filesheet, locationprompt, replacebox};
        match self {
            Measured::Replace(f) => (
                replacebox::client(f, text, px, u),
                replacebox::form(f, text, px, u),
            ),
            Measured::EndSession(f) => (
                endsession::client(f, text, px, u),
                endsession::form(f, text, px, u),
            ),
            Measured::Location(f) => (
                locationprompt::client(f, text, px, u),
                locationprompt::form(f, text, px, u),
            ),
            Measured::Attributes { verb, subject } => (
                filesheet::ATTR_SIZE,
                filesheet::attr_form(verb, *subject, text, px, u),
            ),
            Measured::Advanced { subject, robots } => (
                filesheet::advanced_size(*subject),
                filesheet::advanced_form(*subject, None, *robots, text, px, u),
            ),
        }
    }
}

impl FormDialog {
    pub fn from_sheet(mut sheet: PropSheet, kind: FormKind) -> FormDialog {
        let (x, y) = FORM_CLIENT;
        sheet.place(FORM_CLIENT);
        let (w, h) = sheet.size;
        FormDialog {
            size: (w + 2 * x, h + x + y),
            body: FormBody::Sheet(Box::new(sheet)),
            kind,
            frame_d: (0, 0),
            measured: None,
        }
    }

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
        let (Some(m), FormBody::Plain(old)) = (&self.measured, &self.body) else {
            return;
        };
        let ((w, h), mut form) = m.build(text, px, u);
        form.carry(old);
        let (x, y) = FORM_CLIENT;
        form.origin = FORM_CLIENT;
        self.size = (w + 2 * x, h + x + y);
        self.body = FormBody::Plain(Box::new(form));
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
            body: FormBody::Plain(Box::new(form)),
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

    /// Fit a sheet to an outer `area` at the text size `u` ([`PropSheet::fit`]): one larger
    /// shows what fits and scrolls its page; one that fits shows whole. Idempotent.
    pub(super) fn fit_area(
        &mut self,
        u: TextUnits,
        text: &TextRenderer,
        px: u32,
        area: (i32, i32),
    ) {
        let pad = self.pad();
        if let FormBody::Sheet(s) = &mut self.body {
            s.fit((u.within(area.0 - pad.0), u.within(area.1 - pad.1)));
            self.fit_tabs(text, px);
        }
    }

    /// Make room for tabs that wrap onto more rows, the sheet growing unless it was cut;
    /// idempotent.
    pub fn fit_tabs(&mut self, text: &TextRenderer, px: u32) {
        let pad = self.pad();
        if let FormBody::Sheet(s) = &mut self.body {
            s.fit_tabs(text, px);
            let (w, h) = s.shown();
            self.size = (w + pad.0, h + pad.1);
        }
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
        match &mut self.body {
            FormBody::Sheet(s) => s.place((FORM_CLIENT.0 + d.0, FORM_CLIENT.1 + d.1)),
            FormBody::Plain(f) => f.origin = (f.origin.0 + dx, f.origin.1 + dy),
        }
    }

    pub fn sheet(&self) -> Option<&PropSheet> {
        match &self.body {
            FormBody::Sheet(s) => Some(&**s),
            FormBody::Plain(_) => None,
        }
    }

    pub fn sheet_mut(&mut self) -> Option<&mut PropSheet> {
        match &mut self.body {
            FormBody::Sheet(s) => Some(&mut **s),
            FormBody::Plain(_) => None,
        }
    }

    pub fn pages(&self) -> &[reprisekit::controls::form::Form] {
        self.body.pages()
    }

    pub fn pages_mut(&mut self) -> &mut [reprisekit::controls::form::Form] {
        self.body.pages_mut()
    }

    pub fn open_index(&self) -> usize {
        self.sheet().map_or(0, |s| s.tabs.open)
    }

    pub fn open_page(&self) -> Option<&reprisekit::controls::form::Form> {
        self.pages().get(self.open_index())
    }

    pub fn open_page_mut(&mut self) -> Option<&mut reprisekit::controls::form::Form> {
        let i = self.open_index();
        self.pages_mut().get_mut(i)
    }

    pub(crate) fn event(&mut self, ev: reprisekit::controls::form::Event) -> Outcome {
        let page = self.open_index();
        match self.kind {
            FormKind::RecycleBin(_) => crate::binprops::event(self, page, ev),
            FormKind::Attributes(_) => attr_event(self, page, ev),
            FormKind::Properties(_) => props_event(self, page, ev),
            FormKind::SetProperties(_) => set_event(self, page, ev),
            FormKind::Replace(_) => replace_event(self, ev),
            FormKind::EndSession => session_end_event(ev),
            FormKind::Location(ref id) => location_event(self, id.clone(), ev),
            FormKind::AdvancedAttributes(live) => advanced_event(self, live, ev),
            FormKind::ChangeIcon => change_icon_event(self, ev),
        }
    }

    /// The strip's answer as the dialog's; `None` for a key the page takes.
    fn strip_outcome(&mut self, sk: StripKey) -> Option<Outcome> {
        Some(match sk {
            StripKey::Pass => return None,
            StripKey::None => Outcome::Nothing,
            StripKey::Moved => Outcome::Redraw,
            StripKey::Turned => Outcome::Relayout,
        })
    }

    pub(super) fn cycle_page(&mut self, back: bool) -> Outcome {
        let Some(s) = self.sheet_mut() else {
            return Outcome::Nothing;
        };
        let sk = s.strip_key(&Key::Tab { back }, Some(back));
        self.strip_outcome(sk).unwrap_or(Outcome::Nothing)
    }

    /// A key for the open page, the strip's first; a sheet cut to its output scrolls the
    /// focused control into view.
    pub(super) fn key(&mut self, key: Key) -> Outcome {
        let out = self.page_key(key);
        match self.sheet_mut().map(|s| s.reveal_focus()) {
            Some(true) if out == Outcome::Nothing => Outcome::Redraw,
            _ => out,
        }
    }

    fn page_key(&mut self, key: Key) -> Outcome {
        if let Some(s) = self.sheet_mut() {
            let sk = s.strip_key(&key, None);
            if let Some(out) = self.strip_outcome(sk) {
                return out;
            }
        }
        // Backspace goes in as an edit key; the form turns it back into
        // `Key::Backspace` (and an insert into `Char` or `Space`) when no
        // edit field has the focus.
        let k = match key {
            Key::Backspace => Key::Edit(EditKey::Backspace { word: false }),
            k => k,
        };
        let Some(form) = self.open_page_mut() else {
            return Outcome::Nothing;
        };
        let ev = form.key(k);
        self.event(ev)
    }

    pub(super) fn click(&mut self, hit: Option<DialogHit>) -> Outcome {
        match hit {
            Some(DialogHit::Tab(i)) => {
                let Some(s) = self.sheet_mut() else {
                    return Outcome::Nothing;
                };
                let sk = s.tab_click(i);
                self.strip_outcome(sk).unwrap_or(Outcome::Nothing)
            }
            Some(DialogHit::Control(h)) => {
                let Some(form) = self.open_page_mut() else {
                    return Outcome::Nothing;
                };
                let ev = form.click(Some(h));
                if let Some(s) = self.sheet_mut() {
                    s.page_clicked();
                }
                self.event(ev)
            }
            Some(DialogHit::PageBar(h)) => match self.sheet_mut() {
                Some(s) => redraw_if(s.click(Some(h)) != SheetEvent::None),
                None => Outcome::Nothing,
            },
            _ => {
                let Some(form) = self.open_page_mut() else {
                    return Outcome::Nothing;
                };
                let ev = form.click(None);
                self.event(ev)
            }
        }
    }
}

/// Change Icon: OK/Enter hand the pick to the sheet (standard folder as None), Restore Defaults
/// picks the standard folder.
fn change_icon_event(fd: &mut FormDialog, ev: reprisekit::controls::form::Event) -> Outcome {
    use reprisekit::controls::filesheet::{
        CHANGE_ICON_CANCEL, CHANGE_ICON_OK, CHANGE_ICON_RESTORE, STANDARD_FOLDER_ICON,
        change_icon_picked, change_icon_restore,
    };
    use reprisekit::controls::form::Event;
    let Some(form) = fd.body.pages_mut().first_mut() else {
        return Outcome::Nothing;
    };
    match ev {
        Event::Default | Event::Pressed(CHANGE_ICON_OK) => {
            // A folder's box (the one with Restore Defaults) stands the
            // standard folder for no icon of its own.
            let folder = form.item(CHANGE_ICON_RESTORE).is_some();
            let picked =
                change_icon_picked(form).filter(|n| !(folder && n == STANDARD_FOLDER_ICON));
            Outcome::IconChosen(picked)
        }
        Event::Pressed(CHANGE_ICON_RESTORE) => {
            change_icon_restore(form);
            Outcome::Redraw
        }
        Event::Cancel | Event::Pressed(CHANGE_ICON_CANCEL) => Outcome::Close,
        Event::Redraw | Event::Changed(_) => Outcome::Redraw,
        Event::None | Event::Pressed(_) => Outcome::Nothing,
    }
}

/// Advanced Attributes: OK/Enter hand the boxes to the sheet; with them greyed (`live` false) OK
/// closes like Cancel.
fn advanced_event(fd: &FormDialog, live: bool, ev: reprisekit::controls::form::Event) -> Outcome {
    use reprisekit::controls::filesheet::{ADV_CANCEL, ADV_OK, robots_chosen};
    use reprisekit::controls::form::Event;
    match ev {
        Event::Redraw | Event::Changed(_) => Outcome::Redraw,
        Event::None => Outcome::Nothing,
        Event::Default | Event::Pressed(ADV_OK) if live => match fd.pages().first() {
            Some(form) => Outcome::AdvancedChosen(robots_chosen(form)),
            None => Outcome::Close,
        },
        Event::Default | Event::Cancel | Event::Pressed(ADV_OK | ADV_CANCEL) => Outcome::Close,
        Event::Pressed(_) => Outcome::Nothing,
    }
}

/// Confirm Attribute Changes' events: OK and Enter re-emit the sheet's
/// write with the picked radio as the answer.
fn attr_event(fd: &FormDialog, page: usize, ev: reprisekit::controls::form::Event) -> Outcome {
    use reprisekit::controls::filesheet::{ATTR_CANCEL, ATTR_OK, attr_tree_picked};
    use reprisekit::controls::form::Event;
    let (FormKind::Attributes(a), Some(form)) = (&fd.kind, fd.pages().get(page)) else {
        return Outcome::Nothing;
    };
    match ev {
        Event::Default | Event::Pressed(ATTR_OK) => a.answered(attr_tree_picked(form)),
        Event::Cancel | Event::Pressed(ATTR_CANCEL) => Outcome::Close,
        Event::Redraw | Event::Changed(_) => Outcome::Redraw,
        Event::None | Event::Pressed(_) => Outcome::Nothing,
    }
}

/// The replace prompt's events: each button hands the batch back to the
/// host with its answer.
fn replace_event(fd: &FormDialog, ev: reprisekit::controls::form::Event) -> Outcome {
    use reprisefs::replace::Answer;
    use reprisekit::controls::form::Event;
    use reprisekit::controls::replacebox::{Choice, choice};
    let FormKind::Replace(batch) = &fd.kind else {
        return Outcome::Nothing;
    };
    let answer = match choice(&ev) {
        Some(Choice::Yes) => Answer::Yes,
        Some(Choice::YesToAll) => Answer::YesToAll,
        Some(Choice::No) => Answer::No,
        Some(Choice::Cancel) => Answer::Cancel,
        None if ev == Event::None => return Outcome::Nothing,
        None => return Outcome::Redraw,
    };
    Outcome::Replace {
        batch: batch.clone(),
        answer,
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
    let answer = fd
        .open_page()
        .and_then(|form| locationprompt::answer(form, &ev));
    match answer {
        Some(answer) => Outcome::Location { desktop_id, answer },
        None if ev == Event::None => Outcome::Nothing,
        None => Outcome::Redraw,
    }
}
