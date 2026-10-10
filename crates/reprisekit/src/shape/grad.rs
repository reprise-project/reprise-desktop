//! Gradients: colours straight between stops, along any one measure
//! ([`Stops::at`]) or out from a centre ([`radial`]).

use super::Px;

/// A gradient through `stops`, each (position, colour), positions rising;
/// straight between stops (alpha and every channel alike), flat past the
/// first and the last. Two stops at one position make a hard step there.
#[derive(Clone, Copy)]
pub struct Stops<'a>(pub &'a [(f64, Px)]);

impl Stops<'_> {
    pub fn at(&self, t: f64) -> Px {
        let stops = self.0;
        let (first, last) = (stops[0], stops[stops.len() - 1]);
        if t <= first.0 {
            return first.1;
        }
        for w in stops.windows(2) {
            let ((t0, a), (t1, b)) = (w[0], w[1]);
            if t <= t1 {
                let k = if t1 == t0 { 0.0 } else { (t - t0) / (t1 - t0) };
                return (
                    a.0 + (b.0 - a.0) * k,
                    std::array::from_fn(|i| a.1[i] + (b.1[i] - a.1[i]) * k),
                );
            }
        }
        last.1
    }
}

/// `stops` at the middle of the pixel at (`x`, `y`) (its top-left corner),
/// by how far out it lies on the ellipse round (`cx`, `cy`) with radii
/// `rx`, `ry`: 0 at the centre, 1 on the ellipse, more past it. All in
/// device pixels.
pub fn radial(x: f64, y: f64, cx: f64, cy: f64, rx: f64, ry: f64, stops: &Stops) -> Px {
    let (u, v) = ((x + 0.5 - cx) / rx, (y + 0.5 - cy) / ry);
    stops.at((u * u + v * v).sqrt())
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLACK: Px = (255.0, [0.0; 3]);
    const WHITE: Px = (255.0, [255.0; 3]);

    #[test]
    fn stops_hold_their_ends_and_meet_halfway() {
        let s = Stops(&[(0.2, BLACK), (0.6, WHITE)]);
        assert_eq!(s.at(0.0), BLACK);
        assert_eq!(s.at(0.2), BLACK);
        let mid = s.at(0.4);
        assert!(mid.1.iter().all(|v| (v - 127.5).abs() < 1e-9), "{mid:?}");
        assert_eq!(s.at(0.6), WHITE);
        assert_eq!(s.at(1.0), WHITE);
    }

    #[test]
    fn two_stops_at_one_place_step() {
        let s = Stops(&[(0.0, BLACK), (0.5, BLACK), (0.5, WHITE), (1.0, WHITE)]);
        assert_eq!(s.at(0.49), BLACK);
        assert_eq!(s.at(0.51), WHITE);
    }

    #[test]
    fn radial_runs_from_centre_to_rim() {
        let s = Stops(&[(0.0, WHITE), (1.0, BLACK)]);
        assert_eq!(radial(9.5, 9.5, 10.0, 10.0, 8.0, 4.0, &s), WHITE);
        assert_eq!(radial(17.5, 9.5, 10.0, 10.0, 8.0, 4.0, &s), BLACK);
        assert_eq!(radial(9.5, 13.5, 10.0, 10.0, 8.0, 4.0, &s), BLACK);
    }
}
