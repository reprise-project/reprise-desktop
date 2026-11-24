//! Explorer's modal dialogs in a child window: the error box, the delete and replace confirmations, the Open With chooser, the Properties sheet's Advanced Attributes, Confirm Attribute Changes and Change Icon, the Move and Copy folder picker (`browse`), Unlock Drive, and Connect to Server with its password and question boxes.
//!
//! One child at a time: a box raised while another is up waits for it ([`Explorer::raise`]). Two stack instead: the chooser's Browse... opens the program picker (`PROGRAM_BROWSE`) over it, and a refused name or a failed Make New Folder in the Move/Copy picker raises its error box (`browse::ERROR`). The message, question, confirm and replace boxes keep only their fields and focused button, so their [`Form`] is rebuilt per event ([`Explorer::box_form`]); the chooser keeps an `openwith` control that takes its own events ([`crate::openwith`]).

use crate::icons::{IconSlot, land_picture};
use crate::place;
use crate::props_dialog::{ADV_CANCEL, ADV_OK, ATTR_CANCEL, ATTR_OK, ATTR_SIZE, AttrConfirm};
use crate::{BTN_LEFT, Explorer};
use repriseapp::filedlg::Host;
use repriseapp::unlock::{self, Unlock};
use repriseapp::{Caption, Child, Ctx, Event, PointerInput, Reply, WinId};
use reprisekit::controls::aboutbox;
use reprisekit::controls::browsefolder;
use reprisekit::controls::dlgtemplate::{self, FOLDER_TEXT_H, Template};
use reprisekit::controls::filedlg::FileDlg;
use reprisekit::controls::form::{self, Form};
use reprisekit::controls::msgbox;
use reprisekit::controls::openwith::{self, OpenWith};
use reprisekit::controls::replacebox;
use reprisekit::look::Paint;
use reprisekit::raster::Canvas;
use std::path::PathBuf;

/// The dialog's own window; each is modal and one at a time.
pub const DIALOG: WinId = WinId::named("dialog");

/// The box window's twin: a box that comes up after another takes the one the last is not in
/// ([`Explorer::dialog_win`]).
pub const NEXT_DIALOG: WinId = DIALOG.named_in("next");

pub(crate) fn is_dialog(win: WinId) -> bool {
    win == DIALOG || win == NEXT_DIALOG
}

pub const PROGRAM_BROWSE: WinId = WinId::named("program browse");

pub use reprisefs::bincap::ConfirmKind;

pub struct Confirm {
    pub title: &'static str,
    pub text: String,
    pub min_text_h: Option<i32>,
    pub template: Template,
    pub icon: IconSlot,
    pub paths: Vec<PathBuf>,
    pub erase: bool,
    /// The Recycle Bin's Empty task raised it: Yes empties the trash
    /// rather than deleting `paths`.
    pub empty_bin: bool,
    /// Raised in the Recycle Bin: Yes deletes these items for good, as
    /// they were listed when the prompt went up.
    pub bin: Option<Vec<crate::fileops::BinItem>>,
    /// The button Tab and the arrows moved the focus to, which each
    /// event's new form gets back; `None` ([`Confirm::default_button`]) when the prompt opens.
    pub focus: Option<&'static str>,
    /// The question is about an item too big for its Recycle Bin: Yes
    /// erases it and No leaves it, and a batch of more than one item
    /// adds Yes to All and Cancel. The answers go to this batch.
    pub too_big: Option<reprisefs::bincap::TooBig>,
    /// Undo stopped at an item its Recycle Bin refuses: Yes undoes this entry again, erasing it,
    /// while it is still the one Undo reverses next ([`crate::Explorer::undo_with`]).
    pub undo_erase: Option<reprisefs::undo::EntryId>,
    /// The question before running `paths[0]` (`confirm_scripts`): Run runs a script as the
    /// `ScriptOpen` says, Open in Notepad opens it there.
    pub script: Option<(reprisefs::program::ScriptOpen, reprisefs::program::Asked)>,
    /// The question before mounting the disk image `paths[0]`: Mount mounts it.
    pub mount: bool,
    /// An item of a job failed and the job holds for the answer: Skip, Retry or Cancel
    /// ([`reprisefs::progress::Decision`]).
    pub job: Option<reprisefs::progress::Asked>,
}

/// The buttons of the box a failed item raises, in [`Explorer::ask_failed`]'s order.
pub(crate) const FAILED_BUTTONS: [&str; 3] = ["&Skip", "&Retry", "Cancel"];

/// The icon of the error boxes, and of the questions before running or mounting a file.
pub(crate) const ERROR_ICON: &[&str] = &["dialog-error", "dialog-warning"];
pub(crate) const QUESTION_ICON: &[&str] = &["dialog-warning", "dialog-question"];
pub(crate) const TRASH_ICON: &[&str] = &["user-trash-full"];

impl Confirm {
    pub(crate) fn buttons(&self) -> Vec<String> {
        if let Some((_, asked)) = self.script {
            return asked.buttons().iter().map(|s| s.to_string()).collect();
        }
        if self.mount {
            return reprisefs::udisks::MOUNT_BUTTONS.map(String::from).to_vec();
        }
        let labels: &[&str] = match &self.too_big {
            _ if self.job.is_some() => &FAILED_BUTTONS,
            Some(q) if q.many() => &[dlgtemplate::YES, "Yes to &All", dlgtemplate::NO, "Cancel"],
            _ => &[dlgtemplate::YES, dlgtemplate::NO],
        };
        labels.iter().map(|s| s.to_string()).collect()
    }

    /// The button Enter presses when the box opens: No on a delete that erases, Cancel on the
    /// mount and program questions, Run on the script question.
    pub(crate) fn default_button(&self) -> usize {
        let cancel_last = match self.script {
            Some((_, asked)) => asked != reprisefs::program::Asked::Script,
            None => self.mount,
        };
        if cancel_last {
            return self.buttons().len() - 1;
        }
        dlgtemplate::delete_default_index(&self.buttons(), self.erase)
    }

    /// The box's client layout for text `text` sets at `px`, which grows by `u`.
    pub(crate) fn layout(
        &self,
        text: &reprisekit::text::TextRenderer,
        px: u32,
        u: reprisekit::theme::TextUnits,
    ) -> dlgtemplate::Layout {
        dlgtemplate::layout(
            self.template,
            text,
            px,
            &dlgtemplate::Fields {
                text: &self.text,
                icon: true,
                field: false,
                buttons: &self.buttons(),
                text_h: self.min_text_h,
            },
            u,
        )
        .client()
    }

    /// The answer button `id` gives the question about an item too big
    /// for its Recycle Bin.
    fn too_big_answer(&self, out: &form::Event) -> Option<reprisefs::replace::Answer> {
        use reprisefs::replace::Answer;
        let many = self.too_big.as_ref()?.many();
        let answers: &[Answer] = match many {
            true => &[Answer::Yes, Answer::YesToAll, Answer::No, Answer::Cancel],
            false => &[Answer::Yes, Answer::No],
        };
        match out {
            form::Event::Default => Some(Answer::Yes),
            form::Event::Pressed(id) => dlgtemplate::BUTTON_IDS
                .iter()
                .position(|b| b == id)
                .and_then(|i| answers.get(i).copied()),
            _ => None,
        }
    }
}

