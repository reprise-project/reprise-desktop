//! Sticky notes in every colour under Classic, Blue and Glass, the first with its New button lit
//! and the second with Delete held, written to `target/reprisekit-gallery-notes@<scale>.png`.

use reprisekit::controls::multiedit::{self, MultiEdit, View};
use reprisekit::controls::stickynote::{self, Colour, Part, Strip};
use reprisekit::interface::Interface;
use reprisekit::interfaces;
use reprisekit::paint::Paint;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::{Align, TextRenderer};
use reprisekit::textedit::Caret;
use reprisekit::theme::{Color, FontConfig};

const NOTE: (i32, i32) = (150, 110);

pub fn render(s: i32) -> Canvas {
    let looks = [Interface::Classic, Interface::Blue, Interface::Glass];
    let label_w = 70;
    let (w, h) = (
        label_w + Colour::ALL.len() as i32 * (NOTE.0 + 10),
        looks.len() as i32 * (NOTE.1 + 10),
    );
    let mut c = super::blurred_stripes(w * s, h * s);
    let head = TextRenderer::load(&FontConfig::default());
    for (i, look) in looks.into_iter().enumerate() {
        let y = i as i32 * (NOTE.1 + 10);
        head.draw(
            &mut c,
            Rect::new(4, y + 4, label_w - 8, 16).scaled(s),
            &format!("{look:?}"),
            true,
            Color::rgb(0xFF, 0xFF, 0xFF),
            Align::Left,
            11 * s as u32,
        );
        let mut theme = interfaces::theme_for(look);
        theme.look = theme.look.at_scale(s);
        let text = TextRenderer::load(&theme.fonts);
        for (j, colour) in Colour::ALL.into_iter().enumerate() {
            let strip = match j {
                0 => Strip {
                    hot: Some(Part::New),
                    held: None,
                },
                1 => Strip {
                    hot: Some(Part::Delete),
                    held: Some(Part::Delete),
                },
                _ => Strip::default(),
            };
            let cs = stickynote::scheme(&theme.colors, colour);
            let p = Paint {
                cs: &cs,
                s: 1,
                ..theme.paint()
            };
            let mut d = Canvas::with_scale(NOTE.0 as u32, NOTE.1 as u32, s);
            let r = Rect::new(0, 0, NOTE.0, NOTE.1);
            stickynote::draw(&mut d, &p, r, colour, strip);
            let m = multiedit::Metrics {
                font_px: 13,
                line_h: 16,
                bars_always: false,
                flat: true,
                wrap: true,
                bar: theme.look.controls().scroll,
                thumb_min: theme.look.controls().thumb_min,
                ..Default::default()
            };
            let lines = if j == 2 {
                "Milk\nBread\nEggs\nCoffee\nRice\nTea\nApples\nSoap"
            } else {
                "Call the plumber about the sink before Friday"
            };
            let edit = MultiEdit::new(lines);
            let lay = multiedit::layout(&text, &m, &edit, stickynote::layout(r).text, 1);
            multiedit::draw(
                &mut d,
                &text,
                &p,
                &m,
                &edit,
                View::default(),
                &lay,
                Caret::new(j == 0, true),
                None,
            );
            let d = d.at_scale(1);
            c.blit_canvas((label_w + j as i32 * (NOTE.0 + 10)) * s, y * s, &d);
        }
    }
    c
}
