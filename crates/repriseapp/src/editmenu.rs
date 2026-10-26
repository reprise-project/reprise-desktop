//! The edit menu over every single-line text field a program draws: a right press in the
//! focused field (or one the press focused), the Menu key or Shift+F10 open
//! [`reprisekit::controls::editmenu`]'s menu, and a pick runs as its shortcut key, so the field's
//! own Ctrl+Z, Ctrl+X, Ctrl+C, Ctrl+V, Delete and Ctrl+A handling does the work. Fields are
//! found by the [`TextSpot`] the focused one reports as it draws; multi-line ones keep their
//! program's own menu. The menu shows on popups of its own ([`crate::menupopup`]).

use reprisekit::controls::editmenu::{self, Live, Menu, Step};
use reprisekit::controls::hosted::{Chain, Hosted};
use reprisekit::controls::menu::{MenuCtx, PopupMenu};
use reprisekit::textedit::{TextSpot, key_for};

use crate::{BTN_RIGHT, Event, KeyInput, Mods, PointerInput, WinId};

#[derive(Default)]
pub(crate) struct EditMenu {
    open: Option<Open>,
    /// A right press waiting for the frame that shows whether it landed in a field.
    pending: Option<(WinId, (i32, i32))>,
}

struct Open {
    win: WinId,
    menu: Menu,
}

impl Chain for Open {
    fn chain(&self) -> &PopupMenu {
        self.menu.chain()
    }
    fn chain_mut(&mut self) -> &mut PopupMenu {
        self.menu.chain_mut()
    }
}

/// What [`EditMenu::event`] did with an event.
#[derive(Debug, PartialEq)]
pub(crate) enum Outcome {
    /// The program gets the event.
    Pass,
    /// The menu took it.
    Taken,
    /// The menu took it and changed: the window draws again.
    Redraw,
    /// The menu closed on a pick: the window draws again and the program gets this key.
    Run(KeyInput),
}

/// The focused field the menu works on: a single-line one.
fn field(spot: Option<&TextSpot>) -> Option<&TextSpot> {
    spot.filter(|s| !s.multiline)
}

/// The key press a pick stands for, as the seat would have sent it.
pub(crate) fn pick_key(cmd: editmenu::Command) -> KeyInput {
    let (sym, ctrl) = editmenu::shortcut(cmd);
    KeyInput {
        sym,
        utf8: None,
        mods: Mods {
            ctrl,
            ..Mods::default()
        },
        edit: key_for(sym, ctrl, false, None),
    }
}

impl EditMenu {
    #[cfg(test)]
    pub fn is_open(&self, win: WinId) -> bool {
        self.open.as_ref().is_some_and(|o| o.win == win)
    }

    pub fn is_pending(&self, win: WinId) -> bool {
        self.pending.is_some_and(|(w, _)| w == win)
    }

    /// `ev` for window `win`, before the program sees it. `spot` is the field `win` last drew
    /// focused, `paste` whether the clipboard holds text.
    pub fn event(
        &mut self,
        win: WinId,
        ev: &Event,
        spot: Option<&TextSpot>,
        paste: bool,
        cx: &MenuCtx,
    ) -> Outcome {
        if let Some(Open { win: w, menu, .. }) = self.open.as_mut()
            && *w == win
        {
            let step = match ev {
                Event::Key(k) => menu.key(k.sym, k.utf8.as_deref(), cx),
                Event::Pointer(PointerInput::Press { x, y, .. }) => menu.press(*x, *y, cx),
                Event::Pointer(PointerInput::Motion { x, y }) => match menu.motion(*x, *y, cx) {
                    true => Step::Redraw,
                    false => Step::None,
                },
                Event::KeyUp(_) | Event::Pointer(_) => Step::None,
                Event::Focus(false) | Event::Resized | Event::CloseRequest => {
                    self.open = None;
                    return Outcome::Pass;
                }
                _ => return Outcome::Pass,
            };
            return match step {
                Step::None => Outcome::Taken,
                Step::Redraw => Outcome::Redraw,
                Step::Close => {
                    self.open = None;
                    Outcome::Redraw
                }
                Step::Run(cmd) => {
                    self.open = None;
                    Outcome::Run(pick_key(cmd))
                }
            };
        }
        match ev {
            Event::Key(k) if editmenu::opens(k.sym, k.mods.shift) => {
                let Some(spot) = field(spot) else {
                    return Outcome::Pass;
                };
                let at = editmenu::under_caret(spot);
                let menu = Menu::open(Live::of(spot, paste), at, cx, true);
                self.open = Some(Open { win, menu });
                Outcome::Redraw
            }
            Event::Pointer(PointerInput::Press {
                x,
                y,
                button: BTN_RIGHT,
                ..
            }) => {
                self.pending = Some((win, (*x, *y)));
                Outcome::Pass
            }
            Event::Pointer(PointerInput::Press { .. }) | Event::Focus(false) => {
                self.pending = None;
                Outcome::Pass
            }
            _ => Outcome::Pass,
        }
    }