/// The Open With chooser that is up: the control, its file and type, the host's icons and its input state.
pub struct ChooseProgram {
    pub path: PathBuf,
    pub mime: Option<String>,
    pub ow: OpenWith,
    /// The Properties sheet whose Change... raised it: OK writes the default
    /// and puts the program on that sheet instead of running it.
    pub sheet: Option<SheetId>,
    pub icon: Option<Canvas>,
    pub row_icons: Vec<Option<Canvas>>,
    /// The icons `icon` and `row_icons` wear stand-ins for ([`ChooseProgram::wear_icons`]).
    pub waits: repriseapp::iconjobs::IconWaits,
    pub input: repriseapp::openwith::ChooserInput,
    pub browse: Option<ProgramBrowse>,
}

impl ChooseProgram {
    /// The chooser's icon and its rows' icons at text size `u`, stand-ins while they decode off
    /// the loop.
    pub fn wear_icons(
        &mut self,
        icons: &mut reprisekit::icontheme::IconCache,
        u: reprisekit::theme::TextUnits,
    ) {
        self.waits = Default::default();
        let px = reprisekit::controls::grouplist::icon_px(u);
        self.row_icons = crate::openwith::chooser_row_icons(&self.ow, icons, px, &mut self.waits);
        self.icon = self.waits.first(icons, &[openwith::ICON], 32);
    }
}

/// The Open With chooser's Browse... picker: a file dialog whose Open
/// adds the picked program to the chooser's list.
pub struct ProgramBrowse {
    pub host: Host,
    pub dlg: FileDlg,
}

pub struct ReplaceAsk {
    pub fields: replacebox::Fields,
    /// The folder's icon and the two files' (of a file's prompt), worn into `fields` as they land.
    pub icons: (IconSlot, Option<[IconSlot; 2]>),
    pub asker: reprisefs::replace::Asker,
    pub pending: crate::fileops::PendingPut,
    pub focus: Option<&'static str>,
}

/// One opening of a sheet window: a sheet closed and opened again, or replaced by another
/// file's, is another sheet, so an answer landing late can tell the sheet that asked is gone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SheetId {
    pub win: WinId,
    /// The sheet's number among the sheets opened ([`Explorer::next_sheet`]).
    pub opened: u64,
}

pub enum Dialog {
    Message {
        title: String,
        text: String,
        /// The Properties sheet that raised it ([`Dialog::sheet`]); `None` for the folder window.
        sheet: Option<SheetId>,
        /// Up when another file's sheet was asked for, so read: it leaves with the sheet the new
        /// one replaces, where an error not read yet stays ([`Explorer::note_sheet_error_read`]).
        read: bool,
    },
    Confirm(Confirm),
    /// Confirm File or Folder Replace; its form is rebuilt per event.
    Replace(Box<ReplaceAsk>),
    // Boxed: the picker it can hold carries a whole file dialog, which would bloat every variant.
    ChooseProgram(Box<ChooseProgram>),
    AttrConfirm(AttrConfirm),
    /// Advanced Attributes, behind the sheet's Advanced...: OK hands its
    /// archive and index boxes to the sheet when they are `live`, and
    /// otherwise closes like Cancel.
    Advanced {
        form: Form,
        subject: reprisekit::controls::filesheet::Subject,
        live: bool,
        icon: IconSlot,
    },
    /// Change Icon, behind Customize's Change Icon...: OK hands the
    /// picked icon to the sheet; `name` is the folder's, for the title.
    ChangeIcon {
        form: Form,
        name: String,
        /// The list's icons still decoding ([`crate::props_host::land_change_icons`]).
        waits: repriseapp::iconjobs::IconWaits,
    },
    About(Form, IconSlot),
    Browse(crate::browse::Browse),
    /// Unlock Drive, over a locked drive opened from Computer.
    Unlock(Box<Unlock>),
    /// Rename with several items picked; the preview follows each change.
    BatchRename(Box<crate::rename::BatchRename>),
    /// Connect to Server, typed into or showing a connection under way.
    Connect(Box<crate::shares::ConnectBox>),
    /// A server's password request.
    Login(Box<crate::shares::LoginBox>),
    /// A server's question, one button per choice.
    Question {
        title: String,
        text: String,
        choices: Vec<String>,
        about: crate::shares::Asking,
        focus: Option<&'static str>,
    },
    /// A paste whose next clash waits its turn; once it is up, [`Explorer::ask_pastes`] asks it.
    Paste(Box<(crate::fileops::PendingPut, reprisefs::replace::Asker)>),
    /// A replace prompt whose two files are still being read off the loop. Nothing shows while
    /// it is up, and the boxes behind it wait ([`Explorer::take_asking`]).
    Reading(Box<crate::fileops::ReadingSides>),
}

impl Dialog {
    /// A job, a paste or a server connection holds for the answer, so it comes up before the
    /// other boxes waiting.
    fn held(&self) -> bool {
        match self {
            Dialog::Replace(_) | Dialog::Login(_) | Dialog::Paste(_) | Dialog::Reading(_) => true,
            Dialog::Confirm(c) => c.job.is_some(),
            Dialog::Question { about, .. } => matches!(about, crate::shares::Asking::Connection),
            _ => false,
        }
    }

    /// Whether it is a box with a window, not a paste's work.
    pub(crate) fn shows(&self) -> bool {
        !matches!(self, Dialog::Paste(_) | Dialog::Reading(_))
    }

    /// The sheet that raised it, a file's Properties or Recycle Bin Properties: it covers that
    /// sheet and leaves with it. `None` for a box of the folder window's or a job's.
    pub(crate) fn sheet(&self) -> Option<WinId> {
        match self {
            Dialog::AttrConfirm(_) | Dialog::Advanced { .. } | Dialog::ChangeIcon { .. } => {
                Some(crate::props_dialog::PROPS)
            }
            Dialog::ChooseProgram(cp) => cp.sheet.map(|s| s.win),
            Dialog::Message { sheet, .. } => sheet.map(|s| s.win),
            _ => None,
        }
    }

