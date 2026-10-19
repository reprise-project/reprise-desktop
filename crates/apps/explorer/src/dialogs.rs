//! Explorer's modal dialogs in one child window: the error box, the delete and replace confirmations, the Open With chooser, the Properties sheet's Advanced Attributes, Confirm Attribute Changes and Change Icon, the Move and Copy folder picker (`browse`), and Unlock Drive.
//!
//! One child at a time: raising a second replaces the first. Two stack instead: the chooser's Browse... opens the program picker (`PROGRAM_BROWSE`) over it, and a refused name in the Move/Copy picker raises its error box (`browse::ERROR`). The error and confirm boxes keep only their text and focused button, so their [`Form`] is rebuilt per event; the chooser keeps an `openwith` control that takes its own events ([`crate::openwith`]).

use crate::place;
use crate::props_dialog::{ADV_CANCEL, ADV_OK, ATTR_CANCEL, ATTR_OK, ATTR_SIZE, AttrConfirm};
use crate::{BTN_LEFT, Explorer};
use repriseapp::filedlg::Host;
use repriseapp::unlock::{self, Unlock};
use repriseapp::{Caption, Child, Ctx, Event, PointerInput, Reply, WinId};
use reprisekit::controls::aboutbox;
use reprisekit::controls::browsefolder;
use reprisekit::controls::dlgtemplate::{self, Template};
use reprisekit::controls::filedlg::FileDlg;
use reprisekit::controls::form::{self, Form};
use reprisekit::controls::msgbox;
use reprisekit::controls::openwith::{self, OpenWith};
use reprisekit::controls::replacebox;
use reprisekit::look::Paint;
use reprisekit::raster::Canvas;
use std::path::PathBuf;

/// The dialog's own window; each is modal and one at a time.
pub const DIALOG: WinId = "dialog";

pub const PROGRAM_BROWSE: WinId = "program browse";

/// The text height Confirm Folder Delete reserves, whatever the folder
/// is called (the 51 in `src/dialog.rs`'s `confirm_delete`).
const FOLDER_TEXT_H: i32 = 51;

pub use reprisefs::bincap::ConfirmKind;

pub struct Confirm {
    pub title: &'static str,
    pub text: String,
    pub min_text_h: Option<i32>,
    pub template: Template,
    pub icon: Option<Canvas>,
    pub paths: Vec<PathBuf>,
    pub erase: bool,
    /// The Recycle Bin's Empty task raised it: Yes empties the trash
    /// rather than deleting `paths`.
    pub empty_bin: bool,
    /// Raised in the Recycle Bin: Yes deletes these items for good, as
    /// they were listed when the prompt went up.
    pub bin: Option<Vec<crate::fileops::BinItem>>,
    /// The button Tab and the arrows moved the focus to, which each
    /// event's new form gets back; `None` (Yes) when the prompt opens.
    pub focus: Option<&'static str>,
    /// The question is about an item too big for its Recycle Bin: Yes
    /// erases it and No leaves it, and a batch of more than one item
    /// adds Yes to All and Cancel. The answers go to this batch.
    pub too_big: Option<reprisefs::bincap::TooBig>,
    /// Undo stopped at an item its Recycle Bin refuses: Yes undoes again, erasing it
    /// ([`reprisefs::undo::UndoStack::undo_erasing`]).
    pub undo_erase: bool,
}

impl Confirm {
    pub(crate) fn buttons(&self) -> Vec<String> {
        let labels: &[&str] = match &self.too_big {
            Some(q) if q.many() => &["&Yes", "Yes to &All", "&No", "Cancel"],
            _ => &["&Yes", "&No"],
        };
        labels.iter().map(|s| s.to_string()).collect()
    }

