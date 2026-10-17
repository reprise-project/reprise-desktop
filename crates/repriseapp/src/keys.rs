//! Keyboard state a host keeps that is not a control's (Alt alone activating
//! the menu bar), and the one reading of a key press every form-driven window
//! shares.

use reprisekit::controls::form::{self, Form};
use reprisekit::controls::hotkey;
use reprisekit::controls::propsheet::{self, PropSheet};
use reprisekit::textedit::EditKey;

use crate::{BTN_LEFT, Ctx, Event, KeyInput, PointerInput, Reply};

/// Alt-release menu-bar activation: Alt down arms it, Alt up fires it, anything
/// in between cancels it.
///
/// Hosts call [`AltArm::interrupt`] on a pointer press, any child-window
/// event, focus loss and any non-Alt key release (a compositor grab like
/// Alt+Space eats the press but not the release); otherwise a stale arm
/// opens the menu bar, which then swallows letters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AltArm {
    armed: bool,
}

impl AltArm {
    pub fn press_alt(&mut self) {
        self.armed = true;
    }

    /// Something else took the keyboard: another key, a click, a popup, a child
    /// window, focus loss.
    pub fn interrupt(&mut self) {
        self.armed = false;
    }

    /// An Alt key came up. `true` when it is still the bare Alt press this one
    /// answers, which activates the menu bar.
    pub fn release_alt(&mut self) -> bool {
        let fire = self.armed;
        self.armed = false;
        fire
    }
}

/// The dialog key a press stands for: Tab and Shift+Tab, Enter, Escape, Up and
/// Down (Alt+Down or F4 drops a combo), Page Up and Down, otherwise the editing
/// key. Alt with a printable character is the access key whatever has the focus.
pub fn dialog_key(k: &KeyInput) -> Option<form::Key> {
    if let Some(special) = form::special_key(k.sym, k.mods.shift, k.mods.alt) {
        return Some(special);
    }
    if k.mods.alt {
        return match k.edit.as_ref() {
            Some(EditKey::Insert(s)) => {
                let mut chars = s.chars();
                match (chars.next(), chars.next()) {
                    (Some(c), None) if !c.is_whitespace() => Some(form::Key::Access(c)),
                    _ => None,
                }
            }
            _ => None,
        };
    }
    k.edit.clone().map(form::Key::Edit)
}

/// [`dialog_key`] for a form, read with Ctrl through [`form::ctrl_key`]:
/// Ctrl+Enter breaks a line in a memo, Ctrl+Home and Ctrl+End move its caret.
fn form_key(k: &KeyInput) -> Option<form::Key> {
    let key = dialog_key(k)?;
    Some(if k.mods.ctrl {
        form::ctrl_key(key)
    } else {
        key
    })
}

/// The clipboard half of an event for a window of forms, given its focused edit
/// field: Ctrl+C and Ctrl+X put the selection on the clipboard (a secret
/// field's never), read before the key reaches the form so a cut still has it;
/// Ctrl+V asks for the clipboard text (`Event::Pasted`). Empty otherwise.
pub fn form_clipboard(focused: Option<form::FocusedEdit>, ev: &Event) -> Reply {
    let Event::Key(KeyInput { edit: Some(k), .. }) = ev else {
        return Reply::none();
    };
    match k {
        EditKey::Copy | EditKey::Cut => Reply {
            copy: focused
                .and_then(|e| e.copy_text())
                .filter(|s| !s.is_empty()),
            ..Reply::none()
        },
        EditKey::Paste if focused.is_some() => Reply {
            paste: true,
            ..Reply::none()
        },
        _ => Reply::none(),
    }
}

