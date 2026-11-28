//! A folder wearing its picture (Customize > Choose Picture...): the picture stands in an open
//! manila folder, its tab and back behind it and the front lip over its foot, so the cell still
//! reads as a folder. The desktop draws it in the icon square, Thumbnails views in the
//! thumbnail square.

use crate::raster::{Canvas, fit_size};
use crate::shape::{self, Px, Stops, over, premul, rounded4, unpack};
use crate::theme::Color;

const EDGE: Color = Color::rgb(170, 128, 40);
const BACK: Color = Color::rgb(224, 182, 88);
const FRONT_TOP: Color = Color::rgb(255, 230, 146);
const FRONT_FOOT: Color = Color::rgb(242, 198, 94);

/// The design grid the geometry is laid on, scaled to each side.
const GRID: f64 = 32.0;
/// Each part as (x, y, w, h) on the grid. The front covers the opening's last three units.
const TAB: (f64, f64, f64, f64) = (2.0, 4.0, 11.0, 5.0);
const BACK_PANEL: (f64, f64, f64, f64) = (2.0, 7.0, 28.0, 21.0);
const OPENING: (f64, f64, f64, f64) = (5.0, 8.0, 22.0, 16.0);
const FRONT: (f64, f64, f64, f64) = (2.0, 21.0, 28.0, 7.0);
/// Corner radii on the grid: the tab's top ones, the back's and front's foot.
const TAB_R: f64 = 1.5;
const FOOT_R: f64 = 1.5;
/// The front's top corners.
const LIP_R: f64 = 0.5;

