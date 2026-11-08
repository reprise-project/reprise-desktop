use super::*;
use crate::classic::widgets::{BUTTON_H, BUTTON_W, BarHit, LINE_H};
use crate::controls::form::{self, Control, Form, Item};
use crate::controls::{pagelist, tabstrip};
use crate::look::Paint;
use crate::look::classic_paint;
use crate::look::widgets::draw_page;
use crate::raster::{Canvas, Rect};
use crate::text::{Align, DialogFont, TextRenderer};
use crate::textedit::EditKey;
use crate::theme::TextUnits;
use crate::theme::{FontConfig, Theme};

mod geometry;
mod hover;
mod lists;
mod restore_defaults;
mod scrolling;
mod search;
mod tabs;

fn sheet(labels: &[&str], size: (i32, i32)) -> PropSheet {
    let pages = labels
        .iter()
        .map(|l| {
            Form::new(vec![Item::new(
                "check",
                Rect::new(20, 50, 13, 13),
                Control::Check {
                    label: (*l).into(),
                    checked: false,
                    enabled: true,
                    mixed: None,
                },
            )])
        })
        .collect();
    PropSheet::new(labels, pages, size)
}

fn check_page(label: &str) -> Form {
    Form::new(vec![Item::new(
        "check",
        Rect::new(20, 50, 13, 13),
        Control::Check {
            label: label.into(),
            checked: false,
            enabled: true,
            mixed: None,
        },
    )])
}

const LIST_GROUPS: &[(&str, &[pagelist::Item])] = &[
    (
        "Look",
        &[
            pagelist::Item::Page("Night Light"),
            pagelist::Item::Page("Effects"),
        ],
    ),
    ("Input", &[pagelist::Item::Page("Mouse")]),
];

fn list_sheet() -> PropSheet {
    let pages = ["Night Light", "Effects", "Mouse"]
        .into_iter()
        .map(check_page)
        .collect();
    PropSheet::with_list(LIST_GROUPS, pages, (398, 424))
}

const LINK_GROUPS: &[(&str, &[pagelist::Item])] = &[
    (
        "Look",
        &[
            pagelist::Item::Page("Night Light"),
            pagelist::Item::Page("Effects"),
        ],
    ),
    (
        "Devices",
        &[
            pagelist::Item::Link {
                label: "Bluetooth",
                name: "Bluetooth",
            },
            pagelist::Item::Page("Mouse"),
            pagelist::Item::Link {
                label: "Printers",
                name: "Printers",
            },
        ],
    ),
];

/// Two pages under Look; Bluetooth, Mouse and Printers under Devices, the first and last links.
fn link_sheet() -> PropSheet {
    let pages = ["Night Light", "Effects", "Mouse"]
        .into_iter()
        .map(check_page)
        .collect();
    PropSheet::with_list(LINK_GROUPS, pages, (398, 424))
}

fn cursor(s: &PropSheet) -> Option<usize> {
    match &s.nav {
        Nav::List(l) => l.cursor,
        Nav::Tabs => unreachable!(),
    }
}

fn grey(s: &mut PropSheet, link: usize) {
    match &mut s.nav {
        Nav::List(l) => l.set_link_enabled(link, false),
        Nav::Tabs => unreachable!(),
    }
}

/// A sheet drawn under `theme`, which tells its pages whether the look
/// lights parts under the pointer.
fn drawn(s: &PropSheet, theme: &Theme) -> Canvas {
    let text = TextRenderer::load(&theme.fonts);
    let mut c = Canvas::new(s.size.0 as u32, s.size.1 as u32);
    c.fill_rect(c.bounds(), theme.colors.button_face);
    draw(&mut c, &text, &theme.paint(), s, false, theme.fonts.size_px);
    c
}
