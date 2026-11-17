//! One-shot dialogs: boxes with no state beyond being open. Their form is rebuilt on every event,
//! so a press resolves its release at once. [`MsgBox`] wraps the common case. The theme they take
//! is the one their forms draw in ([`crate::Ctx::forms`]), whose text size the forms grow with.
//! The form drawn last stays in a [`Shown`], so a cut label tips its whole text.

use crate::asks::{Answer, Asks};
use crate::keys::dialog_key;
use crate::{BTN_LEFT, Child, Event, PointerInput, Reply, WinId};
use reprisekit::access::{Tree, accesskit::NodeId};
use reprisekit::controls::dlgtemplate::{self, Template};
use reprisekit::controls::form::{self, Form};
use reprisekit::controls::msgbox::{self, Fields};
use reprisekit::controls::propsheet::PropSheet;
use reprisekit::look::Paint;
use reprisekit::raster::Canvas;
use reprisekit::text::TextRenderer;
use reprisekit::theme::Theme;
use std::cell::RefCell;
use std::time::Instant;

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
        crate::keys::note_tip(last.as_ref().is_some_and(Form::tip_waiting));
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
    let (px, u) = (theme.fonts.size_px, theme.text);
    box_event(ev, text, theme, fields, shown, |f| {
        msgbox::form(f, text, px, u)
    })
}

/// [`msgbox_event`] for a box whose form `build` makes from `fields`.
fn box_event(
    ev: &Event,
    text: &TextRenderer,
    theme: &Theme,
    fields: &mut Fields,
    shown: &Shown,
    build: impl Fn(&Fields) -> Form,
) -> Step {
    if let Some(step) = shown.event(ev) {
        return step;
    }
    let Event::Key(k) = ev else {
        return event(ev, text, theme, || build(fields));
    };
    let Some(key) = dialog_key(k) else {
        return Step::None;
    };
    let mut f = build(fields);
    let out = f.key(key);
    // On a consent box the default stays the refusing button.
    if !f.is_consent()
        && let Some(i) = f.focus()
    {
        let id = f.items[i].id;
        if let Some(b) = (0..fields.buttons.len()).find(|&b| button_is(b, id)) {
            fields.default = b;
        }
    }
    Step::Out(out)
}

/// Whether `id` is button `i`'s: a message box's and a template's buttons share their ids.
fn button_is(i: usize, id: &str) -> bool {
    dlgtemplate::BUTTON_IDS.get(i) == Some(&id)
}

/// A box's client size at 100% and its form, for text at `theme`'s size: a message box, or one
/// laid out on `template`.
fn built(
    f: &Fields,
    template: Option<Template>,
    text: &TextRenderer,
    theme: &Theme,
) -> ((i32, i32), Form) {
    let (px, u) = (theme.fonts.size_px, theme.text);
    let Some(template) = template else {
        return (msgbox::client(f, text, px, u), msgbox::form(f, text, px, u));
    };
    let lay = dlgtemplate::layout(
        template,
        text,
        px,
        &dlgtemplate::Fields {
            text: &f.text,
            icon: true,
            field: false,
            buttons: &f.buttons,
            text_h: f.min_text_h,
        },
        u,
    )
    .client();
    let form = dlgtemplate::form(&lay, f.icon.clone(), &f.buttons, f.default);
    (lay.size, form)
}

