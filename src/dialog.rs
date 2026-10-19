//! Compositor dialogs (Run, confirmations, Properties, Open With, Welcome panels, forms): model,
//! layout, drawing. `dialogview.rs` puts them on the scene.

use self::Kind as FormKind;
use crate::config::{Action, PowerConfig};
use crate::raster::{Canvas, Rect};
use crate::text::{Align, DialogFont, TextRenderer, access_key, split_mnemonic, wrap};
use crate::textedit::{EditKey, TextEdit};
use crate::theme::{Color, Metrics, TextUnits, Theme};
use reprisekit::classic::widgets::{
    BUTTON_GAP, BUTTON_H, BUTTON_W, BarHit, CheckState, LINE_H, draw_label,
};
use reprisekit::controls::combo::{self, Combo};
use reprisekit::controls::copyprogress::{self, CopyProgress};
use reprisekit::controls::dlgtemplate::{self, KEY_AID_TEXT_H, Template};
use reprisekit::controls::endsession::Goal;
use reprisekit::controls::form::{FocusedEdit, Key};
use reprisekit::controls::metrics::ControlMetrics;
use reprisekit::controls::openwith;
use reprisekit::controls::propsheet::{PropSheet, StripKey};
use reprisekit::controls::scrollbar::ThumbDrag;
use reprisekit::controls::traycustomize::{self, TrayCustomize};
use reprisekit::cues::Cues;
use reprisekit::frame::{CaptionButton, Chrome, Edges, FrameGeometry, FrameState};
use reprisekit::look::Paint;
use reprisekit::look::widgets::draw_button;
use reprisekit::pointer::{Interaction, Lit, Pointer, lit_of};
use reprisekit::style::TextSpec;

/// What pressing a button (or Enter for the default one) does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DialogAction {
    Cancel,
    /// Run the text field's content; inert while it is empty.
    RunField,
    /// Run's Browse...: open Browse, the pick lands in the field.
    BrowseRun,
    Spawn(String),
    /// End the session toward `Goal`, then run this command (`sessionend`).
    EndSession(Goal, String),
    /// Delete these paths, to the trash or, with `erase`, for good.
    Delete {
        paths: Vec<std::path::PathBuf>,
        erase: bool,
    },
    EmptyBin,
    Act(Action),
    HideSystemIcon(String),
    /// Take these `system_icons` off the desktop (none for No), then ask about deleting `files`.
    DeleteIcons {
        hide: Vec<String>,
        files: Vec<std::path::PathBuf>,
    },
    EndProgram(i32),
    TurnOn(crate::keyfilter::Aid),
    /// Answer the too-big-for-the-bin question, then ask the next or delete the batch.
    TooBig {
        asker: Box<reprisefs::bincap::TooBig>,
        answer: reprisefs::replace::Answer,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DialogButton {
    /// Label with an optional `&` access key.
    pub label: String,
    pub action: DialogAction,
    /// Drawn with the black default ring; Enter presses it.
    pub default: bool,
    /// Welcome panel icon button: the 32 px icon above the label.
    pub icon: Option<String>,
    /// Greyed out and inert (Stand By without power support).
    pub enabled: bool,
    /// Welcome balloon tip shown while the pointer rests on the button.
    pub tip: Option<String>,
}

/// Run's command line: an editable combo over earlier commands, newest first. Opens on the last
/// command selected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    pub label: String,
    pub combo: Combo,
    pub thumb: ThumbDrag,
}

pub const HISTORY_MAX: usize = 26;
pub const LIST_ROWS: usize = 3;

impl Field {
    pub fn edit(&self) -> Option<&TextEdit> {
        self.combo.edit.as_ref()
    }

    fn edit_mut(&mut self) -> Option<&mut TextEdit> {
        self.combo.edit.as_mut()
    }
}

/// Add `cmd` to the front of `history`, dropping an earlier copy and
/// anything past `HISTORY_MAX`.
pub fn remember(history: &mut Vec<String>, cmd: &str) {
    history.retain(|h| h != cmd);
    history.insert(0, cmd.to_string());
    history.truncate(HISTORY_MAX);
}

/// How the dialog is drawn, with the state only that kind has.
#[derive(Clone, Debug, PartialEq)]
pub enum Style {
    /// The classic grey dialog with a caption bar and a row of buttons.
    Classic {
        /// Which template gives the margins.
        template: Template,
        /// Text box height when taller than the text (Confirm Folder Delete: 51 px).
        text_h: Option<i32>,
        /// Red exclamation badge over the icon's bottom-left (Confirm Delete on a system icon).
        delete_badge: bool,
    },
    /// The "Open With" chooser: the `openwith` control in a fixed frame.
    Chooser(Box<Chooser>),
    /// Blue Welcome panel (Log Off, Turn off): title band with logo, icon buttons on a gradient,
    /// Cancel below. Under glass a framed dialog with push buttons.
    Welcome,
    /// Form controls at fixed positions, optionally with a tab strip over a page: Properties sheets
    /// and similar.
    Form(Box<FormDialog>),
    /// The Open file dialog in a resizable frame; `filepick.rs` resolves its folders.
    Browse(Box<crate::filepick::FilePicker>),
    /// End Program: classic dialog on a fixed 354x243 template (see `EP_PARA_Y`).
    EndProgram,
    /// The `traycustomize` control in a fixed frame.
    TrayCustomize(Box<TrayCustomize>),
    /// A desktop copy or move's progress: the `copyprogress` control in a fixed frame.
    Copy(Box<CopyDialog>),
}

/// The progress of the put into `dir`; `units` is the text growth its control is laid out for.
#[derive(Clone, Debug, PartialEq)]
pub struct CopyDialog {
    pub cp: CopyProgress,
    pub dir: std::path::PathBuf,
    pub units: reprisekit::theme::TextUnits,
}

/// What a folder's or a set's Properties sheet asked to write, held back
/// until the Confirm Attribute Changes dialog says how far it reaches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttrConfirm {
    pub write: AttrWrite,
    /// The attribute changes asked for besides Hidden (Read-only, and
    /// archive and index from Advanced Attributes).
    pub attrs: reprisefs::props::AttrMoves,
    /// Whether the sheet closes after the write (its OK rather than Apply).
    pub close: bool,
}

/// The rest of the write Confirm Attribute Changes holds: a folder's
/// rename and the Hidden state it takes, or a set's items and theirs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttrWrite {
    One {
        path: std::path::PathBuf,
        name: Option<String>,
        hidden: Option<bool>,
        folder_icon: Option<Option<String>>,
    },
    Set {
        paths: Vec<std::path::PathBuf>,
        hidden: Option<bool>,
        folder_icon: Option<Option<String>>,
    },
}

impl AttrConfirm {
    /// The sheet's write with `tree`, the answer, as how far Read-only
    /// and Hidden reach.
    fn answered(&self, tree: bool) -> Outcome {
        let (attrs, close, recurse) = (self.attrs, self.close, Some(tree));
        match &self.write {
            AttrWrite::One {
                path,
                name,
                hidden,
                folder_icon,
            } => Outcome::Properties {
                path: path.clone(),
                name: name.clone(),
                attrs,
                hidden: *hidden,
                // Only a folder is asked, and a folder has no Summary.
                summary: Vec::new(),
                launcher: Vec::new(),
                folder_icon: folder_icon.clone(),
                close,
                recurse,
            },
            AttrWrite::Set {
                paths,
                hidden,
                folder_icon,
            } => Outcome::SetProperties {
                paths: paths.clone(),
                attrs,
                hidden: *hidden,
                folder_icon: folder_icon.clone(),
                close,
                recurse,
            },
        }
    }

    /// The Hidden state asked for, None when the box did not move.
    pub fn hidden(&self) -> Option<bool> {
        match &self.write {
            AttrWrite::One { hidden, .. } | AttrWrite::Set { hidden, .. } => *hidden,
        }
    }

    fn path(&self) -> Option<&std::path::Path> {
        match &self.write {
            AttrWrite::One { path, .. } => Some(path),
            AttrWrite::Set { paths, .. } => paths.first().map(|p| p.as_path()),
        }
    }
}

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
    fn remeasure(&mut self, text: &TextRenderer, px: u32, u: TextUnits) {
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
        let (x, y) = FORM_CLIENT;
        let (dx, dy) = self.frame_d;
        let pad = (2 * (x + dx), x + y + dx + dy);
        let (w, h) = u.size((self.size.0 - pad.0, self.size.1 - pad.1));
        (w + pad.0, h + pad.1)
    }

    /// Make room for tabs that wrap onto more rows; idempotent.
    pub fn fit_tabs(&mut self, text: &TextRenderer, px: u32) {
        if let FormBody::Sheet(s) = &mut self.body {
            self.size.1 += s.fit_tabs(text, px);
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

    fn cycle_page(&mut self, back: bool) -> Outcome {
        let Some(s) = self.sheet_mut() else {
            return Outcome::Nothing;
        };
        let sk = s.strip_key(&Key::Tab { back }, Some(back));
        self.strip_outcome(sk).unwrap_or(Outcome::Nothing)
    }

    fn key(&mut self, key: Key) -> Outcome {
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

    fn click(&mut self, hit: Option<DialogHit>) -> Outcome {
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

/// Properties sheet: OK/Enter/Apply ask the host to write; Change.../Advanced... open dialogs;
/// Apply lights exactly while there is something to write.
fn props_event(
    fd: &mut FormDialog,
    _page: usize,
    ev: reprisekit::controls::form::Event,
) -> Outcome {
    use reprisekit::controls::filesheet::{ADVANCED, CHANGE};
    use reprisekit::controls::form::Event;
    use reprisekit::controls::propsheet::{APPLY, CANCEL, OK};
    let FormKind::Properties(base) = &fd.kind else {
        return Outcome::Nothing;
    };
    // The changes are the General page's, whichever page is open.
    let Some(form) = fd.pages().first() else {
        return Outcome::Nothing;
    };
    let summary = summary_changes(fd.pages(), base);
    let launcher = launcher_changes(fd.pages(), base);
    // A launcher's icon is one of its keys.
    let folder_icon = icon_move(base).filter(|_| base.launcher.is_none());
    let rest = summary.is_empty() && folder_icon.is_none() && launcher.is_empty();
    let write = |close: bool| match props_changes(form, base) {
        (None, attrs, None) if attrs.is_empty() && rest && close => Outcome::Close,
        (None, attrs, None) if attrs.is_empty() && rest => Outcome::Nothing,
        (name, attrs, hidden) => Outcome::Properties {
            path: base.path.clone(),
            name,
            attrs,
            hidden,
            summary: summary.clone(),
            folder_icon: folder_icon.clone(),
            launcher: launcher.clone(),
            close,
            recurse: None,
        },
    };
    match ev {
        Event::Default | Event::Pressed(OK) => write(true),
        Event::Pressed(APPLY) => write(false),
        Event::Cancel | Event::Pressed(CANCEL) => Outcome::Close,
        Event::Pressed(CHANGE) => Outcome::ChangeDefault(base.path.clone()),
        Event::Pressed(reprisekit::controls::filesheet::CHANGE_ICON) => Outcome::ChangeIcon {
            name: base
                .path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            current: base
                .icon_chosen
                .clone()
                .unwrap_or_else(|| base.own_icon.clone()),
            launcher: base.launcher.is_some(),
        },
        Event::Pressed(reprisekit::controls::filesheet::FIND_TARGET) => {
            find_target(fd.pages(), base)
        }
        Event::Pressed(ADVANCED) => Outcome::AdvancedAttributes {
            subject: reprisekit::controls::filesheet::Subject::single(base.is_dir),
            robots: base.robots.map(|opened| base.adv.unwrap_or(opened)),
        },
        Event::Changed(_) | Event::Redraw => {
            sync_apply(fd);
            Outcome::Redraw
        }
        Event::None | Event::Pressed(_) => Outcome::Nothing,
    }
}

/// Set Properties: as a file's, across the set.
fn set_event(fd: &mut FormDialog, _page: usize, ev: reprisekit::controls::form::Event) -> Outcome {
    use reprisekit::controls::filesheet::{ADVANCED, Subject};
    use reprisekit::controls::form::Event;
    use reprisekit::controls::propsheet::{APPLY, CANCEL, OK};
    let FormKind::SetProperties(set) = &fd.kind else {
        return Outcome::Nothing;
    };
    let Some(form) = fd.pages().first() else {
        return Outcome::Nothing;
    };
    let folder_icon = set.icon_chosen.clone();
    let write = |close: bool| match set_changes(form, set) {
        (attrs, None) if attrs.is_empty() && folder_icon.is_none() && close => Outcome::Close,
        (attrs, None) if attrs.is_empty() && folder_icon.is_none() => Outcome::Nothing,
        (attrs, hidden) => Outcome::SetProperties {
            paths: set.paths.clone(),
            attrs,
            hidden,
            folder_icon: folder_icon.clone(),
            close,
            recurse: None,
        },
    };
    match ev {
        Event::Default | Event::Pressed(OK) => write(true),
        Event::Pressed(APPLY) => write(false),
        Event::Cancel | Event::Pressed(CANCEL) => Outcome::Close,
        Event::Pressed(ADVANCED) => Outcome::AdvancedAttributes {
            subject: Subject::Set {
                has_dir: set.has_dir,
            },
            robots: set.robots().map(|opened| set.adv.unwrap_or(opened)),
        },
        Event::Pressed(reprisekit::controls::filesheet::CHANGE_ICON) => Outcome::ChangeIcon {
            name: set
                .paths
                .first()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            current: set.icon_chosen.clone().flatten(),
            launcher: false,
        },
        Event::Changed(_) | Event::Redraw => {
            sync_apply(fd);
            Outcome::Redraw
        }
        Event::None | Event::Pressed(_) => Outcome::Nothing,
    }
}

/// The folder icon Change Icon left, when it differs from the folder's
/// own.
fn icon_move(base: &SheetBase) -> Option<Option<String>> {
    base.icon_chosen.clone().filter(|c| *c != base.own_icon)
}

/// The shortcut keys OK and Apply would write: the Shortcut or Web
/// Document fields that moved off what the file said, then the icon
/// Change Icon left.
fn launcher_changes(
    pages: &[reprisekit::controls::form::Form],
    base: &SheetBase,
) -> Vec<(reprisefs::launcher::Key, String)> {
    use reprisekit::controls::filesheet::{shortcut_moves, shortcut_of};
    match (base.launcher.as_ref(), shortcut_of(pages)) {
        (Some(l), Some(page)) => l.edits(&shortcut_moves(page, &l.fields()), icon_move(base)),
        _ => Vec::new(),
    }
}

/// What a shortcut's Shortcut page shows: its name, and what it runs or
/// points at, read from `l`, its keys.
pub fn shortcut_facts(
    name: &str,
    l: &reprisefs::launcher::Launcher,
) -> reprisekit::controls::filesheet::ShortcutFacts {
    reprisekit::controls::filesheet::ShortcutFacts {
        name: name.to_string(),
        target_type: l.target_type.clone(),
        target_location: l.location.clone(),
        symlink: l.kind == reprisefs::launcher::Kind::Symlink,
        fields: l.fields(),
    }
}

/// What the sheet over `info` is about, for its tabs.
fn sheet_over(info: &reprisefs::props::FileInfo) -> reprisekit::controls::filesheet::Over {
    use reprisefs::launcher::Kind;
    use reprisekit::controls::filesheet::Over;
    match &info.launcher {
        _ if info.is_dir => Over::Folder,
        Some(l) => match l.kind {
            Kind::Program => Over::Launcher,
            Kind::Symlink => Over::Symlink { runs: l.runs },
            Kind::Web => Over::WebLink,
        },
        None => Over::File {
            shortcut: info.is_shortcut,
        },
    }
}

/// Find Target...: the program, or what the symlink points at, as the
/// Target field names it, for a window on its folder with it selected,
/// or an error box when nothing is found there.
fn find_target(pages: &[reprisekit::controls::form::Form], base: &SheetBase) -> Outcome {
    use reprisekit::controls::filesheet::{shortcut_of, shortcut_values};
    let (Some(page), Some(l)) = (shortcut_of(pages), base.launcher.as_ref()) else {
        return Outcome::Nothing;
    };
    let [target, ..] = shortcut_values(page);
    match l.find(&target, &base.path) {
        Some(found) => Outcome::FindTarget(found),
        None => Outcome::Notice(Box::new(Dialog::error(
            "Problem with Shortcut",
            l.not_found_text(&target),
        ))),
    }
}

/// The Summary fields that moved off what the file said, when the sheet
/// has a live Summary page.
fn summary_changes(
    pages: &[reprisekit::controls::form::Form],
    base: &SheetBase,
) -> Vec<(usize, String)> {
    use reprisekit::controls::filesheet::{summary_moves, summary_of};
    match (base.summary.as_ref(), summary_of(pages)) {
        (Some(opened), Some(page)) => summary_moves(page, opened),
        _ => Vec::new(),
    }
}

/// The archive and index moves Advanced Attributes' OK handed a sheet:
/// each flag its `chosen` boxes moved off `opened`, the states read.
fn robots_moves(
    opened: Option<[CheckState; 2]>,
    chosen: Option<[CheckState; 2]>,
) -> [Option<bool>; 2] {
    match (opened, chosen) {
        (Some(o), Some(c)) => reprisekit::controls::filesheet::robots_moves(o, c),
        _ => [None, None],
    }
}

/// What OK and Apply would write across a set from its page `form`: the
/// Read-only bit and the archive and index flags, and the Hidden state,
/// each when it moved off where it started to a settled state.
fn set_changes(
    form: &reprisekit::controls::form::Form,
    set: &SetBase,
) -> (reprisefs::props::AttrMoves, Option<bool>) {
    use reprisekit::controls::filesheet::{set_moves, shared_state};
    let (read_only, hidden) = set_moves(
        form,
        (shared_state(set.read_only), shared_state(set.hidden)),
    );
    let [backup, index] = robots_moves(set.robots(), set.adv);
    let attrs = reprisefs::props::AttrMoves {
        read_only,
        backup,
        index,
        executable: None,
    };
    (attrs, hidden)
}

/// What OK and Apply would write from sheet page `form`: the new name when the field or Hidden
/// changed it, attributes whose boxes moved to a settled state, and Hidden when its box moved. The
/// typed name is kept as is (spaces survive; all spaces renames nothing). Hidden dots the name
/// unless the folder's `.hidden` lists it.
fn props_changes(
    form: &reprisekit::controls::form::Form,
    base: &SheetBase,
) -> (Option<String>, reprisefs::props::AttrMoves, Option<bool>) {
    use reprisekit::controls::filesheet;
    let typed = filesheet::typed_name(form);
    let hidden = filesheet::hidden(form);
    let dotted = base
        .path
        .file_name()
        .is_some_and(|n| n.as_encoded_bytes().starts_with(b"."));
    let dot = reprisefs::hidden::wants_dot(hidden, dotted, base.listed);
    let name = filesheet::wanted_name(typed, dot, &base.ext);
    let same = base
        .path
        .file_name()
        .is_some_and(|n| n.to_string_lossy() == name);
    let rename = (!typed.trim().is_empty() && !same && !name.contains('/')).then_some(name);
    let read_only = match filesheet::read_only(form) {
        CheckState::Mixed => None,
        state if state == base.read_only => None,
        state => Some(state.is_on()),
    };
    let [backup, index] = robots_moves(base.robots, base.adv);
    let executable = filesheet::executable(form);
    let attrs = reprisefs::props::AttrMoves {
        read_only,
        backup,
        index,
        executable: (executable != base.executable).then_some(executable),
    };
    (rename, attrs, (hidden != base.hidden).then_some(hidden))
}

/// Light a sheet's Apply, on every page, exactly while its General page
/// (or Advanced Attributes' boxes) holds a change.
fn sync_apply(fd: &mut FormDialog) {
    let Some(form) = fd.body.pages().first() else {
        return;
    };
    let dirty = match &fd.kind {
        FormKind::Properties(base) => {
            let (name, attrs, hidden) = props_changes(form, base);
            name.is_some()
                || !attrs.is_empty()
                || hidden.is_some()
                || !summary_changes(fd.body.pages(), base).is_empty()
                || icon_move(base).is_some()
                || !launcher_changes(fd.body.pages(), base).is_empty()
        }
        FormKind::SetProperties(set) => {
            let (attrs, hidden) = set_changes(form, set);
            hidden.is_some() || !attrs.is_empty() || set.icon_chosen.is_some()
        }
        _ => return,
    };
    if let Some(sheet) = fd.sheet_mut() {
        sheet.set_apply_enabled(dirty);
    }
}

/// Where the dialog lands on the output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    Center,
    /// Centered across the usable area, top where a dialog this tall would have it.
    CenterAs(i32),
    /// Centered across the whole output, a third of the way down, for Welcome panels.
    UpperThird,
    /// Flush with the usable area's corner by the Start button
    /// ([`by_start`]), for Run.
    ByStart,
    /// Top-left corner at a layout point (an icon's cell), pulled inside
    /// the usable area, for a Properties sheet.
    At(i32, i32),
}

/// Top-left of a dialog `size` in the corner of `area` nearest the Start button of a bar along
/// `edge`, kept inside `area` (`window::keep_inside`).
pub fn by_start(area: Rect, edge: reprisekit::taskbar::Edge, size: (i32, i32)) -> (i32, i32) {
    use reprisekit::taskbar::Edge;
    let corner = match edge {
        Edge::Bottom => (area.x, area.bottom() - size.1),
        Edge::Top | Edge::Left => (area.x, area.y),
        Edge::Right => (area.right() - size.0, area.y),
    };
    crate::window::keep_inside(corner, size, area)
}

/// Where Run's Browse... dialog goes: `by_start`'s corner, 3 px in from the screen edge on Start's
/// side.
pub fn browse_by_start(
    area: Rect,
    edge: reprisekit::taskbar::Edge,
    size: (i32, i32),
) -> (i32, i32) {
    let (x, y) = by_start(area, edge, size);
    let inset = if edge == reprisekit::taskbar::Edge::Right {
        -3
    } else {
        3
    };
    (x + inset, y)
}

/// The Open With chooser: the `openwith` control, the file, its type, and whether OK only writes
/// the default (Change...).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chooser {
    pub ow: openwith::OpenWith,
    pub set_default: bool,
    pub path: std::path::PathBuf,
    pub mime: Option<String>,
}

/// What a Properties sheet opened on, so OK and Apply know what changed:
/// the file, the extension its name field leaves out, where Read-only
/// started, and the Opens with program's icon, which the host resolves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SheetBase {
    pub path: std::path::PathBuf,
    /// The name the sheet was opened under.
    pub name: String,
    pub ext: String,
    /// The file's read-only bit, or indeterminate on a folder.
    pub read_only: CheckState,
    /// The Executable box as it opened (unticked on a folder).
    pub executable: bool,
    pub hidden: bool,
    pub listed: bool,
    pub is_dir: bool,
    pub opens_icon: Option<String>,
    /// Advanced Attributes' archive and index boxes as read from the
    /// file, or None where its file system keeps no such flags.
    pub robots: Option<[CheckState; 2]>,
    /// Where Advanced Attributes' OK left those boxes, until the sheet
    /// writes them.
    pub adv: Option<[CheckState; 2]>,
    pub summary: Option<[String; 6]>,
    /// A folder's or a launcher's own icon as it opened (None: the
    /// standard folder, or no icon of the launcher's own), and the one
    /// Change Icon left until the sheet writes it.
    pub own_icon: Option<String>,
    pub icon_chosen: Option<Option<String>>,
    /// A shortcut's keys as the file had them (a program launcher's, a
    /// symlink's, a web link's), None for anything else.
    pub launcher: Option<reprisefs::launcher::Launcher>,
    /// A shortcut (a launcher, a web link, a symlink): the header icon
    /// wears the shortcut arrow.
    pub shortcut: bool,
}

/// What an OK or Apply wrote to one item: where it is now, and each
/// field that landed as it was written. A field whose write failed is
/// left out, so the sheet keeps it as a change.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Written {
    pub path: std::path::PathBuf,
    pub attrs: reprisefs::props::AttrMoves,
    pub hidden: Option<bool>,
    pub summary: Vec<(usize, String)>,
    pub folder_icon: Option<Option<String>>,
    pub launcher: Vec<(reprisefs::launcher::Key, String)>,
    /// The shortcut read back after its keys were written, for [`Dialog::sheet_reread`].
    pub reread: Option<reprisefs::props::FileInfo>,
}

/// A set's Properties sheet: what its items share, and where Advanced
/// Attributes' OK left the archive and index boxes until the sheet
/// writes them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetBase {
    pub set: reprisefs::props::SetInfo,
    pub adv: Option<[CheckState; 2]>,
    pub icon_chosen: Option<Option<String>>,
}

impl SetBase {
    pub fn robots(&self) -> Option<[CheckState; 2]> {
        reprisekit::controls::filesheet::robots_states(self.set.robots)
    }
}

impl std::ops::Deref for SetBase {
    type Target = reprisefs::props::SetInfo;
    fn deref(&self) -> &reprisefs::props::SetInfo {
        &self.set
    }
}

impl std::ops::DerefMut for SetBase {
    fn deref_mut(&mut self) -> &mut reprisefs::props::SetInfo {
        &mut self.set
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Dialog {
    pub title: String,
    pub style: Style,
    pub placement: Placement,
    /// Icon theme name, drawn at 32 px left of the text or at a form's "icon" picture.
    pub icon: Option<String>,
    pub text: String,
    pub field: Option<Field>,
    pub buttons: Vec<DialogButton>,
    /// ? button in the caption (Run).
    pub help: bool,
    /// Classic button the pointer rests on or holds; a held one draws pushed while the pointer is
    /// on it and acts on release there.
    pub pointer: Pointer<usize>,
    /// The caption button a styled look lights under the pointer.
    pub caption_hot: Option<CaptionButton>,
    /// The caption button (close, or a windowed dialog's minimize) held by the pointer; it draws
    /// pushed while the pointer is on it and acts on release there.
    pub caption_box: Pointer<CaptionButton>,
    /// Pointer on the ? box; it does nothing when clicked.
    pub help_box: Pointer<()>,
    /// Part of the text field the pointer rests on; the drop button lights only from itself.
    pub field_lit: Pointer<FieldPart>,
    /// In a dialog with a field, the button Tab moved focus to (`None`: the field). It wears the
    /// default ring and dotted rectangle; Enter and Space press it.
    pub button_focus: Option<usize>,
}

/// The tray dialog's part under the pointer, when it is on it.
pub(crate) fn tray_part(over: Option<DialogHit>) -> Option<traycustomize::Hit> {
    match over {
        Some(DialogHit::Tray(h)) => Some(h),
        _ => None,
    }
}

/// The file picker's part under the pointer, when it is on the picker.
fn browse_part(over: Option<DialogHit>) -> Option<reprisekit::controls::filedlg::Hit> {
    match over {
        Some(DialogHit::Browse(h)) => Some(h),
        _ => None,
    }
}

fn field_part(hit: Option<DialogHit>) -> Option<FieldPart> {
    match hit {
        Some(DialogHit::Field(_)) => Some(FieldPart::Edit),
        Some(DialogHit::Drop) => Some(FieldPart::Drop),
        _ => None,
    }
}

/// A part of a dialog's text field that the pointer lights.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldPart {
    Edit,
    Drop,
}

impl Dialog {
    pub fn new(title: impl Into<String>, style: Style, placement: Placement) -> Dialog {
        Dialog {
            title: title.into(),
            style,
            placement,
            icon: None,
            text: String::new(),
            field: None,
            buttons: Vec::new(),
            help: false,
            pointer: Pointer::new(),
            caption_hot: None,
            caption_box: Pointer::new(),
            help_box: Pointer::new(),
            field_lit: Pointer::new(),
            button_focus: None,
        }
    }

    /// [`Dialog::fit`] on an output whose usable area is `area` big: the tray's Customize page
    /// lays itself out in the client a frame that size leaves.
    pub fn fit_in(&mut self, theme: &Theme, text: &TextRenderer, area: (i32, i32)) {
        if let Style::TrayCustomize(t) = &mut self.style {
            let pad = DialogLayout::compute_client((0, 0))
                .framed(theme, self.help)
                .size;
            t.room = Some((area.0 - pad.0, area.1 - pad.1));
        }
        self.fit(theme, text);
    }

    /// Fit a form dialog to `theme`: a form laid out around measured text is built again, and
    /// wrapped tabs and a larger frame grow it.
    pub fn fit(&mut self, theme: &Theme, text: &TextRenderer) {
        match &mut self.style {
            Style::Form(f) => {
                f.remeasure(text, theme.fonts.size_px, theme.text);
                let p = theme.paint();
                f.body.pages().iter().for_each(|page| page.drawn_with(&p));
                f.fit_tabs(text, theme.fonts.size_px);
                f.fit_frame(client_shift(theme));
            }
            Style::TrayCustomize(t) => {
                t.metrics = theme.controls();
                t.set_wording(traycustomize::Wording::of(theme));
                t.fit(text, theme.fonts.size_px);
                self.title = t.wording.title().into();
            }
            Style::Browse(p) => p.dlg.set_text(theme.text),
            Style::Copy(c) => {
                c.cp.follow(&theme.paint());
                c.units = theme.text;
                self.title = c.cp.title().into();
            }
            _ => {}
        }
    }

    /// What makes this dialog "the same window" as one already open: its
    /// title, plus the file it is about, so two same-named files get a
    /// Properties sheet each while Run or Recycle Bin Properties stay single.
    pub fn identity(&self) -> (&str, Option<&std::path::Path>) {
        let path = match &self.style {
            Style::Chooser(c) => Some(c.path.as_path()),
            Style::Form(f) => match &f.kind {
                FormKind::Attributes(a) => a.path(),
                FormKind::Properties(b) => Some(b.path.as_path()),
                FormKind::SetProperties(s) => s.paths.first().map(|p| p.as_path()),
                FormKind::Replace(b) => Some(b.identity()),
                _ => None,
            },
            Style::Copy(c) => Some(c.dir.as_path()),
            _ => None,
        };
        (self.title.as_str(), path)
    }

    /// The pid End Program's End Now would kill; None for any other dialog.
    pub fn end_program_pid(&self) -> Option<i32> {
        if !matches!(self.style, Style::EndProgram) {
            return None;
        }
        self.buttons.iter().find_map(|b| match b.action {
            DialogAction::EndProgram(pid) => Some(pid),
            _ => None,
        })
    }

    /// Whether the dialog has a taskbar button: End Program, which stands for an unresponsive
    /// program, and every [`Dialog::windowed`] one.
    pub fn on_taskbar(&self) -> bool {
        matches!(self.style, Style::EndProgram) || self.windowed()
    }

    /// Whether the dialog stands among the windows rather than over them: it goes behind the
    /// window clicked, and its task button minimises and restores it. The copy progress dialog.
    pub fn windowed(&self) -> bool {
        matches!(self.style, Style::Copy(_))
    }

    /// The icon theme name its task button shows; without one it draws the fallback glyph.
    pub fn task_icon(&self) -> Option<&str> {
        matches!(self.style, Style::Copy(_)).then_some("system-file-manager")
    }

    /// The job's progress its task button fills with.
    pub fn task_progress(&self) -> Option<reprisekit::taskbar::TaskProgress> {
        self.copy().map(|c| reprisekit::taskbar::TaskProgress {
            fraction: c.cp.fraction(),
            paused: c.cp.status.paused,
        })
    }

    pub fn chooser(&self) -> Option<&Chooser> {
        match &self.style {
            Style::Chooser(c) => Some(&**c),
            _ => None,
        }
    }

    pub(crate) fn chooser_mut(&mut self) -> Option<&mut Chooser> {
        match &mut self.style {
            Style::Chooser(c) => Some(&mut **c),
            _ => None,
        }
    }

    pub fn copy(&self) -> Option<&CopyDialog> {
        match &self.style {
            Style::Copy(c) => Some(c),
            _ => None,
        }
    }

    pub(crate) fn copy_mut(&mut self) -> Option<&mut CopyDialog> {
        match &mut self.style {
            Style::Copy(c) => Some(c),
            _ => None,
        }
    }

    pub fn tray(&self) -> Option<&TrayCustomize> {
        match &self.style {
            Style::TrayCustomize(t) => Some(t),
            _ => None,
        }
    }

    pub(crate) fn tray_mut(&mut self) -> Option<&mut TrayCustomize> {
        match &mut self.style {
            Style::TrayCustomize(t) => Some(t),
            _ => None,
        }
    }

    pub fn picker(&self) -> Option<&crate::filepick::FilePicker> {
        match &self.style {
            Style::Browse(p) => Some(p),
            _ => None,
        }
    }

    pub fn picker_mut(&mut self) -> Option<&mut crate::filepick::FilePicker> {
        match &mut self.style {
            Style::Browse(p) => Some(p),
            _ => None,
        }
    }

    /// The dialog font the dialog's own text and buttons are set in: End
    /// Program takes the second one. A form's pages pick their own.
    pub fn font(&self) -> DialogFont {
        match self.style {
            Style::EndProgram => DialogFont::ShellDlg,
            _ => DialogFont::Ui,
        }
    }

    pub fn form(&self) -> Option<&FormDialog> {
        match &self.style {
            Style::Form(f) => Some(f),
            _ => None,
        }
    }

    pub fn form_mut(&mut self) -> Option<&mut FormDialog> {
        match &mut self.style {
            Style::Form(f) => Some(f),
            _ => None,
        }
    }

    fn props_mut(&mut self) -> Option<(&mut SheetBase, &mut reprisekit::controls::form::Form)> {
        let Style::Form(f) = &mut self.style else {
            return None;
        };
        let FormKind::Properties(base) = &mut f.kind else {
            return None;
        };
        Some((base, f.body.pages_mut().get_mut(0)?))
    }

    /// After an OK or Apply wrote `w`: the sheet's base takes each field
    /// as it was written, so Apply greys for those and stays lit for an
    /// edit made while the write ran or a field whose write failed.
    pub fn sheet_applied(&mut self, w: &Written) {
        let state = |on: bool| if on { CheckState::On } else { CheckState::Off };
        let Some((base, _)) = self.props_mut() else {
            return;
        };
        base.path = w.path.clone();
        if let Some(on) = w.attrs.read_only {
            base.read_only = state(on);
        }
        if let Some(on) = w.attrs.executable {
            base.executable = on;
        }
        // A cleared Hidden took the name off the folder's .hidden.
        if let Some(h) = w.hidden {
            base.hidden = h;
            base.listed &= h;
        }
        if let Some(robots) = base.robots.as_mut() {
            for (k, moved) in [w.attrs.backup, w.attrs.index].into_iter().enumerate() {
                if let Some(on) = moved {
                    robots[k] = state(on);
                }
            }
        }
        if let Some(icon) = &w.folder_icon {
            base.own_icon = icon.clone();
        }
        if let Some(fields) = base.summary.as_mut() {
            for (k, v) in &w.summary {
                if let Some(f) = fields.get_mut(*k) {
                    *f = v.clone();
                }
            }
        }
        for (key, v) in &w.launcher {
            if let Some(l) = base.launcher.as_mut() {
                l.set(*key, v);
            }
            if *key == reprisefs::launcher::Key::Icon {
                base.own_icon = Some(v.clone());
            }
        }
        if let Some(f) = self.form_mut() {
            sync_apply(f);
        }
    }

    /// After an OK or Apply that rewrote a shortcut's keys: rebuild the sheet over `info` with
    /// `icon` in the header, keeping caption, place and tab. The host fits it to the theme again.
    pub fn sheet_reread(&mut self, info: &reprisefs::props::FileInfo, icon: &str) {
        let Some(open) = self
            .form()
            .and_then(|f| f.sheet())
            .and_then(|s| s.tabs.labels.get(s.tabs.open).cloned())
        else {
            return;
        };
        let fresh = Dialog::properties(info, icon, (0, 0));
        self.style = fresh.style;
        self.icon = fresh.icon;
        if let Some(sheet) = self.form_mut().and_then(|f| f.sheet_mut())
            && let Some(i) = sheet.tabs.labels.iter().position(|l| *l == open)
        {
            sheet.open_tab(i);
        }
    }

    /// After Apply wrote a set's changes: its items are at `paths` now
    /// and the boxes that settled are where they start, so Apply greys
    /// again.
    pub fn set_applied(&mut self, paths: Vec<std::path::PathBuf>) {
        use reprisekit::controls::filesheet;
        let Some(f) = self.form_mut() else {
            return;
        };
        let (FormKind::SetProperties(set), Some(form)) = (&mut f.kind, f.body.pages().first())
        else {
            return;
        };
        set.paths = paths;
        let settled = |s: CheckState| (s != CheckState::Mixed).then_some(s.is_on());
        if let Some(on) = settled(filesheet::read_only(form)) {
            set.read_only = Some(on);
        }
        if let Some(on) = settled(filesheet::hidden_state(form)) {
            set.hidden = Some(on);
        }
        set.icon_chosen = None;
        if let (Some(adv), Some(read)) = (set.adv.take(), set.set.robots.as_mut()) {
            for (k, flag) in read.iter_mut().enumerate() {
                if let Some(on) = settled(adv[k]) {
                    *flag = Some(on);
                }
            }
        }
        sync_apply(f);
    }

    /// Change Icon's OK over this sheet: the icon waits here until the
    /// sheet's OK or Apply writes it, and Customize shows it meanwhile.
    /// False when this is no Properties sheet.
    pub fn icon_chosen(&mut self, icon: Option<String>) -> bool {
        let launcher = matches!(
            self.form().map(|f| &f.kind),
            Some(FormKind::Properties(b)) if b.launcher.is_some()
        );
        // A launcher's Shortcut page wears the pick in its header.
        if launcher && let Some(name) = &icon {
            self.icon = Some(name.clone());
        }
        let Some(f) = self.form_mut() else {
            return false;
        };
        match &mut f.kind {
            FormKind::Properties(base) => base.icon_chosen = Some(icon),
            FormKind::SetProperties(set) => set.icon_chosen = Some(icon),
            _ => return false,
        }
        sync_apply(f);
        true
    }

    /// The Change Icon box for folder `name`, or a launcher's with `launcher`; Restore Defaults on
    /// a folder's. OK hands the pick to the sheet.
    pub fn change_icon(
        name: &str,
        source: &str,
        icons: reprisekit::controls::iconlist::IconList,
        launcher: bool,
    ) -> Dialog {
        use reprisekit::controls::filesheet;
        let form = filesheet::change_icon_form(source, icons, !launcher);
        let title = if launcher {
            "Change Icon".to_string()
        } else {
            format!("Change Icon for {name} Folder")
        };
        Dialog {
            help: true,
            ..Dialog::new(
                title,
                Style::Form(Box::new(FormDialog::from_client(
                    filesheet::CHANGE_ICON_SIZE,
                    form,
                    FormKind::ChangeIcon,
                ))),
                Placement::Center,
            )
        }
    }

    /// Advanced Attributes' OK over this sheet: the archive and index
    /// boxes wait here until the sheet's OK or Apply writes them, and
    /// Apply lights while they differ from the file. False when this is
    /// no Properties sheet.
    pub fn advanced_chosen(&mut self, chosen: [CheckState; 2]) -> bool {
        let Some(f) = self.form_mut() else {
            return false;
        };
        match &mut f.kind {
            FormKind::Properties(base) => base.adv = Some(chosen),
            FormKind::SetProperties(set) => set.adv = Some(chosen),
            _ => return false,
        }
        sync_apply(f);
        true
    }

    pub fn set_paths(&self) -> Option<&[std::path::PathBuf]> {
        match &self.form()?.kind {
            FormKind::SetProperties(s) => Some(&s.paths),
            _ => None,
        }
    }
}

/// What the view does after a key or click.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Nothing,
    Redraw,
    /// The copy progress dialog's Pause (true) or Resume (false).
    CopyPause(bool),
    /// The copy progress dialog's Cancel, Escape or close box.
    CopyCancel,
    /// Redraw after recomputing the layout: the open tab changed, and a
    /// tab's rect depends on which one is open.
    Relayout,
    /// A press on Browse's size grip: the pointer sizes the dialog from
    /// these edges until the release.
    Size(Edges),
    Close,
    Spawn(String),
    EndSession(Goal, String),
    /// The session-end prompt's answer: End Now or Cancel.
    SessionEnd(reprisekit::controls::endsession::Choice),
    /// The location prompt's answer for the program `desktop_id`.
    Location {
        desktop_id: String,
        answer: reprisekit::controls::locationprompt::Answer,
    },
    Delete {
        paths: Vec<std::path::PathBuf>,
        erase: bool,
    },
    EmptyBin,
    Notice(Box<Dialog>),
    /// Run what the user typed in the field: spawned like `Spawn`, and
    /// remembered in the Run history.
    Run(String),
    Act(Action),
    /// A Properties sheet's OK or Apply: rename `path` to `name` when the
    /// name or the Hidden box changed, write the attributes whose boxes
    /// moved (`attrs`), and `hidden` when Hidden moved (a folder's
    /// `.hidden` line goes when it is cleared). `close` closes the sheet
    /// after (OK); Apply keeps it open. `recurse` is the Confirm Attribute
    /// Changes answer for a folder: None until it has been asked, then
    /// whether the attributes and Hidden reach the subfolders and files.
    Properties {
        path: std::path::PathBuf,
        name: Option<String>,
        attrs: reprisefs::props::AttrMoves,
        hidden: Option<bool>,
        /// The Summary fields that moved, by index into
        /// `filesheet::SUMMARY_IDS`, with what they hold now.
        summary: Vec<(usize, String)>,
        /// A folder's icon when Change Icon moved it: its own, or the
        /// standard folder's for None.
        folder_icon: Option<Option<String>>,
        /// A program launcher's keys that moved: its Shortcut fields and
        /// the icon Change Icon left.
        launcher: Vec<(reprisefs::launcher::Key, String)>,
        close: bool,
        recurse: Option<bool>,
    },
    /// Open `path` with the program whose desktop-file id is `app`;
    /// `always` makes it the default for `mime`.
    OpenWith {
        path: std::path::PathBuf,
        app: String,
        always: bool,
        mime: Option<String>,
    },
    /// A set's Properties sheet's OK or Apply: write the attributes that
    /// moved on every item in `paths`, and hide or show each when Hidden
    /// did. `close` and `recurse` are as for `Properties`; only a set
    /// with a folder is asked how far they reach.
    SetProperties {
        paths: Vec<std::path::PathBuf>,
        attrs: reprisefs::props::AttrMoves,
        hidden: Option<bool>,
        folder_icon: Option<Option<String>>,
        close: bool,
        recurse: Option<bool>,
    },
    /// Change Icon... on Customize or Shortcut: the Change Icon box for
    /// the folder or the launcher (`launcher`) called `name`, on
    /// `current` (None: the standard folder, or no icon of its own).
    ChangeIcon {
        name: String,
        current: Option<String>,
        launcher: bool,
    },
    /// Shortcut's Find Target...: open the folder this program sits in
    /// with it selected, the sheet staying up.
    FindTarget(std::path::PathBuf),
    IconChosen(Option<String>),
    ChangeDefault(std::path::PathBuf),
    /// The Properties sheet's Advanced...: Advanced Attributes for what
    /// the sheet is about, its archive and index boxes at `robots` (None:
    /// greyed, the file system keeps no such flags).
    AdvancedAttributes {
        subject: reprisekit::controls::filesheet::Subject,
        robots: Option<[CheckState; 2]>,
    },
    AdvancedChosen([CheckState; 2]),
    /// OK on the chooser Change... opened: make `app` the default for
    /// `mime` without running the file, and tell the sheet behind it.
    SetDefault {
        path: std::path::PathBuf,
        app: String,
        mime: Option<String>,
    },
    BrowseRun,
    /// Browse's Open for Run: close it and put `path`, quoted, in the
    /// Run field under it.
    RunPick(std::path::PathBuf),
    BrowseProgram,
    /// Browse's Open for the chooser: close it and add `path` to the
    /// chooser's list under it.
    ProgramPick(std::path::PathBuf),
    BinSettings {
        settings: reprisefs::bincap::Policy,
        close: bool,
    },
    /// Close the question about an item too big for the Recycle Bin and
    /// carry on with its batch.
    TooBig {
        asker: Box<reprisefs::bincap::TooBig>,
        answer: reprisefs::replace::Answer,
    },
    HideSystemIcon(String),
    DeleteIcons {
        hide: Vec<String>,
        files: Vec<std::path::PathBuf>,
    },
    EndProgram(i32),
    TurnOn(crate::keyfilter::Aid),
    TrayChoices(traycustomize::Choices),
    Replace {
        batch: Box<crate::dragview::Batch>,
        answer: reprisefs::replace::Answer,
    },
}

fn button(label: &str, action: DialogAction, default: bool) -> DialogButton {
    DialogButton {
        label: label.into(),
        action,
        default,
        icon: None,
        enabled: true,
        tip: None,
    }
}

fn icon_button(
    label: &str,
    icon: &str,
    tip: &str,
    action: DialogAction,
    default: bool,
    enabled: bool,
) -> DialogButton {
    DialogButton {
        label: label.into(),
        action,
        default,
        icon: Some(icon.into()),
        enabled,
        tip: Some(tip.into()),
    }
}

impl Dialog {
    /// Browse for `picker`'s purpose: the file dialog in a sizable frame opening at
    /// `filedlg::CLIENT`; the host places it (`Center` is the fallback).
    pub fn browse(picker: crate::filepick::FilePicker) -> Dialog {
        Dialog {
            help: true,
            ..Dialog::new("Browse", Style::Browse(Box::new(picker)), Placement::Center)
        }
    }

    /// Advanced Attributes: `filesheet::advanced_form` in a dialog frame, boxes at `robots` (None:
    /// greyed, OK closes like Cancel).
    pub fn advanced_attributes(
        subject: reprisekit::controls::filesheet::Subject,
        robots: Option<[CheckState; 2]>,
        text: &TextRenderer,
        px: u32,
        u: TextUnits,
    ) -> Dialog {
        Dialog {
            icon: Some("document-properties".into()),
            help: true,
            ..Dialog::new(
                "Advanced Attributes",
                Style::Form(Box::new(FormDialog::measured(
                    Measured::Advanced { subject, robots },
                    FormKind::AdvancedAttributes(robots.is_some()),
                    text,
                    px,
                    u,
                ))),
                Placement::Center,
            )
        }
    }

    /// Confirm Attribute Changes: holds a folder's or set's write until OK says how far down the
    /// tree it reaches; opens on "subfolders and files".
    pub fn confirm_attributes(
        a: AttrConfirm,
        text: &TextRenderer,
        px: u32,
        u: TextUnits,
    ) -> Dialog {
        use reprisekit::controls::filesheet::Subject;
        let verb = a.attrs.verbs(a.hidden());
        let subject = match a.write {
            AttrWrite::One { .. } => Subject::Folder,
            AttrWrite::Set { .. } => Subject::Set { has_dir: true },
        };
        Dialog::new(
            "Confirm Attribute Changes",
            Style::Form(Box::new(FormDialog::measured(
                Measured::Attributes { verb, subject },
                FormKind::Attributes(a),
                text,
                px,
                u,
            ))),
            Placement::Center,
        )
    }

    /// Confirm File/Folder Replace over `batch`; buttons answer for the item the batch asks about.
    pub fn confirm_replace(
        fields: reprisekit::controls::replacebox::Fields,
        batch: crate::dragview::Batch,
        text: &TextRenderer,
        px: u32,
        u: TextUnits,
    ) -> Dialog {
        Dialog::new(
            fields.title(),
            Style::Form(Box::new(FormDialog::measured(
                Measured::Replace(fields),
                FormKind::Replace(Box::new(batch)),
                text,
                px,
                u,
            ))),
            Placement::Center,
        )
    }

    /// The session-end prompt: programs Log Off, Turn Off or Restart waits for, with End Now and
    /// Cancel.
    pub fn end_session(
        fields: &reprisekit::controls::endsession::Fields,
        text: &TextRenderer,
        px: u32,
        u: TextUnits,
    ) -> Dialog {
        Dialog::new(
            fields.title(),
            Style::Form(Box::new(FormDialog::measured(
                Measured::EndSession(fields.clone()),
                FormKind::EndSession,
                text,
                px,
                u,
            ))),
            Placement::Center,
        )
    }

    /// The location prompt: may program `desktop_id` have the location.
    pub fn location_prompt(
        fields: &reprisekit::controls::locationprompt::Fields,
        desktop_id: String,
        text: &TextRenderer,
        px: u32,
        u: TextUnits,
    ) -> Dialog {
        use reprisekit::controls::locationprompt;
        Dialog::new(
            locationprompt::TITLE,
            Style::Form(Box::new(FormDialog::measured(
                Measured::Location(fields.clone()),
                FormKind::Location(desktop_id),
                text,
                px,
                u,
            ))),
            Placement::Center,
        )
    }

    pub fn end_session_form(&mut self) -> Option<&mut reprisekit::controls::form::Form> {
        match &mut self.style {
            Style::Form(f) if f.kind == FormKind::EndSession => f.open_page_mut(),
            _ => None,
        }
    }

    /// The Run dialog: a command line, OK / Cancel / Browse. It opens on
    /// the last command in `history` (newest first), selected.
    pub fn run(history: &[String]) -> Dialog {
        Dialog {
            icon: Some("system-run".into()),
            text: "Type the name of a program, folder, document, or Internet resource, and \
                   Reprise will open it for you."
                .into(),
            field: Some(Field {
                label: "&Open:".into(),
                combo: {
                    let items: Vec<&str> = history.iter().map(String::as_str).collect();
                    let mut c = Combo::new(&items, 0, LIST_ROWS);
                    c.edit = Some(TextEdit::new(
                        history.first().cloned().unwrap_or_default(),
                        !history.is_empty(),
                    ));
                    c
                },
                thumb: ThumbDrag::default(),
            }),
            buttons: vec![
                button("OK", DialogAction::RunField, true),
                button("Cancel", DialogAction::Cancel, false),
                button("&Browse...", DialogAction::BrowseRun, false),
            ],
            help: true,
            ..Dialog::new(
                "Run",
                Style::Classic {
                    template: Template::Run,
                    text_h: None,
                    delete_badge: false,
                },
                Placement::ByStart,
            )
        }
    }

    /// The Open With chooser for `path`: programs registered for its type under "Recommended
    /// Programs:", the rest under "Other Programs:", `pick` (the type's default) picked, else the
    /// first. `set_default` (the sheet's Change...) ticks and greys the box; OK then only writes
    /// the default.
    pub fn open_with(
        path: &std::path::Path,
        mime: Option<String>,
        recommended: Vec<openwith::Program>,
        others: Vec<openwith::Program>,
        pick: Option<&str>,
        set_default: bool,
    ) -> Dialog {
        let file = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut ow = openwith::OpenWith::new(&file, recommended, others);
        if let Some(id) = pick {
            ow.select_id(id);
        }
        if set_default {
            ow.always = openwith::Check {
                on: true,
                enabled: false,
            };
        }
        Dialog {
            icon: Some(openwith::ICON.into()),
            help: true,
            ..Dialog::new(
                openwith::TITLE,
                Style::Chooser(Box::new(Chooser {
                    ow,
                    set_default,
                    path: path.to_path_buf(),
                    mime,
                })),
                Placement::Center,
            )
        }
    }

    pub fn copy_progress(cp: CopyProgress, dir: std::path::PathBuf) -> Dialog {
        let title = cp.title();
        let c = CopyDialog {
            cp,
            dir,
            units: Default::default(),
        };
        Dialog::new(title, Style::Copy(Box::new(c)), Placement::Center)
    }

    fn copy_outcome(o: copyprogress::Outcome) -> Outcome {
        match o {
            copyprogress::Outcome::None => Outcome::Nothing,
            copyprogress::Outcome::Redraw => Outcome::Redraw,
            copyprogress::Outcome::Resized => Outcome::Relayout,
            copyprogress::Outcome::Pause(on) => Outcome::CopyPause(on),
            copyprogress::Outcome::Cancel => Outcome::CopyCancel,
        }
    }

    pub fn tray_customize(t: TrayCustomize) -> Dialog {
        Dialog {
            help: true,
            ..Dialog::new(
                t.wording.title(),
                Style::TrayCustomize(Box::new(t)),
                Placement::Center,
            )
        }
    }

    fn tray_outcome(ev: traycustomize::Event) -> Outcome {
        match ev {
            traycustomize::Event::None => Outcome::Nothing,
            traycustomize::Event::Redraw => Outcome::Redraw,
            traycustomize::Event::Cancel => Outcome::Close,
            traycustomize::Event::Accept(c) => Outcome::TrayChoices(c),
        }
    }

    /// Browse...'s pick: the program at `path` joins the list under
    /// "Other Programs:" by its file name and is selected, so OK opens the
    /// file with it; one already listed is just selected. Its `app` is the
    /// path itself, which the host runs as a command when no desktop entry
    /// has that id. A path that is not UTF-8 is refused with a notice.
    pub fn add_program(&mut self, path: &std::path::Path) -> Outcome {
        let Some(c) = self.chooser_mut() else {
            return Outcome::Nothing;
        };
        let Some(app) = path.to_str().map(str::to_string) else {
            return Outcome::Notice(Box::new(Dialog::not_utf8_program(openwith::TITLE, path)));
        };
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| app.clone());
        c.ow.add_program(openwith::Program {
            id: app,
            name,
            icon: Some(openwith::PROGRAM_ICON.into()),
        });
        Outcome::Redraw
    }

    fn chooser_outcome(&self, ev: openwith::Event) -> Outcome {
        let Some(c) = self.chooser() else {
            return Outcome::Nothing;
        };
        match ev {
            openwith::Event::None => Outcome::Nothing,
            openwith::Event::Redraw => Outcome::Redraw,
            openwith::Event::Cancel => Outcome::Close,
            openwith::Event::Browse => Outcome::BrowseProgram,
            openwith::Event::Choose { id, .. } if c.set_default => Outcome::SetDefault {
                path: c.path.clone(),
                app: id,
                mime: c.mime.clone(),
            },
            openwith::Event::Choose { id, always } => Outcome::OpenWith {
                path: c.path.clone(),
                app: id,
                always,
                mime: c.mime.clone(),
            },
        }
    }

    /// The "<name> Properties" sheet for a desktop icon: the `filesheet` General page. A folder not
    /// walked yet reads "Calculating..." until `set_folder_totals`.
    pub fn properties(info: &reprisefs::props::FileInfo, icon: &str, at: (i32, i32)) -> Dialog {
        use reprisefs::props::{format_contains, format_size, format_time};
        use reprisekit::controls::filesheet::{self, CALCULATING, Facts};
        let is_12h = reprisefs::locale::is_12h();
        let walking = info.is_dir && info.contains.is_none();
        let total = |v: String| if walking { CALCULATING.to_string() } else { v };
        let facts = Facts {
            is_dir: info.is_dir,
            type_name: info.type_name.clone(),
            opens_with: info.opens_with.clone(),
            description: info.description.clone(),
            location: info.location.clone(),
            size: total(format_size(info.size)),
            on_disk: total(format_size(info.on_disk)),
            contains: match info.contains {
                Some((files, folders)) => Some(format_contains(files, folders)),
                None => walking.then(|| CALCULATING.to_string()),
            },
            created: info.created.map(|t| format_time(t, is_12h)),
            modified: format_time(info.modified, is_12h),
            accessed: format_time(info.accessed, is_12h),
            read_only: info.read_only,
            executable: info.executable,
            can_set_executable: info.can_set_executable,
            hidden: info.hidden,
        };
        // The field shows the file name less the extension the icon hides
        // and the dot the Hidden box stands for; a place or launcher keeps
        // its real name so OK cannot rename it by accident.
        let shown = filesheet::shown_name(&info.file_name(), &info.hidden_ext, info.hidden);
        let (page, height) = filesheet::general_page(&filesheet::rows(&facts), &shown, None, None);
        let over = sheet_over(info);
        let sheet = filesheet::sheet(page, height, &filesheet::sheet_tabs(over), |tab| {
            match (tab, &info.launcher) {
                (filesheet::Tab::Shortcut, Some(l)) => {
                    filesheet::shortcut_page(&shortcut_facts(&info.name, l), None)
                }
                (filesheet::Tab::WebDocument, Some(l)) => filesheet::web_page(
                    &filesheet::WebFacts {
                        name: info.name.clone(),
                        url: l.target.clone(),
                    },
                    None,
                ),
                _ => filesheet::summary_page(info.summary.as_ref()),
            }
        });
        let base = SheetBase {
            path: info.path.clone(),
            name: info.name.clone(),
            ext: info.hidden_ext.clone(),
            // By the box's id: a launcher's sheet opens on Shortcut.
            read_only: sheet
                .pages
                .iter()
                .find_map(|p| p.check_state(filesheet::READ_ONLY))
                .unwrap_or_default(),
            executable: sheet
                .pages
                .iter()
                .find_map(|p| p.checked(filesheet::EXECUTABLE))
                .unwrap_or_default(),
            hidden: info.hidden,
            listed: info.listed,
            is_dir: info.is_dir,
            opens_icon: info.opens_with_icon.clone(),
            robots: info
                .robots
                .map(|r| r.map(|on| filesheet::shared_state(Some(on)))),
            adv: None,
            summary: info.summary.clone(),
            own_icon: match &info.launcher {
                Some(l) => l.icon.clone(),
                None => info.own_icon.clone(),
            },
            icon_chosen: None,
            launcher: info.launcher.clone(),
            shortcut: info.is_shortcut,
        };
        Dialog {
            icon: Some(icon.into()),
            help: true,
            ..Dialog::new(
                format!("{} Properties", info.name),
                Style::Form(Box::new(FormDialog::from_sheet(
                    sheet,
                    FormKind::Properties(Box::new(base)),
                ))),
                Placement::At(at.0, at.1),
            )
        }
    }

    /// The combined Properties sheet of a multi-selection, titled after `name`: the `filesheet` set
    /// page. Folders not walked yet read "Calculating..." until `set_folder_totals`.
    pub fn set_properties(set: &reprisefs::props::SetInfo, name: &str, at: (i32, i32)) -> Dialog {
        use reprisefs::props::format_size;
        use reprisekit::controls::filesheet::{self, CALCULATING, SetFacts};
        let total = |v: String| {
            if set.walking {
                CALCULATING.to_string()
            } else {
                v
            }
        };
        let facts = SetFacts {
            has_dir: set.has_dir,
            count: total(set.count_text()),
            type_text: set.type_text(),
            location: set.location_text(),
            size: total(format_size(set.size)),
            on_disk: total(format_size(set.on_disk)),
            read_only: filesheet::shared_state(set.read_only),
            hidden: filesheet::shared_state(set.hidden),
        };
        let (page, height) = filesheet::general_page(&filesheet::set_rows(&facts), "", None, None);
        let over = filesheet::Over::Set {
            document: set.has_document,
            shortcut: set.has_shortcut,
            folder: set.has_dir,
        };
        // A set's Summary is greyed: its items share no one description.
        let sheet = filesheet::sheet(page, height, &filesheet::sheet_tabs(over), |_| {
            filesheet::summary_page(None)
        });
        Dialog {
            icon: Some(filesheet::SET_ICON.into()),
            help: true,
            ..Dialog::new(
                format!("{name}, ... Properties"),
                Style::Form(Box::new(FormDialog::from_sheet(
                    sheet,
                    FormKind::SetProperties(SetBase {
                        set: set.clone(),
                        adv: None,
                        icon_chosen: None,
                    }),
                ))),
                Placement::At(at.0, at.1),
            )
        }
    }

    /// "Confirm File Delete" ("Folder" for a folder, whose template reserves four text lines): Yes
    /// trashes `path`, No/Escape leave it. With `erase` Yes removes it outright, with a folder icon
    /// under a red badge; `no_bin` says its drive has no Recycle Bin.
    pub fn confirm_delete(
        name: &str,
        path: &std::path::Path,
        is_dir: bool,
        erase: bool,
        no_bin: bool,
    ) -> Dialog {
        let yes = DialogAction::Delete {
            paths: vec![path.to_path_buf()],
            erase,
        };
        let kind = reprisefs::bincap::ConfirmKind::of(1, is_dir);
        let lack = match no_bin {
            true => reprisefs::bincap::BinLack::All,
            false => reprisefs::bincap::BinLack::None,
        };
        let (title, text) = reprisefs::bincap::delete_prompt(kind, name, 1, erase, lack);
        let icon = if erase { "folder" } else { "user-trash-full" };
        Dialog {
            icon: Some(icon.into()),
            text,
            buttons: vec![
                button("&Yes", yes, true),
                button("&No", DialogAction::Cancel, false),
            ],
            ..Dialog::new(
                title,
                Style::Classic {
                    template: Template::MessageBox,
                    text_h: is_dir.then_some(51),
                    delete_badge: erase,
                },
                Placement::Center,
            )
        }
    }

    /// "Confirm Multiple File Delete" for a selection of `paths`: "send
    /// these N items to the Recycle Bin", or "delete these N items" with
    /// `erase`, led by the missing Recycle Bin for `no_bin` of them. Yes trashes
    /// (or erases) every one of them.
    pub fn confirm_delete_many(
        paths: &[std::path::PathBuf],
        erase: bool,
        no_bin: reprisefs::bincap::BinLack,
    ) -> Dialog {
        use reprisefs::bincap::{ConfirmKind, delete_prompt};
        let count = paths.len();
        let yes = DialogAction::Delete {
            paths: paths.to_vec(),
            erase,
        };
        let (title, text) = delete_prompt(ConfirmKind::Multiple, "", count, erase, no_bin);
        let icon = if erase { "folder" } else { "user-trash-full" };
        Dialog {
            icon: Some(icon.into()),
            text,
            buttons: vec![
                button("&Yes", yes, true),
                button("&No", DialogAction::Cancel, false),
            ],
            ..Dialog::new(
                title,
                Style::Classic {
                    template: Template::MultiDelete,
                    text_h: None,
                    delete_badge: erase,
                },
                Placement::Center,
            )
        }
    }

    /// The question `asker` asks next, about `path` (a folder with
    /// `is_dir`): it is too big for its Recycle Bin, so delete it for
    /// good? Yes erases it, No leaves it; a batch of more than one item
    /// adds Yes to All and Cancel. Escape and the close box drop the
    /// batch, as Cancel does.
    pub fn too_big(
        path: &std::path::Path,
        is_dir: bool,
        asker: reprisefs::bincap::TooBig,
    ) -> Dialog {
        use reprisefs::replace::Answer;
        let asker = Box::new(asker);
        let answer = |label: &str, answer: Answer, default: bool| {
            let action = DialogAction::TooBig {
                asker: asker.clone(),
                answer,
            };
            button(label, action, default)
        };
        let mut buttons = vec![answer("&Yes", Answer::Yes, true)];
        if asker.many() {
            buttons.push(answer("Yes to &All", Answer::YesToAll, false));
        }
        buttons.push(answer("&No", Answer::No, false));
        if asker.many() {
            buttons.push(button("Cancel", DialogAction::Cancel, false));
        }
        let title = match is_dir {
            true => "Confirm Folder Delete",
            false => "Confirm File Delete",
        };
        Dialog {
            icon: Some(if is_dir { "folder" } else { "text-x-generic" }.into()),
            text: reprisefs::bincap::too_big_text(&reprisefs::bincap::label(path)),
            buttons,
            ..Dialog::new(
                title,
                Style::Classic {
                    template: Template::MessageBox,
                    text_h: None,
                    delete_badge: true,
                },
                Placement::Center,
            )
        }
    }

    /// "Confirm Delete" on a system icon: Yes only takes the icon off
    /// the desktop (the folder behind Documents or Home is left alone), No
    /// and Escape keep it. Listing it in `system_icons` again brings it
    /// back.
    pub fn confirm_hide(place: crate::desktop::Place, label: &str) -> Dialog {
        let restore = match place {
            crate::desktop::Place::Documents | crate::desktop::Place::Home => {
                "The contents of the folder will not be deleted. You can restore this icon \
                 to the desktop on the Background page of Reprise Settings."
            }
            _ => "To restore it later, use the Background page of Reprise Settings.",
        };
        Dialog {
            // Folder with a document under the `draw_delete_badge` badge; `folder-documents` is the
            // nearest themed icon.
            icon: Some("folder-documents".into()),
            text: format!(
                "Are you sure you want to delete the {label} icon from your desktop?\n\n{restore}"
            ),
            buttons: vec![
                button(
                    "&Yes",
                    DialogAction::HideSystemIcon(place.config_name().into()),
                    true,
                ),
                button("&No", DialogAction::Cancel, false),
            ],
            ..Dialog::new(
                "Confirm Delete",
                Style::Classic {
                    template: Template::ShellNotice,
                    text_h: None,
                    delete_badge: true,
                },
                Placement::Center,
            )
        }
    }

    /// "Confirm Delete" on a multi-selection holding system icons, `hide`
    /// by their `system_icons` names, and the files `files`: Yes takes
    /// the icons off the desktop, and either answer goes on to ask about
    /// the files as Delete on them would; Escape does neither. With no
    /// files, the second line only says where the icons come back.
    pub fn confirm_delete_icons(hide: Vec<String>, files: Vec<std::path::PathBuf>) -> Dialog {
        let restore = if files.is_empty() {
            "To restore them later, use the Background page of Reprise Settings."
        } else {
            "One or more of the items you are about to delete cannot be restored from the \
             Recycle Bin.\nTo restore them later, use the Background page of Reprise Settings."
        };
        let no = DialogAction::DeleteIcons {
            hide: Vec::new(),
            files: files.clone(),
        };
        Dialog {
            icon: Some("folder".into()),
            text: format!(
                "Are you sure you want to delete these icons from your desktop?\n\n{restore}"
            ),
            buttons: vec![
                button("&Yes", DialogAction::DeleteIcons { hide, files }, true),
                button("&No", no, false),
            ],
            ..Dialog::new(
                "Confirm Delete",
                Style::Classic {
                    template: Template::ShellNotice,
                    text_h: None,
                    delete_badge: true,
                },
                Placement::Center,
            )
        }
    }

    /// What a key aid's shortcut asks: turn the aid on (Yes), leave it
    /// off (No), or open its settings, where the shortcut itself can be
    /// turned off.
    pub fn key_aid_prompt(aid: crate::keyfilter::Aid) -> Dialog {
        use crate::keyfilter::Aid;
        let what = match aid {
            Aid::StickyKeys => {
                "Pressing the Shift key five times is the shortcut for StickyKeys. With \
                 StickyKeys, Shift, Ctrl, Alt and the Super key work one key at a time."
            }
            Aid::SlowKeys => {
                "Holding down the right Shift key for eight seconds is the shortcut for \
                 SlowKeys. With SlowKeys, a key counts only once it has been held down for a \
                 moment, so brief keystrokes are ignored."
            }
            Aid::MouseKeys => {
                "Pressing left Alt, left Shift and Num Lock together is the shortcut for \
                 MouseKeys. With MouseKeys, the keypad moves the pointer and clicks."
            }
        };
        let name = aid.name();
        Dialog {
            text: format!(
                "{what}\n\nTurn on {name}?\nTo leave it off, click No.\n\
                 To turn off this shortcut, click Settings..."
            ),
            buttons: vec![
                button("&Yes", DialogAction::TurnOn(aid), true),
                button("&No", DialogAction::Cancel, false),
                button(
                    "&Settings...",
                    DialogAction::Spawn(format!(
                        "reprise-settings --page accessibility --focus {}",
                        aid.key()
                    )),
                    false,
                ),
            ],
            ..Dialog::new(
                name,
                Style::Classic {
                    template: Template::KeyAid,
                    text_h: Some(KEY_AID_TEXT_H),
                    delete_badge: false,
                },
                Placement::CenterAs(180),
            )
        }
    }

    /// End Program, over a window whose client did not answer a ping
    /// after it was asked to close. `program` names the process in the
    /// caption; End Now kills `pid`, Cancel (the default) leaves it running.
    pub fn end_program(program: &str, pid: i32) -> Dialog {
        Dialog {
            icon: Some("application-x-executable".into()),
            text: "This program has stopped responding.\n\
                   To go back to the desktop and give the program time to recover, click \
                   Cancel.\n\
                   Ending the program right away loses anything it has not saved. To end \
                   the program now, click End Now."
                .into(),
            buttons: vec![
                button("End Now", DialogAction::EndProgram(pid), false),
                button("Cancel", DialogAction::Cancel, true),
            ],
            ..Dialog::new(
                format!("End Program - {program}"),
                Style::EndProgram,
                Placement::Center,
            )
        }
    }

    /// Message box with the error icon and OK.
    pub fn error(title: &str, text: String) -> Dialog {
        Dialog {
            icon: Some("dialog-error".into()),
            text,
            buttons: vec![button("OK", DialogAction::Cancel, true)],
            ..Dialog::new(
                title,
                Style::Classic {
                    template: Template::MessageBox,
                    text_h: None,
                    delete_badge: false,
                },
                Placement::Center,
            )
        }
    }

    /// The error for a picked program at `path` whose name is not
    /// UTF-8: a command line and a desktop entry's `Exec` hold text only,
    /// so it cannot be run from one.
    pub fn not_utf8_program(title: &str, path: &std::path::Path) -> Dialog {
        Dialog::error(
            title,
            format!(
                "Cannot use '{}': its name is not valid UTF-8.",
                path.display()
            ),
        )
    }

    /// The question before the Recycle Bin is emptied: "Confirm File
    /// Delete" naming the one item, or "Confirm Multiple File Delete" with
    /// the count. Yes empties it.
    pub fn confirm_empty_bin(count: usize, first: Option<&str>) -> Dialog {
        use reprisefs::bincap::{BinLack, ConfirmKind, delete_prompt};
        let (title, text) = match (count, first) {
            (1, Some(name)) => delete_prompt(ConfirmKind::File, name, 1, true, BinLack::None),
            _ => delete_prompt(ConfirmKind::Multiple, "", count, true, BinLack::None),
        };
        Dialog {
            icon: Some("user-trash-full".into()),
            text,
            buttons: vec![
                button("&Yes", DialogAction::EmptyBin, true),
                button("&No", DialogAction::Cancel, false),
            ],
            ..Dialog::new(
                title,
                Style::Classic {
                    template: Template::MessageBox,
                    text_h: None,
                    delete_badge: false,
                },
                Placement::Center,
            )
        }
    }

    /// The "Log Off Reprise" panel: Switch User (there is no one to
    /// switch to, so greyed out) and Log Off, which runs the `exit` action
    /// and holds the focus.
    pub fn log_off() -> Dialog {
        Dialog {
            buttons: vec![
                icon_button(
                    "&Switch User",
                    "system-switch-user",
                    "Lets another user log on while your programs and files remain open.",
                    DialogAction::Cancel,
                    false,
                    false,
                ),
                icon_button(
                    "&Log Off",
                    "gnome-session-logout",
                    "Closes your programs and ends your Reprise session.",
                    DialogAction::Act(Action::Exit),
                    true,
                    true,
                ),
                button("Cancel", DialogAction::Cancel, false),
            ],
            ..Dialog::new("Log Off Reprise", Style::Welcome, Placement::UpperThird)
        }
    }

    pub fn turn_off(power: &PowerConfig) -> Dialog {
        Dialog {
            buttons: vec![
                icon_button(
                    "&Stand By",
                    "system-suspend",
                    "Keeps your session in memory and puts the computer in a low-power \
                     state so that you can quickly resume working.",
                    DialogAction::Spawn(power.standby.clone()),
                    false,
                    true,
                ),
                icon_button(
                    "T&urn Off",
                    "system-shutdown",
                    "Shuts down Reprise so that you can safely turn off the computer.",
                    DialogAction::EndSession(Goal::TurnOff, power.poweroff.clone()),
                    true,
                    true,
                ),
                icon_button(
                    "&Restart",
                    "system-restart",
                    "Shuts down Reprise and then starts Reprise again.",
                    DialogAction::EndSession(Goal::Restart, power.reboot.clone()),
                    false,
                    true,
                ),
                button("Cancel", DialogAction::Cancel, false),
            ],
            ..Dialog::new("Turn off computer", Style::Welcome, Placement::UpperThird)
        }
    }

    pub fn cycle_page(&mut self, back: bool) -> Outcome {
        match self.form_mut() {
            Some(f) => f.cycle_page(back),
            None => Outcome::Nothing,
        }
    }

    /// A key. A form or the chooser takes it first. Otherwise Enter presses the default button,
    /// Escape cancels, text edits the field; without a field Space presses the focused button and a
    /// letter the button with that access key. With Run's drop list open, Up/Down move the
    /// highlight, Enter takes it, Escape closes the list only. The file picker's keys go through
    /// `picker_mut`.
    pub fn key(&mut self, key: Key) -> Outcome {
        if let Some(f) = self.form_mut() {
            return f.key(key);
        }
        // Ctrl+Enter, Ctrl+Home and Ctrl+End are their own keys only in
        // a form's memo.
        let key = match key {
            Key::NewLine => Key::Enter,
            Key::DocEdge { end: false, select } => Key::Edit(EditKey::Home { select }),
            Key::DocEdge { end: true, select } => Key::Edit(EditKey::End { select }),
            k => k,
        };
        if let Some(c) = self.copy_mut() {
            let key = match key {
                Key::Edit(k) => match reprisekit::controls::form::flat_key(&k) {
                    Some(k) => k,
                    None => return Outcome::Nothing,
                },
                k => k,
            };
            let out = match key {
                Key::Access(ch) if ch.eq_ignore_ascii_case(&'d') => c.cp.toggle_details(),
                k => {
                    let ev = c.cp.form.key(k);
                    c.cp.event(ev)
                }
            };
            return Self::copy_outcome(out);
        }
        if let Some(t) = self.tray_mut() {
            // Nothing in it edits text: a space is Space.
            let key = match key {
                Key::Edit(k) => match reprisekit::controls::form::flat_key(&k) {
                    Some(k) => k,
                    None => return Outcome::Nothing,
                },
                k => k,
            };
            return Self::tray_outcome(t.key(key));
        }
        if let Some(c) = self.chooser_mut() {
            // The chooser has no edit field: a space is Space, a letter
            // the list's type-ahead.
            let key = match key {
                Key::Edit(k) => match reprisekit::controls::form::flat_key(&k) {
                    Some(k) => k,
                    None => return Outcome::Nothing,
                },
                k => k,
            };
            let ev = c.ow.key(key);
            return self.chooser_outcome(ev);
        }
        if self.buttons_only() {
            // Left and Right come in as editing keys; with no field to
            // take them they are the plain arrows.
            let arrow = match &key {
                Key::Edit(EditKey::Left { .. }) => Key::Left,
                Key::Edit(EditKey::Right { .. }) => Key::Right,
                _ => key.clone(),
            };
            match arrow {
                Key::Tab { back } => return self.move_default(!back, true),
                Key::Right | Key::Down => return self.move_default(true, false),
                Key::Left | Key::Up => return self.move_default(false, false),
                _ => {}
            }
        }
        if self.field.is_some()
            && let Some(out) = self.field_focus_key(&key)
        {
            return out;
        }
        if let Some(f) = self.field.as_mut().filter(|f| f.combo.open.is_some()) {
            match key {
                Key::Escape | Key::Enter | Key::Drop => {
                    f.combo.open = None;
                    return Outcome::Redraw;
                }
                Key::Up | Key::Down => {
                    return redraw_if(f.combo.step(key == Key::Down));
                }
                Key::Access(_) | Key::Backspace | Key::Edit(_) | Key::Char(_) | Key::Space => {
                    f.combo.open = None
                }
                _ => return Outcome::Nothing,
            }
        }
        match key {
            Key::Escape => Outcome::Close,
            Key::Enter => match self.buttons.iter().position(|b| b.default) {
                Some(i) => self.press(i),
                None => Outcome::Nothing,
            },
            Key::Up | Key::Down => match self.field.as_mut() {
                Some(f) => redraw_if(f.combo.step(key == Key::Down)),
                None => Outcome::Nothing,
            },
            Key::Drop => match self.field.as_mut() {
                Some(f) => redraw_if(f.combo.toggle()),
                None => Outcome::Nothing,
            },
            Key::Backspace => match self.field.as_mut().and_then(Field::edit_mut) {
                Some(e) => redraw_if(e.key(EditKey::Backspace { word: false })),
                None => Outcome::Nothing,
            },
            // Typed text goes into the field. Without one the default
            // button has the focus, so a space presses it and a letter
            // is a button's access key.
            Key::Edit(EditKey::Insert(s)) => match self.field.as_mut().and_then(Field::edit_mut) {
                Some(e) => {
                    e.key(EditKey::Insert(s));
                    Outcome::Redraw
                }
                None if s == " " => match self.buttons.iter().position(|b| b.default) {
                    Some(i) => self.press(i),
                    None => Outcome::Nothing,
                },
                None => self.press_access(&s),
            },
            Key::Edit(k) => match self.field.as_mut().and_then(Field::edit_mut) {
                Some(e) => redraw_if(e.key(k)),
                None => Outcome::Nothing,
            },
            Key::Char(c) => self.key(Key::Edit(EditKey::Insert(c.to_string()))),
            Key::Space => self.key(Key::Edit(EditKey::Insert(" ".into()))),
            Key::Access(c) => self.press_access(&c.to_string()),
            Key::Tab { .. } | Key::PageUp | Key::PageDown => Outcome::Nothing,
            // Left, Right, Home and End reach a field folded into an
            // `Edit` key; bare, they mean nothing here.
            Key::Left | Key::Right | Key::Home | Key::End => Outcome::Nothing,
            Key::NewLine | Key::DocEdge { .. } => unreachable!("read as Enter, Home or End above"),
        }
    }

    /// The pointer let go over `over`. A form's or the chooser's held
    /// push button acts now (on the release, not the press), the chooser's
    /// only if `over` is still that button, and a held thumb lets go.
    pub fn release(&mut self, over: Option<DialogHit>) -> Outcome {
        if self.field.as_mut().is_some_and(|f| f.thumb.release()) {
            return Outcome::Redraw;
        }
        if self.field_lit.holding() {
            self.field_lit.motion(field_part(over));
            self.field_lit.release();
            return Outcome::Redraw;
        }
        if let Some(p) = self.picker_mut() {
            return p.release();
        }
        if let Some(c) = self.chooser_mut() {
            let h = match over {
                Some(DialogHit::Chooser(h)) => Some(h),
                _ => None,
            };
            let ev = c.ow.release(h);
            return self.chooser_outcome(ev);
        }
        if let Some(t) = self.tray_mut() {
            return Self::tray_outcome(t.release(tray_part(over)));
        }
        if let Some(c) = self.copy_mut() {
            let ev = c.cp.form.release();
            return Self::copy_outcome(c.cp.event(ev));
        }
        let Some(f) = self.form_mut() else {
            return Outcome::Nothing;
        };
        let Some(form) = f.open_page_mut() else {
            return Outcome::Nothing;
        };
        let ev = form.release();
        f.event(ev)
    }

    /// A left press other than on a button. A form or the chooser takes it first; on the drop arrow
    /// it toggles the list, on a row takes the entry, on the scroll arrows scrolls, elsewhere
    /// closes an open list. Not the file picker's (see `key`).
    pub fn click(&mut self, hit: Option<DialogHit>) -> Outcome {
        if let Some(f) = self.form_mut() {
            return f.click(hit);
        }
        if let Some(c) = self.chooser_mut() {
            let h = match hit {
                Some(DialogHit::Chooser(h)) => Some(h),
                _ => None,
            };
            let ev = c.ow.click(h);
            return self.chooser_outcome(ev);
        }
        if let Some(t) = self.tray_mut() {
            return Self::tray_outcome(t.click(tray_part(hit)));
        }
        if let Some(c) = self.copy_mut() {
            let out = match hit {
                Some(DialogHit::CopyDetails) => c.cp.toggle_details(),
                Some(DialogHit::Control(h)) => {
                    let ev = c.cp.form.click(Some(h));
                    c.cp.event(ev)
                }
                _ => {
                    let ev = c.cp.form.click(None);
                    c.cp.event(ev)
                }
            };
            return Self::copy_outcome(out);
        }
        if hit == Some(DialogHit::Drop) && self.field.is_some() {
            // The drop button draws held until the release, and the
            // field takes the focus.
            self.field_lit.press(Some(FieldPart::Drop), false);
            self.button_focus = None;
        }
        let Some(f) = self.field.as_mut() else {
            return Outcome::Nothing;
        };
        let c = &mut f.combo;
        let open = c.open.is_some();
        match hit {
            Some(DialogHit::Drop) => {
                c.toggle();
                Outcome::Redraw
            }
            Some(DialogHit::Row(i)) if open => {
                c.pick(i);
                Outcome::Redraw
            }
            // The bar is part of the list: a press on it keeps it open,
            // and one on the thumb grabs it until the release.
            Some(DialogHit::ListBar(bar)) if open => redraw_if(c.press_bar(bar, &mut f.thumb)),
            _ if open => {
                c.open = None;
                Outcome::Redraw
            }
            _ => Outcome::Nothing,
        }
    }

    /// The second press of a double click. On a row of the chooser's list
    /// it takes the row and presses OK, in a form's edit field it selects
    /// the word there; anywhere else it is a plain press.
    pub fn double_click(&mut self, hit: Option<DialogHit>) -> Outcome {
        if let Some(f) = self.form_mut() {
            let h = match hit {
                Some(DialogHit::Control(h)) => Some(h),
                _ => None,
            };
            let Some(form) = f.open_page_mut() else {
                return Outcome::Nothing;
            };
            let ev = form.double_click(h);
            return f.event(ev);
        }
        if let Some(c) = self.chooser_mut() {
            let h = match hit {
                Some(DialogHit::Chooser(h)) => Some(h),
                _ => None,
            };
            let ev = c.ow.double_click(h);
            return self.chooser_outcome(ev);
        }
        self.click(hit)
    }

    /// The wheel turned `rows` rows (down is positive) over `hit`. A form's page takes it as
    /// `Form::wheel` does, the chooser's list scrolls while the pointer is on it, Run's open drop
    /// list scrolls anywhere. Not the tray dialog's or the file picker's.
    pub fn wheel(&mut self, hit: Option<DialogHit>, rows: i32) -> Outcome {
        if let Some(f) = self.form_mut() {
            let h = match hit {
                Some(DialogHit::Control(h)) => Some(h),
                _ => None,
            };
            let Some(form) = f.open_page_mut() else {
                return Outcome::Nothing;
            };
            let ev = form.wheel(h, rows);
            return f.event(ev);
        }
        if let Some(c) = self.chooser_mut() {
            let h = match hit {
                Some(DialogHit::Chooser(h)) => Some(h),
                _ => None,
            };
            let ev = c.ow.wheel(h, rows);
            return self.chooser_outcome(ev);
        }
        match self.field.as_mut() {
            Some(f) if f.combo.open.is_some() => redraw_if(f.combo.scroll(rows)),
            _ => Outcome::Nothing,
        }
    }

    /// A press in the field `x` px from its text's origin (the list is
    /// closed): the caret goes to the nearest char boundary, extending the
    /// selection with Shift held (`extend`), else dropping it.
    pub fn press_field(&mut self, text: &TextRenderer, px: u32, x: i32, extend: bool) -> Outcome {
        let Some(f) = self.field.as_mut().filter(|f| f.combo.open.is_none()) else {
            return Outcome::Nothing;
        };
        let i = f.combo.edit_index(text, px, x);
        let placed = f.edit_mut().is_some_and(|e| e.place(i, extend));
        redraw_if(self.button_focus.take().is_some() || placed)
    }

    pub fn double_click_field(&mut self, text: &TextRenderer, px: u32, x: i32) -> Outcome {
        let Some(f) = self.field.as_mut().filter(|f| f.combo.open.is_none()) else {
            return Outcome::Nothing;
        };
        let i = f.combo.edit_index(text, px, x);
        let Some(e) = f.edit_mut() else {
            return Outcome::Nothing;
        };
        let before = e.clone();
        e.select_word_at(i);
        redraw_if(*e != before)
    }

    pub fn drag_field(&mut self, text: &TextRenderer, px: u32, x: i32) -> Outcome {
        self.press_field(text, px, x, true)
    }

    pub fn holding_thumb(&self) -> bool {
        self.field.as_ref().is_some_and(|f| f.thumb.holding())
    }

    pub fn drag_list(&mut self, lay: &DialogLayout, y: i32) -> Outcome {
        let (Some(f), Some(field)) = (self.field.as_mut(), lay.field) else {
            return Outcome::Nothing;
        };
        let cl = combo::layout(&f.combo, field, &lay.controls);
        redraw_if(f.combo.drag_bar(&cl, &f.thumb, y))
    }

    /// A message box or a Welcome panel: a dialog whose only controls are
    /// its buttons, so the keyboard focus is always on one of them.
    fn buttons_only(&self) -> bool {
        matches!(
            self.style,
            Style::Classic { .. } | Style::EndProgram | Style::Welcome
        ) && self.field.is_none()
    }

    /// Move focus to the next enabled button (`forward`) or the previous. Tab wraps (`wrap`);
    /// arrows stop at the ends. The focused button is the default.
    fn move_default(&mut self, forward: bool, wrap: bool) -> Outcome {
        let n = self.buttons.len();
        let Some(from) = self.buttons.iter().position(|b| b.default) else {
            return Outcome::Nothing;
        };
        let order: Vec<usize> = if forward {
            (from + 1..n)
                .chain(if wrap { 0..from } else { 0..0 })
                .collect()
        } else {
            (0..from)
                .rev()
                .chain(if wrap {
                    (from + 1..n).rev()
                } else {
                    (0..0).rev()
                })
                .collect()
        };
        let Some(to) = order.into_iter().find(|&i| self.buttons[i].enabled) else {
            return Outcome::Nothing;
        };
        self.buttons[from].default = false;
        self.buttons[to].default = true;
        Outcome::Redraw
    }

    /// A key in a dialog with a field when Tab or a focused button answers it: Tab/Shift+Tab walk
    /// the field and enabled buttons; on a button Left/Up and Right/Down move to the previous/next,
    /// Enter and Space press, Escape closes, a letter is an access key. `None` leaves the key to
    /// the field.
    fn field_focus_key(&mut self, key: &Key) -> Option<Outcome> {
        if let Key::Tab { back } = *key {
            if let Some(f) = self.field.as_mut() {
                f.combo.open = None;
            }
            let stops: Vec<Option<usize>> = std::iter::once(None)
                .chain(
                    (0..self.buttons.len())
                        .filter(|&i| self.buttons[i].enabled)
                        .map(Some),
                )
                .collect();
            let at = stops
                .iter()
                .position(|&s| s == self.button_focus)
                .unwrap_or(0);
            let n = stops.len();
            self.button_focus = stops[if back { (at + n - 1) % n } else { (at + 1) % n }];
            return Some(Outcome::Redraw);
        }
        let i = self.button_focus?;
        let enabled = |&j: &usize| self.buttons[j].enabled;
        let arrow = match key {
            Key::Left | Key::Up | Key::Edit(EditKey::Left { .. }) => {
                Some((0..i).rev().find(enabled))
            }
            Key::Right | Key::Down | Key::Edit(EditKey::Right { .. }) => {
                Some((i + 1..self.buttons.len()).find(enabled))
            }
            _ => None,
        };
        if let Some(to) = arrow {
            let Some(to) = to else {
                return Some(Outcome::Nothing);
            };
            self.button_focus = Some(to);
            return Some(Outcome::Redraw);
        }
        Some(match key {
            Key::Escape => Outcome::Close,
            Key::Enter | Key::Space => self.press(i),
            Key::Edit(EditKey::Insert(s)) if s == " " => self.press(i),
            Key::Edit(EditKey::Insert(s)) => self.press_access(s),
            Key::Char(c) | Key::Access(c) => self.press_access(&c.to_string()),
            _ => Outcome::Nothing,
        })
    }

    /// Press the enabled button whose access key is `s`'s first letter.
    /// The field's label's access key puts the focus back on the field.
    fn press_access(&mut self, s: &str) -> Outcome {
        let key = s.chars().next().and_then(|c| c.to_lowercase().next());
        if key.is_some()
            && self.button_focus.is_some()
            && self
                .field
                .as_ref()
                .is_some_and(|f| access_key(&f.label) == key)
        {
            self.button_focus = None;
            return Outcome::Redraw;
        }
        let hit = self
            .buttons
            .iter()
            .position(|b| b.enabled && access_key(&b.label) == key);
        match hit {
            Some(i) => self.press(i),
            None => Outcome::Nothing,
        }
    }

    pub fn set_field(&mut self, text: &str) -> Outcome {
        match self.field.as_mut() {
            Some(f) => {
                f.combo.open = None;
                if let Some(e) = f.edit_mut() {
                    e.set_selected(text);
                }
                Outcome::Redraw
            }
            None => Outcome::Nothing,
        }
    }

    pub fn paste(&mut self, text: &str) -> Outcome {
        if let Some(f) = self.form_mut() {
            let Some(form) = f.open_page_mut() else {
                return Outcome::Nothing;
            };
            let ev = form.paste(text.to_string());
            return f.event(ev);
        }
        match self.field.as_mut().and_then(Field::edit_mut) {
            Some(e) => redraw_if(e.paste(text, &[])),
            None => Outcome::Nothing,
        }
    }

    /// A reader moved the keyboard to what lies at dialog point (`x`, `y`), as Tab would: a form's
    /// control, or one of the dialog's buttons. Nothing is pressed.
    pub fn focus_at(
        &mut self,
        layout: &DialogLayout,
        x: i32,
        y: i32,
        text: &TextRenderer,
        px: u32,
    ) -> Outcome {
        match layout.hit(self, x, y, text, px) {
            Some(DialogHit::Button(i)) => self.focus_button(i),
            Some(DialogHit::Browse(_)) => match (self.picker_mut(), layout.page) {
                (Some(p), Some(client)) => p.focus_at(client, (x, y), text, px),
                _ => Outcome::Nothing,
            },
            Some(DialogHit::Chooser(h)) => match self.chooser_mut() {
                Some(c) => redraw_if(c.ow.focus_on(Some(h)) != openwith::Event::None),
                None => Outcome::Nothing,
            },
            Some(DialogHit::Tray(h)) => match self.tray_mut() {
                Some(t) => redraw_if(t.focus_on(Some(h)) != traycustomize::Event::None),
                None => Outcome::Nothing,
            },
            Some(DialogHit::Control(_)) => {
                if let Some(f) = self.form_mut() {
                    return redraw_if(match &mut f.body {
                        FormBody::Sheet(s) => {
                            s.focus_at(x, y, text, px)
                                != reprisekit::controls::propsheet::Event::None
                        }
                        FormBody::Plain(form) => {
                            form.focus_at(x, y, text, px) != reprisekit::controls::form::Event::None
                        }
                    });
                }
                match (self.copy_mut(), layout.page) {
                    (Some(c), Some(client)) => {
                        let ev = c.cp.form.focus_at(x - client.x, y - client.y, text, px);
                        redraw_if(ev != reprisekit::controls::form::Event::None)
                    }
                    _ => Outcome::Nothing,
                }
            }
            _ => Outcome::Nothing,
        }
    }

    /// Button `i` takes the keyboard: in a dialog with a field it wears the focus as Tab gives it,
    /// in one of buttons only it becomes the default as the arrows make it.
    fn focus_button(&mut self, i: usize) -> Outcome {
        if !self.buttons.get(i).is_some_and(|b| b.enabled) {
            return Outcome::Nothing;
        }
        if let Some(f) = self.field.as_mut() {
            if self.button_focus == Some(i) {
                return Outcome::Nothing;
            }
            f.combo.open = None;
            self.button_focus = Some(i);
            return Outcome::Redraw;
        }
        if !self.buttons_only() || self.buttons[i].default {
            return Outcome::Nothing;
        }
        for (k, b) in self.buttons.iter_mut().enumerate() {
            b.default = k == i;
        }
        Outcome::Redraw
    }

    /// A reader asked to see row `row` of the list at dialog point (`x`, `y`): Browse's file list,
    /// Open With's programs, the tray's items, an open drop list, a sheet's page list, or a form
    /// page's list box or tree, scrolls it in.
    pub fn reveal(
        &mut self,
        layout: &DialogLayout,
        (x, y): (i32, i32),
        row: usize,
        text: &TextRenderer,
        px: u32,
    ) -> Outcome {
        if let (Some(p), Some(client)) = (self.picker_mut(), layout.page) {
            return p.reveal(client, (x, y), row, text, px);
        }
        if let Some(c) = self.chooser_mut() {
            return redraw_if(c.ow.reveal(row) != openwith::Event::None);
        }
        if let (Some(t), Some(client)) = (self.tray_mut(), layout.page) {
            let lay = t.layout(text, px, (client.x, client.y));
            let hit = t.hit(&lay, x, y);
            return redraw_if(t.reveal(hit, row));
        }
        if let (Some(f), Some(fr)) = (self.field.as_mut(), layout.field)
            && f.combo.open.is_some()
            && combo::layout(&f.combo, fr, &layout.controls)
                .list
                .contains(x, y)
        {
            return redraw_if(f.combo.reveal(row));
        }
        let Some(f) = self.form_mut() else {
            return Outcome::Nothing;
        };
        if let FormBody::Sheet(s) = &mut f.body {
            let ev = s.reveal(x, y, row, text, px);
            return redraw_if(ev != reprisekit::controls::propsheet::Event::None);
        }
        let Some(form) = f.open_page_mut() else {
            return Outcome::Nothing;
        };
        let ev = form.reveal(x, y, row, text, px);
        f.event(ev)
    }

    /// A drag at dialog point (`x`, `y`): Run's field, or a form's edit field, shows a drop
    /// caret there and takes the drag (`true`); anywhere else the drop caret goes.
    pub fn drag_over(
        &mut self,
        layout: &DialogLayout,
        x: i32,
        y: i32,
        text: &TextRenderer,
        px: u32,
    ) -> bool {
        if let Some(f) = self.form_mut() {
            return f
                .open_page_mut()
                .is_some_and(|form| form.drag_over(x, y, text, px));
        }
        match (self.field.as_mut(), layout.field) {
            (Some(f), Some(r)) => {
                f.combo
                    .drag_over(&combo::layout(&f.combo, r, &layout.controls), x, y)
            }
            _ => false,
        }
    }

    /// The drag left or ended: `true` when a drop caret was showing.
    pub fn drag_leave(&mut self) -> bool {
        if let Some(f) = self.form_mut() {
            return f.open_page_mut().is_some_and(|form| form.drag_leave());
        }
        self.field.as_mut().is_some_and(|f| f.combo.drag_leave())
    }

    /// A drag resting at a field's edge scrolls it a step; `true` when it moved.
    pub fn drag_tick(&mut self, layout: &DialogLayout, text: &TextRenderer, px: u32) -> bool {
        if let Some(f) = self.form_mut() {
            return f
                .open_page_mut()
                .is_some_and(|form| form.drag_tick(text, px));
        }
        match (self.field.as_mut(), layout.field) {
            (Some(f), Some(r)) => {
                let lay = combo::layout(&f.combo, r, &layout.controls);
                f.combo.drag_tick(&lay, text, px)
            }
            _ => false,
        }
    }

    /// A drag dropped `line` on the field under it: it goes in at the drop caret, selected, and
    /// the field takes the keyboard.
    pub fn drop_text(
        &mut self,
        layout: &DialogLayout,
        line: &crate::textedit::DropLine,
        text: &TextRenderer,
        px: u32,
    ) -> Outcome {
        if let Some(f) = self.form_mut() {
            let Some(form) = f.open_page_mut() else {
                return Outcome::Nothing;
            };
            let ev = form.drop_text(line, text, px);
            return f.event(ev);
        }
        let (Some(f), Some(r)) = (self.field.as_mut(), layout.field) else {
            return Outcome::Nothing;
        };
        let lay = combo::layout(&f.combo, r, &layout.controls);
        if !f.combo.drop_text(&lay, line, text, px) {
            return Outcome::Nothing;
        }
        self.button_focus = None;
        Outcome::Redraw
    }

    /// The edit control that has the keyboard: a form's focused field or
    /// memo, Browse's File name field, else the dialog's own field.
    pub fn focused_edit(&self) -> Option<FocusedEdit<'_>> {
        if let Some(p) = self.picker() {
            return p.dlg.focused_edit().map(FocusedEdit::Line);
        }
        match self.form() {
            Some(f) => f.open_page()?.focused_edit(),
            None => self
                .field
                .as_ref()
                .filter(|_| self.button_focus.is_none())
                .and_then(Field::edit)
                .map(FocusedEdit::Line),
        }
    }

    /// The balloon tip for button `i`: its label (access key stripped) over
    /// its description. None on a plain button or a greyed one, which
    /// shows nothing.
    pub fn tip(&self, i: usize) -> Option<(String, String)> {
        let b = self.buttons.get(i).filter(|b| b.enabled)?;
        let tip = b.tip.clone()?;
        Some((split_mnemonic(&b.label).0, tip))
    }

    pub fn sheet_path(&self) -> Option<&std::path::Path> {
        match &self.form()?.kind {
            FormKind::Properties(b) => Some(b.path.as_path()),
            _ => None,
        }
    }

    pub fn sheet_name(&self) -> Option<&str> {
        match &self.form()?.kind {
            FormKind::Properties(b) => Some(&b.name),
            _ => None,
        }
    }

    /// Put a folder walk's `t` on the sheet's Size, Size on disk and
    /// Contains rows; a set's sheet adds it to what its items hold and
    /// shows the sums with the count. False when the sheet has none of
    /// them.
    pub fn set_folder_totals(&mut self, t: reprisefs::props::FolderTotals) -> bool {
        use reprisefs::props::{format_contains, format_size};
        if let Some(f) = self.form_mut()
            && let (FormKind::SetProperties(set), Some(form)) =
                (&mut f.kind, f.body.pages_mut().first_mut())
        {
            set.add_totals(t);
            form.set_text(reprisekit::controls::filesheet::COUNT, &set.count_text());
            form.set_text("v_size", &format_size(set.size));
            form.set_text("v_disk", &format_size(set.on_disk));
            return true;
        }
        let Some((_, form)) = self.props_mut() else {
            return false;
        };
        let mut set = false;
        for (id, value) in [
            ("v_size", format_size(t.size)),
            ("v_disk", format_size(t.on_disk)),
            ("v_has", format_contains(t.files, t.folders)),
        ] {
            if form.item(id).is_some() {
                form.set_text(id, &value);
                set = true;
            }
        }
        set
    }

    /// Put `name` (with its `icon`) on the sheet's Opens with row, after
    /// the chooser behind Change... changed the file's default program.
    pub fn set_opens_with(&mut self, name: String, icon: Option<String>) -> bool {
        let Some((base, form)) = self.props_mut() else {
            return false;
        };
        if form.item("v_opens").is_none() {
            return false;
        }
        form.set_text("v_opens", &name);
        base.opens_icon = icon;
        true
    }

    fn live_button(&self, hit: Option<DialogHit>) -> Option<usize> {
        match hit {
            Some(DialogHit::Button(i)) if self.buttons.get(i).is_some_and(|b| b.enabled) => Some(i),
            _ => None,
        }
    }

    /// The pointer went down on classic button `i`: it draws pushed until
    /// the release. False (and nothing held) for a disabled button.
    pub fn press_button(&mut self, i: usize) -> bool {
        if !self.buttons.get(i).is_some_and(|b| b.enabled) {
            return false;
        }
        self.pointer.press(Some(i), false);
        true
    }

    pub fn holding_button(&self) -> bool {
        self.pointer.holding()
    }

    /// The pointer is taken away mid-press (the Start menu or another
    /// overlay opened over it): whatever it held lets go without acting.
    pub fn cancel_press(&mut self) -> Outcome {
        self.pointer.clear();
        self.caption_box.clear();
        self.help_box.clear();
        self.field_lit.clear();
        if let Some(f) = self.field.as_mut() {
            f.thumb.release();
        }
        if let Some(p) = self.picker_mut() {
            // A toolbar button, which acted on the press, or a column
            // title is held there: either lets go without acting.
            p.release();
        }
        if let Some(c) = self.chooser_mut() {
            c.ow.release(None);
        }
        if let Some(page) = self.form_mut().and_then(|f| f.open_page_mut()) {
            page.cancel_pointer();
        }
        Outcome::Redraw
    }

    /// The held box draws pushed, not hot, so dragged off it draws plain.
    pub fn press_caption(&mut self, b: CaptionButton) {
        self.caption_hot = None;
        self.caption_box.press(Some(b), false);
    }

    pub fn drag_caption(&mut self, over: Option<DialogHit>) -> bool {
        self.caption_box.holding() && self.caption_box.motion(caption_button(over))
    }

    /// The pointer let go of the held caption button: the button when it was over it, which acts.
    pub fn release_caption(&mut self) -> Option<CaptionButton> {
        self.caption_box.release()
    }

    /// The pointer went down on the caption's ? box: it draws pressed
    /// while the pointer stays on it, and its release does nothing.
    pub fn press_help(&mut self) {
        self.help_box.press(Some(()), false);
    }

    pub fn drag_help(&mut self, on: bool) -> bool {
        self.help_box.holding() && self.help_box.motion(on.then_some(()))
    }

    pub fn release_help(&mut self) -> bool {
        let before = self.help_box.lit();
        self.help_box.release();
        self.help_box.lit() != before
    }

    pub fn drag_drop(&mut self, over: Option<DialogHit>) -> bool {
        self.field_lit.holding() && self.field_lit.motion(field_part(over))
    }

    pub fn drag_picker(&mut self, over: Option<DialogHit>) -> bool {
        self.picker_mut()
            .is_some_and(|p| p.hover(browse_part(over)))
    }

    pub fn drag_button(&mut self, over: Option<usize>) -> bool {
        self.pointer.holding() && self.pointer.motion(over)
    }

    /// The pointer let go: the held button acts only if the release lands
    /// on it. `None` when no button was held.
    pub fn release_button(&mut self, over: Option<usize>) -> Option<Outcome> {
        if !self.pointer.holding() {
            return None;
        }
        self.pointer.motion(over);
        Some(match self.pointer.release() {
            Some(b) => self.press(b),
            None => Outcome::Redraw,
        })
    }

    pub fn lit(&self) -> Lit<usize> {
        self.pointer.lit()
    }

    /// The pointer rests on `over` at dialog point (`x`, `y`) with nothing
    /// held: the classic button, the closed tab or the page's control
    /// under it lights, and the one it left goes cold. True when a look
    /// that is `styled` has something to redraw; Classic draws no hot
    /// state, except a Welcome panel's hot icon.
    pub fn hover(&mut self, over: Option<DialogHit>, (x, y): (i32, i32), styled: bool) -> bool {
        let button = self.live_button(over);
        let mut redraw = self.pointer.motion(button) && (styled || self.lights_icons());
        let help = matches!(over, Some(DialogHit::Help)).then_some(());
        redraw |= self.help_box.motion(help) && styled;
        redraw |= self.field_lit.motion(field_part(over)) && styled;
        if let Some(p) = self.picker_mut() {
            redraw |= p.hover(browse_part(over));
        }
        if let Some(t) = self.tray_mut() {
            redraw |= t.motion(tray_part(over)) != traycustomize::Event::None && styled;
        }
        if let Some(c) = self.copy_mut() {
            let on = match over {
                Some(DialogHit::Control(h)) => Some(h),
                _ => None,
            };
            redraw |= c.cp.form.motion(x, y, on) != reprisekit::controls::form::Event::None;
        }
        if let Some(f) = self.form_mut() {
            let holding = f.open_page().is_some_and(|p| p.holding());
            let tab = match over {
                Some(DialogHit::Tab(t)) if !holding => Some(t),
                _ => None,
            };
            if let Some(s) = f.sheet_mut() {
                redraw |= s.tabs.hover(tab) && styled;
            }
            let control = match over {
                Some(DialogHit::Control(h)) => Some(h),
                _ => None,
            };
            if let Some(page) = f.open_page_mut() {
                redraw |= page.motion(x, y, control) != reprisekit::controls::form::Event::None;
            }
        }
        redraw
    }

    pub fn leave(&mut self, styled: bool) -> bool {
        let mut redraw = self.pointer.leave() && (styled || self.lights_icons());
        redraw |= (self.help_box.leave() | self.field_lit.leave()) && styled;
        if let Some(p) = self.picker_mut() {
            redraw |= p.leave();
        }
        if let Some(t) = self.tray_mut() {
            redraw |= t.leave() != traycustomize::Event::None && styled;
        }
        if let Some(c) = self.copy_mut() {
            redraw |= c.cp.form.leave() != reprisekit::controls::form::Event::None;
        }
        if let Some(f) = self.form_mut() {
            if let Some(s) = f.sheet_mut() {
                redraw |= s.tabs.hover(None) && styled;
            }
            if let Some(page) = f.open_page_mut() {
                redraw |= page.leave() != reprisekit::controls::form::Event::None;
            }
        }
        redraw
    }

    /// Whether the pointer lights buttons in every look: a Welcome
    /// panel's icon buttons draw hot under it.
    fn lights_icons(&self) -> bool {
        matches!(self.style, Style::Welcome)
    }

    /// Press button `i`; a disabled one does nothing. Run's Browse...
    /// hands the focus back to the field, where the picker's answer (a
    /// picked path, or nothing on Cancel) leaves Enter on OK.
    pub fn press(&mut self, i: usize) -> Outcome {
        let Some(b) = self.buttons.get(i).filter(|b| b.enabled) else {
            return Outcome::Nothing;
        };
        if matches!(b.action, DialogAction::BrowseRun) {
            self.button_focus = None;
        }
        let b = &self.buttons[i];
        match &b.action {
            DialogAction::Cancel => Outcome::Close,
            DialogAction::Spawn(cmd) => Outcome::Spawn(cmd.clone()),
            DialogAction::EndSession(goal, cmd) => Outcome::EndSession(*goal, cmd.clone()),
            DialogAction::Delete { paths, erase } => Outcome::Delete {
                paths: paths.clone(),
                erase: *erase,
            },
            DialogAction::EmptyBin => Outcome::EmptyBin,
            DialogAction::BrowseRun => Outcome::BrowseRun,
            DialogAction::Act(a) => Outcome::Act(a.clone()),
            DialogAction::HideSystemIcon(name) => Outcome::HideSystemIcon(name.clone()),
            DialogAction::DeleteIcons { hide, files } => Outcome::DeleteIcons {
                hide: hide.clone(),
                files: files.clone(),
            },
            DialogAction::EndProgram(pid) => Outcome::EndProgram(*pid),
            DialogAction::TurnOn(aid) => Outcome::TurnOn(*aid),
            DialogAction::TooBig { asker, answer } => Outcome::TooBig {
                asker: asker.clone(),
                answer: *answer,
            },
            DialogAction::RunField => {
                let cmd = self
                    .field
                    .as_ref()
                    .map(|f| f.combo.text().trim().to_string())
                    .unwrap_or_default();
                if cmd.is_empty() {
                    Outcome::Nothing
                } else {
                    Outcome::Run(cmd)
                }
            }
        }
    }
}

/// Logical-pixel positions inside the dialog; `scaled` for drawing on a
/// HiDPI output, as is for hit testing against layout coordinates.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DialogLayout {
    pub size: (i32, i32),
    /// The frame around the body in the theme's metrics; None on a Welcome
    /// panel, which draws its own.
    pub frame: Option<FrameGeometry>,
    pub caption: Rect,
    /// The caption's close button (none on a Welcome panel): 16x14, 2 px in from the caption's top
    /// and right.
    pub close: Option<Rect>,
    /// Caption ? button, 2 px left of the close button.
    pub help: Option<Rect>,
    /// A windowed dialog's minimize box, where the ? button would be.
    pub minimize: Option<Rect>,
    pub icon: Option<Rect>,
    pub lines: Vec<(Rect, String)>,
    pub field_label: Option<Rect>,
    pub field: Option<Rect>,
    /// The combo box's drop button inside the field's right end, when the
    /// field has a history.
    pub drop: Option<Rect>,
    /// The drop list under the field (drawn only while open): `LIST_ROWS`
    /// rows of the look's `combo_row` inside a 1 px border.
    pub list: Option<Rect>,
    pub buttons: Vec<Rect>,
    /// A form dialog's tab rects and its page, or the Browse dialog's or
    /// the Open With chooser's client area.
    pub tabs: Vec<Rect>,
    pub page: Option<Rect>,
    /// The look's control sizes it was laid out with, which `hit`,
    /// `scrollbar` and `access` lay the Run list, the chooser and the
    /// Browse dialog out with.
    pub controls: ControlMetrics,
}

/// What a point on a dialog lands on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogHit {
    Button(usize),
    Close,
    /// A windowed dialog's minimize box.
    Minimize,
    Help,
    Caption,
    /// A sizable frame's border (Browse's), by the edges a drag from
    /// there moves. See [`DialogLayout::sizing_edges`].
    Resize(reprisekit::frame::Edges),
    /// The text field itself, with the point's x from the text's origin.
    Field(i32),
    ListBar(BarHit),
    Drop,
    Row(usize),
    Tab(usize),
    Control(reprisekit::controls::form::Hit),
    Browse(reprisekit::controls::filedlg::Hit),
    Chooser(openwith::Hit),
    Tray(traycustomize::Hit),
    /// The copy progress dialog's More/Fewer details toggle.
    CopyDetails,
}

/// The caption button `over` is on.
pub fn caption_button(over: Option<DialogHit>) -> Option<CaptionButton> {
    match over? {
        DialogHit::Close => Some(CaptionButton::Close),
        DialogHit::Minimize => Some(CaptionButton::Minimize),
        _ => None,
    }
}

/// The frame every dialog is built for: a fixed dialog border is 3 px
/// (the 2 px raised edge and one of face), a pixel thinner than a sizable
/// window's, under an 18 px caption. A look whose frame differs moves the
/// body by `client_shift`.
const DIALOG_FRAME: i32 = dlgtemplate::FRAME;
const CAPTION_H: i32 = dlgtemplate::CAPTION_H;
/// Where a form dialog's client area starts: inside the frame, under the
/// caption.
pub(crate) const FORM_CLIENT: (i32, i32) = (DIALOG_FRAME, DIALOG_FRAME + CAPTION_H);
pub const ICON_PX: i32 = dlgtemplate::ICON_PX;
const CHOOSER_W: i32 = 396;
const CHOOSER_H: i32 = 419;
const _: () = assert!(
    CHOOSER_W == openwith::CLIENT.0 + 2 * DIALOG_FRAME
        && CHOOSER_H == openwith::CLIENT.1 + DIALOG_FRAME + FORM_CLIENT.1
);

/// End Program: a fixed 354x243 template. The 32 px icon, then the text
/// in a column inking from `EP_TEXT_X` to 18 px short of the right edge, each
/// paragraph starting at its own `EP_PARA_Y` rather than on one line grid,
/// and End Now / Cancel (69x23, 9 apart) ending 19 px from the right.
const EP_W: i32 = 354;
const EP_H: i32 = 243;
const EP_ICON: (i32, i32) = (13, 32);
const EP_TEXT_X: i32 = 58;
const EP_TEXT_RIGHT: i32 = 18;
const EP_PARA_Y: [i32; 3] = [35, 74, 120];
const EP_BUTTONS: (i32, i32) = (188, 204);
const EP_BUTTON_W: i32 = 69;
const EP_BUTTON_PITCH: i32 = 78;

/// The Welcome panel: 314x200 with a 1 px black border, a 43 px title
/// band, icon buttons in cells 64 px tall on the gradient with the icon
/// 8 px into the cell and the label 44 px in, and a 60x20 Cancel button
/// 10 px from the right edge in the bottom band. Three cells are 85 px
/// wide and start at 29, 115 and 200; two are 115 px wide and start at
/// 43 and 158.
const WELCOME_W: i32 = 314;
const WELCOME_H: i32 = 200;
const WELCOME_BORDER: i32 = 1;
const WELCOME_TITLE_H: i32 = 43;
const WELCOME_CELL_Y: i32 = 72;
const WELCOME_CELL_H: i32 = 64;
const WELCOME_ICON_Y: i32 = 8;
const WELCOME_LABEL_Y: i32 = 44;
const WELCOME_THREE: (i32, [i32; 3]) = (85, [29, 115, 200]);
const WELCOME_TWO: (i32, [i32; 2]) = (115, [43, 158]);
const WELCOME_CANCEL: (i32, i32, i32) = (60, 20, 167);
/// The art tables' columns across the panel and the gradient's rows, at 100%.
const PANEL_W: i32 = 312;
const PANEL_H: i32 = 110;

/// The Welcome panel under a look with glass, a dialog like any other
/// in its frame: a 312 px wide client, the icon buttons 80x64 push
/// buttons 12 px apart, centred, 14 px under the client's top, each with
/// its icon 8 px and its label 44 px into it; Cancel a standard button
/// 11 px in from the client's right and bottom edges, 14 px under them.
const GLASS_WELCOME_W: i32 = 312;
const GLASS_WELCOME_TOP: i32 = 14;
const GLASS_WELCOME_CELL: (i32, i32) = (80, 64);
const GLASS_WELCOME_GAP: i32 = 12;
const GLASS_WELCOME_MARGIN: i32 = 11;

fn chrome(resizable: bool) -> Chrome {
    Chrome {
        resizable,
        ..Chrome::DIALOG
    }
}

/// How far `theme`'s frame puts a fixed dialog's client area from where
/// the dialog was built: inside `DIALOG_FRAME` under a `CAPTION_H`
/// caption. (0, 0) under the default metrics; Blue's taller caption moves
/// it down.
fn client_shift(theme: &Theme) -> (i32, i32) {
    let g = FrameGeometry::layout(&theme.metrics, 1, 1, false, chrome(false));
    (
        g.client.x - DIALOG_FRAME,
        g.client.y - DIALOG_FRAME - CAPTION_H,
    )
}

impl DialogLayout {
    /// Lay `d` out at scale 1 with the text wrapped at `theme`'s font size,
    /// inside `theme`'s frame. A form dialog must have been fitted to it
    /// first ([`Dialog::fit`]).
    pub fn compute(theme: &Theme, text: &TextRenderer, d: &Dialog) -> DialogLayout {
        DialogLayout {
            controls: theme.look.controls().grown(theme.text),
            ..Self::compute_body(theme, text, d)
        }
    }

    fn compute_body(theme: &Theme, text: &TextRenderer, d: &Dialog) -> DialogLayout {
        let body = match &d.style {
            Style::Welcome if theme.glass().is_some() => Self::compute_welcome_glass(d),
            Style::Welcome => return Self::compute_welcome(d, theme.text),
            Style::Form(f) => return Self::compute_form(theme, text, d, f),
            Style::Chooser(_) => Self::compute_chooser(),
            Style::TrayCustomize(t) => {
                return Self::compute_client(t.client()).framed(theme, d.help);
            }
            Style::Copy(c) => {
                return Self::compute_client(theme.text.size(c.cp.client())).framed_as(
                    theme,
                    d.help,
                    Chrome::WINDOWED_DIALOG,
                );
            }
            Style::Browse(p) => return Self::compute_browse(theme, d.help, p.dlg.client()),
            Style::EndProgram => Self::compute_end_program(theme, text.dialog(d.font()), d),
            Style::Classic {
                template, text_h, ..
            } => return Self::compute_template(theme, text, d, *template, *text_h),
        };
        body.grown(theme.text).framed(theme, d.help)
    }

    /// A body laid out at 100% under `DIALOG_FRAME` and a `CAPTION_H` caption, at the text size
    /// `u`: the client and its rects grow from the client's corner, the icon keeps its size.
    fn grown(mut self, u: TextUnits) -> DialogLayout {
        let (ox, oy) = FORM_CLIENT;
        let map = |r: &mut Rect| {
            let g = u.rect(Rect::new(r.x - ox, r.y - oy, r.width, r.height));
            *r = Rect::new(g.x + ox, g.y + oy, g.width, g.height);
        };
        [&mut self.field_label, &mut self.field, &mut self.page]
            .into_iter()
            .flatten()
            .for_each(map);
        if let Some(icon) = &mut self.icon {
            let g = u.at(Rect::new(icon.x - ox, icon.y - oy, icon.width, icon.height));
            *icon = Rect::new(g.x + ox, g.y + oy, g.width, g.height);
        }
        self.lines.iter_mut().for_each(|(r, _)| map(r));
        self.buttons.iter_mut().for_each(map);
        let pad = (2 * DIALOG_FRAME, DIALOG_FRAME + FORM_CLIENT.1);
        let (w, h) = u.size((self.size.0 - pad.0, self.size.1 - pad.1));
        self.size = (w + pad.0, h + pad.1);
        self
    }

    /// A body laid out for `DIALOG_FRAME` and a `CAPTION_H` caption, put
    /// inside `theme`'s frame: the body moves by `client_shift` and the
    /// dialog grows around it.
    fn framed(self, theme: &Theme, help: bool) -> DialogLayout {
        self.framed_as(theme, help, chrome(false))
    }

    /// [`Self::framed`] in a frame with `chrome`'s caption buttons.
    fn framed_as(mut self, theme: &Theme, help: bool, chrome: Chrome) -> DialogLayout {
        let (dx, dy) = client_shift(theme);
        let mv = |r: &mut Rect| {
            r.x += dx;
            r.y += dy;
        };
        let rects = [
            &mut self.icon,
            &mut self.field_label,
            &mut self.field,
            &mut self.drop,
            &mut self.list,
            &mut self.page,
        ];
        rects.into_iter().flatten().for_each(mv);
        self.lines.iter_mut().for_each(|(r, _)| mv(r));
        self.buttons.iter_mut().chain(&mut self.tabs).for_each(mv);
        let f = DIALOG_FRAME;
        let (w, h) = (self.size.0 - 2 * f, self.size.1 - 2 * f - CAPTION_H);
        let g = FrameGeometry::layout(&theme.metrics, w, h, false, chrome);
        self.with_frame(g, help)
    }

    fn with_frame(self, g: FrameGeometry, help: bool) -> DialogLayout {
        DialogLayout {
            size: g.total,
            caption: g.caption,
            close: Some(g.buttons[2]),
            help: help.then_some(g.help_box()),
            minimize: (g.chrome.buttons.has(CaptionButton::Minimize))
                .then(|| g.button_rect(CaptionButton::Minimize)),
            frame: Some(g),
            ..self
        }
    }

    /// A classic dialog on `template` (see [`dlgtemplate`]).
    fn compute_template(
        theme: &Theme,
        text: &TextRenderer,
        d: &Dialog,
        template: Template,
        text_h: Option<i32>,
    ) -> DialogLayout {
        let u = theme.text;
        let lay = dlgtemplate::layout(
            template,
            text,
            theme.fonts.size_px,
            &dlgtemplate::Fields {
                text: &d.text,
                icon: d.icon.is_some(),
                field: d.field.is_some(),
                buttons: d.buttons.len(),
                text_h,
            },
            u,
        )
        .grown(u);
        let mut body = DialogLayout {
            size: lay.size,
            icon: lay.icon,
            lines: lay.lines,
            field_label: lay.field_label,
            field: lay.field,
            buttons: lay.buttons,
            ..Default::default()
        }
        .framed(theme, d.help);
        if let (Some(fl), Some(box_r)) = (&d.field, body.field) {
            let cl = combo::layout(&fl.combo, box_r, &theme.look.controls().grown(u));
            body.drop = Some(cl.drop);
            body.list = Some(cl.list);
        }
        body
    }

    /// End Program on its fixed template: each `\n`-separated paragraph
    /// of the text wraps in the column and starts at its own height.
    fn compute_end_program(theme: &Theme, text: &TextRenderer, d: &Dialog) -> DialogLayout {
        let px = theme.fonts.size_px;
        let width = EP_W - EP_TEXT_RIGHT - EP_TEXT_X;
        // The column is laid out at its ink: the second dialog font's pen
        // sits a column left of it.
        let x = EP_TEXT_X - text.lead();
        let lines = d
            .text
            .split('\n')
            .zip(EP_PARA_Y)
            .flat_map(|(para, y)| {
                wrap(text, para, theme.text.len(width), px)
                    .into_iter()
                    .enumerate()
                    .map(move |(i, s)| (Rect::new(x, y + i as i32 * LINE_H, width, LINE_H), s))
            })
            .collect();
        let buttons = (0..d.buttons.len() as i32)
            .map(|i| {
                Rect::new(
                    EP_BUTTONS.0 + i * EP_BUTTON_PITCH,
                    EP_BUTTONS.1,
                    EP_BUTTON_W,
                    BUTTON_H,
                )
            })
            .collect();
        DialogLayout {
            size: (EP_W, EP_H),
            icon: Some(Rect::new(EP_ICON.0, EP_ICON.1, ICON_PX, ICON_PX)),
            lines,
            buttons,
            ..Default::default()
        }
    }

    /// Browse: the file dialog's `client` size in a resizable frame, so under default metrics
    /// 563x477 with the client at (4, 22); `page` is the client area.
    fn compute_browse(theme: &Theme, help: bool, client: (i32, i32)) -> DialogLayout {
        let (cw, ch) = client;
        let g = FrameGeometry::layout(&theme.metrics, cw, ch, false, chrome(true));
        DialogLayout {
            page: Some(g.client),
            ..Default::default()
        }
        .with_frame(g, help)
    }

    /// A control's client area `client` inside the fixed 3 px frame under
    /// the caption, as `page`: the tray's Customize dialog.
    fn compute_client(client: (i32, i32)) -> DialogLayout {
        let (cw, ch) = client;
        DialogLayout {
            size: (cw + 2 * DIALOG_FRAME, ch + DIALOG_FRAME + FORM_CLIENT.1),
            page: Some(Rect::new(FORM_CLIENT.0, FORM_CLIENT.1, cw, ch)),
            ..Default::default()
        }
    }

    fn compute_chooser() -> DialogLayout {
        let (cw, ch) = openwith::CLIENT;
        DialogLayout {
            size: (CHOOSER_W, CHOOSER_H),
            page: Some(Rect::new(FORM_CLIENT.0, FORM_CLIENT.1, cw, ch)),
            ..Default::default()
        }
    }

    /// The Welcome panel: icon buttons in a row of cells across the
    /// gradient, the rest (Cancel) as small classic buttons from the right
    /// of the bottom band.
    fn compute_welcome(d: &Dialog, u: TextUnits) -> DialogLayout {
        let b = WELCOME_BORDER;
        let inner_w = WELCOME_W - 2 * b;
        let caption = Rect::new(b, b, inner_w, WELCOME_TITLE_H);
        let icons = d.buttons.iter().filter(|x| x.icon.is_some()).count();
        let (cell_w, cells): (i32, &[i32]) = if icons == 2 {
            (WELCOME_TWO.0, &WELCOME_TWO.1)
        } else {
            (WELCOME_THREE.0, &WELCOME_THREE.1)
        };
        // Log Off and Turn off computer are the only panels; a cell past
        // the table would sit at the left edge.
        let cell_x = |i: usize| cells.get(i).copied().unwrap_or(b);
        let (cw, ch, cy) = WELCOME_CANCEL;
        let mut next_icon = 0;
        let mut next_plain = 0;
        let buttons: Vec<Rect> = d
            .buttons
            .iter()
            .map(|x| {
                if x.icon.is_some() {
                    let r = Rect::new(
                        cell_x(next_icon),
                        b + WELCOME_CELL_Y,
                        cell_w,
                        WELCOME_CELL_H,
                    );
                    next_icon += 1;
                    r
                } else {
                    let r = Rect::new(
                        WELCOME_W - b - 10 - cw - next_plain * (cw + BUTTON_GAP),
                        b + cy,
                        cw,
                        ch,
                    );
                    next_plain += 1;
                    r
                }
            })
            .collect();
        let buttons = buttons.into_iter().map(|r: Rect| u.rect(r)).collect();
        DialogLayout {
            size: u.size((WELCOME_W, WELCOME_H)),
            caption: u.rect(caption),
            buttons,
            ..Default::default()
        }
    }

    /// The Welcome panel under a look with glass (`GLASS_WELCOME_W` on):
    /// the icon buttons in a centred row, the rest (Cancel) from the
    /// right of the row under them.
    fn compute_welcome_glass(d: &Dialog) -> DialogLayout {
        let (x0, y0) = (DIALOG_FRAME, DIALOG_FRAME + CAPTION_H);
        let (cw, ch) = GLASS_WELCOME_CELL;
        let icons = d.buttons.iter().filter(|x| x.icon.is_some()).count() as i32;
        let row = icons * cw + (icons - 1).max(0) * GLASS_WELCOME_GAP;
        let left = x0 + (GLASS_WELCOME_W - row) / 2;
        let plain_y = y0 + GLASS_WELCOME_TOP + ch + GLASS_WELCOME_TOP;
        let mut next_icon = 0;
        let mut next_plain = 0;
        let buttons = d
            .buttons
            .iter()
            .map(|x| {
                if x.icon.is_some() {
                    let r = Rect::new(
                        left + next_icon * (cw + GLASS_WELCOME_GAP),
                        y0 + GLASS_WELCOME_TOP,
                        cw,
                        ch,
                    );
                    next_icon += 1;
                    r
                } else {
                    let r = Rect::new(
                        x0 + GLASS_WELCOME_W
                            - GLASS_WELCOME_MARGIN
                            - BUTTON_W
                            - next_plain * (BUTTON_W + BUTTON_GAP),
                        plain_y,
                        BUTTON_W,
                        BUTTON_H,
                    );
                    next_plain += 1;
                    r
                }
            })
            .collect();
        let client_h = plain_y - y0 + BUTTON_H + GLASS_WELCOME_MARGIN;
        DialogLayout {
            size: (
                GLASS_WELCOME_W + 2 * DIALOG_FRAME,
                client_h + 2 * DIALOG_FRAME + CAPTION_H,
            ),
            buttons,
            ..Default::default()
        }
    }

    /// A form dialog: the caption, the tab strip's rects when it has one,
    /// and its fixed size; the controls carry their own rects.
    fn compute_form(
        theme: &Theme,
        text: &TextRenderer,
        d: &Dialog,
        f: &FormDialog,
    ) -> DialogLayout {
        let px = theme.fonts.size_px;
        let (tw, th) = f.grown_size(theme.text);
        let (w, h) =
            FrameGeometry::client_size_for_total(&theme.metrics, tw, th, false, chrome(false));
        let g = FrameGeometry::layout(&theme.metrics, w, h, false, chrome(false));
        let (tabs, page) = match f.sheet() {
            Some(s) => {
                let l = s.layout(text, px);
                (l.tabs, Some(l.page))
            }
            None => (Vec::new(), None),
        };
        // The dialog's icon (a message box's question mark, Power Options'
        // plug) sits where the open page's "icon" picture is; a page
        // without one shows none.
        let icon = f.open_page().and_then(|p| p.rect("icon"));
        DialogLayout {
            icon,
            tabs,
            page,
            ..Default::default()
        }
        .with_frame(g, d.help)
    }

    pub fn scrollbar(&self, d: &Dialog) -> Option<Rect> {
        self.list?;
        let f = d.field.as_ref()?;
        combo::layout(&f.combo, self.field?, &self.controls).scrollbar
    }

    /// The edges a drag from (`x`, `y`) sizes, when that is on a sizable
    /// frame's border under the metrics `m` it was laid out with: the
    /// band and corners a window's frame has. None on a fixed frame.
    pub fn sizing_edges(&self, x: i32, y: i32, m: &Metrics) -> Option<Edges> {
        let g = self.frame.filter(|g| g.chrome.resizable)?;
        match g.hit(x, y, m) {
            reprisekit::frame::FrameHit::Resize(e) => Some(e),
            _ => None,
        }
    }

    /// What the dialog has at (`x`, `y`). The renderer and font size go in
    /// because a hit inside a form's edit field resolves to a byte index
    /// in its value, which takes measuring the text.
    pub fn hit(
        &self,
        d: &Dialog,
        x: i32,
        y: i32,
        text: &TextRenderer,
        px: u32,
    ) -> Option<DialogHit> {
        if self.close.is_some_and(|r| r.contains(x, y)) {
            return Some(DialogHit::Close);
        }
        if self.minimize.is_some_and(|r| r.contains(x, y)) {
            return Some(DialogHit::Minimize);
        }
        if self.help.is_some_and(|r| r.contains(x, y)) {
            return Some(DialogHit::Help);
        }
        // The caption, less the buttons already taken out of it above: the
        // grip a dialog is dragged by. Welcome panels are not draggable.
        if !matches!(d.style, Style::Welcome) && self.caption.contains(x, y) {
            return Some(DialogHit::Caption);
        }
        if let Some(f) = d.form() {
            use reprisekit::controls::propsheet::Hit;
            return match &f.body {
                FormBody::Sheet(s) => s.hit(x, y, text, px).and_then(|h| match h {
                    Hit::Tab(i) => Some(DialogHit::Tab(i)),
                    Hit::Control(h) => Some(DialogHit::Control(h)),
                    Hit::ListBar(_) | Hit::List | Hit::PageBar(_) | Hit::PageHBar(_) => None,
                }),
                FormBody::Plain(form) => form.hit(x, y, text, px).map(DialogHit::Control),
            };
        }
        if let (Some(p), Some(client)) = (d.picker(), self.page) {
            let hit = p.dlg.hit(text, px, client, x, y);
            return hit.map(DialogHit::Browse);
        }
        if let (Some(c), Some(client)) = (d.chooser(), self.page) {
            let lay = c.ow.layout(text, px, (client.x, client.y), &self.controls);
            return c.ow.hit(&lay, x, y).map(DialogHit::Chooser);
        }
        if let (Some(t), Some(client)) = (d.tray(), self.page) {
            let lay = t.layout(text, px, (client.x, client.y));
            return t.hit(&lay, x, y).map(DialogHit::Tray);
        }
        if let (Some(c), Some(client)) = (d.copy(), self.page) {
            // The control is laid out from its own origin, the client's corner.
            let (lx, ly) = (x - client.x, y - client.y);
            if c.cp.on_toggle(lx, ly, c.units) {
                return Some(DialogHit::CopyDetails);
            }
            return c.cp.form.hit(lx, ly, text, px).map(DialogHit::Control);
        }
        if let (Some(f), Some(fr)) = (d.field.as_ref(), self.field)
            && f.combo.open.is_some()
        {
            let lay = combo::layout(&f.combo, fr, &self.controls);
            if lay.list.contains(x, y) {
                return combo::hit(&f.combo, &lay, x, y).map(|h| match h {
                    combo::Hit::Row(i) => DialogHit::Row(i),
                    combo::Hit::Field | combo::Hit::Drop => DialogHit::Drop,
                    h => h.bar().map_or(DialogHit::Drop, DialogHit::ListBar),
                });
            }
        }
        if self.drop.is_some_and(|r| r.contains(x, y)) {
            return Some(DialogHit::Drop);
        }
        if let (Some(f), Some(r)) = (d.field.as_ref(), self.field.filter(|r| r.contains(x, y))) {
            let origin = combo::layout(&f.combo, r, &self.controls).text.x;
            return Some(DialogHit::Field(x - origin));
        }
        self.buttons
            .iter()
            .position(|r| r.contains(x, y))
            .map(DialogHit::Button)
    }
}

fn redraw_if(changed: bool) -> Outcome {
    if changed {
        Outcome::Redraw
    } else {
        Outcome::Nothing
    }
}

/// The Run box's sunken field, drop button and value. Its caret is the
/// only one a dialog blinks, so a blink repaints just this over the kept
/// canvas; the field fills its own background, so nothing blends twice.
pub fn draw_field(
    c: &mut Canvas,
    theme: &Theme,
    text: &TextRenderer,
    layout: &DialogLayout,
    d: &Dialog,
    caret: bool,
) {
    let px = theme.fonts.size_px;
    if let (Some(fr), Some(field)) = (layout.field, &d.field) {
        let focused = d.button_focus.is_none();
        let lay = combo::layout(&field.combo, fr, &layout.controls);
        let drop = d.field_lit.of(FieldPart::Drop);
        let lit = combo::ComboLit {
            frame: match drop {
                Interaction::Idle => d.field_lit.of(FieldPart::Edit),
                lit => lit,
            },
            drop,
        };
        combo::draw(
            c,
            text,
            &theme.paint(),
            &field.combo,
            &lay,
            focused,
            false,
            reprisekit::textedit::Caret::new(focused, caret),
            lit,
            px,
        );
    }
}

/// Render `d` at `theme.scale`; `layout` is the logical layout, `icon` the
/// dialog's 32 px (times scale) icon, `caption_icon` the same icon at the
/// theme's icon size for a styled caption, `button_icons` those of a
/// Welcome panel's buttons (parallel to `d.buttons`), `row_icons` the
/// chooser's row icons or a Properties sheet's Opens with icon (the
/// first), and `logo` the Welcome panel's logo.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    theme: &Theme,
    text: &TextRenderer,
    layout: &DialogLayout,
    icon: Option<&Canvas>,
    caption_icon: Option<&Canvas>,
    button_icons: &[Option<Canvas>],
    row_icons: &[Option<&Canvas>],
    logo: Option<&Canvas>,
    d: &Dialog,
    caret: bool,
    lit: Lit<usize>,
    cues: Cues,
    // Whether the dialog has the keyboard. An inactive one draws a grey
    // caption, no default ring and no focus rectangle.
    active: bool,
    preedit: Option<&reprisekit::textedit::Preedit>,
) -> Canvas {
    // Under a look with glass the panel has a frame and draws as a dialog.
    if let (Style::Welcome, None) = (&d.style, &layout.frame) {
        return draw_welcome(theme, text, layout, button_icons, logo, d, lit, cues);
    }
    let s = theme.scale.max(1);
    let cs = &theme.colors;
    let px = theme.fonts.size_px;
    let dots = cues.focus && active;
    let paint = Paint {
        active,
        access_keys: cues.keys,
        ..theme.paint()
    };
    let (w, h) = (layout.size.0 * s, layout.size.1 * s);
    let mut c = Canvas::new(w.max(1) as u32, h.max(1) as u32);
    c.set_preedit(preedit.cloned());
    if let Some(g) = layout.frame {
        let state = FrameState {
            title: &d.title,
            active,
            maximized: false,
            pressed: d
                .caption_box
                .lit()
                .and_then(|(b, how)| (how == Interaction::Pressed).then_some(b)),
            glow: reprisekit::frame::Glow::lit(d.caption_hot),
            icon: caption_icon,
            tabbed: false,
        };
        reprisekit::look::draw_dialog_frame(
            &mut c,
            theme,
            text,
            &g.scaled(s),
            &state,
            layout.help.map(|_| d.help_box.of(())),
        );
    }

    if let (Some(p), Some(client)) = (d.picker(), layout.page) {
        p.dlg.draw(&mut c, text, &paint, client, px);
    }
    if let Some(f) = d.form() {
        draw_form(&mut c, &paint, theme, text, f, dots, row_icons);
    }
    if let (Some(ch), Some(client)) = (d.chooser(), layout.page) {
        let lay = ch
            .ow
            .layout(text, px, (client.x, client.y), &layout.controls);
        ch.ow
            .draw(&mut c, text, &paint, &lay, icon, row_icons, cues, px);
    }
    if let (Some(t), Some(client)) = (d.tray(), layout.page) {
        let lay = t.layout(text, px, (client.x, client.y));
        t.draw(&mut c, text, &paint, &lay, cues, px);
    }
    if let (Some(r), Some(ic)) = (layout.icon, icon) {
        let r = r.scaled(s);
        c.blit_over(
            r.x + (r.width - ic.width as i32) / 2,
            r.y + (r.height - ic.height as i32) / 2,
            ic,
        );
    }
    if let (
        Some(r),
        Style::Classic {
            delete_badge: true, ..
        },
    ) = (layout.icon, &d.style)
    {
        let r = r.scaled(s);
        draw_delete_badge(&mut c, r.x, r.y, s);
    }
    if let (Some(r), Some(FormKind::Properties(b))) = (layout.icon, d.form().map(|f| &f.kind))
        && b.shortcut
    {
        let r = r.scaled(s);
        reprisekit::controls::iconview::draw_shortcut_arrow(&mut c, r.x, r.bottom(), s);
    }
    // The dialog's own text and buttons, in the font it is set in; the
    // frame and a form's pages above pick their own.
    let text = text.dialog(d.font());
    for (r, line) in &layout.lines {
        text.draw(
            &mut c,
            r.scaled(s),
            line,
            false,
            cs.button_text,
            Align::Left,
            px,
        );
    }
    if let (Some(lr), Some(field)) = (layout.field_label, &d.field) {
        draw_label(
            &mut c,
            text,
            lr.scaled(s),
            &paint.label(&field.label),
            false,
            cs.button_text,
            Align::Left,
            px,
            s,
        );
    }
    draw_field(&mut c, theme, text, layout, d, caret);
    for (i, (r, b)) in layout.buttons.iter().zip(&d.buttons).enumerate() {
        // With no field to hold the focus, the default button has it and
        // wears the dotted focus rectangle. With one, a button Tab moved
        // to has it and takes the ring from the default.
        let (ring, focus) = match (&d.field, d.button_focus) {
            (None, _) => (b.default, b.default && dots),
            (Some(_), Some(f)) => (f == i, f == i && dots),
            (Some(_), None) => (b.default, false),
        };
        if b.icon.is_some() {
            let icon = button_icons.get(i).and_then(|o| o.as_ref());
            let look = IconButton {
                default: ring && active,
                ptr: lit_of(lit, i),
                focus,
            };
            draw_icon_button(
                &mut c,
                theme,
                text,
                &paint,
                r.scaled(s),
                b,
                icon,
                look,
                cues,
            );
            continue;
        }
        draw_button(
            &mut c,
            text,
            &paint,
            r.scaled(s),
            &b.label,
            ring && active,
            lit_of(lit, i),
            b.enabled,
            focus,
            px,
        );
    }
    if let (Some(f), Some(l)) = (
        d.field.as_ref().filter(|f| f.combo.open.is_some()),
        layout.list,
    ) {
        draw_list(&mut c, &paint, text, f, l, layout.scrollbar(d), px);
    }
    c
}

/// The copy progress control of `c` into `client` of a dialog drawn at `scale`, from the unscaled
/// `theme`: the control draws on a canvas of the output's scale, underlining while `keys`.
#[allow(clippy::too_many_arguments)]
pub fn draw_copy(
    canvas: &mut Canvas,
    theme: &Theme,
    text: &TextRenderer,
    c: &CopyDialog,
    client: Rect,
    scale: i32,
    active: bool,
    keys: bool,
) {
    let mut page = Canvas::with_scale(client.width as u32, client.height as u32, scale);
    let paint = Paint {
        active,
        access_keys: keys,
        s: 1,
        ..theme.paint()
    };
    c.cp.draw(&mut page, text, &paint, theme.fonts.size_px);
    canvas.blit_canvas(client.x * scale, client.y * scale, &page.at_scale(1));
}

/// The copy progress control's nodes under `ROOT`, moved from its own origin to `client`'s.
pub fn access_copy(
    c: &CopyDialog,
    client: Rect,
    theme: &Theme,
    text: &TextRenderer,
    tree: &mut reprisekit::access::Tree,
) {
    use reprisekit::access::ROOT;
    let before: std::collections::HashSet<_> = tree.nodes().map(|(id, _)| id).collect();
    c.cp.access(tree, text, &theme.paint(), theme.fonts.size_px);
    let added: Vec<_> = tree
        .nodes()
        .map(|(id, _)| id)
        .filter(|id| *id != ROOT && !before.contains(id))
        .collect();
    let (dx, dy) = (f64::from(client.x), f64::from(client.y));
    for id in added {
        if let Some(n) = tree.get_mut(id)
            && let Some(b) = n.bounds()
        {
            n.set_bounds(reprisekit::access::accesskit::Rect::new(
                b.x0 + dx,
                b.y0 + dy,
                b.x1 + dx,
                b.y1 + dy,
            ));
        }
    }
}

/// `d`'s `access` part under the dialog node `ROOT`, in dialog
/// coordinates: its text, Run's labelled field, a form's tabs and open
/// page, and the buttons. Returns the node with the focus: the field, the
/// form's focused control, else the default button. The Browse dialog
/// and the Open With chooser add their own control's tree and return its
/// focus instead.
pub fn access(
    d: &Dialog,
    layout: &DialogLayout,
    text: &TextRenderer,
    px: u32,
    tree: &mut reprisekit::access::Tree,
) -> reprisekit::access::accesskit::NodeId {
    use reprisekit::access::{self as ax, ROOT};
    if let (Some(p), Some(client)) = (d.picker(), layout.page) {
        p.dlg.access(tree, client, text, px);
        return tree.focus();
    }
    if let (Some(ch), Some(client)) = (d.chooser(), layout.page) {
        let lay = ch
            .ow
            .layout(text, px, (client.x, client.y), &layout.controls);
        return ch.ow.access(tree, &lay);
    }
    if let (Some(t), Some(client)) = (d.tray(), layout.page) {
        let lay = t.layout(text, px, (client.x, client.y));
        return t.access(tree, &lay, text, px);
    }
    let mut focus = None;
    if let Some(f) = d.form() {
        focus = match &f.body {
            FormBody::Sheet(s) => s.access(tree, ROOT, text, px).1,
            FormBody::Plain(form) => form.access(tree, ROOT, text, px),
        };
    }
    if let Some((first, _)) = layout.lines.first() {
        let all = layout.lines.iter().fold(*first, |u, (r, _)| u.union(r));
        tree.push(
            ROOT,
            "text",
            ax::label(all, &d.text.replace('\n', " "), true),
        );
    }
    if let (Some(fr), Some(field)) = (layout.field, &d.field) {
        let label = layout
            .field_label
            .map(|lr| tree.push(ROOT, "field label", ax::label(lr, &field.label, true)));
        let lay = combo::layout(&field.combo, fr, &layout.controls);
        focus = Some(combo::access(
            &field.combo,
            &lay,
            text,
            px,
            tree,
            ROOT,
            "field",
        ));
        let combo_id = reprisekit::access::Tree::id_of(ROOT, "field");
        if let (Some(l), Some(n)) = (label, tree.get_mut(combo_id)) {
            n.push_labelled_by(l);
        }
    }
    let mut default = None;
    for (i, (r, b)) in layout.buttons.iter().zip(&d.buttons).enumerate() {
        let id = tree.push(
            ROOT,
            ("button", i),
            ax::button(*r, &b.label, b.enabled, b.default),
        );
        if b.default {
            default = Some(id);
        }
        if d.button_focus == Some(i) {
            focus = Some(id);
        }
    }
    focus.or(default).unwrap_or(ROOT)
}

/// The icons a Properties sheet's open page shows in its pictures, by
/// item id and icon name, which the host resolves at 32 px and hands to
/// `draw` as row icons from 1 on (row 0 is the Opens with program).
pub fn page_pictures(d: &Dialog) -> Vec<(&'static str, String, i32)> {
    use reprisekit::controls::filesheet::{
        CUSTOM_ICON, CUSTOM_PREVIEW, PAGE_ICONS, PREVIEW_PX, STANDARD_FOLDER_ICON,
    };
    let Some(f) = d.form() else {
        return Vec::new();
    };
    let folder = match &f.kind {
        FormKind::Properties(b) => b.icon_chosen.clone().unwrap_or_else(|| b.own_icon.clone()),
        FormKind::SetProperties(s) => s.icon_chosen.clone().flatten(),
        _ => return Vec::new(),
    }
    .unwrap_or_else(|| STANDARD_FOLDER_ICON.to_string());
    let Some(page) = f.open_page() else {
        return Vec::new();
    };
    PAGE_ICONS
        .into_iter()
        .map(|(id, n)| (id, n.to_string(), ICON_PX))
        .chain([
            (CUSTOM_ICON, folder.clone(), ICON_PX),
            (CUSTOM_PREVIEW, folder, PREVIEW_PX),
        ])
        .filter(|(id, _, _)| page.item(id).is_some())
        .collect()
}

/// A form dialog: a sheet through `propsheet::draw`, or the plain form.
/// A Properties sheet's Opens with icon (row 0 of `row_icons`, from the
/// host's cache) and its page's other pictures (`page_pictures`, from
/// row 1 on) are filled in here, since the pages are built without them.
/// `p` is the dialog's paint.
fn draw_form(
    c: &mut Canvas,
    p: &Paint,
    theme: &Theme,
    text: &TextRenderer,
    f: &FormDialog,
    cues: bool,
    row_icons: &[Option<&Canvas>],
) {
    let opens_icon = row_icons.first().copied().flatten();
    let s = theme.scale.max(1);
    let px = theme.fonts.size_px;
    match &f.body {
        FormBody::Sheet(sheet) => {
            reprisekit::controls::propsheet::draw(c, text, p, sheet, cues, px)
        }
        FormBody::Plain(form) => reprisekit::controls::form::draw(c, text, p, form, cues, px),
    }
    let Some(form) = f.open_page() else {
        return;
    };
    if let (FormKind::Properties(_), Some(r), Some(ic)) =
        (&f.kind, form.rect("opens_icon"), opens_icon)
    {
        c.blit_over(r.x * s, r.y * s, ic);
    }
    if matches!(f.kind, FormKind::Properties(_) | FormKind::SetProperties(_)) {
        // In `page_pictures` order, which the host resolved from row 1 on.
        let pictures = [
            reprisekit::controls::filesheet::PAGE_ICONS[0].0,
            reprisekit::controls::filesheet::PAGE_ICONS[1].0,
            reprisekit::controls::filesheet::CUSTOM_ICON,
            reprisekit::controls::filesheet::CUSTOM_PREVIEW,
        ];
        let rects = pictures.into_iter().filter_map(|id| form.rect(id));
        for (r, ic) in rects.zip(row_icons.iter().skip(1)) {
            if let Some(ic) = ic {
                let (w, h) = (ic.width as i32, ic.height as i32);
                let r = r.scaled(s);
                c.blit_over(
                    r.x + (r.width - w).max(0) / 2,
                    r.y + (r.height - h).max(0) / 2,
                    ic,
                );
            }
        }
    }
}

/// The red exclamation badge on the Confirm Delete icon: a 12 px disc
/// with a white exclamation mark over the bottom-left corner of the 32 px
/// icon at (`x`, `y`).
pub fn draw_delete_badge(c: &mut Canvas, x: i32, y: i32, s: i32) {
    reprisekit::art::icons::DELETE_BADGE.paint(c, x + 3 * s, y + 18 * s, s);
}

fn draw_list(
    c: &mut Canvas,
    p: &Paint,
    text: &TextRenderer,
    f: &Field,
    l: Rect,
    scrollbar: Option<Rect>,
    px: u32,
) {
    combo::draw_list(
        c,
        text,
        p,
        &f.combo.items,
        f.combo.open,
        f.combo.top,
        f.combo.rows,
        l,
        scrollbar,
        None,
        px,
    );
}

/// The Welcome panel's blue colours, a drawing constant rather than
/// theme colours: the panel keeps them whatever the classic scheme says.
const PANEL_BAND: Color = Color::rgb(0x00, 0x33, 0x99);
const PANEL_SEPARATOR: [Color; 21] = [
    Color::rgb(0x30, 0x53, 0xb5),
    Color::rgb(0x3c, 0x64, 0xb6),
    Color::rgb(0x4a, 0x6f, 0xba),
    Color::rgb(0x55, 0x79, 0xc0),
    Color::rgb(0x60, 0x83, 0xc5),
    Color::rgb(0x6f, 0x8f, 0xcb),
    Color::rgb(0x80, 0x9d, 0xd2),
    Color::rgb(0x90, 0xaa, 0xd9),
    Color::rgb(0x9d, 0xb5, 0xdf),
    Color::rgb(0xa5, 0xbc, 0xe3),
    Color::rgb(0xad, 0xc2, 0xe6),
    Color::rgb(0xb5, 0xc9, 0xea),
    Color::rgb(0xbc, 0xcf, 0xed),
    Color::rgb(0xc0, 0xd2, 0xef),
    Color::rgb(0xc4, 0xd5, 0xf0),
    Color::rgb(0xc7, 0xd8, 0xf2),
    Color::rgb(0xcb, 0xdc, 0xf4),
    Color::rgb(0xcf, 0xdf, 0xf6),
    Color::rgb(0xd2, 0xe1, 0xf7),
    Color::rgb(0xd5, 0xe4, 0xf8),
    Color::rgb(0xd8, 0xe6, 0xfa),
];
/// The separator's two rows under the title band, left to right across
/// its 312 columns, as runs of (`PANEL_SEPARATOR` index, columns). The
/// lightest run sits left of centre; the second row is the first with
/// the steps dithered a column either way.
const SEPARATOR_RUNS: [&[(u8, u8)]; 2] = [
    &[
        (1, 9),
        (2, 8),
        (3, 6),
        (4, 7),
        (5, 9),
        (6, 9),
        (7, 9),
        (8, 7),
        (9, 5),
        (10, 4),
        (11, 6),
        (12, 5),
        (13, 2),
        (14, 4),
        (15, 3),
        (16, 6),
        (17, 4),
        (18, 3),
        (19, 7),
        (20, 41),
        (19, 19),
        (18, 11),
        (17, 12),
        (16, 16),
        (15, 5),
        (16, 3),
        (15, 3),
        (14, 4),
        (13, 2),
        (12, 5),
        (11, 6),
        (10, 4),
        (9, 5),
        (8, 7),
        (7, 9),
        (6, 9),
        (5, 9),
        (4, 7),
        (3, 6),
        (2, 8),
        (1, 8),
    ],
    &[
        (1, 1),
        (0, 1),
        (1, 6),
        (2, 1),
        (1, 2),
        (2, 4),
        (3, 1),
        (2, 1),
        (3, 4),
        (4, 1),
        (3, 2),
        (4, 6),
        (5, 1),
        (4, 1),
        (5, 5),
        (6, 1),
        (5, 1),
        (6, 1),
        (5, 1),
        (6, 6),
        (7, 1),
        (6, 2),
        (7, 6),
        (8, 1),
        (7, 1),
        (8, 4),
        (9, 1),
        (8, 1),
        (9, 5),
        (10, 5),
        (11, 4),
        (12, 1),
        (11, 1),
        (12, 4),
        (13, 2),
        (14, 4),
        (15, 4),
        (16, 3),
        (17, 1),
        (16, 1),
        (17, 4),
        (18, 3),
        (19, 7),
        (20, 41),
        (19, 19),
        (18, 11),
        (17, 12),
        (16, 1),
        (17, 1),
        (16, 10),
        (15, 1),
        (16, 1),
        (15, 1),
        (16, 1),
        (15, 5),
        (16, 2),
        (15, 4),
        (14, 4),
        (13, 2),
        (12, 4),
        (11, 1),
        (12, 1),
        (11, 4),
        (10, 5),
        (9, 5),
        (8, 1),
        (9, 1),
        (8, 4),
        (7, 1),
        (8, 1),
        (7, 6),
        (6, 2),
        (7, 1),
        (6, 6),
        (5, 1),
        (6, 1),
        (5, 1),
        (6, 1),
        (5, 5),
        (4, 1),
        (5, 1),
        (4, 6),
        (3, 2),
        (4, 1),
        (3, 4),
        (2, 1),
        (3, 1),
        (2, 4),
        (1, 2),
        (2, 1),
        (1, 6),
        (0, 1),
    ],
];
/// The two rows under the gradient, left to right as runs of (colour,
/// columns): a line lighter under the middle, then the bottom band's
/// first row, a shade redder under the middle.
const PANEL_LINE_RUNS: [[(Color, i32); 3]; 2] = [
    [
        (Color::rgb(0x0f, 0x40, 0xa4), 87),
        (Color::rgb(0x14, 0x44, 0xa6), 191),
        (Color::rgb(0x0f, 0x40, 0xa4), 34),
    ],
    [
        (PANEL_BAND, 121),
        (Color::rgb(0x01, 0x33, 0x99), 125),
        (PANEL_BAND, 66),
    ],
];
const PANEL_LEFT: Color = Color::rgb(0x5a, 0x7d, 0xde);
const PANEL_PEAK: Color = Color::rgb(0x9b, 0xb9, 0xf7);
const PANEL_DISABLED: Color = Color::rgb(0xa0, 0xa0, 0xa0);
/// The gradient's light across its 312 columns and down its 110 rows,
/// as (offset, per mille) stops joined by straight lines. A pixel's
/// colour runs from `PANEL_LEFT` to `PANEL_PEAK` by the product of its
/// column's and its row's light: flat at the left edge, lightest on
/// column 183, a fifth of the way at the right edge; lightest across
/// rows 36-48, darkening towards the bottom, the last row darker again.
const PANEL_ACROSS: [(i32, i32); 12] = [
    (0, 0),
    (9, 0),
    (20, 49),
    (45, 167),
    (100, 562),
    (153, 916),
    (170, 977),
    (183, 1000),
    (196, 970),
    (210, 926),
    (260, 599),
    (311, 224),
];
const PANEL_DOWN: [(i32, i32); 8] = [
    (0, 933),
    (18, 970),
    (36, 994),
    (48, 992),
    (70, 946),
    (90, 884),
    (108, 817),
    (109, 706),
];

/// The per-mille value at `at` along `stops`, joined by straight lines
/// and held past the last stop.
fn stop_value(stops: &[(i32, i32)], at: i32) -> i32 {
    for w in stops.windows(2) {
        let ((x0, y0), (x1, y1)) = (w[0], w[1]);
        if at <= x1 {
            return y0 + ((y1 - y0) * (at - x0)).div_euclid(x1 - x0);
        }
    }
    stops.last().map_or(0, |s| s.1)
}

/// The gradient's colour at (`x`, `y`), unscaled panel px.
fn panel_gradient(x: i32, y: i32) -> Color {
    let t = stop_value(&PANEL_ACROSS, x) * stop_value(&PANEL_DOWN, y);
    let mix = |a: u8, b: u8| (a as i32 + ((b as i32 - a as i32) * t + 500_000) / 1_000_000) as u8;
    Color::rgb(
        mix(PANEL_LEFT.r, PANEL_PEAK.r),
        mix(PANEL_LEFT.g, PANEL_PEAK.g),
        mix(PANEL_LEFT.b, PANEL_PEAK.b),
    )
}

/// The Welcome title's face: the first family at 20 px (capitals 14 rows
/// tall), else 18 px, falling back as the Blue Start label does. `bold`
/// takes each family's heaviest face.
fn welcome_title_text() -> TextSpec {
    let white = Color::rgb(0xFF, 0xFF, 0xFF);
    TextSpec {
        family: vec![
            "Franklin Gothic Medium".into(),
            "Trebuchet MS".into(),
            "DejaVu Sans".into(),
        ],
        size_px: 18,
        family_sizes: vec![("Franklin Gothic Medium".into(), 20)],
        bold: true,
        italic: false,
        antialias: true,
        full_hinting: true,
        color: white,
        inactive: white,
        shadow: None,
        ink_gamma: 1.0,
    }
}

thread_local! {
    static WELCOME_TITLE: TextRenderer = TextRenderer::load(&welcome_title_text().font());
}

/// The Welcome panel: black border, title band with the logo, the icon
/// buttons on the gradient (`panel_gradient`), Cancel in the bottom
/// band. `button_icons` is parallel to `d.buttons`. The focus
/// sits on the default button: an icon button shows it by drawing its
/// icon hot, as the one `lit` names under the pointer does, and Cancel
/// by its black default frame, with the dotted rectangle while `cues`
/// shows it. The labels underline their access keys only while `cues`
/// shows them.
#[allow(clippy::too_many_arguments)]
fn draw_welcome(
    theme: &Theme,
    text: &TextRenderer,
    layout: &DialogLayout,
    button_icons: &[Option<Canvas>],
    logo: Option<&Canvas>,
    d: &Dialog,
    lit: Lit<usize>,
    cues: Cues,
) -> Canvas {
    let s = theme.scale.max(1);
    let px = theme.fonts.size_px;
    let white = Color::rgb(255, 255, 255);
    let (w, h) = (layout.size.0 * s, layout.size.1 * s);
    let mut c = Canvas::new(w.max(1) as u32, h.max(1) as u32);
    c.fill_rect(Rect::new(0, 0, w, h), Color::rgb(0, 0, 0));
    let b = WELCOME_BORDER * s;
    let inner = Rect::new(b, b, w - 2 * b, h - 2 * b);
    c.fill_rect(inner, PANEL_BAND);
    // The bands are drawn for 312 columns and a 110-row gradient at 100%; grown with the text
    // they stretch, each logical px taking the colour of the 100% one it maps back to.
    let u = theme.text;
    let caption = layout.caption;
    let title = caption.scaled(s);
    let band_top = caption.bottom() + 2;
    let band = Rect::new(caption.x, band_top, caption.width, u.len(156) - band_top);
    let across = |n: i32| n * band.width / PANEL_W;
    let runs_row = |c: &mut Canvas, y: i32, runs: &mut dyn Iterator<Item = (Color, i32)>| {
        let mut at = 0;
        for (ink, n) in runs {
            let (x0, x1) = (across(at), across(at + n));
            c.fill_rect(Rect::new(band.x + x0, y, x1 - x0, 1).scaled(s), ink);
            at += n;
        }
    };
    for (row, runs) in SEPARATOR_RUNS.iter().enumerate() {
        let mut inks = runs
            .iter()
            .map(|&(i, n)| (PANEL_SEPARATOR[i as usize], n as i32));
        runs_row(&mut c, caption.bottom() + row as i32, &mut inks);
    }
    for y in 0..band.height {
        for x in 0..band.width {
            let at = Rect::new(band.x + x, band.y + y, 1, 1).scaled(s);
            let ink = panel_gradient(x * PANEL_W / band.width, y * PANEL_H / band.height);
            c.fill_rect(at, ink);
        }
    }
    for (row, runs) in PANEL_LINE_RUNS.iter().enumerate() {
        runs_row(
            &mut c,
            band.bottom() + row as i32,
            &mut runs.iter().copied(),
        );
    }
    let spec = welcome_title_text();
    WELCOME_TITLE.with(|t| {
        t.draw(
            &mut c,
            Rect::new(
                title.x + 12 * s,
                title.y,
                title.width - 60 * s,
                title.height,
            ),
            &d.title,
            spec.bold,
            spec.color,
            Align::Left,
            u.px(spec.size_in(t)) * s as u32,
        )
    });
    if let Some(logo) = logo {
        let dy = 6 + (caption.height - WELCOME_TITLE_H) / 2;
        c.blit_over(title.right() - 43 * s, title.y + dy * s, logo);
    }
    let mut fallback: Option<Canvas> = None;
    for (i, (r, btn)) in layout.buttons.iter().zip(&d.buttons).enumerate() {
        let r = r.scaled(s);
        if btn.icon.is_some() {
            let icon_px = ICON_PX * s;
            let (icon_y, label) =
                welcome_cell(r, WELCOME_CELL_H, WELCOME_CELL_H - WELCOME_LABEL_Y, u, s);
            let at = (r.x + (r.width - icon_px) / 2, icon_y);
            let icon = match button_icons.get(i).and_then(|o| o.as_ref()) {
                Some(icon) => icon,
                None => {
                    &*fallback.get_or_insert_with(|| reprisekit::icon::fallback(theme, icon_px))
                }
            };
            let hot = lit.is_some_and(|(j, _)| j == i) || btn.default;
            if btn.enabled && hot {
                c.blit_over(at.0, at.1, &reprisekit::icon::hot(icon));
            } else if btn.enabled {
                c.blit_over(at.0, at.1, icon);
            } else {
                let mut faded = icon.clone();
                faded.fade(1, 3);
                c.blit_over(at.0, at.1, &faded);
            }
            let color = if btn.enabled { white } else { PANEL_DISABLED };
            if cues.keys {
                draw_label(
                    &mut c,
                    text,
                    label,
                    &btn.label,
                    true,
                    color,
                    Align::Center,
                    px,
                    s,
                );
            } else {
                let plain = split_mnemonic(&btn.label).0;
                text.draw(&mut c, label, &plain, true, color, Align::Center, px);
            }
        } else {
            draw_button(
                &mut c,
                text,
                &Paint {
                    access_keys: cues.keys,
                    ..theme.paint()
                },
                r,
                &btn.label,
                btn.default,
                Interaction::Idle,
                true,
                btn.default && cues.focus,
                px,
            );
        }
    }
    c
}

/// A Welcome icon cell `r` (scaled) laid out `cell_h` tall at 100% with a `label_h` label line
/// `WELCOME_LABEL_Y` down: the icon's top and the label's box at the text size `u`. The label
/// keeps its 100% gap to the cell's bottom, grown; the icon keeps its px and moves down half
/// as far as the label did.
fn welcome_cell(r: Rect, cell_h: i32, label_h: i32, u: TextUnits, s: i32) -> (i32, Rect) {
    let h = r.height / s;
    let label_y = (h - u.len(cell_h - WELCOME_LABEL_Y)).max(WELCOME_LABEL_Y);
    let icon_y = WELCOME_ICON_Y + (label_y - WELCOME_LABEL_Y) / 2;
    let label = Rect::new(r.x, r.y + label_y * s, r.width, u.len(label_h) * s);
    (r.y + icon_y * s, label)
}

/// How a framed Welcome panel's icon button shows: wearing the default
/// ring, what the pointer does to it, and the dotted focus rectangle.
#[derive(Clone, Copy)]
struct IconButton {
    default: bool,
    ptr: Interaction,
    focus: bool,
}

/// A framed Welcome panel's icon button `b` across `r` (scaled): a push
/// button with `icon` (the fallback glyph when None) `WELCOME_ICON_Y`
/// into it, faded when greyed, and the label `WELCOME_LABEL_Y` in,
/// centred, its access key underlined while `cues` shows the keys.
#[allow(clippy::too_many_arguments)]
fn draw_icon_button(
    c: &mut Canvas,
    theme: &Theme,
    text: &TextRenderer,
    p: &Paint,
    r: Rect,
    b: &DialogButton,
    icon: Option<&Canvas>,
    look: IconButton,
    cues: Cues,
) {
    let s = p.s;
    let px = theme.fonts.size_px;
    let icon_px = ICON_PX * s;
    draw_button(
        c,
        text,
        p,
        r,
        "",
        look.default,
        look.ptr,
        b.enabled,
        look.focus,
        px,
    );
    let fallback;
    let icon = match icon {
        Some(icon) => icon,
        None => {
            fallback = reprisekit::icon::fallback(theme, icon_px);
            &fallback
        }
    };
    let cell_h = GLASS_WELCOME_CELL.1;
    let (icon_y, label) = welcome_cell(r, cell_h, cell_h - WELCOME_LABEL_Y - 4, theme.text, s);
    let at = (r.x + (r.width - icon_px) / 2, icon_y);
    if b.enabled {
        c.blit_over(at.0, at.1, icon);
    } else {
        let mut faded = icon.clone();
        faded.fade(1, 3);
        c.blit_over(at.0, at.1, &faded);
    }
    let ink = if b.enabled {
        p.cs.button_text
    } else {
        p.cs.gray_text
    };
    if cues.keys {
        draw_label(c, text, label, &b.label, false, ink, Align::Center, px, s);
    } else {
        let plain = split_mnemonic(&b.label).0;
        text.draw(c, label, &plain, false, ink, Align::Center, px);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reprisefs::bincap::BinLack;
    use reprisekit::controls::filesheet::Subject;
    use reprisekit::controls::grouplist;
    use reprisekit::controls::openwith::{Check, Program};

    #[test]
    fn delete_badge_pins_at_scale_1_and_2() {
        for (s, want) in [(1, 0x056e_b456_7847_bf11u64), (2, 0x5abe_c64f_f838_2a75)] {
            let mut c = Canvas::new(32 * s as u32, 32 * s as u32);
            c.fill_rect(c.bounds(), Color::rgb(128, 128, 128));
            draw_delete_badge(&mut c, 0, 0, s);
            let mut h = 0xcbf2_9ce4_8422_2325u64;
            for b in c.pixels.iter().flat_map(|p| p.to_le_bytes()) {
                h = (h ^ b as u64).wrapping_mul(0x100_0000_01b3);
            }
            assert_eq!(h, want, "the delete badge at scale {s}");
        }
    }

    #[test]
    fn run_opens_by_the_start_button() {
        use reprisekit::taskbar::Edge;
        let area = Rect::new(0, 0, 1920, 1050);
        assert_eq!(by_start(area, Edge::Bottom, (347, 179)), (0, 1050 - 179));
        let area = Rect::new(0, 30, 1920, 1050);
        assert_eq!(by_start(area, Edge::Top, (347, 179)), (0, 30));
        let area = Rect::new(60, 0, 1860, 1080);
        assert_eq!(
            by_start(area, Edge::Left, (347, 179)),
            (60, 0),
            "Start tops a left bar"
        );
        let area = Rect::new(0, 0, 1860, 1080);
        assert_eq!(by_start(area, Edge::Right, (347, 179)), (1860 - 347, 0));
    }

    #[test]
    fn run_beside_a_right_bar_stays_on_a_narrow_output() {
        use reprisekit::taskbar::Edge;
        let area = Rect::new(1920, 0, 260, 1080);
        assert_eq!(by_start(area, Edge::Right, (347, 179)), (1920, 0));
    }

    #[test]
    fn runs_browse_keeps_3_px_off_the_screens_edge_on_starts_side() {
        use reprisekit::taskbar::Edge;
        let area = Rect::new(0, 0, 1920, 1050);
        assert_eq!(browse_by_start(area, Edge::Bottom, (400, 300)), (3, 750));
        let area = Rect::new(0, 0, 1860, 1080);
        assert_eq!(
            browse_by_start(area, Edge::Right, (400, 300)),
            (1860 - 400 - 3, 0)
        );
    }

    #[test]
    fn identity_tells_same_named_files_apart() {
        let mut a = file_info();
        a.path = "/one/x.txt".into();
        let mut b = file_info();
        b.path = "/two/x.txt".into();
        let a = Dialog::properties(&a, "text-x-generic", (0, 0));
        let b = Dialog::properties(&b, "text-x-generic", (0, 0));
        assert_eq!(a.title, b.title);
        assert_ne!(a.identity(), b.identity());
        assert_eq!(
            Dialog::run(&history()).identity(),
            Dialog::run(&history()).identity()
        );
    }
    use crate::theme::FontConfig;

    /// A renderer for tests that pin exact pixels, which hold only for the
    /// default UI face; None (the test skips) without it.
    fn tahoma() -> Option<TextRenderer> {
        let text = TextRenderer::load(&FontConfig::default());
        reprisekit::text::test_tahoma().then_some(text)
    }

    fn power() -> PowerConfig {
        PowerConfig {
            standby: "systemctl suspend".into(),
            poweroff: "systemctl poweroff".into(),
            reboot: "systemctl reboot".into(),
            ..PowerConfig::default()
        }
    }

    /// A command longer than the box scrolls with the caret; `draw` keeps `Field::first` in step.
    #[test]
    fn a_long_run_command_scrolls_instead_of_overflowing_the_field() {
        let theme = Theme::default();
        let Some(text) = tahoma() else { return };
        let px = theme.fonts.size_px;
        let mut d = Dialog::run(&[]);
        let long = "/usr/local/bin/some-very-long-command --with --plenty --of --flags";
        d.key(Key::Edit(EditKey::Insert(long.into())));
        let l = DialogLayout::compute(&theme, &text, &d);
        let _ = draw(
            &theme,
            &text,
            &l,
            None,
            None,
            &[],
            &[],
            None,
            &d,
            true,
            None,
            Cues::ALL,
            true,
            None,
        );

        let f = d.field.as_ref().unwrap();
        let from = f.combo.first.get();
        assert!(from > 0, "the value scrolled to follow the caret");
        let field = l.field.unwrap();
        let drop = l.drop.unwrap();
        let inner = Rect::new(
            field.x + combo::EDIT_TEXT_X,
            field.y,
            drop.x - 2 - (field.x + combo::EDIT_TEXT_X),
            field.height,
        );
        assert!(text.measure(&f.edit().unwrap().text[from..], false, px) <= inner.width);
        assert!(
            text.measure(
                &f.edit().unwrap().text[from..f.edit().unwrap().caret()],
                false,
                px
            ) <= inner.width
        );

        d.key(Key::Edit(crate::textedit::EditKey::Home { select: false }));
        let _ = draw(
            &theme,
            &text,
            &l,
            None,
            None,
            &[],
            &[],
            None,
            &d,
            true,
            None,
            Cues::ALL,
            true,
            None,
        );
        assert_eq!(
            d.field.as_ref().unwrap().combo.first.get(),
            0,
            "the caret at the start shows the start of the command"
        );
    }

    #[test]
    fn run_drop_button_has_the_combo_bevel() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let Some(text) = tahoma() else { return };
        let d = Dialog::run(&history());
        let l = DialogLayout::compute(&theme, &text, &d);
        let c = draw(
            &theme,
            &text,
            &l,
            None,
            None,
            &[],
            &[],
            None,
            &d,
            false,
            None,
            Cues::NONE,
            true,
            None,
        );
        let r = l.drop.expect("a history draws the drop button");
        assert_eq!(c.get(r.x, r.y), cs.button_face.argb(), "outer top-left");
        assert_eq!(
            c.get(r.x + 1, r.y + 1),
            cs.button_hilight.argb(),
            "inner top-left"
        );
        let (x1, y1) = (r.right() - 1, r.bottom() - 1);
        assert_eq!(
            c.get(x1, y1),
            cs.button_dk_shadow.argb(),
            "outer bottom-right"
        );
        assert_eq!(
            c.get(x1 - 1, y1 - 1),
            cs.button_shadow.argb(),
            "inner bottom-right"
        );
    }

    #[test]
    fn run_highlights_its_command_13_rows_tall_from_6_px_in() {
        let theme = Theme::default();
        let Some(text) = tahoma() else { return };
        let d = Dialog::run(&history());
        let l = DialogLayout::compute(&theme, &text, &d);
        let c = draw(
            &theme,
            &text,
            &l,
            None,
            None,
            &[],
            &[],
            None,
            &d,
            true,
            None,
            Cues::NONE,
            true,
            None,
        );
        let f = l.field.unwrap();
        let hl = theme.colors.hilight.argb();
        let row = f.y + 3;
        let first = (f.x..f.right()).find(|&x| c.get(x, row) == hl);
        assert_eq!(first, Some(f.x + 6), "the highlight's first column");
        let rows: Vec<i32> = (f.y..f.bottom())
            .filter(|&y| (f.x..f.right()).any(|x| c.get(x, y) == hl))
            .collect();
        assert_eq!(rows, (f.y + 3..f.y + 16).collect::<Vec<_>>(), "13 rows");
        assert_eq!(
            l.hit(&d, f.x + 10, f.y + 5, &text, theme.fonts.size_px),
            Some(DialogHit::Field(4))
        );
    }

    #[test]
    fn run_dialog_edits_and_runs_the_field() {
        let mut d = Dialog::run(&[]);
        assert_eq!(
            d.key(Key::Edit(EditKey::Insert("b".into()))),
            Outcome::Redraw
        );
        assert_eq!(d.press(2), Outcome::BrowseRun);
        assert_eq!(d.set_field("'/usr/bin/foot'"), Outcome::Redraw);
        assert_eq!(d.key(Key::Enter), Outcome::Run("'/usr/bin/foot'".into()));
        let mut d = Dialog::run(&[]);
        assert_eq!(
            d.key(Key::Enter),
            Outcome::Nothing,
            "empty field stays open"
        );
        assert_eq!(
            d.key(Key::Edit(EditKey::Insert("xt".into()))),
            Outcome::Redraw
        );
        assert_eq!(
            d.key(Key::Edit(EditKey::Insert("erm".into()))),
            Outcome::Redraw
        );
        assert_eq!(d.key(Key::Backspace), Outcome::Redraw);
        assert_eq!(d.key(Key::Backspace), Outcome::Redraw);
        assert_eq!(
            d.key(Key::Edit(EditKey::Insert("rm ".into()))),
            Outcome::Redraw
        );
        assert_eq!(d.key(Key::Enter), Outcome::Run("xterm".into()));
        assert_eq!(d.press(1), Outcome::Close);
        assert_eq!(d.key(Key::Escape), Outcome::Close);
        let mut empty = Dialog::run(&[]);
        assert_eq!(empty.key(Key::Backspace), Outcome::Nothing);
    }

    #[test]
    fn the_wheel_scrolls_run_s_open_list_wherever_the_pointer_is() {
        let mut d = Dialog::run(&history());
        assert_eq!(d.wheel(None, 3), Outcome::Nothing, "the list is closed");
        d.click(Some(DialogHit::Drop));
        let top = |d: &Dialog| d.field.as_ref().unwrap().combo.top;
        assert_eq!(d.wheel(Some(DialogHit::Button(0)), 3), Outcome::Redraw);
        assert_eq!(top(&d), 2, "five commands, three rows: the last two");
        assert_eq!(d.wheel(None, 3), Outcome::Nothing, "at the bottom");
        assert_eq!(d.wheel(None, -3), Outcome::Redraw);
        assert_eq!(top(&d), 0);
        assert_eq!(d.field.as_ref().unwrap().combo.text(), "notepad");
    }

    #[test]
    fn the_wheel_scrolls_a_form_dialog_s_open_combo() {
        use reprisekit::controls::form::{Control, Form, Item};
        let items = ["a", "b", "c", "d", "e", "f"];
        let form = Form::new(vec![Item::combo("c", 10, 10, 120, &items, 3)]);
        let fd = FormDialog::plain((200, 100), form, FormKind::ChangeIcon);
        let mut d = Dialog::new("Form", Style::Form(Box::new(fd)), Placement::Center);
        assert_eq!(d.wheel(None, 3), Outcome::Nothing, "nothing to scroll");
        let page = &mut d.form_mut().unwrap().pages_mut()[0];
        let Control::Combo(c) = &mut page.items[0].control else {
            unreachable!()
        };
        c.toggle();
        assert_eq!(d.wheel(None, 3), Outcome::Redraw);
        let Control::Combo(c) = &d.form().unwrap().pages()[0].items[0].control else {
            unreachable!()
        };
        assert_eq!(c.top, 3);
    }

    #[test]
    fn a_readers_focus_on_a_button_moves_the_keyboard_without_pressing_it() {
        let Some(text) = tahoma() else { return };
        let theme = Theme::default();
        let px = theme.fonts.size_px;
        let middle = |r: Rect| (r.x + r.width / 2, r.y + r.height / 2);
        let mut run = Dialog::run(&history());
        let l = DialogLayout::compute(&theme, &text, &run);
        let (x, y) = middle(l.buttons[1]);
        assert_eq!(run.focus_at(&l, x, y, &text, px), Outcome::Redraw);
        assert_eq!(run.button_focus, Some(1), "Cancel wears the focus");
        assert_eq!(run.focus_at(&l, x, y, &text, px), Outcome::Nothing);
        assert_eq!(run.key(Key::Tab { back: false }), Outcome::Redraw);
        assert_eq!(run.button_focus, Some(2), "Tab goes on from Cancel");

        let path = std::path::Path::new("/srv/a.txt");
        let mut ask = Dialog::confirm_delete("a.txt", path, false, false, false);
        let l = DialogLayout::compute(&theme, &text, &ask);
        let no = ask.buttons.iter().position(|b| !b.default).unwrap();
        let (x, y) = middle(l.buttons[no]);
        assert_eq!(ask.focus_at(&l, x, y, &text, px), Outcome::Redraw);
        let defaults: Vec<bool> = ask.buttons.iter().map(|b| b.default).collect();
        assert!(defaults[no] && defaults.iter().filter(|&&d| d).count() == 1);
    }

    #[test]
    fn run_s_tab_walks_the_field_and_its_buttons() {
        let mut d = Dialog::run(&history());
        let tab = |d: &mut Dialog, back| d.key(Key::Tab { back });
        assert_eq!(d.button_focus, None, "Run opens on the field");
        assert_eq!(tab(&mut d, false), Outcome::Redraw);
        assert_eq!(d.button_focus, Some(0), "Tab moves to OK");
        tab(&mut d, false);
        assert_eq!(d.button_focus, Some(1), "then Cancel");
        tab(&mut d, false);
        assert_eq!(d.button_focus, Some(2), "then Browse...");
        tab(&mut d, false);
        assert_eq!(d.button_focus, None, "and round to the field");
        assert_eq!(tab(&mut d, true), Outcome::Redraw);
        assert_eq!(
            d.button_focus,
            Some(2),
            "Shift+Tab from the field is Browse..."
        );
        tab(&mut d, true);
        assert_eq!(d.button_focus, Some(1), "then Cancel");
        let before = d.field.clone();
        assert_eq!(
            d.key(Key::Edit(EditKey::Insert(" ".into()))),
            Outcome::Close,
            "Space presses Cancel"
        );
        assert_eq!(d.field, before, "the space stayed out of the field");
        assert_eq!(d.key(Key::Enter), Outcome::Close, "Enter presses Cancel");
        tab(&mut d, false);
        assert_eq!(
            d.key(Key::Down),
            Outcome::Nothing,
            "Down leaves the history alone"
        );
        assert_eq!(d.field, before, "the field is unchanged");
        assert_eq!(
            d.key(Key::Enter),
            Outcome::BrowseRun,
            "Enter presses Browse..."
        );
        assert_eq!(d.button_focus, None);
        d.key(Key::Edit(EditKey::End { select: false }));
        d.key(Key::Edit(EditKey::Insert(" x".into())));
        assert_eq!(d.key(Key::Enter), Outcome::Run("notepad x".into()));
    }

    #[test]
    fn run_s_arrows_walk_its_buttons_and_stop_at_the_ends() {
        let mut d = Dialog::run(&history());
        let left = || {
            Key::Edit(EditKey::Left {
                select: false,
                word: false,
            })
        };
        let right = || {
            Key::Edit(EditKey::Right {
                select: false,
                word: false,
            })
        };
        let before = d.field.clone();
        assert_eq!(d.key(Key::Down), Outcome::Redraw);
        assert_ne!(d.field, before, "Down on the field steps the history");
        assert_eq!(d.button_focus, None, "the field keeps the focus");
        let before = d.field.clone();
        d.key(left());
        assert_ne!(d.field, before, "Left on the field moves the caret");
        assert_eq!(d.button_focus, None, "the field keeps the focus");
        d.key(Key::Tab { back: false });
        assert_eq!(d.button_focus, Some(0), "Tab moves to OK");
        let field = d.field.clone();
        assert_eq!(d.key(right()), Outcome::Redraw);
        assert_eq!(d.button_focus, Some(1), "Right moves to Cancel");
        assert_eq!(d.key(Key::Down), Outcome::Redraw);
        assert_eq!(d.button_focus, Some(2), "Down moves to Browse...");
        assert_eq!(d.key(Key::Down), Outcome::Nothing);
        assert_eq!(d.button_focus, Some(2), "Down stops at Browse...");
        assert_eq!(d.key(right()), Outcome::Nothing);
        assert_eq!(d.button_focus, Some(2), "Right stops at Browse...");
        d.key(Key::Up);
        assert_eq!(d.button_focus, Some(1), "Up moves to Cancel");
        d.key(left());
        assert_eq!(d.button_focus, Some(0), "Left moves to OK");
        assert_eq!(d.key(Key::Up), Outcome::Nothing);
        assert_eq!(d.key(left()), Outcome::Nothing);
        assert_eq!(d.button_focus, Some(0), "Up and Left stop at OK");
        assert_eq!(d.field, field, "the field is unchanged");
    }

    #[test]
    fn run_s_field_has_the_focus_after_browse_returns() {
        let mut d = Dialog::run(&[]);
        d.key(Key::Edit(EditKey::Insert("b".into())));
        for _ in 0..3 {
            d.key(Key::Tab { back: false });
        }
        assert_eq!(d.button_focus, Some(2), "Browse... has the focus");
        assert_eq!(d.key(Key::Enter), Outcome::BrowseRun);
        assert_eq!(
            d.button_focus, None,
            "the field has the focus once the picker is cancelled"
        );
        assert_eq!(d.key(Key::Enter), Outcome::Run("b".into()), "Enter is OK");
        let mut d = Dialog::run(&[]);
        d.key(Key::Tab { back: true });
        assert_eq!(d.key(Key::Space), Outcome::BrowseRun);
        d.set_field("'/usr/bin/foot'");
        assert_eq!(
            d.button_focus, None,
            "the field has the focus once the pick lands"
        );
        assert_eq!(
            d.key(Key::Enter),
            Outcome::Run("'/usr/bin/foot'".into()),
            "Enter runs the picked path"
        );
    }

    #[test]
    fn run_s_focused_button_wears_the_ring_and_the_field_hides_its_caret() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let mut d = Dialog::run(&[]);
        d.key(Key::Edit(EditKey::Insert("ab".into())));
        d.key(Key::Tab { back: false });
        d.key(Key::Tab { back: false });
        assert_eq!(d.button_focus, Some(1), "Cancel has the focus");
        let l = DialogLayout::compute(&theme, &text, &d);
        let c = draw(
            &theme,
            &text,
            &l,
            None,
            None,
            &[],
            &[],
            None,
            &d,
            true,
            None,
            Cues::ALL,
            true,
            None,
        );
        let cs = &theme.colors;
        let mid = |r: Rect| (r.x, r.y + r.height / 2);
        let (ok, cancel) = (l.buttons[0], l.buttons[1]);
        assert_eq!(
            c.get(mid(cancel).0, mid(cancel).1),
            cs.button_text.argb(),
            "Cancel's left edge is the black ring"
        );
        assert_eq!(
            c.get(mid(ok).0, mid(ok).1),
            cs.button_hilight.argb(),
            "OK's left edge is the plain white edge"
        );
        let dots = cancel.inset(4);
        assert!(
            (dots.x..dots.right()).any(|x| c.get(x, dots.y) == cs.button_text.argb()),
            "Cancel wears the focus dots"
        );
        let f = l.field.unwrap();
        let inner = f.inset(3);
        let caret_x = f.x + combo::EDIT_TEXT_X + text.measure("ab", false, theme.fonts.size_px);
        assert_eq!(
            c.get(caret_x, inner.y + inner.height / 2),
            cs.window.argb(),
            "the field shows no caret"
        );
    }
    #[test]
    fn log_off_and_turn_off_answer_enter_and_access_keys() {
        let mut d = Dialog::log_off();
        assert_eq!(d.key(Key::Enter), Outcome::Act(Action::Exit));
        assert_eq!(
            d.key(Key::Edit(EditKey::Insert("l".into()))),
            Outcome::Act(Action::Exit)
        );
        assert_eq!(
            d.key(Key::Edit(EditKey::Insert("z".into()))),
            Outcome::Nothing
        );
        let mut t = Dialog::turn_off(&power());
        assert_eq!(
            t.key(Key::Enter),
            Outcome::EndSession(Goal::TurnOff, "systemctl poweroff".into())
        );
        assert_eq!(
            t.key(Key::Edit(EditKey::Insert("S".into()))),
            Outcome::Spawn("systemctl suspend".into())
        );
        assert_eq!(
            t.key(Key::Edit(EditKey::Insert("r".into()))),
            Outcome::EndSession(Goal::Restart, "systemctl reboot".into())
        );
        assert_eq!(t.press(3), Outcome::Close);
        assert_eq!(t.press(9), Outcome::Nothing);
    }

    fn tree_of(
        d: &Dialog,
    ) -> (
        reprisekit::access::Tree,
        reprisekit::access::accesskit::NodeId,
    ) {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let layout = DialogLayout::compute(&theme, &text, d);
        let mut t = reprisekit::access::Tree::window(&d.title, layout.size);
        let focus = access(d, &layout, &text, theme.fonts.size_px, &mut t);
        (t, focus)
    }

    #[test]
    fn run_focuses_its_labelled_field_and_names_its_buttons() {
        use reprisekit::access::accesskit::Role;
        let (t, focus) = tree_of(&Dialog::run(&history()));
        assert_eq!(t.get(focus).unwrap().role(), Role::TextInput);
        let combo = t.nodes().find(|(_, n)| n.role() == Role::EditableComboBox);
        let (_, combo) = combo.expect("the field is an editable combo");
        let label = t.get(combo.labelled_by()[0]).unwrap();
        assert_eq!(label.value(), Some("Open:"), "the field is labelled");
        for name in ["OK", "Cancel", "Browse..."] {
            assert!(
                t.nodes().any(|(_, n)| n.label() == Some(name)),
                "{name} is a button"
            );
        }
    }

    #[test]
    fn a_message_box_focuses_its_default_button_and_reads_its_text() {
        use reprisekit::access::accesskit::Role;
        let (t, focus) = tree_of(&Dialog::key_aid_prompt(crate::keyfilter::Aid::StickyKeys));
        let yes = t.get(focus).unwrap();
        assert_eq!(yes.role(), Role::DefaultButton);
        assert_eq!(yes.label(), Some("Yes"));
        assert!(
            t.nodes()
                .any(|(_, n)| n.value().is_some_and(|v| v.contains("Turn on StickyKeys?"))),
            "the text is there"
        );
        let (t, focus) = tree_of(&Dialog::turn_off(&power()));
        assert_eq!(t.get(focus).unwrap().label(), Some("Turn Off"));
    }

    #[test]
    fn wrap_breaks_on_words_and_inside_words_too_long_for_a_line() {
        let text = TextRenderer::load(&FontConfig::default());
        let px = 11;
        let one = text.measure("aaaa", false, px);
        assert_eq!(
            wrap(&text, "aaaa bbbb cccc", one, px),
            vec!["aaaa", "bbbb", "cccc"]
        );
        let all = text.measure("aaaa bbbb cccc", false, px);
        assert_eq!(
            wrap(&text, "aaaa bbbb cccc", all, px),
            vec!["aaaa bbbb cccc"]
        );
        assert_eq!(
            wrap(
                &text,
                "toolongtofit",
                text.measure("toolong", false, px),
                px
            ),
            vec!["toolong", "tofit"]
        );
        assert!(wrap(&text, "   ", 100, px).is_empty());
    }

    #[test]
    fn layout_stacks_text_field_and_buttons() {
        let theme = Theme::default();
        let Some(text) = tahoma() else { return };
        let d = Dialog::run(&[]);
        let l = DialogLayout::compute(&theme, &text, &d);
        assert_eq!(l.size, (347, 179));
        assert_eq!(l.icon.unwrap(), Rect::new(14, 40, 32, 32));
        assert_eq!(l.lines.len(), 2, "the Run text wraps next to the icon");
        assert_eq!(l.lines[0].0, Rect::new(57, 40, 276, 13));
        assert_eq!(l.lines[1].0.y, 53);
        let field = l.field.unwrap();
        assert_eq!(field, Rect::new(57, 82, 275, 21));
        assert_eq!(l.field_label.unwrap().x, 14);
        assert_eq!(l.buttons.len(), 3);
        assert_eq!(l.buttons[0], Rect::new(96, 136, 75, 23));
        assert_eq!(l.buttons[1].x, 177);
        assert_eq!(l.buttons[2].right(), field.right() + 1);
        let b = l.buttons[1];
        assert_eq!(
            l.hit(&d, b.x + 1, b.y + 1, &text, theme.fonts.size_px),
            Some(DialogHit::Button(1))
        );
        assert_eq!(
            l.hit(&d, b.x - 1, b.y + 1, &text, theme.fonts.size_px),
            None
        );
        assert!(l.help.is_some() && l.drop.is_some());
        let c = Dialog::confirm_delete(
            "New Text Document",
            std::path::Path::new("/x"),
            false,
            false,
            false,
        );
        let l = DialogLayout::compute(&theme, &text, &c);
        assert_eq!(l.size.0, 419);
        assert_eq!(l.icon.unwrap(), Rect::new(17, 38, 32, 32));
        assert_eq!(l.lines[0].0, Rect::new(65, 38, 330, 13));
        assert_eq!(l.lines.len(), 2);
        assert_eq!(l.buttons[0], Rect::new(239, 96, 80, 23));
        assert_eq!(l.buttons[1], Rect::new(326, 96, 80, 23));
        assert_eq!(l.size.1, 133);
        let dir = std::env::temp_dir();
        let f = Dialog::confirm_delete("New Folder", &dir, true, false, false);
        assert_eq!(f.title, "Confirm Folder Delete");
        assert!(
            f.text
                .starts_with("Are you sure you want to remove the folder 'New Folder'")
        );
        let l = DialogLayout::compute(&theme, &text, &f);
        assert_eq!(l.buttons[0].y, 115);
        assert_eq!(l.size, (419, 152));
    }

    #[test]
    fn confirm_delete_icons_asks_about_the_places_then_the_files() {
        let theme = Theme::default();
        let Some(text) = tahoma() else { return };
        let hide = vec!["home".to_string()];
        let files = vec![
            std::path::PathBuf::from("/d/a.txt"),
            std::path::PathBuf::from("/d/b.txt"),
        ];
        let mut d = Dialog::confirm_delete_icons(hide.clone(), files.clone());
        assert_eq!(d.title, "Confirm Delete");
        let l = DialogLayout::compute(&theme, &text, &d);
        assert_eq!(l.size, (512, 139));
        let ys: Vec<i32> = l.lines.iter().map(|(r, _)| r.y).collect();
        assert_eq!(ys, [33, 59, 72]);
        assert_eq!(
            l.lines[2].1,
            "To restore them later, use the Background page of Reprise Settings."
        );
        assert_eq!(l.buttons[0].y, 102);
        let yes = Outcome::DeleteIcons {
            hide,
            files: files.clone(),
        };
        assert_eq!(d.key(Key::Enter), yes);
        assert_eq!(
            d.key(Key::Edit(EditKey::Insert("n".into()))),
            Outcome::DeleteIcons {
                hide: Vec::new(),
                files
            }
        );
        assert_eq!(d.key(Key::Escape), Outcome::Close);

        let places =
            Dialog::confirm_delete_icons(vec!["computer".into(), "home".into()], Vec::new());
        assert!(
            places.text.ends_with(
                "\n\nTo restore them later, use the Background page of Reprise Settings."
            )
        );
        let l = DialogLayout::compute(&theme, &text, &places);
        assert_eq!(l.size, (404, 126));
    }

    #[test]
    fn confirm_hide_fits_its_text() {
        use crate::desktop::Place;
        let theme = Theme::default();
        let Some(text) = tahoma() else { return };
        let mut d = Dialog::confirm_hide(Place::Computer, "Computer");
        assert_eq!(d.title, "Confirm Delete");
        assert_eq!(d.icon.as_deref(), Some("folder-documents"));
        assert_eq!(
            d.text,
            "Are you sure you want to delete the Computer icon from your desktop?\n\n\
             To restore it later, use the Background page of Reprise Settings."
        );
        let l = DialogLayout::compute(&theme, &text, &d);
        assert_eq!(l.size, (418, 126));
        assert_eq!(l.icon.unwrap(), Rect::new(14, 33, 32, 32));
        assert_eq!(l.lines.len(), 2);
        assert_eq!((l.lines[0].0.x, l.lines[0].0.y), (65, 33));
        assert_eq!(l.lines[1].0.y, 59);
        assert_eq!(l.buttons[0], Rect::new(131, 89, 75, 23));
        assert_eq!(l.buttons[1], Rect::new(212, 89, 75, 23));
        let yes = Outcome::HideSystemIcon("computer".into());
        assert_eq!(d.key(Key::Enter), yes);
        assert_eq!(d.key(Key::Edit(EditKey::Insert("y".into()))), yes);
        assert_eq!(
            d.key(Key::Edit(EditKey::Insert("n".into()))),
            Outcome::Close
        );
        assert_eq!(d.key(Key::Escape), Outcome::Close);
        let folder_restore = Some(
            "The contents of the folder will not be deleted. You can restore this icon \
             to the desktop on the Background page of Reprise Settings.",
        );
        let home = Dialog::confirm_hide(Place::Home, "u's Home");
        assert_eq!(home.text.split("\n\n").nth(1), folder_restore);
        let docs = Dialog::confirm_hide(Place::Documents, "Documents");
        assert_eq!(docs.text.split("\n\n").nth(1), folder_restore);
        let l = DialogLayout::compute(&theme, &text, &docs);
        assert_eq!(l.size, (707, 126));
        assert_eq!(l.buttons[0].x, 275);
        let c = draw(
            &theme,
            &text,
            &l,
            None,
            None,
            &[],
            &[],
            None,
            &docs,
            true,
            None,
            Cues::ALL,
            true,
            None,
        );
        let red = Color::rgb(255, 0, 0).argb();
        assert_eq!(c.get(14 + 3, 33 + 25), red);
        assert_eq!(c.get(14 + 8, 33 + 22), Color::rgb(255, 255, 255).argb());
        assert_ne!(c.get(14 + 3, 33 + 18), red);
    }

    #[test]
    fn welcome_panels_lay_out_their_cells() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let turn_off = Dialog::turn_off(&power());
        let t = DialogLayout::compute(&theme, &text, &turn_off);
        assert_eq!(t.size, (314, 200));
        assert_eq!(t.caption, Rect::new(1, 1, 312, 43));
        assert!(t.lines.is_empty() && t.icon.is_none());
        let centres: Vec<i32> = t.buttons[..3].iter().map(|r| r.x + r.width / 2).collect();
        assert_eq!(centres, vec![71, 157, 242]);
        assert_eq!(t.buttons[0], Rect::new(29, 73, 85, 64));
        assert_eq!(t.buttons[3], Rect::new(243, 168, 60, 20));
        assert_eq!(
            t.hit(&turn_off, 156, 100, &text, theme.fonts.size_px),
            Some(DialogHit::Button(1))
        );
        assert_eq!(
            t.hit(&turn_off, 250, 175, &text, theme.fonts.size_px),
            Some(DialogHit::Button(3))
        );
        assert_eq!(t.hit(&turn_off, 5, 100, &text, theme.fonts.size_px), None);
        let l = DialogLayout::compute(&theme, &text, &Dialog::log_off());
        let centres: Vec<i32> = l.buttons[..2].iter().map(|r| r.x + r.width / 2).collect();
        assert_eq!(centres, vec![100, 215]);
        assert_eq!(l.buttons[2], t.buttons[3]);
    }

    #[test]
    fn classic_dialog_has_a_close_button_and_a_focus_ring() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let d = Dialog::confirm_delete("x", std::path::Path::new("/x"), false, false, false);
        let l = DialogLayout::compute(&theme, &text, &d);
        let close = l.close.unwrap();
        assert_eq!(
            close,
            Rect::new(l.caption.right() - 18, l.caption.y + 2, 16, 14)
        );
        assert_eq!(
            l.hit(&d, close.x + 5, close.y + 5, &text, theme.fonts.size_px),
            Some(DialogHit::Close)
        );
        assert_eq!(
            l.hit(
                &d,
                l.caption.x + 20,
                l.caption.y + 5,
                &text,
                theme.fonts.size_px
            ),
            Some(DialogHit::Caption)
        );
        assert_eq!(
            l.hit(
                &d,
                l.caption.x + 20,
                l.caption.bottom() + 4,
                &text,
                theme.fonts.size_px
            ),
            None,
            "below the caption is the body, not a grip"
        );
        assert!(l.help.is_none(), "only Run has the ? button");
        let c = draw(
            &theme,
            &text,
            &l,
            None,
            None,
            &[],
            &[],
            None,
            &d,
            true,
            None,
            Cues::ALL,
            true,
            None,
        );
        let cs = &theme.colors;
        assert_eq!(c.get(close.x, close.y + 7), cs.button_hilight.argb());
        assert_eq!(c.get(close.x + 7, close.y + 6), cs.button_text.argb());
        let yes = l.buttons[0].inset(4);
        assert_ne!(c.get(yes.x, yes.y), cs.button_text.argb());
        assert_eq!(c.get(yes.x + 1, yes.y), cs.button_text.argb());
        assert_ne!(c.get(yes.x + 2, yes.y), cs.button_text.argb());
        let no = l.buttons[1].inset(4);
        assert_ne!(c.get(no.x, no.y), cs.button_text.argb());
        let run = Dialog::run(&[]);
        let rl = DialogLayout::compute(&theme, &text, &run);
        let rc = draw(
            &theme,
            &text,
            &rl,
            None,
            None,
            &[],
            &[],
            None,
            &run,
            true,
            None,
            Cues::ALL,
            true,
            None,
        );
        let ok = rl.buttons[0].inset(4);
        assert_ne!(rc.get(ok.x + 1, ok.y), cs.button_text.argb());
        assert!(
            DialogLayout::compute(&theme, &text, &Dialog::log_off())
                .close
                .is_none()
        );
    }

    #[test]
    fn confirm_delete_trashes_on_yes_only() {
        let path = std::path::Path::new("/home/u/Desktop/it's mine.txt");
        let mut d = Dialog::confirm_delete("it's mine.txt", path, false, false, false);
        assert_eq!(d.placement, Placement::Center);
        assert!(d.text.contains("'it's mine.txt'"));
        let yes = Outcome::Delete {
            paths: vec![path.to_path_buf()],
            erase: false,
        };
        assert_eq!(d.key(Key::Enter), yes);
        assert_eq!(d.key(Key::Edit(EditKey::Insert("y".into()))), yes);
        assert_eq!(
            d.key(Key::Edit(EditKey::Insert("n".into()))),
            Outcome::Close
        );
        assert_eq!(d.key(Key::Escape), Outcome::Close);
        assert_eq!(d.press(1), Outcome::Close);
        let mut e = Dialog::confirm_delete("it's mine.txt", path, false, true, false);
        assert_eq!(e.text, "Are you sure you want to delete 'it's mine.txt'?");
        assert_eq!(e.icon.as_deref(), Some("folder"));
        assert!(matches!(
            e.style,
            Style::Classic {
                delete_badge: true,
                ..
            }
        ));
        assert_eq!(
            e.key(Key::Enter),
            Outcome::Delete {
                paths: vec![path.to_path_buf()],
                erase: true,
            }
        );
        let dir = std::env::temp_dir();
        let f = Dialog::confirm_delete("New Folder", &dir, true, true, false);
        assert_eq!(f.title, "Confirm Folder Delete");
        assert_eq!(
            f.text,
            "Are you sure you want to remove the folder 'New Folder' and all its contents?"
        );
    }

    #[test]
    fn too_big_asks_with_the_batch_buttons() {
        use reprisefs::bincap::{Measured, TooBig};
        use reprisefs::replace::Answer;
        let big = std::path::PathBuf::from("/d/big.iso");
        let too_big = Some(Measured::unstamped(2, 1));
        let asker = TooBig::new(vec!["/d/a".into(), big.clone()], vec![None, too_big]);
        let mut d = Dialog::too_big(&big, false, asker.clone());
        assert_eq!(d.title, "Confirm File Delete");
        assert_eq!(
            d.text,
            "'big.iso' is too big for the Recycle Bin. Do you want to delete it permanently?"
        );
        let labels: Vec<&str> = d.buttons.iter().map(|b| b.label.as_str()).collect();
        assert_eq!(labels, ["&Yes", "Yes to &All", "&No", "Cancel"]);
        let answer = |answer| Outcome::TooBig {
            asker: Box::new(asker.clone()),
            answer,
        };
        assert_eq!(d.key(Key::Enter), answer(Answer::Yes));
        assert_eq!(d.press(1), answer(Answer::YesToAll));
        assert_eq!(d.press(2), answer(Answer::No));
        assert_eq!(d.press(3), Outcome::Close);
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let l = DialogLayout::compute(&theme, &text, &d);
        assert_eq!(l.size.0, 419);
        assert_eq!(l.buttons.len(), 4);
        assert_eq!(l.buttons[3].right(), 419 - 13);
        for w in l.buttons.windows(2) {
            assert_eq!((w[0].width, w[1].x - w[0].right()), (80, 7));
        }

        let one = TooBig::new(vec![big.clone()], vec![too_big]);
        let d = Dialog::too_big(&big, true, one);
        assert_eq!(d.title, "Confirm Folder Delete");
        let labels: Vec<&str> = d.buttons.iter().map(|b| b.label.as_str()).collect();
        assert_eq!(labels, ["&Yes", "&No"]);
    }

    #[test]
    fn confirm_delete_many_counts_the_items_and_trashes_them_all() {
        let paths = vec![
            std::path::PathBuf::from("/d/New Folder"),
            std::path::PathBuf::from("/d/a b.txt"),
        ];
        let mut d = Dialog::confirm_delete_many(&paths, false, BinLack::None);
        assert_eq!(d.title, "Confirm Multiple File Delete");
        assert_eq!(
            d.text,
            "Are you sure you want to send these 2 items to the Recycle Bin?"
        );
        assert_eq!(
            d.key(Key::Enter),
            Outcome::Delete {
                paths: paths.clone(),
                erase: false,
            }
        );
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let l = DialogLayout::compute(&theme, &text, &d);
        assert_eq!(l.size, (419, 135));
        assert_eq!(l.icon.map(|r| r.y), Some(38));
        assert_eq!(l.lines[0].0.y, 45);
        assert_eq!(l.buttons[0], Rect::new(224, 96, 87, BUTTON_H));
        assert_eq!(l.buttons[1], Rect::new(318, 96, 87, BUTTON_H));
        let mut e = Dialog::confirm_delete_many(&paths, true, BinLack::None);
        assert_eq!(e.text, "Are you sure you want to delete these 2 items?");
        assert!(matches!(
            e.style,
            Style::Classic {
                delete_badge: true,
                ..
            }
        ));
        assert_eq!(e.key(Key::Enter), Outcome::Delete { paths, erase: true });
    }

    /// Yes deletes the path by its bytes: a name that is not UTF-8 reaches
    /// the host as it is, not with U+FFFD in place of the stray byte.
    #[test]
    fn confirm_delete_keeps_a_name_that_is_not_utf8() {
        use std::os::unix::ffi::OsStrExt;
        let path = std::path::PathBuf::from(std::ffi::OsStr::from_bytes(b"/d/a\xffb.txt"));
        let mut d = Dialog::confirm_delete("a\u{fffd}b.txt", &path, false, false, false);
        assert_eq!(
            d.key(Key::Enter),
            Outcome::Delete {
                paths: vec![path.clone()],
                erase: false,
            }
        );
        let paths = vec![path, std::path::PathBuf::from("/d/c.txt")];
        let mut m = Dialog::confirm_delete_many(&paths, true, BinLack::None);
        assert_eq!(m.key(Key::Enter), Outcome::Delete { paths, erase: true });
    }

    #[test]
    fn a_delete_off_a_drive_with_no_bin_says_so_first() {
        let one =
            Dialog::confirm_delete("d.txt", std::path::Path::new("/m/d.txt"), false, true, true);
        assert_eq!(
            one.text,
            "This drive has no Recycle Bin. Are you sure you want to delete 'd.txt'?"
        );
        let paths = vec!["/m/a".into(), "/m/b".into()];
        let all = Dialog::confirm_delete_many(&paths, true, BinLack::All);
        assert_eq!(
            all.text,
            "These items are on a drive with no Recycle Bin. Are you sure you want to delete \
             these 2 items?"
        );
        let some = Dialog::confirm_delete_many(&paths, true, BinLack::Some);
        assert_eq!(
            some.text,
            "Some of these items are on a drive with no Recycle Bin. Are you sure you want to \
             delete these 2 items?"
        );
    }

    #[test]
    fn confirm_empty_bin_names_one_item_or_counts_them() {
        let one = Dialog::confirm_empty_bin(1, Some("notes.txt"));
        assert_eq!(one.title, "Confirm File Delete");
        assert_eq!(one.text, "Are you sure you want to delete 'notes.txt'?");
        let mut many = Dialog::confirm_empty_bin(3, Some("a"));
        assert_eq!(many.title, "Confirm Multiple File Delete");
        assert_eq!(many.text, "Are you sure you want to delete these 3 items?");
        assert_eq!(many.key(Key::Enter), Outcome::EmptyBin);
    }

    #[test]
    fn disabled_button_is_inert() {
        let mut d = Dialog::log_off();
        assert!(!d.buttons[0].enabled);
        assert_eq!(d.press(0), Outcome::Nothing);
        assert_eq!(
            d.key(Key::Edit(EditKey::Insert("s".into()))),
            Outcome::Nothing
        );
        assert_eq!(d.press(1), Outcome::Act(Action::Exit));
        assert_eq!(d.press(2), Outcome::Close);
    }

    #[test]
    fn run_s_drop_button_draws_held_until_the_release() {
        let mut d = Dialog::run(&["a".into()]);
        assert_eq!(d.click(Some(DialogHit::Drop)), Outcome::Redraw);
        assert_eq!(d.field_lit.of(FieldPart::Drop), Interaction::Pressed);
        assert!(d.drag_drop(None), "dragged off");
        assert_eq!(d.field_lit.of(FieldPart::Drop), Interaction::Idle);
        assert!(d.drag_drop(Some(DialogHit::Drop)), "back on");
        assert_eq!(d.release(Some(DialogHit::Drop)), Outcome::Redraw);
        assert!(!d.field_lit.holding());
        assert_eq!(d.field_lit.of(FieldPart::Drop), Interaction::Hot);
    }

    #[test]
    fn run_s_field_lights_its_frame_from_either_part_and_its_button_alone() {
        let mut d = Dialog::run(&[]);
        assert!(d.hover(Some(DialogHit::Field(3)), (0, 0), true));
        assert_eq!(d.field_lit.of(FieldPart::Edit), Interaction::Hot);
        assert_eq!(d.field_lit.of(FieldPart::Drop), Interaction::Idle);
        assert!(d.hover(Some(DialogHit::Drop), (0, 0), true));
        assert_eq!(d.field_lit.of(FieldPart::Drop), Interaction::Hot);
        assert!(d.leave(true));
        assert_eq!(d.field_lit.lit(), None);
    }

    #[test]
    fn a_classic_button_lights_under_the_pointer_and_a_leave_clears_it() {
        let mut d = Dialog::log_off();
        let on = Some(DialogHit::Button(1));
        assert!(d.hover(on, (0, 0), true));
        assert_eq!(d.lit(), Some((1, Interaction::Hot)));
        assert!(!d.hover(on, (0, 0), true), "still on it");
        assert!(
            d.hover(Some(DialogHit::Button(0)), (0, 0), true),
            "onto a greyed one"
        );
        assert_eq!(d.lit(), None, "a greyed button does not light");
        d.hover(on, (0, 0), true);
        assert!(d.leave(true));
        assert_eq!(d.lit(), None);
        let mut m = Dialog::end_program("foot", 4321);
        assert!(
            !m.hover(Some(DialogHit::Button(0)), (0, 0), false),
            "a Classic message box has nothing to redraw"
        );
    }

    #[test]
    fn the_help_box_lights_hot_and_shows_the_press_until_the_release() {
        let mut d = Dialog::log_off();
        let help = Some(DialogHit::Help);
        assert!(d.hover(help, (0, 0), true));
        assert_eq!(d.help_box.of(()), Interaction::Hot);
        d.press_help();
        assert_eq!(d.help_box.of(()), Interaction::Pressed);
        assert!(d.drag_help(false), "dragged off");
        assert_eq!(d.help_box.of(()), Interaction::Idle);
        assert!(d.drag_help(true), "back on");
        assert!(d.release_help());
        assert_eq!(d.help_box.of(()), Interaction::Hot);
        assert!(d.leave(true));
        assert_eq!(d.help_box.of(()), Interaction::Idle);
    }

    #[test]
    fn a_blue_dialog_caption_shows_its_caption_icon_as_given() {
        let blue = reprisekit::interfaces::theme_for(reprisekit::interface::Interface::Blue);
        let text = TextRenderer::load(&blue.fonts);
        let d = crate::binprops::dialog(
            crate::binprops::BinSheet::new(
                Default::default(),
                vec![crate::binprops::Drive {
                    key: "home".into(),
                    label: "Local Disk (/home)".into(),
                    size: 21_365_735_424,
                }],
            ),
            (0, 0),
            "user-trash",
        );
        let l = DialogLayout::compute(&blue, &text, &d);
        let mut big = Canvas::new(32, 32);
        big.fill_rect(big.bounds(), Color::rgb(0x00, 0xFF, 0x00));
        let mut small = Canvas::new(16, 16);
        small.fill_rect(small.bounds(), Color::rgb(0xFF, 0x00, 0xFF));
        let c = draw(
            &blue,
            &text,
            &l,
            Some(&big),
            Some(&small),
            &[],
            &[],
            None,
            &d,
            false,
            Lit::default(),
            Cues::NONE,
            true,
            None,
        );
        let caption = l.frame.expect("a styled dialog has a frame").caption;
        let count = |argb: u32| {
            (caption.y..caption.bottom())
                .flat_map(|y| (caption.x..caption.right()).map(move |x| (x, y)))
                .filter(|&(x, y)| c.get(x, y) == argb)
                .count()
        };
        assert_eq!(count(0xFFFF_00FF), 256, "the 16x16 caption icon, unscaled");
        assert_eq!(count(0xFF00_FF00), 0, "no trace of the 32 px icon");
    }

    #[test]
    fn a_blue_form_dialog_lights_its_tab_and_control_and_a_leave_clears_them() {
        use reprisekit::controls::form::Light;
        let blue = reprisekit::interfaces::theme_for(reprisekit::interface::Interface::Blue);
        let text = TextRenderer::load(&blue.fonts);
        let px = blue.fonts.size_px;
        let mut d = crate::binprops::dialog(
            crate::binprops::BinSheet::new(
                Default::default(),
                vec![crate::binprops::Drive {
                    key: "home".into(),
                    label: "Local Disk (/home)".into(),
                    size: 21_365_735_424,
                }],
            ),
            (0, 0),
            "user-trash",
        );
        let l = DialogLayout::compute(&blue, &text, &d);
        draw(
            &blue,
            &text,
            &l,
            None,
            None,
            &[],
            &[],
            None,
            &d,
            false,
            None,
            Cues::NONE,
            true,
            None,
        );
        assert!(d.hover(Some(DialogHit::Tab(1)), (0, 0), true));
        let tabs = |d: &Dialog| d.form().unwrap().sheet().unwrap().tabs.hot;
        assert_eq!(tabs(&d), Some(1));
        let page = d.form().unwrap().open_page().unwrap();
        let ok = page.index_of("ok").unwrap();
        let r = page.rect("ok").unwrap();
        let (x, y) = (r.x + 5, r.y + 5);
        let over = l.hit(&d, x, y, &text, px);
        assert!(matches!(over, Some(DialogHit::Control(_))));
        assert!(d.hover(over, (x, y), true));
        let lit = |d: &Dialog| d.form().unwrap().open_page().unwrap().lit(ok, Light::Body);
        assert_eq!(lit(&d), Interaction::Hot);
        assert_eq!(tabs(&d), None, "off the tab");
        assert!(d.leave(true));
        assert_eq!(lit(&d), Interaction::Idle);
    }

    #[test]
    fn a_button_acts_on_the_release_over_it() {
        let mut d = Dialog::log_off();
        assert!(!d.press_button(0), "disabled");
        assert_eq!(d.lit(), None);
        assert!(d.press_button(1));
        assert_eq!(d.lit(), Some((1, Interaction::Pressed)));
        assert!(d.drag_button(None), "left the button");
        assert_eq!(d.lit(), None);
        assert!(!d.drag_button(None), "still off it");
        assert_eq!(d.release_button(None), Some(Outcome::Redraw));
        assert_eq!(d.release_button(None), None, "nothing held");
        assert!(d.press_button(1));
        assert!(d.drag_button(Some(2)));
        assert!(d.drag_button(Some(1)));
        assert_eq!(d.release_button(Some(1)), Some(Outcome::Act(Action::Exit)));
    }

    #[test]
    fn a_cancelled_press_does_not_act_on_the_release() {
        let mut d = Dialog::log_off();
        assert!(d.press_button(1));
        d.press_help();
        assert_eq!(d.cancel_press(), Outcome::Redraw);
        assert_eq!(d.lit(), None, "drawn released");
        assert!(!d.help_box.holding(), "the ? box lets go too");
        assert_eq!(d.release_button(Some(1)), None, "nothing held");
        assert_eq!(d.release(None), Outcome::Nothing);
    }

    #[test]
    fn welcome_gradient_varies_across_and_down() {
        const PIXELS: [(i32, i32, u32); 18] = [
            (1, 46, 0x5a7dde),
            (162, 46, 0x93b2f4),
            (158, 60, 0x94b3f4),
            (157, 155, 0x84a4ee),
            (41, 150, 0x6284e1),
            (196, 62, 0x97b5f5),
            (249, 150, 0x7f9fec),
            (270, 50, 0x7b9beb),
            (310, 60, 0x698be4),
            (20, 70, 0x5d80df),
            (300, 146, 0x6b8de5),
            (100, 60, 0x7d9deb),
            (229, 143, 0x87a7ef),
            (5, 156, 0x0f40a4),
            (157, 156, 0x1444a6),
            (300, 156, 0x0f40a4),
            (157, 157, 0x013399),
            (300, 157, 0x003399),
        ];
        for d in [Dialog::log_off(), Dialog::turn_off(&power())] {
            let (c, _) = welcome_drawn(&d, None, Cues::NONE);
            for (x, y, rgb) in PIXELS {
                assert_eq!(
                    c.get(x, y) & 0xff_ffff,
                    rgb,
                    "{} at ({x}, {y}) is {rgb:06X}",
                    d.title
                );
            }
        }
    }

    /// Writes both panels to `target/log-off.png` and `target/turn-off.png`.
    #[test]
    #[ignore]
    fn render_the_welcome_panels() {
        for (d, name) in [
            (Dialog::log_off(), "log-off"),
            (Dialog::turn_off(&power()), "turn-off"),
        ] {
            let (c, _) = welcome_drawn(&d, None, Cues::NONE);
            let path = format!("{}/target/{name}.png", env!("CARGO_MANIFEST_DIR"));
            std::fs::write(&path, reprisekit::raster::to_png(&c)).unwrap();
        }
    }

    /// At 200% text the Turn Off panel doubles: its title, cells and Cancel grow, the gradient
    /// and the lines under it stretch over the grown band, and the icons keep their 32 px while
    /// the labels grow under them.
    #[test]
    fn the_welcome_panel_grows_with_the_text_size() {
        let theme = Theme::default().with_text_size(200);
        let u = theme.text;
        let text = TextRenderer::load(&theme.fonts);
        let d = Dialog::turn_off(&power());
        let l = DialogLayout::compute(&theme, &text, &d);
        assert_eq!(l.size, (628, 400));
        assert_eq!(l.caption, u.rect(Rect::new(1, 1, 312, 43)));
        assert_eq!(l.buttons[0], u.rect(Rect::new(29, 73, 85, 64)));
        assert_eq!(l.buttons[3], u.rect(Rect::new(243, 168, 60, 20)));
        let icon = Canvas::new(ICON_PX as u32, ICON_PX as u32);
        let mut icon = icon;
        icon.fill_rect(icon.bounds(), Color::rgb(1, 2, 3));
        let c = draw(
            &theme,
            &text,
            &l,
            None,
            None,
            &[Some(icon.clone()), Some(icon.clone()), Some(icon), None],
            &[],
            None,
            &d,
            true,
            None,
            Cues::ALL,
            true,
            None,
        );
        assert_eq!((c.width as i32, c.height as i32), l.size);
        // The band runs from under the separator's two rows to the lines at 2 x 156.
        let band_top = l.caption.bottom() + 2;
        assert_eq!(c.get(3, band_top), panel_gradient(0, 0).argb());
        assert_eq!(c.get(3, 311), panel_gradient(0, 109).argb());
        assert_eq!(c.get(3, 312), PANEL_LINE_RUNS[0][0].0.argb());
        assert_eq!(c.get(3, 313), PANEL_LINE_RUNS[1][0].0.argb());
        let (bw, bh) = (l.caption.width, 312 - band_top);
        let at = panel_gradient(366 * PANEL_W / bw, 100 * PANEL_H / bh);
        assert_eq!(
            c.get(1 + 366, band_top + 100),
            at.argb(),
            "the gradient stretched"
        );
        let cell = l.buttons[2];
        let shown = (cell.y..cell.bottom())
            .filter(|&y| c.get(cell.x + cell.width / 2, y) == Color::rgb(1, 2, 3).argb())
            .count();
        assert_eq!(shown, ICON_PX as usize, "the icon keeps its 32 px");
        let white = Color::rgb(255, 255, 255).argb();
        let label_rows: Vec<i32> = (cell.y..cell.bottom())
            .filter(|&y| (cell.x..cell.right()).any(|x| c.get(x, y) == white))
            .collect();
        let (top, bottom) = (label_rows[0], *label_rows.last().unwrap());
        assert!(
            bottom - top >= 12,
            "the label's ink grows, rows {top}..={bottom}"
        );
        assert!(bottom < cell.bottom(), "and stays in its cell");
    }

    /// At 200% text the Open With chooser's and the Browse dialog's clients double inside their
    /// frames, and a click lands on the grown Cancel.
    #[test]
    fn open_with_and_browse_grow_with_the_text_size() {
        let theme = Theme::default().with_text_size(200);
        let text = TextRenderer::load(&theme.fonts);
        let px = theme.fonts.size_px;
        let caption_grow = theme.metrics.caption_height - 18;
        let d = chooser();
        let l = DialogLayout::compute(&theme, &text, &d);
        let (cw, ch) = openwith::CLIENT;
        assert_eq!(l.page.map(|p| (p.width, p.height)), Some((2 * cw, 2 * ch)));
        assert_eq!(l.size, (2 * cw + 6, 2 * ch + 21 + caption_grow + 3));

        let dir = std::env::temp_dir().join(format!("reprise-browse-grown-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut d = Dialog::browse(crate::filepick::FilePicker::run_command(
            &dir,
            crate::filepick::Icons::default(),
            true,
        ));
        d.fit(&theme, &text);
        let l = DialogLayout::compute(&theme, &text, &d);
        let page = l.page.unwrap();
        assert_eq!((page.width, page.height), (1110, 902));
        let c = render(&theme, &text, &l, &d);
        assert_eq!((c.width as i32, c.height as i32), l.size);
        // Cancel: 150x46, 12 px in from the right, 2 px above the Files of type row.
        let (x, y) = (page.right() - 12 - 150 + 10, page.y + 844 - 2 + 10);
        assert_eq!(
            l.hit(&d, x, y, &text, px),
            Some(DialogHit::Browse(
                reprisekit::controls::filedlg::Hit::Cancel
            ))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn welcome_panel_draws_bands_gradient_and_labels() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let d = Dialog::log_off();
        let l = DialogLayout::compute(&theme, &text, &d);
        let c = draw(
            &theme,
            &text,
            &l,
            None,
            None,
            &[None, None, None],
            &[],
            None,
            &d,
            true,
            None,
            Cues::ALL,
            true,
            None,
        );
        assert_eq!(c.get(0, 0), Color::rgb(0, 0, 0).argb());
        assert_eq!(c.get(5, 5), PANEL_BAND.argb());
        assert_eq!(c.get(5, 190), PANEL_BAND.argb());
        let count = |r: Rect, color: Color| {
            (r.x..r.right())
                .flat_map(|x| (r.y..r.bottom()).map(move |y| (x, y)))
                .filter(|&(x, y)| c.get(x, y) == color.argb())
                .count()
        };
        let label = |r: Rect| Rect::new(r.x, r.y + 44, r.width, 20);
        assert!(count(label(l.buttons[0]), PANEL_DISABLED) > 10);
        assert_eq!(count(label(l.buttons[0]), Color::rgb(255, 255, 255)), 0);
        assert!(count(label(l.buttons[1]), Color::rgb(255, 255, 255)) > 10);
        let cancel = l.buttons[2];
        assert_eq!(
            c.get(cancel.x, cancel.y + 9),
            theme.colors.button_hilight.argb()
        );
        assert_eq!(
            c.get(cancel.x + 5, cancel.y + 3),
            theme.colors.button_face.argb()
        );
    }

    #[test]
    fn the_welcome_separator_fades_toward_its_ends() {
        for runs in SEPARATOR_RUNS {
            let width: i32 = runs.iter().map(|&(_, n)| n as i32).sum();
            assert_eq!(width, WELCOME_W - 2 * WELCOME_BORDER);
        }
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let d = Dialog::turn_off(&power());
        let l = DialogLayout::compute(&theme, &text, &d);
        let c = draw(
            &theme,
            &text,
            &l,
            None,
            None,
            &[None, None, None],
            &[],
            None,
            &d,
            true,
            None,
            Cues::NONE,
            true,
            None,
        );
        let rgb = |x, y| c.get(x, y) & 0x00ff_ffff;
        assert_eq!(rgb(5, 43), 0x003399, "the title band above");
        for (x, want) in [
            (1, 0x3c64b6),
            (10, 0x4a6fba),
            (60, 0x9db5df),
            (113, 0xd5e4f8),
            (114, 0xd8e6fa),
            (154, 0xd8e6fa),
            (155, 0xd5e4f8),
            (230, 0xbccfed),
            (304, 0x4a6fba),
            (305, 0x3c64b6),
            (312, 0x3c64b6),
        ] {
            assert_eq!(rgb(x, 44), want, "row 44, column {x}");
        }
        for (x, want) in [
            (1, 0x3c64b6),
            (2, 0x3053b5),
            (10, 0x3c64b6),
            (134, 0xd8e6fa),
            (311, 0x3c64b6),
            (312, 0x3053b5),
        ] {
            assert_eq!(rgb(x, 45), want, "row 45, column {x}");
        }
        assert_eq!(rgb(0, 44), 0x000000, "the border");
        assert_eq!(rgb(313, 45), 0x000000, "the border");
    }

    #[test]
    fn the_welcome_labels_sit_under_their_icons_and_underline_with_the_cues() {
        let Some(text) = tahoma() else {
            return;
        };
        let theme = Theme::default();
        let d = Dialog::turn_off(&power());
        let l = DialogLayout::compute(&theme, &text, &d);
        let red = Color::rgb(255, 0, 0);
        let mut icon = Canvas::new(32, 32);
        icon.fill_rect(icon.bounds(), red);
        let icons = [Some(icon.clone()), Some(icon.clone()), Some(icon), None];
        let drawn = |cues: Cues| {
            draw(
                &theme,
                &text,
                &l,
                None,
                None,
                &icons,
                &[],
                None,
                &d,
                true,
                None,
                cues,
                true,
                None,
            )
        };
        let ink = |c: &Canvas, r: Rect, hit: &dyn Fn(u32) -> bool| {
            let pts: Vec<(i32, i32)> = (r.y..r.bottom())
                .flat_map(|y| (r.x..r.right()).map(move |x| (x, y)))
                .filter(|&(x, y)| hit(c.get(x, y)))
                .collect();
            let xs = pts.iter().map(|p| p.0);
            let ys = pts.iter().map(|p| p.1);
            (xs.clone().min(), ys.clone().min(), xs.max(), ys.max())
        };
        let white = |p: u32| p == Color::rgb(255, 255, 255).argb();
        let label = |i: usize| {
            let r = l.buttons[i];
            Rect::new(r.x, r.y + 41, r.width, r.height - 41)
        };
        let plain = drawn(Cues::NONE);
        assert_eq!(
            ink(&plain, label(1), &white),
            (Some(133), Some(122), Some(179), Some(130)),
            "Turn Off without its underline"
        );
        assert_eq!(
            ink(&plain, label(2), &white),
            (Some(221), Some(123), Some(262), Some(130)),
            "Restart without its underline"
        );
        let cued = drawn(Cues::ALL);
        assert_eq!(
            ink(&cued, label(1), &white),
            (Some(133), Some(122), Some(179), Some(132)),
            "Turn Off with the u underlined on row 132"
        );
        let is_red = |p: u32| p == red.argb();
        let lefts: Vec<Option<i32>> = (0..3)
            .map(|i| ink(&plain, l.buttons[i], &is_red).0)
            .collect();
        assert_eq!(
            lefts,
            [Some(55), Some(141), Some(226)],
            "the icons' left columns"
        );
        assert_eq!(
            ink(&plain, l.buttons[1], &is_red).1,
            Some(81),
            "the icons' top row"
        );
        let black = |p: u32| p == theme.colors.button_text.argb();
        let cancel = l.buttons[3];
        assert_eq!(cancel, Rect::new(243, 168, 60, 20));
        for c in [&plain, &cued] {
            let (x0, y0, _, y1) = ink(c, cancel.inset(2), &black);
            assert_eq!(
                (x0, y0, y1),
                (Some(257), Some(173), Some(181)),
                "Cancel's text, never underlined"
            );
        }
    }

    fn welcome_drawn(d: &Dialog, lit: Lit<usize>, cues: Cues) -> (Canvas, DialogLayout) {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let mut icon = Canvas::new(32, 32);
        icon.fill_rect(icon.bounds(), Color::rgb(138, 82, 2));
        let icons: Vec<_> = d
            .buttons
            .iter()
            .map(|b| b.icon.is_some().then(|| icon.clone()))
            .collect();
        let l = DialogLayout::compute(&theme, &text, d);
        let c = draw(
            &theme,
            &text,
            &l,
            None,
            None,
            &icons,
            &[],
            None,
            d,
            true,
            lit,
            cues,
            true,
            None,
        );
        (c, l)
    }

    #[test]
    fn under_glass_the_welcome_panels_are_framed_dialogs() {
        let theme = reprisekit::interfaces::theme_for(reprisekit::interface::Interface::Glass);
        let text = TextRenderer::load(&theme.fonts);
        let orange = Color::rgb(138, 82, 2);
        let mut icon = Canvas::new(32, 32);
        icon.fill_rect(icon.bounds(), orange);
        for (d, icons) in [(Dialog::log_off(), 2), (Dialog::turn_off(&power()), 3)] {
            let l = DialogLayout::compute(&theme, &text, &d);
            let g = l.frame.as_ref().expect("a frame");
            let client = g.client;
            assert_eq!(client.width, GLASS_WELCOME_W, "{}", d.title);
            let row = &l.buttons[..icons];
            for r in row {
                assert_eq!((r.width, r.height), GLASS_WELCOME_CELL);
                assert_eq!(r.y, client.y + GLASS_WELCOME_TOP);
            }
            let (first, last) = (row[0], row[icons - 1]);
            assert_eq!(
                first.x - client.x,
                client.right() - last.right(),
                "{}: the row is centred",
                d.title
            );
            let cancel = l.buttons[icons];
            assert_eq!((cancel.width, cancel.height), (BUTTON_W, BUTTON_H));
            assert_eq!(client.right() - cancel.right(), GLASS_WELCOME_MARGIN);
            assert_eq!(client.bottom() - cancel.bottom(), GLASS_WELCOME_MARGIN);
            assert_eq!(cancel.y - last.bottom(), GLASS_WELCOME_TOP);
            assert!(
                DialogLayout::compute(&Theme::default(), &text, &d)
                    .frame
                    .is_none(),
                "Classic keeps the panel"
            );
            let pics: Vec<_> = d
                .buttons
                .iter()
                .map(|b| b.icon.is_some().then(|| icon.clone()))
                .collect();
            let c = draw(
                &theme,
                &text,
                &l,
                None,
                None,
                &pics,
                &[],
                None,
                &d,
                true,
                None,
                Cues::NONE,
                true,
                None,
            );
            for (i, r) in row.iter().enumerate() {
                let mid = c.get(r.x + r.width / 2, r.y + WELCOME_ICON_Y + ICON_PX / 2);
                if d.buttons[i].enabled {
                    assert_eq!(mid, orange.argb(), "{}: icon {i}", d.title);
                } else {
                    assert_ne!(mid, orange.argb(), "{}: icon {i} is faded", d.title);
                }
            }
            let st = theme.look.style().unwrap();
            let frame = |state| {
                let mut f = Canvas::new(GLASS_WELCOME_CELL.0 as u32, GLASS_WELCOME_CELL.1 as u32);
                f.fill_rect(f.bounds(), theme.colors.button_face);
                let r = Rect::new(0, 0, GLASS_WELCOME_CELL.0, GLASS_WELCOME_CELL.1);
                st.draw(&mut f, r, reprisekit::style::Part::PushButton, state, 1);
                f.get(GLASS_WELCOME_CELL.0 / 2, 1)
            };
            for (i, r) in row.iter().enumerate() {
                let b = &d.buttons[i];
                let want = if !b.enabled {
                    reprisekit::style::State::Disabled
                } else if b.default {
                    reprisekit::style::State::Default
                } else {
                    reprisekit::style::State::Normal
                };
                assert_eq!(
                    c.get(r.x + r.width / 2, r.y + 1),
                    frame(want),
                    "{}: button {i} in its {want:?} frame",
                    d.title
                );
            }
        }
    }

    fn welcome_icons(d: &Dialog, lit: Lit<usize>) -> Vec<u32> {
        let (c, l) = welcome_drawn(d, lit, Cues::NONE);
        let n = d.buttons.iter().filter(|b| b.icon.is_some()).count();
        l.buttons[..n]
            .iter()
            .map(|r| c.get(r.x + r.width / 2, r.y + WELCOME_ICON_Y + ICON_PX / 2))
            .collect()
    }

    #[test]
    fn the_hovered_and_focused_welcome_icons_draw_hot() {
        let cold = Color::rgb(138, 82, 2).argb();
        let hot = Color::rgb(163, 97, 2).argb();
        let log_off = Dialog::log_off();
        let faded = welcome_icons(&log_off, None)[0];
        assert_ne!(faded, cold);
        assert_eq!(
            welcome_icons(&log_off, None),
            [faded, hot],
            "Log Off opens with the focus"
        );
        assert_eq!(
            welcome_icons(&log_off, Some((0, Interaction::Hot))),
            [faded, hot],
            "the greyed Switch User stays faded under the pointer"
        );
        let turn_off = Dialog::turn_off(&power());
        assert_eq!(
            welcome_icons(&turn_off, None),
            [cold, hot, cold],
            "Turn Off opens with the focus"
        );
        assert_eq!(
            welcome_icons(&turn_off, Some((2, Interaction::Hot))),
            [cold, hot, hot],
            "Restart under the pointer, Turn Off still focused"
        );
        let mut d = Dialog::turn_off(&power());
        d.key(Key::Right);
        assert_eq!(
            welcome_icons(&d, None),
            [cold, cold, hot],
            "Restart focused"
        );
        assert_eq!(
            welcome_icons(&d, Some((1, Interaction::Hot))),
            [cold, hot, hot],
            "Turn Off under the pointer, Restart still focused"
        );
        d.key(Key::Right);
        assert_eq!(
            welcome_icons(&d, None),
            [cold, cold, cold],
            "Cancel focused"
        );
        assert_eq!(
            welcome_icons(&d, Some((0, Interaction::Hot))),
            [hot, cold, cold],
            "Stand By under the pointer, Cancel focused"
        );
        let mut d = Dialog::turn_off(&power());
        assert!(d.hover(Some(DialogHit::Button(2)), (0, 0), false));
        assert_eq!(d.lit(), Some((2, Interaction::Hot)));
        assert!(d.leave(false));
        assert_eq!(d.buttons.iter().position(|b| b.default), Some(1));
        assert!(!Dialog::run(&[]).hover(Some(DialogHit::Button(0)), (0, 0), false));
    }

    #[test]
    fn keys_walk_a_welcome_panels_focus_and_cancel_shows_it() {
        let focused = |d: &Dialog| d.buttons.iter().position(|b| b.default);
        let mut d = Dialog::log_off();
        assert_eq!(focused(&d), Some(1), "Log Off holds the focus");
        assert_eq!(d.key(Key::Left), Outcome::Nothing, "Switch User is greyed");
        assert_eq!(d.key(Key::Right), Outcome::Redraw);
        assert_eq!(focused(&d), Some(2), "Right goes on to Cancel");
        assert_eq!(d.key(Key::Right), Outcome::Nothing, "and stops there");
        assert_eq!(d.key(Key::Tab { back: false }), Outcome::Redraw);
        assert_eq!(
            focused(&d),
            Some(1),
            "Tab wraps past the greyed Switch User"
        );

        let mut d = Dialog::turn_off(&power());
        let order: Vec<_> = (0..4)
            .map(|_| {
                d.key(Key::Tab { back: false });
                focused(&d)
            })
            .collect();
        assert_eq!(order, [Some(2), Some(3), Some(0), Some(1)]);
        d.key(Key::Left);
        assert_eq!(focused(&d), Some(0), "Left from Turn Off to Stand By");

        let cancel = |d: &Dialog, cues: Cues| {
            let (c, l) = welcome_drawn(d, None, cues);
            let r = l.buttons[3];
            (c.get(r.x, r.y + r.height / 2), c.get(r.x + 4, r.y + 5))
        };
        let black = Color::rgb(0, 0, 0).argb();
        let face = Theme::default().colors.button_face.argb();
        let d = Dialog::turn_off(&power());
        assert_ne!(cancel(&d, Cues::ALL).0, black, "Cancel unfocused");
        assert_eq!(cancel(&d, Cues::ALL).1, face);
        let mut d = Dialog::turn_off(&power());
        d.key(Key::Tab { back: true });
        d.key(Key::Tab { back: true });
        assert_eq!(focused(&d), Some(3), "Shift+Tab wraps round to Cancel");
        assert_eq!(cancel(&d, Cues::NONE), (black, face), "focused, no cues");
        assert_eq!(cancel(&d, Cues::ALL), (black, black), "focused, with cues");
    }

    #[test]
    fn the_welcome_title_asks_for_franklin_gothic_medium_then_trebuchet_ms() {
        let spec = welcome_title_text();
        assert_eq!(
            spec.family[..2],
            ["Franklin Gothic Medium", "Trebuchet MS"],
            "Franklin Gothic Medium first, Trebuchet MS as its fallback"
        );
        assert_eq!(spec.family_sizes, [("Franklin Gothic Medium".into(), 20)]);
        assert_eq!(spec.size_px, 18, "the fallback's size");
        assert!(spec.bold && !spec.italic);
    }

    #[test]
    fn the_welcome_title_sets_in_franklin_gothic_medium_at_20_px() {
        let installed = WELCOME_TITLE.with(|t| t.has_face("Franklin Gothic Medium"));
        let Some(text) = tahoma().filter(|_| installed) else {
            return;
        };
        let theme = Theme::default();
        let d = Dialog::turn_off(&power());
        let l = DialogLayout::compute(&theme, &text, &d);
        let c = draw(
            &theme,
            &text,
            &l,
            None,
            None,
            &[None, None, None, None],
            &[],
            None,
            &d,
            true,
            None,
            Cues::ALL,
            true,
            None,
        );
        let band = PANEL_BAND.argb();
        let ink: Vec<(i32, i32)> = (1..44)
            .flat_map(|y| (1..250).map(move |x| (x, y)))
            .filter(|&(x, y)| c.get(x, y) != band)
            .collect();
        let xs = ink.iter().map(|p| p.0);
        assert_eq!((xs.clone().min(), xs.max()), (Some(13), Some(164)));
        // The T's crossbar is the title's first full row, and the last row
        // with more than the p's descender is the baseline's.
        let row = |y: i32| ink.iter().filter(|p| p.1 == y).count();
        let rows: Vec<i32> = (1..44).filter(|&y| row(y) > 4).collect();
        assert_eq!((rows.first(), rows.last()), (Some(&15), Some(&28)));
    }

    #[test]
    fn an_inactive_dialog_has_a_grey_caption_and_no_default_ring() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let d = Dialog::run(&[]);
        let l = DialogLayout::compute(&theme, &text, &d);
        let c = draw(
            &theme,
            &text,
            &l,
            None,
            None,
            &[],
            &[],
            None,
            &d,
            false,
            None,
            Cues::ALL,
            false,
            None,
        );
        assert_eq!(
            c.get(l.caption.x, l.caption.bottom() - 2),
            Color::rgb(128, 128, 128).argb()
        );
        let ok = l.buttons[0];
        assert_eq!(
            c.get(ok.x, ok.y + ok.height / 2),
            theme.colors.button_hilight.argb(),
            "OK's left edge is the hilight, not the black ring"
        );
    }

    #[test]
    fn default_button_has_a_black_ring_and_field_shows_a_caret() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let mut d = Dialog::run(&[]);
        d.key(Key::Edit(EditKey::Insert("ab".into())));
        let l = DialogLayout::compute(&theme, &text, &d);
        let c = draw(
            &theme,
            &text,
            &l,
            None,
            None,
            &[],
            &[],
            None,
            &d,
            true,
            None,
            Cues::ALL,
            true,
            None,
        );
        let black = theme.colors.button_text.argb();
        let ok = l.buttons[0];
        assert_eq!(c.get(ok.x, ok.y + ok.height / 2), black);
        assert_eq!(
            c.get(ok.x + 1, ok.y + ok.height / 2),
            theme.colors.button_hilight.argb()
        );
        let cancel = l.buttons[1];
        assert_eq!(
            c.get(cancel.x, cancel.y + cancel.height / 2),
            theme.colors.button_hilight.argb()
        );
        let f = l.field.unwrap();
        let inner = f.inset(3);
        let caret_x = f.x + combo::EDIT_TEXT_X + text.measure("ab", false, theme.fonts.size_px);
        assert_eq!(
            c.get(caret_x, inner.y + inner.height / 2),
            theme.colors.button_text.argb()
        );
        assert_eq!(
            c.get(l.drop.unwrap().x - 3, inner.y + 2),
            theme.colors.window.argb()
        );
    }

    /// A blink repaints the field over the caret-off raster; the result
    /// is the caret-on raster, pixel for pixel, and painting the field
    /// again with the caret off gets the caret-off one back.
    #[test]
    fn a_blink_over_the_kept_raster_matches_a_full_draw() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let mut d = Dialog::run(&history());
        d.key(Key::Edit(EditKey::Insert("notepad".into())));
        let l = DialogLayout::compute(&theme, &text, &d);
        let full = |caret: bool| {
            draw(
                &theme,
                &text,
                &l,
                None,
                None,
                &[],
                &[],
                None,
                &d,
                caret,
                None,
                Cues::ALL,
                true,
                None,
            )
        };
        let off = full(false);
        let on = full(true);
        assert_ne!(off, on, "the caret shows");
        let mut c = off.clone();
        draw_field(&mut c, &theme, &text, &l, &d, true);
        assert_eq!(c, on);
        draw_field(&mut c, &theme, &text, &l, &d, false);
        assert_eq!(c, off);
    }

    fn history() -> Vec<String> {
        ["notepad", "mspaint", "calc", "cmd", "regedit"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    /// The open drop list's bottom frame pixel sits under the third row:
    /// it is no row, rather than the hidden fourth one.
    #[test]
    fn the_run_lists_bottom_frame_row_hits_no_row() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let px = theme.fonts.size_px;
        let mut d = Dialog::run(&history());
        d.key(Key::Drop);
        let l = DialogLayout::compute(&theme, &text, &d);
        let list = l.list.unwrap();
        assert_eq!(
            l.hit(&d, list.x + 5, list.bottom() - 2, &text, px),
            Some(DialogHit::Row(2))
        );
        assert_eq!(l.hit(&d, list.x + 5, list.bottom() - 1, &text, px), None);
    }

    #[test]
    fn run_opens_on_the_last_command_selected_and_typing_replaces_it() {
        let mut d = Dialog::run(&history());
        let f = d.field.as_ref().unwrap();
        assert_eq!(f.edit().unwrap().text, "notepad");
        assert!(f.edit().unwrap().all_selected());
        assert_eq!(
            d.key(Key::Edit(EditKey::Insert("x".into()))),
            Outcome::Redraw
        );
        let f = d.field.as_ref().unwrap();
        assert_eq!(
            (
                f.edit().unwrap().text.as_str(),
                f.edit().unwrap().has_selection()
            ),
            ("x", false)
        );
        let mut d = Dialog::run(&history());
        assert_eq!(d.key(Key::Backspace), Outcome::Redraw);
        assert_eq!(d.field.as_ref().unwrap().combo.text(), "");
        let mut d = Dialog::run(&history());
        assert_eq!(d.key(Key::Enter), Outcome::Run("notepad".into()));
    }

    #[test]
    fn up_and_down_walk_the_history_and_f4_opens_the_list() {
        let mut d = Dialog::run(&history());
        let value = |d: &Dialog| d.field.as_ref().unwrap().combo.text().to_string();
        assert_eq!(d.key(Key::Up), Outcome::Nothing, "newest already");
        d.key(Key::Down);
        assert_eq!(value(&d), "mspaint");
        d.key(Key::Up);
        assert_eq!(value(&d), "notepad");
        d.key(Key::Edit(EditKey::Insert("z".into())));
        d.key(Key::Down);
        assert_eq!(value(&d), "notepad");
        assert_eq!(d.key(Key::Drop), Outcome::Redraw);
        let field = |d: &Dialog| d.field.clone().unwrap();
        assert_eq!((field(&d).combo.open, field(&d).combo.top), (Some(0), 0));
        for _ in 0..3 {
            d.key(Key::Down);
        }
        assert_eq!((field(&d).combo.open, field(&d).combo.top), (Some(3), 1));
        assert_eq!(value(&d), "cmd");
        assert_eq!(d.key(Key::Enter), Outcome::Redraw);
        assert_eq!(field(&d).combo.open, None);
        assert!(field(&d).edit().unwrap().all_selected());
        d.key(Key::Drop);
        assert_eq!(d.key(Key::Escape), Outcome::Redraw);
        assert_eq!(d.key(Key::Escape), Outcome::Close);
        let mut plain = Dialog::run(&[]);
        assert_eq!(plain.key(Key::Drop), Outcome::Nothing);
        assert_eq!(plain.key(Key::Down), Outcome::Nothing);
    }

    #[test]
    fn welcome_buttons_tip_their_description_but_not_when_greyed() {
        let d = Dialog::turn_off(&power());
        let (title, body) = d.tip(1).unwrap();
        assert_eq!(title, "Turn Off");
        assert!(body.starts_with("Shuts down Reprise"));
        assert!(d.tip(3).is_none(), "Cancel has no tip");
        let l = Dialog::log_off();
        assert!(l.tip(0).is_none(), "greyed Switch User has none");
        assert!(l.tip(1).is_some());
        assert!(Dialog::run(&[]).tip(0).is_none());
    }

    #[test]
    fn keys_move_the_focus_along_a_message_boxs_buttons() {
        let focused = |d: &Dialog| d.buttons.iter().position(|b| b.default);
        let mut d = Dialog::end_program("foot", 4321);
        assert_eq!(focused(&d), Some(1), "Cancel has the focus at first");
        assert_eq!(d.key(Key::Tab { back: false }), Outcome::Redraw);
        assert_eq!(focused(&d), Some(0), "Tab wraps round to End Now");
        assert_eq!(d.key(Key::Tab { back: true }), Outcome::Redraw);
        assert_eq!(focused(&d), Some(1), "Shift+Tab wraps back to Cancel");
        assert_eq!(d.key(Key::Right), Outcome::Nothing, "Right stops at Cancel");
        assert_eq!(d.key(Key::Down), Outcome::Nothing, "Down stops at Cancel");
        assert_eq!(d.key(Key::Left), Outcome::Redraw);
        assert_eq!(focused(&d), Some(0));
        assert_eq!(d.key(Key::Up), Outcome::Nothing, "Up stops at End Now");
        // The view sends Left and Right as editing keys.
        let right = EditKey::Right {
            select: false,
            word: false,
        };
        assert_eq!(d.key(Key::Edit(right)), Outcome::Redraw);
        assert_eq!(focused(&d), Some(1), "Right moves on to Cancel");
        let left = EditKey::Left {
            select: false,
            word: false,
        };
        assert_eq!(d.key(Key::Edit(left)), Outcome::Redraw);
        assert_eq!(focused(&d), Some(0), "Left moves back to End Now");
        assert_eq!(d.key(Key::Enter), Outcome::EndProgram(4321));

        let mut three = Dialog::error("Title", "Text".into());
        three.buttons = vec![
            button("&Yes", DialogAction::Cancel, true),
            button("&No", DialogAction::Cancel, false),
            button("Cancel", DialogAction::Cancel, false),
        ];
        three.buttons[1].enabled = false;
        three.key(Key::Tab { back: false });
        assert_eq!(focused(&three), Some(2), "Tab skips the greyed No");
        three.key(Key::Left);
        assert_eq!(focused(&three), Some(0), "Left skips the greyed No");
    }

    #[test]
    fn space_presses_a_message_boxs_focused_button() {
        let space = || Key::Edit(EditKey::Insert(" ".into()));
        let mut d = Dialog::end_program("foot", 4321);
        assert_eq!(d.key(space()), Outcome::Close, "Space presses Cancel");
        let mut d = Dialog::end_program("foot", 4321);
        d.key(Key::Tab { back: false });
        assert_eq!(d.key(space()), Outcome::EndProgram(4321));
        let mut d = Dialog::end_program("foot", 4321);
        d.key(Key::Tab { back: false });
        assert_eq!(d.key(Key::Space), Outcome::EndProgram(4321));

        let mut run = Dialog::run(&[]);
        assert_eq!(run.key(space()), Outcome::Redraw);
        let field = run.field.as_ref().and_then(Field::edit);
        assert_eq!(field.map(|e| e.text.as_str()), Some(" "));
    }

    #[test]
    fn end_program_lays_out_its_template() {
        let theme = Theme::default();
        let Some(text) = tahoma() else { return };
        let mut d = Dialog::end_program("signal-desktop", 4321);
        assert_eq!(d.title, "End Program - signal-desktop");
        let l = DialogLayout::compute(&theme, &text, &d);
        assert_eq!(l.size, (354, 243));
        assert_eq!(l.icon.unwrap(), Rect::new(13, 32, 32, 32));
        let tops: Vec<i32> = l.lines.iter().map(|(r, _)| r.y).collect();
        assert_eq!(tops, vec![35, 74, 87, 120, 133]);
        assert!(l.lines.iter().all(|(r, _)| r.x == 58 && r.right() == 336));
        assert_eq!(l.lines[0].1, "This program has stopped responding.");
        assert_eq!(
            l.buttons,
            vec![Rect::new(188, 204, 69, 23), Rect::new(266, 204, 69, 23),]
        );
        assert_eq!(d.key(Key::Enter), Outcome::Close);
        assert_eq!(d.key(Key::Escape), Outcome::Close);
        assert_eq!(d.press(0), Outcome::EndProgram(4321));
    }

    #[test]
    fn end_program_and_windowed_dialogs_have_a_taskbar_button() {
        assert!(Dialog::end_program("foot", 4321).on_taskbar());
        assert!(!Dialog::end_program("foot", 4321).windowed());
        assert!(!Dialog::run(&[]).on_taskbar());
        assert!(!Dialog::log_off().on_taskbar());
        assert!(!Dialog::error("Title", "Text".into()).on_taskbar());
        assert!(!Dialog::run(&[]).windowed());
        assert!(!Dialog::error("Title", "Text".into()).windowed());
    }

    #[test]
    fn only_end_program_names_a_pid() {
        assert_eq!(
            Dialog::end_program("foot", 4321).end_program_pid(),
            Some(4321)
        );
        assert_eq!(Dialog::run(&[]).end_program_pid(), None);
        assert_eq!(Dialog::log_off().end_program_pid(), None);
    }

    /// The advances a second dialog font gives at 11 px where they differ
    /// from the UI font's, for tests that stand a stub in for it: a wider
    /// E and N than the default UI face's.
    fn shell_dlg_advance(c: char) -> i32 {
        match c {
            'E' => 7,
            'N' | 'w' => 8,
            ' ' | 'i' | 'l' | '.' | ',' => 3,
            _ => 6,
        }
    }

    #[test]
    fn end_program_sets_its_text_and_buttons_in_the_second_dialog_font() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default())
            .with_shell_dlg(TextRenderer::stub(11, shell_dlg_advance));
        let d = Dialog::end_program("signal-desktop", 4321);
        assert_eq!(d.font(), DialogFont::ShellDlg);
        let l = DialogLayout::compute(&theme, &text, &d);
        assert_eq!(l.size, (354, 243));
        let second = text.dialog(DialogFont::ShellDlg);
        assert_eq!(second.measure("End Now", false, 11), 44);
        let width = 354 - 18 - 58;
        let want = wrap(second, &d.text, width, 11);
        let got: Vec<&str> = l.lines.iter().map(|(_, s)| s.as_str()).collect();
        assert_eq!(
            got,
            want.iter()
                .filter(|s| !s.is_empty())
                .map(String::as_str)
                .collect::<Vec<_>>()
        );
        assert!(l.lines.iter().all(|(r, _)| r.x == 57), "{:?}", l.lines);
        let ui = DialogLayout::compute(&theme, &TextRenderer::load(&FontConfig::default()), &d);
        assert!(ui.lines.iter().all(|(r, _)| r.x == 58), "{:?}", ui.lines);
        assert_eq!(l.buttons[0], Rect::new(188, 204, 69, 23));
        let c = draw(
            &theme,
            &text,
            &l,
            None,
            None,
            &[],
            &[],
            None,
            &d,
            false,
            None,
            Cues::NONE,
            true,
            None,
        );
        let b = l.buttons[0].inset(3);
        let pens: Vec<i32> = (b.x..b.right())
            .filter(|&x| (b.y..b.bottom()).any(|y| c.get(x, y) == 0xFF00_0000))
            .map(|x| x - l.buttons[0].x)
            .collect();
        assert_eq!(pens, vec![12, 19, 25, 34, 42, 48]);
    }

    #[test]
    fn advanced_attributes_hands_its_boxes_to_the_sheet() {
        use reprisekit::controls::filesheet::ADVANCED;
        let (on, off) = (CheckState::On, CheckState::Off);
        let text = TextRenderer::load(&FontConfig::default());
        let px = Theme::default().fonts.size_px;
        let mut info = file_info();
        info.robots = Some([true, true]);
        let mut d = Dialog::properties(&info, "text-x-generic", (0, 0));
        assert_eq!(
            press_control(&mut d, ADVANCED),
            Outcome::AdvancedAttributes {
                subject: Subject::File,
                robots: Some([on, on]),
            }
        );
        let mut adv =
            Dialog::advanced_attributes(Subject::File, Some([on, on]), &text, px, TextUnits::ONE);
        adv.key(Key::Space);
        assert_eq!(adv.key(Key::Enter), Outcome::AdvancedChosen([off, on]));
        assert!(d.advanced_chosen([off, on]));
        let apply = |d: &Dialog| {
            matches!(
                sheet_form(d)
                    .item(reprisekit::controls::propsheet::APPLY)
                    .map(|i| &i.control),
                Some(reprisekit::controls::form::Control::Button { enabled: true, .. })
            )
        };
        assert!(apply(&d), "Apply lights");
        assert_eq!(
            press_control(&mut d, reprisekit::controls::propsheet::OK),
            Outcome::Properties {
                path: info.path.clone(),
                name: None,
                attrs: reprisefs::props::AttrMoves {
                    backup: Some(false),
                    ..Default::default()
                },
                hidden: None,
                folder_icon: None,
                summary: Vec::new(),
                launcher: Vec::new(),
                close: true,
                recurse: None,
            }
        );
        d.sheet_applied(&written(&d.clone().key(Key::Enter)));
        assert!(!apply(&d), "written, Apply greys");
    }

    #[test]
    fn advanced_attributes_frames_the_shared_box() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let px = theme.fonts.size_px;
        let d = Dialog::advanced_attributes(Subject::File, None, &text, px, TextUnits::ONE);
        assert_eq!(d.title, "Advanced Attributes");
        let l = DialogLayout::compute(&theme, &text, &d);
        assert_eq!(l.size, (384, 287));
        assert_eq!(l.icon, Some(Rect::new(14, 33, 32, 32)));
        let folder = Dialog::advanced_attributes(Subject::Folder, None, &text, px, TextUnits::ONE);
        assert_eq!(
            DialogLayout::compute(&theme, &text, &folder).size,
            (384, 309)
        );
        assert_eq!(
            d.clone().click(l.hit(&d, 30, 110, &text, px)),
            Outcome::Nothing
        );
        assert_eq!(d.clone().key(Key::Enter), Outcome::Close);
        let mut ok = d.clone();
        let hit = l.hit(&ok, 250, 260, &text, px);
        assert!(
            matches!(hit, Some(DialogHit::Control(_))),
            "OK is a control"
        );
        ok.click(hit);
        assert_eq!(ok.release(None), Outcome::Close);
    }

    /// Under Glass a form dialog's default button draws as a plain one
    /// while the dialog is inactive: a 707070 top line, not 3C7EB1.
    #[test]
    fn an_inactive_glass_form_dialog_draws_its_default_button_plain() {
        use reprisekit::controls::form::Control;
        let theme = reprisekit::interfaces::theme_for(reprisekit::interface::Interface::Glass);
        let text = TextRenderer::load(&FontConfig::default());
        let d = Dialog::advanced_attributes(
            Subject::File,
            None,
            &text,
            theme.fonts.size_px,
            TextUnits::ONE,
        );
        let l = DialogLayout::compute(&theme, &text, &d);
        let ok = d
            .form()
            .and_then(|f| f.open_page())
            .and_then(|form| {
                let it = form
                    .items
                    .iter()
                    .find(|it| matches!(it.control, Control::Button { default: true, .. }))?;
                form.rect(it.id)
            })
            .expect("a default button");
        let top = |active| {
            let c = draw(
                &theme,
                &text,
                &l,
                None,
                None,
                &[],
                &[],
                None,
                &d,
                false,
                None,
                Cues::NONE,
                active,
                None,
            );
            c.get(ok.x + 10, ok.y + 1)
        };
        assert_eq!(top(true), 0xFF3B_76AD, "active, the default frame");
        assert_eq!(top(false), 0xFF6D_6D6D, "inactive, a plain button");
    }

    #[test]
    fn an_inactive_glass_chooser_draws_its_ok_plain() {
        let theme = reprisekit::interfaces::theme_for(reprisekit::interface::Interface::Glass);
        let text = TextRenderer::load(&FontConfig::default());
        let px = theme.fonts.size_px;
        let d = chooser();
        let l = DialogLayout::compute(&theme, &text, &d);
        let client = l.page.expect("the chooser's client");
        let ok = d
            .chooser()
            .unwrap()
            .ow
            .layout(&text, px, (client.x, client.y), &l.controls)
            .ok;
        let top = |active| {
            let c = draw(
                &theme,
                &text,
                &l,
                None,
                None,
                &[],
                &[],
                None,
                &d,
                false,
                None,
                Cues::NONE,
                active,
                None,
            );
            c.get(ok.x + 10, ok.y + 1)
        };
        assert_eq!(top(true), 0xFF3B_76AD, "active, the default frame");
        assert_eq!(top(false), 0xFF6D_6D6D, "inactive, a plain button");
    }

    fn file_info() -> reprisefs::props::FileInfo {
        reprisefs::props::FileInfo {
            name: "New Text Document".into(),
            hidden_ext: ".txt".into(),
            path: "/home/u/Desktop/New Text Document.txt".into(),
            is_dir: false,
            type_name: "Plain text document".into(),
            opens_with: Some("Micro".into()),
            opens_with_icon: Some("text-editor".into()),
            location: "/home/u/Desktop".into(),
            size: 0,
            on_disk: 0,
            contains: None,
            created: Some(1_788_998_400),
            modified: 1_788_998_400,
            accessed: 1_788_998_400,
            read_only: false,
            executable: false,
            can_set_executable: true,
            hidden: false,
            listed: false,
            is_shortcut: false,
            robots: None,
            summary: None,
            own_icon: None,
            launcher: None,
            description: None,
        }
    }

    fn written(o: &Outcome) -> Written {
        let Outcome::Properties {
            path,
            name,
            attrs,
            hidden,
            summary,
            folder_icon,
            launcher,
            ..
        } = o.clone()
        else {
            panic!("a Properties write, not {o:?}");
        };
        Written {
            path: name.map_or(path.clone(), |n| path.with_file_name(n)),
            attrs,
            hidden,
            summary,
            folder_icon,
            launcher,
            reread: None,
        }
    }

    fn sheet_form(d: &Dialog) -> &reprisekit::controls::form::Form {
        d.form().and_then(|f| f.open_page()).expect("a sheet page")
    }

    fn sheet_form_mut(d: &mut Dialog) -> &mut reprisekit::controls::form::Form {
        d.form_mut()
            .and_then(|f| f.open_page_mut())
            .expect("a sheet page")
    }

    fn set_name(d: &mut Dialog, name: &str) {
        use reprisekit::controls::form::Control;
        if let Some(Control::Edit { edit, .. }) = sheet_form_mut(d)
            .item_mut(reprisekit::controls::filesheet::NAME)
            .map(|i| &mut i.control)
        {
            *edit = TextEdit::new(name, false);
        }
    }

    fn press_control(d: &mut Dialog, id: &str) -> Outcome {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let r = sheet_form(d).rect(id).expect(id);
        let l = DialogLayout::compute(&theme, &text, d);
        let hit = l.hit(d, r.x + 3, r.y + 3, &text, theme.fonts.size_px);
        let out = d.click(hit);
        match d.release(None) {
            Outcome::Nothing => out,
            released => released,
        }
    }

    #[test]
    fn properties_sheet_frames_the_general_page() {
        use reprisekit::controls::filesheet::{ADVANCED, CHANGE, NAME};
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let d = Dialog::properties(&file_info(), "text-x-generic", (600, 500));
        assert_eq!(d.title, "New Text Document Properties");
        let l = DialogLayout::compute(&theme, &text, &d);
        assert_eq!(l.size, (367, 502));
        assert_eq!(l.icon, Some(Rect::new(24, 62, 32, 32)));
        let page = l.page.unwrap();
        assert_eq!((page.x, page.y, page.right()), (9, 49, 358));
        let form = sheet_form(&d);
        assert_eq!(form.rect(NAME).unwrap(), Rect::new(97, 69, 246, 23));
        assert_eq!(
            reprisekit::controls::filesheet::typed_name(form),
            "New Text Document"
        );
        assert_eq!(
            d.clone().key(Key::Enter),
            Outcome::Close,
            "nothing to write"
        );
        let mut d2 = d.clone();
        set_name(&mut d2, "Notes");
        assert_eq!(
            d2.key(Key::Enter),
            Outcome::Properties {
                path: "/home/u/Desktop/New Text Document.txt".into(),
                name: Some("Notes.txt".into()),
                attrs: Default::default(),
                hidden: None,
                folder_icon: None,
                summary: Vec::new(),
                launcher: Vec::new(),
                close: true,
                recurse: None,
            }
        );
        let mut d3 = d.clone();
        assert_eq!(
            press_control(&mut d3, CHANGE),
            Outcome::ChangeDefault("/home/u/Desktop/New Text Document.txt".into())
        );
        assert_eq!(
            press_control(&mut d3, ADVANCED),
            Outcome::AdvancedAttributes {
                subject: Subject::File,
                robots: None,
            }
        );
        let mut folder = file_info();
        folder.is_dir = true;
        folder.opens_with = None;
        folder.contains = Some((2, 1));
        let mut fd = Dialog::properties(&folder, "folder", (0, 0));
        assert_eq!(
            press_control(&mut fd, ADVANCED),
            Outcome::AdvancedAttributes {
                subject: Subject::Folder,
                robots: None,
            },
            "a folder's Advanced... asks for the folder box"
        );
    }

    #[test]
    fn folder_sheet_reads_calculating_until_its_totals_arrive() {
        use reprisekit::controls::form::Control;
        let mut folder = file_info();
        folder.is_dir = true;
        folder.opens_with = None;
        folder.type_name = "File Folder".into();
        let mut d = Dialog::properties(&folder, "folder", (0, 0));
        let values = |d: &Dialog| -> Vec<String> {
            ["v_size", "v_disk", "v_has"]
                .iter()
                .map(|id| match sheet_form(d).item(id).map(|i| &i.control) {
                    Some(Control::Label { text, .. }) => text.clone(),
                    other => panic!("{other:?}"),
                })
                .collect()
        };
        assert_eq!(values(&d), vec!["Calculating..."; 3]);
        let totals = reprisefs::props::FolderTotals {
            files: 2,
            folders: 1,
            size: 1536,
            on_disk: 8192,
        };
        assert!(d.set_folder_totals(totals));
        assert_eq!(
            values(&d),
            vec![
                reprisefs::props::format_size(1536),
                reprisefs::props::format_size(8192),
                "2 Files, 1 Folder".to_string(),
            ]
        );
    }

    #[test]
    fn the_executable_box_writes_the_execute_bit_and_greys_when_it_cannot() {
        use reprisekit::controls::filesheet::EXECUTABLE;
        use reprisekit::controls::propsheet::APPLY;
        let path: std::path::PathBuf = "/home/u/Desktop/New Text Document.txt".into();
        let exec = |on: bool| reprisefs::props::AttrMoves {
            executable: Some(on),
            ..Default::default()
        };
        let mut d = Dialog::properties(&file_info(), "text-x-generic", (0, 0));
        assert_eq!(press_control(&mut d, EXECUTABLE), Outcome::Redraw);
        assert_eq!(
            press_control(&mut d, APPLY),
            Outcome::Properties {
                path: path.clone(),
                name: None,
                attrs: exec(true),
                hidden: None,
                folder_icon: None,
                summary: Vec::new(),
                launcher: Vec::new(),
                close: false,
                recurse: None,
            }
        );
        d.sheet_applied(&Written {
            path: path.clone(),
            attrs: exec(true),
            ..Default::default()
        });
        press_control(&mut d, EXECUTABLE);
        assert!(
            matches!(press_control(&mut d, APPLY), Outcome::Properties { attrs, .. } if attrs == exec(false)),
            "the written state is the new start"
        );
        let mut theirs = file_info();
        (theirs.executable, theirs.can_set_executable) = (true, false);
        let mut d = Dialog::properties(&theirs, "text-x-generic", (0, 0));
        assert!(reprisekit::controls::filesheet::executable(sheet_form(&d)));
        press_control(&mut d, EXECUTABLE);
        assert!(
            reprisekit::controls::filesheet::executable(sheet_form(&d)),
            "a greyed box takes no click"
        );
    }

    #[test]
    fn attribute_boxes_toggle_and_light_apply() {
        use reprisekit::controls::filesheet::{HIDDEN, READ_ONLY};
        use reprisekit::controls::propsheet::{APPLY, OK};
        let apply_on = |d: &Dialog| {
            matches!(
                sheet_form(d).item(APPLY).map(|i| &i.control),
                Some(reprisekit::controls::form::Control::Button { enabled: true, .. })
            )
        };
        let mut d = Dialog::properties(&file_info(), "text-x-generic", (0, 0));
        assert!(!apply_on(&d), "Apply starts greyed");
        assert_eq!(press_control(&mut d, READ_ONLY), Outcome::Redraw);
        assert!(apply_on(&d), "Apply is lit");
        assert_eq!(
            press_control(&mut d, APPLY),
            Outcome::Properties {
                path: "/home/u/Desktop/New Text Document.txt".into(),
                name: None,
                attrs: reprisefs::props::AttrMoves {
                    read_only: Some(true),
                    ..Default::default()
                },
                hidden: None,
                folder_icon: None,
                summary: Vec::new(),
                launcher: Vec::new(),
                close: false,
                recurse: None,
            }
        );
        d.sheet_applied(&Written {
            path: "/home/u/Desktop/New Text Document.txt".into(),
            attrs: reprisefs::props::AttrMoves {
                read_only: Some(true),
                ..Default::default()
            },
            ..Default::default()
        });
        assert!(!apply_on(&d));
        press_control(&mut d, HIDDEN);
        assert_eq!(
            press_control(&mut d, OK),
            Outcome::Properties {
                path: "/home/u/Desktop/New Text Document.txt".into(),
                name: Some(".New Text Document.txt".into()),
                attrs: Default::default(),
                hidden: Some(true),
                folder_icon: None,
                summary: Vec::new(),
                launcher: Vec::new(),
                close: true,
                recurse: None,
            }
        );
        // A file its folder's .hidden lists reads hidden and keeps its
        // name: clearing the box writes Hidden alone, ticking it again
        // writes nothing.
        let mut listed = file_info();
        (listed.hidden, listed.listed) = (true, true);
        let mut d = Dialog::properties(&listed, "text-x-generic", (0, 0));
        assert!(reprisekit::controls::filesheet::hidden(sheet_form(&d)));
        press_control(&mut d, HIDDEN);
        assert_eq!(
            press_control(&mut d, APPLY),
            Outcome::Properties {
                path: "/home/u/Desktop/New Text Document.txt".into(),
                name: None,
                attrs: Default::default(),
                hidden: Some(false),
                folder_icon: None,
                summary: Vec::new(),
                launcher: Vec::new(),
                close: false,
                recurse: None,
            }
        );
        press_control(&mut d, HIDDEN);
        assert_eq!(press_control(&mut d, OK), Outcome::Close);
        let mut folder = file_info();
        folder.is_dir = true;
        folder.opens_with = None;
        folder.contains = Some((2, 1));
        let mut fd = Dialog::properties(&folder, "folder", (0, 0));
        let changes = |d: &Dialog| match &d.form().unwrap().kind {
            FormKind::Properties(b) => props_changes(sheet_form(d), b),
            _ => unreachable!(),
        };
        assert_eq!(changes(&fd), (None, Default::default(), None));
        press_control(&mut fd, READ_ONLY);
        assert_eq!(changes(&fd).1.read_only, Some(false));
        assert!(apply_on(&fd));
        press_control(&mut fd, READ_ONLY);
        assert_eq!(changes(&fd).1.read_only, Some(true));
        press_control(&mut fd, READ_ONLY);
        assert_eq!(changes(&fd).1.read_only, None, "back where it started");
        assert!(!apply_on(&fd));
    }

    #[test]
    fn sheet_keeps_spaces_around_an_unedited_name() {
        let mut info = file_info();
        info.name = " notes ".into();
        info.hidden_ext = String::new();
        info.path = "/home/u/Desktop/ notes ".into();
        let d = Dialog::properties(&info, "text-x-generic", (0, 0));
        assert_eq!(d.clone().key(Key::Enter), Outcome::Close, "OK only closes");
        let mut d = Dialog::properties(&file_info(), "text-x-generic", (0, 0));
        set_name(&mut d, "   ");
        assert_eq!(d.key(Key::Enter), Outcome::Close, "a blank name is ignored");
    }

    #[test]
    fn properties_sheet_draws_the_opens_with_icon() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let mut info = file_info();
        info.read_only = true;
        let d = Dialog::properties(&info, "text-x-generic", (0, 0));
        let l = DialogLayout::compute(&theme, &text, &d);
        let mut app_icon = Canvas::new(16, 16);
        app_icon.fill_rect(Rect::new(0, 0, 16, 16), theme.colors.hilight);
        let c = draw(
            &theme,
            &text,
            &l,
            None,
            None,
            &[],
            &[Some(&app_icon)],
            None,
            &d,
            true,
            None,
            Cues::ALL,
            true,
            None,
        );
        let cs = &theme.colors;
        let ir = sheet_form(&d).rect("opens_icon").unwrap();
        assert_eq!(ir, Rect::new(98, 142, 16, 16));
        assert_eq!(c.get(ir.x, ir.y), cs.hilight.argb());
        assert_eq!(c.get(ir.right() - 1, ir.bottom() - 1), cs.hilight.argb());
        assert_eq!(c.get(ir.x - 1, ir.y), cs.button_face.argb());
        assert_eq!(c.get(ir.right(), ir.bottom()), cs.button_face.argb());
        let ro = sheet_form(&d)
            .rect(reprisekit::controls::filesheet::READ_ONLY)
            .unwrap();
        assert_eq!(c.get(ro.x + 6, ro.y + 8), cs.button_text.argb());
    }

    fn chooser() -> Dialog {
        let app = |i: usize, n: &str| Program {
            id: format!("app{i}.desktop"),
            name: n.into(),
            icon: Some("text-x-generic".into()),
        };
        let others: Vec<_> = (0..12)
            .map(|i| app(10 + i, &format!("Program {i:02}")))
            .collect();
        Dialog::open_with(
            std::path::Path::new("/home/u/Desktop/New Text Document.txt"),
            Some("text/plain".into()),
            vec![app(1, "Micro"), app(2, "Vim")],
            others,
            None,
            false,
        )
    }

    fn attr_confirm() -> Dialog {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        Dialog::confirm_attributes(
            AttrConfirm {
                write: AttrWrite::One {
                    path: "/home/u/Desktop/New Folder".into(),
                    name: None,
                    hidden: None,
                    folder_icon: None,
                },
                attrs: reprisefs::props::AttrMoves {
                    read_only: Some(true),
                    ..Default::default()
                },
                close: false,
            },
            &text,
            theme.fonts.size_px,
            TextUnits::ONE,
        )
    }

    #[test]
    fn a_folders_hidden_asks_how_far_it_reaches() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let dir = std::path::Path::new("/d/Reports");
        let confirm = |read_only| AttrConfirm {
            write: AttrWrite::One {
                path: dir.into(),
                name: Some(".Reports".into()),
                hidden: Some(true),
                folder_icon: None,
            },
            attrs: reprisefs::props::AttrMoves {
                read_only,
                ..Default::default()
            },
            close: true,
        };
        let verb = |read_only| {
            let d = Dialog::confirm_attributes(
                confirm(read_only),
                &text,
                theme.fonts.size_px,
                TextUnits::ONE,
            );
            match &d
                .form()
                .unwrap()
                .open_page()
                .unwrap()
                .item("attr_verb")
                .unwrap()
                .control
            {
                reprisekit::controls::form::Control::Label { text, .. } => text.clone(),
                c => panic!("{c:?}"),
            }
        };
        assert_eq!(verb(None), "hide");
        assert_eq!(verb(Some(true)), "set read-only, hide");
        assert_eq!(
            confirm(None).answered(true),
            Outcome::Properties {
                path: dir.into(),
                name: Some(".Reports".into()),
                attrs: Default::default(),
                hidden: Some(true),
                folder_icon: None,
                summary: Vec::new(),
                launcher: Vec::new(),
                close: true,
                recurse: Some(true),
            }
        );
    }

    #[test]
    fn attribute_confirm_lays_out_its_template() {
        use reprisekit::controls::filesheet::{ATTR_ONLY, ATTR_TREE};
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let px = theme.fonts.size_px;
        let mut d = attr_confirm();
        assert_eq!(d.title, "Confirm Attribute Changes");
        let l = DialogLayout::compute(&theme, &text, &d);
        assert_eq!(l.size, (392, 240));
        let form = d.form().unwrap().open_page().unwrap();
        let at = |id: &str| form.rect(id).unwrap();
        assert_eq!((at("attr_intro").x, at("attr_intro").y), (14, 33));
        assert_eq!((at("attr_verb").x, at("attr_verb").y), (59, 59));
        assert!(matches!(
            &form.item("attr_verb").unwrap().control,
            reprisekit::controls::form::Control::Label { text, .. } if text == "set read-only"
        ));
        assert_eq!((at("attr_q0").x, at("attr_q0").y), (14, 98));
        assert_eq!(at("attr_q1").y, 111, "the question wraps into two lines");
        assert_eq!((at(ATTR_ONLY).x, at(ATTR_ONLY).y), (57, 138));
        assert_eq!(at(ATTR_TREE).y, 161);
        assert_eq!(at("attr_ok"), Rect::new(222, 202, BUTTON_W, BUTTON_H));
        assert_eq!(at("attr_cancel").x, 303);
        let only = form.index_of(ATTR_ONLY);
        let hit = l.hit(&d, 80, 141, &text, px);
        assert!(matches!(hit, Some(DialogHit::Control(h)) if Some(h.item) == only));
        assert_eq!(d.click(hit), Outcome::Redraw);
        assert_eq!(d.release(hit), Outcome::Redraw);
        assert_eq!(
            d.key(Key::Enter),
            Outcome::Properties {
                path: "/home/u/Desktop/New Folder".into(),
                name: None,
                attrs: reprisefs::props::AttrMoves {
                    read_only: Some(true),
                    ..Default::default()
                },
                hidden: None,
                folder_icon: None,
                summary: Vec::new(),
                launcher: Vec::new(),
                close: false,
                recurse: Some(false),
            }
        );
        assert_eq!(d.key(Key::Down), Outcome::Redraw);
        assert!(matches!(
            d.key(Key::Enter),
            Outcome::Properties {
                recurse: Some(true),
                ..
            }
        ));
        assert_eq!(d.key(Key::Escape), Outcome::Close);
    }

    /// Two files on the desktop, the second read-only, and a folder not
    /// walked yet.
    #[test]
    fn change_icon_gives_a_folder_its_own_icon() {
        use reprisekit::controls::filesheet::CHANGE_ICON;
        use reprisekit::controls::iconlist::IconList;
        let mut folder = file_info();
        folder.is_dir = true;
        folder.opens_with = None;
        folder.path = "/home/u/Desktop/Tunes".into();
        folder.name = "Tunes".into();
        folder.hidden_ext = String::new();
        let mut d = Dialog::properties(&folder, "folder", (0, 0));
        d.cycle_page(false);
        d.cycle_page(false);
        assert_eq!(
            press_control(&mut d, CHANGE_ICON),
            Outcome::ChangeIcon {
                name: "Tunes".into(),
                current: None,
                launcher: false,
            }
        );
        let names = ["folder", "folder-music"].map(String::from).to_vec();
        let mut list = IconList::new(names, vec![None, None], None);
        list.pick("folder");
        let mut ci = Dialog::change_icon("Tunes", "theme", list, false);
        assert_eq!(ci.title, "Change Icon for Tunes Folder");
        ci.key(Key::Down);
        assert_eq!(
            ci.key(Key::Enter),
            Outcome::IconChosen(Some("folder-music".into()))
        );
        assert!(d.icon_chosen(Some("folder-music".into())));
        assert_eq!(
            page_pictures(&d)[0],
            ("cu_icon", "folder-music".to_string(), 32)
        );
        assert_eq!(
            press_control(&mut d, reprisekit::controls::propsheet::OK),
            Outcome::Properties {
                path: folder.path.clone(),
                name: None,
                attrs: Default::default(),
                hidden: None,
                summary: Vec::new(),
                launcher: Vec::new(),
                folder_icon: Some(Some("folder-music".into())),
                close: true,
                recurse: None,
            }
        );
    }

    #[test]
    fn a_shortcuts_header_icon_wears_the_arrow() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let mut icon = Canvas::new(32, 32);
        icon.fill_rect(Rect::new(0, 0, 32, 32), Color::rgb(255, 0, 0));
        let render = |d: &Dialog| {
            let l = DialogLayout::compute(&theme, &text, d);
            let r = l.icon.expect("the header icon");
            let c = draw(
                &theme,
                &text,
                &l,
                Some(&icon),
                None,
                &[],
                &[],
                None,
                d,
                true,
                None,
                Cues::ALL,
                true,
                None,
            );
            (c.get(r.x, r.bottom() - 1), c.get(r.x, r.bottom() - 12))
        };
        let black = Color::rgb(0, 0, 0).argb();
        let red = Color::rgb(255, 0, 0).argb();
        let mut info = file_info();
        info.name = "Shell".into();
        info.hidden_ext = String::new();
        info.path = "/home/u/Desktop/shell.desktop".into();
        info.is_shortcut = true;
        info.launcher = Some(reprisefs::launcher::Launcher {
            target: "/bin/sh".into(),
            ..Default::default()
        });
        let mut d = Dialog::properties(&info, "utilities-terminal", (0, 0));
        assert_eq!(render(&d), (black, red), "on Shortcut");
        d.form_mut().unwrap().sheet_mut().unwrap().open_tab(0);
        assert_eq!(render(&d), (black, red), "on General");
        let plain = Dialog::properties(&file_info(), "text-plain", (0, 0));
        assert_eq!(render(&plain), (red, red));
    }

    #[test]
    fn a_launchers_shortcut_tab_takes_its_keys_to_ok() {
        use reprisefs::launcher::{Key as LKey, Launcher};
        use reprisekit::controls::filesheet::{CHANGE_ICON, FIND_TARGET, SHORTCUT_IDS};
        use reprisekit::controls::form::Control;
        let mut info = file_info();
        info.name = "Shell".into();
        info.hidden_ext = String::new();
        info.path = "/home/u/Desktop/shell.desktop".into();
        info.is_shortcut = true;
        info.opens_with = None;
        info.launcher = Some(Launcher {
            target: "/bin/sh -l".into(),
            path: String::new(),
            comment: "A shell".into(),
            icon: Some("utilities-terminal".into()),
            location: "bin".into(),
            ..Default::default()
        });
        let mut d = Dialog::properties(&info, "utilities-terminal", (0, 0));
        let f = d.form().unwrap();
        let sheet = f.sheet().unwrap();
        assert_eq!(sheet.tabs.labels, ["General", "Shortcut", "Compatibility"]);
        assert_eq!(sheet.tabs.open, 1);
        assert_eq!(f.size, (367, 502));
        assert!(matches!(
            sheet_form(&d).item("sc_where").map(|i| &i.control),
            Some(Control::Label { text, .. }) if text == "bin"
        ));

        sheet_form_mut(&mut d).focus_id(SHORTCUT_IDS[2]);
        d.key(Key::Edit(crate::textedit::EditKey::End { select: false }));
        d.key(Key::Char('!'));
        assert!(matches!(
            sheet_form(&d)
                .item(reprisekit::controls::propsheet::APPLY)
                .map(|i| &i.control),
            Some(Control::Button { enabled: true, .. })
        ));
        assert_eq!(
            press_control(&mut d, CHANGE_ICON),
            Outcome::ChangeIcon {
                name: "shell.desktop".into(),
                current: Some("utilities-terminal".into()),
                launcher: true,
            }
        );
        let list = reprisekit::controls::iconlist::IconList::new(Vec::new(), Vec::new(), None);
        let ci = Dialog::change_icon("shell.desktop", "theme", list, true);
        assert_eq!(ci.title, "Change Icon");
        assert!(d.icon_chosen(Some("terminal".into())));
        assert_eq!(d.icon.as_deref(), Some("terminal"));
        assert_eq!(
            press_control(&mut d, FIND_TARGET),
            Outcome::FindTarget("/bin/sh".into())
        );
        assert_eq!(
            press_control(&mut d, reprisekit::controls::propsheet::OK),
            Outcome::Properties {
                path: info.path.clone(),
                name: None,
                attrs: Default::default(),
                hidden: None,
                summary: Vec::new(),
                folder_icon: None,
                launcher: vec![
                    (LKey::Comment, "A shell!".to_string()),
                    (LKey::Icon, "terminal".to_string())
                ],
                close: true,
                recurse: None,
            }
        );
    }

    #[test]
    fn a_symlinks_shortcut_tab_takes_its_target_to_ok() {
        use reprisefs::launcher::Key as LKey;
        use reprisekit::controls::filesheet::{FIND_TARGET, SHORTCUT_IDS};
        use reprisekit::controls::form::Control;
        let dir =
            std::env::temp_dir().join(format!("reprise-symlink-sheet-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let real = std::fs::canonicalize(&dir).unwrap();
        std::fs::create_dir(real.join("Documents")).unwrap();
        let link = real.join("Docs");
        std::os::unix::fs::symlink("Documents", &link).unwrap();
        let info = reprisefs::props::glance(&link, "Docs", &[]).unwrap();
        let mut d = Dialog::properties(&info, "folder", (0, 0));
        let sheet = d.form_mut().unwrap().sheet_mut().unwrap();
        assert_eq!(sheet.tabs.labels, ["General", "Shortcut"]);
        assert_eq!(sheet.tabs.open, 1);
        assert!(matches!(
            sheet.pages[0].item("v_desc").map(|i| &i.control),
            Some(Control::Label { text, .. }) if text == "Documents"
        ));
        assert!(matches!(
            sheet_form(&d).item(SHORTCUT_IDS[1]).map(|i| &i.control),
            Some(Control::Edit { enabled: false, .. })
        ));
        assert_eq!(
            press_control(&mut d, FIND_TARGET),
            Outcome::FindTarget(real.join("Documents"))
        );
        sheet_form_mut(&mut d).focus_id(SHORTCUT_IDS[0]);
        d.key(Key::Edit(crate::textedit::EditKey::End { select: false }));
        d.key(Key::Char('2'));
        assert_eq!(
            press_control(&mut d, reprisekit::controls::propsheet::OK),
            Outcome::Properties {
                path: link.clone(),
                name: None,
                attrs: Default::default(),
                hidden: None,
                summary: Vec::new(),
                folder_icon: None,
                launcher: vec![(LKey::Link, "Documents2".to_string())],
                close: true,
                recurse: None,
            }
        );
        assert!(matches!(
            press_control(&mut d, FIND_TARGET),
            Outcome::Notice(_)
        ));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_untrusted_launchers_sheet_is_a_files() {
        use reprisekit::controls::form::Control;
        let dir = std::env::temp_dir().join(format!("reprise-untrusted-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("term.desktop");
        let app = "[Desktop Entry]\nType=Application\nName=Terminal\nExec=foot\n";
        std::fs::write(&path, app).unwrap();
        let db = reprisefs::apps::MimeDb::empty();
        let info = reprisefs::props::glance_in(&path, "term.desktop", &[], &db).unwrap();
        let mut d = Dialog::properties(&info, "text-x-generic", (0, 0));
        let sheet = d.form_mut().unwrap().sheet_mut().unwrap();
        assert_eq!(sheet.tabs.labels, ["General", "Summary"]);
        assert_eq!(sheet.tabs.open, 0);
        let general = &sheet.pages[0];
        assert!(matches!(
            general.item("v_type").map(|i| &i.control),
            Some(Control::Label { text, .. }) if text == "DESKTOP File"
        ));
        assert!(general.item("v_desc").is_none());
        assert!(matches!(
            general.item(reprisekit::controls::filesheet::NAME).map(|i| &i.control),
            Some(Control::Edit { edit, .. }) if edit.text == "term.desktop"
        ));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_symlink_to_a_document_shows_opens_with() {
        use reprisekit::controls::form::Control;
        let dir = std::env::temp_dir().join(format!("reprise-doc-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("song.rtest"), "x").unwrap();
        let link = dir.join("Song");
        std::os::unix::fs::symlink("song.rtest", &link).unwrap();
        let db = reprisefs::apps::MimeDb::parse(["50:application/x-reprise-test:*.rtest\n"]);
        let player = reprisefs::apps::AppEntry {
            name: "Player".into(),
            exec_line: "player %f".into(),
            terminal: false,
            id: "player.desktop".into(),
            icon: None,
            startup_wm_class: None,
            categories: Vec::new(),
            mime_types: vec!["application/x-reprise-test".into()],
            shown: true,
        };
        let info = reprisefs::props::glance_in(&link, "Song", &[player], &db).unwrap();
        let mut d = Dialog::properties(&info, "text-x-generic", (0, 0));
        let sheet = d.form_mut().unwrap().sheet_mut().unwrap();
        assert_eq!(sheet.tabs.labels, ["General", "Shortcut"]);
        let general = &sheet.pages[0];
        assert_eq!(
            general.item("v_opens").map(|i| &i.control),
            Some(&Control::Lines(vec!["Player".into()]))
        );
        assert!(general.item("v_desc").is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_symlinks_apply_shows_where_it_points_now() {
        use reprisekit::controls::filesheet::SHORTCUT_IDS;
        use reprisekit::controls::form::Control;
        use reprisekit::controls::propsheet::APPLY;
        let dir = std::env::temp_dir().join(format!("reprise-relink-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let real = std::fs::canonicalize(&dir).unwrap();
        std::fs::create_dir_all(real.join("a/Documents")).unwrap();
        std::fs::create_dir_all(real.join("b")).unwrap();
        std::fs::write(real.join("b/song.rtest"), "").unwrap();
        let link = real.join("Docs");
        std::os::unix::fs::symlink("a/Documents", &link).unwrap();
        let db = reprisefs::apps::MimeDb::parse(["50:application/x-reprise-test:*.rtest\n"]);
        let apps = [reprisefs::apps::AppEntry {
            name: "Player".into(),
            exec_line: "player %f".into(),
            terminal: false,
            id: "player.desktop".into(),
            icon: None,
            startup_wm_class: None,
            categories: Vec::new(),
            mime_types: vec!["application/x-reprise-test".into()],
            shown: true,
        }];
        let info = reprisefs::props::glance_in(&link, "Docs", &apps, &db).unwrap();
        let mut d = Dialog::properties(&info, "folder", (0, 0));
        let label = |d: &Dialog, page: usize, id: &str| {
            let form = &d.form().unwrap().pages()[page];
            match form.item(id).map(|i| &i.control) {
                Some(Control::Label { text, .. }) => Some(text.clone()),
                Some(Control::Lines(lines)) => Some(lines.join(" ")),
                _ => None,
            }
        };
        let open = |d: &Dialog| {
            let s = d.form().unwrap().sheet().unwrap();
            s.tabs.labels[s.tabs.open].clone()
        };
        assert_eq!(label(&d, 1, "sc_type").as_deref(), Some("File Folder"));
        assert_eq!(label(&d, 0, "v_desc").as_deref(), Some("Documents"));
        sheet_form_mut(&mut d).focus_id(SHORTCUT_IDS[0]);
        d.key(Key::Edit(crate::textedit::EditKey::SelectAll));
        for c in "b/song.rtest".chars() {
            d.key(Key::Char(c));
        }
        let asked = press_control(&mut d, APPLY);
        let Outcome::Properties { launcher, .. } = &asked else {
            panic!("Apply asks for the write, not {asked:?}");
        };
        reprisefs::launcher::write_keys(&link, launcher).unwrap();
        let name = d.sheet_name().unwrap().to_string();
        assert_eq!(name, "Docs", "read back under the name it opened under");
        let now = reprisefs::props::glance_in(&link, &name, &apps, &db).unwrap();
        d.sheet_reread(&now, "text-x-generic");
        assert_eq!(d.icon.as_deref(), Some("text-x-generic"));
        assert_eq!(d.title, "Docs Properties");
        assert_eq!(open(&d), "Shortcut");
        assert_eq!(label(&d, 1, "sc_type").as_deref(), Some("RTEST File"));
        assert_eq!(label(&d, 1, "sc_where").as_deref(), Some("b"));
        assert_eq!(label(&d, 0, "v_opens").as_deref(), Some("Player"));
        assert_eq!(label(&d, 0, "v_desc"), None);
        assert!(
            !matches!(
                sheet_form(&d).item(APPLY).map(|i| &i.control),
                Some(Control::Button { enabled: true, .. })
            ),
            "nothing left to write"
        );
        d.form_mut().unwrap().sheet_mut().unwrap().open_tab(0);
        let back = [(reprisefs::launcher::Key::Link, "a/Documents".to_string())];
        reprisefs::launcher::write_keys(&link, &back).unwrap();
        let now = reprisefs::props::glance_in(&link, &name, &apps, &db).unwrap();
        d.sheet_reread(&now, "folder");
        assert_eq!(open(&d), "General");
        assert_eq!(d.icon.as_deref(), Some("folder"));
        assert_eq!(label(&d, 0, "v_desc").as_deref(), Some("Documents"));
        assert_eq!(label(&d, 0, "v_opens"), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_web_links_web_document_tab_takes_its_url_to_ok() {
        use reprisefs::launcher::Key as LKey;
        use reprisekit::controls::filesheet::SHORTCUT_IDS;
        let dir = std::env::temp_dir().join(format!("reprise-web-sheet-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ex.desktop");
        let _ = std::fs::remove_file(&path);
        let text = reprisefs::apps::link_text("Example", "https://example.org/").unwrap();
        reprisefs::apps::write_launcher(&path, &text).unwrap();
        let info = reprisefs::props::glance(&path, "Example", &[]).unwrap();
        let mut d = Dialog::properties(&info, "text-html", (0, 0));
        let sheet = d.form_mut().unwrap().sheet_mut().unwrap();
        assert_eq!(sheet.tabs.labels, ["General", "Web Document"]);
        assert_eq!(sheet.tabs.open, 1);
        sheet_form_mut(&mut d).focus_id(SHORTCUT_IDS[0]);
        d.key(Key::Edit(crate::textedit::EditKey::End { select: false }));
        d.key(Key::Char('x'));
        assert_eq!(
            press_control(&mut d, reprisekit::controls::propsheet::OK),
            Outcome::Properties {
                path: path.clone(),
                name: None,
                attrs: Default::default(),
                hidden: None,
                summary: Vec::new(),
                folder_icon: None,
                launcher: vec![(LKey::Url, "https://example.org/x".to_string())],
                close: true,
                recurse: None,
            }
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_read_only_launchers_sheet_starts_from_its_general_box() {
        use reprisefs::launcher::Launcher;
        use reprisekit::controls::filesheet::READ_ONLY;
        use reprisekit::controls::propsheet::OK;
        let mut info = file_info();
        info.name = "Shell".into();
        info.hidden_ext = String::new();
        info.path = "/home/u/Desktop/shell.desktop".into();
        info.is_shortcut = true;
        info.read_only = true;
        info.launcher = Some(Launcher {
            target: "/bin/sh".into(),
            path: String::new(),
            comment: String::new(),
            icon: None,
            ..Default::default()
        });
        let mut d = Dialog::properties(&info, "utilities-terminal", (0, 0));
        assert_eq!(
            d.clone().key(Key::Enter),
            Outcome::Close,
            "nothing moved, nothing to write"
        );
        d.form_mut().unwrap().sheet_mut().unwrap().open_tab(0);
        press_control(&mut d, READ_ONLY);
        let Outcome::Properties { attrs, .. } = press_control(&mut d, OK) else {
            panic!("OK asks for the write");
        };
        assert_eq!(attrs.read_only, Some(false));
    }

    /// Apply's write settles only what it wrote: a Comment typed while it
    /// ran, and a field whose write failed, keep Apply lit, and OK then
    /// writes them.
    #[test]
    fn apply_settles_only_the_fields_it_wrote() {
        use reprisefs::launcher::{Key as LKey, Launcher};
        use reprisekit::controls::filesheet::SHORTCUT_IDS;
        use reprisekit::controls::form::Control;
        use reprisekit::controls::propsheet::{APPLY, OK};
        let mut info = file_info();
        info.name = "Shell".into();
        info.hidden_ext = String::new();
        info.path = "/home/u/Desktop/shell.desktop".into();
        info.is_shortcut = true;
        info.launcher = Some(Launcher {
            target: "/bin/sh".into(),
            path: String::new(),
            comment: "A shell".into(),
            icon: None,
            ..Default::default()
        });
        let apply_lit = |d: &Dialog| {
            matches!(
                sheet_form(d).item(APPLY).map(|i| &i.control),
                Some(Control::Button { enabled: true, .. })
            )
        };
        let mut d = Dialog::properties(&info, "utilities-terminal", (0, 0));
        let type_in = |d: &mut Dialog, id: &str, c: char| {
            sheet_form_mut(d).focus_id(id);
            d.key(Key::Edit(crate::textedit::EditKey::End { select: false }));
            d.key(Key::Char(c));
        };
        type_in(&mut d, SHORTCUT_IDS[2], '!');
        let asked = press_control(&mut d, APPLY);
        type_in(&mut d, SHORTCUT_IDS[2], '?');
        d.sheet_applied(&written(&asked));
        assert!(apply_lit(&d), "the Comment typed since is still to write");
        let Outcome::Properties { launcher, .. } = press_control(&mut d, OK) else {
            panic!("OK asks for the write");
        };
        assert_eq!(launcher, [(LKey::Comment, "A shell!?".to_string())]);

        let mut d = Dialog::properties(&info, "utilities-terminal", (0, 0));
        type_in(&mut d, SHORTCUT_IDS[0], 'x');
        press_control(&mut d, APPLY);
        d.sheet_applied(&Written {
            path: info.path.clone(),
            ..Default::default()
        });
        assert!(apply_lit(&d), "a failed write stays a change");
    }

    #[test]
    fn a_files_summary_takes_its_fields_to_ok() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let mut info = file_info();
        info.summary = Some(["Report", "", "", "", "", ""].map(String::from));
        let mut d = Dialog::properties(&info, "text-x-generic", (0, 0));
        let labels = |d: &Dialog| d.form().unwrap().sheet().unwrap().tabs.labels.clone();
        assert_eq!(labels(&d), ["General", "Summary"]);
        d.cycle_page(false);
        d.key(Key::Edit(crate::textedit::EditKey::End { select: false }));
        d.key(Key::Char('s'));
        assert!(matches!(
            sheet_form(&d)
                .item(reprisekit::controls::propsheet::APPLY)
                .map(|i| &i.control),
            Some(reprisekit::controls::form::Control::Button { enabled: true, .. })
        ));
        assert_eq!(
            press_control(&mut d, reprisekit::controls::propsheet::OK),
            Outcome::Properties {
                path: info.path.clone(),
                name: None,
                attrs: Default::default(),
                hidden: None,
                folder_icon: None,
                summary: vec![(0, "Reports".to_string())],
                launcher: Vec::new(),
                close: true,
                recurse: None,
            }
        );

        let docs = reprisefs::props::glance_set(&[file_info(), file_info()]);
        let d = Dialog::set_properties(&docs, "a", (0, 0));
        assert_eq!(labels(&d), ["General", "Summary"]);
        assert_eq!(DialogLayout::compute(&theme, &text, &d).size, (367, 502));
        let mut link = file_info();
        link.is_shortcut = true;
        let links = reprisefs::props::glance_set(&[link.clone(), link]);
        let d = Dialog::set_properties(&links, "a", (0, 0));
        assert_eq!(labels(&d), ["General", "Compatibility"]);
        assert_eq!(
            DialogLayout::compute(&theme, &text, &d).size,
            (367, 437 + 24)
        );
    }

    #[test]
    fn a_folder_and_a_shortcut_set_get_their_tabs() {
        use reprisekit::controls::filesheet::PAGE_ICONS;
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let mut folder = file_info();
        folder.is_dir = true;
        folder.opens_with = None;
        let mut d = Dialog::properties(&folder, "folder", (0, 0));
        let labels = |d: &Dialog| d.form().unwrap().sheet().unwrap().tabs.labels.clone();
        assert_eq!(labels(&d), ["General", "Sharing", "Customize"]);
        assert!(page_pictures(&d).is_empty(), "General has none");
        d.cycle_page(false);
        assert_eq!(
            page_pictures(&d),
            PAGE_ICONS.map(|(id, n)| (id, n.to_string(), 32))
        );
        d.cycle_page(false);
        assert_eq!(
            page_pictures(&d),
            [
                ("cu_icon", "folder".to_string(), 32),
                ("cu_preview", "folder".to_string(), 96)
            ],
            "Customize shows the standard folder"
        );
        assert_eq!(DialogLayout::compute(&theme, &text, &d).size.1, 424 + 24);

        let mut link = file_info();
        link.is_shortcut = true;
        link.path = "/home/u/Desktop/app.desktop".into();
        let set = reprisefs::props::glance_set(&[file_info(), link]);
        let d = Dialog::set_properties(&set, "New Text Document", (0, 0));
        assert_eq!(labels(&d), ["General", "Compatibility", "Summary"]);
        assert_eq!(DialogLayout::compute(&theme, &text, &d).size, (367, 502));
    }

    fn a_set() -> reprisefs::props::SetInfo {
        let mut folder = file_info();
        folder.path = "/home/u/Desktop/Reports".into();
        folder.is_dir = true;
        folder.type_name = "File Folder".into();
        let mut other = file_info();
        other.path = "/home/u/Desktop/b.txt".into();
        other.read_only = true;
        reprisefs::props::glance_set(&[file_info(), other, folder])
    }

    #[test]
    fn a_set_sheet_frames_the_set_page_and_adds_up_the_walk() {
        use reprisekit::controls::filesheet::{ADVANCED, COUNT};
        use reprisekit::controls::form::Control;
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let mut d = Dialog::set_properties(&a_set(), "New Text Document", (0, 0));
        assert_eq!(d.title, "New Text Document, ... Properties");
        assert_eq!(DialogLayout::compute(&theme, &text, &d).size, (367, 502));
        let label = |d: &Dialog, id: &str| match sheet_form(d).item(id).map(|i| &i.control) {
            Some(Control::Label { text, .. }) => text.clone(),
            other => panic!("{id}: {other:?}"),
        };
        assert_eq!(label(&d, COUNT), "Calculating...");
        assert_eq!(label(&d, "v_type"), "Multiple Types");
        assert_eq!(label(&d, "v_loc"), "All in /home/u/Desktop");
        assert!(d.set_folder_totals(reprisefs::props::FolderTotals {
            files: 3,
            folders: 1,
            size: 1536,
            on_disk: 8192,
        }));
        assert_eq!(label(&d, COUNT), "5 Files, 2 Folders");
        assert_eq!(label(&d, "v_size"), reprisefs::props::format_size(1536));
        assert_eq!(d.clone().key(Key::Enter), Outcome::Close);
        assert_eq!(
            press_control(&mut d, ADVANCED),
            Outcome::AdvancedAttributes {
                subject: Subject::Set { has_dir: true },
                robots: None,
            }
        );
    }

    #[test]
    fn a_set_sheet_writes_what_moved_across_the_set() {
        use reprisekit::controls::filesheet::{HIDDEN, READ_ONLY};
        use reprisekit::controls::propsheet::{APPLY, OK};
        let apply_on = |d: &Dialog| {
            matches!(
                sheet_form(d).item(APPLY).map(|i| &i.control),
                Some(reprisekit::controls::form::Control::Button { enabled: true, .. })
            )
        };
        let set = a_set();
        let mut d = Dialog::set_properties(&set, "New Text Document", (0, 0));
        assert_eq!(
            reprisekit::controls::filesheet::read_only(sheet_form(&d)),
            CheckState::Mixed
        );
        press_control(&mut d, READ_ONLY);
        assert!(apply_on(&d));
        assert_eq!(
            press_control(&mut d, APPLY),
            Outcome::SetProperties {
                paths: set.paths.clone(),
                attrs: reprisefs::props::AttrMoves {
                    read_only: Some(false),
                    ..Default::default()
                },
                hidden: None,
                folder_icon: None,
                close: false,
                recurse: None,
            }
        );
        d.set_applied(set.paths.clone());
        assert!(!apply_on(&d), "nothing left to write");
        press_control(&mut d, HIDDEN);
        assert_eq!(
            press_control(&mut d, OK),
            Outcome::SetProperties {
                paths: set.paths.clone(),
                attrs: Default::default(),
                hidden: Some(true),
                folder_icon: None,
                close: true,
                recurse: None,
            }
        );
    }

    #[test]
    fn a_set_attribute_confirm_answers_for_the_set() {
        use reprisekit::controls::filesheet::{ATTR_TREE, SET_ATTR_LABELS};
        use reprisekit::controls::form::Control;
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let paths = a_set().paths;
        let mut d = Dialog::confirm_attributes(
            AttrConfirm {
                write: AttrWrite::Set {
                    paths: paths.clone(),
                    hidden: Some(true),
                    folder_icon: None,
                },
                attrs: reprisefs::props::AttrMoves {
                    read_only: Some(true),
                    ..Default::default()
                },
                close: true,
            },
            &text,
            theme.fonts.size_px,
            TextUnits::ONE,
        );
        assert!(matches!(
            &d.form().unwrap().open_page().unwrap().item(ATTR_TREE).unwrap().control,
            Control::Radio { label, .. } if label == SET_ATTR_LABELS[1]
        ));
        assert_eq!(
            d.key(Key::Enter),
            Outcome::SetProperties {
                paths,
                attrs: reprisefs::props::AttrMoves {
                    read_only: Some(true),
                    ..Default::default()
                },
                hidden: Some(true),
                folder_icon: None,
                close: true,
                recurse: Some(true),
            }
        );
    }

    #[test]
    fn open_with_hosts_the_chooser_in_its_frame_and_answers_for_the_file() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let d = chooser();
        assert_eq!(d.title, "Open With");
        let l = DialogLayout::compute(&theme, &text, &d);
        assert_eq!(l.size, (396, 419));
        assert_eq!(l.page, Some(Rect::new(3, 21, 390, 395)));
        let px = theme.fonts.size_px;
        assert_eq!(
            l.hit(&d, 27 + 40, 102 + 2 + 3 * 18 + 5, &text, px),
            Some(DialogHit::Chooser(openwith::Hit::List(
                grouplist::Hit::Row(3)
            )))
        );
        assert_eq!(
            l.hit(&d, 250, 394, &text, px),
            Some(DialogHit::Chooser(openwith::Hit::Ok))
        );
        let mut d2 = d.clone();
        assert_eq!(
            d2.key(Key::Enter),
            Outcome::OpenWith {
                path: "/home/u/Desktop/New Text Document.txt".into(),
                app: "app1.desktop".into(),
                always: false,
                mime: Some("text/plain".into()),
            }
        );
        let browse = Some(DialogHit::Chooser(openwith::Hit::Browse));
        assert_eq!(d2.click(browse), Outcome::Redraw);
        assert_eq!(d2.release(browse), Outcome::BrowseProgram);
        assert_eq!(d2.key(Key::Escape), Outcome::Close);
        assert_eq!(
            d2.add_program(std::path::Path::new("/usr/bin/nano")),
            Outcome::Redraw
        );
        match d2.key(Key::Enter) {
            Outcome::OpenWith { app, .. } => assert_eq!(app, "/usr/bin/nano"),
            other => panic!("expected OpenWith for /usr/bin/nano, got {other:?}"),
        }
        use std::os::unix::ffi::OsStrExt;
        let odd = std::path::Path::new(std::ffi::OsStr::from_bytes(b"/usr/bin/a\xffb"));
        assert!(matches!(d2.add_program(odd), Outcome::Notice(n) if n.title == "Open With"));
        let mut d3 = d.clone();
        d3.key(Key::Tab { back: false });
        assert_eq!(
            d3.key(Key::Edit(EditKey::Insert(" ".into()))),
            Outcome::Redraw
        );
        assert!(d3.chooser().unwrap().ow.always.on, "Space ticks the box");
        let mut d4 = d.clone();
        assert_eq!(
            d4.key(Key::Edit(EditKey::Insert("o".into()))),
            Outcome::Redraw
        );
        assert_eq!(
            d4.chooser().unwrap().ow.list.selected,
            Some(3),
            "o picks the Other Programs header"
        );
        let mut d5 = d.clone();
        assert_eq!(
            d5.key(Key::Edit(EditKey::Insert("op".into()))),
            Outcome::Nothing
        );
        assert_eq!(
            d5.chooser().unwrap().ow.list.selected,
            Some(1),
            "the pick stays put"
        );
    }

    #[test]
    fn the_wheel_scrolls_the_chooser_list_under_the_pointer() {
        let mut d = chooser();
        let row = Some(DialogHit::Chooser(openwith::Hit::List(
            grouplist::Hit::Row(3),
        )));
        let before = d.chooser().unwrap().ow.clone();
        assert_eq!(d.wheel(row, 3), Outcome::Redraw);
        let ow = &d.chooser().unwrap().ow;
        assert_eq!(ow.list.top, 3);
        assert_eq!(ow.list.selected, before.list.selected, "the pick stays");
        let ok = Some(DialogHit::Chooser(openwith::Hit::Ok));
        assert_eq!(d.wheel(ok, 3), Outcome::Nothing, "off the list");
        assert_eq!(d.wheel(row, -9), Outcome::Redraw);
        assert_eq!(d.chooser().unwrap().ow.list.top, 0);
    }

    #[test]
    fn a_chooser_button_acts_on_a_release_over_it() {
        let ok = Some(DialogHit::Chooser(openwith::Hit::Ok));
        let mut d = chooser();
        assert_eq!(d.click(ok), Outcome::Redraw, "the press only arms OK");
        assert_eq!(
            d.release(ok),
            Outcome::OpenWith {
                path: "/home/u/Desktop/New Text Document.txt".into(),
                app: "app1.desktop".into(),
                always: false,
                mime: Some("text/plain".into()),
            }
        );
        let mut d = chooser();
        d.click(ok);
        assert_eq!(d.release(None), Outcome::Redraw, "off OK it only pops up");
        assert!(!d.chooser().unwrap().ow.holding());
    }

    #[test]
    fn a_double_click_on_a_program_row_opens_and_on_a_header_only_picks() {
        let row = |i| {
            Some(DialogHit::Chooser(openwith::Hit::List(
                grouplist::Hit::Row(i),
            )))
        };
        let mut d = chooser();
        assert_eq!(
            d.double_click(row(2)),
            Outcome::OpenWith {
                path: "/home/u/Desktop/New Text Document.txt".into(),
                app: "app2.desktop".into(),
                always: false,
                mime: Some("text/plain".into()),
            }
        );
        assert_eq!(d.double_click(row(3)), Outcome::Redraw);
        assert_eq!(d.chooser().unwrap().ow.list.selected, Some(3));
    }

    #[test]
    fn the_chooser_opens_on_the_default_program() {
        let others: Vec<_> = (0..12)
            .map(|i| Program {
                id: format!("app{i}.desktop"),
                name: format!("Program {i:02}"),
                icon: None,
            })
            .collect();
        let open = |pick| {
            Dialog::open_with(
                std::path::Path::new("/tmp/a.txt"),
                Some("text/plain".into()),
                vec![],
                others.clone(),
                pick,
                false,
            )
        };
        let d = open(Some("app11.desktop"));
        let ow = &d.chooser().unwrap().ow;
        assert_eq!(ow.chosen().map(|p| p.id.as_str()), Some("app11.desktop"));
        assert_eq!(ow.list.top, 13 - openwith::ROWS, "the last row is in view");
        let d = open(Some("gone.desktop"));
        let ow = &d.chooser().unwrap().ow;
        assert_eq!(ow.chosen().map(|p| p.id.as_str()), Some("app0.desktop"));
        assert_eq!(ow.list.top, 0);
    }

    #[test]
    fn enter_on_an_empty_chooser_does_nothing() {
        let mut d = Dialog::open_with(
            std::path::Path::new("/x"),
            None,
            vec![],
            vec![],
            None,
            false,
        );
        assert_eq!(d.key(Key::Enter), Outcome::Nothing);
    }

    #[test]
    fn change_default_chooser_only_sets_the_default() {
        let mut d = Dialog::open_with(
            std::path::Path::new("/home/u/Desktop/New Text Document.txt"),
            Some("text/plain".into()),
            vec![Program {
                id: "app1.desktop".into(),
                name: "Micro".into(),
                icon: Some("micro".into()),
            }],
            Vec::new(),
            None,
            true,
        );
        let greyed = Check {
            on: true,
            enabled: false,
        };
        assert_eq!(
            d.chooser().unwrap().ow.always,
            greyed,
            "ticked and greyed from the start"
        );
        assert_eq!(
            d.click(Some(DialogHit::Chooser(openwith::Hit::Always))),
            Outcome::Nothing
        );
        assert_eq!(d.chooser().unwrap().ow.always, greyed, "still ticked");
        assert_eq!(
            d.key(Key::Enter),
            Outcome::SetDefault {
                path: "/home/u/Desktop/New Text Document.txt".into(),
                app: "app1.desktop".into(),
                mime: Some("text/plain".into()),
            }
        );
        assert_eq!(d.key(Key::Access('b')), Outcome::BrowseProgram);
        d.add_program(std::path::Path::new("/opt/tools/viewer"));
        assert_eq!(
            d.key(Key::Enter),
            Outcome::SetDefault {
                path: "/home/u/Desktop/New Text Document.txt".into(),
                app: "/opt/tools/viewer".into(),
                mime: Some("text/plain".into()),
            }
        );
        let mut sheet = Dialog::properties(&file_info(), "text-x-generic", (0, 0));
        assert!(sheet.set_opens_with("Vim".into(), Some("vim".into())));
        assert_eq!(
            sheet_form(&sheet).item("v_opens").map(|i| &i.control),
            Some(&reprisekit::controls::form::Control::Lines(vec![
                "Vim".into()
            ]))
        );
        assert!(matches!(
            &sheet.form().unwrap().kind,
            FormKind::Properties(b) if b.opens_icon.as_deref() == Some("vim")
        ));
        assert_eq!(
            sheet.sheet_path(),
            Some(std::path::Path::new(
                "/home/u/Desktop/New Text Document.txt"
            ))
        );
        let theme = Theme::default();
        let cs = &theme.colors;
        let text = TextRenderer::load(&FontConfig::default());
        let l = DialogLayout::compute(&theme, &text, &d);
        let c = draw(
            &theme,
            &text,
            &l,
            None,
            None,
            &[],
            &[],
            None,
            &d,
            true,
            None,
            Cues::ALL,
            true,
            None,
        );
        let lay = d.chooser().unwrap().ow.layout(
            &text,
            theme.fonts.size_px,
            (3, 21),
            &theme.look.controls(),
        );
        let (b, lr) = (lay.check, lay.check_label);
        let has = |r: Rect, color: u32| {
            (r.x..r.right()).any(|x| (r.y..r.bottom()).any(|y| c.get(x, y) == color))
        };
        assert!(has(b.inset(2), cs.button_face.argb()), "greyed face");
        assert!(has(b, cs.button_shadow.argb()), "the mark is shadow grey");
        assert!(!has(b, cs.button_text.argb()), "and never black");
        assert!(has(lr, cs.button_shadow.argb()) && has(lr, cs.button_hilight.argb()));
        assert!(!has(lr, cs.button_text.argb()), "the label is etched");
    }

    #[test]
    fn open_with_chooser_draws_group_list_and_selection() {
        let theme = Theme::default();
        let Some(text) = tahoma() else { return };
        let d = chooser();
        let l = DialogLayout::compute(&theme, &text, &d);
        let c = draw(
            &theme,
            &text,
            &l,
            None,
            None,
            &[],
            &[],
            None,
            &d,
            true,
            None,
            Cues::ALL,
            true,
            None,
        );
        let cs = &theme.colors;
        let lay = d.chooser().unwrap().ow.layout(
            &text,
            theme.fonts.size_px,
            (3, 21),
            &theme.look.controls(),
        );
        let g = lay.group;
        assert_eq!(c.get(g.x + 200, g.y), cs.button_shadow.argb());
        assert_eq!(c.get(g.x + 200, g.y + 1), cs.button_hilight.argb());
        assert_eq!(c.get(g.x + 12, g.y), cs.button_face.argb());
        assert_eq!(c.get(g.x, g.y + 100), cs.button_shadow.argb());
        let list = lay.list.frame;
        assert_eq!(c.get(list.x, list.y + 50), cs.button_shadow.argb());
        assert_eq!(c.get(list.x + 1, list.y + 50), cs.button_dk_shadow.argb());
        assert_eq!(c.get(list.x + 10, list.y + 50), cs.window.argb());
        let sel_y = list.y + 2 + 18 + 8;
        assert_eq!(c.get(list.x + 45 + 3, sel_y), cs.hilight.argb());
        assert_eq!(c.get(list.x + 300, sel_y - 18), cs.window.argb());
        let cb = lay.check;
        assert_eq!(c.get(cb.x + 6, cb.y + 6), cs.window.argb());
        assert_eq!(c.get(lay.ok.x, lay.ok.y + 10), cs.button_text.argb());
    }

    #[test]
    fn remember_moves_a_repeat_to_the_front_and_caps_the_list() {
        let mut h = history();
        remember(&mut h, "calc");
        assert_eq!(h[..3], ["calc", "notepad", "mspaint"]);
        assert_eq!(h.len(), 5);
        for i in 0..40 {
            remember(&mut h, &format!("cmd{i}"));
        }
        assert_eq!(h.len(), HISTORY_MAX);
        assert_eq!(h[0], "cmd39");
    }

    #[test]
    fn a_press_on_the_drop_lists_thumb_keeps_the_list_open() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let px = theme.fonts.size_px;
        let mut d = Dialog::run(&history());
        let l = DialogLayout::compute(&theme, &text, &d);
        d.click(Some(DialogHit::Drop));
        let sb = l.scrollbar(&d).unwrap();
        let thumb = l.hit(&d, sb.x + 8, sb.y + sb.height / 2, &text, px);
        assert!(matches!(thumb, Some(DialogHit::ListBar(BarHit::Thumb(_)))));
        d.click(thumb);
        assert!(
            d.field.as_ref().unwrap().combo.open.is_some(),
            "the list stays open"
        );
        // The thumb is grabbed until the release. Three rows leave a 7 px
        // track that is all thumb, so the drag holds the list where it is.
        assert!(d.holding_thumb());
        assert_eq!(d.drag_list(&l, sb.bottom() + 200), Outcome::Nothing);
        assert_eq!(d.field.as_ref().unwrap().combo.top, 0);
        assert_eq!(d.release(None), Outcome::Redraw);
        assert!(!d.holding_thumb());
        assert!(d.field.as_ref().unwrap().combo.open.is_some());
    }

    #[test]
    fn text_dropped_on_runs_field_lands_at_the_drop_caret_and_takes_the_keyboard() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let px = theme.fonts.size_px;
        let mut d = Dialog::run(&[]);
        d.set_field("foot");
        d.button_focus = Some(0);
        let l = DialogLayout::compute(&theme, &text, &d);
        let field = l.field.unwrap();
        let ok = l.buttons[0];
        assert!(
            !d.drag_over(&l, ok.x + 5, ok.y + 5, &text, px),
            "a button takes no drop"
        );
        let x = field.x + combo::EDIT_TEXT_X + text.measure("fo", false, px);
        assert!(d.drag_over(&l, x, field.y + 5, &text, px));
        assert_eq!(
            d.drop_text(&l, &crate::textedit::DropLine::Text("X".into()), &text, px),
            Outcome::Redraw
        );
        let edit = d.field.as_ref().unwrap().edit().unwrap();
        assert_eq!(edit.text, "foXot");
        assert_eq!(edit.selected_text(), "X");
        assert_eq!(d.button_focus, None, "the field has the keyboard");
        assert_eq!(
            d.drop_text(&l, &crate::textedit::DropLine::Text("Y".into()), &text, px),
            Outcome::Nothing,
            "the drop ended the drag"
        );
    }

    #[test]
    fn combo_layout_and_clicks() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let mut d = Dialog::run(&history());
        let l = DialogLayout::compute(&theme, &text, &d);
        let field = l.field.unwrap();
        assert_eq!(field.height, 21);
        let drop = l.drop.unwrap();
        assert_eq!(drop, Rect::new(field.right() - 18, field.y + 2, 16, 17));
        let list = l.list.unwrap();
        assert_eq!(list, Rect::new(field.x, field.bottom(), field.width, 41));
        assert!(list.bottom() <= l.size.1, "the list fits on the dialog");
        assert_eq!(
            l.hit(&d, drop.x + 8, drop.y + 8, &text, theme.fonts.size_px),
            Some(DialogHit::Drop)
        );
        let ok = l.buttons[0];
        assert_eq!(
            l.hit(&d, ok.x + 5, ok.y + 5, &text, theme.fonts.size_px),
            Some(DialogHit::Button(0))
        );
        assert_eq!(
            l.hit(&d, field.x + 5, field.y + 5, &text, theme.fonts.size_px),
            Some(DialogHit::Field(5 - combo::EDIT_TEXT_X))
        );
        assert_eq!(d.click(Some(DialogHit::Drop)), Outcome::Redraw);
        assert!(d.field.as_ref().unwrap().combo.open.is_some());
        assert_eq!(
            l.hit(
                &d,
                list.x + 5,
                list.y + 1 + 13 + 6,
                &text,
                theme.fonts.size_px
            ),
            Some(DialogHit::Row(1))
        );
        let sb = l.scrollbar(&d).unwrap();
        assert_eq!(sb, Rect::new(list.right() - 17, list.y + 1, 16, 39));
        assert_eq!(
            l.hit(&d, sb.x + 8, sb.y + 3, &text, theme.fonts.size_px),
            Some(DialogHit::ListBar(BarHit::Back))
        );
        assert_eq!(
            l.hit(&d, sb.x + 8, sb.bottom() - 3, &text, theme.fonts.size_px),
            Some(DialogHit::ListBar(BarHit::Forward))
        );
        // The 7 px track between the arrows is all thumb, so the hit is
        // never a page here; a page click still moves three rows of five.
        assert_eq!(
            l.hit(
                &d,
                sb.x + 8,
                sb.y + ControlMetrics::CLASSIC.scroll + 3,
                &text,
                theme.fonts.size_px
            ),
            Some(DialogHit::ListBar(BarHit::Thumb(3)))
        );
        assert_eq!(
            d.click(Some(DialogHit::ListBar(BarHit::PageForward))),
            Outcome::Redraw
        );
        assert_eq!(d.field.as_ref().unwrap().combo.top, 2);
        assert_eq!(
            d.click(Some(DialogHit::ListBar(BarHit::PageBack))),
            Outcome::Redraw
        );
        assert_eq!(d.field.as_ref().unwrap().combo.top, 0);
        assert_eq!(
            d.click(Some(DialogHit::ListBar(BarHit::Forward))),
            Outcome::Redraw
        );
        assert_eq!(d.field.as_ref().unwrap().combo.top, 1);
        assert_eq!(
            l.hit(&d, list.x + 5, list.y + 5, &text, theme.fonts.size_px),
            Some(DialogHit::Row(1))
        );
        assert_eq!(d.click(Some(DialogHit::Row(2))), Outcome::Redraw);
        let f = d.field.as_ref().unwrap();
        assert_eq!(
            (
                f.edit().unwrap().text.as_str(),
                f.combo.open,
                f.edit().unwrap().all_selected()
            ),
            ("calc", None, true)
        );
        d.click(Some(DialogHit::Drop));
        assert_eq!(d.click(None), Outcome::Redraw);
        assert_eq!(d.field.as_ref().unwrap().combo.open, None);
        let text = TextRenderer::load(&FontConfig::default());
        let px = Theme::default().fonts.size_px;
        assert_eq!(d.press_field(&text, px, 1000, false), Outcome::Redraw);
        let f = |d: &Dialog| d.field.as_ref().unwrap().edit().unwrap().clone();
        assert!(!f(&d).has_selection());
        assert_eq!(f(&d).caret(), f(&d).text.len());
        assert_eq!(d.drag_field(&text, px, 0), Outcome::Redraw);
        assert_eq!(f(&d).selected_text(), f(&d).text);
        assert_eq!(d.double_click_field(&text, px, 2), Outcome::Redraw);
        assert_eq!(f(&d).selected_text(), f(&d).text);
        assert_eq!(d.paste("ls -l\nrm"), Outcome::Redraw);
        assert_eq!(f(&d).text, "ls -l");
    }

    #[test]
    fn combo_draws_help_button_selection_and_list() {
        let theme = Theme::default();
        let Some(text) = tahoma() else { return };
        let cs = &theme.colors;
        let mut d = Dialog::run(&history());
        let l = DialogLayout::compute(&theme, &text, &d);
        let c = draw(
            &theme,
            &text,
            &l,
            None,
            None,
            &[],
            &[],
            None,
            &d,
            true,
            None,
            Cues::ALL,
            true,
            None,
        );
        let help = l.help.unwrap();
        assert_eq!(help.right(), l.close.unwrap().x - 2);
        assert_eq!(c.get(help.x + 7, help.y + 2), cs.button_text.argb());
        assert_eq!(c.get(help.x + 7, help.y + 8), cs.button_face.argb());
        let f = l.field.unwrap();
        assert_eq!(c.get(f.x, f.y + 10), cs.button_shadow.argb());
        assert_eq!(c.get(f.x + 1, f.y + 10), cs.button_dk_shadow.argb());
        assert_eq!(c.get(f.x + 5, f.y + 4), cs.window.argb());
        assert_eq!(c.get(f.x + 6, f.y + 4), cs.hilight.argb());
        let dr = l.drop.unwrap();
        assert_eq!(c.get(dr.x + 8, dr.y + 7), cs.button_text.argb());
        assert_eq!(c.get(dr.x + 4, dr.y + 7), cs.button_face.argb());
        let list = l.list.unwrap();
        assert_eq!(c.get(list.x, list.y + 5), cs.button_face.argb());
        d.click(Some(DialogHit::Drop));
        let c = draw(
            &theme,
            &text,
            &l,
            None,
            None,
            &[],
            &[],
            None,
            &d,
            true,
            None,
            Cues::ALL,
            true,
            None,
        );
        assert_eq!(c.get(list.x, list.y + 5), cs.button_text.argb());
        assert_eq!(c.get(list.x + 30, list.y + 6), cs.hilight.argb());
        assert_eq!(c.get(list.x + 2, list.y + 1 + 13 + 6), cs.window.argb());
        let sb = l.scrollbar(&d).unwrap();
        assert_eq!(c.get(sb.x, sb.y), cs.button_face.argb());
        assert_eq!(c.get(sb.x + 1, sb.y + 1), cs.button_hilight.argb());
        assert_eq!(c.get(sb.x + 1, sb.y + 17), cs.button_hilight.argb());
        assert_eq!(c.get(sb.x + 8, sb.y + 20), cs.button_face.argb());
        assert_eq!(c.get(sb.x + 8, sb.y + 16 + 7 + 7), cs.button_text.argb());
    }

    #[test]
    fn browse_is_a_frame_around_the_file_dialog() {
        let theme = Theme::default();
        let Some(text) = tahoma() else { return };
        let dir = std::env::temp_dir().join(format!("reprise-browse-frame-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut d = Dialog::browse(crate::filepick::FilePicker::run_command(
            &dir,
            crate::filepick::Icons::default(),
            true,
        ));
        let l = DialogLayout::compute(&theme, &text, &d);
        assert_eq!(l.size, (563, 477));
        assert_eq!(l.page, Some(Rect::new(4, 22, 555, 451)));
        assert!(l.help.is_some());
        let px = theme.fonts.size_px;
        assert_eq!(
            l.hit(&d, 4 + 472 + 30, 22 + 416 + 10, &text, px),
            Some(DialogHit::Browse(
                reprisekit::controls::filedlg::Hit::Cancel
            ))
        );
        let client = l.page.unwrap();
        assert_eq!(
            d.picker_mut()
                .unwrap()
                .click(client, Some(reprisekit::controls::filedlg::Hit::Cancel)),
            Outcome::Close
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_grown_browse_dialog_frames_its_bigger_client() {
        use reprisekit::controls::filedlg::{CLIENT, Hit};
        let (dw, dh) = (168, 55);
        let theme = Theme::default();
        let Some(text) = tahoma() else { return };
        let dir = std::env::temp_dir().join(format!("reprise-browse-grown-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut d = Dialog::browse(crate::filepick::FilePicker::run_command(
            &dir,
            crate::filepick::Icons::default(),
            true,
        ));
        let draw_it = |d: &Dialog, l: &DialogLayout| {
            draw(
                &theme,
                &text,
                l,
                None,
                None,
                &[],
                &[],
                None,
                d,
                false,
                None,
                Cues::NONE,
                true,
                None,
            )
        };
        let small_l = DialogLayout::compute(&theme, &text, &d);
        let small = draw_it(&d, &small_l);
        d.picker_mut()
            .unwrap()
            .dlg
            .set_size((CLIENT.0 + dw, CLIENT.1 + dh));
        let l = DialogLayout::compute(&theme, &text, &d);
        assert_eq!(l.size, (731, 532));
        assert_eq!(l.page, Some(Rect::new(4, 22, 723, 506)));
        assert_eq!(
            l.close,
            small_l.close.map(|r| Rect::new(r.x + dw, r.y, 16, 14))
        );
        let m = &theme.metrics;
        assert_eq!(l.sizing_edges(1, 300, m), Some(Edges::LEFT));
        assert_eq!(l.sizing_edges(729, 300, m), Some(Edges::RIGHT));
        assert_eq!(l.sizing_edges(300, 1, m), Some(Edges::TOP));
        assert_eq!(
            l.sizing_edges(730, 531, m),
            Some(Edges::BOTTOM | Edges::RIGHT)
        );
        assert_eq!(l.sizing_edges(300, 10, m), None, "the caption moves it");
        assert_eq!(l.sizing_edges(300, 300, m), None, "the client is no border");
        let px = theme.fonts.size_px;
        assert_eq!(
            l.hit(&d, 4 + 472 + 30 + dw, 22 + 416 + 10 + dh, &text, px),
            Some(DialogHit::Browse(Hit::Cancel))
        );
        assert_eq!(
            l.hit(&d, 4 + 723 - 4, 22 + 506 - 4, &text, px),
            Some(DialogHit::Browse(Hit::Grip))
        );
        let client = l.page.unwrap();
        assert_eq!(
            d.picker_mut().unwrap().click(client, Some(Hit::Grip)),
            Outcome::Size(Edges::BOTTOM | Edges::RIGHT)
        );
        let big = draw_it(&d, &l);
        for y in 477 - 80..477 {
            for x in 563 - 120..563 {
                assert_eq!(big.get(x + dw, y + dh), small.get(x, y), "at ({x}, {y})");
            }
        }
        for y in 22..82 {
            for x in 0..90 {
                assert_eq!(big.get(x, y), small.get(x, y), "at ({x}, {y})");
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_fixed_dialogs_border_sizes_nothing() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let l = DialogLayout::compute(&theme, &text, &chooser());
        assert_eq!(l.sizing_edges(1, 100, &theme.metrics), None);
        assert_eq!(l.sizing_edges(1, 1, &theme.metrics), None);
    }

    fn blue() -> Theme {
        reprisekit::interfaces::theme_for(reprisekit::interface::Interface::Blue)
    }

    #[test]
    fn a_browse_dialogs_column_title_lights_under_the_pointer() {
        use reprisekit::controls::{filedlg, shellview};
        let theme = blue();
        let text = TextRenderer::load(&FontConfig::default());
        let dir = std::env::temp_dir().join(format!("reprise-browse-hover-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut d = Dialog::browse(crate::filepick::FilePicker::run_command(
            &dir,
            crate::filepick::Icons::default(),
            true,
        ));
        let l = DialogLayout::compute(&theme, &text, &d);
        let client = l.page.expect("the picker's page");
        let mut c = Canvas::new(600, 500);
        let px = theme.fonts.size_px;
        d.picker()
            .unwrap()
            .dlg
            .draw(&mut c, &text, &theme.paint(), client, px);
        let title = Some(DialogHit::Browse(filedlg::Hit::Shell(
            shellview::Hit::Header(0),
        )));
        assert!(d.hover(title, (0, 0), true), "the title lights");
        assert!(d.leave(true), "and goes cold");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_blue_dialog_is_framed_by_the_style() {
        use reprisekit::style::{Part, State};
        let classic = Theme::default();
        let blue = blue();
        let text = TextRenderer::load(&FontConfig::default());
        let px = blue.fonts.size_px;
        let d = Dialog::confirm_delete("x", std::path::Path::new("/x"), false, false, false);
        let cl = DialogLayout::compute(&classic, &text, &d);
        let bl = DialogLayout::compute(&blue, &text, &d);
        assert_eq!(bl.caption, Rect::new(3, 3, cl.caption.width, 26));
        assert_eq!(bl.size, (cl.size.0, cl.size.1 + 8));
        let down = |r: Rect| Rect::new(r.x, r.y + 8, r.width, r.height);
        assert_eq!(bl.icon, cl.icon.map(down));
        assert_eq!(
            bl.buttons,
            cl.buttons.iter().copied().map(down).collect::<Vec<_>>()
        );
        assert_eq!(bl.lines[0].0, down(cl.lines[0].0));
        let close = bl.close.unwrap();
        assert_eq!(close, Rect::new(bl.caption.right() - 23, 5, 21, 21));
        assert_eq!(
            bl.hit(&d, close.x + 10, close.y + 10, &text, px),
            Some(DialogHit::Close)
        );
        assert_eq!(
            bl.hit(&d, 40, bl.caption.bottom() - 1, &text, px),
            Some(DialogHit::Caption)
        );

        let style = blue.look.style().unwrap();
        let top = Rect::new(0, 0, bl.size.0, bl.caption.bottom());
        for (active, state) in [(true, State::Normal), (false, State::Inactive)] {
            let c = draw(
                &blue,
                &text,
                &bl,
                None,
                None,
                &[],
                &[],
                None,
                &d,
                false,
                None,
                Cues::NONE,
                active,
                None,
            );
            let mut want = Canvas::new(bl.size.0 as u32, top.height as u32);
            style.draw(&mut want, top, Part::Caption, state, 1);
            assert_eq!(c.get(12, 2), want.get(12, 2), "active {active}");
            assert_eq!(c.get(22, 15), want.get(22, 15), "active {active}");
            assert_eq!(
                c.get(20, bl.caption.bottom() + 2),
                blue.colors.button_face.argb(),
                "the body is button face"
            );
        }
    }

    #[test]
    fn a_glass_dialog_wears_the_window_frame_with_a_lone_close_box() {
        let classic = Theme::default();
        let glass = reprisekit::interfaces::theme_for(reprisekit::interface::Interface::Glass);
        let text = TextRenderer::load(&FontConfig::default());
        let px = glass.fonts.size_px;
        let d = Dialog::confirm_delete("x", std::path::Path::new("/x"), false, false, false);
        let cl = DialogLayout::compute(&classic, &text, &d);
        let gl = DialogLayout::compute(&glass, &text, &d);
        let (w, h) = gl.size;
        assert_eq!((w, h), (cl.size.0 + 10, cl.size.1 + 14));
        let f = gl.frame.unwrap();
        assert_eq!(f.frame, 8);
        assert_eq!(f.client, Rect::new(8, 30, w - 16, h - 38));
        assert_eq!(gl.close, Some(Rect::new(w - 55, 1, 49, 20)));
        assert_eq!(f.buttons[2], Rect::new(w - 55, 1, 49, 20));
        let down = |r: Rect| Rect::new(r.x + 5, r.y + 9, r.width, r.height);
        assert_eq!(gl.icon, cl.icon.map(down));
        assert_eq!(
            gl.hit(&d, w - 30, 10, &text, px),
            Some(DialogHit::Close),
            "the close box"
        );
        assert_eq!(
            gl.hit(&d, w - 70, 10, &text, px),
            Some(DialogHit::Caption),
            "left of the close box is caption"
        );
        let c = draw(
            &glass,
            &text,
            &gl,
            None,
            None,
            &[],
            &[],
            None,
            &d,
            false,
            None,
            Cues::NONE,
            true,
            None,
        );
        for (x, y) in [(0, 0), (w - 1, 0), (0, h - 1), (w - 1, h - 1)] {
            assert_eq!(c.get(x, y) >> 24, 0, "corner ({x}, {y}) is clear");
        }
        let band = c.get(w / 2, 15) >> 24;
        assert!(band > 0 && band < 0xFF, "the top band is glass: {band:#x}");
        assert!(
            c.get(0, 15) >> 24 > band,
            "the outline is denser than glass"
        );
        assert_eq!(
            c.get(20, f.client.y + 2),
            glass.colors.button_face.argb(),
            "the body is opaque button face"
        );
    }

    #[test]
    fn a_blue_dialogs_close_box_draws_hot() {
        use reprisekit::style::{Part, State};
        let blue = blue();
        let text = TextRenderer::load(&FontConfig::default());
        let mut d = Dialog::confirm_delete("x", std::path::Path::new("/x"), false, false, false);
        d.caption_hot = Some(CaptionButton::Close);
        let l = DialogLayout::compute(&blue, &text, &d);
        let c = draw(
            &blue,
            &text,
            &l,
            None,
            None,
            &[],
            &[],
            None,
            &d,
            false,
            None,
            Cues::NONE,
            true,
            None,
        );
        let r = l.close.unwrap();
        let mut want = Canvas::new(l.size.0 as u32, l.size.1 as u32);
        let style = blue.look.style().unwrap();
        style.draw(&mut want, r, Part::CloseButton, State::Hot, 1);
        assert_eq!(c.get(r.x + 2, r.y + 3), want.get(r.x + 2, r.y + 3));
        let mut normal = Canvas::new(l.size.0 as u32, l.size.1 as u32);
        style.draw(&mut normal, r, Part::CloseButton, State::Normal, 1);
        assert_ne!(want.get(r.x + 2, r.y + 3), normal.get(r.x + 2, r.y + 3));
    }

    #[test]
    fn the_close_box_closes_only_on_a_release_over_it() {
        let mut d = Dialog::log_off();
        d.press_caption(CaptionButton::Close);
        assert_eq!(d.caption_box.of(CaptionButton::Close), Interaction::Pressed);
        assert!(d.drag_caption(None), "dragged off");
        assert_eq!(d.caption_box.of(CaptionButton::Close), Interaction::Idle);
        assert!(
            d.release_caption().is_none(),
            "a release off the box does not close"
        );
        assert!(!d.caption_box.holding());
        d.press_caption(CaptionButton::Close);
        assert!(!d.drag_caption(Some(DialogHit::Close)), "still on it");
        assert!(d.release_caption().is_some(), "a release on the box closes");
        d.press_caption(CaptionButton::Close);
        d.cancel_press();
        assert!(
            !d.caption_box.holding(),
            "a taken pointer lets go of the close box"
        );
    }

    #[test]
    fn a_held_close_box_draws_pushed_only_while_the_pointer_is_on_it() {
        let text = TextRenderer::load(&FontConfig::default());
        let classic = Theme::default();
        let glass = reprisekit::interfaces::theme_for(reprisekit::interface::Interface::Glass);
        for theme in [&classic, &blue(), &glass] {
            let mut d =
                Dialog::confirm_delete("x", std::path::Path::new("/x"), false, false, false);
            d.caption_hot = Some(CaptionButton::Close);
            let l = DialogLayout::compute(theme, &text, &d);
            let shot = |d: &Dialog| {
                draw(
                    theme,
                    &text,
                    &l,
                    None,
                    None,
                    &[],
                    &[],
                    None,
                    d,
                    true,
                    None,
                    Cues::NONE,
                    true,
                    None,
                )
            };
            let r = l.close.unwrap();
            let pixels = |c: &Canvas| -> Vec<u32> {
                (r.y..r.bottom())
                    .flat_map(|y| (r.x..r.right()).map(move |x| (x, y)))
                    .map(|(x, y)| c.get(x, y))
                    .collect()
            };
            let plain = {
                let mut cold = d.clone();
                cold.caption_hot = None;
                pixels(&shot(&cold))
            };
            let hot = pixels(&shot(&d));
            d.press_caption(CaptionButton::Close);
            let held = pixels(&shot(&d));
            assert_ne!(
                held, hot,
                "{:?}: the held close box draws pushed",
                theme.look
            );
            d.drag_caption(None);
            assert_eq!(
                pixels(&shot(&d)),
                plain,
                "{:?}: dragged off, it draws plain",
                theme.look
            );
        }
        let mut d = Dialog::confirm_delete("x", std::path::Path::new("/x"), false, false, false);
        let l = DialogLayout::compute(&classic, &text, &d);
        d.press_caption(CaptionButton::Close);
        let c = draw(
            &classic,
            &text,
            &l,
            None,
            None,
            &[],
            &[],
            None,
            &d,
            true,
            None,
            Cues::NONE,
            true,
            None,
        );
        let r = l.close.unwrap();
        let cs = &classic.colors;
        assert_eq!(
            c.get(r.x, r.y),
            cs.button_shadow.argb(),
            "sunken: shadow outer top-left"
        );
        assert_eq!(c.get(r.x + 1, r.y + 1), cs.button_dk_shadow.argb());
        assert_eq!(
            c.get(r.right() - 1, r.bottom() - 1),
            cs.button_hilight.argb()
        );
    }

    #[test]
    fn a_form_dialog_fits_the_frame_it_is_shown_in() {
        use reprisekit::controls::filesheet::NAME;
        let classic = Theme::default();
        let blue = blue();
        let text = TextRenderer::load(&FontConfig::default());
        let px = classic.fonts.size_px;
        let mut d = Dialog::properties(&file_info(), "text-x-generic", (0, 0));
        d.fit(&classic, &text);
        let cl = DialogLayout::compute(&classic, &text, &d);
        let name = sheet_form(&d).rect(NAME).unwrap();
        d.fit(&blue, &text);
        d.fit(&blue, &text);
        let bl = DialogLayout::compute(&blue, &text, &d);
        assert_eq!(bl.size, (cl.size.0, cl.size.1 + 8));
        assert_eq!(bl.caption.height, 26);
        let page = cl.page.unwrap();
        assert_eq!(
            bl.page,
            Some(Rect::new(page.x, page.y + 8, page.width, page.height))
        );
        assert_eq!(bl.tabs[0].y, cl.tabs[0].y + 8);
        let moved = sheet_form(&d).rect(NAME).unwrap();
        assert_eq!(
            moved,
            Rect::new(name.x, name.y + 8, name.width, name.height)
        );
        assert!(matches!(
            bl.hit(&d, moved.x + 3, moved.y + 3, &text, px),
            Some(DialogHit::Control(_))
        ));
        d.fit(&classic, &text);
        assert_eq!(DialogLayout::compute(&classic, &text, &d), cl);
        assert_eq!(sheet_form(&d).rect(NAME).unwrap(), name);
    }

    #[test]
    fn a_blue_client_area_starts_under_the_styles_caption() {
        let blue = blue();
        let text = TextRenderer::load(&FontConfig::default());
        let browse = Dialog::browse(crate::filepick::FilePicker::run_command(
            &std::env::temp_dir(),
            crate::filepick::Icons::default(),
            true,
        ));
        let l = DialogLayout::compute(&blue, &text, &browse);
        assert_eq!(l.page, Some(Rect::new(4, 30, 555, 451)));
        assert_eq!(l.size, (563, 485));
        assert_eq!(l.caption, Rect::new(4, 4, 555, 26));
        let l = DialogLayout::compute(&blue, &text, &chooser());
        assert_eq!(l.page.map(|p| (p.x, p.y)), Some((3, 29)));
        assert_eq!(l.size, (CHOOSER_W, CHOOSER_H + 8));
    }

    /// Renders Browse to `target/browse-preview.png` for a look. Run with
    /// `cargo test render_browse_preview -- --ignored`; `REPRISE_PREVIEW_DIR`
    /// names the folder it lists.
    #[test]
    #[ignore]
    fn render_browse_preview() {
        use crate::icontheme::{IconCache, IconTheme};
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let name = std::env::var("REPRISE_PREVIEW_ICONS").unwrap_or_else(|_| "breeze".into());
        let mut icons = IconCache::new(IconTheme::load(&name, &IconTheme::default_roots()));
        let dir = std::env::var("REPRISE_PREVIEW_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir());
        let mut picker = crate::filepick::FilePicker::run_command(
            &dir,
            crate::filepick::icons(&mut icons, 1),
            true,
        );
        for r in picker.take_requests() {
            picker.reply(r.run(), &mut icons, 1);
        }
        let d = Dialog::browse(picker);
        let l = DialogLayout::compute(&theme, &text, &d);
        let c = draw(
            &theme,
            &text,
            &l,
            None,
            None,
            &[],
            &[],
            None,
            &d,
            false,
            None,
            Cues::NONE,
            true,
            None,
        );
        reprisekit::raster::write_preview(&c, "browse-preview.png");
    }

    #[test]
    fn a_key_aid_prompt_asks_on_its_fixed_template() {
        use crate::keyfilter::Aid;
        let theme = Theme::default();
        let Some(text) = tahoma() else { return };
        for (aid, lines) in [
            (Aid::StickyKeys, 6),
            (Aid::SlowKeys, 7),
            (Aid::MouseKeys, 6),
        ] {
            let mut d = Dialog::key_aid_prompt(aid);
            assert_eq!(d.title, aid.name());
            assert_eq!(d.placement, Placement::CenterAs(180));
            let l = DialogLayout::compute(&theme, &text, &d);
            assert_eq!(l.size, (410, 163), "{aid:?}");
            assert_eq!(l.icon, None);
            assert_eq!(l.lines.len(), lines, "{aid:?}: {:?}", l.lines);
            assert_eq!(l.lines[0].0, Rect::new(18, 30, 374, LINE_H));
            assert_eq!(l.lines[1].0.y, 43);
            assert_eq!(
                l.buttons,
                vec![
                    Rect::new(78, 129, 75, 23),
                    Rect::new(165, 129, 75, 23),
                    Rect::new(252, 129, 75, 23),
                ]
            );
            assert_eq!(d.key(Key::Enter), Outcome::TurnOn(aid));
            assert_eq!(d.key(Key::Escape), Outcome::Close);
            assert_eq!(d.press(1), Outcome::Close);
            assert_eq!(
                d.press(2),
                Outcome::Spawn(format!(
                    "reprise-settings --page accessibility --focus {}",
                    aid.key()
                ))
            );
        }
    }

    /// Renders the key aid prompts to `target/key-aid-preview.png`. Run
    /// with `cargo test key_aid_preview -- --ignored`.
    #[test]
    #[ignore]
    fn render_key_aid_preview() {
        use crate::keyfilter::Aid;
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let canvases: Vec<Canvas> = [Aid::StickyKeys, Aid::SlowKeys, Aid::MouseKeys]
            .into_iter()
            .map(|aid| {
                let d = Dialog::key_aid_prompt(aid);
                let l = DialogLayout::compute(&theme, &text, &d);
                draw(
                    &theme,
                    &text,
                    &l,
                    None,
                    None,
                    &[],
                    &[],
                    None,
                    &d,
                    true,
                    None,
                    Cues::ALL,
                    true,
                    None,
                )
            })
            .collect();
        let mut out = Canvas::new(410, 2 * 163 + 8);
        out.fill_rect(out.bounds(), theme.colors.background);
        out.blit_canvas(0, 0, &canvases[0]);
        out.blit_canvas(0, 171, &canvases[1]);
        reprisekit::raster::write_preview(&out, "key-aid-preview.png");
    }

    /// Renders the dialogs to `target/dialogs-preview.png`, and under Blue
    /// to `target/dialogs-blue-preview.png`, for eyeballing. Run with
    /// `cargo test preview -- --ignored`.
    #[test]
    #[ignore]
    fn render_dialogs_preview() {
        for (theme, name) in [
            (Theme::default(), "dialogs-preview.png"),
            (blue(), "dialogs-blue-preview.png"),
        ] {
            dialogs_preview(&theme, name);
        }
    }

    fn dialogs_preview(theme: &Theme, file: &str) {
        use crate::icontheme::{IconCache, IconTheme};
        let theme = theme.clone();
        let text = TextRenderer::load(&FontConfig::default());
        let name = std::env::var("REPRISE_PREVIEW_ICONS").unwrap_or_else(|_| "breeze".into());
        let mut icons = IconCache::new(IconTheme::load(&name, &IconTheme::default_roots()));
        let run = Dialog::run(&history());
        let mut open = Dialog::run(&history());
        open.key(Key::Drop);
        let props = Dialog::properties(&file_info(), "text-x-generic", (0, 0));
        let mut folder = file_info();
        folder.name = "New Folder".into();
        folder.hidden_ext = String::new();
        folder.path = "/home/u/Desktop/New Folder".into();
        folder.is_dir = true;
        folder.opens_with = None;
        folder.type_name = "File Folder".into();
        folder.contains = Some((2, 1));
        let folder = Dialog::properties(&folder, "folder", (0, 0));
        let dialogs = [
            run,
            open,
            props,
            folder,
            chooser(),
            {
                let mut c = chooser();
                c.key(Key::Up);
                c
            },
            attr_confirm(),
            Dialog::advanced_attributes(
                Subject::File,
                None,
                &text,
                theme.fonts.size_px,
                TextUnits::ONE,
            ),
            Dialog::advanced_attributes(
                Subject::Folder,
                None,
                &text,
                theme.fonts.size_px,
                TextUnits::ONE,
            ),
            Dialog::confirm_hide(crate::desktop::Place::Computer, "Computer"),
            Dialog::end_program("signal-desktop", 1234),
            Dialog::log_off(),
            Dialog::turn_off(&power()),
        ];
        let canvases: Vec<Canvas> = dialogs
            .into_iter()
            .map(|mut d| {
                d.fit(&theme, &text);
                let d = &d;
                let l = DialogLayout::compute(&theme, &text, d);
                let icon = d.icon.as_deref().and_then(|n| icons.get(n, ICON_PX));
                let caption = d
                    .icon
                    .as_deref()
                    .and_then(|n| icons.get(n, theme.icon_size()));
                let button_icons: Vec<Option<Canvas>> = d
                    .buttons
                    .iter()
                    .map(|b| b.icon.as_deref().and_then(|n| icons.get(n, ICON_PX)))
                    .collect();
                let logo = icons.get("start-here", ICON_PX);
                draw(
                    &theme,
                    &text,
                    &l,
                    icon.as_ref(),
                    caption.as_ref(),
                    &button_icons,
                    &[],
                    logo.as_ref(),
                    d,
                    true,
                    None,
                    Cues::ALL,
                    true,
                    None,
                )
            })
            .collect();
        let gap = 8;
        let w = canvases.iter().map(|c| c.width).max().unwrap() as i32 + 2 * gap;
        let h: i32 = canvases.iter().map(|c| c.height as i32 + gap).sum::<i32>() + gap;
        let mut out = Canvas::new(w as u32, h as u32);
        out.fill_rect(out.bounds(), theme.colors.background);
        let mut y = gap;
        for c in &canvases {
            out.blit_canvas(gap, y, c);
            y += c.height as i32 + gap;
        }
        reprisekit::raster::write_preview(&out, file);
    }

    /// Renders the tabbed sheets under each interface to
    /// `target/sheets-{classic,blue,glass}.png`: file and set Properties,
    /// Recycle Bin Properties on both tabs with Apply lit and the pointer
    /// over the closed tab. Run with
    /// `cargo test render_sheets_preview -- --ignored`.
    #[test]
    #[ignore]
    fn render_sheets_preview() {
        use crate::binprops::{self, BinSheet, Drive};
        use reprisekit::interface::Interface;
        for (i, file) in [
            (Interface::Classic, "sheets-classic.png"),
            (Interface::Blue, "sheets-blue.png"),
            (Interface::Glass, "sheets-glass.png"),
        ] {
            let theme = reprisekit::interfaces::theme_for(i);
            let text = TextRenderer::load(&FontConfig::default());
            let px = theme.fonts.size_px;
            let styled = theme.look.lights_pointer();
            let bin = |tab: usize| {
                let mut s = BinSheet::new(
                    reprisefs::bincap::Policy::default(),
                    vec![Drive {
                        key: "home".into(),
                        label: "Local Disk (/home)".into(),
                        size: 21_365_735_424,
                    }],
                );
                s.pending.global.remove_immediately = true;
                let mut d = binprops::dialog(s, (0, 0), "user-trash-full");
                if tab == 1 {
                    d.cycle_page(false);
                }
                d
            };
            let dialogs = [
                Dialog::properties(&file_info(), "text-x-generic", (0, 0)),
                Dialog::set_properties(&a_set(), "Reports", (0, 0)),
                bin(0),
                bin(1),
            ];
            let canvases: Vec<Canvas> = dialogs
                .into_iter()
                .map(|mut d| {
                    d.fit(&theme, &text);
                    let l = DialogLayout::compute(&theme, &text, &d);
                    if let Some(t) = l.tabs.last().filter(|_| l.tabs.len() > 1) {
                        let (x, y) = (t.x + 4, t.y + 6);
                        let over = l.hit(&d, x, y, &text, px);
                        d.hover(over, (x, y), styled);
                    }
                    draw(
                        &theme,
                        &text,
                        &l,
                        None,
                        None,
                        &[],
                        &[],
                        None,
                        &d,
                        false,
                        None,
                        Cues::ALL,
                        true,
                        None,
                    )
                })
                .collect();
            let gap = 8;
            let w = canvases.iter().map(|c| c.width).max().unwrap() as i32 + 2 * gap;
            let h: i32 = canvases.iter().map(|c| c.height as i32 + gap).sum::<i32>() + gap;
            let mut out = Canvas::new(w as u32, h as u32);
            out.fill_rect(out.bounds(), theme.colors.background);
            let mut y = gap;
            for c in &canvases {
                out.blit_canvas(gap, y, c);
                y += c.height as i32 + gap;
            }
            reprisekit::raster::write_preview(&out, file);
        }
    }

    fn render(theme: &Theme, text: &TextRenderer, l: &DialogLayout, d: &Dialog) -> Canvas {
        draw(
            theme,
            text,
            l,
            None,
            None,
            &[],
            &[],
            None,
            d,
            false,
            Lit::default(),
            Cues::NONE,
            true,
            None,
        )
    }

    /// At 150% and 200% Run grows inside its frame: its lines sit a grown line apart and fit
    /// their rows, the field holds a grown line, its open list's rows grow, and the buttons sit
    /// inside the dialog under the field. 100% is Run as laid out.
    /// A dialog open when the text size changes lays out as one opened at the new size: the
    /// config reload refits every open dialog (`configwatch::retheme`).
    #[test]
    fn an_open_dialog_lays_out_again_at_a_new_text_size() {
        use reprisekit::controls::filesheet;
        let Some(text) = tahoma() else { return };
        let one = Theme::default();
        let big = Theme::default().with_text_size(200);
        let opened = |theme: &Theme| {
            let (px, u) = (theme.fonts.size_px, theme.text);
            let path = std::path::Path::new("/tmp/a.txt");
            let mut ds = vec![
                Dialog::run(&history()),
                Dialog::confirm_delete("a.txt", path, false, false, false),
                Dialog::advanced_attributes(filesheet::Subject::File, None, &text, px, u),
            ];
            ds.iter_mut().for_each(|d| d.fit(theme, &text));
            ds
        };
        for (mut d, fresh) in opened(&one).into_iter().zip(opened(&big)) {
            d.fit(&big, &text);
            assert_eq!(
                DialogLayout::compute(&big, &text, &d),
                DialogLayout::compute(&big, &text, &fresh),
                "{}",
                d.title
            );
        }
    }

    fn measured_forms() -> Vec<Measured> {
        use reprisekit::controls::{endsession, locationprompt, replacebox};
        let side = |size: &str| replacebox::Side {
            size: size.into(),
            modified: "modified: Monday, September 14, 2026, 3:43:53 PM".into(),
            icon: None,
        };
        let replace = |subject, many| replacebox::Fields {
            name: "Quarterly report for the board.txt".into(),
            folder: None,
            subject,
            icon: None,
            many,
        };
        vec![
            Measured::Replace(replace(
                replacebox::Subject::File {
                    existing: Box::new(side("size: 8 bytes")),
                    incoming: Box::new(side("size: 12.4 KB")),
                },
                false,
            )),
            Measured::Replace(replace(replacebox::Subject::Folder, true)),
            Measured::EndSession(endsession::Fields {
                goal: Goal::Restart,
                rows: vec![endsession::Row {
                    name: "Text Editor".into(),
                    reason: "Unsaved changes".into(),
                }],
                icon: None,
            }),
            Measured::Location(locationprompt::Fields {
                program: "Maps".into(),
                icon: None,
            }),
            Measured::Attributes {
                verb: "Hide".into(),
                subject: Subject::Set { has_dir: true },
            },
            Measured::Advanced {
                subject: Subject::Folder,
                robots: Some([CheckState::On, CheckState::Off]),
            },
        ]
    }

    fn opened_measured(m: &Measured, theme: &Theme, text: &TextRenderer) -> Dialog {
        let (px, u) = (theme.fonts.size_px, theme.text);
        let form = FormDialog::measured(m.clone(), FormKind::EndSession, text, px, u);
        let mut d = Dialog::new("Measured", Style::Form(Box::new(form)), Placement::Center);
        d.fit(theme, text);
        d
    }

    #[test]
    fn the_copy_dialog_answers_its_keys_and_close_box_as_the_put_controls() {
        use reprisekit::controls::copyprogress::{CopyProgress, Kind};
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let cp = CopyProgress::new(Kind::Copy, "Downloads", "KINGSTON", Color::rgb(0, 0, 0));
        let mut d = Dialog::copy_progress(cp, "/media/KINGSTON".into());
        d.fit(&theme, &text);
        assert_eq!(d.title, "Copying...");
        assert_eq!(d.key(Key::Access('p')), Outcome::CopyPause(true));
        assert_eq!(d.key(Key::Escape), Outcome::CopyCancel);

        let layout = DialogLayout::compute(&theme, &text, &d);
        let client = layout.page.expect("a client");
        assert_eq!((client.width, client.height), d.copy().unwrap().cp.client());
        let px = theme.fonts.size_px;
        let cancel = (client.right() - 20, client.bottom() - 15);
        let hit = layout.hit(&d, cancel.0, cancel.1, &text, px);
        assert!(matches!(hit, Some(DialogHit::Control(_))), "{hit:?}");
        d.click(hit);
        assert_eq!(d.release(hit), Outcome::CopyCancel);
    }

    #[test]
    fn a_windowed_dialog_minimizes_from_the_box_left_of_close() {
        use reprisekit::controls::copyprogress::{CopyProgress, Kind};
        let text = TextRenderer::load(&FontConfig::default());
        let glass = reprisekit::interfaces::theme_for(reprisekit::interface::Interface::Glass);
        for theme in [Theme::default(), glass] {
            let cp = CopyProgress::new(Kind::Copy, "Downloads", "KINGSTON", Color::rgb(0, 0, 0));
            let mut d = Dialog::copy_progress(cp, "/media/KINGSTON".into());
            d.fit(&theme, &text);
            let l = DialogLayout::compute(&theme, &text, &d);
            let g = l.frame.expect("a frame");
            let min = l.minimize.expect("a minimize box");
            assert_eq!(min, g.button_rect(CaptionButton::Minimize));
            assert!(min.right() <= l.close.unwrap().x + 1, "left of close");
            let px = theme.fonts.size_px;
            let hit = l.hit(&d, min.x + 2, min.y + 2, &text, px);
            assert_eq!(hit, Some(DialogHit::Minimize));
            d.press_caption(CaptionButton::Minimize);
            assert!(d.drag_caption(None), "dragged off");
            assert!(d.drag_caption(hit), "back on");
            assert_eq!(d.release_caption(), Some(CaptionButton::Minimize));

            let run = DialogLayout::compute(&theme, &text, &Dialog::run(&[]));
            assert_eq!(
                run.minimize, None,
                "a dialog over the windows has no minimize box"
            );
        }
    }

    #[test]
    fn the_copy_dialog_is_a_windowed_task_with_the_jobs_progress() {
        use reprisekit::controls::copyprogress::{CopyProgress, Kind, Phase, Status};
        let mut cp = CopyProgress::new(Kind::Copy, "Downloads", "KINGSTON", Color::rgb(0, 0, 0));
        let st = Status {
            phase: Phase::Copying,
            paused: true,
            bytes: 25,
            total_bytes: 100,
            ..Status::default()
        };
        cp.update(st, 0);
        let d = Dialog::copy_progress(cp, "/media/KINGSTON".into());
        assert!(d.windowed());
        assert!(d.on_taskbar());
        assert_eq!(d.task_icon(), Some("system-file-manager"));
        let p = d.task_progress().expect("a copy's progress");
        assert_eq!((p.fraction, p.paused), (0.25, true));
        assert_eq!(Dialog::end_program("foot", 4321).task_progress(), None);
    }

    #[test]
    fn a_measured_dialog_is_built_again_at_a_new_text_size_and_font() {
        let Some(tahoma) = tahoma() else { return };
        let dejavu = TextRenderer::load(&FontConfig {
            family: vec!["DejaVu Sans".into()],
            ..FontConfig::default()
        });
        let one = Theme::default();
        let big = Theme::default().with_text_size(200);
        let mut moves = vec![
            ((&one, &tahoma), (&big, &tahoma)),
            ((&big, &tahoma), (&one, &tahoma)),
        ];
        if dejavu.has_face("DejaVu Sans") {
            moves.push(((&one, &dejavu), (&one, &tahoma)));
        }
        for (i, m) in measured_forms().iter().enumerate() {
            for &((from, from_text), (to, to_text)) in &moves {
                let mut d = opened_measured(m, from, from_text);
                d.fit(to, to_text);
                assert!(d == opened_measured(m, to, to_text), "form {i}");
            }
        }
    }

    #[test]
    fn a_measured_dialog_built_again_keeps_what_its_controls_hold() {
        use reprisekit::controls::form::Control;
        use reprisekit::controls::{endsession, locationprompt};
        let text = TextRenderer::load(&FontConfig::default());
        let big = Theme::default().with_text_size(200);
        for m in measured_forms() {
            let mut d = opened_measured(&m, &Theme::default(), &text);
            let Some(form) = d.form_mut().and_then(|f| f.open_page_mut()) else {
                panic!("a plain form");
            };
            let rows = vec![endsession::Row {
                name: "Mail".into(),
                reason: String::new(),
            }];
            match m {
                Measured::Location(_) => form.set_check(locationprompt::REMEMBER, false),
                Measured::EndSession(_) => assert!(endsession::set_rows(form, &rows)),
                _ => {}
            }
            let last = form.items.iter().rev().find_map(|it| match it.control {
                Control::Button { .. } => Some(it.id),
                _ => None,
            });
            form.focus_id(last.expect("a button"));
            let before = form.clone();
            d.fit(&big, &text);
            let after = d.form().and_then(|f| f.open_page()).unwrap();
            let focused = |f: &reprisekit::controls::form::Form| f.focus().map(|i| f.items[i].id);
            assert_eq!(focused(after), focused(&before));
            for a in &after.items {
                if !matches!(a.control, Control::Label { .. } | Control::Lines(_)) {
                    assert_eq!(Some(&a.control), before.item(a.id).map(|b| &b.control));
                }
            }
        }
    }

    /// The usable area of a 1920x1080 output over a 40 px taskbar.
    const AREA_1080P: (i32, i32) = (1920, 1040);

    /// The tray's Customize dialog in `theme`'s words over three items, as the tray opens it.
    fn tray_dialog(theme: &Theme) -> Dialog {
        use reprisekit::controls::traycustomize::{Behaviour, Entry, Wording};
        let entry = |key: &str, name: &str| Entry {
            key: key.into(),
            name: name.into(),
            detail: format!("{name}: all is well"),
            icon: std::sync::Arc::from([]),
            passive: false,
            behaviour: Behaviour::Auto,
        };
        let w = Wording::of(theme);
        Dialog::tray_customize(TrayCustomize::new(
            w,
            vec![entry("nm-applet", "Network"), entry("volume", "Volume")],
            vec![entry("sticky", "Sticky Keys")],
            (w == Wording::Glass).then_some(false),
            theme.look.controls(),
        ))
    }

    /// At 100% the tray's Customize page keeps its 1184x602 client on a 1080p output; at 150% it
    /// grows with the text, and at 200% it fits the output, its page scrolling above the band.
    /// A text size change while it is open lays it out again.
    #[test]
    fn the_tray_customize_page_grows_and_fits_its_output() {
        let text = TextRenderer::load(&FontConfig::default());
        let glass = reprisekit::interfaces::theme_for(reprisekit::interface::Interface::Glass);
        let mut d = tray_dialog(&glass);
        let client = |d: &Dialog| d.tray().unwrap().client();
        d.fit_in(&glass, &text, AREA_1080P);
        assert_eq!(client(&d), (1184, 602));
        let big = glass.with_text_size(150);
        d.fit_in(&big, &text, AREA_1080P);
        assert_eq!(client(&d), big.text.size((1184, 602)));
        let huge = glass.with_text_size(200);
        d.fit_in(&huge, &text, AREA_1080P);
        let l = DialogLayout::compute(&huge, &text, &d);
        assert!(
            l.size.0 <= AREA_1080P.0 && l.size.1 <= AREA_1080P.1,
            "{:?}",
            l.size
        );
        let client = l.page.unwrap();
        let tray = d
            .tray()
            .unwrap()
            .layout(&text, huge.fonts.size_px, (client.x, client.y));
        assert!(tray.page_bar.is_some(), "the page scrolls");
        assert_eq!(tray.band.unwrap().bottom(), client.bottom());
        d.fit_in(&glass, &text, AREA_1080P);
        assert_eq!(d.tray().unwrap().client(), (1184, 602), "back at 100%");
    }

    /// Every compositor dialog laid out as a form, under each interface, at each text size.
    fn form_dialogs(theme: &Theme, text: &TextRenderer) -> Vec<Dialog> {
        use reprisekit::controls::iconlist::IconList;
        let (px, u) = (theme.fonts.size_px, theme.text);
        let mut folder = file_info();
        folder.is_dir = true;
        folder.opens_with = None;
        folder.name = "Reports".into();
        folder.hidden_ext = String::new();
        folder.path = "/home/u/Desktop/Reports".into();
        folder.contains = Some((12, 3));
        let mut launcher = file_info();
        launcher.name = "Shell".into();
        launcher.hidden_ext = String::new();
        launcher.path = "/home/u/Desktop/shell.desktop".into();
        launcher.is_shortcut = true;
        launcher.launcher = Some(reprisefs::launcher::Launcher {
            target: "/bin/sh".into(),
            ..Default::default()
        });
        let bin = crate::binprops::BinSheet::new(
            reprisefs::bincap::Policy::default(),
            vec![crate::binprops::Drive {
                key: "home".into(),
                label: "Local Disk (/home)".into(),
                size: 21_365_735_424,
            }],
        );
        let confirm = |write| AttrConfirm {
            write,
            attrs: reprisefs::props::AttrMoves {
                read_only: Some(true),
                ..Default::default()
            },
            close: true,
        };
        let one = AttrWrite::One {
            path: "/d/Reports".into(),
            name: Some(".Reports".into()),
            hidden: Some(true),
            folder_icon: None,
        };
        let set = AttrWrite::Set {
            paths: vec!["/d/a".into(), "/d/Reports".into()],
            hidden: Some(true),
            folder_icon: None,
        };
        let icons = IconList::new(
            ["folder", "folder-music"].map(String::from).to_vec(),
            vec![None, None],
            None,
        );
        let mut zip = file_info();
        zip.opens_with = Some("Compressed (zipped) Folders".into());
        let mut ds = vec![
            Dialog::properties(&file_info(), "text-x-generic", (0, 0)),
            Dialog::properties(&zip, "package-x-generic", (0, 0)),
            Dialog::properties(&folder, "folder", (0, 0)),
            Dialog::properties(&launcher, "utilities-terminal", (0, 0)),
            Dialog::set_properties(&a_set(), "Reports", (0, 0)),
            crate::binprops::dialog(bin, (0, 0), "user-trash"),
            Dialog::change_icon("Reports", "theme", icons, false),
            Dialog::advanced_attributes(Subject::File, None, text, px, u),
            Dialog::advanced_attributes(Subject::Folder, None, text, px, u),
            Dialog::advanced_attributes(Subject::Set { has_dir: true }, None, text, px, u),
            Dialog::confirm_attributes(confirm(one), text, px, u),
            Dialog::confirm_attributes(confirm(set), text, px, u),
        ];
        // Dates in a long weekday and month, whatever the locale.
        let date = "Wednesday, September 30, 2026, 12:00:00 PM";
        for d in &mut ds[..4] {
            let page = &mut d.form_mut().unwrap().pages_mut()[0];
            for id in ["v_made", "v_mod", "v_acc"] {
                page.set_text(id, date);
            }
        }
        ds.iter_mut().for_each(|d| d.fit(theme, text));
        ds.extend(
            measured_forms()
                .iter()
                .map(|m| opened_measured(m, theme, text)),
        );
        ds
    }

    /// Every compositor dialog drawn from a template or as a Welcome panel.
    fn template_dialogs() -> Vec<Dialog> {
        let path = std::path::Path::new("/home/u/Desktop/Quarterly report.txt");
        let paths: Vec<std::path::PathBuf> = vec!["/d/a".into(), "/d/b".into()];
        let too_big = reprisefs::bincap::TooBig::new(
            vec!["/d/big.iso".into()],
            vec![Some(reprisefs::bincap::Measured::unstamped(2, 1))],
        );
        let mut ds = vec![
            Dialog::run(&history()),
            Dialog::log_off(),
            Dialog::turn_off(&power()),
            Dialog::confirm_delete("Quarterly report.txt", path, false, false, false),
            Dialog::confirm_delete("Reports", path, true, true, true),
            Dialog::confirm_delete_many(&paths, false, BinLack::None),
            Dialog::confirm_delete_many(&paths, true, BinLack::All),
            Dialog::too_big(std::path::Path::new("/d/big.iso"), false, too_big),
            Dialog::confirm_delete_icons(vec!["computer".into()], paths.clone()),
            Dialog::end_program("Text Editor", 42),
            Dialog::error("Run", "Reprise cannot find 'frobnicate'.".into()),
            Dialog::confirm_empty_bin(3, None),
            Dialog::confirm_empty_bin(1, Some("Quarterly report.txt")),
        ];
        ds.extend(
            [
                crate::desktop::Place::Documents,
                crate::desktop::Place::Computer,
                crate::desktop::Place::Home,
            ]
            .map(|p| Dialog::confirm_hide(p, &p.label())),
        );
        ds.extend(
            [
                crate::keyfilter::Aid::StickyKeys,
                crate::keyfilter::Aid::SlowKeys,
                crate::keyfilter::Aid::MouseKeys,
            ]
            .map(Dialog::key_aid_prompt),
        );
        ds
    }

    /// Forms through `Form::overflows`; templates by their lines, field label and push buttons.
    #[test]
    fn every_dialog_fits_its_text_under_every_interface() {
        use reprisekit::interface::Interface;
        let mut over = Vec::new();
        for family in ["Tahoma", "DejaVu Sans"] {
            let text = TextRenderer::load(&FontConfig {
                family: vec![family.into()],
                ..FontConfig::default()
            });
            if !text.has_face(family) {
                continue;
            }
            for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
                for pct in [100, 125, 150, 175, 200] {
                    let theme = reprisekit::interfaces::theme_for(i).with_text_size(pct);
                    let px = theme.fonts.size_px;
                    let mut put = |title: &str, what: String| {
                        over.push(format!("{family}, {}, {pct}%, {title}: {what}", i.as_str()))
                    };
                    for d in form_dialogs(&theme, &text) {
                        for (n, page) in d.form().unwrap().pages().iter().enumerate() {
                            for (id, by) in page.overflows(&text, px) {
                                put(&d.title, format!("page {n}, {id} {by} px over"));
                            }
                        }
                    }
                    for mut d in template_dialogs() {
                        d.fit(&theme, &text);
                        let l = DialogLayout::compute(&theme, &text, &d);
                        let t = text.dialog(d.font());
                        let width = |s: &str| t.measure(&split_mnemonic(s).0, false, px);
                        for (r, line) in &l.lines {
                            if width(line) > r.width {
                                put(
                                    &d.title,
                                    format!("{line:?} {} px over", width(line) - r.width),
                                );
                            }
                        }
                        if let (Some(f), Some(r)) = (&d.field, l.field_label)
                            && width(&f.label) > r.width
                        {
                            put(&d.title, format!("{:?} over", f.label));
                        }
                        if matches!(d.style, Style::Welcome) {
                            continue;
                        }
                        for (b, r) in d.buttons.iter().zip(&l.buttons) {
                            if width(&b.label) > r.width - theme.text.len(6) {
                                put(&d.title, format!("{:?} over its button", b.label));
                            }
                        }
                    }
                    let mut d = tray_dialog(&theme);
                    d.fit_in(&theme, &text, AREA_1080P);
                    let l = DialogLayout::compute(&theme, &text, &d);
                    if l.size.0 > AREA_1080P.0 || l.size.1 > AREA_1080P.1 {
                        put(&d.title, format!("{:?} past a 1080p output", l.size));
                    }
                    for (part, by) in d.tray().unwrap().overflows(&text, px) {
                        put(&d.title, format!("{part} {by} px over"));
                    }
                }
            }
        }
        assert!(over.is_empty(), "{}", over.join("\n"));
    }

    #[test]
    fn run_grows_with_the_text_size() {
        let Some(text) = tahoma() else { return };
        let one = DialogLayout::compute(&Theme::default(), &text, &Dialog::run(&history()));
        assert_eq!(one.size, (347, 179));
        for pct in [150, 200] {
            let theme = Theme::default().with_text_size(pct);
            let (u, px) = (theme.text, theme.fonts.size_px);
            let mut d = Dialog::run(&history());
            d.fit(&theme, &text);
            let l = DialogLayout::compute(&theme, &text, &d);
            let caption_grow = theme.metrics.caption_height - 18;
            assert_eq!(l.size.0, 2 * 3 + u.len(347 - 6), "{pct}%");
            assert_eq!(l.size.1, 21 + caption_grow + u.len(179 - 24) + 3, "{pct}%");
            for pair in l.lines.windows(2) {
                assert!(pair[0].0.bottom() <= pair[1].0.y, "{pct}%: lines apart");
            }
            for (r, line) in &l.lines {
                assert!(text.measure(line, false, px) <= r.width, "{pct}%: {line:?}");
                assert!(r.height >= u.len(LINE_H) - 1, "{pct}%");
            }
            let field = l.field.unwrap();
            assert!(
                field.height >= px as i32 + 6,
                "{pct}%: the field holds a line"
            );
            assert!(l.lines.last().unwrap().0.bottom() <= field.y);
            let list = l.list.unwrap();
            assert_eq!(list.height, LIST_ROWS as i32 * u.len(13) + 2, "{pct}%");
            for b in &l.buttons {
                assert!(b.y >= field.bottom(), "{pct}%");
                assert!(
                    b.right() <= l.size.0 - 3 && b.bottom() <= l.size.1 - 3,
                    "{pct}%"
                );
            }
            let c = render(&theme, &text, &l, &d);
            assert_eq!((c.width as i32, c.height as i32), l.size);
        }
    }

    /// A message box and a form dialog at 150%: the template's client grows by the units, and a
    /// form dialog's outer size and its page's controls grow from the client's corner.
    #[test]
    fn prompts_and_form_dialogs_grow_with_the_text_size() {
        use reprisekit::controls::filesheet;
        let Some(text) = tahoma() else { return };
        let theme = Theme::default().with_text_size(150);
        let u = theme.text;
        let d = Dialog::confirm_delete(
            "a.txt",
            std::path::Path::new("/tmp/a.txt"),
            false,
            false,
            false,
        );
        let l = DialogLayout::compute(&theme, &text, &d);
        let base = DialogLayout::compute(&Theme::default(), &text, &d);
        assert_eq!(l.size.0, 6 + u.len(base.size.0 - 6));
        for b in &l.buttons {
            assert!((u.len(23) - 1..=u.len(23) + 1).contains(&b.height));
            assert!(b.right() <= l.size.0 - 3 && b.bottom() <= l.size.1 - 3);
        }
        let icon = l.icon.unwrap();
        assert_eq!((icon.width, icon.height), (ICON_PX, ICON_PX));

        let mut adv = Dialog::advanced_attributes(
            filesheet::Subject::File,
            None,
            &text,
            theme.fonts.size_px,
            u,
        );
        adv.fit(&theme, &text);
        let l = DialogLayout::compute(&theme, &text, &adv);
        let f = adv.form().unwrap();
        let caption_grow = theme.metrics.caption_height - 18;
        let (w, h) = filesheet::advanced_size(filesheet::Subject::File);
        assert_eq!(l.size, (6 + u.len(w), 21 + caption_grow + u.len(h) + 3));
        let page = f.open_page().unwrap();
        let ok = page.rect(filesheet::ADV_OK).unwrap();
        let design = page.item(filesheet::ADV_OK).unwrap().rect;
        assert_eq!(ok.width, u.len(design.right()) - u.len(design.x));
        assert!(ok.right() <= l.size.0 && ok.bottom() <= l.size.1);
        let c = render(&theme, &text, &l, &adv);
        assert_eq!((c.width as i32, c.height as i32), l.size);
    }
}
