//! A raster editor's colour box: the primary and secondary colours in a sunken box, then the
//! swatches in rows of sunken wells. Layout is logical; `draw` scales by `s`.

use crate::access::{self, Tree, accesskit};
use crate::look::Paint;
use crate::look::widgets::draw_field_frame;
use crate::raster::{Canvas, Rect};
use crate::theme::Color;
use accesskit::{NodeId, Role};

/// The height a host reserves for the strip.
pub const HEIGHT: i32 = 48;
const PAD: i32 = 4;
const CURRENT: (i32, i32) = (30, 31);
const CHIP: i32 = 15;
const PRIMARY_AT: (i32, i32) = (4, 4);
const SECONDARY_AT: (i32, i32) = (11, 12);
const GAP: i32 = 4;
pub const SWATCH: i32 = 16;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Palette {
    pub swatches: Vec<(Color, String)>,
    pub rows: usize,
    pub primary: Color,
    pub secondary: Color,
}

impl Palette {
    /// Swatch `k` becomes the primary colour, or the secondary one; false when nothing changed.
    pub fn pick(&mut self, k: usize, secondary: bool) -> bool {
        let Some(&(c, _)) = self.swatches.get(k) else {
            return false;
        };
        let slot = if secondary {
            &mut self.secondary
        } else {
            &mut self.primary
        };
        std::mem::replace(slot, c) != c
    }