pub struct MsgBox {
    pub title: String,
    pub fields: Fields,
    /// Laid out on this shell template ([`dlgtemplate`]) instead of as a message box: the
    /// delete prompts, which wear the template Explorer's do.
    template: Option<Template>,
    shown: Shown,
    /// A consent box's wait ([`MsgBox::consent`]), on a form of its own: the box's form is built
    /// again for every event.
    gate: Option<Form>,
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
            template: None,
            shown: Shown::default(),
            gate: None,
        }
    }

    /// A box laid out on `template`, its text box at least `fields.min_text_h` tall. Its
    /// buttons are at most four.
    pub fn on_template(title: impl Into<String>, template: Template, fields: Fields) -> MsgBox {
        MsgBox {
            title: title.into(),
            fields,
            template: Some(template),
            shown: Shown::default(),
            gate: None,
        }
    }

    pub fn child(&self, id: WinId, text: &TextRenderer, theme: &Theme) -> Child {
        Child {
            id,
            title: self.title.clone(),
            size: theme
                .text
                .size(built(&self.fields, self.template, text, theme).0),
            fixed: true,
            modal: true,
        }
    }

    /// Make this a consent box ([`Form::set_consent`]), for a prompt that grants something: a
    /// letter needs Alt, Enter, Space and Tab leave the default on the refusing button, and keys
    /// and presses count only [`form::CONSENT_ARM`] after the box gets the keyboard.
    pub fn consent(mut self) -> MsgBox {
        let mut gate = Form::new(Vec::new());
        gate.set_consent();
        self.gate = Some(gate);
        self
    }

    /// The box's window got the keyboard at `now`, or the box took the place of another in a
    /// window that has it: a consent box starts its wait again.
    pub fn consent_focused(&mut self, now: Instant) {
        if let Some(g) = &mut self.gate {
            g.consent_focused(now);
        }
    }

    pub fn event(&mut self, ev: &Event, text: &TextRenderer, theme: &Theme) -> Outcome {
        let now = Instant::now();
        if let Some(g) = &mut self.gate {
            match ev {
                Event::Focus(true) => {
                    g.consent_focused(now);
                    return Outcome::None;
                }
                Event::Key(k) if !g.consent_key(k.sym, now) => return Outcome::None,
                Event::KeyUp(k) => g.consent_key_up(k.sym),
                Event::Pointer(PointerInput::Press {
                    button: BTN_LEFT, ..
                }) if !g.consent_press(now) => return Outcome::None,
                _ => {}
            }
        }
        let (template, consent) = (self.template, self.gate.is_some());
        let step = box_event(ev, text, theme, &mut self.fields, &self.shown, |f| {
            let mut form = built(f, template, text, theme).1;
            if consent {
                form.set_consent();
            }
            form
        });
        match step {
            Step::Closed => Outcome::Closed(None),
            Step::Redraw => Outcome::Redraw,
            Step::None => Outcome::None,
            Step::Out(form::Event::Cancel) => Outcome::Closed(None),
            Step::Out(form::Event::Default) => Outcome::Closed(Some(self.fields.default)),
            Step::Out(form::Event::Pressed(id)) => {
                Outcome::Closed((0..self.fields.buttons.len()).find(|&i| button_is(i, id)))
            }
            Step::Out(form::Event::None) => Outcome::None,
            Step::Out(_) => Outcome::Redraw,
        }
    }

    pub fn draw(&self, c: &mut Canvas, theme: &Theme, text: &TextRenderer) {
        let f = built(&self.fields, self.template, text, theme).1;
        self.shown.draw(c, theme, text, f);
    }

    pub fn access(&self, tree: &mut Tree, text: &TextRenderer, theme: &Theme) {
        let f = built(&self.fields, self.template, text, theme).1;
        f.drawn_with(&theme.paint());
        access_form(&f, tree, text, theme.fonts.size_px);
    }
}

/// One-OK error boxes in a modal child window: a program's word that something failed. Errors
/// wait their turn ([`crate::asks`]); the program routes the windows [`ErrorBox::owns`] here.
/// Each box carries the window [`ErrorBox::show_over`] named.
pub struct ErrorBox(Asks<Option<WinId>>);

impl Default for ErrorBox {
    fn default() -> ErrorBox {
        ErrorBox::new(WinId::named("error"))
    }
}

impl ErrorBox {
    /// Boxes in `win` and the id [`Asks::new`] derives from it.
    pub fn new(win: WinId) -> ErrorBox {
        ErrorBox(Asks::new(win))
    }

    /// The boxes cover `parent` instead of [`crate::MAIN`].
    pub fn over(self, parent: WinId) -> ErrorBox {
        ErrorBox(self.0.over(parent))
    }

    /// Shows `text` under `title`, with `icon` beside it, once the boxes before it are answered.
    /// An error equal in title and text to one up or waiting is shown once.
    pub fn show(&mut self, title: &str, text: String, icon: Option<Canvas>) {
        self.raise(None, title, text, icon);
    }

    /// [`ErrorBox::show`] over `parent`, such as the sheet the error is about, in place of the
    /// boxes' own parent ([`Asks::ask_over`]). [`ErrorBox::withdraw`] takes it when `parent` closes.
    pub fn show_over(&mut self, parent: WinId, title: &str, text: String, icon: Option<Canvas>) {
        self.raise(Some(parent), title, text, icon);
    }

    fn raise(&mut self, over: Option<WinId>, title: &str, text: String, icon: Option<Canvas>) {
        if (self.0.iter()).any(|(b, _)| b.title == title && b.fields.text == text) {
            return;
        }
        let fields = Fields {
            text,
            icon,
            buttons: vec![reprisekit::kit_tr!("ok")],
            default: 0,
            min_text_h: None,
            min_w: 0,
            max_w: crate::dialog_max_w(),
        };
        let b = MsgBox::new(title, fields);
        match over {
            Some(parent) => self.0.ask_over(parent, b, over),
            None => self.0.ask(b, None),
        }
    }

    /// Takes every box shown over a window `gone` picks, up or waiting: that window closed.
    pub fn withdraw(&mut self, gone: impl Fn(WinId) -> bool) {
        self.0.withdraw(|over| over.is_some_and(&gone));
    }

