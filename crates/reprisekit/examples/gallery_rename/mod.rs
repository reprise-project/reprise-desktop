//! The Rename dialog for several items in Classic, Blue and Glass light and dark, replacing with
//! a clash and numbering, written to `target/reprisekit-gallery-rename@<scale>.png`.

use reprisekit::controls::batchrename::{self, Fields, Row};
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
    let fields = Fields {
        number: false,
        find: "IMG_".into(),
        with: "Holiday ".into(),
        ignore_case: true,
        base: "Photo".into(),
        keep_ext: true,
    };
    let names = ["IMG_0041.jpg", "img_0042.jpg", "IMG_0043.png", "notes.txt"];
    let mut replace = batchrename::form(&fields);
    let rows: Vec<Row> = names
        .iter()
        .enumerate()
        .map(|(i, old)| Row {
            old: old.to_string(),
            new: old.replace("IMG_", "Holiday ").replace("img_", "Holiday "),
            problem: (i == 2).then(|| "in use".to_string()),
        })
        .collect();
    batchrename::set_preview(&mut replace, &rows, false);
    let mut number = batchrename::form(&Fields {
        number: true,
        ..fields
    });
    let rows: Vec<Row> = names
        .iter()
        .enumerate()
        .map(|(i, old)| {
            let ext = old.rsplit_once('.').map_or("", |(_, e)| e);
            Row {
                old: old.to_string(),
                new: format!("Photo ({}).{ext}", i + 1),
                problem: None,
            }
        })
        .collect();
    batchrename::set_preview(&mut number, &rows, true);
    vec![replace, number]
}

pub fn render(s: i32) -> Canvas {
    let (cw, ch) = batchrename::CLIENT;
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
