//! A wizard: one page of controls over a row of Back, Next and Cancel, with
//! Next reading Finish on the last page. First and last pages are exterior (a
//! dithered navy/black panel at the left, a white body with a large bold title);
//! the pages between are interior (a white band with a bold title and an
//! indented subtitle, an etched line under it).
//!
//! The host builds each page's controls inside [`EXTERIOR_BODY`] or
//! [`INTERIOR_BODY`], keeps one [`Wizard`] per page it shows, feeds the form its
//! input and hands the form's event to [`Wizard::event`]. The buttons are items
//! on the page's [`Form`], so Tab order and access keys work as in any dialog.

use crate::access::{self, Tree, accesskit};
use crate::classic::widgets::{BUTTON_H, BUTTON_W};
use crate::controls::form::{self, Control, Form, Item};
use crate::look::Paint;
use crate::raster::{Canvas, Rect};
use crate::text::{Align, TextRenderer};
use crate::theme::Color;
use accesskit::{NodeId, Role};

pub const BACK: &str = "back";
pub const NEXT: &str = "next";
pub const CANCEL: &str = "cancel";

// Client coordinates. Text rects are placed so TextRenderer::draw's
// baseline centring puts the ink where each constant's comment says.
pub const CLIENT: (i32, i32) = (497, 360);
const PANEL_W: i32 = 164; // the exterior panel's width
/// The exterior panel's dither: a picture, not a themed surface, so a literal
/// colour pair.
const PANEL_NAVY: Color = Color::rgb(0, 0, 128);
const PANEL_BLACK: Color = Color::rgb(0, 0, 0);
const HEADER_H: i32 = 58; // the interior band; its etched line starts here
const TITLE_AT: (i32, i32) = (23, 10); // the interior title's rect top-left
/// One line under the title, indented 15 px past it.
const SUBTITLE_AT: (i32, i32) = (TITLE_AT.0 + 15, TITLE_AT.1 + 16);
const ICON_AT: (i32, i32) = (443, 5); // the header icon's top-left
const EXT_TITLE_AT: (i32, i32) = (171, 13); // the exterior title's rect top-left
const EXT_TITLE_PX: u32 = 16; // 16 px bold gives the title a 12 px cap height
const RULE_Y: i32 = 313; // the etched line over the buttons
const BUTTON_Y: i32 = 327;
const BACK_X: i32 = 252;
const CANCEL_GAP: i32 = 10; // Next's right edge to Cancel's left
pub const EXTERIOR_BODY: Rect = Rect::new(172, 67, 300, RULE_Y - 67);
pub const INTERIOR_BODY: Rect = Rect::new(44, 84, 376, RULE_Y - 84);

pub enum Style {
    Exterior {
        title: String,
    },
    Interior {
        title: String,
        subtitle: String,
        icon: Option<Canvas>,
    },
}

pub struct Wizard {
    pub style: Style,
    pub form: Form,
    pub last: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    None,
    Redraw,
    Back,
    Next,
    Finish,
    Cancel,
    Page(form::Event),
}

impl Wizard {
    /// A page of `items` with the three buttons appended: Back greyed on the
    /// `first` page, Next reading "Finish" on the `last` (the default button).
    pub fn new(style: Style, items: Vec<Item>, first: bool, last: bool) -> Wizard {
        let mut items = items;
        let at = |x| Rect::new(x, BUTTON_Y, BUTTON_W, BUTTON_H);
        items.push(Item::button(BACK, at(BACK_X), "< &Back", !first, false));
        let next = if last { "Finish" } else { "&Next >" };
        items.push(Item::button(NEXT, at(BACK_X + BUTTON_W), next, true, true));
        items.push(Item::button(
            CANCEL,
            at(BACK_X + 2 * BUTTON_W + CANCEL_GAP),
            "Cancel",
            true,
            false,
        ));
        let mut form = Form::new(items);
        if form.focus().is_none() || !first {
            form.focus_id(NEXT);
        }
        Wizard { style, form, last }
    }

    fn enabled(&self, id: &str) -> bool {
        matches!(
            self.form.item(id).map(|i| &i.control),
            Some(Control::Button { enabled: true, .. })
        )
    }

    pub fn set_next_enabled(&mut self, on: bool) {
        self.form.set_enabled(NEXT, on);
    }
    pub fn set_back_enabled(&mut self, on: bool) {
        self.form.set_enabled(BACK, on);
    }
    pub fn set_cancel_enabled(&mut self, on: bool) {
        self.form.set_enabled(CANCEL, on);
    }

