//! The location prompt: Reprise asks whether a program may have the location, with Allow
//! and Deny and a check box that remembers the answer. The model is [`Fields`]; [`form`]
//! lays it out over [`client`]'s size as a [`Form`]; [`answer`] reads a form event as the answer.

use crate::classic::widgets::{BUTTON_GAP, BUTTON_H, BUTTON_W, LINE_H};
use crate::controls::form::{self, Event, Form, Item};
use crate::look::Paint;
use crate::raster::{Canvas, Rect};
use crate::text::TextRenderer;

pub const ALLOW: &str = "allow";
pub const DENY: &str = "deny";
pub const REMEMBER: &str = "remember";
const QUESTION: &str = "question";
const DETAIL: &str = "detail";

pub const TITLE: &str = "Location";

/// The prompt's model.
#[derive(Clone, Debug, PartialEq)]
pub struct Fields {
    pub program: String,
    /// The 32 px picture at the top left (the program's icon); `None`
    /// leaves the slot empty.
    pub icon: Option<Canvas>,
}

impl Fields {
    fn question(&self) -> String {
        format!("Allow {} to use your location?", self.program)
    }
}

const DETAIL_TEXT: &str = "A remembered choice can be forgotten on the Advanced page of \
                           Reprise Settings.";

/// What the user answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Answer {
    pub allow: bool,
    /// Keep the answer for the next time the program asks.
    pub remember: bool,
}

/// The answer a form event stands for: Allow or Deny with the check box
/// as it is. Enter presses Deny, the default, so Enter never grants by
/// accident. Escape and the frame's close box (`Event::Cancel`) deny
/// without remembering: closing the prompt is no decision to keep.
/// Anything else is no answer yet.
pub fn answer(form: &Form, ev: &Event) -> Option<Answer> {
    let remember = form.checked(REMEMBER).unwrap_or(false);
    match ev {
        Event::Pressed(ALLOW) => Some(Answer {
            allow: true,
            remember,
        }),
        Event::Pressed(DENY) | Event::Default => Some(Answer {
            allow: false,
            remember,
        }),
        Event::Cancel => Some(Answer {
            allow: false,
            remember: false,
        }),
        _ => None,
    }
}

const W: i32 = 360;
const ICON: i32 = 32;
const ICON_XY: (i32, i32) = (14, 14);
const TEXT_X: i32 = 62;
const TEXT_Y: i32 = 16;
const MARGIN: i32 = 11;
const GAP: i32 = 8;
const BUTTON_GAP_Y: i32 = 14;

struct Plan {
    question: Vec<String>,
    detail: Vec<String>,
    detail_y: i32,
    check_y: i32,
    button_y: i32,
}

fn plan(f: &Fields, text: &TextRenderer, px: u32) -> Plan {
    let col = W - TEXT_X - MARGIN;
    let question = crate::text::wrap(text, &f.question(), col, px);
    let detail_y = TEXT_Y + question.len() as i32 * LINE_H + GAP;
    let detail = crate::text::wrap(text, DETAIL_TEXT, col, px);
    let check_y = (detail_y + detail.len() as i32 * LINE_H + GAP).max(ICON_XY.1 + ICON + GAP);
    let button_y = check_y + LINE_H + BUTTON_GAP_Y;
    Plan {
        question,
        detail,
        detail_y,
        check_y,
        button_y,
    }
}

/// The client size the prompt needs: a long program name wraps and
/// makes it taller.
pub fn client(f: &Fields, text: &TextRenderer, px: u32) -> (i32, i32) {
    (W, plan(f, text, px).button_y + BUTTON_H + MARGIN)
}