    /// After `win` drew, reporting `spot`: a pending right press inside that field opens the
    /// menu at the press. `true` when it opened.
    pub fn drawn(
        &mut self,
        win: WinId,
        spot: Option<&TextSpot>,
        paste: bool,
        cx: &MenuCtx,
    ) -> bool {
        let Some((w, (x, y))) = self.pending else {
            return false;
        };
        if w != win {
            return false;
        }
        self.pending = None;
        let Some(spot) = field(spot).filter(|s| s.field.contains(x, y)) else {
            return false;
        };
        let menu = Menu::open(Live::of(spot, paste), (x, y), cx, false);
        self.open = Some(Open { win, menu });
        true
    }

    pub fn menu(&self, win: WinId) -> Option<&Menu> {
        self.open.as_ref().filter(|o| o.win == win).map(|o| &o.menu)
    }

    /// The open menu and its window, for its popups ([`crate::menupopup`]); dismissing it
    /// closes the menu.
    pub fn hosted(&mut self) -> Option<(WinId, Box<dyn Hosted + '_>)> {
        let win = self.open.as_ref()?.win;
        Some((win, Box::new(&mut self.open)))
    }

    /// Drops what belongs to a window `live` says is gone.
    pub fn retain(&mut self, live: impl Fn(WinId) -> bool) {
        if self.open.as_ref().is_some_and(|o| !live(o.win)) {
            self.open = None;
        }
        if self.pending.is_some_and(|(w, _)| !live(w)) {
            self.pending = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::{form_clipboard, form_input};
    use crate::{Ctx, MAIN};
    use reprisekit::controls::form::{self, Control, Form, Item};
    use reprisekit::controls::menu::MenuItem;
    use reprisekit::raster::{Canvas, Rect};
    use reprisekit::textedit::{TextEdit, keysym};
    use reprisekit::theme::Theme;

    fn ctx() -> Ctx {
        Ctx {
            theme: Theme::default(),
            text: std::rc::Rc::new(reprisekit::text::TextRenderer::load(
                &reprisekit::theme::FontConfig::default(),
            )),
            icons: reprisekit::icontheme::IconCache::new(reprisekit::icontheme::IconTheme::load(
                "test",
                &[],
            )),
            caret_on: true,
            size: (300, 200),
            maximized: false,
            origin: (0, 0),
            wake: crate::Waker::noop(),
            heads: Default::default(),
            want_tick: Default::default(),
            output_size: None,
            at_pick: false,
            text_size: 100,
        }
    }

    fn cx(ctx: &Ctx) -> MenuCtx<'_> {
        MenuCtx {
            text: &ctx.text,
            theme: &ctx.theme,
            px: ctx.theme.fonts.size_px,
            within: Rect::new(0, 0, ctx.size.0, ctx.size.1),
        }
    }

    const NAME: Rect = Rect::new(10, 10, 200, 21);
    const PASSWORD: Rect = Rect::new(10, 50, 200, 21);

    fn form() -> Form {
        let mut f = Form::new(vec![
            Item::edit("name", NAME, "alpha beta", true),
            Item::edit("password", PASSWORD, "", true),
        ]);
        if let Some(Item {
            control: Control::Edit { edit, .. },
            ..
        }) = f.item_mut("password")
        {
            *edit = TextEdit::secret();
            edit.paste("pw", &[]);
        }
        f
    }

    /// The spot the form's focused field reports as it draws.
    fn drawn(f: &Form, ctx: &Ctx) -> Option<TextSpot> {
        let mut c = Canvas::new(ctx.size.0 as u32, ctx.size.1 as u32);
        let px = ctx.theme.fonts.size_px;
        form::draw(&mut c, &ctx.text, &ctx.theme.paint(), f, false, px);
        c.take_spot()
    }

    fn name(f: &Form) -> &str {
        match f.item("name").map(|it| &it.control) {
            Some(Control::Edit { edit, .. }) => &edit.text,
            _ => unreachable!(),
        }
    }

    fn right_press(x: i32, y: i32) -> Event {
        Event::Pointer(PointerInput::Press {
            x,
            y,
            button: BTN_RIGHT,
            time: 0,
            mods: Mods::default(),
        })
    }

    fn key(sym: u32, utf8: Option<&str>, shift: bool) -> Event {
        Event::Key(KeyInput {
            sym,
            utf8: utf8.map(str::to_string),
            mods: Mods {
                shift,
                ..Mods::default()
            },
            edit: None,
        })
    }

    fn rows(m: &EditMenu) -> Vec<bool> {
        m.menu(MAIN)
            .expect("the menu is open")
            .tree()
            .iter()
            .filter_map(|item| match item {
                MenuItem::Entry { enabled, .. } => Some(*enabled),
                MenuItem::Separator => None,
            })
            .collect()
    }

    #[test]
    fn a_right_press_focuses_the_field_under_it_and_the_next_frame_opens_the_menu_there() {
        let ctx = ctx();
        let mut f = form();
        f.focus_id("password");
        let mut m = EditMenu::default();
        let at = (NAME.x + 40, NAME.y + 8);
        let ev = right_press(at.0, at.1);
        let spot = drawn(&f, &ctx);
        assert_eq!(
            m.event(MAIN, &ev, spot.as_ref(), true, &cx(&ctx)),
            Outcome::Pass
        );
        assert!(m.is_pending(MAIN));
        form_input(&mut f, &ev, &ctx);
        assert_eq!(f.focus(), f.index_of("name"), "the press focused the field");
        assert!(m.drawn(MAIN, drawn(&f, &ctx).as_ref(), true, &cx(&ctx)));
        let r = m.menu(MAIN).unwrap().rect();
        assert_eq!((r.x, r.y), at, "the menu's corner sits at the press");
        assert_eq!(
            rows(&m),
            [false, false, false, false, true, false, true],
            "nothing selected, to undo or to redo: only Paste and Select All"
        );

        let mut m = EditMenu::default();
        let off = right_press(NAME.x, NAME.bottom() + 5);
        m.event(MAIN, &off, None, true, &cx(&ctx));
        form_input(&mut f, &off, &ctx);
        assert!(
            !m.drawn(MAIN, drawn(&f, &ctx).as_ref(), true, &cx(&ctx)),
            "a press off the field opens nothing"
        );
    }

    #[test]
    fn a_pick_runs_as_the_fields_shortcut() {
        let ctx = ctx();
        let mut f = form();
        f.focus_id("name");
        let select_all = Event::Key(pick_key(editmenu::Command::SelectAll));
        form_input(&mut f, &select_all, &ctx);
        let mut m = EditMenu::default();
        let spot = drawn(&f, &ctx);
        let open = key(keysym::F10, None, true);
        assert_eq!(
            m.event(MAIN, &open, spot.as_ref(), false, &cx(&ctx)),
            Outcome::Redraw
        );
        assert_eq!(rows(&m), [false, false, true, true, false, true, true]);

        let Outcome::Run(k) = m.event(MAIN, &key(0x63, Some("c"), false), None, false, &cx(&ctx))
        else {
            panic!("C picks Copy");
        };
        assert!(!m.is_open(MAIN));
        let pick = Event::Key(k);
        let copied = form_clipboard(f.focused_edit(), &pick).copy;
        assert_eq!(
            copied.as_deref(),
            Some("alpha beta"),
            "Copy puts the selection on the clipboard"
        );

        assert!(
            form_clipboard(
                f.focused_edit(),
                &Event::Key(pick_key(editmenu::Command::Paste))
            )
            .paste
        );
        let delete = Event::Key(pick_key(editmenu::Command::Delete));
        assert_eq!(
            form_input(&mut f, &delete, &ctx),
            Some(form::Event::Changed("name"))
        );
        assert_eq!(name(&f), "");
        assert!(
            drawn(&f, &ctx).is_some_and(|s| s.can_undo),
            "the delete can be undone"
        );
        let undo = Event::Key(pick_key(editmenu::Command::Undo));
        form_input(&mut f, &undo, &ctx);
        assert_eq!(name(&f), "alpha beta", "Undo puts it back");
        assert!(drawn(&f, &ctx).is_some_and(|s| s.can_redo));
        let redo = Event::Key(pick_key(editmenu::Command::Redo));
        form_input(&mut f, &redo, &ctx);
        assert_eq!(name(&f), "", "Redo deletes it again");
    }

    #[test]
    fn a_password_field_greys_cut_and_copy_and_keeps_paste() {
        let ctx = ctx();
        let mut f = form();
        f.focus_id("password");
        form_input(
            &mut f,
            &Event::Key(pick_key(editmenu::Command::SelectAll)),
            &ctx,
        );
        let mut m = EditMenu::default();
        let spot = drawn(&f, &ctx);
        assert!(spot.as_ref().is_some_and(|s| s.secret && s.selected));
        m.event(
            MAIN,
            &key(keysym::MENU, None, false),
            spot.as_ref(),
            true,
            &cx(&ctx),
        );
        assert_eq!(
            rows(&m),
            [false, false, false, false, true, true, true],
            "Undo, Redo, Cut and Copy greyed; Paste, Delete and Select All live"
        );
        assert_eq!(
            m.event(MAIN, &key(0x63, Some("c"), false), None, true, &cx(&ctx)),
            Outcome::Taken
        );
        assert!(m.is_open(MAIN), "a greyed row's letter picks nothing");
    }

    #[test]
    fn a_multi_line_field_keeps_its_own_menu() {
        let ctx = ctx();
        let mut spot = TextEdit::new("x", false).spot(NAME, NAME);
        spot.multiline = true;
        let mut m = EditMenu::default();
        let menu = key(keysym::MENU, None, false);
        assert_eq!(
            m.event(MAIN, &menu, Some(&spot), true, &cx(&ctx)),
            Outcome::Pass
        );
        m.event(
            MAIN,
            &right_press(NAME.x + 2, NAME.y + 2),
            Some(&spot),
            true,
            &cx(&ctx),
        );
        assert!(!m.drawn(MAIN, Some(&spot), true, &cx(&ctx)));
        assert!(m.menu(MAIN).is_none());
    }

    #[test]
    fn the_open_menu_takes_the_keyboard_and_closes_on_escape_or_focus_loss() {
        let ctx = ctx();
        let spot = TextEdit::new("x", false).spot(NAME, NAME);
        let mut m = EditMenu::default();
        let menu = key(keysym::MENU, None, false);
        m.event(MAIN, &menu, Some(&spot), true, &cx(&ctx));
        assert_eq!(
            m.event(MAIN, &key(0x7a, Some("z"), false), None, true, &cx(&ctx)),
            Outcome::Taken
        );
        assert_eq!(
            m.event(
                MAIN,
                &key(keysym::ESCAPE, None, false),
                None,
                true,
                &cx(&ctx)
            ),
            Outcome::Redraw
        );
        assert!(!m.is_open(MAIN));
        m.event(MAIN, &menu, Some(&spot), true, &cx(&ctx));
        assert_eq!(
            m.event(MAIN, &Event::Focus(false), None, true, &cx(&ctx)),
            Outcome::Pass
        );
        assert!(!m.is_open(MAIN));
    }
}