    /// The form's event as the wizard's: Enter stands for a live Next, Escape for
    /// Cancel.
    pub fn event(&self, ev: form::Event) -> Event {
        let next = || {
            if self.last {
                Event::Finish
            } else {
                Event::Next
            }
        };
        match ev {
            form::Event::Pressed(BACK) => Event::Back,
            form::Event::Pressed(NEXT) => next(),
            form::Event::Default if self.enabled(NEXT) => next(),
            form::Event::Default => Event::None,
            form::Event::Pressed(CANCEL) | form::Event::Cancel => Event::Cancel,
            form::Event::None => Event::None,
            form::Event::Redraw => Event::Redraw,
            ev => Event::Page(ev),
        }
    }
}

pub fn draw(c: &mut Canvas, text: &TextRenderer, p: &Paint, w: &Wizard, cues: bool, px: u32) {
    let Paint { cs, s, .. } = *p;
    let (cw, _) = CLIENT;
    c.fill_rect(c.bounds(), cs.button_face);
    match &w.style {
        Style::Exterior { title } => {
            c.fill_rect(Rect::new(PANEL_W, 0, cw - PANEL_W, RULE_Y), cs.window);
            c.dither_rect_t(Rect::new(0, 0, PANEL_W, RULE_Y), PANEL_NAVY, PANEL_BLACK, s);
            let tr = Rect::new(
                EXT_TITLE_AT.0,
                EXT_TITLE_AT.1,
                cw - EXT_TITLE_AT.0 - 8,
                EXT_TITLE_PX as i32 + 4,
            );
            for (i, line) in crate::text::wrap_bold(text, title, tr.width, EXT_TITLE_PX, true)
                .iter()
                .enumerate()
            {
                let lr = Rect::new(
                    tr.x,
                    tr.y + i as i32 * (EXT_TITLE_PX as i32 + 4),
                    tr.width,
                    EXT_TITLE_PX as i32 + 4,
                );
                text.draw(c, lr, line, true, cs.window_text, Align::Left, EXT_TITLE_PX);
            }
        }
        Style::Interior {
            title,
            subtitle,
            icon,
        } => {
            c.fill_rect(Rect::new(0, 0, cw, HEADER_H), cs.window);
            text.draw(
                c,
                Rect::new(TITLE_AT.0, TITLE_AT.1, cw - TITLE_AT.0 - 60, 13),
                title,
                true,
                cs.window_text,
                Align::Left,
                px,
            );
            text.draw(
                c,
                Rect::new(SUBTITLE_AT.0, SUBTITLE_AT.1, cw - SUBTITLE_AT.0 - 60, 13),
                subtitle,
                false,
                cs.window_text,
                Align::Left,
                px,
            );
            if let Some(icon) = icon {
                c.blit_over(ICON_AT.0, ICON_AT.1, icon);
            }
            c.etched_hline_t(0, cw, HEADER_H, cs, s);
        }
    }
    c.etched_hline_t(0, cw, RULE_Y, cs, s);
    form::draw(c, text, p, &w.form, cues, px);
}

