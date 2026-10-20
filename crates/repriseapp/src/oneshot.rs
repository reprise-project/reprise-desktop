//! One-shot dialogs: boxes with no state beyond being open. Their form is rebuilt on every event,
//! so a press resolves its release at once. [`MsgBox`] wraps the common case. The theme they take
//! is the one their forms draw in ([`crate::Ctx::forms`]), whose text size the forms grow with.
//! The form drawn last stays in a [`Shown`], so a cut label tips its whole text.

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
use std::cell::RefCell;

pub enum Step {
    Closed,
    Redraw,
    None,
    Out(form::Event),
}

pub fn event(ev: &Event, text: &TextRenderer, theme: &Theme, build: impl FnOnce() -> Form) -> Step {
    let px = theme.fonts.size_px;
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
            f.drawn_with(&theme.paint());
            let hit = f.hit(*x, *y, text, px);
            f.click(hit);
            Step::Out(f.release())
        }
        _ => Step::None,
    }
}

/// The form a box drew last. The pointer's rest on it times the tip of a cut label
/// ([`Form::tip_tick`]), and the next draw carries that rest over.
#[derive(Default)]
pub struct Shown(RefCell<Option<Form>>);

impl Shown {
    /// Fills the window and draws `f` over it; the runtime draws the label tip it reports.
    pub fn draw(&self, c: &mut Canvas, theme: &Theme, text: &TextRenderer, mut f: Form) {
        let mut last = self.0.borrow_mut();
        if let Some(old) = &*last {
            f.carry_tip(old);
        }
        c.fill_rect(c.bounds(), theme.colors.button_face);
        let p = Paint {
            s: 1,
            ..theme.paint()
        };
        form::draw(c, text, &p, &f, true, theme.fonts.size_px);
        *last = Some(f);
    }

    /// The motion, leave and blink that time the tip; `None` for any other event.
    fn event(&self, ev: &Event) -> Option<Step> {
        let mut last = self.0.borrow_mut();
        let changed = match ev {
            Event::Blink => last.as_mut().is_some_and(Form::tip_tick),
            Event::Pointer(PointerInput::Motion { x, y }) => last
                .as_mut()
                .is_some_and(|f| f.rest_pointer(Some((*x, *y)))),
            Event::Pointer(PointerInput::Leave) => {
                last.as_mut().is_some_and(|f| f.rest_pointer(None))
            }
            _ => return None,
        };
        Some(if changed { Step::Redraw } else { Step::None })
    }
}

