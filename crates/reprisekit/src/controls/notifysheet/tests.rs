use super::draw::{draw_parts, relight};
use super::glass::sheet_spec;
use super::opening::{glass_at, opening_fades};
use super::stack::{BEAD_FADE, BEAD_UNTIL};
use super::*;
use crate::access::ROOT;
use crate::access::accesskit::{Live, Role};
use crate::controls::tooltip::{NOTIFY_BODY_LINES, NOTIFY_BUTTONS, notify_layout};
use crate::interface::Interface;
use crate::interfaces::{theme_for, theme_with};
use crate::raster::Canvas;
use crate::shape::scale;
use crate::taskbar::Edge;
use crate::textedit::{Caret, TextEdit};
use crate::theme::Color;
use crate::theme::FontConfig;
use crate::{blue, glass};

mod droplets;
mod halos;
mod lozenges;
mod pills;
mod placement;
mod repaint;
mod text_size;

fn text() -> TextRenderer {
    TextRenderer::load(&FontConfig::default())
}

fn content<'a>(app: &'a str, title: &'a str, body: &'a str) -> SheetContent<'a> {
    SheetContent {
        app,
        when: "now",
        title,
        body,
        actions: &[],
        action_icons: &[],
        reply: None,
        has_picture: false,
        has_app_icon: true,
    }
}

fn lozenge(c: &SheetContent, expanded: bool) -> SheetLayout {
    sheet_layout(
        &theme_for(Interface::Glass),
        &text(),
        SheetShape::Lozenge,
        c,
        expanded,
    )
    .unwrap()
}

fn glass(dark: bool, scale: i32) -> Theme {
    let g = glass::Settings {
        mode: if dark {
            glass::Mode::Dark
        } else {
            glass::Mode::Light
        },
        ..glass::Settings::default()
    };
    theme_with(Interface::Glass, blue::Variant::Blue, &g).scaled(scale)
}

fn fnv(c: &Canvas) -> u64 {
    [c.width, c.height]
        .iter()
        .chain(c.pixels.iter())
        .fold(0xcbf2_9ce4_8422_2325_u64, |h, &p| {
            p.to_le_bytes().iter().fold(h, |h, &b| {
                (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
            })
        })
}

const REPLY: NotifyReply = NotifyReply {
    placeholder: "Write a reply",
    send: "Send",
};

fn swatch(c: Color, side: i32) -> Canvas {
    let mut k = Canvas::new(side as u32, side as u32);
    k.fill_rect(Rect::new(0, 0, side, side), c);
    k
}

fn pin_case(case: usize, theme: &Theme, t: &TextRenderer) -> Canvas {
    let s = theme.scale;
    let acts = ["Reply", "Later"];
    let base = content("Chat", "Ana", "Lunch at noon? The usual place.");
    let (c, expanded, hot) = match case {
        0 => (base, false, None),
        1 => (base, false, Some(NotifyHit::Close)),
        2 => (
            SheetContent {
                actions: &acts,
                has_picture: true,
                ..base
            },
            false,
            Some(NotifyHit::Button(0)),
        ),
        _ => (
            SheetContent {
                actions: &acts,
                reply: Some(REPLY),
                ..base
            },
            true,
            Some(NotifyHit::Field),
        ),
    };
    let lay = sheet_layout(theme, t, SheetShape::Lozenge, &c, expanded).unwrap();
    let app = swatch(Color::rgb(0x2E, 0x7D, 0xE0), 46 * s);
    let picture = swatch(Color::rgb(0xE0, 0x6A, 0x2E), 46 * s);
    let icons = SheetIcons {
        app: Some(&app),
        picture: c.has_picture.then_some(&picture),
        actions: &[],
    };
    let typed = TextEdit::new("Sounds good", false);
    let reply = (case == 3).then_some((&typed, Caret::Shown, None));
    draw_sheet(theme, t, &lay, icons, None, None, hot, reply)
}

fn halo(c: &SheetContent) -> SheetLayout {
    sheet_layout(
        &theme_for(Interface::Glass),
        &text(),
        SheetShape::Halo,
        c,
        false,
    )
    .unwrap()
}

fn on_backdrop(c: &Canvas) -> Canvas {
    let mut bg = Canvas::new(c.width, c.height);
    let w = c.width as i32;
    bg.fill_rect(
        Rect::new(0, 0, w / 2, c.height as i32),
        Color::rgb(0x3A, 0x6E, 0xA5),
    );
    bg.fill_rect(
        Rect::new(w / 2, 0, w - w / 2, c.height as i32),
        Color::rgb(0xC8, 0xD4, 0xE0),
    );
    bg.blit_canvas(0, 0, c);
    bg
}

const LONG_ACTS: [&str; 2] = ["Reply", "Later"];

fn long() -> SheetContent<'static> {
    SheetContent {
        actions: &LONG_ACTS,
        ..content(
            "Chat",
            "Ana",
            "Lunch at noon?\nThe usual place by the river.\nBring the photos.",
        )
    }
}

const LONG_PILLS: [&str; 4] = [
    "Open the disk",
    "Show Disk Usage",
    "Empty Recycle Bin",
    "Remind me tomorrow",
];

const SHAPES: [SheetShape; 3] = [SheetShape::Lozenge, SheetShape::Halo, SheetShape::Droplet];
