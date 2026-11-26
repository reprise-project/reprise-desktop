//! Sticky Notes' program icon: a yellow note with three lines of writing and its lower right
//! corner folded up. The `note_icon` example writes it at [`SIZES`] into
//! `data/icons/hicolor/<n>x<n>/apps/reprise-notes.png`, which the packages install.

use crate::raster::Canvas;
use crate::shape::{self, Px, Stops, over, polygon, premul, stroke};
use crate::theme::Color;

/// The hicolor sizes the icon ships at, in px; 32 and 48 also serve 16 and 24 at scale 2.
pub const SIZES: [i32; 4] = [16, 24, 32, 48];

const PAPER_TOP: Color = Color::rgb(255, 245, 157);
const PAPER_FOOT: Color = Color::rgb(252, 224, 88);
const FOLD: Color = Color::rgb(232, 194, 64);
const EDGE: Color = Color::rgb(176, 136, 24);
const WRITING: Color = Color::rgb(150, 116, 36);

/// The design grid the geometry is laid on, scaled to each size.
const GRID: f64 = 16.0;
/// The note's outer edge and where the fold starts, on the grid; the outline is centred on it.
const NEAR: f64 = 1.5;
const FAR: f64 = 14.5;
const FOLD_AT: f64 = 10.5;
/// Each line of writing: its y and the x it runs to, from x 4.
const LINES: [(f64, f64); 3] = [(5.5, 12.0), (7.5, 12.0), (9.5, 9.0)];

/// The icon `px` x `px`, drawn afresh at each size with anti-aliased edges.
pub fn draw(px: i32) -> Canvas {
    let k = f64::from(px) / GRID;
    let at = |pts: &[(f64, f64)]| -> Vec<(f64, f64)> {
        pts.iter().map(|&(x, y)| (x * k, y * k)).collect()
    };
    let body = at(&[
        (NEAR, NEAR),
        (FAR, NEAR),
        (FAR, FOLD_AT),
        (FOLD_AT, FAR),
        (NEAR, FAR),
    ]);
    let outline = at(&[
        (NEAR, NEAR),
        (FAR, NEAR),
        (FAR, FOLD_AT),
        (FOLD_AT, FAR),
        (NEAR, FAR),
        (NEAR, NEAR),
    ]);
    let fold = at(&[(FOLD_AT, FAR), (FOLD_AT, FOLD_AT), (FAR, FOLD_AT)]);
    let lines: Vec<Vec<(f64, f64)>> = LINES
        .iter()
        .map(|&(y, to)| at(&[(4.0, y), (to, y)]))
        .collect();
    let solid = |c: Color| premul(c, 255);
    let paper = [(0.0, solid(PAPER_TOP)), (1.0, solid(PAPER_FOOT))];
    let ink = |c: Color, cover: f64| shape::scale(solid(c), cover);
    shape::draw(px, px, 1, |x, y| {
        let t = ((y + 0.5) / k - NEAR) / (FAR - NEAR);
        let mut p: Px = shape::scale(Stops(&paper).at(t), polygon(x, y, &body));
        for l in &lines {
            p = over(ink(WRITING, stroke(x, y, l, k)), p);
        }
        p = over(ink(FOLD, polygon(x, y, &fold)), p);
        p = over(ink(EDGE, stroke(x, y, &fold, k)), p);
        over(ink(EDGE, stroke(x, y, &outline, k)), p)
    })
}

/// `c`'s pixels as straight-alpha RGBA bytes, as a PNG holds them.
pub fn rgba(c: &Canvas) -> Vec<u8> {
    c.pixels
        .iter()
        .flat_map(|&p| {
            let a = p >> 24;
            let ch = |shift: u32| match a {
                0 => 0,
                _ => (((p >> shift) & 0xFF) * 255 + a / 2) / a,
            };
            [ch(16) as u8, ch(8) as u8, ch(0) as u8, a as u8]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alpha(c: &Canvas, x: i32, y: i32) -> u32 {
        c.get(x, y) >> 24
    }

    #[test]
    fn the_note_icon_pins_at_every_size() {
        let got: Vec<_> = SIZES
            .iter()
            .map(|&n| (format!("{n}"), crate::pins::fnv(&draw(n))))
            .collect();
        crate::pins::check("art/noteicon", &got);
    }

    #[test]
    fn the_corners_outside_the_note_and_past_the_fold_are_clear() {
        for n in SIZES {
            let c = draw(n);
            let last = n - 1;
            assert_eq!(alpha(&c, 0, 0), 0, "{n}: top left");
            assert_eq!(alpha(&c, last, 0), 0, "{n}: top right");
            assert_eq!(alpha(&c, 0, last), 0, "{n}: bottom left");
            let past = n * 15 / 16 - 1;
            assert_eq!(
                alpha(&c, past, past),
                0,
                "{n}: the folded corner is cut away"
            );
        }
    }

    #[test]
    fn the_paper_is_opaque_yellow_and_the_fold_darker() {
        for n in SIZES {
            let c = draw(n);
            let paper = c.get(n * 3 / 16, n * 12 / 16);
            assert_eq!(paper >> 24, 255, "{n}: opaque paper");
            let (r, g, b) = ((paper >> 16) & 0xFF, (paper >> 8) & 0xFF, paper & 0xFF);
            assert!(
                r > 240 && g > 200 && b < 140,
                "{n}: yellow, got {paper:08x}"
            );
            let fold = c.get(n * 12 / 16, n * 12 / 16);
            assert!(
                (fold >> 8) & 0xFF < g,
                "{n}: the fold is darker than the paper, got {fold:08x}"
            );
        }
    }

    #[test]
    fn the_slanted_fold_edge_is_anti_aliased_at_24_and_48() {
        for n in [24, 48] {
            let c = draw(n);
            let partial = (0..n)
                .flat_map(|y| (0..n).map(move |x| (x, y)))
                .filter(|&(x, y)| x > n / 2 && y > n / 2)
                .any(|(x, y)| (1..255).contains(&alpha(&c, x, y)));
            assert!(partial, "{n}: partial alpha along the fold");
        }
    }

    #[test]
    fn the_shipped_pngs_are_the_drawn_icon() {
        let root =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/icons/hicolor");
        for n in SIZES {
            let path = root.join(format!("{n}x{n}/apps/reprise-notes.png"));
            let file = std::fs::File::open(&path).unwrap_or_else(|e| {
                panic!(
                    "{}: {e}; run `cargo run -p reprisekit --example note_icon`",
                    path.display()
                )
            });
            let mut png = png::Decoder::new(std::io::BufReader::new(file))
                .read_info()
                .unwrap();
            let mut buf = vec![0; png.output_buffer_size().unwrap()];
            let info = png.next_frame(&mut buf).unwrap();
            assert_eq!((info.width, info.height), (n as u32, n as u32));
            assert_eq!(info.color_type, png::ColorType::Rgba);
            assert!(
                buf[..info.buffer_size()] == rgba(&draw(n)),
                "{}: rerun `cargo run -p reprisekit --example note_icon`",
                path.display()
            );
        }
    }
}
