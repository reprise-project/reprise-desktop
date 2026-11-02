//! A file's Checksums page in Classic, Blue and Glass light and dark, calculating and done with a
//! matching hash pasted, written to `target/reprisekit-gallery-checksums@<scale>.png`.

use reprisekit::controls::filesheet::{self, SumsState};
use reprisekit::controls::form;
use reprisekit::interface::Interface;
use reprisekit::interfaces;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::{Align, TextRenderer};
use reprisekit::theme::{Color, FontConfig, Theme};

fn looks() -> Vec<(String, Theme)> {
    let mut out = vec![
        ("Classic".to_string(), Theme::default()),
        (
            "Blue".to_string(),
            interfaces::blue_theme(reprisekit::blue::Variant::ALL[0]),
        ),
    ];
    for mode in [
        reprisekit::glass::Mode::Light,
        reprisekit::glass::Mode::Dark,
    ] {
        let g = reprisekit::glass::Settings {
            mode,
            ..Default::default()
        };
        let t = interfaces::theme_with(Interface::Glass, Default::default(), &g);
        out.push((format!("Glass {mode:?}"), t));
    }
    out
}

fn forms() -> Vec<form::Form> {
    let mut working = filesheet::checksums_page();
    filesheet::show_sums(&mut working, &SumsState::Working(42));
    let mut done = filesheet::checksums_page();
    filesheet::show_sums(
        &mut done,
        &SumsState::Done {
            sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".into(),
            md5: "900150983cd24fb0d6963f7d28e17f72".into(),
        },
    );
    filesheet::show_verdict(&mut done, "Matches SHA-256");
    vec![working, done]
}

pub fn render(s: i32) -> Canvas {
    let (cw, ch) = (filesheet::SHEET_W, 270);
    let looks = looks();
    let forms = forms();
    let label_w = 90;
    let (w, h) = (
        label_w + forms.len() as i32 * (cw + 10),
        looks.len() as i32 * (ch + 10),
    );
    let mut c = super::blurred_stripes(w * s, h * s);
    let head = TextRenderer::load(&FontConfig::default());
    for (i, (name, base)) in looks.iter().enumerate() {
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
        let theme = base.scaled(s);
        let text = TextRenderer::load(&theme.fonts);
        let p = theme.paint();
        for (j, f) in forms.iter().enumerate() {
            let mut d = Canvas::new((cw * s) as u32, (ch * s) as u32);
            d.fill_rect(d.bounds(), p.cs.button_face);
            form::draw(&mut d, &text, &p, f, true, theme.fonts.size_px);
            c.blit_canvas((label_w + j as i32 * (cw + 10)) * s, y * s, &d);
        }
    }
    c
}
