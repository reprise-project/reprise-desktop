//! Input and the consent prompts: the portal's prompt windows (`Window::consent`) and the location
//! prompt, a consent form ([`Form::set_consent`]). An app's RemoteDesktop session reaches neither.

use std::time::Instant;

use reprisekit::controls::form::Form;
use smithay::utils::{Logical, Point, SERIAL_COUNTER};

use crate::compositor::State;
use crate::input::Hit;
use crate::inputproto::InputSource;
use crate::keyroute::KeyRoute;

impl State {
    fn app_input(&self) -> bool {
        self.inputproto.source == InputSource::App
    }

    fn focused_consent(&self) -> bool {
        (self.focused.and_then(|w| self.win(w))).is_some_and(|w| w.consent)
    }

    /// The consent form of the dialog with the keyboard.
    fn consent_dialog_mut(&mut self) -> Option<&mut Form> {
        let id = self.active_dialog_id()?;
        let v = self.dialogs.views.iter_mut().find(|v| v.id == id)?;
        v.dialog
            .form_mut()?
            .open_page_mut()
            .filter(|f| f.is_consent())
    }

    /// Whether `route` takes the text being typed, an app's RemoteDesktop session's, to a consent
    /// prompt.
    pub fn consent_refuses_text(&mut self, route: &KeyRoute) -> bool {
        self.app_input()
            && match route {
                KeyRoute::Client => self.focused_consent(),
                KeyRoute::Dialog => self.consent_dialog_mut().is_some(),
                _ => false,
            }
    }

    /// Whether key `code`, pressed or released and routed to `route`, stays away from a consent
    /// prompt: an app's RemoteDesktop session's key, or a press the location prompt's arm delay
    /// refuses. A release always reaches the prompt's gate.
    pub fn consent_refuses_key(&mut self, route: &KeyRoute, code: u32, pressed: bool) -> bool {
        if !pressed {
            if let Some(f) = self.consent_dialog_mut() {
                f.consent_key_up(code);
            }
            return false;
        }
        if self.consent_refuses_text(route) {
            return true;
        }
        *route == KeyRoute::Dialog
            && (self.consent_dialog_mut()).is_some_and(|f| !f.consent_key(code, Instant::now()))
    }