    /// The button a rebuilt box's keys moved the focus to ([`Explorer::box_form`]); `None` while
    /// it is on the default.
    fn focus_slot(&mut self) -> Option<&mut Option<&'static str>> {
        match self {
            Dialog::Confirm(Confirm { focus, .. }) | Dialog::Question { focus, .. } => Some(focus),
            Dialog::Replace(r) => Some(&mut r.focus),
            _ => None,
        }
    }

    fn focus(&self) -> Option<&'static str> {
        match self {
            Dialog::Confirm(Confirm { focus, .. }) | Dialog::Question { focus, .. } => *focus,
            Dialog::Replace(r) => r.focus,
            _ => None,
        }
    }

    fn title(&self) -> String {
        match self {
            Dialog::Message { title, .. } => title.clone(),
            Dialog::Confirm(c) => c.title.to_string(),
            Dialog::Replace(r) => r.fields.title().to_string(),
            Dialog::ChooseProgram(_) => "Open With".to_string(),
            Dialog::AttrConfirm(_) => "Confirm Attribute Changes".to_string(),
            Dialog::Advanced { .. } => "Advanced Attributes".to_string(),
            // A launcher's box has no Restore Defaults.
            Dialog::ChangeIcon { form, .. }
                if form
                    .item(reprisekit::controls::filesheet::CHANGE_ICON_RESTORE)
                    .is_none() =>
            {
                "Change Icon".to_string()
            }
            Dialog::ChangeIcon { name, .. } => format!("Change Icon for {name} Folder"),
            Dialog::About(..) => "About Explorer".to_string(),
            Dialog::Browse(b) if b.mv => "Move Items".to_string(),
            Dialog::Browse(_) => "Copy Items".to_string(),
            Dialog::Unlock(_) => unlock::TITLE.to_string(),
            Dialog::BatchRename(_) => "Rename".to_string(),
            Dialog::Connect(_) => reprisekit::controls::netconnect::CONNECT_TITLE.to_string(),
            Dialog::Login(b) => crate::shares::login_title(&b.host),
            Dialog::Question { title, .. } => title.clone(),
            Dialog::Paste(_) | Dialog::Reading(_) => String::new(),
        }
    }

    /// Theme icons decoded off the loop landed: the dialog wears those it waited on. True when
    /// it changed.
    pub(crate) fn land_icons(&mut self, ctx: &mut Ctx) -> bool {
        if let Dialog::ChooseProgram(cp) = self {
            let landed = cp.waits.landed(&mut ctx.icons);
            if landed {
                let u = ctx.forms().controls().text;
                cp.wear_icons(&mut ctx.icons, u);
            }
            return landed;
        }
        let icons = &mut ctx.icons;
        match self {
            Dialog::Confirm(c) => c.icon.land(icons),
            Dialog::Replace(r) => r.land_icons(icons),
            Dialog::ChooseProgram(_) => false,
            Dialog::Advanced { form, icon, .. } | Dialog::About(form, icon) => {
                land_picture(form, "icon", icon, icons)
            }
            Dialog::ChangeIcon { form, waits, .. } => {
                crate::props_host::land_change_icons(form, waits, icons)
            }
            Dialog::Browse(b) => {
                crate::foldertree::land_icons(&mut b.bf.tree, &mut b.places, icons)
            }
            Dialog::Unlock(u) => u.land_icon(ctx),
            Dialog::Connect(b) => b.land_icon(ctx),
            Dialog::Login(b) => b.land_icon(ctx),
            Dialog::Message { .. }
            | Dialog::AttrConfirm(_)
            | Dialog::BatchRename(_)
            | Dialog::Question { .. }
            | Dialog::Paste(_)
            | Dialog::Reading(_) => false,
        }
    }
}

impl ReplaceAsk {
    fn land_icons(&mut self, icons: &mut reprisekit::icontheme::IconCache) -> bool {
        let (folder, sides) = &mut self.icons;
        let mut landed = folder.land(icons);
        self.fields.icon = folder.icon();
        if let (Some([a, b]), replacebox::Subject::File { existing, incoming }) =
            (sides, &mut self.fields.subject)
        {
            landed |= a.land(icons) | b.land(icons);
            (existing.icon, incoming.icon) = (a.icon(), b.icon());
        }
        landed
    }
}

pub(crate) use repriseapp::keys::dialog_key;

impl Explorer {
    pub(crate) fn follow_forms(&mut self, ctx: &Ctx) {
        self.dialog_text = ctx.text.clone();
        self.forms = ctx.forms();
        self.forms.fonts.size_px = self.forms.text.px(self.dialog_px);
    }

    /// `f` laid out for the forms theme before its first draw, so it can be hit or read.
    pub(crate) fn built(&self, f: Form) -> Form {
        f.drawn_with(&self.forms.paint());
        f
    }

    /// The one way a box goes up ([`repriseapp::asks::Queue`]): behind the box up, which it never replaces, and
    /// behind the boxes waiting, or ahead of the plain ones when its owner holds for it. A box an
    /// answer raises takes the answered box's place ([`Explorer::take_up`]). Connect to Server
    /// showing a connection under way is the one box a held box replaces: it waits behind every
    /// box raised so far, so a job's question is never stuck behind it.
    pub(crate) fn raise(&mut self, d: Dialog) -> Reply {
        let mut up = self.dialogs.is_empty() || self.answered;
        if std::mem::take(&mut self.answered) {
            self.dialogs.restore(d);
            // `restore` gives back the window of the box taken; this one is another box.
            if self.answered_in == Some(self.dialog_win()) {
                self.flip = !self.flip;
            }
        } else if d.held() && self.connecting_up() {
            let connect = self.dialogs.close();
            self.dialogs.restore(d);
            // `restore` gives back Connect to Server's window; the held box maps one of its own.
            self.flip = !self.flip;
            if let Some(connect) = connect {
                self.dialogs.raise(connect);
            }
            up = true;
        } else if d.held() {
            self.dialogs.raise_held(d);
        } else {
            self.dialogs.raise(d);
        }
        if !up {
            return Reply::none();
        }
        // Mapping without an activation, a box no window of ours covers maps behind the focused
        // window: the raise focuses it while this program holds the keyboard, else flags its task button.
        let alone = self.dialog().is_some() && self.box_stands_alone();
        Reply {
            raise: alone.then(|| self.dialog_win()),
            ..Reply::redraw_all()
        }
    }

    /// Connect to Server going busy, taken by its handler ([`Explorer::take_up`]): with a held
    /// box next, that box comes up in its place and Connect to Server waits behind every box, as
    /// when a held box is raised over it. Else it comes back for the handler to put up.
    pub(crate) fn connect_behind_held(&mut self, connect: Dialog) -> Option<Dialog> {
        if !self.dialogs.up().is_some_and(Dialog::held) {
            return Some(connect);
        }
        self.dialogs.raise(connect);
        None
    }

    /// Whether the box up stands over no window of this program: the folder window never shows
    /// ([`Explorer::alone`]) or has closed, and no sheet holds the box.
    fn box_stands_alone(&self) -> bool {
        (self.alone || self.closing) && self.parent_of(self.dialog_win()) == repriseapp::MAIN
    }

    /// Puts back the box [`Explorer::take_up`] took to handle an event that leaves it up, in the
    /// window it had.
    pub(crate) fn put_back(&mut self, d: Dialog) {
        if std::mem::take(&mut self.answered) {
            self.dialogs.restore(d);
        } else {
            self.raise(d);
        }
    }

    /// The window the box up shows in. The two windows take turns, so the compositor maps a box
    /// that comes up after another afresh instead of retitling the last one's window.
    pub(crate) fn dialog_win(&self) -> WinId {
        [DIALOG, NEXT_DIALOG][self.dialogs.turn() ^ usize::from(self.flip)]
    }

    /// The box up, unless the front of the queue is a paste's work, which shows nothing.
    pub(crate) fn dialog(&self) -> Option<&Dialog> {
        self.dialogs.up().filter(|d| d.shows())
    }

    /// Takes the box up to answer it. Until the handler that took it returns
    /// ([`Explorer::answer_ends`]), the next box raised goes up in its place, ahead of those
    /// waiting, in the other window: the answer's own box. A box that stays up goes back in its
    /// own window ([`Explorer::put_back`]).
    pub(crate) fn take_up(&mut self) -> Option<Dialog> {
        self.take_up_if(|_| true)
    }

    /// The handler that took a box returned: a box raised from here on waits its turn. A box's
    /// own event ends at [`Explorer::raise_waiting`]; a wake runs several handlers, and each one
    /// that can take a box ends its answer itself.
    pub(crate) fn answer_ends(&mut self) {
        self.answered = false;
        self.answered_in = None;
    }

    /// The window `win` covers ([`repriseapp::Extended::parent`]).
    pub(crate) fn parent_of(&self, win: WinId) -> WinId {
        match win {
            crate::browse::ERROR | PROGRAM_BROWSE => self.dialog_win(),
            w if is_dialog(w) => match self.dialog().and_then(Dialog::sheet) {
                Some(sheet) if self.sheet_open(sheet) => sheet,
                _ => repriseapp::MAIN,
            },
            _ => repriseapp::MAIN,
        }
    }