    pub fn is_shown(&self) -> bool {
        !self.0.is_empty()
    }

    /// The box up.
    pub fn shown(&self) -> Option<&MsgBox> {
        self.0.up().map(|(b, _)| b)
    }

    /// The box up, to give it an icon that loaded after it opened.
    pub fn shown_mut(&mut self) -> Option<&mut MsgBox> {
        self.0.up_mut()
    }

    /// Every box, the one up first.
    pub fn boxes(&self) -> impl Iterator<Item = &MsgBox> {
        self.0.iter().map(|(b, _)| b)
    }

    /// Takes the box up down, and the next is up.
    pub fn close(&mut self) {
        self.0.close();
    }

    /// Whether `win` is one of the boxes' windows ([`Asks::owns`]).
    pub fn owns(&self, win: WinId) -> bool {
        self.0.owns(win)
    }

    /// The window of the box up.
    pub fn win(&self) -> Option<WinId> {
        self.0.win()
    }

    /// The window a box window covers ([`Asks::parent`]).
    pub fn parent(&self, win: WinId) -> Option<WinId> {
        self.0.parent(win)
    }

    /// The box's window while one is up.
    pub fn child(&self, text: &TextRenderer, theme: &Theme) -> Option<Child> {
        self.0.child(text, theme)
    }

    /// An event for window `win`; OK and closing take the box down.
    pub fn event(&mut self, win: WinId, ev: &Event, text: &TextRenderer, theme: &Theme) -> Reply {
        match self.0.event(win, ev, text, theme) {
            Answer::Closed(..) => Reply::redraw_all(),
            Answer::Redraw => Reply::redraw(),
            Answer::None => Reply::none(),
        }
    }

    pub fn draw(&self, c: &mut Canvas, theme: &Theme, text: &TextRenderer) {
        self.0.draw(c, theme, text);
    }

