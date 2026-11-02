//! Calculator's window: the display over a pad of push buttons, the Scientific keys and angle
//! units to their left, the history list to their right. The pad is a [`Form`]; the display
//! draws over its blank item.

use super::{Angle, Calc, Error, Func, Input, Mode};
use crate::access::accesskit::{NodeId, Role};
use crate::access::{self, Tree};
use crate::controls::form::{Control, Form, Item};
use crate::controls::listbox::ListBox;
use crate::look::Paint;
use crate::look::widgets::draw_field_frame;
use crate::raster::{Canvas, Rect};
use crate::text::{Align, TextRenderer};

pub const DISPLAY: &str = "display";
pub const HISTORY_LIST: &str = "history";
pub const CLEAR_HISTORY: &str = "clearhistory";
pub const DEGREES: &str = "deg";
pub const RADIANS: &str = "rad";
pub const GRADS: &str = "grad";

const MARGIN: i32 = 8;
const KEY_W: i32 = 40;
const KEY_H: i32 = 28;
const GAP: i32 = 6;
const DISPLAY_H: i32 = 52;
const PAD_Y: i32 = MARGIN + DISPLAY_H + 8;
const ROWS: i32 = 6;
const PAD_H: i32 = ROWS * (KEY_H + GAP) - GAP;
const HEIGHT: i32 = PAD_Y + PAD_H + MARGIN;
/// The Scientific keys' columns, left of the standard pad's five.
const SCI_COLS: i32 = 3;
const HISTORY_W: i32 = 180;
const RADIO_PITCH: i32 = 21;
/// The expression line's height in the display, above the number.
const EXPR_H: i32 = 16;

/// A pad key: its id, label, input, and cell (column, row, columns spanned, rows spanned) in
/// the standard pad.
type Key = (&'static str, &'static str, Input, (i32, i32, i32, i32));

const STANDARD: &[Key] = &[
    ("mc", "MC", Input::MemClear, (0, 0, 1, 1)),
    ("mr", "MR", Input::MemRecall, (1, 0, 1, 1)),
    ("ms", "MS", Input::MemStore, (2, 0, 1, 1)),
    ("madd", "M+", Input::MemAdd, (3, 0, 1, 1)),
    ("msub", "M\u{2212}", Input::MemSub, (4, 0, 1, 1)),
    ("back", "\u{2190}", Input::Backspace, (0, 1, 1, 1)),
    ("ce", "CE", Input::ClearEntry, (1, 1, 1, 1)),
    ("c", "C", Input::Clear, (2, 1, 1, 1)),
    ("neg", "\u{b1}", Input::Func(Func::Negate), (3, 1, 1, 1)),
    ("sqrt", "\u{221a}", Input::Func(Func::Sqrt), (4, 1, 1, 1)),
    ("7", "7", Input::Digit(7), (0, 2, 1, 1)),
    ("8", "8", Input::Digit(8), (1, 2, 1, 1)),
    ("9", "9", Input::Digit(9), (2, 2, 1, 1)),
    ("div", "\u{f7}", Input::Op('/'), (3, 2, 1, 1)),
    ("pct", "%", Input::Func(Func::Percent), (4, 2, 1, 1)),
    ("4", "4", Input::Digit(4), (0, 3, 1, 1)),
    ("5", "5", Input::Digit(5), (1, 3, 1, 1)),
    ("6", "6", Input::Digit(6), (2, 3, 1, 1)),
    ("mul", "\u{d7}", Input::Op('*'), (3, 3, 1, 1)),
    ("recip", "1/x", Input::Func(Func::Reciprocal), (4, 3, 1, 1)),
    ("1", "1", Input::Digit(1), (0, 4, 1, 1)),
    ("2", "2", Input::Digit(2), (1, 4, 1, 1)),
    ("3", "3", Input::Digit(3), (2, 4, 1, 1)),
    ("sub", "\u{2212}", Input::Op('-'), (3, 4, 1, 1)),
    ("eq", "=", Input::Equals, (4, 4, 1, 2)),
    ("0", "0", Input::Digit(0), (0, 5, 2, 1)),
    ("point", ".", Input::Point, (2, 5, 1, 1)),
    ("add", "+", Input::Op('+'), (3, 5, 1, 1)),
];