    /// Whether the sheet window `sheet` is up.
    fn sheet_open(&self, sheet: WinId) -> bool {
        match sheet {
            crate::props_dialog::PROPS => self.props.is_some(),
            crate::bin_props::BIN_PROPS => self.bin_props.is_some(),
            crate::folder_opts::FOLDER_OPTS => self.folder_opts.is_some(),
            _ => false,
        }
    }

    /// Whether the sheet `id` names is still the one up in its window.
    pub(crate) fn sheet_up(&self, id: SheetId) -> bool {
        let up = match id.win {
            crate::props_dialog::PROPS => self.props.as_ref().map(|p| p.opened),
            crate::bin_props::BIN_PROPS => self.bin_props.as_ref().map(|b| b.opened),
            crate::folder_opts::FOLDER_OPTS => self.folder_opts.as_ref().map(|f| f.opened),
            _ => None,
        };
        up == Some(id.opened)
    }

    /// The number the next sheet to open takes ([`SheetId::opened`]).
    pub(crate) fn next_sheet(&mut self) -> u64 {
        self.sheets_opened += 1;
        self.sheets_opened
    }

    /// The boxes waiting behind the one up.
    #[cfg(test)]
    pub(crate) fn boxes_waiting(&self) -> usize {
        self.dialogs.len().saturating_sub(1)
    }

    /// The pastes waiting their turn to ask a clash.
    #[cfg(test)]
    pub(crate) fn pastes_waiting(&self) -> usize {
        self.dialogs
            .iter()
            .filter(|d| matches!(d, Dialog::Paste(_)))
            .count()
    }

    /// Whether a replace prompt's files are being read.
    #[cfg(test)]
    pub(crate) fn reading(&self) -> bool {
        self.dialogs.iter().any(|d| matches!(d, Dialog::Reading(_)))
    }

    /// [`Explorer::take_up`] when the box up is one `is` picks.
    pub(crate) fn take_up_if(&mut self, is: impl FnOnce(&Dialog) -> bool) -> Option<Dialog> {
        let win = self.dialog().is_some().then(|| self.dialog_win());
        let d = self.dialogs.close_if(is)?;
        self.answered = true;
        self.answered_in = win.or(self.answered_in);
        Some(d)
    }

    /// Raise a box titled `title` over `text` with the theme's error icon.
    pub fn message_box(&mut self, title: &str, text: String, ctx: &mut Ctx) -> Reply {
        self.message_over(None, title, text, ctx)
    }

    /// [`Explorer::message_box`] raised by the sheet `sheet`: it covers the sheet while it is
    /// up, and the folder window once it has closed or another sheet took its window.
    pub(crate) fn message_over(
        &mut self,
        sheet: Option<SheetId>,
        title: &str,
        text: String,
        ctx: &mut Ctx,
    ) -> Reply {
        self.wear_error_icon(ctx);
        self.raise(Dialog::Message {
            title: title.to_string(),
            text,
            sheet: sheet.filter(|&s| self.sheet_up(s)),
            read: false,
        })
    }

    /// The message boxes' icon, fetched for the first box.
    pub(crate) fn wear_error_icon(&mut self, ctx: &mut Ctx) {
        if self.error_icon.is_none() {
            self.error_icon = Some(IconSlot::chrome(&mut ctx.icons, ERROR_ICON, 32));
        }
    }

    pub(crate) fn error_icon(&self) -> Option<Canvas> {
        self.error_icon.as_ref().and_then(IconSlot::icon)
    }

    pub fn not_found(&mut self, typed: &str, ctx: &mut Ctx) -> Reply {
        let text = place::not_found_text(typed);
        self.message_box("Explorer", text, ctx)
    }

    pub fn advanced_attributes(
        &mut self,
        subject: reprisekit::controls::filesheet::Subject,
        robots: Option<[reprisekit::classic::widgets::CheckState; 2]>,
        ctx: &mut Ctx,
    ) -> Reply {
        let icon = IconSlot::chrome(&mut ctx.icons, &["document-properties"], 32);
        let form = reprisekit::controls::filesheet::advanced_form(
            subject,
            icon.icon(),
            robots,
            &self.dialog_text,
            self.forms.fonts.size_px,
            self.forms.text,
        );
        let live = robots.is_some();
        self.raise(Dialog::Advanced {
            form,
            subject,
            live,
            icon,
        })
    }

    pub fn about_box(&mut self, ctx: &mut Ctx) -> Reply {
        let icon = IconSlot::chrome(&mut ctx.icons, &["system-file-manager"], 32);
        let fields = repriseapp::about::fields("Explorer", icon.icon());
        let px = ctx.theme.fonts.size_px;
        let mut form = aboutbox::form(&fields, &ctx.text, px);
        let u = self.forms.text;
        let w = u.len(aboutbox::CLIENT.0);
        form.set_picture("banner", aboutbox::banner_wide(&ctx.text, px, w, u));
        self.raise(Dialog::About(form, icon))
    }

    /// Raise a delete prompt over `paths` ([`Explorer::delete_question`]).
    #[allow(clippy::too_many_arguments)]
    pub fn confirm_box(
        &mut self,
        kind: ConfirmKind,
        name: &str,
        paths: Vec<PathBuf>,
        erase: bool,
        no_bin: reprisefs::bincap::BinLack,
        icon: IconSlot,
        empty_bin: Option<usize>,
        bin: Option<Vec<crate::fileops::BinItem>>,
    ) -> Reply {
        let c = self.delete_question(kind, name, paths, erase, no_bin, icon, empty_bin, bin);
        self.raise(Dialog::Confirm(c))
    }

