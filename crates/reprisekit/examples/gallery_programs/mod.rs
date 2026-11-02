//! Calculator in Classic, Blue and Glass light and dark, Standard and Scientific with its
//! history, written to `target/reprisekit-gallery-calculator@<scale>.png`.

use reprisekit::controls::calculator::{self, Calc, Input, Mode};
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

fn fed(mode: Mode, keys: &str) -> Calc {
    let mut calc = Calc::new(mode);
    for ch in keys.chars() {
        let sym = if ch == '\n' { 0xff0d } else { ch as u32 };
        if let Some(i) = calculator::input_for(sym, Some(ch).filter(|c| *c != '\n'), false) {
            calc.input(i);
        }
    }
    calc
}

/// Each look's row: Standard with an operation in progress and the memory set, Scientific
/// with its history.
pub fn calculator(s: i32) -> Canvas {
    let mut standard = fed(Mode::Standard, "1250*3+");
    standard.input(Input::MemStore);
    let scientific = fed(Mode::Scientific, "2^10\n(1+2)*3\n30s");
    let shown = [(standard, false), (scientific, true)];
    let label_w = 90;
    let widths: Vec<i32> = shown
        .iter()
        .map(|(c, h)| calculator::client(c.mode, *h).0)
        .collect();
    let ch = calculator::client(Mode::Standard, false).1;
    let looks = looks();
    let w = label_w + widths.iter().map(|w| w + 10).sum::<i32>();
    let h = looks.len() as i32 * (ch + 10);
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
        let mut x = label_w;
        for ((calc, history), cw) in shown.iter().zip(&widths) {
            let f = calculator::form(calc, *history);
            let mut d = Canvas::new((cw * s) as u32, (ch * s) as u32);
            d.fill_rect(d.bounds(), p.cs.button_face);
            calculator::draw(&mut d, &text, &p, &f, calc, theme.fonts.size_px);
            c.blit_canvas(x * s, y * s, &d);
            x += cw + 10;
        }
    }
    c
}
