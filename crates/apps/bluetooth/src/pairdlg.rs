//! Bluetooth Pairing Request: the modal box for a pairing the Add wizard is not running. Closing it
//! unanswered refuses the request.

use reprisekit::classic::widgets::{BUTTON_H, BUTTON_W};
use reprisekit::controls::form::{self, Form, Item};
use reprisekit::raster::Rect;
use reprisekit::text::TextRenderer;

use crate::bz::{AgentRequest, Answer};
use crate::passkey;

pub const CLIENT: (i32, i32) = (380, 200);
const BODY: Rect = Rect::new(12, 12, CLIENT.0 - 24, CLIENT.1 - 24 - BUTTON_H - 12);
const OK: &str = "ok";
const CANCEL: &str = "cancel";

pub struct PairDialog {
    pub req: AgentRequest,
    pub form: Form,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    None,
    Redraw,
    Answered(Answer),
    Done,
}

impl PairDialog {
    pub fn new(req: AgentRequest, name: &str, text: &TextRenderer, px: u32) -> PairDialog {
        let mut items = passkey::items(&req.ask, name, BODY, text, px);
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
        };
        d.sync();
        d
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
        let (done, wait) = async_channel::bounded(1);
        let req = AgentRequest {
            device: "/d/1".into(),
            ask,
            done: Some(done),
        };
        let text = TextRenderer::load(&FontConfig::default());
        (
            PairDialog::new(req, "Pad", &text, Theme::default().fonts.size_px),
            wait,
        )
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
