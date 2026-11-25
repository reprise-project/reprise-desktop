//! Compositor dialogs (Run, confirmations, Properties, Open With, Welcome panels, forms): model,
//! layout, drawing. `dialogview.rs` puts them on the scene.

use self::Kind as FormKind;
use crate::config::Action;
use crate::raster::Rect;
use crate::text::{DialogFont, TextRenderer};
use crate::textedit::{EditKey, TextEdit};
use crate::theme::Theme;
use reprisekit::controls::combo::Combo;
use reprisekit::controls::dlgtemplate::Template;
use reprisekit::controls::endsession::Goal;
use reprisekit::controls::pathcomplete::{self, PathComplete};
use reprisekit::controls::scrollbar::ThumbDrag;
use reprisekit::controls::traycustomize::{self, TrayCustomize};
use reprisekit::frame::CaptionButton;
use reprisekit::pointer::Pointer;
use reprisekit::powerpanel::{Choice, First, Leave, Panel};

mod build;
mod draw;
mod form;
mod input;
mod layout;
#[cfg(test)]
mod tests;
mod welcome;

pub use self::draw::{access, draw, draw_field};
pub use self::form::{FormDialog, Kind, Measured};
pub(crate) use self::input::tray_part;
pub(crate) use self::layout::FORM_CLIENT;
use self::layout::{CAPTION_H, DIALOG_FRAME, client_shift};
pub use self::layout::{DialogHit, DialogLayout, ICON_PX, caption_button};
use self::welcome::{IconButton, draw_icon_button, draw_welcome};

