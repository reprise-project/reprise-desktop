use crate::raster::Rect;

/// A cell, the gap between cells and the padding round them, logical px.
pub const CELL: (i32, i32) = (98, 64);
pub const GAP: i32 = 12;
pub const PAD: i32 = 13;
pub const TILE_GAP: i32 = 4;
pub const TILE_RADIUS: i32 = 4;
pub const PANEL_RADIUS: i32 = 8;
/// [`PANEL_RADIUS`] as a corner table (`backdrop::corner_radius`): what
/// each of the top and bottom rows leaves clear at either end.
pub const PANEL_CORNER: &[i32] = &[6, 4, 3, 2, 1, 1, 1];
pub const COLS: usize = 3;
pub const ICON_PX: i32 = 16;

pub type Zone = (usize, usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Grid,
    Row,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CellSpec {
    pub zones: Vec<Rect>,
    pub suggestion: bool,
    pub filled: Vec<(usize, String)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PickerLayout {
    pub width: i32,
    pub height: i32,
    pub cells: Vec<Rect>,
    pub tiles: Vec<Vec<Rect>>,
    pub filled: Vec<Vec<bool>>,
    pub suggestion: Vec<bool>,
    pub scale: i32,
}

impl PickerLayout {
    pub fn compute(shape: Shape, work: (i32, i32), specs: &[CellSpec]) -> PickerLayout {
        let n = specs.len().max(1);
        let cols = match shape {
            Shape::Grid => n.min(COLS),
            Shape::Row => n,
        };
        let rows = n.div_ceil(cols) as i32;
        let cols = cols as i32;
        let (cw, ch) = CELL;
        let cells: Vec<Rect> = (0..specs.len() as i32)
            .map(|i| {
                Rect::new(
                    PAD + (i % cols) * (cw + GAP),
                    PAD + (i / cols) * (ch + GAP),
                    cw,
                    ch,
                )
            })
            .collect();
        let tiles = (specs.iter().zip(&cells))
            .map(|(s, c)| s.zones.iter().map(|z| tile(*c, *z, work)).collect())
            .collect();
        let filled = (specs.iter())
            .map(|s| {
                (0..s.zones.len())
                    .map(|z| s.filled.iter().any(|f| f.0 == z))
                    .collect()
            })
            .collect();
        PickerLayout {
            width: 2 * PAD + cols * cw + (cols - 1) * GAP,
            height: 2 * PAD + rows * ch + (rows - 1) * GAP,
            cells,
            tiles,
            filled,
            suggestion: specs.iter().map(|s| s.suggestion).collect(),
            scale: 1,
        }
    }

    pub fn scaled(&self, n: i32) -> PickerLayout {
        let n = n.max(1);
        PickerLayout {
            width: self.width * n,
            height: self.height * n,
            cells: self.cells.iter().map(|r| r.scaled(n)).collect(),
            tiles: (self.tiles.iter())
                .map(|ts| ts.iter().map(|r| r.scaled(n)).collect())
                .collect(),
            filled: self.filled.clone(),
            suggestion: self.suggestion.clone(),
            scale: self.scale * n,
        }
    }

    pub fn hit(&self, x: i32, y: i32) -> Option<Zone> {
        self.order()
            .into_iter()
            .find(|&(l, z)| self.tiles[l][z].contains(x, y))
    }

    pub fn order(&self) -> Vec<Zone> {
        (self.tiles.iter().enumerate())
            .flat_map(|(l, ts)| (0..ts.len()).map(move |z| (l, z)))
            .filter(|&(l, z)| !self.filled[l][z])
            .collect()
    }

    /// Whether cell `l` holds the lit tile `hot`: its filled tiles then
    /// draw their edge lit.
    pub fn cell_lit(&self, hot: Option<Zone>, l: usize) -> bool {
        hot.is_some_and(|(h, _)| h == l)
    }
}

/// Zone `z` of a work area `work` big as a tile of `cell`: each side at
/// its share of the cell's side plus one [`TILE_GAP`], the far sides
/// giving the gap back, so an even split's tiles are equal, stand
/// `TILE_GAP` apart and meet the cell's edges.
fn tile(cell: Rect, z: Rect, work: (i32, i32)) -> Rect {
    let at = |v: i32, len: i32, total: i32| {
        let total = i64::from(total.max(1));
        ((i64::from(v) * i64::from(len + TILE_GAP) + total / 2) / total) as i32
    };
    let x0 = at(z.x, cell.width, work.0);
    let x1 = at(z.right(), cell.width, work.0) - TILE_GAP;
    let y0 = at(z.y, cell.height, work.1);
    let y1 = at(z.bottom(), cell.height, work.1) - TILE_GAP;
    Rect::new(cell.x + x0, cell.y + y0, x1 - x0, y1 - y0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORK: (i32, i32) = (1920, 1050);

    fn cols(cuts: &[i32]) -> Vec<Rect> {
        cuts.windows(2)
            .map(|p| Rect::new(p[0], 0, p[1] - p[0], 1050))
            .collect()
    }

    fn fixed() -> Vec<CellSpec> {
        let spec = |zones| CellSpec {
            zones,
            ..CellSpec::default()
        };
        vec![
            spec(cols(&[0, 960, 1920])),
            spec(cols(&[0, 1280, 1920])),
            spec(cols(&[0, 640, 1280, 1920])),
            spec(vec![
                Rect::new(0, 0, 960, 1050),
                Rect::new(960, 0, 960, 525),
                Rect::new(960, 525, 960, 525),
            ]),
            spec(cols(&[0, 480, 1440, 1920])),
        ]
    }

    #[test]
    fn tiles_of_an_even_split_are_equal_and_four_apart() {
        let l = PickerLayout::compute(Shape::Grid, WORK, &fixed());
        assert_eq!(l.cells[0], Rect::new(13, 13, 98, 64));
        assert_eq!(
            l.tiles[0],
            vec![Rect::new(13, 13, 47, 64), Rect::new(64, 13, 47, 64)]
        );
        let w = |l: &PickerLayout, c: usize| l.tiles[c].iter().map(|t| t.width).collect::<Vec<_>>();
        assert_eq!(w(&l, 1), vec![64, 30], "two thirds and a third");
        assert_eq!(w(&l, 2), vec![30, 30, 30], "thirds");
        assert_eq!(w(&l, 4), vec![22, 47, 21], "a wide middle");
        let stack = &l.tiles[3];
        assert_eq!((stack[1].height, stack[2].height), (30, 30));
        assert_eq!(stack[2].y - stack[1].bottom(), TILE_GAP);
    }

    #[test]
    fn a_grid_is_three_wide_and_a_row_is_one_high() {
        let nine: Vec<CellSpec> = fixed().into_iter().cycle().take(9).collect();
        let g = PickerLayout::compute(Shape::Grid, WORK, &nine);
        assert_eq!((g.width, g.height), (344, 242));
        assert_eq!(g.cells[4], Rect::new(123, 89, 98, 64), "the middle cell");
        let eight: Vec<CellSpec> = fixed().into_iter().cycle().take(8).collect();
        let r = PickerLayout::compute(Shape::Row, WORK, &eight);
        assert_eq!((r.width, r.height), (894, 90));
        assert_eq!(r.cells[7], Rect::new(783, 13, 98, 64));
    }

    #[test]
    fn filled_zones_are_inert() {
        let mut specs = fixed();
        specs[0].suggestion = true;
        specs[0].filled = vec![(0, "Notepad".into())];
        let l = PickerLayout::compute(Shape::Grid, WORK, &specs);
        assert_eq!(l.hit(20, 30), None, "the filled half");
        assert_eq!(l.hit(70, 30), Some((0, 1)));
        assert_eq!(l.hit(61, 30), None, "the 4 px between tiles");
        assert_eq!(l.order()[0], (0, 1), "the keys skip it");
        assert!(l.cell_lit(Some((0, 1)), 0));
        let l2 = l.scaled(2);
        assert_eq!(l2.hit(140, 60), Some((0, 1)));
        assert_eq!(l2.tiles[0][1], Rect::new(128, 26, 94, 128));
    }

    #[test]
    fn the_panel_corner_table_steps_an_eight_px_circle() {
        let r = f64::from(crate::backdrop::corner_radius(PANEL_CORNER));
        assert_eq!(r, f64::from(PANEL_RADIUS));
        for (row, &want) in PANEL_CORNER.iter().enumerate() {
            let dy = r - (row as f64 + 0.5);
            assert_eq!(
                (r - (r * r - dy * dy).sqrt()).ceil() as i32,
                want,
                "row {row}"
            );
        }
    }
}
