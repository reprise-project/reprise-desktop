//! Mini views in Classic, Blue and Glass light and dark, at rest and with the pointer on them
//! (close box shown), over a stand-in picture, written to
//! `target/reprisekit-gallery-mini-view@<scale>.png`.

use reprisekit::controls::miniview;
use reprisekit::interface::Interface;
use reprisekit::interfaces;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::theme::{Color, Theme};

const GAP: i32 = 24;
const SIZE: (i32, i32) = (240, 186);

fn looks() -> Vec<Theme> {
    let mut out = vec![
        Theme::default(),
        interfaces::blue_theme(reprisekit::blue::Variant::ALL[0]),
    ];
    for mode in [
        reprisekit::glass::Mode::Light,
        reprisekit::glass::Mode::Dark,
    ] {
        let g = reprisekit::glass::Settings {
            mode,
            ..Default::default()
        };
        out.push(interfaces::theme_with(
            Interface::Glass,
            Default::default(),
            &g,
        ));
    }
    out
}

/// A window's picture letterboxed into `r` as the host's would be: a light page with dark lines.
fn picture(c: &mut Canvas, r: Rect, s: i32) {
    let page = Rect::new(r.x, r.y + 8 * s, r.width, r.height - 16 * s);
    c.fill_rect(page, Color::rgb(0xF4, 0xF4, 0xF0));
    for i in 0..8 {
        let line = Rect::new(
            page.x + 8 * s,
            page.y + (10 + i * 16) * s,
            (120 + i * 9) * s,
            3 * s,
        );
        c.fill_rect(line, Color::rgb(0x40, 0x48, 0x58));
    }
}

pub fn render(scale: i32) -> Canvas {
    let looks = looks();
    let cell = (SIZE.0 + 2 * GAP, SIZE.1 + 2 * GAP);
    let mut c = Canvas::new(
        (cell.0 * looks.len() as i32 * scale) as u32,
        (cell.1 * 2 * scale) as u32,
    );
    c.fill_rect(c.bounds(), Color::rgb(0x3A, 0x6E, 0xA5));
    for (col, theme) in looks.iter().enumerate() {
        let theme = theme.scaled(scale);
        let look = miniview::look(&theme);
        let l = miniview::layout(look, SIZE);
        for row in 0..2 {
            let at = (
                (col as i32 * cell.0 + GAP) * scale,
                (row * cell.1 + GAP) * scale,
            );
            let frame = miniview::draw_frame(&theme, &l);
            let m = look.margin;
            c.blit_over(at.0 - m.left * scale, at.1 - m.top * scale, &frame);
            let p = l.picture.scaled(scale);
            picture(
                &mut c,
                Rect::new(at.0 + p.x, at.1 + p.y, p.width, p.height),
                scale,
            );
            if row == 1 {
                c.blit_over(at.0, at.1, &miniview::draw_controls(&theme, &l));
            }
        }
    }
    c
}