/// One event fed to a property sheet: a key through [`form_key`] with Ctrl+Tab
/// and Ctrl+Page Down turning the page ([`propsheet::page_step`]), a left press
/// hit-tested and clicked, motion tracked, a release released, Leave clearing
/// what the pointer lit, a drop on a field ([`crate::fielddrop`]) heard as the
/// open page's, a tick stepping a held scrollbar arrow. `None` is an event the host answers
/// itself. First the sheet fits the window ([`PropSheet::fit`], [`PropSheet::fit_tabs`]): a
/// window cut short or narrow scrolls the page, and one cut narrow wraps the tabs to it. While an arrow is held the host keeps ticking ([`Ctx::want_tick`]).
pub fn sheet_input(sheet: &mut PropSheet, ev: &Event, ctx: &Ctx) -> Option<propsheet::Event> {
    let out = sheet_event(sheet, ev, ctx);
    if sheet.ticking() {
        ctx.want_tick.set(true);
    }
    out
}

fn sheet_event(sheet: &mut PropSheet, ev: &Event, ctx: &Ctx) -> Option<propsheet::Event> {
    let u = ctx.form_units();
    let (ox, oy) = sheet.origin();
    sheet.fit((u.within(ctx.size.0 - ox), u.within(ctx.size.1 - oy)));
    let px = sheet.open().units().px(ctx.base_px());
    sheet.fit_tabs(&ctx.text, px);
    Some(match ev {
        Event::Tick if sheet.ticking() => sheet.tick(std::time::Instant::now()),
        Event::Key(k) => {
            let ctrl_tab = propsheet::page_step(k.sym, k.mods.ctrl, k.mods.shift);
            sheet.key(form_key(k)?, ctrl_tab)
        }
        Event::Pointer(PointerInput::Press {
            x,
            y,
            button: BTN_LEFT,
            ..
        }) => {
            let hit = sheet.hit(*x, *y, &ctx.text, px);
            sheet.click(hit)
        }
        Event::Pointer(PointerInput::Motion { x, y }) => {
            let hit = sheet.hit(*x, *y, &ctx.text, px);
            sheet.motion(*x, *y, hit)
        }
        Event::Pointer(PointerInput::Scroll { x, y, rows }) => {
            let hit = sheet.hit(*x, *y, &ctx.text, px);
            sheet.wheel(hit, *rows)
        }
        Event::Pointer(PointerInput::Release {
            button: BTN_LEFT, ..
        }) => sheet.release(),
        Event::Pointer(PointerInput::Leave) => sheet.leave(),
        Event::FieldDropped(e) => sheet.dropped(*e),
        _ => return None,
    })
}

/// A press or release for a focused hot-key field, which reads raw keys ahead
/// of `dialog_key`; `None` when no field has the focus or it passes the key on.
pub fn hotkey_input(form: &mut Form, k: &KeyInput, down: bool) -> Option<form::Event> {
    let name = repriseconf::keybind::key_name(k.sym).unwrap_or_default();
    let held = hotkey::Mods {
        logo: false,
        ctrl: k.mods.ctrl,
        alt: k.mods.alt,
        shift: k.mods.shift,
    };
    form.hotkey_key(k.sym, &name, held, down)
}

