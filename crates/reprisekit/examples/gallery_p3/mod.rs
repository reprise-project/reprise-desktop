//! Blue's colour variants side by side, written to `target/reprisekit-gallery-blue-variants.png`: one column per variant, one band per surface. Each band is a file here with a `HEIGHT` and a `draw`.

mod chrome;
mod controls;
mod explorerbar;
mod header;
pub(crate) mod menu;
mod rebar;
mod statusbar;
mod taskpane;
mod toolbar;
mod tooltip;

use reprisekit::blue::Variant;
use reprisekit::interfaces;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::{Align, TextRenderer};
use reprisekit::theme::{Color, FontConfig, Theme};

struct Band {
    title: &'static str,
    height: i32,
    draw: fn(&mut Canvas, &Theme, &TextRenderer, Rect),
}

const BANDS: [Band; 10] = [
    Band {
        title: "frames and taskbar",
        height: chrome::HEIGHT,
        draw: chrome::draw,
    },
    Band {
        title: "common controls",
        height: controls::HEIGHT,
        draw: controls::draw,
    },
    Band {
        title: "toolbar",
        height: toolbar::HEIGHT,
        draw: toolbar::draw,
    },
    Band {
        title: "rebar",
        height: rebar::HEIGHT,
        draw: rebar::draw,
    },
    Band {
        title: "list header",
        height: header::HEIGHT,
        draw: header::draw,
    },
    Band {
        title: "status bar",
        height: statusbar::HEIGHT,
        draw: statusbar::draw,
    },
    Band {
        title: "tooltip",
        height: tooltip::HEIGHT,
        draw: tooltip::draw,
    },
    Band {
        title: "menus",
        height: menu::HEIGHT,
        draw: menu::draw,
    },
    Band {
        title: "task pane",
        height: taskpane::HEIGHT,
        draw: taskpane::draw,
    },
    Band {
        title: "explorer bar",
        height: explorerbar::HEIGHT,
        draw: explorerbar::draw,
    },
];

const COLUMN: i32 = 640;
const TITLE: i32 = 16;

pub fn render() -> Canvas {
    let text = TextRenderer::load(&FontConfig::default());
    let height: i32 = BANDS.iter().map(|b| TITLE + b.height).sum();
    let mut c = Canvas::new(COLUMN as u32 * Variant::ALL.len() as u32, height as u32);
    for (i, v) in Variant::ALL.into_iter().enumerate() {
        let theme = interfaces::blue_theme(v);
        let x = i as i32 * COLUMN;
        let mut y = 0;
        for b in &BANDS {
            let title = Rect::new(x, y, COLUMN, TITLE);
            c.fill_rect(title, Color::rgb(0xFF, 0xFF, 0xFF));
            text.draw(
                &mut c,
                Rect::new(x + 4, y + 2, COLUMN - 8, TITLE - 2),
                &format!("{}: {}", v.name(), b.title),
                false,
                Color::rgb(0, 0, 0),
                Align::Left,
                theme.fonts.size_px,
            );
            let r = Rect::new(x, y + TITLE, COLUMN, b.height);
            c.fill_rect(r, theme.colors.button_face);
            (b.draw)(&mut c, &theme, &text, r);
            y += TITLE + b.height;
        }
    }
    c
}
