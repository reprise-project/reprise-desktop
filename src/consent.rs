//! Input and the location prompt, a consent form ([`Form::set_consent`]): its keys and presses
//! wait out the form's arm delay each time it gets the keyboard.

use std::time::Instant;

use reprisekit::controls::form::Form;
use smithay::utils::{Logical, Point};

use crate::compositor::State;
use crate::input::Hit;
use crate::keyroute::KeyRoute;

impl State {
    /// The consent form of the dialog with the keyboard.
    fn consent_dialog_mut(&mut self) -> Option<&mut Form> {
        let id = self.active_dialog_id()?;
        let v = self.dialogs.views.iter_mut().find(|v| v.id == id)?;
        v.dialog
            .form_mut()?
            .open_page_mut()
            .filter(|f| f.is_consent())
    }

    /// Whether key `code`, pressed or released and routed to `route`, stays away from a consent
    /// prompt: a press the location prompt's arm delay refuses. A release always reaches the
    /// prompt's gate.
    pub fn consent_refuses_key(&mut self, route: &KeyRoute, code: u32, pressed: bool) -> bool {
        if !pressed {
            if let Some(f) = self.consent_dialog_mut() {
                f.consent_key_up(code);
            }
            return false;
        }
        *route == KeyRoute::Dialog
            && (self.consent_dialog_mut()).is_some_and(|f| !f.consent_key(code, Instant::now()))
    }

    /// Whether a press at `pos` stays away from a consent prompt: one the location prompt's arm
    /// delay refuses.
    pub fn consent_refuses_press(&self, pos: Point<f64, Logical>) -> bool {
        match self.scene_hit(pos) {
            Hit::Dialog { index, .. } => (self.dialogs.views.get(index))
                .and_then(|v| v.dialog.form()?.open_page())
                .is_some_and(|f| f.is_consent() && !f.consent_press(Instant::now())),
            _ => false,
        }
    }

    /// [`State::consent_refuses_press`] for button `button` going down or up at `pos`; a refused
    /// press's release stays away too.
    pub fn consent_refuses_button(
        &mut self,
        button: u32,
        pressed: bool,
        pos: Point<f64, Logical>,
    ) -> bool {
        if !pressed {
            let held = &mut self.inputproto.consent_held;
            let refused = held.contains(&button);
            held.retain(|&b| b != button);
            return refused;
        }
        let refused = self.consent_refuses_press(pos);
        if refused {
            self.inputproto.consent_held.push(button);
        }
        refused
    }

    /// Start the arm delay of the consent form the keyboard just moved to; runs after every
    /// change of the dialog with the keyboard.
    pub fn follow_consent_keyboard(&mut self) {
        let holder = self.active_dialog_id();
        if holder == self.dialogs.keyboard_holder {
            return;
        }
        self.dialogs.keyboard_holder = holder;
        if let Some(f) = self.consent_dialog_mut() {
            f.consent_focused(Instant::now());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use reprisekit::controls::form::CONSENT_ARM;
    use smithay::backend::input::InputTime;

    use crate::harness::Harness;
    use crate::inputproto::InputSource;

    const KEY_ESC: u32 = 1;

    fn tap(h: &mut Harness, source: InputSource, code: u32) {
        for down in [true, false] {
            h.from(source, |s| {
                s.handle_key((code + 8).into(), down, InputTime::now())
            });
        }
    }

    fn location_prompt(h: &mut Harness) {
        h.output("o", 800, 600);
        let fields = reprisekit::controls::locationprompt::Fields {
            program: "Maps".into(),
            icon: None,
        };
        let t = &h.state.config.theme;
        let (px, u) = (t.fonts.size_px, t.text);
        let d = crate::dialog::Dialog::location_prompt(
            &fields,
            "org.example.Maps".into(),
            &h.state.text,
            px,
            u,
        );
        h.state.open_dialog(d);
        h.dispatch();
    }

    fn arm(h: &mut Harness) {
        let f = h
            .state
            .consent_dialog_mut()
            .expect("the prompt has the keyboard");
        f.consent_focused(Instant::now() - CONSENT_ARM);
    }

    #[test]
    fn the_location_prompt_ignores_keys_right_after_it_opens() {
        let mut h = Harness::new();
        location_prompt(&mut h);
        tap(&mut h, InputSource::Physical, KEY_ESC);
        assert_eq!(
            h.state.dialogs.views.len(),
            1,
            "Escape at once does nothing"
        );
        arm(&mut h);
        tap(&mut h, InputSource::Physical, KEY_ESC);
        assert!(h.state.dialogs.views.is_empty(), "Escape later closes it");
    }
}