    /// The answer button `id` gives the question about an item too big
    /// for its Recycle Bin; Escape and the close box cancel the batch.
    fn too_big_answer(&self, out: &form::Event) -> Option<reprisefs::replace::Answer> {
        use reprisefs::replace::Answer;
        let many = self.too_big.as_ref()?.many();
        let answers: &[Answer] = match many {
            true => &[Answer::Yes, Answer::YesToAll, Answer::No, Answer::Cancel],
            false => &[Answer::Yes, Answer::No],
        };
        match out {
            form::Event::Default => Some(Answer::Yes),
            form::Event::Cancel => Some(Answer::Cancel),
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
    /// The Properties sheet's Change... raised it: OK writes the default
    /// and puts the program on the sheet instead of running it.
    pub set_default: bool,
    pub icon: Option<Canvas>,
    pub row_icons: Vec<Option<Canvas>>,
    pub input: repriseapp::openwith::ChooserInput,
    pub browse: Option<ProgramBrowse>,
}

/// The Open With chooser's Browse... picker: a file dialog whose Open
/// adds the picked program to the chooser's list.
pub struct ProgramBrowse {
    pub host: Host,
    pub dlg: FileDlg,
}

pub struct ReplaceAsk {
    pub fields: replacebox::Fields,
    pub asker: reprisefs::replace::Asker,
    pub pending: crate::fileops::PendingPut,
    pub focus: Option<&'static str>,
}

pub enum Dialog {
    Message {
        title: String,
        text: String,
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
    },
    /// Change Icon, behind Customize's Change Icon...: OK hands the
    /// picked icon to the sheet; `name` is the folder's, for the title.
    ChangeIcon {
        form: Form,
        name: String,
    },
    About(Form),
    Browse(crate::browse::Browse),
    /// Unlock Drive, over a locked drive opened from Computer.
    Unlock(Box<Unlock>),
}

impl Dialog {
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
            Dialog::About(_) => "About Explorer".to_string(),
            Dialog::Browse(b) if b.mv => "Move Items".to_string(),
            Dialog::Browse(_) => "Copy Items".to_string(),
            Dialog::Unlock(_) => unlock::TITLE.to_string(),
        }
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

    /// Raise a box titled `title` over `text` with the theme's error icon. While a replace prompt or
    /// another box is up the box waits for it (`messages_waiting`).
    pub fn message_box(&mut self, title: &str, text: String, ctx: &mut Ctx) -> Reply {
        if matches!(
            self.dialog,
            Some(Dialog::Replace(_) | Dialog::Message { .. })
        ) {
            self.messages_waiting.push_back((title.to_string(), text));
            return Reply::none();
        }
        if self.error_icon.is_none() {
            self.error_icon = crate::chrome_icon(ctx, &["dialog-error", "dialog-warning"], 32);
        }
        self.dialog = Some(Dialog::Message {
            title: title.to_string(),
            text,
        });
        Reply::redraw_all()
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
        let icon = crate::chrome_icon(ctx, &["document-properties"], 32);
        let form = reprisekit::controls::filesheet::advanced_form(
            subject,
            icon,
            robots,
            &self.dialog_text,
            self.forms.fonts.size_px,
            self.forms.text,
        );
        let live = robots.is_some();
        self.dialog = Some(Dialog::Advanced {
            form,
            subject,
            live,
        });
        Reply::redraw_all()
    }

    pub fn about_box(&mut self, ctx: &mut Ctx) -> Reply {
        let icon = crate::chrome_icon(ctx, &["system-file-manager"], 32);
        let fields = repriseapp::about::fields("Explorer", icon);
        let px = ctx.theme.fonts.size_px;
        let mut form = aboutbox::form(&fields, &ctx.text, px);
        let u = self.forms.text;
        let w = u.len(aboutbox::CLIENT.0);
        form.set_picture("banner", aboutbox::banner_wide(&ctx.text, px, w, u));
        self.dialog = Some(Dialog::About(form));
        Reply::redraw_all()
    }

    /// Raise a delete prompt over `paths` on the template its kind takes (see [`dlgtemplate`]). `no_bin` leads with the drive having no Recycle Bin for some or all of `paths` ([`reprisefs::bincap::delete_prompt`]). `empty_bin` is the Recycle Bin's Empty task, which counts the listed items since it has no paths; `bin` is the picked items of a prompt raised in the Recycle Bin, which Yes deletes for good.
    #[allow(clippy::too_many_arguments)]
    pub fn confirm_box(
        &mut self,
        kind: ConfirmKind,
        name: &str,
        paths: Vec<PathBuf>,
        erase: bool,
        no_bin: reprisefs::bincap::BinLack,
        icon: Option<Canvas>,
        empty_bin: bool,
        bin: Option<Vec<crate::fileops::BinItem>>,
    ) -> Reply {
        let count = if empty_bin {
            self.items.len()
        } else {
            paths.len()
        };
        let (title, text) = reprisefs::bincap::delete_prompt(kind, name, count, erase, no_bin);
        let min_text_h = (kind == ConfirmKind::Folder).then_some(FOLDER_TEXT_H);
        self.dialog = Some(Dialog::Confirm(Confirm {
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
            empty_bin,
            bin,
            focus: None,
            too_big: None,
            undo_erase: false,
        }));
        Reply::redraw_all()
    }

    fn box_fields(&self) -> Option<msgbox::Fields> {
        match self.dialog.as_ref()? {
            Dialog::Message { title, text } => Some(msgbox::Fields {
                text: text.clone(),
                icon: self.error_icon.clone(),
                buttons: vec!["OK".to_string()],
                default: 0,
                min_text_h: None,
                min_w: repriseapp::caption_fit(title),
            }),
            Dialog::Confirm(_)
            | Dialog::ChooseProgram(_)
            | Dialog::Replace(_)
            | Dialog::AttrConfirm(_)
            | Dialog::Advanced { .. }
            | Dialog::ChangeIcon { .. }
            | Dialog::About(_)
            | Dialog::Browse(_)
            | Dialog::Unlock(_) => None,
        }
    }

    fn replace_form(&self) -> Option<Form> {
        let Some(Dialog::Replace(r)) = self.dialog.as_ref() else {
            return None;
        };
        let (px, u) = (self.forms.fonts.size_px, self.forms.text);
        let mut f = replacebox::form(&r.fields, &self.dialog_text, px, u);
        if let Some(id) = r.focus {
            f.focus_id(id);
        }
        Some(self.built(f))
    }

    fn confirm_layout(&self) -> Option<dlgtemplate::Layout> {
        let Some(Dialog::Confirm(c)) = &self.dialog else {
            return None;
        };
        let lay = dlgtemplate::layout(
            c.template,
            &self.dialog_text,
            self.forms.fonts.size_px,
            &dlgtemplate::Fields {
                text: &c.text,
                icon: true,
                field: false,
                buttons: c.buttons().len(),
                text_h: c.min_text_h,
            },
            self.forms.text,
        );
        Some(lay.client())
    }

    /// The form of whichever text box is up, focus on the button the keys left it on.
    pub fn box_form(&self) -> Option<Form> {
        if let (Some(lay), Some(Dialog::Confirm(c))) = (self.confirm_layout(), &self.dialog) {
            let mut f = dlgtemplate::form(&lay, c.icon.clone(), &c.buttons(), 0);
            if let Some(id) = c.focus {
                f.focus_id(id);
            }
            return Some(self.built(f));
        }
        let (px, u) = (self.forms.fonts.size_px, self.forms.text);
        let f = msgbox::form(&self.box_fields()?, &self.dialog_text, px, u);
        Some(self.built(f))
    }

    pub fn min_size(&self, win: WinId) -> Option<(i32, i32)> {
        match (win, &self.dialog) {
            (DIALOG, Some(Dialog::Browse(_))) => Some(browsefolder::min(&self.forms.controls())),
            (PROGRAM_BROWSE, Some(Dialog::ChooseProgram(cp))) => {
                cp.browse.as_ref().map(|b| b.dlg.min_client())
            }
            _ => None,
        }
    }

    /// The caption `win` wears: the sheets and pickers add the ? box; the message boxes, confirmations and About have the close box alone.
    pub fn caption(&self, win: WinId) -> Caption {
        match win {
            DIALOG => match self.dialog {
                Some(
                    Dialog::ChooseProgram(_)
                    | Dialog::Browse(_)
                    | Dialog::Advanced { .. }
                    | Dialog::ChangeIcon { .. },
                ) => Caption::Help,
                _ => Caption::Prompt,
            },
            PROGRAM_BROWSE | crate::props_dialog::PROPS => Caption::Help,
            crate::browse::ERROR => Caption::Prompt,
            crate::copydlg::COPY | crate::copydlg::REMOVE => Caption::Prompt,
            _ => Caption::Window,
        }
    }

    pub fn children(&self) -> Vec<Child> {
        let mut out = Vec::new();
        let (text, px, u) = (&self.dialog_text, self.forms.fonts.size_px, self.forms.text);
        if let Some(d) = &self.dialog {
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
                    Dialog::About(_) => u.size(aboutbox::CLIENT),
                    Dialog::Browse(_) => browsefolder::client(&self.forms.controls()),
                    Dialog::ChooseProgram(_) => openwith::client(&self.forms.controls()),
                    Dialog::Unlock(_) => u.size(unlock::CLIENT),
                    Dialog::Confirm(_) => {
                        u.size(self.confirm_layout().expect("a delete prompt is up").size)
                    }
                    Dialog::Message { .. } => unreachable!("box_fields answers it"),
                },
            };
            out.push(Child {
                id: DIALOG,
                title: d.title(),
                size,
                fixed: !matches!(d, Dialog::Browse(_)),
                modal: true,
            });
        }
        // A name the picker's rename box was refused: the error box over
        // the picker.
        if let Some(f) = self.browse_error_fields() {
            out.push(Child {
                id: crate::browse::ERROR,
                title: crate::browse::ERROR_TITLE.into(),
                size: u.size(msgbox::client(&f, text, px, u)),
                fixed: true,
                modal: true,
            });
        }
        if let Some(Dialog::ChooseProgram(cp)) = &self.dialog
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
        out.extend(self.progress_children());
        out
    }

    pub fn draw_dialog(&self, c: &mut Canvas, ctx: &Ctx) {
        c.fill_rect(c.bounds(), ctx.theme.colors.button_face);
        if let Some(Dialog::Unlock(u)) = &self.dialog {
            u.draw(c, ctx);
            return;
        }
        if let Some(Dialog::Browse(b)) = &self.dialog {
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
        if let Some(Dialog::ChooseProgram(cp)) = &self.dialog {
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
        let form = match &self.dialog {
            Some(Dialog::AttrConfirm(a)) => &a.form,
            Some(Dialog::Advanced { form, .. }) => form,
            Some(Dialog::ChangeIcon { form, .. }) => form,
            Some(Dialog::About(f)) => f,
            Some(_) => match self.box_form().or_else(|| self.replace_form()) {
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
        if let Some(Dialog::Unlock(u)) = &self.dialog {
            u.access(tree, ctx);
            return;
        }
        if let Some(Dialog::Browse(b)) = &self.dialog {
            b.bf.access(
                tree,
                &self.dialog_text,
                self.forms.fonts.size_px,
                &self.forms.controls(),
            );
            return;
        }
        if let Some(Dialog::ChooseProgram(cp)) = &self.dialog {
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
        let form = match &self.dialog {
            Some(Dialog::AttrConfirm(a)) => &a.form,
            Some(Dialog::Advanced { form, .. }) => form,
            Some(Dialog::ChangeIcon { form, .. }) => form,
            Some(Dialog::About(f)) => f,
            Some(_) => match self.box_form().or_else(|| self.replace_form()) {
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
    /// button, on Escape and on the frame's close box.
    pub fn dialog_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        if matches!(ev, Event::CloseRequest) {
            // The close box on a replace prompt is its Cancel.
            self.dialog = None;
            return self.show_waiting(ctx);
        }
        if self.dialog.is_none() {
            return Reply::none();
        }
        if matches!(self.dialog, Some(Dialog::Browse(_))) {
            return self.browse_event(ev, ctx);
        }
        if matches!(self.dialog, Some(Dialog::ChooseProgram(_))) {
            return self.chooser_event(ev, ctx);
        }
        if matches!(self.dialog, Some(Dialog::Unlock(_))) {
            return self.unlock_event(ev, ctx);
        }
        // The boxes that keep a form (attribute confirm, Advanced Attributes, Change Icon) take their events in place; the text boxes rebuild theirs per event.
        let kept = match self.dialog.as_mut() {
            Some(Dialog::AttrConfirm(a)) => Some(&mut a.form),
            Some(Dialog::Advanced { form, .. }) => Some(form),
            Some(Dialog::ChangeIcon { form, .. }) => Some(form),
            Some(Dialog::About(f)) => Some(f),
            _ => None,
        };
        let out = if let Some(form) = kept {
            match ev {
                Event::Key(k) => match dialog_key(&k) {
                    Some(key) => form.key(key),
                    None => return Reply::none(),
                },
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
                // A push button acts on the release over it; the release
                // also lets go of whatever the press held.
                Event::Pointer(PointerInput::Release {
                    button: BTN_LEFT, ..
                }) => form.release(),
                Event::Pointer(PointerInput::Motion { x, y }) => {
                    let hit = form.hit(x, y, &self.dialog_text, self.forms.fonts.size_px);
                    form.motion(x, y, hit)
                }
                Event::Pointer(PointerInput::Scroll { x, y, rows }) => {
                    let hit = form.hit(x, y, &self.dialog_text, self.forms.fonts.size_px);
                    form.wheel(hit, rows)
                }
                Event::Pointer(PointerInput::Leave) => form.leave(),
                _ => return Reply::none(),
            }
        } else {
            let Some(mut f) = self.box_form().or_else(|| self.replace_form()) else {
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

    /// Keep the focus of the button `f` is on in the prompt `f` was built for; the next form puts it back so Enter and Space press it.
    fn keep_box_focus(&mut self, f: &Form) {
        let id = f.focus().map(|i| f.items[i].id);
        match self.dialog.as_mut() {
            Some(Dialog::Confirm(c)) => c.focus = id,
            Some(Dialog::Replace(r)) => r.focus = id,
            _ => {}
        }
    }

    pub(crate) fn dialog_outcome(&mut self, out: form::Event, ctx: &mut Ctx) -> Reply {
        enum Answer {
            Stay,
            Close,
            Accept,
        }
        if let (Some(Dialog::ChangeIcon { form, .. }), form::Event::Pressed(id)) =
            (self.dialog.as_mut(), &out)
            && *id == reprisekit::controls::filesheet::CHANGE_ICON_RESTORE
        {
            reprisekit::controls::filesheet::change_icon_restore(form);
            return Reply::redraw();
        }
        let Some(d) = &self.dialog else {
            return Reply::none();
        };
        if let Dialog::Confirm(c) = d
            && c.too_big.is_some()
        {
            return match c.too_big_answer(&out) {
                Some(a) => {
                    let Some(Dialog::Confirm(c)) = self.dialog.take() else {
                        return Reply::none();
                    };
                    let mut asker = c.too_big.expect("a question about an item too big");
                    asker.answer(a);
                    let reply = self.too_big_step(asker, ctx);
                    match self.dialog.is_some() {
                        true => Reply {
                            redraw_all: true,
                            ..reply
                        },
                        false => Reply {
                            redraw_all: true,
                            ..self.show_waiting(ctx)
                        },
                    }
                }
                None if out == form::Event::None => Reply::none(),
                None => Reply::redraw(),
            };
        }
        let answer = match d {
            Dialog::Message { .. } => match out {
                form::Event::Cancel | form::Event::Default => Answer::Close,
                form::Event::Pressed(id) if id == msgbox::OK => Answer::Close,
                _ => Answer::Stay,
            },
            Dialog::Confirm(_) => match out {
                form::Event::Cancel => Answer::Close,
                form::Event::Pressed(id) if id == msgbox::button_id(1) => Answer::Close,
                form::Event::Default => Answer::Accept,
                form::Event::Pressed(id) if id == msgbox::button_id(0) => Answer::Accept,
                _ => Answer::Stay,
            },
            Dialog::About(_) => match out {
                form::Event::Cancel | form::Event::Default => Answer::Close,
                form::Event::Pressed(id) if id == aboutbox::OK => Answer::Close,
                _ => Answer::Stay,
            },
            // OK hands the pick to the sheet, Restore Defaults picks the
            // standard folder again.
            Dialog::ChangeIcon { .. } => {
                use reprisekit::controls::filesheet::{CHANGE_ICON_CANCEL, CHANGE_ICON_OK};
                match out {
                    form::Event::Default | form::Event::Pressed(CHANGE_ICON_OK) => Answer::Accept,
                    form::Event::Cancel | form::Event::Pressed(CHANGE_ICON_CANCEL) => Answer::Close,
                    _ => Answer::Stay,
                }
            }
            Dialog::Advanced { live, .. } => match out {
                form::Event::Default | form::Event::Pressed(ADV_OK) if *live => Answer::Accept,
                form::Event::Cancel | form::Event::Default => Answer::Close,
                form::Event::Pressed(ADV_OK | ADV_CANCEL) => Answer::Close,
                _ => Answer::Stay,
            },
            Dialog::AttrConfirm(_) => match out {
                form::Event::Cancel => Answer::Close,
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
            Dialog::ChooseProgram(_) | Dialog::Unlock(_) => Answer::Stay,
        };
        match answer {
            Answer::Stay if out == form::Event::None => Reply::none(),
            Answer::Stay => Reply::redraw(),
            Answer::Close => {
                // Cancelling the attribute question leaves the sheet's
                // box where the file says it should be.
                let attrs = matches!(self.dialog, Some(Dialog::AttrConfirm(_)));
                self.dialog = None;
                if attrs {
                    return self.attrs_cancelled();
                }
                self.show_waiting(ctx)
            }
            Answer::Accept => match self.dialog.take() {
                Some(Dialog::Confirm(c)) if c.empty_bin => self.empty_bin_now(ctx),
                Some(Dialog::Confirm(c)) if c.undo_erase => self.undo_with(true, ctx),
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
    fn replace_answered(&mut self, c: replacebox::Choice, ctx: &mut Ctx) -> Reply {
        use reprisefs::replace::Answer;
        let Some(Dialog::Replace(r)) = self.dialog.take() else {
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
        if self.dialog.is_some() {
            return Reply {
                redraw_all: true,
                ..reply
            };
        }
        self.show_waiting(ctx)
    }

    /// With no box up, raise what waited for the replace prompt: the next
    /// batch's prompt first, then the error boxes, one at a time.
    pub(crate) fn show_waiting(&mut self, ctx: &mut Ctx) -> Reply {
        while self.dialog.is_none() && self.asking.is_none() {
            if let Some((pending, asker)) = self.replace_waiting.pop_front() {
                self.ask_replace(pending, asker, ctx);
            } else if let Some((title, text)) = self.messages_waiting.pop_front() {
                self.message_box(&title, text, ctx);
            } else {
                break;
            }
        }
        Reply::redraw_all()
    }
}
