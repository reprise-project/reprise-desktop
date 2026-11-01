//! The Blue Start menu's Log Off and Turn Off Computer: the Welcome panels, drawn Blue under
//! every interface. Shift turns the panel's Stand By into Hibernate.

use crate::compositor::State;
use crate::dialog::Dialog;

// The Blue Start menu's Log Off and Turn Off Computer buttons call these.
#[allow(dead_code)]
impl State {
    pub fn blue_log_off(&mut self) {
        self.open_dialog(Dialog::log_off().always_blue());
    }

    pub fn blue_turn_off(&mut self) {
        let d = Dialog::turn_off(&self.config.power).always_blue();
        self.open_dialog(d);
    }

    /// Shift went down or up: the top dialog, a Welcome panel, follows it.
    pub(crate) fn dialog_shift(&mut self, on: bool) {
        let outcome = match self.dialogs.views.last_mut() {
            Some(v) => v.dialog.welcome_shift(on),
            None => return,
        };
        self.apply_dialog(outcome);
    }
}
