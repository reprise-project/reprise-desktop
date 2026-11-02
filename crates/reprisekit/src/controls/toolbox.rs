//! A raster editor's tool box: square toggle buttons in two columns, the current one latched
//! down, over a sunken well of the current tool's options. Layout is logical; `draw` scales by `s`.

use crate::access::{self, Tree, accesskit};
use crate::look::Paint;
use crate::look::widgets::{draw_field_frame, toolbar as look};
use crate::raster::{Canvas, Rect};
use crate::theme::Color;
use accesskit::{NodeId, Role, Toggled};

/// The width a host reserves for the box.
pub const WIDTH: i32 = 56;
pub const CELL: i32 = 25;
const COLS: i32 = 2;
const PAD: i32 = 3;
const WELL_GAP: i32 = 4;
pub const WELL_H: i32 = 66;
/// The well's sunken frame, which the option cells sit inside.
const WELL_EDGE: i32 = 2;

/// A tool button. `glyph` is a coverage mask drawn at the paint scale (16x16 logical); only its
/// alpha is read.
#[derive(Clone, Debug, PartialEq)]
pub struct Tool {
    pub name: String,
    pub glyph: Option<Canvas>,
}

/// One cell of the options well, a coverage mask like [`Tool::glyph`].
#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
    pub name: String,
    pub glyph: Option<Canvas>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolBox {
    pub tools: Vec<Tool>,
    pub current: usize,
    pub hot: Option<usize>,
    pub pressed: Option<usize>,
    pub options: Vec<Choice>,
    pub option_cols: usize,
    pub option: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub bounds: Rect,
    pub tools: Vec<Rect>,
    pub well: Rect,
    pub options: Vec<Rect>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Tool(usize),
    Option(usize),
}

fn tool_rows(tb: &ToolBox) -> i32 {
    (tb.tools.len() as i32 + COLS - 1) / COLS
}

/// The height the box needs: its buttons and the options well.
pub fn height(tb: &ToolBox) -> i32 {
    PAD + tool_rows(tb) * CELL + WELL_GAP + WELL_H + PAD
}

/// The buttons centred across `r` from its top, the well under them and its option cells.
pub fn layout(tb: &ToolBox, r: Rect) -> Layout {
    let x0 = r.x + (r.width - COLS * CELL) / 2;
    let y0 = r.y + PAD;
    let tools = (0..tb.tools.len() as i32)
        .map(|i| Rect::new(x0 + i % COLS * CELL, y0 + i / COLS * CELL, CELL, CELL))
        .collect();
    let well = Rect::new(
        x0,
        y0 + tool_rows(tb) * CELL + WELL_GAP,
        COLS * CELL,
        WELL_H,
    );
    let inner = well.inset(WELL_EDGE);
    let cols = tb.option_cols.max(1) as i32;
    let rows = (tb.options.len() as i32 + cols - 1) / cols;
    let (cw, ch) = (inner.width / cols, inner.height / rows.max(1));
    let ox = inner.x + (inner.width - cw * cols) / 2;
    let options = (0..tb.options.len() as i32)
        .map(|i| Rect::new(ox + i % cols * cw, inner.y + i / cols * ch, cw, ch))
        .collect();
    Layout {
        bounds: r,
        tools,
        well,
        options,
    }
}

pub fn hit(lay: &Layout, x: i32, y: i32) -> Option<Hit> {
    let find = |rects: &[Rect]| rects.iter().position(|r| r.contains(x, y));
    find(&lay.tools)
        .map(Hit::Tool)
        .or_else(|| find(&lay.options).map(Hit::Option))
}

/// `glyph`'s coverage in `ink`, centred in device rect `r` and moved `off` right and down.
fn draw_glyph(c: &mut Canvas, r: Rect, glyph: Option<&Canvas>, ink: Color, off: i32) {
    let Some(g) = glyph else {
        return;
    };
    let (w, h) = g.device_size();
    let x = r.x + (r.width - w as i32) / 2 + off;
    let y = r.y + (r.height - h as i32) / 2 + off;
    c.blit_alpha(x, y, g, ink);
}

