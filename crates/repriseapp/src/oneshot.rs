//! One-shot dialogs: boxes with no state beyond being open. Their form is rebuilt on every event,
//! so a press resolves its release at once. [`MsgBox`] wraps the common case.

use crate::keys::dialog_key;
use crate::{BTN_LEFT, Child, Event, PointerInput, WinId};
use reprisekit::access::{Tree, accesskit::NodeId};
use reprisekit::controls::form::{self, Form};
use reprisekit::controls::msgbox::{self, Fields};
use reprisekit::controls::propsheet::PropSheet;
use reprisekit::look::Paint;
use reprisekit::raster::Canvas;
use reprisekit::text::TextRenderer;
use reprisekit::theme::Theme;

pub enum Step {
    Closed,
    Redraw,
    None,
    Out(form::Event),
}

pub fn event(ev: &Event, text: &TextRenderer, px: u32, build: impl FnOnce() -> Form) -> Step {
    match ev {
        Event::CloseRequest => Step::Closed,
        Event::Blink => Step::Redraw,
        Event::Key(k) => {
            let Some(key) = dialog_key(k) else {
                return Step::None;
            };
            let mut f = build();
            Step::Out(f.key(key))
        }
        Event::Pointer(PointerInput::Press {
            x,
            y,
            button: BTN_LEFT,
            ..
        }) => {
            let mut f = build();
            let hit = f.hit(*x, *y, text, px);
            f.click(hit);
            Step::Out(f.release())
        }
        _ => Step::None,
    }
}

/// between events, so a key that moves the focus (Tab, the arrows) moves
/// `fields.default` with it: the next form puts the focus and the ring
/// on that button, and Enter and Space press it.
pub fn msgbox_event(ev: &Event, text: &TextRenderer, px: u32, fields: &mut Fields) -> Step {
    let Event::Key(k) = ev else {
        return event(ev, text, px, || msgbox::form(fields, text, px));
    };
    let Some(key) = dialog_key(k) else {
        return Step::None;
    };
    let mut f = msgbox::form(fields, text, px);
    let out = f.key(key);
    if let Some(i) = f.focus() {
        let id = f.items[i].id;
        if let Some(b) = (0..fields.buttons.len()).find(|&b| msgbox::button_id(b) == id) {
            fields.default = b;
        }
    }
    Step::Out(out)
}

pub struct MsgBox {
    pub title: String,
    pub fields: Fields,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    None,
    Redraw,
    Closed(Option<usize>),
}

impl MsgBox {
    /// A box at least as wide as its caption needs for `title` ([`crate::caption_fit`]).
    pub fn new(title: impl Into<String>, mut fields: Fields) -> MsgBox {
        let title = title.into();
        fields.min_w = fields.min_w.max(crate::caption_fit(&title));
        MsgBox { title, fields }
    }

    pub fn child(&self, id: WinId, text: &TextRenderer, px: u32) -> Child {
        Child {
            id,
            title: self.title.clone(),
            size: msgbox::client(&self.fields, text, px),
            fixed: true,
            modal: true,
        }
    }

    pub fn event(&mut self, ev: &Event, text: &TextRenderer, px: u32) -> Outcome {
        match msgbox_event(ev, text, px, &mut self.fields) {
            Step::Closed => Outcome::Closed(None),
            Step::Redraw => Outcome::Redraw,
            Step::None => Outcome::None,
            Step::Out(form::Event::Cancel) => Outcome::Closed(None),
            Step::Out(form::Event::Default) => Outcome::Closed(Some(self.fields.default)),
            Step::Out(form::Event::Pressed(id)) => Outcome::Closed(
                (0..self.fields.buttons.len()).find(|&i| msgbox::button_id(i) == id),
            ),
            Step::Out(form::Event::None) => Outcome::None,
            Step::Out(_) => Outcome::Redraw,
        }
    }

    pub fn draw(&self, c: &mut Canvas, theme: &Theme, text: &TextRenderer, px: u32) {
        c.fill_rect(c.bounds(), theme.colors.button_face);
        let f = msgbox::form(&self.fields, text, px);
        form::draw(
            c,
            text,
            &Paint {
                s: 1,
                ..theme.paint()
            },
            &f,
            true,
            px,
        );
    }

    pub fn access(&self, tree: &mut Tree, text: &TextRenderer, px: u32) {
        access_form(&msgbox::form(&self.fields, text, px), tree, text, px);
    }
}

