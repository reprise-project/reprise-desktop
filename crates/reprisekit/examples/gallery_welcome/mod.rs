//! Welcome's three pages under Classic, Blue and Glass, written to
//! `target/reprisekit-gallery-welcome@<scale>.png`.

use crate::interfaces;
use reprisekit::controls::welcome::{self, Link, Page, Picks, Welcome};
use reprisekit::interface::Interface;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::{Align, TextRenderer};
use reprisekit::theme::{Color, FontConfig};

pub fn render(s: i32) -> Canvas {
    let looks = [
        ("Classic", Interface::Classic),
        ("Blue", Interface::Blue),
        ("Glass", Interface::Glass),
    ];
    let grown = looks.map(|(_, i)| interfaces::theme_for(i).text.size(welcome::CLIENT));
    let cw = grown.iter().map(|g| g.0).max().unwrap_or(welcome::CLIENT.0);
    let ch = grown.iter().map(|g| g.1).max().unwrap_or(welcome::CLIENT.1);
    let pages = [Page::Interface, Page::Look, Page::Ready];
    let label_w = 70;
    let (w, h) = (
        label_w + pages.len() as i32 * (cw + 10),
        looks.len() as i32 * (ch + 10),
    );
    let mut c = super::blurred_stripes(w * s, h * s);
    let head = TextRenderer::load(&FontConfig::default());
    for (i, (name, interface)) in looks.into_iter().enumerate() {
        let y = i as i32 * (ch + 10);
        head.draw(
            &mut c,
            Rect::new(4, y + 4, label_w - 8, 16).scaled(s),
            name,
            true,
            Color::rgb(0xFF, 0xFF, 0xFF),
            Align::Left,
            11 * s as u32,
        );
        let base = super::cued(interfaces::theme_for(interface));
        let theme = base.scaled(s);
        let text = TextRenderer::load(&theme.fonts);
        let picks = Picks {
            interface,
            dark: false,
            text_size: 100,
        };
        let links = vec![Link {
            id: "settings",
            label: "Open &Settings".into(),
        }];
        let sample = super::cued(reprisekit::interfaces::theme_for(interface));
        let mut wiz = Welcome::new(picks, sample, links, &text, &theme);
        for (j, page) in pages.into_iter().enumerate() {
            wiz.go(page, &text, &theme);
            let (dw, dh) = base.text.size(welcome::CLIENT);
            let mut d = Canvas::new((dw * s) as u32, (dh * s) as u32);
            welcome::draw(
                &mut d,
                &text,
                &theme.paint(),
                &wiz,
                true,
                theme.fonts.size_px,
            );
            c.blit_canvas((label_w + j as i32 * (cw + 10)) * s, y * s, &d);
        }
    }
    c
}
