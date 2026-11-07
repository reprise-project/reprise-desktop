//! The Blue Start menu's Log Off and Turn Off Computer: the Welcome panels, drawn Blue under
//! every interface. Shift turns the panel's Stand By into Hibernate.

use crate::compositor::State;
use crate::dialog::Dialog;

impl State {
    /// The Blue Start menu's Log Off button, after the menu closed.
    pub fn blue_log_off(&mut self) {
        let d = Dialog::log_off(crate::console::leave_offered(self));
        self.open_dialog(d.always_blue());
    }

    /// The Blue Start menu's Turn Off Computer button, after the menu closed.
    pub fn blue_turn_off(&mut self) {
        let offered = crate::console::disconnect_offered(self);
        let d = Dialog::turn_off(&self.config.power, offered).always_blue();
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
