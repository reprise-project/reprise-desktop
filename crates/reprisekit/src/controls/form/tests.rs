use std::cell::Cell;

use crate::access::accesskit;
use crate::art::clockface;
use crate::classic::widgets::{Arrow, Axis, CheckState, LINE_H};
use crate::controls::calendar::{self, Calendar};
use crate::controls::colorbutton::ColorButton;
use crate::controls::combo::{self, Combo};
use crate::controls::iconlist;
use crate::controls::listbox::{self, ListBox};
use crate::controls::metrics::ControlMetrics;
use crate::controls::multiedit;
use crate::controls::timefield::TimeField;
use crate::controls::treeview::{self, TreeView};
use crate::controls::updown::{self, UpDown};
use crate::look::classic_paint;
use crate::look::{Look, Paint};
use crate::pointer::{Interaction, WHEEL_ROWS};
use crate::raster::{Canvas, Rect};
use crate::text::{Align, DialogFont, TextRenderer};
use crate::textedit::{EditKey, TextEdit};
use crate::theme::{Color, FontConfig, TextUnits, Theme};
use crate::trackbar::{self, TrackBar};
use accesskit::Role;

use super::draw::ellipsized;
use super::layout::{icon_list_layout, note_lines};
use super::tip::TIP_TICKS;
use super::*;

mod controls;
mod fields;
mod focus;
mod growth;
mod layout;
mod looks;
mod memo;
mod pointer;
mod popups;
mod tips;

fn radio_form() -> Form {
    Form::new(vec![
        Item::radio("up", 10, 10, "&Up", "dir"),
        Item::radio("down", 10, 32, "&Down", "dir").checked(true),
        Item::radio("all", 10, 60, "&All", "range").checked(true),
        Item::new(
            "ok",
            Rect::new(10, 90, 75, 23),
            Control::Button {
                label: "OK".into(),
                enabled: true,
                default: true,
            },
        ),
    ])
}

/// The font size the dialogs are drawn at.
const PX: u32 = 11;

/// A renderer for the tests that hit-test or draw text.
fn tr() -> TextRenderer {
    TextRenderer::load(&FontConfig::default())
}

fn edit_form() -> Form {
    Form::new(vec![
        Item::new(
            "lbl",
            Rect::new(10, 10, 60, 13),
            Control::Label {
                text: "Fi&nd what:".into(),
                enabled: true,
                align: Align::Left,
            },
        ),
        Item::new(
            "find",
            Rect::new(80, 10, 100, 20),
            Control::Edit {
                edit: TextEdit::new(String::new(), false),
                first: Cell::new(0),
                enabled: true,
            },
        ),
        Item::new(
            "ok",
            Rect::new(10, 40, 75, 23),
            Control::Button {
                label: "OK".into(),
                enabled: true,
                default: true,
            },
        ),
    ])
}

fn form() -> Form {
    Form::new(vec![
        Item::new(
            "lbl",
            Rect::new(10, 10, 60, 13),
            Control::Label {
                text: "&Theme:".into(),
                enabled: true,
                align: Align::Left,
            },
        ),
        Item::new(
            "combo",
            Rect::new(10, 30, 150, 21),
            Control::Combo(Combo::new(&["Alpha", "Beta", "Gamma"], 0, 8)),
        ),
        Item::new(
            "check",
            Rect::new(10, 60, 13, 13),
            Control::Check {
                label: "&Password protect".into(),
                checked: false,
                enabled: true,
                mixed: None,
            },
        ),
        Item::new(
            "spin",
            Rect::new(10, 80, 50, 20),
            Control::UpDown(UpDown::new(10, 1, 99)),
        ),
        Item::new(
            "ok",
            Rect::new(10, 110, 75, 23),
            Control::Button {
                label: "OK".into(),
                enabled: true,
                default: true,
            },
        ),
        Item::new(
            "cancel",
            Rect::new(90, 110, 75, 23),
            Control::Button {
                label: "Cancel".into(),
                enabled: true,
                default: false,
            },
        ),
    ])
}

fn spin(f: &Form) -> &UpDown {
    match &f.item("spin").unwrap().control {
        Control::UpDown(u) => u,
        _ => unreachable!("spin is an up-down"),
    }
}

/// `f` drawn through `theme`'s look on its face; the draw also tells
/// the form whether that look lights parts under the pointer.
fn drawn(f: &Form, theme: &Theme) -> Canvas {
    let mut c = Canvas::new(200, 200);
    c.fill_rect(c.bounds(), theme.colors.button_face);
    draw(&mut c, &tr(), &theme.paint(), f, false, PX);
    c
}

/// What (`x`, `y`) hits on `f`.
fn at(f: &Form, x: i32, y: i32) -> Option<Hit> {
    f.hit(x, y, &tr(), PX)
}

fn blue() -> Theme {
    crate::interfaces::theme_for(crate::interface::Interface::Blue)
}
