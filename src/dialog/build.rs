//! Each kind of dialog's constructor.

use super::*;
use crate::config::PowerConfig;
use crate::theme::TextUnits;
use reprisekit::controls::dlgtemplate::KEY_AID_TEXT_H;
use reprisekit::controls::pathcomplete::Scope;
use reprisekit::{kit_tr, tr};

pub(super) fn button(label: &str, action: DialogAction, default: bool) -> DialogButton {
    DialogButton {
        label: label.into(),
        action,
        default,
        choice: None,
        enabled: true,
        tip: None,
    }
}

impl Dialog {
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
            locationprompt::title(),
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
            Style::Form(f) if f.kind == FormKind::EndSession => Some(f.open_page_mut()),
            _ => None,
        }
    }

    /// The Run dialog: a command line, OK / Cancel / Browse. It opens on
    /// the last command in `history` (newest first), selected.
    pub fn run(history: &[String]) -> Dialog {
        Dialog {
            icon: Some("system-run".into()),
            text: tr!("dlg-run-text"),
            field: Some(Field {
                label: tr!("dlg-run-open"),
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
                complete: PathComplete::new(Scope::Shell),
            }),
            buttons: vec![
                button(&kit_tr!("ok"), DialogAction::RunField, true),
                button(&kit_tr!("cancel"), DialogAction::Cancel, false),
                button(&tr!("dlg-run-browse"), DialogAction::BrowseRun, false),
            ],
            help: true,
            ..Dialog::new(
                tr!("dlg-run-title"),
                Style::Classic {
                    template: Template::Run,
                    text_h: None,
                    delete_badge: false,
                },
                Placement::ByStart,
            )
        }
    }

    /// Rename Desktop for desktop `n`: Run's box with its `name` (if any) in the field, selected.
    pub fn rename_desktop(n: usize, name: Option<&String>) -> Dialog {
        let name = name.map_or("", |s| s.trim());
        Dialog {
            icon: Some("preferences-desktop".into()),
            text: tr!("dlg-rename-desktop-text", number = (n + 1).to_string()),
            field: Some(Field {
                label: tr!("dlg-rename-desktop-name"),
                combo: {
                    let mut c = Combo::new(&[], 0, LIST_ROWS);
                    c.edit = Some(TextEdit::new(name.to_string(), !name.is_empty()));
                    c
                },
                thumb: ThumbDrag::default(),
                complete: PathComplete::new(Scope::Rooted),
            }),
            buttons: vec![
                button(&kit_tr!("ok"), DialogAction::RenameDesktop(n), true),
                button(&kit_tr!("cancel"), DialogAction::Cancel, false),
            ],
            ..Dialog::new(
                tr!("dlg-rename-desktop-title"),
                Style::Classic {
                    template: Template::Run,
                    text_h: None,
                    delete_badge: false,
                },
                Placement::Center,
            )
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

    /// "Confirm Delete" on a system icon: Yes only takes the icon off
    /// the desktop (the folder behind Documents or Home is left alone), No
    /// and Escape keep it. Listing it in `system_icons` again brings it
    /// back.
    pub fn confirm_hide(place: crate::desktop::Place, label: &str) -> Dialog {
        let restore = match place {
            crate::desktop::Place::Documents | crate::desktop::Place::Home => {
                tr!("dlg-hide-restore-folder")
            }
            _ => tr!("dlg-hide-restore"),
        };
        Dialog {
            // Folder with a document under the `draw_delete_badge` badge; `folder-documents` is the
            // nearest themed icon.
            icon: Some("folder-documents".into()),
            text: tr!("dlg-hide-text", label = label, restore = restore),
            buttons: vec![
                button(
                    &kit_tr!("yes"),
                    DialogAction::HideSystemIcon(place.config_name().into()),
                    true,
                ),
                button(&kit_tr!("no"), DialogAction::Cancel, false),
            ],
            ..Dialog::new(
                tr!("dlg-confirm-delete-title"),
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
            tr!("dlg-icons-restore")
        } else {
            tr!("dlg-icons-restore-files")
        };
        let no = DialogAction::DeleteIcons {
            hide: Vec::new(),
            files: files.clone(),
        };
        Dialog {
            icon: Some("folder".into()),
            text: tr!("dlg-icons-text", restore = restore),
            buttons: vec![
                button(
                    &kit_tr!("yes"),
                    DialogAction::DeleteIcons { hide, files },
                    true,
                ),
                button(&kit_tr!("no"), no, false),
            ],
            ..Dialog::new(
                tr!("dlg-confirm-delete-title"),
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
    /// turned off. Without `settings` (the login screen) Settings... is greyed.
    pub fn key_aid_prompt(aid: crate::keyfilter::Aid, settings: bool) -> Dialog {
        use crate::keyfilter::Aid;
        let (title, text) = match aid {
            Aid::StickyKeys => (
                tr!("dlg-key-aid-sticky-title"),
                tr!("dlg-key-aid-sticky-text"),
            ),
            Aid::SlowKeys => (tr!("dlg-key-aid-slow-title"), tr!("dlg-key-aid-slow-text")),
            Aid::MouseKeys => (
                tr!("dlg-key-aid-mouse-title"),
                tr!("dlg-key-aid-mouse-text"),
            ),
        };
        Dialog {
            text,
            buttons: vec![
                button(&kit_tr!("yes"), DialogAction::TurnOn(aid), true),
                button(&kit_tr!("no"), DialogAction::Cancel, false),
                DialogButton {
                    enabled: settings,
                    ..button(
                        &tr!("dlg-key-aid-settings"),
                        DialogAction::Spawn(aid.settings_argv().join(" ")),
                        false,
                    )
                },
            ],
            ..Dialog::new(
                title,
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
            text: tr!("dlg-end-program-text"),
            buttons: vec![
                button(
                    &tr!("dlg-end-program-end"),
                    DialogAction::EndProgram(pid),
                    false,
                ),
                button(&kit_tr!("cancel"), DialogAction::Cancel, true),
            ],
            ..Dialog::new(
                tr!("dlg-end-program-title", program = program),
                Style::EndProgram,
                Placement::Center,
            )
        }
    }

    /// The question before the `program` file at `path` runs (`confirm_scripts`); Run remembers
    /// the pick and runs a script as `open` says.
    pub fn ask_run(
        path: std::path::PathBuf,
        open: reprisefs::program::ScriptOpen,
        program: reprisefs::program::Program,
    ) -> Dialog {
        use reprisefs::program::{Asked, Program};
        let asked = Asked::of(program);
        let labels = asked.buttons();
        let script = |run| DialogAction::Script {
            path: path.clone(),
            run,
            program,
        };
        let is_script = program == Program::Script;
        let mut buttons = vec![button(&labels[0], script(Some(open)), is_script)];
        if is_script {
            buttons.push(button(&labels[1], script(None), false));
        }
        buttons.push(button(
            &labels[labels.len() - 1],
            DialogAction::Cancel,
            !is_script,
        ));
        Dialog::run_question(&path, asked, buttons)
    }

    /// The question before `path`, which its type's program runs, opens in `with` (a desktop
    /// entry id) or its default program; Run remembers the pick, Cancel is the default.
    pub fn ask_open(
        path: std::path::PathBuf,
        with: Option<reprisefs::apps::DesktopId>,
        recent: bool,
    ) -> Dialog {
        let asked = reprisefs::program::Asked::Typed;
        let labels = asked.buttons();
        let run = DialogAction::OpenRunnable {
            path: path.clone(),
            with,
            recent,
        };
        let buttons = vec![
            button(&labels[0], run, false),
            button(&labels[1], DialogAction::Cancel, true),
        ];
        Dialog::run_question(&path, asked, buttons)
    }

    /// The question when desktop Undo would bin an item and its Recycle Bin refuses (`text`,
    /// [`reprisefs::undo::NoBin::question`]): Yes undoes `entry` again and deletes the item for
    /// good, No leaves the entry for a later Undo.
    pub fn ask_undo_erase(text: String, entry: reprisefs::undo::EntryId) -> Dialog {
        Dialog {
            icon: Some("text-x-generic".into()),
            text,
            buttons: vec![
                button(&kit_tr!("yes"), DialogAction::UndoErase(entry), true),
                button(&kit_tr!("no"), DialogAction::Cancel, false),
            ],
            ..Dialog::new(
                tr!("dlg-confirm-file-delete-title"),
                Style::Classic {
                    template: Template::ShellNotice,
                    text_h: None,
                    delete_badge: true,
                },
                Placement::Center,
            )
        }
    }

    /// The question before the disk image at `path` is mounted: Mount mounts it read-only,
    /// Cancel, the default, leaves it.
    pub fn ask_mount(path: std::path::PathBuf) -> Dialog {
        use reprisefs::udisks::{mount_buttons, mount_text, mount_title};
        let [mount, cancel] = mount_buttons();
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        Dialog {
            icon: Some("dialog-warning".into()),
            text: mount_text(&name),
            buttons: vec![
                button(&mount, DialogAction::MountImage(path.clone()), false),
                button(&cancel, DialogAction::Cancel, true),
            ],
            ..Dialog::new(
                mount_title(),
                Style::Classic {
                    template: Template::MessageBox,
                    text_h: None,
                    delete_badge: false,
                },
                Placement::Center,
            )
        }
    }

    fn run_question(
        path: &std::path::Path,
        asked: reprisefs::program::Asked,
        buttons: Vec<DialogButton>,
    ) -> Dialog {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        Dialog {
            icon: Some("dialog-warning".into()),
            text: asked.text(&name),
            buttons,
            ..Dialog::new(
                reprisefs::program::ask_title(),
                Style::Classic {
                    template: Template::MessageBox,
                    text_h: None,
                    delete_badge: false,
                },
                Placement::Center,
            )
        }
    }

    /// The question when a restart target held a power action back (`text`): `go` goes ahead with
    /// `again` anyway, Cancel, the default, stays.
    pub fn power_anyway(
        title: &str,
        text: String,
        go: &str,
        again: crate::sessionend::Again,
    ) -> Dialog {
        Dialog {
            icon: Some("dialog-warning".into()),
            text,
            buttons: vec![
                button(go, DialogAction::PowerAnyway(again), false),
                button(&kit_tr!("cancel"), DialogAction::Cancel, true),
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

    /// Message box with the error icon and OK.
    pub fn error(title: &str, text: String) -> Dialog {
        Dialog {
            icon: Some("dialog-error".into()),
            text,
            buttons: vec![button(&kit_tr!("ok"), DialogAction::Cancel, true)],
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

    /// Run's answer to a line whose program `word` is not there; the Run box stays open under it.
    pub fn run_not_found(word: &str) -> Dialog {
        Dialog::error(word, tr!("dlg-run-not-found", word = word))
    }

    /// The error for a picked program at `path` whose name is not
    /// UTF-8: a command line and a desktop entry's `Exec` hold text only,
    /// so it cannot be run from one.
    pub fn not_utf8_program(title: &str, path: &std::path::Path) -> Dialog {
        Dialog::error(
            title,
            tr!("dlg-not-utf8", path = path.display().to_string()),
        )
    }

    /// The Log Off panel: `leave` (Switch User or Disconnect), and Log Off, which runs the
    /// `exit` action and holds the focus.
    pub fn log_off(leave: Leave) -> Dialog {
        Dialog::welcome(Welcome {
            panel: Panel::log_off(leave),
            actions: vec![
                (Choice::SwitchUser, DialogAction::SwitchUser),
                (Choice::Disconnect, DialogAction::Disconnect),
                (Choice::LogOff, DialogAction::Act(Action::Exit)),
            ],
            always_blue: false,
            updates: false,
        })
    }

    /// The Turn off computer panel running the `[power]` commands, Disconnect in Stand By's place
    /// with `disconnect`; Stand By and Hibernate are greyed or left out while their command is unset.
    pub fn turn_off(power: &PowerConfig, disconnect: bool) -> Dialog {
        let set = |cmd: &str| !cmd.trim().is_empty();
        let first = if disconnect {
            First::Disconnect
        } else {
            First::StandBy {
                live: set(&power.standby),
                hibernate: set(&power.hibernate),
            }
        };
        Dialog::welcome(Welcome {
            panel: Panel::turn_off(first),
            actions: vec![
                (Choice::Disconnect, DialogAction::Disconnect),
                (
                    Choice::StandBy,
                    DialogAction::StandBy(power.standby.clone()),
                ),
                (
                    Choice::Hibernate,
                    DialogAction::Hibernate(power.hibernate.clone()),
                ),
                (
                    Choice::TurnOff,
                    DialogAction::EndSession(Goal::TurnOff, power.poweroff.clone()),
                ),
                (
                    Choice::Restart,
                    DialogAction::EndSession(Goal::Restart, power.reboot.clone()),
                ),
            ],
            always_blue: false,
            updates: true,
        })
    }

    /// The login screen's Turn off computer panel: Stand By and Hibernate greyed, Turn Off and
    /// Restart asked of reprise-login.
    pub fn greeter_turn_off() -> Dialog {
        use repriselogin::greeter::PowerAction;
        Dialog::welcome(Welcome {
            panel: Panel::turn_off(First::StandBy {
                live: false,
                hibernate: false,
            }),
            actions: vec![
                (
                    Choice::TurnOff,
                    DialogAction::GreeterPower(PowerAction::PowerOff),
                ),
                (
                    Choice::Restart,
                    DialogAction::GreeterPower(PowerAction::Reboot),
                ),
            ],
            always_blue: false,
            updates: false,
        })
    }

    fn welcome(w: Welcome) -> Dialog {
        let title = w.panel.title.clone();
        let mut d = Dialog::new(title, Style::Welcome(Box::new(w)), Placement::UpperThird);
        d.sync_welcome();
        d
    }

    /// A Welcome panel drawn Blue under glass too.
    pub fn always_blue(mut self) -> Dialog {
        if let Style::Welcome(w) = &mut self.style {
            w.always_blue = true;
        }
        self
    }
}
