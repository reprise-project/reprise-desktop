//! The font viewer in Classic, Blue and Glass light and dark, one face installable and a
//! collection installed for every user, written to `target/reprisekit-gallery-fonts@<scale>.png`.
//! The sample is the Tahoma `REPRISE_TEST_FONTS` holds; without it the wells stay empty.

use reprisekit::controls::fontview::{self, Install};
use reprisekit::interface::Interface;
use reprisekit::interfaces;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::fontfile::{self, FaceNames};
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

pub fn render(s: i32) -> Canvas {
    let file = std::env::var_os("REPRISE_TEST_FONTS")
        .map(|d| std::path::PathBuf::from(d).join("tahoma.ttf"));
    let sample = file.as_deref().and_then(|f| TextRenderer::from_file(f, 0));
    let mut faces: Vec<FaceNames> = file
        .as_deref()
        .and_then(fontfile::faces)
        .unwrap_or_default();
    if faces.is_empty() {
        faces.push(FaceNames {
            family: "Sample".into(),
            ..Default::default()
        });
    }
    let mut collection = faces.clone();
    collection.push(FaceNames {
        index: 1,
        style: "Bold".into(),
        full_name: format!("{} Bold", faces[0].family),
        ..faces[0].clone()
    });
    let forms = [
        fontview::form(&faces, 0, &Install::No),
        fontview::form(&collection, 1, &Install::System),
    ];
    let (cw, ch) = fontview::CLIENT;
    let looks = looks();
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
        for (j, f) in forms.iter().enumerate() {
            let mut d = Canvas::new((cw * s) as u32, (ch * s) as u32);
            fontview::draw(
                &mut d,
                &theme.paint(),
                &text,
                theme.fonts.size_px,
                f,
                sample.as_ref(),
            );
            c.blit_canvas((label_w + j as i32 * (cw + 10)) * s, y * s, &d);
        }
    }
    c
}