    /// A delete prompt over `paths` on the template its kind takes (see [`dlgtemplate`]). `no_bin` leads with the drive having no Recycle Bin for some or all of `paths` ([`reprisefs::bincap::delete_prompt`]). `empty_bin` is the Recycle Bin's Empty task over that many items, since it has no paths; `bin` is the picked items of a prompt raised in the Recycle Bin, which Yes deletes for good.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn delete_question(
        &self,
        kind: ConfirmKind,
        name: &str,
        paths: Vec<PathBuf>,
        erase: bool,
        no_bin: reprisefs::bincap::BinLack,
        icon: IconSlot,
        empty_bin: Option<usize>,
        bin: Option<Vec<crate::fileops::BinItem>>,
    ) -> Confirm {
        let count = empty_bin.unwrap_or(paths.len());
        let (title, text) = reprisefs::bincap::delete_prompt(kind, name, count, erase, no_bin);
        let min_text_h = (kind == ConfirmKind::Folder).then_some(FOLDER_TEXT_H);
        Confirm {
            title,
            text,
            min_text_h,
            template: match kind {
                ConfirmKind::Multiple => Template::MultiDelete,
                ConfirmKind::File | ConfirmKind::Folder => Template::MessageBox,
            },
            icon,
            paths,
            erase,
            empty_bin: empty_bin.is_some(),
            bin,
            focus: None,
            too_big: None,
            undo_erase: None,
            script: None,
            mount: false,
            job: None,
        }
    }

    /// Whether the box up is [`Explorer::ask_failed`]'s, which a job waits on.
    #[cfg(test)]
    pub(crate) fn holds_a_job(&self) -> bool {
        matches!(self.dialogs.up(), Some(Dialog::Confirm(c)) if c.job.is_some())
    }

    /// Ask what the job held on a failed item does next: Skip goes on with the rest, Retry runs
    /// the item again, Cancel ends the job.
    pub(crate) fn ask_failed(
        &mut self,
        asked: reprisefs::progress::Asked,
        title: &'static str,
        text: String,
        ctx: &mut Ctx,
    ) -> Reply {
        if self.closing {
            return Reply::none();
        }
        self.raise(Dialog::Confirm(Confirm {
            title,
            text,
            min_text_h: None,
            template: Template::MessageBox,
            icon: IconSlot::chrome(&mut ctx.icons, ERROR_ICON, 32),
            paths: Vec::new(),
            erase: false,
            empty_bin: false,
            bin: None,
            focus: None,
            too_big: None,
            undo_erase: None,
            script: None,
            mount: false,
            job: Some(asked),
        }))
    }

    /// The answer to [`Explorer::ask_failed`], by the button `out` pressed.
    fn failed_answered(&mut self, out: form::Event) -> Reply {
        use reprisefs::progress::Decision;
        let decision = match out {
            form::Event::Default => Decision::Skip,
            form::Event::Pressed(id) => match dlgtemplate::BUTTON_IDS.iter().position(|&b| b == id)
            {
                Some(0) => Decision::Skip,
                Some(1) => Decision::Retry,
                Some(_) => Decision::Abort,
                None => return Reply::redraw(),
            },
            form::Event::None => return Reply::none(),
            _ => return Reply::redraw(),
        };
        if let Some(Dialog::Confirm(Confirm { job: Some(m), .. })) = self.take_up() {
            m.answer(decision);
        }
        if decision != Decision::Abort {
            self.show_progress_dialogs();
        }
        Reply::redraw_all()
    }

    /// Ask whether to run the file at `path`, a script as `open` says, or (a script) open it in
    /// Notepad.
    pub(crate) fn ask_run(
        &mut self,
        path: &std::path::Path,
        open: reprisefs::program::ScriptOpen,
        asked: reprisefs::program::Asked,
        ctx: &mut Ctx,
    ) -> Reply {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        self.raise(Dialog::Confirm(Confirm {
            title: reprisefs::program::ASK_TITLE,
            text: asked.text(&name),
            min_text_h: None,
            template: Template::MessageBox,
            icon: IconSlot::chrome(&mut ctx.icons, QUESTION_ICON, 32),
            paths: vec![path.to_path_buf()],
            erase: false,
            empty_bin: false,
            bin: None,
            focus: None,
            too_big: None,
            undo_erase: None,
            script: Some((open, asked)),
            mount: false,
            job: None,
        }))
    }

    /// The answer to [`Explorer::ask_run`]: the button `out` pressed, by its place in
    /// [`reprisefs::program::Asked::buttons`].
    fn script_answered(&mut self, out: form::Event, ctx: &mut Ctx) -> Reply {
        let default = match self.dialogs.up() {
            Some(Dialog::Confirm(c)) => c.default_button(),
            _ => 0,
        };
        let pick = match out {
            form::Event::Default => default,
            form::Event::Pressed(id) => match dlgtemplate::BUTTON_IDS.iter().position(|&b| b == id)
            {
                Some(i) => i,
                None => return Reply::redraw(),
            },
            form::Event::None => return Reply::none(),
            _ => return Reply::redraw(),
        };
        let Some(Dialog::Confirm(Confirm {
            script: Some((open, asked)),
            mut paths,
            ..
        })) = self.take_up()
        else {
            return Reply::none();
        };
        let Some(path) = paths.pop() else {
            return Reply::redraw_all();
        };
        let notepad = asked == reprisefs::program::Asked::Script;
        let reply = match pick {
            0 => self.open_file(&path, true, open, reprisefs::program::Confirm::Allow, ctx),
            1 if notepad => Reply {
                spawn: vec![reprisefs::program::notepad_argv(&path)],
                spawn_app: Some(reprisefs::apps::AppId::from_static("reprise-notepad")),
                ..Reply::default()
            },
            _ => Reply::none(),
        };
        Reply {
            redraw_all: true,
            ..reply
        }
    }

    fn box_fields(&self) -> Option<msgbox::Fields> {
        match self.dialogs.up()? {
            Dialog::Message { title, text, .. } => Some(msgbox::Fields {
                text: text.clone(),
                icon: self.error_icon(),
                buttons: vec!["OK".to_string()],
                default: 0,
                min_text_h: None,
                min_w: repriseapp::caption_fit(title),
                max_w: repriseapp::dialog_max_w(),
            }),
            Dialog::Question {
                title,
                text,
                choices,
                about,
                ..
            } => Some(msgbox::Fields {
                text: text.clone(),
                icon: self.error_icon(),
                buttons: choices.clone(),
                default: about.default_choice(choices.len()),
                min_text_h: None,
                min_w: repriseapp::caption_fit(title),
                max_w: repriseapp::dialog_max_w(),
            }),
            Dialog::Confirm(_)
            | Dialog::ChooseProgram(_)
            | Dialog::Replace(_)
            | Dialog::AttrConfirm(_)
            | Dialog::Advanced { .. }
            | Dialog::ChangeIcon { .. }
            | Dialog::About(..)
            | Dialog::Browse(_)
            | Dialog::Unlock(_)
            | Dialog::BatchRename(_)
            | Dialog::Connect(_)
            | Dialog::Login(_)
            | Dialog::Paste(_)
            | Dialog::Reading(_) => None,
        }
    }

    fn confirm_layout(&self) -> Option<dlgtemplate::Layout> {
        let Some(Dialog::Confirm(c)) = self.dialogs.up() else {
            return None;
        };
        Some(c.layout(&self.dialog_text, self.forms.fonts.size_px, self.forms.text))
    }

    /// The form of whichever message, question, confirm or replace box is up, focus on the button
    /// its keys left it on.
    pub fn box_form(&self) -> Option<Form> {
        let d = self.dialogs.up()?;
        let (text, px, u) = (&self.dialog_text, self.forms.fonts.size_px, self.forms.text);
        let mut f = match d {
            Dialog::Confirm(c) => {
                let lay = self.confirm_layout()?;
                dlgtemplate::form(&lay, c.icon.icon(), &c.buttons(), c.default_button())
            }
            Dialog::Replace(r) => replacebox::form(&r.fields, text, px, u),
            _ => msgbox::form(&self.box_fields()?, text, px, u),
        };
        if let Some(id) = d.focus() {
            f.focus_id(id);
        }
        Some(self.built(f))
    }

    pub fn min_size(&self, win: WinId) -> Option<(i32, i32)> {
        match (win, self.dialogs.up()) {
            (w, Some(Dialog::Browse(_))) if is_dialog(w) => {
                Some(browsefolder::min(&self.forms.controls()))
            }
            (PROGRAM_BROWSE, Some(Dialog::ChooseProgram(cp))) => {
                cp.browse.as_ref().map(|b| b.dlg.min_client())
            }
            _ => None,
        }
    }

    /// The caption `win` wears: the sheets and pickers add the ? box; the message boxes, confirmations and About have the close box alone.
    /// The program Browse is a file dialog and keeps the plain caption, which tags it `file-dialog`.
    pub fn caption(&self, win: WinId) -> Caption {
        match win {
            w if is_dialog(w) => match self.dialogs.up() {
                Some(
                    Dialog::ChooseProgram(_)
                    | Dialog::Browse(_)
                    | Dialog::Advanced { .. }
                    | Dialog::ChangeIcon { .. },
                ) => Caption::Help,
                // A prompt has no task button, and nothing else of this program would show one.
                _ if self.box_stands_alone() => Caption::Fixed,
                _ => Caption::Prompt,
            },
            crate::props_dialog::PROPS
            | crate::bin_props::BIN_PROPS
            | crate::folder_opts::FOLDER_OPTS => Caption::Help,
            crate::browse::ERROR => Caption::Prompt,
            crate::copydlg::COPY | crate::copydlg::REMOVE | crate::copydlg::EXTRACT => {
                Caption::Prompt
            }
            _ => Caption::Window,
        }
    }

    pub fn children(&self) -> Vec<Child> {
        let mut out = Vec::new();
        let (text, px, u) = (&self.dialog_text, self.forms.fonts.size_px, self.forms.text);
        if let Some(d) = self.dialog() {
            let size = match self.box_fields() {
                Some(f) => u.size(msgbox::client(&f, text, px, u)),
                None => match d {
                    Dialog::Replace(r) => u.size(replacebox::client(&r.fields, text, px, u)),
                    Dialog::AttrConfirm(_) => u.size(ATTR_SIZE),
                    Dialog::Advanced { subject, .. } => {
                        u.size(reprisekit::controls::filesheet::advanced_size(*subject))
                    }
                    Dialog::ChangeIcon { .. } => {
                        u.size(reprisekit::controls::filesheet::CHANGE_ICON_SIZE)
                    }
                    Dialog::About(..) => u.size(aboutbox::CLIENT),
                    Dialog::Browse(_) => browsefolder::client(&self.forms.controls()),
                    Dialog::ChooseProgram(_) => openwith::client(&self.forms.controls()),
                    Dialog::Unlock(_) => u.size(unlock::CLIENT),
                    Dialog::BatchRename(_) => u.size(reprisekit::controls::batchrename::CLIENT),
                    Dialog::Connect(_) => u.size(reprisekit::controls::netconnect::CONNECT_CLIENT),
                    Dialog::Login(b) => u.size(reprisekit::controls::netconnect::login_client(
                        b.form
                            .item(reprisekit::controls::netconnect::DOMAIN)
                            .is_some(),
                    )),
                    Dialog::Confirm(_) => {
                        u.size(self.confirm_layout().expect("a delete prompt is up").size)
                    }
                    Dialog::Message { .. } | Dialog::Question { .. } => {
                        unreachable!("box_fields answers it")
                    }
                    Dialog::Paste(_) | Dialog::Reading(_) => unreachable!("it shows nothing"),
                },
            };
            out.push(Child {
                id: self.dialog_win(),
                title: d.title(),
                size,
                fixed: !matches!(d, Dialog::Browse(_)),
                modal: true,
            });
        }
        // A picker write that failed: the error box over the picker.
        if let Some(f) = self.browse_error_fields()
            && let Some(Dialog::Browse(b)) = self.dialogs.up()
            && let Some(r) = &b.refused
        {
            out.push(Child {
                id: crate::browse::ERROR,
                title: r.title.into(),
                size: u.size(msgbox::client(&f, text, px, u)),
                fixed: true,
                modal: true,
            });
        }
        if let Some(Dialog::ChooseProgram(cp)) = self.dialogs.up()
            && let Some(b) = &cp.browse
        {
            out.push(Child {
                id: PROGRAM_BROWSE,
                title: "Browse".into(),
                size: b.dlg.min_client(),
                fixed: false,
                modal: true,
            });
        }
        out.extend(self.props_child());
        out.extend(self.bin_props_child());
        out.extend(self.folder_opts_child());
        out.extend(self.preview_child());
        out.extend(self.progress_children());
        out
    }

    pub fn draw_dialog(&self, c: &mut Canvas, ctx: &Ctx) {
        c.fill_rect(c.bounds(), ctx.theme.colors.button_face);
        if let Some(Dialog::Unlock(u)) = self.dialogs.up() {
            u.draw(c, ctx);
            return;
        }
        if let Some(Dialog::Browse(b)) = self.dialogs.up() {
            let lay = b.bf.layout(
                &self.dialog_text,
                self.forms.fonts.size_px,
                &self.forms.controls(),
            );
            b.bf.draw(
                c,
                &self.dialog_text,
                &Paint {
                    s: 1,
                    ..self.forms.paint()
                },
                &lay,
                self.forms.fonts.size_px,
            );
            return;
        }
        if let Some(Dialog::ChooseProgram(cp)) = self.dialogs.up() {
            let lay = cp.ow.layout(
                &self.dialog_text,
                self.forms.fonts.size_px,
                (0, 0),
                &self.forms.controls(),
            );
            let rows: Vec<Option<&Canvas>> = cp.row_icons.iter().map(Option::as_ref).collect();
            cp.ow.draw(
                c,
                &self.dialog_text,
                &Paint {
                    s: 1,
                    ..self.forms.paint()
                },
                &lay,
                cp.icon.as_ref(),
                &rows,
                cp.input.cues(),
                self.forms.fonts.size_px,
            );
            return;
        }
        // The kept form is drawn itself, not a copy: it keeps the sizes of
        // the look it was drawn in, which its hit test then reads.
        let built;
        let form = match self.dialogs.up() {
            Some(Dialog::AttrConfirm(a)) => &a.form,
            Some(Dialog::Advanced { form, .. }) => form,
            Some(Dialog::ChangeIcon { form, .. }) => form,
            Some(Dialog::About(f, _)) => f,
            Some(Dialog::BatchRename(b)) => &b.form,
            Some(Dialog::Connect(b)) => &b.form,
            Some(Dialog::Login(b)) => &b.form,
            Some(_) => match self.box_form() {
                Some(f) => {
                    built = f;
                    &built
                }
                None => return,
            },
            None => return,
        };
        form::draw(
            c,
            &self.dialog_text,
            &Paint {
                s: 1,
                ..self.forms.paint()
            },
            form,
            true,
            self.forms.fonts.size_px,
        );
    }

    pub fn access_dialog(&self, tree: &mut reprisekit::access::Tree, ctx: &Ctx) {
        if let Some(Dialog::Unlock(u)) = self.dialogs.up() {
            u.access(tree, ctx);
            return;
        }
        if let Some(Dialog::Browse(b)) = self.dialogs.up() {
            b.bf.access(
                tree,
                &self.dialog_text,
                self.forms.fonts.size_px,
                &self.forms.controls(),
            );
            return;
        }
        if let Some(Dialog::ChooseProgram(cp)) = self.dialogs.up() {
            let lay = cp.ow.layout(
                &self.dialog_text,
                self.forms.fonts.size_px,
                (0, 0),
                &self.forms.controls(),
            );
            cp.ow.access(tree, &lay);
            return;
        }
        let built;
        let form = match self.dialogs.up() {
            Some(Dialog::AttrConfirm(a)) => &a.form,
            Some(Dialog::Advanced { form, .. }) => form,
            Some(Dialog::ChangeIcon { form, .. }) => form,
            Some(Dialog::About(f, _)) => f,
            Some(Dialog::BatchRename(b)) => &b.form,
            Some(Dialog::Connect(b)) => &b.form,
            Some(Dialog::Login(b)) => &b.form,
            Some(_) => match self.box_form() {
                Some(f) => {
                    built = f;
                    &built
                }
                None => return,
            },
            None => return,
        };
        let px = self.forms.fonts.size_px;
        repriseapp::oneshot::access_form(form, tree, &self.dialog_text, px);
    }

    /// The dialog's own event loop. Every box closes on its cancel
    /// button, and on Escape and the frame's close box through [`Explorer::dismiss`].
    pub fn dialog_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        if matches!(ev, Event::CloseRequest) {
            return self.dismiss_up(ctx);
        }
        if matches!(
            self.dialogs.up(),
            Some(Dialog::Connect(_) | Dialog::Login(_))
        ) {
            return self.connect_event(ev, ctx);
        }
        if self.dialog().is_none() {
            return Reply::none();
        }
        if matches!(self.dialogs.up(), Some(Dialog::Browse(_))) {
            return self.browse_event(ev, ctx);
        }
        if matches!(self.dialogs.up(), Some(Dialog::ChooseProgram(_))) {
            return self.chooser_event(ev, ctx);
        }
        if matches!(self.dialogs.up(), Some(Dialog::Unlock(_))) {
            return self.unlock_event(ev, ctx);
        }
        // The boxes that keep a form (attribute confirm, Advanced Attributes, Change Icon) take their events in place; the text boxes rebuild theirs per event.
        let kept = match self.dialogs.up_mut() {
            Some(Dialog::AttrConfirm(a)) => Some(&mut a.form),
            Some(Dialog::Advanced { form, .. }) => Some(form),
            Some(Dialog::ChangeIcon { form, .. }) => Some(form),
            Some(Dialog::About(f, _)) => Some(f),
            Some(Dialog::BatchRename(b)) => Some(&mut b.form),
            _ => None,
        };
        let out = if let Some(form) = kept {
            match ev {
                Event::Pointer(PointerInput::Press {
                    x,
                    y,
                    button: BTN_LEFT,
                    time,
                    ..
                }) => {
                    let hit = form.hit(x, y, &self.dialog_text, self.forms.fonts.size_px);
                    let at = ctx.theme.metrics.click(x, y, time);
                    let double = self.dialog_clicks.press(hit.map(|h| h.item), at);
                    if double {
                        form.double_click(hit)
                    } else {
                        form.click(hit)
                    }
                }
                ev => match repriseapp::keys::form_input(form, &ev, ctx) {
                    Some(out) => out,
                    None => return Reply::none(),
                },
            }
        } else {
            let Some(mut f) = self.box_form() else {
                return Reply::none();
            };
            match ev {
                Event::Key(k) => match dialog_key(&k) {
                    Some(key) => {
                        let out = f.key(key);
                        self.keep_box_focus(&f);
                        out
                    }
                    None => return Reply::none(),
                },
                Event::Pointer(PointerInput::Press {
                    x,
                    y,
                    button: BTN_LEFT,
                    ..
                }) => {
                    let hit = f.hit(x, y, &self.dialog_text, self.forms.fonts.size_px);
                    f.click(hit);
                    f.release()
                }
                _ => return Reply::none(),
            }
        };
        self.dialog_outcome(out, ctx)
    }

    /// Keep the focus of the button `f` is on in the box `f` was built for; the next form puts it
    /// back so Enter and Space press it.
    fn keep_box_focus(&mut self, f: &Form) {
        if let Some(slot) = self.dialogs.up_mut().and_then(Dialog::focus_slot) {
            *slot = f.focus().map(|i| f.items[i].id);
        }
    }

    pub(crate) fn dialog_outcome(&mut self, out: form::Event, ctx: &mut Ctx) -> Reply {
        enum Answer {
            Stay,
            Close,
            Accept,
        }
        if let (Some(Dialog::ChangeIcon { form, .. }), form::Event::Pressed(id)) =
            (self.dialogs.up_mut(), &out)
            && *id == reprisekit::controls::filesheet::CHANGE_ICON_RESTORE
        {
            reprisekit::controls::filesheet::change_icon_restore(form);
            return Reply::redraw();
        }
        if out == form::Event::Cancel {
            return self.dismiss_up(ctx);
        }
        let Some(d) = self.dialogs.up() else {
            return Reply::none();
        };
        if let Dialog::BatchRename(_) = d {
            return self.batch_rename_answered(out, ctx);
        }
        if let Dialog::Confirm(c) = d
            && c.script.is_some()
        {
            return self.script_answered(out, ctx);
        }
        if let Dialog::Confirm(c) = d
            && c.job.is_some()
        {
            return self.failed_answered(out);
        }
        if let Dialog::Confirm(c) = d
            && c.too_big.is_some()
        {
            return match c.too_big_answer(&out) {
                Some(a) => {
                    let Some(Dialog::Confirm(c)) = self.take_up() else {
                        return Reply::none();
                    };
                    let mut asker = c.too_big.expect("a question about an item too big");
                    asker.answer(a);
                    let reply = self.too_big_step(asker, ctx);
                    Reply {
                        redraw_all: true,
                        ..reply
                    }
                }
                None if out == form::Event::None => Reply::none(),
                None => Reply::redraw(),
            };
        }
        if let Dialog::Question { choices, about, .. } = d {
            let n = choices.len();
            let default = about.default_choice(n);
            return match out {
                form::Event::Default => self.question_answered(Some(default), ctx),
                form::Event::Pressed(id) => {
                    let i = (0..n).find(|&i| msgbox::button_id(i) == id);
                    self.question_answered(i, ctx)
                }
                form::Event::None => Reply::none(),
                _ => Reply::redraw(),
            };
        }
        let answer = match d {
            Dialog::Message { .. } => match out {
                form::Event::Default => Answer::Close,
                form::Event::Pressed(id) if id == msgbox::OK => Answer::Close,
                _ => Answer::Stay,
            },
            Dialog::Confirm(c) => match out {
                form::Event::Pressed(id) if id == msgbox::button_id(1) => Answer::Close,
                form::Event::Default if c.mount => Answer::Close,
                form::Event::Default => Answer::Accept,
                form::Event::Pressed(id) if id == msgbox::button_id(0) => Answer::Accept,
                _ => Answer::Stay,
            },
            Dialog::About(..) => match out {
                form::Event::Default => Answer::Close,
                form::Event::Pressed(id) if id == aboutbox::OK => Answer::Close,
                _ => Answer::Stay,
            },
            // OK hands the pick to the sheet, Restore Defaults picks the
            // standard folder again.
            Dialog::ChangeIcon { .. } => {
                use reprisekit::controls::filesheet::{CHANGE_ICON_CANCEL, CHANGE_ICON_OK};
                match out {
                    form::Event::Default | form::Event::Pressed(CHANGE_ICON_OK) => Answer::Accept,
                    form::Event::Pressed(CHANGE_ICON_CANCEL) => Answer::Close,
                    _ => Answer::Stay,
                }
            }
            Dialog::Advanced { live, .. } => match out {
                form::Event::Default | form::Event::Pressed(ADV_OK) if *live => Answer::Accept,
                form::Event::Default => Answer::Close,
                form::Event::Pressed(ADV_OK | ADV_CANCEL) => Answer::Close,
                _ => Answer::Stay,
            },
            Dialog::AttrConfirm(_) => match out {
                form::Event::Pressed(ATTR_CANCEL) => Answer::Close,
                form::Event::Default | form::Event::Pressed(ATTR_OK) => Answer::Accept,
                _ => Answer::Stay,
            },
            Dialog::Replace(_) => {
                return match replacebox::choice(&out) {
                    Some(c) => self.replace_answered(c, ctx),
                    None if out == form::Event::None => Reply::none(),
                    None => Reply::redraw(),
                };
            }
            // The picker has no form: `browse_event` answers it.
            Dialog::Browse(_) => Answer::Stay,
            // The chooser and Unlock Drive answer in `chooser_event` and
            // `unlock_event`; `dialog_event` never reaches here for them.
            Dialog::ChooseProgram(_)
            | Dialog::Unlock(_)
            | Dialog::BatchRename(_)
            | Dialog::Connect(_)
            | Dialog::Login(_)
            | Dialog::Question { .. }
            | Dialog::Paste(_)
            | Dialog::Reading(_) => Answer::Stay,
        };
        match answer {
            Answer::Stay if out == form::Event::None => Reply::none(),
            Answer::Stay => Reply::redraw(),
            Answer::Close => self.dismiss_up(ctx),
            Answer::Accept => match self.take_up() {
                Some(Dialog::Confirm(c)) if c.empty_bin => self.empty_bin_now(ctx),
                Some(Dialog::Confirm(c)) if c.mount => match c.paths.first() {
                    Some(path) => self.mount_image_at(path.clone(), ctx),
                    None => Reply::redraw_all(),
                },
                Some(Dialog::Confirm(Confirm {
                    undo_erase: Some(entry),
                    ..
                })) => self.undo_with(Some(entry), ctx),
                Some(Dialog::Confirm(Confirm {
                    bin: Some(items), ..
                })) => self.erase_from_bin(items, ctx),
                Some(Dialog::Confirm(c)) => self.delete_now(&c.paths, c.erase, ctx),
                Some(Dialog::AttrConfirm(a)) => self.attrs_confirmed(a, ctx),
                Some(Dialog::Advanced { form, .. }) => self.advanced_chosen(&form),
                Some(Dialog::ChangeIcon { form, .. }) => {
                    use reprisekit::controls::filesheet::{
                        CHANGE_ICON_RESTORE, STANDARD_FOLDER_ICON, change_icon_picked,
                    };
                    // A folder's box stands the standard folder for no
                    // icon of its own.
                    let folder = form.item(CHANGE_ICON_RESTORE).is_some();
                    let picked = change_icon_picked(&form)
                        .filter(|n| !(folder && n == STANDARD_FOLDER_ICON));
                    self.icon_chosen(picked, ctx)
                }
                _ => Reply::redraw_all(),
            },
        }
    }
}