    /// `c`'s swatch name, else `#rrggbb`.
    pub fn name_of(&self, c: Color) -> String {
        self.swatches
            .iter()
            .find(|(sc, _)| *sc == c)
            .map(|(_, n)| n.clone())
            .unwrap_or_else(|| format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub bounds: Rect,
    pub current: Rect,
    pub swatches: Vec<Rect>,
}

/// The current-colours box at `r`'s left, the swatches right of it, both centred down `r`.
pub fn layout(pal: &Palette, r: Rect) -> Layout {
    let rows = pal.rows.max(1) as i32;
    let cols = (pal.swatches.len() as i32 + rows - 1) / rows;
    let block = CURRENT.1.max(rows * SWATCH);
    let y0 = r.y + (r.height - block) / 2;
    let current = Rect::new(
        r.x + PAD,
        y0 + (block - CURRENT.1) / 2,
        CURRENT.0,
        CURRENT.1,
    );
    let (sx, sy) = (current.right() + GAP, y0 + (block - rows * SWATCH) / 2);
    let swatches = (0..pal.swatches.len() as i32)
        .map(|k| {
            let (col, row) = (k % cols.max(1), k / cols.max(1));
            Rect::new(sx + col * SWATCH, sy + row * SWATCH, SWATCH, SWATCH)
        })
        .collect();
    Layout {
        bounds: r,
        current,
        swatches,
    }
}

/// The swatch under (`x`, `y`).
pub fn hit(lay: &Layout, x: i32, y: i32) -> Option<usize> {
    lay.swatches.iter().position(|r| r.contains(x, y))
}

fn well(c: &mut Canvas, p: &Paint, r: Rect, fill: Color) {
    let r = r.scaled(p.s);
    c.fill_rect(r, fill);
    draw_field_frame(c, p, r, fill);
}

pub fn draw(c: &mut Canvas, p: &Paint, pal: &Palette, lay: &Layout) {
    p.fill_back(c, lay.bounds.scaled(p.s));
    let cur = lay.current;
    well(c, p, cur, p.cs.button_face);
    let chip = |(x, y): (i32, i32)| Rect::new(cur.x + x, cur.y + y, CHIP, CHIP);
    well(c, p, chip(SECONDARY_AT), pal.secondary);
    well(c, p, chip(PRIMARY_AT), pal.primary);
    for ((color, _), &r) in pal.swatches.iter().zip(&lay.swatches) {
        well(c, p, r, *color);
    }
}

/// A group holding the current colours, read by name, and a button per swatch.
pub fn access(pal: &Palette, lay: &Layout, tree: &mut Tree, parent: NodeId) -> NodeId {
    let group = tree.push(
        parent,
        "palette",
        access::named(Role::Group, lay.bounds, "Colors"),
    );
    let label = format!(
        "Primary {}, secondary {}",
        pal.name_of(pal.primary),
        pal.name_of(pal.secondary)
    );
    tree.push(
        group,
        "current",
        access::titled(Role::ColorWell, lay.current, &label),
    );
    for (k, ((_, name), &r)) in pal.swatches.iter().zip(&lay.swatches).enumerate() {
        tree.push(group, ("swatch", k), access::button(r, name, true, false));
    }
    group
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

    const BLACK: Color = Color::rgb(0, 0, 0);
    const WHITE: Color = Color::rgb(0xFF, 0xFF, 0xFF);
    const RED: Color = Color::rgb(0xFF, 0, 0);

    fn sample() -> Palette {
        let swatches = [
            (BLACK, "Black"),
            (WHITE, "White"),
            (Color::rgb(0x80, 0x80, 0x80), "Gray"),
            (Color::rgb(0xC0, 0xC0, 0xC0), "Silver"),
            (Color::rgb(0x80, 0, 0), "Maroon"),
            (RED, "Red"),
            (Color::rgb(0, 0x80, 0), "Green"),
            (Color::rgb(0, 0xFF, 0), "Lime"),
        ];
        Palette {
            swatches: swatches
                .into_iter()
                .map(|(c, n)| (c, n.to_string()))
                .collect(),
            rows: 2,
            primary: BLACK,
            secondary: WHITE,
        }
    }

    fn themes() -> [(&'static str, Theme); 3] {
        [
            ("classic", Theme::default()),
            ("blue", theme_for(Interface::Blue)),
            ("glass", theme_for(Interface::Glass)),
        ]
    }

    fn drawn(theme: &Theme, pal: &Palette) -> (Canvas, Layout) {
        let s = theme.scale.max(1);
        let lay = layout(pal, Rect::new(0, 0, 120, HEIGHT));
        let mut c = Canvas::new((120 * s) as u32, (HEIGHT * s) as u32);
        draw(&mut c, &theme.paint(), pal, &lay);
        (c, lay)
    }

    #[test]
    fn each_state_is_pinned_in_every_look_at_scale_1_and_2() {
        let mut got = Vec::new();
        for (name, theme) in themes() {
            for s in [1, 2] {
                let theme = theme.scaled(s);
                let swapped = Palette {
                    primary: RED,
                    secondary: Color::rgb(0x12, 0x34, 0x56),
                    ..sample()
                };
                for (state, pal) in [("rest", sample()), ("custom", swapped)] {
                    got.push((format!("{name} {s} {state}"), fnv(&drawn(&theme, &pal).0)));
                }
            }
        }
        crate::pins::check("controls/palette", &got);
    }

    #[test]
    fn the_primary_lies_over_the_secondary_and_swatches_hold_their_colours() {
        let pal = Palette {
            primary: RED,
            ..sample()
        };
        for (name, theme) in themes() {
            for s in [1, 2] {
                let theme = theme.scaled(s);
                let (c, lay) = drawn(&theme, &pal);
                let cur = lay.current.scaled(s);
                let at = |x: i32, y: i32| c.get(cur.x + x * s, cur.y + y * s);
                assert_eq!(at(13, 14), RED.argb(), "{name} {s}: the overlap is primary");
                assert_eq!(at(7, 7), RED.argb(), "{name} {s}: the primary's corner");
                assert_eq!(at(22, 23), WHITE.argb(), "{name} {s}: the secondary shows");
                for ((color, _), r) in pal.swatches.iter().zip(&lay.swatches) {
                    let (x, y) = ((r.x + SWATCH / 2) * s, (r.y + SWATCH / 2) * s);
                    assert_eq!(c.get(x, y), color.argb(), "{name} {s}: swatch at {r:?}");
                }
            }
        }
    }

    #[test]
    fn hit_maps_each_swatch_back_to_its_index() {
        let pal = sample();
        let lay = layout(&pal, Rect::new(5, 300, 200, HEIGHT));
        assert_eq!(lay.swatches[4].y, lay.swatches[0].bottom(), "four per row");
        for (k, r) in lay.swatches.iter().enumerate() {
            assert_eq!(hit(&lay, r.x, r.y), Some(k));
            assert_eq!(hit(&lay, r.right() - 1, r.bottom() - 1), Some(k));
        }
        let cur = lay.current;
        assert_eq!(hit(&lay, cur.x + 5, cur.y + 5), None, "the current box");
    }

    #[test]
    fn pick_sets_the_primary_or_the_secondary() {
        let mut pal = sample();
        assert!(pal.pick(5, false) && pal.primary == RED);
        assert!(!pal.pick(5, false), "the same colour again");
        assert!(pal.pick(0, true) && pal.secondary == BLACK);
        assert!(!pal.pick(99, true));
    }

    #[test]
    fn access_reads_the_current_colours_by_name_or_hex() {
        let pal = Palette {
            secondary: Color::rgb(0x12, 0x34, 0x56),
            ..sample()
        };
        let lay = layout(&pal, Rect::new(0, 0, 120, HEIGHT));
        let mut tree = Tree::new(access::node(Role::Window, lay.bounds));
        access(&pal, &lay, &mut tree, access::ROOT);
        assert!(
            tree.find(Role::ColorWell, "Primary Black, secondary #123456")
                .is_some()
        );
        assert!(tree.find(Role::Button, "Lime").is_some());
    }
}