/// Lay the prompt out over [`client`]'s size: the check box ticked, the
/// focus and the default on Deny.
pub fn form(f: &Fields, text: &TextRenderer, px: u32) -> Form {
    let p = plan(f, text, px);
    let col = W - TEXT_X - MARGIN;
    let mut items = Vec::new();
    if let Some(icon) = &f.icon {
        items.push(Item::picture(
            "icon",
            Rect::new(ICON_XY.0, ICON_XY.1, ICON, ICON),
            Some(icon.clone()),
        ));
    }
    let question: Vec<&str> = p.question.iter().map(String::as_str).collect();
    items.push(Item::lines_at(QUESTION, TEXT_X, TEXT_Y, col, &question));
    let detail: Vec<&str> = p.detail.iter().map(String::as_str).collect();
    items.push(Item::lines_at(DETAIL, TEXT_X, p.detail_y, col, &detail));
    items.push(
        Item::check(
            REMEMBER,
            TEXT_X,
            p.check_y,
            "&Remember my choice for this program",
        )
        .checked(true),
    );
    let deny_x = W - MARGIN - BUTTON_W;
    let allow_x = deny_x - BUTTON_GAP - BUTTON_W;
    items.push(Item::button(
        ALLOW,
        Rect::new(allow_x, p.button_y, BUTTON_W, BUTTON_H),
        "&Allow",
        true,
        false,
    ));
    items.push(Item::button(
        DENY,
        Rect::new(deny_x, p.button_y, BUTTON_W, BUTTON_H),
        "&Deny",
        true,
        true,
    ));
    let mut form = Form::new(items);
    form.focus_id(DENY);
    form
}

