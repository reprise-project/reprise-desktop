//! Document tab strips in every look, Blue scheme and state, Glass light and dark, written to
//! `target/reprisekit-gallery-doc-tabs@<scale>.png`.

use reprisekit::controls::tabstrip::{self, AdderPart, Drag, TabStrip};
use reprisekit::interface::Interface;
use reprisekit::interfaces;
use reprisekit::look;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::{Align, TextRenderer};
use reprisekit::theme::{Color, FontConfig, TextUnits, Theme};

const STRIP_W: i32 = 330;
const ROW_H: i32 = 40;

fn states() -> Vec<(&'static str, TabStrip)> {
    let labels = ["~/projects", "htop", "ssh box"].map(String::from).to_vec();
    let base = TabStrip::document(labels, 0);
    let with = |f: &dyn Fn(&mut TabStrip)| {
        let mut s = base.clone();
        f(&mut s);
        s
    };
    vec![
        ("open, closed", base.clone()),
        ("hot tab", with(&|s| s.hot = Some(1))),
        (
            "hot close",
            with(&|s| {
                s.hot = Some(2);
                s.hot_close = Some(2);
            }),
        ),
        (
            "held close",
            with(&|s| {
                s.hot_close = Some(0);
                s.held = true;
            }),
        ),
        ("+ hot", with(&|s| s.hot_adder = Some(AdderPart::Add))),
        (
            "menu held",
            with(&|s| {
                s.hot_adder = Some(AdderPart::Menu);
                s.held = true;
            }),
        ),
        (
            "dragged",
            with(&|s| {
                s.dragging = Some(Drag {
                    tab: 0,
                    grab: 20,
                    x: 110,
                })
            }),
        ),
        (
            "many",
            TabStrip::document((0..12).map(|i| format!("shell {i}")).collect(), 3),
        ),
    ]
}

fn looks() -> Vec<(String, Theme, bool)> {
    let mut out = vec![("Classic".to_string(), Theme::default(), false)];
    for v in reprisekit::blue::Variant::ALL {
        out.push((v.name().to_string(), interfaces::blue_theme(v), false));
    }
    for mode in [
        reprisekit::glass::Mode::Light,
        reprisekit::glass::Mode::Dark,
    ] {
        let g = reprisekit::glass::Settings {
            mode,
            ..Default::default()
        };
        let t = interfaces::theme_with(Interface::Glass, Default::default(), &g);
        out.push((format!("Glass {mode:?}"), t, true));
    }
    out
}

pub fn render(s: i32) -> Canvas {
    let looks = looks();
    let states = states();
    let label_w = 90;
    let (w, h) = (
        label_w + states.len() as i32 * (STRIP_W + 10),
        20 + looks.len() as i32 * ROW_H,
    );
    let mut c = super::blurred_stripes(w * s, h * s);
    let head = TextRenderer::load(&FontConfig::default());
    for (j, (name, _)) in states.iter().enumerate() {
        let r = Rect::new(label_w + j as i32 * (STRIP_W + 10), 2, STRIP_W, 16).scaled(s);
        head.draw(
            &mut c,
            r,
            name,
            true,
            Color::rgb(0xFF, 0xFF, 0xFF),
            Align::Left,
            11 * s as u32,
        );
    }
    for (i, (name, base, on_glass)) in looks.iter().enumerate() {
        let y = 20 + i as i32 * ROW_H;
        let r = Rect::new(4, y, label_w - 8, 22).scaled(s);
        head.draw(
            &mut c,
            r,
            name,
            true,
            Color::rgb(0xFF, 0xFF, 0xFF),
            Align::Left,
            11 * s as u32,
        );
        let one = TextRenderer::load(&base.fonts);
        let theme = base.scaled(s);
        let text = TextRenderer::load(&theme.fonts);
        let p = theme.paint();
        for (j, (_, strip)) in states.iter().enumerate() {
            let x = label_w + j as i32 * (STRIP_W + 10);
            let area = Rect::new(x, y, STRIP_W, tabstrip::doc_height(TextUnits::ONE));
            let client = Rect::new(x, y, STRIP_W, ROW_H - 4);
            if *on_glass {
                look::draw_client_glass(&mut c, &theme, client.scaled(s), client.height * s, false);
            } else {
                c.fill_rect(client.scaled(s), p.cs.button_face);
            }
            let term = Rect::new(x, area.bottom(), STRIP_W, client.bottom() - area.bottom());
            c.fill_rect(term.scaled(s), Color::rgb(0x0C, 0x0C, 0x0C));
            let l = tabstrip::doc_layout(&one, base.fonts.size_px, strip, area, TextUnits::ONE);
            tabstrip::doc_draw(&mut c, &text, &p, strip, &l, *on_glass, theme.fonts.size_px);
        }
    }
    c
}
