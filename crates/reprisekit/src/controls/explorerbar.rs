//! The Explorer bar's frame: a title strip (title left, close box right), a
//! line under it, and the 4 px splitter between the bar and the view. The
//! body under the line is the host's. The look's [`BarMetrics`] size the
//! pieces and [`crate::look::widgets::explorerbar`] paints the strip and
//! close box.
//!
//! Classic's bar, 200 px wide:
//!
//! ```text
//! x 0 / 1              an etched ring's shadow and hilight columns (x w-2
//!                      and w-1 on the right, the last two rows at the
//!                      bottom)
//! y 0 / 1              its top rows; the strip's face from y 2
//! y 22                 shadow from x 2 to w-2, joining the right column
//! y 23                 hilight from x 1 to w-3
//! (2, 24)              the body's top-left
//! ```
//!
//! Its close box is [`Glyph::Close`](crate::art::glyph::Glyph::Close) in a
//! 16x14 unit, bare cold and in a 1 px `ThinRaised` ring hot. Blue's has no
//! ring: a 22 px strip over a 1 px line, the body from the bar's edges, and
//! a 20x17 close box against the strip's top.

use crate::controls::metrics::{BarMetrics, ControlMetrics};
use crate::look::Paint;
use crate::look::widgets::explorerbar as look;
use crate::pointer::Interaction;
use crate::raster::{Canvas, Rect};
use crate::text::{Align, TextRenderer};

pub const SPLITTER_W: i32 = 4;

pub const MIN_W: i32 = 100;

/// The width the Folders tree and the task pane open at, splitter excluded.
pub const DEFAULT_W: i32 = 200;

/// One Explorer bar: which one, its width, and whether the pointer rests on its X.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExplorerBar {
    pub title: String,
    pub width: i32,
    pub hot_close: bool,
    /// The title in grey rather than window text: a bar that is drawn
    /// but has nothing behind it.
    pub greyed: bool,
}

impl ExplorerBar {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            width: DEFAULT_W,
            hot_close: false,
            greyed: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub frame: Rect,
    pub header: Rect,
    pub close: Rect,
    /// Inside the ring and under the strip's line, for the host to fill.
    pub body: Rect,
    /// The 4 px face column between the bar and the view.
    pub splitter: Rect,
    /// Title text box start from the strip's left, and its line's offset below the strip's middle.
    pub title_x: i32,
    pub title_dy: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Close,
    Splitter,
    Body,
    Nothing,
}

/// The bar's pieces inside `r`, which spans the bar *and* its splitter,
/// at `cm.bar`'s sizes: the bar itself is `eb.width` wide, at least
/// [`MIN_W`], and never more than `r` leaves after the splitter, even
/// when that is under `MIN_W`.
pub fn layout(eb: &ExplorerBar, r: Rect, cm: &ControlMetrics) -> Layout {
    let BarMetrics {
        ring,
        header: header_h,
        line,
        title_x,
        title_dy,
        close_w,
        close_h,
        close_right,
        close_top,
    } = cm.bar;
    let width = eb.width.max(MIN_W).min((r.width - SPLITTER_W).max(0));
    let header = Rect::new(r.x, r.y + ring, width, header_h);
    Layout {
        frame: r,
        header,
        close: Rect::new(
            header.right() - close_right,
            header.y + close_top,
            close_w,
            close_h,
        ),
        body: Rect::new(
            r.x + ring,
            header.bottom() + line,
            (width - 2 * ring).max(0),
            (r.bottom() - ring - header.bottom() - line).max(0),
        ),
        splitter: Rect::new(r.x + width, r.y, SPLITTER_W, r.height),
        title_x,
        title_dy,
    }
}

/// What is under `(x, y)`. The header's empty face is [`Hit::Nothing`]:
/// only the X and the splitter do anything there. Nothing outside the
/// frame hits, even where a squeezed bar's X unit pokes past it.
pub fn hit(lay: &Layout, x: i32, y: i32) -> Hit {
    if !lay.frame.contains(x, y) {
        Hit::Nothing
    } else if lay.close.contains(x, y) {
        Hit::Close
    } else if lay.splitter.contains(x, y) {
        Hit::Splitter
    } else if lay.body.contains(x, y) {
        Hit::Body
    } else {
        Hit::Nothing
    }
}

