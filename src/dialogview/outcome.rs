use crate::compositor::State;
use crate::dialog::{self, Dialog, Outcome};
use reprisefs::program::{Confirm, Program};

use super::*;

impl State {
    pub(crate) fn apply_dialog(&mut self, outcome: Outcome) {
        self.run_picker_requests();
        match outcome {
            Outcome::Nothing => {}
            Outcome::Redraw => self.redraw_top_dialog(),
            Outcome::Relayout => self.relayout_top_dialog(),
            Outcome::Size(edges) => self.begin_dialog_size(edges),
            Outcome::Close | Outcome::CopyCancel
                if self
                    .dialogs
                    .views
                    .last()
                    .is_some_and(|v| v.dialog.copy().is_some()) =>
            {
                let id = self.dialogs.views[self.dialogs.views.len() - 1].id;
                self.cancel_put(id);
            }
            Outcome::CopyCancel => {}
            Outcome::CopyPause(on) => {
                if let Some(id) = self.dialogs.views.last().map(|v| v.id) {
                    self.pause_put(id, on);
                }
            }
            Outcome::Close => self.close_dialog(),
            Outcome::Spawn(cmd) => {
                self.close_dialog();
                self.spawn_activated(&cmd);
            }
            Outcome::StandBy(cmd) => {
                self.close_dialog();
                crate::sessionend::sleep(self, crate::sessionend::Act::StandBy, cmd);
            }
            Outcome::EndSession(goal, cmd) => {
                self.close_dialog();
                crate::sessionend::begin(self, goal, Some(cmd));
            }
            Outcome::SessionEnd(choice) => crate::sessionend::answer(self, choice),
            Outcome::Location { desktop_id, answer } => {
                crate::location::answer(self, &desktop_id, answer)
            }
            Outcome::Delete { paths, erase } => {
                self.close_dialog();
                self.delete_paths(paths, erase);
            }
            Outcome::EmptyBin => {
                self.close_dialog();
                self.empty_bin();
            }
            Outcome::Notice(d) => self.open_dialog_over(*d),
            Outcome::Run(cmd) => {
                self.close_dialog();
                dialog::remember(&mut self.dialogs.run_history, &cmd);
                crate::state::save_dialogs(&self.dialogs.run_history);
                self.run_line_activated(&cmd);
            }
            Outcome::Act(a) => {
                self.close_dialog();
                self.run_action(&a, 0);
            }
            Outcome::OpenWith {
                path,
                app,
                always,
                mime,
            } => self.apply_open_with(&path, &app, always, mime.as_deref()),
            Outcome::ChangeDefault(path) => {
                // A symlink's Change... asks for the type of its target, resolved in a file job.
                self.spawn_fs_job(move || {
                    let real = std::fs::canonicalize(&path).unwrap_or(path);
                    Box::new(move |state: &mut State| {
                        let d = state.choose_program(&real, true);
                        state.open_dialog_over(d);
                    })
                });
            }
            Outcome::SetDefault { path, app, mime } => {
                self.apply_set_default(&path, &app, mime.as_deref())
            }
            Outcome::AdvancedAttributes { subject, robots } => {
                let t = &self.config.theme;
                let px = t.fonts.size_px;
                let d = Dialog::advanced_attributes(subject, robots, &self.text, px, t.text);
                self.open_dialog_over(d)
            }
            Outcome::AdvancedChosen(chosen) => {
                if !self.close_to_parent() {
                    return;
                }
                if let Some(i) = self.dialogs.views.len().checked_sub(1)
                    && self.dialogs.views[i].dialog.advanced_chosen(chosen)
                {
                    self.redraw_dialog(i);
                }
            }
            Outcome::BrowseRun => self.browse_run(),
            Outcome::BrowseProgram => self.browse_program(),
            Outcome::ProgramPick(path) => {
                if !self.close_to_parent() {
                    return;
                }
                let outcome = match self.dialogs.views.last_mut() {
                    Some(v) => v.dialog.add_program(&path),
                    None => Outcome::Nothing,
                };
                self.apply_dialog(outcome);
            }
            Outcome::RunPick(path) => {
                if !self.close_to_parent() {
                    return;
                }
                // The field holds text, so a non-UTF-8 name cannot go into it.
                let Some(picked) = path.to_str() else {
                    self.open_dialog_over(Dialog::not_utf8_program("Run", &path));
                    return;
                };
                let quoted = reprisefs::ops::shell_quote(picked);
                let outcome = match self.dialogs.views.last_mut() {
                    Some(v) => v.dialog.set_field(&quoted),
                    None => Outcome::Nothing,
                };
                self.restart_caret();
                self.apply_dialog(outcome);
            }
            Outcome::Properties {
                path,
                name,
                attrs,
                hidden,
                summary,
                folder_icon,
                launcher,
                close,
                recurse,
            } => self.apply_properties(
                Props {
                    path,
                    name,
                    attrs,
                    hidden,
                    summary,
                    folder_icon,
                    launcher,
                },
                close,
                recurse,
            ),
            Outcome::SetProperties {
                paths,
                attrs,
                hidden,
                folder_icon,
                close,
                recurse,
            } => self.apply_set_properties(
                SetWrite {
                    paths,
                    attrs,
                    hidden,
                    folder_icon,
                },
                close,
                recurse,
            ),
            Outcome::ChangeIcon {
                name,
                current,
                launcher,
            } => self.open_change_icon(&name, current, launcher),
            Outcome::FindTarget(program) => self.spawn_argv_activated(&[
                std::ffi::OsStr::new("reprise-explorer"),
                std::ffi::OsStr::new("--select"),
                program.as_os_str(),
            ]),
            Outcome::IconChosen(icon) => {
                if !self.close_to_parent() {
                    return;
                }
                if let Some(i) = self.dialogs.views.len().checked_sub(1)
                    && self.dialogs.views[i].dialog.icon_chosen(icon)
                {
                    self.redraw_dialog(i);
                }
            }
            Outcome::HideSystemIcon(name) => {
                self.close_dialog();
                self.hide_system_icons(&[name]);
            }
            Outcome::DeleteIcons { hide, files } => {
                self.close_dialog();
                self.hide_system_icons(&hide);
                self.trash_files(files);
            }
            Outcome::EndProgram(pid) => {
                self.close_dialog();
                self.end_program(pid);
            }
            Outcome::Script { path, run } => {
                self.close_dialog();
                match run {
                    Some(open) => self.run_program(&path, Program::Script, open, Confirm::Allow),
                    None => self.spawn_argv_activated(&reprisefs::program::notepad_argv(&path)),
                }
            }
            Outcome::TurnOn(aid) => {
                self.close_dialog();
                self.set_aid(aid, true);
                self.aid_switched(aid, true);
            }
            Outcome::TrayChoices(c) => {
                self.close_dialog();
                crate::tray::apply_choices(self, c);
            }
            Outcome::Replace { batch, answer } => {
                self.close_dialog();
                self.replace_answered(*batch, answer);
            }
            Outcome::TooBig { mut asker, answer } => {
                self.close_dialog();
                asker.answer(answer);
                self.too_big_step(*asker);
            }
            Outcome::BinSettings { settings, close } => {
                crate::configwrite::save_recycle_bin(&self.config_path, settings, true);
                if close {
                    self.close_dialog();
                } else {
                    self.redraw_top_dialog();
                }
            }
        }
    }
}