pub fn draw(c: &mut Canvas, p: &Paint, tb: &ToolBox, lay: &Layout) {
    let (cs, s) = (p.cs, p.s);
    p.fill_back(c, lay.bounds.scaled(s));
    for (i, (tool, r)) in tb.tools.iter().zip(&lay.tools).enumerate() {
        let state = look::state(
            true,
            tb.hot == Some(i),
            tb.pressed == Some(i),
            tb.current == i,
        );
        let sr = r.scaled(s);
        look::draw_button(c, p, sr, None, state);
        let off = look::content_shift(p, state);
        draw_glyph(c, sr, tool.glyph.as_ref(), cs.button_text, off);
    }
    let well = lay.well.scaled(s);
    c.fill_rect(well, cs.button_face);
    draw_field_frame(c, p, well, cs.button_face);
    for (i, (choice, r)) in tb.options.iter().zip(&lay.options).enumerate() {
        let sr = r.scaled(s);
        let ink = if tb.option == Some(i) {
            c.fill_rect(sr, cs.hilight);
            cs.hilight_text
        } else {
            cs.button_text
        };
        draw_glyph(c, sr, choice.glyph.as_ref(), ink, 0);
    }
}

/// A toolbar of toggle buttons named by their tools, the current one pressed, and a radio group
/// of the options.
pub fn access(tb: &ToolBox, lay: &Layout, tree: &mut Tree, parent: NodeId) -> NodeId {
    let bar = tree.push(
        parent,
        "toolbox",
        access::named(Role::Toolbar, lay.bounds, "Tools"),
    );
    for (i, (tool, &r)) in tb.tools.iter().zip(&lay.tools).enumerate() {
        let n = access::toggle(
            Role::Button,
            r,
            &tool.name,
            Toggled::from(tb.current == i),
            true,
        );
        tree.push(bar, ("tool", i), n);
    }
    let group = tree.push(bar, "options", access::node(Role::RadioGroup, lay.well));
    for (i, (choice, &r)) in tb.options.iter().zip(&lay.options).enumerate() {
        let n = access::toggle(
            Role::RadioButton,
            r,
            &choice.name,
            Toggled::from(tb.option == Some(i)),
            true,
        );
        tree.push(group, ("option", i), n);
    }
    bar
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interface::Interface;
    use crate::interfaces::theme_for;
    use crate::theme::Theme;

    fn fnv(c: &Canvas) -> u64 {
        [c.width, c.height]
            .into_iter()
            .chain(c.pixels.iter().copied())
            .fold(0xcbf2_9ce4_8422_2325_u64, |h, p| {
                p.to_le_bytes().iter().fold(h, |h, &b| {
                    (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
                })
            })
    }

    fn square(s: i32, inset: i32) -> Canvas {
        let mut g = Canvas::with_scale(16, 16, s);
        g.fill_rect(
            Rect::new(inset, inset, 16 - 2 * inset, 16 - 2 * inset),
            Color::rgb(0xFF, 0xFF, 0xFF),
        );
        g
    }

    fn sample(s: i32) -> ToolBox {
        ToolBox {
            tools: (0..6)
                .map(|i| Tool {
                    name: format!("Tool {i}"),
                    glyph: Some(square(s, 4)),
                })
                .collect(),
            current: 0,
            hot: None,
            pressed: None,
            options: (0..3)
                .map(|i| Choice {
                    name: format!("Size {i}"),
                    glyph: Some(square(s, 6 - i)),
                })
                .collect(),
            option_cols: 1,
            option: Some(1),
        }
    }

    fn themes() -> [(&'static str, Theme); 3] {
        [
            ("classic", Theme::default()),
            ("blue", theme_for(Interface::Blue)),
            ("glass", theme_for(Interface::Glass)),
        ]
    }

    fn drawn(theme: &Theme, tb: &ToolBox) -> (Canvas, Layout) {
        let s = theme.scale.max(1);
        let r = Rect::new(0, 0, WIDTH, height(tb));
        let lay = layout(tb, r);
        let mut c = Canvas::new((r.width * s) as u32, (r.height * s) as u32);
        draw(&mut c, &theme.paint(), tb, &lay);
        (c, lay)
    }

    fn cell(c: &Canvas, r: Rect) -> Vec<u32> {
        (r.y..r.bottom())
            .flat_map(|y| (r.x..r.right()).map(move |x| (x, y)))
            .map(|(x, y)| c.get(x, y))
            .collect()
    }

    #[test]
    fn each_state_is_pinned_in_every_look_at_scale_1_and_2() {
        let mut got = Vec::new();
        for (name, theme) in themes() {
            for s in [1, 2] {
                let theme = theme.scaled(s);
                let rest = sample(s);
                let states = [
                    ("rest", rest.clone()),
                    (
                        "hot",
                        ToolBox {
                            hot: Some(3),
                            ..rest.clone()
                        },
                    ),
                    (
                        "pressed",
                        ToolBox {
                            hot: Some(3),
                            pressed: Some(3),
                            ..rest.clone()
                        },
                    ),
                    (
                        "no options",
                        ToolBox {
                            current: 5,
                            options: vec![],
                            option: None,
                            ..rest.clone()
                        },
                    ),
                ];
                for (state, tb) in states {
                    got.push((format!("{name} {s} {state}"), fnv(&drawn(&theme, &tb).0)));
                }
            }
        }
        crate::pins::check("controls/toolbox", &got);
    }

    #[test]
    fn the_current_tool_is_latched_and_the_others_alike() {
        for (name, theme) in themes() {
            let (c, lay) = drawn(&theme, &sample(1));
            let (current, other) = (cell(&c, lay.tools[0]), cell(&c, lay.tools[2]));
            assert_ne!(current, other, "{name}: the current tool is drawn down");
            assert_eq!(
                other,
                cell(&c, lay.tools[4]),
                "{name}: tools at rest look alike"
            );
        }
    }

    #[test]
    fn glyphs_take_the_ink_and_the_picked_option_is_highlighted() {
        for (name, theme) in themes() {
            for s in [1, 2] {
                let theme = theme.scaled(s);
                let cs = &theme.colors;
                let (c, lay) = drawn(&theme, &sample(s));
                let mid = |r: Rect| (r.x + r.width / 2, r.y + r.height / 2);
                let (x, y) = mid(lay.tools[2].scaled(s));
                assert_eq!(
                    c.get(x, y),
                    cs.button_text.argb(),
                    "{name} {s}: a tool's glyph"
                );
                let picked = lay.options[1].scaled(s);
                assert_eq!(
                    c.get(picked.x, picked.y),
                    cs.hilight.argb(),
                    "{name} {s}: the picked option's cell"
                );
                let (x, y) = mid(picked);
                assert_eq!(
                    c.get(x, y),
                    cs.hilight_text.argb(),
                    "{name} {s}: the picked option's glyph"
                );
                let first = lay.options[0].scaled(s);
                let (x, y) = mid(first);
                assert_eq!(
                    c.get(x, y),
                    cs.button_text.argb(),
                    "{name} {s}: another option's glyph"
                );
                assert_ne!(
                    c.get(first.x, first.y),
                    cs.hilight.argb(),
                    "{name} {s}: another option's cell"
                );
            }
        }
    }

    #[test]
    fn hit_maps_each_rect_back_to_its_index() {
        let mut tb = sample(1);
        tb.options = (0..12)
            .map(|i| Choice {
                name: format!("{i}"),
                glyph: None,
            })
            .collect();
        tb.option_cols = 3;
        let lay = layout(&tb, Rect::new(10, 20, WIDTH, height(&tb)));
        for (i, r) in lay.tools.iter().enumerate() {
            assert_eq!(hit(&lay, r.x, r.y), Some(Hit::Tool(i)));
            assert_eq!(hit(&lay, r.right() - 1, r.bottom() - 1), Some(Hit::Tool(i)));
        }
        for (i, r) in lay.options.iter().enumerate() {
            assert_eq!(hit(&lay, r.x, r.y), Some(Hit::Option(i)));
            assert_eq!(
                hit(&lay, r.right() - 1, r.bottom() - 1),
                Some(Hit::Option(i))
            );
            assert!(lay.well.inset(WELL_EDGE).contains(r.x, r.y));
            assert!(
                lay.well
                    .inset(WELL_EDGE)
                    .contains(r.right() - 1, r.bottom() - 1)
            );
        }
        assert_eq!(hit(&lay, lay.well.x, lay.well.y), None, "the well's frame");
        assert_eq!(lay.options[3].y, lay.options[0].bottom(), "three per row");
    }

    #[test]
    fn access_names_the_tools_and_marks_the_current_and_the_option() {
        let tb = sample(1);
        let lay = layout(&tb, Rect::new(0, 0, WIDTH, height(&tb)));
        let mut tree = Tree::new(access::node(Role::Window, lay.bounds));
        access(&tb, &lay, &mut tree, access::ROOT);
        let toggled = |role, name: &str| {
            let id = tree.find(role, name).expect(name);
            tree.get(id).unwrap().toggled()
        };
        assert_eq!(toggled(Role::Button, "Tool 0"), Some(Toggled::True));
        assert_eq!(toggled(Role::Button, "Tool 1"), Some(Toggled::False));
        assert_eq!(toggled(Role::RadioButton, "Size 1"), Some(Toggled::True));
        assert_eq!(toggled(Role::RadioButton, "Size 0"), Some(Toggled::False));
    }
}