pub fn access_form(form: &Form, tree: &mut Tree, text: &TextRenderer, px: u32) {
    if let Some(f) = form.access(tree, reprisekit::access::ROOT, text, px) {
        tree.set_focus(f);
    }
}

pub fn access_sheet(sheet: &PropSheet, tree: &mut Tree, text: &TextRenderer, px: u32) -> NodeId {
    let (panel, focus) = sheet.access(tree, reprisekit::access::ROOT, text, px);
    if let Some(f) = focus {
        tree.set_focus(f);
    }
    panel
}

#[cfg(test)]
mod tests {
    use super::*;
    use reprisekit::textedit::{EditKey, keysym};
    use reprisekit::theme::FontConfig;

    fn two_buttons() -> MsgBox {
        MsgBox::new(
            "T",
            Fields {
                text: "Replace?".into(),
                icon: None,
                buttons: vec!["Yes".into(), "No".into()],
                default: 1,
                min_text_h: None,
                min_w: 0,
            },
        )
    }

    #[test]
    fn a_short_box_widens_for_its_title_up_to_the_output() {
        use reprisekit::interface::Interface;
        let text = TextRenderer::load(&FontConfig::default());
        let px = 11;
        let short = || Fields {
            text: "Access denied.".into(),
            icon: None,
            buttons: vec!["OK".into()],
            default: 0,
            min_text_h: None,
            min_w: 0,
        };
        // Widths per look are pinned by look::caption_fit's own test.
        let plain = msgbox::client(&short(), &text, px).0;
        if text.has_face("Tahoma") {
            assert_eq!(plain, 162);
        }
        let mut glass = 0;
        for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
            let theme = reprisekit::interfaces::theme_for(i);
            crate::publish_theme(&theme);
            let b = MsgBox::new("User Accounts", short());
            let w = b.child("box", &text, px).size.0;
            let fit = reprisekit::look::caption_fit(
                &theme,
                "User Accounts",
                reprisekit::frame::Chrome::DIALOG,
            );
            assert_eq!(w, plain.max(fit), "{i:?}: the wider of text and title");
            let ok = msgbox::form(&b.fields, &text, px)
                .item(msgbox::OK)
                .unwrap()
                .rect;
            assert_eq!(ok.x, (w - ok.width) / 2, "{i:?}: OK centres in the box");
            glass = w;
        }
        crate::CAPTION_FIT.with_borrow_mut(|(_, out)| *out = Some(180));
        let b = MsgBox::new("User Accounts", short());
        assert_eq!(
            b.child("box", &text, px).size.0,
            glass.min(180 - 2 * 8),
            "capped to a 180 px output less the Glass dialog border"
        );
    }

    #[test]
    fn an_arrow_moves_the_focus_and_enter_presses_the_focused_button() {
        let mut b = two_buttons();
        let text = TextRenderer::load(&FontConfig::default());
        let px = 11;
        let key = |sym, edit| {
            Event::Key(crate::KeyInput {
                sym,
                utf8: None,
                mods: Default::default(),
                edit,
            })
        };
        let left = EditKey::Left {
            select: false,
            word: false,
        };
        assert_eq!(
            b.event(&key(keysym::LEFT, Some(left)), &text, px),
            Outcome::Redraw
        );
        assert_eq!(b.fields.default, 0, "Left moves the focus from No to Yes");
        assert_eq!(
            b.event(&key(keysym::RETURN, None), &text, px),
            Outcome::Closed(Some(0)),
            "Enter presses Yes"
        );
    }

    #[test]
    fn enter_is_the_default_button_and_a_click_names_its_own() {
        let mut b = two_buttons();
        let text = TextRenderer::load(&FontConfig::default());
        let px = 11;
        let child = b.child("box", &text, px);
        assert!(child.modal && child.fixed);
        assert_eq!(child.size, msgbox::client(&b.fields, &text, px));
        assert_eq!(
            b.event(&Event::CloseRequest, &text, px),
            Outcome::Closed(None)
        );
        let f = msgbox::form(&b.fields, &text, px);
        let yes = f.item(msgbox::button_id(0)).expect("Yes").rect;
        let press = Event::Pointer(PointerInput::Press {
            x: yes.x + 2,
            y: yes.y + 2,
            button: BTN_LEFT,
            mods: Default::default(),
            time: 0,
        });
        assert_eq!(b.event(&press, &text, px), Outcome::Closed(Some(0)));
    }
}
