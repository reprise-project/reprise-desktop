//! The snap layouts panel's model (the lit tile, by the pointer or the
//! keys), its access part, and Classic's and Blue's painter: the look's
//! flyout frame with flat tiles. The geometry is the root
//! [`crate::snaplayouts`], re-exported here.

pub use crate::snaplayouts::*;

use crate::access::accesskit::{NodeId, Role};
use crate::access::{self, Tree};
use crate::flyout;
use crate::raster::{Canvas, Rect};
use crate::theme::Theme;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Left,
    Right,
    Up,
    Down,
    Enter,
    Escape,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    None,
    Redraw,
    Pick(Zone),
    Close,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SnapPicker {
    pub hot: Option<Zone>,
}

impl SnapPicker {
    /// The pointer is on free tile `zone` (None: off every free tile).
    /// Leaving the tiles keeps the lit one, so the keys go on from it.
    pub fn hover(&mut self, zone: Option<Zone>) -> Event {
        match zone {
            Some(z) if self.hot != Some(z) => {
                self.hot = Some(z);
                Event::Redraw
            }
            _ => Event::None,
        }
    }

    pub fn click(&self, zone: Option<Zone>) -> Event {
        zone.map_or(Event::None, Event::Pick)
    }

    /// Right and Left step through the free tiles in reading order, round
    /// from the last to the first; Down and Up go to the first free tile
    /// of the next or previous cell that has one. With nothing lit, any
    /// arrow lights the first free tile. Enter picks the lit tile, Escape
    /// closes.
    pub fn key(&mut self, layout: &PickerLayout, key: Key) -> Event {
        let order = layout.order();
        if order.is_empty() {
            return if key == Key::Escape {
                Event::Close
            } else {
                Event::None
            };
        }
        let n = layout.tiles.len();
        let first_in = |l: usize| order.iter().copied().find(|&(c, _)| c == l);
        let next = match (key, self.hot) {
            (Key::Escape, _) => return Event::Close,
            (Key::Enter, hot) => return hot.map_or(Event::None, Event::Pick),
            (_, None) => order[0],
            (Key::Right | Key::Left, Some(h)) => {
                let i = order.iter().position(|&z| z == h).unwrap_or(0);
                let step = if key == Key::Right {
                    1
                } else {
                    order.len() - 1
                };
                order[(i + step) % order.len()]
            }
            (Key::Down, Some((l, _))) => (1..=n)
                .map(|k| (l + k) % n)
                .find_map(first_in)
                .unwrap_or(order[0]),
            (Key::Up, Some((l, _))) => (1..=n)
                .map(|k| (l + n - k) % n)
                .find_map(first_in)
                .unwrap_or(order[0]),
        };
        self.hot = Some(next);
        Event::Redraw
    }
}

/// Classic's and Blue's panel, `layout` in device pixels at
/// `theme.scale`: the look's flyout frame in the face colour, each tile
/// flat in the face colour inside a 1 px `button_shadow` edge, the lit
/// one `hilight`; a filled tile `button_shadow` with its window's icon
/// (`icons`) centred, its edge `hilight` while its cell holds the lit
/// tile.
pub fn draw(
    theme: &Theme,
    layout: &PickerLayout,
    icons: &[(Zone, &Canvas)],
    hot: Option<Zone>,
) -> Canvas {
    let cs = &theme.colors;
    let s = layout.scale.max(1);
    let mut c = Canvas::new(layout.width.max(1) as u32, layout.height.max(1) as u32);
    let all = Rect::new(0, 0, layout.width, layout.height);
    flyout::draw_panel(theme, &mut c, all, cs.button_face, flyout::Backdrop::None);
    for (l, tiles) in layout.tiles.iter().enumerate() {
        let lit_cell = layout.cell_lit(hot, l);
        for (z, r) in tiles.iter().enumerate() {
            let filled = layout.filled[l][z];
            let (edge, fill) = if hot == Some((l, z)) {
                (cs.hilight, cs.hilight)
            } else if filled {
                let edge = if lit_cell {
                    cs.hilight
                } else {
                    cs.button_shadow
                };
                (edge, cs.button_shadow)
            } else {
                (cs.button_shadow, cs.button_face)
            };
            c.fill_rect(*r, edge);
            c.fill_rect(r.inset(s), fill);
            if let Some((_, icon)) = icons.iter().find(|(k, _)| *k == (l, z)) {
                let (w, h) = (icon.width as i32, icon.height as i32);
                c.blit_over(r.x + (r.width - w) / 2, r.y + (r.height - h) / 2, icon);
            }
        }
    }
    c
}

