//! The progress bar through [`Part::ProgressBar`] and [`Part::ProgressChunk`]: the bar's
//! frame, then the chunk part stepped across `fraction` of the content rect. A nine-grid
//! part can only tile or stretch both axes at once, so one chunk frame (a block and the
//! clear gap after it, the content rect's height) is asked for at a time, stepping by its
//! width and clipping to the fraction, so the last block lands whole or is cut mid-block.
//! A style with [`Style::smooth_progress`] draws the chunk part once across the whole run.

use crate::look::Paint;
use crate::raster::{Canvas, Rect};
use crate::style::{Part, State, Style};

/// [`crate::look::widgets::draw_progress`] through `st`; `false` when `st`
/// lacks the bar or the chunk part. `r` is logical, as Classic's.
pub fn draw_progress(
    st: &Style,
    c: &mut Canvas,
    p: &Paint,
    r: Rect,
    fraction: f64,
    enabled: bool,
) -> bool {
    if !st.has_parts(&[Part::ProgressBar, Part::ProgressChunk]) {
        return false;
    }
    let state = if enabled {
        State::Normal
    } else {
        State::Disabled
    };
    let bar = r.scaled(p.s);
    st.draw(c, bar, Part::ProgressBar, state, p.s);
    let inner = st.content_rect(Part::ProgressBar, bar, p.s);
    let w = (inner.width as f64 * fraction.clamp(0.0, 1.0)).round() as i32;
    if w > 0 && st.smooth_progress {
        let run = Rect::new(inner.x, inner.y, w, inner.height);
        st.draw(c, run, Part::ProgressChunk, state, p.s);
    } else if w > 0 {
        let logical = inner.width / p.s.max(1);
        let w = (logical as f64 * fraction.clamp(0.0, 1.0)).round() as i32 * p.s;
        let run = Rect::new(inner.x, inner.y, w, inner.height);
        let pitch = st
            .part(Part::ProgressChunk)
            .map_or(1, |img| (img.frame(state).width / img.scale).max(1))
            * p.s;
        c.with_clip(run, |c| {
            let mut x = inner.x;
            while x < inner.x + w {
                let block = Rect::new(x, inner.y, pitch, inner.height);
                st.draw(c, block, Part::ProgressChunk, state, p.s);
                x += pitch;
            }
        });
    }
    true
}

#[cfg(test)]
mod tests {
    use crate::classic;
    use crate::interface::Interface;
    use crate::interfaces::theme_for;
    use crate::look::widgets::draw_progress;
    use crate::raster::{Canvas, Rect};
    use crate::style::{Part, State};
    use crate::theme::Color;

    #[test]
    fn a_classic_look_paints_the_classic_painter() {
        let theme = theme_for(Interface::Classic);
        let r = Rect::new(2, 2, 100, 16);
        let (mut a, mut b) = (Canvas::new(110, 26), Canvas::new(110, 26));
        draw_progress(&mut a, &theme.paint(), r, 0.4, false);
        classic::progress::draw(&mut b, &theme.paint(), r, 0.4, false);
        assert!(a == b, "a Classic look paints the Classic painter");
    }

    /// Navy's chunk atlas pixel at (`x`, `y`), Normal frame, scale 1.
    fn chunk_px(x: i32, y: i32) -> u32 {
        let theme = theme_for(Interface::Blue);
        let paint = theme.paint();
        let st = paint.look.style().expect("Blue carries a style");
        let img = st.part(Part::ProgressChunk).expect("Blue has a chunk");
        img.atlas.get(x, y)
    }

    /// The trough's corner is rounded off: its outer tip shows mostly what
    /// lies under the bar, its rim runs straight along all four sides past
    /// the curve, and it is white behind the fill.
    #[test]
    fn a_blue_progress_bar_turns_its_own_rounded_corner() {
        let theme = theme_for(Interface::Blue);
        let paint = theme.paint();
        let st = paint.look.style().expect("Blue carries a style");
        let sentinel = Color::rgb(0x12, 0x34, 0x56);
        let mut c = Canvas::new(120, 30);
        c.fill_rect(Rect::new(0, 0, 120, 30), sentinel);
        let r = Rect::new(3, 3, 101, 13);
        st.draw(&mut c, r, Part::ProgressBar, State::Normal, 1);
        let at = |dx: i32, dy: i32| c.get(r.x + dx, r.y + dy);
        let rim = crate::blue::Variant::Blue
            .scheme()
            .progress_bar
            .normal
            .rim
            .argb();
        let blue = |p: u32| p & 0xFF;
        for (x, y) in [(0, 0), (100, 0), (0, 12), (100, 12)] {
            assert!(
                blue(at(x, y)) < 0x80,
                "the corner tip at ({x}, {y}) shows mostly the backdrop"
            );
        }
        assert_eq!(at(0, 6), rim, "the left rim runs straight past the corner");
        assert_eq!(
            at(100, 6),
            rim,
            "the right rim runs straight past the corner"
        );
        assert_eq!(at(50, 0), rim, "the top rim runs straight");
        assert_eq!(at(50, 12), rim, "the bottom rim runs straight");
        assert_eq!(at(50, 6), 0xFFFF_FFFF, "white behind the fill");
    }

