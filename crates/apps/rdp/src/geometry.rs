//! Client points on the shared picture, mapped onto the session's logical layout.

/// The shared output's place in the layout, in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Region {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// The logical layout point under client pixel `p` of a `stream`-sized picture of `region`.
/// Points past the picture's edge clamp to its last pixel.
pub fn to_logical(p: (u16, u16), stream: (u32, u32), region: Region) -> (f64, f64) {
    let axis = |p: u16, pixels: u32, origin: i32, logical: u32| {
        let p = f64::from(u32::from(p).min(pixels.saturating_sub(1)));
        f64::from(origin) + p * f64::from(logical) / f64::from(pixels.max(1))
    };
    (
        axis(p.0, stream.0, region.x, region.width),
        axis(p.1, stream.1, region.y, region.height),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const RIGHT_OF_A_LAPTOP: Region = Region {
        x: 1920,
        y: 0,
        width: 1920,
        height: 1080,
    };

    #[test]
    fn a_scale_2_output_maps_to_logical() {
        assert_eq!(
            to_logical((1000, 500), (3840, 2160), RIGHT_OF_A_LAPTOP),
            (2420.0, 250.0)
        );
    }

    #[test]
    fn the_far_corner_stays_inside() {
        let (x, y) = to_logical((3839, 2159), (3840, 2160), RIGHT_OF_A_LAPTOP);
        assert!(
            x < 3840.0 && y < 1080.0,
            "({x}, {y}) is inside (3840, 1080)"
        );
        let (x, y) = to_logical((5000, 3000), (3840, 2160), RIGHT_OF_A_LAPTOP);
        assert!(
            x < 3840.0 && y < 1080.0,
            "({x}, {y}) past the edge clamps inside"
        );
    }

    #[test]
    fn a_size_change_maps_with_the_new_size() {
        let region = Region {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        };
        assert_eq!(
            to_logical((1000, 500), (1920, 1080), region),
            (1000.0, 500.0)
        );
        assert_eq!(
            to_logical((1000, 500), (3840, 2160), region),
            (500.0, 250.0)
        );
    }
}
