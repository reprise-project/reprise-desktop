//! The desktop widgets in each look: the analog and digital clock, the month calendar and the
//! system meter's bars and graph, written to `target/reprisekit-gallery-desk-widgets@<scale>.png`.

use reprisekit::controls::calendar::Calendar;
use reprisekit::controls::deskwidget::{self, Clock, Content, Face, Meter, MeterStyle};
use reprisekit::interface::Interface;
use reprisekit::interfaces::theme_for;
use reprisekit::look::panel;
use reprisekit::raster::Canvas;
use reprisekit::text::TextRenderer;

const LOOKS: [Interface; 3] = [Interface::Classic, Interface::Blue, Interface::Glass];

fn samples() -> Vec<Content> {
    let mut cal = Calendar::new(2026, 11, None, 1);
    cal.today = Some((2026, 11, 2));
    let meter = |style| {
        let mut m = Meter::new(style);
        for i in 0..deskwidget::HISTORY {
            let wave = ((i as f64) / 6.0).sin() * 0.3 + 0.4;
            m.push(Some(wave), Some(0.42));
        }
        Content::Meter(m)
    };
    vec![
        Content::Clock(Clock::new(Face::Analog, true, (10, 9, 30), "", "")),
        Content::Clock(Clock::new(
            Face::Digital,
            false,
            (15, 39, 0),
            "15:39",
            "Monday, November 2, 2026",
        )),
        Content::Calendar(cal),
        meter(MeterStyle::Bars),
        meter(MeterStyle::Graph),
    ]
}

/// One row per look over the stripes, the widgets side by side, at `scale`.
pub fn render(scale: i32) -> Canvas {
    let gap = 24 * scale;
    let rows: Vec<Vec<(Canvas, i32, i32)>> = LOOKS
        .iter()
        .map(|&i| {
            let theme = super::cued(theme_for(i)).scaled(scale);
            let text = TextRenderer::load(&theme.fonts);
            let m = panel::margin(&theme);
            samples()
                .iter()
                .map(|c| {
                    (
                        deskwidget::draw(&theme, &text, c),
                        m.left * scale,
                        m.top * scale,
                    )
                })
                .collect()
        })
        .collect();
    let w = rows
        .iter()
        .map(|r| r.iter().map(|p| p.0.width as i32 + gap).sum::<i32>() + gap)
        .max()
        .unwrap_or(0);
    let h = rows
        .iter()
        .map(|r| r.iter().map(|p| p.0.height as i32).max().unwrap_or(0) + gap)
        .sum::<i32>()
        + gap;
    let mut c = super::blurred_stripes(w, h);
    let mut y = gap;
    for row in &rows {
        let mut x = gap;
        for (p, left, top) in row {
            c.blit_over(x - left, y - top, p);
            x += p.width as i32 + gap;
        }
        y += row.iter().map(|p| p.0.height as i32).max().unwrap_or(0) + gap;
    }
    c
}
