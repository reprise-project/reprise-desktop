//! The task pane at rest and lit.

use reprisekit::controls::taskpane::{self, Row, Section, TaskPane};
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::TextRenderer;
use reprisekit::theme::{Color, Theme};

pub const HEIGHT: i32 = 170;

pub fn draw(c: &mut Canvas, theme: &Theme, text: &TextRenderer, r: Rect) {
    let px = theme.fonts.size_px;
    let p = theme.paint();
    let icon = |colour: Color| {
        let mut i = Canvas::new(16, 16);
        i.fill_rect(i.bounds(), colour);
        Some(i)
    };
    let mut folded = Section::lines("Details", vec![("Notes".into(), true)]);
    folded.open = false;
    let mut tp = TaskPane {
        sections: vec![
            Section::links(
                "File and Folder Tasks",
                vec![
                    Row::new("Make a new folder", icon(Color::rgb(255, 204, 0))),
                    Row::new(
                        "Publish this folder to the Web",
                        icon(Color::rgb(0, 128, 0)),
                    ),
                ],
            ),
            folded,
        ],
        ..Default::default()
    };
    for (i, lit) in [false, true].into_iter().enumerate() {
        if lit {
            tp.hot_header = Some(0);
            tp.hot = Some((0, 0));
        }
        let pane = Rect::new(r.x + 10 + i as i32 * 220, r.y, 200, r.height);
        let lay = taskpane::layout(&tp, text, px, pane, &theme.look.controls());
        taskpane::draw(c, text, &p, &tp, &lay, px);
        taskpane::draw_gap(
            c,
            &p,
            &lay,
            Rect::new(pane.right(), pane.y, 10, pane.height),
        );
    }
}
