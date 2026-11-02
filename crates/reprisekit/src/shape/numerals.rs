//! Numerals for counts on badges: the digits and `+`, each a few whole-pixel bars in a
//! [`NUMERAL_W`] x [`NUMERAL_H`] cell, one pixel apart.

pub const NUMERAL_W: i32 = 3;
pub const NUMERAL_H: i32 = 5;
const GAP: i32 = 1;

/// A bar's first and last column, first and last row, in its cell.
type Bar = (i32, i32, i32, i32);

const TOP: Bar = (0, 2, 0, 0);
const MID: Bar = (0, 2, 2, 2);
const FOOT: Bar = (0, 2, 4, 4);
const LEFT_UP: Bar = (0, 0, 0, 2);
const LEFT_DOWN: Bar = (0, 0, 2, 4);
const RIGHT_UP: Bar = (2, 2, 0, 2);
const RIGHT_DOWN: Bar = (2, 2, 2, 4);

fn bars(c: char) -> &'static [Bar] {
    match c {
        '0' => &[TOP, FOOT, LEFT_UP, LEFT_DOWN, RIGHT_UP, RIGHT_DOWN],
        '1' => &[(1, 1, 0, 4), (0, 0, 1, 1), FOOT],
        '2' => &[TOP, RIGHT_UP, MID, LEFT_DOWN, FOOT],
        '3' => &[TOP, RIGHT_UP, MID, RIGHT_DOWN, FOOT],
        '4' => &[LEFT_UP, RIGHT_UP, MID, RIGHT_DOWN],
        '5' => &[TOP, LEFT_UP, MID, RIGHT_DOWN, FOOT],
        '6' => &[TOP, LEFT_UP, MID, LEFT_DOWN, RIGHT_DOWN, FOOT],
        '7' => &[TOP, RIGHT_UP, RIGHT_DOWN],
        '8' => &[TOP, MID, FOOT, LEFT_UP, LEFT_DOWN, RIGHT_UP, RIGHT_DOWN],
        '9' => &[TOP, LEFT_UP, RIGHT_UP, MID, RIGHT_DOWN, FOOT],
        '+' => &[(1, 1, 1, 3), MID],
        _ => &[],
    }
}

/// `text`'s width in design pixels.
pub fn numerals_width(text: &str) -> i32 {
    let n = text.chars().count() as i32;
    (n * (NUMERAL_W + GAP) - GAP).max(0)
}

/// How much of the device pixel at (`x`, `y`) `text` covers, set from (`x0`, `y0`) (device
/// pixels) at `d` device pixels per design pixel. Every bar lies on whole design pixels, so a
/// pixel is covered wholly or not at all.
pub fn numerals(x: f64, y: f64, text: &str, x0: f64, y0: f64, d: i32) -> f64 {
    let k = f64::from(d.max(1));
    let (cx, cy) = (x + 0.5 - x0, y + 0.5 - y0);
    let inside = text.chars().enumerate().any(|(i, c)| {
        let left = f64::from(i as i32 * (NUMERAL_W + GAP));
        bars(c).iter().any(|&(c0, c1, r0, r1)| {
            let (l, r) = ((left + f64::from(c0)) * k, (left + f64::from(c1 + 1)) * k);
            let (t, b) = (f64::from(r0) * k, f64::from(r1 + 1) * k);
            cx >= l && cx < r && cy >= t && cy < b
        })
    });
    if inside { 1.0 } else { 0.0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(text: &str, d: i32) -> Vec<String> {
        let w = numerals_width(text) * d;
        (0..NUMERAL_H * d)
            .map(|y| {
                (0..w)
                    .map(|x| {
                        let on = numerals(f64::from(x), f64::from(y), text, 0.0, 0.0, d);
                        if on > 0.0 { '#' } else { '.' }
                    })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn a_count_is_three_pixel_numerals_one_pixel_apart() {
        assert_eq!(numerals_width(""), 0);
        assert_eq!(numerals_width("7"), 3);
        assert_eq!(numerals_width("99+"), 11);
        assert_eq!(
            rows("10", 1),
            [".#..###", "##..#.#", ".#..#.#", ".#..#.#", "###.###"]
        );
        assert_eq!(
            rows("4+", 1),
            ["#.#....", "#.#..#.", "###.###", "..#..#.", "..#...."]
        );
    }

    #[test]
    fn every_numeral_draws_apart_and_doubles_exactly_at_2() {
        let all: Vec<_> = "0123456789+"
            .chars()
            .map(|c| rows(&c.to_string(), 1))
            .collect();
        let distinct: std::collections::HashSet<_> = all.iter().collect();
        assert_eq!(distinct.len(), all.len());
        for c in "0123456789+".chars() {
            let (one, two) = (rows(&c.to_string(), 1), rows(&c.to_string(), 2));
            for (y, row) in two.iter().enumerate() {
                let doubled: String = one[y / 2].chars().flat_map(|p| [p, p]).collect();
                assert_eq!(*row, doubled, "{c} row {y}");
            }
        }
    }
}