/// The Scientific keys, in cells of their own columns (0 to 2) under the angle units.
const SCIENTIFIC: &[Key] = &[
    ("sin", "sin", Input::Func(Func::Sin), (0, 2, 1, 1)),
    ("cos", "cos", Input::Func(Func::Cos), (1, 2, 1, 1)),
    ("tan", "tan", Input::Func(Func::Tan), (2, 2, 1, 1)),
    ("ln", "ln", Input::Func(Func::Ln), (0, 3, 1, 1)),
    ("log", "log", Input::Func(Func::Log), (1, 3, 1, 1)),
    ("fact", "n!", Input::Func(Func::Factorial), (2, 3, 1, 1)),
    ("sq", "x\u{b2}", Input::Func(Func::Square), (0, 4, 1, 1)),
    ("pow", "x^y", Input::Op('^'), (1, 4, 1, 1)),
    ("pi", "\u{3c0}", Input::Pi, (2, 4, 1, 1)),
    ("e", "e", Input::E, (0, 5, 1, 1)),
    ("open", "(", Input::Open, (1, 5, 1, 1)),
    ("close", ")", Input::Close, (2, 5, 1, 1)),
];

/// What a pad press means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    Input(Input),
    Angle(Angle),
    ClearHistory,
}

/// The pad item `id` stands for.
pub fn press_of(id: &str) -> Option<Press> {
    let key = STANDARD.iter().chain(SCIENTIFIC).find(|k| k.0 == id);
    if let Some(k) = key {
        return Some(Press::Input(k.2));
    }
    Some(match id {
        DEGREES => Press::Angle(Angle::Degrees),
        RADIANS => Press::Angle(Angle::Radians),
        GRADS => Press::Angle(Angle::Grads),
        CLEAR_HISTORY => Press::ClearHistory,
        _ => return None,
    })
}

/// The client size at 100% text for `mode`, with the history list or not.
pub fn client(mode: Mode, history: bool) -> (i32, i32) {
    let w = pad_x(mode) + pad_w() + MARGIN;
    let h = if history { HISTORY_W + MARGIN } else { 0 };
    (w + h, HEIGHT)
}

fn pad_w() -> i32 {
    5 * (KEY_W + GAP) - GAP
}

fn pad_x(mode: Mode) -> i32 {
    match mode {
        Mode::Standard => MARGIN,
        Mode::Scientific => MARGIN + SCI_COLS * (KEY_W + GAP),
    }
}

fn cell(x0: i32, (c, r, cw, rh): (i32, i32, i32, i32)) -> Rect {
    Rect::new(
        x0 + c * (KEY_W + GAP),
        PAD_Y + r * (KEY_H + GAP),
        cw * (KEY_W + GAP) - GAP,
        rh * (KEY_H + GAP) - GAP,
    )
}

fn key_item(x0: i32, k: &Key) -> Item {
    Item::button(k.0, cell(x0, k.3), k.1, true, false)
}

/// The pad for `calc` laid out for its mode, the history list shown or not.
pub fn form(calc: &Calc, history: bool) -> Form {
    let (w, _) = client(calc.mode, false);
    let x0 = pad_x(calc.mode);
    let mut items = vec![Item::new(
        DISPLAY,
        Rect::new(MARGIN, MARGIN, w - 2 * MARGIN, DISPLAY_H),
        Control::Blank,
    )];
    if calc.mode == Mode::Scientific {
        for (i, (id, label, angle)) in [
            (DEGREES, tr!("calc-degrees"), Angle::Degrees),
            (RADIANS, tr!("calc-radians"), Angle::Radians),
            (GRADS, tr!("calc-grads"), Angle::Grads),
        ]
        .into_iter()
        .enumerate()
        {
            let y = PAD_Y + 4 + i as i32 * RADIO_PITCH;
            items.push(Item::radio(id, MARGIN, y, &label, "angle").checked(calc.angle == angle));
        }
        items.extend(SCIENTIFIC.iter().map(|k| key_item(MARGIN, k)));
    }
    items.extend(STANDARD.iter().map(|k| key_item(x0, k)));
    if history {
        let x = w;
        let button_y = HEIGHT - MARGIN - KEY_H;
        let list_h = button_y - GAP - MARGIN;
        items.push(Item::new(
            HISTORY_LIST,
            Rect::new(x, MARGIN, HISTORY_W, list_h),
            Control::ListBox {
                list: ListBox::new(Vec::new(), None),
                icons: Vec::new(),
            },
        ));
        let bw = crate::classic::widgets::BUTTON_W;
        items.push(Item::button(
            CLEAR_HISTORY,
            Rect::new(x + HISTORY_W - bw, button_y, bw, KEY_H),
            &tr!("calc-clear-history"),
            true,
            false,
        ));
    }
    let mut f = Form::new(items);
    // The keyboard types into the display; Tab brings the focus to the pad.
    f.set_focus(None);
    follow(&mut f, calc);
    f
}

