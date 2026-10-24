//! Each kind of dialog's constructor.

use super::*;
use crate::config::PowerConfig;
use crate::theme::TextUnits;
use reprisekit::controls::copyprogress;
use reprisekit::controls::dlgtemplate::KEY_AID_TEXT_H;

pub(super) fn button(label: &str, action: DialogAction, default: bool) -> DialogButton {
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

    pub(super) fn copy_outcome(o: copyprogress::Outcome) -> Outcome {
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

    pub(super) fn tray_outcome(ev: traycustomize::Event) -> Outcome {
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

    pub(super) fn chooser_outcome(&self, ev: openwith::Event) -> Outcome {
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
                    DialogAction::StandBy(power.standby.clone()),
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
}