/// What pressing a button (or Enter for the default one) does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DialogAction {
    Cancel,
    /// Run the text field's content; inert while it is empty.
    RunField,
    /// Name desktop n the text field's content; empty gives back its number.
    RenameDesktop(usize),
    /// Run's Browse...: open Browse, the pick lands in the field.
    BrowseRun,
    Spawn(String),
    /// Run this `[power]` standby command, reporting a refusal (`sessionend::sleep`).
    StandBy(String),
    /// Run this `[power]` hibernate command, as `StandBy` does.
    Hibernate(String),
    /// End the session toward `Goal`, then run this command (`sessionend`).
    EndSession(Goal, String),
    /// Turn Off or Restart from the Turn off computer panel while an offline update is prepared:
    /// [`DialogAction::EndSession`] doing as the Install updates check box says (`offlineupdate`).
    EndSessionUpdates {
        goal: Goal,
        cmd: String,
        updates: reprisekit::powerpanel::Updates,
    },
    /// The Turn off computer panel's Install updates check box.
    ToggleUpdates,
    /// Lock and hand the screen to the login screen (`loginlink::switch_user`).
    SwitchUser,
    /// Lock the console and let every remote client go (`console::disconnect`).
    Disconnect,
    /// Ask reprise-login to turn the machine off or restart it (`greeterview::power`).
    GreeterPower(repriselogin::greeter::PowerAction),
    /// Open the boot entries to restart into under the button (`restartinto`).
    RestartInto,
    /// Go ahead with a power action a restart target held back (`sessionend::anyway`).
    PowerAnyway(crate::sessionend::Again),
    Act(Action),
    HideSystemIcon(String),
    /// Take these `system_icons` off the desktop (none for No), then ask about deleting `files`.
    DeleteIcons {
        hide: Vec<String>,
        files: Vec<std::path::PathBuf>,
    },
    EndProgram(i32),
    TurnOn(crate::keyfilter::Aid),
    /// The run question's answer for the `program` file at `path`: run it (a script as `run`
    /// says), or (None) open it in Notepad.
    Script {
        path: std::path::PathBuf,
        run: Option<reprisefs::program::ScriptOpen>,
        program: reprisefs::program::Program,
    },
    /// The run question's Run for a file its type's program runs: open it in `with` (a desktop
    /// entry id), else its default program, into the recent files with `recent`.
    OpenRunnable {
        path: std::path::PathBuf,
        with: Option<reprisefs::apps::DesktopId>,
        recent: bool,
    },
    /// The Mount Disk Image question's Mount for the disk image at this path.
    MountImage(std::path::PathBuf),
    /// The desktop Undo question's Yes: undo this entry again, deleting its item for good.
    UndoErase(reprisefs::undo::EntryId),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DialogButton {
    /// Label with an optional `&` access key.
    pub label: String,
    pub action: DialogAction,
    /// Drawn with the black default ring; Enter presses it.
    pub default: bool,
    /// A Welcome panel's big button: what it offers, drawn as its tile above the label.
    pub choice: Option<Choice>,
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
    /// A typed path's completion.
    pub complete: PathComplete,
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

    /// An editing key into the field, through its completion.
    fn edit_key(&mut self, k: EditKey) -> bool {
        match self.combo.edit.as_mut() {
            Some(e) => self.complete.edit(e, k),
            None => false,
        }
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
    /// Log Off or Turn off computer: the Blue `powerpanel`, or under glass a framed dialog with
    /// push buttons unless the panel is to be Blue there too.
    Welcome(Box<Welcome>),
    /// A form laid out around its measured text: the session-end prompt, the location prompt.
    Form(Box<FormDialog>),
    /// End Program: classic dialog on a fixed 354x243 template (see `EP_PARA_Y`).
    EndProgram,
    /// The `traycustomize` control in a fixed frame.
    TrayCustomize(Box<TrayCustomize>),
}

/// A Welcome panel's model and what each of its choices does; `d.buttons` are drawn from them
/// ([`Dialog::sync_welcome`]).
#[derive(Clone, Debug, PartialEq)]
pub struct Welcome {
    pub panel: Panel,
    pub actions: Vec<(Choice, DialogAction)>,
    /// Drawn Blue under glass too, as the Blue Start menu opens it.
    pub always_blue: bool,
    /// Offers the Install updates check box while an offline update is prepared.
    pub updates: bool,
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

    /// Fit a form dialog to `theme`: a form laid out around measured text is built again, and a
    /// larger frame grows it.
    pub fn fit(&mut self, theme: &Theme, text: &TextRenderer) {
        match &mut self.style {
            Style::Form(f) => {
                f.remeasure(text, theme.fonts.size_px, theme.text);
                let p = theme.paint();
                f.form.drawn_with(&p);
                f.fit_frame(client_shift(theme));
            }
            Style::TrayCustomize(t) => {
                t.metrics = theme.controls();
                t.set_wording(traycustomize::Wording::of(theme));
                t.fit(text, theme.fonts.size_px);
                self.title = t.wording.title().into();
            }
            _ => {}
        }
    }

    /// What makes this dialog "the same window" as one already open: its title, its text and,
    /// for a message box, what its first button other than Cancel does. Two questions about
    /// hiding different place icons both stay up, as do two errors with one title and different
    /// text, while Run stays single.
    pub fn identity(&self) -> (&str, &str, Option<&DialogAction>) {
        let acts = match self.style {
            Style::Classic { .. } => (self.buttons.iter())
                .map(|b| &b.action)
                .find(|a| **a != DialogAction::Cancel),
            _ => None,
        };
        (self.title.as_str(), self.text.as_str(), acts)
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
    /// program.
    pub fn on_taskbar(&self) -> bool {
        matches!(self.style, Style::EndProgram)
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
}

/// What the view does after a key or click.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Nothing,
    Redraw,
    Close,
    Spawn(String),
    StandBy(String),
    Hibernate(String),
    EndSession(Goal, String),
    /// [`DialogAction::EndSessionUpdates`].
    EndSessionUpdates {
        goal: Goal,
        cmd: String,
        updates: reprisekit::powerpanel::Updates,
    },
    /// The session-end prompt's answer: End Now or Cancel.
    SessionEnd(reprisekit::controls::endsession::Choice),
    /// The location prompt's answer for the program `desktop_id`.
    Location {
        desktop_id: String,
        answer: reprisekit::controls::locationprompt::Answer,
    },
    SwitchUser,
    Disconnect,
    GreeterPower(repriselogin::greeter::PowerAction),
    /// [`DialogAction::RestartInto`], pressed from the keyboard with `keys`.
    RestartInto {
        keys: bool,
    },
    /// [`DialogAction::PowerAnyway`].
    PowerAnyway(crate::sessionend::Again),
    Notice(Box<Dialog>),
    /// [`DialogAction::Script`].
    Script {
        path: std::path::PathBuf,
        run: Option<reprisefs::program::ScriptOpen>,
        program: reprisefs::program::Program,
    },
    /// [`DialogAction::OpenRunnable`].
    OpenRunnable {
        path: std::path::PathBuf,
        with: Option<reprisefs::apps::DesktopId>,
        recent: bool,
    },
    /// [`DialogAction::MountImage`].
    MountImage(std::path::PathBuf),
    /// [`DialogAction::UndoErase`].
    UndoErase(reprisefs::undo::EntryId),
    /// Run what the user typed in the field: spawned like `Spawn`, and
    /// remembered in the Run history.
    Run(String),
    /// [`DialogAction::RenameDesktop`] with the field's trimmed text.
    RenameDesktop(usize, String),
    Act(Action),
    BrowseRun,
    HideSystemIcon(String),
    DeleteIcons {
        hide: Vec<String>,
        files: Vec<std::path::PathBuf>,
    },
    EndProgram(i32),
    TurnOn(crate::keyfilter::Aid),
    TrayChoices(traycustomize::Choices),
}
