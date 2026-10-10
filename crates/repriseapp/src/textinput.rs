//! The seat's text-input-v3, which an input method (fcitx5, ibus) types
//! through. The compositor enters it only while an input method runs.

use reprisekit::imeclient::{ImeClient, Request};
use reprisekit::textedit::{EditKey, Preedit, TextSpot};
use smithay_client_toolkit::reexports::protocols::wp::text_input::zv3::client::zwp_text_input_v3::{
    ChangeCause, ContentHint, ContentPurpose, ZwpTextInputV3,
};

use crate::WinId;

pub(crate) struct TextInput {
    ti: ZwpTextInputV3,
    pub entered: Option<WinId>,
    client: ImeClient,
}

impl TextInput {
    pub fn new(ti: ZwpTextInputV3) -> TextInput {
        TextInput {
            ti,
            entered: None,
            client: ImeClient::default(),
        }
    }

    pub fn preedit(&self) -> Option<&Preedit> {
        self.client.preedit.as_ref()
    }

    pub fn sync(&mut self, win: WinId, spot: Option<TextSpot>) {
        if self.entered != Some(win) {
            return;
        }
        for r in self.client.sync(spot) {
            self.request(r);
        }
    }

    fn request(&self, r: Request) {
        match r {
            Request::Enable => self.ti.enable(),
            Request::Disable => self.ti.disable(),
            Request::Surrounding(s) => {
                self.ti
                    .set_surrounding_text(s.text, s.cursor as i32, s.anchor as i32);
            }
            Request::CausedElsewhere => self.ti.set_text_change_cause(ChangeCause::Other),
            Request::ContentType { secret, multiline } => {
                let (hint, purpose) = if secret {
                    (
                        ContentHint::SensitiveData | ContentHint::HiddenText,
                        ContentPurpose::Password,
                    )
                } else if multiline {
                    (ContentHint::Multiline, ContentPurpose::Normal)
                } else {
                    (ContentHint::None, ContentPurpose::Normal)
                };
                self.ti.set_content_type(hint, purpose);
            }
            Request::Cursor(r) => self.ti.set_cursor_rectangle(r.x, r.y, r.width, r.height),
            Request::Commit => self.ti.commit(),
        }
    }

    pub fn enter(&mut self, win: WinId) {
        self.entered = Some(win);
        self.client.reset();
    }

    pub fn leave(&mut self) -> Option<(WinId, bool)> {
        let had = self.client.reset();
        self.entered.take().map(|w| (w, had))
    }

    pub fn preedit_string(&mut self, text: Option<String>, begin: i32, end: i32) {
        self.client.preedit_string(text, begin, end);
    }

    pub fn commit_string(&mut self, text: Option<String>) {
        self.client.commit_string(text);
    }

    pub fn delete_surrounding(&mut self, before: u32, after: u32) {
        self.client.delete_surrounding(before, after);
    }

    pub fn done(&mut self) -> Vec<EditKey> {
        self.client.done()
    }

    pub fn destroy(self) {
        self.ti.destroy();
    }
}