impl Explorer {
    /// [`Explorer::dismiss`] on the box up.
    pub(crate) fn dismiss_up(&mut self, ctx: &mut Ctx) -> Reply {
        match self.take_up_if(Dialog::shows) {
            Some(d) => self.dismiss(d, ctx),
            None => Reply::none(),
        }
    }

    /// The one exit of a box that leaves unanswered: Escape, its close box, its owner gone, the
    /// window closing. Whoever waits on it gets its cancel answer, once.
    pub(crate) fn dismiss(&mut self, d: Dialog, ctx: &mut Ctx) -> Reply {
        use reprisefs::progress::Decision;
        let reply = match d {
            Dialog::Confirm(c) => {
                if let Some(job) = c.job {
                    job.answer(Decision::Abort);
                }
                match c.too_big {
                    Some(mut asker) => {
                        asker.answer(reprisefs::replace::Answer::Cancel);
                        self.too_big_step(asker, ctx)
                    }
                    None => Reply::none(),
                }
            }
            Dialog::Replace(r) => self.paste_dismissed(r.pending, r.asker, ctx),
            Dialog::Paste(p) => {
                let (pending, asker) = *p;
                self.paste_dismissed(pending, asker, ctx)
            }
            Dialog::Reading(r) => {
                let (ask, ..) = *r;
                let (pending, asker) = ask.into_paste();
                self.paste_dismissed(pending, asker, ctx)
            }
            // Cancelling the attribute question leaves the sheet's box where the file says it
            // should be.
            Dialog::AttrConfirm(_) => self.attrs_cancelled(),
            Dialog::Browse(b) => {
                self.browse_left.extend(b.making);
                Reply::none()
            }
            Dialog::Connect(_)
            | Dialog::Login(_)
            | Dialog::Question {
                about: crate::shares::Asking::Connection,
                ..
            } => self.cancel_connection(),
            Dialog::Message { .. }
            | Dialog::About(..)
            | Dialog::ChooseProgram(_)
            | Dialog::Advanced { .. }
            | Dialog::ChangeIcon { .. }
            | Dialog::Unlock(_)
            | Dialog::BatchRename(_)
            | Dialog::Question {
                about: crate::shares::Asking::Disconnect(_),
                ..
            } => Reply::none(),
        };
        Reply {
            redraw_all: true,
            ..reply
        }
    }

