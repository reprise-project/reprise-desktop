use reprisekit::controls::statusbar::{self, Panel, StatusBar};
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::TextRenderer;
use reprisekit::theme::{Color, Theme};

pub const HEIGHT: i32 = 2 * 23 + 12;

pub fn draw(c: &mut Canvas, theme: &Theme, text: &TextRenderer, r: Rect) {
    let h = statusbar::height(&theme.controls());
    let mut icon = Canvas::new(statusbar::ICON_PX as u32, statusbar::ICON_PX as u32);
    icon.fill_rect(Rect::new(2, 2, 12, 12), Color::rgb(80, 193, 255));
    for (i, grip) in [true, false].into_iter().enumerate() {
        let sb = StatusBar {
            panels: vec![
                Panel {
                    text: "4 objects".into(),
                    width: 0,
                    pad: statusbar::EXPLORER_TEXT_PAD,
                    icon: None,
                },
                Panel {
                    text: "11 bytes".into(),
                    width: 76,
                    pad: statusbar::EXPLORER_TEXT_PAD,
                    icon: None,
                },
                Panel {
                    text: "Computer".into(),
                    width: 153,
                    pad: statusbar::EXPLORER_TEXT_PAD,
                    icon: Some(icon.clone()),
                },
            ],
            grip,
        };
        let bar = Rect::new(r.x + 4, r.y + 4 + i as i32 * (h + 4), r.width - 8, h);
        statusbar::draw(c, text, &theme.paint(), &sb, bar, theme.fonts.size_px);
    }
}