    /// Neither the trough nor its fill shades any differently when
    /// disabled, so a disabled bar repeats the enabled one.
    #[test]
    fn a_blue_progress_bar_disabled_frame_repeats_normal() {
        let theme = theme_for(Interface::Blue);
        let r = Rect::new(2, 2, 100, 16);
        let (mut enabled, mut disabled) = (Canvas::new(110, 26), Canvas::new(110, 26));
        draw_progress(&mut enabled, &theme.paint(), r, 0.6, true);
        draw_progress(&mut disabled, &theme.paint(), r, 0.6, false);
        assert!(
            enabled == disabled,
            "a disabled progress bar repeats the enabled trough and fill"
        );
    }

    /// The fill steps a 6 px block and its 2 px gap across the trough's
    /// content rect, starting 4 px in from the trough's left; a full bar's
    /// 95 px run lands its 12th block whole, one pixel of its gap showing
    /// past it.
    #[test]
    fn a_blue_progress_chunk_steps_its_block_and_gap() {
        let theme = theme_for(Interface::Blue);
        let r = Rect::new(3, 3, 101, 13);
        let mut c = Canvas::new(110, 20);
        draw_progress(&mut c, &theme.paint(), r, 1.0, true);
        let row = r.y + 3 + 3;
        let block = |x: i32| c.get(r.x + 4 + x, row);
        let ink = chunk_px(0, 3);
        let white = 0xFFFF_FFFF;
        assert_eq!(
            block(0),
            ink,
            "the first block starts flush with the content rect"
        );
        assert_eq!(block(5), ink, "a block is 6 px wide");
        assert_eq!(block(6), white, "the gap shows the trough's white");
        assert_eq!(block(8), ink, "the next block starts one 8 px pitch in");
        assert_eq!(block(93), ink, "a full bar's last block lands whole");
        assert_eq!(block(94), white, "one pixel of its gap shows past it");
    }

    /// A fraction that ends inside a block cuts it there.
    #[test]
    fn a_blue_progress_chunk_clips_a_block_mid_run() {
        let theme = theme_for(Interface::Blue);
        let r = Rect::new(3, 3, 101, 13);
        let mut c = Canvas::new(110, 20);
        draw_progress(&mut c, &theme.paint(), r, 10.0 / 95.0, true);
        let row = r.y + 3 + 3;
        let block = |x: i32| c.get(r.x + 4 + x, row);
        let ink = chunk_px(0, 3);
        assert_eq!(block(8), ink, "the second block's first column shows");
        assert_eq!(
            block(9),
            ink,
            "its second column is the last the run reaches"
        );
        assert_eq!(block(10), 0xFFFF_FFFF, "the run's edge cuts it there");
    }

    /// A block stretches its top-to-bottom shading over the content rect's
    /// height in one sweep, its own top row at the top and its bottom row
    /// at the bottom, rather than repeating every 7 px.
    #[test]
    fn a_blue_progress_chunk_stretches_over_a_taller_bar() {
        let theme = theme_for(Interface::Blue);
        let paint = theme.paint();
        let st = paint.look.style().expect("Blue carries a style");
        let (top, middle, bottom) = (chunk_px(0, 0), chunk_px(0, 3), chunk_px(0, 6));
        for h in [10, 12] {
            let mut c = Canvas::new(20, 20);
            let r = Rect::new(2, 2, 6, h);
            st.draw(&mut c, r, Part::ProgressChunk, State::Normal, 1);
            let at = |dy: i32| c.get(r.x, r.y + dy);
            assert_eq!(at(0), top, "{h}: the block's top row opens the sweep");
            assert_eq!(at(h / 2), middle, "{h}: its middle row halfway down");
            assert_eq!(at(h - 1), bottom, "{h}: its bottom row closes it");
        }
    }

    /// On a dialog's face (the trough's corners are soft, so the face
    /// shows through them).
    #[test]
    fn a_blue_progress_bar_on_a_scale_2_canvas_doubles_every_pixel() {
        let theme = theme_for(Interface::Blue);
        let r = Rect::new(3, 3, 101, 16);
        let (mut one, mut two) = (Canvas::new(110, 23), Canvas::with_scale(110, 23, 2));
        for c in [&mut one, &mut two] {
            c.fill_rect(Rect::new(0, 0, 110, 23), theme.colors.button_face);
        }
        draw_progress(&mut one, &theme.paint(), r, 0.37, true);
        draw_progress(&mut two, &theme.paint(), r, 0.37, true);
        assert!(crate::look::styled::fixture::doubles(&one, &two));
    }

    #[test]
    fn a_style_with_only_the_bar_part_falls_back_to_classic_whole() {
        let look = crate::look::styled::fixture::one_part_look(Part::ProgressBar);
        let cs = crate::theme::ColorScheme::default();
        let p = crate::look::Paint {
            cs: &cs,
            look: &look,
            s: 1,
            back: crate::look::Backdrop::Face,
            active: true,
            access_keys: true,
            text: crate::theme::TextUnits::ONE,
        };
        let r = Rect::new(2, 2, 100, 16);
        let (mut a, mut b) = (Canvas::new(110, 26), Canvas::new(110, 26));
        draw_progress(&mut a, &p, r, 0.4, true);
        classic::progress::draw(&mut b, &p, r, 0.4, true);
        assert!(a == b, "a style with no chunk part paints Classic's bar");
    }

    #[test]
    fn a_blue_progress_bar_at_paint_scale_2_doubles_every_pixel() {
        let draw = |c: &mut Canvas, p: &crate::look::Paint| {
            draw_progress(c, p, Rect::new(3, 3, 101, 16), 0.37, true);
        };
        assert!(
            crate::look::styled::fixture::blue_doubles_at_paint_scale_2(110, 23, draw),
            "a bar 37% full at s = 2 is its s = 1 pixels doubled"
        );
    }
}