    /// A paste whose prompt left unanswered is cancelled; on a closed window, where nothing can be
    /// asked, every clash answers No and the rest is put ([`Explorer::ask_replace`]).
    fn paste_dismissed(
        &mut self,
        pending: crate::fileops::PendingPut,
        mut asker: reprisefs::replace::Asker,
        ctx: &mut Ctx,
    ) -> Reply {
        if !self.closing {
            asker.answer(reprisefs::replace::Answer::Cancel);
        }
        self.ask_replace(pending, asker, ctx)
    }

    /// A job that ended takes down its failure boxes, up or waiting.
    pub(crate) fn job_ended(
        &mut self,
        meter: &std::sync::Arc<reprisefs::progress::Meter>,
        ctx: &mut Ctx,
    ) {
        let ours = |d: &Dialog| matches!(d, Dialog::Confirm(Confirm { job: Some(j), .. }) if j.is_for(meter));
        for d in self.dialogs.withdraw(ours) {
            self.dismiss(d, ctx);
        }
    }

    fn replace_answered(&mut self, c: replacebox::Choice, ctx: &mut Ctx) -> Reply {
        use reprisefs::replace::Answer;
        let Some(Dialog::Replace(r)) = self.take_up() else {
            return Reply::none();
        };
        let ReplaceAsk {
            mut asker, pending, ..
        } = *r;
        asker.answer(match c {
            replacebox::Choice::Yes => Answer::Yes,
            replacebox::Choice::YesToAll => Answer::YesToAll,
            replacebox::Choice::No => Answer::No,
            replacebox::Choice::Cancel => Answer::Cancel,
        });
        let reply = self.ask_replace(pending, asker, ctx);
        Reply {
            redraw_all: true,
            ..reply
        }
    }

    /// After every event: a paste whose turn came asks its clash.
    pub(crate) fn raise_waiting(&mut self, reply: Reply, ctx: &mut Ctx) -> Reply {
        let reply = reply.and(self.ask_pastes(ctx));
        self.answer_ends();
        reply
    }

    /// Asks the clashes of the pastes up in turn, each in the place of the paste
    /// ([`Explorer::ask_replace`]), until a box or a replace prompt's read is up.
    pub(crate) fn ask_pastes(&mut self, ctx: &mut Ctx) -> Reply {
        let mut reply = Reply::none();
        while matches!(self.dialogs.up(), Some(Dialog::Paste(_))) {
            let Some(Dialog::Paste(p)) = self.take_up() else {
                break;
            };
            let (pending, asker) = *p;
            reply = reply.and(self.ask_replace(pending, asker, ctx));
        }
        reply
    }
}
