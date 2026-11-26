//! The mixer body in a window too short for it: once the volume trackbars are at their
//! shortest, the columns keep that height and a bar down the body's right edge scrolls them.

use reprisekit::controls::mixer::{self, Mixer};
use reprisekit::controls::scrollview::{self, Frame, Hit, ScrollView};
use reprisekit::raster::Rect;
use reprisekit::text::TextRenderer;
use reprisekit::theme::Theme;

/// The body showing in `body`, laid out for scrolling.
pub struct Placed {
    pub frame: Frame,
    pub view: scrollview::Layout,
    /// Where the mixer lays its columns out: `body` while it fits, else as tall as the columns
    /// at their shortest, beside the bar and moved up by the scroll.
    pub mixer: Rect,
    pub thumb_min: i32,
}

impl Placed {
    /// Whether the body scrolls.
    pub fn scrolls(&self) -> bool {
        self.view.bar.is_some()
    }

    pub fn hit(&self, s: &ScrollView, x: i32, y: i32) -> Option<Hit> {
        s.hit(&self.frame, &self.view, self.thumb_min, x, y)
    }
}

/// The bar's width the window adds for a body `h` tall, 0 when the columns fit it.
pub fn bar_room(m: &Mixer, theme: &Theme, h: i32) -> i32 {
    if h < m.min_height(theme.text) {
        theme.controls().scroll
    } else {
        0
    }
}

/// The body in `body` scrolled as `s` is. Columns that scroll sideways too keep their
/// shortest height above the mixer's own bar along the bottom.
pub fn place(m: &Mixer, theme: &Theme, text: &TextRenderer, body: Rect, s: &ScrollView) -> Placed {
    let cm = theme.controls();
    let mut frame = Frame {
        inside: body,
        reach: (body.x, body.y + m.min_height(theme.text)),
        bar: cm.scroll,
        edge: 0,
    };
    let mut view = scrollview::layout(&frame, body, cm.scroll);
    if view.bar.is_some() {
        let h = frame.reach.1 - body.y;
        let tall = Rect::new(body.x, body.y, view.clip.width, h);
        if let Some(bar) = mixer::layout(m, theme, text, tall).bar {
            frame.reach.1 += bar.height;
            view = scrollview::layout(&frame, body, cm.scroll);
        }
    }
    let mixer = match view.bar {
        Some(_) => Rect::new(
            body.x,
            body.y - s.offset().1,
            view.clip.width,
            frame.reach.1 - body.y,
        ),
        None => body,
    };
    Placed {
        frame,
        view,
        mixer,
        thumb_min: cm.thumb_min,
    }
}
