//! The Fingerprints page in each state and the Add a Fingerprint dialog in each step, in every
//! look, written to `target/reprisekit-gallery-fingerprints@<scale>.png`.

use reprisekit::controls::addfinger::{AddFinger, Refusal, Scan, Scanned};
use reprisekit::controls::fingerprints::{self, Facts, Finger, Reader};
use reprisekit::controls::form::{self, Event, Form, Key};
use reprisekit::interface::Interface;
use reprisekit::interfaces;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::{Align, TextRenderer};
use reprisekit::theme::{Color, FontConfig, Theme};

/// The width of an account sheet's page inside its margins.
const W: i32 = 366;
/// Room around each form.
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

fn yours(reader: Reader) -> Facts {
    Facts {
        reader,
        yours: true,
        busy: false,
    }
}

fn found(fingers: &[Finger]) -> Reader {
    Reader::Found {
        name: "Synaptics Sensors".into(),
        fingers: fingers.to_vec(),
    }
}

/// Each page state and dialog step as a form at `(M, M)`, with the canvas it draws on.
fn forms() -> Vec<(Form, (i32, i32))> {
    let pages = [
        yours(Reader::NoService),
        yours(Reader::NoReader),
        yours(found(&[])),
        yours(found(&[Finger::LeftThumb, Finger::RightIndex])),
        Facts {
            yours: false,
            ..yours(found(&[Finger::RightIndex]))
        },
    ];
    let mut out: Vec<(Form, (i32, i32))> = pages
        .iter()
        .map(|f| {
            let s = fingerprints::section(f);
            let (items, bottom) = fingerprints::items(&s, M, M, W);
            let mut form = Form::new(items);
            if s.fingers.len() == 2 {
                form.focus_id(fingerprints::LIST);
                form.key(Key::Down);
                fingerprints::follow(&mut form, &s);
            }
            (form, (W + 2 * M, bottom + M))
        })
        .collect();
    let dialog = || AddFinger::new(vec![Finger::LeftThumb], Scan::Press, 5, 1);
    let mut steps = vec![dialog()];
    let mut scanning = dialog();
    scanning.event(Event::Default);
    scanning.started();
    scanning.scanned(Scanned::Passed);
    scanning.scanned(Scanned::Passed);
    scanning.scanned(Scanned::NotCentred);
    steps.push(scanning);
    let mut added = dialog();
    added.event(Event::Default);
    added.started();
    added.scanned(Scanned::Completed);
    steps.push(added);
    let mut failed = dialog();
    failed.event(Event::Default);
    failed.refused(Refusal::NotAuthorized);
    steps.push(failed);
    out.extend(steps.into_iter().map(|d| {
        let size = d.client();
        (d.form, size)
    }));
    out
}

pub fn render(s: i32) -> Canvas {
    let forms = forms();
    let looks = looks();
    let label_w = 90;
    let row_h = forms.iter().map(|f| f.1.1).max().unwrap_or(0) + 10;
    let w = label_w + forms.iter().map(|f| f.1.0 + 10).sum::<i32>();
    let h = looks.len() as i32 * row_h;
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
        let mut x = label_w;
        for (f, (fw, fh)) in &forms {
            let mut d = Canvas::new((fw * s) as u32, (fh * s) as u32);
            d.fill_rect(d.bounds(), p.cs.button_face);
            form::draw(&mut d, &text, &p, f, true, theme.fonts.size_px);
            c.blit_canvas(x * s, y * s, &d);
            x += fw + 10;
        }
    }
    c
}
