use crate::classic::widgets::{BUTTON_GAP, BUTTON_H, BUTTON_W, LINE_H};
use crate::controls::form::{self, Control, Event, Form, Item};
use crate::controls::listbox::{self, ListBox};
use crate::look::Paint;
use crate::raster::{Canvas, Rect};
use crate::text::TextRenderer;
use crate::theme::TextUnits;

pub const END_NOW: &str = "end_now";
pub const CANCEL: &str = "cancel";
pub const LIST: &str = "list";
const INTRO: &str = "intro";
const OUTRO: &str = "outro";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Goal {
    LogOff,
    TurnOff,
    Restart,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Row {
    pub name: String,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Fields {
    pub goal: Goal,
    pub rows: Vec<Row>,
    pub icon: Option<Canvas>,
}

impl Fields {
    pub fn title(&self) -> String {
        match self.goal {
            Goal::LogOff => tr!("endsession-title-log-off"),
            Goal::TurnOff => tr!("endsession-title-turn-off"),
            Goal::Restart => tr!("endsession-title-restart"),
        }
    }

    fn intro(&self) -> String {
        match self.goal {
            Goal::LogOff => tr!("endsession-intro-log-off"),
            Goal::TurnOff => tr!("endsession-intro-turn-off"),
            Goal::Restart => tr!("endsession-intro-restart"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    EndNow,
    Cancel,
}

/// Enter and Escape press Cancel, the default; the host sends the frame's
/// close box as `Event::Cancel`.
pub fn choice(ev: &Event) -> Option<Choice> {
    match ev {
        Event::Pressed(END_NOW) => Some(Choice::EndNow),
        Event::Default | Event::Cancel | Event::Pressed(CANCEL) => Some(Choice::Cancel),
        _ => None,
    }
}

// Client coordinates.
const W: i32 = 400;
const ICON: i32 = 32;
const ICON_XY: (i32, i32) = (14, 14);
const TEXT_X: i32 = 62;
const TEXT_Y: i32 = 16;
const MARGIN: i32 = 11;
const GAP: i32 = 10;
const LIST_ROWS: i32 = 6;
const LIST_H: i32 = LIST_ROWS * listbox::ROW_H + 4;
const BUTTON_GAP_Y: i32 = 12;

struct Plan {
    intro: Vec<String>,
    list_y: i32,
    outro: Vec<String>,
    outro_y: i32,
    button_y: i32,
}

/// The prompt at 100% for text set at `px`, which grows by `u`: each paragraph wraps at the
/// width its column grows to.
fn plan(f: &Fields, text: &TextRenderer, px: u32, u: TextUnits) -> Plan {
    let intro = crate::text::wrap(text, &f.intro(), u.len(W - TEXT_X - MARGIN), px);
    let intro_end = TEXT_Y + intro.len() as i32 * LINE_H;
    let list_y = intro_end.max(ICON_XY.1 + ICON) + GAP;
    let outro_y = list_y + LIST_H + GAP;
    let outro = crate::text::wrap(text, &tr!("endsession-outro"), u.len(W - 2 * MARGIN), px);
    let button_y = outro_y + outro.len() as i32 * LINE_H + BUTTON_GAP_Y;
    Plan {
        intro,
        list_y,
        outro,
        outro_y,
        button_y,
    }
}

/// The client size the prompt needs. It depends on the goal alone, so
/// refilling the list never moves the frame.
pub fn client(f: &Fields, text: &TextRenderer, px: u32, u: TextUnits) -> (i32, i32) {
    (W, plan(f, text, px, u).button_y + BUTTON_H + MARGIN)
}

fn list(rows: &[Row]) -> ListBox {
    let mut lb = ListBox::new(rows.iter().map(|r| r.name.clone()).collect(), None);
    lb.detail = rows.iter().map(|r| r.reason.clone()).collect();
    lb
}

pub fn form(f: &Fields, text: &TextRenderer, px: u32, u: TextUnits) -> Form {
    let p = plan(f, text, px, u);
    let mut items = Vec::new();
    if let Some(icon) = &f.icon {
        items.push(Item::picture(
            "icon",
            Rect::new(ICON_XY.0, ICON_XY.1, ICON, ICON),
            Some(icon.clone()),
        ));
    }
    let intro: Vec<&str> = p.intro.iter().map(String::as_str).collect();
    items.push(Item::lines(
        INTRO,
        Rect::new(
            TEXT_X,
            TEXT_Y,
            W - TEXT_X - MARGIN,
            intro.len() as i32 * LINE_H,
        ),
        &intro,
    ));
    items.push(Item::new(
        LIST,
        Rect::new(MARGIN, p.list_y, W - 2 * MARGIN, LIST_H),
        Control::ListBox {
            list: list(&f.rows),
            icons: Vec::new(),
        },
    ));
    let outro: Vec<&str> = p.outro.iter().map(String::as_str).collect();
    items.push(Item::lines(
        OUTRO,
        Rect::new(
            MARGIN,
            p.outro_y,
            W - 2 * MARGIN,
            outro.len() as i32 * LINE_H,
        ),
        &outro,
    ));
    let cancel_x = W - MARGIN - BUTTON_W;
    let end_x = cancel_x - BUTTON_GAP - BUTTON_W;
    items.push(Item::button(
        END_NOW,
        Rect::new(end_x, p.button_y, BUTTON_W, BUTTON_H),
        &tr!("endsession-end-now"),
        true,
        false,
    ));
    items.push(Item::button(
        CANCEL,
        Rect::new(cancel_x, p.button_y, BUTTON_W, BUTTON_H),
        &tr!("cancel"),
        true,
        true,
    ));
    let mut form = Form::new(items);
    form.focus_id(CANCEL);
    form
}

pub fn set_rows(form: &mut Form, rows: &[Row]) -> bool {
    let Some(Control::ListBox { list: lb, .. }) = form.item_mut(LIST).map(|i| &mut i.control)
    else {
        return false;
    };
    let mut new = list(rows);
    if new.items == lb.items && new.detail == lb.detail {
        return false;
    }
    new.selected = lb
        .selected
        .and_then(|i| lb.items.get(i))
        .and_then(|name| new.items.iter().position(|n| n == name));
    new.top = lb
        .top
        .min(new.items.len().saturating_sub(LIST_ROWS as usize));
    *lb = new;
    true
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

    fn row(name: &str, reason: &str) -> Row {
        Row {
            name: name.into(),
            reason: reason.into(),
        }
    }

    fn fields() -> Fields {
        Fields {
            goal: Goal::LogOff,
            rows: vec![
                row("Untitled - Notepad", ""),
                row("Text Editor", "There are unsaved documents"),
            ],
            icon: None,
        }
    }

    fn rows_of(f: &Form) -> (Vec<String>, Vec<String>, Option<usize>) {
        match &f.item(LIST).unwrap().control {
            Control::ListBox { list, .. } => {
                (list.items.clone(), list.detail.clone(), list.selected)
            }
            other => panic!("the list is a list box, not {other:?}"),
        }
    }

    #[test]
    fn the_buttons_map_to_answers_and_cancel_is_the_default() {
        let (text, px) = rend();
        let f = form(&fields(), &text, px, TextUnits::ONE);
        assert_eq!(f.focus().map(|i| f.items[i].id), Some(CANCEL));
        assert_eq!(f.default_button().map(|i| f.items[i].id), Some(CANCEL));
        assert_eq!(choice(&Event::Pressed(END_NOW)), Some(Choice::EndNow));
        assert_eq!(choice(&Event::Pressed(CANCEL)), Some(Choice::Cancel));
        assert_eq!(choice(&Event::Default), Some(Choice::Cancel));
        assert_eq!(choice(&Event::Cancel), Some(Choice::Cancel));
        assert_eq!(choice(&Event::Redraw), None);
    }

    #[test]
    fn the_goal_words_the_prompt_and_rows_never_resize_it() {
        let (text, px) = rend();
        let mut f = fields();
        assert_eq!(f.title(), "Log Off Reprise");
        assert!(f.intro().contains("before Reprise can log off"));
        let size = client(&f, &text, px, TextUnits::ONE);
        f.rows = (0..20).map(|i| row(&format!("Program {i}"), "")).collect();
        assert_eq!(client(&f, &text, px, TextUnits::ONE), size);
        f.goal = Goal::Restart;
        assert_eq!(f.title(), "Restart Computer");
        assert!(f.intro().contains("the computer can restart"));
        f.goal = Goal::TurnOff;
        assert_eq!(f.title(), "Turn Off Computer");
    }

    #[test]
    fn the_prompt_lays_out_on_its_grid() {
        let (text, px) = rend();
        let fields = fields();
        let (w, h) = client(&fields, &text, px, TextUnits::ONE);
        assert_eq!(w, 400);
        let f = form(&fields, &text, px, TextUnits::ONE);
        let r = |id: &str| f.item(id).expect("the item exists").rect;
        let list = r(LIST);
        assert_eq!((list.x, list.width, list.height), (11, 378, 106));
        assert!(list.y >= 14 + 32 + 10, "the list starts under the picture");
        assert_eq!(r(OUTRO).y, list.bottom() + 10);
        let (end, cancel) = (r(END_NOW), r(CANCEL));
        assert_eq!((cancel.right(), cancel.bottom()), (w - 11, h - 11));
        assert_eq!(end.right() + 6, cancel.x);
        assert_eq!((end.width, end.height), (75, 23));
    }

    #[test]
    fn set_rows_refills_the_list_in_place() {
        let (text, px) = rend();
        let mut f = form(&fields(), &text, px, TextUnits::ONE);
        assert_eq!(
            rows_of(&f).1,
            ["", "There are unsaved documents"],
            "the reason is the row's detail"
        );
        if let Some(Control::ListBox { list, .. }) = f.item_mut(LIST).map(|i| &mut i.control) {
            list.selected = Some(1);
        }
        assert!(
            !set_rows(&mut f, &fields().rows),
            "the same rows change nothing"
        );
        let fewer = [row("Text Editor", "There are unsaved documents")];
        assert!(set_rows(&mut f, &fewer));
        assert_eq!(
            rows_of(&f),
            (
                vec!["Text Editor".into()],
                vec![fewer[0].reason.clone()],
                Some(0)
            )
        );
        assert!(set_rows(&mut f, &[]));
        assert_eq!(rows_of(&f).2, None, "a program gone takes the pick with it");
    }

    #[test]
    fn the_classic_prompt_draws_its_list_and_buttons() {
        if !crate::text::test_tahoma() {
            return;
        }
        let (text, px) = rend();
        let theme = Theme::default();
        let cs = &theme.colors;
        let fields = fields();
        let (w, h) = client(&fields, &text, px, TextUnits::ONE);
        let f = form(&fields, &text, px, TextUnits::ONE);
        let mut c = Canvas::new(w as u32, h as u32);
        draw(&mut c, &text, &theme.paint(), &f, px);
        let r = |id: &str| f.item(id).unwrap().rect;
        let (end, cancel, list) = (r(END_NOW), r(CANCEL), r(LIST));
        assert_eq!(
            c.get(cancel.x, cancel.y + 5),
            cs.button_text.argb(),
            "Cancel's default ring"
        );
        assert_eq!(
            c.get(end.x, end.y + 5),
            cs.button_hilight.argb(),
            "End Now's raised edge"
        );
        assert_eq!(
            c.get(list.x, list.y + 10),
            cs.button_shadow.argb(),
            "the list's sunken outer edge"
        );
        assert_eq!(
            c.get(list.right() - 4, list.y + 4),
            cs.window.argb(),
            "the list's well"
        );
        let ink = |x0: i32, x1: i32, y: i32, color: u32| {
            (x0..x1)
                .flat_map(|x| (y..y + listbox::ROW_H).map(move |y| (x, y)))
                .filter(|&(x, y)| c.get(x, y) == color)
                .count()
        };
        let row0 = list.y + 2;
        let tx = list.x + 2 + listbox::TEXT_X;
        assert!(
            ink(tx, tx + 80, row0, cs.button_text.argb()) > 20,
            "the first program's name"
        );
        assert!(
            ink(
                tx + 60,
                list.right() - 2,
                row0 + listbox::ROW_H,
                cs.gray_text.argb()
            ) > 20,
            "the second program's reason, dimmed"
        );
        assert!(
            (0..w).all(|x| c.get(x, h - 1) == cs.button_face.argb()),
            "the bottom row is empty"
        );
    }

    #[test]
    fn blue_and_glass_draw_the_prompt_in_their_looks() {
        let (text, px) = rend();
        let fields = fields();
        let (w, h) = client(&fields, &text, px, TextUnits::ONE);
        let f = form(&fields, &text, px, TextUnits::ONE);
        let classic = {
            let mut c = Canvas::new(w as u32, h as u32);
            draw(&mut c, &text, &Theme::default().paint(), &f, px);
            c
        };
        let end = f.item(END_NOW).unwrap().rect;
        for i in [Interface::Blue, Interface::Glass] {
            let theme = theme_for(i);
            let mut c = Canvas::new(w as u32, h as u32);
            draw(&mut c, &text, &theme.paint(), &f, px);
            assert_eq!(
                c.get(2, h - 2),
                theme.colors.button_face.argb(),
                "{i:?}: the face is the look's"
            );
            let differs = (end.y..end.bottom())
                .flat_map(|y| (end.x..end.right()).map(move |x| (x, y)))
                .filter(|&(x, y)| c.get(x, y) != classic.get(x, y))
                .count();
            assert!(
                differs > 100,
                "{i:?}: End Now is the look's button ({differs} px)"
            );
        }
    }

    #[test]
    #[ignore]
    fn render_endsession() {
        let (text, px) = rend();
        let theme = Theme {
            access_keys: true,
            ..Theme::default()
        };
        let fields = fields();
        let (w, h) = client(&fields, &text, px, TextUnits::ONE);
        let mut c = Canvas::new(w as u32, h as u32);
        draw(
            &mut c,
            &text,
            &theme.paint(),
            &form(&fields, &text, px, TextUnits::ONE),
            px,
        );
        crate::raster::write_preview(&c, "endsession-preview.png");
    }
}
