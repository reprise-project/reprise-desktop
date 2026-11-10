//! Bluetooth Pairing Request: the modal box for a pairing the Add wizard is not running. Closing it
//! unanswered refuses the request. A request the user did not start from Settings starts on its
//! refusing answer and trusts the device only through its "trust" box.

use reprisekit::classic::widgets::{BUTTON_H, BUTTON_W};
use reprisekit::controls::form::{self, Form, Item};
use reprisekit::raster::Rect;
use reprisekit::text::TextRenderer;
use reprisekit::theme::Theme;

use crate::bz::{AgentRequest, Answer};
use crate::passkey;

pub const CLIENT: (i32, i32) = (380, 200);
const BODY: Rect = Rect::new(12, 12, CLIENT.0 - 24, CLIENT.1 - 24 - BUTTON_H - 12);
const OK: &str = "ok";
const CANCEL: &str = "cancel";
const TRUST: &str = "trust";

pub struct PairDialog {
    pub req: AgentRequest,
    pub form: Form,
    /// The user started this pairing from Settings.
    started: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    None,
    Redraw,
    Answered(Answer),
    Done,
}

impl PairDialog {
    pub fn new(
        req: AgentRequest,
        name: &str,
        started: bool,
        text: &TextRenderer,
        forms: &Theme,
    ) -> PairDialog {
        let mut items = passkey::items(&req.ask, name, BODY, text, forms, started);
        if !started && req.done.is_some() {
            let y = items
                .iter()
                .map(|i| i.rect.bottom())
                .max()
                .unwrap_or(BODY.y)
                + 10;
            items.push(Item::check(
                TRUST,
                BODY.x,
                y,
                "Connect to this device auto&matically",
            ));
        }
        let y = CLIENT.1 - 12 - BUTTON_H;
        let waits = req.done.is_some();
        items.push(Item::button(
            OK,
            Rect::new(CLIENT.0 - 12 - 2 * BUTTON_W - 6, y, BUTTON_W, BUTTON_H),
            "OK",
            true,
            true,
        ));
        items.push(Item::button(
            CANCEL,
            Rect::new(CLIENT.0 - 12 - BUTTON_W, y, BUTTON_W, BUTTON_H),
            "Cancel",
            waits,
            false,
        ));
        let mut d = PairDialog {
            req,
            form: Form::new(items),
            started,
        };
        d.sync();
        d
    }

    /// Whether accepting should trust the device, so it may connect its services on its own.
    pub fn trusts(&self) -> bool {
        self.started || self.form.checked(TRUST) == Some(true)
    }

    fn ok_live(&self) -> bool {
        self.req.done.is_none() || passkey::ready(&self.req.ask, &self.form)
    }

    fn sync(&mut self) {
        let live = self.ok_live();
        self.form.set_enabled(OK, live);
    }

    pub fn event(&mut self, ev: form::Event) -> Outcome {
        match ev {
            form::Event::Pressed(OK) | form::Event::Default => {
                if !self.ok_live() {
                    return Outcome::None;
                }
                match passkey::answer(&self.req.ask, &self.form) {
                    Some(a) if self.req.done.is_some() => {
                        self.req.answer(a.clone());
                        Outcome::Answered(a)
                    }
                    _ => Outcome::Done,
                }
            }
            form::Event::Pressed(CANCEL) | form::Event::Cancel => Outcome::Done,
            form::Event::Changed(_) => {
                self.sync();
                Outcome::Redraw
            }
            form::Event::None => Outcome::None,
            _ => Outcome::Redraw,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bz::{Answer, Ask};
    use reprisekit::text::TextRenderer;
    use reprisekit::theme::{FontConfig, Theme};

    fn dialog(ask: Ask) -> (PairDialog, async_channel::Receiver<Answer>) {
        asked(ask, true)
    }

    /// A box for `ask`; `started` when the user started the pairing from Settings.
    fn asked(ask: Ask, started: bool) -> (PairDialog, async_channel::Receiver<Answer>) {
        let (done, wait) = async_channel::bounded(1);
        let req = AgentRequest {
            device: "/d/1".into(),
            ask,
            done: Some(done),
        };
        let text = TextRenderer::load(&FontConfig::default());
        (
            PairDialog::new(req, "Pad", started, &text, &Theme::default()),
            wait,
        )
    }

    /// The box fits its text for each request under every interface and text size.
    #[test]
    fn the_box_fits_its_text() {
        let over = reprisekit::interfaces::form_overflows(|_, forms, text| {
            [
                Ask::PinCode,
                Ask::Passkey,
                Ask::ShowPin("0000".into()),
                Ask::ShowPasskey(123_456),
                Ask::Confirm(123_456),
                Ask::Authorize,
            ]
            .into_iter()
            .map(|ask| {
                let name = format!("{ask:?}");
                let req = AgentRequest {
                    device: "/d/1".into(),
                    ask,
                    done: None,
                };
                (
                    name,
                    PairDialog::new(req, "Pad", false, text, forms).form.clone(),
                )
            })
            .collect()
        });
        assert!(over.is_empty(), "{}", over.join("\n"));
    }

    #[test]
    fn ok_answers_and_cancel_drops() {
        let (mut d, wait) = dialog(Ask::Confirm(7));
        assert_eq!(
            d.event(form::Event::Pressed(OK)),
            Outcome::Answered(Answer::Accept)
        );
        assert_eq!(wait.try_recv(), Ok(Answer::Accept));
        let (mut d, wait) = dialog(Ask::Authorize);
        assert_eq!(d.event(form::Event::Cancel), Outcome::Done);
        drop(d);
        assert!(wait.try_recv().is_err(), "closed unanswered");
    }

    #[test]
    fn a_request_nobody_started_refuses_by_default() {
        for ask in [Ask::Confirm(7), Ask::Authorize] {
            let (mut d, wait) = asked(ask.clone(), false);
            assert_eq!(
                d.event(form::Event::Default),
                Outcome::Answered(Answer::Reject),
                "{ask:?}"
            );
            assert_eq!(wait.try_recv(), Ok(Answer::Reject));
        }
    }

    #[test]
    fn only_a_started_pairing_or_the_ticked_box_trusts_the_device() {
        let (d, _wait) = asked(Ask::Confirm(7), true);
        assert!(d.trusts());
        assert!(d.form.item(TRUST).is_none(), "no box to tick");
        let (mut d, _wait) = asked(Ask::Confirm(7), false);
        assert_eq!(d.form.checked(TRUST), Some(false), "off by default");
        assert!(!d.trusts());
        d.form.set_check(TRUST, true);
        assert!(d.trusts());
    }

    #[test]
    fn ok_waits_for_a_valid_passkey() {
        let (mut d, _wait) = dialog(Ask::Passkey);
        assert_eq!(
            d.event(form::Event::Default),
            Outcome::None,
            "nothing typed"
        );
        crate::props::set_edit(&mut d.form, passkey::CODE, "123456");
        d.event(form::Event::Changed(passkey::CODE));
        assert_eq!(
            d.event(form::Event::Default),
            Outcome::Answered(Answer::Passkey(123456))
        );
    }
}
