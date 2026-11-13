//! Welcome's three pages under Classic, Blue and Glass, written to
//! `target/reprisekit-gallery-welcome@<scale>.png`.

use reprisekit::controls::welcome::{self, Link, Page, Picks, Welcome};
use reprisekit::interface::Interface;
use reprisekit::interfaces;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::{Align, TextRenderer};
use reprisekit::theme::{Color, FontConfig};

pub fn render(s: i32) -> Canvas {
    let (cw, ch) = welcome::CLIENT;
    let looks = [
        ("Classic", Interface::Classic),
        ("Blue", Interface::Blue),
        ("Glass", Interface::Glass),
    ];
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
            label: "Open &Settings",
        }];
        let mut wiz = Welcome::new(picks, base.clone(), links, &text, &theme);
        for (j, page) in pages.into_iter().enumerate() {
            wiz.go(page, &text, &theme);
            let mut d = Canvas::new((cw * s) as u32, (ch * s) as u32);
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
