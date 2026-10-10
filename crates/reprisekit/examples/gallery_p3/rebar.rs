//! The rebar Explorer hangs its menu bar, toolbar and address bar from:
//! three bands (22, 38 and 22 px) across the column, empty, then the same stack 792 px wide, clipped at the column's edge.

use reprisekit::controls::rebar;
use reprisekit::controls::toolbar;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::TextRenderer;
use reprisekit::theme::Theme;

pub const HEIGHT: i32 = 184;

pub fn draw(c: &mut Canvas, theme: &Theme, _text: &TextRenderer, r: Rect) {
    let cm = theme.look.controls();
    let rb = rebar::Rebar {
        bands: vec![
            rebar::Band { height: 22 },
            rebar::Band {
                height: toolbar::height(true, &cm),
            },
            rebar::Band { height: 22 },
        ],
    };
    let p = theme.paint();
    let h = rebar::height(&rb, &cm.rebar);
    let across = Rect::new(r.x, r.y + 4, r.width, h);
    rebar::draw(c, &p, &rb, across);
    // The 792 px stack runs past the 640 px column: clip it.
    let mut wide = Canvas::new(792, h as u32);
    rebar::draw(&mut wide, &p, &rb, Rect::new(0, 0, 792, h));
    let y = r.y + 8 + h;
    for row in 0..h {
        for x in 0..r.width {
            c.put_raw(r.x + x, y + row, wide.get(x, row));
        }
    }
}
