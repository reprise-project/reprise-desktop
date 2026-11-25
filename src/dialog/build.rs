//! Each kind of dialog's constructor.

use super::*;
use crate::config::PowerConfig;
use crate::theme::TextUnits;
use reprisekit::controls::dlgtemplate::KEY_AID_TEXT_H;
use reprisekit::controls::pathcomplete::Scope;

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
            Style::Form(f) if f.kind == FormKind::EndSession => Some(f.open_page_mut()),
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
                complete: PathComplete::new(Scope::Shell),
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

    /// Rename Desktop for desktop `n`: Run's box with its `name` (if any) in the field, selected.
    pub fn rename_desktop(n: usize, name: Option<&String>) -> Dialog {
        let name = name.map_or("", |s| s.trim());
        Dialog {
            icon: Some("preferences-desktop".into()),
            text: format!(
                "Type a name for desktop {}. With no name it is called Desktop {}.",
                n + 1,
                n + 1
            ),
            field: Some(Field {
                label: "&Name:".into(),
                combo: {
                    let mut c = Combo::new(&[], 0, LIST_ROWS);
                    c.edit = Some(TextEdit::new(name.to_string(), !name.is_empty()));
                    c
                },
                thumb: ThumbDrag::default(),
                complete: PathComplete::new(Scope::Rooted),
            }),
            buttons: vec![
                button("OK", DialogAction::RenameDesktop(n), true),
                button("Cancel", DialogAction::Cancel, false),
            ],
            ..Dialog::new(
                "Rename Desktop",
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
    /// turned off. Without `settings` (the login screen) Settings... is greyed.
    pub fn key_aid_prompt(aid: crate::keyfilter::Aid, settings: bool) -> Dialog {
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
                DialogButton {
                    enabled: settings,
                    ..button(
                        "&Settings...",
                        DialogAction::Spawn(aid.settings_argv().join(" ")),
                        false,
                    )
                },
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
        let mut buttons = vec![button(labels[0], script(Some(open)), is_script)];
        if is_script {
            buttons.push(button(labels[1], script(None), false));
        }
        buttons.push(button(
            labels[labels.len() - 1],
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
            button(labels[0], run, false),
            button(labels[1], DialogAction::Cancel, true),
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
                button("&Yes", DialogAction::UndoErase(entry), true),
                button("&No", DialogAction::Cancel, false),
            ],
            ..Dialog::new(
                "Confirm File Delete",
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
        use reprisefs::udisks::{MOUNT_BUTTONS, MOUNT_TITLE, mount_text};
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        Dialog {
            icon: Some("dialog-warning".into()),
            text: mount_text(&name),
            buttons: vec![
                button(
                    MOUNT_BUTTONS[0],
                    DialogAction::MountImage(path.clone()),
                    false,
                ),
                button(MOUNT_BUTTONS[1], DialogAction::Cancel, true),
            ],
            ..Dialog::new(
                MOUNT_TITLE,
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
                reprisefs::program::ASK_TITLE,
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
                button("Cancel", DialogAction::Cancel, true),
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

    /// Run's answer to a line whose program `word` is not there; the Run box stays open under it.
    pub fn run_not_found(word: &str) -> Dialog {
        Dialog::error(
            word,
            format!(
                "Reprise cannot find '{word}'. Make sure you typed the name correctly, and then \
                 try again."
            ),
        )
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