    /// Whether a press at `pos` stays away from a consent prompt: an app's RemoteDesktop session's
    /// press on one, or one the location prompt's arm delay refuses.
    pub fn consent_refuses_press(&self, pos: Point<f64, Logical>) -> bool {
        let consent = |w| self.win(w).is_some_and(|x| x.consent);
        match self.scene_hit(pos) {
            Hit::Surface {
                window: Some(w), ..
            }
            | Hit::Frame { window: w, .. } => self.app_input() && consent(w),
            Hit::Dialog { index, .. } => {
                let view = self.dialogs.views.get(index);
                let form = view
                    .and_then(|v| v.dialog.form()?.open_page())
                    .filter(|f| f.is_consent());
                // A press that gives the prompt the keyboard is a focus gain, refused as one.
                let gains = view.map(|v| v.id) != self.active_dialog_id();
                form.is_some_and(|f| self.app_input() || gains || !f.consent_press(Instant::now()))
            }
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

    /// The screen just woke (saver, blank or outputs off): a consent prompt with the keyboard
    /// starts its arm delay again, so the input that woke it and any typed blind after answer
    /// nothing. A portal prompt hears it as a fresh `wl_keyboard.enter`.
    pub fn rearm_consent(&mut self) {
        if let Some(f) = self.consent_dialog_mut() {
            f.consent_focused(Instant::now());
        }
        let Some(keyboard) = self.seat.get_keyboard() else {
            return;
        };
        let Some(focus) = keyboard.current_focus() else {
            return;
        };
        let consent = (self.window_for_surface(&focus))
            .and_then(|w| self.win(w))
            .is_some_and(|w| w.consent);
        if consent {
            self.flush_motion();
            keyboard.set_focus(self, None, SERIAL_COUNTER.next_serial());
            keyboard.set_focus(self, Some(focus), SERIAL_COUNTER.next_serial());
        }
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
    use smithay::backend::input::{ButtonState, InputTime};
    use smithay::utils::Point;

    use crate::harness::{Harness, TestClient, Toplevel};
    use crate::inputproto::InputSource;

    const KEY_ESC: u32 = 1;
    const KEY_A: u32 = 30;
    const LEFT: u32 = 0x110;

    /// A consent prompt mapped with the keyboard, and a plain window of another client.
    fn prompt_and_window(h: &mut Harness) -> ((TestClient, Toplevel), (TestClient, Toplevel)) {
        h.output("o", 800, 600);
        let mut c = h.connect();
        let app = h.map_toplevel(&mut c, "app", 300, 150);
        let mut p = h.connect_as(crate::mapfocus::TEST_PROMPT_PID);
        let prompt = h.map_toplevel(&mut p, crate::mapfocus::CONSENT_APP_ID, 300, 150);
        assert_eq!(
            h.state.focused,
            Some(prompt.id),
            "the prompt has the keyboard"
        );
        ((p, prompt), (c, app))
    }

    fn tap(h: &mut Harness, source: InputSource, code: u32) {
        for down in [true, false] {
            h.from(source, |s| {
                s.handle_key((code + 8).into(), down, InputTime::now())
            });
        }
    }

    fn click(h: &mut Harness, source: InputSource, t: &Toplevel) {
        let r = h.state.win(t.id).unwrap().frame_rect();
        let at = Point::from((f64::from(r.x + r.width / 2), f64::from(r.y + r.height / 2)));
        h.from(source, |s| {
            s.pointer_to(at, InputTime::now());
            s.settle_pointer();
        });
        for down in [ButtonState::Pressed, ButtonState::Released] {
            h.from(source, |s| s.handle_button(LEFT, down, InputTime::now()));
        }
    }

    #[test]
    fn an_apps_remote_keys_never_reach_a_consent_prompt() {
        let mut h = Harness::new();
        let ((mut p, _), (mut c, app)) = prompt_and_window(&mut h);
        tap(&mut h, InputSource::App, KEY_A);
        h.roundtrip(&mut p);
        assert_eq!(p.side.keys, [], "an app's session types nothing");
        tap(&mut h, InputSource::Remote, KEY_A);
        h.roundtrip(&mut p);
        assert_eq!(
            p.side.keys,
            [(KEY_A, true), (KEY_A, false)],
            "the remote desktop server's viewer types"
        );
        h.state.focus_window(app.id);
        tap(&mut h, InputSource::App, KEY_A);
        h.roundtrip(&mut c);
        assert_eq!(
            c.side.keys,
            [(KEY_A, true), (KEY_A, false)],
            "an app's session still types into a plain window"
        );
    }

    #[test]
    fn an_apps_remote_clicks_never_reach_a_consent_prompt() {
        let mut h = Harness::new();
        let ((mut p, prompt), _) = prompt_and_window(&mut h);
        click(&mut h, InputSource::App, &prompt);
        h.roundtrip(&mut p);
        assert_eq!(p.side.buttons, [], "an app's session clicks nothing");
        click(&mut h, InputSource::Remote, &prompt);
        h.roundtrip(&mut p);
        assert_eq!(
            p.side.buttons,
            [(LEFT, true), (LEFT, false)],
            "the remote desktop server's viewer clicks"
        );
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

    #[test]
    fn a_consent_prompt_waits_again_when_the_screen_saver_stops() {
        let mut h = Harness::new();
        let ((mut p, _), _) = prompt_and_window(&mut h);
        h.roundtrip(&mut p);
        let enters = p.side.enters;
        crate::saverview::start(&mut h.state, reprisekit::saver::Kind::Blank, false);
        h.from(InputSource::Physical, crate::idle::notify_activity);
        tap(&mut h, InputSource::Physical, KEY_A);
        h.roundtrip(&mut p);
        assert!(!crate::saverview::running(&h.state));
        assert_eq!(p.side.enters, enters + 1, "the keyboard enters it again");
        let heard = p.side.keys.iter().position(|k| *k == (KEY_A, true));
        assert!(heard.is_some(), "the waking key comes after the enter");
    }

    #[test]
    fn the_location_prompt_waits_again_when_the_screen_saver_stops() {
        let mut h = Harness::new();
        location_prompt(&mut h);
        arm(&mut h);
        crate::saverview::start(&mut h.state, reprisekit::saver::Kind::Blank, false);
        h.from(InputSource::Physical, crate::idle::notify_activity);
        tap(&mut h, InputSource::Physical, KEY_ESC);
        assert_eq!(
            h.state.dialogs.views.len(),
            1,
            "the Escape that woke the screen does nothing"
        );
    }

    #[test]
    fn a_click_that_gives_the_location_prompt_the_keyboard_answers_nothing() {
        let mut h = Harness::new();
        location_prompt(&mut h);
        arm(&mut h);
        let v = &h.state.dialogs.views[0];
        let at = Point::from((f64::from(v.x + 40), f64::from(v.y + 40)));
        assert!(matches!(
            h.state.scene_hit(at),
            crate::input::Hit::Dialog { .. }
        ));
        assert!(
            !h.state.consent_refuses_press(at),
            "armed with the keyboard"
        );
        let mut c = h.connect();
        let app = h.map_toplevel(&mut c, "app", 100, 100);
        h.state.focus_window(app.id);
        h.dispatch();
        assert_eq!(h.state.active_dialog_id(), None);
        assert!(
            h.state.consent_refuses_press(at),
            "the press that brings the keyboard back"
        );
    }

    #[test]
    fn a_key_the_slow_keys_aid_holds_keeps_its_apps_source() {
        let mut h = Harness::new();
        let ((mut p, _), _) = prompt_and_window(&mut h);
        let a = &mut h.state.config.accessibility;
        a.slow_keys = true;
        a.slow_keys_delay = 1;
        h.state.aids_configured();
        let accepted = |h: &mut Harness, source| {
            h.from(source, |s| {
                s.handle_key((KEY_A + 8).into(), true, InputTime::now())
            });
            std::thread::sleep(std::time::Duration::from_millis(20));
            h.dispatch();
            h.from(source, |s| {
                s.handle_key((KEY_A + 8).into(), false, InputTime::now())
            });
        };
        accepted(&mut h, InputSource::App);
        h.roundtrip(&mut p);
        assert!(
            !p.side.keys.contains(&(KEY_A, true)),
            "an app's held key never reaches the prompt"
        );
        accepted(&mut h, InputSource::Remote);
        h.roundtrip(&mut p);
        assert!(
            p.side.keys.contains(&(KEY_A, true)),
            "the viewer's held key does"
        );
    }

    #[test]
    fn an_apps_remote_keys_never_answer_the_location_prompt() {
        let mut h = Harness::new();
        location_prompt(&mut h);
        arm(&mut h);
        tap(&mut h, InputSource::App, KEY_ESC);
        assert_eq!(
            h.state.dialogs.views.len(),
            1,
            "an app's Escape does nothing"
        );
        tap(&mut h, InputSource::Remote, KEY_ESC);
        assert!(
            h.state.dialogs.views.is_empty(),
            "the viewer's Escape closes it"
        );
    }
}