impl Wizard {
    /// The page's `access` part: title (and an interior page's subtitle) as a
    /// heading, then the form. Returns the focused node, as [`Form::access`] does.
    pub fn access(
        &self,
        tree: &mut Tree,
        parent: NodeId,
        text: &TextRenderer,
        px: u32,
    ) -> Option<NodeId> {
        let (cw, _) = CLIENT;
        match &self.style {
            Style::Exterior { title } => {
                let r = Rect::new(
                    EXT_TITLE_AT.0,
                    EXT_TITLE_AT.1,
                    cw - EXT_TITLE_AT.0 - 8,
                    RULE_Y,
                );
                let mut n = access::label(r, title, true);
                n.set_role(Role::Heading);
                tree.push(parent, "title", n);
            }
            Style::Interior {
                title, subtitle, ..
            } => {
                let r = Rect::new(TITLE_AT.0, TITLE_AT.1, cw - TITLE_AT.0 - 60, 13);
                let mut n = access::label(r, title, true);
                n.set_role(Role::Heading);
                tree.push(parent, "title", n);
                let r = Rect::new(SUBTITLE_AT.0, SUBTITLE_AT.1, cw - SUBTITLE_AT.0 - 60, 13);
                tree.push(parent, "subtitle", access::label(r, subtitle, true));
            }
        }
        self.form.access(tree, parent, text, px)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::look::classic_paint;
    use crate::text::TextRenderer;
    use crate::theme::{FontConfig, Theme};

    fn interior() -> Wizard {
        Wizard::new(
            Style::Interior {
                title: "Title".into(),
                subtitle: "Subtitle".into(),
                icon: None,
            },
            vec![],
            false,
            false,
        )
    }

    #[test]
    fn buttons_sit_where_the_frame_puts_them() {
        let w = interior();
        let rect = |id| w.form.item(id).expect(id).rect;
        assert_eq!(rect(BACK), Rect::new(BACK_X, BUTTON_Y, BUTTON_W, BUTTON_H));
        assert_eq!(rect(NEXT).x, BACK_X + BUTTON_W, "Back and Next touch");
        assert_eq!(rect(CANCEL).x, BACK_X + 2 * BUTTON_W + CANCEL_GAP);
    }

    #[test]
    fn the_first_page_greys_back_and_the_last_says_finish() {
        let first = Wizard::new(Style::Exterior { title: "W".into() }, vec![], true, false);
        assert!(matches!(
            first.form.item(BACK).unwrap().control,
            Control::Button { enabled: false, .. }
        ));
        let last = Wizard::new(Style::Exterior { title: "C".into() }, vec![], false, true);
        assert!(matches!(
            &last.form.item(NEXT).unwrap().control,
            Control::Button { label, .. } if label == "Finish"
        ));
        assert_eq!(last.event(form::Event::Pressed(NEXT)), Event::Finish);
        assert_eq!(last.event(form::Event::Default), Event::Finish);
    }

    #[test]
    fn next_is_the_default_and_escape_cancels() {
        let mut w = interior();
        assert_eq!(w.event(form::Event::Default), Event::Next);
        assert_eq!(w.event(form::Event::Cancel), Event::Cancel);
        assert_eq!(w.event(form::Event::Pressed(BACK)), Event::Back);
        w.set_next_enabled(false);
        assert_eq!(
            w.event(form::Event::Default),
            Event::None,
            "a greyed Next ignores Enter"
        );
    }

    #[test]
    fn interior_draws_the_white_band_and_its_etched_rule() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let text = TextRenderer::load(&FontConfig::default());
        let mut c = Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32);
        draw(
            &mut c,
            &text,
            &classic_paint(cs, 1),
            &interior(),
            false,
            theme.fonts.size_px,
        );
        assert_eq!(
            c.get(CLIENT.0 - 2, 2),
            cs.window.argb(),
            "the band is window white"
        );
        assert_eq!(
            c.get(4, HEADER_H),
            cs.button_shadow.argb(),
            "the rule's dark line"
        );
        assert_eq!(
            c.get(4, HEADER_H + 1),
            cs.button_hilight.argb(),
            "and its light line"
        );
        assert_eq!(
            c.get(4, HEADER_H + 4),
            cs.button_face.argb(),
            "the body is face"
        );
        assert_eq!(
            c.get(4, RULE_Y),
            cs.button_shadow.argb(),
            "the rule over the buttons"
        );
    }

    #[test]
    fn exterior_draws_the_panel_and_a_white_body() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let text = TextRenderer::load(&FontConfig::default());
        let w = Wizard::new(Style::Exterior { title: "W".into() }, vec![], true, false);
        let mut c = Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32);
        draw(
            &mut c,
            &text,
            &classic_paint(cs, 1),
            &w,
            false,
            theme.fonts.size_px,
        );
        assert_eq!(c.get(2, 2), PANEL_NAVY.argb(), "one dither cell");
        assert_eq!(c.get(3, 2), PANEL_BLACK.argb(), "the adjacent cell");
        assert_eq!(
            c.get(PANEL_W + 4, 4),
            cs.window.argb(),
            "the body is window white"
        );
    }

    #[test]
    #[ignore]
    fn render_wizard_pages() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let text = TextRenderer::load(&FontConfig::default());
        let px = theme.fonts.size_px;

        let exterior = Wizard::new(
            Style::Exterior {
                title: "Welcome to the Add Bluetooth Device Wizard".into(),
            },
            vec![],
            true,
            false,
        );
        let mut ce = Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32);
        draw(&mut ce, &text, &classic_paint(cs, 1), &exterior, true, px);
        crate::raster::write_preview(&ce, "wizard-exterior.png");

        let interior = Wizard::new(
            Style::Interior {
                title: "Searching for devices".into(),
                subtitle: "This may take a moment.".into(),
                icon: None,
            },
            vec![],
            false,
            false,
        );
        let mut ci = Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32);
        draw(&mut ci, &text, &classic_paint(cs, 1), &interior, true, px);
        crate::raster::write_preview(&ci, "wizard-interior.png");
    }
}
