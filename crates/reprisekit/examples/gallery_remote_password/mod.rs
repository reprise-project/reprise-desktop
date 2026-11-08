//! The Remote Desktop password section of an account's Password page in each state, in
//! Classic, Blue and Glass light and dark, written to
//! `target/reprisekit-gallery-remote-password@<scale>.png`.

use reprisekit::controls::form::{self, Form};
use reprisekit::controls::remotepassword::{self, Access, Answer, Facts, Manager};
use reprisekit::interface::Interface;
use reprisekit::interfaces;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::{Align, TextRenderer};
use reprisekit::theme::{Color, FontConfig, Theme};

/// The width of an account sheet's page inside its margins.
const W: i32 = 366;
/// Room around the group box, so its title and frame draw whole.
const M: i32 = 8;

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

fn states() -> Vec<Facts> {
    let yours = |has: bool| Facts {
        manager: Manager::Running(Answer::Has(has)),
        yours: true,
        access: Access::Allowed,
        busy: false,
    };
    vec![
        yours(false),
        yours(true),
        Facts {
            access: Access::NotInGroup("reprise-remote".into()),
            ..yours(false)
        },
        Facts {
            access: Access::NotInGroup("reprise-remote".into()),
            ..yours(true)
        },
        Facts {
            access: Access::RemoteLoginsOff,
            ..yours(false)
        },
        Facts {
            access: Access::Root,
            ..yours(false)
        },
        Facts {
            manager: Manager::Stopped,
            ..yours(false)
        },
        Facts {
            manager: Manager::Running(Answer::Silent),
            ..yours(false)
        },
        Facts {
            yours: false,
            ..yours(true)
        },
    ]
}

pub fn render(s: i32) -> Canvas {
    let forms: Vec<(Form, i32)> = states()
        .iter()
        .map(|f| {
            let shown = remotepassword::section(f).expect("shown");
            let (items, bottom) = remotepassword::items(&shown, M, M, W);
            (Form::new(items), bottom + M)
        })
        .collect();
    let looks = looks();
    let label_w = 90;
    let row_h = forms.iter().map(|f| f.1).max().unwrap_or(0) + 10;
    let (w, h) = (
        label_w + forms.len() as i32 * (W + 2 * M + 10),
        looks.len() as i32 * row_h,
    );
    let mut c = super::blurred_stripes(w * s, h * s);
    let head = TextRenderer::load(&FontConfig::default());
    for (i, (name, base)) in looks.iter().enumerate() {
        let y = i as i32 * row_h;
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
        for (j, (f, bottom)) in forms.iter().enumerate() {
            let mut d = Canvas::new(((W + 2 * M) * s) as u32, (bottom * s) as u32);
            d.fill_rect(d.bounds(), p.cs.button_face);
            form::draw(&mut d, &text, &p, f, true, theme.fonts.size_px);
            c.blit_canvas((label_w + j as i32 * (W + 2 * M + 10)) * s, y * s, &d);
        }
    }
    c
}
