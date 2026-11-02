//! The performance graph and meters in Classic, Blue and Glass light and dark, written to
//! `target/reprisekit-gallery-perf@<scale>.png`.

use reprisekit::controls::perfgraph::{self, History};
use reprisekit::interface::Interface;
use reprisekit::interfaces;
use reprisekit::look::Paint;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::TextRenderer;
use reprisekit::theme::Theme;

fn looks() -> Vec<Theme> {
    let mut out = vec![Theme::default(), interfaces::theme_for(Interface::Blue)];
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

pub fn render(scale: i32) -> Canvas {
    let mut cpu = History::new(100);
    let mut mem = History::new(100);
    let mut swap = History::new(100);
    for i in 0..140 {
        let t = f64::from(i);
        cpu.push(0.35 + (t * 0.21).sin() * 0.25 + (t * 0.05).cos() * 0.1);
        mem.push(0.55 + t * 0.001);
        swap.push(0.08);
    }
    let (w, h) = (380, 120);
    let looks = looks();
    let mut c = Canvas::with_scale(w as u32, (h * looks.len() as i32) as u32, scale);
    for (i, theme) in looks.iter().enumerate() {
        let look = theme.look.at_scale(scale);
        let p = Paint {
            look: &look,
            ..theme.paint()
        };
        let text = TextRenderer::load(&theme.fonts);
        let px = theme.fonts.size_px;
        let y = i as i32 * h;
        c.fill_rect(Rect::new(0, y, w, h), theme.colors.button_face);
        perfgraph::draw_meter(
            &mut c,
            &text,
            &p,
            Rect::new(8, y + 8, 60, 104),
            0.42,
            "42 %",
            px,
        );
        perfgraph::draw(&mut c, &p, Rect::new(76, y + 8, 140, 104), &[&cpu]);
        perfgraph::draw(&mut c, &p, Rect::new(224, y + 8, 148, 104), &[&mem, &swap]);
    }
    c
}
