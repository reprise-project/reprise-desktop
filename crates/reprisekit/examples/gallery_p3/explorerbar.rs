//! The explorer bar's title strip with its close box at rest, lit and held, over a white body, beside the file dialog's places bar panel.

use reprisekit::controls::explorerbar::{self, ExplorerBar};
use reprisekit::controls::placesbar::{self, Place, PlacesBar};
use reprisekit::look::widgets::Interaction;
use reprisekit::look::widgets::explorerbar::draw_close;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::TextRenderer;
use reprisekit::theme::Theme;

pub const HEIGHT: i32 = 150;

pub fn draw(c: &mut Canvas, theme: &Theme, text: &TextRenderer, r: Rect) {
    let px = theme.fonts.size_px;
    let p = theme.paint();
    let cm = theme.look.controls();
    let eb = ExplorerBar::new("Folders");
    for (i, ptr) in [Interaction::Idle, Interaction::Hot, Interaction::Pressed]
        .into_iter()
        .enumerate()
    {
        let frame = Rect::new(r.x + 10, r.y + 8 + i as i32 * 46, 204, 40);
        let lay = explorerbar::layout(&eb, frame, &cm);
        c.fill_rect(lay.body, theme.colors.window);
        explorerbar::draw(c, text, &p, &eb, &lay, px);
        draw_close(c, &p, lay.close, ptr);
    }
    let pb = PlacesBar {
        places: ["Desktop", "Documents"]
            .into_iter()
            .map(|l| Place {
                label: l.into(),
                icon: None,
                enabled: true,
            })
            .collect(),
        ..PlacesBar::default()
    };
    let bar = Rect::new(r.x + 240, r.y + 8, placesbar::WIDTH, 2 * placesbar::TILE_H);
    placesbar::draw(c, text, &p, &pb, bar, px);
}