/// A `side` x `side` square at `scale` (device pixels, canvas scale 1): the folder, with the
/// picture `src` (premultiplied device pixels, `src_w` x `src_h`) of a `dims` picture fitted
/// inside its opening, never past the picture's own size, and centred there.
pub fn draw(
    side: u32,
    scale: i32,
    dims: (u32, u32),
    (src_w, src_h): (u32, u32),
    src: impl Fn(u32, u32) -> u32,
) -> Canvas {
    let d = scale.max(1);
    let k = f64::from(side) / GRID;
    let open_w = (OPENING.2 * k).floor() as u32;
    let open_h = (OPENING.3 * k).floor() as u32;
    let (lw, lh) = fit_size(dims.0, dims.1, (open_w, open_h));
    let s = d as u32;
    let (pw, ph) = (lw.max(1) * s, lh.max(1) * s);
    let picture = Canvas::resample_box_from(src_w, src_h, pw, ph, src);
    let u = k * f64::from(d);
    let px0 = ((OPENING.0 + OPENING.2 / 2.0) * u - f64::from(pw) / 2.0).round() as i32;
    let py0 = ((OPENING.1 + OPENING.3 / 2.0) * u - f64::from(ph) / 2.0).round() as i32;
    let e = f64::from(d);
    let solid = |c: Color| premul(c, 255);
    let front = [(0.0, solid(FRONT_TOP)), (1.0, solid(FRONT_FOOT))];
    // A part filled with its edge colour, then its body inset by one design pixel.
    let part = |x: f64, y: f64, (px, py, w, h): (f64, f64, f64, f64), r: [f64; 4], body: Px| {
        let (x, y) = (x - px * u, y - py * u);
        let (w, h) = (w * u, h * u);
        let edge = shape::scale(solid(EDGE), rounded4(x, y, w, h, r.map(|r| r * u)));
        let inner = r.map(|r| (r * u - e).max(0.0));
        let fill = rounded4(x - e, y - e, w - 2.0 * e, h - 2.0 * e, inner);
        over(shape::scale(body, fill), edge)
    };
    shape::draw(side as i32, side as i32, d, |x, y| {
        let mut p = part(x, y, TAB, [TAB_R, TAB_R, 0.0, 0.0], solid(BACK));
        p = over(
            part(x, y, BACK_PANEL, [0.0, TAB_R, FOOT_R, FOOT_R], solid(BACK)),
            p,
        );
        let (ix, iy) = (x as i32 - px0, y as i32 - py0);
        if (0..pw as i32).contains(&ix) && (0..ph as i32).contains(&iy) {
            p = over(unpack(picture.get(ix, iy)), p);
        }
        let t = (y + 0.5 - FRONT.1 * u) / (FRONT.3 * u);
        let lip = [LIP_R, LIP_R, FOOT_R, FOOT_R];
        over(part(x, y, FRONT, lip, Stops(&front).at(t)), p)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 40 x 30 picture, blue over its top half and red under it.
    fn picture() -> Canvas {
        let mut c = Canvas::new(40, 30);
        c.fill_rect(
            crate::raster::Rect::new(0, 0, 40, 15),
            Color::rgb(0, 0, 255),
        );
        c.fill_rect(
            crate::raster::Rect::new(0, 15, 40, 15),
            Color::rgb(255, 0, 0),
        );
        c
    }

    fn worn(side: u32, scale: i32) -> Canvas {
        let p = picture();
        draw(side, scale, (40, 30), (40, 30), |x, y| {
            p.get(x as i32, y as i32)
        })
    }

    #[test]
    fn the_folder_pins_at_the_icon_and_thumbnail_sides_at_scale_1_and_2() {
        let got: Vec<_> = [(32, 1), (32, 2), (94, 1), (94, 2)]
            .into_iter()
            .map(|(side, d)| (format!("{side} @{d}"), crate::pins::fnv(&worn(side, d))))
            .collect();
        crate::pins::check("art/folderpic", &got);
    }

    #[test]
    fn the_picture_stands_in_the_opening_with_the_front_over_its_foot() {
        for (side, d) in [(32, 1), (32, 2), (94, 1), (94, 2)] {
            let c = worn(side, d);
            let n = (side as i32) * d;
            assert_eq!(c.width as i32, n, "{side} @{d}: device pixels");
            let at = |gx: f64, gy: f64| {
                c.get(
                    (gx * f64::from(n) / GRID) as i32,
                    (gy * f64::from(n) / GRID) as i32,
                )
            };
            assert_eq!(
                at(16.0, 12.0) & 0xff_ffff,
                0x0000ff,
                "{side} @{d}: the picture's top shows"
            );
            assert_eq!(
                at(16.0, 20.0) & 0xff_ffff,
                0xff0000,
                "{side} @{d}: its foot shows above the lip"
            );
            let lip = at(16.0, 24.0);
            assert!(
                (lip >> 16) & 0xff > 230 && lip & 0xff < 160,
                "{side} @{d}: the front lip covers the foot, got {lip:08x}"
            );
            assert_eq!(at(3.0, 5.0) >> 24, 255, "{side} @{d}: the tab");
            assert_eq!(at(20.0, 5.0) >> 24, 0, "{side} @{d}: clear beside the tab");
        }
    }

    #[test]
    fn the_corners_are_clear_and_the_curves_anti_aliased_at_scale_2() {
        for side in [32, 94] {
            let c = worn(side, 2);
            let last = c.width as i32 - 1;
            for (x, y) in [(0, 0), (last, 0), (0, last), (last, last)] {
                assert_eq!(c.get(x, y) >> 24, 0, "{side}: corner ({x}, {y})");
            }
            let partial = (0..=last)
                .flat_map(|y| (0..=last).map(move |x| (x, y)))
                .any(|(x, y)| (1..255).contains(&(c.get(x, y) >> 24)));
            assert!(partial, "{side}: partial alpha on the rounded corners");
        }
    }

    #[test]
    fn a_small_picture_never_grows() {
        let mut q = Canvas::new(4, 4);
        q.fill_rect(q.bounds(), Color::rgb(0, 255, 0));
        let c = draw(94, 1, (4, 4), (4, 4), |x, y| q.get(x as i32, y as i32));
        let green = c
            .pixels
            .iter()
            .filter(|&&px| px & 0xff_ffff == 0x00ff00)
            .count();
        assert_eq!(green, 16, "a 4 x 4 picture stays 4 x 4");
    }
}