/// [`sheet_input`] for a one-form window: a dialog. A focused hot-key field gets
/// the keys first ([`hotkey_input`]), and is blurred when the window loses the
/// keyboard or the compositor takes its shortcuts back.
pub fn form_input(form: &mut Form, ev: &Event, ctx: &Ctx) -> Option<form::Event> {
    let px = form.units().px(ctx.base_px());
    Some(match ev {
        Event::Focus(false) | Event::ShortcutsInactive => {
            form.blur_hotkey().then_some(form::Event::Redraw)?
        }
        Event::Key(k) => match hotkey_input(form, k, true) {
            Some(e) => e,
            None => form.key(form_key(k)?),
        },
        Event::KeyUp(k) => hotkey_input(form, k, false)?,
        Event::Pointer(PointerInput::Press {
            x,
            y,
            button: BTN_LEFT,
            ..
        }) => {
            let hit = form.hit(*x, *y, &ctx.text, px);
            form.click(hit)
        }
        Event::Pointer(PointerInput::Motion { x, y }) => {
            let hit = form.hit(*x, *y, &ctx.text, px);
            form.motion(*x, *y, hit)
        }
        Event::Pointer(PointerInput::Scroll { x, y, rows }) => {
            let hit = form.hit(*x, *y, &ctx.text, px);
            form.wheel(hit, *rows)
        }
        Event::Pointer(PointerInput::Release {
            button: BTN_LEFT, ..
        }) => form.release(),
        Event::Pointer(PointerInput::Leave) => form.leave(),
        Event::FieldDropped(e) => *e,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_alt_fires_and_an_interrupted_one_does_not() {
        let mut a = AltArm::default();
        a.press_alt();
        assert!(a.release_alt(), "Alt down, Alt up: the bar activates");
        // The release consumes the arm: a second one fires nothing.
        assert!(!a.release_alt());

        // Alt+D: the letter interrupts, so the Alt release means nothing.
        a.press_alt();
        a.interrupt();
        assert!(!a.release_alt());

        // A release with nothing armed at all.
        assert!(!AltArm::default().release_alt());
    }

    /// A held page arrow asks the host for ticks, and a tick steps it once it repeats.
    #[test]
    fn a_held_page_arrow_keeps_the_window_ticking() {
        use reprisekit::controls::form::Item;
        use reprisekit::controls::propsheet::{self, PropSheet};
        use reprisekit::raster::Rect;
        let ctx = Ctx {
            theme: reprisekit::theme::Theme::default(),
            text: std::rc::Rc::new(reprisekit::text::TextRenderer::load(
                &reprisekit::theme::FontConfig::default(),
            )),
            icons: reprisekit::icontheme::IconCache::new(reprisekit::icontheme::IconTheme::load(
                "test",
                &[],
            )),
            caret_on: true,
            size: (398, 300),
            maximized: false,
            origin: (0, 0),
            wake: crate::Waker::noop(),
            heads: Default::default(),
            want_tick: Default::default(),
            at_pick: false,
            text_size: 100,
        };
        let page = Form::new(vec![
            Item::label("l", Rect::new(20, 50, 100, 13), "Label"),
            Item::label("low", Rect::new(20, 370, 100, 13), "Low"),
        ]);
        let mut sheet = PropSheet::new(&["General"], vec![page], (398, 424));
        let _ = sheet_input(&mut sheet, &Event::Pointer(PointerInput::Leave), &ctx);
        assert!(!ctx.want_tick.take());
        let bar = sheet.layout(&ctx.text, 11).bar.expect("the page scrolls");
        let (x, y) = (bar.x + 2, bar.bottom() - 3);
        let press = Event::Pointer(PointerInput::Press {
            x,
            y,
            button: BTN_LEFT,
            time: 0,
            mods: crate::Mods::default(),
        });
        let _ = sheet_input(
            &mut sheet,
            &Event::Pointer(PointerInput::Motion { x, y }),
            &ctx,
        );
        assert_eq!(
            sheet_input(&mut sheet, &press, &ctx),
            Some(propsheet::Event::Redraw)
        );
        assert!(ctx.want_tick.take(), "the press asks for a tick");
        let stepped = sheet.page_offset().1;
        let _ = sheet_input(&mut sheet, &Event::Tick, &ctx);
        assert!(ctx.want_tick.take(), "and each tick for the next");
        std::thread::sleep(reprisekit::controls::scrollbar::REPEAT_AFTER);
        assert_eq!(
            sheet_input(&mut sheet, &Event::Tick, &ctx),
            Some(propsheet::Event::Redraw)
        );
        assert!(
            sheet.page_offset().1 < stepped,
            "the held arrow stepped again"
        );
        assert!(ctx.want_tick.take());
        let release = Event::Pointer(PointerInput::Release {
            x,
            y,
            button: BTN_LEFT,
            mods: crate::Mods::default(),
        });
        let _ = sheet_input(&mut sheet, &release, &ctx);
        assert!(!ctx.want_tick.take(), "the release stops it");
    }

    /// The window losing the keyboard blurs a focused hot-key field: the
    /// modifier held when it went is forgotten, since its release goes
    /// elsewhere, and the next key records without it.
    #[test]
    fn losing_the_keyboard_blurs_a_focused_hot_key_field() {
        use reprisekit::controls::form::{Control, Item};
        use reprisekit::controls::hotkey::HotKey;
        use reprisekit::raster::Rect;
        let ctx = Ctx {
            theme: reprisekit::theme::Theme::default(),
            text: std::rc::Rc::new(reprisekit::text::TextRenderer::load(
                &reprisekit::theme::FontConfig::default(),
            )),
            icons: reprisekit::icontheme::IconCache::new(reprisekit::icontheme::IconTheme::load(
                "test",
                &[],
            )),
            caret_on: true,
            size: (200, 100),
            maximized: false,
            origin: (0, 0),
            wake: crate::Waker::noop(),
            heads: Default::default(),
            want_tick: Default::default(),
            at_pick: false,
            text_size: 100,
        };
        let mut form = Form::new(vec![Item::new(
            "keys",
            Rect::new(10, 10, 150, 20),
            Control::HotKey(HotKey::new(None)),
        )]);
        form.focus_id("keys");
        let key = |sym, ctrl| KeyInput {
            sym,
            utf8: None,
            mods: crate::Mods {
                ctrl,
                ..Default::default()
            },
            edit: None,
        };
        const CONTROL_L: u32 = 0xffe3;
        let _ = form_input(&mut form, &Event::Key(key(CONTROL_L, false)), &ctx);
        let shown = |form: &Form| match &form.items[0].control {
            Control::HotKey(h) => h.shown(true),
            _ => unreachable!(),
        };
        assert_eq!(shown(&form), "Ctrl + ");
        assert_eq!(
            form_input(&mut form, &Event::Focus(false), &ctx),
            Some(form::Event::Redraw)
        );
        assert_eq!(shown(&form), "None");
        let _ = form_input(&mut form, &Event::Key(key(u32::from(b't'), false)), &ctx);
        // A bare letter records with Super, as the field reads one.
        assert_eq!(shown(&form), "Super + T", "no Ctrl left over");
    }

    /// Super+Shift+Escape hands the shortcuts back mid-chord and the
    /// compositor keeps the Escape: the field forgets Super and Shift, so
    /// letting them go records nothing.
    #[test]
    fn the_shortcuts_going_back_leaves_the_chord_alone() {
        use reprisekit::controls::form::{Control, Item};
        use reprisekit::controls::hotkey::HotKey;
        use reprisekit::raster::Rect;
        let ctx = Ctx {
            theme: reprisekit::theme::Theme::default(),
            text: std::rc::Rc::new(reprisekit::text::TextRenderer::load(
                &reprisekit::theme::FontConfig::default(),
            )),
            icons: reprisekit::icontheme::IconCache::new(reprisekit::icontheme::IconTheme::load(
                "test",
                &[],
            )),
            caret_on: true,
            size: (200, 100),
            maximized: false,
            origin: (0, 0),
            wake: crate::Waker::noop(),
            heads: Default::default(),
            want_tick: Default::default(),
            at_pick: false,
            text_size: 100,
        };
        let mut form = Form::new(vec![Item::new(
            "keys",
            Rect::new(10, 10, 150, 20),
            Control::HotKey(HotKey::from_keys("Ctrl+t")),
        )]);
        form.focus_id("keys");
        let key = |sym| KeyInput {
            sym,
            utf8: None,
            mods: crate::Mods::default(),
            edit: None,
        };
        const SUPER_L: u32 = 0xffeb;
        const SHIFT_L: u32 = 0xffe1;
        for sym in [SUPER_L, SHIFT_L] {
            let _ = form_input(&mut form, &Event::Key(key(sym)), &ctx);
        }
        assert_eq!(
            form_input(&mut form, &Event::ShortcutsInactive, &ctx),
            Some(form::Event::Redraw)
        );
        for sym in [SHIFT_L, SUPER_L] {
            let _ = form_input(&mut form, &Event::KeyUp(key(sym)), &ctx);
        }
        let Control::HotKey(h) = &form.items[0].control else {
            unreachable!()
        };
        assert_eq!(h.keys(), "Ctrl+t");
    }

    /// A Blue form window's pointer: a motion over its button lights it,
    /// the pointer leaving the window clears it, and a held button dragged
    /// off asks for a redraw. The sheet does the same through its page.
    #[test]
    fn form_and_sheet_input_light_the_control_and_clear_it_on_leave() {
        use reprisekit::controls::form::{Item, Light};
        use reprisekit::pointer::Interaction;
        use reprisekit::raster::{Canvas, Rect};
        let ctx = Ctx {
            theme: reprisekit::interfaces::theme_for(reprisekit::interface::Interface::Blue),
            text: std::rc::Rc::new(reprisekit::text::TextRenderer::load(
                &reprisekit::theme::FontConfig::default(),
            )),
            icons: reprisekit::icontheme::IconCache::new(reprisekit::icontheme::IconTheme::load(
                "test",
                &[],
            )),
            caret_on: true,
            size: (200, 100),
            maximized: false,
            origin: (0, 0),
            wake: crate::Waker::noop(),
            heads: Default::default(),
            want_tick: Default::default(),
            at_pick: false,
            text_size: 100,
        };
        let px = ctx.theme.fonts.size_px;
        let mut form = Form::new(vec![Item::button(
            "ok",
            Rect::new(10, 10, 75, 23),
            "OK",
            true,
            true,
        )]);
        let mut c = Canvas::new(200, 100);
        form::draw(&mut c, &ctx.text, &ctx.theme.paint(), &form, false, px);
        let motion = |x, y| Event::Pointer(PointerInput::Motion { x, y });
        let ev = form_input(&mut form, &motion(20, 20), &ctx);
        assert_eq!(ev, Some(form::Event::Redraw));
        assert_eq!(form.lit(0, Light::Body), Interaction::Hot);
        let leave = Event::Pointer(PointerInput::Leave);
        assert_eq!(
            form_input(&mut form, &leave, &ctx),
            Some(form::Event::Redraw)
        );
        assert_eq!(form.lit(0, Light::Body), Interaction::Idle);
        let press = Event::Pointer(PointerInput::Press {
            x: 20,
            y: 20,
            button: crate::BTN_LEFT,
            time: 0,
            mods: crate::Mods::default(),
        });
        form_input(&mut form, &press, &ctx);
        let ev = form_input(&mut form, &motion(150, 80), &ctx);
        assert_eq!(ev, Some(form::Event::Redraw), "dragged off");
        assert_eq!(form.lit(0, Light::Body), Interaction::Idle);

        let page = Form::new(vec![Item::check("check", 20, 50, "Check")]);
        let mut sheet = PropSheet::new(&["General"], vec![page], (398, 424));
        let mut c = Canvas::new(398, 424);
        reprisekit::controls::propsheet::draw(
            &mut c,
            &ctx.text,
            &ctx.theme.paint(),
            &sheet,
            false,
            px,
        );
        let ev = sheet_input(&mut sheet, &motion(25, 55), &ctx);
        assert_eq!(ev, Some(propsheet::Event::Redraw));
        assert_eq!(
            sheet_input(&mut sheet, &leave, &ctx),
            Some(propsheet::Event::Redraw)
        );
        assert_eq!(sheet.open().lit(0, Light::Body), Interaction::Idle);
    }

    fn press(sym: u32, utf8: &str, alt: bool) -> KeyInput {
        let utf8 = (!utf8.is_empty()).then(|| utf8.to_string());
        let edit = reprisekit::textedit::key_for(sym, false, false, utf8.as_deref());
        KeyInput {
            sym,
            utf8,
            mods: crate::Mods {
                ctrl: false,
                shift: false,
                alt,
            },
            edit,
        }
    }

    #[test]
    fn alt_with_a_letter_is_the_access_key_not_an_insert() {
        assert_eq!(
            dialog_key(&press(0x62, "b", false)),
            Some(form::Key::Edit(EditKey::Insert("b".into())))
        );
        assert_eq!(
            dialog_key(&press(0x62, "b", true)),
            Some(form::Key::Access('b'))
        );
        // Alt+Down still drops a combo; a bare Alt is nothing.
        assert_eq!(
            dialog_key(&press(reprisekit::textedit::keysym::DOWN, "", true)),
            Some(form::Key::Drop)
        );
        assert_eq!(dialog_key(&press(0xffe9, "", true)), None);
    }

    /// Ctrl+C and Ctrl+X copy the focused field's selection, a secret
    /// field's never; Ctrl+V asks for the clipboard only for a field.
    #[test]
    fn form_clipboard_copies_the_selection_and_asks_to_paste() {
        use reprisekit::textedit::TextEdit;
        let ctrl = |ch: u8| {
            Event::Key(KeyInput {
                sym: u32::from(ch),
                utf8: None,
                mods: crate::Mods {
                    ctrl: true,
                    ..Default::default()
                },
                edit: reprisekit::textedit::key_for(u32::from(ch), true, false, None),
            })
        };
        let mut e = TextEdit::new("abc", false);
        e.key(EditKey::SelectAll);
        let field = Some(form::FocusedEdit::Line(&e));
        assert_eq!(
            form_clipboard(field, &ctrl(b'c')).copy.as_deref(),
            Some("abc")
        );
        assert_eq!(
            form_clipboard(field, &ctrl(b'x')).copy.as_deref(),
            Some("abc")
        );
        assert!(form_clipboard(field, &ctrl(b'v')).paste);
        assert_eq!(form_clipboard(field, &ctrl(b'a')), Reply::none());
        assert!(!form_clipboard(None, &ctrl(b'v')).paste, "no field");
        let mut pw = TextEdit::secret();
        pw.key(EditKey::Insert("pw".into()));
        pw.key(EditKey::SelectAll);
        let secret = Some(form::FocusedEdit::Line(&pw));
        assert_eq!(form_clipboard(secret, &ctrl(b'c')).copy, None, "a secret");
    }

    /// Ctrl+Home and Ctrl+End take a memo's caret to its first and last
    /// character, and the next draw scrolls the view there.
    #[test]
    fn ctrl_home_and_ctrl_end_scroll_a_memo_to_its_ends() {
        use reprisekit::controls::form::{Control, Item};
        use reprisekit::controls::multiedit::Pos;
        use reprisekit::raster::{Canvas, Rect};
        use reprisekit::textedit::keysym::{END, HOME};
        let ctx = Ctx {
            theme: reprisekit::theme::Theme::default(),
            text: std::rc::Rc::new(reprisekit::text::TextRenderer::load(
                &reprisekit::theme::FontConfig::default(),
            )),
            icons: reprisekit::icontheme::IconCache::new(reprisekit::icontheme::IconTheme::load(
                "test",
                &[],
            )),
            caret_on: true,
            size: (200, 100),
            maximized: false,
            origin: (0, 0),
            wake: crate::Waker::noop(),
            heads: Default::default(),
            want_tick: Default::default(),
            at_pick: false,
            text_size: 100,
        };
        let px = ctx.theme.fonts.size_px;
        let mut form = Form::new(vec![Item::memo("m", Rect::new(0, 0, 200, 80), "", true)]);
        form.focus_id("m");
        let lines: Vec<String> = (0..20).map(|k| format!("line {k}")).collect();
        form.paste(lines.join("\n"));
        let draw = |form: &Form| {
            let mut c = Canvas::new(200, 80);
            form::draw(&mut c, &ctx.text, &ctx.theme.paint(), form, false, px);
            match &form.items[0].control {
                Control::Memo(m) => (m.edit.caret(), m.view.get().top),
                _ => unreachable!(),
            }
        };
        let (end, bottom) = draw(&form);
        assert_eq!(end, Pos { line: 19, col: 7 }, "the caret ends the paste");
        assert!(bottom > 0, "the view follows the paste down");
        let ctrl = |sym| {
            Event::Key(KeyInput {
                sym,
                utf8: None,
                mods: crate::Mods {
                    ctrl: true,
                    ..Default::default()
                },
                edit: reprisekit::textedit::key_for(sym, true, false, None),
            })
        };
        form_input(&mut form, &ctrl(HOME), &ctx);
        assert_eq!(draw(&form), (Pos::default(), 0), "Ctrl+Home: the top");
        form_input(&mut form, &ctrl(END), &ctx);
        assert_eq!(draw(&form), (end, bottom), "Ctrl+End: the bottom");
    }
}