pub fn draw(c: &mut Canvas, text: &TextRenderer, p: &Paint, form: &Form, px: u32) {
    c.fill_rect(c.bounds(), p.cs.button_face);
    form::draw(c, text, p, form, true, px);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interface::Interface;
    use crate::interfaces::theme_for;
    use crate::theme::{FontConfig, Theme};

    fn rend() -> (TextRenderer, u32) {
        let text = TextRenderer::load(&FontConfig::default());
        (text, Theme::default().fonts.size_px)
    }

    fn fields() -> Fields {
        Fields {
            program: "Maps".into(),
            icon: None,
        }
    }

    /// Allow and Deny answer with the check box; Enter denies, Escape or the close box deny without remembering.
    #[test]
    fn the_buttons_answer_and_closing_remembers_nothing() {
        let (text, px) = rend();
        let mut f = form(&fields(), &text, px);
        assert_eq!(f.focus().map(|i| f.items[i].id), Some(DENY));
        assert_eq!(f.default_button().map(|i| f.items[i].id), Some(DENY));
        assert_eq!(
            f.checked(REMEMBER),
            Some(true),
            "remembering is the default"
        );
        let yes = |allow, remember| Some(Answer { allow, remember });
        assert_eq!(answer(&f, &Event::Pressed(ALLOW)), yes(true, true));
        assert_eq!(answer(&f, &Event::Pressed(DENY)), yes(false, true));
        assert_eq!(answer(&f, &Event::Default), yes(false, true));
        assert_eq!(answer(&f, &Event::Cancel), yes(false, false));
        assert_eq!(answer(&f, &Event::Redraw), None);
        if let Some(form::Control::Check { checked, .. }) =
            f.item_mut(REMEMBER).map(|i| &mut i.control)
        {
            *checked = false;
        }
        assert_eq!(answer(&f, &Event::Pressed(ALLOW)), yes(true, false));
    }

    #[test]
    fn the_prompt_lays_out_on_its_grid() {
        let (text, px) = rend();
        let fields = fields();
        let (w, h) = client(&fields, &text, px);
        assert_eq!(w, 360);
        let f = form(&fields, &text, px);
        let r = |id: &str| f.item(id).expect("the item exists").rect;
        match &f.item(QUESTION).unwrap().control {
            form::Control::Lines(l) => {
                assert_eq!(l, &["Allow Maps to use your location?"])
            }
            other => panic!("the question is lines, not {other:?}"),
        }
        assert_eq!((r(QUESTION).x, r(QUESTION).y), (62, 16));
        assert_eq!(r(DETAIL).y, r(QUESTION).bottom() + 8);
        let check = r(REMEMBER);
        assert_eq!(check.x, 62);
        assert!(check.y >= 14 + 32 + 8, "the check box is under the picture");
        assert!(check.y >= r(DETAIL).bottom() + 8);
        let (allow, deny) = (r(ALLOW), r(DENY));
        assert_eq!((deny.right(), deny.bottom()), (w - 11, h - 11));
        assert_eq!(allow.right() + 6, deny.x);
        assert_eq!((allow.width, allow.height), (75, 23));
    }

    #[test]
    fn a_long_name_wraps_and_grows_the_prompt() {
        let (text, px) = rend();
        let short = client(&fields(), &text, px);
        let long = Fields {
            program: "A Program With A Rather Long Name From Its Desktop Entry".into(),
            icon: None,
        };
        let tall = client(&long, &text, px);
        assert_eq!(tall.0, short.0);
        assert!(tall.1 > short.1, "{tall:?} taller than {short:?}");
    }

    #[test]
    fn the_classic_prompt_draws_its_buttons_and_tick() {
        if !crate::text::test_tahoma() {
            return;
        }
        let (text, px) = rend();
        let theme = Theme::default();
        let cs = &theme.colors;
        let fields = fields();
        let (w, h) = client(&fields, &text, px);
        let f = form(&fields, &text, px);
        let mut c = Canvas::new(w as u32, h as u32);
        draw(&mut c, &text, &theme.paint(), &f, px);
        let r = |id: &str| f.item(id).unwrap().rect;
        let (allow, deny, check) = (r(ALLOW), r(DENY), r(REMEMBER));
        assert_eq!(
            c.get(deny.x, deny.y + 5),
            cs.button_text.argb(),
            "Deny's ring"
        );
        assert_eq!(
            c.get(allow.x, allow.y + 5),
            cs.button_hilight.argb(),
            "Allow's raised edge"
        );
        assert_eq!(
            c.get(check.x, check.y + 5),
            cs.button_shadow.argb(),
            "the box edge"
        );
        let tick = (check.x + 2..check.right() - 2)
            .flat_map(|x| (check.y + 2..check.bottom() - 2).map(move |y| (x, y)))
            .filter(|&(x, y)| c.get(x, y) == cs.window_text.argb())
            .count();
        assert!(tick >= 7, "the tick ({tick} px)");
        assert!(
            (0..w).all(|x| c.get(x, h - 1) == cs.button_face.argb()),
            "the bottom row is empty"
        );
    }

    #[test]
    fn blue_and_glass_draw_the_prompt_in_their_looks() {
        let (text, px) = rend();
        let fields = fields();
        let (w, h) = client(&fields, &text, px);
        let f = form(&fields, &text, px);
        let classic = {
            let mut c = Canvas::new(w as u32, h as u32);
            draw(&mut c, &text, &Theme::default().paint(), &f, px);
            c
        };
        let allow = f.item(ALLOW).unwrap().rect;
        for i in [Interface::Blue, Interface::Glass] {
            let theme = theme_for(i);
            let mut c = Canvas::new(w as u32, h as u32);
            draw(&mut c, &text, &theme.paint(), &f, px);
            assert_eq!(
                c.get(2, h - 2),
                theme.colors.button_face.argb(),
                "{i:?}: the face is the look's"
            );
            let differs = (allow.y..allow.bottom())
                .flat_map(|y| (allow.x..allow.right()).map(move |x| (x, y)))
                .filter(|&(x, y)| c.get(x, y) != classic.get(x, y))
                .count();
            assert!(
                differs > 100,
                "{i:?}: Allow is the look's button ({differs} px)"
            );
        }
    }

    /// Writes target/locationprompt-preview.png for a visual check.
    #[test]
    #[ignore]
    fn render_locationprompt() {
        let (text, px) = rend();
        let theme = Theme::default();
        let fields = fields();
        let (w, h) = client(&fields, &text, px);
        let mut c = Canvas::new(w as u32, h as u32);
        draw(&mut c, &text, &theme.paint(), &form(&fields, &text, px), px);
        crate::raster::write_preview(&c, "locationprompt-preview.png");
    }
}
