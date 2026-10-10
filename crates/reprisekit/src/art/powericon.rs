//! The notification area's power icons, 16x16, from the PNGs in [`icons`]:
//! a plug on AC without a battery, and a battery whose fill follows the
//! charge (blue, yellow when low, red when critical) with a bolt while charging.

use crate::art::icons;
use crate::raster::{Canvas, Rect};
use crate::theme::Color;

pub const SIZE: u32 = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Ac,
    Battery {
        percent: u8,
        charging: bool,
        low: bool,
        critical: bool,
    },
}

const FILL_OK: Color = Color::rgb(49, 106, 197);
const FILL_LOW: Color = Color::rgb(236, 196, 0);
const FILL_CRITICAL: Color = Color::rgb(200, 24, 24);

/// Battery outline (1, 4) to (12, 11) with its nub at column 13; the fill
/// covers columns 2..=11 of rows 5..=10.
const FILL_X: (i32, i32) = (2, 11);
const FILL_Y: (i32, i32) = (5, 10);

/// Of ten fill columns: at least one with any charge, all ten only at 100.
pub fn fill_columns(percent: u8) -> i32 {
    let cols = FILL_X.1 - FILL_X.0 + 1;
    match percent {
        0 => 0,
        100.. => cols,
        p => ((i32::from(p) * cols + 50) / 100).clamp(1, cols - 1),
    }
}

pub fn draw(src: Source) -> Canvas {
    let mut c = Canvas::new(SIZE, SIZE);
    match src {
        Source::Ac => icons::AC.paint(&mut c, 0, 0, 1),
        Source::Battery {
            percent,
            charging,
            low,
            critical,
        } => {
            icons::BATTERY.paint(&mut c, 0, 0, 1);
            let fill = if critical {
                FILL_CRITICAL
            } else if low {
                FILL_LOW
            } else {
                FILL_OK
            };
            let cols = fill_columns(percent);
            c.fill_rect(
                Rect::new(FILL_X.0, FILL_Y.0, cols, FILL_Y.1 - FILL_Y.0 + 1),
                fill,
            );
            if charging {
                icons::BOLT.paint(&mut c, 0, 0, 1);
            }
        }
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUTLINE: Color = Color::rgb(64, 64, 64);
    const BODY: Color = Color::rgb(232, 232, 232);
    const PLUG: Color = Color::rgb(160, 160, 160);
    const PRONG: Color = Color::rgb(96, 96, 96);
    const BOLT: Color = Color::rgb(255, 222, 0);
    const BOLT_EDGE: Color = Color::rgb(90, 70, 0);

    fn px(c: &Canvas, x: i32, y: i32) -> Option<Color> {
        let v = c.get(x, y);
        (v >> 24 != 0).then(|| Color::rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
    }

    #[test]
    fn fill_follows_the_percentage() {
        assert_eq!(fill_columns(0), 0);
        assert_eq!(fill_columns(1), 1);
        assert_eq!(fill_columns(50), 5);
        assert_eq!(fill_columns(94), 9);
        assert_eq!(fill_columns(99), 9);
        assert_eq!(fill_columns(100), 10);
    }

    #[test]
    fn battery_is_outlined_filled_and_clear_around() {
        let full = draw(Source::Battery {
            percent: 100,
            charging: false,
            low: false,
            critical: false,
        });
        assert_eq!(px(&full, 1, 4), Some(OUTLINE));
        assert_eq!(px(&full, 13, 7), Some(OUTLINE), "the terminal nub");
        assert_eq!(px(&full, 2, 5), Some(FILL_OK));
        assert_eq!(px(&full, 11, 10), Some(FILL_OK));
        assert_eq!(px(&full, 0, 0), None);
        assert_eq!(px(&full, 15, 15), None);
        let half = draw(Source::Battery {
            percent: 50,
            charging: false,
            low: false,
            critical: false,
        });
        assert_eq!(px(&half, 6, 7), Some(FILL_OK));
        assert_eq!(px(&half, 7, 7), Some(BODY));
    }

    #[test]
    fn low_and_critical_recolour_the_fill() {
        let low = draw(Source::Battery {
            percent: 8,
            charging: false,
            low: true,
            critical: false,
        });
        assert_eq!(px(&low, 2, 7), Some(FILL_LOW));
        assert_eq!(px(&low, 3, 7), Some(BODY));
        let crit = draw(Source::Battery {
            percent: 2,
            charging: false,
            low: true,
            critical: true,
        });
        assert_eq!(px(&crit, 2, 7), Some(FILL_CRITICAL));
    }

    #[test]
    fn charging_lays_a_bolt_over_the_body() {
        let c = draw(Source::Battery {
            percent: 72,
            charging: true,
            low: false,
            critical: false,
        });
        assert_eq!(px(&c, 8, 7), Some(BOLT));
        assert_eq!(px(&c, 5, 7), Some(BOLT_EDGE));
        assert_eq!(px(&c, 2, 5), Some(FILL_OK), "the fill shows beside it");
    }

    #[test]
    fn ac_is_a_plug() {
        let c = draw(Source::Ac);
        assert_eq!(px(&c, 5, 1), Some(PRONG));
        assert_eq!(px(&c, 9, 3), Some(PRONG));
        assert_eq!(px(&c, 7, 6), Some(PLUG));
        assert_eq!(px(&c, 3, 4), Some(OUTLINE));
        assert_eq!(px(&c, 12, 15), Some(OUTLINE), "the cord");
        assert_eq!(px(&c, 0, 0), None);
    }
}
