//! The debug overlay's panel: a bold title and label/value rows in a box of fixed width [`W`], so
//! it keeps still while numbers change. Under a glass look the box is glass and the text white. It
//! takes no clicks: [`hit`] only says which row a point is on.

use crate::access::{self, Tree, accesskit};
use crate::look::{Paint, panel};
use crate::raster::{Canvas, EdgeStyle, Rect};
use crate::text::{Align, TextRenderer};
use crate::theme::{Color, Theme};
use accesskit::{NodeId, Role};

pub const W: i32 = 236;
const PAD: i32 = 8;
pub const LINE_H: i32 = 15;
const LABEL_W: i32 = 92;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub label: String,
    pub value: String,
}

impl Row {
    pub fn new(label: &str, value: impl Into<String>) -> Row {
        Row {
            label: label.into(),
            value: value.into(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StatsPanel {
    pub title: String,
    pub rows: Vec<Row>,
}

pub fn height(rows: usize) -> i32 {
    2 * PAD + LINE_H * (rows as i32 + 1)
}

pub struct Layout {
    pub title: Rect,
    pub rows: Vec<(Rect, Rect)>,
}

pub fn layout(p: &StatsPanel, r: Rect) -> Layout {
    let inner = r.width - 2 * PAD;
    let line = |i: i32| r.y + PAD + LINE_H * i;
    Layout {
        title: Rect::new(r.x + PAD, line(0), inner, LINE_H),
        rows: (1..=p.rows.len() as i32)
            .map(|i| {
                let label = Rect::new(r.x + PAD, line(i), LABEL_W, LINE_H);
                let value = Rect::new(label.right(), line(i), inner - LABEL_W, LINE_H);
                (label, value)
            })
            .collect(),
    }
}

pub fn draw(theme: &Theme, text: &TextRenderer, p: &StatsPanel) -> Canvas {
    let paint = &theme.paint();
    let (cs, s) = (paint.cs, paint.s);
    let h = height(p.rows.len());
    if let Some((style, spec)) = panel::glass(theme) {
        return panel::draw(style, spec, W, h, s, |c| {
            body(c, paint, text, theme.fonts.size_px, p, Some(panel::INK))
        });
    }
    let mut c = Canvas::new((W * s) as u32, (h * s) as u32);
    c.fill_rect(c.bounds(), cs.button_face);
    c.edge_t(c.bounds(), EdgeStyle::WindowRaised, cs, s);
    body(&mut c, paint, text, theme.fonts.size_px, p, None);
    c
}

fn body(
    c: &mut Canvas,
    paint: &Paint,
    text: &TextRenderer,
    px: u32,
    p: &StatsPanel,
    ink: Option<Color>,
) {
    let s = paint.s;
    let lay = layout(p, Rect::new(0, 0, W, height(p.rows.len())));
    let mut put = |r: Rect, t: &str, bold: bool| {
        let r = r.scaled(s);
        let t = text.ellipsize(t, r.width, bold, px);
        match ink {
            Some(ink) => {
                let base = text.baseline(r, bold, px).unwrap_or(r.bottom());
                text.draw_run_over(c, r.x, base, r, &t, bold, ink, px);
            }
            None => text.draw(c, r, &t, bold, paint.cs.button_text, Align::Left, px),
        }
    };
    put(lay.title, &p.title, true);
    for (row, (l, v)) in p.rows.iter().zip(lay.rows) {
        put(l, &row.label, false);
        put(v, &row.value, false);
    }
}

pub fn hit(p: &StatsPanel, r: Rect, x: i32, y: i32) -> Option<usize> {
    layout(p, r)
        .rows
        .iter()
        .position(|(l, v)| l.contains(x, y) || v.contains(x, y))
}

pub fn access(p: &StatsPanel, r: Rect, tree: &mut Tree, parent: NodeId) -> NodeId {
    let id = tree.push(parent, "stats", access::named(Role::Group, r, &p.title));
    for (i, (row, (l, v))) in p.rows.iter().zip(layout(p, r).rows).enumerate() {
        let line = format!("{}: {}", row.label, row.value);
        tree.push(id, i, access::label(l.union(&v), &line, true));
    }
    id
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::FontConfig;

    fn sample() -> StatsPanel {
        StatsPanel {
            title: "Virtual-1".into(),
            rows: vec![
                Row::new("Renders/s", "60"),
                Row::new("Render CPU", "1.2 ms avg, 3.4 max"),
            ],
        }
    }

    #[test]
    fn the_rows_stack_under_the_title_label_then_value() {
        let p = sample();
        let lay = layout(&p, Rect::new(0, 0, W, height(2)));
        assert_eq!(height(2), 61);
        assert_eq!(lay.title, Rect::new(8, 8, 220, 15));
        assert_eq!(
            lay.rows,
            [
                (Rect::new(8, 23, 92, 15), Rect::new(100, 23, 128, 15)),
                (Rect::new(8, 38, 92, 15), Rect::new(100, 38, 128, 15)),
            ]
        );
        assert_eq!(hit(&p, Rect::new(0, 0, W, 61), 150, 40), Some(1));
        assert_eq!(hit(&p, Rect::new(0, 0, W, 61), 150, 10), None, "the title");
    }

    #[test]
    fn classic_draws_a_raised_face_box_with_dark_text_in_each_column() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let text = TextRenderer::load(&FontConfig::default());
        let p = sample();
        let c = draw(&theme, &text, &p);
        assert_eq!((c.width, c.height), (W as u32, 61));
        assert_eq!(c.get(0, 0), cs.button_face.argb());
        assert_eq!(c.get(1, 1), cs.button_hilight.argb());
        assert_eq!(c.get(W - 1, 60), cs.button_dk_shadow.argb());
        let ink = cs.button_text.argb();
        let inked =
            |r: Rect| (r.y..r.bottom()).any(|y| (r.x..r.right()).any(|x| c.get(x, y) == ink));
        let lay = layout(&p, Rect::new(0, 0, W, 61));
        assert!(inked(lay.title));
        for (l, v) in lay.rows {
            assert!(inked(l) && inked(v), "{l:?} and {v:?} hold text");
        }
        assert!((54..58).all(|y| (2..W - 2).all(|x| c.get(x, y) == cs.button_face.argb())));
    }

    #[test]
    fn under_glass_the_panel_is_glass_with_white_text() {
        let theme = crate::interfaces::theme_for(crate::interface::Interface::Glass);
        let text = TextRenderer::load(&FontConfig::default());
        let c = draw(&theme, &text, &sample());
        let m = panel::margin(&theme);
        assert_eq!(
            (c.width as i32, c.height as i32),
            (W + m.left + m.right, 61 + m.top + m.bottom),
            "the panel and its shadow"
        );
        let at = |x: i32, y: i32| c.get(m.left + x, m.top + y);
        let a = at(W / 2, 58) >> 24;
        assert!(a > 0 && a < 0xFF, "glass under the last row: alpha {a:#x}");
        let white = (38..53).any(|y| (100..228).any(|x| at(x, y) == 0xFFFF_FFFF));
        assert!(white, "the last value is white");
    }
}