/// Paint the strip and its frame, the splitter and the close box, clipped
/// to the frame, with the title ellipsized to the room left of the X. The
/// body is left untouched for the host.
pub fn draw(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    eb: &ExplorerBar,
    lay: &Layout,
    px: u32,
) {
    let Paint { cs, s, .. } = *p;
    c.with_clip(lay.frame.scaled(s), |c| {
        let head = lay.header.scaled(s);
        let bar = Rect::new(lay.frame.x, lay.frame.y, lay.header.width, lay.frame.height);
        look::draw_strip(c, p, bar.scaled(s), head);
        c.fill_rect(lay.splitter.scaled(s), cs.button_face);
        let title = Rect::new(
            head.x + lay.title_x * s,
            head.y + lay.title_dy * s,
            (lay.close.x - lay.header.x - lay.title_x) * s,
            head.height,
        );
        let ink = if eb.greyed {
            cs.gray_text
        } else {
            cs.button_text
        };
        let shown = text.ellipsize(&eb.title, title.width, false, px);
        text.draw(c, title, &shown, false, ink, Align::Left, px);
        let ptr = if eb.hot_close {
            Interaction::Hot
        } else {
            Interaction::Idle
        };
        look::draw_close(c, p, lay.close.scaled(s), ptr);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::look::classic_paint;
    use crate::theme::{FontConfig, Theme};

    const CM: ControlMetrics = ControlMetrics::CLASSIC;
    const TITLE_X: i32 = 8;

    /// A Folders bar at its default width, 83 px down a client: the
    /// bar and its splitter, 204 px across.
    const BAR: Rect = Rect::new(0, 83, 204, 300);
    const HEADER_Y: i32 = 85;
    const SPLITTER_X: i32 = 200;
    const X_LEFT: i32 = 183;
    const X_TOP: i32 = 90;

    fn bar() -> ExplorerBar {
        ExplorerBar::new("Folders")
    }

    #[test]
    fn layout_puts_the_header_under_the_etch_and_the_splitter_at_200() {
        let lay = layout(&bar(), BAR, &CM);
        assert_eq!(lay.header, Rect::new(0, HEADER_Y, 200, 20));
        assert_eq!(lay.splitter, Rect::new(SPLITTER_X, 83, 4, 300));
        assert_eq!(lay.body, Rect::new(2, 107, 196, 274));
        assert_eq!((lay.close.x + 4, lay.close.y + 3), (X_LEFT, X_TOP));
        assert_eq!((lay.close.width, lay.close.height), (16, 14));
    }

    #[test]
    fn a_dragged_width_moves_the_splitter_and_stops_at_min_w() {
        let mut eb = bar();
        eb.width = 300;
        let lay = layout(&eb, Rect::new(0, 83, 400, 300), &CM);
        assert_eq!(lay.splitter.x, 300);
        assert_eq!(lay.header.width, 300);
        eb.width = 20;
        assert_eq!(layout(&eb, BAR, &CM).header.width, MIN_W);
        eb.width = 9999;
        assert_eq!(layout(&eb, BAR, &CM).header.width, 200);
    }

    /// A rect narrower than `MIN_W` plus the splitter shrinks the bar to
    /// what is left, so nothing lands on the view beside it.
    #[test]
    fn a_rect_narrower_than_min_w_shrinks_the_bar_to_fit() {
        let r = Rect::new(20, 83, 60, 300);
        let lay = layout(&bar(), r, &CM);
        assert_eq!(lay.header.width, 56);
        assert_eq!(lay.body.width, 52);
        assert_eq!(lay.splitter, Rect::new(76, 83, 4, 300));
        let lay = layout(&bar(), Rect::new(20, 83, 2, 300), &CM);
        assert_eq!(lay.header.width, 0);
        assert_eq!(lay.splitter.x, 20);
        assert_eq!(hit(&lay, lay.close.x, lay.close.y + 1), Hit::Nothing);
    }

    /// Squeezed below the X's own width, the bar keeps every pixel inside
    /// its rect.
    #[test]
    fn a_squeezed_bar_draws_nothing_outside_its_rect() {
        if !crate::text::tahoma_installed() {
            return;
        }
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let cs = &theme.colors;
        let mut eb = bar();
        eb.hot_close = true;
        let sentinel = crate::theme::Color::rgb(1, 2, 3);
        for w in [60, 30, 12] {
            let r = Rect::new(40, 0, w, 60);
            let lay = layout(&eb, r, &CM);
            let mut c = Canvas::new(160, 60);
            c.fill_rect(c.bounds(), sentinel);
            draw(
                &mut c,
                &text,
                &classic_paint(cs, 1),
                &eb,
                &lay,
                theme.fonts.size_px,
            );
            for y in 0..60 {
                for x in (0..40).chain(r.right()..160) {
                    assert_eq!(
                        c.get(x, y),
                        sentinel.argb(),
                        "({x}, {y}) stays untouched at width {w}"
                    );
                }
            }
        }
    }

    #[test]
    fn hit_finds_the_x_the_splitter_and_the_body() {
        let lay = layout(&bar(), BAR, &CM);
        assert_eq!(hit(&lay, X_LEFT + 1, X_TOP + 1), Hit::Close);
        assert_eq!(hit(&lay, 201, 200), Hit::Splitter);
        assert_eq!(hit(&lay, 100, 200), Hit::Body);
        assert_eq!(hit(&lay, 10, 92), Hit::Nothing);
        assert_eq!(hit(&lay, lay.close.x - 1, 92), Hit::Nothing);
    }

    #[test]
    fn draw_lays_the_x_and_the_title_in_the_header() {
        if !crate::text::tahoma_installed() {
            return;
        }
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let cs = &theme.colors;
        let px = theme.fonts.size_px;
        let eb = bar();
        let lay = layout(&eb, BAR, &CM);
        let mut c = Canvas::new(204, 383);
        draw(&mut c, &text, &classic_paint(cs, 1), &eb, &lay, px);
        let ink = cs.button_text.argb();
        let face = cs.button_face.argb();
        assert_eq!(c.get(100, 83), cs.button_shadow.argb());
        assert_eq!(c.get(100, 84), cs.button_hilight.argb());
        assert_eq!(c.get(100, 85), face);
        assert_eq!(c.get(100, 104), face);
        assert_eq!(c.get(X_LEFT + 3, X_TOP + 3), ink);
        assert_eq!(c.get(X_LEFT, X_TOP), ink);
        assert_eq!(c.get(X_LEFT + 7, X_TOP + 6), ink);
        assert_eq!(c.get(X_LEFT - 1, X_TOP), face);
        assert_eq!(c.get(lay.close.x, lay.close.y), face);
        let mut hot = eb.clone();
        hot.hot_close = true;
        draw(&mut c, &text, &classic_paint(cs, 1), &hot, &lay, px);
        assert_eq!(c.get(lay.close.x, lay.close.y), cs.button_hilight.argb());
        assert!((85..105).any(|y| c.get(TITLE_X, y) == ink));
        assert!((85..105).all(|y| c.get(TITLE_X - 1, y) == face));
    }

    /// The bar's edges, client-local for a 200 px bar from y 83: an
    /// etched ring round it (shadow then hilight on the left and top,
    /// shadow at x 198 and hilight at x 199, shadow then hilight on its
    /// last two rows), down the header's sides too; under the header a
    /// shadow row from x 2 into the right shadow column and a hilight row
    /// from x 1 to x 197.
    #[test]
    fn the_bar_wears_an_etched_ring_and_a_line_under_its_header() {
        const LEFT: [i32; 2] = [0, 1];
        const RIGHT: [i32; 2] = [198, 199];
        const HEADER_ROW: i32 = 95;
        const LINE_Y: i32 = 105;
        const BOTTOM: [i32; 2] = [381, 382];
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let cs = &theme.colors;
        let lay = layout(&bar(), BAR, &CM);
        let sentinel = crate::theme::Color::rgb(1, 2, 3);
        let mut c = Canvas::new(204, 383);
        c.fill_rect(c.bounds(), sentinel);
        draw(
            &mut c,
            &text,
            &classic_paint(cs, 1),
            &bar(),
            &lay,
            theme.fonts.size_px,
        );
        let (sh, hi) = (cs.button_shadow.argb(), cs.button_hilight.argb());
        for y in [HEADER_ROW, 200] {
            assert_eq!(c.get(LEFT[0], y), sh, "left shadow at y {y}");
            assert_eq!(c.get(LEFT[1], y), hi, "left hilight at y {y}");
            assert_eq!(c.get(RIGHT[0], y), sh, "right shadow at y {y}");
            assert_eq!(c.get(RIGHT[1], y), hi, "right hilight at y {y}");
        }
        assert_eq!(c.get(100, BOTTOM[0]), sh);
        assert_eq!(c.get(100, BOTTOM[1]), hi);
        assert_eq!(c.get(1, LINE_Y), hi, "the ring's hilight column");
        assert_eq!(c.get(2, LINE_Y), sh);
        assert_eq!(c.get(197, LINE_Y), sh);
        assert_eq!(c.get(1, LINE_Y + 1), hi);
        assert_eq!(c.get(197, LINE_Y + 1), hi);
        assert_eq!(c.get(RIGHT[0], LINE_Y + 1), sh);
        assert_eq!(lay.body.x, 2);
        assert_eq!(lay.body.y, LINE_Y + 2);
        assert_eq!(c.get(lay.body.x, lay.body.y), sentinel.argb());
        assert_eq!(
            c.get(lay.body.right() - 1, lay.body.bottom() - 1),
            sentinel.argb()
        );
    }

    /// A stub bar (Explorer's Search) writes its title in grey; the X
    /// and the frame are unchanged.
    #[test]
    fn a_greyed_bar_writes_its_title_in_gray_text() {
        if !crate::text::tahoma_installed() {
            return;
        }
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let cs = &theme.colors;
        let px = theme.fonts.size_px;
        let mut eb = ExplorerBar::new("Search");
        eb.greyed = true;
        let lay = layout(&eb, BAR, &CM);
        let mut c = Canvas::new(204, 383);
        draw(&mut c, &text, &classic_paint(cs, 1), &eb, &lay, px);
        assert!((85..105).any(|y| c.get(TITLE_X, y) == cs.gray_text.argb()));
        assert!((85..105).all(|y| c.get(TITLE_X, y) != cs.button_text.argb()));
    }

    /// Writes target/explorerbar-blue-{blue,olive,silver}-preview.png.
    #[test]
    #[ignore]
    fn render_explorerbar_blue() {
        for v in crate::blue::Variant::ALL {
            let theme = crate::interfaces::blue_theme(v);
            let text = TextRenderer::load(&theme.fonts);
            let p = theme.paint();
            let mut c = Canvas::new(204, 120);
            for (i, ptr) in [Interaction::Idle, Interaction::Hot, Interaction::Pressed]
                .into_iter()
                .enumerate()
            {
                let lay = layout(
                    &bar(),
                    Rect::new(0, i as i32 * 40, 204, 40),
                    &theme.look.controls(),
                );
                c.fill_rect(lay.body, theme.colors.window);
                draw(&mut c, &text, &p, &bar(), &lay, theme.fonts.size_px);
                look::draw_close(&mut c, &p, lay.close, ptr);
            }
            crate::raster::write_preview(
                &c,
                &format!("explorerbar-blue-{}-preview.png", v.as_str()),
            );
        }
    }

    /// Writes target/explorerbar-preview.png.
    #[test]
    #[ignore]
    fn render_explorerbar() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let eb = bar();
        let lay = layout(&eb, Rect::new(0, 0, 210, 30), &CM);
        let mut c = Canvas::new(210, 30);
        c.fill_rect(c.bounds(), theme.colors.window);
        draw(
            &mut c,
            &text,
            &classic_paint(&theme.colors, 1),
            &eb,
            &lay,
            theme.fonts.size_px,
        );
        crate::raster::write_preview(&c, "explorerbar-preview.png");
    }
}