    pub fn access(&self, tree: &mut Tree, text: &TextRenderer, theme: &Theme) {
        self.0.access(tree, text, theme);
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
        let full = b.child(WinId::named("box"), &text, &theme).size.0 as u32;
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
        crate::keys::take_tip_waits();
        assert_eq!(b.event(&on, &text, &theme), Outcome::None);
        assert!(
            crate::keys::take_tip_waits(),
            "the window keeps taking blinks"
        );
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

    fn short() -> Fields {
        Fields {
            text: "Access denied.".into(),
            icon: None,
            buttons: vec!["OK".into()],
            default: 0,
            min_text_h: None,
            min_w: 0,
            max_w: 0,
        }
    }

    #[test]
    fn a_short_boxs_text_is_162_px_wide() {
        reprisekit::pins::fonts();
        let text = TextRenderer::load(&FontConfig::default());
        assert_eq!(msgbox::client(&short(), &text, 11, TextUnits::ONE).0, 162);
    }

    #[test]
    fn a_short_box_widens_for_its_title_up_to_the_output() {
        use reprisekit::interface::Interface;
        let text = TextRenderer::load(&FontConfig::default());
        let px = 11;
        // Widths per look are pinned by look::caption_fit's own test.
        let plain = msgbox::client(&short(), &text, px, TextUnits::ONE).0;
        let mut glass = 0;
        for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
            let theme = reprisekit::interfaces::theme_for(i);
            crate::look::publish_theme(&theme);
            let b = MsgBox::new("User Accounts", short());
            let w = b
                .child(WinId::named("box"), &text, &Theme::default())
                .size
                .0;
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
        crate::look::CAPTION_FIT.with_borrow_mut(|(_, out)| *out = Some(180));
        let b = MsgBox::new("User Accounts", short());
        assert_eq!(
            b.child(WinId::named("box"), &text, &Theme::default())
                .size
                .0,
            glass.min(180 - 2 * 8),
            "capped to a 180 px output less the Glass dialog border"
        );
    }

    #[test]
    fn an_error_box_shows_one_ok_and_closes_on_it() {
        let text = TextRenderer::load(&FontConfig::default());
        let theme = Theme::default();
        let mut e = ErrorBox::default();
        assert_eq!(e.child(&text, &theme), None);
        e.show("Sounds", "The device is gone.".into(), None);
        assert_eq!(
            e.shown().map(|b| b.fields.text.as_str()),
            Some("The device is gone.")
        );
        assert_eq!(
            e.child(&text, &theme).map(|c| c.id),
            Some(WinId::named("error"))
        );
        assert_eq!(
            e.shown().map(|b| b.fields.buttons.clone()),
            Some(vec!["OK".into()])
        );
        let win = e.win().unwrap();
        assert!(e.event(win, &enter(), &text, &theme).redraw_all);
        assert!(!e.is_shown());
    }

    fn enter() -> Event {
        Event::Key(crate::KeyInput {
            sym: keysym::RETURN,
            utf8: None,
            mods: Default::default(),
            edit: None,
        })
    }

    #[test]
    fn a_second_error_is_up_after_the_first_ones_ok_with_its_own_text() {
        let text = TextRenderer::load(&FontConfig::default());
        let theme = Theme::default();
        let mut e = ErrorBox::default();
        e.show("Image Viewer", "a.png is read-only.".into(), None);
        e.show("Image Viewer", "b.png is read-only.".into(), None);
        let text_up = |e: &ErrorBox| e.shown().map(|b| b.fields.text.clone());
        assert_eq!(text_up(&e).as_deref(), Some("a.png is read-only."));
        let first = e.win().unwrap();
        assert!(e.event(first, &enter(), &text, &theme).redraw_all);
        assert_eq!(text_up(&e).as_deref(), Some("b.png is read-only."));
        let second = e.child(&text, &theme).expect("the second box").id;
        assert_ne!(second, first, "a window of its own");
        assert!(e.owns(second));
        e.event(second, &enter(), &text, &theme);
        assert!(!e.is_shown());
    }

    #[test]
    fn an_error_shown_over_a_window_covers_it_and_leaves_when_it_closes() {
        let (sheet, other) = (crate::window_id(1), crate::window_id(2));
        let mut e = ErrorBox::default();
        e.show_over(sheet, "Printers", "about the sheet".into(), None);
        e.show("Printers", "about the program".into(), None);
        e.show_over(other, "Printers", "about the other".into(), None);
        assert_eq!(e.parent(e.win().unwrap()), Some(sheet));
        e.withdraw(|w| w == sheet);
        let left: Vec<_> = e.boxes().map(|b| b.fields.text.as_str()).collect();
        assert_eq!(left, ["about the program", "about the other"]);
        assert_eq!(e.parent(e.win().unwrap()), Some(crate::MAIN));
    }

    #[test]
    fn an_error_equal_to_one_up_or_waiting_is_shown_once() {
        let mut e = ErrorBox::default();
        e.show("Sounds", "The device is gone.".into(), None);
        e.show("Sounds", "The mixer is gone.".into(), None);
        e.show("Sounds", "The device is gone.".into(), None);
        e.show("Sounds", "The mixer is gone.".into(), None);
        e.show("Volume", "The device is gone.".into(), None);
        let shown: Vec<_> = e
            .boxes()
            .map(|b| (b.title.as_str(), b.fields.text.as_str()))
            .collect();
        assert_eq!(
            shown,
            [
                ("Sounds", "The device is gone."),
                ("Sounds", "The mixer is gone."),
                ("Volume", "The device is gone.")
            ]
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
    fn a_consent_box_waits_for_the_keyboard_and_grants_only_through_alt() {
        let boxed = || {
            let mut b = two_buttons();
            b.fields.buttons = vec!["&Yes".into(), "&No".into()];
            b.consent()
        };
        let mut b = boxed();
        let text = TextRenderer::load(&FontConfig::default());
        let theme = Theme::default();
        let key = |sym, ch: &str, alt| {
            Event::Key(crate::KeyInput {
                sym,
                utf8: None,
                mods: crate::Mods {
                    alt,
                    ..Default::default()
                },
                edit: (!ch.is_empty()).then(|| EditKey::Insert(ch.into())),
            })
        };
        let up = |ev: &Event| {
            let Event::Key(k) = ev else { unreachable!() };
            Event::KeyUp(k.clone())
        };
        let y = key(0x79, "y", false);
        let enter = key(keysym::RETURN, "", false);
        let tab = key(keysym::TAB, "", false);
        assert_eq!(b.event(&Event::Focus(true), &text, &theme), Outcome::None);
        assert_eq!(b.event(&enter, &text, &theme), Outcome::None, "too soon");
        b.event(&up(&enter), &text, &theme);
        b.consent_focused(Instant::now() - form::CONSENT_ARM);
        assert_eq!(b.event(&y, &text, &theme), Outcome::None, "a bare y");
        b.event(&tab, &text, &theme);
        assert_eq!(b.fields.default, 1, "Tab leaves the default on No");
        assert_eq!(
            b.event(&enter, &text, &theme),
            Outcome::Closed(Some(1)),
            "Enter presses No"
        );
        let mut b = boxed();
        b.consent_focused(Instant::now() - form::CONSENT_ARM);
        assert_eq!(
            b.event(&key(0x79, "y", true), &text, &theme),
            Outcome::Closed(Some(0)),
            "Alt+Y presses Yes"
        );
    }

    #[test]
    fn enter_is_the_default_button_and_a_click_names_its_own() {
        let mut b = two_buttons();
        let text = TextRenderer::load(&FontConfig::default());
        let px = 11;
        let child = b.child(WinId::named("box"), &text, &Theme::default());
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
