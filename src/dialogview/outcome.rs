use crate::compositor::State;
use crate::dialog::Outcome;
use reprisefs::program::Confirm;

impl State {
    pub(crate) fn apply_dialog(&mut self, outcome: Outcome) {
        match outcome {
            Outcome::Nothing => {}
            Outcome::Redraw => self.redraw_top_dialog(),
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
            Outcome::GreeterPower(a) => {
                self.close_dialog();
                crate::greeterview::power(self, a);
            }
            Outcome::RestartInto { keys } => crate::restartinto::open_menu(self, keys),
            Outcome::PowerAnyway(again) => {
                self.close_dialog();
                crate::sessionend::anyway(self, again);
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
                if let Some(from) = self.dialogs.views.last().map(|v| v.id) {
                    self.run_line_activated(&cmd, from);
                }
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
            Outcome::UndoErase(entry) => {
                self.close_dialog();
                self.undo_icons_erasing(entry);
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
        }
    }
}
