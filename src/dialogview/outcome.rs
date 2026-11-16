use crate::compositor::State;
use crate::dialog::{Dialog, Outcome};
use reprisefs::program::Confirm;

impl State {
    pub(crate) fn apply_dialog(&mut self, outcome: Outcome) {
        match outcome {
            Outcome::Nothing => {}
            Outcome::Redraw => self.redraw_top_dialog(),
            Outcome::Relayout => self.relayout_top_dialog(),
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
            Outcome::Hibernate(cmd) => {
                self.close_dialog();
                crate::sessionend::sleep(self, crate::sessionend::Act::Hibernate, cmd);
            }
            Outcome::EndSession(goal, cmd) => {
                self.close_dialog();
                crate::sessionend::begin(self, goal, Some(cmd), crate::lockview::Origin::Desktop);
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
            Outcome::GreeterPower(a) => {
                self.close_dialog();
                crate::greeterview::power(self, a);
            }
            Outcome::SwitchUser => {
                self.close_dialog();
                if crate::loginlink::desk_offers(self) {
                    crate::loginlink::switch_user(self);
                }
            }
            Outcome::Disconnect => {
                self.close_dialog();
                if crate::console::disconnect_offered(self) {
                    crate::console::disconnect(self);
                }
            }
            Outcome::Notice(d) => self.open_dialog_over(*d),
            Outcome::Run(cmd) => {
                let from = self.dialogs.views.last().map(|v| v.id);
                self.run_line_activated(&cmd, from);
            }
            Outcome::RenameDesktop(n, name) => {
                self.close_dialog();
                self.rename_desktop(n, name);
            }
            Outcome::Act(a) => {
                self.close_dialog();
                self.run_action(&a, 0);
            }
            Outcome::BrowseRun => self.browse_run(),
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
            Outcome::Script { path, run, program } => {
                self.close_dialog();
                match run {
                    Some(open) => self.run_program(&path, program, open, Confirm::Allow),
                    None => self.spawn_argv_activated(&reprisefs::program::notepad_argv(&path)),
                }
            }
            Outcome::OpenRunnable { path, with, recent } => {
                self.close_dialog();
                let with = with.and_then(|id| self.app_index.iter().find(|a| a.id == id).cloned());
                self.open_document(&path, with, Confirm::Allow, recent);
            }
            Outcome::MountImage(path) => {
                self.close_dialog();
                self.mount_image(&path);
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
                let tx = self.loop_sender();
                crate::configwrite::save_recycle_bin(&self.config_path, settings, true, |e| {
                    let text = format!("Cannot save the Recycle Bin settings in config.toml: {e}");
                    let shown: crate::fsjobs::OnLoop = Box::new(move |state: &mut State| {
                        state.open_dialog(Dialog::error("Recycle Bin Properties", text));
                    });
                    if let Some(tx) = tx {
                        let _ = tx.send(shown);
                    }
                });
                reprisefs::binage::poke();
                if close {
                    self.close_dialog();
                } else {
                    self.redraw_top_dialog();
                }
            }
        }
    }
}