/// The panel's access part, from its top-left: a group per cell, "Layout
/// 2 of 9" ("..., suggestion" for a suggestion), holding a button per
/// zone, "Zone 1 of 3" ("..., Notepad" for a filled zone, which takes no
/// focus). Returns the lit tile's node, else `parent`.
pub fn access(
    layout: &PickerLayout,
    specs: &[CellSpec],
    hot: Option<Zone>,
    tree: &mut Tree,
    parent: NodeId,
) -> NodeId {
    let n = layout.cells.len();
    let mut focus = parent;
    for (l, (cell, tiles)) in layout.cells.iter().zip(&layout.tiles).enumerate() {
        let name = if layout.suggestion[l] {
            format!("Layout {} of {n}, suggestion", l + 1)
        } else {
            format!("Layout {} of {n}", l + 1)
        };
        let group = tree.push(parent, l, access::named(Role::Group, *cell, &name));
        for (z, r) in tiles.iter().enumerate() {
            let title = (specs.get(l))
                .and_then(|s| s.filled.iter().find(|f| f.0 == z))
                .map(|f| f.1.as_str());
            let label = match title {
                Some(t) => format!("Zone {} of {}, {t}", z + 1, tiles.len()),
                None => format!("Zone {} of {}", z + 1, tiles.len()),
            };
            let mut node = access::named(Role::Button, *r, &label);
            access::focusable(&mut node, title.is_none());
            let id = tree.push(group, z, node);
            if hot == Some((l, z)) {
                focus = id;
            }
        }
    }
    focus
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interface::Interface;
    use crate::interfaces::theme_for;
    use crate::theme::Color;

    fn sample() -> Vec<CellSpec> {
        let cols = |cuts: &[i32]| -> Vec<Rect> {
            cuts.windows(2)
                .map(|p| Rect::new(p[0], 0, p[1] - p[0], 1050))
                .collect()
        };
        let spec = |zones| CellSpec {
            zones,
            ..CellSpec::default()
        };
        let stack = vec![
            Rect::new(0, 0, 960, 1050),
            Rect::new(960, 0, 960, 525),
            Rect::new(960, 525, 960, 525),
        ];
        let quarters = vec![
            Rect::new(0, 0, 960, 525),
            Rect::new(0, 525, 960, 525),
            Rect::new(960, 0, 960, 525),
            Rect::new(960, 525, 960, 525),
        ];
        vec![
            CellSpec {
                zones: cols(&[0, 960, 1920]),
                suggestion: true,
                filled: vec![(0, "Notepad".into())],
            },
            spec(cols(&[0, 960, 1920])),
            spec(cols(&[0, 1280, 1920])),
            spec(cols(&[0, 640, 1280, 1920])),
            spec(stack),
            spec(quarters),
            spec(cols(&[0, 480, 1440, 1920])),
        ]
    }

    const WORK: (i32, i32) = (1920, 1050);

    fn icon() -> Canvas {
        let mut c = Canvas::new(16, 16);
        c.fill_rect(c.bounds(), Color::rgb(0x31, 0x6A, 0xC5));
        c
    }

    #[test]
    fn the_arrows_walk_the_free_zones_and_enter_picks() {
        let l = PickerLayout::compute(Shape::Grid, WORK, &sample());
        let mut p = SnapPicker::default();
        assert_eq!(p.key(&l, Key::Enter), Event::None, "nothing lit");
        assert_eq!(p.key(&l, Key::Right), Event::Redraw);
        assert_eq!(p.hot, Some((0, 1)), "the suggestion's free half first");
        p.key(&l, Key::Right);
        assert_eq!(p.hot, Some((1, 0)));
        p.key(&l, Key::Left);
        p.key(&l, Key::Left);
        assert_eq!(p.hot, Some((6, 2)), "round to the last zone");
        p.key(&l, Key::Down);
        assert_eq!(p.hot, Some((0, 1)), "the next cell's first free zone");
        p.key(&l, Key::Up);
        assert_eq!(p.hot, Some((6, 0)));
        assert_eq!(p.key(&l, Key::Enter), Event::Pick((6, 0)));
        assert_eq!(p.key(&l, Key::Escape), Event::Close);
    }

    #[test]
    fn the_pointer_lights_a_zone_and_a_click_picks_it() {
        let mut p = SnapPicker::default();
        assert_eq!(p.hover(Some((1, 1))), Event::Redraw);
        assert_eq!(p.hover(Some((1, 1))), Event::None);
        assert_eq!(p.hover(None), Event::None);
        assert_eq!(p.hot, Some((1, 1)), "kept for the keys");
        assert_eq!(p.click(Some((0, 1))), Event::Pick((0, 1)));
        assert_eq!(p.click(None), Event::None);
    }

    /// Classic: a raised panel, flat tiles in the face colour with a 1 px
    /// shadow edge, the lit tile `hilight`, a filled tile `button_shadow`
    /// with the window's icon and, beside the lit tile, a `hilight` edge.
    #[test]
    fn classic_draws_flat_tiles_on_its_raised_panel() {
        let t = Theme::default();
        let cs = &t.colors;
        let l = PickerLayout::compute(Shape::Grid, WORK, &sample());
        let i = icon();
        let c = draw(&t, &l, &[((0, 0), &i)], Some((0, 1)));
        assert_eq!((c.width as i32, c.height as i32), (344, 242));
        assert_eq!(c.get(1, 1), cs.button_hilight.argb(), "the raised edge");
        let free = l.tiles[1][0];
        assert_eq!(
            c.get(free.x, free.y),
            cs.button_shadow.argb(),
            "the tile's edge"
        );
        assert_eq!(c.get(free.x + 5, free.y + 5), cs.button_face.argb());
        let lit = l.tiles[0][1];
        assert_eq!(c.get(lit.x, lit.y), cs.hilight.argb());
        assert_eq!(c.get(lit.x + 5, lit.y + 5), cs.hilight.argb());
        let filled = l.tiles[0][0];
        assert_eq!(
            c.get(filled.x, filled.y),
            cs.hilight.argb(),
            "lit beside the hot tile"
        );
        assert_eq!(c.get(filled.x + 2, filled.y + 2), cs.button_shadow.argb());
        let mid = (filled.x + filled.width / 2, filled.y + filled.height / 2);
        assert_eq!(c.get(mid.0, mid.1), i.get(8, 8), "the icon in the middle");
    }

    #[test]
    fn classic_doubles_at_scale_2() {
        let t1 = Theme::default();
        let t2 = t1.scaled(2);
        let l = PickerLayout::compute(Shape::Grid, WORK, &sample());
        let a = draw(&t1, &l, &[], Some((5, 2)));
        let b = draw(&t2, &l.scaled(2), &[], Some((5, 2)));
        for y in 0..l.height {
            for x in 0..l.width {
                assert_eq!(b.get(2 * x + 1, 2 * y + 1), a.get(x, y), "({x}, {y})");
            }
        }
    }

    #[test]
    fn blue_draws_flat_tiles_in_its_colours() {
        let t = theme_for(Interface::Blue);
        let cs = &t.colors;
        let l = PickerLayout::compute(Shape::Grid, WORK, &sample());
        let c = draw(&t, &l, &[], Some((2, 1)));
        let lit = l.tiles[2][1];
        assert_eq!(c.get(lit.x + 3, lit.y + 3), cs.hilight.argb());
        assert_eq!(
            cs.hilight,
            crate::blue::Variant::Blue.scheme().system.hilight
        );
        let free = l.tiles[3][0];
        assert_eq!(c.get(free.x + 3, free.y + 3), cs.button_face.argb());
    }

    #[test]
    fn the_bar_row_draws_every_cell_in_one_row() {
        let t = Theme::default();
        let l = PickerLayout::compute(Shape::Row, WORK, &sample());
        let c = draw(&t, &l, &[], Some((6, 2)));
        assert_eq!(
            (c.width as i32, c.height as i32),
            (13 * 2 + 7 * 98 + 6 * 12, 90)
        );
        let lit = l.tiles[6][2];
        assert_eq!(c.get(lit.x + 3, lit.y + 3), t.colors.hilight.argb());
    }

    #[test]
    fn access_counts_layouts_and_names_filled_zones() {
        let l = PickerLayout::compute(Shape::Grid, WORK, &sample());
        let mut tree = Tree::window("Snap layouts", (l.width, l.height));
        let focus = access(&l, &sample(), Some((5, 3)), &mut tree, access::ROOT);
        assert!(
            tree.find(Role::Group, "Layout 1 of 7, suggestion")
                .is_some()
        );
        assert!(tree.find(Role::Group, "Layout 6 of 7").is_some());
        assert!(tree.find(Role::Button, "Zone 1 of 2, Notepad").is_some());
        assert_eq!(tree.get(focus).unwrap().label(), Some("Zone 4 of 4"));
    }
}
