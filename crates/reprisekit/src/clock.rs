//! Where an analog clock's hands point.

/// The hour, minute and second hands at `(h, m, s)`, in turns clockwise from 12.
pub fn turns((h, m, s): (u32, u32, u32)) -> (f64, f64, f64) {
    let (h, m, s) = (f64::from(h % 12), f64::from(m), f64::from(s));
    (
        (h + m / 60.0 + s / 3600.0) / 12.0,
        (m + s / 60.0) / 60.0,
        s / 60.0,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hands_turn_with_the_time() {
        assert_eq!(turns((3, 0, 0)), (0.25, 0.0, 0.0));
        assert_eq!(turns((15, 30, 0)), (3.5 / 12.0, 0.5, 0.0));
        assert_eq!(turns((0, 0, 45)), (45.0 / 3600.0 / 12.0, 0.75 / 60.0, 0.75));
    }
}