/// Brings the form's history rows, newest first, and angle units to `calc`'s.
pub fn follow(f: &mut Form, calc: &Calc) {
    for (id, angle) in [
        (DEGREES, Angle::Degrees),
        (RADIANS, Angle::Radians),
        (GRADS, Angle::Grads),
    ] {
        if f.item(id).is_some() {
            f.set_check(id, calc.angle == angle);
        }
    }
    let rows: Vec<String> = calc.history.iter().rev().map(super::Line::text).collect();
    if let Some(Control::ListBox { list, .. }) = f.item_mut(HISTORY_LIST).map(|i| &mut i.control)
        && list.items != rows
    {
        list.top = 0;
        list.selected = None;
        list.items = rows;
    }
}

/// The history line the list's pick stands for, once picked.
pub fn picked_line(f: &Form, calc: &Calc) -> Option<usize> {
    match &f.item(HISTORY_LIST)?.control {
        Control::ListBox { list, .. } => calc.history.len().checked_sub(list.selected? + 1),
        _ => None,
    }
}

/// The message the display shows for `e`.
pub fn error_text(e: Error) -> String {
    match e {
        Error::DivideByZero => tr!("calc-divide-by-zero"),
        Error::Invalid => tr!("calc-invalid"),
        Error::Overflow => tr!("calc-overflow"),
    }
}

/// The number line's text and the expression line's.
fn shown(calc: &Calc) -> (String, String) {
    match calc.error {
        Some(e) => (error_text(e), String::new()),
        None => (calc.display(), calc.expression()),
    }
}

/// Draws the pad and over it the display: the expression line, the number right-aligned at
/// twice the text size (smaller when it would not fit), and an M while the memory holds a value.
pub fn draw(c: &mut Canvas, text: &TextRenderer, p: &Paint, f: &Form, calc: &Calc, px: u32) {
    crate::controls::form::draw(c, text, p, f, true, px);
    let Some(r) = f.rect(DISPLAY) else {
        return;
    };
    let s = p.s;
    let r = r.scaled(s);
    c.fill_rect(r, p.cs.window);
    draw_field_frame(c, p, r, p.cs.window);
    let inner = r.inset(4 * s);
    let (number, expr) = shown(calc);
    let small = Rect::new(inner.x, inner.y, inner.width, p.text.len(EXPR_H) * s);
    text.draw(c, small, &expr, false, p.cs.gray_text, Align::Right, px);
    let line = Rect::new(
        inner.x,
        small.bottom(),
        inner.width,
        inner.bottom() - small.bottom(),
    );
    let mark_w = text.measure("M", true, px) + 4 * s;
    let room = inner.width - mark_w;
    let mut big = px * 2;
    while big > px && text.measure(&number, false, big) > room {
        big -= 1;
    }
    text.draw(c, line, &number, false, p.cs.window_text, Align::Right, big);
    if calc.memory.is_some() {
        text.draw(c, line, "M", true, p.cs.window_text, Align::Left, px);
    }
}

/// The access part: the form's nodes, the display's text as a status under `parent`. Returns
/// the focused node.
pub fn access(
    f: &Form,
    calc: &Calc,
    tree: &mut Tree,
    parent: NodeId,
    text: &TextRenderer,
    px: u32,
) -> Option<NodeId> {
    let focus = f.access(tree, parent, text, px);
    if let Some(r) = f.rect(DISPLAY) {
        let (number, expr) = shown(calc);
        let mut n = access::node(Role::Status, r);
        n.set_value(number);
        if !expr.is_empty() {
            n.set_description(expr);
        }
        tree.push(parent, DISPLAY, n);
    }
    focus
}