/// [`event`] for a message box whose form `fields` builds, drawn through `shown`. The box lives
/// between events, so a key that moves the focus (Tab, the arrows) moves
/// `fields.default` with it: the next form puts the focus and the ring
/// on that button, and Enter and Space press it.
pub fn msgbox_event(
    ev: &Event,
    text: &TextRenderer,
    theme: &Theme,
    fields: &mut Fields,
    shown: &Shown,
) -> Step {
    if let Some(step) = shown.event(ev) {
        return step;
    }
    let (px, u) = (theme.fonts.size_px, theme.text);
    let Event::Key(k) = ev else {
        return event(ev, text, theme, || msgbox::form(fields, text, px, u));
    };
    let Some(key) = dialog_key(k) else {
        return Step::None;
    };
    let mut f = msgbox::form(fields, text, px, u);
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
    shown: Shown,
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
        MsgBox {
            title,
            fields,
            shown: Shown::default(),
        }
    }

    pub fn child(&self, id: WinId, text: &TextRenderer, theme: &Theme) -> Child {
        let u = theme.text;
        Child {
            id,
            title: self.title.clone(),
            size: u.size(msgbox::client(&self.fields, text, theme.fonts.size_px, u)),
            fixed: true,
            modal: true,
        }
    }

    pub fn event(&mut self, ev: &Event, text: &TextRenderer, theme: &Theme) -> Outcome {
        match msgbox_event(ev, text, theme, &mut self.fields, &self.shown) {
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

    pub fn draw(&self, c: &mut Canvas, theme: &Theme, text: &TextRenderer) {
        let f = msgbox::form(&self.fields, text, theme.fonts.size_px, theme.text);
        self.shown.draw(c, theme, text, f);
    }

    pub fn access(&self, tree: &mut Tree, text: &TextRenderer, theme: &Theme) {
        let px = theme.fonts.size_px;
        let f = msgbox::form(&self.fields, text, px, theme.text);
        f.drawn_with(&theme.paint());
        access_form(&f, tree, text, px);
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
    use reprisekit::theme::{FontConfig, TextUnits};

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
                max_w: 0,
            },
        )
    }

    #[test]
    fn a_label_cut_by_a_narrow_window_tips_its_whole_text_after_a_blink() {
        const LONG: &str = "The file name, directory name, or volume label syntax is incorrect.";
        let text = TextRenderer::load(&FontConfig::default());
        let theme = Theme::default();
        let mut b = MsgBox::new(
            "Notepad",
            Fields {
                text: LONG.into(),
                icon: None,
                buttons: vec!["OK".into()],
                default: 0,
                min_text_h: None,
                min_w: 0,
                max_w: 0,
            },
        );
        let draw = |b: &MsgBox, w: u32| {
            let mut c = Canvas::new(w, 120);
            b.draw(&mut c, &theme, &text);
            c.take_tip().map(|tip| tip.text)
        };
        let full = b.child("box", &text, &theme).size.0 as u32;
        let label = msgbox::form(&b.fields, &text, theme.fonts.size_px, theme.text)
            .item("text")
            .unwrap()
            .rect;
        let on = Event::Pointer(PointerInput::Motion {
            x: label.x + 10,
            y: label.y + 5,
        });
        let blink = |b: &mut MsgBox| b.event(&Event::Blink, &text, &theme);

        let leave = Event::Pointer(PointerInput::Leave);
        assert_eq!(draw(&b, full), None);
        assert_eq!(b.event(&on, &text, &theme), Outcome::None);
        assert_eq!(blink(&mut b), Outcome::None, "a label that fits has no tip");
        assert_eq!(b.event(&leave, &text, &theme), Outcome::None);

        assert_eq!(draw(&b, 150), None, "no tip before a blink");
        assert_eq!(b.event(&on, &text, &theme), Outcome::None);
        assert_eq!(
            blink(&mut b),
            Outcome::Redraw,
            "the cut label's tip comes up"
        );
        assert_eq!(
            draw(&b, 150).as_deref(),
            Some(LONG),
            "the next draw reports the whole text"
        );
        assert_eq!(
            b.event(&leave, &text, &theme),
            Outcome::Redraw,
            "leaving takes it down"
        );
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
            max_w: 0,
        };
        // Widths per look are pinned by look::caption_fit's own test.
        let plain = msgbox::client(&short(), &text, px, TextUnits::ONE).0;
        if reprisekit::text::test_tahoma() {
            assert_eq!(plain, 162);
        }
        let mut glass = 0;
        for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
            let theme = reprisekit::interfaces::theme_for(i);
            crate::publish_theme(&theme);
            let b = MsgBox::new("User Accounts", short());
            let w = b.child("box", &text, &Theme::default()).size.0;
            let fit = reprisekit::look::caption_fit(
                &theme,
                "User Accounts",
                reprisekit::frame::Chrome::DIALOG,
            );
            assert_eq!(w, plain.max(fit), "{i:?}: the wider of text and title");
            let ok = msgbox::form(&b.fields, &text, px, TextUnits::ONE)
                .item(msgbox::OK)
                .unwrap()
                .rect;
            assert_eq!(ok.x, (w - ok.width) / 2, "{i:?}: OK centres in the box");
            glass = w;
        }
        crate::CAPTION_FIT.with_borrow_mut(|(_, out)| *out = Some(180));
        let b = MsgBox::new("User Accounts", short());
        assert_eq!(
            b.child("box", &text, &Theme::default()).size.0,
            glass.min(180 - 2 * 8),
            "capped to a 180 px output less the Glass dialog border"
        );
    }

    #[test]
    fn an_arrow_moves_the_focus_and_enter_presses_the_focused_button() {
        let mut b = two_buttons();
        let text = TextRenderer::load(&FontConfig::default());
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
            b.event(&key(keysym::LEFT, Some(left)), &text, &Theme::default()),
            Outcome::Redraw
        );
        assert_eq!(b.fields.default, 0, "Left moves the focus from No to Yes");
        assert_eq!(
            b.event(&key(keysym::RETURN, None), &text, &Theme::default()),
            Outcome::Closed(Some(0)),
            "Enter presses Yes"
        );
    }

    #[test]
    fn enter_is_the_default_button_and_a_click_names_its_own() {
        let mut b = two_buttons();
        let text = TextRenderer::load(&FontConfig::default());
        let px = 11;
        let child = b.child("box", &text, &Theme::default());
        assert!(child.modal && child.fixed);
        assert_eq!(
            child.size,
            msgbox::client(&b.fields, &text, px, TextUnits::ONE)
        );
        assert_eq!(
            b.event(&Event::CloseRequest, &text, &Theme::default()),
            Outcome::Closed(None)
        );
        let f = msgbox::form(&b.fields, &text, px, TextUnits::ONE);
        let yes = f.item(msgbox::button_id(0)).expect("Yes").rect;
        let press = Event::Pointer(PointerInput::Press {
            x: yes.x + 2,
            y: yes.y + 2,
            button: BTN_LEFT,
            mods: Default::default(),
            time: 0,
        });
        assert_eq!(
            b.event(&press, &text, &Theme::default()),
            Outcome::Closed(Some(0))
        );
    }
}
