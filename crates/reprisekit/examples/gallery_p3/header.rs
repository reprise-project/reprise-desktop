//! A list header: columns at rest, under the pointer and held, the sort
//! arrow rising and falling, and the strip past the last column. Each row
//! is a list view with no rows of its own.

use reprisekit::controls::listview::{self, Column, ListView};
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::TextRenderer;
use reprisekit::theme::Theme;

/// One list view per row.
const ROW: i32 = 26;
const ROWS: i32 = 3;

pub const HEIGHT: i32 = ROWS * ROW + 4;

pub fn draw(c: &mut Canvas, theme: &Theme, text: &TextRenderer, r: Rect) {
    let cm = theme.look.controls();
    // (sorted column and rising, the column under the pointer, the column
    // held)
    let rows = [
        (Some((0, true)), Some(1), None),
        (Some((0, false)), None, Some(2)),
        (Some((1, true)), Some(0), None),
    ];
    for (i, (sort, hot, held)) in rows.into_iter().enumerate() {
        let mut lv: ListView = ListView {
            columns: vec![
                Column::new("Name", 180),
                Column {
                    title: "Size".into(),
                    width: 96,
                    right: true,
                },
                Column::new("Type", 120),
                Column::new("Date Modified", 120),
            ],
            sort,
            frame: false,
            ..ListView::default()
        };
        lv.header.motion(hot);
        if held.is_some() {
            lv.header.press(held, false);
        }
        let box_r = Rect::new(
            r.x + 4,
            r.y + 4 + i as i32 * ROW,
            r.width - 8,
            cm.list_header + 2,
        );
        let lay = listview::layout(&lv, box_r, &cm);
        listview::draw(
            c,
            text,
            &theme.paint(),
            &lv,
            &lay,
            None,
            theme.fonts.size_px,
        );
    }
}
