//! Compositor dialogs (Run, confirmations, Properties, Open With, Welcome panels, forms): model,
//! layout, drawing. `dialogview.rs` puts them on the scene.

use self::Kind as FormKind;
use crate::config::Action;
use crate::raster::Rect;
use crate::text::{DialogFont, TextRenderer};
use crate::textedit::TextEdit;
use crate::theme::Theme;
use reprisekit::classic::widgets::CheckState;
use reprisekit::controls::combo::Combo;
use reprisekit::controls::copyprogress::CopyProgress;
use reprisekit::controls::dlgtemplate::Template;
use reprisekit::controls::endsession::Goal;
use reprisekit::controls::openwith;
use reprisekit::controls::scrollbar::ThumbDrag;
use reprisekit::controls::traycustomize::{self, TrayCustomize};
use reprisekit::frame::{CaptionButton, Edges};
use reprisekit::pointer::Pointer;

mod build;
mod draw;
mod form;
mod input;
mod layout;
mod sheet;
#[cfg(test)]
mod tests;
mod welcome;

pub use self::draw::{access, access_copy, draw, draw_copy, draw_field, page_pictures};
pub use self::form::{FormBody, FormDialog, Kind, Measured};
use self::input::redraw_if;
pub(crate) use self::input::tray_part;
pub(crate) use self::layout::FORM_CLIENT;
use self::layout::{CAPTION_H, DIALOG_FRAME, client_shift};
pub use self::layout::{DialogHit, DialogLayout, ICON_PX, caption_button};
pub use self::sheet::{AttrConfirm, AttrWrite, SetBase, SheetBase, Written, shortcut_facts};
use self::sheet::{props_event, set_event, sheet_over};
use self::welcome::{IconButton, draw_icon_button, draw_welcome};

/// What pressing a button (or Enter for the default one) does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DialogAction {
    Cancel,
    /// Run the text field's content; inert while it is empty.
    RunField,
    /// Run's Browse...: open Browse, the pick lands in the field.
    BrowseRun,
    Spawn(String),
    /// Run this `[power]` standby command, reporting a refusal (`sessionend::sleep`).
    StandBy(String),
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
    /// lays itself out in the client a frame that size leaves, and a sheet larger than that
    /// client scrolls its page through the rest.
    pub fn fit_in(&mut self, theme: &Theme, text: &TextRenderer, area: (i32, i32)) {
        if let Style::TrayCustomize(t) = &mut self.style {
            let pad = DialogLayout::compute_client((0, 0))
                .framed(theme, self.help)
                .size;
            t.room = Some((area.0 - pad.0, area.1 - pad.1));
        }
        self.fit(theme, text);
        if let Style::Form(f) = &mut self.style {
            f.fit_area(theme.text, text, theme.fonts.size_px, area);
        }
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
    StandBy(String),
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
